# Report: `lib-f13a-f16a-f15` (library lane — packages F13a, F16a, and F15 core)

- **Base commit:** `4e0fbaaf4ef51cf83e7fa3d65b484cb4709a3431` (`origin/gemini/work/fixed`)
- **Work branch:** `work/lib-f13a-f16a-f15` (pushed incrementally after F13a, F16a, and F15 core to `gemini/modes/lib-f13a-f16a-f15`)
- **Toolchain on this machine:** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `cargo 1.100.0 (3acc79c0c 2026-11-05)`, Python `3.12.16` (with `sympy`, `numpy`, `mpmath`), Node `v22.23.5` on Linux x86_64
- **Unchanged guard rails:** `Cargo.lock` untouched (0 new dependencies); `crates/research-tools/tests/golden/tool_defs.txt` untouched (no tool definitions added or modified); `crates/export/src/latex.rs` (771 lines) untouched; `crates/policy` and `crates/ledger` remain free of `tokio`, `rmcp`, and `windows-sys`; `crates/research-tools` remains free of `rusqlite`; `http_srv.rs`, `routes_session.rs`, `turn_runner.rs`, `resume.rs`, `index.html`, and `app.js` untouched; `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` passes (all touched source files <= 600 lines).

---

## 1. What was implemented (package by package)

### Package 1: F13a — Paper sources, sync, Kandinsky-vault adapter, folder adapter, dedupe, and `/api/sources`

#### `crates/ledger` and `crates/research-tools` (F10 library integration helpers)
- `crates/ledger/src/library_db_ext.rs` (396 lines, +32): added `list_all_source_items`, `delete_source_item`, and `delete_user_library_fulltext_for_paths` for cross-source deduplication and clean source removal without touching disk files or sessions.
- `crates/research-tools/src/library.rs` (562 lines, +30): exposed `list_all_source_items`, `delete_source_item`, `delete_user_library_fulltext_for_paths`, and `has_seen`.

#### `crates/research-tools` (new F13a modules)
- `crates/research-tools/src/paper_sources.rs` (519 lines, new): read-only `PaperSourceAdapter` trait, `ScanContext` / `ScannedItem` / `ScanWarning` / `ScanOutput`, `TitleLookupHook` trait (hook left unwired/`None` for future approval-gated network title lookups), strict path containment helpers (`canonicalize_source_root`, `resolve_contained_rel`, `resolve_contained_abs`, `read_contained_bytes`, `hash_contained_file`, `file_meta_without_reading`) refusing `..`, NUL/control bytes, symlinks, and Windows NTFS junctions (`sandbox::is_symlink_or_junction`), F2 `derive_paper_key`, and `attach_source_paper_to_session` (reads strictly contained user-library files, prefers TeX over PDF, hashes on open, and pins bytes into the session via `ingest_bytes`).
- `crates/research-tools/src/source_frontmatter.rs` (357 lines, new) + `crates/research-tools/src/source_frontmatter_tests.rs` (133 lines, new): zero-dependency flat Markdown front-matter YAML and flat TOML reader (for Kandinsky `<citekey>.md`, legacy `info.yaml`, and `.kandinsky/plugin-settings/catalog-papers.toml`), plus ported reading tests for Kandinsky `entry.rs`.
- `crates/research-tools/src/source_kandinsky.rs` (578 lines, new) + `crates/research-tools/src/source_kandinsky_roots.rs` (167 lines, new): ported in part from Kandinsky (`353368b`: `entry.rs`, `external.rs`, `external_roots.rs`). Read-only `KandinskyVaultAdapter` reading `.kandinsky/plugin-settings/catalog-papers.toml` overlaid with `source.settings`, `<refs_dir>/<citekey>/<citekey>.md` (with `RENAMED` and `own()` semantics) or legacy `info.yaml`, vault-local and `external_pdf_roots` PDFs, and unpacked `arxiv_source_dir/<id>/` TeX sources. Never writes to the vault; refuses escaping paths and symlinks/junctions.
- `crates/research-tools/src/source_folder.rs` (517 lines, new) + `crates/research-tools/src/source_folder_meta.rs` (134 lines, new): ported in part from Kandinsky (`353368b`: `scan_roots.rs`, `external.rs`). Read-only `FolderAdapter` walking PDFs, `.tex`, `.tar.gz`/`.tgz`, and `.zip` files with subfolder collections, cloud-drive-safe `(path, size, mtime_ms)` incremental skip, F14a first-page + `/Info` extraction (`pdf_pages`, `pdf_firstpage`, `pdf_match`), and strict symlink/escape refusal.
- `crates/research-tools/src/source_dedupe.rs` (453 lines, new) + `crates/research-tools/src/source_dedupe_tests.rs` (221 lines, new): ported in part from Kandinsky (`353368b`: `inbox_dedupe.rs`). Five-signal duplicate detection: exact DOI (with `10.48550/arxiv.*` also matched on arXiv), base arXiv id, PDF SHA-256, and ISBN merge index rows and aliases; fuzzy title (Jaccard >= 0.88 on folded ASCII tokens) + first author surname + year within 1 marks `possible_duplicate` and **never merges**.
- `crates/research-tools/src/source_sync.rs` (508 lines, new) + `crates/research-tools/src/source_sync_store.rs` (159 lines, new): ported in part from Kandinsky (`353368b`: `scan_roots.rs`). `register_source`, `sync_source`, `sync_source_with_hook`, `sync_all_sources`, and `remove_source` with dry-run preview before acceptance, idempotent second sync, moved-file detection by SHA-256 keeping item identity, missing-file retention (`SourceItemState::Missing`), and F10 `Library` indexing (`paper`, `alias`, `source_item`, `fulltext` with `origin = "user_library"`).
- `crates/research-tools/tests/paper_sources_sync.rs` (586 lines, new): 4 integration/property tests with self-authored synthetic fixtures covering evaluation set B (30-entry Kandinsky vault + plain folder with moved file, edited note, deleted file, DOI duplicate, PDF SHA-256 duplicate, preprint+journal pair, unpacked TeX preferred over PDF, `..` and symlink escapes, fuzzy non-merging duplicate), whole-tree SHA-256 read-only property test across dry-run and two syncs, and session quote verification surviving source removal + `library.db` deletion + source folder deletion.

#### `crates/desktop-server` (F13a HTTP routes)
- `crates/desktop-server/src/routes_sources.rs` (250 lines, new): single dispatcher `handle_sources(state, method, rest, query, body) -> ApiResult` implementing `GET /api/sources`, `POST /api/sources` (preview-first by default when `accept: false`), `GET /api/sources/{id}`, `DELETE /api/sources/{id}`, `POST /api/sources/{id}/sync`, `GET /api/sources/{id}/items`, `POST /api/sources/sync`, and `GET /api/sources/duplicates`.
- `crates/desktop-server/tests/sources_routes.rs` (165 lines, new): 2 integration tests covering preview/accept/list/sync/duplicates/delete/unsupported-F13b-kind and `AuthGuard::authorize_api`.

---

### Package 2: F16a — Corrections reused by code (no model involved)

#### `crates/ledger`
- `crates/ledger/src/lessons_db.rs` (550 lines, new) + `crates/ledger/src/lessons_rows.rs` (62 lines, new) + `crates/ledger/src/lessons_db_tests.rs` (124 lines, new): SQLite persistence for `<data_dir>/library/lessons.db` (separate non-rebuildable file, WAL, `DEFENSIVE`, `trusted_schema = OFF`, forward-only `PRAGMA user_version = 1` refusing downgrade) storing `term_alias`, `symbol_default`, `convention_default`, `skeleton_override`, `ocr_overlay`, and `ingested_cursor`, plus atomic snapshot export/replace.
- `crates/ledger/src/lib.rs` (518 lines, +3): exports `lessons_db`, `LessonsDb`, `LESSONS_DB_NAME`, `LESSONS_SCHEMA_VERSION`.

#### `crates/research-tools`
- `crates/research-tools/src/lessons_schema.rs` (354 lines, new): typed enums/records (`TermAliasDecision`, `TermAliasRecord`, `SymbolDefault`, `ConventionDefault`, `SkeletonOverride`, `OcrOverlay`, `IngestCorrectionsReport`, `LessonsBackup`) and named constants (`OCR_OVERLAY_LABEL = "corrected by you"`, existing correction event kinds, ranking adjustments).
- `crates/research-tools/src/lessons.rs` (540 lines, new): `LessonsStore` CRUD, idempotent read-only ingestion from existing ledger correction events (`ingest_session_events`), and opt-in SHA-256-checksummed JSON backup/restore (`export_backup_bytes`, `restore_backup_bytes`).
- `crates/research-tools/src/lessons_select.rs` (341 lines, new): pure deterministic code-reuse selectors (zero model calls): `apply_mark_ranking` (`irrelevant` demoted but still listed, `key` boosted), `matching_key_paper_seeds`, `expand_query_aliases`, `is_pair_rejected`, `suggest_solve_defaults`, `apply_skeleton_overrides` (pure skeleton untouched), and `apply_ocr_overlays` (pinned text untouched, tagged `"corrected by you"`).
- `crates/research-tools/src/lib.rs` (312 lines, +12 total across F13a & F16a): exports F13a and F16a modules.
- `crates/research-tools/tests/lessons_reuse.rs` (473 lines, new): 2 tests — the required property test (`00-overview.md` 2.14 & 7.3) proving Solve tiers/badges, `quote_core` checks, and pure TeX skeletons are bit-for-bit identical with and without a populated `lessons.db`, plus evaluation set K scripted session pairs covering all 5 F16a correction reuse paths and backup tamper rejection.

#### `crates/desktop-server`
- `crates/desktop-server/src/routes_lessons.rs` (391 lines, new): single dispatcher `handle_lessons(state, method, rest, query, body) -> ApiResult` implementing `GET /api/lessons`, `POST /api/lessons/ingest/{session_id}`, alias/default/skeleton-override/OCR-overlay CRUD, opt-in `GET /api/lessons/backup?confirm=true`, and `POST /api/lessons/restore`.
- `crates/desktop-server/src/lib.rs` (217 lines, +2 total across F13a & F16a): exports `routes_sources` and `routes_lessons`.
- `crates/desktop-server/tests/lessons_routes.rs` (203 lines, new): 2 integration tests covering all `/api/lessons/..` routes (including opt-in backup enforcement) and `AuthGuard::authorize_api`.

---

### Package 3: F15 core — Write-up document model and writers

#### `crates/export`
- `crates/export/src/writeup.rs` (544 lines, new): mode-neutral `Writeup` document model (`WriteupMode`, `ProseBasis`, `HowChecked`, `QuoteLocator`, `WriteupQuote`, `WriteupBlock`, `WriteupSection`, `BibEntry`, `CheckTableRow`, `WriteupOptions`), single `writeup_wording` table delegating Solve tier labels directly to `research_tools::solve_status::Tier::label()` (`solve_badge.rs`), Gate G7 refusal for Solve Failed/Stale steps, `"verified"` wording restriction, and F4 `quote_core` quote verification/filtering (`build_verified_quote`, `filter_verified_quotes`).
- `crates/export/src/writeup_bib.rs` (218 lines, new): `resolve_bib_entry` (prefers user's own `source_item.citekey` from F10/F13 `Library`, else synthesizes a collision-free deterministic key) and `render_bibtex`.
- `crates/export/src/writeup_tex.rs` (226 lines, new): deterministic LaTeX writer (`render_writeup_tex`, `render_verified_writeup_tex`, `escape_latex`). Emits verbatim quote text inside `\begin{quote}...\end{quote}` and attribution/citation/locator/`(OCR)` immediately after so `tex_audit` verifies quote blocks cleanly. Always emits status line, coverage paragraph (when present), and "How each statement was checked" table even when `include_inline_badges == false`.
- `crates/export/src/writeup_md.rs` (171 lines, new): deterministic Markdown writer (`render_writeup_markdown`, `render_verified_writeup_markdown`) using the exact same `writeup_wording` table.
- `crates/export/src/lib.rs` (136 lines, +4): exports `writeup`, `writeup_bib`, `writeup_md`, `writeup_tex`. `crates/export/src/latex.rs` (771 lines) was **not touched**.
- `crates/export/tests/golden/writeup_sample.{tex,md,bib}` (63, 47, 16 lines, new): hand-built golden outputs for all three F15 core writers.
- `crates/export/tests/writeup_core.rs` (590 lines, new): 4 integration/property tests verifying golden outputs byte-for-byte + end-to-end `research_tools::audit_tex_sources` (`tex_audit`) validation (0 blockers, 0 errors, 2 bib keys indexed, verbatim quote verified), walking every `Tier` of `solve_badge.rs` and every non-Solve `HowChecked` variant across both writers, verifying F4 `quote_core` drops unmatched quotes before writing, and verifying mandatory status line / coverage paragraph / check table cannot be suppressed.

---

## 2. Kandinsky porting audit (`00-overview.md` 2.11 rules 1–7)

- Source repository inspected locally at `/home/antigravity/antimajorana/Kandinsky` at exact commit `353368b`.
- **Rule 2 (provenance history check):** Ran `git log --follow --format="%h %an <%ae> %ad %s" 353368b -- <file>` on all five ported files (`entry.rs`, `external.rs`, `external_roots.rs`, `scan_roots.rs`, `inbox_dedupe.rs`). Every commit was authored solely by `antimajorana` (plus one bugfix commit by `Antigravity` on `external.rs`), with zero Atlas or third-party copyright notices.
- **Rule 1 (header):** Every ported file (`source_kandinsky.rs`, `source_kandinsky_roots.rs`, `source_folder.rs`, `source_folder_meta.rs`, `source_dedupe.rs`, `source_sync.rs`, `source_sync_store.rs`) begins with `// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 353368b), the same author's project.`
- **Rule 3 (zero new dependencies & flat YAML/TOML reader limits):** Replaced `kandinsky-text`, `kandinsky-plugin`, `serde_yaml`, and `toml` with `research-tools`'s `sandbox.rs` helpers and `source_frontmatter.rs`. **What `source_frontmatter.rs` explicitly does NOT read:** nested YAML mappings, flow mappings (`{k: v}`), YAML anchors/aliases (`&a`/`*a`), merge keys (`<<`), YAML tags (`!!str`), multi-document streams, or TOML nested `[table]` sections / inline tables. It reads flat top-level `key: value` scalars (unquoted, single-quoted, double-quoted), inline lists `[a, b]`, block lists (`- item`), block scalars (`|`/`|-`/`>`/`>-` for `abstractNote`), and flat TOML string/string-array keys (`dir`, `bib`, `external_pdf_roots`, `arxiv_source_dir`).
- **Rule 4 (600-line ceiling):** Split large ported modules (`scan_roots.rs` 1337 lines, `inbox_dedupe.rs` 912 lines, `external.rs` 499 lines) into <= 600-line files verified by `scripts/check_file_length.py`.
- **Rule 6 & 7:** Only the reading half was ported; Kandinsky's reading tests (RENAMED fields, legacy `info.yaml`, non-entry folder skip, wikilink stripping, 5-signal dedupe) are preserved with self-authored synthetic fixtures.

---

## 3. Decisions taken alone, deviations, and assumptions

### Decisions taken alone
1. **Stable `external_id` on moved folder files:** When a folder file moves from `a/x.pdf` to `b/y.pdf` and matches by SHA-256, `source_sync` preserves the original `source_item.external_id` and updates `files[0].path` and `collections`. Subsequent incremental syncs index `previous_by_path` as well as `previous_by_id`, so post-move syncs hit the `(path, size, mtime_ms)` fast path with zero content reads.
2. **All symlinks and junctions inside source roots refused:** `resolve_contained_rel`, `resolve_contained_abs`, `KandinskyVaultAdapter`, and `FolderAdapter` refuse all symbolic links and Windows NTFS directory junctions (`sandbox::is_symlink_or_junction`) inside registered roots (recording a `ScanWarning`) in addition to canonical prefix containment checks (`WorkspaceSandbox::is_same_or_inside`).
3. **First-page PDF extraction clamping in `source_folder.rs`:** `pdf_pages::extract_pdf_pages` rejects `page_range = (1, 5)` with `PdfError::InvalidPageRange { total_pages, .. }` on PDFs shorter than 5 pages; `source_folder.rs` catches `InvalidPageRange` when `total_pages > 0` and retries with `(1, total_pages.min(5))`, supporting both 1-page PDFs and >400-page books without reading past page 5.
4. **Opt-in `/api/lessons/backup` gate:** `GET /api/lessons/backup` requires `?confirm=true` (returning HTTP 400 otherwise) to enforce `00-overview.md` 2.14's rule that `lessons.db` is local-only and exported only when the user ticks/confirms it.
5. **TeX `\begin{quote}` layout for `tex_audit` compatibility:** `writeup_tex.rs` emits strictly the verbatim escaped quote inside `\begin{quote}...\end{quote}` and puts the attribution (`\noindent --- \cite{key} (locator) (OCR)`) immediately after `\end{quote}` because `research_tools::tex_scan::extract_tex_quote_blocks` audits the exact body of every `\begin{quote}` environment against ingested session sources.
6. **Own-tier verified note kept strictly in the check table:** Following `00-overview.md` 2.13 rule 4, inline statement badges emit only `how_checked.label()` (the effective tier), while `(own check: verified)` appears only in the "How each statement was checked" table beside `rests_on`. `Writeup::validate` also rejects `"verified"` in `status_line` for Solve when `final_effective_tier != Some(Tier::Verified)`, and unconditionally for Explore and Investigate.

### Deviations from the design
- Split `source_kandinsky.rs` (`+ source_kandinsky_roots.rs`), `source_folder.rs` (`+ source_folder_meta.rs`), `source_sync.rs` (`+ source_sync_store.rs`), and `crates/ledger/src/lessons_db.rs` (`+ lessons_rows.rs`) into small private sibling modules so every touched/added file stays strictly <= 600 lines. All public items are re-exported from the named modules.
- Did not edit `crates/desktop-server/src/http_srv.rs` (703 lines, forbidden by `RULES.md` and brief); both `routes_sources::handle_sources` and `routes_lessons::handle_lessons` are exported with their exact one-line registration comments.
- Did not wire network title metadata lookup in F13a (as required by brief: needs the approval card); left the `TitleLookupHook` trait and `sync_source_with_hook(..., None)` hook ready.

### Assumptions
- Kandinsky's vault settings file path is `.kandinsky/plugin-settings/catalog-papers.toml` (confirmed in Kandinsky `353368b` `plugins/beta/catalog-papers/engine/src/settings.rs:1` and `crates/kandinsky-core/src/external_roots.rs:42`), with defaults `dir = "refs/papers"` and `arxiv_source_dir = ".kandinsky/cache/arxiv-src"`.
- Future mode event payloads for Explore alias confirmations/rejections, Solve setup revisions, Investigate reader node overrides, and F14b OCR line corrections are assumed to match the event kind names and JSON field names listed in `lessons_schema.rs` (`EVENT_*` constants) and `lessons.rs` (`extract_*` helpers). Adding alternate field aliases later is local to `lessons.rs`.
- Moved-file detection by SHA-256 requires the previously indexed item to have a recorded `sha256` (from `hash_files: true`, Kandinsky's `pdfSha256`, or a prior session open); an unhashed cloud-drive file that was never read cannot be matched by content hash after being moved.

---

## 4. Merge notes

### Required one-line registrations in `crates/desktop-server/src/http_srv.rs`
Inside `dispatch_api` (after `/api/` auth and body reading, beside `routes_library` / `routes_subagents`), add:
```rust
(_, ["api", "sources", rest @ ..]) => write_api_res(
    &mut stream,
    crate::routes_sources::handle_sources(state, method, rest, &req.query, &req.body),
),
(_, ["api", "lessons", rest @ ..]) => write_api_res(
    &mut stream,
    crate::routes_lessons::handle_lessons(state, method, rest, &req.query, &req.body),
),
```
No changes to `AuthGuard::authorize_api` are required: header-token and loopback `Origin` checks apply automatically to all `/api/` paths.

### Edits to existing files
- `crates/ledger/src/library_db_ext.rs` (commit `fca79a2`): added `list_all_source_items`, `delete_source_item`, `delete_user_library_fulltext_for_paths`.
- `crates/research-tools/src/library.rs` (commit `fca79a2`): exposed those three methods plus `has_seen`.
- `crates/ledger/src/lib.rs` (commit `9cc94e4`): exported `lessons_db` (+ private `lessons_rows`) and re-exported `LessonsDb`, `LESSONS_DB_NAME`, `LESSONS_SCHEMA_VERSION`.
- `crates/research-tools/src/lib.rs` (commits `3270e5e`, `b156b5d`): declared F13a and F16a modules.
- `crates/desktop-server/src/lib.rs` (commits `eea6e2c`, `6af4519`): exported `routes_sources` and `routes_lessons`.
- `crates/export/src/lib.rs` (commit `d044988`): exported `writeup`, `writeup_bib`, `writeup_md`, `writeup_tex`.

### Integration points for other briefs / later packages
- **F5 / `http_srv.rs`:** wire `/api/sources` and `/api/lessons` dispatchers above.
- **F13a UI (`assets/library_sources.js`, lane D):** call `/api/sources` (preview with `accept: false`, confirm with `accept: true`), `/api/sources/{id}/sync`, `/api/sources/duplicates`, `DELETE /api/sources/{id}`.
- **F13b (`source_bibtex.rs`, `source_zotero.rs`):** implement `PaperSourceAdapter` for `SourceKind::Bibtex` and `SourceKind::Zotero` and register them in `source_sync::register_source` and `sync_source_with_hook`.
- **F13 approval-gated title lookup:** implement `paper_sources::TitleLookupHook` backed by F14a's `PaperFetchMetadataAdapter` + `Router::try_call` approval card and pass it to `sync_source_with_hook`.
- **F10 / `routes_library.rs`:** `POST /api/library/{key}/attach` can delegate to `paper_sources::attach_source_paper_to_session` so user-library papers (not yet cached as blobs) attach seamlessly into sessions.
- **F16b (lessons shown to a model):** add a forward-only v2 migration to `crates/ledger/src/lessons_db.rs` for the `user_lesson` table and `lessons.supplied` ledger event; F16a's `lessons.db` tables remain compatible.
- **Mode write-up adapters (`writeup_solve.rs`, `writeup_idea.rs`, `writeup_paper.rs`):** build `export::writeup::Writeup` values using `writeup_bib::resolve_bib_entry` and `writeup::build_verified_quote`, then call `render_verified_writeup_tex` / `render_verified_writeup_markdown` / `render_bibtex`.

---

## 5. Verification, test counts, and pre-existing findings

### Commands run

| Command | Result |
|---|---|
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | **Pass** — `No touched source file is over 600 lines.` (max touched file: `crates/export/tests/writeup_core.rs` at 590 lines) |
| `git diff --stat origin/gemini/work/fixed -- Cargo.lock crates/research-tools/tests/golden/tool_defs.txt crates/export/src/latex.rs crates/desktop-server/src/http_srv.rs crates/desktop-server/src/routes_session.rs crates/desktop-server/src/turn_runner.rs crates/agent/src/resume.rs crates/desktop-server/assets/index.html crates/desktop-server/assets/app.js` | **Pass** — empty diff |
| `cargo clippy -p ledger --all-targets -- -D warnings` | **Pass** — 0 warnings |
| `cargo clippy -p research-tools --lib --test lessons_reuse --test paper_sources_sync --test tool_def_golden -- -D warnings` | **Pass** — 0 warnings |
| `cargo clippy -p desktop-server --test lessons_routes --test sources_routes -- -D warnings -A clippy::manual-unwrap-or` | **Pass** — 0 warnings |
| `cargo clippy -p export --all-targets -- -D warnings` | **Pass** — 0 warnings |
| `cargo clippy --workspace --all-targets -- --no-deps` | **Pass (exact base-commit match)** — 6 pre-existing warnings only, 0 warnings in any added or modified file |
| `RE_SEARCH_PYTHON="$(command -v python3)" cargo test -p export --no-fail-fast` | **Pass** — **12 passed, 0 failed, 0 ignored** (8 existing export/Python replay tests + 4 new `writeup_core` tests) |
| `cargo test --workspace --no-fail-fast` | **Pass** — **801 passed, 0 failed, 2 ignored** (2 ignored are pre-existing live eval tests in `crates/agent/tests/eval_harness.rs`; +20 new tests across F13a, F16a, and F15 core) |

### Pre-existing clippy warnings (same 6 warnings already recorded on `origin/gemini/work/fixed` in `reports/f10-library.md` and `reports/f14a-pdf-core.md`)
1. `crates/desktop-server/src/evidence.rs:744` — `clippy::manual_unwrap_or` (also documented in `RULES.md`).
2. `crates/agent/tests/eval_support/manifest.rs:289` — `clippy::match_like_matches_macro`.
3. `crates/agent/tests/eval_support/sheet.rs:102` — `clippy::unnecessary_map_or`.
4. `crates/research-tools/tests/sandbox_and_fs.rs:6` — `unused_imports` (`TEX_AUDIT`, `is_symlink_or_junction`, Linux build).
5. `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` — `unused_imports` (`BufRead`, `BufReader`, Linux build).
6. `crates/agent/src/guardrails.rs:335` — `clippy::items_after_test_module`.

### What could not be compiled or run on Linux
- `crates/desktop-app` and `cfg(windows)` code paths/tests (Windows NTFS junction detection in `sandbox::is_symlink_or_junction`, AppContainer sandbox, Windows verbatim path handling, Windows Playwright tests).
- Running `pdflatex` to compile `writeup_sample.tex` (no TeX Live installation checked/invoked here; structural, citation, label, escaping, and verbatim quote validity was verified by `research_tools::audit_tex_sources`).
- Running `RE_SEARCH_PYTHON=$(command -v python3) cargo test --workspace` hung inside pre-existing `crates/cli/tests/desktop_and_ext.rs` (`cli_research_workspace_read_fork_diff_export_and_verify`, > 60 s); running `cargo test --workspace --no-fail-fast` without `RE_SEARCH_PYTHON` and `RE_SEARCH_PYTHON="$(command -v python3)" cargo test -p export --no-fail-fast` separately (matching `reports/f10-library.md`) passed cleanly (801 + 12 passed, 0 failed).

### Honest status: what does not work yet (by scope)
- `/api/sources/..` and `/api/lessons/..` are not reachable over live TCP until the two 1-line arms are added to `crates/desktop-server/src/http_srv.rs`.
- BibTeX and Zotero adapters (`F13b`), sending papers back to Kandinsky (`F13c`), approval-gated network title lookup, UI panels (`library_sources.js`, `lessons.js`), model-supplied lessons (`F16b`), and per-mode write-up adapters (`writeup_solve.rs`, `writeup_idea.rs`, `writeup_paper.rs`) are out of scope for this brief.

### Bugs / oddities noticed but not fixed
- `crates/research-tools/src/pdf_pages.rs:386`: `extract_pdf_pages` returns `PdfError::InvalidPageRange` when `page_range = Some((1, 5))` is requested on a PDF with fewer than 5 pages instead of clamping to `total_pages`. Worked around cleanly in `source_folder.rs`; left `pdf_pages.rs` untouched as out of scope.
- `crates/cli/tests/desktop_and_ext.rs`: `cli_research_workspace_read_fork_diff_export_and_verify` stalls over 60 s on Linux when `RE_SEARCH_PYTHON` is exported globally to the workspace test run; left untouched as out of scope.
- `crates/desktop-server/src/original_pdf.rs:32` (already noted in `f14a-pdf-core.md`): still enforces `MAX_PDF_BYTES` (16 MiB) rather than `MAX_USER_PDF_BYTES` (64 MiB) when serving original PDFs.

### Contradictions or impossible items in the design
- None that blocked F13a, F16a, or F15 core.
