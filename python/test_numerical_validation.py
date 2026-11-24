import json
import unittest
from decimal import Decimal
from fractions import Fraction
from itertools import count

from mpmath import mp

from numerical_validation import check_convergence


class ConvergenceTests(unittest.TestCase):
    def test_resolution_requires_consecutive_tail_and_reference(self):
        report = check_convergence(
            lambda n: 1 + mp.mpf(1) / n**2,
            [8, 16, 32, 64],
            atol="0.004",
            rtol=0,
            reference=1,
        )
        self.assertEqual(report["kind"], "numerical_convergence")
        self.assertEqual(report["schema_version"], 1)
        self.assertEqual(report["status"], "within_tolerance")
        self.assertIs(report["stable"], True)
        self.assertIs(report["within_tolerance"], True)
        self.assertIs(report["reference_within_tolerance"], True)
        self.assertEqual(
            [sample["passes"] for sample in report["samples"]],
            [None, False, True, True],
        )
        self.assertEqual(report["samples"][2]["delta"], "0.0029296875")
        self.assertEqual(report["samples"][3]["reference_error"], "0.000244140625")
        self.assertEqual(json.loads(json.dumps(report, allow_nan=False)), report)

    def test_plateau_can_remain_outside_reference_tolerance(self):
        report = check_convergence(
            lambda n: Decimal("1.11"), [1, 2, 3], atol=0, rtol=Fraction(1, 10), reference=1
        )
        self.assertEqual(report["status"], "outside_reference_tolerance")
        self.assertIs(report["stable"], True)
        self.assertIs(report["reference_within_tolerance"], False)
        self.assertIs(report["within_tolerance"], False)
        self.assertEqual(report["samples"][2]["reference_threshold"], "0.1")

    def test_decimal_tolerance_boundary_matches_recorded_values(self):
        for atol, rtol, reference, second, expected in [
            ("0.1", "0", "1.2", "1.3", "within_tolerance"),
            ("0.1", "0", "1.2", "1.300000000000000001", "not_stable"),
            ("0", "0.1", "1.2", "1.32", "within_tolerance"),
            ("0", "0.1", "1.2", "1.320000000000000001", "outside_reference_tolerance"),
        ]:
            with self.subTest(atol=atol, rtol=rtol, second=second):
                values = [Decimal(reference), Decimal(second)]
                report = check_convergence(
                    lambda level: values[level - 1], [1, 2], atol=atol, rtol=rtol,
                    reference=reference, tail=1, work_dps=30,
                )
                self.assertEqual(report["status"], expected)

    def test_oscillation_fails_when_only_one_recent_difference_passes(self):
        results = {1: 0, 2: 1, 3: 0, 4: Decimal("0.01")}
        report = check_convergence(results.__getitem__, [1, 2, 3, 4], atol="0.1", rtol=0, axis="basis")
        self.assertEqual(report["status"], "not_stable")
        self.assertIs(report["stable"], False)
        self.assertIs(report["within_tolerance"], False)
        self.assertEqual([sample["passes"] for sample in report["samples"][-2:]], [False, True])

    def test_precision_axis_uses_requested_context_and_preserves_small_differences(self):
        context_before = mp.dps
        with mp.workdps(110):
            reference = mp.sqrt(2)
        observed_precisions = []

        def square_root(_level):
            observed_precisions.append(mp.dps)
            return mp.sqrt(2)

        report = check_convergence(
            square_root, [30, 60, 90], atol="1e-50", rtol=0, reference=reference, axis="precision", tail=1
        )
        self.assertEqual(observed_precisions, [30, 60, 90])
        self.assertEqual([sample["level"] for sample in report["samples"]], [30, 60, 90])
        self.assertEqual(report["analysis_dps"], 95)
        self.assertIs(report["stable"], True)
        self.assertIs(report["reference_within_tolerance"], True)
        self.assertGreater(mp.mpf(report["samples"][1]["delta"]), mp.mpf("1e-35"))
        self.assertGreater(mp.mpf(report["samples"][2]["delta"]), 0)
        self.assertLess(mp.mpf(report["samples"][2]["delta"]), mp.mpf("1e-59"))
        self.assertEqual(mp.dps, context_before)

    def test_resolution_work_precision_and_complex_norm(self):
        context_before = mp.dps
        observed_precisions = []

        def wave(n):
            observed_precisions.append(mp.dps)
            return mp.mpc(1, mp.mpf(1) / n)

        report = check_convergence(
            wave, [16, 32, 64], atol="0.1", rtol=0, reference="1", work_dps=60
        )
        self.assertEqual(observed_precisions, [60, 60, 60])
        self.assertEqual(report["status"], "within_tolerance")
        self.assertEqual(report["samples"][2]["value"], {"real": "1.0", "imag": "0.015625"})
        self.assertEqual(report["samples"][2]["delta"], "0.015625")
        self.assertEqual(mp.dps, context_before)

    def test_failed_sample_breaks_adjacency_and_prevents_overall_acceptance(self):
        context_before = mp.dps

        def unstable_call(level):
            if level == 2:
                mp.dps = 70
                raise ArithmeticError("failed evaluation")
            return mp.mpf(1)

        report = check_convergence(unstable_call, [1, 2, 3, 4], atol=0, rtol=0, tail=1)
        self.assertEqual(report["status"], "evaluation_failed")
        self.assertIs(report["within_tolerance"], None)
        self.assertIs(report["stable"], True)
        self.assertEqual(report["samples"][1]["error"], {"type": "ArithmeticError", "message": "failed evaluation"})
        self.assertIs(report["samples"][2]["delta"], None)
        self.assertEqual(report["samples"][3]["delta"], "0.0")
        self.assertEqual(mp.dps, context_before)

    def test_insufficient_tail_and_nonfinite_values(self):
        short = check_convergence(lambda n: 1, [1, 2], atol=0, rtol=0)
        self.assertEqual(short["status"], "insufficient_data")
        self.assertIs(short["stable"], None)
        failed = check_convergence(lambda n: mp.inf if n == 2 else 1, [1, 2, 3], atol=0, rtol=0, tail=1)
        self.assertEqual(failed["status"], "evaluation_failed")
        self.assertIs(failed["stable"], None)
        self.assertIn("finite", failed["samples"][1]["error"]["message"])

    def test_rejects_invalid_controls_before_running(self):
        def unexpected(_):
            self.fail("callback ran for invalid controls")

        cases = [
            ([], {}),
            ([1, 1], {}),
            ([2, 1], {}),
            ([0, 1], {}),
            ([1, mp.inf], {}),
            ([1, mp.nan], {}),
            ([1, True], {}),
            ([1, 10**9 + 1], {}),
            (count(1), {}),
            ([1, 4], {"tail": 32}),
            ([1, 4], {"axis": "unknown"}),
            ([1, 4.0], {"axis": "precision"}),
            ([1, 257], {"axis": "precision"}),
            ([1, 4], {"work_dps": 257}),
            ([1, 4], {"atol": -1}),
            ([1, 4], {"atol": "nan"}),
            ([1, 4], {"rtol": "inf"}),
            ([1, 4], {"reference": mp.inf}),
        ]
        for levels, controls in cases:
            with self.subTest(levels=levels, controls=controls), self.assertRaises(ValueError):
                check_convergence(unexpected, levels, **{"atol": 0, "rtol": 0, **controls})


if __name__ == "__main__":
    unittest.main()
