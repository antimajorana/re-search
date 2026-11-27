# Investigate V15 (`.rsmap` & Standalone HTML Export / Import) Report

- **Brief**: `docs/design/modes-2026-11/agents/inv-v15-share.md`
- **Branch**: `gemini/modes/inv-v15-share`
- **Base commit at branch creation**: `0ec99d316c70025a2acc73c2f9ea12414ea59a45` (`origin/gemini/work/fixed` at start; remote advanced to `374744ebf426f2081891faeaabb19a91e34a9158` after PR #46 V9 merged)
- **CP1 commit**: `78090e8ec40e5e98a3c369f2784a979bc323500c`

## Implementation Summary & File Counts

### CP1 (`crates/export`)
- `crates/export/src/lib.rs` (142 lines, +3): exports `paper_map`, `paper_map_html`, and `paper_map_import`.
- `crates/export/src/paper_map.rs` (523 lines): defines `.rsmap` schema v1 (`PaperMapExport`), conservative `is_open_licence` check (`CC-BY` / `CC0` / `Public Domain`), skeleton summary + deterministic skeleton nodes/edges/bib/symbols, Layer-2 gists/authors' claims/model nodes/model edges, clipped quotes (`<= 300` Unicode scalar values via `research_tools::quote_core::clip_quote`, omitting surrounding long `excerpt` text), reserved `context_cards: []` ready for V12, and strict copyright/privacy gates (statements included only for open licences or explicit `include_full_statements: true`; private notes included only when `include_notes: true`; zero `"verified"`).
- `crates/export/src/paper_map_html.rs` (358 lines): pure self-contained HTML5 exporter with embedded CSS only (zero `<script>`, zero external fonts/stylesheets/CDNs), strict HTML escaping on all untrusted strings, F6 solid vs. dashed border cues, `"(model)"` labels, copyright/notes gates & status badges, and zero `"verified"`.
- `crates/export/src/paper_map_import.rs` (579 lines): Rule 6.11 fail-closed importer with zero network calls. Matches local sources via active session receipt, `Library::read_fulltext_bytes`, or verified ledger `source.ingested` receipts/blobs (`raw_sha256`, `files_root_sha256`, or `(paper_key, version)`). Re-ingests host-side via `ingest_bytes`, rebuilds `PaperSkeleton` deterministically, persists `investigation.skeleton` with both `"blob"` and `"raw_blob"` so `Ledger::verify_all().unreferenced_blobs == 0`, re-checks every gist/claim/model node/model edge/note via `quote_core::verify_quote` + span containment + node fingerprint + host cue extraction (`detect_cues_from_anchors`), drops mismatched items (`items_dropped`), tags surviving records `"imported"`, or returns read-only fallback with `"quotes not matched on this machine"` when no local source exists.
- `crates/export/tests/paper_map_share.rs` (418 lines): tests copyright/licence gates, private notes gate, 300-char quote clipping, HTML XSS escaping & zero-script/zero-external checks, round-trip import with `unreferenced_blobs == 0`, tampered quote drop count, missing local source banner, and zero `"verified"`.

### CP2 (`crates/desktop-server` + UI + Tests)
- `crates/desktop-server/src/lib.rs` (241 lines, +1): declares `mod investigation_share;`.
- `crates/desktop-server/src/investigation_share.rs` (216 lines): implements `GET /api/sessions/{id}/investigation/export?format=rsmap|html&include_statements=0|1&include_notes=0|1` (owner decision: `format=md` returns HTTP `400` pointing to F15 `writeup_paper.rs`; appends `investigation.exported { format, sha256 }`) and `POST /api/sessions/{id}/investigation/import` (runs Rule 6.11 import, appends `investigation.imported { format, sha256, source_match, items_matched, items_dropped, ... }`, includes sanitized read-only `preview` when `source_match == false`).
- `crates/desktop-server/src/api_dispatch.rs` (460 lines, +27): dispatches `GET .../investigation/export` (with `Content-Disposition: attachment`, `Cache-Control: no-store`, and strict `Content-Security-Policy`) and `POST .../investigation/import` before the catch-all `investigation` operation arm.
- `crates/desktop-server/src/assets.rs` (550 lines, +4): registers `/assets/investigation_share_ui.js`.
- `crates/desktop-server/assets/api.js` (540 lines, +18): adds `exportInvestigationFile` (fetch + `X-ReSearch-Token` + Blob download) and `importInvestigationMap`.
- `crates/desktop-server/assets/investigation_v3_shell.js` (599 lines, +1): adds side-effect import `import '/assets/investigation_share_ui.js';`.
- `crates/desktop-server/assets/investigation_share_ui.js` (268 lines): self-initializing share controls (`mountInvestigationShare`, `syncInvestigationEntryShare`), opt-in checkboxes defaulting unchecked (`include_statements`, `include_notes`), `.rsmap`/`.html` export & `.rsmap` import actions, persistent `"quotes not matched on this machine"` and `"Imported, N quotes could not be matched"` banners, strictly DOM-safe (`textContent` / `createElement` only, zero `"verified"`).
- `crates/desktop-server/tests/investigation_share_http.rs` (304 lines): HTTP integration test covering missing-token `401`, stream-cookie-only `401`, foreign `Origin` `403`, `format=md` `400`, default copyright/privacy gates vs. opt-in HTML/`.rsmap` export, `investigation.exported`/`investigation.imported` events, round-trip import, tampered quote drop banner, and missing-source read-only fallback.
- `crates/desktop-server/tests/investigation_share_ui.test.mjs` (201 lines): `node --test` checks for opt-in defaults/toggles, download/import flows, read-only & dropped-quotes banners, XSS escaping, zero raw HTML writes, and zero `"verified"`.

## Decisions & Merge Notes
1. **`investigation_v3_shell.js` 1-line hook (V9 headroom)**: Per Coordinator heads-up that PR #46 (V9) left `investigation_v3_shell.js` at 599 lines on `origin/gemini/work/fixed`, only 1 line (`import '/assets/investigation_share_ui.js';`) was added to `investigation_v3_shell.js` (598 -> 599 locally; <= 600 on merge with V9). All UI mounting runs automatically via `queueMicrotask` wrapping `__reSearchInvestigationV3.syncSession`.
2. **Auth on `GET .../investigation/export`**: `AuthGuard::is_cookie_route` (`crates/desktop-server/src/auth.rs`) is outside V15 file ownership and only allows `/events` and top-level `/export`. Accordingly, `investigation/export` strictly enforces `X-ReSearch-Token` (verified with a stream-cookie `401` test), and the browser client fetches bytes with `X-ReSearch-Token` and downloads via `Blob` URL.
3. **Orphan-blob guarantee on skeleton import**: `crates/ledger/src/verify.rs:280` checks `"raw_blob"` across all event kinds (`"blob"` is only checked in `tool.request`/`tool.result`). `paper_map_import.rs` stores both `"blob"` (expected by Investigate readers) and `"raw_blob"` on imported `investigation.skeleton` events so `Ledger::verify_all().unreferenced_blobs == 0`. Note observed (not modified): `crates/desktop-server/src/investigation_skeleton.rs:344` writes `"blob"` without `"raw_blob"`.
4. **`format=md` owner decision**: `GET /api/sessions/{id}/investigation/export?format=md` returns HTTP `400` directing callers to the F15 reading-notes write-up (`writeup_paper.rs`).
5. **Read-only reload persistence**: When `source_match == false`, `handle_import` stores a bounded, sanitized `preview` inside `investigation.imported` so reopening the session keeps rendering the `"quotes not matched on this machine"` banner and read-only card without extra endpoints.

## Verification Results
All commands below ran after the final code edit (`investigation_share_http.rs` `.rfind` cleanup):

- File length check (`py -3 scripts/check_file_length.py --base 0ec99d316c70025a2acc73c2f9ea12414ea59a45` & `--base origin/gemini/work/fixed`): **passed** (0 touched files > 600 lines; max 599 in `investigation_v3_shell.js`).
- JS syntax check (`node --check` across all 72 `crates/desktop-server/assets/*.js`): **72/72 passed**.
- Node test suite (`node --test crates/desktop-server/tests/*.test.mjs`): **210 passed, 0 failed** (`investigation_share_ui.test.mjs`: 2/2 passed).
- Python unit tests (`python3 -m unittest discover -s python -t python` in WSL): **33 passed, 0 failed**.
- Workspace Clippy (`/var/tmp/research-cargo.sh inv-v15 cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` in WSL): **0 errors, 0 warnings**.
- Workspace Cargo tests (`/var/tmp/research-cargo.sh inv-v15 cargo test --workspace --locked --no-fail-fast -j 2` in WSL with `$HOME/.venvs/bootloops/bin` and `nvm` node on `PATH`): **1131 passed, 0 failed, 2 ignored** across **140** suites (`export` `paper_map_share`: 2/2 passed; `desktop-server` `investigation_share_http`: 1/1 passed; `research-tools` `solve_s7_exprs`: 5/5 passed). The 2 ignored tests are pre-existing in `crates/agent/tests/eval_harness.rs`.
