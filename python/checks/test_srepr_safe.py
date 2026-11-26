"""Tests for the restricted `srepr` parser (`python/checks/srepr_safe.py`, S6).

Covers hostile inputs (security & DoS) and measures acceptance share + rejected
constructors on an ordinary math and theoretical-physics expression corpus.
"""

from __future__ import annotations

import hashlib
import unittest

import sympy as sp
from sympy.physics.matrices import msigma
from sympy.physics.quantum import Bra, Commutator, Dagger, Ket, Operator, TensorProduct

from checks.srepr_safe import (
    MAX_AST_DEPTH,
    MAX_SREPR_CHARS,
    MAX_STR_PREVIEW_CHARS,
    UnsafeSreprError,
    canonicalize_srepr,
    parse_srepr_safe,
)


def build_ordinary_corpus() -> list[tuple[str, object]]:
    """55 self-authored math & theoretical-physics SymPy expressions."""
    x, y, z, t = sp.symbols("x y z t")
    m = sp.Symbol("m", positive=True, real=True)
    mu = sp.Symbol("mu", positive=True, real=True)
    lam = sp.Symbol("lambda", positive=True, real=True)
    phi = sp.Symbol("phi", real=True)
    eps = sp.Symbol("eps", real=True)
    n = sp.Symbol("n", integer=True, nonnegative=True)
    k = sp.Symbol("k", integer=True)
    N = sp.Symbol("N", integer=True, positive=True)
    A = sp.Symbol("A", commutative=False)
    B = sp.Symbol("B", commutative=False)
    f = sp.Function("f")
    psi = sp.Function("psi")
    i, j = sp.symbols("i j", cls=sp.Idx)
    idx_bounded = sp.Idx("a", (1, N))
    T = sp.IndexedBase("T")
    g_metric = sp.IndexedBase("g")
    M = sp.MatrixSymbol("M", 3, 3)
    K = sp.MatrixSymbol("K", 3, 3)
    op_a, op_b = Operator("A"), Operator("B")

    return [
        # Core algebra & polynomials
        ("poly_cubic", x**3 - 2 * x + sp.Rational(1, 3)),
        ("assumed_radical", sp.sqrt(m**2 + phi**2) + sp.Rational(3, 4) * m),
        ("effective_potential_msbar", m**4 / (64 * sp.pi**2) * (sp.log(m**2 / mu**2) - sp.Rational(3, 2))),
        ("tree_potential", lam * phi**4 / sp.factorial(4)),
        ("noncomm_commutator", A * B - B * A),
        ("noncomm_bch_order2", A + B + sp.Rational(1, 2) * (A * B - B * A)),
        ("poly_object", sp.Poly(x**2 + 2 * x * y + y**2, x, y)),
        ("complex_rootof", sp.CRootOf(x**5 + x + 1, 0)),
        ("high_precision_float", sp.Float("3.141592653589793238462643383279", 30)),
        ("constants_combo", sp.pi * sp.E + sp.I * sp.EulerGamma + sp.Catalan + sp.GoldenRatio),
        ("dummy_symbol", sp.Dummy("d", real=True) + x),
        # Calculus, series, ODEs
        ("undef_function_2d", psi(x, t) + f(x)),
        ("ode_harmonic", sp.Derivative(f(x), x, 2) + m**2 * f(x)),
        ("gaussian_integral", sp.Integral(sp.exp(-m * x**2), (x, -sp.oo, sp.oo))),
        ("dimreg_integral", sp.Integral(sp.log(x**2 + m**2), (x, 0, sp.oo))),
        ("basel_sum", sp.Sum(1 / n**2, (n, 1, sp.oo))),
        ("finite_product", sp.Product(1 + x / n, (n, 1, N))),
        ("one_sided_limit", sp.Limit(sp.sin(x) / x, x, 0, "+")),
        ("taylor_series_order", sp.series(sp.sin(x), x, 0, 5)),
        ("eps_series_order", sp.series(sp.gamma(eps), eps, 0, 2)),
        ("derivative_subs", sp.Subs(sp.Derivative(f(x), x), x, 0)),
        # Relations, logic, sets, piecewise
        ("equation", sp.Eq(x**2 + y**2, m**2)),
        ("inequality_and", sp.And(x > 0, x <= 1, sp.Ne(y, 0))),
        ("piecewise_abs", sp.Piecewise((x, x > 0), (-x, True))),
        ("piecewise_integral", sp.integrate(x**k, x)),
        ("interval_union", sp.Union(sp.Interval.open(0, sp.oo), sp.FiniteSet(0))),
        ("condition_set", sp.ConditionSet(x, x > 0, sp.S.Reals)),
        # Matrices & linear algebra
        ("dense_matrix", sp.Matrix([[1, x], [0, sp.exp(y)]])),
        ("immutable_pauli_y", sp.ImmutableMatrix([[0, -sp.I], [sp.I, 0]])),
        ("pauli_msigma", msigma(1) + sp.I * msigma(2) + msigma(3)),
        ("sparse_matrix", sp.ImmutableSparseMatrix(3, 3, {(0, 2): x, (2, 0): -x})),
        ("matrix_trace_det", sp.Trace(M * K.T) + sp.Determinant(M)),
        ("matrix_inv_adj", sp.Adjoint(M) * M.inv() + sp.Identity(3)),
        ("matrix_pow_add", M**3 + K + sp.ZeroMatrix(3, 3)),
        ("kronecker_product", sp.KroneckerProduct(M, K)),
        # Physics indexed tensors & deltas
        ("indexed_contraction", T[i, j] * T[j, i]),
        ("metric_bounded_idx", g_metric[idx_bounded, j] * T[j]),
        ("kronecker_levicivita", sp.KroneckerDelta(i, j) + sp.LeviCivita(1, 2, 3)),
        ("distributions", sp.DiracDelta(x - m) + sp.Heaviside(x)),
        # Elementary & special functions
        ("trig_hyperbolic", sp.sin(x) * sp.cosh(y) + sp.atan2(y, x) + sp.sech(z)),
        ("complex_ops", sp.re(z) + sp.I * sp.im(z) + sp.Abs(z) * sp.conjugate(z) + sp.arg(z)),
        ("floor_mod_minmax", sp.floor(x) + sp.ceiling(y) + sp.Mod(k, 2) + sp.Max(x, y)),
        ("bessel_hankel", sp.besselj(n, x) + sp.besselk(0, m * x) + sp.hankel1(1, x)),
        ("gamma_beta_polygamma", sp.gamma(x) * sp.beta(x, y) + sp.polygamma(1, x) + sp.loggamma(m)),
        ("zeta_polylog_erf", sp.zeta(3) + sp.polylog(2, x) + sp.erf(x) + sp.erfc(y)),
        ("orthogonal_polys", sp.legendre(n, x) + sp.hermite(3, x) + sp.laguerre(2, x) + sp.chebyshevt(4, x)),
        ("spherical_harmonic", sp.Ynm(2, 1, x, y)),
        ("hypergeometric", sp.hyper([1, 2], [3], x)),
        ("meijerg_fn", sp.meijerg([[1], []], [[], [0]], x)),
        ("elliptic_integrals", sp.elliptic_k(x) + sp.elliptic_e(x) + sp.elliptic_f(x, y)),
        ("lambert_airy_ei", sp.LambertW(x) + sp.Ei(x) + sp.Si(x) + sp.airyai(x) + sp.fresnels(x)),
        ("combinatorics", sp.binomial(n, k) + sp.RisingFactorial(x, 3) + sp.FallingFactorial(x, 2)),
        # Intentionally outside core allowlist (measured honestly per overview sec 6)
        ("sympy_lambda", sp.Lambda(x, x**2)),
        ("imageset_lambda", sp.ImageSet(sp.Lambda(x, 2 * x), sp.S.Integers)),
        ("quantum_outer_product", Ket("psi") * Bra("phi")),
        ("quantum_dagger", Dagger(op_a)),
        ("quantum_commutator", Commutator(op_a, op_b)),
        ("quantum_tensor_product", TensorProduct(op_a, op_b)),
    ]


class SreprSafeHostileTests(unittest.TestCase):
    def test_rejects_dunders_imports_and_builtins(self):
        hostile = [
            ("__import__('os').system('echo pwn')", "Attribute"),
            ("__import__('os')", "__import__"),
            ("__builtins__", "__builtins__"),
            ("__class__", "__class__"),
            ("eval('1 + 1')", "eval"),
            ("exec('import os')", "exec"),
            ("open('secret.txt')", "open"),
            ("compile('1', '', 'eval')", "compile"),
            ("getattr(Symbol('x'), '__class__')", "getattr"),
            ("type(Integer(1))", "type"),
            ("Symbol('__import__')", "__import__"),
            ("Function('__import__')(Symbol('x'))", "__import__"),
        ]
        for payload, expected_rejected in hostile:
            with self.subTest(payload=payload):
                with self.assertRaises(UnsafeSreprError) as ctx:
                    parse_srepr_safe(payload)
                self.assertEqual(ctx.exception.rejected_name, expected_rejected)
                canon = canonicalize_srepr(payload)
                self.assertFalse(canon["portable"])
                self.assertEqual(canon.get("rejected_constructor"), expected_rejected)

    def test_rejects_attribute_access_lambdas_comprehensions_subscripts_binops(self):
        hostile = [
            "Symbol('x').__class__",
            "().__class__.__base__.__subclasses__()",
            "(lambda x: x)(Integer(1))",
            "(lambda f: (lambda x: f(f)(x)))(lambda y: y)",
            "Lambda(Tuple(Symbol('x')), Pow(Symbol('x'), Integer(2)))",
            "[Integer(1) for x in (1, 2)]",
            "{Integer(1): Integer(2) for x in (1,)}",
            "(Integer(1) for x in (1,))",
            "Tuple(Integer(1))[0]",
            "Integer(1) + Integer(2)",
            "10 ** 5000",
            "Add(*[Integer(1), Integer(2)])",
            "Symbol('x', **{'real': True})",
            "Symbol('x', __class__=True)",
            "Symbol('x', arbitrary_kwarg=True)",
            "Poly(Symbol('x'), Symbol('x'), domain='__import__')",
        ]
        for payload in hostile:
            with self.subTest(payload=payload):
                with self.assertRaises(UnsafeSreprError):
                    parse_srepr_safe(payload)
                self.assertFalse(canonicalize_srepr(payload)["portable"])

    def test_rejects_raw_string_sympify_bypass(self):
        bypass_attempts = [
            "sin(\"__import__('os').system('id')\")",
            "Add('1+1', Integer(1))",
            "MutableDenseMatrix([[\"__import__('os')\"]])",
            "Function('f')('1+1')",
            "Eq('1+1', Integer(2))",
            "Pow('2', Integer(3))",
            "Rational('1/2')",
            "Integer('123')",
            "Limit(sin(Symbol('x')), Symbol('x'), Integer(0), 'evil')",
        ]
        for payload in bypass_attempts:
            with self.subTest(payload=payload):
                with self.assertRaises(UnsafeSreprError):
                    parse_srepr_safe(payload)

    def test_rejects_dos_resource_caps(self):
        deep = "Symbol('x')"
        for _ in range(MAX_AST_DEPTH + 2):
            deep = f"sin({deep})"

        dos_cases = [
            f"Integer({'9' * 2000})",
            "Pow(Integer(2), Integer(1000000))",
            "Pow(Symbol('x'), Integer(100000))",
            "Pow(Integer(1000000000), Integer(500))",
            "MatPow(MatrixSymbol(Str('A'), Integer(2), Integer(2)), Integer(50000))",
            "Float('1.5', precision=4096)",
            "Float('1.5', dps=500)",
            "Float('1e99999')",
            "Float('inf')",
            "Float('nan')",
            "factorial(Integer(100000))",
            "Identity(Integer(1000))",
            deep,
            "Add(" + ", ".join(["Integer(1)"] * 1025) + ")",
            "Symbol('" + "a" * 300 + "')",
            " " * (MAX_SREPR_CHARS + 1),
        ]
        for payload in dos_cases:
            with self.subTest(payload=payload[:60]):
                with self.assertRaises(UnsafeSreprError):
                    parse_srepr_safe(payload)

    def test_rejects_non_strings_and_bad_syntax(self):
        for bad in (None, 123, ["Integer(1)"], "", "   ", "Add(", "Symbol('x'); import os"):
            with self.subTest(bad=bad):
                with self.assertRaises(UnsafeSreprError):
                    parse_srepr_safe(bad)  # type: ignore[arg-type]


class SreprSafeCorpusAndCanonicalizeTests(unittest.TestCase):
    def test_ordinary_math_and_physics_corpus_measurement(self):
        corpus = build_ordinary_corpus()
        accepted: list[str] = []
        rejected: list[tuple[str, str | None, str]] = []

        for label, expr in corpus:
            raw = sp.srepr(expr)
            result = canonicalize_srepr(raw)
            if result["portable"]:
                accepted.append(label)
                rebuilt = parse_srepr_safe(raw)
                self.assertEqual(sp.srepr(rebuilt), result["srepr"], msg=f"round-trip drift for {label}")
                self.assertEqual(
                    result["srepr_sha256"],
                    hashlib.sha256(result["srepr"].encode("utf-8")).hexdigest(),
                )
                self.assertLessEqual(len(result["str_preview"]), MAX_STR_PREVIEW_CHARS)
            else:
                rejected.append((label, result.get("rejected_constructor"), result.get("error", "")))

        total = len(corpus)
        # All 52 core math & physics expressions must be accepted; only the 6 intentionally
        # unallowlisted expressions (Lambda, ImageSet, quantum OuterProduct/Dagger/Commutator/TensorProduct) reject.
        self.assertEqual(total, 58)
        self.assertEqual(len(accepted), 52, msg=f"unexpected rejections: {rejected}")
        rejected_ctors = sorted({ctor for _, ctor, _ in rejected if ctor})
        self.assertEqual(
            rejected_ctors,
            ["Commutator", "Dagger", "ImageSet", "Lambda", "OuterProduct", "TensorProduct"],
        )

    def test_canonicalize_free_symbols_and_preview_cap(self):
        x = sp.Symbol("x")
        m = sp.Symbol("m", positive=True, real=True)
        a = sp.Symbol("A", commutative=False)
        expr = a * m + x
        res = canonicalize_srepr(sp.srepr(expr))
        self.assertTrue(res["portable"])
        self.assertEqual(
            res["free_symbols"],
            [
                {"name": "A", "assumptions": ["commutative=false"]},
                {"name": "m", "assumptions": ["positive", "real"]},
                {"name": "x", "assumptions": []},
            ],
        )
        self.assertEqual(res["latex"], sp.latex(expr))

        long_expr = sum(sp.Symbol(f"v_{idx:03d}") for idx in range(120))
        long_res = canonicalize_srepr(sp.srepr(long_expr))
        self.assertTrue(long_res["portable"])
        self.assertEqual(len(long_res["str_preview"]), MAX_STR_PREVIEW_CHARS)


if __name__ == "__main__":
    unittest.main()
