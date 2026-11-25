# Fix round report: `gemini/work/fixed`

- **Base commit (`origin/gemini/work/merged`):** `351b1287c9a54a700b0e33111b3891f195afd6d1`
- **Target branch:** `gemini/work/fixed`
- **Environment:** Linux (Ubuntu 24.04), `rustc 1.100.0`, `cargo 1.100.0`, `node v22.23.5`, Python 3.12 (`sympy`, `numpy`, `mpmath`, `playwright 1.65.2` + headless Chromium).
- **Subagent execution:** **Worked.** Every implementation batch (Step 1 plus all four audit lenses) was delegated via `invoke_subagent` (`TypeName: "self"`, `Model: "inherit"`, inheriting the Gemini 4 Neon (High) remote-agent model). Subagents ran concurrently only on strictly disjoint file sets (17 subagents across 5 batches; never two subagents on the same file). Every subagent returned an uncommitted diff that the orchestrator reviewed for scope, escaping, honesty invariants, and the 600-line ceiling before committing and running full workspace checks.
- **Audit branches handled:** All **4 of 4** arrived and were merged with `--no-ff` (0 missing):
  1. `origin/gemini/work/audit-honesty` — wave 1 (`8b65860`, merge `e4a71d9`) and Carbon pass (`fa369d5`, merge `588fdd3`)
  2. `origin/gemini/work/audit-consistency` (`e274b92`, merge `af7a92b`)
  3. `origin/gemini/work/audit-flow` (`a3024eb`, merge `6b04c58`)
  4. `origin/gemini/work/audit-ui` (`61d9ac6`, merge `467292c`)

---

## 1. Step 1: Windows browser failure (`investigation_browser.py:365`)

### Root cause (test bug, not app bug)
Reproduced locally on Linux by installing Playwright + Chromium and running `crates/desktop-server/tests/workbench_browser.py --binary target/debug/re-search` (7/8 passed, `investigation` failed at the exact same line `365` with the ARIA snapshot showing `status: Choose a primary paper.`).

Three test-harness races in `check_question_first_investigation` (`crates/desktop-server/tests/investigation_browser.py`, written by `ux-ui`):
1. **File input cleared by form reset:** After clicking `"Investigate a paper"`, `page.get_by_label("Primary paper", exact=True).set_input_files(...)` ran immediately without waiting for `#investigation-setup` to become visible (Playwright `set_input_files` bypasses visibility checks). The async `createAndSelectSession({ mode: 'investigation' })` then completed and called `showInvestigationSession`, which ran `byId('investigation-setup').reset()` and cleared `#investigation-file`. Clicking `"Find main results"` hit `Choose a primary paper.` instead of `callbacks.requestOptions()` / `#settings-dialog`.
2. **Shared data directory between installs:** Both `open_install` blocks reused the same `root`, so the investigation session created in the no-models block was auto-selected at startup of the second block, detaching the empty-hero `"Investigate a paper"` button.
3. **Unsynchronized `POST /messages` and teardown route fetch:** Assertions on `sends[0..2]` ran immediately after `.click()` without awaiting the intercepted `/messages` response, and `open_install`'s `server.kill()` before `context.close()` could race with an in-flight `GET /api/sessions/<id>` inside `session_view` (`route.fetch()` -> `ECONNRESET`).

### Fix & verification
- **`8bf50bb`**: Waited for `#investigation-setup` visibility before `set_input_files` and `listSessions()`, isolated `no-models` and `question` data directories, waited for `#composer-model` selection matching `check_investigation`, and wrapped the three submit clicks in `with page.expect_response("**/" + sid + "/messages"):`.
- **`26a6d7a`**: Added `page.unroute_all(behavior="ignoreErrors")` after the final assertion in `check_question_first_investigation` before `open_install` terminates the server.
- **Linux Playwright run:** `workbench_browser.py` ran cleanly on Linux with **8/8 PASS** (including `[7/8] PASS investigation`).

---

## 2. Finding-by-finding disposition table (62 findings across 4 lenses)

### 2.1 `audit-honesty` (21 findings: F1–F15 wave 1, F16–F21 Carbon pass)

| Lens | # | Severity | Verdict | Reason / Resolution | Commit |
|---|---|---|---|---|---|
| `audit-honesty` | F1 | `critical` | **confirmed** | `collect_received_ranges` ignored `tool.request` `caller` and accepted unmatched `tool.result`s, crediting worker/UI reads to Orchestrator answers. Required `call_id` match and main-agent caller allowlist (`agent:`, `claude-code-cli:`, `antigravity:`). | `ec79e63` |
| `audit-honesty` | F2 | `critical` | **confirmed** | `verify_pointer` used `s.min(final_start - 3)`, letting label lookback scan 60 narrowed-away lines. Bounded strictly to `final_start.saturating_sub(LABEL_ABOVE_RADIUS).max(1)`. | `bcd2329` |
| `audit-honesty` | F3 | `critical` | **confirmed** | `collect_received_ranges` marked `line_truncated` `read_paper_lines`, 0-char past-EOF `fs_read`, and partial mid-line slices as `read: true`. Excluded them in `investigation_cites.rs`. | `ec79e63` |
| `audit-honesty` | F4 | `major` | **confirmed** | `window_around` could drop the tail of quotes spanning >3,200 bytes, and `resolve_evidence` checked `label` against windowed `derived` instead of `full_lines`. Rejected >3,200-byte quote spans (`quote_span_too_wide`), snapped char boundaries around quote, and checked `full_lines.contains(label)`. | `bcd2329` |
| `audit-honesty` | F5 | `major` | **confirmed** | Later conventions batches could finalize `done: true, coverage: "paper"` while `pending_items` remained, wipe prior `limitations`, and lock out UI/API resume. Tracked pending items across batches, merged distinct limitations, and allowed resume whenever `pending_items` is non-empty. | `6233c82`, `f7f87cc` |
| `audit-honesty` | F6 | `major` | **confirmed** | `clip_to_selected_pages` and `paper_reader::plan_slices` did not clamp partially overlapping section ranges or `front_matter` to selected PDF page line bounds. Clamped targets, ranges, `front_matter`, and reader slices to `first_page..=last_page` line bounds. | `f7f87cc` |
| `audit-honesty` | F7 | `major` | **confirmed** | Narrowing/trimming followed by `label_extended` back to `[s, e]` emitted `cited_range == [start_line, end_line]` and rendered `"lines corrected by the app (cited 10–13)"`. Cleared `cited_range`/`range_basis` when unchanged and guarded UI note in `investigation_map.js`. | `bcd2329`, `58ba23f` |
| `audit-honesty` | F8 | `minor` | **confirmed** | Request-body `paper_reader` override auto-enabled readers and sent paper text to an unassigned provider (violating Decision #4). Removed `req.get("paper_reader")` override from `paper_reader.rs` and `agent_models.rs`. | `f7f87cc` |
| `audit-honesty` | F9 | `minor` | **confirmed** | `source_total_lines` and `paper_reader::prepare_for_turn` read disk files without checking `files_sha256` (and raw `pdf_page_lines`). Added SHA-256 verification and `verified_page_lines` fail-closed checks in both. | `ec79e63`, `f7f87cc` |
| `audit-honesty` | F10 | `minor` | **confirmed** | Answer citations >513 lines were marked `read: true` but rejected with HTTP 400 by `routes_tools.rs:163`. Capped `read: true` citations to `end - start <= 512` in `investigation_cites.rs`. | `ec79e63` |
| `audit-honesty` | F11 | `minor` | **confirmed** | `investigation_map.js` `mathSpans` paired currency `$5` with later `$`, widening `<mark>` across unquoted prose. Added currency `$` guard in `mathSpans`. | `58ba23f` |
| `audit-honesty` | F12 | `minor` | **confirmed** | `anchor_schema()` advertised `quote.minLength: 12`, contradicting 3-char whole-line quotes (`E = mc^2`) in `verify_pointer`, `resolve_evidence`, and `note_evidence_def`. Aligned `anchor_schema` to `MIN_WHOLE_LINE_NON_SPACE` (`3`). | `bcd2329` |
| `audit-honesty` | F13 | `nit` | **confirmed** | Trivial & safe: guarded primary PDF `page_for_line` lookup in `paper_read.rs::collect_evidence_rows` with `rec.source_id == primary_source_id`. | `58ba23f` |
| `audit-honesty` | F14 | `nit` | **confirmed** | Trivial & safe: formatted singular `"1 item"` vs plural `"N items"` and kept batch limitations `<= 12` in `investigation_conventions.rs`. | `6233c82` |
| `audit-honesty` | F15 | `nit` | **confirmed** | Trivial & safe: avoided duplicating `(lines a-b)` in `investigation_scope.rs` instruction headings when `label` already contains the range. | `f7f87cc` |
| `audit-honesty` | F16 | `critical` | **confirmed** | `collect_received_ranges` did not compare tool result `file_sha256` against `source.ingested.payload["files_sha256"]`, and `finish_question` used `read_session`. Switched to `read_verified_session` and required `file_sha256 == expected_sha` on all credited reads/totals. | `b02d567` |
| `audit-honesty` | F17 | `critical` | **confirmed** | `answer_cites_for_session` ignored pinned PDF `page_start..=page_end`, marking unselected pages `read: true`. Loaded `verified_page_lines` fail-closed and marked out-of-page citations `read: false`. | `b02d567` |
| `audit-honesty` | F18 | `major` | **confirmed** | `is_bare_section_name` missed `"Appendix A"`, `"Section 1. Introduction"`, `"Sec. II. Results"`, `"§2 Methods"`, `"II.A Discussion"`, and `excerpt_window` re-searched from byte 0 after `label_extended`. Extended bare section detection and tracked exact matched byte offsets in `FoldMatch`. | `cc00ce4` |
| `audit-honesty` | F19 | `major` | **confirmed** | `delegate_read_paper` counted secondary-source and `line_truncated: true` `read_paper_lines` calls in `lines_read_pct`. Restricted `read_ranges` credit to complete primary-path reads. | `e535d40` |
| `audit-honesty` | F20 | `major` | **confirmed** | `evidence.rs` rejected `read_paper_lines` as `unsupported_reference` and omitted it from `linked_session_sources`. Resolved together with `audit-consistency` C-01. | `c77778b` |
| `audit-honesty` | F21 | `minor` | **confirmed** | `investigation_map.js` `mathSpans` ignored `\begin{env}...\end{env}`, splitting equation environments across `<mark>`. Added 30 `MATH_ENVS` span handling while keeping file at 594 lines (`<= 600`). | `72e574d` |

### 2.2 `audit-consistency` (12 findings: C-01–C-12)

| Lens | # | Severity | Verdict | Reason / Resolution | Commit |
|---|---|---|---|---|---|
| `audit-consistency` | C-01 | `major` | **confirmed** | Wired `read_paper_lines` across `Router::call` (`tool_call_ref`), `evidence.rs` (`source_read` + `linked_session_sources`), `verifiers.rs` (`<research_sources>`), `delegate.rs` (`literature_worker_tools` + `read_only` profile), and Research/Investigation/CLI reading prompts. | `c77778b`, `2e27ff7` |
| `audit-consistency` | C-02 | `major` | **confirmed** | Fixed section-scoped conventions under D4: first click on a section-scoped map builds `"section"` scope (not `"paper"` expansion of `null`); incomplete section catalogs stay `"section"`; omitted conventions clause from D4 `mapping_instructions_suffix` (updated stale pre-D4 assertion in `investigation_scope_tests.rs:155`). | `45aa454` |
| `audit-consistency` | C-03 | `major` | **confirmed** | PDF `selected_pages` clipping + duplicate `(lines a-b)` in prompts. Already resolved by `audit-honesty` F6 + F15 (proof test passed immediately on merge). | `f7f87cc` |
| `audit-consistency` | C-04 | `major` | **confirmed** | Added `DelegationConfig::is_investigation` to omit Research Proof-DAG/`final_answer` lead-in and `reference_lookup` instructions on Investigation turns, gated compute/lean/explore/custom workers on question turns, and updated `INVESTIGATION_SYSTEM_PROMPT` item 5 to accurately state `$...$`, `\(...\)`, `$$...$$`, `\[...\]` support. | `2e27ff7` |
| `audit-consistency` | C-05 | `major` | **confirmed** | Stripped client-supplied `_paper_scope` in `investigation.rs`; recognized `scope == "section"` on `conventions_resumed` (and inherited parent map scope ranges) in `paper_reader.rs`; replaced `#paper-readers-enabled` 2-state checkbox with 3-state `<select>` (`auto`/`true`/`false`) in `subagents.js`. | `45aa454`, `ad5109c` |
| `audit-consistency` | C-06 | `major` | **confirmed** | Implemented question-only Investigation handoff to linked Research (Design 02 P1b) in `investigation.rs::open_research` and `export/src/fork.rs::research_from_paper`, verifying `answer_seq`/`answer_hash` and primary source receipt and returning HTTP 201. | `45aa454` |
| `audit-consistency` | C-07 | `minor` | **rejected** | Proof test (`audit_consistency_ui.test.mjs:223-245`) hardcodes a resolved `scope` object and asserts `scopeChipData(resolved_scope) === null`, which cannot pass without breaking all resolved scope chips. Moreover, Design 01 §2.6 / WP11–WP12 tracks map reading scope separately from conventions catalog coverage (`coverage_scope: "paper"` updates the conventions summary while the map remains section-scoped). Deleted the broken audit test; no production change. | `ad5109c` |
| `audit-consistency` | C-08 | `minor` | **confirmed** | Recorded `"status": "stopped"` for user-stopped Investigation action turns in `investigation_finish.rs`, labeled claim-check turns `"Checking selected claim · m:ss"`, and surfaced stopped non-question turns on mapped papers in `investigation_state.js`. | `45aa454`, `ad5109c` |
| `audit-consistency` | C-09 | `minor` | **confirmed** | Normalized OpenAI (`prompt_tokens`/`completion_tokens`) and Gemini (`promptTokenCount`/`candidatesTokenCount`) Orchestrator usage in `session_summary.rs`. | `c77778b` |
| `audit-consistency` | C-10 | `minor` | **confirmed** | Updated `source_links.js::sessionSourcesUsed` to credit read-verified citations in `investigation.answer` / `session.investigation.answers`. | `ad5109c` |
| `audit-consistency` | C-11 | `minor` | **confirmed** | Updated `docs/reference.md`, `README.md`, `THREAT-MODEL.md`, and `assets/guide.js` to match question-first Investigation, `paper_reader`, `read_paper_lines`, `note_evidence`, and split `math_*.js`. | `ad5109c` |
| `audit-consistency` | C-12 | `nit` | **rejected** | Orphaned `/vendor/katex/auto-render.min.js` is explicitly deferred to WP6 by `README.md` Decision #8 (`research_display_browser.py:24` still checks it). Left untouched. | — |

### 2.3 `audit-flow` (11 findings: F1–F11)

| Lens | # | Severity | Verdict | Reason / Resolution | Commit |
|---|---|---|---|---|---|
| `audit-flow` | F1 | `major` | **confirmed** | Worker reads & 0-char past-EOF `fs_read` marking answer cites `read: true`. Already resolved by `audit-honesty` F1 + F3 (test passed immediately on merge). | `ec79e63` |
| `audit-flow` | F2 | `major` | **confirmed** | Investigation `session.start` Research delegation prompt & workers resolved by C-04 (`2e27ff7`); also skipped Research `search_formal_library`/`sympy_eval` prompt prefixes on non-recheck Investigation CLI turns in `cli_turn.rs`. Narrowed overbroad `audit_flow_turns.rs:312` `!contains("ledger_append")` (which collided with Design 02 D3's `INVESTIGATION_SYSTEM_PROMPT` Rule 4). | `2e27ff7`, `824e7c4` |
| `audit-flow` | F3 | `major` | **confirmed** | Parts 1–3 (D4 map prompt, null catalog section scope, incomplete section catalog) resolved by C-02 (`45aa454`); part 4 fixed by gating `conventions_view_expanding` on `running == true` and absence of turn termination after `conventions_resumed`. | `45aa454`, `824e7c4` |
| `audit-flow` | F4 | `major` | **confirmed** | Part 1 (PDF page clipping) resolved by `audit-honesty` F6 (`f7f87cc`); part 2 fixed via `plan_slices_for_ranges` in `paper_reader.rs` so disjoint `scope.ranges` do not span unscoped gaps across the paper. | `f7f87cc`, `7c5be1e` |
| `audit-flow` | F5 | `major` | **confirmed** | Passed resolved `reader_provider` (not Orchestrator `primary_provider`) to `effective_max_parallel`; required configured non-fallback reader credentials before auto-enabling readers under `paper_readers_enabled: None`; capped explicit `sections` at `MAX_PAPER_READER_SLICES = 8` in `paper_read.rs`. | `7c5be1e`, `581ff7f` |
| `audit-flow` | F6 | `major` | **confirmed** | Skipped packed numbered lists and `ends_with_dot` items in `assemble_aps` so `V.D.2..V.D.4` are preserved; fixed `SEC_RE` for lowercase/mixed-separator APS refs (`sec. v d 4`, `section V.D 4`); scanned TeX math blocks through `\end{...}` before assigning `eq_num` vs `\tag`. | `dda5830` |
| `audit-flow` | F7 | `minor` | **confirmed** | Updated `looks_like_tex` in `investigation_outline_tex.rs` to recognize `\documentclass` / `\begin{document}` and math/caption environments for PRL-style `.tex` letters. | `dda5830` |
| `audit-flow` | F8 | `minor` | **confirmed** | Computed `any_ok` from this call's reader outcomes/pointers (not prior `marker_seq` evidence) and removed `"focus"` from top-level schema `required` (kept runtime check for `mode == "read"`). | `581ff7f` |
| `audit-flow` | F9 | `minor` | **confirmed** | Updated `investigation_view.rs::mark_last_running` so a subsequent Investigation turn request closes an earlier crashed `"interrupted"` row instead of resurrecting it as `"running"`. | `824e7c4` |
| `audit-flow` | F10 | `minor` | **confirmed** | Unproven summary of question-only handoff and `read_paper_lines` in question prompts; already resolved by `audit-consistency` C-06 and C-01. | `45aa454`, `2e27ff7` |
| `audit-flow` | F11 | `nit` | **confirmed** | Duplicated `(lines a-b)` in scope instructions; already resolved by `audit-honesty` F15. | `f7f87cc` |

### 2.4 `audit-ui` (18 findings: F1–F18)

| Lens | # | Severity | Verdict | Reason / Resolution | Commit |
|---|---|---|---|---|---|
| `audit-ui` | F1 | `critical` | **confirmed** | Completed unclosed mid-line `$$` / `\[` openers in `para` before treating a line-start closer as a new block opener in `markdown.js`. | `ad18a95` |
| `audit-ui` | F2 | `critical` | **confirmed** | Rewrote `repairTextCmd` in `math_katex.js` to track nested `$...$` / `\(...\)` inside `\text{...}` so exponents/subscripts inside nested math are never escaped. | `a701a6b` |
| `audit-ui` | F3 | `critical` | **confirmed** | Fixed `harvestPreambleMacros` in `tex_preview.js` to preserve `[argc][default]`, `\providecommand`, `\DeclareMathOperator(*)`, and `\def` parameters, desugaring optional-arg `\newcommand` via `\@ifnextchar[` and replaying declarations through KaTeX into `docState`. | `a701a6b` |
| `audit-ui` | F4 | `critical` | **confirmed** | Pushed non-display trailing text after a mid-line display closer directly onto `para` (while preserving `$$$$` back-to-back displays) so leading `>`, `-`, `#` stay paragraph text. | `ad18a95` |
| `audit-ui` | F5 | `major` | **confirmed** | Added `stripTexComments` in `math_scan.js` and skipped `%...\n` comments in `findEnvRowSpans` and `numberEquations` so trailing `%` comments never swallow `\tag{n}` or `\\`. | `bf0a640` |
| `audit-ui` | F6 | `major` | **confirmed** | Split multi-equation `\begin{subequations}` blocks into distinct numbered displays with separate labels/refs in `math_scan.js` and `math_equations.js` (keeping single-env `subequations` unchanged). | `bf0a640` |
| `audit-ui` | F7 | `major` | **confirmed** | Replaced `[^{}]*` `\tag` regexes with balanced-brace parsers in `math_equations.js` and `math_katex.js::stripTagCommands`, wrapping bare math-only font commands in text-mode `\tag` bodies for KaTeX. | `a701a6b`, `bf0a640` |
| `audit-ui` | F8 | `major` | **confirmed** | Harvested earlier collapsed answers' macros chronologically in `investigation_answer.js` before rendering the newest answer, and called `resetMathMacros()` on blank reset and full stream re-renders in `app.js`. | `c5b7158` |
| `audit-ui` | F9 | `major` | **confirmed** | Honored explicit boolean `sess.approvalWaiting` in `investigation_state.js` and cleared stale `session.pending_approvals` on `research:approvals` `{ waiting: false }` in `investigation.js`. | `c5b7158` |
| `audit-ui` | F10 | `major` | **confirmed** | `\begin{equation}` quote widening in `investigation_map.js`. Already resolved by `audit-honesty` F21 (test passed immediately on merge). | `72e574d` |
| `audit-ui` | F11 | `major` | **confirmed** | Included `inv.conventions_expanding` in `investigation_conventions.js` signature, appended `' · reading whole paper…'` while running, and surfaced `inv.conventions_error` when `sectionDone` is true. | `c5b7158` |
| `audit-ui` | F12 | `minor` | **confirmed** | Capped clickable `.line-cite` spans to `end - start <= 512` in `paper_cites.js` so wider ranges stay plain text and never trigger HTTP 400. | `c5b7158` |
| `audit-ui` | F13 | `minor` | **confirmed** | Inserted `#investigation-answer` before `createScopeChip(byId('investigation-status'))` in `investigation.js` so DOM order is `investigation-status -> investigation-scope-chip -> investigation-answer`. | `c5b7158` |
| `audit-ui` | F14 | `minor` | **confirmed** | Replaced regex `stripComments` in `markdown.js` with a linear scanner preserving HTML comments inside blockquote fences, `>= 4`-space list fences, and multi-backtick code spans. | `ad18a95` |
| `audit-ui` | F15 | `minor` | **confirmed** | Switched `isMacroDefined` probe in `math_katex.js` to `\newcommand{\name}{}` (detecting text-mode built-ins `\ae`, `\oe`, `\ss`, `\o`) and cached the post-render canonicalized key for Pass-2 streaming cache hits. | `a701a6b` |
| `audit-ui` | F16 | `minor` | **confirmed** | Distinguished `mappingStopped` from `otherTurnStopped` in `investigation_state.js` so stopping a later conventions/branch/check turn never overwrites an earlier `mapping_error` while still showing `"Stopped..."` on mapped papers with no `mapping_error` (preserving `audit-consistency` C-08). | `c5b7158` |
| `audit-ui` | F17 | `minor` | **confirmed** | Added `.investigation-inner .rich-prose p { white-space: normal; }` in `investigation.css` so `<br>\n` hard breaks in Answer cards do not double. | `ad18a95` |
| `audit-ui` | F18 | `nit` | **confirmed** | Trivial & safe: removed dead `threw` branch in `math_katex.js` and skipped `.katex, .katex-error, .math-error` ancestors in `math.js::renderMathInContainer`. | `a701a6b` |

---

## 3. Decisions taken unattended

1. **Test edits (recorded per `fix.md`):**
   - `crates/desktop-server/tests/investigation_browser.py` (`8bf50bb`, `26a6d7a`): fixed Step 1 test-harness races without weakening any assertion.
   - `crates/desktop-server/src/investigation_scope_tests.rs:155` (`45aa454`): updated a stale pre-D4 assertion written in parallel by `split-3` (`assert!(ctx.contains("record conventions for the scope plus global notation"))`) to `assert!(!ctx.contains("record conventions for the scope"))` so it enforces UX Decision #1 (D4) and `audit-consistency` C-02 / `audit-flow` F3.
   - `crates/desktop-server/tests/audit_consistency_ui.test.mjs` (`ad5109c`): deleted only the rejected C-07 test (`lines 223–245`) because it hardcoded a resolved scope object and asserted `scopeChipData(resolved_scope) === null`.
   - `crates/desktop-server/tests/audit_flow_turns.rs:312` (`824e7c4`): narrowed `!frozen_sys.contains("ledger_append")` (which accidentally matched Design 02 D3's own `INVESTIGATION_SYSTEM_PROMPT` Rule 4 `"Use ledger_append only when a claim-check turn runs an explicit calculation"`) to `!frozen_sys.contains("final_answer") && !frozen_sys.contains("`ledger_append` with `entry_type") && !frozen_sys.contains("delegate_compute") && !frozen_sys.contains("delegate_lean") && !frozen_sys.contains("delegate_explore")`.
2. **Coexistence of `audit-consistency` C-08 and `audit-ui` F16:** Prioritized `mappingStopped` before `mapping_error` and `otherTurnStopped` after `mapping_error` in `investigation_state.js`, satisfying both findings simultaneously.
3. **Preserving core invariants & 600-line discipline:** Zero edits to `Cargo.toml`, `Cargo.lock`, `crates/policy`, or `crates/ledger`. Moved `investigation_cites.rs` unit tests into `investigation_cites_tests.rs` so `investigation_cites.rs` stays at 472 lines. Every newly created or sub-600 modified production file remains `<= 600` lines (`investigation_anchor.rs`: 597, `investigation_map.js`: 594, `delegate.rs`: 594, `paper_reader.rs`: 590, `turn_runner.rs`: 586, `investigation_outline_plain.rs`: 582, `investigation.rs`: 579, `paper_read.rs`: 557). The only touched production files > 600 lines were already > 600 lines before this branch (or exempted `app.js`): `app.js` (935, Decision #6), `cli_turn.rs` (628, was 626), `verifiers.rs` (878, was 867), `evidence.rs` (1057, was 1020).
4. **No new project dependencies:** Installed only Playwright/Chromium check on the VM; no manifest changes.

---

## 4. Final verification commands and exact counts

| Command | Result | Exact counts |
|---|---|---|
| `cargo clippy --workspace --all-targets -- -D warnings -A clippy::manual_unwrap_or -A clippy::items_after_test_module -A unused_imports` | **PASS** | **0 warnings, 0 errors** across all 13 crates |
| `RE_SEARCH_PYTHON="$(command -v python3)" cargo test --workspace --no-fail-fast -- --skip cli_research_workspace_read_fork_diff_export_and_verify` | **PASS** | **94 suites: 591 passed, 0 failed, 0 ignored, 1 skipped** |
| `env -u RE_SEARCH_PYTHON cargo test -p cli --test desktop_and_ext cli_research_workspace_read_fork_diff_export_and_verify` | **PASS** | **1 passed, 0 failed** (**592/592** total Rust tests pass) |
| `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` | **PASS** | **32/32** JS assets syntax-checked, exit 0 |
| `node --test crates/desktop-server/tests/*.test.mjs` | **PASS** | **140 passed, 0 failed, 0 skipped** |
| `/opt/pyenv/shims/python3 crates/desktop-server/tests/workbench_browser.py --binary target/debug/re-search` | **PASS** | **8/8 browser suites passed** (`workbench` 1440/1024/390, `research display`, `pdf preview`, `recovery example`, `investigation`, `session folders`) |

---

## 5. What is left failing (0 new failures; pre-existing Linux-only diagnostics)

- **New failures:** **None (`0`).**
- **Pre-existing Linux diagnostics (verified on base `351b1287` and `origin/main`):**
  1. `cargo clippy --workspace --all-targets -- -D warnings` without the 3 pre-existing Linux allow flags fails only on the 4 pre-existing untouched diagnostics (`crates/desktop-server/src/evidence.rs:744` `clippy::manual_unwrap_or`, `crates/agent/src/guardrails.rs:335` `clippy::items_after_test_module`, `crates/research-tools/tests/sandbox_and_fs.rs:6` `unused_imports`, `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` `unused_imports`).
  2. `crates/cli/tests/desktop_and_ext.rs::cli_research_workspace_read_fork_diff_export_and_verify` hangs on Linux when `RE_SEARCH_PYTHON` is exported globally; passes standalone (`1 passed, 0 failed`) without `RE_SEARCH_PYTHON`.
  3. Unproven narrow torn-read/pre-start stop race notes in `audit-flow` F9 items 2–3 (`turn_runner.rs` teardown gap and immediate pre-loop `POST /stop`) and `audit-ui` F13 item 2 (follow-up question drift on `inv.question` in `investigation_view.rs:120`) were not reproduced by failing tests and were left untouched as low-risk follow-ups.

---

## 6. What only Windows can check

- All `#[cfg(windows)]` code and tests: `crates/worker` (`AppContainerSandbox`, `JobObjectSandbox`, `python_worker`), `crates/cli` (`contained_router`, `init_abort`), `crates/desktop-server` (`claude_cli_route`, `chatgpt_route`, Windows process-tree paths), `crates/desktop-app` (WebView2 shell & CSP host), and `crates/chatgpt-auth` (DPAPI vault and `vault_lock_multiprocess`).
- Windows CI `workbench-checks.yml` with the staged Windows math runtime (`packaging/windows/stage_runtime.py`) and Chromium `--only-shell`.
- Visual WebView2 LaTeX parity checks in light and dark themes (`03-latex-kandinsky-parity.md` §4), plus live provider/CLI turns and real-paper timings.
