# All briefs run at the same time (user decision, 2026-11-25)

The user chose to run every brief at once from the same base (`gemini/runner` at the commit you
cloned) and to merge and check everything at the end. So:

- The other briefs' work does **not** exist in your checkout. Do not wait for it, and do not
  implement another brief's packages. The table in `README.md` says who does what.
- Where your design text says a package runs "after" another brief's package, implement your part
  against the code as it is at your base, keeping the interfaces the design defines, so the
  coordinator can merge the two. Note each such place in your report under "Merge notes" with the
  file, the function and what the other brief will change there.
- Prefer new files over edits to files that another brief owns. When you must edit a shared file
  (for example `crates/desktop-server/src/investigation.rs`, `turn_runner.rs`, `lib.rs`,
  `crates/research-tools/src/investigation.rs`, `src/assets.rs`, `app.js`), keep the edit small and
  local, in its own commit, and list it under "Merge notes".
- Nobody runs your code together with the other briefs before the end; your own tests must pass on
  your branch alone.

Who owns what, when two briefs touch the same file:

| File | Main owner | Others may make small, listed edits |
|---|---|---|
| `crates/desktop-server/src/investigation.rs` (and the split-out files) | ux-rust | split-1 (route, prompts), split-3 (scope) |
| `crates/research-tools/src/investigation.rs` | ux-rust (P0 move) | split-1 (anchors, recorders), split-2 (evidence form) |
| `crates/desktop-server/src/turn_runner.rs` | split-2 (reader wiring) | ux-rust (system prompt), split-1 (DelegationConfig gating) |
| `crates/desktop-server/assets/math*.js`, `markdown.js`, CSS for math | latex | none |
| `crates/desktop-server/assets/investigation.js`, `composer.js`, `index.html`, `app.js` | ux-ui | latex (`app.js` 2 lines), split-3 (`investigation.js` scope chip) |
| `crates/desktop-server/assets/investigation_map.js`, `investigation_conventions.js` | split-1 | ux-ui (`openFirst` option, header gate), split-2 (reader label) |

## Which Gemini model runs which agent (user's rule, 2026-11-25)

- Default for every agent (implementation, merge, fixes): **Gemini 4 Neon (High)**,
  `--model gemini-4-neon-high`.
- Long reviews and audits: **Gemini 4 Carbon**, `--model gemini-4-carbon-high`.

The coordinator sets the model when it launches the agent; an agent does not switch model itself.
- Fix rounds (user's choice, 2026-11-25): one orchestrating agent (`gemini-pro-orchestrator` is refused for
  remote agents, "not available for remote agents"; the CLI falls back to Neon High),
  which hands each fix to Neon (High) subagents; the coordinator verifies its output at the end.
- Orchestration from now on (user, 2026-11-25): **Gemini 4 Carbon (Medium)**, `--model gemini-4-carbon-medium`,
  with the `/teamwork` command, which does what the Orchestrator model would. Agents already running are left
  as they are.
