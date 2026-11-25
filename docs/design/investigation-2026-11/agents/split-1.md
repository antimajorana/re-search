# Brief: split-1

Stop one inexact quote from forcing a whole record to be rewritten, and let the model cite paper
lines by pointer, with the app filling in the exact text. Read `PARALLEL.md` first.

Design: `01-reader-recorder-split.md`, Phase 1: **WP1a, WP1b, WP2, WP3, WP4 and WP5** (section 3),
behaviour in sections 2.0–2.4 and 2.7, and the user's decisions 1–3 in `README.md`. Do WP1a first.
Do not do Phase 2 or 3 (other briefs do them in parallel).

In parallel, brief `ux-rust` is moving code out of `crates/desktop-server/src/investigation.rs` and
`crates/research-tools/src/investigation.rs` (UX design P0) and adding question turns and an
Investigation system prompt. To keep the merge small: put the anchor work in the new files the design
names (`investigation_anchor.rs`, `investigation_anchor_tests.rs`, `paper_lines.rs`,
`investigation_legacy_defs.rs`, `investigation_prompts.rs` in desktop-server); in the two
`investigation.rs` files change only call sites and mod lines, and do not do the 600-line reduction
there (ux-rust does it); list every edit to those two files under "Merge notes". For WP4, add only the
route-aware wording, the pointer-citation instructions and the Phase 1b `DelegationConfig` gating;
leave the Research `final_answer` gating to ux-rust's Investigation system prompt and say so.

WP3 needs golden `def_hash` values of the four recorders at `7c36a0e`: compute them from a separate
checkout (`git worktree add /tmp/base 7c36a0e`) and hard-code them.

Your UI part (WP5) is limited to `investigation_map.js` and `investigation_conventions.js` and their
tests; render through the existing exports `renderMathTextInto` / `renderRichInto` (brief `latex` is
rewriting their insides, not their names) and build no HTML strings.

Done means: clippy and `cargo test --workspace --no-fail-fast` pass on Linux except proven
pre-existing failures; every test the design names for WP1a–WP5 exists and passes; the legacy excerpt
path is byte-identical and `crates/research-tools/tests/investigation.rs` is unchanged and green; new
files at or under 600 lines; the report maps each honesty guarantee of section 2.7 to the test that
proves it, and has "Merge notes".
