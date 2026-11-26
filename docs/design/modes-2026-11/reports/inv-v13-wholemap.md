# Report: `inv-v13-wholemap` (Package V13 — Investigate Focused and Whole-Paper Graph Views)

- **Brief:** `docs/design/modes-2026-11/agents/inv-v13-wholemap.md`
- **Base commit:** `28ac1111f99e49eab151799c629c091b7996b8c2` (`origin/gemini/work/fixed`)
- **Branch pushed:** `gemini/modes/inv-v13-wholemap`
- **Environment:** Linux x86_64 (Ubuntu 24.04), `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `node v22.23.5`, `Python 3.12.16`
- **Guard rails verified:**
  - Zero new dependencies; `Cargo.lock` untouched (`git diff` empty).
  - Zero tool definitions added or changed; `crates/research-tools/tests/golden/tool_defs.txt` untouched; `tool_def_golden` passes (`3 passed, 0 failed`).
  - Zero model calls anywhere in V13.
  - Zero edits to `crates/policy`, `crates/ledger`, `index.html`, `app.js`, `graph_layout.js`, `graph_view.js`, `investigation_shell.css`, `canvas_dag.js`, `investigation_graph.js`, `investigation_map.js`, `investigation.js`, or any Rust file other than `crates/desktop-server/src/assets.rs`.
  - Every created and touched source file is strictly <= 600 lines (`python3 scripts/check_file_length.py --base origin/gemini/work/fixed` passes; max file is 573 lines).
  - Zero `innerHTML` writes; zero occurrences of `"verified"` or `"not needed"` anywhere in rendered DOM, legends, tooltips, filters, or badges.

---

## 1. What was implemented (Package V13 — Checkpoints 1 & 2)

### New assets and tests (`crates/desktop-server/`)

| File | Lines | Commit | Responsibility & exports |
|---|---:|---|---|
| `crates/desktop-server/assets/investigation_wholemap_model.js` | 493 | `5561fb2` | Pure, DOM-free Investigate `mapview` adapter over F6 `graph_layout.js` / `graph_view.js` (never duplicates layering, lane placement, SVG drawing, >150-node folding, or Rust skip-plan closure computation). Exports named wording/decision constants (`VIEW_MODES`, `RESULT_KINDS`, `STATEMENT_FALLBACK_KINDS`, `STRUCTURAL_KINDS`, `AUTHORS_CLAIM_OPTIONS`, `WHOLEMAP_LABELS`), heuristic edge normalization (`isHeuristicEdge`, `normalizeEdge`), V7 optional field inspection (`extractAuthorsClaimLabel`, `isMainResultNode`, `inspectMapviewCapabilities`), Rule 6.14 skip-plan honesty (`enforceSkipPlanHonesty` with unnumbered `"Section: <label>"` and multi-hop `parent` chain support), visible node/edge filtering (`extractVisibleNodes`, `extractVisibleEdges`, `applyV7NodeFilters`), focused 3-direction + conditional prose neighborhood (`collectFocusedNeighborhood`), three mode projections (`projectPaperScaleResultsGraph`, `projectFocusedGraph`, `projectWholePaperGraph`), and breadcrumb helpers (`pushBreadcrumb`, `truncateBreadcrumb`, `pickDefaultFocusNode`). |
| `crates/desktop-server/assets/investigation_wholemap.js` | 566 | `5561fb2` | Thin UI adapter mounting F6 `createGraphView` and `createBadge`. Exports `createInvestigateWholeMap` (alias `createInvestigationWholeMap`) and re-exports `VIEW_MODES`, `AUTHORS_CLAIM_OPTIONS`, `WHOLEMAP_LABELS`. Supports both `(host, mapview, options)` and `(host, options)` signatures. Coordinates three views: (1) Paper-scale main results view (falling back cleanly to source-built theorem-like result candidates or definitions/equations on theorem-less physics papers), (2) Focused node view with explicit **Statement uses**, **Proof uses**, **Used by** (and conditional **Other links** for physics `prose` edges), interactive step-by-step navigation, clickable breadcrumb trail + `← Back`, and always-visible `"Whole map (M)"` control, and (3) Whole-paper map (`M` shortcut, ignoring editable targets, hidden ancestors, repeat, and modifiers) with section lanes, manual section collapse/expand, >150-node automatic section folding via F6 `foldGraphView`, skip-plan basis badge (`"by explicit cross-references"` solid vs `"by cross-references (includes model-inferred links)"` dashed), and defensive Checkpoint 2 V7 controls (`"Main results only"` + `"Authors' label (model)"` chips, cleanly hidden when V7 data is absent). |
| `crates/desktop-server/assets/investigation_wholemap.css` | 272 | `5561fb2` | Standalone `.inv-wm-*` stylesheet using `tokens.css` variables with light-theme fallbacks, zero overlap with `investigation_shell.css`, solid heavy (`2px`) vs light (`1px`) vs dashed neighbour chips, responsive single-column collapse below `900px`. |
| `crates/desktop-server/tests/investigation_wholemap.test.mjs` | 573 | `5561fb2` | 7 Node UI test suites covering `investigate_graph_view.json` and self-authored synthetic fixtures (0 third-party text): focused view & breadcrumb restore, `M` shortcut & 2-section / 160-node folding without merging `code`/`model` edges + manual section collapse, line-style & stroke-weight honesty (`2.8` statement solid, `1.4` proof solid, `6 4` dashed model & `text_mention` heuristic), Rule 6.14 skip-plan honesty, V7-absent vs V7-present filtering + reader `pin: 'not_main_result'` override, XSS (`<img>`, `<script>`, `<svg onload>`) & forbidden-word assertions across all 3 modes, and regression tests for all independent audit findings. |

### Small edit to an existing file

| File | Lines | Commit | Change |
|---|---:|---|---|
| `crates/desktop-server/src/assets.rs` | 478 (was 466) | `bf4c4ec` | Added 3 match arms in `lookup_asset` right after `"/assets/investigation_skip.js"` serving `/assets/investigation_wholemap_model.js`, `/assets/investigation_wholemap.js`, and `/assets/investigation_wholemap.css`. |

---

## 2. Independent Gemini 4 Carbon Audit & Fixes Applied

An independent read-only adversarial audit (Gemini 4 Carbon / `pro`) reviewed all V13 files against `RULES.md`, `inv-v13-wholemap.md`, `graph_layout.js`, `graph_view.js`, `investigation_mapview.rs`, and `skip.rs`. Every High and Medium finding was fixed and locked down with regression tests before commit:

1. **Stale state across `setMapview()` transitions (HIGH):** `setMapview(nextMapview, { preserveViewState = false })` now resets `wholeCollapsedGroups`, `wholeExpandedGroups`, `v7Filter.mainResultsOnly`, and `v7Filter.authorsLabels` by default (unless `preserveViewState: true`) and prunes dead node IDs from `trail`.
2. **Spurious F6 `Fold groups` banner & live F6 toolbar stats sync (HIGH):** Only populates F6 `expandedGroups` when `projected.nodes.length > foldThreshold`; clicking a folded box on a `<= 150`-node paper cleanly recreates `graphView` without leaving a stale `Fold groups` banner; hooks F6 `.gv-toggle` clicks and `.gv-search` input so `.inv-wm-stats` and section pills stay synchronized live.
3. **Physics paper `prose` edges in Focused view (HIGH):** `renderFocusPanel` now renders a conditional 4th column (`data-wm-rel="other-uses"`, `WHOLEMAP_LABELS.otherUses`) whenever incoming `prose` edges exist (`otherIncoming.length > 0`), so equation-to-equation prose links drawn in the SVG remain clickable without mislabelling them as `statement` or `proof` uses.
4. **Unnumbered section label & multi-hop `parent` dimming (MEDIUM):** `collectDimmedSectionKeys` matches both `sec.label` and `"Section: <label>"` (bridging `skip.rs:271` vs `investigation_mapview.rs:347`), and `nodeMatchesDimmedSection` walks ancestor `parent` chains with a cycle guard.
5. **`__ungrouped__` collapse pill on `<= 150`-node papers (MEDIUM):** Omitted as a manual-collapse pill when `<= foldThreshold` because F6 `graph_layout.js:172` only folds `__ungrouped__` under `autoFold`.
6. **Keyboard `M` hidden-ancestor & repeat guard (MEDIUM):** `onKeyDown` now ignores `ev.repeat`, `ev.defaultPrevented`, and hidden/detached ancestors (`isHiddenInTree(root)`).
7. **`Main results only` chip behavior & `goToBreadcrumb` mode sync (MEDIUM):** `goToBreadcrumb()` / `stepBack()` now always switch back to `VIEW_MODES.FOCUSED` (firing `onModeChange`), and clicking `"Main results only"` produces an immediate visual effect from all three modes (`RESULTS`, `FOCUSED`, and `WHOLE`).

---

## 3. Deviations from the Design, Decisions Taken Alone, and Assumptions

### Deviations from design
- **Split model helper file (`investigation_wholemap_model.js`):** Used the split explicitly authorized by `inv-v13-wholemap.md` §5 so both JS modules stay comfortably under the 600-line limit (`493` and `566` lines).
- **V3 Investigate shell & `index.html` left untouched:** Per `inv-v13-wholemap.md` §§5–6, `index.html` and `investigation_v3_shell.js` were not edited; exact mount instructions are recorded in Section 4 ("Merge notes") below.

### Decisions taken alone
1. **Strict 1-box-per-section folding in `projectWholePaperGraph`:** F6's `foldGraphView` (`graph_layout.js:156`) checks `node.parent` before `node.group`. To guarantee "one box per section" above 150 nodes (and on manual section collapse) without editing `graph_layout.js`, `projectWholePaperGraph` clears `parent: null` on section-grouped nodes while preserving `originalParent: node.parent ?? null`.
2. **Recreating `graphView` on mode transitions:** F6 `graph_view.js` retains internal `collapsedGroups`/`expandedGroups` Sets across `setData` calls. Recreating `createGraphView` on mode switches (preserving F6's `getFilter()` state) prevents whole-map collapsed sections from leaking into `focused` or `results` views and triggers `fitToView()` cleanly.
3. **Dual calling signature for `createInvestigateWholeMap`:** Accepts both `(host, mapview, options)` (matching `createGraphView`) and `(host, options)` (matching the brief's `createInvestigateWholeMap(host, callbacks)` mount seam), plus `setMapview` / `setData` for live updates.
4. **Defensive V7 detection:** Detects main results from `node.pinned`/`node.pin` (reader override winning over model role per `investigate.md` §4.3), `node.role === 'main_result'`, `mapview.main_results`, `mapview.main_result_tags`, `mapview.skip_plan.main_result_tags`, and badge/flag fallbacks (`investigate_graph_view.json`); detects authors' claims from `node.authors_claim.label`, `node.authors_label`, or `Authors: ...` badges.
5. **Heuristic `text_mention` edge normalization:** Forces `basis: 'model'` on any edge with `rule === 'text_mention'` or `confidence === 'heuristic'` so F6 draws it dashed (`6 4`) and hides it under the `Model links` toggle even if an upstream payload omits or mislabels `basis`.
6. **`M` key toggle behavior:** Pressing `M` from `results` or `focused` switches to `whole` (recording `previousMode`), and pressing `M` again in `whole` toggles back to `previousMode`.

### Assumptions
- **V7 `mapview` JSON shape (not yet merged on `28ac111`):** Assumed optional node fields `role: "main_result"`, `pinned: boolean` / `pin: "main_result" | "not_main_result"`, and `authors_claim: { label: "new" | "restated" | "background" | "not_stated" }` as documented in `inv-v13-wholemap.md` §3 and `investigate.md` §4.3. If V7 introduces different field names, only `isMainResultNode` and `extractAuthorsClaimLabel` in `investigation_wholemap_model.js` need updating.
- **No JS skip-plan computation:** Assumed Rust V5 (`skeleton/skip.rs` + `build_mapview_json_with_plan`) computes skip plans; V13 only reads `node.dim` / `skip_plan` and enforces Rule 6.14 (forcing zero dimming when no main results exist).

---

## 4. Merge Notes

### Existing file touched (isolated commit `bf4c4ec`)
- `crates/desktop-server/src/assets.rs`: added 3 match arms in `lookup_asset` right after `"/assets/investigation_skip.js"`.

### Integration seams for the shell / `index.html` lane
1. **`crates/desktop-server/assets/index.html`:** link the standalone V13 stylesheet right after `investigation_shell.css`:
   ```html
   <link rel="stylesheet" href="/assets/investigation_wholemap.css">
   ```
2. **`crates/desktop-server/assets/investigation_v3_shell.js` (around lines 213–221):**
   Replace the direct `createGraphView(graphHost, graphData, { onSelect: ... })` call with `createInvestigateWholeMap` from `/assets/investigation_wholemap.js`:
   ```js
   import { createInvestigateWholeMap, VIEW_MODES } from '/assets/investigation_wholemap.js';
   // Mount in the centre pane's graph stage:
   const wm = createInvestigateWholeMap(graphHost, mv, {
     initialMode: st.wholeMap ? VIEW_MODES.WHOLE : (st.selectedTag ? VIEW_MODES.FOCUSED : VIEW_MODES.RESULTS),
     selectedId: st.selectedTag,
     onSelectNode: (tag) => selectNode(tag, true),
     onModeChange: (nextMode) => { st.wholeMap = nextMode === VIEW_MODES.WHOLE; },
   });
   ```
   Remember to call `wm.destroy()` before replacing `graphHost` on shell re-render or session switch so the `window` `M` keydown listener is cleaned up.
3. **V7 (`draft-inv-v7-v8-v10`) integration:** Once V7 populates `role`, `pinned`/`pin`, `authors_claim`, and calls `build_mapview_json_with_plan` on `GET .../investigation/mapview`, V13's `"Main results only"` toggle, `"Authors' label (model)"` chips, and skip-plan basis bar activate automatically with zero V13 code changes.

---

## 5. Test Commands Run and Exact Counts

| Command | Exit | Exact result |
|---|---:|---|
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | `0` | `No touched source file is over 600 lines.` |
| `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` | `0` | 0 syntax errors across all JS files under `crates/desktop-server/assets/` |
| `node --test crates/desktop-server/tests/investigation_wholemap.test.mjs` | `0` | **7 passed, 0 failed, 0 skipped** |
| `node --test crates/desktop-server/tests/*.test.mjs` | `0` | **174 passed, 0 failed, 0 skipped** (167 baseline + 7 new V13 tests) |
| `cargo test --locked -p research-tools --test tool_def_golden` | `0` | **3 passed, 0 failed, 0 ignored** |
| `cargo clippy --locked -p desktop-server --all-targets -- -D warnings -A clippy::manual_unwrap_or` | `0` | **0 warnings, 0 errors** in the only Rust crate touched |
| `cargo clippy --locked --workspace --all-targets -- -D warnings -A clippy::manual_unwrap_or -A clippy::match_like_matches_macro -A clippy::unnecessary_map_or -A unused_imports -A clippy::items_after_test_module` | `0` | **0 warnings, 0 errors** across entire workspace |
| `cargo test --locked --workspace --no-fail-fast` | `0` | **911 passed, 0 failed, 2 ignored** across 115 test targets (the 2 ignored tests are pre-existing live-evaluation tests in `crates/agent/tests/eval_harness.rs`) |

### Pre-existing clippy warnings (untouched files, proven on `28ac111` with `rustc 1.100.0`)
Running `cargo clippy --locked --workspace --all-targets --keep-going -- -D warnings` reports the same 6 pre-existing warnings in untouched files previously documented on base (`RULES.md`, `f6-graph-ui.md`, `inv-a-v2-v5.md`):
1. `crates/desktop-server/src/evidence.rs:744` — `clippy::manual_unwrap_or` (documented in `RULES.md`)
2. `crates/agent/tests/eval_support/manifest.rs:289` — `clippy::match_like_matches_macro`
3. `crates/agent/tests/eval_support/sheet.rs:102` — `clippy::unnecessary_map_or`
4. `crates/agent/src/guardrails.rs:335` — `clippy::items_after_test_module`
5. `crates/research-tools/tests/sandbox_and_fs.rs:6` — `unused_imports` (`TEX_AUDIT`, `is_symlink_or_junction`, Windows-only tests on Linux)
6. `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` — `unused_imports` (`BufRead`, `BufReader`, Linux build)

None of these files were touched by this brief.

---

## 6. What Could Not Be Compiled or Run on Linux / What Does Not Work Yet

- **Not compiled/run on Linux:** `#[cfg(windows)]` targets (`desktop-app` WebView2 shell, `AppContainerSandbox`, `python_worker` tests, NTFS junction tests, Windows DPAPI vault).
- **Not run in a real browser:** Headless browser / visual WebView2 verification was not run; all DOM/SVG/keyboard behavior was tested in Node's fake DOM (`node --test`).
- **Not yet live in the running Investigate tab until Merge notes are applied:** Because `index.html` and `investigation_v3_shell.js` are outside V13's owned files, `/assets/investigation_wholemap*.{js,css}` are served by `desktop-server` and verified by unit/UI tests, but are not mounted into the active desktop page until the shell lane applies Section 4.

---

## 7. Bugs Noticed in Existing Code (Not Fixed, Out of Scope)

1. **`crates/desktop-server/assets/investigation_v3_shell.js:216`:** Passes `onSelect: (id) => selectNode(id, true)` to F6 `createGraphView(...)`, whereas `crates/desktop-server/assets/graph_view.js:361` invokes `options.onSelectNode(node)`. Consequently, clicking a node in the current V3 graph does not trigger `selectNode`. Mounting V13 `createInvestigateWholeMap(..., { onSelectNode })` per Section 4 resolves this cleanly.
2. **`crates/desktop-server/assets/graph_layout.js:172`:** `resolveFoldTarget` only folds `'__ungrouped__'` nodes when `autoFold` (`nodes.length > 150`) is true, ignoring `collapsed.has('__ungrouped__')` on smaller graphs. Worked around cleanly in `investigation_wholemap.js` without editing `graph_layout.js`.

---

## 8. Contradictions or Impossibilities Found in the Design

None blocking V13. The build-order tension (`00-overview.md` §6 week 7 vs `investigate.md` §9 V13 row) and line-style rule (`00-overview.md` §8.2 #4) were already explicitly resolved in `inv-v13-wholemap.md` §8 and implemented accordingly.
