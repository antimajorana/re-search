# Brief: split-2

Add isolated paper-reader subagents that read slices of the paper in parallel and save pinpointed
evidence, so the main model cites evidence by reference instead of holding the whole paper. Read
`PARALLEL.md` first.

Design: `01-reader-recorder-split.md`, Phase 2: **WP6, WP7, WP8 and WP9** (section 3), behaviour in
section 2.5 (and 2.0, 2.7), and the user's decisions 4–7 in `README.md`: reader model = the primary
provider unless one is assigned in settings; parallelism 2 on ChatGPT-plan routes and 4 on API keys,
configurable; reader effort `low`; readers **off by default**, on only when a smaller reader model is
set or the question is section-scoped, otherwise an opt-in setting.

Phase 1 (brief `split-1`) is being written in parallel and is not in your checkout. You depend on its
anchor verifier: `verify_pointer`, `AnchorFault`, `SourceIndex` in the new
`crates/research-tools/src/investigation_anchor.rs`, and the `read_paper_lines` tool in
`paper_lines.rs`. Do this: implement the evidence store and fan-out against **the interfaces exactly as
section 2.1–2.3 of the design defines them**; if a type or function you need does not exist at your
base, add a minimal version in a file of yours named `investigation_anchor_shim.rs` /
`paper_lines_shim.rs` with the design's signatures and enough behaviour for your tests, and say in the
report that the coordinator must replace the shim with split-1's real code at merge. Never weaken a
check to make a shim easier.

Files you own: the files the design lists for WP6–WP9 (new `crates/agent/src/paper_read.rs`,
`paper_read_tests.rs`, `crates/research-tools/src/investigation_evidence.rs`,
`crates/desktop-server/src/paper_reader.rs`, tests named there), plus small listed edits to
`delegate.rs` (must stay at or under 600 lines), `agent/src/lib.rs`, `honesty.rs`, `turn_runner.rs`,
`agent_models.rs`, `subagents.rs`, `routes_session.rs`, `assets/trace_cards.js`, `assets/subagents.js`
and the "located by reader" label in `assets/investigation_map.js`.

Security points to keep exactly: readers get exactly three tools; no `ledger_append`, no
`record_paper_*`, no `delegate_*`, no `ask_user`; `note_evidence` is never in the main registry; every
evidence check in 2.5 (`resolve_evidence`) is enforced; the reader card escapes all model text.

Done means: clippy and `cargo test --workspace --no-fail-fast` pass on Linux except proven
pre-existing failures; every test the design names for WP6–WP9 exists and passes; the report has
"Merge notes" and lists each shim.
