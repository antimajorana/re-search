# Solve Packages S8 -> S9 -> S11 Implementation & Verification Report

- **Branch**: `gemini/modes/solve-s8-s9-s11`
- **Branch base**: `374744e` (`Merge pull request #46 from antimajorana/coord/modes-inv-v9`)
- **Checkpoint commits**:
  - **S8**: `f57373a0f70d6f12971822e89515ea8d3187abf5`
  - **S9**: `3102b5fc2ceeaf6c2eebd82fdf2efc86abb4f92d`
  - **S11**: final commit on `gemini/modes/solve-s8-s9-s11`

---

## 1. Scope Implemented

### Package S8 — `step_record`, Receipts View, `attach-source`, and `solve/ack`
- **`crates/research-tools/src/solve_receipts.rs` (`287`L) + `solve_receipts_tests.rs` (`138`L)**:
  - Pure receipt builder (`build_receipts`, `find_receipt`, `resolve_cited_receipts`, `has_successful_sympy_receipt`) reconstructing `call_id -> ToolReceiptRecord` over ledger events with verified blob decoding and fail-closed `result_missing` detection.
- **`crates/research-tools/src/solve_steps.rs` (`364`L) + `solve_steps_parse.rs` (`283`L) + `solve_steps_tests.rs` (`418`L)**:
  - `step_record_handler` / `run_step_record` enforcing R5 (renders claim LaTeX strictly from harvested `solve.expr`), R9 (requires non-empty `depends_on` for `derivation`, `result`, `final_answer`), R10 (requires successful `sympy_eval` receipt for computation steps), R17 (`reference_lookup` requires `source_ref` verified via F4 `verify_quote_ref` + `lines_sent_with_ledger`), handle portability & latest-version rules, `Failed`-closure refusal on `final_answer`, host-computed `derived_depends_on` via `proof_closure::compute_derived_deps`, and atomic `claim_sha256`/`entry_sha256` appending `proof.entry` v1 (and optional `solve.plan` v1).
- **`crates/research-tools/src/solve_state.rs` (`525`L)**:
  - Added `extract_exprs_out` handling both JSON array and object shapes emitted by protocol-2 `worker.py`.
- **`crates/desktop-server/src/solve_routes_ext.rs` (`401`L) + `solve.rs` (`332`L) + `lib.rs` (`244`L)**:
  - `POST /api/sessions/{id}/solve/attach-source` (F10 library / F3 fetch / session source -> F4 `verify_quote` -> 300-char clipped `evidence.quote`, `ocr` page tagging).
  - `POST /api/sessions/{id}/solve/ack` (hash-checked `AckKind::{Assumption, Transcription, HandChecked}` appending `solve.ack` v1 and returning updated badge + view).

### Package S9 — Algebra Check Registry, `check_run`, `AutoCheckGate`, and `solve/check` & `recheck`
- **`python/checks/algebra.py` (`383`L), `python/checks/algebra_probe.py` (`455`L), `python/checks/__init__.py` (`124`L), `python/tests/test_checks_algebra.py` (`208`L)**:
  - Five host-registered checks using safe `srepr_safe.py` parsing (zero `eval`/`exec`/string `sympify`):
    1. `identity_symbolic`: domain/assumption-aware exact SymPy simplification with numeric mpmath counterexample guard (catches `Q-check-01` sampled-domain sqrt trap and `Q-check-02` complex principal-branch cut error).
    2. `identity_numeric`: high-precision mpmath evaluation (`>= 12` points, `>= 50` digits for `CheckedNumerically` eligibility).
    3. `series_match`: exact + numeric Taylor series residual comparison to requested order.
    4. `vanishes_to_order`: exact + numeric vanishing order verification (`order_variable`, `order_power`).
    5. `counterexample_search`: domain-constrained numerical inequality falsification.
- **`crates/worker/src/protocol.rs` (`419`L) & `packaging/windows/re-search.nsi` (`244`L)**:
  - Staged `checks/algebra.py` and `checks/algebra_probe.py` in `STAGED_CHECK_FILES` and Windows NSIS installer.
- **`crates/research-tools/src/solve_checks.rs` (`568`L) + `solve_checks_tests.rs` (`261`L)**:
  - `CheckProcessRunner` boundary, `check_run_handler` (R3: check name + params only; 10s default timeout -> `inconclusive`; `receipt.result_seq = expected_check_seq + 1` matching Router `tool.result`; `process: "check"` + `wall_ms`), `evaluate_recheck_diff` (R19), and `AutoCheckGate` (`tools::Gate` post-call hook running `host:check` after `step_record`, recording `not_run` on denial, symbolic -> numeric fallback, and S10 `dimensions` `not_run` fallback).
- **`crates/desktop-server/src/solve_check_routes.rs` (`382`L) + `solve_check_routes_tests.rs` (`318`L) + `tests/solve_s8_s9_routes.rs` (`357`L)**:
  - `POST /api/sessions/{id}/solve/check` and `POST /api/sessions/{id}/solve/recheck` (R18 containment fail-closed returning HTTP `403` when uncontained, R19 reproducibility + `"result changed on re-run"` warning).

### Package S11 — Solve Gates (`G1`/`G4`/`G5`/`G6`/`G7`), `solve_finish`, and Set Q M3 Gate
- **`crates/research-tools/src/solve_done.rs` (`360`L) + `solve_done_criteria.rs` (`291`L) + `solve_done_tests.rs` (`393`L)**:
  - `solve_finish_handler` / `run_solve_finish`: evaluates all five `DoneCriterionKind`s (`FinalTierAtLeast`, `ClosureTierAtLeast`, `NamedCheckPassed`, `NoOpenFlags`, `UserConfirms`), builds `rests_on_summary` + `ModelCallsBreakdown` via `run_budget::count_model_calls_breakdown`, enforces R15 (host-computed authoritative `finished` vs `finished_open_items`), enforces G7 closure guard (refuses `Failed` or `Stale` closures), and enforces Solve-tier-aware `lint_solve_summary_r22` (never calls `agent::honesty`, disallows retired `"Computed"` and non-negated `"proved"`/`"proven"`, and permits `"verified"` only on `Tier::Verified` steps/closures).
- **`crates/research-tools/src/solve_gates.rs` (`423`L) + `solve_gates_tests.rs` (`245`L)**:
  - Placed in `crates/research-tools` to avoid any `agent` dependency in `mcp-server` and keep `Cargo.lock` untouched.
  - `SolveGate` implementing `tools::Gate`:
    - **G1**: blocks 4th un-cited `sympy_eval` (and non-cleanup tools when `> 3` pending runs exist) until cited via `step_record` or discarded via `ledger_note` `"discard:..."`.
    - **G4**: compares F9 `run_budget::count_model_calls` (`RunScope::AfterSeq`) and wall elapsed time against `SolveBudgets` (`leg_cap`, `budget`).
    - **G5**: pauses (`gate_bounces`, R12) when `state.gate_bounces >= budgets.max_gate_bounces`.
    - **G6**: detects 2 rejected `ProofReview` events on the same supersede lineage (`verification_stuck`).
    - **G7**: validates closure & R22 summary before `solve_finish` executes.
  - Exposes `solve_check_host_v1` and `solve_gate_chain` while preserving uncontained S7 stubs in `solve_defs.rs` when `ProfileHostTools::solve_check` is `None`.
- **`crates/research-tools/tests/solve_rig/mod.rs` (`332`L), `solve_s8_s9_s11.rs` (`173`L), `solve_set_q_m3.rs` (`285`L)**:
  - End-to-end tests driving real OS child Python `worker.py --stdio --role solver` and `--role check` through `solve/1` `Router` + `GateChain(SolveGate, AutoCheckGate)` + `solve_check_host_v1`.
  - Audited handler sources (`solve_checks.rs`, `solve_done.rs`, `solve_gates.rs`, `solve_steps.rs`, `solve_steps_parse.rs`) added to `crates/desktop-server/tests/mode_markers_r2.rs` (`442`L).

---

## 2. File Inventory & Line Counts (All <= 600 Lines)

| File | Lines | Scope |
|---|---|---|
| `crates/desktop-server/src/lib.rs` | 244 | S8/S9 route dispatch |
| `crates/desktop-server/src/solve.rs` | 332 | S8/S9 `/solve/*` dispatch |
| `crates/desktop-server/src/solve_check_routes.rs` | 382 | S9 `/solve/check` & `/recheck` |
| `crates/desktop-server/src/solve_check_routes_tests.rs` | 318 | S9 route tests (R18/R19) |
| `crates/desktop-server/src/solve_routes_ext.rs` | 401 | S8 `attach-source` & `ack` |
| `crates/desktop-server/tests/mode_markers_r2.rs` | 442 | R2 source audit list |
| `crates/desktop-server/tests/solve_s8_s9_routes.rs` | 357 | S8/S9 HTTP integration tests |
| `crates/research-tools/src/lib.rs` | 347 | Solve S8/S9/S11 exports |
| `crates/research-tools/src/solve_checks.rs` | 568 | S9 `check_run` & `AutoCheckGate` |
| `crates/research-tools/src/solve_checks_tests.rs` | 261 | S9 `AutoCheckGate` tests |
| `crates/research-tools/src/solve_done.rs` | 360 | S11 `solve_finish`, G7, R22 lint |
| `crates/research-tools/src/solve_done_criteria.rs` | 291 | S11 5 `DoneCriterionKind` evaluators |
| `crates/research-tools/src/solve_done_tests.rs` | 393 | S11 `solve_done` / R15 / R22 tests |
| `crates/research-tools/src/solve_gates.rs` | 423 | S11 `SolveGate` (G1/G4/G5/G6/G7) |
| `crates/research-tools/src/solve_gates_tests.rs` | 245 | S11 `SolveGate` unit tests |
| `crates/research-tools/src/solve_receipts.rs` | 287 | S8 receipt builder |
| `crates/research-tools/src/solve_receipts_tests.rs` | 138 | S8 receipt unit tests |
| `crates/research-tools/src/solve_state.rs` | 525 | S8 `extract_exprs_out` fix |
| `crates/research-tools/src/solve_steps.rs` | 364 | S8 `step_record` handler |
| `crates/research-tools/src/solve_steps_parse.rs` | 283 | S8 `step_record` arg parser |
| `crates/research-tools/src/solve_steps_tests.rs` | 418 | S8 `step_record` unit tests |
| `crates/research-tools/tests/solve_rig/mod.rs` | 332 | Cross-process Python E2E harness |
| `crates/research-tools/tests/solve_s8_s9_s11.rs` | 173 | S8/S9/S11 E2E gate tests |
| `crates/research-tools/tests/solve_set_q_m3.rs` | 285 | Set Q M3 gate (3/3 `Failed`) |
| `crates/worker/src/protocol.rs` | 419 | S9 `STAGED_CHECK_FILES` |
| `packaging/windows/re-search.nsi` | 244 | S9 NSIS check script staging |
| `python/checks/__init__.py` | 124 | S9 algebra check registration |
| `python/checks/algebra.py` | 383 | S9 5 algebraic checks |
| `python/checks/algebra_probe.py` | 455 | S9 domain-aware mpmath probing |
| `python/tests/__init__.py` | 1 | Python test package marker |
| `python/tests/test_checks_algebra.py` | 208 | S9 Python check unit tests |

---

## 3. Frozen-File Disjointness

Zero modifications were made against branch base `374744e` to any frozen or out-of-lane file (`Cargo.lock`, `tool_profile.rs`, `tests/golden/tool_profiles.txt`, `tests/golden/tool_defs.txt`, `policy_preset.rs`, `desktop-server/src/routers.rs`, `proof_dag.rs`, `solve_status_tests.rs`, `investigation_evidence.rs`, `agent/src/honesty.rs`, `agent/src/runner.rs`, `agent/src/verifiers.rs`, `agent/src/resume.rs`, `.github/`).

---

## 4. Set Q M3 Gate Result

All 3 planted-error `check_derivation` cases in `tests/fixtures/eval/manifest-q.json` (`Q-check-01-sampled-domain-sqrt`, `Q-check-02-branch-cut-log`, `Q-check-03-gaussian-moment-ibp`) executed through real cross-process Python `worker.py --stdio --role solver` & `--role check`, `step_record`, `AutoCheckGate`, `SolveGate` G7, and `solve_finish`:
- **3 / 3 (100%)** detected at the planted erroneous step (`effective_tier == Failed`).
- Downstream algebraically-valid dependent steps inherited `effective_tier == Failed` (`"Broken: rests on failed ..."`).
- **0 / 3 (0%)** ended `Verified` or `Numerically checked`.
- **3 / 3 (100%)** refused by `SolveGate` G7 on `solve_finish` (`closure contains Failed step(s)`), writing zero `solve.done` events.
