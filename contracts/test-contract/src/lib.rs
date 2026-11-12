//! REDΣSIGMA Minimal Test Contract for NEAR Testnet Deployment

use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::state::ContractState;
use near_sdk::{env, near_bindgen, AccountId, Gas};

#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize)]
pub struct RedSigmaContract {
    owner_id: AccountId,
    contract_count: u32,
}

// Manual ContractState implementation
impl ContractState for RedSigmaContract {
    fn state_key() -> &'static [u8] {
        b"STATE"
    }

    fn state_read() -> Option<Self>
    where
        Self: BorshDeserialize,
    {
        env::storage_read(Self::state_key()).map(|data| {
            borsh::from_slice(&data)
                .unwrap_or_else(|_| env::panic_str("Cannot deserialize contract state."))
        })
    }

    fn state_write(&self) -> bool
    where
        Self: BorshSerialize,
    {
        env::storage_write(
            Self::state_key(),
            &borsh::to_vec(self)
                .unwrap_or_else(|_| env::panic_str("Cannot serialize contract state.")),
        )
    }
}

impl Default for RedSigmaContract {
    fn default() -> Self {
        Self {
            owner_id: env::predecessor_account_id(),
            contract_count: 0,
        }
    }
}

#[near_bindgen]
impl RedSigmaContract {
    #[init]
    pub fn new(owner_id: AccountId) -> Self {
        Self {
            owner_id,
            contract_count: 0,
        }
    }

    /// Deploy a new REDΣSIGMA contract (simplified)
    #[handle_result]
    pub fn deploy_contract(
        &mut self,
        contract_name: String,
        source: String,
    ) -> Result<String, String> {
        // Parameter names ARE the JSON field names in a NEAR call, so `_source`
        // here forced every caller to pass `{"_source": ...}`. Keep them public.
        let _ = &source;
        let caller = env::predecessor_account_id();
        if caller != self.owner_id {
            return Err("Only owner can deploy contracts".to_string());
        }

        self.contract_count += 1;

        Ok(format!("Contract {} deployed successfully", contract_name))
    }

    /// Execute a contract function (simplified simulation)
    #[handle_result]
    pub fn execute(
        &mut self,
        contract_name: String,
        method_name: String,
        args: Vec<u8>,
    ) -> Result<Vec<u8>, String> {
        let _ = &args;
        // Check gas
        let prepaid_gas = env::prepaid_gas();
        if prepaid_gas < near_sdk::Gas::from_gas(10_000_000_000_000) {
            return Err("Insufficient gas".to_string());
        }

        // Simulate execution
        Ok(format!("Executed {} on {}", method_name, "contract").into_bytes())
    }

    /// View call (read-only)
    #[handle_result]
    pub fn view(
        &self,
        contract_name: String,
        method_name: String,
        args: Vec<u8>,
    ) -> Result<Vec<u8>, String> {
        let _ = (&contract_name, &args);
        Ok(format!("View {} on contract", method_name).into_bytes())
    }

    /// Get contract count
    pub fn get_contract_count(&self) -> u32 {
        self.contract_count
    }

    /// Get owner
    pub fn get_owner(&self) -> AccountId {
        self.owner_id.clone()
    }
}

#[near_bindgen]
impl RedSigmaContract {
    /// Callback for cross-contract calls
    #[private]
    pub fn callback(&mut self, _result: Vec<u8>) -> Vec<u8> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::test_utils::VMContextBuilder;
    use near_sdk::{testing_env, VMContext};

    fn get_context(predecessor: AccountId) -> VMContext {
        VMContextBuilder::new()
            .predecessor_account_id(predecessor)
            .build()
    }

    #[test]
    fn test_deploy_and_execute() {
        let context = get_context("owner.testnet".parse().unwrap());
        testing_env!(context);

        let mut contract = RedSigmaContract::default();

        let result = contract.deploy_contract("counter".to_string(), "source".to_string());
        assert!(result.is_ok());

        let context = get_context("alice.testnet".parse().unwrap());
        testing_env!(context);

        let result = contract.execute("counter".to_string(), "increment".to_string(), vec![]);
        assert!(result.is_ok());
    }
}
