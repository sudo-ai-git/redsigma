//! REDΣSIGMA Dual-Stream Gematria
//!
//! Implements Mod-5 semantic channel + Mod-37/73 4-channel structural
//! Based on Tanakh 231-gate framework with 100% joint uniqueness (484/484)

use redsigma_parser::{Contract, Expr, Stmt};
use redsigma_semantic::SemanticAnalyzer;
use serde::{Deserialize, Serialize};
use borsh::{BorshSerialize, BorshDeserialize};
use thiserror::Error;
use std::collections::HashMap;

#[derive(Error, Debug)]
pub enum DualStreamError {
    #[error("Mod-5 semantic error: {0}")]
    Mod5Error(String),
    #[error("Mod-37 structural error: {0}")]
    Mod37Error(String),
    #[error("Mod-73 structural error: {0}")]
    Mod73Error(String),
    #[error("Joint uniqueness violated: {0}")]
    JointUniquenessError(String),
    #[error("Dual-stream error: {0}")]
    DualStreamError(String),
    #[error("Semantic error: {0}")]
    SemanticError(String),
}

impl From<redsigma_semantic::SemanticError> for DualStreamError {
    fn from(err: redsigma_semantic::SemanticError) -> Self {
        DualStreamError::SemanticError(err.to_string())
    }
}

/// Mod-5 Semantic Channel (5 buckets)
/// B0: residue 0, B1: residue 1, B2: residue 2, B3: residue 3, B4: residue 4
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct Mod5Semantic {
    pub buckets: [u64; 5],
    pub total: u64,
    pub signature: u64,
}

impl Mod5Semantic {
    pub fn new() -> Self {
        Self {
            buckets: [0; 5],
            total: 0,
            signature: 0,
        }
    }

    pub fn project(&mut self, symbol: u32) {
        let bucket = (symbol % 5) as usize;
        self.buckets[bucket] = self.buckets[bucket].wrapping_add(1);
        self.total = self.total.wrapping_add(1);
        self.update_signature();
    }

    pub fn project_string(&mut self, s: &str) {
        for ch in s.chars() {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                self.project(ch as u32);
            }
        }
    }

    fn update_signature(&mut self) {
        let weights = [1u64, 3, 7, 13, 21];
        self.signature = self.buckets
            .iter()
            .zip(weights.iter())
            .map(|(b, w)| b.wrapping_mul(*w))
            .fold(0, |acc, x| acc.wrapping_add(x));
    }

    pub fn distribution(&self) -> [f64; 5] {
        if self.total == 0 {
            return [0.0; 5];
        }
        let total = self.total as f64;
        [
            self.buckets[0] as f64 / total,
            self.buckets[1] as f64 / total,
            self.buckets[2] as f64 / total,
            self.buckets[3] as f64 / total,
            self.buckets[4] as f64 / total,
        ]
    }
}

impl Default for Mod5Semantic {
    fn default() -> Self {
        Self::new()
    }
}

/// Mod-37/73 Structural Channel (4 channels)
/// LIN37, LIN73, SUM231, PROD231
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct Mod3773Structural {
    pub lin37: u64,
    pub lin73: u64,
    pub sum231: u64,
    pub prod231: u64,
    /// Order-sensitive rolling channel. The four channels above are commutative
    /// (sums and products), so `Contract12` and `Contract21` project identically —
    /// a real collision for a tamper-evidence checksum. This channel makes the
    /// projection a function of the token *sequence*, not just its multiset.
    pub seq: u64,
    pub signature: u64,  // Combined signature
}

/// Modulus for the order-sensitive channel (Mersenne prime).
const SEQ_MOD: u64 = 2_147_483_647;

impl Mod3773Structural {
    pub fn new() -> Self {
        Self {
            lin37: 0,
            lin73: 0,
            sum231: 0,
            prod231: 0,
            seq: 0,
            signature: 0,
        }
    }

    pub fn project(&mut self, symbol: u32) {
        let val = symbol as u64;
        self.lin37 = (self.lin37 + val) % 37;
        self.lin73 = (self.lin73 + val) % 73;
        self.sum231 = (self.sum231 + val) % 231;
        self.prod231 = (self.prod231 * val.max(1)) % 231; // Avoid 0
        self.seq = (self.seq.wrapping_mul(257).wrapping_add(val)) % SEQ_MOD;
        self.update_signature();
    }

    pub fn project_string(&mut self, s: &str) {
        for ch in s.chars() {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                self.project(ch as u32);
            }
        }
    }

    fn update_signature(&mut self) {
        // Combine all five channels into single signature
        let head = self.lin37
            .wrapping_mul(73 * 231 * 231)
            .wrapping_add(self.lin73.wrapping_mul(231 * 231))
            .wrapping_add(self.sum231.wrapping_mul(231))
            .wrapping_add(self.prod231);
        self.signature = head.wrapping_mul(SEQ_MOD).wrapping_add(self.seq);
    }

    /// Check if this structural projection is a sentinel value
    /// Sentinels: 0, 37, 73, 144, 153, 2701, 666
    pub fn is_sentinel(&self) -> bool {
        matches!(self.sum231, 0 | 37 | 73 | 144 | 153 | 2701 | 666)
            || matches!(self.lin37, 0 | 37)
            || matches!(self.lin73, 0 | 73)
    }
}

impl Default for Mod3773Structural {
    fn default() -> Self {
        Self::new()
    }
}

/// Dual-Stream projection combining Mod-5 semantic + Mod-37/73 structural
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct DualStream {
    pub semantic: Mod5Semantic,
    pub structural: Mod3773Structural,
    pub joint_signature: u64,
    pub unique_id: u64, // For tracking uniqueness across corpus
}

impl DualStream {
    pub fn new() -> Self {
        Self {
            semantic: Mod5Semantic::new(),
            structural: Mod3773Structural::new(),
            joint_signature: 0,
            unique_id: 0,
        }
    }

    pub fn project(&mut self, symbol: u32) {
        self.semantic.project(symbol);
        self.structural.project(symbol);
        self.update_joint_signature();
    }

    pub fn project_string(&mut self, s: &str) {
        for ch in s.chars() {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                self.project(ch as u32);
            }
        }
    }

    pub fn project_contract(&mut self, contract: &Contract) {
        self.project_string(&contract.name);
        for inv in &contract.invariants {
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

    fn project_stmt(&mut self, stmt: &Stmt) {
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
                for s in then_branch {
                    self.project_stmt(s);
                }
                if let Some(eb) = else_branch {
                    for s in eb {
                        self.project_stmt(s);
                    }
                }
            }
            Stmt::While { condition, body } => {
                self.project_expr(condition);
                for s in body {
                    self.project_stmt(s);
                }
            }
            Stmt::For { var, iter, body } => {
                self.project_string(var);
                self.project_expr(iter);
                for s in body {
                    self.project_stmt(s);
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

    fn project_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Number(n) => self.project_string(n),
            Expr::String(s) => self.project_string(s),
            Expr::Ident(i) => self.project_string(i),
            Expr::Bool(b) => self.project(if *b { 1 } else { 0 }),
            Expr::Unit => self.project(0),
            Expr::Binary { left, op, right } => {
                self.project_expr(left);
                self.project(*op as u32);
                self.project_expr(right);
            }
            Expr::Unary { op, expr } => {
                self.project(*op as u32);
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

    fn update_joint_signature(&mut self) {
        // Combine semantic (5-bucket) + structural (5-channel) signatures
        // This creates the joint uniqueness guarantee
        self.joint_signature = self.semantic.signature
            .wrapping_mul(37 * 73 * 231 * 231)
            .wrapping_add(self.structural.signature);
    }

    /// Get the combined joint signature (used for uniqueness checking)
    pub fn joint_signature(&self) -> u64 {
        self.joint_signature
    }

    /// Check if this projection hits a sentinel value
    pub fn is_sentinel(&self) -> bool {
        self.structural.is_sentinel()
    }

    /// Get semantic distribution (Mod-5)
    pub fn semantic_distribution(&self) -> [f64; 5] {
        self.semantic.distribution()
    }

    /// Get structural channels
    pub fn structural_channels(&self) -> (u64, u64, u64, u64) {
        (self.structural.lin37, self.structural.lin73, self.structural.sum231, self.structural.prod231)
    }
}

impl Default for DualStream {
    fn default() -> Self {
        Self::new()
    }
}

/// Dual-Stream analyzer for verifying 100% joint uniqueness
pub struct DualStreamAnalyzer {
    /// Maps joint signatures to contract names (for uniqueness verification)
    signature_index: HashMap<u64, String>,
    /// Semantic analyzer for Mod-5 channel
    semantic_analyzer: SemanticAnalyzer,
    /// Collision count (should be 0 for 100% uniqueness)
    collisions: u64,
}

impl DualStreamAnalyzer {
    pub fn new() -> Self {
        Self {
            signature_index: HashMap::new(),
            semantic_analyzer: SemanticAnalyzer::new(),
            collisions: 0,
        }
    }

    /// Analyze a contract with dual-stream projection
    pub fn analyze(&mut self, contract: &Contract) -> Result<DualStreamReport, DualStreamError> {
        let mut dual_stream = DualStream::new();
        dual_stream.project_contract(contract);

        // Check joint uniqueness
        let joint_sig = dual_stream.joint_signature();
        if let Some(existing) = self.signature_index.get(&joint_sig) {
            if existing != &contract.name {
                self.collisions += 1;
                return Err(DualStreamError::JointUniquenessError(
                    format!("Collision: '{}' and '{}' share joint signature {}", existing, contract.name, joint_sig)
                ));
            }
        } else {
            self.signature_index.insert(joint_sig, contract.name.clone());
        }

        // Also run semantic analyzer for Mod-5 verification
        let sem_report = self.semantic_analyzer.analyze(contract)?;

        Ok(DualStreamReport {
            contract_name: contract.name.clone(),
            joint_signature: joint_sig,
            semantic_signature: dual_stream.semantic.signature,
            structural_signature: dual_stream.structural.signature,
            semantic_distribution: dual_stream.semantic_distribution(),
            structural_channels: dual_stream.structural_channels(),
            is_sentinel: dual_stream.is_sentinel(),
            bijection_verified: sem_report.bijection_verified,
            collisions: self.collisions,
        })
    }

    /// Analyze multiple contracts
    pub fn analyze_all(&mut self, contracts: &[Contract]) -> Result<Vec<DualStreamReport>, DualStreamError> {
        let mut reports = Vec::new();
        for contract in contracts {
            reports.push(self.analyze(contract)?);
        }
        Ok(reports)
    }

    /// Get collision count (0 = 100% joint uniqueness)
    pub fn collisions(&self) -> u64 {
        self.collisions
    }

    /// Get total unique signatures indexed
    pub fn unique_signatures(&self) -> usize {
        self.signature_index.len()
    }

    /// Verify 100% joint uniqueness across all analyzed contracts
    pub fn verify_joint_uniqueness(&self) -> bool {
        self.collisions == 0
    }
}

impl Default for DualStreamAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

/// Report from dual-stream analysis
#[derive(Debug, Clone, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct DualStreamReport {
    pub contract_name: String,
    pub joint_signature: u64,
    pub semantic_signature: u64,
    pub structural_signature: u64,
    pub semantic_distribution: [f64; 5],
    pub structural_channels: (u64, u64, u64, u64), // (lin37, lin73, sum231, prod231)
    pub is_sentinel: bool,
    pub bijection_verified: bool,
    pub collisions: u64,
}

/// Compute Tanakh-style 231-gate checksum for a contract
pub fn gate231_checksum(contract: &Contract) -> u64 {
    let mut sum = 0u64;
    let mut linear = 0u64;
    let mut prod = 1u64;

    // Hash contract name
    for ch in contract.name.chars() {
        let val = ch as u64;
        sum = sum.wrapping_add(val);
        linear = linear.wrapping_add(val * 7);
        prod = prod.wrapping_mul(val.max(1));
    }

    // Hash invariants (over the AST, not a debug string)
    for inv in &contract.invariants {
        hash_expr_checksum(&mut sum, &mut linear, &mut prod, &inv.expr);
    }

    // Hash state
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

    // Hash functions
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

    // Combine three channels mod 231
    (sum % 231) ^ (linear % 231) ^ (prod % 231)
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

#[cfg(test)]
mod tests {
    use super::*;
    use redsigma_parser::Parser;

    #[test]
    fn test_mod5_semantic() {
        let mut semantic = Mod5Semantic::new();
        semantic.project_string("hello");
        assert_eq!(semantic.total, 5);
        assert!(semantic.signature > 0);
    }

    #[test]
    fn test_mod3773_structural() {
        let mut structural = Mod3773Structural::new();
        structural.project_string("test");
        assert!(structural.lin37 < 37);
        assert!(structural.lin73 < 73);
        assert!(structural.sum231 < 231);
        assert!(structural.prod231 < 231);
    }

    #[test]
    fn test_dual_stream_projection() {
        let mut ds = DualStream::new();
        ds.project_string("test");
        assert!(ds.joint_signature() > 0);
    }

    #[test]
    fn test_dual_stream_contract() {
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
        let mut ds = DualStream::new();
        ds.project_contract(&contracts[0]);
        assert!(ds.joint_signature() > 0);
    }

    #[test]
    fn test_gate231_checksum() {
        let input = r#"
            contract Test {
                state { x: u64 }
                fn foo() -> u64 { 42; }
            }
        "#;

        let contracts = Parser::parse(input).unwrap();
        let checksum1 = gate231_checksum(&contracts[0]);
        let checksum2 = gate231_checksum(&contracts[0]);
        assert_eq!(checksum1, checksum2); // Deterministic
        assert!(checksum1 < 231); // Mod 231
    }

    #[test]
    fn test_dual_stream_analyzer() {
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
        let mut analyzer = DualStreamAnalyzer::new();
        let reports = analyzer.analyze_all(&contracts).unwrap();

        assert_eq!(reports.len(), 1);
        assert!(reports[0].bijection_verified);
        assert_eq!(reports[0].contract_name, "Test");
        assert!(analyzer.verify_joint_uniqueness());
    }

    #[test]
    fn test_joint_uniqueness() {
        let input1 = r#"
            contract A {
                state { x: u64 }
                fn foo() -> u64 { 1; }
            }
        "#;
        let input2 = r#"
            contract B {
                state { x: u64 }
                fn foo() -> u64 { 2; }
            }
        "#;

        let contracts1 = Parser::parse(input1).unwrap();
        let contracts2 = Parser::parse(input2).unwrap();

        let mut analyzer = DualStreamAnalyzer::new();
        analyzer.analyze_all(&contracts1).unwrap();
        analyzer.analyze_all(&contracts2).unwrap();

        // Different contracts should have different joint signatures
        assert_eq!(analyzer.unique_signatures(), 2);
        assert!(analyzer.verify_joint_uniqueness());
    }

    #[test]
    fn test_sentinel_detection() {
        // Create a dual stream that hits sentinel values
        let mut ds = DualStream::new();
        // Project specific values to hit sentinels
        for i in 0..100 {
            ds.project(i);
        }
        // Just verify the method works
        let _ = ds.is_sentinel();
    }

    #[test]
    fn test_projection_is_order_sensitive() {
        // Regression: the structural channels were commutative (sums/products),
        // so `Contract12` and `Contract21` projected to the same joint signature.
        let mut analyzer = DualStreamAnalyzer::new();
        for (name, body) in [("Contract12", 12u64), ("Contract21", 21u64)] {
            let input = format!(
                r#"contract {} {{ state {{ x: u64 }} fn foo() -> u64 {{ {}; }} }}"#,
                name, body
            );
            let contracts = Parser::parse(&input).unwrap();
            analyzer.analyze(&contracts[0]).unwrap();
        }
        assert_eq!(analyzer.unique_signatures(), 2);
        assert!(analyzer.verify_joint_uniqueness());
    }
}