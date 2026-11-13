use redsigma_codegen::{emit_near_module, assemble};
use redsigma_parser::Parser;

fn main() {
    let input = r#"
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
            fn get() -> u64 { count; }
        }
    "#;
    let contracts = Parser::parse(input).unwrap();
    match emit_near_module(&contracts[0]) {
        Ok(wat) => {
            println!("=== NEAR WAT ===");
            println!("{}", wat);
            match assemble(&wat) {
                Ok(bytes) => {
                    println!("\n=== WASM: {} bytes ===", bytes.len());
                    println!("Magic: {:02x?}", &bytes[0..4]);
                    println!("Version: {:02x?}", &bytes[4..8]);
                }
                Err(e) => println!("ASSEMBLE ERROR: {}", e),
            }
        }
        Err(e) => println!("EMIT ERROR: {}", e),
    }
}
