# Report: `audit-orchestration-routing` — F9b Orchestrator & Smaller-Worker Execution Design Review

- **Brief:** `docs/design/modes-2026-11/agents/audit-orchestration-routing.md`
- **Base branch / commit:** `origin/gemini/work/fixed` @ `f68999d6aef1afa780d100a9bc746d6b6372539a`
- **Target branch:** `gemini/modes/audit-orchestration-routing`
- **Scope & deliverables:** Design review, codebase audit, and focused test verification only. Concurrent feature briefs keep ownership of production code.
- **Files changed in this branch:**
  - `docs/design/modes-2026-11/reports/audit-orchestration-routing.md` (added, 426 lines; 0 production `.rs`/`.js`/`.py`/`.css`/`.html` files modified; `Cargo.lock` and `crates/research-tools/tests/golden/tool_defs.txt` untouched).

---

## Executive Summary

`00-overview.md` §3.4 (F9b) establishes four core guarantees across Investigate, Explore, and Solve:
1. Smaller/cheaper worker models handle bounded scanning, screening, triage, passage extraction, and routine algebra while the frontier Orchestrator retains problem framing, synthesis, proof-step recording, and final judgement (`00-overview.md:599-609`).
2. Worker assignments are resolved from presets/settings, **pinned for a run**, and **never silently fall back to the frontier model** without an explicit per-run or preset opt-in (`00-overview.md:610-615`).
3. Every provider call (including sent retries) charges the shared F9 model-call budget **while reserving headroom for the frontier synthesis/closing pass**, and missing token usage is reported as **unknown** rather than `0` (`00-overview.md:624-631`).
4. Worker runs, check evidence, and dispositions (`accepted` / `rejected` / `partial`) are inspectable on demand without polluting the Orchestrator's prompt or main UI surface (`00-overview.md:619-623, 638-652`).

Auditing `f68999d6aef1afa780d100a9bc746d6b6372539a` end to end and running three disjoint Carbon (`pro`) hostile mode reviews uncovered **seven systemic gaps** between `00-overview.md` §3.4 and the current codebase:
- **Pervasive silent frontier fallbacks:** `agent_models.rs:289-380`, `paper_reader.rs:142-157, 454-460`, `turn_runner.rs:287-321`, `autonomous.rs:438`, and `autonomous_prompts.rs:160` silently fall back to `"isolated_primary_model"` (or `solver`) whenever `Catalog` is cold/errored/`--api-base`, whenever an explicitly configured worker's `build_transport` fails (`.ok()` swallows the error!), or whenever an Investigation turn is section-scoped.
- **Three disconnected role stores & env-key misrouting:** `UiSettings.agent_models`, `UiSettings.role_presets`, and `ProviderKeyStore.roles` (`providers.json`) disagree on role names and `ProviderKeyStore.roles` (including per-role `base_url`) is never read at dispatch; `DesktopState::build_transport` (`state.rs:495-549`) checks `RE_SEARCH_API_KEY` before `providers.json`, misrouting an Anthropic env key onto an OpenAI worker.
- **Zero pre-dispatch budget reservation:** `run_budget::count_model_calls` (`run_budget.rs:130-175`) is purely post-hoc over ledger events, while `paper_read.rs:522-531` and `explore.rs:356-373` emit `delegate.turn` / `explore.branch` only **after** parallel threads join. Four parallel reader slices (`4 * 12 = 48` calls) can blow Investigate Orient (`cap = 8`) by 6x and leave `0` calls for the frontier Orchestrator's mandatory 2-call closing sequence (`ToolUse(record_map_reading)` + `EndTurn` per `runner.rs:409-417`).
- **Missing usage coerced to `0` & dropped verifier/strategist/narrator usage:** `OpenAiTransport` (`openai.rs:131`) streams without `stream_options.include_usage` (while `AnthropicTransport` shares `stream_http::inject_stream_flag` at `anthropic.rs:81`, making naive injection into `stream_http.rs` a 400-breaking bug); `delegate.rs:231-235` has a `Value::Null` `.or_else` bug and `.unwrap_or(0)`; `verifiers.rs:665-688`, `strategist.rs:150-163`, and `narrator.rs:224-236` drop `resp.usage` and `provider` completely.
- **Cross-caller provenance & approval leaks:**
  - `DesktopState::session_capability_grants` (`state.rs:170`, `session_lifecycle.rs:30-67`) keys grants by `(session_id, EffectClass)` **without `caller`, `tool_id`, or `policy_version`**, leaking a host plan grant (`host:idea:<session>`) or solver `ExecuteCode` grant across LLM/worker callers and dead worker epochs.
  - F4 `SentLines` (`lines_sent.rs:111-144`) strictly isolates `caller`, so Investigate Orchestrator (`V6–V8`) and Explore Conversation (`X15`) cannot cite worker-read lines unless exact verified spans delivered into the frontier prompt are logged via `context.supplied` for the frontier caller.
  - Solve `SolveState::apply` (`solve_state.rs:190-204`) pushes **every** caller's `sympy_eval` (and any >4 KiB blobbed Python traceback!) into `pending_runs`, causing `verifier:solve` (`S14`), `compute_blind` (`S20`), or `compute_worker` scratch runs to deadlock Solver Gate `G1`.
- **Windows multi-worker sandbox teardown bug:** `AppContainerProfile::drop` (`crates/worker/src/appcontainer.rs:110-122`) calls `DeleteAppContainerProfile("re-search.worker.<session>")` and `AppContainerSandbox::drop` (`218-224`) deletes `.worker.lock`, tearing down the active sandbox profile out from under Solver and Check processes whenever a transient Solve Verifier or Blind Worker stops.
- **600-line file-ceiling blockers:** Critical orchestration files (`runner.rs` 626L, `verifiers.rs` 878L, `cli_turn.rs` 626L, `routes_tools.rs` 610L, `routes_session.rs` 616L, `http_srv.rs` 702L, `session_view.rs` 753L) are already over the 600-line F0 gate, and `paper_reader.rs` (599L), `state.rs` (598L), `investigation.rs` (597L), and `delegate.rs` (594L) have `< 7` lines of headroom. F9b must land via new focused `< 400`-line modules plus coordinated pre-splits.

---

## 1. End-to-End Traces & Silent Frontier-Spend Audit

### 1.1 Trace A — Investigate Reader Call (`paper_reader`)

1. **Settings & Turn Entry:**
   - Stored in `<data_dir>/ui_settings.json` as `UiSettings.agent_models["paper_reader"]` (`crates/desktop-server/src/state.rs:98`, `agent_models.rs:23`), `UiSettings.paper_readers_enabled: Option<bool>` (`state.rs:112`), and `UiSettings.paper_reader_max_parallel: Option<usize>` (`state.rs:114`).
   - Entered via `POST /api/sessions/{id}/messages` (`turn_runner.rs:78` `run_api_turn_worker`, called directly or from `investigation.rs` mapping/conventions/branch/question endpoints, and in `V6` from `POST /api/sessions/{id}/investigation/tasks`).
   - `turn_runner.rs:188` calls `paper_reader::prepare_for_turn(state, session_id, req, primary_provider, &primary_model, &existing)`.
2. **Resolver & Transport Construction:**
   - `prepare_for_turn` (`paper_reader.rs:413-460`) checks for an active `"investigation.paper"` marker (`422`), verifies the turn is an Investigation recording turn (`_paper_map_request`, `_paper_conventions_request`, or `_paper_branch_request` at `127-134`), calls `should_enable_readers` (`142-157`), and resolves transport via `agent_models::resolve_paper_reader_transport` (`agent_models.rs:335-358`) -> `DesktopState::build_transport` (`state.rs:476-551`).
   - Sets wire reasoning effort `"low"` on each `ReaderSlice` (`paper_reader.rs:487`, verified on wire as `"output_config": {"effort": "low"}` in `paper_reader_wiring.rs:321`).
3. **Tool Policy, Containment & Approval:**
   - `build_reader_router` (`paper_reader.rs:159-169`) builds an isolated per-turn `tools::Router` with `research_policy()` (`v0.4-research`) and **only three tools** (`paper_read::register_reader_tools`, `crates/agent/src/paper_read.rs:96-112`): `search_paper_passages`, `read_paper_lines` (7,000-char per-call budget `READER_LINE_BUDGET`), and `note_evidence`.
   - `UiApprover` is constructed with `contained: false` and `allow_session_network_grant: false` (`paper_reader.rs:161-167`). All three tools have `EffectClass::ReadWorkspace`, which `v0.4-research` evaluates to `Decision::Allow` (0 approval prompts); any hallucinated call to `ledger_append` or `sympy_eval` is rejected as unknown/denied (`paper_read_tests.rs`). No Python subprocess is spawned.
4. **Execution & Ledger Receipts:**
   - When the Orchestrator invokes `delegate_read_paper` (`runner.rs:515-527` -> `paper_read::run_paper_readers` at `paper_read.rs:442`), the host plans `1..=max_parallel` slices (`MAX_PAPER_READER_SLICES = 8`, `DEFAULT_API_MAX_PARALLEL = 4`, `DEFAULT_CHATGPT_MAX_PARALLEL = 2`, `paper_reader.rs:23-25`), emits `delegate.start { call_id, role: "paper_reader", worker: "paper_reader", provider, model, routing_reason, slice_count, max_parallel }`, and runs `run_single_reader` threads in `std::thread::scope` (`paper_read.rs:496-514`, up to `PAPER_READER_MAX_TURNS = 12` turns/slice).
   - Each slice calls tools on `reader_router` as caller `"worker:paper_reader:<provider>:<session>:r<idx>"` (`paper_read.rs:152`), appending `tool.request` + `tool.result` and `investigation.evidence` (`ev-<seq>`, `investigation_evidence.rs:226-260` via `append_at_head` with 3 CAS retries).
   - **Post-join:** Only *after* all slice threads join (`paper_read.rs:522-537`), the main thread stores raw response blobs and emits one `delegate.turn` per *successful* slice turn (`ok` turns only — failed/aborted turns emit no `delegate.turn`!), then `delegate.end { call_id, worker: "paper_reader", model, routing_reason, ok, turns, tokens_in, tokens_out, evidence_count, gap_count, raw_blob }`, and `runner.rs:578-588` emits `model.tool_result` (clipped to `RESULT_TEXT_LIMIT = 4000` chars in the ledger summary while passing full `<= 24 KiB` JSON in `ContentBlock::ToolResult`). On resume, `resume.rs:289-312` reconstructs full tool output from `delegate.end.raw_blob` without leaking reader internal turns into Orchestrator `messages`.
5. **Token & Call Reporting:**
   - `paper_read.rs:533-534` sums `extract_usage_tokens(&resp.usage)` (`.unwrap_or(0)`) across completed turns into `delegate.end.{tokens_in, tokens_out}`. `session_summary.rs:132-135` adds those to `worker_input_tokens` / `worker_output_tokens`.
   - `run_budget::count_model_calls` (`run_budget.rs:148-167`) deduplicates `delegate.turn` vs `delegate.end.turns` by `call_id` (`delegated += turns.max(seen_turns)`).
6. **Caller-Provenance Check (`ev-<seq>` today vs `F4` in `V6–V8`):**
   - *Legacy Investigation (`record_paper_map` / `summary` / `conventions` / `branch`):* `investigation_evidence::resolve_evidence` (`crates/research-tools/src/investigation_evidence.rs:291-398`) accepts an `ev-<seq>` logged by `"worker:paper_reader:..."` when cited by `"agent:<provider>:<session>"` **if** `rec.marker_seq == active_marker_seq` and `anchor.quote` matches inside `rec.excerpt` (`380-396`), without checking Orchestrator `lines_sent`.
   - *Legacy Question turns (`_paper_question_request`):* `investigation_cites.rs:56-62` (`is_main_agent_caller`) explicitly **excludes** `"worker:*"`! And `paper_reader` is not even offered on `_paper_question_request` (`paper_reader.rs:127-134`).
   - *New Investigate (`V6` `record_map_reading`, `V7` `Read`/`Check`, `V8` `Ask` on F4 `quote_core::accept_quote` + `lines_sent_with_ledger`):* `SentLines::covers` (`lines_sent.rs:129-144`) partitions strictly by `(caller, source_id, path, file_sha256)`. Lines read by `"worker:paper_reader:..."` give **zero** `SentLines` credit to `"agent:<provider>:<session>"` unless handed off explicitly (resolved in §2.4 & §3.7 below).

---

### 1.2 Trace B — Explore Literature & Passage-Reader Calls

Two distinct mechanisms exist in the repository and must be kept strictly separate:

#### Mechanism B1 — Legacy Research/Investigation `delegate_literature` (and Ultramath `delegate_explore`)
1. **Settings -> Transport:** `turn_runner.rs:219, 309-321` resolves `UiSettings.agent_models["literature_worker"]` via `agent_models::resolve_worker_transport` (`agent_models.rs:312-331`). (`delegate_explore` at `crates/agent/src/explore.rs` uses `"explorer_worker"` for parallel SymPy/Ultramath math branches — `explore.md:673-675` keeps this solely as Ultramath and retires `delegate_literature` from Explore mode at `explore.md:645-646, 655`.)
2. **Policy, Containment & Approval:** `delegate_literature` (`delegate.rs:119-122, 389-504`) runs up to `max_turns` (`8`, clamped `<= 12`) on the **shared session `Router`** with caller `"worker:literature_worker:<provider>:<session>"` (`delegate.rs:418`) and capability filter `["arxiv_search", "literature_search", "doc_ingest", "search_paper_passages", "read_paper_lines", "fs_read", "fs_list", "ledger_query", "ledger_note"]`.
   - `arxiv_search` (`arxiv.rs:67`) and `literature_search` (`literature.rs:53`) have `EffectClass::NetworkRead`, `ApprovalScope::Once`. `v0.4-research` hard-clamps `NetworkRead` to `Decision::Ask` (`policy/src/lib.rs:158-184`).
   - **Approval mismatch found:** `ApprovalStore` (`crates/tools/src/approval.rs:145, 162-173`) keys `NetworkReadGrant` by `(caller, policy_version)`, **whereas `DesktopState::session_capability_grants` (`state.rs:170`, `session_lifecycle.rs:40`) keys by `(session_id, EffectClass)` with no `caller`!** If `auto_approve_tools == false` and the user approved `"Allow for session"` for `agent:...`, `UiApprover::decide_ext` (`approver.rs:158-165`) auto-approves `worker:literature_worker:...` without prompting and binds a second `ApprovalStore` `NetworkReadGrant` (`router.rs:359`).
3. **Receipts, Tokens & Provenance:** Emits `delegate.start`, `tool.request`/`tool.result`, `delegate.turn` (on `Ok` turns only, `delegate.rs:447-459`), and `delegate.end` (`481-497`). Worker cannot call `ledger_append` or `record_*`; any ingested paper (`doc_ingest`) writes `source.ingested`, which the Orchestrator must subsequently read under its own `agent:...` caller to satisfy F4 `lines_sent`.

#### Mechanism B2 — Explore Mode Staged Pipeline (`explore.md` §5.1 `X6–X16`)
1. **Settings -> Pinned Roles -> Transport:**
   - Explore is a **host-driven staged pipeline**, not an LLM `delegate_*` tool call (`explore.md:118-135, 445-476`).
   - Deterministic host stages make **0 model calls**: `X7` Search calls `Router::try_call("host:idea:<session>", "idea_search", ...)` (`EffectClass::NetworkRead`, batched query plan card) and `X9` Fetch calls `idea_fetch_papers` (`paper_fetch::fetch` -> `ingest_bytes`).
   - Structured single-call model stages (`1` call + `<= 2` schema-validation repairs via `agent/src/exchange.rs`, audited as `idea.exchange`):
     - **Smaller worker role (`literature_worker` / `idea_reader`):** `X8` Triage (`record_idea_triage`, max 3 calls) and `X10` Passage Readers (`record_paper_evidence`, concurrent per-paper readers `worker:idea_reader:<provider>:<session>:<idx>`).
     - **Frontier Orchestrator (`orchestrator`):** `X6` Sharpen (`record_idea_brief`, max 3), `X11` Map & Overlaps (`record_idea_map`, max 3), `X13` Directions (`record_idea_directions`, max 3), `X16` Thorough self-check, and `X15` Conversation (`explore/1` loop on `runner.rs`).
2. **Receipts, Tokens & Cross-Caller Provenance (`C1–C6` + `F4`):**
   - `X10` host code packs selected passages into each reader's prompt, logs `context.supplied` (`lines_sent.rs:45-91`) for `worker:idea_reader:...`, and validates returned quotes via `quote_core::accept_quote` (`quote_core.rs:221-254`), writing `evidence.quote` (`xe-<seq>`).
   - `X11` (`record_idea_map`) cites **only** `xe-<seq>` and `xa-<seq>` IDs (`explore.md:452, 526` rule `C4` forbids raw quotes/line numbers in `record_idea_map`) and deterministic Rust checks `C5/C6/V1` (`definition`/`known_result`/`open_question` cannot rest on abstract-only `xa-*`). Thus **`X10` worker -> `X11` frontier map synthesis succeeds cleanly without cross-caller `lines_sent` failure**.
   - **Hazard in `X15` Conversation (`explore/1`):** `runner.rs:246` runs as `agent:<provider>:<session>`. When the user asks about a paper read by `worker:idea_reader:...` in `X10`, `SentLines` has **no** entry for `agent:<provider>:<session>` unless `X15` context assembly logs `context.supplied` for the exact `xe-<seq>` spans injected into `X15`'s prompt (or the Orchestrator calls `read_paper_lines`). Furthermore, `investigation_cites.rs:417-440` hard-requires `"investigation.paper"` and a single `primary_path`, so `X15` needs a multi-paper F4 cite checker built on `lines_sent_with_ledger`.

---

### 1.3 Trace C — Solve Compute Call (`compute_worker` & `compute_blind`)

1. **Settings & Schemas:**
   - Three stores exist today: (a) `UiSettings.agent_models["compute_worker"]` (`agent_models.rs:20`), (b) `ProviderKeyStore.roles` (`presets.rs:24-31`: `solver`, `step_verifier`, `full_verifier`, `strategist`, `narrator`, `sidetalk` — **no `compute` key and never read by `agent_models.rs`!**), and (c) Solve's merged S2 `SolveRoles { solver: RoleModel, verifier: RoleModel, compute: Option<RoleModel> }` (`crates/research-tools/src/solve_model_records.rs:80-85`).
2. **Legacy Research `delegate_compute` vs Solve Mode `solve/1`:**
   - *Today (`research/1`):* `turn_runner.rs:218, 300-308` -> `resolve_worker_transport("compute_worker")` -> `delegate_compute` (`delegate.rs:114-117, 389-504`) running on the **shared session `Router`** as caller `"worker:compute_worker:<provider>:<session>"` with tools `["sympy_eval", "check_expression_sanity", "ledger_append", "ledger_query", "ledger_note"]` (`delegate.rs:367-368` blocks `entry_type == "final_answer"`).
   - *In Solve (`solve/1`, `solve.md` §4–5):* `ledger_append` is replaced by `step_record` (**frontier `solver` only**). Smaller `compute_worker` runs routine SymPy exploration (`sympy_eval` saving handles `name@v` via the S6 outbox -> Check Process `srepr_safe.py` -> `solve.expr`), while `S20` `compute_blind` runs an isolated blind worker for `independent_recompute`, `host:check` runs deterministic `check_run` in the Check Process (0 model calls), and `verifier:solve` (`S14`) runs adversarial checks in its own Verifier Worker.
3. **Containment, Multi-Process Isolation & `WorkerSession` Approvals:**
   - On Windows, `Worker` spawns Python inside zero-capability `AppContainerSandbox` (`crates/worker/src/appcontainer.rs:228-286`). On Linux, `build_worker` (`crates/desktop-server/src/routers.rs:50-54`, `crates/cli/src/runtime.rs:74-76`) fails closed with `"AppContainer sandbox requires Windows"`.
   - `sympy_eval` is registered via `register_worker_session` (`ApprovalScope::WorkerSession`, `EffectClass::ExecuteCode`). `ApprovalStore::matching_session_grant` (`crates/tools/src/approval.rs:175-193`, `router.rs:184-210`) keys grants by **`(caller, tool_id, tool_def_hash, policy_version, worker_epoch)`** and requires `epoch_probe.is_contained() == true`.
   - **Three concrete collisions discovered in Solve:**
     1. When `auto_approve_tools == true` (default at `state.rs:80-82`), `UiApprover` (`approver.rs:129-153`) bypasses all prompts, nullifying `solve.md` D5 setup approval and `PauseReason::WorkerRestarted`. When `auto_approve_tools == false`, `state.session_capability_grants` (`(session_id, ExecuteCode)`) ignores `caller`, `tool_id`, and `worker_epoch`!
     2. `Registry` (`crates/tools/src/registry.rs:136-174`) binds only **one** `(ToolHandler, WorkerEpoch)` per `tool_id`, so a single `Router` cannot dispatch `"sympy_eval"` to separate Solver, Verifier, and Blind Python processes (`solve.md:267-268` R6 forbids model-written `sympy_eval` in the Check Process, and `S20` Gate `G3` forbids Blind Worker sharing Solver globals/`scratch`).
     3. `AppContainerProfile::drop` (`appcontainer.rs:110-122`) deletes `"re-search.worker.<session>"` when *any* worker for `<session>` drops!
4. **Receipts, Token Reporting & Caller-Provenance (`SolveState` & `solve_own_tier`):**
   - `SolveState::apply` (`solve_state.rs:190-204`) records `SolveReceipt` (`call_id`, `code_sha256`, `exprs_out`) without checking `caller`, so `solver` calling `step_record(tool_call_ref: worker_cid, claims: [lhs: "h1@1", rhs: "h2@1"])` satisfies `has_successful_cited_receipt` (`solve_own_tier.rs:75-85`) and `handles_status` (`88-110`).
   - **Critical `pending_runs` deadlock:** `SolveState::apply` (`199-201`) pushes **every** caller's `sympy_eval` into `self.pending_runs`, and `retire_cited_runs` (`251-266`) checks only `tool_call_ref`/`tool_call_refs` (not handle `produced_by.call_id` or `delegate.end` scratch runs), causing `verifier:solve`, `compute_blind`, and multi-step `compute_worker` scratch calls to trip Gate `G1` (`pending_runs.len() > 3`) and block `solve_finish`!

---

### 1.4 Complete Inventory of Silent Frontier-Spend Fallbacks on `f68999d6aef1afa780d100a9bc746d6b6372539a`

| # | Exact Location (`file:line`) | Trigger Condition | Silent Frontier Spend Impact |
|---|---|---|---|
| **F-1** | `crates/desktop-server/src/paper_reader.rs:155-156, 454-460` + `agent_models.rs:352-357` | Any **section-scoped** Investigation turn (`is_section_scoped == true`), even with default `paper_readers_enabled: None` and no `paper_reader` configured (or an unconfigured `paper_reader` provider!). `!is_section_scoped(...)` skips **both** the `isolated_primary_model` guard and `reader_provider_configured`. | Spawns up to `4` parallel reader threads (`12` turns each = up to **48 frontier calls** per `delegate_read_paper`) on `primary_model` with zero user opt-in. |
| **F-2** | `crates/desktop-server/src/paper_reader.rs:152-153, 454` + `agent_models.rs:335-358` | User toggles `paper_readers_enabled: Some(true)` without assigning `agent_models.paper_reader`. `resolve_paper_reader_transport` **never calls `pick_smaller_model`** and goes straight to `"isolated_primary_model"`. | Up to `4..8 * 12` parallel reader calls billed to the frontier model even when a smaller model (`claude-haiku-4-5`, `gpt-4.1-mini`, `gemini-2.5-flash`) is available on the same provider. |
| **F-3** | `crates/desktop-server/src/agent_models.rs:278-280, 320-330, 347-351, 366-379` | User **explicitly configures** `compute_worker`, `literature_worker`, `lean_worker`, `explorer_worker`, `paper_reader`, or a custom subagent, and `state.build_transport(p, m)` returns `Err` (missing API key, expired ChatGPT auth, env-key collision). `.ok()` swallows the error! | Silently discards the user's explicit smaller-model choice and falls through to `primary_model` (`"isolated_primary_model"`), billing the frontier provider instead of surfacing an unavailable-worker state (`00-overview.md:612`). |
| **F-4** | `crates/desktop-server/src/agent_models.rs:289-310` + `model_catalog.rs:124-157` | Worker role unassigned (`None`), and `Catalog::pick_smaller_model` returns `None`: (a) server startup before `GET /api/models` populates cache (`ok_ms.is_none()`), (b) after `Catalog::clear()` on settings PUT, (c) `--api-base` override (`ListAccess::NoCall`), (d) transient model-list error (`last_error.is_some()`), or (e) Ollama / local models lacking `"haiku"\|"mini"\|"flash"\|"nano"` substrings. | Unconditionally returns `primary_model` with `routing_reason: "isolated_primary_model"`, spending frontier tokens on `compute_worker`, `literature_worker`, `lean_worker`, `explorer_worker`, and custom subagents. |
| **F-5** | `crates/desktop-server/src/turn_runner.rs:287-289, 309-321` | Investigation turns: `literature_worker` (`309-321`) is **not** gated by `if paper_reading_turn { None }` (contradicting `investigation_prompts.rs:10-11`), and `paper_reading_turn` (`287-289`) omits `_paper_question_request`, `_paper_request`, and future `V6` tasks. | Offers worker delegation tools on Investigation turns where delegation should be absent or restricted, hitting **F-4** silent frontier fallback. |
| **F-6** | `crates/desktop-server/src/agent_models.rs:266-270` + `crates/agent/src/autonomous.rs:438` + `autonomous_prompts.rs:160, 182-185` | `step_verifier`, `full_verifier`, or `strategist` is unassigned OR its `build_transport` fails (`.ok()` returns `None` at `269`). `autonomous.rs:438` and `autonomous_prompts.rs:160` execute `.unwrap_or((solver_transport, solver_model))`. | Silently runs multi-round verification and mandatory strategist consultations on the frontier `solver` model (and erases the distinction between explicit `SameModel` setup vs broken verifier transport). |
| **F-7** | `crates/desktop-server/src/agent_models.rs:168-230` (`resolve_in_core_role_model`) | On-demand `claim_review.rs:108` (`step_verifier`), `routes_tools.rs:502, 538` (`narrator`, `explain`/sidetalk), and `routes_subagents.rs:119` (`subagent_builder`) when unassigned (`fallback_to_default = true`). | Falls back to `ui.default_provider`/`ui.default_model` (frontier Orchestrator) or the first connected provider's default model. |
| **F-8** | `crates/cli/src/api_session.rs:216, 414` | CLI `re-search run --route api` / `re-search resume` ignores `providers.json` & `ui_settings.json` and passes `AutonomousRoles::default()` (`verifier: None`, `strategist: None`). | Every CLI autonomous verification/strategist call runs on the primary `solver` model even when `providers.json` defines smaller role models. |
| **F-9** | `crates/desktop-server/src/state.rs:495-549` + `crates/agent/src/transport.rs:98-104` | `RE_SEARCH_API_KEY` is set in env for primary provider A (e.g. Anthropic) while worker is assigned to provider B (e.g. OpenAI) using `providers.json`. `OpenAiTransport::from_env` reads `RE_SEARCH_API_KEY` first! | Either sends provider A's key to provider B (`401`) or triggers `.ok()` fallback **F-3** onto frontier provider A. |

---

## 2. Minimum Compatible F9b Execution Design

### 2.1 Strict Role Resolver & Run Pinning (`RoleRoutingPolicy` + `ResolvedRunRoles`)

1. **Single Precedence Order & Key Scoping (Fixes Disconnected Stores + F-9):**
   - Precedence for resolving any role `R`:
     1. Explicit per-run/request override (`req.roles[R]` or setup card override),
     2. `UiSettings.agent_models[R]` (extending `AGENT_ROLE_KEYS` additively with `"verifier"`, `"idea_reader"`, and `"compute_blind"`, while keeping legacy aliases `"step_verifier"`/`"full_verifier"`/`"paper_reader"`/`"literature_worker"` working),
     3. `ProviderKeyStore.roles` (`providers.json`, honoring per-role `base_url` validated by `provider_config.rs:124-156`),
     4. Mode/preset default policy (`RoleRoutingPolicy`).
   - In `DesktopState::build_transport` (and a shared `agent::role_routing::build_role_transport` used by CLI `api_session.rs`), replace raw `Transport::from_env` with provider-scoped key lookup matching `connections::own_key` (`connections.rs:162-178`: consult `GENERIC_KEY_ENV_VARS` **only** when `RE_SEARCH_PROVIDER` matches `provider`, else provider-specific env vars, else `store.key_for(provider)`), and honor role `base_url` when `--api-base` is not set.
2. **Eliminate Silent Frontier Fallbacks (Fixes F-1 through F-8):**
   - Represent each worker/helper resolution explicitly as:
     ```rust
     pub enum ResolvedRole {
         Pinned {
             provider: ProviderKind,
             model: String,
             base_url: Option<String>,
             routing_reason: RoutingReason, // ExplicitRole | PresetRole | AutoSmallerTier | ExplicitFrontierOptIn | ExplicitSameModel
         },
         Disabled { reason: &'static str },
         Unavailable { configured_provider: String, configured_model: String, error: String },
     }
     ```
   - **Rule R-Explicit:** If a role is explicitly assigned in run overrides, `agent_models`, or `providers.json` and transport validation fails, return **`ResolvedRole::Unavailable`** — **never** swallow `.ok()` into `primary_model`. Deterministic mode actions (`ingest`, `skeleton`, `idea_search`, `check_run`, reading saved ledger state) stay usable (`00-overview.md:612`); starting a model stage that requires that role surfaces the clear unavailable banner with two explicit user choices: fix credentials/model, or tick **"Use frontier model for this run"** (`allow_frontier_fallback: true` -> `ExplicitFrontierOptIn`).
   - **Rule R-Unassigned:** When a worker role is unassigned (`None`):
     - Query a **Catalog-independent deterministic tier table** first for known API/ChatGPT providers (`Anthropic` -> `claude-haiku-4-5` when primary is Sonnet/Opus; `OpenAi`/`ChatGpt` -> `gpt-4.1-mini` / `gpt-5-mini` when primary is `gpt-4.1`/`gpt-5`/`o3`; `Gemini` -> `gemini-2.5-flash` when primary is `pro`), cross-checked against `Catalog` when `Catalog` is populated (so deprecated slugs are skipped) plus Ollama parameter-tag comparison (`:1b`/`:3b`/`:7b`/`:8b` smaller than `:32b`/`:70b`).
     - If no distinct smaller model is available on a connected provider and `allow_frontier_fallback != true`: return **`ResolvedRole::Disabled { reason: "no_smaller_model_configured" }`** — optional sub-loops (`delegate_read_paper` in Investigate, `delegate_compute` in Solve) stay **off** (Orchestrator works directly within its profile), while host-staged worker steps (`Explore X8/X10`, `Solve S20`) prompt on the setup/plan card before spending frontier tokens.
     - Preserve legacy Investigate non-V6 test compatibility (`paper_reader_wiring.rs:252` `routing_reason_isolated_primary_model`) **only** when legacy `paper_readers_enabled == Some(true)` is explicitly set on the legacy `/messages` route, while removing `&& !is_section_scoped(...)` from `paper_reader.rs:455` so section-scoped turns never auto-enable `isolated_primary_model`.
3. **Run & Stage Pinning (`00-overview.md:613-614, 644`):**
   - Write `roles: BTreeMap<String, PinnedRoleRecord>` onto:
     - **Investigate (`V6`):** `investigation.task_requested` (`Orient`, `Open`, `Check`),
     - **Explore (`X4`/`X6–X15`):** `idea.started` (initial run defaults) **and** each `idea.stage { state: "requested", roles }` + `X15` turn start (so changing a worker model while paused before `Continue` takes effect cleanly on the next stage without rewriting history, `00-overview.md:644`),
     - **Solve (`S2`/`S4`/`S12`):** `solve.setup` (`SolveRoles` + additive `compute_blind: Option<RoleModel>`, `routing: BTreeMap<String, String>`, `allow_frontier_workers: Option<bool>`) and any mid-pause `solve.setup` revision,
     - **Research / CLI / Headless:** `session.start` and `session.resume` (`roles` metadata object).
   - Retries, schema repairs (`exchange.rs`), and resumes within an active stage/leg reconstruct transports strictly from that pinned `roles` record rather than re-reading mutable `ui_settings.json`.

---

### 2.2 Normal Entry Points Coverage

| Entry Point | Current Behavior | Minimum F9b Wiring |
|---|---|---|
| **Desktop In-Core HTTP** (`turn_runner.rs`, future `V6`/`X`/`S` task routes, `claim_review.rs`, `routes_tools.rs`) | Reads `ui_settings.json` ad hoc per helper; ignores `providers.json` `roles`; allows helper routes in all modes. | Call `role_routing::resolve_pinned_run_roles` once per run/stage; gate non-Investigate workers (`compute_worker`, `literature_worker`, `lean_worker`, `explorer_worker`, custom subagents) off on **all** Investigate turns (`session_mode_from_events == Investigate`); gate `narrate`/`sidetalk` per mode. |
| **Desktop Subscription-CLI** (`cli_turn.rs`, `mcp-server`, `cli-route/src/relay.rs`) | `orchestrator` can be `claude-cli`/`antigravity-cli`; workers reject CLI (`agent_models.rs:148`); `cli_turn.rs:454` ignores `ToolProfile`; `relay.rs:189` rejects any caller `!= expected_caller`. | Keep subscription CLIs disallowed for smaller-worker sub-loops; gate Explore pre-`X18` exchanges with a clear message when `orchestrator` is CLI (`explore.md:765`); enforce `ToolProfile` in `cli_turn.rs` (after <600L split); in `relay.rs:189`, allow session-bound Solve internal callers (`host:check`, `verifier:solve`, `worker:compute_worker:*`, `worker:compute_blind:*`) alongside `expected_caller` for S15 parity. |
| **CLI Supervisor** (`crates/cli/src/api_session.rs`) | Reads env vars only; passes `delegation: None` and `AutonomousRoles::default()`. | Load `<data_dir>/providers.json` (+ optional `ui_settings.json`) through `agent::role_routing`, populating `AutonomousRoles` and mode worker transports identically to desktop. |
| **Headless / Eval Harness** (`eval_set_r.rs`, `eval_support/routed_mock.rs`) | `--api-base http://127.0.0.1:<port>` triggers `ListAccess::NoCall`, making `Catalog` empty. | Static smaller-model tier table + explicit `ResolvedRunRoles` works offline under `--api-base` with `0` live `/v1/models` calls. |

---

### 2.3 In-Flight Shared Budget Reservation (`RunBudgetPool`)

`run_budget::ModelCallBudget` (`crates/research-tools/src/run_budget.rs:189-218`) is a pure value type over past ledger events. Add `crates/research-tools/src/budget_pool.rs` (`Arc<RunBudgetPool>`) shared across Orchestrator, parallel workers, verifiers, and exchanges within a run:

1. **Atomic Pre-Send Lease Protocol:**
   - Initialized at run/stage/leg start from `count_model_calls_breakdown(&events, scope)`.
   - Tracks `committed`, `in_flight`, `frontier_reserve`, `verifier_reserve`, and optional per-item sub-caps (`sub_caps: Mutex<BTreeMap<String, SubCap>>`, e.g. Explore `calls_per_paper`).
   - **Order of operations on every provider call (`runner`, `delegate`, `paper_read`, `explore`, `exchange`, `verifiers`, `strategist`):**
     1. Build & validate `ModelRequest` and run `transport.prepare_request(&req)` **first** (local CPU only; fixes `delegate.rs:435`, `paper_read.rs:170`, `explore.rs:116` where `turns_used` was incremented before `prepare_request` failed!).
     2. Call `pool.try_acquire(PoolCaller::Worker { sub_key } | PoolCaller::Frontier | PoolCaller::Verifier)` **immediately before** `send_prepared_streaming`.
     3. If `Err(BudgetExhausted)`: send **0** HTTP bytes, record `stopped_at_cap: true` on the worker/stage outcome, and return partial work cleanly.
     4. Once HTTP send begins, the 1-call charge is **non-refundable** (`00-overview.md:625`: sent retries and cancelled-in-flight requests were received by the provider). Emit pre-send `model.request` / `verifier.request` / `idea.exchange_attempt` or always emit `delegate.turn` (even on error/cancel) before converting `in_flight -> committed`.
     5. Pass a `PreSendPermit` hook into `Transport` (or `ChatGptTransport` `responses.rs:142-151`) so a `401` token-refresh retry acquires a second permit + records `retry_count: 1` before firing its second HTTP POST.

2. **Verified Mode Reserve Arithmetic (Proven by Carbon Cross-Mode Audits):**
   - **Investigate (`Orient` cap `8`, `Open` cap `10`):**
     - `runner.rs:409-417` aborts *before* `dispatch_tool_calls` if `ToolUse` occurs on the final allowed turn. Therefore recording `record_map_reading` after workers return requires **minimum `frontier_reserve = 2`** (`1` `ToolUse(record_map_reading)` + `1` final `EndTurn`).
     - On `Orient` (`cap = 8`), `investigate.md:476-479` already pre-injects title/abstract/headings/intro/conclusion deterministically (`0` worker calls default). If `Open` (`cap = 10`) dispatches `paper_reader`, `worker_pool = 10 - 1 (dispatch) - 2 (frontier_reserve) = 7`. Pre-inject verified slice lines into each reader's initial `user_msg`, allow 1-turn completion when `note_evidence` succeeds on `wturn == max_turns` (`paper_read.rs:249` fix), and clamp slice count to `worker_pool.min(max_parallel)`. On the penultimate Orchestrator call (`remaining == 2`), restrict advertised tools to `record_map_reading` (`wrapup`-style) so the Orchestrator cannot burn its closing call on another read.
   - **Explore (`Quick` cap `5`, `Standard` cap `24`, `Thorough` cap `64+`):**
     - Derive stage pools dynamically from `(preset.max_model_calls, remaining)` instead of fixed stage caps that sum past `24`:
       - Before `X10` Passage Readers, hold `frontier_reserve = 6` on Standard/Thorough (`3` for `X11` Map `1 + <=2` repairs + `3` for `X13` Directions `1 + <=2` repairs; plus `X16` reserve on Thorough; on Quick cap `5`, `X10` is `0` by design).
       - Enforce atomic `calls_per_paper` sub-cap inside `X10` (`3` on Standard, `4` on Thorough, covering initial call + `want_lines` + repairs combined).
       - Scope `X15` Conversation to its own per-turn `RunScope::RunId` budget (`max_turns = 8`, overriding `turn_runner.rs:174`'s `48`) so finishing the map at `22/24` never starves follow-up Q&A (`explore.md:126`).
   - **Solve (`max_model_calls = 60`, `max_model_calls_per_leg = 48`, `max_review_loops = 6`):**
     - Enforce `min(run_remaining, leg_remaining)` with `SOLVER_RESERVE = 2` (`step_record` + `solve_finish`/checkpoint) and `VERIFIER_RESERVE = 5` (when `setup.review != ReviewPolicy::Off` and closing review has not yet run).
     - Clamp `worker.max_turns = worker.max_turns.min(available_for_workers)` before spawning `compute_worker` / `compute_blind`. Remove `REVISION_TURN_RESERVE = 8` (`autonomous.rs:550`) in Solve mode so caps are hard ceilings (`PauseReason::Budget` / `PauseReason::LegCap`).

---

### 2.4 Transport & Token Usage Accounting (`Option<u64>`, Never False `0`)

1. **Scoped OpenAI SSE Usage Opt-In:**
   - Keep `stream_http::inject_stream_flag` (`crates/agent/src/stream_http.rs:24-32`) **unchanged** because `AnthropicTransport::prepare_request` (`anthropic.rs:81`) calls it (Anthropic rejects `stream_options` with HTTP `400`).
   - In `OpenAiTransport::prepare_request` (`crates/agent/src/openai.rs:129-133`), inject `"stream_options": {"include_usage": true}` **only when `self.stream && self.provider == ProviderKind::OpenAi`** (never when `self.stream == false` on `http://` loopback/Ollama, and with a 1-time strip-on-400 fallback for custom OpenAI-compatible gateways that reject `stream_options`).
2. **Honest `TokenUsage` Extraction (`crates/agent/src/usage.rs`):**
   - Replace `delegate.rs:231-235` and `session_summary.rs:203-217` with:
     ```rust
     pub struct TokenUsage {
         pub input: Option<u64>,
         pub output: Option<u64>,
     }
     ```
     evaluating `.and_then(Value::as_u64)` **per candidate field** (`input_tokens` -> `prompt_tokens` -> `promptTokenCount`, fixing `delegate.rs:232`'s `Some(Value::Null)` short-circuit bug).
   - Roll up per `(role, provider, used_model)` into `{ calls: u32, input_tokens: u64, output_tokens: u64, unknown_usage_calls: u32 }`: when `unknown_usage_calls > 0` and `calls == unknown_usage_calls`, serialize token total as `null` (`"unknown"` in UI); when mixed, report known sum + `unknown_usage_calls` count (`"1,420 tokens + 2 calls without usage"`), **never `0`** (`00-overview.md:628-629`).
3. **Complete Emitter Coverage:**
   - Record `run_id`, `caller`, `role`, `provider`, `used_model` (`resp.model`), and `usage: { input_tokens: Option<u64>, output_tokens: Option<u64> }` on `model.response`, `delegate.turn`, `delegate.end` (`used_model` in addition to requested `model`), `explore.branch`, `explore.end`, `idea.exchange`, `proof.verification` / `claim.review` (`verifiers.rs:665`, always writing `"verifier_model"` to match `solve_state.rs:369`), `strategist.consult` (`strategist.rs:150`), and `narrator.report` (`narrator.rs:224`).

---

### 2.5 Worker Activity, Check-Evidence Inspection & Reader Controls (`00-overview.md:638-652`)

1. **Main Surface Stays Clean (`00-overview.md:621, 648`):**
   - Extend `run_bar.js` (`crates/desktop-server/assets/run_bar.js:141-225`) via its existing `extras[]` slot with compact live badges only (`"2 readers · haiku-4-5"`, `"7 of at most 24 model calls"`), keeping worker internal transcripts and scratch SymPy runs out of the paper outline, idea map, and proof step list.
2. **Lazy 0-Model-Call Inspection Endpoints (`crates/desktop-server/src/worker_activity.rs`):**
   - `GET /api/sessions/{id}/workers` -> list of worker/exchange/verifier jobs folded from `delegate.*`, `explore.*`, `idea.exchange`, `verifier.*` with `{ job_id, run_id, stage_or_task, role, provider, requested_model, used_model, routing_reason, calls_used, usage, status, disposition: "accepted"|"rejected"|"partial"|"pending", disposition_reason, artifact_refs }`.
   - `GET /api/sessions/{id}/workers/{job_id}` -> on-demand detail reading request/response/tool blobs, exact input line ranges (`context.supplied`) or input handles, tool receipts, returned `ev-*`/`xe-*`/`name@v` items, and explicit `"artifact_missing": true` if a referenced blob/file is absent (`00-overview.md:651`). Clearly labels model prose summaries `(worker model summary — unverified)` separately from deterministic tool/quote receipts (`00-overview.md:650`).
   - `GET /api/sessions/{id}/solve/checks/{check_id}` (owned with S8/S9) -> exact `solve.expr` handles (`latex`, `srepr`, `free_symbols`), setup domain/assumptions, sample points/seed/precision/tolerance/residual/counterexample, `code_sha256`, Python/SymPy version from `SolveReceipt.env`, and plot/table artifact links (`00-overview.md:640-641`).
3. **Reader Controls (`00-overview.md:644`):**
   - Append lightweight control events (`run.control`: `focus_note`, `pin_worker_item`, `dismiss_worker_item`, `pause_after_stage`, `next_run_roles`, `budget_delta`) consumed at the next stage/leg boundary without mutating completed receipts.

---

### 2.6 Offline Test Seams (`00-overview.md:631`)

Build `crates/agent/tests/eval_support/dual_scripted.rs` extending existing `ScriptedTransport` (`eval_support/scripted.rs`) and `RoutedMock` (`eval_support/routed_mock.rs`):
- **Two distinct `Transport` instances** (`frontier_transport` reporting e.g. `claude-sonnet-4-6`, `worker_transport` reporting e.g. `claude-haiku-4-5` or `gpt-4.1-mini`) with independent request logs, barriers (to force deterministic concurrent race interleaving), programmable HTTP `500`/`401`/mid-SSE-cancel faults, and programmable `usage: None` / `{"input_tokens": null, "prompt_tokens": 42}` payloads.
- **`DecideExtMockApprover`** implementing `Approver::decide_ext` (unlike `tests/support/mod.rs:83` which only implements `decide`) so tests exercise `approve_for_session: true/false`, `WorkerSession` epoch restarts, and per-caller `NetworkReadGrant` isolation with zero live network calls.

---

### 2.7 Separation: Reusable Today on `gemini/work/fixed` vs Waiting on In-Flight Packages

| Capability | Reusable Today on `f68999d6` | Hook Depending on In-Flight Mode Package |
|---|---|---|
| **Tool profiles & policy clamps** | `tool_profile.rs` (`research/1`, `investigation/1`), `policy` (`Deny > Ask > Allow`, `NetworkRead -> Ask`), `ApprovalStore` (`approval.rs`). | `F5/F7` (`f5-f7-mode-shell`): `session_mode.rs` + `turn_runner.rs` mode router wiring; `X3`/`S11`: `explore/1` & `solve/1` built profiles; `S3` (`q-f11-x1-s3-s6`): `Gate` trait on `Router`. |
| **Quote & line provenance (F4)** | `quote_core::{verify_quote, accept_quote}`, `lines_sent::{lines_sent_with_ledger, ContextSuppliedPayload}` (merged!). | `V6` (`inv-v6-tasks`): `read_map_node` crediting in `lines_sent.rs:456` + `record_map_reading`; `X10/X15`: multi-paper Explore cite checker; `investigation_cites.rs` switch to `lines_sent_with_ledger`. |
| **Budget counting & UI run bar (F9)** | `run_budget::{count_model_calls_breakdown, RunState, InvestigateTaskState, ExploreStageState}` + `assets/run_bar.js` (merged!). | New `budget_pool.rs` (`RunBudgetPool` atomic pre-send leases) + wiring into `runner.rs` (after <600L split), `exchange.rs` (`X6–X13`), `V6` tasks, and `S12` `solve_loop.rs`. |
| **Worker sub-loops & resume isolation** | `delegate.rs`, `paper_read.rs` (`effort="low"`, 3-tool isolated router, 24 KiB cap), `explore.rs`, `resume.rs:289-312` (keeps worker history out of Orchestrator `messages`). | `role_routing.rs` (eliminates silent `isolated_primary_model` + `.ok()` swallowing), Path I-A verified-line `context.supplied` in `paper_read.rs` (after <600L split), and `agent/src/exchange.rs` for Explore stages. |
| **Solve state & tier engine (S2)** | `solve_model*.rs`, `solve_state.rs`, `solve_own_tier.rs`, `solve_status.rs` (merged!). | `S6` (`q-f11-x1-s3-s6`): outbox handle harvest -> `solve.expr`; `S4/S8/S9/S12/S14/S20`: caller-aware `pending_runs`, 4-slot `SolveWorkerPool`, `appcontainer.rs` role profiles, `/solve/checks/{id}` inspector. |

---

## 3. Seven-Scenario Stress-Test Matrix

### 3.1 Unavailable Transports (Preflight & Mid-Run)
- **Preflight (missing API key, expired ChatGPT token, CLI selected for worker/pre-`X18` Explore, unreachable local Ollama):**
  - `resolve_pinned_run_roles` returns `ResolvedRole::Unavailable` (never `.ok()` -> `primary_model`).
  - Deterministic actions remain 100% live (`00-overview.md:612`): Investigate PDF upload + `V2–V5` skeleton outline + search/read; Explore manual paper ingest + saved map browsing; Solve `S4` setup editing + manual `check_run`.
  - Optional sub-loop workers (`paper_reader` in Investigate, `compute_worker` in Solve) disable cleanly with an inline notice unless the user opts into `allow_frontier_fallback: true`. Required stage workers (`Explore X8/X10`, `Solve verifier` when `ReviewPolicy != Off`, `Solve S20 compute_blind`) pause at the setup/stage card (`RunState::WaitingForYou` / `Failed` pre-send, `0` model calls spent).
- **Mid-run transport error (HTTP `500`/`429`/timeout on worker slice or verifier):**
  - **Investigate:** `paper_read.rs:265-272` records the failed slice as a `ReaderGap` (`reason: "reader model error: ..."`), keeps verified `ev-<seq>`/`evidence.quote` pointers from sibling slices, emits `delegate.turn` (charging the sent attempt) + `delegate.end`, and hands surviving slices + explicit gap ranges to the Orchestrator so it can read the gap directly or finish `partial`.
  - **Explore:** `exchange.rs` logs `idea.exchange` with error metadata, pauses the stage as `ExploreStageState::Failed` -> `RunState::Interrupted` (`Retry` button in `run_bar.js`, reusing pinned inputs + already-saved `xe-<seq>` quotes from completed papers).
  - **Solve:** Non-`SameModel` verifier transport failure emits `solve.paused { reason: "transport_error" }` (`RunState::Interrupted`), **never** falling back to `solver` (`autonomous.rs:438` bypass removed for Solve).

### 3.2 Missing Token Usage
- **Triggers:** Local Ollama / custom OpenAI-compatible SSE omitting `usage` (`stream_codecs.rs:310` `"usage": null`), provider mid-stream disconnect/cancel, or legacy `proof.verification` events without `usage`.
- **Behavior:**
  - `RunBudgetPool` and `run_budget::count_model_calls` count **provider requests** (`1` integer per sent call), so F9 call caps (`8`, `10`, `24`, `60`) remain 100% enforced even when every response omits `usage`.
  - `extract_usage_tokens_opt` yields `TokenUsage { input: None, output: None }`; ledger records `"tokens_in": null, "tokens_out": null` and increments `unknown_usage_calls`. Session summary and worker inspector render `"unknown"` (or `"N tokens + K calls without usage"`), never `0`.

### 3.3 Concurrent Dispatch
- **Investigate (`<= 8` reader slices) & Explore (`<= 8` `X10` passage readers):**
  - **Ledger safety:** SQLite WAL + `busy_timeout` + `append_at_head` retry loop (`investigation_evidence.rs:248-259`) serializes concurrent appends without corrupting the hash chain (`paper_read_tests.rs::readers_run_concurrently_barrier` verified green).
  - **Budget race safety:** `RunBudgetPool::try_acquire` uses a single mutex/atomic check-and-decrement *before* each HTTP send; 8 threads competing for 5 remaining worker permits deterministically grant 5 and stop 3 cleanly with `0` overshoot.
  - **Approval modal race safety:** Adding a per-`(session_id, caller, effect)` single-flight lock in `UiApprover` (`approver.rs:158-201`) + keying `session_capability_grants` by `(session_id, caller, policy_version, effect)` prevents N concurrent approval modals AND blocks cross-caller grant leakage (`host:idea` -> `agent:`!).
- **Solve (`SolveWorkerPool`, max 3 concurrent Python processes per `solve.md:276`):**
  - **Process & `AppContainer` isolation:** Separate `scratch/{solver,check,verifier,blind}` and `pycache/{solver,check,verifier,blind}` subdirectories + role-qualified Windows AppContainer profiles (`re-search.w.<role>.<hash_or_sid>` <= 60 chars, `.worker.<role>.lock`, never deleting another live role's profile on `Drop` in `appcontainer.rs:110-122`).
  - **Epoch & `Router` isolation:** Dedicated sub-`Router`s for `Solver` (`agent:*`, `worker:compute_worker:*`), `Verifier` (`verifier:solve`), and `Blind` (`worker:compute_blind:*`) each bound to its own `EpochHandle` so restarting Verifier/Blind never invalidates Solver's `WorkerSession` grant, while restarting Solver invalidates both `solver` and `compute_worker` (`PauseReason::WorkerRestarted`).

### 3.4 Cancellation After a Send
- **Sequence:** User clicks `Stop` (`POST /api/sessions/{id}/stop` -> `turn_control::handle_stop_turn` at `turn_control.rs:45-80`, extended with `StageTurnGuard` so async Explore stages `X6–X13` are registered in `api_turns` + `active_turns` too!).
- `CancelToken::cancel()` fires + `state.deny_pending` unblocks any waiting `UiApprover` modal immediately.
- `stream_http::drive_channel` (`stream_http.rs:57-95`) aborts within `40 ms` with `TransportError::cancelled()`.
- **Accounting & partial preservation:**
  - Because the HTTP request already left the machine, the pre-send `RunBudgetPool` permit stays **charged (`+1`)** and `delegate.turn` / `idea.exchange` / `model.request` records `cancelled: true, usage: null`.
  - Any `investigation.evidence`, `evidence.quote` (`xe-<seq>`), or `solve.expr` (`name@v`) committed before `Stop` remains durable in `ledger.db`.
  - **Fix to `paper_read.rs:535-537`:** Instead of overwriting the entire `delegate_read_paper` result with `"DENIED: stopped by user"` (which currently discards completed sibling slices!), commit `delegate.end { ok: false, cancelled: true }` preserving completed slice records while marking unfinished slices in `gaps`, and finalize the task as `Stopped` -> `RunState::Paused` (`Continue` enabled, `0` unreferenced blobs).

### 3.5 Capped Partial Work
- **Investigate (`Orient`/`Open`):** When `worker_pool` hits `0`, active readers finish their in-flight call, remaining slices record `ReaderGap { reason: "stopped_at_cap" }`, and **`frontier_reserve = 2` remains untouched** so the Orchestrator still calls `record_map_reading` (`1`) + `EndTurn` (`1`) to save anchored nodes for the read sections and finishes as `InvestigateTaskState::Partial` (`RunState::Paused`: `"Paused: budget reached · 8 of at most 8 model calls"` + `Continue`).
- **Explore (`X10` -> `X11`):** If paper #5 hits `BudgetExhausted` mid-repair, `0` extra HTTP calls fire; `xe-<seq>` quotes from papers #1–#4 remain saved; stage `X10` enters `stopped_at_cap` (`RunState::Paused`), offering **both** `Continue (+N calls)` and `"Map what we have"` (which runs `X11` from the reserved frontier calls and marks unscreened core papers as `incomplete` under F12/V3 — **never** `none_in_scope`, `00-overview.md:632-635`).
- **Solve:** When `available_for_workers == 0`, `delegate_compute` / `compute_blind` returns immediately with `stopped_at_cap` (`0` calls), leaving `SOLVER_RESERVE = 2` so `solver` records any pending step + `solve.checkpoint` and pauses cleanly with `PauseReason::LegCap` / `PauseReason::Budget`.

### 3.6 Duplicate Task Requests
- **In-flight overlap:** `reserve_turn` (`.active.lock`) + `active_turns` + `StageTurnGuard` reject a *different* concurrent task on the session with `409 Conflict`, while an identical `(session_id, task_key)` retry returns `200`/`202` pointing to the running `{ request_seq, run_id }` without spawning a duplicate turn.
- **Completed task reuse (`00-overview.md:618`):** Each mode computes a canonical `task_key`:
  - *Investigate:* `(task_kind, source_id, primary_sha256, skeleton_seq, target_node_or_pages, focus_norm)` (`rechecks` at `investigation.rs:472-496` gains this check too).
  - *Explore:* `(stage, brief_sha256, corpus_sha256, preset)`.
  - *Solve:* `(check_kind, claim_sha256, params_canon)` for `check_run` / `compute_blind`.
  If the latest terminal record for `task_key` is `Done` and `action != "rerun"`, return the saved result immediately (`reused: true`, **`0` model calls**).
- **Continuation after `Partial`/`Stopped`:** `action: "continue"` chains `parent_request_seq` under the same `task_key` lineage, raises the cap via `ModelCallBudget::with_raised_cap`, and widens `investigation_evidence.rs:331` / `paper_read.rs:334` to accept prior `ev-<seq>` (or F4 `evidence.quote`) from any `marker_seq` in the same `(task_key, file_sha256)` lineage so workers do not re-read completed sections!

### 3.7 Orchestrator Citing a Worker's Finding (Preserving `F1`, Containment, Policy & `F4`)
- **Investigate (`Path I-A` — exact-surviving-row `context.supplied`, selected over `Path I-B`):**
  - *Why not `Path I-B`?* Requiring the Orchestrator to re-call `read_paper_lines` on every worker-found span costs `1–4` extra frontier turns, blowing Orient (`cap = 8`) and Open (`cap = 10`).
  - *Exact `Path I-A` contract (closing all 5 traps found by Carbon Audit #1):*
    1. For each worker-noted span, **host Rust code** (never trusting worker `quote`/`gist` text!) loads the exact numbered lines `start_line..=end_line` from SHA-256-verified `SourceIndex` wrapped in `<quoted_source source_id=... path=... lines="a-b">...</quoted_source>` (`THREAT-MODEL.md:228, 357`, `investigate.md:539` rule 6.19).
    2. `assemble_return_json` (`paper_read.rs:355-384`) enforces the `<= 24 KiB` ceiling by dropping whole excess rows into `omitted` (**never** clipping mid-span lines!).
    3. **Only after** `!is_err` and the final `canonical` JSON is committed to `delegate.end.raw_blob`, host code appends `context.supplied { run_id, caller: orchestrator_caller, ranges }` covering **exclusively the surviving rows actually present in `canonical`**, and exempts active-task `delegate_read_paper` `ToolResult`s from `mask_old_tool_outputs_stepped` (`guardrails.rs:115`).
    4. Result: The Orchestrator genuinely sees the exact verified source lines in context, `quote_core::accept_quote` and `lines_sent_with_ledger` pass with zero weakening of `F4`, and `tool_defs.txt` / `investigation/1` remain untouched.
- **Explore (`X10` -> `X11` -> `X15`):**
  - `X11` (`record_idea_map`) cites `xe-<seq>` / `xa-<seq>` IDs (`C4` forbids raw quotes) + host emits `context.supplied` for the `xe-<seq>` excerpts shown in `X11`'s prompt.
  - `X15` (`explore/1` Conversation) emits `context.supplied` for `agent:<provider>:<session>` covering **only** the exact `[start_line, end_line]` spans of `xe-<seq>` quotes injected into `X15`'s turn context, validated by a multi-paper F4 cite checker scoped to the current turn's `run_id`/`SeqRange`.
- **Solve (`compute_worker` & `S20` `compute_blind` -> `solver`):**
  - **Caller-aware `pending_runs` (`solve_state.rs:190-266`):** Record `caller: Option<String>` on `InFlightTool` and `SolveReceipt`. Only `caller == None` (preserving S2 unit tests!) or `caller.starts_with("agent:")` enters `SolveState.pending_runs`. `compute_worker` scratch runs are tracked per delegation call and retired on `delegate.end` (which lists `handles_created` + `producing_call_ids`), while `retire_cited_runs` also retires `handle.produced_by.call_id` for handles cited in `step_record.claims`. `verifier:solve` and `worker:compute_blind:*` **never** enter `pending_runs` and are exempt from Solver Gate `G1`.
  - **Blobbed receipt headers (`router.rs:425`):** Promote `{ code_sha256, wall_ms, process, exprs_out, env, sympy_ok }` onto top-level `tool.result` even when `result` >4 KiB goes to a blob so `solve_state.rs:443` never mistakes a >4 KiB Python traceback for a successful run or loses `code_sha256`.
  - **`S20` `compute_blind` (`solve_own_tier.rs:324-332`):** Runs in isolated **Blind Worker** (`scratch/blind`, Gate `G3` strips `compare_with`), harvests `output@v` + compares in **Check Process**, emits `solve.check` + returns `recommended_claim` so `solver`'s `step_record` produces a byte-identical `claim_sha256`, and hardens `solve_own_tier.rs:324` to require `agreed != false && value_two_runs_differ && blind_caller_provenance`.

---

## 4. Independent Cross-Mode Reviews, File Ownership & Merge Plan

### 4.1 Summary of Disjoint Carbon (`pro`) Hostile Reviews

Three independent read-only Carbon (`Model: "pro"`) subagents audited the initial F9b draft separately for Investigate, Explore, and Solve. Every flaw they caught was verified against source files on `f68999d6aef1afa780d100a9bc746d6b6372539a` and incorporated into §§1–3 above:

| Subagent | Key Flaws Caught in Initial Draft / Codebase & Fixed in Final Design |
|---|---|
| **Carbon #1 — Investigate** | (1) Caught `paper_reader.rs:455` `!is_section_scoped(...)` bypassing both `isolated_primary_model` and `reader_provider_configured`, plus `turn_runner.rs:309` leaving `literature_worker` active on paper-reading turns; (2) Proved `frontier_reserve` must be **`>= 2`** (not `1`) due to `runner.rs:409-417` pre-dispatch `max_turns` abort, and caught `paper_read.rs:249` rejecting 1-turn `note_evidence` completion; (3) Identified 5 `Path I-A` traps (`24 KiB` row omission desync, cancel `"DENIED"` overwrite, missing `run_id`/`caller` on `Audit`, `<quoted_source>` injection framing, `guardrails.rs:115` stepped tool masking); (4) Caught `marker_seq` lock (`paper_read.rs:334`, `investigation_evidence.rs:331`) breaking `ev-<seq>` reuse across `Stop`/`Continue`; (5) Flagged `runner.rs` (626L) & `paper_reader.rs` (599L) F0 line-limit blockers. |
| **Carbon #2 — Explore** | (1) Proved fixed stage caps (`3+3+12+4+4=26 > 24`) overflow Standard cap `24`, break Quick cap `5`, and starve Thorough (`12 * 4 = 48`) and `X15` (`max_turns = 48` default), replacing them with dynamic reserve math + `calls_per_paper` sub-cap + per-turn `RunScope::RunId` for `X15`; (2) Caught **critical cross-caller `session_capability_grants` leak** (`state.rs:170` missing `caller` & `policy_version`), `auto_approve_tools = true` bypassing `X7` plan approval, duplicate concurrent modal race, and `POST /stop` returning `409` on unregistered async stage threads; (3) Caught **`AnthropicTransport` sharing `stream_http::inject_stream_flag` (`anthropic.rs:81`)** and `delegate.rs:232` `Value::Null` `.or_else` bug; (4) Proved `X15` cannot reuse `investigation_cites.rs` and needs `idea.stage` role snapshots on `Continue`. |
| **Carbon #3 — Solve** | (1) Caught **fatal R6 violation** if `verifier:solve` `sympy_eval` ran in the Check Process (must use a separate Verifier Worker, plus isolated Blind Worker for `S20` keeping live workers `<= 3`); (2) Caught **Windows `AppContainerProfile::drop` (`appcontainer.rs:110-122`) deleting `"re-search.worker.<session>"` while sibling Solve workers are still running**; (3) Verified `SolveState.pending_runs` (`solve_state.rs:199`) deadlocking Gate `G1` on verifier/blind/worker-scratch runs AND >4 KiB blobbed Python tracebacks (`443-447`); (4) Caught `solve_own_tier.rs:324` `independent_recompute` missing `value_two_runs_differ`/blind-caller checks and `S20` `claim_sha256` mismatch; (5) Caught `verifiers.rs:680` writing `"model"` only on `on_demand` while `solve_state.rs:369` reads `"verifier_model"`, `responses.rs:142` unbilled 401 retry, and `relay.rs:189` blocking internal Solve callers on CLI route. |

---

### 4.2 Concrete Proposed File & Test Ownership (`<= 600` Lines per File, Zero Lane Collisions)

Measured line counts on `f68999d6aef1afa780d100a9bc746d6b6372539a` show several files at or above the 600-line F0 ceiling (`runner.rs` 626, `verifiers.rs` 878, `cli_turn.rs` 626, `routes_session.rs` 616, `routes_tools.rs` 610, `http_srv.rs` 702, `session_view.rs` 753, `paper_reader.rs` 599, `state.rs` 598, `investigation.rs` 597, `delegate.rs` 594). New F9b logic **must** live in new `< 400`-line modules, with mechanical pre-splits before touching any >600L or cliff file, respecting active briefs (`f5-f7-mode-shell`, `inv-a-v2-v5`, `inv-v6-tasks`, `q-f11-x1-s3-s6`, `solve-evidence-labels`, `lib-f13a-f16a-f15`):

| Sub-Package / Sequence | New Files Owned (Est. Lines) | Existing Files Touched (Current -> Est. Lines, all `<= 600`) | Owned Tests |
|---|---|---|---|
| **F9b-1: Core Budget Pool, Usage & Role Types** *(can run now; zero overlap with active briefs)* | `crates/research-tools/src/budget_pool.rs` (~280)<br>`crates/research-tools/src/budget_pool_tests.rs` (~240)<br>`crates/agent/src/usage.rs` (~180)<br>`crates/agent/src/role_routing.rs` (~300)<br>`crates/agent/src/responses_auth.rs` (~120)<br>`crates/agent/tests/eval_support/dual_scripted.rs` (~220) | `crates/research-tools/src/lib.rs` (+1 merge note)<br>`crates/agent/src/lib.rs` (99 -> ~103)<br>`crates/agent/src/openai.rs` (492 -> ~510, scoped `include_usage`)<br>`crates/agent/src/responses.rs` (587 -> ~515 after extracting `responses_auth.rs`, +401 retry permit)<br>`crates/agent/src/strategist.rs` (209 -> ~235, usage/model)<br>`crates/agent/src/narrator.rs` (245 -> ~270, usage/model) | `budget_pool_tests.rs` (concurrency, `frontier_reserve`, `calls_per_paper`), `usage.rs` unit tests (`None` vs `0`, `Value::Null` precedence, OpenAI SSE vs HTTP/Anthropic), `role_routing` unit tests. |
| **F9b-2: Runner Split, Delegation & Reader Handoff** *(after `f5-f7-mode-shell` lands)* | `crates/agent/src/runner_audit.rs` (~185)<br>`crates/agent/src/runner_dispatch.rs` (~140)<br>`crates/agent/src/delegate_specs.rs` (~160)<br>`crates/agent/src/paper_read_worker.rs` (~260)<br>`crates/desktop-server/src/role_pin.rs` (~290)<br>`crates/desktop-server/src/paper_reader_plan.rs` (~230)<br>`crates/desktop-server/src/worker_activity.rs` (~310) | `crates/agent/src/runner.rs` (626 -> ~365, `run_id` + `caller` + `RunBudgetPool`)<br>`crates/agent/src/delegate.rs` (594 -> ~435, post-`prepare_request` turn count + error `delegate.turn` + `Option` usage)<br>`crates/agent/src/paper_read.rs` (573 -> ~330, Path I-A verified-line `context.supplied`, 1-turn `note_evidence`, cancel partial keep)<br>`crates/desktop-server/src/paper_reader.rs` (599 -> ~250, remove `!is_section_scoped` bypass)<br>`crates/desktop-server/src/approver.rs` (284 -> ~325) & `session_lifecycle.rs` (231 -> ~265, caller-keyed grants + single-flight lock)<br>`crates/desktop-server/src/turn_control.rs` (198 -> ~240, `StageTurnGuard`)<br>`crates/desktop-server/src/session_summary.rs` (217 -> ~270, per-role/model `Option` usage)<br>`crates/desktop-server/assets/run_bar.js` (284 -> ~320) | `crates/desktop-server/tests/f9b_routing_and_budget.rs` (~420, testing all 7 stress scenarios end to end over `dual_scripted` loopback mocks). |
| **Explore Hooks (`X3/X6–X15`)** | `crates/agent/src/exchange.rs` (~360) + `exchange_tests.rs` (~280)<br>`crates/desktop-server/src/idea_cites.rs` (~240) | `idea*.rs` (`idea.started.roles` + `idea.stage` `roles`, dynamic Quick/Standard/Thorough reserve math, `X15` `context.supplied`). | `exchange_tests.rs`, `idea_eval.rs` dual-transport & partial-map (`incomplete` vs `none_in_scope`) tests. |
| **Solve Hooks (`S3/S6` follow-up + `S4/S8/S12/S14/S15/S20`)** | `crates/desktop-server/src/solve_runtime.rs` (~270, 4-slot `SolveWorkerPool`)<br>`crates/desktop-server/src/solve_grants.rs` (~220) | *After `q-f11-x1-s3-s6` & `solve-evidence-labels` merge:*<br>`crates/tools/src/router.rs` (504 -> ~535, top-level receipt fields on blob + `run_id`)<br>`crates/worker/src/appcontainer.rs` (377 -> ~430, role-scoped profiles/locks/scratch)<br>`crates/research-tools/src/solve_model_records.rs` (351 -> ~375), `solve_state.rs` (517 -> ~560, caller-aware `pending_runs`), `solve_own_tier.rs` (415 -> ~445, `S20` provenance)<br>*In `S14`/`S15`:* split `verifiers.rs` (878 -> 2 files `<480`L) and `cli_turn.rs` (626 -> 2 files `<420`L) + `relay.rs` (403 -> ~425). | `solve_state_tests.rs` (verifier/blind/scratch `pending_runs` non-pollution, >4 KiB traceback blob), `solve_own_tier` `S20` anti-self-certify test, Windows multi-worker `AppContainer` lifecycle test. |

---

### 4.3 Unresolved Design Decisions Recorded & Recommended Defaults

Because this audit ran unattended (`RULES.md` + user instructions), we decided and recorded concrete defaults for every ambiguity encountered; the owner/integrator may override any item before F9b implementation lands:

1. **Decision D-F9b-1 (Investigate `paper_reader` default when unassigned vs `paper_readers_enabled`):**
   - *Ambiguity:* Legacy Investigate Decision 4 (`paper_reader_wiring.rs:252`) intentionally routed `paper_readers_enabled: Some(true)` + unassigned `paper_reader` to `"isolated_primary_model"`, whereas `00-overview.md:614-615` states *"A separate context on the same frontier model does not count as a smaller-model assignment"* and requires an explicit frontier-fallback setting.
   - *Decision recorded:* For new V6 `investigation/tasks`, `paper_reader` uses `RoleRoutingPolicy` (auto-smaller tier if available on a connected provider; else disabled unless `allow_frontier_fallback: true` is explicitly set on the task/preset). Legacy `/messages` with `paper_readers_enabled: Some(true)` keeps `"isolated_primary_model"` as a compatibility alias so existing WP8 tests stay green, **except** the `&& !is_section_scoped(...)` bug (`paper_reader.rs:455`) is removed everywhere.
2. **Decision D-F9b-2 (Explore `X10` Passage Reader role key):**
   - *Ambiguity:* Should Explore `X10` share `paper_reader`, share `literature_worker`, or expose its own `idea_reader` role key?
   - *Decision recorded:* Default `X10` to `idea_reader` falling back to `literature_worker` (not legacy `paper_reader`), so configuring one smaller literature/screening model in Settings covers both `X8` Triage and `X10` Passage Readers while allowing advanced users to split them.
3. **Decision D-F9b-3 (`auto_approve_tools` in Explore `X7` and Solve `D5`):**
   - *Ambiguity:* Desktop `ui.auto_approve_tools` defaults to `true` (`state.rs:80-82`), which silently skips `explore.md` `P5` search-plan confirmation and `solve.md` `D5` setup code-execution confirmation.
   - *Decision recorded:* Treat Explore's `X7` search plan card and Solve's `D5` setup Run card as **mode workflow checkpoints** (always shown once per plan/setup unless run via headless eval with explicit `--approve`), and key `DesktopState::session_capability_grants` strictly by `(session_id, caller, policy_version, effect)` (plus `worker_epoch` for `ExecuteCode`).
4. **Decision D-F9b-4 (ChatGPT `401` refresh retry vs F9 budget cap):**
   - *Decision recorded:* Under `00-overview.md:625` (*"Charge each provider request once, including any sent retry"*), both the initial `401` POST and the post-refresh POST in `responses.rs:142-151` count (`+2` total if both were sent over the network); if `RunBudgetPool` has `0` permits left after the `401`, stop cleanly before sending POST #2.

---

### 4.4 Merge Notes

- **Zero merge conflicts from this branch:** Only `docs/design/modes-2026-11/reports/audit-orchestration-routing.md` is added.
- **Recommended implementation order for follow-up lanes:**
  1. Land active foundation lanes (`f5-f7-mode-shell`, `q-f11-x1-s3-s6`, `solve-evidence-labels`, `inv-a-v2-v5`).
  2. Land **F9b-1 + F9b-2** (including the mechanical `<600`L splits of `runner.rs`, `delegate.rs`, `paper_read.rs`, `paper_reader.rs`, and the `session_capability_grants` caller-key fix) **before** or together with `inv-v6-tasks`, `Explore X6–X15`, and `Solve S12/S14/S20` so all three modes consume one `RunBudgetPool`, one `role_routing` resolver, and one `usage` extractor rather than inventing three incompatible copies.

---

## 5. Verification, Pre-Existing Issues, Linux Scope & Codebase Bug Catalog

### 5.1 Focused Test & Lint Commands Run on Base `f68999d6aef1afa780d100a9bc746d6b6372539a`

All tests used offline loopback/scripted fixtures only (zero external network or provider calls; `RE_SEARCH_PYTHON` left unset so Linux CLI tests are unaffected):

| Command | Result | Exact Counts |
|---|---|---|
| `cargo test -p research-tools --test tool_def_golden --test tool_profile_golden` | **PASS** | `tool_def_golden`: **3 passed, 0 failed, 0 ignored**<br>`tool_profile_golden`: **3 passed, 0 failed, 0 ignored** |
| `cargo test -p research-tools --lib -- quote_core lines_sent run_budget tool_profile solve_ investigation_evidence` | **PASS** | **49 passed, 0 failed, 0 ignored** (79 filtered out) |
| `cargo test -p tools` | **PASS** | **23 passed, 0 failed, 0 ignored** (0 doc-tests) |
| `cargo test -p agent --lib -- delegate paper_read explore resume guardrails stream_codecs provider_config` | **PASS** | **29 passed, 0 failed, 0 ignored** (20 filtered out) |
| `cargo test -p desktop-server --lib -- agent_models paper_reader session_view session_summary turn_control investigation_cites` | **PASS** | **27 passed, 0 failed, 0 ignored** (45 filtered out) |
| `cargo test -p desktop-server --test stage14_worker_delegation --test paper_reader_wiring --test eval_set_r` | **PASS** | `eval_set_r`: **5 passed, 0 failed, 0 ignored**<br>`paper_reader_wiring`: **7 passed, 0 failed, 0 ignored**<br>`stage14_worker_delegation`: **4 passed, 0 failed, 0 ignored** |
| `cargo test -p desktop-server --test audit_flow_readers_and_pages` | **PASS** | **4 passed, 0 failed, 0 ignored** |
| `node --test crates/desktop-server/tests/*.test.mjs` | **PASS** | **157 passed, 0 failed, 0 skipped** |
| `python3 scripts/check_file_length.py --self-test` & `--base origin/gemini/work/fixed` | **PASS** | Exit `0` (verified before commit) |

### 5.2 Pre-Existing Lint Failure on Linux (Verified on Unmodified Base `f68999d6aef1afa780d100a9bc746d6b6372539a`)

- Command: `cargo clippy --workspace --all-targets -- -D warnings` (`rustc 1.100.0 (0ef1bc386 2026-11-09)`)
- Result: Fails on unmodified base `f68999d6aef1afa780d100a9bc746d6b6372539a` with `clippy::manual_unwrap_or` at `crates/desktop-server/src/evidence.rs:744:40` (documented in `RULES.md:107-109`; `evidence.rs` is 1,057 lines and untouched). Note: running as non-root user `antigravity`, `paper_read::tests::try_call_err_aborts_all_and_fails_turn` **passed** (`RULES.md:110-112`).

### 5.3 What Could Not Be Compiled or Run on Linux (`#[cfg(windows)]`)

- `crates/worker/src/appcontainer.rs`, `crates/worker/src/win.rs`, `crates/worker/src/stage.rs` (`#[cfg(windows)]` Win32 `CreateAppContainerProfile`, `DeleteAppContainerProfile`, `icacls` SID grants, Job Objects) and `crates/desktop-app` (WebView2 shell) are `#[cfg(windows)]`-gated and cannot be compiled or executed on Linux; `appcontainer.rs` was audited statically line by line (`1-290`).

### 5.4 Bugs Noticed in Existing Code (`file:line`) — Not Fixed Under Audit-Only Scope

1. `crates/desktop-server/src/paper_reader.rs:455`: `&& !is_section_scoped(req, Some(&marker.payload))` skips both `built.routing_reason == "isolated_primary_model"` and `!reader_provider_configured(state, reader_provider)`, auto-enabling parallel frontier readers on section-scoped Investigation turns even when `paper_readers_enabled` is `None` or the configured reader provider has no API key.
2. `crates/desktop-server/src/turn_runner.rs:309-321`: `literature_worker` is not gated by `if paper_reading_turn { None }`, contradicting `investigation_prompts.rs:10-11`, and `paper_reading_turn` (`287-289`) omits `_paper_question_request` and `_paper_request`.
3. `crates/desktop-server/src/state.rs:170` & `session_lifecycle.rs:30-67` & `approver.rs:158-165`: `session_capability_grants` is keyed by `(session_id, EffectClass)` without `caller`, `tool_id`, `policy_version`, or `worker_epoch`, leaking session grants across distinct callers (`host:*`, `agent:*`, `worker:*`, `verifier:*`) and across restarted workers (violating `THREAT-MODEL.md:257-260`).
4. `crates/tools/src/router.rs:277`: For `ApprovalScope::WorkerSession` tools, `bind_session_digest` is bound unconditionally even when `outcome.approve_for_session == false`.
5. `crates/desktop-server/src/state.rs:495-549` + `crates/agent/src/transport.rs:98-104`: `DesktopState::build_transport` calls `from_env` before `providers.json`, causing `RE_SEARCH_API_KEY` set for one provider to override another provider's saved key in `providers.json`.
6. `crates/desktop-server/src/agent_models.rs:269, 278-280, 325, 349, 372`: `.ok()` silently swallows `build_transport` errors on explicitly configured roles and falls back to the primary frontier model.
7. `crates/agent/src/delegate.rs:232-233`: `usage.get("input_tokens").or_else(|| usage.get("prompt_tokens")).and_then(Value::as_u64)` short-circuits on `{"input_tokens": null, "prompt_tokens": 50}` and returns `0` instead of `50`; lines `232-234` and `session_summary.rs:203-217` coerce missing usage (`null`/`{}`) to `0`.
8. `crates/agent/src/openai.rs:131`: Live streaming OpenAI requests omit `"stream_options": {"include_usage": true}`, causing `stream_codecs.rs:310` to record `"usage": null` (reported as `0` tokens) on every streamed OpenAI response.
9. `crates/agent/src/delegate.rs:435`, `paper_read.rs:170`, `explore.rs:116`: `turns_used` is incremented *before* `transport.prepare_request`, over-charging `delegate.end.turns` when request serialization/validation fails locally without sending an HTTP call; conversely, failed/cancelled post-send turns omit `delegate.turn` (`delegate.rs:444`, `paper_read.rs:515`).
10. `crates/agent/src/verifiers.rs:665-688`, `strategist.rs:150-163`, `narrator.rs:224-236`: Drop `resp.usage` and `provider`; `verifiers.rs:680` writes `"model"` only when `params.on_demand == true`, whereas `solve_state.rs:369` reads `"verifier_model"`.
11. `crates/research-tools/src/solve_state.rs:199-201, 443-447`: Enqueues every caller's `sympy_eval` into `pending_runs` regardless of `caller`, and treats any >4 KiB blobbed `sympy_eval` output (`result_blob.is_some()`) as `sympy_succeeded = true` even when Python raised an exception.
12. `crates/research-tools/src/solve_own_tier.rs:324-332`: `"independent_recompute"` checks only `is_exact_comparison(check)`, omitting `agreed != false`, `value_two_runs_differ`, and blind-caller provenance.
13. `crates/worker/src/appcontainer.rs:110-122, 218-224`: `Drop` on `AppContainerProfile` / `AppContainerSandbox` deletes `"re-search.worker.<session>"` and `.worker.lock`, corrupting sibling workers in multi-process Solve sessions.

### 5.5 Design Contradictions & Tensions Identified

1. **Explore stage cap sum vs preset hard caps (`explore.md:130-134, 753-760` vs `00-overview.md` §3.4):** Summing per-stage worst-case exchange caps (`3 + 3 + 18 + 3 + 3 = 30` on Standard; `12` papers on Thorough vs a static `12`-call `X10` cap) contradicts Standard's hard cap of `24` and Quick's hard cap of `5` unless stage pools are derived dynamically from `(preset.max_model_calls - frontier_reserve)` with a per-paper sub-cap (§2.3).
2. **Solve `max_review_loops = 6` vs `max_model_calls = 60` (`solve_model_records.rs:90-105`):** Six multi-turn `S14` verifier loops (`6 * 8..12 = 48..72` calls) exceed the entire 60-call Solve session budget by themselves unless `RunBudgetPool` enforces a per-review-loop call cap (`<= 4` calls/loop) inside the shared 60-call pool.
3. **Investigate legacy Decision 4 (`isolated_primary_model` when `paper_readers_enabled: true`) vs `00-overview.md:614-615`:** Resolved via Decision `D-F9b-1` (§4.3) — strict explicit opt-in (`allow_frontier_fallback`) on all new mode tasks while keeping legacy `/messages` `paper_readers_enabled: true` backward-compatible.
4. **`S20` `compute_blind` signature (`solve.md:403`) vs `solve_own_tier.rs:188` `claim_sha256` matching:** Because `compute_blind` takes `{ inputs, operation, compare_with, tolerance }` without an `entry_id` or `Claim`, a `solve.check` written by `compute_blind` will never match a step's `claim_sha256` unless `compute_blind` returns `recommended_claim: { form, lhs: compare_with, rhs: output@v, relation }` for `solver` to pass verbatim to `step_record` (§3.7).
