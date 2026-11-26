# Brief draft-solve-s12-loop — Solve loop, prompts, pause & resume brief (S12)

**Draft status:** Planning brief (do not launch until start conditions below are merged).
**Package:** S12 (Solve execution loop, short prompts per task kind, mode-neutral `SessionOutcome::Paused` in `runner.rs`, deterministic `resume_brief.rs`, `resume_mode: "brief"` rule R2, `POST .../solve/continue` & `turn_control.rs`, interrupted-leg reconciliation).
**Governing design:** `00-overview.md` 2.2 (F1, R2), 2.14 (F16b `lessons.supplied` hook, R24), 3.1 (rows 10, 11), 3.3 (F9 run-state vocabulary), 6 (post-M3 order); `solve.md` 4.1 (`solve.paused`, `solve.checkpoint`, `lessons.supplied`), 5.2, 5.5, 6 (R2, R12, R16, R24), 7.2–7.4, 8.2, 9 (row S12), 12 (risks 6, 7, 14), 13 (D6, D8, D15, D20); `THREAT-MODEL.md:333` (GET handlers never mutate); `agents/RULES.md`.
**Read reports first:** `reports/f1-tool-profiles.md`, `reports/f4-f9-quote-budget.md`, `reports/s2-solve-model.md`, and (once merged) `reports/f5-f7-mode-shell.md`, `reports/solve-s7-s8-s9-s11.md`.

## 1. Start conditions & governance gates

- **Required merges on `origin/gemini/work/fixed` before S12 starts:**
  - Already merged on base `832eab5bf`: F0, F1 (`resume_profile.rs`), F4, F9 (`run_budget.rs`), S2.
  - **Hard merge gates:** `f5-f7-mode-shell` (F5: `session_mode.rs`, `api_dispatch.rs`, `turn_runner.rs` profile selection) and `solve-s7-s8-s9-s11` (**S11**: `research_tools::solve_gates`, `solve_done`, complete frozen `solve/1` profile).
- **Foundation-owned file gate (`solve.md` 7.2, 9 S12; `RULES.md`):**
  - `crates/agent/src/resume_profile.rs` (310 lines) and `crates/desktop-server/src/turn_runner.rs` (463 lines on base) are foundation-owned seams. This brief authorizes ONLY the minimal, isolated hooks in `resume_profile.rs` (Solve-only `resume_mode: "brief"` rule R2) and `turn_runner.rs` (exhaustive `SessionOutcome::Paused` match in S12b + Solve dispatch branch in S12e, ONLY after F5 merges), each in its own reviewable commit. Keep `crates/agent/src/resume.rs` untouched!

## 2. Interfaces: base-verified vs future-merge

**Verified on base `832eab5bf`:**
- **`crates/agent/src/runner.rs` is 626 lines (OVER THE 600-LINE LIMIT!):**
  - `SessionOutcome` (`runner.rs:38-42`): `{ Completed, Refusal, Aborted }`.
  - **CRITICAL EXHAUSTIVE MATCH SITES FOR `SessionOutcome::Paused`:**
    1. `crates/cli/src/api_session.rs:163-168` (`map_outcome`, 527 lines)
    2. `crates/desktop-server/src/turn_runner.rs:459-462` (463 lines on base)
    Both MUST be updated in Checkpoint S12b so Checkpoint S12b compiles cleanly on its own!
  - `max_turns` abort (`runner.rs:410-417`): emits `session.abort`.
  - `dispatch_tool_calls` (`runner.rs:539-592`), `record_session_title` (594–607), `collect_text` (609–618), `clip_text` (620–626).
  - **Verified `solve.md` 12 risk 15 on base:** `runner.rs:181,200` writes `router.registry().defs()` into `session.start["tool_defs"]`, whereas host-handled tool specs (`ask_user` at line 257, `set_session_title` at line 264, `delegate_*`) are appended only to provider `tool_specs` and are **outside** the `tool_defs` drift check.
- `crates/agent/src/resume_profile.rs` (310 lines): `resolve_session_profile` (lines 43–95) rejects every `session.resume` whose `tool_profile` differs from `session.start` (line 85), with doc comment on line 42 naming S12's resume-brief hook.
- `crates/desktop-server/src/routes_session.rs:392-393`: existing turn endpoint rejects empty prompts — so Solve Continue (bare Continue AND Continue-with-note / `"Correct this step"`) needs its own dedicated `POST /api/sessions/{id}/solve/continue` route!
- `crates/desktop-server/src/turn_control.rs` (198 lines): `ActiveTurnGuard`, `handle_stop_turn`, `stop_all_turns`.
- `crates/agent/src/autonomous.rs` (573 lines): Research/Investigation loop — DO NOT merge Solve into it.
- `crates/research-tools/src/run_budget.rs` (464 lines): `RUN_ID_FIELD`, `count_model_calls`, `SolvePauseReason`, `solve_run_state`.
- `crates/research-tools/src/solve_model.rs`: `event_kinds::{SOLVE_PAUSED, SOLVE_CHECKPOINT, LESSONS_SUPPLIED, SOLVE_DONE}`, `PauseReason`, `SolvePaused`, `SolveCheckpoint`, `CheckpointReason`.

**Future-merge interfaces to verify before coding:**
- From **F5** (`f5-f7-mode-shell`): `turn_runner.rs` mode-to-profile dispatch, `api_dispatch.rs`, and `session_mode.rs`.
- From **S11** (`solve-s7-s8-s9-s11`): `crates/research-tools/src/solve_gates.rs` (`SolveGate` G1/G4/G5/G6/G7 pause signals) and `solve_done.rs`.
- From **S7** (`solve-s7-s8-s9-s11`): handle restaging helper in `solve_exprs.rs` after worker restart.

## 3. Smallest implementation checkpoints (strict order)

1. **Checkpoint S12a — MANDATORY ZERO-BEHAVIOUR-CHANGE SPLIT OF `runner.rs` (626 -> `< 540` lines):**
   - Move `dispatch_tool_calls` (lines 539–592) and its private helpers (`record_session_title`, `collect_text`, `clip_text`) from `crates/agent/src/runner.rs` into new `crates/agent/src/dispatch.rs`.
   - Zero behaviour change. Run `cargo test -p agent`, `eval_set_r`, and `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` before touching anything else! Commit alone.
2. **Checkpoint S12b — Mode-neutral `SessionOutcome::Paused` in `runner.rs` + match sites:**
   - Add `SessionOutcome::Paused { turns: u32, reason: String }` to `crates/agent/src/runner.rs` (`00-overview.md` 3.1 row 10, `solve.md` 5.5).
   - Update exhaustive match sites in `crates/cli/src/api_session.rs:163-168` (`map_outcome`, 527 lines) and `crates/desktop-server/src/turn_runner.rs:459-462` so the entire workspace compiles cleanly at this checkpoint!
   - Keep existing `run_agent_loop` / `autonomous.rs` / Investigate abort paths 100% unchanged (Investigate switches to `Paused` in a later follow-up).
3. **Checkpoint S12c — Deterministic `resume_brief.rs` & `solve_prompts.rs`:**
   - `crates/agent/src/resume_brief.rs` (+ `_tests.rs`): `build_resume_brief(events, upto_seq, budget_chars) -> String`, pure and byte-deterministic (`solve.md` 5.5, R16):
     - Sections: setup, active step table (`id`, `type`, `own` and `effective` tier, summary, handles, deps), full detail for closure of latest steps + one-line summary for older steps, open items (failed checks with counterexamples, rejected reviews, stale steps, unmet done criteria, uninterpreted runs), handle list, last 4 turns verbatim, fixed line `"Python variables do not survive; only handles do"`.
     - **F16b clean hook (R24):** If the ledger contains a `lessons.supplied` event for the current leg, render its blob verbatim as quoted data under `"Notes from the user's earlier corrections"`; **never** read the live `lessons.db` store!
   - `crates/agent/src/solve_prompts.rs`: short, task-kind-specific system prompts (`calculate`, `prove`, `check_derivation`): role, handle workflow, gate error recovery. Freeze `RESEARCH_SYSTEM_PROMPT` untouched for `research/1` (`solve.md` 7.4).
4. **Checkpoint S12d — `solve_loop.rs`, `solve_turn_routes.rs`, checkpointing, Stop/Continue, interrupted reconciliation:**
   - `crates/agent/src/solve_loop.rs` (+ `_tests.rs`): separate in-core Solve loop driving `Transport` + `Router` with S11's `research_tools::solve_gates::SolveGate`:
     - Records `"tool_profile": "solve/1"` and optional `"run_id"` / `"caller": "solver"` on `model.request` (closing F9's attribution gap in `reports/f4-f9-quote-budget.md` section 2 so `solve.done.model_calls.{solver,verifier,workers}` is exact).
     - Enforces all pause reasons (`leg_cap`, `budget`, `gate_bounces`, `verification_stuck`, `approval_needed`, `worker_restarted`, `user`, `interrupted`, `transport_error`) writing `solve.paused` v1 and returning `SessionOutcome::Paused` (NEVER `session.abort` on Solve limits; R16).
     - **Stop vs Continue responsibility:** `turn_control::handle_stop_turn` signals the `CancelToken`; `solve_loop.rs` observes cancellation cleanly and appends `solve.paused { reason: "user" }` (no race writing the ledger from `turn_control.rs`!).
     - Automatic checkpoint at `>= 60%` estimated prompt window at turn boundary (`solve.checkpoint { reason: "context" }`, 0 model calls for summarizing), starting the next leg from `build_resume_brief`.
     - `Continue` / `Continue with a note` / `"Correct this step"`: writes `solve.checkpoint { reason: "continue", brief_blob, brief_sha256 }`, then `session.resume { tool_profile, resume_mode: "brief", brief_sha256 }` with a fresh message list starting from the resume brief (+ optional user note / step-correction note).
     - One automatic repair round on unmet criteria at finish if budget remains (D6), then stop as `finished_open_items`.
     - Worker restart after timeout: restage all handles via S7 `solve_exprs`; epoch changed -> grant void -> `solve.paused { reason: "approval_needed" }` (`"Waiting for you"`).
     - Clean host-handled tool dispatch slots for `ask_user`, `set_session_title`, and future **S14** (`review_steps`) and **S20** (`compute_blind`) without implementing S14/S20 here.
   - `crates/desktop-server/src/solve_turn_routes.rs` (new file — never colliding with `solve.rs`!, registered via 1-line arm in F5 `api_dispatch.rs`):
     - `POST /api/sessions/{id}/solve/continue` (auth-guarded; accepts `{ note?: string, correct_entry_id?: string }` so bare Continue with empty note AND Continue-with-note / `"Correct this step"` both work without hitting `routes_session.rs:392-393` empty-prompt rejection; runs in background while another session is in view).
     - **Interrupted-leg reconciliation (`THREAT-MODEL.md:333` compliant — NEVER mutating inside a read-only `GET`!):** Expose `reconcile_interrupted_solve_sessions(state)` run once at desktop-server startup (marking any Solve session with an open leg and no live lock as `solve.paused { reason: "interrupted" }`) PLUS an explicit `POST /api/sessions/{id}/solve/reconcile` (called by the UI on open if `GET solve/view` reports an unclosed stale leg), so `GET` handlers remain 100% side-effect-free and interrupted legs never auto-re-run!
5. **Checkpoint S12e — Rule R2 `resume_mode: "brief"` & `turn_runner.rs` Solve dispatch:**
   - In `crates/agent/src/resume_profile.rs` (isolated commit): in `resolve_session_profile`, permit a `tool_profile` transition (`solve/1` -> future `solve/2`) **ONLY** when the session is in Solve mode AND `session.resume` carries `"resume_mode": "brief"` with a valid `brief_sha256` matching a preceding host-written `solve.checkpoint`; reject any replay-resume profile switch and any `research/1` -> `solve/1` switch without fork + `solve.started` as `Drift` (R2).
   - In `crates/desktop-server/src/turn_runner.rs` (isolated commit, post-F5): route `Mode::Solve` turns to `solve_loop`, leaving Research and Investigate paths byte-identical.

## 4. Owned files

- **New:**
  - `crates/agent/src/dispatch.rs` (extracted from `runner.rs`)
  - `crates/agent/src/solve_loop.rs`, `solve_prompts.rs`, `resume_brief.rs` (plus `*_tests.rs` siblings `<= 600` lines each)
  - `crates/desktop-server/src/solve_turn_routes.rs` (+ integration test)
- **Small localized edits (`<= 600` lines after S12a split):**
  - `crates/agent/src/runner.rs` (626 -> `< 540` after S12a, then `SessionOutcome::Paused`)
  - `crates/cli/src/api_session.rs` (527: exhaustive `SessionOutcome::Paused` arm in `map_outcome` at lines 163–168)
  - `crates/agent/src/lib.rs` (+4 module exports)
  - `crates/desktop-server/src/lib.rs` (+1 `pub mod solve_turn_routes;`)
  - `crates/desktop-server/src/turn_control.rs` (198 lines — only if Solve active-turn hook needs a helper; Stop still only signals cancel)
  - Isolated seam commits (post-F5): `crates/agent/src/resume_profile.rs` (310), `crates/desktop-server/src/turn_runner.rs` (463 on base), F5 `api_dispatch.rs` (+1 `/solve/continue` & `/solve/reconcile` arm)

## 5. Shared registration seams & Do-not-edit files

- **Seams left for downstream briefs:**
  - `solve_loop.rs`: clean host-handled tool hook enum/callback so **S14** plugs in `review_steps` + finish-time review batching and **S20** plugs in `compute_blind` cleanly (S14 and S20 are serialized).
  - `resume_brief.rs`: reads `lessons.supplied` from the ledger (Group 4 **F16b** only appends `lessons.supplied` at leg start; zero edits to `resume_brief.rs` needed).
  - `POST /api/sessions/{id}/solve/continue`: consumed by S13's Run bar (`Continue`, `Continue with a note`) and Why popover (`"Correct this step"`).
- **Do NOT edit:** `crates/agent/src/autonomous.rs` (573), `crates/agent/src/resume.rs` (535), `crates/agent/src/verifiers.rs` (878, S14), `crates/desktop-server/src/solve.rs` (owned by S4/S8/S9/S17), `crates/desktop-server/src/http_srv.rs` (702), `routes_session.rs` (F5), `cli_turn.rs` (626, S15/X18), `tests/golden/tool_defs.txt`, `tests/golden/tool_profiles.txt`, `Cargo.lock`.

## 6. Tests through the entry point

- **Rule R16 (`solve_loop` + `resume_brief` tests with scripted `Transport`):**
  - Per-leg cap (`max_model_calls_per_leg`) ends with `solve.paused { reason: "leg_cap" }` and `SessionOutcome::Paused`, NEVER `session.abort`.
  - Stop cancellation via `turn_control::handle_stop_turn` causes `solve_loop` to emit `solve.paused { reason: "user" }`.
  - `build_resume_brief` run twice on identical events produces byte-identical output; still byte-identical regardless of external/live state (R24).
  - `POST /api/sessions/{id}/solve/continue` works with empty note AND with a user correction note after a ledger truncated mid-turn at a `tool.request` with no `tool.result`.
  - Auto-checkpoint triggers at `>= 60%` context budget with 0 summarizer model calls.
  - Worker restart restages handles and pauses with `approval_needed` (`"Waiting for you"`).
  - Startup reconciler & `POST /api/sessions/{id}/solve/reconcile` write `solve.paused { reason: "interrupted" }` without auto-re-running, while `GET /api/sessions/{id}/solve/view` never mutates the ledger (`THREAT-MODEL.md:333`).
- **Rule R2 (`resume_profile` + marker tests):**
  - Replay resume attempting `solve/1` -> `solve/2` fails closed with `Drift`.
  - Brief resume (`solve.checkpoint` + `session.resume { resume_mode: "brief", brief_sha256 }`) permits Solve profile transition while rejecting `research/1` -> `solve/1`.
  - Tool-emitted event enumeration confirms no registry tool can emit `solve.started`, `solve.expr`, `solve.check`, `solve.ack`, `solve.done`, `solve.paused`, or `solve.checkpoint`.
- **Rule R12:** scripted transport ignoring gate errors pauses with `gate_bounces` after `max_gate_bounces` and bounds `model.request` count.
- **Regression:** `cargo test --workspace` (including `cli` `api_session`, `eval_set_r`, `resume_loop`, `resume_drift_tests`, `tool_def_golden`, `tool_profile_golden`) and `check_file_length.py`.

## 7. Document conflicts & resolutions

1. **`00-overview.md` 3.1 row 10 (pause at cap):** S12 makes `SessionOutcome::Paused` mode-neutral in `runner.rs` (and updates exhaustive matches in `cli/src/api_session.rs:163-168` and `turn_runner.rs:459-462`) without changing Investigate's current code; Investigate switches later.
2. **`solve.md` 5.5 ("on open, a Solve session whose last leg has no end event gets `solve.paused {interrupted}`") vs `THREAT-MODEL.md:333` (read-only `GET` handlers):** **Resolution:** Reconcile at server startup + explicit `POST /api/sessions/{id}/solve/reconcile`, never mutating inside `GET`.
3. **`routes_session.rs:392-393` (empty prompt rejected) vs `solve.md` 5.5 (one-click `Continue`):** **Resolution:** Dedicated `POST /api/sessions/{id}/solve/continue` in `solve_turn_routes.rs` accepting optional `note` / `correct_entry_id`.
4. **`solve.md` 5.5 ("brief") vs `00-overview.md` 3.2 row 18 ("resume brief"):** **`00-overview.md` wins:** module is `resume_brief.rs` and UI/doc wording is `"resume brief"`.
5. **F16b lessons in `solve.md` 4.1 & 9 vs Group 4 ownership:** S12 reads `lessons.supplied` from the ledger only (R24) and does NOT implement `lessons.db` (Group 4 F16b).

## 8. Out of scope & hand-back

- **Out of scope:** Verifier loop & `review_steps` (S14), blind compute & `compute_blind` (S20), CLI route parity (S15), derivation UI (S13), F16b lessons store/UI (Group 4), switching Investigate to `SessionOutcome::Paused`, Lean/S16 (cut), Cadabra2/FORM/S22 (deferred).
- **Hand-back:** Write `docs/design/modes-2026-11/reports/solve-s12-loop.md` (recording `runner.rs` line counts before/after split and all R2/R12/R16/R24 test results) and push only to `gemini/modes/solve-s12-loop`.
