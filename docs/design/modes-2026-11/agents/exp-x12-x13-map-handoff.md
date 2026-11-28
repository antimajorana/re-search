# Brief exp-x12-x13-map-handoff — Map & verdict UI (X12) and Directions & hand-over buttons (X13)

## 1. Packages and governing design sections

- **Packages (strictly sequential — push after X12, then build X13):** `X12` (Map & verdict UI on F6 `graph_view.js`) -> `X13` (Directions recorder, cards & hand-over buttons on F7). Both share `routes_idea.rs` / `assets.rs` / `idea.js`; never run concurrently.
- **Governing design:**
  - `docs/design/modes-2026-11/00-overview.md`: sections 0.1 (no `lean` calculation method; `"verified"` forbidden in Explore), 2.6 (F6 `GraphView`, `quote_chip.js`, `ui_badge.js`, `createNodeCardShell`), 2.10, 3.1 row 9 (`solve_setup` uses Solve's `task_kind` and structured `target`!), 3.2 rows 20, 24, 25, 4.1–4.3 (F7 `handoff.packet`, `handoff.sent`, read-only child status), 5.3, and 8.1–8.3 (decision 7 `"New research"`).
  - `docs/design/modes-2026-11/explore.md`: sections 3, 4.1 (`MapNode`, `MapEdge`, `Verdict`, `Direction`), 4.2 (`idea.directions`, `idea.note`, `handoff.sent`), 5.1 (`record_idea_directions`), 6 (Rules V5, V6, V8, D1–D3, S4, S8), 9 (rows X12, X13), and 13 (decisions 6, 13).
  - `docs/design/modes-2026-11/agents/RULES.md`.

## 2. Start conditions and verified seams

- **Required merge on `gemini/work/fixed` before starting:** **`exp-x9-x10-x11-read-verdict`** (`X11`).
- **Already merged on `gemini/work/fixed`:**
  - F5 (`new_menu.js`), F6 (`graph_view.js`, `ui_badge.js`, `quote_chip.js`), F7 (`export/src/handoff*.rs`, `desktop-server/src/handoff.rs`), `assets_fonts.rs` split (`assets.rs` ~`486`L), `X3/X4`, Solve `S12` (`solve.setup` / `solve_state.rs` / `solve_turn_routes.rs`).
- **Line-count headroom warnings on merged files (keep ALL `<= 600`L):**
  - `crates/desktop-server/src/routes_idea.rs` (~`551`L+ after X8–X11): if adding X12/X13 arms would push `routes_idea.rs` near `580`L, put the X12 `note` / X13 `directions` & `handoff` handlers in `idea_handoff.rs` / `routes_idea_map.rs` and delegate in 1–3 lines from `routes_idea.rs` (or 1-line arm in `api_dispatch.rs`).
  - `crates/desktop-server/assets/idea.js` (~`556`L+ after X8–X11): keep X12/X13 mount calls minimal (`<= 10` lines) or delegate tab mounting cleanly so `idea.js` stays `<= 600`L!
- **Check on `origin/gemini/work/fixed` before X13:** check whether Solve `S13` (`solve-s13-s17-ui-intake`) AND `X15` (`DESKTOP_MODE_EXPLORE_ENABLED`) have already merged cleanly for `"New research"` in `new_menu.js` (`00-overview.md` 8.2 Decision 7 — do not break `mode_shell.rs` or `new_menu_and_rail.test.mjs` if Solve/Explore left-rail enablement isn't complete; leave untouched and document exact flip in report if gated).

## 3. Smallest implementation checkpoints (push after each!)

1. **Checkpoint 1 — X12 (Map & verdict UI):**
   - `crates/desktop-server/assets/idea_map.js` (new, `<= 600`L; adapter ONLY — zero graph layout code):
     - Verdict table: one row per `subclaim_id` with verbatim idea sentence, status chip from F12 wording (`"Model judges: same"`, `"Model judges: partly the same"`, `"Model judges: same under another name"` until user confirms -> `"found (same)"` / `"found (partly)"` / `"found under another name"`), matched quote via `quote_chip.js` (PDF page link, `"OCR"` chip opens page image, `"abstract only"` dashed), `"what differs (model)"`, and one-click `"Looks the same to me"` (`POST /api/sessions/{id}/idea/note` `confirm_same`) + `"Not the same"` (`reject_same`) across ALL THREE `judged_*` statuses (Rule V6).
     - Concept map: grouped list + F6 `createGraphView` synchronized on selection; node drawer via `createNodeCardShell` & F6 `ui_badge.js` (icon + text, never colour alone); flag controls (`wrong`, `not_relevant`, `pin`, `comment`); `not_in_current_map` user-noted nodes visible (Rule S4); zero raw `innerHTML`.
     - Stale banner when idea revision advances past `map.idea_rev` with `"Re-map against revision n"` (zero reader cost, decision 13).
   - Wire `POST /api/sessions/{id}/idea/note` (`confirm_same`, `reject_same`, node flags), register `/assets/idea_map.js` in `assets.rs`, mount minimally in `idea.js` (keeping `<= 600`L).
   - Commit and push `X12` to `gemini/modes/exp-x12-x13-map-handoff`.

2. **Checkpoint 2 — X13 (Directions recorder, cards & hand-over buttons):**
   - `crates/research-tools/src/idea_directions.rs` (new, `<= 600`L; sibling `_tests.rs` if needed): `Direction`, `FirstCalculation`, `SolveSetup`, `Concreteness::{Concrete, Sketch}`, `record_idea_directions` (`#[serde(deny_unknown_fields)]`, `<= 5` directions):
     - Rule D1: reject unknown node IDs, `missing` without `no_evidence_in_scope` or `open_question(q-N)`, and Rule V8 banned phrases (allow `"first calculation"`).
     - Rule D2: compute `concreteness` (`Concrete` requires `>= 1` `known` node with full-text `xe` evidence, `first_calculation.setup >= 40` chars with resolvable `inputs`, non-empty `check`, `>= 1` risk, non-empty `stop_if`; else `Sketch` under `"Loose ideas"`).
     - `first_calculation.method`: strictly `by_hand | sympy | numeric | literature` (**NO `lean`**).
     - `solve_setup` uses Solve's exact field names (`00-overview.md` 3.1 row 9): `task_kind: calculate | prove | check_derivation`, `target: { description, form: closed_form | series | numeric | bound | proof | per_step_verdict }`, `statement`, `conventions`, `source_keys[]`.
   - `crates/desktop-server/src/idea_handoff.rs` (new, `<= 600`L):
     - Builds F7 `handoff.packet` for `"Open in Solve"` (Concrete or Loose direction -> `target_mode: "solve"`, `ask: solve_setup`, `xe` `QuoteRef`s, `check.reproduces` suggested criterion, prose in `model_text`) and `"Investigate this paper"` (`target_mode: "investigate"`, `papers: [PaperRef]`, matched quotes, idea parts as focus).
     - Delegates child session creation to F7's handoff route (`handoff.rs`), writes shared `handoff.sent` (`0` model calls; child re-ingests & re-verifies every `QuoteRef`; badges/verdicts never travel). Reads child `solve.done` / Investigate state strictly read-only (`"Opened in Solve: (state)"`).
   - `crates/desktop-server/assets/idea_directions.js` (new, `<= 600`L): Concrete vs collapsed Loose ideas groups, fixed card layout, free prose tagged `"suggestion (model)"` (Rule D3), `"Open in Solve"` and `"Investigate this paper"` buttons, read-only child status chip, zero `innerHTML`.
   - Wire `directions` and `handoff` ops (update `idea_flow.rs` 501 stub list if needed), register `/assets/idea_directions.js` in `assets.rs`, mount minimally in `idea.js` (`<= 600`L).

## 4. Owned files & Do-Not-Edit (`<= 600` lines each)

- **Owned (`X12` then `X13`):** `desktop-server/assets/{idea_map,idea_directions}.js`, `desktop-server/tests/{idea_map,idea_directions}.test.mjs`, `research-tools/src/idea_directions.rs`, `desktop-server/src/idea_handoff.rs`, `desktop-server/tests/idea_handoff.rs` (plus sibling `*_tests.rs` / `routes_idea_map.rs` if needed).
- **Small edits:** `research-tools/src/lib.rs`, `desktop-server/src/{lib.rs,routes_idea.rs,assets.rs}`, `desktop-server/assets/idea.js`, `desktop-server/tests/idea_flow.rs`, and `new_menu.js` only if both S13 + X15 already enabled Solve+Explore cleanly.
- **Do NOT edit:** `graph_layout.js`, `graph_view.js`, `ui_badge.js`, `quote_chip.js`, `export/src/handoff*.rs`, `desktop-server/src/handoff.rs`, `app.js`, `index.html`, `tool_profile.rs`, `tests/golden/{tool_defs,tool_profiles}.txt`, `Cargo.lock`.

## 5. Required tests & hand-back

- Zero live network/model calls, zero third-party paper text, zero `#[ignore]`. Cover `idea_same_needs_user_confirmation` & all 3 `judged_*` confirmation transitions, `idea_map.test.mjs`, `idea_direction_rejects_bad_refs` (D1), `idea_direction_concreteness_is_computed` (D2), `"lean"` rejection, `idea_direction_marks_suggestions` (D3), `idea_handoff.rs` (0 model calls, 100% `QuoteRef` re-verification, tampered quote drop, read-only child status), and Rule S8 (zero `"verified"`/`"new"`/`"novel"`).
- Verify before every push as non-root (`RE_SEARCH_PYTHON` unset): `check_file_length.py`, `node --check` + `node --test`, `cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or`, `cargo test --workspace --locked --no-fail-fast`.
- Write `docs/design/modes-2026-11/reports/exp-x12-x13-map-handoff.md` and push to `gemini/modes/exp-x12-x13-map-handoff`.
