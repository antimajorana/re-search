# Brief Report: `split-2` (Phase 2: Isolated Parallel Paper Readers — WP6, WP7, WP8, WP9)

- **Branch:** `work/split-2` (pushed to `gemini/work/split-2`)
- **Base commit:** `91753b86b4392814cbe0fe4dd213c9be1d15fc80` (`origin/gemini/runner`)
- **Toolchain (Linux):** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, Node `v22.23.5`, Python `3.12.16`
- **No new Cargo dependencies** added (`Cargo.toml`, `Cargo.lock`, and all `crates/*/Cargo.toml` unchanged; `policy` and `ledger` remain free of `tokio`, `rmcp`, and `windows-sys`).

---

## 1. Summary by Work Package

### WP7: Evidence Store (`research-tools`) — commit `dc05c63`
- **`crates/research-tools/src/investigation_evidence.rs`** (392 lines):
  - Implemented `note_evidence` (`NOTE_EVIDENCE`), `note_evidence_with_hook` (test seam for `SessionChanged` interleaving), `register_reader_tools`, `EvidenceRecord` (`#[serde(deny_unknown_fields)]`), and `resolve_evidence`.
  - `register_reader_tools` registers **only** the 3 allowed reader tools: `search_paper_passages`, `read_paper_lines` (at 7,000-char canonical JSON budget), and `note_evidence`.
  - `note_evidence` is **never** registered in `register_research_tools` or `all_research_tool_defs`.
  - Enforces caller gating (`worker:paper_reader:...`), active Investigation marker gating (refuses without active marker, after `*_finished`, after `session.end` / `session.abort`, or after `investigation.handoff`), per-item batch results, `verify_pointer` verification, and `append_at_head` with bounded `SessionChanged` retry (up to 3 retries; returns `ledger_busy` after exhaustion without surfacing `SessionChanged`).
  - `resolve_evidence` enforces every Section 2.5 check: `ev-N` exists in current active `investigation.started` scope, event kind is `investigation.evidence` (not `tool.request` or another event), `marker_seq` matches active marker, `source_id` matches anchor, file SHA-256 unchanged on disk, window re-sliced from `start_line..=end_line` matches `excerpt`, and recorder's `quote` matches inside the evidence excerpt (producing `excerpt_basis: "evidence"`).
- **Phase 1 Shims** (documented in detail under **Merge notes** below):
  - `crates/research-tools/src/investigation_anchor_shim.rs` (485 lines): Section 2.1 `Anchor`, `anchor_schema`, `AnchorFault`, `AnchorNote`, `VerifiedFile`, `SelectedPages`, `SourceIndex`, legacy `verify`, 10-step `verify_pointer`, `reject_app_only_fields`, and `verify_any` (dispatching `evidence` anchors to `resolve_evidence`).
  - `crates/research-tools/src/paper_lines_shim.rs` (198 lines): Section 2.3 `read_paper_lines` (`register_with_budget`, line-boundary cut, `a_meta`, `next_line`, `line_truncated`, hash pin, assembled TeX refusal).
- **Recorder integration:** Updated `investigation.rs` (reduced from 772 to 512 lines), `investigation_summary.rs` (83 lines), `investigation_conventions.rs` (194 lines), `investigation_branch.rs` (196 lines), and `lib.rs` (266 lines) to verify anchors via `verify_any`.
- **Tests (`crates/research-tools/tests/investigation_evidence.rs`):** All 14 design-specified WP7 tests pass.

### WP6: Agent Fan-out (`agent`) — commits `c197070`, `589e591`
- **`crates/agent/src/paper_read.rs`** (542 lines):
  - Implemented `DELEGATE_READ_PAPER` (`delegate_read_paper`), `PaperReaderConfig`, `PaperReaderSlice`, `paper_read_tool_spec`, and `run_paper_readers`.
  - Runs slice readers concurrently in `std::thread::scope` via a work queue bounded by `max_parallel`.
  - Enforces the strict 3-tool whitelist (`search_paper_passages`, `read_paper_lines`, `note_evidence`).
  - Final-turn wrap-up (`PAPER_READER_MAX_TURNS = 12`): on the final turn offers **only** `note_evidence` and denies any other dispatched tool with `DENIED`, while preserving all already-noted pointers on `max_turns` or transport errors.
  - Transport errors record per-slice gaps without losing other slices; `Router::try_call` `Err` sets a shared `AtomicBool` abort flag, stops peer readers, and fails the turn.
  - Live progress sent through `mpsc` to the thinking sink; post-join `flush_interleaved` emits per-reader `delegate.turn` blobs tagged `reader: i` (preserving exact tool-result interleaving for resume).
  - Deterministic `<= 24 KB` return trimming (`g` stripped in `scope` -> `method` -> `motivation` order, then pointers dropped by focus relevance/priority with `omitted` counter); `mode: "list"` zero-model rebuild from ledger evidence.
  - Writes `delegate.start` and **exactly one** `delegate.end` with `raw_blob` equal to the exact returned string across all exit paths (`read`, `list`, cancel, arg error).
- **Hooks:** `crates/agent/src/delegate.rs` (586 lines), `crates/agent/src/lib.rs` (94 lines), and `crates/agent/src/honesty.rs` (448 lines; maps `paper_reader` -> `CampaignPhase::Ingest`).
- **Tests (`crates/agent/src/paper_read_tests.rs`, 562 lines):** All 12 design-specified WP6 tests pass.

### WP8: Desktop Wiring (`desktop-server`) — commit `5ab27ed`
- **`crates/desktop-server/src/paper_reader.rs`** (458 lines):
  - `build_reader_router` (isolated reader `Registry` via `register_reader_tools` + `research_policy()` + `UiApprover`), `plan_slices` (PDF page boundaries or ~45 KB blank-line-snapped ranges), `default_max_parallel` (2 on ChatGPT-plan routes, 4 on API keys, Decision 5), `effective_max_parallel` (clamped `1..=8`), `parse_reader_settings_patch` (`ReaderSettingsPatch`), `should_enable_readers` (off by default; on automatically only for smaller/dedicated reader model or section-scoped turn, otherwise opt-in via `paper_readers_enabled`, Decision 7), `prepare_for_turn`, `append_reader_guidance` (Phase 2 sentence injected for `InCore` turns only), and `poll_event_payload` (strips `excerpt` and `quote` from `investigation.evidence` in the 4-second poll response, emitting `excerpt_chars`).
- **Wiring edits:**
  - `crates/desktop-server/src/agent_models.rs` (546 lines): added `"paper_reader"` to `AGENT_ROLE_KEYS` and `resolve_paper_reader_transport` (falls back to primary provider/model with `routing_reason: "isolated_primary_model"`, Decision 4).
  - `crates/desktop-server/src/subagents.rs` (490 lines): added `paper-reader` / `paper_reader` to `RESERVED_IDS` and built-in metadata in `builtin_agents_json`.
  - `crates/desktop-server/src/state.rs` (598 lines): persisted `paper_readers_enabled: Option<bool>` and `paper_reader_max_parallel: Option<usize>` in `UiSettings` and `settings_json()`.
  - `crates/desktop-server/src/turn_runner.rs` (568 lines): parameterized `make_worker_spec` with `effort: Option<String>` (`Some("low")` for `paper_reader` per Decision 6; `None` for all existing worker roles), prepared `PaperReaderConfig` on `InCore` paper-recording turns, and appended Phase 2 reader guidance.
  - `crates/desktop-server/src/routes_session.rs` (620 lines, reduced from 630 at base): wired `poll_event_payload` in `handle_get_session` and `parse_reader_settings_patch` in `handle_put_settings`.
- **Tests (`crates/desktop-server/tests/paper_reader_wiring.rs`, 492 lines):** All 6 design-specified WP8 tests pass.

### WP9: UI, Phase 2 (`desktop-server/assets`) — commit `95bbb83`
- **`crates/desktop-server/assets/trace_cards.js`** (554 lines): added `delegate_read_paper: 'Paper Reader'` card rendering coverage, slice status/errors, omitted count, and evidence pointers with **all** untrusted text (`focus`, `lab`, `g`, `q`, `r`, `e`, `l`, slice `error`) passed through `escapeHtml` or `mathTextToHtml`.
- **`crates/desktop-server/assets/subagents.js`** (465 lines): renders `Default (Primary Model)` option label for `paper_reader`, opt-in checkbox `#paper-readers-enabled`, and max-parallel selector `#paper-reader-max-parallel` (1..8 + default), persisted via `api.updateProviderSettings`.
- **`crates/desktop-server/assets/investigation_map.js`** (529 lines): renders `<small class="investigation-anchor-basis">located by reader</small>` in `sources()` when `anchor.excerpt_basis === 'evidence'`.
- **Tests (`crates/desktop-server/tests/paper_reader_ui.test.mjs`, 153 lines):** Both design-specified WP9 tests (`reader_card_escapes_gist_label_focus`, `located_by_reader_label`) pass.

---

## 2. Deviations from the Design & Why

1. **Reconciled `evidence` + `quote` anchor form (Section 2.1 vs. Sections 2.4/2.5/2.7):**
   - Section 2.1 states "exactly one of `excerpt`, `quote` or `evidence`", whereas Sections 2.4, 2.5, and 2.7 require `{source_id, evidence: "ev-N", quote, kind, label?}` so `resolve_evidence` can verify the recorder's own `quote` inside the reader's evidence excerpt.
   - Implemented the three valid mutually exclusive forms: (1) legacy `excerpt` only, (2) pointer `quote` only (`start_line`..`end_line`), and (3) `evidence` + `quote` (never `excerpt` combined with either; `evidence` without `quote` is rejected).
2. **Phase 2 guidance injected in `turn_runner.rs` instead of `crates/desktop-server/src/investigation.rs`:**
   - At base `91753b8`, `crates/desktop-server/src/investigation.rs` is 811 lines and owned by `ux-rust` (with `split-1` and `split-3` also editing it).
   - To avoid merge conflicts and keep every file we touch compliant with the 600-line rule, Phase 2 guidance (`PHASE2_READER_GUIDANCE`) is appended in `turn_runner.rs::run_session_turn` (`InCore` path only) via `paper_reader::append_reader_guidance`, which guarantees CLI routes never receive it.
3. **WP8 integration tests placed in dedicated `crates/desktop-server/tests/paper_reader_wiring.rs`:**
   - Instead of appending to `stage14_worker_delegation.rs`, `subagents_and_effort.rs`, and `investigation_flow.rs` (which other briefs may touch), all 6 required WP8 tests live in a new `paper_reader_wiring.rs` while all existing tests in those three suites were re-run and pass unchanged.
4. **Line counts:**
   - Every new and edited source file is `<= 600` lines except `crates/desktop-server/src/routes_session.rs`, which was already 630 lines at base `91753b8` and is now **620 lines** (net -10 lines), and the integration test file `crates/research-tools/tests/investigation_evidence.rs` (641 lines).

---

## 3. Decisions Taken Alone

1. **Automatic enablement check (`should_enable_readers`):** Precedence is: explicit per-request `paper_readers_enabled` bool > saved `ui.paper_readers_enabled` (`Some(true)` / `Some(false)`) > automatic rule (`has_dedicated_reader_model || is_section_scoped`). A configured `paper_reader` model counts as dedicated/smaller whenever its resolved `(provider, model)` differs from the turn's primary `(provider, model)`. Section-scoped detection checks both `req["_paper_scope"]["resolved"] == true` and `marker.payload["conventions_scope"] == "section"` / `marker.payload["scope"]["resolved"] == true` so `split-3` integrates cleanly.
2. **Slice planning fallback (`plan_slices`):** Uses PDF `pdf_page_lines` (filtered by `selected_pages` when present and no `scope_lines` override) when available; otherwise splits `scope_lines` (or `1..=total_lines`) into `ceil(active_chars / 45_000)` clamped to `1..=n_max` slices, snapping cut points to the nearest blank line within `+-20` lines.
3. **Reader settings validation:** `paper_reader_max_parallel` accepts integer `1..=8` or `null` (reset to provider default); `paper_readers_enabled` accepts `bool` or `null` (reset to automatic default). Invalid values return HTTP 400.
4. **Reader card XSS test assertions:** Confirmed that `trace_cards.js` escapes `<script>`, `<svg>`, `<img` into `&lt;...&gt;` HTML entities and verified both the absence of raw HTML elements/event-handler attributes (`/<(?:script|img|svg)\b/i`, `/<[^>]*\son[a-z]+\s*=/i`) and the presence of the escaped entity text.

---

## 4. Test Commands & Exact Pass/Fail Counts

### Targeted WP6 / WP7 / WP8 / WP9 Runs
| Command | Passed | Failed | Ignored |
|---|---:|---:|---:|
| `cargo test -p research-tools --test investigation_evidence --test investigation` | 20 (14 + 6) | 0 | 0 |
| `cargo test -p agent --lib paper_read` | 12 | 0 | 0 |
| `RE_SEARCH_PYTHON="$(command -v python3)" cargo test -p desktop-server --test paper_reader_wiring --test stage14_worker_delegation --test subagents_and_effort --test investigation_flow` | 12 (6 + 4 + 1 + 1) | 0 | 0 |
| `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` | all assets OK | 0 | 0 |
| `node --test crates/desktop-server/tests/*.test.mjs` | 65 (63 existing + 2 new) | 0 | 0 |

### Full Workspace Verification
1. **`RE_SEARCH_PYTHON="$(command -v python3)" cargo test --workspace --no-fail-fast -- --skip cli_research_workspace_read_fork_diff_export_and_verify`** followed by **`cargo test -p cli --test desktop_and_ext cli_research_workspace_read_fork_diff_export_and_verify`**:
   - **Total across workspace: 458 passed, 2 failed, 0 ignored.**
   - Every crate (`agent`: 91/91, `chatgpt-auth`: 28/28, `cli`: 36/36, `cli-route`: 56/56, `desktop-app`: 21/21, `desktop-server`: 104/104, `ledger`: 17/17, `mcp-server`: 1/1, `policy`: 9/9, `research-tools`: 60/60, `tools`: 23/23, `worker`: 1/1) passed 100%, except `export` (6 passed, 2 failed — both pre-existing, proven below).
2. **`cargo clippy --workspace --all-targets`**:
   - **0 warnings** in any file created or modified on `work/split-2`.
   - 5 pre-existing warnings remain in untouched files (proven on base commit `91753b8` below), causing `-- -D warnings` to exit 101 identically on base and on this branch.

---

## 5. Pre-Existing Failures & Warnings on Linux (Proven on Base `91753b8`)

Verified in a clean detached worktree at base commit `91753b86b4392814cbe0fe4dd213c9be1d15fc80`:

### Pre-Existing Test Failures (2)
Both fail identically on `91753b8` because the container's `python3` lacks `sympy`:
- `export::tests/export_integrity.rs::handoff_checks_source_hashes_and_compares_seeded_values` — `ModuleNotFoundError: No module named 'sympy'`
- `export::tests/fork_diff_export.rs::export_tex_html_and_reproducible_zip_and_replay_checks` — `ModuleNotFoundError: No module named 'sympy'`

### Pre-Existing Clippy Warnings (5, Rust `1.100.0`, Linux)
All 5 reproduce identically on `91753b8`:
1. `crates/desktop-server/src/evidence.rs:737` — `clippy::manual_unwrap_or` (false positive; see Bugs Noticed below)
2. `crates/desktop-server/src/investigation.rs:384` — `clippy::items_after_test_module`
3. `crates/agent/src/guardrails.rs:335` — `clippy::items_after_test_module`
4. `crates/research-tools/tests/sandbox_and_fs.rs:6` — `unused_imports` (`TEX_AUDIT`, `is_symlink_or_junction`, only used in `#[cfg(windows)]` tests)
5. `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` — `unused_imports` (`BufRead`, `BufReader`, only used in `#[cfg(windows)]` tests)

---

## 6. What Could Not Be Compiled or Run on Linux

- All `#[cfg(windows)]` code paths (AppContainer sandbox in `crates/worker`, DPAPI vault/key protection in `crates/chatgpt-auth`, WebView2 desktop shell in `crates/desktop-app`, and Windows-gated tests in `worker/tests/appcontainer_sandbox.rs`, `worker/tests/python_worker.rs`, `sandbox_and_fs.rs`, `vault_lock_multiprocess.rs`).
- Playwright / real-browser end-to-end tests (out of scope per `RULES.md`; to be run on Windows by the coordinator).

---

## 7. Bugs Noticed But Not Fixed

- **`crates/desktop-server/src/evidence.rs:737-743`:** Under `rustc 1.100.0`, `clippy::manual_unwrap_or` triggers a **false positive** on `match result["status"].as_str() { Some(status @ ("verified" | ...)) => status, _ => "result_unavailable" }`. Accepting clippy's autofix (`unwrap_or`) would weaken the status allowlist; suppressing with `#[allow(clippy::manual_unwrap_or)]` or rewriting as `.filter(|s| matches!(...))` is the safe fix. Left untouched because `evidence.rs` is outside `split-2` scope.
- **`crates/cli/tests/desktop_and_ext.rs:55-85`:** `ScriptedMock::start` loops `for val in replies { listener.accept().unwrap(); ... }` with no listener timeout or shutdown signal. If the spawned `re-search` child exits early with an error before making all expected requests, `mock.finish()` blocks indefinitely on `listener.accept()`. Left untouched as outside scope.

---

## 8. Merge Notes

### Shims to Replace with `split-1`'s Real Modules
`split-2` created two shims in `crates/research-tools/src/` matching the Sections 2.1–2.3 interfaces. At merge time, replace both with `split-1`'s real implementations:

1. **`crates/research-tools/src/investigation_anchor_shim.rs`** -> replace with `split-1`'s **`crates/research-tools/src/investigation_anchor.rs`**:
   - Types/functions consumed by `investigation_evidence.rs` and `investigation*.rs`: `Anchor`, `anchor_schema`, `AnchorFault`, `AnchorNote`, `VerifiedFile`, `SelectedPages`, `SourceIndex` (`new`, `verify`, `verify_pointer`, `verify_any`), `reject_app_only_fields`, `ANCHOR_KINDS`, `MAX_FILE_BYTES`, `MAX_TOTAL_BYTES`.
   - Ensure `split-1`'s `investigation_anchor.rs` keeps the `evidence` + `quote` dispatch in `verify_any` (calling `crate::investigation_evidence::resolve_evidence`) and retains `evidence: Option<String>` on `Anchor` and in `anchor_schema()`.
   - Remove `pub mod investigation_anchor_shim;` from `crates/research-tools/src/lib.rs` and update `use crate::investigation_anchor_shim::*` imports in `investigation.rs`, `investigation_evidence.rs`, `investigation_summary.rs`, `investigation_conventions.rs`, `investigation_branch.rs`, and `tests/investigation_evidence.rs` to `crate::investigation_anchor::*`.

2. **`crates/research-tools/src/paper_lines_shim.rs`** -> replace with `split-1`'s **`crates/research-tools/src/paper_lines.rs`**:
   - Symbols consumed by `investigation_evidence.rs::register_reader_tools`: `READ_PAPER_LINES` and `register_with_budget(registry, env, READER_LINE_BUDGET)` (7,000-char canonical JSON budget on the reader router).
   - Remove `pub mod paper_lines_shim;` from `crates/research-tools/src/lib.rs` and update the import in `investigation_evidence.rs` to `crate::paper_lines`.

### Shared Files Edited by `split-2`
| File | What `split-2` changed | Integration note for other briefs |
|---|---|---|
| `crates/research-tools/src/lib.rs` | Exported `investigation_evidence` (`NOTE_EVIDENCE`, `EVIDENCE_EVENT`, `READER_CALLER_PREFIX`, `note_evidence_def`, `register_reader_tools`, `EvidenceRecord`) and the two shim modules. `note_evidence` is **not** added to `register_research_tools` or `all_research_tool_defs`. | Replace the two shim `pub mod` lines with `split-1`'s `investigation_anchor` / `paper_lines`. |
| `crates/research-tools/src/investigation.rs` | Switched anchor storage/verification to `investigation_anchor_shim::*` (`verify_any`); kept under 600 lines (512 lines). | `ux-rust` (P0 move) and `split-1` (partial acceptance / pointer verification) also edit this file; keep `verify_any` dispatch to `resolve_evidence`. |
| `crates/research-tools/src/investigation_summary.rs`, `investigation_conventions.rs`, `investigation_branch.rs` | Switched `index.verify(anchor)` calls to `index.verify_any(owner, anchor)` so `evidence` anchors work across all four paper recorders. | `split-1` (partial acceptance) and `split-3` (`coverage_scope` in `investigation_conventions.rs`) also touch these; retain `evidence` anchor resolution in all four recorders. |
| `crates/agent/src/delegate.rs` | Added `paper_reader: Option<PaperReaderConfig<'a>>` to `DelegationConfig`, added `DELEGATE_READ_PAPER` to `is_delegation_tool`, `delegation_tool_specs`, and `run_delegation_call` (586 lines). | Keep `delegate.rs <= 600` lines; `split-1` may gate other workers in `DelegationConfig` without affecting `paper_reader`. |
| `crates/agent/src/lib.rs` | Added `pub mod paper_read;` and re-exports. | Trivial additive merge. |
| `crates/agent/src/honesty.rs` | Mapped `"paper_reader"` in `infer_campaign_phase` to `CampaignPhase::Ingest` (+1 unit test line). | Trivial 2-line merge. |
| `crates/desktop-server/src/lib.rs` | Added `pub mod paper_reader;`. | Trivial additive merge. |
| `crates/desktop-server/src/turn_runner.rs` | Added `effort: Option<String>` param to `make_worker_spec` (`None` for existing workers, `Some("low")` for `paper_reader`); wired `paper_reader::prepare_for_turn` + `append_reader_guidance` on `InCore` turns only. | `ux-rust` (Investigation system prompt) and `split-1` (`prepare(route)` / `DelegationConfig` worker gating) also touch `turn_runner.rs`; ensure `paper_reader` stays wired on `InCore` paper-recording turns and `append_reader_guidance` runs only when `prepared_paper_reader.is_some()`. |
| `crates/desktop-server/src/agent_models.rs` | Added `"paper_reader"` to `AGENT_ROLE_KEYS` and `resolve_paper_reader_transport`. | Additive; no conflict expected. |
| `crates/desktop-server/src/subagents.rs` | Added `"paper-reader"` / `"paper_reader"` to `RESERVED_IDS` and `builtin_agents_json()`. | Additive; no conflict expected. |
| `crates/desktop-server/src/state.rs` | Added `paper_readers_enabled` and `paper_reader_max_parallel` to `UiSettings` and `settings_json()` (598 lines). | Keep `state.rs <= 600` lines if another brief adds fields. |
| `crates/desktop-server/src/routes_session.rs` | Wired `paper_reader::poll_event_payload` in `handle_get_session` and `parse_reader_settings_patch` in `handle_put_settings` (620 lines, down from 630). | Preserve `poll_event_payload` sanitization on `investigation.evidence`. |
| `crates/desktop-server/assets/trace_cards.js` | Added `delegate_read_paper` title, metadata summary, slice list, and escaped evidence pointer list. | Additive; preserve escaping for all untrusted text. |
| `crates/desktop-server/assets/subagents.js` | Added `Default (Primary Model)` label, `#paper-readers-enabled` checkbox, and `#paper-reader-max-parallel` dropdown for `paper_reader`. | Additive; preserve `paper_readers_enabled` and `paper_reader_max_parallel` persistence. |
| `crates/desktop-server/assets/investigation_map.js` | Added 3-line `'located by reader'` `<small class="investigation-anchor-basis">` label in `sources()` when `anchor.excerpt_basis === 'evidence'`. | `split-1` (WP5 quote highlighting / corrected note) and `ux-ui` (`openFirst` / header gate) also edit `investigation_map.js`; preserve the `excerpt_basis === 'evidence'` badge in `sources()`. |
| `crates/desktop-server/src/investigation.rs` | **Not modified by `split-2`.** | `ux-rust`, `split-1`, and `split-3` own this file. If the coordinator later prefers placing `PHASE2_READER_GUIDANCE` inside `investigation_prompts.rs` (`InCore` only), remove `append_reader_guidance` from `turn_runner.rs` at the same time so the sentence is not duplicated. |
