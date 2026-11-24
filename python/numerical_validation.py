"""Empirical convergence checks for numerical research calculations."""

import operator
from decimal import Context, Decimal, InvalidOperation, localcontext
from fractions import Fraction
from itertools import islice

from mpmath import mp

MAX_SAMPLES = 32
MAX_WORK_DPS = 256
MAX_LEVEL = 10**9
MAX_EXACT_COMPARISON_EXPONENT = 512


def _positive_integer(value, name, maximum):
    message = f"{name} must be an integer from 1 through {maximum}"
    if isinstance(value, bool):
        raise ValueError(message)
    try:
        integer = operator.index(value)
    except TypeError as error:
        raise ValueError(message) from error
    if not 1 <= integer <= maximum:
        raise ValueError(message)
    return integer


def _scalar(value, name):
    message = f"{name} must be a finite real or complex scalar"
    try:
        if isinstance(value, bool):
            raise TypeError(message)
        if isinstance(value, Fraction):
            number = mp.mpf(value.numerator) / value.denominator
        elif isinstance(value, Decimal):
            number = mp.mpf(str(value))
        else:
            try:
                number = mp.convert(value)
            except TypeError:
                number = mp.mpf(operator.index(value))
    except (TypeError, ValueError, OverflowError) as error:
        raise ValueError(message) from error
    if not isinstance(number, (mp.mpf, mp.mpc)) or not mp.isfinite(number):
        raise ValueError(message)
    return number


def _real(value, name):
    number = _scalar(value, name)
    if isinstance(number, mp.mpc):
        raise ValueError(f"{name} must be real")
    return number


def _portable_value(value, digits):
    if isinstance(value, mp.mpc):
        return {
            "real": mp.nstr(value.real, digits),
            "imag": mp.nstr(value.imag, digits),
        }
    return mp.nstr(value, digits)


def _compare(value, target, atol, rtol, digits, *, adjacent=False):
    scale = max(abs(value), abs(target)) if adjacent else abs(target)
    error = abs(value - target)
    threshold = atol + rtol * scale
    if isinstance(value, mp.mpf) and isinstance(target, mp.mpf):
        # Compare the same claimed decimal values as the recorded report. Binary
        # rounding inside mpmath can flip a result exactly on the tolerance edge.
        try:
            decimals = tuple(Decimal(_portable_value(x, digits)) for x in (value, target, atol, rtol))
        except InvalidOperation:
            pass
        else:
            if all(x.is_finite() and abs(x.adjusted()) <= MAX_EXACT_COMPARISON_EXPONENT for x in decimals):
                a, b, absolute, relative = decimals
                # Accommodate the full decimal span, including the relative product.
                precision = 4 * (MAX_EXACT_COMPARISON_EXPONENT + digits) + 4
                with localcontext(Context(prec=precision)):
                    decimal_scale = max(abs(a), abs(b)) if adjacent else abs(b)
                    return error, threshold, abs(a - b) <= absolute + relative * decimal_scale
    return error, threshold, bool(error <= threshold)


def _tail_verdict(samples, tail, field):
    if len(samples) <= tail:
        return None
    results = [sample[field] for sample in samples[-tail:]]
    return None if None in results else all(results)


def check_convergence(
    run, levels, *, atol, rtol, reference=None, axis="resolution", tail=2, work_dps=None
):
    """Measure whether scalar results stabilize on an increasing sequence of levels.

    run(level) must return a finite real or complex scalar. Pass a scalar observable
    or norm if the calculation produces arrays. Decimal strings, Python numbers,
    and mpmath values can express tolerances or reference values. Positive levels
    must increase strictly. Supply at most 32 levels, with resolution and basis
    levels up to 10**9 and precisions up to 256 decimal digits.

    On the "precision" axis, levels are positive integers in decimal digits and
    mpmath uses each level's precision for its callback. On the "resolution" and
    "basis" axes, callbacks receive the supplied levels and use work_dps or the
    caller's current mpmath precision. work_dps also sets a floor for comparison
    precision. It cannot add precision to a callback's binary64 result.

    Each adjacent tolerance is atol + rtol * max(abs(a), abs(b)). A requested
    reference uses atol + rtol * abs(reference). The final tail consecutive
    differences, and the latest tail reference errors if supplied, must pass.
    Real values in the scalar plan's exponent range use their reported decimals
    at comparison precision. Complex and very large or small values use mpmath.
    Nonfinite outputs and callback exceptions are recorded at their levels.
    The result reports observed behavior over sampled levels. Stability can also
    come from shared bias, precision loss, or pre-asymptotic behavior.
    """
    if not callable(run):
        raise TypeError("run must be callable")
    if axis not in ("resolution", "basis", "precision"):
        raise ValueError("axis must be 'resolution', 'basis', or 'precision'")
    tail = _positive_integer(tail, "tail", MAX_SAMPLES - 1)
    if isinstance(levels, (str, bytes)):
        raise TypeError("levels must be an iterable of numbers")
    levels = tuple(islice(levels, MAX_SAMPLES + 1))
    if not levels:
        raise ValueError("levels must include at least one sample")
    if len(levels) > MAX_SAMPLES:
        raise ValueError(f"levels must contain at most {MAX_SAMPLES} samples")

    if work_dps is None:
        work_dps = min(mp.dps, MAX_WORK_DPS) if axis == "precision" else mp.dps
    requested_dps = _positive_integer(work_dps, "work_dps", MAX_WORK_DPS)
    if axis == "precision":
        levels = tuple(_positive_integer(level, "precision level", MAX_WORK_DPS) for level in levels)
        sample_precisions = levels
    else:
        sample_precisions = (requested_dps,) * len(levels)
    analysis_dps = max(requested_dps, max(sample_precisions)) + 5

    with mp.workdps(analysis_dps):
        numeric_levels = tuple(_real(level, "level") for level in levels)
        if numeric_levels[0] <= 0 or any(a >= b for a, b in zip(numeric_levels, numeric_levels[1:])):
            raise ValueError("levels must be positive and strictly increasing")
        if numeric_levels[-1] > MAX_LEVEL:
            raise ValueError(f"levels must be at most {MAX_LEVEL}")
        atol = _real(atol, "atol")
        rtol = _real(rtol, "rtol")
        if atol < 0 or rtol < 0:
            raise ValueError("atol and rtol must be nonnegative")
        reference = None if reference is None else _scalar(reference, "reference")
        def encode(value):
            return None if value is None else _portable_value(value, analysis_dps)

        samples = []
        previous = None
        for level, numeric_level, dps in zip(levels, numeric_levels, sample_precisions):
            value = None
            error = None
            try:
                with mp.workdps(dps):
                    observed = run(level)
                value = _scalar(observed, "calculation result")
            except Exception as exception:
                error = {"type": type(exception).__name__, "message": str(exception)[:512]}

            delta = threshold = passes = None
            if value is not None and previous is not None:
                delta, threshold, passes = _compare(value, previous, atol, rtol, analysis_dps, adjacent=True)
            reference_error = reference_limit = reference_passes = None
            if reference is not None and value is not None:
                reference_error, reference_limit, reference_passes = _compare(
                    value, reference, atol, rtol, analysis_dps
                )
            samples.append({
                "level": level if axis == "precision" else encode(numeric_level),
                "work_dps": dps,
                "value": encode(value),
                "delta": encode(delta),
                "threshold": encode(threshold),
                "passes": passes,
                "reference_error": encode(reference_error),
                "reference_threshold": encode(reference_limit),
                "reference_passes": reference_passes,
                "error": error,
            })
            previous = value

        stable = _tail_verdict(samples, tail, "passes")
        reference_within_tolerance = (
            None if reference is None else _tail_verdict(samples, tail, "reference_passes")
        )
        within_tolerance = None
        if any(sample["error"] is not None for sample in samples):
            status = "evaluation_failed"
        elif stable is None:
            status = "insufficient_data"
        else:
            if not stable:
                status = "not_stable"
            elif reference_within_tolerance is False:
                status = "outside_reference_tolerance"
            else:
                status = "within_tolerance"
            within_tolerance = status == "within_tolerance"

        return {
            "kind": "numerical_convergence",
            "schema_version": 1,
            "axis": axis,
            "status": status,
            "stable": stable,
            "within_tolerance": within_tolerance,
            "reference_within_tolerance": reference_within_tolerance,
            "tail": tail,
            "analysis_dps": analysis_dps,
            "atol": encode(atol),
            "rtol": encode(rtol),
            "reference": encode(reference),
            "samples": samples,
        }
