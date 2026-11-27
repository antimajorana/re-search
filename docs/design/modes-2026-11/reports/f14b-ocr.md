# Report: `f14b-ocr` (package F14b — Free Default Tesseract OCR Text Layer, Local Detection & AppContainer Helper)

- **Base commit:** `6491394e28ef390f5f40e5fe56ac3aec8c1a97d7` (`origin/gemini/work/fixed`)
- **Work branch:** `work/f14b-ocr` (pushed to `gemini/modes/f14b-ocr`)
- **Toolchain on this machine:** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `cargo 1.100.0 (3acc79c0c 2026-11-05)`, Node `v22.23.5`, `tesseract 5.3.4` (`tesseract-ocr-eng` `1:4.1.0-2`), running as non-root `uid=1001(antigravity)` on Linux x86_64.
- **Unchanged guard rails:**
  - `Cargo.lock` and all `Cargo.toml` files: **untouched** (0 new crate dependencies).
  - `crates/research-tools/tests/golden/tool_defs.txt` & `tool_profiles.txt`: **untouched** (0 tool definitions added or changed).
  - `crates/policy`, `crates/ledger`, `crates/agent`, `.github/`, and all Do-Not-Edit files (`ingest.rs`, `pdf_match.rs`, `lines_sent.rs`, `lit_*`, `http_client.rs`, `net_rate.rs`, `solve_*`, `skeleton/html*`, `routes_tools.rs`, `http_srv.rs`, `routes_session.rs`, `state.rs`): **untouched**.
  - `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`: **Pass** (`No touched source file is over 600 lines.`).
  - Zero skipped or `#[ignore]` tests added; zero network downloads; zero hardcoded or invented `.traineddata` SHA-256 digests.

---

## 1. What was implemented (package F14b)

### Checkpoint 1 — `PageBasis::Ocr`, Renderer Interface, Local Tesseract Detector & Contained `OcrEngine` Helper

- **Core `PageBasis::Ocr` & honesty updates** (commit `5f54c0e`, audit fix `67cda01`):
  - `crates/research-tools/src/pdf_pages.rs` (572 lines): added `PageBasis::Ocr` (`as_str() == "ocr"`, `is_scan() == true`, `needs_ocr() == false`).
  - `crates/research-tools/src/pdf_pages_tests.rs` (131 lines): unit test verifying `PageBasis::Ocr` invariants and JSON serde `"ocr"`.
  - `crates/research-tools/src/skeleton/pdf_honesty.rs` (527 lines): updated `node_page_basis`, `match_pdf_quote_for_display`, and `classify_line_range` to treat `PageBasis::Ocr` as `PdfTextOrigin::ScanOcrText` (`is_ocr: true`, `render_math: false`, `"matched in OCR text, p. N"`, worst-basis order `None > Ocr > ScanTextLayer > TextLayer`).
  - `crates/research-tools/src/skeleton/pdf.rs` (348 lines): updated `summarize_pages` so `PageBasis::Ocr` pages are included in `scan_text_pages` and carry `PDF_QUALITY_SCAN_TAG` (`"(scan)"`) in `quality_reason`.
  - `crates/research-tools/src/pdf_pages_bench.rs` (253 lines): generalized `scans_reported_as_text_layer` check to `expected_basis.is_scan()`.
  - `crates/desktop-server/src/investigation_mapview_v7.rs` (588 lines): emits `"ocr": true` and `"page_basis": "ocr"` (preserving conservative `Ocr > ScanTextLayer` precedence across multi-page spans).

- **Pure-Rust PDF page renderer & PNG codec** (commit `fd85bd3`, lint fix `609e61b`):
  - `crates/research-tools/src/pdf_render_png.rs` (376 lines, new): pure-Rust PNG encoder, bounded zlib decoder (`flate2`, defeating zip bombs before pixel allocation), all 5 PNG scanline filters, PNG predictor reversal (`/Predictor 10..=15`), pixel converters (1-bit packed monochrome with `/Decode`, 8-bit Gray/RGB/RGBA, 8-bit CMYK, 1/4/8-bit `/Indexed /DeviceRGB`), and normalized PNG cropper.
  - `crates/research-tools/src/pdf_render.rs` (531 lines, new): `PdfPageRenderer` trait (`render_page`, `render_crop`), `RenderedPage`, `CropRect` (normalized `[0, 1]`), `RenderLimits` (`MAX_RENDER_WIDTH/HEIGHT = 8192`, `MAX_RENDER_PIXELS = 36M`, `MAX_RENDER_PNG_BYTES = 32 MiB`, `MAX_RENDER_PDF_BYTES = 64 MiB`, `DEFAULT_RENDER_TIMEOUT_MS = 15_000`), `RasterXObjectRenderer`, opt-in `ExternalCommandRenderer::pdftoppm`, `FallbackPageRenderer`, and injectable `MapPageRenderer`.
  - `crates/research-tools/src/pdf_render_tests.rs` (370 lines, new): 6 unit tests covering PNG round-trips, all 5 filters, crop clamping/validation, 1-bit/8-bit Gray/RGB/CMYK/Indexed/predicted XObjects, dimension-bomb rejection, fallback delegation, and bounds enforcement.

- **Local Tesseract & `tessdata` detector + runtime SHA-256 provenance** (commit `e455fd4`, lint fix `609e61b`):
  - `crates/research-tools/src/pdf_ocr_models.rs` (482 lines, new): `DEFAULT_OCR_ENABLED = true` (Owner Decision 2026-11-26), `OCR_INSTALL_PROMPT = "Install Tesseract to enable OCR"` (Coordinator Clarification 2026-11-26), `OcrLocalConfig`, `OcrEnvProbe`, `OcrStatus`, `OcrModelProvenance`, discovery precedence (`config` -> `RE_SEARCH_TESSERACT`/`TESSDATA_PREFIX` -> `PATH` -> sibling `tessdata` -> standard Windows & Unix locations), bounded `probe_tesseract_version`, `format_ocr_engine_id`, and streaming 64 KiB runtime disk SHA-256 hashing (`hash_local_traineddata`). Zero network downloads, zero hardcoded hashes.
  - `crates/research-tools/src/pdf_ocr_models_tests.rs` (297 lines, new): 9 unit tests verifying precedence, missing binary/missing `eng.traineddata` install prompt, runtime SHA-256 computation, and lang-tag sanitization.

- **Contained Tesseract helper & `OcrEngine` trait in `worker`** (commit `5e53cdd`, audit fix `67cda01`):
  - `crates/worker/src/ocr_helper.rs` (516 lines, new): `OcrEngine` trait, `OcrPageInput`, `OcrPageResult`, `OcrHelperError`, `TesseractCliConfig`, `TesseractCliEngine`, and deterministic `FakeOcrEngine`. On `#[cfg(windows)]`, creates an ephemeral per-page `AppContainerProfile` (`network_restricted: true`, `filesystem_restricted: true`, `KILL_ON_JOB_CLOSE`), stages ONLY `page.png` and requested `.traineddata` inside `<data_dir>/ocr-stage/`, applies `grant_icacls_inside` (`RX` on stage dir, `M` only on `out`/`tmp`), launches via `win::spawn_extended`, enforces per-page timeout (`kill_tree()` on expiry) and bounded output reads (`2 MiB`), and cleans up via RAII. On `#[cfg(not(windows))]`, runs isolated ephemeral-dir subprocess with minimal env, timeout kill/reap, and bounded reads, reporting containment honestly (`backend: "isolated-tempdir"`, `filesystem_restricted: false`, `network_restricted: false`).
  - `crates/worker/src/ocr_helper_tests.rs` (289 lines, new): 7 unit tests covering input/PNG/lang validation, `FakeOcrEngine`, missing binary/tessdata/traineddata `Unavailable`, bounded text reads, and `wait_with_timeout` exit/timeout/error kill paths.
  - `crates/worker/src/lib.rs` (487 lines): exports `pub mod ocr_helper;`.

### Checkpoint 2 — OCR Pipeline, 64 MiB Scan Ingest, Authenticated Routes, UI & Real Set P6 Measurement

- **OCR pipeline, background job & 64 MiB scan ingest** (commit `ec131f7`):
  - `crates/research-tools/src/pdf_ocr.rs` (508 lines, new): `OcrPageRunner` trait, `FakeOcrPageRunner`, `LocalTesseractPageRunner`, `PdfOcrOptions` (defaulting to `PdfExtractOptions::user_supplied()` = 64 MiB, `DEFAULT_OCR_ENABLED = true`), `extract_and_ocr_pdf_pages` / `_with_progress` (selectively OCRs ONLY `PageBasis::None` pages, keeps `TextLayer` & `ScanTextLayer` byte-identical, splices `\n\x0c\n`-separated reading file matching `pdf_pages.rs` line math, surfaces `"Install Tesseract to enable OCR"` when unavailable while preserving `PageBasis::None`), and `cache_pdf_ocr_in_library` (stores extracted+OCR text and `PageBasis::Ocr` in F10 `paper_text`).
  - `crates/research-tools/src/pdf_ocr_job.rs` (200 lines, new): thread-capable `PdfOcrJob` with `Queued`/`ExtractingText`/`OcrPage`/`Done`/`Cancelled`/`Failed` stages, `snapshot()`, and cooperative `cancel()`.
  - `crates/research-tools/src/ingest_ocr.rs` (320 lines, new): `ingest_pdf_with_ocr`, `ingest_pdf_path_with_ocr`, and `pin_ocr_outcome_to_session` (64 MiB cap, atomic materialization under `sessions/<id>/sources/`, immutable fresh `source.ingested` receipts pinning `pdf_extractor`, `pdf_pages`, `ocr_engine`, and runtime `ocr_models[{lang, sha256, bytes}]`, plus F10 `Library` `paper_text` caching).
  - `crates/research-tools/src/pdf_ocr_tests.rs` (341 lines, new): 5 unit tests covering selective OCR, missing runner prompt, disabled config, background job + cancellation, receipt pinning, F10 library caching, `quote_core::verify_quote`, and `skeleton::match_pdf_quote_for_display`.
  - `crates/research-tools/src/lib.rs` (358 lines): exports `ingest_ocr`, `pdf_ocr`, `pdf_ocr_job`, `pdf_ocr_models`, `pdf_render` (and private `pdf_render_png`).

- **Authenticated desktop routes & 64 MiB PDF serving** (commit `25bc60d`, audit fix `c487fc7`):
  - `crates/desktop-server/src/original_pdf.rs` (44 lines): raised cap from `MAX_PDF_BYTES` (16 MiB) to `MAX_USER_PDF_BYTES` (64 MiB).
  - `crates/desktop-server/src/ocr_settings.rs` (234 lines, new): atomic `<data_dir>/ocr_settings.json` sidecar (zero `state.rs` growth), strict explicit `tesseract_path`/`tessdata_dir` override status evaluation, `GET`/`PUT /api/settings/ocr`.
  - `crates/desktop-server/src/routes_ocr.rs` (509 lines, new): `WorkerOcrRunner` bridging `worker::ocr_helper::TesseractCliEngine` (AppContainer on Windows) into `research_tools::pdf_ocr::OcrPageRunner`; process-wide capped background job registry scoped by `(data_dir, session_id)`; `POST /api/sessions/{id}/ocr/ingest` (sync & async `202`, JSON base64 / multipart / raw `application/pdf`, serialized ledger writes under `state.ledger_write_lock`); `GET /api/sessions/{id}/ocr/jobs/{job_id}`; `POST /api/sessions/{id}/ocr/jobs/{job_id}/cancel` (releases buffered `raw_bytes`); `GET /api/sessions/{id}/sources/{source}/page-image?page=N&crop=x,y,w,h&dpi=D` serving `image/png` with `X-Content-Type-Options: nosniff`, `Content-Security-Policy: sandbox`, and `Cache-Control: no-store`.
  - `crates/desktop-server/src/lib.rs` (255 lines) & `crates/desktop-server/src/api_dispatch.rs` (506 lines): module & route wiring.
  - `crates/desktop-server/tests/ocr_routes.rs` (366 lines, new): 4 integration tests covering auth (`401` unauthenticated & cookie-only, `200` with token/Bearer), settings CRUD/validation/immutability, sync & async ingest/job isolation/cancel, and full/cropped `page-image` PNG serving & security headers.

- **UI assets & Node tests** (commit `1708e62`, audit fix `9d88d57`):
  - `crates/desktop-server/assets/ocr_settings.js` (244 lines, new): zero-`innerHTML` settings panel & status banner (`DEFAULT_OCR_ENABLED = true`, `"Install Tesseract to enable OCR"` when unavailable, cleanly separates configured overrides from auto-detected local paths/languages).
  - `crates/desktop-server/assets/page_image_viewer.js` (282 lines, new): zero-`innerHTML` scanned page image viewer fetching `/api/sessions/{id}/sources/{source}/page-image` via authenticated `X-ReSearch-Token` + `Blob` + race-safe `URL.revokeObjectURL`, aligned normalized `[0, 1]` crop validation, plain-text OCR quote display (never KaTeX math), and `bindQuoteChipPageImageViewer` adapter for `quote_chip.js`.
  - `crates/desktop-server/assets/investigation_node_model.js` (452 lines): recognizes `page_basis === 'ocr'` and `anchor.ocr` in Block B quote chips so OCR quotes show `"matched in OCR text, p. N"` and open the page image.
  - `crates/desktop-server/src/assets.rs` (502 lines): registers `/assets/ocr_settings.js` and `/assets/page_image_viewer.js`.
  - `crates/desktop-server/tests/ocr_ui.test.mjs` (359 lines, new): 4 Node tests covering settings panel, nested `GET /api/settings/ocr` contract, page image viewer auth/Blob lifecycle/crop validation, `quote_chip.js` integration, and zero-`innerHTML` XSS safety.

- **Self-authored Set P6 scanned PDF fixtures & pipeline test** (commit `f35017a`):
  - `crates/research-tools/tests/fixtures/p6_ocr/` (`manifest.json` + 6 self-authored synthetic PDFs, total 100.3 KiB, zero third-party text).
  - `crates/research-tools/tests/pdf_ocr_pipeline.rs` (534 lines, new): end-to-end honesty/library/Solve-flag test + real-Tesseract Set P6 measurement harness.

---

## 2. Mandatory Real Set P6 Measurement (`00-overview.md` §7.2 Set P6)

Measured using real **`tesseract 5.3.4`** (`eng.traineddata`, observed runtime SHA-256 `7d4322bd2a774972...`, 4,113,088 bytes) via:

```bash
unset RE_SEARCH_PYTHON && cargo test -p research-tools --locked --test pdf_ocr_pipeline -- --nocapture
```

| Fixture | Kind | Pages | OCR'd (`None` -> `Ocr`) | Skipped (`TextLayer` / `ScanTextLayer`) | Sentences Matched | Avg CER / 100 chars | `wrong_page_chip` | `scanned_reported_text_layer` |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| `p6_01` | 1-bit monochrome fax scan (FlateDecode 1 bpc) | 2 | 2 | 0 | 6 / 6 (100.0%) | 0.00 | 0 | 0 |
| `p6_02` | 8-bit grayscale scan (off-white bg=246, ink=20) | 2 | 2 | 0 | 4 / 4 (100.0%) | 0.00 | 0 | 0 |
| `p6_03` | Pre-1990 `3 Tr` publisher scan layer + image-only appendix | 3 | 1 | 2 | 6 / 6 (100.0%) | 0.00 | 0 | 0 |
| `p6_04` | Formula-heavy 1-bit scan (`render_math == false`) | 1 | 1 | 0 | 3 / 3 (100.0%) | 0.00 | 0 | 0 |
| `p6_05` | Mixed digital `TextLayer` + 1-bit scan insert | 3 | 1 | 2 | 6 / 6 (100.0%) | 0.00 | 0 | 0 |
| `p6_06` | Two-column 1-bit scan | 1 | 1 | 0 | 4 / 4 (100.0%) | 0.00 | 0 | 0 |
| `p5_10` | Repo fixture (`ScanTextLayer` p1/p3 + 1x1 blank scan p2) | 3 | 0* | 2 | 2 / 2 (100.0%) | 0.00 | 0 | 0 |
| **Total (7 fixtures)** | | **15** | **8** | **6** | **31 / 31 (100.0%)** | **0.00** | **0** | **0** |

*\*Note on `p5_10` page 2:* `tests/fixtures/eval/p5/p5_10_pdf_scanned_ocr_and_image_only.pdf` page 2 contains a synthetic 1x1-pixel blank scan image with no glyphs; real Tesseract returned empty text and the pipeline kept page 2 honestly at `PageBasis::None` (`pages_remaining_none == [2]`), never promoting it to `TextLayer` or fabricating text.

**Honesty note on fixture scope:** These 6 Set P6 fixtures are crisp, self-authored 200 DPI synthetic scans (`DejaVuSerif.ttf`) because third-party papers cannot be committed to the repository (`RULES.md`). Real historical pre-1990 scans (skewed, noisy, degraded halftone, JBIG2/CCITT/JPEG-compressed streams) will have higher CER and may require an external rasterizer (`ExternalCommandRenderer::pdftoppm`) when embedded images use `/JBIG2Decode`, `/CCITTFaxDecode`, or `/DCTDecode`. Live Set P6 measurement on the owner's real historical papers remains to be run once those papers are supplied locally.

---

## 3. Decisions Taken Alone, Deviations, and Assumptions

### Decisions
1. **Trait boundary between `research-tools` and `worker`:** Because `crates/research-tools/Cargo.toml` does not depend on `worker` (and `Cargo.toml`/`Cargo.lock` cannot be edited), `pdf_ocr.rs` defines `OcrPageRunner`, `worker::ocr_helper` defines `OcrEngine` (with Windows `AppContainerSandbox`), and `desktop-server::routes_ocr::WorkerOcrRunner` bridges `worker::ocr_helper::TesseractCliEngine` into `research_tools::pdf_ocr::OcrPageRunner`. `LocalTesseractPageRunner` inside `research-tools` provides a direct cross-platform subprocess runner for CLI/library/test callers outside `desktop-server`.
2. **Sidecar settings file (`<data_dir>/ocr_settings.json`) and process-wide job registry:** Because `state.rs` (599 lines) is on the Do-Not-Edit list, OCR settings use an atomic JSON sidecar mirroring `investigation_settings.rs`, and background OCR jobs live in a capped (64-entry) `OnceLock<Mutex<HashMap<...>>>` registry scoped by `(data_dir, session_id)`.
3. **Pure-Rust raster XObject renderer + opt-in external fallback:** To obey zero `Cargo.lock` edits, `RasterXObjectRenderer` decodes uncompressed and `/FlateDecode` (1/8-bit Gray, 8-bit RGB/CMYK, 1/4/8-bit Indexed, PNG predictors 10..=15) PDF Image XObjects directly to PNG using workspace `flate2`. Unsupported image codecs (`/DCTDecode`, `/CCITTFaxDecode`, `/JBIG2Decode`, `/JPXDecode`) and vector-only pages return descriptive errors and can delegate via `FallbackPageRenderer` + `ExternalCommandRenderer::pdftoppm`.
4. **No Unix-only shell-outs in tests:** All Rust tests are cross-platform (no `#!/bin/sh` test scripts); real Tesseract execution in `pdf_ocr_pipeline.rs` runs automatically whenever `ocr_status` detects local `tesseract` + `eng.traineddata` and falls back to testing `OCR_INSTALL_PROMPT` + `FakeOcrPageRunner` when absent (zero `#[ignore]` or skipped tests).
5. **Independent audit remediation:** Ran a full adversarial audit before final verification and fixed all 4 blockers (`skeleton/pdf.rs` `(scan)` tag for `PageBasis::Ocr`, `investigation_node_model.js` `'ocr'` basis check, `ocr_settings.js` nested config-vs-status separation, Windows missing-traineddata `"Install Tesseract to enable OCR"` prompt) and all 6 fixable medium items (`ledger_write_lock` serialization in `routes_ocr.rs`, `final_snapshot` persistence before join, strict explicit `tessdata_dir` override in `ocr_settings.rs`, conservative `Ocr > ScanTextLayer` multi-page precedence in `investigation_mapview_v7.rs`, `raw_bytes` release on cancel + `data_dir` job scoping, and normalized `[0, 1]` crop contract alignment in `page_image_viewer.js`).

### Deviations from `00-overview.md` 2.12
- **Per Coordinator Clarification (2026-11-26):** Local system `tesseract` + `tessdata` detection is used instead of in-app downloading of language models over HTTPS; no `tessdata` host was added to `ALLOWLISTED_HTTPS_HOSTS`, and `.traineddata` SHA-256 digests are computed from local files at runtime instead of pinned as hardcoded constants.
- Split `pdf_render.rs` into `pdf_render.rs` + `pdf_render_png.rs`, and split desktop routes into `ocr_settings.rs` + `routes_ocr.rs`, so every file remains well under 600 lines.

### Assumptions
- On Windows, standard `C:\Program Files\Tesseract-OCR\tesseract.exe` inherits `ALL APPLICATION PACKAGES (S-1-15-2-1) RX` permissions from `Program Files` and can launch inside a zero-capability `AppContainerProfile`; user-profile installs (e.g. `%LOCALAPPDATA%\Programs\Tesseract-OCR`) or custom paths without `S-1-15-2-1` RX may require staging or ACL adjustment on Windows (untested on Linux).
- `pdf_render::RasterXObjectRenderer` handles uncompressed and `/FlateDecode` raster scans; real-world PDFs encoded with `/CCITTFaxDecode`, `/JBIG2Decode`, `/DCTDecode`, or vector-only pages need either an opt-in `pdftoppm` external helper or a future approved Rust PDF rasterizer dependency.

---

## 4. Merge Notes

### Small edits to existing files
- `crates/research-tools/src/pdf_pages.rs`: added `PageBasis::Ocr` variant (+3 lines).
- `crates/research-tools/src/pdf_pages_tests.rs`: added `page_basis_ocr_is_scan_and_does_not_need_ocr` (+11 lines).
- `crates/research-tools/src/pdf_pages_bench.rs`: changed `matches!(..., None | ScanTextLayer)` to `expected_basis.is_scan()`.
- `crates/research-tools/src/skeleton/pdf.rs`: `summarize_pages` now matches `PageBasis::ScanTextLayer | PageBasis::Ocr`.
- `crates/research-tools/src/skeleton/pdf_honesty.rs`: `node_page_basis`, `match_pdf_quote_for_display`, and `classify_line_range` handle `PageBasis::Ocr` as `ScanOcrText` (`is_ocr: true`, `render_math: false`).
- `crates/research-tools/src/lib.rs`: added module declarations (`ingest_ocr`, `pdf_ocr`, `pdf_ocr_job`, `pdf_ocr_models`, `pdf_render`, `pdf_render_png`).
- `crates/worker/src/lib.rs`: added `pub mod ocr_helper;`.
- `crates/desktop-server/src/original_pdf.rs`: raised `MAX_PDF_BYTES` -> `MAX_USER_PDF_BYTES` (64 MiB).
- `crates/desktop-server/src/investigation_mapview_v7.rs`: treats `PageBasis::Ocr` as OCR (`"ocr": true`, `"page_basis": "ocr"`, conservative `Ocr > ScanTextLayer` fold).
- `crates/desktop-server/src/lib.rs`: added `pub mod ocr_settings;` and `pub mod routes_ocr;`.
- `crates/desktop-server/src/api_dispatch.rs`: wired 6 OCR routes (`GET/PUT /api/settings/ocr`, `POST /api/sessions/{id}/ocr/ingest`, `GET /api/sessions/{id}/ocr/jobs/{job_id}`, `POST /api/sessions/{id}/ocr/jobs/{job_id}/cancel`, `GET /api/sessions/{id}/sources/{source}/page-image`).
- `crates/desktop-server/src/assets.rs`: registered `/assets/ocr_settings.js` and `/assets/page_image_viewer.js`.
- `crates/desktop-server/assets/investigation_node_model.js`: recognizes `page_basis === 'ocr'` and `anchor.ocr` in Block B quote chips.

### Integration points for other briefs / integrator
- **`crates/desktop-server/src/http_srv.rs:18,227-235` (Do-Not-Edit on this branch):** `http_srv.rs` currently caps non-`/upload` HTTP request bodies at `MAX_BODY_BYTES = 60 MiB`. While `ingest_ocr.rs`, `routes_ocr.rs`, and `original_pdf.rs` support the full `MAX_USER_PDF_BYTES = 64 MiB`, over live HTTP a 61–64 MiB raw PDF (or >~45 MiB base64 JSON PDF) to `POST /api/sessions/{id}/ocr/ingest` will hit HTTP `413` in `http_srv.rs` until `["api", "sessions", _, "ocr", "ingest"]` is added to the raised upload body cap (e.g., `64 MiB` raw / `~86 MiB` base64).
- **UI shell mounting (`index.html` / `app.js` / Settings dialog):** `/assets/ocr_settings.js` (`createOcrSettingsPanel`, `loadOcrSettings`, `saveOcrSettings`) and `/assets/page_image_viewer.js` (`createPageImageViewer`, `bindQuoteChipPageImageViewer`) are served and tested against `quote_chip.js`, ready to be mounted into the Settings dialog and Investigate/Explore/Solve quote-chip click handlers without touching `index.html` on this branch.
- **`f14c-vision-transcribe`:** Disjoint (`lines_sent.rs` untouched). Can reuse `research_tools::pdf_render::PdfPageRenderer` (`render_page` / `render_crop`) and `GET /api/sessions/{id}/sources/{source}/page-image` for vision formula transcription crops.
- **Optional external rasterizer wiring:** `routes_ocr.rs` currently instantiates `RasterXObjectRenderer::default()`; wiring `FallbackPageRenderer::new(Box::new(RasterXObjectRenderer::default()), optional_pdftoppm)` will add `/DCTDecode`, `/CCITTFaxDecode`, `/JBIG2Decode`, and vector-only page rendering whenever `pdftoppm` is installed.

---

## 5. Verification Commands & Exact Results

| Command | Result |
|---|---|
| `id` | `uid=1001(antigravity)` (non-root) |
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | **Pass** — `No touched source file is over 600 lines.` |
| `git diff --stat origin/gemini/work/fixed -- Cargo.lock Cargo.toml crates/research-tools/tests/golden/tool_defs.txt crates/policy crates/ledger crates/agent` | **Pass** — empty diff |
| `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` | **Pass** — exit 0 across all JS assets |
| `node --test crates/desktop-server/tests/*.test.mjs` | **Pass** — **235 passed, 0 failed, 0 skipped** (9.4 s) |
| `unset RE_SEARCH_PYTHON && cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` | **Pass** — 0 warnings, 0 errors |
| `unset RE_SEARCH_PYTHON && cargo test --workspace --locked --no-fail-fast` | **Pass** — **1246 passed, 0 failed, 2 ignored** across 158 test/doc-test targets (the 2 ignored are pre-existing live-evaluation tests in `crates/agent/tests/eval_harness.rs`; zero new ignored tests) |
| `unset RE_SEARCH_PYTHON && cargo test -p research-tools --locked --test pdf_ocr_pipeline -- --nocapture` | **Pass** — **2 passed, 0 failed, 0 ignored** (real `tesseract 5.3.4` Set P6 measurement: 31/31 sentences matched, 0.00 CER/100, `wrong_page_chip == 0`, `scanned_reported_text_layer == 0`) |

### Pre-existing failures
- None observed on Linux (non-root with `RE_SEARCH_PYTHON` unset). Note: running `cargo clippy` without `-A clippy::manual_unwrap_or` still triggers the known pre-existing warning in `crates/desktop-server/src/evidence.rs:744` documented in `RULES.md`.

### What could not be compiled or run on Linux
- `#[cfg(windows)]` code paths and Windows-only tests, specifically:
  - `crates/worker/src/ocr_helper.rs` Windows `AppContainerProfile` + `grant_icacls_inside` + `win::spawn_extended` execution path (audited line-by-line against `appcontainer.rs`, `stage.rs`, `win.rs`, and `win/launch.rs` for signature/import accuracy, but NOT compiled with `x86_64-pc-windows-msvc` or executed on Windows).
  - Windows `CommandExt::creation_flags(0x0800_0000)` branches in `pdf_ocr_models.rs`, `pdf_render.rs`, and `pdf_ocr.rs`.
  - `crates/desktop-app`, Windows `AppContainerSandbox` Python tests, and Playwright browser tests.
- Live Set P6 measurement on real historical journal PDFs (including CCITT/JBIG2/JPEG scans).

---

## 6. What Does Not Work Yet / Known Limitations

- **Unsupported raster image codecs in pure-Rust renderer:** `RasterXObjectRenderer` does not decode `/DCTDecode` (JPEG), `/JPXDecode` (JPEG2000), `/CCITTFaxDecode` (Group 3/4 fax), `/JBIG2Decode`, or pure vector text pages without an external renderer (`ExternalCommandRenderer::pdftoppm` is implemented in `pdf_render.rs` but not auto-wired in `routes_ocr.rs` yet). Those pages currently return a clear error rather than crashing or faking text.
- **HTTP body limit for 61–64 MiB uploads over live HTTP:** As noted in Merge Notes, `http_srv.rs` (untouched per Do-Not-Edit list) caps non-`/upload` bodies at `60 MiB`, so live HTTP uploads between 60 and 64 MiB (or >45 MiB base64 JSON) will need the 1-line `http_srv.rs` allowance at merge time.
- **Windows AppContainer ACL on user-installed `tesseract.exe`:** If a Windows user installs Tesseract under a private directory lacking `ALL APPLICATION PACKAGES (S-1-15-2-1) RX`, AppContainer launch will fail cleanly with `OcrHelperError::Spawn` rather than weakening containment.
