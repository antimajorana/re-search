# Brief draft-inv-v7-v8-v10 — Investigate Orient (V7), Open any node (V8), and Notes (V10)

## 1. Packages and governing design sections

- **Packages:** **V7** (Orient: deterministic cues, orient prompt, main-result markers, gists, authors' half-badge with quote, reader pins, auto-start setting), **V8** (Open any node, any depth: open prompt, branch-only expansion, model-created child nodes with content-based tags, Block B streaming + shared run bar, breadcrumb and revisit reuse), and **V10 (Notes only)** (free-text reader notes on nodes, shown on the node card and preserved for F15 reading notes).
- **Governing design:**
  - `00-overview.md`: 0.1 (scope revision of 2026-11-25), 2.3 (F2 tags/prefixes), 2.5 (F4 quotes & `context.supplied`), 2.6 (F6 shared UI parts & evidence scale), 2.10 (display grades), 3.2 (rows 19–25 wording), 3.3 (F9 run-state vocabulary), 6 (build order: V7 week 8 / M3; V8 and V10 after week 8), 7.2–7.3 (Set P3, Set R, wording property tests), 8.1–8.3 (accepted decisions).
  - `investigate.md`: sections 2, 3 ("Follow the argument", stages D, E, K, node card, novelty row, provenance styles), 4.2–4.4 (content-based `m-` tags, layer-2 records, `investigation.note`), 5.1–5.3 (caps: Orient 8, Open 10), rules 6.1, 6.4–6.6, 6.10, 6.12, 6.14–6.15, 6.17–6.19, 6.22, sections 7–9 (rows V7, V8, V10), section 13 (decisions 1, 2, 3, 14, 18).
- **Owner decisions (2026-11-25) applied:**
  - **Version tracking (half of V10, stage M, eval set P2) is deferred.** Do not implement version checking, `POST .../investigation/version`, `Carry` matching, stale-version banners, or builder upgrade inside an existing session.
  - **V14 (prior work inside Investigate) is deferred.** Do not implement `confirm_prior_work`, `record_prior_work_check`, or the right half-badge prior-work task.
  - **Word "verified":** never appears anywhere in Investigate UI, badges, or copy (`00-overview.md` 0.1, 3.2 row 20). Quotes are `"matched"`; model text carries `"(model)"` (or starts `"Model judges:"` for verdicts).

## 2. Start conditions

- **Required merges on `gemini/work/fixed` before coding:**
  - Already merged on base `832eab5bf`: **F0, F1, F2, F3, F4, F6, F8, F9, F10, F14a, V1**.
  - Must be merged before starting V7: **V2, V3, V5** (`inv-a-v2-v5`), **V6** (`inv-v6-tasks`), and **F5** (`f5-f7-mode-shell`, for `api_dispatch.rs` route registration and `session_mode.rs`).
  - V8 starts after V7 checkpoint passes; V10 starts after V7 (and cleanly after V8 within this sequential brief).
- **Owner gates:** None beyond the accepted 2026-11-25 decisions (`00-overview.md` 8.1–8.3; `investigate.md` 13 #1, #2, #18).

## 3. Base-commit verified interfaces vs future-merge interfaces

### Verified on base `832eab5bf`
- **F2 (`crates/research-tools/src/paper_key.rs`):** `PaperKey`, `Mode::Investigate`, `base32_lower_nopad`, `ID_PREFIXES` (already reserves `"n-"`, `"m-"`, `"legacy:"`), `node_tag`.
- **F4 (`crates/research-tools/src/quote_core.rs`, `lines_sent.rs`):** `verify_quote`, `accept_quote_with_sent`, `clip_quote` (300-char cap), `CONTEXT_SUPPLIED_EVENT`, `QUOTE_MATCHED_WORD`.
- **F9 (`crates/research-tools/src/run_budget.rs`):** `count_model_calls`, `ModelCallBudget`, `RunState`, `InvestigateTaskState`, `investigate_task_run_state`.
- **F6 (`crates/desktop-server/assets/`):** `ui_badge.js` (`createBadge`, `createSplitBadge`, `createNodeCardShell`, `formatModelText`, `MODEL_TEXT_SUFFIX`, `QUOTE_MATCHED_WORD`, `BORDER_SOLID`, `BORDER_DASHED`), `run_bar.js` (`createRunBar`, `normalizeRunState`, `formatModelCallBudget`), `quote_chip.js` (`createQuoteChip`, `MAX_QUOTE_CHARS`). All registered in `crates/desktop-server/src/assets.rs` (365 lines).
- **V1 (`crates/research-tools/src/skeleton/model.rs`):** `SkNode` (`tag`, `kind`, `head_cites`, `intro_refs`, `fp`, `spans`, `proof_of`), `SkNodeKind::is_theorem_like` / `is_provable`.
- **Existing Investigation hooks (keep untouched):** `crates/desktop-server/src/investigation_finish.rs` (98 lines), `investigation_prompts.rs` (217 lines), `investigation_system_prompt.rs` (84 lines); `crates/desktop-server/src/state.rs` (**598 lines** — only 2 lines under 600; do NOT grow over 600).

### Future-merge interfaces to verify before coding (not yet in `832eab5bf`)
- **V2/V3/V5 (`inv-a-v2-v5`):** `skeleton/{edges, number, tags, stats, skip, symbols, bib}.rs`, `desktop-server/src/investigation_skeleton.rs`, `mapview` JSON shape, `assets/investigation_{outline, results, node, skip}.js`, `assets/investigation_shell.css`.
- **V6 (`inv-v6-tasks`):** `research-tools/src/{investigation_markers, map_node_read, map_reading, map_defs, map_fold}.rs`, `desktop-server/src/investigation_{tasks, task_context}.rs`, `investigation/2` profile in `tool_profile.rs`, and the exact `investigation.reading` / `investigation.task_requested` / `investigation.task_finished` payload schemas.
- **F5 (`f5-f7-mode-shell`):** `desktop-server/src/api_dispatch.rs` (split out of `http_srv.rs`) and `desktop-server/src/session_mode.rs`.
- **S12 (optional later):** Mode-neutral `SessionOutcome::Paused`. Until S12 merges, keep using V6's finish-hook `partial` -> `"Paused"` mapping via F9/F6; do **not** edit `crates/agent/src/runner.rs` (626 lines).

## 4. Smallest implementation checkpoints (push after each)

Run full checks (`check_file_length.py`, `cargo clippy`, `cargo test`, `node --check`, `node --test`), commit, and push to `gemini/modes/inv-v7-v8-v10` after each checkpoint, merging `origin/gemini/work/fixed` between checkpoints:

1. **Checkpoint 1 — V7 (Orient + Authors' badge + Pins):**
   - `crates/research-tools/src/status_badge.rs`: pure authors'-side badge builder (`status_badge(authors_claim, head_cites)`), deterministic cue-word detector on quoted lines (`"we prove"`, `"we show"`, `"our main"`, `"recalled from"`, `"restated"`, `"known"`, `"see e.g."`, etc.) producing explicit `cues` or `"no explicit cue in the quoted lines"` (rule 6.6), and heading-citation wording `"cites [<key>] in its heading"` (never `"attributed by the authors to"`). Never emits `"verified"`.
   - Enforce rule 6.6 in V6's `map_reading.rs` (or a new sibling `map_authors_claim.rs` if `map_reading.rs` approaches 500 lines): `new`/`restated`/`background` require a matched quote from this paper; `attributed_to` keys must exist in `skeleton.bib`; `cues` are app-filled (reject caller-supplied app-only fields).
   - `crates/desktop-server/src/investigation_task_prompts.rs`: Orient prompt (wrapping abstract, intro, conclusion, and theorem-like headings as quoted data, rule 6.19; cap 8 model calls).
   - `crates/desktop-server/src/investigation_settings.rs`: persisted user setting for Orient auto-start (default on when a model is connected, off toggleable; `investigate.md` 13 #1). **Do NOT grow `state.rs` (598 lines on base) over 600 lines** — persist Orient auto-start in this small sidecar helper module (or move unit tests out of `state.rs` in a dedicated zero-behaviour-change commit first if `DesktopState` needs a field).
   - `crates/desktop-server/src/investigation_notes.rs`: `POST /api/sessions/{id}/investigation/notes` writing an additive `investigation.note` event for `pin: "main_result" | "not_main_result"` (carrying `tag` and `node_fp`, validated against pinned skeleton tag; rule 6.4/6.12). Wire reader pins into `map_fold` (pins override model `role` and recompute V5 `skip_plan`) and expose them in `investigation_skeleton.rs`'s `mapview`.
   - `crates/desktop-server/assets/investigation_badges.js`: thin adapter over F6 `createSplitBadge` / `createBadge` / `createQuoteChip` showing `"Authors: New / Restated / Background / Not stated (model)"`, quote popover, deterministic cue line, pin/unpin toggle, and a disabled placeholder slot on the right half-badge (reserved for V16's `"Search wider in Explore"`).
2. **Checkpoint 2 — V8 (Open any node, any depth):**
   - Content-based model-node tag helper (`crates/research-tools/src/map_model_nodes.rs` or in `map_reading.rs` keeping <= 600 lines): `"m-" + base32_lower_nopad(sha256(attach_to | kind | normalised first quote))[..10]` (`investigate.md` 4.2), ensuring re-runs update rather than duplicate `m-` nodes. Extend `map_reading` / `map_fold` so `attach_to` accepts both `n-` skeleton tags and existing `m-` tags (arbitrary depth), enforcing rule 6.4 (requested tag, descendants, and used nodes only), rule 6.5 (containment), and rule 6.12 (additive history + `"changed since you last opened this"` diff indicator).
   - Extend `crates/desktop-server/src/investigation_task_prompts.rs`: Open-node prompt (cap 10 model calls) injecting node statement, proof, and statements of used nodes (wrapped as quoted material and logged via F4 `context.supplied`), instructing proof split into 3–7 child step nodes without expanding sibling branches ("Follow the argument", `investigate.md` 3).
   - `crates/desktop-server/assets/investigation_node_model.js`: Block B inside F6 `createNodeCardShell` — streaming updates, gist `"(model)"`, hypotheses checklist with `quote_chip.js`, proof steps as expandable child nodes (clicking an unexpanded child opens only that branch; revisiting reuses accepted saved steps), open questions, `"Connect a model to get an explanation"` when unconnected, breadcrumb navigation back through parent chain, and F6 `createRunBar` (Stop, `"Paused: (reason)"` + Continue, Interrupted/Failed + Retry; never `"Partial"` as state). Drop the word `"branch"` everywhere in new UI (`investigate.md` 13 #18).
3. **Checkpoint 3 — V10 (Notes only; version tracking deferred):**
   - Extend `crates/desktop-server/src/investigation_notes.rs` to accept free-text notes `{ tag, node_fp, text }` (length-bounded, validated against pinned skeleton or `m-` tag) appended as additive `investigation.note` events (rule 6.12), folded into `map_fold` and exposed in `investigation_skeleton.rs`'s `mapview` per node with history.
   - `crates/desktop-server/assets/investigation_notes_ui.js`: Note composer and list mounted inside the node card (plain `"you"` style from F6 `EVIDENCE_GRADES.user`, zero `innerHTML`, escaped via `textContent`). Notes stay intact on re-runs and are exposed cleanly in `mapview` so F15 (`writeup_paper.rs`) and V16 (`handoff.packet.user_notes`) can consume them.

## 5. Owned files (all <= 600 lines)

- **New Rust files:**
  - `crates/research-tools/src/status_badge.rs` (+ `status_badge_tests.rs` if needed)
  - `crates/research-tools/src/map_authors_claim.rs` (if splitting rule 6.6 from V6's `map_reading.rs`)
  - `crates/research-tools/src/map_model_nodes.rs` (if splitting `m-` tag/depth logic from `map_reading.rs`)
  - `crates/desktop-server/src/investigation_task_prompts.rs`
  - `crates/desktop-server/src/investigation_settings.rs` (Orient auto-start persistence without growing `state.rs`)
  - `crates/desktop-server/src/investigation_notes.rs` (+ `investigation_notes_tests.rs`)
- **Small targeted edits to V3/V6 files (only after V3 and V6 merge, keeping each <= 600 lines):**
  - `crates/research-tools/src/map_reading.rs`, `map_fold.rs` (if > 550 lines, split into the sibling files above first)
  - `crates/desktop-server/src/investigation_tasks.rs`, `investigation_task_context.rs` (wire Orient/Open prompts and `m-` node context)
  - `crates/desktop-server/src/investigation_skeleton.rs` (thin `mapview` hook folding pins/notes/badges; V9 serializes its `mapview` hook after V7 to avoid concurrent edits)
- **New UI & test files:**
  - `crates/desktop-server/assets/investigation_badges.js`
  - `crates/desktop-server/assets/investigation_node_model.js`
  - `crates/desktop-server/assets/investigation_notes_ui.js`
  - `crates/desktop-server/tests/investigation_orient_open_notes.test.mjs`
  - `crates/desktop-server/tests/investigation_v7_v8_v10.rs`

## 6. Shared registration seams and "Do not edit" files

- **Registration seams:**
  - `crates/research-tools/src/lib.rs` (300 lines): export `pub mod status_badge;` (and any new `map_*` module).
  - `crates/desktop-server/src/lib.rs` (215 lines): declare `mod investigation_task_prompts;`, `mod investigation_settings;`, `mod investigation_notes;`.
  - `crates/desktop-server/src/investigation_skeleton.rs` (from V3): add the thin `mapview` projection hook for folded Orient labels, pins, `m-` nodes, and reader notes. (V9 must not edit `investigation_skeleton.rs` concurrently with V7.)
  - `crates/desktop-server/src/api_dispatch.rs` (delivered by F5): add 1 arm for `("POST", ["api", "sessions", id, "investigation", "notes"])` (and 1 settings arm if Orient auto-start uses its own route) behind `AuthGuard::authorize_api`. (If F5 is not yet merged, leave `http_srv.rs` untouched and document the exact 1-line arm in the report.)
  - `crates/desktop-server/src/assets.rs` (365 lines): register `/assets/investigation_badges.js`, `/assets/investigation_node_model.js`, `/assets/investigation_notes_ui.js`.
  - **UI mount seam (`investigation_node.js`, owned by V3):** mount `investigation_badges.js`, `investigation_node_model.js`, and `investigation_notes_ui.js` only via clean slot hooks after V3 merges; preserve a dedicated `.inv-node-handoff-slot` (and `investigation_badges.js`'s right half-badge slot) so V16 mounts conflict-free.
- **Do NOT edit:**
  - `crates/research-tools/tests/golden/tool_defs.txt`, `tool_profiles.txt`, `tool_profile.rs` (no new tools in V7/V8/V10).
  - `crates/desktop-server/src/investigation_finish.rs` (98 lines), `investigation_prompts.rs` (217 lines), `investigation_system_prompt.rs` (84 lines) — legacy Investigation hooks stay untouched.
  - `crates/desktop-server/src/state.rs` (598 lines — never grow over 600; only touch if first splitting tests out in a separate zero-behaviour-change commit).
  - `crates/agent/src/resume.rs`, `crates/agent/src/runner.rs` (626 lines), `crates/desktop-server/src/http_srv.rs` (702 lines), `routes_session.rs` (616), `evidence.rs` (1057), `session_view.rs` (753), `investigation.rs` (597 — do not grow), `assets/index.html` (768), `assets/app.js` (932), `assets/investigation_map.js` (594).
  - `Cargo.lock` (zero new dependencies).

## 7. Tests through the entry point

- **Rust unit & integration (zero network, zero real model calls; scripted transport only):**
  - Rule 6.6 table tests: `new`/`restated`/`background` without quote rejected; unknown `attributed_to` bib key rejected; app-only `cues` rejected if supplied by model; cue-word extraction and `"no explicit cue in the quoted lines"` verified; heading citation renders `"cites [k] in its heading"`.
  - **Eval Set P3 alignment (fixed tier):** self-authored paper fixture with known new/restated/background/not-stated results tested through scripted Orient transport; verify Orient cap (<= 8 model calls), incremental item saving, pause-at-cap (`partial` -> `"Paused"`), and Continue preserving run-1 records (rule 6.12, 6.15).
  - Pins & skip plan (rule 6.14): pinning/unpinning `main_result` overrides model `role` and recomputes skip plan; nothing dimmed when zero main results exist.
  - Open any depth (V8): open a theorem, open its proof, expand one proof-step `m-` child without expanding siblings, open a grandchild `m-` node; verify deterministic `m-` tag stability across re-runs (no duplicates), rule 6.4/6.5 scope rejection, `"changed since you last opened this"` history preservation, and revisit reuse without model calls.
  - Notes (V10): `POST .../investigation/notes` auth + validation + persistence across re-runs; verify opening a `tex-1` session under a newer builder keeps stored skeleton unchanged (rule 6.13 interim test).
  - **Wording property test (`00-overview.md` 7.3):** assert zero occurrences of `"verified"` (case-insensitive) across all `status_badge`, prompt UI payloads, and rendered DOM text; assert `"new"`/`"novel"` never appear outside the authors' left half-badge; assert `"Partial"` never appears as a run state label.
  - Regression: `tool_def_golden`, `tool_profile_golden`, `eval_set_r`, existing `investigation*` suites.
- **Node UI tests (`node --test`):** XSS escaping (`<script>`, `<img onerror>`) on gists, quotes, notes, badges; split half-badge rendering; Block B streaming + `run_bar.js` states + breadcrumb; note creation and re-render.
- **No third-party paper text** in any fixture.

## 8. Document conflicts & resolutions

- **`investigate.md` 7 vs base `832eab5bf`:** `desktop-server/src/investigation.rs` is 597 lines (not 579) and `investigation_view.rs` is 385 lines (not 217). Do not add code to `investigation.rs`; place all new routes in `investigation_notes.rs` and new view helpers in V6/V7 modules.
- **`00-overview.md` 2.1 row F12 vs `investigate.md` 9 V7:** Overview mentions `status_badge.rs` having a "check side (V7, V14)". Because V14 is deferred (`00-overview.md` 0.1 wins), V7 implements only the authors' side of `status_badge.rs`; the right half-badge is owned by V16 (`"Search wider in Explore"`).
- **`investigate.md` 9 V10 vs `00-overview.md` 0.1:** Version tracking, `Carry` matching, `POST .../investigation/version`, and eval set P2 are deferred (`00-overview.md` 0.1 wins). V10 delivers notes only.
- **`investigate.md` 6.15 vs `00-overview.md` 3.1 row 10:** Keep V6's finish-hook `partial` -> `"Paused"` repair until Solve S12 lands mode-neutral `SessionOutcome::Paused`; do not touch `runner.rs` (626 lines).

## 9. Out-of-scope / deferred items and hand-back

- **Out of scope / deferred:** Version tracking (`POST .../investigation/version`, `Carry` matching, stale diffs, Set P2); V14 (`record_prior_work_check`, `confirm_prior_work`, prior-work coverage); V16 hand-off buttons and right half-badge Explore link; F16b lessons injection; F15 reading-notes writer (`writeup_paper.rs`).
- **Hand-back (`RULES.md`):** Write `docs/design/modes-2026-11/reports/inv-v7-v8-v10.md` (base commit, per-package files & line counts, measured timing/caps, merge notes, exact test counts, pre-existing failures) and push to `gemini/modes/inv-v7-v8-v10`.
