# Report: `f15-writeup-investigate` — F15 Investigate Write-Up Adapter (Reading Notes)

- **Branch:** `gemini/modes/f15-writeup-investigate` (local `work/f15-writeup-investigate`)
- **Base commit (merged via `--no-ff`):** `2104137a228ae7948ee1d5ef6228d2e30304b461` (`origin/gemini/work/fixed`, PR #70; includes `f15-writeup-solve` PR #57, `f16b-lessons` PR #69, and Investigate `V2–V18`)
- **Earlier branch checkpoint preserved (fast-forward ancestry):** `41aaf1289014d42370decd874a206287c20fcc6b` (from initial base `24be58e6797679cfa423dcc16ea2073b54288714`)
- **Date:** 2026-11-28
- **Status:** Complete — Checkpoints 1 & 2 implemented, merged onto `origin/gemini/work/fixed`, wired across `export`, `desktop-server` HTTP, `cli`, and desktop Investigate UI (`writeup_btn.js`), and hardened after an independent Gemini 4 Carbon audit.

---

## 1. What was implemented (by checkpoint & file)

### Checkpoint 1 — Core Investigate reading-notes adapter (`export`)
- `crates/export/src/writeup_paper.rs` (**429 lines**): Public entrypoints (`build_writeup`, `build_writeup_with_options`, `PaperWriteupOptions`, `PaperWriteupBuild`, `is_permissive_licence`) and internal `build_writeup_from_verified` reusing the verified `(Ledger, Vec<Event>)` pair. Cross-checks `source_id`, `paper_key`, and `builder` between `investigation.skeleton` and its content-addressed blob, validates `files_root_sha256` and per-file SHA-256 digests, builds the mandatory status line (`paper_key (version). Structure from ... (builder, quality — reason). Task summary`), guards `status_line` against forbidden claim words, and assembles a validated `Writeup` with zero model calls.
- `crates/export/src/writeup_paper_meta.rs` (**593 lines**): Fail-closed licence allow-list (`is_permissive_licence` accepting exact `CC-BY*` / `CC0*` / `Public Domain` and rejecting `nonexclusive-distrib`, `NC`, `ND`, `SA`, negated, unknown, or absent licences), deterministic licence precedence (`investigation.paper` → `source.ingested` → read-only `Library::get_paper`), PDF physical page + OCR index (`PdfPageIndex`, handling `text_layer`, `scan_text_layer`, `none`, `empty_text_layer`, and future F14b `"ocr"`), F16a override folding (`lessons.db` + `investigation.override` / `investigation.node_override` with `node_fp` check), verified source loading, clean `\begin{...}`/`\end{...}`/`\input{...}` statement span extraction (preserving `node.spans` order and returning `None` on 2-line empty wrappers), and bibliography resolution preferring user `source_item.citekey` and `investigation.cited_lookup` `PaperRef`s.
- `crates/export/src/writeup_paper_nodes.rs` (**565 lines**): Deterministic Outline, Main Results (user `ReaderPin::MainResult` / `NotMainResult` override model roles; Orient-aware fallback to theorem-like skeleton nodes), Additional Annotated Nodes & Reader Notes (including `SkNodeKind::Section`-attached reader notes), licence-gated verbatim statements (`<= 300`-char `WriteupQuote` vs permissive `CC-BY` full `WriteupBlock::Statement`), mapped `head_cites` emission even without `authors_claim`, `include_notes` gating on both user notes and F16a override notes, forbidden-word guarding on model prose (`gist`, `purpose`, `hypothesis.text`), and mandatory non-removable check-table rows.
- `crates/export/src/writeup_paper_anchors.rs` (**363 lines**): Re-verifies every Layer-2 anchor through F4 `quote_core::verify_quote_ref` before emitting claims or context cards, omits ungrounded claims/cards with check-table disclosure, enforces `ProseBasis::Model` + `(model)` on all model-written context cards (`Paper`, `External`, `ModelBackground`), maps `head_cites` through resolved citekeys, resolves external anchors via both `card.bib_key` and external `source_id` (`Prerequisite`/`Symbol` cards without `bib_key`), and looks up physical PDF pages + `(OCR)` per `anchor.source_id`.
- `crates/export/src/writeup_paper_ext.rs` (**289 lines**): Multi-source `PdfPageIndex` map across all `source.ingested` events, `investigation.cited_lookup` and external `source.ingested` bibliography resolution (`ExternalSourceBib`), and word-boundary forbidden-word detector (`find_forbidden_investigate_word`).
- `crates/export/src/writeup_md.rs` (**185 lines**): Neutralizes raw TeX/Markdown math delimiters (`$$`, `$`, `\[`, `\]`, `\(`, `\)`) when rendering `ocr == true` `WriteupBlock::Quote` blocks while keeping `WriteupQuote.text` hash-bound (`investigate.md` rule 6.21, `00-overview.md` 8.3 Q3).

### Checkpoint 2 — Shared dispatcher, HTTP/CLI integration, Investigate UI mount, and tests
- `crates/export/src/lib.rs` (**382 lines**): Merged `f15-writeup-solve` and `writeup_paper` cleanly; wired `Mode::Investigate => writeup_paper::build_writeup_from_verified(...)` into `build_session_writeup` alongside `Mode::Solve`, retaining `origin/gemini/work/fixed`'s `sanitize_bib_stem`, `.bib` collision guards, and `"ledger verification failed"` error contract (HTTP 409).
- `crates/desktop-server/assets/investigation_share_ui.js` (**335 lines**): Mounted `assets/writeup_btn.js` cleanly in the Investigate toolbar inside `mountInvestigationShare` (keeping `investigation_v3_shell.js` untouched at 597 lines), passing `apiMod.getCsrfToken()` for `X-ReSearch-Token`, tracking `activeWriteupCtrl` globally (plus `destroyInvestigationWriteup()` on mode switch / hidden shell) so fresh toolbar nodes created by `renderShell` cancel stale downloads, and observing `shellHost` `childList` updates via `MutationObserver` so node/focus/map clicks preserve Share + Write-up controls. Zero `innerHTML`, zero `"verified"`.
- `crates/export/tests/writeup_paper.rs` (**580 lines**) & `crates/export/tests/writeup_paper_support/mod.rs` (**565 lines**): 7 integration / Set W tests (skeleton-only + `tex_audit`, user pins/overrides/notes/inline toggle, fail-closed licence allow-list + CC-BY full statements, PDF/OCR physical pages & zero math delimiters, wording property across all 4 `AuthorsClaimLabel` variants + byte determinism, source/blob/anchor tamper refusal, shared `writeup_session_*` dispatch).
- `crates/export/tests/writeup_paper_audit.rs` (**303 lines**): 2 Carbon audit regression tests (external `Prerequisite` context card without `bib_key` on an OCR PDF with `$`/`$$` delimiters; `Section`-attached reader notes, override note privacy under `include_notes: false`, and disallowed model wording omission).
- `crates/export/tests/writeup_seam.rs` (**160 lines**) & `crates/desktop-server/tests/writeup_solve_routes.rs` (**265 lines**): Updated skeletonless Investigate session expectations from `"write-up adapter not built yet"` (501) to fail-closed `"investigation.skeleton"` (400), keeping Explore strictly asserting 501 `"write-up adapter not built yet"`.
- `crates/desktop-server/tests/writeup_investigate_routes.rs` (**416 lines**): 2 live desktop-server integration tests covering end-to-end Investigate session creation, live HTTP `GET /api/sessions/{id}/writeup` (`tex`/`md`/`bib`, `inline_marks=0`, `X-ReSearch-Token` & `Authorization: Bearer`, download/sandbox/nosniff/no-store headers, byte determinism, fail-closed source tamper), V15 Share disjointness (`/investigation/export?format=md` → 400 pointing to `writeup_paper.rs`, `rsmap` → 200), and strict `AuthGuard::authorize_api` cookie-only rejection (401).
- `crates/cli/tests/writeup_investigate_cli.rs` (**268 lines**): End-to-end CLI test running `re-search writeup` on an Investigate session for `tex` (with sidecar `.bib` + `tex_audit`), `md --no-inline-marks`, and `bib`.
- `crates/desktop-server/tests/investigation_share_ui.test.mjs` (**333 lines**): Added Node tests for `writeup_btn.js` mounting in Investigate toolbar, `X-ReSearch-Token` header auth, Blob URL lifecycle, fresh-toolbar stale download cancellation, and `destroyInvestigationWriteup()` on mode leave.

---

## 2. Independent Gemini 4 Carbon Audit & Fixes Applied

An independent Gemini 4 Carbon (`pro`) audit reviewed all files against `origin/gemini/work/fixed` and identified 5 `HIGH`, 3 `MEDIUM`, and 5 `LOW` items; all owned findings were fixed and regression-tested across commits `cc3a395`, `c827083`, and `f7a703f`:
1. **External `Prerequisite`/`Symbol` V12 context cards without `card.bib_key` & `cited_lookup` resolution (HIGH):** Fixed via `writeup_paper_ext.rs` indexing `investigation.cited_lookup` and external `source.ingested` receipts by `source_id` and `bib_key`.
2. **External PDF anchors dropping physical page numbers and `(OCR)` tags (HIGH):** Fixed via per-`source_id` `PdfPageIndex` map across all `source.ingested` events.
3. **Missing `(model)` on `Paper`/`External` context cards & unguarded forbidden words in model prose / `status_line` (HIGH):** Fixed by marking all context-card prose `ProseBasis::Model`, adding `find_forbidden_investigate_word` on `status_line` (fail-closed) and model prose (omit with check-table disclosure).
4. **OCR `WriteupBlock::Quote` raw `$`/`$$` math delimiters in Markdown (HIGH):** Fixed in `crates/export/src/writeup_md.rs` (`neutralize_ocr_math` when `q.ocr == true`) while keeping `WriteupQuote.text` hash-verified.
5. **Investigate UI `writeupCtrl` stale-download leak across fresh `renderShell` toolbars & disappearing button on node clicks (HIGH):** Fixed in `investigation_share_ui.js` via module-level `activeWriteupCtrl`, `destroyInvestigationWriteup()` on mode leave / hidden shell, and a `MutationObserver` on `shellHost`.
6. **Multi-file `\input`, span ordering, 2-line empty wrapper, `Section` notes, mapped `head_cites`, Orient-aware fallback note, `ov.note` privacy, and `skel_ev` `paper_key`/`builder` cross-checks (MEDIUM/LOW):** All fixed in `writeup_paper*.rs`.

---

## 3. Decisions, Deviations, and Assumptions

### Decisions taken independently
1. **Fail-closed licence allow-list:** Only exact `CC-BY*`, `CC0*`, and `Public Domain` permit full `> 300`-char statements; `nonexclusive-distrib`, `NC`, `ND`, `SA`, negated, unknown, and absent licences enforce `<= 300`-char `WriteupQuote` excerpts.
2. **Model prose forbidden-word handling:** Forbidden claim words (`verified`, `proved`, `novel`, `new` outside `"Authors: New (model)"`) in `status_line` fail closed with an error; in model-authored reading fields (`gist`, `purpose`, `relation_note`, `hypothesis.text`, `card.title`/`body`), the offending model field is omitted with a check-table disclosure row so zero forbidden words ever leak while keeping the user's paper reading notes exportable.
3. **UI mount location:** Mounted `writeup_btn.js` in `crates/desktop-server/assets/investigation_share_ui.js` (335 lines) rather than `investigation_v3_shell.js` (already 597 lines, 3 lines from ceiling).
4. **Updating placeholder `"not built yet"` tests:** Updated `s-inv` (`writeup_seam.rs:51-52`) and `s-wu-inv` (`writeup_solve_routes.rs:241-244`) from `"write-up adapter not built yet"` (501) to fail-closed `"investigation.skeleton"` (400), matching the precedent of `solve-s12-loop` (`7dba77d`).

### Deviations from the design
- Split the Investigate adapter across five files (`writeup_paper.rs`, `writeup_paper_meta.rs`, `writeup_paper_nodes.rs`, `writeup_paper_anchors.rs`, `writeup_paper_ext.rs`) so every file stays `<= 600` lines (max 593 lines).
- Edited `crates/export/src/writeup_md.rs` (+15/-1 lines, commit `cc3a395`) to close the Markdown OCR math-delimiter leak (`investigate.md` rule 6.21) without breaking `WriteupQuote.quote_sha256`.

### Assumptions
- Future F14b OCR receipts will record `"basis": "ocr"` (or `"ocr": true`) on `source.ingested.pdf_pages[]`; `PdfPageIndex::from_receipt` parses raw JSON strings permissively so F14a and F14b both work without schema breakage.

---

## 4. Merge notes

### Edits to existing files on this branch (vs `origin/gemini/work/fixed`)
- `crates/export/src/lib.rs`: registered `pub mod writeup_paper;` (+ private helper modules `writeup_paper_{anchors,ext,meta,nodes}`) and wired `Mode::Investigate` into `build_session_writeup`.
- `crates/export/src/writeup_md.rs`: neutralized TeX/Markdown math delimiters when rendering `q.ocr == true` `WriteupBlock::Quote` blocks.
- `crates/export/tests/writeup_seam.rs` & `crates/desktop-server/tests/writeup_solve_routes.rs`: updated skeletonless `Mode::Investigate` expectation from `"write-up adapter not built yet"` (501) to fail-closed `"investigation.skeleton"` (400); `Mode::Explore` remains 501 `"write-up adapter not built yet"`.
- `crates/desktop-server/assets/investigation_share_ui.js` & `crates/desktop-server/tests/investigation_share_ui.test.mjs`: mounted `assets/writeup_btn.js` in the Investigate toolbar with stale-download cleanup and `MutationObserver` persistence across `renderShell` updates.

### Optional follow-up seam for `include_notes=0` over HTTP/CLI
- `export::writeup_paper::build_writeup_with_options` already supports `PaperWriteupOptions { include_notes: false }` (verified in `writeup_paper.rs` and `writeup_paper_audit.rs`). If desired in a future PR, `routes_writeup.rs` and `cmd_writeup.rs` (owned by `f15-writeup-solve`, left untouched here) can parse `?include_notes=0` / `--no-notes` and forward it for `Mode::Investigate`.

---

## 5. Verification, exact test counts, and Linux limits

### Commands run (non-root `uid=1001(antigravity)`, `RE_SEARCH_PYTHON` unset)

| Command | Result |
|---|---|
| `id` | **Non-root** (`uid=1001(antigravity)`) |
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | **Pass** — `No touched source file is over 600 lines.` (max touched file: `crates/export/src/writeup_paper_meta.rs`, 593 lines) |
| `unset RE_SEARCH_PYTHON && cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` | **Pass** — 0 warnings, 0 errors |
| `unset RE_SEARCH_PYTHON && cargo test --workspace --locked --no-fail-fast` | **Pass** — **175 suites: 1317 passed, 0 failed, 2 ignored** (`writeup_paper`: 7/7, `writeup_paper_audit`: 2/2, `writeup_seam`: 2/2, `writeup_core`: 4/4, `writeup_investigate_routes`: 2/2, `writeup_solve_routes`: 2/2, `writeup_investigate_cli`: 1/1) |
| `unset RE_SEARCH_PYTHON && cargo test -p research-tools --test tool_def_golden --locked` | **Pass** — **3 passed, 0 failed, 0 ignored** |
| `for f in crates/desktop-server/assets/*.js; do node --check "$f" \|\| exit 1; done` | **Pass** — 0 syntax errors |
| `node --test crates/desktop-server/tests/*.test.mjs` | **Pass** — **245 passed, 0 failed, 0 skipped** |
| Forbidden-file diff check (`Cargo.lock`, `tests/golden/`, `latex.rs`, `paper_map*.rs`, `investigation_share.rs`, `writeup_solve.rs`, `http_srv.rs`, `routes_session.rs`, `auth.rs`, `.github/`) | **Pass** — empty diff |

### Pre-existing items
- **2 ignored tests** in `cargo test --workspace`: pre-existing live-network eval tests in `crates/agent/tests/eval_harness.rs` (`file_sheet_from_json`, `live_manifests_validate`). Zero new ignored or skipped tests added.
- **1 pre-existing clippy lint** suppressed via `-A clippy::manual_unwrap_or` per `RULES.md` (`crates/desktop-server/src/evidence.rs:744`).

### What could not be compiled or run on Linux
- `crates/desktop-app` and `cfg(windows)` code paths/tests (CI runs on Windows 11).
- Running an external `pdflatex` binary on generated `.tex` (verified deterministically with `research_tools::audit_tex_sources` / `tex_audit`: 0 blockers, 0 errors, citations & verbatim quotes verified).
- Headless Chromium browser click-through of `investigation_share_ui.js`'s `MutationObserver` on a live desktop server (covered in Node unit tests for `mountInvestigationShare`, `writeup_btn.js`, and `destroyInvestigationWriteup`).

---

## 6. Honest status & out-of-scope bugs/limitations noticed (not fixed)

1. **`crates/desktop-server/src/routes_writeup.rs:39-61` & `crates/cli/src/cmd_writeup.rs:14-48`:** Do not yet accept `?include_notes=0` / `--no-notes`, so HTTP and CLI Investigate write-ups currently default to `include_notes: true` (`PaperWriteupOptions::default()`).
2. **`crates/research-tools/src/library.rs:37` & `crates/research-tools/src/lessons.rs:24`:** Lack `SQLITE_OPEN_READ_ONLY` constructors; `writeup_paper_meta.rs` checks `.is_file()` before opening so missing DBs are never created, but existing DBs still open via SQLite `READ_WRITE` constructors.
3. **`crates/export/src/writeup.rs:440`:** `build_verified_quote` stores `verified.excerpt` (the full physical line) beside `quote_ref.quote_sha256` (which hashes the clipped `<= 300`-char text), which fails `Writeup::validate()` if a single source line exceeds 300 characters (`writeup_paper` avoids this by constructing `WriteupQuote` directly from the clipped verified text).
4. **`crates/export/src/writeup_tex.rs:93` vs `crates/research-tools/src/tex_audit.rs:426-437`:** `writeup_tex.rs` runs `escape_latex` inside `\begin{quote}...\end{quote}` while `tex_audit` matches literal normalized substrings without unescaping LaTeX escapes (`\$`, `\textbackslash{}`), so quotes containing raw TeX commands can trigger `unverified_quote` in `tex_audit`.
5. **`crates/export/src/writeup_tex.rs:58` & `crates/export/src/writeup_md.rs:39`:** `include_inline_badges: false` omits inline `"(model)"` suffixes on `ProseBasis::Model` blocks (the mandatory closing check table still labels them `"Model statement (model)"` and `"Authors: ... (model)"` stays intact).
6. **`crates/desktop-server/assets/investigation_v3_shell.js` (597 lines):** Left untouched (3 lines below ceiling); `writeup_btn.js` is mounted via `investigation_share_ui.js`.
