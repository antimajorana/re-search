# Brief f12-coverage — coverage gate and evidence wording (F12)

Start from `gemini/work/fixed` after F11 merges. Read `00-overview.md` 2.9–2.10,
and Explore sections 4.1 and 6, especially rule V3 and the preset limits.
Investigate's prior-work checks are deferred. Build the shared core for Explore.

- Own `research-tools/src/search_coverage.rs`, `prior_work.rs`,
  `prior_work_wording.rs`, their tests and module lines. Reuse F11's provider
  IDs/statuses and the paper and evidence keys from F2 and F4.
- The coverage gate names every unsatisfied condition. Require two answered
  external services, no attempted service failed or rate-limited, answered
  exact and alternative-name perspectives, enough screened candidates, full
  text or explicit reader dismissal for each triage-core paper, and every
  supplied seed found by search. The private library is listed for the reader
  but never counts toward the service minimum. Quick mode never qualifies
  for `none_in_scope`. Distinguish zero results from no answer.
- Use the shared prior-work statuses in overview 2.9. Wording must disclose
  search scope and failing conditions. Abstract evidence is limited to the
  recorded verbatim abstract and possible relevance. Model judgments stay
  attributed, and the other-name status needs full-text evidence of the
  name. Test that generated wording never calls a claim new or novel.
- Keep it pure and test each gate. No network/model calls, routes, new tools,
  or changes to current profiles. Leave the Explore state machine to X7–X11
  and second-model independence to S14. Document the downstream interfaces.

Write `reports/f12-coverage.md` and push to `gemini/modes/f12-coverage`
as directed in `RULES.md`.
