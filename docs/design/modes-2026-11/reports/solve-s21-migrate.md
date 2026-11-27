# Report: solve-s21-migrate — Fork Earlier Research Session Into Solve (S21)

- **Brief:** `docs/design/modes-2026-11/agents/solve-s21-migrate.md`
- **Base commit (`origin/gemini/work/fixed`):** `24be58e6797679cfa423dcc16ea2073b54288714` (merged forward with `origin/gemini/work/fixed` `eb0eb63` on follow-up hardening pass)
- **Branch:** `gemini/modes/solve-s21-migrate`
- **Package:** `S21` (Checkpoint `S21a` + post-audit hardening)
- **Date:** 2026-11-27

---

## 1. What Was Implemented

### Package S21 — `POST /api/sessions/{id}/open-in-solve`

| File | Change | Line Count | Purpose |
| --- | --- | ---: | --- |
| `crates/desktop-server/src/solve_migrate.rs` | New | 583 | Host handler `handle_open_in_solve(state, session_id, body)` and deterministic Research-to-Solve migration helpers |
| `crates/desktop-server/tests/solve_migrate.rs` | New | 595 | End-to-end integration tests for D19 tier mapping, B1 solver-override rejection, `AuthGuard` 401/403 + `Origin`, pre-fork fail-closed validation, `at_seq` prefix forks, legacy `investigation.handoff` Research sessions, post-F1 `research/1` migration + Rule R2 `Drift`, and intact parent `research/1` resume (`eval_set_r`) |
| `crates/desktop-server/src/lib.rs` | Small edit (+1 line) | 251 | Export `pub mod solve_migrate;` |
| `crates/desktop-server/src/api_dispatch.rs` | Small edit (+3 lines) | 484 | Register `("POST", ["api", "sessions", id, "open-in-solve"])` behind `AuthGuard::authorize_api` and loopback `Origin` enforcement |

**Do-not-edit verification:** `crates/export/src/*`, `crates/agent/src/*`, `crates/desktop-server/src/{http_srv,routes_session,solve,solve_view,session_mode}.rs`, `crates/desktop-server/tests/eval_set_r.rs`, `crates/research-tools/tests/golden/tool_defs.txt`, `crates/research-tools/tests/golden/tool_profiles.txt`, and `Cargo.lock` were not modified.

### Behaviour Delivered

1. **Authentication, origin, and preconditions:**
   - Routed through `http_srv.rs` -> `AuthGuard::authorize_api` -> `api_dispatch::dispatch_api`: missing/invalid `X-ReSearch-Token` or cookie-only requests answer `401`; foreign `Origin` headers answer `403` before any body is read or session is created.
   - Validates `session_id` (`400`), checks `state.session_exists(session_id)` (`404`), requires a JSON object body with only allowed keys (`at_seq`, `statement`, `conventions`, `task_kind`, `verifier`; `"solver"` is explicitly rejected with `400` to protect D19 legacy same-model review tiers from spoofing, unknown keys return `400`, and explicit `"verifier"` requires non-empty `provider` and `model`), checks `is_turn_running` inside `state.ledger_write_lock` (`409`), verifies the parent ledger fail-closed via `ledger.read_verified_session(session_id)` before building setup, and rejects empty Research ledgers or non-Research sessions (`Mode::Investigate`, `Mode::Explore`, `Mode::Solve`) with `400`.
   - Validates `at_seq` bounds (`1..=events.len()`), checks that BOTH the full session and the selected prefix `1..=at_seq` have `session_mode == Mode::Research`, and builds + validates + serializes `SolveSetup` **before** calling `export::fork_session` so validation failures never commit an orphan fork in SQLite or `sessions/<child>/sources/`.
2. **Fork, mode marker, and prefilled `solve.setup`:**
   - Calls `export::fork_session(&state.cfg.data_dir, &ForkOptions { parent_session, at_seq, modified_assumption: None })`, verifying the parent hash chain, blobs, and copied `source.ingested` file SHA-256 digests fail-closed while leaving the parent Research ledger completely untouched.
   - Appends host-written `solve.started` (`research_tools::tool_profile::MARKER_SOLVE_STARTED`, schema v1, `{}`) and `solve.setup` (`event_kinds::SOLVE_SETUP`, schema v1, `revision: 1`, `tool_profile: "solve/1"`, `ask_before_running: false`, computed `math_sha256`).
   - Populates `solve.setup` with:
     - `origin`: `NodeRef { session: parent, mode: Mode::Research, id: format!("seq:{at_seq}"), paper: None, node_fp: Some(anchor.event_hash) }`.
     - `statement`: first non-empty `prompt` from `session.start`/`session.resume` (fallback `investigation.handoff["question"]`, then `meta.title`, then `Continued from earlier Research session <id>`), or explicit body override.
     - `conventions`: explicit override, else `meta.pinned_conventions` (respecting explicitly cleared `Some("")` without resurrecting earlier turns), else latest turn's `pinned_conventions` or `<pinned_conventions>...</pinned_conventions>` block in `system_prompt`, else `ui.pinned_conventions` when no turn event exists.
     - `assumptions`: active (non-superseded) `proof.entry` entries of type `assumption` from the forked prefix, keeping their original `e-N` IDs (deduplicated/sanitized, capped at `MAX_ASSUMPTIONS = 128` with a warning if truncated).
     - `sources`: unique `source_id` values from copied `source.ingested` events (capped at `MAX_SOURCES = 64` with a warning if truncated).
     - `roles`: solver extracted from latest `session.start`/`session.resume` (`provider`/`route`, `model`/`model_requested`) with fallback to `cli.init`/`model.response`/`model.request`, `meta`, and `ui`; verifier extracted from `ui.agent_models` (`full_verifier`, `step_verifier`, `verifier`), `ui.role_presets`, `proof.verification` (`verifier_model`/`model`), and `verifier.request`; API provider names canonicalized via `agent::ProviderKind::parse(..).cli_name()`.
   - Saves child `SessionMeta` (with `fork_info`, `verification_mode: false`, `calculation_mode: false`, inherited workspace/settings, non-resurrected `pinned_conventions`) and attaches the child to the parent's project if present.
3. **D19 legacy step tiers & legacy receipts:**
   - Computes exact tiers via `research_tools::solve_status::compute_all_tiers(&SolveState::from_events(&child_events))`:
     - Legacy active `assumption` steps -> `AssumedByModel` (never auto-promoted to `Given`).
     - Legacy `sympy_eval` computation steps without handles -> `Unchecked` (sublabel `"ran"`, never `Verified` or `CheckedNumerically`).
     - Legacy accepted `proof.verification` -> `ModelAgreedSameModel` (with `"same model"` flag) when same model or unattributed; `ModelAgreed` when a different `verifier_model` was recorded.
     - Legacy rejected `proof.verification` -> `Failed`.
     - Legacy `lean_check` steps -> `Unchecked` (no Lean tier).
   - Builds `legacy_receipts` using `research_tools::solve_receipts::build_receipts(&child_events, Some(&ledger))` (decoding blobs safely so inner Python exceptions and `lean_check` `rejected_sorry`/`failed` results report `"ok": false`), labelling `lean_check` receipts with `LEGACY_LEAN_CHECK_RECEIPT_LABEL = "Lean check recorded in the earlier session"` (`grants_tier: false`) and `sympy_eval` receipts with `LEGACY_SYMPY_RECEIPT_LABEL` (`grants_tier: false`).

---

## 2. Decisions Taken Alone, Deviations, and Assumptions

1. **HTTP status `201 Created`:** `POST /api/sessions/{id}/open-in-solve` returns `201` (matching `POST /api/sessions/{id}/handoff` in `handoff.rs`) with `{ ok, session_id, parent_session, mode, mode_label, fork, origin, setup, ask_before_running, tiers, legacy_receipts, project_id, warning, warnings, view }`.
2. **`NodeRef.id` and `NodeRef.node_fp` format for Research sessions:** Earlier Research sessions have no reserved node prefix (`paper_key.rs:407`), so `origin` uses `id: format!("seq:{at_seq}")` and `node_fp: Some(anchor.event_hash)` to identify the exact fork anchor unambiguously without colliding with `ID_PREFIXES`.
3. **Named constants for D19 display strings (`RULES.md` §Scope):** `LEGACY_LEAN_CHECK_RECEIPT_LABEL` and `LEGACY_SYMPY_RECEIPT_LABEL` live in `solve_migrate.rs` as named constants so wording changes happen in one place.
4. **No duplicate `Given` `proof.entry` writes on migration:** Copied legacy `assumption` entries stay in the forked ledger under their original `e-N` IDs as `AssumedByModel` (requiring user acknowledgement per D10) and are also listed in `solve.setup.assumptions` for the setup card. No extra `StepOrigin::Setup` `proof.entry` rows are written during `open-in-solve`.
5. **Strict request schema (`ALLOWED_OPTION_KEYS`):** Unknown top-level JSON fields on `POST /api/sessions/{id}/open-in-solve` are rejected with `400` so client typos (e.g., `atSeq`) cannot silently trigger a full-session fork.
6. **Zero-defaults prefill (`defaults: None`):** Migration calls `prefill_solve_setup` with `defaults: None` so migrated setup is a pure, deterministic function of the Research session and request rather than external `lessons.db` state.
7. **Mid-append I/O atomicity limitation (assumption/constraint):** Because `crates/export/src/fork.rs` is outside S21's owned files and `ledger.db` is append-only (`event_no_delete` trigger), `solve_migrate.rs` eliminates all deterministic `400` failures before calling `fork_session`; only an abrupt mid-write disk/SQLite I/O error between `fork_session`, `solve.started`, and `solve.setup` could leave a partial fork.
8. **Deviation / UI boundary:** S21 is strictly the server endpoint and tests; the frontend `"Open in Solve"` button on Research sessions and `DESKTOP_MODE_SOLVE_ENABLED` remain for the UI and S12 lanes.

---

## 3. Independent Audit Summary (Gemini 4 Carbon)

1. Initial independent adversarial audit (`Model: pro`) reviewed the first S21 draft and flagged 3 blockers (`B1` unused helper warning in `tests/solve_migrate.rs`, `B2` `fork_session` running before `validate_and_build_setup` allowing orphan forks, `B3` missing report) and 5 high findings (`H1` verifier role keys/fields, `H2` solver role fallback coverage, `H3` cleared conventions resurrection, `H4` `at_seq` prefix mode check, `H5` blob-unaware `legacy_receipts` reporting `"ok": true` on failed `sympy_eval`/`lean_check`); all were fixed in `c4330a5` / `fdcd647`.
2. A second independent read-only audit (`7a01ce388c918f9e1c9b62f6bcf36a30`) flagged an additional D19 integrity flaw: allowing a request-supplied `"solver"` override on `POST /api/sessions/{id}/open-in-solve` enabled silently upgrading legacy same-model `proof.verification` rows from `Tier::ModelAgreedSameModel` to `Tier::ModelAgreed` via `solve_own_tier.rs:492-498`. Follow-up hardening commits (`629fb52`, `65a990d`) closed this by:
   - Explicitly rejecting `"solver"` in `open-in-solve` requests (`400`) and always preserving `solver` from the parent Research session.
   - Requiring non-empty `provider` and `model` for explicit `"verifier"` overrides (`400`).
   - Calling `ledger.read_verified_session(session_id)` fail-closed before constructing `SolveSetup`.
   - Ignoring transport route `"api"` in `extract_solver_role` so `provider` never leaks `"api"`.
   - Adding regression tests for B1 solver-override rejection, blank/oversized verifier rejection, post-F1 `research/1` migration, and Rule R2 `ResumeRejectKind::Drift` on replay-resume of migrated Solve children.

---

## 4. Merge Notes

- **Edited existing files:**
  - `crates/desktop-server/src/lib.rs`: added `pub mod solve_migrate;` at line 76.
  - `crates/desktop-server/src/api_dispatch.rs`: added `("POST", ["api", "sessions", id, "open-in-solve"])` arm at lines 342–344, right below `("POST", ["api", "sessions", id, "fork"])`.
- **Cross-lane integration notes:**
  - **`solve-s12-loop`:** `session_mode.rs` keeps `DESKTOP_MODE_SOLVE_ENABLED = false` here; once S12 flips it to `true` and enables brief-mode resume, migrated Solve sessions are immediately resumable via Solve's resume-brief loop (`solve.started` + `solve.setup` already present, `tool_profile: "solve/1"`). Note two important seams for S12:
    1. `crates/agent/src/resume_profile.rs:59-75` (`resolve_session_profile`): for **post-F1** Research sessions whose `session.start` carries `"tool_profile": "research/1"`, `resolve_session_profile` currently returns `research/1` even after `solve.started` is appended (whereas pre-F1 sessions fall through to `legacy_profile_name_for_events` -> `solve/1`). S12's `resume_mode: "brief"` dispatcher should key off `session_mode_from_events` / `solve.started` rather than `session.start["tool_profile"]`.
    2. `crates/desktop-server/src/solve_view.rs:84-91,142` reports `run_state.state = "interrupted"` for migrated sessions containing historical `session.start`/`model.request`/`tool.request` events before `solve.started`; S12/S13 may want `has_leg_activity` to consider only events after `solve.started` so freshly migrated sessions report `"ready"`.
  - **`solve-s13-s17-ui-intake` / `solve_view.rs`:** `GET /api/sessions/{id}/solve/view` currently serializes `SolveReceipt` directly (`solve_view.rs:458-462`) without attaching `LEGACY_LEAN_CHECK_RECEIPT_LABEL` on `lean_check` receipts; UI or a small follow-up in `solve_view.rs` can read `tool == "lean_check"` (or reuse `solve_migrate::LEGACY_LEAN_CHECK_RECEIPT_LABEL`) to display `"Lean check recorded in the earlier session"` on subsequent view reloads.

---

## 5. Bugs / Gaps Noticed Outside Owned Files (Not Fixed Here)

1. **`crates/research-tools/src/solve_state.rs:377` (`parse_proof_verification`):** Reads only `p.get("verifier_model")` on `proof.verification`. Real Research verifiers in `crates/agent/src/verifiers.rs:579,680` write `"model"` (on-demand reviews) or record `"model"` only on the preceding `verifier.request` event (autonomous reviews). Consequently, for real historical Research ledgers written by `agent::verifiers`, `SolveReview.verifier_model` parses as `None` and `review_accepted_tier` conservatively falls back to `Tier::ModelAgreedSameModel` even when a different model verified the step.
2. **`crates/research-tools/src/solve_own_tier.rs:492-498` (`review_accepted_tier`):** Compares legacy `review.verifier_model` against the current `setup.roles.solver.model`. While `open-in-solve` now refuses `"solver"` overrides at migration time, if `setup.roles.solver.model` is empty (`""`) or later changed via a `POST /solve/setup` revision, `vm != sm` can still misclassify a historical same-model review as `ModelAgreed` (or downgrade a different-model review).
3. **`crates/research-tools/src/solve_state.rs:199-201` (`SolveState::apply`):** Pushes pre-fork Research `sympy_eval` tool results into `pending_runs`, so migrated sessions can inherit historical Research SymPy calls in gate G1's pending-runs list unless scoped to events after `solve.started`.
4. **`crates/desktop-server/src/solve_view.rs:115` (`build_run_state_json`):** Calls `count_model_calls(events, RunScope::All)`, counting historical Research `model.request` events before `solve.started` against the Solve session's `model_calls_used` budget.
5. **`crates/desktop-server/src/solve_routes_ext.rs:184` (`handle_ack`):** Requires `entry.entry_sha256.as_deref() == Some(entry_sha256.as_str())`. Legacy Research `proof.entry` events written by `ledger_append` do not store `entry_sha256` (`None`), so `POST /api/sessions/{id}/solve/ack` currently rejects acknowledging migrated `AssumedByModel` assumptions or `hand_checked` legacy steps with `409 Conflict`.
6. **`crates/desktop-server/src/solve.rs:202-275` (`sync_setup_given_steps`):** When a migrated Solve session later submits `POST /api/sessions/{id}/solve/setup` (revision 2), `sync_setup_given_steps` creates new `StepOrigin::Setup` `Given` steps (`e-K`) for entries in `setup.assumptions` without superseding or linking the forked legacy `e-1` `AssumedByModel` step.

---

## 6. Verification Results (Non-Root Linux, `RE_SEARCH_PYTHON` Unset)

Environment: `uid=1001(antigravity)`, `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `node v22.23.5`.

| Command | Result | Exact Counts |
| --- | --- | --- |
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | PASS (exit 0) | `No touched source file is over 600 lines.` (`solve_migrate.rs`: 583, `tests/solve_migrate.rs`: 595, `lib.rs`: 251, `api_dispatch.rs`: 484) |
| `unset RE_SEARCH_PYTHON && cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` | PASS (exit 0) | 0 errors, 0 warnings across all workspace targets |
| `unset RE_SEARCH_PYTHON && cargo test -p desktop-server --test solve_migrate --locked` | PASS (exit 0) | 3 passed, 0 failed, 0 ignored |
| `unset RE_SEARCH_PYTHON && cargo test --workspace --locked --no-fail-fast` | PASS (exit 0) | 152 test suites (including `tool_def_golden`, `eval_set_r`, `mode_markers_r2`, `solve_migrate`): **1,198 passed, 0 failed, 2 ignored** (pre-existing live-tier `eval_harness` tests `file_sheet_from_json` and `live_manifests_validate`) |
| `for f in crates/desktop-server/assets/*.js; do node --check "$f" \|\| exit 1; done` | PASS (exit 0) | All desktop JS assets syntactically valid |
| `node --test crates/desktop-server/tests/*.test.mjs` | PASS (exit 0) | **226 passed, 0 failed, 0 skipped** |

### Pre-existing Failures & What Could Not Run on Linux

- **Pre-existing failures on Linux:** None observed (running as non-root `uid=1001` with `RE_SEARCH_PYTHON` unset; `-A clippy::manual_unwrap_or` covers the known base-commit lint in `evidence.rs:744`).
- **Not compiled or run on Linux:** Windows-only `cfg(windows)` code (`AppContainerSandbox`, `WinChild` Job Objects, `crates/desktop-app` WebView2 host, Windows installer/runtime staging). `solve_migrate.rs` and `tests/solve_migrate.rs` use only cross-platform `std` / workspace APIs with no Unix-specific paths or shell-outs.
