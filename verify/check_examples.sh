#!/usr/bin/env bash
# Assert that each example produces its documented verdict/exit code.
#
#   verify/check_examples.sh
#
# Exit 0 only if every example matches. Run by CI and by hand.
set -u

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fail=0

check() {
    local file="$1" expected="$2"
    local got
    if "$ROOT/verify/verify.sh" "$ROOT/examples/$file" >/dev/null 2>&1; then
        got=0
    else
        got=$?
    fi
    if [ "$got" -ne "$expected" ]; then
        echo "FAIL  $file: expected exit $expected, got $got"
        fail=1
    else
        echo "ok    $file -> exit $got"
    fi
}

# 0 = every obligation Proven; 1 = a Counterexample or a decline.
check verified_counter.redsigma 0
check unsigned.redsigma 0
check counterexample.redsigma 1
check declined_division.redsigma 1

exit $fail
