# Brief solve-s14-s20-review-blind — Solve Verifier Loop (S14) & Blind Compute (S20)

**Status:** Queued right behind **`S12`** (`solve-s12-loop`, with `S9` + `S11` merged). **Sequential inside this branch (`S14 -> S20` in order)**; file-disjoint from `solve-s13-s17-ui-intake`, `solve-s21-migrate`, `solve-s15-cli-parity`, and `f15-writeup-solve`! **`RULES.md` authorizes `agent_models_tests.rs` extraction + Solve verifier resolver in `agent_models.rs`.** Zero `Cargo.lock` edits!
**Push branch:** `gemini/modes/solve-s14-s20-review-blind`

---

## 1. Package & Governing Design

- **Packages (strict order):**
  1. **S14:** Split `crates/agent/src/verifiers.rs` (`836`L -> `<= 450`L) & `crates/desktop-server/src/agent_models.rs` (`509`L, extract tests first), shared host `compute_review_independence` (`00-overview.md` 2.9 & 9), `IsolatedWorkerFactory` trait (`agent` stays free of `crates/worker`), `verifier_tools.rs` + `verifier_loop.rs` (`caller = "verifier:solve"`, 12-call cap, `report_verdict` citation integrity, `verifier.incomplete`), `desktop-server/src/solve_workers.rs` (`IsolatedWorkerFactory` + `POST /api/sessions/{id}/solve/review`), and Solve `SameModel` verifier resolution (D2).
  2. **S20 (starts immediately after CP1–CP3 of S14 pass):** Gate **G3** in `crates/research-tools/src/solve_gates.rs`, `crates/agent/src/delegate_blind.rs` (`compute_blind`, isolated worker via `IsolatedWorkerFactory`, check-process `independent_recompute` emitting `"exact": true` only for symbolic equality -> `Tier::Verified` else `Tier::CheckedNumerically`), and wiring in `solve_loop.rs` + `solve_workers.rs`.
- **Governing design:** `00-overview.md` 0.1, 2.2, 2.9, 9; `solve.md` 4.1, 4.3, 4.4 (rules 3–4), 5.2–5.4 (G3, G6), 6 (R8, R13), 7.2–7.3, 8.4, 9 (`S14`, `S20`), 12 (risks 8, 10, 14, 15), 13 (D2, D3); `agents/RULES.md`.

---

## 2. Checkpoints — Push After Each!

1. **Checkpoint S14a — Zero-behaviour-change splits (`verifiers.rs` `836`L -> `verifiers.rs` + `verifier_context.rs` + `verifier_parse.rs`; `agent_models.rs` -> `agent_models_tests.rs`):** Commit alone; verify all existing verifier/model tests pass.
2. **Checkpoint S14b (`compute_review_independence` in `research_tools::solve_model_records`):** Pure host function (`DifferentProvider`, `DifferentModel`, `SameModel`, fail-closed to `SameModel`), never trusting model strings (R3, R13).
3. **Checkpoint S14c (`verifier_tools.rs`, `verifier_loop.rs`, `solve_workers.rs`, `agent_models.rs`, `solve_loop.rs`):**
   - `verifier_loop.rs`: stable `"verifier:solve"` caller, up to 12 calls, `16_384` tokens, union closure clipped deepest-first, lazy isolated worker via `IsolatedWorkerFactory`, verifies `checks_cited` against `claim_sha256` & `math_sha256` (rejecting mismatches, R13), appends `proof.verification` or `verifier.incomplete`.
   - `solve_loop.rs`: host-handled `review_steps { entry_ids: [1..5] }` + finish-time review (`prose_only` default, `all`, `final_only`, `off`) + G6 (`verification_stuck` after 2 rejections on same lineage).
   - `solve_workers.rs`: `IsolatedWorkerFactory` over `crates/worker` (`#[cfg(not(windows))]` graceful test path) + `POST /api/sessions/{id}/solve/review` in `api_dispatch.rs`.
   - `agent_models.rs`: Solve verifier resolver returns a transport even when `verifier == solver` (`SameModel`, D2), keeping Research helpers untouched.
4. **Checkpoint S20a (Gate G3 + `delegate_blind.rs`):**
   - Gate G3 in `research_tools::solve_gates.rs`: refuse `compute_blind` if `compare_with` appears in `inputs` or `operation`.
   - `crates/agent/src/delegate_blind.rs`: host-handled `compute_blind`, isolated worker via `IsolatedWorkerFactory` (never sees `compare_with`), check-process comparison -> `solve.check { name: "independent_recompute" }` honoring S2 `solve_own_tier.rs:142-148`.

---

## 3. Owned Files & Do-Not-Edit (`<= 600` Lines Each)

- **New:** `crates/agent/src/{verifier_context,verifier_parse,verifier_tools,verifier_loop,delegate_blind}.rs` (+ `*_tests.rs`), `crates/desktop-server/src/{agent_models_tests,solve_workers}.rs`, `crates/desktop-server/tests/solve_review_blind.rs`.
- **Small edits:** `crates/research-tools/src/{solve_model_records,solve_gates}.rs`, `crates/agent/src/{lib,verifiers,solve_loop}.rs`, `crates/desktop-server/src/{lib,agent_models,routers,turn_runner,api_dispatch}.rs`.
- **Do NOT edit:** `Cargo.lock`, `crates/agent/Cargo.toml` (NO `worker` dep!), `tool_defs.txt`, `tool_profiles.txt`, `autonomous.rs`, `runner.rs`, `http_srv.rs`, `routes_session.rs`, `handoff.rs`.

---

## 4. Mandatory Verification Before Every Push (`RULES.md`)

Run on Linux **as non-root (`id -u != 0`) with `RE_SEARCH_PYTHON` unset**:
```bash
python3 scripts/check_file_length.py --base origin/gemini/work/fixed
unset RE_SEARCH_PYTHON
cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or
cargo test --workspace --locked --no-fail-fast
```
Verify D2 same-model review, R13 worker isolation (`sympy` var isolated), citation forgery rejection, G6 `verification_stuck`, G3 `compare_with` refusal, `independent_recompute` exact vs numeric tiering. Write `docs/design/modes-2026-11/reports/solve-s14-s20-review-blind.md` and push to `gemini/modes/solve-s14-s20-review-blind`. End with `PUSHED`.
