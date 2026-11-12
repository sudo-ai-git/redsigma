# REDΣSIGMA

**REDΣSIGMA** (Red Sigma) — A Self-Verifying Smart Contract Language with Mathematical Integrity

> Where mathematical proof replaces trust. Every contract carries its own verification.

[![Build Status](https://img.shields.io/badge/build-passing-brightgreen)](https://github.com/sudosudo/redsigma/actions)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust 1.86+](https://img.shields.io/badge/rust-1.86+-orange.svg)](https://www.rust-lang.org)

## Overview

REDΣSIGMA is a smart contract language where every function carries a
machine-checked proof. `@verified(pre:, post:, invariant:)` clauses are parsed
into the AST and discharged by a tri-state z3 verifier over the straight-line
integer subset. A clause the verifier cannot model is **DECLINED** and reported
as `Unverified` — never as passed.

**Real today (each claim backed by an executed gate):**

- Hand-written recursive-descent parser: contracts, state, contract invariants,
  `@verified`, `let`/`const`/`if`/`while`/`for`/`match`, expressions, comments.
- z3 verifier (`verify/redsigma_verify.py`): `pre ∧ ¬post` unsat ⇒ **Proven**;
  sat ⇒ **Counterexample** with a model; unmodelled construct ⇒ **Unverified**.
- 231-gate / dual-stream / causal / 6-layer reflection integrity layers over
  parsed contracts (`cargo test --workspace`).
- NEAR test-contract crate + runtime verification library.

**Not implemented yet — do not cite as done:**

- `compile()` is a stub; no WASM is produced from REDΣSIGMA source.
- Refinement types (`VerifiedU64<invariant: ...>`), compound assignment (`+=`),
  contract-level `struct`/`enum`, `Map`/`Set`/`Result` are **design sketches**.
- Proof-carrying WASM codegen is a stub.

### Core Innovation: Mathematical Integrity as Language Primitive

Every REDΣSIGMA function carries its own proof. The verifier discharges the
`pre` / `post` / `invariant` clauses against the function body; state
transitions are tracked by the 231-gate checksum layers.

```redsigma
// REDΣSIGMA: every function carries its own proof
contract Counter {
    invariant count >= 0;

    state {
        count: u64,
    }

    @verified(
        pre: true,
        post: count == old(count) + 1,
        invariant: count >= 0
    )
    fn increment() -> u64 {
        count = count + 1;
        count;
    }
}
```

### What the verifier does and does not say

A `Proven` verdict means the clause follows from the modelled body for **all**
inputs satisfying the precondition, in the z3 model of that subset. It is not a
proof about the generated WASM (none is generated), and `Unverified` means
exactly nothing was proved.

## Mathematical Foundation

REDΣSIGMA integrates the **Tanakh 231-Gate Mathematical Framework** — a six-layer integrity system verified across the Hebrew Bible's 1.17M letters:

| Framework Component | REDΣSIGMA Usage |
|---------------------|-----------------|
| **231-Gate** (SUM/LINEAR/PRODUCT mod 231) | Every state transition checksum |
| **Dual-Stream** (Mod-5 + Mod-37/73) | Semantic/structural integrity per execution |
| **SemanticSpace** (5-bucket) | Deterministic projection of every state |
| **BijectiveVerifier** | Forward determinism + Inverse preimage |
| **CausalMemory** | Parent→child chains for every transaction |
| **6-Layer Self-Reflection** | Pre-execution verification |

**Verified Results**: 10/10 application spike tests passed at the framework
level — tamper-evident memory, adversarial compression, syndrome diagnostics,
primality-verified skills, self-verifying language, cross-lingual transfer,
curriculum scheduler, fugue decomposition, quantum-resistant VRE,
embedding-free retrieval. These exercise the 231-gate mathematics, not the
REDΣSIGMA compiler (which is a stub).

## Architecture

```
REDΣSIGMA Source
       ↓
[Parser + Invariant Extraction]  ← Hand-written recursive descent parser
       ↓
[z3 verification of @verified]   ← verify/redsigma_verify.py (REAL)
       ↓
[SemanticSpace Projection] → 5-bucket deterministic signature (Mod-5)
       ↓
[BijectiveVerifier] → Forward determinism + Inverse preimage
       ↓
[CausalMemory] → Parent→child causal chains
       ↓
[DualStream] → Mod-5 semantic + Mod-37/73 structural (100% joint uniqueness)
       ↓
[6-Layer Self-Reflection] → Consistency→Memory→Oracle→Completeness→Uncertainty→Contradiction
       ↓
[Proof-Carrying WASM] → WASM + Proof Objects   ← STUB (not implemented)
       ↓
[NEAR Deployment] + [Runtime Verification Library]  ← test contract builds; deployment DNS-blocked
```

## Quick Start

### Prerequisites

- Rust 1.86+
- cargo
- Python 3 + z3-solver (`pip install z3-solver`) for `@verified` verification

### Installation

```bash
git clone https://github.com/sudosudo/redsigma
cd redsigma
cargo build --release
```

### Running Tests

```bash
cargo test --workspace
python3 verify/redsigma_verify.py --selftest
```

## Usage Examples

### Verified Contract (parser-supported)

This is the example that actually parses and verifies today:

```redsigma
// examples/verified_counter.redsigma
contract Counter {
    invariant count >= 0;

    state {
        count: u64,
    }

    @verified(
        pre: true,
        post: count == old(count) + 1,
        invariant: count >= 0
    )
    fn increment() -> u64 {
        count = count + 1;
        count;
    }

    @verified(
        pre: count > 0,
        post: count == old(count) - 1,
        invariant: count >= 0
    )
    fn decrement() -> u64 {
        count = count - 1;
        count;
    }

    @verified(pre: true, post: count == old(count), invariant: count >= 0)
    fn get() -> u64 { count; }
}
```

Verify it (z3):

```bash
verify/verify.sh examples/verified_counter.redsigma
```

### Design sketches (NOT parser-supported)

`examples/simple_counter.redsigma`, `examples/basic_escrow.redsigma`,
`examples/verified_token.redsigma` and `examples/verified_dao.redsigma` record
**intended** syntax — refinement types such as
`VerifiedU64<invariant: count >= 0>`, compound assignment (`+=`), contract-level
`struct`/`enum`, `Map`/`Set`, `caller()`, `Result<(), Error>`, quantifiers — that
the hand-written parser does **not** accept yet. `emit_verification` exits
non-zero on them. They are not compilable REDΣSIGMA and must not be cited as
such.

## Using from Rust

```rust
use redsigma_parser::{Parser, Contract};
use redsigma_compiler::{compile, parse_contract, CompilerError};

fn main() -> Result<(), CompilerError> {
    let source = r#"
        contract Test {
            invariant balance >= 0;
            state { balance: u64 }
            fn deposit(amount: u64) -> u64 {
                balance += amount;
                balance
            }
        }
    "#;

    // Parse and validate
    let contracts = parse_contract(source)?;
    println!("Parsed {} contract(s)", contracts.len());

    // Compile to WASM (stub - full pipeline TODO)
    let wasm = compile(source)?;
    println!("Compiled to {} bytes", wasm.len());

    Ok(())
}
```

## Language Reference

### Types

| Type | Description |
|------|-------------|
| `u8`..`u64` | Unsigned integers (modelled by the z3 verifier) |
| `i8`..`i64` | Signed integers (modelled by the z3 verifier) |
| `u128` / `i128` | Parsed, but DECLINED by the z3 verifier (no 128-bit BitVec model) |
| `bool` | Boolean |
| `Vec<T>` | Typed generic (parsed; element operations are declined by the verifier) |
| `VerifiedU8`..`VerifiedU128<invariant: ...>` | **Planned** refinement types — NOT parser-supported yet |
| `String`, `AccountId`, `Map<K, V>`, `Set<T>`, `Option<T>`, `Result<T, E>` | **Planned** — parsed only as opaque type names, not modelled |

### Function Annotations

```redsigma
@verified(
    pre: <boolean expression>,      // Precondition (must hold before call)
    post: <boolean expression>,     // Postcondition (must hold after call)
    invariant: <boolean expression> // State invariant (must always hold)
)
fn function_name(params) -> ReturnType { ... }
```

Every function must carry `@verified`; a function without clauses is reported as
not passed, never as proven.

### State Declarations

```redsigma
state {
    field_name: u64,
    ...
}
```

The `Type<invariant: <boolean expression>>` refinement form shown in older
docs is **planned**, not parser-supported. State invariants go in the
`invariant:` clause of `@verified` and in contract-level `invariant` statements,
which the z3 verifier does check.

### Contract Invariants

```redsigma
contract Name {
    invariant <boolean expression>;
    invariant <boolean expression>;
    ...
}
```

## Crates

| Crate | Description |
|-------|-------------|
| `redsigma-parser` | Hand-written recursive descent parser |
| `redsigma-semantic` | SemanticSpace + BijectiveVerifier |
| `redsigma-causal` | CausalMemory with parent→child chains |
| `redsigma-dualstream` | Dual-Stream Gematria (Mod-5 + Mod-37/73) |
| `redsigma-reflection` | 6-Layer Self-Reflection Engine |
| `redsigma-codegen` | Proof-Carrying WASM Codegen |
| `redsigma-runtime` | NEAR Runtime Verification Library |
| `redsigma-stdlib` | Verified DeFi/DAO/Bridge Primitives |
| `redsigma-compiler` | Compiler Pipeline Orchestration |
| `redsigma-playground` | Browser IDE |

## NEAR Integration

REDΣSIGMA is designed for **NEAR Protocol** with three production integrations:

| Project | Integration | Status |
|---------|-------------|--------|
| **Co-op** (TypeScript) | `vre-integration.ts` wraps AgentRuntime | Code complete |
| **Setoff** (Rust) | `vre_integration.rs` wraps solver | ✅ Compiles + passes tests |
| **near-agent-guard** (Python) | `vre_integration.py` wraps Policy | ✅ 27 tests pass |

```python
# Co-op agent decisions
await verify_agent_decision(policy, pool, agent, decision)

# Setoff solver clearing
verify_clearing_with_vre(solver, asset, reductions, limits, source)

# near-agent-guard policy audit
await verify_guard_decision(intent, key, contract, method, now, gas, policy)
```

### REDΣSIGMA test contract — live on testnet

`redsigma.s03-gate.testnet` — code hash `5Tc9emgMWUQSmmBuh9VuXH4RD4ygniQ5ZqkGcz1VvFqq`,
storage 185,925 bytes. Deployed from
`target/wasm32-unknown-unknown/release/redsigma_test_contract.wasm`.

```bash
near view redsigma.s03-gate.testnet get_contract_count '{}'   # -> 2
near call redsigma.s03-gate.testnet deploy_contract \
  '{"contract_name":"counter","source":"..."}' \
  --accountId redsigma.s03-gate.testnet
```

This is a **handwritten Rust** NEAR contract, not one generated from a
`.redsigma` file — `compile()` is still a stub. Details and gotchas:
`DEPLOYMENT_STATUS.md`.

## Development

### Building

```bash
# Build all crates
cargo build --workspace

# Build specific crate
cargo build -p redsigma-parser

# Release build
cargo build --release --workspace
```

### Testing

```bash
# All tests
cargo test --workspace

# Specific crate
cargo test -p redsigma-parser

# With output
cargo test -p redsigma-parser -- --nocapture
```

### Code Quality

```bash
# Format
cargo fmt --workspace

# Lint
cargo clippy --workspace -- -D warnings

# Check
cargo check --workspace
```

### Verification (z3)

`@verified` clauses are discharged by a tri-state z3 verifier:

```bash
verify/verify.sh examples/verified_counter.redsigma   # a real contract
python3 verify/redsigma_verify.py --selftest          # the verifier's own gate
```

- **Proven** — `pre ∧ ¬goal` is unsat; the clause follows from the body.
- **Counterexample** — z3 produced a model; the clause is refuted.
- **Unverified** — the layer DECLINED to model something (division, calls,
  loops, mixed-width arithmetic, ...). It proved nothing and is never reported
  as True.

Exit code is 0 only when every function carries `@verified` and every
obligation is Proven. Integers are modelled as `BitVec` at the declared width
and signedness; `/` and `%` are declined rather than mis-modelled. The
self-test is falsifiable: reverting the `old()` pre-state fix, the unsigned
comparison fix, or body execution each makes it fail.

## Roadmap

### ✅ Completed (Phase 1)
- [x] Hand-written recursive descent parser
- [x] SemanticSpace + BijectiveVerifier
- [x] CausalMemory
- [x] Dual-Stream Gematria
- [x] 6-Layer Self-Reflection
- [x] Proof-Carrying WASM Codegen (stub)
- [x] NEAR Runtime Verification
- [x] Standard Library
- [x] Compiler Pipeline (stub)
- [x] 10/10 Mathematical Spike Tests Passed

### ✅ Completed (Phase 2a, verification)
- [x] `@verified` clauses parsed into the AST (were discarded)
- [x] Verification input exported as JSON (`compiler/examples/emit_verification.rs`)
- [x] z3 tri-state verifier with a falsifiable self-test (`verify/redsigma_verify.py`)
- [x] Parser supports `//` and `/* */` comments

### 🔄 In Progress (Phase 2b)
- [ ] Full compiler pipeline implementation (`compile()` is still a stub)
- [ ] WASM codegen with proof objects
- [ ] Refinement types (`VerifiedU64<invariant: ...>`) and compound assignment
- [ ] `@near/tanakh-vre-guard` shared library
- [ ] Testnet integration for all 3 projects (DNS-blocked at last attempt)

### 📋 Planned (Phase 3+)
- [ ] Unified Agent Runtime
- [ ] Formal verification bridge (Certora/Halmos specs from 231-gate)
- [ ] Mainnet-ready Agent Economy OS with insurance/slashing
- [ ] Cross-chain deployment (EVM, Solana, Cosmos)

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Run `cargo test --workspace` and `cargo clippy --workspace`
5. Submit a PR

## License

MIT License — see [LICENSE](LICENSE) for details.

## Citation

If you use REDΣSIGMA in academic work, please cite:

```bibtex
@software{redsigma2026,
  title = {REDΣSIGMA: Self-Verifying Smart Contract Language},
  author = {sudosudo},
  year = {2026},
  url = {https://github.com/sudosudo/redsigma}
}
```

## Acknowledgments

- **Tanakh 231-Gate Mathematical Framework** — Six-layer integrity system from the Hebrew Bible
- **Dual-Stream Gematria** — Mod-5 semantic + Mod-37/73 structural analysis
- **Verifiable Reasoning Engine (VRE)** — Three-layer verification (Syntax + Type + Theorem)
- **NEAR Protocol** — Target deployment platform

---

**REDΣSIGMA** — *Every function carries a proof; the modelled subset is discharged by z3.*