# Brief f9b-routing-core — Model Role Pinning, Pre-Send Budget Leases, Honest Token Usage & Worker Activity Routes/UI (F9b Core)

**Status:** Unblocked on `gemini/work/fixed` (`F1`, `F4`, `F9`, and `reports/audit-orchestration-routing.md` are merged).
**Package:** `F9b Core` — builds the standalone, strictly file-disjoint foundation modules diagnosed in `reports/audit-orchestration-routing.md` so Investigate (`V6–V8`), Solve (`S12–S20`), Explore (`X6–X15`), and `DesktopState` / `agent::runner` can wire them cleanly without cross-lane file conflicts.
**Governing design:**
- `00-overview.md`: 3.4 (F9b orchestrator vs smaller workers, role pinning, no silent frontier fallback, pre-dispatch parallel budget reservation, frontier reserve, honest `unknown` missing usage, lazy worker-activity inspection & reader controls, dual scripted-transport acceptance).
- `reports/audit-orchestration-routing.md`: §2.1–§2.6 (exact audit inventory `F-1`..`F-9`, `ResolvedRole`, `RunBudgetPool`, `TokenUsage`, `OpenAiTransport` `stream_options`, worker-activity API, dual scripted tests).
- `agents/RULES.md` (`<= 600`-line limit, no `Cargo.lock` edits, non-root Linux workspace Clippy & tests, push after each checkpoint).

---

## 1. Strict Parallel Disjointness Rules (CRITICAL)

- Other active lanes own `turn_runner.rs`, `runner.rs`, `verifiers.rs`, `cli_turn.rs`, `state.rs`, `agent_models.rs`, `paper_reader.rs`, `investigation*.rs`, `solve.rs`, `idea_exchanges*.rs`, and `eval_harness.rs`.
- **Therefore, `f9b-routing-core` MUST implement its deliverables in strictly new standalone modules (+ tiny 1-line module/route/asset exports) and document the exact 1–3 line call-site wiring in `reports/f9b-routing-core.md` ("Merge notes")!**

---

## 2. Checkpoints — Push to `gemini/modes/f9b-routing-core` After Each!

### Checkpoint 1 — Shared Atomic Pre-Send Budget Reservation (`crates/research-tools/src/budget_pool.rs`)
1. Implement `RunBudgetPool` (`budget_pool.rs` + `budget_pool_tests.rs`, re-export from `crates/research-tools/src/lib.rs` `339`L; do **not** bloat `run_budget.rs`):
   - Initialized from `count_model_calls_breakdown(&events, scope)` (`committed`, `total_cap`, `frontier_reserve`, `verifier_reserve`, optional per-item `sub_caps: BTreeMap<String, SubCap>` e.g. Explore `calls_per_paper`).
   - `enum PoolCaller { Frontier, Worker { role: String, sub_key: Option<String> }, Verifier { role: String } }`.
   - `try_acquire(&self, caller: PoolCaller) -> Result<BudgetLease, BudgetExhausted>`: atomic check under single mutex guaranteeing `committed + in_flight + reserved_for_frontier_and_verifier < total_cap` before a single HTTP byte leaves the host.
   - `BudgetLease`: converts cleanly on send start (`mark_sent`) to non-refundable committed count (failed/retried/cancelled requests after HTTP send still count once per `00-overview.md` 3.4!), or releases unused slot automatically via `Drop` if `prepare_request` fails locally before HTTP send.
   - Add concurrent multi-thread stress tests proving parallel readers/workers stop cleanly (`BudgetExhausted`) without ever encroaching on `frontier_reserve` (`2` calls for Investigate/Solve closing, configurable for Explore) or exceeding `total_cap`!
2. Run mandatory checks and push Checkpoint 1 to `gemini/modes/f9b-routing-core`.

### Checkpoint 2 — Strict Role Pinning (`role_routing.rs`), Honest Usage (`usage.rs`), OpenAI SSE Opt-In & Dual Scripted Seam
1. Create `crates/agent/src/role_routing.rs` (+ `role_routing_tests.rs`):
   - Define `RoleRoutingPolicy`, `ResolvedRole::{Pinned { provider, model, base_url, routing_reason }, Disabled { reason }, Unavailable { configured_provider, configured_model, error }}`, `RoutingReason::{ExplicitRole, PresetRole, AutoSmallerTier, ExplicitFrontierOptIn, ExplicitSameModel}`, and `PinnedRoleRecord` (`serde` serializable for mode start/task/stage/setup events).
   - Implement unified precedence (`run_override` -> `UiSettings.agent_models` [supporting `"verifier"`, `"idea_reader"`, `"compute_blind"` + legacy aliases] -> `ProviderKeyStore.roles` [honoring per-role `base_url`] -> static Catalog-independent smaller-tier table [`Anthropic -> claude-haiku-4-5`, `OpenAi/ChatGpt -> gpt-4.1-mini` / `gpt-5-mini`, `Gemini -> gemini-2.5-flash`, Ollama parameter tag comparison] cross-checked with optional Catalog slugs).
   - **Never silently fall back to `primary_model`:** explicit misconfigured role yields `ResolvedRole::Unavailable`; unassigned role with no smaller model yields `ResolvedRole::Disabled` unless `allow_frontier_fallback == true` (`ExplicitFrontierOptIn`).
   - Provide `resolve_provider_scoped_api_key(...)` implementing `GENERIC_KEY_ENV_VARS` (`RE_SEARCH_API_KEY`) only when `RE_SEARCH_PROVIDER` matches the target `provider`, preventing cross-provider key contamination (`F-9` in `reports/audit-orchestration-routing.md`).
2. Create `crates/agent/src/usage.rs` (+ `usage_tests.rs`):
   - `TokenUsage { pub input: Option<u64>, pub output: Option<u64> }` (`extract_usage(&Value)` checking `Value::as_u64` per field across `input_tokens`/`prompt_tokens`/`promptTokenCount` and `output_tokens`/`completion_tokens`/`candidatesTokenCount`, handling `Value::Null` safely and returning `None` — **never false `0`**!).
   - `RoleUsageRollup` aggregating `{ calls, input_tokens, output_tokens, unknown_usage_calls }` per `(role, provider, used_model)` and folding session ledger events into JSON where purely unknown usage serializes tokens as `null` (`"unknown"` on screen).
3. Put pure helper `inject_openai_stream_usage(raw_body: Vec<u8>, provider: ProviderKind, stream: bool) -> Vec<u8>` in `crates/agent/src/usage.rs` (injecting `"stream_options": {"include_usage": true}` **only** when `stream && provider == ProviderKind::OpenAi`) and document the 1-line call in `crates/agent/src/openai.rs` under Merge notes — do **NOT** edit `openai.rs` or `stream_http.rs` (`f14c-vision-transcribe` owns `openai.rs` in parallel!). Expose `pub mod role_routing;` and `pub mod usage;` in `crates/agent/src/lib.rs` (`99`L).
4. Add offline acceptance tests in `crates/agent/tests/f9b_dual_scripted.rs` (self-contained inside `crates/agent/tests/` without editing `eval_support/`): two distinct loopback/scripted transports (`frontier` vs `worker`), barrier-synchronized parallel worker calls under `RunBudgetPool`, `null`/missing usage reporting, failed worker leaving partial result safely, and unavailable-role fail-closed test.
5. Run mandatory checks and push Checkpoint 2 to `gemini/modes/f9b-routing-core`.

### Checkpoint 3 — Lazy Worker Activity Routes & Inspector UI (`crates/desktop-server`)
1. Create `crates/desktop-server/src/routes_worker_activity.rs` (+ `routes_worker_activity_tests.rs`):
   - Pure ledger projector folding `delegate.*`, `explore.*`, `idea.exchange`, `verifier.*`, `context.supplied`, `run.control`, and session blobs with zero model calls and zero secret leakage:
     - `GET /api/sessions/{id}/workers` -> list of worker/exchange/verifier jobs with `{ job_id, run_id, stage_or_task, role, provider, requested_model, used_model, routing_reason, calls_used, usage, status, disposition, disposition_reason, artifact_refs }` + usage rollup by `(role, provider, used_model)` + resolved role availability/routing display.
     - `GET /api/sessions/{id}/workers/{job_id}` -> lazy detail loading request/response blobs, input `context.supplied` ranges, tool receipts, returned item IDs (`ev-*`, `xe-*`, `name@v`), explicit `"artifact_missing": true` when a blob/artifact is missing, and `(worker model summary — unverified)` label distinct from evidence receipts.
     - `POST /api/sessions/{id}/run-control` -> appends validated `run.control` v1 host events (`focus_note`, `pin_worker_item`, `dismiss_worker_item`, `pause_after_stage`, `next_run_roles`, `budget_delta`).
   - Register in a separate commit: `pub mod routes_worker_activity;` in `desktop-server/src/lib.rs` (`234`L) and route arms in `api_dispatch.rs` (`424`L) behind `AuthGuard::authorize_api`.
2. Create `crates/desktop-server/assets/worker_activity.js` (+ `crates/desktop-server/tests/worker_activity.test.mjs`) + `/assets/worker_activity.js` arm in `assets.rs` (`530`L):
   - Renders the on-demand worker activity inspector & reader-control panel (zero `innerHTML`, XSS-escaped, distinguishes `"unknown"` tokens from `0`, renders missing artifacts as missing, allows pin/dismiss/focus/pause controls, and exposes clean mount hook for `run_bar.js` / `mode_stages.js` documented in Merge notes).
3. Run full mandatory checks (`check_file_length.py`, `node --check`, `node --test`, non-root Linux workspace Clippy & `cargo test --workspace --locked`), write `docs/design/modes-2026-11/reports/f9b-routing-core.md` (with exact Merge notes for mode wiring), and push to `gemini/modes/f9b-routing-core`.

---

## 3. Do-Not-Edit List & Verification (`RULES.md`)

- **Do NOT edit:** `crates/agent/src/{runner,verifiers,autonomous,paper_read,delegate,explore,stream_http,resume,resume_profile}.rs`, `crates/desktop-server/src/{turn_runner,cli_turn,agent_models,paper_reader,state,solve,idea_view,investigation*,routes_idea,routes_session,http_srv,session_view,routers}.rs`, `crates/desktop-server/assets/{app,run_bar,solve_setup,canvas_dag,index.html}.*`, `crates/agent/tests/{eval_harness,eval_support/*}`, `Cargo.lock`, `tool_defs.txt`, `tool_profiles.txt`.
- Run on Linux **as non-root (`id -u != 0`) with `RE_SEARCH_PYTHON` unset**:
  ```bash
  python3 scripts/check_file_length.py --base origin/gemini/work/fixed
  for f in crates/desktop-server/assets/*.js; do node --check "$f"; done
  node --test crates/desktop-server/tests/*.test.mjs
  unset RE_SEARCH_PYTHON
  cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or
  cargo test --workspace --locked --no-fail-fast
  ```
- Push **only** via `git push origin HEAD:refs/heads/gemini/modes/f9b-routing-core`. Never squash, never open/merge PRs, and end with `PUSHED`.
