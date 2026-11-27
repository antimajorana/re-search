# Brief solve-s12-loop — Solve Execution Loop, Pause & Resume Brief (S12)

**Status:** Critical-path Solve lane queued immediately behind **`S11`** (`solve-s8-s9-s11`). (`S13–S17`, `S14/S20`, `S15`, `S21`, `F16b` wait for `S12`.) Zero `Cargo.lock` edits! `RULES.md` explicitly authorizes isolated seam commits in `resume_profile.rs`, `turn_runner.rs`, and `session_mode.rs`.
**Push branch:** `gemini/modes/solve-s12-loop`

---

## 1. Package & Governing Design

- **Package:** **S12** — Solve execution loop (`solve_loop.rs`), task-kind prompts (`solve_prompts.rs`), mode-neutral `SessionOutcome::Paused` (`runner.rs`, `cli/src/api_session.rs`, `turn_runner.rs`), deterministic `resume_brief.rs` with F16b `lessons.supplied` hook (R24), `resume_mode: "brief"` rule R2 (`resume_profile.rs`), `POST /api/sessions/{id}/solve/continue` + startup/`POST .../solve/reconcile` (`solve_turn_routes.rs`), and enabling `DESKTOP_MODE_SOLVE_ENABLED = true`.
- **Governing design:** `00-overview.md` 2.2 (F1, R2), 2.14 (F16b hook, R24), 3.1 (rows 10, 11), 3.3 (F9), 6; `solve.md` 4.1, 5.2, 5.5, 6 (R2, R12, R16, R24), 7.2–7.4, 8.2, 9 (`S12`), 12 (risks 6, 7, 14, 15), 13 (D6, D8, D15, D20); `THREAT-MODEL.md:333`; `agents/RULES.md`.

---

## 2. Verified Line Counts & Interfaces on `gemini/work/fixed`

- **`crates/agent/src/runner.rs` (`582`L unless already split by `f14c`):** Check line count before adding `SessionOutcome::Paused`. Split `dispatch_tool_calls` + helpers (`record_session_title`, `collect_text`, `clip_text`) into `crates/agent/src/dispatch.rs` first if `runner.rs > 540` lines!
- **Exhaustive `SessionOutcome` match sites:** (1) `crates/cli/src/api_session.rs` (`521`L, `map_outcome`), (2) `crates/desktop-server/src/turn_runner.rs` (`537`L). Update BOTH in Checkpoint S12b!
- **`crates/agent/src/resume_profile.rs` (`310`L):** Rejects profile transitions on replay resume; keep `crates/agent/src/resume.rs` (`535`L) untouched!
- **`crates/desktop-server/src/session_mode.rs` (`230`L):** `const DESKTOP_MODE_SOLVE_ENABLED: bool = false;` currently disables Solve turns (keep `mode_shell.rs` passing cleanly; check `reports/solve-s4-s5-setup.md` §5 for exact `mode_shell.rs` expectation when `DESKTOP_MODE_SOLVE_ENABLED` flips to `true`).
- **`crates/research-tools/src/run_budget.rs` (`464`L) & S7/S11 (`solve_gates.rs`, `solve_done.rs`, `solve_exprs.rs`):** Bind S7's `CheckCanonicalizer` + S8/S9/S11 `SolveCheckHost` + `research_tools::solve_gates::SolveGate` cleanly without adding `worker` to `crates/agent/Cargo.toml`!

---

## 3. Smallest Checkpoints — Push After Each!

1. **Checkpoint S12a (`runner.rs` split if `> 540`L -> `dispatch.rs`, zero behaviour change):** Commit alone; verify tests pass.
2. **Checkpoint S12b (Mode-neutral `SessionOutcome::Paused { turns: u32, reason: String }`):** Update `runner.rs`, `crates/cli/src/api_session.rs`, and `crates/desktop-server/src/turn_runner.rs` cleanly without changing Research or Investigate behavior.
3. **Checkpoint S12c (`resume_brief.rs` & `solve_prompts.rs`):**
   - `crates/agent/src/resume_brief.rs`: pure, byte-deterministic `build_resume_brief(events, upto_seq, budget_chars) -> String`: setup, active step table (`id`, `type`, `own`, `effective`, summary, handles, deps), union closure full detail for latest steps + 1-line summary for older steps, open items, handle list, last 4 turns verbatim, `"Python variables do not survive; only handles do"`. Render any frozen `lessons.supplied` blob verbatim as quoted data under `"Notes from the user's earlier corrections"` (never read live `lessons.db`, R24!).
   - `crates/agent/src/solve_prompts.rs`: concise task-kind system prompts (`calculate`, `prove`, `check_derivation`); leave `RESEARCH_SYSTEM_PROMPT` untouched.
4. **Checkpoint S12d (`solve_loop.rs` + `solve_turn_routes.rs`):**
   - `crates/agent/src/solve_loop.rs`: in-core Solve loop with `"tool_profile": "solve/1"`, `run_id`, `"caller": "solver"` on `model.request`, `SolveGate` (G1/G4/G5/G6/G7), all `solve.paused` reasons (`leg_cap`, `budget`, `gate_bounces`, `verification_stuck`, `approval_needed`, `worker_restarted`, `user`, `interrupted`, `transport_error`) returning `SessionOutcome::Paused` (NEVER `session.abort`, R16), auto-checkpoint at `>= 60%` prompt budget (`solve.checkpoint { reason: "context" }`, 0 summarizer model calls), 1 repair round on unmet criteria at finish (D6), worker restart -> restage S7 handles -> `approval_needed` on epoch change, and clean host-handled tool hook seams for future **S14** (`review_steps`) and **S20** (`compute_blind`).
   - `crates/desktop-server/src/solve_turn_routes.rs`: `POST /api/sessions/{id}/solve/continue` (accepts `{ note?, correct_entry_id? }`, runs in background, registered in `api_dispatch.rs`), startup `reconcile_interrupted_solve_sessions(state)` + explicit `POST /api/sessions/{id}/solve/reconcile` (`GET solve/view` NEVER mutates ledger per `THREAT-MODEL.md:333`).
5. **Checkpoint S12e (Isolated seam commits — R2 `resume_profile.rs`, `turn_runner.rs`, `session_mode.rs`):**
   - `resume_profile.rs`: allow Solve-only profile transition ONLY when `session.resume` has `"resume_mode": "brief"` with valid `brief_sha256` matching a preceding `solve.checkpoint` (reject replay-resume transition & `research/1 -> solve/1` as `Drift`, R2).
   - `turn_runner.rs` + `routers.rs` + `session_mode.rs`: route `Mode::Solve` turns to `solve_loop` with solver+check `SolveCheckHost` & `CheckCanonicalizer`, flip `DESKTOP_MODE_SOLVE_ENABLED = true`, keeping `Research` and `Investigate` byte-identical.

---

## 4. Owned Files & Do-Not-Edit (`<= 600` Lines Each)

- **New:** `crates/agent/src/dispatch.rs` (if needed), `crates/agent/src/{solve_loop,solve_prompts,resume_brief}.rs` (+ `*_tests.rs`), `crates/desktop-server/src/solve_turn_routes.rs`, `crates/desktop-server/tests/solve_loop_routes.rs`.
- **Small edits:** `crates/agent/src/{lib,runner,resume_profile}.rs`, `crates/cli/src/api_session.rs`, `crates/desktop-server/src/{lib,api_dispatch,routers,session_mode,turn_runner,turn_control}.rs`.
- **Do NOT edit:** `autonomous.rs`, `resume.rs`, `verifiers.rs` (`836`L, S14), `solve.rs` (`S8/S9/S17`), `http_srv.rs`, `routes_session.rs`, `cli_turn.rs` (`S15/X18`), `tests/golden/{tool_defs,tool_profiles}.txt`, `Cargo.lock`.

---

## 5. Mandatory Verification Before Every Push (`RULES.md`)

Run on Linux **as non-root (`id -u != 0`) with `RE_SEARCH_PYTHON` unset**:
```bash
python3 scripts/check_file_length.py --base origin/gemini/work/fixed
unset RE_SEARCH_PYTHON
cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or
cargo test --workspace --locked --no-fail-fast
```
Verify R2 (`resume_mode: "brief"` vs replay `Drift`), R12 (`gate_bounces`), R16 (never `session.abort`), R24 (`build_resume_brief` reads frozen `lessons.supplied` blob only), `eval_set_r`, `resume_loop`, `resume_drift_tests`. Write `docs/design/modes-2026-11/reports/solve-s12-loop.md` and push to `gemini/modes/solve-s12-loop`. End with `PUSHED`.
