# Report: `exp-x15-chat` — Explore Conversation Turns & Multi-Paper Line Citations (Package X15)

- **Brief:** `docs/design/modes-2026-11/agents/exp-x15-chat.md`
- **Base commit (branch start):** `ffee92f4bb0372a362b2e2781bb3c8da13b7cdaa` (`origin/gemini/work/fixed` at start, after PR #79 `exp-x9-x10-x11-read-verdict`)
- **Merged `origin/gemini/work/fixed` commit:** `7b1c1910f63ec580049c152dca8a8baf0f7ec608` (merged `--no-ff` in `4e29922`, bringing PR #80 `f8-set-c-harness`)
- **Push branch:** `gemini/modes/exp-x15-chat`
- **Governing design:** `00-overview.md` (§2.2, §2.4, §2.5, §3.1 row 12, §3.2 row 20, §3.3, §8.2 D1); `explore.md` (§3, §5.2, §6 C1/C2/C7/P3/P5/P6/P7/S2/S8, §8 items 3–4, §9 X15); `agents/RULES.md`.

---

## 1. Commits (Separate Per Checkpoint & Fix — Never Squashed)

1. `5c3eddf` — **Checkpoint 1a:** Route `explore/1` through `build_idea_router` in `crates/desktop-server/src/routers.rs::build_desktop_router_for_profile`.
2. `d913775` — **Checkpoint 1b:** Add Explore Conversation system prompt (`idea_system_prompt.rs`), `explore/1` turn runner (`idea_chat.rs`), F4 multi-paper line-citation checker (`idea_chat_cites.rs`), crate exports (`lib.rs`), and integration tests (`tests/idea_chat.rs`).
3. `95a07af` — **Checkpoint 2a:** Wire `Mode::Explore` preconditions, Investigate-hook exclusion, and dispatch in `crates/desktop-server/src/turn_runner.rs` (`590`L).
4. `fabdd7a` — **Checkpoint 2b:** Enable `DESKTOP_MODE_EXPLORE_ENABLED = true` (with `DESKTOP_MODE_EXPLORE_CLI_ENABLED = false`) in `session_mode.rs`, and update Explore-enabled assertions in `tests/mode_shell.rs` and `tests/handoff_routes.rs`.
5. `c5921ec` — **Checkpoint 2c (isolated 1-line seam commit):** Keep `cli_turn_setup.rs:272` Explore CLI guard behind `DESKTOP_MODE_EXPLORE_CLI_ENABLED` until `exp-x18-cli` lands (see §3.1 & §5).
6. `6761639` — **Checkpoint 2d:** Add multi-paper chat citation UI (`assets/idea_chat_ui.js`), register `/assets/idea_chat_ui.js` in `assets.rs`, and add Node DOM/XSS/S8 tests (`tests/idea_chat_ui.test.mjs`).
7. `cd2ae98` — **Independent Carbon Audit Fixes & HTTP E2E Tests:** Fix all findings from the independent Carbon (`pro`) audit (B2, M1–M3, M5; B1/M4 already landed in `6761639`), add regression tests in `tests/idea_chat.rs`, and add full HTTP `POST /api/sessions/{id}/messages` E2E test suite (`tests/idea_chat_http.rs`).
8. `4e29922` — **Base Sync:** `Merge origin/gemini/work/fixed (7b1c191) into work/exp-x15-chat` (`--no-ff`, zero conflicts).
9. *(Final commit)* — This report (`docs/design/modes-2026-11/reports/exp-x15-chat.md`).

---

## 2. Package X15 Inventory & Line Counts (`<= 600` Lines Every File)

Measured on `4e29922` (`python3 scripts/check_file_length.py --base origin/gemini/work/fixed` passed):

| File | Status | Lines | Purpose |
| --- | --- | ---: | --- |
| `crates/desktop-server/src/idea_system_prompt.rs` | New | 334 | Explore Conversation system prompt (`EXPLORE_CHAT_SYSTEM_PROMPT`) wrapping latest idea statement, selected map node, and latest ingested paper manifest as quoted untrusted data (Rule C7); enforces `[paper_key: Lstart-end]` citation rules and forbids novelty/verification prose (Rule S8). |
| `crates/desktop-server/src/idea_chat_cites.rs` | New | 520 | Multi-paper citation parser (`[paper_key: Lstart-end]`, `[source_id: Lstart-end]`, versioned arXiv canonicalization, en-dash, semicolon groups) and strict F4 ledger checker (`lines_sent_with_ledger`) scoped to the Conversation caller from the first `session.start` at/after latest `idea.started`; multi-source resolution, multi-page PDF OCR detection (`scan_text_layer` / `\x0c`), fail-closed `idea.chat_citations` event persistence. |
| `crates/desktop-server/src/idea_chat.rs` | New | 336 | Explore Conversation turn runner (`run_desktop_explore_turn`) and synchronous pre-turn validator (`validate_explore_turn_preconditions`: requires `idea.stated`, F9 `model_calls` budget, refuses CLI providers across all config fallback layers until X18); runs on `build_desktop_router_for_profile(..., PROFILE_EXPLORE_V1)` with `research_policy()` (Rule P7), F1 `resume_profile`, F9 `run_budget` & `CancelToken`, and emits `idea.chat_citations` over SSE. |
| `crates/desktop-server/src/routers.rs` | Modified | 260 | Routes `PROFILE_EXPLORE_V1` in `build_desktop_router_for_profile` through `build_idea_router` (`explore/1`: `idea_search`, `idea_fetch_papers`, `search_paper_passages`, `read_paper_lines`; `0` code/write tools). |
| `crates/desktop-server/src/turn_runner.rs` | Modified | 590 | Calls `validate_explore_turn_preconditions` for `Mode::Explore`, strictly excludes `Mode::Explore` from `investigation::prepare`, `investigation_finish::finish_with_cancel`, `investigation::turn_context`, and `investigation_system_prompt::base_system_prompt`, and dispatches `Mode::Explore` to `idea_chat::run_desktop_explore_turn`. |
| `crates/desktop-server/src/session_mode.rs` | Modified | 283 | Flips `DESKTOP_MODE_EXPLORE_ENABLED = true`, adds `DESKTOP_MODE_EXPLORE_CLI_ENABLED = false` for X18, and updates unit tests. |
| `crates/desktop-server/src/cli_turn_setup.rs` | Modified | 584 | 1-line edit at line 272 switching Explore CLI guard from `!DESKTOP_MODE_EXPLORE_ENABLED` to `!DESKTOP_MODE_EXPLORE_CLI_ENABLED` (see §3.1). |
| `crates/desktop-server/src/lib.rs` | Modified | 282 | Exports `pub mod idea_chat;`, `pub mod idea_chat_cites;`, `pub mod idea_system_prompt;`. |
| `crates/desktop-server/src/assets.rs` | Modified | 542 | Registers static asset route `/assets/idea_chat_ui.js` (stays below the 580-line `assets_fonts.rs` split threshold). |
| `crates/desktop-server/assets/idea_chat_ui.js` | New | 449 | Multi-paper chat citation renderer integrating F6 `quote_chip.js` & `ui_badge.js`, explicit unread/wrong-paper warning cards, single & semicolon-grouped inline citation linker (skipping `<code>`/`<pre>`), OCR `onOpenPageImage` wiring, standalone `mountExploreChatCitations` with SSE `applySseEvent`, zero raw `innerHTML`, zero `"verified"`. |
| `crates/desktop-server/tests/idea_chat.rs` | New | 593 | Integration tests for prompt untrusted-data wrapping & S8, parser formats, Turn-1 & Turn-2 multi-paper citations + F1 fail-closed resume + zero code/write tools (Rule P7), `UiApprover` gating on `idea_search`/`idea_fetch_papers`, F9 budget & cancel, and Carbon audit M1/M2/M3/M5 regressions. |
| `crates/desktop-server/tests/idea_chat_http.rs` | New | 316 | End-to-end HTTP `POST /api/sessions/{id}/messages` tests verifying Turn-1 & Turn-2 Explore Conversation turns, zero Investigation hooks (`investigation.*` / `paper_claims.checked` absent), `idea.chat_citations` persistence, CLI provider refusal across explicit & `ui_settings.json` fallback paths, and F9 budget refusal. |
| `crates/desktop-server/tests/idea_chat_ui.test.mjs` | New | 329 | Node DOM tests for multi-paper matched chips, OCR `onOpenPageImage`, unread/wrong-paper warnings, inline linking (single + semicolon groups + `<code>` skip), XSS payload escaping, SSE updates, and static zero-`innerHTML` / zero-`"verified"` audit. |
| `crates/desktop-server/tests/mode_shell.rs` | Modified | 290 | Updated `s-explore-1` refusal needle from `"not built yet"` to `"idea statement"` (bare `POST /api/sessions { mode: "explore" }` without `text` assertion left intact). |
| `crates/desktop-server/tests/handoff_routes.rs` | Modified | 351 | Updated Explore handoff assertion to `assert_eq!(exp_res["target_mode_built"], true)`. |

**Frozen / forbidden files verified untouched (`git diff --name-only origin/gemini/work/fixed` empty):**
`Cargo.lock`, `crates/research-tools/src/tool_profile.rs`, `crates/research-tools/tests/golden/tool_defs.txt`, `crates/research-tools/tests/golden/tool_profiles.txt`, `crates/agent/src/resume.rs`, `crates/desktop-server/src/http_srv.rs`, `crates/desktop-server/src/routes_session.rs`, `crates/desktop-server/src/investigation_cites.rs`, `crates/desktop-server/assets/idea.js` (`593`L), `.github/`.

---

## 3. Deviations, Solo Decisions & Assumptions

1. **Deviation — 1-line edit in `crates/desktop-server/src/cli_turn_setup.rs:272` (`c5921ec`):**
   - *Context:* `exp-x15-chat.md` §4 lists `cli_turn*.rs` under Do-Not-Edit. However, `solve-s15-cli-parity` (merged into `gemini/work/fixed` before X15 started) gated `cli_turn_setup.rs::resolve_cli_turn_profile` on `if mode == Mode::Explore && !crate::session_mode::DESKTOP_MODE_EXPLORE_ENABLED`, and `crates/desktop-server/src/cli_turn_tests.rs:259` (`resolve_cli_turn_profile_maps_modes_and_rejects_unbuilt_explore`) explicitly asserts that an `idea.started` session returns `Err(... "profile not built yet")`.
   - *Conflict:* Flipping `DESKTOP_MODE_EXPLORE_ENABLED = true` (explicitly required by `RULES.md:89` and `exp-x15-chat.md:22,33`) without changing `cli_turn_setup.rs:272` simultaneously breaks untouched `cli_turn_tests.rs` and prematurely unlocks unbuilt Explore CLI turns before `exp-x18-cli` lands.
   - *Decision:* Added `pub const DESKTOP_MODE_EXPLORE_CLI_ENABLED: bool = false;` in `session_mode.rs` and switched line 272 of `cli_turn_setup.rs` in standalone commit `c5921ec` (`1 insertion, 1 deletion`). Both `cli_turn_tests.rs` and fail-closed Explore CLI refusal stay intact, and `exp-x18-cli` can flip or remove that single guard when X18 lands.
2. **Solo decision — `crates/desktop-server/assets/idea.js` left untouched (`593`L > `590`L):**
   - `assets/idea.js` is already `593` lines on `origin/gemini/work/fixed`. Per `exp-x15-chat.md` §3.2 ("export a standalone mounter documented in `Merge notes` if `idea.js` is at `> 590`L"), `idea.js` was not modified; `idea_chat_ui.js` exports `mountExploreChatCitations`, `renderExploreChatCitations`, and `linkExploreChatCitations` (see §5).
3. **Solo decision — Split `idea_chat_cites.rs` from `idea_chat.rs`:**
   - Keeping the pure multi-paper citation parser, PDF OCR range check, and F4 `lines_sent_with_ledger` verification in sibling `crates/desktop-server/src/idea_chat_cites.rs` (`520`L) keeps `idea_chat.rs` (`336`L) well under the 600-line ceiling and makes the citation checker directly reusable by `exp-x18-cli`.
4. **Solo decision — Pre-turn `idea.stated` requirement & CLI provider check across all fallback layers:**
   - Mirroring Solve's `solve.setup` pre-turn check in `turn_runner.rs`, `validate_explore_turn_preconditions` requires `latest_statement_seq(&pre_events) > 0` (returning HTTP 400 with `EXPLORE_CHAT_STATEMENT_REQUIRED_MSG`), rejects exhausted F9 `model_calls` budgets before spawning, and refuses `claude-cli`/`antigravity` synchronously (HTTP 400 with `EXPLORE_CHAT_CLI_UNSUPPORTED_MSG`) across `req.provider`, `meta.route`, `meta.default_provider`, `ui.agent_models["orchestrator"]`, and `ui.default_provider` (case-insensitive).
   - Bare `POST /api/sessions { "mode": "explore" }` without `"text"` in `routes_session.rs` was left untouched (`mode_shell.rs` still asserts its refusal).
5. **Solo decision — Additive ledger event `idea.chat_citations`:**
   - Persisted fail-closed after each Explore Conversation turn and broadcast via SSE `session_event`, recording `{ turn_seq, caller, node_id, matched_count, unread_count, citations }` so both replay and live UI consume exact F4-checked multi-paper citations without mutating any existing event schema.
6. **Named decision constants (per `RULES.md` §Scope):**
   - `EXPLORE_CHAT_CITE_MATCHED_WORD = "matched"`, `EXPLORE_SHARPENED_IDEA_LABEL = "Sharpened idea"` (`idea_system_prompt.rs`)
   - `EVENT_IDEA_CHAT_CITATIONS = "idea.chat_citations"`, `CHAT_CITE_STATUS_MATCHED`, `CHAT_CITE_STATUS_UNREAD` (`idea_chat_cites.rs`)
   - `EXPLORE_CHAT_STATEMENT_REQUIRED_MSG`, `EXPLORE_CHAT_CLI_UNSUPPORTED_MSG` (`idea_chat.rs`)
   - `DESKTOP_MODE_EXPLORE_ENABLED = true`, `DESKTOP_MODE_EXPLORE_CLI_ENABLED = false` (`session_mode.rs`)
   - `EVENT_IDEA_CHAT_CITATIONS`, `CHAT_CITE_MATCHED_WORD`, `CHAT_CITE_UNREAD_WORD`, `CHAT_CITE_UNREAD_DEFAULT_REASON`, `CHAT_CITE_INLINE_TITLE` (`assets/idea_chat_ui.js`)
7. **Assumption:**
   - Explore Conversation UI styling for `.idea-chat-*` classes is deferred to the Explore UI integration lane (or a follow-up CSS pass) because `style.css` is shared across concurrent UI lanes and not owned by `exp-x15-chat`.

---

## 4. Independent Carbon Audit Findings & Fixes

An independent read-only audit (`pro` subagent) reviewed Checkpoint 1 & Checkpoint 2 and identified 2 BLOCKING and 5 MAJOR defects; all 7 were fixed and covered by regression tests before final verification:

- **B1 (`idea_chat_ui.js`):** Inline citation regex failed to link semicolon-grouped citations (`[src-a: L1-5; src-b: L2-4]`). Fixed via full-bracket matching + `splitInlineBracketSegments` mirroring the Rust parser; regression in `tests/idea_chat_ui.test.mjs`.
- **B2 (`idea_chat.rs`):** Omitted `req.provider` with `ui.default_provider` or `ui.agent_models.orchestrator` set to `claude-cli`/`antigravity` bypassed synchronous 400 CLI refusal. Fixed by inspecting all provider fallback layers case-insensitively in `validate_explore_turn_preconditions`; regression in `tests/idea_chat_http.rs`.
- **M1 (`idea_chat_cites.rs`):** Versioned arXiv citations (`[arxiv:2501.00001v2: L2-3]`) failed `PaperKey::parse`. Fixed via `canonical_paper_key` falling back to `paper_fetch::parse_paper_input`; regression in `tests/idea_chat.rs`.
- **M2 (`idea_chat_cites.rs`):** Multiple `source_id`s sharing one canonical `paper_key` only checked the first source. Fixed via `resolve_matching_papers` checking all matching ingested sources; regression in `tests/idea_chat.rs`.
- **M3 (`idea_chat_cites.rs`):** Multi-page PDF ranges crossing `text_layer` page 1 onto `scan_text_layer` page 2 (or `\x0c` form-feed page breaks) dropped `"OCR"`. Fixed via `pdf_page_and_basis_for_range` inspecting all overlapping pages; regression in `tests/idea_chat.rs`.
- **M4 (`idea_chat_ui.js`):** `linkExploreChatCitations` ignored `options.onOpenPageImage` for inline OCR citations. Wired `onOpenPageImage` and `data-ocr="true"`; regression in `tests/idea_chat_ui.test.mjs`.
- **M5 (`idea_chat_cites.rs` & `idea_system_prompt.rs`):** `check_explore_answer_citations` fell back to scanning pre-chat events when no `session.start` existed after `idea.started` (now fails closed with `&[]`), and `idea_system_prompt.rs` kept the first instead of latest `source.ingested` per `source_id` (now takes latest); regressions in `tests/idea_chat.rs` (`carbon_audit_m1_m2_m3_m5_citation_regressions`).

---

## 5. Merge Notes

Exact seams for sibling/downstream lanes:

- **`exp-x18-cli` (Explore CLI turns):**
  - `crates/desktop-server/src/session_mode.rs`: flip `DESKTOP_MODE_EXPLORE_CLI_ENABLED = true` (or remove the guard in `cli_turn_setup.rs:272` and update `cli_turn_tests.rs:259` + `idea_chat.rs::validate_explore_turn_preconditions`) once `explore/1` CLI turns work end-to-end.
  - Reuse `crates/desktop-server/src/idea_chat_cites::{check_explore_answer_citations, record_chat_citations_event}` and `crates/desktop-server/src/idea_system_prompt::build_explore_chat_system_prompt` directly for Explore CLI turns so API and CLI share identical prompt wrapping and F4 multi-paper citation checking.
- **Explore UI integration (`assets/idea.js` / `exp-x12-x13-map-handoff`):**
  - Load `/assets/idea_chat_ui.js` (already served by `assets.rs`).
  - Standalone mounter signature:
    - `const ctrl = mountExploreChatCitations(hostEl, initialSource, { onOpenPassage, onOpenPageImage, turnSeq });`
    - `ctrl.applySseEvent(ev)` updates live on `idea.chat_citations` SSE events; `ctrl.render(citations)` re-renders on replay; `ctrl.destroy()` cleans up.
    - `linkExploreChatCitations(messageBodyEl, citations, { onOpenPassage, onOpenPageImage })` upgrades inline `[paper_key: Lstart-end]` and semicolon-grouped `[a: L1-2; b: L3-4]` tokens inside rendered assistant message DOM without using `innerHTML`.
  - Add CSS rules for the `.idea-chat-*` classes emitted by `idea_chat_ui.js`: `idea-chat-citations`, `idea-chat-citations-head`, `idea-chat-citations-title`, `idea-chat-citations-matched`, `idea-chat-citations-unread`, `idea-chat-cite-warning`, `idea-chat-cite-warning-head`, `idea-chat-cite-token`, `idea-chat-cite-loc`, `idea-chat-cite-reason`, `idea-chat-inline-matched`, `idea-chat-inline-unread`.
- **Localized edits in existing files (conflict awareness):**
  - `crates/desktop-server/src/routers.rs` (`build_desktop_router_for_profile` `PROFILE_EXPLORE_V1` arm)
  - `crates/desktop-server/src/turn_runner.rs` (`590`L — `Mode::Explore` preconditions, Investigate exclusion, dispatch; do not add > 10 lines without extracting a helper)
  - `crates/desktop-server/src/session_mode.rs` (`DESKTOP_MODE_EXPLORE_ENABLED = true`, `DESKTOP_MODE_EXPLORE_CLI_ENABLED = false`)
  - `crates/desktop-server/src/cli_turn_setup.rs:272` (1-line guard swap)
  - `crates/desktop-server/src/lib.rs` (3 `pub mod` lines)
  - `crates/desktop-server/src/assets.rs` (`542`L — 1 `/assets/idea_chat_ui.js` entry)

---

## 6. Verification Commands & Exact Measured Results

All commands ran on Linux as non-root (`uid=1001(antigravity)`, `gid=1001(antigravity)`), `rustc 1.100.0`, Node `v22.23.5`, Python `3.12.16`, on merged commit `4e29922`:

1. **File-length & frozen-file checks:**
   - `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` -> **PASS** (`No touched source file is over 600 lines.`)
   - `git diff --name-only origin/gemini/work/fixed -- Cargo.lock crates/research-tools/src/tool_profile.rs crates/research-tools/tests/golden/ crates/agent/src/resume.rs crates/desktop-server/src/http_srv.rs crates/desktop-server/src/routes_session.rs crates/desktop-server/src/investigation_cites.rs crates/desktop-server/assets/idea.js .github/` -> **PASS** (empty output)
2. **Workspace Clippy (`RE_SEARCH_PYTHON` unset):**
   - `unset RE_SEARCH_PYTHON && cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` -> **PASS** (`0` warnings, `0` errors)
3. **Full workspace tests (`RE_SEARCH_PYTHON` unset):**
   - `unset RE_SEARCH_PYTHON && cargo test --workspace --locked --no-fail-fast` -> **PASS**: **`1453` passed, `0` failed, `2` ignored**
   - Key test targets included in that run:
     - `crates/desktop-server/tests/idea_chat.rs`: `5 passed, 0 failed, 0 ignored`
     - `crates/desktop-server/tests/idea_chat_http.rs`: `1 passed, 0 failed, 0 ignored`
     - `crates/desktop-server/tests/mode_shell.rs`: `4 passed, 0 failed, 0 ignored`
     - `crates/desktop-server/tests/handoff_routes.rs`: `2 passed, 0 failed, 0 ignored`
     - `crates/desktop-server/src/cli_turn_tests.rs::resolve_cli_turn_profile_maps_modes_and_rejects_unbuilt_explore`: `ok`
     - `crates/research-tools/tests/tool_def_golden.rs`: `3 passed, 0 failed, 0 ignored`
     - `crates/research-tools/tests/tool_profile_golden.rs`: `3 passed, 0 failed, 0 ignored`
   - **Pre-existing ignored tests (2, unmodified from `origin/gemini/work/fixed`):**
     - `file_sheet_from_json ... ignored, live tier: needs EVAL_SHEET_JSON`
     - `live_manifests_validate ... ignored, live tier: needs EVAL_LIVE_MANIFEST_DIR`
   - Zero skipped or `#[ignore]` tests added by this brief.
4. **Node JS syntax & UI test suite:**
   - `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` -> **PASS** (all JS assets valid)
   - `node --test crates/desktop-server/tests/*.test.mjs` -> **PASS**: **`266` passed, `0` failed, `0` skipped** (including all new `tests/idea_chat_ui.test.mjs` tests)

---

## 7. What Could Not Be Run on Linux & Plain Statement of Limits

- **Not compiled or run on Linux:** Windows-only `#[cfg(windows)]` desktop/AppContainer runtime paths and the live Windows 11 + `[9/9]` Chromium UI batch (scheduled per `RULES.md` §Integration cadence).
- **Plain statement of what does not work yet:**
  1. **Explore CLI turns (`claude-cli` / `antigravity`)** intentionally return HTTP 400 (`EXPLORE_CHAT_CLI_UNSUPPORTED_MSG`) and fail closed in `cli_turn_setup.rs:272` (`DESKTOP_MODE_EXPLORE_CLI_ENABLED = false`) until `exp-x18-cli` implements `explore/1` CLI/MCP support.
  2. **Automatic UI mounting & styling in the Explore tab:** `/assets/idea_chat_ui.js` is served and unit-tested, but not auto-mounted inside `assets/idea.js` (because `idea.js` is already `593`L > `590`L), and CSS rules for `.idea-chat-*` classes are not yet added to `style.css`.
- **Other bugs noticed but not fixed:** None outside the scope of X15.
