//! Emit the z3 verification input for REDΣSIGMA contracts.
//!
//! ```sh
//! cargo run -p redsigma-compiler --example emit_verification -- contract.redsigma \
//!   | python3 verify/redsigma_verify.py
//! ```
//!
//! Prints a JSON array with one entry per parsed contract: its state fields and,
//! per function, the parsed `pre` / `post` / `invariant` AST nodes plus the
//! function body the verifier symbolically executes. The Python verifier reads it.

use redsigma_compiler::parse_contract;
use redsigma_compiler::verify_json::verification_input;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let source = if args.len() > 1 {
        std::fs::read_to_string(&args[1])?
    } else {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
        s
    };

    // The parser returns every contract in the file; silently dropping all but
    // the first would verify a program the user did not write.
    let contracts = parse_contract(&source)?;
    if contracts.is_empty() {
        eprintln!("no contracts found in input");
        std::process::exit(2);
    }
    let inputs: Vec<_> = contracts.iter().map(verification_input).collect();
    println!("{}", serde_json::to_string_pretty(&inputs)?);
    Ok(())
}
