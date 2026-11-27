# Report: solve-s19-export-replay — Solve Notebook Export & ZIP Replay (S19)

- **Brief:** `docs/design/modes-2026-11/agents/solve-s19-export-replay.md`
- **Base commit (`origin/gemini/work/fixed`):** `140e9acdc53e9fdaa0ce7b1c1bdbd2c24c59ea47` (lane started from `acf7cb0d`, cleanly merged `d78f4e1`, `eb0eb63`, and `140e9ac`)
- **Branch:** `gemini/modes/solve-s19-export-replay`
- **Packages:** `S19a`, `S19b`, `S19c`
- **Date:** 2026-11-27

---

## 1. What Was Implemented

### Checkpoint S19a — Route Extraction
| File | Change | Line Count | Purpose |
| --- | --- | ---: | --- |
| `crates/desktop-server/src/routes_tools.rs` | Shrink refactor | 585 | Extracted `handle_export` out of `routes_tools.rs` (down from 610 lines) |
| `crates/desktop-server/src/routes_export.rs` | New | 38 | Standalone `handle_export(state, session_id, query)` serving `tex`, `html`, `zip`, `notebook`/`ipynb`, and `explain_*` |
| `crates/desktop-server/src/lib.rs` | Small edit (+1 line) | 254 | Expose `pub mod routes_export;` |
| `crates/desktop-server/src/api_dispatch.rs` | Small edit | 488 | Dispatch `/api/sessions/{id}/export` via `routes_export::handle_export` |

### Checkpoint S19b — Deterministic `nbformat` 4.5 Notebook Exporter
| File | Change | Line Count | Purpose |
| --- | --- | ---: | --- |
| `crates/export/src/notebook.rs` | New | 534 | Verified ledger loader, D18 closure selection (`include_all_runs: false` default), topological ordering, deterministic 8-hex cell ID allocation with collision dedup |
| `crates/export/src/notebook_prelude.rs` | New | 424 | Self-contained Cell 2 SymPy prelude with inlined `solve.expr` table, restricted AST `srepr` parser (`_parse_srepr_safe`), `expr()`, `load_expr()`, `save_expr()`, `assumed_symbol()`, `set_random_seed()`, plus per-check standalone Python snippets |
| `crates/export/src/notebook_render.rs` | New | 507 | Cell 1 setup Markdown, per-step R5-strict LaTeX & R22-strict badge/quote Markdown cells, receipt cells with stored outputs, and final checklist + `rests_on` cell |
| `crates/export/src/notebook_tests.rs` | New | 562 | Unit tests for nbformat 4.5 structure, deterministic IDs & collision dedup, D18 closure, R5 ignoring tampered `entry.latex`, and R22 wording placement |

### Checkpoint S19c — Format Dispatch, Solve ZIP Bundle, `replay_checks.py`, HTTP Route & Integration Tests
| File | Change | Line Count | Purpose |
| --- | --- | ---: | --- |
| `crates/export/src/lib.rs` | Edit | 372 | Added `ExportFormat::Notebook` (`"ipynb" \| "notebook"`), `export_session_bytes_with_options`, and Solve dispatch bypassing `latex::load_session_export_data` |
| `crates/export/src/zip.rs` | Edit | 377 | Shared `insert_runtime_files` (including `_re_search_runtime/checks/*` + S10 `physics*.py`), `checks_sha256` runtime field, `write_zip_entries`, and `pub(crate)` manifest helpers |
| `crates/export/src/zip_solve.rs` | New | 431 | Solve ZIP builder (`notebook.ipynb`, `ledger.json`, `code/step_<seq>.py` with incremental `exprs_in`, `sources/*`, `_re_search_runtime/checks/*`, `replay_manifest.json` with `solve_checks`, `REPRODUCE.md`) |
| `crates/export/src/replay_checks.py` | Edit | 219 | Protocol-2 `exprs_in` passing to `worker.evaluate`, `solve_checks` handle SHA-256 verification under `--verify-only`, and `--solve-checks` isolated replay checking R19 `evidence_sha256` parity |
| `crates/export/tests/solve_notebook_zip.rs` | New | 556 | Standalone plain Python + SymPy execution of `.ipynb` cells, Solve ZIP replay, R19 tamper detection, R22 property sweep across tiers/unmet criteria, and Research notebook refusal |
| `crates/desktop-server/tests/solve_export_route.rs` | New | 181 | Integration tests for `GET /api/sessions/{id}/export?format=notebook\|ipynb\|zip`, `AuthGuard` 401 vs cookie/token access, `Content-Type: application/x-ipynb+json; charset=utf-8`, `include_all_runs`, and 400 on Research |

**Do-not-edit verification:** `crates/export/src/latex.rs`, `crates/export/src/writeup*.rs`, `crates/desktop-server/src/{http_srv,auth}.rs`, `python/checks/*`, `crates/research-tools/src/tool_profile.rs`, `crates/research-tools/tests/golden/{tool_defs,tool_profiles}.txt`, and `Cargo.lock` were not modified.

---

## 2. Decisions Taken Alone, Deviations, and Assumptions

1. **`_re_search_runtime/checks/` path vs `_re_search_runtime/python/checks/` brief phrasing:** `python/worker.py` and `python/solver_calc.py` already live directly under `_re_search_runtime/` inside ZIP bundles and import top-level `checks` (`import checks`, `from checks.srepr_safe import parse_srepr_safe`), matching `crates/worker/src/protocol.rs::STAGED_CHECK_FILES`. Placing the seven check modules at `_re_search_runtime/checks/{__init__,srepr_safe,algebra,algebra_probe,physics,physics_helpers,physics_matrix}.py` makes `replay_checks.py` and `solver_calc.py` import them directly without path hacks.
2. **S10 alignment after merging `140e9ac`:** Updated `zip.rs` to embed S10's three new check modules (`physics.py`, `physics_helpers.py`, `physics_matrix.py`) and updated `zip_solve.rs::build_solve_check_manifest` to include `assumptions` in `full_params` plus `target` handle staging for `limit_case` checks (`target_handle`), matching `crates/research-tools/src/solve_checks_params.rs`.
3. **Standalone `.ipynb` prelude & safety:** The Cell 2 SymPy prelude inlines only canonical `srepr` strings for portable handles in scope and reconstructs them via an allowlisted `ast.parse(..., mode="eval")` evaluator (`_parse_srepr_safe`, zero `eval`/`exec`/`sympify` on strings), using `_stage_handles` before receipt cells so staged `expr()` semantics hold without requiring `_re_search_runtime`.
4. **R22 wording guarantees in exported Markdown:** Step badges and `"own check: verified"` lines come strictly from `badge_for_step`; criteria requirements map `"verified"` to `"exact symbolic check (tier 4)"` so unmet criteria or numeric/model-only sessions never leak literal `"verified"`; `solve.done.summary_markdown` is included only when `lint_solve_summary_r22` accepts it.
5. **Paused/in-progress notebooks without `solve.done`:** When `state.done` is `None`, `select_ordered_steps` falls back to active non-superseded entries in `topological_order` and previews `evaluate_done_criteria` + `rests_on_summary` so partial Solve sessions remain inspectable.
6. **Subprocess isolation & R19 replay rules in `replay_checks.py`:** `--solve-checks` executes in its own subprocess (`sys.path` rooted at `_re_search_runtime`) separate from worker evaluation processes, attaches `lhs_code_sha256`/`rhs_code_sha256` via `setdefault`, compares canonical JSON (`ensure_ascii=False, sort_keys=True, separators=(",", ":")`) SHA-256 against `evidence_sha256`, and fails with `"result changed on re-run"` on mismatch; `not_run` and unbacked timed-out checks are skipped explicitly.
7. **Notebook test validation scope (assumption):** The Python `nbformat` library is not installed in the local test environment, so tests validate the nbformat 4.5 JSON schema structurally in Rust and execute all code cells sequentially in plain `python3` + `sympy` inside an isolated temporary directory with `PYTHONPATH` unset.
8. **R20 file splitting:** Extracted `notebook_prelude.rs` and `notebook_render.rs` from `notebook.rs`, and `zip_solve.rs` from `zip.rs`, so every touched/created source file stays at or under 585 lines.

---

## 3. Merge Notes

- **Edited existing files:**
  - `crates/desktop-server/src/routes_tools.rs` (extracted `handle_export`), `crates/desktop-server/src/lib.rs` (added `pub mod routes_export;`), `crates/desktop-server/src/api_dispatch.rs` (routed `/export` to `routes_export::handle_export`), `crates/desktop-server/src/routes_export.rs` (added `ExportFormat::Notebook` `Content-Type: application/x-ipynb+json; charset=utf-8` and `include_all_runs` query parsing).
  - `crates/export/src/lib.rs` (added `ExportFormat::Notebook`, `export_session_bytes_with_options`, Solve dispatch before `latex::load_session_export_data`), `crates/export/src/zip.rs` (added `checks/*` runtime embedding + `pub(crate)` helpers), `crates/export/src/replay_checks.py` (protocol-2 `exprs_in` + `--solve-checks`).
- **Coexistence with concurrent packages:** Merged cleanly with `F15` (`writeup_solve` in `crates/export/src/lib.rs`) and `S10` (`python/checks/physics*.py` + `solve_checks_params.rs`) at `140e9ac`. UI download/open controls for `?format=notebook` remain with the UI lane (`S13`).

---

## 4. Verification Results (Non-Root Linux + Windows File-Length Check)

Environment: `uid=1000(antimajorana)` on WSL2 Ubuntu, `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `Python 3.12.3` (`sympy 1.14.0`, `mpmath 1.3.0`, `numpy 2.5.4` in `/home/antimajorana/.venvs/bootloops/bin/python3`), `node v24.22.2`.

| Command | Result | Exact Counts |
| --- | --- | --- |
| `py -3 scripts/check_file_length.py --base origin/gemini/work/fixed` | PASS (exit 0) | `No touched source file is over 600 lines.` (max touched: `routes_tools.rs` 585, `notebook_tests.rs` 562, `solve_notebook_zip.rs` 556) |
| `python3 -m py_compile crates/export/src/replay_checks.py` | PASS (exit 0) | Clean bytecode compile |
| `python3 -m unittest discover -s python/checks -t python` | PASS (exit 0) | `7` passed, `0` failed |
| `python3 -m unittest discover -s python/tests -t python` | PASS (exit 0) | `19` passed, `0` failed |
| `unset RE_SEARCH_PYTHON && cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` | PASS (exit 0) | `0` errors, `0` warnings across all 13 crates |
| `unset RE_SEARCH_PYTHON && cargo test --workspace --locked --no-fail-fast -j 2` | PASS (exit 0) | `1223` passed, `0` failed, `2` pre-existing ignored (`agent/tests/eval_harness.rs`) across `159` test suites (includes `tool_def_golden`: `3` passed, `tool_profile_golden`: `3` passed, `solve_export_route`: `1` passed) |
| `env RE_SEARCH_PYTHON=/home/antimajorana/.venvs/bootloops/bin/python3 cargo test --locked -p export --no-fail-fast` | PASS (exit 0) | `36` passed, `0` failed, `0` ignored across `10` suites (real standalone `.ipynb` SymPy execution + Solve ZIP replay + Research ZIP replay) |

**What could not run on Linux:** Windows-only `#[cfg(windows)]` AppContainer tests in `crates/worker/tests/appcontainer_sandbox.rs` and `crates/worker/tests/python_worker.rs` compile to `0` active tests on Linux; `crates/desktop-server` was not modified in the asset directory (`.js`/`.mjs` untouched).
