# Report: `f13bc-sources-ext` — BibTeX & Zotero sources (F13b) and opt-in sending papers back (F13c)

- **Base commit:** `921edc8955e111565f6ddf7e9c0f1dfc88b3b170` (`origin/gemini/work/fixed`)
- **Work / push branch:** `gemini/modes/f13bc-sources-ext`
- **Toolchain on this machine:** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `cargo 1.100.0 (3acc79c0c 2026-11-05)`, Python `3.12.16`, Node `v22.23.5` on Linux x86_64
- **Unchanged guard rails:**
  - `Cargo.lock` and all `Cargo.toml` files untouched (0 new dependencies).
  - `crates/research-tools/tests/golden/tool_defs.txt` and `tool_profiles.txt` untouched (`tool_def_golden` 3/3 pass; `tool_profile_golden` 3/3 pass).
  - `crates/ledger` and `crates/policy` remain free of `tokio`, `rmcp`, and `windows-sys`.
  - `crates/research-tools` remains free of `rusqlite`.
  - Forbidden files untouched: `http_srv.rs`, `routes_session.rs`, `turn_runner.rs`, `resume.rs`, `index.html`, `app.js`, `library_schema.rs`, `ingest.rs`, `pdf_match.rs`, `pdf_pages.rs`, `source_kandinsky.rs`, `source_folder.rs`, `source_dedupe.rs`, `.github/`, and all other `docs/design/` files.
  - `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` passes (every touched/added source file is <= 600 lines).
  - Zero credentials, personal data, or third-party paper text anywhere in code, tests, fixtures, or this report; zero access to Vivaldi, its profile, or the owner's personal notes.

---

## 1. What was implemented (package by package)

### Package 1: F13b — Read-only BibTeX source adapter (`bibtex`)

- `crates/research-tools/src/source_bibtex.rs` (565 lines, new): `BibtexAdapter` implementing `PaperSourceAdapter` for `SourceKind::Bibtex`. Accepts either a `.bib` file path or a directory containing `refs.bib` / a single `.bib` file (`settings.bib` override supported). Parses `file` fields in Zotero Better BibTeX (`Description:/path/to/file.pdf:application/pdf`, `;`-separated multi-attachments with escaped `\:`, `\;`, `\\`), JabRef (`:rel/path.pdf:PDF`), and Kandinsky `refs.bib` formats; handles `file://`, `file://localhost/`, and Windows `file:///C:/...` URIs; resolves relative paths against the `.bib` directory and optional `settings.external_pdf_roots`; refuses any `..`, symlink, or Windows NTFS junction escape via F13a's containment helpers (`resolve_contained_rel`, `resolve_contained_abs`, `is_symlink_or_junction`). Deduplicates duplicate citekeys within a `.bib` file (`key#2`, ...) with a `ScanWarning`. Maps DOIs, arXiv IDs (`eprint`/`archiveprefix`/`arxiv`/`doi` `10.48550/arXiv.*`/`url`), ISBNs, years/dates, authors, and keywords/groups to F2 `PaperKey` while preserving user citekeys in `SourceItemRecord.citekey`. Never writes to the `.bib` file or attachment tree.
- `crates/research-tools/src/source_bibtex_parse.rs` (532 lines, new): zero-dependency deterministic pure-Rust `.bib` parser handling `@article`/`@inproceedings`/`@book`/`@misc`/etc., `@string` macro expansion and `#` concatenation, nested `{...}` and `"..."` values, parenthesis-delimited entries `@type(...)`, author splitting (`and` at brace depth 0, `"Last, First"` normalization), year/date extraction, and `file` field tokenization.
- `crates/research-tools/src/source_bibtex_tex.rs` (332 lines, new): TeX accent and ligature decoder (`\'e`, `\"o`, `{\\o}`, `\\ss`, etc., safely handling empty braced accents like `\\'{} Bar` without consuming following tokens) plus author/title cleanup helpers.
- `crates/research-tools/src/source_bibtex_tests.rs` (210 lines, new): unit tests for `.bib` parsing, `@string` macros, nested braces, TeX accent decoding, Zotero/JabRef/Kandinsky `file` fields, duplicate citekey handling, and symlink/`..` escape refusal.

### Package 2: F13b — Read-only Zotero SQLite source adapter (`zotero`)

- `crates/ledger/src/zotero_reader.rs` (595 lines, new): strictly read-only SQLite reader confining all `rusqlite` usage to `crates/ledger`. Rejects symlinks on `zotero.sqlite` / `better-bibtex.sqlite`, opens live `zotero.sqlite` exclusively with `SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_NO_MUTEX` + `PRAGMA query_only = ON` (never opens the user's live DB read-write). On `SQLITE_BUSY` / `SQLITE_LOCKED` (or WAL read-only lock), copies `zotero.sqlite` (and `-wal` when present, propagating copy errors and omitting `-shm` so SQLite rebuilds `-shm` cleanly in the isolated temp dir) into a unique temporary directory with a RAII drop guard, replays/reads the disposable temp copy with `PRAGMA query_only = ON`, retries once on concurrent WAL checkpoint race, and deletes the temporary snapshot on drop. Reads items (excluding deleted/trashed items), creators, item fields, collections (including full nested paths), tags, attachments, and optional Better BibTeX pinned citekeys from `better-bibtex.sqlite` (`citationkey` / `citekeys` schemas).
- `crates/ledger/src/zotero_fixture.rs` (254 lines, new): typed synthetic Zotero database fixture builder (`SyntheticZoteroDb`, `write_synthetic_zotero_dir`, `ZoteroExclusiveLock`) used by cross-crate integration tests in `research-tools` without exposing raw SQL or `rusqlite` outside `crates/ledger`. Refuses to overwrite any existing `zotero.sqlite`.
- `crates/ledger/src/zotero_reader_tests.rs` (247 lines, new): 4 unit tests covering direct read-only query (items, creators, collections, tags, attachments, BBT citekeys), `BEGIN EXCLUSIVE` lock snapshot fallback + temp-dir cleanup + byte-identical source verification, WAL-mode uncheckpointed-rows snapshot read + byte-identical source DB/`-wal`, and symlink refusal on `zotero.sqlite`.
- `crates/research-tools/src/source_zotero.rs` (498 lines, new): ported in part from Kandinsky (`353368b`, `plugins/beta/catalog-papers/engine/src/zotero_import.rs`). `ZoteroAdapter` implementing `PaperSourceAdapter` for `SourceKind::Zotero`: resolves `storage:<KEY>/<file>` under `<zotero_dir>/storage/<KEY>/` and `attachments:<rel>` under configured `base_attachment_path` / `zotmoov_dst_dir` (from `source.settings` or `prefs.js`, rejecting symlink/junction `prefs.js` via `is_symlink_or_junction`), enforces strict containment on every attachment, prefers pinned BBT citekeys over `Extra: Citation Key:` lines, extracts DOI/arXiv/ISBN/URL identifiers into F2 `PaperKey`, and preserves all collections and tags.
- `crates/research-tools/src/source_zotero_parse.rs` (223 lines, new): ported pure helpers from Kandinsky `zotero_import.rs` (identity folding stripping rich-text HTML markup/entities/diacritics, `Extra` field `Citation Key:` / `arXiv:` / `DOI:` parsing, `prefs.js` parser for `baseAttachmentPath` and `zotmoov.dst_dir`, Zotero identifier normalization).
- `crates/research-tools/src/source_zotero_tests.rs` (153 lines, new): ported read-only unit tests from Kandinsky `zotero_import.rs` (identity folding, `Extra`/`prefs.js` parsing, DOI/arXiv/URL resolution) plus containment refusal tests.
- `crates/research-tools/tests/sources_ext.rs` (580 lines, new): Set B integration tests for both BibTeX and Zotero through `register_source` / `sync_source` / `attach_source_paper_to_session`:
  - BibTeX: Zotero/JabRef/Kandinsky `file` formats, synthetic PDFs, moved PDF recognized by SHA-256, edited `.bib` metadata update, deleted PDF (`state = missing`), DOI + SHA-256 duplicates, preprint + journal version pair, `../escape.pdf` and out-of-root symlink refusal, whole-tree SHA-256 immutability across two syncs (second sync writes 0 rows), and C9 evidence independence (`ingest_bytes` + `quote_core::verify_quote` passes after removing the source and deleting `library.db` and the source tree).
  - Zotero: synthetic `zotero.sqlite` + `better-bibtex.sqlite` + `storage/<KEY>/` + linked `attachments:` base dir, active `BEGIN EXCLUSIVE` lock during sync, snapshot cleanup + whole-tree byte-identity check, and `..` / symlink attachment containment refusal.

### Package 3: F13c — Opt-in sending papers back (`source_export`, HTTP routes, UI controls)

- `crates/research-tools/src/source_export.rs` (491 lines, new):
  - **Export BibTeX (`export_bibtex_to_path`, `validate_export_destination`):** writes BibTeX atomically (temp sibling + `rename`) strictly to a user-chosen path; rejects `..`, control bytes, symlinks/junctions, any path inside any registered `PaperSource.root` or configured `external_pdf_roots` / `base_attachment_path` / `zotmoov_dst_dir`, and (defense-in-depth even when unregistered) walks every ancestor of the canonical parent directory refusing any path under a Kandinsky vault (`.kandinsky` marker) or Zotero data directory (`zotero.sqlite` marker). Re-verifies canonical parent and non-symlink destination right before `fs::rename` (TOCTOU guard).
  - **Send to Kandinsky (`validate_kandinsky_capture_target`, `build_kandinsky_capture_url`, `invoke_kandinsky_capture_cli`):** accepts only validated ASCII `arXiv:...`, `doi:...`, or `https://...` targets; rejects leading `-` (option injection), whitespace/control characters, shell metacharacters (`;`, `|`, `$`, `` ` ``, `&`, `<`, `>`, `\n`), and non-HTTPS schemes (`file://`, `javascript:`, `http://`, `data:`). Builds percent-encoded `kandinsky://capture-paper?target=...` deep links and invokes the Kandinsky binary directly via `std::process::Command::new(bin).args(["capture-paper", &target])` (strictly argv-only, never `cmd /c` or `sh -c`). Zero automatic/background write-back.
- `crates/research-tools/src/source_export_bib.rs` (207 lines, new): standalone BibTeX formatter (`format_library_papers_bibtex`) preferring stored `source_item.citekey` over app-generated deterministic collision-free keys, cleanly rewriting `@type{` or `@type(` citekeys, and escaping special TeX/BibTeX characters including trailing backslashes (`\textbackslash{}`).
- `crates/research-tools/src/source_export_tests.rs` (243 lines, new): unit tests covering user citekey preservation, escaping, atomic write, refusal of registered source roots and unregistered Kandinsky/Zotero subdirectories, Kandinsky target validation (arXiv/DOI/HTTPS accepted; option/shell/scheme/control/non-ASCII injections rejected), exact deep-link percent-encoding, and argv-only execution against a tiny compiled Rust stub binary (cross-platform, zero shell).
- `crates/desktop-server/src/routes_sources_ext.rs` (254 lines, new): single exported dispatcher `handle_sources_ext(state, method, rest, query, body) -> ApiResult` implementing `POST /api/sources/export/bibtex` (reusing F15's `export::writeup_bib::{resolve_bib_entry, render_bibtex}` plus `source_export::export_bibtex_to_path`) and `POST /api/sources/send/kandinsky` (supporting `mode: "deep_link"` or `mode: "cli"`). Both endpoints require explicit `user_initiated: true` in the JSON body (HTTP 400 otherwise) and sit behind `api_dispatch.rs`'s `AuthGuard::authorize_api` + loopback `Origin` checks.
- `crates/desktop-server/tests/sources_ext_routes.rs` (258 lines, new): 3 integration tests verifying BibTeX export route (citekey preservation, atomic write, source-root refusal, missing `user_initiated` refusal), Kandinsky send-back route (deep-link encoding, injection rejection, argv-only stub execution, missing `user_initiated` refusal), and live HTTP `AuthGuard` + `Origin` enforcement.
- `crates/desktop-server/assets/library_send_back.js` (339 lines, new): standalone ES module exporting `mountPaperSendBackActions(slotEl, paper, options)`, `createExportBibtexControl`, `createSendToKandinskyControl`, `validateClientCaptureTarget`, `derivePaperCaptureTarget`, `buildKandinskyDeepLink`, and named UI constants (`EXPORT_BIBTEX_LABEL`, `SEND_TO_KANDINSKY_LABEL`, `SEND_BACK_NOTICE`, etc.). Strictly opt-in on user click (`user_initiated: true`), zero `innerHTML` writes anywhere (DOM constructed strictly with `createElement` / `textContent`).
- `crates/desktop-server/tests/library_send_back.test.mjs` (179 lines, new): 2 Node tests covering XSS payloads (`<img onerror>`, `<script>`) in title/citekey/identifiers (0 `innerHTML` writes, 0 injected elements), client target validation, deep-link encoding, and opt-in `POST` payloads for both controls.

---

## 2. Kandinsky porting audit (`00-overview.md` 2.11 rules 1–7)

- Inspected local Kandinsky checkout at exact commit `353368b840fad8c95b7f48dd37c62c91b174db18` (`plugins/beta/catalog-papers/engine/src/zotero_import.rs`, 982 lines, and `docs/THIRD-PARTY.md`).
- **Rule 2 (provenance git-history audit):** Ran `git log --follow --stat --format="fuller" 353368b -- plugins/beta/catalog-papers/engine/src/zotero_import.rs`. All 5 commits (`1d221b2`, `082f6d6`, `ac5bad2`, `d700f0d`, `0d8b5c8`) were authored solely by `antimajorana`, with zero Atlas (Alneb) or third-party notices in the file.
- **Rule 1 (header):** `crates/research-tools/src/source_zotero.rs` and `crates/research-tools/src/source_zotero_parse.rs` both begin with `// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 353368b), the same author's project.`
- **Rule 3 (zero new dependencies):** Dropped all Kandinsky-internal crates; SQLite reading lives solely in `crates/ledger/src/zotero_reader.rs` using existing `rusqlite 0.32`.
- **Rule 4 (600-line ceiling):** Split Kandinsky's 982-line `zotero_import.rs` cleanly across `ledger/src/zotero_reader.rs` (595), `research-tools/src/source_zotero.rs` (498), `source_zotero_parse.rs` (223), and sibling test modules; every file is <= 600 lines.
- **Rule 5, 6 & 7:** Dropped all vault-writing, annotation, and note-generation code; ported only the read-only Zotero extraction/normalization half and preserved its read-only unit tests in `source_zotero_tests.rs`.

---

## 3. Independent audit findings and hardening (commit `5f32cb8`)

An independent read-only audit subagent reviewed the full diff against `origin/gemini/work/fixed` and identified several edge cases and security hardenings, all fixed and re-tested in commit `5f32cb8`:
1. **`zotero_reader.rs` WAL & error propagation:** Stopped swallowing non-lock `ReadErr::Other` errors into snapshot fallback; propagated `-wal` copy errors instead of `let _ =`; stopped copying `-shm` so SQLite cleanly rebuilds it in the isolated temp dir; added a single retry on concurrent WAL checkpoint race; allowed WAL replay on the disposable temp snapshot copy while still setting `PRAGMA query_only = ON` (live `zotero.sqlite` is never opened read-write); added `wal_mode_zotero_database_reads_uncheckpointed_rows_via_snapshot_cleanly` test.
2. **`source_export.rs` unregistered vault/Zotero subdirs & TOCTOU:** Walked all `canon_parent.ancestors()` for `.kandinsky` or `zotero.sqlite` markers (blocking writes into e.g. `<vault>/refs/papers/out.bib` or `<zotero>/storage/out.bib` even when unregistered); re-verified `fs::canonicalize(parent)` and `fs::symlink_metadata(&canon_dest)` right before `fs::rename`; restricted capture targets to ASCII.
3. **`source_sync_reconcile.rs` shared-attachment-path panic:** Replaced `prev_by_id.remove(&pid).unwrap()` with a filtered lookup (`prev_by_id.contains_key(*pid)`) + `if let Some((pid, prev))` so two items referencing the same attachment path cannot panic in Pass 1.
4. **`source_bibtex.rs` & `source_bibtex_tex.rs`:** Deduplicated duplicate citekeys within a `.bib` file (`key#2`, ... + warning); handled `file://localhost/...` and Windows `file:///C:/...` URIs; fixed `read_accent_arg` on empty braced accents (`\'{} Bar`); stripped verbatim prefixes and applied case-insensitive Windows component comparison in `is_syntactically_inside_missing`.
5. **`source_zotero.rs`:** Used `is_symlink_or_junction` instead of `is_symlink()` on `prefs.js`.
6. **`source_export_bib.rs`:** Located `@type{` or `@type(` properly in `rewrite_entry_citekey` and escaped trailing `\` to `\textbackslash{}` in `escape_bibtex_value`.
7. **`routes_sources_ext.rs`:** Required explicit `user_initiated: true` (`!= Some(true)` -> HTTP 400) rather than defaulting missing fields to `true`.

---

## 4. Decisions taken alone, deviations, and assumptions

### Decisions taken alone
1. **Explicit `user_initiated: true` required on F13c HTTP routes:** Both `POST /api/sources/export/bibtex` and `POST /api/sources/send/kandinsky` reject requests unless JSON `"user_initiated": true` is present, reinforcing the opt-in-on-explicit-user-click rule at the API boundary in addition to `AuthGuard::authorize_api` + loopback `Origin`.
2. **BibTeX source root flexibility:** `BibtexAdapter` accepts either a direct `.bib` file path or a directory containing `refs.bib` / a single `.bib` file (or `settings.bib`), while keeping `canonicalize_source_root` and allowed-root containment strictly bound to the `.bib` parent directory plus any configured `settings.external_pdf_roots`.
3. **Sync reconciliation fixes for F13b idempotency:** Updated `source_sync.rs` and `source_sync_reconcile.rs` so (a) Pass-1 `updated` counts are re-evaluated after exact dedupe merges (preventing a false `updated = 1` on the second sync of a DOI-deduplicated entry) and (b) `detect_same_id_moved_file` preserves `prev.key` when `hash_files: false` (preventing a false `updated = 1` on a post-move sync).
4. **F15 `writeup_bib` reuse in `desktop-server` + standalone formatter in `research-tools`:** `routes_sources_ext.rs` reuses F15's `export::writeup_bib::{resolve_bib_entry, render_bibtex}` as recommended by the brief, while `research-tools/src/source_export_bib.rs` provides a crate-local formatter because `research-tools` cannot depend on `export` without creating a Cargo dependency cycle (`export` depends on `research-tools`).
5. **Cross-platform argv-only CLI tests via `rustc` stub:** Following `crates/research-tools/tests/write_and_compile.rs`, CLI send-back tests compile a tiny temporary Rust executable with `rustc` (`std::env::consts::EXE_SUFFIX`) instead of a Unix shell script, keeping tests Windows-compatible and shell-free.
6. **Named constants for UI strings and endpoints:** `EXPORT_BIBTEX_LABEL`, `SEND_TO_KANDINSKY_LABEL`, `EXPORT_BIBTEX_ENDPOINT`, `SEND_KANDINSKY_ENDPOINT`, `KANDINSKY_DEEP_LINK_PREFIX`, and `SEND_BACK_NOTICE` are centralized at the top of `library_send_back.js` (and corresponding Rust constants in `source_export.rs`).

### Deviations from the design
- **Sub-module splits for the 600-line ceiling:** Split BibTeX into `source_bibtex.rs` + `source_bibtex_parse.rs` + `source_bibtex_tex.rs`, Zotero into `ledger::zotero_reader.rs` + `ledger::zotero_fixture.rs` + `source_zotero.rs` + `source_zotero_parse.rs`, and export into `source_export.rs` + `source_export_bib.rs`.
- **UI mount deferred to `library_sources.js` / `library.js`:** The brief noted mounting `assets/library_send_back.js` from F13a's `assets/library_sources.js` (or X17's `assets/library.js`), but neither file exists on base commit `921edc89` (`reports/lib-f13a-f16a-f15.md` confirmed UI panels were out of scope for that lane). `library_send_back.js` is implemented, served at `/assets/library_send_back.js`, and tested in Node, with the exact mount snippet documented under Merge notes below.
- **Crossref network title lookup untouched:** Deferred to Solve S5's approval card as required by brief §2 & §9.

### Assumptions
- Kandinsky's external entry point accepts `kandinsky capture-paper <id_or_url>` (argv) and `kandinsky://capture-paper?target=<encoded_id_or_url>[&title=<encoded_title>]` (deep link), per `00-overview.md` 2.11 and brief §1/§4. Exact CLI exit codes or custom flags beyond `capture-paper <target>` were not tested against a live Kandinsky binary.
- Zotero's SQLite schema (`items`, `itemTypes`, `itemData`, `itemDataValues`, `fields`, `itemCreators`, `creators`, `collections`, `collectionItems`, `tags`, `itemTags`, `itemAttachments`, `deletedItems`) and Better BibTeX's `better-bibtex.sqlite` (`citationkey` / `citekeys`) match Kandinsky `353368b`'s `zotero_import.rs`.
- Zotero database files are named `zotero.sqlite` and `better-bibtex.sqlite` (exact lowercase) inside the Zotero data directory.

---

## 5. Merge notes

### Edits to existing files (with commit SHAs)
- `crates/ledger/src/lib.rs` (526 lines, commit `af432e8`): added `pub mod zotero_fixture;`, `pub mod zotero_reader;` and re-exports (`read_zotero_dir`, `ZoteroSnapshot`).
- `crates/research-tools/src/lib.rs` (327 lines, commits `f046cdb`, `af88369`, `7653328`): declared `source_bibtex`, `source_bibtex_parse`, `source_bibtex_tex`, `source_zotero`, `source_zotero_parse`, `source_export`, `source_export_bib`.
- `crates/research-tools/src/paper_sources.rs` (581 lines, commits `f046cdb`, `af432e8`): added `SourceKind::Bibtex` and `SourceKind::Zotero` dispatch arms in `adapter_for_kind` and `source_allowed_roots`.
- `crates/research-tools/src/source_sync.rs` (379 lines, commits `f046cdb`, `af432e8`): allowed `.bib` file roots for `SourceKind::Bibtex`, supported `SourceKind::Zotero` root canonicalization, and re-evaluated Pass-1 `updated` flags after exact deduplication.
- `crates/research-tools/src/source_sync_reconcile.rs` (532 lines, commits `f046cdb`, `af432e8`, `5f32cb8`): fixed Pass-1 dedupe/move key preservation and guarded shared-attachment-path reconciliation against double-remove panic.
- `crates/desktop-server/src/lib.rs` (225 lines, commit `7653328`): exported `pub mod routes_sources_ext;`.
- `crates/desktop-server/src/assets.rs` (470 lines, commit `7653328`): registered `"/assets/library_send_back.js"` serving `assets/library_send_back.js`.
- `crates/desktop-server/src/api_dispatch.rs` (409 lines, commit `7653328`): added delegating match arms for `["api", "sources", "export", ..]` and `["api", "sources", "send", ..]` routing to `routes_sources_ext::handle_sources_ext` behind `AuthGuard::authorize_api` + loopback `Origin` checks.

### UI mount seam for future `assets/library_sources.js` / `assets/library.js`
When F13a's `assets/library_sources.js` or X17's `assets/library.js` adds a per-paper action slot, mount F13c with:
```js
import { mountPaperSendBackActions } from '/assets/library_send_back.js';

// Inside the paper row / detail panel renderer:
mountPaperSendBackActions(actionSlotEl, paper, {
  getToken: () => currentApiToken(),
});
```
No changes to `index.html` or `app.js` are required.

---

## 6. Verification commands and exact results

1. **File length gate:**
   - `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` -> **PASS** (`No touched source file is over 600 lines.`).
2. **Tool definition golden check:**
   - `cargo test -p research-tools --test tool_def_golden` -> **3 passed, 0 failed, 0 ignored**.
   - `cargo test -p research-tools --test tool_profile_golden` -> **3 passed, 0 failed, 0 ignored**.
3. **Targeted F13b / F13c tests:**
   - `cargo test -p ledger zotero` -> **4 passed, 0 failed, 0 ignored**.
   - `cargo test -p research-tools --lib source_` -> **18 passed, 0 failed, 0 ignored**.
   - `cargo test -p research-tools --test sources_ext --test paper_sources_sync --test source_sync_integrity --test tool_def_golden` -> **14 passed, 0 failed, 0 ignored** (`sources_ext` 2, `paper_sources_sync` 4, `source_sync_integrity` 5, `tool_def_golden` 3).
   - `cargo test -p desktop-server --test sources_ext_routes --test sources_routes` -> **7 passed, 0 failed, 0 ignored** (`sources_ext_routes` 3, `sources_routes` 4).
4. **Full workspace test suite:**
   - `cargo test --workspace --no-fail-fast` -> **1,008 passed, 0 failed, 2 ignored** across 124 test binaries/doctests (exit code 0). The 2 ignored tests are pre-existing live-tier evaluation tests (`file_sheet_from_json` needing `EVAL_SHEET_JSON` and `live_manifests_validate` needing `EVAL_LIVE_MANIFEST_DIR`).
   - `RE_SEARCH_PYTHON="$(command -v python3)" cargo test -p worker -p export --no-fail-fast` -> **26 passed, 0 failed, 0 ignored** (exit code 0).
5. **JS syntax & test suite:**
   - `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done && node --test crates/desktop-server/tests/*.test.mjs` -> **169 passed, 0 failed, 0 skipped** (including 2 new tests in `library_send_back.test.mjs`).
6. **Clippy:**
   - Targeted crates (`ledger --all-targets`, `research-tools --lib --test sources_ext --test paper_sources_sync --test source_sync_integrity --test tool_def_golden`, `desktop-server --test sources_ext_routes --test sources_routes -A clippy::manual-unwrap-or`) -> **0 warnings, exit code 0**.
   - `cargo clippy --workspace --all-targets -- -D warnings` fails **only** on pre-existing base warnings in untouched files (0 warnings in any file touched or added by this branch). Full warning list from `cargo clippy --workspace --all-targets -- --cap-lints warn`:
     - `crates/desktop-server/src/evidence.rs:744:40` (`clippy::manual_unwrap_or`, documented as known on base in `RULES.md` and `lib-f13a-f16a-f15.md`)
     - `crates/agent/tests/eval_support/manifest.rs:289:19` (`clippy::match_like_matches_macro`, pre-existing on base)
     - `crates/agent/tests/eval_support/sheet.rs:102:8` (`clippy::unnecessary_map_or`, pre-existing on base)
     - `crates/research-tools/tests/sandbox_and_fs.rs:6:5` (unused imports on Linux `not(windows)`, pre-existing on base)
     - `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6:15` (unused imports on Linux `not(windows)`, pre-existing on base)
     - `crates/agent/src/guardrails.rs:335:1` (`clippy::items_after_test_module`, pre-existing on base)

---

## 7. What could not be compiled or run on Linux

- `#[cfg(windows)]` branches (including Windows NTFS junction detection in `sandbox::is_symlink_or_junction`, Windows case-insensitive component comparison in `is_syntactically_inside_missing`, and Windows drive-letter `file:///C:/...` real filesystem resolution) were not compiled or executed on Linux; CI on Windows must verify them.
- Live integration against real Zotero Desktop (while actively holding OS file locks on Windows) and a real Kandinsky installation (`kandinsky capture-paper` / OS `kandinsky://` protocol handler) was not run (strictly synthetic fixtures and a compiled argv-only stub executable were used per `RULES.md`).

---

## 8. Noticed bugs in untouched files (not fixed)

1. `crates/desktop-server/src/routes_sources.rs:90` (and doc comment at line 79): when `POST /api/sources` omits `kind`, the error message still reads `` `kind` is required (`kandinsky_vault` or `folder`) `` even though `SourceKind::parse` and `paper_sources.rs` now support `bibtex` and `zotero` as well. Left untouched because `routes_sources.rs` is F13a-owned and `sources_routes.rs` tests against it.
2. `crates/export/src/writeup_bib.rs:192-218`: `rewrite_bibtex_key` uses `raw.find('{')`, which fails to rewrite parenthesis-delimited `@article(...)` entries or entries preceded by a comment containing `{`; and `escape_bibtex_value` does not escape a trailing backslash `\`, which could escape a field's closing `}`. Left untouched because `writeup_bib.rs` is F15-owned (our `source_export_bib.rs` handles both cases).

---

## 9. Contradictions / gaps and honest status of what does not work yet

- **Design vs base checkout gap:** Brief §6 expected `assets/library_sources.js` from F13a to already exist on base for mounting `library_send_back.js`, but F13a merged backend-only. Until `library_sources.js` or X17's `assets/library.js` lands and calls `mountPaperSendBackActions`, the F13c send-back buttons are served at `/assets/library_send_back.js` and tested in isolation, not yet visibly mounted in the browser UI.
- **Deferred hook:** Optional Crossref network metadata lookup by title is intentionally not wired (waits for Solve S5's approval card).
- **Case-sensitive Zotero filenames:** `zotero_reader.rs` looks up exact lowercase `zotero.sqlite` and `better-bibtex.sqlite`; on case-sensitive Linux filesystems, an atypically cased filename would not be detected.
