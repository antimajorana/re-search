# Agent Report: split-3 (WP10, WP11, WP12)

- **Base commit:** `91753b86b4392814cbe0fe4dd213c9be1d15fc80` (`gemini/runner`)
- **Working branch:** `work/split-3`
- **Toolchain:** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `cargo 1.100.0`, Ubuntu 24.04 Linux, Node.js 22

---

## 1. What was implemented, package by package

### WP10: Deterministic paper outline and question-scope resolver
- **New files:**
  - `crates/research-tools/src/investigation_outline.rs` (564 lines): public types (`OutlineSection`, `OutlineItem`, `PaperOutline`, `ScopeTarget`, `ScopeResolution`), `build_outline(text, page_lines)`, `format_outline_for_prompt(outline, max_bytes)`, `resolve_scope(outline, question)`, plain-text equation/caption indexers, and question reference parser/matcher.
  - `crates/research-tools/src/investigation_outline_plain.rs` (553 lines, `#[path]` submodule): plain-text/PDF heading scanner, APS Roman/Letter/Arabic assembler with successor checks, dotted Arabic assembler, appendix/fixed-heading handling, and numbered/lettered list false-positive guards.
  - `crates/research-tools/src/investigation_outline_tex.rs` (206 lines, `#[path]` submodule): LaTeX `\section`/`\subsection`/`\subsubsection`/`\chapter`/`\appendix`, `\begin{abstract}`, `\begin{thebibliography}`, equation (`\begin{equation}`/`align`/`\tag{}`), and figure/table `\caption{}` extractor.
  - `crates/research-tools/tests/investigation_outline.rs` (421 lines): 6 tests covering all required fixtures and scope query variants.
- **Edited files:**
  - `crates/research-tools/src/lib.rs`: added `pub mod investigation_outline;`.

#### Outline performance on each fixture
1. **APS paper fixture** (`aps_fixture_detects_v_d_4_appendices_equations_captions_and_skips_lists` & `scope_parsing_handles_v_d_4_variants_appendix_eq_fig_table_and_unresolved`):
   - Realistic RevTeX-style extracted text with `Abstract`, Roman sections `I.`–`VI.`, letter subsections `A.`–`D.` under `V.`, Arabic subsubsections `1.`–`4.` under `D.` (`"4. Dislocations and Twist Defects"` under `"D."` under `"V."`), `Acknowledgments`, `Appendix A`, `Appendix B`, `B.1`, `References`, equations `(1)`, `(52)`, `(A3)`, figures `FIG. 1.`, `FIG. 3.`, table `TABLE A2.`, stray author initials (`J. Smith, Phys. Rev. B ...`), an in-section 3-item numbered list inside `V.D.4`, and numbered references.
   - **Result:** `style == "aps"`, `weak == false`. Detects all 19 structural sections including `V.D.4` (level 3, bounded up to `VI.start_line - 1`, page 3), parent `V.D` spanning `V.D.1`–`V.D.4`, appendices `A`, `B`, `B.1`, equations `52` and `A3`, `Fig. 3`, and `Table A2`. Rejects author initials, the in-section numbered list, numbered references, and 4-digit years (`2003`, `2019`).
   - **Scope queries resolved:** `"Help me understand section V.D.4"`, `"What does § V.D.4 prove?"`, `"Explain §V.D.4 carefully"`, `"Summarize sec. V.D.4"`, `"Walk me through Sec. V D 4 please"`, `"help me understand section v.d.4"`, `"appendix B"`, `"App. B.1"`, `"eq. (52)"`, `"fig. 3"`, `"Table A2"`.
   - **Scope queries unresolved:** general questions (`"What are the main results of this paper?"`, `"Explain the section in detail without citing a number"`) and absent targets (`"section IX.Z.9"`, `"Appendix Z"`, `"Eq. (999)"`).
   - `format_outline_for_prompt(&outline, 1536)` fits within 1,536 bytes and includes `V.D.4`.
2. **Dotted Arabic fixture** (`dotted_arabic_fixture_builds_hierarchy_and_resolves`):
   - `Abstract`, `1.`, `2.`, `2.1`, `2.2`, `2.2.1`, `3.`, `Appendix A`, Eq. `(2.1)`.
   - **Result:** `style == "dotted"`, `weak == false`, exact section list `["Abstract", "1", "2", "2.1", "2.2", "2.2.1", "3", "A"]`; resolves combined query `"Explain section 2.2.1 and eq. (2.1)"` to both targets.
3. **TeX fixture** (`tex_fixture_parses_sections_appendices_equations_and_captions`):
   - `\begin{abstract}`, commented-out `% \section{...}`, `\section`, `\subsection`, `\subsubsection`, `\begin{equation}`, `\tag{A1}`, `\begin{figure}`/`\caption`, `\begin{table}`/`\caption`, `\appendix`, `\begin{thebibliography}`.
   - **Result:** `style == "tex"`, `weak == false`; skips commented section, builds `1`, `2`, `2.1`, `2.1.1`, `A`, `Abstract`, `References`, indexes equations `1` and `A1` plus figure 1 and table 1, and resolves `"section 2.1.1"` and `"appendix A and Eq. (A1)"`.
4. **Numbered-list false positives fixture** (`numbered_list_false_positives_are_rejected`):
   - Colon-led numbered list (`We summarize our assumptions as follows:\n1. ...`), packed sentence list (`1. ... .\n2. ... .`), wrapped multi-line sentence list (`... with\n...`), and packed lettered list (`A. ...\nB. ...`) between `I. INTRODUCTION` and `II. CONCLUSION`.
   - **Result:** `style == "aps"`, `weak == false`, only `["I", "II"]` detected; zero list items promoted.
5. **Weak outline fixture** (`weak_outline_for_unstructured_text_still_indexes_equations_and_returns_unresolved_for_sections`):
   - Unstructured prose note with zero headings and one equation `(7)`.
   - **Result:** `weak == true`, `style == "weak"`, `sections` empty, `eq. (7)` still indexed and resolvable, `"section V.D.4"` returns `Unresolved`.

### WP11: Scope flow and section-scoped conventions catalog
- **Edited `crates/research-tools/src/investigation_conventions.rs`** (222 lines; `crates/research-tools/src/investigation.rs` left completely untouched and `conventions_def()` schema unchanged so zero `def_hash` drift):
  - Inspects the active marker (`investigation.mapping_requested` or `investigation.conventions_resumed`) from `history` and derives `coverage_scope` (`"section"` or `"paper"`). When neither marker specifies a scope (whole-paper default), `coverage_scope` is omitted so existing whole-paper payloads remain byte-identical.
  - Allows a subsequent batch after a finalized (`done: true`) batch **only** when that prior batch had `coverage_scope == "section"` **and** the active marker is `investigation.conventions_resumed` with `scope == "paper"`.
  - Preserves `prev.payload["coverage_scope"]` on identical re-sends of earlier batches so idempotency stays intact while modified re-sends still fail.
- **New `crates/desktop-server/src/investigation_scope.rs`** (268 lines) + **`crates/desktop-server/src/investigation_scope_tests.rs`** (390 lines):
  - `compute_scope`: resolves and SHA-256-verifies the primary source file via `WorkspaceSandbox`, parses verified PDF `page_lines` when applicable, runs `build_outline` + `resolve_scope`, clips resolved targets against any active PDF page selection, and formats the compact (~1.5 KB) outline + scoped reading guidance.
  - Helpers for `investigation.scope` event payloads, `latest_scope`, `conventions_resume_allows`, `conventions_expanding_to_paper`, `conventions_finish_done`, and `conventions_view_expanding`.
- **Minimal hooks in `crates/desktop-server/src/investigation.rs`** and **`crates/desktop-server/src/lib.rs`** (detailed under Merge notes below):
  - `prepare` mapping branch computes scope, attaches `scope` and `conventions_scope: "section"` to `mapping_requested` when resolved, writes `investigation.scope`, and appends the outline + scoped instructions.
  - `prepare` conventions branch allows resuming a `coverage_scope: "section"` done catalog and emits `investigation.conventions_resumed` with `scope: "paper"`.
  - `handle` (`operation == "conventions"`) uses `conventions_resume_allows` so clicking "Catalog all conventions" on a section-done catalog starts the expansion turn (HTTP 202) instead of returning HTTP 200 `"complete"`.
  - `finish` accepts a done section catalog on mapping turns, and requires `coverage_scope == "paper"` when finishing a paper-expansion conventions resume turn.
  - `view` exposes `"scope"` (for the current mapping request) and `"conventions_expanding"`.
- **New tests:**
  - `crates/research-tools/tests/investigation_scope.rs`: `batch_after_section_done_without_paper_marker_refused` (also tests upgrade under `scope: "paper"`, idempotent batch 1 re-send, and refusal after paper-scope finalization).
  - `crates/desktop-server/src/investigation_scope_tests.rs`: `finish_accepts_section_catalog`, `whole_paper_unchanged`, `catalog_all_after_section_done_allowed`.

### WP12: UI, Phase 3 (scope chip and section conventions labels)
- **New `crates/desktop-server/assets/investigation_scope.js`** (92 lines):
  - Pure `scopeChipData(scope)` formatting target labels, line ranges, and physical page ranges (returning `null` for unresolved/missing scope).
  - `createScopeChip(anchorEl)`: inserts `#investigation-scope-chip` after `#investigation-status` (so `index.html` stays untouched) and renders all untrusted text strictly via `textContent` and `setAttribute('title', ...)`.
- **Minimal edits:**
  - `crates/desktop-server/src/assets.rs`: serves `/assets/investigation_scope.js`.
  - `crates/desktop-server/assets/investigation.css`: adds `.investigation-scope-chip` and `.investigation-scope-chip-label` rules (+2 lines).
  - `crates/desktop-server/assets/investigation.js`: imports/wires `createScopeChip` (+6 lines) and polls while `inv.conventions_expanding === true` (+1 condition).
  - `crates/desktop-server/assets/investigation_conventions.js`: appends `" · section catalog"` in `conventionStatus` when `coverage_scope === 'section'` and keeps the resume button visible with label `"Catalog all conventions"` when a section catalog is done (+6 lines).
  - `crates/desktop-server/tests/investigation_origin.test.mjs`: adds a 1-line stub for `investigation_scope.js` so the existing origin test harness resolves the import without modifying any test assertions.
- **New tests:**
  - `crates/desktop-server/tests/investigation_scope.test.mjs` (235 lines, 3 test cases): verifies `scopeChipData`, `createScopeChip` (including XSS `<img onerror=...>` textContent safety and `innerHTML` trap), `"section catalog"` status labels, `"Catalog all conventions"` button visibility/click handler, and hiding after paper-scope completion.

---

## 2. Deviations from design and decisions taken unattended

1. **Submodule split in `research-tools` (`investigation_outline_plain.rs` and `investigation_outline_tex.rs`):**
   - To obey the 600-line source-file limit in `RULES.md` without cramping the heuristics, `investigation_outline.rs` delegates plain-text scanning and TeX scanning to two `#[path]` submodules (564, 553, and 206 lines respectively). The public API remains `research_tools::investigation_outline::{build_outline, format_outline_for_prompt, resolve_scope, ...}`.
2. **Kept `crates/research-tools/src/investigation.rs` untouched:**
   - Rather than altering `conventions::payload`'s signature to pass `marker` from `investigation.rs` (which is 772 lines at base and owned by `ux-rust` / `split-1`), `investigation_conventions.rs` reads the active marker directly from `history` (safe because `record_document` already gated the active marker before calling `conventions::payload`).
3. **Also updated `handle` (`operation == "conventions"`) and `finish` (`_paper_conventions_request`) in `desktop-server`:**
   - The design text highlighted the resume gate in `prepare` (`:464`). Inspecting the code showed that `handle` (`:689-693`) also short-circuited `done == true` to HTTP 200 `"complete"`, and `finish` (`:589-590`) would treat an empty paper-expansion resume as `"complete"` because the earlier section batch already had `done == true`. Both were updated via `investigation_scope.rs` helpers so clicking "Catalog all conventions" works end-to-end.
4. **Equation/figure scope context radius:**
   - Resolved equation, figure, and table targets expand to a deterministic ±20-line context window (`ITEM_CONTEXT_RADIUS = 20`, clipped to `1..=total_lines`) so the model sees surrounding derivation/prose without exceeding the 64-line anchor limit.
5. **PDF selected-page clipping:**
   - When the user selects an explicit PDF page range (`page_start..=page_end`) and a question target falls completely outside those pages, `compute_scope` marks the scope `unresolved` rather than instructing the model to cite out-of-range lines.
6. **`investigation.scope` recorded on both resolved and unresolved mapping turns:**
   - Recording `status: "unresolved"` on whole-paper mapping turns gives audit provenance and lets `split-2` readers distinguish "no scope resolved" from "not run", while `mapping_requested` receives `scope` and `conventions_scope: "section"` **only** when resolved (keeping whole-paper behavior unchanged).
7. **JS scope chip inserted dynamically after `#investigation-status`:**
   - Avoids editing `crates/desktop-server/assets/index.html` (owned by `ux-ui`) and matches `ux-ui`'s pattern for `#investigation-answer`.

---

## 3. Merge notes (for coordinating parallel briefs)

| File | Symbol / Location | What `split-3` changed | What other parallel briefs change here & how to merge |
|---|---|---|---|
| `crates/desktop-server/src/lib.rs` | Top module list (`mod investigation_scope;`) | Added 1 line `mod investigation_scope;`. | `ux-rust` P0 adds `mod investigation_view;`, `mod investigation_finish;`, `mod investigation_question;`, `mod investigation_system_prompt;`, `mod investigation_title;`. Merge trivially by keeping all `mod` lines. |
| `crates/desktop-server/src/investigation.rs` | `view()` (`:123-140`), `prepare()` (`:471-495`, `:555-583`), `finish()` (`:617-624`), `handle()` (`:717-722`) | Added small calls into `crate::investigation_scope::*` (no new helpers inside `investigation.rs`). Note: `investigation.rs` was already 811 lines at base; `ux-rust` P0 splits `view` and `finish` out to bring it under 600 lines. | **`ux-rust`**: moves `view` to `investigation_view.rs`, `finish` to `investigation_finish.rs`, replaces `prepare`'s routing chain with `investigation_question::scope` (adding `prepare_question` and setting `conventions_requested: false` on mapping). Keep our `latest_scope` / `conventions_view_expanding` in `investigation_view.rs`, our `conventions_finish_done` in `investigation_finish.rs`, our `conventions_resume_allows` / `conventions_expanding_to_paper` in `prepare` & `handle`, and run `compute_scope` on **both** `prepare_question` (so question-first turns get outline + line scope!) and mapping turns (where `conventions_scope: "section"` applies when conventions are requested). **`split-1`**: adds `route: PaperRoute` to `prepare`, strips pointer fields in `context_text`, lists `pending_items` in conventions resume prompt — orthogonal to our hooks. **`split-2`**: reads `investigation.scope` (or `mapping_requested["scope"]`) when building the default reader plan in `paper_read.rs`. |
| `crates/research-tools/src/lib.rs` | Top module list (`pub mod investigation_outline;`) | Added 1 line. | `split-1` (WP2 `paper_lines`) and `split-2` (WP7 `investigation_evidence`) add their own `pub mod` lines. Keep all. |
| `crates/research-tools/src/investigation_conventions.rs` | `payload()` (`:117-140`, `:190-200`) | Added `active_marker` / `derived_scope` check, `section_upgrade` exception on `done == true`, and `payload["coverage_scope"]`. Did not change `conventions_def()`. | `split-1` (WP1b) adds two-pass pointer verification, item exclusion, `rejected_items`, `next_batch_index`, and `pending_items`. Merge our `active_marker`/`derived_scope`/`section_upgrade` check into the prior-batch loop and our `coverage_scope` assignment right next to `payload["coverage_basis"] = json!("model_reported")`. |
| `crates/desktop-server/src/assets.rs` | `lookup_asset()` (`:103-106`) | Added `"/assets/investigation_scope.js"` arm. | `ux-ui` and `latex` add their new JS asset arms here. Keep all arms. |
| `crates/desktop-server/assets/investigation.js` | Imports (`:7`), `initInvestigation` (`:15`), `showInvestigationSession` (`:239,252`), `scheduleSummaryRefresh` (`:287`) | Added 6 lines to wire `createScopeChip(byId('investigation-status'))` and `inv.conventions_expanding === true` polling check. | `ux-ui` (main owner) heavily updates `investigation.js` (answer card, state gates, status timer). Re-apply the 4 tiny `scopeChip` lines (`import`, `init`, `reset`, `update`) and `inv.conventions_expanding` polling check on top of `ux-ui`'s `investigation.js`. |
| `crates/desktop-server/assets/investigation_conventions.js` | `conventionStatus()` (`:37-39`), `createConventionCatalog::update()` (`:123-127`) | Added `" · section catalog"` label and `"Catalog all conventions"` button when `catalog?.done && catalog.coverage_scope === 'section'`. | `split-1` (WP5) adds `"N citations pending"` display; `ux-ui` drops `inv.source_id` from `host.hidden` (`:128`). Orthogonal — merge both cleanly. |
| `crates/desktop-server/assets/investigation.css` | Line 26–27 | Added 2 lines for `.investigation-scope-chip`. | `ux-ui` edits line 8 (`#composer-input,#composer-send` rule) and adds answer card CSS. Disjoint lines. |
| `crates/desktop-server/tests/investigation_origin.test.mjs` | Line 119 | Added 1 stub entry for `'investigation_scope.js'`. | `ux-ui` may add stubs for its new `investigation.js` imports here; keep all stub entries. |

---

## 4. Test commands run and exact results

| Command | Result |
|---|---|
| `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` | **Pass** (all JS assets valid syntax, exit 0) |
| `node --test crates/desktop-server/tests/*.test.mjs` | **Pass: 66 passed, 0 failed, 0 skipped** (includes 3 new WP12 tests in `investigation_scope.test.mjs`) |
| `cargo test --workspace --no-fail-fast` | **Pass: 438 passed, 0 failed, 0 ignored** across 79 test binaries (includes 6 new WP10 tests in `research-tools/tests/investigation_outline.rs`, 1 new WP11 test in `research-tools/tests/investigation_scope.rs`, and 3 new WP11 tests in `desktop-server/src/investigation_scope_tests.rs`) |
| `cargo clippy -p research-tools --lib --test investigation_outline --test investigation_scope -- -D warnings` | **Pass** (exit 0, 0 warnings) |
| `cargo clippy -p research-tools -p desktop-server --all-targets -- -D warnings -A clippy::manual_unwrap_or -A unused_imports -A clippy::items_after_test_module` | **Pass** (exit 0, 0 warnings across all targets of modified crates) |
| `cargo clippy --workspace --all-targets -- -D warnings` | **Failed (exit 101)** solely due to 4 pre-existing diagnostics on Linux / `rustc 1.100.0` (listed below); 0 diagnostics in any code added or changed by `split-3` |

---

## 5. Pre-existing failures/diagnostics on Linux (with proof)

1. **`RE_SEARCH_PYTHON="$(command -v python3)" cargo test --workspace --no-fail-fast` hangs in `cli`:**
   - **Proof:** Ran on unmodified base commit `91753b86b4392814cbe0fe4dd213c9be1d15fc80` before making any edits. `crates/cli/tests/desktop_and_ext.rs::cli_research_workspace_read_fork_diff_export_and_verify` hung past 60 s and timed out the command.
   - **Cause:** On non-Windows hosts, `re-search run --route api` rejects a configured `RE_SEARCH_PYTHON` (tested by `main_tests::route_api_rejects_missing_key_and_non_windows_python`). The child process exits immediately with an error before contacting `ScriptedMock`, and `mock.finish()` (`crates/cli/tests/desktop_and_ext.rs:89-93,168`) blocks forever in `listener.accept()` waiting for 3 HTTP requests. Running `cargo test --workspace --no-fail-fast` without `RE_SEARCH_PYTHON` passes all 438 workspace tests (0 failures).
2. **Pre-existing `cargo clippy --workspace --all-targets -- -D warnings` failures on Linux / `rustc 1.100.0`:**
   - `crates/research-tools/tests/sandbox_and_fs.rs:6,9` — `unused_imports` (`is_symlink_or_junction`, `TEX_AUDIT` only referenced under `#[cfg(windows)]`; file untouched from base `91753b8`).
   - `crates/desktop-server/src/evidence.rs:737` — `clippy::manual_unwrap_or` (new lint in `rustc 1.100.0`; file untouched from base `91753b8`).
   - `crates/desktop-server/src/investigation.rs:384` (shifted to `:394` by our `view()` lines) — `clippy::items_after_test_module` (pre-existing inline `mod tests` before `prepare`, which `split-1`/`ux-rust` relocate).
   - `crates/agent/src/guardrails.rs:335` — `clippy::items_after_test_module` (file untouched from base `91753b8`).

---

## 6. What could not be compiled or run on Linux

- All `#[cfg(windows)]` code paths (AppContainer sandbox, Job Objects, DPAPI vault, WebView2 `desktop-app` runtime, Windows path/junction tests).
- Playwright / browser tests (`crates/desktop-server/tests/*_browser.py`, including `investigation_browser.py`), out of scope per `RULES.md` and run on Windows by the coordinator.

---

## 7. Bugs / issues noticed but not fixed (outside brief scope)

- `crates/cli/tests/desktop_and_ext.rs:89-93,168`: `ScriptedMock::finish()` blocks indefinitely if the spawned `re-search` process exits before consuming all scripted replies (triggered on Linux whenever `RE_SEARCH_PYTHON` is exported). Checking `out.status.success()` before `mock.finish()` or using a non-blocking/timeout listener would prevent the hang.
- `crates/desktop-server/src/evidence.rs:737-743`: `rustc 1.100.0`'s `clippy::manual_unwrap_or` fires on the status allowlist `match`, suggesting `.unwrap_or("result_unavailable")`. Note that blindly applying clippy's suggestion would drop the status-value allowlist filter and accept arbitrary status strings; this looks like a false-positive suggestion from `clippy::manual_unwrap_or` (or wants `filter(|s| matches!(...))`).
- `crates/research-tools/tests/sandbox_and_fs.rs:6,9`: `is_symlink_or_junction` and `TEX_AUDIT` are unused on non-Windows targets (causing `-D warnings` clippy failure on Linux).
