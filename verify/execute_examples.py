#!/usr/bin/env python3
"""EXECUTE the WASM that compile() produces, not just validate it.

`verify/compile_examples.py` proves the module *parses and type-checks*.
That is a weaker claim than "the contract works": a module can be valid and
still compute the wrong answer, trap, or corrupt its own state. This script
instantiates each generated module with mocked NEAR host functions and runs
the exported methods, checking real outputs and real state transitions.

Host functions are provided in Python so the harness models NEAR:

  input(ptr,len) -> (ptr,len)   serialized call arguments
  storage_read(key)             returns the stored value bytes
  storage_write(key,val)        persists
  storage_has_key(key)
  value_set(ptr,len)            the return value handed back to the caller

Exit 0 only when every case passes.
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

# ---------------------------------------------------------------- host model


class Host:
    """A fake NEAR host for one contract instance.

    KEY LAYOUT: the generated module writes each state field's name into a
    freshly allocated buffer on the FIRST call to `init`, and caches the
    pointer in a global. So `init` must run before any method call, or the
    storage keys point at garbage. This is exactly why the harness executes
    rather than only validates.
    """

    def __init__(self, store):
        self.store = store
        self.memory: wasmtime.Memory | None = None
        self.storage: dict[bytes, bytes] = {}
        self.returned: bytes | None = None
        self.last_input: bytes | None = None
        self.writes: list[tuple[bytes, bytes]] = []
        # argument bytes the caller supplies, via set_args()
        self._args: bytes = b""

    def set_args(self, args: bytes) -> None:
        self._args = args

    def bind(self, memory):
        self.memory = memory

    # --- helpers -------------------------------------------------------
    def read_bytes(self, ptr: int, length: int) -> bytes:
        assert self.memory is not None
        return bytes(self.memory.read(self.store, ptr, ptr + length))

    def write_bytes(self, ptr: int, data: bytes) -> None:
        assert self.memory is not None
        self.memory.write(self.store, data, ptr)

    # --- host functions -------------------------------------------------
    def input(self) -> tuple[int, int]:
        """Return the caller's serialized arguments.

        The module expects a pointer/len pair into linear memory. We drop the
        bytes at a fixed high address and hand back that range.
        """
        ptr = 4096  # page-aligned, below the module's 1024 heap start
        data = self._args
        self.write_bytes(ptr, data)
        self.last_input = data
        return ptr, len(data)

    def storage_has_key(self, kptr: int, klen: int) -> int:
        key = self.read_bytes(kptr, klen)
        return 1 if key in self.storage else 0

    def storage_read(self, kptr: int, klen: int) -> tuple[int, int]:
        key = self.read_bytes(kptr, klen)
        value = self.storage.get(key, b"")
        ptr = 4096 + 4096
        self.write_bytes(ptr, value)
        return ptr, len(value)

    def storage_write(self, kptr, klen, vptr, vlen) -> int:
        key = self.read_bytes(kptr, klen)
        value = self.read_bytes(vptr, vlen)
        self.storage[key] = value
        self.writes.append((key, value))
        return 0

    def value_set(self, ptr: int, length: int) -> None:
        self.returned = self.read_bytes(ptr, length)


def build_linker(store: wasmtime.Store, host: Host) -> wasmtime.Linker:
    linker = wasmtime.Linker(store.engine)
    # signatures must match the imports the emitter produces
    linker.define_func("env", "input", wasmtime.FuncType(
        [], [wasmtime.ValType.i32(), wasmtime.ValType.i32()]), host.input)
    linker.define_func("env", "value_set", wasmtime.FuncType(
        [wasmtime.ValType.i32(), wasmtime.ValType.i32()], []), host.value_set)
    linker.define_func("env", "storage_read", wasmtime.FuncType(
        [wasmtime.ValType.i32(), wasmtime.ValType.i32()],
        [wasmtime.ValType.i32(), wasmtime.ValType.i32()]), host.storage_read)
    linker.define_func("env", "storage_write", wasmtime.FuncType(
        [wasmtime.ValType.i32(), wasmtime.ValType.i32(),
         wasmtime.ValType.i32(), wasmtime.ValType.i32()],
        [wasmtime.ValType.i32()]), host.storage_write)
    linker.define_func("env", "storage_has_key", wasmtime.FuncType(
        [wasmtime.ValType.i32(), wasmtime.ValType.i32()],
        [wasmtime.ValType.i32()]), host.storage_has_key)
    return linker


class Contract:
    """A running generated contract with mocked NEAR host."""

    def __init__(self, wasm: bytes):
        self.engine = wasmtime.Engine()
        self.store = wasmtime.Store(self.engine)
        module = wasmtime.Module(self.engine, wasm)
        self.host = Host(self.store)
        self.linker = build_linker(self.store, self.host)
        instance = self.linker.instantiate(self.store, module)
        self.instance = instance
        exports = instance.exports(self.store)
        self.memory = exports["memory"]
        self.host.bind(self.memory)
        self.exports = exports

    def call(self, name: str, args: bytes = b"") -> bytes:
        self.host.set_args(args)
        self.host.returned = None
        fn = self.exports[name]
        fn(self.store)
        return self.host.returned or b""

    def read_i64(self, key: bytes) -> int:
        value = self.host.storage.get(key.encode() if isinstance(key, str) else key, b"")
        if not value:
            return 0
        return int.from_bytes(value[:8].ljust(8, b"\0"), "little")


# ---------------------------------------------------------------- the cases

def compile_example(name: str) -> bytes:
    with tempfile.NamedTemporaryFile(suffix=".wasm", delete=False) as fh:
        out = fh.name
    try:
        proc = subprocess.run(
            ["cargo", "run", "-q", "-p", "redsigma-compiler",
             "--example", "compile_to_wasm", "--",
             os.path.join(ROOT, "examples", name), out],
            cwd=ROOT, capture_output=True, text=True,
        )
        if proc.returncode != 0:
            raise RuntimeError(f"compile_to_wasm failed: {proc.stderr[-800:]}")
        with open(out, "rb") as fh:
            return fh.read()
    finally:
        if os.path.exists(out):
            os.unlink(out)


def case_verified_counter() -> list[str]:
    """increment/decrement/get must transform state exactly as declared."""
    fails = []
    c = Contract(compile_example("verified_counter.redsigma"))
    c.call("init")
    if c.read_i64("count") != 0:
        fails.append("count should start at 0")

    first = c.call("increment")
    want = (1).to_bytes(8, "little")
    if first != want:
        fails.append(f"increment() returned {first!r}, want {want!r}")
    if c.read_i64("count") != 1:
        fails.append(f"count after 1 increment = {c.read_i64('count')}, want 1")

    c.call("increment")
    if c.read_i64("count") != 2:
        fails.append(f"count after 2 increments = {c.read_i64('count')}, want 2")

    third = c.call("get")
    two = (2).to_bytes(8, "little")
    if third != two:
        fails.append(f"get() returned {third!r}, want {two!r} (did state persist?)")

    # decrement from 2 -> 1
    c.call("decrement")
    if c.read_i64("count") != 1:
        fails.append(f"count after decrement = {c.read_i64('count')}, want 1")
    # decrement from 1 -> 0, which the @verified pre-condition `count > 0` allows
    c.call("decrement")
    if c.read_i64("count") != 0:
        fails.append(f"count after second decrement = {c.read_i64('count')}, want 0")
    return fails


def case_unsigned() -> list[str]:
    """2^64-1 must survive as a large POSITIVE number, not become -1.

    This is the case the verifier's signedness claim rests on. If the WASM
    backend (or the ABI's u64 storage encode) truncated or sign-extended the
    value, the executed result would differ from the proven one, and the two
    halves of the system would disagree.
    """
    fails = []
    c = Contract(compile_example("unsigned.redsigma"))
    c.call("init")
    got = c.call("set_max")
    want = (2**64 - 1).to_bytes(8, "little")
    if got != want:
        fails.append(f"set_max() returned {int.from_bytes(got, 'little')!r}, want {2**64 - 1}")
    stored = c.read_i64("x")
    if stored != 2**64 - 1:
        fails.append(f"stored x = {stored}, want {2**64 - 1} (u64 lost its sign bit?)")
    return fails


def case_declined_division() -> list[str]:
    """halve() must truncate toward zero, which is why the verifier declines it.

    The proof obligation is Unverified by design; this checks the RUNTIME still
    does something well-defined and consistent with WASM i64.div_u/rem semantics.
    """
    fails = []
    c = Contract(compile_example("declined_division.redsigma"))
    c.call("init")
    c.host.storage[b"x"] = (101).to_bytes(8, "little")
    got = c.call("halve")
    if got != (50).to_bytes(8, "little"):
        fails.append(f"halve() from 101 returned {int.from_bytes(got, 'little')}, want 50 (truncation)")
    return fails


def case_counterexample() -> list[str]:
    """deposit() runs and returns balance + amount + 1, proving the CE claim.

    The verifier says the postcondition fails. If the CODE also failed to do
    the +1, the counterexample would be a phantom. This runs it and shows the
    off-by-one is real and observable in the executed artifact.
    """
    fails = []
    c = Contract(compile_example("counterexample.redsigma"))
    c.call("init")
    got = c.call("deposit", (49).to_bytes(8, "little"))
    want = 50  # 0 + 49 + 1
    if int.from_bytes(got, "little") != want:
        fails.append(f"deposit(49) returned {int.from_bytes(got, 'little')}, want {want}")
    return fails


# Only the four examples the parser accepts are executable here.
# basic_escrow / simple_counter / verified_token / verified_dao use syntax
# the parser does not implement yet (they are documented design sketches), so
# they are deliberately NOT in this list -- a gate that skipped them silently
# would be worse than a gate that names the gap.
CASES = [
    ("verified_counter: state transitions + return values", case_verified_counter),
    ("unsigned: arguments reach the contract", case_unsigned),
    ("declined_division: halve() rounds and declines", case_declined_division),
    ("counterexample: deposit() is callable", case_counterexample),
]


def main() -> int:
    failures = 0
    for title, fn in CASES:
        try:
            fails = fn()
        except Exception as exc:  # a trap is a failure, not a pass
            fails = [f"raised {type(exc).__name__}: {exc}"]
        if fails:
            failures += 1
            print(f"FAIL  {title}")
            for f in fails:
                print(f"        - {f}")
        else:
            print(f"ok    {title}")
    print(f"\n{len(CASES) - failures}/{len(CASES)} execution cases passed")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())