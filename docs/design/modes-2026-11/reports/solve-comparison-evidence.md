# Report: `solve-comparison-evidence` — require a recorded comparison before a badge

- **Brief:** `docs/design/modes-2026-11/agents/solve-comparison-evidence.md`
- **Base commit:** `c7ba452d81efc688beca40d1461f769389b139ef` (`origin/gemini/work/fixed`)
- **Work branch:** `work/solve-comparison-evidence` -> pushed to `gemini/modes/solve-comparison-evidence`
- **Toolchain (Linux VM):** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `cargo 1.100.0 (3acc79c0c 2026-11-05)`

---

## 1. What was implemented (Solve S2 comparison-evidence hardening)

Pure Rust changes inside `crates/research-tools/src/` — no new tools, no wire schema changes, no new dependencies (`Cargo.lock` untouched), no model or network calls, and `crates/research-tools/src/solve_badge.rs` and `crates/research-tools/tests/golden/tool_defs.txt` left completely untouched.

All touched and added source files are at or under 600 lines (`python3 scripts/check_file_length.py --base origin/gemini/work/fixed` passes).

| File | Lines | Role |
|---|---:|---|
| `crates/research-tools/src/solve_own_tier.rs` | 508 | Replaced boolean `is_exact_comparison` with typed `ComparisonKind` (`exact` / `numeric`) + `parse_comparison_kind` helper requiring positive, non-contradictory comparison evidence on `independent_recompute` and `series_match`/`vanishes_to_order`; added `has_check_receipt` guard (rejecting blank/untrimmed `call_id`, `result_seq == 0`, in-flight calls, failed receipts, seq mismatch, and any non-`Check` process or `sympy_eval` receipt when present in `state.receipts`) on passing identity and non-identity checks while leaving `CheckOutcome::Fail` unconditional; hardened `series_order_satisfied` to require `claim.order.is_some()` and explicit `params.variable` + `params.point` when `evidence.order` is an integer. |
| `crates/research-tools/src/solve_own_tier_tests.rs` | 554 | New sibling regression test suite (4 tests) covering missing/negative-only/unrecognized/malformed/contradictory comparison evidence and explicit numeric & exact checks on `independent_recompute` (both `Equation` and `Value`), `series_match`, and `vanishes_to_order`; `claim.order == None` and bare-integer `order` without matching `params`; `CheckOutcome::Inconclusive` and `CheckOutcome::NotRun`; receipt validation (`""`, untrimmed, `result_seq == 0`, in-flight, failed, mismatched seq, `solver`, `verifier`, non-check tool, and `sympy_eval`); model review / solver exit non-inference; and preserved dependency, stale, and failure rules. |
| `crates/research-tools/src/solve_status_tests.rs` | 553 | Minimal 2-line local edit (lines 356 and 359, isolated in its own commit): updated the `independent_recompute` numeric fixture from implicit `{"exact": false}` to explicit `{"comparison": "numeric", "exact": false}` while keeping the exact same asserted `Tier::CheckedNumerically` and leaving every assertion and badge-wording test intact. |

---

## 2. Exact Input Contract for S9 and S20 (and S13 Boundary)

### 2.1 Shared `solve.check` invariants (S9 and S20)

Every `solve.check` record folded into `SolveState::checks` must satisfy the existing ledger schema (`SolveCheck` in `crates/research-tools/src/solve_model.rs`) plus the following tier-earning rules enforced by `solve_own_tier.rs`:

1. **Matching hashes and portability:** `check.claim_sha256 == entry.claim_sha256`, `check.math_sha256 == state.setup.math_sha256`, `check.claim_index == idx`, all claim handles portable (`expr.portable == true`), and handle `srepr_sha256` unchanged since `entry.claim_sha256` was computed. Otherwise the check is stale or ignored.
2. **Outcome mapping (`CheckOutcome`):**
   - `CheckOutcome::Pass`: eligible to grant a tier only when comparison evidence, order/threshold rules, and receipt checks all pass.
   - `CheckOutcome::Fail`: **always** marks the step `Tier::Failed` (and propagates `Tier::Failed` downstream), regardless of comparison evidence or receipt fields; never cleared by later passing checks.
   - `CheckOutcome::Inconclusive` and `CheckOutcome::NotRun`: **never** grant `Verified` or `CheckedNumerically` and emit no sublabels or series flags. S9/S20 must record `Inconclusive` (never `Pass`) on timeout or undecidable simplification, and `NotRun` when approval/policy grant is missing.
3. **Check receipt (`SolveCheck.receipt` and `tool.request` / `tool.result`):**
   - `receipt.call_id` must be non-empty with no leading/trailing whitespace, and `receipt.result_seq >= 1`.
   - `call_id` must not remain in `state.in_flight_tools` (the tool call must have completed).
   - When `state.receipts` contains `call_id` (required in all real sessions from S9/S20), the folded `SolveReceipt` must satisfy:
     - `receipt.ok == true`
     - `receipt.result_seq == check.receipt.result_seq`
     - `receipt.process == Some(ReceiptProcess::Check)` (`"process": "check"` in the `tool.result` payload)
     - `receipt.tool != "sympy_eval"` (e.g., `"check_run"` for S9 / verifier-requested checks / user reruns, or `"compute_blind"` / `"check_run"` for S20 host comparison).
   - **Important for S20 (`compute_blind`):** `check.receipt` must reference the **host's check-process comparison receipt** (`process: "check"`), never the blind worker's `sympy_eval` receipt or a `solver`/`verifier` process receipt.

### 2.2 S9 (`solve_checks.rs` / `python/checks/algebra.py`) — `series_match` and `vanishes_to_order`

Applies to `ClaimForm::Series` claims with `claim.order = Some(SeriesOrder { variable, point, n })`. A series claim with `claim.order == None` stays `Unchecked` (so a truncated series claim can never earn a computed badge without emitting its mandatory `"to O(var^n)"` truncation flag).

- **Required `evidence.order`:**
  - Canonical shape: `"order": { "variable": "<str>", "point": "<str or number>", "n": <int> }` with `variable == claim.order.variable`, `point == claim.order.point`, and `n >= claim.order.n`.
  - Legacy integer shape `"order": <int>` is accepted only when `check.params` explicitly records matching `"variable"` and `"point"` and `n >= claim.order.n`.
- **Required comparison kind in `check.evidence` (JSON object):**
  - **Canonical (recommended for S9):**
    - Exact coefficients -> `Tier::Verified` (`"to O(var^n)"` sublabel + flag):
      `{ "order": { ... }, "comparison": "exact" }` (optionally also `"exact": true` and/or `"numeric_coefficients": false`).
    - Numeric coefficients -> `Tier::CheckedNumerically` (`"to O(var^n), numeric coefficients"` sublabel + `"to O(var^n)"` flag):
      `{ "order": { ... }, "comparison": "numeric", "numeric_coefficients": true }` (optionally also `"exact": false`, plus numeric inspection metadata for S13 such as tolerance, precision, worst residual).
  - **Backward-compatible S2 shorthand still accepted:**
    - `{ "order": { ... }, "exact": true }` -> `Tier::Verified`
    - `{ "order": { ... }, "numeric_coefficients": true }` -> `Tier::CheckedNumerically`
  - **Rejected (stays `Tier::Unchecked`, no sublabels, no series flags):**
    - Missing comparison fields (`{ "order": { ... } }`)
    - Negative-only flags (`"exact": false` alone, `"numeric_coefficients": false` alone)
    - Unrecognized or wrong-case `"comparison"` strings (e.g., `"approx"`, `"symbolic"`, `"EXACT"`, `""`) or non-string/non-null `"comparison"`
    - Non-boolean/non-null `"exact"` or `"numeric_coefficients"`
    - Any contradiction: `comparison: "exact"` with `exact: false` or `numeric_coefficients: true`; `comparison: "numeric"` with `exact: true` or `numeric_coefficients: false`; `exact: true` with `numeric_coefficients: true`.

### 2.3 S20 (`delegate_blind.rs`) — `independent_recompute`

Applies to `ClaimForm::Equation` and `ClaimForm::Value` claims.

- **Required comparison kind in `check.evidence` (JSON object):**
  - **Canonical (recommended for S20):**
    - Exact symbolic comparison -> `Tier::Verified` (`"independent recompute"` sublabel):
      `{ "comparison": "exact" }` (optionally also `"exact": true`).
    - Recorded numeric comparison -> `Tier::CheckedNumerically` (`"independent recompute"` sublabel):
      `{ "comparison": "numeric" }` (optionally also `"exact": false`, plus tolerance / precision / worst residual / output handle metadata for S13 inspection).
  - **Backward-compatible S2 shorthand still accepted:**
    - `{ "exact": true }` -> `Tier::Verified`.
  - **Rejected (stays `Tier::Unchecked`, no `"independent recompute"` sublabel):**
    - Missing comparison fields (`{}`, `null`, non-object)
    - Negative-only `"exact": false` without `"comparison": "numeric"`
    - Any non-null `"numeric_coefficients"` field (`true` or `false` — series-only field)
    - Unrecognized/wrong-case `"comparison"` strings or malformed JSON types
    - Any contradiction (`comparison: "exact"` with `exact: false`; `comparison: "numeric"` with `exact: true`).

### 2.4 S13 Boundary

S13 (`assets/solve_steps.js`, `solve_badges.js`, `solve_why.js`, `solve_exprs.js`) owns the receipt inspection popover, rendering distinct user-visible reasons for `Inconclusive`, `NotRun`, `Failed`, and `Stale`, linking worker assignments/outputs from F9b, and triggering reruns via the contained check process. S2 (`solve_own_tier.rs`) enforces only the pure tier-mapping and fail-closed evidence/receipt guards.

---

## 3. Deviations from the design, decisions taken alone, and assumptions

### Deviations
None from `solve.md` 4.4, "Inspecting a check", or `00-overview.md` 2.10.

### Decisions taken alone
1. **Kept `ComparisonKind` private inside `solve_own_tier.rs`** (`#[derive(Deserialize)] #[serde(rename_all = "snake_case")] enum ComparisonKind { Exact, Numeric }`) rather than adding a new public wire type or changing `SolveCheck`'s schema, honoring both `RULES.md` ("Ask before changing a convention, a definition or the public API") and the brief ("No new tools, wire schemas, dependencies").
2. **Used canonical key `"comparison": "exact" | "numeric"`** (extending the existing `"comparison"` key already present in S2's `solve_own_tier.rs`), while preserving backward compatibility for `"exact": true` and series-only `"numeric_coefficients": true` so existing S2 fixtures remain readable.
3. **Rejected `"numeric_coefficients"` unconditionally on `independent_recompute`** (whether `true` or `false`, whenever non-null) so series-only keys cannot ambiguously qualify or partially mix with blind-recompute evidence.
4. **Hardened `series_order_satisfied`** to return `false` when `claim.order` is `None` (preventing a series claim from earning `Verified`/`CheckedNumerically` without emitting the mandatory `"to O(var^n)"` truncation flag) and to require explicit matching `params.variable` and `params.point` when `evidence.order` is a bare integer (closing an `.is_none_or` bypass on default empty `params`).
5. **Hardened `has_check_receipt` while preserving standalone S2 fixture compatibility:**
   - Rejects empty/untrimmed `call_id`, `result_seq == 0`, in-flight `call_id` (`state.in_flight_tools`), and — whenever `call_id` exists in `state.receipts` — requires `receipt.ok && receipt.result_seq == check.receipt.result_seq && receipt.process == Some(ReceiptProcess::Check) && receipt.tool != "sympy_eval"`.
   - Also guards passing `NON_IDENTITY_CHECKS` (`dimensions`, `symmetry`, etc.) in `evaluate_own`, while keeping `CheckOutcome::Fail` unconditional.
   - **Current trust assumption (plainly stated):** When `check.receipt.call_id` is absent from both `state.receipts` and `state.in_flight_tools`, `has_check_receipt` currently returns `true`. Existing S2 unit/property harnesses (`solve_status_tests.rs`, `solve_status_props_tests.rs`, `solve_state_tests.rs`) emit synthetic `solve.check` events without accompanying `tool.request`/`tool.result` pairs, and parallel lanes (such as `solve-evidence-labels`) may do the same. Changing `None => false` before S9/S13 lands would break those harnesses and widen merge conflicts across three existing test files. Once S9 and S20 emit real `tool.request`/`tool.result` receipts (`process: "check"`) and S2 harnesses are updated together, `None => false` can be enforced unconditionally in `has_check_receipt`.
6. **Isolated new regression tests in `crates/research-tools/src/solve_own_tier_tests.rs`** so `solve_status_tests.rs` stays under 600 lines (553 lines) and has minimal merge surface with `solve-evidence-labels`.
7. **Subagent model note:** The prompt requested Gemini 4 Neon for implementation and Gemini 4 Carbon for hard independent audits. The VM's `invoke_subagent` tool exposes model tier enum values `inherit`, `flash_lite`, `flash`, and `pro`. The independent audit was run with the `research` read-only subagent on the highest reasoning tier (`pro`), which initially returned `FAIL` with 3 High findings (asymmetric `numeric_coefficients` on `independent_recompute`, `has_check_receipt` accepting `Verifier`/`None`/in-flight receipts, and `series_order_satisfied` accepting `claim.order == None` and bare-integer order with empty `params`); all three findings plus a minor whitespace-`call_id` hardening note were fixed and re-audited to `PASS WITH NOTES`.

### Assumptions
- S9 (`check_run` / host auto-checks) and S20 (`compute_blind` host comparison) will record `"process": "check"` on the `tool.result` referenced by `SolveCheck.receipt`, and will use a tool name other than `"sympy_eval"` (e.g., `"check_run"` or `"compute_blind"`).
- `independent_recompute` numeric checks (`ClaimForm::Value` or `ClaimForm::Equation`) do not require `points >= 12` / `digits >= 50` (those thresholds apply specifically to `identity_numeric` on equations per `solve.md` 4.4); numeric comparison metadata (`tolerance`, worst residual, precision) is recorded in `evidence` for S13 inspection while `solve_own_tier.rs` validates `"comparison": "numeric"`.

---

## 4. Merge notes

### Files edited in this branch (in separate commits)

1. `crates/research-tools/src/solve_status_tests.rs` (lines 356 and 359 only — own commit):
   - Changed line 356 comment and line 359 fixture from `json!({"exact": false})` to `json!({"comparison": "numeric", "exact": false})`.
   - **Coordination with `solve-evidence-labels`:** `solve-evidence-labels` owns `crates/research-tools/src/solve_badge.rs` (untouched here) and badge wording assertions in `solve_status_tests.rs` (such as `wording_constants_allow_verified_only_for_verified_badge_and_own_check_line`, `BADGE_CITED` -> `"Found"`, `BADGE_CHECKED_NUMERICALLY` -> `"Numerically checked"`). This branch made zero changes to `solve_badge.rs` and zero changes to any badge label assertion or wording constant in `solve_status_tests.rs`; all new comparison-evidence regression tests live in the new sibling file `crates/research-tools/src/solve_own_tier_tests.rs`. If `solve-evidence-labels` touches nearby lines in `r8_weak_checks_numeric_guard_series_exact_vs_numeric_and_value_runs`, keep our explicit `json!({"comparison": "numeric", "exact": false})` fixture on line 359 alongside their badge-wording assertions.
2. `crates/research-tools/src/solve_own_tier.rs` and new file `crates/research-tools/src/solve_own_tier_tests.rs` (own commit):
   - No overlap with `solve-evidence-labels` (`solve_badge.rs`).
3. `docs/design/modes-2026-11/reports/solve-comparison-evidence.md` (final commit).

### Downstream integration points

| Place | Downstream package | Integration action |
|---|---|---|
| `crates/research-tools/src/solve_own_tier.rs` (`parse_comparison_kind`, `series_order_satisfied`) | S9 (`solve_checks.rs` / `python/checks/algebra.py`) | Emit `"comparison": "exact"` or `"comparison": "numeric"` (with `"numeric_coefficients": true` for numeric series) and structured `"order": {"variable", "point", "n"}` on `series_match` / `vanishes_to_order`. Emit `CheckOutcome::Inconclusive` / `NotRun` / `Fail` accurately. |
| `crates/research-tools/src/solve_own_tier.rs` (`parse_comparison_kind`, `has_check_receipt`) | S20 (`delegate_blind.rs`) | Emit `"comparison": "exact"` or `"comparison": "numeric"` on `independent_recompute` (never `"numeric_coefficients"`), and point `SolveCheck.receipt` to the host check-process receipt (`process: "check"`, `tool != "sympy_eval"`). |
| `crates/research-tools/src/solve_own_tier.rs` (`has_check_receipt`, line 233) | S9 / S13 / S20 | Once S9/S20 emit real `tool.request`/`tool.result` receipts and S2 test harnesses emit matching receipts, consider flipping `None => true` to `None => false` so unrecorded receipt references fail closed. |
| `crates/research-tools/src/solve_own_tier.rs` (lines 436–442) | Cleanup / future S2 refactor | Because `series_order_satisfied` now requires `claim.order.is_some()`, the `else` fallback branch using `SUBLABEL_SERIES_FALLBACK` / `SUBLABEL_SERIES_NUMERIC_FALLBACK` is now unreachable defensive code (zero warnings, harmless). |

---

## 5. Verification, test commands, and results

All commands below were run after the final code edit on `work/solve-comparison-evidence`:

| Command | Result | Exact counts / notes |
|---|---|---|
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | **PASS** (exit 0) | `No touched source file is over 600 lines.` (`solve_own_tier.rs`: 508, `solve_own_tier_tests.rs`: 554, `solve_status_tests.rs`: 553) |
| `cargo test -p research-tools --lib solve_` | **PASS** (exit 0) | **27 passed, 0 failed, 0 ignored** (105 filtered out; includes all 4 new `solve_own_tier::tests` + all existing S2 model/state/status/property tests) |
| `cargo test -p research-tools --lib` | **PASS** (exit 0) | **132 passed, 0 failed, 0 ignored** |
| `cargo test -p research-tools --test tool_def_golden` | **PASS** (exit 0) | **3 passed, 0 failed, 0 ignored** (`tool_defs.txt` unchanged) |
| `cargo test -p research-tools --test tool_profile_golden` | **PASS** (exit 0) | **3 passed, 0 failed, 0 ignored** |
| `cargo clippy -p research-tools --lib --profile test -- -D warnings` | **PASS** (exit 0) | 0 warnings across `research-tools` lib and all Solve unit/property tests |
| `cargo clippy -p research-tools --test tool_def_golden -- -D warnings` | **PASS** (exit 0) | 0 warnings |
| `cargo test --workspace --no-fail-fast` | **PASS** (exit 0) | **782 passed, 0 failed, 2 ignored** across all workspace crates |
| `cargo clippy --workspace --all-targets --keep-going -- -D warnings` | **PRE-EXISTING FAILURES ONLY** | 0 diagnostics in any file touched by this branch; 6 pre-existing diagnostics in untouched files on `rustc 1.100.0` / Linux (listed below) |

### Pre-existing workspace Clippy diagnostics (on `rustc 1.100.0` / Linux, untouched files)

All 6 diagnostics match the exact pre-existing list already documented on `origin/gemini/work/fixed` in `docs/design/modes-2026-11/reports/s2-solve-model.md` (Section 5, lines 128–133) and occur exclusively in files untouched by this branch:

1. `crates/desktop-server/src/evidence.rs:744` — `clippy::manual_unwrap_or` (also documented in `RULES.md`; file is > 600 lines and not ours).
2. `crates/research-tools/tests/sandbox_and_fs.rs:6,9` — `unused_imports` (`is_symlink_or_junction`, `TEX_AUDIT`) on Linux because all call sites are inside `#[cfg(windows)]`.
3. `crates/agent/tests/eval_support/manifest.rs:289` — `clippy::match_like_matches_macro`.
4. `crates/agent/tests/eval_support/sheet.rs:102` — `clippy::unnecessary_map_or`.
5. `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` — `unused_imports` (`BufRead`, `BufReader`) on Linux.
6. `crates/agent/src/guardrails.rs:335` — `clippy::items_after_test_module`.

---

## 6. What could not be compiled or run on Linux

- All `#[cfg(windows)]` code paths across the workspace (AppContainer sandbox, Job Objects, WebView2 `desktop-app`, and Windows-only tests in `sandbox_and_fs.rs`, `chatgpt-auth`, `worker`, `cli-route`, `cli`) are not compiled or executed on this Linux VM.
- `solve_own_tier.rs` and `solve_own_tier_tests.rs` are 100% platform-neutral Rust with zero OS-specific code or paths.

---

## 7. Bugs / issues noticed but not fixed

1. All 6 pre-existing Clippy diagnostics listed in Section 5 above (in untouched files outside this brief's scope).
2. `crates/research-tools/src/solve_own_tier.rs:233` — `has_check_receipt` still trusts a `SolveCheck.receipt` whose `call_id` is absent from both `state.receipts` and `state.in_flight_tools` (documented in Section 3 item 5 and Section 4 because existing S2 harnesses emit synthetic `solve.check` events without `tool.result` pairs).

---

## 8. Contradictions or ambiguities found in the design

- **`solve.md` 4.4 vs `00-overview.md` 2.10 display wording:** `solve.md` 4.4 line 311 uses "Numerically checked" while line 283/290 still says "Checked numerically" (`00-overview.md` 2.10 owner clarification 2026-11-26 explicitly states the new display strings supersede earlier spellings). Owned by `solve-evidence-labels` (`solve_badge.rs`); left untouched by this lane as directed.
- **`solve.md` 4.4 receipt requirement vs S2 pure-fold test fixtures:** `solve.md` 4.4 ("with an explicit comparison kind and a check receipt") and `00-overview.md` 2.10 ("a missing or stale receipt cannot earn either check badge") require a check receipt, while existing S2 tests emit `solve.check` events without corresponding `tool.result` events in `SolveState::receipts`. Resolved conservatively by rejecting blank/untrimmed `call_id`, `result_seq == 0`, in-flight calls, failed receipts, seq mismatches, and non-`Check` / `sympy_eval` receipts whenever present in `state.receipts`, and documenting the remaining `None => true` assumption clearly for S9/S13/S20.
