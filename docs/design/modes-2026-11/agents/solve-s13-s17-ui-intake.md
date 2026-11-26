# Brief solve-s13-s17-ui-intake — Solve Derivation UI (S13) & TeX Intake (S17)

**Status:** Queued right behind **`S12`** (`solve-s12-loop`, with `S4/S5/S8/S9/F6` on `gemini/work/fixed`). Zero `Cargo.lock` edits!
**Push branch:** `gemini/modes/solve-s13-s17-ui-intake`

---

## 1. Package & Governing Design

- **Packages:** **S13** (`solve_badges.js`, `solve_steps.js`, `solve_why.js`, `solve_exprs.js` built directly on F6 `graph_view.js` + `run_bar.js` + `ui_badge.js` + `quote_chip.js`, exposing `registerSolveTab` for `S18` and `onStepCorrected` for `F16b`!) -> **S17** (`research-tools::solve_derivation.rs` TeX-only check-a-derivation split reusing `investigation_outline_tex.rs` read-only + `POST .../solve/transcribe` / `solve/ack { kind: "transcription" }` + `assets/solve_transcribe.js` two-column card).
- **Governing design:** `00-overview.md` 0.1, 2.6, 2.10, 2.12, 3.2 (rows 20, 24, 25), 3.3, 4.2, 7.3 (R22), 8.1–8.3; `solve.md` 2, 3, 4.1, 4.4, 5.1, 5.5, 6 (R5, R15, R17, R19, R22), 7.2–7.3, 9 (`S13`, `S17`), 13 (D1, D10, D11, D13); `agents/RULES.md`.

---

## 2. Verified Line Counts & Constraints on `gemini/work/fixed`

- **DO NOT touch `canvas_dag.js` (`688`L) or `hitl_ui.test.mjs` (`868`L) or `app.js` (`932`L)!** Mount F6's `graph_view.js` (`539`L) directly!
- **`assets.rs` (`546`L):** Adding 5 new JS modules pushes `assets.rs` near `600`L! First split KaTeX font arms into `crates/desktop-server/src/assets_fonts.rs` (zero behaviour change, separate commit) so `assets.rs` drops `< 450`L!
- **S2 wording check (`solve_badge.rs` + `solve_state.js`):** Note `"Numerically checked"` (2026-11-26 clarification in `solve_badge.rs` and `solve_state.js`) — align badge labels directly with `solve_view.rs` / `solve_badge.rs`. Also re-sync `solve_setup.js` (`596`L — split tab registry into `solve_steps.js`/`solve_state.js` instead of bloating `solve_setup.js`).
- **S17 TeX-only:** PDF derivations stay out of scope (`00-overview.md` 2.12); reuse `investigation_outline_tex.rs` read-only (`<= 534`L).

---

## 3. Checkpoints — Push After Each!

1. **Checkpoint S13a (`assets_fonts.rs` split in `assets.rs` + `solve_badges.js`):** Map all S2 `StepBadge` / `StepTier` JSON onto `ui_badge.js` + `graph_view.js` styles, preserving server-formatted `"rests on ..."` flags verbatim and exact `"Verified"` legend text.
2. **Checkpoint S13b (`solve_steps.js`, `solve_why.js`, `solve_exprs.js`, `solve_ui.test.mjs`):**
   - `solve_steps.js`: topological step cards (`solve.expr.latex` via `math_katex.js` ONLY, R5), Result sheet (`Done` vs `Done with open items` strictly from `solve.done.status`, R15), exposes `registerSolveTab({ id, label, mount })` for `S18a` Artifacts + F6 `run_bar.js` + F6 `graph_view.js` (`"closure of the final answer"` filter), and enables `"Solve"` in `new_menu.js`.
   - `solve_why.js`: Why popover with checks, code toggle, lowering ancestors, all 7 actions (`Fork`, `Attach source`, `Ack`, `Add check`, `Re-run checks`, `Correct this step` firing `onStepCorrected` for `F16b`, and `"Review with another model"` gracefully disabled until S14 merges).
   - `solve_exprs.js`: Expressions handle table (`name@version`, LaTeX, `str_preview`, `free_symbols`, `portable` flag).
   - Node tests (`solve_ui.test.mjs`): R15, R22 wording sweep across all tiers, XSS safety (zero `innerHTML`).
3. **Checkpoint S17a (`crates/research-tools/src/solve_derivation.rs` + `solve_intake.rs`):** Deterministic TeX split (`< 1`s, 0 model calls), `ingest_bytes`, consecutive equation pairs, `solve.ack { kind: "transcription" }` for `[Confirm]` / `[Confirm all]` (D13). Put HTTP handler in sibling `solve_intake.rs` (+ small arm in `solve.rs`, keeping `<= 600`L).
4. **Checkpoint S17b (`assets/solve_transcribe.js` + `tests/solve_transcribe.test.mjs`):** Side-by-side `"your TeX"` vs `"as checked"`, `"transcription unconfirmed"` flag cleared only after confirmed `entry_sha256`.

---

## 4. Owned Files & Do-Not-Edit (`<= 600` Lines Each)

- **New:** `crates/desktop-server/src/assets_fonts.rs`, `crates/desktop-server/src/solve_intake.rs`, `crates/research-tools/src/solve_derivation.rs` (+ `_tests.rs`), `crates/desktop-server/assets/{solve_badges,solve_steps,solve_why,solve_exprs,solve_transcribe}.js`, `crates/desktop-server/tests/{solve_ui,solve_transcribe}.test.mjs`, `tests/fixtures/solve_view_fixture.json`.
- **Small edits:** `crates/desktop-server/src/{lib,assets,solve}.rs`, `crates/desktop-server/assets/{solve_state,new_menu}.js`, `solve.css`, `crates/research-tools/src/lib.rs`.
- **Do NOT edit:** `canvas_dag.js`, `hitl_ui.test.mjs`, `app.js`, `http_srv.rs`, `solve_setup.js` (`596`L), `investigation_outline_tex.rs`, `tool_defs.txt`, `tool_profiles.txt`, `Cargo.lock`.

---

## 5. Mandatory Verification Before Every Push (`RULES.md`)

```bash
python3 scripts/check_file_length.py --base origin/gemini/work/fixed
for f in crates/desktop-server/assets/*.js; do node --check "$f"; done
node --test crates/desktop-server/tests/*.test.mjs
unset RE_SEARCH_PYTHON
cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or
cargo test --workspace --locked --no-fail-fast
```
Write `docs/design/modes-2026-11/reports/solve-s13-s17-ui-intake.md` and push to `gemini/modes/solve-s13-s17-ui-intake`. End with `PUSHED`.
