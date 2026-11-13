//! The NEAR ABI layer must produce a VALID module for every integer width.
//!
//! Found by execution (2026-11-13): a contract declaring a narrow state field
//! (`u8`/`u16`/`u32`/`bool`) produced WASM that wasmtime rejected with
//! `type mismatch: expected i64, found i32` — the read path stored a 32-bit
//! load into an i64 local. The write path had the mirror defect (`expected i32,
//! found i64`) once the read was fixed. Both widths were type-checked nowhere
//! except in the live validator, so the defects were invisible to the test
//! suite. Every example in `examples/` uses `u64` state, which is why the
//! existing gates stayed green.
//!
//! The gate here is `wasmparser`, i.e. a real validator, not a structural check.

use redsigma_codegen::{assemble, emit_near_module};
use redsigma_parser::Parser;

const WIDTHS: &[&str] = &[
    "u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64", "bool",
];

fn validate(bytes: &[u8]) -> Result<(), String> {
    assert_eq!(&bytes[0..4], b"\0asm");
    let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::default());
    v.validate_all(bytes).map(|_| ()).map_err(|e| e.to_string())
}

#[test]
fn every_integer_width_yields_a_valid_near_module() {
    let mut failures = Vec::new();
    for width in WIDTHS {
        let src = format!(
            r##"contract C {{
                state {{ n: {w} }}
                fn bump() -> {w} {{ n = n + 1; n; }}
                fn get() -> {w} {{ n; }}
            }}"##,
            w = width
        );
        let contracts = match Parser::parse(&src) {
            Ok(c) => c,
            Err(e) => {
                failures.push(format!("{width}: parse failed: {e}"));
                continue;
            }
        };
        let wat = match emit_near_module(&contracts[0]) {
            Ok(w) => w,
            Err(e) => {
                failures.push(format!("{width}: emit failed: {e}"));
                continue;
            }
        };
        let bytes = match assemble(&wat) {
            Ok(b) => b,
            Err(e) => {
                failures.push(format!("{width}: assemble failed: {e}"));
                continue;
            }
        };
        if let Err(e) = validate(&bytes) {
            failures.push(format!("{width}: {e}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} widths produced an invalid module:\n{}",
        failures.len(),
        WIDTHS.len(),
        failures.join("\n")
    );
}

/// The ABI must export every contract function (a NEAR wallet calls these by
/// name) plus `memory`, which the host needs to reach the contract's linear
/// memory. This caught the inline-export defect in `wat2wasm`.
#[test]
fn contract_functions_are_exported_under_their_own_names() {
    let src = r##"contract C {
            state { count: u8, total: u64 }
            fn bump() -> u8 { count = count + 1; count; }
            fn get_count() -> u8 { count; }
            fn add(amount: u64) -> u64 { total = total + amount; total; }
        }"##;
    let contracts = Parser::parse(src).unwrap();
    let wat = emit_near_module(&contracts[0]).unwrap();
    let bytes = assemble(&wat).unwrap();
    validate(&bytes).unwrap();

    let exports = export_names(&bytes);
    for want in ["bump", "get_count", "add", "init", "memory"] {
        assert!(
            exports.iter().any(|e| e == want),
            "missing export {want:?}; got {exports:?}"
        );
    }
}

/// A state field of a type the WASM backend cannot lower must be a clean
/// error, not an invalid module.
#[test]
fn unsupported_state_type_is_rejected_not_emitted_broken() {
    // `u128` is declared as a supported integer by the parser but the WAT
    // backend lowers everything to i64, so the ABI layer must refuse it.
    let src = r##"contract C {
            state { n: u128 }
            fn get() -> u128 { n; }
        }"##;
    let contracts = Parser::parse(src).unwrap();
    match emit_near_module(&contracts[0]) {
        Ok(wat) => {
            // If it emits, the result had better assemble AND validate —
            // an unvalidatable module is a defect, not a fallback.
            let bytes = assemble(&wat).expect("emitted WAT must assemble");
            validate(&bytes).expect("emitted module must validate");
        }
        Err(_) => { /* a clean refusal is also acceptable */ }
    }
}

fn export_names(bytes: &[u8]) -> Vec<String> {
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
            let mut j = i;
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
            for _ in 0..read_leb(&mut j) {
                let len = read_leb(&mut j);
                if j + len > bytes.len() {
                    break;
                }
                out.push(String::from_utf8_lossy(&bytes[j..j + len]).to_string());
                j += len;
                j += 1; // export kind byte
                read_leb(&mut j); // export index (LEB, may be multi-byte)
            }
        }
        i += size;
    }
    out
}
