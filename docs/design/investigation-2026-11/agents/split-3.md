# Brief: split-3

Build a deterministic outline of the paper (sections, appendices, equations, figures) and use it to
scope a question such as "help me understand section V.D.4" to the right lines, with a section-scoped
conventions catalog. Read `PARALLEL.md` first.

Design: `01-reader-recorder-split.md`, Phase 3: **WP10, WP11 and WP12** (section 3), behaviour in
section 2.6, and the user's decision 8 in `README.md`.

The question-first turn (brief `ux-rust`, UX design D1/D2) is being written in parallel and is not in
your checkout. The two fit together like this: ux-rust decides *which kind* of turn runs; your scope
says *which lines* the turn should read. Implement `build_outline` and `resolve_scope` as pure
functions in the new `crates/research-tools/src/investigation_outline.rs` with thorough tests (WP10),
including a test on realistic text shaped like an APS paper whose heading for V.D.4 looks like
"4. Dislocations ..." under "D." under "V." (write your own fixture text; do not fetch papers). Then
make the smallest possible changes to `prepare` and the conventions code for WP11, listed under
"Merge notes", and the UI chip and labels for WP12 as a separate small module if you can
(`assets/investigation_scope.js`) rather than large edits to `investigation.js`.

Done means: clippy and `cargo test --workspace --no-fail-fast` pass on Linux except proven
pre-existing failures; every test the design names for WP10–WP12 exists and passes; the report says
how the outline does on each fixture and has "Merge notes".
