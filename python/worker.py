"""Compute worker for re:Search (v0).

Started by the Rust `worker` crate as:

    python worker.py <request-handle> <response-handle>

The two arguments are Windows handle values of two inherited anonymous pipes.
The protocol is one JSON object per line (UTF-8) on those pipes and nowhere
else: stdin, stdout and stderr of this process are not part of it, so code
that prints cannot corrupt it.

Requests:   {"id": 1, "method": "evaluate", "params": {"code": "..."}}
            {"id": 2, "method": "shutdown"}
Responses:  {"id": 1, "ok": true,  "result": {...}}
            {"id": 1, "ok": false, "error": {"type": ..., "message": ...}}
On start the worker sends {"ready": true, ...} once, before any request.

This process is NOT a sandbox. The code it runs has the user's full rights.
"""

import ast
import contextlib
import contextvars
import hashlib
import importlib.metadata
import io
import json
import math
import os
import random
import sys
import traceback
from pathlib import Path

PROTOCOL_VERSION = 1
MAX_TEXT = 64_000  # characters kept of each of stdout, stderr, value
MAX_COMPARISON_TEXT = 8_000
MAX_VALIDATION_TEXT = 64_000
MAX_SEED_EVENTS = 32
_active_seeds = contextvars.ContextVar("active_seeds", default=None)


def open_pipes(argv):
    if len(argv) != 3:
        raise SystemExit("usage: worker.py <request-handle> <response-handle>")
    import msvcrt

    fd_in = msvcrt.open_osfhandle(int(argv[1]), os.O_RDONLY | os.O_BINARY)
    fd_out = msvcrt.open_osfhandle(int(argv[2]), os.O_BINARY)
    return os.fdopen(fd_in, "rb", buffering=0), os.fdopen(fd_out, "wb", buffering=0)


def clip(text):
    if len(text) <= MAX_TEXT:
        return text, False
    return text[:MAX_TEXT], True


def set_random_seed(seed):
    """Seed Python's random module and NumPy's legacy generator when available."""
    if type(seed) is not int or not 0 <= seed < 2**32:
        raise ValueError("seed must be an integer from 0 through 2**32 - 1")
    generators = ["python.random"]
    try:
        import numpy
    except ImportError:
        pass
    else:
        numpy.random.seed(seed)
        generators.append("numpy.random")
    random.seed(seed)

    receipt = _active_seeds.get()
    if receipt is not None:
        if len(receipt["seed_events"]) < MAX_SEED_EVENTS:
            receipt["seed_events"].append({"seed": seed, "generators": generators})
        else:
            receipt["seeds_truncated"] = True


def runtime_environment():
    """Return version and source hashes from the active Python runtime."""
    import sympy

    info = {
        "protocol": PROTOCOL_VERSION,
        "python": sys.version.split()[0],
        "sympy": sympy.__version__,
        "platform": sys.platform,
    }
    for package in ("numpy", "mpmath"):
        try:
            info[package] = importlib.metadata.version(package)
        except (importlib.metadata.PackageNotFoundError, OSError, ValueError):
            pass
    script = Path(__file__)
    for key, path in (
        ("worker_sha256", script),
        ("solver_calc_sha256", script.with_name("solver_calc.py")),
        ("symbolic_validation_sha256", script.with_name("symbolic_validation.py")),
        ("numerical_validation_sha256", script.with_name("numerical_validation.py")),
    ):
        try:
            info[key] = hashlib.sha256(path.read_bytes()).hexdigest()
        except OSError:
            pass
    return info


def make_namespace():
    ns = {"__name__": "__worker__"}
    exec("import sympy\nfrom sympy import *", ns)
    script_dir = os.path.dirname(os.path.abspath(__file__))
    if script_dir and script_dir not in sys.path:
        sys.path.insert(0, script_dir)
    try:
        import solver_calc  # noqa: F401

        ns["solver_calc"] = solver_calc
        for name in getattr(solver_calc, "__all__", ()):
            ns[name] = getattr(solver_calc, name)
    except Exception:
        pass
    for module_name, helper in (
        ("symbolic_validation", "check_symbolic_identity"),
        ("numerical_validation", "check_convergence"),
    ):
        try:
            module = importlib.import_module(module_name)
        except ModuleNotFoundError as error:
            if error.name != module_name:
                raise
            continue
        ns[helper] = getattr(module, helper)
    ns["set_random_seed"] = set_random_seed
    return ns


def comparison_value(value):
    """Return an exact representation for supported values, independent of repr addresses."""
    import sympy

    value_type = type(value)
    if value_type is bool:
        kind, text = "bool", str(value)
    elif value_type is int:
        kind, text = "int", str(value)
    elif value_type is str:
        kind, text = "text", value
    elif value_type is float and math.isfinite(value):
        kind, text = "float", value.hex()
    elif value_type is complex and math.isfinite(value.real) and math.isfinite(value.imag):
        kind, text = "complex", f"{value.real.hex()},{value.imag.hex()}"
    elif isinstance(value, sympy.Basic):
        kind, text = "sympy", sympy.srepr(value)
    else:
        return None
    if len(text) <= MAX_COMPARISON_TEXT:
        return {"kind": kind, "value": text}
    return None


def evaluate(code, ns):
    """Run `code`; if its last statement is an expression, return its value."""
    out, err = io.StringIO(), io.StringIO()
    value = None
    has_value = False
    error = None
    result = {"seed_events": []}
    token = _active_seeds.set(result)
    try:
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            try:
                tree = ast.parse(code, filename="<evaluate>", mode="exec")
                last = None
                if tree.body and isinstance(tree.body[-1], ast.Expr):
                    last = ast.Expression(tree.body.pop().value)
                exec(compile(tree, "<evaluate>", "exec"), ns)
                if last is not None:
                    value = eval(compile(last, "<evaluate>", "eval"), ns)
                    has_value = value is not None
            except BaseException as exc:  # includes SystemExit raised by the code
                error = {
                    "type": type(exc).__name__,
                    "message": str(exc),
                    "traceback": traceback.format_exc(limit=8),
                }
    finally:
        _active_seeds.reset(token)
    truncated = False
    for key, text in (("stdout", out.getvalue()), ("stderr", err.getvalue())):
        result[key], cut = clip(text)
        truncated = truncated or cut
    if has_value:
        result["value"], cut = clip(repr(value))
        truncated = truncated or cut
        try:
            if (type(value) is dict and type(value.get("schema_version")) is int
                    and value["schema_version"] == 1
                    and value.get("kind") in ("symbolic_identity", "numerical_convergence")):
                encoded = json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False)
                if len(encoded) <= MAX_VALIDATION_TEXT:
                    result["validation"] = json.loads(encoded)
                    comparison = {"kind": "validation_sha256", "value": hashlib.sha256(encoded.encode()).hexdigest()}
                else:
                    result["validation_truncated"] = True
                    comparison = None
            else:
                comparison = comparison_value(value)
            if comparison is not None:
                result["comparison"] = comparison
        except Exception:
            pass
        try:
            import sympy

            if isinstance(value, sympy.Basic):
                result["latex"], _ = clip(sympy.latex(value))
        except Exception:
            pass
    else:
        result["value"] = None
    result["truncated"] = truncated
    if error is not None:
        result["error"] = error
    return result


def main(argv):
    requests, responses = open_pipes(argv)

    def send(obj):
        responses.write((json.dumps(obj, ensure_ascii=True) + "\n").encode("utf-8"))

    try:
        ns = make_namespace()
        send({
            "ready": True,
            "pid": os.getpid(),
            **runtime_environment(),
        })
    except Exception as exc:
        send({"ready": False, "error": f"{type(exc).__name__}: {exc}"})
        return 1

    buffer = b""
    while True:
        chunk = requests.read(65536)
        if not chunk:
            return 0  # the core closed the pipe
        buffer += chunk
        while b"\n" in buffer:
            line, buffer = buffer.split(b"\n", 1)
            if not line.strip():
                continue
            req_id = None
            try:
                req = json.loads(line.decode("utf-8"))
                req_id = req.get("id")
                method = req.get("method")
                if method == "shutdown":
                    send({"id": req_id, "ok": True, "result": {}})
                    return 0
                if method == "evaluate":
                    code = req.get("params", {}).get("code")
                    if not isinstance(code, str):
                        raise ValueError("evaluate needs params.code (a string)")
                    send({"id": req_id, "ok": True, "result": evaluate(code, ns)})
                else:
                    raise ValueError(f"unknown method: {method!r}")
            except Exception as exc:
                send({"id": req_id, "ok": False,
                      "error": {"type": type(exc).__name__, "message": str(exc)}})


if __name__ == "__main__":
    sys.exit(main(sys.argv))
