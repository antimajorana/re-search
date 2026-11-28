"""Tests for lazy Matplotlib/SciPy loading and artifact rendering in worker.py."""

import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

PYTHON_DIR = Path(__file__).resolve().parents[1]
WORKER_PY = PYTHON_DIR / "worker.py"
if str(PYTHON_DIR) not in sys.path:
    sys.path.insert(0, str(PYTHON_DIR))

import worker  # noqa: E402


def _has_package(name):
    return importlib.util.find_spec(name) is not None


class WorkerLazyArtifactsTest(unittest.TestCase):
    def _run_stdio(self, role, requests, cwd, clear_mpl_env=True):
        env = dict(os.environ)
        env["PYTHONDONTWRITEBYTECODE"] = "1"
        if clear_mpl_env:
            env.pop("MPLBACKEND", None)
            env.pop("MPLCONFIGDIR", None)
        payload = "\n".join(json.dumps(r) for r in requests) + "\n"
        proc = subprocess.run(
            [sys.executable, "-u", str(WORKER_PY), "--stdio", "--role", role],
            input=payload,
            capture_output=True,
            text=True,
            cwd=str(cwd),
            env=env,
            timeout=90,
        )
        self.assertEqual(proc.returncode, 0, f"worker failed: {proc.stderr}")
        lines = [json.loads(line) for line in proc.stdout.splitlines() if line.strip()]
        return lines

    def test_startup_and_pure_sympy_never_import_scipy_or_matplotlib(self):
        with tempfile.TemporaryDirectory() as tmp:
            scratch = Path(tmp)
            reqs = [
                {
                    "id": 1,
                    "method": "evaluate",
                    "params": {
                        "code": (
                            "import os, sys\n"
                            "x = sympy.Symbol('x')\n"
                            "expr = sympy.simplify((x + 1)**2 - x**2 - 2*x)\n"
                            "({\n"
                            "  'expr': str(expr),\n"
                            "  'scipy': 'scipy' in sys.modules,\n"
                            "  'matplotlib': 'matplotlib' in sys.modules,\n"
                            "  'mplbackend': os.environ.get('MPLBACKEND'),\n"
                            "  'mplconfigdir': os.environ.get('MPLCONFIGDIR'),\n"
                            "})"
                        )
                    },
                },
                {"id": 2, "method": "shutdown", "params": {}},
            ]
            lines = self._run_stdio("solver", reqs, scratch)
            hello = lines[0]
            self.assertTrue(hello["ready"])
            eval_msg = lines[1]
            self.assertTrue(eval_msg["ok"])
            res = eval_msg["result"]
            self.assertIsNone(res.get("error"))
            val = eval(res["value"])  # dict literal from str/bool/None
            self.assertEqual(val["expr"], "1")
            self.assertFalse(val["scipy"])
            self.assertFalse(val["matplotlib"])
            self.assertIsNone(val["mplbackend"])
            self.assertIsNone(val["mplconfigdir"])
            self.assertFalse((scratch / "mplconfig").exists())

    def test_check_role_startup_stays_clean(self):
        with tempfile.TemporaryDirectory() as tmp:
            lines = self._run_stdio(
                "check",
                [{"id": 1, "method": "shutdown", "params": {}}],
                Path(tmp),
            )
            self.assertTrue(lines[0]["ready"])
            self.assertEqual(lines[0]["role"], "check")
            self.assertTrue(lines[1]["ok"])

    @unittest.skipUnless(_has_package("matplotlib"), "matplotlib not installed")
    def test_lazy_matplotlib_config_and_png_artifact_in_stdio_worker(self):
        with tempfile.TemporaryDirectory() as tmp:
            scratch = Path(tmp)
            reqs = [
                {
                    "id": 1,
                    "method": "evaluate",
                    "params": {
                        "code": (
                            "import os, pathlib\n"
                            "import matplotlib\n"
                            "import matplotlib.pyplot as plt\n"
                            "pathlib.Path('artifacts').mkdir(parents=True, exist_ok=True)\n"
                            "fig, ax = plt.subplots()\n"
                            "ax.plot([0, 1, 2], [0, 1, 4])\n"
                            "fig.savefig('artifacts/curve.png')\n"
                            "plt.close(fig)\n"
                            "(matplotlib.get_backend(), os.environ.get('MPLBACKEND'), os.environ.get('MPLCONFIGDIR'))"
                        )
                    },
                },
                {"id": 2, "method": "shutdown", "params": {}},
            ]
            lines = self._run_stdio("solver", reqs, scratch)
            eval_msg = lines[1]
            self.assertTrue(eval_msg["ok"])
            res = eval_msg["result"]
            self.assertIsNone(res.get("error"))
            backend, env_backend, env_cfg = eval(res["value"])
            self.assertEqual(backend.lower(), "agg")
            self.assertEqual(env_backend, "Agg")
            expected_cfg = str(scratch.resolve() / "mplconfig")
            self.assertEqual(str(Path(env_cfg).resolve()), expected_cfg)
            self.assertTrue((scratch / "mplconfig").is_dir())
            png_bytes = (scratch / "artifacts" / "curve.png").read_bytes()
            self.assertTrue(png_bytes.startswith(b"\x89PNG\r\n\x1a\n"))

    @unittest.skipUnless(_has_package("matplotlib"), "matplotlib not installed")
    def test_evaluate_direct_importlib_triggers_lazy_matplotlib_env(self):
        saved_mods = {
            name: mod
            for name, mod in list(sys.modules.items())
            if name == "matplotlib" or name.startswith("matplotlib.") or name == "pylab"
        }
        saved_backend = os.environ.pop("MPLBACKEND", None)
        saved_cfg = os.environ.pop("MPLCONFIGDIR", None)
        try:
            for name in list(saved_mods):
                sys.modules.pop(name, None)
            with tempfile.TemporaryDirectory() as tmp:
                scratch = Path(tmp)
                ns = worker.make_namespace()
                r1 = worker.evaluate("sympy.Symbol('x') + 2", ns, scratch_dir=scratch)
                self.assertIsNone(r1.get("error"))
                self.assertNotIn("matplotlib", sys.modules)
                self.assertFalse((scratch / "mplconfig").exists())

                r2 = worker.evaluate(
                    "import importlib, os\nmpl = importlib.import_module('matplotlib')\n"
                    "(mpl.get_backend(), os.environ.get('MPLCONFIGDIR'))",
                    ns,
                    scratch_dir=scratch,
                )
                self.assertIsNone(r2.get("error"))
                backend, cfg = eval(r2["value"])
                self.assertEqual(backend.lower(), "agg")
                self.assertEqual(Path(cfg).resolve(), (scratch / "mplconfig").resolve())
                self.assertTrue((scratch / "mplconfig").is_dir())
        finally:
            for name in list(sys.modules):
                if name == "matplotlib" or name.startswith("matplotlib.") or name == "pylab":
                    sys.modules.pop(name, None)
            sys.modules.update(saved_mods)
            if saved_backend is None:
                os.environ.pop("MPLBACKEND", None)
            else:
                os.environ["MPLBACKEND"] = saved_backend
            if saved_cfg is None:
                os.environ.pop("MPLCONFIGDIR", None)
            else:
                os.environ["MPLCONFIGDIR"] = saved_cfg

    @unittest.skipUnless(_has_package("scipy"), "scipy not installed")
    def test_lazy_scipy_import_works_inside_evaluate(self):
        with tempfile.TemporaryDirectory() as tmp:
            reqs = [
                {
                    "id": 1,
                    "method": "evaluate",
                    "params": {
                        "code": (
                            "import sys\n"
                            "had_scipy_before = 'scipy' in sys.modules\n"
                            "from scipy import integrate\n"
                            "val, _ = integrate.quad(lambda t: t**2, 0.0, 1.0)\n"
                            "(had_scipy_before, round(val, 6), 'scipy' in sys.modules)"
                        )
                    },
                },
                {"id": 2, "method": "shutdown", "params": {}},
            ]
            lines = self._run_stdio("solver", reqs, Path(tmp))
            res = lines[1]["result"]
            self.assertIsNone(res.get("error"))
            had_before, val, has_after = eval(res["value"])
            self.assertFalse(had_before)
            self.assertAlmostEqual(val, 0.333333, places=5)
            self.assertTrue(has_after)


if __name__ == "__main__":
    unittest.main()
