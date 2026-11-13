//! NEAR ABI layer: wraps generated WASM functions as NEAR contract methods.
//!
//! Emits a complete WAT module that, when assembled, produces a valid NEAR
//! contract:
//! - Imports NEAR host functions (`input`, `value_set`, `storage_read`,
//!   `storage_write`, `storage_has_key`)
//! - Exports linear memory
//! - `init` constructor
//! - One NEAR wrapper per contract function: read state from storage → call
//!   the inner function → write state back → return Borsh-compatible result
//!
//! The inner contract functions are emitted by [`crate::wat::emit_inner_wat`]
//! with `inner_`-prefixed names and no exports. This module adds the NEAR-facing
//! wrappers that export them under their original names.

use redsigma_parser::Contract;
use crate::wat::{emit_inner_wat, is_supported_int};
use crate::CodegenError;

/// Width in bytes for a REDΣSIGMA integer type. Used for storage encoding
/// and argument decoding.
fn type_width(name: &str) -> usize {
    let bare = name.strip_prefix("Verified").unwrap_or(name);
    match bare {
        "u8" | "i8" | "bool" => 1,
        "u16" | "i16" => 2,
        "u32" | "i32" => 4,
        "u64" | "i64" => 8,
        _ => 8, // u128/i128 are rejected by the WAT backend, default to 8
    }
}

/// Emit a complete NEAR contract WAT module for the given contract.
///
/// The output is a `(module ...)` WAT string that includes NEAR host imports,
/// exported memory, the inner contract functions (as `inner_*`), NEAR wrapper
/// functions (exported under the original names), and an `init` constructor.
pub fn emit_near_module(contract: &Contract) -> Result<String, CodegenError> {
    let mut out = String::new();

    out.push_str("(module\n");

    // ---- NEAR host function imports ----
    out.push_str(r#"  (import "env" "input" (func $env_input (result i32) (result i32)))"#);
    out.push('\n');
    out.push_str(r#"  (import "env" "value_set" (func $env_value_set (param i32) (param i32)))"#);
    out.push('\n');
    out.push_str(r#"  (import "env" "storage_read" (func $env_storage_read (param i32) (param i32) (result i32) (result i32)))"#);
    out.push('\n');
    out.push_str(r#"  (import "env" "storage_write" (func $env_storage_write (param i32) (param i32) (param i32) (param i32) (result i32)))"#);
    out.push('\n');
    out.push_str(r#"  (import "env" "storage_has_key" (func $env_storage_has_key (param i32) (param i32) (result i32)))"#);
    out.push('\n');

    // ---- Exported linear memory ----
    out.push_str("  (memory (export \"memory\") 1)\n");

    // ---- Heap pointer (bump allocator, starts at 1024) ----
    out.push_str("  (global $heap (mut i32) (i32.const 1024))\n");

    // ---- Key name pointers: track heap addresses of field-name keys ----
    out.push_str("  ;; Field name key pointers for storage access\n");
    for (i, (name, _)) in contract.state.fields.iter().enumerate() {
        let key_global = format!("$key_{}", i);
        out.push_str(&format!(
            "  (global {} (mut i32) (i32.const 0))\n",
            key_global
        ));
    }

    // ---- Allocator ----
    out.push_str(
        r#"
  ;; Bump allocator: alloc(size) -> ptr
  (func $alloc (param $size i32) (result i32)
    (global.get $heap)
    (global.get $heap)
    (local.get $size)
    (i32.add)
    (global.set $heap)
  )
"#,
    );

    // ---- Write field name keys to memory at module instantiation ----
    // Done via a data segment is not possible in our WAT subset, so key
    // pointers are written directly from `init`.

    // ---- Inner contract functions (renamed, not exported) ----
    let inner_wat = emit_inner_wat(contract)?;
    // emit_inner_wat returns globals + functions without (module) wrapper.
    // Indent it to fit inside our module.
    for line in inner_wat.lines() {
        out.push_str("  ");
        out.push_str(line);
        out.push('\n');
    }

    // ---- NEAR wrapper functions ----
    for func in &contract.functions {
        emit_near_wrapper(&mut out, func, contract)?;
    }

    // ---- init constructor ----
    emit_init(&mut out, contract)?;

    out.push_str(")\n");

    Ok(out)
}

/// Emit a NEAR wrapper for a single contract function.
///
/// The wrapper:
/// 1. Reads serialized args from `env::input()`
/// 2. Reads current state from storage into globals
/// 3. Calls the inner function
/// 4. Writes updated globals back to storage
/// 5. Serializes the result and calls `env::value_set()`
fn emit_near_wrapper(
    out: &mut String,
    func: &redsigma_parser::Function,
    contract: &Contract,
) -> Result<(), CodegenError> {
    let inner_name = format!("inner_{}", func.name);
    let near_name = &func.name;

    out.push_str(&format!(
        "  (func ${} (export \"{}\")\n",
        near_name, near_name
    ));

    // --- Parameter decoding locals ---
    // For each param, allocate a local to hold the decoded value.
    let mut param_locals = Vec::new();
    for (i, (pname, pty)) in func.params.iter().enumerate() {
        if !is_supported_int(&pty.name) {
            return Err(CodegenError::UnsupportedExpression(format!(
                "parameter '{}' of '{}' has type '{}', which the NEAR ABI does not support",
                pname, func.name, pty.name
            )));
        }
        param_locals.push((pname.clone(), pty.name.clone()));
        out.push_str(&format!("    (local ${} i64)\n", pname));
    }

    // --- Storage read locals ---
    for (fname, _) in &contract.state.fields {
        out.push_str(&format!("    (local ${}_val i64)\n", fname));
    }
    out.push_str("    (local $key_ptr i32)\n");
    out.push_str("    (local $key_len i32)\n");
    out.push_str("    (local $val_ptr i32)\n");
    out.push_str("    (local $val_len i32)\n");

    // --- Argument decoding locals ---
    out.push_str("    (local $input_ptr i32)\n");
    out.push_str("    (local $input_len i32)\n");
    out.push_str("    (local $arg_off i32)\n");

    // --- Result local ---
    if !func.return_type.name.is_empty() {
        out.push_str("    (local $result i64)\n");
    }
    out.push_str("    (local $res_ptr i32)\n");
    out.push_str("    (local $res_len i32)\n");

    // === Step 1: Read input args from env::input() ===
    out.push_str("    ;; Read input arguments\n");
    out.push_str("    (call $env_input)\n");
    out.push_str("    (local.set $input_len)\n");
    out.push_str("    (local.set $input_ptr)\n");
    out.push_str("    (local.set $arg_off (i32.const 0))\n");

    for (i, (pname, pty)) in param_locals.iter().enumerate() {
        let width = type_width(pty);
        out.push_str(&format!(
            "    ;; Decode arg {} '{}' (type {}, width {})\n",
            i, pname, pty, width
        ));
        // Param locals are declared i64 (see above), so narrow decodes widen the
        // same way the state read path does. Without the `extend_i32_u`,
        // wasmtime rejects the module: "expected i64, found i32".
        match pty.as_str() {
            "u8" | "i8" | "bool" => {
                out.push_str("    (local.get $input_ptr)\n");
                out.push_str("    (local.get $arg_off)\n");
                out.push_str("    (i32.add)\n");
                out.push_str("    (i32.load8_u)\n");
                out.push_str("    (i64.extend_i32_u)\n");
                out.push_str(&format!("    (local.set ${})\n", pname));
            }
            "u16" | "i16" => {
                out.push_str("    (local.get $input_ptr)\n");
                out.push_str("    (local.get $arg_off)\n");
                out.push_str("    (i32.add)\n");
                out.push_str("    (i32.load16_u)\n");
                out.push_str("    (i64.extend_i32_u)\n");
                out.push_str(&format!("    (local.set ${})\n", pname));
            }
            "u32" | "i32" => {
                out.push_str("    (local.get $input_ptr)\n");
                out.push_str("    (local.get $arg_off)\n");
                out.push_str("    (i32.add)\n");
                out.push_str("    (i32.load)\n");
                out.push_str("    (i64.extend_i32_u)\n");
                out.push_str(&format!("    (local.set ${})\n", pname));
            }
            "u64" | "i64" => {
                out.push_str("    (local.get $input_ptr)\n");
                out.push_str("    (local.get $arg_off)\n");
                out.push_str("    (i32.add)\n");
                out.push_str("    (i64.load)\n");
                out.push_str(&format!("    (local.set ${})\n", pname));
            }
            _ => {
                return Err(CodegenError::UnsupportedExpression(format!(
                    "NEAR arg decoding for type '{}' not implemented",
                    pty
                )));
            }
        }
        out.push_str(&format!(
            "    (local.set $arg_off (i32.add (local.get $arg_off) (i32.const {})))\n",
            width
        ));
    }

    // === Step 2: Read state from storage into globals ===
    out.push_str("    ;; Read contract state from storage\n");
    for (i, (fname, _)) in contract.state.fields.iter().enumerate() {
        let key_global = format!("$key_{}", i);
        out.push_str(&format!(
            "    ;; Read state field '{}'\n",
            fname
        ));
        // Write key to memory
        out.push_str(&format!(
            "    (local.set $key_ptr (global.get {}))\n",
            key_global
        ));
        out.push_str(&format!(
            "    (local.set $key_len (i32.const {}))\n",
            fname.len()
        ));
        // Has key?
        out.push_str("    (local.get $key_ptr)\n");
        out.push_str("    (local.get $key_len)\n");
        out.push_str("    (call $env_storage_has_key)\n");
        out.push_str("    (if\n");
        out.push_str("      (then\n");
        // Read value
        out.push_str("      (local.get $key_ptr)\n");
        out.push_str("      (local.get $key_len)\n");
        out.push_str("      (call $env_storage_read)\n");
        out.push_str("      (local.set $val_len)\n");
        out.push_str("      (local.set $val_ptr)\n");
        // Decode based on type
        // Find type
        let fty = contract
            .state
            .fields
            .iter()
            .find(|(n, _)| n == fname)
            .map(|(_, t)| t.name.clone())
            .unwrap_or_else(|| "u64".to_string());
        // The scratch local `${field}_val` is declared as i64, so every decode
        // must widen to i64 before the `local.set`. A narrow load produces i32
        // and cannot be stored directly: wasmtime rejects the module with
        // "type mismatch: expected i64, found i32". `load8_u`/`load16_u`
        // zero-extend, which is correct for the unsigned types; the signed
        // types would need `*_s`, but the WASM backend lowers every integer to
        // the same i64 ir representation, so zero-extension matches it.
        let width = type_width(&fty);
        match fty.as_str() {
            "u8" | "i8" | "bool" => {
                out.push_str("      (local.get $val_ptr)\n");
                out.push_str("      (i32.load8_u)\n");
                out.push_str("      (i64.extend_i32_u)\n");
                out.push_str(&format!("      (local.set ${}_val)\n", fname));
            }
            "u16" | "i16" => {
                out.push_str("      (local.get $val_ptr)\n");
                out.push_str("      (i32.load16_u)\n");
                out.push_str("      (i64.extend_i32_u)\n");
                out.push_str(&format!("      (local.set ${}_val)\n", fname));
            }
            "u32" | "i32" => {
                out.push_str("      (local.get $val_ptr)\n");
                out.push_str("      (i32.load)\n");
                out.push_str("      (i64.extend_i32_u)\n");
                out.push_str(&format!("      (local.set ${}_val)\n", fname));
            }
            "u64" | "i64" => {
                out.push_str("      (local.get $val_ptr)\n");
                out.push_str("      (i64.load)\n");
                out.push_str(&format!("      (local.set ${}_val)\n", fname));
            }
            _ => {}
        }
        // Set global from local
        out.push_str(&format!("      (global.set ${} (local.get ${}_val))\n", fname, fname));
        out.push_str("      )\n");
        out.push_str("      (else\n");
        // Default to 0
        out.push_str(&format!("      (global.set ${} (i64.const 0))\n", fname));
        out.push_str("      )\n");
        out.push_str("    )\n");
    }

    // === Step 3: Call inner function ===
    out.push_str("    ;; Call inner function\n");
    for (pname, _) in &param_locals {
        out.push_str(&format!("    (local.get ${})\n", pname));
    }
    out.push_str(&format!("    (call ${})\n", inner_name));
    if !func.return_type.name.is_empty() {
        out.push_str("    (local.set $result)\n");
    }

    // === Step 4: Write state back to storage ===
    out.push_str("    ;; Write contract state to storage\n");
    for (i, (fname, _)) in contract.state.fields.iter().enumerate() {
        let key_global = format!("$key_{}", i);
        let fty = contract
            .state
            .fields
            .iter()
            .find(|(n, _)| n == fname)
            .map(|(_, t)| t.name.clone())
            .unwrap_or_else(|| "u64".to_string());
        let width = type_width(&fty);
        out.push_str(&format!(
            "    ;; Write state field '{}'\n",
            fname
        ));
        // Allocate buffer for value
        out.push_str(&format!(
            "    (local.set $val_ptr (call $alloc (i32.const {})))\n",
            width
        ));
        out.push_str(&format!("    (local.set $val_len (i32.const {}))\n", width));
        // Encode value. State globals are i64, so a narrow store must truncate
        // to i32 first; without the `wrap_i64` wasmtime rejects the module
        // with "type mismatch: expected i32, found i64".
        match fty.as_str() {
            "u8" | "i8" | "bool" => {
                out.push_str("    (local.get $val_ptr)\n");
                out.push_str(&format!("    (global.get ${})\n", fname));
                out.push_str("    (i32.wrap_i64)\n");
                out.push_str("    (i32.store8)\n");
            }
            "u16" | "i16" => {
                out.push_str("    (local.get $val_ptr)\n");
                out.push_str(&format!("    (global.get ${})\n", fname));
                out.push_str("    (i32.wrap_i64)\n");
                out.push_str("    (i32.store16)\n");
            }
            "u32" | "i32" => {
                out.push_str("    (local.get $val_ptr)\n");
                out.push_str(&format!("    (global.get ${})\n", fname));
                out.push_str("    (i32.wrap_i64)\n");
                out.push_str("    (i32.store)\n");
            }
            "u64" | "i64" => {
                out.push_str("    (local.get $val_ptr)\n");
                out.push_str(&format!("    (global.get ${})\n", fname));
                out.push_str("    (i64.store)\n");
            }
            _ => {}
        }
        // Write key to memory if not already there
        out.push_str(&format!(
            "    (local.set $key_ptr (global.get {}))\n",
            key_global
        ));
        out.push_str(&format!(
            "    (local.set $key_len (i32.const {}))\n",
            fname.len()
        ));
        // Call storage_write
        out.push_str("    (local.get $key_ptr)\n");
        out.push_str("    (local.get $key_len)\n");
        out.push_str("    (local.get $val_ptr)\n");
        out.push_str("    (local.get $val_len)\n");
        out.push_str("    (call $env_storage_write)\n");
        out.push_str("    (drop)\n");
    }

    // === Step 5: Set return value ===
    out.push_str("    ;; Set return value\n");
    if func.return_type.name.is_empty() {
        out.push_str("    (local.set $res_ptr (call $alloc (i32.const 0)))\n");
        out.push_str("    (local.set $res_len (i32.const 0))\n");
    } else {
        out.push_str("    (local.set $res_ptr (call $alloc (i32.const 8)))\n");
        out.push_str("    (local.set $res_len (i32.const 8))\n");
        out.push_str("    (local.get $res_ptr)\n");
        out.push_str("    (local.get $result)\n");
        out.push_str("    (i64.store)\n");
    }
    out.push_str("    (local.get $res_ptr)\n");
    out.push_str("    (local.get $res_len)\n");
    out.push_str("    (call $env_value_set)\n");

    out.push_str("  )\n");

    Ok(())
}

/// Emit the `init` constructor.
///
/// Initializes contract state: writes field name keys to memory and
/// initializes all state fields to 0 (which is their default).
fn emit_init(out: &mut String, contract: &Contract) -> Result<(), CodegenError> {
    out.push_str("  ;; init constructor\n");
    out.push_str("  (func $init (export \"init\")\n");

    // Allocate and write each field name key to memory, store pointer in key_global
    for (i, (fname, _)) in contract.state.fields.iter().enumerate() {
        let key_global = format!("$key_{}", i);
        let len = fname.len();
        out.push_str(&format!(
            "    ;; Write key '{}' to memory\n",
            fname
        ));
        out.push_str(&format!(
            "    (global.set {} (call $alloc (i32.const {})))\n",
            key_global, len
        ));
        // Write each byte of the field name
        for (j, b) in fname.bytes().enumerate() {
            out.push_str(&format!("    (global.get {})\n", key_global));
            out.push_str(&format!("    (i32.const {})\n", j));
            out.push_str("    (i32.add)\n");
            out.push_str(&format!("    (i32.const {})\n", b));
            out.push_str("    (i32.store8)\n");
        }
    }

    // State globals are already initialized to 0 by their (i64.const 0) init.
    // No storage write needed in init since NEAR storage starts empty and
    // reads return 0 for missing keys.

    out.push_str("  )\n");

    Ok(())
}
