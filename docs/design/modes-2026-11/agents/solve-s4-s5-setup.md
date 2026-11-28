# Brief solve-s4-s5-setup — Solve setup, view & first screen (S4 -> S5)

**Status:** Active implementation brief (F5, F7, S2, and S2 label/comparison fixes are merged on `gemini/work/fixed`; `mode_shell.rs` verified `UiApprover` prompts outside a running model turn; S2 `solve_badge.rs` uses `"Numerically checked"` and `"Found"`).
**Packages:** S4 (session marker, setup endpoint, deterministic prefill, `solve/view`) -> S5 (UI setup card, first screen, shared `approval_card.js`).
**Governing design:** `00-overview.md` 0.1, 2.1 (F5, S4, S5), 2.5, 2.6, 2.10, 3.1 (rows 3, 6, 13), 3.3, 5.1, 8.1–8.3 (owner decisions of 2026-11-25: Lean cut, `"Verified"` vs `"Checked numerically"`, `"prove"` kept with prose-review notice); `solve.md` 2–4, 5.1, 5.5, 7.2–7.3, 9 (rows S4, S5), 13 (D1, D2, D4, D5, D8, D10, D15, D20, D21); `agents/RULES.md`.
**Read reports first:** `reports/s2-solve-model.md`, `reports/f1-tool-profiles.md`, `reports/f4-f9-quote-budget.md`, `reports/f6-graph-ui.md`, and (once merged) `reports/f5-f7-mode-shell.md`.

## 1. Start conditions & owner gates

- **Required merges on `origin/gemini/work/fixed` before S4:**
  - Already merged on base `832eab5bf`: F0, F1, F2, F3, F4, F6, F8, F9, F10, F14a, S2.
  - **Must merge before S4 starts:** `f5-f7-mode-shell` (at minimum its **F5** checkpoint: `session_mode.rs`, `api_dispatch.rs`, slimmed `routes_session.rs` `< 560` lines and `index.html` `< 560` lines, `new_menu.js`, `session_rail.js`).
- **Required before S5 starts:** S4 checkpoint committed and pushed, plus F5 merged.
- **Owner / foundation confirmation gates (before S5 builds `approval_card.js`):**
  1. **Single owner of `assets/approval_card.js` (`00-overview.md` 3.1 row 13 & section 9 open item):** **S5 is designated the single owner** of `crates/desktop-server/assets/approval_card.js` (registered in `assets.rs`). Explore X7 and Library F13b wait on S5 rather than racing on conditional creation. Confirm with owner/integrator if any earlier foundation follow-up should take ownership instead.
  2. **Pre-turn `UiApprover` verification (`00-overview.md` 3.1 row 13, `solve.md` D5):** Note that `agents/f5-f7-mode-shell.md` does **not** currently include a `UiApprover` pre-turn verification task! Therefore **S4/S5 MUST verify** (or flag a tiny pre-S5 F5 follow-up to settle) whether `UiApprover` can prompt and record grants outside an active model turn. If it cannot prompt pre-turn, S5 renders the grant checklist on the setup card and binds `"Allow listed"` to the first-turn approval flow without inventing any auto-approve path or new scope.

## 2. Interfaces: base-verified vs future-merge

**Verified on base `832eab5bf`:**
- S2 (`crates/research-tools/src/`): `solve_model.rs` (486 lines: `event_kinds::*`, `SolveSetup`, `TaskKind`, `SolveTarget`, `DoneCriterion*`, `SymbolSpec`, `SetupAssumption`, `SolveRoles`, `SolveBudgets`, `ReviewPolicy`, `compute_math_sha256`, `compute_entry_sha256`), `solve_state.rs` (517 lines: `SolveState::from_events`, including `SolveState.artifacts`), `solve_status.rs` (230 lines: `compute_all_tiers`), `solve_badge.rs` (398 lines: `wording::*`, `DisplayGrade`, `BorderStyle`, `Tier`, `StepBadge`, `badge_for_step`, `compute_all_badges`, `rests_on_summary`).
- **S2->F4/F9 alignment seam on base:** `solve_model.rs:161-170` still holds a local `QuoteRef` struct (`reports/s2-solve-model.md` section 4), while `run_budget.rs:446` already defines `SolveRunState` + `solve_run_state` mapping into F9's shared `RunState`! Checkpoint S4a replaces S2's local `QuoteRef` with `pub use crate::quote_core::QuoteRef;` and maps `solve_model_records::SolveRunStatus` onto `run_budget::SolveRunState` / `RunState` cleanly so every downstream Solve package (S7–S21) uses one `QuoteRef` and one `RunState`.
- Base `routes_session.rs:143,170` (616 lines, over limit on base, shrunk `< 560` by F5): currently rejects unknown session creation modes and handles only Research/Investigation on `POST /api/sessions` and `GET /api/sessions/{id}`.
- F1 (`tool_profile.rs`, 329 lines): `MARKER_SOLVE_STARTED`, `PROFILE_SOLVE_V1`, `session_mode_from_events`.
- F6 (`crates/desktop-server/assets/`): `ui_badge.js` (387 lines), `run_bar.js` (283 lines), `graph_view.js` (539 lines), `quote_chip.js` (178 lines), `math_katex.js`. All registered in `crates/desktop-server/src/assets.rs` (365 lines).
- Existing approval stores (`session_lifecycle.rs`, 231 lines: `can_grant_for_session`, `list_session_grants`, `matching_session_grant`; `tools/src/router.rs`: per-caller/tool/def_hash/policy/epoch `WorkerSession` grants).

**Future-merge interfaces to verify after F5 (and optionally F16a) merge:**
- From **F5** (`f5-f7-mode-shell`): `crates/desktop-server/src/session_mode.rs`, `api_dispatch.rs` (extracted from 702-line `http_srv.rs`), slimmed `routes_session.rs` (`< 560` lines), slimmed `index.html` (`< 560` lines), `assets/new_menu.js`, `assets/session_rail.js`.
- From **F7** (when merged later): `handoff.packet` prefill calls S4's `solve_setup.rs` helpers (`00-overview.md` 4.3); keep S4's prefill functions pure and reusable by F7.
- From **F16a** (optional hook, in-flight `lib-f13a-f16a-f15`): S4's prefill accepts an optional `SetupDefaults { conventions, symbols, dimensions }` struct so F16a user-corrected defaults plug in later without rewriting S4.

## 3. Smallest implementation checkpoints

Push to `gemini/modes/solve-s4-s5-setup` after each checkpoint and run `git fetch origin && git merge origin/gemini/work/fixed` before S5.

1. **Checkpoint S4a (S2/F4/F9 type alignment, zero behaviour change):** Replace local `QuoteRef` in `solve_model.rs:161-170` with `pub use crate::quote_core::QuoteRef;` and map `SolveRunStatus` onto F9's `run_budget::SolveRunState` / `RunState` (`run_budget.rs:446`). Keep `solve_model.rs` `<= 500` lines and all existing S2/F4/F9 tests green.
2. **Checkpoint S4b (deterministic setup & prefill in `research-tools`):** Add `crates/research-tools/src/solve_setup.rs` (+ `solve_setup_tests.rs`): validate `SolveSetup` (`statement <= 16,000` chars, `conventions <= 2,000` chars, valid symbols/dimensions/budgets/roles, `tool_profile: "solve/1"`), compute `math_sha256`, deterministic prefill from statement text + pinned conventions + optional `SetupDefaults` (zero model calls, `< 1 s`): task-kind guess (`calculate` / `prove` / `check_derivation`), TeX/math symbol scan with default assumptions, default done checklist per kind (D4, D21: `"prove"` defaults to different-model review and carries the prose-proof notice), and `"show ..."` cue detection for suggested `vanishes_to_order` criterion.
3. **Checkpoint S4c (desktop routes, post-F5 `routes_session.rs` 2-line seam & `solve/view`):** Add `crates/desktop-server/src/solve.rs` and `solve_view.rs`:
   - Post-F5 2-line registration seam in slimmed `crates/desktop-server/src/routes_session.rs` (`< 560` lines): accept `mode: "solve"` on `POST /api/sessions` (appending host-only `solve.started`; rule R2) and include the `"solve"` summary/view payload on `GET /api/sessions/{id}` when `session_mode == Mode::Solve`.
   - `POST /api/sessions/{id}/solve/setup` (auth-guarded, validates via `solve_setup.rs`, appends `solve.setup` v1 with incrementing `revision`, emits `proof.entry` `assumption` steps with `origin: "setup"` -> badge `Given` for conventions and assumptions; on revision, leaves old checks to turn `Stale` via `math_sha256` per D15).
   - `POST /api/sessions/{id}/solve/prefill` (pure deterministic preview: kind, symbols, default checklist, same-model warning per D2, required grant descriptors for the approval card, toolbox version line with graceful `#[cfg(not(windows))]` Linux fallback reporting `"unavailable (Windows sandbox required)"` or configured Python versions without spawning Windows-only `AppContainerSandbox`).
   - `GET /api/sessions/{id}/solve/view` returning `{ setup, run_state, steps, graph, handles, receipts, checks, artifacts, done, toolbox, approval_grants }` where `graph` uses the exact shared `GraphView` schema of `00-overview.md` 2.6 (`src` = prerequisite, `dst` = dependent, solid/dashed/plain borders per `solve.md` 3, `DisplayGrade` per `00-overview.md` 2.10, badges from S2's `badge_for_step`), and `artifacts` exposes `SolveState.artifacts` for S18.
   - Verify `UiApprover` pre-turn capability here and document the result in the report (for S5a).
   - Register `/api/sessions/{id}/solve/..` via 1 line in F5's `api_dispatch.rs` behind `AuthGuard::authorize_api`.
4. **Checkpoint S5a (shared `approval_card.js` — S5 is single owner):** Add `crates/desktop-server/assets/approval_card.js` (registered in `assets.rs`): renders both grant kinds (`session_capability` from `session_lifecycle.rs` and `worker_session` per caller/tool/epoch from `router.rs`), one line per grant, `"Allow listed"` dispatching each grant separately through the existing approver (zero auto-approve, zero new scope, clear note that a worker restart asks again). Mode-neutral API for Solve (S5), Explore (X7), and Library sources (F13b).
5. **Checkpoint S5b (Solve setup card & first screen):** Add `crates/desktop-server/assets/solve_setup.js`, `solve_state.js`, `solve.css`:
   - Centre-stage setup card (not a modal): statement input with live KaTeX preview (`math_katex.js`), 3-way kind switch (`Calculate` / `Prove` / `Check a derivation`), explicit D21 notice on `Prove` (`"a proof in prose is reviewed by a model; only calculations are computed"`), symbol assumption table, conventions + optional dimensions table, editable done checklist (`Verified` or `Checked numerically` toggle per D4), solver/verifier model selectors with D2 same-model warning, `approval_card.js` grant list (D5), toolbox version line, `"Ask before running"` opt-in switch (D20), `"Run"` button collapsing setup to one line and displaying `Given` steps.
   - Works with zero model connected (shows setup, prefill, and re-run-checks capability per `00-overview.md` 5.1).

## 4. Owned files

- **New:**
  - `crates/research-tools/src/solve_setup.rs`, `crates/research-tools/src/solve_setup_tests.rs`
  - `crates/desktop-server/src/solve.rs`, `crates/desktop-server/src/solve_view.rs`
  - `crates/desktop-server/tests/solve_setup_routes.rs`
  - `crates/desktop-server/assets/approval_card.js`, `solve_setup.js`, `solve_state.js`, `solve.css`
  - `crates/desktop-server/tests/solve_setup_ui.test.mjs`
- **Small localized edits (`<= 600` lines each):**
  - `crates/research-tools/src/solve_model.rs` (486 -> ~475: `QuoteRef` re-export) and `solve_model_records.rs` (351: `SolveRunStatus -> SolveRunState/RunState` mapping)
  - `crates/research-tools/src/lib.rs` (300: +1 `pub mod solve_setup;`)
  - `crates/desktop-server/src/lib.rs` (+2 `pub mod solve; pub mod solve_view;`)
  - `crates/desktop-server/src/assets.rs` (365: +4 static asset arms)
  - Post-F5 `crates/desktop-server/src/routes_session.rs` (`< 560`: 2-line `mode: "solve"` creation + `GET /api/sessions/{id}` `"solve"` field)
  - Post-F5 `crates/desktop-server/src/api_dispatch.rs` (+1 `/api/sessions/{id}/solve/..` arm)

## 5. Shared registration seams & Do-not-edit files

- **Seams left for downstream briefs:**
  - `desktop-server/src/solve.rs`: S8 adds attach-source + `POST .../solve/ack`; S9 adds `POST .../solve/check` + `POST .../solve/recheck`; S17 adds TeX intake & transcription ack extension. Keep `solve.rs` modular (`< 350` lines) with a sub-dispatcher. (S12 puts `POST .../solve/continue` in `solve_turn_routes.rs`; S18 puts artifact GET in `solve_artifacts_route.rs` so they never collide with `solve.rs`.)
  - `assets/approval_card.js`: S5 owns creation & `assets.rs` registration; Explore X7 and F13b import it read-only.
- **Do NOT edit:** `crates/desktop-server/src/http_srv.rs` (702 lines), `app.js` (932 lines), `canvas_dag.js` (688 lines), `index.html` (unless F5 slimmed it `< 560` and only for a 1-line script/CSS mount; otherwise record in Merge notes), `crates/research-tools/src/tool_profile.rs`, `tests/golden/tool_defs.txt`, `tests/golden/tool_profiles.txt`, `Cargo.lock`.

## 6. Tests through the entry point

- **Rust (`cargo test -p research-tools -p desktop-server`):**
  - Prefill determinism & speed (`< 1 s` on worked effective-potential statement and synthetic inputs; zero model/network calls; Linux `#[cfg(not(windows))]` toolbox fallback succeeds cleanly).
  - Setup validation limits (`statement > 16,000`, `conventions > 2,000`, invalid symbol/dimension/budget rejected with 400).
  - `POST /api/sessions` with `mode: "solve"` writes host-only `solve.started`; `POST /api/sessions/{id}/solve/setup` enforces `AuthGuard::authorize_api`, writes `solve.setup` + `Given` `proof.entry` steps; revision increments `revision` and changes `math_sha256` (turning old checks `Stale` per D15); `GET /api/sessions/{id}` and `GET solve/view` return valid `GraphView` JSON + `artifacts` + badges matching S2.
  - Rule R22 wording test on `solve_view` and prefill payloads: `"verified"` never appears outside allowed places, `"Computed"` never appears, `"proved"` never on computed items.
  - Tool goldens (`tool_def_golden`, `tool_profile_golden`) pass untouched.
- **Node (`node --check crates/desktop-server/assets/*.js` & `node --test crates/desktop-server/tests/*.test.mjs`):**
  - Setup card states (empty, prefilled Calculate / Prove / Check-a-derivation, same-model warning, no-model-connected banner, revision edit, collapsed summary + `Given` steps).
  - `approval_card.js` renders both `session_capability` and `worker_session` grants, dispatches separate per-grant calls, and never creates auto-approve state.
  - XSS test: `<script>`, `<img onerror>`, `<svg onload>` in statement/conventions/symbols produce zero `innerHTML` writes or injected elements.
- **Guard rails:** `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`, zero network/model calls, zero third-party paper text.

## 7. Document conflicts & resolutions

1. **`solve.md` 4.1 & 9 (S4/S5) vs `00-overview.md` 2.1 (F5), 3.1 row 3, 5.1 & base `routes_session.rs:143,170`:** F5 owns `session_mode`, home-screen cards, New menu, rail tags, and shrinking `routes_session.rs` `< 560` lines; S4 adds the 2-line post-F5 `mode: "solve"` creation + `GET /api/sessions/{id}` seam (resolving base `routes_session.rs:143,170` rejection).
2. **`solve.md` D5 & `00-overview.md` 3.1 row 13 vs `agents/f5-f7-mode-shell.md`:** `00-overview.md` 3.1 row 13 says F5 settles whether `UiApprover` can prompt outside a running turn and section 9 leaves `approval_card.js` owner open, while `f5-f7-mode-shell.md` omits `UiApprover` verification. **Resolution:** S4/S5 explicitly verifies `UiApprover` pre-turn behaviour (or triggers a tiny pre-S5 F5 follow-up) and **S5 is designated sole owner of `assets/approval_card.js`**.
3. **`solve.md` 2/D4/D21 vs `00-overview.md` 0.1, 8.1, 8.3 (q9, q11):** Lean is cut; `"Computed"` is retired in favour of `"Verified"` (exact symbolic) and `"Checked numerically"` (numeric); `"prove"` remains with an explicit setup-card warning and default different-model review criterion.
4. **S2 local `QuoteRef` (`solve_model.rs:161-170`) vs F4 `quote_core::QuoteRef` & F9 `run_budget::SolveRunState` (`run_budget.rs:446`):** Resolved in Checkpoint S4a by re-exporting `quote_core::QuoteRef` and mapping `SolveRunStatus` onto `SolveRunState` / `RunState`.

## 8. Out of scope & hand-back

- **Out of scope:** Tool additions (`solve/1` stays unbuilt until S7), running model turns (S12), derivation cards / Why popover / Expressions tab (S13), `handoff.packet` prefill (F7), earlier Research migration (S21), F16a store implementation (`lib-f13a-f16a-f15`), Lean/S16 (cut), Cadabra2/FORM/S22 (deferred).
- **Hand-back:** Follow `agents/RULES.md` — write `docs/design/modes-2026-11/reports/solve-s4-s5-setup.md` (including the `UiApprover` pre-turn verification finding), run all Rust, Node, golden, and file-length checks, and push only to `gemini/modes/solve-s4-s5-setup`.
