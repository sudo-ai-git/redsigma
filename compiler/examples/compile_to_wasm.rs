//! Compile a REDΣSIGMA contract to a WASM binary.
//!
//! ```sh
//! cargo run -p redsigma-compiler --example compile_to_wasm -- \
//!   examples/verified_counter.redsigma /tmp/counter.wasm
//! python3 -c "import wasmtime; wasmtime.Module(wasmtime.Engine(), open('/tmp/counter.wasm','rb').read())"
//! ```

use redsigma_compiler::{compile, CompilerError};
use std::fs;

fn main() -> Result<(), CompilerError> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: compile_to_wasm <contract.redsigma> <out.wasm>");
        std::process::exit(2);
    }
    let source = fs::read_to_string(&args[1])
        .map_err(|e| CompilerError::ParseError(format!("cannot read {}: {e}", args[1])))?;
    let wasm = compile(&source)?;
    fs::write(&args[2], &wasm)
        .map_err(|e| CompilerError::CodegenError(format!("cannot write {}: {e}", args[2])))?;
    println!("wrote {} bytes to {}", wasm.len(), args[2]);
    Ok(())
}
