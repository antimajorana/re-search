"""Regression tests for WP1 provenance honesty in checks (`srepr_safe`, `algebra`, `algebra_probe`)."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

import sympy as sp
from sympy.parsing.sympy_parser import parse_expr

PYTHON_DIR = Path(__file__).resolve().parents[1]
if str(PYTHON_DIR) not in sys.path:
    sys.path.insert(0, str(PYTHON_DIR))

import checks  # noqa: E402
from checks.srepr_safe import UnsafeSreprError, canonicalize_srepr, parse_srepr_safe  # noqa: E402


class Wp1ChecksRegressionTests(unittest.TestCase):
    def test_srepr_safe_preserves_unevaluated_expr_and_blocks_hostile_usage(self) -> None:
        x = sp.Symbol("x", real=True)
        with sp.evaluate(False):
            u_const = sp.UnevaluatedExpr(sp.Add(sp.Integer(2), sp.Integer(3)))
            u_trig = sp.UnevaluatedExpr(sp.sin(sp.Integer(0)))
        u_poly = sp.UnevaluatedExpr((x + 1) ** 2)
        u_nested = 1 + sp.UnevaluatedExpr(x + 1)

        for expr in (u_const, u_trig, u_poly, u_nested):
            raw = sp.srepr(expr)
            rebuilt = parse_srepr_safe(raw)
            self.assertEqual(sp.srepr(rebuilt), raw)
            canon = canonicalize_srepr(raw)
            self.assertTrue(canon["portable"], msg=canon)
            self.assertEqual(canon["srepr"], raw)

        self.assertEqual(canonicalize_srepr(sp.srepr(u_const))["latex"], "2 + 3")

        for bad in (
            "UnevaluatedExpr('1+1')",
            "UnevaluatedExpr(Pow(Integer(2), Integer(1000000)))",
            "UnevaluatedExpr(Integer(1), Integer(2))",
        ):
            with self.assertRaises(UnsafeSreprError):
                parse_srepr_safe(bad)

    def test_identical_sides_return_trivial_across_algebra_checks(self) -> None:
        x = sp.Symbol("x", real=True)
        m = sp.Matrix([[sp.Integer(1), x], [sp.Integer(0), sp.Integer(2)]])
        cases = [
            ("Integer(0)", sp.Integer(0), sp.Integer(0)),
            ("poly", x**2 + 1, x**2 + 1),
            ("matrix", m, m),
            ("unevaluated", sp.UnevaluatedExpr(x + 1), sp.UnevaluatedExpr(x + 1)),
        ]
        order = {"variable": "x", "point": "0", "n": 3}
        for label, lhs, rhs in cases:
            handles = {"lhs": sp.srepr(lhs), "rhs": sp.srepr(rhs)}
            for check_name in ("identity_symbolic", "identity_numeric", "series_match", "vanishes_to_order"):
                params: dict = {"symbols": [{"name": "x", "assumptions": ["real"]}]}
                if check_name in ("series_match", "vanishes_to_order"):
                    params["order"] = order
                with self.subTest(case=label, check=check_name):
                    res = checks.run_registered_check(check_name, handles, params)
                    self.assertEqual(res["outcome"], "trivial")
                    self.assertTrue(res["evidence"]["identical_sides"])

    def test_unevaluated_and_parse_expr_vs_expanded_are_non_trivial_and_pass(self) -> None:
        x = sp.Symbol("x", real=True)
        expanded = x**2 + 2 * x + 1

        # 1. UnevaluatedExpr((x + 1)**2) vs x**2 + 2*x + 1
        res_u = checks.run_registered_check(
            "identity_symbolic",
            {"lhs": sp.srepr(sp.UnevaluatedExpr((x + 1) ** 2)), "rhs": sp.srepr(expanded)},
            {"symbols": [{"name": "x", "assumptions": ["real"]}]},
        )
        self.assertEqual(res_u["outcome"], "pass")

        # 2. Nested UnevaluatedExpr(2 + 3) vs Integer(5)
        with sp.evaluate(False):
            u_five = sp.UnevaluatedExpr(sp.Add(sp.Integer(2), sp.Integer(3)))
        res_five = checks.run_registered_check(
            "identity_symbolic",
            {"lhs": sp.srepr(u_five), "rhs": sp.srepr(sp.Integer(5))},
            {},
        )
        self.assertEqual(res_five["outcome"], "pass")

        # 3. parse_expr("(x + 1)**2", evaluate=False) vs x**2 + 2*x + 1 across symbolic, numeric, series
        lhs_pe = parse_expr("(x + 1)**2", local_dict={"x": x}, evaluate=False)
        for check_name, extra in (
            ("identity_symbolic", {}),
            ("identity_numeric", {}),
            ("series_match", {"order": {"variable": "x", "point": "0", "n": 4}}),
            ("vanishes_to_order", {"order": {"variable": "x", "point": "0", "n": 4}}),
        ):
            res = checks.run_registered_check(
                check_name,
                {"lhs": sp.srepr(lhs_pe), "rhs": sp.srepr(expanded)},
                {"symbols": [{"name": "x", "assumptions": ["real"]}], **extra},
            )
            self.assertEqual(res["outcome"], "pass", msg=f"{check_name}: {res}")

    def test_matrix_valued_identities_and_mismatches(self) -> None:
        x = sp.Symbol("x", real=True)
        pauli_x = sp.Matrix([[0, 1], [1, 0]])
        pauli_z = sp.diag(1, -1)

        # Constant 2x2 Pauli identity via MatMul vs -eye(2)
        res_pauli = checks.run_registered_check(
            "identity_symbolic",
            {"lhs": sp.srepr(sp.MatMul(pauli_x, pauli_z, pauli_x, pauli_z)), "rhs": sp.srepr(-sp.eye(2))},
            {},
        )
        self.assertEqual(res_pauli["outcome"], "pass")
        self.assertEqual(res_pauli["evidence"]["guard_points"], 1)

        # Equal symbolic matrices -> pass in symbolic, numeric, and smoke check
        m1 = sp.Matrix([[sp.sin(x) ** 2 + sp.cos(x) ** 2, 0], [0, x]])
        m2 = sp.Matrix([[1, 0], [0, x]])
        for chk in ("identity_symbolic", "identity_numeric", "symbolic_zero_smoke"):
            res = checks.run_registered_check(
                chk,
                {"lhs": sp.srepr(m1), "rhs": sp.srepr(m2)},
                {"symbols": [{"name": "x", "assumptions": ["real"]}]},
            )
            self.assertEqual(res["outcome"], "pass", msg=f"{chk}: {res}")

        # Unequal matrices -> fail with counterexample
        m_bad = sp.Matrix([[1, 0], [0, x + 1]])
        for chk in ("identity_symbolic", "identity_numeric"):
            res = checks.run_registered_check(
                chk,
                {"lhs": sp.srepr(m2), "rhs": sp.srepr(m_bad)},
                {"symbols": [{"name": "x", "assumptions": ["real"]}]},
            )
            self.assertEqual(res["outcome"], "fail", msg=f"{chk}: {res}")
            self.assertIn("counterexample", res["evidence"])

        # Shape mismatch and matrix-vs-scalar -> inconclusive (never crash)
        m_2x3 = sp.Matrix([[1, 0, 0], [0, 1, 0]])
        for bad_rhs in (m_2x3, sp.Integer(1)):
            for chk in ("identity_symbolic", "identity_numeric", "symbolic_zero_smoke"):
                res = checks.run_registered_check(
                    chk,
                    {"lhs": sp.srepr(m2), "rhs": sp.srepr(bad_rhs)},
                    {"symbols": [{"name": "x", "assumptions": ["real"]}]},
                )
                self.assertEqual(res["outcome"], "inconclusive", msg=f"{chk}: {res}")

        # Non-eq relation on matrices -> inconclusive
        rel_inc = checks.run_registered_check(
            "counterexample_search",
            {"lhs": sp.srepr(m1), "rhs": sp.srepr(m2)},
            {"relation": "le", "symbols": [{"name": "x", "assumptions": ["real"]}]},
        )
        self.assertEqual(rel_inc["outcome"], "inconclusive")

    def test_zero_point_numeric_probe_max_residual_none_and_series_inconclusive(self) -> None:
        xz = sp.Symbol("x", zero=True)
        res = checks.run_registered_check(
            "identity_numeric",
            {"lhs": sp.srepr(1 / xz), "rhs": sp.srepr(sp.Integer(1))},
            {"symbols": [{"name": "x", "assumptions": ["zero"]}]},
        )
        self.assertEqual(res["outcome"], "inconclusive")
        self.assertEqual(res["evidence"]["evaluated_points"], 0)
        self.assertIsNone(res["evidence"]["max_residual"])

        # _compare_series with float coefficients + un-evaluable parameter -> inconclusive (never fail)
        eps = sp.Symbol("eps", real=True)
        res_series = checks.run_registered_check(
            "series_match",
            {
                "lhs": sp.srepr(sp.Float("1.0") * eps + (1 / xz)),
                "rhs": sp.srepr(sp.Float("1.0") * eps + sp.Integer(1)),
            },
            {
                "symbols": [
                    {"name": "eps", "assumptions": ["real"]},
                    {"name": "x", "assumptions": ["zero"]},
                ],
                "order": {"variable": "eps", "point": "0", "n": 2},
            },
        )
        self.assertEqual(res_series["outcome"], "inconclusive")
        self.assertIsNone(res_series["evidence"]["max_coefficient_residual"])


if __name__ == "__main__":
    unittest.main()
