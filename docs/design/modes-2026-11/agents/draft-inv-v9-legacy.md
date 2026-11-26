# Brief draft-inv-v9-legacy — Investigate Old Sessions and Legacy Adapter (V9)

## 1. Packages and governing design sections

- **Package:** **V9** (Old sessions: read-side legacy adapter attaching old `investigation.map` and `investigation.branch` nodes onto the new skeleton under `"Earlier map"`, `"Unplaced (older map)"`, `"unknown (older map)"` edge roles, `investigation.correction` as notes, relabelled `"Rebuild earlier map"` button, and Set R mapview equivalence tests).
- **Governing design:**
  - `00-overview.md`: 2.3 (`legacy:` id prefix), 2.6 (F6 `GraphView` shape & dashed/solid rules), 2.10 (evidence scale), 3.2 row 20 (zero `"verified"` in Investigate), 5.3 (what happens to today's sessions), 7.2–7.3 (Evaluation Set R), 8.1–8.2 (accepted decisions).
  - `investigate.md`: 0.2 #6 (legacy edges must be `"unknown (older map)"`, never guessed as `"proof"`), 4.4 (unchanged legacy event kinds still read), 7 (fate of existing files), 8 (migration of existing sessions, items 4–6 and 9), 9 (row V9), 12 ("Two generations of map in old sessions"), 13 (decision 10: upgrade in place on first open; decision 11: do **not** retire old recorders here).
- **Owner decisions (2026-11-25) applied:**
  - Old sessions upgrade **in place on first open** (V3 appends a deterministic `investigation.skeleton` if source files still match their receipt hashes; otherwise opens in the old view with a clear note). **Nothing already recorded is rewritten or deleted.**
  - Old recorders (`record_paper_map`, `record_paper_summary`, `record_paper_branch`, `record_paper_conventions`), old routes (`/investigation/branches`, `/rechecks`, `/conventions`), and old prompts stay intact and working (retirement is V19, gated on one release + owner decision 11).

## 2. Start conditions

- **Required merges on `gemini/work/fixed` before coding:**
  - Already merged on base `832eab5bf`: **F0, F1, F2, F4, F6, F8, V1**.
  - Must be merged before starting V9: **V2, V3** (`inv-a-v2-v5`, for `investigation.skeleton` + `mapview` + `Span`), **V6** (`inv-v6-tasks`, for `map_fold` and `investigation/2` + `investigation/1.may_gain`), and **F5** (`f5-f7-mode-shell`, for `session_mode.rs` and `api_dispatch.rs`).
  - **Collision avoidance on `investigation_skeleton.rs` (C11):** V7 (`draft-inv-v7-v8-v10.md`) also adds a `mapview` projection hook in `crates/desktop-server/src/investigation_skeleton.rs`. Either start V9's Checkpoint 2 `mapview` wiring **after V7 merges** (keeping Checkpoint 1 pure in `investigation_legacy_view.rs`), OR keep `investigation_skeleton.rs` untouched during parallel work and deliver V9's exact 1-line `mapview` call in `reports/inv-v9-legacy.md` (or via a dedicated `GET .../investigation/legacy` hook wired after V7) so V9 and V7/V8/V10 **never** edit `investigation_skeleton.rs` concurrently.
- **Owner gates:** None (`investigate.md` 13 #10 accepted; V9 does not retire anything).

## 3. Base-commit verified interfaces vs future-merge interfaces

### Verified on base `832eab5bf`
- **F2 (`crates/research-tools/src/paper_key.rs`):** `ID_PREFIXES` already reserves `"legacy:"` (`Mode::Investigate`, `"node of an earlier map, as legacy:<seq>:<id>"`, per session).
- **F1 (`crates/research-tools/src/tool_profile.rs`):** `PROFILE_INVESTIGATION_V1`, `session_mode_from_events`, `legacy_profile_name_for_events`.
- **Existing legacy Investigation readers (`crates/research-tools/src/investigation.rs`, 591 lines):** `expanded_map(events, map_event)`, `paper_conventions(events, seq)`, `RECORD_PAPER_{MAP, SUMMARY, BRANCH, CONVENTIONS}`.
- **Existing desktop legacy view (`crates/desktop-server/src/investigation_view.rs`, 385 lines):** `view(events, running)` reading `investigation.{paper, map, summary, conventions, branch*, correction, requested, result, question_requested, answer}`.
- **Existing Set R harness (`crates/desktop-server/tests/eval_set_r.rs`, 449 lines + `eval_set_r_support/mod.rs`, 380 lines):** Already constructs real pre-F1 Research, pre-pointer `7c36a0e` Investigation, current pointer-cited Investigation, and `research_from_paper` hand-off child ledgers and verifies resume under F1.
- **V1 (`crates/research-tools/src/skeleton/model.rs`):** `PaperSkeleton`, `SkNode.spans`, `Span::contains`.
- **F6 (`crates/desktop-server/assets/ui_badge.js`):** `createBadge`, `createQuoteChip`, `createNodeCardShell`.

### Future-merge interfaces to verify before coding (not yet in `832eab5bf`)
- **V3 (`inv-a-v2-v5`):** `desktop-server/src/investigation_skeleton.rs` (first-open skeleton generation + fallback when source files no longer match receipt hashes), `GET .../investigation/mapview` response schema, and V3's new Investigate shell mount entry (`assets/investigation_node.js` / `investigation_results.js`).
- **V6 (`inv-v6-tasks`):** `research-tools/src/map_fold.rs` folded map structure and `investigation_markers.rs` shared marker list.
- **V7 (`draft-inv-v7-v8-v10`, serialize before editing `investigation_skeleton.rs`):** Orient action label and V7's `mapview` hook in `investigation_skeleton.rs`; V9 keeps `"Rebuild earlier map"` for the legacy mapping action while Orient is the default action once V7 is present.

## 4. Smallest implementation checkpoints

1. **Checkpoint 1 — Pure read-side legacy adapter (`investigation_legacy_view.rs`):**
   - Implement `adapt_legacy_investigation(events: &[Event], skeleton: Option<&PaperSkeleton>) -> LegacyMapProjection` (pure, zero ledger writes, zero model calls).
   - Read `investigation.map` (via `research_tools::investigation::expanded_map` to include legacy `investigation.branch` expansions) and standalone `investigation.branch` events:
     - Assign deterministic display id `legacy:<map_seq>:<id>` (or `legacy:<branch_seq>:<id>` for branch-only nodes), matching F2's reserved `"legacy:"` prefix.
     - Attach each legacy node to the **innermost** skeleton node whose physical `spans` contain the legacy node's first anchor `{path, start_line, end_line}` (preferring smallest span line-count when nested; deterministic tie-break by `SkNode.order`).
     - Nodes whose first anchor does not lie inside any skeleton node's spans (including pre-pointer `7c36a0e` anchors without physical spans or old PDF sessions with weak skeletons) go into `unplaced: Vec<LegacyNode>` (`"Unplaced (older map)"`).
     - Convert legacy edges with explicit role `"unknown (older map)"` (`investigate.md` 0.2 #6, 8 #5) — **never** default to `"proof"`. Keep edge direction `src` (prerequisite) -> `dst` (dependent), and mark inferred legacy edges dashed (`basis: "model"`).
     - Map `investigation.correction` events into read-only note entries attached to the corresponding legacy/skeleton node (or unplaced if unmatched).
     - Preserve legacy `investigation.summary` (overview), `investigation.conventions` (conventions panel), `investigation.question_requested`/`answer`, and `investigation.requested`/`result` activities unchanged.
     - When `skeleton` is `None` (e.g., source files missing or hash-mismatched on open), emit `fallback_old_view: true` with an honest diagnostic note so the UI falls back to the old view without breaking or rewriting anything.
2. **Checkpoint 2 — Mapview integration (serialized after V7) & thin legacy UI panel:**
   - Once V7's `investigation_skeleton.rs` edit is merged (or via a single integrator registration line if running in parallel), embed `LegacyMapProjection` into `GET .../investigation/mapview` as an additive `legacy` field (leaving non-legacy sessions with `legacy: null` and zero overhead).
   - Add `crates/desktop-server/assets/investigation_legacy.js` (thin DOM renderer, zero `innerHTML`): renders `"Earlier map"` section inside attached skeleton node cards, `"Unplaced (older map)"` group drawer/section, `"unknown (older map)"` edge badge, read-only old overview & conventions panels, and relabels legacy `"Find main results"` to `"Rebuild earlier map"` on old sessions (`investigate.md` 8 #6) without growing `investigation_map.js` (594 lines) or `investigation.js` (573 lines).

## 5. Owned files (all <= 600 lines)

- **New Rust files:**
  - `crates/desktop-server/src/investigation_legacy_view.rs` (+ `investigation_legacy_view_tests.rs` if > 450 lines)
  - `crates/desktop-server/tests/investigation_legacy_view.rs` (Set R legacy-view integration tests)
- **New UI & test files:**
  - `crates/desktop-server/assets/investigation_legacy.js`
  - `crates/desktop-server/tests/investigation_legacy.test.mjs`
- **Minimal hook edits (<= 15 lines each, only on files < 560 lines):**
  - `crates/desktop-server/src/investigation_view.rs` (385 lines on base — small hook only if legacy status payload needs a pointer to `mapview`/`legacy`, keeping existing `view()` output backward-compatible for all existing tests).
  - `crates/desktop-server/src/investigation_skeleton.rs` (**only after V7 merges**, 1 call embedding `legacy` in `mapview` JSON; otherwise leave untouched and give the integrator the exact 1-line hook in the report).

## 6. Shared registration seams and "Do not edit" files

- **Registration seams:**
  - `crates/desktop-server/src/lib.rs` (215 lines): declare `mod investigation_legacy_view;`.
  - `crates/desktop-server/src/investigation_skeleton.rs` (serialized after V7): 1-line `legacy` field embed in `mapview` JSON.
  - `crates/desktop-server/src/assets.rs` (365 lines): register `/assets/investigation_legacy.js`.
  - **V3 shell UI mount seam:** export `mountLegacyEarlierMap(nodeHost, legacyNodes)` (for the `"Earlier map"` slot inside V3's `investigation_node.js`) and `mountLegacyPanel(shellHost, legacyPayload)` (for `"Unplaced (older map)"`, read-only overview/conventions, and `"Rebuild earlier map"` in V3's Investigate shell) when `mapview.legacy` is non-null.
- **Do NOT edit:**
  - `crates/desktop-server/assets/investigation_map.js` (**594 lines** — keep untouched to serve the fallback old view only!).
  - `crates/desktop-server/assets/investigation.js` (**573 lines** — keep untouched or delegate via <= 5 lines only if strictly needed; prefer mounting from V3's new shell).
  - `crates/desktop-server/src/investigation.rs` (**597 lines**), `crates/research-tools/src/investigation.rs` (**591 lines**).
  - `crates/desktop-server/tests/eval_set_r.rs` (leave existing F1 Set R tests passing untouched; add V9 assertions in new `tests/investigation_legacy_view.rs`).
  - `crates/research-tools/tests/golden/tool_defs.txt`, `tool_profiles.txt`, `Cargo.lock`, `crates/agent/src/resume.rs`, `http_srv.rs`, `routes_session.rs`, `index.html`, `app.js`.

## 7. Tests through the entry point

- **Set R mapview & resume integration (`crates/desktop-server/tests/investigation_legacy_view.rs`):**
  - Build/reuse all four Set R session fixtures (Research, pre-pointer `7c36a0e` Investigation, current pointer-cited Investigation, hand-off child) from self-authored synthetic paper fixtures (zero third-party text):
    - Current Investigation ledger: first open appends `investigation.skeleton` (hash-verified); `mapview` attaches legacy map & branch nodes to the innermost enclosing skeleton node under `legacy:<seq>:<id>`, preserves every legacy sentence/anchor/correction, and labels every legacy edge `"unknown (older map)"`.
    - Pre-pointer `7c36a0e` Investigation ledger: unplaceable nodes land cleanly in `"Unplaced (older map)"` with zero data loss; old summary and conventions remain visible.
    - Tampered/missing source file on an old session: skeleton build declines gracefully, opens in old view with diagnostic note, and never mutates the ledger.
    - Research and hand-off child ledgers: untouched, mode stays `Research`, `eval_set_r` passes untouched.
  - Determinism: running the adapter twice yields identical JSON; zero ledger writes by `mapview`.
  - **Wording check:** zero occurrences of `"verified"` anywhere in `LegacyMapProjection` or rendered JS output; zero `"proof"` guessed for legacy edge roles.
- **Node UI tests (`node --test`):** `"Earlier map"` attachment rendering, `"Unplaced (older map)"` list, `"Rebuild earlier map"` label on old sessions, XSS escaping on legacy node text/corrections, and untouched pass of existing `investigation_map.test.mjs` and `paper_reader_ui.test.mjs`.

## 8. Document conflicts & resolutions

- **`investigate.md` 7 line counts vs base `832eab5bf`:** `investigation_view.rs` is **385 lines** (not 217), `desktop-server/src/investigation.rs` is **597 lines** (not 579), and `assets/investigation.js` is **573 lines** (not 542). Keeping all new V9 logic in `investigation_legacy_view.rs` and `investigation_legacy.js` avoids crossing the 600-line ceiling on any existing file.
- **Old-session button wording (`investigate.md` 8 #6):** Relabel `"Find main results"` -> `"Rebuild earlier map"` ONLY for legacy-map actions on old sessions; do not alter existing `investigation_map.test.mjs` assertions if they test `investigation_map.js` directly (apply the relabel inside the new Investigate shell / `investigation_legacy.js` wrapper).

## 9. Out-of-scope / deferred items and hand-back

- **Out of scope:** Retiring old recorders, old routes, or `investigation_map.js` (V19 — gated on one release + owner decision 11); rewriting old ledger events; modifying `resume.rs` or tool profiles.
- **Hand-back (`RULES.md`):** Write `docs/design/modes-2026-11/reports/inv-v9-legacy.md` and push to `gemini/modes/inv-v9-legacy`.
