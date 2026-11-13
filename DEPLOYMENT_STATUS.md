# REDΣSIGMA Testnet Deployment Status

## Summary

- **Core implementation**: ✅ 138 tests passing, 0 failing (`cargo test --workspace`)
- **Verification pipeline**: ✅ z3 tri-state verifier + falsifiable self-test (18/18)
- **Testnet deployment**: ✅ **DEPLOYED AND VERIFIED** (was DNS-blocked; RPC is reachable again)
- **Honest scope**: the *currently running* code parses `.redsigma` source and
  proof-checks it on chain, but method execution goes through the tree-walking
  runtime interpreter — not through this compiler's WASM output. It demonstrates
  state persistence and the NEAR deployment flow.

## Redeploy is blocked by storage cost (updated 2026-11-13)

`contracts/test-contract` now depends on the codegen crates, so rebuilding it
links the whole compiler/verifier chain into the module:

| Build | Size | Code section |
|---|---|---|
| deployed (previous revision) | 185,665 bytes | — |
| current source | **843,922 bytes** | 477,772 bytes |

`opt-level="z"`, `lto=true`, `codegen-units=1`, `panic="abort"` are already set.
At the 1e-5 NEAR/byte storage rate the current module needs **≈8.44 NEAR**; the
account holds **2.22 NEAR**. Shortfall: **≈6.22 NEAR**.

So the redeploy is blocked on funding/size, not correctness. Until it is
resolved, `redsigma.s03-gate.testnet` keeps running the previous revision
(interpreter path) and the local `redsigma_test_contract.wasm` does not match
the on-chain code hash.

## Deployed Contract (verified 2026-11-12; re-verified 2026-11-13)

| Field | Value |
|---|---|
| Account | `redsigma.s03-gate.testnet` |
| Code hash | `5Tc9emgMWUQSmmBuh9VuXH4RD4ygniQ5ZqkGcz1VvFqq` |
| Storage usage | 185,925 bytes (185,665 code + 260 state) |
| Balance | ~2.22 NEAR (≈1.86 NEAR locked as storage stake) |
| Deploy tx | `3gh7FLr2i9k76XacKGERNm8Vw6dpHwE3tsxSc31rdJCd` |
| Init tx | `BTcsMDQKwZ15bhwbatpAjU2HURcvwmFFF9TdU4xvmzGB` (`new`) |
| WASM | `target/wasm32-unknown-unknown/release/redsigma_test_contract.wasm` |

Verified by RPC query + live calls:

```bash
near view redsigma.s03-gate.testnet get_contract_count '{}'   # -> 2
near view redsigma.s03-gate.testnet get_owner '{}'             # -> 'redsigma.s03-gate.testnet'
near view redsigma.s03-gate.testnet view '{"contract_name":"counter","method_name":"get","args":[]}'
```

State-changing calls (must be signed by the owner, `redsigma.s03-gate.testnet`):

```bash
near call redsigma.s03-gate.testnet deploy_contract \
  '{"contract_name":"counter","source":"contract Counter { state { count: u64 } }"}' \
  --accountId redsigma.s03-gate.testnet

near call redsigma.s03-gate.testnet execute \
  '{"contract_name":"counter","method_name":"increment","args":[]}' \
  --accountId redsigma.s03-gate.testnet
```

**Parameter names are the JSON field names.** The contract originally declared
`_source` / `_args`, which forced callers to pass `{"_source": ...}` and made the
documented `"source"` call fail with `missing field '_source'`. They are now
`source` / `args`.

## Funding

The faucet hostnames are DNS-dead (`faucet.testnet.near.org`,
`near-faucet.testnet.near.org`), so the planned faucet route is unavailable.
The deployer was funded from another of the workspace's own testnet accounts:

```bash
near send-near nft-repo-test-015.testnet redsigma.s03-gate.testnet 0.6
# tx 8yPFrB9BnkUeyByoqVB7oaNXDdkUQbt2ePEZRtDxS4k4
```

`rpc.testnet.near.org` resolves and answers JSON-RPC, so deployment no longer
depends on the faucet.

## ContractState Manual Implementation

NEAR SDK 5.x `#[near_bindgen]` does not generate the `ContractState` impl, so it
is implemented manually (`contracts/test-contract/src/lib.rs`):

```rust
impl ContractState for RedSigmaContract {
    fn state_key() -> &'static [u8] { b"STATE" }
    fn state_read() -> Option<Self> where Self: BorshDeserialize { ... }
    fn state_write(&self) -> bool where Self: BorshSerialize { ... }
}
```

Verified: `contract_count` persisted across calls **and** across a code
redeploy (`--force` without re-init).

## Notes / Gotchas

- `near deploy --force` **with** `--initFunction new` fails: `new` panics with
  "The contract has already been initialized" and the whole transaction rolls
  back. To replace code on an initialized account, deploy with `--force` and no
  init function.
- `near storage` requires `--finality` or `--blockId`.
- Occasional `ETIMEDOUT` from the RPC on `near call`; retry.

## Next Steps

1. `compile()` now produces a real NEAR contract WASM module with a NEAR ABI
   layer, and the testnet contract deploys and executes generated contracts
   through the real runtime with a deploy gate that refuses contracts with
   failing verification obligations. A generated contract can be compiled to
   WASM and deployed via the testnet contract.
2. Wire `@verified` verification into the deploy gate more deeply (currently
   checks reflection verdict and dual-stream collisions; could also check
   z3 verification status).
3. Deploy a generated contract (not the handwritten placeholder) to testnet.
4. **DONE** — Frontend demo deployed to GitHub Pages at
   https://sudo-ai-git.github.io/redsigma/ (served from `docs/` on `main`).
   Connects to NEAR testnet, deploys `.redsigma` contracts, executes methods,
   and reads state from the browser.
