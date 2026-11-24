#!/usr/bin/env python3
"""Replay successful computation checks using the live worker's evaluator."""

import argparse
import hashlib
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent


def archived_bytes(entry):
    path = (ROOT / entry["path"]).resolve()
    path.relative_to(ROOT)
    payload = path.read_bytes()
    expected = entry.get("sha256")
    if expected and hashlib.sha256(payload).hexdigest() != expected:
        raise ValueError(f"hash mismatch for {entry['path']}")
    return payload


def compare_result(step, actual):
    recorded = step.get("recorded_result") or {}
    if recorded.get("seed_events") is not None:
        if any(recorded.get(key) != actual.get(key) for key in ("seed_events", "seeds_truncated")):
            print(f"[run_all_checks] WARN: recorded seed applications differ for {step['path']}")
    if recorded.get("comparison") is not None:
        return "computed value", actual.get("comparison") == recorded["comparison"]
    if (
        recorded.get("value") is None
        and recorded.get("stdout")
        and recorded.get("truncated") is False
    ):
        return "stdout", actual.get("stdout") == recorded["stdout"] and not actual["truncated"]
    return None, True


def replay_worker(steps):
    # Embedded Windows Python can omit the script directory from sys.path.
    sys.path.insert(0, str(ROOT))
    from _re_search_runtime import worker

    namespace = worker.make_namespace()
    if any(step.get("worker_environment") for step in steps):
        current = worker.runtime_environment()
        differences = {
            field
            for step in steps
            for field, recorded in (step.get("worker_environment") or {}).items()
            if current.get(field) != recorded
        }
        for field in sorted(differences):
            print(f"[run_all_checks] WARN: replay runtime differs for {field}")
    for step in steps:
        path = step["path"]
        print(f"[run_all_checks] running {path}...", flush=True)
        code = archived_bytes(step).decode("utf-8")
        result = worker.evaluate(code, namespace)
        sys.stdout.write(result["stdout"])
        sys.stderr.write(result["stderr"])
        if "error" in result:
            print(f"[run_all_checks] FAILED: {path}", file=sys.stderr)
            print(result["error"]["traceback"], file=sys.stderr)
            return 1
        compared, matches = compare_result(step, result)
        if not matches:
            print(f"[run_all_checks] FAILED: {path} {compared} differs from the recorded result", file=sys.stderr)
            return 1
        if compared is None:
            print(f"[run_all_checks] UNCOMPARED: {path} has no comparable recorded result")
        else:
            print(f"[run_all_checks] MATCH: {path} ({compared})")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify-only", action="store_true", help="check bundle hashes without executing calculations")
    parser.add_argument("--worker", type=int, help=argparse.SUPPRESS)
    args = parser.parse_args()
    manifest_path = ROOT / "replay_manifest.json"
    if manifest_path.is_file():
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        if manifest["schema_version"] != 1:
            raise ValueError("unsupported replay manifest version")
        workers = [group["steps"] for group in manifest["workers"]]
        if args.worker is None:
            hashed_files = manifest["runtime_files"] + manifest.get("document_files", []) + [
                source for source in manifest["source_files"] if source.get("sha256")
            ] + [step for steps in workers for step in steps]
            for entry in hashed_files:
                archived_bytes(entry)
            for source in manifest["source_files"]:
                if source["status"] in ("hash_mismatch", "unavailable"):
                    print(f"[run_all_checks] omitted source: {source['path']} ({source['status']})")
            if args.verify_only:
                print(f"[run_all_checks] verified {len(hashed_files)} archived file hash(es)")
                return 0
    else:
        if args.verify_only:
            raise ValueError("this legacy bundle has no file hash manifest")
        legacy = ROOT / "_re_search_runtime" / "replay_steps.json"
        workers = [
            [{"path": name} for name in group]
            for group in json.loads(legacy.read_text(encoding="utf-8"))
        ]
    if args.worker is not None:
        return replay_worker(workers[args.worker])

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
    try:
        raise SystemExit(main())
    except (IndexError, KeyError, OSError, TypeError, ValueError) as error:
        print(f"[run_all_checks] FAILED: {error}", file=sys.stderr)
        raise SystemExit(1)
