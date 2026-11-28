# Report: `exp-x12-x13-map-handoff` — Explore Map & Verdict UI (X12) and Directions & Hand-over Buttons (X13)

- **Branch:** `gemini/modes/exp-x12-x13-map-handoff` (local `work/exp-x12-x13-map-handoff`)
- **Initial base commit (`origin/gemini/work/fixed`):** `ffee92f4bb0372a362b2e2781bb3c8da13b7cdaa`
- **Merged latest `origin/gemini/work/fixed`:** `61fd8c2b1d784feed1ee97b3d7a415290c4db9fa` (includes `f8-set-c-harness` PR #80, `exp-x15-chat` PR #82, `f15-writeup-explore` PR #83, `exp-x16-cite` PR #84, and `exp-x17-library` PR #85)
- **Checkpoints & commits:**
  1. `b1fe1d2` — **X12**: Map & verdict UI over F6 `graph_view.js`, `quote_chip.js`, `ui_badge.js`, `createNodeCardShell`, and `POST /api/sessions/{id}/idea/note` Rule V6 confirmation handler.
  2. `3dc9ae1` — **X13**: Directions recorder (`record_idea_directions`), Rules D1/D2/V8 validator, Directions exchange, F7 hand-over builders (`"Open in Solve"` & `"Investigate this paper"`), Directions cards (`idea_directions.js`), SSE reducer updates, and Rust/Node tests.
  3. `6ba20b9` + `b25d93a` — `--no-ff` merge of `origin/gemini/work/fixed` (`575a72c`, PRs #80 & #82).
  4. `4632985` — `--no-ff` merge of `origin/gemini/work/fixed` (`61fd8c2`, PRs #83–#85), resolving additive module declarations in `desktop-server/src/lib.rs` and updating `idea_flow.rs` now that `directions`, `handoff`, and `more` are all wired.
  5. Final commit — this report.

---

## 1. What was implemented

### Package X12 — Map & verdict UI (pushed at `b1fe1d2`)

| File | Status | Lines | Purpose |
| --- | --- | ---: | --- |
| `crates/desktop-server/src/routes_idea_map.rs` | new | 451 | `POST /api/sessions/{id}/idea/note` (`confirm_same`, `reject_same`, node flags `wrong`/`not_relevant`/`pin`/`comment`); enforces Rule V6 across all 3 `judged_*` statuses, binds `confirm_same`/`reject_same` to active `map_seq` (HTTP 409 on missing/mismatched `map_seq`), attaches `subclaim_id`; includes unit test `idea_same_needs_user_confirmation_across_all_three_judged_statuses`. |
| `crates/desktop-server/assets/idea_verdict_ui.js` | new | 355 | Verdict table (`VERDICT_STATUS_CHIPS`, `resolveVerdictChip`, `map_seq`-scoped `indexNotes`, `toQuoteChipSpec`, `projectVerdictRow`, `renderVerdictTable`) and lazy live source/OCR page-image open bridges (`openExploreSource`, `openExplorePageImage`). |
| `crates/desktop-server/assets/idea_map.js` | new | 414 | Thin adapter over F6 `createGraphView`, `createNodeCardShell`, `createBadge`, `createQuoteChip` (zero layout code, zero SVG drawing, zero `innerHTML`). Renders verdict table, Decision 13 stale banner (`"Re-map against revision n"`), synchronized grouped list <-> F6 GraphView <-> node drawer (`codeBlock` vs `modelBlock` separation), node flag controls, and Rule S4 `not_in_current_map` visibility. |
| `crates/desktop-server/tests/idea_map.test.mjs` | new | 569 | Node DOM tests for Rule V6 across all 3 `judged_*` statuses + `reject_same`, `quote_chip.js` PDF/OCR/abstract rendering, synchronized grouped list <-> GraphView <-> drawer, node flags, Rule S4 dimming, Decision 13 stale banner, XSS safety, zero `innerHTML`, and Rule S8 sweep. |
| `crates/desktop-server/src/idea_view_map.rs` | edited | 125 | Projects `map.idea_rev` on the `GraphView` payload for the Decision 13 stale banner. |
| `crates/desktop-server/src/idea_view.rs` | edited | 593 | Projects `subclaim_id` & `map_seq` on `notes`, retains pre-revision evidence while `map_stale` is true so stale-map quote chips keep text/page/OCR metadata, and clears `latest_directions` on `EVENT_IDEA_MAP` re-map. |
| `crates/desktop-server/src/routes_idea.rs` | edited | 537 | Delegates `"note"` to `routes_idea_map::handle_note`. |
| `crates/desktop-server/src/lib.rs` | edited | 288 | Registers `routes_idea_map`. |
| `crates/desktop-server/src/assets.rs` | edited | 566 | Registers `/assets/idea_map.js` and `/assets/idea_verdict_ui.js`. |
| `crates/desktop-server/assets/idea.js` | edited | 586 | Mounts `renderIdeaMapSection`, persists `selectedMapNodeId` across re-renders, wires `onNote`/`onRemap`/live source & OCR page-image bridges. |
| `crates/desktop-server/assets/idea.css` | edited | 480 | Styles X12 map/verdict/drawer/stale-banner and Rule S4 dimming using only valid `tokens.css` variables and responsive `@media (max-width: 960px)`. |

### Package X13 — Directions recorder, cards & F7 hand-over buttons (pushed at `3dc9ae1`)

| File | Status | Lines | Purpose |
| --- | --- | ---: | --- |
| `crates/research-tools/src/idea_directions.rs` | new | 386 | `Direction`, `FirstCalculation`, `FirstCalcMethod` (`ByHand | Sympy | Numeric | Literature` — **NO `lean`**), `EffortLevel`, `RiskKind`, `Concreteness::{Concrete, Sketch}`, `MissingBasis`, `SolveSetup` (Solve's exact `TaskKind` + `SolveSetupTarget { description, form: TargetForm }` + `statement`, `conventions`, `source_keys`), `#[serde(deny_unknown_fields)]` inputs, and `record_idea_directions_schema()`. |
| `crates/research-tools/src/idea_directions_validate.rs` | new | 359 | `validate_record_idea_directions` enforcing Rules D1 (rejects unknown node IDs, `not_in_current_map` nodes, invalid `missing.basis`, non-`known_result` `check.reproduces`, invalid `PaperKey`s, `> 5` directions, and Rule V8 banned phrases while permitting `"first calculation"`) and D2 (strictly app-computed `Concreteness::Concrete` vs `Sketch`, revision-scoped `xe-N`). |
| `crates/research-tools/src/idea_directions_tests.rs` | new | 296 | Unit tests covering Rules D1, D2, V8, `"first calculation"`, `"lean"` rejection, Solve field alignment (`task_kind: "check"` rejected), `> 5` directions, mixed batches, `not_in_current_map` rejection, prior-revision `xe-N` non-promotion, and `open_question(q-0)` rejection. |
| `crates/desktop-server/src/idea_directions_exchange.rs` | new | 396 | `run_directions_with_transport` & `handle_directions_op`: Rule P1 stage gate + `validate_recorder_stage`, F9 budget check before call & repairs, folded brief answers + `idea.note` flags inside `wrap_untrusted_data`, `append_at_head` under `ledger_write_lock` with revision-race cancellation, SSE emissions. |
| `crates/desktop-server/src/idea_handoff.rs` | new | 556 | `handle_idea_handoff_op`, `build_open_in_solve_request` (Concrete & Loose directions -> Solve F7 `HandoffRequest` with Solve-valid `suggested_done` [`NamedCheckPassed` with sanitized `params.check` for `check.reproduces`, or `UserConfirms` for free check text], `xe` `QuoteRef`s, unioned `known`-node + `source_keys` papers, prose in `model_text`, `user_notes`, `node_fp`), `build_investigate_paper_request` (preserves matched quotes, node summaries & `user_notes` even when `map_stale == true`), and read-only `read_explore_handoffs`. Delegates execution directly to `crate::handoff::handle_create_handoff` (0 model/network calls). |
| `crates/desktop-server/assets/idea_directions.js` | new | 414 | Concrete group (expanded) vs Loose ideas group (collapsed `<details>`), fixed card layout, Rule D3 (`"suggestion (model)"` dashed badges on all free prose; only `data-citation-chip="true"` node chips count as citations), `"Open in Solve"` (Concrete & Loose) + `"Investigate this paper"` buttons, `map_seq`-scoped read-only child status chips, `executeExploreHandoff`, `fetchExploreHandoffsSafe`, `openChildSessionDefault` (normalizing `"investigate"` -> `"investigation"`), zero `innerHTML`, zero `"lean"`, Rule S8 clean. |
| `crates/desktop-server/tests/idea_directions.test.mjs` | new | 443 | Node DOM tests for Rule D3 (`idea_direction_marks_suggestions`), Concrete vs collapsed Loose `<details>`, fixed card layout, `"Open in Solve"` & `"Investigate this paper"`, read-only child status chips, controller handoff wiring, SSE reducer updates/clearing, `map_seq`-scoped status filtering, XSS safety, zero `innerHTML`, Rule S8 sweep, zero `"lean"`. |
| `crates/desktop-server/tests/idea_handoff.rs` | new | 541 | Integration tests: scripted Directions repair on `"lean"` + Concrete/Sketch persistence + Rule S8 check; `"Open in Solve"` (Concrete & Loose) with 0 model/network calls, 100% `QuoteRef` re-verification, Solve `validate_and_build_setup` acceptance of `suggested_done`, `user_notes` carry-over, HTTP 404/400 error paths, strictly read-only child status flow-back; `"Investigate this paper"` under `map_stale == true`; tampered quote drop (`quotes_dropped >= 1`, step arrives as `given`). |
| `crates/research-tools/src/lib.rs` | edited | 385 | Exports `pub mod idea_directions;` and `pub mod idea_directions_validate;`. |
| `crates/desktop-server/src/lib.rs` | edited | 288 | Exports `pub mod idea_directions_exchange;` and `pub mod idea_handoff;`. |
| `crates/desktop-server/src/routes_idea.rs` | edited | 537 | Wires `"directions"`, `"handoff"`, `IdeaStage::Directions` in `handle_continue`, and returns `200 { ok: true, done: true }` when `next_stage` is `None`. |
| `crates/desktop-server/src/assets.rs` | edited | 566 | Registers `/assets/idea_directions.js`. |
| `crates/desktop-server/assets/api_idea.js` | edited | 99 | Adds `getExploreHandoffs(sessionId)` (`GET /api/sessions/{id}/handoffs`). |
| `crates/desktop-server/assets/idea.js` | edited | 586 | Mounts `renderIdeaDirectionsSection` and wires non-blocking read-only handoffs fetch + `onOpenInSolve`/`onInvestigatePaper`/`onOpenChildSession`. |
| `crates/desktop-server/assets/idea_state.js` | edited | 590 | Handles SSE `idea.directions` updates and clears `directions` on `idea.stated` and `idea.map`. |
| `crates/desktop-server/assets/idea.css` | edited | 480 | Adds X13 `.idea-directions-*` and `.idea-direction-*` styles using `tokens.css` variables. |
| `crates/desktop-server/tests/idea_flow.rs` | edited | 481 | Replaces 501 stub op checks with unknown-op 404 check after `directions`/`handoff` (X13) and `more` (X16) all landed. |

---

## 2. Independent Carbon Audits & Fixes Applied

Two independent read-only Carbon audits (`research` subagent, `Model: pro`) were executed — one after X12 and one after X13 — and every actionable finding was resolved and covered by regression tests:

- **X12 Audit Fixes (all landed in `b1fe1d2`):**
  - Split verdict table + live source/OCR open bridges into `idea_verdict_ui.js` (355L) so `idea_map.js` (414L) stays well below 600 lines.
  - Fixed `routes_idea_map.rs` unit test fixture (`src-1/paper.tex` + `files_sha256`) so `idea_verdict::compute` reaches all three `judged_*` states cleanly.
  - Wired lazy live `openExploreSource` & `openExplorePageImage` defaults in `idea.js`/`idea_verdict_ui.js`, derived grouped list/drawer badges strictly from F6 `grade`, bound `confirm_same`/`reject_same` to active `map_seq` (HTTP 409 on mismatch), persisted `selectedMapNodeId`, separated model-authored node properties into `modelBlock`, fixed legend spacing, edge `abstract_only` fallback, responsive CSS, and Rule S4 dimming.
- **X13 Audit Fixes (all landed in `3dc9ae1`):**
  - **CRITICAL:** Fixed `suggested_done` in `idea_handoff.rs` to emit valid Solve `DoneCriterion` payloads (`NamedCheckPassed` with sanitized lowercase `params.check` like `reproduce_k_1` when `check.reproduces` is set, or `UserConfirms` with sanitized `params.text` 1..=400 chars otherwise) and verified acceptance via `research_tools::solve_setup::validate_and_build_setup` in `tests/idea_handoff.rs`.
  - **HIGH:** Rejected `MapNodeStatus::NotInCurrentMap` nodes in `idea_directions_validate.rs` (Rule D1); preserved matched quotes, map node summaries, and `user_notes` on `"Investigate this paper"` while `map_stale == true`; added SSE `idea.directions` / `idea.stated` / `idea.map` handling in `idea_state.js`; carried `EVENT_IDEA_NOTE` `user_notes` across both hand-offs; cleared `latest_directions` on `EVENT_IDEA_MAP` re-map in `idea_view.rs`, checked `payload.map_seq == active_map_seq` in `find_active_direction`, populated `from.node_fp`, and scoped Solve direction status chips by active `map_seq` in `idea_directions.js`.
  - **MEDIUM & LOW:** Folded brief edits/answers and active `idea.note` flags into `build_directions_prompt`; unioned `known`-node papers with `solve_setup.source_keys` into `packet.papers`; handled `next_stage == None` cleanly in `handle_continue` (`200 { ok: true, done: true }`).

---

## 3. Decisions Taken Alone, Deviations & Assumptions

1. **Left `crates/desktop-server/assets/new_menu.js` untouched (Decision 7 check):**
   - Although `exp-x15-chat` (`575a72c`) flipped `DESKTOP_MODE_EXPLORE_ENABLED = true` in `session_mode.rs`, `"New research"` is already under `More` (`underMore: true`) in `new_menu.js` (from Solve S13), and `new_menu.js` intentionally keeps Explore `built: false` (`crates/desktop-server/tests/new_menu_and_rail.test.mjs:133-195` explicitly asserts `#new-explore-btn` is disabled with `"not built yet"`). Per brief §2, `new_menu.js` was left untouched so `new_menu_and_rail.test.mjs` stays green.
2. **Sibling splits for the 600-line ceiling (Rule S6):**
   - Split X12 into `routes_idea_map.rs` + `idea_verdict_ui.js` + `idea_map.js`, and X13 into `idea_directions.rs` + `idea_directions_validate.rs` + `idea_directions_tests.rs` + `idea_directions_exchange.rs` + `idea_handoff.rs` + `idea_directions.js`. Zero touched source files exceed 600 lines (max 593L on `idea_view.rs`).
3. **`suggested_done` mapping for `first_calculation.check`:**
   - When `check.reproduces` names `k-N`, `idea_handoff.rs` emits `DoneCriterionKind::NamedCheckPassed` with `params.check = "reproduce_k_N"` (sanitized `[a-z][a-z0-9_]*`, `<= 64` chars) alongside `reproduces`, `method`, and `description`. When `check.reproduces` is absent and `check.text` is non-empty, it emits `DoneCriterionKind::UserConfirms` with sanitized `params.text` (`1..=400` chars). Both pass Solve's `validate_and_build_setup`.
4. **Linux VM disk space workaround (`CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`):**
   - Full workspace test builds with default debuginfo across ~50 integration test binaries exceeded `/overlay` free space (`79 GB`). Disabling debuginfo in test builds kept `target/debug` around ~11 GB without modifying `Cargo.toml` or `Cargo.lock`.
5. **Do-Not-Edit list respected 100%:**
   - Zero changes to `graph_layout.js`, `graph_view.js`, `ui_badge.js`, `quote_chip.js`, `crates/export/src/handoff*.rs`, `crates/desktop-server/src/handoff.rs`, `app.js`, `index.html`, `tool_profile.rs`, `tests/golden/{tool_defs,tool_profiles}.txt`, `Cargo.lock`, `resume.rs`, `routes_session.rs`, `turn_runner.rs`, `http_srv.rs`.

---

## 4. Merge Notes & Known Limitations

1. **Decision 13 stale re-map backend seam (X11 files — outside X12/X13 ownership):**
   - **Files:** `crates/desktop-server/src/idea_map_exchange.rs`, `crates/research-tools/src/idea_map_validate.rs`, `crates/research-tools/src/idea_verdict.rs` (owned by `exp-x9-x10-x11-read-verdict`, not modified by this brief).
   - **Behaviour today:** X12 renders the Decision 13 stale banner (`"Re-map against revision n"`) when `map_stale` is true (`map.idea_rev < statement.rev`) and posts `{ idea_rev, reuse_evidence: true }` to `POST /api/sessions/{id}/idea/map`, while `idea_view.rs` retains pre-revision evidence for stale-map quote chips and `idea_handoff.rs` preserves matched quotes for stale-map `"Investigate this paper"` actions. However, X11's `idea_map_exchange.rs` / `idea_map_validate.rs` / `idea_verdict.rs` still scope stage readiness and evidence strictly to `ev.seq > latest_statement_seq`, so calling `POST /api/sessions/{id}/idea/map` immediately after `revise` without re-running Sharpen/Triage/Read returns HTTP `409 Conflict`. A follow-up seam PR on X11's map/verdict modules is needed to honour `reuse_evidence: true` at zero reader cost.
2. **Exact `new_menu.js` flip once Explore left-rail enablement is signed off (after X18):**
   - In `crates/desktop-server/assets/new_menu.js` (`MODE_MENU_ENTRIES`, lines 24–32): change Explore's `built: false` to `built: true` and `title` to `'Explore an idea'` (and mirror `isSolveMenuEnabled` / `buildModeMenuEntries` if an opt-out flag is desired), and update `crates/desktop-server/tests/new_menu_and_rail.test.mjs:133-195` accordingly.
3. **Small shared-file edits in this branch:**
   - `crates/research-tools/src/lib.rs`, `crates/desktop-server/src/{lib.rs,routes_idea.rs,assets.rs,idea_view.rs,idea_view_map.rs}`, `crates/desktop-server/assets/{idea.js,idea_state.js,idea.css,api_idea.js}`, `crates/desktop-server/tests/idea_flow.rs`. Already merged cleanly against `origin/gemini/work/fixed` (`61fd8c2`).

---

## 5. Verification Results (Non-Root Linux, `RE_SEARCH_PYTHON` Unset)

All checks ran as non-root (`uid=1001(antigravity)`) on the final merged head (`4632985`):

| Check | Command | Outcome |
| --- | --- | --- |
| File length (<= 600L) | `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | **PASS** — `No touched source file is over 600 lines.` (max touched file: `idea_view.rs` at 593L) |
| JS syntax + Node tests | `for f in crates/desktop-server/assets/*.js; do node --check "$f" \|\| exit 1; done && node --test crates/desktop-server/tests/*.test.mjs` | **PASS** — **284 passed, 0 failed, 0 skipped** (includes X12 `idea_map.test.mjs`, X13 `idea_directions.test.mjs`, X15 `idea_chat_ui.test.mjs`, X16 `idea_net_ui.test.mjs`, X17 `library_ui.test.mjs`) |
| Workspace Clippy | `unset RE_SEARCH_PYTHON && CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` | **PASS** — **0 warnings, 0 errors** |
| Workspace Tests | `unset RE_SEARCH_PYTHON && CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 cargo test --workspace --locked --no-fail-fast` | **PASS** — **1,495 passed, 0 failed, 2 ignored** (pre-existing ignored tests only; 0 skipped/ignored tests added) |
| Golden & Lockfiles | `git diff --stat origin/gemini/work/fixed -- Cargo.lock crates/research-tools/tests/golden/` | **CLEAN** — 0 changes |

### Pre-existing / Not Run on Linux

- **Pre-existing ignored tests (2):** Existing workspace `#[ignore]` tests untouched by this brief; zero new `#[ignore]` or skipped tests were introduced.
- **Not compiled or run on Linux:** `cfg(windows)` code paths (e.g. AppContainer / JobObject sandbox tests in `worker`, Windows DPAPI vault paths in `chatgpt-auth`), and real Windows 11 Chromium browser E2E suites. No live network calls, no live model API calls, and no third-party paper text were used.
