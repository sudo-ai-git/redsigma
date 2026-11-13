# REDΣSIGMA — Hackathon-Readiness Panel Review

An adversarial panel of five independent expert subagents (delegated with
`delegate_task`, distinct lenses, no shared context) reviewed the repository and
voted on whether it is good enough for the NEAR hackathon. Every expert was told
to verify claims by execution and that a clean bill of health is the suspicious
outcome.

## Vote

| Expert | Lens | Ready | Score |
|---|---|---|---|
| panel_verification | formal verification & soundness | conditional | 5/10 |
| panel_security | security & adversarial robustness | conditional | 5/10 |
| panel_near | NEAR hackathon engineering & demo | conditional | 4/10 |
| panel_product | product, differentiation, viability | conditional | 3/10 |
| panel_code | code quality, honesty, readiness | conditional | 4/10 |

**Unanimous: `conditional`. Mean score 4.2/10.**
No expert said yes; no expert said no. The agreement is that the z3 verifier is a
real asset and the end-to-end "self-verifying language on NEAR" claim is not yet
earned.

## Findings that were fixed during the review

The panel ran against a moving tree; the following were reproduced by hand and
fixed with regression tests (self-test is now **18/18**, mutation-checked):

1. **False proof via `let` shadowing a state field** — `post: count == 0` read the
   local (which dies at return) instead of the field, so the verifier proved a
   statement about nothing. Now declined.
2. **False proof via a parameter shadowing a state field** — same class. Now
   declined, and duplicate parameters are refused.
3. **z3 symbol-name collision** — state fields were bound to symbols named
   `<field>__pre`; a parameter named `x__pre` aliased the same z3 variable.
   Symbols are now `z3.FreshConst`, so they cannot collide with user names.
4. **Contract-level invariants were exported but never checked.** They are now
   obligations on every function (`contract_invariant[i]`), assumed pre-state and
   proved post-state.
5. **A function with no `@verified` could report `passed=True`** once contract
   invariants existed. Every function must carry its own clauses again.
6. **Mixed-width assignment** (`x: u8 = y: u64`) stored a 64-bit term under an
   8-bit binding and crashed z3 with a sort mismatch. Bindings are checked
   against their declared width/signedness.
7. **Operand-order-dependent verdicts** — `0 <= x` (literal on the left) was
   declined as mixed-width while `x >= 0` was proven. A bare literal now adopts
   the typed operand's width.
8. **Deep generic nesting** (`Vec<Vec<...>>`) aborted the parser with SIGABRT;
   `parse_type` now has the same depth guard as expressions and statements.
9. **Deep index/binary chains** (`a[0][0]...`, `1+1+1+...`) built deep ASTs
   iteratively and overflowed every recursive walk. An iterative AST depth check
   rejects them.
10. **Malformed verification input** (missing `body`/`name`/`type`, non-list
    `state`/`functions`) produced tracebacks. Now reported as declines.
11. **`verify.sh` piped an empty exporter stdout into Python**, so a parse error
    printed a `JSONDecodeError` traceback. It now fails cleanly with exit 2.
12. **A recorded decline no longer coexists with a green exit code** — `passed`
    requires no declines.
13. **Docs overclaimed.** The fabricated `NEAR_SDK_CONTRACTSTATE_PANEL.md` was
    deleted; the NEAR section no longer claims the deployed contract exercises
    the language; the unreproducible "ten application spike tests" claim was
    replaced with an honest description of the integrity layers; `codegen/src/wat.rs`
    is labelled example-only; CI now runs the tests, self-test, hostile probes and
    the documented example verdicts.

## Remaining gaps (not fixed — stated plainly)

1. **There is no compiler.** `redsigma_compiler::compile()` is a stub and returns
   an empty vector. No `.redsigma` file has ever become WASM, and nothing on the
   deployed path exercises the language.
2. **The testnet contract is a handwritten placeholder.** It stores a counter and
   returns canned strings; `deploy_contract` ignores its `source` argument and
   `execute` ignores its `args`. It demonstrates state persistence and the NEAR
   deployment flow, nothing more.
3. **The runtime and the verifier are independent implementations with different
   integer semantics.** The verifier uses per-width/signedness `BitVec` and
   declines out-of-range literals; the runtime encodes everything as `u128` and
   truncates on store. A contract the verifier declines can still run in the
   runtime. This is documented in README "Known divergences".
4. **The runtime does not enforce `@verified` clauses.** It evaluates
   contract-level invariants only; a function whose postcondition is false will
   execute.
5. **Contract invariants are checked as preservation only** (assume pre-state,
   prove post-state). Initial-state satisfaction and vacuous pre-states are not
   checked.
6. **The 231-gate / dual-stream / causal / reflection layers are integrity
   tooling, not program verification.** They are deterministic-projection and
   consistency checks; they do not prove correctness and must not be presented as
   a security mechanism.
7. **No frontend, no deploy gate, no real demo.** A judge who asks "show me a
   contract compiled from your language" cannot be answered yet.

## What the panel agreed is genuinely good

- The tri-state z3 verifier: per-width/signedness `BitVec`, honest declines
  (division, out-of-range literals, mixed boolean/integer), a falsifiable
  self-test, and a concrete wrapping counterexample (`amount = 2^64-1`).
- Hostile-input hardening on both the parser and the verifier.
- 121 Rust tests passing, 0 failing; the documented examples' verdicts are
  asserted by `verify/check_examples.sh` and run in CI.
- A reproducible testnet deployment (code hash equals `base58(sha256(wasm))`,
  verified live).

## Bottom line

**Conditional, unanimously.** Reframe the hackathon pitch as *"a symbolic
verifier for a tiny contract DSL that catches integer-safety bugs before you
write the Rust"* — which is exactly what the tool does — and the panel's
condition is met. The "self-verifying smart contract language" claim needs
`compile()` and a generated contract before it can be made honestly.
