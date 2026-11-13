# REDΣSIGMA — NEAR module size (session 2026-11-13)

Repo: /home/sudosudo/.openclaw/workspace/redsigma (git, branch main)

## Current state

The borsh migration is complete and the contract is redeployed on NEAR testnet
(`redsigma.s03-gate.testnet`). See `DEPLOYMENT_STATUS.md` for the live account
state and transaction evidence.

| Build | Bytes |
|---|---|
| Fresh release build (`contracts/test-contract`) | 636,732 |
| On-chain storage usage (`near state`) | 639,182 (~6.39 NEAR locked) |
| Account balance | ~9.19 NEAR |

## Cost

At the 1e-5 NEAR/byte storage rate, 636,732 bytes ≈ 6.37 NEAR. The account
holds ~9.19 NEAR with ~6.39 NEAR locked as storage stake, leaving ~2.80 NEAR
headroom. Redeploy is **not** blocked — it has been completed.

The earlier figures in prior revisions of this note (838,148 bytes ≈ 8.38 NEAR
against a 2.22 NEAR balance, ≈6.16 NEAR shortfall) are **superseded** — they
predated the borsh migration and the successful redeploy.

## Borsh migration (done)

`ContractData` serialization switched from `serde_json` to `borsh`:
- `Contract`, `ProofObject`, `WasmCompilation`, and all nested report types
  (`SemanticReport`, `DualStreamReport`, `ReflectionReport`) now derive
  `BorshSerialize/BorshDeserialize` across `parser`/`codegen`/`semantic`/
  `dualstream`/`reflection`/`runtime`.
- Removed the unused `serde`/`serde_json` dependencies from
  `contracts/test-contract/Cargo.toml`.
- `state_read` made resilient to the legacy (pre-borsh) `STATE` blob so the
  contract keeps working after redeploy; no contract data was lost (the legacy
  `contracts` map was empty).

## Where the bytes are (decoded from name + code sections)

Attributed by symbol, not guessed (from a prior 838,148-byte build; the current
636,732-byte build reflects the borsh migration):

- `serde_json` (serialize/deserialize/format) ~89,000 — removed from the
  contract storage path; no longer in the on-chain module
- `redsigma_parser` ~23,000
- `core::panic` / formatting ~12,300
- `redsigma_codegen` ~12,400

1,938 function bodies; the 11 largest exceed 4 KB and hold ~1/3 of all body
bytes. Largest single body: `emit_match_arm_chain` at 9,588 bytes.

## How to decode a wasm module this way (no wabt/binaryen available)

Python: walk sections after the 8-byte header (section id u8, size LEB128).
Named functions live in the custom section named `name`, subsection id 1;
each entry is (index LEB, name_len LEB, name bytes). Pair with body sizes
from the code section to attribute bytes per symbol. Export entries are
(name_len LEB, name, kind u8, index LEB) in section id 7.
Validated with `wasmtime.Module(Engine, bytes)` from the wasmtime python bindings.
