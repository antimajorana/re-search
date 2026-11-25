# Brief: audit (one lens per agent)

You are one of four auditors of the merged result. Your prompt names your lens. You **find and
prove** bugs and inconsistencies; you do not fix production code.

```bash
git clone https://github.com/antimajorana/reSearch.git ~/re-search && cd ~/re-search
git switch -c work/audit-<lens> origin/gemini/work/merged
git diff --stat origin/main...HEAD        # what changed (about 92 files)
```

Read `README.md` of this folder, `RULES.md` (all of it applies; install any package a test needs),
the three designs, and every file in `reports/` (each lists its own decisions, deviations and
suspected bugs: check those claims, do not trust them). `main` is the behaviour before this work.

## Lenses

- **honesty**: the guarantees of design 01 section 2.7. Can any path save a citation, quote,
  evidence note or "lines corrected" mark that the paper text does not support? Relocation (±12
  lines), narrowing, partial acceptance, the legacy `excerpt` form, reader evidence
  (`note_evidence`, `ev` anchors), `read_paper_lines` ranges, checked line citations on the answer
  card. Also: policy engine, approval flow, sandbox, ledger hash chain and fail-closed behaviour
  untouched or weakened? Any test or assertion weakened, deleted or loosened versus `main`
  (`git diff origin/main...HEAD -- '*test*' 'crates/*/tests'`)?
- **flow**: the Rust turn flow in `crates/agent`, `crates/research-tools`, `crates/desktop-server/src`.
  Question-first turns, scope resolution, outline, conventions gating, reader fan-out
  (parallelism 2/4, effort, model fallback, cancellation, a reader that fails or returns nothing),
  races between a running turn and session reads, error paths, turn limits, Research mode
  unchanged. Check each decision in `README.md` against the code.
- **ui**: `crates/desktop-server/assets`. Everything from a model, paper or provider escaped before
  `innerHTML`; LaTeX and Markdown behaviour against every case in design 03 (write extra Node tests
  for cases the suite misses); macros reset on session switch; equation numbering; answer card,
  scope chip, tabs, Esc, status; state left over when switching session or mode; event listeners
  added twice; dead or duplicated code from the merge.
- **consistency**: the app as a whole. Places where two briefs disagree (names, statuses, event
  kinds, JSON field names between Rust and JS, settings keys, defaults), strings shown to the user
  that contradict behaviour, Investigation versus Research mode differences that are not in the
  designs, settings that exist in UI but not in Rust or the reverse, docs (`README.md`,
  `THREAT-MODEL.md`) now out of date, unused code, the cross-brief wiring listed in
  `reports/merge.md`.

## What to deliver

For each finding: severity (`critical` wrong or unsafe result / `major` broken feature /
`minor` / `nit`), `file:line`, what is wrong, a concrete input or sequence that shows it, and the
fix you propose. **Prove it**: where possible add a failing test (new test files only, named
`audit_<lens>_*.rs` or `audit_<lens>_*.test.mjs`; never edit existing tests or production code) and
quote its output. Label anything you could not prove as "unproven". Say what you checked and found
correct, and what you did not get to. Do not pad: ten proven findings beat fifty guesses.

Write `docs/design/investigation-2026-11/reports/audit-<lens>.md`, findings most severe first,
commit, and push `git push origin HEAD:refs/heads/gemini/work/audit-<lens>`. End with the PUSHED
block (brief: `audit-<lens>`).
