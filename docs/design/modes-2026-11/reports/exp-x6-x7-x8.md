# Report: `exp-x6-x7-x8` — Exchange runner & Sharpen (X6), plan execution & `idea_search` (X7), Triage & abstract evidence (X8)

- **Brief:** `docs/design/modes-2026-11/agents/exp-x6-x7-x8.md`
- **Base commit (`origin/gemini/work/fixed`):** `6491394e28ef390f5f40e5fe56ac3aec8c1a97d7`
- **Branch pushed:** `gemini/modes/exp-x6-x7-x8`
- **Commits (in chronological order, excluding this report commit):**
  1. `ab11969` `X6: add single-job exchange runner and IdeaBrief recorder`
  2. `5353e9f` `X6: wire Explore Sharpen exchange, answer handler, and coverage projection`
  3. `de6569f` `X6: render Sharpened idea card and quote highlights in Explore UI`
  4. `391569c` `X7: freeze 4-tool explore/1 profile and add idea_search tool`
  5. `1acf2df` `X7: wire build_idea_router, SearchPlan execution via Router::try_call, and F12 coverage`
  6. `43b7588` `X7: mount read-only approval_card.js and live Where I looked UI`
  7. `ab1f54d` `X8: add record_idea_triage recorder and Rule C3 abstract evidence verification`
  8. `c72dcd1` `X8: wire Explore Triage exchange, routes, view projection, and E2E tests`
  9. `82ef723` `X8: render grouped triage list, reading-list ticks, and pause toggle`

---

## 1. What was implemented (package by package, with line counts)

All 39 touched/new source files are strictly `<= 600` lines (`python3 scripts/check_file_length.py --base origin/gemini/work/fixed` exits 0).

### Package X6 — Single-job exchange runner & Sharpen

| File | Lines | Role |
|---|---:|---|
| `crates/agent/src/exchange.rs` (new) | 565 | Generic single-job `run_exchange` over `Transport`: offers exactly one recorder `ToolSpec`, rejects any other tool call (Rule P2), runs 1 call + up to 2 repair attempts, checks F9 budget callback before every provider call (Rule P3), honors `CancelToken`, and records `idea.exchange` audit events with `request_blob`/`response_blob` on every attempt. |
| `crates/agent/src/exchange_tests.rs` (new) | 301 | Unit tests covering single-tool enforcement, Rule P2 rejection, Rule P3 budget stop, repair loop exhaustion, cancellation, and `idea.exchange` audit recording. |
| `crates/agent/src/lib.rs` (edit) | 103 | Exports `pub mod exchange;`. |
| `crates/research-tools/src/idea_brief.rs` (new) | 528 | `IdeaBrief`, `Perspective`, `record_idea_brief`, `validate_brief_input` (`#[serde(deny_unknown_fields)]`): enforces `restatement <= 1200`, `assumptions <= 8`, `subclaims[1..=4]` with `idea_quote` as an exact substring of `IdeaStatement.text`, `perspectives[2..=6]` requiring `exact` and `other_names` (`obstruction` optional), `questions[0..=2]`, sequential `sc-N`/`p-N` IDs, and named UI constant `UI_SHARPENED_IDEA_LABEL = "Sharpened idea"`. |
| `crates/research-tools/src/idea_brief_tests.rs` (new) | 143 | Unit tests for bounds, exact quote substring checks, required perspective kinds, and sequential ID assignment. |
| `crates/desktop-server/src/idea_exchanges.rs` (new) | 482 | Sharpen exchange runner (`run_sharpen_with_transport`, `handle_sharpen_op`), shared `emit_stage` & Rule C7 `wrap_untrusted_data` helpers, and CLI-route guard returning `"Explore exchanges are not supported on CLI routes yet (awaiting X18)"`. |
| `crates/desktop-server/src/idea_answer.rs` (new) | 383 | `POST /api/sessions/{id}/idea/answer` handler recording `idea.answer` for question answers, user edits to sharpened idea/perspectives, reading-list overrides (validated against active-revision candidates and `max_papers` cap, rejecting non-empty lists in Quick preset), and `pause_before_reading`. |
| `crates/desktop-server/src/idea_view_coverage.rs` (new) | 398 | Extracted from `idea_view.rs` to stay `<= 600` lines: `fold_answers` (scoped by `latest_statement_seq`, `latest_brief_seq`, `latest_triage_seq`), `build_where_i_looked`, `build_plan_approval`, and `build_map_graph_view`. |
| `crates/desktop-server/assets/idea.js` (edit) | 597 | Renders `"Sharpened idea"` card (never `"brief"`), subclaim quote highlights, editable queries/answers, batched search approval callbacks, and triage/reading-list/pause callbacks. |
| `crates/desktop-server/assets/idea_state.js` (edit) | 524 | Pure reducer & selectors (`highlightSubclaimQuotes`, `SHARPENED_IDEA_TITLE`, `TRIAGE_BASIS_LABEL`, SSE handlers for `idea.brief`, `idea.answer`, `idea.search_planned`, `idea.query_result`, `idea.search_finished`, `idea.triage`, `approval_requested`/`approval_resolved`, and `idea.stated` revision reset). |
| `crates/desktop-server/assets/api_idea.js` (edit) | 88 | Client API helpers for `/idea/answer`, `/idea/plan`, `/idea/search`, `/idea/triage`, `/idea/continue`, and `/idea/stop`. |

### Package X7 — Frozen 4-tool `explore/1` profile, `idea_search`, `build_idea_router`, SearchPlan execution & F12 coverage

| File | Lines | Role |
|---|---:|---|
| `crates/research-tools/src/idea_tools.rs` (new) | 493 | `idea_search` tool (`queries[1..=24]`, all 6 providers in schema from day one: `arxiv`, `inspire`, `semantic_scholar`, `crossref`, `openalex`, `theoremsearch`; `openalex`/`theoremsearch` return `status: "disabled"`; `q <= 300`; `relation: none | cites | cited_by + key`; `max_results 1..=25`; `EffectClass::NetworkRead`, `Ask`) and frozen `idea_fetch_papers` definition (`keys[1..=8]`, `NetworkRead`, `Ask`) with fail-closed placeholder handler (`Err("idea_fetch_papers: not yet wired (package X9)")`). |
| `crates/research-tools/src/idea_tools_tests.rs` (new) | 226 | Unit tests for schema bounds, disabled providers, fail-closed `idea_fetch_papers`, and `idea_search` execution via mock HTTP / F11 raw blob recording. |
| `crates/research-tools/src/tool_profile.rs` (edit) | 529 | Freezes `explore/1` (`BUILT_PROFILE_NAMES`, `RESERVED_PROFILE_NAMES = &[]`, `explore_v1()`, `lookup`, `built_profiles`, `register_profile`). |
| `crates/research-tools/src/tool_profile_tests.rs` (edit) | 298 | Covers `explore/1` tool list, Rule P7 isolation (zero `ExecuteCode`/`WriteWorkspace`), and policy check. |
| `crates/research-tools/tests/golden/tool_profiles.txt` (edit) | 154 | Appended ONLY `[profile explore/1]` (prior 4 profiles byte-for-byte untouched; `tool_defs.txt` untouched). |
| `crates/research-tools/tests/tool_profile_golden.rs` (edit) | 212 | Updated for `explore/1` now built (`RESERVED_PROFILE_NAMES` empty). |
| `crates/agent/src/resume_profile_tests.rs` (edit) | 549 | Updated reserved-profile test for `explore/1` moving from reserved to built. |
| `crates/cli-route/src/core_tools.rs` (edit) | 202 | Keeps `explore/1` out of legacy research-only CLI tool maps while keeping profile resolution exhaustive. |
| `crates/desktop-server/src/routers.rs` (edit) | 194 | Adds `build_idea_router` built from `tool_profile::lookup(PROFILE_EXPLORE_V1)` with `UiApprover::with_session_network_grant(true)` and zero `ExecuteCode`/`WriteWorkspace` tools (Rule P7). |
| `crates/desktop-server/src/idea_plan.rs` (new) | 475 | `POST /api/sessions/{id}/idea/plan` (`SearchPlan` assembly from sharpened perspectives or explicit queries, emitting `idea.search_planned` and `idea.stage(search, plan_approval)`). |
| `crates/desktop-server/src/idea_plan_search.rs` (new) | 497 | `POST /api/sessions/{id}/idea/search` executing strictly via `Router::try_call("host:idea:<session>", "idea_search", ...)` (Rule P5), single-provider retry preserving prior rounds' coverage, F11 raw-blob preservation, X1 candidate merge (`idea.candidates`), and F12 `search_coverage` + `known_paper_recall`. |
| `crates/desktop-server/src/idea_search.rs` (edit) | 523 | Preserves `abstract_ref` provenance cleanly during candidate merging and library matching. |
| `crates/desktop-server/src/session_mode.rs` (edit) | 269 | Keeps `DESKTOP_MODE_EXPLORE_ENABLED: bool = false` until X15 so `mode_shell.rs` stays green while `build_idea_router` looks up `explore/1` directly. |
| `crates/desktop-server/tests/mode_markers_r2.rs` (edit) | 447 | Added `idea_search -> &["literature.fetched"]` and `idea_fetch_papers -> &[]` to exhaustive `tool_emitted_event_kinds`. |
| `crates/desktop-server/assets/idea_search.js` (edit) | 507 | Mounts read-only `createApprovalCard` from `/assets/approval_card.js` for SearchPlan approval, renders live `"Where I looked"` with per-service status, `"Retry <service>"`, library summary, known-paper recall, and X8 grouped triage cards. |

Frozen `[profile explore/1]` hashes in `crates/research-tools/tests/golden/tool_profiles.txt`:
- `policy`: `v0.4-research`
- `def 0 idea_fetch_papers`: `c4ed4a30d0a366f31d4c0078317e7cca37f17046e6bc3202e52b77b05da11e68`
- `def 1 idea_search`: `704a514a1a9845d6ab4abb4316b7e4db93699782ae862540d0fd2ed600d93a88`
- `def 2 read_paper_lines`: `be20a5fededade9bcc553c1dc289f4941ad1d190285490d8533942cb2a030fbb`
- `def 3 search_paper_passages`: `6941ebc16a75d47f1c63659e9bd36871802d15dea5123a1dbf8a5c59fc4283aa`

### Package X8 — Triage, Rule C3 abstract evidence (`xa-<seq>`), Quick preset 0-papers path & grouped UI

| File | Lines | Role |
|---|---:|---|
| `crates/research-tools/src/idea_triage.rs` (new) | 535 | `Triage`, `record_idea_triage`, `validate_triage_input` (`#[serde(deny_unknown_fields)]`): enforces full candidate coverage up to `MAX_TRIAGE_ITEMS = 80`, verifies `abstract_quote` as a whitespace-normalised substring of F11 stored raw blobs ONLY when `abstract_ref.verbatim == true` (with cross-provider `alias_keys` fallback), rejects non-verbatim rebuilt abstracts (Rule C3), fills `(start, end)` offsets, emits deduplicated `evidence.abstract` (`xa-<seq>`, Rule S3), and enforces `reading_list == []` for Quick preset (`max_papers == 0`, Rule V2). |
| `crates/research-tools/src/idea_triage_tests.rs` (new) | 227 | Unit tests for Rule C3 verbatim vs fabricated vs rebuilt quotes, `xa-<seq>` deduplication, Quick preset enforcement, cross-provider DOI alias lookup, and partial-triage rejection. |
| `crates/research-tools/src/lib.rs` (edit) | 355 | Exports `idea_brief`, `idea_tools`, `idea_triage`. |
| `crates/research-tools/tests/idea_exchanges_validate.rs` (new) | 223 | Integration tests covering X6 `record_idea_brief`, X7 `explore/1` 4-tool profile isolation, and X8 `record_idea_triage` Rule C3 / Rule V2. |
| `crates/desktop-server/src/idea_exchanges_triage.rs` (new) | 529 | Triage exchange runner (`run_triage_with_transport`, `handle_triage_op`) over `agent::exchange::run_exchange`: Rule P1 gate, Rule P3 budget gate, Rule C7 `<CANDIDATES_DATA_{nonce}>` wrapping, `max_tokens = 8192`, F10 `SeenRole::Triaged` recording with canonical key resolution, and SSE emission. |
| `crates/desktop-server/src/idea.rs` (edit) | 574 | Updated `next_stage` and `validate_recorder_stage(RecordIdeaTriage)` for offline Search->Triage advancement, multi-round re-triage (`latest_triage_seq > latest_cands_seq`), and strict online post-Sharpen Search prerequisite. |
| `crates/desktop-server/src/idea_view.rs` (edit) | 482 | Projects `brief`/`sharpened_idea`, `question_answers`, `reading_list_override` (null when `max_papers == 0`), `pause_before_reading`, `plan_approval`, `where_i_looked`/`coverage`, `triage`, and `abstract_evidence` (`xa-<seq>`). |
| `crates/desktop-server/src/lib.rs` (edit) | 260 | Exports `idea_answer`, `idea_exchanges`, `idea_exchanges_triage`, `idea_plan`, `idea_plan_search`, `idea_view_coverage`. |
| `crates/desktop-server/src/routes_idea.rs` (edit) | 588 | Wires `sharpen`, `answer`, `plan`, `search`, `triage`, `continue` (dispatches Sharpen/Search/Triage; keeps 501 for X9–X13 stages), `stop` (also cancels active `UiApprover` prompts), and `revise`. |
| `crates/desktop-server/tests/idea_flow.rs` (edit) | 485 | Updated the 501 stub loop for newly wired ops (`sharpen`, `answer`, `plan`, `search`, `triage`, `continue`) while keeping unbuilt X9–X13 ops asserting 501. |
| `crates/desktop-server/tests/idea_search_triage.rs` (new) | 597 | E2E tests for X6 Sharpen repair/budget/P1/CLI-guard, X7 SearchPlan + `UiApprover` denial + session grant + single-provider retry + `/idea/stop` unblock + F12 coverage/recall, and X8 Triage after X7 Search. |
| `crates/desktop-server/tests/idea_triage_e2e.rs` (new) | 334 | E2E tests for X8 Triage repair loop, Rule C7 boundary verification, `xa-<seq>`, F10 `SeenRole::Triaged`, `/idea/answer` reading list + `pause_before_reading`, and Quick preset 0-papers path + 400 rejection of non-empty reading list. |
| `crates/desktop-server/tests/idea_search.test.mjs` (new) | 490 | Node DOM/unit tests for X6 Sharpened idea card & quote spans, X7 `approval_card.js` mounting & `"Where I looked"` retry/recall, X8 triage grouping, `"from abstract"` badges, collapsed `unrelated`, capped reading-list checkboxes, Quick preset disabled checkboxes, SSE sync, zero `innerHTML`, and zero Rule S8 / `"brief"` wording. |

---

## 2. Independent Carbon Audits & Fixes Applied

Each checkpoint underwent an independent read-only audit (`pro` tier) before commit/push:

1. **X6 Audit (9 findings — all fixed before pushing `de6569f`):**
   - Tightened Rule P1 pre-flight gate before emitting `idea.stage(sharpen, requested)` and emitted `Failed` on recorder error.
   - Enforced CLI-route guard before emitting `Requested` and checked session-level route consistency.
   - Fixed repeated-quote highlight spans in `highlightSubclaimQuotes` and reset `sharpened_idea` / `question_answers` on `idea.stated` SSE.
2. **X7 Audit (12 findings — all 10 in-scope findings fixed before pushing `43b7588`; 2 out-of-scope items documented in Merge notes):**
   - Fixed `idea.js` approval payload field name from `grant_session` to `approve_for_session` (matching `mode_shell.rs:273`).
   - Fixed `KnownPaperRecall` field names (`given`, `found_by_search`, `missed`) across `idea_search.js`, `idea_search.test.mjs`, and `idea_search_triage.rs`.
   - Made `/idea/stop` cancel pending `UiApprover` prompts immediately and set `auto_approve_tools: false` in `idea_search_triage.rs` so interactive denial/session-grant actually execute.
   - Split `idea_plan_search.rs` from `idea_plan.rs` to stay `<= 600` lines and preserved prior provider rows + original `abstract_ref.provider` on single-provider retries.
3. **X8 Audit (0 CRITICAL, 4 HIGH, 6 MEDIUM, 4 LOW — all in-scope findings fixed before pushing `82ef723`; 2 out-of-scope items documented in Merge notes):**
   - **HIGH #1:** Validated `reading_list` inside `ledger_write_lock` in `idea_answer.rs` against `effective_budget` (rejects non-empty list in Quick preset `max_papers == 0` with 400, enforces `len <= max_papers`, validates membership in active candidates), nulled `reading_list_override` in `idea_view.rs` when `max_papers == 0`, and enforced `maxPapers` cap + optimistic `readingSet` mutation in `idea_search.js`.
   - **HIGH #2:** Raised `MAX_TRIAGE_ITEMS` to `80` (matching Thorough `max_screened = 80`) and set `.with_max_tokens(8_192)` on Triage `ExchangeSpec`.
   - **HIGH #3:** Added `alias_keys: Vec<PaperKey>` to `TriageCandidateContext` (populated from `LibraryIds::from_paper_ids(&c.ids).canonical_aliases()`) so Rule C3 raw-blob lookup succeeds when a paper's canonical key (`arxiv:...`) differs from the blob's provider key (`doi:...`).
   - **HIGH #4:** Updated `idea.rs` `next_stage` (offline Search->Triage advancement and multi-round re-triage when `latest_cands_seq > latest_triage_seq`) and `validate_recorder_stage(RecordIdeaTriage)` (online post-Sharpen Search prerequisite); tightened `idea_exchanges_triage.rs` Rule P1 gate to `next_stage == Some(Triage) && stage_state != Requested`; emitted `ExploreStageState::Failed` on `record_idea_triage` error.
   - **MEDIUM #1/#2/#3/#5 & LOW #3/#4:** Scoped `fold_answers` reading-list overrides by `latest_triage_seq`, resolved canonical keys via `Library::resolve_key` before `record_seen`, required full candidate coverage + `reading_list` membership in `validate_triage_input`, hardcoded `TRIAGE_BASIS_LABEL` (`"from abstract"`) in `idea_search.js`, synced `triage.reading_list` on SSE `idea.answer` / cleared `reading_list_override` on SSE `idea.triage`, bumped schema key `maxLength` to 512, and filtered SSE emission by `ev.seq > prev_last_seq`.

---

## 3. Decisions Taken & Assumptions

- **Single-file ceiling splits:** Split `idea_view_coverage.rs` out of `idea_view.rs`, `idea_answer.rs` and `idea_exchanges_triage.rs` out of `idea_exchanges.rs`, `idea_plan_search.rs` out of `idea_plan.rs`, and `idea_triage_e2e.rs` out of `idea_search_triage.rs` so every touched/new file stays strictly `<= 600` lines (max 597).
- **Explore feature flag (`DESKTOP_MODE_EXPLORE_ENABLED = false`):** Kept `false` per brief section 2 until X15 enables conversation turns, so `tests/mode_shell.rs` remains green untouched while `build_idea_router` looks up `tool_profile::lookup(PROFILE_EXPLORE_V1)` directly.
- **`idea_fetch_papers` frozen placeholder:** Registered in `explore/1` with its exact `explore.md` 5.2 schema (`keys[1..=8]`, `NetworkRead`, `Ask`) and a fail-closed placeholder handler (`Err("idea_fetch_papers: not yet wired (package X9)")`), freezing `explore/1` hashes once with zero churn for X9/X15.
- **Full candidate coverage in Triage:** `validate_triage_input` requires the model to classify every supplied candidate (up to `min(max_screened, 80)`) and requires `reading_list` entries to be drawn from classified candidates, preventing silent candidate drops.
- **Assumption (Windows-only code):** `cfg(windows)` code paths (AppContainer sandbox, Windows desktop shell) could not be compiled or run on Linux; no Unix-only paths or shell-outs were introduced.

---

## 4. Merge Notes & Out-of-Scope Observations

1. **Existing files modified (localized, separate commits):**
   - `crates/desktop-server/tests/idea_flow.rs`: updated the 501-stub list to remove `sharpen`, `answer`, `plan`, `search`, `triage`, `continue` now that X6–X8 implement them (X9–X13 routes still assert 501).
   - `crates/research-tools/tests/golden/tool_profiles.txt`: appended `[profile explore/1]` only; `research/1`, `investigation/1`, `investigation/2`, `solve/1`, and `tool_defs.txt` are byte-for-byte unchanged.
   - `crates/desktop-server/src/session_mode.rs`: `DESKTOP_MODE_EXPLORE_ENABLED = false` (X15 flips this when wiring Explore turn loop).
   - `crates/desktop-server/tests/mode_markers_r2.rs`: registered `idea_search` and `idea_fetch_papers` in `tool_emitted_event_kinds`.
   - `crates/desktop-server/src/idea_tools.rs` placeholder for `idea_fetch_papers`: **X9** replaces the handler body inside `crates/research-tools/src/idea_tools.rs` without changing `idea_fetch_papers_def()` or its `def_hash`.
   - `crates/desktop-server/src/idea_exchanges.rs` CLI guard: **X18** replaces the CLI-route rejection once CLI-backed Explore exchanges are built.
2. **Pre-existing / parallel-lane gaps noticed during audits (NOT edited because files are outside this brief's ownership):**
   - `crates/desktop-server/src/routes_tools.rs` (`handle_literature`): legacy `/api/sessions/{id}/tools/literature` route does not yet reject `Mode::Explore` sessions (Rule P5 recommends routing Explore literature access exclusively through `idea_search`). Recommend adding a `Mode::Explore` guard in X15 or a follow-up coordinator cleanup.
   - `crates/desktop-server/assets/app.js`: main desktop shell SSE dispatcher does not yet forward `session_event` messages for `idea.*` / `evidence.abstract` into `mode_stages.js`'s Explore controller (the reducer in `idea_state.js` already handles them). Recommend wiring in X15/shell integration.
   - `crates/research-tools/src/library_rebuild.rs` (`handle_idea_seen`): reads flat `payload["paper_key"]` rather than iterating `idea.triage["items"][]` keys during full library DB rebuild from ledger. Live `record_seen` writes `SeenRole::Triaged` correctly during `run_triage_with_transport`, but rebuilding `library.db` solely from `idea.triage` events will miss those `SeenRole::Triaged` rows until `library_rebuild.rs` iterates `items[]`.
   - `crates/ledger/src/verify.rs` (`verify_all`): validates event hash chains and event-referenced blobs, and `evidence.abstract` stores `raw_blob` as a SHA-256 hex digest pointing to the immutable F11 raw blob in the ledger blob store.

---

## 5. Verification Commands & Measured Outcomes

Environment: Linux non-root (`uid=1001(antigravity)`), `RE_SEARCH_PYTHON` unset, `Cargo.lock` untouched.

| Command | Result |
|---|---|
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | **PASS** (exit 0 — `No touched source file is over 600 lines.`) |
| `for f in crates/desktop-server/assets/*.js; do node --check "$f" \|\| exit 1; done` | **PASS** (exit 0, all JS assets valid syntax) |
| `node --test crates/desktop-server/tests/*.test.mjs` | **PASS** (`235 passed, 0 failed, 0 skipped`) |
| `unset RE_SEARCH_PYTHON && cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` | **PASS** (exit 0, 0 warnings) |
| `unset RE_SEARCH_PYTHON && cargo clippy --locked -p agent -p research-tools -p desktop-server --lib --tests -- -D warnings -A clippy::manual_unwrap_or -A clippy::permissions_set_readonly_false -A clippy::items_after_test_module -A clippy::match_like_matches_macro -A clippy::unnecessary_map_or` | **PASS** (exit 0, 0 warnings) |
| `unset RE_SEARCH_PYTHON && cargo test --locked -p agent --lib exchange` | **PASS** (`4 passed, 0 failed`) |
| `unset RE_SEARCH_PYTHON && cargo test --locked -p agent --lib resume` | **PASS** (`13 passed, 0 failed`) |
| `unset RE_SEARCH_PYTHON && cargo test --locked -p agent --test resume_loop` | **PASS** (`6 passed, 0 failed`) |
| `unset RE_SEARCH_PYTHON && cargo test --locked -p research-tools --test tool_def_golden --test tool_profile_golden --test idea_exchanges_validate` | **PASS** (`3 + 3 + 3 = 9 passed, 0 failed`) |
| `unset RE_SEARCH_PYTHON && cargo test --locked -p desktop-server --test idea_flow --test idea_search_triage --test idea_triage_e2e --test mode_shell --test mode_markers_r2 --test eval_set_r` | **PASS** (`4 + 4 + 2 + 4 + 3 + 5 = 22 passed, 0 failed`) |
| `unset RE_SEARCH_PYTHON && cargo test --workspace --locked --no-fail-fast` | **PASS** (`1234 passed, 0 failed, 2 pre-existing ignored`) |

### Pre-existing / Platform Limitations
- **2 pre-existing ignored tests** in the workspace (unchanged from base commit `6491394e`); zero new `#[ignore]` or skipped tests were added.
- **Not run on Linux:** Windows `cfg(windows)` compilation targets (`appcontainer_sandbox.rs`, `python_worker.rs` Windows-gated tests — 0 tests compiled on Linux) and real browser / vendor CLI / live external network calls (all tests run against local loopback mocks and scripted transports).
