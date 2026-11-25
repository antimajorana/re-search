# re:Search SOLVE mode: design

Reconciled with 00-overview.md on 2026-11-25; shared packages are referred to by their F ids.
Package ids `S..` are this document's own work packages; `F0` to `F16` are the shared foundation packages
defined in `00-overview.md` section 2 ("the overview" below), built once for all three modes.

Scope revision of 2026-11-25 (owner decisions, overview 0.1), applied below: **Lean and physlib are
cut** (no Lean-checked tier, no `formal_check`, no formal-library search; package S16 cut);
**Cadabra2 and FORM are deferred** (package S22 deferred as a whole); a **write-up** of the
derivation in TeX is added (F15); **corrections** made by the user are reused (F16); papers cited
in a step may come from the user's own library (F13) and from PDF-only papers (F14). The top tier
is Computed. The word "verified" is shown nowhere. Section 0 below is the record of the earlier
review and still mentions Lean, Cadabra2 and FORM; it was not rewritten.

Written 2026-11-25. Built on `reSearch-modes-gaps-and-prior-art-2026-11-25.md` (section 4) and on two
proposals (researcher-first and architecture-first). Code was read at `origin/gemini/work/fixed` with
`git show` and `git grep` only. Nothing was checked out, built or run. Waiting times and model-call counts
below are targets or estimates, not measurements.

How to read this: section 0 is the review that shaped the design. Sections 1 to 13 are the design.

---

## 0. Review of the two proposals

### 0.1 Facts checked in the code

| Claim | Result | Evidence |
|---|---|---|
| Resume rejects any changed, removed or reordered tool definition; additions come from a hand-written list | True | `crates/agent/src/resume.rs::check_tool_defs_drift` (line 191). The allowlist names 7 tool ids and requires effect `ReadWorkspace`. |
| `def_hash` covers id, description, input schema, effect and approval scope | True | `crates/tools/src/registry.rs` line 26. Editing a description is enough to strand old sessions. |
| "`dispatch_tool_calls` is the single choke point for every tool call" (architecture proposal) | **False** | `crates/mcp-server/src/main.rs:101` calls `router.call` directly. Delegated workers (`delegate.rs:367`), explore (the existing `delegate_explore` tool, not the Explore mode), paper readers and the canvas route (`routes_tools.rs:303`) call `Router::try_call` without `dispatch_tool_calls`. A gate placed in `dispatch_tool_calls` does not bind CLI routes or workers. |
| Session approval grants are reused per worker epoch | True, and narrower than both proposals assume | `crates/tools/src/router.rs` lines 170 to 196: a grant is keyed by caller, tool id, def_hash, policy version and worker epoch. A verifier is a different caller, so it needs its own grant. "One click at setup" is not what the code does today. |
| The worker job has no memory cap | True | `crates/worker/src/win.rs:151` sets only `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. |
| `load_expr` rebuilds expressions with `sympify(string)` | True | `python/solver_calc.py:61`. `sympify` on a string evaluates it. Any process that loads a stored expression this way runs text written by the solver's process. |
| Turn cap ends the session with `session.abort` | True | `crates/agent/src/autonomous.rs:325`. |
| Verifier is one call, 8,192 tokens, no tools; the closure slicer exists | True | `crates/agent/src/verifiers.rs` lines 21, 133, 140. The file is 878 lines. |
| Worker allowlist is SymPy, mpmath, NumPy only | True | `crates/worker/src/stage.rs:17`. Bundled Python is 3.14 (`packaging/windows/stage_runtime.py:14`). |
| Mode comes from a ledger marker, not from session metadata | True | `crates/desktop-server/src/routes_session.rs` lines 143, 260, 351 (`investigation.started`). The shared rule is one function, `session_mode` (F5): the latest host-written marker wins. |
| The proof graph reader ignores unknown event kinds and unknown payload fields | True | `crates/research-tools/src/proof_dag.rs` reads fields by key and has a catch-all arm (line 465). |
| Lean check scans text for `sorry`; approval scope is `Once` | True | `crates/research-tools/src/lean_check.rs` lines 72, 137. |
| Files already over 600 lines near this work | True | verifiers.rs 878, runner.rs 626, routes_session.rs 616, cli_turn.rs 628, app.js 932, index.html 768, canvas_dag.js 688, stream.js 660, export/latex.rs 771. |

### 0.2 Where the researcher-first proposal fails

| Problem | Kind | Fix in this design |
|---|---|---|
| A step is Computed if "at least one check passes". A dimension check or a symmetry check alone would give a green badge to a wrong formula. | Unsafe for honesty | Computed needs an identity-class check on every claim of the step (6.1). Other named checks only add sublabels, or turn the step Failed. |
| `step_record` is declared `ObserveLocal` but runs Python checks inside its handler. That executes code under a no-approval effect class. | Weakens the policy engine | `step_record` only writes the ledger. Checks run through `check_run` (`ExecuteCode`, normal policy and approval path), started by the host after the step is recorded. |
| Checks run in the same Python process as the model's code. Model code can replace `simplify`, `latex` or a helper in `sys.modules`, by accident or not, and every later check and every displayed formula inherits it. | Unsafe for honesty | A separate check process that never runs model code, fed only by a restricted parser of stored expressions (4.3). |
| "The formula is rendered from the checked handle" but the LaTeX is produced in the solver's process. | Unsafe for honesty | LaTeX is produced by the check process from the stored expression. |
| Cited sits inside the grade order (between Computed and Model agreed), so a Computed step that uses a cited formula is shown as "Cited". | Wrong model | Given and Cited are neutral: they do not enter the minimum. They are listed under "rests on". |
| The finish gate refuses to finish until every criterion is met. A model that cannot meet one criterion burns calls until the budget pauses it. | Slow, costly | Finishing is always allowed when nothing in the closure is Failed or Stale; the outcome is "Done" or "Done with open items", computed by the host. |
| Gate G1 allows nothing after one successful run except recording it. Normal work is run, look, adjust, run. | Slow | Up to 3 uninterpreted runs are tolerated (architecture proposal). |
| New `proof.entry` types `cited` and `plan_item` | Breaks old readers' assumptions | No new entry types. A cited step is `reference_lookup` with a source reference. A plan is its own event. |
| A new `compute` tool duplicating `sympy_eval`, a new `exec.receipt` event duplicating `tool.request`/`tool.result`, `plan_propose`, `artifact_view`, `step_supersede`, `step_get`, 24 packages | Over-built | `sympy_eval`, `ledger_supersede`, `ledger_query` are reused unchanged. Receipts are a view. |
| "Approval asked at setup, 4 clicks" | Vague | See the grant key above. The design lists every grant on the setup card (decision D5). |
| `formal_check` and `engine_run` frozen into `solve/1` before Cadabra2 or FORM are known to run on Windows | Risky for resume | `engine_run` is left out of `solve/1`. Profiles can change at a leg boundary (8.2), so adding it later is safe. |

### 0.3 Where the architecture-first proposal fails

| Problem | Kind | Fix in this design |
|---|---|---|
| Gates in `dispatch_tool_calls`; no Solve on CLI routes in the first release | Ignores "model access is the user's own"; and the stated reason is a false fact | Gates live in the Router, so they bind every route. CLI parity is an early package. |
| Checks and the verifier use a "namespace" inside the solver's worker process | Unsafe for honesty (same as above) | Separate processes. |
| The same tool id `lean_check` with two definitions (v1 and v2), plus a behaviour change that can turn an old "verified" into "rejected" | Fragile, surprising | `lean_check` is untouched. Solve gets a new id, `formal_check`. |
| A failed check can be "cleared" by a later pass from an "equal or stronger author" | Vague, exploitable | A fail with a counterexample is permanent for that claim. The way out is to supersede the step with a corrected claim or a stated domain. |
| Gate G4 forces a strategist call after two rejections | Costs model calls the user did not ask for; the strategist role may not be configured | Replaced by a pause with reason `verification_stuck`. |
| 7 kinds of done criterion, 12 routes, `solve.input`, `stage_input`, `delegate_compute` v2 in the core path | Over-built for a first release | 5 criterion kinds; inputs and blind compute are later packages. |
| The evaluation set is the last package | Wrong order | It is built first: set Q of the shared evaluation set, package F8 (previously called S0). |
| `rendered_latex` stored on the step from the solver worker's output | Unsafe for honesty | Rendered by the check process. |
| Verifier caller id `verifier:<batch>` | Each batch would be a new caller and need a new approval | One stable caller id `verifier:solve`. |

### 0.4 What is kept from each

- From the researcher: the worked example and minute-by-minute view; badge text rules ("verified" reserved for Lean); auto-checks with zero extra model calls; flags that are never hidden; gates in the Router; receipts keyed to the claim so a corrected justification costs nothing downstream; "Correct this step"; honest list of what code cannot close.
- From the architect: tool profiles (now the shared package F1); reuse of `sympy_eval` with an unchanged definition; receipts as a view; derived dependencies from handle ownership; the invariant-and-test table; the bounce counter; the deterministic resume brief with no provider replay; the 600-line bookkeeping (now the shared package F0).

---

## 1. What this mode is for

1. You set up a calculation, a proof, or a derivation you want checked; the app carries it out as numbered steps.
2. Every step shows how it was checked: by the computer (computer algebra and numerics), by a second model, by you, or not at all.
3. A step never looks better than the weakest thing it rests on.
4. The model never writes a check result, a badge or a formula on screen. The host computes all three from records it made itself.
5. You can stop, correct one step, continue, and take the result out as a notebook or as a written-up derivation in TeX (F15). A correction you make can be kept for later runs (F16).

Terms used throughout:

- **Ledger**: the existing append-only, hash-chained event log of a session.
- **Step**: one `proof.entry` event (the existing proof-graph node).
- **Handle**: a named, versioned, exact expression stored in the ledger (for example `V1_msbar@2`). Models pass handles instead of retyping formulas.
- **Claim**: a relation between two handles attached to a step (for example `lhs = rhs`).
- **Check**: a host-shipped test run on a claim, with a recorded outcome.
- **Receipt**: the record of one code execution (what ran, on what inputs, with what result and versions).
- **Tier**: the kind of checking a step has passed. **Badge**: the tier as shown to the user.
- **Closure** of a step: the step plus everything it depends on, directly or indirectly.
- **Model call**: one request to the model provider. Every cap and every screen counts model calls. A **turn** is the internal unit of the loop (one model call plus the tool calls it asks for); the word is used only for internals.
- **Leg**: one uninterrupted stretch of model turns (up to 48 model calls). **Resume brief**: the deterministic text the next leg starts from (previously called "brief"; in Explore, "brief" means the sharpened idea).
- **Tool profile**: a frozen, named list of tool definitions that a session records and resumes with. The mechanism is the shared package F1; Solve's profile is `solve/1`.
- **Matched quote**: a quote that the shared quote core (F4) found word for word at the stated lines of an ingested source. The word "verified" is reserved for a machine-checked formal proof; since Lean was cut it is shown nowhere, and quotes are "matched". A quote found in the text of a scanned page is "matched in OCR text" (overview 2.12).
- **QuoteRef**, **PaperRef**, **NodeRef**: the shared records that point at a quote, a paper and a node of any session (overview 2.3 and 2.5; fields in 4.1 below).
- **Hand-off packet**: the one shared event (`handoff.packet`, F7) with which one mode opens a session in another.

---

## 2. A worked example

Task: "Compute the one-loop effective potential V_1(phi) of massless lambda phi^4 theory, V_tree = lambda phi^4/4!,
in d = 4 - 2 eps with MS-bar, and show mu dV/dmu = 0 at order lambda^2."
Expected: V_1 = m^4/(64 pi^2) (ln(m^2/mu^2) - 3/2) with m^2 = lambda phi^2/2.

**0 s.** Home screen, **Solve** card (overview 5.1). Paste the statement. No model call.

**Under 1 s.** The setup card is filled in from local data only:

- Kind: Calculate (guessed from "compute"; three-way switch).
- Symbols found in the statement: lambda, phi, mu, eps, d, m. Each has an assumption menu (default "real, positive" for lambda, mu, m).
- Conventions: copied from the pinned conventions (Euclidean signature, hbar = c = 1). The user adds "MS-bar".
- Dimensions table (optional): phi, m, mu: mass^1; lambda: mass^0.
- Done checklist, prefilled: "final result is Computed"; "everything it rests on is at least Model agreed"; "dimensions check passes on the final result"; and from the word "show": "named check `vanishes_to_order` passes (mu dV/dmu, order lambda^2)".
- Models: solver, verifier. If they are the same: "Same model: reviews will be labelled as weaker".
- Approvals needed for this run, one line each (see D5).

The user presses Run. The host writes `solve.setup`, one `assumption` step per convention and assumption (badge Given), and warms the two Python processes.

**About 5 to 20 s.** First solver turn. Steps appear as cards:

| Step | What the solver does | What the host does | Badge shown |
|---|---|---|---|
| e-1, e-2 | nothing | written from the setup | Given |
| e-3 | records "V_1 = (1/2) Int d^dk/(2 pi)^d ln(k^2 + m^2)" as an assumption (no source attached) | nothing | Assumed by model (needs your OK) |
| e-4 | `sympy_eval`: does the d-dimensional integral, `save_expr("I_d", ...)`. Then `step_record` with claim `V1_d = I_d` | harvests handle `I_d@1`; renders its LaTeX in the check process | Unchecked (this is the definition of `V1_d`; nothing to compare it with) |
| e-5 | `sympy_eval`: numeric quadrature of the second m^2-derivative at d = 3, m = 1.3, saved as `num_d3`; a second run evaluates the closed form there, saved as `cf_d3`. `step_record` with value claim `num_d3 ~ cf_d3` | auto-check `identity_numeric`: the two numbers came from two different runs and agree to 12 digits | Computed (two runs agree, same solver) |
| e-6 | `sympy_eval`: expands in eps, `save_expr("V1_series", ...)`; `step_record` with claim `I_d(d = 4 - 2 eps) = V1_series` to order eps^0 | auto-check `series_match` passes | Computed (series to O(eps^0)) |
| e-7 | removes the pole and the MS-bar constants, `save_expr("V1_msbar", ...)`; claim `V1_series - pole_terms = V1_msbar + O(eps)` | auto-checks `series_match`, `dimensions` (mass^4) | Computed (series, dimensions) |
| e-8 | calls `check_run` on e-7 with `symmetry` {phi -> -phi} | runs it | e-7 gains sublabel "symmetry" |
| e-9 | `step_record`, type result: `mu d/dmu (V_tree + V1_msbar) + beta dV_tree/dlambda = 0` to order lambda^2, with beta = 3 lambda^2/(16 pi^2) recorded as its own assumption e-8b | auto-check `vanishes_to_order` passes | Computed |
| e-10 | `step_record`, type final_answer, depends on e-7 and e-9; then `solve_finish` | evaluates the done checklist | see below |

**The honest ending.** e-7 is Computed, but its closure contains e-3 (Assumed by model) and e-4 (Unchecked).
So the effective badge of the final answer is:

> Computed. Rests on: e-3 "one-loop formula" (assumed by the model, not acknowledged), e-4 (unchecked), e-8b "beta function" (assumed by the model).

The run ends as **Done with open items**. The result sheet offers three ways forward, each one click:
"Acknowledge e-3" (you know the formula), "Attach a source for e-3" (it becomes Cited with a matched quote),
"Review e-4 with the verifier" (it becomes Model agreed). With e-3 and e-8b acknowledged and e-4 reviewed by a
different model, the sheet reads "Done: 4 computed, 1 model agreed, 3 given by you."

**If a check fails.** Suppose the solver drops a factor 1/2 in e-6. `series_match` returns fail with
"lhs - rhs = 0.0312 at m = 1.7, mu = 0.9, order eps^0". The card turns red with that line. The solver sees the
same text as the result of its `step_record` call and must supersede e-6. Steps recorded on top of the failed
one show "Broken: rests on failed e-6".

**If the per-leg limit is reached.** The run bar turns amber: "Paused: model-call limit for this leg, at e-9. [Continue]". One click.

**Estimated model calls** for this run: 12 to 18 by the solver, 0 for auto-checks, 1 verifier loop (up to 12
model calls, usually 3 to 5) if the user asks for e-4 to be reviewed. Today's worst case is 48 turns times 3
verification rounds.

**Variant: check a derivation.** The user drops a `.tex`. Within a second the equations appear as numbered
steps, all Unchecked (deterministic split, no model). The solver's only job per step is to restate each
equation as handles. Each card shows "your TeX" next to "as checked", with "transcription (model),
unconfirmed [Confirm]". The check between consecutive equations is run by the host. A planted factor-of-2
error shows as one red step.

---

## 3. What the user sees, stage by stage

| Stage | Waiting time (target) | Model calls | On screen |
|---|---|---|---|
| A. Solve card on the home screen | under 0.3 s | 0 | Empty setup card in the centre stage (not a dialog). The home screen with its three cards is built in F5; the card's button says "Run" |
| B. Statement typed or pasted | under 1 s after typing stops | 0 | Live KaTeX preview; kind guessed; symbol table; conventions from pins; default done checklist; toolbox line (SymPy, NumPy, mpmath versions) |
| C. Run pressed | 1 to 3 s | 0 | Setup collapses to one line. Given steps appear in the Derivation view and the graph. Approval lines resolve. Both Python processes warm up in the background (the first check should not pay the SymPy import) |
| D. Solving | first step card within the first model turn (5 to 20 s, provider dependent) | 1 per turn | Step cards appear as recorded. Run bar (the shared `run_bar.js` of F6): "Running. Step 5: checking series (CAS), 02:14, 9 of at most 60 model calls". Never a bare timer |
| E. Auto-check on a step | under 1 s typical; hard cap 10 s per check, then "inconclusive" | 0 | Badge spinner, then badge with sublabels |
| F. Review of prose steps | 20 to 90 s per batch of up to 5 steps | up to 12 per batch, usually 3 to 5 | "Reviewing e-4 (verifier, model X)" |
| G. Pause | immediate | 0 | Amber bar "Paused: (reason)" with Continue, Continue with a note. An approval shows "Waiting for you"; a lost app or provider shows "Interrupted" with Retry (state names in 5.5) |
| H. Finish | under 1 s after the last step | 0 for the checklist; 1 optional review batch | Result sheet: answer rendered from the final step's handles, effective badge, checklist with links, "rests on" list, the solver's own summary with the suffix "(model)" |
| I. Afterwards | re-run all checks: seconds, 0 model calls | 0 | Write up (the derivation as TeX or Markdown with its check table, F15), Export (Notebook, HTML, ZIP), Fork from step, Correct this step (with "Remember this?", F16), Review with another model |

Views (plain-JS modules, see 7.3):

- **Derivation** (main column): numbered step cards in dependency order. A card has the equation (KaTeX, from the handle), a one-line reason, the badge, depends-on chips, and a caret for code, receipts and checks.
- **Badge**: always text plus an icon, never colour alone (drawn with the shared `ui_badge.js` of F6). A second line appears when the effective tier is lower than the step's own: "rests on e-4 (unchecked)".
- **Why popover** (click a badge): each check with outcome, numbers (residual, points, digits), who asked for it, versions, "show code"; the ancestors that lower the tier; actions: Re-run checks, Add check, Review with another model, Correct this step, Acknowledge assumption, Attach a source (5.1), Fork from here.
- **Canvas tabs**: Graph (the shared graph view of F6, filter "closure of the final answer"), Expressions (handle table), Sources, Log (today's conversation stream and tool trace). Artifacts arrives with package S18.
- **Graph rules** (shared, overview 2.6): `solve_view` returns the shared `GraphView` shape. An edge runs from the step relied on (`src`) to the step that relies on it (`dst`), drawn top to bottom; this is `depends_on` and `derived_depends_on` read directly. A solid border means checked by code or backed by a matched quote (Computed, Cited); a dashed border means not checked by code (Model agreed, Assumed by model, Unchecked). Given and Checked by you are drawn plain with the tag "you" (overview 2.10). Failed and Stale have no border style of their own; their text badges say it. Each node carries its effective tier as a badge (text plus icon).
- **Display grade** (shared, overview 2.10), so the same look means the same thing in every mode: Computed is "By code"; Cited is "Matched quote, full text" (or "Matched quote, scanned page" with the tag "OCR" when the source is a scan); Model agreed and Assumed by model are "Model statement"; Given and Checked by you are "User statement".

Badge texts: Computed; Model agreed; Model agreed (same model); Checked by you; Unchecked;
Failed; Given; Cited; Assumed by model; Stale. The word "verified" appears nowhere (a quote is
"matched"). The word "proved" never appears on a computed step. Model text follows the two shared patterns:
a verdict starts "Model agreed"; any other model text carries the suffix "(model)".

---

## 4. Data model

Everything is a ledger event. Views are pure functions of the events. Rust types go in
`crates/research-tools/src/solve_model.rs`; the fold in `solve_state.rs`; tiers in `solve_status.rs`.

### 4.1 New and changed events

| Event | Written by | Fields |
|---|---|---|
| `solve.started` v1 | host only, at session creation, after a fork, or after a `handoff.packet` (8.4) | `{}`. The mode marker, same role as `investigation.started`. Mode is read by the shared `session_mode` (F5): the latest of `investigation.started`, `idea.started`, `solve.started` wins; none means `research` |
| `solve.setup` v1 | host, on Run and on each revision | `revision`, `statement` (up to 16,000 chars), `task_kind` (`calculate`, `prove`, `check_derivation`), `target {description, form: closed_form/series/numeric/bound/proof/per_step_verdict, order?, digits?}`, `done: [DoneCriterion]`, `conventions` (up to 2,000 chars, existing validator), `symbols: [{name, assumptions}]`, `dimensions: {symbol: {base: power}}`, `assumptions: [{id, text}]`, `given_source?: {source_id, file_sha256}`, `sources: [source_id]`, `roles: {solver, verifier, compute?}` each `{provider, model}`, `budgets: {max_model_calls: 60, max_wall_minutes: 30, max_model_calls_per_leg: 48, max_review_loops: 6, max_gate_bounces: 3}`, `review: prose_only/all/final_only/off`, `tool_profile: "solve/1"`, `origin?: NodeRef` (the node or session this run was opened from; shared record, below), `math_sha256` (hash of symbols, dimensions, assumptions and conventions: the part checks depend on) |
| `solve.plan` v1 | host, from an optional `plan` field of the first `step_record` calls | `items: [{title}]`. Display only. No tier, no gate |
| `proof.entry` v1 (existing, additive fields) | `step_record`; host for setup and given steps | existing: `entry_id`, `entry_type` (the 9 existing types, none added), `summary`, `details`, `depends_on`, `tool_call_ref`. New, all optional: `claims: [Claim]`, `tool_call_refs: [call_id]`, `derived_depends_on: [entry_id]`, `claim_sha256`, `entry_sha256`, `origin: solver/setup/given/user/worker`, `source_ref?: QuoteRef` (shared record, below), `transcription?: {source_tex_sha256}` |
| `solve.expr` v1 | host only | `name` (`^[A-Za-z_][A-Za-z0-9_]{0,63}$`), `version`, `srepr_blob`, `srepr_sha256` (of the canonical form written by the check process), `latex` (from the check process), `str_preview` (up to 400 chars), `free_symbols: [{name, assumptions}]`, `produced_by: {call_id, result_seq}`, `owner_entry?`, `portable: bool` |
| `solve.check` v1 | host only | `check_id`, `claim_sha256`, `claim_index`, `entry_id` (informational), `math_sha256`, `name` (registry), `registry_sha256`, `params`, `outcome: pass/fail/inconclusive/not_run`, `asked_by: host/solver/verifier/user`, `receipt: {call_id, result_seq}`, `evidence` (residual, points, digits, order, counterexample point, dimensions), `evidence_sha256` |
| `proof.verification` (existing, additive fields) | verifier loop | new: `entry_sha256`, `independence: different_provider/different_model/same_model` (computed by the one host function all modes share, overview 2.9), `verifier_model`, `checks_cited: [check_id]`, `model_calls_used` |
| `solve.ack` v1 | host, on a user click | `entry_id`, `kind: assumption/transcription/hand_checked`, `entry_sha256` |
| `solve.paused` v1 | host | `reason: leg_cap/budget/gate_bounces/verification_stuck/approval_needed/worker_restarted/user/interrupted/transport_error`, `leg`, `turn`, `model_calls_used`, `open_items`. How each reason is shown: 5.5 |
| `solve.checkpoint` v1 | host | `leg`, `upto_seq`, `brief_blob`, `brief_sha256` (the resume brief), `reason: continue/context` |
| `evidence.quote` (shared kind, F4; id `xe-<seq>`) | host, when a quote is attached to a step | the stored matched quote a Cited step points at. Not a Solve kind: defined in overview 2.5 |
| `context.supplied` (shared kind, F4) | host | `{run_id, caller, ranges}`: the source lines the app put in front of a model. Defined in overview 2.5 |
| `handoff.packet` v1 (shared kind, F7) | host, in the receiving route | read by the setup prefill (8.4). Defined in overview 4.1 |
| `lessons.supplied` (shared kind, F16) | host, at the start of a leg | `{run_id, lesson_ids, blob}`: the notes from the user's earlier corrections that were put in front of the model. The resume brief reads them from this event, not from the live store. Defined in overview 2.14 |
| `solve.done` v1 | host, in `solve_finish` | `final_entry_id`, `own_tier`, `effective_tier`, `criteria: [{id, met, evidence}]`, `rests_on: {given, cited, assumed_by_model, weakest: [{entry_id, tier}]}`, `status: finished/finished_open_items`, `model_calls: {solver, verifier, workers}` |
| `guardrail.blocked` (existing kind, new fields) | Router gate | `gate`, `tool`, `call_id`, `message` |
| `solve.artifact` v1 (package S18) | host | `artifact_id`, `name`, `mime` (by magic bytes), `blob`, `sha256`, `bytes`, `call_id`, `entry_id?` |

Smaller structures:

- **Claim**: `{form: equation/inequality/value/series, lhs: "name@v", rhs: "name@v", relation: eq/le/ge/approx, order?: {variable, point, n}, domain?: text, how?: text}`.
- **claim_sha256**: hash of the claims with the `srepr_sha256` of every handle they name. Prose is excluded: checks test mathematics. `solve.check` also stores `math_sha256` so a setup revision that changes assumptions makes old checks stale.
- **entry_sha256**: hash of claims, summary and details. Reviews and acknowledgements bind to this, because a reviewer judged the prose too.
- **DoneCriterion**: `{id, kind, params}` with kinds `final_tier_at_least {tier}`, `closure_tier_at_least {tier}`, `named_check_passed {check, on: final/all_results}`, `no_open_flags {}`, `user_confirms {text}`.
- **QuoteRef** (shared, F4): `{source_id, path, start_line, end_line, file_sha256, quote_sha256, evidence_seq}`. On a Cited step, `evidence_seq` names an `evidence.quote` event in the same session. The line fields are `start_line` and `end_line` in every mode.
- **NodeRef** (shared, F2): `{session, mode, id, paper: Option<PaperRef>, node_fp: Option<String>}`. A pointer across sessions is always a `NodeRef`, never a bare id. Solve ids keep their form inside a session: `e-N` for a step, `name@version` for a handle, the check id.
- **Receipt** (a view, not an event): built from the existing `tool.request`/`tool.result` pair. The handler adds `code_sha256`, `wall_ms`, `process: solver/check/verifier`, `exprs_out`, `env` (Python, SymPy, NumPy, mpmath versions and helper hashes) to the result JSON. The tool definition does not change.

Why checks are keyed by claim and not by step: if the solver supersedes a step only to fix its wording or its
dependencies and re-records the same claim, the passing checks still apply. Nothing is re-run and no model
call is spent. If any handle in the claim changes, the hash changes and the checks no longer count.

### 4.2 Derived state (never stored)

`SolveState { setup, entries, handles, checks, receipts, tiers, pending_runs, gate_bounces, upto_seq }` with
`apply(&mut self, &Event)` so the Router gate updates it per event instead of re-reading the session.

`StepTier { own, effective, lowered_by: [entry_id], sublabels, flags, stale }`.

### 4.3 Three Python processes

All three are the existing sandboxed runtime (AppContainer, no network). They differ in what they are allowed to run.

| Process | Runs | Lifetime | Why separate |
|---|---|---|---|
| Solver worker (exists today) | the solver's `sympy_eval` code | the session; restart on timeout | unchanged |
| Check process (new) | only host-shipped check code from `python/checks/`; parses stored expressions; produces canonical `srepr` and LaTeX | the session; started at Run | model code never runs here, so it cannot alter a check, a pass criterion or a rendered formula |
| Verifier worker (new) | the verifier's `sympy_eval` code | one review loop; started lazily, stopped at loop end | the verifier must not share state with the solver |

Stored expressions enter the check process and the verifier worker through a **restricted `srepr` parser**
(`python/checks/srepr_safe.py`): the text is parsed with `ast`, and only calls to an allowlist of SymPy
constructors with literal or nested-call arguments are accepted. No attribute access, no names outside the
list, no `sympify` on strings. An expression that does not parse is stored with `portable: false`: the
solver can keep using it, but no claim may name it, so it can never carry a badge.

Reference machine: at most three Python processes at a time. A job memory cap is added with package S18
(today there is none).

### 4.4 How a badge is computed

Tier order, low to high:

`Unchecked (0) < Model agreed, same model (1) < Model agreed (2) = Checked by you (2) < Computed (3)`

Computed is the top tier. The Lean-checked tier (4) was cut on 2026-11-25. A proof in prose can
therefore reach at most Model agreed or Checked by you; only claims between handles are computed.

Outside the order: **Failed** (overrides everything), **Given** (setup assumption or acknowledged model
assumption), **Cited** (`reference_lookup` whose `source_ref` is a matched quote: accepted by `verify_quote` of the shared quote core, F4),
**Assumed by model** (an assumption with origin solver, not acknowledged; counts as Unchecked).

**Own tier of step s.** Only records that match the step count: checks with `claim_sha256` and `math_sha256`
equal to the current ones; reviews and acknowledgements with `entry_sha256` equal to the current one.

1. **Failed** if any matching check has outcome fail, or the latest matching review is rejected. A fail is never cleared; the step must be superseded.
2. (Removed with the Lean-checked tier. The number is kept so that "rule 3" below keeps its name.)
3. **Computed** if the step has claims and every claim has a passing identity-class check:

   | Claim form | Checks that count | Sublabel |
   |---|---|---|
   | equation | `identity_symbolic` | "symbolic" |
   | equation | `identity_numeric` (at least 12 points, 50 digits, points drawn from the declared assumptions and domain) | "numeric only" |
   | series | `series_match` to the order stated in the claim; `vanishes_to_order` | "series to O(x^n)" |
   | value | `identity_numeric` where lhs and rhs were produced by two receipts with different `code_sha256` | "two runs agree, same solver" |
   | value, equation | `independent_recompute` (blind task, package S20) | "independent recompute" |
   | inequality | none. `counterexample_search` passing gives the sublabel "sampled, no counterexample" and the tier stays Unchecked | |

   Other named checks (`dimensions`, `limit_case`, `symmetry`, `conservation`, `convergence`, `matrix_property`) never give Computed. Passing adds a sublabel. Failing gives Failed.
4. **Model agreed** or **Model agreed (same model)** if the latest matching review is accepted, by its `independence`. **Checked by you** if a matching `hand_checked` acknowledgement exists.
5. **Given**, **Cited**, **Assumed by model** by the rules above.
6. Otherwise **Unchecked**. A computation step with a successful receipt and no claim is Unchecked with sublabel "ran".

An accepted review never lifts a step above Model agreed. If the verifier ran checks, those checks lift the
step through rule 3, like anyone else's.

**Effective tier.** Steps are appended in dependency order (enforced on append today), so one forward pass is enough:

`effective(s) = min(own(s), effective(d) for d in depends_on UNION derived_depends_on)`, skipping Given and Cited.

- `derived_depends_on` is computed by the host in `step_record`: the owner step of every handle the claims name, and of every receipt cited. A step cannot look better by leaving out a dependency.
- If any dependency is Failed: effective is shown as "Broken: rests on failed e-N".
- If any dependency is superseded: the step is **Stale** (effective Unchecked) until re-recorded. Re-recording the same claim is one tool call and keeps its checks.
- `lowered_by` lists the dependencies that attain the minimum; the UI shows them as "rests on".

**Flags**, shown on the badge and never hidden: "transcription unconfirmed", "same model", "numeric only",
"rests on N assumptions made by the model", "OCR source" (a Cited step whose quote was matched in the text
of a scanned page).

---

## 5. Tools

Tool profile `solve/1`: one row of the shared profile table (overview 2.2, package F1), whose tool list is
the table in 5.1. "Who" names the callers allowed by the Solve policy preset; the preset belongs to the
profile, so resume compares the right policy version.

### 5.1 Registry tools (drift-checked)

| Tool | Input | Output | Effect, approval | Who |
|---|---|---|---|---|
| `sympy_eval` (unchanged definition) | `{code}` | value, stdout, error; new result keys `exprs`, `code_sha256`, `wall_ms`, `env` | ExecuteCode, WorkerSession | solver, verifier (own worker) |
| `step_record` (new) | `{entry_type, summary (<=500), details (<=16000), depends_on, tool_call_refs, claims, source_ref?, plan?}` | `{entry_id, derived_depends_on, rendered: [latex per claim], auto_checks: [{check, outcome, evidence}], own_tier}` | ObserveLocal | solver |
| `check_run` (new) | `{entry_id, claim_index?, check, params}` | the `solve.check` record | ExecuteCode, WorkerSession (epoch of the check process) | solver, verifier, `host:check`, user |
| `expr_show` (new) | `{name?, version?, form: latex/str/srepr, max_chars <= 4000}` | text, or the handle list | ObserveLocal | solver, verifier |
| `solve_finish` (new) | `{final_entry_id, summary_markdown}` | `{status, criteria, rests_on}` or a refusal listing the Failed or Stale steps | ObserveLocal | solver |
| `ledger_supersede`, `ledger_query`, `ledger_note` (unchanged) | as today | as today | as today | solver; `ledger_query` also verifier (answers limited to the closure it was given) |
| `read_paper_lines`, `search_paper_passages`, `arxiv_search`, `literature_search`, `fs_read`, `fs_list`, `check_expression_sanity`, `tex_audit` (unchanged) | as today | as today | as today | solver; the two paper readers also verifier |

Not offered in Solve: `ledger_append` (replaced by `step_record`), `lean_check` and
`search_formal_library` (Lean and physlib are cut; nothing replaces them, and `formal_check` is not built),
`fs_write`, `latex_patch`, `latex_compile`, the Investigation recorders (the four new Investigate tools
live only in profile `investigation/2`), the Explore tools `idea_search` and `idea_fetch_papers`.

What `step_record` refuses (each is a tool error the model sees at once):

- a claim naming an unknown, superseded-version or non-portable handle;
- a `latex` field (the tool has none: formulas come from handles);
- a computation type without at least one successful receipt in `tool_call_refs`;
- `derivation`, `result` or `final_answer` with empty `depends_on`;
- `final_answer` whose closure contains a Failed step;
- a `reference_lookup` whose `source_ref` (a `QuoteRef`) does not name an `evidence.quote` record of this session, or whose quote no longer passes `verify_quote` (F4), or whose lines were never sent to the caller that cites it (`lines_sent` over `context.supplied` events and line-returning tool results, F4). Without a source the model must use `assumption`, which shows as Assumed by model.

Attaching a source ("Cite a paper", overview 4.2): the user picks a `PaperRef` from the shared library (F10),
which includes the papers of the user's own sources (F13: a Kandinsky vault, a folder, a `.bib` file),
or types an arXiv id (fetched through F3). The paper may be PDF-only: the lines are then lines of the
page-broken PDF text and the chip names the page (F14); a quote from a scan gives the flag "OCR source"; the user or the model points at lines; `verify_quote` runs; the
host writes `evidence.quote` and the step becomes Cited. The Cited chip (shared `quote_chip.js`, F6) opens the
paper in Investigate at the node whose span contains the lines. A quote shown or exported is at most 300
characters. Solve does not call `investigation_evidence.rs`: that code refuses to run without an open
Investigation request.

What `check_run` does: the host builds the request from the registry entry for `check` and the handles the
claim names, sends it to the check process, maps the returned evidence to an outcome with a fixed rule, and
writes `solve.check`. The model chooses the check and its parameters. It cannot pass code, an expected
value or an outcome.

Auto-checks: after a successful `step_record` with claims, the Router's post-call hook issues `check_run`
as caller `host:check` through the normal policy and approval path: the identity-class check for the claim
form (symbolic first, numeric if symbolic is inconclusive), then `dimensions` if the setup has a
dimensions table. Each is capped at 10 s and returns inconclusive on timeout. The outcomes are appended to
the `step_record` result. If the grant is missing the outcome is `not_run` and the step stays Unchecked.

Check registry v1 (`python/checks/`, each a pure function over handles, hashed into `registry_sha256`):
`identity_symbolic`, `identity_numeric`, `series_match`, `vanishes_to_order`, `counterexample_search`
(package S9); `dimensions`, `limit_case`, `symmetry`, `conservation`, `convergence`, `matrix_property`
(package S10). `identity_symbolic` returns fail only when a numeric counterexample point exists; a nonzero
residual that `simplify` cannot reduce is inconclusive.

### 5.2 Host-handled tools (in-core loop only)

| Tool | Input | Output | Notes |
|---|---|---|---|
| `review_steps` | `{entry_ids: [1..5]}` | per step `{verdict, issues, checks_cited}` | Starts the verifier loop on the union closure. Counts against `max_review_loops`. |
| `ask_user`, `set_session_title` | as today | as today | unchanged |
| `compute_blind` (package S20) | `{inputs: [handle], operation (<=600 chars), compare_with: handle, tolerance}` | `{agreed, evidence, output: handle}` | The worker sees only `inputs`. The host compares in the check process and writes an `independent_recompute` check. |

### 5.3 Verifier loop

`crates/agent/src/verifier_loop.rs`. Caller id `verifier:solve`. Transport and model from `roles.verifier`.
Up to 12 model calls, 16,384 tokens. Input: setup, the closure of the target steps (ids, types, rendered LaTeX,
summaries, details clipped deepest-first to a character budget), the checks that already exist. Tools:
`ledger_query`, `expr_show`, `sympy_eval` (verifier worker), `check_run`,
`read_paper_lines`, `search_paper_passages`, and the terminal tool `report_verdict {verdicts: [{entry_id,
verdict, summary, counterexample?, issues, checks_cited}]}`. The host rejects a verdict that cites a check
that does not exist or does not match the step. Running out of model calls records `verifier.incomplete`; the
steps stay Unchecked.

When it runs: when the solver calls `review_steps`; when the user clicks "Review"; and once at finish for
closure steps below what the done checklist requires, according to `review` in the setup (default
`prose_only`: steps with no claims, batched 5 per loop).

### 5.4 Gates on every tool call

A `tools::Gate` hook inside `Router::try_call`, before policy evaluation. A gate can only refuse; it can
never turn a policy deny or an unanswered approval into an allow. A refusal is returned as a tool error
`GATE <id>: <what to do next>` and recorded as `guardrail.blocked`.

| Gate | Rule |
|---|---|
| G1 interpret | While more than 3 successful runs are neither cited by a step nor discarded (`ledger_note` starting "discard:"), only `step_record`, `expr_show`, `ledger_query`, `ledger_note` and read tools are allowed. Failed runs need no interpretation. |
| G2 receipts | enforced inside `step_record` (5.1). |
| G3 blind | `compute_blind` is refused if `compare_with` is among `inputs` or its text appears in `operation`. A tripwire; the real protection is that the worker never receives it and the host does the comparison. |
| G4 budget | When `max_model_calls`, `max_wall_minutes` or `max_model_calls_per_leg` is reached, the next model call is not sent; `solve.paused`. Model calls are counted from ledger events by the one shared function of F9 (`run_budget.rs`), not by Solve code. |
| G5 bounce | `max_gate_bounces` refusals in a row: `solve.paused {gate_bounces}`. Stops a confused model from spending calls on the same refused action. |
| G6 stuck | Two rejected reviews on the same step lineage: `solve.paused {verification_stuck}` with the reviewer's issues shown. |
| G7 finish | `solve_finish` is refused if the closure of the final step has a Failed or Stale step. Otherwise it writes `solve.done`. The existing `lint_honesty_with_ledger` runs on `summary_markdown`: "verified" is refused in a Solve summary, and "proved" must match the tiers. |

On CLI routes the host cannot stop the CLI from printing a final text. The result sheet is built from the
ledger, so it reads "Not done: 3 steps unchecked" whatever the text says.

### 5.5 Long runs

- **Pause, not abort.** Solve legs end with `solve.paused` for every limit. `SessionOutcome::Paused` is a new variant, added mode-neutral in the runner (package S12) so that Investigate can switch to it afterwards.
- **Run states on screen** (the shared vocabulary of overview 3.3, shown by the shared run bar and the rail):

  | State on screen | Solve run |
  |---|---|
  | Running | a leg is running; elapsed time and model calls used are shown |
  | Waiting for you | `solve.paused {approval_needed}` |
  | Paused: (reason) + Continue | `solve.paused` with `leg_cap`, `budget`, `gate_bounces`, `verification_stuck`, `worker_restarted`, `user` |
  | Interrupted + Retry | `solve.paused` with `interrupted` or `transport_error`; never re-run unasked |
  | Done | `solve.done {finished}` |
  | Done with open items | `solve.done {finished_open_items}` |

  The state "Failed + Retry" is not used by a Solve run; "Failed" in Solve is a step tier.
- **Resume brief** (`crates/agent/src/resume_brief.rs`): `build_resume_brief(events, upto_seq, budget_chars) -> String`, pure. Contents: setup; table of active steps (id, type, own and effective tier, summary, handles, dependencies), full detail for the closure of the latest steps and one line for the rest; open items (failed checks with counterexamples, rejected reviews, stale steps, unmet criteria, uninterpreted runs); handle list; the last 4 turns verbatim; the fixed line "Python variables do not survive; only handles do". Same events give the same bytes.
- **Continue**: `solve.checkpoint` then `session.resume {tool_profile, resume_mode: "brief", brief_sha256}` and a fresh message list containing the resume brief. No provider-response replay.
- **Automatic checkpoint**: when the estimated prompt passes 60% of the model's window, the host ends the leg at a turn boundary and starts the next from the resume brief. Zero model calls spent on summarising.
- **Worker restart** (after a timeout kill): the host restages every handle from `solve.expr`. The epoch changed, so the grant is void and the run shows "Waiting for you" (`approval_needed`). This is today's rule, kept.
- **Interrupted**: on open, a Solve session whose last leg has no end event gets a host-written `solve.paused {interrupted}` and shows "Interrupted" with Retry.
- Runs continue while another session is in view. The rail shows the state names of the table above.

### 5.6 Toolbox, artifacts, notebook

- **SciPy and matplotlib** (package S18): pinned wheels with hashes in `packaging/windows/stage_runtime.py`; `stage.rs` allowlists extended; Agg backend with `MPLCONFIGDIR` in scratch; lazy imports. Files written under `scratch/artifacts/` are harvested (16 files and 20 MiB per run, type by magic bytes, symlinks and junctions refused), stored as blobs, `solve.artifact` appended. A job memory cap is added in the same package.
- **Optional engines** (package S22, profile `solve/2`): **deferred as a whole.** Lean, its first content, is cut; Cadabra2 and FORM are deferred. The design kept for later: a manifest (id, version, URL, sha256, size, licence); download is a user action in Settings only, never a model tool; hash verified before unpacking; run inside the same sandbox; Cadabra2 and FORM only after a native Windows build is confirmed to run in the AppContainer.
- **Write-up** (shared package F15, overview 2.13; Solve adapter `export/src/writeup_solve.rs`): the steps the final answer rests on, in dependency order, as TeX or Markdown. Assumptions come first. Every formula is the check process's rendering of its handle (rule R5 holds in the file too). Cited steps become citations, with the user's own citation key when the paper is in their library. The file starts with the run's status line and ends with the table "How each statement was checked"; neither can be turned off. Zero model calls; "Polish the prose (model)" is optional and cannot touch formulas, citations or the table.
- **Notebook export** (`crates/export/src/notebook.rs`, nbformat 4.5 written as JSON from Rust): cell 1 the setup; cell 2 a prelude defining `expr()` and `save_expr()` with the handle table inlined, so it runs with plain SymPy outside the app; then per step of the final answer's closure a Markdown cell (id, badge, flags, summary, LaTeX) and one code cell per cited receipt with stored outputs; then one code cell per check with its recorded outcome; last, the done checklist and "rests on". Cell ids are the first 8 hex digits of the content hash. Option: include all runs.

---

## 6. Rules enforced in code and the tests that prove them

| # | Rule | Where | Test |
|---|---|---|---|
| R1 | An old session resumes with exactly its recorded tool definitions | Shared: packages F0 and F1 (`tool_profile.rs`; `resume.rs` builds expected definitions from the profile recorded on each `session.start` and `session.resume`) | In F0 and F1: one golden `(id, def_hash, order)` file per profile, `solve/1` among them; the recorded ledger fixtures of set R of the evaluation set (research, investigation before pointers, investigation current, hand-off child) resume with Solve code present |
| R2 | A profile changes only at a leg boundary where the host resumes from a resume brief; research to solve only after a fork plus `solve.started`. Solve-only for now (overview 2.2) | `resume.rs` (edited by the foundation lane), with the Solve part in package S12 | Replay resume with a different profile is Drift; a resume from a resume brief `solve/1` to `solve/2` passes; no registry tool can append `solve.started`, `solve.expr`, `solve.check`, `solve.ack` or `solve.done` (test enumerates emitted kinds per handler; the marker part of this test is shared with F5) |
| R3 | The model cannot write a check outcome, a tier, a handle, a rendered formula or a review's independence | only `solve_checks.rs`, `solve_exprs.rs`, `verifier_loop.rs` append these; none reads them from tool input | Property test: arbitrary JSON into every Solve tool never yields a passing `solve.check` unless the check-process stub returned passing evidence |
| R4 | A check counts only for the exact claim and setup mathematics | `solve_status.rs` | Re-saving a handle drops the tier; a check on claim 0 does not lift a two-claim step; a setup revision that changes an assumption makes checks stale; re-recording the same claim keeps them |
| R5 | Shown formula equals checked formula | `step_record` has no `latex` field; LaTeX comes from the check process | LaTeX on the card equals the check process's rendering of the stored `srepr`; a solver worker with a patched `latex` printer does not change it |
| R6 | Model code cannot alter a check | check process runs only `python/checks/`; `srepr_safe.py` | Hostile `srepr` strings (attribute access, `__import__`, unknown names, lambda) are rejected and marked non-portable; a solver worker that patches `sympy.simplify` does not change any check outcome |
| R7 | Tiers are monotone | `solve_status.rs` | Property tests on random graphs: effective <= own and <= every dependency; adding a fail or an edge never raises any tier |
| R8 | One weak check never gives Computed | rule 3 of 4.4 | A step with only `dimensions` and `symmetry` passing is Unchecked; an inequality with only `counterexample_search` is Unchecked |
| R9 | Dependencies cannot be dodged or manufactured | `derived_depends_on`; no "depends on everything" fallback in Solve | Using handle h owned by e-3 without declaring it still inherits e-3's tier; `final_answer` with empty `depends_on` is refused |
| R10 | No computation step without a real successful run in this session | `step_record` (reuses `tool_reference_matches`) | Fabricated reference, failed run, other session: refused |
| R11 | Gates bind every route and never widen permissions | `tools::Gate` in `Router::try_call` | Scripted in-core transport, MCP server call and delegated worker call each hit G1; a gate returning allow on a policy-denied call still ends denied |
| R12 | A gate cannot burn calls | G5 | A transport that ignores gate errors is paused after 3; the count of `model.request` events is bounded |
| R13 | Reviews are labelled truthfully and isolated | `verifier_loop.rs` | Same provider and model gives `same_model` and the lower tier; a variable set in the solver worker is undefined in the verifier worker; a verdict citing a non-existent check is rejected |
| R14 | (Cut with Lean. Number kept.) | - | - |
| R15 | "Done" is computed from the ledger | `solve_done.rs` | One fixture per criterion kind, met and unmet; unmet gives `finished_open_items`; the string "Done" without "with open items" is unreachable otherwise (view-model test) |
| R16 | Pause, not abort; rebuild is deterministic | `solve_loop.rs`, `resume_brief.rs` | The per-leg cap ends with `solve.paused` and no `session.abort`; `build_resume_brief` twice is byte-identical; continue after a ledger cut at a `tool.request` with no result gives a valid leg |
| R17 | Nothing is cited that was not read | Shared: the quote core F4 (`verify_quote`, `lines_sent`), called by `step_record` | In F4: a quote is accepted only if it verifies and its lines were sent to the caller that cites it. In S8: a `reference_lookup` without a matching `evidence.quote` record is refused; file hash mismatch is refused; a Cited step can be recorded in a session with no Investigation marker |
| R18 | Sandbox and approvals unchanged | Solve policy preset (`solve_policy()` with its own version string), held by profile `solve/1` so resume compares that version; default Deny | Every `solve/1` tool id has a rule; every ExecuteCode tool is Ask; `host:check` without a grant gets `not_run`, not a run |
| R19 | Re-run reproduces | `POST solve/recheck` | Same stored request gives the same `evidence_sha256`; a difference is surfaced as "result changed on re-run" |
| R20 | Files stay at or under 600 lines | Shared: the CI check of package F0, for all modes | fails on any new or touched file over the limit |
| R21 | Nothing arrives as checked through a hand-off | Shared: the receiving route of F7 | In F7: every `QuoteRef` of a packet is re-run through `verify_quote` in the new session; a step made from a packet is Given or Cited, never a higher tier; no badge or tier is read from a packet |
| R22 | Screen words stay inside their allowed places | view models, exports, write-up | Property test (shared, evaluation set 7.3): no Solve view, export or write-up shows "verified" at all, or "proved", "new" or "novel" outside the allowed places |
| R23 | A write-up says how each statement was checked | Shared: F15 (`writeup.rs`), Solve adapter | Evaluation set W: every formula equals the check process's rendering of its handle; the check table lists every step of the closure with its badge; the status line equals `solve.done.status`; a write-up is refused while the closure has a Failed or Stale step |
| R24 | A lesson never decides anything | Shared: F16; `solve_status.rs` reads no lesson | Evaluation set K: with and without lessons the same events give the same tiers; the resume brief built twice from the same events is byte-identical whatever the live lesson store holds |

What code cannot close, stated plainly: a handle can be checked correctly and still not mean what the prose
says, and a model can pick a weak check. The countermeasures are the formula rendered from the handle, rule
R8, a verifier on another model, and the user's eye. The UI must never say more than "Computed" for such a step.

---

## 7. What is reused, changed and retired

### 7.1 Reused unchanged

`crates/ledger` (ledger and blobs); `crates/tools` policy, approvals and registry; `crates/policy`;
`crates/worker` (AppContainer launcher); `python/symbolic_validation.py`, `numerical_validation.py`
(called from the new check modules); `crates/research-tools/src/proof_dag.rs` handlers for
`ledger_supersede` and `ledger_query`; the paper reader tools (`paper_passages.rs`);
`crates/agent/src/honesty.rs`; `crates/export` fork, diff, TeX, HTML;
`crates/agent/src/sidetalk.rs`, `narrator.rs`, `strategist.rs` (available on a pause, never forced);
`assets/math_katex.js`.

Not reused: `investigation_evidence.rs`. It needs an open Investigation request marker, so it cannot run in
a Solve session. Cited steps use the shared quote core (F4) instead.

Used from the shared foundation, not built here: tool profiles and resume (F1), the 600-line check and
golden tool tests (F0), `PaperRef` and `NodeRef` (F2), the network gate and paper fetch (F3), the quote core
(F4), mode detection, home screen and rail tags (F5), the graph view, run bar, badge and quote chip (F6),
the hand-off packet (F7), the evaluation harness (F8), the model-call counter (F9), the library (F10) with the user's own
paper sources (F13), PDF text with page links (F14), the write-up writers (F15), corrections and lessons (F16). The one
host function that computes a reviewer's `independence` is built with S14 (it was listed under F12).

### 7.2 Changed

| Path | Change |
|---|---|
| `crates/tools/src/router.rs` (491 lines) | call `Gate::before` and a post-call hook; new code in `crates/tools/src/gate.rs` |
| `crates/agent/src/resume.rs` (579) | expected definitions per recorded profile: done in F1, not here. Solve adds only `resume_mode: brief` (package S12), as an edit made by the foundation lane, which owns this file |
| `crates/agent/src/runner.rs` (626, already over) | `SessionOutcome::Paused`, mode-neutral; move `dispatch_tool_calls` to `dispatch.rs` first |
| `crates/agent/src/verifiers.rs` (878, already over) | split into `verifiers.rs`, `verifier_context.rs`, `verifier_parse.rs` before the verifier loop is added; behaviour unchanged |
| `crates/research-tools/src/proof_dag.rs` (549) | `ProofEntry` gains `#[serde(default)]` fields; `closure()` over declared plus derived edges moves to `proof_closure.rs` |
| `python/worker.py` (277) | protocol 2: `evaluate` accepts `exprs_in`; outbox for `save_expr`; `expr()` loader; result keys |
| `python/solver_calc.py` | `save_expr` writes to the outbox; the scratch bus file is no longer read by anything |
| `crates/worker/src/lib.rs` (336), `stage.rs` (475) | second and third process roles; `python/checks/` added to staged helpers |
| `crates/research-tools/src/policy_preset.rs` (77) | `solve_policy()`, referenced by profile `solve/1` |
| `crates/desktop-server/src/routes_session.rs` (616, over), `turn_runner.rs` (586) | mode by `session_mode`, mode to profile to router, and shrinking both files: done in F5 and F0, not here. Solve adds only the dispatch to the Solve loop (S12), as an edit made by the foundation lane. New Solve code goes in `solve.rs`, `solve_view.rs` |
| `crates/mcp-server/src/main.rs`, `crates/cli-route/src/core_tools.rs` | build the registry from the session's profile: done in F1. Gates come with the Router (S3); Solve parity on these routes is S15 |
| `crates/export/src/zip.rs`, `replay_checks.py`, `lib.rs` | new event kinds in the export; `ExportFormat::Notebook` |
| `assets/index.html` (768, over), `app.js` (932, over), `composer.js`, `workbench.js` | mount points only; any package touching an over-limit file moves code out first. The home screen, the New menu and the rail tags (`index.html`, `session_rail.js`) are edited in F5; `canvas_dag.js` (688, over) is slimmed under 600 lines in F6 when `graph_layout.js` is extracted |

### 7.3 New files

Rust: `crates/tools/src/gate.rs`; `crates/research-tools/src/{solve_model, solve_state,
solve_status, solve_setup, solve_exprs, solve_steps, solve_receipts, solve_checks, solve_done,
proof_closure}.rs`; `crates/agent/src/{solve_loop, solve_prompts, solve_gates, resume_brief,
verifier_loop, verifier_tools, verifier_context, verifier_parse, dispatch}.rs`;
`crates/desktop-server/src/{solve, solve_view, solve_migrate}.rs`; `crates/export/src/notebook.rs`.

Python: `python/checks/{__init__, srepr_safe, algebra, physics}.py` and their tests.

UI (`crates/desktop-server/assets/`): `solve_setup.js`, `solve_state.js`, `solve_steps.js`,
`solve_badges.js` (the tier-to-badge mapping, drawn with the shared `ui_badge.js`), `solve_why.js`,
`solve_exprs.js`, `solve.css`.

Not new here (shared files this mode uses): `tool_profile.rs`, `resume_profile.rs` (F1); `quote_core.rs`,
`lines_sent.rs` (F4); `session_mode.rs`, `new_menu.js` (F5); `graph_layout.js`, `graph_view.js`,
`ui_badge.js`, `run_bar.js`, `quote_chip.js` (F6); `export/src/handoff.rs`, `desktop-server/src/handoff.rs`
(F7); `run_budget.rs` (F9). The run bar is `run_bar.js`; there is no separate Solve run bar file.

### 7.4 Retired (for new sessions only)

- "New research": it moves under "More" and stays there until Solve can run a problem end to end (S13) and Explore can map an idea (its package X13); then it is hidden behind a setting (shared decision, overview section 8, decision 7; detail in overview 5.3).
- In Solve sessions: the Direct/Verified/Deep rigor chips, `verification_mode`, `calculation_mode`, the end-of-leg `check_computation_interpretation`, the "final answer depends on every entry" fallback, the tool-less verifier call, `session.abort` at the per-leg cap.
- `RESEARCH_SYSTEM_PROMPT` and the calculation and verification prompt sections are frozen and used only by `research/1` sessions. The Solve prompt is short (role, workflow, how handles work, what a gate error means); every "must" in it has a gate or a tool refusal behind it.

---

## 8. Migration of existing sessions and def_hash

### 8.1 Tool profiles

The profiles are the shared table of overview 2.2 (package F1), not a Solve enum: `research/1`,
`investigation/1`, `investigation/2`, `explore/1`, `solve/1`. Each profile is data:
`{name, defs, may_gain, earlier_defs, policy}`. `session.start` and `session.resume` record
`tool_profile`. A session without the field is `research/1`, or `investigation/1` if its latest mode marker
is `investigation.started`; today's 7 tolerated additions and 2 special cases move into that profile's data.
`research/1` returns exactly today's research tool list and is frozen by a golden test (F0) that fails on
any edit to an id, description, schema, effect or approval scope. The four new Investigate tools live in
`investigation/2` and are not added to the research list, so this freeze holds.

`solve/1` reuses several existing definitions byte for byte (`sympy_eval`, `ledger_supersede`,
`ledger_query`, `ledger_note`, the readers). Their `def_hash` is therefore shared with `research/1`, and the
golden tests of both profiles pin them. No existing definition is edited.

### 8.2 When a profile may change

Because a Solve leg starts from a resume brief and not from replayed provider responses, the tool list may
change between legs without breaking anything. This rule is Solve-only for now (overview 2.2). So:

- Within a profile: resume checks the recorded definitions against that profile (today's check, unchanged in strictness).
- `solve/1` to `solve/2`: allowed only on a host-written resume from a resume brief. Old legs are never replayed under new definitions.
- `research/1` to `solve/1`: only through "Open in Solve" (8.4).
- Anything else: Drift, fail-closed, as today.

### 8.3 Event compatibility

- `proof.entry` and `proof.verification` gain optional fields and keep schema version 1. Old readers read by key.
- New kinds (`solve.*`) fall into the catch-all arm of `read_proof_dag`. The ZIP export and its replay script need the new kinds added.
- `sympy_eval` results gain keys. `successful_sympy_result` and the honesty lint read only `value`, `error` and `validation`; this must be re-confirmed by a test in package S6.
- Worker protocol goes from 1 to 2. Core and worker ship in one installer and the staged runtime is fingerprinted, so a stale staged worker is re-staged on first run.

### 8.4 Today's Research sessions

- Not converted. They stay readable, exportable, forkable and resumable under `research/1` with their recorded system prompt. The rail tags them "Research (earlier)".
- **Open in Solve**: `export::fork_session` into a new session, then host-written `solve.started` and a `solve.setup` prefilled from the first prompt, the pinned conventions, existing `assumption` entries and ingested sources; `solve.setup.origin` is a `NodeRef` to the Research session. Old steps keep their ids and get tiers from the same function: an accepted old review counts as Model agreed (same model, unless the old event recorded a different verifier model); old SymPy validations are shown as receipts and do not give Computed, because they are not bound to a claim. The old ledger is untouched.
- Investigation hand-offs created before this change (`investigation.handoff`) stay readable and stay Research.
- **New hand-overs into Solve use the one shared packet (F7, overview section 4).** The new session's first events are, in order: copies of the needed `source.ingested` events, one `handoff.packet`, then `solve.started`. The setup prefill reads the packet; nothing Solve-specific is written by the sending mode:
  - `statement.text` fills the statement; `conventions` fills the conventions; `ask.kind` is the task kind (`calculate`, `prove`, `check_derivation`); `ask.fields` fill `target` and suggested done criteria (from Explore: `check.reproduces` becomes a suggested criterion and "Check first" is shown above the done checklist; `risks` and `stop_if` become notes the user sees); symbols fill the symbols table; `from` becomes `solve.setup.origin` and an origin chip back to the node.
  - Each `context` item (definition, assumption, equation used, known result) is listed as a step. It is Cited once its `QuoteRef` passes `verify_quote` against the new session's own receipt, and Given if the user keeps it as plain text. A quote that fails is dropped and counted on screen.
  - No badge or tier travels. `model_text` (gists, summaries) is shown with the suffix "(model)" and gives no tier.
  - The user sees and can edit the setup card before any model call. A hand-over costs zero model calls.
  - Nothing is written back into the origin session. The origin node shows a chip ("Opened in Solve: Done with open items") by reading this session's latest `solve.done`.
- Hand-overs out of Solve: "Attach a source" (5.1) and, later and outside the two-month plan, "Has this been computed before?" into Explore with the statement rendered from the step's handles.
- Rollback: everything new is behind the `solve.started` marker.

---

## 9. Work packages

Sizes: S up to 2 days, M up to 1 week, L 1 to 2 weeks (the overview's scale). Ids starting with F are the
shared foundation packages of overview 2.1; they are built once, mostly by the foundation lane, and are not
repeated here. **The first three to build in the Solve lane are the Solve problem set inside F8 (previously
S0), S2 and S3**; they do not depend on each other and can run in parallel, and S6 follows with no
dependency either. In the same weeks the foundation lane builds F0 and F1 (previously S1), which S7 waits
for. A usable Solve with computed badges and no model reviewer exists after S13.

Order in the shared two-month plan (overview section 6): week 1 F8 problem set and S2; week 2 S3 then S6;
week 3 S4; week 4 S7 and S5; week 5 S8; weeks 6 and 7 S9, with S13 starting against fixture views; week 8
S11. Then S12 and S13, S10, S14, S15, and the rest.

| Id | Title | What | Files | Size | Depends on |
|---|---|---|---|---|---|
| S0 | Evaluation set: moved to F8 | The harness, the manifest and sheet formats and the old-session ledgers are F8. Solve remainder, written by the Solve lane inside F8 in week 1: set Q of overview 7.2 (10 calculations with known closed forms including the example above, 3 derivations with one planted error each, 2 prose-heavy proofs), each with final answer, expected tier per step and the planted step; scripted-transport fixtures for later packages. A planted error must end Failed, never Computed. Solve also supplies the target cases of the hand-over chains (set C) | `crates/agent/tests/eval_support/`, `tests/fixtures/eval/`, sheets in `docs/design/eval-2026-11/` (all F8) | in F8 | none |
| S1 | Tool profiles: replaced by F1 | No Solve package. F1 builds the profile mechanism, the frozen `research/1` and the golden files. What stays in Solve: the tool list of `solve/1` (5.1, added to the profile data by S7 to S9, S11 and S16 as each tool lands) and the resume-brief part of rule R2 (S12) | none here | - | - |
| **S2** | **Solve model, state fold, badge function** | Types, incremental fold, own and effective tier, stale and fail rules, closure over declared plus derived edges; property tests R4, R7, R8. Pure code, no I/O | `crates/research-tools/src/solve_model.rs`, `solve_state.rs`, `solve_status.rs`, `proof_closure.rs`, `proof_dag.rs` | M | none |
| **S3** | **Router gate and post-call hook** | `tools::Gate`; refusal becomes a tool error plus `guardrail.blocked`; tests R11 (cannot widen) | `crates/tools/src/gate.rs`, `router.rs`, `router/tests.rs` | S | none |
| S4 | Session marker, setup endpoint, view | host-written `solve.started`; `POST solve/setup` (validate, write `solve.setup` and Given steps); deterministic prefill (kind guess, symbol scan, default checklist); `GET solve/view` in the shared `GraphView` shape plus the step list. Mode detection (`session_mode`), mode to profile to router and the edits to `routes_session.rs` are F5, not here | `crates/desktop-server/src/solve.rs`, `solve_view.rs`; `crates/research-tools/src/solve_setup.rs` | M | S2, F5 |
| S5 | UI: setup card, first screen | Setup form with all states, same-model note, approval lines (the shared approval card that lists each grant), Given steps, toolbox line. No model call in this package. The home screen card, the New menu and the rail tag are F5, not here | `assets/solve_setup.js`, `solve_state.js`, `solve.css` | M | S4, F5 |
| S6 | Worker protocol 2, check process, safe parser | `exprs_in`, outbox, `expr()`; second process role; `srepr_safe.py`; canonical `srepr` and LaTeX from the check process; tests R5, R6 | `python/worker.py`, `solver_calc.py`, `python/checks/__init__.py`, `srepr_safe.py`, tests; `crates/worker/src/lib.rs`, `stage.rs` | M | none |
| S7 | Expression store and `expr_show` | harvest, validate (100 handles and 2 MiB per run), blobs, `solve.expr`, restage after restart, previews in tool results | `crates/research-tools/src/solve_exprs.rs`; adds `expr_show` to the `solve/1` data in `tool_profile.rs` (F1's file) | M | F1, S2, S6 |
| S8 | `step_record` and receipts view | the tool with all refusals (R9, R10), `derived_depends_on`, hashes. The Cited path is not built here: `step_record` calls `verify_quote` and `lines_sent` of F4 and checks that `source_ref` names an `evidence.quote` event (Solve side of R17). Also here: the small host route behind "Attach a source" (5.1), which calls `verify_quote` and writes `evidence.quote`; it holds no quote logic of its own | `crates/research-tools/src/solve_steps.rs`, `solve_receipts.rs`; the route in `crates/desktop-server/src/solve.rs` | M | S7, F4 |
| S9 | Check registry (algebra), `check_run`, auto-checks | five algebra checks, outcome rules, `solve.check`, `host:check` auto-run with time caps, Solve policy preset; tests R3, R18, R19 | `python/checks/algebra.py`, tests; `crates/research-tools/src/solve_checks.rs`, `policy_preset.rs` | L | S3, S8 |
| S10 | Physics checks | dimensions, limit_case, symmetry, conservation, convergence, matrix_property | `python/checks/physics.py`, tests | M | S9 |
| S11 | Solve gates, `solve_finish`, done | G1, G4, G5, G6, G7; `solve.done`; tests R12, R15. G4 only compares against the limits: the model-call count comes from F9 | `crates/agent/src/solve_gates.rs`, `crates/research-tools/src/solve_done.rs` | M | S3, S9, F9 |
| S12 | Solve loop, prompt, pause, resume brief, continue | separate loop from `autonomous.rs`; short prompt per task kind; `SessionOutcome::Paused` made mode-neutral in the runner (Investigate switches to it afterwards); resume brief; checkpoint; `resume_mode: brief` and the resume-brief part of R2; interrupted detection; the run states of 5.5; tests R16 | `crates/agent/src/solve_loop.rs`, `solve_prompts.rs`, `resume_brief.rs`, `dispatch.rs`, `runner.rs`; `crates/desktop-server/src/turn_control.rs`; the edits to `resume.rs` and `turn_runner.rs` are made by the foundation lane | L | S11, F1 |
| S13 | UI: derivation, badges, Why popover | cards from handles, all states, popover with receipts and actions (including "Attach a source"; its library picker needs F10), Expressions tab. The run bar, the graph view and the quote chip are the shared parts of F6: this package supplies only the Solve adapter (tier to badge, grade and border). Slimming `canvas_dag.js` is F6, not here | `assets/solve_steps.js`, `solve_badges.js`, `solve_why.js`, `solve_exprs.js` | M | S5, S9, F6; S8, F3 and F10 for "Attach a source" |
| S14 | Verifier loop with tools | split `verifiers.rs`; loop of up to 12 model calls; verifier worker; `report_verdict` validated; independence by one host function, built in this package (overview 2.9); `review_steps`; batching; tests R13 | `crates/agent/src/verifiers.rs`, `verifier_context.rs`, `verifier_parse.rs`, `verifier_loop.rs`, `verifier_tools.rs`; `crates/desktop-server/src/agent_models.rs` | L | S9, S12 (the independence function is built here; F12 no longer holds it) |
| S15 | CLI route parity | `solve/1` through the MCP core server with gates and auto-checks; result sheet from the ledger; review through an API transport or a second CLI call when one is configured | `crates/mcp-server/src/main.rs`, `crates/cli-route/src/core_tools.rs`, `crates/desktop-server/src/cli_turn.rs` (628, move code out first); building the registry from the profile on these routes is already done by F1 | M | S11, F1 |
| S16 | Formal tier: **cut** (owner decision of 2026-11-25) | Not built. No `formal_check`, no Lean-checked tier. The row is kept so numbering stays stable | none | - | - |
| S17 | Check-a-derivation intake | TeX only at first; a derivation given as PDF waits for F14 and is not planned here. Deterministic split of the given TeX into Given steps (`investigation_outline_tex.rs`), transcription flag and Confirm, pairwise checks | `crates/research-tools/src/solve_derivation.rs`, `assets/solve_transcribe.js` | M | S9, S13 |
| S18 | SciPy, matplotlib, artifacts, memory cap | wheels, allowlists, Agg, harvest, `solve.artifact`, Artifacts tab, read-only copy of user data files, job memory cap, installer smoke test | `packaging/windows/stage_runtime.py`, `smoke-install.ps1`, `crates/worker/src/stage.rs`, `win.rs`, `crates/research-tools/src/solve_artifacts.rs`, `assets/solve_artifacts.js` | L | S7 |
| S19 | Notebook export | The machine-readable export; the readable TeX derivation is the write-up of F15, not this package. Writer, prelude, check cells, deterministic ids, new kinds in ZIP | `crates/export/src/notebook.rs`, `zip.rs`, `lib.rs`, `replay_checks.py` | M | S9 |
| S20 | Blind compute | `compute_blind`, worker sees inputs only, host comparison, G3, `independent_recompute` | `crates/agent/src/delegate_blind.rs`, `solve_gates.rs` | M | S9, S12 |
| S21 | Earlier Research sessions | Open in Solve for a Research session (fork plus `solve.started`, prefill, `origin` as a `NodeRef`), tiers for old steps (8.4). The hand-off half is F7: the packet, the receiving route and the prefill from a packet (8.4) are built there, on top of S4. The rail tag "Research (earlier)" and moving "New research" under "More" are F5 | `crates/desktop-server/src/solve_migrate.rs` | S | S4, S12, F5 |
| S22 | Engine manager, `engine_run`, profile `solve/2`: **deferred** (owner decision of 2026-11-25) | Not in the build order. Lean was its first engine and is cut; Cadabra2 and FORM are deferred. When it returns: manifest, hash-verified download as a user action, Settings page; `solve/2` is a new row of the F1 profile data | `crates/desktop-server/src/engines.rs`, `assets/toolbox.js`, `crates/research-tools/src/engine_run.rs` | L | F1 |

Shared packages added by the scope revision that have a Solve part (built in the shared lane, listed
here so the Solve lane knows what to supply):

| Package | Solve part | Needs from Solve |
|---|---|---|
| F15 Write-up | `export/src/writeup_solve.rs`: the closure of the final answer as TeX or Markdown, with status line and check table (5.6, R23). First write-up to be built | S9 (checks), S11 (`solve.done`) |
| F16 Corrections and lessons | F16a: conventions, symbol assumptions and dimensions the user set or corrected are offered as defaults in the next setup. F16b: after "Correct this step" the card offers "Remember this?"; kept notes are supplied at the start of a leg and recorded in `lessons.supplied` (R24) | S4 and S5 (setup), S13 ("Correct this step" on screen), S12 (resume brief) |
| F13 Paper sources, F14 PDF | "Cite a paper" picks from the user's own library; a PDF-only source gives page-linked quotes | S8 (the "Attach a source" route) |

---

## 10. Free core versus optional add-ons

| Free core (works with no paid service and offline apart from the user's own model) | Optional add-on |
|---|---|
| Setup, steps, handles, all computed checks, badges, gates, pause and continue | Hosted model access for users who have none |
| Verifier with tools on the user's own second model, or the same model with the weaker label | |
| SciPy, matplotlib, artifacts (in the installer, D7) | Prebuilt Cadabra2 or FORM bundles (deferred with S22) |
| Write-up of the derivation as TeX or Markdown (F15); Notebook, HTML, ZIP export | |
| Citing papers from the user's own library, PDF-only papers included (F13, F14) | |

Lean and Mathlib checking is not offered, in the core or as an add-on (cut on 2026-11-25).

Nothing in the free column calls a paid service. The number of model calls is shown live and capped by the budget.

---

## 11. What is borrowed from where

| Source | Licence | Taken as | What |
|---|---|---|---|
| Solver agent (`re-Solve`), `stepVerificationAgent.ts`, `fullVerificationAgent.ts` | AGPL-3.0 | idea, re-coded in Rust | verifiers as loops of up to 12 model calls with compute and ledger-read tools on a separately routed model |
| Solver agent, `mainSolverAgent.ts` | AGPL-3.0 | idea | interpretation gate on every tool call. Changed: 3 runs tolerated, failed runs exempt, bounce counter |
| Solver agent, `python/casBus.ts` (the fork owner's file) | AGPL-3.0 | idea; code may be ported | host harvests named exact expressions, name rule, 100-handle and 2 MiB caps, symbol assumptions. Changed: the ledger is the source of truth, handles are versioned and hashed, loading uses a restricted parser |
| Solver agent, `symbolicAgent.ts`, `systemPrompts.ts` | AGPL-3.0 | idea | a "success" with no zero-exit run is downgraded; the expected result is never given to a worker. Made structural here |
| Solver agent, Lean plugin (the fork owner's file) | AGPL-3.0 | not used (Lean cut) | - |
| Solver agent change log | - | lesson | no text-only adviser roles; no rule that lives only in a prompt |
| leanblueprint | Apache-2.0 | idea | status from a node and its ancestors: own versus effective tier |
| SymPy, mpmath | BSD-3-Clause | used in the worker | three-rung equality: simplify to zero, numeric at random points, series comparison |
| sympy-mcp | Apache-2.0 | idea | expression keys; assumptions attached when a symbol is created |
| Get Physics Done | licence unconfirmed | idea only, no code or text | a registry of named physics checks. Read its licence file before taking more |
| Lean 4 REPL | Apache-2.0 | not used (Lean cut) | - |
| Jupyter nbformat 4.5 | BSD-3-Clause (schema) | file format only | notebook written directly as JSON |
| Cadabra2, FORM | GPL-3.0 | deferred | optional engines, separate programs, only if native Windows builds run in the sandbox |
| Kandinsky (the owner's notes app) | AGPL-3.0 | approach and ported code (decided; overview 2.11, "Porting") | the user's own paper library as a source of cited papers, citation keys kept for the write-up, PDF text with pages (overview 2.11, 2.12) |
| re:Search Investigation mode | own code | pattern | session marker event, frozen definitions with golden tests, deterministic content before any model call |

---

## 12. Risks

1. **A checked handle that does not mean what the prose says.** Cannot be closed in code. See the end of section 6.
2. **SymPy's reach.** Tensor algebra, non-commutative operators at scale and most loop integrals are out of reach. Many steps will end as Model agreed. With Lean cut there is no tier above Computed, so every step that is not a relation between handles (a prose argument, an existence claim, most of a "prove" task) stays at Model agreed or Checked by you at best. If most of a run is amber the mode is worth less. Set Q of the evaluation set (F8) measures the share of steps Computed, Model agreed and Unchecked before any UI polish.
3. **Numeric identity checks.** Random points can miss a difference on a branch cut or outside the sampled domain, and can fail falsely across a cut. Receipts show domain, points and digits; "numeric only" is a visible sublabel; "Computed" is evidence, not proof, and the legend says so.
4. **Symbol assumptions.** In SymPy a symbol with different assumptions is a different object. A handle saved with a positive `a` and a claim built with a plain `a` gives a false mismatch. Setup assumptions must be applied wherever a symbol is created (`assumed_symbol`), and a mismatch must be reported as "inconclusive: symbol assumptions differ", not as a fail.
5. **The restricted parser's allowlist.** Too narrow and useful expressions become non-portable (no badge possible); too wide and it is a hole. Start narrow, log every rejection by constructor name, widen from data.
6. **Resume.** F1 touches the most fragile path, and all three modes now depend on it. The golden test for `research/1` (F0) lands before anything else. A session started after F1 records a profile and will not resume in an older build. Legs started from a resume brief lose provider-side state (thinking blocks, prompt cache): one cache miss per checkpoint.
7. **Approvals.** A grant is per caller, tool and worker epoch. A run needs several grants, and a worker restart voids them and pauses the run. This is correct and it is also the thing most likely to annoy on long unattended runs. Needs sensible timeouts and a visible pause in the rail.
8. **Cost of reviews.** A 40-step prose proof at 5 steps per loop is 8 loops of up to 12 model calls. Default is prose-only, capped by `max_review_loops`; the budget pauses instead of overspending. Defaults must be set from the set Q sheets (F8).
9. **Check-a-derivation.** Turning the user's TeX into handles is a model step and can be wrong. The two-column card and the "transcription unconfirmed" flag are the only protection. No prior art exists.
10. **Memory on the reference machine.** Three Python processes and the WebView. Not measured. Verifier worker stopped when idle; memory cap in S18.
11. **SciPy and matplotlib in the AppContainer.** DLL loading, font-cache writes and wheel availability for CPython 3.14 on win_amd64 were not verified. Installer growth is an estimate (tens of MB).
12. **Cadabra2 and FORM** are deferred (S22). They may have no native Windows build; the design promises neither. Until they return, tensor algebra and large non-commutative calculations stay outside what code can check (risk 2).
13. **CLI routes.** The host cannot stop a CLI from printing an unchecked final text; only the result sheet is under host control.
14. **Over-limit files.** runner.rs, routes_session.rs, cli_turn.rs, index.html, app.js, canvas_dag.js are already over 600 lines; each package that touches one must move code out first, which makes nominally small packages larger.
15. **Not verified here:** that host-handled tool specs are outside the drift check (taken from one proposal); the exact list of tool ids registered for research sessions; the default `sympy_eval` timeout.

---

## 13. Decisions for the owner

| # | Question | Recommended default |
|---|---|---|
| D1 | Top badge wording on a normal run | "Computed"; it is also the top tier now. "Proved" never on a computed step. "Verified" is shown nowhere (decided, overview 8.1). |
| D2 | Verifier is the same model as the solver | Run and label "Model agreed (same model)". It does not satisfy the default done checklist unless "accept same-model review" is ticked at setup. Suggest a second model once. |
| D3 | Which steps the verifier reviews by default | Steps with no computed claim, batched 5 per loop, plus whatever the done checklist still requires at finish. |
| D4 | Default done checklist for a calculation | Final result Computed, dimensions check passed, everything it rests on at least Model agreed, no unacknowledged model assumptions. "Every step Computed" is one tick. |
| D5 | Approvals at setup | The setup card lists every grant the run needs (solver runs code; host runs checks; verifier runs code and checks), one line each. One "Allow listed" click records each grant separately through the existing approver. No new approval scope; a worker restart still asks again. The card is the one approval card component all modes use (overview 3.1, row 13); whether the approver can prompt outside a running model turn is settled in F5. |
| D6 | Unmet criteria at the end | One automatic repair round inside the remaining budget, then stop as "Done with open items". No silent extra spending. |
| D7 | SciPy and matplotlib | In the installer, so the free core works offline. |
| D8 | Default budgets | 60 model calls, 30 minutes, 48 model calls per leg, 6 review loops; pause at every limit. Revisit after the first set Q sheets (F8). |
| D9 | "New research" once Solve and Explore exist | Shared decision: overview section 8, decision 7. |
| D10 | Assumptions the model adds mid-run | Continue, mark "Assumed by model", count as Unchecked for everything resting on it, list at the top of the result sheet for one-click acknowledgement. |
| D11 | A step you checked by hand | Allow "Checked by you", ranked with Model agreed, recorded with your acknowledgement and the step's hash. |
| D12 | A formula stated without a source | Allowed only as an assumption (Assumed by model), never as Cited. |
| D13 | Check-a-derivation: confirm transcriptions | Not required per step. Show "Computed (transcription unconfirmed)"; the done checklist stays open until "Confirm all" after a side-by-side look. |
| D14 | (No longer asked: Lean is cut.) | - |
| D15 | Changing the setup after the run starts | Allowed as a new revision. Checks that depended on changed assumptions or conventions become Stale; "Re-run all checks" is free. |
| D16 | Solve for subscription-CLI users | Package S15, right after the core loop. Do not hold the first release for it, but do not ship a release that hides Solve from them without saying why on the setup card. |
| D17 | First optional engine | Decided: none for now. Lean is cut; Cadabra2 and FORM are deferred with S22. |
| D18 | Notebook export default | Only what the final answer rests on; "include all runs" as an option. |
| D19 | Old Research sessions opened in Solve | Earlier reviews count as Model agreed (same model unless a different one was recorded). Earlier SymPy validations are shown as receipts and do not give Computed until re-recorded with handles. An earlier `lean_check` result is shown as a receipt ("Lean check recorded in the earlier session") and gives no tier, since Solve has no Lean tier. |
| D21 | A "prove" task without Lean | Keep the task kind; the setup card says "a proof in prose is reviewed by a model; only calculations are computed"; the default done checklist asks for a review by a different model (overview 8.3, question 9). |
| D22 | The write-up without its check table | Not offered (overview 8.3, question 10). |
| D20 | Start at once or wait for plan approval | Start at once. "Ask before running" is an opt-in switch. |
