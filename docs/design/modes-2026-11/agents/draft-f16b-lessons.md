# Draft brief: `f16b-lessons` — User-approved lessons shown to models (F16b)

> **Status:** Draft brief prepared against base commit `832eab5bf856cec90b3a4a2f08f95827ec9d1acd` (`work/draft-next-lanes`). Do not dispatch until start conditions below are satisfied.

---

## 1. Package and governing design sections

- **F16b — User-approved lessons shown to models:**
  - Lets the user explicitly save a short note (`<= 300` Unicode chars) after an on-screen correction via **"Remember this?"** (user's own words prefilled and editable; an optional model-written summary is opt-in and never the default; nothing is learnt silently).
  - Selects applicable lessons deterministically **by code** (scope match, then most recent, up to a fixed count and total character budget — no model selects lessons).
  - Supplies lessons to a model strictly as **quoted data** under the heading `"Notes from the user's earlier corrections"`, freezes the exact supplied text in a ledger event **`lessons.supplied { run_id, lesson_ids, blob }`** so resume/replay is byte-identical even if `lessons.db` later changes, and displays `"used N of your notes"` (expandable to show those exact notes) on screen.
  - Provides the **"What I have learnt"** page (`assets/lessons.js`) listing every lesson with its origin (`NodeRef`), scope, and use count, with edit, delete, and backup/opt-in export controls.
  - Enforces **Rule R24 / V10 / 6.22 ("a lesson never decides anything")** via cross-mode property tests covering evaluation set K: for identical ledger events with and without lessons, every tier, badge, verdict, coverage gate, and quote-matching result is identical.
- **Governing design sections (with owner decisions of 2026-11-25):**
  - `00-overview.md` sections `0.1`, `2.1` (row F16), `2.3` (`NodeRef`), `2.7` (`lessons.db` separate from rebuildable `library.db`), `2.14` (F16b in full), `6` (build order), `7.2` Set K & `7.3` corrections pass rules, `8.1`, `8.2` #5 (user marks and lessons backed up), `8.3` #7 (**accepted:** kept only on "Remember this?", user's own words, optional model summary, default scope = this mode and this field, never changes a badge or verdict).
  - `solve.md` sections `4.2` (`lessons.supplied`), `6` (rule R24: identical tiers + byte-identical resume brief whatever live `lessons.db` holds), `8` (F16 row: S13 "Correct this step" + S12 resume brief).
  - `explore.md` sections `4.2` (`idea.note`, `lessons.supplied`), `4.4`, `6` (rule V10: `idea_verdict::compute` and coverage gate read no lessons), `9` (F16 row).
  - `investigate.md` sections `4.4` (`lessons.supplied`), `6.22` (corrections never decide a label).
  - `agents/RULES.md` and `agents/lib-f13a-f16a-f15.md` (F16a contract).

---

## 2. Start conditions and owner gates

| Gate | Condition |
|---|---|
| **Merge: F2, F6, F9, F10, S2** | **Already merged on base `832eab5bf`.** (`NodeRef`, shared UI parts, `RUN_ID_FIELD`, `library.rs`, `solve_model::event_kinds::LESSONS_SUPPLIED`, `solve_state.rs` / `solve_status.rs` R24 support.) |
| **Merge: F16a (`lib-f13a-f16a-f15`)** | **Must merge first.** Delivers `<data_dir>/library/lessons.db` (SQLite in `crates/ledger`), `crates/research-tools/src/{lessons.rs, lessons_schema.rs, lessons_select.rs}`, and `crates/desktop-server/src/routes_lessons.rs`. |
| **Merge: First mode correction UI + run host** | **Must merge first** (`00-overview.md` 2.14: *"after at least one mode records corrections on screen"*). Primary first target: **Solve S12** (`crates/agent/src/solve_loop.rs` leg runner; note S12's `resume_brief.rs` already reads `lessons.supplied` from the ledger and needs zero edits) + **Solve S13** (`assets/solve_why.js` "Correct this step" UI). Explore (`X6`/`X12`: `idea_exchanges.rs`, `idea_map.js`) and Investigate (`V6`/`V7`/`V10`: `investigation_tasks.rs`, `investigation_notes_ui.js`) adapters plug into the same mode-neutral contract as those packages merge and do **not** block F16b's start. |
| **Merge: F5 (`f5-f7-mode-shell`)** | Recommended before live HTTP/nav registration (`api_dispatch.rs`, `assets/new_menu.js`). If not merged, keep single-dispatcher routes in `routes_lessons*.rs` and document exact registration lines. |
| **Owner gate: Dependencies & tools** | **Zero new dependencies (`Cargo.lock` unchanged)** and **zero tool definition changes** (`tool_def_golden` and `tool_profiles` untouched — lessons are host-supplied quoted context, never a model tool). |
| **Owner gate: Scope schema reconciliation** | Verify the compound default scope (`"this mode and this field"`, `00-overview.md` 8.3 #7 vs 2.14's 4-variant list) against F16a's merged `lessons_schema.rs` before coding (see Section 8 conflict #1). |

---

## 3. Verified base interfaces vs future-merge interfaces

### Verified on base commit `832eab5bf` (use directly)
- **F2 (`crates/research-tools/src/paper_key.rs`):** `Mode` (`Research`, `Investigate`, `Explore`, `Solve`), `PaperKey`, `PaperRef`, `NodeRef` (required for `lesson.from`).
- **F4 (`crates/research-tools/src/quote_core.rs:116`):** `clip_to_chars(text, max_chars)` for safe UTF-8 boundary truncation (`<= 300` Unicode chars).
- **F9 (`crates/research-tools/src/run_budget.rs`):** `RUN_ID_FIELD` (`"run_id"`), consistent with `lessons.supplied.run_id`.
- **F10 backup pattern (`crates/research-tools/src/library_marks.rs`, 265 lines):** deterministic SHA-256-checksummed JSON envelope + rolling `.bak` rotation — reuse the exact security pattern for `<data_dir>/library/lessons.db` portable backup/restore (non-rebuildable user data, `00-overview.md` 8.2 #5).
- **S2 (`crates/research-tools/src/solve_model.rs:41`, `solve_state.rs`, `solve_status.rs`, `solve_status_tests.rs:485`):**
  - `solve_model::event_kinds::LESSONS_SUPPLIED = "lessons.supplied"` is **already defined**.
  - `SolveState::apply` already ignores `lessons.supplied` and `solve_status_tests.rs:485` already has a unit test checking `lessons.supplied` changes no tier or badge.
- **Crate boundary verified:** `rusqlite` lives **only** in `crates/ledger`; `crates/research-tools` must remain free of `rusqlite`; `crates/ledger` must remain free of `tokio`, `rmcp`, `windows-sys`.
- **F6 (`crates/desktop-server/assets/ui_badge.js`, `run_bar.js`):** shared badge/chip conventions (always icon + text, zero `innerHTML`).

### Future-merge interfaces (MUST verify before coding)
- **F16a (`lib-f13a-f16a-f15`):**
  - Exact SQLite module/schema version in `crates/ledger` for `<data_dir>/library/lessons.db` (check whether the `lesson` table already exists in F16a or needs a forward-only schema migration v1 -> v2).
  - Exact public types and functions in `crates/research-tools/src/{lessons.rs, lessons_schema.rs, lessons_select.rs}` and route dispatcher in `crates/desktop-server/src/routes_lessons.rs`.
- **F5 (`f5-f7-mode-shell`):**
  - `crates/desktop-server/assets/new_menu.js` (or Settings/navigation slot) to mount the `"What I have learnt"` page entry without editing `index.html` or `app.js`.
- **Solve S12 & S13:**
  - Exact leg-start hook in `crates/agent/src/solve_loop.rs` (S12) so a new leg selects lessons via `lessons_select.rs`, formats them as quoted data, and appends `lessons.supplied { run_id, lesson_ids, blob }`. Verify that S12's `resume_brief.rs` already reads `lessons.supplied` from the ledger blob as specified in `draft-solve-s12-loop.md` (**zero edits to `resume_brief.rs`**).
  - Exact `onStepCorrected` callback in S13's `assets/solve_why.js` to mount the `"Remember this?"` composer (`assets/lesson_remember.js`).
- **Explore X6/X12 & Investigate V6/V7/V10 (if merged when F16b runs):**
  - Exact exchange/task start hooks in `idea_exchanges.rs` and `investigation_tasks.rs`, plus correction UI callbacks in `assets/idea_map.js` and `assets/investigation_notes_ui.js`. If not yet merged, expose clean mode-neutral traits/helpers and document exact wiring points in Merge notes.

---

## 4. Smallest implementation checkpoints

Execute in order; commit and run full checks after each checkpoint:

1. **Checkpoint 1 — Lesson store CRUD, compound scope, and portable backup (`ledger::lessons_b_store` + `lessons_capture.rs`):**
   - Inspect F16a's `lessons.db` schema; add a forward-only migration if the F16b `lesson` table (or `mode`/`field` filter columns, `uses` counter, `from_node_ref_json`) is not yet present.
   - Enforce validation in code: `text` trimmed, non-empty, `<= 300` Unicode chars, control characters stripped; `from: NodeRef` validated against F2's `paper_key`; scope supports all four axes (`global`, `field`, `paper`, `mode`) AND the owner-approved default compound scope `(mode, field)` (falling back to `mode` when no field is set).
   - Implement `RememberPromptDraft` builder from a correction event (Solve "Correct this step", Explore `idea.note`, Investigate override/note), prefilling the **user's own words** by default (works with zero model calls). Add an opt-in helper for optional model summarization (strictly user-clicked, tagged `"(model)"`, editable before saving, never auto-called).
   - Implement SHA-256-checksummed portable backup/restore and an **opt-in** export filter (off by default unless the user explicitly ticks "Include my lessons").
2. **Checkpoint 2 — Deterministic selection, quoted prompt block, and `lessons.supplied` ledger freezing (`lessons_prompt.rs`):**
   - Deterministic selection by code (extending F16a's `lessons_select.rs`): filter strictly by scope match (`global`, matching `mode`, matching `field`, matching `paper_key`), sort by `(specificity_desc, created_ms_desc, id_asc)`, and cap at fixed named constants (`MAX_SUPPLIED_LESSONS`, e.g., `8`, and `MAX_SUPPLIED_LESSON_CHARS`, e.g., `1800`). No model is called.
   - Format selected lessons as **quoted data** under the exact heading `"Notes from the user's earlier corrections"`, escaping/neutralizing delimiter sequences so lesson text can never masquerade as system instructions.
   - Write the exact rendered block via `Ledger::put_blob`, append `lessons.supplied { run_id, lesson_ids, blob }` to the session ledger, and increment `uses` in `lessons.db` **only** when a fresh run supplies the lessons.
   - Provide `read_supplied_lessons(events, ledger, run_id) -> Option<SuppliedLessons>` for UI disclosure and replay verification: reads **exclusively** from the session's `lessons.supplied` event + content-addressed blob, never from live `lessons.db`, matching S12 `resume_brief.rs`'s byte-identical replay contract (R24).
3. **Checkpoint 3 — Mode wiring (`solve_loop.rs` first) and routes (`routes_lessons_ui.rs`):**
   - Wire `crates/agent/src/solve_loop.rs` at leg start to select lessons via `lessons_select.rs`, format them via `lessons_prompt.rs`, append `lessons.supplied`, and surface `"used N of your notes"` (leaving S12's `resume_brief.rs` untouched, since it already reads `lessons.supplied` from the ledger). Expose the same mode-neutral hook for later wiring in `idea_exchanges.rs` and `investigation_tasks.rs`.
   - Expose authenticated HTTP endpoints (under `/api/lessons/..`, extending `routes_lessons.rs` or delegating to `routes_lessons_ui.rs`): list all lessons (with origin, scope, use count), create (explicit user save), edit, delete, optional model-summary draft (explicit user action only), backup, and restore.
4. **Checkpoint 4 — "What I have learnt" page, "Remember this?" UI, and UI mount seams (`assets/lessons.js`, `assets/lesson_remember.js`):**
   - `assets/lessons.js`: lists every lesson with text, scope badge, origin chip (clickable `NodeRef`), created date, use count, inline edit (`<= 300` char live counter), delete with confirmation, and backup/restore buttons. Mount `"What I have learnt"` via F5's `assets/new_menu.js` (or Settings/navigation slot) so it is reachable without editing `index.html` or `app.js`.
   - `assets/lesson_remember.js`: `"Remember this?"` card (user's own words prefilled and editable, default scope `"this mode and this field"`, optional `"Draft summary (model)"` button never clicked by default) + `"used N of your notes"` disclosure widget showing the exact frozen notes for a run. Hook `"Remember this?"` via S13's `assets/solve_why.js` `onStepCorrected` callback (and later Explore `assets/idea_map.js` / Investigate `assets/investigation_notes_ui.js`).
   - Zero `innerHTML`; every user/model/origin string written via `textContent`.
5. **Checkpoint 5 — Cross-mode R24 invariance & Set K property suite (`lessons_r24_props.rs`):**
   - Prove R24/V10/6.22 across Solve (`solve_state`/`solve_status`), Explore verdict/coverage (if merged), and Investigate recorders (if merged), plus all Set K scripted session pairs.

---

## 5. Owned files (disjoint, <= 600 lines each)

| Owned file (new unless noted) | Purpose & crate boundary |
|---|---|
| `crates/ledger/src/lessons_b_store.rs` (+ `lessons_b_store_tests.rs`) | Forward-only migration & SQLite CRUD/use-count queries for F16b lessons in `<data_dir>/library/lessons.db` (keeps `rusqlite` in `ledger`, avoids bloating F16a's `lessons_db.rs`). |
| `crates/research-tools/src/lessons_capture.rs`, `lessons_capture_tests.rs` | `<= 300`-char validation, compound scope (`mode` + `field`), `"Remember this?"` draft builder, opt-in backup/export helpers. |
| `crates/research-tools/src/lessons_prompt.rs`, `lessons_prompt_tests.rs` | Deterministic selection, quoted `"Notes from the user's earlier corrections"` formatter, `lessons.supplied` writer, and ledger-only replay reader. |
| `crates/research-tools/tests/lessons_r24_props.rs` | Set K scripted pairs & R24/V10/6.22 invariance property tests. |
| `crates/desktop-server/src/routes_lessons_ui.rs`, `crates/desktop-server/tests/lessons_b_routes.rs` | HTTP handlers for F16b lesson CRUD, backup/restore, and explicit opt-in summary draft. |
| `crates/desktop-server/assets/lessons.js`, `assets/lesson_remember.js`, `crates/desktop-server/tests/lessons_ui.test.mjs` | "What I have learnt" page, "Remember this?" modal/card, `"used N of your notes"` disclosure; zero `innerHTML`. |

---

## 6. Shared registration seams and "Do not edit" files

### Minimal registration seams (each in its own small commit, listed under Merge notes)
- `crates/ledger/src/lib.rs`: export `lessons_b_store` (or wire into F16a's `lessons_db` if `< 540` lines).
- `crates/research-tools/src/lib.rs`: export `lessons_capture` and `lessons_prompt`.
- `crates/desktop-server/src/lib.rs`: `pub mod routes_lessons_ui;`.
- `crates/desktop-server/src/assets.rs` (365 lines): register `"/assets/lessons.js"` and `"/assets/lesson_remember.js"`.
- `crates/desktop-server/src/routes_lessons.rs` (delivered by F16a) or `api_dispatch.rs` (delivered by F5): single delegating arm to `routes_lessons_ui`.
- **Run-host seam:** `crates/agent/src/solve_loop.rs` (S12) leg-start hook calling `lessons_select.rs` + `lessons_prompt.rs` to append `lessons.supplied` (and later matching hooks in `idea_exchanges.rs` and `investigation_tasks.rs`). **Leave S12's `resume_brief.rs` untouched** — it already reads `lessons.supplied` from the ledger.
- **UI mount seams:** F5's `crates/desktop-server/assets/new_menu.js` (or Settings/navigation slot) to open `"What I have learnt"` (`assets/lessons.js`), and S13's `crates/desktop-server/assets/solve_why.js` `onStepCorrected` callback to open `"Remember this?"` (`assets/lesson_remember.js`) (plus later Explore `assets/idea_map.js` and Investigate `assets/investigation_notes_ui.js`).

### Explicit "Do not edit" files
- `Cargo.lock` and all `Cargo.toml` files.
- `crates/research-tools/tests/golden/tool_defs.txt` and `tool_profiles.txt` (zero tool changes).
- `crates/research-tools/src/solve_status.rs` / `solve_own_tier.rs` / `solve_badge.rs` (must continue reading **zero** lessons per R24 — never pass a `LessonsStore` into any tier/verdict/gate function!).
- S12's `resume_brief.rs` (already reads `lessons.supplied` from the ledger; zero edits needed).
- `crates/desktop-server/src/http_srv.rs` (702 lines), `routes_session.rs` (616 lines), `assets/index.html` (768 lines), `assets/app.js` (932 lines), `crates/agent/src/resume.rs`, `turn_runner.rs`.
- `crates/research-tools/src/library_schema.rs` (594 lines), `ingest.rs` (590 lines), `pdf_match.rs` (589 lines), `pdf_pages.rs` (569 lines).
- F13/F14/V17 files and F16a's code-only override/alias/ranking logic (keep F16a tests green untouched).

---

## 7. Tests through the entry point

All tests use strictly synthetic self-authored ledgers/corrections (zero third-party paper text, zero live network or model calls):

1. **R24 / V10 / 6.22 invariance property tests (`lessons_r24_props.rs` — CI blocking per `00-overview.md` 7.3):**
   - Over randomized Solve DAGs and event streams (reusing S2's deterministic xorshift64 style), compare `SolveState::from_events` and `solve_status::{compute_all_tiers, compute_all_badges}` on identical events **with** and **without** injected `lessons.supplied` events (including adversarial lesson text containing `"Verified"`, `"Passed"`, `"ignore previous instructions"`, `"override tier"`): assert **every tier, badge, flag, and gate result is 100% identical**.
   - Replay / resume immutability test (`solve.md` R24): emit `lessons.supplied` for a run, then edit and delete those lessons in live `lessons.db`, rebuild the resume brief twice, and assert **byte-identical output** matching the frozen ledger blob.
   - Where Explore (`idea_verdict::compute`) or Investigate recorders are merged, include matching invariance assertions confirming identical verdicts/gates with and without lessons.
2. **Evaluation set K scripted pairs (`lessons_r24_props.rs`):**
   - Pair 5: user approves a lesson via "Remember this?" in session 1 -> session 2 supplies the lesson in `lessons.supplied`, increments `uses` from `0` to `1`, and exposes `"used 1 of your notes"` with the exact text.
   - Pair 6: user deletes that lesson in "What I have learnt" -> session 3 does **not** supply it (while session 2's replay still sees its frozen `lessons.supplied` blob!).
   - Negative test: a correction event where the user dismisses "Remember this?" creates **zero** rows in `lessons.db` (nothing learnt silently).
   - Scope filtering & deterministic cap test: verify `global`, `mode`, `field`, `(mode, field)`, and `paper` filtering, deterministic ordering by recency, and fixed count/size caps.
3. **Security, backup, and opt-in export tests (`lessons_capture_tests.rs`, `lessons_b_routes.rs`):**
   - Lesson text `> 300` Unicode chars is rejected (or rejected on direct API POST with 400); prompt-injection payloads are strictly wrapped as quoted data under `"Notes from the user's earlier corrections"`.
   - `AuthGuard::authorize_api` + loopback `Origin` enforced on all `/api/lessons/..` routes.
   - Backup/restore round-trip verifies SHA-256 checksum and refuses tampered payloads; export excludes lessons unless the opt-in flag is explicitly `true`.
4. **UI tests (`lessons_ui.test.mjs`):**
   - `"What I have learnt"` renders origin, scope, use count, edit (enforcing 300-char limit), delete, and backup actions.
   - `"Remember this?"` prefills user's own words and defaults scope to `"this mode and this field"`; optional model summary button is never auto-triggered.
   - `"used 2 of your notes"` displays exact count and expandable notes.
   - XSS test: `<script>`, `<img onerror>`, `<svg onload>` inside lesson text or origin title produce **zero** `innerHTML` writes.

---

## 8. Document conflicts and resolutions

| # | Conflict | Exact sources | Resolution (`00-overview.md` wins) |
|---|---|---|---|
| 1 | Single-axis scope enum vs compound default `"this mode and this field"` | `00-overview.md` 2.14 (`scope: global \| field \| paper \| mode`) vs `00-overview.md` 8.3 #7 (**accepted 2026-11-25:** `"Default scope: this mode and this field"`) | Support both cleanly with structured scope (e.g., `scope_kind` + optional `mode`, `field`, `paper_key` filters) so `"Remember this?"` defaults to `(mode, field)` while allowing single-axis `global`, `mode`, `field`, or `paper` scopes. Reconcile with F16a's merged `lessons_schema.rs`. |
| 2 | `lessons.rs` listed under `research-tools` vs SQLite `rusqlite` boundary | `00-overview.md` 2.1 (`research-tools/src/lessons*.rs`) & 2.14 (`lessons.db`) vs `agents/f10-library.md` & `reports/f10-library.md` (`rusqlite` confined to `crates/ledger`) | Keep all SQLite access to `<data_dir>/library/lessons.db` in `crates/ledger` and typed API / selection / prompt formatting in `crates/research-tools`. |
| 3 | Editing/deleting a lesson in "What I have learnt" vs reproducible replay | `00-overview.md` 2.14 (edit/delete in live store) vs `solve.md` R24 (byte-identical resume brief whatever live store holds) | Live edits/deletes affect **future** runs only; past runs always read the immutable blob referenced by `lessons.supplied { run_id, lesson_ids, blob }`. |
| 4 | Model-written summary vs zero-model local default | `00-overview.md` 2.14 & 8.3 #7 | User's own words are always prefilled and editable (0 model calls); a model-written summary is an explicit opt-in button, tagged `"(model)"`, never default. |

---

## 9. Out of scope and hand-back

- **Out of scope:** Fine-tuning or training any model (`00-overview.md` 2.14); silent/automatic lesson creation; F16a code-only corrections (paper marks ranking, confirmed/rejected name aliases, Solve setup symbol/convention defaults, Investigate skeleton overrides, OCR line overlays — built in `lib-f13a-f16a-f15`); letting any lesson alter a badge, tier, verdict, coverage gate, or quote match; adding any new tool definition.
- **Hand-back:** Run `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`, `cargo test -p research-tools --test tool_def_golden`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --no-fail-fast`, and `node --check` / `node --test` on touched JS files. Write `docs/design/modes-2026-11/reports/f16b-lessons.md` and push only to `gemini/modes/f16b-lessons` under `agents/RULES.md`.
