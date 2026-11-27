//! Self-contained SymPy prelude and check code-cell generator for Solve `.ipynb` exports (S19b).
//!
//! Generated Python cells execute standalone with plain `python3` + `sympy` outside re:Search,
//! using an allowlisted AST `srepr` reconstructor rather than `eval`/`exec`/`sympify` on strings.

use research_tools::solve_model::{Claim, HandleRef, SolveCheck, SolveExpr, SolveSetup};
use serde_json::json;
use std::collections::BTreeMap;
use std::fmt::Write as _;

const PRELUDE_BODY_HEAD: &str = r#"# Self-contained re:Search Solve notebook prelude (runs with plain Python + SymPy).
import ast
import math
import random

import sympy
import sympy as sp
from sympy import *  # noqa: F401,F403

try:
    from sympy.core.symbol import Str as _SympyStr
except ImportError:  # pragma: no cover
    _SympyStr = None

try:
    from sympy.functions.elementary.piecewise import ExprCondPair as _ExprCondPair
except ImportError:  # pragma: no cover
    _ExprCondPair = None

try:
    from sympy.functions.special.hyper import TupleArg as _TupleArg
except ImportError:  # pragma: no cover
    _TupleArg = None

_ALLOWED_ASSUMPTIONS = frozenset({
    "algebraic", "antihermitian", "commutative", "complex", "composite", "even",
    "extended_negative", "extended_nonnegative", "extended_nonpositive",
    "extended_nonzero", "extended_positive", "extended_real", "finite",
    "hermitian", "imaginary", "infinite", "integer", "irrational", "negative",
    "nonnegative", "nonpositive", "nonzero", "odd", "polar", "positive", "prime",
    "rational", "real", "transcendental", "zero",
})

_ALLOWED_SINGLETONS = {
    "oo": sp.oo, "zoo": sp.zoo, "nan": sp.nan, "pi": sp.pi, "E": sp.E, "I": sp.I,
    "EulerGamma": sp.EulerGamma, "Catalan": sp.Catalan, "GoldenRatio": sp.GoldenRatio,
    "TribonacciConstant": sp.S.TribonacciConstant, "true": sp.S.true, "false": sp.S.false,
    "EmptySet": sp.S.EmptySet, "UniversalSet": sp.S.UniversalSet, "Reals": sp.S.Reals,
    "Integers": sp.S.Integers, "Rationals": sp.S.Rationals, "Naturals": sp.S.Naturals,
    "Naturals0": sp.S.Naturals0, "Complexes": sp.S.Complexes,
}

_SINGLETON_THUNKS = {
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

_STATIC_CTORS = (
    "Add", "Mul", "Pow", "Integer", "Rational", "Float",
    "Symbol", "Dummy", "Wild", "Tuple",
    "Derivative", "Integral", "Sum", "Product", "Limit", "Order", "Subs",
    "Eq", "Equality", "Ne", "Unequality", "Lt", "StrictLessThan", "Le", "LessThan",
    "Gt", "StrictGreaterThan", "Ge", "GreaterThan", "Piecewise", "And", "Or", "Not",
    "Xor", "Nand", "Nor", "Implies", "Equivalent", "FiniteSet", "Interval", "Union",
    "Intersection", "Complement", "SymmetricDifference", "ConditionSet", "ProductSet",
    "Poly", "ComplexRootOf", "CRootOf", "RootOf",
    "ImmutableDenseMatrix", "MutableDenseMatrix", "ImmutableSparseMatrix", "MutableSparseMatrix",
    "MatrixSymbol", "Identity", "ZeroMatrix", "OneMatrix", "MatAdd", "MatMul", "MatPow",
    "Transpose", "Adjoint", "Trace", "Determinant", "Inverse", "HadamardProduct", "KroneckerProduct",
    "Indexed", "IndexedBase", "Idx", "KroneckerDelta", "LeviCivita",
    "sin", "cos", "tan", "cot", "sec", "csc", "asin", "acos", "atan", "atan2", "acot", "asec", "acsc",
    "sinh", "cosh", "tanh", "coth", "sech", "csch", "asinh", "acosh", "atanh", "acoth", "asech", "acsch",
    "exp", "log", "Abs", "sign", "re", "im", "conjugate", "arg", "floor", "ceiling",
    "Mod", "Min", "Max", "sqrt", "cbrt", "Heaviside", "DiracDelta",
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

_ALLOWED_CTORS = {
    **{n: getattr(sp, n) for n in _STATIC_CTORS if hasattr(sp, n)},
    **_SINGLETON_THUNKS,
}
if _SympyStr is not None:
    _ALLOWED_CTORS["Str"] = _SympyStr
if _TupleArg is not None:
    _ALLOWED_CTORS["TupleArg"] = _TupleArg
if _ExprCondPair is not None:
    _ALLOWED_CTORS["ExprCondPair"] = _ExprCondPair
"#;

const PRELUDE_BODY_TAIL: &str = r#"
def _eval_srepr_node(node, depth=0):
    if depth > 64:
        raise ValueError("srepr AST depth exceeded")
    if isinstance(node, ast.Constant):
        v = node.value
        if v is None or isinstance(v, (bool, int, str)):
            return v
        if isinstance(v, float) and math.isfinite(v):
            return v
        raise ValueError(f"disallowed constant in srepr: {type(v).__name__}")
    if isinstance(node, ast.UnaryOp) and isinstance(node.op, (ast.UAdd, ast.USub)):
        val = _eval_srepr_node(node.operand, depth + 1)
        if isinstance(val, (int, float)) or val is sp.oo:
            return +val if isinstance(node.op, ast.UAdd) else -val
        raise ValueError("unary +/- only allowed on numbers or oo")
    if isinstance(node, ast.Name):
        if node.id.startswith("__") or node.id not in _ALLOWED_SINGLETONS:
            raise ValueError(f"disallowed name in srepr: {node.id!r}")
        return _ALLOWED_SINGLETONS[node.id]
    if isinstance(node, (ast.Tuple, ast.List)):
        items = [_eval_srepr_node(e, depth + 1) for e in node.elts]
        return tuple(items) if isinstance(node, ast.Tuple) else items
    if isinstance(node, ast.Call):
        if (
            isinstance(node.func, ast.Call)
            and isinstance(node.func.func, ast.Name)
            and node.func.func.id == "Function"
        ):
            fn = _eval_srepr_call(node.func, depth + 1)
            args = [_eval_srepr_node(a, depth + 1) for a in node.args]
            return fn(*args)
        return _eval_srepr_call(node, depth)
    raise ValueError(f"disallowed AST node in srepr: {type(node).__name__}")


def _eval_srepr_call(node, depth):
    if not isinstance(node.func, ast.Name):
        raise ValueError("disallowed callee in srepr")
    ctor = node.func.id
    if ctor.startswith("__"):
        raise ValueError(f"dunder call forbidden in srepr: {ctor!r}")
    for kw in node.keywords:
        if kw.arg is None or kw.arg.startswith("_"):
            raise ValueError("disallowed keyword unpacking in srepr")
    args = [_eval_srepr_node(a, depth + 1) for a in node.args]
    kwargs = {kw.arg: _eval_srepr_node(kw.value, depth + 1) for kw in node.keywords}
    if ctor == "Function":
        if len(args) != 1 or not isinstance(args[0], str) or args[0].startswith("__"):
            raise ValueError("invalid Function name in srepr")
        for k in kwargs:
            if k not in _ALLOWED_ASSUMPTIONS:
                raise ValueError(f"disallowed Function assumption {k!r}")
        return sp.Function(args[0], **kwargs)
    if ctor not in _ALLOWED_CTORS:
        raise ValueError(f"disallowed constructor in srepr: {ctor!r}")
    if ctor in ("Symbol", "Dummy", "Wild"):
        for k in kwargs:
            if not (k in _ALLOWED_ASSUMPTIONS or (ctor == "Dummy" and k == "dummy_index")):
                raise ValueError(f"disallowed kwarg {k!r} for {ctor}")
    elif ctor == "Float":
        for k in kwargs:
            if k not in ("precision", "dps"):
                raise ValueError(f"disallowed kwarg {k!r} for Float")
    elif ctor == "Poly":
        for k in kwargs:
            if k != "domain":
                raise ValueError(f"disallowed kwarg {k!r} for Poly")
    elif kwargs and list(kwargs.keys()) != ["evaluate"]:
        raise ValueError(f"disallowed kwargs {list(kwargs.keys())} for {ctor}")
    return _ALLOWED_CTORS[ctor](*args, **kwargs)


def _parse_srepr_safe(text):
    tree = ast.parse(text, mode="eval")
    return _eval_srepr_node(tree.body, 0)


_STAGE_IN = {}
_STAGE_OUT = {}


def _stage_handles(handle_map):
    _STAGE_OUT.clear()
    _STAGE_IN.clear()
    for local_name, handle_ref in handle_map.items():
        if handle_ref in _SOLVE_EXPRS:
            _STAGE_IN[local_name] = _SOLVE_EXPRS[handle_ref]


def expr(name_or_handle):
    if not isinstance(name_or_handle, str) or not name_or_handle.strip():
        raise ValueError("expression handle must be a non-empty string")
    if name_or_handle in _STAGE_OUT:
        return _parse_srepr_safe(_STAGE_OUT[name_or_handle])
    if name_or_handle in _STAGE_IN:
        return _parse_srepr_safe(_STAGE_IN[name_or_handle])
    if name_or_handle in _SOLVE_EXPRS:
        return _parse_srepr_safe(_SOLVE_EXPRS[name_or_handle])
    if "@" not in name_or_handle and name_or_handle in _SOLVE_LATEST:
        return _parse_srepr_safe(_SOLVE_EXPRS[_SOLVE_LATEST[name_or_handle]])
    raise KeyError(f"handle {name_or_handle!r} not found in notebook expression table")


def load_expr(name_or_handle):
    return expr(name_or_handle)


def save_expr(name, obj):
    if not isinstance(name, str) or not name.strip():
        raise ValueError("expression name must be a non-empty string")
    encoded = sp.srepr(sp.sympify(obj))
    _STAGE_OUT[name] = encoded
    prefix = f"{name}@"
    versions = [int(k[len(prefix):]) for k in _SOLVE_EXPRS if k.startswith(prefix) and k[len(prefix):].isdigit()]
    next_ver = (max(versions) + 1) if versions else 1
    key = f"{name}@{next_ver}"
    _SOLVE_EXPRS[key] = encoded
    _SOLVE_LATEST[name] = key
    return encoded


def assumed_symbol(name, **assumptions):
    if not assumptions:
        assumptions = {"real": True}
    return sp.Symbol(name, **assumptions)


def set_random_seed(seed):
    random.seed(seed)
    try:
        import numpy as np
        np.random.seed(seed)
    except Exception:
        pass


def commutator(a, b):
    return sp.simplify(a * b - b * a)


def anticommutator(a, b):
    return sp.simplify(a * b + b * a)


def _apply_setup_symbols(obj):
    if not isinstance(obj, sp.Basic) or not _SETUP_SYMBOLS:
        return obj
    subs = {}
    for sym in getattr(obj, "free_symbols", ()):
        name = getattr(sym, "name", None)
        if name in _SETUP_SYMBOLS:
            subs[sym] = sp.Symbol(name, **_SETUP_SYMBOLS[name])
    return obj.subs(subs) if subs else obj
"#;

pub(crate) fn py_str_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let code = c as u32;
                if code <= 0xff {
                    let _ = write!(out, "\\x{code:02x}");
                } else {
                    let _ = write!(out, "\\u{{{code:x}}}");
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub(crate) fn render_prelude_cell(
    setup: Option<&SolveSetup>,
    handles: &BTreeMap<HandleRef, (SolveExpr, String)>,
) -> String {
    let mut out = String::from(PRELUDE_BODY_HEAD);
    out.push('\n');
    out.push_str("_SETUP_SYMBOLS = {\n");
    if let Some(s) = setup {
        for sym in &s.symbols {
            let mut assump_items = Vec::new();
            for raw in &sym.assumptions {
                if let Some((k, v)) = raw.split_once('=') {
                    let b = match v.trim() {
                        "true" | "True" => "True",
                        "false" | "False" => "False",
                        _ => continue,
                    };
                    assump_items.push(format!("{}: {b}", py_str_literal(k.trim())));
                } else if !raw.trim().is_empty() {
                    assump_items.push(format!("{}: True", py_str_literal(raw.trim())));
                }
            }
            let _ = writeln!(
                out,
                "    {}: {{{}}},",
                py_str_literal(&sym.name),
                assump_items.join(", ")
            );
        }
    }
    out.push_str("}\n\n_SOLVE_EXPRS = {\n");
    let mut latest: BTreeMap<String, (u32, String)> = BTreeMap::new();
    for (href, (_, srepr)) in handles {
        let key = href.to_string();
        let _ = writeln!(out, "    {}: {},", py_str_literal(&key), py_str_literal(srepr));
        let replace = latest.get(&href.name).is_none_or(|(v, _)| *v < href.version);
        if replace {
            latest.insert(href.name.clone(), (href.version, key));
        }
    }
    out.push_str("}\n\n_SOLVE_LATEST = {\n");
    for (name, (_, key)) in &latest {
        let _ = writeln!(out, "    {}: {},", py_str_literal(name), py_str_literal(key));
    }
    out.push_str("}\n");
    out.push_str(PRELUDE_BODY_TAIL);
    out
}

pub(crate) fn render_check_code_cell(
    check: &SolveCheck,
    claim: Option<&Claim>,
    handles: &BTreeMap<HandleRef, (SolveExpr, String)>,
    step_id: &str,
) -> String {
    let outcome = serde_json::to_value(check.outcome)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "inconclusive".to_string());
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# Host check `{}` ({}, step `{step_id}`, claim {})",
        check.check_id, check.name, check.claim_index
    );
    let _ = writeln!(out, "_check_id = {}", py_str_literal(&check.check_id));
    let _ = writeln!(out, "_check_name = {}", py_str_literal(&check.name));
    let _ = writeln!(out, "_recorded_outcome = {}", py_str_literal(&outcome));
    let _ = writeln!(out, "_recorded_evidence_sha256 = {}", py_str_literal(&check.evidence_sha256));

    let Some(c) = claim else {
        let _ = writeln!(
            out,
            "print(f\"check {{_check_id}} ({{_check_name}}): recorded={{_recorded_outcome}}\")"
        );
        return out;
    };

    let lhs_ok = handles.contains_key(&c.lhs);
    let rhs_ok = handles.contains_key(&c.rhs);
    if !lhs_ok || !rhs_ok {
        let _ = writeln!(
            out,
            "# Claim handle(s) `{}`, `{}` not portable; printing recorded outcome.",
            c.lhs, c.rhs
        );
        let _ = writeln!(
            out,
            "print(f\"check {{_check_id}} ({{_check_name}}): recorded={{_recorded_outcome}} (non-portable handle)\")"
        );
        return out;
    }

    let _ = writeln!(out, "_lhs = _apply_setup_symbols(expr({}))", py_str_literal(&c.lhs.to_string()));
    let _ = writeln!(out, "_rhs = _apply_setup_symbols(expr({}))", py_str_literal(&c.rhs.to_string()));
    match check.name.as_str() {
        "symbolic_zero_smoke" | "identity_symbolic" => {
            out.push_str(
                "_raw_diff = _lhs - _rhs\n\
                 _diff = sp.simplify(_raw_diff)\n\
                 if _diff != 0 and _diff.has(sp.Integral, sp.Sum, sp.Derivative):\n\
                     _diff = sp.simplify(_raw_diff.doit())\n\
                 print(f\"check {_check_id} ({_check_name}): recorded={_recorded_outcome}, residual={_diff}\")\n",
            );
        }
        "series_match" | "vanishes_to_order" => {
            if let Some(ord) = &c.order {
                let var = py_str_literal(&ord.variable);
                let pt = py_str_literal(&ord.point);
                let _ = writeln!(
                    out,
                    "_var = _apply_setup_symbols(sp.Symbol({var}))\n\
                     _pt = _parse_srepr_safe({pt}) if {pt} in _SOLVE_EXPRS else sp.sympify(0) if {pt} == \"0\" else sp.Symbol({pt})\n\
                     _diff = sp.series((_lhs - _rhs).subs(_var, _var + _pt), _var, 0, {n}).removeO()\n\
                     print(f\"check {{_check_id}} ({{_check_name}}): recorded={{_recorded_outcome}}, series_residual={{_diff}}\")",
                    n = ord.n.max(1)
                );
            } else {
                out.push_str("print(f\"check {_check_id} ({_check_name}): recorded={_recorded_outcome}, residual={sp.simplify(_lhs - _rhs)}\")\n");
            }
        }
        _ => {
            let _ = writeln!(
                out,
                "print(f\"check {{_check_id}} ({{_check_name}}): recorded={{_recorded_outcome}}, params={{}}\", {})",
                py_str_literal(&json!(check.params).to_string())
            );
        }
    }
    out
}
