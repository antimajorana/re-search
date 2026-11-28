# Brief exp-x17-library — Full Library Pane UI & JSON/BibTeX Export with Coverage (Package X17)

**Split note:** Split out from `draft-exp-x17-x18-lib-cli.md` into an independent, file-disjoint lane from **`exp-x18-cli`**, **`exp-x15-chat`**, **`exp-x16-cite`**, and **`exp-x12-x13-map-handoff`** (and does NOT wait on Solve `S15`!).

## 1. Package and governing design sections

- **Package:** `X17` (Full Library pane UI over F10 `/api/library` + F13a `/api/sources` + F13c `library_send_back.js`, and deterministic Explore JSON/BibTeX export `crates/export/src/idea.rs` with F12 coverage & Rule V7 verdict recomputation).
- **Governing design:**
  - `docs/design/modes-2026-11/00-overview.md`: sections 2.7 (F10 global library), 2.11 (F13a/b/c sources & opt-in send-back), 2.13 (F15 write-up is OUT OF SCOPE here — owned by `f15-writeup-explore`), 3.2 row 20 (`"verified"` forbidden in Explore views/exports), 5.1, and 7.3.
  - `docs/design/modes-2026-11/explore.md`: sections 4.3, 5.4, 6 (Rules V7, S5, S8), 9 (row X17), and 12 (risk 10).
  - `docs/design/modes-2026-11/agents/RULES.md`.

## 2. Start conditions and verified base interfaces

- **Required merge on `gemini/work/fixed` before starting:** **`exp-x9-x10-x11-read-verdict`** (needed for `idea_verdict::compute`, `idea.map`, `evidence.quote`, `idea.directions` optional).
- **Already merged on `gemini/work/fixed` — verified directly on repo:**
  - `crates/desktop-server/src/routes_library.rs` (`287`L): `GET /api/library` (filters `q`, `title`, `author`, `year`, `key`, `status`, `role`, `limit`, `offset`), `GET /api/library/{key}`, `POST/DELETE /api/library/{key}/mark`, `POST /api/library/{key}/attach`, `DELETE /api/library/{key}`, `POST /api/library/rebuild`, `GET /api/library/marks/backup`, `POST /api/library/marks/restore`. **CRITICAL:** `http_srv.rs` decodes `'+'` to `' '` in URL paths — ALWAYS wrap every `PaperKey` with `encodeURIComponent(key)`!
  - `crates/desktop-server/src/routes_sources.rs` (`272`L, F13a): `GET /api/sources`, `POST /api/sources` (preview vs `accept: true`), `GET/DELETE /api/sources/{id}`, `POST /api/sources/{id}/sync`, `GET /api/sources/{id}/items`, `POST /api/sources/sync`, `GET /api/sources/duplicates`.
  - `crates/desktop-server/src/routes_sources_ext.rs` (`254`L, F13c): `POST /api/sources/export/bibtex`, `POST /api/sources/send/kandinsky`.
  - `crates/desktop-server/assets/library_send_back.js` (`339`L, F13c): `mountPaperSendBackActions(slotEl, paper, { getToken })`.
  - **NOTE:** `assets/library_sources.js` does NOT exist yet on `gemini/work/fixed` (`F13a` delivered backend `/api/sources` routes, while `F13c` added `library_send_back.js` in `project_library.js`). Build user-sources management (`add/preview/accept`, `sync`, `duplicates`, `remove`) directly inside `X17` (`assets/library.js` + optional sibling `assets/library_sources.js` if splitting to stay `<= 600`L) and compose `mountPaperSendBackActions` cleanly!
  - `crates/desktop-server/assets/api.js` (`465`L): `saveProjectSource(id, payload)` already exists for `"Save to project"` — **do NOT edit `projects.rs` (806L) or `routes_tools.rs` (610L)!**

## 3. Smallest implementation checkpoints

1. **Checkpoint 1 — `export/src/idea.rs` & `routes_idea_export.rs`:**
   - `crates/export/src/idea.rs` (new, `<= 600`L; sibling `idea_tests.rs` if needed):
     - Deterministic JSON and BibTeX export (`export_idea_json`, `export_idea_bibtex`) of a hash-chain-verified Explore session (`Ledger::read_verified_session`):
       - Recomputes F12 `Coverage` and per-part `Verdict` (`idea_verdict::compute`) from ledger events on export and rejects mismatches against stored `idea.verdict` fail-closed (Rule V7).
       - JSON export: original idea statement, sharpened idea, candidates, papers read (`text_basis`, `origin`), verified/clipped matched quotes (`<= 300` chars), concept map, verdicts, optional directions, and full F12 `Coverage` record.
       - BibTeX export: prefers user's `source_item.citekey` from F10/F13 `Library` via read-only lookup (`export::writeup_bib::resolve_bib_entry` or `library.rs`), falling back cleanly when `library.db` is deleted (Rule S5), with header metadata citing searched scope.
       - Rule S8 check: zero `"verified"`, `"new"`, or `"novel"` anywhere in exported JSON/BibTeX wording fields.
   - `crates/desktop-server/src/routes_idea_export.rs` (new, `<= 600`L): serves `GET /api/sessions/{id}/idea/export?format=json|bib` behind `AuthGuard::authorize_api`, registered via a single 1-line arm in `api_dispatch.rs` (**do NOT edit `routes_idea.rs` or `routes_tools.rs`**).
   - Keep **`export/src/lib.rs`** edits minimal (`pub mod idea;`, **do NOT edit `writeup*.rs` or `writeup_session_*`** — F15 adapters remain completely separate!).

2. **Checkpoint 2 — Full Library pane UI (`assets/library.js`):**
   - `crates/desktop-server/assets/library.js` (new, `<= 600`L; split F13a source panel into `assets/library_sources.js` `<= 600`L if approaching `540`L):
     - Search/filter over `GET /api/library` (`q`, `title`, `author`, `year`, `status`, `role`), paper list & detail drawer (`source_item` provenance, `owned` badge, matched findings, sessions where seen), mark controls (`unread | skimmed | key | irrelevant`, tags, notes), `"Attach PDF"` upload action, `"Rebuild index"` button, mark backup download & restore, `"Save to project"` via existing `saveProjectSource`, F13a `/api/sources` source management (preview -> accept, sync, duplicates), and per-paper `mountPaperSendBackActions` (`Export BibTeX`, `Send to Kandinsky`) from `/assets/library_send_back.js`.
     - Always `encodeURIComponent(key)`. Zero raw `innerHTML` with untrusted paper text; zero `"verified"`.
   - Register `/assets/library.js` (and `/assets/library_sources.js` if created) in `crates/desktop-server/src/assets.rs`. Keep `app.js` (932L) and `index.html` untouched (document home-screen mount snippet in `Merge notes`).

## 4. Owned files & Do-Not-Edit (`<= 600` lines each)

- **New:** `crates/export/src/idea.rs` (+ `idea_tests.rs`), `crates/export/tests/idea_export.rs`, `crates/desktop-server/src/routes_idea_export.rs`, `crates/desktop-server/assets/library.js` (+ optional `library_sources.js`), `crates/desktop-server/tests/library_ui.test.mjs`.
- **Small edits:** `crates/export/src/lib.rs` (`pub mod idea;`), `crates/desktop-server/src/{lib.rs,api_dispatch.rs,assets.rs}`.
- **Do NOT edit:** `crates/export/src/{latex.rs,writeup*.rs}`, `routes_idea.rs`, `routes_idea_net.rs`, `routes_tools.rs` (610L), `projects.rs` (806L), `http_srv.rs` (702L), `app.js` (932L), `index.html`, `cli_turn*.rs`, `tool_profile.rs`, `Cargo.lock`, `tests/golden/{tool_defs,tool_profiles}.txt`.

## 5. Required tests & hand-back

- `idea_export.rs` & HTTP `/api/sessions/{id}/idea/export`: auth enforced (`401`/`403` without header token), F12 coverage & Rule V7 verdict recomputation, stored `idea.verdict` mismatch rejected fail-closed, `source_item.citekey` preferred, `<= 300`-char quotes, survives `library.db` deletion (Rule S5), zero `"verified"`/`"new"`/`"novel"` (Rule S8).
- `library_ui.test.mjs`: search/filter, mark update, `"Save to project"`, `encodeURIComponent` with `/` and `+` in `PaperKey`, F13a `/api/sources` preview/accept/sync + F13c `mountPaperSendBackActions` mount, zero `"verified"`, XSS escaping.
- Verify before push as non-root (`RE_SEARCH_PYTHON` unset): `check_file_length.py`, `node --check` + `node --test`, `cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or`, `cargo test --workspace --locked --no-fail-fast`.
- Write `docs/design/modes-2026-11/reports/exp-x17-library.md` and push to `gemini/modes/exp-x17-library`.
