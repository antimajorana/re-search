#!/usr/bin/env python3
"""Replay successful computation checks using the live worker's evaluator."""

import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent


def replay_worker(steps):
    from _re_search_runtime.worker import evaluate, make_namespace

    namespace = make_namespace()
    for step in steps:
        print(f"[run_all_checks] running {step}...", flush=True)
        result = evaluate((ROOT / step).read_text(encoding="utf-8"), namespace)
        sys.stdout.write(result["stdout"])
        sys.stderr.write(result["stderr"])
        if "error" in result:
            print(f"[run_all_checks] FAILED: {step}", file=sys.stderr)
            print(result["error"]["traceback"], file=sys.stderr)
            return 1
    return 0


def main():
    manifest = ROOT / "_re_search_runtime" / "replay_steps.json"
    workers = json.loads(manifest.read_text(encoding="utf-8"))
    if len(sys.argv) == 3 and sys.argv[1] == "--worker":
        return replay_worker(workers[int(sys.argv[2])])

    for index in range(len(workers)):
        # A fresh process also resets globals in imported modules after a worker restarts.
        result = subprocess.run(
            [sys.executable, str(ROOT / "run_all_checks.py"), "--worker", str(index)],
            cwd=ROOT,
            check=False,
        )
        if result.returncode != 0:
            return 1
    print(f"[run_all_checks] passed {sum(map(len, workers))} check(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
