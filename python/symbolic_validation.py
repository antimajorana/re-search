"""Assumption-aware checks for SymPy scalar expressions.

Domain analysis covers real continuity in one variable. Exact probes work with
real or complex arguments and retain the original expression on each side.
"""

from collections.abc import Mapping
from itertools import islice

import sympy as sp
from sympy.calculus.util import continuous_domain

_FLAGS = (
    "real", "complex", "integer", "rational", "positive", "negative",
    "nonnegative", "nonpositive", "nonzero", "imaginary", "commutative", "zero", "even", "odd",
)
_BOTH = (-2, -1, 0, sp.Rational(1, 2), 1, 2)
_NONFINITE = (sp.nan, sp.zoo, sp.oo, -sp.oo)


def _expression(value):
    if type(value) in (int, float, complex):
        value = sp.sympify(value)
    if not isinstance(value, sp.Expr):
        raise TypeError("expected a SymPy scalar expression or a Python number")
    return value


def _domain_from_assumptions(symbol):
    if symbol is None or symbol.is_real is not True:
        return None
    if symbol.is_zero is True:
        return sp.FiniteSet(0)
    if symbol.is_even or symbol.is_odd:
        n = sp.Dummy("n", integer=True)
        parity = 1 if symbol.is_odd else 0
        domain = sp.ImageSet(sp.Lambda(n, 2 * n + parity), sp.S.Integers)
    elif symbol.is_integer:
        domain = sp.S.Integers
    elif symbol.is_rational:
        domain = sp.S.Rationals
    elif symbol.is_rational is False:
        domain = sp.S.Reals - sp.S.Rationals
    else:
        domain = sp.S.Reals
    if symbol.is_integer is False and symbol.is_rational is not False:
        domain -= sp.S.Integers
    if symbol.is_positive:
        return domain.intersect(sp.Interval.open(0, sp.oo))
    if symbol.is_negative:
        return domain.intersect(sp.Interval.open(-sp.oo, 0))
    if symbol.is_nonnegative:
        return domain.intersect(sp.Interval(0, sp.oo))
    if symbol.is_nonpositive:
        return domain.intersect(sp.Interval(-sp.oo, 0))
    if symbol.is_nonzero:
        return domain - sp.FiniteSet(0)
    return domain


def _compare(lhs, rhs, *, allow_infinite=False):
    if lhs.has(sp.Limit) or rhs.has(sp.Limit) or lhs.free_symbols or rhs.free_symbols:
        return "undetermined"
    if lhs.has(*_NONFINITE) or rhs.has(*_NONFINITE):
        if allow_infinite:
            if lhs in (sp.oo, -sp.oo) and rhs in (sp.oo, -sp.oo):
                return "agreement_infinite" if lhs == rhs else "counterexample"
            if (lhs in (sp.oo, -sp.oo) and rhs.is_finite is True
                    or rhs in (sp.oo, -sp.oo) and lhs.is_finite is True):
                return "counterexample"
            return "undetermined"
        return "undefined"
    if lhs.is_finite is not True or rhs.is_finite is not True or lhs.has(sp.Float) or rhs.has(sp.Float):
        return "undetermined"
    difference = sp.simplify(lhs - rhs)
    if difference.is_zero is True:
        return "agreement"
    return "counterexample" if difference.is_zero is False else "undetermined"


def _real_domain(lhs, rhs, variable, requested, effective):
    report = {
        "status": "undeclared" if requested is None else "undetermined",
        "requested": None if requested is None else str(requested),
        "effective": None if effective is None else str(effective),
        "common": None,
    }
    if variable is None or effective is None or effective.is_subset(sp.S.Reals) is not True:
        return report, None
    try:
        common = sp.Intersection(
            effective,
            continuous_domain(lhs, variable, sp.S.Reals),
            continuous_domain(rhs, variable, sp.S.Reals),
        )
    except (NotImplementedError, ValueError):
        return report, None
    report["common"] = str(common)
    if common == effective or effective.is_subset(common) is True:
        report["status"] = "complete"
    elif (effective - common).is_empty is False:
        report["status"] = "restricted"
    return report, common


def _refine_on_domain(expr, variable, domain):
    if domain is None or variable is None or domain.is_subset(sp.S.Reals) is not True:
        return expr
    for interval, test in (
        (sp.Interval.open(0, sp.oo), sp.Q.positive),
        (sp.Interval.open(-sp.oo, 0), sp.Q.negative),
        (sp.Interval(0, sp.oo), sp.Q.nonnegative),
        (sp.Interval(-sp.oo, 0), sp.Q.nonpositive),
    ):
        if domain.is_subset(interval) is True:
            return sp.refine(expr, test(variable) & sp.Q.real(variable))
    return sp.refine(expr, sp.Q.real(variable))


def _point_report(lhs, rhs, bindings, symbols, variable, domain, *, automatic=False):
    if not isinstance(bindings, Mapping):
        if variable is None:
            raise ValueError("scalar sample points require one declared or inferred variable")
        bindings = {variable: bindings}
    if set(bindings) != symbols:
        raise ValueError("each sample must assign all variables by SymPy Symbol")
    values = {key: _expression(value) for key, value in bindings.items()}
    if any(value.is_finite is not True or value.free_symbols or value.has(sp.Float) for value in values.values()):
        raise ValueError("sample values must be exact finite numbers, such as Rational(1, 2)")
    report = {
        "point": ", ".join(f"{key} = {values[key]}" for key in sorted(values, key=sp.default_sort_key)),
        "lhs": None,
        "rhs": None,
        "status": "undetermined",
    }
    if variable in values and domain is not None:
        membership = domain.contains(values[variable])
        if membership is not sp.S.true:
            if membership is sp.S.false:
                report["status"] = "outside_domain"
            return None if automatic else report
    for key, value in values.items():
        for flag in _FLAGS:
            want = getattr(key, f"is_{flag}")
            has = getattr(value, f"is_{flag}")
            if want is not None and want != has:
                report["status"] = "undetermined" if has is None else "assumption_mismatch"
                return None if automatic else report

    left, right = lhs.subs(values, simultaneous=True), rhs.subs(values, simultaneous=True)
    report.update(lhs=str(left), rhs=str(right), status=_compare(left, right))
    return report


def _one_sided_limit(lhs, rhs, variable, domain, point, direction):
    point = _expression(point)
    if (direction not in ("+", "-") or point.is_extended_real is not True
            or point.free_symbols or point.has(sp.Float)):
        raise ValueError("supply a real limit point (or ±oo) and direction '+' or '-'")
    report = {"point": str(point), "direction": direction, "lhs": None, "rhs": None, "status": "undetermined"}
    if variable is None or domain is None or domain.is_subset(sp.S.Reals) is not True:
        return report
    if point == sp.oo:
        direction = "-"
    elif point == -sp.oo:
        direction = "+"
    report["direction"] = direction
    neighborhood = sp.Interval.open(point, sp.oo) if direction == "+" else sp.Interval.open(-sp.oo, point)
    try:
        reachable = domain.intersect(neighborhood)
        if reachable.is_empty is True:
            report["status"] = "outside_domain"
            return report
        boundary = reachable.inf if direction == "+" else reachable.sup
    except (NotImplementedError, ValueError):
        return report
    at_boundary = sp.Eq(boundary, point)
    if at_boundary is sp.S.false:
        report["status"] = "outside_domain"
        return report
    if at_boundary is not sp.S.true:
        return report
    try:
        left = sp.limit(lhs, variable, point, dir=direction)
        right = sp.limit(rhs, variable, point, dir=direction)
    except (NotImplementedError, ValueError):
        return report
    report.update(lhs=str(left), rhs=str(right), status=_compare(left, right, allow_infinite=True))
    return report


class _UnknownDimension(ValueError):
    pass


class _DimensionMismatch(ValueError):
    pass


def _dimension_map(dimensions):
    if not isinstance(dimensions, Mapping):
        raise TypeError("dimensions must map SymPy Symbols to base-dimension exponents")
    parsed = {}
    for symbol, powers in dimensions.items():
        if not isinstance(symbol, sp.Symbol) or not isinstance(powers, Mapping):
            raise TypeError("each dimension must map a SymPy Symbol to its base-dimension powers")
        parsed[symbol] = {}
        for base, power in powers.items():
            if not isinstance(base, str) or not base.strip():
                raise ValueError("base dimensions must have nonempty names")
            exponent = _expression(power)
            if not isinstance(exponent, sp.Rational):
                raise ValueError("dimension powers must be exact rational numbers")
            if exponent:
                parsed[symbol][base] = exponent
    return parsed


def _dimension_of(expr, dimensions):
    if expr in dimensions:
        return dimensions[expr]
    if isinstance(expr, sp.Symbol):
        raise _UnknownDimension(f"dimension of {expr} is unspecified")
    if isinstance(expr, (sp.Number, sp.NumberSymbol)) or expr == sp.I:
        if expr.has(*_NONFINITE):
            raise _UnknownDimension(f"{expr} has no finite dimension")
        return {}
    if isinstance(expr, sp.Mul):
        total = {}
        for term in expr.args:
            for base, exponent in _dimension_of(term, dimensions).items():
                total[base] = total.get(base, 0) + exponent
        return {base: exponent for base, exponent in total.items() if exponent}
    if isinstance(expr, sp.Add):
        parts = [_dimension_of(term, dimensions) for term in expr.args if term != 0]
        if parts and any(part != parts[0] for part in parts[1:]):
            raise _DimensionMismatch(f"incompatible dimensions in sum {expr}")
        return parts[0] if parts else {}
    if isinstance(expr, sp.Pow):
        base, exponent = expr.args
        if _dimension_of(exponent, dimensions):
            raise _DimensionMismatch(f"exponent {exponent} must be dimensionless")
        base_dim = _dimension_of(base, dimensions)
        if base_dim and not isinstance(exponent, sp.Rational):
            raise _UnknownDimension("a dimensional base requires a fixed rational power")
        return {key: power * exponent for key, power in base_dim.items() if power * exponent}
    if expr.func in (sp.Abs, sp.conjugate, sp.re, sp.im):
        return _dimension_of(expr.args[0], dimensions)
    if expr.func in (sp.exp, sp.log, sp.sin, sp.cos, sp.tan, sp.asin, sp.acos, sp.atan,
                     sp.sinh, sp.cosh, sp.tanh):
        if any(_dimension_of(arg, dimensions) for arg in expr.args):
            raise _DimensionMismatch(f"{expr.func} requires dimensionless arguments")
        return {}
    raise _UnknownDimension(f"dimension rule unavailable for {expr.func}")


def _dimensional_report(lhs, rhs, dimensions):
    if dimensions is None:
        return None
    declared = _dimension_map(dimensions)
    report = {"status": "undetermined", "lhs": None, "rhs": None, "error": None}
    try:
        left, right = _dimension_of(lhs, declared), _dimension_of(rhs, declared)
    except _DimensionMismatch as error:
        return dict(report, status="mismatch", error=str(error))
    except _UnknownDimension as error:
        return dict(report, error=str(error))
    if lhs == 0:
        left = right
    elif rhs == 0:
        right = left

    def label(powers):
        return " ".join(f"{key}^{power}" if power != 1 else key for key, power in sorted(powers.items())) or "1"

    return dict(report, status="match" if left == right else "mismatch", lhs=label(left), rhs=label(right))


def check_symbolic_identity(lhs, rhs, *, variable=None, domain=None, samples=(), limits=(), dimensions=None):
    """Record a symbolic residual, exact sample evidence and optional physics checks.

    Pass constructed SymPy expressions. An optional univariate SymPy Set
    constrains real continuity, exact probes, and approached one-sided limits.
    Explicit samples can be exact scalar values or assignments of all symbols.
    Limits are pairs (point, direction). Dimensions map symbols to base powers,
    for example {length: {"L": 1}, speed: {"L": 1, "T": -1}}.
    """
    lhs, rhs = _expression(lhs), _expression(rhs)
    symbols = lhs.free_symbols | rhs.free_symbols
    if not all(isinstance(symbol, sp.Symbol) for symbol in symbols):
        raise ValueError("identities require scalar Symbol parameters")
    if variable is None and len(symbols) == 1:
        variable = next(iter(symbols))
    if variable is not None:
        if not isinstance(variable, sp.Symbol):
            raise TypeError("variable must be a SymPy Symbol")
        symbols.add(variable)
    if domain is not None and (variable is None or not isinstance(domain, sp.Set)):
        raise ValueError("an explicit domain requires a variable and a SymPy Set")
    inferred = _domain_from_assumptions(variable)
    requested = domain if domain is not None else inferred
    effective = requested.intersect(inferred) if requested is not None and inferred is not None else requested
    if effective is not None and effective.is_empty is True:
        raise ValueError("the domain is empty under the variable's assumptions")
    if variable is not None and variable.is_real is False and effective is not None and effective.is_subset(sp.S.Reals) is True:
        raise ValueError("the real domain conflicts with the variable's assumptions")
    domain_report, common = _real_domain(lhs, rhs, variable, requested, effective)

    assumptions = {}
    for symbol in sorted(symbols, key=sp.default_sort_key):
        label = str(symbol)
        if label in assumptions:
            raise ValueError(f"multiple distinct variables have the name {label!r}")
        assumptions[label] = {
            flag: value for flag in _FLAGS
            if (value := getattr(symbol, f"is_{flag}")) is not None
        }

    sample_points = list(islice(samples, 33))
    limit_points = list(islice(limits, 13))
    if len(sample_points) > 32 or len(limit_points) > 12:
        raise ValueError("at most 32 samples and 12 directional limits per check")
    explicit_samples = bool(sample_points)
    if not sample_points and variable is not None and len(symbols) == 1 and effective is not None:
        sample_points = _BOTH if effective.is_subset(sp.S.Reals) is True else (-1, -sp.I, sp.I, 1 + sp.I, 2)
    sample_report = [
        row for point in sample_points
        if (row := _point_report(lhs, rhs, point, symbols, variable, effective, automatic=not explicit_samples)) is not None
    ]
    if not symbols:
        sample_report = [_point_report(lhs, rhs, {}, symbols, variable, effective)]
    limit_domain = common if common is not None else effective
    limit_report = [_one_sided_limit(lhs, rhs, variable, limit_domain, point, direction) for point, direction in limit_points]

    left = _refine_on_domain(lhs, variable, effective)
    right = _refine_on_domain(rhs, variable, effective)
    residual = sp.simplify(left - right)
    dimensions_report = _dimensional_report(lhs, rhs, dimensions)
    status = "undetermined"
    if any(row["status"] == "counterexample" for row in sample_report + limit_report):
        status = "counterexample"
    elif (residual == 0 and not lhs.has(sp.Float, *_NONFINITE) and not rhs.has(sp.Float, *_NONFINITE)
          and common is not sp.S.EmptySet and not (dimensions_report and dimensions_report["status"] == "mismatch")):
        status = "symbolic_agreement"

    return {
        "schema_version": 1,
        "kind": "symbolic_identity",
        "status": status,
        "lhs": str(lhs),
        "rhs": str(rhs),
        "residual": str(residual),
        "assumptions": assumptions,
        "domain": domain_report,
        "samples": sample_report,
        "limits": limit_report,
        "dimensions": dimensions_report,
    }
