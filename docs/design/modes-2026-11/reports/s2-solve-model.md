# Report: `s2-solve-model` (Solve model, state fold and badge function — package S2)

- **Brief:** `docs/design/modes-2026-11/agents/s2-solve-model.md` (updated to the 2026-11-25 revision on `origin/gemini/work/fixed` at `e151747`)
- **Base commit:** `49edc18dd66b85044046dacac186af3593a44de3` (`origin/claude/project-thread-caszr2`)
- **Work branch:** `work/s2-solve-model` -> pushed to `gemini/modes/s2-solve-model`
- **Toolchain (Linux VM):** `rustc 1.100.0`, `cargo 1.100.0`, `Python 3.12.16` (`sympy 1.14.0`, `numpy 2.5.5`, `mpmath 1.3.0`)

---

## 1. What was implemented (package S2)

Pure Rust code in `crates/research-tools/src/` — no I/O, no ledger writes, no network, no new dependencies (`Cargo.lock` untouched), and no tool definitions added or modified (`tool_def_golden` passes with `crates/research-tools/tests/golden/tool_defs.txt` unchanged).

All touched and added source files are under the 600-line limit (`python3 scripts/check_file_length.py --base origin/claude/project-thread-caszr2` and `--base origin/gemini/work/fixed` both pass).

| File | Lines | Role |
|---|---:|---|
| `crates/research-tools/src/solve_model.rs` | 486 | Event kind constants (including `LESSONS_SUPPLIED`, owner-decision annotations), `HandleRef` (`name@version`), `validate_expr_name`, `is_valid_step_id`/`validate_step_id` (`e-N`), `QuoteRef` (`start_line`/`end_line`), `EvidenceQuoteRecord` (with `ocr: bool`), `SolveSetup` (reusing `paper_key::NodeRef`), `StepEntryType` (exact 9 existing `proof.entry` types), `Claim`, `SolveEntry`, `SolveExpr`, `SolveCheck` (no Lean/formal fields), deterministic hash helpers (`compute_math_sha256`, `compute_claim_sha256`, `compute_entry_sha256`). Re-exports `solve_model_records::*`. |
| `crates/research-tools/src/solve_model_records.rs` | 351 | Private helper module re-exported by `solve_model`: setup sub-records (`TaskKind`, `TargetForm`, `SolveTarget`, `DoneCriterion*`, `SymbolSpec`, `SetupAssumption`, `GivenSource`, `RoleModel`, `SolveRoles` without `formal`, `SolveBudgets`, `ReviewPolicy`, `SolvePlan`), review/ack/lifecycle/receipt/done/artifact/guardrail types (`AckKind` with 3 variants: `Assumption`, `Transcription`, `HandChecked`, `SolveReview`, `SolveAck`, `PauseReason`, `SolvePaused`, `SolveCheckpoint`, `SolveDoneStatus`, `SolveRunStatus`, `SolveDone`, `SolveReceipt`, `SolveArtifact`, `GuardrailBlocked`). |
| `crates/research-tools/src/solve_model_tests.rs` | 242 | Unit tests for handle validation and serde, `e-N` step-id validation, exact 9-type parity with `ledger_append_def()`, `SolveSetup` + `NodeRef` roundtrip, `QuoteRef` `start_line`/`end_line` field names, claim/entry hash separation of math vs prose, `PauseReason` and `SolveDoneStatus` roundtrips. |
| `crates/research-tools/src/proof_closure.rs` | 247 | `DepNode` trait implemented for both `ProofEntry` (declared edges) and `SolveEntry` (declared + derived edges); `all_deps`, `closure_ids`, `proper_ancestor_ids`, `closure_entries`, `closure_for_target`, deterministic cycle-detecting `topological_order`, and `compute_derived_deps` (handle owners + receipt owners for rule R9). |
| `crates/research-tools/src/proof_closure_tests.rs` | 161 | Unit tests for declared+derived closure, `ProofEntry` compatibility, deduplication order, out-of-order topological sort, cycle safety, and `compute_derived_deps`. |
| `crates/research-tools/src/solve_state.rs` | 517 | `SolveState` and incremental `apply(&mut self, &Event)` / `from_events` / `apply_slice` folding all Solve and shared events (`solve.started`, `solve.setup`, `solve.plan`, `proof.entry`, `proof.supersede`, `solve.expr`, `solve.check`, `proof.verification` via `effective_verification_verdict`, `solve.ack`, `evidence.quote` including `ocr` flag, `tool.request`/`tool.result`, `note` discard handling, `guardrail.blocked`, `solve.paused`, `solve.checkpoint`, `session.resume`, `solve.done`, `solve.artifact`; ignores `lessons.supplied` per R24), keeping `tiers`, `pending_runs`, `gate_bounces`, `run_status`, and `upto_seq` up to date. |
| `crates/research-tools/src/solve_state_tests.rs` | 366 | Tests proving incremental fold invariance across every prefix split `k in 0..=len` (continuation and idempotent re-application, including `lessons.supplied`), Gate G1 `pending_runs` retirement and `discard:` notes, Gate G5 `gate_bounces` reset rules, and conservative counterexample review rejection. |
| `crates/research-tools/src/solve_badge.rs` | 398 | Private helper module re-exported by `solve_status`: single `wording` constant table with `NON_VERIFIED_CONSTANTS` (allowing `"verified"` ONLY in `BADGE_VERIFIED` and `OWN_CHECK_VERIFIED`), `DisplayGrade` (overview 2.10 including `MatchedQuoteScannedPage`), `BorderStyle` (overview 2.6), `Tier` (`Unchecked (0) < ModelAgreedSameModel (1) < ModelAgreed (2) = CheckedByYou (2) < CheckedNumerically (3) < Verified (4)`; neutral `Given`/`Cited`, overriding `Failed`, `Stale`, `AssumedByModel`; no `Computed` and no `LeanChecked`), `StepTier`, `StepBadge` (with `own_check_line`), `badge_for_step` (formatting superseded deps as `(superseded)`), `compute_all_badges`, and `rests_on_summary`. |
| `crates/research-tools/src/solve_own_tier.rs` | 415 | Private helper module for `solve_status`: own-tier rules 1 and 3–6 (`solve.md` 4.4; rule 2 cut with Lean), strict `claim_sha256` + `math_sha256` + handle portability/freshness matching (R4), per-claim best / per-step min computed tier (`Verified` vs `CheckedNumerically`) across `identity_symbolic`, `identity_numeric`, `series_match`/`vanishes_to_order` (exact vs numeric), two-run `value` checks, `independent_recompute` (exact vs numeric), `counterexample_search` (R8), `Cited` OCR detection, and legacy Research review fallback (8.4/D19). |
| `crates/research-tools/src/solve_status.rs` | 230 | Public facade re-exporting `solve_badge::*` plus `compute_all_tiers` (monotone forward pass over `topological_order` with fixed-point fallback for cyclic inputs, stale/fail propagation, `lowered_by`/`broken_by`, and flags: `"transcription unconfirmed"`, `"same model"`, `"to O(x^n)"`, `"rests on N assumptions made by the model"`, `"OCR source"`). Reads no lessons (R24). |
| `crates/research-tools/src/solve_status_tests.rs` | 553 | Named unit tests for wording-constant exclusivity (R22), explicit tier order, R4 handle re-save/same-claim re-record/two-claim/setup staleness, R8 weak checks/mixed symbolic+numeric step/failed numeric guard/exact vs numeric series/`independent_recompute`/two-run values/non-portable handles, R9 derived dependency lowering + `"own check: verified"` line, permanent fail + `"Broken: rests on failed e-6"` + supersede-induced Stale, neutral Given/Cited/OCR Cited, earlier `lean_check` receipt (D19), R24 `lessons.supplied` invariance, and the worked example. |
| `crates/research-tools/src/solve_status_props_tests.rs` | 464 | Built-in deterministic xorshift64 property tests (zero new dependencies): R7 & R22 over 200 random DAGs (effective <= own, effective <= every declared/derived dep, adding a fail or edge never raises any tier, `"verified"` appears ONLY in `label` when `effective == Verified` or `own_check_line` when `own == Verified && effective != Verified`, and never `"proved"`, `"new"`, `"novel"`, or `"computed"`), R4 over 60 randomized multi-claim/setup scenarios, R8 over 60 randomized weak/under-threshold/numeric check scenarios (confirming `identity_numeric` always yields `CheckedNumerically`, never `Verified`). |
| `crates/research-tools/src/lib.rs` | 276 (+7) | Declares `pub mod proof_closure`, `pub mod solve_model`, `pub mod solve_state`, `pub mod solve_status` and the three private helper modules (`solve_badge`, `solve_model_records`, `solve_own_tier`). |

---

## 2. Deviations from the design and why

1. **`proof_dag.rs` left untouched (no fields added to `ProofEntry`).**
   - `solve.md` 7.2 lists `ProofEntry` in `proof_dag.rs` as gaining `#[serde(default)]` fields.
   - However, `crates/agent/src/guardrails.rs:227` constructs `ProofEntry { ... }` by struct literal (outside S2's scope), `proof_dag.rs` is already 550 lines, and `ledger_query` serializes `ProofEntry` directly.
   - Following `s2-solve-model.md`, S2 keeps `proof_dag.rs` untouched, defines `SolveEntry` in `solve_model.rs`, and implements `proof_closure::DepNode` for both `ProofEntry` and `SolveEntry`.
2. **Internal helper modules split for the 600-line rule (R20).**
   - `solve_model.rs` delegates sub-records to private `solve_model_records.rs` (re-exported via `pub use`).
   - `solve_status.rs` delegates badge types/wording to private `solve_badge.rs` (re-exported via `pub use`) and own-tier evaluation to private `solve_own_tier.rs`.
   - External callers still import exclusively from `research_tools::{solve_model, proof_closure, solve_state, solve_status}`.
3. **Rule ordering between Rule 4 (`Model agreed`) and Rule 5 (`Given`, `Cited`, `Assumed by model`).**
   - In `solve.md` 4.4, Rule 5 is listed after Rule 4. Evaluating Rule 4 before Rule 5 would allow a model review to downgrade a neutral `Given` or `Cited` step to `Model agreed` (lowering downstream `Verified`/`CheckedNumerically` steps) or allow a second model to bypass user acknowledgement on an unacknowledged solver assumption (contradicting D10).
   - `solve_own_tier.rs` evaluates Rule 1 (`Failed`) first, then `Given` / `Cited` / `AssumedByModel`, then Rules 3, 4, and 6. `Failed` still overrides everything.

---

## 3. Decisions taken alone and assumptions

1. **Meet-semilattice rank for neutral tiers (`Given`, `Cited`) = 4 (equal to `Verified`).**
   - Modeled `Failed = -1 < Stale/Unchecked/AssumedByModel = 0 < ModelAgreedSameModel = 1 < ModelAgreed/CheckedByYou = 2 < CheckedNumerically = 3 < Verified = Given = Cited = 4`, keeping `eval.own` when `min_rank == 4`.
   - This skips `Given`/`Cited` in `min` for non-neutral steps, guarantees a `Given` or `Cited` step can NEVER be relabeled `"Verified"` (even if a malformed graph gives it an edge to a `Verified` step), prevents a `Given`/`Cited` step from laundering a weaker (`0..=3`), `Stale`, or `Failed` dependency, and keeps R7 monotonicity unconditional across arbitrary graphs.
2. **Superseded lowering dependencies shown as `(superseded)` in `rests_on_line`.**
   - When a `Verified` step `e-2` is superseded, its dependent `e-4` becomes `Stale` with `lowered_by = ["e-2"]`. `badge_for_step` formats `rests_on_line` as `"rests on e-2 (superseded)"` rather than `"rests on e-2 (verified)"`, both for accuracy and to satisfy R22.
3. **Fail-closed exact-vs-numeric evidence check for `series_match`, `vanishes_to_order`, and `independent_recompute`.**
   - Assumption: `solve_own_tier::is_exact_comparison` grants `Tier::Verified` only when `evidence["exact"] == true` or `evidence["comparison"] == "exact"` (and `evidence["numeric_coefficients"] != true`); otherwise a passing check yields `Tier::CheckedNumerically`. This fails closed so a numeric check missing an explicit flag can never receive `"Verified"`.
4. **Tie-breaking between `Model agreed (2)` and `Checked by you (2)`.**
   - For a step's `own` tier, explicit user `hand_checked` takes precedence over `Model agreed` (and outranks `Model agreed (same model)`). During `effective` propagation at rank 2, `Model agreed` is chosen if any rank-2 contributor is `Model agreed` (more conservative provenance); otherwise `Checked by you`.
5. **Stale propagation and setup-math staleness.**
   - A step is `stale: true` (effective `Tier::Stale`, rank 0) when: (a) any dependency is superseded or stale, (b) a setup revision changes `math_sha256` after checks were recorded without a fresh check under the new `math_sha256`, or (c) an in-place re-saved handle changes `srepr_sha256` relative to the step's recorded `claim_sha256`. `Failed` overrides `Stale`.
6. **Prose exclusion in `compute_claim_sha256`.**
   - `Claim.how` is excluded from `claim_sha256`; `form`, `lhs` + `lhs_srepr_sha256`, `rhs` + `rhs_srepr_sha256`, `relation`, `order`, and `domain` are included.
7. **OCR flag detection on `evidence.quote`.**
   - Assumption: `solve_state::parse_evidence_quote` sets `EvidenceQuoteRecord.ocr = true` if `payload["ocr"] == true`, `payload["scanned"] == true`, or `payload["source_kind"] == "ocr"` (defaulting to `false`). When `ocr == true`, `solve_status` adds flag `"OCR source"`, sets `DisplayGrade::MatchedQuoteScannedPage`, and sets provenance tag `"OCR"`.
8. **`discard:` note semantics for Gate G1 (`pending_runs`).**
   - Assumption: a `note` event whose trimmed `text` starts with `"discard:"` retires only the named pending runs if any token matches a pending run via `tool_reference_matches`, or clears all currently pending runs if none is named explicitly.
9. **Pure-fold handling of blob-backed `tool.result` payloads.**
   - Assumption: because S2 has no I/O, `SolveState::apply` decodes inline or direct `result` JSON for `code_sha256`, `wall_ms`, `process`, `exprs_out`, `env`, and `value`/`error`; for blob-backed results, `SolveReceipt.result_blob` records the digest and value-claim two-run checks fall back to `evidence.lhs_code_sha256`/`rhs_code_sha256`.
10. **Typed vs `Value` fields in `SolveSetup`.**
    - Assumption: `SolveTarget.order` and dimension powers in `SolveSetup.dimensions` use `serde_json::Value` so integers, floats, rational strings (`"3/2"`), and structured order objects all round-trip losslessly.

---

## 4. Merge notes

### Changes made to align with the 2026-11-25 brief revision (second commit on branch)

The initial implementation (`aa13da7`) followed the pre-revision brief on `origin/claude/project-thread-caszr2` (which had `Tier::Computed (3)`, `Tier::LeanChecked (4)`, `formal_check`, `AckKind::FormalStatement`, `SolveRoles.formal`, and banned `"verified"` outside Lean). Commit `6871878` brought S2 in line with the revised brief on `origin/gemini/work/fixed`:

- **Cut Lean / formal tier:** removed `Tier::LeanChecked`, `FORMAL_CHECK_NAME`, `STANDARD_LEAN_AXIOMS`, `FormalOutcome` / `classify_formal_check`, `SolveRoles.formal`, `AckKind::FormalStatement`, `SolveCheck.entry_sha256`, `BADGE_LEAN_CHECKED`, and `FLAG_STATEMENT_MATCH_UNCONFIRMED`. Verified via test that an earlier `lean_check` tool result folds as a receipt and grants no tier (D19).
- **Replaced single `Computed` tier with two computed tiers:** removed `Tier::Computed` and `BADGE_COMPUTED`; added `Tier::CheckedNumerically (3)` (`"Checked numerically"`) and `Tier::Verified (4)` (`"Verified"`). Implemented per-claim best / per-step min selection in `solve_own_tier.rs` (symbolic equation -> `Verified`, 12-point/50-digit numeric equation -> `CheckedNumerically` with `"12 points, 50 digits"` sublabel, exact vs numeric series -> `Verified` vs `CheckedNumerically` with `"to O(x^n)"` flag, two-run value -> `CheckedNumerically`, exact vs numeric `independent_recompute` -> `Verified` vs `CheckedNumerically`, inequality `counterexample_search` -> `Unchecked`).
- **Updated `"verified"` wording rules (R22):** added `wording::BADGE_VERIFIED`, `wording::OWN_CHECK_VERIFIED`, `wording::REASON_SUPERSEDED`, `wording::NON_VERIFIED_CONSTANTS`, and `StepBadge.own_check_line` (`Some("own check: verified")` strictly when `own == Tier::Verified && effective != Tier::Verified`). Updated `Tier::lattice_rank` (`Given`/`Cited` = 4) and superseded-dep `rests_on_line` formatting (`"(superseded)"`) so no non-Verified badge or other string contains `"verified"`.
- **Added OCR Cited support & R24 lessons invariance:** added `EvidenceQuoteRecord.ocr`, `DisplayGrade::MatchedQuoteScannedPage`, `wording::FLAG_OCR_SOURCE`, `wording::TAG_OCR`, `event_kinds::LESSONS_SUPPLIED`, and tests verifying OCR Cited badges and that `lessons.supplied` never affects tiers.

### Cross-brief integration points

| Place | What another brief / later package delivers | Integration action on merge |
|---|---|---|
| `crates/research-tools/src/lib.rs` (lines 21, 27–32) | Small edit adding `pub mod` declarations for S2 (`proof_closure`, `solve_model`, `solve_state`, `solve_status`) and 3 private helper modules. Isolated in its own commit (`221ff62`). | Resolve adjacent `pub mod` insertions from F3, F4/F9, V1 alphabetically. |
| `crates/research-tools/src/solve_model.rs` (`QuoteRef`, `EvidenceQuoteRecord`, `event_kinds::EVIDENCE_QUOTE`, `CONTEXT_SUPPLIED`) | Brief `f4-f9-quote-budget` delivers `crates/research-tools/src/quote_core.rs` with `QuoteRef` (`source_id`, `path`, `start_line: usize`, `end_line: usize`, `file_sha256`, `quote_sha256`, `evidence_seq: Option<u64>`) and stored-quote payload types. | Replace S2's local `QuoteRef` struct with `pub use crate::quote_core::QuoteRef;` and align `EvidenceQuoteRecord.ocr` with F4/F14's exact scanned-page field name. |
| `crates/research-tools/src/solve_model_records.rs` (`PauseReason`, `SolveDoneStatus`, `SolveRunStatus`) | Brief `f4-f9-quote-budget` delivers `crates/research-tools/src/run_budget.rs` with the shared 7-state on-screen `RunState` enum (`00-overview.md` 3.3). | Implement `From<SolveRunStatus> for RunState` (or map via `run_budget`'s total mapping function). |
| `crates/research-tools/src/solve_model_records.rs` (`ReviewIndependence`) | Package S14 delivers the shared host `independence` computation function (moved from F12 in the 2026-11-25 revision). | Re-export or map `ReviewIndependence` (`different_provider`, `different_model`, `same_model`) to S14's shared enum. |
| `crates/research-tools/src/solve_own_tier.rs` (`is_exact_comparison`) | Packages S9 (`solve_checks.rs` / `algebra.py`) and S20 (`delegate_blind.rs`). | Ensure S9 (`series_match`, `vanishes_to_order`) and S20 (`independent_recompute`) emit `"exact": true` (or `"comparison": "exact"`) in `solve.check.evidence` for exact symbolic comparisons, and emit `"numeric_coefficients": true` / `"exact": false` for numeric ones. |
| `crates/research-tools/src/proof_dag.rs` | Not edited by S2. | If S8/S14 later wants `ProofEntry` itself to carry Solve optional fields, note that `crates/agent/src/guardrails.rs:227` constructs `ProofEntry` literally and must be updated together. |

---

## 5. Verification, test commands, and results

Every command was run on `work/s2-solve-model` after the 2026-11-25 brief-revision code commit (`6871878`):

| Command | Result | Exact counts / notes |
|---|---|---|
| `python3 scripts/check_file_length.py --base origin/claude/project-thread-caszr2` | **PASS** (exit 0) | `No touched source file is over 600 lines.` (max touched file: `solve_status_tests.rs` at 553 lines) |
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | **PASS** (exit 0) | `No touched source file is over 600 lines.` |
| `cargo test -p research-tools --test tool_def_golden` | **PASS** (exit 0) | 3 passed, 0 failed, 0 ignored; `crates/research-tools/tests/golden/tool_defs.txt` unchanged |
| `cargo test -p research-tools --lib` | **PASS** (exit 0) | 77 passed, 0 failed, 0 ignored (27 new S2 unit + property tests, 50 existing unit tests) |
| `cargo test --workspace --no-fail-fast` | **PASS** (exit 0) | **666 passed, 0 failed, 2 ignored** across all workspace crates |
| `RE_SEARCH_PYTHON="$(command -v python3)" cargo test -p export --no-fail-fast` | **PASS** (exit 0) | 8 passed, 0 failed, 0 ignored (including `run_all_checks.py` Python replay checks) |
| `cargo clippy -p research-tools --lib --profile test -- -D warnings` | **PASS** (exit 0) | 0 warnings across `research-tools` library and all S2 unit/property tests |
| `cargo clippy -p research-tools --test tool_def_golden -- -D warnings` | **PASS** (exit 0) | 0 warnings |
| `cargo clippy --workspace --all-targets --keep-going -- -D warnings` | **PRE-EXISTING FAILURES ONLY** | 0 diagnostics in any S2 file; 6 pre-existing diagnostics in untouched files on `rustc 1.100.0` / Linux (listed below) |

### Breakdown of new S2 tests (27 tests, all passing)

- `solve_model::tests` (7): handle validation/serde, `e-N` step-id validation, exact 9-type parity with `ledger_append_def()`, `SolveSetup`/`NodeRef` roundtrip, `QuoteRef` `start_line`/`end_line` field names, claim/entry hash separation, `PauseReason`/`SolveDoneStatus` roundtrip.
- `proof_closure::tests` (5): declared+derived closure, `ProofEntry` compatibility, `all_deps` deduplication order, out-of-order topological sort + cycle detection, `compute_derived_deps`.
- `solve_state::tests` (4): incremental fold invariance across every prefix split `k in 0..=len` (including `lessons.supplied`), `pending_runs` and `discard:` note handling, `gate_bounces` increment/reset, conservative counterexample review rejection.
- `solve_status::tests` (8): wording constant `"verified"` exclusivity & explicit 0..=4 tier order, R4 handle re-save & same-claim re-record, R4 two-claim mixed symbolic+numeric & setup staleness, R8 weak checks / failed numeric guard / exact vs numeric `independent_recompute` / two-run values / non-portable handles, R9 derived dependency lowering + `"own check: verified"` line, permanent fail + `"Broken: rests on failed e-6"` + supersede-induced Stale, neutral Given / Cited / OCR Cited + earlier `lean_check` receipt (D19) + R24 `lessons.supplied` invariance, worked example exact vs numeric series + transcription/`ran` flags.
- `solve_status::prop_tests` (3): R7 & R22 over 200 deterministic random DAGs, R4 over 60 randomized multi-claim/setup scenarios, R8 over 60 randomized weak/under-threshold/numeric check scenarios.

### Pre-existing failures / diagnostics (proved on base commit `49edc18`)

All 6 diagnostics from `cargo clippy --workspace --all-targets --keep-going -- -D warnings` occur in files untouched by S2:

1. `crates/desktop-server/src/evidence.rs:744` — `clippy::manual_unwrap_or` (observed on base commit `49edc18` before any edits, documented in `RULES.md`).
2. `crates/research-tools/tests/sandbox_and_fs.rs:6,9` — `unused_imports` (`is_symlink_or_junction`, `TEX_AUDIT`) on Linux because all call sites (lines 243, 265, 337) live inside `#[cfg(windows)]` blocks (verified identical on base commit via `git show`).
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

1. `crates/desktop-server/src/evidence.rs:744` — pre-existing Rust 1.97+/1.100 `clippy::manual_unwrap_or` error (known on base commit; file is over 600 lines and not in S2's scope). Clippy's suggestion (`unwrap_or`) would alter behavior for unexpected `Some(other)` status strings, so this is a clippy false positive.
2. `crates/research-tools/tests/sandbox_and_fs.rs:6,9` — pre-existing Linux-only `unused_imports` warning (`is_symlink_or_junction` and `TEX_AUDIT`) because all call sites (lines 243, 265, 337) are gated behind `#[cfg(windows)]`. Left untouched per RULES.md.
3. `crates/cli/tests/desktop_and_ext.rs:149-168` — when `RE_SEARCH_PYTHON` is exported in the environment on Linux, `re-search run --route api` rejects non-Windows `--python`/`RE_SEARCH_PYTHON` and exits immediately before contacting `ScriptedMock`. Line 168 then calls `mock.finish()` before asserting `out.status`, deadlocking on `listener.accept()` waiting for 3 HTTP requests that will never arrive. Left untouched as outside S2's scope.

---

## 8. Contradictions or ambiguities found in the design

1. **`proof_dag.rs` vs `guardrails.rs`:** `solve.md` 7.2 proposes adding `#[serde(default)]` fields to `ProofEntry` in `proof_dag.rs`, which breaks struct-literal construction in `crates/agent/src/guardrails.rs:227` (outside S2) and risks altering `ledger_query` serialization. Resolved cleanly by keeping `ProofEntry` unchanged and introducing `SolveEntry` + `proof_closure::DepNode`.
2. **Rule 4 vs Rule 5 precedence in `solve.md` 4.4:** Listing `Given`/`Cited`/`Assumed by model` after `Model agreed` would let a model review downgrade neutral `Given`/`Cited` steps or bypass D10 on solver assumptions. Resolved by evaluating `Given`/`Cited`/`AssumedByModel` right after `Failed`.
3. **Exact evidence key distinguishing exact vs numeric series and `independent_recompute` checks:** `solve.md` 4.4 distinguishes exact vs numeric coefficient comparison on `series_match`/`vanishes_to_order` and exact vs numeric comparison on `independent_recompute`, without naming the JSON field in `solve.check.evidence`. Resolved fail-closed via `evidence.exact == true` / `evidence.comparison == "exact"` (documented under Merge notes for S9/S20).
4. **Exact OCR field name on `evidence.quote`:** `00-overview.md` 2.10 and `solve.md` 4.4 specify the `"OCR source"` flag and `"OCR"` chip tag for quotes from scanned pages, without naming the payload field on `evidence.quote`. Resolved by accepting `ocr: true`, `scanned: true`, or `source_kind: "ocr"` (documented under Merge notes for F4/F14).
5. **Step id `e-8b` in worked example (`solve.md` line 131):** `00-overview.md` 2.3 and `s2-solve-model.md` define step ids as `e-N`, while the worked example prose uses `e-8b`. Resolved by validating `e-N` strictly in `validate_step_id` while accepting any non-empty `entry_id` in `SolveState::apply`.
6. **Exact types of `SolveTarget.order` and `SolveSetup.dimensions` powers:** Unspecified in `solve.md` 4.1. Typed as `serde_json::Value` to remain lossless across integers, floats, rational strings, and structured order objects.
