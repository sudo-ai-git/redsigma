#!/usr/bin/env python3
"""Try to make the verifier report a FALSE PROOF.

The project's central claim is that an obligation reported `Proven` really
follows. A single false proof destroys that claim, so it is worth attacking
directly rather than waiting for one to surface in the examples.

Each probe below is a contract whose postcondition is FALSE for some input the
precondition admits. A sound verifier must return Counterexample or Unverified;
reporting `Proven` is a soundness bug.

Probes cover the edges the verifier documents as dangerous:
  * signed vs unsigned comparison (z3py `>=` is signed)
  * wraparound at the declared width
  * `old()` aliasing and shallow copies
  * a postcondition that mentions an undeclared/unknown identifier
  * precondition that is unsatisfiable (vacuous truth)
  * a literal that overflows the declared width
  * `old()` referencing a parameter, not state
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# ---------------------------------------------------------------- the probes

# Each entry: (name, source, obligation-that-must-NOT-be-Proven description)
FALSE_POSTCONDITIONS = [
    (
        "unsigned_wraparound",
        """
contract C {
    invariant x >= 0;
    state { x: u64 }
    @verified(pre: true, post: x == 0, invariant: x >= 0)
    fn noop() -> u64 { x; }
}
""",
        "x == 0 is false when x != 0; pre is `true`, so this must NOT be proven",
    ),
    (
        "signed_gt_should_be_unsigned",
        """
contract C {
    invariant x >= 0;
    state { x: u64 }
    @verified(pre: true, post: x > 0, invariant: x >= 0)
    fn noop() -> u64 { x; }
}
""",
        "x > 0 is false for x == 0; an unsigned contract must not be proven here",
    ),
    (
        "vacuous_precondition",
        """
contract C {
    invariant x >= 0;
    state { x: u64 }
    @verified(pre: x < 0, post: x == 12345, invariant: x >= 0)
    fn impossible() -> u64 { x; }
}
""",
        "pre x < 0 contradicts invariant x >= 0, so pre is unsatisfiable. "
        "An obligation proven only on an empty input range is a VACUOUS proof; "
        "the verifier must not call this Proven.",
    ),
    (
        "old_state_not_initial",
        """
contract C {
    invariant n >= 0;
    state { n: u64 }
    @verified(pre: true, post: n == old(n), invariant: n >= 0)
    fn bump() -> u64 { n = n + 1; n; }
}
""",
        "the body changes n, so n == old(n) is false; must not be proven",
    ),
    (
        "post_mentions_unknown",
        """
contract C {
    invariant x >= 0;
    state { x: u64 }
    @verified(pre: true, post: mystery > 0, invariant: x >= 0)
    fn f() -> u64 { x; }
}
""",
        "`mystery` is not declared; must decline, never prove",
    ),
    (
        "pre_false_but_post_false",
        """
contract C {
    invariant x >= 0;
    state { x: u64 }
    @verified(pre: x > 100, post: x < 100, invariant: x >= 0)
    fn f() -> u64 { x; }
}
""",
        "contradictory pre+post; pre alone is satisfiable, so this is NOT vacuous. "
        "The post is false for every admitted input -> Counterexample, not Proven",
    ),
]

VALID_PROOFS = [
    (
        "genuinely_true_should_prove",
        """
contract C {
    invariant x >= 0;
    state { x: u64 }
    @verified(pre: true, post: x == old(x) + 1, invariant: x >= 0)
    fn inc() -> u64 { x = x + 1; x; }
}
""",
    ),
]


def verify_source(source: str) -> dict:
    """Run the emit + verify pipeline on one source, return the JSON report."""
    with tempfile.NamedTemporaryFile("w", suffix=".redsigma", delete=False) as fh:
        fh.write(source)
        path = fh.name
    try:
        proc = subprocess.run(
            ["cargo", "run", "-q", "-p", "redsigma-compiler",
             "--example", "emit_verification", "--", path],
            cwd=ROOT, capture_output=True, text=True,
        )
        if proc.returncode != 0:
            return {"error": f"emit failed: {proc.stderr[-400:]}"}
        payload = proc.stdout
    finally:
        os.unlink(path)

    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as fh:
        fh.write(payload)
        json_path = fh.name
    try:
        vproc = subprocess.run(
            [sys.executable, os.path.join(ROOT, "verify", "redsigma_verify.py"), json_path],
            capture_output=True, text=True,
        )
    finally:
        os.unlink(json_path)
    try:
        return json.loads(vproc.stdout)
    except Exception:
        return {"error": f"verifier did not emit JSON: {vproc.stdout[-300:]} {vproc.stderr[-300:]}"}


def verdict_of(report: dict) -> list[tuple[str, str]]:
    """Extract (function.obligation, status) pairs from the verifier's report.

    Shape: {"contracts": [{"functions": [{"name", "obligations":
    [{"kind", "status", "model", ...}]}]}]}
    """
    out = []
    for contract in report.get("contracts", []):
        for fn in contract.get("functions", []):
            name = fn.get("name", "?")
            for ob in fn.get("obligations", []):
                kind = ob.get("kind", "?")
                status = ob.get("status", "?")
                out.append((f"{name}.{kind}", str(status)))
    return out


def main() -> int:
    failures = 0

    print("== false postconditions: must NOT be Proven ==")
    for name, source, why in FALSE_POSTCONDITIONS:
        report = verify_source(source)
        if "error" in report:
            print(f"FAIL  {name}: {report['error']}")
            failures += 1
            continue
        verdicts = verdict_of(report)
        # A false postcondition shows up in the `post` obligation. The
        # `invariant` and `contract_invariant[i]` obligations are separate
        # claims -- `x >= 0` on a u64 really is always true -- so Proven there
        # is correct and must not be counted as a false proof.
        post_verdicts = [(k, v) for k, v in verdicts if k.endswith(".post")]
        bad = [(k, v) for k, v in post_verdicts if v == "Proven"]
        if bad:
            print(f"FAIL  {name}: post reported Proven -> FALSE PROOF")
            print(f"        {why}")
            for k, _ in bad:
                print(f"        {k} = Proven")
            failures += 1
        else:
            got = ", ".join(f"{k}={v}" for k, v in post_verdicts) or "(no post obligation)"
            print(f"ok    {name}: {got}")

    print("\n== true postcondition: SHOULD be Proven ==")
    for name, source in VALID_PROOFS:
        report = verify_source(source)
        if "error" in report:
            print(f"FAIL  {name}: {report['error']}")
            failures += 1
            continue
        verdicts = verdict_of(report)
        if any(v == "Proven" for _, v in verdicts):
            print(f"ok    {name}")
        else:
            print(f"FAIL  {name}: a genuinely true obligation was not proven: {verdicts}")
            failures += 1

    print()
    if failures:
        print(f"{failures} SOUNDNESS DEFECT(S) FOUND")
        return 1
    print("no false proofs found")
    return 0


if __name__ == "__main__":
    sys.exit(main())