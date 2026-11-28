# Brief solve-s7-exprs — Solve profile freeze (`solve/1`), policy, and expression store (S7)

## 1. Package and governing design sections

- **Package:** **S7** — Freeze the complete 16-tool `solve/1` profile and `solve_policy()` (`"v0.5-solve"`), implement the versioned expression handle store (`solve_exprs.rs`) harvesting protocol-2 `sympy_eval` outbox expressions through the S6 check process (`srepr_safe.py` canonicalization + `latex` + `free_symbols`), wire `expr_show`, and provide fail-closed unbuilt stubs for `step_record`, `check_run`, and `solve_finish` so S8, S9, and S11 drop in their handlers later with **zero** `def_hash` or `policy_version` changes.
- **Governing design:** `00-overview.md` 0.1, 2.2 (F1), 3.1 (rows 1, 6, 7), 6 ("What can move"), 8.1–8.3; `solve.md` 4.2, 5.1, 6 (R1, R5, R6, R18), 7.1–7.2, 8.1, 8.3, 9 (rows S1, S7), 12; `agents/RULES.md`.
- **Read reports first:** `reports/f1-tool-profiles.md`, `reports/s2-solve-model.md`, `reports/solve-evidence-labels.md`, and `reports/solve-comparison-evidence.md`. Note: S6's `srepr_safe.py` stop-gate ALREADY passed (`52/52` core math/physics expressions accepted, `48/48` hostile inputs rejected), and `RULES.md:57-60` explicitly authorizes `solve-s7-exprs` to register `solve/1` and its 4 Solve tool definitions while keeping `research/1`, `investigation/1`, and `tests/golden/tool_defs.txt` untouched.

## 2. Start conditions & file-disjointness with `solve-s4-s5-setup`

- **Merged on `gemini/work/fixed`:** F0–F12, F13a, F14a, F15 core, F16a, S2 (with evidence-label and comparison-evidence fixes), S3 (`tools::Gate`), and S6 (`worker` protocol 2, `WorkerRole::Check`, `python/checks/srepr_safe.py`).
- **Parallel lane safety:** `solve-s4-s5-setup` runs in parallel and owns `solve_setup.rs`, `solve_model.rs` (`QuoteRef` re-export), `desktop-server/src/solve.rs`, `solve_view.rs`, `routes_session.rs`, and `assets/solve_*.js`. **Do NOT edit any of those files.** S8, S9, and S11 follow in a separate brief after `solve-s4-s5-setup` and `solve-s7-exprs` both merge.

## 3. Implementation details

1. **`crates/research-tools/src/policy_preset.rs`:**
   - Add `pub const SOLVE_POLICY_VERSION: &str = "v0.5-solve";` and `pub fn solve_policy() -> PolicyConfig`: default `Deny`, explicit rules for all 16 `solve/1` tool IDs, every `ExecuteCode` tool (`sympy_eval`, `check_run`) set to `Ask` with per-caller `WorkerSession` scope (`solve.md` 5.1, R18).
2. **Frozen 16-tool `solve/1` profile (`crates/research-tools/src/tool_profile.rs` + `solve_defs.rs` if helpful for the 600-line limit):**
   - **12 reused `research/1` tools (exact `def_hash` preserved):** `sympy_eval`, `ledger_supersede`, `ledger_query`, `ledger_note`, `read_paper_lines`, `search_paper_passages`, `arxiv_search`, `literature_search`, `fs_read`, `fs_list`, `check_expression_sanity`, `tex_audit`.
   - **4 new Solve tool definitions (canonical schemas frozen now so S8/S9/S11 never alter `def_hash`):**
     - `expr_show` (`ObserveLocal`): inspect stored handle `name` (or `name@version`), returning canonical `latex`, `str_preview`, `free_symbols`, `portable`, `srepr_sha256`, and optionally `srepr` when `include_srepr: true`.
     - `step_record` (`ObserveLocal`): canonical S8 input schema (`solve.md` 5.1 / 7.2; NO `latex` input field per R5); handler in S7 returns a clean fail-closed error `"step_record is not wired yet (package S8)"` writing zero domain events.
     - `check_run` (`ExecuteCode`, `ApprovalScope::WorkerSession` on the check-process `WorkerEpoch`): canonical S9 input schema (`entry_id`, `claim_index`, `check`, `params` — model picks check name + params only, never code/expected/outcome per R3); handler in S7 returns `"check_run is not wired yet (package S9)"`.
     - `solve_finish` (`ObserveLocal`): canonical S11 input schema (`outcome`, `final_entry_id`, `summary_markdown`, `open_items`); handler in S7 returns `"solve_finish is not wired yet (package S11)"`.
   - **Explicitly EXCLUDED from `solve/1`:** `doc_ingest`, `ledger_append`, `lean_check`, `search_formal_library`, `fs_write`, `latex_patch`, `latex_compile`, all `record_paper_*` / `read_map_node` / `record_map_reading` / `record_context_cards`, and all `idea_*` tools.
   - **Extend `ProfileHostTools` cleanly without breaking existing callers:** add optional check-worker host plumbing (e.g. builder method `.with_solve_check(...)` or default `None` falling back to `NoWorker` + fail-closed handler during static profile enumeration in `tool_profile_tests.rs` and `cli-route/src/core_tools.rs`), updating all `ProfileHostTools` struct-literal call sites cleanly if a field is added (`tool_profile_tests.rs`, `desktop-server/src/routers.rs`, `desktop-server/tests/eval_set_r_support/mod.rs`, `cli-route/src/core_tools.rs`, `cli/src/runtime.rs`).
   - Update `crates/research-tools/tests/golden/tool_profiles.txt` **ONLY** to add `[profile solve/1]`. **Keep `tests/golden/tool_defs.txt`, `research/1`, and `investigation/1` byte-for-byte untouched.** Note: `session_mode::profile_for_mode(Mode::Solve)` in `desktop-server` will now return `Ok(solve/1)`! Check `crates/desktop-server/tests/mode_shell.rs` — do NOT weaken or modify any existing test; if `mode_shell.rs` asserts `"solve"` is unbuilt before S4/S12 wire Solve session creation/turns, check `session_mode.rs` carefully so `mode_shell.rs` continues to pass untouched (e.g. keep `session_mode::is_mode_enabled` or check what `mode_shell.rs` tests!).
3. **Versioned expression store & `expr_show` (`crates/research-tools/src/solve_exprs.rs` + `solve_exprs_tests.rs`):**
   - Validate handle names (`validate_expr_name`: ASCII identifier `[A-Za-z_][A-Za-z0-9_]{0,63}`, rejecting reserved names).
   - Enforce per-run caps (`MAX_EXPRS_PER_RUN = 100`, `MAX_EXPR_SREPR_BYTES_PER_RUN = 2 * 1024 * 1024`).
   - Canonicalize each outbox expression via a ` CheckCanonicalizer` trait (backed in production/tests by S6's check process `python/worker.py --role check --stdio` / `Worker::new_with_role(..., WorkerRole::Check)` `canonicalize_expr`):
     - Valid `srepr_safe.py` expressions store canonical `srepr` in `Ledger::put_blob`, compute `srepr_sha256`, `latex`, `str_preview` (`<= 400` chars UTF-8 safe), `free_symbols`, `portable: true`, and increment version `name@1`, `name@2`, ...
     - Non-parseable / rejected `srepr` expressions record `portable: false` (never crash the turn, never pass raw solver `latex`, rule R6).
   - Append `solve.expr` v1 matching S2's `SolveExpr` schema in `solve_model.rs` (`SolveState::from_events` must fold every emitted `solve.expr` cleanly!).
   - Provide `stage_exprs_in` helper reconstructing `{ name: canonical_srepr }` from `Ledger` blobs for portable handles so `Worker::evaluate_with_exprs` re-stages expressions cleanly across worker restarts without `.cas_bus.json`.
   - Implement the real `expr_show` `ToolHandler` reading folded `SolveState` + `Ledger` blobs.

## 4. Owned files & guard rails

- **New:** `crates/research-tools/src/solve_defs.rs` (optional), `crates/research-tools/src/solve_exprs.rs`, `crates/research-tools/src/solve_exprs_tests.rs`, `crates/research-tools/tests/solve_s7_exprs.rs`.
- **Small localized edits (`<= 600` lines each):** `crates/research-tools/src/policy_preset.rs`, `crates/research-tools/src/tool_profile.rs`, `crates/research-tools/src/tool_profile_tests.rs`, `crates/research-tools/src/lib.rs`, `crates/research-tools/tests/golden/tool_profiles.txt`, and (only if needed for `ProfileHostTools` struct literals) the 4 external call sites. Check `desktop-server/src/session_mode.rs` vs `tests/mode_shell.rs` before committing!
- **Do NOT edit:** `tests/golden/tool_defs.txt`, `solve_model.rs`, `solve_state.rs`, `proof_dag.rs`, `resume.rs`, `desktop-server/tests/mode_shell.rs`, `desktop-server/tests/eval_set_r.rs`, `Cargo.lock`.

## 5. Required verification & hand-back

- Run `python3 -m unittest discover -s python`, `cargo test --locked -p research-tools --test tool_def_golden --test tool_profile_golden --test solve_s7_exprs`, `cargo test --locked -p desktop-server --test mode_shell --test mode_markers_r2 --test eval_set_r`, `cargo test --locked -p agent --test resume_loop`, `cargo test --locked -p agent --lib resume`, `cargo test --locked --workspace --no-fail-fast`, `cargo clippy --locked -p research-tools --all-targets -- -D warnings`, and `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`.
- Include cross-process R5/R6 tests (solver monkey-patching `sympy.latex` cannot alter `solve.expr.latex`; non-allowlisted `Lambda` marks `portable: false`; restart re-stages `exprs_in` from ledger blobs) and verify `mode_markers_r2` passes with all 4 new Solve tools mapped in its audit table if `mode_markers_r2` enumerates `solve/1`!
- Write `docs/design/modes-2026-11/reports/solve-s7-exprs.md` and push to `gemini/modes/solve-s7-exprs`.
