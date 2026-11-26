"""Theoretical physics & CAS helper module for re:Search compute workers.

Provides exact SymPy `srepr` persistence (`save_expr`/`load_expr`), assumed symbols,
quantum/operator algebra (`commutator`, `anticommutator`, `pauli_decompose`,
`bch_expand`, `schrieffer_wolff`), series/resummation (`pade_approximant`,
`pslq_recognize`), and 50-digit `mpmath` numerical identity checks.
"""

from __future__ import annotations

import contextlib
import contextvars
import json
import os
from pathlib import Path
from typing import Any, Iterator

import sympy
from sympy import Symbol, expand, factor, nsimplify, Rational, simplify, srepr, sympify

_CAS_BUS_DEFAULT = Path("scratch/.cas_bus.json")
_active_exprs_out: contextvars.ContextVar[dict[str, str] | None] = contextvars.ContextVar(
    "active_exprs_out", default=None
)
_active_exprs_in: contextvars.ContextVar[dict[str, str] | None] = contextvars.ContextVar(
    "active_exprs_in", default=None
)


def _parse_stored_srepr(raw: str) -> Any:
    """Reconstruct a stored `srepr` string strictly via the restricted AST parser."""
    from checks.srepr_safe import parse_srepr_safe

    return parse_srepr_safe(raw)


@contextlib.contextmanager
def expr_context(exprs_in: dict[str, str] | None = None) -> Iterator[dict[str, str]]:
    """Bind protocol 2 staged input expressions and capture `save_expr` outputs in memory."""
    outbox: dict[str, str] = {}
    staged = dict(exprs_in) if exprs_in else {}
    tok_out = _active_exprs_out.set(outbox)
    tok_in = _active_exprs_in.set(staged)
    try:
        yield outbox
    finally:
        _active_exprs_in.reset(tok_in)
        _active_exprs_out.reset(tok_out)


def _bus_path(path: str | os.PathLike[str] | None = None) -> Path:
    if path is not None:
        return Path(path)
    env_path = os.environ.get("RE_SEARCH_CAS_BUS")
    if env_path:
        return Path(env_path)
    return _CAS_BUS_DEFAULT


def save_expr(name: str, expr: Any, bus_path: str | os.PathLike[str] | None = None) -> str:
    """Persist a SymPy expression as exact `srepr` into the active protocol 2 outbox.

    Inside a worker `evaluate` call, writes only to the in-memory outbox (never to
    `scratch/.cas_bus.json`). When invoked standalone with no active outbox or an
    explicit `bus_path`, falls back to the JSON bus file.
    """
    if not isinstance(name, str) or not name.strip():
        raise ValueError("expression name must be a non-empty string")
    encoded = srepr(sympify(expr))
    outbox = _active_exprs_out.get()
    if outbox is not None and bus_path is None:
        outbox[name] = encoded
        return encoded
    target = _bus_path(bus_path)
    target.parent.mkdir(parents=True, exist_ok=True)
    store: dict[str, str] = {}
    if target.is_file():
        try:
            loaded = json.loads(target.read_text(encoding="utf-8"))
            if isinstance(loaded, dict):
                store = {str(k): str(v) for k, v in loaded.items()}
        except Exception:
            store = {}
    store[name] = encoded
    tmp = target.with_suffix(".tmp")
    tmp.write_text(json.dumps(store, indent=2, sort_keys=True), encoding="utf-8")
    os.replace(tmp, target)
    return encoded


def expr(name: str) -> Any:
    """Load a staged protocol 2 expression handle by name via the restricted `srepr` parser."""
    if not isinstance(name, str) or not name.strip():
        raise ValueError("expression name must be a non-empty string")
    outbox = _active_exprs_out.get()
    if outbox is not None and name in outbox:
        return _parse_stored_srepr(outbox[name])
    staged = _active_exprs_in.get()
    if staged is not None and name in staged:
        return _parse_stored_srepr(staged[name])
    raise KeyError(f"expression {name!r} is not staged in exprs_in or saved in this call")


def load_expr(name: str, bus_path: str | os.PathLike[str] | None = None) -> Any:
    """Load and reconstruct a SymPy expression safely via the restricted `srepr` parser.

    Inside a protocol 2 worker call (when `bus_path` is omitted), delegates to `expr(name)`
    and never reads `scratch/.cas_bus.json`.
    """
    if bus_path is None and (_active_exprs_in.get() is not None or _active_exprs_out.get() is not None):
        return expr(name)
    target = _bus_path(bus_path)
    if not target.is_file():
        raise KeyError(f"CAS bus store not found at {target}")
    store = json.loads(target.read_text(encoding="utf-8"))
    if name not in store:
        raise KeyError(f"expression {name!r} not found in {target}")
    return _parse_stored_srepr(store[name])


def assumed_symbol(name: str, **assumptions: Any) -> Symbol:
    """Create a SymPy `Symbol` with explicit physical domain assumptions."""
    if not assumptions:
        assumptions = {"real": True}
    return Symbol(name, **assumptions)


def commutator(a: Any, b: Any) -> Any:
    """Compute `[A, B] = A * B - B * A`."""
    return simplify(a * b - b * a)


def anticommutator(a: Any, b: Any) -> Any:
    """Compute `{A, B} = A * B + B * A`."""
    return simplify(a * b + b * a)


def pauli_decompose(mat: Any) -> dict[str, Any]:
    """Decompose a 2x2 matrix into `{I, X, Y, Z}` Pauli coefficients `M = sum c_P P`."""
    m = sympy.Matrix(mat)
    if m.shape != (2, 2):
        raise ValueError(f"pauli_decompose expects a 2x2 matrix, got {m.shape}")
    paulis = {
        "I": sympy.Matrix([[1, 0], [0, 1]]),
        "X": sympy.Matrix([[0, 1], [1, 0]]),
        "Y": sympy.Matrix([[0, -sympy.I], [sympy.I, 0]]),
        "Z": sympy.Matrix([[1, 0], [0, -1]]),
    }
    return {label: simplify((m * p).trace() / 2) for label, p in paulis.items()}


def bch_expand(a: Any, b: Any, order: int = 4) -> Any:
    """Baker-Campbell-Hausdorff series for `log(exp(A) * exp(B))` up to `order <= 4`."""
    if order < 1 or order > 4:
        raise ValueError("bch_expand supports order in 1..=4")
    c1 = a + b
    if order == 1:
        return c1
    ab = commutator(a, b)
    c2 = c1 + Rational(1, 2) * ab
    if order == 2:
        return c2
    c3 = c2 + Rational(1, 12) * (commutator(a, ab) - commutator(b, ab))
    if order == 3:
        return c3
    aab = commutator(a, ab)
    c4 = c3 - Rational(1, 24) * commutator(b, aab)
    return expand(c4)


def schrieffer_wolff(h0: Any, v: Any, generator_s: Any, order: int = 2) -> Any:
    """Effective Hamiltonian `e^S (H0 + V) e^{-S}` truncated at `order <= 3`."""
    if order < 1 or order > 3:
        raise ValueError("schrieffer_wolff supports order in 1..=3")
    s = generator_s
    h_full = h0 + v
    eff = h0 + v + commutator(s, h_full)
    if order == 1:
        return simplify(eff)
    eff = eff + Rational(1, 2) * commutator(s, commutator(s, h_full))
    if order == 2:
        return simplify(eff)
    eff = eff + Rational(1, 6) * commutator(s, commutator(s, commutator(s, h_full)))
    return simplify(eff)


def pade_approximant(expr: Any, var: Any, m: int, n: int, x0: Any = 0) -> Any:
    """Construct the `[m/n]` Padé rational approximant around `var = x0`."""
    x = sympify(var)
    shifted = sympify(expr).subs(x, x + x0)
    series = sympy.series(shifted, x, 0, m + n + 1).removeO()
    coeffs = [series.coeff(x, k) for k in range(m + n + 1)]
    if n == 0:
        num = sum(coeffs[k] * (x - x0) ** k for k in range(m + 1))
        return simplify(num)
    mat_rows = []
    rhs_vals = []
    for row in range(m + 1, m + n + 1):
        mat_rows.append([coeffs[row - col] if row - col >= 0 else 0 for col in range(1, n + 1)])
        rhs_vals.append(-coeffs[row])
    b_vec = sympy.Matrix(mat_rows).LUsolve(sympy.Matrix(rhs_vals))
    q_coeffs = [sympy.Integer(1)] + [b_vec[i] for i in range(n)]
    p_coeffs = []
    for k in range(m + 1):
        val = sum(q_coeffs[j] * coeffs[k - j] for j in range(min(k, n) + 1))
        p_coeffs.append(val)
    dx = x - x0
    num = sum(p_coeffs[k] * dx**k for k in range(m + 1))
    den = sum(q_coeffs[k] * dx**k for k in range(n + 1))
    return factor(num / den)


def pslq_recognize(value: Any, basis: list[Any] | tuple[Any, ...] | None = None, tol: float = 1e-30) -> Any:
    """Identify an exact closed form or integer relation via `mpmath.pslq` / `nsimplify`."""
    import mpmath

    mpmath.mp.dps = 50
    if basis is None:
        return nsimplify(value, tolerance=tol)
    vec = [mpmath.mpf(value)] + [mpmath.mpf(b) for b in basis]
    rel = mpmath.pslq(vec, tol=tol)
    if rel is None or rel[0] == 0:
        return None
    return simplify(-sum(sympy.Integer(c) * sympify(b) for c, b in zip(rel[1:], basis)) / sympy.Integer(rel[0]))


def check_identity_mpmath(
    lhs: Any,
    rhs: Any,
    samples: dict[Any, list[Any] | tuple[Any, ...]] | list[dict[Any, Any]],
    *,
    dps: int = 50,
    tol: str = "1e-35",
) -> dict[str, Any]:
    """Numerically verify `lhs == rhs` at `dps = 50` precision across deterministic points."""
    import mpmath

    mpmath.mp.dps = dps
    diff_expr = sympify(lhs) - sympify(rhs)
    if isinstance(samples, dict):
        keys = list(samples.keys())
        count = min((len(samples[k]) for k in keys), default=0)
        point_dicts = [{k: samples[k][i] for k in keys} for i in range(count)]
    else:
        point_dicts = list(samples)

    max_err = mpmath.mpf("0")
    tol_val = mpmath.mpf(tol)
    checked = 0
    for pt in point_dicts:
        val = mpmath.mpc(diff_expr.subs(pt).evalf(dps))
        err = abs(val)
        if err > max_err:
            max_err = err
        checked += 1
        if err > tol_val:
            return {"ok": False, "checked": checked, "dps": dps, "max_abs_error": str(max_err)}
    return {"ok": checked > 0, "checked": checked, "dps": dps, "max_abs_error": str(max_err)}


__all__ = [
    "assumed_symbol",
    "anticommutator",
    "bch_expand",
    "check_identity_mpmath",
    "commutator",
    "expr",
    "load_expr",
    "pade_approximant",
    "pauli_decompose",
    "pslq_recognize",
    "save_expr",
    "schrieffer_wolff",
]
