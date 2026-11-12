//! REDΣSIGMA Proof-Carrying WASM Codegen
//!
//! Generates WebAssembly bytecode with embedded proof objects for verification.
//! Simplified implementation focusing on core functionality.

use redsigma_parser::{Contract, Expr, Stmt, Function, Type, BinOp, UnOp};
use redsigma_semantic::{SemanticAnalyzer, SemanticReport};
use redsigma_dualstream::{DualStreamAnalyzer, DualStreamReport};
use redsigma_reflection::{ReflectionEngine, ReflectionReport};
use redsigma_causal::CausalMemory;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use std::collections::HashMap;

pub mod wat;

pub use wat::{collect_locals, emit_wat, is_supported_int};

#[derive(Error, Debug)]
pub enum CodegenError {
    #[error("WASM encoding error: {0}")]
    WasmEncodingError(String),
    #[error("Type translation error: {0}")]
    TypeTranslationError(String),
    #[error("Unsupported expression: {0}")]
    UnsupportedExpression(String),
    #[error("Codegen error: {0}")]
    CodegenError(String),
    #[error("Semantic error: {0}")]
    SemanticError(String),
    #[error("Dual-stream error: {0}")]
    DualStreamError(String),
    #[error("Reflection error: {0}")]
    ReflectionError(String),
}

impl From<redsigma_semantic::SemanticError> for CodegenError {
    fn from(err: redsigma_semantic::SemanticError) -> Self {
        CodegenError::SemanticError(err.to_string())
    }
}

impl From<redsigma_dualstream::DualStreamError> for CodegenError {
    fn from(err: redsigma_dualstream::DualStreamError) -> Self {
        CodegenError::DualStreamError(err.to_string())
    }
}

impl From<redsigma_reflection::ReflectionError> for CodegenError {
    fn from(err: redsigma_reflection::ReflectionError) -> Self {
        CodegenError::ReflectionError(err.to_string())
    }
}

/// Proof object embedded in WASM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofObject {
    /// Contract name
    pub contract_name: String,
    /// Semantic report (Mod-5 projection)
    pub semantic_report: SemanticReport,
    /// Dual-stream report (Mod-5 + Mod-37/73)
    pub dual_stream_report: DualStreamReport,
    /// Reflection report (6-layer verification)
    pub reflection_report: ReflectionReport,
    /// 231-gate checksum
    pub gate231_checksum: u64,
    /// Causal chain root hash
    pub causal_root: u64,
}

/// WASM compilation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmCompilation {
    /// WASM bytecode (simplified - just placeholder)
    pub bytecode: Vec<u8>,
    /// Proof object
    pub proof: ProofObject,
    /// Function index mapping
    pub function_index: HashMap<String, u32>,
    /// Contract source for reference
    pub source: String,
}

/// Gate231 checksum computation
pub fn gate231_checksum(contract: &Contract) -> u64 {
    let mut sum = 0u64;
    let mut linear = 0u64;
    let mut prod = 1u64;

    for ch in contract.name.chars() {
        let val = ch as u64;
        sum = sum.wrapping_add(val);
        linear = linear.wrapping_add(val * 7);
        prod = prod.wrapping_mul(val.max(1));
    }

    for inv in &contract.invariants {
        hash_expr_checksum(&mut sum, &mut linear, &mut prod, &inv.expr);
    }

    for (name, ty) in &contract.state.fields {
        for ch in name.chars() {
            let val = ch as u64;
            sum = sum.wrapping_add(val);
            linear = linear.wrapping_add(val * 17);
            prod = prod.wrapping_mul(val.max(1));
        }
        for ch in ty.name.chars() {
            let val = ch as u64;
            sum = sum.wrapping_add(val);
            linear = linear.wrapping_add(val * 19);
            prod = prod.wrapping_mul(val.max(1));
        }
    }

    for func in &contract.functions {
        for ch in func.name.chars() {
            let val = ch as u64;
            sum = sum.wrapping_add(val);
            linear = linear.wrapping_add(val * 23);
            prod = prod.wrapping_mul(val.max(1));
        }
        for stmt in &func.body {
            hash_stmt_checksum(&mut sum, &mut linear, &mut prod, stmt);
        }
    }

    // The three channels are each reduced mod 231, but XOR of three values in
    // [0,230] can reach 255 — so the result must be reduced again or the
    // "checksum < 231" invariant is violated.
    ((sum % 231) ^ (linear % 231) ^ (prod % 231)) % 231
}

fn hash_stmt_checksum(sum: &mut u64, linear: &mut u64, prod: &mut u64, stmt: &Stmt) {
    match stmt {
        Stmt::Let { name, value, .. } => {
            for ch in name.chars() {
                let val = ch as u64;
                *sum = sum.wrapping_add(val);
                *linear = linear.wrapping_add(val * 29);
                *prod = prod.wrapping_mul(val.max(1));
            }
            hash_expr_checksum(sum, linear, prod, value);
        }
        Stmt::Const { name, value, .. } => {
            for ch in name.chars() {
                let val = ch as u64;
                *sum = sum.wrapping_add(val);
                *linear = linear.wrapping_add(val * 31);
                *prod = prod.wrapping_mul(val.max(1));
            }
            hash_expr_checksum(sum, linear, prod, value);
        }
        Stmt::Expr(expr) => hash_expr_checksum(sum, linear, prod, expr),
        Stmt::Return(expr) => {
            if let Some(e) = expr {
                hash_expr_checksum(sum, linear, prod, e);
            }
        }
        Stmt::If { condition, then_branch, else_branch } => {
            hash_expr_checksum(sum, linear, prod, condition);
            for s in then_branch {
                hash_stmt_checksum(sum, linear, prod, s);
            }
            if let Some(eb) = else_branch {
                for s in eb {
                    hash_stmt_checksum(sum, linear, prod, s);
                }
            }
        }
        Stmt::While { condition, body } => {
            hash_expr_checksum(sum, linear, prod, condition);
            for s in body {
                hash_stmt_checksum(sum, linear, prod, s);
            }
        }
        Stmt::For { var, iter, body } => {
            for ch in var.chars() {
                let val = ch as u64;
                *sum = sum.wrapping_add(val);
                *linear = linear.wrapping_add(val * 37);
                *prod = prod.wrapping_mul(val.max(1));
            }
            hash_expr_checksum(sum, linear, prod, iter);
            for s in body {
                hash_stmt_checksum(sum, linear, prod, s);
            }
        }
        Stmt::Match { expr, arms } => {
            hash_expr_checksum(sum, linear, prod, expr);
            for (pat, arm_expr) in arms {
                for ch in pat.chars() {
                    let val = ch as u64;
                    *sum = sum.wrapping_add(val);
                    *linear = linear.wrapping_add(val * 41);
                    *prod = prod.wrapping_mul(val.max(1));
                }
                hash_expr_checksum(sum, linear, prod, arm_expr);
            }
        }
        Stmt::Break | Stmt::Continue => {}
    }
}

fn hash_expr_checksum(sum: &mut u64, linear: &mut u64, prod: &mut u64, expr: &Expr) {
    match expr {
        Expr::Number(n) => {
            for ch in n.chars() {
                let val = ch as u64;
                *sum = sum.wrapping_add(val);
                *linear = linear.wrapping_add(val * 43);
                *prod = prod.wrapping_mul(val.max(1));
            }
        }
        Expr::String(s) => {
            for ch in s.chars() {
                let val = ch as u64;
                *sum = sum.wrapping_add(val);
                *linear = linear.wrapping_add(val * 47);
                *prod = prod.wrapping_mul(val.max(1));
            }
        }
        Expr::Ident(i) => {
            for ch in i.chars() {
                let val = ch as u64;
                *sum = sum.wrapping_add(val);
                *linear = linear.wrapping_add(val * 53);
                *prod = prod.wrapping_mul(val.max(1));
            }
        }
        Expr::Bool(b) => {
            let val = if *b { 1 } else { 0 };
            *sum = sum.wrapping_add(val);
            *linear = linear.wrapping_add(val * 59);
            *prod = prod.wrapping_mul(val.max(1));
        }
        Expr::Unit => {}
        Expr::Binary { left, op, right } => {
            hash_expr_checksum(sum, linear, prod, left);
            let val = *op as u64;
            *sum = sum.wrapping_add(val);
            *linear = linear.wrapping_add(val * 61);
            *prod = prod.wrapping_mul(val.max(1));
            hash_expr_checksum(sum, linear, prod, right);
        }
        Expr::Unary { op, expr } => {
            let val = *op as u64;
            *sum = sum.wrapping_add(val);
            *linear = linear.wrapping_add(val * 67);
            *prod = prod.wrapping_mul(val.max(1));
            hash_expr_checksum(sum, linear, prod, expr);
        }
        Expr::Range { from, to } => {
            hash_expr_checksum(sum, linear, prod, from);
            hash_expr_checksum(sum, linear, prod, to);
        }
        Expr::Call { func, args } => {
            for ch in func.chars() {
                let val = ch as u64;
                *sum = sum.wrapping_add(val);
                *linear = linear.wrapping_add(val * 71);
                *prod = prod.wrapping_mul(val.max(1));
            }
            for arg in args {
                hash_expr_checksum(sum, linear, prod, arg);
            }
        }
        Expr::MethodCall { receiver, method, args } => {
            hash_expr_checksum(sum, linear, prod, receiver);
            for ch in method.chars() {
                let val = ch as u64;
                *sum = sum.wrapping_add(val);
                *linear = linear.wrapping_add(val * 73);
                *prod = prod.wrapping_mul(val.max(1));
            }
            for arg in args {
                hash_expr_checksum(sum, linear, prod, arg);
            }
        }
        Expr::FieldAccess { receiver, field } => {
            hash_expr_checksum(sum, linear, prod, receiver);
            for ch in field.chars() {
                let val = ch as u64;
                *sum = sum.wrapping_add(val);
                *linear = linear.wrapping_add(val * 79);
                *prod = prod.wrapping_mul(val.max(1));
            }
        }
        Expr::Index { array, index } => {
            hash_expr_checksum(sum, linear, prod, array);
            hash_expr_checksum(sum, linear, prod, index);
        }
        Expr::Array(items) => {
            for item in items {
                hash_expr_checksum(sum, linear, prod, item);
            }
        }
        Expr::Struct { name, fields } => {
            for ch in name.chars() {
                let val = ch as u64;
                *sum = sum.wrapping_add(val);
                *linear = linear.wrapping_add(val * 83);
                *prod = prod.wrapping_mul(val.max(1));
            }
            for (k, v) in fields {
                for ch in k.chars() {
                    let val = ch as u64;
                    *sum = sum.wrapping_add(val);
                    *linear = linear.wrapping_add(val * 89);
                    *prod = prod.wrapping_mul(val.max(1));
                }
                hash_expr_checksum(sum, linear, prod, v);
            }
        }
        Expr::Block(stmts) => {
            for stmt in stmts {
                hash_stmt_checksum(sum, linear, prod, stmt);
            }
        }
        Expr::If { condition, then_branch, else_branch } => {
            hash_expr_checksum(sum, linear, prod, condition);
            hash_expr_checksum(sum, linear, prod, then_branch);
            if let Some(eb) = else_branch {
                hash_expr_checksum(sum, linear, prod, eb);
            }
        }
        Expr::Match { expr, arms } => {
            hash_expr_checksum(sum, linear, prod, expr);
            for (pat, arm_expr) in arms {
                for ch in pat.chars() {
                    let val = ch as u64;
                    *sum = sum.wrapping_add(val);
                    *linear = linear.wrapping_add(val * 97);
                    *prod = prod.wrapping_mul(val.max(1));
                }
                hash_expr_checksum(sum, linear, prod, arm_expr);
            }
        }
    }
}

/// Main codegen pipeline
pub struct CodegenPipeline {
    semantic_analyzer: SemanticAnalyzer,
    dual_stream_analyzer: DualStreamAnalyzer,
    reflection_engine: ReflectionEngine,
    causal_memory: CausalMemory,
}

impl CodegenPipeline {
    pub fn new() -> Self {
        Self {
            semantic_analyzer: SemanticAnalyzer::new(),
            dual_stream_analyzer: DualStreamAnalyzer::new(),
            reflection_engine: ReflectionEngine::new(),
            causal_memory: CausalMemory::new(),
        }
    }

    /// Compile a contract to WASM with proof objects
    pub fn compile(&mut self, contract: &Contract) -> Result<WasmCompilation, CodegenError> {
        // Run verification pipeline
        let sem_report = self.semantic_analyzer.analyze(contract)?;
        let ds_report = self.dual_stream_analyzer.analyze(contract)?;
        let refl_report = self.reflection_engine.reflect(contract)?;
        let gate231 = gate231_checksum(contract);

        // Record in causal memory
        let _ = self.causal_memory.record_transition(
            &contract.name,
            "deploy",
            0,
            ds_report.joint_signature,
            gate231,
            0,
            None,
        );

        // Generate WASM (simplified - returns placeholder bytecode)
        let bytecode = self.generate_wasm_placeholder(contract)?;

        // Create proof object
        let proof = ProofObject {
            contract_name: contract.name.clone(),
            semantic_report: sem_report,
            dual_stream_report: ds_report,
            reflection_report: refl_report,
            gate231_checksum: gate231,
            causal_root: self.causal_memory.len() as u64,
        };

        // Build function index
        let mut function_index = HashMap::new();
        for (idx, func) in contract.functions.iter().enumerate() {
            function_index.insert(func.name.clone(), idx as u32);
        }

        // Store source for reference
        let source = self.contract_to_source(contract);

        Ok(WasmCompilation {
            bytecode,
            proof,
            function_index,
            source,
        })
    }

    fn generate_wasm_placeholder(&self, contract: &Contract) -> Result<Vec<u8>, CodegenError> {
        // Generate a minimal WASM module as placeholder
        // In production, this would use wasm-encoder to generate real WASM
        let mut bytecode = Vec::new();

        // WASM magic number and version
        bytecode.extend_from_slice(&[0x00, 0x61, 0x73, 0x6D]); // \0asm
        bytecode.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]); // version 1

        // Type section (empty for placeholder)
        bytecode.push(0x01); // type section
        bytecode.push(0x00); // length 0

        // Function section (empty for placeholder)
        bytecode.push(0x03); // function section
        bytecode.push(0x00); // length 0

        // Export section
        bytecode.push(0x07); // export section
        bytecode.push(0x00); // length 0

        // Code section
        bytecode.push(0x0A); // code section
        bytecode.push(0x00); // length 0

        // Custom section for proof object
        bytecode.push(0x00); // custom section
        let proof_data = serde_json::to_vec(&format!("REDΣSIGMA Proof for {}", contract.name)).unwrap();
        bytecode.push(proof_data.len() as u8);
        bytecode.extend_from_slice(&proof_data);

        Ok(bytecode)
    }

    fn contract_to_source(&self, contract: &Contract) -> String {
        // The parser owns the canonical printer; this used to emit `{:?}` for
        // statements, which the parser could not read back.
        redsigma_parser::contract_to_source(contract)
    }
}

impl Default for CodegenPipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redsigma_parser::Parser;

    #[test]
    fn test_gate231_checksum() {
        let input = r#"
            contract Test {
                state { x: u64 }
                fn foo() -> u64 { 42; }
            }
        "#;
        let contracts = Parser::parse(input).unwrap();
        let checksum = gate231_checksum(&contracts[0]);
        assert!(checksum < 231);
    }

    #[test]
    fn test_gate231_deterministic() {
        let input = r#"
            contract Test {
                state { x: u64 }
                fn foo() -> u64 { 42; }
            }
        "#;
        let contracts = Parser::parse(input).unwrap();
        let checksum1 = gate231_checksum(&contracts[0]);
        let checksum2 = gate231_checksum(&contracts[0]);
        assert_eq!(checksum1, checksum2);
    }

    #[test]
    fn test_gate231_checksum_always_in_range() {
        // Regression: XOR of three mod-231 channels can reach 255, which broke
        // the "< 231" invariant that verify_proof enforces.
        let inputs = [
            r#"contract View { invariant x >= 0; state { x: u64 } fn set(v: u64) -> u64 { x = v; v; } fn get() -> u64 { x; } }"#,
            r#"contract Test { state { x: u64 } fn foo() -> u64 { 1; } }"#,
            r#"contract Counter { invariant count >= 0; state { count: u64 } fn increment() -> u64 { let n = count + 1; count = n; n; } }"#,
        ];
        for src in inputs {
            let contracts = Parser::parse(src).unwrap();
            let checksum = gate231_checksum(&contracts[0]);
            assert!(
                checksum < 231,
                "checksum {} out of range for {}",
                checksum,
                contracts[0].name
            );
        }
    }

    #[test]
    fn test_codegen_pipeline() {
        let input = r#"
            contract Counter {
                invariant count >= 0;
                state {
                    count: u64,
                }
                fn increment() -> u64 {
                    let new_count = count + 1;
                    count = new_count;
                    new_count;
                }
                fn get() -> u64 { count; }
            }
        "#;

        let contracts = Parser::parse(input).unwrap();
        let mut pipeline = CodegenPipeline::new();
        let result = pipeline.compile(&contracts[0]).unwrap();

        assert!(!result.bytecode.is_empty());
        assert_eq!(result.proof.contract_name, "Counter");
        assert!(result.function_index.contains_key("increment"));
        assert!(result.function_index.contains_key("get"));
        assert!(result.source.contains("contract Counter"));
        // The invariant expression is stored as debug format
        assert!(result.source.contains("Ge") || result.source.contains("count"));
    }

    #[test]
    fn test_wasm_bytecode_structure() {
        let input = r#"
            contract Test {
                state { x: u64 }
                fn foo() -> u64 { 42; }
            }
        "#;

        let contracts = Parser::parse(input).unwrap();
        let mut pipeline = CodegenPipeline::new();
        let result = pipeline.compile(&contracts[0]).unwrap();

        // Check WASM magic number
        assert_eq!(&result.bytecode[0..4], &[0x00, 0x61, 0x73, 0x6D]); // \0asm
        assert_eq!(&result.bytecode[4..8], &[0x01, 0x00, 0x00, 0x00]); // version 1
    }

    #[test]
    fn test_proof_object_serialization() {
        let input = r#"
            contract Test {
                state { x: u64 }
                fn foo() -> u64 { 42; }
            }
        "#;

        let contracts = Parser::parse(input).unwrap();
        let mut pipeline = CodegenPipeline::new();
        let result = pipeline.compile(&contracts[0]).unwrap();

        let proof_json = serde_json::to_string(&result.proof).unwrap();
        assert!(proof_json.contains("Test"));
        assert!(proof_json.contains("gate231_checksum"));
    }
}