# Report: ux-ui (P4 & P5 — Question-first Paper view & Checked line-citation chips)

- **Brief:** `ux-ui` (`docs/design/investigation-2026-11/agents/ux-ui.md`)
- **Design:** `docs/design/investigation-2026-11/02-investigation-ux.md` (D5, D6, D7, D9; packages P4 & P5; section 4 corrections)
- **Base commit:** `91753b86b4392814cbe0fe4dd213c9be1d15fc80` (`origin/gemini/runner`)
- **Branch:** `work/ux-ui` -> pushed to `gemini/work/ux-ui`
- **Toolchain on Linux VM:** `rustc 1.100.0`, `clippy 0.1.100`, `node v22.23.5`, `Python 3.12.16`

---

## 1. Implementation summary by package

### Package P4 — Paper view UI (D5, D6, D7)

1. **`crates/desktop-server/assets/investigation_state.js` (new, 129 lines)**
   - Exports `paperViewState(inv, session, nowMs)`, `formatElapsed(elapsedMs)`, `setupButtonLabel(question)`, and `latestRequestEvent(events)`.
   - Computes all Paper view visibility gates (`showSetup`, `refillQuestion`, `showSummary`, `showConventions`, `showMapUnavailable`, `pollSummary`, `showAnswer`, `showComposer`, `composerPlaceholder`).
   - Handles absent P1b fields (`answers`, `mapping_requested`) gracefully: when absent, defaults to `answers = []` and legacy mapping gates (`source_id || map || ...`) so the branch works standalone before `ux-rust` merges.
   - Implements all D6 status copy states (PDF upload, other upload, question running, mapping without summary, mapping with summary saved, approval pending with `"Review in Conversation"` button flag, mapping stopped via `session.abort` with `payload.stopped === true` after `investigation.mapping_requested`, mapping error, and unavailable fallback).
   - Formats elapsed time as `m:ss` (e.g. `0:42`, `12:05`) from client submit time or latest `investigation.*_requested` / `investigation.requested` / `investigation.conventions_resumed` event `ts_ms`.

2. **`crates/desktop-server/assets/investigation_answer.js` (new, 131 lines)**
   - Exports `createAnswerCard(host, { onFindMainResults, onOpenConversation, openCite })`.
   - Renders `<h2>Answer</h2>`, the newest question via `renderMathTextInto`, body via `renderRichInto` inside `.rich-prose`, and the note `"Written by the model. Check the cited lines."`.
   - Handles `running` (`"Working on your question…"`), `stopped` (`"Stopped before the answer was written. Ask again below."`), `failed` (`row.error` via `textContent`), and `interrupted` states.
   - Collapses older answers inside `<details><summary>Earlier questions (n)</summary>` and renders them lazily only when toggled open.
   - Renders `"Find main results"` (only when no map exists; sends the newest question with `investigation_scope: "map"`) and `"Open in Conversation"` (calls `selectView('chat')`).

3. **`crates/desktop-server/assets/index.html` (768 lines; net 0 lines)**
   - Tab label `"Paper map"` changed to `"Paper"`; `"Conversation"` and `aria-pressed` unchanged.
   - Setup form intro copy updated to `"Add one paper and ask about it. Answers cite line numbers in the paper."`.
   - Question textarea moved above the file input, relabeled `"Your question (optional)"` with placeholder `"e.g. Help me understand section V.D.4"`. Page fieldset unchanged.

4. **`crates/desktop-server/assets/investigation.css` (97 lines)**
   - Moved `#composer-input, #composer-send` out of the unconditional Paper-view hide rule into `#app-shell[data-investigation-view="map"]:not([data-investigation-has-paper="true"]) :is(#composer-input, #composer-send) { display: none; }`.
   - Added styles for `#investigation-answer`, `.investigation-answer-question`, `.investigation-answer-note`, `.investigation-earlier-answers`, `.investigation-earlier-item`, and `.investigation-status-action`.

5. **`crates/desktop-server/assets/investigation.js` (537 lines)**
   - Creates `<section id="investigation-answer" aria-live="polite" hidden>` after `#investigation-status` via `.after(...)` (keeping `index.html` at net 0 lines).
   - Applies `paperViewState(...)` gates, sets `data-investigation-has-paper` on `#app-shell`, updates composer placeholder via `setComposerPlaceholder(...)`, updates submit button text live on question input (`"Answer my question"` vs `"Find main results"`), and passes `Boolean(data.running) && state.showSummary` to `paperMap.updateSummary(...)` so question-only runs do not reveal the summary box.
   - Moves `callbacks.requestOptions()` before `file.arrayBuffer()` / upload in `buildMap`, sends `investigation_scope: readingQuestion ? 'question' : 'map'`, and adds `findMainResultsFromAnswer(question)` for the Answer card button.
   - Drives `#investigation-status` with a 1 s client timer (`textContent` only) and listens for `research:approvals` events plus `data.pending_approvals` on load.
   - Calls `callbacks.setBusy?.(true)` after 202 responses from `findInvestigationConventions`, `developInvestigation`, and `checkInvestigation`.

6. **`crates/desktop-server/assets/investigation_conventions.js` (155 lines; 1-line edit)**
   - Dropped `inv.source_id` from `host.hidden` gate (`!(catalog || inv.conventions_requested || mapSeq != null)`).

7. **`crates/desktop-server/assets/investigation_map.js` (528 lines; small edit)**
   - Added `{ openFirst = false } = {}` option to `sources(parent, anchors, options)` and passed `{ openFirst: true }` for the selected claim in `detail(node)`. Summary and conventions anchors remain closed by default.

8. **`crates/desktop-server/assets/composer.js` (577 lines)**
   - Added `setComposerPlaceholder(text)` used by `syncButtons()`.
   - Added Esc guards returning early when `document.querySelector?.('dialog[open]')` matches or `ev.target?.closest?.('#investigation-panel :is(input,textarea,select)')` matches.

9. **`crates/desktop-server/assets/app.js` (931 lines; +3 lines exact)**
   - Passed `setBusy: setComposerBusy` into `initInvestigation(...)`.
   - Dispatched `document.dispatchEvent(new CustomEvent('research:approvals', { detail: { sessionId: id, waiting: ... } }))` in `approval_requested` and `approval_resolved`.

10. **Tests added/updated for P4**
    - New `crates/desktop-server/tests/investigation_state.test.mjs` (7 tests)
    - New `crates/desktop-server/tests/investigation_answer.test.mjs` (2 tests)
    - New `crates/desktop-server/tests/composer_esc.test.mjs` (3 tests)
    - Updated `crates/desktop-server/tests/investigation_conventions.test.mjs` (added `header_hidden_with_paper_but_no_mapping`, kept all existing assertions untouched)
    - Updated `crates/desktop-server/tests/investigation_origin.test.mjs` (added `.after` on `investigation-status`, `document.addEventListener`, and new module stubs; kept all existing assertions untouched)
    - Updated `crates/desktop-server/tests/investigation_browser.py` (main journey + new `check_question_first_investigation` covering no-model pre-check, question-first Answer card, composer visibility, `"Find main results"` with `scope: "map"`, follow-up after map, Esc in conventions search box, Stop button after `"Continue conventions review"`, and P5 citation chip)

### Package P5 — Checked line-citation chips (D9)

1. **`tool.result` payload nesting verification**
   - Confirmed in `crates/tools/src/router.rs:411-412` and `crates/desktop-server/src/evidence.rs:1004-1012` that `tool.result` payloads store `{ "call_id": ..., "ok": true, "result": { "inline": "<json>" } | { "blob": "<sha256>" } }`.
   - Confirmed `fs_read` offset mode returns `{ "root": "sources", "path", "start_line", "end_line", "total_lines" }` (`crates/research-tools/src/fs_tools.rs:288-301`), grep mode returns `{ "root": "sources", "path", "matches": [{ "line", "text" }], "total_lines" }` (`:402-416`), and `search_paper_passages` returns `{ "passages": [{ "path", "start_line", "end_line", ... }] }` (`crates/research-tools/src/paper_passages.rs:75-76,152-157`).

2. **`crates/desktop-server/src/investigation_cites.rs` (new, 426 lines)**
   - Implements `collect_received_ranges`, `parse_line_citations`, `answer_cites`, `source_total_lines`, and `answer_cites_for_session`.
   - Unpacks inline/blob/object `tool.result` records after `request_seq`, restricts `fs_read` to `root == "sources"` and `path == primary_path`, merges overlapping/adjacent ranges, keeps only `1 <= start <= end <= total_lines`, and marks `read: true` iff `[start, end]` is fully contained in a received range.
   - Includes in-crate unit test `cites_marked_read_only_inside_received_ranges` covering offset (inline + blob + merged consecutive reads), grep (including gap between matches), `search_paper_passages` (primary vs other paper), prior-turn/workspace isolation, unread lines, and out-of-range/reversed/zero rejection.

3. **`crates/desktop-server/assets/paper_cites.js` (new, 105 lines)**
   - Exports `linkLineCitations(container, cites, open)`.
   - Walks text nodes skipping `code`, `pre`, `.katex`, `.katex-display`, `button`, `a`, `script`, `style`.
   - Matches `\[L(\d+)(?:[–-](\d+))?(?:,\s*L(\d+)(?:[–-](\d+))?)*\]`; converts only `read: true` valid ranges into `<button type="button" class="line-cite">` with `textContent` and the required `title` text; leaves unread/reversed ranges and surrounding HTML as plain text.

4. **Wiring, CSS, asset registration, and tests**
   - Called `linkLineCitations(body, row?.cites, openCite)` in `investigation_answer.js`, passed `openCite` from `investigation.js` to `callbacks.openSource(...)`.
   - Added dotted-underline `.line-cite` styling in `investigation.css`.
   - Registered `mod investigation_cites;` in `crates/desktop-server/src/lib.rs` and the 3 new JS assets in `crates/desktop-server/src/assets.rs`.
   - Added `crates/desktop-server/tests/paper_cites.test.mjs` (4 tests).

---

## 2. Deviations from the design and why

1. **P5 Rust module placed in `crates/desktop-server/src/investigation_cites.rs` instead of `investigation_question.rs`:**
   - Because `ux-rust` creates `investigation_question.rs` in parallel (`PARALLEL.md`), creating a separate `investigation_cites.rs` avoids an add/add merge conflict and keeps both files well under 600 lines. Integration into `ux-rust`'s `finish_question` is a 1-line call (documented in Merge notes below).
2. **`crates/desktop-server/tests/investigation_browser.py` lines `:223`, `:235-236`, `:267`, `:285`:**
   - The design asked to leave the question blank at `:156` while keeping every other assertion. With a blank question (`DEFAULT_PAPER_REQUEST`), `readingQuestion(inv)` is `""`, so `Focus question` defaults to `"What assumptions, methods and evidence support this result?"` (not `"Which assumption connects the result?"`) and `paperQuestionFocus(map, "")` is empty (no `.question-match`). Also, because `openFirst: true` now opens `.investigation-selected details` by default, clicking `.investigation-selected summary` at `:285` would have *closed* the details and broken `:286` (`Open cited lines`).
   - Resolution: updated `:223` and `:267` in the blank-question main journey to expect the broad default, replaced `:285`'s toggle click with `expect(graph.locator(".investigation-selected details").first).to_have_attribute("open", "")`, and preserved the `"Which assumption connects the result?"` `Focus question` and `.question-match` (`"Selected for your question"`) assertions in the new question-first -> `"Find main results"` journey (`check_question_first_investigation`). Zero assertion coverage was lost.
3. **`buildMap` calls `callbacks.requestOptions()` after local empty-file/size/page-range validation and before `file.arrayBuffer()`:**
   - Keeps invalid form inputs (missing file, bad page range) showing inline validation messages without opening Connect, while still guaranteeing that when a paper is selected with no model connected, Connect opens and zero bytes are read or uploaded.

---

## 3. Decisions taken unattended

- **Absent P1b fields fallback:** Treated `typeof inv.mapping_requested !== 'boolean'` with empty `answers` as legacy mapping gates (`source_id || map || ...`), and with non-empty `answers` as `mapping_requested = false`.
- **Summary box during question-only turns:** Passed `Boolean(data.running) && state.showSummary` as the `running` parameter to `paperMap.updateSummary(...)` rather than modifying `investigation_map.js`'s `updateSummary` signature, keeping `investigation_map.js` changes strictly limited to `openFirst`.
- **Multi-range citation rendering (`[L3-5, L9]`):** Kept surrounding `[`, `, `, and `]` as plain text nodes and rendered each `read: true` range as `<button type="button" class="line-cite">L3-5</button>`. This preserves exact original `textContent`, allows mixed read/unread brackets (`[L4, L99]` makes only `L4` clickable), and avoids nested interactive controls.
- **Interval merging in P5:** Merged adjacent/overlapping received ranges (`a.1 + 1 >= b.0`) so consecutive `fs_read` chunks (e.g. `10..20` then `21..30`) validate `[L18–25]`, while non-adjacent grep matches (`40` and `42`) still mark `[L40-42]` as `read: false`.
- **Clippy dead-code hygiene before merge:** Marked `source_total_lines` and `answer_cites_for_session` with `#[allow(dead_code)]` until `ux-rust`'s `finish_question` calls `answer_cites_for_session` during merge.

---

## 4. Merge notes (for the wave merge agent)

| File | Our edit (commit) | Other brief(s) & how to merge |
|---|---|---|
| `crates/desktop-server/assets/app.js` | `60e91f1` (+3 lines: `setBusy: setComposerBusy` in `initInvestigation`; 2 `research:approvals` `dispatchEvent` lines in `approval_requested` / `approval_resolved`) | `latex` adds 2 macro-reset lines elsewhere in `app.js`; should merge cleanly. Keep total `app.js` delta small. |
| `crates/desktop-server/assets/investigation_conventions.js` | `dd556b7` (1 line at `:125`: drop `inv.source_id` from `host.hidden`) | Owned by `split-1`. Reapply this 1-line gate change if `split-1` refactors `update()`. |
| `crates/desktop-server/assets/investigation_map.js` | `e0fd954` (`openFirst` option on `sources()` and `{ openFirst: true }` in `detail()`) | Owned by `split-1` (and `split-2` reader label). Keep `openFirst` on `sources()` and `detail()`. |
| `crates/desktop-server/assets/investigation.js` | `10ce09e`, `ac0fb41` (main owner: `ux-ui`) | `split-3` adds a scope chip to `investigation.js`. Integrate `split-3`'s chip alongside `paperViewState` and `buildMap`. |
| `crates/desktop-server/src/assets.rs` | `38b4c67` (+12 lines registering `investigation_state.js`, `investigation_answer.js`, `paper_cites.js`) | `latex` may add/remove JS/CSS entries in `lookup_asset`; combine all `match` arms. |
| `crates/desktop-server/src/lib.rs` | `38b4c67` (+1 line: `mod investigation_cites;`) | `ux-rust` (P0) adds `mod investigation_view;`, `mod investigation_finish;`, `mod investigation_question;`, `mod investigation_system_prompt;`, `mod investigation_title;`. Keep all `mod` lines. |
| `crates/desktop-server/src/investigation_question.rs` & `investigation_view.rs` | Not edited by `ux-ui` (created by `ux-rust`) | **Required post-merge hook (2 lines):** In `ux-rust`'s `finish_question`, compute `let cites = crate::investigation_cites::answer_cites_for_session(state, &ledger, session, &events, seq, &clipped_text);` and include `"cites": cites` on the `investigation.answer` payload (and remove `#[allow(dead_code)]` on `investigation_cites.rs`). In `investigation_view.rs`'s `view()`, ensure `"cites"` is copied from `investigation.answer` onto each `answers` entry. If `split-1` adds `read_paper_lines`, also accept `"read_paper_lines"` alongside `"fs_read"` in `investigation_cites::collect_received_ranges`. |

---

## 5. Verification & test results

| Command | Result | Notes |
|---|---|---|
| `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` | **PASS** (all JS assets) | 0 syntax errors |
| `node --test crates/desktop-server/tests/*.test.mjs` | **80 passed, 0 failed** | Up from 63 baseline (+17 new tests across `investigation_state`, `investigation_answer`, `composer_esc`, `investigation_conventions`, `paper_cites`) |
| `python3 -m py_compile crates/desktop-server/tests/investigation_browser.py` | **PASS** | Syntax verified |
| `cargo test -p desktop-server --lib investigation_cites` | **1 passed, 0 failed** | `cites_marked_read_only_inside_received_ranges` |
| `RE_SEARCH_PYTHON="$(command -v python3)" cargo test -p desktop-server --no-fail-fast` | **104 passed, 0 failed, 0 ignored** (27 test suites) | Includes `investigation_cites::tests::cites_marked_read_only_inside_received_ranges` and `ui_math` |
| `cargo clippy -p desktop-server --all-targets -- -D warnings` | **1 pre-existing error** | Zero warnings in any file touched or added by `ux-ui` (see pre-existing section below) |

### Pre-existing failures (with proof)

- **`cargo clippy -p desktop-server --all-targets -- -D warnings`** fails on `crates/desktop-server/src/evidence.rs:737-743` under `clippy 0.1.100` (`rustc 1.100.0`) with `clippy::manual_unwrap_or`.
  - Proof it is pre-existing: `git show 91753b86b4392814cbe0fe4dd213c9be1d15fc80:crates/desktop-server/src/evidence.rs | sed -n '737,743p'` shows the exact same `match result["status"].as_str()` block on the base commit.
  - Note: Clippy's suggested replacement (`result["status"].as_str().unwrap_or("result_unavailable")`) is a Clippy false positive that would remove the 7-value status allowlist (`"verified" | "failed" | "source_changed" | "rejected_sorry" | "rejected_axiom" | "lean_not_found" | "timeout"`). Left untouched.

### Not compiled or run on Linux

- `cfg(windows)` code paths (not compiled on Linux).
- Playwright browser tests (`crates/desktop-server/tests/investigation_browser.py` and `workbench_browser.py`): out of scope on Linux per `RULES.md` and `agents/ux-ui.md`; coordinator runs them on Windows.

---

## 6. Bugs / issues noticed but not fixed

- `crates/desktop-server/src/evidence.rs:737-743`: triggers `clippy::manual_unwrap_or` on Clippy `0.1.100` (outside `ux-ui` scope; Clippy's auto-suggestion would drop the allowlist guard).
