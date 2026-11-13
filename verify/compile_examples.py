#!/usr/bin/env python3
"""Compile every example contract to WASM and VALIDATE it.

Structure-level checks (magic number, section size) cannot see an invalid
module: an empty stack at a `local.set` still passes them. This script compiles
through `compile()` (parse → verify layers → WAT → WASM) and then hands the
bytes to `wasmtime.Module`, which is the only trustworthy validity gate.

Exit 0 only if every example compiles AND validates.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile

try:
    import wasmtime
except ImportError:  # pragma: no cover
    print("error: the wasmtime python module is required (pip install wasmtime)")
    sys.exit(2)

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# example -> expected exported function names
EXPECTED = {
    "verified_counter.redsigma": ["increment", "decrement", "get"],
    "counterexample.redsigma": ["deposit"],
    "declined_division.redsigma": ["halve"],
    "unsigned.redsigma": ["set_max"],
}


def compile_example(name: str) -> bytes:
    src = os.path.join(ROOT, "examples", name)
    with tempfile.NamedTemporaryFile(suffix=".wasm", delete=False) as fh:
        out = fh.name
    try:
        proc = subprocess.run(
            [
                "cargo",
                "run",
                "-q",
                "-p",
                "redsigma-compiler",
                "--example",
                "compile_to_wasm",
                "--",
                src,
                out,
            ],
            cwd=ROOT,
            capture_output=True,
            text=True,
        )
        if proc.returncode != 0:
            raise RuntimeError(
                f"compile_to_wasm failed ({proc.returncode}):\n{proc.stderr[-800:]}"
            )
        with open(out, "rb") as fh:
            return fh.read()
    finally:
        if os.path.exists(out):
            os.unlink(out)


def main() -> int:
    failures = 0
    for name, expected in EXPECTED.items():
        try:
            wasm = compile_example(name)
        except Exception as exc:
            print(f"FAIL  {name}: {exc}")
            failures += 1
            continue
        try:
            module = wasmtime.Module(wasmtime.Engine(), wasm)
        except Exception as exc:
            print(f"FAIL  {name}: wasmtime rejected the module: {str(exc).splitlines()[-1]}")
            failures += 1
            continue
        exports = sorted(e.name for e in module.exports)
        missing = [e for e in expected if e not in exports]
        if missing:
            print(f"FAIL  {name}: missing exports {missing} (got {exports})")
            failures += 1
            continue
        print(f"ok    {name}: {len(wasm)} bytes, exports={exports}")

    if failures:
        print(f"\n{failures} example(s) failed")
        return 1
    print(f"\nall {len(EXPECTED)} examples compile to valid WASM")
    return 0


if __name__ == "__main__":
    sys.exit(main())
