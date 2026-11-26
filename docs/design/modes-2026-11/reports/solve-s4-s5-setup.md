# Report: `solve-s4-s5-setup` (Packages S4 -> S5)

- **Brief:** `docs/design/modes-2026-11/agents/solve-s4-s5-setup.md`
- **Branch:** `gemini/modes/solve-s4-s5-setup`
- **Base commit (initial start):** `921edc8955e111565f6ddf7e9c0f1dfc88b3b170` (`origin/gemini/work/fixed`)
- **Mid-brief sync merge (before S5):** `d174ab6a3eb62244cc4e3adef831c0cd3df4bf44` (`origin/gemini/work/fixed`, merged cleanly at `74a0bc0` with zero conflicts)
- **Toolchain (Linux VM):** `rustc 1.100.0`, `cargo 1.100.0`, `node v22.23.5`, `python3 3.12.16` (`sympy 1.14.0`, `numpy 2.5.5`, `mpmath 1.3.0`)

---

## 1. What Was Implemented (Package by Package)

Every touched and new source file is at or under 600 lines (`python3 scripts/check_file_length.py --base origin/gemini/work/fixed` exits 0), `solve.rs` is 328 lines (`< 350` per brief section 5), `Cargo.toml` / `Cargo.lock` and all tool golden files are untouched.

### Checkpoint S4a — S2/F4/F9 Type Alignment (zero behaviour change)
- `crates/research-tools/src/solve_model.rs` (475 lines, modified): removed local `QuoteRef` struct and replaced it with `pub use crate::quote_core::QuoteRef;` so Solve uses the single shared F4 `QuoteRef` (`start_line` / `end_line`).
- `crates/research-tools/src/solve_model_records.rs` (410 lines, modified): added lossless `From<PauseReason> for run_budget::SolvePauseReason` (and reverse), `PauseReason::as_str()`, and `SolveRunStatus::to_solve_run_state()` / `to_run_state()` mapping cleanly onto F9's `SolveRunState` and 7-state `RunState`.
- `crates/research-tools/src/solve_model_tests.rs` (301 lines, modified): added tests asserting `solve_model::QuoteRef` equals `quote_core::QuoteRef` and verifying every `SolveRunStatus` / `PauseReason` mapping onto F9 `SolveRunState` / `RunState`.

### Checkpoint S4b — Deterministic Setup Validation & Zero-Model Prefill (`research-tools`)
- `crates/research-tools/src/solve_setup.rs` (572 lines, new): `SolveSetupInput`, `SetupDefaults` (F16a plug-in hook including `SetupDefaults::from_suggested_lessons`), `PrefillRequest`, `SolvePrefillResult`, `VanishesCue`, `validate_and_build_setup` (enforcing `statement <= 16,000` chars, `conventions <= 2,000` chars, valid symbol names/duplicates, valid integer/rational dimension powers and trimmed duplicate rejection, valid budgets `max_model_calls_per_leg <= max_model_calls`, `tool_profile == "solve/1"`, boolean `require_different_model`, control-char checks on `given_source.source_id`, computing `math_sha256`, incrementing `revision`).
- `crates/research-tools/src/solve_setup_prefill.rs` (519 lines, new, private helper module re-exported through `solve_setup`): zero-model deterministic prefill (`< 1 s`), `guess_task_kind` (`calculate` / `prove` / `check_derivation`), `scan_statement_symbols` (TeX/Greek/math scanning with single-letter context filtering that avoids false positives like `n-th`, `x-ray`, `(b)`), `detect_show_vanishes_cue` (word-boundary `"show "` matching, stripping `"that "` / `"vanishes"`, unwrapping `O(...)` and `\mathcal{O}(...)`), `default_done_criteria` (D4 exact vs numeric tier, D21 `"prove"` different-model review default + prose-review notice, `vanishes_to_order` criterion suggestion), and D2 `is_same_model`.
- `crates/research-tools/src/solve_setup_tests.rs` (308 lines, new): deterministic & sub-second prefill tests, worked effective-potential statement, Prove D21 notice & criteria, Check-a-derivation + F16a override precedence, R22 wording checks, validation edge cases, and audit regression tests.
- `crates/research-tools/src/lib.rs` (322 lines, modified): +2 lines (`pub mod solve_setup; mod solve_setup_prefill;`).

### Checkpoint S4c — Desktop Routes, Session Creation Seam & `solve/view`
- `crates/desktop-server/src/solve.rs` (328 lines, new): modular sub-dispatcher `handle_solve` registered at `/api/sessions/{id}/solve/..` behind `AuthGuard::authorize_api`, leaving clean seams for S8 (`attach-source`, `ack`), S9 (`check`, `recheck`), and S17 (`transcribe`). Implements:
  - `POST /api/sessions/{id}/solve/prefill`: validates statement/conventions size bounds, merges session/UI pinned conventions and optional local F16a lessons defaults, returns `{ prefill, roles, toolbox, approval_grants, model_connected }` with zero model/network calls.
  - `POST /api/sessions/{id}/solve/setup`: validates setup + pinned conventions under `ledger_write_lock`, appends `solve.setup` v1 with incrementing `revision` and `ask_before_running`, writes new `Given` `proof.entry` steps (`origin: "setup"`) before superseding replaced setup steps with summary-matched `replaced_by` pointers, emits `solve_updated` SSE event, and returns `{ ok, setup, ask_before_running, given_entry_ids, view }`. Old checks turn `Stale` automatically via `math_sha256` change (D15).
  - `GET /api/sessions/{id}/solve/view`: returns the full Solve projection.
  - `has_connected_model`: checks ChatGPT readiness, configured API provider keys, Antigravity acknowledgment, and configured default provider/model.
- `crates/desktop-server/src/solve_view.rs` (580 lines, new):
  - `build_solve_view`: projects `{ setup, run_state, steps, graph, handles, receipts, checks, artifacts, done, toolbox, approval_grants, model_connected }`.
  - `graph` matches `00-overview.md` 2.6 (`GraphView`: `src` = prerequisite, `dst` = dependent, self-loop guard, solid/dashed/plain borders, `DisplayGrade`, S2 badges from `badge_for_step`, active non-stale `"Verified"` legend gating, and `"Needs re-check"` primary badge text when `stale: true` so `graph_view.js` never renders a duplicate `"Stale"` pill).
  - `run_state` maps into F9's 7-state vocabulary, surfacing `WaitingForYou` (`"waiting"`) when approvals are pending, `Interrupted` (`"interrupted"`) when `!running` with recorded leg activity, and `DoneWithOpenItems` (`"done_open_items"`) with `open_items` extracted from `state.done`.
  - `probe_toolbox`: graceful `#[cfg(not(windows))]` Linux fallback reporting `"Toolbox: unavailable (Windows sandbox required)"` (or configured Python runtime note) without spawning Windows-only `AppContainerSandbox`.
  - `build_solve_approval_grants`: describes both `worker_session` and `session_capability` grants and reports pre-turn support honestly (see Section 2).
- `crates/desktop-server/src/lib.rs` (226 lines, modified): +2 lines (`pub mod solve; pub mod solve_view;`).
- `crates/desktop-server/src/api_dispatch.rs` (392 lines, modified): +1 arm dispatching `["sessions", id, "solve", rest @ ..]` behind `AuthGuard::authorize_api`.
- `crates/desktop-server/src/routes_session.rs` (528 lines, modified): localized 2-place seam accepting `mode: "solve"` on `POST /api/sessions` (appending host-only `solve.started`; R2) and attaching `"solve": crate::solve::session_solve_view(...)` on `GET /api/sessions/{id}` for `Mode::Solve`.
- `crates/desktop-server/tests/solve_setup_routes.rs` (563 lines, new): 5 end-to-end tests covering auth/mode/R2 marker enforcement, prefill determinism & Linux toolbox fallback & 400 size limits, setup validation/`Given` steps/`GraphView` + `artifacts`/D15 revision staleness/R22 wording, pre-turn `UiApprover` session grant test (including `WaitingForYou`), and `Interrupted` / `DoneWithOpenItems` / stale graph legend audit checks.

### Checkpoint S5a — Shared `approval_card.js` (S5 is Single Owner)
- `crates/desktop-server/assets/approval_card.js` (326 lines, new): mode-neutral approval checklist card shared by Solve (S5), Explore (X7), and Library sources (F13b). Renders both `session_capability` and `worker_session` grants cleanly, shows `WORKER_RESTART_NOTE` (`"A worker restart asks again."`) and `NO_AUTO_APPROVE_NOTE`, dispatches `"Allow listed"` strictly one grant at a time through `onAllowGrant` (zero auto-approve, zero new scope), snapshots ungranted grant IDs before awaiting so mid-flight `update()` calls remain safe, and uses DOM APIs exclusively (zero `innerHTML`).

### Checkpoint S5b — Solve Setup Card, State Model & First Screen
- `crates/desktop-server/assets/solve_state.js` (401 lines, new): pure DOM-free client state model holding all owner-accepted constants (`TASK_KINDS`, `PROVE_PROSE_NOTICE`, `SAME_MODEL_WARNING`, `NO_MODEL_BANNER`, `TIER_VERIFIED_WORD = 'Verified'`, `TIER_NUMERIC_WORD = 'Numerically checked'`, `TIER_MODEL_AGREED_WORD = 'Model agreed'`, `TIER_GIVEN_WORD = 'Given'`, `ASK_BEFORE_RUNNING_LABEL`), `defaultTargetForKind`, `defaultCriteriaForKind`, `applyPrefillToState`, full-fidelity `applySolveViewToState` revision hydration, `buildSetupSubmission` (including D2 `acceptSameModel` tier downgrade to `model_agreed_same_model`, D4 `everyStepVerified` add/remove, and `target.order`/`target.digits` preservation), and `buildLocalGivenPreviewSteps`.
- `crates/desktop-server/assets/solve_setup.js` (596 lines, new): centre-stage setup card (zero `<dialog>` modals), live safe KaTeX preview (`renderSafeMathPreview` via `katex.render` + `textContent` only; zero `innerHTML`), 3-way kind switch (`Calculate` / `Prove` / `Check a derivation`) with explicit D21 notice on `Prove`, symbols & assumptions table, conventions & optional dimensions table, editable done checklist with D4 toggles, solver/verifier selectors with D2 same-model warning + acceptance checkbox, embedded `approval_card.js` (preserving grant state across re-renders), toolbox version line, D20 `"Ask before running"` opt-in switch (off by default), zero-model banner (`00-overview.md` 5.1), monotonic `prefillSeq` race guard, collapsed summary bar with `"Edit setup"` / `"Re-run checks"` and active steps list.
- `crates/desktop-server/assets/solve.css` (301 lines, new): styling for `approval_card.js` and `solve_setup.js` using existing theme tokens.
- `crates/desktop-server/src/assets.rs` (482 lines, modified): +4 static asset arms (`/assets/approval_card.js`, `/assets/solve_state.js`, `/assets/solve_setup.js`, `/assets/solve.css`).
- `crates/desktop-server/tests/solve_setup_ui.test.mjs` (487 lines, new): 6 Node tests covering `approval_card.js` dual-kind rendering & per-grant dispatch, mid-flight `update()` safety, `solve_setup.js` empty/prefilled/Calculate/Prove/Check-derivation/same-model/no-model/collapse/revision flows, D2 `model_agreed_same_model` submission & full revision hydration & `prefillSeq` race guard, zero-`innerHTML` XSS tests (`<script>`, `<img onerror>`, `<svg onload>`), and `assets.rs` registration checks.

---

## 2. `UiApprover` Pre-Turn Verification Finding (Brief Gate #2)

Verified directly in code (`crates/desktop-server/src/approver.rs:116`, `crates/tools/src/router.rs:240-266`, `crates/desktop-server/tests/mode_shell.rs:188`) and tested end-to-end in `crates/desktop-server/tests/solve_setup_routes.rs` (`solve_ui_approver_prompts_and_records_session_grant_outside_turn`):

1. **`session_capability` grants (`write-workspace`, `network-read`): YES, pre-turn works.**
   `UiApprover` prompts through `GET /api/sessions/{id}/approvals` + `POST /api/sessions/{id}/approvals/{call_id}` + SSE and records grants in `session_lifecycle` while `is_turn_running(state, sid) == false`.
2. **`worker_session` grants (`sympy_eval`, `check_run` for `solver:solve`, `host:check`, `verifier:solve`): NO, cannot be pre-bound before a worker starts.**
   - `UiApprover::decide_ext` (`crates/desktop-server/src/approver.rs:116`) fails closed on `WorkerSession` unless `contained == true`.
   - `Router` (`crates/tools/src/router.rs:240-266`) keys `WorkerSession` grants by `(caller, tool_id, def_hash, policy_hash, live WorkerEpoch)` on a specific `Router` instance. No live `WorkerEpoch` exists prior to starting the contained worker on the first turn.
3. **Resulting honest behaviour:**
   - `build_solve_approval_grants` reports `"pre_turn_session_capability_supported": true`, `"pre_turn_worker_session_supported": false`, and `"pre_turn_approver_supported": false`.
   - `approval_card.js` renders all required grants honestly on the setup card with `preTurnSupported: false`, informing the user that approvals are prompted when the first turn starts and that a worker restart asks again, with zero auto-approve and zero new approval scope.

---

## 3. Independent Adversarial Audit (Carbon-Tier) & Fixes Applied

An independent read-only adversarial audit (`research` subagent, `pro` tier) reviewed every commit across S4a–S5b and identified blocking and important issues, all of which were fixed and regression-tested in commit `40ce287`:

- **Pre-turn approval honesty:** Changed `build_solve_approval_grants` from reporting `pre_turn_approver_supported: true` (misleading for `WorkerSession` grants) to the explicit split above.
- **D2 same-model acceptance:** Updated `buildSetupSubmission` in `solve_state.js` so checking `"Accept same-model review"` downgrades `model_agreed` to `model_agreed_same_model` (in addition to clearing `require_different_model`), matching S2's tier hierarchy.
- **Run-state completeness (`solve_view.rs`):** Added `WaitingForYou` when pending approvals exist, `Interrupted` when `!running` with prior leg activity (`session.start` / `session.resume` / `model.request` / `tool.request`), and `open_items` extraction from `state.done` on `DoneWithOpenItems`.
- **GraphView edge & badge integrity (`solve_view.rs`):** Guarded self-loop edges (`dep != &entry.entry_id`), emitted `"Needs re-check"` as primary badge text on stale nodes so `graph_view.js` does not render duplicate `"Stale"` pills, and gated `"Verified"` in the graph legend strictly on active non-stale steps (`superseded_by.is_none() && !stale && own == Tier::Verified`).
- **Setup revision ordering & prefill bounds (`solve.rs`):** Enforced `MAX_STATEMENT_CHARS` / `MAX_CONVENTIONS_CHARS` on `POST /solve/prefill`, wrote newly created `Given` entries before appending `proof.supersede` with summary-matched `replaced_by`, detected `state.chatgpt.ready()` and `ui.antigravity_ack.is_some()` in `has_connected_model`, and kept `solve.rs` at 328 lines (`< 350`).
- **Validation & prefill heuristics (`solve_setup.rs`, `solve_setup_prefill.rs`):** Validated `require_different_model` as boolean, trimmed IDs and rejected duplicate trimmed dimension keys, checked control characters on `given_source.source_id`, enforced word boundary on `"show "`, stripped leading `"that "` and trailing `"vanishes"`, unwrapped `O(...)` / `\mathcal{O}(...)`, included `show_vanishes_to_order` for `Prove` when a vanishes cue is present, and eliminated single-letter false positives (`n-th`, `x-ray`, `(b)`).
- **UI revision fidelity & async races (`solve_state.js`, `solve_setup.js`, `approval_card.js`):** Hydrated all setup fields on `applySolveViewToState` (`target`, `solver`, `verifier`, `budgets`, `review`, `allowNumericTarget`, `everyStepVerified`, `acceptSameModel`), updated `state.target` on kind switch, removed `every_step_verified` when unchecked, added monotonic `prefillSeq` guard on `triggerPrefill`, preserved `approvalCard` grant updates via `onChange`, and snapshotted ungranted IDs in `approval_card.js`'s `allowListed()` for mid-flight `update()` resilience.

---

## 4. Decisions Taken Alone, Deviations & Assumptions

1. **S2 numeric-tier wording (`"Numerically checked"` vs `"Checked numerically"`):** Followed the 2026-11-26 clarification merged on base (`solve_badge.rs`: `"Numerically checked"` and `"Found"`) consistently across `solve_setup.rs`, `solve_view.rs`, `solve_state.js`, and `solve_setup.js`. See Section 7 regarding base `ui_badge.js:9`.
2. **Splitting `solve_setup_prefill.rs`:** Split the prefill heuristics out of `solve_setup.rs` into a private sibling module `crates/research-tools/src/solve_setup_prefill.rs` (all public items re-exported from `research_tools::solve_setup`) to keep both files under 600 lines.
3. **`index.html` and `app.js` intentionally untouched:** Per brief section 5 (`app.js` is 932 lines, over 600; `index.html` is left for S13 / shell integration), the new JS/CSS assets are registered in `assets.rs` and unit-tested directly via Node without modifying `index.html` or `app.js`. See Merge Notes for the 2-line mount snippet.
4. **Leaving `crates/desktop-server/tests/mode_shell.rs` untouched (deviation from green workspace count):** F5's existing test `mode_shell.rs:86-98` (`unbuilt_modes_refuse_session_creation_and_create_nothing`) loops over `[("solve", "Solve"), ("explore", "Explore")]` and asserts `POST /api/sessions` with `mode: "solve"` returns HTTP 400 `"not built yet"`. Brief `solve-s4-s5-setup.md` explicitly requires `POST /api/sessions` with `mode: "solve"` to succeed (HTTP 200) and append `solve.started`. Per `RULES.md:64` ("Never weaken, delete, skip (`#[ignore]`) or loosen an existing test or assertion. If a test is truly wrong, leave it as is and explain in your report"), `mode_shell.rs` was **not edited**. Note that `mode_shell.rs:169` (asserting `POST /api/sessions/{id}/messages` still refuses unbuilt `solve/1` turns) continues to pass.
5. **`ask_before_running` storage (D20):** Stored `"ask_before_running": bool` as an additive field on the `solve.setup` v1 ledger payload (and surfaced in `view.setup.ask_before_running`) without modifying S2's `SolveSetup` struct or `compute_math_sha256` hash input, since D20 is a UI/orchestration preference rather than mathematical context.
6. **Assumption on Windows sandbox toolbox reporting:** On `#[cfg(windows)]`, `probe_toolbox` reports whether a bundled/configured Python path is present (`"Toolbox: bundled Python runtime ready (versions reported on worker start)"`) without spawning `AppContainerSandbox` during setup prefill; exact live Python/SymPy/NumPy/mpmath versions from the sandboxed worker will be surfaced once S7/S12 starts the worker.

---

## 5. Merge Notes

### Small localized edits to existing files
- `crates/research-tools/src/solve_model.rs` (475 lines): replaced local `QuoteRef` with `pub use crate::quote_core::QuoteRef;` (commit `6329144`).
- `crates/research-tools/src/solve_model_records.rs` (410 lines): added `PauseReason <-> SolvePauseReason` and `SolveRunStatus::to_solve_run_state` / `to_run_state` (commit `6329144`).
- `crates/research-tools/src/solve_model_tests.rs` (301 lines): added S4a alignment tests (commit `6329144`).
- `crates/research-tools/src/lib.rs` (322 lines): added `pub mod solve_setup; mod solve_setup_prefill;` (commit `3f9a324`).
- `crates/desktop-server/src/lib.rs` (226 lines): added `pub mod solve; pub mod solve_view;` (commit `e2e9ef0`).
- `crates/desktop-server/src/api_dispatch.rs` (392 lines): added 1 match arm `["sessions", id, "solve", rest @ ..] => crate::solve::handle_solve(...)` behind `AuthGuard::authorize_api` (commit `e2e9ef0`).
- `crates/desktop-server/src/routes_session.rs` (528 lines): accepted `mode: "solve"` in `POST /api/sessions` (writing host-only `solve.started`) and added `"solve"` payload in `GET /api/sessions/{id}` when `mode == Mode::Solve` (commit `e2e9ef0`).
- `crates/desktop-server/src/assets.rs` (482 lines): added 4 static asset routes for `approval_card.js`, `solve_state.js`, `solve_setup.js`, `solve.css` (commit `097e41e`).

### Required 1-line integrator update on merge
- `crates/desktop-server/tests/mode_shell.rs:86`: update
  ```rust
  for (mode, label) in [("solve", "Solve"), ("explore", "Explore")] {
  ```
  to
  ```rust
  for (mode, label) in [("explore", "Explore")] {
  ```
  once S4 merges (Solve session creation is now built; Explore remains unbuilt until X-lane).

### Downstream seams ready for other briefs
- **S8 / S9 / S17 (`crates/desktop-server/src/solve.rs`, 328 lines):** Add match arms in `handle_solve` (`("POST", ["attach-source"])`, `("POST", ["ack"])`, `("POST", ["check"])`, `("POST", ["recheck"])`, `("POST", ["transcribe"])`).
- **S12 (`solve_turn_routes.rs`):** Adds `POST /api/sessions/{id}/solve/continue` and connects `solve/1` turns; `solve_view.rs` already reads `SolveState` + `run_state` + `ask_before_running`.
- **S13 (Solve UI mount & derivation cards):** Mounts `createSolveSetupView` from `/assets/solve_setup.js` and `/assets/solve.css` in the desktop shell (`index.html` + `new_menu.js` enabling `"Solve"`):
  ```html
  <link rel="stylesheet" href="/assets/solve.css">
  <script type="module" src="/assets/solve_setup.js"></script>
  ```
- **S18 (`solve_artifacts_route.rs`):** `GET /api/sessions/{id}/solve/view` already exposes `artifacts` from `SolveState.artifacts`.
- **F7 (`handoff.packet` prefill):** Calls `research_tools::solve_setup::prefill_solve_setup` directly (pure, deterministic, zero-model).
- **X7 (Explore) & F13b (Library sources):** Import `createApprovalCard` read-only from `/assets/approval_card.js` (owned by S5).

---

## 6. Verification Commands & Exact Measured Results

All commands were run after commit `40ce287` (final code state):

| Check | Exact Command | Exact Result |
| --- | --- | --- |
| Forbidden files unchanged | `git diff origin/gemini/work/fixed -- Cargo.lock Cargo.toml crates/research-tools/tests/golden/ crates/research-tools/src/tool_profile.rs crates/desktop-server/tests/mode_shell.rs crates/desktop-server/src/http_srv.rs crates/desktop-server/assets/app.js crates/desktop-server/assets/canvas_dag.js crates/desktop-server/assets/index.html .github/ \| wc -c` | `0` bytes |
| File length guardrail | `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | Exit 0 (`No touched source file is over 600 lines.`) |
| JS syntax check | `for f in crates/desktop-server/assets/*.js; do node --check "$f" \|\| exit 1; done` | Exit 0 (all assets valid JS) |
| Node UI tests | `node --test crates/desktop-server/tests/*.test.mjs` | **173 passed, 0 failed** (including 6 new tests in `solve_setup_ui.test.mjs`) |
| Focused `research-tools` tests & goldens | `cargo test --locked -p research-tools --lib --test tool_def_golden --test tool_profile_golden --test solve_evidence_labels` | **260 passed, 0 failed** (251 lib + 3 `solve_evidence_labels` + 3 `tool_def_golden` + 3 `tool_profile_golden`) |
| Focused `desktop-server` Solve/handoff/R2 tests | `cargo test --locked -p desktop-server --test solve_setup_routes --test handoff_routes --test mode_markers_r2` | **10 passed, 0 failed** (5 `solve_setup_routes` + 2 `handoff_routes` + 3 `mode_markers_r2`) |
| Focused clippy (`research-tools`) | `cargo clippy --locked -p research-tools --lib --test tool_def_golden --test tool_profile_golden --test solve_evidence_labels -- -D warnings` | Exit 0 (**0 warnings**) |
| Focused clippy (`desktop-server`) | `cargo clippy --locked -p desktop-server --lib --test solve_setup_routes -- -D warnings -A clippy::manual_unwrap_or` | Exit 0 (**0 warnings**; `-A clippy::manual_unwrap_or` suppresses only pre-existing `evidence.rs:744`) |
| Full workspace clippy | `cargo clippy --locked --workspace --all-targets -- -D warnings` | Fails **only** on pre-existing `crates/desktop-server/src/evidence.rs:744:40` (`manual_unwrap_or`, documented in `RULES.md:109`) |
| `export` crate with Python | `RE_SEARCH_PYTHON="$(command -v python3)" cargo test --locked -p export --no-fail-fast` | **17 passed, 0 failed** (5 lib + 4 `export_integrity` + 4 `fork_diff_export` + 4 `writeup_core`) |
| Full workspace test suite | `cargo test --locked --workspace --no-fail-fast` | **1010 passed, 1 failed, 2 ignored** — the sole failure is `crates/desktop-server/tests/mode_shell.rs` (`unbuilt_modes_refuse_session_creation_and_create_nothing`, Section 4 #4) |

---

## 7. Pre-Existing Diagnostics, Unrun Paths, Out-of-Scope Bugs & What Does Not Work Yet

### Pre-existing issues on base (not modified)
- `crates/desktop-server/src/evidence.rs:744`: `clippy::manual_unwrap_or` under Rust 1.97+/1.100 (documented in `RULES.md:109`; file is over 600 lines and untouched).
- `crates/cli/tests/desktop_and_ext.rs:149-168`: on Linux, exporting `RE_SEARCH_PYTHON` globally across `cargo test --workspace` causes this pre-existing CLI test to hang waiting on a child process; running `cargo test --workspace` without `RE_SEARCH_PYTHON` and `RE_SEARCH_PYTHON=$(command -v python3) cargo test -p export` separately completes cleanly.

### Could not compile or run on Linux
- All `#[cfg(windows)]` code paths (including `AppContainerSandbox` and the `#[cfg(windows)]` branch of `solve_view::probe_toolbox`) were not compiled or executed on this Linux VM; CI on Windows will compile and verify them.
- Live browser rendering inside the desktop WebView was not executed (no browser opened per `RULES.md:75`); DOM and KaTeX behaviour was verified in Node via `solve_setup_ui.test.mjs`.

### Out-of-scope bugs noticed (not fixed)
- `crates/desktop-server/assets/ui_badge.js:9`: still defines `SOLVE_NUMERIC_WORD = 'Checked numerically'` whereas base `crates/research-tools/src/solve_badge.rs` was updated on 2026-11-26 to `"Numerically checked"`. `solve_state.js` and `solve_setup.js` use `"Numerically checked"`, but `ui_badge.js` was outside our owned files and left untouched for a follow-up or S13 cleanup.
- `crates/desktop-server/assets/new_menu.js:6-10`: still marks Solve as disabled/unbuilt in the New menu (owned by F5 / enabled in S13 once live Solve workspace UI is mounted).

### What does not work yet (honestly out of scope for S4/S5)
- **Live desktop shell mounting:** `solve_setup.js`, `solve_state.js`, `approval_card.js`, and `solve.css` are served by `assets.rs` and tested in Node, but are not yet mounted into `index.html` / `app.js` / `new_menu.js` (deferred to S13 so `app.js` [932 lines] stays untouched).
- **Running Solve model turns (`POST /api/sessions/{id}/messages`):** Intentionally still returns 400 (`solve/1` tool profile is unbuilt until S7 and Solve turn runner until S12).
- **Execution of `"Re-run checks"` button:** Displays an explicit zero-model notice until S9 adds `POST /api/sessions/{id}/solve/recheck`.
- **Pre-turn binding of `worker_session` grants:** As verified in Section 2, `WorkerSession` grants bind only once a contained worker starts on the first turn (S7/S12); `"Allow listed"` cannot pre-approve them before a worker epoch exists.
