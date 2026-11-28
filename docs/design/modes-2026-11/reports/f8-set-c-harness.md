# Report: `f8-set-c-harness` — F8 `eval_harness.rs` `Set::C` Prerequisite Fix

- **Base commit (`origin/gemini/work/fixed`):** `21d78929fa836c1cf1385330aff901d0d432c55f`
- **Working branch:** `work/f8-set-c-harness`
- **Target push branch:** `gemini/modes/f8-set-c-harness`
- **Code commit:** `7fbdf3b764ea527a7967f4f2cdefe758ae99ef44` `fix(eval): retain non-Set-C manifests before synthetic Set::C check in with_chains`

## 1. What Was Implemented

| File | Lines | Change |
|---|---:|---|
| `crates/agent/tests/eval_harness.rs` | 494 | Added a single line `all.retain(\|m\| m.set != Set::C);` at line 190 inside `manifests_are_checked_together`'s `with_chains` closure right after `let mut all = fixed.manifests.clone();` and before pushing the synthetic `Set::C` fixed manifest. |

- `fixed_tier_manifests_load_and_validate` (lines 39–100) and the `Set::P` duplicate-manifest test (lines 199–202, formerly 198–201) remain completely intact.
- No test assertion was weakened, deleted, or ignored.
- `Cargo.lock`, `crates/research-tools/tests/golden/`, and all other files are untouched.
- Every touched file is LF-only and strictly `<= 600` lines.

## 2. Deviations, Decisions & Assumptions

- **Deviations from design / brief:** None.
- **Decisions taken alone:** None beyond the exact single-line fix specified to resolve the §6 blocker documented in `docs/design/modes-2026-11/reports/exp-x14-eval.md`.
- **Assumptions:** Windows-only `cfg(windows)` code and tests cannot compile or run on Linux; this test-only change introduces no platform-specific paths or shell-outs.

## 3. Merge Notes

- **Edited existing file:** `crates/agent/tests/eval_harness.rs` (`manifests_are_checked_together` -> `with_chains`, line 190).
- **Downstream effect:** Unblocks `exp-x14-eval` Checkpoint 2 from adding `tests/fixtures/eval/manifest-c.json` on disk without colliding with `with_chains`'s synthetic `Set::C` manifest (`eval_support/manifest.rs:317-319` `"more than one fixed manifest for set C"`). Duplicate-manifest enforcement remains covered by the untouched `Set::P` check in the same test.

## 4. Non-Root Linux Verification Commands & Measured Outcomes

Environment: Linux x86_64 non-root (`uid=1001(antigravity)`), `RE_SEARCH_PYTHON` unset.

| Command | Measured Result |
|---|---|
| `id` | `uid=1001(antigravity) gid=1001(antigravity) groups=1001(antigravity),27(sudo),990(docker)` (non-root) |
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | **PASS** (exit `0` — `No touched source file is over 600 lines.`) |
| `git diff --stat origin/gemini/work/fixed -- Cargo.lock crates/research-tools/tests/golden/` | **PASS** (empty diff) |
| `unset RE_SEARCH_PYTHON && cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` | **PASS** (exit `0`, `0` warnings) |
| `unset RE_SEARCH_PYTHON && cargo test --workspace --locked --no-fail-fast` | **PASS** (exit `0` — `1410 passed, 0 failed, 2 ignored` across `183` test/doc-test targets) |
| `unset RE_SEARCH_PYTHON && cargo test --locked -p agent --test eval_harness` | **PASS** (`16 passed, 0 failed, 2 ignored`) |
| `unset RE_SEARCH_PYTHON && cargo test --locked -p research-tools --test tool_def_golden` | **PASS** (`3 passed, 0 failed, 0 ignored`) |

## 5. Pre-Existing Notes, Not Run on Linux & Other Observations

- **Pre-existing failures:** None.
- **Pre-existing ignored tests:** 2 `#[ignore = "live tier: ..."]` tests in `crates/agent/tests/eval_harness.rs` (`live_manifests_validate` and `file_sheet_from_json`), untouched from `origin/gemini/work/fixed`.
- **Pre-existing clippy lint:** Known base-commit `manual_unwrap_or` in `crates/desktop-server/src/evidence.rs:744` (>600 lines, untouched, suppressed via `-A clippy::manual_unwrap_or` per `RULES.md`).
- **Not compiled or run on Linux:** Windows-only `cfg(windows)` code and integration tests (`crates/worker/tests/{appcontainer_sandbox,job_memory_limit,python_worker}.rs` report `0 passed` on Linux; `re-search-app` WebView2 shell).
- **Bugs noticed / design contradictions:** None.
