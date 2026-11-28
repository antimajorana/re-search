"""Unit tests for the five S9 host algebra checks (`python/checks/algebra.py`)."""

from __future__ import annotations

import unittest

import sympy as sp

import checks


class AlgebraChecksTests(unittest.TestCase):
    def test_all_five_checks_registered(self) -> None:
        names = set(checks.list_checks())
        expected = {
            "identity_symbolic",
            "identity_numeric",
            "series_match",
            "vanishes_to_order",
            "counterexample_search",
            "symbolic_zero_smoke",
        }
        self.assertTrue(expected.issubset(names), names)

    def test_identity_symbolic_pass_mismatch_unreduced_and_q_check_counterexamples(self) -> None:
        x = sp.Symbol("x", real=True)
        # Valid exact identity passes with comparison=exact, exact=True.
        ok_res = checks.run_registered_check(
            "identity_symbolic",
            {
                "lhs": sp.srepr(sp.sin(x) ** 2 + sp.cos(x) ** 2),
                "rhs": sp.srepr(sp.Integer(1)),
            },
            {"symbols": [{"name": "x", "assumptions": ["real"]}]},
        )
        self.assertEqual(ok_res["outcome"], "pass")
        self.assertEqual(ok_res["evidence"]["comparison"], "exact")
        self.assertTrue(ok_res["evidence"]["exact"])

        # Symbol assumption mismatch -> inconclusive: symbol assumptions differ.
        x_pos = sp.Symbol("x", positive=True)
        mismatch = checks.run_registered_check(
            "identity_symbolic",
            {"lhs": sp.srepr(sp.sqrt(x_pos**2)), "rhs": sp.srepr(x_pos)},
            {"symbols": [{"name": "x", "assumptions": ["real"]}]},
        )
        self.assertEqual(mismatch["outcome"], "inconclusive")
        self.assertIn("symbol assumptions differ", mismatch["evidence"]["reason"])

        # Q-check-01: sqrt((x - 2)**2) + x vs 2*x - 2 on real x in [0, 4] -> fail with counterexample.
        xr = sp.Symbol("x", real=True)
        q1 = checks.run_registered_check(
            "identity_symbolic",
            {"lhs": sp.srepr(sp.sqrt((xr - 2) ** 2) + xr), "rhs": sp.srepr(2 * xr - 2)},
            {"symbols": [{"name": "x", "assumptions": ["real"]}], "domain": "real x in [0, 4]"},
        )
        self.assertEqual(q1["outcome"], "fail")
        self.assertIn("counterexample", q1["evidence"])

        # Q-check-02: log(z**2) vs 2*log(z) with r>0, theta in (pi/2, pi) -> fail with counterexample.
        r = sp.Symbol("r", real=True, positive=True)
        theta = sp.Symbol("theta", real=True)
        z = r * sp.exp(sp.I * theta)
        q2 = checks.run_registered_check(
            "identity_symbolic",
            {"lhs": sp.srepr(sp.log(z**2)), "rhs": sp.srepr(2 * sp.log(z))},
            {
                "symbols": [
                    {"name": "r", "assumptions": ["real", "positive"]},
                    {"name": "theta", "assumptions": ["real"]},
                ],
                "domain": "r > 0 and theta in (pi/2, pi)",
            },
        )
        self.assertEqual(q2["outcome"], "fail")
        self.assertIn("counterexample", q2["evidence"])

        # Q-check-03: fourth Gaussian moment vs wrong 2x recursion (3/alpha)*I_2(alpha) -> fail.
        alpha = sp.Symbol("alpha", real=True, positive=True)
        x_int = sp.Symbol("x", real=True)
        lhs_q3 = sp.Integral(x_int**4 * sp.exp(-alpha * x_int**2), (x_int, -sp.oo, sp.oo))
        rhs_q3 = (3 / alpha) * sp.Integral(x_int**2 * sp.exp(-alpha * x_int**2), (x_int, -sp.oo, sp.oo))
        q3 = checks.run_registered_check(
            "identity_symbolic",
            {"lhs": sp.srepr(lhs_q3), "rhs": sp.srepr(rhs_q3)},
            {"symbols": [{"name": "alpha", "assumptions": ["real", "positive"]}, {"name": "x", "assumptions": ["real"]}]},
        )
        self.assertEqual(q3["outcome"], "fail")
        self.assertIn("counterexample", q3["evidence"])

        # Unreduced nonzero residual with NO counterexample -> inconclusive (never fail).
        # Euler's dilogarithm reflection identity holds numerically on (0.1, 0.9) while
        # simplify(lhs - rhs) leaves a nonzero symbolic residual in SymPy 1.14.
        dilog_lhs = sp.polylog(2, xr) + sp.polylog(2, 1 - xr)
        dilog_rhs = sp.pi**2 / 6 - sp.log(xr) * sp.log(1 - xr)
        inc = checks.run_registered_check(
            "identity_symbolic",
            {"lhs": sp.srepr(dilog_lhs), "rhs": sp.srepr(dilog_rhs)},
            {"symbols": [{"name": "x", "assumptions": ["real"]}], "domain": "x in (0.1, 0.9)"},
        )
        self.assertEqual(inc["outcome"], "inconclusive")

    def test_identity_numeric_50_digits_and_counterexample(self) -> None:
        xr = sp.Symbol("x", real=True, positive=True)
        res = checks.run_registered_check(
            "identity_numeric",
            {
                "lhs": sp.srepr(sp.besselj(0, xr) + sp.besselj(2, xr)),
                "rhs": sp.srepr(2 * sp.besselj(1, xr) / xr),
            },
            {
                "symbols": [{"name": "x", "assumptions": ["real", "positive"]}],
                "lhs_code_sha256": "aaa",
                "rhs_code_sha256": "bbb",
            },
        )
        self.assertEqual(res["outcome"], "pass")
        self.assertGreaterEqual(res["evidence"]["points"], 12)
        self.assertGreaterEqual(res["evidence"]["digits"], 50)
        self.assertTrue(res["evidence"]["domain_checked"])
        self.assertEqual(res["evidence"]["lhs_code_sha256"], "aaa")

        bad = checks.run_registered_check(
            "identity_numeric",
            {"lhs": sp.srepr(sp.sin(xr)), "rhs": sp.srepr(xr)},
            {"symbols": [{"name": "x", "assumptions": ["real", "positive"]}]},
        )
        self.assertEqual(bad["outcome"], "fail")
        self.assertIn("counterexample", bad["evidence"])

    def test_series_match_and_vanishes_to_order_exact_and_numeric(self) -> None:
        eps = sp.Symbol("eps", real=True)
        order = {"variable": "eps", "point": "0", "n": 4}

        # Exact series match -> exact=True, comparison=exact.
        exact_res = checks.run_registered_check(
            "series_match",
            {
                "lhs": sp.srepr(sp.exp(eps)),
                "rhs": sp.srepr(1 + eps + eps**2 / 2 + eps**3 / 6),
            },
            {"order": order},
        )
        self.assertEqual(exact_res["outcome"], "pass")
        self.assertEqual(exact_res["evidence"]["comparison"], "exact")
        self.assertTrue(exact_res["evidence"]["exact"])
        self.assertEqual(exact_res["evidence"]["order"], order)

        # Numeric coefficient series -> comparison=numeric, numeric_coefficients=True, exact=False.
        num_res = checks.run_registered_check(
            "series_match",
            {
                "lhs": sp.srepr(sp.Float("1.0") * sp.exp(eps)),
                "rhs": sp.srepr(sp.Float("1.0") + eps + sp.Float("0.5") * eps**2 + sp.Rational(1, 6) * eps**3),
            },
            {"order": order},
        )
        self.assertEqual(num_res["outcome"], "pass")
        self.assertEqual(num_res["evidence"]["comparison"], "numeric")
        self.assertTrue(num_res["evidence"]["numeric_coefficients"])
        self.assertFalse(num_res["evidence"]["exact"])

        # Vanishes to order O(eps^3) vs O(eps^4).
        van_ok = checks.run_registered_check(
            "vanishes_to_order",
            {"lhs": sp.srepr(sp.sin(eps) - eps), "rhs": sp.srepr(sp.Integer(0))},
            {"order": {"variable": "eps", "point": "0", "n": 3}},
        )
        self.assertEqual(van_ok["outcome"], "pass")
        self.assertTrue(van_ok["evidence"]["exact"])

        van_fail = checks.run_registered_check(
            "vanishes_to_order",
            {"lhs": sp.srepr(sp.sin(eps) - eps), "rhs": sp.srepr(sp.Integer(0))},
            {"order": {"variable": "eps", "point": "0", "n": 4}},
        )
        self.assertEqual(van_fail["outcome"], "fail")

    def test_counterexample_search_inequality(self) -> None:
        x = sp.Symbol("x", real=True)
        ok = checks.run_registered_check(
            "counterexample_search",
            {"lhs": sp.srepr(sp.sin(x) ** 2), "rhs": sp.srepr(sp.Integer(1))},
            {"relation": "le", "symbols": [{"name": "x", "assumptions": ["real"]}]},
        )
        self.assertEqual(ok["outcome"], "pass")
        self.assertTrue(ok["evidence"]["sampled_no_counterexample"])
        self.assertNotIn("comparison", ok["evidence"])

        bad = checks.run_registered_check(
            "counterexample_search",
            {"lhs": sp.srepr(x**2), "rhs": sp.srepr(x)},
            {"relation": "ge", "symbols": [{"name": "x", "assumptions": ["real"]}], "domain": "x in [0, 2]"},
        )
        self.assertEqual(bad["outcome"], "fail")
        self.assertIn("counterexample", bad["evidence"])

    def test_rejects_unsafe_srepr(self) -> None:
        with self.assertRaises(checks.UnsafeSreprError):
            checks.run_registered_check(
                "identity_symbolic",
                {"lhs": "__import__('os')", "rhs": "Integer(0)"},
                {},
            )


if __name__ == "__main__":
    unittest.main()
