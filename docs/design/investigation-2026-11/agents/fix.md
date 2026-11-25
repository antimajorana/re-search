# Brief: fix (orchestrator)

You are the orchestrator of the fix round. You plan, judge and check; you hand each fix to a
**Gemini 4 Neon (High) subagent** and review what it returns. If you cannot start subagents, say so
in your report and do the fixes yourself.

```bash
git clone https://github.com/antimajorana/reSearch.git ~/re-search && cd ~/re-search
git switch -c gemini/work/fixed origin/gemini/work/merged
git fetch origin 'refs/heads/gemini/work/*:refs/remotes/origin/gemini/work/*'
```

Read `README.md` of this folder, `RULES.md` (all of it applies; install any package or tool a test
needs), the three designs and `reports/merge.md`.

## Work, in this order

**1. The known Windows failure (start now).** GitHub "Workbench checks" run 45606098476 on
`gemini/work/merged` passed every step except the browser tests, where 7 of 8 passed and
`investigation` failed:

```
crates/desktop-server/tests/investigation_browser.py:365, check_question_first_investigation
expect(page.locator("#settings-dialog")).to_be_visible()  ->  hidden (14 retries, 5 s)
```

The sidebar at that moment showed "New research" and "New investigation". Find out whether the
app or the test is wrong (the test was written by brief `ux-ui`; the settings dialog and its opener
may have been changed by another brief or by the merge). Fix the cause. Install Playwright and
Chromium (`pip install --user playwright && python3 -m playwright install --with-deps chromium`)
and try to run `crates/desktop-server/tests/workbench_browser.py` on Linux the way
`.github/workflows/workbench-checks.yml` does; if it cannot run on Linux, say exactly why.

**2. The audit findings (as they arrive).** Four auditors are pushing
`gemini/work/audit-honesty`, `audit-flow`, `audit-ui`, `audit-consistency`. Each holds
`reports/audit-<lens>.md` and proof tests named `audit_<lens>_*`. After step 1, fetch; for every
audit branch that exists and you have not handled: merge it (`git merge --no-ff`), read the report,
and judge each finding yourself:

- **confirmed**: its proof test fails for the reason given, or you reproduce it. Fix it.
- **rejected**: the test is wrong, the behaviour is what the designs or `README.md` decisions ask
  for, or it cannot be reproduced. Do not change production code; if its proof test asserts
  something the designs do not require, delete that audit test and record why.
- `nit` findings: fix only when trivial and safe.

Then `sleep 600` and fetch again, until all four are handled or 2 hours have passed since you
started. Say in your report which audit branches never arrived.

## Rules for fixes

- One finding (or one tight group in the same function) per subagent and per commit. Never run two
  subagents on the same file at once. Commit message names the finding (`audit-flow #3: ...`).
- A fix makes the audit's proof test pass **without editing that test**, except where the test
  itself is wrong (record it). Existing tests are never weakened (`RULES.md`).
- No fix may weaken the honesty guarantees of design 01 section 2.7, the policy engine, approvals,
  sandbox or ledger. A finding whose only fix would do that is reported, not fixed.
- Review each subagent's diff before committing it: smallest change that fixes the cause, escaping
  kept, 600-line limit kept.
- After each batch and at the end run the full checks from `RULES.md` (skip only
  `cli_research_workspace_read_fork_diff_export_and_verify`, which hangs on Linux).

## Deliver

Push after step 1 and after each audit branch: `git push origin HEAD:refs/heads/gemini/work/fixed`
(only that branch, no force). Write `docs/design/investigation-2026-11/reports/fix.md`: whether
subagents worked and on which model; the Windows failure's cause and fix; a table of every finding
(lens, number, severity, confirmed/rejected, reason, commit); final test counts; what is left
failing; what only Windows can check. End with the PUSHED block for `gemini/work/fixed`.
