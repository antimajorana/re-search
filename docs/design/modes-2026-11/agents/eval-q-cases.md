# Brief eval-q-cases — Solve fixed evaluation cases (F8, former S0)

The F8 harness already landed. Fill its open fixed-tier Q slot. Read
`00-overview.md` 0.1 and 7.1–7.3, `solve.md` 2 and 4.4,
`docs/design/eval-2026-11/README.md`, and `crates/agent/tests/eval_harness.rs`.
S2 is merged. S3 and S6 are in another lane. This work needs only F8 and the
S2 badge vocabulary, and can proceed independently of the Solve runtime.

- Own the new fixed Q cases in `tests/fixtures/eval/manifest-q.json` and any
  new associated fixtures under `tests/fixtures/eval/`. Keep both harness
  self-test cases and their existing scripted transport fixture intact.
- Add ten original calculations with known closed forms, including the design's
  effective-potential problem, three derivations with exactly one planted
  error each, and two prose-heavy proofs. Give explicit domains and assumptions
  where they affect the answer. Use stable case and step IDs, final answers,
  expected per-step badges, and a planted-step ID for the three failures.
- Follow the accepted badge names in S2. An exact symbolic check may earn
  `Verified`, numerical agreement earns `Checked numerically`, and a planted
  bad step must end `Failed`. Applying a proof argument in prose does not by
  itself earn a computational badge. Report the difference between an expected
  badge in an answer key and a measured badge in a run.
- Use separate Neon subagents for authoring disjoint groups and for independent
  mathematical review of the keys. Have the reviewer solve or check the inputs
  from their stated assumptions before comparing against the authored keys.
  Record reviews and any corrected key in the report.
- Use the existing scripted transport format where it can represent a case
  without inventing unmerged tool definitions. Document the exact cases that
  still need S9/S11 integration before scripted end-to-end evaluation. Run
  `cargo test --locked -p agent --test eval_harness` and verify that the new
  manifest cases are loaded. Add a focused assertion only if current tests do
  not cover validation of the new cases.

No production changes, real models, external paper text, golden regeneration,
or claimed live scores. Report under `reports/eval-q-cases.md` and push to
`gemini/modes/eval-q-cases` according to `RULES.md`.
