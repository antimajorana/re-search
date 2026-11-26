# Brief f5-f7-mode-shell: packages F5 (mode shell) and F7 (hand-off packet), in order

You own the files that every mode has to register in. Other agents are forbidden to edit them
and leave one registration line each in their reports; you make room for those lines.

Design: `00-overview.md` rows F5 and F7 of section 2.1, sections 2.2 (profiles, rule R2), 3.1 and
4 (hand-off) in full. Package F1 (tool profiles) is in your checkout: read
`reports/f1-tool-profiles.md` first, above all its "Integration hooks" for F5.

## Order and hand-back

F5 first, then F7. After each: run the checks of RULES.md, commit, and push at once to
`gemini/modes/f5-f7-mode-shell`. Before F7, `git fetch origin` and
`git merge origin/gemini/work/fixed`.

## F5

1. **Make room first, as separate commits that change no behaviour.** `http_srv.rs` (702 lines),
   `routes_session.rs` (616) and `assets/index.html` (768) are over the 600-line limit. Move
   code out until each is under 560, so that later one-line registrations fit. For `http_srv.rs`
   move the API dispatch table into a new `api_dispatch.rs`; every existing route keeps its
   path, method, auth check and response. Run every existing desktop-server test and every
   `tests/*.test.mjs` after each move.
2. **Register what is waiting.** Add the `/api/library/..` arm exactly as written in the header
   of `routes_library.rs` (it must sit behind the same `AuthGuard::authorize_api` check as every
   other `/api/` path). Add a test that an unauthenticated request to `/api/library` is refused
   and an authenticated one answers.
3. **`session_mode.rs`.** One function for the mode of a session. Reuse
   `research_tools::tool_profile::session_mode_from_events`; do not write a second rule.
   `turn_runner.rs` selects the profile from the mode and calls
   `routers::build_desktop_router_for_profile`. A mode whose profile is not built yet gives a
   plain error to the user and starts nothing.
4. **Marker test (rule R2).** Mode markers (`investigation.started`, `investigation.handoff`,
   `idea.started`, `solve.started`) are written only by the host. Add a test that enumerates
   every registered tool handler's emitted event kinds and fails if any of them is a marker.
5. **New menu and rail tags** (`assets/new_menu.js`, `session_rail.js`). Explore and Solve are
   listed but disabled with the words "not built yet" until their profiles exist; nothing is
   faked. Node tests for both.

## F7

As section 4 of the overview says: one packet type in `export/src/handoff.rs`, the three
payloads, and `desktop-server/src/handoff.rs`. The existing Investigation to Research hand-off
(`export::fork::research_from_paper`) keeps working byte for byte for old sessions: the Set R
tests in `crates/desktop-server/tests/eval_set_r.rs` must pass untouched.

## What must hold

- No tool is added and no tool definition changes (both golden tests pass untouched).
- Every session that resumes today still resumes (`eval_set_r`, `resume_loop`,
  `resume_drift_tests`, untouched).
- No model call and no network call in tests.
