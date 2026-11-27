"""Host-shipped clean-room physics checks for the Solve check process (package S10).

Six checks (`solve.md` 4.4, 5.1; rule R8):
`dimensions`, `limit_case`, `symmetry`, `conservation`, `convergence`, and `matrix_property`.

Passing any S10 check adds its name as a sublabel ONLY and NEVER grants `Verified` or
`Numerically checked`; failing flips the step permanently to `Failed` (R8). Every input
expression enters through `srepr_safe.parse_srepr_safe` — zero `eval`, `exec`, or string `sympify`.
"""

from __future__ import annotations

from typing import Any

import mpmath as mp
import sympy as sp

from .algebra_probe import assumptions_snapshot, eval_mp_scalar
from .physics_helpers import (
    DimensionMismatchError,
    UnknownDimensionError,
    assumption_mismatch_result,
    expr_dimension,
    format_dim_dict,
    format_dim_str,
    normalize_physics_handles,
    parse_dimension_table,
    parse_limit_point,
    parse_safe_expr,
    sample_parameter_points,
    validate_symbol_name,
)
from .physics_matrix import (
    check_matrix_zero,
    check_scalar_zero,
    convergence,
    matrix_property,
    require_lhs_rhs,
)


def dimensions(handles: dict[str, Any], params: dict[str, Any]) -> dict[str, Any]:
    """Dimensional analysis over `setup.dimensions` (`{symbol: {base: power}}`)."""
    require_lhs_rhs(handles)
    norm, _assumed, mismatch = normalize_physics_handles(handles, params)
    if mismatch is not None or norm is None:
        return assumption_mismatch_result(mismatch)

    raw_dims = params.get("dimensions")
    if not raw_dims:
        return {"outcome": "inconclusive", "evidence": {"reason": "no dimensions declared in setup"}}
    table = parse_dimension_table(raw_dims)
    lhs, rhs = norm["lhs"], norm["rhs"]

    if isinstance(lhs, sp.MatrixBase) or isinstance(rhs, sp.MatrixBase):
        if not (isinstance(lhs, sp.MatrixBase) and isinstance(rhs, sp.MatrixBase)) or lhs.shape != rhs.shape:
            return {"outcome": "fail", "evidence": {"reason": "matrix shape mismatch in dimensions check"}}
        try:
            for i in range(lhs.rows):
                for j in range(lhs.cols):
                    le, re_ = lhs[i, j], rhs[i, j]
                    ld = expr_dimension(le, table)
                    rd = expr_dimension(re_, table)
                    if le == 0:
                        ld = rd
                    elif re_ == 0:
                        rd = ld
                    if ld != rd:
                        return {
                            "outcome": "fail",
                            "evidence": {
                                "row": i, "col": j,
                                "lhs_dimensions": format_dim_dict(ld),
                                "rhs_dimensions": format_dim_dict(rd),
                                "lhs_dim_str": format_dim_str(ld),
                                "rhs_dim_str": format_dim_str(rd),
                                "reason": f"dimension mismatch at ({i}, {j}): {format_dim_str(ld)} != {format_dim_str(rd)}",
                            },
                        }
        except DimensionMismatchError as exc:
            return {"outcome": "fail", "evidence": {"reason": str(exc)}}
        except UnknownDimensionError as exc:
            return {"outcome": "inconclusive", "evidence": {"reason": f"missing symbol dimension: {exc}"}}
        return {"outcome": "pass", "evidence": {"shape": [lhs.rows, lhs.cols], "status": "match"}}

    try:
        ld = expr_dimension(lhs, table)
        rd = expr_dimension(rhs, table)
    except DimensionMismatchError as exc:
        return {"outcome": "fail", "evidence": {"reason": str(exc)}}
    except UnknownDimensionError as exc:
        return {"outcome": "inconclusive", "evidence": {"missing_symbol": str(exc), "reason": f"missing symbol dimension: {exc}"}}

    if lhs == 0 or sp.simplify(lhs) == 0:
        ld = rd
    elif rhs == 0 or sp.simplify(rhs) == 0:
        rd = ld

    ev = {
        "lhs_dimensions": format_dim_dict(ld),
        "rhs_dimensions": format_dim_dict(rd),
        "lhs_dim_str": format_dim_str(ld),
        "rhs_dim_str": format_dim_str(rd),
        "dimensionless": len(ld) == 0 and len(rd) == 0,
    }
    if ld != rd:
        ev["reason"] = f"dimension mismatch: lhs is {ev['lhs_dim_str']}, rhs is {ev['rhs_dim_str']}"
        return {"outcome": "fail", "evidence": ev}
    return {"outcome": "pass", "evidence": ev}


def _compute_sympy_limit(expr: sp.Basic, var_sym: sp.Symbol, point: sp.Expr, eff_dir: str) -> sp.Expr:
    res = sp.limit(expr, var_sym, point, dir=eff_dir)
    if res.has(sp.Limit) and expr.has(sp.Integral, sp.Sum, sp.Derivative):
        res = sp.limit(expr.doit(), var_sym, point, dir=eff_dir)
    return res


def _numeric_limit_counterexample(
    expr_a: sp.Basic,
    expr_b: sp.Basic,
    var_sym: sp.Symbol,
    point: sp.Expr,
    eff_dir: str,
    params: dict[str, Any],
) -> dict[str, Any] | None:
    """Detect persistent disagreement approaching `point` along `eff_dir` when symbolic limit is unreduced."""
    rem_syms = sorted(
        {s for s in (expr_a.free_symbols | expr_b.free_symbols) if hasattr(s, "name") and s != var_sym},
        key=lambda s: s.name,
    )
    base_subs = sample_parameter_points(rem_syms, params, 2)[0] if rem_syms else {}
    point_sub = point.subs(base_subs) if rem_syms else point
    if point == sp.oo:
        probes = [sp.Integer(10**k) for k in (3, 5, 7, 9)]
    elif point == -sp.oo:
        probes = [-sp.Integer(10**k) for k in (3, 5, 7, 9)]
    elif not point_sub.free_symbols and not point_sub.has(sp.oo, -sp.oo, sp.zoo, sp.nan):
        deltas = [sp.Rational(1, 10**k) for k in (3, 5, 7, 9)]
        probes = (
            [point_sub + d for d in deltas] if eff_dir == "+"
            else [point_sub - d for d in deltas] if eff_dir == "-"
            else [point_sub + d for d in deltas]
        )
    else:
        return None

    old_dps = mp.mp.dps
    mp.mp.dps = 50
    try:
        errs: list[tuple[sp.Expr, mp.mpf, Any, Any]] = []
        for pv in probes:
            sub = dict(base_subs)
            sub[var_sym] = pv
            va = eval_mp_scalar(expr_a, sub, 50)
            vb = eval_mp_scalar(expr_b, sub, 50)
            if va is None or vb is None:
                continue
            errs.append((pv, abs(va - vb) / max(mp.mpf("1"), abs(va), abs(vb)), va, vb))
        if len(errs) >= 2:
            tail_prev, tail_curr = errs[-2], errs[-1]
            if tail_curr[1] > mp.mpf("1e-3") and tail_prev[1] <= tail_curr[1] * mp.mpf("2"):
                return {
                    "point": f"{var_sym.name}={tail_curr[0]}",
                    "lhs": str(tail_curr[2]),
                    "rhs": str(tail_curr[3]),
                    "relative_residual": mp.nstr(tail_curr[1], 6),
                }
    finally:
        mp.mp.dps = old_dps
    return None


def limit_case(handles: dict[str, Any], params: dict[str, Any]) -> dict[str, Any]:
    """Symbolic and numeric limit check `var -> point` (`lhs` vs `rhs` or `target` handle)."""
    require_lhs_rhs(handles)
    norm, assumed, mismatch = normalize_physics_handles(handles, params)
    if mismatch is not None or norm is None:
        return assumption_mismatch_result(mismatch)

    var_raw = params.get("variable") or params.get("var")
    var_name = validate_symbol_name(var_raw, "variable")
    var_sym = assumed.get(var_name, sp.Symbol(var_name))

    point_raw = params.get("point") if params.get("point") is not None else params.get("to")
    if point_raw is None:
        point_raw = params.get("target_point")
    if point_raw is None:
        raise ValueError("limit_case requires `point`")
    point_expr, point_str = parse_limit_point(point_raw, assumed)

    direction = str(params.get("direction", "+-")).strip()
    if direction not in ("+", "-", "+-"):
        raise ValueError(f"invalid limit direction {direction!r}")
    eff_dir = "-" if point_expr == sp.oo else ("+" if point_expr == -sp.oo else direction)

    if "target" in norm:
        side = str(params.get("side", "lhs")).strip()
        if side not in ("lhs", "rhs"):
            raise ValueError("limit_case `side` must be 'lhs' or 'rhs'")
        expr_a, expr_b = norm[side], norm["target"]
    else:
        expr_a, expr_b = norm["lhs"], norm["rhs"]

    meta = {"variable": var_name, "point": point_str, "direction": eff_dir}
    try:
        lim_a = _compute_sympy_limit(expr_a, var_sym, point_expr, eff_dir)
        lim_b = _compute_sympy_limit(expr_b, var_sym, point_expr, eff_dir)
    except Exception as exc:
        ce = _numeric_limit_counterexample(expr_a, expr_b, var_sym, point_expr, eff_dir, params)
        if ce is not None:
            return {"outcome": "fail", "evidence": {**meta, "counterexample": ce, "reason": "numeric limit disagreement"}}
        return {"outcome": "inconclusive", "evidence": {**meta, "reason": f"limit evaluation incomplete: {exc}"}}

    if lim_a.has(sp.Limit) or lim_b.has(sp.Limit):
        ce = _numeric_limit_counterexample(expr_a, expr_b, var_sym, point_expr, eff_dir, params)
        if ce is not None:
            return {"outcome": "fail", "evidence": {**meta, "counterexample": ce, "reason": "numeric limit disagreement"}}
        return {"outcome": "inconclusive", "evidence": {**meta, "reason": "symbolic limit did not reduce"}}

    inf_set = (sp.oo, -sp.oo)
    if lim_a in inf_set or lim_b in inf_set:
        if lim_a == lim_b:
            return {"outcome": "pass", "evidence": {**meta, "limit_lhs": str(lim_a), "limit_rhs": str(lim_b)}}
        return {
            "outcome": "fail",
            "evidence": {**meta, "limit_lhs": str(lim_a), "limit_rhs": str(lim_b), "reason": "infinite vs finite/opposite limit mismatch"},
        }
    if lim_a.has(sp.zoo, sp.nan) or lim_b.has(sp.zoo, sp.nan):
        return {"outcome": "inconclusive", "evidence": {**meta, "limit_lhs": str(lim_a), "limit_rhs": str(lim_b), "reason": "indeterminate or complex-infinite limit"}}

    res = check_scalar_zero(lim_a, lim_b, assumed, params, label="limit_case")
    res["evidence"].update({**meta, "limit_lhs": str(lim_a), "limit_rhs": str(lim_b)})
    return res


def symmetry(handles: dict[str, Any], params: dict[str, Any]) -> dict[str, Any]:
    """Check invariance or covariance under simultaneous substitution `{var: expr}`."""
    require_lhs_rhs(handles)
    norm, assumed, mismatch = normalize_physics_handles(handles, params)
    if mismatch is not None or norm is None:
        return assumption_mismatch_result(mismatch)

    raw_subs = params.get("subs") or params.get("transform") or params.get("map")
    kind = str(params.get("kind", "")).strip()
    sign_val = params.get("sign")
    factor_raw = params.get("factor")

    subs_pairs: list[tuple[sp.Symbol, sp.Expr]] = []
    if isinstance(raw_subs, dict) and raw_subs:
        for k, v in raw_subs.items():
            name = validate_symbol_name(k, "substitution variable")
            sym = assumed.get(name, sp.Symbol(name))
            subs_pairs.append((sym, parse_safe_expr(v, assumed)))
    elif kind in ("even", "odd"):
        vname = validate_symbol_name(params.get("variable"), "variable")
        sym = assumed.get(vname, sp.Symbol(vname))
        subs_pairs.append((sym, -sym))
        if sign_val is None and factor_raw is None:
            sign_val = 1 if kind == "even" else -1
    elif kind == "exchange":
        raw_vars = params.get("variables")
        if not isinstance(raw_vars, list) or len(raw_vars) != 2:
            raise ValueError("symmetry kind 'exchange' requires two `variables`")
        n0, n1 = validate_symbol_name(raw_vars[0]), validate_symbol_name(raw_vars[1])
        s0, s1 = assumed.get(n0, sp.Symbol(n0)), assumed.get(n1, sp.Symbol(n1))
        subs_pairs = [(s0, s1), (s1, s0)]
    else:
        raise ValueError("symmetry requires non-empty `subs` mapping or `kind`")

    if factor_raw is not None:
        factor = parse_safe_expr(factor_raw, assumed)
    elif sign_val is not None:
        if sign_val not in (1, -1, "+1", "-1"):
            raise ValueError("symmetry `sign` must be +1 or -1")
        factor = sp.Integer(int(sign_val))
    else:
        factor = sp.Integer(1)

    lhs, rhs = norm["lhs"], norm["rhs"]
    side = params.get("side")
    if side is not None:
        side = str(side).strip()
        if side not in ("lhs", "rhs", "both"):
            raise ValueError("symmetry `side` must be 'lhs', 'rhs', or 'both'")
        targets = [("lhs", lhs)] if side == "lhs" else ([("rhs", rhs)] if side == "rhs" else [("lhs", lhs), ("rhs", rhs)])
    else:
        sub_syms = {s for s, _ in subs_pairs}
        relevant = [(k, ex) for k, ex in (("lhs", lhs), ("rhs", rhs)) if isinstance(ex, sp.Basic) and (ex.free_symbols & sub_syms)]
        if len(relevant) == 2 and lhs == rhs:
            relevant = [relevant[0]]
        targets = relevant if relevant else [("lhs", lhs)]

    checked_sides: list[str] = []
    for label, expr in targets:
        transformed = expr.subs(subs_pairs, simultaneous=True)
        expected = factor * expr
        if isinstance(expr, sp.MatrixBase):
            res = check_matrix_zero(transformed - expected, assumed, params, label=f"symmetry({label})")
        else:
            res = check_scalar_zero(transformed, expected, assumed, params, label=f"symmetry({label})")
        checked_sides.append(label)
        if res["outcome"] != "pass":
            res["evidence"]["checked_side"] = label
            res["evidence"]["factor"] = str(factor)
            return res

    return {
        "outcome": "pass",
        "evidence": {
            "checked_sides": checked_sides,
            "factor": str(factor),
            "subs": {s.name: str(v) for s, v in subs_pairs},
            "assumptions": assumptions_snapshot(assumed),
        },
    }


def conservation(handles: dict[str, Any], params: dict[str, Any]) -> dict[str, Any]:
    """Check vanishing time derivative, commutator `[H, Q]=0`, continuity, or Poisson bracket."""
    require_lhs_rhs(handles)
    norm, assumed, mismatch = normalize_physics_handles(handles, params)
    if mismatch is not None or norm is None:
        return assumption_mismatch_result(mismatch)

    lhs, rhs = norm["lhs"], norm["rhs"]
    raw_subs = params.get("subs")
    if isinstance(raw_subs, dict) and raw_subs:
        sub_pairs = [(assumed.get(validate_symbol_name(k), sp.Symbol(validate_symbol_name(k))), parse_safe_expr(v, assumed)) for k, v in raw_subs.items()]
        lhs = lhs.subs(sub_pairs, simultaneous=True)
        rhs = rhs.subs(sub_pairs, simultaneous=True)

    mode = str(params.get("mode", "")).strip()
    if not mode:
        if params.get("space_vars") is not None:
            mode = "continuity"
        elif params.get("q_vars") is not None and params.get("p_vars") is not None:
            mode = "poisson_bracket"
        elif isinstance(lhs, sp.MatrixBase) and isinstance(rhs, sp.MatrixBase):
            mode = "commutator"
        elif params.get("variable") or params.get("time_var"):
            mode = "derivative"
        else:
            mode = "residual"

    if mode == "commutator":
        if isinstance(lhs, sp.MatrixBase) or isinstance(rhs, sp.MatrixBase):
            if not (isinstance(lhs, sp.MatrixBase) and isinstance(rhs, sp.MatrixBase)) or lhs.shape != rhs.shape or lhs.rows != lhs.cols:
                return {"outcome": "fail", "evidence": {"mode": mode, "reason": "commutator requires square matrices of equal shape"}}
            res = check_matrix_zero(lhs * rhs - rhs * lhs, assumed, params, label="commutator")
        else:
            res = check_scalar_zero(lhs * rhs, rhs * lhs, assumed, params, label="commutator")
        res["evidence"]["mode"] = mode
        return res

    if mode in ("derivative", "time_derivative"):
        vname = validate_symbol_name(params.get("variable") or params.get("time_var") or "t", "variable")
        vsym = assumed.get(vname, sp.Symbol(vname))
        side = str(params.get("side", "lhs")).strip()
        target = norm["rhs"] if (params.get("side") is None and vsym not in lhs.free_symbols and vsym in rhs.free_symbols) else (lhs if side == "lhs" else rhs)
        deriv = sp.diff(target, vsym)
        if not isinstance(deriv, sp.MatrixBase) and deriv.has(sp.Derivative, sp.Integral, sp.Sum):
            deriv = deriv.doit()
        res = check_matrix_zero(deriv, assumed, params, label=f"d/d{vname}") if isinstance(deriv, sp.MatrixBase) else check_scalar_zero(deriv, sp.Integer(0), assumed, params, label=f"d/d{vname}")
        res["evidence"].update({"mode": mode, "variable": vname})
        return res

    if mode in ("continuity", "divergence"):
        tname = validate_symbol_name(params.get("time_var") or params.get("variable") or "t", "time_var")
        tsym = assumed.get(tname, sp.Symbol(tname))
        space_raw = params.get("space_vars")
        if not isinstance(space_raw, list) or not space_raw:
            raise ValueError("continuity check requires non-empty `space_vars`")
        space_syms = [assumed.get(validate_symbol_name(s, "space_var"), sp.Symbol(validate_symbol_name(s, "space_var"))) for s in space_raw]
        flux_terms = list(rhs) if isinstance(rhs, sp.MatrixBase) else [rhs]
        if len(flux_terms) != len(space_syms):
            return {"outcome": "fail", "evidence": {"mode": mode, "reason": f"flux dimension {len(flux_terms)} != space_vars {len(space_syms)}"}}
        residual = sp.diff(lhs, tsym) + sum(sp.diff(j, x) for j, x in zip(flux_terms, space_syms))
        res = check_scalar_zero(residual, sp.Integer(0), assumed, params, label="continuity")
        res["evidence"].update({"mode": mode, "time_var": tname, "space_vars": [s.name for s in space_syms]})
        return res

    if mode == "poisson_bracket":
        q_raw, p_raw = params.get("q_vars"), params.get("p_vars")
        if not isinstance(q_raw, list) or not isinstance(p_raw, list) or not q_raw or len(q_raw) != len(p_raw):
            raise ValueError("poisson_bracket requires equal-length non-empty `q_vars` and `p_vars`")
        q_syms = [assumed.get(validate_symbol_name(q, "q_var"), sp.Symbol(validate_symbol_name(q, "q_var"))) for q in q_raw]
        p_syms = [assumed.get(validate_symbol_name(p, "p_var"), sp.Symbol(validate_symbol_name(p, "p_var"))) for p in p_raw]
        pb = sum(sp.diff(lhs, q) * sp.diff(rhs, p) - sp.diff(lhs, p) * sp.diff(rhs, q) for q, p in zip(q_syms, p_syms))
        res = check_scalar_zero(pb, sp.Integer(0), assumed, params, label="poisson_bracket")
        res["evidence"]["mode"] = mode
        return res

    if mode == "residual":
        res = check_scalar_zero(lhs, rhs, assumed, params, label="conservation_residual")
        res["evidence"]["mode"] = mode
        return res

    raise ValueError(f"unsupported conservation mode {mode!r}")


PHYSICS_CHECKS = {
    "dimensions": dimensions,
    "limit_case": limit_case,
    "symmetry": symmetry,
    "conservation": conservation,
    "convergence": convergence,
    "matrix_property": matrix_property,
}

__all__ = ["PHYSICS_CHECKS", *PHYSICS_CHECKS.keys()]
