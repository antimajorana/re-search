# Brief: ux-rust (wave 1)

Make Investigation mode answer the user's question first, without a whole-paper map and conventions
catalog, and give Investigation sessions their own system prompt.

Design: `02-investigation-ux.md`. Implement the Rust packages **P0, P1a, P1b, P2 and P3** (section 3),
in that order: P0 is a pure move and must land first, as its own commit, with all existing tests green.
Read sections 2.1–2.9 for the behaviour (D1–D4, D8) and section 4 for reviewer corrections.

Files you own: the files the design lists for P0–P3, which are
`crates/desktop-server/src/investigation.rs` and the new `investigation_view.rs`,
`investigation_finish.rs`, `investigation_question.rs`, `investigation_system_prompt.rs`,
`investigation_title.rs`, `investigation_tests.rs`; `crates/desktop-server/src/lib.rs` (mod lines);
`crates/desktop-server/src/turn_runner.rs` (system prompt selection only);
`crates/desktop-server/src/routes_session.rs` (`session_title` only);
`crates/research-tools/src/investigation.rs`, new `crates/research-tools/src/investigation_prompts.rs`,
`crates/research-tools/src/lib.rs` (mod line), `crates/research-tools/tests/investigation.rs`
(only additions), `crates/desktop-server/tests/investigation_flow.rs` (only additions), and the
resume test file the design names for P2. Another agent is changing the UI assets for math at the
same time: do not touch anything under `crates/desktop-server/assets/`. The UI half of this design
(P4, P5) is a later wave; keep the new view fields (`answers`, `mapping_requested`) exactly as the
design defines them so the UI agent can rely on them.

Also needed later by another wave: `crates/research-tools/src/investigation.rs` must end at or under
600 lines after P0 (it is 771 at the base). If the P0 move alone does not get it there, move more
pure helpers into new files and say so in the report.

Done means: `cargo clippy --workspace --all-targets -- -D warnings` and
`cargo test --workspace --no-fail-fast` pass on Linux except for failures you prove pre-existing;
every test the design lists for P0–P3 exists and passes; no existing assertion changed; every touched
or new file at or under 600 lines; the report states which behaviours could only be checked on Windows.
