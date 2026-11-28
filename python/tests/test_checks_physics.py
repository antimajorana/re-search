"""Unit tests for the six S10 clean-room physics checks (`python/checks/physics.py`)."""

from __future__ import annotations

import json
import subprocess
import sys
import unittest
from pathlib import Path

import sympy as sp

import checks

WORKER_SCRIPT = Path(__file__).resolve().parents[1] / "worker.py"


class PhysicsChecksTests(unittest.TestCase):
    def test_all_six_physics_checks_registered_and_deterministic_hash(self) -> None:
        names = set(checks.list_checks())
        expected = {
            "dimensions",
            "limit_case",
            "symmetry",
            "conservation",
            "convergence",
            "matrix_property",
        }
        self.assertTrue(expected.issubset(names), names)
        self.assertEqual(checks.registry_sha256(), checks.registry_sha256())

    def test_dimensions_pass_rational_mismatch_and_missing_symbol(self) -> None:
        phi = sp.Symbol("phi", real=True)
        m = sp.Symbol("m", positive=True)
        mu = sp.Symbol("mu", positive=True)
        lam = sp.Symbol("lambda", positive=True)
        a0 = sp.Symbol("a0", positive=True)
        r = sp.Symbol("r", positive=True)
        t = sp.Symbol("t", real=True)
        x = sp.Symbol("x", real=True)
        v = sp.Symbol("v", real=True)
        setup_dims = {
            "phi": {"mass": 1},
            "m": {"mass": 1},
            "mu": {"mass": 1},
            "lambda": {"mass": 0},
            "a0": {"length": 1},
            "r": {"length": 1},
            "t": {"time": 1},
            "x": {"length": 1},
            "v": {"length": 1, "time": -1},
        }
        syms = [
            {"name": "phi", "assumptions": ["real"]},
            {"name": "m", "assumptions": ["positive"]},
            {"name": "mu", "assumptions": ["positive"]},
            {"name": "lambda", "assumptions": ["positive"]},
            {"name": "a0", "assumptions": ["positive"]},
            {"name": "r", "assumptions": ["positive"]},
            {"name": "t", "assumptions": ["real"]},
            {"name": "x", "assumptions": ["real"]},
            {"name": "v", "assumptions": ["real"]},
        ]

        # Effective potential MS-bar: mass^4 with dimensionless log argument.
        v_eff = m**4 / (64 * sp.pi**2) * (sp.log(m**2 / mu**2) - sp.Rational(3, 2))
        v_tree_like = lam**2 * phi**4 / (256 * sp.pi**2)
        ok = checks.run_registered_check(
            "dimensions",
            {"lhs": sp.srepr(v_eff), "rhs": sp.srepr(v_tree_like)},
            {"symbols": syms, "dimensions": setup_dims},
        )
        self.assertEqual(ok["outcome"], "pass")
        self.assertEqual(ok["evidence"]["lhs_dim_str"], "mass^4")

        # Rational dimension power "3/2": radial wavefunction R_10 = 2 * a0^(-3/2) * exp(-r/a0).
        r10 = 2 * a0 ** (-sp.Rational(3, 2)) * sp.exp(-r / a0)
        psi_norm_density = r10**2 * r**2
        ok_rat = checks.run_registered_check(
            "dimensions",
            {"lhs": sp.srepr(psi_norm_density), "rhs": sp.srepr(1 / r)},
            {
                "symbols": syms,
                "dimensions": {**setup_dims, "psi_scale": {"length": "-3/2"}},
            },
        )
        self.assertEqual(ok_rat["outcome"], "pass")
        self.assertEqual(ok_rat["evidence"]["lhs_dim_str"], "length^-1")

        # Derivative & Integral dimensional consistency.
        ok_calc = checks.run_registered_check(
            "dimensions",
            {"lhs": sp.srepr(sp.Derivative(x, t)), "rhs": sp.srepr(v)},
            {"symbols": syms, "dimensions": setup_dims},
        )
        self.assertEqual(ok_calc["outcome"], "pass")

        # Mismatch: x vs v, and non-dimensionless log argument.
        bad = checks.run_registered_check(
            "dimensions",
            {"lhs": sp.srepr(x), "rhs": sp.srepr(v)},
            {"symbols": syms, "dimensions": setup_dims},
        )
        self.assertEqual(bad["outcome"], "fail")
        self.assertEqual(bad["evidence"]["lhs_dim_str"], "length")
        self.assertEqual(bad["evidence"]["rhs_dim_str"], "length * time^-1")

        bad_exp = checks.run_registered_check(
            "dimensions",
            {"lhs": sp.srepr(sp.exp(x)), "rhs": sp.srepr(sp.Integer(1))},
            {"symbols": syms, "dimensions": setup_dims},
        )
        self.assertEqual(bad_exp["outcome"], "fail")

        # Missing symbol dimension -> inconclusive (never fail).
        k_unknown = sp.Symbol("k_unknown", real=True)
        miss = checks.run_registered_check(
            "dimensions",
            {"lhs": sp.srepr(k_unknown * x), "rhs": sp.srepr(x)},
            {"symbols": syms, "dimensions": setup_dims},
        )
        self.assertEqual(miss["outcome"], "inconclusive")

        # Symbol assumption mismatch -> inconclusive: symbol assumptions differ.
        x_pos = sp.Symbol("x", positive=True)
        assump_diff = checks.run_registered_check(
            "dimensions",
            {"lhs": sp.srepr(x_pos), "rhs": sp.srepr(v * t)},
            {"symbols": syms, "dimensions": setup_dims},
        )
        self.assertEqual(assump_diff["outcome"], "inconclusive")
        self.assertIn("symbol assumptions differ", assump_diff["evidence"]["reason"])

    def test_limit_case_symbolic_target_handle_and_failure(self) -> None:
        a = sp.Symbol("a", real=True, positive=True)
        b = sp.Symbol("b", real=True, positive=True)
        x = sp.Symbol("x", real=True)
        syms = [
            {"name": "a", "assumptions": ["real", "positive"]},
            {"name": "b", "assumptions": ["real", "positive"]},
            {"name": "x", "assumptions": ["real"]},
        ]

        # Q-calc-08 degenerate limit b -> a+ using named `target` handle.
        ok = checks.run_registered_check(
            "limit_case",
            {
                "lhs": sp.srepr(sp.pi / (a * b * (a + b))),
                "rhs": sp.srepr(sp.pi / (a * b * (a + b))),
                "target": sp.srepr(sp.pi / (2 * a**3)),
            },
            {"symbols": syms, "variable": "b", "point": "a", "direction": "+"},
        )
        self.assertEqual(ok["outcome"], "pass")

        # sin(x)/x -> 1 vs wrong limit 2 -> fail with counterexample.
        bad = checks.run_registered_check(
            "limit_case",
            {"lhs": sp.srepr(sp.sin(x) / x), "rhs": sp.srepr(sp.Integer(2))},
            {"symbols": syms, "variable": "x", "point": "0"},
        )
        self.assertEqual(bad["outcome"], "fail")
        self.assertIn("counterexample", bad["evidence"])

        # Infinite vs finite mismatch -> fail.
        inf_bad = checks.run_registered_check(
            "limit_case",
            {"lhs": sp.srepr(1 / x), "rhs": sp.srepr(sp.Integer(0))},
            {"symbols": syms, "variable": "x", "point": "0", "direction": "+"},
        )
        self.assertEqual(inf_bad["outcome"], "fail")

    def test_symmetry_parity_exchange_scale_and_hostile_guard(self) -> None:
        phi = sp.Symbol("phi", real=True)
        x = sp.Symbol("x", real=True)
        y = sp.Symbol("y", real=True)
        lam = sp.Symbol("lambda", positive=True)
        mu = sp.Symbol("mu", positive=True)
        s = sp.Symbol("s", positive=True)
        syms = [
            {"name": "phi", "assumptions": ["real"]},
            {"name": "x", "assumptions": ["real"]},
            {"name": "y", "assumptions": ["real"]},
            {"name": "lambda", "assumptions": ["positive"]},
            {"name": "mu", "assumptions": ["positive"]},
            {"name": "s", "assumptions": ["positive"]},
        ]

        # Even Z_2 symmetry phi -> -phi on effective potential.
        v_msbar = lam**2 * phi**4 / (256 * sp.pi**2) * (sp.log(lam * phi**2 / (2 * mu**2)) - sp.Rational(3, 2))
        ok_even = checks.run_registered_check(
            "symmetry",
            {"lhs": sp.srepr(sp.Symbol("V1")), "rhs": sp.srepr(v_msbar)},
            {"symbols": syms, "subs": {"phi": "-phi"}, "sign": 1},
        )
        self.assertEqual(ok_even["outcome"], "pass")

        # Odd symmetry violation -> fail.
        bad_odd = checks.run_registered_check(
            "symmetry",
            {"lhs": sp.srepr(sp.cos(x)), "rhs": sp.srepr(sp.cos(x))},
            {"symbols": syms, "kind": "odd", "variable": "x"},
        )
        self.assertEqual(bad_odd["outcome"], "fail")
        self.assertIn("counterexample", bad_odd["evidence"])

        # Simultaneous exchange symmetry {x: y, y: x}.
        ok_ex = checks.run_registered_check(
            "symmetry",
            {"lhs": sp.srepr(x * y + x + y), "rhs": sp.srepr(x * y + x + y)},
            {"symbols": syms, "kind": "exchange", "variables": ["x", "y"]},
        )
        self.assertEqual(ok_ex["outcome"], "pass")

        # Scale homogeneity f(s*x) = s^2 * f(x).
        ok_scale = checks.run_registered_check(
            "symmetry",
            {"lhs": sp.srepr(x**2 + y**2), "rhs": sp.srepr(x**2 + y**2)},
            {"symbols": syms, "subs": {"x": "s*x", "y": "s*y"}, "factor": "s**2"},
        )
        self.assertEqual(ok_scale["outcome"], "pass")

        # Hostile expression in `subs` is rejected.
        with self.assertRaises(ValueError):
            checks.run_registered_check(
                "symmetry",
                {"lhs": sp.srepr(x), "rhs": sp.srepr(x)},
                {"symbols": syms, "subs": {"x": "__import__('os').system('echo x')"}},
            )

    def test_conservation_commutator_derivative_continuity_and_poisson(self) -> None:
        t = sp.Symbol("t", real=True)
        x = sp.Symbol("x", real=True)
        y = sp.Symbol("y", real=True)
        px = sp.Symbol("px", real=True)
        py = sp.Symbol("py", real=True)
        omega = sp.Symbol("omega", positive=True)
        c = sp.Symbol("c", positive=True)
        syms = [
            {"name": n, "assumptions": ["positive" if n in ("omega", "c") else "real"]}
            for n in ("t", "x", "y", "px", "py", "omega", "c")
        ]

        # Commutator [sigma_z, sigma_z] == 0 vs [sigma_x, sigma_y] != 0.
        sz = sp.Matrix([[1, 0], [0, -1]])
        sx = sp.Matrix([[0, 1], [1, 0]])
        sy = sp.Matrix([[0, -sp.I], [sp.I, 0]])
        self.assertEqual(
            checks.run_registered_check("conservation", {"lhs": sp.srepr(sz), "rhs": sp.srepr(sz)}, {"mode": "commutator"})["outcome"],
            "pass",
        )
        bad_comm = checks.run_registered_check(
            "conservation", {"lhs": sp.srepr(sx), "rhs": sp.srepr(sy)}, {"mode": "commutator"}
        )
        self.assertEqual(bad_comm["outcome"], "fail")

        # Time derivative of oscillator energy vanishes.
        energy = sp.cos(omega * t) ** 2 + sp.sin(omega * t) ** 2
        self.assertEqual(
            checks.run_registered_check(
                "conservation",
                {"lhs": sp.srepr(energy), "rhs": sp.srepr(sp.Integer(0))},
                {"symbols": syms, "mode": "derivative", "variable": "t"},
            )["outcome"],
            "pass",
        )
        self.assertEqual(
            checks.run_registered_check(
                "conservation",
                {"lhs": sp.srepr(sp.exp(-t)), "rhs": sp.srepr(sp.Integer(0))},
                {"symbols": syms, "mode": "derivative", "variable": "t"},
            )["outcome"],
            "fail",
        )

        # 1D continuity equation: d/dt f(x - c*t) + d/dx (c * f(x - c*t)) = 0.
        rho = sp.sin(x - c * t)
        flux = c * sp.sin(x - c * t)
        self.assertEqual(
            checks.run_registered_check(
                "conservation",
                {"lhs": sp.srepr(rho), "rhs": sp.srepr(flux)},
                {"symbols": syms, "mode": "continuity", "time_var": "t", "space_vars": ["x"]},
            )["outcome"],
            "pass",
        )

        # 2D central-force Poisson bracket {H, L_z} = 0.
        h_central = (px**2 + py**2) / 2 + (x**2 + y**2) ** 2
        lz = x * py - y * px
        self.assertEqual(
            checks.run_registered_check(
                "conservation",
                {"lhs": sp.srepr(h_central), "rhs": sp.srepr(lz)},
                {"symbols": syms, "mode": "poisson_bracket", "q_vars": ["x", "y"], "p_vars": ["px", "py"]},
            )["outcome"],
            "pass",
        )

    def test_convergence_sum_integral_and_refinement(self) -> None:
        n = sp.Symbol("n", integer=True, positive=True)
        x = sp.Symbol("x", real=True)
        syms = [{"name": "n", "assumptions": ["integer", "positive"]}, {"name": "x", "assumptions": ["real"]}]

        # Convergent and divergent symbolic sums.
        self.assertEqual(
            checks.run_registered_check(
                "convergence",
                {"lhs": sp.srepr(sp.Sum(1 / n**2, (n, 1, sp.oo))), "rhs": sp.srepr(sp.pi**2 / 6)},
                {"symbols": syms},
            )["outcome"],
            "pass",
        )
        self.assertEqual(
            checks.run_registered_check(
                "convergence",
                {"lhs": sp.srepr(sp.Sum(1 / n, (n, 1, sp.oo))), "rhs": sp.srepr(sp.oo)},
                {"symbols": syms},
            )["outcome"],
            "fail",
        )

        # Sequence convergence 1 + 1/n^2 -> 1.
        seq_ok = checks.run_registered_check(
            "convergence",
            {"lhs": sp.srepr(1 + 1 / n**2), "rhs": sp.srepr(sp.Integer(1))},
            {"symbols": syms, "variable": "n", "levels": [8, 16, 32, 64], "atol": "0.004", "rtol": "0"},
        )
        self.assertEqual(seq_ok["outcome"], "pass")
        self.assertTrue(seq_ok["evidence"]["residual_ratios"])

        # Oscillating sequence fails.
        seq_bad = checks.run_registered_check(
            "convergence",
            {"lhs": sp.srepr((-1) ** n), "rhs": sp.srepr(sp.Integer(0))},
            {"symbols": syms, "variable": "n", "levels": [2, 3, 4, 5], "atol": "0.01", "rtol": "0"},
        )
        self.assertEqual(seq_bad["outcome"], "fail")

    def test_matrix_property_all_seven_properties_and_witnesses(self) -> None:
        theta = sp.Symbol("theta", real=True)
        a = sp.Symbol("a", real=True, positive=True)
        syms = [
            {"name": "theta", "assumptions": ["real"]},
            {"name": "a", "assumptions": ["real", "positive"]},
        ]

        sy = sp.Matrix([[0, -sp.I], [sp.I, 0]])
        u_su2 = sp.cos(theta) * sp.eye(2) - sp.I * sp.sin(theta) * sy
        rot = sp.Matrix([[sp.cos(theta), -sp.sin(theta)], [sp.sin(theta), sp.cos(theta)]])
        pd_mat = sp.Matrix([[a + 2, 1], [1, a + 2]])
        singular = sp.Matrix([[1, 2], [2, 4]])
        non_pd = sp.Matrix([[1, 2], [2, 1]])

        self.assertEqual(
            checks.run_registered_check("matrix_property", {"lhs": sp.srepr(sy), "rhs": sp.srepr(sy)}, {"properties": ["hermitian", "traceless", "unitary", "invertible"]})["outcome"],
            "pass",
        )
        self.assertEqual(
            checks.run_registered_check("matrix_property", {"lhs": sp.srepr(u_su2), "rhs": sp.srepr(u_su2)}, {"symbols": syms, "property": "unitary"})["outcome"],
            "pass",
        )
        self.assertEqual(
            checks.run_registered_check("matrix_property", {"lhs": sp.srepr(rot), "rhs": sp.srepr(rot)}, {"symbols": syms, "properties": ["orthogonal", "invertible"]})["outcome"],
            "pass",
        )
        self.assertEqual(
            checks.run_registered_check("matrix_property", {"lhs": sp.srepr(sy), "rhs": sp.srepr(sy)}, {"property": "symmetric"})["outcome"],
            "fail",
        )
        self.assertEqual(
            checks.run_registered_check("matrix_property", {"lhs": sp.srepr(sy), "rhs": sp.srepr(sy)}, {"property": "orthogonal"})["outcome"],
            "fail",
        )
        self.assertEqual(
            checks.run_registered_check("matrix_property", {"lhs": sp.srepr(pd_mat), "rhs": sp.srepr(pd_mat)}, {"symbols": syms, "property": "positive_definite"})["outcome"],
            "pass",
        )
        self.assertEqual(
            checks.run_registered_check("matrix_property", {"lhs": sp.srepr(non_pd), "rhs": sp.srepr(non_pd)}, {"property": "positive_definite"})["outcome"],
            "fail",
        )
        self.assertEqual(
            checks.run_registered_check("matrix_property", {"lhs": sp.srepr(singular), "rhs": sp.srepr(singular)}, {"property": "invertible"})["outcome"],
            "fail",
        )

    def test_stdio_check_worker_executes_physics_checks_and_blocks_unsafe_srepr(self) -> None:
        with subprocess.Popen(
            [sys.executable, "-u", str(WORKER_SCRIPT), "--stdio", "--role", "check"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        ) as proc:
            try:
                assert proc.stdin is not None and proc.stdout is not None
                hello = json.loads(proc.stdout.readline().decode("utf-8"))
                self.assertEqual(hello["role"], "check")
                self.assertEqual(hello["checks_sha256"], checks.registry_sha256())

                x = sp.Symbol("x", real=True)
                t = sp.Symbol("t", real=True)
                req = {
                    "id": 1,
                    "method": "check",
                    "params": {
                        "name": "dimensions",
                        "handles": {"lhs": sp.srepr(x), "rhs": sp.srepr(t)},
                        "params": {
                            "symbols": [{"name": "x", "assumptions": ["real"]}, {"name": "t", "assumptions": ["real"]}],
                            "dimensions": {"x": {"length": 1}, "t": {"time": 1}},
                        },
                    },
                }
                proc.stdin.write((json.dumps(req) + "\n").encode("utf-8"))
                proc.stdin.flush()
                resp = json.loads(proc.stdout.readline().decode("utf-8"))
                self.assertTrue(resp["ok"], msg=resp)
                self.assertEqual(resp["result"]["outcome"], "fail")

                bad = {
                    "id": 2,
                    "method": "check",
                    "params": {"name": "matrix_property", "handles": {"lhs": "eval('1')", "rhs": "Integer(1)"}, "params": {"property": "hermitian"}},
                }
                proc.stdin.write((json.dumps(bad) + "\n").encode("utf-8"))
                proc.stdin.flush()
                bad_resp = json.loads(proc.stdout.readline().decode("utf-8"))
                self.assertFalse(bad_resp["ok"])
                self.assertEqual(bad_resp["error"]["type"], "UnsafeSreprError")
            finally:
                try:
                    if proc.stdin and proc.stdout and proc.poll() is None:
                        proc.stdin.write((json.dumps({"id": 99, "method": "shutdown"}) + "\n").encode("utf-8"))
                        proc.stdin.flush()
                        proc.stdout.readline()
                except Exception:
                    pass
                if proc.poll() is None:
                    proc.kill()
                proc.wait(timeout=5)


if __name__ == "__main__":
    unittest.main()
