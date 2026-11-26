# Brief eval-p-cases — Investigate fixed evaluation cases (F8)

Fill the open fixed-tier P slot using the merged F8 harness. Read the overview
2.3, 6 and 7.1–7.3, Investigate 4.1–4.2 and 6.1–6.2, and
`docs/design/eval-2026-11/README.md`. The V2–V5 implementation has a separate
owner. These cases supply independent expected results from the source text.

- Keep the two existing harness self-tests. Own the new cases in
  `tests/fixtures/eval/manifest-p.json` and new `tests/fixtures/eval/p/eval-*`
  fixtures. Use only original TeX written for this evaluation. Include physics
  notes without theorem environments, a multi-file document with custom
  theorem macros, and a long generated document. Cover position-sensitive
  cross-references, ambiguous printed numbers, and an inner equation used in
  a proof. Add a citation with a locator if it fits the fixed schema.
- Record the expected section tree, theorem-like statements and a total of
  30 sampled uses in the manifest. Use stable fixture labels or coordinates
  that a future V2 runner can resolve. Edges point from the prerequisite to
  the dependent. Record whether each connection is a statement use, proof
  use or a heuristic mention. Keep the expected result independent of the
  current scanner output. Cite fixture file and line in the report for the
  harder cases.
- Use a second Neon reviewer to read the raw fixtures and check the keys
  independently. Record disagreements and their resolution. Keep version
  tracking, author novelty labels and the PDF case set out of this brief.
- Run `cargo test --locked -p agent --test eval_harness` against these fixtures.
  If the harness already validates the new cases, leave its code alone. Note
  any milestone checks that still need V2 or V3 before they can be run.

No production changes, private library access, third-party paper text, network
or model calls in tests, or live claims. Write `reports/eval-p-cases.md` and push
to `gemini/modes/eval-p-cases` as directed in `RULES.md`.
