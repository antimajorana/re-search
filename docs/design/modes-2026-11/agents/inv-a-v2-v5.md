# Brief inv-a-v2-v5: Investigate lane A, packages V2, V3, V4, V5 in order

The owner's first priority. You build the deterministic part of Investigate from the skeleton to
the screen: a TeX paper opens and shows its structure with zero model calls (milestone M1 and
most of M2 in `00-overview.md` section 6).

Design: `investigate.md` in full (sections 3, 4, 5.2, 6, 7, 8 and the rows V2 to V5 of section
9), `00-overview.md` 2.3, 2.4, 2.6, 2.10, 3.2, 5 and 7.2 set P. Package V1 (skeleton core for
TeX) is in your checkout under `crates/research-tools/src/skeleton/`; read its code and its
report `reports/v1-skeleton.md` first, including the places where it says the scanner fails.

## Order and hand-back

Do the packages **in order V2, V3, V4, V5**. After each package: run the checks of RULES.md,
commit, and **push at once** to `gemini/modes/inv-a-v2-v5` (a normal push of new commits; the
branch grows package by package), so the integrator can review V2 while you build V3. Then
`git fetch origin` and `git merge origin/gemini/work/fixed` before starting the next package, to
pick up what other agents delivered meanwhile (the shared UI parts of F6, tool profiles F1, the
library F10, the PDF core F14a). Resolve conflicts in favour of keeping both sides' behaviour;
if you cannot, stop that package and report.

Within a package, use subagents on separate files where the work divides (for V2: edges,
numbering, tags, stats and bibliography are five files).

## The packages

Each row of section 9 of `investigate.md` is binding for content and files. In addition:

- **V2.** Tags use `paper_key::node_tag` exactly as it is (the format is permanent: do not
  change it, and do not hash anything else). Define the kind classes in one table with a golden
  test, because they enter the hash. Version tracking is deferred: build first-build tags only.
  Extend V1's fixtures with expected edges and numbers written by hand from the fixture text,
  not saved from your own output.
- **V3.** The Rust half first (the `investigation.skeleton` event with blob, build on pin and on
  opening an old session, skeleton and mapview routes in the `GraphView` shape of overview 2.6).
  New event kinds are written only by the host; existing Investigation events, sessions and
  tests keep working untouched (run every existing `investigation*` test). The UI half is built
  on F6's `graph_view.js`, `ui_badge.js` and the node-card shell **only if they are in your
  checkout after the merge**; if they are not there yet, deliver the Rust half and fixture JSON,
  say so, and go on to V4. You may not edit `routes_session.rs`, `http_srv.rs`, `turn_runner.rs`,
  `index.html` or `app.js` (over the limit or owned by another lane): add new route and asset
  files and name, under "Merge notes", the single registration line each needs.
- **V4.** Fetching is `paper_fetch` (F3), already in your checkout; the rule for when a request
  needs no approval card is the doc comment at the top of `paper_fetch.rs` and overview 2.4:
  only a request the user's own keystroke named. Tests against the loopback mock only.
- **V5.** As the row says.

## What must hold throughout

- Zero model calls anywhere in these four packages. No tool is added and no tool definition
  changes (golden test).
- On screen, the word "verified" appears nowhere in this mode; quotes are "matched"; a dashed
  line means "not checked by code" (overview 3.2).
- Timing: record how long the skeleton build takes on each fixture and on a generated 80-page
  paper, in the report. Quote no figure you did not measure.
