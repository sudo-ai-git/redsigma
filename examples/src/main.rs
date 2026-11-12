//! REDΣSIGMA Rust Usage Example
//! 
//! This example demonstrates how to use the REDΣSIGMA parser and compiler
//! from Rust code.

use redsigma_parser::{Parser, Contract, Function, Stmt, Expr, Type};
use redsigma_compiler::{compile, parse_contract, CompilerError};

fn main() -> Result<(), CompilerError> {
    println!("🔴 REDΣSIGMA Rust Integration Example");
    println!("=====================================\n");

    // Example 1: Parse a simple contract
    let simple_contract = r#"
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
            fn get_count() -> u64 { count; }
        }
    "#;

    println!("📝 Example 1: Parsing a simple contract");
    println!("Source:\n{}", simple_contract);

    let contracts = parse_contract(simple_contract)?;
    println!("✅ Parsed {} contract(s)\n", contracts.len());

    for contract in &contracts {
        print_contract(contract);
    }

    // Example 2: Parse a more complex contract with invariants
    let escrow_contract = r#"
        contract Escrow {
            invariant balance >= 0;
            invariant locked + released == total_deposited;
            
            state {
                balance: u128,
                locked: u128,
                released: u128,
                total_deposited: u128,
            }
            
            fn deposit(amount: u128) -> u128 {
                let new_balance = balance + amount;
                let new_locked = locked + amount;
                let new_total = total_deposited + amount;
                balance = new_balance;
                locked = new_locked;
                total_deposited = new_total;
                new_locked;
            }
        }
    "#;

    println!("📝 Example 2: Parsing Escrow contract");
    let contracts = parse_contract(escrow_contract)?;
    println!("✅ Parsed {} contract(s)\n", contracts.len());

    for contract in &contracts {
        print_contract(contract);
    }

    // Example 3: Compile to WASM (stub implementation)
    println!("📝 Example 3: Compiling to WASM");
    let wasm = compile(simple_contract)?;
    println!("✅ Compiled to {} bytes (stub)\n", wasm.len());

    // Example 4: Error handling
    println!("📝 Example 4: Error handling");
    let invalid_contract = r#"
        contract Bad {
            state { x: u64 }
            fn bad() { }
        }
    "#;

    match parse_contract(invalid_contract) {
        Ok(_) => println!("⚠️  Parsed (but would fail at compile-time verification)"),
        Err(e) => println!("❌ Parse error: {}", e),
    }

    println!("\n🎉 All examples completed successfully!");
    Ok(())
}

fn print_contract(contract: &Contract) {
    println!("  Contract: {}", contract.name);
    println!("  Invariants: {}", contract.invariants.len());
    for inv in &contract.invariants {
        println!("    - {}", inv.expression);
    }
    println!("  State fields: {}", contract.state.fields.len());
    for (name, ty) in &contract.state.fields {
        println!("    - {}: {}", name, type_to_string(ty));
    }
    println!("  Functions: {}", contract.functions.len());
    for func in &contract.functions {
        println!("    fn {}()", func.name);
        if let Some(verified) = &func.verified {
            println!(
                "      @verified(pre: {:?}, post: {:?}, invariant: {:?})",
                verified.pre, verified.post, verified.invariant
            );
        }
    }
    println!();
}

fn type_to_string(ty: &Type) -> String {
    let mut s = ty.name.clone();
    if !ty.generics.is_empty() {
        s.push('<');
        s.push_str(
            &ty.generics
                .iter()
                .map(type_to_string)
                .collect::<Vec<_>>()
                .join(", "),
        );
        s.push('>');
    }
    s
}