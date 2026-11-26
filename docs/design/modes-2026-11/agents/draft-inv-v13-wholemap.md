# Brief draft-inv-v13-wholemap — Investigate Focused and Whole-Paper Graph Views (V13)

## 1. Packages and governing design sections

- **Package:** **V13** (Focused and whole-paper graph views: thin adapter from Investigate's `mapview` JSON to F6's shared `graph_view.js` / `graph_layout.js`, supporting paper-scale overview, focused single-node neighborhood view with Statement uses / Proof uses / Used by + breadcrumb, full-map toggle `M`, statement/proof weight filters, evidence solid/dashed styling, skip-plan dimming, section collapse / >150-node folding, and V7 main-result & novelty toggles).
- **Governing design:**
  - `00-overview.md`: 2.6 (F6 `GraphView` shape, fixed `src -> dst` prerequisite-to-dependent direction, solid vs dashed rule, line weight for statement vs proof uses, >150-node folding, icons+text badges), 2.10 (evidence grade scale), 3.2 (rows 20, 24, 25), 6 (week 7: `V13` whole map without novelty toggles in parallel with V6; V7 week 8 adds Orient/novelty data), 8.2 (#4 dashed always means not checked by code).
  - `investigate.md`: 2 (worked example 8:00), 3 ("Follow the argument", stage J Whole map, stage K Skip plan, provenance/line-style rules), 4.1 (`SkEdge` direction & roles), 4.3 (skip plan, derived mapview), rule 6.14 (skip plan honesty), sections 7 & 9 (row V13), section 12 ("Large papers and books").
- **Owner decisions (2026-11-25) applied:**
  - **Dashed always means not checked by code** (`00-overview.md` 8.2 #4, 3.2 row 25). Statement uses vs proof uses are **both solid** when found by code (`basis: "code"` or full-text `"evidence"`), distinguished by stroke weight (heavy vs light) and interactive filter toggles — never by dashes.
  - **Zero `"verified"`** anywhere in legends, tooltips, filters, or badges (`00-overview.md` 3.2 row 20).
  - **Zero model calls** in V13.

## 2. Start conditions

- **Two-checkpoint start (enables parallel execution with V6 and V7, matching `00-overview.md` 6 week 7):**
  - **Checkpoint 1 (Base focused & whole-paper graph views):** starts as soon as **V5** (`inv-a-v2-v5`, which includes V2/V3/V5) merges. **F6** is already merged on base `832eab5bf`! Can run completely in parallel with V6 and V7.
  - **Checkpoint 2 (Main-result & novelty filters):** either implemented defensively in Checkpoint 1 over optional V7 fields in `mapview` JSON (gracefully hidden/inactive until Orient or pins exist) and verified with fixture JSON, or finalized right after **V7** merges.
- **Owner gates:** None.

## 3. Base-commit verified interfaces vs future-merge interfaces

### Verified on base `832eab5bf`
- **F6 (`crates/desktop-server/assets/graph_layout.js`, 557 lines):** `DEFAULT_FOLD_THRESHOLD` (`150`), `edgeSrc`, `edgeDst`, `computeTopology`, `projectVisibleGraph`, `collectUpstreamDependencies`, `filterGraphView`, `foldGraphView`, `assignLayers`, `buildLanes`, `layoutGraphView`.
- **F6 (`crates/desktop-server/assets/graph_view.js`, 539 lines):** `EDGE_DASH_ARRAY` (`'6 4'`), `EDGE_WEIGHTS` (`{ statement: 2.8, proof: 1.4, prose: 1.8, default: 1.9 }`), `GRAPH_VIEW_LABELS`, `isEdgeDashed`, `edgeStrokeWidth`, `isNodeDashed`, `createGraphView(host, graphViewJson, options)`. Already supports section lanes (`group`), folding above 150 nodes with expand/refold, pan/zoom, statement/proof/model/dim toggles, stale muting, and zero `innerHTML`.
- **F6 (`crates/desktop-server/assets/ui_badge.js`, 387 lines):** `createBadge`, `describeEvidenceGrade`, `EVIDENCE_GRADES`.
- **F6 fixture (`crates/desktop-server/tests/fixtures/investigate_graph_view.json`, 201 lines):** self-authored 9-node / 4-lane Investigate `GraphView` fixture ready for immediate unit testing.
- **Asset server (`crates/desktop-server/src/assets.rs`, 365 lines):** all five F6 JS files already registered at `/assets/*.js`.

### Future-merge interfaces to verify before coding (not yet in `832eab5bf`)
- **V3 & V5 (`inv-a-v2-v5`):** exact `GET .../investigation/mapview` JSON fields (node list, edge roles/bases, section `group` ids, `skip_plan` dimmed section/node ids & basis string, `investigation_shell.css`, and keyboard shortcut wiring for `M`).
- **V7 (`draft-inv-v7-v8-v10`, optional at Checkpoint 1):** optional fields on `mapview` nodes for `role` (`main_result`, etc.), `pinned`, and `authors_claim.label` (`new`, `restated`, `background`, `not_stated`).

## 4. Smallest implementation checkpoints (push after each)

Run full checks (`check_file_length.py`, `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done`, `node --test crates/desktop-server/tests/*.test.mjs`, `cargo clippy`, `cargo test`), commit, and push to `gemini/modes/inv-v13-wholemap` after each checkpoint (merging `origin/gemini/work/fixed` before Checkpoint 2 if V7 lands meanwhile):

1. **Checkpoint 1 — Core whole-paper & focused graph adapter (after V5 + F6; parallel with V6/V7):**
   - Create `crates/desktop-server/assets/investigation_wholemap.js` (thin adapter over `graph_view.js` & `graph_layout.js`; never duplicate layering, lane placement, SVG drawing, or >150-node folding) and its standalone stylesheet `crates/desktop-server/assets/investigation_wholemap.css`.
   - **Three coordinated view modes** ("Follow the argument", `investigate.md` 3):
     1. **Paper-scale results view** (default graph view): shows main results (once Orient/pins exist) and their direct relationships, or falls back cleanly to source-built theorem-like result candidates when no main results exist yet.
     2. **Focused node view**: selecting any result focuses the graph on that node and its immediate neighbours along three explicit directions — incoming **Statement uses**, incoming **Proof uses**, and outgoing **Used by** — with step-by-step navigation along either direction and a breadcrumb trail that restores prior selections. Keeps a one-click `"Whole map (M)"` control visible at all times.
     3. **Whole-paper map (`M` shortcut):** all visible theorem-like nodes in section lanes (`group`), heavy solid arrows (`statement`), light solid arrows (`proof`), dashed arrows (`basis === "model"` or heuristic/unchecked), skip-plan dimming (`dim: true` ONLY when at least one main result exists, rule 6.14), and section collapse / automatic folding above `DEFAULT_FOLD_THRESHOLD` (150 nodes).
   - **Filters & honesty:** statement-uses toggle, proof-uses toggle, model-links toggle, hide-dimmed toggle (showing the skip-plan basis line: `"by explicit cross-references"` or `"includes model-inferred links"`), section collapse/expand, and heuristic `text_mention` edge styling (marked heuristic/dashed and excluded from confirmed skip dimming).
   - Zero `innerHTML`; every label/title/badge written via `textContent`.
2. **Checkpoint 2 — V7 main-result & authors'-novelty filter controls (optional fields ready in CP1, verified against V7 once merged):**
   - Add `"Main results only"` toggle (enabled once any Orient or pinned `main_result` exists) and `"Authors' label"` filter chips (`New`, `Restated`, `Background`, `Not stated`, all clearly labelled as authors' claims `"(model)"`).
   - When V7 data is absent, hide or disable novelty filter controls cleanly with no errors and keep nothing dimmed.

## 5. Owned files (all <= 600 lines)

- **New UI & test files:**
  - `crates/desktop-server/assets/investigation_wholemap.js` (if needed, split focused-subgraph/filter pure helpers into `crates/desktop-server/assets/investigation_wholemap_model.js` so each file stays well under 600 lines)
  - `crates/desktop-server/assets/investigation_wholemap.css` (unconditional standalone stylesheet — zero overlap with V3's `investigation_shell.css`)
  - `crates/desktop-server/tests/investigation_wholemap.test.mjs`
- **Small registration edit:**
  - `crates/desktop-server/src/assets.rs` (365 lines): register `/assets/investigation_wholemap.js` (and `_model.js` if split) and `/assets/investigation_wholemap.css`.

## 6. Shared registration seams and "Do not edit" files

- **Registration seams:**
  - `crates/desktop-server/src/assets.rs`: 1 match arm per new JS/CSS file (`/assets/investigation_wholemap.js`, `/assets/investigation_wholemap.css`).
  - V3 Investigate shell mount seam: mount `createInvestigateWholeMap(host, callbacks)` in the centre pane's `"Map"` tab and bind keyboard key `M` (ignoring inputs/textareas) to switch to the whole map; clicking a node invokes `onSelectNode(tag)` to open/sync the node card.
- **Do NOT edit:**
  - `crates/desktop-server/assets/investigation_shell.css` (owned by V3 — all V13 styles live in `investigation_wholemap.css`).
  - `crates/desktop-server/assets/graph_layout.js` (557 lines — F6 already covers layout/folding), `graph_view.js` (539 lines — use its options/callbacks; only open a tiny PR note if an extension hook is missing).
  - `crates/desktop-server/assets/canvas_dag.js` (688 lines — over limit, blocked by 868-line `hitl_ui.test.mjs`).
  - `crates/desktop-server/assets/investigation_graph.js` (235 lines — serves legacy map), `investigation_map.js` (594), `investigation.js` (573), `index.html` (768), `app.js` (932).
  - Any Rust backend file other than `assets.rs`; `Cargo.lock`; golden tool files.

## 7. Tests through the entry point

- **Node UI tests (`node --test crates/desktop-server/tests/investigation_wholemap.test.mjs`):**
  - Load `investigate_graph_view.json` plus a self-authored 2-section / 160-node synthetic `mapview` fixture (zero third-party text).
  - **Focused view & "Follow the argument" (`investigate.md` 3):** selecting a main result exposes its incoming **Statement uses**, incoming **Proof uses**, and outgoing **Used by** neighbours; stepping to a neighbour and clicking the breadcrumb restores the exact previous selection.
  - **Whole map & >150-node folding:** pressing `M` opens whole-paper view in section lanes; >150 nodes fold to one box per section via F6 `foldGraphView` without merging solid (`code`) and dashed (`model`) edges; expanding/refolding works interactively.
  - **Line style & weight honesty (`00-overview.md` 8.2 #4):** `statement` code edges are solid heavy (`2.8`), `proof` code edges are solid light (`1.4`), `model` / heuristic edges are dashed (`6 4`); toggling statement/proof/model updates visibility accurately.
  - **Skip-plan honesty (rule 6.14):** when `main_results` is empty, **zero** nodes are dimmed; when a main result exists, dimmed sections show the basis line (`"by explicit cross-references"` vs `"includes model-inferred links"`).
  - **V7-absent vs V7-present:** Checkpoint 1 fixture without Orient/novelty fields renders cleanly with novelty controls hidden; fixture with V7 fields filters accurately by `main_result` and authors' label.
  - **Wording & XSS:** assert zero `"verified"` anywhere in rendered DOM/legend; inject `<img onerror>`, `<script>`, `<svg onload>` into node labels/section titles/flags and assert zero `innerHTML` writes.
- Also run `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` and full `node --test crates/desktop-server/tests/*.test.mjs`.

## 8. Document conflicts & resolutions

- **V13 build order (`00-overview.md` 6 week 7 vs `investigate.md` 9 V13 row):** `00-overview.md` schedules V13 in week 7 *before* V7 ("whole map without novelty toggles"), whereas `investigate.md` 9 mentions V7 for main-result and novelty filters. Resolution (`00-overview.md` 6 wins for parallelism): build V13 with Checkpoint 1 runnable immediately after V5 + F6 (gracefully treating V7 main-result/novelty fields as optional), and Checkpoint 2 verifying V7 main-result and novelty toggles once V7 merges.
- **Line style (`investigate.md` 0 vs `00-overview.md` 2.6 & 3.2 row 25):** Earlier proposal used dashes for proof uses; `00-overview.md` 8.2 #4 resolved dashes to ALWAYS mean unchecked/model-inferred and line weight + toggle for statement vs proof uses. F6 `graph_view.js` already enforces this; V13 must never override it.

## 9. Out-of-scope / deferred items and hand-back

- **Out of scope:** Prior-work status filters (V14, deferred); modifying `canvas_dag.js` (Solve S13), `investigation_graph.js` (legacy), or `investigation_shell.css` (V3); computing skip plans in JS (computed in Rust by V5 `skeleton/skip.rs`).
- **Hand-back (`RULES.md`):** Write `docs/design/modes-2026-11/reports/inv-v13-wholemap.md` (base commit, files & line counts, merge notes, exact test counts, pre-existing failures) and push to `gemini/modes/inv-v13-wholemap`.
