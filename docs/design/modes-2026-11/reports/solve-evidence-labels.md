# Report: `solve-evidence-labels` (Accepted evidence labels and display-grade mapping in Solve S2)

- **Brief:** `docs/design/modes-2026-11/agents/solve-evidence-labels.md`
- **Base commit:** `f68999d6aef1afa780d100a9bc746d6b6372539a` (`origin/gemini/work/fixed`)
- **Work branch:** `work/solve-evidence-labels` -> pushed to `gemini/modes/solve-evidence-labels`
- **Toolchain (Linux VM):** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `cargo 1.100.0`, `Python 3.12`

---

## 1. What was implemented (S2 wording and badge mapping)

Scoped strictly to S2 badge wording, display-grade mapping, and Solve badge tests in `crates/research-tools/`. No tool definitions, tool profiles, resume logic, sandbox rules, F8 fixtures/metrics, or dependencies (`Cargo.toml` / `Cargo.lock`) were changed. All touched and added source files are under the 600-line limit (`python3 scripts/check_file_length.py --base origin/gemini/work/fixed` passes).

| File | Lines | Commit | Role |
|---|---:|---|---|
| `crates/research-tools/src/solve_badge.rs` | 407 | `596e82f` | Updated `wording::BADGE_CITED` from `"Cited"` to `"Found"` and `wording::BADGE_CHECKED_NUMERICALLY` from `"Checked numerically"` to `"Numerically checked"` per the 2026-11-26 owner clarification (`00-overview.md` 2.10), while keeping `BADGE_VERIFIED = "Verified"`, `OWN_CHECK_VERIFIED = "own check: verified"`, serialized `Tier` variants (`cited`, `checked_numerically`, `verified`, etc.), and event formats unchanged. Updated `Tier::display_grade()` so `DisplayGrade::ByCode` is restricted strictly to `Tier::Verified` and `Tier::CheckedNumerically`; moved `Tier::Failed` out of `DisplayGrade::ByCode` into `DisplayGrade::ModelStatement` (keeping `BorderStyle::None`). |
| `crates/research-tools/src/solve_status_tests.rs` | 582 | `596e82f` | Updated wording assertions to `"Numerically checked"` and `"Found"`, added explicit serde stability checks for `Tier` variants (`"verified"`, `"checked_numerically"`, `"cited"`, `"failed"`, `"stale"`), asserted `DisplayGrade::ByCode` exclusivity across all 11 `Tier` variants, and asserted `ModelStatement` / `BorderStyle::None` on `Failed`, `"Broken: rests on failed ..."`, and `Stale` badges and `"Found"` on full-text and OCR `Cited` badges. |
| `crates/research-tools/src/solve_status_props_tests.rs` | 504 | `596e82f` | Strengthened `assert_r22_wording` across all 200 randomized DAGs and 60 R8 property runs to enforce that `DisplayGrade::ByCode` appears iff `effective` is `Verified` or `CheckedNumerically`, `Cited` renders `"Found"` with `MatchedQuote*` grade, `CheckedNumerically` renders `"Numerically checked"`, and `Failed`/`Stale`/`ModelAgreed*`/`AssumedByModel`/`Unchecked` always use `DisplayGrade::ModelStatement` and never solid/check styling. |
| `crates/research-tools/tests/solve_evidence_labels.rs` | 542 | `6f88823` | New public-API integration test suite verifying: (1) distinct labels `"Found"` (full-text + OCR with `"OCR source"` flag & `"OCR"` tag), `"Verified"`, and `"Numerically checked"` with unchanged serialized tiers; (2) citations (matched & unmatched), model opinions (`ModelAgreed`, `ModelAgreedSameModel`, `AssumedByModel`), tool successes without a claim check (`"ran"`, weak checks, under-threshold numeric), stale checks, failed checks (including failed numeric guard after symbolic pass), and user statements (`Given`, `Checked by you`) are never styled as `DisplayGrade::ByCode` or check badges; (3) inheritance (`Verified` over `CheckedNumerically` -> `"Numerically checked"` with `"own check: verified"` and `"rests on e-num (numerically checked)"`, `CheckedNumerically` and OCR `Cited` lowered by `Unchecked`, and `Verified`/`Cited` broken by `Failed`). |

An independent read-only adversarial audit (run via a `pro` subagent) found zero blocking defects and suggested the extra edge-case assertions for `Given`/`CheckedByYou` provenance styling, lowered `CheckedNumerically` (`own_check_line == None`), and lowered OCR `Cited` (retaining `"OCR source"` flag while losing `"Found"` / `MatchedQuoteScannedPage`), all of which were added to `tests/solve_evidence_labels.rs` and verified before commit.

---

## 2. Inspection of `solve.check` -> badge fold path, trust assumptions, and S9/S13 integration points

No production code in `gemini/work/fixed` emits `solve.check` yet (`crates/research-tools/src/solve_checks.rs`, `python/checks/algebra.py`, `solve_receipts.rs`, `solve_view.rs`, and `assets/solve_{badges,why,steps}.js` belong to S4, S8, S9, and S13). The existing pure-fold path in S2 is:

1. **Event model (`crates/research-tools/src/solve_model.rs:408-426`):** `SolveCheck` stores `check_id`, `claim_sha256`, `claim_index`, optional `entry_id`, `math_sha256`, `name`, `registry_sha256`, `params`, `outcome`, `asked_by`, `receipt: ReceiptRef { call_id, result_seq }`, `evidence: Value`, and `evidence_sha256`.
2. **Fold (`crates/research-tools/src/solve_state.rs:155-160, 179-204, 246-248`):** `SolveState::apply` deserializes `solve.check` directly into `self.checks`, folds `tool.request`/`tool.result` into `self.receipts`, and calls `self.recompute_tiers()`.
3. **Own-tier check matching (`crates/research-tools/src/solve_own_tier.rs:180-200, 287-386`):**
   - Matches checks where `check.claim_sha256 == entry.claim_sha256`, `setup.math_sha256 == check.math_sha256`, all present claim handles are `portable == true`, and recomputed `compute_claim_sha256` matches `entry.claim_sha256`.
   - Grants `Tier::Verified` for `identity_symbolic` (`311-314`), exact `series_match`/`vanishes_to_order` (`333-356`, `is_exact_comparison`), or exact `independent_recompute` (`324-332`).
   - Grants `Tier::CheckedNumerically` for qualifying equation `identity_numeric` (`points >= 12`, `digits >= 50`, `domain_checked != Some(false)`, `166-172`), numeric series (`333-356`), two-run distinct-`code_sha256` value `identity_numeric` (`150-164`, `357-362`), or numeric `independent_recompute` (`324-332`).
   - Any matching `CheckOutcome::Fail` overrides to `Tier::Failed` (`230-235`).
4. **Effective propagation and badge rendering (`crates/research-tools/src/solve_status.rs:16-222`, `solve_badge.rs:299-365`):** Propagates minimum rank over `depends_on ∪ derived_depends_on`, marks stale/broken steps, attaches flags, and produces `StepBadge`.

### Current trust assumptions in S2 (pure fold, no I/O)

- **Receipt existence & check-process provenance are NOT cross-checked in S2:** `solve_state::apply` (`solve_state.rs:155-160`) and `solve_own_tier::evaluate_own` (`solve_own_tier.rs:187-200, 306-372`) do **not** verify that `SolveCheck.receipt` (`call_id`, `result_seq`) resolves to an `ok == true` entry in `SolveState.receipts` with `process == Some(ReceiptProcess::Check)`, do **not** recompute/verify `SolveCheck.evidence_sha256`, and do **not** validate `registry_sha256` or `asked_by`. S2 assumes rule R3 (`solve.md` 4.1 & 6): `solve.check` events are written exclusively by host check code (`solve_checks.rs`), never by model tools.
- **Symbolic check guards rely on S9/S6:** `solve_own_tier.rs:311-314` grants `Tier::Verified` solely on `name == "identity_symbolic"` and `outcome == CheckOutcome::Pass`. The three guards on `Verified` (`solve.md` 4.4 rule 3: check-process isolation, numeric branch-cut guard emitting `CheckOutcome::Fail` on disagreement, and storing assumptions in `evidence`) live in S9 (`solve_checks.rs` + `python/checks/algebra.py`) and S6.
- **Numeric evidence defaults:** `equation_numeric_evidence` (`solve_own_tier.rs:166-172`) checks `points >= 12`, `digits >= 50`, and `domain_checked != Some(false)` (missing `domain_checked` defaults to allowed); it does not check tolerance, seed, or residual fields. S9 must emit `domain_checked: false` when domain validation fails/skips, and populate domain, sample points/seed, precision, tolerance, and residuals for inspectable receipts (`00-overview.md` 3.4).
- **Missing `solve.expr` events:** In `handles_status` (`solve_own_tier.rs:88-110`), if a handle referenced by a claim is absent from `state.handles`, `all_present` becomes `false` and `handles_status` returns `(true, false)`, trusting the stored `entry.claim_sha256`. S8 (`step_record`) is the write-time guard preventing claims from naming unknown or non-portable handles.
- **Blob-backed `tool.result` receipts for two-run value checks:** `parse_tool_result` (`solve_state.rs:424`) leaves `code_sha256: None` when a `tool.result` payload is stored in a blob (S2 has no I/O). S7/S9 must either keep `code_sha256` inline in `tool.result` or emit `lhs_code_sha256` / `rhs_code_sha256` in `solve.check.evidence` (`solve_own_tier.rs:161-163`).

### Exact integration points left to S9, S4/S8, and S13

- **S9 (`crates/research-tools/src/solve_checks.rs`, `python/checks/algebra.py`):** Sole authorized writer of `solve.check`. Must execute checks in the isolated check process (`ReceiptProcess::Check`), emit paired `tool.request`/`tool.result` receipts with `code_sha256`/`process`/`env`, populate `SolveCheck.receipt`, `registry_sha256`, inspectable `evidence` (symbolic: expressions, assumptions, domain, engine versions, result; numeric: domain, sample points/seed, precision, tolerance, residuals/counterexample; exact-vs-numeric flags `exact`/`numeric_coefficients` for series & `independent_recompute`), compute `evidence_sha256`, and run the numeric branch-cut guard. Once S9 lands and emits paired check receipts, S2's `solve_own_tier::evaluate_own` (`solve_own_tier.rs:187-200`) is the exact hook if the owner wants the pure fold itself to reject orphan/missing-receipt `solve.check` events (doing so will also require updating S2's synthetic test harnesses in `solve_status_tests.rs:151-180`, `solve_status_props_tests.rs:103-132`, and `tests/solve_evidence_labels.rs:167-196` to emit matching check-process `tool.request`/`tool.result` events).
- **S4 / S8 (`crates/research-tools/src/solve_receipts.rs`, `crates/desktop-server/src/solve_view.rs`):** `StepBadge` (`solve_badge.rs:267-287`) intentionally carries display status only, not receipt payloads or blob bytes. S4/S8 must join each `StepBadge` with its matching `SolveCheck` records, `SolveReceipt` entries (and blob resolution, flagging any missing receipt/blob visibly as missing per `00-overview.md` 3.4), `SolveExpr` records, and `EvidenceQuoteRecord` (for `"Found"` steps).
- **S13 (`crates/desktop-server/assets/solve_badges.js`, `solve_why.js`, `solve_steps.js`) + F6 (`ui_badge.js`):** Must wire clicking `"Found"` to open the source + matched passage (or scanned page image for OCR) and clicking `"Verified"` / `"Numerically checked"` to open the Why popover with the inspectable computation, inputs, and output without an extra model call.

---

## 3. Deviations from the design and why

None within S2. Out-of-scope files (`crates/desktop-server/assets/ui_badge.js` in F6, `crates/agent/tests/eval_support/measures.rs` in F8, and future S9/S13 packages) were intentionally left untouched per `solve-evidence-labels.md` and `RULES.md`.

---

## 4. Decisions taken alone and assumptions

1. **Kept existing Rust constant and enum identifiers (`BADGE_CITED`, `BADGE_CHECKED_NUMERICALLY`, `Tier::Cited`, `Tier::CheckedNumerically`):** Only the emitted display strings changed (`"Found"` and `"Numerically checked"`). Serialized `Tier` values (`"cited"`, `"checked_numerically"`, `"verified"`, etc.), event schemas, and public Rust symbol names remain 100% backwards-compatible.
2. **Moved `Tier::Failed` from `DisplayGrade::ByCode` to `DisplayGrade::ModelStatement` (`solve_badge.rs:228-240`):** `DisplayGrade` has the exact 6 rows of `00-overview.md` 2.10 (`ByCode`, `MatchedQuoteFullText`, `MatchedQuoteScannedPage`, `MatchedQuoteAbstract`, `ModelStatement`, `UserStatement`), with no dedicated `Failed`/`Stale` grade. `Tier::Stale` was already mapped to `DisplayGrade::ModelStatement` with `BorderStyle::None`. Moving `Tier::Failed` alongside `Tier::Stale` (keeping `BorderStyle::None`) guarantees `DisplayGrade::ByCode` is emitted strictly for `Tier::Verified` and `Tier::CheckedNumerically` — never for a failed check or a step broken by a failed dependency.
3. **Kept `own_check_line` restricted to `own == Tier::Verified && effective != Tier::Verified`:** Per `solve.md` 3 & R22, `"own check: verified"` is emitted only when a step's own check passed exact symbolic verification while its effective tier was lowered; `own == Tier::CheckedNumerically` when lowered emits `own_check_line: None`.
4. **Left `solve_own_tier.rs` receipt trust assumption intact:** Per the brief ("Limit this independent change to the wording and its checked mapping in S2... If the work required to enforce inspectable receipts lives in S9 or S13, report the exact integration point and the current trust assumption. Leave those packages' implementation to their own lanes"), `solve_own_tier.rs` and `solve_state.rs` were not altered.

---

## 5. Merge notes

### Edited existing files (in commit `596e82f`)

- `crates/research-tools/src/solve_badge.rs` (lines 26-44, 224-240): updated `BADGE_CHECKED_NUMERICALLY`, `BADGE_CITED`, and `Tier::display_grade`.
- `crates/research-tools/src/solve_status_tests.rs` (lines 196-226, 427-487): updated wording/serde/grade assertions.
- `crates/research-tools/src/solve_status_props_tests.rs` (lines 271-311): strengthened `assert_r22_wording` to check accepted labels and `DisplayGrade::ByCode` exclusivity.

### Downstream cross-lane integration points

| Place | What another lane delivers / must align | Integration action |
|---|---|---|
| `crates/desktop-server/assets/ui_badge.js:9` | F6 / S13 shared UI badge constants | Update `SOLVE_NUMERIC_WORD = 'Checked numerically'` -> `'Numerically checked'` to match `solve_badge::wording::BADGE_CHECKED_NUMERICALLY`. |
| `crates/desktop-server/assets/ui_badge.js:99-123, 202-204` | F6 / S13 `GRADE_ALIASES` & border handling | `DisplayGrade` serializes as `"matched_quote_full_text"` and `"matched_quote_scanned_page"`, which are not in `GRADE_ALIASES` today (they would fall back to `'model'`). S4/S13 must either map `DisplayGrade` to `'quote_fulltext'`/`'quote_ocr'` or add those aliases, and handle `BorderStyle::Plain` / `BorderStyle::None` (`ui_badge.js:202-204` currently recognizes only `'solid'` and `'dashed'`). |
| `crates/research-tools/src/quote_core.rs:274-293` vs `solve_state.rs:384-398` | F4 / F14 / S8 `EvidenceQuotePayload` | `EvidenceQuotePayload` has `#[serde(deny_unknown_fields)]` and currently has no `ocr`/`scanned`/`source_kind` field; align when F14/S8 wires OCR quotes into `evidence.quote`. |
| `crates/research-tools/src/solve_checks.rs` (S9) & `solve_own_tier.rs:187-200` | S9 check runner & inspectable receipts | Emit paired `tool.request`/`tool.result` (`ReceiptProcess::Check`) receipts, inspectable `evidence` fields, `evidence_sha256`, and numeric branch-cut guards. Decide whether to enforce `SolveCheck.receipt` presence in `solve_own_tier::evaluate_own` once S9 lands. |
| `crates/agent/tests/eval_support/measures.rs` | Brief `eval-solve-badges` (F8) | Untouched here per brief; `eval-solve-badges` owns F8 Q metrics and should align display wording if referencing the numeric badge name. |

---

## 6. Verification, test commands, and results

All commands were run on `work/solve-evidence-labels` after final code edits (`6f88823`):

| Command | Result | Exact counts / notes |
|---|---|---|
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | **PASS** (exit 0) | `No touched source file is over 600 lines.` (`solve_badge.rs` 407, `solve_status_tests.rs` 582, `solve_status_props_tests.rs` 504, `tests/solve_evidence_labels.rs` 542) |
| `cargo test --locked -p research-tools --lib solve` | **PASS** (exit 0) | 23 passed, 0 failed, 0 ignored (105 filtered out) |
| `cargo test --locked -p research-tools --test solve_evidence_labels --test tool_def_golden --test tool_profile_golden` | **PASS** (exit 0) | 9 passed, 0 failed, 0 ignored (3 + 3 + 3); `tests/golden/tool_defs.txt` and `tests/golden/tool_profiles.txt` unchanged |
| `cargo clippy --locked -p research-tools --lib --test solve_evidence_labels --test tool_def_golden --test tool_profile_golden -- -D warnings` | **PASS** (exit 0) | 0 warnings / 0 errors |
| `cargo test --locked --workspace --no-fail-fast` | **PASS** (exit 0) | **781 passed, 0 failed, 2 ignored** across 106 test binaries |
| `RE_SEARCH_PYTHON="$(command -v python3)" cargo test --locked -p export --no-fail-fast` | **PASS** (exit 0) | **8 passed, 0 failed, 0 ignored** |
| `node --test crates/desktop-server/tests/shared_ui_parts.test.mjs` | **PASS** (exit 0) | 4 passed, 0 failed (verified by independent audit) |
| `cargo clippy --locked --workspace --all-targets --keep-going -- -D warnings` | **PRE-EXISTING FAILURES ONLY** (exit 101) | 0 diagnostics in any touched file; 6 pre-existing diagnostics in untouched files on Rust 1.100.0 / Linux (listed below) |

### Pre-existing clippy diagnostics (untouched files on base commit `f68999d`)

All 6 diagnostics from `cargo clippy --locked --workspace --all-targets --keep-going -- -D warnings` occur in files completely untouched by this branch (`git diff origin/gemini/work/fixed -- <files>` is empty), matching the base-commit diagnostics already recorded in `docs/design/modes-2026-11/reports/s2-solve-model.md`:

1. `crates/desktop-server/src/evidence.rs:744` — `clippy::manual_unwrap_or` (file > 600 lines; documented in `RULES.md`).
2. `crates/research-tools/tests/sandbox_and_fs.rs:6,9` — `unused_imports` (`is_symlink_or_junction`, `TEX_AUDIT`) on Linux because all call sites are `#[cfg(windows)]`.
3. `crates/agent/tests/eval_support/manifest.rs:289` — `clippy::match_like_matches_macro` (Rust 1.100).
4. `crates/agent/tests/eval_support/sheet.rs:102` — `clippy::unnecessary_map_or` (Rust 1.100).
5. `crates/agent/src/guardrails.rs:335` — `clippy::items_after_test_module`.
6. `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` — `unused_imports` (`BufRead`, `BufReader`) on Linux.

---

## 7. What could not be compiled or run on Linux

- All `#[cfg(windows)]` code paths across the workspace (AppContainer sandbox, Job Objects, WebView2 `desktop-app`, and Windows-gated tests in `sandbox_and_fs.rs`, `chatgpt-auth`, `worker`, `cli-route`, `cli/tests/contained_router.rs`) are not compiled or executed on this Linux VM.
- All code changed or added by this brief (`solve_badge.rs` and the Solve badge tests) is 100% platform-neutral Rust with no OS-specific paths or shell-outs.

---

## 8. Bugs / issues noticed but not fixed (out of scope)

1. `crates/desktop-server/assets/ui_badge.js:9` — still exports `SOLVE_NUMERIC_WORD = 'Checked numerically'` instead of `'Numerically checked'`. Left untouched as outside S2 scope (F6/S13).
2. `crates/desktop-server/assets/ui_badge.js:99-123, 202-204` — `GRADE_ALIASES` does not include `DisplayGrade`'s snake_case `"matched_quote_full_text"` / `"matched_quote_scanned_page"`, and `createBadge` only handles `'solid'`/`'dashed'` borders (not `'plain'`/`'none'`). Left untouched as F6/S13 scope.
3. `crates/research-tools/src/quote_core.rs:274-293` — `EvidenceQuotePayload` uses `#[serde(deny_unknown_fields)]` with no OCR/scanned field, while `solve_state.rs:384-398` checks `ocr`/`scanned`/`source_kind`. Left untouched as F4/F14/S8 scope.
4. `crates/research-tools/src/solve_own_tier.rs:88-110` — when `solve.expr` events for claim handles are missing from `state.handles`, `handles_status` returns `(true, false)` and trusts stored `entry.claim_sha256` instead of failing closed. Left untouched (documented under trust assumptions; S8 `step_record` guards write-time creation).
5. `crates/research-tools/src/solve_status.rs:91` & `solve_badge.rs:319` — superseding step `e-1` marks its dependents `Stale` (with `(superseded)` in `rests_on_line`), while `e-1`'s own `StepBadge` keeps its prior effective tier unless callers filter via `SolveState::active_entries()`. Pre-existing S2 behavior; left untouched.
6. The 6 pre-existing clippy diagnostics listed in Section 6.

---

## 9. Contradictions or ambiguities found in the design

1. **Earlier mode-draft spellings vs 2026-11-26 clarification:** `solve.md` 3 & 4.4 and `00-overview.md` 0.1, 3.2 row 20, 8.1, and 8.3 #11 still use `"Checked numerically"` and `"Cited"`, whereas `00-overview.md` 2.10 (Owner clarification, 2026-11-26) specifies `"Numerically checked"` and `"Found"` and explicitly states *"These display strings supersede the earlier spellings in the mode drafts"*. Implemented `"Found"` and `"Numerically checked"` per the 2026-11-26 clarification and brief.
2. **No dedicated `DisplayGrade` variant for `Failed` or `Stale`:** `00-overview.md` 2.10 defines 6 grades (`ByCode`, `MatchedQuoteFullText`, `MatchedQuoteScannedPage`, `MatchedQuoteAbstract`, `ModelStatement`, `UserStatement`) and omits `Failed` and `Stale` from the table, while `solve.md` 3 states `"Failed and Stale have no border style of their own; their text badges say it"`. Resolved cleanly by mapping both `Failed` and `Stale` to `DisplayGrade::ModelStatement` with `BorderStyle::None`, preserving the 6-grade enum and keeping `DisplayGrade::ByCode` exclusive to `Verified` and `CheckedNumerically`.
