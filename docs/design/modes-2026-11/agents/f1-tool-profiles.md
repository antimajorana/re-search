# Brief f1-tool-profiles: tool profiles (package F1)

**You own this package and the hot files it names.** The rule in RULES.md against editing
`crates/agent/src/resume.rs` does not apply to you. You still may not edit `routes_session.rs`,
`turn_runner.rs` or `http_srv.rs`.

Design: `00-overview.md` section 2.2 (all of it), row F1 of 2.1, section 1 (rows on the drift
check and on mode detection), 3.1 rows 1, 2, 3 and 12, 5.3 (what happens to today's sessions),
7.2 set R. `investigate.md` section 8 (migration and def_hash), `solve.md` 5.1 and the rules R1
and R2, `explore.md` 8.4.

## Deliver

1. `crates/research-tools/src/tool_profile.rs`: a profile is data, `{name, defs, may_gain: [id],
   earlier_defs: {id: [ToolDef]}, policy}`. Profiles today: `research/1` (exactly
   `all_research_tool_defs()` plus the host tools a router adds, as pinned by the golden file) and
   `investigation/1` (the same list; the 7 tolerated additions and the 2 special cases of
   `check_tool_defs_drift` moved into data). Define the lookup for `investigation/2`, `explore/1`
   and `solve/1` so that asking for one of them is a clear error "profile not built yet", not a
   panic: their tools do not exist, and you add no tool.
2. `crates/agent/src/resume_profile.rs` and the edit to `resume.rs`: resume builds the expected
   definitions from the recorded profile and stays fail-closed. A session with no `tool_profile`
   field resumes exactly as today (`research/1`, or `investigation/1` when its latest mode marker
   is `investigation.started`). The behaviour of `check_tool_defs_drift` for every ledger that
   resumes today must not change: every existing test in `resume_drift_tests.rs`,
   `tests/resume_loop.rs` and `paper_read_tests.rs` passes untouched, and you add table-driven
   tests that run the old function's cases against the new data-driven one (keep the old logic in
   the test module as the oracle, or prove equivalence case by case).
3. `session.start` and `session.resume` record `tool_profile`. Edits in `autonomous.rs`,
   `guardrails.rs` if it writes a start event, `desktop-server/src/routers.rs`,
   `mcp-server/src/main.rs`, `cli-route/src/core_tools.rs`, `cli/src/runtime.rs`: each builds its
   router from a profile. No tool gains or loses a definition; the golden file is unchanged. Add
   one golden section per profile in a new file next to it (do not regenerate `tool_defs.txt`).
4. The policy belongs to the profile, so resume compares the right policy version (3.1 row 12).
5. Set R of the evaluation plan: hand-build ledger fixtures of the four existing session types
   (research, investigation before pointer citations, investigation current, hand-off child) with
   the real writer code in a test, and assert each resumes with all new code present. If you
   cannot construct one faithfully, say which and why; do not fake it.
6. A session started after this change records a profile and will not resume in an older build:
   say so in a short `docs/releases/` note draft inside your report, not in a release file.

## Care

This is the fail-closed heart of resume. When unsure, refuse to resume rather than accept. List
every behaviour difference you know of, however small, under "Deviations".
