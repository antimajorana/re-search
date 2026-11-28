# Report: `f1-tool-profiles` (package F1 — tool profiles)

- **Base commit:** `e1517477cbceba0a6de6e4a143c34fe3b8b8411d` (`origin/gemini/work/fixed`)
- **Work branch:** `work/f1-tool-profiles` (pushed to `gemini/modes/f1-tool-profiles`)
- **Toolchain on this machine:** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `cargo 1.100.0 (3acc79c0c 2026-11-05)`, `Python 3.12.16` (SymPy 1.14.0, NumPy 2.5.5, mpmath 1.3.0), `Node v22.23.5` on Linux x86_64
- **Unchanged guard rails:** `Cargo.lock` untouched (0 new dependencies); `crates/research-tools/tests/golden/tool_defs.txt` untouched and `tool_def_golden` passes; no tool definition added or modified; `crates/policy` and `crates/ledger` remain free of `tokio`, `rmcp`, and `windows-sys`; `crates/desktop-server/src/{routes_session,turn_runner,http_srv}.rs`, `crates/agent/src/runner.rs`, `crates/agent/src/resume_drift_tests.rs`, `crates/agent/tests/resume_loop.rs`, and `crates/agent/src/paper_read_tests.rs` are untouched; `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` passes (`No touched source file is over 600 lines.`).

---

## 1. What was implemented (package F1)

### Router profile metadata (`crates/tools`)
- `crates/tools/src/router.rs` (504 lines, +13): adds optional `tool_profile: Option<String>` on `Router` with `Router::with_tool_profile(...)` and `Router::tool_profile() -> Option<&str>`. `Router::new` signature is unchanged and defaults to `None`. This allows profile-built routers (`desktop-server/src/routers.rs`, `cli/src/runtime.rs`, `mcp-server/src/main.rs`) to carry their profile id into `autonomous.rs`, `guardrails.rs`, and `resume.rs` without editing `turn_runner.rs` or breaking `AgentConfig` / `ResumeConfig` literals.

### Tool profile data model, lookup, registration, and golden pin (`crates/research-tools`)
- `crates/research-tools/src/tool_profile.rs` (329 lines, new):
  - Single-place named constants referencing `00-overview.md` 2.2, 3.1 row 3, 5.2, and 8.2 decision 1: `TOOL_PROFILE_FIELD` (`"tool_profile"`), `PROFILE_RESEARCH_V1` (`"research/1"`), `PROFILE_INVESTIGATION_V1` (`"investigation/1"`), `PROFILE_INVESTIGATION_V2` (`"investigation/2"`), `PROFILE_EXPLORE_V1` (`"explore/1"`), `PROFILE_SOLVE_V1` (`"solve/1"`), `BUILT_PROFILE_NAMES`, `RESERVED_PROFILE_NAMES`, `MARKER_INVESTIGATION_STARTED`, `MARKER_INVESTIGATION_HANDOFF`, `MARKER_IDEA_STARTED`, `MARKER_SOLVE_STARTED`, and `LEGACY_MAY_GAIN`.
  - Data struct `ToolProfile { name, defs, may_gain, earlier_defs, policy }` with helpers `def_for`, `is_canonical_def`, `allows_gain` (requiring `ReadWorkspace`), `earlier_defs_for`, and `mcp_defs` (adding `set_session_title` in id order for MCP `core.exe`).
  - Built profiles `research_v1()` (`"research/1"`) and `investigation_v1()` (`"investigation/1"`): each carries the exact sorted 22-tool list (`all_research_tool_defs()` + `ledger_note` + `sympy_eval`), `research_policy()` (`v0.4-research`), the 7 historical `ReadWorkspace` `may_gain` ids, and the 2 historical `earlier_defs` cases (current `record_paper_map` without `answer_node_ids`, plus the `7c36a0e` pre-pointer recorder schemas).
  - `lookup(name)` returns `Ok(ToolProfile)` for `research/1` and `investigation/1`, `Err(ProfileError::NotBuiltYet { name })` (`"tool profile `<name>`: profile not built yet"`) for `investigation/2`, `explore/1`, and `solve/1` (never panicking, adding no tools), and `Err(ProfileError::Unknown { name })` otherwise.
  - Mode-marker detection (`session_mode_from_events`, reusing F2's `paper_key::Mode`, `legacy_profile_name_for_events`, `profile_for_session_events`): latest of `investigation.started` (`Investigate`), `investigation.handoff` (`Research`), `idea.started` (`Explore`), `solve.started` (`Solve`); none means `Research`.
  - `register_profile(&profile, &mut registry, env, host)` registers a profile's tools and fail-closed self-checks `defs_match_exact` against `profile.defs` (or `profile.mcp_defs()` when `session_title_handler` is supplied), returning `profile.policy`.
- `crates/research-tools/src/tool_profile_tests.rs` (200 lines, new): unit tests for lookup, reserved/unknown errors, id ordering, `may_gain` effect clamping, `earlier_defs` golden hashes, latest-marker mode detection, fail-closed event profile resolution, and `register_profile`.
- `crates/research-tools/src/lib.rs` (275 lines, +1): exports `pub mod tool_profile;`.
- `crates/research-tools/tests/tool_profile_golden.rs` (145 lines, new) and `crates/research-tools/tests/golden/tool_profiles.txt` (79 lines, new): golden file pinning one section per built profile (`research/1`, `investigation/1`: policy version, every `(position, id, def_hash)`, every `may_gain`, every `earlier_def`), cross-checking every hash against the untouched `tests/golden/tool_defs.txt`, and verifying `NotBuiltYet` for the three reserved profiles.

### Data-driven profile resume (`crates/agent`)
- `crates/agent/src/resume_profile.rs` (310 lines, new):
  - `ResolvedSessionProfile { profile, explicit }` and `resolve_session_profile(session, events)`: reads `session.start["tool_profile"]` when present (failing closed with `Drift` on non-string, unbuilt, or unknown values), falls back to `legacy_profile_name_for_events(events)` when absent, and enforces rule R2 (any `session.resume` carrying a different `tool_profile` is rejected as `Drift`).
  - `check_tool_defs_drift_for_profile(session, payload, expected_defs, profile)`: pure data-driven drift check using `profile.allows_gain`, `profile.is_canonical_def`, and `profile.earlier_defs_for`.
  - `verify_profile_policy_and_defs`: enforces that policy belongs to the profile (`3.1 row 12`: when `tool_profile` is explicit, both `session.start["policy_version"]` and the active router's policy must match `profile.policy.version`, and the active router's defs must match `profile.defs`), enforces active-router profile agreement (rule R2), checks `session.resume["policy_version"]` if present, and checks both `session.start` and the latest `session.resume` `tool_defs`.
  - `inspect_session_with_profile(ledger, session)`: builds expected policy and defs directly from the resolved profile and verifies chain, blobs, policy, defs, and history rehydration.
  - `verify_router_for_resume`, `effective_router_profile`, and `insert_tool_profile`: shared helpers for `resume.rs`, `autonomous.rs`, and `guardrails.rs`.
- `crates/agent/src/resume_profile_tests.rs` (435 lines, new):
  - Retains the verbatim pre-F1 `check_tool_defs_drift` from base commit `e1517477` as `legacy_check_tool_defs_drift_oracle` and runs a 24-case table-driven equivalence test comparing the oracle against `check_tool_defs_drift_for_profile` for both `research/1` and `investigation/1`.
  - Explicitly tests the one stricter fail-closed boundary (`non_canonical_map_without_answer_index_is_stricter_than_legacy_oracle`, documented in Section 2).
  - Tests explicit vs legacy profile resolution, reserved/unbuilt profiles, unbuilt mode markers, non-string `tool_profile`, rule R2 profile-switch rejection, active-router profile mismatch, and policy-belongs-to-profile enforcement.
- `crates/agent/src/resume.rs` (535 lines, net -44): adds `tool_profile: String` to `ResumeInspection`, replaces hard-coded policy/drift logic with `resume_profile::verify_profile_policy_and_defs`, keeps a `#[cfg(test)]` wrapper `check_tool_defs_drift` so `resume_drift_tests.rs` runs untouched against `check_tool_defs_drift_for_profile`, and updates `resume_agent_loop` to verify the active router profile and record `tool_profile` on `session.resume`.
- `crates/agent/src/lib.rs` (99 lines, +5): exports `pub mod resume_profile;` and re-exports `check_tool_defs_drift_for_profile`, `inspect_session_with_profile`, `resolve_session_profile`, and `ResolvedSessionProfile`.
- `crates/agent/src/autonomous.rs` (573 lines, net -2): records `tool_profile` on `session.start` and `session.resume` via `insert_tool_profile` and verifies resume via `verify_router_for_resume`.
- `crates/agent/src/guardrails.rs` (491 lines, net 0): records `tool_profile` on `session.start` in `run_research_loop` via `insert_tool_profile` (also reading `last_seq` from existing events if any).

### Profile-built routers (`desktop-server`, `cli-route`, `mcp-server`, `cli`)
- `crates/desktop-server/src/routers.rs` (150 lines, +22): adds `build_desktop_router_for_profile` using `tool_profile::lookup` + `tool_profile::register_profile` + `.with_tool_profile(profile.name)`. `build_desktop_router` keeps its exact signature (for untouched `turn_runner.rs` and `routes_tools.rs`), resolves the session's profile via `profile_for_session_events(&existing)`, and delegates to `build_desktop_router_for_profile`.
- `crates/cli-route/src/core_tools.rs` (188 lines, +62): adds `register_core_tools_for_profile` and `core_tool_set_for_profile` backed by `tool_profile::register_profile` (with MCP `set_session_title`); routes `register_core_tools(..., Some(env))` and `core_tool_set(Some(env))` through `PROFILE_RESEARCH_V1`; adds a unit test covering built and reserved profiles.
- `crates/mcp-server/src/main.rs` (317 lines, +28): when research tools are enabled, resolves the profile from `CORE_TOOL_PROFILE` (if set) or `profile_for_session_events(&events)`, registers via `cli_route::core_tools::register_core_tools_for_profile`, and attaches `.with_tool_profile(profile_name)` to the `Router`.
- `crates/cli/src/runtime.rs` (202 lines, +37): adds `build_router_for_profile`; updates `build_research_router` to resolve the session's profile from `profile_for_session_events(&existing)`, register via `tool_profile::register_profile`, and attach `.with_tool_profile(profile.name)`. Minimal v0 2-tool `build_in_process_router` (`v0.2` policy) is unchanged.

### Evaluation Set R integration tests (`crates/desktop-server`)
- `crates/desktop-server/tests/eval_set_r.rs` (449 lines, new) and `crates/desktop-server/tests/eval_set_r_support/mod.rs` (380 lines, new): hand-builds all four Set R legacy session types using the real writers (`Ledger`, `ingest_bytes`, `Router::try_call`, `export::fork::research_from_paper`) on a self-written synthetic paper fixture and verifies resume (leg 2 and leg 3), rule R2 mismatch rejection, content preservation, and `Ledger::verify_all().unreferenced_blobs == 0`:
  1. `set_r_1_research_session_resumes_under_research_v1`: pre-F1 Research session with real `ledger_append` Proof DAG entry, plus earlier pre-F1 Research session missing all 7 historical `ReadWorkspace` tools.
  2. `set_r_2_investigation_before_pointer_citations_resumes_under_investigation_v1`: pre-pointer (`7c36a0e`) Investigation session (both `GOLDEN_MAP_7C36A0E` and `GOLDEN_MAP_NO_ANSWER_INDEX_7C36A0E`, omitting `read_paper_lines`) with real `ingest_bytes`.
  3. `set_r_3_investigation_current_resumes_under_investigation_v1`: current pre-F1 Investigation session using real `ingest_bytes` and real `Router::try_call(..., "record_paper_map", ...)` producing a real verified `investigation.map`.
  4. `set_r_4_handoff_child_resumes_under_research_v1`: hand-off child created via real `export::fork::research_from_paper` from a real Investigation session, verifying both pre-F1 resume under `research/1` (rejecting `investigation/1`) and post-F1 start/resume recording `"tool_profile": "research/1"`.
  5. `guardrails_run_research_loop_records_tool_profile_and_resumes`: verifies `guardrails::run_research_loop` records `"tool_profile": "research/1"` and resumes via `resume_agent_loop`.

---

## 2. Deviations, behaviour differences, decisions, and assumptions

### Behaviour differences vs pre-F1 `resume.rs` (listed exhaustively)
1. **Stricter fail-closed check on `record_paper_map` without `answer_node_ids`:**
   - Pre-F1 `check_tool_defs_drift` cloned whatever `exp` was passed for `record_paper_map` and removed `answer_node_ids` on the fly, so a hypothetical non-canonical `exp` (e.g. with a modified description) would still match a recorded hash of that same non-canonical `exp` minus `answer_node_ids`.
   - With data-driven `earlier_defs`, `record_paper_map`'s pre-`answer_node_ids` `ToolDef` is stored as frozen data in `profile.earlier_defs["record_paper_map"]` and is only accepted when `exp` matches `profile`'s canonical `record_paper_map` (`profile.is_canonical_def(exp)`). For every real released build this behaves identically; for mutated/non-canonical definitions it is strictly more fail-closed (verified in `non_canonical_map_without_answer_index_is_stricter_than_legacy_oracle`).
2. **Additional fail-closed checks on `session.resume["policy_version"]` and `session.resume["tool_profile"]`:**
   - Pre-F1 `verify_and_read` checked `policy_version` only on `session.start`. F1 also checks `policy_version` on the latest `session.resume` (when present) and rejects any `session.resume` whose `tool_profile` differs from `session.start`'s resolved profile (rule R2) or is non-string.
3. **Explicit `tool_profile` binds exact `profile.defs` and `profile.policy.version`:**
   - For sessions started after F1 that record `"tool_profile"`, resume fails closed if either `session.start["policy_version"]` or the caller's active policy differs from `profile.policy.version`, if the active router's defs differ from `profile.defs`, or if `router.tool_profile()` differs from the session's resolved profile.
   - Pre-F1 sessions without `"tool_profile"` keep resuming identically to today (including minimal `v0.2` 2-tool CLI sessions and partial-registry unit tests such as `paper_read_tests.rs`).
4. **Unbuilt mode markers now fail closed as `Drift`:**
   - A session with no `tool_profile` whose latest mode marker is `idea.started` or `solve.started` resolves to `explore/1` or `solve/1`, which returns `ProfileError::NotBuiltYet` -> `ResumeRejectKind::Drift` (`"profile not built yet"`), instead of being treated as a Research session.

### Decisions taken alone (and why)
1. **Both `research/1` and `investigation/1` carry the 7 historical `may_gain` ids and the 2 historical `earlier_defs` cases:**
   - *Why:* Today's codebase registers `record_paper_map`, `record_paper_summary`, `record_paper_branch`, `record_paper_conventions`, `search_formal_library`, `search_paper_passages`, and `read_paper_lines` in `register_research_tools` for *all* sessions (Research and Investigation alike). Furthermore, `crates/agent/tests/resume_loop.rs:485-502` (which must pass untouched) starts a session with *no* `investigation.started` marker and asserts that gaining 6 of those 7 tools succeeds. Pre-pointer (`7c36a0e`) Research sessions and hand-off children likewise recorded `7c36a0e` hashes for those 4 recorders. Keeping the 7 already-present historical tools in `research/1.may_gain` and the 4 recorders' `7c36a0e`/no-`answer_node_ids` schemas in `research/1.earlier_defs` guarantees every pre-F1 Research session resumes identically while rejecting every new/unknown tool (such as `read_map_node` or `unrelated_tool`). When V6 adds `investigation/2`, *only* `investigation/1` will add `investigation/2`'s new tool ids to `may_gain`, keeping `research/1` frozen.
2. **Leaving `crates/agent/src/runner.rs` untouched (no `tool_profile` emitted by `run_agent_loop`):**
   - *Why:* `runner.rs` is 626 lines (already over the 600-line ceiling; `RULES.md` forbids touching files already over 600 lines), was not in the brief's edit list, and `run_agent_loop` runs only the v0 2-tool `v0.2` builtin route (`ledger_note` + `sympy_eval`), which has no mode profile in `00-overview.md` 2.2. Research/Investigation sessions go through `autonomous.rs` or `guardrails.rs`.
3. **Carrying `tool_profile` on `tools::Router` instead of changing `AgentConfig` / `ResumeConfig`:**
   - *Why:* `crates/desktop-server/src/turn_runner.rs` may not be edited in this brief and constructs `AgentConfig` and `ResumeConfig` as struct literals. Putting `tool_profile: Option<String>` on `Router` lets `desktop-server/src/routers.rs` pass the profile seamlessly into `autonomous.rs`, `guardrails.rs`, and `resume.rs` without touching `turn_runner.rs`.
4. **Omission of `tool_profile` on minimal `v0.2` / ad-hoc test routers (`effective_router_profile`):**
   - *Why:* Existing tests (`resume_loop.rs:551`) call `run_autonomous_research_loop` with a 2-tool `v0.2` builtin router (`Router::new` without `.with_tool_profile`). Stamping `"research/1"` onto a `v0.2` 2-tool session would contradict `research/1`'s `v0.4-research` policy and 22-tool list on resume. `effective_router_profile` records `tool_profile` whenever `router.tool_profile()` is `Some(...)` or the router's policy and defs exactly match a built profile, and omits it for legacy `v0.2` / ad-hoc partial routers.
5. **Placement of Set R tests in `crates/desktop-server/tests/eval_set_r.rs` (not adding `manifest-r.json` under `tests/fixtures/eval/`):**
   - *Why:* (a) `crates/agent/tests/eval_harness.rs:35` asserts `assert_eq!(sets, [Set::P, Set::Q])` on `tests/fixtures/eval/`, and existing tests may not be weakened; (b) `desktop-server` already depends on `export`, `agent`, `research-tools`, `tools`, and `ledger`, enabling real `export::fork::research_from_paper` calls without adding any crate dependency.
6. **Honest construction note for Set R fixture 2 (pre-pointer `7c36a0e` Investigation):**
   - *Faithfulness note:* The `source.ingested` event, pinned file, ledger hash chain, and `model.request`/`model.response` blobs are all written by the real `Ledger` and `ingest_bytes` writers. Because the retired `7c36a0e` `record_paper_map` handler no longer exists in this binary (only the current pointer-citation handler exists), the pre-pointer `tool_defs` hashes come from `research_tools::investigation::earlier_defs` (`GOLDEN_*_7C36A0E`) and the pre-pointer `investigation.map` event payload is appended via `Ledger::append_with_hash`. All other three Set R fixtures (Research, Investigation current, Hand-off child) invoke the live tool/fork writers (`ledger_append`, `record_paper_map`, `research_from_paper`) directly.

### Assumptions
- None about external network services (F1 makes zero network calls).
- Assumed that when V6 builds `investigation/2`, `investigation/2` will decide whether to drop `lean_check` / `search_formal_library` (`00-overview.md` 8.3 question 8); F1 confirmed `search_formal_library` IS in `research/1` (and `lean_check` is registered separately on `build_lean_router`, not in `research/1`).

---

## 3. Merge notes

### Existing files edited (small, local, in separate commits)
- `crates/tools/src/router.rs`: added `tool_profile: Option<String>` field, `with_tool_profile`, and `tool_profile()`.
- `crates/research-tools/src/lib.rs`: added `pub mod tool_profile;`.
- `crates/agent/src/lib.rs`: added `pub mod resume_profile;` and re-exports.
- `crates/agent/src/resume.rs`: added `ResumeInspection.tool_profile`, wired `verify_and_read` / `verify_and_read_with_profile` / `resume_agent_loop` to `resume_profile`.
- `crates/agent/src/autonomous.rs`: emits `tool_profile` on `session.start` / `session.resume` and verifies via `verify_router_for_resume`.
- `crates/agent/src/guardrails.rs`: emits `tool_profile` on `session.start` in `run_research_loop`.
- `crates/desktop-server/src/routers.rs`: added `build_desktop_router_for_profile`; `build_desktop_router` resolves session profile from ledger events.
- `crates/cli-route/src/core_tools.rs`: added `register_core_tools_for_profile` and `core_tool_set_for_profile`; routed research registration through `PROFILE_RESEARCH_V1`.
- `crates/mcp-server/src/main.rs`: resolves profile (`CORE_TOOL_PROFILE` or session events) and builds router via `register_core_tools_for_profile`.
- `crates/cli/src/runtime.rs`: added `build_router_for_profile`; `build_research_router` builds router via `tool_profile::register_profile`.

### Integration hooks for other briefs / later packages
- **F5 (`desktop-server/src/session_mode.rs`, `routes_session.rs`, `turn_runner.rs`):**
  - Reuse `research_tools::tool_profile::session_mode_from_events` (same latest-marker rule over `investigation.started`, `investigation.handoff`, `idea.started`, `solve.started` returning F2's `paper_key::Mode`).
  - `desktop-server/src/routers.rs::build_desktop_router_for_profile` is ready to be called once `turn_runner.rs` selects a mode profile explicitly.
  - Note on rule R2 marker test: F5/S12 should enumerate emitted event kinds per registered tool handler to prove no tool can emit mode markers (`investigation.started`, `idea.started`, `solve.started`).
- **V6 (`investigation/2`):**
  - Add `investigation_v2()` in `crates/research-tools/src/tool_profile.rs`, wire `PROFILE_INVESTIGATION_V2` in `lookup`, `built_profiles`, and `register_profile`, add `investigation/2`'s new `ReadWorkspace` tool ids (`read_map_node`, `record_map_reading`, `record_context_cards`, plus `record_prior_work_check` under `may_gain`) to `investigation/1`'s `may_gain` (never `research/1`), and add a `[profile investigation/2]` section to `tests/golden/tool_profiles.txt`.
  - Confirm `search_formal_library` exclusion per `00-overview.md` 8.3 question 8 (`lean_check` is already absent from `research/1`; only `search_formal_library` is in `research/1`).
- **Explore (`explore/1`) and Solve (`solve/1`):**
  - Wire `PROFILE_EXPLORE_V1` / `PROFILE_SOLVE_V1` in `tool_profile.rs::{lookup, built_profiles, register_profile}` (with `solve_policy()` on `solve/1`) and add their golden sections to `tests/golden/tool_profiles.txt`.
- **S12 (`solve/1` -> `solve/2` resume-brief profile transition, rule R2):**
  - Extend `crates/agent/src/resume_profile.rs::resolve_session_profile` (the `session.resume` loop) to permit a profile transition solely when a `session.resume` event marks a host-written resume-brief leg boundary in Solve mode.
- **F8 (`tests/fixtures/eval/manifest-r.json`):**
  - Once `crates/agent/tests/eval_harness.rs:35` is relaxed from `[Set::P, Set::Q]` to allow Set R in the fixed-tier manifest directory, Set R's fixtures can be linked into the evaluation manifest as well.

---

## 4. Release note draft (Deliverable 6 — for `docs/releases/`, not committed as a release file)

> ### Session compatibility note (tool profiles)
>
> Starting with this release, new Research and Investigation sessions record their active `tool_profile` (`research/1` or `investigation/1`) in `session.start` and `session.resume` ledger events, and resumed pre-upgrade sessions record `tool_profile` on their next `session.resume`.
>
> - **Upgrading:** Existing Research sessions, pre-pointer and current Investigation sessions, and Investigation-to-Research hand-off sessions continue to resume without migration.
> - **Downgrading:** A session started or resumed after this release records `tool_profile` and **will not resume in an older build** (older builds reject unrecognized tool profiles or schemas fail-closed). Keep a backup of your data directory before upgrading if you need to roll back.

---

## 5. Verification commands and exact results

| Command | Result |
|---|---|
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | Exit 0: `No touched source file is over 600 lines.` |
| `cargo test -p research-tools --test tool_def_golden` | Exit 0: **3 passed, 0 failed, 0 ignored** (`tool_defs.txt` unchanged) |
| `cargo test -p research-tools --test tool_profile_golden` | Exit 0: **3 passed, 0 failed, 0 ignored** |
| `cargo test -p research-tools --lib tool_profile` | Exit 0: **6 passed, 0 failed, 0 ignored** |
| `cargo test -p agent --lib resume` | Exit 0: **12 passed, 0 failed, 0 ignored** (6 `resume::drift_tests` + 4 `resume_profile::tests` + 2 `paper_read` resume tests) |
| `cargo test -p agent --lib paper_read` | Exit 0: **12 passed, 0 failed, 0 ignored** |
| `cargo test -p agent --test resume_loop` | Exit 0: **6 passed, 0 failed, 0 ignored** |
| `cargo test -p desktop-server --test eval_set_r` | Exit 0: **5 passed, 0 failed, 0 ignored** |
| `cargo test --workspace --no-fail-fast` (without `RE_SEARCH_PYTHON`) | Exit 0: **682 passed, 0 failed, 2 ignored** (pre-existing live-tier `eval_harness.rs:420,433`) |
| `RE_SEARCH_PYTHON=$(command -v python3) cargo test -p worker -p export --no-fail-fast` | Exit 0: **9 passed, 0 failed, 0 ignored** |
| `cargo clippy -p tools --all-targets -- -D warnings` | Exit 0 (0 warnings) |
| `cargo clippy -p research-tools --lib --test tool_profile_golden --test tool_def_golden -- -D warnings` | Exit 0 (0 warnings) |
| `cargo clippy -p cli-route --all-targets -- -D warnings` | Exit 0 (0 warnings) |
| `cargo clippy -p mcp-server --all-targets -- -D warnings` | Exit 0 (0 warnings) |
| `cargo clippy -p agent --lib --test resume_loop -- -D warnings` | Exit 0 (0 warnings) |
| `cargo clippy -p desktop-server --lib --test eval_set_r -- -D warnings -A clippy::manual_unwrap_or` | Exit 0 (0 warnings) |
| `cargo clippy -p cli --bins -- -D warnings -A clippy::manual_unwrap_or` | Exit 0 (0 warnings) |

---

## 6. Pre-existing failures/lints on Linux, unrun Windows paths, and bugs noticed

### Pre-existing clippy errors under `rustc 1.100.0` on Linux (untouched files)
- `crates/desktop-server/src/evidence.rs:744`: `clippy::manual_unwrap_or` — verified on base commit `e1517477` (also listed in `RULES.md`; note that clippy's suggested replacement would drop the status allowlist match).
- `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6`: unused imports `BufRead`, `BufReader` on non-Windows (`#[cfg(windows)]` test body).
- `crates/research-tools/tests/sandbox_and_fs.rs:6,9`: unused imports `is_symlink_or_junction`, `TEX_AUDIT` on non-Windows.
- `crates/agent/tests/eval_support/manifest.rs:289` (`clippy::match_like_matches_macro`) and `crates/agent/tests/eval_support/sheet.rs:102` (`clippy::unnecessary_map_or`).

### Pre-existing Linux test interaction noticed (not fixed; outside F1 scope)
- `crates/cli/tests/desktop_and_ext.rs:149-168` (`cli_research_workspace_read_fork_diff_export_and_verify`): when `RE_SEARCH_PYTHON` is exported in the environment on Linux, the spawned `re-search run` binary reads `RE_SEARCH_PYTHON` (`crates/cli/src/cmd.rs:172`), attempts to initialize `AppContainerSandbox` (`crates/cli/src/runtime.rs:75`, which fails on Linux with `"AppContainer sandbox requires Windows"`), and exits with code 2 before contacting `ScriptedMock`. Because `desktop_and_ext.rs:168` calls `mock.finish()` (joining a thread blocked in `listener.accept()`) *before* checking `out.status`, the test hangs indefinitely when `RE_SEARCH_PYTHON` is set on Linux. Without `RE_SEARCH_PYTHON`, it passes in 0.48s; running `RE_SEARCH_PYTHON=$(command -v python3) cargo test -p worker -p export` exercises all Python-backed tests cleanly.

### Could not be compiled or run on Linux
- All `#[cfg(windows)]` code paths: `AppContainerSandbox` and `JobObjectSandbox` (`crates/worker`, `crates/desktop-server/src/routers.rs:34-62`, `crates/mcp-server/src/main.rs:118-144`, `crates/cli/src/runtime.rs:49-71`), `crates/desktop-app` WebView2 runtime, and Windows-only tests (`crates/cli/tests/contained_router.rs`, `crates/worker/tests/appcontainer_sandbox.rs`, `crates/worker/tests/python_worker.rs`, Windows branch of `crates/chatgpt-auth/tests/flow.rs`).
- `crates/agent/tests/eval_harness.rs:420,433`: live-tier evaluation tests (`#[ignore]`).

### Design notes
- `00-overview.md` 2.2 describes `research/1` as frozen and `investigation/1` as holding today's 7 tolerated additions and 2 special cases, while existing Research sessions (and `resume_loop.rs:500`) share `register_research_tools` and rely on those same 7 historical `may_gain` entries and 4 recorder `earlier_defs`. Carrying those 7 already-present historical entries on `research/1` as well as `investigation/1` resolves this cleanly without admitting any new tool to `research/1` (see Section 2, Decision 1).
- `crates/agent/src/runner.rs` is 626 lines (over the 600-line ceiling), so `run_agent_loop` was left untouched; if a future brief wants `run_agent_loop` to record `tool_profile`, `runner.rs` must first be split below 600 lines.
