#!/usr/bin/env python3
"""z3 tri-state verifier for REDΣSIGMA ``@verified`` annotations.

Reads the verification input emitted by ``cargo run -p redsigma-compiler
--example emit_verification`` and discharges every function's obligations.

TRI-STATE, ALWAYS
    Proven         ``pre ∧ ¬post`` is unsat  -- the postcondition follows.
    Counterexample ``pre ∧ ¬post`` is sat    -- a z3 model is reported.
    Unverified     the layer DECLINED to model something (division, a call,
                   a loop, mixed-width arithmetic, ...) or z3 returned
                   ``unknown``. It proved nothing, so ``passed`` is False and it
                   is never reported as True.

SOUNDNESS RULES (the panel's first concern)
    * Integers lower to z3 ``BitVec`` at the declared width AND signedness, so
      wrapping and comparisons match WASM two's-complement. Unbounded ``Int``
      would call ``2**63`` "safe" where the machine traps or wraps, and z3py's
      ``>=`` on a BitVecRef is SIGNED, so an unsigned contract needs the
      explicit ``UGE``/``UGT``/``ULE``/``ULT`` operators — unsignedness is
      carried per binding, never guessed from the term.
    * ``/`` and ``%`` are DECLINED, not modelled. WASM integer division truncates
      toward zero and has signed-overflow edge cases that z3's BitVec division
      does not reproduce; modelling one as the other yields false proofs.
    * Calls, loops, ``if`` statements, strings, arrays, structs, indexing and
      field access are DECLINED.
    * Arithmetic between different widths is DECLINED rather than silently
      extended. Numeric literals adopt the width of the other operand.
    * One unsupported construct does not abandon the function: it is recorded
      and the remaining obligations are still checked.

EXIT CODES: 0 if every function is Proven — a function with no ``@verified``
clauses is reported as not passed, never as proven — 1 otherwise.
"""

from __future__ import annotations

import json
import sys
from dataclasses import dataclass, field
from typing import Any, Dict, List, Optional, Tuple

import z3

# Deep ASTs recurse once per level in eval_expr; give the interpreter headroom
# over the parser's MAX_DEPTH (100) and convert overruns into a Decline rather
# than a RecursionError traceback.
sys.setrecursionlimit(max(sys.getrecursionlimit(), 10_000))


_INTEGER_WIDTHS = {
    "u8": 8,
    "u16": 16,
    "u32": 32,
    "u64": 64,
    "i8": 8,
    "i16": 16,
    "i32": 32,
    "i64": 64,
}


class Decline(Exception):
    """The layer cannot soundly model this construct."""

    def __init__(self, reason: str) -> None:
        super().__init__(reason)
        self.reason = reason


def _bare(ty_name: str) -> str:
    return ty_name[len("Verified"):] if ty_name.startswith("Verified") else ty_name


def declared_width(ty_name: str) -> Optional[int]:
    """Width of a declared integer type; None for bool / unsupported."""
    bare = _bare(ty_name)
    if bare == "bool":
        return None
    return _INTEGER_WIDTHS.get(bare)


def type_kind(ty_name: str) -> str:
    bare = _bare(ty_name)
    if bare == "bool":
        return "bool"
    if bare in _INTEGER_WIDTHS:
        return "int"
    return "other"


_SIGNED_TYPES = {"i8", "i16", "i32", "i64"}


def type_signed(ty_name: str) -> bool:
    """Whether a declared integer type is signed.

    Signedness cannot be recovered from a z3 term (the same 64-bit vector is
    either), so it is carried alongside every binding. z3py's `>=` on a
    BitVecRef is SIGNED; treating a `u64` comparison as unsigned requires the
    explicit `UGE`/`UGT`/`ULE`/`ULT` operators.
    """
    return _bare(ty_name) in _SIGNED_TYPES


def parse_int_literal(
    body: str, width: int, signed: Optional[bool], magnitude: bool = False
) -> int:
    """Parse an AST numeric literal, validating it against the declared width.

    The parser stores numbers as strings (decimal or ``0x`` hex). Plain
    ``int(body)`` is too permissive on both sides: it accepts ``1_000`` and
    values far outside the type, and ``z3.BitVecVal`` then silently truncates
    the latter — so a literal the type cannot hold would be modelled as if it
    could. Anything malformed or out of range is DECLINED instead.

    ``magnitude=True`` allows the value ``2**(width-1)``, which is only
    representable as the operand of a unary minus (``-2**63`` for ``i64``).
    """
    if not body or "_" in body:
        raise Decline(f"malformed numeric literal {body!r}")
    try:
        value = int(body, 16) if body.lower().startswith("0x") else int(body, 10)
    except ValueError:
        raise Decline(f"malformed numeric literal {body!r}")

    if signed:
        lo = -(1 << (width - 1))
        hi = (1 << (width - 1)) if magnitude else (1 << (width - 1)) - 1
    else:
        lo, hi = 0, (1 << width) - 1
    if not lo <= value <= hi:
        kind = "i" if signed else "u"
        raise Decline(f"literal {body} is out of range for {kind}{width}")
    return value


# A binding is (z3 term, BitVec width or None, signed or None).
Binding = Tuple[Any, Optional[int], Optional[bool]]


# --------------------------------------------------------------------------
# expression evaluation
# --------------------------------------------------------------------------

def eval_expr(
    expr: Any,
    env: Dict[str, Binding],
    state: Dict[str, Binding],
    declines: List[str],
    expected_width: Optional[int] = None,
    pre_env: Optional[Dict[str, Binding]] = None,
    pre_state: Optional[Dict[str, Binding]] = None,
    expected_signed: Optional[bool] = None,
) -> Binding:
    """Wrapper that turns a recursion overrun into a Decline, never a traceback."""
    try:
        return _eval_expr_inner(
            expr, env, state, declines, expected_width, pre_env, pre_state,
            expected_signed,
        )
    except RecursionError:
        raise Decline("expression is too deeply nested to model")


def _eval_expr_inner(
    expr: Any,
    env: Dict[str, Binding],
    state: Dict[str, Binding],
    declines: List[str],
    expected_width: Optional[int] = None,
    pre_env: Optional[Dict[str, Binding]] = None,
    pre_state: Optional[Dict[str, Binding]] = None,
    expected_signed: Optional[bool] = None,
) -> Binding:
    """Evaluate an AST expression to a z3 term; returns ``(term, width, signed)``.

    ``width`` / ``signed`` are ``None`` for booleans. A numeric literal adopts
    the width AND signedness of the other operand, so `u64_var > 0` compares
    unsigned.

    ``pre_env`` / ``pre_state`` are the bindings from before the function body
    ran. They exist solely so ``old(x)`` can resolve against the PRE-state
    value; without them ``old(total)`` would silently read the post-state value
    the body just wrote, which proves false postconditions.
    """
    if isinstance(expr, str):
        raise Decline(f"bare unit expression {expr!r}")
    if not isinstance(expr, dict) or len(expr) != 1:
        raise Decline(f"malformed expression node {expr!r}")

    tag, body = next(iter(expr.items()))

    if tag == "Number":
        if not isinstance(body, str):
            raise Decline(f"malformed number node {expr!r}")
        width = expected_width or 64
        value = parse_int_literal(body, width, expected_signed)
        return z3.BitVecVal(value, width), width, bool(expected_signed)

    if tag == "Bool":
        return z3.BoolVal(bool(body)), None, None

    if tag == "Unit":
        raise Decline("unit value cannot be used inside an expression")

    if tag == "Ident":
        name = body
        if name in env:
            return env[name]
        if name in state:
            return state[name]
        raise Decline(f"unknown name {name!r}")

    if tag == "Binary":
        return _eval_binary(
            body, env, state, declines, expected_width, pre_env, pre_state,
            expected_signed,
        )

    if tag == "Unary":
        op = body["op"]
        # Fold `-<literal>` BEFORE evaluating the operand: `2**(width-1)` is
        # rejected as a literal on its own but is exactly representable as
        # `-2**(width-1)` in two's complement (e.g. -2**63 for i64).
        if op == "Neg":
            operand = body["expr"]
            if isinstance(operand, dict) and "Number" in operand:
                width = expected_width or 64
                magnitude = parse_int_literal(
                    operand["Number"], width, True, magnitude=True
                )
                return z3.BitVecVal(-magnitude, width), width, bool(expected_signed)
            # otherwise fall through and evaluate the operand normally
        inner, width, signed = eval_expr(
            body["expr"], env, state, declines, expected_width, pre_env,
            pre_state, expected_signed,
        )
        if op == "Not":
            if z3.is_bool(inner):
                return z3.Not(inner), None, None
            raise Decline("`!` applied to a non-boolean")
        if op == "Neg":
            if width is None:
                raise Decline("unary `-` applied to a boolean")
            return -inner, width, signed
        raise Decline(f"unary operator {op} is not modelled")

    if tag == "Call":
        fn = body["func"]
        if fn == "old":
            args = body["args"]
            if len(args) != 1:
                raise Decline("`old()` takes exactly one argument")
            if pre_env is None or pre_state is None:
                raise Decline("`old()` used where no pre-state is available")
            return eval_expr(
                args[0], pre_env, pre_state, declines, expected_width,
                expected_signed=expected_signed,
            )
        raise Decline(f"function call {fn!r} is not modelled")

    raise Decline(f"expression form {tag} is not in the modelled subset")


def _eval_binary(
    body: Dict[str, Any],
    env: Dict[str, Binding],
    state: Dict[str, Binding],
    declines: List[str],
    expected_width: Optional[int],
    pre_env: Optional[Dict[str, Binding]] = None,
    pre_state: Optional[Dict[str, Binding]] = None,
    expected_signed: Optional[bool] = None,
) -> Binding:
    op = body["op"]

    if op == "Assign":
        raise Decline("nested assignment is not an expression")

    # The left operand sets the expected width/sign for the right one, so a
    # bare literal in `u64_var > 0` is treated as unsigned.
    left, lw, ls = eval_expr(
        body["left"], env, state, declines, expected_width, pre_env,
        pre_state, expected_signed,
    )
    right, rw, rs = eval_expr(
        body["right"], env, state, declines, lw, pre_env, pre_state, ls
    )

    # A bare numeric literal has no intrinsic width. If exactly one side is a
    # literal, let it adopt the other side's width/signedness instead of
    # declining — otherwise `0 <= x` and `x >= 0` mean different things
    # depending on operand order.
    if lw is not None and rw is not None and lw != rw:
        left_is_lit = isinstance(body["left"], dict) and set(body["left"]) == {"Number"}
        right_is_lit = isinstance(body["right"], dict) and set(body["right"]) == {"Number"}
        if right_is_lit and not left_is_lit:
            right, rw, rs = eval_expr(
                body["right"], env, state, declines, lw, pre_env, pre_state, ls
            )
        elif left_is_lit and not right_is_lit:
            left, lw, ls = eval_expr(
                body["left"], env, state, declines, rw, pre_env, pre_state, rs
            )

    if lw is not None and rw is not None and lw != rw:
        raise Decline(
            f"mixed-width arithmetic: {lw}-bit vs {rw}-bit operands are not "
            "silently extended"
        )
    if lw is not None and rw is not None and ls != rs:
        raise Decline(
            f"mixed-signedness arithmetic: {'signed' if ls else 'unsigned'} vs "
            f"{'signed' if rs else 'unsigned'} operands are not silently coerced"
        )

    # `&&` / `||` are truthiness operators: booleans AND integers are both
    # accepted (non-zero = true). They must be handled before the boolean
    # decline below, otherwise `b && c` on two booleans is refused while
    # `1 && 2` is accepted — the exact opposite of useful.
    if op == "And":
        return z3.And(_truthy(left), _truthy(right)), None, None
    if op == "Or":
        return z3.Or(_truthy(left), _truthy(right)), None, None

    if (lw is None) != (rw is None):
        # A boolean operand meeting an integer one (e.g. `b + 1`). Adding a
        # z3 Bool to a BitVec raises a sort mismatch, which is not a verdict.
        raise Decline("mixed boolean and integer operands are not modelled")
    width = lw if lw is not None else rw
    signed = ls if ls is not None else rs

    if op in ("Eq", "Ne", "Lt", "Gt", "Le", "Ge"):
        if width is None:
            if op not in ("Eq", "Ne"):
                raise Decline(f"operator {op} on booleans is not modelled")
            return (left == right if op == "Eq" else left != right), None, None
        if op == "Eq":
            return left == right, None, None
        if op == "Ne":
            return left != right, None, None
        # z3py's `<` / `>=` on BitVecRef are SIGNED; unsigned needs the
        # explicit U-prefixed operators.
        if op == "Lt":
            return ((left < right) if signed else z3.ULT(left, right)), None, None
        if op == "Gt":
            return ((left > right) if signed else z3.UGT(left, right)), None, None
        if op == "Le":
            return ((left <= right) if signed else z3.ULE(left, right)), None, None
        if op == "Ge":
            return ((left >= right) if signed else z3.UGE(left, right)), None, None

    if width is None:
        raise Decline(f"operator {op} on booleans is not modelled")

    if op == "Add":
        return left + right, width, signed
    if op == "Sub":
        return left - right, width, signed
    if op == "Mul":
        return left * right, width, signed
    if op == "Div":
        raise Decline("integer division `/` is not modelled (declined, not guessed)")
    if op == "Mod":
        raise Decline("integer remainder `%` is not modelled (declined, not guessed)")
    if op == "BitAnd":
        return left & right, width, signed
    if op == "BitOr":
        return left | right, width, signed
    if op == "BitXor":
        return left ^ right, width, signed
    if op == "Shl":
        # WASM interprets the shift count modulo the bit width; z3's bvshl
        # yields 0 once the count reaches the width. Mask to match the machine.
        mask = z3.BitVecVal(width - 1, width)
        return left << (right & mask), width, signed
    if op == "Shr":
        # `>>` in z3py is the ARITHMETIC shift; the logical one is LShR.
        mask = z3.BitVecVal(width - 1, width)
        count = right & mask
        return ((left >> count) if signed else z3.LShR(left, count)), width, signed
    raise Decline(f"binary operator {op} is not modelled")


def _truthy(term: Any) -> Any:
    if z3.is_bool(term):
        return term
    return term != z3.BitVecVal(0, term.size())


def as_bool(term: Any, declines: List[str]) -> Any:
    if z3.is_bool(term):
        return term
    if z3.is_bv(term):
        declines.append(
            "a non-boolean postcondition/invariant was treated as `!= 0`; "
            "this is a modelling choice, not a proof of the written predicate"
        )
        return _truthy(term)
    raise Decline("goal expression is neither boolean nor integer")


# --------------------------------------------------------------------------
# statement execution (straight-line subset)
# --------------------------------------------------------------------------

def execute_body(
    body: List[Dict[str, Any]],
    env: Dict[str, Binding],
    state: Dict[str, Binding],
    declines: List[str],
) -> None:
    """Symbolically execute a straight-line function body in place.

    ``env`` holds locals/parameters; ``state`` holds contract state fields and is
    mutated by assignments.
    """
    for stmt in body:
        if not isinstance(stmt, dict) or len(stmt) != 1:
            raise Decline(f"malformed statement node {stmt!r}")
        tag, body_s = next(iter(stmt.items()))

        if tag in ("Let", "Const"):
            name = body_s["name"]
            if name in state:
                # A local with the same name as a state field makes every
                # postcondition ambiguous: `post: count == 0` would read the
                # local (which dies at return) instead of the field, so the
                # verifier would prove a statement about nothing. Decline it.
                raise Decline(
                    f"local '{name}' shadows state field '{name}'; postconditions "
                    "about state would be unsound"
                )
            ty = body_s.get("ty")
            width = declared_width(ty["name"]) if isinstance(ty, dict) else None
            declared_sign = type_signed(ty["name"]) if isinstance(ty, dict) else None
            value, vwidth, vsigned = eval_expr(
                body_s["value"], env, state, declines, width,
                expected_signed=declared_sign,
            )
            # An explicit annotation wins over the inferred type; without one
            # the value's own type is kept.
            binding = (
                value,
                width if width is not None else vwidth,
                declared_sign if declared_sign is not None else vsigned,
            )
            _check_binding(name, binding, "local")
            env[name] = binding
            continue

        if tag == "Expr":
            expr = body_s
            if isinstance(expr, dict) and len(expr) == 1 and "Binary" in expr:
                binary = expr["Binary"]
                if binary.get("op") == "Assign" and isinstance(binary["left"], dict):
                    left = binary["left"]
                    if "Ident" in left:
                        name = left["Ident"]
                        width, signed = _binding_of(env, state, name)
                        value, _, _ = eval_expr(
                            binary["right"],
                            env,
                            state,
                            declines,
                            width,
                            expected_signed=signed,
                        )
                        _assign(name, value, env, state, declines)
                        continue
            eval_expr(expr, env, state, declines)
            continue

        if tag == "Return":
            if body_s is not None:
                eval_expr(body_s, env, state, declines)
            return

        raise Decline(
            f"statement form {tag} is not in the modelled subset "
            "(if/while/for/match/break/continue are declined)"
        )


def _binding_of(
    env: Dict[str, Binding], state: Dict[str, Binding], name: str
) -> Tuple[Optional[int], Optional[bool]]:
    if name in env:
        return env[name][1], env[name][2]
    if name in state:
        return state[name][1], state[name][2]
    return None, None


def _check_binding(name: str, binding: Binding, where: str) -> None:
    """Refuse a binding whose z3 term does not match its declared type.

    Without this, `x = y` with `x: u8` and `y: u64` stores a 64-bit term under a
    binding recorded as 8-bit; the recorded width then drives the literal width
    for later comparisons while the term keeps its real sort, and z3 raises a
    sort mismatch (or worse, silently compares different widths).
    """
    term, width, signed = binding
    if width is None:
        if not z3.is_bool(term):
            raise Decline(
                f"{where} '{name}' is declared boolean but the value is an integer"
            )
        return
    if z3.is_bool(term):
        raise Decline(
            f"{where} '{name}' is declared {'i' if signed else 'u'}{width} "
            "but the value is boolean"
        )
    if not z3.is_bv(term):
        raise Decline(f"{where} '{name}' has a non-bitvector value")
    if term.size() != width:
        kind = "i" if signed else "u"
        raise Decline(
            f"{where} '{name}' is declared {kind}{width} but the value is "
            f"{term.size()}-bit (mixed-width assignment is not silently truncated)"
        )


def _assign(
    name: str,
    value: Any,
    env: Dict[str, Binding],
    state: Dict[str, Binding],
    declines: List[str],
) -> None:
    if name in env:
        binding = (value, env[name][1], env[name][2])
        _check_binding(name, binding, "assignment to local")
        env[name] = binding
        return
    if name in state:
        binding = (value, state[name][1], state[name][2])
        _check_binding(name, binding, "assignment to state field")
        state[name] = binding
        return
    raise Decline(f"assignment to unknown name {name!r}")


# --------------------------------------------------------------------------
# obligations
# --------------------------------------------------------------------------

@dataclass
class Obligation:
    function: str
    kind: str
    status: str
    detail: str = ""
    model: Optional[Dict[str, str]] = None


@dataclass
class FunctionReport:
    name: str
    obligations: List[Obligation] = field(default_factory=list)
    declines: List[str] = field(default_factory=list)

    @property
    def passed(self) -> bool:
        # A recorded decline means the layer did not fully model the function.
        # Reporting `passed` while a decline is on the record would let a green
        # exit code coexist with "this is not a proof of the written predicate".
        return (
            bool(self.obligations)
            and all(o.status == "Proven" for o in self.obligations)
            and not self.declines
        )


def verify_function(
    fn: Dict[str, Any],
    state_types: Dict[str, str],
    contract_invariants: Optional[List[Any]] = None,
) -> FunctionReport:
    report = FunctionReport(name=fn["name"])

    env: Dict[str, Binding] = {}
    param_names = set()
    for p in fn.get("params", []):
        name = p["name"]
        # A parameter that shadows a state field (or another parameter) makes
        # every clause ambiguous: `post: count == 0` would read the parameter,
        # not the field, and the verifier would prove a statement about nothing.
        if name in state_types:
            raise Decline(
                f"parameter '{name}' shadows state field '{name}'; clauses "
                "about state would be unsound"
            )
        if name in param_names:
            raise Decline(f"duplicate parameter '{name}'")
        param_names.add(name)
        ty = p["type"]
        width = declared_width(ty)
        kind = type_kind(ty)
        if kind == "int":
            # FreshConst, not a user-named symbol: a parameter named `x__pre`
            # would otherwise BE the state symbol and silently alias it.
            env[name] = (
                z3.FreshConst(z3.BitVecSort(width or 64), "rs_param"),
                width,
                type_signed(ty),
            )
        elif kind == "bool":
            env[name] = (z3.FreshConst(z3.BoolSort(), "rs_param"), None, None)
        else:
            report.declines.append(f"parameter '{name}' has unsupported type '{ty}'")

    state: Dict[str, Binding] = {}
    for name, ty in state_types.items():
        width = declared_width(ty)
        kind = type_kind(ty)
        if kind == "int":
            state[name] = (
                z3.FreshConst(z3.BitVecSort(width or 64), "rs_state_pre"),
                width,
                type_signed(ty),
            )
        elif kind == "bool":
            state[name] = (z3.FreshConst(z3.BoolSort(), "rs_state_pre"), None, None)
        else:
            report.declines.append(f"state field '{name}' has unsupported type '{ty}'")

    pre = fn.get("pre")
    post = fn.get("post")
    invariant = fn.get("invariant")

    if post is None and invariant is None:
        # A function must carry its OWN proof. Contract invariants are
        # additional obligations, never a substitute: otherwise a function with
        # no `@verified` at all would report passed=True.
        report.declines.append("no postcondition and no invariant clause")
        return report

    goals: List[Tuple[str, Any]] = []
    if post is not None:
        goals.append(("post", post))
    if invariant is not None:
        goals.append(("invariant", invariant))
    # Contract-level invariants are obligations on EVERY function: assume they
    # hold before the call, prove they hold after it. Without this they are
    # exported to JSON and then silently ignored.
    for i, inv in enumerate(contract_invariants or []):
        goals.append((f"contract_invariant[{i}]", inv))

    pre_term = z3.BoolVal(True)
    if pre is not None:
        try:
            raw, _, _ = eval_expr(pre, env, state, report.declines)
            pre_term = as_bool(raw, report.declines)
        except Decline as d:
            report.declines.append(f"precondition: {d.reason}")
            # The precondition could not be modelled, so nothing downstream can
            # be discharged honestly: record an Unverified obligation per goal
            # rather than an empty report that reads as "nothing to check".
            for kind, _ in goals:
                report.obligations.append(
                    Obligation(fn["name"], kind, "Unverified", f"precondition: {d.reason}")
                )
            return report

    state_pre = dict(state)
    env_pre = dict(env)

    # VACUOUS-PROOF GUARD (computed once; see the use site below).
    #
    # A `post` goal is discharged when `pre ∧ ¬post` is unsat. That unsat can
    # mean "the post follows" OR "the premise is empty" -- and an empty premise
    # proves nothing about executable behaviour. Compute the premise's
    # satisfiability ONCE here rather than inside the goal loop: it does not
    # depend on the goal.
    #
    # NOTE the inductive goals (`invariant`, `contract_invariant[i]`) use a
    # DIFFERENT premise -- pre ∧ invariant -- so their premise is checked
    # separately inside the loop, where that term is available.
    try:
        execute_body(fn["body"], env, state, report.declines)
    except Decline as d:
        report.declines.append(f"body: {d.reason}")
        # Same rule: a body that cannot be modelled leaves every goal
        # unchecked, and that must be visible in the obligations.
        for kind, _ in goals:
            report.obligations.append(
                Obligation(fn["name"], kind, "Unverified", f"body: {d.reason}")
            )
        return report

    for kind, expr in goals:
        try:
            raw, _, _ = eval_expr(
                expr, env, state, report.declines,
                pre_env=env_pre, pre_state=state_pre,
            )
            goal = as_bool(raw, report.declines)
        except Decline as d:
            report.obligations.append(
                Obligation(fn["name"], kind, "Unverified", f"goal: {d.reason}")
            )
            continue

        # VACUOUS-PROOF GUARD.
        #
        # `premise ∧ ¬goal` unsat is the proof condition, but unsat can arise
        # from two very different reasons: the goal genuinely follows from the
        # premise, or the premise itself is empty. `contract C { state { x: u64 }
        # @verified(pre: x < 0, post: x == 12345) }` has an unsatisfiable
        # precondition (a u64 is never negative), so pre ∧ ¬goal is trivially
        # unsat and the naive check reports `Proven`. That is a vacuous proof:
        # no executable call can satisfy the premise, so the obligation says
        # nothing about the running contract and must not be reported as
        # discharged. Soundness of the discharged claim requires a non-empty
        # premise, so every goal is Unverified instead.
        #
        # Each kind of goal has its own premise:
        #   post                  -> pre
        #   invariant, contract_* -> pre ∧ invariant   (inductive protocol)
        # so satisfiability is evaluated per goal, not once per function.
        premise = pre_term
        if kind != "post":
            # Inductive invariant protocol: assume the invariant held in the
            # pre-state, then prove it still holds in the post-state.
            try:
                inv_pre_raw, _, _ = eval_expr(expr, env_pre, state_pre, report.declines)
                premise = z3.And(pre_term, as_bool(inv_pre_raw, report.declines))
            except Decline as d:
                report.obligations.append(
                    Obligation(fn["name"], kind, "Unverified", f"pre-state: {d.reason}")
                )
                continue
        premise_solver = z3.Solver()
        premise_solver.add(premise)
        if premise_solver.check() != z3.sat:
            report.obligations.append(
                Obligation(
                    fn["name"],
                    kind,
                    "Unverified",
                    "the premise is unsatisfiable; a proof over an empty input "
                    "range is vacuous, not a discharge",
                )
            )
            continue

        # Reuse the premise just checked: the goal loop's solver asserts the
        # same premise the vacuity guard just proved satisfiable.
        solver = z3.Solver()
        solver.add(premise)
        solver.add(z3.Not(goal))

        result = solver.check()
        if result == z3.unsat:
            report.obligations.append(Obligation(fn["name"], kind, "Proven"))
        elif result == z3.sat:
            model = solver.model()
            pretty = {
                name: _model_value(model, term)
                for name, (term, _w, _s) in list(env.items()) + list(state.items())
            }
            report.obligations.append(
                Obligation(
                    fn["name"],
                    kind,
                    "Counterexample",
                    "pre ∧ ¬goal is satisfiable",
                    pretty,
                )
            )
        else:
            report.obligations.append(
                Obligation(fn["name"], kind, "Unverified", "z3 returned unknown")
            )

    return report


def _model_value(model: z3.ModelRef, term: Any) -> str:
    try:
        value = model.eval(term, model_completion=True)
    except Exception:
        return "<unknown>"
    if z3.is_true(value):
        return "true"
    if z3.is_false(value):
        return "false"
    if z3.is_bv_value(value):
        return str(value.as_long())
    return str(value)


def verify_contract(contract: Dict[str, Any]) -> Dict[str, Any]:
    if not isinstance(contract, dict):
        r = FunctionReport(name="?")
        r.declines.append(f"verification input is not an object: {type(contract).__name__}")
        return {
            "contract": None,
            "functions": [],
            "summary": {
                "proven": 0, "counterexample": 0, "unverified": 0,
                "declines": 1, "all_proven": False,
            },
        }

    state_types: Dict[str, str] = {}
    malformed: List[str] = []

    raw_state = contract.get("state", [])
    if not isinstance(raw_state, list):
        malformed.append(f"'state' is not a list: {type(raw_state).__name__}")
        raw_state = []
    for f in raw_state:
        if not isinstance(f, dict) or "name" not in f or "type" not in f:
            malformed.append(f"malformed state entry {f!r}")
            continue
        state_types[f["name"]] = f["type"]

    contract_invariants = contract.get("invariants") or []
    if not isinstance(contract_invariants, list):
        malformed.append(f"'invariants' is not a list: {type(contract_invariants).__name__}")
        contract_invariants = []

    raw_functions = contract.get("functions", [])
    if not isinstance(raw_functions, list):
        malformed.append(f"'functions' is not a list: {type(raw_functions).__name__}")
        raw_functions = []

    reports: List[FunctionReport] = []
    for fn in raw_functions:
        if not isinstance(fn, dict):
            r = FunctionReport(name=repr(fn)[:40])
            r.declines.append(f"function entry is not an object: {fn!r}")
            reports.append(r)
            continue
        try:
            reports.append(verify_function(fn, state_types, contract_invariants))
        except Decline as d:
            r = FunctionReport(name=str(fn.get("name", "?")))
            r.declines.append(str(d))
            reports.append(r)
        except (KeyError, TypeError) as exc:
            # A hand-edited JSON document can omit keys the exporter always
            # writes. That is a malformed input, not a traceback.
            r = FunctionReport(name=str(fn.get("name", "?")))
            r.declines.append(f"malformed verification input: {exc}")
            reports.append(r)

    if not reports:
        r = FunctionReport(name=str(contract.get("contract", "?")))
        r.declines.extend(malformed or ["no functions to verify"])
        reports.append(r)
    elif malformed:
        reports[0].declines.extend(malformed)

    obligations = [o for r in reports for o in r.obligations]
    declines = [d for r in reports for d in r.declines]
    # Every function must carry its own proof. A function with no @verified
    # clauses has no obligations, and counting that as "proven" would make the
    # gate vacuous — the language's premise is that such a function is an
    # error, so it is reported as not passed.
    all_proven = bool(reports) and all(r.passed for r in reports)
    return {
        "contract": contract.get("contract"),
        "functions": [
            {
                "name": r.name,
                "obligations": [
                    {"kind": o.kind, "status": o.status, "detail": o.detail, "model": o.model}
                    for o in r.obligations
                ],
                "declines": r.declines,
                "passed": r.passed,
            }
            for r in reports
        ],
        "summary": {
            "proven": sum(1 for o in obligations if o.status == "Proven"),
            "counterexample": sum(1 for o in obligations if o.status == "Counterexample"),
            "unverified": sum(1 for o in obligations if o.status == "Unverified"),
            "declines": len(declines),
            "all_proven": all_proven,
        },
    }


def verify_contracts(contracts: List[Dict[str, Any]]) -> Dict[str, Any]:
    results = [verify_contract(c) for c in contracts]
    return {
        "contracts": results,
        "summary": {
            "contracts": len(results),
            "all_proven": bool(results) and all(r["summary"]["all_proven"] for r in results),
        },
    }


# --------------------------------------------------------------------------
# self-test
# --------------------------------------------------------------------------
#
# Every case below is written in the exact JSON shape `emit_verification`
# produces, so the self-test exercises the same path a real contract takes.
# Each case names the status it EXPECTS, and case 2 is the falsifier for the
# `old()` fix: if `old(x)` reads the post-state again, it reports Proven and
# this self-test fails.

def _num(n: Any) -> Dict[str, Any]:
    return {"Number": str(n)}


def _id(name: str) -> Dict[str, Any]:
    return {"Ident": name}


def _bin(op: str, left: Any, right: Any) -> Dict[str, Any]:
    return {"Binary": {"left": left, "op": op, "right": right}}


def _set(name: str, value: Any) -> Dict[str, Any]:
    return {"Expr": _bin("Assign", _id(name), value)}


def _ret(value: Any = None) -> Dict[str, Any]:
    return {"Return": value}


def _old(name: str) -> Dict[str, Any]:
    return {"Call": {"func": "old", "args": [_id(name)]}}


def _neg(value: Any) -> Dict[str, Any]:
    return {"Unary": {"op": "Neg", "expr": value}}


def _contract(name: str, state: List[Any], fn: Dict[str, Any],
              invariants: Optional[List[Any]] = None) -> Dict[str, Any]:
    # state entries are either a name (default u64) or a (name, type) pair.
    fields = []
    for item in state:
        if isinstance(item, tuple):
            fields.append({"name": item[0], "type": item[1]})
        else:
            fields.append({"name": item, "type": "u64"})
    out: Dict[str, Any] = {"contract": name, "state": fields, "functions": [fn]}
    if invariants:
        out["invariants"] = invariants
    return out


def _fn(name: str, body: List[Any], pre: Any = None, post: Any = None,
        invariant: Any = None, params: Optional[List[str]] = None) -> Dict[str, Any]:
    return {
        "name": name,
        "params": [{"name": p, "type": "u64"} for p in (params or [])],
        "pre": pre,
        "post": post,
        "invariant": invariant,
        "body": body,
    }


_SELFTEST_CASES = [
    (
        "deposit proves post and invariant",
        _contract(
            "Deposit",
            ["total"],
            _fn(
                "deposit",
                [_set("total", _bin("Add", _id("total"), _id("amount"))), _ret(_id("amount"))],
                pre=_bin("Gt", _id("amount"), _num(0)),
                post=_bin("Eq", _id("total"), _bin("Add", _old("total"), _id("amount"))),
                invariant=_bin("Ge", _id("total"), _num(0)),
                params=["amount"],
            ),
        ),
        {"post": "Proven", "invariant": "Proven"},
    ),
    (
        "old() reads the PRE-state, not the post-state",
        _contract(
            "OldSemantics",
            ["x"],
            _fn(
                "zero",
                [_set("x", _num(0)), _ret(None)],
                pre={"Bool": True},
                post=_bin("Eq", _id("x"), _old("x")),
                invariant=_bin("Ge", _id("x"), _num(0)),
            ),
        ),
        {"post": "Counterexample"},
    ),
    (
        "a body that adds 2 does not prove a +1 postcondition",
        _contract(
            "WrongBody",
            ["x"],
            _fn(
                "bump",
                [_set("x", _bin("Add", _id("x"), _num(2))), _ret(None)],
                pre={"Bool": True},
                post=_bin("Eq", _id("x"), _bin("Add", _old("x"), _num(1))),
                invariant=_bin("Ge", _id("x"), _num(0)),
            ),
        ),
        {"post": "Counterexample"},
    ),
    (
        "division is declined, never modelled",
        _contract(
            "DivDeclined",
            ["x"],
            _fn(
                "half",
                [_set("x", _bin("Div", _id("x"), _num(2))), _ret(None)],
                pre={"Bool": True},
                post=_bin("Eq", _id("x"), _old("x")),
                invariant=_bin("Ge", _id("x"), _num(0)),
            ),
        ),
        {"post": "Unverified"},
    ),
    (
        # Falsifier for signedness: 2^64-1 is a large POSITIVE u64. Modelled as
        # signed it is -1, `x > 0` is refuted, and this case reports
        # Counterexample instead of Proven.
        "u64 comparisons are unsigned (2^64-1 > 0)",
        _contract(
            "Unsigned",
            ["x"],
            _fn(
                "set_max",
                [_set("x", _num(18446744073709551615)), _ret(None)],
                pre={"Bool": True},
                post=_bin("Gt", _id("x"), _num(0)),
                invariant=_bin("Ge", _id("x"), _num(0)),
            ),
        ),
        {"post": "Proven", "invariant": "Proven"},
    ),
    (
        # Regression: `&&` was unreachable for booleans — an earlier
        # `width is None` guard declined it, so `b && c` could never be proved.
        "&& on booleans is modelled",
        _contract(
            "BoolAnd",
            [("b", "bool"), ("c", "bool")],
            _fn(
                "f",
                [_ret(None)],
                pre=_bin("And", _id("b"), _id("c")),
                post=_id("b"),
            ),
        ),
        {"post": "Proven"},
    ),
    (
        # Regression: z3's bvshl yields 0 once the count reaches the width;
        # WASM interprets the count modulo the width, so x << 64 == x for u64.
        "WASM shift count is modulo the width",
        _contract(
            "Shift",
            ["x"],
            _fn(
                "f",
                [_set("x", _bin("Shl", _id("x"), _num(64))), _ret(None)],
                pre={"Bool": True},
                post=_bin("Eq", _id("x"), _old("x")),
            ),
        ),
        {"post": "Proven"},
    ),
    (
        "hex literal 0xFF is 255",
        _contract(
            "Hex",
            ["x"],
            _fn(
                "f",
                [_set("x", _num("0xFF")), _ret(None)],
                pre={"Bool": True},
                post=_bin("Eq", _id("x"), _num(255)),
            ),
        ),
        {"post": "Proven"},
    ),
    (
        "literal beyond u64 is declined, not truncated",
        _contract(
            "TooBig",
            ["x"],
            _fn(
                "f",
                [_set("x", _num(18446744073709551616)), _ret(None)],
                pre={"Bool": True},
                post=_bin("Ge", _id("x"), _num(0)),
            ),
        ),
        {"post": "Unverified"},
    ),
    (
        "boolean arithmetic is declined (z3 sort mismatch)",
        _contract(
            "BoolArith",
            [("b", "bool")],
            _fn(
                "f",
                [_ret(None)],
                pre={"Bool": True},
                post=_bin("Gt", _bin("Add", _id("b"), _num(1)), _num(0)),
            ),
        ),
        {"post": "Unverified"},
    ),
    (
        "-2**63 is representable as an i64 literal",
        _contract(
            "I64Min",
            [("x", "i64")],
            _fn(
                "f",
                [_set("x", _neg(_num(9223372036854775808))), _ret(None)],
                pre={"Bool": True},
                post=_bin("Lt", _id("x"), _num(0)),
            ),
        ),
        {"post": "Proven"},
    ),
    (
        # Regression: a local with the same name as a state field made
        # `post: count == 0` read the local (which dies at return) and PROVE.
        "a local cannot shadow a state field",
        _contract(
            "Shadow",
            ["count"],
            _fn(
                "fake_reset",
                [{"Let": {"mutable": False, "name": "count", "ty": None,
                          "value": _num(0)}}, _ret(None)],
                pre={"Bool": True},
                post=_bin("Eq", _id("count"), _num(0)),
            ),
        ),
        {"post": "Unverified"},
    ),
    (
        # Regression: contract-level invariants were exported but never checked.
        "contract-level invariants are discharged per function",
        _contract(
            "ContractInv",
            ["x"],
            _fn(
                "set1",
                [_set("x", _num(1)), _ret(None)],
                pre={"Bool": True},
                post=_bin("Eq", _id("x"), _num(1)),
            ),
            invariants=[_bin("Eq", _id("x"), _num(0))],
        ),
        {"post": "Proven", "contract_invariant[0]": "Counterexample"},
    ),
    (
        # Regression: `x = y` across widths stored a 64-bit term under a u8
        # binding; z3 then raised a sort mismatch instead of returning a verdict.
        "mixed-width assignment is declined, not silently truncated",
        _contract(
            "MixedWidth",
            [("x", "u8"), ("y", "u64")],
            _fn(
                "copy",
                [_set("x", _id("y")), _ret(None)],
                pre={"Bool": True},
                post=_bin("Ge", _id("x"), _num(0)),
            ),
        ),
        {"post": "Unverified"},
    ),
    (
        # Regression: a parameter with the same name as a state field made the
        # clause read the parameter, so `post: x == 0` was Proven by nothing.
        "a parameter cannot shadow a state field",
        _contract(
            "ParamShadow",
            ["x"],
            _fn(
                "f",
                [_ret(None)],
                pre={"Bool": True},
                post=_bin("Eq", _id("x"), _num(0)),
                params=["x"],
            ),
        ),
        {},
    ),
    (
        # Regression: a parameter named `x__pre` aliased the state's pre-state
        # z3 symbol (both were named after the source identifier).
        "a parameter named like the old state symbol does not alias it",
        _contract(
            "SymbolAlias",
            ["x"],
            _fn(
                "f",
                [_set("x", _bin("Add", _id("x"), _id("x__pre"))), _ret(None)],
                pre={"Bool": True},
                post=_bin("Eq", _id("x"), _bin("Add", _old("x"), _id("x__pre"))),
                params=["x__pre"],
            ),
        ),
        {"post": "Proven"},
    ),
    (
        # Regression: contract invariants alone must not make an unannotated
        # function pass — every function carries its OWN proof.
        "an unannotated function is not passed even with contract invariants",
        _contract(
            "NoAnnotation",
            ["x"],
            _fn("get", [_ret(_id("x"))], pre=None, post=None, invariant=None),
            invariants=[_bin("Ge", _id("x"), _num(0))],
        ),
        {},
    ),
    (
        # Regression: VACUOUS PROOF.
        #
        # `pre ∧ ¬post` unsat is the proof condition, but unsat also arises when
        # the precondition is EMPTY. `pre: x < 0` cannot hold for a u64 state
        # field, so pre ∧ ¬post is trivially unsat and the naive check reported
        # the postcondition as Proven. No executable call satisfies that
        # premise, so the obligation proved nothing about the running contract.
        # A proof over an empty input range is not a discharge: report
        # Unverified instead.
        "an unsatisfiable precondition is not a proof",
        _contract(
            "Vacuous",
            ["x"],
            _fn(
                "impossible",
                [_ret(_id("x"))],
                pre=_bin("Lt", _id("x"), _num(0)),
                post=_bin("Eq", _id("x"), _num(12345)),
            ),
        ),
        {"post": "Unverified"},
    ),
    (
        # The same premise becomes non-vacuous once it can hold, and then the
        # false postcondition is a genuine Counterexample rather than a vacuous
        # pass. Without this pair, the case above could be "fixed" by declining
        # every unsatisfiable-looking contract.
        "a satisfiable precondition still catches a false postcondition",
        _contract(
            "NonVacuous",
            ["x"],
            _fn(
                "bad",
                [_ret(_id("x"))],
                pre=_bin("Gt", _id("x"), _num(100)),
                post=_bin("Lt", _id("x"), _num(100)),
            ),
        ),
        {"post": "Counterexample"},
    ),
    (
        # Regression: a bare literal has no intrinsic width, so `0 <= x` was
        # declined as mixed-width while `x >= 0` was Proven — the same predicate
        # had different verdicts depending on operand order.
        "literal adopts the typed operand's width",
        _contract(
            "OperandOrder",
            [("x", "u8")],
            _fn(
                "f",
                [_ret(None)],
                pre={"Bool": True},
                post=_bin("Le", _num(0), _id("x")),
            ),
        ),
        {"post": "Proven"},
    ),
]


_EXPECT_NOT_PASSED = {
    "an unannotated function is not passed even with contract invariants",
    "a parameter cannot shadow a state field",
}


def selftest() -> int:
    failures: List[str] = []
    for label, contract, expected in _SELFTEST_CASES:
        result = verify_contract(contract)
        obligations = result["functions"][0]["obligations"]
        got = {o["kind"]: o["status"] for o in obligations}
        case_failures: List[str] = []
        for kind, want in expected.items():
            actual = got.get(kind, "MISSING")
            if actual != want:
                case_failures.append(
                    f"{label}: {kind} expected {want}, got {actual} "
                    f"(declines={result['functions'][0]['declines']})"
                )
        # Some cases assert the overall verdict, not individual obligations.
        if label in _EXPECT_NOT_PASSED and result["functions"][0]["passed"]:
            case_failures.append(
                f"{label}: expected passed=False, got True "
                f"(declines={result['functions'][0]['declines']})"
            )
        failures.extend(case_failures)
        print(
            f"{'PASS' if not case_failures else 'FAIL'}  {label}: "
            f"{got or 'no obligations'}"
        )

    if failures:
        print("\nself-test FAILURES:")
        for f in failures:
            print(f"  - {f}")
        return 1
    print(f"\nself-test: {len(_SELFTEST_CASES)}/{len(_SELFTEST_CASES)} cases as expected")
    return 0


def main(argv: List[str]) -> int:
    if "--selftest" in argv:
        return selftest()

    if len(argv) > 1 and argv[1] != "--selftest":
        raw = open(argv[1], "r", encoding="utf-8").read()
    else:
        raw = sys.stdin.read()

    data = json.loads(raw)
    # `emit_verification` emits one entry per parsed contract; a bare object is
    # still accepted so hand-written inputs keep working.
    if isinstance(data, list):
        result = verify_contracts(data)
        json.dump(result, sys.stdout, indent=1, sort_keys=True)
        print()
        return 0 if result["summary"]["all_proven"] else 1

    result = verify_contract(data)
    json.dump(result, sys.stdout, indent=1, sort_keys=True)
    print()
    return 0 if result["summary"]["all_proven"] else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
