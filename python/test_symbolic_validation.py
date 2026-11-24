"""Regression coverage for the symbolic research checks and their worker output."""

import hashlib
import json
import unittest

import sympy as sp

from symbolic_validation import check_symbolic_identity
from worker import evaluate, make_namespace


class SymbolicValidationTests(unittest.TestCase):
    def setUp(self):
        self.x = sp.Symbol("x")

    def test_square_root_uses_declared_domain(self):
        x = self.x
        root = sp.sqrt(x**2)
        real = check_symbolic_identity(root, x, domain=sp.S.Reals)
        self.assertEqual(real["status"], "counterexample")
        self.assertTrue(any(row["point"] == "x = -1" and row["status"] == "counterexample"
                            for row in real["samples"]))
        positive = check_symbolic_identity(root, x, domain=sp.Interval.open(0, sp.oo))
        self.assertEqual(positive["status"], "symbolic_agreement")
        self.assertEqual(positive["domain"]["status"], "complete")
        self.assertEqual(positive["residual"], "0")

    def test_hole_survives_symbolic_cancellation(self):
        x = self.x
        result = check_symbolic_identity(
            (x**2 - 1) / (x - 1), x + 1, domain=sp.S.Reals, limits=[(1, "-"), (1, "+")]
        )
        self.assertEqual(result["status"], "symbolic_agreement")
        self.assertEqual(result["domain"]["status"], "restricted")
        point = next(row for row in result["samples"] if row["point"] == "x = 1")
        self.assertEqual(point["status"], "undefined")
        self.assertEqual([row["status"] for row in result["limits"]], ["agreement", "agreement"])

    def test_complex_principal_branch_counterexample(self):
        x = self.x
        result = check_symbolic_identity(sp.sqrt(x**2), x, domain=sp.S.Complexes, samples=[-sp.I, sp.I])
        self.assertEqual(result["domain"]["status"], "undetermined")
        self.assertEqual(result["status"], "counterexample")
        self.assertEqual(result["samples"][0]["lhs"], "I")
        self.assertEqual(result["samples"][0]["rhs"], "-I")

    def test_declared_domain_intersects_parity_and_excluded_number_sets(self):
        for assumptions, domain in [
            ({"even": True}, sp.FiniteSet(1)),
            ({"odd": True}, sp.FiniteSet(2)),
            ({"irrational": True}, sp.S.Rationals),
            ({"integer": False, "real": True}, sp.S.Integers),
        ]:
            with self.subTest(assumptions=assumptions, domain=domain):
                x = sp.Symbol("x", **assumptions)
                with self.assertRaisesRegex(ValueError, "domain is empty"):
                    check_symbolic_identity(x, x, domain=domain)
        even = sp.Symbol("n", even=True)
        result = check_symbolic_identity(even, even, domain=sp.FiniteSet(1, 2))
        self.assertEqual(result["domain"]["effective"], "{2}")
        self.assertEqual(result["status"], "symbolic_agreement")

    def test_undecidable_sample_membership_remains_undetermined(self):
        x, a = sp.symbols("x a", real=True)
        result = check_symbolic_identity(x, x, domain=sp.Interval(a, sp.oo), samples=[1])
        self.assertEqual(result["samples"][0]["status"], "undetermined")

    def test_directional_limits_respect_domain_and_detect_different_infinities(self):
        x = self.x
        poles = check_symbolic_identity(1 / x, 1 / sp.Abs(x), domain=sp.S.Reals,
                                       limits=[(0, "-"), (0, "+")])
        self.assertEqual([row["status"] for row in poles["limits"]],
                         ["counterexample", "agreement_infinite"])
        bounded = check_symbolic_identity(sp.sqrt(x), sp.sqrt(x), domain=sp.Interval(0, 1),
                                          limits=[(0, "-"), (0, "+"), (sp.oo, "-")])
        self.assertEqual([row["status"] for row in bounded["limits"]],
                         ["outside_domain", "agreement", "outside_domain"])

    def test_multivariate_probes_use_all_symbol_assumptions(self):
        y = sp.Symbol("y", positive=True)
        x = self.x
        result = check_symbolic_identity(x + sp.Abs(y), x + y, samples=[{x: 1, y: -1}, {x: 1, y: 2}])
        self.assertEqual(result["assumptions"]["y"]["positive"], True)
        self.assertEqual([row["status"] for row in result["samples"]],
                         ["assumption_mismatch", "agreement"])
        with self.assertRaises(ValueError):
            check_symbolic_identity(x + y, x - y, samples=[{x: 1}])

    def test_dimension_mismatch_and_unknown_rules_are_explicit(self):
        x, t, v = sp.symbols("x t v")
        units = {x: {"L": 1}, t: {"T": 1}, v: {"L": 1, "T": -1}}
        correct = check_symbolic_identity(x, v * t, dimensions=units)["dimensions"]
        self.assertEqual((correct["status"], correct["lhs"], correct["rhs"]), ("match", "L", "L"))
        mismatch = check_symbolic_identity(sp.exp(x), sp.exp(x), dimensions=units)
        self.assertEqual(mismatch["dimensions"]["status"], "mismatch")
        self.assertEqual(mismatch["status"], "undetermined")
        unknown = check_symbolic_identity(x, v * t, dimensions={x: {"L": 1}})["dimensions"]
        self.assertEqual(unknown["status"], "undetermined")

    def test_approximate_numbers_are_not_exact_evidence(self):
        result = check_symbolic_identity(sp.Float("0.1"), sp.Rational(1, 10))
        self.assertEqual(result["status"], "undetermined")
        with self.assertRaises(ValueError):
            check_symbolic_identity(self.x, self.x, samples=[0.5])

    def test_worker_emits_reports_as_replayable_structured_values(self):
        for code, expected_kind in [
            ("x = Symbol('x', real=True)\ncheck_symbolic_identity(sqrt(x**2), x)", "symbolic_identity"),
            ("check_convergence(lambda n: Rational(1, n), [10, 20, 40], atol='0.1', rtol=0)",
             "numerical_convergence"),
        ]:
            with self.subTest(kind=expected_kind):
                result = evaluate(code, make_namespace())
                self.assertNotIn("error", result)
                validation = result["validation"]
                self.assertEqual(validation["kind"], expected_kind)
                encoded = json.dumps(validation, sort_keys=True, separators=(",", ":")).encode()
                self.assertEqual(result["comparison"],
                                 {"kind": "validation_sha256", "value": hashlib.sha256(encoded).hexdigest()})


if __name__ == "__main__":
    unittest.main()
