"""Host-shipped algebra checks for the Solve check process (package S9, solve.md 4.4 & 5.1).

Five checks: `identity_symbolic`, `identity_numeric`, `series_match`,
`vanishes_to_order`, and `counterexample_search`.

Every input expression arrives already reconstructed through `srepr_safe.parse_srepr_safe`.
This module never calls `eval`, `exec`, or string `sympify`.
"""

from __future__ import annotations

import re
from typing import Any

import mpmath as mp
import sympy as sp

from .algebra_probe import (
    DEFAULT_COUNTEREXAMPLE_POINTS,
    DEFAULT_NUMERIC_DIGITS,
    DEFAULT_NUMERIC_POINTS,
    MAX_DIGITS,
    MAX_POINTS,
    MAX_SERIES_ORDER,
    assumptions_snapshot,
    eval_mp_scalar,
    is_exact_zero,
    normalize_symbols,
    numeric_probe,
    parse_simple_real,
)


def _require_lhs_rhs(handles: dict[str, Any]) -> tuple[sp.Basic, sp.Basic]:
    if "lhs" not in handles or "rhs" not in handles:
        raise ValueError("check requires both 'lhs' and 'rhs' handles")
    return handles["lhs"], handles["rhs"]


def _identical_sides_result() -> dict[str, Any]:
    return {
        "outcome": "trivial",
        "evidence": {
            "identical_sides": True,
            "reason": "both sides are structurally identical before simplification; nothing was checked",
        },
    }


def _unwrap_unevaluated(expr: Any) -> Any:
    if not isinstance(expr, (sp.Basic, sp.MatrixBase)):
        return expr
    res = expr
    for _ in range(8):
        if not res.has(sp.UnevaluatedExpr):
            break
        res = res.replace(
            lambda e: isinstance(e, sp.UnevaluatedExpr),
            lambda e: (
                e.args[0].doit()
                if isinstance(e.args[0], (sp.Basic, sp.MatrixBase))
                and not e.args[0].has(sp.Integral, sp.Sum, sp.Derivative, sp.Limit, sp.MatMul, sp.Trace)
                else e.args[0]
            ),
        )
    if isinstance(res, sp.MatrixExpr) and res.has(sp.MatMul, sp.MatAdd, sp.MatPow):
        evaluated = res.doit()
        if isinstance(evaluated, sp.MatrixBase):
            res = evaluated
    elif isinstance(res, sp.Basic) and res.has(sp.MatMul, sp.Trace) and not res.has(sp.MatrixSymbol):
        res = res.replace(
            lambda e: isinstance(e, (sp.MatMul, sp.Trace)),
            lambda e: e.doit(),
        )
    return res


def identity_symbolic(handles: dict[str, Any], params: dict[str, Any]) -> dict[str, Any]:
    """Exact symbolic identity check with symbol-assumption check and Guard (b) numeric check."""
    lhs_raw, rhs_raw = _require_lhs_rhs(handles)
    norm, assumed, mismatch = normalize_symbols(handles, params)
    if mismatch is not None or norm is None:
        return {
            "outcome": "inconclusive",
            "evidence": {
                "comparison": "exact",
                "exact": False,
                "reason": mismatch or "inconclusive: symbol assumptions differ",
            },
        }
    if sp.srepr(lhs_raw) == sp.srepr(rhs_raw):
        return _identical_sides_result()
    lhs = _unwrap_unevaluated(norm["lhs"])
    rhs = _unwrap_unevaluated(norm["rhs"])
    assump_info = assumptions_snapshot(assumed)

    try:
        diff = sp.simplify(lhs - rhs)
        if not is_exact_zero(diff) and isinstance(diff, (sp.Basic, sp.MatrixBase)) and diff.has(
            sp.Integral, sp.Sum, sp.Derivative, sp.Limit, sp.MatMul, sp.Trace
        ):
            diff = sp.simplify((lhs - rhs).doit())
    except (sp.ShapeError, TypeError) as exc:
        return {
            "outcome": "inconclusive",
            "evidence": {
                "comparison": "exact",
                "exact": False,
                "assumptions": assump_info,
                "reason": f"inconclusive: incompatible expression shapes or types ({exc})",
            },
        }

    guard_probe = numeric_probe(
        lhs,
        rhs,
        params,
        num_points=max(
            DEFAULT_NUMERIC_POINTS,
            int(params.get("points", DEFAULT_NUMERIC_POINTS) or DEFAULT_NUMERIC_POINTS),
        ),
        digits=max(
            DEFAULT_NUMERIC_DIGITS,
            int(params.get("digits", DEFAULT_NUMERIC_DIGITS) or DEFAULT_NUMERIC_DIGITS),
        ),
        relation="eq",
    )
    diff_is_zero = is_exact_zero(diff)

    if guard_probe["status"] == "fail":
        reason = (
            "symbolic simplification disagreed with numeric evaluation "
            "(suspected branch-cut or domain error)"
            if diff_is_zero
            else "expression mismatch confirmed by numeric counterexample"
        )
        return {
            "outcome": "fail",
            "evidence": {
                "comparison": "exact",
                "exact": False,
                "guard_failed": diff_is_zero,
                "counterexample": guard_probe["counterexample"],
                "residual_srepr": sp.srepr(diff),
                "residual_latex": sp.latex(diff),
                "assumptions": assump_info,
                "reason": reason,
            },
        }

    if diff_is_zero:
        return {
            "outcome": "pass",
            "evidence": {
                "comparison": "exact",
                "exact": True,
                "guard_points": guard_probe["valid"],
                "assumptions": assump_info,
                "residual_srepr": sp.srepr(diff),
                "residual_latex": sp.latex(diff),
            },
        }

    return {
        "outcome": "inconclusive",
        "evidence": {
            "comparison": "exact",
            "exact": False,
            "residual_srepr": sp.srepr(diff),
            "residual_latex": sp.latex(diff),
            "guard_points": guard_probe["valid"],
            "assumptions": assump_info,
            "reason": "unreduced symbolic residual; no numeric counterexample found",
        },
    }


def identity_numeric(handles: dict[str, Any], params: dict[str, Any]) -> dict[str, Any]:
    """50-digit mpmath evaluation over >= 12 domain/assumption points."""
    lhs_raw, rhs_raw = _require_lhs_rhs(handles)
    norm, assumed, mismatch = normalize_symbols(handles, params)
    if mismatch is not None or norm is None:
        return {
            "outcome": "inconclusive",
            "evidence": {"reason": mismatch or "inconclusive: symbol assumptions differ"},
        }
    if sp.srepr(lhs_raw) == sp.srepr(rhs_raw):
        return _identical_sides_result()
    lhs = _unwrap_unevaluated(norm["lhs"])
    rhs = _unwrap_unevaluated(norm["rhs"])
    points = min(
        MAX_POINTS,
        max(
            DEFAULT_NUMERIC_POINTS,
            int(params.get("points", DEFAULT_NUMERIC_POINTS) or DEFAULT_NUMERIC_POINTS),
        ),
    )
    digits = min(
        MAX_DIGITS,
        max(
            DEFAULT_NUMERIC_DIGITS,
            int(params.get("digits", DEFAULT_NUMERIC_DIGITS) or DEFAULT_NUMERIC_DIGITS),
        ),
    )
    free_count = len(lhs.free_symbols | rhs.free_symbols)

    probe = numeric_probe(lhs, rhs, params, num_points=points, digits=digits, relation="eq")
    evidence: dict[str, Any] = {
        "comparison": "numeric",
        "points": points if free_count == 0 and probe["status"] == "pass" else probe["valid"],
        "evaluated_points": probe["valid"],
        "digits": digits,
        "domain_checked": True,
        "tolerance": probe["tol"],
        "max_residual": probe["max_err"] if probe["valid"] > 0 else None,
        "samples": probe["samples"],
        "assumptions": assumptions_snapshot(assumed),
    }
    if "lhs_code_sha256" in params:
        evidence["lhs_code_sha256"] = params["lhs_code_sha256"]
    if "rhs_code_sha256" in params:
        evidence["rhs_code_sha256"] = params["rhs_code_sha256"]

    if probe["status"] == "fail":
        evidence["counterexample"] = probe["counterexample"]
        return {"outcome": "fail", "evidence": evidence}
    if probe["status"] == "pass":
        return {"outcome": "pass", "evidence": evidence}
    evidence["reason"] = probe.get("reason") or "fewer than required finite evaluation points succeeded"
    return {"outcome": "inconclusive", "evidence": evidence}


def _parse_series_order(
    params: dict[str, Any], assumed: dict[str, sp.Symbol]
) -> tuple[sp.Symbol, str, sp.Expr, str, int]:
    order_obj = params.get("order") if isinstance(params.get("order"), dict) else params
    var_name = str(order_obj.get("variable", "")).strip()
    if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]{0,63}", var_name):
        raise ValueError("series check requires a valid `order.variable` symbol name")
    raw_n = order_obj.get("n")
    if isinstance(raw_n, bool) or raw_n is None:
        raise ValueError("series check requires integer `order.n`")
    n = int(raw_n)
    if not 0 <= n <= MAX_SERIES_ORDER:
        raise ValueError(f"`order.n` must be between 0 and {MAX_SERIES_ORDER}")
    point_str = str(order_obj.get("point", "0")).strip()
    point_expr = parse_simple_real(point_str)
    if point_expr is None:
        raise ValueError(f"unsupported series expansion point {point_str!r}")
    var_sym = assumed.get(var_name, sp.Symbol(var_name))
    return var_sym, var_name, point_expr, point_str, n


def _compare_series(
    expr_a: sp.Basic,
    expr_b: sp.Basic,
    assumed: dict[str, sp.Symbol],
    params: dict[str, Any],
    *,
    vanish_mode: bool,
) -> dict[str, Any]:
    var_sym, var_name, point_expr, point_str, n = _parse_series_order(params, assumed)
    order_meta = {"variable": var_name, "point": point_str, "n": n}

    a_unwrapped = _unwrap_unevaluated(expr_a)
    b_unwrapped = _unwrap_unevaluated(expr_b)
    target = (a_unwrapped - b_unwrapped) if vanish_mode else a_unwrapped
    if target.has(sp.Integral, sp.Sum, sp.Derivative):
        target = target.doit()
    rhs_base = sp.Integer(0) if vanish_mode else b_unwrapped
    if rhs_base.has(sp.Integral, sp.Sum, sp.Derivative):
        rhs_base = rhs_base.doit()

    s_a = sp.series(target, var_sym, x0=point_expr, n=n).removeO()
    s_b = sp.Integer(0) if vanish_mode else sp.series(rhs_base, var_sym, x0=point_expr, n=n).removeO()
    has_floats = bool(s_a.has(sp.Float) or s_b.has(sp.Float))

    if has_floats:
        digits = min(MAX_DIGITS, max(20, int(params.get("digits", 30) or 30)))
        tol = mp.power(10, -(digits - 12))
        old_dps = mp.mp.dps
        mp.mp.dps = digits
        try:
            poly_diff = sp.expand(s_a - s_b)
            max_coeff_err = mp.mpf("0")
            param_syms = {s for s in poly_diff.free_symbols if s != var_sym}
            if not param_syms:
                coeffs = poly_diff.as_poly(var_sym).all_coeffs() if poly_diff != 0 else [sp.Integer(0)]
                for c in coeffs:
                    val = eval_mp_scalar(c, {}, digits)
                    if val is None:
                        return {
                            "outcome": "inconclusive",
                            "evidence": {
                                "order": order_meta,
                                "comparison": "numeric",
                                "numeric_coefficients": True,
                                "exact": False,
                            },
                        }
                    max_coeff_err = max(max_coeff_err, abs(val))
                passed = bool(max_coeff_err <= tol)
                return {
                    "outcome": "pass" if passed else "fail",
                    "evidence": {
                        "order": order_meta,
                        "comparison": "numeric",
                        "numeric_coefficients": True,
                        "exact": False,
                        "max_coefficient_residual": mp.nstr(max_coeff_err, 6),
                    },
                }

            probe = numeric_probe(
                s_a, s_b, params, num_points=DEFAULT_NUMERIC_POINTS, digits=digits
            )
            max_res = probe["max_err"] if probe["valid"] > 0 else None
            if probe["status"] == "inconclusive":
                return {
                    "outcome": "inconclusive",
                    "evidence": {
                        "order": order_meta,
                        "comparison": "numeric",
                        "numeric_coefficients": True,
                        "exact": False,
                        "max_coefficient_residual": max_res,
                        "reason": probe.get("reason") or "fewer than required finite evaluation points succeeded",
                    },
                }
            return {
                "outcome": "pass" if probe["status"] == "pass" else "fail",
                "evidence": {
                    "order": order_meta,
                    "comparison": "numeric",
                    "numeric_coefficients": True,
                    "exact": False,
                    "max_coefficient_residual": max_res,
                },
            }
        finally:
            mp.mp.dps = old_dps

    diff = sp.simplify(s_a - s_b)
    if is_exact_zero(diff):
        return {
            "outcome": "pass",
            "evidence": {
                "order": order_meta,
                "comparison": "exact",
                "exact": True,
                "series_lhs_srepr": sp.srepr(s_a),
                "series_rhs_srepr": sp.srepr(s_b),
                "residual_srepr": sp.srepr(diff),
            },
        }

    probe = numeric_probe(
        s_a, s_b, params, num_points=DEFAULT_NUMERIC_POINTS, digits=DEFAULT_NUMERIC_DIGITS
    )
    if probe["status"] == "fail":
        return {
            "outcome": "fail",
            "evidence": {
                "order": order_meta,
                "comparison": "exact",
                "exact": False,
                "counterexample": probe["counterexample"],
                "residual_srepr": sp.srepr(diff),
            },
        }
    return {
        "outcome": "inconclusive",
        "evidence": {
            "order": order_meta,
            "comparison": "exact",
            "exact": False,
            "residual_srepr": sp.srepr(diff),
        },
    }


def series_match(handles: dict[str, Any], params: dict[str, Any]) -> dict[str, Any]:
    """Compare power series expansions of `lhs` and `rhs` up to `order` (`variable`, `point`, `n`)."""
    lhs_raw, rhs_raw = _require_lhs_rhs(handles)
    norm, assumed, mismatch = normalize_symbols(handles, params)
    if mismatch is not None or norm is None:
        return {
            "outcome": "inconclusive",
            "evidence": {"reason": mismatch or "inconclusive: symbol assumptions differ"},
        }
    if sp.srepr(lhs_raw) == sp.srepr(rhs_raw):
        return _identical_sides_result()
    return _compare_series(norm["lhs"], norm["rhs"], assumed, params, vanish_mode=False)


def vanishes_to_order(handles: dict[str, Any], params: dict[str, Any]) -> dict[str, Any]:
    """Verify `lhs - rhs` vanishes to order `n` around `variable = point`."""
    lhs_raw, rhs_raw = _require_lhs_rhs(handles)
    norm, assumed, mismatch = normalize_symbols(handles, params)
    if mismatch is not None or norm is None:
        return {
            "outcome": "inconclusive",
            "evidence": {"reason": mismatch or "inconclusive: symbol assumptions differ"},
        }
    if sp.srepr(lhs_raw) == sp.srepr(rhs_raw):
        return _identical_sides_result()
    return _compare_series(norm["lhs"], norm["rhs"], assumed, params, vanish_mode=True)


def counterexample_search(handles: dict[str, Any], params: dict[str, Any]) -> dict[str, Any]:
    """Sampled inequality / counterexample search (sublabel only, never grants computed tier, R8)."""
    _require_lhs_rhs(handles)
    norm, assumed, mismatch = normalize_symbols(handles, params)
    if mismatch is not None or norm is None:
        return {
            "outcome": "inconclusive",
            "evidence": {"reason": mismatch or "inconclusive: symbol assumptions differ"},
        }
    lhs = _unwrap_unevaluated(norm["lhs"])
    rhs = _unwrap_unevaluated(norm["rhs"])
    relation = str(params.get("relation", "le")).strip()
    if relation not in ("eq", "le", "ge", "approx"):
        raise ValueError(f"unsupported relation `{relation}` in counterexample_search")

    points = min(
        MAX_POINTS,
        max(
            DEFAULT_NUMERIC_POINTS,
            int(params.get("points", DEFAULT_COUNTEREXAMPLE_POINTS) or DEFAULT_COUNTEREXAMPLE_POINTS),
        ),
    )
    digits = min(
        MAX_DIGITS,
        max(
            DEFAULT_NUMERIC_DIGITS,
            int(params.get("digits", DEFAULT_NUMERIC_DIGITS) or DEFAULT_NUMERIC_DIGITS),
        ),
    )
    free_count = len(lhs.free_symbols | rhs.free_symbols)

    probe = numeric_probe(lhs, rhs, params, num_points=points, digits=digits, relation=relation)
    evidence: dict[str, Any] = {
        "relation": relation,
        "points": points if free_count == 0 and probe["status"] == "pass" else probe["valid"],
        "digits": digits,
        "sampled_no_counterexample": probe["status"] == "pass",
        "samples": probe["samples"],
        "assumptions": assumptions_snapshot(assumed),
    }
    if probe["status"] == "fail":
        evidence["counterexample"] = probe["counterexample"]
        return {"outcome": "fail", "evidence": evidence}
    if probe["status"] == "pass":
        return {"outcome": "pass", "evidence": evidence}
    evidence["reason"] = probe.get("reason") or "no finite domain sample could be evaluated"
    return {"outcome": "inconclusive", "evidence": evidence}


ALGEBRA_CHECKS = {
    "identity_symbolic": identity_symbolic,
    "identity_numeric": identity_numeric,
    "series_match": series_match,
    "vanishes_to_order": vanishes_to_order,
    "counterexample_search": counterexample_search,
}

__all__ = ["ALGEBRA_CHECKS", *ALGEBRA_CHECKS.keys()]
