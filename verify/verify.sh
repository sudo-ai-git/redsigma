#!/usr/bin/env bash
# Verify a REDΣSIGMA contract's @verified clauses with z3.
#
#   verify/verify.sh examples/verified_counter.redsigma
#
# Exits 0 only when EVERY function in the file carries @verified clauses and
# every obligation is Proven. Counterexamples and declines are printed.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="${1:?usage: verify/verify.sh <contract.redsigma>}"

cargo run -q -p redsigma-compiler --example emit_verification -- "$SRC" \
  | python3 "$ROOT/verify/redsigma_verify.py"
