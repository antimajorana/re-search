# Draft brief: `f13bc-sources-ext` — BibTeX & Zotero sources (F13b) and opt-in sending papers back (F13c)

> **Status:** Draft brief prepared against base commit `832eab5bf856cec90b3a4a2f08f95827ec9d1acd` (`work/draft-next-lanes`). Do not dispatch until start conditions below are satisfied.

---

## 1. Packages and governing design sections

- **F13b — Read-only external library adapters (`bibtex`, `zotero`):**
  - `bibtex`: reads a user-chosen `.bib` file and resolves its `file` field (supporting Zotero auto-export, JabRef, and Kandinsky `refs.bib`). Needs no PDF first-page extraction for metadata.
  - `zotero`: reads `zotero.sqlite` strictly read-only (taking a temporary snapshot copy when Zotero holds a write lock), extracting items, citation keys (pinned Better BibTeX keys first), identifiers, collections, tags, and attachment paths. Ported in part from Kandinsky's `zotero_import.rs` under the seven porting rules.
- **F13c — Opt-in sending papers back:**
  - **"Export BibTeX"**: writes BibTeX entries for selected library/session papers to a user-chosen file path.
  - **"Send to Kandinsky"**: invokes Kandinsky's own entry point (`kandinsky capture-paper <id or url>` or a `kandinsky://capture-paper` URL) strictly on an explicit user click. **re:Search never writes files into a Kandinsky vault or Zotero database.**
- **Governing design sections (with owner decisions of 2026-11-25):**
  - `00-overview.md` sections `0.1`, `2.1` (row F13), `2.3` (F2 `PaperKey` vs user citation keys), `2.4` (approval rule for composed network requests), `2.7` (F10 library tables), `2.11` (F13 model, adapters, sync, sending back, porting rules 1–7, rules & tests), `6` (build order), `7.2` Set B & `7.3` sync pass rules, `8.1` (AGPL-3.0 licence; bring your own papers added), `8.2` #5 (one global library), `8.3` #2 (AGPL-3.0 porting), #5 (read-only sync; send-back only via Kandinsky entry point or user-picked BibTeX), #6 (source order: Kandinsky vault & folder in F13a, then BibTeX & Zotero in F13b).
  - `explore.md` sections `4.3`, `6` (rule C9: `ingest_bytes` required before quote checks), `9` (F13 row).
  - `solve.md` sections `6`, `8` (F13 row); `investigate.md` sections `8`, `9` (F13 row).
  - `agents/RULES.md` and `agents/lib-f13a-f16a-f15.md` (F13a conventions).

---

## 2. Start conditions and owner gates

Split F13b and F13c into two independent checkpoints (or two sub-briefs) once their merge gates clear:

| Gate | Required for | Condition |
|---|---|---|
| **Merge: F2, F10, F14a** | F13b, F13c | **Already merged on base `832eab5bf`.** |
| **Merge: F13a (`lib-f13a-f16a-f15`)** | F13b, F13c | **Must merge first.** Delivers the `PaperSource` adapter trait (`paper_sources.rs`), `source_sync.rs`, `source_dedupe.rs`, sandboxed external-root path containment, `routes_sources.rs`, and `assets/library_sources.js`. |
| **Merge: F15 core (`lib-f13a-f16a-f15`)** | F13c (optional) | If merged before F13c, reuse `export/src/writeup_bib.rs` for BibTeX formatting instead of duplicating escaping/key generation. |
| **Merge: F5 (`f5-f7-mode-shell`)** | Route registration | Recommended before live HTTP wiring (`api_dispatch.rs`). If not merged, follow the single-dispatcher pattern of `routes_library.rs` and record the one-line registration in the report. |
| **Owner gate: Dependencies** | F13b, F13c | **Zero new dependencies (`Cargo.lock` unchanged).** `rusqlite 0.32` (bundled) is already in `crates/ledger/Cargo.toml`. Any crate request (e.g., a third-party BibTeX parser) requires explicit owner approval first; a small deterministic pure-Rust `.bib` parser avoids any `Cargo.lock` change. |
| **Owner gate: Kandinsky git-history audit** | F13b (`zotero`) | Before porting `plugins/beta/catalog-papers/engine/src/zotero_import.rs` (982 lines at commit `353368b`), verify git history for any Atlas (Alneb) or third-party notice (`00-overview.md` 2.11 rule 2). If Kandinsky cannot be cloned, write from design without the `Ported` header and document in the report. |
| **Deferred hook (do not build here)** | F13b | Optional network metadata lookup by title (Crossref) remains deferred until Solve **S5**'s shared `assets/approval_card.js` lands (after pre-turn `UiApprover` is verified, `00-overview.md` 2.4, 3.1 row 13). Neither `bibtex` nor `zotero` requires PDF-first-page or network title lookup (`00-overview.md` 2.11 table). Keep the hook untouched. |

---

## 3. Verified base interfaces vs future-merge interfaces

### Verified on base commit `832eab5bf` (use directly)
- **F2 (`crates/research-tools/src/paper_key.rs`):** `PaperKey`, `PaperRef`, `PaperIds`, `split_arxiv_id`, `canonical_doi`. User citation keys never replace `PaperKey`; they live in `SourceItemRecord.citekey`.
- **F10 SQLite layer (`crates/ledger/src/library_schema.rs`, 315 lines; `library_db_ext.rs`, 364 lines):**
  - `SOURCE_KINDS` (`library_schema.rs:54`) **already permits** `"kandinsky_vault"`, `"folder"`, `"bibtex"`, `"zotero"` (`CHECK` constraint in `source` table, line 137) — **zero schema migration required for F13b**.
  - `source_item` table (`library_schema.rs:145–162`) already has `source_id`, `external_id`, `paper_key`, `citekey`, `ids_json`, `version`, `title`, `authors_json`, `year`, `item_kind`, `collections_json`, `tags_json`, `files_json`, `state`, `updated_ms`.
  - `fulltext` table (`library_schema.rs:60–79`) already supports `origin = 'user_library'` with `file_path`, `file_size`, `file_mtime_ms`, `file_sha256` and empty `raw_blob`.
- **F10 typed API (`crates/research-tools/src/library.rs`, 532 lines; `library_schema.rs`, 594 lines; `library_ext_types.rs`, 273 lines):**
  - `SourceKind::{Bibtex, Zotero}`, `SourceItemState`, `SourceRecord`, `SourceItemFile`, `SourceItemRecord`, `Library::{upsert_source, list_sources, delete_source, upsert_source_item, list_source_items_for_source, upsert_fulltext, get_paper, list, search}`.
  - Note: `library_schema.rs` is 594 lines — do not add to it.
- **Crate boundary verified in `Cargo.toml`:** `rusqlite` lives **only** in `crates/ledger/Cargo.toml`; `crates/research-tools/Cargo.toml` does **not** depend on `rusqlite` (and must stay free of it). `crates/ledger` must stay free of `tokio`, `rmcp`, and `windows-sys`.
- **Route pattern (`crates/desktop-server/src/routes_library.rs`, 287 lines):** single exported dispatcher `handle_*(state, method, rest, query, body) -> ApiResult`, sitting behind `AuthGuard::authorize_api` + loopback `Origin` checks.

### Future-merge interfaces (MUST verify after F13a / F15 / F5 merge before coding)
- **F13a (`lib-f13a-f16a-f15`):**
  - Exact adapter trait signature in `crates/research-tools/src/paper_sources.rs` (expected: read-only `scan(source) -> Vec<SourceItemRecord>` plus attachment root resolution).
  - Exact external-root path containment helper (symlinks, Windows NTFS junctions, `..` traversal rejection) and `source_sync.rs` dry-run / polling / moved-file-by-SHA-256 hooks.
  - Exact `source_dedupe.rs` entry points (4 exact signals merge index rows; fuzzy title+author+year marks `possible_duplicate`).
  - Exact endpoint paths and dispatcher signature in `crates/desktop-server/src/routes_sources.rs` and paper-action UI slot in `crates/desktop-server/assets/library_sources.js` (and later X17's `assets/library.js`).
- **F15 core (`lib-f13a-f16a-f15`):** exact BibTeX serialization signature in `crates/export/src/writeup_bib.rs` (if reused by F13c).
- **F5 (`f5-f7-mode-shell`):** `crates/desktop-server/src/api_dispatch.rs` registration arm pattern.

---

## 4. Smallest implementation checkpoints

Execute in order; commit and run full checks after each checkpoint:

1. **Checkpoint 1 — F13b BibTeX adapter (`source_bibtex.rs`):**
   - Pure-Rust `.bib` parser handling `@article`/`@inproceedings`/`@book`/`@misc`/etc., nested `{...}` and `"..."` values, `@string` macros, TeX accent decoding for author/title normalization, `doi`, `eprint`/`archiveprefix`/`arxiv`, `isbn`, `year`/`date`, `keywords`/`groups`, and citekey preservation.
   - Parse `file` field formats used by Zotero Better BibTeX (`Description:/path/to/file.pdf:application/pdf` with `;`-separated multi-attachments and escaped `\:`, `\;`), JabRef (`:rel/path.pdf:PDF`), and Kandinsky `refs.bib`.
   - Resolve relative paths against the `.bib` directory (and optional configured `external_pdf_roots` in `source.settings`); refuse any path, symlink, or junction leaving the allowed roots via F13a's containment helper.
   - Register `SourceKind::Bibtex` in F13a's adapter registry (`paper_sources.rs`).
2. **Checkpoint 2 — F13b Zotero read-only adapter (`ledger::zotero_reader` + `source_zotero.rs`):**
   - Audit Kandinsky `zotero_import.rs` (982 lines at `353368b`) per all 7 porting rules in `00-overview.md` 2.11; split cleanly under 600 lines per file and drop all vault-writing/annotation code.
   - Implement `crates/ledger/src/zotero_reader.rs`: open `zotero.sqlite` with `SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_NO_MUTEX` + `PRAGMA query_only = ON`; if `SQLITE_BUSY` / `SQLITE_LOCKED`, copy `zotero.sqlite` (and `-wal`/`-shm` if present) to an isolated temp directory, open the snapshot read-only, query items/creators/fields/collections/tags/attachments/Better BibTeX keys (if `better-bibtex.sqlite` is beside it, read-only), and remove the temp snapshot on drop. **Never open the user's live `zotero.sqlite` read-write.**
   - Implement `crates/research-tools/src/source_zotero.rs`: translate plain row structs into `SourceItemRecord`, resolve `storage:<key>/<file>` and `attachments:` paths inside the Zotero data directory / configured linked-attachment base directory using F13a's containment helper, map to F2 `PaperKey`, and register `SourceKind::Zotero` in `paper_sources.rs`.
3. **Checkpoint 3 — F13c opt-in sending papers back (`source_export.rs`, `routes_sources_ext.rs`, `library_send_back.js`):**
   - **Export BibTeX:** format selected papers (preferring stored `source_item.citekey` over app-generated keys, falling back to `paper.bibtex` or structured fields via F15's `writeup_bib.rs` if merged) and write atomically ONLY to a user-chosen destination path (refuse writing inside any registered read-only `PaperSource.root` unless explicitly separated; reject `..` / symlink escapes).
   - **Send to Kandinsky:** build either (a) a safe argv invocation `kandinsky capture-paper <validated_id_or_url>` executed directly via `std::process::Command::new` (NEVER through a shell such as `cmd /c` or `sh -c`; reject any argument starting with `-` or containing control characters/whitespace; allow only validated `arXiv:` / `doi:` / `https://` targets), or (b) a validated `kandinsky://capture-paper?...` deep link with `percent_encoding`. Strictly triggered by an explicit user click (`POST` endpoint with loopback auth + `Origin` check); zero automatic or background write-back.
   - Add UI controls ("Export BibTeX", "Send to Kandinsky") in `assets/library_send_back.js` with strict DOM `textContent` escaping (zero `innerHTML`), and mount them into F13a's `assets/library_sources.js` (and later X17's `assets/library.js`) paper-action slot.

---

## 5. Owned files (disjoint, <= 600 lines each)

| Package | Owned files (new unless noted) | Crate / Boundary Notes |
|---|---|---|
| F13b | `crates/ledger/src/zotero_reader.rs`, `crates/ledger/src/zotero_reader_tests.rs` | Confines `rusqlite` strictly to `crates/ledger`; no `tokio`, `rmcp`, or `windows-sys`. |
| F13b | `crates/research-tools/src/source_bibtex.rs`, `source_bibtex_tests.rs` | Pure Rust `.bib` parser + `file` field resolver; no `rusqlite`. |
| F13b | `crates/research-tools/src/source_zotero.rs`, `source_zotero_tests.rs` (+ optional `source_zotero_parse.rs` if needed to stay `<= 600` lines) | Ported read-only half of Kandinsky `zotero_import.rs` (982 lines split `<= 600`); calls `ledger::zotero_reader`. |
| F13b | `crates/research-tools/tests/sources_ext.rs` | Integration tests for Set B `.bib` and synthetic `zotero.sqlite` fixtures. |
| F13c | `crates/research-tools/src/source_export.rs`, `source_export_tests.rs` | Opt-in BibTeX export writer + safe Kandinsky capture argv/URL builder & validator. |
| F13c | `crates/desktop-server/src/routes_sources_ext.rs`, `crates/desktop-server/tests/sources_ext_routes.rs` | Single-dispatcher route module for opt-in export & Kandinsky send-back. |
| F13c | `crates/desktop-server/assets/library_send_back.js`, `crates/desktop-server/tests/library_send_back.test.mjs` | UI buttons/dialogs; zero `innerHTML`. |

---

## 6. Shared registration seams and "Do not edit" files

### Minimal registration seams (each in its own small commit, listed under Merge notes)
- `crates/ledger/src/lib.rs` (515 lines): `pub mod zotero_reader;` (+ re-exports).
- `crates/research-tools/src/lib.rs`: `pub mod source_bibtex;`, `pub mod source_zotero;`, `pub mod source_export;`.
- `crates/research-tools/src/paper_sources.rs` (delivered by F13a): add `SourceKind::Bibtex` and `SourceKind::Zotero` match arms to the adapter dispatcher.
- `crates/desktop-server/src/lib.rs`: `pub mod routes_sources_ext;`.
- `crates/desktop-server/src/assets.rs` (365 lines): register `"/assets/library_send_back.js"`.
- `crates/desktop-server/src/api_dispatch.rs` (delivered by F5): wire the delegating arm for `routes_sources_ext.rs` (`/api/sources/export/bibtex` and `/api/sources/send/kandinsky`) cleanly behind `AuthGuard::authorize_api` + loopback `Origin` checks.
- **UI mount seam (F13c):** mount `assets/library_send_back.js` ("Export BibTeX" and "Send to Kandinsky") from F13a's `assets/library_sources.js` paper-action slot (and from X17's `assets/library.js` once merged), without editing `index.html` or `app.js`.

### Explicit "Do not edit" files
- `Cargo.lock` and all `Cargo.toml` files (unless the owner explicitly approves a dependency).
- `crates/research-tools/tests/golden/tool_defs.txt` and `tool_profiles.txt` (zero tool definitions added or changed).
- `crates/desktop-server/src/http_srv.rs` (702 lines), `routes_session.rs` (616 lines), `assets/index.html` (768 lines), `assets/app.js` (932 lines), `crates/agent/src/resume.rs`, `crates/desktop-server/src/turn_runner.rs`.
- `crates/research-tools/src/library_schema.rs` (594 lines), `ingest.rs` (590 lines), `pdf_match.rs` (589 lines), `pdf_pages.rs` (569 lines).
- Any F13a-owned adapter (`source_kandinsky.rs`, `source_folder.rs`, `source_dedupe.rs`) or F14/F16/V17 files.

---

## 7. Tests through the entry point

All tests use strictly self-authored synthetic fixtures (zero third-party paper text, zero live network/model calls, zero real Kandinsky/Zotero installations):

1. **Set B — BibTeX fixture (`sources_ext.rs`):**
   - Self-authored `.bib` with Zotero-style, JabRef-style, and Kandinsky-style `file` fields + self-made synthetic PDFs (`tests/pdf_support`-style).
   - Planted cases: moved PDF (recognized by SHA-256), edited `.bib` entry (updates metadata), deleted PDF (`state = missing`), duplicate by DOI and by file hash, preprint + journal version, and a `file` entry with `../escape.pdf` and an out-of-root symlink (both refused).
   - **Immutability property test:** hash the entire `.bib` + attachment tree before and after two consecutive syncs; assert byte-identical tree and that the second sync changes zero index rows.
   - **Evidence independence (C9):** attach a paper to a session via `ingest_bytes`, verify a quote via `quote_core::verify_quote`, then remove the source and delete `library.db`; assert the session quote check still passes.
2. **Set B — Zotero SQLite fixture (`zotero_reader_tests.rs`, `sources_ext.rs`):**
   - Programmatically create a minimal synthetic `zotero.sqlite` matching Zotero's core tables (`items`, `itemTypes`, `itemData`, `itemDataValues`, `fields`, `itemCreators`, `creators`, `collections`, `collectionItems`, `tags`, `itemTags`, `itemAttachments`, plus synthetic BBT citekey table) and synthetic attachments in `storage/<KEY>/`.
   - **Locked-database test:** hold an exclusive `BEGIN EXCLUSIVE` transaction on a second connection to `zotero.sqlite` while scanning; assert the snapshot fallback succeeds, cleans up its temp directory, and leaves the source `zotero.sqlite` + `storage/` byte-identical.
   - **Path containment test:** an attachment pointing outside the Zotero storage/base directory (via `..` or symlink) is refused.
   - Ported Kandinsky read-only unit tests kept and passing (`00-overview.md` 2.11 rule 7).
3. **F13c opt-in send-back & security tests (`source_export_tests.rs`, `sources_ext_routes.rs`, `library_send_back.test.mjs`):**
   - BibTeX export preserves user `citekey` when present, escapes special characters, writes atomically to the requested output file, refuses writing inside a registered read-only source root, and passes `AuthGuard::authorize_api` + loopback `Origin` enforcement.
   - Kandinsky capture tests: validate arXiv/DOI/HTTPS inputs; **reject** shell metacharacters (`;`, `|`, `$(...)`, backticks, newlines), option injection (`--config=...`, `-e`), non-HTTPS schemes (`file://`, `javascript:`), and verify `Command` uses argv-only execution (with a fake/stub executable in tests, never a real Kandinsky binary or shell) and `kandinsky://capture-paper?...` URL encoding is exact.
   - Verify syncing never invokes export or Kandinsky capture automatically.
   - Node XSS test on `library_send_back.js`: `<img onerror>` / `<script>` in title/citekey produces zero `innerHTML` writes.

---

## 8. Document conflicts and resolutions

| # | Conflict | Exact sources | Resolution (`00-overview.md` wins) |
|---|---|---|---|
| 1 | Zotero link as optional add-on vs core package F13b | `explore.md` section 10 ("Zotero link" add-on) vs `00-overview.md` 0.1, 2.1 (row F13), 2.11 | Core package F13b (`00-overview.md` 2.11, scope revision 2026-11-25). |
| 2 | `source_zotero.rs` listed under `research-tools` vs workspace `rusqlite` boundary | `00-overview.md` 2.1 (`research-tools/src/source_zotero.rs`) vs `agents/f10-library.md` & `reports/f10-library.md` (`research-tools` must not depend on `rusqlite`) | Put SQLite open/snapshot/queries in `crates/ledger/src/zotero_reader.rs` and typed adapter logic in `crates/research-tools/src/source_zotero.rs` (exact F10 pattern). |
| 3 | Kandinsky `zotero_import.rs` is 982 lines vs 600-line ceiling | `00-overview.md` 2.11 table (982 lines) vs `00-overview.md` 2.11 rule 4 & `agents/RULES.md` (<= 600 lines) | Split on import across `ledger/src/zotero_reader.rs`, `research-tools/src/source_zotero.rs`, and sibling test/parse files; limit is not waived. |
| 4 | Outbound write-back vs read-only library guarantee | `00-overview.md` 2.11 ("Sending papers back") & 8.3 #5 | Never write Kandinsky vault or Zotero files directly; only user-chosen BibTeX export and `kandinsky capture-paper` / `kandinsky://capture-paper` on explicit user click. |

---

## 9. Out of scope and hand-back

- **Out of scope:** Kandinsky vault and folder adapters (F13a); writing into Kandinsky vaults, answering the Zotero browser extension, drop inbox, merging duplicates on disk, reading notes or PDF annotations (all stay Kandinsky's job per `00-overview.md` 2.11); optional network metadata lookup by title over Crossref (waits for Solve **S5**'s shared `assets/approval_card.js` after pre-turn `UiApprover` is verified); OCR or PDF vision (F14b/c); lessons (F16b).
- **Hand-back:** Run `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`, `cargo test -p research-tools --test tool_def_golden`, `cargo clippy --workspace --all-targets -- -D warnings` (checking zero warnings in touched files), `cargo test --workspace --no-fail-fast`, and `node --check` / `node --test` for touched JS assets. Write `docs/design/modes-2026-11/reports/f13bc-sources-ext.md` and push only to `gemini/modes/f13bc-sources-ext` under `agents/RULES.md`.
