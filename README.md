# REDΣSIGMA

**REDΣSIGMA** — a smart contract language where every function carries a
machine-checked proof.

You write `@verified(pre:, post:, invariant:)` clauses on your functions. The
clauses are parsed into the AST, exported as JSON, and discharged by a
**tri-state z3 verifier** that symbolically executes the function body:

| Verdict | Meaning |
|---------|---------|
| **Proven** | `pre ∧ ¬goal` is unsatisfiable — the clause follows from the body for every input satisfying the precondition. |
| **Counterexample** | z3 produced a concrete model — the clause is refuted, and the model is printed. |
| **Unverified** | the verifier *declined* to model something (division, a call, a loop, …). It proved nothing, and it is never reported as `True`. |

The exit code is `0` only when **every** function carries `@verified` and every
obligation is `Proven`.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust 1.86+](https://img.shields.io/badge/rust-1.86+-orange.svg)](https://www.rust-lang.org)

---

## Quick start

### Prerequisites

- **Rust 1.86+** and `cargo` — the parser, compiler and integrity layers
- **Python 3** with **`z3-solver`** — the `@verified` verifier

```bash
pip install z3-solver
```

### Build and test

```bash
git clone https://github.com/sudo-ai-git/redsigma
cd redsigma
cargo build --release --workspace
cargo test --workspace          # 106 tests
```

### Verify your first contract

```bash
verify/verify.sh examples/verified_counter.redsigma
```

---

## How it works

```
   your .redsigma file
        │
        ▼
┌───────────────────────────────────────────────────────────────┐
│ 1. Parser (redsigma-parser)                                   │
│    hand-written recursive descent; captures pre / post /       │
│    invariant as AST expressions, plus the function body        │
└───────────────────────────────────────────────────────────────┘
        │
        ▼
┌───────────────────────────────────────────────────────────────┐
│ 2. Verification input (redsigma-compiler --example            │
│    emit_verification)                                          │
│    state fields, parameters, and per function: the clause ASTs │
│    and the statement list — as JSON                            │
└───────────────────────────────────────────────────────────────┘
        │
        ▼
┌───────────────────────────────────────────────────────────────┐
│ 3. z3 verifier (verify/redsigma_verify.py)                    │
│    for each clause: build a z3 model of the body, then ask     │
│    "is pre ∧ ¬goal satisfiable?"                               │
│      unsat   → Proven                                          │
│      sat     → Counterexample (model printed)                  │
│      decline → Unverified                                      │
└───────────────────────────────────────────────────────────────┘
```

### Step 2 in detail — what the verifier receives

For `increment` in `examples/verified_counter.redsigma`, the exporter emits:

```json
{
  "name": "increment",
  "params": [],
  "return_type": "u64",
  "pre": { "Bool": true },
  "post": { "Binary": {
      "left":  { "Ident": "count" },
      "op": "Eq",
      "right": { "Binary": {
          "left":  { "Call": { "func": "old", "args": [ { "Ident": "count" } ] } },
          "op": "Add",
          "right": { "Number": "1" }
      } }
  } },
  "invariant": { "Binary": {
      "left": { "Ident": "count" }, "op": "Ge", "right": { "Number": "0" }
  } },
  "body": [
    { "Expr": { "Binary": {
        "left":  { "Ident": "count" },
        "op":    "Assign",
        "right": { "Binary": {
            "left": { "Ident": "count" }, "op": "Add", "right": { "Number": "1" }
        } }
    } } },
    { "Expr": { "Ident": "count" } }
  ]
}
```

Every node is one of: `Number`, `Bool`, `Ident`, `Binary`, `Unary`, `Call`,
`Unit`. `old(x)` is an ordinary call node — the verifier binds it to the
**pre-state** value of `x`.

### Step 3 in detail — what the verifier does

1. Bind each state field to a fresh z3 symbol at its declared **width and
   signedness** (`u64` → 64-bit unsigned `BitVec`), and each parameter to a
   symbolic input.
2. Evaluate `pre`. If it cannot be modelled, the function is `Unverified`.
3. Symbolically execute the body statement by statement, updating the state
   bindings. `old(x)` is resolved against the bindings captured **before** the
   body ran.
4. For each goal (`post`, `invariant`) ask z3 whether `pre ∧ ¬goal` is
   satisfiable:
   - **unsat** → the goal is implied → `Proven`
   - **sat** → z3's model is a concrete counterexample → `Counterexample`
5. A construct the verifier cannot model is **declined**, recorded, and the
   remaining obligations are still checked.

---

## Tutorial: a verified counter

`examples/verified_counter.redsigma`:

```redsigma
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

    // A view still carries its proof: it must not change state.
    @verified(pre: true, post: count == old(count), invariant: count >= 0)
    fn get() -> u64 { count; }
}
```

Run it:

```bash
$ verify/verify.sh examples/verified_counter.redsigma
```

```json
{
 "contracts": [
  {
   "contract": "Counter",
   "functions": [
    {
     "declines": [],
     "name": "increment",
     "obligations": [
      { "detail": "", "kind": "post",       "model": null, "status": "Proven" },
      { "detail": "", "kind": "invariant", "model": null, "status": "Proven" }
     ],
     "passed": true
    },
    {
     "declines": [],
     "name": "decrement",
     "obligations": [
      { "detail": "", "kind": "post",       "model": null, "status": "Proven" },
      { "detail": "", "kind": "invariant", "model": null, "status": "Proven" }
     ],
     "passed": true
    },
    {
     "declines": [],
     "name": "get",
     "obligations": [
      { "detail": "", "kind": "post",       "model": null, "status": "Proven" },
      { "detail": "", "kind": "invariant", "model": null, "status": "Proven" }
     ],
     "passed": true
    }
   ],
   "summary": {
    "all_proven": true, "counterexample": 0, "declines": 0,
    "proven": 6, "unverified": 0
   }
  }
 ],
 "summary": { "all_proven": true, "contracts": 1 }
}
```

Exit code `0`.

Reading the result: `increment`'s body sets `count = count + 1`, so the
post-state `count` equals the pre-state `count + 1` — exactly what `post` says.
z3 proves `pre ∧ ¬post` unsatisfiable. The same holds for `decrement` (whose
precondition `count > 0` makes the subtraction safe) and for `get` (which
changes nothing).

---

## Examples

Every example below is a real file in `examples/`. Run any of them with
`verify/verify.sh <file>`.

### 1. A false postcondition → `Counterexample`

`examples/counterexample.redsigma` claims the body adds `amount`, but it
actually adds `amount + 1`:

```redsigma
contract Vault {
    invariant balance >= 0;

    state { balance: u64 }

    @verified(
        pre: amount > 0,
        post: balance == old(balance) + amount,
        invariant: balance >= 0
    )
    fn deposit(amount: u64) -> u64 {
        balance = balance + amount + 1;
        balance;
    }
}
```

```json
{
 "contract": "Vault",
 "functions": [
  {
   "name": "deposit",
   "obligations": [
    {
     "kind": "post",
     "status": "Counterexample",
     "detail": "pre ∧ ¬goal is satisfiable",
     "model": { "amount": "18446744073709551615", "balance": "0" }
    },
    { "kind": "invariant", "status": "Proven" }
   ],
   "passed": false
  }
 ],
 "summary": { "all_proven": false, "counterexample": 1, "proven": 1 }
}
```

Exit code `1`. The model is a genuine counterexample: with `amount = 2^64 − 1`
and a starting balance of `0`, the body computes `0 + (2^64−1) + 1`, which
**wraps to `0`** in 64-bit arithmetic, so the post-state balance is `0` while
the postcondition demands `2^64 − 1`. The verifier models wrapping exactly as
the machine does, so it catches the bug instead of "proving" it away.

### 2. A construct that is declined → `Unverified`

`examples/declined_division.redsigma`:

```redsigma
contract Split {
    invariant x >= 0;

    state { x: u64 }

    @verified(pre: true, post: x == old(x) / 2, invariant: x >= 0)
    fn halve() -> u64 {
        x = x / 2;
        x;
    }
}
```

```json
{
 "contract": "Split",
 "functions": [
  {
   "name": "halve",
   "obligations": [
    { "kind": "post", "status": "Unverified",
      "detail": "body: integer division `/` is not modelled (declined, not guessed)" },
    { "kind": "invariant", "status": "Unverified",
      "detail": "body: integer division `/` is not modelled (declined, not guessed)" }
   ],
   "declines": ["body: integer division `/` is not modelled (declined, not guessed)"],
   "passed": false
  }
 ],
 "summary": { "all_proven": false, "unverified": 2, "declines": 1 }
}
```

Exit code `1`. Integer division is **declined, not guessed**: WASM integer
division truncates toward zero and has signed-overflow edge cases that z3's
`BitVec` division does not reproduce, so modelling one as the other could
produce a false proof. The verifier says so instead.

### 3. Unsigned arithmetic

`examples/unsigned.redsigma`:

```redsigma
contract Unsigned {
    invariant x >= 0;

    state { x: u64 }

    @verified(pre: true, post: x > 0, invariant: x >= 0)
    fn set_max() -> u64 {
        x = 18446744073709551615;   // 2^64 - 1, the largest u64
        x;
    }
}
```

Result: **both obligations Proven**, exit `0`.

`18446744073709551615` is a large *positive* `u64`. Signedness cannot be read
off a z3 term — the same 64 bits are `−1` as `i64` — so the verifier carries
the declared type with every binding and uses the unsigned comparison operators
(`ULT`/`UGT`/`ULE`/`UGE`) for `u64`. A signed model would refute `x > 0` here.

### 4. `old()` refers to the pre-state

`old(x)` is bound to the value `x` had **before** the function body ran. This is
what makes postconditions about a state change expressible at all:

```redsigma
@verified(pre: true, post: count == old(count) + 1, invariant: count >= 0)
fn increment() -> u64 { count = count + 1; count; }
```

The verifier captures the state bindings before executing the body and resolves
`old(count)` against that snapshot. Evaluating `old(count)` against the
post-state instead would make `x = 0; post: x == old(x)` "prove" — the verifier
does not do that, and its self-test fails if it regresses.

---

## The verifier's own self-test

```bash
$ python3 verify/redsigma_verify.py --selftest
PASS  deposit proves post and invariant: {'post': 'Proven', 'invariant': 'Proven'}
PASS  old() reads the PRE-state, not the post-state: {'post': 'Counterexample', 'invariant': 'Proven'}
PASS  a body that adds 2 does not prove a +1 postcondition: {'post': 'Counterexample', 'invariant': 'Proven'}
PASS  division is declined, never modelled: {'post': 'Unverified', 'invariant': 'Unverified'}
PASS  u64 comparisons are unsigned (2^64-1 > 0): {'post': 'Proven', 'invariant': 'Proven'}

self-test: 5/5 cases as expected
```

Each case names the verdict it expects, so the gate is falsifiable: reverting
the `old()` pre-state fix, the unsigned-comparison fix, or body execution each
makes a case fail.

---

## Modelled subset

| Construct | Status |
|-----------|--------|
| `u8` `u16` `u32` `u64` | modelled (unsigned `BitVec`) |
| `i8` `i16` `i32` `i64` | modelled (signed `BitVec`) |
| `u128` `i128` | parsed, **declined** (no 128-bit model) |
| `bool` | modelled |
| `+ - *` `& \| ^ << >>` | modelled |
| `< <= > >= == !=` | modelled (signedness from the declared type) |
| `&& \|\| !` | modelled |
| `/` `%` | **declined** |
| calls, loops, `if`/`while`/`for`/`match`, strings, arrays, structs, indexing, field access | **declined** |
| mixed-width or mixed-signedness arithmetic | **declined**, not silently extended |

A declined construct is recorded and the remaining obligations are still
checked — one unsupported statement does not abandon the function.

---

## Language reference

### Contracts, state, invariants

```redsigma
contract Counter {
    invariant count >= 0;          // contract-level invariant

    state {
        count: u64,                // fields with declared types
    }

    // functions...
}
```

### Function annotations

```redsigma
@verified(
    pre: <boolean expression>,       // must hold before the call
    post: <boolean expression>,      // must hold after the call; old(x) = pre-state x
    invariant: <boolean expression>  // state invariant this call must preserve
)
fn name(params) -> ReturnType { ... }
```

Every function must carry `@verified`; a function with no clauses is reported as
**not passed**, never as proven.

### Statements and expressions

`let` / `let mut` / `const`, assignment `x = v`, `return`, `if` / `else`,
`while`, `for x in a..b`, `match`, `break`, `continue`, blocks, and the
operators listed above.

### Types

`u8`–`u64`, `i8`–`i64`, `bool`, `Vec<T>`, and opaque type names (`String`,
`AccountId`, `Map<K, V>`, …). Only the modelled subset in the table above is
verified; the rest are parsed and declined.

---

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
                balance = balance + amount;
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

---

## Architecture

```
.redsigma source
      ↓
[redsigma-parser]            recursive descent; AST carries pre/post/invariant + body
      ↓
[redsigma-compiler]          parse_contract + verification_input JSON export
      ↓
[verify/redsigma_verify.py]  z3 tri-state verifier
      ↓
[integrity layers]            SemanticSpace · BijectiveVerifier · CausalMemory ·
                              DualStream · 6-Layer Reflection · 231-gate checksums
      ↓
[redsigma-codegen]           WASM + proof objects        (stub)
      ↓
[redsigma-runtime]           NEAR deployment + verification
```

| Crate | Responsibility |
|-------|----------------|
| `redsigma-parser` | Hand-written recursive descent parser |
| `redsigma-semantic` | SemanticSpace + BijectiveVerifier |
| `redsigma-causal` | CausalMemory with parent→child chains |
| `redsigma-dualstream` | Dual-stream integrity (Mod-5 + Mod-37/73) |
| `redsigma-reflection` | 6-layer self-reflection engine |
| `redsigma-codegen` | Proof-carrying WASM codegen (stub) |
| `redsigma-runtime` | NEAR deployment / execution / proof verification |
| `redsigma-stdlib` | Standard library primitives |
| `redsigma-compiler` | Pipeline orchestration + verification input export |

---

## NEAR testnet — live

A NEAR contract exercising the same state/verification model is deployed:

| | |
|---|---|
| Account | `redsigma.s03-gate.testnet` |
| Code hash | `5Tc9emgMWUQSmmBuh9VuXH4RD4ygniQ5ZqkGcz1VvFqq` |
| Storage | 185,925 bytes |

```bash
# read
near view redsigma.s03-gate.testnet get_contract_count '{}'    # -> 2
near view redsigma.s03-gate.testnet get_owner '{}'              # -> redsigma.s03-gate.testnet

# write (must be signed by the owner)
near call redsigma.s03-gate.testnet deploy_contract \
  '{"contract_name":"counter","source":"..."}' \
  --accountId redsigma.s03-gate.testnet

near call redsigma.s03-gate.testnet execute \
  '{"contract_name":"counter","method_name":"increment","args":[]}' \
  --accountId redsigma.s03-gate.testnet
```

This is a handwritten Rust NEAR contract, not one generated from a `.redsigma`
file — see **Project status** below.

---

## Mathematical foundation

REDΣSIGMA integrates the **Tanakh 231-gate** integrity framework (SUM / LINEAR /
PRODUCT mod 231) with dual-stream gematria (Mod-5 semantic + Mod-37/73
structural), a 5-bucket semantic projection, a bijective verifier, causal
parent→child memory chains, and a 6-layer self-reflection engine
(Consistency → Memory → Oracle → Completeness → Uncertainty → Contradiction).

Ten application spike tests were executed and passed at the framework level
(tamper-evident memory, adversarial compression, syndrome diagnostics,
primality-verified skills, self-verifying language, cross-lingual transfer,
curriculum scheduler, fugue decomposition, quantum-resistant VRE,
embedding-free retrieval). They exercise the 231-gate mathematics, not the
compiler pipeline.

---

## Project status

**Working and verified by execution:**

- Parser for the subset described above; `@verified` clauses captured as AST.
- z3 tri-state verifier with a falsifiable 5/5 self-test.
- Integrity layers: SemanticSpace, BijectiveVerifier, CausalMemory, DualStream,
  6-layer reflection, 231-gate checksums.
- `cargo test --workspace`: 106 tests passing.
- A NEAR test contract, deployed and exercised on testnet.

**Not implemented yet:**

- `redsigma_compiler::compile()` is a stub — no WASM is generated from
  `.redsigma` source. The deployed NEAR contract is handwritten Rust.
- `redsigma-codegen` proof-carrying WASM is a stub.
- Refinement types (`VerifiedU64<invariant: ...>`), compound assignment (`+=`),
  contract-level `struct`/`enum`, `Map`/`Set`/`Result`, and quantifiers are
  design sketches (see the headers of `examples/simple_counter.redsigma`,
  `examples/basic_escrow.redsigma`, `examples/verified_token.redsigma`,
  `examples/verified_dao.redsigma`).

---

## Contributing

1. Fork the repository
2. Create a feature branch
3. Run `cargo test --workspace` and `python3 verify/redsigma_verify.py --selftest`
4. Submit a PR

## License

MIT — see [LICENSE](LICENSE).

## Citation

```bibtex
@software{redsigma2026,
  title = {REDΣSIGMA: Self-Verifying Smart Contract Language},
  author = {sudosudo},
  year = {2026},
  url = {https://github.com/sudo-ai-git/redsigma}
}
```
