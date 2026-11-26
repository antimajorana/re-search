# Report: `solve-s7-exprs` — Solve profile freeze (`solve/1`), policy (`v0.5-solve`), and expression store (package S7)

- **Brief:** `docs/design/modes-2026-11/agents/solve-s7-exprs.md`
- **Base commit:** `921edc8955e111565f6ddf7e9c0f1dfc88b3b170` (`origin/gemini/work/fixed`)
- **Work branch:** `work/solve-s7-exprs` -> pushed to `gemini/modes/solve-s7-exprs`
- **Toolchain (Linux VM):** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `cargo 1.100.0 (3acc79c0c 2026-11-05)`, `Python 3.12.16`
- **Unchanged guard rails:** `Cargo.lock` untouched (0 new dependencies); `crates/research-tools/tests/golden/tool_defs.txt`, `[profile research/1]`, and `[profile investigation/1]` byte-for-byte untouched; `solve_model.rs`, `solve_state.rs`, `proof_dag.rs`, `resume.rs`, `desktop-server/tests/mode_shell.rs`, `desktop-server/tests/eval_set_r.rs`, and all `solve-s4-s5-setup` files (`solve_setup.rs`, `desktop-server/src/{solve,solve_view,routes_session}.rs`, `assets/solve_*.js`) untouched; `crates/policy` and `crates/ledger` remain free of `tokio`, `rmcp`, and `windows-sys`; `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` passes (`No touched source file is over 600 lines.`).

---

## 1. What was implemented (package S7)

### Frozen 16-tool `solve/1` definitions, stubs, policy, and profile registration (`crates/research-tools`)

| File | Lines | Commit | Role |
|---|---:|---|---|
| `crates/research-tools/src/solve_defs.rs` | 389 | `b9a496a` | New module freezing `EXPR_SHOW`, `STEP_RECORD`, `CHECK_RUN`, `SOLVE_FINISH`, `SOLVE_V1_TOOL_IDS` (16 sorted IDs), `SOLVE_V1_CHECK_NAMES` (11 v1 checks), canonical schemas (`expr_show_def`, `step_record_def` with no `latex` field per R5, `check_run_def` with `ExecuteCode` + `ApprovalScope::WorkerSession` and no `code`/`expected`/`outcome` per R3, `solve_finish_def`), fail-closed S7 stub handlers writing zero domain events, `solve_v1_defs()` (12 reused `research/1` tools with exact `def_hash` + 4 frozen Solve definitions), `register_solve_v1_reused_tools` (registering `ledger_supersede` & `ledger_query` without `ledger_append` and without touching `proof_dag.rs`), and `FallbackCheckEpoch`. |
| `crates/research-tools/src/policy_preset.rs` | 258 | `b9a496a` | Added `SOLVE_POLICY_VERSION = "v0.5-solve"`, `SOLVE_CHECK_CALLER = "host:check"`, `SOLVE_VERIFIER_CALLER = "verifier:solve"`, and `solve_policy() -> Policy` (default `Deny`, explicit rules for all 16 `solve/1` tools + MCP-only `set_session_title`, `ExecuteCode` (`sympy_eval`, `check_run`) and `NetworkRead` (`arxiv_search`, `literature_search`) set to `Ask`, explicit `Deny` caller scoping restricting `host:check` to `check_run` only and `verifier:solve` to verifier tools only), plus unit tests. |
| `crates/research-tools/src/tool_profile.rs` | 448 | `b9a496a` | Added `SolveCheckHost` and optional `solve_check: Option<SolveCheckHost>` on `ProfileHostTools` (plus builder methods `new`, `with_session_title`, `with_solve_check`), moved `PROFILE_SOLVE_V1` from `RESERVED_PROFILE_NAMES` to `BUILT_PROFILE_NAMES`, added `solve_v1()`, wired `PROFILE_SOLVE_V1` in `lookup`, `built_profiles`, and `register_profile` (preserving fail-closed `defs_match_exact` check). |
| `crates/research-tools/src/tool_profile_tests.rs` | 269 | `b9a496a` | Updated unbuilt-profile assertions from `solve/1` to `explore/1`, added `solve_v1_contains_16_tools_excludes_forbidden_and_preserves_reused_def_hashes` verifying all 12 reused `def_hash`es and all 16 excluded tools, and updated `ProfileHostTools` construction. |
| `crates/research-tools/src/lib.rs` | 325 | `b9a496a` | Declared `pub mod solve_defs;`, `pub mod solve_exprs;`, `mod solve_exprs_show;`, and re-exported `solve_policy` and `SOLVE_POLICY_VERSION`. |
| `crates/research-tools/tests/tool_profile_golden.rs` | 176 | `b9a496a` | Added verification that all 12 reused tools in `solve/1` match `tests/golden/tool_defs.txt` byte-for-byte while the 4 new Solve tools do not appear in `tool_defs.txt`, and updated `RESERVED_PROFILE_NAMES` assertion to `[investigation/2, explore/1]`. |
| `crates/research-tools/tests/golden/tool_profiles.txt` | 98 | `b9a496a` | Appended `[profile solve/1]` (`policy v0.5-solve`, `def 0..15`) ONLY; `[profile research/1]` and `[profile investigation/1]` are byte-for-byte unchanged. |

### Versioned expression handle store & `expr_show` (`crates/research-tools`)

| File | Lines | Commit | Role |
|---|---:|---|---|
| `crates/research-tools/src/solve_exprs.rs` | 502 | `b9a496a` | New module implementing `validate_expr_name` (`^[A-Za-z_][A-Za-z0-9_]{0,63}$` rejecting dunder `__*` and reserved keywords/prototype/CAS helper names), `clip_utf8_chars`, `MAX_EXPRS_PER_RUN = 100`, `MAX_EXPR_SREPR_BYTES_PER_RUN = 2 MiB`, `MAX_EXPR_LATEX_CHARS = 64_000`, `CanonicalizedExpr`, `CheckCanonicalizer` trait, `harvest_exprs_out` (using `read_verified_session`, blob digest checks, pre-write canonicalization, non-empty `srepr_sha256` verification, `Ledger::put_blob`, head-pinned `append_at_head`, and non-portable fallback never crashing turns or leaking solver LaTeX per R5/R6), `harvest_from_sympy_result` (reporting malformed `exprs_out` in `skipped`), `attach_harvest_previews`, `load_handle_srepr`, `stage_exprs_in_from_state`, and `stage_exprs_in` (reconstructing `{ name: canonical_srepr }` for latest portable versions across restarts without `.cas_bus.json`, never resurrecting stale earlier portable versions when latest is non-portable). |
| `crates/research-tools/src/solve_exprs_show.rs` | 180 | `b9a496a` | Private helper module (re-exported by `solve_exprs.rs`) implementing read-only `expr_show_handler` and `run_expr_show` (listing all handles or inspecting `name` / `name@version` / `name` + `version`, returning canonical `latex`, `str_preview`, `free_symbols`, `portable`, `srepr_sha256`, optional `srepr` when `include_srepr: true` or `form: "srepr"`, optional `text` for `form`, UTF-8 safe `max_chars` clipping in `1..=4000`, diagnostic `note` for non-portable handles, and full Rust-side argument validation). |
| `crates/research-tools/src/solve_exprs_tests.rs` | 375 | `b9a496a` | 8 unit tests covering identifier & reserved-name validation, handle versioning & `SolveState::from_events` folding, 100-handle cap, 2 MiB byte cap (including deduplicated-blob bypass prevention), rejected/errored/tampered/empty-hash non-portable fallback, non-resurrection of stale `f@1` when `f@2` is non-portable, UTF-8 safe clipping & all `expr_show` forms/errors, malformed `exprs_out`, and concurrent head-change fail-closed race protection (`append_at_head`). |
| `crates/research-tools/tests/solve_s7_exprs.rs` | 559 | `b9a496a` | 4 integration & cross-process tests spawning real separate OS processes (`python/worker.py --stdio --role solver` and `--role check`): (1) R5 solver monkey-patching `sympy.latex` cannot alter `solve.expr.latex` or `expr_show` output; (2) R6 non-allowlisted `Lambda` and hostile `srepr` strings mark `portable: false` without crashing or leaking solver strings; (3) killing solver #1, calling `stage_exprs_in` from ledger blobs, and spawning fresh solver #2 (distinct PID) evaluates `expr("V1")` cleanly with zero `.cas_bus.json`; (4) real `solve/1` `Router` + `solve_policy()` verifying per-caller `WorkerSession` grants, `host:check` denial without grant (R18), `verifier:solve` scoping, excluded-tool denial, `step_record` rejecting `latex` (R5), `check_run` rejecting `code`/`expected`/`outcome` (R3), and S7 stubs writing zero domain events. |

### Localized external caller & test updates (`desktop-server`, `cli-route`, `cli`, `agent`)

| File | Lines | Commit | Role |
|---|---:|---|---|
| `crates/desktop-server/src/session_mode.rs` | 221 | `8014355` | Added `const DESKTOP_MODE_SOLVE_ENABLED: bool = false;` gate in `profile_for_mode` and `profile_for_session` so `desktop-server` continues refusing Solve session creation and turns until S4/S12 wire Solve setup and loop (keeping `tests/mode_shell.rs` and `session_mode::tests` passing untouched). |
| `crates/desktop-server/src/routers.rs` | 151 | `8014355` | Added `solve_check: None` to `ProfileHostTools` literal. |
| `crates/desktop-server/tests/eval_set_r_support/mod.rs` | 381 | `8014355` | Added `solve_check: None` to `ProfileHostTools` literal. |
| `crates/desktop-server/tests/mode_markers_r2.rs` | 435 | `8014355` | Added `solve_check: None` to both `ProfileHostTools` literals, mapped `check_run`, `expr_show`, `solve_finish`, and `step_record` in `tool_emitted_event_kinds()`, and added `solve_defs.rs`, `solve_exprs.rs`, and `solve_exprs_show.rs` to the R2 static source audit. |
| `crates/cli-route/src/core_tools.rs` | 196 | `8014355` | Added `solve_check: None` to `ProfileHostTools` literal; updated `profile_set_builds_known_profiles_and_rejects_unbuilt` to verify `solve/1` builds 17 MCP tools with `v0.5-solve` and `explore/1` fails as unbuilt. |
| `crates/cli/src/runtime.rs` | 203 | `8014355` | Added `solve_check: None` to `ProfileHostTools` literal. |
| `crates/agent/src/resume_profile_tests.rs` | 481 | `8014355` | Updated unbuilt-profile assertions (`investigation/2`, `explore/1`, `idea.started` stay `NotBuiltYet`), verified `solve/1` and `solve.started` resolve to `solve/1` with `v0.5-solve`, and added a `solve/1` `inspect_session_with_profile` + policy-mismatch drift test. |

---

## 2. Independent adversarial audit summary

An independent read-only adversarial audit was run via the `research` subagent on the `pro` reasoning tier (note: the VM's `invoke_subagent` tool exposes `inherit`, `flash_lite`, `flash`, and `pro` rather than explicit Neon/Carbon names; `pro` was used as the highest-tier independent auditor).

- **Initial audit verdict:** `FAIL` — surfaced 1 High finding (MCP `set_session_title` registered by `mcp_defs()` on `solve/1` would be denied by `solve_policy()`'s `default: Deny`), 3 Medium findings (`harvest_exprs_out` non-atomic read/append TOCTOU & unverified `read_session`, empty `canon.srepr_sha256` acceptance & unbounded `latex` & silent malformed `exprs_out`, and S8/S9/S11 freeze notes), and 3 Low findings (non-portable `expr_show` diagnostic note, `check_args` runtime validation note, `session_mode` vs `tool_profile` split-brain note).
- **Fixes applied:**
  1. Added explicit `ObserveLocal -> Allow` rule for `builtin::SET_SESSION_TITLE` in `solve_policy()`, paired with explicit `Deny` rules blocking `SET_SESSION_TITLE` for `host:check` and `verifier:solve` (unit-tested).
  2. Switched `harvest_exprs_out` to `ledger.read_verified_session`, verified `expr.srepr_blob == expr.srepr_sha256` in `count_existing_srepr_bytes`, completed all canonicalization before any ledger write, and committed via head-pinned `ledger.append_at_head` (race-tested).
  3. Required non-empty matching `canon.srepr_sha256`, capped stored `latex` at `MAX_EXPR_LATEX_CHARS = 64_000`, reported malformed `exprs_out` entries in `HarvestReport.skipped`, added a diagnostic `note` for non-portable `expr_show` responses, and split `solve_exprs_show.rs` to stay cleanly under 600 lines.
- **Re-audit verdict:** **`PASS WITH NOTES`** (every High/Medium/Low code finding resolved; remaining notes are downstream S8/S9/S11 implementation reminders documented in Merge notes below).

---

## 3. Deviations from the design and why

1. **Private `solve_exprs_show.rs` split beside `solve_exprs.rs`:**
   - Kept every source file comfortably under the 600-line ceiling (`solve_exprs.rs`: 502 lines, `solve_exprs_show.rs`: 180 lines). External callers import exclusively from `research_tools::solve_exprs` (re-exports `expr_show_handler`, `run_expr_show`).
2. **Return type `solve_policy() -> policy::Policy` (not `PolicyConfig`):**
   - The brief mentioned `PolicyConfig`, whereas the workspace policy type in `crates/policy/src/lib.rs`, `policy_preset.rs::research_policy()`, and `ToolProfile.policy` is `policy::Policy`. Used `policy::Policy`.
3. **No `worker` crate dependency in `crates/research-tools/Cargo.toml`:**
   - `RULES.md` forbids new dependencies and `Cargo.lock` edits. `solve_exprs.rs` defines the `CheckCanonicalizer` trait + `CanonicalizedExpr` struct; integration tests in `tests/solve_s7_exprs.rs` spawn real `python/worker.py --stdio --role check` and `--role solver` OS processes, and host crates (`desktop-server`, `cli`, `mcp-server`) that already depend on `worker` can trivially implement `CheckCanonicalizer` over `Worker::canonicalize_expr` when S12 wires the Solve loop.
4. **Desktop-server `DESKTOP_MODE_SOLVE_ENABLED = false` gate in `session_mode.rs`:**
   - Because `solve/1` is now a built profile in `research_tools::tool_profile`, `desktop-server/src/session_mode.rs` explicitly keeps `Mode::Solve` disabled for `POST /api/sessions` and `POST /api/sessions/{id}/messages` until S4/S12 wire Solve setup and loop, preventing `turn_runner.rs` from accidentally running `autonomous.rs` on a Solve session and keeping `tests/mode_shell.rs` passing untouched.

---

## 4. Decisions taken alone and assumptions

1. **Unified `expr_show` schema supporting both `solve.md` 5.1 and brief §3.2:**
   - Included optional `name` (`foo` or `foo@2`; omit to list all handles), `version`, `form` (`"latex" | "str" | "srepr"`), `include_srepr`, and `max_chars` (`1..=4000`) with `additionalProperties: false`. Both query styles work without future `def_hash` changes.
2. **Unified `solve_finish` schema supporting both `solve.md` 5.1 and brief §3.2:**
   - Required `["final_entry_id", "summary_markdown"]`, with optional `outcome` (`"finished" | "finished_open_items"`) and `open_items` (`[string]`) and `additionalProperties: false`, documented as advisory solver notes while the host computes authoritative `SolveDoneStatus` from the ledger (rule R15).
3. **Required `evidence_seq` inside `step_record.source_ref`:**
   - Matches `solve.md` 5.1 ("a `reference_lookup` whose `source_ref` (a `QuoteRef`) does not name an `evidence.quote` record of this session") and `solve_model.rs::EvidenceQuoteRecord::matches_ref` (which requires `qref.evidence_seq == Some(self.seq)`).
4. **Explicit MCP `set_session_title` rule in `solve_policy()` (`v0.5-solve`):**
   - Added explicit `ObserveLocal -> Allow` for `set_session_title` (and explicit `Deny` for `host:check` and `verifier:solve`) so `cli-route` / `mcp-server` serving `solve/1` via `mcp_defs()` works without bumping `v0.5-solve` in S15.
5. **Reusing `ledger_supersede` & `ledger_query` without editing `proof_dag.rs`:**
   - `register_solve_v1_reused_tools` registers `proof_dag` into a temporary `Registry` and copies only `ledger_supersede` and `ledger_query` (excluding `ledger_append`), honoring the brief's "Do NOT edit `proof_dag.rs`" rule with zero handler duplication.
6. **Non-portable version shadowing in `stage_exprs_in`:**
   - If `f@1` is portable and `f@2` is `portable: false`, `stage_exprs_in` omits `"f"` rather than silently staging stale `f@1` under the bare name `"f"`.
7. **Counting `MAX_EXPR_SREPR_BYTES_PER_RUN` per handle rather than per unique blob:**
   - Prevents bypassing the 2 MiB cap by saving the same large expression repeatedly under different names.
8. **Assumptions:**
   - No external network service assumptions (S7 makes zero network calls).
   - Assumed S8 (`step_record`), S9 (`check_run`), and S11 (`solve_finish`) will perform full Rust-side validation of nested schema properties (`claims`, `order`, `source_ref`, `plan`, `enum` values, `minimum`/`maximum`, and `check_run.params` size), because `tools::registry::check_args` enforces only top-level `type`, `required`, `maxLength`, and `additionalProperties: false`.

---

## 5. Merge notes

### Existing files edited (small, local)

- Commit `b9a496a`:
  - `crates/research-tools/src/policy_preset.rs`: added `SOLVE_POLICY_VERSION`, `SOLVE_CHECK_CALLER`, `SOLVE_VERIFIER_CALLER`, `solve_policy()`, and unit tests.
  - `crates/research-tools/src/tool_profile.rs`: added `SolveCheckHost`, `ProfileHostTools.solve_check` + builders, `solve_v1()`, wired `PROFILE_SOLVE_V1` in `BUILT_PROFILE_NAMES`, `lookup`, `built_profiles`, `register_profile`.
  - `crates/research-tools/src/tool_profile_tests.rs`: updated unbuilt test to `explore/1`, added `solve_v1` assertions, updated `ProfileHostTools` literals.
  - `crates/research-tools/src/lib.rs`: added `pub mod solve_defs;`, `pub mod solve_exprs;`, `mod solve_exprs_show;`, and re-exported `solve_policy`, `SOLVE_POLICY_VERSION`.
  - `crates/research-tools/tests/tool_profile_golden.rs` & `tests/golden/tool_profiles.txt`: appended `[profile solve/1]` section only and cross-checked its 12 reused hashes against `tool_defs.txt`.
- Commit `8014355`:
  - `crates/desktop-server/src/session_mode.rs`: added `DESKTOP_MODE_SOLVE_ENABLED = false` gate for `Mode::Solve`.
  - `crates/desktop-server/src/routers.rs`, `crates/desktop-server/tests/eval_set_r_support/mod.rs`, `crates/cli-route/src/core_tools.rs`, `crates/cli/src/runtime.rs`: added `solve_check: None` to `ProfileHostTools` literals (and updated `core_tools.rs` profile unit test).
  - `crates/desktop-server/tests/mode_markers_r2.rs`: added `solve_check: None`, mapped all 4 Solve tools in `tool_emitted_event_kinds()`, and added `solve_defs.rs`, `solve_exprs.rs`, `solve_exprs_show.rs` to `tool_files`.
  - `crates/agent/src/resume_profile_tests.rs`: updated `PROFILE_SOLVE_V1` / `MARKER_SOLVE_STARTED` expectations from `NotBuiltYet` to built `solve/1` and added a `solve/1` resume inspection test.

### Cross-lane integration points

| Place | What another brief / later package delivers | Integration action |
|---|---|---|
| `crates/research-tools/src/tool_profile.rs` (`ProfileHostTools::solve_check`) | Parallel lanes (`inv-v6-tasks`, `exp-x3-x4-shell`) | Any new `ProfileHostTools { ... }` struct literal in another branch should add `solve_check: None` or use `ProfileHostTools::new(...)`. When `inv-v6-tasks` adds `investigation/2`, merge `BUILT_PROFILE_NAMES` / `RESERVED_PROFILE_NAMES` and append `[profile investigation/2]` beside `[profile solve/1]` in `tool_profiles.txt`. |
| `crates/desktop-server/src/session_mode.rs` (`DESKTOP_MODE_SOLVE_ENABLED`) | S4 / S12 | Flip `DESKTOP_MODE_SOLVE_ENABLED = true` (or remove the gate) once S4/S12 wire Solve session creation, setup, and the Solve turn loop in `routes_session.rs` / `turn_runner.rs` (and update `mode_shell.rs` at that time). |
| `crates/research-tools/src/tool_profile.rs` (`SolveCheckHost`) & `solve_defs.rs` | S8 (`step_record`), S9 (`check_run`), S11 (`solve_finish`) | Drop in real handlers via `SolveCheckHost::new(check_epoch).with_step_record_handler(...).with_check_handler(...).with_solve_finish_handler(...)` (or wire them as defaults in `register_profile` for `PROFILE_SOLVE_V1`) with zero `def_hash` or `policy_version` changes. Update `mode_markers_r2.rs` `tool_emitted_event_kinds()` when S8 (`proof.entry`, `solve.plan`), S9 (`solve.check`), and S11 (`solve.done`) begin emitting domain events. |
| `crates/desktop-server/src/routers.rs`, `crates/cli/src/runtime.rs`, `crates/mcp-server/src/main.rs` | S9 / S12 / S15 | Attach a real `WorkerRole::Check` `WorkerEpoch` + `CheckCanonicalizer` adapter (wrapping `Worker::canonicalize_expr`) and call `solve_exprs::harvest_from_sympy_result` + `attach_harvest_previews` + `stage_exprs_in` around protocol-2 `sympy_eval`. |
| `crates/ledger/src/verify.rs:271-308` (`verify_event_blobs`) | Future ledger/S19 update | `verify_event_blobs` currently scans only `raw_blob`, `tool.request.args.blob`, and `tool.result.{result,error}.blob`, not `solve.expr.srepr_blob`. Adding `"solve.expr" => srepr_blob` (when non-empty) will let `Ledger::verify_all()` count expression blobs as referenced rather than `unreferenced_blobs`. |

---

## 6. Verification commands and exact results

All commands were run on `work/solve-s7-exprs`:

| Command | Result | Exact counts / notes |
|---|---|---|
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | **PASS** (exit 0) | `No touched source file is over 600 lines.` (max touched/new file: `tests/solve_s7_exprs.rs` at 559 lines) |
| `python3 -m unittest discover -s python` | **PASS** (exit 0) | **33 passed, 0 failed** (`OK`) |
| `cargo test --locked -p research-tools --test tool_def_golden --test tool_profile_golden --test solve_s7_exprs` | **PASS** (exit 0) | **10 passed, 0 failed, 0 ignored** (3 + 3 + 4; `tool_defs.txt` untouched) |
| `cargo test --locked -p research-tools --lib solve_exprs` | **PASS** (exit 0) | **8 passed, 0 failed, 0 ignored** |
| `cargo test --locked -p research-tools --lib policy_preset` | **PASS** (exit 0) | **2 passed, 0 failed, 0 ignored** |
| `cargo test --locked -p research-tools --lib tool_profile` | **PASS** (exit 0) | **7 passed, 0 failed, 0 ignored** |
| `cargo test --locked -p desktop-server --test mode_shell --test mode_markers_r2 --test eval_set_r` | **PASS** (exit 0) | **12 passed, 0 failed, 0 ignored** (4 + 3 + 5) |
| `cargo test --locked -p agent --test resume_loop` | **PASS** (exit 0) | **6 passed, 0 failed, 0 ignored** |
| `cargo test --locked -p agent --lib resume` | **PASS** (exit 0) | **12 passed, 0 failed, 0 ignored** |
| `cargo test --locked -p cli-route --lib core_tools` | **PASS** (exit 0) | **5 passed, 0 failed, 0 ignored** |
| `cargo test --locked --workspace --no-fail-fast` | **PASS** (exit 0) | **1003 passed, 0 failed, 2 ignored** (2 pre-existing live-tier tests in `agent/tests/eval_harness.rs`) |
| `RE_SEARCH_PYTHON="$(command -v python3)" cargo test --locked -p worker -p export --no-fail-fast` | **PASS** (exit 0) | **26 passed, 0 failed, 0 ignored** (17 `export` + 9 `worker`) |
| `cargo clippy --locked -p research-tools --lib --test tool_def_golden --test tool_profile_golden --test solve_s7_exprs -- -D warnings` | **PASS** (exit 0) | 0 warnings / 0 errors |
| `cargo clippy --locked -p agent --lib --test resume_loop -- -D warnings` | **PASS** (exit 0) | 0 warnings / 0 errors |
| `cargo clippy --locked -p cli-route --all-targets -- -D warnings` | **PASS** (exit 0) | 0 warnings / 0 errors |
| `cargo clippy --locked -p cli --bins -- -D warnings -A clippy::manual_unwrap_or` | **PASS** (exit 0) | 0 warnings / 0 errors |
| `cargo clippy --locked -p desktop-server --test mode_shell --test mode_markers_r2 --test eval_set_r -- -D warnings -A clippy::manual_unwrap_or` | **PASS** (exit 0) | 0 warnings / 0 errors |

### Pre-existing clippy diagnostics on Linux (`rustc 1.100.0`, untouched files)

As already documented on the base commit in `RULES.md` and `reports/{f1-tool-profiles,s2-solve-model,solve-evidence-labels}.md`, running `cargo clippy --workspace --all-targets -- -D warnings` on Linux hits pre-existing diagnostics exclusively in untouched files outside S7's scope:
- `crates/desktop-server/src/evidence.rs:744` (`clippy::manual_unwrap_or`, file > 600 lines)
- `crates/research-tools/tests/sandbox_and_fs.rs:6,9` (`unused_imports` on non-Windows)
- `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` (`unused_imports` on non-Windows)
- `crates/agent/tests/eval_support/manifest.rs:289` (`clippy::match_like_matches_macro`) and `crates/agent/tests/eval_support/sheet.rs:102` (`clippy::unnecessary_map_or`)

---

## 7. What could not be compiled or run on Linux

- All `#[cfg(windows)]` code paths across the workspace: `AppContainerSandbox`, `JobObjectSandbox`, Windows pipe transport in `crates/worker/src/{win,appcontainer,stage}.rs`, `desktop-server/src/routers.rs:36-65`, `cli/src/runtime.rs:48-71`, `desktop-app` WebView2 runtime, and Windows-only tests (`crates/worker/tests/{appcontainer_sandbox,python_worker}.rs`, `crates/cli/tests/contained_router.rs`, Windows branches of `sandbox_and_fs.rs` and `chatgpt-auth`).
- All S7 production code (`solve_defs.rs`, `solve_exprs.rs`, `solve_exprs_show.rs`, `policy_preset.rs`, `tool_profile.rs`) is platform-neutral Rust with no Unix-only paths or shell-outs; cross-process R5/R6/restart tests exercised real `python/worker.py --stdio --role solver` and `--role check` processes on Linux.

---

## 8. Bugs / issues noticed but not fixed (out of scope)

1. `crates/ledger/src/verify.rs:271-308` — `verify_event_blobs` does not yet recognize `solve.expr.srepr_blob`, so `Ledger::verify_all()` counts `srepr_blob` files under `unreferenced_blobs` (while `load_handle_srepr` and `count_existing_srepr_bytes` verify every blob SHA-256 directly on read). Left untouched as outside S7's owned files.
2. `crates/desktop-server/src/routers.rs:79` — `build_desktop_router` calls `research_tools::tool_profile::profile_for_session_events` directly rather than `crate::session_mode::profile_for_session`. Live turns are still gated earlier in `turn_runner.rs:30,273` via `session_mode::profile_for_session`; S12 should unify this when enabling Solve turns.
3. `crates/research-tools/src/solve_state.rs:440` — `parse_tool_result` reads `exprs_out` as `string_vec(meta, "exprs_out")` (array of strings), whereas protocol-2 `python/worker.py:322` emits `exprs_out` as a JSON object `{ name: raw_srepr }` (so `SolveReceipt.exprs_out` stays empty until updated in S8/S12 or populated via harvested handle names). Left untouched because `solve_state.rs` is forbidden to edit in S7.
4. Pre-existing Linux clippy diagnostics listed in Section 6.

---

## 9. Design contradictions or ambiguities found

1. **`expr_show` and `solve_finish` parameter lists (`solve.md` 5.1 vs brief §3.2):**
   - `solve.md` 5.1 lists `expr_show {name?, version?, form, max_chars}` and `solve_finish {final_entry_id, summary_markdown}`, while brief §3.2 names `expr_show` (`name` or `name@version`, `include_srepr`) and `solve_finish` (`outcome`, `final_entry_id`, `summary_markdown`, `open_items`). Reconciled by supporting both parameter sets cleanly in the frozen schemas (`outcome` and `open_items` optional and advisory on `solve_finish`; `name`/`version`/`form`/`include_srepr`/`max_chars` all optional on `expr_show`) so neither callers nor S11 require schema changes.
2. **MCP `set_session_title` vs `solve_policy()` default `Deny`:**
   - `ToolProfile::mcp_defs()` adds `set_session_title` for `core.exe` over MCP, while `solve.md` 5.1 lists only the 16 in-core registry tools. Resolved by keeping `solve/1`'s in-core `defs` strictly at 16 tools and adding an explicit `ObserveLocal -> Allow` rule for `set_session_title` in `solve_policy()` (explicitly denied for `host:check` and `verifier:solve`), ensuring MCP Solve routing will not break when S15 lands.
