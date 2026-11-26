# Brief exp-x6-x7-x8-search-triage — Exchange runner & Sharpen (X6), plan execution & `idea_search` (X7), Triage & abstract evidence (X8)

## 1. Packages and governing design sections

- **Packages (in order):** `X6` (Exchange runner `agent/src/exchange.rs` & Sharpen) -> `X7` (Plan execution, `idea_search` in `explore/1`, batched approval, F12 coverage) -> `X8` (Triage & abstract evidence `evidence.abstract`).
- **Governing design:**
  - `docs/design/modes-2026-11/00-overview.md`: sections 2.2 (tool profiles, `explore/1`), 2.4 (network gate & approval rule), 2.5 (`evidence.abstract` `xa-<seq>`), 2.8 (F11), 2.9 (F12 coverage/gate), 3.1 rows 12 & 13 (policy per profile; shared approval card & prompting outside a running turn), 3.2 rows 17, 18, 20, 21, 24 (`exchange`, `"Sharpened idea"`, `"matched"`, `"Model judges:"`/`"(model)"`), 3.3, 8.2/8.3, and 9 (shared approval card open item).
  - `docs/design/modes-2026-11/explore.md`: sections 3 (Sharpen, Search, Triage, Quick preset), 4.1 (`IdeaBrief`, `Perspective`, `SearchPlan`, `Triage`, `Evidence` abstract, `Coverage`), 4.2, 5.1–5.2 (recorders, `explore/1` tools), 6 (rules C3, C7, P1–P8, S2, S3, S8), 7–8, 9 (rows X6, X7, X8), 12 (risk 8), and 13 (decisions 2, 4, 5).
  - `docs/design/modes-2026-11/agents/RULES.md`.

## 2. Start conditions and gates

- **Required merges on `gemini/work/fixed`:**
  - Before **X6**: `X3` + `X4` (`exp-x3-x4-shell`). Base already has `F1`, `F3`, `F4`, `F6`, `F9`, `F10`.
  - Before **X7**: `X6`, `F5` (`f5-f7-mode-shell`), `F11` (`f11-lit-services`), `F12` (`exp-f12-coverage`), **plus the pre-turn `UiApprover` verification and shared `assets/approval_card.js` from Solve `S5`** (or an explicit pre-X7 prerequisite commit if Explore is dispatched ahead of Solve `S5`; NEVER race `S5` in parallel on `approval_card.js` and `assets.rs`!).
  - Before **X8**: `X7` (base already has `F4`).
- **Mandatory pre-X7 technical & approval gates:**
  1. **Pre-turn `UiApprover` verification (`00-overview.md` 3.1 row 13, `explore.md` 12 risk 8):** Note that the existing `f5-f7-mode-shell.md` brief does **NOT** include verifying/enabling `UiApprover` prompting outside a running model turn. That verification (confirming `Router::try_call("host:idea:<session>", ...)` prompts cleanly when no model turn is active) MUST land in a pre-X7/S5 prerequisite before X7 starts.
  2. **Shared approval card single owner (`00-overview.md` 3.1 row 13 & section 9):** Solve **`S5`** is designated as the single owner of shared `crates/desktop-server/assets/approval_card.js` (supporting both per-session network grants and per-caller worker grants). X7 consumes `approval_card.js` verbatim once `S5` merges; only if the coordinator explicitly dispatches Explore ahead of `S5` (and pauses `S5` until X7 merges) may a prerequisite or X7 create `assets/approval_card.js` under that neutral name. Never run X7 and S5 concurrently on `approval_card.js` / `assets.rs`.
  3. **Coordinator/owner `RULES.md:57-59` authorization gate for `explore/1`:** Before dispatching X7, the coordinator/owner MUST extend `RULES.md:57-59` (matching the existing `inv-v6-tasks` exception) to authorize registering `[profile explore/1]` and its two new tools (`idea_search`, `idea_fetch_papers`) in `crates/research-tools/tests/golden/tool_profiles.txt` ONLY, while keeping `research/1`, `investigation/1`, and `crates/research-tools/tests/golden/tool_defs.txt` byte-for-byte frozen.

## 3. Base-commit verified interfaces vs future-merge interfaces

### Verified on base commit `832eab5bf`
- `crates/agent/src/transport.rs` (372 lines): `Transport` trait, `ProviderKind`, `TransportInfo` (X6 builds `exchange.rs` on `Transport`; no worker loop in `delegate.rs` to wrap — `explore.md` Appendix A).
- `crates/research-tools/src/tool_profile.rs` (329 lines): `PROFILE_EXPLORE_V1` (`"explore/1"`), `ToolProfile`, `lookup`, `built_profiles`, `register_profile`, `defs_match_exact`.
- `crates/research-tools/src/paper_passages.rs` (`SEARCH_PAPER_PASSAGES`, `def()`, `register()`) and `paper_lines.rs` (`READ_PAPER_LINES`, `def()`, `register()`): existing tool definitions reused unchanged inside `explore/1`.
- `crates/research-tools/src/quote_core.rs` (472 lines): `EVIDENCE_ABSTRACT_EVENT`, `EvidenceAbstractPayload`, `abstract_id` (`xa-<seq>`), `clip_quote`.
- `crates/research-tools/src/run_budget.rs` (464 lines): `IDEA_EXCHANGE_EVENT` (`"idea.exchange"`), `RUN_ID_FIELD`, `count_model_calls`, `ModelCallBudget`.
- `crates/desktop-server/src/routers.rs` (150 lines): `build_desktop_router_for_profile`.
- `crates/desktop-server/src/approver.rs` (284 lines) & `session_lifecycle.rs` (231 lines): `UiApprover::with_session_network_grant(true)`.

### Future-merge interfaces to verify before coding
- **`X3`/`X4`:** `idea.rs` (`next_stage`, budget caps, candidate merging), `routes_idea.rs` (`answer`, `plan`, `search`, `triage`, `continue`, `stop`), `idea_setup.js`, `idea_search.js`.
- **Pre-turn `UiApprover` prerequisite & Solve `S5`:** verified host-initiated `UiApprover` prompting outside a running model turn; shared `assets/approval_card.js` export surface.
- **`F11`:** `lit_providers::run_query` and `lit_abstracts` (`abstract_ref { raw_blob, provider, verbatim }`).
- **`F12`:** `search_coverage::compute`, `Coverage`, `prior_work_wording`.

## 4. Smallest implementation checkpoints

Push after each package (`X6`, then `X7` — which unblocks `X14` Checkpoint 1 — then `X8`), merging `origin/gemini/work/fixed` between packages:

1. **Checkpoint 1 — X6 (Exchange runner & Sharpen):**
   - `crates/agent/src/exchange.rs` (new; NOT `leg.rs` per `00-overview.md` 3.2 row 17): generic single-job exchange runner over `Transport` offering exactly ONE tool spec built from the recorder schema; rejects any other tool call with an immediate error (Rule P2); runs 1 call + at most 2 repair attempts; invokes F9 budget check callback BEFORE every provider call (Rule P3); appends `idea.exchange` audit event (`exchange`, `attempt`, `model`, `tokens_in`, `tokens_out`, `request_blob`, `response_blob`, `accepted`, `faults`, `run_id`) on every attempt.
   - `crates/research-tools/src/idea_brief.rs` (new): `IdeaBrief`, `Perspective`, `record_idea_brief` (`#[serde(deny_unknown_fields)]`) enforcing `restatement <= 1200`, `assumptions <= 8`, `subclaims[1..=4]` with `idea_quote` as an exact substring of `IdeaStatement.text`, `perspectives[2..=6]` requiring `exact` and `other_names` (`obstruction` optional), `questions[0..=2]`, sequential `sc-N` / `p-N` assignment.
   - `crates/desktop-server/src/idea_exchanges.rs` (new): runs Sharpen exchange in parallel with first search, writes `idea.brief`, handles `idea.answer` (question answers & user edits to brief/queries). On CLI-subscription routes (`claude-cli`/`antigravity`), returns a clear `"Explore exchanges are not supported on CLI routes yet (awaiting X18)"` message so deterministic preview/search keeps working until X18.
   - `crates/desktop-server/assets/idea_setup.js`: render `"Sharpened idea"` card (NEVER the word `"brief"` on screen — `00-overview.md` 3.2 row 18), highlight each `subclaim.idea_quote` span in the user's text, allow editing queries and answering questions.
2. **Checkpoint 2 — X7 (Plan execution, `explore/1` profile & `idea_search`, batched approval, F12 coverage):**
   - `crates/research-tools/src/idea_tools.rs` (new):
     - `idea_search` tool (`queries[1..=24]`, provider enum with ALL 6 services `arxiv | inspire | semantic_scholar | crossref | openalex | theoremsearch` from day one so X19 never changes `def_hash`; `openalex`/`theoremsearch` return `status: "disabled"`; `q <= 300`; `relation: none | cites | cited_by + key`; `max_results 1..=25`; `EffectClass::NetworkRead`, `Ask`, batched approval).
     - `idea_fetch_papers_def()` (exact frozen schema from `explore.md` 5.2: `keys[1..=8]`, `NetworkRead`, `Ask`) registered with an honest fail-closed placeholder handler returning `Err("idea_fetch_papers: not yet wired (package X9)")` so **the complete 4-tool `explore/1` profile (`idea_fetch_papers`, `idea_search`, `read_paper_lines`, `search_paper_passages` with `research_policy()`) is frozen ONCE in `tool_profile.rs` and `tests/golden/tool_profiles.txt` with zero `def_hash` churn in X9 or X15**! (Conversation turns remain unexposed until X15.)
   - `crates/desktop-server/src/routers.rs`: add `build_idea_router` built from `PROFILE_EXPLORE_V1` (verifies zero `ExecuteCode` or `WriteWorkspace` tools — Rule P7).
   - `crates/desktop-server/src/idea_exchanges.rs`: assemble `SearchPlan` (`idea.search_planned`), execute strictly via `Router::try_call("host:idea:<session>", "idea_search", ...)` (Rule P5: zero direct bypass), write `idea.search_finished`, merge candidates via X1 into `idea.candidates`, invoke F12 `search_coverage::compute` + `prior_work_wording` including `known_paper_recall` and library counts.
   - `crates/desktop-server/assets/idea_search.js`: mount shared `approval_card.js` (delivered by Solve `S5` / pre-X7 prerequisite; do not duplicate or race `S5`) to show all planned queries, services, and grants (`"Allow for this session"`), and render the live `"Where I looked"` panel with per-service status and `"Retry <service>"`.
3. **Checkpoint 3 — X8 (Triage & abstract evidence):**
   - `crates/research-tools/src/idea_triage.rs` (new): `Triage`, `record_idea_triage` (`#[serde(deny_unknown_fields)]`); verifies every `abstract_quote` as a whitespace-normalised substring of F11's stored raw-response blob ONLY when `abstract_ref.verbatim == true` (Rule C3: rebuilt abstracts can NEVER become evidence), fills `(start, end)` offsets, emits shared F4 `evidence.abstract` (`xa-<seq>`). Supports Quick preset path (0 papers read -> abstract-only map path, Rule V2).
   - `crates/desktop-server/src/idea_exchanges.rs`: runs Triage exchange (wraps candidate titles/abstracts as quoted data — Rule C7), writes `idea.triage` + `evidence.abstract`, updates F10 `seen` (`Triaged`).
   - `crates/desktop-server/assets/idea_search.js`: grouped candidate list (`core`, `related`, `background`, collapsed `unrelated`), every triage label marked `"from abstract"`, editable reading-list ticks, optional `"pause before reading"` toggle (decision 2).

## 5. Owned files

- `crates/agent/src/exchange.rs` (new, `<= 600` lines; sibling `exchange_tests.rs` if needed)
- `crates/research-tools/src/idea_brief.rs` (new, `<= 600` lines)
- `crates/research-tools/src/idea_tools.rs` (new, `<= 600` lines)
- `crates/research-tools/src/idea_triage.rs` (new, `<= 600` lines)
- `crates/desktop-server/src/idea_exchanges.rs` (new, `<= 600` lines)
- `crates/desktop-server/src/routers.rs` (150 lines — add `build_idea_router`)
- `crates/desktop-server/assets/idea_setup.js` (owned sequentially after X4)
- `crates/desktop-server/assets/idea_search.js` (owned sequentially after X4; complete & merge before X9 starts!)
- Tests: `crates/research-tools/tests/idea_exchanges_validate.rs`, `crates/desktop-server/tests/idea_search_triage.rs`, `crates/desktop-server/tests/idea_search.test.mjs`.

## 6. Shared registration seams and "Do not edit" list

- **Registration seams:**
  - `crates/agent/src/lib.rs` (99 lines): `pub mod exchange;`.
  - `crates/research-tools/src/lib.rs`: `pub mod idea_brief;`, `pub mod idea_tools;`, `pub mod idea_triage;`.
  - `crates/research-tools/src/tool_profile.rs` (329 lines) & `crates/research-tools/tests/golden/tool_profiles.txt`: wire `explore_v1()` into `lookup`, `built_profiles`, `register_profile`, and append `[profile explore/1]` to `tool_profiles.txt` ONLY after `RULES.md:57-59` is updated (do NOT touch `research/1`, `investigation/1`, or `tool_defs.txt`).
  - `crates/desktop-server/src/lib.rs`: `pub mod idea_exchanges;`.
  - `crates/desktop-server/src/routes_idea.rs`: wire `answer`, `plan`, `search`, `triage`, `continue`, `stop` ops to `idea_exchanges.rs`.
- **Do not edit:** `crates/desktop-server/assets/approval_card.js` (owned by Solve `S5` / pre-X7 prerequisite; consume read-only), `crates/desktop-server/src/assets.rs`, `crates/research-tools/tests/golden/tool_defs.txt`, `crates/agent/src/resume.rs`, `crates/agent/src/runner.rs` (626), `crates/desktop-server/src/cli_turn.rs` (626), `paper_passages.rs` (605), `http_client.rs`, `net_rate.rs`, `Cargo.lock`.

## 7. Tests through the entry point

Run `cargo test -p agent --lib exchange`, `cargo test -p research-tools --test tool_def_golden --test tool_profile_golden --test idea_exchanges_validate`, `cargo test -p desktop-server --test idea_search_triage --test eval_set_r`, `cargo test -p agent --test resume_loop`, `node --test crates/desktop-server/tests/*.test.mjs`, `cargo clippy --workspace --all-targets -- -D warnings`, and `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`.

Required tests (scripted `Transport` + loopback mock only, zero live network/model calls, zero third-party text):
- `idea_exchange_rejects_other_tool_calls` (Rule P2) & repair cap (at most 2 repairs; 3rd failure ends exchange cleanly with `idea.exchange` events recorded).
- `idea_budget_stops_search_at_cap` & `idea_budget_stops_exchanges_at_cap` (Rule P3): stops as `stopped_at_cap` -> F9 `"Paused: (reason)"` with Continue, never silently cuts.
- `idea_model_queries_need_approval` & `idea_search_has_no_bypass` (Rule P5): model-written plan queries MUST traverse `Router::try_call("host:idea:...", "idea_search", ...)` and block on real `UiApprover` denial.
- `idea_router_has_no_exec_or_write_tools` (Rule P7): `build_idea_router` registers zero `ExecuteCode` or `WriteWorkspace` tools and uses `research_policy()`.
- `idea_requests_carry_no_identity` (Rule P8): default `User-Agent` stays `re-search/<version>`, keys go only to their own host.
- `idea_abstract_quote_must_be_in_blob` & `idea_abstract_rebuilt_is_never_evidence` (Rule C3): fabricated abstract quote or OpenAlex-style `verbatim: false` abstract is rejected and produces no `evidence.abstract`.
- `idea_brief` validation: rejects missing `exact` or `other_names` perspective, rejects `idea_quote` not in verbatim idea text, rejects unknown fields (`deny_unknown_fields`).
- UI tests (`idea_search.test.mjs`): `"Sharpened idea"` (never `"brief"`), `"from abstract"` labels, batched approval card, `"Where I looked"` with `"Retry"`, zero `"verified"` (Rule S8), XSS escaping.

## 8. Document conflicts and resolutions

1. **"Leg" vs "Exchange":** Earlier `explore.md` used `agent/src/leg.rs`, `idea_legs.rs`, `idea.leg`. **Resolution:** `00-overview.md` 3.2 row 17 (owner decision 11) renames them to `agent/src/exchange.rs`, `idea_exchanges.rs`, `idea.exchange`.
2. **Screen label for `IdeaBrief`:** Data event stays `idea.brief`, but Solve uses `"resume brief"`. **Resolution:** `00-overview.md` 3.2 row 18 mandates `"Sharpened idea"` on screen; never show `"brief"`.
3. **Abstract evidence event kind:** Earlier `explore.md` used `idea.abstract_evidence`. **Resolution:** `00-overview.md` 2.5 (owner decision 10) mandates shared `evidence.abstract` (`xa-<seq>`).
4. **Incremental `explore/1` registration across X7/X9/X15:** Adding `idea_fetch_papers` later in X9 would mutate `explore/1`'s golden hash and cannot use `may_gain` (`NetworkRead` is disallowed in `may_gain` by F1). **Resolution:** X7 freezes all 4 tool definitions in `explore/1` once (with `idea_fetch_papers` returning a fail-closed `"not yet wired (package X9)"` error until X9 plugs in the handler body without altering its `ToolDef`).

## 9. Out of scope and hand-back

- **Out of scope:** Reading queue & `idea_fetch_papers` handler body (`X9`), passage reader (`X10`), map/verdict (`X11`), Conversation turns (`X15`), CLI-subscription exchanges (`X18`), OpenAlex/TheoremSearch allowlist (`X19`).
- **Hand-back:** Write `docs/design/modes-2026-11/reports/exp-x6-x7-x8-search-triage.md` (documenting the `approval_card.js` seam, `explore/1` golden hash, and F5 host-approver verification), commit with `Co-Authored-By`, and push after each checkpoint to `gemini/modes/exp-x6-x7-x8-search-triage`.
