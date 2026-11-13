//! Regression tests for `wat2wasm::assemble`.
//!
//! Each test below corresponds to a defect that made `assemble` emit a module
//! that no WASM validator accepts. The gates that caught them are the ones the
//! project already trusts: `wasmtime.Module` (structure + validation) and
//! section-order inspection. A regression in any of these paths must FAIL here
//! rather than silently producing an invalid module that fails later in CI.

use redsigma_codegen::assemble;

/// Minimal-but-valid module with one exported function and no imports.
fn tiny() -> &'static str {
    r##"(module
  (memory (export "memory") 1)
  (global $g (mut i64) (i64.const 0))
  (func $f (export "f") (result i64)
    (local $x i64)
    (local.set $x (i64.const 41))
    (local.get $x))
)"##
}

fn assert_valid(wat: &str) {
    let bytes = assemble(wat).expect("assemble should succeed");
    // Structural sanity first: the WASM magic + version must be present,
    // otherwise the validator's error is misleading.
    assert_eq!(&bytes[0..4], b"\0asm", "missing WASM magic");
    assert_eq!(&bytes[4..8], b"\x01\0\0\0", "wrong WASM version");
    // The validator is the only trustworthy gate for module validity.
    // `wasmparser` re-parses the binary sections and checks every index, type
    // and instruction-level invariant the encoder is supposed to have gotten
    // right — the same class of check `wasmtime` performs in
    // `verify/compile_examples.py`.
    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::default());
    validator
        .validate_all(&bytes)
        .unwrap_or_else(|e| panic!("wasmparser rejected the module: {e}"));
}

fn exports(bytes: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 8;
    while i < bytes.len() {
        let sid = bytes[i];
        i += 1;
        let mut shift = 0u32;
        let mut size = 0usize;
        loop {
            let b = bytes[i];
            i += 1;
            size |= ((b & 0x7f) as usize) << shift;
            shift += 7;
            if b & 0x80 == 0 {
                break;
            }
        }
        if sid == 7 {
            // entry: name_len (LEB u32) | name bytes | kind (u8) | index (LEB u32)
            let mut j = i;
            let mut shift = 0u32;
            let mut count = 0usize;
            loop {
                let b = bytes[j];
                j += 1;
                count |= ((b & 0x7f) as usize) << shift;
                shift += 7;
                if b & 0x80 == 0 {
                    break;
                }
            }
            let mut read_leb = |j: &mut usize| -> usize {
                let mut shift = 0u32;
                let mut v = 0usize;
                loop {
                    let b = bytes[*j];
                    *j += 1;
                    v |= ((b & 0x7f) as usize) << shift;
                    shift += 7;
                    if b & 0x80 == 0 {
                        break;
                    }
                }
                v
            };
            for _ in 0..count {
                let len = read_leb(&mut j);
                if j + len > bytes.len() {
                    break;
                }
                out.push(String::from_utf8_lossy(&bytes[j..j + len]).to_string());
                j += len;
                j += 1; // export kind
                read_leb(&mut j); // export index
            }
        }
        i += size;
    }
    out
}

/// Sections must appear in the spec-mandated order. Emitting them in the
/// order the WAT text happened to list them produced
/// "unknown type 0: type index out of bounds".
#[test]
fn sections_are_in_spec_order() {
    let bytes = assemble(tiny()).unwrap();
    let order = {
        let mut v = Vec::new();
        let mut i = 8;
        while i < bytes.len() {
            let sid = bytes[i];
            i += 1;
            let mut shift = 0u32;
            let mut size = 0usize;
            loop {
                let b = bytes[i];
                i += 1;
                size |= ((b & 0x7f) as usize) << shift;
                shift += 7;
                if b & 0x80 == 0 {
                    break;
                }
            }
            v.push(sid);
            i += size;
        }
        v
    };
    // type(1) < import(2) < func(3) < table(4) < memory(5) < global(6)
    // < export(7) < code(10)
    let rank = |id: u8| -> u32 {
        match id {
            1 | 2 | 3 | 4 | 5 | 6 | 7 | 10 => id as u32,
            other => panic!("unexpected section id {other}"),
        }
    };
    let mut sorted = order.clone();
    sorted.sort_by_key(|id| rank(*id));
    assert_eq!(order, sorted, "sections emitted out of spec order");
}

/// An inline `(export "f")` inside a `(func ...)` node must produce a real
/// export entry. It was ignored entirely, so every generated contract exported
/// nothing but memory.
#[test]
fn inline_func_export_is_emitted() {
    let bytes = assemble(tiny()).unwrap();
    let ex = exports(&bytes);
    assert!(ex.contains(&"memory".to_string()), "memory export missing");
    assert!(ex.contains(&"f".to_string()), "func export missing: {ex:?}");
}

/// The same module must actually validate.
#[test]
fn tiny_module_validates() {
    assert_valid(tiny());
}

/// Mixed i32/i64 local groups: indices are assigned in declaration order, not
/// in grouped-declaration order. Getting this wrong makes an i32 local read an
/// i64 slot and the validator reports "expected i32, found i64".
#[test]
fn mixed_i32_i64_locals_keep_their_types() {
    let wat = r##"(module
  (memory (export "memory") 1)
  (func $f (export "f") (result i32)
    (local $a i64)
    (local $p i32)
    (local $q i32)
    (local $b i64)
    (local.set $a (i64.const 7))
    (local.set $p (i32.const 3))
    (local.set $q (i32.const 4))
    (local.set $b (local.get $a))
    (i32.add
      (local.get $p)
      (local.get $q)))
)"##;
    assert_valid(wat);
}

/// A module that imports host functions must put the import indices before the
/// defined-function indices, and a `call` to an import must use the import's
/// index. This mirrors the NEAR ABI shape the compiler emits.
#[test]
fn imported_function_indices_precede_defined() {
    let wat = r##"(module
  (import "env" "value_set" (func $env_value_set (param i32) (param i32)))
  (import "env" "input" (func $env_input (result i32) (result i32)))
  (memory (export "memory") 1)
  (global $heap (mut i32) (i32.const 1024))
  (func $alloc (param $size i32) (result i32)
    (global.get $heap)
    (global.get $heap)
    (local.get $size)
    (i32.add)
    (global.set $heap))
  (func $init (export "init")
    (local $p i32)
    (local.set $p (call $alloc (i32.const 8)))
    (local.get $p)
    (i32.const 0)
    (call $env_value_set))
)"##;
    assert_valid(wat);
}