//! REDΣSIGMA 6-Layer Self-Reflection Engine
//!
//! Implements the six-layer self-reflection verification:
//! 1. Consistency - Internal logical consistency
//! 2. Memory - Causal memory integrity
//! 3. Oracle - External oracle verification
//! 4. Completeness - Specification completeness
//! 5. Uncertainty - Uncertainty quantification
//! 6. Contradiction - Contradiction detection

use redsigma_parser::{BinOp, Contract, Expr, Stmt, UnOp, collect_idents, expr_to_source};
use redsigma_semantic::{SemanticAnalyzer, SemanticReport};
use redsigma_dualstream::{DualStreamAnalyzer, DualStreamReport};
use redsigma_causal::CausalMemory;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use std::collections::HashMap;

#[derive(Error, Debug)]
pub enum ReflectionError {
    #[error("Consistency check failed: {0}")]
    ConsistencyError(String),
    #[error("Memory integrity check failed: {0}")]
    MemoryError(String),
    #[error("Oracle verification failed: {0}")]
    OracleError(String),
    #[error("Completeness check failed: {0}")]
    CompletenessError(String),
    #[error("Uncertainty quantification failed: {0}")]
    UncertaintyError(String),
    #[error("Contradiction detected: {0}")]
    ContradictionError(String),
    #[error("Reflection error: {0}")]
    ReflectionError(String),
    #[error("Semantic error: {0}")]
    SemanticError(String),
    #[error("Dual-stream error: {0}")]
    DualStreamError(String),
}

impl From<redsigma_semantic::SemanticError> for ReflectionError {
    fn from(err: redsigma_semantic::SemanticError) -> Self {
        ReflectionError::SemanticError(err.to_string())
    }
}

impl From<redsigma_dualstream::DualStreamError> for ReflectionError {
    fn from(err: redsigma_dualstream::DualStreamError) -> Self {
        ReflectionError::DualStreamError(err.to_string())
    }
}

/// Result of a single reflection layer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerResult {
    pub layer: u8,
    pub name: String,
    pub passed: bool,
    pub score: f64, // 0.0 to 1.0
    pub details: String,
    pub evidence: Vec<String>,
}

/// Full reflection report across all 6 layers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionReport {
    pub contract_name: String,
    pub layers: Vec<LayerResult>,
    pub overall_passed: bool,
    pub overall_score: f64,
    pub critical_issues: Vec<String>,
}

/// Layer 1: Consistency - Internal logical consistency
pub fn check_consistency(contract: &Contract) -> LayerResult {
    let mut evidence = Vec::new();
    let mut score: f64 = 1.0;
    let mut passed = true;

    // Check 1: No duplicate state field names
    let mut state_names = HashMap::new();
    for (name, _) in &contract.state.fields {
        *state_names.entry(name).or_insert(0) += 1;
    }
    for (name, count) in state_names {
        if count > 1 {
            evidence.push(format!("Duplicate state field: {}", name));
            score -= 0.2;
            passed = false;
        }
    }

    // Check 2: No duplicate function names
    let mut func_names = HashMap::new();
    for func in &contract.functions {
        *func_names.entry(&func.name).or_insert(0) += 1;
    }
    for (name, count) in func_names {
        if count > 1 {
            evidence.push(format!("Duplicate function: {}", name));
            score -= 0.2;
            passed = false;
        }
    }

    // Check 3: All invariants reference valid state fields
    let state_field_names: Vec<&str> = contract.state.fields.iter().map(|(n, _)| n.as_str()).collect();
    for inv in &contract.invariants {
        let mut idents = Vec::new();
        collect_idents(&inv.expr, &mut idents);
        let has_valid_ref = state_field_names
            .iter()
            .any(|f| idents.contains(&f.to_string()));
        if !has_valid_ref && !contract.state.fields.is_empty() {
            evidence.push(format!(
                "Invariant may not reference state: {}",
                expr_to_source(&inv.expr)
            ));
            score -= 0.1;
        }
    }

    // Check 4: Function parameters have valid types
    for func in &contract.functions {
        for (param_name, param_ty) in &func.params {
            if param_ty.name.is_empty() {
                evidence.push(format!("Function {} has parameter {} with empty type", func.name, param_name));
                score -= 0.1;
                passed = false;
            }
        }
    }

    score = score.max(0.0);
    LayerResult {
        layer: 1,
        name: "Consistency".to_string(),
        passed,
        score,
        details: "Internal logical consistency check".to_string(),
        evidence,
    }
}

/// Layer 2: Memory - Causal memory integrity
pub fn check_memory(
    _contract: &Contract,
    causal_memory: &CausalMemory,
) -> LayerResult {
    let mut evidence = Vec::new();
    let mut score: f64 = 1.0;
    let mut passed = true;

    // Check causal integrity
    if let Err(e) = causal_memory.verify_integrity() {
        evidence.push(format!("Causal integrity violation: {}", e));
        score -= 0.5;
        passed = false;
    }

    // Check contract has history in causal memory
    // (We can't easily check specific contract history without the name)
    // The verify_integrity already checks for orphaned nodes

    score = score.max(0.0);
    LayerResult {
        layer: 2,
        name: "Memory".to_string(),
        passed,
        score,
        details: "Causal memory integrity check".to_string(),
        evidence,
    }
}

/// Layer 3: Oracle - External oracle verification
pub fn check_oracle(
    contract: &Contract,
    dual_stream_report: &DualStreamReport,
) -> LayerResult {
    let mut evidence = Vec::new();
    let mut score: f64 = 1.0;
    let mut passed = true;

    // Check joint uniqueness (oracle verification)
    if dual_stream_report.collisions > 0 {
        evidence.push(format!("Joint uniqueness violation: {} collisions", dual_stream_report.collisions));
        score -= 0.4;
        passed = false;
    }

    // Check bijection verification
    if !dual_stream_report.bijection_verified {
        evidence.push("Bijective verification failed".to_string());
        score -= 0.3;
        passed = false;
    }

    // Check sentinel values
    if dual_stream_report.is_sentinel {
        evidence.push("Sentinel value detected in structural channels".to_string());
        score -= 0.1; // Not necessarily an error, but noteworthy
    }

    // Check structural channels are in valid ranges
    let (lin37, lin73, sum231, prod231) = dual_stream_report.structural_channels;
    if lin37 >= 37 || lin73 >= 73 || sum231 >= 231 || prod231 >= 231 {
        evidence.push("Structural channels out of valid range".to_string());
        score -= 0.3;
        passed = false;
    }

    score = score.max(0.0);
    LayerResult {
        layer: 3,
        name: "Oracle".to_string(),
        passed,
        score,
        details: "External oracle verification (dual-stream, bijection)".to_string(),
        evidence,
    }
}

/// Layer 4: Completeness - Specification completeness
pub fn check_completeness(contract: &Contract) -> LayerResult {
    let mut evidence = Vec::new();
    let mut score: f64 = 1.0;
    let mut passed = true;

    // Check 1: Has at least one invariant
    if contract.invariants.is_empty() {
        evidence.push("No invariants declared".to_string());
        score -= 0.3;
        passed = false;
    }

    // Check 2: Has state fields
    if contract.state.fields.is_empty() {
        evidence.push("No state fields declared".to_string());
        score -= 0.2;
        passed = false;
    }

    // Check 3: All functions have @verified annotation (in real implementation)
    // For now, check they have bodies
    for func in &contract.functions {
        if func.body.is_empty() {
            evidence.push(format!("Function {} has empty body", func.name));
            score -= 0.1;
        }
    }

    // Check 4: State fields have valid types
    for (name, ty) in &contract.state.fields {
        if ty.name.is_empty() {
            evidence.push(format!("State field {} has empty type", name));
            score -= 0.2;
            passed = false;
        }
    }

    // Check 5: Functions have return types
    for func in &contract.functions {
        if func.return_type.name.is_empty() {
            evidence.push(format!("Function {} has empty return type", func.name));
            score -= 0.2;
            passed = false;
        }
    }

    score = score.max(0.0);
    LayerResult {
        layer: 4,
        name: "Completeness".to_string(),
        passed,
        score,
        details: "Specification completeness check".to_string(),
        evidence,
    }
}

/// Layer 5: Uncertainty - Uncertainty quantification
pub fn check_uncertainty(
    contract: &Contract,
    sem_report: &SemanticReport,
) -> LayerResult {
    let mut evidence = Vec::new();
    let mut score: f64 = 1.0;
    let mut passed = true;

    // Uncertainty based on semantic distribution entropy
    let dist = sem_report.bucket_distribution;
    let mut entropy = 0.0;
    for p in dist {
        if p > 0.0 {
            entropy -= p * p.ln();
        }
    }

    // Normalize entropy (max entropy for 5 buckets is ln(5) ≈ 1.609)
    let normalized_entropy = entropy / 1.609;

    // High entropy = high uncertainty = lower score
    // We want moderate entropy (not too ordered, not too random)
    if normalized_entropy > 0.95 {
        evidence.push(format!("High semantic uncertainty (entropy: {:.3})", normalized_entropy));
        score -= 0.2;
    } else if normalized_entropy < 0.1 {
        evidence.push(format!("Low semantic diversity (entropy: {:.3})", normalized_entropy));
        score -= 0.1;
    }

    // Check for collisions in bijective verifier
    if sem_report.collisions > 0 {
        evidence.push(format!("Semantic collisions detected: {}", sem_report.collisions));
        score -= 0.3;
        passed = false;
    }

    // Check total symbols processed
    if sem_report.total_symbols == 0 {
        evidence.push("No symbols processed in semantic analysis".to_string());
        score -= 0.2;
        passed = false;
    }

    score = score.max(0.0);
    LayerResult {
        layer: 5,
        name: "Uncertainty".to_string(),
        passed,
        score,
        details: format!("Uncertainty quantification (entropy: {:.3})", normalized_entropy),
        evidence,
    }
}

/// Layer 6: Contradiction - Contradiction detection
pub fn check_contradiction(contract: &Contract) -> LayerResult {
    let mut evidence = Vec::new();
    let mut score: f64 = 1.0;
    let mut passed = true;

    // Check for contradictory invariants
    // e.g., "x > 0" and "x < 0" would be contradictory
    // This is a simplified check - real implementation would use SMT solver
    for i in 0..contract.invariants.len() {
        for j in (i + 1)..contract.invariants.len() {
            let inv1 = &contract.invariants[i].expr;
            let inv2 = &contract.invariants[j].expr;

            // Structural comparison over the AST, not string matching
            if is_contradictory(inv1, inv2) {
                evidence.push(format!(
                    "Potential contradiction: {} vs {}",
                    expr_to_source(inv1),
                    expr_to_source(inv2)
                ));
                score -= 0.4;
                passed = false;
            }
        }
    }

    // Check for functions that violate their own invariants
    // (simplified: check if any assignment could violate invariant)
    for func in &contract.functions {
        for stmt in &func.body {
            if let Stmt::Let { name, value, .. } = stmt {
                // Check if this assignment could violate any invariant
                for inv in &contract.invariants {
                    let mut idents = Vec::new();
                    collect_idents(&inv.expr, &mut idents);
                    if idents.contains(name) {
                        // This is a very rough check
                        evidence.push(format!(
                            "Assignment to {} may affect invariant: {}",
                            name,
                            expr_to_source(&inv.expr)
                        ));
                    }
                }
            }
        }
    }

    score = score.max(0.0);
    LayerResult {
        layer: 6,
        name: "Contradiction".to_string(),
        passed,
        score,
        details: "Contradiction detection".to_string(),
        evidence,
    }
}

/// `(ident, op, literal)` for a simple `ident <op> <number>` comparison.
/// Accepts a negated literal: `x <= -1` parses as `Unary(Neg, Number(1))`.
fn comparison_triple(expr: &Expr) -> Option<(String, BinOp, i128)> {
    let Expr::Binary { left, op, right } = expr else {
        return None;
    };
    let name = match left.as_ref() {
        Expr::Ident(n) => n.clone(),
        _ => return None,
    };
    let val = match right.as_ref() {
        Expr::Number(n) => n.parse::<i128>().ok()?,
        Expr::Unary {
            op: UnOp::Neg,
            expr: inner,
        } => match inner.as_ref() {
            Expr::Number(n) => -n.parse::<i128>().ok()?,
            _ => return None,
        },
        _ => return None,
    };
    Some((name, *op, val))
}

/// Structural contradiction detection over the AST.
///
/// Returns `false` whenever it cannot decide: this is a heuristic layer, not a
/// proof. The SMT layer owns real discharge; this exists to catch the obvious
/// `x >= 0` alongside `x < 0` without pretending to more than it checks.
fn is_contradictory(a: &Expr, b: &Expr) -> bool {
    let (Some((n1, op1, v1)), Some((n2, op2, v2))) =
        (comparison_triple(a), comparison_triple(b))
    else {
        return false;
    };
    if n1 != n2 {
        return false;
    }
    let ge0_1 = matches!(op1, BinOp::Ge) && v1 == 0;
    let lt0_1 = matches!(op1, BinOp::Lt) && v1 == 0;
    let le_neg1_1 = matches!(op1, BinOp::Le) && v1 == -1;
    let ge0_2 = matches!(op2, BinOp::Ge) && v2 == 0;
    let lt0_2 = matches!(op2, BinOp::Lt) && v2 == 0;
    let le_neg1_2 = matches!(op2, BinOp::Le) && v2 == -1;

    // x >= 0 vs x < 0 ; x >= 0 vs x <= -1  (and the symmetric forms)
    if (ge0_1 && (lt0_2 || le_neg1_2)) || (ge0_2 && (lt0_1 || le_neg1_1)) {
        return true;
    }
    // x == k vs x != k for the same k
    if v1 == v2
        && ((matches!(op1, BinOp::Eq) && matches!(op2, BinOp::Ne))
            || (matches!(op1, BinOp::Ne) && matches!(op2, BinOp::Eq)))
    {
        return true;
    }
    false
}

    /// Main reflection engine
    pub struct ReflectionEngine {
        semantic_analyzer: SemanticAnalyzer,
        dual_stream_analyzer: DualStreamAnalyzer,
        causal_memory: CausalMemory,
    }

    impl ReflectionEngine {
        pub fn new() -> Self {
            Self {
                semantic_analyzer: SemanticAnalyzer::new(),
                dual_stream_analyzer: DualStreamAnalyzer::new(),
                causal_memory: CausalMemory::new(),
            }
        }

        /// Run full 6-layer reflection on a contract
        pub fn reflect(&mut self, contract: &Contract) -> Result<ReflectionReport, ReflectionError> {
        // Run semantic analysis first
        let sem_report = self.semantic_analyzer.analyze(contract)?;

        // Run dual-stream analysis
        let ds_report = self.dual_stream_analyzer.analyze(contract)?;

        // Record transition in causal memory (simulated)
        let tx_hash = 0; // Would be real transaction hash
        let _ = self.causal_memory.record_transition(
            &contract.name,
            "deploy",
            0,
            ds_report.joint_signature,
            ds_report.joint_signature % 231,
            tx_hash,
            None,
        );

        // Run all 6 layers
        let mut layers = Vec::new();
        layers.push(check_consistency(contract));
        layers.push(check_memory(contract, &self.causal_memory));
        layers.push(check_oracle(contract, &ds_report));
        layers.push(check_completeness(contract));
        layers.push(check_uncertainty(contract, &sem_report));
        layers.push(check_contradiction(contract));

        // Compute overall results
        let overall_passed = layers.iter().all(|l| l.passed);
        let overall_score = layers.iter().map(|l| l.score).sum::<f64>() / layers.len() as f64;

        // Collect critical issues
        let critical_issues: Vec<String> = layers.iter()
            .filter(|l| !l.passed)
            .flat_map(|l| l.evidence.iter().cloned())
            .collect();

        Ok(ReflectionReport {
            contract_name: contract.name.clone(),
            layers,
            overall_passed,
            overall_score,
            critical_issues,
        })
    }

    /// Reflect multiple contracts
    pub fn reflect_all(&mut self, contracts: &[Contract]) -> Result<Vec<ReflectionReport>, ReflectionError> {
        let mut reports = Vec::new();
        for contract in contracts {
            reports.push(self.reflect(contract)?);
        }
        Ok(reports)
    }

    /// Get the causal memory for inspection
    pub fn causal_memory(&self) -> &CausalMemory {
        &self.causal_memory
    }
}

impl Default for ReflectionEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redsigma_parser::Parser;

    #[test]
    fn test_consistency_check() {
        let input = r#"
            contract Test {
                invariant count >= 0;
                state {
                    count: u64,
                }
                fn increment() -> u64 {
                    let new_count = count + 1;
                    count = new_count;
                    new_count;
                }
            }
        "#;

        let contracts = Parser::parse(input).unwrap();
        let result = check_consistency(&contracts[0]);
        assert!(result.passed);
        assert!(result.score > 0.5);
    }

    #[test]
    fn test_consistency_duplicate_fields() {
        let input = r#"
            contract Test {
                state {
                    count: u64,
                    count: u64,
                }
                fn foo() -> u64 { 1; }
            }
        "#;

        let contracts = Parser::parse(input).unwrap();
        let result = check_consistency(&contracts[0]);
        assert!(!result.passed);
        assert!(result.evidence.iter().any(|e| e.contains("Duplicate state field")));
    }

    #[test]
    fn test_memory_check() {
        let input = r#"
            contract Test {
                state { x: u64 }
                fn foo() -> u64 { 1; }
            }
        "#;

        let contracts = Parser::parse(input).unwrap();
        let mut causal_memory = CausalMemory::new();
        causal_memory.record_transition("Test", "foo", 1, 2, 3, 4, None).unwrap();

        let result = check_memory(&contracts[0], &causal_memory);
        assert!(result.passed);
    }

    #[test]
    fn test_oracle_check() {
        let input = r#"
            contract Test {
                state { x: u64 }
                fn foo() -> u64 { 1; }
            }
        "#;

        let contracts = Parser::parse(input).unwrap();
        let mut ds_analyzer = DualStreamAnalyzer::new();
        let ds_report = ds_analyzer.analyze(&contracts[0]).unwrap();

        let result = check_oracle(&contracts[0], &ds_report);
        assert!(result.passed);
    }

    #[test]
    fn test_completeness_check() {
        let input = r#"
            contract Test {
                invariant x >= 0;
                state { x: u64 }
                fn foo() -> u64 { 1; }
            }
        "#;

        let contracts = Parser::parse(input).unwrap();
        let result = check_completeness(&contracts[0]);
        assert!(result.passed);
    }

    #[test]
    fn test_completeness_missing_invariants() {
        let input = r#"
            contract Test {
                state { x: u64 }
                fn foo() -> u64 { 1; }
            }
        "#;

        let contracts = Parser::parse(input).unwrap();
        let result = check_completeness(&contracts[0]);
        assert!(!result.passed);
        assert!(result.evidence.iter().any(|e| e.contains("No invariants")));
    }

    #[test]
    fn test_uncertainty_check() {
        let input = r#"
            contract Test {
                state { x: u64 }
                fn foo() -> u64 { 1; }
            }
        "#;

        let contracts = Parser::parse(input).unwrap();
        let mut sem_analyzer = SemanticAnalyzer::new();
        let sem_report = sem_analyzer.analyze(&contracts[0]).unwrap();

        let result = check_uncertainty(&contracts[0], &sem_report);
        assert!(result.passed);
    }

    #[test]
    fn test_contradiction_check() {
        let input = r#"
            contract Test {
                invariant x >= 0;
                invariant x <= -1;
                state { x: i64 }
                fn foo() -> i64 { 1; }
            }
        "#;

        let contracts = Parser::parse(input).unwrap();
        let result = check_contradiction(&contracts[0]);
        assert!(!result.passed);
        assert!(result.evidence.iter().any(|e| e.contains("contradiction")));
    }

    #[test]
    fn test_full_reflection() {
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
        let mut engine = ReflectionEngine::new();
        let report = engine.reflect(&contracts[0]).unwrap();

        assert_eq!(report.contract_name, "Counter");
        assert_eq!(report.layers.len(), 6);
        // Should pass for well-formed contract
        assert!(report.overall_passed);
        assert!(report.overall_score > 0.7);
    }

    #[test]
    fn test_layer_names() {
        let input = r#"
            contract Test {
                state { x: u64 }
                fn foo() -> u64 { 1; }
            }
        "#;

        let contracts = Parser::parse(input).unwrap();
        let mut engine = ReflectionEngine::new();
        let report = engine.reflect(&contracts[0]).unwrap();

        let layer_names: Vec<&str> = report.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(layer_names, vec!["Consistency", "Memory", "Oracle", "Completeness", "Uncertainty", "Contradiction"]);
    }
}