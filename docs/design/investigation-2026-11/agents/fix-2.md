# Brief: fix, second pass

Your first pass is on `gemini/work/fixed` at `90f166d`. It accepted every honesty and flow finding.
An independent review (read-only Claude reviewers who judged each finding against the designs and
read your first commits) disagrees on a number of them. This pass applies that review. Use
subagents as before, one file per subagent at a time.

```bash
cd ~/re-search && git fetch origin && git switch gemini/work/fixed && git status
git show origin/gemini/runner:docs/design/investigation-2026-11/reports/claude-review.md > /tmp/claude-review.md
```

Read `/tmp/claude-review.md` completely before changing anything. Where it and an audit report
disagree, the review wins; where you believe the review is wrong, do not silently ignore it: keep
your code, and say why in `reports/fix.md` with the decisive code quoted.

## Work

1. **The section "Review of the first honesty fix commits".** Apply its seven numbered items exactly
   (F12 revert, F2 revert, F3 adjust, F10 revert that part and fix `openCite` in JS instead, F8
   revert, F5 adjust, F1 adjust). F12 matters most: the anchor schema must be byte-for-byte what it
   was on `origin/gemini/work/merged`, so that `def_hash` of the four `record_paper_*` definitions is
   unchanged (prove it: compare the `anchor_schema()` JSON, or the definition hashes, between
   `origin/gemini/work/merged` and your branch, and put the result in the report).
2. **Every finding the review judged `by_design` or `not_a_bug`** (honesty F2, F12, F13 is "keep",
   F16, F17, F8, F9 is "keep as hardening"; consistency C-07, C-12, C-04 item 2; flow F2's
   `ledger_append` assertion; C-08 and anything marked `unclear`): read the review's reason. Undo
   production changes made only to satisfy such a finding, unless the review says keep. For a proof
   test that asserts something the designs do not require, delete that test and record why. Do not
   delete or reword rule 4 or the rule-3 honesty sentence of `INVESTIGATION_SYSTEM_PROMPT`, and keep
   the header line `Orchestrator -> Worker Delegation:` exactly as it is.
3. **Fix-risk notes on findings you already fixed.** For each `real` finding, compare your fix with
   the review's "Fix risk". In particular:
   - UI F3: definitions must not be replayed through KaTeX as a whole preamble (vendored KaTeX
     rejects `\newcommand{..}[2][2]{..}`); each definition handled on its own, optional-argument
     defaults supported or skipped without losing the other macros; `\providecommand` must not
     overwrite an existing macro.
   - UI F4: text after a mid-line `$$` closer must not be parsed as a list, quote, heading or table,
     and `$$$$` must still close one display and open the next (case 18 of `math_parity.test.mjs`).
   - UI F1: unclosed `$$` stays text; a half-arrived display during streaming still shows as source.
   - UI F8: no `resetMathMacros()` inside `renderSessionStream`.
   - flow F3 item 1: coordinator decision at the top of the review's second batch (README UX
     decision 1 wins: no conventions on a first-pass map turn, also for section-scoped maps).
   - flow F4: clamp the scope, never loosen `note_evidence`'s page gate.
4. **W1** (section "Pre-existing tests weakened"): restore a node-level `.question-match` assertion
   in `crates/desktop-server/tests/investigation_browser.py` for the new flow.
5. **Every "Missed by the auditor" item** in every section of the review: treat as a finding. Confirm
   it in the code first; fix it with a new test named `review_<area>_*`; if it is not real, say why.
6. No tool schema may change relative to `origin/gemini/work/merged` except where design 01 asks for
   it. No pre-existing test may be weakened. `RULES.md` applies; install any package a test needs.

## Checks and delivery

Run everything in `RULES.md` plus `crates/desktop-server/tests/workbench_browser.py` as you did in
the first pass. Commit per finding or tight group (`review: honesty F12 revert ...`). Push
`git push origin HEAD:refs/heads/gemini/work/fixed` (no force). Update
`docs/design/investigation-2026-11/reports/fix.md`: add a section "Second pass" with a table of every
item above (item, what you did, commit, or why not), the `def_hash` proof, final test counts, and what
is left. End with the PUSHED block for `gemini/work/fixed`.
