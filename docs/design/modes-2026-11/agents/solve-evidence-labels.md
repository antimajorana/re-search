# Brief solve-evidence-labels — display the accepted evidence names in Solve

Read `00-overview.md` 2.10, 3.4 and the owner clarification dated 2026-11-26,
`solve.md` 4.4, and the merged S2 report. Start from `gemini/work/fixed`.
Limit this independent change to the wording and its checked mapping in S2.

- Own the badge wording and its focused tests in `solve_badge.rs` and the
  existing Solve badge tests. Keep the serialized tiers and existing event
  format unchanged. Render a source-backed `cited` step as **Found** with its
  existing source and OCR flags. Render the numeric tier as **Numerically
  checked**. **Verified** remains restricted to exact symbolic checks.
- Test that a matched source, an exact symbolic receipt and a numeric check
  each get the right distinct label. A citation, model opinion, tool success
  without a claim check, stale check, or failed check must not be styled as a
  machine-confirmed claim. Existing inheritance and failure checks stay strong.
- Inspect the path that feeds `solve.check` into the badge fold. If the work
  required to enforce inspectable receipts lives in S9 or S13, report the exact
  integration point and the current trust assumption. Leave those packages'
  implementation to their own lanes. Do not change F8 fixtures or metrics.
- Run the Solve badge unit and integration tests plus the frozen tool goldens.
  No tool profile, resume, sandbox or dependency changes.

Write `reports/solve-evidence-labels.md` and push to
`gemini/modes/solve-evidence-labels` as directed in `RULES.md`.
