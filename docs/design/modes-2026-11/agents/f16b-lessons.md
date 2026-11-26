# Brief f16b-lessons — User-Approved Lessons Shown to Models (F16b)

**Status:** Queued right behind **`S12 + S13`** (`solve-s12-loop` + `solve-s13-s17-ui-intake`). F16a (`lessons_db.rs`, `lessons_schema.rs` `324`L, `lessons.rs` `495`L, `lessons_select.rs` `322`L, `routes_lessons.rs` `369`L) is ALREADY merged on `gemini/work/fixed`! Zero `Cargo.lock` edits and zero tool definition edits!
**Push branch:** `gemini/modes/f16b-lessons`

---

## 1. Package & Governing Design

- **Package:** **F16b** — explicit `"Remember this?"` save (`<= 300` Unicode chars, user's own words prefilled; optional model summary strictly opt-in & tagged `"(model)"`), compound `(mode, field)` default + `{global, mode, field, paper}` scopes, forward-only `lessons.db` migration in `crates/ledger/src/lessons_b_store.rs`, deterministic selection by code + `"Notes from the user's earlier corrections"` quoted prompt injection frozen into `lessons.supplied { run_id, lesson_ids, blob }`, `"used N of your notes"` disclosure, `"What I have learnt"` page (`assets/lessons.js`), and Set K R24 invariance tests (`lessons_r24_props.rs`).
- **Governing design:** `00-overview.md` 0.1, 2.3 (`NodeRef`), 2.7, 2.14, 7.2 Set K, 7.3, 8.2 #5, 8.3 #7; `solve.md` 4.2, 6 (R24); `explore.md` 4.2, 6 (V10); `investigate.md` 4.4, 6.22; `agents/RULES.md`.

---

## 2. Verified Interfaces on `gemini/work/fixed`

- **F16a (`crates/ledger/src/lessons_db.rs` `PRAGMA user_version = 1`, `crates/research-tools/src/{lessons_schema,lessons,lessons_select}.rs`, `desktop-server/src/routes_lessons.rs`):** Keep existing F16a reuse logic green; add new F16b CRUD/migration in sibling `crates/ledger/src/lessons_b_store.rs` (`rusqlite` stays ONLY in `crates/ledger`!).
- **Solve R24 (`solve_loop.rs` & `resume_brief.rs` from S12; `solve_why.js` from S13):** `resume_brief.rs` already reads frozen `lessons.supplied` from the ledger (NEVER edit `resume_brief.rs`). Wire leg-start `lessons_prompt` call into `solve_loop.rs`, and hook `"Remember this?"` onto `solve_why.js` `onStepCorrected`. (Wire/expose identical mode-neutral hooks for Explore/Investigate cleanly.)
- **`assets.rs` (`<= 600`L):** Ensure `assets_fonts.rs` split (S13) keeps `assets.rs` well under 550L before adding `lessons.js` and `lesson_remember.js`.

---

## 3. Checkpoints — Push After Each!

1. **Checkpoint 1 (`ledger::lessons_b_store` + `research-tools::lessons_capture`):** Forward-only schema migration for F16b lessons, `<= 300` Unicode char validation (`quote_core::clip_to_chars`), `(mode, field)` default scope + single-axis scopes, prefilled user-word `RememberPromptDraft`, opt-in SHA-256 backup/export filter.
2. **Checkpoint 2 (`lessons_prompt.rs` + `solve_loop.rs` wiring):** Deterministic selection + quoted `"Notes from the user's earlier corrections"` block + `lessons.supplied` blob freezing + `uses` counter update only on fresh runs + ledger-only replay reader.
3. **Checkpoint 3 (`routes_lessons_ui.rs` + `assets/lessons.js` + `assets/lesson_remember.js`):** Authenticated `/api/lessons/..` CRUD, optional `"Draft summary (model)"`, mount `"What I have learnt"` via `new_menu.js`, mount `"Remember this?"` on `solve_why.js` `onStepCorrected`, `"used N of your notes"` widget. Zero `innerHTML`!
4. **Checkpoint 4 (`lessons_r24_props.rs` + `lessons_ui.test.mjs`):** Set K Pairs 5–6, R24/V10/6.22 invariance property suite, byte-identical resume brief test after live `lessons.db` deletion/mutation, XSS tests. Write `docs/design/modes-2026-11/reports/f16b-lessons.md` and push to `gemini/modes/f16b-lessons`. End with `PUSHED`.

---

## 4. Owned Files & Do-Not-Edit (`<= 600` Lines Each)

- **New:** `crates/ledger/src/lessons_b_store.rs` (+ `_tests.rs`), `crates/research-tools/src/{lessons_capture,lessons_prompt}.rs` (+ `*_tests.rs`), `crates/research-tools/tests/lessons_r24_props.rs`, `crates/desktop-server/src/routes_lessons_ui.rs`, `crates/desktop-server/tests/lessons_b_routes.rs`, `crates/desktop-server/assets/{lessons,lesson_remember}.js`, `crates/desktop-server/tests/lessons_ui.test.mjs`.
- **Small edits:** `crates/ledger/src/lib.rs`, `crates/research-tools/src/lib.rs`, `crates/agent/src/solve_loop.rs`, `crates/desktop-server/src/{lib,assets,routes_lessons,api_dispatch}.rs`, `assets/{new_menu,solve_why}.js`.
- **Do NOT edit:** `solve_status.rs`, `solve_own_tier.rs`, `solve_badge.rs`, `resume_brief.rs`, `http_srv.rs`, `routes_session.rs`, `index.html`, `app.js`, `resume.rs`, `turn_runner.rs`, `Cargo.lock`, `tool_defs.txt`, `tool_profiles.txt`.

---

## 5. Mandatory Verification Before Every Push (`RULES.md`)

```bash
python3 scripts/check_file_length.py --base origin/gemini/work/fixed
for f in crates/desktop-server/assets/*.js; do node --check "$f"; done
node --test crates/desktop-server/tests/*.test.mjs
unset RE_SEARCH_PYTHON
cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or
cargo test --workspace --locked --no-fail-fast
```
