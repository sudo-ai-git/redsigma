//! REDΣSIGMA compiler - parse, verify, and lower to WASM

use redsigma_codegen::CodegenPipeline;
use redsigma_parser::{Contract, Parser};

#[derive(Debug, thiserror::Error)]
pub enum CompilerError {
    #[error("Parse error: {0}")]
    ParseError(String),
    #[error("Type error: {0}")]
    TypeError(String),
    #[error("Verification error: {0}")]
    VerificationError(String),
    #[error("Codegen error: {0}")]
    CodegenError(String),
}

/// Compile a REDΣSIGMA source file to WASM bytecode with proof objects.
///
/// Pipeline: parse → semantic projection → dual-stream → reflection →
/// causal memory → WAT lowering → WASM assembly.
///
/// The input must contain exactly one contract; `parse_contract` returns every
/// contract in the file so callers can inspect them or compile one at a time.
pub fn compile(source: &str) -> Result<Vec<u8>, CompilerError> {
    let contracts = parse_contract(source)?;
    if contracts.is_empty() {
        return Err(CompilerError::ParseError(
            "no contracts found in input".to_string(),
        ));
    }
    if contracts.len() > 1 {
        let names: Vec<&str> = contracts.iter().map(|c| c.name.as_str()).collect();
        return Err(CompilerError::ParseError(format!(
            "compile() takes exactly one contract; found {}: {}",
            contracts.len(),
            names.join(", ")
        )));
    }
    let mut pipeline = CodegenPipeline::new();
    let compilation = pipeline
        .compile(&contracts[0])
        .map_err(|e| CompilerError::CodegenError(e.to_string()))?;
    Ok(compilation.bytecode)
}

pub mod verify_json;

/// Parse and validate REDΣSIGMA contracts.
pub fn parse_contract(source: &str) -> Result<Vec<Contract>, CompilerError> {
    Parser::parse(source).map_err(CompilerError::ParseError)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE: &str = r#"
        contract Counter {
            invariant count >= 0;
            state { count: u64 }
            fn increment() -> u64 { count = count + 1; count; }
        }
    "#;

    #[test]
    fn test_compile_produces_wasm_bytes() {
        let wasm = compile(SIMPLE).expect("must compile");
        assert_eq!(&wasm[0..4], &[0x00, 0x61, 0x73, 0x6d], "WASM magic");
        assert_eq!(&wasm[4..8], &[0x01, 0x00, 0x00, 0x00], "WASM version");
        assert!(wasm.len() > 40, "module is only {} bytes", wasm.len());
    }

    #[test]
    fn test_compile_rejects_multiple_contracts() {
        let src = r#"
            contract A { state { x: u64 } fn f() -> u64 { 1; } }
            contract B { state { y: u64 } fn g() -> u64 { 2; } }
        "#;
        let err = compile(src).unwrap_err().to_string();
        assert!(err.contains("exactly one contract"), "got: {err}");
        assert!(err.contains("A") && err.contains("B"), "got: {err}");
    }

    #[test]
    fn test_compile_rejects_empty_input() {
        let err = compile("").unwrap_err().to_string();
        assert!(err.contains("no contracts"), "got: {err}");
    }

    #[test]
    fn test_compile_rejects_parse_errors() {
        let err = compile("contract {").unwrap_err().to_string();
        assert!(err.contains("Parse error"), "got: {err}");
    }
}
