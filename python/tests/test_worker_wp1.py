"""Regression tests for WP1 provenance capture in `worker.py` and `solver_calc.py`."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

import sympy as sp

PYTHON_DIR = Path(__file__).resolve().parents[1]
if str(PYTHON_DIR) not in sys.path:
    sys.path.insert(0, str(PYTHON_DIR))

import solver_calc  # noqa: E402
import worker  # noqa: E402


class Wp1WorkerProvenanceTests(unittest.TestCase):
    def test_ast_source_expr_and_unevaluated_srepr_for_strings_and_lazy_objects(self) -> None:
        ns = worker.make_namespace()
        code = (
            "x = assumed_symbol('x', real=True)\n"
            "A = MatrixSymbol('A', 2, 2)\n"
            "B = MatrixSymbol('B', 2, 2)\n"
            "save_expr('lhs_str', '(x + 1)**2', locals={'x': x})\n"
            "save_expr('arith_str', '2 + 3')\n"
            "save_expr(name='kw_rhs', expr=x**2 + 2*x + 1)\n"
            "solver_calc.save_expr('mod_u', UnevaluatedExpr((x + 1)**2))\n"
            "save_expr('int_lazy', Integral(x, (x, 0, 1)))\n"
            "save_expr('mat_mul', MatMul(A, B))\n"
            "save_expr('tr_expr', Trace(MatMul(A, B)))\n"
            "save_expr('plain_num', Integer(7))\n"
        )
        res = worker.evaluate(code, ns)
        self.assertNotIn("error", res, msg=res)

        out = res["exprs_out"]
        meta = res["exprs_out_meta"]

        # Backward-compatible exprs_out remains {name: srepr_str}
        for key, val in out.items():
            self.assertIsInstance(key, str)
            self.assertIsInstance(val, str)

        self.assertEqual(out["arith_str"], "Integer(5)")
        self.assertEqual(meta["arith_str"]["srepr"], "Integer(5)")
        self.assertEqual(meta["arith_str"]["unevaluated_srepr"], "Add(Integer(2), Integer(3))")
        self.assertEqual(meta["arith_str"]["source_expr"], "'2 + 3'")

        self.assertEqual(
            meta["lhs_str"]["unevaluated_srepr"],
            "Pow(Add(Symbol('x', real=True), Integer(1)), Integer(2))",
        )
        self.assertEqual(meta["lhs_str"]["source_expr"], "'(x + 1)**2'")

        self.assertEqual(meta["kw_rhs"]["source_expr"], "x**2 + 2*x + 1")
        self.assertNotIn("unevaluated_srepr", meta["kw_rhs"])

        self.assertEqual(
            meta["mod_u"]["unevaluated_srepr"],
            "UnevaluatedExpr(Pow(Add(Symbol('x', real=True), Integer(1)), Integer(2)))",
        )
        self.assertEqual(meta["mod_u"]["source_expr"], "UnevaluatedExpr((x + 1)**2)")

        for lazy_name, expected_src in (
            ("int_lazy", "Integral(x, (x, 0, 1))"),
            ("mat_mul", "MatMul(A, B)"),
            ("tr_expr", "Trace(MatMul(A, B))"),
        ):
            self.assertEqual(meta[lazy_name]["srepr"], out[lazy_name])
            self.assertEqual(meta[lazy_name]["unevaluated_srepr"], out[lazy_name])
            self.assertEqual(meta[lazy_name]["source_expr"], expected_src)

        self.assertEqual(meta["plain_num"]["srepr"], "Integer(7)")
        self.assertEqual(meta["plain_num"]["source_expr"], "Integer(7)")
        self.assertNotIn("unevaluated_srepr", meta["plain_num"])

    def test_sequential_save_expr_overwrite_and_same_line_calls(self) -> None:
        ns = worker.make_namespace()
        code = (
            "x = assumed_symbol('x', real=True)\n"
            "save_expr('dup', x + 1)\n"
            "save_expr('dup', x + 2)\n"
            "save_expr('s1', 10); save_expr('s2', 20)\n"
        )
        res = worker.evaluate(code, ns)
        self.assertNotIn("error", res, msg=res)
        self.assertEqual(res["exprs_out"]["dup"], sp.srepr(sp.Symbol("x", real=True) + 2))
        self.assertEqual(res["exprs_out_meta"]["dup"]["source_expr"], "x + 2")
        self.assertEqual(res["exprs_out_meta"]["s1"]["source_expr"], "10")
        self.assertEqual(res["exprs_out_meta"]["s2"]["source_expr"], "20")

    def test_save_expr_standalone_string_with_locals(self) -> None:
        x = sp.Symbol("x", positive=True)
        with solver_calc.expr_context() as outbox:
            encoded = solver_calc.save_expr("e", "(x + 1)**2", locals={"x": x})
        self.assertEqual(
            encoded,
            "Pow(Add(Symbol('x', positive=True), Integer(1)), Integer(2))",
        )
        self.assertEqual(outbox["e"], encoded)


if __name__ == "__main__":
    unittest.main()
