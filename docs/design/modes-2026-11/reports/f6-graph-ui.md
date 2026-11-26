# Report: f6-graph-ui (Package F6 — Graph View and Shared UI Parts)

- **Brief:** `docs/design/modes-2026-11/agents/f6-graph-ui.md`
- **Base commit:** `e1517477cbceba0a6de6e4a143c34fe3b8b8411d` (`origin/gemini/work/fixed`)
- **Branch pushed:** `gemini/modes/f6-graph-ui`
- **Environment:** Linux (Ubuntu 24.04), `rustc 1.100.0`, `node v22.23.5`, `python3 3.12.16`

---

## 1. What was implemented (Package F6)

### New assets (`crates/desktop-server/assets/`)

| File | Lines | Purpose & exports |
|---|---:|---|
| `crates/desktop-server/assets/graph_layout.js` | 557 | Pure DOM-free layout engine over the `GraphView` JSON of overview 2.6, plus topology/projection helpers extracted from `investigation_graph.js`. Exports `DEFAULT_FOLD_THRESHOLD` (`150`), `DEFAULT_LAYOUT_METRICS`, `edgeSrc`, `edgeDst`, `computeTopology`, `projectVisibleGraph`, `collectUpstreamDependencies`, `filterGraphView`, `foldGraphView`, `assignLayers`, `buildLanes`, `layoutGraphView`. |
| `crates/desktop-server/assets/graph_view.js` | 539 | Interactive SVG/DOM `GraphView` component. Fixed top-to-bottom direction (`src` prerequisite above `dst` dependent); solid = `code` or matched full-text `evidence`, dashed (`6 4`) = `model`, `abstract_only`, or unchecked; statement uses (`2.8`) vs proof uses (`1.4`) by stroke weight and interactive toggles; folding above 150 nodes with click-to-expand and refold; pointer-drag pan, Ctrl/Meta+wheel zoom, keyboard pan, fit/reset; node and edge selection; stale colour-stripping + `Stale` badge; dim toggle; zero `innerHTML`. Exports `EDGE_DASH_ARRAY`, `EDGE_WEIGHTS`, `GRAPH_VIEW_LABELS`, `isEdgeDashed`, `edgeStrokeWidth`, `isNodeDashed`, `createGraphView`. |
| `crates/desktop-server/assets/ui_badge.js` | 387 | Shared badges (always SVG icon + text, never colour alone), evidence-grade scale (overview 2.10), Investigate provenance styles (`from_paper`, `outside_paper`, `model_background`), split half-badge row (`createSplitBadge`), model-text formatter (`formatModelText`, overview 3.2 row 24), and shared two-block node-card shell (`createNodeCardShell`: instant `data-block="code"` on top, dashed `data-block="model"` below). Exports named wording constants with decision comments (`SOLVE_EXACT_WORD`, `SOLVE_NUMERIC_WORD`, `QUOTE_MATCHED_WORD`, `OCR_TAG_WORD`, `ABSTRACT_ONLY_WORD`, `USER_TAG_WORD`, `MODEL_TEXT_SUFFIX`, `MODEL_VERDICT_PREFIX`, `MODEL_AGREED_LABEL`, `BORDER_SOLID`, `BORDER_DASHED`, `BADGE_ICONS`, `EVIDENCE_GRADES`, `PROVENANCE_STYLES`, `describeEvidenceGrade`, `createBadge`, `createSplitBadge`, `createNodeCardShell`, etc.). |
| `crates/desktop-server/assets/run_bar.js` | 283 | Unified run status bar for all seven states of overview 3.3 (`running`, `waiting`, `paused`, `interrupted`, `failed`, `done`, `done_open_items`), mode alias normalization (`partial` -> `paused`, never displays `"Partial"` as a state word), elapsed formatting (`formatElapsed`), budget formatting (`"7 of at most 24 model calls"`), Stop / Continue / Retry buttons, and open-items list. Exports `RUN_STATES`, `MODEL_CALL_SINGULAR`, `MODEL_CALL_PLURAL`, `RUN_ACTION_STOP`, `RUN_ACTION_CONTINUE`, `RUN_ACTION_RETRY`, `normalizeRunState`, `formatElapsed`, `formatModelCallBudget`, `formatRunStateLabel`, `formatRunSummary`, `renderRunBar`, `createRunBar`. |
| `crates/desktop-server/assets/quote_chip.js` | 178 | Shared quote chip: clamps quote text to at most 300 characters (`MAX_QUOTE_CHARS = 300`), shows paper label, `start_line`/`end_line`, optional PDF `page` (`p. N`), uses `"matched"` (never `"verified"`), distinguishes full-text (solid), OCR (`"matched in OCR text, p. N"` + `"OCR"` tag, solid, opens page image via `onOpenPageImage` / `openPageImage: true`, never rendered as math), and abstract-only (`"abstract only"`, dashed). Exports `MAX_QUOTE_CHARS`, `clampQuoteText`, `formatPaperLabel`, `formatLineRange`, `formatPageLabel`, `resolveQuoteBasis`, `formatQuoteLocation`, `createQuoteChip`. |

### Fixture and tests (`crates/desktop-server/tests/`)

| File | Lines | Purpose |
|---|---:|---|
| `crates/desktop-server/tests/fixtures/investigate_graph_view.json` | 201 | Self-authored small Investigate `GraphView` JSON (9 nodes across 4 section lanes, 9 edges covering `statement`/`proof`/`prose` roles and `code`/`evidence`/`model` bases, parent nesting, `stale`, `dim`, `flags`, `badges`, `legend`; zero third-party paper text; never contains the word "Verified"). Ready for V3 UI work. |
| `crates/desktop-server/tests/graph_layout_view.test.mjs` | 294 | Tests `graph_layout.js` and `graph_view.js` against the Investigate fixture, folding at 150 vs 160 nodes without merging `code`/`model` edges, cycle handling, solid/dashed and weight encoding, statement/proof/dim toggles, pointer-drag pan, zoom, selection, and interactive group expand/refold. |
| `crates/desktop-server/tests/shared_ui_parts.test.mjs` | 394 | Tests `ui_badge.js`, `run_bar.js` (all 7 states + aliases), `quote_chip.js` (300-char clamp, full-text/OCR/abstract, source-open callback), node-card shell, and feeds `<img onerror>`, `<script>`, and `<svg onload>` XSS payloads through every F6 component asserting zero `innerHTML` writes, zero injected elements, and zero `on*`/`javascript:` attributes. |

### Small edits to existing files

| File | Lines | Change |
|---|---:|---|
| `crates/desktop-server/assets/investigation_graph.js` | 235 (was 281) | Replaced inline `paperTopology`, `projectedPaperGraph`, and `resultDependencies` implementations with delegation to `./graph_layout.js`. All exports and outputs for existing tests are unchanged. |
| `crates/desktop-server/tests/investigation_map.test.mjs` | 223 | Updated 2-line `graphUrl` loader from `data:text/javascript;base64,...` to `pathToFileURL(...)` (matching `mathUrl` on line 9) so Node resolves the new `./graph_layout.js` relative import. Zero assertions changed. |
| `crates/desktop-server/tests/paper_reader_ui.test.mjs` | 152 | Updated 2-line `graphUrl` loader in `located_by_reader_label` from `data:` URL to `pathToFileURL(...)` (matching `mathUrl` on line 56). Zero assertions changed. |
| `crates/desktop-server/src/assets.rs` | 365 (was 346) | Registered `/assets/graph_layout.js`, `/assets/graph_view.js`, `/assets/ui_badge.js`, `/assets/run_bar.js`, and `/assets/quote_chip.js` in `lookup_asset` so `desktop-server` serves `./graph_layout.js` for `investigation_graph.js` at runtime and exposes all five F6 modules to future mode views. |

---

## 2. Deviations from the design and why

1. **`canvas_dag.js` left untouched (per brief item 1 fallback):**
   - `crates/desktop-server/assets/canvas_dag.js` is 688 lines (over the 600-line limit). Extracting only its layout helpers (`dependencyLayers`, 31 lines, and pure row placement, ~18 lines) removes ~49 lines, leaving it around ~640 lines (still over 600) unless pan/zoom and UI state are also extracted, which carries risk.
   - More decisively, `crates/desktop-server/tests/hitl_ui.test.mjs` (lines 104–114) loads `canvas_dag.js` as a `data:text/javascript;base64,...` module and only rewrites five hardcoded `/assets/...` import paths; adding any new import to `canvas_dag.js` would fail `hitl_ui.test.mjs`, and `hitl_ui.test.mjs` is 868 lines (over the 600-line rule, so it must not be touched).
   - Therefore, per brief item 1, `canvas_dag.js` was left untouched and extraction was performed from `investigation_graph.js` only. `graph_layout.js` and `graph_view.js` still cover `canvas_dag.js`'s ungrouped centered-row layout and pan/zoom model so Solve (S13) can mount `graph_view.js` directly.
2. **Node-card shell exported from `ui_badge.js`:**
   - Overview 2.6 and `investigate.md` section 3 mention F6's shared node-card shell, while the brief lists five JS files (`graph_layout.js`, `graph_view.js`, `ui_badge.js`, `run_bar.js`, `quote_chip.js`). Exporting `createNodeCardShell` from `ui_badge.js` keeps the exact five-file surface while delivering the shell for V3.
3. **Registered F6 JS files in `crates/desktop-server/src/assets.rs`:**
   - Because `investigation_graph.js` now imports `./graph_layout.js`, the running server would return 404 for `/assets/graph_layout.js` without updating `assets.rs`. Registering all five new assets in `assets.rs` (365 lines, under 600, separate commit) keeps the live desktop server working and unblocks V3/F5 imports.
4. **No dedicated CSS file linked in `index.html`:**
   - Per the brief, `index.html` (768 lines) was not edited. Components emit semantic class names (`.ui-badge`, `.ui-run-bar`, `.ui-quote-chip`, `.ui-node-card`, `.gv-*`) and set essential SVG attributes using existing `tokens.css` variables (`var(--accent)`, `var(--border-strong)`, etc.) so they render with theme colors and exact solid/dashed/weight styling even before mode CSS lands.

---

## 3. Decisions taken alone and assumptions

- **Relative imports between F6 assets (`./graph_layout.js`, `./ui_badge.js`):** Follows the pattern already used by `math.js`, `math_katex.js`, `math_scan.js`, and `tex_preview.js`. Works identically in browsers served under `/assets/` and in Node tests imported via `file://` URLs without string rewriting.
- **Folding edge aggregation honesty:** `foldGraphView` deduplicates lifted edges by `(src, dst, role, basis, abstract_only)` so `code`/`evidence` (solid) edges and `model`/`abstract_only` (dashed) edges are never merged into a single edge.
- **Fail-closed defaults for unknown grades/edges:** `describeEvidenceGrade` defaults unknown grades to `model` (`dashed`), and `isEdgeDashed` treats any basis other than `'code'` or full-text `'evidence'` as dashed, so missing or unexpected metadata never displays as code-checked.
- **Stale node badge tone:** Per `investigate.md` 4.2 ("lose their badge colour"), `graph_view.js` and `createNodeCardShell` force badge `tone: 'muted'` when `stale: true` and append an explicit `Stale` icon+text badge.
- **SVG icon allowlist:** `BADGE_ICONS` in `ui_badge.js` uses a frozen static map of 16x16 SVG path definitions; unknown icon names fall back to `'info'` and untrusted strings are never interpolated into SVG markup or attributes.
- **Assumptions:**
  - `GraphView` edges in Explore/Solve may carry `abstract_only: true` on `basis: "evidence"` or use `basis: "abstract_only"` / `"unchecked"`; `isEdgeDashed` handles all of those as dashed.
  - `quote_chip.js` assumes canonical F4 line fields `start_line` and `end_line` (1-based positive integers) and optional 1-based `page`.
  - Browser layout and CSS visual polish for `.ui-*` and `.gv-*` classes are assumed to be added by V3 (`investigation_shell.css`) and F5 when mounting these exports into `index.html` / `app.js`.

---

## 4. Merge notes

### Existing files touched (keep in mind when merging parallel briefs)

1. **`crates/desktop-server/assets/investigation_graph.js`** (commit `3060bc7`):
   - Top of file (`paperTopology`, `projectedPaperGraph`, `resultDependencies`) now imports from `./graph_layout.js`. No other functions (`savedFocusMap`, `mappedQuestionMatches`, `paperQuestionFocus`, `renderPaperGraph`) were modified.
2. **`crates/desktop-server/tests/investigation_map.test.mjs`** & **`crates/desktop-server/tests/paper_reader_ui.test.mjs`** (commit `3060bc7`):
   - 1-line module loader change to `pathToFileURL` for `investigation_graph.js`.
3. **`crates/desktop-server/src/assets.rs`** (commit `91df162`):
   - Added 5 arms in `lookup_asset` right after `"/assets/investigation_graph.js"`. Any parallel brief adding static assets to `assets.rs` (e.g. F5 `new_menu.js`, V3 `investigation_*.js`) will only need a trivial match-arm merge.

### Interfaces ready for downstream briefs

- **V3 (Investigate skeleton UI):**
  - Mount `createGraphView(host, mapviewJson, { onSelectNode, onSelectEdge })` from `/assets/graph_view.js`.
  - Use `createBadge`, `createSplitBadge`, `createNodeCardShell` from `/assets/ui_badge.js`, `createRunBar` from `/assets/run_bar.js`, `createQuoteChip` from `/assets/quote_chip.js`.
  - Starter fixture available at `crates/desktop-server/tests/fixtures/investigate_graph_view.json`.
- **F9 (Run budget / run state):**
  - `run_bar.js` accepts `{ state, reason, detail, extras, elapsedMs | elapsedSec, modelCallsUsed, modelCallsMax, openItems, busy }` and normalizes all mode state names from overview 3.3.
- **S13 (Solve UI) & X12 (Explore map UI):**
  - `graph_view.js` handles both grouped (`hasLanes: true`) and ungrouped (`hasLanes: false`, centered rows) `GraphView` payloads.
  - Note: if a future task slims `canvas_dag.js` under 600 lines, `hitl_ui.test.mjs` (868 lines) must first be split under 600 lines or switched off `data:` URL stubbing so `canvas_dag.js` can import `/assets/graph_layout.js`.

---

## 5. Verification and test results

### Commands run and exact results

1. **File length guard:**
   ```bash
   python3 scripts/check_file_length.py --base origin/gemini/work/fixed
   ```
   Result: exit `0` — `No touched source file is over 600 lines.`

2. **JavaScript syntax check (all 47 files in `crates/desktop-server/assets/*.js`):**
   ```bash
   for f in crates/desktop-server/assets/*.js; do node --check "$f"; done
   ```
   Result: exit `0` — 0 syntax errors.

3. **All Node UI tests:**
   ```bash
   node --test crates/desktop-server/tests/*.test.mjs
   ```
   Result: exit `0` — **157 passed, 0 failed, 0 skipped** (148 existing + 9 new F6 tests: 5 in `graph_layout_view.test.mjs`, 4 in `shared_ui_parts.test.mjs`).

4. **Golden tool-definition check:**
   ```bash
   cargo test -p research-tools --test tool_def_golden
   ```
   Result: exit `0` — **3 passed, 0 failed** (`Cargo.lock` and `crates/research-tools/tests/golden/tool_defs.txt` untouched, verified with `git diff`).

5. **Full Rust workspace tests:**
   ```bash
   cargo test --workspace --no-fail-fast
   ```
   Result: exit `0` — **663 passed, 0 failed, 2 ignored** across 97 test binaries (including all `desktop-server` integration tests).

6. **Workspace clippy (`rustc 1.100.0`):**
   - `cargo clippy --workspace --all-targets -- -D warnings` failed on **6 pre-existing lints** in untouched files (proven below on base commit `e1517477`).
   - `cargo clippy --workspace --all-targets -- -D warnings -A clippy::manual_unwrap_or -A clippy::match_like_matches_macro -A clippy::unnecessary_map_or -A unused_imports -A clippy::items_after_test_module` -> exit `0` (0 warnings).
   - `cargo clippy -p desktop-server --all-targets -- -D warnings -A clippy::manual_unwrap_or` -> exit `0` (0 warnings in the only crate touched by this brief).

### Pre-existing lints / issues proven on base commit `e1517477cbceba0a6de6e4a143c34fe3b8b8411d`

Verified via a detached worktree at `e1517477` running `cargo clippy --workspace --all-targets --keep-going -- -D warnings` on `rustc 1.100.0` (Linux):
1. `crates/desktop-server/src/evidence.rs:744` — `clippy::manual_unwrap_or` (also documented in `RULES.md`).
2. `crates/agent/tests/eval_support/manifest.rs:289` — `clippy::match_like_matches_macro`.
3. `crates/agent/tests/eval_support/sheet.rs:102` — `clippy::unnecessary_map_or`.
4. `crates/agent/src/guardrails.rs:335` — `clippy::items_after_test_module`.
5. `crates/research-tools/tests/sandbox_and_fs.rs:6` — `unused_imports` (`TEX_AUDIT`, `is_symlink_or_junction`, Windows-only test uses on Linux).
6. `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` — `unused_imports` (`BufRead`, `BufReader`, Linux build).

None of these files were touched by this brief.

---

## 6. What could not be compiled or run on Linux / What does not work yet

- **Windows-only `cfg(windows)` code** (`AppContainerSandbox`, Win32 DPAPI vault, `desktop-app` WebView2 shell) was not compiled or tested on Linux.
- **Headless browser tests (`crates/desktop-server/tests/*_browser.py`)** and live visual inspection in a browser were not run; DOM behavior was tested via Node's fake DOM in `node --test`.
- **Styling:** Because `index.html` is not edited in F6, no stylesheet is loaded for the new `.ui-badge`, `.ui-run-bar`, `.ui-quote-chip`, `.ui-node-card`, or `.gv-*` classes yet. SVG stroke/fill/dash/weight attributes render via `tokens.css` variables directly, while HTML layout polish for the toolbar/legend/cards awaits V3/F5 CSS wiring.

---

## 7. Bugs noticed (not fixed, out of scope)

- **`crates/cli/tests/desktop_and_ext.rs:149-168` (`cli_research_workspace_read_fork_diff_export_and_verify`):** When `RE_SEARCH_PYTHON` is set in the environment on Linux, the spawned `re-search run --route api` rejects `--python` on non-Windows and exits immediately before contacting `ScriptedMock`, causing `mock.finish()` to block indefinitely on `listener.accept()` (10-minute timeout). Running `cargo test --workspace` without `RE_SEARCH_PYTHON` passes this test in < 1 s.
- **Design tension noticed (`canvas_dag.js` vs `hitl_ui.test.mjs`):** `00-overview.md` 2.6 planned to drop `canvas_dag.js` (688 lines) under 600 lines in F6, but `hitl_ui.test.mjs` (868 lines, itself over 600 lines) imports `canvas_dag.js` via `data:` URL with a fixed 5-entry replacement map, blocking any new ES module import in `canvas_dag.js` until `hitl_ui.test.mjs` is split or switched to file URLs.
