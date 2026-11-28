# Brief draft-solve-s13-s17-ui-intake — Solve derivation UI (S13) & TeX intake (S17)

**Draft status:** Planning brief (do not launch until start conditions below are merged; S13 UI shell may prototype against fixture JSON after S5 + F6, while live actions and S17 require S8/S9/S12).
**Packages:** S13 (UI: derivation cards, badges, Why popover, Expressions tab, Graph/Run-bar/Quote-chip adapters on F6) -> S17 (check-a-derivation TeX intake & side-by-side confirmation).
**Governing design:** `00-overview.md` 0.1, 2.6 (F6), 2.10 (evidence scale), 2.12 (PDF/OCR quote chips, TeX-only derivation), 2.14 (F16b hook only), 3.2 (rows 20, 24, 25), 3.3 (run states), 4.2 ("Cite a paper"), 7.3 (R22), 8.1–8.3; `solve.md` 2 (worked example & check-a-derivation variant), 3, 4.1, 4.4, 5.1, 5.5, 6 (R5, R15, R17, R19, R22), 7.2–7.3, 9 (rows S13, S17), 12 (risk 9), 13 (D1, D10, D11, D13); `agents/RULES.md`.
**Read reports first:** `reports/f6-graph-ui.md`, `reports/s2-solve-model.md`, `reports/f10-library.md`, and (once merged) `reports/solve-s4-s5-setup.md`, `reports/solve-s7-s8-s9-s11.md`, `reports/solve-s12-loop.md`, `reports/f5-f7-mode-shell.md`.

## 1. Start conditions & owner gates

- **Required merges on `origin/gemini/work/fixed` before S13:**
  - Already merged on base `832eab5bf`: F3, F4, F6 (`graph_view.js`, `graph_layout.js`, `ui_badge.js`, `run_bar.js`, `quote_chip.js`), F9, F10 (`routes_library.rs`), S2 (`solve_badge.rs`).
  - **Hard merge gates for S13:** `solve-s4-s5-setup` (S4 `GET solve/view` JSON contract + S5 `solve_state.js`, `solve.css`) and `f5-f7-mode-shell` (F5 registers `/api/library` in `api_dispatch.rs`).
  - **Required for live S13 actions:** `solve-s7-s8-s9-s11` (S8 attach-source & `solve/ack`; S9 `solve/check` & `solve/recheck`) and `solve-s12-loop` (S12 `POST .../solve/continue` for Run bar & `"Correct this step"`). `"Review with another model"` binds to S14's `POST .../solve/review` and stays gracefully hidden/disabled until S14 merges!
- **Required before S17 starts:** S9 and S13 merged (push S13 checkpoint first, then `git fetch origin && git merge origin/gemini/work/fixed`).
- **Owner gates:** None beyond the accepted 2026-11-25 decisions. Note `00-overview.md` 5.3 / decision 7: `"New research"` moves behind a setting only AFTER both S13 AND Explore X13 ship (do not hide it here).

## 2. Interfaces: base-verified vs future-merge

**Verified on base `832eab5bf`:**
- **CRITICAL BASE-COMMIT DETAIL — DO NOT EDIT `canvas_dag.js` (`reports/f6-graph-ui.md` section 2):**
  - `crates/desktop-server/assets/canvas_dag.js` is **688 lines** (over 600) and was **NOT** slimmed by F6 because `crates/desktop-server/tests/hitl_ui.test.mjs` (868 lines, also over 600) loads it via `data:` URL with a hardcoded 5-import map.
  - **S13 MUST leave `canvas_dag.js` untouched** and mount F6's `crates/desktop-server/assets/graph_view.js` (`createGraphView(host, graphViewJson, handlers)`) directly — F6 explicitly supports ungrouped centered-row `GraphView` payloads for Solve!
- S2 (`solve_badge.rs:102-107`): singular/plural `"rests on 1 assumption made by the model"` / `"rests on N assumptions made by the model"` flags are formatted **server-side** in Rust; `solve_badges.js` renders the server-provided flag strings verbatim (escaped via `textContent`) rather than re-formatting them in JS!
- F6 shared UI components (all registered in `crates/desktop-server/src/assets.rs`, 365 lines): `ui_badge.js` (387), `run_bar.js` (283), `quote_chip.js` (178), `graph_view.js` (539), `math_katex.js`.
- F10 (`reports/f10-library.md` section 5 quirk): `GET /api/library` & `GET /api/library/{key}` — JS callers MUST `encodeURIComponent(key)` because `http_srv.rs:645` decodes raw `'+'` in URL paths as space!
- TeX outline scanner (`crates/research-tools/src/investigation_outline_tex.rs`, 534 lines): verified in base; S17 reuses it read-only to split a dropped `.tex` derivation into numbered equations/steps without expanding `investigation_outline_tex.rs` past 600 lines.

**Future-merge interfaces & concrete endpoint ownership (every Why-popover action accounted for):**
- `GET /api/sessions/{id}/solve/view` — **S4** (`solve_view.rs`, includes `steps`, `graph`, `handles`, `receipts`, `checks`, `artifacts`, `done`, `run_state`).
- `"Fork from here"` -> existing **`POST /api/sessions/{id}/fork`** (base).
- `"Attach a source"` -> **S8** (`POST /api/sessions/{id}/solve/attach-source` in `solve.rs`).
- `"Acknowledge assumption"` / `"Checked by you"` -> **S8** (`POST /api/sessions/{id}/solve/ack` in `solve.rs`; S17 extends with `kind: "transcription"`).
- `"Add check"` -> **S9** (`POST /api/sessions/{id}/solve/check` in `solve.rs`).
- `"Re-run checks"` -> **S9** (`POST /api/sessions/{id}/solve/recheck` in `solve.rs`).
- `"Correct this step"` & Run-bar `"Continue"` / `"Continue with a note"` -> **S12** (`POST /api/sessions/{id}/solve/continue` in `solve_turn_routes.rs`), plus S13's `onStepCorrected` callback hook for Group 4 **F16b** (`"Remember this?"`).
- `"Review with another model"` -> **S14** (`POST /api/sessions/{id}/solve/review` in `solve_workers.rs` / `solve_review_route.rs`; gracefully disabled/hidden with tooltip `"Verifier loop not available yet"` until S14 is present).

## 3. Smallest implementation checkpoints (push after each)

1. **Checkpoint S13a (`solve_badges.js` — pure Solve tier-to-UI adapter):**
   - `crates/desktop-server/assets/solve_badges.js`: maps S2 `StepBadge` / `StepTier` JSON onto F6 `ui_badge.js` (`createBadge`) and `graph_view.js` styles:
     - Every badge is SVG icon + text, never colour alone.
     - Border/grade scale (`00-overview.md` 2.10, `solve.md` 3): `Verified` / `Checked numerically` -> solid (`by_code`); `Cited` -> solid (`matched_quote_full_text` or `matched_quote_scanned_page` + `"OCR"` tag); `Model agreed` / `Assumed by model` -> dashed (`model_statement`); `Given` / `Checked by you` -> plain + `"you"` tag (`user_statement`); `Failed` / `Stale` -> text badge (`"Broken: rests on failed e-N"`).
     - Second line `"rests on ..."` + `"own check: verified"` ONLY when `own == Verified && effective != Verified` (R22).
     - Renders all server-formatted flags verbatim (never hidden; singular/plural handled by `solve_badge.rs:102-107`).
     - Exact `"Verified"` legend text (`solve.md` 3): `"the two sides are equal as exact expressions under the stated assumptions, as decided by SymPy; this does not show that the expression means what the text says"`.
2. **Checkpoint S13b (`solve_steps.js`, `solve_why.js`, `solve_exprs.js` — Derivation, Why popover, Expressions, Graph & Run bar):**
   - `solve_steps.js`: renders numbered step cards in topological order (equation via `math_katex.js` from check-process `solve.expr.latex` ONLY — R5; one-line reason; badge via `solve_badges.js`; depends-on chips; Cited step quote chip via F6 `quote_chip.js`; collapsible caret for code, receipts, and checks) plus the Result sheet (`Done` vs `Done with open items` strictly from `solve.done.status` — R15; final answer from handles; checklist with links; `"rests on"` list with one-click Acknowledge / Attach source / Review actions; solver summary suffixed `"(model)"`).
   - Exposes a clean **canvas tab registration hook** (e.g. `registerSolveTab({ id, label, mount })` in `solve_state.js` or `solve_steps.js`) so S18 mounts its `Artifacts` tab cleanly without editing S13 files!
   - `solve_why.js`: Why popover on badge click showing each check (outcome, residual/points/digits/order, assumptions used, requester, env versions, `"show code"`), lowering ancestors, and all 7 actions wired to their exact owners above (with `"Review with another model"` feature-gated on S14 availability, and `"Correct this step"` calling S12 `POST .../solve/continue` + firing `onStepCorrected({ session, entryId, text })` for F16b).
   - `solve_exprs.js`: Expressions tab handle table (`name@version`, rendered LaTeX from check process, `str_preview`, `free_symbols` + assumptions, `owner_entry`, `portable` badge/warning).
   - Mounts F6 `run_bar.js` (header) and `graph_view.js` (Graph tab, with `"closure of the final answer"` filter toggle). Zero edits to `canvas_dag.js` or `app.js`!
3. **Checkpoint S17a (`crates/research-tools/src/solve_derivation.rs` — deterministic TeX split):**
   - TeX-only check-a-derivation intake (`00-overview.md` 2.12, `solve.md` 9 S17 — PDF derivations explicitly out of scope).
   - Reuses `investigation_outline_tex.rs` (read-only) to split a dropped/pasted `.tex` derivation into ordered numbered steps (`origin: "given"`, `transcription: { source_tex_sha256 }`) in `< 1 s` with zero model calls.
   - Records source bytes via existing `ingest_bytes` (`given_source: { source_id, file_sha256 }`), pairs consecutive equations for solver handle-restatement and host pairwise checks, and extends S8's `POST .../solve/ack` with `kind: "transcription"` for per-step `[Confirm]` and sheet-level `[Confirm all]` (D13).
   - Add intake route in `crates/desktop-server/src/solve.rs` (or sibling `solve_intake.rs` if `solve.rs > 520` lines).
4. **Checkpoint S17b (`crates/desktop-server/assets/solve_transcribe.js` — two-column transcription card):**
   - Renders `"your TeX"` side-by-side with `"as checked"` (check-process handle LaTeX), permanent `"transcription unconfirmed"` flag (`Verified (transcription unconfirmed)` / `Checked numerically (transcription unconfirmed)`), per-step `[Confirm]` button, and result-sheet `[Confirm all]` button (done checklist stays open until confirmed per D13).

## 4. Owned files

- **New:**
  - S13: `crates/desktop-server/assets/solve_badges.js`, `solve_steps.js`, `solve_why.js`, `solve_exprs.js`, `crates/desktop-server/tests/fixtures/solve_view_fixture.json`, `crates/desktop-server/tests/solve_ui.test.mjs`
  - S17: `crates/research-tools/src/solve_derivation.rs`, `solve_derivation_tests.rs`, `crates/desktop-server/assets/solve_transcribe.js`, `crates/desktop-server/tests/solve_transcribe.test.mjs`
- **Small localized edits (`<= 600` lines each):**
  - `crates/desktop-server/assets/solve.css` & `solve_state.js` (from S5; stay `<= 600` lines; tab hook)
  - `crates/desktop-server/src/assets.rs` (365: +5 JS asset arms)
  - `crates/research-tools/src/lib.rs` (300: +1 `pub mod solve_derivation;`)
  - `crates/desktop-server/src/solve.rs` (S17 TeX intake + `transcription` ack extension, or sibling `solve_intake.rs`)

## 5. Shared registration seams & Do-not-edit files

- **Seams left for downstream briefs:**
  - `solve_why.js` `"Correct this step"` exposes `onStepCorrected` callback/event hook for **F16b** (Group 4) to attach `"Remember this?"`.
  - `registerSolveTab` hook in `solve_state.js`/`solve_steps.js` lets **S18** mount `solve_artifacts.js` as the `Artifacts` tab with zero edits to S13 files.
- **Do NOT edit:** `crates/desktop-server/assets/canvas_dag.js` (688 lines — DO NOT touch!), `crates/desktop-server/tests/hitl_ui.test.mjs` (868 lines), `crates/desktop-server/assets/app.js` (932 lines — document 1-line mount hook in Merge notes), `crates/desktop-server/src/http_srv.rs` (702 lines), `crates/research-tools/src/investigation_outline_tex.rs` (534 lines, reuse read-only), `tool_defs.txt`, `tool_profiles.txt`, `Cargo.lock`.

## 6. Tests through the entry point

- **Node (`for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` & `node --test crates/desktop-server/tests/*.test.mjs`):**
  - S13 view-model tests over `solve_view_fixture.json` (self-authored synthetic effective-potential fixture): every badge tier, `rests_on_line`, `own_check_line`, server-formatted singular/plural assumption flags, `"Broken: rests on failed e-6"`, OCR Cited chip, all 7 Why-popover actions (including S14-absent graceful disable for `"Review with another model"`), Expressions tab, `registerSolveTab` hook, Graph tab mounting F6 `createGraphView` with `"closure of the final answer"` filter, Run bar mounting F6 `createRunBar`.
  - **Rule R15 view-model test:** string `"Done"` without `"with open items"` is unreachable whenever any criterion is unmet or `solve.done.status != "finished"`.
  - **Rule R22 wording sweep:** walk every rendered DOM node across all tiers; `"verified"` appears ONLY on `effective == Verified` badges, `own_check_line` when `own == Verified && effective != Verified`, and verified-step counts/legend — never `"Computed"`, never `"proved"` on computed steps, never `"verified quote"`.
  - **XSS hygiene test:** `<script>`, `<img onerror>`, `<svg onload>`, `javascript:` in step summary, details, check evidence, handle name, quote text, and dropped `.tex` produce zero `innerHTML` writes, zero injected elements, and zero `on*` attributes.
  - S17 UI test (`solve_transcribe.test.mjs`): two-column `"your TeX"` vs `"as checked"`, `"transcription unconfirmed"` flag cleared only after `[Confirm]` / `[Confirm all]`, planted factor-of-2 error renders red `Failed` step.
- **Rust (`cargo test -p research-tools -p desktop-server`):**
  - S17 deterministic TeX split on self-authored multi-equation `.tex` fixture (`< 1 s`, zero model calls, stable step ids/hashes), `solve.ack { kind: "transcription" }` clearing flag only for matching `entry_sha256` (R4).
- **Guard rails:** `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`, zero network/model calls, zero third-party paper text.

## 7. Document conflicts & resolutions

1. **`solve.md` 7.2 / `00-overview.md` 2.6 (`canvas_dag.js` slimmed in F6) vs base reality (`reports/f6-graph-ui.md` section 2):** F6 did NOT slim `canvas_dag.js` (688 lines) due to `hitl_ui.test.mjs` (868 lines). **Resolution:** S13 leaves `canvas_dag.js` untouched and mounts F6's `graph_view.js` directly.
2. **`solve.md` 2 & 9 S17 (check a derivation) vs `00-overview.md` 2.12 (PDFs):** **`00-overview.md` 2.12 wins explicitly:** check-a-derivation is TeX-only in S17; PDF derivations are out of scope.
3. **`solve.md` 9 (F16b "Remember this?" after "Correct this step") vs Group 4 ownership:** S13 owns `"Correct this step"` UI (posting to S12 `POST .../solve/continue`) + `onStepCorrected` callback hook; Group 4 F16b owns `"Remember this?"` modal and `lessons.db` persistence.

## 8. Out of scope & hand-back

- **Out of scope:** Slimming `canvas_dag.js` or `app.js`, PDF derivation intake, Artifacts tab implementation (`solve_artifacts.js`, S18), F16b lessons UI/store (Group 4), F15 write-up export button implementation (Group 5), Lean/S16 (cut), Cadabra2/FORM/S22 (deferred).
- **Hand-back:** Write `docs/design/modes-2026-11/reports/solve-s13-s17-ui-intake.md` (with exact Node + Rust test counts, XSS/R22 verification, and any 1-line `app.js` mount note) and push only to `gemini/modes/solve-s13-s17-ui-intake`.
