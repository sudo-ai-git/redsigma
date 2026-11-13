//! REDΣSIGMA Test Contract for NEAR Testnet Deployment — backed by the real runtime.
//!
//! `deploy_contract` now parses `.redsigma` source, compiles it through the
//! full verification pipeline (producing NEAR ABI WASM), and stores the
//! deployed contract. `execute` loads the stored contract and runs the
//! requested function through the `NearRuntime` interpreter with proof
//! verification and invariant enforcement.

use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::state::ContractState;
use near_sdk::{env, near_bindgen, AccountId};
use redsigma_codegen::{CodegenPipeline, ProofObject};
use redsigma_parser::{Contract, Parser};
use redsigma_runtime::{
    AccountId as RuntimeAccountId, DeployedContract, ExecutionContext, NearRuntime,
};
use std::collections::HashMap;

type StateValue = redsigma_runtime::StateValue;

/// Serialized contract data stored in NEAR storage per deployed contract.
/// Serialized with BORSH (not serde_json): borsh is what the NEAR SDK uses
/// natively, and it avoids linking the JSON serializer into the contract,
/// which was a measurable share of the module size.
/// State is keyed by field name (String) rather than StateKey (Vec<u8>)
/// because JSON object keys must be strings.
#[derive(Clone, BorshSerialize, BorshDeserialize)]
struct ContractData {
    wasm_bytecode: Vec<u8>,
    proof: ProofObject,
    source: String,
    contract: Contract,
    /// state field name -> serialized bytes
    state: HashMap<String, Vec<u8>>,
}

/// REDΣSIGMA test contract — manages deployed .redsigma contracts.
#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize)]
pub struct RedSigmaContract {
    owner_id: AccountId,
    contract_count: u32,
    /// contract_name -> borsh-serialized ContractData
    contracts: HashMap<String, Vec<u8>>,
}

// Manual ContractState implementation (NEAR SDK 5.x does not auto-generate it).
impl ContractState for RedSigmaContract {
    fn state_key() -> &'static [u8] {
        b"STATE"
    }

    fn state_read() -> Option<Self>
    where
        Self: BorshDeserialize,
    {
        env::storage_read(Self::state_key()).map(|data| {
            match borsh::from_slice(&data) {
                Ok(contract) => contract,
                // Legacy state (pre-borsh-migration) predates the `contracts`
                // HashMap field: it holds owner_id (u32 len + UTF-8) followed by
                // contract_count (u32), then ends. Salvage the owner_id and
                // start fresh with an empty contracts map so the contract keeps
                // working after the redeploy. No contract data is lost — the
                // legacy map was empty (contract_count was a stale counter).
                Err(_) => {
                    let owner_id = if data.len() > 4 {
                        let len = u32::from_le_bytes(data[0..4].try_into().unwrap_or([0; 4])) as usize;
                        if 4 + len <= data.len() {
                            String::from_utf8_lossy(&data[4..4 + len]).into_owned()
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    };
                    RedSigmaContract {
                        owner_id: owner_id
                            .parse::<near_sdk::AccountId>()
                            .unwrap_or_else(|_| env::current_account_id()),
                        contract_count: 0,
                        contracts: HashMap::new(),
                    }
                }
            }
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
            contracts: HashMap::new(),
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
            contracts: HashMap::new(),
        }
    }

    /// Deploy a new REDΣSIGMA contract from `.redsigma` source.
    ///
    /// Parses the source, runs the full verification pipeline, compiles to
    /// NEAR ABI WASM, and stores the deployed contract.
    #[handle_result]
    pub fn deploy_contract(
        &mut self,
        contract_name: String,
        source: String,
    ) -> Result<String, String> {
        let caller = env::predecessor_account_id();
        if caller != self.owner_id {
            return Err("Only owner can deploy contracts".to_string());
        }

        // Parse the .redsigma source.
        let contracts = Parser::parse(&source)
            .map_err(|e| format!("Parse error: {}", e))?;
        if contracts.len() != 1 {
            return Err(format!(
                "Expected exactly 1 contract, found {}",
                contracts.len()
            ));
        }
        let contract = contracts[0].clone();

        // Compile with full verification (produces NEAR ABI WASM module).
        let mut pipeline = CodegenPipeline::new();
        let compilation = pipeline
            .compile(&contract)
            .map_err(|e| format!("Compilation error: {}", e))?;

        // Deploy gate: refuse to deploy a contract with failing verification
        // obligations. This is the gate the panel asked for — a contract that
        // fails reflection or dual-stream verification cannot be deployed.
        let proof = &compilation.proof;
        if !proof.reflection_report.overall_passed {
            return Err("Deployment rejected: reflection verification failed".to_string());
        }
        if proof.dual_stream_report.collisions > 0 {
            return Err("Deployment rejected: dual-stream collision detected".to_string());
        }

        let data = ContractData {
            wasm_bytecode: compilation.bytecode,
            proof: compilation.proof,
            source: compilation.source,
            contract,
            state: HashMap::new(),
        };

        let serialized =
            borsh::to_vec(&data).map_err(|e| format!("Serialization error: {}", e))?;
        self.contracts.insert(contract_name.clone(), serialized);
        self.contract_count += 1;

        Ok(format!("Contract {} deployed successfully", contract_name))
    }

    /// Execute a contract function through the real runtime.
    #[handle_result]
    pub fn execute(
        &mut self,
        contract_name: String,
        method_name: String,
        args: Vec<u8>,
    ) -> Result<Vec<u8>, String> {
        let prepaid_gas = env::prepaid_gas();
        if prepaid_gas.as_gas() < near_sdk::Gas::from_tgas(10).as_gas() {
            return Err("Insufficient gas".to_string());
        }

        // Load and deserialize stored contract data.
        let serialized = self
            .contracts
            .get(&contract_name)
            .ok_or_else(|| format!("Contract '{}' not found", contract_name))?;
        let data: ContractData =
            borsh::from_slice(serialized).map_err(|e| format!("Deserialization error: {}", e))?;

        // Build a runtime and insert the stored contract.
        // Convert state from String-keyed (JSON) to StateKey-keyed (runtime).
        let runtime_state: HashMap<redsigma_runtime::StateKey, StateValue> = data
            .state
            .into_iter()
            .map(|(k, v)| (redsigma_runtime::StateKey(k.into_bytes()), v))
            .collect();
        let mut runtime = NearRuntime::new();
        let account_id = RuntimeAccountId::new(contract_name.clone());
        let deployed = DeployedContract {
            account_id: account_id.clone(),
            wasm_bytecode: data.wasm_bytecode,
            proof: data.proof,
            source: data.source,
            deployed_at_height: env::block_height(),
            state: runtime_state,
            contract: data.contract,
        };
        runtime.insert_contract(account_id, deployed);

        // Build NEAR execution context from host environment.
        let context = ExecutionContext {
            contract_id: RuntimeAccountId::new(contract_name.clone()),
            caller_id: RuntimeAccountId::new(env::predecessor_account_id().to_string()),
            predecessor_id: RuntimeAccountId::new(env::predecessor_account_id().to_string()),
            attached_deposit: env::attached_deposit().as_yoctonear(),
            prepaid_gas: prepaid_gas.as_gas(),
            block_height: env::block_height(),
            block_timestamp: env::block_timestamp(),
            random_seed: env::random_seed(),
        };

        // Execute the method through the runtime (verifies proof + enforces invariants).
        let result = runtime
            .execute(
                &RuntimeAccountId::new(contract_name.clone()),
                &method_name,
                args,
                context,
            )
            .map_err(|e| format!("Execution error: {}", e))?;

        // Persist the updated state back to storage.
        // Convert state from StateKey-keyed (runtime) to String-keyed (JSON).
        let updated = runtime
            .get_contract(&RuntimeAccountId::new(contract_name.clone()))
            .ok_or_else(|| "Contract not found after execution".to_string())?;
        let mut data = borsh::from_slice::<ContractData>(serialized)
            .map_err(|e| format!("Deserialization error: {}", e))?;
        data.state = updated
            .state
            .iter()
            .map(|(k, v)| (String::from_utf8(k.0.clone()).unwrap_or_default(), v.clone()))
            .collect();
        let reserialized =
            borsh::to_vec(&data).map_err(|e| format!("Serialization error: {}", e))?;
        self.contracts.insert(contract_name, reserialized);

        Ok(result)
    }

    /// View call (read-only, no state changes persisted).
    #[handle_result]
    pub fn view(
        &self,
        contract_name: String,
        method_name: String,
        args: Vec<u8>,
    ) -> Result<Vec<u8>, String> {
        let serialized = self
            .contracts
            .get(&contract_name)
            .ok_or_else(|| format!("Contract '{}' not found", contract_name))?;
        let data: ContractData =
            borsh::from_slice(serialized).map_err(|e| format!("Deserialization error: {}", e))?;

        let mut runtime = NearRuntime::new();
        let account_id = RuntimeAccountId::new(contract_name.clone());
        // Convert state from String-keyed (JSON) to StateKey-keyed (runtime).
        let runtime_state: HashMap<redsigma_runtime::StateKey, StateValue> = data
            .state
            .into_iter()
            .map(|(k, v)| (redsigma_runtime::StateKey(k.into_bytes()), v))
            .collect();
        let deployed = DeployedContract {
            account_id,
            wasm_bytecode: data.wasm_bytecode,
            proof: data.proof,
            source: data.source,
            deployed_at_height: env::block_height(),
            state: runtime_state,
            contract: data.contract,
        };
        runtime.insert_contract(RuntimeAccountId::new(contract_name.clone()), deployed);

        let context = ExecutionContext {
            contract_id: RuntimeAccountId::new(contract_name.clone()),
            // A view call has no real caller/predecessor (predecessor_account_id()
            // is prohibited in views). Use the contract's own account ID for both
            // so the runtime has a valid context without touching host calls that
            // are disallowed in view mode.
            caller_id: RuntimeAccountId::new(env::current_account_id().to_string()),
            predecessor_id: RuntimeAccountId::new(env::current_account_id().to_string()),
            attached_deposit: 0,
            prepaid_gas: 0,
            block_height: env::block_height(),
            block_timestamp: env::block_timestamp(),
            random_seed: env::random_seed(),
        };

        runtime
            .view(&RuntimeAccountId::new(contract_name.clone()), &method_name, args)
            .map_err(|e| format!("View error: {}", e))
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
    /// Callback for cross-contract calls.
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

        let source = r#"
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

        let result = contract.deploy_contract("counter".to_string(), source.to_string());
        assert!(result.is_ok());
        assert_eq!(contract.get_contract_count(), 1);

        let context = get_context("alice.testnet".parse().unwrap());
        testing_env!(context);

        // First increment
        let result = contract
            .execute("counter".to_string(), "increment".to_string(), vec![])
            .expect("increment should succeed");
        let val = u64::from_le_bytes(result.as_slice().try_into().unwrap_or([0; 8]));
        assert_eq!(val, 1, "first increment should return 1");

        // Second increment
        let result = contract
            .execute("counter".to_string(), "increment".to_string(), vec![])
            .expect("increment should succeed");
        let val = u64::from_le_bytes(result.as_slice().try_into().unwrap_or([0; 8]));
        assert_eq!(val, 2, "second increment should return 2");

        // View call
        let result = contract
            .view("counter".to_string(), "get".to_string(), vec![])
            .expect("get should succeed");
        let val = u64::from_le_bytes(result.as_slice().try_into().unwrap_or([0; 8]));
        assert_eq!(val, 2, "get should return 2");
    }

    #[test]
    fn test_deploy_rejects_invalid_source() {
        let context = get_context("owner.testnet".parse().unwrap());
        testing_env!(context);

        let mut contract = RedSigmaContract::default();

        let result = contract.deploy_contract("bad".to_string(), "not valid source".to_string());
        assert!(result.is_err());
        assert_eq!(contract.get_contract_count(), 0);
    }

    #[test]
    fn test_execute_rejects_unknown_contract() {
        let context = get_context("alice.testnet".parse().unwrap());
        testing_env!(context);

        let mut contract = RedSigmaContract::default();
        let result = contract
            .execute("nonexistent".to_string(), "foo".to_string(), vec![])
            .expect_err("executing unknown contract should fail");
        assert!(result.contains("not found"));
    }
}
