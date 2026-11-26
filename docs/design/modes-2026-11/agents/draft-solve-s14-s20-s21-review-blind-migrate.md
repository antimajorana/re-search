# Brief draft-solve-s14-s20-s21-review-blind-migrate — Verifier loop (S14), Blind compute (S20) & Research migration (S21)

**Draft status:** Planning brief — **S14 and S20 MUST be serialized** (S14 merges before S20 starts), while **S21 is file-disjoint** and may run in parallel with S14 (or S20) once S4, S11/S12, and F5 are merged.
**Packages:**
- **S14:** Verifier loop with tools (`verifiers.rs` split first, `verifier_loop.rs`, `verifier_tools.rs`, `IsolatedWorkerFactory` trait + `desktop-server/src/solve_workers.rs`, `POST .../solve/review` route, shared host `independence` function, `review_steps`, `report_verdict`, `agent_models.rs` split-tests + Solve verifier resolver).
- **S20:** Blind compute (`compute_blind`, `delegate_blind.rs` via `IsolatedWorkerFactory`, host comparison `independent_recompute`, Gate G3 in `crates/research-tools/src/solve_gates.rs`).
- **S21:** Earlier Research sessions `"Open in Solve"` (`solve_migrate.rs`, fork + `solve.started` + prefill + legacy step tiers per D19).
**Governing design:** `00-overview.md` 0.1, 2.2 (F1), 2.9 & 9 (`independence` built with S14), 4.3 & 5.3 (S21 scope vs F7/F5), 8.1–8.3; `solve.md` 4.1, 4.3, 4.4 (rule 3 `independent_recompute`, rule 4 `independence`), 5.2–5.4 (G3, G6), 6 (R13, R8, R4), 7.2–7.3, 8.4, 9 (rows S14, S20, S21), 12 (risks 8, 10, 14, 15), 13 (D2, D3, D19); `agents/RULES.md`.
**Read reports first:** `reports/s2-solve-model.md`, `reports/f1-tool-profiles.md`, and (once merged) `reports/solve-s4-s5-setup.md`, `reports/solve-s7-s8-s9-s11.md`, `reports/solve-s12-loop.md`, `reports/f5-f7-mode-shell.md`.

## 1. Start conditions & serialization gates

- **Required merges on `origin/gemini/work/fixed`:**
  - Already merged on base `832eab5bf`: F0, F1, F2, F4, F9, S2.
  - **Before S14 starts:** `solve-s7-s8-s9-s11` (S9 checks + S11 G6 in `research_tools::solve_gates`), `solve-s12-loop` (S12 `solve_loop.rs` host-handled tool seam), and `f5-f7-mode-shell` (F5 `api_dispatch.rs` for `POST .../solve/review`).
  - **Before S20 starts (HARD SERIALIZATION GATE):** **S14 MUST merge first** (or S20 runs as checkpoint 2 on the same sequential branch after S14 is pushed and verified). **Never run S14 and S20 concurrently:** both wire host-handled tools into `crates/agent/src/solve_loop.rs` and isolated worker injection into `crates/desktop-server/src/solve_workers.rs`, and S20 adds G3 to `crates/research-tools/src/solve_gates.rs`!
  - **Before S21 starts (parallel-safe):** `f5-f7-mode-shell` (F5), `solve-s4-s5-setup` (S4 prefill), and `solve-s7-s8-s9-s11` + `solve-s12-loop` (so `solve/1` is available to open and run the migrated session). S21 shares ZERO files with S14 or S20 and may run in parallel.

## 2. Interfaces: base-verified vs future-merge

**Verified on base `832eab5bf`:**
- **`crates/agent/src/verifiers.rs` is 878 lines (OVER THE 600-LINE LIMIT!):**
  - Existing exports (`verify_step`, `review_step`, `review_answer_claim`, `verify_full_solution`, `verify_full_solution_cancellable`, `verify_delivered_answer`) + context helpers (`lines 65-336`) + parsing helpers (`lines 379-452`). MUST be split first in a dedicated zero-behaviour-change commit!
- **CRITICAL CRATE-BOUNDARY FACT (`crates/agent/Cargo.toml` — ZERO `worker` DEP!):**
  - `crates/agent/Cargo.toml` does **NOT** depend on `crates/worker`, and `Cargo.lock` must stay untouched!
  - **Resolution:** Define a trait `pub trait IsolatedWorkerFactory: Send + Sync` (spawning a fresh isolated Python worker / check-process runner and returning a drop-on-exit handle) inside `crates/agent` (e.g. `verifier_tools.rs`), and implement + inject it from `crates/desktop-server/src/solve_workers.rs` (wired from `routers.rs` / `turn_runner.rs`)! Both S14 (`verifier_loop.rs`) and S20 (`delegate_blind.rs`) consume `IsolatedWorkerFactory` without adding any dependency to `crates/agent/Cargo.toml`!
- **`crates/desktop-server/src/agent_models.rs` is 546 lines (close to 600) & verifier-role semantics:**
  - Inline tests occupy `lines 450-546` — **extract them into `agent_models_tests.rs` FIRST** so `agent_models.rs` drops to ~450 lines before adding Solve verifier routing!
  - `AGENT_ROLE_KEYS` (`lines 18-31`) currently lists `step_verifier` and `full_verifier` — there is **NO `"verifier"`** key! Add `"solve_verifier"` (or `"verifier"`) cleanly without breaking existing `AGENT_ROLE_KEYS` tests.
  - **CRITICAL D2 REQUIREMENT:** Existing Research verifier helpers may skip/return `None` when verifier == primary model. The new Solve verifier resolver MUST **NOT** return `None` when `roles.verifier == roles.solver` — per `solve.md` D2, same-model verification MUST still run and be labelled `ReviewIndependence::SameModel` (`"Model agreed (same model)"`)!
- **`solve.md` 12 risk 15 verified on base (`runner.rs:181,200,257,264`):** Host-handled tool specs (`ask_user`, `set_session_title`, `delegate_*` — and in Solve: `review_steps`, `compute_blind`, and verifier-internal `report_verdict`) live only in provider `tool_specs`, outside `router.registry().defs()` and `tool_defs` drift checks. Neither `tool_defs.txt` nor `tool_profiles.txt` changes.
- **CRITICAL S2 `solve_own_tier.rs:142-148` CONTRACT FOR S20 (`independent_recompute`):**
  - `is_exact_comparison` grants `Tier::Verified` ONLY when `solve.check.evidence["exact"] == true` or `evidence["comparison"] == "exact"` (and `numeric_coefficients != true`); otherwise it fails closed to `Tier::CheckedNumerically`. S20 MUST emit `"exact": true` only when the check process confirmed exact symbolic equality, and `"exact": false` / `"comparison": "numeric"` for tolerance-based agreement!
- S2 (`solve_model_records.rs` & `solve_own_tier.rs`): `ReviewIndependence { DifferentProvider, DifferentModel, SameModel }`, `SolveReview`, and legacy Research review/receipt folding (D19).
- `crates/export/src/lib.rs` (132 lines): `fork_session(data_dir, &ForkOptions)` reused by S21.

**Future-merge interfaces to verify before coding:**
- From **S9 / S11** (`solve-s7-s8-s9-s11`): `solve_checks.rs` (running `check_run` as `verifier:solve` and host comparison for `independent_recompute`) and `crates/research-tools/src/solve_gates.rs` (G6 stuck tracking + G3 insertion slot).
- From **S12** (`solve-s12-loop`): `solve_loop.rs` host-handled tool dispatch hook and `model.request` `run_id` / `caller` attribution (`"verifier"`, `"worker"`).
- From **S6** (`q-f11-x1-s3-s6`): third worker process role (`verifier` worker: started lazily per review loop, stopped at loop end, zero shared state with solver worker).

## 3. Smallest implementation checkpoints (push after each)

### Lane 1 — S14 (Verifier loop, `IsolatedWorkerFactory`, `POST .../solve/review` & host `independence`)
1. **Checkpoint S14a — MANDATORY ZERO-BEHAVIOUR-CHANGE SPLITS (`verifiers.rs` 878 -> 3 files `<= 450` lines; `agent_models.rs` 546 -> ~450 lines):**
   - Split `crates/agent/src/verifiers.rs` into `verifiers.rs`, `verifier_context.rs` (`problem_context`, ancestor slicing, XML & linked-check formatting), and `verifier_parse.rs` (`raw_verdict`, `parse_verifier_response`).
   - Move `crates/desktop-server/src/agent_models.rs` inline tests (`lines 450-546`) into `crates/desktop-server/src/agent_models_tests.rs`.
   - Zero behaviour change; run `cargo test -p agent -p desktop-server` and `check_file_length.py` before proceeding. Commit alone.
2. **Checkpoint S14b — Shared host `independence` function (`00-overview.md` 2.9 & 9):**
   - Implement one pure host function `compute_review_independence(solver: &RoleModel, reviewer: &RoleModel) -> ReviewIndependence` (in `research_tools::solve_model_records` or `research_tools::solve_model`, returning `DifferentProvider`, `DifferentModel`, or `SameModel`, fail-closed to `SameModel` when ambiguous). Never read from model output (R3, R13).
3. **Checkpoint S14c — `verifier_loop.rs`, `verifier_tools.rs`, `solve_workers.rs`, `POST .../solve/review`, `agent_models.rs`:**
   - `crates/agent/src/verifier_tools.rs`: defines `IsolatedWorkerFactory` trait + verifier tool specs (`ledger_query` clamped to target union closure, `expr_show`, `sympy_eval` routed to the isolated verifier worker, `check_run` as caller `"verifier:solve"`, `read_paper_lines`, `search_paper_passages`, and terminal `report_verdict`).
   - `crates/agent/src/verifier_loop.rs`: stable caller id `"verifier:solve"` (`solve.md` 0.3, 5.3 — NEVER `verifier:<batch>`), up to 12 model calls, `16_384` tokens, union closure input clipped deepest-first to budget, lazily spawns and drops the isolated verifier Python worker via `IsolatedWorkerFactory` (`solve.md` 4.3, R13), validates every `checks_cited` id exists and matches the step's `claim_sha256` & `math_sha256` (rejecting fabricated/mismatched citations per R13), appends `proof.verification` with host-computed `entry_sha256`, `independence`, `verifier_model`, `checks_cited`, `model_calls_used`; on model-call cap exhaustion appends `verifier.incomplete` (steps stay `Unchecked`).
   - Wire host-handled `review_steps { entry_ids: [1..5] }` + finish-time review batching (`review: prose_only | all | final_only | off`, default `prose_only`, 5 steps/batch, capped by `max_review_loops`, G6 pause after 2 rejections on same step lineage) in `solve_loop.rs`.
   - `crates/desktop-server/src/solve_workers.rs`: implements `IsolatedWorkerFactory` using `crates/worker` (with Linux `#[cfg(not(windows))]` graceful test/stub path), wires injection in `routers.rs` / `turn_runner.rs`, AND provides auth-guarded **`POST /api/sessions/{id}/solve/review`** (serving S13 Why popover `"Review with another model"`, registered via 1-line arm in F5 `api_dispatch.rs`).
   - In `crates/desktop-server/src/agent_models.rs`: add Solve verifier role resolution that returns a transport even when `roles.verifier == roles.solver` (D2: same-model runs with `"Model agreed (same model)"`).

### Lane 2 — S20 (Blind compute & G3 — starts ONLY after S14 merges)
4. **Checkpoint S20a — Gate G3 (`research-tools::solve_gates`) & `delegate_blind.rs`:**
   - Add Gate **G3** to **`crates/research-tools/src/solve_gates.rs`** (matching C8 location in `research-tools`!): refuse `compute_blind` if `compare_with` appears in `inputs` or its handle name/text appears in `operation` (tripwire; `solve.md` 5.4).
   - `crates/agent/src/delegate_blind.rs`: host-handled `compute_blind { inputs: [handle], operation (<=600 chars), compare_with: handle, tolerance }`. Spawns an isolated blind worker via S14's `IsolatedWorkerFactory` (seeing ONLY `inputs` handles + `operation`, never `compare_with`), harvests its output handle via S7 `solve_exprs`, runs comparison in the **check process** (never in the worker), appends `solve.check` with `name: "independent_recompute"`, `"exact": true` ONLY for exact symbolic comparison and `"exact": false` for tolerance numeric comparison (S2 `solve_own_tier.rs` contract!).
   - Wire `compute_blind` into `solve_loop.rs` and `solve_workers.rs`.

### Parallel-safe Lane 3 — S21 (Earlier Research sessions "Open in Solve")
5. **Checkpoint S21a — `crates/desktop-server/src/solve_migrate.rs`:**
   - `POST /api/sessions/{id}/open-in-solve` (auth-guarded, registered via 1 line in F5 `api_dispatch.rs`): forks an earlier Research session via `export::fork_session`, appends host-written `solve.started` + prefilled `solve.setup` (first prompt, pinned conventions, existing `assumption` entries, ingested sources, `origin: NodeRef` pointing back to the Research session).
   - Original Research ledger is untouched and keeps resuming under `research/1`.
   - Old steps keep their ids and compute tiers through S2 `compute_all_tiers` (D19: old accepted reviews -> `ModelAgreed` or `ModelAgreedSameModel`, old SymPy validations -> receipts only, old `lean_check` -> receipt `"Lean check recorded in the earlier session"` with no tier).

## 4. Owned files

- **S14:** `crates/agent/src/{verifiers.rs, verifier_context.rs, verifier_parse.rs, verifier_loop.rs, verifier_tools.rs}` (+ `*_tests.rs`), `crates/desktop-server/src/{agent_models.rs, agent_models_tests.rs, solve_workers.rs}`, small hook in `solve_loop.rs`, `routers.rs` / `turn_runner.rs`, F5 `api_dispatch.rs` (+1 `/solve/review` arm), `research-tools/src/solve_model_records.rs` (`compute_review_independence`).
- **S20 (after S14 merges):** `crates/agent/src/delegate_blind.rs` (+ `_tests.rs`), G3 addition in **`crates/research-tools/src/solve_gates.rs`**, `compute_blind` hook in `solve_loop.rs` & `solve_workers.rs`.
- **S21 (parallel-safe):** `crates/desktop-server/src/solve_migrate.rs` (+ `tests/solve_migrate.rs`), 1-line arm in F5 `api_dispatch.rs`, `desktop-server/src/lib.rs`.

## 5. Shared registration seams & Do-not-edit files

- **Serialization rule:** S14 and S20 MUST NOT run concurrently because both touch `solve_loop.rs` and `solve_workers.rs` (and S20 touches `research-tools/src/solve_gates.rs`). Zero changes to `Cargo.lock`, `tool_defs.txt`, or `tool_profiles.txt`.
- **Do NOT edit:** `Cargo.lock`, `crates/agent/Cargo.toml` (NO `worker` dependency!), `crates/research-tools/tests/golden/tool_defs.txt`, `crates/research-tools/tests/golden/tool_profiles.txt`, `crates/agent/src/autonomous.rs`, `crates/agent/src/runner.rs`, `crates/desktop-server/src/http_srv.rs` (702), `crates/desktop-server/src/routes_session.rs` (F5), `crates/export/src/handoff.rs` (F7).

## 6. Tests through the entry point

- **S14 (Rule R13, D2, D3):**
  - `verifiers.rs` & `agent_models.rs` splits keep 100% of existing verifier and agent-model tests green.
  - `compute_review_independence`: same provider+model -> `same_model` (`ModelAgreedSameModel`); same provider different model -> `different_model`; different provider -> `different_provider` (`ModelAgreed`).
  - Same-model Solve verifier resolver returns a live transport (does NOT return `None`!) and stamps `SameModel` (D2).
  - Worker isolation via `IsolatedWorkerFactory`: a Python variable set in the solver worker is undefined in the verifier worker.
  - `report_verdict` citing a non-existent or wrong-step `check_id` is rejected; hitting 12 model calls writes `verifier.incomplete` and leaves steps `Unchecked`; `ledger_query` refuses steps outside target closure; caller id is always `"verifier:solve"`; `POST .../solve/review` runs behind `AuthGuard::authorize_api`; G6 pauses with `verification_stuck` after 2 rejections on same lineage.
- **S20 (Gate G3, Rule R8, S2 contract):**
  - G3 in `research_tools::solve_gates` refuses `compute_blind` when `compare_with` is in `inputs` or named in `operation` (recording `guardrail.blocked`).
  - Blind worker prompt/inputs never contain `compare_with`; check process runs comparison and writes `independent_recompute`: `"exact": true` -> `Tier::Verified`; `"exact": false` -> `Tier::CheckedNumerically` (never `Verified`).
- **S21 (D19, Set R):**
  - Forking a **real legacy Research ledger** containing `proof.entry`, `sympy_eval` tool receipt, `lean_check` tool receipt, and `proof.verification` produces a valid Solve session with `solve.started`, `solve.setup.origin` `NodeRef`, untouched parent ledger (still passes `eval_set_r` resume under `research/1`), and exact D19 tiers (no computed tier for old SymPy, no tier for old `lean_check`, `ModelAgreedSameModel` / `ModelAgreed` for old review).
- **Guard rails:** `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`, zero network/model calls, zero third-party paper text.

## 7. Document conflicts & resolutions

1. **`crates/agent/Cargo.toml` has no `worker` dependency vs `solve.md` 4.3/5.3/9 (S14 & S20 spawning isolated workers):** Adding `worker` to `agent` changes `Cargo.toml` / dependency graph. **Resolution:** Define `IsolatedWorkerFactory` trait in `crates/agent` and implement/inject it from `crates/desktop-server/src/solve_workers.rs`.
2. **`solve.md` 9 S20 (`crates/agent/src/solve_gates.rs`) vs `mcp-server/Cargo.toml` (C8):** `solve_gates.rs` lives in `crates/research-tools/src/solve_gates.rs`, so S20 adds G3 there!
3. **`00-overview.md` 2.9 & 9 (`independence` in S14) and 4.3 (`handoff.packet` in F7, S21 limited to earlier Research sessions).**
4. **`solve.md` 12 risk 15 ("host-handled tool specs outside drift check"):** Verified true on base `832eab5bf` (`runner.rs:181,200,257,264`).
5. **`agent_models.rs` (546 lines, no `"verifier"` key, skips same-model in legacy Research) vs `solve.md` D2:** Extract `agent_models_tests.rs` first and add a D2-compliant Solve verifier resolver that runs same-model with `SameModel` independence.

## 8. Out of scope & hand-back

- **Out of scope:** `handoff.packet` prefill (F7), CLI route verifier invocation (S15), Lean/S16 (cut), Cadabra2/FORM/S22 (deferred), F15 write-up (Group 5), F16b lessons (Group 4).
- **Hand-back:** Write `docs/design/modes-2026-11/reports/solve-s14-s20-s21-review-blind-migrate.md` (with `verifiers.rs` & `agent_models.rs` split line counts, `IsolatedWorkerFactory` seam, R13/G3/D19 test counts) and push only to `gemini/modes/solve-s14-s20-s21-review-blind-migrate` (or separate branches if S21 runs independently).
