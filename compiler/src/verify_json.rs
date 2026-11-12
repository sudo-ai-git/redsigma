//! Verification-input export.
//!
//! `@verified(pre:, post:, invariant:)` used to be discarded at the parser's only
//! construction site, so no verifier could ever see it. The AST now carries it,
//! and this module turns a contract into the JSON a z3-based verifier consumes.
//!
//! The AST is serialized with serde's default externally-tagged enum encoding:
//! unit variants are bare strings (`"Unit"`), newtype variants are
//! `{"Number": "5"}`, struct variants are `{"Binary": {"left": ..., "op":
//! "Add", "right": ...}}`. The Python side handles all three.

use redsigma_parser::Contract;
use serde_json::{json, Value};

/// Build the verification input for a contract.
pub fn verification_input(contract: &Contract) -> Value {
    let state: Vec<Value> = contract
        .state
        .fields
        .iter()
        .map(|(name, ty)| json!({ "name": name, "type": ty.name }))
        .collect();

    let invariants: Vec<Value> = contract
        .invariants
        .iter()
        .map(|i| serde_json::to_value(&i.expr).expect("Expr is Serialize"))
        .collect();

    let functions: Vec<Value> = contract
        .functions
        .iter()
        .map(|f| {
            let ann = f.verified.as_ref();
            let params: Vec<Value> = f
                .params
                .iter()
                .map(|(name, ty)| json!({ "name": name, "type": ty.name }))
                .collect();
            json!({
                "name": f.name,
                "params": params,
                "return_type": f.return_type.name,
                "pre": ann.and_then(|a| a.pre.as_ref()),
                "post": ann.and_then(|a| a.post.as_ref()),
                "invariant": ann.and_then(|a| a.invariant.as_ref()),
                // Without the body the verifier can check the clauses against
                // nothing; it must see what the function actually does.
                "body": f.body,
            })
        })
        .collect();

    json!({
        "contract": contract.name,
        "invariants": invariants,
        "state": state,
        "functions": functions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use redsigma_parser::Parser;

    #[test]
    fn test_verification_input_carries_clauses() {
        let src = r#"
            contract Test {
                invariant total >= 0;
                state { total: u64 }
                @verified(pre: amount > 0, post: total == old(total) + amount, invariant: total >= 0)
                fn deposit(amount: u64) -> u64 { total = total + amount; amount; }
            }
        "#;
        let contract = &Parser::parse(src).unwrap()[0];
        let input = verification_input(contract);

        assert_eq!(input["contract"], "Test");
        assert_eq!(input["state"][0]["name"], "total");
        assert_eq!(input["state"][0]["type"], "u64");

        let f = &input["functions"][0];
        assert_eq!(f["name"], "deposit");
        assert!(
            f["pre"].is_object(),
            "pre must be an AST node: {}",
            f["pre"]
        );
        assert!(
            f["post"].is_object(),
            "post must be an AST node: {}",
            f["post"]
        );
        assert!(
            f["invariant"].is_object(),
            "invariant must be an AST node: {}",
            f["invariant"]
        );
        // The body must be present: without it the clauses are checked against
        // nothing and every postcondition is vacuously "proven".
        assert!(
            f["body"].is_array(),
            "body must be a statement list: {}",
            f["body"]
        );
        assert_eq!(f["body"].as_array().unwrap().len(), 2);
        assert_eq!(input["invariants"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_function_without_annotation_has_null_clauses() {
        let src = r#"
            contract Test {
                invariant x >= 0;
                state { x: u64 }
                fn get() -> u64 { x; }
            }
        "#;
        let contract = &Parser::parse(src).unwrap()[0];
        let input = verification_input(contract);
        let f = &input["functions"][0];
        assert!(f["pre"].is_null());
        assert!(f["post"].is_null());
        assert!(f["invariant"].is_null());
    }
}
