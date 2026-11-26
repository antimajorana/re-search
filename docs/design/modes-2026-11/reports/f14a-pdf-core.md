# Report: `f14a-pdf-core` (package F14a — PDF text core)

- **Base commit:** `e1517477cbceba0a6de6e4a143c34fe3b8b8411d` (`origin/gemini/work/fixed`)
- **Work branch:** `work/f14a-pdf-core` (pushed to `gemini/modes/f14a-pdf-core`)
- **Toolchain on this machine:** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `cargo 1.100.0 (3acc79c0c 2026-11-05)` on Linux x86_64
- **Unchanged guard rails:** `Cargo.lock` untouched (0 new dependencies); `crates/research-tools/tests/golden/tool_defs.txt` untouched; no tool definitions added or modified; `crates/policy` and `crates/ledger` untouched (free of `tokio`, `rmcp`, `windows-sys`); `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` passes (all touched source files `<= 600` lines).

---

## 1. What was implemented (package F14a)

### PDF text core, layout column splitting, scan detection, background job, and benchmark harness
- `crates/research-tools/src/pdf_pages.rs` (569 lines, new):
  - Reading file & page map (`PAGE_BREAK_CHAR = '\x0c'`, `PageBasis::{TextLayer, ScanTextLayer, None}`, `PageQualityMetrics`, `PageRecord`, `PageMap` with `page_of`, `record_of_line`, `record`, `page_lines`, `has_scanned_pages`, `pages_needing_ocr`, plus free helper `page_of_line`).
  - Pluggable `PdfExtractor` trait with two zero-new-dependency implementations over `pdf-extract` 0.12.1: `PdfExtractPlain` (`"pdf-extract/0.12.1"`) and `PdfExtractLayout` (`"pdf-extract-layout/0.12.1"`, default `DEFAULT_PDF_EXTRACTOR_ID`).
  - `PdfExtractOptions` keeping the existing 16 MiB / 400-page / 8 MiB defaults (`Default`), adding `PdfExtractOptions::user_supplied()` (`MAX_USER_PDF_BYTES = 64 MiB` for local scans) and `with_page_range(start, end)` so PDFs over 400 pages can be opened by a user-selected page range.
  - Scrubbing of NUL bytes, `\r\n`/`\r`, and embedded `\x0c` inside page streams so `\x0c` is strictly the inter-page separator; `catch_unwind` isolation around `pdf_extract`/`lopdf`; cooperative per-page cancellation (`PdfError::Cancelled`).
  - Re-exports `PdfExtractJob`, `PdfExtractJobSnapshot`, `PdfExtractStage` and the benchmark types (`ExtractorFixture`, `ExpectedSentence`, `OrderedSequence`, `ExtractorCaseMetrics`, `ExtractorBenchmarkReport`, `measure_extractor`).
- `crates/research-tools/src/pdf_pages_layout.rs` (347 lines, new):
  - `LayoutOutputDev` implementing `pdf_extract::OutputDev` over positioned glyphs `(x, y, width, font_size)` from `output_doc_page`, grouping glyphs into `y`-bands and pieces, detecting two-column vertical gutters in `[0.35*W, 0.65*W]` (tolerating up to 1–3 spanning titles/captions while requiring vertical column overlap), and emitting full-width bands plus left-column then right-column reading order.
- `crates/research-tools/src/pdf_pages_scan.rs` (361 lines, new):
  - `inspect_page_signals`: reads `/CropBox` / `/MediaBox` (with `/Parent` inheritance) and decodes page content streams (plus 1-level Form XObjects) to measure Image XObject coverage (`q`/`Q`/`cm`/`Do`), inline images (`BI`), and invisible OCR text rendering mode (`3 Tr`).
  - `classify_page_basis`: classifies each page into `text_layer`, `scan_text_layer`, or `none`, conservatively treating uncertain large-image + sparse-text pages as scans and sparse/empty pages as `none` (needing F14b OCR).
- `crates/research-tools/src/pdf_pages_job.rs` (232 lines, new):
  - Thread-capable `PdfExtractJob` (`start`, `start_with_extractor`, `run_sync`, `cancel`, `snapshot`, `is_finished`, `join`) mirroring F3's `FetchJob` shape with per-page `PdfExtractStage::ExtractingPage { page, total_pages }` progress.
- `crates/research-tools/src/pdf_pages_bench.rs` (255 lines, new):
  - Reusable comparison harness `measure_extractor(&extractor, &fixtures, &opts)` measuring elapsed time, failures, sentence match rate, wrong page links, scans reported as `text_layer`, empty page accuracy, reading-order errors, hyphenation, and ligatures — ready for a second dependency extractor (e.g. `pdf_oxide`) to plug in later.
- `crates/research-tools/src/pdf_pages_tests.rs` (119 lines, new): unit tests for `PageMap::page_of` boundaries/separators/empty pages, sanitization, unknown extractor id, pre-cancelled extraction, and byte-identical determinism.

### First-page metadata candidates and PDF Unicode normalization
- `crates/research-tools/src/pdf_firstpage.rs` (539 lines, new):
  - Ported in part from Kandinsky (`353368b`, `plugins/beta/catalog-papers/engine/src/firstpage.rs`; git history verified to be authored solely by `antimajorana` with no third-party/Atlas code).
  - Extracts heuristic `title`, `authors`, `year`, `dois`, `arxiv_ids` (validated through F2 `paper_key::split_arxiv_id`), and `isbns` (check-digit validated) across pages 1..=5, skipping blank pages and repository cover sheets, rejecting OCR-broken words, and recording exact 1-based `line`, `end_line`, and `page` provenance with `certain: false` on every candidate.
- `crates/research-tools/src/pdf_firstpage_norm.rs` (310 lines, new):
  - Ported in part from Kandinsky (`353368b`, `plugins/beta/catalog-papers/engine/src/pdf_meta.rs`; git history verified to be authored solely by `antimajorana` with no third-party/Atlas code).
  - `normalize_pdf_unicode` (NFKC ligatures, Adobe PUA, fullwidth ASCII, Unicode spaces, soft hyphens), `fold_diacritics`, `decode_pdf_string` (UTF-16BE/LE, UTF-8, PDFDocEncoding), `is_junk_pdf_title`, `parse_pdf_date_year`, `normalize_valid_isbn`, and panic-shielded `extract_pdf_info_candidates` reading `/Info` via `pdf_extract::Document`.
- `crates/research-tools/src/pdf_firstpage_tests.rs` (150 lines, new): unit tests with strictly self-authored synthetic fixtures covering Unicode normalization, junk titles, line provenance, cover-sheet skipping, OCR rejection, and `10.48550/arXiv.*` handling.

### PDF quote matching and `PaperKey` matching
- `crates/research-tools/src/pdf_match.rs` (589 lines, new):
  - PDF-normalized quote matching (`normalize_for_pdf_match`, `match_pdf_quote`, `quotes_match_pdf`): ignores ligatures, PUA/fullwidth glyphs, soft hyphens, both dropped (`eigen-\nvalue` -> `eigenvalue`) and preserved (`quasi-\nlocality` -> `quasi-locality`) line-end hyphens, typographic quotes/dashes, and whitespace wrapping without mutating stored text; maps matches back to exact raw byte/line spans, `PageMap` page/basis, and verbatim `stored_excerpt`.
  - Offline `match_candidates_offline`: resolves `FirstPageCandidates` strictly to F2 `PaperKey` / `PaperRef` (`arxiv:` -> `doi:` -> `file:<16-hex>`), preserving provenance line/page and `confirmed: false`.
  - Injectable `MetadataFetcher` trait + `lookup_candidates_with_fetcher` (verifying identifier match or Kandinsky's exact normalized title + first-author surname match) + `PaperFetchMetadataAdapter` built on F3's `paper_fetch::fetch_with_gate` and gated Crossref `http_client::fetch_response_with_gate`.
- `crates/research-tools/src/pdf_match_tests.rs` (187 lines, new): unit tests for ligature/soft-hyphen/line-end-hyphen matching, F2 precedence, and mock `MetadataFetcher` rejection/acceptance.

### Edits to existing files
- `crates/research-tools/src/pdf.rs` (211 lines, +30 net): adds `PdfError::{InvalidPageRange, Cancelled}`; routes `extract_pdf_bytes` through `pdf_pages::extract_pdf_pages`; keeps `extract_pdf_bytes_legacy` (`--- [Page N] ---` format) so `verified_page_lines` still verifies pre-F14a receipts both with and without saved `pdf_page_lines`; relaxes per-range `page > MAX_PDF_PAGES` to `ranges.len() > MAX_PDF_PAGES` (still checking `page <= count`) so user-picked page ranges on >400-page PDFs validate cleanly.
- `crates/research-tools/src/ingest.rs` (590 lines, +24 net): derives `Default` on `PreparedIngestion`; routes `prepare_pdf_ingestion` through `pdf_pages::extract_pdf_pages`; adds `ingest_pdf_pages` so background `PdfExtractJob` results can be pinned into `source.ingested` without re-extracting; pins `"pdf_extractor"` and `"pdf_pages"` on PDF receipts (non-PDF receipts unchanged).
- `crates/research-tools/src/lib.rs` (282 lines, +7 net): exports `pdf_firstpage`, `pdf_match`, `pdf_pages` and declares private submodules `pdf_firstpage_norm`, `pdf_pages_bench`, `pdf_pages_job`, `pdf_pages_layout`, `pdf_pages_scan`.

### Self-authored PDF fixtures and integration tests
- `crates/research-tools/tests/pdf_support/mod.rs` (433 lines, new): pure-Rust minimal PDF 1.4 writer generating single-column, two-column (sequential and interleaved), custom `/ToUnicode` CMap ligatures/soft-hyphens, blank/image-only/publisher-OCR (`3 Tr` and `0 Tr`)/tiled scans, `/Info` metadata, and encrypted PDFs strictly from text written in tests (zero third-party PDFs).
- `crates/research-tools/tests/pdf_core.rs` (388 lines, new): legacy pre-F14a receipt backward compatibility (both with and without `pdf_page_lines`), form-feed reading file & pinned extractor/page map, scan detection matrix, background `PdfExtractJob` + `ingest_pdf_pages` + 405-page user range `401..=403`, and malformed/encrypted/oversized refusal without panic.
- `crates/research-tools/tests/pdf_core_bench_and_match.rs` (357 lines, new): extractor comparison benchmark (`PdfExtractPlain` vs `PdfExtractLayout`) and end-to-end first-page + `/Info` + offline/loopback Crossref `PaperKey` matching.

---

## 2. Extractor measurement results (Deliverable 2)

Measured via `cargo test -p research-tools --test pdf_core_bench_and_match -- --nocapture` (debug build, Linux x86_64) across 4 self-authored fixtures (8 pages total, 11 ground-truth sentences, 10 ordered reading-order pairs):

| Fixture | Metric | `pdf-extract/0.12.1` (`PdfExtractPlain`) | `pdf-extract-layout/0.12.1` (`PdfExtractLayout`) |
|---|---|---:|---:|
| `single_col_ligatures_hyphens` (3 pp) | Time | 11.44 ms | 11.59 ms |
| | Sentences matched | 5 / 5 (ligatures 2/2, hyphens 2/2) | 5 / 5 (ligatures 2/2, hyphens 2/2) |
| | Reading-order errors | 0 / 2 | 0 / 2 |
| `two_col_sequential` (1 p) | Time | 5.24 ms | 5.59 ms |
| | Sentences matched | 2 / 2 | 2 / 2 |
| | Reading-order errors | 0 / 4 | 0 / 4 |
| `two_col_interleaved` (1 p) | Time | 5.24 ms | 5.97 ms |
| | Sentences matched | 2 / 2 | 2 / 2 |
| | Reading-order errors | **1 / 4 (columns mixed)** | **0 / 4** |
| `mixed_empty_and_scan` (3 pp) | Time | 6.72 ms | 7.08 ms |
| | Sentences matched | 2 / 2 | 2 / 2 |
| | Empty pages correct | 1 / 1 | 1 / 1 |
| | Scans reported as `text_layer` | 0 | 0 |
| **Total (4 fixtures, 8 pp)** | **Total time** | **28.63 ms** | **30.23 ms (+5.6%)** |
| | **Failures / wrong page links / scan false negatives** | **0 / 0 / 0** | **0 / 0 / 0** |
| | **Reading-order errors** | **1 / 10** | **0 / 10** |

**Selection:** `PdfExtractLayout` (`"pdf-extract-layout/0.12.1"`) was chosen as `DEFAULT_PDF_EXTRACTOR_ID` because it eliminates interleaved two-column reading-order errors (0/10 vs 1/10) with only ~5.6% time overhead on the fixture suite and zero new dependencies.

**Not measured yet:** `pdf_oxide` (and `pdftotext`) are not in `Cargo.lock`, and `RULES.md` prohibits adding dependencies or modifying `Cargo.lock`. Live evaluation set P6 (real journal PDFs and pre-1990 scans) was not run (no third-party PDFs or external fetches in unattended runs). When `pdf_oxide` is approved later, it implements `PdfExtractor` and runs straight through `measure_extractor` on the same fixtures and P6.

---

## 3. Decisions taken alone, deviations, and assumptions

### Decisions
1. **Form-feed separator formatting (`"\n\x0c\n"` between all extracted pages):**
   - Each non-empty page's lines `(first_line..=last_line)` contain zero `\x0c` bytes, while `text.split('\x0c')` preserves exact 1-to-1 correspondence with the extracted physical pages (including empty/scanned pages) and `VerifiedFile::line_starts` (which splits strictly on `\n`) agrees line-for-line with `PageMap::page_of`.
   - Any literal `\x0c` inside a page's extracted stream is converted to `\n` before joining so `\x0c` is unambiguously a page break.
   - All-empty documents still produce `status: EmptyTextLayer`, `text: ""`, `page_lines: []` so existing callers and tests behave identically.
2. **Column-aware `OutputDev` over `pdf-extract` as second measurable extractor:**
   - Rather than measuring only `PlainTextOutput` (which mixes interleaved two-column streams), we implemented `LayoutOutputDev` directly against `pdf-extract`'s public `OutputDev` trait (zero new dependencies) and measured both side-by-side.
3. **Conservative scan-detection thresholds (`pdf_pages_scan.rs`):**
   - `PageBasis::None`: 0 non-whitespace chars; OR `total_image_coverage >= 0.70` with `< 15` alpha chars; OR `3 Tr` invisible text with `< 15` alpha chars; OR `>= 35%` image coverage with `< 100` chars/std-page and `< 15` alpha chars; OR `< 15` chars + `< 20` chars/std-page + `< 8` alpha chars (lone page number / stray glyph on a full-size page).
   - `PageBasis::ScanTextLayer`: `total_image_coverage >= 0.70` with `>= 15` alpha chars; OR `>= 50%` of text ops using `3 Tr` invisible rendering mode; OR `>= 35%` image coverage (or inline image `BI`) with `< 100` chars/std-page and `>= 15` alpha chars.
   - `PageBasis::TextLayer`: all other pages with extractable text.
   - These thresholds are fixed on the self-authored fixtures and should be checked against live P6 papers before M1/M3.
4. **Raised byte cap for user-supplied PDFs (`MAX_USER_PDF_BYTES = 64 MiB`):**
   - `PdfExtractOptions::default()` preserves `MAX_PDF_BYTES = 16 MiB` and `MAX_PDF_PAGES = 400` so existing download validation and tests are unchanged; `PdfExtractOptions::user_supplied()` raises the byte cap to 64 MiB (tunable once live P6 scan sizes are measured) and `with_page_range(start, end)` allows opening slices of >400-page PDFs.
5. **Dual role of `pdf_match.rs`:**
   - `00-overview.md` 2.12 assigns PDF quote normalization to `pdf_match.rs`, while `f14a-pdf-core.md` deliverable 4 assigns offline + fetcher `PaperKey` matching to `pdf_match.rs`. Both are implemented in `pdf_match.rs` (589 lines).
6. **Pre-F14a legacy receipt verification:**
   - Kept `extract_pdf_bytes_legacy` (`--- [Page N] ---` format) inside `pdf.rs` so `verified_page_lines` still reproduces and verifies pre-F14a receipts even when `pdf_page_lines` is absent.

### Deviations from the design
- Split `pdf_pages.rs` into `pdf_pages.rs`, `pdf_pages_layout.rs`, `pdf_pages_scan.rs`, `pdf_pages_job.rs`, `pdf_pages_bench.rs`, and `pdf_pages_tests.rs`, and split `pdf_firstpage.rs` into `pdf_firstpage.rs`, `pdf_firstpage_norm.rs`, and `pdf_firstpage_tests.rs` so every file stays at or under 600 lines. All public items are re-exported from `pdf_pages` and `pdf_firstpage`.
- Did not implement `paper_text` caching in `library.db` (belongs to F10; overview 2.7 notes F10 adds tables beyond F3's `fulltext`) or XMP XML metadata parsing from `pdf_oxide` (since `pdf_oxide` is not a dependency; `/Info` dictionary extraction is implemented via `pdf_extract::Document`).
- Did not add `pdf_oxide` or `pdftotext` subprocess fallback (`Cargo.lock` must stay untouched and no Unix/external binary shell-out is used).

### Assumptions
- `pdf-extract` 0.12.1 re-exports `lopdf` (`Document`, `Object`, `Dictionary`, `content::Content`) and passes glyph transforms in PDF user-space points to `OutputDev::output_character`. Verified against the installed crate source in Cargo cache.
- Publisher scanned PDFs with embedded OCR text either draw an Image XObject (single or tiled strips) covering `>= 70%` of the page area or render text with PDF text rendering mode `3 Tr` (ISO 32000-1 Table 106). Real-world P6 papers may include rotated `/Rotate` pages or masked/JBIG2 images inside nested Form XObjects beyond depth 2; live P6 measurement should verify scan recall.
- Crossref bibliographic search endpoint accepts `GET /works?rows=3&select=DOI,title,author,published&query.bibliographic=<encoded>` returning `message.items[].{DOI, title, author, published}` (consistent with Kandinsky `firstpage.rs:195` and existing `literature.rs:249`).

---

## 4. Merge notes

### Edits to existing files
- `crates/research-tools/src/pdf.rs`: added `PdfError::{InvalidPageRange, Cancelled}`, routed `extract_pdf_bytes` through `pdf_pages::extract_pdf_pages`, updated `verified_page_lines` with `pdf_extractor` dispatch + pre-F14a legacy fallback (`extract_pdf_bytes_legacy`).
- `crates/research-tools/src/ingest.rs`: derived `Default` on `PreparedIngestion`, routed `prepare_pdf_ingestion` through `pdf_pages::extract_pdf_pages`, added `pub fn ingest_pdf_pages`, pinned `"pdf_extractor"` and `"pdf_pages"` on PDF `source.ingested` receipts.
- `crates/research-tools/src/lib.rs`: exported `pdf_firstpage`, `pdf_match`, `pdf_pages` and private submodules.

### Integration points for other briefs / later packages
- **V17 (`skeleton/pdf.rs`):** consume `source.ingested` reading file (`\x0c`-separated), `pdf_pages` (`PageRecord` with `basis: PageBasis`), and `pdf_page_lines` via `pdf_pages::page_of_line` / `PageMap::page_of`; use `pdf_match::match_pdf_quote` for PDF quote matching and `start_page_basis` to tag `scan_text_layer` quotes as OCR.
- **F13a (`source_folder.rs`, `source_sync.rs`):** call `pdf_pages::extract_pdf_pages` (or `PdfExtractJob`), `pdf_firstpage::{extract_first_page_candidates, extract_pdf_info_candidates, merge_first_page_candidates}`, and `pdf_match::match_candidates_offline` for local PDFs without identifiers; call `pdf_match::lookup_candidates_with_fetcher` with `PaperFetchMetadataAdapter` only after user approval (overview 2.4, 2.11).
- **F10 (`library.rs`, `library_schema.rs`):** store `PdfDocumentPages` (`extractor_id`, `text`, `page_map`) in the `paper_text` table keyed by `(paper_key, version, file_sha256, extractor_id)`.
- **F4 (`quote_core.rs`):** for PDF sources, delegate normalized comparison to `pdf_match::quotes_match_pdf` / `match_pdf_quote` without modifying stored reading-file text.
- **F14b (`pdf_ocr.rs`):** use `PageMap::pages_needing_ocr()` to identify `PageBasis::None` pages requiring Tesseract OCR.
- **V4 / desktop routes:** spawn `pdf_pages::PdfExtractJob::start` on a background thread, poll `snapshot()`, and pin completed `PdfDocumentPages` into the session via `ingest::ingest_pdf_pages`.
- **Browser Playwright test `crates/desktop-server/tests/pdf_preview_browser.py:52-58` (not touched; outside brief scope):** see Section 6 below — changing newly ingested PDF reading files from `--- [Page N] ---` headers to `\x0c` separators affects that Playwright script's hardcoded `line=2` assertion and Python `.splitlines()` line indexing.

---

## 5. Verification, test counts, and pre-existing findings

### Test and check commands run

| Command | Result |
|---|---|
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | **Pass** — `No touched source file is over 600 lines.` |
| `git diff --stat origin/gemini/work/fixed -- Cargo.lock crates/research-tools/tests/golden/tool_defs.txt crates/policy crates/ledger` | **Pass** — empty diff |
| `cargo clippy -p research-tools --lib --test pdf_core --test pdf_core_bench_and_match --test ingest_archive_pdf --test tool_def_golden -- -D warnings` | **Pass** — 0 warnings |
| `cargo test -p research-tools --test tool_def_golden` | **Pass** — 3 passed, 0 failed, 0 ignored |
| `cargo test -p research-tools --lib` | **Pass** — 74 passed, 0 failed, 0 ignored (includes 11 new unit tests across `pdf_pages`, `pdf_firstpage`, `pdf_match`) |
| `cargo test -p research-tools --test pdf_core` | **Pass** — 5 passed, 0 failed, 0 ignored |
| `cargo test -p research-tools --test pdf_core_bench_and_match -- --nocapture` | **Pass** — 2 passed, 0 failed, 0 ignored |
| `cargo test -p research-tools --test ingest_archive_pdf` | **Pass** — 6 passed, 0 failed, 0 ignored |
| `cargo test --workspace --no-fail-fast` | **Pass** — **681 passed, 0 failed, 2 ignored** across 99 test/doc-test targets (the 2 ignored tests are pre-existing live-evaluation tests in `crates/agent/tests/eval_harness.rs`) |
| `RE_SEARCH_PYTHON="$(command -v python3)" cargo test -p export --no-fail-fast` | **Pass** — **8 passed, 0 failed, 0 ignored** |
| `cargo clippy --workspace --all-targets -- --no-deps` | **6 pre-existing warnings only** (0 warnings in any file added or modified by this branch; see proof below) |

### Pre-existing clippy warnings (proven on base commit `e1517477cbceba0a6de6e4a143c34fe3b8b8411d`)

Running `cargo clippy --workspace --all-targets -- --no-deps` in a detached worktree of `origin/gemini/work/fixed` (`e1517477cbceba0a6de6e4a143c34fe3b8b8411d`) under `rustc 1.100.0` produced the exact same 6 warnings (and zero others on this branch):

1. `crates/desktop-server/src/evidence.rs:744` — `clippy::manual_unwrap_or` (also documented in `RULES.md`).
2. `crates/agent/tests/eval_support/manifest.rs:289` — `clippy::match_like_matches_macro`.
3. `crates/agent/tests/eval_support/sheet.rs:102` — `clippy::unnecessary_map_or`.
4. `crates/research-tools/tests/sandbox_and_fs.rs:6,9` — `unused_imports` (`is_symlink_or_junction`, `TEX_AUDIT`, Linux build).
5. `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` — `unused_imports` (`BufRead`, `BufReader`, Linux build).
6. `crates/agent/src/guardrails.rs:335` — `clippy::items_after_test_module`.

### What could not be compiled or run on Linux
- `cfg(windows)` code paths and tests in `crates/desktop-app`, `crates/worker` (`appcontainer_sandbox.rs`, `python_worker.rs`), `crates/cli/tests/contained_router.rs`, `crates/research-tools/src/sandbox.rs` (NTFS junctions), `crates/research-tools/tests/sandbox_and_fs.rs`, `crates/chatgpt-auth`, and Windows Playwright browser tests (`crates/desktop-server/tests/pdf_preview_browser.py`).
- Live external network lookups and real-world P6 PDFs (prohibited in unattended tests; all HTTP tests ran against a loopback mock server with `FakeRateClock`).

---

## 6. Bugs / oddities noticed but not fixed & design notes

- **`crates/desktop-server/tests/pdf_preview_browser.py:52-58`:** This Windows Playwright test (outside F14a scope, not run by `cargo test`) assumes two pre-F14a details for newly uploaded PDFs: (1) line 57 requests `line=2` expecting page 1 on a 1-line page because the old format prepended a `--- [Page 1] ---` header line (making page 1 span lines 1–2 instead of line 1); (2) line 52 uses Python `str.splitlines()`, which splits on `\x0c` in addition to `\n`, shifting line numbers after a `\x0c` page break relative to Rust's `\n`-only line numbering (`VerifiedFile::line_starts` and `str::lines()`). The integrator should update line 52 to `.split("\n")` and line 57 to `line=1` when merging F14a.
- **`crates/desktop-server/src/original_pdf.rs:32`:** Enforces `bytes.len() > MAX_PDF_BYTES` (16 MiB) when serving original PDFs. Once user-supplied local scans up to `MAX_USER_PDF_BYTES` (64 MiB) are ingested via F13a/routes, `original_pdf.rs` will need to allow `MAX_USER_PDF_BYTES`.
- **Minor design overlap:** `00-overview.md` 2.12 describes `pdf_match.rs` as PDF quote normalization, while `agents/f14a-pdf-core.md` deliverable 4 describes `pdf_match.rs` as matching first-page metadata candidates to F2 `PaperKey` offline and with a fetcher. Both fit cleanly in `pdf_match.rs` (589 lines), with no contradiction in behavior.
- **What does not work yet (by scope):** OCR for `PageBasis::None` pages (F14b), page images to a model (F14c), PDF skeleton extraction (V17), `paper_text` caching in `library.db` (F10), `pdf_oxide` / `pdftotext` comparison (awaiting dependency approval), and live P6 real-paper evaluation (awaiting owner's P6 set).
