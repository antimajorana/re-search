# Brief: merge

Combine the six briefs' branches into one branch that builds and passes its tests. You start while
some briefs are still running, so merge in rounds.

```bash
git clone https://github.com/antimajorana/reSearch.git ~/re-search && cd ~/re-search
git switch -c gemini/work/merged origin/gemini/runner
git fetch origin 'refs/heads/gemini/work/*:refs/remotes/origin/gemini/work/*'
```

Merge order (it follows the ownership table in `PARALLEL.md`): `ux-rust`, `ux-ui`, `latex`,
`split-1`, `split-2`, `split-3`. Use `git merge --no-ff origin/gemini/work/<brief>`, one brief per
merge commit. Before each merge read that brief's `reports/<brief>.md`, especially "Merge notes".

Rules for resolving conflicts:
- The main owner in `PARALLEL.md` wins the structure of a file; the other brief's change is
  re-applied on top of it. Never drop either side's behaviour or tests.
- `split-2` may contain shims (`investigation_anchor_shim.rs`, `paper_lines_shim.rs`): replace each
  with `split-1`'s real code and delete the shim.
- `split-1` kept the Research `final_answer` gating out on purpose; `ux-rust`'s Investigation system
  prompt covers it. `split-3`'s scope feeds `ux-rust`'s question turn: wire them together as section
  2.6 of design 01 and D1/D2 of design 02 describe.
- All of `RULES.md` applies (no weakened tests, escaping, 600-line limit, no credentials).

After each merge: `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace --no-fail-fast` (set `RE_SEARCH_PYTHON`; the test
`cli_research_workspace_read_fork_diff_export_and_verify` in `crates/cli/tests/desktop_and_ext.rs`
is known to hang on Linux: skip it with `-- --skip` for that crate and say so),
`node --check` on every asset and `node --test crates/desktop-server/tests/*.test.mjs`. Fix what the
combination broke, in separate commits after the merge commit.

After each round push: `git push origin HEAD:refs/heads/gemini/work/merged` (only that branch, no
force). If a brief's branch is not on GitHub yet, merge what exists, push, then run
`sleep 600` and fetch again; repeat until all six are merged or 3 hours have passed. Write
`docs/design/investigation-2026-11/reports/merge.md`: per brief, the conflicts and how each was
resolved; shims replaced; test counts after the last merge; anything left failing; what only Windows
can check. End with the PUSHED block for `gemini/work/merged`, listing which briefs are in it.
