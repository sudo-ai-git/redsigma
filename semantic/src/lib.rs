//! REDΣSIGMA Semantic Analysis
//!
//! Implements the 5-bucket SemanticSpace projection (Mod-5 semantic)
//! and BijectiveVerifier for forward determinism + inverse preimage verification.

use std::hash::Hasher;
use redsigma_parser::{Contract, Expr, Stmt};
use serde::{Deserialize, Serialize};
use borsh::{BorshSerialize, BorshDeserialize};
use thiserror::Error;
use indexmap::IndexMap;

#[derive(Error, Debug)]
pub enum SemanticError {
    #[error("Type error: {0}")]
    TypeError(String),
    #[error("Verification error: {0}")]
    VerificationError(String),
    #[error("Invariant violation: {0}")]
    InvariantViolation(String),
    #[error("Semantic error: {0}")]
    SemanticError(String),
}

/// 5-bucket SemanticSpace projection using Mod-5 arithmetic
/// Based on Tanakh 231-gate framework: Mod-5 semantic channel
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct SemanticSpace {
    /// 5 buckets representing semantic residue classes modulo 5
    pub buckets: [u64; 5],
    /// Total symbols processed
    pub total_symbols: u64,
    /// Projection signature (deterministic hash)
    pub signature: u64,
}

impl SemanticSpace {
    /// Create a new empty SemanticSpace
    pub fn new() -> Self {
        Self {
            buckets: [0; 5],
            total_symbols: 0,
            signature: 0,
        }
    }

    /// Project a symbol into the 5-bucket space
    /// Uses the Hebrew letter ordinal value mod 5 (Mod-5 semantic channel)
    pub fn project_symbol(&mut self, symbol: u32) {
        let bucket = (symbol % 5) as usize;
        self.buckets[bucket] = self.buckets[bucket].wrapping_add(1);
        self.total_symbols = self.total_symbols.wrapping_add(1);
        self.update_signature();
    }

    /// Project a string (contract source) into semantic space
    pub fn project_string(&mut self, s: &str) {
        for ch in s.chars() {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                let val = ch as u32;
                self.project_symbol(val);
            }
        }
    }

    /// Project an AST expression into semantic space
    pub fn project_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Number(n) => self.project_string(n),
            Expr::String(s) => self.project_string(s),
            Expr::Ident(i) => self.project_string(i),
            Expr::Bool(b) => self.project_symbol(if *b { 1 } else { 0 }),
            Expr::Unit => self.project_symbol(0),
            Expr::Binary { left, op, right } => {
                self.project_expr(left);
                self.project_symbol(*op as u32);
                self.project_expr(right);
            }
            Expr::Unary { op, expr } => {
                self.project_symbol(*op as u32);
                self.project_expr(expr);
            }
            Expr::Range { from, to } => {
                self.project_expr(from);
                self.project_expr(to);
            }
            Expr::Call { func, args } => {
                self.project_string(func);
                for arg in args {
                    self.project_expr(arg);
                }
            }
            Expr::MethodCall { receiver, method, args } => {
                self.project_expr(receiver);
                self.project_string(method);
                for arg in args {
                    self.project_expr(arg);
                }
            }
            Expr::FieldAccess { receiver, field } => {
                self.project_expr(receiver);
                self.project_string(field);
            }
            Expr::Index { array, index } => {
                self.project_expr(array);
                self.project_expr(index);
            }
            Expr::Array(items) => {
                for item in items {
                    self.project_expr(item);
                }
            }
            Expr::Struct { name, fields } => {
                self.project_string(name);
                for (k, v) in fields {
                    self.project_string(k);
                    self.project_expr(v);
                }
            }
            Expr::Block(stmts) => {
                for stmt in stmts {
                    self.project_stmt(stmt);
                }
            }
            Expr::If { condition, then_branch, else_branch } => {
                self.project_expr(condition);
                self.project_expr(then_branch);
                if let Some(eb) = else_branch {
                    self.project_expr(eb);
                }
            }
            Expr::Match { expr, arms } => {
                self.project_expr(expr);
                for (pat, arm_expr) in arms {
                    self.project_string(pat);
                    self.project_expr(arm_expr);
                }
            }
        }
    }

    /// Project a statement into semantic space
    pub fn project_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let { name, value, .. } => {
                self.project_string(name);
                self.project_expr(value);
            }
            Stmt::Const { name, value, .. } => {
                self.project_string(name);
                self.project_expr(value);
            }
            Stmt::Expr(expr) => self.project_expr(expr),
            Stmt::Return(expr) => {
                if let Some(e) = expr {
                    self.project_expr(e);
                }
            }
            Stmt::If { condition, then_branch, else_branch } => {
                self.project_expr(condition);
                for stmt in then_branch {
                    self.project_stmt(stmt);
                }
                if let Some(eb) = else_branch {
                    for stmt in eb {
                        self.project_stmt(stmt);
                    }
                }
            }
            Stmt::While { condition, body } => {
                self.project_expr(condition);
                for stmt in body {
                    self.project_stmt(stmt);
                }
            }
            Stmt::For { var, iter, body } => {
                self.project_string(var);
                self.project_expr(iter);
                for stmt in body {
                    self.project_stmt(stmt);
                }
            }
            Stmt::Match { expr, arms } => {
                self.project_expr(expr);
                for (pat, arm_expr) in arms {
                    self.project_string(pat);
                    self.project_expr(arm_expr);
                }
            }
            Stmt::Break | Stmt::Continue => {}
        }
    }

    /// Project a full contract into semantic space
    pub fn project_contract(&mut self, contract: &Contract) {
        self.project_string(&contract.name);
        for inv in &contract.invariants {
            // Parse invariant expression string back to project
            self.project_expr(&inv.expr);
        }
        for (name, ty) in &contract.state.fields {
            self.project_string(name);
            self.project_string(&ty.name);
        }
        for func in &contract.functions {
            self.project_string(&func.name);
            for (param_name, param_ty) in &func.params {
                self.project_string(param_name);
                self.project_string(&param_ty.name);
            }
            self.project_string(&func.return_type.name);
            for stmt in &func.body {
                self.project_stmt(stmt);
            }
        }
    }

    fn update_signature(&mut self) {
        // Deterministic signature: weighted sum of buckets
        let weights = [1u64, 3, 7, 13, 21]; // Primes for good distribution
        self.signature = self.buckets
            .iter()
            .zip(weights.iter())
            .map(|(b, w)| b.wrapping_mul(*w))
            .fold(0, |acc, x| acc.wrapping_add(x));
    }

    /// Get the 5-bucket distribution as ratios (for comparison)
    pub fn distribution(&self) -> [f64; 5] {
        if self.total_symbols == 0 {
            return [0.0; 5];
        }
        let total = self.total_symbols as f64;
        [
            self.buckets[0] as f64 / total,
            self.buckets[1] as f64 / total,
            self.buckets[2] as f64 / total,
            self.buckets[3] as f64 / total,
            self.buckets[4] as f64 / total,
        ]
    }

    /// Check if two semantic spaces are equivalent (for verification)
    pub fn is_equivalent(&self, other: &SemanticSpace) -> bool {
        self.signature == other.signature && self.buckets == other.buckets
    }
}

impl Default for SemanticSpace {
    fn default() -> Self {
        Self::new()
    }
}

/// BijectiveVerifier - verifies forward determinism and inverse preimage existence
/// Ensures every semantic projection has a unique preimage (bijectivity)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BijectiveVerifier {
    /// Forward mapping: input hash -> semantic signature
    forward_map: IndexMap<u64, u64>,
    /// Inverse mapping: semantic signature -> input hash (for preimage check)
    inverse_map: IndexMap<u64, u64>,
    /// Collision count (should be 0 for perfect bijection)
    collisions: u64,
}

impl BijectiveVerifier {
    pub fn new() -> Self {
        Self {
            forward_map: IndexMap::new(),
            inverse_map: IndexMap::new(),
            collisions: 0,
        }
    }

    /// Verify forward determinism: same input always produces same output
    pub fn verify_forward(&mut self, input_hash: u64, semantic_sig: u64) -> bool {
        match self.forward_map.get(&input_hash) {
            Some(&existing) => existing == semantic_sig,
            None => {
                self.forward_map.insert(input_hash, semantic_sig);
                true
            }
        }
    }

    /// Verify inverse preimage: every output has exactly one input
    pub fn verify_inverse(&mut self, input_hash: u64, semantic_sig: u64) -> bool {
        match self.inverse_map.get(&semantic_sig) {
            Some(&existing) => {
                if existing != input_hash {
                    self.collisions += 1;
                    false
                } else {
                    true
                }
            }
            None => {
                self.inverse_map.insert(semantic_sig, input_hash);
                true
            }
        }
    }

    /// Full bijectivity check for a projection
    pub fn verify_bijection(&mut self, input_hash: u64, semantic_sig: u64) -> Result<(), SemanticError> {
        let forward_ok = self.verify_forward(input_hash, semantic_sig);
        let inverse_ok = self.verify_inverse(input_hash, semantic_sig);

        if !forward_ok {
            return Err(SemanticError::VerificationError(
                "Forward determinism violated: same input produces different output".to_string()
            ));
        }
        if !inverse_ok {
            return Err(SemanticError::VerificationError(
                "Inverse preimage violated: output has multiple preimages".to_string()
            ));
        }
        Ok(())
    }

    /// Get collision count (0 = perfect bijection)
    pub fn collisions(&self) -> u64 {
        self.collisions
    }

    /// Get mapping sizes
    pub fn forward_size(&self) -> usize {
        self.forward_map.len()
    }

    pub fn inverse_size(&self) -> usize {
        self.inverse_map.len()
    }
}

impl Default for BijectiveVerifier {
    fn default() -> Self {
        Self::new()
    }
}

/// Semantic analyzer - runs SemanticSpace projection and BijectiveVerifier on contracts
pub struct SemanticAnalyzer {
    verifier: BijectiveVerifier,
    contract_hashes: IndexMap<String, u64>,
}

impl SemanticAnalyzer {
    pub fn new() -> Self {
        Self {
            verifier: BijectiveVerifier::new(),
            contract_hashes: IndexMap::new(),
        }
    }

    /// Analyze a contract: project to semantic space and verify bijection
    pub fn analyze(&mut self, contract: &Contract) -> Result<SemanticReport, SemanticError> {
        // Compute input hash (deterministic from contract structure)
        let input_hash = self.compute_contract_hash(contract);

        // Project to semantic space
        let mut space = SemanticSpace::new();
        space.project_contract(contract);

        // Verify bijection. A collision here is a *reported* property, not a
        // fatal one: the Mod-5 projection is deliberately coarse (5 buckets), so
        // distinct contracts routinely share a semantic signature. Cross-contract
        // uniqueness is the DualStream analyzer's joint-signature gate, not this
        // one. `verify_bijection` still returns Err for the primitive (unit-tested);
        // we count it instead of aborting the whole analysis.
        let bijection_verified = self.verifier.verify_bijection(input_hash, space.signature).is_ok();

        // Store for cross-contract verification
        self.contract_hashes.insert(contract.name.clone(), input_hash);

        Ok(SemanticReport {
            contract_name: contract.name.clone(),
            input_hash,
            semantic_signature: space.signature,
            bucket_distribution: space.distribution(),
            total_symbols: space.total_symbols,
            bijection_verified,
            collisions: self.verifier.collisions(),
        })
    }

    /// Analyze multiple contracts together (for cross-contract verification)
    pub fn analyze_all(&mut self, contracts: &[Contract]) -> Result<Vec<SemanticReport>, SemanticError> {
        let mut reports = Vec::new();
        for contract in contracts {
            reports.push(self.analyze(contract)?);
        }
        Ok(reports)
    }

    fn compute_contract_hash(&self, contract: &Contract) -> u64 {
        // Simple deterministic hash based on contract structure
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        contract.name.hash(&mut hasher);
        for inv in &contract.invariants {
            Self::hash_expr(&mut hasher, &inv.expr);
        }
        for (name, ty) in &contract.state.fields {
            name.hash(&mut hasher);
            ty.name.hash(&mut hasher);
        }
        for func in &contract.functions {
            func.name.hash(&mut hasher);
            for (param_name, param_ty) in &func.params {
                param_name.hash(&mut hasher);
                param_ty.name.hash(&mut hasher);
            }
            func.return_type.name.hash(&mut hasher);
            // Hash function body structure
            for stmt in &func.body {
                Self::hash_stmt(&mut hasher, stmt);
            }
        }
        hasher.finish()
    }

    fn hash_stmt(hasher: &mut impl Hasher, stmt: &Stmt) {
        use std::hash::Hash;
        match stmt {
            Stmt::Let { name, value, mutable, .. } => {
                name.hash(hasher);
                mutable.hash(hasher);
                Self::hash_expr(hasher, value);
            }
            Stmt::Const { name, value, .. } => {
                name.hash(hasher);
                Self::hash_expr(hasher, value);
            }
            Stmt::Expr(expr) => Self::hash_expr(hasher, expr),
            Stmt::Return(expr) => {
                if let Some(e) = expr {
                    Self::hash_expr(hasher, e);
                }
            }
            Stmt::If { condition, then_branch, else_branch } => {
                Self::hash_expr(hasher, condition);
                for s in then_branch {
                    Self::hash_stmt(hasher, s);
                }
                if let Some(eb) = else_branch {
                    for s in eb {
                        Self::hash_stmt(hasher, s);
                    }
                }
            }
            Stmt::While { condition, body } => {
                Self::hash_expr(hasher, condition);
                for s in body {
                    Self::hash_stmt(hasher, s);
                }
            }
            Stmt::For { var, iter, body } => {
                var.hash(hasher);
                Self::hash_expr(hasher, iter);
                for s in body {
                    Self::hash_stmt(hasher, s);
                }
            }
            Stmt::Match { expr, arms } => {
                Self::hash_expr(hasher, expr);
                for (pat, arm_expr) in arms {
                    pat.hash(hasher);
                    Self::hash_expr(hasher, arm_expr);
                }
            }
            Stmt::Break | Stmt::Continue => {}
        }
    }

    fn hash_expr(hasher: &mut impl Hasher, expr: &Expr) {
        use std::hash::Hash;
        match expr {
            Expr::Number(n) => n.hash(hasher),
            Expr::String(s) => s.hash(hasher),
            Expr::Ident(i) => i.hash(hasher),
            Expr::Bool(b) => b.hash(hasher),
            Expr::Unit => 0u8.hash(hasher),
            Expr::Binary { left, op, right } => {
                Self::hash_expr(hasher, left);
                (*op as u8).hash(hasher);
                Self::hash_expr(hasher, right);
            }
            Expr::Unary { op, expr } => {
                (*op as u8).hash(hasher);
                Self::hash_expr(hasher, expr);
            }
            Expr::Range { from, to } => {
                Self::hash_expr(hasher, from);
                Self::hash_expr(hasher, to);
            }
            Expr::Call { func, args } => {
                func.hash(hasher);
                for arg in args {
                    Self::hash_expr(hasher, arg);
                }
            }
            Expr::MethodCall { receiver, method, args } => {
                Self::hash_expr(hasher, receiver);
                method.hash(hasher);
                for arg in args {
                    Self::hash_expr(hasher, arg);
                }
            }
            Expr::FieldAccess { receiver, field } => {
                Self::hash_expr(hasher, receiver);
                field.hash(hasher);
            }
            Expr::Index { array, index } => {
                Self::hash_expr(hasher, array);
                Self::hash_expr(hasher, index);
            }
            Expr::Array(items) => {
                for item in items {
                    Self::hash_expr(hasher, item);
                }
            }
            Expr::Struct { name, fields } => {
                name.hash(hasher);
                for (k, v) in fields {
                    k.hash(hasher);
                    Self::hash_expr(hasher, v);
                }
            }
            Expr::Block(stmts) => {
                for stmt in stmts {
                    Self::hash_stmt(hasher, stmt);
                }
            }
            Expr::If { condition, then_branch, else_branch } => {
                Self::hash_expr(hasher, condition);
                Self::hash_expr(hasher, then_branch);
                if let Some(eb) = else_branch {
                    Self::hash_expr(hasher, eb);
                }
            }
            Expr::Match { expr, arms } => {
                Self::hash_expr(hasher, expr);
                for (pat, arm_expr) in arms {
                    pat.hash(hasher);
                    Self::hash_expr(hasher, arm_expr);
                }
            }
        }
    }
}

impl Default for SemanticAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

/// Report from semantic analysis
#[derive(Debug, Clone, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct SemanticReport {
    pub contract_name: String,
    pub input_hash: u64,
    pub semantic_signature: u64,
    pub bucket_distribution: [f64; 5],
    pub total_symbols: u64,
    pub bijection_verified: bool,
    pub collisions: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use redsigma_parser::Parser;

    #[test]
    fn test_semantic_space_projection() {
        let mut space = SemanticSpace::new();
        space.project_string("hello");
        assert_eq!(space.total_symbols, 5);
        assert!(space.signature > 0);
    }

    #[test]
    fn test_semantic_space_equivalence() {
        let mut space1 = SemanticSpace::new();
        space1.project_string("test");

        let mut space2 = SemanticSpace::new();
        space2.project_string("test");

        assert!(space1.is_equivalent(&space2));
    }

    #[test]
    fn test_bijective_verifier() {
        let mut verifier = BijectiveVerifier::new();

        // First mapping should succeed
        assert!(verifier.verify_bijection(1, 100).is_ok());
        assert_eq!(verifier.collisions(), 0);

        // Same input -> same output should succeed
        assert!(verifier.verify_bijection(1, 100).is_ok());
        assert_eq!(verifier.collisions(), 0);

        // Different input -> same output should fail (collision)
        assert!(verifier.verify_bijection(2, 100).is_err());
        assert_eq!(verifier.collisions(), 1);
    }

    #[test]
    fn test_semantic_analyzer() {
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
        let mut analyzer = SemanticAnalyzer::new();
        let reports = analyzer.analyze_all(&contracts).unwrap();

        assert_eq!(reports.len(), 1);
        assert!(reports[0].bijection_verified);
        assert_eq!(reports[0].contract_name, "Test");
        assert!(reports[0].total_symbols > 0);
    }

    #[test]
    fn test_contract_hash_determinism() {
        let input = r#"
            contract Test {
                state { x: u64 }
                fn foo() -> u64 { 42; }
            }
        "#;

        let contracts = Parser::parse(input).unwrap();
        let mut analyzer1 = SemanticAnalyzer::new();
        let mut analyzer2 = SemanticAnalyzer::new();

        let report1 = analyzer1.analyze(&contracts[0]).unwrap();
        let report2 = analyzer2.analyze(&contracts[0]).unwrap();

        assert_eq!(report1.input_hash, report2.input_hash);
        assert_eq!(report1.semantic_signature, report2.semantic_signature);
    }
}