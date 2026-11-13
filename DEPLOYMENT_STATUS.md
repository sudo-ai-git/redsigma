# REDΣSIGMA Testnet Deployment Status

## Summary

- **Core implementation**: ✅ 118 tests passing, 0 failing (`cargo test --workspace`)
- **Verification pipeline**: ✅ z3 tri-state verifier + falsifiable self-test (5/5)
- **Testnet deployment**: ✅ **DEPLOYED AND VERIFIED** (was DNS-blocked; RPC is reachable again)

## Deployed Contract (verified 2026-11-12)

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

1. Implement `compile()` — the REDΣSIGMA→WASM pipeline is still a stub, so the
   deployed contract is handwritten Rust, not generated from a `.redsigma` file.
2. Deploy a contract generated from REDΣSIGMA source once (1) exists.
3. Wire `@verified` verification into the deploy gate (refuse to deploy a
   contract with `Unverified`/`Counterexample` obligations).
