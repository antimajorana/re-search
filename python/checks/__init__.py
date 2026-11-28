"""Host-shipped mathematical check package for the Solve check process (package S6).

Only modules in this package execute inside the check-role Python process.
All expression handles enter through `srepr_safe.parse_srepr_safe` — never via
`eval`, `exec`, or `sympify` on strings. Packages S9 (`algebra.py`) and S10
(`physics.py`) register their domain checks via `register_check`.
"""

from __future__ import annotations

import hashlib
from pathlib import Path
from typing import Any, Callable

import sympy as sp

from .algebra import ALGEBRA_CHECKS
from .physics import PHYSICS_CHECKS
from .srepr_safe import UnsafeSreprError, canonicalize_srepr, parse_srepr_safe

CheckFn = Callable[[dict[str, Any], dict[str, Any]], dict[str, Any]]

_CHECK_REGISTRY: dict[str, CheckFn] = {}
_PACKAGE_DIR = Path(__file__).resolve().parent


class UnknownCheckError(KeyError):
    """Raised when a caller requests a check name that is not in the host registry."""


def register_check(name: str, fn: CheckFn) -> None:
    """Register a host-shipped check function by canonical name."""
    if not isinstance(name, str) or not name or name.startswith("_"):
        raise ValueError(f"invalid check name: {name!r}")
    _CHECK_REGISTRY[name] = fn


def list_checks() -> list[str]:
    """Return sorted names of all currently registered host checks."""
    return sorted(_CHECK_REGISTRY)


def canonicalize_expression(srepr_text: str) -> dict[str, Any]:
    """Parse, canonicalize, hash, and render LaTeX for a stored `srepr` string."""
    return canonicalize_srepr(srepr_text)


def registry_sha256() -> str:
    """Compute a deterministic SHA-256 over all non-test source files in `python/checks/`."""
    hasher = hashlib.sha256()
    files = sorted(
        p for p in _PACKAGE_DIR.rglob("*.py")
        if not p.name.startswith("test_") and "__pycache__" not in p.parts
    )
    for path in files:
        rel = path.relative_to(_PACKAGE_DIR).as_posix().encode("utf-8")
        data = path.read_bytes()
        hasher.update(len(rel).to_bytes(8, "little"))
        hasher.update(rel)
        hasher.update(len(data).to_bytes(8, "little"))
        hasher.update(data)
    return hasher.hexdigest()


def run_registered_check(
    name: str,
    handles: dict[str, str],
    params: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Safely parse `handles` via `srepr_safe` and invoke a registered host check."""
    if not isinstance(name, str) or name not in _CHECK_REGISTRY:
        raise UnknownCheckError(f"unknown check: {name!r}")
    if not isinstance(handles, dict):
        raise ValueError("handles must be a dict mapping handle names to srepr strings")
    check_params: dict[str, Any] = {} if params is None else params
    if not isinstance(check_params, dict):
        raise ValueError("params must be a dict")

    parsed: dict[str, Any] = {}
    for key, raw_srepr in handles.items():
        if not isinstance(key, str) or not key:
            raise ValueError("handle keys must be non-empty strings")
        if not isinstance(raw_srepr, str):
            raise UnsafeSreprError(f"handle {key!r} srepr must be a string")
        parsed[key] = parse_srepr_safe(raw_srepr)

    return _CHECK_REGISTRY[name](parsed, check_params)


def _symbolic_zero_smoke(handles: dict[str, Any], _params: dict[str, Any]) -> dict[str, Any]:
    """Minimal S6 pure check proving check-process isolation (rule R6).

    Checks whether `simplify(lhs - rhs)` reduces to exact zero inside the check
    process. Full algebra checks (`identity_symbolic`, `identity_numeric`,
    `series_match`, `vanishes_to_order`, `counterexample_search`) land in S9.
    """
    if "lhs" not in handles or "rhs" not in handles:
        raise ValueError("symbolic_zero_smoke requires 'lhs' and 'rhs' handles")
    diff = sp.simplify(handles["lhs"] - handles["rhs"])
    is_zero = bool(diff == 0)
    return {
        "outcome": "pass" if is_zero else "inconclusive",
        "evidence": {
            "is_zero": is_zero,
            "residual_srepr": sp.srepr(diff),
            "residual_latex": sp.latex(diff),
        },
    }


register_check("symbolic_zero_smoke", _symbolic_zero_smoke)
for _check_name, _check_fn in ALGEBRA_CHECKS.items():
    register_check(_check_name, _check_fn)
for _check_name, _check_fn in PHYSICS_CHECKS.items():
    register_check(_check_name, _check_fn)

__all__ = [
    "UnknownCheckError",
    "UnsafeSreprError",
    "canonicalize_expression",
    "canonicalize_srepr",
    "list_checks",
    "parse_srepr_safe",
    "register_check",
    "registry_sha256",
    "run_registered_check",
]
