# Report: `f10-library` (package F10 — the global paper library)

- **Base commit:** `e1517477cbceba0a6de6e4a143c34fe3b8b8411d` (`origin/gemini/work/fixed`)
- **Work branch:** `work/f10-library` (pushed to `gemini/modes/f10-library`)
- **Toolchain on this machine:** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `cargo 1.100.0 (3acc79c0c 2026-11-05)`, Python `3.12.16` (with `sympy`, `numpy`, `mpmath`) on Linux x86_64
- **Unchanged guard rails:** `Cargo.lock` untouched (0 new dependencies); `crates/research-tools/tests/golden/tool_defs.txt` untouched (no tool definitions added or modified); `crates/policy` and `crates/ledger` remain free of `tokio`, `rmcp`, and `windows-sys`; `crates/research-tools` remains free of `rusqlite`; `http_srv.rs`, `routes_session.rs`, `turn_runner.rs`, and `resume.rs` untouched.

---

## 1. What was implemented (package F10)

### SQLite schema v1, forward-only migration, and `LibraryDb` (`crates/ledger`)
- `crates/ledger/src/library_schema.rs` (315 lines, new): schema constants (`LIBRARY_SCHEMA_VERSION = 1`, `USER_MARKS_FILE_NAME = "user_marks.json"`, `USER_MARKS_BACKUP_NAME = "user_marks.json.bak"`, table/role/status/origin/source-kind/state constants tied to `00-overview.md` section 8.2 decision 5), full v1 SQLite DDL (`paper`, `alias`, `fulltext` extended with nullable `file_path`/`file_size`/`file_mtime_ms`/`file_sha256` for `origin = 'user_library'`, `source`, `source_item`, `paper_text`, `seen` with all 6 roles including `investigated`/`cited_lookup`/`solve_source`, `skeleton`, `finding`, `search_cache`, `user_mark`, `paper_fts` FTS5 virtual table, `schema_migrations`), and forward-only `migrate_library_db` (upgrades an F3 v0 `fulltext`-only database in place without data loss; fails closed if `PRAGMA user_version > LIBRARY_SCHEMA_VERSION`).
- `crates/ledger/src/library_rows.rs` (244 lines, new): plain string-keyed row structs (`LibraryPaperRow`, `LibraryFulltextRow`, `LibrarySourceRow`, `LibrarySourceItemRow`, `LibraryPaperTextRow`, `LibrarySeenRow`, `LibrarySkeletonRow`, `LibraryFindingRow`, `LibrarySearchCacheRow`, `LibraryUserMarkRow`, `LibrarySearchParams`, `LibrarySearchRow`, `RebuildBatch`) and SQLite row mappers.
- `crates/ledger/src/library_db.rs` (456 lines, new): `LibraryDb` opening `<data_dir>/library/library.db` (WAL, `busy_timeout(10s)`, `DEFENSIVE`, `trusted_schema = OFF`, `foreign_keys = ON`) with CRUD for `paper`, `alias`, `fulltext`, `seen`, `skeleton`, `finding`, `user_mark`, and verified blob reads (`<data_dir>/blobs/` first, `<data_dir>/library/blobs/` second).
- `crates/ledger/src/library_db_ext.rs` (364 lines, new): CRUD for F13 (`source`, `source_item`), F14 (`paper_text`), F11 (`search_cache`), and transactional `apply_rebuild_batch` (replaces ledger-derived tables while keeping `user_mark`, `source`, `source_item`, `search_cache`, and `user_library` fulltext rows intact).
- `crates/ledger/src/library_db_query.rs` (301 lines, new): `paper_fts` FTS5 refresh (`title`, `authors`, `abstract`, finding `gists`, and verified `paper_text` full text capped at 2 MiB per paper), safe FTS5 token quoting (`build_safe_fts_query`), `search_papers` (FTS5 `q`, case-insensitive `title`, `json_each` `author`, exact `year`, `paper_key` with `alias` resolution, `status`, `role`, pagination), and `recent_papers_by_role`.
- `crates/ledger/src/library_db_tests.rs` (250 lines, new): 5 unit tests confirming FTS5 availability in bundled `rusqlite 0.32`, F3-to-F10 in-place upgrade without row loss, F10-to-F3 `FulltextCache` usability, forward-only downgrade refusal, and CHECK constraints across `seen`, `user_mark`, `skeleton`, and `source`.
- `crates/ledger/src/fulltext_cache.rs` (312 lines, +13/-7): minimal compatibility guard so `FulltextCache::get_bytes` treats unhashed `user_library` rows (`raw_blob == ""`) as a clean cache miss (`Ok(None)`) instead of `Error::BadHash`, prefers blob-backed rows on unversioned lookup, and exposes internal blob helpers `pub(crate)` to sibling `library_db`.
- `crates/ledger/src/lib.rs` (515 lines, +9): exports `pub mod library_db`, `pub mod library_schema`, private `mod library_db_ext`, `mod library_db_query`, `mod library_rows`, and re-exports `LibraryDb`, `LIBRARY_SCHEMA_VERSION`, `ORIGIN_USER_LIBRARY`, `USER_MARKS_FILE_NAME`, `USER_MARKS_BACKUP_NAME`.

### Typed library API, ledger rebuild, and user-mark backup (`crates/research-tools`)
- `crates/research-tools/src/library_schema.rs` (594 lines, new): typed enums (`SeenRole`, `MarkStatus`), structs keyed by F2's `PaperKey` (`LibraryIds`, `PaperFlags`, `PaperRecord`, `FulltextRecord`, `SeenRecord`, `SkeletonRecord`, `FindingRecord`, `UserMark`, `UserMarksBackup`, `PaperSummary`, `PaperDetail`, `LibraryQuery`, `RebuildReport`), constants, and re-exports of F13/F14/F11 extension types.
- `crates/research-tools/src/library_ext_types.rs` (273 lines, new): typed records for F13 (`SourceKind`, `SourceItemState`, `SourceRecord`, `SourceItemFile`, `SourceItemRecord`), F14 (`PageBasis`, `PaperTextRecord`), and F11 (`SearchCacheRecord`), re-exported from `library_schema`.
- `crates/research-tools/src/library_marks.rs` (265 lines, new): validation/normalization of marks, durable sidecar persistence at `<data_dir>/library/user_marks.json` with rolling previous-generation backup `<data_dir>/library/user_marks.json.bak`, cross-platform atomic temp+replace writes, automatic `.bak` recovery if primary is missing/corrupt, and portable SHA-256-checksummed JSON export/restore (`UserMarksBackup`).
- `crates/research-tools/src/library_rebuild.rs` (549 lines, new): deterministic `rebuild_from_ledger` scanning `<data_dir>/ledger.db` and `<data_dir>/blobs/` across all sessions (handling `literature.fetched`, `source.ingested`, `investigation.skeleton`, `investigation.evidence`, `evidence.quote`, `evidence.abstract`, `idea.paper`, `idea.triage`, and `paper.text_extracted`), applying mode detection (`00-overview.md` 3.1 row 3) for `seen.role`, rebuilding `paper_fts`, and restoring `user_mark` from `user_marks.json` (or `.bak`).
- `crates/research-tools/src/library_rebuild_lit.rs` (141 lines, new): `literature.fetched` blob parsing (`arxiv::parse_atom_feed`, `literature::parse_hits`) with payload-array fallback.
- `crates/research-tools/src/library.rs` (532 lines, new): `Library` API (`open`, `schema_version`, `upsert_paper`, `resolve_key`, `get_paper`, `list`, `search`, `recent_by_role`, `record_seen`, `upsert_fulltext`, `read_fulltext_bytes`, `upsert_skeleton`, `get_skeleton`, `upsert_finding`, `upsert_paper_text`, F13 source/item methods, F11 search_cache methods, `set_user_mark`/`get_user_mark`/`delete_user_mark`/`list_user_marks`, `export_user_marks_backup`/`restore_user_marks_backup`, `delete_paper`, `attach_to_session` via `ingest_bytes`, and `rebuild`).
- `crates/research-tools/src/lib.rs` (280 lines, +6): exposes `pub mod library`, `pub mod library_schema`, and private `mod library_ext_types`, `mod library_marks`, `mod library_rebuild`, `mod library_rebuild_lit`.
- `crates/research-tools/tests/library_store.rs` (533 lines, new): 5 integration tests using self-written synthetic fixtures covering F3->F10 upgrade + F3 interoperability, delete-`library.db`-rebuild-compare across Investigate/Solve/Explore sessions, `.bak` recovery + portable checksummed backup/restore, multi-filter search + F13/F14 table readiness + alias lookup, and Explore rule S5 (`library_attach_reingests_same_bytes` + `library_loss_does_not_break_verify`).

### Desktop HTTP routes (`crates/desktop-server`)
- `crates/desktop-server/src/routes_library.rs` (287 lines, new): single exported dispatcher `handle_library(state, method, rest, query, body) -> ApiResult` implementing `GET /api/library` (list & search by `q`, `title`, `author`, `year`, `key`, `status`, `role`, `limit`, `offset`), `GET /api/library/{key}`, `POST /api/library/{key}/mark`, `DELETE /api/library/{key}/mark`, `POST /api/library/{key}/attach`, `DELETE /api/library/{key}`, `POST /api/library/rebuild`, `GET /api/library/marks/backup`, and `POST /api/library/marks/restore`. Handles slash-containing `PaperKey`s (`doi:10.x/y`, `arxiv:hep-th/9901001`) re-joined after `http_srv.rs` percent-decoding. No network or model calls.
- `crates/desktop-server/src/lib.rs` (215 lines, +1): exposes `pub mod routes_library;`.
- `crates/desktop-server/tests/library_routes.rs` (362 lines, new): 4 integration tests covering list/search/detail/slash-keys/validation errors, mark writes/deletes/backup/restore, attach/delete/rebuild end-to-end against real sessions and ledger, and `AuthGuard::authorize_api` enforcement on `/api/library` paths.

---

## 2. Where user marks live and how they are backed up (`00-overview.md` 8.2 row 5)

- **Authoritative on-disk location:** `<data_dir>/library/user_marks.json` (outside `library.db`, alongside the planned `<data_dir>/library/lessons.db` of F16). It is formatted as a deterministic, versioned JSON envelope (`UserMarksBackup { schema_version: 1, exported_ms, sha256, marks }`) whose `sha256` covers the canonical JSON encoding of the sorted `marks` array.
- **Query index copy:** mirrored into `library.db`'s `user_mark` table on every write so `GET /api/library?status=key` and `GET /api/library/{key}` run as single SQLite queries without re-reading JSON on reads.
- **Rolling local backup:** before replacing `user_marks.json`, every write checks the existing primary file and rotates it atomically to `<data_dir>/library/user_marks.json.bak`. Both `Library::open` (when `user_mark` is empty, e.g. after deleting `library.db`) and `Library::rebuild` load `user_marks.json`, automatically falling back to `user_marks.json.bak` if the primary is missing or fails checksum verification.
- **Portable backup & restore:** `Library::export_user_marks_backup_bytes` / `restore_user_marks_backup` (and routes `GET /api/library/marks/backup`, `POST /api/library/marks/restore`) export and verify checksummed JSON backups suitable for inclusion in external/app backups. Tampered payloads fail SHA-256 verification before modifying disk or SQLite.

---

## 3. Decisions taken alone, design deviations, and assumptions

### Decisions
1. **Uniform `paper_key` SQL column name across all tables:** F3's `fulltext` table already named its key column `paper_key`. All F10 tables (including `skeleton`) use `paper_key` in SQL while serializing the Rust/JSON field as `key` (matching F2's `PaperRef.key`).
2. **No foreign key from `fulltext`, `seen`, `skeleton`, `finding`, `paper_text`, or `user_mark` to `paper`:** `FulltextCache::put_bytes` (F3) writes directly to `fulltext` without inserting a `paper` row, and `user_mark` must survive deleting or rebuilding `paper`. `alias.paper_key -> paper(paper_key)` and `source_item.source_id -> source(id)` keep `ON DELETE CASCADE` with `PRAGMA foreign_keys = ON`.
3. **Minimal 3-line guard in `crates/ledger/src/fulltext_cache.rs`:** `FulltextCache::get_bytes` previously returned `Error::BadHash("")` if a `user_library` row had an empty `raw_blob`. Treating empty `raw_blob` as `Ok(None)` and ordering blob-backed rows ahead of unhashed rows keeps F3 100% compatible with F10 databases containing `user_library` rows.
4. **Rebuild role mapping for earlier Research sessions:** Sessions with no mode marker (`Mode::Research`) map `source.ingested` events to `SeenRole::Read` (while `Investigate -> Investigated`, `Solve -> SolveSource`, `Explore -> Read`, and an explicit `payload.role` field overrides when present).
5. **Rebuild preservation of non-ledger user state:** In-place `rebuild` (`POST /api/library/rebuild`) wipes and repopulates ledger-derived tables (`paper`, `alias`, `non-user_library fulltext`, `seen`, `skeleton`, `finding`, `paper_text`, `paper_fts`), preserves `source`, `source_item`, `search_cache`, and `user_library` `fulltext` rows when `library.db` exists, and reloads `user_mark` from `user_marks.json` (or `.bak`).
6. **`DELETE /api/library/{key}` does not delete `user_mark` or touch `ledger.db` / `blobs/`:** Removing a paper clears only its rebuildable index rows; marks are cleared explicitly via `DELETE /api/library/{key}/mark`.
7. **Slash-containing `PaperKey` routing in `routes_library.rs`:** Because `http_srv.rs:629` percent-decodes the path before splitting on `'/'`, keys like `doi:10.9999/a/b` arrive as multiple segments. `handle_library` takes `rest: &[&str]` after `["api", "library"]` and re-joins key segments with `'/'` for `GET`, `DELETE`, `POST .../mark`, `DELETE .../mark`, and `POST .../attach`.
8. **Extra helper routes for mark backups:** Added `GET /api/library/marks/backup`, `POST /api/library/marks/restore`, and `DELETE /api/library/{key}/mark` alongside the `explore.md` 5.4 routes; none can collide with a valid `PaperKey` because every `PaperKey` contains `':'`.

### Deviations from the design
- `crates/research-tools/src/library.rs` and `library_schema.rs` were factored with small sibling private modules (`library_ext_types.rs`, `library_marks.rs`, `library_rebuild.rs`, `library_rebuild_lit.rs`) and `crates/ledger/src/library_db.rs` with `library_rows.rs`, `library_db_ext.rs`, `library_db_query.rs`, `library_schema.rs` so every touched/added file stays strictly at or under 600 lines. All public types are re-exported from `research_tools::library` and `research_tools::library_schema`.
- `crates/desktop-server/src/http_srv.rs` was **not** edited (703 lines, forbidden by `RULES.md` and `f10-library.md`); `handle_library` is exported ready for the single-line integration below.

### Assumptions
- Future events written by V3/V11 (`investigation.skeleton`), F4 (`evidence.quote`, `evidence.abstract`), Explore (`idea.paper`, `idea.triage`), and F14 (`paper.text_extracted`) are assumed to carry `paper_key` (or `key`, or `source_id` referencing a `source.ingested` in the same session) and the field names documented in `00-overview.md` 2.5/2.7/2.12 and `explore.md` 4.1/4.3. If any later brief picks a different payload field name, only `library_rebuild.rs` needs an extra fallback key.
- `LiteratureHit` in today's `crates/research-tools/src/literature.rs` does not carry an abstract field yet (F11 adds abstracts), so rebuilt papers from current INSPIRE/Crossref/S2 `literature.fetched` blobs populate title, authors, year, IDs, retracted flag, and `bibtex` (leaving `abstract` empty until F11 enriches them).

---

## 4. Merge notes

### Required one-line registration in `crates/desktop-server/src/http_srv.rs`
Inside `dispatch_api` (after `/api/` header-token auth via `state.auth.authorize_api(&creds)` and `read_http_body`, right beside the existing `routes_subagents` arms around line 487), add:
```rust
(_, ["api", "library", rest @ ..]) => write_api_res(&mut stream, crate::routes_library::handle_library(state, method, rest, &req.query, &req.body)),
```
No changes to `AuthGuard::authorize_api` are needed: `/api/library` is not a cookie route, so header-token authentication and loopback `Origin` checks on mutations already apply automatically.

### Edits to existing files
- `crates/ledger/src/fulltext_cache.rs` (commit `a80710a`): empty-`raw_blob` cache-miss guard, blob-first unversioned ordering, `pub(crate)` on blob helpers.
- `crates/ledger/src/lib.rs` (commit `6b9af84`): module declarations and re-exports for `library_schema` and `library_db`.
- `crates/research-tools/src/lib.rs` (commit `afc51a4`): module declarations for `library` and `library_schema` (+ private siblings).
- `crates/desktop-server/src/lib.rs` (commit `5103cc3`): `pub mod routes_library;`.

### Integration points for other briefs / later packages
- **F13 (`paper_sources.rs`, `source_sync.rs`, `routes_sources.rs`):** `source`, `source_item`, and `fulltext` (`origin = "user_library"`, `file_path`, `file_size`, `file_mtime_ms`, `file_sha256`) tables and typed `Library` methods (`upsert_source`, `list_sources`, `delete_source`, `upsert_source_item`, `list_source_items_for_source`, `upsert_fulltext`) are ready with zero schema migration needed. `Library::attach_to_session` currently reads only verified blobs in `<data_dir>/blobs/` or `<data_dir>/library/blobs/`; F13 should extend it (or provide a sandbox-validated reader) for `user_library` files whose bytes have not yet been ingested as a blob.
- **F14 (`pdf_pages.rs`):** `paper_text` table + `Library::upsert_paper_text` are ready; indexing extracted text automatically updates `paper_fts` for full-text search.
- **F11 (`lit_providers.rs`):** `search_cache` table + `Library::{upsert_search_cache, get_search_cache, prune_search_cache}` are ready.
- **V3 / V11 (`investigation_skeleton.rs`, `investigation_cited.rs`):** `skeleton` table + `Library::{upsert_skeleton, get_skeleton}` are ready; `rebuild` already indexes `investigation.skeleton` events.
- **V4 / Investigate "recent papers":** call `Library::recent_by_role(SeenRole::Investigated, limit)`.
- **F15 (`writeup_bib.rs`):** `source_item.citekey` and `paper.bibtex` are stored and returned via `PaperDetail`.
- **F16 (`lessons.rs`):** `<data_dir>/library/lessons.db` stays a separate file as required by `00-overview.md` 2.7 and 2.14; F16a can read `Library::get_user_mark` / `list_user_marks` for `key` and `irrelevant` ranking adjustments.

---

## 5. Verification, test counts, and pre-existing findings

### Commands run

| Command | Result |
|---|---|
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | **Pass** — `No touched source file is over 600 lines.` (max touched file: `library_schema.rs` at 594 lines) |
| `git diff --stat origin/gemini/work/fixed -- Cargo.lock crates/research-tools/tests/golden/tool_defs.txt` | **Pass** — empty diff |
| `cargo test -p ledger` | **Pass** — 27 passed, 0 failed, 0 ignored (10 lib unit tests incl. 5 new `library_db` tests + 8 `hash_chain` + 9 `schema_rejects`) |
| `cargo clippy -p ledger --all-targets -- -D warnings` | **Pass** — 0 warnings |
| `cargo test -p research-tools --test library_store --test tool_def_golden` | **Pass** — 8 passed, 0 failed, 0 ignored (5 new `library_store` + 3 `tool_def_golden`) |
| `cargo clippy -p research-tools --lib --test library_store --test tool_def_golden -- -D warnings` | **Pass** — 0 warnings |
| `cargo test -p desktop-server --test library_routes` | **Pass** — 4 passed, 0 failed, 0 ignored |
| `cargo test --doc -p desktop-server` | **Pass** — 0 passed, 0 failed, 0 ignored |
| `cargo clippy -p desktop-server --test library_routes -- -D warnings -A clippy::manual-unwrap-or` | **Pass** — 0 warnings (only pre-existing `evidence.rs:744` triggers `manual_unwrap_or`) |
| `cargo test --workspace --no-fail-fast` | **Pass** — **677 passed, 0 failed, 2 ignored** (after switching `routes_library.rs` doc fence from `ignore` to `text`; before that tweak it ran 677 passed, 0 failed, 3 ignored across 99 targets; the 2 remaining ignored tests are pre-existing live-evaluation tests in `crates/agent/tests/eval_harness.rs`) |
| `RE_SEARCH_PYTHON="$(command -v python3)" cargo test -p export --no-fail-fast` | **Pass** — 8 passed, 0 failed, 0 ignored (Python SymPy/NumPy/mpmath replay checks executed and passed) |
| `cargo clippy --workspace --all-targets -- --no-deps` | **Exact match to base commit** — 6 pre-existing warnings, 0 warnings in any added or modified file |

### Pre-existing clippy warnings (proven on base commit `e1517477cbceba0a6de6e4a143c34fe3b8b8411d`)

Running `cargo clippy --workspace --all-targets -- --no-deps` in a clean detached worktree at `e1517477cbceba0a6de6e4a143c34fe3b8b8411d` produced the exact same 6 warnings (and zero others on this branch):

1. `crates/desktop-server/src/evidence.rs:744` — `clippy::manual_unwrap_or` (noted in `RULES.md`).
2. `crates/research-tools/tests/sandbox_and_fs.rs:6` — `unused_imports` (`TEX_AUDIT`, `is_symlink_or_junction`, Linux build).
3. `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` — `unused_imports` (`BufRead`, `BufReader`, Linux build).
4. `crates/agent/src/guardrails.rs:335` — `clippy::items_after_test_module`.
5. `crates/agent/tests/eval_support/manifest.rs:289` — `clippy::match_like_matches_macro`.
6. `crates/agent/tests/eval_support/sheet.rs:102` — `clippy::unnecessary_map_or`.

### What could not be compiled or run on Linux
- `crates/desktop-app` and `cfg(windows)` code paths/tests in `crates/worker` (`appcontainer_sandbox.rs`, `python_worker.rs`), `crates/cli/tests/contained_router.rs`, `crates/research-tools/src/sandbox.rs`, `crates/research-tools/tests/sandbox_and_fs.rs`, `crates/chatgpt-auth`, and `crates/cli`.
- Windows-specific rename-over-existing behavior in `library_marks::atomic_replace_file` was coded for Windows semantics (`remove_file` fallback on `rename` failure when destination exists) but executed only on Linux here.

### What does not work yet (honest status)
- `/api/library/..` is **not** reachable over live TCP yet until the 1-line delegating arm is added to `crates/desktop-server/src/http_srv.rs` (left untouched per `RULES.md` and `f10-library.md`). All route handlers and `AuthGuard::authorize_api` behavior are tested directly in `crates/desktop-server/tests/library_routes.rs`.
- `Library::attach_to_session` attaches papers backed by verified blobs in `<data_dir>/blobs/` or `<data_dir>/library/blobs/`. Attaching a `user_library` file that exists only at an external `file_path` (not yet hashed/cached as a blob) returns an error until F13 adds external-root path containment and reading.

### Bugs / oddities noticed but not fixed
- `crates/desktop-server/src/http_srv.rs:645`: `percent_decode` translates `'+'` to `' '` in URL **paths** (not only query strings). If a DOI contains a literal `'+'` and a client does not percent-encode it as `%2B`, `http_srv.rs` will decode it to a space before passing `rest` to `handle_library` (which will then fail `PaperKey::parse` with 400). Left untouched because `http_srv.rs` is over 600 lines and out of scope; UI callers should `encodeURIComponent` keys (which encodes `+` as `%2B`).
- Pre-existing clippy warnings listed above (all in untouched files).

### Contradictions or gaps noticed in the design
- None that block F10. One minor schema alignment detail: `00-overview.md` 2.7 states that `fulltext` gains `origin = 'user_library'` where a row names a file path/size/mtime and optional `sha256` instead of a blob, whereas F3's `fulltext` table had a strict `CHECK (length(raw_blob) = 64 ...)` constraint and `FulltextCache::get_bytes` treated non-64-hex `raw_blob` as `Error::BadHash`. Relaxing the CHECK strictly for `origin = 'user_library'` in migration v1 and returning `Ok(None)` for empty `raw_blob` in `FulltextCache::get_bytes` reconciles both cleanly without breaking any F3 contract.
