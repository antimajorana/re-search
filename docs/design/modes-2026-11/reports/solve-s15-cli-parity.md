# Report: `solve-s15-cli-parity` — Solve CLI & MCP Parity (S15)

- **Brief:** `docs/design/modes-2026-11/agents/solve-s15-cli-parity.md`
- **Base branch / commit:** `origin/gemini/work/fixed` (`24be58e6797679cfa423dcc16ea2073b54288714`)
- **Push branch:** `gemini/modes/solve-s15-cli-parity`
- **Governing design:** `00-overview.md` (§2.2, §3.1 row 12, §8.2 D1); `solve.md` (§0.1, §0.3, §5.1, §5.4, §6 R1/R11/R12/R15/R18/R20/R22/R24, §7.2, §9 S15, §12 risks 13 & 14, D16); `agents/RULES.md`.
- **Predecessor state on base commit:** S8, S9, and S11 (`research_tools::solve_gates::SolveGate`, `AutoCheckGate`, `solve_check_host_v1`, `solve_done`, `solve_checks`, `solve/1`, `solve_policy`) are merged on `24be58e6`. **S12** (`agent::solve_loop`, `agent::resume_brief`, `DESKTOP_MODE_SOLVE_ENABLED`), **S14** (`agent::verifier_loop`), **S13** (Solve workspace UI rendering of `result_sheet` / `cli_review.notice`), and **X18** (`cli_turn.rs` split) are **not** yet merged on base. Per `RULES.md`, none of S12/S14/S13/X18 were implemented here; clean seams and exact merge notes are documented below.

---

## 1. Commits (Separate Per Checkpoint & Audit Fix — Never Squashed)

1. `b8aa212` — **Checkpoint S15a:** Zero-behaviour-change split of `crates/desktop-server/src/cli_turn.rs` (`626`L -> `272`L) into `cli_turn_setup.rs` and `cli_turn_tests.rs` (shared seam with Explore X18).
2. `b40740d` — **Checkpoint S15b:** Profile-aware `cli-route/src/core_tools.rs` (`CORE_TOOL_PROFILE` first, `NoWorker` tolerance for Antigravity `enabledTools` = 17 `solve/1.mcp_defs()`), multi-caller `cli-route/src/relay.rs` (`start_for_callers` allowing `host:check` alongside vendor CLI caller), and mode-aware `desktop-server/src/cli_turn{,_setup,_tests}.rs` (strict `Mode::Research` prompt gating, deterministic Solve CLI prompt from `SolveState` + frozen `lessons.supplied`, check-worker env propagation, `"tool_profile"` recorded on `session.start`/`session.resume`).
3. `20ec124` — **Checkpoint S15c:** `mcp-server` (`core.exe`) Solve wiring (`crates/mcp-server/src/{lib,main,solve_wire}.rs`) and integration tests (`crates/mcp-server/tests/solve_mcp_gates.rs`): `CORE_TOOL_PROFILE` activation without `CORE_ENABLE_RESEARCH_TOOLS` or workspace, `SolveGate` (G1/G4/G5/G6/G7), `AutoCheckGate` (`host:check` via `TcpApprover`, zero auto-approve), `SympyHarvestGate` + protocol-2 `exprs_in` handle staging, and `McpCheckRunner`, with `mcp-server/Cargo.toml` and `Cargo.lock` completely untouched.
4. `66fa84b` — **Checkpoint S15d:** Ledger-driven `result_sheet` honesty (`crates/desktop-server/src/cli_turn_solve.rs`, `solve_view.rs`, `solve.rs`) and D16 CLI review status / CP2 API-verifier transport resolution seam (`resolve_api_verifier_transport_for_cli`), plus `crates/desktop-server/tests/solve_cli_parity.rs`.
5. `e8621ab` — **Independent Carbon Audit Fixes:** Resolved 1 CRITICAL, 4 HIGH, and 5 MEDIUM findings from an independent hard audit (`pro` subagent):
   - Isolated `WorkerRole::Check` AppContainer profile/SID/lock/dirs via `check_sandbox_id(session)` (`sessions/{check_sid}/{scratch,pycache}`) so the untrusted Solver AppContainer never shares a SID or writable `pycache` with Check (R6).
   - Fixed post-`solve.done` invalidation in `build_ledger_result_sheet` (superseded final entry, failed checks, or stale steps after a D15 setup revision revert `result_sheet` to `done: false` / `"Not done: ..."`; live `own_tier`/`effective_tier`/`badge_label` always read from `state.tier(...)`, never stale `done.effective_tier`).
   - Removed silent fallback to `resolve_in_core_role_model` so unrelated keys in `providers.json` never fabricate an unrequested verifier when Verifier is blank; prevented explicit API solver overrides from falling through to session/orchestrator CLI routes; aligned `solve.rs` prefill with `agent_models.rs` (`step_verifier` -> `full_verifier` -> `verifier`).
   - Triggered `SympyHarvestGate` on non-null malformed `exprs_out` (preserving `expr_warnings`) and surfaced `expr_harvest_error` on harvest failures; trimmed `CORE_TOOL_PROFILE` in `main.rs`; aligned `cli_turn_setup.rs` and `main.rs` on `active_profile_for_session_events`; recorded `brief_sha256` on Solve `session.resume`; made `solve_mcp_gates.rs` Windows-safe (targeted `env_remove`, `RE_SEARCH_PYTHON` lookup); added regression tests for G6 `verification_stuck`, malformed `exprs_out`, Antigravity 403 ack on Solve, and D15 staleness invalidation.
6. *(Final commit)* — This report (`docs/design/modes-2026-11/reports/solve-s15-cli-parity.md`).

---

## 2. Files Changed & Line Counts (`<= 600` Lines Every File)

Measured after `e8621ab` (`python3 scripts/check_file_length.py --base origin/gemini/work/fixed` passed):

| File | Status | Lines | Purpose |
| --- | --- | ---: | --- |
| `crates/cli-route/src/core_tools.rs` | Modified | 280 | `CORE_TOOL_PROFILE_ENV`, `register_core_tools_for_profile_with_solve`, profile-first `core_tool_ids_for` (17 `solve/1` MCP tool IDs with `NoWorker` tolerance, fail-closed fallback on invalid/unbuilt profiles). |
| `crates/cli-route/src/relay.rs` | Modified | 478 | `start_for_callers` / `handle_stream_checked_callers` allowing `host:check` alongside the session's vendor CLI caller on Solve turns while still rejecting spoofed callers. |
| `crates/desktop-server/src/cli_turn.rs` | Modified (split from 626L) | 308 | Slimmed subscription-CLI turn runner: resolves `(mode, profile)`, creates isolated `sessions/{check_sid}/{scratch,pycache}`, launches multi-caller Solve relay, passes `CORE_TOOL_PROFILE` + `CORE_CHECK_*`, records `"tool_profile"` and Solve `"resume_mode": "brief"` + `"brief_sha256"`. |
| `crates/desktop-server/src/cli_turn_setup.rs` | New | 577 | Extracted route/model/`core.exe` helpers, `resolve_cli_turn_profile` (`active_profile_for_session_events`), `check_sandbox_id`, `core_env_for_profile`, strict `Mode::Research`-only prompt gating, and deterministic `build_solve_cli_prompt` reading `SolveState` + frozen `lessons.supplied` (R24). |
| `crates/desktop-server/src/cli_turn_solve.rs` | New | 265 | Ledger-driven `build_ledger_result_sheet` (R15/R22, post-`solve.done` invalidation), D16 plain-notice constants, `resolve_api_verifier_transport_for_cli` (CP2 seam enforcing `agent_models.rs` helper-role guard), and `resolve_cli_verifier_status`. |
| `crates/desktop-server/src/cli_turn_tests.rs` | New | 342 | Unit tests for CLI route switching, billing/env-auth guard, strict `Mode::Research` prompt gating, Solve env & 17-tool ID parity, `investigation/1` -> `investigation/2` upgrade, Antigravity 403 ack on Solve, and `check_sandbox_id` bounds. |
| `crates/desktop-server/src/lib.rs` | Modified | 252 | Exposes `pub mod cli_turn_setup;` and `pub mod cli_turn_solve;`. |
| `crates/desktop-server/src/solve.rs` | Modified | 353 | Aligns verifier prefill (`step_verifier`/`full_verifier`/`verifier`) and returns `cli_review` on `POST /api/sessions/{id}/solve/prefill`. |
| `crates/desktop-server/src/solve_view.rs` | Modified | 580 | Attaches ledger-driven `result_sheet` and D16 `cli_review` to `GET /api/sessions/{id}/solve/view`. |
| `crates/desktop-server/tests/solve_cli_parity.rs` | New | 436 | Integration tests for ledger-driven `result_sheet` honesty despite false CLI prose, D16 notices & CLI helper rejection, CP2 API verifier resolution, blank verifier non-fabrication, API solver override, Antigravity ack, and D15 post-`solve.done` staleness invalidation. |
| `crates/mcp-server/src/lib.rs` | New | 5 | Exposes `pub mod solve_wire;` so `tests/solve_mcp_gates.rs` exercises the exact `build_solve_router_with_bindings` production wiring. |
| `crates/mcp-server/src/main.rs` | Modified | 408 | Trims `CORE_TOOL_PROFILE`, activates profile branch on `CORE_TOOL_PROFILE` without requiring `CORE_ENABLE_RESEARCH_TOOLS` or a workspace, dispatches `PROFILE_SOLVE_V1` via `solve_wire`, attaches `HarvestPreviews` to `sympy_eval` MCP responses, uses `active_profile_for_session_events` fallback. |
| `crates/mcp-server/src/solve_wire.rs` | New | 480 | `WorkerEpochAdapter`, `McpCheckRunner` (`CheckProcessRunner` + `CheckCanonicalizer`), `HarvestPreviews`, `SympyHarvestGate`, `check_sandbox_id`, `build_check_worker` (`#[cfg(windows)]` AppContainer with distinct check SID), `solve_sympy_handler` (protocol-2 `exprs_in`), `solve_gate_anchors`, `build_solve_router_with_bindings`, `parse_check_timeout`. |
| `crates/mcp-server/tests/solve_mcp_gates.rs` | New | 526 | Real `core` binary stdio MCP integration test (`solve/1` activation without `CORE_ENABLE_RESEARCH_TOOLS`, 17-tool `tools/list`, G1/G5/G6/G7 refusals) + real two-process Python (`--role solver` & `--role check`) `TcpApprover` + `cli_route::relay::start_for_callers` test proving `host:check` asks human (zero auto-approve), harvests handles, verifies claims, and runs `solve_finish`. |

**Frozen / forbidden files verified untouched (`git diff origin/gemini/work/fixed..HEAD` empty):**
`Cargo.lock`, `crates/mcp-server/Cargo.toml`, `crates/research-tools/src/tool_profile.rs`, `crates/research-tools/tests/golden/tool_defs.txt`, `crates/research-tools/tests/golden/tool_profiles.txt`, `crates/desktop-server/src/http_srv.rs`, `crates/agent/src/autonomous.rs`, `.github/`.

---

## 3. Solo Decisions, Deviations & Assumptions

1. **Multi-caller approval relay (`crates/cli-route/src/relay.rs`):**
   - *Reason:* `cli-route/src/relay.rs::start_for_caller` previously pinned a single expected caller string (the vendor CLI caller). Under `solve/1`, S9 `AutoCheckGate` invokes `check_run` as `host:check` (`SOLVE_CHECK_CALLER`) through `TcpApprover` so the human explicitly approves host checks (zero auto-approve). Without permitting `host:check`, every auto-check approval on a Solve CLI session would be rejected by the relay as a spoofed caller.
   - *Decision:* Added `start_for_callers` / `handle_stream_checked_callers` while keeping `start_for_caller` untouched for Research/Investigate sessions. Solve CLI turns permit exactly `[vendor.caller(session_id), "host:check"]`; all other callers are still rejected with a forged-caller error.
2. **Library target for `mcp-server` (`crates/mcp-server/src/lib.rs`, 5 lines):**
   - *Reason:* `crates/mcp-server/Cargo.toml` is frozen (no `Cargo.toml`/`Cargo.lock` edits) and Cargo automatically recognizes `src/lib.rs` alongside `[[bin]] name = "core"`. Adding `src/lib.rs` containing only `pub mod solve_wire;` lets `tests/solve_mcp_gates.rs` directly test `build_solve_router_with_bindings` with real Linux stdio Python workers without duplicating production wiring or touching `Cargo.toml`.
3. **Distinct AppContainer SID for `WorkerRole::Check` (`check_sandbox_id`):**
   - *Reason:* `AppContainerSandbox::new` derives profile `re-search.worker.{session}`, enforces `scratch`/`pycache` inside `data_dir/sessions/{session}`, locks `.worker.lock`, and deletes the profile on `Drop`. Passing the same session ID to both Solver and Check would give the untrusted Solver worker RW access to Check's `PYTHONPYCACHEPREFIX` (violating R6) and cause profile drop races.
   - *Decision:* Derived `check_sandbox_id(session)` (`"{lower}-chk"` when valid and `<= 43` chars, else `"chk-{sha256[..16]}"`) identically in `mcp-server/src/solve_wire.rs` and `desktop-server/src/cli_turn_setup.rs`, placing check dirs at `data_dir/sessions/{check_sid}/{scratch,pycache}`.
4. **CP2 (S14 `verifier_loop`) seam status — plainly stated:**
   - S14 (`agent::verifier_loop`) is **not** merged on `origin/gemini/work/fixed`. Accordingly, `resolve_api_verifier_transport_for_cli` resolves and validates an in-core API `Transport` (rejecting CLI helper roles and never fabricating a verifier when Verifier is blank), and `cli_review` honestly reports `"automatic_review_available": false`, `"s14_verifier_loop_ready": false`, and `D16_CP2_PENDING_S14_NOTICE` when an API verifier is configured (or `D16_CLI_REVIEW_UNAVAILABLE_NOTICE` when none is configured). Automatic post-turn verifier-loop execution does **not** run yet until S14 merges and wires into `resolve_api_verifier_transport_for_cli`.
5. **Deterministic Solve CLI prompt without S12 `resume_brief`:**
   - Because S12 (`agent::resume_brief`) is not merged yet, `cli_turn_setup.rs::build_solve_cli_prompt` synthesizes a byte-deterministic Solve brief directly from `SolveState::from_events` + frozen `lessons.supplied` ledger blobs (R24, never reading live `lessons.db`), including `SOLVE_HANDLE_SURVIVAL_LINE`. When S12 merges, its state summary section can delegate directly to `agent::resume_brief::build_resume_brief`.
6. **UI assets (`solve_setup.js` / `solve_state.js`) left untouched:**
   - `solve_setup.js` is 597 lines and owned by S5/S13, not S15. The backend now serves `result_sheet` on `/solve/view` and `cli_review` (including `notice`) on both `/solve/prefill` and `/solve/view`, ready for S13 UI rendering.
7. **Named decision constants:**
   - `D16_CLI_REVIEW_UNAVAILABLE_NOTICE`, `D16_CLI_VERIFIER_REJECTED_NOTICE`, `D16_CP2_PENDING_S14_NOTICE` (`crates/desktop-server/src/cli_turn_solve.rs`), `SOLVE_HANDLE_SURVIVAL_LINE` (`crates/desktop-server/src/cli_turn_setup.rs`), `CORE_TOOL_PROFILE_ENV` (`crates/cli-route/src/core_tools.rs`).

---

## 4. Merge Notes

Exact seams for sibling briefs merging alongside S15:

- **Explore X18 (`cli_turn.rs`, `cli_turn_setup.rs`, `cli_turn_tests.rs`, `core_tools.rs`, `mcp-server/src/main.rs`):**
  - `cli_turn.rs` has already been split in standalone commit `b8aa212` (`cli_turn.rs` 308L, `cli_turn_setup.rs` 577L, `cli_turn_tests.rs` 342L).
  - X18 can plug `Mode::Explore` / `explore/1` directly into `cli_turn_setup.rs::build_cli_prompt` (Research instructions are already strictly gated to `Mode::Research`), `core_env_for_profile` (already propagates `CORE_TOOL_PROFILE`), `core_tools.rs::core_tool_ids_for`, and `mcp-server/src/main.rs::build_router`.
  - Note: `cli_turn_setup.rs` is currently 577 lines; if X18 adds substantial Explore prompt code, place Explore prompt synthesis in a sibling `cli_turn_explore.rs` (mirroring `cli_turn_solve.rs`).
- **Solve S12 (`agent::resume_brief`, `DESKTOP_MODE_SOLVE_ENABLED`):**
  - `crates/desktop-server/src/cli_turn_setup.rs::build_solve_cli_prompt`: once S12 lands `agent::resume_brief::build_resume_brief`, delegate the setup/steps/open-items/handles/lessons brief body to it so CLI and in-core Solve share one brief formatter and identical `brief_sha256` on `session.resume`.
  - `DESKTOP_MODE_SOLVE_ENABLED` stays `false` in `session_mode.rs` (owned by S12); Solve CLI routes become user-reachable in the desktop UI as soon as S12 flips it.
- **Solve S14 (`agent::verifier_loop`):**
  - `crates/desktop-server/src/cli_turn_solve.rs::resolve_api_verifier_transport_for_cli` returns `Result<Option<ResolvedCliVerifier>, String>` (`(Box<dyn Transport>, provider, model)`) with `agent_models.rs` CLI-helper rejection already enforced.
  - Once S14 merges, `crates/desktop-server/src/cli_turn.rs::run_cli_turn` (after a `Mode::Solve` turn completes) should call `resolve_api_verifier_transport_for_cli`, run S14's `verifier_loop` against unreviewed active steps when `Ok(Some(...))`, and flip `automatic_review_available` / `s14_verifier_loop_ready` to `true` in `resolve_cli_verifier_status`.
- **Solve S13 (Solve Workspace UI):**
  - `GET /api/sessions/{id}/solve/view` now returns `result_sheet` (`done`, `status`, `status_line`, `unchecked_steps`, `failed_steps`, `stale_steps`, `final_entry_id`, `own_tier`, `effective_tier`, `badge_label`) and `cli_review`.
  - `POST /api/sessions/{id}/solve/prefill` returns `cli_review` (`on_cli_route`, `cli_provider`, `api_verifier_configured`, `automatic_review_available`, `cli_helper_rejected`, `verifier`, `notice`). S13 should render `cli_review.notice` in the setup card and `result_sheet.status_line` in the result sheet.
- **Localized edits to existing files (for conflict awareness):**
  - `crates/cli-route/src/relay.rs` (`start_for_callers`, `handle_stream_checked_callers`)
  - `crates/desktop-server/src/lib.rs` (2 `pub mod` lines)
  - `crates/desktop-server/src/solve.rs` (`verifier` prefill lookup + `cli_review` field on prefill response)
  - `crates/desktop-server/src/solve_view.rs` (`result_sheet` + `cli_review` fields on view response; currently 580L — do not grow past 600L without splitting)

---

## 5. Verification Commands & Exact Measured Results

All verification ran on Linux as non-root (`uid=1001(antigravity)`, `gid=1001(antigravity)`), Python 3 with `sympy 1.14.0` + `mpmath 1.3.0`, Node `v22.23.5`:

1. **File-length & frozen-file guards:**
   - `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` -> **PASS** (`No touched source file is over 600 lines.`)
   - `git diff --name-only origin/gemini/work/fixed -- Cargo.lock crates/mcp-server/Cargo.toml crates/research-tools/src/tool_profile.rs crates/research-tools/tests/golden/ crates/desktop-server/src/http_srv.rs crates/agent/src/autonomous.rs .github/` -> **PASS** (empty output)
2. **Workspace Clippy:**
   - `unset RE_SEARCH_PYTHON && cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` -> **PASS** (`0` warnings, `0` errors)
3. **Full workspace tests (`RE_SEARCH_PYTHON` unset):**
   - `unset RE_SEARCH_PYTHON && cargo test --workspace --locked --no-fail-fast` -> **PASS**: **`1212` passed, `0` failed, `2` ignored**
   - The 2 ignored tests are pre-existing live-tier eval tests on `origin/gemini/work/fixed` (`file_sheet_from_json ... ignored, live tier: needs EVAL_SHEET_JSON` and `live_manifests_validate ... ignored, live tier: needs EVAL_LIVE_MANIFEST_DIR`). Zero skipped or `#[ignore]` tests were added by S15.
4. **Targeted real-Python worker / research-tools / mcp-server tests:**
   - `RE_SEARCH_PYTHON="$(command -v python3)" cargo test --locked -p worker -p research-tools -p mcp-server --no-fail-fast` -> **PASS**: **`537` passed, `0` failed, `0` ignored** (including `tool_def_golden`, `tool_profile_golden`, `solve_s7_exprs`, `solve_s8_s9_s11`, `solve_set_q_m3`, and both `solve_mcp_gates` tests).
5. **Node asset syntax & UI unit tests:**
   - `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` -> **PASS** (all JS assets valid)
   - `node --test crates/desktop-server/tests/*.test.mjs` -> **PASS**: **`226` passed, `0` failed, `0` skipped**

---

## 6. What Could Not Be Compiled or Run on Linux

- `#[cfg(windows)]` code paths in `crates/mcp-server/src/solve_wire.rs::build_check_worker` and `crates/mcp-server/src/main.rs` (`AppContainerSandbox::new` / Windows SID ACLs / `.worker.lock` / `WorkerRole::Check` inside Windows AppContainer) are not compiled or executed on Linux. Their pure helper `check_sandbox_id` and non-Windows router/gate/worker/TCP-approver integration paths were compiled and verified end-to-end on Linux, and Windows CI must still compile-check and run the `#[cfg(windows)]` AppContainer tests.
- Windows-only fake-CLI integration test binaries (`crates/desktop-server/tests/claude_cli_route.rs`, `claude_cli_guards.rs`, `antigravity_route.rs`, `crates/worker/tests/appcontainer_sandbox.rs`, `crates/worker/tests/python_worker.rs`) compile to 0 tests on Linux due to `#![cfg(windows)]`.

---

## 7. Pre-Existing Bugs & Limitations Noticed (Not Fixed Outside S15 Scope)

1. **`crates/desktop-server/src/solve_check_routes.rs:92` (S9, pre-existing):** `AppContainerSandbox::new` for the desktop-server check worker passes the raw `session_id` rather than a distinct check sandbox ID (unlike `mcp-server/src/solve_wire.rs::check_sandbox_id`), risking AppContainer SID/lock collision with the solver worker on Windows. Left untouched as outside S15's owned files; recommend porting `check_sandbox_id` there during integration.
2. **`crates/research-tools/src/run_budget.rs:141-161` (F9, pre-existing) — G4 model-call counting on CLI routes:** `SolveGate` counts model calls via `run_budget::count_model_calls`, which counts in-core `model.request` events. External-loop CLI turns (`cli-route`) emit `cli.init`, `model.text`, `model.tool_use`, and `cli.result` instead of `model.request`. Consequently, on subscription-CLI Solve sessions, **G4 wall-clock timeout is enforced**, while **G4 `max_model_calls` and `max_model_calls_per_leg` currently read `0`** until `run_budget::count_model_calls` (or `cli-route/src/turn.rs`) accounts for CLI turns. Left untouched because `run_budget.rs` is outside S15's owned files.
3. **`crates/desktop-server/src/cli_turn_solve.rs` (CP2 / S14 dependency):** Automatic post-turn model verification on CLI routes does not execute yet because S14 (`agent::verifier_loop`) is not merged on base; `cli_review` honestly reports `automatic_review_available: false` and `D16_CP2_PENDING_S14_NOTICE`.
4. **`crates/desktop-server/assets/solve_setup.js` (S5/S13):** Currently 597 lines and owned by S5/S13; does not yet render `cli_review.notice` or `result_sheet.status_line` in the browser DOM until S13 wires those JSON fields.
5. **Pre-existing Clippy lint on Rust 1.97:** `crates/desktop-server/src/evidence.rs:744` (`clippy::manual_unwrap_or`, >600L, documented in `RULES.md`), untouched.

---

## 8. Integration Addendum — CP2 & `turn_runner.rs` CLI Dispatch (Rebased onto `78e453d`)

Integrated after PR #66 (`S14+S20`) and PR #67 (`S18b`) merged onto `origin/gemini/work/fixed` (`78e453d`):

- **`crates/desktop-server/src/turn_runner.rs` (570 lines):** Removed lines `61–70` (the temporary `"until S15"` 400 refusal on `claude-cli`/`antigravity`), while preserving the `solve.setup` guard (`"Solve session requires setup before running a turn"`) and `wants_cli_route` routing.
- **`crates/desktop-server/src/cli_turn_solve.rs` & `cli_turn.rs` (CP2 S14 `verifier_loop` wiring):** Flipped `automatic_review_available` and `s14_verifier_loop_ready` to `true` (`notice: null`) in `resolve_cli_verifier_status`, added `maybe_run_post_cli_solve_review` (invoking `run_finish_review_batches` / `run_solve_verifier_loop` and emitting `"solve_updated"` SSE), and called it from `cli_turn::run_cli_turn` upon `TurnEnd::Completed { exit_code: Some(0), init_verified: true }` in `Mode::Solve`.
- **`crates/mcp-server/src/solve_wire.rs:283`:** Preserved S18b `job_memory_limit_bytes: None` in the `#[cfg(windows)]` `WorkerRole::Check` `WorkerConfig`.
- **Tests (`solve_cli_parity.rs` + `solve_cli_parity_cp2.rs`):** Updated D16 configured-verifier assertions (`automatic_review_available == true`, `s14_verifier_loop_ready == true`, `notice == null`) and added cross-platform pre-setup 400 / post-setup 202 / CP2 `verifier_loop` tests plus a `#[cfg(windows)]` `FakeCli` E2E test.
- **Post-edit verification (`6b4d191`, rebased onto `78e453d`):**
  - `py -3 scripts/check_file_length.py --base origin/gemini/work/fixed`: passed (`No touched source file is over 600 lines.`)
  - Frozen files check: `0` frozen files modified
  - `node --check` + `node --test crates/desktop-server/tests/*.test.mjs`: `240 passed, 0 failed, 0 skipped`
  - Windows `cargo check --workspace --all-targets --locked` + `cargo test --locked -p desktop-server --test solve_cli_parity --test solve_cli_parity_cp2`: passed (`solve_cli_parity` `4/4`, `solve_cli_parity_cp2` `2/2` including Windows `FakeCli` E2E)
  - Non-root WSL2 Clippy (`-- -D warnings -A clippy::manual_unwrap_or`): `0` warnings/errors
  - Non-root WSL2 targeted `solve_cli_parity` (`4/4`) + `solve_cli_parity_cp2` (`1/1`)
  - Non-root WSL2 full workspace tests (`RE_SEARCH_PYTHON` unset): **`170` suites, `1,294 passed, 0 failed, 2 ignored`** (`0` new ignored)
  - Non-root WSL2 real-Python targeted tests (`-p worker -p research-tools -p mcp-server`, `sympy 1.14.0`, `mpmath 1.3.0`, `numpy 2.5.4`): **`52` suites, `560 passed, 0 failed, 0 ignored`**

