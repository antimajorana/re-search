# Report: `f13a-sync-integrity` — review and hardening of the F13a checkpoint

- **Base commit:** `cf2544dc9c98718ab7b311e7a28609fee745062a` (`origin/gemini/work/fixed`)
- **Merged F13a checkpoint:** `eea6e2cc4ef11d8e7e95e038b3f6c444d50865d0` from `origin/gemini/modes/lib-f13a-f16a-f15` (merge commit `f467c19`, bringing commits `fca79a2`, `3270e5e`, and `eea6e2c`; F16a and F15 were left untouched on the original branch)
- **Work branch:** `work/f13a-sync-integrity` (pushed to `gemini/modes/f13a-sync-integrity`)
- **Toolchain on this machine:** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `cargo 1.100.0 (3acc79c0c 2026-11-05)`, Python `3.12.16` on Linux x86_64
- **Unchanged guard rails:** `Cargo.lock`, `crates/research-tools/tests/golden/tool_defs.txt`, `crates/research-tools/tests/golden/tool_profiles.txt`, `crates/desktop-server/src/http_srv.rs`, `crates/desktop-server/src/routes_session.rs`, `crates/desktop-server/src/turn_runner.rs`, `crates/agent/src/resume.rs`, `crates/ledger/Cargo.toml`, and `crates/policy/Cargo.toml` are untouched. `crates/ledger` and `crates/policy` remain free of `tokio`, `rmcp`, and `windows-sys`. Zero `unsafe` blocks added.

---

## 1. Exact F13a code and test checkpoints & attribution

### Commit checkpoints on this branch
- **Merged upstream F13a checkpoint** (from `origin/gemini/modes/lib-f13a-f16a-f15`, merge commit `f467c19`):
  - `fca79a2` — cross-source listing and cleanup helpers in `LibraryDb` / `Library`
  - `3270e5e` — read-only paper sources, Kandinsky vault and folder adapters, 5-signal dedupe, frontmatter reader, sync loop, and `paper_sources_sync.rs`
  - `eea6e2c` — `/api/sources` route dispatcher (`routes_sources.rs`) and `sources_routes.rs`
- **Hardening code and test checkpoints** (this brief):
  - `778d438` — split oversized F13a modules (`source_folder.rs`, `source_kandinsky.rs`, `source_sync.rs`) under the 600-line ceiling while preserving Kandinsky `353368b` headers
  - `f9d4086` — atomic `BEGIN IMMEDIATE` source sync commit (`apply_source_sync_batch`) and overlap-safe atomic source unregistration (`unregister_source_atomic`) in `crates/ledger`, with unit tests (`library_db_sync_tests.rs`)
  - `c8f98ff` — two-phase `prepare_source_sync` / `commit_source_sync`, pure read-only `preview_source`, exact-dedupe `dirty` tracking, and atomic `remove_source` wired through `research-tools` and `desktop-server`
  - `036f8cb` — synthetic integration tests for atomic rollback, interleaved/concurrent syncs, overlapping folder/vault unregistration, unrelated fulltext origin preservation, and route preview safety (`crates/research-tools/tests/source_sync_integrity.rs` and `crates/desktop-server/tests/sources_routes.rs`)
  - `3554ca2` — independent audit (`pro`) hardening pass: non-clobbering `register_source_atomic`, live `alias` convergence inside `BEGIN IMMEDIATE`, re-keyed orphan `fulltext` pruning, pure-SQLite zero-disk-IO `refresh_fts_metadata_in_tx`, version-aware `attach_source_paper_to_session`, Windows case-insensitive `resolve_contained_abs`, surfaced DB errors in `routes_sources.rs`, and `clippy::too_many_arguments` cleanup in `source_dedupe_tests.rs`

### Attribution & contributor history
- All ported/split F13a files retain the exact Kandinsky `353368b` attribution header (`// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 353368b), the same author's project.`): `source_dedupe.rs`, `source_folder.rs`, `source_folder_parse.rs`, `source_kandinsky.rs`, `source_kandinsky_resolve.rs`, `source_sync.rs`, and `source_sync_reconcile.rs`.
- **Mixed contributor history note:** As noted in the brief and already verified by the integrator, the upstream Kandinsky source files carry mixed contributor history (Kandinsky's `docs/THIRD-PARTY.md` records Atlas lineage; Kandinsky is `AGPL-3.0-or-later`, re:Search is `AGPL-3.0-only`). No third-party paper text, real user folders, notes, or paper files are included anywhere.

---

## 2. What was implemented and hardened (file inventory & line counts)

All 25 touched/added source files relative to `origin/gemini/work/fixed` are strictly at or under 600 lines (verified by `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`):

### `crates/ledger`
- `crates/ledger/src/library_rows.rs` (312 lines): added `SourceSyncSnapshot`, `PaperUpdateMode`, `SourceSyncPaperUpdate`, and `SourceSyncBatch` (including `stale_user_paths`, `stale_user_fulltext_keys`, and `affected_fulltext_keys`).
- `crates/ledger/src/library_db_query.rs` (344 lines): exposed `pub(crate) fn refresh_fts_with_conn` and added pure-SQLite `pub(crate) fn refresh_fts_metadata_in_tx` so sync commits update `paper_fts` metadata (`title`, `authors`, `abstract`) inside `BEGIN IMMEDIATE` without reading `paper_text` blobs from disk.
- `crates/ledger/src/library_db_sync.rs` (577 lines, new):
  - `ImmediateTx` RAII `BEGIN IMMEDIATE` / `COMMIT` / `Drop` `ROLLBACK` guard (zero `unsafe`, fails closed on nested transactions).
  - `LibraryDb::register_source_atomic`: idempotent when `(id, kind, root, settings_json)` matches an existing source (preserving `last_sync_ms` and `state`); fails closed if `id` already exists with different config.
  - `LibraryDb::apply_source_sync_batch`: single atomic `BEGIN IMMEDIATE` transaction across `source_item`, `paper`, `alias`, `fulltext`, `paper_fts`, and the `source` (`last_sync_ms`, `state`) marker. Performs upfront validation before locking, verifies optimistic CAS on `SourceSyncSnapshot` (rejecting stale scans and refusing to resurrect removed sources), advances `last_sync_ms` strictly monotonically (`max(ts, prev + 1)`), resolves live aliases inside the transaction so concurrent syncs of two different sources converge on one canonical `paper` row, surfaces all alias/constraint errors (rolling back cleanly without advancing `last_sync_ms` or setting `synced`), and performs zero disk I/O.
  - `LibraryDb::unregister_source_atomic`: single atomic `BEGIN IMMEDIATE` transaction that deletes `source` (cascading `source_item`), checks all remaining active (`state <> 'missing'`) `source_item`s across other registered sources, preserves any `fulltext` path still indexed by another source, preserves every `fulltext` row from unrelated origins (`origin <> 'user_library'`), restores fallback `user_library` fulltext rows from surviving sources when a displaced `(paper_key, version)` has another copy, and handles retries idempotently (`Ok(false)` when already absent).
  - `LibraryDb::upsert_user_library_fulltext_preserve_other_origins` and `prune_unreferenced_user_fulltext_atomic`.
- `crates/ledger/src/library_db_sync_ft.rs` (392 lines, new): overlap-safe fulltext helpers (`ActiveFilesIndex`, `collect_source_paths_and_keys`, `collect_active_source_files`, `upsert_user_fulltext_in_tx` preserving unrelated origins and active TeX over PDF, `prune_unreferenced_user_fulltext_in_tx`, `prune_rekeyed_user_fulltext_in_tx`, `restore_surviving_user_fulltext_in_tx`, `normalize_index_path`).
- `crates/ledger/src/library_db_ext.rs` (388 lines): delegated `delete_user_library_fulltext_for_paths` to `prune_unreferenced_user_fulltext_atomic` so direct callers are also atomic and overlap-safe.
- `crates/ledger/src/library_db_sync_tests.rs` (583 lines, new): 4 focused unit tests in `ledger` covering atomic rollback on alias FK failure, optimistic CAS conflict rejection, monotonic `last_sync_ms` advancement, removed-source non-resurrection, overlapping path preservation + fallback re-pointing + unrelated origin preservation + retry idempotency, `register_source_atomic` idempotency/conflict, live alias convergence, and re-keyed orphan fulltext pruning.
- `crates/ledger/src/lib.rs` (517 lines): declared `mod library_db_sync;` and `mod library_db_sync_ft;`.

### `crates/research-tools`
- `crates/research-tools/src/library.rs` (592 lines): exposed `register_source_atomic`, `apply_source_sync_batch`, `unregister_source`, and `upsert_user_library_fulltext`.
- `crates/research-tools/src/source_sync.rs` (366 lines): split out reconciliation into `source_sync_reconcile.rs`; introduced two-phase `prepare_source_sync` (all filesystem scanning, PDF parsing, and SHA-256 hashing strictly outside any DB write lock) and `commit_source_sync` (single atomic `apply_source_sync_batch`), pure read-only `preview_source` (zero DB writes), non-clobbering `register_source` / `register_source_with_status`, atomic overlap-safe `remove_source`, and fixed exact-dedupe `dirty` tracking.
- `crates/research-tools/src/source_sync_reconcile.rs` (474 lines, new): reconciliation, SHA-256 move detection, source-id validation/derivation, and `build_source_sync_batch` (assembling `SourceSyncBatch`, canonical + exact-merge aliases, `stale_user_paths`, `stale_user_fulltext_keys`, and `affected_fulltext_keys` with normalized path comparison).
- `crates/research-tools/src/paper_sources.rs` (565 lines): fixed Windows case-insensitive component prefix stripping in `resolve_contained_abs`; hardened `attach_source_paper_to_session` to avoid swallowing `ingest_bytes`/ledger errors, honor `version` in `pick_best_user_file`, use origin-preserving `upsert_user_library_fulltext` with the chosen item's version, and propagate DB errors instead of discarding them.
- `crates/research-tools/src/source_folder.rs` (517 lines) + `crates/research-tools/src/source_folder_parse.rs` (135 lines, new): split under 600 lines.
- `crates/research-tools/src/source_kandinsky.rs` (454 lines) + `crates/research-tools/src/source_kandinsky_resolve.rs` (297 lines, new): split under 600 lines (re-exporting `clean_ref_token`).
- `crates/research-tools/src/source_dedupe.rs` (453 lines), `crates/research-tools/src/source_dedupe_tests.rs` (221 lines; added `#[allow(clippy::too_many_arguments)]` on test helper `make_item` to clear a branch clippy error), `crates/research-tools/src/source_frontmatter.rs` (357 lines), `crates/research-tools/src/source_frontmatter_tests.rs` (133 lines).
- `crates/research-tools/src/lib.rs` (314 lines): declared private split modules `source_folder_parse`, `source_kandinsky_resolve`, `source_sync_reconcile`.
- `crates/research-tools/tests/paper_sources_sync.rs` (586 lines): preserved intact (all 4 existing dry-run, path containment, moved-file, duplicate, read-only, and session-quote tests unchanged and passing).
- `crates/research-tools/tests/source_sync_integrity.rs` (571 lines, new): 5 synthetic integration tests covering interrupted/failing sync rollback & recovery, interleaved stale-scan rejection & removed-source non-resurrection, multi-threaded concurrent `sync_source` + concurrent DB writes during scan, overlapping vault + folder + nested-folder unregistration (shared path preservation, TeX-over-PDF retention, fallback copy restoration, idempotent retries, byte-identical source trees), and unrelated fulltext origin preservation (`arxiv_eprint`, `arxiv_pdf`).

### `crates/desktop-server`
- `crates/desktop-server/src/routes_sources.rs` (272 lines): switched `POST /api/sources` (`accept: false`) to pure read-only `preview_source` (fixing a bug where preview previously called `register_source` + `remove_source` and could delete an already-registered source), rolled back newly created sources if initial `accept: true` sync fails without touching pre-existing sources, propagated DB errors in `handle_list_sources` (removing `.unwrap_or(0)`), mapped concurrent sync conflicts to HTTP 409, and avoided masking concurrent deletion with `.unwrap_or(source)`.
- `crates/desktop-server/src/lib.rs` (216 lines): exposes `pub mod routes_sources;`.
- `crates/desktop-server/tests/sources_routes.rs` (213 lines): preserved existing 2 tests and added `preview_does_not_clobber_or_delete_already_registered_source`.

---

## 3. Decisions taken alone, deviations, and assumptions

### Decisions
1. **No SQLite schema version bump (`LIBRARY_SCHEMA_VERSION = 1` unchanged):** Optimistic concurrency control uses the existing `source` columns (`kind`, `root`, `settings_json`, `added_ms`, `last_sync_ms`, `state`) paired with strictly monotonic `last_sync_ms` (`max(ts, prev + 1)` on every committed sync), so existing v1 databases work without migration.
2. **Fail-closed conflict reporting on interleaved same-source syncs:** Rather than silently retrying or last-writer-wins overwriting, `apply_source_sync_batch` rejects stale commits with `"concurrent sync conflict ..."` (mapped to HTTP 409 in `routes_sources.rs`). This guarantees old scan results never overwrite a newer update and never leave mixed partial state.
3. **RAII `ImmediateTx` on `&Connection`:** Keeps the existing `&Library` public signatures of `sync_source`, `remove_source`, `register_source`, and `sync_all_sources` backwards-compatible while enforcing atomic `BEGIN IMMEDIATE` transactions with automatic `ROLLBACK` on error/drop and zero `unsafe`.
4. **Two-phase `prepare_source_sync` / `commit_source_sync` API:** Cleanly separates expensive read-only filesystem/PDF/hash work from the short zero-disk-IO SQLite write transaction, and allows deterministic interleaving and fault-injection testing without adding `cfg(test)` hooks to product code.
5. **Preserve existing `UpsertMerge` update semantics for dirty notes:** When an item is dirty (e.g., user edited a Kandinsky vault note's title), non-empty incoming fields update `paper` (required by `00-overview.md` 2.11 and verified by `paper_sources_sync.rs`), while empty incoming fields preserve existing `paper` values and `ids_json` merges non-destructively (upgrading `10.48550/arxiv.*` to publisher DOIs).
6. **Hard independent audit (`pro` subagent):** Conducted after Commit 4; all critical/high findings (`register_source` clobbering, live cross-source alias convergence, re-keyed orphan fulltext cleanup, zero-disk-IO FTS metadata refresh, `attach_source_paper_to_session` version/error handling, Windows case-insensitive `resolve_contained_abs`, swallowed errors in `routes_sources.rs`, and `source_dedupe_tests.rs` clippy warning) were fixed in Commit 5 (`3554ca2`).

### Deviations from design
- Split `source_folder.rs`, `source_kandinsky.rs`, `source_sync.rs`, and `library_db_sync.rs` with small private sibling modules (`source_folder_parse.rs`, `source_kandinsky_resolve.rs`, `source_sync_reconcile.rs`, `library_db_sync_ft.rs`) to enforce the 600-line ceiling (`RULES.md` and `00-overview.md` 2.11 porting rule 4). All public types/functions remain exported from the original modules.
- `crates/desktop-server/src/http_srv.rs` was not edited (per `RULES.md` and the brief: the integrator registers the dispatcher after F5).

### Assumptions
- SQLite `WAL` mode + `busy_timeout(10s)` (already configured in `LibraryDb::open`) is assumed sufficient for serializing short zero-disk-IO `BEGIN IMMEDIATE` commit transactions across concurrent handles.
- Windows case-insensitive path matching in `normalize_index_path` and `strip_root_components` uses ASCII case folding (`eq_ignore_ascii_case` / `make_ascii_lowercase`), matching `WorkspaceSandbox::is_same_or_inside` in `crates/research-tools/src/sandbox.rs`.

---

## 4. Merge notes

### Required one-line registration in `crates/desktop-server/src/http_srv.rs` (after F5)
Inside `dispatch_api` (beside `routes_library::handle_library`):
```rust
(_, ["api", "sources", rest @ ..]) => write_api_res(
    &mut stream,
    crate::routes_sources::handle_sources(state, method, rest, &req.query, &req.body),
),
```
No changes to `AuthGuard::authorize_api` are needed; `/api/sources` is protected by header-token authentication and loopback `Origin` checks automatically.

### Existing files edited relative to `origin/gemini/work/fixed`
- `crates/ledger/src/lib.rs` (`f9d4086`): declare `mod library_db_sync;` and `mod library_db_sync_ft;`.
- `crates/ledger/src/library_rows.rs` (`f9d4086`, `3554ca2`): add `SourceSyncSnapshot`, `PaperUpdateMode`, `SourceSyncPaperUpdate`, `SourceSyncBatch`.
- `crates/ledger/src/library_db_ext.rs` (`fca79a2`, `f9d4086`): add cross-source helpers (`list_all_source_items`, `delete_source_item`) and delegate `delete_user_library_fulltext_for_paths` to `prune_unreferenced_user_fulltext_atomic`.
- `crates/ledger/src/library_db_query.rs` (`f9d4086`, `3554ca2`): `pub(crate)` on `refresh_fts_with_conn` and add `refresh_fts_metadata_in_tx`.
- `crates/research-tools/src/lib.rs` (`3270e5e`, `778d438`): declare F13a modules and private split submodules.
- `crates/research-tools/src/library.rs` (`fca79a2`, `c8f98ff`, `3554ca2`): add `list_all_source_items`, `delete_source_item`, `delete_user_library_fulltext_for_paths`, `has_seen`, `register_source_atomic`, `apply_source_sync_batch`, `unregister_source`, `upsert_user_library_fulltext`.
- `crates/desktop-server/src/lib.rs` (`eea6e2c`): expose `pub mod routes_sources;`.

### Coordination note with `origin/gemini/modes/lib-f13a-f16a-f15`
- This branch merges only up to the F13a checkpoint `eea6e2cc4ef11d8e7e95e038b3f6c444d50865d0`. When integrating F16a and F15 from `lib-f13a-f16a-f15`, rebase or merge those later commits on top of this hardened F13a base so the split F13a modules and atomic sync/unregister implementation are preserved.

---

## 5. Verification, exact test counts, and pre-existing Linux findings

### Commands run and exact results

| Command | Result |
|---|---|
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | **Pass** — `No touched source file is over 600 lines.` (max touched file: `crates/research-tools/src/library.rs` at 592 lines) |
| `git diff --stat origin/gemini/work/fixed -- Cargo.lock crates/research-tools/tests/golden/tool_defs.txt crates/research-tools/tests/golden/tool_profiles.txt crates/desktop-server/src/http_srv.rs crates/desktop-server/src/routes_session.rs crates/desktop-server/src/turn_runner.rs crates/agent/src/resume.rs crates/ledger/Cargo.toml crates/policy/Cargo.toml` | **Pass** — empty diff |
| `cargo clippy -p ledger --all-targets -- -D warnings` | **Pass** — 0 warnings |
| `cargo clippy -p research-tools --lib --tests -- -D warnings -A unused_imports` | **Pass** — 0 warnings (only untouched `tests/sandbox_and_fs.rs:6` triggers Linux `unused_imports`) |
| `cargo clippy -p desktop-server --test sources_routes -- -D warnings -A clippy::manual-unwrap-or` | **Pass** — 0 warnings (only untouched `evidence.rs:744` triggers `manual_unwrap_or`) |
| `cargo clippy --workspace --all-targets -- --no-deps` | **Pass** — 0 warnings in any added or modified file; only the 6 pre-existing warnings below remain |
| `cargo test -p ledger --no-fail-fast` | **Pass** — **31 passed, 0 failed, 0 ignored** (14 unit incl. 4 new `library_db_sync` tests + 8 `hash_chain` + 9 `schema_rejects`) |
| `cargo test -p research-tools --no-fail-fast` | **Pass** — **339 passed, 0 failed, 0 ignored** (incl. 4 `paper_sources_sync`, 5 new `source_sync_integrity`, 5 `library_store`, 3 `tool_def_golden`, 3 `tool_profile_golden`) |
| `cargo test -p desktop-server --no-fail-fast` | **Pass** — **193 passed, 0 failed, 0 ignored** (incl. 3 `sources_routes` and 4 `library_routes`) |
| `RE_SEARCH_PYTHON="$(command -v python3)" cargo test -p <crate> --no-fail-fast` across remaining non-`cli` crates | **Pass** — `policy` 9/0, `tools` 35/0, `worker` 1/0, `chatgpt-auth` 28/0, `cli-route` 57/0, `export` 8/0, `agent` 114/0 (2 pre-existing ignored live eval tests), `mcp-server` 1/0 |
| `cargo test -p cli --no-fail-fast -- --skip cli_research_workspace_read_fork_diff_export_and_verify` | **Pass** — **35 passed, 0 failed, 0 ignored** (1 filtered out) |

### Pre-existing Linux issues (proven on base commit `cf2544dc9c98718ab7b311e7a28609fee745062a`)
1. **Pre-existing test hang:** `cli::tests::desktop_and_ext::cli_research_workspace_read_fork_diff_export_and_verify` (`crates/cli/tests/desktop_and_ext.rs:97`) timed out (`exit=124` after 20s) on a clean detached worktree at `origin/gemini/work/fixed` (`cf2544d`) as well as on this branch. Left untouched (`crates/cli` is out of scope).
2. **Pre-existing clippy warnings (6 total, all in untouched files, matching `reports/f10-library.md`):**
   - `crates/desktop-server/src/evidence.rs:744` (`clippy::manual_unwrap_or`, documented in `RULES.md`)
   - `crates/research-tools/tests/sandbox_and_fs.rs:6` (`unused_imports` on Linux)
   - `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` (`unused_imports` on Linux)
   - `crates/agent/src/guardrails.rs:335` (`clippy::items_after_test_module`)
   - `crates/agent/tests/eval_support/manifest.rs:289` (`clippy::match_like_matches_macro`)
   - `crates/agent/tests/eval_support/sheet.rs:102` (`clippy::unnecessary_map_or`)

### What could not be compiled or run on Linux
- `crates/desktop-app` and `cfg(windows)` code paths/tests (including the `#[cfg(windows)]` case-folding branches in `normalize_index_path`, `normalize_sync_path`, and `strip_root_components` in `paper_sources.rs`).

### Honest status & bugs/oddities noticed but not fixed
- `/api/sources/..` is not yet reachable over live HTTP until the integrator adds the 1-line arm to `crates/desktop-server/src/http_srv.rs` after F5.
- `crates/desktop-server/src/routes_sources.rs:39-44`: custom source IDs literally named `"duplicates"` or `"sync"` are shadowed by `GET /api/sources/duplicates` and `POST /api/sources/sync` (pre-existing from `eea6e2c`; noted for F5/UI validation).
- `crates/research-tools/src/source_sync.rs:338-346`: `SyncReport.exact_duplicates` / `duplicate_groups` reflect cross-source dedupe over `combined` (all registered sources), not filtered solely to groups touching the currently synced source (pre-existing from `3270e5e`; left unchanged to preserve report shape).
- `crates/research-tools/src/source_sync_reconcile.rs:159-168`: when a moved file is matched by SHA-256, `prev.ids`, `prev.version`, `prev.authors`, and `prev.year` are retained from `prev` rather than refreshed from the renamed filename (pre-existing from `3270e5e`; minor).
- `crates/cli/tests/desktop_and_ext.rs:97`: `cli_research_workspace_read_fork_diff_export_and_verify` hangs on Linux on both `cf2544d` and this branch (untouched, out of scope).
