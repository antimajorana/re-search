# Report: `solve-s14-s20-review-blind` — Solve Verifier Loop (S14) & Blind Compute (S20)

- **Brief:** `docs/design/modes-2026-11/agents/solve-s14-s20-review-blind.md`
- **Base commit:** `24be58e6797679cfa423dcc16ea2073b54288714` (`origin/gemini/work/fixed`)
- **Working branch:** `work/solve-s14-s20-review-blind`
- **Pushed branch:** `gemini/modes/solve-s14-s20-review-blind`

---

## 1. Commit History (Separate Per Checkpoint / Fix)

1. `b1c6d07` — `refactor(solve-s14): split verifiers.rs and extract agent_models_tests.rs` (Checkpoint S14a)
2. `4eabc38` — `feat(solve-s14): add shared host compute_review_independence function` (Checkpoint S14b)
3. `2fa966f` — `fix(solve-s14): canonicalize wire/alias models and isolate verifier receipts from G1` (Carbon S14 audit hardening)
4. `64e6504` — `feat(solve-s14): add Solve verifier loop, isolated worker factory, D2 resolver, and review route` (Checkpoint S14c)
5. `ca98a90` — `feat(solve-s20): add Gate G3 and blind compute delegation (compute_blind)` (Checkpoint S20a)
6. *(Final commit)* — `docs(solve-s14-s20): add completion report for S14 and S20`

No commits were squashed. `Cargo.lock`, `crates/agent/Cargo.toml` (still free of `crates/worker`), `crates/research-tools/tests/golden/tool_defs.txt`, `crates/research-tools/tests/golden/tool_profiles.txt`, `autonomous.rs`, `runner.rs`, `http_srv.rs`, `routes_session.rs`, and `handoff.rs` are completely untouched.

---

## 2. Package-by-Package Implementation & File Line Counts

Every new or touched source file is strictly `<= 600` lines (`verifiers.rs <= 450` lines), verified by `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`.

### Package S14 — Solve Verifier Loop

| File | Lines | Role |
|---|---:|---|
| `crates/agent/src/verifiers.rs` | 435 | Slimmed Research verifier entrypoints (`verify_step`, `verify_full_solution`); zero behaviour change (`878 -> 435`L). |
| `crates/agent/src/verifier_context.rs` | 419 | Extracted ledger/prompt context builders from `verifiers.rs`. |
| `crates/agent/src/verifier_parse.rs` | 165 | Extracted `VerificationVerdict`, `VerificationIssue`, and verdict JSON parser. |
| `crates/research-tools/src/solve_model_records.rs` | 467 | Added shared pure host `compute_review_independence(&RoleModel, &RoleModel) -> ReviewIndependence` (`DifferentProvider`, `DifferentModel`, `SameModel`, fail-closed to `SameModel`), canonicalizing provider families (`claude-cli`/`anthropic-messages` -> `anthropic`, `chatgpt`/`codex-cli`/`openai-chat`/`chatgpt-responses` -> `openai`, `gemini-cli`/`gemini-generate-content` -> `gemini`, `ollama-chat` -> `ollama`), vendor prefixes, and Anthropic `"haiku"` -> `"claude-haiku-4-5"`. |
| `crates/research-tools/src/solve_model_records_tests.rs` | 150 | Unit tests for `compute_review_independence` (families, prefixes, wire IDs, aliases, fail-closed blank/empty inputs). |
| `crates/research-tools/src/solve_state.rs` | 546 | Captured `InFlightTool.caller`, prioritized host envelope `tool.result.process`, and excluded `ReceiptProcess::Verifier \| Check` (including `verifier:solve`, `worker:blind_compute`, and >4 KiB blobbed outputs) from solver `pending_runs` (G1 non-poisoning) and `gate_bounces` reset. |
| `crates/agent/src/verifier_tools.rs` | 417 | `IsolatedPythonWorker` + `IsolatedWorkerFactory` traits (keeping `agent` decoupled from `crates/worker`), `LazyIsolatedWorker`, `report_verdict_tool_spec()`, `review_steps_tool_spec()`, `build_verifier_router()` scoped to the 6 `verifier:solve` tools with closure clamping on `ledger_query` and `check_run`. |
| `crates/agent/src/verifier_tools_tests.rs` | 225 | Router allowlist, lazy worker spawn/drop, and closure clamping tests. |
| `crates/agent/src/verifier_loop.rs` | 446 | Multi-turn Solve verifier loop (`SOLVE_VERIFIER_CALLER = "verifier:solve"`, max 12 calls, max 16,384 tokens, lazy isolated worker dropped on exit, cancellation checked pre- and post-stream, `verifier.incomplete` on exhaustion, G6 `verification_stuck` detection). |
| `crates/agent/src/verifier_loop_support.rs` | 592 | Strict R3 `report_verdict` validation (root/item key allowlists, maxLength caps, issue severity enum, rejecting `accepted` with counterexample or blocker issue), R13 citation integrity (`claim_sha256` + `math_sha256`), mid-review mutation guard (`initial_shas` + superseded check), `batch_id` persistence, deepest-first union closure clipping by depth tier. |
| `crates/agent/src/verifier_finish.rs` | 228 | `count_review_loops` (keyed by `batch_id`), `select_finish_review_targets` (`Off`/`FinalOnly`/`ProseOnly`/`All`, skipping `AssumedByModel`, capping prose lift at `ModelAgreed`, honoring `FinalTierAtLeast` & `.max()`), 5-step chunking (`batch_review_targets`), `handle_review_steps`, and `run_finish_review_batches`. |
| `crates/agent/src/verifier_test_support.rs` | 543 | Shared test harness (`ScriptedTransport`, `TestFactory`, `RealStdioPythonWorker` probing `RE_SEARCH_PYTHON` / `python3` / `python` via `import sympy`, session seeders). |
| `crates/agent/src/verifier_loop_tests.rs` | 579 | S14c tests: citation forgery, cross-step citations, R3 smuggling, `accepted`+blocker/counterexample, 12-call `verifier.incomplete`, real Python worker isolation + G1 non-poisoning, G6 `verification_stuck`, deepest-first clipping, mid-review mutation guard, D3 policies/chunking/`max_review_loops`/`batch_id`. |
| `crates/desktop-server/src/agent_models.rs` | 587 | Added `"verifier"` to `AGENT_ROLE_KEYS` and `resolve_solve_verifier_transport` (Decision D2: same-model review still builds a live transport and stamps `SameModel` on host; rejects malformed/CLI/unknown explicit `req.verifier` with 400; normalizes `solver_role` before independence check; legacy Research `resolve_optional_role_transport` untouched). |
| `crates/desktop-server/src/agent_models_tests.rs` | 273 | Extracted inline `agent_models` tests + D2 same-model, alias normalization, and 400 explicit `req.verifier` tests. |
| `crates/desktop-server/src/solve_workers.rs` | 594 | `DesktopIsolatedWorkerFactory` over `crates/worker` using unique `isolated_appcontainer_key` (`iso-<tag>-<16hex>`, `<= 43` chars `[a-z0-9-]`) on Windows with ephemeral dir cleanup on `Drop` and graceful `#[cfg(not(windows))]` error path; auth- and concurrency-guarded (`reserve_turn`, `active_turns` 409, `ApiTurnGuard`) `POST /api/sessions/{id}/solve/review`. |
| `crates/desktop-server/src/api_dispatch.rs` | 484 | Wired `("POST", ["api", "sessions", id, "solve", "review"])` arm into authenticated dispatch. |
| `crates/desktop-server/src/lib.rs` | 251 | Exported `pub mod solve_workers;`. |
| `crates/agent/src/lib.rs` | 107 | Exported `verifier_context`, `verifier_parse`, `verifier_tools`, `verifier_loop`, and `delegate_blind`. |

### Package S20 — Blind Compute & Gate G3

| File | Lines | Role |
|---|---:|---|
| `crates/research-tools/src/solve_gates.rs` | 541 | Added Gate **G3** (`COMPUTE_BLIND`, `COMPUTE_BLIND_MAX_OPERATION_CHARS = 600`, `check_g3_blind_compute`, `enforce_g3_blind_compute`, `SolveGate::before` hook) refusing `compare_with` in `inputs` (exact or base name) or in `operation` (handle substring or identifier-boundary token) and operations `> 600` chars, recording `guardrail.blocked`. |
| `crates/research-tools/src/solve_gates_tests.rs` | 335 | Gate G3 unit tests (valid calls, identifier-boundary precision, input/operation leaks, >600-char refusal, `SolveGate::before`, `enforce_g3_blind_compute` + `gate_bounces`). |
| `crates/agent/src/delegate_blind.rs` | 576 | Host-handled `compute_blind`: `compute_blind_tool_spec()`, `run_compute_blind()`, `handle_compute_blind()`. Enforces G3, G5, and G1 before spawning a fresh isolated Python worker via `IsolatedWorkerFactory`; tracks `delegate.turn` / `delegate.end` (F9 `workers` accounting); breaks early only on explicit `save_expr("blind_result", ...)`; drops worker and canonicalizer promptly before check-process comparison. |
| `crates/agent/src/delegate_blind_check.rs` | 572 | Strict arg parsing (`tolerance` bounded to `(0, 1e-3]`, `points` in `12..=64`, `digits` in `50..=120`), claim binding validation (`Equation \| Value` referencing `compare_with`, forwarding `claim.domain`), blob integrity via `solve_exprs::load_handle_srepr`, blind prompt/exprs builder (strictly `inputs` only), and host check-process comparison (`identity_symbolic` at full Guard (b) precision with timeout -> inconclusive fallback to `identity_numeric`, writing `tool.request` + `tool.result` + `solve.check { name: "independent_recompute" }` with `canonical_json` `evidence_sha256`). |
| `crates/agent/src/delegate_blind_tests.rs` | 562 | Real-Python end-to-end tests: exact symbolic match -> `Tier::Verified` + `compare_with` & solver variable isolation + F9 `delegated` count + G1 non-poisoning; numeric fallback -> `Tier::CheckedNumerically` (never `Verified`); mismatch -> `Tier::Failed`; G3 refusal with 0 calls/spawns; forged claim binding rejection; tolerance/points/digits bounds; pre-cancel zero-spawn; worker drop & `delegate.end { status: "error" }`. |
| `crates/desktop-server/tests/solve_review_blind.rs` | 315 | Desktop integration tests: `POST /api/sessions/{id}/solve/review` auth (401), mode guard (400), D2 same-model execution -> `ModelAgreedSameModel`, 400 on invalid explicit `verifier`, and `handle_compute_blind` Gate G3 zero-HTTP refusal + `DesktopIsolatedWorkerFactory` non-Windows path. |

---

## 3. Deviations, Decisions & Assumptions

1. **S12 (`solve_loop.rs`) not yet merged on `gemini/work/fixed`:** Per `RULES.md` §Scope ("Do not wait for it and do not implement it"), we did **not** create `crates/agent/src/solve_loop.rs` or edit `crates/desktop-server/src/turn_runner.rs`. Instead, we exposed clean, directly callable host-handled seams (`review_steps_tool_spec`, `handle_review_steps`, `run_finish_review_batches` for S14; `compute_blind_tool_spec`, `handle_compute_blind` for S20) and documented exact wiring steps under **Merge notes** below.
2. **Additional small edits outside the brief's short list (justified by audit):**
   - `crates/research-tools/src/solve_state.rs`: Required to prevent verifier and blind-compute `sympy_eval` receipts (including >4 KiB blobbed outputs) from entering solver `pending_runs` and poisoning Gate G1, and to prevent verifier/check receipts from resetting solver `gate_bounces` (G5). Kept small and isolated in its own commit (`2fa966f` + 1-line S20 caller extension).
3. **Bounded `tolerance` on `compute_blind` (`(0, 1e-3]`):** Decided autonomously during S20 audit hardening so a caller cannot pass a loose tolerance (e.g. `1e6`) to disable `identity_symbolic` Guard (b) or earn `Tier::CheckedNumerically` on unrelated expressions. `identity_symbolic` never receives caller `tolerance` (Guard (b) always runs at full precision `10^-(digits - 12)`), and `guard_failed == true` always produces `CheckOutcome::Fail`.
4. **Unique Windows AppContainer keys per isolated worker (`isolated_appcontainer_key`):** Because `crates/worker/src/appcontainer.rs` derives profile name `re-search.worker.<session>` and calls `DeleteAppContainerProfile` on drop, sharing the solver's `session` key would share SIDs and delete the active solver's AppContainer profile when the verifier/blind worker drops. Derived an ephemeral `iso-<tag>-<16hex>` key (`<= 33` chars, `[a-z0-9-]`) with isolated `sessions/<iso_key>/{scratch,pycache}` removed on drop after closing the child process.
5. **Blind-compute `sympy_eval` receipt process attribution:** `ReceiptProcess` currently has `Solver | Verifier | Check`. Blind-compute `sympy_eval` receipts carry `caller: "worker:blind_compute"` and `process: "verifier"` so they do not pollute solver `pending_runs` (G1) and cannot impersonate the check process (`ReceiptProcess::Check`).
6. **Named constants for decision-dependent strings:** All event kinds (`EVENT_VERIFIER_INCOMPLETE`, `SOLVE_VERIFIER_KIND`, `INDEPENDENT_RECOMPUTE_CHECK`, `BLIND_COMPUTE_ROLE`, `BLIND_COMPUTE_CALLER`, `COMPUTE_BLIND`) and caps (`SOLVE_VERIFIER_MAX_MODEL_CALLS`, `SOLVE_VERIFIER_MAX_TOKENS`, `SOLVE_VERIFIER_MAX_BATCH_SIZE`, `COMPUTE_BLIND_MAX_OPERATION_CHARS`, `BLIND_COMPUTE_MAX_TOLERANCE`) live in single named constants with design references.

---

## 4. Merge Notes

### Wiring into S12 (`crates/agent/src/solve_loop.rs` & `crates/desktop-server/src/turn_runner.rs`)

When `solve-s12-loop` merges alongside this branch, connect S14 and S20 at these exact hooks:

1. **Tool advertisement in `crates/agent/src/solve_loop.rs`:**
   - Append `agent::verifier_tools::review_steps_tool_spec()` and `agent::delegate_blind::compute_blind_tool_spec()` to the in-core Solve loop's advertised `ModelRequest.tools` list (host-handled tools, outside the `solve/1` registry).
2. **Host-handled tool dispatch in `crates/agent/src/solve_loop.rs`:**
   - `"review_steps"` -> call `agent::verifier_loop::handle_review_steps(&verifier_ctx, &input)`.
   - `"compute_blind"` -> call `agent::delegate_blind::handle_compute_blind(&blind_ctx, &tool_use_id, &input)`.
3. **Finish-time review before `solve_finish` in `crates/agent/src/solve_loop.rs`:**
   - Before executing `solve_finish`, call `agent::verifier_loop::run_finish_review_batches(&verifier_ctx, &final_entry_id)`. If `summary.verification_stuck` is `Some(_)`, pause with `PauseReason::VerificationStuck` (G6).
4. **Desktop host wiring in `crates/desktop-server/src/turn_runner.rs`:**
   - Construct `Arc::new(crate::solve_workers::DesktopIsolatedWorkerFactory::new(Arc::clone(state)))` as the `IsolatedWorkerFactory` for Solve turns.
   - Resolve the Solve verifier transport via `crate::agent_models::resolve_solve_verifier_transport(state, &ui, Some(&setup.roles), req, Some(&setup.roles.solver))` (Decision D2).

### Small Edits to Shared Existing Files

- `crates/research-tools/src/solve_model_records.rs` (+`compute_review_independence`)
- `crates/research-tools/src/solve_state.rs` (+`InFlightTool.caller`, host `process` priority, G1/G5 exclusion for `Verifier | Check`)
- `crates/research-tools/src/solve_gates.rs` (+Gate G3 in `SolveGate::before`, `check_g3_blind_compute`, `enforce_g3_blind_compute`)
- `crates/agent/src/verifiers.rs` (zero-behaviour-change split into `verifier_context.rs` + `verifier_parse.rs`)
- `crates/agent/src/lib.rs` (module exports only)
- `crates/desktop-server/src/agent_models.rs` (extracted tests to `agent_models_tests.rs`, added `"verifier"` to `AGENT_ROLE_KEYS` and `resolve_solve_verifier_transport`)
- `crates/desktop-server/src/api_dispatch.rs` (+1 route arm for `POST /api/sessions/{id}/solve/review`)
- `crates/desktop-server/src/lib.rs` (+`pub mod solve_workers;`)

---

## 5. Independent Carbon Audits

Both packages underwent adversarial read-only audits (`carbon_auditor`) before final verification and push:
- **S14 audit:** Found 12 issues (2 CI blockers: `verifier_loop_tests.rs` 813L > 600L and Windows `python3` probe; 4 High: wire/alias independence canonicalization, G1 poisoning by verifier `sympy_eval`, missing `reserve_turn`/`active_turns`/`ApiTurnGuard` on `POST /solve/review`, Windows AppContainer profile collision & deletion on drop; 5 Medium: R3 strict key allowlists & `accepted`+blocker/counterexample check, mid-review mutation guard, `batch_id` loop counting, `select_finish_review_targets` `AssumedByModel` skip & `FinalTierAtLeast`, 400 on invalid explicit `req.verifier`; 1 Low-Medium: missing test cases). All 12 fixed and re-audited -> **AUDIT PASS**.
- **S20 audit:** Found 8 issues (1 Critical: `resolve_target_claim` `.unwrap_or(0)` and unverified `claim_index` allowed unrelated `compare_with` to promote a false claim; 1 High: unbounded `tolerance` passed to `identity_symbolic` could disable Guard (b) or earn `CheckedNumerically` on wrong expressions; 4 Medium: blob integrity via `load_handle_srepr`, host-side G5/G1 enforcement, premature break on intermediate `save_expr`, cancel-before-spawn & `delegate.end` on transport error; 1 Low-Medium: symbolic timeout fallback, sequential canonicalizer/check drop, `canonical_json` hash, `call_id` trim, `points`/`digits` bounds; 1 Low: hardening tests). All 8 fixed and re-audited -> **AUDIT PASS**.

---

## 6. Verification Results

All workspace checks were executed on Linux as **non-root (`uid=1001`)** with `RE_SEARCH_PYTHON` **unset** (and no JS assets under `crates/desktop-server/assets/` were touched):

| Command | Result |
|---|---|
| `id -u` | `1001` (non-root) |
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | Exit 0 (`No touched source file is over 600 lines.`) |
| `unset RE_SEARCH_PYTHON && cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` | Exit 0 (0 warnings) |
| `unset RE_SEARCH_PYTHON && cargo test --workspace --locked --no-fail-fast` | Exit 0 — **1214 passed; 0 failed; 2 ignored** (pre-existing live-eval tests: `file_sheet_from_json` and `live_manifests_validate`; 0 `#[ignore]` added by this branch) |
| `cargo test -p research-tools --test tool_def_golden --test tool_profile_golden --locked` | Exit 0 — **6 passed; 0 failed** (`tool_defs.txt` and `tool_profiles.txt` unchanged) |
| `cargo test -p desktop-server --test solve_review_blind --locked` | Exit 0 — **2 passed; 0 failed** |
| `RE_SEARCH_PYTHON="$(command -v python3)" cargo test --locked -p worker -p research-tools -- solve_worker_compat solve_s7_exprs solve_s8_s9_s11 python_worker` | Exit 0 (targeted Python worker & Solve checks) |

### Pre-Existing Failures / Ignored Tests

- Zero pre-existing test failures on Linux when run as non-root with `RE_SEARCH_PYTHON` unset.
- The 2 ignored tests (`file_sheet_from_json` and `live_manifests_validate`) are pre-existing upstream live-tier eval tests gated on `EVAL_SHEET_JSON` / `EVAL_LIVE_MANIFEST_DIR`.

---

## 7. What Could Not Be Compiled or Run on Linux

- All `#[cfg(windows)]` code paths in `crates/desktop-server/src/solve_workers.rs` (`DesktopIsolatedWorkerFactory::build_sandboxed_worker`, `AppContainerSandbox::new` with `isolated_appcontainer_key`, `WorkerRole::Solver` and `WorkerRole::Check` spawning on Windows) are **not compiled or executed on Linux**. Windows CI must verify `#[cfg(windows)]` compilation and AppContainer isolation end-to-end.
- `crates/worker/tests/appcontainer_sandbox.rs` and `crates/worker/tests/python_worker.rs` run 0 tests on Linux (Windows-gated upstream).
- On Linux, real Python worker isolation (`verifier_loop_tests.rs`) and real Python blind-compute + check-process `identity_symbolic` / `identity_numeric` / `canonicalize` (`delegate_blind_tests.rs`) were verified against `python/worker.py --stdio --role solver|check` via `RealStdioPythonWorker` (Python 3.12, SymPy 1.14.0, mpmath 1.3.0, NumPy 2.5.5).

---

## 8. Known Limitations / Observations

- `crates/desktop-server/src/solve_workers.rs` is at `594` lines and `crates/agent/src/verifier_loop_support.rs` is at `592` lines; any future non-trivial additions to either file should extract tests or helpers into a sibling module first to stay under the 600-line ceiling.
- `solve_loop.rs` integration (wiring `review_steps`, finish-time review batching, and `compute_blind` into the live solver loop) remains deferred until `solve-s12-loop` merges on the shared branch, per `RULES.md` §Scope.
