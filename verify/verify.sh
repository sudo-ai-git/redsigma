#!/usr/bin/env bash
# Verify a REDΣSIGMA contract's @verified clauses with z3.
#
#   verify/verify.sh examples/verified_counter.redsigma
#
# Exits 0 only when EVERY function carries `@verified` and every obligation is
# Proven. Counterexamples and declines are printed; either way the exit code is
# non-zero.
#
# A parse/export failure is reported cleanly (exit 2) — the exporter's stdout is
# never piped into the verifier, which would otherwise die with a JSONDecodeError
# traceback on an empty stream.
set -u

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="${1:?usage: verify/verify.sh <contract.redsigma>}"

out="$(mktemp)"
err="$(mktemp)"
trap 'rm -f "$out" "$err"' EXIT

if ! cargo run -q -p redsigma-compiler --example emit_verification -- "$SRC" > "$out" 2> "$err"; then
    echo "error: could not export verification input for $SRC" >&2
    tail -20 "$err" >&2
    exit 2
fi

python3 "$ROOT/verify/redsigma_verify.py" "$out"
exit $?
