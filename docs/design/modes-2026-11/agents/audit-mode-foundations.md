# Brief audit-mode-foundations — review F1, F4 and F14a ahead of V6 and V17

Use Carbon for the review. Start from `gemini/work/fixed` and record the exact
commit you audit. Read `RULES.md`, the overview 2.2, 2.5 and 2.12, the relevant
sections of `investigate.md`, and the existing reports for F1, F4 and F14a.
Independently verify the published implementation as preparation for the next
packages. This is a review of merged code while other agents build V2–V5 and
F5/F7. Scope the report to issues with a concrete effect on the plan.

- Parallel review A covers tool profiles and resume, task-specific proof of
  which lines were sent to the citing caller, and how V6 can extend those
  mechanisms without changing the frozen research definitions. Trace a new
  Investigate session and an existing Investigation ledger through the actual
  entry points. Verify the current fixture and golden checks independently.
- Parallel review B covers the PDF receipt, page breaks and 1-based line
  lookups, scan provenance, and the original-PDF route. Trace integration
  from upload through a sampled physical page link, including a blank page
  between two pages with text and the legacy receipt fallback. Verify the
  finding in the F14a report about the browser test, and look for any separate
  production defect. Use the self-authored repository fixtures. Leave user
  libraries and external papers out of the cloud review.
- For each confirmed finding, report the exact file and line, user-visible
  consequence, existing check or minimal synthetic example, and the smallest
  package that should own the fix. Call out what Linux can and cannot confirm.
  Follow up apparent findings by checking the caller, test and receipt format.

Run existing targeted tests as appropriate. Change only your report at
`reports/audit-mode-foundations.md`, then push it to
`gemini/modes/audit-mode-foundations` as directed in `RULES.md`. Keep credentials,
paper text and local paths out of the report.
