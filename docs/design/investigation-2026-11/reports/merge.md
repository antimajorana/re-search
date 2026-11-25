# Merge report: `gemini/work/merged`

- **Base commit (`origin/gemini/runner`):** `ea6b5e84436ac91a3f5b660b3dcec754a31a06a1`
- **Merged briefs (all 6, in `PARALLEL.md` ownership order):**
  1. `ux-rust` (`c4b8be68427a851e2eb146cf8bafe2c1d23fe86b`) -> merge `f6ed36f`
  2. `ux-ui` (`015d20932407e124d2b82ebe60018e0da5c79dcc`) -> merge `29d69b0`, integration `23ecf7d`
  3. `latex` (`40920e8865bbca1bfafde92be4ade9dfc75bcf78`) -> merge `84e4522`
  4. `split-1` (`1171360d2145188e190881a09ea1fe73d2fec75e`) -> merge `9be75a0`, integration `420910a`
  5. `split-2` (`e4bae867692191b3aeab81ef4a3e0a27e3c22423`) -> merge `749c6cb`, shim replacement `00a192f`
  6. `split-3` (`0a8c47ab6366be00fda5abe5806f96630821574a`) -> merge `f8709b2`, integration `1961485`, tidy-up `ed2b745`, race fix `a31a038`

All six branches were already present on `origin` during round 1, so the full merge completed in a single pass with verification after every brief.

---

## 1. Per-brief conflicts, resolutions, and integration commits

### 1.1 `ux-rust` (merge `f6ed36f`)
- **Conflicts:** None (clean merge onto `ea6b5e84`).
- **Verification after merge:** `cargo test --workspace --no-fail-fast -- --skip cli_research_workspace_read_fork_diff_export_and_verify` (77 suites, 448 passed, 0 failed, 1 filtered), 63/63 Node tests, 0 new clippy warnings.

### 1.2 `ux-ui` (merge `29d69b0`, integration `23ecf7d`)
- **Conflicts:**
  - `crates/desktop-server/src/lib.rs`: additive `mod investigation_*;` declarations from `ux-rust` and `ux-ui` (`mod investigation_cites;`). Kept all module declarations in alphabetical order.
- **Post-merge integration (`23ecf7d`):**
  - Passed `&DesktopState` into `investigation_question::finish_question` from `investigation_finish::finish`.
  - Called `investigation_cites::answer_cites_for_session` in `finish_question` on `Ok(reply)` and stored `"cites"` on `investigation.answer` (`[]` on `Err`).
  - Removed the temporary `#[allow(dead_code)]` attributes on `source_total_lines` and `answer_cites_for_session` in `crates/desktop-server/src/investigation_cites.rs`.
  - Initialized `"cites": []` on `"investigation.question_requested"` rows and copied `"cites"` from `"investigation.answer"` in `crates/desktop-server/src/investigation_view.rs`.

### 1.3 `latex` (merge `84e4522`)
- **Conflicts:** None (clean auto-merge in `crates/desktop-server/assets/app.js` and `crates/desktop-server/src/assets.rs`; verified both `ux-ui`'s `setBusy`/`research:approvals` edits and `latex`'s `resetMathMacros` edits and all static asset arms were preserved).
- **Verification after merge:** 99/99 Node tests, 2/2 `ui_math` integration tests, 0 new clippy warnings.

### 1.4 `split-1` (merge `9be75a0`, integration `420910a`)
- **Conflicts:**
  - `crates/desktop-server/src/investigation.rs`: main owner `ux-rust` won the `Scope` match structure in `prepare()`; re-applied `split-1`'s `PaperRoute` parameter and `prompts::{conventions_resume_instructions, branch_turn_instructions, map_base_instructions}` calls. Updated `investigation_prompts.rs::map_base_instructions` to pass `include_conventions = false` (Decision D4) and preserved `ux-rust`'s first-batch `"Review only the paper's conventions."` lead when `catalog.is_null()` inside `conventions_resume_instructions`.
  - `crates/research-tools/src/investigation.rs` & `investigation_prompts.rs`: kept `ux-rust`'s `investigation_prompts.rs` (`paper_map_instructions(include_conventions: bool)`) and updated its `MAP_INTRO`/`MAP_OUTRO` text with `split-1`'s `read_paper_lines` and `{source_id, start_line, end_line, quote, kind}` pointer wording; replaced the superseded `investigation_sources.rs` (deleted with `git rm`) with `split-1`'s `investigation_anchor.rs` (`SourceIndex`, `SelectedPages`, `AnchorReport`).
  - `crates/desktop-server/tests/investigation_flow.rs`: kept all 3 integration tests (`question_turn_session_view_and_handoff_research_after_mapping`, `mapping_turn_offers_no_compute_lean_explore_subagent`, `check_action_keeps_workers`).
  - `crates/desktop-server/tests/investigation_conventions.test.mjs`: kept both `header_hidden_with_paper_but_no_mapping` (`ux-ui`) and `pending_count_shown` (`split-1`).
- **Post-merge integration (`420910a`):**
  - Updated all 11 `prepare(...)` calls in `crates/desktop-server/src/investigation_tests.rs` to pass `PaperRoute::InCore`.
  - Made `investigation_question::conventions_resume_prompt` a `#[cfg(test)]` delegator to `crate::investigation::prompts::conventions_resume_instructions` so `resume_prompt_says_review_when_no_batch` tests the live merged prompt builder.
  - Extended `crates/desktop-server/src/investigation_cites.rs::collect_received_ranges` to accept `"read_paper_lines"` tool results (`val["a_meta"].{path,start_line,end_line,total_lines}`) alongside `"fs_read"` and `"search_paper_passages"` (per `ux-ui`'s merge notes).
  - Updated the exact expected string in `crates/research-tools/tests/investigation.rs::map_instructions_omit_conventions_unless_requested` to match the merged WP4 `read_paper_lines` / pointer-citation wording while retaining exact full-string `assert_eq!`.

### 1.5 `split-2` (merge `749c6cb`, shim replacement `00a192f`)
- **Conflicts:**
  - `crates/desktop-server/assets/investigation_map.js`: kept both `split-1`'s `cited_range` note (`lines corrected by the app (cited ...)`) and `split-2`'s `excerpt_basis === 'evidence'` badge (`located by reader`).
  - `crates/desktop-server/src/turn_runner.rs`: kept both `split-1`'s `paper_reading_turn` gating of `custom_subagents` (and `compute`/`lean`/`explorer` workers) and `split-2`'s `paper_reader: prepared_paper_reader.as_ref().map(|r| r.as_config())`.
  - `crates/research-tools/src/lib.rs`: kept `split-1`'s `paper_lines` and `split-2`'s `investigation_evidence`, dropping `investigation_anchor_shim` and `paper_lines_shim`.
  - `crates/research-tools/src/investigation.rs`, `investigation_summary.rs`, `investigation_conventions.rs`, `investigation_branch.rs`: kept `split-1`'s partial-acceptance `verify_any` loops and `prompts` module, plus `split-2`'s `history` and `active_marker_seq: Some(marker.seq)` on `SourceIndex`.
- **Shims replaced and deleted (`00a192f`):**
  - Deleted `crates/research-tools/src/investigation_anchor_shim.rs` and `crates/research-tools/src/paper_lines_shim.rs`.
  - Wired `crates/research-tools/src/investigation_evidence.rs` directly to `crate::investigation::anchor::*` and `crate::paper_lines::register_with_budget(..., READER_LINE_BUDGET)`, and added `"investigation.question_requested" | "investigation.answer"` to `find_active_marker` so question turns close stale mapping markers for readers as well.
  - Added `history: &'a [Event]` and `active_marker_seq: Option<u64>` to `SourceIndex` in `crates/research-tools/src/investigation_anchor.rs`, exposed `verify_pointer` and `is_bare_section_name` as `pub(crate)`, removed `#[allow(dead_code)]` on `AnchorBasis::Evidence`, and dispatched `anchor.evidence.is_some()` (when `excerpt.is_none()`) in `verify_any` to `crate::investigation_evidence::resolve_evidence`.
  - Updated `crates/research-tools/src/investigation_anchor_tests.rs::excerpt_and_quote_together_refused`: replaced the Phase 1 stub refusal code `"evidence_unavailable"` with Phase 2's fail-closed `"evidence_quote_missing"` (proving Guarantee #5: evidence anchors require the recorder's own quote).
  - Updated `crates/desktop-server/tests/paper_reader_wiring.rs::e2e_mock_reader_then_ev_anchors_finish_mapped`: under Decision D4 (`ux-rust`), the initial mapping prompt omits `record_paper_conventions` and its `request_seq=`, so the mock orchestrator now reads `request_seq` from the preceding `record_paper_map` tool result (`results[2]`) with fallback to `messages[0]`. All assertions (all 4 tools, `mapping_status == "mapped"`, `excerpt_basis == "evidence"`, ledger verification, 0 unreferenced blobs) remain unchanged.
  - Compacted anchor rendering lines in `crates/desktop-server/assets/investigation_map.js` from 605 to 598 lines to stay under the 600-line limit.

### 1.6 `split-3` (merge `f8709b2`, integration `1961485`, tidy-up `ed2b745`, race fix `a31a038`)
- **Conflicts:**
  - `crates/desktop-server/src/investigation.rs`: kept `ux-rust`'s P0 split (`view`/`branch_history` in `investigation_view.rs`, `finish` in `investigation_finish.rs`) and `Scope` routing; integrated `split-3`'s `conventions_resume_allows`, `conventions_expanding_to_paper` + `scope: "paper"` on `investigation.conventions_resumed`, `compute_scope` on both `Scope::Map` (retaining D4 `conventions_requested: false`) and `Scope::Question`.
  - `crates/research-tools/src/investigation_conventions.rs`: kept both `split-1`'s `rejected_items`/`done_requested`/`coverage_reported` metadata and `split-3`'s `coverage_scope` derivation (`active_marker`/`paper_resume`/`section_upgrade` auto-merged cleanly).
  - `crates/desktop-server/assets/investigation.js`: main owner `ux-ui` won the layout/state logic; re-applied `split-3`'s `scopeChip` (`init`, `reset` on session switch and non-Investigation sessions, `scopeChip.update(inv.scope)`) and `inv.conventions_expanding === true` polling check.
  - `crates/desktop-server/assets/investigation_conventions.js`: combined `split-1`'s `pendingCitationsLabel` suffix with `split-3`'s `" · section catalog"` label and `"Catalog all conventions"` button text, while preserving `ux-ui`'s `host.hidden` header gate (without `inv.source_id`).
  - `crates/desktop-server/assets/investigation.css`, `src/assets.rs`, `src/lib.rs`, `tests/investigation_origin.test.mjs`, `crates/research-tools/src/lib.rs`: additive merges keeping all rules, assets, modules, and test stubs from every brief.
- **Post-merge integration (`1961485`):**
  - Added `PreparedScope::question_instructions_suffix` in `crates/desktop-server/src/investigation_scope.rs` and wired `split-3`'s outline and question scope into `investigation_question::prepare_question` (recording `investigation.scope` with `request_seq == q_seq` and optional `scope` on `investigation.question_requested`, injecting outline + scoped-line focus without adding mapping or conventions duties, per section 2.6 of design 01 and D1/D2 of design 02).
  - Exposed `scope` (matching the latest `investigation.mapping_requested` or `investigation.question_requested` event so the UI scope chip works on question-first turns as well as mapped turns) and `conventions_expanding` in `crates/desktop-server/src/investigation_view.rs`.
  - Wired `investigation_scope::conventions_finish_done` into `crates/desktop-server/src/investigation_finish.rs`.
  - Added `expanding: bool` to `investigation_prompts::conventions_resume_instructions` so section-to-paper expansion uses `"Expand the section conventions catalog to cover the whole paper."` while retaining `split-1`'s `pending_items`, `read_paper_lines`, pointer citation, and `PAPER_READING_NO_DELEGATION_NOTE` wording.
  - Fixed schema alignment between `split-2` and `split-3` in `crates/desktop-server/src/paper_reader.rs`: accepted `scope.status == "resolved"` in `is_section_scoped`, extracted bounding `(min_start, max_end)` from `split-3`'s `scope.ranges` array in `prepare_for_turn` (so resolved sections actually scope parallel paper-reader slices), and added `"investigation.question_requested" | "investigation.answer"` to `active_marker`.
  - Updated `crates/desktop-server/src/investigation_scope_tests.rs` for `PaperRoute::InCore` and added `question_turn_writes_scope_and_injects_outline`.
- **Tidy-up (`ed2b745`):**
  - Extracted `Scope::Map` preparation into `investigation_scope::prepare_map`, bringing `crates/desktop-server/src/investigation.rs` from 618 down to 584 lines (`<= 600`).
  - Added `investigation_cites::tests::read_paper_lines_marks_cites_read` verifying P5 line citations against `split-1`'s `read_paper_lines` `a_meta` output.
- **Snapshot race fix (`a31a038`):**
  - Observed a transient failure of `paper_reader_wiring::e2e_mock_reader_then_ev_anchors_finish_mapped` under full-workspace load where `GET /api/sessions/{id}` returned `running: false` while `mapping_status` was still `null` (last event `session.end`).
  - Root cause in `crates/desktop-server/src/routes_session.rs::handle_get_session`: `ledger.read_session` ran *before* `is_turn_running(state, session_id)` (which was also called twice), allowing a torn read when `investigation::finish` + `active_turns.remove` completed between the two calls.
  - Fixed by sampling `let running = is_turn_running(state, session_id);` once *before* `ledger.read_session`, guaranteeing any `running == false` response sees events written by `investigation::finish`.

---

## 2. Decisions taken unattended

1. **Question-turn scope prompt wording:** Added `PreparedScope::question_instructions_suffix()` rather than reusing `mapping_instructions_suffix()` on `Scope::Question`, because `mapping_instructions_suffix()` asks to map main results and record conventions, which would contradict Design 02 D2 ("Do not build a paper overview, results map or conventions catalog").
2. **`view()["scope"]` source event:** Selected the latest of `investigation.mapping_requested` or `investigation.question_requested` so question-first sessions display the scope chip before any map exists, and mapping sessions remain identical to `split-3`.
3. **Reader gating on question turns:** Kept `paper_reader::is_paper_recording_turn` scoped to mapping, branch, and conventions turns (not question turns), matching `split-2` WP8 and `01-reader-recorder-split.md` §2.5 (`note_evidence` is gated to paper-recording markers).
4. **No `Cargo.toml` / dependency changes:** `policy` and `ledger` remain untouched and free of `tokio`, `rmcp`, and `windows-sys`.
5. **600-line check across all changed files:** Every new/merged production module is `<= 600` lines (`investigation.rs`: 584, `investigation_scope.rs`: 336, `investigation_cites.rs`: 481, `investigation_map.js`: 598, `investigation_anchor.rs`: 585, `turn_runner.rs`: 583). Only pre-existing or explicitly exempted files exceed 600 lines: `assets/app.js` (932, exempted by Decision 6), `src/routes_session.rs` (616, down from 629 at base), `research-tools/tests/investigation.rs` (777, was 671 at base), `desktop-server/tests/math_render.test.mjs` (652, was 651 at base), and `research-tools/tests/investigation_evidence.rs` (641, test file from `split-2`).

---

## 3. Final verification and exact test counts (at `a31a038`)

Environment: Linux (Ubuntu 24.04), `rustc 1.100.0`, `cargo 1.100.0`, `node v22.23.5`, `RE_SEARCH_PYTHON="$(command -v python3)"` (Python 3.12 with `sympy`, `numpy`, `mpmath`).

| Command | Result | Exact counts |
|---|---|---|
| `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` | **PASS** | All JS assets syntax-checked, exit 0 |
| `node --test crates/desktop-server/tests/*.test.mjs` | **PASS** | **109 passed, 0 failed, 0 skipped** |
| `python3 -m py_compile crates/desktop-server/tests/investigation_browser.py` | **PASS** | Exit 0 |
| `RE_SEARCH_PYTHON="$(command -v python3)" cargo test --workspace --no-fail-fast -- --skip cli_research_workspace_read_fork_diff_export_and_verify` | **PASS** | **83 suites: 544 passed, 0 failed, 0 ignored, 1 skipped** |
| `cargo clippy --workspace --all-targets -- -D warnings -A clippy::manual_unwrap_or -A clippy::items_after_test_module -A unused_imports` | **PASS** | **0 warnings / 0 errors** across all 13 crates |
| `cargo clippy --workspace --all-targets -- -D warnings` | **FAIL (pre-existing only)** | Fails solely on 4 pre-existing Linux / Rust 1.100.0 diagnostics in untouched files (see §4) |

---

## 4. Pre-existing failures / diagnostics on Linux (nothing new failing)

1. **`crates/cli/tests/desktop_and_ext.rs::cli_research_workspace_read_fork_diff_export_and_verify`** — skipped via `-- --skip` per `agents/merge.md`: known to hang on Linux when `RE_SEARCH_PYTHON` is exported because `re-search run --route api` rejects non-Windows Python worker config and exits before `ScriptedMock` finishes accepting connections.
2. **4 pre-existing `cargo clippy --workspace --all-targets -- -D warnings` diagnostics on Linux / Rust 1.100.0** in untouched files (all verified present on base `ea6b5e84` and reported by individual briefs):
   - `crates/desktop-server/src/evidence.rs:737` — `clippy::manual_unwrap_or` (false positive in Rust 1.100.0 on guarded allowlist match `Some(status @ (...)) => status`; applying clippy's autofix would weaken the status allowlist).
   - `crates/agent/src/guardrails.rs:335` — `clippy::items_after_test_module`.
   - `crates/research-tools/tests/sandbox_and_fs.rs:6` — `unused_imports` (`TEX_AUDIT`, `is_symlink_or_junction` used only under `#[cfg(windows)]`).
   - `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` — `unused_imports` (`BufRead`, `BufReader` used only under `#[cfg(windows)]`).

---

## 5. What only Windows can check

- All `#[cfg(windows)]` code and tests: `crates/worker` (`AppContainerSandbox`, `JobObjectSandbox`, `appcontainer_sandbox`, `python_worker`), `crates/cli` (`contained_router`, `init_abort`), `crates/desktop-server` (`claude_cli_route`, `chatgpt_route`, Windows worker/process-tree paths), `crates/desktop-app` (WebView2 shell), and `crates/chatgpt-auth` (DPAPI vault and `vault_lock_multiprocess`).
- Playwright / real-browser end-to-end tests (`crates/desktop-server/tests/investigation_browser.py` and `workbench_browser.py`), including the new question-first -> `"Find main results"`, Esc/Stop, citation chip, scope chip, and `"Catalog all conventions"` UI journeys.
- Visual LaTeX parity checks in WebView2 (light & dark themes) listed in `03-latex-kandinsky-parity.md` §4 and `reports/latex.md` §7.
- Live provider/CLI turns and real-paper performance/latency measurements.
