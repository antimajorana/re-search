# Brief draft-solve-s15-cli-parity — Solve CLI route parity (S15)

**Draft status:** Planning brief (do not launch until start conditions merge AND coordinate merge order on `cli_turn.rs` with Explore **X18** in Group 3!).
**Package:** S15 (Solve parity on subscription-CLI routes: MCP `core.exe`, `cli-route`, `cli_turn.rs`, ledger-derived result sheet, plus Checkpoint 2 API-transport review after S14).
**Governing design:** `00-overview.md` 2.2 (F1), 3.1 (row 12), 8.2 (decision 1); `solve.md` 0.1, 0.3, 5.1, 5.4 (CLI route honesty note), 6 (R1, R11, R12, R15, R18, R20, R22), 7.2, 9 (row S15), 12 (risks 13, 14), 13 (D16); `agents/RULES.md`.
**Read reports first:** `reports/f1-tool-profiles.md`, `reports/s2-solve-model.md`, and (once merged) `reports/q-f11-x1-s3-s6.md`, `reports/f5-f7-mode-shell.md`, `reports/solve-s7-s8-s9-s11.md`, `reports/solve-s12-loop.md`.

## 1. Start conditions & cross-group conflict gate

- **Required merges on `origin/gemini/work/fixed`:**
  - Already merged on base `832eab5bf`: F0, F1 (`mcp-server` profile lookup + `cli-route/src/core_tools.rs::core_tool_set_for_profile`), F4, F9, S2.
  - **Hard merge gates for S15 Checkpoint 1 (core CLI parity):** `q-f11-x1-s3-s6` (S3 `tools::Gate` + S6 check process), `f5-f7-mode-shell` (F5 `session_mode.rs`), and `solve-s7-s8-s9-s11` (**S11**: **`research_tools::solve_gates::SolveGate`**, `solve_done`, `solve_checks`, complete frozen `solve/1` profile + `solve_policy()`), plus `solve-s12-loop` (`resume_brief.rs` if CLI Solve turns build from resume brief).
  - **Hard prerequisite for S15 Checkpoint 2 (CLI-route verifier review):** `solve-s14-s20-s21-review-blind-migrate` (**S14** `verifier_loop` + `agent_models.rs` Solve API verifier resolver). Ship CP1 right after S11/S12 (showing clear D16 notice when no API verifier is configured) and wire CP2 review once S14 merges, without violating `agent_models.rs`'s restriction against CLI-only helper roles!
- **CRITICAL CROSS-GROUP SEAM — EXPLORE X18 (Group 3):**
  - Explore **X18** ALSO edits `crates/desktop-server/src/cli_turn.rs`, `crates/cli-route/src/core_tools.rs`, and `crates/mcp-server/src/main.rs`.
  - **Do NOT run S15 and X18 concurrently on the same base!** Whichever merges first MUST use the agreed split filenames **`crates/desktop-server/src/cli_turn_setup.rs`** + **`cli_turn_tests.rs`** (zero-behaviour-change split of 626-line `cli_turn.rs`), make `cli_turn.rs` pass `CORE_TOOL_PROFILE` mode-neutrally, stop `cli_turn.rs:407-419` from injecting Research instructions for non-Research modes, and fix `core_tools::core_tool_ids_for` + `mcp-server/src/main.rs:184`. The second lane merges cleanly on top.

## 2. Interfaces: base-verified vs future-merge

**Verified on base `832eab5bf`:**
- **`crates/desktop-server/src/cli_turn.rs` is 626 lines (OVER THE 600-LINE LIMIT!):**
  - `lines 407-419`: currently injects Research-specific `search_formal_library` / `sympy_eval` instructions into the prompt — MUST be gated strictly to `Mode::Research` so Solve and Explore CLI turns NEVER receive Research/Lean instructions!
  - `line 454`: currently hardcodes `let tools = core_tool_set(Some(Arc::clone(&research)));` and never sets `CORE_TOOL_PROFILE` in `core.env`!
- **`crates/cli-route/src/core_tools.rs` (188 lines):**
  - Has `register_core_tools_for_profile` (line 26) and `core_tool_set_for_profile` (line 106).
  - **Verified base gap (`lines 130-134`):** `core_tool_ids_for(core_env)` ignores `CORE_TOOL_PROFILE` — must honour `CORE_TOOL_PROFILE` when present (returning clean fallback without panicking on invalid/unknown profile names, and tolerating `NoWorker` cleanly during `register_profile(&solve_v1)` tool-ID enumeration so Antigravity `enabledTools` advertises `solve/1`'s exact MCP tool list including `set_session_title` via `solve/1.mcp_defs()`!).
- **`crates/mcp-server/src/main.rs` (317 lines) & `crates/mcp-server/Cargo.toml` (24 lines):**
  - **CRITICAL BASE BUG (`main.rs:184`):** `let use_research = workspace_root.is_some() || sources_exist || enable_flag;` — setting `CORE_TOOL_PROFILE=solve/1` WITHOUT `CORE_ENABLE_RESEARCH_TOOLS=1` or a workspace skips the profile branch completely! Fix line 184 to `let use_profile = workspace_root.is_some() || sources_exist || enable_flag || env("CORE_TOOL_PROFILE").is_some();`!
  - **CRITICAL CRATE BOUNDARY (`mcp-server/Cargo.toml`):** `mcp-server` depends on `research-tools` and does **NOT** depend on `agent` (zero `Cargo.lock` edits!). Import `SolveGate` from **`research_tools::solve_gates`** (placed there by S11 per C8!), NEVER from `agent`!
  - Calls `router.call(&caller, &name, &args)` at line 101 (traversing `Router::try_call`), so attaching `research_tools::solve_gates::SolveGate` + S9 auto-check post-call hook + check Python worker enforces G1/G4/G5/G6/G7 over MCP!
- `crates/research-tools/src/tool_profile.rs` (329 lines): `ToolProfile::mcp_defs()` adds `set_session_title` in id order for `core.exe` (`solve/1.mcp_defs()` = 16 `solve/1` tools + `set_session_title` = 17 MCP tools).

**Future-merge interfaces to verify before coding:**
- From **S3** (`q-f11-x1-s3-s6`): `Router::with_gate` / post-call hook attachment API on `tools::Router`.
- From **S6 / S7 / S9 / S11:** check worker builder, `ProfileHostTools` check-process binding (`NoWorker` safe during enumeration), `research_tools::solve_gates::SolveGate`, and `solve_checks` post-call auto-check hook.
- From **F5 / X18:** `session_mode.rs`; check if X18 already split `cli_turn.rs` into `cli_turn_setup.rs` + `cli_turn_tests.rs` and wired mode-neutral `CORE_TOOL_PROFILE` — reuse directly if present.

## 3. Smallest implementation checkpoints (strict order)

1. **Checkpoint S15a — MANDATORY ZERO-BEHAVIOUR-CHANGE SPLIT OF `cli_turn.rs` (626 -> `< 460` lines):**
   - If not already split by X18, move `CliRouteKind`, `wants_cli_route`, `pick_model`, `resolve_core_exe`, and `core_env` into **`crates/desktop-server/src/cli_turn_setup.rs`** and move unit tests (`lines 540-625`) into **`crates/desktop-server/src/cli_turn_tests.rs`** (exact filenames agreed with X18).
   - Zero behaviour change; run `cargo test -p desktop-server` and `check_file_length.py`. Commit alone.
2. **Checkpoint S15b — Profile-aware `core_tools.rs` & `cli_turn.rs` (mode-neutral seam):**
   - Update `crates/cli-route/src/core_tools.rs::core_tool_ids_for`: read `CORE_TOOL_PROFILE` first -> call `core_tool_set_for_profile` (with `NoWorker` handled cleanly in `register_profile(&solve_v1)`, returning a clean fallback without panicking if invalid/unbuilt), else fall back to `CORE_ENABLE_RESEARCH_TOOLS` for legacy callers. Confirm `set_session_title` is included via `solve/1.mcp_defs()`.
   - Update `cli_turn.rs`: detect session mode (`session_mode_from_events`), gate Research `search_formal_library`/`sympy_eval` prompt injection (`lines 407-419`) strictly to `Mode::Research`, pass `CORE_TOOL_PROFILE` (+ check-worker env vars for Solve) in `core.env`, and call `core_tool_set_for_profile` so `system/init` checks the exact `solve/1` tool list and `solve_policy()` (`v0.5-solve`).
   - For Solve CLI turns, build the prompt from Solve setup + `resume_brief` (not Research replay) and record `"tool_profile": "solve/1"` on `session.start` / `session.resume`.
3. **Checkpoint S15c — `mcp-server` (`core.exe`) Solve gates, check process & auto-checks:**
   - Fix `crates/mcp-server/src/main.rs:184` so `env("CORE_TOOL_PROFILE").is_some()` activates the profile branch even without `CORE_ENABLE_RESEARCH_TOOLS=1` or a workspace.
   - Extract `crates/mcp-server/src/solve_wire.rs` if `main.rs` approaches 520 lines: when resolved profile is `solve/1`, configure the separate check Python worker, register `solve/1` with solver + check `WorkerEpoch` bindings, attach **`research_tools::solve_gates::SolveGate`** (G1, G4, G5, G6, G7) and S9's post-call auto-check hook (`host:check` through `TcpApprover` — zero auto-approve!), and keep `research/1` / `v0.2` behaviour byte-identical.
4. **Checkpoint S15d — Ledger-driven result sheet & CLI review (CP2 after S14):**
   - Verify `solve_view.rs` builds the result sheet strictly from the ledger (`solve.done`, `SolveState`), so unchecked CLI final prose still renders `"Not done: N steps unchecked"` (`solve.md` 5.4, 12 risk 13).
   - **Review on CLI routes (D16, respects `agent_models.rs` helper-role rule):** Once S14 merges, invoke `verifier_loop` through a configured in-core API verifier (`roles.verifier` — never violating `agent_models.rs`'s rule against CLI-only helper roles); if an explicit second-CLI review route is configured separately at the CLI-turn supervisor level, run it isolated; if neither is configured, display a plain setup-card notice explaining why automatic review is unavailable without hiding Solve from CLI users (D16).
   - Keep Antigravity's confinement caveat and acknowledgement check (`crate::antigravity::require_acknowledged`) intact.

## 4. Owned files

- **New:**
  - `crates/desktop-server/src/cli_turn_setup.rs`, `cli_turn_tests.rs` (shared split filenames with X18)
  - Optional: `crates/mcp-server/src/solve_wire.rs` (if needed to keep `mcp-server/src/main.rs <= 550` lines)
  - Integration test: `crates/mcp-server/tests/solve_mcp_gates.rs` (or `crates/desktop-server/tests/solve_cli_parity.rs`)
- **Small localized edits (`<= 600` lines each):**
  - `crates/desktop-server/src/cli_turn.rs` (626 -> `< 460` after S15a)
  - `crates/cli-route/src/core_tools.rs` (188)
  - `crates/mcp-server/src/main.rs` (317)
  - `crates/desktop-server/src/lib.rs` (module export if needed)

## 5. Shared registration seams & Do-not-edit files

- **Cross-group seam:** Serialize merges on `cli_turn.rs` (`cli_turn_setup.rs` / `cli_turn_tests.rs`), `core_tools.rs`, and `mcp-server/src/main.rs` strictly with Explore **X18** (Group 3).
- **Do NOT edit:** `Cargo.lock`, `crates/mcp-server/Cargo.toml` (NO `agent` dependency!), `crates/research-tools/src/tool_profile.rs`, `tests/golden/tool_defs.txt`, `tests/golden/tool_profiles.txt`, `crates/desktop-server/src/http_srv.rs` (702), `crates/agent/src/autonomous.rs`.

## 6. Tests through the entry point

- **MCP / CLI parity integration tests (zero real vendor CLI, zero real network):**
  - **Rule R11 (gates bind MCP `core.exe`):** Drive `mcp-server`'s `Router` / stdio MCP `call_tool` on a `solve/1` session with `CORE_TOOL_PROFILE=solve/1` and NO `CORE_ENABLE_RESEARCH_TOOLS` / workspace (verifying the `main.rs:184` fix!): 4th uninterpreted `sympy_eval` hits `GATE G1` + `guardrail.blocked`; 3 bounces trigger G5; `solve_finish` on Failed/Stale closure hits G7.
  - **Auto-checks & approvals over MCP:** `step_record` with a claim triggers `host:check` through `Router::try_call` + `TcpApprover` (unapproved -> `not_run`, approved -> `Verified`/`Checked numerically`).
  - **Prompt hygiene & honesty:** Solve CLI prompt contains zero Research `search_formal_library` instructions (`cli_turn.rs:407-419` gated); scripted fake CLI printing `"Final answer is verified!"` without `solve_finish` still shows `"Not done"` / `Unchecked` from ledger (R15, R22).
  - **Profile & Antigravity `enabledTools`:** `core_tool_ids_for` with `CORE_TOOL_PROFILE=solve/1` returns exact 17 `solve/1.mcp_defs()` IDs (16 `solve/1` + `set_session_title`, excluding `doc_ingest`, `ledger_append`, `lean_check`, `fs_write`) with `NoWorker`, and falls back cleanly without panicking on invalid `CORE_TOOL_PROFILE`.
  - **Regression:** All existing `cli-route`, `mcp-server`, `desktop-server` CLI-turn, `eval_set_r`, `tool_def_golden`, `tool_profile_golden` tests pass untouched.
- **Guard rails:** `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`, zero real vendor CLI executions, zero network calls.

## 7. Document conflicts & resolutions

1. **`solve.md` 9 S11 (`crates/agent/src/solve_gates.rs`) vs `crates/mcp-server/Cargo.toml` (C8):** `mcp-server` depends on `research-tools`, NOT `agent`. **Resolution:** S11 places `SolveGate` in `crates/research-tools/src/solve_gates.rs`; S15 imports `research_tools::solve_gates::SolveGate` directly with zero `Cargo.lock` changes.
2. **`mcp-server/src/main.rs:184` (`use_research` ignores `CORE_TOOL_PROFILE`) & `core_tools.rs:130-134` (`core_tool_ids_for` ignores `CORE_TOOL_PROFILE`):** Fixed in S15b/S15c so `CORE_TOOL_PROFILE=solve/1` activates without `CORE_ENABLE_RESEARCH_TOOLS` or a workspace.
3. **`cli_turn.rs` line count (`solve.md` cites 628; base `832eab5bf` is 626) + Explore X18 overlap:** Both S15 and X18 serialize on `cli_turn.rs` using identical split filenames (`cli_turn_setup.rs`, `cli_turn_tests.rs`) and mode-neutral `CORE_TOOL_PROFILE` propagation.
4. **`solve.md` 9 S15 ("review through an API transport or a second CLI call") vs `agent_models.rs:115-121` (helper roles forbid CLI providers):** S15 CP1 ships core CLI parity right after S11/S12 with D16 setup notice; CP2 wires in-core API verifier review after S14 merges without weakening `agent_models.rs`'s helper-role guard.

## 8. Out of scope & hand-back

- **Out of scope:** Explore CLI parity (X18, Group 3), adding new tools to `solve/1`, modifying vendor CLI binaries or confinement rules, Lean/S16 (cut), Cadabra2/FORM/S22 (deferred).
- **Hand-back:** Write `docs/design/modes-2026-11/reports/solve-s15-cli-parity.md` (recording `cli_turn.rs` line counts before/after split, `main.rs:184` & `core_tool_ids_for` fixes, Windows-only `AppContainerSandbox` paths uncompiled on Linux, X18 merge notes, and exact MCP/CLI test counts) and push only to `gemini/modes/solve-s15-cli-parity`.
