# REDΣSIGMA Testnet Deployment Status

## Current state (verified by execution, 2026-11-13)

The borsh-only `contracts/test-contract` is deployed and answering views on
NEAR testnet. `.redsigma` source is parsed, proof-checked, compiled through the
full verification pipeline, and stored on chain; method execution runs through
the `NearRuntime` interpreter over the contract AST with proof re-verification
and invariant enforcement.

| Field | Value |
|---|---|
| Account | `redsigma.s03-gate.testnet` |
| Code hash | `EruzgySJeZD7UoLHwtZE5usKUpRFzsH4WMVzdSxMqeo3` |
| Storage usage | 639,182 bytes (~6.39 NEAR locked) |
| Balance | ~9.19 NEAR |

**Verify the live account yourself** (testnet block explorers block anonymous
lookups, so `near state`/`near view` are the authoritative source):

```bash
near state redsigma.s03-gate.testnet --networkId testnet
near view redsigma.s03-gate.testnet get_contract_count '{}' --networkId testnet   # -> 1
near view redsigma.s03-gate.testnet get_owner '{}' --networkId testnet            # -> redsigma.s03-gate.testnet
near view redsigma.s03-gate.testnet view '{"contract_name":"counter","method_name":"get","args":[]}' --networkId testnet  # -> 3
```

## On-chain transactions (verified by resulting account state)

| Action | Transaction hash |
|---|---|
| Fund account (7 NEAR from `nft-repo-test-015.testnet`) | `GiUgkQUQaZSbCM8gGDJcifV9deaih4CScd1GgmzUKpmc` |
| Redeploy borsh-only contract | `CwjHUzAse9PeHbhrJDryzEj7jNwdR7acUN4H3JQX2wjv` |
| Deploy `counter` contract | `7LNKtcLKvTqXtVe9dsFAq65Wm62ps8atV8RuxDVtSjYg` |
| `increment` #1 | `CsG7rGcfVh6V5wCGRPGkr4jokDn7TnH1oQYk2JwHNHQp` |
| `increment` #2 | `7NqkaNUafbjWRHrQiWqGAtRKsG7hNcoWALVn5CX6qYAU` |

> Note: the testnet block explorer (NearBlocks) returns 403 for anonymous
> testnet lookups, so explorer links are omitted. `near state` / `near view` are
> the authoritative, explorer-independent verification.

## What changed since the previous (185,925-byte) deployment

The previous on-chain revision (`code hash 5Tc9emgMWUQSmmBuh9VuXH4RD4ygniQ5ZqkGcz1VvFqq`,
storage 185,925 bytes) used `serde_json` for `ContractData` serialization and
had a `ContractState::state_read` that panicked on the post-borsh state layout.
The current deployment:

1. **Migrated `ContractData` serialization from `serde_json` to `borsh`.** All
   types that flow into storage (`Contract`, `ProofObject`, `WasmCompilation`,
   `SemanticReport`, `DualStreamReport`, `ReflectionReport`) derive
   `BorshSerialize/BorshDeserialize`. Removed the now-unused `serde`/`serde_json`
   dependencies from `contracts/test-contract/Cargo.toml`.
2. **Made `state_read` resilient to legacy state.** The on-chain `STATE` blob
   predated the `contracts` HashMap field, so the new borsh schema failed to
   deserialize ("Cannot deserialize contract state"). `state_read` now salvages
   `owner_id` from the raw legacy bytes and starts fresh with an empty contracts
   map — no contract data was lost (the legacy map was empty).
3. **Fixed `view()` prohibited-host-call bug.** `view()` previously called
   `env::predecessor_account_id()` / `env::prepaid_gas()` / `env::attached_deposit()`,
   which are prohibited in NEAR view calls (`ProhibitedInView`). It now uses
   `env::current_account_id()` for caller/predecessor and `0` for deposit/gas.

## Module size

Fresh release build of `contracts/test-contract`: **636,732 bytes**
(~6.37 NEAR at 1e-5 NEAR/byte). On-chain storage usage is 639,182 bytes
(~6.39 NEAR locked). Account holds ~9.19 NEAR, leaving ~2.80 NEAR headroom.

The earlier "838,148 bytes / ≈8.38 NEAR needed / ≈6.16 NEAR shortfall" figures
in prior revisions of this document are **superseded** — the borsh migration
and the successful redeploy make them obsolete. Redeploy is **not** blocked;
it has been completed.

## Verification gates

- `cargo test --workspace`: all tests pass, 0 failures
- `python3 verify/redsigma_verify.py --selftest`: 20/20 pass (mutation-checked)
- `python3 verify/compile_examples.py`: all 4 examples → valid wasmtime-accepted WASM
- `bash verify/check_examples.sh`: 2 proven (exit 0), 2 correctly rejected (exit 1)
- `python3 verify/execute_examples.py`: 4/4 execution cases pass (the compiled
  WASM actually executes correctly with mocked NEAR host functions)

## Honest scope

Method execution on-chain runs through the tree-walking `NearRuntime`
interpreter over the contract AST (with per-width/signedness integers matching
the verifier's BitVec model), **not** through direct instantiation of the
generated proof-carrying WASM. The WASM is stored, validated, and proven to
execute correctly off-chain (`verify/execute_examples.py`); on-chain execution
dispatches on the AST. This is the known gap to close before the project is
unconditionally competition-ready.

The 231-gate / dual-stream / reflection integrity layers are consistency
tooling, explicitly **not** a security mechanism.
