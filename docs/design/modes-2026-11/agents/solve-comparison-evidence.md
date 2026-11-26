# Brief solve-comparison-evidence — require a recorded comparison before a badge

Start from `gemini/work/fixed`. S2 is merged. Read `solve.md` 4.4 and its new
"Inspecting a check" contract, plus the related `SolveCheck` and `CheckOutcome`
types. Review `evaluate_claims_computed_tier` in `solve_own_tier.rs`.

- Fix the two paths where `independent_recompute` or a series check can earn the
  numeric tier just because exact comparison was not recorded. Require positive
  evidence for an exact or numeric comparison. A missing, unrecognized or
  contradictory kind stays unchecked. Checkable explicit numeric coefficients
  can earn the numeric tier for a valid series claim.
- Use typed parsing or a focused helper as needed. Keep the existing ledger
  format readable and keep all dependency, stale, provenance and failure rules.
  Check outcomes `Inconclusive` and `NotRun` do not earn a computation badge.
  Do not infer a symbolic result from a model review or a solver tool's exit.
- Add a focused regression test for missing and conflicting evidence, then
  assert that an explicitly numeric comparison has the expected tier. Keep
  current test assertions. Where a fixture previously relied on implicit
  numeric classification, make the fixture explicit and keep the same asserted
  tier. Coordinate test file ownership with `solve-evidence-labels` in the
  report. That lane owns `solve_badge.rs` and badge wording.
- Report the exact input contract for S9 and S20. S13 owns the receipt UI,
  rerun action and distinct user-visible reasons. No new tools, wire schemas,
  dependencies, or model and network calls.

Run the exact Solve badge tests and research-tools Clippy. Write
`reports/solve-comparison-evidence.md` and push only to
`gemini/modes/solve-comparison-evidence` as directed in `RULES.md`.
