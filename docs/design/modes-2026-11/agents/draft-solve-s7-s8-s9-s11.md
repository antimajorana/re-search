# Brief draft-solve-s7-s8-s9-s11 — Solve core engine: handles, steps, algebra checks & gates (S7 -> S8 -> S9 -> S11)

**Draft status:** Planning brief (do not launch until start conditions, the `RULES.md` authorization gate, and the S6 parser stop-gate ALL pass).
**Packages:** S7 (freeze full `solve/1` profile + `solve_policy()`, expression store, `expr_show`) -> S8 (`step_record`, receipts view, attach-source & `solve/ack` routes) -> S9 (algebra checks, `check_run`, auto-checks, `solve/check` & `solve/recheck` routes) -> S11 (Solve gates G1/G4/G5/G6/G7 in `research-tools::solve_gates`, `solve_finish`, `solve.done`, M3 Set Q gate).
**Governing design:** `00-overview.md` 0.1, 2.2 (F1), 2.5 (F4), 2.7 (F10), 2.12 (F14), 3.1 (rows 1, 6, 7, 11, 12), 6 ("What can move", M2, M3), 7.2–7.3 (set Q, R22), 8.1–8.3 (owner decisions of 2026-11-25); `solve.md` 4, 5.1, 5.4, 6 (R1, R3–R12, R15, R17–R19, R22), 7.1–7.3, 8.1, 8.3, 9 (rows S7, S8, S9, S11), 12 (risks 3–5); `agents/RULES.md`.
**Read reports first:** `reports/s2-solve-model.md`, `reports/f1-tool-profiles.md`, `reports/f4-f9-quote-budget.md`, `reports/f10-library.md`, `reports/f14a-pdf-core.md`, and (once merged) `reports/q-f11-x1-s3-s6.md`, `reports/solve-s4-s5-setup.md`, `reports/eval-solve-badges.md`, `reports/eval-q-cases.md`.

## 1. Start conditions, coordinator gate & stop-gates

- **HARD COORDINATOR / OWNER GATE (`agents/RULES.md:57-59`) BEFORE S7 STARTS:**
  - `RULES.md:57-59` currently authorizes tool-profile additions ONLY for `inv-v6-tasks`. Before launching S7, the coordinator/owner MUST amend `RULES.md:57-59` to authorize `solve/1`'s **4 new tool definitions** (`expr_show`, `step_record`, `check_run`, `solve_finish`), `solve_policy()`, and the `[profile solve/1]` section in `crates/research-tools/tests/golden/tool_profiles.txt` (keeping `research/1`, `investigation/1`, and `tests/golden/tool_defs.txt` strictly frozen and untouched!).
- **Required merges on `origin/gemini/work/fixed` before S7:**
  - Already merged on base `832eab5bf`: F0, F1, F2, F3, F4, F8, F9, F10, F14a, S2.
  - **Hard merge gates:** `q-f11-x1-s3-s6` (**S3** `tools::Gate` + post-call hook, and **S6** worker protocol 2, check process, `srepr_safe.py`) MUST be merged.
  - **Required before S8:** `solve-s4-s5-setup` (S4 `solve.rs` sub-dispatcher and aligned `QuoteRef` in `solve_model.rs`).
  - **Required before S11 M3 verification:** `eval-solve-badges` and `eval-q-cases` (aligned Set Q measures and fixed-tier cases).
- **MANDATORY STOP-AND-CHECK GATE (`00-overview.md` 6, "What can move"):**
  - Before writing a single line of S7, read `reports/q-f11-x1-s3-s6.md` for S6's measured `srepr_safe.py` corpus acceptance rate and list of rejected constructors.
  - **If `srepr_safe.py` rejects a broad share of ordinary algebra/physics expressions, STOP S7–S9 and widen `srepr_safe.py`'s allowlist first** (in a dedicated PR/checkpoint with hostile + corpus tests).

## 2. Interfaces: base-verified vs future-merge

**Verified on base `832eab5bf`:**
- S2 (`crates/research-tools/src/`): `solve_model.rs` (486 lines), `solve_state.rs` (517 lines — close to 600; put new tests in sibling `solve_*_tests.rs`), `solve_status.rs` (230), `solve_status_tests.rs` (553 — do not grow past 600), `solve_own_tier.rs` (415), `proof_closure.rs` (247).
- **CRITICAL S2 CONTRACT (`solve_own_tier.rs:142-172`):**
  - `is_exact_comparison` grants `Tier::Verified` for `series_match` / `vanishes_to_order` **ONLY** when `solve.check.evidence["exact"] == true` or `evidence["comparison"] == "exact"` (and `evidence["numeric_coefficients"] != true`); otherwise it fails closed to `Tier::CheckedNumerically`. S9 MUST emit `"exact": true` for exact symbolic coefficient comparisons and `"numeric_coefficients": true` / `"exact": false` for numeric ones.
  - `equation_numeric_evidence` requires `points >= 12`, `digits >= 50`, and `domain_checked != false` for `identity_numeric` -> `CheckedNumerically`.
  - `parse_evidence_quote` sets `ocr: true` when `evidence.quote` payload has `ocr: true` (or `scanned: true` / `source_kind: "ocr"`).
- **CRITICAL F1 PROFILE TRAP (`tool_profile.rs:109-111`, `resume_profile.rs:163-199`):**
  - `ToolProfile::allows_gain` ONLY tolerates `ReadWorkspace` tools — NONE of `expr_show`/`step_record`/`solve_finish` (`ObserveLocal`) or `check_run` (`ExecuteCode`) can be added via `may_gain`, and changing `policy_version` in S9 would break resume of any S7/S8 session!
  - **Therefore, freeze the COMPLETE 16-tool `solve/1` definition list AND `solve_policy()` (`"v0.5-solve"`) ONCE in Checkpoint S7** (mirroring Explore X7's pattern), with fail-closed unbuilt stub handlers for `step_record`, `check_run`, and `solve_finish` returning clear errors (`"not wired yet (package S8/S9/S11)"`) until S8/S9/S11 drop in their real handlers with **zero `def_hash` or `policy_version` changes**! Update `tool_profiles.txt` **ONCE in S7**!
- **Exact 16-tool `solve/1` list (`solve.md:347-356`):**
  - 12 reused `research/1` tools (byte-for-byte identical `def_hash`): `sympy_eval`, `ledger_supersede`, `ledger_query`, `ledger_note`, `read_paper_lines`, `search_paper_passages`, `arxiv_search`, `literature_search`, `fs_read`, `fs_list`, `check_expression_sanity`, `tex_audit`.
  - 4 new Solve tools: `expr_show` (`ObserveLocal`), `step_record` (`ObserveLocal`), `check_run` (`ExecuteCode`, `WorkerSession` on check-process epoch), `solve_finish` (`ObserveLocal`).
  - **Explicitly EXCLUDED from `solve/1`:** **`doc_ingest`**, `ledger_append`, `lean_check`, `search_formal_library`, `fs_write`, `latex_patch`, `latex_compile`, all Investigation recorders (`record_paper_*`, `record_map_reading`, `read_map_node`, `record_context_cards`), and all Explore tools (`idea_*`).
- **CRITICAL `ProfileHostTools` 6-CALLER WARNING (`tool_profile.rs:131-137`):**
  - Adding check-process/Solve fields directly to `ProfileHostTools` breaks **6 existing struct-literal callers across 4 crates**:
    1. `crates/research-tools/src/tool_profile_tests.rs:169, 175, 190` (3 call sites)
    2. `crates/desktop-server/src/routers.rs:125`
    3. `crates/desktop-server/tests/eval_set_r_support/mod.rs:123`
    4. `crates/cli-route/src/core_tools.rs:38` (coordinate cleanly with S15/X18; must handle `NoWorker` stub cleanly during `core_tool_set_for_profile`!)
    5. `crates/cli/src/runtime.rs:183`
  - **Required design:** Add `pub solve_check: Option<SolveCheckHost>` (holding `check_epoch: Arc<dyn WorkerEpoch>` + `check_handler: Arc<dyn ToolHandler>`) via constructor/builder (`ProfileHostTools::new(sympy_epoch, sympy_handler).with_session_title(...).with_solve_check(...)`) — or update all 6 call sites cleanly in S7's isolated profile commit, falling back gracefully to `NoWorker`/error-stub when `solve_check` is `None` during tool-definition enumeration (`core_tool_set_for_profile` / `tool_profile_tests.rs`)!
- **CRITICAL CRATE BOUNDARY & G7 HONESTY DESIGN (`crates/mcp-server/Cargo.toml`, `crates/agent/src/honesty.rs`):**
  - `crates/mcp-server/Cargo.toml` depends on `research-tools` and does **NOT** depend on `agent` (zero `Cargo.lock` edits!). Therefore **`SolveGate` (G1, G4, G5, G6, G7) MUST live in `crates/research-tools/src/solve_gates.rs`** (re-exported from `crates/agent` if convenient) so both `agent` (`solve_loop.rs`) and `mcp-server` (`core.exe`, S15) import it directly!
  - **Do NOT blindly call legacy `agent::honesty::lint_honesty_with_ledger` for G7:** legacy `honesty.rs` inspects `ProofDag` + `LEAN_PHRASES`, which **falsely flags** `solve.check`-backed `Verified` steps and ignores `CheckedNumerically`! Build a **Solve-tier-aware G7 summary checker** directly inside `crates/research-tools/src/solve_done.rs` + `solve_gates.rs` over `SolveState` + `compute_all_tiers` + R22 wording rules (`"verified"` allowed ONLY next to a step whose effective tier is `Verified`, and for the answer as a whole ONLY when the final step's effective tier is `Verified`; `"proved"` never on a computed step).
- `crates/research-tools/src/lib.rs` is **300 lines** on base `832eab5bf`. `policy_preset.rs` is 77 lines. `quote_core.rs` (472), `lines_sent.rs` (465), `run_budget.rs` (464), `proof_dag.rs` (549, untouched by S2).

**Future-merge interfaces to verify before coding:**
- From **S3** (`q-f11-x1-s3-s6`): exact `tools::Gate` trait signature (`before` and post-call hook) and `guardrail.blocked` event emission in `crates/tools/src/gate.rs` and `router.rs`.
- From **S6** (`q-f11-x1-s3-s6`): exact worker protocol 2 request/response keys (`exprs_in`, outbox `exprs_out`, check-process IPC method for canonical `srepr`, `latex`, and named checks) in `python/worker.py`, `python/checks/__init__.py`, `srepr_safe.py`, and `crates/worker/src/lib.rs`.
- From **S4** (`solve-s4-s5-setup`): `crates/desktop-server/src/solve.rs` sub-dispatcher and aligned `solve_model::QuoteRef`.

## 3. Smallest implementation checkpoints (push after each)

Run tests + `check_file_length.py`, commit, and push to `gemini/modes/solve-s7-s8-s9-s11` after each checkpoint.

1. **Checkpoint S7 (freeze full 16-tool `solve/1` + `solve_policy()`, expression store & `expr_show`):**
   - `crates/research-tools/src/policy_preset.rs` (77 lines): add `solve_policy()` (`SOLVE_POLICY_VERSION = "v0.5-solve"`), default `Deny`, explicit rule for all 16 `solve/1` tool IDs, every `ExecuteCode` tool (`sympy_eval`, `check_run`) is `Ask`, per-caller scope (`solve.md` 5.1, R18).
   - Define frozen canonical `ToolDef`s for all 4 new Solve tools (`expr_show`, `step_record`, `check_run`, `solve_finish`), bind all 16 tools + `solve_policy()` into `solve_v1()` in `tool_profile.rs`, update the 6 `ProfileHostTools` call sites (with `NoWorker` support in `core_tools.rs` and `tool_profile_tests.rs`), wire real `expr_show` + fail-closed unbuilt stubs for `step_record`/`check_run`/`solve_finish`, and add `[profile solve/1]` **ONCE** to `tests/golden/tool_profiles.txt`. **Never touch `tool_defs.txt`.**
   - `crates/research-tools/src/solve_exprs.rs` (+ `_tests.rs`): harvest outbox expressions after `sympy_eval`, enforce caps (100 handles and 2 MiB per run) and `validate_expr_name`, send to check process for `srepr_safe.py` validation, canonical `srepr` blob + `srepr_sha256`, `latex`, `str_preview` (`<= 400` chars), and `free_symbols` (non-parseable -> `portable: false` per R6), increment handle version `name@v`, append host-only `solve.expr` v1, restage handles after worker restart, attach concise previews to `sympy_eval` result JSON without changing `sympy_eval`'s `ToolDef`.
2. **Checkpoint S8 (`step_record`, receipts view, attach-source & `solve/ack` routes — ZERO `def_hash` change):**
   - `crates/research-tools/src/solve_receipts.rs`: pure view over `tool.request`/`tool.result` pairs (`code_sha256`, `wall_ms`, `process`, `exprs_out`, `env`, blob decoding).
   - `crates/research-tools/src/solve_steps.rs` (+ `_tests.rs`): replace S7's stub with the real `step_record` handler (NO `latex` input field per R5). Enforces all refusals: unknown/superseded/non-portable handle; computation type without successful receipt in `tool_call_refs` (R10, `tool_reference_matches`); `derivation`/`result`/`final_answer` with empty `depends_on` (R9); `final_answer` with Failed closure; `reference_lookup` whose `source_ref` (`QuoteRef`) does not match a session `evidence.quote` event, fails F4 `verify_quote`, or whose lines were not sent to the caller via F4 `lines_sent_with_ledger` (R17; never calls `investigation_evidence.rs`). Computes `derived_depends_on` via `proof_closure::compute_derived_deps` (R9), `claim_sha256`, `entry_sha256`; appends `proof.entry` v1 (additive fields only, keeping `proof_dag.rs` untouched) and optional `solve.plan` v1 on first call.
   - Host routes in `crates/desktop-server/src/solve.rs` (auth-guarded):
     - `"Attach a source"` (`POST /api/sessions/{id}/solve/attach-source`, `solve.md` 5.1): ingests from F10 library / F3 arXiv id if needed, calls F4 `verify_quote`, appends `evidence.quote` (with `ocr: true` when F14a page basis is scanned/OCR, max 300-char excerpt), returns `QuoteRef`.
     - `POST /api/sessions/{id}/solve/ack`: validates `entry_id` + `kind` (`assumption`, `hand_checked`; S17 extends with `transcription`) + current `entry_sha256`, appends host-only `solve.ack` v1 (D10, D11).
3. **Checkpoint S9 (algebra checks, `check_run`, auto-checks, `solve/check` & `solve/recheck` routes — ZERO `def_hash` change):**
   - `python/checks/algebra.py` + `python/tests/test_checks_algebra.py`:
     - `identity_symbolic`: runs in check process under setup symbol assumptions (`assumed_symbol`); applies guard (b) (`solve.md` 4.4) by evaluating at `identity_numeric` points for zero model calls (disagreement -> `fail` as suspected branch-cut/simplification error); returns `fail` ONLY when a numeric counterexample point exists; unreduced nonzero residual without counterexample -> `inconclusive`; symbol assumption mismatch -> `inconclusive: symbol assumptions differ` (`solve.md` 12 risk 4).
     - `identity_numeric`: `>= 12` points, 50 digits, drawn from declared assumptions/domain (`points`, `digits`, `domain_checked`); two-run value comparison emits `lhs_code_sha256` / `rhs_code_sha256`.
     - `series_match` & `vanishes_to_order`: emit `order: {variable, point, n}` plus explicit `"exact": true` (symbolic) or `"numeric_coefficients": true, "exact": false` (numeric).
     - `counterexample_search`: inequality check; passing gives sublabel only, never computed tier (R8).
     - Register all 5 in `python/checks/__init__.py` and hash check files into `registry_sha256`.
   - `crates/research-tools/src/solve_checks.rs` (+ `_tests.rs`): replace S7's stub with the real `check_run` handler (model picks check name + params only, never code/expected/outcome per R3), deterministic outcome mapping, `solve.check` v1 append, `evidence_sha256`, and S3 Router post-call auto-check hook issuing `check_run` as caller `host:check` (10 s cap -> `inconclusive` on timeout; missing grant -> `not_run`, never auto-approved per R18; appends outcomes to `step_record` return payload).
   - Host routes in `crates/desktop-server/src/solve.rs`:
     - `POST /api/sessions/{id}/solve/check` (`Add check` action for S13 Why popover, invoking `check_run` as caller `user` through `Router::try_call`).
     - `POST /api/sessions/{id}/solve/recheck` (R19: same stored request -> same `evidence_sha256`; mismatch surfaced as `"result changed on re-run"`).
4. **Checkpoint S11 (Solve gates G1/G4/G5/G6/G7 in `research-tools`, `solve_finish`, `solve.done` — ZERO `def_hash` change):**
   - `crates/research-tools/src/solve_done.rs` (+ `_tests.rs`): evaluate all 5 `DoneCriterion` kinds, compute `rests_on`, enforce G7 closure refusal (Failed or Stale) AND Solve-tier-aware R22 wording lint on `summary_markdown` (replacing S7's `solve_finish` stub), writing `solve.done` v1 (`finished` vs `finished_open_items`; R15).
   - `crates/research-tools/src/solve_gates.rs` (+ `solve_gates_tests.rs`, re-exported from `crates/agent` if convenient): implements `tools::Gate` over incremental `SolveState` (usable directly by both `agent` AND `mcp-server` with zero `Cargo.lock` change!):
     - **G1 interpret:** `> 3` un-cited, un-discarded successful `sympy_eval` runs -> allow only `step_record`, `expr_show`, `ledger_query`, `ledger_note`, and read tools; failed runs exempt.
     - **G4 budget:** compares F9 `run_budget::count_model_calls` (`RunScope::AfterSeq` until S12 adds `run_id`) + wall clock against `budgets.{max_model_calls, max_wall_minutes, max_model_calls_per_leg}` -> pause signal.
     - **G5 bounce:** `max_gate_bounces` (default 3) consecutive gate refusals -> pause signal `gate_bounces` (R12).
     - **G6 stuck:** 2 rejected reviews on the same step lineage -> pause signal `verification_stuck`.
     - **G7 finish:** Solve-tier-aware closure & R22 wording guard on `solve_finish` (`summary_markdown` checked strictly against `SolveState` + `tiers`, NEVER legacy `honesty.rs` `LEAN_PHRASES`).
   - Run headless Set Q evaluation gate (milestone M3).

## 4. Owned files

- **New:**
  - `crates/research-tools/src/{solve_exprs, solve_steps, solve_receipts, solve_checks, solve_done, solve_gates}.rs` (plus `*_tests.rs` siblings to stay `<= 600` lines each)
  - `python/checks/algebra.py`, `python/tests/test_checks_algebra.py`
- **Small localized edits (`<= 600` lines each):**
  - `crates/research-tools/src/tool_profile.rs` (329), `tool_profile_tests.rs` (200), `policy_preset.rs` (77), `lib.rs` (300)
  - `crates/research-tools/tests/golden/tool_profiles.txt` (updated **ONCE in S7** for `[profile solve/1]` ONLY, after `RULES.md:57-59` authorization!)
  - 4 external `ProfileHostTools` call sites (S7 only): `desktop-server/src/routers.rs` (150), `desktop-server/tests/eval_set_r_support/mod.rs` (380), `cli-route/src/core_tools.rs` (188), `cli/src/runtime.rs` (202)
  - `crates/desktop-server/src/solve.rs` (attach-source, `ack`, `check`, `recheck` routes — split helper file if `> 520` lines)
  - `python/checks/__init__.py` (register 5 algebra checks + `registry_sha256`)

## 5. Shared registration seams & Do-not-edit files

- **Coordination seams:**
  - `tool_profile.rs` & `tool_profiles.txt`: finalized ONCE in S7; S8, S9, S11 assert `tool_profiles.txt` stays byte-identical across their checkpoints!
  - `crates/research-tools/src/solve_gates.rs`: S20 adds G3 (`compute_blind`) ONLY after S11 and S12 merge; leave a clean match arm/hook for G3. S15 (`mcp-server`) imports `research_tools::solve_gates::SolveGate` directly!
  - `python/checks/__init__.py` & `solve_checks.rs`: S10 registers the 6 physics checks after S9 merges; keep check dispatch table data-driven.
- **Do NOT edit:** `crates/research-tools/tests/golden/tool_defs.txt` (frozen!), `research/1` or `investigation/1` profile definitions, `crates/research-tools/src/proof_dag.rs` (549, untouched as in S2), `crates/research-tools/src/solve_status_tests.rs` (553, keep untouched), `crates/research-tools/src/investigation_evidence.rs`, `crates/agent/src/honesty.rs` (legacy Research only), `crates/agent/src/runner.rs` (626, S12), `crates/agent/src/verifiers.rs` (878, S14), `crates/agent/src/resume.rs`, `Cargo.lock`.

## 6. Tests through the entry point

- **Python (`python3 -m unittest discover -s python`):** algebra checks covering exact symbolic identity, branch-cut simplification failure caught by numeric guard (b) (R8), `identity_numeric` (12 points / 50 digits), exact vs numeric `series_match` and `vanishes_to_order` (`"exact": true` vs `"numeric_coefficients": true`), `counterexample_search`, symbol assumption mismatch -> `inconclusive`, and timeout -> `inconclusive`.
- **Rust (`cargo test -p research-tools -p agent -p desktop-server`):**
  - **Profile stability:** `tool_profile_golden` verifies `[profile solve/1]` has exact 16 tools (no `doc_ingest`, no `lean_check`, no `ledger_append`) and `v0.5-solve` policy; S8/S9/S11 prove zero `def_hash` drift from S7's snapshot!
  - **R3/R5/R6:** arbitrary JSON never yields a passing `solve.check` without check-process evidence; patched solver `latex` / `simplify` cannot alter stored `solve.expr.latex` or check outcome.
  - **R8:** numeric check -> `CheckedNumerically` (never `Verified`); symbolic + failed numeric guard -> `Failed`; inequality `counterexample_search` -> `Unchecked`.
  - **R9/R10/R17:** undeclared handle dep inherited via `derived_depends_on`; fabricated/failed receipt refused; `reference_lookup` without `evidence.quote` or unsent lines refused; Cited step succeeds with NO Investigation marker and propagates `ocr: true` -> `"OCR source"`.
  - **R11/R12/R15/R18/R19/R22:** Router gate G1 refuses 4th uninterpreted run, G5 pauses after 3 bounces, G7 Solve-tier-aware check accepts `"verified"` ONLY next to `effective == Verified` (never falsely flagging `solve.check`-backed `Verified` steps like legacy `honesty.rs` would!), refuses `"verified"` on `CheckedNumerically` / `Unchecked` steps and refuses `"proved"` on computed steps, `host:check` without grant -> `not_run`, `POST solve/ack`, `POST solve/check`, and `POST solve/recheck` reproduce `evidence_sha256` and flag mismatch.
  - **Set Q headless evaluation (M3):** `cargo test -p agent --test eval_harness` against fixed-tier Set Q: 100% of planted errors end `Failed`, 0% end `Verified` or `Checked numerically`.
  - **Goldens & Set R:** `tool_def_golden` (`tool_defs.txt` byte-identical), `eval_set_r`, `resume_loop`, `resume_drift_tests`.

## 7. Document conflicts & resolutions

1. **`solve.md` 9 (row S1) progressive profile edits (S7..S11, S16) vs F1 `allows_gain` (`tool_profile.rs:109`) & `00-overview.md` 0.1:** S16 is CUT; moreover F1 rejects non-`ReadWorkspace` gains and `policy_version` changes mid-profile. **Resolution:** Freeze all 16 `solve/1` tools (excluding `doc_ingest`) + `solve_policy()` ONCE in S7 with fail-closed stubs for S8/S9/S11, updating `tool_profiles.txt` once after `RULES.md:57-59` authorization.
2. **`solve.md` 5.4 G7 (`lint_honesty_with_ledger`) & 9 S11 (`crates/agent/src/solve_gates.rs`) vs `crates/agent/src/honesty.rs` & `crates/mcp-server/Cargo.toml`:** Legacy `honesty.rs` checks `ProofDag` + `LEAN_PHRASES` (falsely flagging `solve.check`-backed `Verified` steps) and `mcp-server` does not depend on `agent`. **Resolution:** Implement Solve-tier-aware G7 checking directly over `SolveState` + `tiers` + R22 and place `solve_gates.rs` in `crates/research-tools/` so `agent` and `mcp-server` share it with zero `Cargo.lock` change.
3. **`solve.md` 7.2 (`ProofEntry` in `proof_dag.rs` gaining fields) vs S2 (`reports/s2-solve-model.md`):** Keep `proof_dag.rs` untouched; `step_record` writes additive fields on `proof.entry` v1 folded into `SolveEntry`.
4. **`solve.md` 7.1 vs `00-overview.md` 1 & 2.5:** Cited steps use F4 `quote_core` + `lines_sent` + `evidence.quote`, never `investigation_evidence.rs`.
5. **F9 `run_id` attribution gap (`reports/f4-f9-quote-budget.md` section 2):** S11 G4 scopes F9 `count_model_calls` via `RunScope::AfterSeq` until S12 adds optional `run_id`.

## 8. Out of scope & hand-back

- **Out of scope:** Physics checks (S10), Solve loop / prompts / `runner.rs` pause / resume brief (S12), derivation UI (S13), verifier loop (S14), CLI parity (S15), Lean/S16 (cut), TeX intake (S17), SciPy/artifacts (S18), notebook/ZIP export (S19), blind compute/G3 (S20), Research migration (S21), Cadabra2/FORM/S22 (deferred), F15 write-up adapter (Group 5).
- **Hand-back:** Write `docs/design/modes-2026-11/reports/solve-s7-s8-s9-s11.md` (recording `RULES.md` gate confirmation, S6 parser gate result, frozen 16-tool `solve/1` golden table, 6 `ProfileHostTools` call-site updates, and Set Q headless results) and push only to `gemini/modes/solve-s7-s8-s9-s11`.
