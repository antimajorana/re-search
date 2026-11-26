# Brief exp-x12-x13-map-handoff — Map & verdict UI (X12) and Directions & hand-over buttons (X13)

## 1. Packages and governing design sections

- **Packages (strictly sequential):** `X12` (Map & verdict UI on F6 `graph_view.js`) -> `X13` (Directions recorder, cards & hand-over buttons on F7). Because both packages register in `routes_idea.rs`, `assets.rs`, and `idea.js`, run them **strictly in order `X12 -> X13`** inside this brief (push after `X12`, then build `X13`); never dispatch `X12` and `X13` concurrently.
- **Governing design:**
  - `docs/design/modes-2026-11/00-overview.md`: sections 0.1 (Lean cut — no `lean` calculation method; `"verified"` forbidden in Explore), 2.6 (F6 `GraphView`, `quote_chip.js`, `ui_badge.js`, `createNodeCardShell`), 2.10 (display grade scale), 3.1 row 9 (`solve_setup` uses Solve's `task_kind` and structured `target`!), 3.2 rows 20, 24, 25 (`"matched"`, `"Model judges:"`/`"(model)"`, dashed = unchecked/abstract-only), 4.1–4.3 (F7 `handoff.packet`, `handoff.sent`, read-only child status), 5.3, and 8.1–8.3 (including 8.2 decision 7 on `"New research"`).
  - `docs/design/modes-2026-11/explore.md`: sections 3 (Map and overlaps, Directions, Afterwards), 4.1 (`MapNode`, `MapEdge`, `Verdict`, `Direction`), 4.2 (Hand-overs table, `idea.directions`, `idea.note`, `handoff.sent`), 5.1 (`record_idea_directions`), 6 (rules V5, V6, V8, D1–D3, S4, S8), 9 (rows X12, X13), and 13 (decisions 6, 13).
  - `docs/design/modes-2026-11/agents/RULES.md`.

## 2. Start conditions and gates

- **Required merges on `gemini/work/fixed`:**
  - Before **X12**: `X11` (`exp-x9-x10-x11-read-verdict`) and `X4` (`exp-x3-x4-shell`). Base already has `F6` (`graph_layout.js`, `graph_view.js`, `ui_badge.js`, `quote_chip.js`).
  - Before **X13**: `X12` committed & pushed (sequential inside this brief), `X11` (`exp-x9-x10-x11-read-verdict`), and **`F7`** (`f5-f7-mode-shell`: `export/src/handoff.rs`, `desktop-server/src/handoff.rs`).
- **No owner gate blocks X12/X13** (owner accepted all section 8 recommendations on 2026-11-25, including cutting Lean and adopting Solve's `solve_setup` names).

## 3. Base-commit verified interfaces vs future-merge interfaces

### Verified on base commit `832eab5bf`
- `crates/desktop-server/assets/graph_view.js` (539 lines): `createGraphView(host, graphViewJson, { onSelectNode, onSelectEdge })`, solid/dashed basis handling, folding above 150 nodes, pan/zoom/select, stale styling, zero `innerHTML`.
- `crates/desktop-server/assets/ui_badge.js` (387 lines): `createBadge` (SVG icon + text, never colour alone), `createSplitBadge`, `createNodeCardShell` (code block top, model block bottom), `formatModelText`, `MODEL_VERDICT_PREFIX`, `MODEL_TEXT_SUFFIX`.
- `crates/desktop-server/assets/quote_chip.js` (178 lines): `createQuoteChip` (300-char clamp, `"matched"`, PDF page link, `"OCR"` tag with `onOpenPageImage`, `"abstract only"` dashed).
- `crates/desktop-server/src/assets.rs` (365 lines).
- `crates/research-tools/src/paper_key.rs`: `PaperKey`, `PaperRef`, `NodeRef` (`{ session, mode, id, paper, node_fp }`).

### Future-merge interfaces to verify before coding
- **`X11` (`idea_view.rs`, `idea_map.rs`, `idea_verdict.rs`):** exact `GraphView` JSON, per-part `Verdict` JSON, `not_in_current_map` node representation, and stale flag on idea revision.
- **`F7` (`export/src/handoff.rs`, `desktop-server/src/handoff.rs`):** exact `HandoffPacket` struct, `handoff.sent` writer, and receiving HTTP route so X13 never builds a second hand-off format.
- **`F5` (`assets/new_menu.js`) & `Solve S13` (if merged):** check whether Solve `S13` has already landed for the `"New research"` setting flip (`00-overview.md` 8.2 Decision 7).
- **`Solve` (`solve.done` event shape, if merged):** exact fields read back read-only onto a direction card (`"Opened in Solve: (state)"`); fall back gracefully via `serde_json::Value` if no `solve.done` exists yet.

## 4. Smallest implementation checkpoints

Push after `X12`, merge `origin/gemini/work/fixed`, then build and push `X13`:

1. **Checkpoint 1 — X12 (Map & verdict UI):**
   - `crates/desktop-server/assets/idea_map.js` (adapter ONLY — zero graph layout code of its own!):
     - Verdict table: one row per `subclaim_id` showing verbatim idea sentence, status chip from F12 wording (`"Model judges: same"`, `"Model judges: partly the same"`, `"Model judges: same under another name"` until user confirms -> `"found (same)"` / `"found (partly)"` / `"found under another name"`), side-by-side matched quote via `quote_chip.js` (PDF page link; `"OCR"` chip opens page image; `"abstract only"` dashed), `"what differs (model)"`, and one-click `"Looks the same to me"` (`POST /api/sessions/{id}/idea/note` with `confirm_same`) plus `"Not the same"` (`reject_same`) covering ALL THREE `judged_*` statuses (Rule V6).
     - Concept map: grouped list + F6 `createGraphView` kept in sync on node/edge selection; node drawer built with `createNodeCardShell` and F6 badges (text + icon, never colour alone); flag controls (`wrong`, `not_relevant`, `pin`, `comment`); preserved `not_in_current_map` user-noted nodes visible (Rule S4).
     - Stale banner when idea revision advances past `map.idea_rev`, offering `"Re-map against revision n"` (reuses existing `xe`/`xa` evidence at zero reader cost, decision 13).
   - Wire `POST /api/sessions/{id}/idea/note` (`confirm_same`, `reject_same`, node flags) in `routes_idea.rs`, register `/assets/idea_map.js` in `assets.rs`, and mount `idea_map.js` in `idea.js`.
   - Commit and push `X12` to `gemini/modes/exp-x12-x13-map-handoff`.
2. **Checkpoint 2 — X13 (Directions recorder, cards & hand-over buttons, strictly after X12):**
   - `crates/research-tools/src/idea_directions.rs` (new): `Direction`, `FirstCalculation`, `SolveSetup`, `Concreteness::{Concrete, Sketch}`, `record_idea_directions` (`#[serde(deny_unknown_fields)]`, `<= 5` directions):
     - Enforces **Rule D1**: rejects unknown node IDs, `missing` item without valid basis (`no_evidence_in_scope` or `open_question(q-N)`), and Rule V8 banned absolute phrases (while allowing `"first calculation"`).
     - Enforces **Rule D2**: app-computes `concreteness` (`Concrete` requires `>= 1` `known` item naming a node with full-text `xe` evidence, `first_calculation.setup >= 40` chars with resolvable `inputs`, non-empty `check`, `>= 1` risk, and non-empty `stop_if`; else `Sketch`, kept under `"Loose ideas"`).
     - `first_calculation.method` enum is `by_hand | sympy | numeric | literature` (**NO `lean`** — cut in `00-overview.md` 0.1!).
     - `solve_setup` uses Solve's exact field names (`00-overview.md` 3.1 row 9): `task_kind: calculate | prove | check_derivation`, `target: { description, form: closed_form | series | numeric | bound | proof | per_step_verdict }`, `statement`, `conventions`, `source_keys[]`.
   - `crates/desktop-server/src/idea_handoff.rs` (new):
     - Builds F7 `handoff.packet` for (a) `"Open in Solve"` from any Concrete or Loose direction (`from: NodeRef`, `target_mode: "solve"`, `statement`, `ask: solve_setup`, `context` with `known[]` `xe` `QuoteRef`s, `papers`, `check.reproduces` as suggested done criterion, `risks`/`stop_if` as user notes, direction prose in `model_text`), and (b) `"Investigate this paper"` from any paper row or map node (`from: NodeRef`, `target_mode: "investigate"`, `papers: [PaperRef]`, `context` with matched quotes, idea parts as focus).
     - Delegates creation to F7's hand-off route (zero model calls; child re-ingests bytes and re-matches every `QuoteRef` through F4 `verify_quote`; badges/verdicts never travel). Writes shared `handoff.sent` (NOT `idea.handoff`).
     - Reads child session's latest `solve.done` (or Investigate skeleton state) read-only to display `"Opened in Solve: (state)"` without ever writing back into the Explore session (`00-overview.md` 4.1 rule 5).
   - `crates/desktop-server/assets/idea_directions.js` (new): Concrete vs collapsed Loose ideas groups, fixed card layout (`Question`, `Known` quote chips, `Missing`, `First calculation` with `Check first`, `Effort`, `Risks`, `Stop if`), free text tagged `"suggestion (model)"` (Rule D3), `"Open in Solve"` and `"Investigate this paper"` buttons, and read-only child status chip.
   - Wire `directions` and `handoff` ops in `routes_idea.rs`, register `/assets/idea_directions.js` in `assets.rs`, and mount `idea_directions.js` in `idea.js`.
   - **`"New research"` visibility seam (`00-overview.md` 5.3 & 8.2 Decision 7):** once **both** Solve `S13` and Explore `X13` have merged, `"New research"` moves from `"More"` behind a setting in F5's `new_menu.js` / settings. If `S13` is NOT merged yet when `X13` lands, leave `"New research"` under `"More"` untouched and record the exact pending flip in `reports/exp-x12-x13-map-handoff.md`.

## 5. Owned files

- **Checkpoint 1 (`X12`):**
  - `crates/desktop-server/assets/idea_map.js` (new, `<= 600` lines)
  - `crates/desktop-server/tests/idea_map.test.mjs` (new, `<= 600` lines)
- **Checkpoint 2 (`X13`, strictly after `X12`):**
  - `crates/research-tools/src/idea_directions.rs` (new, `<= 600` lines; sibling `idea_directions_tests.rs` if needed)
  - `crates/desktop-server/src/idea_handoff.rs` (new, `<= 600` lines)
  - `crates/desktop-server/assets/idea_directions.js` (new, `<= 600` lines)
  - `crates/desktop-server/tests/idea_handoff.rs` (new, `<= 600` lines)
  - `crates/desktop-server/tests/idea_directions.test.mjs` (new, `<= 600` lines)

## 6. Shared registration seams and "Do not edit" list

- **Checkpoint 1 (`X12`) registration seams (1–3 lines each):**
  - `crates/desktop-server/src/routes_idea.rs`: wire `POST /api/sessions/{id}/idea/note` (`confirm_same`, `reject_same`, node flags).
  - `crates/desktop-server/src/assets.rs`: register `/assets/idea_map.js`.
  - `crates/desktop-server/assets/idea.js`: mount `idea_map.js` panel.
- **Checkpoint 2 (`X13`) registration seams (1–3 lines each, strictly after `X12`):**
  - `crates/research-tools/src/lib.rs`: `pub mod idea_directions;`.
  - `crates/desktop-server/src/lib.rs`: `pub mod idea_handoff;`.
  - `crates/desktop-server/src/routes_idea.rs`: wire `directions` and `handoff` ops.
  - `crates/desktop-server/src/assets.rs`: register `/assets/idea_directions.js`.
  - `crates/desktop-server/assets/idea.js`: mount `idea_directions.js` panel.
  - `crates/desktop-server/assets/new_menu.js` (ONLY if Solve `S13` is already merged on `gemini/work/fixed`): move `"New research"` behind its setting per `00-overview.md` 8.2 Decision 7; otherwise leave untouched and note the flip in the report.
- **Do not edit:** `graph_layout.js`, `graph_view.js`, `ui_badge.js`, `quote_chip.js` (F6), `export/src/handoff.rs`, `desktop-server/src/handoff.rs` (F7), `app.js` (932), `index.html`, `tool_profile.rs`, `tests/golden/tool_defs.txt`, `tests/golden/tool_profiles.txt`, `Cargo.lock`.

## 7. Tests through the entry point

Run `cargo test -p research-tools --lib idea_directions`, `cargo test -p desktop-server --test idea_handoff`, `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done`, `node --test crates/desktop-server/tests/*.test.mjs`, `cargo clippy --workspace --all-targets -- -D warnings`, and `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`.

Required tests (zero live network/model calls, zero third-party paper text):
- `idea_same_needs_user_confirmation` & `idea_judged_statuses_need_user_confirmation` (Rule V6, `.mjs`): all three `judged_*` statuses show `"Model judges: ..."` until `confirm_same`, then flip to `"found (...)"`.
- `idea_map.test.mjs`: list/graph sync on selection, PDF page chip & OCR page-image click, `"abstract only"` dashed style, stale banner & `"Re-map"`, `not_in_current_map` retention (S4), zero `"verified"`/`"new"`/`"novel"` (S8), XSS payload escaping (zero raw `innerHTML`).
- `idea_direction_rejects_bad_refs` (Rule D1), `idea_direction_concreteness_is_computed` (Rule D2), rejects `"lean"` as `first_calculation.method`, enforces Solve's `task_kind` and structured `target` names.
- `idea_direction_marks_suggestions` (Rule D3, `.mjs`): only node chips count as citations; free text carries `"suggestion (model)"`; Concrete vs collapsed Loose ideas groups.
- `idea_handoff.rs`: `"Open in Solve"` and `"Investigate this paper"` produce valid F7 `handoff.packet` + `handoff.sent`, cost 0 model calls, re-verify 100% of unchanged `QuoteRef`s in the child session (and drop/count tampered quotes), carry no Explore verdicts as badges (only `model_text`), and reflect child `solve.done` read-only without appending to the parent ledger.

## 8. Document conflicts and resolutions

1. **`solve_setup` field names:** Earlier `explore.md` used `solve_setup.kind = calculate | prove | check` and a plain string `target`. **Resolution:** `00-overview.md` 3.1 row 9 (reconciled in `explore.md` 4.1) mandates Solve's `task_kind = calculate | prove | check_derivation` and structured `target: { description, form }`.
2. **`lean` method in `FirstCalculation`:** Earlier `explore.md` included `lean` in `first_calculation.method`. **Resolution:** `00-overview.md` 0.1 & 8.1 cut Lean; `method` is strictly `by_hand | sympy | numeric | literature`.
3. **Hand-off event name:** Earlier `explore.md` wrote `idea.handoff`. **Resolution:** `00-overview.md` 4.1 & 4.3 (owner decision 10) mandate shared `handoff.sent` and `handoff.packet`.
4. **Confirmation click scope:** Earlier `explore.md` decision 6 named only `found_same`. **Resolution:** `00-overview.md` 2.9 and `explore.md` Rule V6 extend `"Model judges: ..."` + `confirm_same` to ALL THREE `judged_*` statuses (`judged_same`, `judged_partial`, `judged_other_name`).

## 9. Out of scope and hand-back

- **Out of scope:** F6 layout/rendering changes, F7 core packet/receiving route, F15 cited summary (`writeup_idea.rs`, Group 5), F16b `"Remember this?"` lesson creation UI (Group 4 — leave clean `idea.note` hook), Conversation turns (`X15`), citation expansion (`X16`).
- **Hand-back:** Write `docs/design/modes-2026-11/reports/exp-x12-x13-map-handoff.md` (including whether `"New research"` moved behind a setting or still waits on Solve `S13`), commit with `Co-Authored-By`, and push after each checkpoint to `gemini/modes/exp-x12-x13-map-handoff`.
