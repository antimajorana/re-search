# Report: `eval-solve-badges` (align the F8 Q gate with the accepted Solve tiers)

- **Brief:** `docs/design/modes-2026-11/agents/eval-solve-badges.md`
- **Base commit:** `afe6d70bf73bd7a1b425b71ab05be37f21f88317` (`origin/gemini/work/fixed`)
- **Work / push branch:** `gemini/modes/eval-solve-badges`
- **Toolchain (Linux VM):** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `cargo 1.100.0 (3acc79c0c 2026-11-05)`, `Python 3.12.16`
- **Scope boundaries respected:** Zero production code touched (`crates/*/src/` unchanged); zero tool definition changes (`crates/research-tools/tests/golden/tool_defs.txt` unchanged); zero dependency changes (`Cargo.lock` unchanged, all Cargo commands run with `--locked`); `tests/fixtures/eval/manifest-q.json` and `tests/fixtures/eval/scripts/` untouched; no private papers, credentials, or live network calls.

---

## 1. What was implemented (F8 Q gate alignment)

All touched and added source files are under the 600-line limit (`python3 scripts/check_file_length.py --base origin/gemini/work/fixed` passes). `measures.rs` remains pure `std`-only (verified via standalone `rustc --crate-type lib --edition 2021 -D warnings --emit metadata` against only a minimal `Set` stub).

| File | Lines | Commit | Role |
|---|---:|---|---|
| `crates/agent/tests/eval_support/measures.rs` | 530 | `52f4e08` | Added single-source `solve_wording` module (using a `define_solve_wording!` `concat!` macro keyed to the 2026-11-25 owner decisions so every Q badge word, label, and release-blocker consequence updates from one literal), replaced `planted_error_ends_computed` with two `Rule::MustBeZero` measures (`planted_error_ends_verified`, `planted_error_ends_checked_numerically`), replaced `steps_computed_share` with two `Rule::Recorded` measures (`steps_verified_share`, `steps_checked_numerically_share`), left `planted_error_ends_failed_pct`, `steps_model_agreed_share`, `steps_unchecked_share`, and `final_answer_equals_key` intact, added `RETIRED_MEASURES` / `retired_measure` with fail-closed rejection and migration guidance in `evaluate()`, added `count_in(count, sample_size)` and checked `SolveQCounts::record_into` (zero-case omission and stale-key clearing), hardened `percent(part, whole)` so `Some(100.0)` holds iff `whole > 0 && part == whole`, and enforced unit/domain bounds (`-0.0`, non-finite, share `> 1`, percent `> 100`, fractional `MustBeZero` counts, and Q step-share sum `> 1`). |
| `crates/agent/tests/eval_support/solve_q_tests.rs` | 360 | `d0b2f72` | New sibling test module included by `eval_harness.rs`: cross-checks `solve_wording::BADGE_*` against `research_tools::solve_status::wording::BADGE_*` (S2), asserts no measure in `MEASURES` uses `"Computed"`, tests all Q planted-error pass/fail/incomplete combinations (`Failed`, `Verified`, `Checked numerically`, `Unchecked`, missing keys), tests full and partial zero-case paths (`SolveQCounts::default()`, steps-only, planted-only, stale-key clearing, inconsistent/overflowing counts, `percent` exactness near `2^53`), tests out-of-domain and double-counted step-share rejection in `evaluate()`, and tests fail-closed rejection of saved pre-revision sheets carrying `planted_error_ends_computed` or `steps_computed_share` (both in mixed realistic `values` maps and in `measure_notes`). |
| `crates/agent/tests/eval_harness.rs` | 458 | `d0b2f72` | Wired `#[path = "eval_support/solve_q_tests.rs"] mod solve_q_tests;`, updated `measure_table_is_the_table_of_section_7_3` to expect the 16 pass rules (`planted_error_ends_verified` and `planted_error_ends_checked_numerically` replacing `planted_error_ends_computed`), strengthened `gate_never_passes_what_was_not_measured` with `share(0, 0)` and `count_in(0, 0)`, strengthened `scripted_case_runs_end_to_end_and_fills_a_sheet` to exercise `SolveQCounts::default().record_into` and verify all three planted-error measures remain `NotMeasured` and render as such, and added `!rendered.to_ascii_lowercase().contains("computed")` to `sheet_template_is_the_rendering_of_a_blank_sheet`. No existing assertion was weakened or deleted. |
| `docs/design/eval-2026-11/sheet-template.md` | 75 | `02fcfa0` | Regenerated via the documented command `EVAL_BLESS=1 cargo test --locked -p agent --test eval_harness` (then verified with a clean run without `EVAL_BLESS`). Updated from `38 measures: 15 with a pass rule, 23 recorded only` to `40 measures: 16 with a pass rule, 24 recorded only`; zero occurrences of `Computed` remain. |

### Independent audit pass
Before committing, an independent read-only audit subagent (running on the strongest available model tier, `pro`, mapped for the requested Carbon audit role) reviewed all changes against `RULES.md`, `eval-solve-badges.md`, `00-overview.md`, `solve.md`, and `solve_badge.rs`. Its findings drove four concrete hardening edits included in commits `52f4e08` and `d0b2f72`:
1. Wired `solve_wording::BADGE_*` directly into `LABEL_*` and `CONSEQUENCE_*` via a `concat!` macro instead of duplicating string literals, and used `solve_wording::ID_*` in touched lines of `eval_harness.rs`.
2. Added strict unit/domain validation in `evaluate()` (`-0.0`, share `> 1.0`, percent `> 100.0`, fractional `MustBeZero` counts, and mutually-exclusive Q step shares summing `> 1.0 + 1e-9`).
3. Updated `SolveQCounts::record_into` to clear stale Q keys when re-recording an empty sample (`planted_total == 0` or `steps_total == 0`) while keeping unrelated keys untouched and guaranteeing no mutation on error, and added exactness guards to `percent(part, whole)`.
4. Expanded `solve_q_tests.rs` to test realistic mixed valid+retired saved sheets, stale-key clearing, `percent` boundary exactness, and out-of-domain inputs.

---

## 2. Review of saved F8 sheets and metric key compatibility

- **Repository inventory:** Checked the entire repository (`docs/design/eval-2026-11/` and `tests/`). There are currently **zero** filed run sheets (`sheet-*.md` or saved `.json` sheet inputs) checked into the repository; `docs/design/eval-2026-11/` contains only `README.md` and `sheet-template.md`.
- **Effect on any out-of-repo pre-revision saved F8 JSON sheets:**
  - Any saved JSON sheet carrying `"planted_error_ends_computed"` or `"steps_computed_share"` in `values` is now **rejected fail-closed** by `measures::evaluate`, `sheet::validate_sheet`, `sheet::render_markdown`, and `sheet::write_sheet`. The error message explicitly names the retired key, states that it was retired by the 2026-11-25 Solve tier revision because `Computed` conflated `Verified` and `Checked numerically`, and names the exact replacement keys to re-measure and supply:
    - `"planted_error_ends_computed"` -> re-measure and supply `"planted_error_ends_verified"` and `"planted_error_ends_checked_numerically"`.
    - `"steps_computed_share"` -> re-measure and supply `"steps_verified_share"` and `"steps_checked_numerically_share"`.
  - Any saved JSON sheet carrying either retired key in `measure_notes` is also rejected fail-closed by `sheet::validate_sheet` (and `sheet::write_sheet`).
  - **Historical meaning preserved without fabrication:** Neither retired key is silently aliased, copied, or split into `Verified` or `Checked numerically`. Even `"planted_error_ends_computed": 0` is rejected rather than auto-mapped to zero `Verified` + zero `Checked numerically`, because pre-revision `Computed` predated the S2 exact-vs-numeric tier semantics (and originally sat below a separate Lean tier). Any previously filed Markdown sheet (`.md`) remains an unchanged historical record of its pre-revision run; any JSON input re-evaluated or re-filed under the revised gate must be re-measured under the accepted S2 tier rules and updated to remove the two retired keys from `values` and `measure_notes` and supply the four new keys.

---

## 3. Deviations from the design and why

1. **Test module split (`eval_support/solve_q_tests.rs`):** Adding all focused Q tier, zero-case, domain, and retired-key tests directly inline in `crates/agent/tests/eval_harness.rs` (446 lines on base) pushed `eval_harness.rs` to 601 lines, exceeding the 600-line limit in `RULES.md`. Placing the new focused test module in `crates/agent/tests/eval_support/solve_q_tests.rs` and including it via `#[path = "eval_support/solve_q_tests.rs"] mod solve_q_tests;` in `eval_harness.rs` keeps every source file well under 600 lines, keeps `eval_harness.rs` edits small and merge-friendly for `eval-q-cases` / `eval-p-cases`, and runs every test under `cargo test --locked -p agent --test eval_harness`.
2. **Non-Q rows and 7.3 row 27 left for their own scope:** `00-overview.md` 7.3 (revised 2026-11-25) also adds sets P6, B, W, K, defers P2/P4, and adds row 27 for set Q (*"A step whose only passing identity check is numeric is never shown as Verified; a Verified equation that disagrees at the numeric guard points ends Failed | Q | Must be 100 percent..."*). Following `eval-solve-badges.md`, this branch changes only the `Computed` -> `Verified` / `Checked numerically` Q measures and does not add unbriefed required pass-rule keys or alter P2/P4/P6/B/W/K. (The S2 property tests for numeric-vs-Verified and numeric guards already run in `research-tools`.)

---

## 4. Decisions taken alone and assumptions

1. **Two separate `MustBeZero` planted-error measures (`planted_error_ends_verified` and `planted_error_ends_checked_numerically`) instead of one combined count:** Splitting `planted_error_ends_computed` into two separate `Rule::MustBeZero` measures mirrors the `steps_verified_share` / `steps_checked_numerically_share` split, immediately tells a reviewer whether a planted error escaped via symbolic CAS (`Verified`) or numeric sampling (`Checked numerically`), and guarantees the gate stays `INCOMPLETE` if either tier count is omitted.
2. **Fail-closed retired-key table (`RETIRED_MEASURES`) instead of silent aliasing:** Prevents any saved pre-revision `Computed` number from falsely implying exact-vs-numeric separation or passing the new gate without re-measurement.
3. **Pure `SolveQCounts` and `count_in` helpers in `measures.rs`:** A naive caller setting `planted_error_ends_verified = 0.0` when 0 planted-error cases ran would falsely pass `Rule::MustBeZero`. `count_in(count, sample_size)` and `SolveQCounts::record_into` enforce that empty samples (`planted_total == 0` or `steps_total == 0`) remain absent (`Outcome::NotMeasured`) and clear any stale Q entries, while rejecting counts whose mutually-exclusive tier sum exceeds the sample total.
4. **Assumption on `SolveQCounts` denominator (`steps_total`):** `steps_verified + steps_checked_numerically + steps_model_agreed + steps_unchecked <= steps_total` (not `==`), because a Solve run can also contain `Given`, `Cited`, `CheckedByYou`, `ModelAgreedSameModel`, `AssumedByModel`, `Failed`, or `Stale` steps. Whether a future Q runner pools `ModelAgreedSameModel` into `steps_model_agreed` or records only independent `ModelAgreed` should be documented in `measure_notes`.
5. **Unit/domain bounds in `evaluate()`:** Enforced finite non-negative (`!v.is_sign_negative()`, rejecting `-0.0`), `share <= 1.0`, `percent <= 100.0`, integer values on `Rule::MustBeZero` counts (`v.fract() == 0.0`), and `sum(Q step shares) <= 1.0 + 1e-9`. Recorded-only `"count"` measures (such as `quotes_accepted_per_model_call` or `model_calls_typical`) may legitimately be non-integer averages and remain unrestricted as long as they are finite and non-negative.
6. **Subagent model mapping:** The prompt requested Neon for implementation and Carbon for hard independent audits. This environment's `invoke_subagent` tool exposes `inherit`, `flash_lite`, `flash`, and `pro` rather than literal Neon/Carbon identifiers; the independent read-only audit subagent was invoked with `Model: "pro"` (the highest reasoning tier available), and implementation was performed directly in the primary session.

---

## 5. Merge notes

### Files touched on this branch
- `crates/agent/tests/eval_support/measures.rs` (commit `52f4e08`): Q rows in `MEASURES`, `solve_wording`, `RETIRED_MEASURES`, `count_in`, `SolveQCounts`, `percent`, `evaluate`.
- `crates/agent/tests/eval_support/solve_q_tests.rs` (new file, commit `d0b2f72`): isolated new test file, zero merge conflict expected.
- `crates/agent/tests/eval_harness.rs` (commit `d0b2f72`): 1 module declaration, updated Q pass-rule entries in `measure_table_is_the_table_of_section_7_3`, strengthened `gate_never_passes_what_was_not_measured`, `scripted_case_runs_end_to_end_and_fills_a_sheet`, and `sheet_template_is_the_rendering_of_a_blank_sheet`.
- `docs/design/eval-2026-11/sheet-template.md` (commit `02fcfa0`): regenerated by `EVAL_BLESS=1 cargo test --locked -p agent --test eval_harness`. If another branch also changes `MEASURES` or `sheet.rs`, re-run `EVAL_BLESS=1 cargo test --locked -p agent --test eval_harness` after merging to regenerate `sheet-template.md` cleanly.

### Required post-merge alignment in `tests/fixtures/eval/manifest-q.json` (owned by `eval-q-cases`)
Per the brief, `tests/fixtures/eval/manifest-q.json` and `tests/fixtures/eval/scripts/Q-selftest-gaussian.json` were left completely untouched on this branch because `eval-q-cases` owns them. After both `eval-q-cases` and `eval-solve-badges` merge, the integrator should align the two harness-only self-test case keys in `tests/fixtures/eval/manifest-q.json` (keeping both case IDs, inputs, `planted_step`, and scripted fixture intact):

1. **`Q-selftest-gaussian` (`tests/fixtures/eval/manifest-q.json:17`):**
   - Current: `{ "id": "s1", "expected_tier": "Computed" }`
   - Change to: `{ "id": "s1", "expected_tier": "Verified" }` (exact symbolic closed-form integral $\int_{-\infty}^{\infty} e^{-x^2}\,dx = \sqrt{\pi}$, matching `solve_badge::Tier::Verified` / `wording::BADGE_VERIFIED`).
2. **`Q-selftest-planted` (`tests/fixtures/eval/manifest-q.json:38`):**
   - Current: `{ "id": "s1", "expected_tier": "Computed" }`
   - Change to: `{ "id": "s1", "expected_tier": "Verified" }` (exact symbolic product-rule step $\frac{d}{dx}(x^2 e^x) = 2x e^x + x^2 e^x$). Step `s2` (`manifest-q.json:39`) is already `"expected_tier": "Failed"` and needs no change.
3. **Additional integration note for S9/S11:** `Q-selftest-planted` (`manifest-q.json:43`) notes `"No Solve code exists yet, so nothing runs this case; it only exercises the format."` Once the Solve runner executes Q manifests, verify whether step IDs `"s1"`/`"s2"` in these two self-test keys should remain harness-local or align with `solve_model::is_valid_step_id` (`"e-N"`).

---

## 6. Verification, test commands, and exact results

| Command | Result | Exact counts / notes |
|---|---|---|
| `EVAL_BLESS=1 cargo test --locked -p agent --test eval_harness` | **PASS** (exit 0) | Regenerated `docs/design/eval-2026-11/sheet-template.md`; 13 passed, 0 failed, 2 ignored (before audit additions) |
| `cargo test --locked -p agent --test eval_harness` | **PASS** (exit 0) | **14 passed, 0 failed, 2 ignored** (`file_sheet_from_json`, `live_manifests_validate` are pre-existing live-tier `#[ignore]` tests); `sheet-template.md` matches byte for byte without `EVAL_BLESS` |
| `cargo test --locked -p research-tools --test tool_def_golden` | **PASS** (exit 0) | **3 passed, 0 failed, 0 ignored**; `crates/research-tools/tests/golden/tool_defs.txt` unchanged |
| `cargo test --locked --workspace --no-fail-fast` | **PASS** (exit 0) | **783 passed, 0 failed, 2 ignored** across 105 test suites |
| `RE_SEARCH_PYTHON="$(command -v python3)" cargo test --locked -p export --no-fail-fast` | **PASS** (exit 0) | **8 passed, 0 failed, 0 ignored** across 4 test suites (including Python replay checks) |
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | **PASS** (exit 0) | `No touched source file is over 600 lines.` (`measures.rs`: 530, `solve_q_tests.rs`: 360, `eval_harness.rs`: 458) |
| `rustc --crate-type lib --edition 2021 -D warnings --emit metadata` on `measures.rs` with minimal `Set` stub | **PASS** (exit 0) | 0 errors, 0 warnings — confirms `measures.rs` stays pure `std`-only |
| `cargo clippy --locked --workspace --all-targets --keep-going -- -D warnings` | **PRE-EXISTING ONLY** | **0 diagnostics** in `measures.rs`, `solve_q_tests.rs`, `eval_harness.rs`; 6 pre-existing `rustc 1.100.0` / Linux diagnostics in untouched files (proved below) |

---

## 7. Pre-existing failures (with proof on base commit `afe6d70`)

Verified by checking out a detached worktree at `origin/gemini/work/fixed` (`afe6d70`) and running `cargo clippy --locked --workspace --all-targets --keep-going -- -D warnings`. The exact same 6 diagnostics occur on the unmodified base commit on `rustc 1.100.0` / Linux:

1. `crates/desktop-server/src/evidence.rs:744:40` — `clippy::manual_unwrap_or` (also noted in `RULES.md`; file is over 600 lines and untouched)
2. `crates/agent/tests/eval_support/manifest.rs:289:19` — `clippy::match_like_matches_macro` (untouched)
3. `crates/agent/tests/eval_support/sheet.rs:102:8` — `clippy::unnecessary_map_or` (untouched)
4. `crates/agent/src/guardrails.rs:335:1` — `clippy::items_after_test_module` (untouched)
5. `crates/research-tools/tests/sandbox_and_fs.rs:6:5` — unused imports `TEX_AUDIT` and `is_symlink_or_junction` on non-Windows (untouched)
6. `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6:15` — unused imports `BufRead` and `BufReader` on non-Windows (untouched)

---

## 8. What could not be compiled or run on Linux

- `cfg(windows)` code and Windows-only tests across `worker` (`AppContainerSandbox`, `JobObjectSandbox`, `python_worker`), `desktop-app` (WebView2), `chatgpt-auth` (Windows DPAPI vault), `cli` (`contained_router`), and `research-tools` Windows symlink/junction tests do not compile/execute on Linux.
- The two `#[ignore]` live-tier tests in `crates/agent/tests/eval_harness.rs` (`live_manifests_validate` and `file_sheet_from_json`) require external `EVAL_LIVE_MANIFEST_DIR` / `EVAL_SHEET_JSON` inputs and were not run.
- No files under `crates/desktop-server/assets/` were touched, so `node --check` / `node --test` were not required.

---

## 9. Bugs noticed but not fixed (outside scope)

1. **`crates/agent/tests/eval_support/sheet.rs:111-115`:** `validate_sheet` checks `super::measures::measure(id).is_none()` for `measure_notes` without consulting `measures::retired_measure(id)`. Retired pre-revision keys present only in `measure_notes` still fail closed, but receive the generic `"not a measure of section 7.3"` error instead of migration guidance naming the replacement keys.
2. **`crates/agent/tests/eval_support/sheet.rs:183-227`:** `render_markdown` validates `sheet.values` via `sheet.gate()?` but does not validate `sheet.measure_notes`. Calling `render_markdown` directly (bypassing `validate_sheet` / `write_sheet`) silently drops unknown or retired keys present only in `measure_notes`.
3. **`crates/cli/tests/desktop_and_ext.rs:149-168`:** Running `cargo test --workspace` on Linux with `RE_SEARCH_PYTHON` exported causes `re-search run --route api` to reject `--python` on non-Windows and exit before contacting `ScriptedMock`, leaving `mock.finish()` blocked indefinitely on `listener.accept()`. Running `cargo test --workspace` without `RE_SEARCH_PYTHON` and passing `RE_SEARCH_PYTHON` only to `cargo test -p export` avoids the hang.
4. **`crates/agent/tests/eval_support/measures.rs:80-140` (non-Q rows & 7.3 row 27):** `MEASURES` still lists deferred P2/P4 measures and does not yet include the 2026-11-25 scope-revision rows for P6, B, W, K or Q row 27 (numeric-vs-Verified & branch-cut numeric guard). These should be synchronized in a dedicated F8 follow-up once P6/B/W/K manifest sets land.

---

## 10. Design contradictions or impossibilities found

None blocking this brief. Note only that `00-overview.md` 8.3 question 11 explicitly accepts `"Checked numerically"` while noting `"Verified numerically"` as an alternative that changes two strings and no logic; `solve_wording` keeps this changeable in a single literal (`define_solve_wording!`), and `solve_q_tests.rs` cross-checks lockstep parity with `crates/research-tools/src/solve_badge.rs`.
