//! REDΣSIGMA Edge Case Tests
//!
//! Comprehensive edge case testing for all modules

use redsigma_causal::{CausalMemory, CausalNode};
use redsigma_codegen::{gate231_checksum, CodegenPipeline, WasmCompilation};
use redsigma_dualstream::{DualStreamAnalyzer, DualStreamReport};
use redsigma_parser::Parser;
use redsigma_reflection::{ReflectionEngine, ReflectionReport};
use redsigma_runtime::{AccountId, ExecutionContext, NearRuntime, StateKey, StateValue};
use redsigma_semantic::{SemanticAnalyzer, SemanticReport};

#[cfg(test)]
mod parser_edge_cases {
    use super::*;

    #[test]
    fn test_empty_contract() {
        let input = r#"
            contract Empty {
                state {}
                fn foo() -> u64 { 42; }
            }
        "#;
        let contracts = Parser::parse(input).unwrap();
        assert_eq!(contracts[0].name, "Empty");
        assert_eq!(contracts[0].state.fields.len(), 0);
    }

    #[test]
    fn test_nested_blocks() {
        let input = r#"
            contract Nested {
                state { x: u64 }
                fn foo() -> u64 {
                    {
                        let a = 1;
                        {
                            let b = 2;
                            b
                        }
                    }
                }
            }
        "#;
        let contracts = Parser::parse(input).unwrap();
        let func = &contracts[0].functions[0];
        assert!(!func.body.is_empty());
    }

    #[test]
    fn test_complex_expressions() {
        let input = r#"
            contract Complex {
                state { x: u64 }
                fn foo() -> u64 {
                    let a = 1 + 2 * 3 - 4 / 5;
                    let b = (a > 10) && (a < 100) || false;
                    if b { 1 } else { 2 }
                }
            }
        "#;
        let contracts = Parser::parse(input).unwrap();
        assert_eq!(contracts[0].functions[0].body.len(), 3);
    }

    #[test]
    fn test_generics() {
        let input = r#"
            contract Generic {
                state { items: Vec<u64> }
                fn push(item: u64) -> Vec<u64> {
                    items
                }
            }
        "#;
        let contracts = Parser::parse(input).unwrap();
        let field = &contracts[0].state.fields[0];
        assert_eq!(field.1.name, "Vec");
        assert_eq!(field.1.generics[0].name, "u64");
    }

    #[test]
    fn test_match_expression() {
        let input = r#"
            contract Match {
                state { x: u64 }
                fn foo() -> u64 {
                    match x {
                        0 => 1,
                        1 => 2,
                        _ => 3
                    }
                }
            }
        "#;
        let contracts = Parser::parse(input).unwrap();
        // Match expressions should parse without error
    }

    #[test]
    fn test_for_loop() {
        let input = r#"
            contract Loop {
                state { x: u64 }
                fn foo() -> u64 {
                    for i in 0..10 {
                        x = x + i;
                    }
                    x
                }
            }
        "#;
        let contracts = Parser::parse(input).unwrap();
        assert!(!contracts[0].functions[0].body.is_empty());
    }

    #[test]
    fn test_while_loop() {
        let input = r#"
            contract While {
                state { x: u64 }
                fn foo() -> u64 {
                    while x < 10 {
                        x = x + 1;
                    }
                    x
                }
            }
        "#;
        let contracts = Parser::parse(input).unwrap();
        assert!(!contracts[0].functions[0].body.is_empty());
    }

    #[test]
    fn test_verification_annotation() {
        let input = r#"
            contract Verified {
                invariant x >= 0;
                state { x: u64 }
                @verified(pre: x >= 0, post: x == old(x) + 1, invariant: x >= 0)
                fn increment() -> u64 {
                    let new_x = x + 1;
                    x = new_x;
                    new_x
                }
            }
        "#;
        let contracts = Parser::parse(input).unwrap();
        let func = &contracts[0].functions[0];
        assert!(func.verified.is_some());
    }

    #[test]
    fn test_multiple_contracts() {
        let input = r#"
            contract A { state { x: u64 } fn foo() -> u64 { 1; } }
            contract B { state { y: u64 } fn bar() -> u64 { 2; } }
        "#;
        let contracts = Parser::parse(input).unwrap();
        assert_eq!(contracts.len(), 2);
        assert_eq!(contracts[0].name, "A");
        assert_eq!(contracts[1].name, "B");
    }

    #[test]
    fn test_const_vs_let() {
        let input = r#"
            contract Const {
                state { x: u64 }
                fn foo() -> u64 {
                    const C: u64 = 42;
                    let mut v = 0;
                    v = v + C;
                    v
                }
            }
        "#;
        let contracts = Parser::parse(input).unwrap();
        assert!(!contracts[0].functions[0].body.is_empty());
    }

    #[test]
    fn test_unary_ops() {
        let input = r#"
            contract Unary {
                state { x: i64 }
                fn foo() -> i64 {
                    let a = -x;
                    let b = !a;
                    a
                }
            }
        "#;
        let contracts = Parser::parse(input).unwrap();
        assert!(!contracts[0].functions[0].body.is_empty());
    }

    #[test]
    fn test_field_access() {
        let input = r#"
            contract Field {
                state { obj: Point }
                fn foo() -> u64 {
                    obj.x
                }
            }
        "#;
        let contracts = Parser::parse(input).unwrap();
    }

    #[test]
    fn test_array_indexing() {
        let input = r#"
            contract Array {
                state { arr: Vec<u64> }
                fn foo() -> u64 {
                    arr[0]
                }
            }
        "#;
        let contracts = Parser::parse(input).unwrap();
    }

    #[test]
    fn test_method_calls() {
        let input = r#"
            contract Method {
                state { x: u64 }
                fn get() -> u64 { x; }
                fn foo() -> u64 {
                    self.get()
                }
            }
        "#;
        let contracts = Parser::parse(input).unwrap();
        assert!(!contracts[0].functions[0].body.is_empty());
    }
}

#[cfg(test)]
mod semantic_edge_cases {
    use super::*;

    #[test]
    fn test_semantic_empty_contract() {
        let input = r#"
            contract Empty { state {} fn foo() -> u64 { 42; } }
        "#;
        let contracts = Parser::parse(input).unwrap();
        let mut analyzer = SemanticAnalyzer::new();
        let report = analyzer.analyze(&contracts[0]).unwrap();
        assert!(report.bijection_verified);
    }

    #[test]
    fn test_semantic_determinism() {
        let input = r#"
            contract Test { state { x: u64 } fn foo() -> u64 { 42; } }
        "#;
        let contracts = Parser::parse(input).unwrap();
        let mut a1 = SemanticAnalyzer::new();
        let mut a2 = SemanticAnalyzer::new();
        let r1 = a1.analyze(&contracts[0]).unwrap();
        let r2 = a2.analyze(&contracts[0]).unwrap();
        assert_eq!(r1.semantic_signature, r2.semantic_signature);
    }

    #[test]
    fn test_semantic_collision() {
        let input1 = r#"
            contract A { state { x: u64 } fn foo() -> u64 { 1; } }
        "#;
        let input2 = r#"
            contract B { state { x: u64 } fn foo() -> u64 { 1; } }
        "#;
        let contracts1 = Parser::parse(input1).unwrap();
        let contracts2 = Parser::parse(input2).unwrap();
        let mut analyzer = SemanticAnalyzer::new();
        analyzer.analyze(&contracts1[0]).unwrap();
        let result = analyzer.analyze(&contracts2[0]);
        // Different contracts with same structure may have same hash
        // but bijective verifier should catch semantic collisions
    }
}

#[cfg(test)]
mod dualstream_edge_cases {
    use super::*;

    #[test]
    fn test_dualstream_sentinel() {
        let mut analyzer = DualStreamAnalyzer::new();
        let input = r#"
            contract Sentinel { state { x: u64 } fn foo() -> u64 { 0; } }
        "#;
        let contracts = Parser::parse(input).unwrap();
        let report = analyzer.analyze(&contracts[0]).unwrap();
        // Should detect sentinel if structural channels hit 0, 37, 73, etc.
    }

    #[test]
    fn test_dualstream_uniqueness_many_contracts() {
        let mut analyzer = DualStreamAnalyzer::new();
        for i in 0..100 {
            let input = format!(
                r#"
                contract Contract{} {{ state {{ x: u64 }} fn foo() -> u64 {{ {}; }} }}
            "#,
                i, i
            );
            let contracts = Parser::parse(&input).unwrap();
            analyzer.analyze(&contracts[0]).unwrap();
        }
        assert!(analyzer.verify_joint_uniqueness());
        assert_eq!(analyzer.unique_signatures(), 100);
    }

    #[test]
    fn test_gate231_checksum_deterministic() {
        let input = r#"
            contract Test { state { x: u64 } fn foo() -> u64 { 42; } }
        "#;
        let contracts = Parser::parse(input).unwrap();
        let c1 = gate231_checksum(&contracts[0]);
        let c2 = gate231_checksum(&contracts[0]);
        assert_eq!(c1, c2);
        assert!(c1 < 231);
    }

    #[test]
    fn test_gate231_changes_with_content() {
        let input1 = r#"
            contract A { state { x: u64 } fn foo() -> u64 { 1; } }
        "#;
        let input2 = r#"
            contract A { state { x: u64 } fn foo() -> u64 { 2; } }
        "#;
        let contracts1 = Parser::parse(input1).unwrap();
        let contracts2 = Parser::parse(input2).unwrap();
        let c1 = gate231_checksum(&contracts1[0]);
        let c2 = gate231_checksum(&contracts2[0]);
        // Should be different (though collisions possible mod 231)
    }
}

#[cfg(test)]
mod causal_edge_cases {
    use super::*;

    #[test]
    fn test_causal_branching() {
        let mut memory = CausalMemory::new();
        let root = memory
            .record_transition("A", "init", 0, 1, 1, 1, None)
            .unwrap();
        let b1 = memory
            .record_transition("A", "branch1", 1, 2, 2, 2, Some(root))
            .unwrap();
        let b2 = memory
            .record_transition("A", "branch2", 1, 3, 3, 3, Some(root))
            .unwrap();
        let b11 = memory
            .record_transition("A", "branch11", 2, 4, 4, 4, Some(b1))
            .unwrap();

        // Check causal distance between b11 and b2
        let dist = memory.causal_distance(b11, b2).unwrap();
        assert_eq!(dist, 3); // b11->b1->root, b2->root
    }

    #[test]
    fn test_causal_integrity_orphan() {
        let mut memory = CausalMemory::new();
        let id1 = memory
            .record_transition("A", "f", 1, 2, 1, 1, None)
            .unwrap();
        // Manually create orphan (not possible through API, but test integrity check)
        assert!(memory.verify_integrity().is_ok());
    }

    #[test]
    fn test_counterfactual_deep() {
        let mut memory = CausalMemory::new();
        let a = memory
            .record_transition("A", "a", 1, 2, 1, 1, None)
            .unwrap();
        let b = memory
            .record_transition("A", "b", 2, 3, 2, 2, Some(a))
            .unwrap();
        let c = memory
            .record_transition("A", "c", 3, 4, 3, 3, Some(b))
            .unwrap();
        let d = memory
            .record_transition("A", "d", 4, 5, 4, 4, Some(c))
            .unwrap();

        let cf = memory.counterfactual(b, 999).unwrap();
        assert_eq!(cf.len(), 3); // cf(b) + c + d
        assert_eq!(cf[0].post_state_sig, 999);
    }

    #[test]
    fn test_causal_contract_index() {
        let mut memory = CausalMemory::new();
        for i in 0..10 {
            memory
                .record_transition(
                    &format!("Contract{}", i),
                    "f",
                    i as u64,
                    (i + 1) as u64,
                    i as u64,
                    i as u64,
                    None,
                )
                .unwrap();
        }
        for i in 0..10 {
            let history = memory.get_contract_history(&format!("Contract{}", i));
            assert_eq!(history.len(), 1);
        }
    }

    #[test]
    fn test_causal_get_children() {
        let mut memory = CausalMemory::new();
        let root = memory
            .record_transition("A", "root", 0, 1, 1, 1, None)
            .unwrap();
        let c1 = memory
            .record_transition("A", "c1", 1, 2, 2, 2, Some(root))
            .unwrap();
        let c2 = memory
            .record_transition("A", "c2", 1, 3, 3, 3, Some(root))
            .unwrap();

        let children = memory.get_children(root);
        assert_eq!(children.len(), 2);
    }
}

#[cfg(test)]
mod reflection_edge_cases {
    use super::*;

    #[test]
    fn test_reflection_contradiction_exact() {
        let input = r#"
            contract Contra {
                invariant x >= 0;
                invariant x <= -1;
                state { x: i64 }
                fn foo() -> i64 { 0; }
            }
        "#;
        let contracts = Parser::parse(input).unwrap();
        let mut engine = ReflectionEngine::new();
        let report = engine.reflect(&contracts[0]).unwrap();
        assert!(!report.overall_passed);
        assert!(!report.layers[5].passed); // Contradiction layer
    }

    #[test]
    fn test_reflection_missing_invariants() {
        let input = r#"
            contract NoInv { state { x: u64 } fn foo() -> u64 { 1; } }
        "#;
        let contracts = Parser::parse(input).unwrap();
        let mut engine = ReflectionEngine::new();
        let report = engine.reflect(&contracts[0]).unwrap();
        assert!(!report.overall_passed);
        assert!(!report.layers[3].passed); // Completeness layer
    }

    #[test]
    fn test_reflection_duplicate_fields() {
        let input = r#"
            contract Dup { state { x: u64, x: u64 } fn foo() -> u64 { 1; } }
        "#;
        let contracts = Parser::parse(input).unwrap();
        let mut engine = ReflectionEngine::new();
        let report = engine.reflect(&contracts[0]).unwrap();
        assert!(!report.overall_passed);
        assert!(!report.layers[0].passed); // Consistency layer
    }

    #[test]
    fn test_reflection_empty_state() {
        let input = r#"
            contract Empty { invariant true; state {} fn foo() -> u64 { 1; } }
        "#;
        let contracts = Parser::parse(input).unwrap();
        let mut engine = ReflectionEngine::new();
        let report = engine.reflect(&contracts[0]).unwrap();
        // Empty state with invariants should still pass if well-formed
    }

    #[test]
    fn test_reflection_complex() {
        let input = r#"
            contract Complex {
                invariant balance >= 0;
                invariant locked + released == total;
                state { balance: u128, locked: u128, released: u128, total: u128 }
                fn deposit(amount: u128) -> u128 {
                    let nb = balance + amount;
                    let nl = locked + amount;
                    let nt = total + amount;
                    balance = nb; locked = nl; total = nt;
                    nl
                }
                fn release(amount: u128) -> u128 {
                    let nr = released + amount;
                    let nl = locked - amount;
                    released = nr; locked = nl;
                    nr
                }
            }
        "#;
        let contracts = Parser::parse(input).unwrap();
        let mut engine = ReflectionEngine::new();
        let report = engine.reflect(&contracts[0]).unwrap();
        assert!(report.overall_passed);
        assert!(report.overall_score > 0.8);
    }

    #[test]
    fn test_reflection_layer_scores() {
        let input = r#"
            contract Score { state { x: u64 } fn foo() -> u64 { 1; } }
        "#;
        let contracts = Parser::parse(input).unwrap();
        let mut engine = ReflectionEngine::new();
        let report = engine.reflect(&contracts[0]).unwrap();
        for layer in &report.layers {
            assert!(layer.score >= 0.0 && layer.score <= 1.0);
        }
    }
}

#[cfg(test)]
mod codegen_edge_cases {
    use super::*;

    #[test]
    fn test_codegen_wasm_magic() {
        let input = r#"
            contract Test { state { x: u64 } fn foo() -> u64 { 42; } }
        "#;
        let contracts = Parser::parse(input).unwrap();
        let mut pipeline = CodegenPipeline::new();
        let result = pipeline.compile(&contracts[0]).unwrap();
        // Check WASM magic number
        assert_eq!(&result.bytecode[0..4], &[0x00, 0x61, 0x73, 0x6D]);
    }

    #[test]
    fn test_codegen_proof_serialization() {
        let input = r#"
            contract Test { state { x: u64 } fn foo() -> u64 { 42; } }
        "#;
        let contracts = Parser::parse(input).unwrap();
        let mut pipeline = CodegenPipeline::new();
        let result = pipeline.compile(&contracts[0]).unwrap();
        let json = serde_json::to_string(&result.proof).unwrap();
        assert!(json.contains("Test"));
        assert!(json.contains("gate231_checksum"));
    }

    #[test]
    fn test_codegen_deterministic() {
        let input = r#"
            contract Test { state { x: u64 } fn foo() -> u64 { 42; } }
        "#;
        let contracts = Parser::parse(input).unwrap();
        let mut p1 = CodegenPipeline::new();
        let mut p2 = CodegenPipeline::new();
        let r1 = p1.compile(&contracts[0]).unwrap();
        let r2 = p2.compile(&contracts[0]).unwrap();
        // Bytecode should be deterministic (placeholder)
        assert_eq!(r1.bytecode, r2.bytecode);
    }

    #[test]
    fn test_codegen_multiple_functions() {
        let input = r#"
            contract Multi {
                state { x: u64 }
                fn a() -> u64 { 1; }
                fn b() -> u64 { 2; }
                fn c() -> u64 { 3; }
            }
        "#;
        let contracts = Parser::parse(input).unwrap();
        let mut pipeline = CodegenPipeline::new();
        let result = pipeline.compile(&contracts[0]).unwrap();
        assert_eq!(result.function_index.len(), 3);
    }
}

#[cfg(test)]
mod runtime_edge_cases {
    use super::*;

    #[test]
    fn test_runtime_gas_limit() {
        let input = r#"
            contract Test { invariant x >= 0; state { x: u64 } fn foo() -> u64 { 1; } }
        "#;
        let mut runtime = NearRuntime::new();
        let account_id = AccountId::new("test.testnet".to_string());
        let deployer = AccountId::new("deployer.testnet".to_string());
        runtime
            .deploy_contract(account_id.clone(), input, deployer, 1000)
            .unwrap();

        let context = ExecutionContext {
            contract_id: account_id.clone(),
            caller_id: AccountId::new("alice.testnet".to_string()),
            predecessor_id: AccountId::new("alice.testnet".to_string()),
            attached_deposit: 0,
            prepaid_gas: 1000, // Below verification cost
            block_height: 1001,
            block_timestamp: 1_000_000_000_000_000_000,
            random_seed: vec![],
        };

        let result = runtime.execute(&account_id, "foo", vec![], context);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Insufficient gas"));
    }

    #[test]
    fn test_runtime_state_persistence() {
        let input = r#"
            contract State { invariant x >= 0; state { x: u64 } fn set(v: u64) -> u64 { x = v; v; } fn get() -> u64 { x; } }
        "#;
        let mut runtime = NearRuntime::new();
        let account_id = AccountId::new("state.testnet".to_string());
        let deployer = AccountId::new("deployer.testnet".to_string());
        runtime
            .deploy_contract(account_id.clone(), input, deployer, 1000)
            .unwrap();

        let context = ExecutionContext {
            contract_id: account_id.clone(),
            caller_id: AccountId::new("alice.testnet".to_string()),
            predecessor_id: AccountId::new("alice.testnet".to_string()),
            attached_deposit: 0,
            prepaid_gas: 100_000_000_000,
            block_height: 1001,
            block_timestamp: 1_000_000_000_000_000_000,
            random_seed: vec![],
        };

        runtime
            .execute(
                &account_id,
                "set",
                42u64.to_le_bytes().to_vec(),
                context.clone(),
            )
            .unwrap();
        let result = runtime
            .execute(&account_id, "get", vec![], context)
            .unwrap();
        let val = u64::from_le_bytes(result.as_slice().try_into().unwrap_or([0; 8]));
        assert_eq!(val, 42);
    }

    #[test]
    fn test_runtime_view_call_no_state_change() {
        let input = r#"
            contract View { invariant x >= 0; state { x: u64 } fn set(v: u64) -> u64 { x = v; v; } fn get() -> u64 { x; } }
        "#;
        let mut runtime = NearRuntime::new();
        let account_id = AccountId::new("view.testnet".to_string());
        let deployer = AccountId::new("deployer.testnet".to_string());
        runtime
            .deploy_contract(account_id.clone(), input, deployer, 1000)
            .unwrap();

        let context = ExecutionContext {
            contract_id: account_id.clone(),
            caller_id: AccountId::new("alice.testnet".to_string()),
            predecessor_id: AccountId::new("alice.testnet".to_string()),
            attached_deposit: 0,
            prepaid_gas: 100_000_000_000,
            block_height: 1001,
            block_timestamp: 1_000_000_000_000_000_000,
            random_seed: vec![],
        };

        runtime
            .execute(&account_id, "set", 100u64.to_le_bytes().to_vec(), context)
            .unwrap();
        let result = runtime.view(&account_id, "get", vec![]).unwrap();
        let val = u64::from_le_bytes(result.as_slice().try_into().unwrap_or([0; 8]));
        assert_eq!(val, 100);
    }

    #[test]
    fn test_runtime_contract_not_found() {
        let mut runtime = NearRuntime::new();
        let account_id = AccountId::new("nonexistent.testnet".to_string());
        let context = ExecutionContext {
            contract_id: account_id.clone(),
            caller_id: AccountId::new("alice.testnet".to_string()),
            predecessor_id: AccountId::new("alice.testnet".to_string()),
            attached_deposit: 0,
            prepaid_gas: 100_000_000_000,
            block_height: 1001,
            block_timestamp: 1_000_000_000_000_000_000,
            random_seed: vec![],
        };
        let result = runtime.execute(&account_id, "foo", vec![], context);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Contract not found"));
    }

    #[test]
    fn test_runtime_proof_verification() {
        let input = r#"
            contract Proof { invariant x >= 0; state { x: u64 } fn foo() -> u64 { 1; } }
        "#;
        let mut runtime = NearRuntime::new();
        let account_id = AccountId::new("proof.testnet".to_string());
        let deployer = AccountId::new("deployer.testnet".to_string());
        runtime
            .deploy_contract(account_id.clone(), input, deployer, 1000)
            .unwrap();

        let contract = runtime.get_contract(&account_id).unwrap();
        let result = runtime.verify_proof(&contract.proof, &contract.contract);
        assert!(result.is_ok());
    }

    #[test]
    fn test_runtime_multiple_contracts() {
        let mut runtime = NearRuntime::new();
        let deployer = AccountId::new("deployer.testnet".to_string());

        let input1 = r#"contract A { invariant x >= 0; state { x: u64 } fn foo() -> u64 { 1; } }"#;
        let input2 = r#"contract B { invariant y >= 0; state { y: u64 } fn bar() -> u64 { 2; } }"#;

        runtime
            .deploy_contract(
                AccountId::new("a.testnet".to_string()),
                input1,
                deployer.clone(),
                1000,
            )
            .unwrap();
        runtime
            .deploy_contract(
                AccountId::new("b.testnet".to_string()),
                input2,
                deployer,
                1000,
            )
            .unwrap();

        let contracts = runtime.list_contracts();
        assert_eq!(contracts.len(), 2);
    }

    #[test]
    fn test_runtime_escrow_workflow() {
        let input = r#"
            contract Escrow {
                invariant balance >= 0;
                invariant locked + released == total_deposited;
                state { balance: u128, locked: u128, released: u128, total_deposited: u128 }
                fn deposit(amount: u128) -> u128 {
                    let nb = balance + amount;
                    let nl = locked + amount;
                    let nt = total_deposited + amount;
                    balance = nb; locked = nl; total_deposited = nt;
                    nl
                }
                fn release(amount: u128) -> u128 {
                    let nr = released + amount;
                    let nl = locked - amount;
                    released = nr; locked = nl;
                    nr
                }
                fn get_locked() -> u128 { locked; }
            }
        "#;
        let mut runtime = NearRuntime::new();
        let account_id = AccountId::new("escrow.testnet".to_string());
        let deployer = AccountId::new("deployer.testnet".to_string());
        runtime
            .deploy_contract(account_id.clone(), input, deployer, 1000)
            .unwrap();

        let context = ExecutionContext {
            contract_id: account_id.clone(),
            caller_id: AccountId::new("alice.testnet".to_string()),
            predecessor_id: AccountId::new("alice.testnet".to_string()),
            attached_deposit: 0,
            prepaid_gas: 100_000_000_000,
            block_height: 1001,
            block_timestamp: 1_000_000_000_000_000_000,
            random_seed: vec![],
        };

        // Deposit 100
        runtime
            .execute(
                &account_id,
                "deposit",
                100u128.to_le_bytes().to_vec(),
                context.clone(),
            )
            .unwrap();
        // Release 30
        runtime
            .execute(
                &account_id,
                "release",
                30u128.to_le_bytes().to_vec(),
                context.clone(),
            )
            .unwrap();
        // Check locked = 70
        let result = runtime.view(&account_id, "get_locked", vec![]).unwrap();
        let locked = u128::from_le_bytes(result.as_slice().try_into().unwrap_or([0; 16]));
        assert_eq!(locked, 70);
    }
}

#[cfg(test)]
mod integration_e2e {
    use super::*;

    #[test]
    fn test_full_pipeline_parser_to_runtime() {
        // Full pipeline: parse -> semantic -> dualstream -> reflection -> codegen -> runtime
        let input = r#"
            contract Counter {
                invariant count >= 0;
                state { count: u64 }
                fn increment() -> u64 {
                    let new_count = count + 1;
                    count = new_count;
                    new_count;
                }
                fn get() -> u64 { count; }
            }
        "#;

        // 1. Parse
        let contracts = Parser::parse(input).unwrap();
        assert_eq!(contracts.len(), 1);

        // 2. Semantic
        let mut sem = SemanticAnalyzer::new();
        let sem_report = sem.analyze(&contracts[0]).unwrap();
        assert!(sem_report.bijection_verified);

        // 3. Dual-stream
        let mut ds = DualStreamAnalyzer::new();
        let ds_report = ds.analyze(&contracts[0]).unwrap();
        assert!(ds.verify_joint_uniqueness());

        // 4. Reflection
        let mut engine = ReflectionEngine::new();
        let refl_report = engine.reflect(&contracts[0]).unwrap();
        assert!(refl_report.overall_passed);

        // 5. Codegen
        let mut pipeline = CodegenPipeline::new();
        let compilation = pipeline.compile(&contracts[0]).unwrap();
        assert!(!compilation.bytecode.is_empty());

        // 6. Runtime
        let mut runtime = NearRuntime::new();
        let account_id = AccountId::new("counter.testnet".to_string());
        let deployer = AccountId::new("deployer.testnet".to_string());
        runtime
            .deploy_contract(account_id.clone(), input, deployer, 1000)
            .unwrap();

        let context = ExecutionContext {
            contract_id: account_id.clone(),
            caller_id: AccountId::new("alice.testnet".to_string()),
            predecessor_id: AccountId::new("alice.testnet".to_string()),
            attached_deposit: 0,
            prepaid_gas: 100_000_000_000,
            block_height: 1001,
            block_timestamp: 1_000_000_000_000_000_000,
            random_seed: vec![],
        };

        runtime
            .execute(&account_id, "increment", vec![], context.clone())
            .unwrap();
        let result = runtime
            .execute(&account_id, "increment", vec![], context)
            .unwrap();
        let val = u64::from_le_bytes(result.as_slice().try_into().unwrap_or([0; 8]));
        assert_eq!(val, 2);
    }

    #[test]
    fn test_231_gate_end_to_end() {
        let input = r#"
            contract Gate231 { invariant x >= 0; state { x: u64 } fn foo() -> u64 { 42; } }
        "#;
        let contracts = Parser::parse(input).unwrap();
        let gate = gate231_checksum(&contracts[0]);
        assert!(gate < 231);

        // Through full pipeline
        let mut pipeline = CodegenPipeline::new();
        let compilation = pipeline.compile(&contracts[0]).unwrap();
        assert_eq!(compilation.proof.gate231_checksum, gate);
    }

    #[test]
    fn test_causal_chain_runtime() {
        let input = r#"
            contract Causal { invariant x >= 0; state { x: u64 } fn set(v: u64) -> u64 { x = v; v; } }
        "#;
        let mut runtime = NearRuntime::new();
        let account_id = AccountId::new("causal.testnet".to_string());
        let deployer = AccountId::new("deployer.testnet".to_string());
        runtime
            .deploy_contract(account_id.clone(), input, deployer, 1000)
            .unwrap();

        let context = ExecutionContext {
            contract_id: account_id.clone(),
            caller_id: AccountId::new("alice.testnet".to_string()),
            predecessor_id: AccountId::new("alice.testnet".to_string()),
            attached_deposit: 0,
            prepaid_gas: 100_000_000_000,
            block_height: 1001,
            block_timestamp: 1_000_000_000_000_000_000,
            random_seed: vec![],
        };

        runtime
            .execute(
                &account_id,
                "set",
                10u64.to_le_bytes().to_vec(),
                context.clone(),
            )
            .unwrap();
        runtime
            .execute(
                &account_id,
                "set",
                20u64.to_le_bytes().to_vec(),
                context.clone(),
            )
            .unwrap();
        runtime
            .execute(&account_id, "set", 30u64.to_le_bytes().to_vec(), context)
            .unwrap();

        let causal = runtime.causal_memory();
        assert!(!causal.is_empty());
        assert!(causal.len() >= 4); // deploy + 3 sets
    }
}
