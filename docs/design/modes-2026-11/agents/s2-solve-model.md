# Brief s2-solve-model: Solve model, state fold and badge function (package S2 of Solve)

Design: `solve.md`, the row S2 in the package table and every section it draws on: the data model
(steps, handles, checks, tiers; own tier and effective tier), the stale and fail rules, the closure
over declared and derived edges, and the rules table with R4, R7 and R8 and the tests it names.
Read `solve.md` from the top through the rules table before you write code. `00-overview.md`
sections 2.3 (ids), 2.10 (the evidence scale), 3.2 rows 19 and 20 (the words "check" and
"verified") and 3.3 (run states; the run-state enum itself is delivered by another brief as
`run_budget.rs`, so define only what S2 needs and list it under "Merge notes").

## Deliver

Pure code, no I/O, no ledger writes, no tool: `crates/research-tools/src/solve_model.rs`,
`solve_state.rs`, `solve_status.rs`, `proof_closure.rs`, with tests.

1. The types of the Solve data model with serde, field names as the design gives them
   (`start_line`, `end_line` in any source reference; ids `e-N`, handles `name@version`).
2. The incremental fold from events to state: the same result whether events are folded one at a
   time or all at once, and from any prefix onwards (test this directly).
3. Own tier and effective tier, the stale and fail rules, and the closure over declared plus
   derived edges, as one badge function from state to what is shown. A step is never shown above
   the weakest thing it relies on. Scope revision of 2026-11-25: Lean is cut, so there is **no
   Lean-checked tier** and no formal-check record; the tier order ends at Computed (`solve.md`
   4.4). The word "verified" must not appear in any on-screen word: keep every on-screen word in
   one table of constants, with a test that none contains "verified". If the tier or the word
   was already written, remove it and say so under "Merge notes".
4. Property tests R4, R7 and R8 as the rules table defines them, written with a small
   deterministic generator (no new dependency), plus the named unit tests of that table that
   belong to S2.

## About `proof_dag.rs`

The design lists `proof_dag.rs` among the files of S2. That file also defines the tools
`ledger_append`, `ledger_supersede` and `ledger_query`, whose definitions are pinned by the golden
test. Do not change those definitions or their behaviour, and do not change what `proof.entry`
events mean for existing sessions. If S2 needs something from `proof_dag.rs`, read it from the new
modules; make an edit there only if it is behaviour-preserving, in its own commit, and listed under
"Merge notes". If the design cannot be met without changing a tool definition, stop that part and
explain it in the report.

## Not in this brief

The check process and SymPy (S3, S6), recording tools (S7), the setup card (S4, S5), gates and
finish (S11), pausing in the runner (S12), any UI.
