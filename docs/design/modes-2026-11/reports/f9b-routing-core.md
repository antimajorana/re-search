# Report: `f9b-routing-core` (F9b Core — Parallel Budgets, Strict Role Routing, Honest Usage & Lazy Worker Activity)

- **Base commit:** `d1b865c39dc934a3432c366f5e7c0328fbee7531` (merge-base with `origin/gemini/work/fixed`; `Merge pull request #47 from antimajorana/coord/modes-inv-v16`)
- **Branch:** `gemini/modes/f9b-routing-core` (`origin/gemini/modes/f9b-routing-core`)
- **Toolchain on Linux VM (WSL2):** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `cargo 1.100.0 (3acc79c0c 2026-11-05)`, `Python 3.12.3`, `node v24.22.2`
- **Guard rails:** `0` Do-Not-Edit files modified; `Cargo.lock` untouched; `tool_defs.txt` and `tool_profiles.txt` untouched; `py -3 scripts/check_file_length.py --base origin/gemini/work/fixed` passes (`No touched source file is over 600 lines.`).

---

## 1. Delivered Modules, Commits & Line Counts

| Checkpoint Commit | File | Lines | Scope / Contents |
|---|---|---:|---|
| `bfe956b` (CP1) | `crates/research-tools/src/budget_pool.rs` | 490 | `RunBudgetPool`, `BudgetLease` RAII slot (`mark_sent` vs drop refund), `CommittedCall`, `BudgetExhausted` (boxed `Err`), `PoolCaller::{Frontier, Worker, Verifier}`, `SubCap`, `ExhaustedScope`, `DEFAULT_FRONTIER_RESERVE`, `EXPLORE_FRONTIER_RESERVE`, `SOLVE_VERIFIER_RESERVE` |
| `bfe956b` (CP1) | `crates/research-tools/src/budget_pool_tests.rs` | 320 | 6 unit + multi-thread barrier concurrency tests (`init_from_events`, drop-refund, Explore `sub_caps`, Solve `verifier_reserve` + `frontier_reserve`, 15-thread barrier with held `in_flight` lease, 6-paper Explore cap) |
| `bfe956b` (CP1) | `crates/research-tools/src/lib.rs` | 343 | 1-line `pub mod budget_pool;` export |
| `bfe271d` (CP2) | `crates/agent/src/role_routing.rs` | 559 | `RoleRoutingPolicy`, `ResolvedRole::{Pinned, Disabled, Unavailable}`, `RoutingReason`, `PinnedRoleRecord` + `to_resolved()`, `StoreRolesMode`, `resolve_role`, `resolve_run_roles`, `to_pinned_records`, `build_role_transport`, 17 canonical role constants + aliases |
| `bfe271d` (CP2) | `crates/agent/src/role_routing_tiers.rs` | 290 | `resolve_provider_scoped_api_key` (`F-9` isolation for `GENERIC_KEY_ENV_VARS`), `parse_ollama_param_millions`, `pick_smaller_tier_model` (static smaller-tier table + optional Catalog filter + Ollama size comparison) |
| `bfe271d` (CP2) | `crates/agent/src/role_routing_tests.rs` | 249 | 5 unit tests: F-9 scoped env key isolation, precedence + alias resolution + `base_url`, misconfigured `Unavailable`, offline static tier + `Disabled` vs opt-in, Ollama tags & `PinnedRoleRecord` roundtrip |
| `bfe271d` (CP2) | `crates/agent/src/usage.rs` | 486 | `TokenUsage` (`None` vs genuine `Some(0)`), per-field `extract_usage` across `input_tokens`/`prompt_tokens`/`promptTokenCount` & `output_tokens`/`completion_tokens`/`candidatesTokenCount`, `inject_openai_stream_usage`, `strip_openai_stream_options`, `RoleUsageRollup` + `from_events()` |
| `bfe271d` (CP2) | `crates/agent/src/usage_tests.rs` | 137 | 3 unit tests: `Value::Null` per-field recovery + genuine `0` preservation, OpenAI SSE `stream_options.include_usage` scope, multi-family `RoleUsageRollup` serializing unknown tokens as `null` |
| `bfe271d` (CP2) | `crates/agent/tests/f9b_dual_scripted.rs` | 567 | 3 offline loopback dual-transport tests: distinct Anthropic frontier vs OpenAI worker servers, 3-way barrier parallel budget enforcement, provider-scoped keys, partial worker 500 failure + surviving evidence, `Unavailable`/`Disabled` zero-frontier-call fail-closed, OpenAI SSE usage opt-in |
| `bfe271d` (CP2) | `crates/agent/src/lib.rs` | 102 | 2-line `pub mod role_routing;` and `pub mod usage;` exports |
| `93f6c23` (CP3a) | `crates/desktop-server/src/routes_worker_activity.rs` | 477 | `handle_list_workers`, `handle_get_worker_detail`, `handle_post_run_control`, strict validation of all 6 `run.control` v1 actions + `base_url` safety + secret collection |
| `93f6c23` (CP3a) | `crates/desktop-server/src/worker_activity_fold.rs` | 559 | Pure ledger projector: `fold_controls`, `fold_worker_activity`, `RunControlsView`, `WorkerJobSummary`, `WorkerJobDetail`, `ArtifactRef` (`missing` blob flag), status/disposition mapping |
| `93f6c23` (CP3a) | `crates/desktop-server/src/worker_activity_detail.rs` | 233 | Lazy detail builder (`build_job_detail`): reads request/response/tool blobs, attaches `context.supplied` ranges, tool receipts, returned item IDs (`ev-*`, `xe-*`, `xa-*`, `name@v`), secret scrubbing, `(worker model summary — unverified)` label |
| `93f6c23` (CP3a) | `crates/desktop-server/src/routes_worker_activity_tests.rs` | 464 | 2 unit tests: `delegate`/`explore`/`idea.exchange`/`verifier` folding, `null` vs `0` tokens, missing blob flag, `[REDACTED]` secret scrubbing, all 6 `run.control` SSE emissions & 400 validation checks |
| `93f6c23` (CP3a) | `crates/desktop-server/assets/worker_activity.js` | 452 | Lazy UI drawer (`mountWorkerActivityButton`, `createWorkerActivityInspector`), `formatTokenCount` / `formatUsageSummary` (`unknown` vs `0`), missing-artifact badges, pin/dismiss/focus/pause/budget controls, zero `innerHTML` writes |
| `93f6c23` (CP3a) | `crates/desktop-server/tests/worker_activity.test.mjs` | 221 | 2 Node ESM tests: `unknown` vs `0` formatting, lazy drawer (0 fetches before open), detail fetch on expand, missing artifact badge, pin/focus/pause/budget controls, zero `innerHTML` / XSS neutralization |
| `9306f63` (CP3b) | `crates/desktop-server/tests/worker_activity_routes.rs` | 125 | Live HTTP loopback + `AuthGuard::authorize_api` test verifying 401 without token, 403 foreign `Origin`, 200 JSON responses for `/workers`, `/workers/{job_id}`, `/run-control`, and `/assets/worker_activity.js` |
| `9306f63` (CP3b) | `crates/desktop-server/src/lib.rs` | 245 | 1-line `pub mod routes_worker_activity;` |
| `9306f63` (CP3b) | `crates/desktop-server/src/api_dispatch.rs` | 451 | 3 `dispatch_api` route arms behind `AuthGuard::authorize_api` |
| `9306f63` (CP3b) | `crates/desktop-server/src/assets.rs` | 558 | Registered `/assets/worker_activity.js` |

---

## 2. Architecture & Design Decisions

1. **Pre-send atomic `RunBudgetPool` (`research_tools::budget_pool`):**
   - Mutex-guarded state checks `committed + in_flight + reserved_for(&caller) < total_cap` before any HTTP request leaves the host (`try_acquire` returns `BudgetLease`, or `Box<BudgetExhausted>` keeping `Result` small for `clippy::result_large_err`).
   - `PoolCaller::Worker` respects both `frontier_reserve` and `unspent_verifier_reserve = verifier_reserve.saturating_sub(verifier_committed + verifier_in_flight)`; `PoolCaller::Verifier` respects `frontier_reserve` while drawing down `unspent_verifier_reserve`; `PoolCaller::Frontier` is exempt from `frontier_reserve` while respecting `unspent_verifier_reserve`.
   - `BudgetLease::mark_sent()` permanently converts the in-flight slot to committed (so HTTP 500 / timeout / cancel still charges once); dropping `BudgetLease` without calling `mark_sent()` (e.g. `prepare_request` failure) refunds the slot automatically.

2. **`StoreRolesMode::Auto` & zero silent fallback (`agent::role_routing`):**
   - `ProviderKeyStore::default().roles` populates Anthropic Haiku defaults whenever `providers.json` omits `"roles"`. Under `StoreRolesMode::Auto` (default), `lookup_store_role` honors a `store.roles` slot only when it differs from `RoleAssignment::default()`, preventing OpenAI-only / Gemini-only sessions from misrouting unassigned workers onto unconfigured Anthropic defaults.
   - `resolve_provider_scoped_api_key` resolves `GENERIC_KEY_ENV_VARS` (`RE_SEARCH_API_KEY`, `RESEARCH_CORE_API_KEY`) only when `RE_SEARCH_PROVIDER` / `RESEARCH_CORE_PROVIDER` explicitly selects that provider (`F-9`).
   - Misconfigured explicit roles resolve to `ResolvedRole::Unavailable`; unassigned roles with no smaller tier resolve to `ResolvedRole::Disabled { reason: "no_smaller_model_configured" }` unless `allow_frontier_fallback == true` (`RoutingReason::ExplicitFrontierOptIn`).

3. **Honest usage (`agent::usage`):**
   - `extract_usage` evaluates `Value::as_u64` per candidate field (`input_tokens`/`prompt_tokens`/`promptTokenCount` and `output_tokens`/`completion_tokens`/`candidatesTokenCount`) so `{ "input_tokens": null, "prompt_tokens": 42 }` recovers `Some(42)` instead of short-circuiting on `Value::Null`. Missing/null counts remain `None` (JSON `null`, UI `"unknown"`), distinct from genuine `Some(0)`.
   - `inject_openai_stream_usage` adds `"stream_options": { "include_usage": true }` strictly when `stream && provider == ProviderKind::OpenAi`.

---

## 3. Merge Notes (Exact Integration Hooks for Wiring Lanes)

Every `Do-Not-Edit` file was left untouched (`0` diff). Wiring packages should apply the following minimal hooks:

1. **OpenAI streaming usage opt-in (`crates/agent/src/openai.rs:130-131`):**
   ```rust
   let raw = serialize_openai_request(self.provider, req)?;
   let body = crate::usage::inject_openai_stream_usage(
       if self.stream { inject_stream_flag(raw) } else { raw },
       self.provider,
       self.stream,
   );
   ```
   On a 1-time HTTP 400 retry for strict OpenAI-compatible proxies, pass `crate::usage::strip_openai_stream_options`.
2. **Per-field `usage` extraction (`crates/agent/src/responses.rs:471`, `crates/agent/src/paper_read.rs:186`, `crates/agent/src/delegate.rs`, `crates/agent/src/runner.rs:347`):**
   Use `agent::usage::extract_usage(&resp.usage)` when projecting/accumulating input & output counts so `Value::Null` never hides sibling provider keys or reports false `0`.
3. **Budget pool & pinned role plumbing (`crates/agent/src/runner.rs:230-347`, `crates/desktop-server/src/turn_runner.rs:248-340`, `crates/desktop-server/src/agent_models.rs:312`, `crates/desktop-server/src/solve.rs:110-147`):**
   - Replace `resolve_worker_transport` / role fallback chains in `agent_models.rs` and `solve.rs` with `agent::role_routing::{resolve_run_roles, build_role_transport, to_pinned_records}`, seeding `run_overrides` / `budget_delta` from `worker_activity_fold::fold_controls(&events)`.
   - Pass `Arc<RunBudgetPool>` into `run_turn_loop`, `delegate`, `paper_read`, `explore`, and `verifiers`; call `pool.try_acquire(caller)` *after* `transport.prepare_request(&req)` and `.mark_sent()` right before `send_prepared_streaming`.
4. **UI drawer mount (`crates/desktop-server/assets/run_bar.js:171-257`, `crates/desktop-server/assets/mode_stages.js:140-189`):**
   - Import `{ mountWorkerActivityButton } from './worker_activity.js';` and pass a control-row container node to `mountWorkerActivityButton(host, { sessionId, csrfToken, fetchJson })`. The button starts closed with `0` eager `/workers` fetches on page load.

---

## 4. Checkpoint Verification (Observed Prior to Final Report Commit)

| Scope | Command | Exact Result |
|---|---|---|
| Length guard | `py -3 scripts/check_file_length.py --base origin/gemini/work/fixed` | Exit `0` (`No touched source file is over 600 lines.`) |
| JS syntax | `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` | Exit `0` (`node --check ok`) |
| JS unit tests | `node --test crates/desktop-server/tests/*.test.mjs` | Exit `0` (`216 passed, 0 failed, 0 skipped`, including `2/2` in `worker_activity.test.mjs`) |
| Python unit tests | `PYTHONPATH=python python3 -m unittest discover -s python` | Exit `0` (`Ran 33 tests in 15.891s — OK`) |
| CP1 (`research-tools`) | `cargo clippy --locked -p research-tools --all-targets -- -D warnings -A clippy::manual_unwrap_or`, `--lib budget_pool`, `--test tool_def_golden` | Exit `0` (`0` warnings; `6 passed; 0 failed` + `3 passed; 0 failed`) |
| CP2 (`agent`) | `cargo clippy --locked -p agent --all-targets -- -D warnings -A clippy::manual_unwrap_or`, `--lib role_routing`, `--lib usage`, `--test f9b_dual_scripted` | Exit `0` (`0` warnings; `5/5`, `3/3`, `3/3` passed) |
| CP3 (`desktop-server`) | `cargo clippy --locked -p desktop-server --all-targets -- -D warnings -A clippy::manual_unwrap_or`, `--lib routes_worker_activity`, `--test worker_activity_routes` | Exit `0` (`0` warnings; `2/2`, `1/1` passed) |

*(Final `--workspace` clippy + test verification runs against the final report commit and is recorded in the completion report.)*

## 5. Linux VM Scope Limits

- Windows-only `#[cfg(windows)]` paths (`crates/desktop-app`, AppContainer sandbox tests, Windows DPAPI vault tests, Windows-specific `fs_read` junction tests) were not compiled or executed in WSL2 Ubuntu Linux.
