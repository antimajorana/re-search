# Report: ux-rust (wave 1)

- **Brief:** `docs/design/investigation-2026-11/agents/ux-rust.md`
- **Design:** `docs/design/investigation-2026-11/02-investigation-ux.md` (P0, P1a, P1b, P2, P3)
- **Branch:** `work/ux-rust`
- **Base commit:** `d68e904c44665752072ff332ba06181366e59e10`
- **Environment:** Linux (Ubuntu 24.04), `rustc 1.100.0`, `cargo 1.100.0`, `node v22.23.5`

## 1. Summary of implementation (package by package)

### P0 — Mechanical split (`2dd3b5b`)
Pure move with no behaviour change:
- Extracted `view` and `branch_history` into `crates/desktop-server/src/investigation_view.rs` (152 lines) and `finish` into `crates/desktop-server/src/investigation_finish.rs` (89 lines); re-exported `view` and `finish` from `crates/desktop-server/src/investigation.rs`.
- Made `marker`, `latest`, `list`, `context_text`, `include_paper_conventions`, and `current_map` `pub(crate)` in `crates/desktop-server/src/investigation.rs`.
- Added empty stubs `investigation_question.rs`, `investigation_system_prompt.rs`, and `investigation_title.rs` and registered all five new modules in `crates/desktop-server/src/lib.rs`.
- Moved `paper_map_instructions(include_conventions: bool) -> String` into `crates/research-tools/src/investigation_prompts.rs` (both values returning the original text in P0; caller updated to pass `true`).
- Moved `VerifiedFile`, `SelectedPages`, and `SourceIndex` into `crates/research-tools/src/investigation_sources.rs` (257 lines) so `crates/research-tools/src/investigation.rs` dropped from 771 lines to 521 lines (<= 600 lines).
- Moved the existing `#[cfg(test)] mod tests` block to the end of `crates/desktop-server/src/investigation.rs` (579 lines after P0, down from 810).

### P1a — Recorder gating and conventions-off prompt (`eaf7511`)
- Updated `crates/research-tools/src/investigation_prompts.rs` so `paper_map_instructions(false)` omits the conventions catalog paragraph (including `record_paper_conventions` and "comprehensive source-linked list"), while `paper_map_instructions(true)` equals the exact original text.
- Added `"investigation.question_requested" | "investigation.answer"` to the recorder gating marker list in `crates/research-tools/src/investigation.rs` (`record_document`).
- Added `map_instructions_omit_conventions_unless_requested`, `question_turn_closes_record_window`, and `answer_event_closes_stale_mapping_request` to `crates/research-tools/tests/investigation.rs`.

### P1b — Question-first routing, question turns, conventions off, view fields (`7c4c6da`)
- Implemented `Scope`, `scope(events, req)`, `map_context(events)`, `question_context(...)`, `prepare_question(...)`, `finish_question(...)`, and `conventions_resume_prompt(...)` in `crates/desktop-server/src/investigation_question.rs` (183 lines).
- Updated `crates/desktop-server/src/investigation.rs` (573 lines, strictly under P0's 579):
  - Added `"_paper_question_request"` to the stripped internal key list in `prepare`.
  - Replaced the `if/else if` routing chain with `match scope(&events, req)` (precedence: `Conventions` > `Branch` > `Action` > `Map` > `Question`) and removed the legacy "explain" branch.
  - Updated `Scope::Map` to write `conventions_requested: false`, call `paper_map_instructions(false)`, and drop the old conventions-batch sentence.
  - Updated `Scope::Conventions` to start with `"Review only the paper's conventions."` when no batch exists and `"Continue only the paper's conventions review."` otherwise.
  - Wired `#[cfg(test)] #[path = "investigation_tests.rs"] mod tests;`.
- Updated `crates/desktop-server/src/investigation_finish.rs` (95 lines) to dispatch `finish_question` when `_paper_question_request` is set (recording `investigation.answer` with `answered` / `stopped` / `failed`, clipping text to 24,000 chars and error to 2,000 chars, returning `outcome` unchanged) and updated the missing-map failure copy to `"The model finished without saving the main results with their sources. Try again, or choose another model."`.
- Updated `crates/desktop-server/src/investigation_view.rs` (183 lines) to expose `mapping_requested: bool` and `answers: [{seq, question, status, text, error}]` with the same `running` vs `interrupted` rule as `activities`.
- Created `crates/desktop-server/src/investigation_tests.rs` (443 lines) preserving the existing `long_model_context_keeps_source_locations_and_reports_omissions` test verbatim and adding all 11 P1b tests named in the design.
- Extended `crates/desktop-server/tests/investigation_flow.rs` (367 lines) with `question_turn_session_view_and_handoff_research_after_mapping` without changing any existing assertion.

### P2 — Investigation system prompt (`085c550`)
- Implemented `INVESTIGATION_SYSTEM_PROMPT` and `base_system_prompt(events)` in `crates/desktop-server/src/investigation_system_prompt.rs` (83 lines), with paper-as-data framing, `[Lstart–end]` line citations, Rule 3 honesty wording, no Proof DAG / `final_answer` duty or `[e-N]` citations, `\(...\)` / `$$...$$` math delimiters, and worker passage-locator guidance.
- Updated `crates/desktop-server/src/turn_runner.rs` (551 lines) to choose the base prompt for new sessions via `base_system_prompt(&existing)` while keeping subagent wrapping and resumed sessions untouched.
- Added `investigation_prompt_has_line_citations_and_no_proof_dag_duty` and `base_prompt_picks_investigation_only_for_started_marker` in `investigation_system_prompt.rs`, and `resume_uses_stored_custom_system_prompt_without_drift` in `crates/agent/tests/resume_loop.rs` (587 lines).

### P3 — Title fallback (`e80dc96`)
- Implemented `reading_title(events)` and `paper_title(events)` plus `title_order_reading_then_model_then_paper_then_meta`, `title_before_upload_is_new_investigation`, and `research_session_title_unchanged` in `crates/desktop-server/src/investigation_title.rs` (227 lines).
- Updated `session_title` in `crates/desktop-server/src/routes_session.rs` (624 lines, down from 629; net -5 lines) to try `reading_title`, `model_session_title`, `paper_title`, then meta title without rewriting meta.

## 2. Deviations from the design and why

1. **Additional P0 split in `research-tools`:** Moving `paper_map_instructions` alone left `crates/research-tools/src/investigation.rs` at ~754 lines. As instructed by `agents/ux-rust.md`, `VerifiedFile`, `SelectedPages`, and `SourceIndex` were moved into a new pure helper module `crates/research-tools/src/investigation_sources.rs` (257 lines), bringing `crates/research-tools/src/investigation.rs` to 521 lines after P0 (523 lines after P1a).
2. **`conventions_resume_prompt` placement:** Placed in `investigation_question.rs` rather than `investigation.rs` so `investigation.rs` (573 lines) stays strictly below its P0 size (579 lines) per P1b acceptance.
3. **`clip_title` helper in `investigation_title.rs`:** Kept a local `clip_title` helper inside `investigation_title.rs` instead of making `routes_session::clip_title` `pub(crate)`, so `routes_session.rs` was edited strictly inside `session_title` only.
4. **Defensive `branch_history` guard:** Added `"investigation.question_requested"` to the newer-request marker list in `investigation_view::branch_history` so an earlier interrupted branch cannot falsely display `"running"` during a later question turn.
5. **`tests/investigation_flow.rs` handoff integration test (design line 275):** The design asks to verify "a session whose only answer is a question turn still opens a handoff Research session." In the codebase, `POST /api/sessions/{id}/investigation/research` requires an `investigation.map` (`investigation.rs::selection` -> `current_map`), and `export::fork::research_from_paper` (`crates/export/src/fork.rs:27-29`, outside `ux-rust` ownership) also requires a matching `investigation.map` by `map_seq` and `map_hash`. The new integration test (`question_turn_session_view_and_handoff_research_after_mapping`) tests a question-only session's view first (`answers` present, `mapping_requested == false`, `map == null`), then records a map with `conventions_requested: false` (no conventions catalog), and verifies `POST /investigation/research` opens a handoff Research session.
6. **Pre-existing >600-line files touched within brief scope:** `crates/desktop-server/src/routes_session.rs` was 629 lines at base and is now 624 lines (-5 net lines). `crates/research-tools/tests/investigation.rs` was 671 lines at base and is now 777 lines (+106 lines of additions explicitly requested by P1a / `ux-rust.md`). Every other touched or new file is <= 587 lines.

## 3. Decisions taken unattended

- **Path formatting in `question_context`:** Passed `primary_path: &str` (`as_str().unwrap_or_default()`) instead of formatting `serde_json::Value` directly so the prompt contains `src/paper.txt` rather than `"src/paper.txt"`.
- **Single page-range sentence on question turns:** `question_context` appends the physical PDF page sentence before `Current map:\n...`, and `prepare` skips its trailing page-range append for `Scope::Question` so the sentence is never duplicated.
- **`finish_question` payload & error handling:** On `Ok`, records `status: "answered"`, `text` (clipped to 24,000 chars), `error: null`. On `Err`, records `status: "stopped"` (if a `session.abort` with `stopped: true` follows `request_seq`) or `"failed"`, `text: null`, `error` (clipped to 2,000 chars). Any ledger open/append failure is ignored so `finish_question` strictly returns `outcome` unchanged.
- **`paper_title` filename fallback:** When `source.ingested.payload["title"]` is null/empty, falls back to the last `/`-separated segment of `primary_path` (or the first entry of `files`), clipped to 80 chars, for both pinned and single-unpinned ingested sources; with 0 or 2+ unpinned sources, returns `"New investigation"`.
- **P2 resume test file:** Added `resume_uses_stored_custom_system_prompt_without_drift` to `crates/agent/tests/resume_loop.rs` (548 -> 587 lines, staying under 600) rather than `crates/agent/tests/autonomous_and_streaming.rs` (already 593 lines at base).
- **`research-tools` module wiring:** Used `#[path = "investigation_prompts.rs"]` and `#[path = "investigation_sources.rs"]` inside `crates/research-tools/src/investigation.rs`, matching existing `investigation_{branch,conventions,summary}.rs`; `crates/research-tools/src/lib.rs` needed no changes.
- **No new crate dependencies** added; `policy` and `ledger` remain untouched.

## 4. Verification and test results

| Command | Result | Details |
|---|---|---|
| `cargo test --workspace --no-fail-fast` (without `RE_SEARCH_PYTHON`) | **PASS** | **77 suites, 449 passed, 0 failed, 0 ignored** (including `desktop-server` lib: 46 passed; `investigation_flow`: 2 passed; `research-tools` `investigation`: 9 passed; `agent` `resume_loop`: 6 passed) |
| `cargo clippy -j 1 --workspace --all-targets` | **0 new warnings** | **4 pre-existing warnings** remain outside `ux-rust` files (down from 5 at base; fixed pre-existing `items_after_test_module` in `desktop-server/src/investigation.rs`) |
| `cargo clippy --workspace --all-targets -- -D warnings` | **FAIL (pre-existing)** | Fails identically on base `d68e904c` on `crates/desktop-server/src/evidence.rs:737` (see below) |
| `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` | **PASS** | All JS assets syntax-checked cleanly (no asset files modified) |
| `node --test crates/desktop-server/tests/*.test.mjs` | **PASS** | **63 passed, 0 failed, 0 skipped** |

## 5. Pre-existing failures/warnings on Linux (proven on base `d68e904c`)

1. **`cargo clippy --workspace --all-targets -- -D warnings` fails on Rust 1.100.0 Linux** on base commit `d68e904c44665752072ff332ba06181366e59e10`:
   - `crates/desktop-server/src/evidence.rs:737:40`: `clippy::manual_unwrap_or` (false positive in Rust 1.100.0 on a guarded `Some(status @ ("verified" | ...)) => status` match; applying clippy's suggestion would break the status allowlist).
   - `crates/agent/src/guardrails.rs:335:1`: `clippy::items_after_test_module`.
   - `crates/research-tools/tests/sandbox_and_fs.rs:6:5`: `unused_imports` (`TEX_AUDIT`, `is_symlink_or_junction`) on non-Windows.
   - `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6:15`: `unused_imports` (`BufRead`, `BufReader`) on non-Windows.
   - (Base also had `crates/desktop-server/src/investigation.rs:384:1` `clippy::items_after_test_module`, which our P0/P1b test move eliminated.)
2. **`RE_SEARCH_PYTHON` set globally on Linux hangs `crates/cli/tests/desktop_and_ext.rs`:**
   - Proven on base `d68e904c`: running `RE_SEARCH_PYTHON=$(command -v python3) cargo test --workspace --no-fail-fast` hangs in `cli_research_workspace_read_fork_diff_export_and_verify` because the spawned `re-search run --route api` child inherits `RE_SEARCH_PYTHON`, rejects non-Windows Python worker configuration, and exits before contacting `ScriptedMock`, leaving `mock.finish()` blocked in `TcpListener::accept`.
   - Running `cargo test --workspace --no-fail-fast` without `RE_SEARCH_PYTHON` completes cleanly with 449 passed, 0 failed (all Linux-compiled tests pass without `RE_SEARCH_PYTHON`; the Python worker tests in `crates/worker/tests/python_worker.rs` are `cfg(windows)`).

## 6. What could not be compiled or run on Linux

- All `#[cfg(windows)]` code and tests: `crates/worker` (`AppContainerSandbox`, `JobObjectSandbox`, `appcontainer_sandbox`, `python_worker`), `crates/cli` (`contained_router`, `init_abort`), `crates/desktop-server` (`claude_cli_route`, `chatgpt_route`, Windows worker/process-tree paths), `crates/desktop-app` (WebView2 window), and `crates/chatgpt-auth` (DPAPI vault).
- Playwright browser suite (`crates/desktop-server/tests/investigation_browser.py`), which requires Windows bundled Chromium and wave-2 UI packages (P4/P5).
- Live network/provider/CLI model turns.

## 7. Bugs or issues noticed but not fixed

- `crates/desktop-server/src/evidence.rs:737`: triggers a Rust 1.100.0 `clippy::manual_unwrap_or` false positive (cannot be "fixed" with `unwrap_or` because the match filters by an allowlist of statuses; would need `#[allow(clippy::manual_unwrap_or)]` or a `filter` rewrite).
- `crates/cli/tests/desktop_and_ext.rs:149-168`: calls `mock.finish()` before checking `out.status`, so any early CLI exit (such as `RE_SEARCH_PYTHON` set on Linux) hangs the test indefinitely.
- `crates/export/src/fork.rs:27-29` & `crates/desktop-server/src/investigation.rs:438`: `POST /investigation/research` requires an `investigation.map`; a question-only Investigation cannot open a handoff Research session until a map exists (contrast with `02-investigation-ux.md:275`).
- `crates/agent/src/guardrails.rs:335`, `crates/research-tools/tests/sandbox_and_fs.rs:6`, `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6`: pre-existing clippy / non-Windows `unused_imports` warnings.
