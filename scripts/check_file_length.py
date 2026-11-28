#!/usr/bin/env python3
"""Fail when a change adds or touches a source file that is over the line limit.

Rule (shared package F0): source files stay at or under 600 lines. Files that were already
over the limit before the rule are left alone until a change touches them; a change that
touches one has to move code out of it first.

Usage:
    python scripts/check_file_length.py --base origin/main
    python scripts/check_file_length.py --self-test

"Touched" means added, modified or renamed between the merge base of `--base` and HEAD.
Only tracked source files are counted (see SOURCE_SUFFIXES); `vendor/` is skipped.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
import tempfile
from pathlib import Path

LIMIT = 600
SOURCE_SUFFIXES = (".rs", ".js", ".mjs", ".py", ".css", ".html")
SKIPPED_PREFIXES = ("vendor/",)


def git(repo: Path, *args: str) -> str:
    done = subprocess.run(
        ["git", "-C", str(repo), *args], check=True, capture_output=True, text=True, encoding="utf-8"
    )
    return done.stdout


def is_source(path: str) -> bool:
    return path.endswith(SOURCE_SUFFIXES) and not path.startswith(SKIPPED_PREFIXES)


def touched_files(repo: Path, base: str) -> list[str]:
    """Paths added, copied, modified or renamed since the merge base with `base`."""
    out = git(repo, "diff", "--name-only", "--diff-filter=ACMR", "-z", f"{base}...HEAD")
    return sorted(p for p in out.split("\0") if p)


def line_count(path: Path) -> int:
    data = path.read_bytes()
    if not data:
        return 0
    return data.count(b"\n") + (0 if data.endswith(b"\n") else 1)


def over_limit(repo: Path, base: str, limit: int = LIMIT) -> list[tuple[str, int]]:
    found = []
    for rel in touched_files(repo, base):
        if not is_source(rel):
            continue
        path = repo / rel
        if not path.is_file():
            continue
        lines = line_count(path)
        if lines > limit:
            found.append((rel, lines))
    return found


def self_test() -> int:
    """Build a throwaway repository and check the rule on it."""
    with tempfile.TemporaryDirectory() as tmp:
        repo = Path(tmp)

        def commit(message: str) -> None:
            git(repo, "add", "-A")
            git(repo, "-c", "user.name=t", "-c", "user.email=t@example.invalid", "commit", "-q", "-m", message)

        def write(rel: str, lines: int, last_newline: bool = True) -> None:
            path = repo / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            text = "\n".join(f"line {i}" for i in range(lines))
            path.write_bytes((text + ("\n" if last_newline and lines else "")).encode())

        git(repo, "init", "-q", "-b", "main")
        write("src/old_long.rs", 700)
        write("src/old_long_untouched.rs", 700)
        write("src/short.rs", 10)
        commit("base")
        git(repo, "checkout", "-q", "-b", "work")

        write("src/short.rs", 600)
        write("src/exact_no_newline.js", 600, last_newline=False)
        write("docs/long.md", 5000)
        write("vendor/long.rs", 5000)
        commit("within the limit")
        assert over_limit(repo, "main") == [], over_limit(repo, "main")

        write("src/new_long.py", 601)
        write("src/new_long_no_newline.css", 601, last_newline=False)
        write("src/old_long.rs", 699)
        commit("over the limit")
        assert over_limit(repo, "main") == [
            ("src/new_long.py", 601),
            ("src/new_long_no_newline.css", 601),
            ("src/old_long.rs", 699),
        ], over_limit(repo, "main")

        git(repo, "rm", "-q", "src/new_long.py", "src/new_long_no_newline.css")
        write("src/old_long.rs", 600)
        commit("moved code out")
        assert over_limit(repo, "main") == [], over_limit(repo, "main")
    print("check_file_length self-test: ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--base", help="ref the change is compared with, for example origin/main")
    parser.add_argument("--limit", type=int, default=LIMIT)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    if not args.base:
        parser.error("--base is required")
    repo = Path(git(Path.cwd(), "rev-parse", "--show-toplevel").strip())
    found = over_limit(repo, args.base, args.limit)
    if not found:
        print(f"No touched source file is over {args.limit} lines.")
        return 0
    print(f"Touched source files over {args.limit} lines:")
    for rel, lines in found:
        print(f"  {lines:5d}  {rel}")
    print("Move code out of each file listed above into a new file, then push again.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
