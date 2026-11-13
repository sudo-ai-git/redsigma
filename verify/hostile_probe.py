#!/usr/bin/env python3
"""Hostile-input probe for the z3 verifier.

Feeds the verifier JSON values it could receive from the exporter (and from a
hand-edited file) and classifies the outcome: a verdict, a clean decline, or an
UNHANDLED EXCEPTION. The verifier must never crash on input it did not model.
"""

from __future__ import annotations

import sys
import traceback

sys.path.insert(0, "/home/sudosudo/.openclaw/workspace/redsigma/verify")
import redsigma_verify as rv  # noqa: E402


def num(n):
    return {"Number": n}


def iden(n):
    return {"Ident": n}


def binop(op, l, r):
    return {"Binary": {"left": l, "op": op, "right": r}}


def old(n):
    return {"Call": {"func": "old", "args": [iden(n)]}}


def assign(name, value):
    return {"Expr": binop("Assign", iden(name), value)}


def ret(value=None):
    return {"Return": value}


def contract(name, state, fns):
    return {
        "contract": name,
        "state": [{"name": s, "type": t} for s, t in state],
        "functions": fns,
    }


def fn(name, body, pre=None, post=None, invariant=None, params=None, ret_ty="u64"):
    return {
        "name": name,
        "params": [{"name": p, "type": "u64"} for p in (params or [])],
        "pre": pre,
        "post": post,
        "invariant": invariant,
        "body": body,
    }


# name, contract, expectation category
CASES = [
    ("hex literal 0xFF",
     contract("Hex", [("x", "u64")],
              [fn("f", [assign("x", num("0xFF")), ret(None)], post=binop("Gt", iden("x"), num("0")))]),
     "no-crash"),
    ("malformed hex 0x",
     contract("HexBad", [("x", "u64")],
              [fn("f", [assign("x", num("0x")), ret(None)], post=binop("Gt", iden("x"), num("0")))]),
     "no-crash"),
    ("literal beyond u64",
     contract("Big", [("x", "u64")],
              [fn("f", [assign("x", num("999999999999999999999999999999")), ret(None)],
                  post=binop("Gt", iden("x"), num("0")))]),
     "no-crash"),
    ("underscore literal",
     contract("Und", [("x", "u64")],
              [fn("f", [assign("x", num("1_000")), ret(None)], post=binop("Gt", iden("x"), num("0")))]),
     "no-crash"),
    ("negative literal node",
     contract("Neg", [("x", "i64")],
              [fn("f", [assign("x", num("-5")), ret(None)], post=binop("Lt", iden("x"), num("0")))]),
     "no-crash"),
    ("empty number string",
     contract("Empty", [("x", "u64")],
              [fn("f", [assign("x", num("")), ret(None)], post=binop("Gt", iden("x"), num("0")))]),
     "no-crash"),
    ("float number string",
     contract("Float", [("x", "u64")],
              [fn("f", [assign("x", num("1.5")), ret(None)], post=binop("Gt", iden("x"), num("0")))]),
     "no-crash"),
    ("assignment to unknown name",
     contract("Unknown", [("x", "u64")],
              [fn("f", [assign("nope", num("1")), ret(None)], post=binop("Ge", iden("x"), num("0")))]),
     "no-crash"),
    ("let shadows state field",
     contract("Shadow", [("x", "u64")],
              [fn("f", [{"Let": {"mutable": False, "name": "x", "ty": None, "value": num("5")}},
                        assign("x", num("6")), ret(None)],
                  post=binop("Eq", iden("x"), num("6")))]),
     "no-crash"),
    ("old() in pre",
     contract("OldPre", [("x", "u64")],
              [fn("f", [assign("x", num("1")), ret(None)],
                  pre=binop("Eq", old("x"), num("0")),
                  post=binop("Eq", iden("x"), num("1")))]),
     "no-crash"),
    ("old() on a parameter",
     contract("OldParam", [],
              [fn("f", [ret(None)], pre={"Bool": True},
                  post=binop("Eq", old("a"), num("0")), params=["a"])]),
     "no-crash"),
    ("mixed widths u8 vs u64",
     contract("Mixed", [("x", "u8"), ("y", "u64")],
              [fn("f", [assign("x", binop("Add", iden("x"), iden("y"))), ret(None)],
                  post=binop("Ge", iden("x"), num("0")))]),
     "no-crash"),
    ("bool in arithmetic",
     contract("BoolArith", [("b", "bool")],
              [fn("f", [ret(None)], pre={"Bool": True},
                  post=binop("Gt", binop("Add", iden("b"), num("1")), num("0")))]),
     "no-crash"),
    ("not on integer",
     contract("NotInt", [("x", "u64")],
              [fn("f", [ret(None)], pre={"Bool": True},
                  post={"Unary": {"op": "Not", "expr": iden("x")}})]),
     "no-crash"),
    ("empty body",
     contract("EmptyBody", [("x", "u64")],
              [fn("f", [], pre={"Bool": True}, post=binop("Ge", iden("x"), num("0")))]),
     "no-crash"),
    ("unsupported state type u128",
     contract("U128", [("x", "u128")],
              [fn("f", [ret(None)], pre={"Bool": True}, post=binop("Ge", iden("x"), num("0")))]),
     "no-crash"),
    ("huge shift amount",
     contract("Shift", [("x", "u64")],
              [fn("f", [assign("x", binop("Shl", iden("x"), num("1000000"))), ret(None)],
                  post=binop("Ge", iden("x"), num("0")))]),
     "no-crash"),
    ("param shadows state",
     contract("ParamShadow", [("x", "u64")],
              [fn("f", [assign("x", num("7")), ret(None)], pre={"Bool": True},
                  post=binop("Eq", iden("x"), num("7")), params=["x"])]),
     "no-crash"),
    ("no functions",
     contract("NoFns", [("x", "u64")], []),
     "no-crash"),
    ("no state fields",
     contract("NoState", [],
              [fn("f", [ret(num("1"))], pre={"Bool": True}, post={"Bool": True})]),
     "no-crash"),
    # Malformed verification input: the exporter always writes these keys, but
    # a hand-edited document may omit them. That is a decline, not a traceback.
    ("function missing body",
     {"contract": "X", "state": [],
      "functions": [{"name": "f", "params": [], "pre": {"Bool": True},
                     "post": {"Bool": True}}]},
     "no-crash"),
    ("function missing name",
     {"contract": "X", "state": [],
      "functions": [{"params": [], "pre": {"Bool": True}, "post": {"Bool": True},
                     "body": []}]},
     "no-crash"),
    ("state entry missing type",
     {"contract": "X", "state": [{"name": "x"}],
      "functions": [{"name": "f", "params": [], "pre": {"Bool": True},
                     "post": {"Bool": True}, "body": []}]},
     "no-crash"),
    ("state is not a list", {"contract": "X", "state": "nope", "functions": []},
     "no-crash"),
    ("functions is not a list", {"contract": "X", "state": [], "functions": "nope"},
     "no-crash"),
]


def deep_expr(depth):
    e = num("1")
    for _ in range(depth):
        e = binop("Add", e, num("1"))
    return e


CASES.append(
    ("deep expression 100000",
     contract("Deep", [("x", "u64")],
              [fn("f", [ret(None)], pre={"Bool": True}, post=deep_expr(100000))]),
     "no-crash")
)


def main() -> int:
    crashes = 0
    for label, c, _expect in CASES:
        try:
            result = rv.verify_contract(c)
            s = result["summary"]
            print(f"{label:34} => OK  proven={s['proven']} ce={s['counterexample']} "
                  f"unv={s['unverified']} declines={s['declines']}")
        except Exception as e:
            crashes += 1
            print(f"{label:34} => UNHANDLED {type(e).__name__}: {e}")
            traceback.print_exc(limit=3)
    print(f"\nunhandled exceptions: {crashes}")
    return 1 if crashes else 0


if __name__ == "__main__":
    sys.exit(main())
