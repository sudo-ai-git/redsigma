//! REDΣSIGMA Causal Memory
//!
//! Implements parent→child causal chains for every transaction.
//! Each state transition creates a causal link from parent state to child state.

use redsigma_parser::{Contract, Expr, Stmt, Function};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

#[derive(Error, Debug)]
pub enum CausalError {
    #[error("Causal chain broken: {0}")]
    ChainBroken(String),
    #[error("Invalid parent reference: {0}")]
    InvalidParent(String),
    #[error("Counterfactual error: {0}")]
    CounterfactualError(String),
    #[error("Causal error: {0}")]
    CausalError(String),
}

/// A causal node representing a state transition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CausalNode {
    /// Unique identifier for this node
    pub id: u64,
    /// Hash of the parent node (0 for genesis)
    pub parent_id: u64,
    /// Contract name this transition belongs to
    pub contract_name: String,
    /// Function that triggered this transition
    pub function_name: String,
    /// Pre-state semantic signature
    pub pre_state_sig: u64,
    /// Post-state semantic signature
    pub post_state_sig: u64,
    /// 231-gate checksum of this transition
    pub gate231_checksum: u64,
    /// Timestamp / logical clock
    pub timestamp: u64,
    /// Transaction hash
    pub tx_hash: u64,
}

/// Causal memory maintaining parent→child chains
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CausalMemory {
    /// All causal nodes indexed by ID
    nodes: HashMap<u64, CausalNode>,
    /// Children index: parent_id -> vec of child_ids
    children: HashMap<u64, Vec<u64>>,
    /// Contract -> list of node IDs
    contract_index: HashMap<String, Vec<u64>>,
    /// Next available node ID
    next_id: u64,
    /// Logical clock for ordering
    logical_clock: u64,
}

impl CausalMemory {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            children: HashMap::new(),
            contract_index: HashMap::new(),
            next_id: 1, // 0 reserved for genesis
            logical_clock: 0,
        }
    }

    /// Record a state transition and create causal link
    pub fn record_transition(
        &mut self,
        contract_name: &str,
        function_name: &str,
        pre_state_sig: u64,
        post_state_sig: u64,
        gate231_checksum: u64,
        tx_hash: u64,
        parent_id: Option<u64>,
    ) -> Result<u64, CausalError> {
        self.logical_clock += 1;
        let id = self.next_id;
        self.next_id += 1;

        let parent_id = parent_id.unwrap_or(0);

        // Verify parent exists (except for genesis)
        if parent_id != 0 && !self.nodes.contains_key(&parent_id) {
            return Err(CausalError::InvalidParent(
                format!("Parent node {} does not exist", parent_id)
            ));
        }

        let node = CausalNode {
            id,
            parent_id,
            contract_name: contract_name.to_string(),
            function_name: function_name.to_string(),
            pre_state_sig,
            post_state_sig,
            gate231_checksum,
            timestamp: self.logical_clock,
            tx_hash,
        };

        // Add to indices
        self.nodes.insert(id, node);
        self.children.entry(parent_id).or_default().push(id);
        self.contract_index.entry(contract_name.to_string()).or_default().push(id);

        Ok(id)
    }

    /// Get the full causal chain from a node back to genesis
    pub fn get_causal_chain(&self, node_id: u64) -> Result<Vec<&CausalNode>, CausalError> {
        let mut chain = Vec::new();
        let mut current_id = node_id;

        while current_id != 0 {
            let node = self.nodes.get(&current_id)
                .ok_or_else(|| CausalError::ChainBroken(
                    format!("Node {} not found in chain", current_id)
                ))?;
            chain.push(node);
            current_id = node.parent_id;
        }

        chain.reverse(); // Genesis -> ... -> node
        Ok(chain)
    }

    /// Get all children of a node (direct descendants)
    pub fn get_children(&self, node_id: u64) -> Vec<&CausalNode> {
        self.children.get(&node_id)
            .map(|ids| ids.iter().filter_map(|id| self.nodes.get(id)).collect())
            .unwrap_or_default()
    }

    /// Get all descendants of a node (recursive)
    pub fn get_descendants(&self, node_id: u64) -> Vec<&CausalNode> {
        let mut descendants = Vec::new();
        let mut stack = vec![node_id];

        while let Some(current) = stack.pop() {
            if let Some(children) = self.children.get(&current) {
                for child_id in children {
                    if let Some(child) = self.nodes.get(child_id) {
                        descendants.push(child);
                        stack.push(*child_id);
                    }
                }
            }
        }
        descendants
    }

    /// Get all transitions for a contract
    pub fn get_contract_history(&self, contract_name: &str) -> Vec<&CausalNode> {
        self.contract_index.get(contract_name)
            .map(|ids| ids.iter().filter_map(|id| self.nodes.get(id)).collect())
            .unwrap_or_default()
    }

    /// Verify causal integrity: every node's parent exists and chain is unbroken
    pub fn verify_integrity(&self) -> Result<(), CausalError> {
        for (id, node) in &self.nodes {
            if node.parent_id != 0 && !self.nodes.contains_key(&node.parent_id) {
                return Err(CausalError::ChainBroken(
                    format!("Node {} references missing parent {}", id, node.parent_id)
                ));
            }

            // Verify children index consistency
            if let Some(children) = self.children.get(&node.parent_id) {
                if !children.contains(id) {
                    return Err(CausalError::ChainBroken(
                        format!("Children index inconsistent for node {}", id)
                    ));
                }
            }
        }
        Ok(())
    }

    /// Counterfactual query: what if a transition had different post-state?
    /// Returns the causal chain that would result
    pub fn counterfactual(
        &self,
        node_id: u64,
        alt_post_state_sig: u64,
    ) -> Result<Vec<CausalNode>, CausalError> {
        let node = self.nodes.get(&node_id)
            .ok_or_else(|| CausalError::CounterfactualError(
                format!("Node {} not found", node_id)
            ))?;

        // Build counterfactual chain from this node forward
        let mut chain = Vec::new();
        chain.push(CausalNode {
            id: u64::MAX, // Marker for counterfactual
            parent_id: node.id,
            contract_name: node.contract_name.clone(),
            function_name: format!("{}_counterfactual", node.function_name),
            pre_state_sig: node.post_state_sig,
            post_state_sig: alt_post_state_sig,
            gate231_checksum: 0, // Would need recomputation
            timestamp: self.logical_clock + 1,
            tx_hash: 0,
        });

        // Add all descendants with adjusted pre-state
        let descendants = self.get_descendants(node_id);
        for desc in descendants {
            chain.push(CausalNode {
                id: u64::MAX - 1,
                parent_id: chain.last().unwrap().id,
                contract_name: desc.contract_name.clone(),
                function_name: format!("{}_cf", desc.function_name),
                pre_state_sig: chain.last().unwrap().post_state_sig,
                post_state_sig: desc.post_state_sig,
                gate231_checksum: 0,
                timestamp: self.logical_clock + 2,
                tx_hash: 0,
            });
        }

        Ok(chain)
    }

    /// Compute causal distance between two nodes (LCA-based)
    pub fn causal_distance(&self, node_a: u64, node_b: u64) -> Option<u64> {
        let chain_a = self.get_causal_chain(node_a).ok()?;
        let chain_b = self.get_causal_chain(node_b).ok()?;

        // Find lowest common ancestor
        let mut lca_depth = 0;
        for (a, b) in chain_a.iter().zip(chain_b.iter()) {
            if a.id == b.id {
                lca_depth += 1;
            } else {
                break;
            }
        }

        let dist_a = chain_a.len() as u64 - lca_depth;
        let dist_b = chain_b.len() as u64 - lca_depth;
        Some(dist_a + dist_b)
    }

    /// Get node by ID
    pub fn get_node(&self, id: u64) -> Option<&CausalNode> {
        self.nodes.get(&id)
    }

    /// Total nodes count
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Iterate over all nodes
    pub fn iter(&self) -> impl Iterator<Item = &CausalNode> {
        self.nodes.values()
    }
}

impl Default for CausalMemory {
    fn default() -> Self {
        Self::new()
    }
}

/// Hash a contract for causal tracking
pub fn hash_contract(contract: &Contract) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    let mut hasher = DefaultHasher::new();
    contract.name.hash(&mut hasher);
    for func in &contract.functions {
        func.name.hash(&mut hasher);
        for stmt in &func.body {
            hash_stmt(&mut hasher, stmt);
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
            hash_expr(hasher, value);
        }
        Stmt::Const { name, value, .. } => {
            name.hash(hasher);
            hash_expr(hasher, value);
        }
        Stmt::Expr(expr) => hash_expr(hasher, expr),
        Stmt::Return(expr) => {
            if let Some(e) = expr {
                hash_expr(hasher, e);
            }
        }
        Stmt::If { condition, then_branch, else_branch } => {
            hash_expr(hasher, condition);
            for s in then_branch {
                hash_stmt(hasher, s);
            }
            if let Some(eb) = else_branch {
                for s in eb {
                    hash_stmt(hasher, s);
                }
            }
        }
        Stmt::While { condition, body } => {
            hash_expr(hasher, condition);
            for s in body {
                hash_stmt(hasher, s);
            }
        }
        Stmt::For { var, iter, body } => {
            var.hash(hasher);
            hash_expr(hasher, iter);
            for s in body {
                hash_stmt(hasher, s);
            }
        }
        Stmt::Match { expr, arms } => {
            hash_expr(hasher, expr);
            for (pat, arm_expr) in arms {
                pat.hash(hasher);
                hash_expr(hasher, arm_expr);
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
            hash_expr(hasher, left);
            (*op as u8).hash(hasher);
            hash_expr(hasher, right);
        }
        Expr::Unary { op, expr } => {
            (*op as u8).hash(hasher);
            hash_expr(hasher, expr);
        }
        Expr::Range { from, to } => {
            hash_expr(hasher, from);
            hash_expr(hasher, to);
        }
        Expr::Call { func, args } => {
            func.hash(hasher);
            for arg in args {
                hash_expr(hasher, arg);
            }
        }
        Expr::MethodCall { receiver, method, args } => {
            hash_expr(hasher, receiver);
            method.hash(hasher);
            for arg in args {
                hash_expr(hasher, arg);
            }
        }
        Expr::FieldAccess { receiver, field } => {
            hash_expr(hasher, receiver);
            field.hash(hasher);
        }
        Expr::Index { array, index } => {
            hash_expr(hasher, array);
            hash_expr(hasher, index);
        }
        Expr::Array(items) => {
            for item in items {
                hash_expr(hasher, item);
            }
        }
        Expr::Struct { name, fields } => {
            name.hash(hasher);
            for (k, v) in fields {
                k.hash(hasher);
                hash_expr(hasher, v);
            }
        }
        Expr::Block(stmts) => {
            for stmt in stmts {
                hash_stmt(hasher, stmt);
            }
        }
        Expr::If { condition, then_branch, else_branch } => {
            hash_expr(hasher, condition);
            hash_expr(hasher, then_branch);
            if let Some(eb) = else_branch {
                hash_expr(hasher, eb);
            }
        }
        Expr::Match { expr, arms } => {
            hash_expr(hasher, expr);
            for (pat, arm_expr) in arms {
                pat.hash(hasher);
                hash_expr(hasher, arm_expr);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_causal_memory_basic() {
        let mut memory = CausalMemory::new();

        let id1 = memory.record_transition(
            "Counter", "increment", 100, 200, 42, 1, None
        ).unwrap();
        assert_eq!(id1, 1);

        let id2 = memory.record_transition(
            "Counter", "increment", 200, 300, 43, 2, Some(id1)
        ).unwrap();
        assert_eq!(id2, 2);

        assert_eq!(memory.len(), 2);
    }

    #[test]
    fn test_causal_chain() {
        let mut memory = CausalMemory::new();

        let id1 = memory.record_transition("A", "f", 1, 2, 1, 1, None).unwrap();
        let id2 = memory.record_transition("A", "g", 2, 3, 2, 2, Some(id1)).unwrap();
        let id3 = memory.record_transition("A", "h", 3, 4, 3, 3, Some(id2)).unwrap();

        let chain = memory.get_causal_chain(id3).unwrap();
        assert_eq!(chain.len(), 3);
        assert_eq!(chain[0].id, id1);
        assert_eq!(chain[1].id, id2);
        assert_eq!(chain[2].id, id3);
    }

    #[test]
    fn test_causal_integrity() {
        let mut memory = CausalMemory::new();

        let id1 = memory.record_transition("A", "f", 1, 2, 1, 1, None).unwrap();
        let id2 = memory.record_transition("A", "g", 2, 3, 2, 2, Some(id1)).unwrap();

        assert!(memory.verify_integrity().is_ok());
    }

    #[test]
    fn test_causal_distance() {
        let mut memory = CausalMemory::new();

        let id1 = memory.record_transition("A", "f", 1, 2, 1, 1, None).unwrap();
        let id2 = memory.record_transition("A", "g", 2, 3, 2, 2, Some(id1)).unwrap();
        let id3 = memory.record_transition("A", "h", 3, 4, 3, 3, Some(id2)).unwrap();
        let id4 = memory.record_transition("A", "i", 4, 5, 4, 4, Some(id1)).unwrap(); // Branch from id1

        let dist = memory.causal_distance(id3, id4).unwrap();
        // id3: 1->2->3 (depth 3), id4: 1->4 (depth 2), LCA at depth 1
        // Distance = (3-1) + (2-1) = 3
        assert_eq!(dist, 3);
    }

    #[test]
    fn test_counterfactual() {
        let mut memory = CausalMemory::new();

        let id1 = memory.record_transition("A", "f", 1, 2, 1, 1, None).unwrap();
        let id2 = memory.record_transition("A", "g", 2, 3, 2, 2, Some(id1)).unwrap();
        let id3 = memory.record_transition("A", "h", 3, 4, 3, 3, Some(id2)).unwrap(); // descendant of id2

        let cf_chain = memory.counterfactual(id2, 999).unwrap();
        assert_eq!(cf_chain.len(), 2); // Counterfactual node + 1 descendant
        assert_eq!(cf_chain[0].post_state_sig, 999);
    }

    #[test]
    fn test_contract_index() {
        let mut memory = CausalMemory::new();

        memory.record_transition("Counter", "inc", 1, 2, 1, 1, None).unwrap();
        memory.record_transition("Counter", "dec", 2, 1, 2, 2, None).unwrap();
        memory.record_transition("Escrow", "deposit", 10, 20, 3, 3, None).unwrap();

        let counter_history = memory.get_contract_history("Counter");
        assert_eq!(counter_history.len(), 2);

        let escrow_history = memory.get_contract_history("Escrow");
        assert_eq!(escrow_history.len(), 1);
    }
}