# Report: `inv-v7-v8-v10` (Investigate V7 Orient, V8 Open any node at any depth, V10 Notes only)

- **Base commit (merged):** `692ffae9c2e78a67470d34578608c2eebcad5541` (`origin/gemini/work/fixed`, merged cleanly in commit `12558cb`; pre-merge Checkpoint 1 branch base was `3ef88bafb5c7afd23be9d9091fc2faab45c71bc6`)
- **Branch:** `gemini/modes/inv-v7-v8-v10`
- **Toolchain on Linux VM:** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `cargo 1.100.0 (3acc79c0c 2026-11-05)`, `node v22.23.5`, `Python 3.12.16` (non-root `uid=1001(antigravity)`, `RE_SEARCH_PYTHON` unset)
- **Guard rails:**
  - `Cargo.lock` untouched (zero new dependencies).
  - `crates/research-tools/tests/golden/tool_defs.txt`, `crates/research-tools/tests/golden/tool_profiles.txt`, and `crates/research-tools/src/tool_profile.rs` untouched.
  - Forbidden files untouched: `investigation_finish.rs`, `investigation_prompts.rs`, `investigation_system_prompt.rs`, `state.rs`, `resume.rs`, `runner.rs`, `http_srv.rs`, `routes_session.rs`, `evidence.rs`, `session_view.rs`, `desktop-server/src/investigation.rs`, `assets/index.html`, `assets/app.js`, `assets/investigation_map.js`.
  - Zero third-party paper text anywhere in code or tests (synthetic toy fixtures only).
  - Zero `"verified"` / `"Verified"` strings in Investigate UI, badges, or copy (quotes are `"matched"`, model prose carries `"(model)"`); `"Partial"` never shown as a run-state label (`"Paused"` used instead); `"branch"` dropped from new UI; `"new"` / `"novel"` appear only in the authors' left half-badge.
  - `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` exits 0 (`No touched source file is over 600 lines.`).

---

## 1. What was implemented

### 1.1 Package V7 — Orient, Rule 6.6 cues, authors' half-badge, reader pins, auto-start setting (`afb6c23`, `2161780`)

| File | Status | Lines | Contents |
|---|---|---:|---|
| `crates/research-tools/src/status_badge.rs` | Added | 336 | Pure `status_badge(authors_claim, head_cites)` builder; deterministic Rule 6.6 cue detector on quoted lines (`"we prove"`, `"we show"`, `"our main"`, `"recalled from"`, `"restated"`, `"known"`, `"see e.g."`, etc.) emitting explicit `cues` or `"no explicit cue in the quoted lines"`; exact heading-citation wording `"cites [<key>] in its heading"` (never `"attributed by the authors to"`); 300-char quote clipping; never emits `"verified"` |
| `crates/research-tools/src/status_badge_tests.rs` | Added | 102 | 5 unit tests covering all 4 authors' labels, cue detection and no-cue fallback, heading citation wording, quote clipping, and absence of `"verified"` |
| `crates/research-tools/src/map_authors_claim.rs` | Added | 173 | Rule 6.6 validation helper for `record_map_reading`: requires matched quote from this paper for `new` / `restated` / `background`, validates every `attributed_to` key against `skeleton.bib`, enforces app-only `cues` rejection, computes deterministic cues |
| `crates/desktop-server/src/investigation_task_prompts.rs` | Added | 201 | Task prompt builders for Orient (Rule 6.19 DATA ONLY wrapping of abstract, intro, conclusion, theorem/definition headings; cap 8) and Open (statement, proof, used-node statements as quoted data, 3–7 proof steps without expanding sibling branches; cap 10) + unit tests |
| `crates/desktop-server/src/investigation_settings.rs` | Added | 154 | Sidecar persistence (`investigation_settings.json`) for `orient_auto_start` (default `true`, toggleable via `GET`/`PUT /api/settings/investigate`), keeping `state.rs` (598 lines) untouched + unit test |
| `crates/desktop-server/src/investigation_mapview_v7.rs` | Added | 453 | `enrich_mapview_with_v7`: projects folded `role`, `pin`/`pinned`, `effective_is_main_result`, `authors_claim` (via `status_badge`), `gist`, `purpose`, `hypotheses`, `anchors` (raw excerpts stripped), `children`, `notes`, `changed_since_last`, `orient_auto_start`, `m-` model nodes and `m-`-touching model edges, and recomputed V5 `skip_plan` + `dim` (Rule 6.14) matching V13 `investigation_wholemap_model.js` |
| `crates/desktop-server/assets/investigation_badges.js` | Added | 244 | Thin adapter over F6 `createSplitBadge` / `createBadge` / `createQuoteChip` rendering `"Authors: New / Restated / Background / Not stated (model)"`, quote popover, deterministic cue line, heading cite line, pin/unpin toggle (`"Main result (you)"` / `"Main result (model)"`), and disabled placeholder right half-badge slot reserved for V16 |

### 1.2 Package V8 — Open any node at any depth, content-based `m-` tags, Block B UI (`ac67047`, `3d866a0`)

| File | Status | Lines | Contents |
|---|---|---:|---|
| `crates/research-tools/src/map_model_nodes.rs` | Added | 355 | Deterministic content-based tag generator `"m-" + base32_lower_nopad(sha256(attach_to | kind | normalised first quote))[..10]` (`investigate.md` §4.2), `normalize_model_kind`, `role_allowed_for_model_kind`, `collect_existing_model_nodes`, `ancestor_chain`, `is_model_descendant_of_any` + 3 unit tests |
| `crates/research-tools/src/map_defs.rs` | Modified | 400 | Added optional `attach_to` and `model_kind` fields (with `skip_serializing_if = "Option::is_none"`) to `ReadingItemRecord` event payload struct; `read_map_node_def()` and `record_map_reading_def()` schemas and goldens untouched |
| `crates/research-tools/src/map_reading_validate.rs` | Modified | 498 | Replaced V6 `unsupported_model_node` stub with full V8 `new: { id, attach_to, kind }` creation, same-call `temp_id_map`, arbitrary-depth `n-`/`m-` parent resolution, Rule 6.4 open scope enforcement, Rule 6.5 node-or-proof containment & model-node role restrictions, and `m-` edge validation |
| `crates/research-tools/src/map_reading.rs` | Modified | 407 | Loads existing `m-` nodes from ledger events, extends `compute_open_scopes` for `n-` & `m-` targets/descendants/ancestors, emits `assigned_tags` and per-item `temp_id`/`attach_to`/`kind` |
| `crates/research-tools/src/map_fold.rs` | Modified | 458 | Added `attach_to`, `model_kind`, `is_model_node`, `children`, `pin`, `effective_is_main_result`, `notes`, `changed_since_last` to `FoldedNodeReading`; resolves owning skeleton `fp` for `m-` nodes, notes, and edges; records parent `children` in exact appearance order (`child_links`) |
| `crates/research-tools/src/map_node_read.rs` | Modified | 488 | Resolves `m-` tags in `read_map_node` to their owning skeleton node spans and attaches `model_node` metadata (`tag`, `attach_to`, `kind`, `root_skeleton_tag`) |
| `crates/desktop-server/src/investigation_task_context.rs` | Modified | 442 | Added `build_task_context_with_events` resolving `m-` target tags to owning skeleton spans while prepending the step ancestor chain and saved gist inside quoted source context |
| `crates/desktop-server/src/investigation_tasks.rs` | Modified | 507 | Updated Open default cap to 10 (`investigate.md` §5.1–5.3), wired V7/V8 prompts via `investigation_task_prompts`, accepted existing `m-` tags in `parse_and_validate_tags`, and switched to `build_task_context_with_events` |
| `crates/desktop-server/assets/investigation_node_model.js` | Added | 452 | Block B component inside F6 `createNodeCardShell`: unconnected message (`"Connect a model to get an explanation"`), F6 `createRunBar` (Stop, `"Paused"` + Continue, Interrupted/Failed + Retry; never `"Partial"`), streaming preview, parent-chain breadcrumb navigation, Rule 6.12 `"Changed since you last opened this"` diff indicator, gist/purpose `"(model)"`, hypotheses with F6 `quote_chip.js`, expandable child proof steps (isolated per-step opening without using the word `"branch"`, saved-step reuse on revisit), open questions |

### 1.3 Package V10 — Notes only (version tracking & V14 deferred) (`afb6c23`, `3d866a0`, `ae5dd4e`)

| File | Status | Lines | Contents |
|---|---|---:|---|
| `crates/desktop-server/src/investigation_notes.rs` | Added | 234 | `POST /api/sessions/{id}/investigation/notes` handler accepting reader pins (`pin: "main_result" | "not_main_result"`) and/or free-text reader notes (`text`, trimmed and bounded to 4,000 chars), validated against pinned skeleton `n-` or folded `m-` tag and owning `node_fp` (409 on mismatch), appending additive `investigation.override` / `investigation.note` events |
| `crates/desktop-server/src/investigation_notes_tests.rs` | Added | 142 | Unit tests verifying reader pin/note validation, wrong `node_fp` 409 refusal, `m-` node note attachment, additive preservation, and Rule 6.14 skip-plan recomputation |
| `crates/desktop-server/assets/investigation_notes_ui.js` | Added | 204 | Reader notes composer and chronological list styled with F6 `EVIDENCE_GRADES.user` (`"you"`), 4,000-char limit, zero `innerHTML` writes (all user/model strings via `textContent`), notes preserved across re-runs |

### 1.4 Wiring, shell mounts, and tests (`2161780`, `12558cb`, `ac67047`, `3d866a0`, `ae5dd4e`)

| File | Status | Lines | Contents |
|---|---|---:|---|
| `crates/desktop-server/src/investigation_skeleton.rs` | Modified | 538 | Wired `GET /api/sessions/{id}/investigation/mapview` to read `orient_auto_start` setting and call `enrich_mapview_with_v7` |
| `crates/desktop-server/src/api_dispatch.rs` | Modified | 433 | Registered `POST /api/sessions/{id}/investigation/notes`, `GET /api/settings/investigate`, and `PUT /api/settings/investigate` behind `AuthGuard::authorize_api` |
| `crates/desktop-server/src/assets.rs` | Modified | 542 | Registered `/assets/investigation_badges.js`, `/assets/investigation_node_model.js`, and `/assets/investigation_notes_ui.js` |
| `crates/desktop-server/src/lib.rs` | Modified | 238 | Exported `investigation_mapview_v7`, `investigation_notes`, `investigation_settings`, `investigation_task_prompts` |
| `crates/research-tools/src/lib.rs` | Modified | 342 | Exported `map_authors_claim`, `map_model_nodes`, `status_badge` |
| `crates/desktop-server/assets/api.js` | Modified | 522 | Added `startInvestigationTask`, `saveInvestigationNote`, `getInvestigateSettings`, `putInvestigateSettings` |
| `crates/desktop-server/assets/investigation_node.js` | Modified | 413 | Mounted V7 `investigation_badges.js`, V8 `investigation_node_model.js`, V10 `investigation_notes_ui.js`, and reserved `.inv-node-handoff-slot` (`data-v16-slot="prior-work-explore"`) for V16 |
| `crates/desktop-server/assets/investigation_v3_shell.js` | Modified | 594 | Added `'Orient paper'` button and one-shot Orient auto-start when `orient_auto_start` is true and a model is connected; wired pin/note/open/stop/continue/retry callbacks; preserved `'← Previous statement'` |
| `crates/research-tools/src/map_fold_tests.rs` | Modified | 407 | Extended fold tests for `m-` node projection, chronological child ordering, pin overrides, and notes |
| `crates/research-tools/src/map_reading_tests.rs` | Modified | 538 | Updated V6 item-5 `unsupported_model_node` check to V8 `m-` acceptance and added `v8_model_nodes_stable_across_reruns_arbitrary_depth_and_sibling_scope_isolation` |
| `crates/desktop-server/tests/investigation_orient_open_notes.test.mjs` | Added | 564 | 5 Node UI tests covering V7 badges/cues/pins, V13 auto-activation (`isMainResultNode` / `extractAuthorsClaimLabel`), V8 Block B streaming/run-bar/breadcrumbs/isolated step expansion, V10 notes composer/persistence, integrated `investigation_node.js` mount, XSS safety (zero injected tags / zero `innerHTML`), and wording property checks |
| `crates/desktop-server/tests/investigation_v7_v8_v10.rs` | Added | 566 | 2 end-to-end HTTP integration tests: Set P3 scripted Orient + Rule 6.6 cues & heading cites + settings auth/toggle + pins & Rule 6.14 skip plan; V8 arbitrary-depth `m-` stability + sibling scope isolation + Rule 6.12 diff + Rule 6.15 `"Paused"` + V10 notes surviving re-runs + static asset serving |

---

## 2. Deviations, decisions taken alone, and assumptions

### Deviations from the design
1. **Deferred scope intentionally omitted per owner decisions (2026-11-25):** Version tracking (half of V10, stage M, Set P2, `POST .../investigation/version`, `Carry` matching) and V14 (`confirm_prior_work`, `record_prior_work_check`, right half-badge prior-work task) were not implemented; the right half-badge holds a disabled placeholder slot for V16 (`"Search wider in Explore"`).
2. **V6 `unsupported_model_node` stub replaced in `map_reading_tests.rs`:** As explicitly directed by `inv-v7-v8-v10.md` §4 Checkpoint 2 and `reports/inv-v6-tasks.md` §4, item 5 in `map_reading_tests.rs` was updated from expecting `unsupported_model_node` to expecting V8 `new` model-node acceptance with content-based `m-` tag generation.
3. **Orient auto-start persisted in sidecar `investigation_settings.json`:** `crates/desktop-server/src/state.rs` is at 598 lines and listed as Do-Not-Edit in `inv-v7-v8-v10.md` §5, so `orient_auto_start` is persisted cleanly in `investigation_settings.json` beside `settings.json` in `data_dir` via `investigation_settings.rs`.

### Decisions taken alone
1. **Content-based `m-` tag normalization (`map_model_nodes.rs`):** Normalized the first anchor quote by collapsing ASCII whitespace runs to a single space, trimming, and lowercasing before hashing `attach_to | kind | normalized_first_quote` with SHA-256 and taking `base32_lower_nopad(..)[..10]`, so minor whitespace/case differences on re-runs update the same `m-` node instead of duplicating it.
2. **Child proof-step ordering (`map_fold.rs`):** Multiple `new` `m-` items recorded in a single `investigation.reading` event share the same ledger `ev.seq`. Added explicit `child_links` insertion tracking so parent `children[]` preserves exact event and item appearance order rather than breaking ties alphabetically by `m-` hash tag.
3. **Relative `./` imports inside node-card JS modules:** `investigation_node.js`, `investigation_badges.js`, `investigation_node_model.js`, and `investigation_notes_ui.js` use relative `./` module imports so both the desktop HTTP server (`/assets/...`) and Node's ESM test runner (`node --test`) load them directly without a stub server.
4. **Named constants for accepted overview §8 wording/event decisions:** Badge labels, `"Changed since you last opened this"`, `"no explicit cue in the quoted lines"`, `"cites [<key>] in its heading"`, and event kind constants are centralized in `status_badge.rs`, `investigation_mapview_v7.rs`, `investigation_notes.rs`, and the JS modules so wording can be changed in one place.

### Assumptions
- Assumed maximum reader note length of 4,000 characters (`investigation_notes.rs` and `investigation_notes_ui.js`) is sufficient for V10 notes while keeping ledger events bounded.
- Assumed `investigation_v3_shell.js` checks connected model state from the existing `/api/settings/providers` / shell session state before auto-starting Orient once per freshly pinned skeleton when `orient_auto_start` is enabled.
- Assumed V16 will mount `"Search wider in Explore"` into `.inv-node-handoff-slot` (`data-v16-slot="prior-work-explore"`) on `investigation_node.js` and optionally replace the disabled right half-badge slot in `investigation_badges.js`.

---

## 3. Merge notes

### Existing files touched (all kept small, local, and <= 600 lines)
- `crates/desktop-server/src/api_dispatch.rs` (433 lines): added 3 route arms behind `AuthGuard::authorize_api`:
  ```rust
  ("POST", ["api", "sessions", id, "investigation", "notes"]) => {
      write_api_res(&mut stream, crate::investigation_notes::handle(state, id, &req.body));
  }
  ("GET", ["api", "settings", "investigate"]) => {
      write_api_res(&mut stream, crate::investigation_settings::handle_get(state));
  }
  ("PUT", ["api", "settings", "investigate"]) => {
      write_api_res(&mut stream, crate::investigation_settings::handle_put(state, &req.body));
  }
  ```
  (Placed immediately after `investigation/tasks` and before `investigation/cited`, and right after `settings/providers`.)
- `crates/desktop-server/src/assets.rs` (542 lines): registered `/assets/investigation_badges.js`, `/assets/investigation_node_model.js`, and `/assets/investigation_notes_ui.js` directly above `/assets/investigation_context.js`.
- `crates/desktop-server/src/lib.rs` (238 lines) & `crates/research-tools/src/lib.rs` (342 lines): additive `pub mod` exports only.
- `crates/desktop-server/src/investigation_skeleton.rs` (538 lines): passes `orient_auto_start` into `enrich_mapview_with_v7`.
- `crates/desktop-server/src/investigation_tasks.rs` (507 lines) & `investigation_task_context.rs` (442 lines): V7/V8 prompts, Open cap = 10, and `m-` tag resolution.
- `crates/research-tools/src/map_defs.rs` (400), `map_reading.rs` (407), `map_reading_validate.rs` (498), `map_fold.rs` (458), `map_node_read.rs` (488): V7 Rule 6.6 + V8 `m-` model-node support; all backward-compatible with V6 events.
- `crates/desktop-server/assets/api.js` (522), `investigation_node.js` (413), `investigation_v3_shell.js` (594): mounted V7/V8/V10 components while preserving `'← Previous statement'`, V11/V12 context mount points, and V13 whole-map controls.

### Downstream handoff seams
- **V13 (`investigation_wholemap_model.js`):** Verified in `investigation_orient_open_notes.test.mjs` that `isMainResultNode` and `extractAuthorsClaimLabel` activate automatically from V7 `mapview` fields (`effective_is_main_result`, `role`, `pin`, `authors_claim.label`).
- **V16 (Handoff to Explore / Solve):** Reserved `.inv-node-handoff-slot` (`data-v16-slot="prior-work-explore"`) in `investigation_node.js` and disabled right half-badge slot in `investigation_badges.js`; reader notes are exposed on every folded node (`node.notes[]`) for `handoff.packet.user_notes`.
- **F15 (`writeup_paper.rs`):** Reader notes are appended as additive `investigation.note` ledger events and projected on `mapview.nodes[].notes[]` with `seq`, `text`, and `ts`.

---

## 4. Verification commands and exact results

All commands were executed as non-root (`uid=1001(antigravity)`) with `RE_SEARCH_PYTHON` unset on commit `ae5dd4e35057079fb831985d90ff88ce9d0db2ab`.

| Command | Exit | Exact result |
|---|---:|---|
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | 0 | `No touched source file is over 600 lines.` |
| `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` | 0 | All asset JS files syntax-valid |
| `node --test crates/desktop-server/tests/*.test.mjs` | 0 | **204 passed; 0 failed; 0 skipped** (includes 5 tests in `investigation_orient_open_notes.test.mjs`) |
| `cargo test --locked -p research-tools --test tool_def_golden --test tool_profile_golden --test investigation_v6_markers_and_tools` | 0 | **9 passed; 0 failed; 0 ignored** (3 + 3 + 3) |
| `cargo test --locked -p desktop-server --test eval_set_r --test mode_markers_r2 --test investigation_v6_tasks --test investigation_v7_v8_v10` | 0 | **12 passed; 0 failed; 0 ignored** (`eval_set_r`: 5, `investigation_v6_tasks`: 2, `investigation_v7_v8_v10`: 2, `mode_markers_r2`: 3) |
| `cargo test --locked -p agent --test resume_loop` | 0 | **6 passed; 0 failed; 0 ignored** |
| `cargo test --locked -p agent --lib resume` | 0 | **12 passed; 0 failed; 0 ignored** |
| `cargo test --locked -p research-tools --lib map_` | 0 | **25 passed; 0 failed; 0 ignored** |
| `cargo test --locked -p research-tools --lib status_badge` | 0 | **5 passed; 0 failed; 0 ignored** |
| `cargo test --locked -p desktop-server --lib investigation_` | 0 | **43 passed; 0 failed; 0 ignored** |
| `cargo test --workspace --locked --no-fail-fast --quiet -- --test-threads=8` | 0 | **134 test targets: 1,108 passed; 0 failed; 2 ignored** (the 2 ignored are pre-existing live-tier tests in `crates/agent/tests/eval_harness.rs`) |
| `cargo clippy --locked -p research-tools --lib --test tool_def_golden --test tool_profile_golden --test investigation_v6_markers_and_tools -- -D warnings` | 0 | 0 warnings, 0 errors |
| `cargo clippy --locked -p desktop-server --lib --tests -- -D warnings -A clippy::manual_unwrap_or` | 0 | 0 warnings, 0 errors (only allowing pre-existing `evidence.rs:744`) |
| `cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or -A clippy::match_like_matches_macro -A clippy::unnecessary_map_or -A unused_imports -A clippy::items_after_test_module` | 0 | 0 warnings, 0 errors across the full workspace |
| `git diff --stat origin/gemini/work/fixed -- Cargo.lock crates/research-tools/tests/golden/` | 0 | Empty diff (untouched) |

---

## 5. Pre-existing warnings on Linux (with proof)

Confirmed on untouched files (already documented in `RULES.md` and `reports/inv-v6-tasks.md` on `origin/gemini/work/fixed`):
1. `crates/desktop-server/src/evidence.rs:744` — `clippy::manual_unwrap_or` (untouched, > 600 lines).
2. `crates/research-tools/tests/sandbox_and_fs.rs:6,9` — `unused_imports` (`TEX_AUDIT`, `is_symlink_or_junction`, used only on `#[cfg(windows)]`).
3. `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` — `unused_imports` (`BufRead`, `BufReader`, used only on `#[cfg(windows)]`).
4. `crates/agent/tests/eval_support/manifest.rs:289` (`clippy::match_like_matches_macro`), `crates/agent/tests/eval_support/sheet.rs:102` (`clippy::unnecessary_map_or`), and `crates/agent/src/guardrails.rs:335` (`clippy::items_after_test_module`).

Zero clippy warnings exist in any file created or modified in this branch.

---

## 6. What could not be compiled or run on Linux

- All `#[cfg(windows)]` targets and Windows-only integration tests (`crates/desktop-app` WebView2 host, Windows AppContainer sandbox tests, Windows DPAPI vault tests).
- Real WebView2 / browser visual rendering (JS modules were verified under Node v22.23.5 with `node --check` and full DOM/event/XSS tests in `investigation_orient_open_notes.test.mjs`).
- Live network / real vendor model runs (forbidden by `RULES.md`; tested via in-process `ScriptedAnthropic` loopback mock in `investigation_v7_v8_v10.rs`).

---

## 7. Bugs noticed (not fixed, outside brief scope) and design notes

- **Pre-existing untouched Linux clippy lints:** Listed in Section 5 (`sandbox_and_fs.rs:6,9`, `vault_lock_multiprocess.rs:6`, `evidence.rs:744`, `eval_support/{manifest,sheet}.rs`, `guardrails.rs:335`); left untouched per `RULES.md`.
- **`runner.rs` `max_turns` boundary on `StopReason::ToolUse`:** `crates/agent/src/runner.rs:410` stops when `leg_turns == params.max_turns` before executing the current turn's tool calls, so a task capped at `max_turns: 1` that immediately calls `record_map_reading` halts before saving the record. This is expected existing behavior (untouched `runner.rs`, 626 lines); Orient (default cap 8) and Open (default cap 10) leave ample turns for reading and saving records, and `investigation_v7_v8_v10.rs` tests Rule 6.15 `"Paused"` with `max_turns: 2`.
- **`investigation_v3_shell.js` near 600-line ceiling (594 lines):** Any future brief adding further shell wiring (such as V16 hand-off controls) should extract a helper module rather than growing `investigation_v3_shell.js` over 600 lines.
