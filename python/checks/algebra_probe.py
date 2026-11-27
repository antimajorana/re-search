"""Symbol-assumption validation and domain-aware mpmath probing for `checks.algebra`."""

from __future__ import annotations

import re
from typing import Any

import mpmath as mp
import sympy as sp

from .srepr_safe import ALLOWED_ASSUMPTION_KWARGS

DEFAULT_NUMERIC_POINTS = 12
DEFAULT_NUMERIC_DIGITS = 50
DEFAULT_COUNTEREXAMPLE_POINTS = 24
MAX_SERIES_ORDER = 24
MAX_POINTS = 64
MAX_DIGITS = 120

_INTERVAL_RE = re.compile(
    r"\b([A-Za-z_][A-Za-z0-9_]*)\s*(?:in|\\in)\s*([\[(])\s*([^,\]]+?)\s*,\s*([^\]\)]+?)\s*([\])])"
)
_INEQ_LOWER_RE = re.compile(
    r"\b([A-Za-z_][A-Za-z0-9_]*)\s*(>=|>)\s*([-+A-Za-z0-9_./*()^\s]+?)(?=$|[,;]|and\b|with\b|\))"
)
_INEQ_UPPER_RE = re.compile(
    r"\b([A-Za-z_][A-Za-z0-9_]*)\s*(<=|<)\s*([-+A-Za-z0-9_./*()^\s]+?)(?=$|[,;]|and\b|with\b|\))"
)

_SAFE_SCALARS: dict[str, sp.Expr] = {
    "pi": sp.pi,
    "e": sp.E,
    "E": sp.E,
    "oo": sp.oo,
    "inf": sp.oo,
    "infty": sp.oo,
    "infty.": sp.oo,
}


def parse_simple_real(text: str) -> sp.Expr | None:
    s = text.strip().replace("^", "**").replace("\\pi", "pi")
    if not s or len(s) > 64:
        return None
    if s in ("-oo", "-inf", "-infty"):
        return -sp.oo
    if s in ("oo", "+oo", "inf", "+inf", "infty", "+infty"):
        return sp.oo
    sign = 1
    if s.startswith("+"):
        s = s[1:].strip()
    elif s.startswith("-"):
        sign = -1
        s = s[1:].strip()
    if "/" in s and s.count("/") == 1:
        num_s, den_s = (part.strip() for part in s.split("/", 1))
        num = _parse_atom(num_s)
        den = _parse_atom(den_s)
        if num is not None and den is not None and den != 0:
            return sign * num / den
        return None
    atom = _parse_atom(s)
    return sign * atom if atom is not None else None


def _parse_atom(s: str) -> sp.Expr | None:
    if s in _SAFE_SCALARS:
        return _SAFE_SCALARS[s]
    if re.fullmatch(r"\d+", s):
        return sp.Integer(int(s))
    if re.fullmatch(r"\d+\.\d+", s):
        return sp.Rational(s)
    m = re.fullmatch(r"sqrt\(\s*(\d+)\s*\)", s)
    if m:
        return sp.sqrt(sp.Integer(int(m.group(1))))
    m2 = re.fullmatch(r"(\d+)\s*\*\s*pi", s)
    if m2:
        return sp.Integer(int(m2.group(1))) * sp.pi
    return None


def _extract_assumed_symbols(params: dict[str, Any]) -> dict[str, sp.Symbol]:
    raw_syms = params.get("symbols") or []
    assumed: dict[str, sp.Symbol] = {}
    if not isinstance(raw_syms, list):
        return assumed
    for item in raw_syms:
        if not isinstance(item, dict):
            continue
        name = str(item.get("name", "")).strip()
        if not name or not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]{0,63}", name):
            continue
        raw_flags = item.get("assumptions") or []
        kwargs: dict[str, bool] = {}
        if isinstance(raw_flags, list):
            for flag in raw_flags:
                if isinstance(flag, str):
                    key = flag.strip()
                    if key in ALLOWED_ASSUMPTION_KWARGS:
                        kwargs[key] = True
        assumed[name] = sp.Symbol(name, **kwargs)
    return assumed


def _explicit_assumptions(sym: sp.Basic) -> dict[str, Any]:
    return {k: v for k, v in sym.assumptions0.items() if k != "commutative"}


def _assumptions_compatible(handle_sym: sp.Basic, setup_sym: sp.Symbol) -> bool:
    h_exp = _explicit_assumptions(handle_sym)
    if not h_exp:
        return True
    s_attrs = setup_sym.assumptions0
    for k, val in h_exp.items():
        if s_attrs.get(k) != val:
            return False
    for k, val in s_attrs.items():
        if k == "commutative":
            continue
        if h_exp.get(k) is False and val is True:
            return False
    return True


def normalize_symbols(
    exprs: dict[str, sp.Basic], params: dict[str, Any]
) -> tuple[dict[str, sp.Basic] | None, dict[str, sp.Symbol], str | None]:
    assumed = _extract_assumed_symbols(params)
    seen_by_name: dict[str, sp.Basic] = {}
    subs_map: dict[sp.Basic, sp.Basic] = {}

    for expr in exprs.values():
        if not isinstance(expr, sp.Basic):
            continue
        for sym in expr.free_symbols:
            name = getattr(sym, "name", None)
            if not name:
                continue
            prev = seen_by_name.get(name)
            if prev is not None and prev.assumptions0 != sym.assumptions0:
                return None, assumed, "inconclusive: symbol assumptions differ"
            seen_by_name[name] = sym
            if name in assumed:
                target = assumed[name]
                if not _assumptions_compatible(sym, target):
                    return None, assumed, "inconclusive: symbol assumptions differ"
                if sym.assumptions0 != target.assumptions0:
                    subs_map[sym] = target
            else:
                assumed[name] = sym

    out = {}
    for k, v in exprs.items():
        out[k] = v.xreplace(subs_map) if subs_map and isinstance(v, sp.Basic) else v
    return out, assumed, None


def _parse_domain_constraints(
    domain_text: str | None, params: dict[str, Any]
) -> dict[str, tuple[sp.Expr | None, bool, sp.Expr | None, bool]]:
    bounds: dict[str, tuple[sp.Expr | None, bool, sp.Expr | None, bool]] = {}
    ranges = params.get("ranges")
    if isinstance(ranges, dict):
        for name, pair in ranges.items():
            if isinstance(pair, (list, tuple)) and len(pair) == 2:
                lo = parse_simple_real(str(pair[0]))
                hi = parse_simple_real(str(pair[1]))
                if lo is not None or hi is not None:
                    bounds[str(name)] = (lo, True, hi, True)
    if not isinstance(domain_text, str) or not domain_text.strip():
        return bounds

    for m in _INTERVAL_RE.finditer(domain_text):
        name, lb, lo_s, hi_s, rb = m.groups()
        lo = parse_simple_real(lo_s)
        hi = parse_simple_real(hi_s)
        if lo is not None or hi is not None:
            bounds[name] = (lo, lb == "[", hi, rb == "]")
    for m in _INEQ_LOWER_RE.finditer(domain_text):
        name, op, raw = m.groups()
        if name in bounds:
            continue
        lo = parse_simple_real(raw)
        if lo is not None:
            bounds[name] = (lo, op == ">=", None, False)
    for m in _INEQ_UPPER_RE.finditer(domain_text):
        name, op, raw = m.groups()
        hi = parse_simple_real(raw)
        if hi is not None:
            prev = bounds.get(name, (None, False, None, False))
            bounds[name] = (prev[0], prev[1], hi, op == "<=")
    return bounds


def _candidates_for_symbol(
    sym: sp.Basic,
    bound: tuple[sp.Expr | None, bool, sp.Expr | None, bool] | None,
    count: int,
    seed: int,
) -> list[sp.Expr]:
    a = sym.assumptions0
    is_int = bool(a.get("integer") or a.get("even") or a.get("odd"))
    if a.get("zero") is True:
        return [sp.Integer(0)]

    if bound is not None:
        lo, lo_inc, hi, hi_inc = bound
        lo_finite = lo is not None and lo.is_finite is True
        hi_finite = hi is not None and hi.is_finite is True
        if lo_finite and hi_finite and (hi - lo).is_positive is True:
            pts: list[sp.Expr] = []
            if is_int:
                start_i = int(sp.ceiling(lo)) + (0 if lo_inc else 1)
                end_i = int(sp.floor(hi)) - (0 if hi_inc else 1)
                return [sp.Integer(k) for k in range(start_i, min(end_i + 1, start_i + count * 2))]
            span = hi - lo
            offsets = [sp.Rational(i + 1, count + 2) for i in range(count)]
            if lo_inc:
                offsets.insert(0, sp.Integer(0))
            if hi_inc:
                offsets.append(sp.Integer(1))
            seen = set()
            for t in offsets:
                val = sp.nsimplify(lo + t * span, rational=False)
                if val not in seen:
                    seen.add(val)
                    pts.append(val)
            return pts
        if lo_finite:
            base = lo
            step_pts = [sp.Rational(k, 3) for k in range(0 if lo_inc else 1, count * 2)]
            return [base + s for s in step_pts]
        if hi_finite:
            base = hi
            step_pts = [sp.Rational(k, 3) for k in range(0 if hi_inc else 1, count * 2)]
            return [base - s for s in step_pts]

    if a.get("even"):
        pool = [0, 2, -2, 4, -4, 6, -6, 8, -8, 10, -10, 12, -12, 14, -14, 16]
        if a.get("positive"):
            pool = [k for k in pool if k > 0]
        elif a.get("nonnegative"):
            pool = [k for k in pool if k >= 0]
        elif a.get("negative"):
            pool = [k for k in pool if k < 0]
        return [sp.Integer(k) for k in pool]
    if a.get("odd"):
        pool = [1, -1, 3, -3, 5, -5, 7, -7, 9, -9, 11, -11, 13, -13, 15, -15]
        if a.get("positive"):
            pool = [k for k in pool if k > 0]
        elif a.get("negative"):
            pool = [k for k in pool if k < 0]
        return [sp.Integer(k) for k in pool]
    if is_int:
        pool = [1, 2, 3, -1, -2, 4, 5, -3, 7, 11, -5, 13, 17, 19, -7, 23]
        if a.get("positive"):
            pool = [k for k in range(1, count * 2)]
        elif a.get("nonnegative"):
            pool = [k for k in range(0, count * 2)]
        elif a.get("negative"):
            pool = [-k for k in range(1, count * 2)]
        return [sp.Integer(k) for k in pool]

    positives = [
        sp.Rational(3, 10), sp.Rational(7, 10), sp.Integer(1), sp.Rational(13, 10),
        sp.Rational(7, 4), sp.Rational(23, 10), sp.Rational(31, 10), sp.Rational(43, 10),
        sp.Rational(59, 10), sp.Rational(73, 10), sp.Rational(97, 10), sp.Rational(127, 10),
        sp.Rational(151, 10), sp.Rational(193, 10), sp.Rational(239, 10), sp.Rational(29, 1),
    ]
    shift = sp.Rational((seed % 7) + 1, 97)
    positives = [p + shift for p in positives]
    if a.get("positive"):
        return positives
    if a.get("nonnegative"):
        return [sp.Integer(0)] + positives
    if a.get("negative"):
        return [-p for p in positives]
    if a.get("nonpositive"):
        return [sp.Integer(0)] + [-p for p in positives]
    if a.get("real") or a.get("extended_real"):
        interleaved: list[sp.Expr] = []
        for idx, p in enumerate(positives):
            interleaved.append(-p if idx % 2 else p)
        interleaved.insert(2, sp.Integer(0))
        return interleaved
    if a.get("imaginary"):
        return [sp.I * p for p in positives] + [-sp.I * p for p in positives]

    pairs = [
        (-1, 1), (-2, 1), (1, 2), (-3, -1), (2, -3), (-1, 3),
        (sp.Rational(1, 2), sp.Rational(3, 4)), (-sp.Rational(3, 2), sp.Rational(5, 4)),
        (sp.Rational(5, 3), -sp.Rational(2, 3)), (-sp.Rational(7, 5), -sp.Rational(4, 5)),
        (sp.Rational(9, 4), sp.Rational(1, 2)), (-sp.Rational(11, 4), sp.Rational(3, 2)),
        (sp.Rational(3, 2), sp.Integer(0)), (-sp.Rational(5, 2), sp.Integer(0)),
    ]
    return [re_part + sp.I * im_part for re_part, im_part in pairs]


def eval_mp_scalar(
    expr: sp.Basic, subs: dict[sp.Basic, sp.Expr], work_dps: int
) -> mp.mpc | mp.mpf | None:
    try:
        substituted = expr.xreplace(subs) if subs else expr
        val = substituted.evalf(n=work_dps)
        if getattr(val, "has", None) and (
            val.has(sp.nan, sp.zoo, sp.oo, -sp.oo) or val.free_symbols
        ):
            return None
        re_v, im_v = val.as_real_imag()
        re_s = mp.nstr(mp.mpf(str(re_v.evalf(n=work_dps))), work_dps)
        im_s = mp.nstr(mp.mpf(str(im_v.evalf(n=work_dps))), work_dps)
        re_mp = mp.mpf(re_s)
        im_mp = mp.mpf(im_s)
        if not (mp.isfinite(re_mp) and mp.isfinite(im_mp)):
            return None
        return re_mp if im_mp == 0 else mp.mpc(re_mp, im_mp)
    except Exception:
        return None


def numeric_probe(
    lhs: sp.Basic,
    rhs: sp.Basic,
    params: dict[str, Any],
    *,
    num_points: int,
    digits: int,
    relation: str = "eq",
) -> dict[str, Any]:
    work_dps = digits + 20
    old_dps = mp.mp.dps
    mp.mp.dps = work_dps
    try:
        tol_raw = params.get("tolerance")
        tol = mp.mpf(str(tol_raw)) if tol_raw is not None else mp.power(10, -(digits - 12))

        free = sorted(
            {s for s in (lhs.free_symbols | rhs.free_symbols) if hasattr(s, "name")},
            key=lambda s: s.name,
        )
        bounds = _parse_domain_constraints(params.get("domain"), params)
        seed = int(params.get("seed", 0) or 0)

        candidate_lists = [
            _candidates_for_symbol(s, bounds.get(s.name), num_points + 8, seed) for s in free
        ]
        total_slots = 1 if not free else max(num_points + 12, max(len(c) for c in candidate_lists))
        samples: list[dict[str, str]] = []
        valid_count = 0
        skipped_singularities = 0
        max_err = mp.mpf("0")

        for idx in range(total_slots):
            subs = {
                sym: cands[(idx * (pos + 1) + idx) % len(cands)]
                if idx >= len(cands)
                else cands[idx]
                for pos, (sym, cands) in enumerate(zip(free, candidate_lists))
            }
            if not free and idx > 0:
                break
            l_val = eval_mp_scalar(lhs, subs, work_dps)
            r_val = eval_mp_scalar(rhs, subs, work_dps)
            if l_val is None or r_val is None:
                skipped_singularities += 1
                continue

            valid_count += 1
            point_str = ", ".join(f"{sym.name}={pt}" for sym, pt in subs.items()) or "constant"

            if relation in ("le", "ge"):
                if abs(mp.im(l_val)) > tol or abs(mp.im(r_val)) > tol:
                    return {
                        "status": "fail",
                        "valid": valid_count,
                        "max_err": "nan",
                        "tol": mp.nstr(tol, 4),
                        "counterexample": {
                            "point": point_str,
                            "lhs": str(l_val),
                            "rhs": str(r_val),
                            "residual": "non-real value for real inequality",
                        },
                        "samples": samples,
                        "skipped_singularities": skipped_singularities,
                    }
                lv, rv = mp.re(l_val), mp.re(r_val)
                scale = max(mp.mpf("1"), abs(lv), abs(rv))
                violation = (lv - rv) if relation == "le" else (rv - lv)
                if violation > tol * scale:
                    return {
                        "status": "fail",
                        "valid": valid_count,
                        "max_err": mp.nstr(violation, 6),
                        "tol": mp.nstr(tol, 4),
                        "counterexample": {
                            "point": point_str,
                            "lhs": mp.nstr(lv, 20),
                            "rhs": mp.nstr(rv, 20),
                            "residual": mp.nstr(lv - rv, 20),
                        },
                        "samples": samples,
                        "skipped_singularities": skipped_singularities,
                    }
                diff_mag = max(mp.mpf("0"), violation)
            else:
                diff = l_val - r_val
                err = abs(diff)
                scale = max(mp.mpf("1"), abs(l_val), abs(r_val))
                if err > tol * scale:
                    return {
                        "status": "fail",
                        "valid": valid_count,
                        "max_err": mp.nstr(err, 6),
                        "tol": mp.nstr(tol, 4),
                        "counterexample": {
                            "point": point_str,
                            "lhs": str(l_val),
                            "rhs": str(r_val),
                            "residual": str(diff),
                        },
                        "samples": samples,
                        "skipped_singularities": skipped_singularities,
                    }
                diff_mag = err

            if diff_mag > max_err:
                max_err = diff_mag
            if len(samples) < 5:
                samples.append({"point": point_str, "residual": mp.nstr(diff_mag, 6)})
            if not free or valid_count >= num_points:
                break

        needed = 1 if not free else num_points
        status = "pass" if valid_count >= needed else "inconclusive"
        return {
            "status": status,
            "valid": valid_count,
            "max_err": mp.nstr(max_err, 6),
            "tol": mp.nstr(tol, 4),
            "samples": samples,
            "skipped_singularities": skipped_singularities,
        }
    finally:
        mp.mp.dps = old_dps


def assumptions_snapshot(assumed: dict[str, sp.Symbol]) -> dict[str, list[str]]:
    snap: dict[str, list[str]] = {}
    for name in sorted(assumed):
        flags = sorted(
            k for k, v in assumed[name].assumptions0.items() if v is True and k != "commutative"
        )
        snap[name] = flags
    return snap
