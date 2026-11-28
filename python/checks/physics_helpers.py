"""Safe parameter parsing, symbol/matrix normalization, and dimensional algebra for `checks.physics`."""

from __future__ import annotations

import ast
import math
import re
from fractions import Fraction
from typing import Any

import mpmath as mp
import sympy as sp

from .algebra_probe import (
    DEFAULT_NUMERIC_DIGITS,
    DEFAULT_NUMERIC_POINTS,
    _candidates_for_symbol,
    _parse_domain_constraints,
    assumptions_snapshot,
    eval_mp_scalar,
    normalize_symbols,
    parse_simple_real,
)

_SYM_NAME_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]{0,63}$")
_MAX_EXPR_CHARS = 256
_MAX_AST_NODES = 256
_MAX_AST_DEPTH = 16
_MAX_POWER_ABS = 1000

_SAFE_CONSTANTS: dict[str, sp.Basic] = {
    "pi": sp.pi,
    "E": sp.E,
    "e": sp.E,
    "I": sp.I,
    "oo": sp.oo,
    "inf": sp.oo,
    "infty": sp.oo,
    "EulerGamma": sp.EulerGamma,
    "Catalan": sp.Catalan,
    "GoldenRatio": sp.GoldenRatio,
}

_SAFE_MATH_FUNCS: dict[str, Any] = {
    "sin": sp.sin,
    "cos": sp.cos,
    "tan": sp.tan,
    "cot": sp.cot,
    "sec": sp.sec,
    "csc": sp.csc,
    "asin": sp.asin,
    "acos": sp.acos,
    "atan": sp.atan,
    "sinh": sp.sinh,
    "cosh": sp.cosh,
    "tanh": sp.tanh,
    "exp": sp.exp,
    "log": sp.log,
    "sqrt": sp.sqrt,
    "Abs": sp.Abs,
    "conjugate": sp.conjugate,
    "re": sp.re,
    "im": sp.im,
    "Rational": sp.Rational,
    "Integer": sp.Integer,
}

_TRANSCENDENTAL_FUNCS = (
    sp.exp, sp.log, sp.sin, sp.cos, sp.tan, sp.cot, sp.sec, sp.csc,
    sp.asin, sp.acos, sp.atan, sp.atan2, sp.acot, sp.asec, sp.acsc,
    sp.sinh, sp.cosh, sp.tanh, sp.coth, sp.sech, sp.csch,
    sp.asinh, sp.acosh, sp.atanh, sp.acoth, sp.asech, sp.acsch,
    sp.erf, sp.erfc, sp.erfi, sp.gamma, sp.loggamma, sp.beta,
    sp.besselj, sp.bessely, sp.besseli, sp.besselk,
    sp.factorial, sp.factorial2, sp.binomial,
    sp.legendre, sp.hermite, sp.laguerre, sp.assoc_legendre, sp.Ynm,
)


class UnknownDimensionError(ValueError):
    """Raised when an expression uses a symbol or node with undeclared dimension."""


class DimensionMismatchError(ValueError):
    """Raised when an expression or equation violates dimensional consistency."""


def validate_symbol_name(raw: Any, label: str = "symbol") -> str:
    if not isinstance(raw, str):
        raise ValueError(f"{label} must be a symbol name string")
    name = raw.strip()
    if not _SYM_NAME_RE.fullmatch(name) or name.startswith("__"):
        raise ValueError(f"invalid {label} name: {raw!r}")
    return name


def coerce_handle_obj(obj: Any) -> Any:
    """Convert mutable SymPy matrices into immutable `sp.Basic` objects for symbol normalization."""
    if isinstance(obj, sp.MatrixBase) and not isinstance(obj, sp.Basic):
        return obj.as_immutable()
    return obj


def normalize_physics_handles(
    handles: dict[str, Any], params: dict[str, Any]
) -> tuple[dict[str, Any] | None, dict[str, sp.Symbol], str | None]:
    coerced = {k: coerce_handle_obj(v) for k, v in handles.items()}
    return normalize_symbols(coerced, params)


def assumption_mismatch_result(reason: str | None) -> dict[str, Any]:
    return {
        "outcome": "inconclusive",
        "evidence": {"reason": reason or "inconclusive: symbol assumptions differ"},
    }


def _eval_safe_ast(node: ast.AST, assumed: dict[str, sp.Symbol], depth: int, counter: list[int]) -> Any:
    counter[0] += 1
    if counter[0] > _MAX_AST_NODES:
        raise ValueError("parameter expression exceeds AST node cap")
    if depth > _MAX_AST_DEPTH:
        raise ValueError("parameter expression exceeds AST nesting cap")

    if isinstance(node, ast.Constant):
        val = node.value
        if type(val) is int:
            if val.bit_length() > 128:
                raise ValueError("integer literal exceeds 128-bit cap")
            return sp.Integer(val)
        if type(val) is float:
            if not math.isfinite(val):
                raise ValueError("non-finite float literal")
            return sp.Rational(str(val))
        raise ValueError(f"disallowed constant type {type(val).__name__}")

    if isinstance(node, ast.Name):
        name = node.id
        if name.startswith("__"):
            raise ValueError(f"dunder name {name!r} forbidden")
        if name in _SAFE_CONSTANTS:
            return _SAFE_CONSTANTS[name]
        if _SYM_NAME_RE.fullmatch(name):
            return assumed.get(name, sp.Symbol(name))
        raise ValueError(f"invalid identifier {name!r}")

    if isinstance(node, ast.UnaryOp):
        operand = _eval_safe_ast(node.operand, assumed, depth + 1, counter)
        if isinstance(node.op, ast.UAdd):
            return +operand
        if isinstance(node.op, ast.USub):
            return -operand
        raise ValueError("disallowed unary operator")

    if isinstance(node, ast.BinOp):
        left = _eval_safe_ast(node.left, assumed, depth + 1, counter)
        right = _eval_safe_ast(node.right, assumed, depth + 1, counter)
        if isinstance(node.op, ast.Add):
            return left + right
        if isinstance(node.op, ast.Sub):
            return left - right
        if isinstance(node.op, ast.Mult):
            return left * right
        if isinstance(node.op, ast.Div):
            if right == 0:
                raise ValueError("division by zero in parameter expression")
            return left / right
        if isinstance(node.op, ast.Pow):
            if isinstance(right, sp.Integer) and abs(int(right)) > _MAX_POWER_ABS:
                raise ValueError("exponent exceeds cap")
            if isinstance(right, sp.Rational) and (
                abs(int(right.p)) > _MAX_POWER_ABS or abs(int(right.q)) > _MAX_POWER_ABS
            ):
                raise ValueError("rational exponent exceeds cap")
            if isinstance(left, sp.Integer) and isinstance(right, sp.Integer):
                if abs(int(left)) > 1 and int(right) > 256:
                    raise ValueError("integer power exceeds cap")
            return left**right
        raise ValueError("disallowed binary operator")

    if isinstance(node, ast.Call):
        if not isinstance(node.func, ast.Name) or node.keywords:
            raise ValueError("disallowed call form in parameter expression")
        fname = node.func.id
        if fname not in _SAFE_MATH_FUNCS:
            raise ValueError(f"disallowed function {fname!r} in parameter expression")
        if any(isinstance(a, ast.Starred) for a in node.args) or len(node.args) > 4:
            raise ValueError("invalid call arguments in parameter expression")
        args = [_eval_safe_ast(a, assumed, depth + 1, counter) for a in node.args]
        return _SAFE_MATH_FUNCS[fname](*args)

    raise ValueError(f"disallowed AST node {type(node).__name__}")


def parse_safe_expr(raw: Any, assumed: dict[str, sp.Symbol], *, allow_oo: bool = False) -> sp.Expr:
    """Parse a safe scalar math expression via `ast` without `eval` or string `sympify`."""
    if type(raw) is int:
        if raw.bit_length() > 128:
            raise ValueError("integer exceeds 128-bit cap")
        return sp.Integer(raw)
    if type(raw) is float:
        if not math.isfinite(raw):
            raise ValueError("non-finite float")
        return sp.Rational(str(raw))
    if not isinstance(raw, str):
        raise ValueError(f"expected string or number expression, got {type(raw).__name__}")
    text = raw.strip()
    if not text or len(text) > _MAX_EXPR_CHARS:
        raise ValueError("parameter expression must be 1..=256 characters")
    text = text.replace("^", "**").replace("\\pi", "pi").replace("\\infty", "oo")
    try:
        tree = ast.parse(text, mode="eval")
    except (SyntaxError, ValueError, RecursionError) as exc:
        raise ValueError(f"invalid parameter expression {raw!r}: {exc}") from exc
    expr = _eval_safe_ast(tree.body, assumed, 0, [0])
    if not allow_oo and expr.has(sp.oo, -sp.oo, sp.zoo, sp.nan):
        raise ValueError(f"non-finite expression {raw!r} not allowed here")
    return expr


def parse_limit_point(raw: Any, assumed: dict[str, sp.Symbol]) -> tuple[sp.Expr, str]:
    text = str(raw).strip()
    if not text:
        raise ValueError("limit_case requires non-empty `point`")
    simple = parse_simple_real(text)
    if simple is not None:
        return simple, text
    if _SYM_NAME_RE.fullmatch(text):
        return assumed.get(text, sp.Symbol(text)), text
    expr = parse_safe_expr(text, assumed, allow_oo=True)
    return expr, text


def parse_dimension_power(raw: Any) -> sp.Rational:
    if type(raw) is bool:
        raise ValueError("dimension power cannot be bool")
    if type(raw) is int:
        if abs(raw) > 1000:
            raise ValueError("dimension power exceeds cap")
        return sp.Rational(raw)
    if type(raw) is float:
        if not math.isfinite(raw):
            raise ValueError("dimension power must be finite")
        frac = Fraction(str(raw)).limit_denominator(10_000)
        return sp.Rational(frac.numerator, frac.denominator)
    if isinstance(raw, str):
        s = raw.strip()
        if not s:
            raise ValueError("empty dimension power")
        frac = Fraction(s)
        if abs(frac.numerator) > 1000 or frac.denominator > 1000:
            raise ValueError(f"dimension power {raw!r} exceeds cap")
        return sp.Rational(frac.numerator, frac.denominator)
    raise ValueError(f"unsupported dimension power {raw!r}")


def parse_dimension_table(raw: Any) -> dict[str, dict[str, sp.Rational]]:
    if not isinstance(raw, dict):
        raise ValueError("setup `dimensions` must be a mapping")
    table: dict[str, dict[str, sp.Rational]] = {}
    for sym_name, bases in raw.items():
        clean_sym = validate_symbol_name(sym_name, "dimension symbol")
        if not isinstance(bases, dict):
            raise ValueError(f"dimensions for {clean_sym!r} must be a dict")
        cleaned: dict[str, sp.Rational] = {}
        for base, power in bases.items():
            if not isinstance(base, str) or not base.strip():
                raise ValueError(f"invalid base unit name for {clean_sym!r}")
            exp = parse_dimension_power(power)
            if exp != 0:
                cleaned[base.strip()] = exp
        table[clean_sym] = cleaned
    return table


def format_dim_dict(powers: dict[str, sp.Rational]) -> dict[str, str]:
    return {k: str(v) for k, v in sorted(powers.items()) if v != 0}


def format_dim_str(powers: dict[str, sp.Rational]) -> str:
    nonzero = [(k, v) for k, v in sorted(powers.items()) if v != 0]
    if not nonzero:
        return "dimensionless"
    return " * ".join(f"{k}^{v}" if v != 1 else k for k, v in nonzero)


def _merge_dims(a: dict[str, sp.Rational], b: dict[str, sp.Rational], sign: int = 1) -> dict[str, sp.Rational]:
    out = dict(a)
    for base, pow_val in b.items():
        val = out.get(base, sp.Rational(0)) + sign * pow_val
        if val == 0:
            out.pop(base, None)
        else:
            out[base] = val
    return out


def expr_dimension(
    expr: Any,
    table: dict[str, dict[str, sp.Rational]],
    bound: dict[str, dict[str, sp.Rational]] | None = None,
) -> dict[str, sp.Rational]:
    """Compute exact rational dimensions of a SymPy expression, or raise Unknown/Mismatch."""
    bound_map = bound or {}
    if isinstance(expr, sp.Symbol):
        if expr.name in bound_map:
            return dict(bound_map[expr.name])
        if expr.name in table:
            return dict(table[expr.name])
        raise UnknownDimensionError(expr.name)

    if isinstance(expr, (sp.Number, sp.NumberSymbol)) or expr is sp.I:
        if expr in (sp.oo, -sp.oo, sp.zoo, sp.nan):
            raise UnknownDimensionError(f"{expr} has no finite dimension")
        return {}

    if isinstance(expr, sp.Mul):
        total: dict[str, sp.Rational] = {}
        for term in expr.args:
            total = _merge_dims(total, expr_dimension(term, table, bound_map), 1)
        return total

    if isinstance(expr, sp.Add):
        parts: list[dict[str, sp.Rational]] = []
        unknown_exc: UnknownDimensionError | None = None
        for term in expr.args:
            if term == 0:
                continue
            try:
                parts.append(expr_dimension(term, table, bound_map))
            except UnknownDimensionError as exc:
                if unknown_exc is None:
                    unknown_exc = exc
        if len(parts) >= 2 and any(p != parts[0] for p in parts[1:]):
            raise DimensionMismatchError(
                f"incompatible dimensions in sum `{expr}`: "
                + ", ".join(format_dim_str(p) for p in parts)
            )
        if unknown_exc is not None:
            raise unknown_exc
        return parts[0] if parts else {}

    if isinstance(expr, sp.Pow):
        base, exp = expr.args
        exp_dim = expr_dimension(exp, table, bound_map)
        if exp_dim:
            raise DimensionMismatchError(
                f"exponent `{exp}` must be dimensionless, got {format_dim_str(exp_dim)}"
            )
        base_dim = expr_dimension(base, table, bound_map)
        if not base_dim:
            return {}
        exp_rat = sp.simplify(exp)
        if not isinstance(exp_rat, sp.Rational):
            raise UnknownDimensionError(f"dimensional base `{base}` requires constant rational exponent")
        return {k: p * exp_rat for k, p in base_dim.items() if p * exp_rat != 0}

    if expr.func in (sp.Abs, sp.conjugate, sp.re, sp.im):
        return expr_dimension(expr.args[0], table, bound_map)

    if expr.func in (sp.Heaviside, sp.sign):
        _ = expr_dimension(expr.args[0], table, bound_map)
        return {}

    if expr.func is sp.DiracDelta:
        arg_dim = expr_dimension(expr.args[0], table, bound_map)
        return {k: -v for k, v in arg_dim.items() if v != 0}

    if expr.func in (sp.Min, sp.Max):
        parts = [expr_dimension(a, table, bound_map) for a in expr.args if a != 0]
        if parts and any(p != parts[0] for p in parts[1:]):
            raise DimensionMismatchError(f"incompatible dimensions in `{expr}`")
        return parts[0] if parts else {}

    if isinstance(expr, sp.Derivative):
        dim = expr_dimension(expr.expr, table, bound_map)
        for var, count in expr.variable_count:
            cnt_dim = expr_dimension(count, table, bound_map)
            if cnt_dim:
                raise DimensionMismatchError("derivative order must be dimensionless")
            cnt_val = sp.simplify(count)
            if not isinstance(cnt_val, sp.Rational):
                raise UnknownDimensionError("derivative order must be a rational constant")
            v_dim = expr_dimension(var, table, bound_map)
            dim = _merge_dims(dim, {k: p * cnt_val for k, p in v_dim.items()}, -1)
        return dim

    if isinstance(expr, sp.Integral):
        dim = expr_dimension(expr.function, table, bound_map)
        for lim in expr.limits:
            var = lim[0]
            v_dim = expr_dimension(var, table, bound_map)
            if len(lim) == 3:
                for bnd in (lim[1], lim[2]):
                    if bnd in (sp.oo, -sp.oo, sp.Integer(0)):
                        continue
                    b_dim = expr_dimension(bnd, table, bound_map)
                    if b_dim != v_dim:
                        raise DimensionMismatchError(
                            f"integral bound `{bnd}` ({format_dim_str(b_dim)}) "
                            f"does not match `{var}` ({format_dim_str(v_dim)})"
                        )
            dim = _merge_dims(dim, v_dim, 1)
        return dim

    if isinstance(expr, sp.Sum):
        local_bound = dict(bound_map)
        for lim in expr.limits:
            var = lim[0]
            if isinstance(var, sp.Symbol) and var.name not in table:
                local_bound[var.name] = {}
            v_dim = expr_dimension(var, table, local_bound)
            if len(lim) == 3:
                for bnd in (lim[1], lim[2]):
                    if bnd in (sp.oo, -sp.oo, sp.Integer(0)):
                        continue
                    if expr_dimension(bnd, table, local_bound) != v_dim:
                        raise DimensionMismatchError(f"sum bound `{bnd}` dimension mismatch")
        return expr_dimension(expr.function, table, local_bound)

    if isinstance(expr, sp.Order):
        return expr_dimension(expr.expr, table, bound_map)

    if isinstance(expr, sp.Limit):
        var, pt = expr.args[1], expr.args[2]
        v_dim = expr_dimension(var, table, bound_map)
        if pt not in (sp.oo, -sp.oo, sp.Integer(0)):
            p_dim = expr_dimension(pt, table, bound_map)
            if p_dim != v_dim:
                raise DimensionMismatchError(f"limit point `{pt}` dimension mismatch")
        return expr_dimension(expr.args[0], table, bound_map)

    if isinstance(expr, sp.Subs):
        for var, val in zip(expr.variables, expr.point):
            if val != 0 and expr_dimension(val, table, bound_map) != expr_dimension(var, table, bound_map):
                raise DimensionMismatchError(f"substitution `{var} = {val}` dimension mismatch")
        return expr_dimension(expr.expr, table, bound_map)

    if isinstance(expr, sp.Piecewise):
        parts = [expr_dimension(val, table, bound_map) for val, _cond in expr.args if val != 0]
        if parts and any(p != parts[0] for p in parts[1:]):
            raise DimensionMismatchError(f"incompatible branch dimensions in `{expr}`")
        return parts[0] if parts else {}

    if expr.func in _TRANSCENDENTAL_FUNCS:
        for arg in expr.args:
            ad = expr_dimension(arg, table, bound_map)
            if ad:
                raise DimensionMismatchError(
                    f"`{expr.func.__name__}` requires dimensionless arguments, got {format_dim_str(ad)}"
                )
        return {}

    raise UnknownDimensionError(f"dimension rule unavailable for `{type(expr).__name__}`")


def sample_parameter_points(
    free_syms: list[sp.Symbol], params: dict[str, Any], count: int
) -> list[dict[sp.Basic, sp.Expr]]:
    """Deterministic parameter sample points respecting symbol assumptions and domain."""
    if not free_syms:
        return [{}]
    bounds = _parse_domain_constraints(params.get("domain"), params)
    seed = int(params.get("seed", 0) or 0)
    cand_lists = [_candidates_for_symbol(s, bounds.get(s.name), count + 8, seed) for s in free_syms]
    total_slots = max(count + 8, max(len(c) for c in cand_lists))
    out: list[dict[sp.Basic, sp.Expr]] = []
    for idx in range(total_slots):
        out.append({
            sym: cands[(idx * (pos + 1) + idx) % len(cands)] if idx >= len(cands) else cands[idx]
            for pos, (sym, cands) in enumerate(zip(free_syms, cand_lists))
        })
    return out


def eval_scalar_residual_points(
    lhs: sp.Basic,
    rhs: sp.Basic,
    assumed: dict[str, sp.Symbol],
    params: dict[str, Any],
    *,
    num_points: int = DEFAULT_NUMERIC_POINTS,
    digits: int = DEFAULT_NUMERIC_DIGITS,
) -> dict[str, Any]:
    """Evaluate `lhs - rhs` at deterministic domain samples using `mpmath`."""
    tol_raw = params.get("tolerance")
    work_dps = digits + 20
    old_dps = mp.mp.dps
    mp.mp.dps = work_dps
    try:
        tol = mp.mpf(str(tol_raw)) if tol_raw is not None else mp.power(10, -(digits - 12))
        free = sorted(
            {s for s in (lhs.free_symbols | rhs.free_symbols) if hasattr(s, "name")},
            key=lambda s: s.name,
        )
        points = sample_parameter_points(free, params, num_points)
        valid = 0
        samples: list[dict[str, str]] = []
        max_err = mp.mpf("0")
        for subs in points:
            lv = eval_mp_scalar(lhs, subs, work_dps)
            rv = eval_mp_scalar(rhs, subs, work_dps)
            if lv is None or rv is None:
                continue
            valid += 1
            err = abs(lv - rv)
            scale = max(mp.mpf("1"), abs(lv), abs(rv))
            pt_str = ", ".join(f"{s.name}={v}" for s, v in subs.items()) or "constant"
            if err > tol * scale:
                return {
                    "status": "fail",
                    "valid": valid,
                    "tol": mp.nstr(tol, 4),
                    "counterexample": {
                        "point": pt_str,
                        "lhs": str(lv),
                        "rhs": str(rv),
                        "residual": str(lv - rv),
                    },
                    "samples": samples,
                    "assumptions": assumptions_snapshot(assumed),
                }
            max_err = max(max_err, err)
            if len(samples) < 5:
                samples.append({"point": pt_str, "residual": mp.nstr(err, 6)})
            if not free or valid >= num_points:
                break
        needed = 1 if not free else min(num_points, max(1, len(points)))
        return {
            "status": "pass" if valid >= needed else "inconclusive",
            "valid": valid,
            "max_err": mp.nstr(max_err, 6),
            "tol": mp.nstr(tol, 4),
            "samples": samples,
            "assumptions": assumptions_snapshot(assumed),
        }
    finally:
        mp.mp.dps = old_dps
