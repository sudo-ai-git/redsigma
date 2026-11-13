# REDΣSIGMA — NEAR Competition Evidence Package

**Repo:** https://github.com/sudo-ai-git/redsigma
**Commit:** `dab3392` (HEAD of `main`)
**Live contract:** `redsigma.s03-gate.testnet` (code hash `FABsVpesCdxi4z7C4WJ5gWaY2od3M1ThLJ3Fptic3z4q`)

This package is what a competition judge needs to verify the project's claims. Every number here is backed by an on-chain transaction or an executable gate that can fail.

---

## 1. The claim (one sentence)

> REDΣSIGMA is a smart contract language where **every function carries a machine-checked proof** — `@verified(pre, post, invariant)` clauses are discharged by a tri-state z3 verifier (Proven / Counterexample / Unverified, never a guess), proof objects are embedded in the generated WASM, and deployment is gated on proof passing.

---

## 2. What is live on NEAR testnet (verifiable on-chain)

| | |
|---|---|
| **Account** | `redsigma.s03-gate.testnet` |
| **Code hash** | `FABsVpesCdxi4z7C4WJ5gWaY2od3M1ThLJ3Fptic3z4q` |
| **Storage** | 634,424 bytes (~6.34 NEAR locked) |
| **Balance** | ~9.21 NEAR |
| **Account page** | https://testnet.nearblocks.io/account/redsigma.s03-gate.testnet |

### Successful on-chain transactions (evidence)

| Action | Transaction | Explorer |
|---|---|---|
| Fund account (7 NEAR) | `GiUgkQUQaZSbCM8gGDJcifV9deaih4CScd1GgmzUKpmc` | https://testnet.nearblocks.io/txns/GiUgkQUQaZSbCM8gGDJcifV9deaih4CScd1GgmzUKpmc |
| Redeploy borsh-only contract | `CwjHUzAse9PeHbhrJDryzEj7jNwdR7acUN4H3JQX2wjv` | https://testnet.nearblocks.io/txns/CwjHUzAse9PeHbhrJDryzEj7jNwdR7acUN4H3JQX2wjv |
| Deploy `counter` contract | `7LNKtcLKvTqXtVe9dsFAq65Wm62ps8atV8RuxDVtSjYg` | https://testnet.nearblocks.io/txns/7LNKtcLKvTqXtVe9dsFAq65Wm62ps8atV8RuxDVtSjYg |
| `increment` #1 | `CsG7rGcfVh6V5wCGRPGkr4jokDn7TnH1oQYk2JwHNHQp` | https://testnet.nearblocks.io/txns/CsG7rGcfVh6V5wCGRPGkr4jokDn7TnH1oQYk2JwHNHQp |
| `increment` #2 | `7NqkaNUafbjWRHrQiWqGAtRKsG7hNcoWALVn5CX6qYAU` | https://testnet.nearblocks.io/txns/7NqkaNUafbjWRHrQiWqGAtRKsG7hNcoWALVn5CX6qYAU |

### Live calls you can make right now

```bash
# read
near view redsigma.s03-gate.testnet get_contract_count '{}'   # -> 1
near view redsigma.s03-gate.testnet get_owner '{}'             # -> redsigma.s03-gate.testnet
near view redsigma.s03-gate.testnet view '{"contract_name":"counter","method_name":"get","args":[]}'  # -> 3

# write (signed by the owner account)
near call redsigma.s03-gate.testnet deploy_contract \
  '{"contract_name":"counter","source":"..."}' \
  --accountId redsigma.s03-gate.testnet

near call redsigma.s03-gate.testnet execute \
  '{"contract_name":"counter","method_name":"increment","args":[]}' \
  --accountId redsigma.s03-gate.testnet
```

---

## 3. Verification gates (each can FAIL — none are narration)

| Gate | Command | Result |
|---|---|---|
| Rust test suite | `cargo test --workspace` | all tests pass, 0 failures |
| z3 verifier selftest | `python3 verify/redsigma_verify.py --selftest` | 20/20 pass (mutation-checked) |
| WASM compiles & validates | `python3 verify/compile_examples.py` | all 4 examples → valid wasmtime-accepted WASM |
| Example verdicts | `bash verify/check_examples.sh` | 2 proven (exit 0), 2 correctly rejected (exit 1) |
| **WASM actually executes** | `python3 verify/execute_examples.py` | **4/4 execution cases pass** |

The last row is the one that matters for judges: `execute_examples.py` instantiates each generated WASM module with mocked NEAR host functions (`input`, `storage_read`, `storage_write`, `storage_has_key`, `value_set`) and runs the exported methods, checking real outputs and state transitions. This proves the compiled WASM **computes correctly when executed** — not just that it parses.

### The 4 execution cases

| Case | What it proves |
|---|---|
| `verified_counter` | `increment()` → 1, → 2; state persists across calls; `decrement()` works; `get()` returns 2 |
| `unsigned` | `2^64 - 1` survives as a large **positive** u64 (not -1) — the verifier's signedness claim holds in execution |
| `declined_division` | `halve()` truncates toward zero (101 → 50) — well-defined even though the verifier declines `/` |
| `counterexample` | `deposit(49)` → 50 (= 0 + 49 + 1) — the verifier's counterexample (off-by-one) is real and observable in the executed artifact |

---

## 4. The honest limitations (judges, read these first)

The project is **CONDITIONAL**, not unconditionally ready. The panel of 3 independent judges voted unanimous `CONDITIONAL` (6.7/10 mean):

1. **Executable subset is small.** The verifier/WASM backend models `u8`–`u64`, `i8`–`i64`, `bool`, `+ - * & \| ^ << >>`, comparisons, `&& \|\| !`, `if/else`, `while`, `for .. in a..b`, `match`. It **declines** `/`, `%`, loops beyond bounded `for`, calls, strings, arrays, structs, indexing, field access. Most real DeFi contracts cannot be written yet.

2. **Runtime executes the AST, not the WASM.** The on-chain `execute` runs a tree-walking interpreter over the contract AST (with per-width/signedness integers matching the verifier's BitVec model). The generated proof-carrying WASM is stored and validated, but on-chain execution does not yet instantiate it. `execute_examples.py` proves the WASM executes correctly off-chain; on-chain WASM execution is the next step.

3. **Integrity layers ≠ security.** The 231-gate / dual-stream / reflection layers are consistency tooling, explicitly **not** a security mechanism (README states this plainly).

4. **Module size is large.** 636,732 bytes (~6.37 NEAR) — redeploy fits in the current balance with ~2.87 NEAR headroom, but little room for growth without further shrinking.

---

## 5. How a judge verifies in 5 minutes

```bash
# 1. Clone and build
git clone https://github.com/sudo-ai-git/redsigma.git
cd redsigma
cargo test --workspace                    # all tests pass

# 2. Run the verifier selftest
python3 verify/redsigma_verify.py --selftest   # 20/20

# 3. Run the execution harness (WASM actually runs)
python3 verify/execute_examples.py         # 4/4 pass

# 4. Check the live contract
near view redsigma.s03-gate.testnet get_contract_count '{}'   # -> 1
near view redsigma.s03-gate.testnet view '{"contract_name":"counter","method_name":"get","args":[]}'  # -> 3
```

---

## 6. Files changed in the competition commit

- `contracts/test-contract/src/lib.rs` — borsh-only on-chain storage; resilient `state_read` (salvages legacy owner_id); `view()` no longer calls host functions prohibited in views (`predecessor_account_id`, `prepaid_gas`, `attached_deposit`)
- `contracts/test-contract/Cargo.toml` — removed unused `serde`/`serde_json`
- `codegen/`, `parser/`, `semantic/`, `dualstream/`, `reflection/`, `runtime/` — `BorshSerialize/BorshDeserialize` added to `Contract`, `ProofObject`, `WasmCompilation`, and all nested report types
- `README.md` — NEAR testnet section rewritten with live transaction evidence

**Commit:** `dab3392` on `main` — https://github.com/sudo-ai-git/redsigma/commit/dab3392

---

*Evidence current as of the live testnet deployment. All on-chain transaction links point to NearBlocks testnet explorer.*
