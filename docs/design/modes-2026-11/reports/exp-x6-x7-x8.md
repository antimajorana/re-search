# Report: `exp-x6-x7-x8` — Exchange runner & Sharpen (X6), plan execution & `idea_search` (X7), Triage & abstract evidence (X8)

- **Brief:** `docs/design/modes-2026-11/agents/exp-x6-x7-x8.md`
- **Original checkpoint base:** `6491394e28ef390f5f40e5fe56ac3aec8c1a97d7`
- **Merged base commit (`origin/gemini/work/fixed`):** `2104137a228ae7948ee1d5ef6228d2e30304b461` (merged `--no-ff` in `1de4f56` onto prior tip `6463ef2`)
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
  10. `6463ef2` `docs: add final report for exp-x6-x7-x8`
  11. `1de4f56` `Merge origin/gemini/work/fixed into work/exp-x6-x7-x8`
  12. `83356b5` `fix(explore): align OpenAlex provider config, coverage, and CLI gates after work/fixed merge`
  13. `dc013e4` `fix(explore): harden continue resume, revision isolation, F2 alias dedup, and guards`
  14. `25b3737` `fix(desktop-ui): prevent plan-approval deadlock, restrict continue budget bumps, and sync SSE state`

---

## 1. What was implemented (package by package, with line counts)

All **41** touched/new files vs `origin/gemini/work/fixed` (`2104137`) are strictly `<= 600` lines (`python3 scripts/check_file_length.py --base origin/gemini/work/fixed` exits 0; maximum is `597` lines).

### Package X6 — Single-job exchange runner & Sharpen

| File | Lines | Role |
|---|---:|---|
| `crates/agent/src/exchange.rs` (new) | 565 | Generic single-job `run_exchange` over `Transport`: offers exactly one recorder `ToolSpec`, rejects any other tool call (Rule P2), runs 1 call + up to 2 repair attempts, checks F9 budget callback before every provider call (Rule P3), honors `CancelToken`, and records `idea.exchange` audit events with `request_blob`/`response_blob` on every attempt. |
| `crates/agent/src/exchange_tests.rs` (new) | 301 | Unit tests covering single-tool enforcement, Rule P2 rejection, Rule P3 budget stop, repair loop exhaustion, cancellation, and `idea.exchange` audit recording. |
| `crates/agent/src/lib.rs` (edit) | 119 | Exports `pub mod exchange;` (merged cleanly alongside `delegate_blind` and `dispatch`). |
| `crates/research-tools/src/idea_brief.rs` (new) | 528 | `IdeaBrief`, `Perspective`, `record_idea_brief`, `validate_brief_input` (`#[serde(deny_unknown_fields)]`): enforces `restatement <= 1200`, `assumptions <= 8`, `subclaims[1..=4]` with `idea_quote` as an exact substring of `IdeaStatement.text`, `perspectives[2..=6]` requiring `exact` and `other_names` (`obstruction` optional), `questions[0..=2]`, sequential `sc-N`/`p-N` IDs, and named UI constant `UI_SHARPENED_IDEA_LABEL = "Sharpened idea"`. |
| `crates/research-tools/src/idea_brief_tests.rs` (new) | 143 | Unit tests for bounds, exact quote substring checks, required perspective kinds, and sequential ID assignment. |
| `crates/desktop-server/src/idea_exchanges.rs` (new) | 482 | Sharpen exchange runner (`run_sharpen_with_transport`, `handle_sharpen_op`), shared `emit_stage` & Rule C7 `wrap_untrusted_data` helpers, and CLI-route guard returning `"Explore exchanges are not supported on CLI routes yet (awaiting X18)"`. |
| `crates/desktop-server/src/idea_answer.rs` (new) | 388 | `POST /api/sessions/{id}/idea/answer` handler recording `idea.answer` for question answers, user edits to sharpened idea/perspectives, reading-list overrides (validated against active-revision candidates and `max_papers` cap, rejecting non-empty lists in Quick preset), and `pause_before_reading`; rejects submissions with `409 Conflict` while an Explore run is active. |
| `crates/desktop-server/src/idea_view_coverage.rs` (new) | 403 | Extracted from `idea_view.rs` to stay `<= 600` lines: `fold_answers` (scoped by `latest_statement_seq`, `latest_brief_seq`, `latest_triage_seq`), revision-scoped `build_where_i_looked` (OpenAlex allowlisted per X19, only TheoremSearch hard-disabled), `build_plan_approval`, and `build_map_graph_view`. |
| `crates/desktop-server/assets/idea.js` (edit) | 593 | Renders `"Sharpened idea"` card (never `"brief"`), subclaim quote highlights, editable queries/answers, non-blocking search approval callbacks checking `sendApproval` results, `stopped_at_cap`-gated Continue budget bumps, and triage/reading-list/pause callbacks. |
| `crates/desktop-server/assets/idea_state.js` (edit) | 557 | Pure reducer & selectors (`highlightSubclaimQuotes`, `SHARPENED_IDEA_TITLE`, `TRIAGE_BASIS_LABEL`, SSE handlers for `idea.brief`, `idea.answer`, `idea.search_planned`, `idea.query_result`, per-`qid`-merging `idea.search_finished`, `idea.budget_raised`, `idea.triage`, `approval_requested`/`approval_resolved`, and full revision reset on `idea.stated`). |
| `crates/desktop-server/assets/api_idea.js` (edit) | 88 | Client API helpers for `/idea/answer`, `/idea/plan`, `/idea/search`, `/idea/triage`, `/idea/continue`, and `/idea/stop`. |

### Package X7 — Frozen 4-tool `explore/1` profile, `idea_search`, `build_idea_router`, SearchPlan execution & F12 coverage

| File | Lines | Role |
|---|---:|---|
| `crates/research-tools/src/idea_tools.rs` (new) | 498 | `idea_search` tool (`queries[1..=24]`, all 6 providers in schema from day one: `arxiv`, `inspire`, `semantic_scholar`, `crossref`, `openalex`, `theoremsearch`; non-allowlisted `theoremsearch` returns `status: "disabled"`, while allowlisted `openalex` runs via X19 F11/F3 with `openalex_api_key`; `q <= 300`; `relation: none \| cites \| cited_by + key`; `max_results 1..=25`; `EffectClass::NetworkRead`, `Ask`) and frozen `idea_fetch_papers` definition (`keys[1..=8]`, `NetworkRead`, `Ask`) with fail-closed placeholder handler (`Err("idea_fetch_papers: not yet wired (package X9)")`). |
| `crates/research-tools/src/idea_tools_tests.rs` (new) | 311 | Unit tests for schema bounds, disabled `theoremsearch`, fail-closed `idea_fetch_papers`, S2 `x-api-key` Rule P8, and OpenAlex `Authorization: Bearer` Rule P8 + rebuilt `verbatim: false` Rule C3. |
| `crates/research-tools/src/tool_profile.rs` (edit) | 529 | Freezes `explore/1` (`BUILT_PROFILE_NAMES`, `RESERVED_PROFILE_NAMES = &[]`, `explore_v1()`, `lookup`, `built_profiles`, `register_profile`). |
| `crates/research-tools/src/tool_profile_tests.rs` (edit) | 298 | Covers `explore/1` tool list, Rule P7 isolation (zero `ExecuteCode`/`WriteWorkspace`), and policy check. |
| `crates/research-tools/tests/golden/tool_profiles.txt` (edit) | 154 | Appended ONLY `[profile explore/1]` (prior 4 profiles byte-for-byte untouched; `tool_defs.txt` untouched). |
| `crates/research-tools/tests/tool_profile_golden.rs` (edit) | 212 | Updated for `explore/1` now built (`RESERVED_PROFILE_NAMES` empty). |
| `crates/agent/src/resume_profile_tests.rs` (edit) | 587 | Updated reserved-profile test for `explore/1` moving from reserved to built. |
| `crates/cli-route/src/core_tools.rs` (edit) | 298 | Updated `core_tool_set_for_profile` and `core_tool_ids_for` unit tests now that `explore/1` is a built 5-tool MCP profile while `"unknown/99"` still falls back fail-closed. |
| `crates/desktop-server/src/cli_turn_setup.rs` (edit) | 584 | Keeps `resolve_cli_turn_profile` fail-closed for `Mode::Explore` while `!DESKTOP_MODE_EXPLORE_ENABLED` (until X18) and forwards `RE_SEARCH_OPENALEX_BASE` in `core_env_for_profile`. |
| `crates/desktop-server/src/routers.rs` (edit) | 223 | Adds `resolve_explore_provider_config` (hydrating X19 `lit_settings::resolve_provider_config` + hermetic `SEMANTIC_SCHOLAR_API_KEY`) and `build_idea_router` built from `tool_profile::lookup(PROFILE_EXPLORE_V1)` with `UiApprover::with_session_network_grant(true)` and zero `ExecuteCode`/`WriteWorkspace` tools (Rule P7). |
| `crates/desktop-server/src/idea_plan.rs` (new) | 516 | `POST /api/sessions/{id}/idea/plan` (`SearchPlan` assembly with offline guard, emitting `idea.search_planned` and `idea.stage(search, plan_approval)`) and multi-round F2 canonical-alias candidate unification (`collect_canonical_keys` + `PaperRef::from_ids_or`). |
| `crates/desktop-server/src/idea_plan_search.rs` (new) | 505 | `POST /api/sessions/{id}/idea/search` executing strictly via `Router::try_call("host:idea:<session>", "idea_search", ...)` (Rule P5), offline pre-plan guard, partial-provider retry skipping already-`ok` `qid`s, F11 raw-blob preservation, X1 candidate merge (`idea.candidates`), and F12 `search_coverage` + `known_paper_recall`. |
| `crates/desktop-server/src/idea_search.rs` (edit) | 570 | Adds `run_first_search_with_config` / `run_first_search_with_gate_and_config` (hydrating `ProviderConfig` from X19 `lit_settings` + env) and preserves `abstract_ref` provenance cleanly during candidate merging and library matching. |
| `crates/desktop-server/src/session_mode.rs` (edit) | 274 | Keeps `pub(crate) const DESKTOP_MODE_EXPLORE_ENABLED: bool = false` until X15 so `mode_shell.rs` and `cli_turn_tests.rs` stay green while `build_idea_router` looks up `explore/1` directly. |
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
| `crates/research-tools/src/lib.rs` (edit) | 370 | Exports `idea_brief`, `idea_tools`, `idea_triage` (merged cleanly with `lit_openalex`). |
| `crates/research-tools/tests/idea_exchanges_validate.rs` (new) | 223 | Integration tests covering X6 `record_idea_brief`, X7 `explore/1` 4-tool profile isolation, and X8 `record_idea_triage` Rule C3 / Rule V2. |
| `crates/desktop-server/src/idea_exchanges_triage.rs` (new) | 530 | Triage exchange runner (`run_triage_with_transport`, `handle_triage_op`) over `agent::exchange::run_exchange`: Rule P1 gate, Rule P3 budget gate, Rule C7 `<CANDIDATES_DATA_{nonce}>` wrapping, `max_tokens = 8192`, F10 `SeenRole::Triaged` recording with canonical key resolution, HTTP 400 on no candidates, and SSE emission. |
| `crates/desktop-server/src/idea.rs` (edit) | 574 | Updated `next_stage` and `validate_recorder_stage(RecordIdeaTriage)` for offline Search->Triage advancement, multi-round re-triage (`latest_triage_seq > latest_cands_seq`), and strict online post-Sharpen Search prerequisite. |
| `crates/desktop-server/src/idea_view.rs` (edit) | 489 | Projects `brief`/`sharpened_idea`, `question_answers`, `reading_list_override` (null when `max_papers == 0`), `pause_before_reading`, `plan_approval`, `where_i_looked`/`coverage`, `triage`, and `abstract_evidence` (`xa-<seq>`); resets all revision-scoped accumulators on `idea.stated`. |
| `crates/desktop-server/src/lib.rs` (edit) | 271 | Exports `idea_answer`, `idea_exchanges`, `idea_exchanges_triage`, `idea_plan`, `idea_plan_search`, `idea_view_coverage`, and `build_idea_router` (merged cleanly with `routes_export` and `lit_settings`). |
| `crates/desktop-server/src/routes_idea.rs` (edit) | 591 | Wires `sharpen`, `answer`, `plan`, `search`, `triage`, `continue` (locks `ledger_write_lock` for `idea.budget_raised` and immediately chains into paused Sharpen/Search/Triage unless `resume: false`; keeps 501 for X9–X13 stages), `stop` (also cancels active `UiApprover` prompts), and `revise` (checks `is_turn_running` inside `ledger_write_lock`). |
| `crates/desktop-server/tests/idea_flow.rs` (edit) | 485 | Updated the 501 stub loop for newly wired ops (`sharpen`, `answer`, `plan`, `search`, `triage`, `continue`) while keeping unbuilt X9–X13 ops asserting 501. |
| `crates/desktop-server/tests/idea_search_triage.rs` (new) | 597 | E2E tests for X6 Sharpen repair/budget/P1/CLI-guard, X7 SearchPlan + `UiApprover` denial + session grant + single-provider retry + `/idea/stop` unblock + F12 coverage/recall, and X8 Triage after X7 Search. |
| `crates/desktop-server/tests/idea_triage_e2e.rs` (new) | 509 | E2E tests for X8 Triage repair loop, Rule C7 boundary verification, `xa-<seq>`, F10 `SeenRole::Triaged`, `/idea/answer` reading list + `pause_before_reading`, Quick preset 0-papers path + 400 rejection of non-empty reading list, revision isolation, offline plan/search 400 guards, `/idea/continue` budget raise, and multi-round F2 canonical-alias dedup. |
| `crates/desktop-server/tests/idea_search.test.mjs` (new) | 547 | Node DOM/unit tests for X6 Sharpened idea card & quote spans, X7 `approval_card.js` mounting & `"Where I looked"` retry/recall, X8 triage grouping, `"from abstract"` badges, collapsed `unrelated`, capped reading-list checkboxes, Quick preset disabled checkboxes, SSE sync (including revision reset, per-`qid` retry merge, and `idea.budget_raised`), zero `innerHTML`, and zero Rule S8 / `"brief"` wording. |

---

## 2. Independent Hard Audits & Fixes Applied

Every checkpoint and the post-merge integration underwent an independent read-only audit (`pro` tier):

1. **X6 Audit (`de6569f`, 9 findings — all resolved):**
   - Tightened Rule P1 pre-flight gate before emitting `idea.stage(sharpen, requested)` and emitted `Failed` on recorder error.
   - Enforced CLI-route guard before emitting `Requested` and checked session-level route consistency.
   - Fixed repeated-quote highlight spans in `highlightSubclaimQuotes` and reset `sharpened_idea` / `question_answers` on `idea.stated` SSE.
2. **X7 Audit (`43b7588`, 12 findings — 10 in-scope resolved, 2 out-of-scope noted in §4):**
   - Fixed `idea.js` approval payload field name (`approve_for_session`), `KnownPaperRecall` field names (`given`, `found_by_search`, `missed`), and `/idea/stop` immediate cancellation of pending `UiApprover` prompts.
   - Split `idea_plan_search.rs` from `idea_plan.rs` to stay `<= 600` lines and preserved prior provider rows + original `abstract_ref.provider` on single-provider retries.
3. **X8 Audit (`82ef723`, 0 CRITICAL, 4 HIGH, 6 MEDIUM, 4 LOW — all in-scope resolved):**
   - Validated `reading_list` inside `ledger_write_lock` in `idea_answer.rs` against `effective_budget` (rejects non-empty list in Quick preset `max_papers == 0` with 400, enforces `len <= max_papers`, validates candidate membership), nulled `reading_list_override` in `idea_view.rs` when `max_papers == 0`, and enforced `maxPapers` cap in `idea_search.js`.
   - Raised `MAX_TRIAGE_ITEMS` to `80` with `.with_max_tokens(8_192)`, added `alias_keys` for Rule C3 cross-provider raw-blob lookup, tightened `next_stage`/`validate_recorder_stage(RecordIdeaTriage)` and Rule P1 gating, scoped `fold_answers` by `latest_triage_seq`, and hardcoded `TRIAGE_BASIS_LABEL = "from abstract"`.
4. **Post-Merge Hard Audit after merging `origin/gemini/work/fixed` (`2104137`, PRs #58–#70 including `exp-x19-openalex` and `solve-s15-cli-parity`) — resolved in `83356b5`, `dc013e4`, `25b3737`:**
   - **C1 (`83356b5`):** Updated `crates/cli-route/src/core_tools.rs` `core_tool_ids_for_falls_back_cleanly_on_invalid_or_unbuilt_profile` for built 5-tool `explore/1` while keeping `"unknown/99"` fallback strict.
   - **C2 (`83356b5`):** Guarded `resolve_cli_turn_profile` in `crates/desktop-server/src/cli_turn_setup.rs` behind `!DESKTOP_MODE_EXPLORE_ENABLED` so CLI Explore turns remain fail-closed until X18 (keeping `cli_turn_tests.rs` untouched and green) and forwarded `RE_SEARCH_OPENALEX_BASE` in `core_env_for_profile`.
   - **H1 & M3 (`83356b5`):** Hydrated `ProviderConfig` from X19 `lit_settings::resolve_provider_config(state)` + hermetic `SEMANTIC_SCHOLAR_API_KEY` in both `build_idea_router` (`routers.rs`) and pre-Sharpen first search (`idea_search.rs` / `routes_idea.rs`); updated `idea_tools.rs` and `idea_view_coverage.rs` to treat allowlisted `openalex` cleanly (only `theoremsearch` remains hard-disabled); added positive loopback OpenAlex Rule P8 (`Authorization: Bearer`) + Rule C3 (`verbatim: false`) test in `idea_tools_tests.rs`.
   - **H2 (`dc013e4`, `25b3737`):** Fixed `/idea/continue` infinite budget-bump loop by locking `ledger_write_lock` for `idea.budget_raised` and immediately chaining into paused Sharpen/Search/Triage (unless `resume: false` or unbuilt X9–X13 stage), and restricted `idea.js` `handleContinueClick` budget bumps to `stopped_at_cap` pauses (not user Stop).
   - **H3 (`25b3737`, UI-owned scope):** Fixed `idea.js` `onAllowGrant` to verify `sendApproval` response when `call_id` is present and to trigger `'search'` asynchronously without deadlocking `approval_card.js` when `!call_id`; updated `idea_state.js` to merge `where_i_looked.queries` by `qid` on `idea.search_finished` and fold `idea.budget_raised`. (`app.js` is 932 lines and outside brief ownership; left untouched per `RULES.md` and noted in §4.)
   - **H4 (`dc013e4`, `25b3737`):** Cleared all revision-scoped accumulators on `idea.stated` in `idea_view.rs` and `idea_state.js`, and scoped F12 `compute_coverage_from_events` in `idea_view_coverage.rs` to `idea.started` + events `>= latest_stmt_seq`.
   - **H5 (`dc013e4`):** Unified multi-round candidate deduplication in `idea_plan.rs` across all F2 canonical aliases (`collect_canonical_keys` + `PaperRef::from_ids_or`).
   - **M1 / M2 / Retry UX (`dc013e4`):** Added offline guards before planning in `handle_plan_op` and `handle_search_op`, moved `is_turn_running` inside `ledger_write_lock` in `handle_revise` and `handle_answer_op`, skipped already-`ok` `qid`s on partial-provider retry, and mapped HTTP 400 (`no candidates available`) / HTTP 500 (`router ledger error`) accurately.

---

## 3. Decisions Taken & Assumptions

- **Single-file ceiling splits:** Split `idea_view_coverage.rs` out of `idea_view.rs`, `idea_answer.rs` and `idea_exchanges_triage.rs` out of `idea_exchanges.rs`, `idea_plan_search.rs` out of `idea_plan.rs`, and `idea_triage_e2e.rs` out of `idea_search_triage.rs` so every touched/new file stays strictly `<= 600` lines (max 597).
- **Explore feature flag (`DESKTOP_MODE_EXPLORE_ENABLED = false`):** Kept `false` per brief section 2 until X15 enables conversation turns (and reused in `cli_turn_setup.rs` until X18 enables CLI Explore turns), so `tests/mode_shell.rs` and `cli_turn_tests.rs` remain green untouched while `build_idea_router` looks up `tool_profile::lookup(PROFILE_EXPLORE_V1)` directly.
- **`idea_fetch_papers` frozen placeholder:** Registered in `explore/1` with its exact `explore.md` 5.2 schema (`keys[1..=8]`, `NetworkRead`, `Ask`) and a fail-closed placeholder handler (`Err("idea_fetch_papers: not yet wired (package X9)")`), freezing `explore/1` hashes once with zero churn for X9/X15.
- **ProviderConfig binding without changing `ProfileHostTools`:** Rather than adding a field to `ProfileHostTools` (which has 10+ struct-literal callers across sibling crates), `build_idea_router` calls `register_profile` to verify exact profile hashes and then re-registers `idea_search_def()` with the hydrated `lit_settings` + hermetic `SEMANTIC_SCHOLAR_API_KEY` handler, keeping `def_hash` byte-for-byte identical.
- **Full candidate coverage in Triage:** `validate_triage_input` requires the model to classify every supplied candidate (up to `min(max_screened, 80)`) and requires `reading_list` entries to be drawn from classified candidates, preventing silent candidate drops.
- **Machine test dependencies (`RULES.md`):** Installed `sympy 1.14.0`, `mpmath 1.3.0`, and `numpy 2.5.5` into the VM's Python environment (no project dependency or `Cargo.lock` change) so Solve's SymPy-backed tests (`delegate_blind`, `verifier_loop`, `solve_check_routes`) execute and pass on Linux.
- **Assumption (Windows-only code):** `cfg(windows)` code paths (AppContainer sandbox, Windows desktop shell) could not be compiled or run on Linux; no Unix-only paths or shell-outs were introduced.

---

## 4. Merge Notes & Out-of-Scope Observations

1. **Existing files modified (localized, separate commits; all `<= 600` lines):**
   - `crates/research-tools/tests/golden/tool_profiles.txt`: appended `[profile explore/1]` only; `research/1`, `investigation/1`, `investigation/2`, `solve/1`, and `tool_defs.txt` are byte-for-byte unchanged.
   - `crates/desktop-server/src/session_mode.rs`: `pub(crate) const DESKTOP_MODE_EXPLORE_ENABLED = false` (X15 flips this when wiring Explore Conversation turns).
   - `crates/desktop-server/src/cli_turn_setup.rs`: minimal 6-line fail-closed guard in `resolve_cli_turn_profile` keeping CLI Explore turns refused until X18, plus 1-line `RE_SEARCH_OPENALEX_BASE` propagation (`cli_turn.rs` untouched).
   - `crates/cli-route/src/core_tools.rs`: updated `profile_set_builds_known_profiles_and_rejects_unbuilt` and `core_tool_ids_for_falls_back_cleanly_on_invalid_or_unbuilt_profile` unit tests for built 5-tool `explore/1`.
   - `crates/desktop-server/src/idea.rs`: updated `next_stage` and `validate_recorder_stage(RecordIdeaTriage)` for offline Search->Triage advancement and multi-round re-triage.
   - `crates/desktop-server/src/idea_search.rs`: added `run_first_search_with_config` / `run_first_search_with_gate_and_config` for X19 `ProviderConfig` hydration and exported `lit_paper_to_raw_hit` / `annotate_and_record_library` for `idea_plan_search.rs`.
   - `crates/desktop-server/tests/idea_flow.rs`: updated the 501-stub list to remove `sharpen`, `answer`, `plan`, `search`, `triage`, `continue` now that X6–X8 implement them (X9–X13 routes still assert 501).
   - `crates/desktop-server/tests/mode_markers_r2.rs`: registered `idea_search` and `idea_fetch_papers` in `tool_emitted_event_kinds`.
   - `crates/research-tools/src/idea_tools.rs` placeholder for `idea_fetch_papers`: **X9** replaces the handler body without changing `idea_fetch_papers_def()` or its `def_hash`.
   - `crates/desktop-server/src/idea_exchanges.rs` CLI guard: **X18** replaces the CLI-route rejection once CLI-backed Explore exchanges are built.
2. **Pre-existing / out-of-scope bugs & gaps noticed during audits (NOT edited because files are outside this brief's ownership or > 600 lines):**
   - `crates/desktop-server/assets/app.js` (932 lines — over 600-line ceiling): main desktop shell SSE dispatcher does not yet forward `session_event`, `approval_requested`, or `approval_resolved` messages into `mode_stages.js`'s active Explore controller (`idea_state.js` reducer already handles them). Wire in X15/shell integration.
   - `crates/mcp-server/src/main.rs:198-206`: resolves `legacy_profile_name_for_events(&events)` (`"explore/1"`) directly; X18 owns full CLI/MCP Explore integration.
   - `crates/desktop-server/src/routes_tools.rs:373-438` (`handle_literature`): legacy `/api/sessions/{id}/tools/literature` route does not yet reject `Mode::Explore` sessions (Rule P5 recommends routing Explore literature access exclusively through `idea_search`).
   - `crates/research-tools/src/lit_abstracts.rs:330` (`extract_abstract_from_blob`): passes `&QueryRelation::None` instead of the recorded query relation, which can miss abstract extraction on Crossref single-work `Cites` payloads.
   - `crates/research-tools/src/library_rebuild.rs:373-393` (`handle_idea_seen`): reads flat `payload["paper_key"]` rather than iterating `idea.triage["items"][]` keys during full library DB rebuild from ledger.
   - `crates/research-tools/src/library_rebuild_lit.rs:54,91-143`: lacks an OpenAlex raw-blob parser branch (falls back to event-payload `hits[]`, as also noted in `reports/exp-x19-openalex.md`).
   - `crates/desktop-server/tests/api_auth.rs:68-123`: route-wide auth matrix does not yet list `/api/idea/preview`, `/api/sessions/{id}/idea/*`, or `/api/settings/literature` (each is covered by `idea_flow.rs` / `lit_settings.rs`).

---

## 5. Verification Commands & Measured Outcomes

Environment: Linux x86_64 non-root (`uid=1001(antigravity)`), `RE_SEARCH_PYTHON` unset, `Cargo.lock` and all 13 Do-Not-Edit files untouched.

| Command | Result |
|---|---|
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | **PASS** (exit 0 — `No touched source file is over 600 lines.`, max 597) |
| `git diff --stat origin/gemini/work/fixed -- Cargo.lock crates/research-tools/tests/golden/tool_defs.txt crates/desktop-server/assets/approval_card.js crates/desktop-server/src/assets.rs crates/desktop-server/tests/mode_shell.rs crates/agent/src/resume.rs crates/agent/src/runner.rs crates/desktop-server/src/cli_turn.rs crates/research-tools/src/paper_passages.rs crates/research-tools/src/http_client.rs crates/research-tools/src/net_rate.rs` | **PASS** (empty diff — all protected files untouched) |
| `for f in crates/desktop-server/assets/*.js; do node --check "$f" \|\| exit 1; done` | **PASS** (exit 0, all JS assets valid syntax) |
| `node --test crates/desktop-server/tests/*.test.mjs` | **PASS** (`249 passed, 0 failed, 0 skipped`) |
| `unset RE_SEARCH_PYTHON && cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` | **PASS** (exit 0, 0 warnings) |
| `unset RE_SEARCH_PYTHON && cargo clippy --locked -p agent -p research-tools -p desktop-server --lib --tests -- -D warnings -A clippy::manual_unwrap_or -A clippy::permissions_set_readonly_false -A clippy::items_after_test_module -A clippy::match_like_matches_macro -A clippy::unnecessary_map_or` | **PASS** (exit 0, 0 warnings) |
| `unset RE_SEARCH_PYTHON && cargo test --locked -p agent --lib exchange` | **PASS** (`4 passed, 0 failed`) |
| `unset RE_SEARCH_PYTHON && cargo test --locked -p agent --lib resume` | **PASS** (`17 passed, 0 failed`) |
| `unset RE_SEARCH_PYTHON && cargo test --locked -p agent --test resume_loop` | **PASS** (`6 passed, 0 failed`) |
| `unset RE_SEARCH_PYTHON && cargo test --locked -p research-tools --test tool_def_golden --test tool_profile_golden --test idea_exchanges_validate` | **PASS** (`3 + 3 + 3 = 9 passed, 0 failed`) |
| `unset RE_SEARCH_PYTHON && cargo test --locked -p desktop-server --test idea_flow --test idea_search_triage --test idea_triage_e2e --test mode_shell --test mode_markers_r2 --test eval_set_r` | **PASS** (`4 + 4 + 4 + 4 + 3 + 5 = 24 passed, 0 failed`) |
| `unset RE_SEARCH_PYTHON && cargo test --workspace --locked --no-fail-fast -- --quiet` | **PASS** (`1336 passed, 0 failed, 2 pre-existing ignored` across `175` targets) |

### Pre-existing / Platform Limitations
- **2 pre-existing ignored tests** (`#[ignore = "live tier: ..."]` in `crates/agent/tests/eval_harness.rs:468,481`, unchanged from `origin/gemini/work/fixed`); zero new `#[ignore]` or skipped tests were added.
- **Known pre-existing clippy lint:** `manual_unwrap_or` in `crates/desktop-server/src/evidence.rs:744` (file > 600 lines, untouched, suppressed via `-A clippy::manual_unwrap_or` per `RULES.md`).
- **Not run on Linux:** Windows `cfg(windows)` compilation targets (`appcontainer_sandbox.rs`, `python_worker.rs` Windows-gated tests, `re-search-app` WebView2 shell) and real browser / vendor CLI / live external network calls (all tests run against local loopback mocks and scripted transports).
