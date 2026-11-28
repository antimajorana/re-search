# Package F16b — User-Approved Lessons Shown to Models: Implementation Report

**Branch:** `coord/modes-f16b-lessons` (merged `origin/gemini/work/fixed` `717879c`)  
**Worktree:** `C:\Users\antimajorana\Documents\Codex\2026-11-26\files-mentioned-by-the-user-00\work\lane-f16b-lessons`

## Summary of Implementation

Implemented Package **F16b — User-Approved Lessons Shown to Models** across four logical checkpoints while preserving all F16a (`user_version = 1`) tables/endpoints and leaving all frozen files unmodified.

### Checkpoint 1 — Schema Migration v2 & Capture Validation (`d2ec665`, refactored under 600-line ceiling in `0d3a2b7`)
- **`crates/ledger/src/lessons_db.rs`**, **`crates/ledger/src/lessons_b_store.rs`**, **`crates/ledger/src/lessons_b_store_tests.rs`**, **`crates/ledger/src/lib.rs`**:
  - Bumped `LESSONS_SCHEMA_VERSION` from `1` to `2` with forward-only `v1 -> v2` migration (`USER_LESSON_SCHEMA_V2`, creating `user_lesson` table + indexes) preserving all existing F16a rows and refusing downgrades when `user_version > 2`.
  - Implemented SQLite CRUD, atomic savepoint-guarded `increment_user_lesson_uses` (deduplicated per call), and `restore_user_lessons` on `LessonsDb`.
- **`crates/research-tools/src/lessons_capture.rs`**, **`crates/research-tools/src/lessons_draft.rs`**, **`crates/research-tools/src/lessons_backup.rs`**, **`crates/research-tools/src/lessons_capture_tests.rs`**, **`crates/research-tools/src/lib.rs`**:
  - Enforced `MAX_LESSON_CHARS = 300` (Unicode scalar count), control-char stripping, non-empty validation, and validated `from: NodeRef` origin provenance.
  - Modeled scopes (`global`, `mode`, `field`, `mode_field`, `paper`) with `LessonScope::default_for_correction` defaulting to compound `(mode, field)` (`"this mode and this field"`, gracefully falling back to `mode` when `field` is absent).
  - Implemented `build_remember_draft` (0 model calls, prefilling user's own words clipped via `clip_to_chars`) and opt-in `build_summary_prompt` / `finalize_model_summary_draft` tagging output with `" (model)"` under `<= 300` Unicode chars.
  - Implemented opt-in (`OptInLessonsExport { include_lessons: bool }`) SHA-256 canonical-JSON checksummed backup (`export_lessons_backup`) and tamper-rejecting restore (`restore_lessons_backup`, bounded by `MAX_LESSONS_B_BACKUP_BYTES = 16 MiB`).

### Checkpoint 2 — Deterministic Prompt Selection & Solve Leg-Start Wiring (`52329c5`)
- **`crates/research-tools/src/lessons_prompt.rs`**, **`crates/research-tools/src/lessons_prompt_tests.rs`**:
  - Implemented zero-model deterministic selection (`select_lessons_for_run`, `select_lessons_with_caps`) ranking `(specificity_desc, created_ms_desc, id_asc)` under fixed caps (`MAX_SUPPLIED_LESSONS = 8`, `MAX_SUPPLIED_LESSONS_CHARS = 1800`).
  - Formatted notes strictly as quoted data under `LESSONS_PROMPT_HEADING` (`"Notes from the user's earlier corrections"`) with delimiter/prompt-injection neutralization (`sanitize_lesson_for_quote`, boundary instruction).
  - Resolved `lessons.supplied` payload duality between uneditable `agent::resume_brief` (reads inline `ev.payload["blob"]` text lines) and `ledger::verify::verify_event_blobs` (tracks `raw_blob` SHA-256 reference so `verify_all().unreferenced_blobs == 0`).
  - Added ledger-only replay reader (`read_supplied_lessons`, never opens live `lessons.db`), disclosure string (`supplied_notes_disclosure` -> `"used N of your notes"`), and mode-neutral `supply_lessons_to_ledger` helper for Explore/Investigate.
- **`crates/agent/src/solve_loop.rs`**, **`crates/agent/src/solve_loop_tests.rs`**:
  - Wired fresh `run_solve_loop` leg start to select matching lessons (skipping gracefully if `<data_dir>/library/lessons.db` does not exist), inject the quoted section into `system_prompt`, emit `lessons.supplied` via `Audit`, and increment `uses` once.
  - Ensured `resume_solve_loop` and 60% auto-checkpoints do not re-read live `lessons.db` or re-increment `uses`, relying solely on the frozen ledger brief (`build_resume_brief`). `resume_brief.rs` was not modified.

### Checkpoint 3 — Authenticated Routes & Zero-`innerHTML` UI (`556705e`)
- **`crates/desktop-server/src/routes_lessons_ui.rs`**, **`crates/desktop-server/src/routes_lessons.rs`**, **`crates/desktop-server/src/lib.rs`**, **`crates/desktop-server/src/solve_view.rs`**, **`crates/desktop-server/tests/lessons_b_routes.rs`**:
  - Added `/api/lessons/notes` (`GET`, `POST`), `/api/lessons/notes/{id}` (`PUT`/`PATCH`, `DELETE`), `/api/lessons/notes/prefill` (`POST`, 0 model calls), `/api/lessons/notes/draft-summary` (`POST`, explicit opt-in `(model)` draft via `agent_models::resolve_in_core_role_model`), `/api/lessons/notes/backup?confirm=true` (`GET`), `/api/lessons/notes/restore` (`POST`), and ledger-only `/api/lessons/supplied/{session_id}` (`GET`), all gated by `AuthGuard::authorize_api` via `api_dispatch.rs`.
  - Exposed frozen `"lessons_supplied"` metadata (`count`, `disclosure`, `records`) on `solve_view::build_solve_view` directly from `&[Event]` with zero live DB reads.
  - Added `lessons_b_routes.rs` testing CRUD, 300-char rejection, opt-in `?confirm=true` backup/tamper-rejection/restore, opt-in mock transport summary drafting, `AuthGuard` coverage, and byte-identical `agent::resume_brief::build_resume_brief` after live DB mutation and file deletion.
- **`crates/desktop-server/assets/lessons.js`**, **`crates/desktop-server/assets/lesson_remember.js`**, **`crates/desktop-server/src/assets.rs`**, **`crates/desktop-server/assets/new_menu.js`**, **`crates/desktop-server/assets/mode_stages.js`**, **`crates/desktop-server/assets/solve_why.js`**:
  - Built `"What I have learnt"` manager (`lessons.js`), mounted via `#open-lessons-btn` in `.rail-footer` (`new_menu.js`) without altering `MODE_MENU_ENTRIES` length `4`, displaying scope, clickable origin `NodeRef`, created date, uses, live `N / 300` edit counter, two-step delete, and opt-in backup/restore.
  - Built `"Remember this?"` card and `"used N of your notes"` expandable disclosure (`lesson_remember.js`), mounted in `solve_why.js` on step correction and `mode_stages.js` on Solve view load, strictly using `createElement` / `textContent` / `replaceChildren` (zero `innerHTML`).

### Checkpoint 4 — Property Tests & Node UI Tests (`29ccc5c`)
- **`crates/research-tools/tests/lessons_r24_props.rs`**:
  - 64-trial `xorshift64` property test confirming `SolveState`, `compute_all_tiers`, and `compute_all_badges` are 100% invariant to adversarial `lessons.supplied` events (R24/V10).
  - Investigate 6.22 invariance test confirming `fold_map` is identical with populated `lessons.db` + `lessons.supplied` vs empty.
  - Set K Pairs 5 & 6 tests covering dismissal (0 saved rows), approval -> S2 supply (`uses` 0->1) -> deletion -> S3 non-supply + S2 ledger-only replay invariance, and scope filtering + fixed selection caps.
- **`crates/desktop-server/tests/lessons_ui.test.mjs`**:
  - Covers `"What I have learnt"` CRUD/300-char guard/opt-in backup/origin links, `"Remember this?"` user-words prefill + `(mode, field)` default + non-auto model draft, `"used N of your notes"` disclosure, asset registration check, and zero-`innerHTML` XSS audit.

## Changed Files & Line Counts (`<= 600` Ceiling Verified)

| File | Lines | Role |
|---|---:|---|
| `crates/ledger/src/lessons_db.rs` | 565 | Schema version 2 migration hook (`cur < 1`, `cur < 2`) |
| `crates/ledger/src/lessons_b_store.rs` | 335 | F16b `user_lesson` SQLite table & CRUD/uses/restore |
| `crates/ledger/src/lessons_b_store_tests.rs` | 216 | Store CRUD & v1 -> v2 migration tests |
| `crates/ledger/src/lib.rs` | 528 | Module export `lessons_b_store` & `UserLessonRow` |
| `crates/research-tools/src/lessons_capture.rs` | 505 | Domain types, 300-char validation, `UserLessonsStore` |
| `crates/research-tools/src/lessons_draft.rs` | 130 | Zero-model prefill & opt-in `(model)` draft helper |
| `crates/research-tools/src/lessons_backup.rs` | 90 | Opt-in SHA-256 checksummed JSON backup/restore |
| `crates/research-tools/src/lessons_capture_tests.rs` | 143 | Validation, prefill, `(model)`, backup/restore tests |
| `crates/research-tools/src/lessons_prompt.rs` | 471 | Deterministic selection, quoting, ledger freeze/replay |
| `crates/research-tools/src/lessons_prompt_tests.rs` | 141 | Ranking, injection neutralization, live-DB delete replay |
| `crates/research-tools/src/lib.rs` | 367 | Module exports for F16b capture/draft/backup/prompt |
| `crates/research-tools/tests/lessons_r24_props.rs` | 435 | R24/V10/6.22 invariance & Set K Pairs 5–6 tests |
| `crates/agent/src/solve_loop.rs` | 527 | Fresh leg-start injection & resume-brief isolation |
| `crates/agent/src/solve_loop_tests.rs` | 497 | Solve supply + single `uses` increment + frozen resume |
| `crates/desktop-server/src/lib.rs` | 260 | Module export `routes_lessons_ui` |
| `crates/desktop-server/src/routes_lessons.rs` | 382 | Delegates `/api/lessons/notes..` & `/supplied..` |
| `crates/desktop-server/src/routes_lessons_ui.rs` | 340 | F16b HTTP API & opt-in `(model)` draft endpoint |
| `crates/desktop-server/src/assets.rs` | 522 | Registers `/assets/lessons.js`, `/assets/lesson_remember.js` |
| `crates/desktop-server/src/solve_view.rs` | 591 | Exposes ledger-only `lessons_supplied` on Solve view |
| `crates/desktop-server/tests/lessons_b_routes.rs` | 279 | Route, AuthGuard, mock draft, `build_resume_brief` invariance |
| `crates/desktop-server/assets/lessons.js` | 453 | `"What I have learnt"` page & rail nav button |
| `crates/desktop-server/assets/lesson_remember.js` | 331 | `"Remember this?"` card & `"used N of your notes"` widget |
| `crates/desktop-server/assets/new_menu.js` | 293 | Auto-mounts `#open-lessons-btn` in `.rail-footer` |
| `crates/desktop-server/assets/mode_stages.js` | 277 | Mounts `used N of your notes` disclosure on Solve |
| `crates/desktop-server/assets/solve_why.js` | 530 | Opens `"Remember this?"` after step correction submit |
| `crates/desktop-server/tests/lessons_ui.test.mjs` | 445 | F16b UI, XSS, char-cap, opt-in draft/backup tests |

## Coordinator Verification Results (Post-Merge `717879c`, Commits `9a92393`..`70121fb`)

- `py -3 scripts/check_file_length.py --base origin/gemini/work/fixed`: **PASSED** (`No touched source file is over 600 lines.`, `solve_view.rs` = 591 lines)
- Frozen files audit (`git diff --name-only origin/gemini/work/fixed...HEAD`): **0** frozen files touched (`Cargo.lock`, `crates/mcp-server/Cargo.toml`, `tool_profile.rs`, `solve_defs.rs`, `tests/golden/`, `runner.rs`, `transport.rs`, `honesty.rs`, `http_srv.rs`, `resume_brief.rs`, `solve_steps.js`, `index.html`, `app.js`, `.github/`, `RULES.md` all untouched).
- `node --check` on all `crates/desktop-server/assets/*.js` + `lessons_ui.test.mjs` (`86` files): **PASSED** (`0` syntax errors)
- `node --test crates/desktop-server/tests/*.test.mjs`: **PASSED** (`244` passed, `0` failed, `0` skipped — `4/4` in `lessons_ui.test.mjs` + `240/240` existing UI tests)
- Windows `cargo check --workspace --all-targets --locked` & `cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or`: **PASSED** (`0` warnings/errors after `9a92393`); Windows targeted checks (`lessons_r24_props` `3/3`, `agent solve_loop` `3/3`, `lessons_b_routes` `3/3`): **PASSED**
- Non-root WSL2 Ubuntu (`verify_f16b.sh`, after `e45dc38` & `70121fb`):
  - `cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or`: **PASSED** (`0` warnings)
  - Targeted F16b suites (`ledger --lib`, `research-tools --lib --test lessons_r24_props`, `agent --lib`, `desktop-server --test lessons_b_routes`): **5 suites, 459 passed, 0 failed, 0 ignored**
  - Full `cargo test --workspace --locked --no-fail-fast -j 2`: **172 suites, 1,310 passed, 0 failed, 2 ignored** (`=== VERIFY_F16B_DONE ===`)
