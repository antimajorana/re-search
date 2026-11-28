# Report: `solve-s13-s17-ui-intake` — Solve Derivation UI (S13) & TeX Intake (S17)

- **Brief:** `docs/design/modes-2026-11/agents/solve-s13-s17-ui-intake.md`
- **Base commit (`origin/gemini/work/fixed`):** `24be58e6797679cfa423dcc16ea2073b54288714`
- **Push branch:** `gemini/modes/solve-s13-s17-ui-intake`
- **Execution environment:** Linux x86_64, non-root (`uid=1001`), `RE_SEARCH_PYTHON` unset for workspace runs.

---

## 1. Summary of Commits & Implemented Files

All touched source files stay strictly at or under 600 lines (`python3 scripts/check_file_length.py --base origin/gemini/work/fixed` passes). Zero edits were made to `Cargo.lock`, `canvas_dag.js`, `hitl_ui.test.mjs`, `app.js`, `http_srv.rs`, `solve_setup.js`, `investigation_outline_tex.rs`, `tool_defs.txt`, `tool_profiles.txt`, or `.github/`.

### Commit 1 (`ac1365d`) — `assets_fonts.rs` split (zero behavior change)
- `crates/desktop-server/src/assets_fonts.rs` (90 lines, new): extracted KaTeX WOFF2 font static asset arms out of `assets.rs`.
- `crates/desktop-server/src/assets.rs` (510 lines final, modified): delegates `/assets/katex/fonts/*` to `assets_fonts::serve_katex_font`.
- `crates/desktop-server/src/lib.rs` (252 lines final, modified): added `mod assets_fonts;`.

### Commit 2 (`a32c9e3`) — Checkpoint S13a (`solve_badges.js`)
- `crates/desktop-server/assets/solve_badges.js` (415 lines, new): maps all 11 S2 `StepTier` / `StepBadge` JSON values onto F6 `ui_badge.js` + `graph_view.js` styles (`Numerically checked`, `Found`, `Verified`, `Given`, `Checked by you`, `Model agreed`, `Model agreed (same model)`, `Assumed by model`, `Unchecked`, `Failed`, `Stale`), preserves server-formatted `"rests on ..."` flags and `broken_by` jump targets verbatim, enforces R22 (never emits `"Verified"` on stale/non-verified badges or legends when 0 verified steps exist), and adapts Solve graph JSON for `graph_view.js` including the `"Closure of the final answer"` filter.

### Commit 3 (`508cb82`) — Checkpoint S13b (`solve_steps.js`, `solve_why.js`, `solve_exprs.js`, `solve_ui.test.mjs`)
- `crates/desktop-server/assets/solve_steps.js` (590 lines final, new): topological derivation step cards rendering claims strictly from `solve.expr.latex` handles via `math_katex.js` (R5; ignores `step.latex`), Result sheet (`Done` vs `Done with open items` strictly from `solve.done.status`, R15), F6 `run_bar.js` + `graph_view.js` (`"Closure of the final answer"` toggle), in-place stateful `ws.update()` preserving open Why popovers / R19 warnings / graph / expressions / custom tabs, and `registerSolveTab({ id, label, mount })` extension registry for S18a Artifacts and S17b Transcribe.
- `crates/desktop-server/assets/solve_why.js` (503 lines, new): Why provenance popover with current/stale checks partitioning, receipt code toggle, R19 `"result changed on re-run"` warning with evidence SHA diff, lowering/broken ancestors, source `QuoteChip` (strictly `source_ref.quote ?? excerpt`, never leaking model summary/details), and all 7 actions (`Fork`, `Attach source`, `Ack`, `Add check` with plain-object JSON check, `Re-run checks`, `Correct this step` firing `onStepCorrected` for F16b, and `"Review with another model"` gracefully disabled until S14 merges).
- `crates/desktop-server/assets/solve_exprs.js` (260 lines, new): searchable Expressions handle table (`name@version`, LaTeX, `str_preview`, `free_symbols`, strict `portable === true` badge, owner step jump) plus `buildHandleLookup`, `renderHandleLatex`, and `renderClaimFormula` (R5).
- `crates/desktop-server/assets/solve_state.js` (412 lines, modified): preserves `handles`, `checks`, `receipts`, `artifacts`, `doneResult`, and `plan` across state hydration.
- `crates/desktop-server/assets/new_menu.js` (288 lines, modified): adds `SOLVE_ENABLED_STORAGE_KEY`, `isSolveMenuEnabled`, `buildModeMenuEntries`, and `enableSolveButton` (see Decision #1 below).
- `crates/desktop-server/assets/solve.css` (483 lines final, modified): S13 workspace/border/Why/`.gv-*` styles and S17b two-column styles.
- `tests/fixtures/solve_view_fixture.json` (244 lines, new): self-written 1D anharmonic oscillator toy fixture covering 10 steps (`e-1`..`e-10`), 9 handles (including non-portable `raw_op@1`), checks (`pass`, `fail`, `inconclusive`, `not_run`, stale), receipts, graph, and `done`.
- `crates/desktop-server/tests/solve_ui.test.mjs` (599 lines, new): covers S13a/S13b badge tiers, R5, R15, R19, R22 zero-verified sweep, `registerSolveTab`, `onStepCorrected`, stateful `ws.update()`, QuoteChip non-leak, and zero-`innerHTML` XSS safety.

### Commit 4 (`aba2b10`) — Checkpoint S17a (`solve_derivation.rs`, `solve_intake.rs`, `solve_intake_routes.rs`)
- `crates/research-tools/src/solve_derivation.rs` (266 lines, new) + `crates/research-tools/src/solve_derivation_scan.rs` (493 lines, new) + `crates/research-tools/src/solve_derivation_tests.rs` (151 lines, new): deterministic (`< 1` s, 0 model calls) TeX-only splitter reusing `investigation_outline::build_outline` (`investigation_outline_tex.rs`, read-only) for section anchors and numbered equation labels; extracts `equation`/`align`/`gather`/`eqnarray`/`multline`/`alignat`/`flalign`/`displaymath` (starred and unstarred), `\[...\]`, and `$$...$$`; produces consecutive equation pairs; rejects PDFs (`.pdf` or `%PDF-`), unclosed delimiters, NUL/control bytes, invalid UTF-8, and > 2 MiB inputs fail-closed.
- `crates/research-tools/src/lib.rs` (353 lines, modified): exports `pub mod solve_derivation;`.
- `crates/desktop-server/src/solve_intake.rs` (430 lines, new): `POST /api/sessions/{id}/solve/transcribe` route; rejects raw/JSON PDFs (`400`), oversize inputs (`413`), and concurrent running turns (`409`); pins raw TeX via `research_tools::ingest::ingest_bytes`; appends `proof.entry` v1 steps (`entry_type: "derivation"`, `origin: "given"`, consecutive `depends_on` pairs, `transcription: { source_tex_sha256 }`, `entry_sha256`); supports idempotent re-submission (`idempotent: true`) and explicit `replace: true` superseding prior transcribed steps; returns `given_source: { source_id, file_sha256 }`, `raw_blob`, `normalized_sha256`, `steps`, `pairs`, and `view`; sanitizes filenames including Windows reserved stems (`CON`, `NUL`, etc.) and 120-char stem caps.
- `crates/desktop-server/src/solve.rs` (332 lines, modified): 1-line dispatch arm `("POST", ["transcribe"]) => crate::solve_intake::handle_transcribe(...)`.
- `crates/desktop-server/tests/solve_intake_routes.rs` (193 lines, new): HTTP integration tests covering deterministic split (< 1 s, 0 model calls), `source.ingested` provenance, consecutive `depends_on` pairs, idempotency, `replace: true`, `409` conflict on wrong `entry_sha256` or unreplaced different derivation, `POST .../solve/ack { kind: "transcription" }` clearing `"transcription unconfirmed"`, `400` PDF/raw-PDF rejection, `413` oversize rejection, and non-Solve session rejection.

### Commit 5 (`96dc7ed`) — Checkpoint S17b (`solve_transcribe.js`, `solve_transcribe.test.mjs`)
- `crates/desktop-server/assets/solve_transcribe.js` (510 lines, new): Check-a-derivation UI with TeX-only drop/file/paste intake and fail-closed client-side PDF rejection (`.pdf`, `application/pdf`, `%PDF-`), side-by-side `"your TeX"` vs `"as checked"` cards (R5 strictly via `solve.expr.latex` handles), hash-bound `"transcription (model), unconfirmed"` chip cleared only on server confirmation matching the current `entry_sha256`, `[Confirm]` and `[Confirm all]` (D13), consecutive pair verdict strip highlighting failed steps in red, `SOLVE_TRANSCRIBE_TAB`, and `registerSolveTranscribeTab`.
- `crates/desktop-server/assets/solve_steps.js` (updated in commit 5): forwards `token`, `fetch`, and `onTranscribe` into `custom.mount(panel, ctx)` and updates mounted custom tabs in-place via `activeExtApi.update(view)` on `ws.update()`.
- `crates/desktop-server/src/assets.rs` (updated in commit 5): registers `/assets/solve_transcribe.js`.
- `crates/desktop-server/tests/solve_transcribe.test.mjs` (397 lines, new): covers PDF rejection (extension, MIME, magic header, drag-and-drop preserving drafts), side-by-side R5 rendering, non-optimistic hash-bound confirmation, `[Confirm]` & `[Confirm all]` (D13), stale `localAcks` reset on `update(nextView)`, workspace `registerSolveTab` + `ws.update()` forwarding, R22 zero-verified sweep, and zero-`innerHTML` XSS audit.

---

## 2. Independent Audits Conducted

Three independent hard audits were run with `pro` (`research` read-only subagent) across S13b, S17a, and S17b, and all blocker/medium findings were fixed and regression-tested before committing each checkpoint:
- **S13b audit:** fixed stateful `ws.update()` preserving open Why popovers / R19 warnings / graph / expressions, removed `step.details`/`step.summary` fallback in `QuoteChip`, added stale R22 guards in `solve_badges.js` & `solve_steps.js`, fixed `broken_by` jumps when `rests_on_line` is null, added strict `portable === true` and plain-text series order in `solve_exprs.js`, validated plain-object JSON check params and all F2 `PaperKey` prefixes in `solve_why.js`, and added `.solve-border--*` / `.gv-*` CSS rules.
- **S17a audit:** split `solve_derivation_scan.rs` for line-length headroom, rejected unclosed delimiters and NUL/control bytes, restricted `\\[...]` skip to genuine TeX length specs (preventing `[A, B]` truncation), added multi-line `\intertext`/`\shortintertext` and `\\begin{` masking for read-only `investigation_outline_tex.rs`, aligned `\nonumber`/`\notag` labels, and added `413` oversize, raw `%PDF-`, running-turn `409`, idempotency/`replace: true`, `given_source`, and Windows reserved filename sanitization in `solve_intake.rs`.
- **S17b audit:** forwarded `token`/`fetch`/`onTranscribe` and in-place `update(view)` for custom tabs in `solve_steps.js`, removed optimistic fallback in `applyAckResponse` and reset `localAcks` on `update(nextView)` / `res.view`, stripped stale `'transcription unconfirmed'` flag from confirmed `createSolveBadgeGroup` display, added CSRF token fallback + `credentials: 'same-origin'`, added MIME + drag/drop PDF rejection without clobbering drafts, and wrapped `&` rows in `aligned` for left-column `"your TeX"` preview.

---

## 3. Decisions, Deviations & Assumptions

1. **Decision — Keeping default `MODE_MENU_ENTRIES[solve].built = false` in `new_menu.js` until S12 merges:**
   - On `gemini/work/fixed`, `crates/desktop-server/src/session_mode.rs:94` still has `const DESKTOP_MODE_SOLVE_ENABLED: bool = false;` (owned by `solve-s12-loop` per `RULES.md:87`). Three existing tests (`new_menu_and_rail.test.mjs`, `mode_stages_mount.test.mjs`, `mode_stages_browser.py`) assert `solveEntry.built === false` and `#new-solve-btn[disabled]`, and `RULES.md:67` forbids weakening existing tests.
   - Therefore `new_menu.js` keeps `MODE_MENU_ENTRIES[solve].built = false` by default and adds `isSolveMenuEnabled`, `buildModeMenuEntries({ solveBuilt })`, and `enableSolveButton(rail, onSelect)` so S12 can flip Solve on in one line without breaking pre-S12 tests.
2. **Decision — Leaving `mode_stages.js` untouched:**
   - Brief §4 does not list `mode_stages.js` as an owned file. `createSolveWorkspace`, `SOLVE_TRANSCRIBE_TAB`, and `registerSolveTab` are fully ready and tested via `solve_ui.test.mjs` and `solve_transcribe.test.mjs`, with exact mount instructions under Merge Notes below.
3. **Decision — S17a idempotency & `replace: true`:**
   - Re-posting the exact same TeX derivation to `POST .../solve/transcribe` returns HTTP 200 with `idempotent: true` and the existing active steps rather than duplicating unconfirmed `e-N` chains. Uploading a *different* derivation while active transcribed steps already exist requires `replace: true` (which supersedes the prior transcribed steps) and otherwise returns `409 Conflict`.
4. **Decision — Subagent model mapping:**
   - The prompt requested "Gemini 4 Neon for implementation subagents and Gemini 4 Carbon for hard independent audits." Because the `invoke_subagent` tool in this environment exposes `inherit | flash_lite | flash | pro`, `inherit` was used for implementation subagents and `pro` was used for all three hard independent audits (`research` read-only subagent).
5. **Assumption — `POST .../solve/transcribe` body format:**
   - Accepts JSON `{ tex, file_name?, replace? }` (with aliases `content` / `source_tex` and `filename`) and also rejects raw binary `%PDF-` request bodies directly with `PDF_REJECT_MESSAGE` (`400`). Multipart upload is not added because the desktop API consistently uses JSON endpoints.

---

## 4. Merge Notes (Cross-Brief Integration Seams)

| Target file | Symbol / location | What the sibling brief delivers / wires |
|---|---|---|
| `crates/desktop-server/assets/new_menu.js` | `MODE_MENU_ENTRIES[solve].built`, `enableSolveButton` | **`solve-s12-loop`**: when flipping `DESKTOP_MODE_SOLVE_ENABLED = true` in `session_mode.rs`, flip `built: true` in `new_menu.js` (or pass `{ solveBuilt: true }` to `buildModeMenuEntries`) and update the 3 pre-S12 menu tests together. |
| `crates/desktop-server/assets/mode_stages.js` | Solve stage mount | **`solve-s12-loop` / shell wiring**: import `createSolveWorkspace` + `registerSolveTab` from `./solve_steps.js`, `SOLVE_TRANSCRIBE_TAB` from `./solve_transcribe.js`, and `SOLVE_ARTIFACTS_TAB` from `./solve_artifacts.js`; register both tabs and mount `createSolveWorkspace` in the Solve stage passing `sessionId`, `token`, `fetch`, `katex`, and route callbacks (`/solve/ack`, `/solve/attach-source`, `/solve/check`, `/solve/recheck`, `/solve/transcribe`, `/solve/continue`). |
| `crates/desktop-server/assets/solve_why.js` | `reviewEnabled`, `onReviewModel` | **`solve-s14-s20-review-blind`**: pass `reviewEnabled: true` and an `onReviewModel(step)` handler into `createSolveWorkspace` to enable `"Review with another model"`. |
| `crates/desktop-server/assets/solve_why.js` | `onStepCorrected` | **`F16b`**: `createSolveWorkspace` forwards `onStepCorrected({ sessionId, session, entryId, entrySha256, step, correctionText, text, originalSummary })` when the user submits `"Correct this step"`. |
| `crates/research-tools/src/solve_steps.rs` & `solve_done_criteria.rs` | `step_record`, `DoneCriterionKind::UserConfirms`, `ClosureTierAtLeast` | **S12 / S17 solver-restatement lifecycle seam (see §6 below)**: `POST .../solve/transcribe` creates `origin: "given"`, `entry_type: "derivation"`, `claims: []`, `transcription: { source_tex_sha256 }` steps (`Unchecked`) and returns `given_source: { source_id, file_sha256 }`. When the S12 solver loop restates transcribed equations as handles in `check_derivation` mode, S12 needs either (a) `step_record` to accept/propagate `transcription` metadata onto handle-bearing steps that supersede the raw split steps (so `active_closure` in `solve_done_criteria.rs:154-173` still enforces `UserConfirms` while allowing `ClosureTierAtLeast` to reach `Verified`), or (b) `solve.setup.given_source` wiring from `handle_transcribe`'s returned `given_source`. |
| Small edits to existing files | `crates/desktop-server/src/lib.rs`, `src/assets.rs`, `src/solve.rs`, `assets/solve_state.js`, `assets/new_menu.js`, `assets/solve.css`, `crates/research-tools/src/lib.rs` | All kept minimal, local, and non-breaking. |

---

## 5. Verification Commands & Exact Results

Executed before the final push as non-root (`uid=1001`) with `RE_SEARCH_PYTHON` unset:

```bash
id
# uid=1001(antigravity) gid=1001(antigravity) ...

python3 scripts/check_file_length.py --base origin/gemini/work/fixed
# No touched source file is over 600 lines. (exit 0)

for f in crates/desktop-server/assets/*.js; do node --check "$f"; done
# exit 0 (all JS assets parse cleanly)

node --test crates/desktop-server/tests/*.test.mjs
# tests 235 | pass 235 | fail 0 | skipped 0 (exit 0)

unset RE_SEARCH_PYTHON
cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or
# exit 0 (0 warnings)

cargo test --workspace --locked --no-fail-fast
# 152 test suites | 1202 passed | 0 failed | 2 ignored (pre-existing) (exit 0)
```

- **Golden tests untouched and passing:** `tool_def_golden` (3/3), `tool_profile_golden` (3/3).
- **Pre-existing ignored tests:** 2 ignored in upstream workspace (0 new `#[ignore]` or skipped tests added).
- **Pre-existing failures observed:** none (non-root execution avoided the root-only `paper_read` permission failure noted in `RULES.md`).

---

## 6. Limitations & Unfixed Issues Noticed Outside Scope

1. **Not compiled or run on Linux:**
   - Windows-only `cfg(windows)` code (AppContainer sandbox tests in `crates/worker/tests/appcontainer_sandbox.rs`, WebView2 launcher/installer paths, Windows CI pipeline).
   - Live headless browser Playwright tests (`*_browser.py`) were not run; Node tests validate DOM rendering, events, R5/R15/R19/R22 invariants, and XSS safety against the project's fake-DOM harness.
2. **Pre-existing / cross-brief seams noticed (not edited because out of brief scope):**
   - `crates/desktop-server/src/solve_view.rs:336-339`: maps `Some(StepOrigin::Setup | StepOrigin::Given | StepOrigin::User)` to graph edge `basis: "code"`, which causes `origin: "given"` `Unchecked` transcribed derivation steps to get solid `"code"` edges in the graph projection even before any check runs.
   - `crates/research-tools/src/solve_steps.rs:134-158` & `crates/research-tools/src/solve_defs.rs:186` & `crates/research-tools/src/solve_done_criteria.rs:154-173`: `step_record` currently does not accept a `transcription` field (`additionalProperties: false`). In a full S12 `check_derivation` run, raw split steps (`claims: []`) are permanently `Tier::Unchecked`; superseding them with handle-bearing steps drops them from `active_closure` in `UserConfirms`, whereas keeping them as ancestors caps `ClosureTierAtLeast` at `Unchecked`. Resolving this requires coordinating `step_record` / `solve_done_criteria` in S12 (outside S13/S17 owned files).
   - `crates/research-tools/src/investigation_outline_tex.rs:328-331`: `MATH_BEGIN_RE` matches `\\begin{equation}` (preceded by an odd number of backslashes) and scans inside `\begin{verbatim}` blocks. Worked around cleanly in `solve_derivation_scan.rs` (`mask_skipped_envs` + `mask_escaped_begins_for_outline`) while leaving `investigation_outline_tex.rs` untouched as required.
