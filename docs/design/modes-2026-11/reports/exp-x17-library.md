# Report: `exp-x17-library` (Package X17 — Global Library Pane UI & Deterministic Explore JSON/BibTeX Export)

- **Brief:** `docs/design/modes-2026-11/agents/exp-x17-library.md`
- **Base branch & commit:** `origin/gemini/work/fixed` at `ffee92f4bb0372a362b2e2781bb3c8da13b7cdaa`
- **Work branch:** `work/exp-x17-library`
- **Target remote branch:** `gemini/modes/exp-x17-library`
- **Commits on branch (excluding this report commit):**
  - `0c691ac` — `feat(export): add deterministic Explore JSON/BibTeX export with F12 coverage and Rule V7/C3/C8/S5/S8 checks`
  - `d9e624f` — `feat(desktop-server): add authenticated GET /api/sessions/{id}/idea/export route`
  - `456c0c3` — `feat(desktop-server): add global Library pane UI and F13a sources panel (X17)`

---

## 1. What Was Implemented (Package X17)

### Checkpoint 1 — Deterministic Explore JSON/BibTeX Export & HTTP Route

1. **`crates/export/src/idea.rs` (`532` lines, new)** and **`crates/export/src/idea_extract.rs` (`546` lines, new)** and **`crates/export/src/idea_tests.rs` (`121` lines, new)**:
   - Implements `export_idea_json(data_dir, session_id)` and `export_idea_bibtex(data_dir, session_id)` plus structured types (`IdeaExport`, `IdeaExportError`, `IdeaExportSummary`, `IdeaExportPaper`, `IdeaExportQuote`, etc.) over a hash-chain-verified Explore session (`Ledger::read_verified_session`).
   - Rejects non-Explore sessions fail-closed (`IdeaExportError::NotExploreSession`).
   - **Rule V7 (verdict recomputation):** Recomputes per-part verdicts via `research_tools::idea_verdict::compute` with `preset_limits` derived from `idea.started` and `idea.budget_raised`. Every stored `idea.verdict` event in the active revision is checked against `idea_verdict::compute` on its exact ledger prefix (`&events[..=idx]`) with unknown-field rejection and symmetric `map_seq` equality; an `idea.map` without a matching `idea.verdict` fails closed (`IdeaExportError::VerdictMismatch`). The exported `verdicts` reflect the full active revision head (including post-verdict `idea.note` `confirm_same`/`reject_same` notes).
   - **F12 Coverage:** Recomputes the full `Coverage` record (`research_tools::search_coverage::build_from_events`) directly from ledger events and includes it verbatim in JSON export and summarized in the BibTeX header comments.
   - **F4 Quote & Rule C3/C8 Verification:** Re-matches every `evidence.quote` via `verify_quote_ref` against `<data_dir>/sessions/<id>/sources/` (independent of `library.db`), failing closed if `path`, `file_sha256`, or `quote_sha256` is missing or not 64-hex. Enforces Rule C3 on `evidence.abstract` (`verbatim: true`, non-empty quote, valid `quote_sha256`, `start < end`, non-empty `provider` and `raw_blob`). Enforces Rule C8 on multi-line OCR PDF spans (`resolve_pdf_page_and_basis` + `pick_effective_basis` with OCR priority over overlapping `[start_line, end_line]` spans). Clips exported `quote`, `excerpt`, and `evidence.abstract.quote` to `<= 300` Unicode scalar values.
   - **JSON Payload:** Deterministic byte-for-byte output (`BTreeMap` / sorted keys) containing original idea statement, sharpened idea (folding `idea.brief` and `idea.answer` edits), candidates, papers read (`text_basis`, `origin`), matched quotes (`<= 300` chars), concept map, recomputed verdicts, optional `idea.directions`, and full F12 `Coverage`.
   - **BibTeX Export & Rule S5:** Resolves canonical keys across `key` + `ids` aliases, merges metadata across candidates / `origin_paper` / read papers / verdicts before deduplication, and prefers user `source_item.citekey` from F10/F13 `Library` when `<data_dir>/library/library.db` exists on disk. When `library.db` is missing or deleted, falls back deterministically without error and without creating `library.db` (Rule S5). Header metadata cites searched scope (`sources_searched`, `sources_skipped`, `date_cutoff`, `query_count`).
   - **Rule S8 Check:** Scans all exported wording fields (including `matched_quotes[*].display_grade`, `ocr_tag`, verdict labels, directions, and BibTeX header/notes) and fails closed (`IdeaExportError::RuleS8Violation`) if banned words (`verified`, `new`, `novel`) appear.
2. **`crates/export/src/lib.rs` (`383` lines, minimal edit):** Added `pub mod idea;`.
3. **`crates/export/tests/idea_export.rs` (`538` lines, new, `4` tests):** End-to-end integration tests covering deterministic JSON/BibTeX export, `source_item.citekey` preference, F4 quote re-verification + `<= 300`-char clipping, Rule S5 (`library.db` deletion), Rule V7 verdict mismatch / missing verdict / post-verdict note handling, tampered source/ledger rejection, Rule C3/C8, and Rule S8 rejection.
4. **`crates/desktop-server/src/routes_idea_export.rs` (`161` lines, new):** Serves `GET /api/sessions/{id}/idea/export?format=json|bib` behind `AuthGuard::authorize_api` (header token required; rejects cookie-only and foreign `Host`), setting `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`, and deterministic `Content-Disposition` attachment filenames (`explore-<id>.json` / `explore-<id>.bib`). Maps invalid format/session id to `400`, missing session to `404`, non-Explore/tampered/verdict-mismatch/Rule-S8 failures to `409`, and sets `Cache-Control: no-store` on error responses too.
5. **`crates/desktop-server/src/lib.rs` (`280` lines, minimal edit)** and **`crates/desktop-server/src/api_dispatch.rs` (`530` lines, minimal edit):** Wired `pub mod routes_idea_export;` and the 3-line `("GET", ["api", "sessions", id, "idea", "export"])` dispatch arm.
6. **`crates/desktop-server/tests/idea_export_route.rs` (`419` lines, new, `2` tests):** End-to-end HTTP server tests verifying `401` (missing token, wrong token, cookie-only), `403` (foreign `Host`), `400` (bad `format`), `404` (unknown session), `409` (verdict mismatch / tampered ledger), and `200` for both `format=json` and `format=bib` with `Cache-Control: no-store`, `Content-Disposition`, F12 coverage, `source_item.citekey`, and Rule S5 survival after deleting `library.db`.

### Checkpoint 2 — Global Library Pane UI & F13a Sources Panel

1. **`crates/desktop-server/assets/library.js` (`578` lines, new):**
   - Full global Library pane (`mountLibraryPane` / `createLibraryPane`) over F10 `/api/library`, F13a `/api/sources`, F13c `library_send_back.js`, and existing `saveProjectSource` (`api.js`).
   - Search & filters (`q`, `title`, `author`, `year`, `status`, `role`), paper list with `in your library` (`OWNED_BADGE_LABEL`), `full text cached` (`FULLTEXT_BADGE_LABEL`), mark status, and matched finding count badges.
   - Paper detail drawer with `source_item` provenance, sessions where seen (with optional `onOpenSession` callback), matched findings (`quote` + `(model)` gist attribution), mark/tags/notes form (`unread | skimmed | key | irrelevant`, save + clear), `"Attach PDF"` upload action (routed to `/api/sessions/{id}/idea/read` when active session mode is `explore`, or `/api/sessions/{id}/upload` otherwise, or `options.onAttachPdf`), `"Attach to session"` (gated on `detail.fulltext` containing a cached blob `raw_blob || file_sha256`), `"Save to project"` (emitting strict `projects.rs` `SourceInput` fields via `buildProjectSourcePayload`, preserving old-style arXiv slashes like `hep-th/9901001`, validating DOI format, and stripping control chars including `\x0c` PDF form-feeds), `"Remove from index"`, and per-paper `mountPaperSendBackActions` (`Export BibTeX`, `Send to Kandinsky`) from `./library_send_back.js`.
   - Top-level `"Rebuild index"`, `"Download mark backup"`, and `"Restore mark backup"` actions.
   - Wraps every `PaperKey` with `encodeURIComponent(key)`. Zero `innerHTML` writes anywhere. Zero `"verified"`, `"new"`, or `"novel"` in any UI label or constant (Rule S8). Visible error surfacing in `.library-pane-status` via `withStatusError`.
2. **`crates/desktop-server/assets/library_sources.js` (`506` lines, new):**
   - F13a `/api/sources` management panel (`createLibrarySourcesPanel`) plus shared zero-`innerHTML` DOM (`domEl`, `clearChildren`, `makeBtn`, `makeInput`, `makeSelect`) and token-authenticated JSON fetch helpers (`requestJsonWithToken`).
   - Implements explicit two-step preview (`accept: false`) -> accept (`accept: true`) registration for `kandinsky_vault`, `folder`, `bibtex`, and `zotero`; single-source and sync-all; duplicate analysis rendering matching exact Rust `DedupeAnalysis` (`exact_groups` / `possible_groups`, `signals`, `members`) and `SyncReport` (`duplicate_groups` array, `exact_duplicates` / `possible_duplicates`, `ScanWarning` `{ path, reason }`); and source removal.
3. **`crates/desktop-server/src/assets.rs` (`546` lines, minimal edit):** Registered `/assets/library.js` and `/assets/library_sources.js` (remains well under the 580-line split threshold in `RULES.md`).
4. **`crates/desktop-server/tests/library_ui.test.mjs` (`535` lines, new, `6` tests):** Covers `PaperKey`/`sourceId` `encodeURIComponent` with `/` and `+`, Rule S8 banned-word sweep, full Library pane workflow (search/filter, drawer, mark save/clear, Save to project, Attach PDF, rebuild, backup/restore, F13c send-back), `buildProjectSourcePayload` sanitization (old-style arXiv slash, `\x0c` stripping, DOI validation), F13a preview->accept/sync/duplicates/remove against exact Rust schemas with non-Array `HTMLCollection` fake DOM and XSS payload assertions (`innerHtmlWrites === 0`), visible error surfacing & cached-blob gating, and `assets.rs` registration.

### Measured Line Counts (All Touched/Created Files `<= 600` Lines)

| File | Lines | Status |
|---|---:|---|
| `crates/export/src/idea.rs` | 532 | new |
| `crates/export/src/idea_extract.rs` | 546 | new |
| `crates/export/src/idea_tests.rs` | 121 | new |
| `crates/export/tests/idea_export.rs` | 538 | new |
| `crates/export/src/lib.rs` | 383 | edited (+1 line) |
| `crates/desktop-server/src/routes_idea_export.rs` | 161 | new |
| `crates/desktop-server/tests/idea_export_route.rs` | 419 | new |
| `crates/desktop-server/src/lib.rs` | 280 | edited (+1 line) |
| `crates/desktop-server/src/api_dispatch.rs` | 530 | edited (+3 lines) |
| `crates/desktop-server/src/assets.rs` | 546 | edited (+8 lines) |
| `crates/desktop-server/assets/library.js` | 578 | new |
| `crates/desktop-server/assets/library_sources.js` | 506 | new |
| `crates/desktop-server/tests/library_ui.test.mjs` | 535 | new |

`python3 scripts/check_file_length.py --base origin/gemini/work/fixed` exits `0` (`No touched source file is over 600 lines.`). `Cargo.lock`, `crates/research-tools/tests/golden/tool_defs.txt`, and `crates/research-tools/tests/golden/tool_profiles.txt` are completely untouched.

---

## 2. Deviations from Design, Autonomous Decisions & Assumptions

1. **Module split for 600-line ceiling (`idea_extract.rs` and `library_sources.js`):**
   - Split ledger extraction, F4 quote re-verification, Rule C3/C8 checks, and BibTeX assembly into sibling `crates/export/src/idea_extract.rs` (`546`L) so `idea.rs` (`532`L) and `idea_extract.rs` both stay under 600 lines.
   - Split the F13a `/api/sources` panel and shared DOM/HTTP helpers into `crates/desktop-server/assets/library_sources.js` (`506`L), explicitly anticipated by the brief, keeping `library.js` at `578`L.
2. **Rule V7 verification across post-verdict user notes (`idea.note` `confirm_same` / `reject_same`):**
   - `idea_map_exchange.rs` writes `idea.verdict` atomically right after `idea.map`. If a user later appends an `idea.note` (`confirm_same` / `reject_same`), comparing the historical `idea.verdict` against the *head* ledger would falsely fail Rule V7.
   - **Decision:** Verify every stored `idea.verdict` in the active revision against `idea_verdict::compute(&events[..=idx], Some(preset_limits))` (with strict field & `map_seq` checks, and failing closed if `idea.map` lacks a stored `idea.verdict`), then emit the head-recomputed verdicts in the export.
3. **Rule S5 `library.db` lookup behavior:**
   - `open_library_if_present` checks `<data_dir>/library/library.db` existence before calling `Library::open`, so deleting `library.db` never breaks export and never recreates `library.db`.
   - **Deviation / caveat:** When `library.db` *already exists* on disk, `Library::open` (`crates/research-tools/src/library.rs:37`) delegates to `LibraryDb::open` (`crates/ledger/src/library_db.rs:31`), which opens SQLite read-write rather than `SQLITE_OPEN_READ_ONLY`. Neither file is owned by X17, so X17 avoids calling `Library::open` unless `library.db` exists and performs only read queries (`get_paper`, `source_items_for_paper`). See Merge Notes for an F10 follow-up to add `Library::open_read_only`.
4. **Relative ES module imports in `library.js`:**
   - Used relative imports (`./api.js`, `./library_send_back.js`, `./library_sources.js`, `./ui_badge.js`) instead of root-relative `/assets/*.js` so `library.js` resolves identically in both the desktop browser (`/assets/library.js`) and `node --test`.
5. **`Attach PDF` and `Attach to session` routing decisions:**
   - `Attach PDF` routes to `POST /api/sessions/{id}/idea/read` (`attach_pdf`) when `options.getActiveSessionMode()` returns `'explore'`, to `POST /api/sessions/{id}/upload` for other modes, or to `options.onAttachPdf` when provided.
   - `Attach to session` is only shown when `detail.fulltext` contains an entry with `raw_blob || file_sha256`, because `routes_library.rs:238-246` returns `404` (`no cached full-text blob`) for unhashed `user_library` rows.
6. **Named constants for owner-accepted decisions (`00-overview.md` §8):**
   - All UI labels and API bases (`LIBRARY_API_BASE`, `MARK_STATUSES`, `SEEN_ROLES`, `OWNED_BADGE_LABEL`, `FULLTEXT_BADGE_LABEL`, `ATTACH_PDF_LABEL`, `ATTACH_SESSION_LABEL`, `REBUILD_INDEX_LABEL`, `BACKUP_MARKS_LABEL`, `RESTORE_MARKS_LABEL`, `SAVE_TO_PROJECT_LABEL`, `LIBRARY_INDEX_NOTICE`, `SOURCES_API_BASE`, `SOURCE_KINDS`, `SOURCE_KIND_LABELS`, `SOURCES_READ_ONLY_NOTICE`) are defined as exported constants with comments pointing to the governing sections.
7. **HTTP `Origin` vs `Host` enforcement on `GET`:**
   - `auth.rs` enforces `Origin` on non-GET/HEAD requests and `Host` on all methods; `idea_export_route.rs` tests verify `401` on missing/wrong/cookie-only credentials and `403` on foreign `Host: evil.example`.

---

## 3. Merge Notes

### Touched Shared Files (Small, Local Edits)
- `crates/export/src/lib.rs`: added `pub mod idea;`.
- `crates/desktop-server/src/lib.rs`: added `pub mod routes_idea_export;`.
- `crates/desktop-server/src/api_dispatch.rs` (lines 508–510): added `("GET", ["api", "sessions", id, "idea", "export"]) => { crate::routes_idea_export::serve_idea_export(&mut stream, state, id, &req.query); }`.
- `crates/desktop-server/src/assets.rs` (lines 438–445): registered `/assets/library.js` and `/assets/library_sources.js`. (`assets.rs` is `546` lines, below the `580`-line split ceiling.)

### Home-Screen Mount Snippet for `app.js` / `index.html` (Intentionally Untouched by X17)
When wiring the global Library view into `app.js` (932L, frozen) and `index.html`:

```html
<!-- index.html: inside the Library view tab/panel container -->
<section id="library-view" class="view-panel" hidden>
  <div id="library-pane-host"></div>
</section>
```

```js
// app.js: lazy-mount once when the user opens the global "Library" view
import { mountLibraryPane } from '/assets/library.js';

let libraryPane = null;
function openGlobalLibraryView() {
  const host = document.getElementById('library-pane-host');
  if (!libraryPane && host) {
    libraryPane = mountLibraryPane(host, {
      getActiveSessionId: () => state.activeSessionId || '',
      getActiveSessionMode: () => state.activeSessionMode || '',
      onOpenSession: (sessionId) => openSessionById(sessionId),
    });
  } else {
    libraryPane?.loadPapers().catch(() => {});
  }
}
```

### Cross-Brief Follow-Ups
- **F10 read-only `Library::open_read_only`:** `crates/ledger/src/library_db.rs:31` opens `library.db` read-write. Adding `Library::open_read_only(data_dir)` (using `OpenFlags::SQLITE_OPEN_READ_ONLY`) in a dedicated F10 commit allows `crates/export/src/idea_extract.rs::open_library_if_present` to open an existing `library.db` strictly read-only.
- **F10 `/api/library/{key}/attach` for `user_library` files:** `crates/desktop-server/src/routes_library.rs:238-246` currently attaches only cached blobs (`raw_blob` / `file_sha256`). Once F10 supports attaching directly from a synced `user_library` path, the `hasCachedBlob` gate in `library.js` can be relaxed.

---

## 4. Independent Audits (Carbon / Pro) & Fixes Applied

1. **Checkpoint 1 Audit (`idea.rs`, `idea_extract.rs`, `routes_idea_export.rs`):**
   - Fixed 5 HIGH and 3 MEDIUM findings before committing `0c691ac` / `d9e624f`:
     - Fail-closed Rule C3 checks on `evidence.abstract` (`verbatim`, non-empty quote, valid `quote_sha256`, `start < end`, `provider`, `raw_blob`).
     - Fail-closed F4 checks rejecting missing or non-64-hex `path`/`file_sha256`/`quote_sha256`.
     - Multi-line OCR PDF span overlap priority (Rule C8) in `resolve_pdf_page_and_basis`.
     - Full Rule V7 validation across every stored `idea.verdict` in active revision, rejecting unknown keys, `map_seq` mismatches, and missing `idea.verdict` after `idea.map`.
     - Folding `idea.answer` updates into `sharpened_idea` alongside `idea.brief`.
     - Resolving BibTeX canonical keys across `key` + `ids` aliases, merging `PaperBibHint` metadata before deduping, and including `origin_paper` and `verdicts` papers.
     - Mapping preflight ledger tamper/corruption errors in `routes_idea_export.rs` to `409 Conflict` with `Cache-Control: no-store`.
     - Extending Rule S8 checks to `matched_quotes[*].display_grade` and `ocr_tag`.
2. **Checkpoint 2 Audit (`library.js`, `library_sources.js`, `library_ui.test.mjs`):**
   - Fixed 1 CRITICAL, 3 HIGH, and 1 MEDIUM finding before committing `456c0c3`:
     - **CRITICAL:** Replaced `node.children.pop()` (which throws `TypeError` on real browser `HTMLCollection`) in `clearChildren` with `replaceChildren()` -> `firstChild`/`removeChild` -> `Array.isArray` fallback; hardened `library_ui.test.mjs` fake DOM to expose a non-Array `HTMLCollection` without `.pop()`.
     - **HIGH:** Fixed `library_sources.js` to read exact Rust `DedupeAnalysis` fields (`possible_groups`, `signals`, `members`) and `SyncReport` fields (`duplicate_groups` array, `exact_duplicates`/`possible_duplicates`, `ScanWarning` `{ path, reason }`).
     - **HIGH:** Fixed `buildProjectSourcePayload` to preserve `/` in old-style arXiv IDs (`hep-th/9901001`), validate DOI format against `projects.rs:validated_doi`, and strip `\x0c`/disallowed control chars rejected by `projects.rs:check_text`.
     - **MEDIUM:** Added `withStatusError` across all Library pane actions, reset `<input type="file">.value = ''`, rejected empty PDF uploads, gated `"Attach to session"` on cached blobs (`raw_blob || file_sha256`), and added regression tests for all of them.

---

## 5. Verification Commands & Exact Measured Results

All verification ran on Linux as non-root (`uid=1001(antigravity)`) with `RE_SEARCH_PYTHON` unset:

| Command | Result |
|---|---|
| `id` | `uid=1001(antigravity)` (non-root) |
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | **PASS** (`0` files > 600 lines) |
| `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` | **PASS** (all JS assets valid syntax) |
| `node --test crates/desktop-server/tests/library_ui.test.mjs` | **PASS** (`6 passed, 0 failed, 0 skipped`) |
| `node --test crates/desktop-server/tests/*.test.mjs` | **PASS** (`269 passed, 0 failed, 0 skipped`) |
| `unset RE_SEARCH_PYTHON && cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` | **PASS** (`0` warnings, `0` errors) |
| `unset RE_SEARCH_PYTHON && cargo test --workspace --locked --no-fail-fast` | **PASS** (`191` test binaries: `1459 passed, 0 failed, 2 ignored`) |
| `git diff --stat origin/gemini/work/fixed -- Cargo.lock crates/research-tools/tests/golden/` | **PASS** (empty diff) |

Notes on specific test suites within the workspace run:
- `crates/export/src/idea_tests.rs`: `5 passed, 0 failed, 0 ignored` (part of `export --lib`: `14 passed`).
- `crates/export/tests/idea_export.rs`: `4 passed, 0 failed, 0 ignored`.
- `crates/desktop-server/tests/idea_export_route.rs`: `2 passed, 0 failed, 0 ignored`.
- `crates/research-tools/tests/tool_def_golden.rs`: `3 passed, 0 failed, 0 ignored`.
- `crates/research-tools/tests/tool_profile_golden.rs`: `3 passed, 0 failed, 0 ignored`.
- The `2 ignored` tests in the workspace run are pre-existing live-tier tests (`file_sheet_from_json` needing `EVAL_SHEET_JSON`, and `live_manifests_validate` needing `EVAL_LIVE_MANIFEST_DIR`). No skipped or `#[ignore]` tests were added.

---

## 6. What Could Not Be Compiled or Run on Linux

- `cfg(windows)` code (AppContainer sandbox, Job Object memory limits, WebView2 desktop shell integration, and Windows-only `worker` integration tests `appcontainer_sandbox`, `job_memory_limit`, `python_worker` which compile `0` tests on Linux).
- The Windows 11 + Chromium end-to-end browser harness (`[9/9]` batch).

---

## 7. Out-of-Scope Issues Noticed (Not Modified)

1. **`crates/desktop-server/src/api_dispatch.rs:31-36`:** Path dispatch splits `req.path` on `'/'` with `.filter(|s| !s.is_empty())` after `http_srv.rs` percent-decodes the path. Any `PaperKey` containing a literal `/` (such as `doi:10.1000/foo//bar` with consecutive slashes) would collapse empty segments in `rest @ ..` before reaching `routes_library.rs`. Single-slash DOIs (`doi:10.1000/xyz`) work because `routes_library.rs` rejoins `rest` with `'/'`, and `library.js` always calls `encodeURIComponent(key)`, so standard DOIs round-trip cleanly.
2. **`crates/ledger/src/library_db.rs:31` & `crates/research-tools/src/library.rs:37`:** `Library::open` only supports opening SQLite read-write (`LibraryDb::open`), so callers wanting strictly read-only metadata lookup against an existing `library.db` cannot pass `SQLITE_OPEN_READ_ONLY` without adding `open_read_only`.
3. **`crates/desktop-server/src/routes_library.rs:238-246`:** `POST /api/library/{key}/attach` only succeeds when a `fulltext` row has a cached blob (`raw_blob` / `file_sha256`); unhashed `user_library` rows return `404` (`no cached full-text blob`).
