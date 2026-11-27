#!/usr/bin/env python3
"""Replay successful computation steps and Solve host checks from an exported bundle."""

import argparse
import hashlib
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent


def canonical_json(obj):
    return json.dumps(obj, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False)


def archived_bytes(entry):
    path = (ROOT / entry["path"]).resolve()
    path.relative_to(ROOT)
    payload = path.read_bytes()
    expected = entry.get("sha256")
    if expected and hashlib.sha256(payload).hexdigest() != expected:
        raise ValueError(f"hash mismatch for {entry['path']}")
    return payload


def verify_solve_check_handles(solve_checks):
    for chk in solve_checks:
        handles = chk.get("handles") or {}
        expected = chk.get("handles_sha256") or {}
        for side, digest in expected.items():
            raw = handles.get(side)
            if raw is None or hashlib.sha256(raw.encode("utf-8")).hexdigest() != digest:
                raise ValueError(
                    f"hash mismatch for solve check {chk.get('check_id', '<unknown>')} handle {side}"
                )


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
        exprs_in = step.get("exprs_in")
        if exprs_in is not None:
            result = worker.evaluate(code, namespace, exprs_in=exprs_in)
        else:
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


def replay_solve_checks(solve_checks):
    if not solve_checks:
        return 0
    sys.path.insert(0, str(ROOT / "_re_search_runtime"))
    import checks

    verify_solve_check_handles(solve_checks)
    live_reg = checks.registry_sha256()
    warned_reg = False
    passed = 0

    for chk in solve_checks:
        cid = chk.get("check_id", "<unknown>")
        name = chk["name"]
        rec_outcome = chk.get("outcome")
        if rec_outcome == "not_run":
            print(f"[run_all_checks] SKIP: solve_check {cid} ({name}) was not_run")
            continue
        handles = chk.get("handles") or {}
        rec_ev = chk.get("recorded_evidence") or {}
        if not handles or (not chk.get("registry_sha256") and "timeout_s" in rec_ev):
            print(f"[run_all_checks] SKIP: solve_check {cid} ({name}) has no replayable handles")
            continue
        rec_reg = chk.get("registry_sha256")
        if rec_reg and rec_reg != live_reg and not warned_reg:
            print("[run_all_checks] WARN: replay runtime differs for checks_registry_sha256")
            warned_reg = True

        res = checks.run_registered_check(name, handles, chk.get("params") or {})
        evidence = dict(res.get("evidence") or {})
        for k, v in (chk.get("code_hashes") or {}).items():
            evidence.setdefault(k, v)
        actual_sha = hashlib.sha256(canonical_json(evidence).encode("utf-8")).hexdigest()
        actual_outcome = res.get("outcome")
        expected_sha = chk.get("evidence_sha256")

        if actual_outcome != rec_outcome or actual_sha != expected_sha:
            print(
                f"[run_all_checks] FAILED: solve_check {cid} ({name}) result changed on re-run "
                f"(expected outcome={rec_outcome} sha={expected_sha}, got outcome={actual_outcome} sha={actual_sha})",
                file=sys.stderr,
            )
            return 1
        print(f"[run_all_checks] MATCH: solve_check {cid} ({name}, evidence_sha256={actual_sha[:12]})")
        passed += 1

    print(f"[run_all_checks] replayed {passed} solve check(s)")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify-only", action="store_true", help="check bundle hashes without executing calculations")
    parser.add_argument("--worker", type=int, help=argparse.SUPPRESS)
    parser.add_argument("--solve-checks", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    manifest_path = ROOT / "replay_manifest.json"
    solve_checks = []
    if manifest_path.is_file():
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        if manifest["schema_version"] != 1:
            raise ValueError("unsupported replay manifest version")
        workers = [group["steps"] for group in manifest["workers"]]
        solve_checks = manifest.get("solve_checks") or []
        if args.worker is None and not args.solve_checks:
            hashed_files = manifest["runtime_files"] + manifest.get("document_files", []) + [
                source for source in manifest["source_files"] if source.get("sha256")
            ] + [step for steps in workers for step in steps]
            for entry in hashed_files:
                archived_bytes(entry)
            verify_solve_check_handles(solve_checks)
            for source in manifest["source_files"]:
                if source["status"] in ("hash_mismatch", "unavailable"):
                    print(f"[run_all_checks] omitted source: {source['path']} ({source['status']})")
            if args.verify_only:
                print(
                    f"[run_all_checks] verified {len(hashed_files)} archived file hash(es)"
                    + (f" and {len(solve_checks)} solve check handle set(s)" if solve_checks else "")
                )
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
    if args.solve_checks:
        return replay_solve_checks(solve_checks)

    for index in range(len(workers)):
        # A fresh process also resets globals in imported modules after a worker restarts.
        result = subprocess.run(
            [sys.executable, str(ROOT / "run_all_checks.py"), "--worker", str(index)],
            cwd=ROOT,
            check=False,
        )
        if result.returncode != 0:
            return 1
    if solve_checks:
        result = subprocess.run(
            [sys.executable, str(ROOT / "run_all_checks.py"), "--solve-checks"],
            cwd=ROOT,
            check=False,
        )
        if result.returncode != 0:
            return 1
        print(f"[run_all_checks] passed {sum(map(len, workers))} worker step(s) and {len(solve_checks)} solve check(s)")
        return 0
    print(f"[run_all_checks] passed {sum(map(len, workers))} check(s)")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (IndexError, KeyError, OSError, TypeError, ValueError) as error:
        print(f"[run_all_checks] FAILED: {error}", file=sys.stderr)
        raise SystemExit(1)
