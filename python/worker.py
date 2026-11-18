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
import io
import json
import os
import sys
import traceback

PROTOCOL_VERSION = 1
MAX_TEXT = 64_000  # characters kept of each of stdout, stderr, value


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


def make_namespace():
    ns = {"__name__": "__worker__"}
    exec("import sympy\nfrom sympy import *", ns)
    return ns


def evaluate(code, ns):
    """Run `code`; if its last statement is an expression, return its value."""
    out, err = io.StringIO(), io.StringIO()
    value = None
    has_value = False
    error = None
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
    result = {}
    truncated = False
    for key, text in (("stdout", out.getvalue()), ("stderr", err.getvalue())):
        result[key], cut = clip(text)
        truncated = truncated or cut
    if has_value:
        result["value"], cut = clip(repr(value))
        truncated = truncated or cut
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
        import sympy

        send({
            "ready": True,
            "protocol": PROTOCOL_VERSION,
            "pid": os.getpid(),
            "python": sys.version.split()[0],
            "sympy": sympy.__version__,
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
