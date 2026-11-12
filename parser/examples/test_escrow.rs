use redsigma_parser::Parser;

fn main() {
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

    println!("Test: Escrow contract");
    match Parser::parse(escrow_contract) {
        Ok(contracts) => println!("Success: {:?}", contracts),
        Err(e) => println!("Error: {}", e),
    }
}
