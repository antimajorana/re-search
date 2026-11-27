# Brief inv-v15-share — Investigate `.rsmap` and Standalone HTML Export / Import (V15)

**Status:** Ready / Unblocked (`V2–V8`, `V10`, `V13`, `V17`, `F5` merged on `gemini/work/fixed`). File-disjoint from `inv-v16-handoff` and `inv-v12-context`! Zero `Cargo.lock` edits!
**Push branch:** `gemini/modes/inv-v15-share`

---

## 1. Package & Governing Design Sections

- **Package:** **V15** — machine-exchangeable `.rsmap` JSON export, standalone read-only `.html` export with copyright/licence & private-notes guards, fail-closed `.rsmap` import that re-matches every quote against the local source (rule 6.11), and UI export/import controls.
- **Governing design:**
  - `00-overview.md`: 0.1 (V14 deferred; F15 write-up is separate), 2.3 (F2 `PaperRef`, `NodeRef`), 2.5 (F4 `verify_quote`, 300-char quote cap), 2.13 (F15 reading-notes write-up is separate from `.rsmap`/`.html`), 3.2 row 20 (zero `"verified"` anywhere in Investigate).
  - `investigate.md`: 0.2 #8 (HTML export copyright rule), 3 (stage L Export/import), 4.4 (`investigation.exported`, `investigation.imported`), 5.2 (`GET .../investigation/export`, `POST .../investigation/import`), rule 6.11, sections 7, 9, 12 (copyright in exports), 13 (#9 opt-in statements/notes).
  - **Owner decision:** Reading notes (`.md`/`.tex`) live strictly in F15 (`writeup_paper.rs`); V15 delivers `.rsmap` and standalone `.html` ONLY (`format=md` returns a clear 400 pointing to F15 write-up).

---

## 2. Disjoint File Ownership (`<= 600` Lines Per File)

- **New Rust modules (owned solely by V15):**
  - `crates/export/src/paper_map.rs` (schema v1 `.rsmap` JSON: `paper_key`, `version`, `builder`, `files_root_sha256`, `licence`, skeleton summary, gists, authors' claims, model nodes/edges, optional `notes`, optional `context_cards: []` ready for V12, quotes clamped `<= 300` chars via `quote_core::clip_quote`, full statements included **only** when paper licence is openly licensed CC-BY/CC0/public-domain OR caller explicitly passes `include_full_statements: true`, notes included **only** when `include_notes: true`).
  - `crates/export/src/paper_map_html.rs` (pure standalone HTML renderer: zero `<script>`, zero external CSS/fonts/CDNs, strict HTML escaping of every untrusted string, same copyright/private-notes gates, `"(model)"` suffixes, F6 provenance border cues, zero `"verified"`).
  - `crates/export/src/paper_map_import.rs` (rule 6.11 fail-closed import: locate matching local source via `arxiv` id + version in F10 library/cache OR file matching `files_root_sha256` / raw hash, write host-only `source.ingested` + rebuilt `investigation.skeleton` blob so `Ledger::verify_all().unreferenced_blobs == 0`, re-verify every imported item/edge/claim/note against the local source via `map_reading` + F4 `verify_quote`, drop failing items cleanly and count `items_matched` / `items_dropped`, tag surviving items `"imported"`, or open read-only with `source_match: false` and banner `"quotes not matched on this machine"` when no local source exists — NEVER `"not verified"`!).
  - `crates/export/tests/paper_map_share.rs` (round-trip export->import, tampered quote drop, missing local source banner, copyright/privacy gates, HTML XSS escaping, `Ledger::verify_all()`).
  - `crates/desktop-server/src/investigation_share.rs` (`GET /api/sessions/{id}/investigation/export?format=rsmap|html&include_statements=0|1&include_notes=0|1` -> streams bytes with `Content-Disposition`, appends `investigation.exported { format, sha256 }`; `POST /api/sessions/{id}/investigation/import` -> runs rule 6.11 import, appends `investigation.imported { format, sha256, source_match, items_matched, items_dropped }`, returns summary for UI banner `"Imported, N quotes could not be matched"`).
  - `crates/desktop-server/tests/investigation_share_http.rs` (HTTP auth + export/import round-trip + reject `format=md`).
- **New UI & Node test files (strictly disjoint from V16's `investigation_handoff_ui.js`!):**
  - `crates/desktop-server/assets/investigation_share_ui.js` (V15 export/import modal/panel with explicit opt-in checkboxes for full statements and private notes + read-only `"quotes not matched on this machine"` banner; clean mount helper `mountInvestigationShare(...)`; zero `innerHTML`).
  - `crates/desktop-server/tests/investigation_share_ui.test.mjs` (`node --test`).
- **Shared 1-line registrations ONLY:**
  - `crates/export/src/lib.rs`: add `pub mod paper_map;` (re-export HTML/import helpers).
  - `crates/desktop-server/src/lib.rs`: add `mod investigation_share;`.
  - `crates/desktop-server/src/api_dispatch.rs`: add 2 arms for `GET /api/sessions/{id}/investigation/export` and `POST /api/sessions/{id}/investigation/import`.
  - `crates/desktop-server/src/assets.rs`: register `/assets/investigation_share_ui.js` + `/assets/api.js` helper functions (`exportInvestigation`, `importInvestigation`) if helpful (`<= 20` lines in `api.js`).

---

## 3. Do-Not-Edit & Verification (`RULES.md`)

- **Do NOT edit:** `crates/export/src/{latex.rs, fork.rs, handoff.rs, handoff_view.rs, handoff_types.rs}`, `crates/desktop-server/src/{handoff.rs, investigation_handoff.rs, turn_runner.rs, agent_models.rs, http_srv.rs, routes_session.rs, routes_tools.rs}`, `crates/desktop-server/assets/{investigation_node.js, investigation_handoff_ui.js, investigation_badges.js, index.html, app.js}`, `crates/research-tools/tests/golden/*`, `Cargo.lock`.
- Checkpoints: CP1 (`crates/export` `paper_map*`), CP2 (`desktop-server` `investigation_share.rs` + UI + HTTP/Node tests). Push to `gemini/modes/inv-v15-share` after each!
- Run non-root (`id -u != 0`, `unset RE_SEARCH_PYTHON`) Linux checks (`check_file_length.py`, `node --check`, `node --test`, `clippy --workspace --all-targets --locked`, `cargo test --workspace --locked`), write `docs/design/modes-2026-11/reports/inv-v15-share.md`, and end with `PUSHED`.
