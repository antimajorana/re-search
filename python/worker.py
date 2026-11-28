"""Compute and check worker for re:Search (protocol v2, backwards-compatible with v1).

Started by the Rust `worker` crate as:

    python worker.py <request-handle> <response-handle> [--role solver|check]
    python worker.py --stdio [--role solver|check]

The protocol is one JSON object per line (UTF-8) on dedicated pipes (or stdio
in `--stdio` test mode, where Python `stdout` is redirected away from the
protocol stream).

Roles:
  * `solver` (default): runs `evaluate` (accepting protocol 1 `{code}` and
    protocol 2 `{code, exprs_in}`) and `shutdown`. Captures `save_expr(...)`
    into `result.exprs_out` and exposes `expr(name)` backed by `exprs_in`.
  * `check`: host-controlled second process role. Never creates a solver
    namespace, never imports `solver_calc`, and forbids `evaluate`. Exposes
    only `canonicalize`, `check`, and `shutdown` over `python/checks/`.

On start the worker sends `{"ready": true, "role": ..., "protocol": 2, ...}`
once, before any request.
"""

import ast
import contextlib
import contextvars
import hashlib
import importlib
import importlib.metadata
import io
import json
import math
import os
import random
import sys
import time
import traceback
from pathlib import Path

PROTOCOL_VERSION = 2
ROLE_SOLVER = "solver"
ROLE_CHECK = "check"
VALID_ROLES = (ROLE_SOLVER, ROLE_CHECK)

MAX_TEXT = 64_000  # characters kept of each of stdout, stderr, value
MAX_COMPARISON_TEXT = 8_000
MAX_VALIDATION_TEXT = 64_000
MAX_SEED_EVENTS = 32
_active_seeds = contextvars.ContextVar("active_seeds", default=None)
_cached_env = {}
_MATPLOTLIB_TOP_MODULES = ("matplotlib", "pylab")


def configure_matplotlib_env(scratch_dir=None):
    """Configure headless Agg backend and scratch MPLCONFIGDIR lazily."""
    os.environ["MPLBACKEND"] = "Agg"
    if scratch_dir is not None:
        cfg_dir = Path(scratch_dir) / "mplconfig"
    else:
        existing = os.environ.get("MPLCONFIGDIR", "").strip()
        cfg_dir = Path(existing) if existing else (Path.cwd() / "mplconfig")
    os.environ["MPLCONFIGDIR"] = str(cfg_dir)
    cfg_dir.mkdir(parents=True, exist_ok=True)
    return cfg_dir


class _LazyMatplotlibEnvFinder:
    """One-shot meta_path hook configuring Matplotlib only on first import."""

    def __init__(self, scratch_dir=None):
        self.scratch_dir = scratch_dir
        self.triggered = False

    def find_spec(self, fullname, path=None, target=None):
        if not self.triggered:
            top = fullname.partition(".")[0]
            if top in _MATPLOTLIB_TOP_MODULES:
                self.triggered = True
                configure_matplotlib_env(self.scratch_dir)
        return None


def _ensure_script_dir_on_path():
    script_dir = os.path.dirname(os.path.abspath(__file__))
    if script_dir and script_dir not in sys.path:
        sys.path.insert(0, script_dir)


def parse_cli_args(argv):
    """Parse CLI arguments into `(mode, req_handle, resp_handle, role)`."""
    args = list(argv[1:])
    role = ROLE_SOLVER
    positional = []
    stdio = False
    idx = 0
    while idx < len(args):
        token = args[idx]
        if token == "--stdio":
            stdio = True
            idx += 1
        elif token == "--role":
            if idx + 1 >= len(args):
                raise SystemExit("--role requires an argument (solver or check)")
            role = args[idx + 1]
            idx += 2
        elif token.startswith("--role="):
            role = token.split("=", 1)[1]
            idx += 1
        else:
            positional.append(token)
            idx += 1
    if role not in VALID_ROLES:
        raise SystemExit(f"invalid --role {role!r}; expected one of {VALID_ROLES}")
    if stdio:
        if positional:
            raise SystemExit("--stdio does not accept pipe handle arguments")
        return "stdio", None, None, role
    if len(positional) != 2:
        raise SystemExit("usage: worker.py <request-handle> <response-handle> [--role solver|check]")
    return "pipes", int(positional[0]), int(positional[1]), role


def open_pipes(argv):
    """Open protocol streams and return `(requests, responses, role)`."""
    mode, req_h, resp_h, role = parse_cli_args(argv)
    if mode == "stdio":
        req_stream = sys.stdin.buffer
        resp_stream = sys.stdout.buffer
        # Keep stray prints off the JSON-lines protocol channel.
        sys.stdout = sys.stderr
        return req_stream, resp_stream, role
    import msvcrt

    fd_in = msvcrt.open_osfhandle(req_h, os.O_RDONLY | os.O_BINARY)
    fd_out = msvcrt.open_osfhandle(resp_h, os.O_BINARY)
    return os.fdopen(fd_in, "rb"), os.fdopen(fd_out, "wb", buffering=0), role


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


def runtime_environment(role=ROLE_SOLVER):
    """Return version and source hashes from the active Python runtime."""
    _ensure_script_dir_on_path()
    import sympy

    info = {
        "protocol": PROTOCOL_VERSION,
        "role": role,
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
    try:
        import checks

        info["checks_sha256"] = checks.registry_sha256()
    except Exception:
        pass
    return info


def _compact_env(role=ROLE_SOLVER):
    if role not in _cached_env:
        _cached_env[role] = runtime_environment(role=role)
    return dict(_cached_env[role])


def make_namespace():
    ns = {"__name__": "__worker__"}
    exec("import sympy\nfrom sympy import *", ns)
    _ensure_script_dir_on_path()
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


def _validate_srepr_map(mapping, field_name):
    if not isinstance(mapping, dict):
        raise ValueError(f"{field_name} must be a JSON object mapping names to srepr strings")
    validated = {}
    for k, v in mapping.items():
        if not isinstance(k, str) or not k:
            raise ValueError(f"{field_name} keys must be non-empty strings")
        if not isinstance(v, str):
            raise ValueError(f"{field_name}[{k!r}] must be a string")
        validated[k] = v
    return validated


def evaluate(code, ns, exprs_in=None, env_info=None, scratch_dir=None):
    """Run `code`; if its last statement is an expression, return its value."""
    staged = None if exprs_in is None else _validate_srepr_map(exprs_in, "exprs_in")
    started = time.perf_counter()
    code_sha256 = hashlib.sha256(code.encode("utf-8")).hexdigest()
    out, err = io.StringIO(), io.StringIO()
    value = None
    has_value = False
    error = None
    result = {"seed_events": []}
    outbox = {}

    solver_calc_mod = ns.get("solver_calc")
    expr_ctx = (
        solver_calc_mod.expr_context(staged)
        if solver_calc_mod is not None and hasattr(solver_calc_mod, "expr_context")
        else contextlib.nullcontext(outbox)
    )

    lazy_mpl = _LazyMatplotlibEnvFinder(scratch_dir=scratch_dir)
    sys.meta_path.insert(0, lazy_mpl)
    token = _active_seeds.set(result)
    try:
        with expr_ctx as active_outbox, contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            outbox = active_outbox
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
        try:
            sys.meta_path.remove(lazy_mpl)
        except ValueError:
            pass

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
    # Additive protocol 2 fields (ignored by protocol 1 readers)
    result["exprs_out"] = dict(outbox)
    result["code_sha256"] = code_sha256
    result["wall_ms"] = int((time.perf_counter() - started) * 1000)
    result["process"] = ROLE_SOLVER
    result["env"] = dict(env_info) if env_info is not None else _compact_env(ROLE_SOLVER)
    return result


def handle_canonicalize(params):
    """Handle `canonicalize` in check role: parse via `srepr_safe`, render canonical form & LaTeX."""
    import checks

    started = time.perf_counter()
    if "srepr" in params:
        raw = params["srepr"]
        if not isinstance(raw, str):
            raise ValueError("canonicalize needs params.srepr (a string)")
        body = checks.canonicalize_expression(raw)
        body["process"] = ROLE_CHECK
        body["wall_ms"] = int((time.perf_counter() - started) * 1000)
        body["registry_sha256"] = checks.registry_sha256()
        return body
    if "exprs" in params:
        mapping = _validate_srepr_map(params["exprs"], "exprs")
        rendered = {name: checks.canonicalize_expression(raw) for name, raw in mapping.items()}
        return {
            "exprs": rendered,
            "process": ROLE_CHECK,
            "wall_ms": int((time.perf_counter() - started) * 1000),
            "registry_sha256": checks.registry_sha256(),
        }
    raise ValueError("canonicalize needs params.srepr (string) or params.exprs (dict)")


def handle_check(params):
    """Handle `check` in check role: parse handles via `srepr_safe` and run registered check."""
    import checks

    started = time.perf_counter()
    name = params.get("name")
    if not isinstance(name, str) or not name:
        raise ValueError("check needs params.name (a non-empty string)")
    handles = _validate_srepr_map(params.get("handles"), "handles")
    check_params = params.get("params")
    if check_params is None:
        check_params = {}
    elif not isinstance(check_params, dict):
        raise ValueError("check params.params must be a JSON object")
    outcome = checks.run_registered_check(name, handles, check_params)
    return {
        "name": name,
        "registry_sha256": checks.registry_sha256(),
        "process": ROLE_CHECK,
        "wall_ms": int((time.perf_counter() - started) * 1000),
        **outcome,
    }


def serve_streams(requests, responses, role=ROLE_SOLVER):
    """Serve JSON-lines protocol 2 requests on `(requests, responses)` for `role`."""
    if role not in VALID_ROLES:
        raise ValueError(f"invalid role {role!r}")

    def send(obj):
        responses.write((json.dumps(obj, ensure_ascii=True) + "\n").encode("utf-8"))
        responses.flush()

    ns = None
    try:
        _ensure_script_dir_on_path()
        if role == ROLE_SOLVER:
            ns = make_namespace()
        else:
            # Check role intentionally imports only host-shipped checks, never solver_calc.
            import checks  # noqa: F401
        env = runtime_environment(role=role)
        _cached_env[role] = env
        send({"ready": True, "pid": os.getpid(), **env})
    except Exception as exc:
        send({"ready": False, "role": role, "error": f"{type(exc).__name__}: {exc}"})
        return 1

    while True:
        line = requests.readline()
        if not line:
            return 0
        if not line.strip():
            continue
        req_id = None
        try:
            req = json.loads(line.decode("utf-8"))
            req_id = req.get("id")
            method = req.get("method")
            params = req.get("params")
            if params is None:
                params = {}
            elif not isinstance(params, dict):
                raise ValueError("params must be a JSON object")

            if method == "shutdown":
                send({"id": req_id, "ok": True, "result": {}})
                return 0
            if role == ROLE_SOLVER:
                if method == "evaluate":
                    code = params.get("code")
                    if not isinstance(code, str):
                        raise ValueError("evaluate needs params.code (a string)")
                    exprs_in = params.get("exprs_in")
                    send({
                        "id": req_id,
                        "ok": True,
                        "result": evaluate(code, ns, exprs_in=exprs_in, env_info=env),
                    })
                elif method in ("canonicalize", "check"):
                    raise PermissionError(f"{method} is only allowed in check role")
                else:
                    raise ValueError(f"unknown method: {method!r}")
            else:
                if method == "evaluate":
                    raise PermissionError("evaluate is forbidden in check role")
                if method == "canonicalize":
                    send({"id": req_id, "ok": True, "result": handle_canonicalize(params)})
                elif method == "check":
                    send({"id": req_id, "ok": True, "result": handle_check(params)})
                else:
                    raise ValueError(f"unknown method: {method!r}")
        except Exception as exc:
            err_obj = {"type": type(exc).__name__, "message": str(exc)}
            rejected = getattr(exc, "rejected_name", None)
            if rejected is not None:
                err_obj["rejected_constructor"] = rejected
            send({"id": req_id, "ok": False, "error": err_obj})


def main(argv):
    requests, responses, role = open_pipes(argv)
    return serve_streams(requests, responses, role=role)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
