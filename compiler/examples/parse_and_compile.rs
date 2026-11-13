//! Parse and (stub-)compile a REDΣSIGMA contract.
//!
//! Mirrors the "Using from Rust" example in the README, so that snippet is
//! executed rather than aspirational.
//!
//! ```sh
//! cargo run -p redsigma-compiler --example parse_and_compile
//! ```

use redsigma_compiler::{compile, parse_contract, CompilerError};

fn main() -> Result<(), CompilerError> {
    let source = r#"
        contract Test {
            invariant balance >= 0;
            state { balance: u64 }
            fn deposit(amount: u64) -> u64 {
                balance = balance + amount;
                balance
            }
        }
    "#;

    // Parse and validate
    let contracts = parse_contract(source)?;
    println!("Parsed {} contract(s)", contracts.len());

    // Compile to WASM (stub - full pipeline TODO)
    let wasm = compile(source)?;
    println!("Compiled to {} bytes", wasm.len());

    Ok(())
}
