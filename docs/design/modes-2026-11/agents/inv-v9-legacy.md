# Brief inv-v9-legacy — Investigate Old Sessions and Legacy Adapter (V9)

## 1. Package and governing design sections

- **Package:** **V9** (Old sessions: read-side legacy adapter attaching old `investigation.map` and `investigation.branch` nodes onto the new skeleton under `"Earlier map"`, `"Unplaced (older map)"`, `"unknown (older map)"` edge roles, `investigation.correction` as notes, relabelled `"Rebuild earlier map"` button, and Set R mapview equivalence tests).
- **Governing design:**
  - `00-overview.md`: 2.3 (`legacy:` id prefix), 2.6 (F6 `GraphView` shape & dashed/solid rules), 2.10 (evidence scale), 3.2 row 20 (zero `"verified"` in Investigate), 5.3 (what happens to today's sessions), 7.2–7.3 (Evaluation Set R), 8.1–8.2 (accepted decisions).
  - `investigate.md`: 0.2 #6 (legacy edges must be `"unknown (older map)"`, never guessed as `"proof"`), 4.4 (unchanged legacy event kinds still read), 7 (fate of existing files), 8 (migration of existing sessions, items 4–6 and 9), 9 (row V9), 12 ("Two generations of map in old sessions"), 13 (decision 10: upgrade in place on first open; decision 11: do **not** retire old recorders here).
- **Owner decisions (2026-11-25) applied:**
  - Old sessions upgrade **in place on first open** (`ensure_skeleton_on_open` in `investigation_skeleton.rs` appends a deterministic `investigation.skeleton` if source files still match their receipt hashes; otherwise opens in the old view with a clear note). **Nothing already recorded is rewritten or deleted.**
  - Old recorders (`record_paper_map`, `record_paper_summary`, `record_paper_branch`, `record_paper_conventions`), old routes (`/investigation/branches`, `/rechecks`, `/conventions`), and old prompts stay intact and working (retirement is V19, gated on one release + owner decision 11).

## 2. Start conditions & parallel file-disjointness

- **All prerequisites are now merged on `gemini/work/fixed`:** F0–F12, F13a, F14a, F5 (`session_mode.rs`, `api_dispatch.rs`), F6 (`ui_badge.js`, `quote_chip.js`), F8 (`eval_set_r.rs`), V1–V5 (`skeleton/*`, `investigation_skeleton.rs`, `investigation_mapview.rs`), V6 (`map_fold.rs`, `investigation_markers.rs`, `investigation/2`), V13, and V17.
- **Strict collision avoidance with `inv-v7-v8-v10` (running in parallel):**
  - `inv-v7-v8-v10` runs concurrently and edits `crates/desktop-server/src/investigation_skeleton.rs`, `investigation_node.js`, and `investigation_v3_shell.js`.
  - **Do NOT edit `investigation_skeleton.rs`, `investigation_mapview.rs`, `investigation_node.js`, or `investigation_v3_shell.js` in this branch.**
  - Instead:
    1. Implement the complete `LegacyMapProjection` adapter and helper `embed_legacy_into_mapview(mv: &mut Value, events: &[Event], skeleton: Option<&PaperSkeleton>)` in `crates/desktop-server/src/investigation_legacy_view.rs`, and expose a read-only `GET /api/sessions/{id}/investigation/legacy` endpoint in `investigation_legacy_view.rs` (registered via 1 line in `api_dispatch.rs` behind `AuthGuard::authorize_api`, and tested end-to-end in `tests/investigation_legacy_view.rs`), PLUS document the exact 1-line `embed_legacy_into_mapview` call for `investigation_skeleton.rs` in `reports/inv-v9-legacy.md` so the coordinator wires it cleanly on merge!
    2. Implement `crates/desktop-server/assets/investigation_legacy.js` exporting `mountLegacyEarlierMap(nodeHost, legacyNodes)` and `mountLegacyPanel(shellHost, legacyPayload)` (registered in `assets.rs` and tested thoroughly in `tests/investigation_legacy.test.mjs`), and document the exact 2-line mount call for `investigation_node.js` / `investigation_v3_shell.js` in `reports/inv-v9-legacy.md`.

## 3. Verified interfaces on `gemini/work/fixed`

- **F2 (`crates/research-tools/src/paper_key.rs`):** `ID_PREFIXES` reserves `"legacy:"` (`Mode::Investigate`, `"node of an earlier map, as legacy:<seq>:<id>"`, per session).
- **F1 & V6 (`crates/research-tools/src/tool_profile.rs`, `map_fold.rs`, `investigation_markers.rs`):** `PROFILE_INVESTIGATION_V1`, `PROFILE_INVESTIGATION_V2`, `session_mode_from_events`, `legacy_profile_name_for_events`, `fold_map`.
- **Existing legacy Investigation readers (`crates/research-tools/src/investigation.rs`, 575 lines):** `expanded_map(events, map_event)`, `paper_conventions(events, seq)`, `RECORD_PAPER_{MAP, SUMMARY, BRANCH, CONVENTIONS}`.
- **Existing desktop legacy view (`crates/desktop-server/src/investigation_view.rs`, 391 lines):** `view(events, running)` reading `investigation.{paper, map, summary, conventions, branch*, correction, requested, result, question_requested, answer}`.
- **Existing Set R harness (`crates/desktop-server/tests/eval_set_r.rs`, 449 lines + `eval_set_r_support/mod.rs`, 380 lines):** Constructs pre-F1 Research, pre-pointer `7c36a0e` Investigation, current pointer-cited Investigation, and `research_from_paper` hand-off child ledgers.
- **V1–V5 & V17 (`crates/research-tools/src/skeleton/model.rs`, `desktop-server/src/investigation_skeleton.rs`):** `PaperSkeleton`, `SkNode.spans`, `Span`, `ensure_skeleton_on_open`.

## 4. Implementation details

1. **Pure read-side legacy adapter & route (`crates/desktop-server/src/investigation_legacy_view.rs`):**
   - Implement `adapt_legacy_investigation(events: &[Event], skeleton: Option<&PaperSkeleton>) -> LegacyMapProjection` (pure, zero ledger writes, zero model calls).
   - Read `investigation.map` (via `research_tools::investigation::expanded_map` to include legacy `investigation.branch` expansions) and standalone `investigation.branch` events:
     - Assign deterministic display id `legacy:<map_seq>:<id>` (or `legacy:<branch_seq>:<id>` for branch-only nodes), matching F2's reserved `"legacy:"` prefix.
     - Attach each legacy node to the **innermost** skeleton node whose physical `spans` contain the legacy node's first anchor `{path, start_line, end_line}` (preferring smallest span line-count when nested; deterministic tie-break by `SkNode.order`).
     - Nodes whose first anchor does not lie inside any skeleton node's spans (including pre-pointer `7c36a0e` anchors without physical spans or old PDF sessions with weak skeletons) go into `unplaced: Vec<LegacyNode>` (`"Unplaced (older map)"`).
     - Convert legacy edges with explicit role `"unknown (older map)"` (`investigate.md` 0.2 #6, 8 #5) — **never** default to `"proof"`. Keep edge direction `src` (prerequisite) -> `dst` (dependent), and mark inferred legacy edges dashed (`basis: "model"`).
     - Map `investigation.correction` events into read-only note entries attached to the corresponding legacy/skeleton node (or unplaced if unmatched).
     - Preserve legacy `investigation.summary` (overview), `investigation.conventions` (conventions panel), `investigation.question_requested`/`answer`, and `investigation.requested`/`result` activities unchanged.
     - When `skeleton` is `None` (e.g., source files missing or hash-mismatched on open), emit `fallback_old_view: true` with an honest diagnostic note so the UI falls back to the old view without breaking or rewriting anything; when no legacy `investigation.map`/`branch`/`summary`/`conventions`/`correction` exist on a pure v2 session, `has_legacy` is `false` (`embed_legacy_into_mapview` sets `"legacy": null`).
   - Expose `pub fn handle_get_legacy(state: &DesktopState, session_id: &str) -> ApiResult` calling `ensure_skeleton_on_open` (which appends `investigation.skeleton` on first open if hash-verified, or returns `Ok(None)` without mutating ledger if source files changed) and returning `LegacyMapProjection` JSON.
2. **Thin legacy UI module (`crates/desktop-server/assets/investigation_legacy.js`):**
   - Pure DOM builder (`createElement` + `textContent`, zero `innerHTML`) exporting `mountLegacyEarlierMap(nodeHost, legacyNodes)` (`"Earlier map"` section inside attached skeleton node cards with `"unknown (older map)"` edge badges and correction notes) and `mountLegacyPanel(shellHost, legacyPayload)` (`"Unplaced (older map)"` drawer/section, read-only old overview & conventions panels, and relabelling legacy `"Find main results"` to `"Rebuild earlier map"` on old sessions per `investigate.md` 8 #6 without modifying `investigation_map.js`).

## 5. Owned files & guard rails (all <= 600 lines)

- **New:** `crates/desktop-server/src/investigation_legacy_view.rs` (+ `investigation_legacy_view_tests.rs` if > 450 lines), `crates/desktop-server/tests/investigation_legacy_view.rs`, `crates/desktop-server/assets/investigation_legacy.js`, `crates/desktop-server/tests/investigation_legacy.test.mjs`.
- **Small 1-line registrations:** `crates/desktop-server/src/lib.rs` (`pub mod investigation_legacy_view;`), `crates/desktop-server/src/api_dispatch.rs` (`("GET", ["api", "sessions", id, "investigation", "legacy"])` behind `AuthGuard::authorize_api`), `crates/desktop-server/src/assets.rs` (`/assets/investigation_legacy.js`).
- **Do NOT edit:** `investigation_skeleton.rs`, `investigation_mapview.rs`, `investigation_node.js`, `investigation_v3_shell.js`, `investigation_map.js` (594), `assets/investigation.js` (573), `desktop-server/src/investigation.rs` (598), `research-tools/src/investigation.rs` (575), `tests/eval_set_r.rs`, `tool_defs.txt`, `tool_profiles.txt`, `Cargo.lock`, `resume.rs`, `http_srv.rs`, `routes_session.rs`, `index.html`, `app.js`.

## 6. Required verification & hand-back

- Run `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`, `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done`, `node --test crates/desktop-server/tests/*.test.mjs`, `cargo test --locked -p research-tools --test tool_def_golden --test tool_profile_golden`, `cargo test --locked -p desktop-server --test eval_set_r --test investigation_legacy_view --test mode_markers_r2`, `cargo test --locked -p agent --test resume_loop`, `cargo test --locked -p agent --lib resume`, `cargo test --locked --workspace --no-fail-fast`, and `cargo clippy --locked -p desktop-server --lib --test investigation_legacy_view --test eval_set_r -- -D warnings -A clippy::manual_unwrap_or`.
- Cover all four Set R session shapes (Research, pre-pointer `7c36a0e` Investigation, current pointer-cited Investigation, hand-off child), tampered/missing source fallback, determinism, auth (401/403), XSS escaping, and wording (`"verified"` = 0, zero `"proof"` guessed for legacy edges). Zero third-party paper text.
- Write `docs/design/modes-2026-11/reports/inv-v9-legacy.md` (with exact merge lines for `investigation_skeleton.rs` and `investigation_node.js`/`investigation_v3_shell.js`) and push to `gemini/modes/inv-v9-legacy`.
