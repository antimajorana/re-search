"""Matrix and convergence checks for `checks.physics` (S10)."""

from __future__ import annotations

import math
from decimal import Decimal
from itertools import islice
from typing import Any

import mpmath as mp
import numpy as np
import sympy as sp
from numerical_validation import check_convergence

from .algebra_probe import (
    DEFAULT_NUMERIC_DIGITS,
    DEFAULT_NUMERIC_POINTS,
    MAX_DIGITS,
    MAX_POINTS,
    assumptions_snapshot,
    eval_mp_scalar,
    numeric_probe,
)
from .physics_helpers import (
    assumption_mismatch_result,
    eval_scalar_residual_points,
    normalize_physics_handles,
    sample_parameter_points,
    validate_symbol_name,
)

_MATRIX_PROPERTIES = frozenset({
    "hermitian", "symmetric", "unitary", "orthogonal",
    "positive_definite", "traceless", "invertible",
})


def require_lhs_rhs(handles: dict[str, Any]) -> tuple[Any, Any]:
    if "lhs" not in handles or "rhs" not in handles:
        raise ValueError("check requires both 'lhs' and 'rhs' handles")
    return handles["lhs"], handles["rhs"]


def check_matrix_zero(
    diff_mat: sp.MatrixBase,
    assumed: dict[str, sp.Symbol],
    params: dict[str, Any],
    *,
    label: str,
) -> dict[str, Any]:
    """Symbolically and numerically verify every entry of `diff_mat` is zero."""
    rows, cols = diff_mat.shape
    digits = min(MAX_DIGITS, max(DEFAULT_NUMERIC_DIGITS, int(params.get("digits", DEFAULT_NUMERIC_DIGITS) or DEFAULT_NUMERIC_DIGITS)))
    points = min(MAX_POINTS, max(DEFAULT_NUMERIC_POINTS, int(params.get("points", DEFAULT_NUMERIC_POINTS) or DEFAULT_NUMERIC_POINTS)))
    for i in range(rows):
        for j in range(cols):
            entry = sp.simplify(diff_mat[i, j])
            probe = numeric_probe(entry, sp.Integer(0), params, num_points=points, digits=digits, relation="eq")
            if probe["status"] == "fail":
                return {
                    "outcome": "fail",
                    "evidence": {
                        "reason": f"{label} violated at matrix entry ({i}, {j})",
                        "counterexample": {"row": i, "col": j, **probe["counterexample"]},
                        "assumptions": assumptions_snapshot(assumed),
                    },
                }
            if entry != 0:
                return {
                    "outcome": "inconclusive",
                    "evidence": {
                        "reason": f"{label} left unreduced residual at ({i}, {j})",
                        "residual_srepr": sp.srepr(entry),
                        "assumptions": assumptions_snapshot(assumed),
                    },
                }
    return {
        "outcome": "pass",
        "evidence": {"shape": [rows, cols], "assumptions": assumptions_snapshot(assumed)},
    }


def check_scalar_zero(
    lhs_expr: sp.Basic,
    rhs_expr: sp.Basic,
    assumed: dict[str, sp.Symbol],
    params: dict[str, Any],
    *,
    label: str,
) -> dict[str, Any]:
    """Symbolically simplify `lhs_expr - rhs_expr` with a high-precision numeric guard."""
    diff = sp.simplify(lhs_expr - rhs_expr)
    if diff != 0 and diff.has(sp.Integral, sp.Sum, sp.Derivative):
        diff = sp.simplify((lhs_expr - rhs_expr).doit())
    digits = min(MAX_DIGITS, max(DEFAULT_NUMERIC_DIGITS, int(params.get("digits", DEFAULT_NUMERIC_DIGITS) or DEFAULT_NUMERIC_DIGITS)))
    points = min(MAX_POINTS, max(DEFAULT_NUMERIC_POINTS, int(params.get("points", DEFAULT_NUMERIC_POINTS) or DEFAULT_NUMERIC_POINTS)))
    probe = eval_scalar_residual_points(lhs_expr, rhs_expr, assumed, params, num_points=points, digits=digits)
    if probe["status"] == "fail":
        return {
            "outcome": "fail",
            "evidence": {
                "reason": f"{label} violated numerically",
                "counterexample": probe["counterexample"],
                "residual_srepr": sp.srepr(diff),
                "assumptions": probe["assumptions"],
            },
        }
    if diff == 0:
        return {
            "outcome": "pass",
            "evidence": {
                "guard_points": probe["valid"],
                "max_residual": probe["max_err"],
                "assumptions": probe["assumptions"],
            },
        }
    return {
        "outcome": "inconclusive",
        "evidence": {
            "reason": f"{label} left unreduced symbolic residual",
            "residual_srepr": sp.srepr(diff),
            "guard_points": probe["valid"],
            "assumptions": probe["assumptions"],
        },
    }


def _map_convergence_report(rep: dict[str, Any], *, mode: str) -> dict[str, Any]:
    ratios: list[str] = []
    deltas = [s["delta"] for s in rep["samples"] if s.get("delta") is not None]
    for a_s, b_s in zip(deltas, deltas[1:]):
        try:
            a, b = Decimal(a_s), Decimal(b_s)
            if b > 0:
                ratios.append(str((a / b).quantize(Decimal("0.0001"))))
        except Exception:
            pass
    ev = {
        "mode": mode,
        "status": rep["status"],
        "stable": rep["stable"],
        "within_tolerance": rep["within_tolerance"],
        "reference_within_tolerance": rep["reference_within_tolerance"],
        "residual_ratios": ratios,
        "samples": rep["samples"],
    }
    status = rep["status"]
    if status == "within_tolerance":
        return {"outcome": "pass", "evidence": ev}
    if status in ("not_stable", "outside_reference_tolerance"):
        ev["reason"] = f"convergence check failed ({status})"
        ev["counterexample"] = rep["samples"][-1]
        return {"outcome": "fail", "evidence": ev}
    if status == "evaluation_failed":
        msgs = [str((s.get("error") or {}).get("message", "")) for s in rep["samples"] if s.get("error")]
        if any("finite" in m for m in msgs):
            ev["reason"] = "sequence diverged to non-finite value"
            ev["counterexample"] = rep["samples"][-1]
            return {"outcome": "fail", "evidence": ev}
    ev["reason"] = f"convergence inconclusive ({status})"
    return {"outcome": "inconclusive", "evidence": ev}


def convergence(handles: dict[str, Any], params: dict[str, Any]) -> dict[str, Any]:
    """Series, integral, sequence, or mesh-refinement convergence check via SymPy & `numerical_validation`."""
    require_lhs_rhs(handles)
    norm, assumed, mismatch = normalize_physics_handles(handles, params)
    if mismatch is not None or norm is None:
        return assumption_mismatch_result(mismatch)

    side = str(params.get("side", "lhs")).strip()
    if side not in ("lhs", "rhs"):
        raise ValueError("convergence `side` must be 'lhs' or 'rhs'")
    target = norm[side]
    other = norm["rhs" if side == "lhs" else "lhs"]

    var_raw = params.get("variable") or params.get("var")
    levels_raw = params.get("levels")
    axis = str(params.get("axis", "resolution")).strip()
    tail = int(params.get("tail", 2) or 2)
    atol = str(params.get("atol", "1e-6"))
    rtol = str(params.get("rtol", "1e-6"))
    digits = min(256, max(20, int(params.get("work_dps", params.get("digits", 50)) or 50)))

    if isinstance(target, sp.Sum) and var_raw is None and levels_raw is None:
        try:
            conv = target.is_convergent()
        except NotImplementedError:
            conv = None
        if conv is sp.S.false:
            return {"outcome": "fail", "evidence": {"mode": "sum", "convergent": False, "reason": "symbolic sum diverges"}}
        if conv is sp.S.true:
            return {"outcome": "pass", "evidence": {"mode": "sum", "convergent": True}}

    if isinstance(target, sp.Integral) and var_raw is None and levels_raw is None:
        try:
            val = target.doit()
        except Exception:
            val = target
        if val in (sp.oo, -sp.oo, sp.zoo):
            return {"outcome": "fail", "evidence": {"mode": "integral", "convergent": False, "reason": "improper integral diverges"}}
        if not val.has(sp.Integral, sp.oo, -sp.oo, sp.zoo, sp.nan) and not val.free_symbols:
            return {"outcome": "pass", "evidence": {"mode": "integral", "convergent": True, "value": str(val)}}
        if len(target.limits) == 1 and not (target.free_symbols - {target.limits[0][0]}):
            ivar, a_lim, b_lim = target.limits[0]
            if b_lim == sp.oo or a_lim == -sp.oo:
                levels_raw = [10, 20, 40, 80]

                def _quad_run(lvl: Any) -> mp.mpf | mp.mpc:
                    lo = -mp.mpf(lvl) if a_lim == -sp.oo else eval_mp_scalar(a_lim, {}, digits)
                    hi = mp.mpf(lvl) if b_lim == sp.oo else eval_mp_scalar(b_lim, {}, digits)
                    if lo is None or hi is None:
                        raise ValueError("non-numeric integral limit")

                    def _integrand(t: mp.mpf) -> mp.mpf | mp.mpc:
                        val_t = eval_mp_scalar(target.function, {ivar: sp.Float(mp.nstr(t, digits + 5), digits + 5)}, digits)
                        if val_t is None:
                            raise ValueError("non-finite integrand sample")
                        return val_t

                    return mp.quad(_integrand, [lo, hi])

                rep = check_convergence(_quad_run, levels_raw, atol=atol, rtol=rtol, axis="resolution", tail=tail, work_dps=digits)
                return _map_convergence_report(rep, mode="integral_quadrature")

    if isinstance(target, (sp.MatrixBase, sp.Tuple)):
        items = list(target)
        if len(items) < tail + 1:
            return {"outcome": "inconclusive", "evidence": {"mode": "vector", "reason": "insufficient refinement levels"}}
        levels = levels_raw if isinstance(levels_raw, list) and len(levels_raw) == len(items) else list(range(1, len(items) + 1))
        ref_val = None
        if params.get("use_rhs_as_reference", True) and not isinstance(other, (sp.MatrixBase, sp.Tuple)) and not other.free_symbols and other != target:
            ref_val = eval_mp_scalar(other, {}, digits)

        def _vec_run(lvl: Any) -> Any:
            idx = levels.index(lvl) if lvl in levels else int(lvl) - 1
            val = eval_mp_scalar(items[idx], {}, digits)
            if val is None:
                raise ValueError("non-finite refinement sample")
            return val

        rep = check_convergence(_vec_run, levels, atol=atol, rtol=rtol, reference=ref_val, axis=axis, tail=tail, work_dps=digits)
        return _map_convergence_report(rep, mode="refinement_vector")

    if var_raw is None:
        return {"outcome": "inconclusive", "evidence": {"reason": "convergence requires a Sum, Integral, vector, or `variable` with `levels`"}}
    vname = validate_symbol_name(var_raw, "variable")
    vsym = assumed.get(vname, sp.Symbol(vname))
    levels = levels_raw if isinstance(levels_raw, list) and levels_raw else [8, 16, 32, 64, 128]
    direction = str(params.get("direction", "n->oo")).strip()
    rem = sorted({s for s in target.free_symbols if hasattr(s, "name") and s != vsym}, key=lambda s: s.name)
    base_subs = sample_parameter_points(rem, params, 1)[0] if rem else {}

    ref_val = None
    if params.get("use_rhs_as_reference", True) and vsym not in other.free_symbols and other != target:
        r_eval = eval_mp_scalar(other, base_subs, digits)
        if r_eval is not None:
            ref_val = r_eval

    def _expr_run(lvl: Any) -> Any:
        sub = dict(base_subs)
        sub[vsym] = sp.Rational(1, int(lvl)) if direction == "h->0" else sp.Integer(int(lvl))
        val = eval_mp_scalar(target, sub, digits)
        if val is None:
            raise ValueError(f"non-finite value at level {lvl}")
        return val

    rep = check_convergence(_expr_run, levels, atol=atol, rtol=rtol, reference=ref_val, axis=axis, tail=tail, work_dps=digits)
    return _map_convergence_report(rep, mode=f"sequence({vname})")


def _to_numpy_matrix(mat: sp.MatrixBase, subs: dict[sp.Basic, sp.Expr]) -> np.ndarray | None:
    rows, cols = mat.shape
    arr = np.zeros((rows, cols), dtype=np.complex128)
    has_imag = False
    for i in range(rows):
        for j in range(cols):
            val = eval_mp_scalar(mat[i, j], subs, 30)
            if val is None:
                return None
            re_f, im_f = float(mp.re(val)), float(mp.im(val))
            if not (math.isfinite(re_f) and math.isfinite(im_f)):
                return None
            if im_f != 0.0:
                has_imag = True
            arr[i, j] = complex(re_f, im_f)
    return arr if has_imag else arr.real.astype(np.float64)


def _check_positive_definite(
    mat: sp.MatrixBase,
    free: list[sp.Symbol],
    samples: list[dict[sp.Basic, sp.Expr]],
    tol: float,
    assumed: dict[str, sp.Symbol],
) -> dict[str, Any]:
    n = mat.rows
    sym_decided = True
    if n <= 6:
        for k in range(1, n + 1):
            minor = sp.simplify(mat[:k, :k].det())
            if minor.is_nonpositive is True or minor == 0:
                return {"outcome": "fail", "evidence": {"counterexample": {"leading_minor": k, "det": str(minor)}, "reason": f"leading principal minor {k} is non-positive"}}
            if minor.is_positive is not True:
                sym_decided = False

    checked = 0
    min_eig_seen: float | None = None
    for sub in samples[:DEFAULT_NUMERIC_POINTS]:
        np_m = _to_numpy_matrix(mat, sub)
        if np_m is None:
            continue
        checked += 1
        herm = 0.5 * (np_m + np_m.conj().T)
        eigs = np.linalg.eigvalsh(herm)
        min_ev = float(np.min(eigs))
        min_eig_seen = min_ev if min_eig_seen is None else min(min_eig_seen, min_ev)
        if min_ev <= tol:
            pt = ", ".join(f"{s.name}={v}" for s, v in sub.items()) or "constant"
            return {"outcome": "fail", "evidence": {"counterexample": {"point": pt, "min_eigenvalue": min_ev}, "reason": "matrix has non-positive eigenvalue"}}
        if not free:
            break

    if checked > 0 and (sym_decided or not free or checked >= DEFAULT_NUMERIC_POINTS):
        return {"outcome": "pass", "evidence": {"min_eigenvalue": min_eig_seen, "samples_checked": checked, "assumptions": assumptions_snapshot(assumed)}}
    return {"outcome": "inconclusive", "evidence": {"reason": "could not prove positive definiteness"}}


def _check_invertible(
    mat: sp.MatrixBase,
    free: list[sp.Symbol],
    samples: list[dict[sp.Basic, sp.Expr]],
    tol: float,
    assumed: dict[str, sp.Symbol],
) -> dict[str, Any]:
    n = mat.rows
    det_expr = sp.simplify(mat.det()) if n <= 6 else None
    if det_expr is not None:
        if det_expr == 0:
            return {"outcome": "fail", "evidence": {"counterexample": {"det": "0"}, "reason": "matrix is singular (det = 0)"}}
        if len(free) == 1:
            roots = sp.solveset(det_expr, free[0], sp.S.Reals if free[0].is_real else sp.S.Complexes)
            if roots is not sp.S.EmptySet and roots.is_empty is not True:
                try:
                    sample_roots = list(islice(roots, 3))
                except TypeError:
                    sample_roots = []
                for rt in sample_roots:
                    if all(getattr(rt, f"is_{k}") is not False for k, v in free[0].assumptions0.items() if k != "commutative" and v is True):
                        return {"outcome": "fail", "evidence": {"counterexample": {"point": f"{free[0].name}={rt}", "det": "0"}, "reason": "determinant vanishes in domain"}}

    checked = 0
    min_sv_seen: float | None = None
    for sub in samples[:DEFAULT_NUMERIC_POINTS]:
        np_m = _to_numpy_matrix(mat, sub)
        if np_m is None:
            continue
        checked += 1
        svs = np.linalg.svd(np_m, compute_uv=False)
        min_sv = float(np.min(svs))
        min_sv_seen = min_sv if min_sv_seen is None else min(min_sv_seen, min_sv)
        if min_sv <= tol:
            pt = ", ".join(f"{s.name}={v}" for s, v in sub.items()) or "constant"
            return {"outcome": "fail", "evidence": {"counterexample": {"point": pt, "min_singular_value": min_sv}, "reason": "matrix is numerically singular"}}
        if not free:
            break

    if (det_expr is not None and det_expr.is_zero is False) or (checked > 0 and (not free or checked >= DEFAULT_NUMERIC_POINTS)):
        return {"outcome": "pass", "evidence": {"min_singular_value": min_sv_seen, "samples_checked": checked, "assumptions": assumptions_snapshot(assumed)}}
    return {"outcome": "inconclusive", "evidence": {"reason": "matrix invertibility could not be established"}}


def matrix_property(handles: dict[str, Any], params: dict[str, Any]) -> dict[str, Any]:
    """Check `hermitian`, `symmetric`, `unitary`, `orthogonal`, `positive_definite`, `traceless`, `invertible`."""
    require_lhs_rhs(handles)
    norm, assumed, mismatch = normalize_physics_handles(handles, params)
    if mismatch is not None or norm is None:
        return assumption_mismatch_result(mismatch)

    props_raw = params.get("properties")
    if isinstance(props_raw, list) and props_raw:
        props = [str(p).strip() for p in props_raw]
    elif params.get("property") is not None:
        props = [str(params["property"]).strip()]
    else:
        raise ValueError("matrix_property requires `property` or `properties`")
    for p in props:
        if p not in _MATRIX_PROPERTIES:
            raise ValueError(f"unsupported matrix_property {p!r}")

    side = str(params.get("side", "lhs")).strip()
    if side not in ("lhs", "rhs", "both"):
        raise ValueError("matrix_property `side` must be 'lhs', 'rhs', or 'both'")
    targets = [("lhs", norm["lhs"])] if side == "lhs" else ([("rhs", norm["rhs"])] if side == "rhs" else [("lhs", norm["lhs"]), ("rhs", norm["rhs"])])
    tol = float(params.get("tolerance", 1e-10) or 1e-10)
    if not (math.isfinite(tol) and tol > 0):
        raise ValueError("matrix_property `tolerance` must be finite and positive")

    for label, raw_mat in targets:
        mat = raw_mat.as_explicit() if isinstance(raw_mat, sp.MatrixExpr) and all(isinstance(d, (int, sp.Integer)) and int(d) <= 64 for d in raw_mat.shape) else raw_mat
        if not isinstance(mat, sp.MatrixBase):
            return {"outcome": "inconclusive", "evidence": {"side": label, "reason": "handle is not an explicit matrix"}}
        if mat.rows != mat.cols or mat.rows == 0:
            return {"outcome": "fail", "evidence": {"side": label, "shape": [mat.rows, mat.cols], "reason": "matrix property requires non-empty square matrix"}}
        n = mat.rows
        eye = sp.eye(n)
        free = sorted({s for s in mat.free_symbols if hasattr(s, "name")}, key=lambda s: s.name)
        samples = sample_parameter_points(free, params, DEFAULT_NUMERIC_POINTS)

        for prop in props:
            if prop == "hermitian":
                res = check_matrix_zero(mat.H - mat, assumed, params, label="hermitian")
            elif prop == "symmetric":
                res = check_matrix_zero(mat.T - mat, assumed, params, label="symmetric")
            elif prop == "unitary":
                res = check_matrix_zero(mat.H * mat - eye, assumed, params, label="unitary(A^H A - I)")
                if res["outcome"] == "pass":
                    res = check_matrix_zero(mat * mat.H - eye, assumed, params, label="unitary(A A^H - I)")
            elif prop == "orthogonal":
                res = check_matrix_zero(mat.T * mat - eye, assumed, params, label="orthogonal(A^T A - I)")
                if res["outcome"] == "pass":
                    for sub in samples[:DEFAULT_NUMERIC_POINTS]:
                        np_m = _to_numpy_matrix(mat, sub)
                        if np_m is not None and np.iscomplexobj(np_m) and float(np.max(np.abs(np_m.imag))) > tol:
                            pt = ", ".join(f"{s.name}={v}" for s, v in sub.items()) or "constant"
                            return {"outcome": "fail", "evidence": {"property": prop, "side": label, "counterexample": {"point": pt, "max_imag": float(np.max(np.abs(np_m.imag)))}, "reason": "matrix has non-zero imaginary entries"}}
            elif prop == "traceless":
                res = check_scalar_zero(mat.trace(), sp.Integer(0), assumed, params, label="traceless")
            elif prop == "positive_definite":
                res = check_matrix_zero(mat.H - mat, assumed, params, label="positive_definite(hermitian)")
                if res["outcome"] == "pass":
                    res = _check_positive_definite(mat, free, samples, tol, assumed)
            else:
                res = _check_invertible(mat, free, samples, tol, assumed)

            if res["outcome"] != "pass":
                res["evidence"].update({"property": prop, "side": label})
                return res

    return {
        "outcome": "pass",
        "evidence": {"properties": props, "side": side, "assumptions": assumptions_snapshot(assumed)},
    }
