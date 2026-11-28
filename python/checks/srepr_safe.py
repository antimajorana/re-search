"""Restricted AST-based SymPy `srepr` parser and canonicalizer (Solve package S6).

Reconstructs SymPy expressions ONLY by walking `ast.parse(text, mode="eval")` and
calling an explicit allowlist of pure SymPy constructors with validated arguments.
Never uses `eval`, `exec`, `sympify` on strings, attribute access, comprehensions,
lambdas, subscripting, or input-driven imports.
"""

from __future__ import annotations

import ast
import hashlib
import math
import re
from typing import Any, Callable

import sympy as sp
from sympy.core.symbol import Str as SympyStr
from sympy.functions.elementary.piecewise import ExprCondPair
from sympy.functions.special.hyper import TupleArg

MAX_SREPR_CHARS = 256_000
MAX_AST_DEPTH = 64
MAX_AST_NODES = 10_000
MAX_INT_BITS = 4096
MAX_EXPONENT_ABS = 10_000
MAX_FLOAT_PRECISION = 512
MAX_FLOAT_DPS = 150
MAX_STR_LEN = 256
MAX_COLLECTION_LEN = 1_024
MAX_MATRIX_DIM = 64
MAX_MATRIX_ELEMS = 4_096
MAX_STR_PREVIEW_CHARS = 400

_FLOAT_STR_RE = re.compile(r"^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE]([+-]?\d{1,5}))?$")
_POLY_ALLOWED_DOMAINS = frozenset({"ZZ", "QQ", "RR", "CC", "EX", "ZZ_I", "QQ_I"})
_LIMIT_DIRS = frozenset({"+", "-", "+-"})


class UnsafeSreprError(ValueError):
    """Raised when an `srepr` string violates the restricted parser allowlist or caps."""

    def __init__(self, reason: str, rejected_name: str | None = None) -> None:
        super().__init__(reason)
        self.reason = reason
        self.rejected_name = rejected_name


ALLOWED_ASSUMPTION_KWARGS: frozenset[str] = frozenset({
    "algebraic", "antihermitian", "commutative", "complex", "composite", "even",
    "extended_negative", "extended_nonnegative", "extended_nonpositive",
    "extended_nonzero", "extended_positive", "extended_real", "finite",
    "hermitian", "imaginary", "infinite", "integer", "irrational", "negative",
    "nonnegative", "nonpositive", "nonzero", "odd", "polar", "positive", "prime",
    "rational", "real", "transcendental", "zero",
})

ALLOWED_SINGLETONS: dict[str, Any] = {
    "oo": sp.oo, "zoo": sp.zoo, "nan": sp.nan, "pi": sp.pi, "E": sp.E, "I": sp.I,
    "EulerGamma": sp.EulerGamma, "Catalan": sp.Catalan, "GoldenRatio": sp.GoldenRatio,
    "TribonacciConstant": sp.S.TribonacciConstant, "true": sp.S.true, "false": sp.S.false,
    "EmptySet": sp.S.EmptySet, "UniversalSet": sp.S.UniversalSet, "Reals": sp.S.Reals,
    "Integers": sp.S.Integers, "Rationals": sp.S.Rationals, "Naturals": sp.S.Naturals,
    "Naturals0": sp.S.Naturals0, "Complexes": sp.S.Complexes,
}

_EVALUATE_KWARG_CONSTRUCTORS: frozenset[str] = frozenset({
    "Add", "Mul", "Pow", "Eq", "Equality", "Ne", "Unequality",
    "Lt", "StrictLessThan", "Le", "LessThan", "Gt", "StrictGreaterThan", "Ge", "GreaterThan",
    "And", "Or", "Not", "Xor", "Nand", "Nor", "Implies", "Equivalent",
    "Piecewise", "Union", "Intersection", "Complement", "SymmetricDifference", "Interval",
    "Derivative", "Integral", "Sum", "Product", "Limit", "Subs",
    "MatAdd", "MatMul", "MatPow",
})

_SYMBOL_LIKE_CONSTRUCTORS: frozenset[str] = frozenset({"Symbol", "Dummy", "Wild"})

_STATIC_SYMPY_CONSTRUCTORS: tuple[str, ...] = (
    # Core arithmetic & numbers
    "Add", "Mul", "Pow", "Integer", "Rational", "Float", "UnevaluatedExpr",
    # Symbols & tuples
    "Symbol", "Dummy", "Wild", "Tuple",
    # Calculus & series
    "Derivative", "Integral", "Sum", "Product", "Limit", "Order", "Subs",
    # Relations, logic, piecewise, sets
    "Eq", "Equality", "Ne", "Unequality", "Lt", "StrictLessThan", "Le", "LessThan",
    "Gt", "StrictGreaterThan", "Ge", "GreaterThan", "Piecewise", "And", "Or", "Not",
    "Xor", "Nand", "Nor", "Implies", "Equivalent", "FiniteSet", "Interval", "Union",
    "Intersection", "Complement", "SymmetricDifference", "ConditionSet", "ProductSet",
    # Polynomials & roots
    "Poly", "ComplexRootOf", "CRootOf", "RootOf",
    # Matrices
    "ImmutableDenseMatrix", "MutableDenseMatrix", "ImmutableSparseMatrix", "MutableSparseMatrix",
    "MatrixSymbol", "Identity", "ZeroMatrix", "OneMatrix", "MatAdd", "MatMul", "MatPow",
    "Transpose", "Adjoint", "Trace", "Determinant", "Inverse", "HadamardProduct", "KroneckerProduct",
    # Indexed / tensor indices / deltas
    "Indexed", "IndexedBase", "Idx", "KroneckerDelta", "LeviCivita",
    # Elementary functions
    "sin", "cos", "tan", "cot", "sec", "csc", "asin", "acos", "atan", "atan2", "acot", "asec", "acsc",
    "sinh", "cosh", "tanh", "coth", "sech", "csch", "asinh", "acosh", "atanh", "acoth", "asech", "acsch",
    "exp", "log", "Abs", "sign", "re", "im", "conjugate", "arg", "floor", "ceiling",
    "Mod", "Min", "Max", "sqrt", "cbrt", "Heaviside", "DiracDelta",
    # Special functions & combinatorics
    "gamma", "loggamma", "lowergamma", "uppergamma", "digamma", "polygamma", "trigamma", "beta",
    "erf", "erfc", "erfi", "erf2", "zeta", "dirichlet_eta", "polylog", "lerchphi",
    "besselj", "bessely", "besseli", "besselk", "hankel1", "hankel2", "jn", "yn",
    "legendre", "assoc_legendre", "hermite", "laguerre", "assoc_laguerre",
    "chebyshevt", "chebyshevu", "jacobi", "gegenbauer", "Ynm", "Znm", "hyper", "meijerg",
    "elliptic_k", "elliptic_e", "elliptic_f", "elliptic_pi",
    "factorial", "factorial2", "subfactorial", "binomial", "RisingFactorial", "FallingFactorial",
    "LambertW", "Si", "Ci", "Shi", "Chi", "Ei", "li", "expint", "fresnels", "fresnelc",
    "airyai", "airybi", "airyaiprime", "airybiprime",
)

_SINGLETON_THUNKS: dict[str, Callable[[], Any]] = {
    "Half": lambda: sp.S.Half,
    "Zero": lambda: sp.S.Zero,
    "One": lambda: sp.S.One,
    "NegativeOne": lambda: sp.S.NegativeOne,
    "ImaginaryUnit": lambda: sp.S.ImaginaryUnit,
    "Pi": lambda: sp.S.Pi,
    "Exp1": lambda: sp.S.Exp1,
    "EulerGamma": lambda: sp.S.EulerGamma,
    "Catalan": lambda: sp.S.Catalan,
    "GoldenRatio": lambda: sp.S.GoldenRatio,
    "Infinity": lambda: sp.S.Infinity,
    "NegativeInfinity": lambda: sp.S.NegativeInfinity,
    "ComplexInfinity": lambda: sp.S.ComplexInfinity,
    "NaN": lambda: sp.S.NaN,
    "BooleanTrue": lambda: sp.S.true,
    "BooleanFalse": lambda: sp.S.false,
}

ALLOWED_CONSTRUCTORS: dict[str, Callable[..., Any]] = {
    **{name: getattr(sp, name) for name in _STATIC_SYMPY_CONSTRUCTORS},
    **_SINGLETON_THUNKS,
    "Str": SympyStr,
    "TupleArg": TupleArg,
    "ExprCondPair": ExprCondPair,
}


def _check_int(value: Any, *, label: str, max_bits: int = MAX_INT_BITS) -> int:
    if type(value) is not int:
        raise UnsafeSreprError(f"{label} expects an exact int, got {type(value).__name__}")
    if value.bit_length() > max_bits:
        raise UnsafeSreprError(f"{label} integer exceeds {max_bits}-bit cap")
    return value


def _check_name_str(value: Any, *, label: str) -> str:
    if type(value) is not str or not value:
        raise UnsafeSreprError(f"{label} name must be a non-empty string")
    if len(value) > MAX_STR_LEN:
        raise UnsafeSreprError(f"{label} name exceeds {MAX_STR_LEN} characters")
    if value.startswith("__"):
        raise UnsafeSreprError(f"dunder name {value!r} is forbidden", rejected_name=value)
    return value


def _validate_kwargs(ctor: str, kwargs: dict[str, Any]) -> None:
    for key, val in kwargs.items():
        if key.startswith("_"):
            raise UnsafeSreprError(f"dunder/private kwarg {key!r} is forbidden", rejected_name=key)
        if ctor in _SYMBOL_LIKE_CONSTRUCTORS or ctor == "Function":
            if key == "dummy_index" and ctor == "Dummy":
                _check_int(val, label="Dummy.dummy_index", max_bits=64)
            elif key in ALLOWED_ASSUMPTION_KWARGS:
                if type(val) is not bool:
                    raise UnsafeSreprError(f"{ctor}.{key} must be bool, got {type(val).__name__}")
            else:
                raise UnsafeSreprError(f"disallowed kwarg {key!r} for {ctor}", rejected_name=key)
        elif ctor == "Float":
            if key == "precision":
                prec = _check_int(val, label="Float.precision", max_bits=32)
                if not 1 <= prec <= MAX_FLOAT_PRECISION:
                    raise UnsafeSreprError(f"Float precision {prec} exceeds cap {MAX_FLOAT_PRECISION}")
            elif key == "dps":
                dps = _check_int(val, label="Float.dps", max_bits=32)
                if not 1 <= dps <= MAX_FLOAT_DPS:
                    raise UnsafeSreprError(f"Float dps {dps} exceeds cap {MAX_FLOAT_DPS}")
            else:
                raise UnsafeSreprError(f"disallowed kwarg {key!r} for Float", rejected_name=key)
        elif ctor == "Poly":
            if key == "domain":
                if type(val) is not str or val not in _POLY_ALLOWED_DOMAINS:
                    raise UnsafeSreprError(f"disallowed Poly domain {val!r}", rejected_name=str(val))
            else:
                raise UnsafeSreprError(f"disallowed kwarg {key!r} for Poly", rejected_name=key)
        elif ctor in _EVALUATE_KWARG_CONSTRUCTORS and key == "evaluate":
            if type(val) is not bool:
                raise UnsafeSreprError(f"{ctor}.evaluate must be bool")
        else:
            raise UnsafeSreprError(f"disallowed kwarg {key!r} for {ctor}", rejected_name=key)


def _forbid_raw_strings(value: Any, *, ctor: str) -> None:
    if type(value) is str:
        raise UnsafeSreprError(
            f"raw string argument is forbidden in {ctor} (prevents string sympify)",
            rejected_name=ctor,
        )
    if isinstance(value, (list, tuple)):
        for item in value:
            _forbid_raw_strings(item, ctor=ctor)


def _guard_call(ctor: str, args: list[Any], kwargs: dict[str, Any]) -> None:
    _validate_kwargs(ctor, kwargs)
    if ctor == "Integer":
        if len(args) != 1:
            raise UnsafeSreprError("Integer expects exactly 1 positional argument")
        _check_int(args[0], label="Integer")
    elif ctor == "Rational":
        if len(args) not in (1, 2):
            raise UnsafeSreprError("Rational expects 1 or 2 integer arguments")
        _check_int(args[0], label="Rational.p")
        if len(args) == 2 and _check_int(args[1], label="Rational.q") == 0:
            raise UnsafeSreprError("Rational denominator cannot be zero")
    elif ctor == "Float":
        if not 1 <= len(args) <= 2:
            raise UnsafeSreprError("Float expects 1 or 2 positional arguments")
        first = args[0]
        if type(first) is str:
            if len(first) > MAX_STR_LEN:
                raise UnsafeSreprError("Float string literal exceeds length cap")
            m = _FLOAT_STR_RE.match(first)
            if not m:
                raise UnsafeSreprError(f"invalid Float literal {first!r}")
            if m.group(1) is not None and abs(int(m.group(1))) > MAX_EXPONENT_ABS:
                raise UnsafeSreprError("Float exponent exceeds cap")
        elif type(first) is float:
            if not math.isfinite(first):
                raise UnsafeSreprError("Float must be finite")
        elif type(first) is int:
            _check_int(first, label="Float")
        else:
            raise UnsafeSreprError(f"unsupported Float argument type {type(first).__name__}")
        if len(args) == 2:
            dps = _check_int(args[1], label="Float.dps", max_bits=32)
            if not 1 <= dps <= MAX_FLOAT_DPS:
                raise UnsafeSreprError(f"Float dps {dps} exceeds cap {MAX_FLOAT_DPS}")
    elif ctor in ("Pow", "MatPow"):
        if len(args) != 2:
            raise UnsafeSreprError(f"{ctor} expects 2 positional arguments")
        _forbid_raw_strings(args, ctor=ctor)
        base, exp = args
        if isinstance(exp, (sp.Integer, int)) and type(exp) is not bool:
            e = int(exp)
            if abs(e) > MAX_EXPONENT_ABS:
                raise UnsafeSreprError(f"{ctor} exponent {e} exceeds cap {MAX_EXPONENT_ABS}")
            if ctor == "Pow" and isinstance(base, (sp.Integer, int)) and type(base) is not bool:
                b = abs(int(base))
                if b > 1 and e > 0 and e * b.bit_length() > MAX_INT_BITS:
                    raise UnsafeSreprError(f"Pow result exceeds {MAX_INT_BITS}-bit cap")
        elif isinstance(exp, sp.Rational):
            if abs(int(exp.p)) > MAX_EXPONENT_ABS or abs(int(exp.q)) > MAX_EXPONENT_ABS:
                raise UnsafeSreprError(f"{ctor} rational exponent exceeds cap {MAX_EXPONENT_ABS}")
    elif ctor in _SYMBOL_LIKE_CONSTRUCTORS or ctor == "Str":
        if len(args) != 1:
            raise UnsafeSreprError(f"{ctor} expects 1 positional argument")
        _check_name_str(args[0], label=ctor)
    elif ctor == "Limit":
        for idx, a in enumerate(args):
            if idx == 3 and type(a) is str:
                if a not in _LIMIT_DIRS:
                    raise UnsafeSreprError(f"invalid Limit direction {a!r}")
            else:
                _forbid_raw_strings(a, ctor=ctor)
    elif ctor == "UnevaluatedExpr":
        if len(args) != 1:
            raise UnsafeSreprError("UnevaluatedExpr expects 1 positional argument")
        _forbid_raw_strings(args, ctor=ctor)
    else:
        _forbid_raw_strings(args, ctor=ctor)
        if ctor in ("factorial", "factorial2", "subfactorial", "binomial", "RisingFactorial", "FallingFactorial"):
            for a in args:
                if isinstance(a, (sp.Integer, int)) and type(a) is not bool and abs(int(a)) > MAX_EXPONENT_ABS:
                    raise UnsafeSreprError(f"{ctor} integer argument exceeds cap {MAX_EXPONENT_ABS}")
        elif ctor in ("Identity", "ZeroMatrix", "OneMatrix"):
            for a in args:
                if isinstance(a, (sp.Integer, int)) and type(a) is not bool and not 0 <= int(a) <= MAX_MATRIX_DIM:
                    raise UnsafeSreprError(f"{ctor} dimension exceeds cap {MAX_MATRIX_DIM}")
        elif ctor in ("ImmutableDenseMatrix", "MutableDenseMatrix", "ImmutableSparseMatrix", "MutableSparseMatrix"):
            if args and isinstance(args[0], (list, tuple)):
                rows = args[0]
                if len(rows) > MAX_MATRIX_DIM:
                    raise UnsafeSreprError(f"{ctor} row count exceeds cap {MAX_MATRIX_DIM}")
                total = 0
                for row in rows:
                    if not isinstance(row, (list, tuple)):
                        raise UnsafeSreprError(f"{ctor} rows must be lists/tuples")
                    if len(row) > MAX_MATRIX_DIM:
                        raise UnsafeSreprError(f"{ctor} col count exceeds cap {MAX_MATRIX_DIM}")
                    total += len(row)
                if total > MAX_MATRIX_ELEMS:
                    raise UnsafeSreprError(f"{ctor} element count exceeds cap {MAX_MATRIX_ELEMS}")


_PRESERVE_EVAL_CTORS: frozenset[str] = frozenset({
    "Add", "Mul", "Pow", "Eq", "Equality", "Ne", "Unequality",
    "Lt", "StrictLessThan", "Le", "LessThan", "Gt", "StrictGreaterThan", "Ge", "GreaterThan",
    "MatAdd", "MatMul", "MatPow", "sqrt",
})


class _SafeEvaluator:
    def __init__(self) -> None:
        self.nodes = 0
        self._uneval_depth = 0

    def eval_node(self, node: ast.AST, depth: int) -> Any:
        self.nodes += 1
        if self.nodes > MAX_AST_NODES:
            raise UnsafeSreprError(f"AST exceeds {MAX_AST_NODES} node cap")
        if depth > MAX_AST_DEPTH:
            raise UnsafeSreprError(f"AST exceeds max nesting depth {MAX_AST_DEPTH}")

        if isinstance(node, ast.Constant):
            val = node.value
            if val is None or type(val) is bool:
                return val
            if type(val) is int:
                return _check_int(val, label="literal")
            if type(val) is float:
                if not math.isfinite(val):
                    raise UnsafeSreprError("non-finite float literal is forbidden")
                return val
            if type(val) is str:
                if len(val) > MAX_STR_LEN:
                    raise UnsafeSreprError(f"string literal exceeds {MAX_STR_LEN} char cap")
                return val
            raise UnsafeSreprError(f"disallowed literal type {type(val).__name__}")

        if isinstance(node, ast.UnaryOp):
            if not isinstance(node.op, (ast.UAdd, ast.USub)):
                op_name = type(node.op).__name__
                raise UnsafeSreprError(f"disallowed unary operator {op_name}", rejected_name=op_name)
            operand = self.eval_node(node.operand, depth + 1)
            if type(operand) in (int, float):
                return +operand if isinstance(node.op, ast.UAdd) else -operand
            if operand is sp.oo:
                return sp.oo if isinstance(node.op, ast.UAdd) else -sp.oo
            raise UnsafeSreprError("unary +/- allowed only on numeric literals or oo")

        if isinstance(node, ast.Name):
            name = node.id
            if name.startswith("__"):
                raise UnsafeSreprError(f"dunder name {name!r} is forbidden", rejected_name=name)
            if name in ALLOWED_SINGLETONS:
                return ALLOWED_SINGLETONS[name]
            raise UnsafeSreprError(f"disallowed name {name!r}", rejected_name=name)

        if isinstance(node, (ast.Tuple, ast.List)):
            if len(node.elts) > MAX_COLLECTION_LEN:
                raise UnsafeSreprError(f"collection exceeds {MAX_COLLECTION_LEN} element cap")
            items = [self.eval_node(elt, depth + 1) for elt in node.elts]
            return tuple(items) if isinstance(node, ast.Tuple) else items

        if isinstance(node, ast.Call):
            return self._eval_call(node, depth)

        node_name = type(node).__name__
        raise UnsafeSreprError(f"disallowed AST node {node_name}", rejected_name=node_name)

    def _eval_call(self, node: ast.Call, depth: int) -> Any:
        if isinstance(node.func, ast.Call):
            if isinstance(node.func.func, ast.Name) and node.func.func.id == "Function":
                fn_cls = self._build_undefined_function(node.func, depth + 1)
                args, kwargs = self._eval_call_args(node, depth + 1)
                if kwargs:
                    raise UnsafeSreprError("applied Function(...) cannot take keyword arguments")
                if not args:
                    raise UnsafeSreprError("applied Function(...) requires at least one argument")
                _forbid_raw_strings(args, ctor="Function")
                return fn_cls(*args)
            raise UnsafeSreprError("nested call is allowed only for Function('name')(...)", rejected_name="Call")

        if not isinstance(node.func, ast.Name):
            kind = type(node.func).__name__
            raise UnsafeSreprError(f"disallowed callee {kind}", rejected_name=kind)

        ctor = node.func.id
        if ctor.startswith("__"):
            raise UnsafeSreprError(f"dunder call {ctor!r} is forbidden", rejected_name=ctor)
        if ctor == "Function":
            return self._build_undefined_function(node, depth + 1)
        if ctor not in ALLOWED_CONSTRUCTORS:
            raise UnsafeSreprError(f"disallowed constructor {ctor!r}", rejected_name=ctor)

        if ctor == "UnevaluatedExpr":
            self._uneval_depth += 1
            try:
                args, kwargs = self._eval_call_args(node, depth + 1)
            finally:
                self._uneval_depth -= 1
        else:
            args, kwargs = self._eval_call_args(node, depth + 1)
        _guard_call(ctor, args, kwargs)
        target_cls = ALLOWED_CONSTRUCTORS[ctor]
        if self._uneval_depth > 0 and "evaluate" not in kwargs:
            from sympy.core.function import FunctionClass

            if ctor in _PRESERVE_EVAL_CTORS or isinstance(target_cls, FunctionClass):
                kwargs = {**kwargs, "evaluate": False}
        return target_cls(*args, **kwargs)

    def _build_undefined_function(self, node: ast.Call, depth: int) -> Any:
        args, kwargs = self._eval_call_args(node, depth)
        if len(args) != 1:
            raise UnsafeSreprError("Function expects exactly 1 positional name argument")
        name = _check_name_str(args[0], label="Function")
        _validate_kwargs("Function", kwargs)
        return sp.Function(name, **kwargs)

    def _eval_call_args(self, node: ast.Call, depth: int) -> tuple[list[Any], dict[str, Any]]:
        if len(node.args) > MAX_COLLECTION_LEN:
            raise UnsafeSreprError(f"call argument count exceeds {MAX_COLLECTION_LEN}")
        args: list[Any] = []
        for arg in node.args:
            if isinstance(arg, ast.Starred):
                raise UnsafeSreprError("starred *args unpacking is forbidden", rejected_name="Starred")
            args.append(self.eval_node(arg, depth))
        kwargs: dict[str, Any] = {}
        for kw in node.keywords:
            if kw.arg is None:
                raise UnsafeSreprError("**kwargs unpacking is forbidden", rejected_name="kwargs")
            kwargs[kw.arg] = self.eval_node(kw.value, depth)
        return args, kwargs


def parse_srepr_safe(text: str) -> Any:
    """Safely parse a SymPy `srepr` string using only the allowlisted AST evaluator."""
    if type(text) is not str:
        raise UnsafeSreprError(f"srepr must be str, got {type(text).__name__}")
    if not text or not text.strip():
        raise UnsafeSreprError("srepr text is empty")
    if len(text) > MAX_SREPR_CHARS:
        raise UnsafeSreprError(f"srepr exceeds {MAX_SREPR_CHARS} character cap")
    try:
        tree = ast.parse(text, mode="eval")
    except (SyntaxError, ValueError, RecursionError) as exc:
        raise UnsafeSreprError(f"invalid srepr syntax: {exc}") from exc
    return _SafeEvaluator().eval_node(tree.body, depth=0)


def _extract_free_symbols(obj: Any) -> list[dict[str, Any]]:
    raw = getattr(obj, "free_symbols", None)
    if not raw:
        return []
    specs: list[tuple[str, tuple[str, ...]]] = []
    for sym in raw:
        name = getattr(sym, "name", str(sym))
        gen = getattr(getattr(sym, "_assumptions", None), "generator", None) or {}
        assumptions: list[str] = []
        for key, val in sorted(gen.items()):
            if key == "commutative" and val is True:
                continue
            if val is True:
                assumptions.append(key)
            elif val is False:
                assumptions.append(f"{key}=false")
        specs.append((name, tuple(assumptions)))
    specs.sort()
    return [{"name": name, "assumptions": list(assumptions)} for name, assumptions in specs]


def canonicalize_srepr(text: str) -> dict[str, Any]:
    """Parse `srepr` safely and produce canonical `srepr`, hash, LaTeX, preview, and free symbols."""
    try:
        obj = parse_srepr_safe(text)
    except UnsafeSreprError as exc:
        out: dict[str, Any] = {
            "portable": False, "srepr": "", "srepr_sha256": "",
            "latex": "", "str_preview": "", "free_symbols": [], "error": exc.reason,
        }
        if exc.rejected_name is not None:
            out["rejected_constructor"] = exc.rejected_name
        return out
    except Exception as exc:
        return {
            "portable": False, "srepr": "", "srepr_sha256": "",
            "latex": "", "str_preview": "", "free_symbols": [],
            "error": f"{type(exc).__name__}: {exc}",
        }

    canon = sp.srepr(obj)
    return {
        "portable": True,
        "srepr": canon,
        "srepr_sha256": hashlib.sha256(canon.encode("utf-8")).hexdigest(),
        "latex": sp.latex(obj),
        "str_preview": str(obj)[:MAX_STR_PREVIEW_CHARS],
        "free_symbols": _extract_free_symbols(obj),
    }


__all__ = [
    "ALLOWED_ASSUMPTION_KWARGS", "ALLOWED_CONSTRUCTORS", "ALLOWED_SINGLETONS",
    "MAX_AST_DEPTH", "MAX_AST_NODES", "MAX_EXPONENT_ABS", "MAX_FLOAT_PRECISION",
    "MAX_INT_BITS", "MAX_SREPR_CHARS", "MAX_STR_LEN", "MAX_STR_PREVIEW_CHARS",
    "UnsafeSreprError", "canonicalize_srepr", "parse_srepr_safe",
]
