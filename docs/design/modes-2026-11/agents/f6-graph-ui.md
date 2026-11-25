# Brief f6-graph-ui: graph view and shared UI parts (package F6)

Plain JavaScript, no build step, no new dependency, no cargo work except running the existing
asset tests. Investigate is the first user of this package.

Design: `00-overview.md` section 2.6 (all of it), 2.10 (the evidence scale and its look), 3.2
rows 20, 24 and 25 (words and the meaning of dashed), 3.3 (run states for the run bar), 5.2
(names). `investigate.md` section 3 (what the user sees) for the first adapter.

## Deliver

`crates/desktop-server/assets/graph_layout.js`, `graph_view.js`, `ui_badge.js`, `run_bar.js`,
`quote_chip.js`, with `node --test` tests in `crates/desktop-server/tests/*.test.mjs`.

1. `graph_layout.js`: pure functions (layering, lanes by `group`, folding above 150 nodes) over
   the `GraphView` JSON of 2.6. Extract what exists in `investigation_graph.js` (280 lines) and
   `canvas_dag.js` (688, over the limit) rather than writing a second layout. `canvas_dag.js`
   must end under 600 lines because you touch it; if the extraction cannot get it there without
   risk, leave `canvas_dag.js` untouched, extract from `investigation_graph.js` only, and say so.
2. `graph_view.js`: draws, pans, selects, filters. Edge direction fixed (`src` prerequisite on
   top, `dst` dependent below). Solid = found by code or backed by a matched full-text quote;
   dashed = model-inferred, abstract-only or unchecked. Statement use versus proof use by line
   weight and a toggle.
3. `ui_badge.js`: a badge is always text plus icon, never colour alone. `run_bar.js`: state,
   elapsed, "7 of at most 24 model calls", Stop, Continue, for the seven states of 3.3.
   `quote_chip.js`: quote (at most 300 characters), paper, lines, optional page, opens the source.
4. Everything that comes from a paper, a model or a ledger payload is set with `textContent` or
   the existing escape helpers. Add a test that feeds `<img onerror>` style strings through every
   component and finds no element created from them.
5. The existing views keep working: `node --check` every asset, run every existing `.test.mjs`,
   and keep `investigation_graph.js` rendering through the new layout with the same output for
   its existing tests.
6. Fixture `GraphView` JSON for one small Investigate map, written by you, used by the tests and
   usable by the later V3 package.

## Not in this brief

Rust views (`mapview`, `idea_view`, `solve_view`), the home screen and New menu (F5), any route.
Do not edit `index.html` (768 lines, over the limit) or `app.js` (932): export functions that a
later package mounts.
