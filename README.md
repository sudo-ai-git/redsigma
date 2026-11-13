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
cargo test --workspace          # 121 tests
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
PASS  && on booleans is modelled: {'post': 'Proven'}
PASS  WASM shift count is modulo the width: {'post': 'Proven'}
PASS  hex literal 0xFF is 255: {'post': 'Proven'}
PASS  literal beyond u64 is declined, not truncated: {'post': 'Unverified'}
PASS  boolean arithmetic is declined (z3 sort mismatch): {'post': 'Unverified'}
PASS  -2**63 is representable as an i64 literal: {'post': 'Proven'}
PASS  a local cannot shadow a state field: {'post': 'Unverified'}
PASS  contract-level invariants are discharged per function: {'post': 'Proven', 'contract_invariant[0]': 'Counterexample'}
PASS  mixed-width assignment is declined, not silently truncated: {'post': 'Unverified'}
PASS  a parameter cannot shadow a state field: no obligations
PASS  a parameter named like the old state symbol does not alias it: {'post': 'Proven'}
PASS  an unannotated function is not passed even with contract invariants: no obligations
PASS  literal adopts the typed operand's width: {'post': 'Proven'}

self-test: 18/18 cases as expected
```

Each case names the verdict it expects, so the gate is falsifiable: reverting
the `old()` pre-state fix, the unsigned-comparison fix, the shift masking, the
`&&`/`||` placement, the literal range check, the boolean/integer check, the
`-2**63` folding, or body execution each makes a case fail (mutation-checked).

---

## Modelled subset

| Construct | Status |
|-----------|--------|
| `u8` `u16` `u32` `u64` | modelled (unsigned `BitVec`) |
| `i8` `i16` `i32` `i64` | modelled (signed `BitVec`) |
| `u128` `i128` | parsed, **declined** (no 128-bit model) |
| `bool` | modelled |
| `+ - *` `& \| ^ << >>` | modelled (`<<` / `>>` use WASM's modulo-width shift count) |
| `< <= > >= == !=` | modelled (signedness from the declared type) |
| `&& \|\| !` | modelled (booleans and integer truthiness) |
| `/` `%` | **declined** |
| calls, loops, `if`/`while`/`for`/`match`, strings, arrays, structs, indexing, field access | **declined** |
| mixed-width, mixed-signedness, or mixed boolean/integer arithmetic | **declined**, not silently extended |
| a `let`/`const` that shadows a state field | **declined** — otherwise a postcondition reads the local and proves a false statement |
| numeric literals that do not fit the declared width | **declined**, not truncated |

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

Contract-level `invariant` statements are discharged too: each one is checked
against **every** function (reported as `contract_invariant[i]`), assumed in the
pre-state and proved in the post-state.

### Statements and expressions

`let` / `let mut` / `const`, assignment `x = v`, `return`, `if` / `else`,
`while`, `for x in a..b`, `match`, `break`, `continue`, blocks, string
literals (`"…"` with `\n` / `\t` / `\r` / `\\` / `\"`), decimal and `0x` hex
integer literals, and the operators listed above.

Expressions and statements may nest up to 100 levels (about 50 nested
parentheses); deeper input is rejected with a clean parse error rather than a
stack overflow.

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

    // Compile to WASM: parse -> verify layers -> WAT -> WASM
    let wasm = compile(source)?;
    println!("Compiled to {} bytes", wasm.len());

    Ok(())
}
```

### Compiling to WASM

```bash
cargo run -p redsigma-compiler --example compile_to_wasm -- \
  examples/verified_counter.redsigma /tmp/counter.wasm
# wrote 106 bytes to /tmp/counter.wasm
```

The pipeline is: parse → semantic projection → dual-stream → reflection →
causal memory → **WAT lowering** → **WASM assembly**. `compile()` is no longer a
stub; it rejects a file with more than one contract (naming them) and returns
real WASM bytes for a single contract.

Structure checks are not enough to call a module valid — an empty stack at a
`local.set` still passes them. The authoritative gate is `wasmtime.Module`:

```bash
python3 verify/compile_examples.py
# ok    verified_counter.redsigma: 106 bytes, exports=['decrement', 'get', 'increment']
# ok    counterexample.redsigma: 64 bytes, exports=['deposit']
# ok    declined_division.redsigma: 58 bytes, exports=['halve']
# ok    unsigned.redsigma: 57 bytes, exports=['set_max']
# all 4 examples compile to valid WASM
```

`wasmtime.Module` validates the module (unlike `wat2wasm`, which only parses
text), and CI runs this script on every push.

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
[redsigma-codegen]           WAT lowering + WASM assembly + proof objects
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
| `redsigma-codegen` | WAT lowering + WASM assembly + proof objects |
| `redsigma-runtime` | NEAR deployment / execution / proof verification |
| `redsigma-stdlib` | Standard library primitives |
| `redsigma-compiler` | Pipeline orchestration + verification input export |

---

## NEAR testnet — live

The borsh-only `contracts/test-contract` is deployed and answering views on
NEAR testnet. `.redsigma` source is parsed, proof-checked, compiled through the
full verification pipeline, and stored on chain; method execution runs through
the `NearRuntime` interpreter over the contract AST with proof re-verification
and invariant enforcement.

| | |
|---|---|
| Account | `redsigma.s03-gate.testnet` |
| Code hash | `EruzgySJeZD7UoLHwtZE5usKUpRFzsH4WMVzdSxMqeo3` |
| Storage | 639,182 bytes (~6.39 NEAR locked) |
| Balance | ~9.19 NEAR |

**Verify the live account yourself** (these commands work without a wallet):

```bash
# The account exists and holds the deployed contract
near state redsigma.s03-gate.testnet --networkId testnet

# View methods answer (no wallet needed)
near view redsigma.s03-gate.testnet get_contract_count '{}' --networkId testnet   # -> 1
near view redsigma.s03-gate.testnet get_owner '{}' --networkId testnet            # -> redsigma.s03-gate.testnet
near view redsigma.s03-gate.testnet view '{"contract_name":"counter","method_name":"get","args":[]}' --networkId testnet  # -> 3

# Write methods are signed by the owner account (redsigma.s03-gate.testnet)
near call redsigma.s03-gate.testnet deploy_contract \
  '{"contract_name":"counter","source":"..."}' \
  --accountId redsigma.s03-gate.testnet --networkId testnet

near call redsigma.s03-gate.testnet execute \
  '{"contract_name":"counter","method_name":"increment","args":[]}' \
  --accountId redsigma.s03-gate.testnet --networkId testnet
```

### On-chain transactions (verified by the resulting account state)

The following transactions were submitted and confirmed (each produced a "Done deploying"/successful result at submission). They are verifiable by the account state they produced — `near state` shows the contract is live with the deployed code, and `get_contract_count` returns `1`.

| Action | Transaction hash |
|---|---|
| Fund account (7 NEAR from `nft-repo-test-015.testnet`) | `GiUgkQUQaZSbCM8gGDJcifV9deaih4CScd1GgmzUKpmc` |
| Redeploy borsh-only contract | `CwjHUzAse9PeHbhrJDryzEj7jNwdR7acUN4H3JQX2wjv` |
| Deploy `counter` contract | `7LNKtcLKvTqXtVe9dsFAq65Wm62ps8atV8RuxDVtSjYg` |
| `increment` #1 | `CsG7rGcfVh6V5wCGRPGkr4jokDn7TnH1oQYk2JwHNHQp` |
| `increment` #2 | `7NqkaNUafbjWRHrQiWqGAtRKsG7hNcoWALVn5CX6qYAU` |

> Note: the testnet block explorer (NearBlocks) returns 403 for anonymous testnet lookups, so explorer links are omitted. The `near state` / `near view` commands above are the authoritative, explorer-independent verification — and they confirm the contract is live.

## Judge evidence package

See [`JUDGES.md`](JUDGES.md) for the competition evidence package: the live
on-chain transaction links, all verification gates (each can fail), the 4
execution cases that prove the generated WASM actually computes correctly, and
the honest limitations.

---

## Live Demo

Try the frontend demo on GitHub Pages:

👉 **[REDΣSIGMA Frontend Demo](https://sudo-ai-git.github.io/redsigma/)**

Connect a NEAR testnet wallet, deploy `.redsigma` contracts, execute methods,
and read contract state — all from the browser.

---

## Mathematical foundation

REDΣSIGMA integrates the **Tanakh 231-gate** integrity framework (SUM / LINEAR /
PRODUCT mod 231) with dual-stream gematria (Mod-5 semantic + Mod-37/73
structural), a 5-bucket semantic projection, a bijective verifier, causal
parent→child memory chains, and a 6-layer self-reflection engine
(Consistency → Memory → Oracle → Completeness → Uncertainty → Contradiction).

These layers are **integrity/consistency tooling, not program verification**: they
check that a contract's projection is deterministic, that its state transitions
form a causal chain, and that it contains no self-contradiction. They do not
prove that a contract computes the right thing — that is the z3 verifier's job.
The 231-gate checksums must not be presented as a security mechanism.

---

## Project status

**Working and verified by execution:**

- Parser for the subset described above; `@verified` clauses captured as AST.
- z3 tri-state verifier with a falsifiable 18/18 self-test.
- Hostile-input probes: the parser returns a clean error (never SIGABRT) on
  deep nesting, unterminated comments/strings and malformed literals; the
  verifier returns `Unverified` (never a traceback) on malformed literals,
  mixed boolean/integer arithmetic, and deeply nested ASTs.
- Integrity layers: SemanticSpace, BijectiveVerifier, CausalMemory, DualStream,
  6-layer reflection, 231-gate checksums.
- `compile()` produces real, wasmtime-validated WASM from `.redsigma` source
  (all four examples, plus if/while/for/match control flow).
- `cargo test --workspace`: 141 tests passing across 28 suites.
- A NEAR test contract, deployed and exercised on testnet.

**Known divergences (honest limitations):**

- `compile()` produces a real NEAR ABI WASM module, but the deployed path still
  uses the tree-walking runtime interpreter rather than executing the generated
  WASM directly. The runtime and verifier now share the same integer semantics
  (per-width/signedness, with `u128`/`i128` declined in both), verified by the
  full test suite.
- `codegen/src/wat.rs` is a real WAT emitter but is reached only by
  `codegen/examples/emit_wat.rs`; it is not on the compile path (the compile
  path now goes through `near_abi::emit_near_module`).
- **Redeploying the rebuilt test contract is storage-cost blocked**, not broken:
  with the codegen crates now linked in, the module is 838,148 bytes (≈8.38
  NEAR to store) against a 2.22 NEAR balance — a ≈6.16 NEAR shortfall. See
  **NEAR testnet — live** above.

**Not implemented yet:**

- The compiler handles one contract per file; a multi-contract file is
  rejected with the contract names.
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
