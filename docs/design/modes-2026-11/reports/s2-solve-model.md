# Report: `s2-solve-model` (Solve model, state fold and badge function — package S2)

- **Brief:** `docs/design/modes-2026-11/agents/s2-solve-model.md`
- **Base commit:** `49edc18dd66b85044046dacac186af3593a44de3` (`origin/claude/project-thread-caszr2`)
- **Work branch:** `work/s2-solve-model` -> pushed to `gemini/modes/s2-solve-model`
- **Toolchain (Linux VM):** `rustc 1.100.0`, `cargo 1.100.0`, `Python 3.12.16` (`sympy 1.14.0`, `numpy 2.5.5`, `mpmath 1.3.0`)

---

## 1. What was implemented (package S2)

Pure Rust code in `crates/research-tools/src/` — no I/O, no ledger writes, no network, no new dependencies (`Cargo.lock` untouched), and no tool definitions added or modified (`tool_def_golden` passes with `crates/research-tools/tests/golden/tool_defs.txt` unchanged).

All touched and added source files are under the 600-line limit (`python3 scripts/check_file_length.py --base origin/claude/project-thread-caszr2` passes).

| File | Lines | Role |
|---|---:|---|
| `crates/research-tools/src/solve_model.rs` | 483 | Event kind constants (provisional owner decisions annotated), `HandleRef` (`name@version`), `validate_expr_name`, `is_valid_step_id`/`validate_step_id` (`e-N`), `QuoteRef` (`start_line`/`end_line`), `EvidenceQuoteRecord`, `SolveSetup` (reusing `paper_key::NodeRef`), `StepEntryType` (exact 9 existing `proof.entry` types), `Claim`, `SolveEntry`, `SolveExpr`, `SolveCheck`, deterministic hash helpers (`compute_math_sha256`, `compute_claim_sha256`, `compute_entry_sha256`). Re-exports `solve_model_records::*`. |
| `crates/research-tools/src/solve_model_records.rs` | 354 | Private helper module re-exported by `solve_model`: setup sub-records (`TaskKind`, `TargetForm`, `SolveTarget`, `DoneCriterion*`, `SymbolSpec`, `SetupAssumption`, `GivenSource`, `RoleModel`, `SolveRoles`, `SolveBudgets`, `ReviewPolicy`, `SolvePlan`), review/ack/lifecycle/receipt/done/artifact/guardrail types (`SolveReview`, `SolveAck`, `PauseReason`, `SolvePaused`, `SolveCheckpoint`, `SolveDoneStatus`, `SolveRunStatus`, `SolveDone`, `SolveReceipt`, `SolveArtifact`, `GuardrailBlocked`). |
| `crates/research-tools/src/solve_model_tests.rs` | 243 | Unit tests for handle validation and serde, `e-N` step-id validation, exact 9-type parity with `ledger_append_def()`, `SolveSetup` + `NodeRef` roundtrip, `QuoteRef` `start_line`/`end_line` field names, claim/entry hash separation of math vs prose, `PauseReason` and `SolveDoneStatus` roundtrips. |
| `crates/research-tools/src/proof_closure.rs` | 247 | `DepNode` trait implemented for both `ProofEntry` (declared edges) and `SolveEntry` (declared + derived edges); `all_deps`, `closure_ids`, `proper_ancestor_ids`, `closure_entries`, `closure_for_target`, deterministic cycle-detecting `topological_order`, and `compute_derived_deps` (handle owners + receipt owners for rule R9). |
| `crates/research-tools/src/proof_closure_tests.rs` | 161 | Unit tests for declared+derived closure, `ProofEntry` compatibility, deduplication order, out-of-order topological sort, cycle safety, and `compute_derived_deps`. |
| `crates/research-tools/src/solve_state.rs` | 513 | `SolveState` and incremental `apply(&mut self, &Event)` / `from_events` / `apply_slice` folding all Solve and shared events (`solve.started`, `solve.setup`, `solve.plan`, `proof.entry`, `proof.supersede`, `solve.expr`, `solve.check`, `proof.verification` via `effective_verification_verdict`, `solve.ack`, `evidence.quote`, `tool.request`/`tool.result`, `note` discard handling, `guardrail.blocked`, `solve.paused`, `solve.checkpoint`, `session.resume`, `solve.done`, `solve.artifact`), keeping `tiers`, `pending_runs`, `gate_bounces`, `run_status`, and `upto_seq` up to date. |
| `crates/research-tools/src/solve_state_tests.rs` | 363 | Tests proving incremental fold invariance across every prefix split `k in 0..=len` (both continuation and idempotent re-application), Gate G1 `pending_runs` retirement and `discard:` notes, Gate G5 `gate_bounces` reset rules, and conservative counterexample review rejection. |
| `crates/research-tools/src/solve_badge.rs` | 320 | Private helper module re-exported by `solve_status`: single `wording` constant table (every on-screen badge, sublabel, flag, broken prefix, and provenance tag, with provisional owner-decision comments), `DisplayGrade` (overview 2.10), `BorderStyle` (overview 2.6, owner decision 4), `Tier` with lattice rank, `StepTier`, `StepBadge`, single badge function `badge_for_step`, `compute_all_badges`, and `rests_on_summary`. |
| `crates/research-tools/src/solve_own_tier.rs` | 433 | Private helper module for `solve_status`: own-tier rules 1–6 (`solve.md` 4.4), strict `claim_sha256` + `math_sha256` + handle portability/freshness matching (R4), identity-class check rules per `ClaimForm` (R8), `formal_check` standard-axiom enforcement (R14), `Cited` verification against `evidence.quote`, legacy Research review fallback (8.4/D19). |
| `crates/research-tools/src/solve_status.rs` | 228 | Public facade re-exporting `solve_badge::*` plus `compute_all_tiers` (monotone forward pass over `topological_order` with fixed-point fallback for cyclic inputs, stale/fail propagation, `lowered_by`/`broken_by`, and closure-level flags). |
| `crates/research-tools/src/solve_status_tests.rs` | 503 | Named unit tests for R4, R8, R9, R14, permanent fail and `"Broken: rests on failed e-N"`, supersede-induced Stale, neutral `Given`/`Cited`, transcription/`ran` flags, and the `solve.md` Section 2 worked example. |
| `crates/research-tools/src/solve_status_props_tests.rs` | 443 | Built-in deterministic xorshift64 property tests (zero new dependencies): R7 & R22 over 200 random DAGs (effective <= own, effective <= every declared/derived dep, adding a fail or an edge never raises any tier, no forbidden screen words), R4 over 60 randomized multi-claim/setup-change scenarios, R8 over 60 randomized weak/non-identity check combinations. |
| `crates/research-tools/src/lib.rs` | 276 (+7) | Declares `pub mod proof_closure`, `pub mod solve_model`, `pub mod solve_state`, `pub mod solve_status` and the three private helper modules (`solve_badge`, `solve_model_records`, `solve_own_tier`). |

---

## 2. Deviations from the design and why

1. **`proof_dag.rs` left untouched (no fields added to `ProofEntry`).**
   - `solve.md` 7.2 lists `ProofEntry` in `proof_dag.rs` as gaining `#[serde(default)]` fields.
   - However, `crates/agent/src/guardrails.rs:227` constructs `ProofEntry { ... }` by struct literal (outside S2's scope), `proof_dag.rs` is already 550 lines, and `ledger_query` serializes `ProofEntry` directly.
   - Following `s2-solve-model.md`'s guidance ("read it from the new modules; make an edit there only if it is behaviour-preserving"), S2 keeps `proof_dag.rs` untouched, defines `SolveEntry` in `solve_model.rs`, and implements `proof_closure::DepNode` for both `ProofEntry` and `SolveEntry`.
2. **Internal helper modules split for the 600-line rule (R20).**
   - `solve_model.rs` delegates sub-records to private `solve_model_records.rs` (re-exported via `pub use`).
   - `solve_status.rs` delegates badge types/wording to private `solve_badge.rs` (re-exported via `pub use`) and own-tier evaluation to private `solve_own_tier.rs`.
   - External callers still import exclusively from `research_tools::{solve_model, proof_closure, solve_state, solve_status}`.
3. **Rule ordering between Rule 4 (`Model agreed`) and Rule 5 (`Given`, `Cited`, `Assumed by model`).**
   - In `solve.md` 4.4, Rule 5 is listed after Rule 4. Evaluating Rule 4 before Rule 5 would allow a model review to downgrade a neutral `Given` or `Cited` step to `Model agreed` (lowering downstream `Computed` steps) or allow a second model to bypass user acknowledgement on an unacknowledged solver assumption (contradicting D10 and the worked example).
   - `solve_own_tier.rs` evaluates Rule 1 (`Failed`) first, then `Given` / `Cited` / `AssumedByModel`, then Rules 2, 3, 4, and 6. `Failed` still overrides everything.
4. **Optional `entry_sha256` field added to `SolveCheck`.**
   - `solve.md` 4.1 only defines `solve.check` (no `solve.formal_check` event), with `claim_sha256` as the binding key. A pure theorem/proof step checked by Lean (`formal_check`) may have no SymPy handle claims (`claims: []`). Adding optional `entry_sha256: Option<String>` (and allowing claimless formal checks to store `entry_sha256` in `claim_sha256`) lets `formal_check` bind cleanly to claimless steps without inventing a new event kind.

---

## 3. Decisions taken alone and assumptions

1. **Meet-semilattice rank for neutral tiers (`Given`, `Cited`).**
   - Modeled `Given` and `Cited` as the top/identity element (`lattice_rank = 5`) of the tier lattice (`Failed = -1 < Stale/Unchecked/AssumedByModel = 0 < ModelAgreedSameModel = 1 < ModelAgreed/CheckedByYou = 2 < Computed = 3 < LeanChecked = 4 < Given/Cited = 5`). This implements "skipping Given and Cited" in `min` while making R7 monotonicity hold unconditionally on arbitrary graphs, even if a `Given` node has an incoming edge from a weaker node.
2. **Tie-breaking between `Model agreed (2)` and `Checked by you (2)`.**
   - For a step's `own` tier, an explicit user `hand_checked` acknowledgement takes precedence over `Model agreed` (and strictly outranks `Model agreed (same model)`).
   - During `effective` propagation at rank 2, if any rank-2 contributor is `Model agreed`, `effective` is `Model agreed` (more conservative epistemic grade: model statement rather than user statement); otherwise `Checked by you`.
3. **Stale propagation and setup-math staleness.**
   - A step is marked `stale: true` (effective `Tier::Stale`, rank 0) when: (a) any dependency is superseded or stale (transitive propagation), (b) a setup revision changes `math_sha256` after checks were recorded for the step's claim/entry without a fresh check under the new `math_sha256`, or (c) an in-place re-saved handle changes `srepr_sha256` relative to the step's recorded `claim_sha256`. `Failed` overrides `Stale`.
4. **Prose exclusion in `compute_claim_sha256`.**
   - `Claim.how` (short prose explanation) is excluded from `claim_sha256`; `form`, `lhs` + `lhs_srepr_sha256`, `rhs` + `rhs_srepr_sha256`, `relation`, `order`, and `domain` are included (`domain` is mathematical, per `solve.md` 0.3).
5. **Strict vs permissive step IDs.**
   - `validate_step_id` enforces `e-N` (positive decimal integer, no leading zero, matching `paper_key::mode_of_id == Some(Mode::Solve)`). `SolveState::apply` accepts any non-empty `entry_id` string for backward compatibility with earlier sessions (or prose examples like `e-8b`).
6. **`discard:` note semantics for Gate G1 (`pending_runs`).**
   - Assumption: when a `note` event's trimmed `text` starts with `"discard:"`, if any token matches a pending run's `call_id` or `seq` via `tool_reference_matches`, only those matching runs are retired; if no pending run is named explicitly (e.g. `"discard: exploratory scratch"`), all currently pending runs are retired.
7. **Pure-fold handling of blob-backed `tool.result` payloads.**
   - Assumption: because S2 has no I/O and cannot read `blobs/` from disk, `SolveState::apply` parses `result.inline` (JSON string) or direct `result` JSON objects for `code_sha256`, `wall_ms`, `process`, `exprs_out`, `env`, and `value`/`error`. When `result.blob` is present, `SolveReceipt.result_blob` records the digest and `sympy_eval` with `ok: true` is conservatively tracked in `pending_runs`; for value-claim two-run checks, `evidence.lhs_code_sha256`/`rhs_code_sha256` are accepted as a fallback when receipt `code_sha256` is absent.
8. **Typed vs `Value` fields in `SolveSetup`.**
   - Assumption: `SolveTarget.order` and dimension powers in `SolveSetup.dimensions` use `serde_json::Value` so integer powers (`1`), fractional floats (`1.5`), rational strings (`"3/2"`), and structured order objects all round-trip losslessly.
9. **Provisional constants (owner decisions in `00-overview.md` section 8).**
   - All provisional event kinds live in `solve_model::event_kinds` (annotating decisions 1, 10, 11).
   - All provisional on-screen words live in `solve_status::wording` (`solve_badge.rs`, annotating decisions 3, 4, D1, D2, D10, D11, D13, D14). `"verified"` never appears in any badge output (Lean uses `"Lean-checked"`).

---

## 4. Merge notes

| Place | What another brief / later package delivers | Integration action on merge |
|---|---|---|
| `crates/research-tools/src/lib.rs` (lines 21, 27–32) | Small edit adding `pub mod` declarations for S2 (`proof_closure`, `solve_model`, `solve_state`, `solve_status`) and 3 private helper modules. Isolated in its own commit. | Resolve any adjacent `pub mod` insertions from F3, F4/F9, V1 alphabetically. |
| `crates/research-tools/src/solve_model.rs` (`QuoteRef`, `EvidenceQuoteRecord`, `event_kinds::EVIDENCE_QUOTE`, `CONTEXT_SUPPLIED`) | Brief `f4-f9-quote-budget` delivers `crates/research-tools/src/quote_core.rs` with `QuoteRef` (`source_id`, `path`, `start_line: usize`, `end_line: usize`, `file_sha256`, `quote_sha256`, `evidence_seq: Option<u64>`) and stored-quote payload types. | Replace S2's local `QuoteRef` struct with `pub use crate::quote_core::QuoteRef;` and align `EvidenceQuoteRecord` / `EVIDENCE_QUOTE` with `quote_core.rs`. |
| `crates/research-tools/src/solve_model_records.rs` (`PauseReason`, `SolveDoneStatus`, `SolveRunStatus`) | Brief `f4-f9-quote-budget` delivers `crates/research-tools/src/run_budget.rs` with the shared 7-state on-screen `RunState` enum (`00-overview.md` 3.3). | Implement `From<SolveRunStatus> for RunState` (or map `PauseReason` / `SolveDoneStatus` via `run_budget`'s total mapping function). |
| `crates/research-tools/src/solve_model_records.rs` (`ReviewIndependence`) | Package F12 (or F9, per `00-overview.md` section 9) delivers the shared host `independence` computation function. | Re-export or map `ReviewIndependence` (`different_provider`, `different_model`, `same_model`) to the shared enum once F12 lands. |
| `crates/research-tools/src/solve_own_tier.rs` (`FORMAL_CHECK_NAME`, `classify_formal_check`) | Package S16 delivers `formal_check.rs` and `lean_axioms.rs`. | Confirm S16 emits `solve.check` with `name: "formal_check"`, `entry_sha256`, `evidence.status`, and `evidence.axioms` (or wire S16's event schema into `solve_state.rs`). |
| `crates/research-tools/src/proof_dag.rs` | Not edited by S2. | If S8/S14 later wants `ProofEntry` itself to carry Solve optional fields, note that `crates/agent/src/guardrails.rs:227` constructs `ProofEntry` literally and must be updated together. |

---

## 5. Verification, test commands, and results

Every command was run on `work/s2-solve-model` after all code commits (`221ff62`):

| Command | Result | Exact counts / notes |
|---|---|---|
| `python3 scripts/check_file_length.py --base origin/claude/project-thread-caszr2` | **PASS** (exit 0) | `No touched source file is over 600 lines.` (max touched file: `solve_state.rs` at 513 lines) |
| `cargo test -p research-tools --test tool_def_golden` | **PASS** (exit 0) | 3 passed, 0 failed, 0 ignored; `crates/research-tools/tests/golden/tool_defs.txt` unchanged |
| `cargo test -p research-tools --lib` | **PASS** (exit 0) | 76 passed, 0 failed, 0 ignored (26 new S2 unit + property tests, 50 existing unit tests) |
| `cargo test --workspace --no-fail-fast` | **PASS** (exit 0) | **665 passed, 0 failed, 2 ignored** across all workspace crates |
| `RE_SEARCH_PYTHON="$(command -v python3)" cargo test -p export --no-fail-fast` | **PASS** (exit 0) | 8 passed, 0 failed, 0 ignored (including `run_all_checks.py` Python replay checks) |
| `cargo clippy -p research-tools --lib --profile test -- -D warnings` | **PASS** (exit 0) | 0 warnings across `research-tools` library and all S2 unit/property tests |
| `cargo clippy -p research-tools --test tool_def_golden -- -D warnings` | **PASS** (exit 0) | 0 warnings |
| `cargo clippy --workspace --all-targets --keep-going -- -D warnings` | **PRE-EXISTING FAILURES ONLY** | 0 diagnostics in any S2 file; 6 pre-existing diagnostics in untouched files on `rustc 1.100.0` / Linux (listed below) |

### Breakdown of new S2 tests (26 tests, all passing)

- `solve_model::tests` (7): handle validation/serde, `e-N` step-id validation, exact 9-type parity with `ledger_append_def()`, `SolveSetup`/`NodeRef` roundtrip, `QuoteRef` `start_line`/`end_line` field names, claim/entry hash separation, `PauseReason`/`SolveDoneStatus` roundtrip.
- `proof_closure::tests` (5): declared+derived closure, `ProofEntry` compatibility, `all_deps` deduplication order, out-of-order topological sort + cycle detection, `compute_derived_deps`.
- `solve_state::tests` (4): incremental fold invariance across every prefix split `k in 0..=len` (continuation and idempotent re-application), `pending_runs` and `discard:` note handling, `gate_bounces` increment/reset, conservative counterexample review rejection.
- `solve_status::tests` (7): R4 handle re-save & same-claim re-record, R4 two-claim & setup revision staleness, R8 weak checks/inequalities/numeric thresholds/two-run value claims/non-portable handles, R9 derived dependency lowering, permanent fail + `"Broken: rests on failed e-6"` + supersede-induced Stale, neutral Given/Cited + R14 Lean axiom enforcement, worked example + transcription/`ran` flags.
- `solve_status::prop_tests` (3): R7 & R22 over 200 deterministic random DAGs, R4 over 60 randomized multi-claim/setup scenarios, R8 over 60 randomized non-identity/weak check scenarios.

### Pre-existing failures / diagnostics (proved on base commit `49edc18`)

All 6 diagnostics from `cargo clippy --workspace --all-targets --keep-going -- -D warnings` occur in files untouched by S2 (`git diff origin/claude/project-thread-caszr2...HEAD` touches only `crates/research-tools/src/{lib,proof_closure*,solve_*}.rs`):

1. `crates/desktop-server/src/evidence.rs:744` — `clippy::manual_unwrap_or` (also observed when running `cargo clippy --workspace --all-targets -- -D warnings` on base commit `49edc18` before any edits, and documented in `RULES.md`).
2. `crates/research-tools/tests/sandbox_and_fs.rs:6,9` — `unused_imports` (`is_symlink_or_junction`, `TEX_AUDIT`) on Linux because all call sites (lines 243, 265, 337) are inside `#[cfg(windows)]` blocks (verified identical on base commit via `git show origin/claude/project-thread-caszr2:crates/research-tools/tests/sandbox_and_fs.rs`).
3. `crates/agent/tests/eval_support/manifest.rs:289` — `clippy::match_like_matches_macro` (new in Rust 1.100, untouched file).
4. `crates/agent/tests/eval_support/sheet.rs:102` — `clippy::unnecessary_map_or` (new in Rust 1.100, untouched file).
5. `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` — `unused_imports` (`BufRead`, `BufReader`) on Linux (untouched file).
6. `crates/agent/src/guardrails.rs:335` — `clippy::items_after_test_module` (untouched file).

Also note: running `RE_SEARCH_PYTHON=/usr/bin/python3 cargo test --workspace` globally on Linux hangs in `crates/cli/tests/desktop_and_ext.rs::cli_research_workspace_read_fork_diff_export_and_verify` (see Section 7, item 3). Running `cargo test --workspace --no-fail-fast` without `RE_SEARCH_PYTHON`, plus `RE_SEARCH_PYTHON=/usr/bin/python3 cargo test -p export --no-fail-fast`, passes with 0 failures.

---

## 6. What could not be compiled or run on Linux

- All `#[cfg(windows)]` code paths across the workspace (AppContainer sandbox, Job Objects, WebView2 `desktop-app`, Windows-only tests in `sandbox_and_fs.rs`, `chatgpt-auth`, `worker`, `cli-route`, `cli/tests/contained_router.rs`, `worker/tests/python_worker.rs`, `worker/tests/appcontainer_sandbox.rs`) are not compiled or executed on this Linux VM.
- S2 itself (`solve_model`, `proof_closure`, `solve_state`, `solve_status`) is 100% platform-neutral Rust with zero OS-specific code or paths.

---

## 7. Bugs / issues noticed but not fixed

1. `crates/desktop-server/src/evidence.rs:744` — pre-existing Rust 1.97+/1.100 `clippy::manual_unwrap_or` error (known on base commit; file is over 600 lines and not in S2's scope). Note that clippy's suggestion (`unwrap_or`) would alter behavior for unexpected `Some(other)` status strings, so this is a clippy false positive.
2. `crates/research-tools/tests/sandbox_and_fs.rs:6,9` — pre-existing Linux-only `unused_imports` warning (`is_symlink_or_junction` and `TEX_AUDIT`) because all call sites (lines 243, 265, 337) are gated behind `#[cfg(windows)]`. Left untouched per RULES.md.
3. `crates/cli/tests/desktop_and_ext.rs:149-168` — when `RE_SEARCH_PYTHON` is exported in the environment on Linux, `re-search run --route api` rejects non-Windows `--python`/`RE_SEARCH_PYTHON` and exits immediately before contacting `ScriptedMock`. Line 168 then calls `mock.finish()` before asserting `out.status`, deadlocking on `listener.accept()` waiting for 3 HTTP requests that will never arrive. Left untouched as outside S2's scope.

---

## 8. Contradictions or ambiguities found in the design

1. **`proof_dag.rs` vs `guardrails.rs`:** `solve.md` 7.2 proposes adding `#[serde(default)]` fields to `ProofEntry` in `proof_dag.rs`, which breaks struct-literal construction in `crates/agent/src/guardrails.rs:227` (outside S2) and risks altering `ledger_query` serialization. Resolved cleanly by keeping `ProofEntry` unchanged and introducing `SolveEntry` + `proof_closure::DepNode`.
2. **Claimless `formal_check` binding:** `solve.md` 4.1 defines `solve.check` keyed by `claim_sha256` without a separate formal-check event kind, yet a Lean theorem proof step may have no handle claims. Resolved by adding optional `entry_sha256` to `SolveCheck`.
3. **Rule 4 vs Rule 5 precedence in `solve.md` 4.4:** Listing `Given`/`Cited`/`Assumed by model` after `Model agreed` would let a model review downgrade neutral `Given`/`Cited` steps or bypass D10 on solver assumptions. Resolved by evaluating `Given`/`Cited`/`AssumedByModel` right after `Failed`.
4. **Step id `e-8b` in worked example (`solve.md` line 131):** `00-overview.md` 2.3 and `s2-solve-model.md` define step ids as `e-N`, while the worked example prose uses `e-8b`. Resolved by validating `e-N` strictly in `validate_step_id` while accepting any non-empty `entry_id` in `SolveState::apply`.
5. **Exact types of `SolveTarget.order` and `SolveSetup.dimensions` powers:** Unspecified in `solve.md` 4.1 (`order?`, `{symbol: {base: power}}`). Typed as `serde_json::Value` to remain lossless across integers, floats, rational strings, and structured order objects.
