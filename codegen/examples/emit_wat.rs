//! Emit WAT for a REDΣSIGMA contract.
//!
//! ```sh
//! cargo run -p redsigma-codegen --example emit_wat -- contract.redsigma
//! ```
//!
//! The output is WAT text. Assemble and validate it with `wasmtime`, which is
//! the only trustworthy validity gate:
//!
//! ```python
//! import wasmtime
//! binary = wasmtime.wat2wasm(wat)      # parse
//! module = wasmtime.Module(engine, binary)  # VALIDATE
//! ```

use redsigma_codegen::emit_wat;
use redsigma_parser::Parser;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let source = if args.len() > 1 {
        std::fs::read_to_string(&args[1])?
    } else {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
        s
    };

    let contracts = Parser::parse(&source).map_err(|e| format!("parse error: {}", e))?;
    if contracts.is_empty() {
        return Err("no contracts parsed".into());
    }
    let wat = emit_wat(&contracts[0]).map_err(|e| format!("codegen error: {}", e))?;
    print!("{}", wat);
    Ok(())
}
