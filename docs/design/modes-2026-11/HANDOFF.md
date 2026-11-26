# Hand-off prompt: coordinating the Gemini agents on the three-modes work

State as of 2026-11-26 08:00 UTC. Everything below was written by the previous coordinator.
"Verified" means the previous coordinator ran it; anything else is marked.

---

You are taking over as coordinator of an implementation effort in the repository
`antimajorana/reSearch` (a Rust workspace, Windows-first desktop research app; licence AGPL-3.0).
The owner is Yo (GitHub `antimajorana`), a theoretical physicist. Your job: launch Gemini agents
on packages of the plan, check every branch they push yourself, and merge what passes. You do not
write the packages yourself unless an agent's work needs a small fix.

## 1. The plan

- Base branch: **`gemini/work/fixed`** (not `main`). All work merges there.
- Plan directory: `docs/design/modes-2026-11/`.
  - `00-overview.md` is the authoritative plan: three modes (Investigate, Explore, Solve) and
    shared foundation packages F0 to F16. Section 2.1 lists the packages with files and
    dependencies, section 6 the build order, section 7 the evaluation sets, section 8 the owner's
    decisions (all recommended answers accepted on 2026-11-25).
  - `investigate.md` (packages V1 to V19), `solve.md` (S2 to S22), `explore.md` (X1 to X19).
    Where a mode document and the overview disagree, the overview wins.
  - `README.md`: the status table. Keep it current in every merge.
  - `agents/RULES.md`: rules every agent follows. `agents/<brief>.md`: one brief per agent.
    `reports/<brief>.md`: each agent's report.
- Owner's priorities: Investigate first. Scope decisions of 2026-11-25: Lean and physlib are cut
  (S16); Cadabra2 and FORM (S22), V14 and version tracking (half of V10) are deferred; added are
  the paper library with sync (F13), PDF and old papers (F14, V17), write-up (F15) and learning
  from corrections (F16). In Solve, "Verified" means an exact symbolic check by code and "Checked
  numerically" means numeric agreement; there is no "Computed" tier.

## 2. Status

**Merged into `gemini/work/fixed` (head `ba3e8a4` plus the commit that adds this file):**
F0 guard rails, F2 paper key, F3 network gate and paper fetch, F4 quote core, F9 run budget, F8
evaluation harness skeleton, V1 TeX skeleton, S2 Solve model, F1 tool profiles, F6 shared UI
parts, F10 library, F14a PDF text core. (Pull requests 7 to 14.)

**Running now** (launched 2026-11-26 about 07:50 UTC; nothing pushed yet at 08:00 UTC):

| Brief | Packages | Branch it will push |
|---|---|---|
| `inv-a-v2-v5` | V2, V3, V4, V5 in order, pushing after each | `gemini/modes/inv-a-v2-v5` |
| `f5-f7-mode-shell` | F5 mode shell, F7 hand-off packet | `gemini/modes/f5-f7-mode-shell` |
| `lib-f13a-f16a-f15` | F13a sources and sync, F16a corrections by code, F15 write-up core | `gemini/modes/lib-f13a-f16a-f15` |
| `q-f11-x1-s3-s6` | F11 literature services, X1, S3, S6 (launched earlier, about 2026-11-25 23:00 UTC) | `gemini/modes/q-f11-x1-s3-s6` |

Leave these running. Known risk: `f5-f7-mode-shell` was launched while `RULES.md` still forbade
editing `routes_session.rs`, `turn_runner.rs` and `http_srv.rs`, which that brief owns. RULES.md
is corrected now, but that agent may have read the old text. If its report says it did not edit
those files, resume it (section 4, follow-ups) with one line telling it the brief wins.

**Not started, with what each waits for:**

| Next brief to write | Packages | Needs merged first |
|---|---|---|
| Investigate lane B | V6, V7, V8, V9 | V3 (and F1, F4, F9, which are in) |
| PDF skeleton | V17 | V2 |
| Investigate rest | V10 (notes only), V11, V12, V13, V15, V16, V18, V19 | V5, V8, F7 |
| Solve lane | S4, S5, then S7 to S15, later S17 to S21 | S3, S6, F5 |
| Explore lane | F12, X3, X4, then X6 to X13, later X14 to X19 | F11, X1, F5 |
| Library rest | F13b (BibTeX, Zotero), F13c, F14b (own OCR), F14c, F16b, mode adapters of F15 | F13a, F16a, F15 core |

Build order and dependencies in detail: overview section 6 and the last column of each package
table. Do not launch a package before what it depends on is merged.

## 3. Open items

- **Windows is unbuilt and untested.** All checks so far ran on Linux. `#[cfg(windows)]` code
  (sandbox, desktop app) was never compiled for this work.
- **No CI on these pull requests.** `.github/workflows/workbench-checks.yml` runs only on pull
  requests into `main`, and only on Windows. Merges into `gemini/work/fixed` had no CI. Before
  anything goes to `main`, a pull request from `gemini/work/fixed` into `main` must go green.
- **Evaluation material from Yo is missing.** F8 is only the harness. The live evaluation sets
  of overview section 7 (real papers, among them set P6 with journal PDFs and scans, and Yo's own
  library for set B) need papers and judgements only Yo can supply. PDF extraction was measured
  on self-made fixtures only.
- The `/api/library` routes exist but are not registered in `http_srv.rs`; F5 does it.
- A session started after F1 does not resume in an older build (release note draft in
  `reports/f1-tool-profiles.md`, section 4).
- `canvas_dag.js` (688 lines) and `runner.rs` (626) are over the line limit and untouched.
- Two tests fail only when run as root on Linux (a read-only file is not refused):
  `paper_read::tests::try_call_err_aborts_all_and_fails_turn` (agent) and
  `investigation_cites::tests::review_honesty_late_m2_finish_question_append_error`
  (desktop-server; cause inferred, not confirmed on the base). `cli/tests/desktop_and_ext.rs`
  hangs on Linux if `RE_SEARCH_PYTHON` is set.

## 4. How to launch an agent (Yo's recipe)

Agents are Antigravity remote agents started with the `agy` CLI (version 1.5.17) on Yo's
Windows computer, where `agy` is signed in. They run in Google's cloud and hand back by pushing a
git branch. You need a terminal on that computer.

1. **One empty folder per agent**, named `modes-<brief>`. The existing ones are under
   `C:\Users\antimajorana\AppData\Local\Temp\claude\5d097baebc286c8e\f139c356-d538-4779-87ec-f704e702d94d\scratchpad\agy\`;
   any stable location works for new ones. Add the folder to `trustedWorkspaces` in
   `~/.gemini/antigravity-cli/settings.json` (keep the existing entries).
2. **Run interactively in its own terminal tab** (a remote agent does not start otherwise):

   ```
   cd '<folder>'
   agy --remote-agent --model gemini-4-neon-high --dangerously-skip-permissions -i 'Clone https://github.com/antimajorana/reSearch.git, then read docs/design/modes-2026-11/agents/RULES.md and docs/design/modes-2026-11/agents/<BRIEF>.md on branch gemini/work/fixed (git show origin/gemini/work/fixed:...). Carry the brief out completely and unattended under those rules, and push the result to branch gemini/modes/<BRIEF> with the exact push command in RULES.md. Nobody can answer questions: decide and record decisions in your report.'
   ```

   No single quote inside the prompt. Models: `gemini-4-neon-high` by default,
   `gemini-4-carbon-high` for long reviews.
3. **At most six terminal tabs at once** on that computer. Four are in use.
4. **Hand-back.** The agent pushes `gemini/modes/<brief>` and ends with a `PUSHED` block (brief,
   branch, head). Watch for branches with `git ls-remote origin 'gemini/modes/*'`.
5. **Follow-ups.** Nobody can type into a running agent. To send one more instruction, exit its
   TUI with Ctrl-C twice (this prints a resume id) and relaunch in the same folder with
   `--conversation=<id>` and the new instruction as the prompt. Resume ids of finished agents:
   f3 `554a9024-64c6-4653-8284-2ac90189b851`, f4-f9 `42e86e45-4988-4374-a1d1-9043f2ee04ab`,
   v1 `8975b1ee-b042-47ad-a2a2-c1d35e50b8a0`, s2 `58c9482c-63d0-45b8-a7b0-8d95c995ca26`,
   f1 `e151bf11-918d-4e53-9f59-059e47dafe55`, f14a `8f371716-61e9-4a5f-9f5f-6a696ae55f90`,
   f6 `f2219258-bf0c-473f-81d1-3297413995b7`, f10 `cb9f0d07-1bda-4f3f-8cbf-4e8a37355378`.
6. **Never print, copy or store a credential**, and never ask Yo to paste one into a chat.

A brief is a file in `agents/` on `gemini/work/fixed`, so write and merge the brief before you
launch its agent. Use `inv-a-v2-v5.md` and `lib-f13a-f16a-f15.md` as models: name the design
sections, the order, the files the agent owns and may not touch, and what must hold. An agent
does exactly what the text says, so say what not to build.

## 5. Verify, then merge

**Treat an agent's report as a claim. These agents accept instructions uncritically and
over-fix. Tell Yo nothing is done before you have checked the branch yourself.** For each pushed
branch:

1. Fetch it. Read `reports/<brief>.md` in full, above all deviations, decisions taken alone, and
   behaviour changes. Read the diff of every existing file it edited.
2. Merge it onto a work branch cut from `gemini/work/fixed` (`git merge --no-ff`). Module lists
   in `crates/research-tools/src/lib.rs` conflict often: keep both sides.
3. Run, for every crate touched (Linux; unset `RE_SEARCH_PYTHON` for the `cli` crate):
   - `CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 cargo test --locked -p <crate> --no-fail-fast`
   - `cargo clippy --locked -p <crate> --lib --bins -- -D warnings`
   - `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`
   - if anything under `crates/desktop-server/assets/` changed:
     `node --test crates/desktop-server/tests/*.test.mjs`
4. Check the fixed points:
   - **No source file over 600 lines**, tests included. A file already over the limit may be
     touched only after code is moved out of it in a separate commit that changes no behaviour.
   - **`Cargo.lock` and every `Cargo.toml` unchanged** (no new dependency without Yo's yes).
   - **Tool definitions unchanged**: `crates/research-tools/tests/golden/tool_defs.txt` and
     `tool_profiles.txt` are not regenerated unless the package is the one that adds a profile
     or a tool, and then the change says how earlier sessions keep resuming.
   - **Resume stays fail-closed and old sessions still resume**:
     `crates/desktop-server/tests/eval_set_r.rs`, `crates/agent/tests/resume_loop.rs` and
     `resume_drift_tests.rs` pass untouched.
   - No test weakened, skipped or deleted. No third-party paper text. No credential anywhere.
     No real network call in tests. The sandbox, approval flow and policy engine not weakened.
5. If it passes: update the README status row, push the work branch, open a pull request into
   `gemini/work/fixed`, and merge it **with a merge commit, not a squash** (agent branches are
   based on older heads and must keep merging cleanly). Yo's standing instruction is "push and
   merge as things are done". In the pull request, say what you checked and what you did not
   (Windows, real PDFs).
6. If it fails: fix a small defect yourself and say so, or resume the agent with a precise
   instruction. Do not merge with a known failure that is not on the base too.

Report to Yo in batches, answer first, and separate what you verified from what you inferred.
Yo wants direct answers, no flattery, and "I don't know" where that is the truth.
