"""Regression tests for Worker Protocol 2, check-process role, and rules R5 & R6 (S6)."""

from __future__ import annotations

import hashlib
import io
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import sympy as sp

import checks
import worker

WORKER_SCRIPT = Path(__file__).resolve().with_name("worker.py")


class _WorkerClient:
    """Minimal cross-platform JSON-lines client for `worker.py --stdio`."""

    def __init__(self, role: str | None = None, cwd: Path | None = None) -> None:
        cmd = [sys.executable, "-u", str(WORKER_SCRIPT), "--stdio"]
        if role is not None:
            cmd.extend(["--role", role])
        self.proc = subprocess.Popen(
            cmd,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            cwd=str(cwd) if cwd is not None else None,
        )
        assert self.proc.stdin is not None and self.proc.stdout is not None
        line = self.proc.stdout.readline()
        if not line:
            stderr_out = self.proc.stderr.read().decode("utf-8", errors="replace") if self.proc.stderr else ""
            raise RuntimeError(f"worker exited without hello; stderr={stderr_out!r}")
        self.hello = json.loads(line.decode("utf-8"))
        self._next_id = 1

    def call(self, method: str, params: dict | None = None) -> dict:
        assert self.proc.stdin is not None and self.proc.stdout is not None
        req_id = self._next_id
        self._next_id += 1
        req: dict = {"id": req_id, "method": method}
        if params is not None:
            req["params"] = params
        self.proc.stdin.write((json.dumps(req) + "\n").encode("utf-8"))
        self.proc.stdin.flush()
        raw = self.proc.stdout.readline()
        if not raw:
            raise RuntimeError("worker closed stdout unexpectedly")
        resp = json.loads(raw.decode("utf-8"))
        assert resp.get("id") == req_id, f"mismatched id: {resp}"
        return resp

    def close(self) -> None:
        try:
            if self.proc.poll() is None:
                self.call("shutdown")
        except Exception:
            pass
        finally:
            for stream in (self.proc.stdin, self.proc.stdout, self.proc.stderr):
                if stream is not None:
                    try:
                        stream.close()
                    except Exception:
                        pass
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait(timeout=5)


class WorkerProtocol2Tests(unittest.TestCase):
    def test_protocol1_backwards_compat_and_stdio_isolation(self):
        with tempfile.TemporaryDirectory() as tmp:
            client = _WorkerClient(role=None, cwd=Path(tmp))
            try:
                self.assertTrue(client.hello["ready"])
                self.assertEqual(client.hello["protocol"], 2)
                self.assertEqual(client.hello["role"], "solver")
                self.assertEqual(client.hello["checks_sha256"], checks.registry_sha256())

                code = "x = Symbol('x')\nprint('hello-stdout')\nintegrate(x * sin(x), x)"
                resp = client.call("evaluate", {"code": code})
                self.assertTrue(resp["ok"], msg=resp)
                res = resp["result"]
                # All protocol 1 fields intact
                self.assertEqual(res["value"], "-x*cos(x) + sin(x)")
                self.assertEqual(res["stdout"], "hello-stdout\n")
                self.assertEqual(res["stderr"], "")
                self.assertFalse(res["truncated"])
                self.assertEqual(res["seed_events"], [])
                self.assertIn("latex", res)
                self.assertIn("comparison", res)
                self.assertNotIn("error", res)
                # Additive protocol 2 fields present
                self.assertEqual(res["exprs_out"], {})
                self.assertEqual(res["code_sha256"], hashlib.sha256(code.encode("utf-8")).hexdigest())
                self.assertIsInstance(res["wall_ms"], int)
                self.assertGreaterEqual(res["wall_ms"], 0)
                self.assertEqual(res["process"], "solver")
                self.assertEqual(res["env"]["protocol"], 2)

                # Solver role forbids check-only methods
                for forbidden in ("canonicalize", "check"):
                    bad = client.call(forbidden, {})
                    self.assertFalse(bad["ok"])
                    self.assertEqual(bad["error"]["type"], "PermissionError")
            finally:
                client.close()

    def test_protocol2_exprs_in_outbox_and_no_scratch_bus_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            client = _WorkerClient(role="solver", cwd=tmp_path)
            try:
                # Call 1: save_expr writes ONLY to protocol 2 outbox, never scratch/.cas_bus.json
                r1 = client.call(
                    "evaluate",
                    {
                        "code": (
                            "m = assumed_symbol('m', positive=True, real=True)\n"
                            "save_expr('e1', (m + 1)**2)\n"
                            "same = expr('e1')\n"
                            "save_expr('e1_copy', same)\n"
                            "same"
                        )
                    },
                )
                self.assertTrue(r1["ok"], msg=r1)
                res1 = r1["result"]
                self.assertNotIn("error", res1)
                self.assertIn("e1", res1["exprs_out"])
                self.assertIn("e1_copy", res1["exprs_out"])
                self.assertEqual(res1["exprs_out"]["e1"], res1["exprs_out"]["e1_copy"])
                self.assertFalse((tmp_path / "scratch" / ".cas_bus.json").exists())

                # Call 2: load from exprs_in via expr() and legacy load_expr()
                r2 = client.call(
                    "evaluate",
                    {
                        "code": (
                            "loaded = expr('e1')\n"
                            "also = load_expr('e1')\n"
                            "assert loaded == also\n"
                            "save_expr('expanded', expand(loaded))\n"
                            "expand(loaded)"
                        ),
                        "exprs_in": {"e1": res1["exprs_out"]["e1"]},
                    },
                )
                self.assertTrue(r2["ok"], msg=r2)
                res2 = r2["result"]
                self.assertNotIn("error", res2)
                self.assertEqual(res2["value"], "m**2 + 2*m + 1")
                self.assertIn("expanded", res2["exprs_out"])
                self.assertFalse((tmp_path / "scratch" / ".cas_bus.json").exists())

                # Call 3: hostile exprs_in is blocked by srepr_safe inside expr()
                r3 = client.call(
                    "evaluate",
                    {
                        "code": "expr('poison')",
                        "exprs_in": {"poison": "__import__('os').system('echo pwned')"},
                    },
                )
                self.assertTrue(r3["ok"])
                self.assertEqual(r3["result"]["error"]["type"], "UnsafeSreprError")
            finally:
                client.close()

    def test_check_role_startup_forbids_evaluate_and_solver_calc(self):
        client = _WorkerClient(role="check")
        try:
            self.assertTrue(client.hello["ready"])
            self.assertEqual(client.hello["protocol"], 2)
            self.assertEqual(client.hello["role"], "check")
            self.assertEqual(client.hello["checks_sha256"], checks.registry_sha256())

            # evaluate is strictly forbidden
            ev = client.call("evaluate", {"code": "1 + 1"})
            self.assertFalse(ev["ok"])
            self.assertEqual(ev["error"]["type"], "PermissionError")
            self.assertIn("forbidden", ev["error"]["message"].lower())

            # canonicalize single expression
            x = sp.Symbol("x", real=True)
            raw = sp.srepr(x**2 + 1)
            c1 = client.call("canonicalize", {"srepr": raw})
            self.assertTrue(c1["ok"], msg=c1)
            self.assertTrue(c1["result"]["portable"])
            self.assertEqual(c1["result"]["latex"], "x^{2} + 1")
            self.assertEqual(c1["result"]["process"], "check")
            self.assertEqual(c1["result"]["free_symbols"], [{"name": "x", "assumptions": ["real"]}])

            # canonicalize batch
            cb = client.call("canonicalize", {"exprs": {"a": raw, "bad": "eval('1')"}})
            self.assertTrue(cb["ok"], msg=cb)
            self.assertTrue(cb["result"]["exprs"]["a"]["portable"])
            self.assertFalse(cb["result"]["exprs"]["bad"]["portable"])
            self.assertEqual(cb["result"]["exprs"]["bad"]["rejected_constructor"], "eval")

            # check rejects unknown check name
            unk = client.call("check", {"name": "no_such_check", "handles": {"a": raw}})
            self.assertFalse(unk["ok"])
            self.assertEqual(unk["error"]["type"], "UnknownCheckError")

            # check rejects hostile srepr handle
            bad_h = client.call(
                "check",
                {"name": "symbolic_zero_smoke", "handles": {"lhs": "eval('0')", "rhs": "Integer(0)"}},
            )
            self.assertFalse(bad_h["ok"])
            self.assertEqual(bad_h["error"]["type"], "UnsafeSreprError")
            self.assertEqual(bad_h["error"]["rejected_constructor"], "eval")
        finally:
            client.close()

    def test_rule_r5_solver_patched_latex_cannot_alter_check_process_rendering(self):
        solver = _WorkerClient(role="solver")
        checker = _WorkerClient(role="check")
        try:
            # Solver monkey-patches sympy.latex globally inside its own process
            r = solver.call(
                "evaluate",
                {
                    "code": (
                        "import sympy\n"
                        "sympy.latex = lambda *a, **k: 'FAKE_POISONED_LATEX'\n"
                        "x = Symbol('x')\n"
                        "e = x**2 + 1\n"
                        "save_expr('e1', e)\n"
                        "e"
                    )
                },
            )
            self.assertTrue(r["ok"], msg=r)
            # Confirm the monkey-patch actually took effect in the solver process
            self.assertEqual(r["result"]["latex"], "FAKE_POISONED_LATEX")
            srepr_e1 = r["result"]["exprs_out"]["e1"]

            # Rule R5: check process renders the stored handle independently
            canon = checker.call("canonicalize", {"srepr": srepr_e1})
            self.assertTrue(canon["ok"], msg=canon)
            self.assertTrue(canon["result"]["portable"])
            self.assertEqual(canon["result"]["latex"], "x^{2} + 1")
            self.assertNotEqual(canon["result"]["latex"], "FAKE_POISONED_LATEX")
        finally:
            solver.close()
            checker.close()

    def test_rule_r6_solver_patched_simplify_cannot_alter_check_process_outcomes(self):
        solver = _WorkerClient(role="solver")
        checker = _WorkerClient(role="check")
        try:
            # Solver monkey-patches sympy.simplify to always return 0!
            r = solver.call(
                "evaluate",
                {
                    "code": (
                        "import sympy\n"
                        "sympy.simplify = lambda *a, **k: sympy.Integer(0)\n"
                        "x = Symbol('x')\n"
                        "save_expr('false_lhs', x)\n"
                        "save_expr('false_rhs', x + 1)\n"
                        "save_expr('true_lhs', (x + 1)**2)\n"
                        "save_expr('true_rhs', x**2 + 2*x + 1)\n"
                        "sympy.simplify(x - (x + 1))"
                    )
                },
            )
            self.assertTrue(r["ok"], msg=r)
            # Confirm solver's own simplify is compromised (returns 0 for -1)
            self.assertEqual(r["result"]["value"], "0")
            out = r["result"]["exprs_out"]

            # Rule R6: separate check process is completely unaffected
            bad_chk = checker.call(
                "check",
                {
                    "name": "symbolic_zero_smoke",
                    "handles": {"lhs": out["false_lhs"], "rhs": out["false_rhs"]},
                },
            )
            self.assertTrue(bad_chk["ok"], msg=bad_chk)
            self.assertEqual(bad_chk["result"]["outcome"], "inconclusive")
            self.assertFalse(bad_chk["result"]["evidence"]["is_zero"])
            self.assertEqual(bad_chk["result"]["evidence"]["residual_latex"], "-1")

            good_chk = checker.call(
                "check",
                {
                    "name": "symbolic_zero_smoke",
                    "handles": {"lhs": out["true_lhs"], "rhs": out["true_rhs"]},
                },
            )
            self.assertTrue(good_chk["ok"], msg=good_chk)
            self.assertEqual(good_chk["result"]["outcome"], "pass")
            self.assertTrue(good_chk["result"]["evidence"]["is_zero"])
            self.assertEqual(good_chk["result"]["evidence"]["residual_latex"], "0")
        finally:
            solver.close()
            checker.close()

    def test_in_process_serve_streams_check_role(self):
        reqs = io.BytesIO(
            (
                json.dumps({"id": 1, "method": "canonicalize", "params": {"srepr": "Integer(42)"}}) + "\n"
                + json.dumps({"id": 2, "method": "evaluate", "params": {"code": "1+1"}}) + "\n"
                + json.dumps({"id": 3, "method": "shutdown"}) + "\n"
            ).encode("utf-8")
        )
        resps = io.BytesIO()
        rc = worker.serve_streams(reqs, resps, role="check")
        self.assertEqual(rc, 0)
        lines = [json.loads(line) for line in resps.getvalue().decode("utf-8").splitlines()]
        self.assertEqual(len(lines), 4)
        self.assertTrue(lines[0]["ready"])
        self.assertEqual(lines[0]["role"], "check")
        self.assertTrue(lines[1]["ok"])
        self.assertEqual(lines[1]["result"]["latex"], "42")
        self.assertFalse(lines[2]["ok"])
        self.assertEqual(lines[2]["error"]["type"], "PermissionError")
        self.assertTrue(lines[3]["ok"])

    def test_check_role_never_imports_solver_calc(self):
        probe = (
            "import io, json, sys; "
            f"sys.path.insert(0, {str(WORKER_SCRIPT.parent)!r}); "
            "import worker; "
            "reqs = io.BytesIO((json.dumps({'id': 1, 'method': 'shutdown'}) + '\\n').encode()); "
            "resps = io.BytesIO(); "
            "rc = worker.serve_streams(reqs, resps, role='check'); "
            "assert rc == 0; "
            "assert 'solver_calc' not in sys.modules, 'check role must not import solver_calc'"
        )
        completed = subprocess.run([sys.executable, "-c", probe], capture_output=True, text=True)
        self.assertEqual(completed.returncode, 0, msg=completed.stderr)


if __name__ == "__main__":
    unittest.main()
