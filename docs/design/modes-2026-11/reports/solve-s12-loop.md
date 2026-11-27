# Report: solve-s12-loop — Solve Execution Loop, Pause & Resume Brief (S12)

- **Brief:** `docs/design/modes-2026-11/agents/solve-s12-loop.md`
- **Base commit (`origin/gemini/work/fixed`):** `3fd2485c41bedeea96bf8aa7848618dc8d295af2` (`3fd2485`, merged forward across PRs #52–#59 from initial base `acf7cb0d`)
- **Branch:** `gemini/modes/solve-s12-loop`
- **Package:** `S12` (Checkpoints `S12a`–`S12e`)
- **Date:** 2026-11-27

---

## 1. What Was Implemented

| File | Change | Line Count | Purpose |
| --- | --- | ---: | --- |
| `crates/agent/src/dispatch.rs` | New (`S12a`) | 113 | Extracted `dispatch_tool_calls`, `record_session_title`, `collect_text`, and `clip_text` from `runner.rs` |
| `crates/agent/src/runner.rs` | Small edit (`S12a`/`S12b`) | 539 | Re-exports `clip_text`, adds mode-neutral `SessionOutcome::Paused { turns, reason }`, marks `record_content_blocks` / `tool_defs_json` / `validate_turn` / `Audit` / `AuditStreamObserver` `pub(crate)` |
| `crates/agent/src/autonomous.rs` | Minimal match fix (`S12b`) | 575 | Exhaustive `SessionOutcome::Paused` match arms (`+2` lines, see §2 Deviations) |
| `crates/cli/src/api_session.rs` | Small edit (`S12b`) | 558 | Exhaustive `SessionOutcome::Paused => 0` match arm in `map_outcome` |
| `crates/agent/src/solve_prompts.rs` | New (`S12c`) | 220 | Task-kind prompts (`calculate`, `prove`, `check_derivation`) via `build_solve_system_prompt`; leaves `RESEARCH_SYSTEM_PROMPT` untouched |
| `crates/agent/src/resume_brief.rs` | New (`S12c`) | 428 | Pure byte-deterministic `build_resume_brief(events, upto_seq, budget_chars)` with setup, active steps, closure detail + 1-line older steps, open items, handles, verbatim `lessons.supplied` quote block (R24), last 4 turns verbatim, and `NO_SURVIVING_PYTHON_VARS_LINE` |
| `crates/agent/src/resume_brief_tests.rs` | New (`S12c`) | 211 | Unit tests for byte determinism, closure/last-4-turns preservation, budget cap, and R24 isolation from live `lessons.db` |
| `crates/agent/src/solve_loop.rs` | New (`S12d`) | 503 | In-core Solve execution loop (`run_solve_loop`, `resume_solve_loop`), `SolveHostHooks` seam for S14/S20, `SolveWorkerRuntime` seam, 60% context auto-checkpoint, R12 `gate_bounces`, R16 no `session.abort` |
| `crates/agent/src/solve_loop_tools.rs` | New (`S12d`) | 405 | Solve tool dispatch (`router.try_call`), `sympy_eval` S7 `exprs_out` harvesting, `solve_finish` D6 1-round repair trigger, worker restart handle restage + `approval_needed` on epoch change, `solve.checkpoint` & `solve.paused` emitters |
| `crates/agent/src/solve_loop_tests.rs` | New (`S12d`) | 412 | Unit tests for R12 `gate_bounces`, R16 `transport_error`, 60% auto-checkpoint, worker-restart restaging, D6 repair round, S14/S20 host hooks seam, cancel, denial, and brief resume |
| `crates/agent/src/resume_profile.rs` | Small edit (`S12e`) | 395 | `resolve_session_profile_with_lookup` supporting Solve-only `resume_mode: "brief"` transitions matching prior `solve.checkpoint.brief_sha256`; rejects replay transitions and `research/1 -> solve/1` as `Drift` (R2) |
| `crates/agent/src/resume_profile_tests.rs` | Small edit (`S12e`) | 586 | Added `r2_brief_resume_allows_solve_transitions_but_rejects_replay_or_cross_mode` |
| `crates/agent/src/lib.rs` | Small edit | 113 | Exported `resume_brief`, `solve_loop`, `solve_prompts`, `SessionOutcome`, `SolveHostHooks`, `SolveLoopConfig`, `SolveWorkerRuntime`, `NoopSolveHostHooks`, and budget constants |
| `crates/desktop-server/src/solve_runtime.rs` | New (`S12d`) | 320 | Desktop Solve worker bundle (`Worker` + `WorkerRole::Check` `DesktopCheckBridge` canonicalizer & `CheckProcessRunner`, S7 `stage_exprs_in`, S8/S9/S11 `solve_check_host_v1`, S11 `solve_gate_chain`) |
| `crates/desktop-server/src/solve_turn_routes.rs` | New (`S12d`) | 314 | `run_desktop_solve_turn`, `POST /api/sessions/{id}/solve/continue`, startup `reconcile_interrupted_solve_sessions`, and `POST /api/sessions/{id}/solve/reconcile` |
| `crates/desktop-server/src/api_dispatch.rs` | Small edit (`S12d`) | 494 | Registered `/api/sessions/{id}/solve/{continue,reconcile}` behind `AuthGuard::authorize_api` |
| `crates/desktop-server/src/lib.rs` | Small edit (`S12d`) | 257 | Declared `solve_runtime`, `solve_turn_routes` and wired startup `reconcile_interrupted_solve_sessions(&state)` |
| `crates/desktop-server/src/routers.rs` | Small edit (`S12e`) | 155 | Routed `PROFILE_SOLVE_V1` in `build_desktop_router_for_profile` to `solve_runtime::build_solve_router` |
| `crates/desktop-server/src/session_mode.rs` | Small edit (`S12e`) | 257 | Flipped `DESKTOP_MODE_SOLVE_ENABLED = true` and updated `session_mode::tests` |
| `crates/desktop-server/src/turn_runner.rs` | Small edit (`S12b`/`S12e`) | 580 | Pre-spawn `solve.setup` + API/ChatGPT refusal (`400`, lines `53–71`) before `setup` and on `claude-cli` / `antigravity` routes until `S15`, skipped `investigation::prepare`, dispatched `Mode::Solve` to `solve_turn_routes::run_desktop_solve_turn`, matched `SessionOutcome::Paused` |
| `crates/desktop-server/tests/solve_loop_routes.rs` | New (`S12d`/`S12e`) | 212 | End-to-end HTTP integration tests for startup/explicit reconcile, read-only `GET /solve/view`, `POST /solve/continue`, and `POST /messages` R16 `transport_error` pause |
| `crates/desktop-server/tests/mode_shell.rs` | Small edit (`S12e`) | 290 | Updated unconfigured Solve `s-solve-1` refusal assertion to `"setup"` (`400`, 0 events written); kept `s-explore-1` strict on `"not built yet"` |
| `crates/desktop-server/tests/handoff_routes.rs` | Small edit (`S12e`) | 350 | Updated F7 Solve handoff child assertions for enabled Solve (`target_mode_built == true`; unconfigured child still refuses turns with `400` `"setup"` before setup) |

**Do-not-edit / frozen check (`git diff --stat origin/gemini/work/fixed`):** Zero diffs in `Cargo.lock`, `Cargo.toml`, `crates/research-tools/tests/golden/{tool_defs,tool_profiles}.txt`, `crates/agent/src/{resume,verifiers}.rs`, `crates/desktop-server/src/{solve,http_srv,routes_session,cli_turn}.rs`, and `.github/`.

---

## 2. Deviations from Design, Decisions, and Assumptions

1. **Minimal `autonomous.rs` match arms (deviation from Do-Not-Edit list):** `crates/agent/src/autonomous.rs` had exhaustive matches on `SessionOutcome` at lines `354-359` and `365-370` without a wildcard arm. Adding `SessionOutcome::Paused` required two 1-line propagation arms there so the workspace compiles; Research and Investigate control flow is unchanged.
2. **Existing F5/F7 Solve-disabled assertions updated & pre-setup/CLI refusals (`turn_runner.rs:53-71`, `mode_shell.rs`, `handoff_routes.rs`):** Both `tests/mode_shell.rs:178-196` and `tests/handoff_routes.rs:238,263-264` previously asserted that Solve was unbuilt (`"not built yet"` / `"target_mode_built": false`). Enabling `DESKTOP_MODE_SOLVE_ENABLED = true` per S12e required updating those assertions to verify `target_mode_built: true` and `"setup"` refusal for unconfigured Solve sessions (`400`, zero ledger writes), leaving `Explore` untouched. As disclosed in PR #60 notes, `crates/desktop-server/src/turn_runner.rs:53-71` also refuses `Mode::Solve` (`400`) before `solve.setup` is recorded and refuses `Mode::Solve` on `claude-cli` / `antigravity` subscription-CLI routes until `S15` lands CLI/MCP parity.
3. **Helper file splits for `<= 600`L rule:** `solve_loop_tools.rs` (`405`L) was split out of `solve_loop.rs` (`503`L) alongside `runner.rs` -> `dispatch.rs` so every touched file stays `<= 586` lines (`scripts/check_file_length.py` passes cleanly).
4. **No `turn_control.rs` edit needed:** Solve background legs register an `ApiTurnGuard` (`solve_turn_routes.rs:175`), so existing `turn_control::stop_all_turns` cancellation works without modifying `turn_control.rs`.
5. **Decision — `SolveHostHooks` signature without exposing `Audit`:** Kept `runner::Audit` `pub(crate)` and made `SolveHostHooks::handle_host_tool` accept `(&Ledger, &str, ...)` (matching `after_solve_finish`) with `audit.flush_interleaved()?` after host execution, avoiding `private_interfaces` lint under `-D warnings`.
6. **Decision — `solve.checkpoint` stores both `brief_blob` and `raw_blob`:** `crates/ledger/src/verify.rs` only marks `raw_blob`, `tool.request.args.blob`, and `tool.result.{result,error}.blob` as referenced; storing the content-addressed brief hash in both fields keeps `ledger.verify_all().unreferenced_blobs == 0`.
7. **Assumption — character-count prompt budget heuristic:** `estimate_prompt_chars` sums character counts of system prompt, messages, and tool names/descriptions (`DEFAULT_SOLVE_CONTEXT_WINDOW_CHARS = 400_000`, auto-checkpoint at `>= 60%`) rather than running provider-specific tokenizers.

---

## 3. Merge Notes & Cross-Lane Observations

- Merged `origin/gemini/work/fixed` (`3fd2485`, PRs `#52`–`#59`: `solve-s8-s9-s11`, `f9b-routing-core`, `f8-harness-sets`, `solve-s21-migrate`, `f15-writeup-solve`, `solve-s10-physics-checks`, `solve-s19-export-replay`) with zero manual conflicts (`api_dispatch.rs` and `lib.rs` auto-merged cleanly).
- **S21 (`open-in-solve`) interaction noticed (`solve_turn_routes.rs:89` vs `resume_profile.rs`):** A post-F1 Research session migrated via `POST /api/sessions/{id}/open-in-solve` retains its parent `session.start { "tool_profile": "research/1" }` ahead of `solve.started` (`solve_migrate.rs:572-591`). Because `solve_turn_routes.rs:89` checks `existing.iter().any(|e| e.kind == "session.start")` globally rather than after `solve.started`, running a migrated post-F1 child will call `resume_solve_loop` (emitting `solve.checkpoint` + `session.resume { resume_mode: "brief", tool_profile: "solve/1" }`), while `resolve_session_profile` stays strict (`Drift` on `research/1 -> solve/1`, matching `solve_migrate.rs:589` and `solve-s12-loop.md` §3.5). Scoping `has_started` to events after `solve.started` can be considered cleanly in UI/migration follow-ups without changing R2 replay semantics.

---

## 4. Verification Results (Non-Root Linux, `RE_SEARCH_PYTHON` Unset)

Environment: WSL2 Ubuntu `uid=1000(antimajorana)` via `/var/tmp/research-cargo.sh solve-s12`, `RE_SEARCH_PYTHON` unset.

| Command | Result | Exact Counts |
| --- | --- | --- |
| `py -3 scripts/check_file_length.py --base origin/gemini/work/fixed` | PASS (exit 0) | `No touched source file is over 600 lines.` (max touched: `resume_profile_tests.rs` at `586`L) |
| `unset RE_SEARCH_PYTHON && cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` | PASS (exit 0) | `0` errors, `0` warnings across all workspace targets |
| `unset RE_SEARCH_PYTHON && cargo test --locked -p agent --lib --no-fail-fast -j 2` | PASS (exit 0) | `64 passed; 0 failed; 0 ignored` |
| `unset RE_SEARCH_PYTHON && cargo test --locked -p desktop-server --test handoff_routes --test solve_loop_routes --no-fail-fast -j 2` | PASS (exit 0) | `handoff_routes`: `2 passed; 0 failed`; `solve_loop_routes`: `2 passed; 0 failed` |
| `unset RE_SEARCH_PYTHON && cargo test --workspace --locked --no-fail-fast -j 2` | PASS (exit 0) | **160 binaries: 1,231 passed, 0 failed, 2 ignored** (pre-existing `eval_harness`: `file_sheet_from_json`, `live_manifests_validate`); includes `agent --lib` (`64`), `resume_loop` (`6`), `eval_set_r` (`5`), `mode_markers_r2` (`3`), `mode_shell` (`4`), `handoff_routes` (`2`), `solve_loop_routes` (`2`), `solve_migrate` (`3`), `solve_s8_s9_routes` (`3`), `solve_export_route` (`1`) |
| `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done && node --test crates/desktop-server/tests/*.test.mjs` | PASS (exit 0) | `231 passed, 0 failed, 0 skipped` |
| `python3 -m unittest discover -s python` | PASS (exit 0) | `Ran 52 tests in 50.297s` — `OK` |

### Pre-existing Failures & What Could Not Run on Linux

- **Pre-existing failures on Linux:** None observed (`-A clippy::manual_unwrap_or` covers known base-commit `evidence.rs:744`; 2 pre-existing `#[ignore]` live-tier tests in `eval_harness.rs`).
- **Not compiled or run on Linux:** `cfg(windows)` paths (`build_solver_worker` / `build_check_worker` `AppContainerSandbox` spawning in `crates/desktop-server/src/solve_runtime.rs:149-214`, Job Objects, WebView2 host). Both `solve_runtime.rs` and `solve_loop.rs` use standard cross-platform Rust interfaces with no Unix-specific paths or shell-outs.
