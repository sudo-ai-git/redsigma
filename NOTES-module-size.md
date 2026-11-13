# REDΣSIGMA — NEAR module size (session 2026-11-13)

Repo: /home/sudosudo/.openclaw/workspace/redsigma (git, branch main)

## Measured module sizes (wasm32-unknown-unknown release)

| build | bytes |
|---|---|
| deployed on-chain (previous revision) | 185,665 |
| first measurement this session | 843,922 |
| after near-sdk default features (wee_alloc) restored | 838,148 |

Cost at the 1e-5 NEAR/byte rate: 838,148 -> ~8.38 NEAR. Account
`redsigma.s03-gate.testnet` holds ~2.22 NEAR. Shortfall ~6.16 NEAR.
Redeploy is size/funding blocked, NOT broken.

## What moved the needle

- `CodegenPipeline::verify_only()` in `verify_proof`: **+1,116 bytes, no
  reduction.** LTO had already eliminated the path. Kept for intent.
- near-sdk default features restored (re-enables `wee_alloc`): **-6,890.**
- These are the ONLY two real wins; net -5,774.

## Falsified — do not repeat

- removing one `serde_json` call site: -345 bytes
- `serde_json` with `default-features=false, features=["alloc"]`: 0 bytes
- near-sdk WITHOUT default features: **+6,630** (bigger; it disables wee_alloc)
- `RUSTFLAGS="-Cpanic=immediate-abort"`: needs nightly + `-Zunstable-options`.
  Toolchain is stable only.
- wasm-snip / wasm-opt / wasm-strip: not installed, no binaryen.

## Where the bytes are (decoded from name + code sections)

Attributed by symbol, not guessed:

- `serde_json` (serialize/deserialize/format) ~89,000
- `redsigma_parser` ~23,000
- `core::panic` / formatting ~12,300
- `redsigma_codegen` ~12,400

1,938 function bodies; the 11 largest exceed 4 KB and hold ~1/3 of all
body bytes. Largest single body: `emit_match_arm_chain` at 9,588 bytes.

## The obvious next step, and why it was NOT taken

`ContractData` (Vec<u8>/String/ProofObject/Contract/HashMap) is the only
thing the contract feeds to `serde_json`. It is trivially borsh-serialisable
and borsh is already a dependency. Blocked because:

1. `ProofObject` and `Contract` are serde-only in `parser`/`codegen` —
   borsh derives must be added across those crates.
2. The deployed contract holds 2 entries already serialised as JSON; a
   borsh switch needs a read-side migration or they become unreadable.

So it needs cross-crate derives + a state-format migration. Deliberate,
documented, not a drive-by.

## How to decode a wasm module this way (no wabt/binaryen available)

Python: walk sections after the 8-byte header (section id u8, size LEB128).
Named functions live in the custom section named `name`, subsection id 1;
each entry is (index LEB, name_len LEB, name bytes). Pair with body sizes
from the code section to attribute bytes per symbol. Export entries are
(name_len LEB, name, kind u8, index LEB) in section id 7.
Validated with `wasmtime.Module(Engine, bytes)` from the wasmtime python bindings.
