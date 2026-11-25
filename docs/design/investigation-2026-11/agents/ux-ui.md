# Brief: ux-ui

Build the Paper view half of the question-first Investigation design. Read `PARALLEL.md` first: the
Rust half (brief `ux-rust`) is being written at the same time and is not in your checkout.

Design: `02-investigation-ux.md`. Implement **P4** and then **P5** (section 3), with the behaviour of
D5, D6, D7 and D9 (sections 2.6–2.10), and section 4's reviewer corrections. The session view will
carry `answers` and `mapping_requested` exactly as section 2.5/2.6 and P1b define them; code against
those fields, treat them as absent when missing (as the design says for "until P1b merges"), and build
your Node tests on fixture views that include them.

Files you own: those the design lists for P4 and P5 under `crates/desktop-server/assets/`
(`investigation.js`, new `investigation_answer.js`, new `investigation_state.js`, `index.html` setup
form and tab label only, `investigation.css`, `investigation_conventions.js` gate only,
`investigation_map.js` `openFirst` option only, `composer.js` Esc guard and placeholder setter,
`app.js` `setBusy` option and two `dispatchEvent` lines), their `tests/*.test.mjs`, new `src/assets.rs`
entries, and the Rust part P5 needs. The math renderer is also being rewritten in parallel (brief
`latex`): render model text only through the existing exports `renderMathTextInto`, `renderRichInto`
or `textContent` (their names stay), never by building HTML strings.

Browser tests (`investigation_browser.py`) need Windows: write or update them as the design says, but
do not run Playwright; say so in the report.

Done means: every Node test the design lists for P4/P5 exists and passes with
`node --test crates/desktop-server/tests/*.test.mjs`; `node --check` passes on every asset;
`cargo test -p desktop-server` passes on Linux if it builds; no existing assertion changed; all files
at or under 600 lines except `app.js`, which gains at most 3 lines; "Merge notes" in the report.
