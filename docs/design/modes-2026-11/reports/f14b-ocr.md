# Report: `f14b-ocr` (package F14b — Free Default Tesseract OCR Text Layer, Local Detection & AppContainer Helper)

- **Merged base commit:** `2104137a228ae7948ee1d5ef6228d2e30304b461` (`origin/gemini/work/fixed`, merged via `--no-ff` commit `d4c73d2` into earlier `origin/gemini/modes/f14b-ocr` checkpoint `ebb16f0`, originally based on `6491394e28ef390f5f40e5fe56ac3aec8c1a97d7`)
- **Work branch:** `work/f14b-ocr` (pushed to `gemini/modes/f14b-ocr`)
- **Toolchain on this machine:** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `cargo 1.100.0 (3acc79c0c 2026-11-05)`, Node `v22.23.5`, `tesseract 5.3.4` (`tesseract-ocr-eng`), Python 3 (`sympy 1.14.0`, `mpmath 1.3.0`, `numpy 2.5.5`, `scipy 1.18.1`, `matplotlib 3.11.2`), running as non-root `uid=1001(antigravity)` on Linux x86_64.
- **Unchanged guard rails:**
  - `Cargo.lock` and all `Cargo.toml` files: **untouched** (0 new crate dependencies).
  - `crates/research-tools/tests/golden/tool_defs.txt` & `tool_profiles.txt`: **untouched** (0 tool definitions added or changed).
  - `crates/policy`, `crates/ledger`, `crates/agent`, `.github/`, and all Do-Not-Edit files (`ingest.rs`, `pdf_match.rs`, `lines_sent.rs`, `lit_*`, `http_client.rs`, `net_rate.rs`, `solve_*`, `skeleton/html*`, `routes_tools.rs`, `http_srv.rs`, `routes_session.rs`, `state.rs`): **untouched**.
  - `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`: **Pass** (`No touched source file is over 600 lines.`).
  - Zero skipped or `#[ignore]` tests added; zero network downloads; zero hardcoded or invented `.traineddata` SHA-256 digests.

---

## 1. What was implemented (package F14b)

### Checkpoint 1 — `PageBasis::Ocr`, Renderer Interface, Local Tesseract Detector & Contained `OcrEngine` Helper

- **Core `PageBasis::Ocr` & honesty updates** (commits `5f54c0e`, `67cda01`):
  - `crates/research-tools/src/pdf_pages.rs` (572 lines): added `PageBasis::Ocr` (`as_str() == "ocr"`, `is_scan() == true`, `needs_ocr() == false`).
  - `crates/research-tools/src/pdf_pages_tests.rs` (131 lines): unit test verifying `PageBasis::Ocr` invariants and JSON serde `"ocr"`.
  - `crates/research-tools/src/skeleton/pdf_honesty.rs` (527 lines): updated `node_page_basis`, `match_pdf_quote_for_display`, and `classify_line_range` to treat `PageBasis::Ocr` as `PdfTextOrigin::ScanOcrText` (`is_ocr: true`, `render_math: false`, `"matched in OCR text, p. N"`, worst-basis order `None > Ocr > ScanTextLayer > TextLayer`).
  - `crates/research-tools/src/skeleton/pdf.rs` (348 lines): updated `summarize_pages` so `PageBasis::Ocr` pages are included in `scan_text_pages` and carry `PDF_QUALITY_SCAN_TAG` (`"(scan)"`) in `quality_reason`.
  - `crates/research-tools/src/pdf_pages_bench.rs` (253 lines): generalized `scans_reported_as_text_layer` check to `expected_basis.is_scan()`.
  - `crates/desktop-server/src/investigation_mapview_v7.rs` (588 lines): emits `"ocr": true` and `"page_basis": "ocr"` (preserving conservative `Ocr > ScanTextLayer` precedence across multi-page spans).

- **Pure-Rust PDF page renderer & PNG codec** (commits `fd85bd3`, `609e61b`, `088347a`):
  - `crates/research-tools/src/pdf_render_png.rs` (376 lines, new): pure-Rust PNG encoder, bounded zlib decoder (`flate2`, defeating zip bombs before pixel allocation), all 5 PNG scanline filters, PNG predictor reversal (`/Predictor 10..=15`), pixel converters (1-bit packed monochrome with `/Decode`, 8-bit Gray/RGB/RGBA, 8-bit CMYK, 1/4/8-bit `/Indexed /DeviceRGB`), and normalized PNG cropper.
  - `crates/research-tools/src/pdf_render.rs` (536 lines, new): `PdfPageRenderer` trait (`render_page`, `render_crop`, `render_limits`), `RenderedPage`, `CropRect` (normalized `[0, 1]`), `RenderLimits` (`MAX_RENDER_WIDTH/HEIGHT = 8192`, `MAX_RENDER_PIXELS = 36M`, `MAX_RENDER_PNG_BYTES = 32 MiB`, `MAX_RENDER_PDF_BYTES = 64 MiB`, `DEFAULT_RENDER_TIMEOUT_MS = 15_000`), `RasterXObjectRenderer`, opt-in `ExternalCommandRenderer::pdftoppm`, `FallbackPageRenderer`, and injectable `MapPageRenderer`.
  - `crates/research-tools/src/pdf_render_tests.rs` (393 lines, new): 7 unit tests covering PNG round-trips, all 5 filters, crop clamping/validation, 1-bit/8-bit Gray/RGB/CMYK/Indexed/predicted XObjects, dimension-bomb rejection, fallback delegation, and `render_crop` custom `RenderLimits` enforcement.

- **Local Tesseract & `tessdata` detector + runtime SHA-256 provenance** (commits `e455fd4`, `609e61b`, `fb66e09`):
  - `crates/research-tools/src/pdf_ocr_models.rs` (544 lines, new): `DEFAULT_OCR_ENABLED = true` (Owner Decision 2026-11-26), `OCR_INSTALL_PROMPT = "Install Tesseract to enable OCR"` (Coordinator Clarification 2026-11-26), `OcrLocalConfig`, `OcrEnvProbe`, `OcrStatus`, `OcrModelProvenance`, `validate_tesseract_path` / `validate_tessdata_dir` (rejecting NUL bytes, UNC/network paths, and non-`tesseract` executable basenames), strict explicit override precedence, discovery precedence (`RE_SEARCH_TESSERACT`/`TESSDATA_PREFIX` -> `PATH` -> sibling `tessdata` -> standard Windows & Unix locations), env-scrubbed non-deadlocking `probe_tesseract_version` with concurrent capped pipe draining, `format_ocr_engine_id`, and streaming 64 KiB runtime disk SHA-256 hashing (`hash_local_traineddata`). Zero network downloads, zero hardcoded hashes.
  - `crates/research-tools/src/pdf_ocr_models_tests.rs` (369 lines, new): 12 unit tests verifying precedence, strict explicit override non-fallback, UNC & non-tesseract path rejection, bounded pipe draining, missing binary/missing `eng.traineddata` install prompt, runtime SHA-256 computation, and lang-tag sanitization.

- **Contained Tesseract helper & `OcrEngine` trait in `worker`** (commits `5e53cdd`, `67cda01`, `3f962ed`):
  - `crates/worker/src/ocr_helper.rs` (523 lines, new): `OcrEngine` trait, `OcrPageInput`, `OcrPageResult`, `OcrHelperError`, `TesseractCliConfig`, `TesseractCliEngine`, `DEFAULT_OCR_JOB_MEMORY_LIMIT_BYTES` (`1 GiB`), and deterministic `FakeOcrEngine`. On `#[cfg(windows)]`, canonicalizes `tesseract_exe`, creates an ephemeral per-page `AppContainerProfile` (`network_restricted: true`, `filesystem_restricted: true`, `KILL_ON_JOB_CLOSE`, `JOB_OBJECT_LIMIT_JOB_MEMORY = 1 GiB`), stages ONLY `page.png` and requested `.traineddata` inside `<data_dir>/ocr-stage/`, applies `grant_icacls_inside` (`RX` on stage dir, `M` only on `out`/`tmp`), launches via `win::spawn_extended`, enforces per-page timeout (`kill_tree()` on expiry) and bounded output reads (`2 MiB`), and cleans up via RAII. On `#[cfg(not(windows))]`, runs isolated ephemeral-dir subprocess with minimal env, timeout kill/reap, and bounded reads, reporting containment honestly (`backend: "isolated-tempdir"`, `filesystem_restricted: false`, `network_restricted: false`).
  - `crates/worker/src/ocr_helper_tests.rs` (289 lines, new): 7 unit tests covering input/PNG/lang validation, `FakeOcrEngine`, missing binary/tessdata/traineddata `Unavailable`, bounded text reads, and `wait_with_timeout` exit/timeout/error kill paths.
  - `crates/worker/src/lib.rs` (516 lines): exports `pub mod ocr_helper;`.

### Checkpoint 2 — OCR Pipeline, 64 MiB Scan Ingest, Authenticated Routes, UI & Real Set P6 Measurement

- **OCR pipeline, background job & 64 MiB scan ingest** (commits `ec131f7`, `fb66e09`, `088347a`):
  - `crates/research-tools/src/pdf_ocr.rs` (509 lines, new): `OcrPageRunner` trait, `FakeOcrPageRunner`, `LocalTesseractPageRunner` (including Windows `TEMP`/`TMP` under `env_clear()`), `PdfOcrOptions` (defaulting to `PdfExtractOptions::user_supplied()` = 64 MiB, `DEFAULT_OCR_ENABLED = true`), `extract_and_ocr_pdf_pages` / `_with_progress` (selectively OCRs ONLY `PageBasis::None` pages, keeps `TextLayer` & `ScanTextLayer` byte-identical, splices `\n\x0c\n`-separated reading file matching `pdf_pages.rs` line math, surfaces `"Install Tesseract to enable OCR"` when unavailable while preserving `PageBasis::None`), and `cache_pdf_ocr_in_library` (stores extracted+OCR text and `PageBasis::Ocr` in F10 `paper_text`).
  - `crates/research-tools/src/pdf_ocr_job.rs` (200 lines, new): thread-capable `PdfOcrJob` with `Queued`/`ExtractingText`/`OcrPage`/`Done`/`Cancelled`/`Failed` stages, `snapshot()`, and cooperative `cancel()`.
  - `crates/research-tools/src/ingest_ocr.rs` (324 lines, new): `ingest_pdf_with_ocr`, `ingest_pdf_path_with_ocr`, and `pin_ocr_outcome_to_session` (rejects 0-byte and `> 64 MiB` payloads, atomic materialization under `sessions/<id>/sources/`, immutable fresh `source.ingested` receipts pinning `pdf_extractor`, `pdf_pages`, `ocr_engine`, and runtime `ocr_models[{lang, sha256, bytes}]`, plus full-document F10 `Library` `paper_text` caching while skipping cache overwrites on partial page-range slices).
  - `crates/research-tools/src/pdf_ocr_tests.rs` (387 lines, new): 6 unit tests covering selective OCR, missing runner prompt, disabled config, background job + cancellation, receipt pinning, F10 library caching, empty-PDF rejection, partial page-range cache isolation, `quote_core::verify_quote`, and `skeleton::match_pdf_quote_for_display`.
  - `crates/research-tools/src/lib.rs` (373 lines): exports `ingest_ocr`, `pdf_ocr`, `pdf_ocr_job`, `pdf_ocr_models`, `pdf_render` (and private `pdf_render_png`).

- **Authenticated desktop routes & 64 MiB PDF serving** (commits `25bc60d`, `c487fc7`, `d4c73d2`, `fb66e09`, `088347a`):
  - `crates/desktop-server/src/original_pdf.rs` (44 lines): raised cap from `MAX_PDF_BYTES` (16 MiB) to `MAX_USER_PDF_BYTES` (64 MiB).
  - `crates/desktop-server/src/ocr_settings.rs` (198 lines, new): atomic `<data_dir>/ocr_settings.json` sidecar (zero `state.rs` growth), strict path/basename/UNC validation on `PUT /api/settings/ocr`, strict explicit `tesseract_path`/`tessdata_dir` override status evaluation, `GET`/`PUT /api/settings/ocr`.
  - `crates/desktop-server/src/routes_ocr.rs` (526 lines, new): `WorkerOcrRunner` bridging `worker::ocr_helper::TesseractCliEngine` (AppContainer on Windows) into `research_tools::pdf_ocr::OcrPageRunner`; process-wide capped background job registry scoped by `(data_dir, session_id)` preferring eviction of already-joined jobs; `POST /api/sessions/{id}/ocr/ingest` (sync & async `202`, JSON base64 / multipart / raw `application/pdf`, serialized ledger writes under `state.ledger_write_lock`); `GET /api/sessions/{id}/ocr/jobs/{job_id}`; `POST /api/sessions/{id}/ocr/jobs/{job_id}/cancel` (sets `cancelled = true` and releases buffered `raw_bytes` so cancel-after-finish never pins empty 0-byte blobs); `GET /api/sessions/{id}/sources/{source}/page-image?page=N&crop=x,y,w,h&dpi=D` serving `image/png` with `X-Content-Type-Options: nosniff`, `Content-Security-Policy: sandbox`, and `Cache-Control: no-store`.
  - `crates/desktop-server/src/lib.rs` (266 lines) & `crates/desktop-server/src/api_dispatch.rs` (519 lines): module & route wiring (cleanly merged with `settings/literature` from `gemini/work/fixed`).
  - `crates/desktop-server/tests/ocr_routes.rs` (394 lines, new): 4 integration tests covering auth (`401` unauthenticated & cookie-only, `200` with token/Bearer), settings CRUD/validation (including UNC & non-tesseract executable rejection)/immutability, sync & async ingest/job isolation/cancel-after-finish race guard, and full/cropped `page-image` PNG serving & security headers.

- **UI assets & Node tests** (commits `1708e62`, `9d88d57`):
  - `crates/desktop-server/assets/ocr_settings.js` (244 lines, new): zero-`innerHTML` settings panel & status banner (`DEFAULT_OCR_ENABLED = true`, `"Install Tesseract to enable OCR"` when unavailable, cleanly separates configured overrides from auto-detected local paths/languages).
  - `crates/desktop-server/assets/page_image_viewer.js` (282 lines, new): zero-`innerHTML` scanned page image viewer fetching `/api/sessions/{id}/sources/{source}/page-image` via authenticated `X-ReSearch-Token` + `Blob` + race-safe `URL.revokeObjectURL`, aligned normalized `[0, 1]` crop validation, plain-text OCR quote display (never KaTeX math), and `bindQuoteChipPageImageViewer` adapter for `quote_chip.js`.
  - `crates/desktop-server/assets/investigation_node_model.js` (452 lines): recognizes `page_basis === 'ocr'` and `anchor.ocr` in Block B quote chips so OCR quotes show `"matched in OCR text, p. N"` and open the page image.
  - `crates/desktop-server/src/assets.rs` (530 lines): registers `/assets/ocr_settings.js` and `/assets/page_image_viewer.js`.
  - `crates/desktop-server/tests/ocr_ui.test.mjs` (359 lines, new): 4 Node tests covering settings panel, nested `GET /api/settings/ocr` contract, page image viewer auth/Blob lifecycle/crop validation, `quote_chip.js` integration, and zero-`innerHTML` XSS safety.

- **Self-authored Set P6 scanned PDF fixtures & pipeline test** (commit `f35017a`):
  - `crates/research-tools/tests/fixtures/p6_ocr/` (`manifest.json` 219 lines + 6 self-authored synthetic PDFs, total 100.3 KiB, zero third-party text).
  - `crates/research-tools/tests/pdf_ocr_pipeline.rs` (534 lines, new): end-to-end honesty/library/Solve-flag test + real-Tesseract Set P6 measurement harness.

---

## 2. Mandatory Real Set P6 Measurement (`00-overview.md` §7.2 Set P6)

Re-measured after merging `origin/gemini/work/fixed` (`2104137`) and applying all post-merge audit fixes, using real **`tesseract 5.3.4`** (`eng.traineddata`, observed runtime SHA-256 `7d4322bd2a774972...`, 4,113,088 bytes) via:

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
2. **Sidecar settings file (`<data_dir>/ocr_settings.json`) and process-wide job registry:** Because `state.rs` is on the Do-Not-Edit list, OCR settings use an atomic JSON sidecar mirroring `investigation_settings.rs`, and background OCR jobs live in a capped (64-entry) `OnceLock<Mutex<HashMap<...>>>` registry scoped by `(data_dir, session_id)`.
3. **Pure-Rust raster XObject renderer + opt-in external fallback:** To obey zero `Cargo.lock` edits, `RasterXObjectRenderer` decodes uncompressed and `/FlateDecode` (1/8-bit Gray, 8-bit RGB/CMYK, 1/4/8-bit Indexed, PNG predictors 10..=15) PDF Image XObjects directly to PNG using workspace `flate2`. `routes_ocr.rs` stays on pure-Rust `RasterXObjectRenderer` rather than auto-spawning unsandboxed external `pdftoppm` from HTTP routes.
4. **No Unix-only shell-outs in tests:** All Rust tests are cross-platform (no `#!/bin/sh` test scripts); real Tesseract execution in `pdf_ocr_pipeline.rs` runs automatically whenever `ocr_status` detects local `tesseract` + `eng.traineddata` and falls back to testing `OCR_INSTALL_PROMPT` + `FakeOcrPageRunner` when absent (zero `#[ignore]` or skipped tests).
5. **Post-merge independent `pro` audit remediation (`3f962ed`, `fb66e09`, `088347a`):** After merging `origin/gemini/work/fixed` (`2104137`), ran a full adversarial independent audit and fixed every in-scope finding:
   - **Windows compile break (`E0063`) from PR #67 (`solve-s18b-windows-runtime`):** Added `DEFAULT_OCR_JOB_MEMORY_LIMIT_BYTES` (`1 GiB`) and populated `WorkerLaunch::job_memory_limit_bytes` plus `tesseract_exe` canonicalization in `crates/worker/src/ocr_helper.rs` (`3f962ed`).
   - **Path validation, strict explicit overrides, and pipe-deadlock fix:** Added `validate_tesseract_path`/`validate_tessdata_dir` (rejecting UNC/network paths and non-`tesseract` executable basenames), enforced strict non-fallback semantics for explicit `tesseract_path`/`tessdata_dir` overrides in `pdf_ocr_models.rs`, scrubbed env and drained stdout/stderr concurrently in `probe_tesseract_version`, and passed `TEMP`/`TMP` to Windows `LocalTesseractPageRunner` (`fb66e09`).
   - **Async cancel-after-finish race, partial-range library cache guard, and `render_crop` limits:** Guarded `handle_ocr_job_status` against pinning empty 0-byte `raw_bytes` when cancelled after background completion, preferred evicting already-joined OCR jobs, rejected 0-byte PDFs in `ingest_ocr::enforce_user_pdf_cap`, skipped full-document `library.paper_text` caching on partial page-range slices, and wired `PdfPageRenderer::render_limits()` into `render_crop` (`088347a`).

### Deviations from `00-overview.md` 2.12
- **Per Coordinator Clarification (2026-11-26):** Local system `tesseract` + `tessdata` detection is used instead of in-app downloading of language models over HTTPS; no `tessdata` host was added to `ALLOWLISTED_HTTPS_HOSTS`, and `.traineddata` SHA-256 digests are computed from local files at runtime instead of pinned as hardcoded constants.
- Split `pdf_render.rs` into `pdf_render.rs` + `pdf_render_png.rs`, and split desktop routes into `ocr_settings.rs` + `routes_ocr.rs`, so every file remains well under 600 lines.

### Assumptions
- On Windows, standard `C:\Program Files\Tesseract-OCR\tesseract.exe` inherits `ALL APPLICATION PACKAGES (S-1-15-2-1) RX` permissions from `Program Files` and can launch inside a zero-capability `AppContainerProfile`; user-profile installs (e.g. `%LOCALAPPDATA%\Programs\Tesseract-OCR`) or custom paths without `S-1-15-2-1` RX may require ACL adjustment or staging on Windows (untested on Linux).
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
- `crates/desktop-server/src/api_dispatch.rs`: wired 6 OCR routes alongside `GET/PUT /api/settings/literature`.
- `crates/desktop-server/src/assets.rs`: registered `/assets/ocr_settings.js` and `/assets/page_image_viewer.js` (530 lines total, safely under the 580-line split threshold).
- `crates/desktop-server/assets/investigation_node_model.js`: recognizes `page_basis === 'ocr'` and `anchor.ocr` in Block B quote chips.

### Integration points & Bugs Noticed in Out-of-Scope / Do-Not-Edit Files (Not Edited Here)
- **`crates/desktop-server/src/http_srv.rs:18,227-235` (Do-Not-Edit):** `http_srv.rs` caps non-`/upload` HTTP request bodies at `MAX_BODY_BYTES = 60 MiB`. Over live HTTP, a 61–64 MiB raw PDF (or >~45 MiB base64 JSON PDF) to `POST /api/sessions/{id}/ocr/ingest` will hit HTTP `413` until `["api", "sessions", _, "ocr", "ingest"]` is added to the raised upload body cap (`64 MiB` raw / `~86 MiB` base64).
- **UI shell mounting (`crates/desktop-server/assets/index.html:429-435`, `investigation_v3_shell.js:361`, `mode_stages.js:269-320`, `solve_steps.js:79-91`):** `/assets/ocr_settings.js` and `/assets/page_image_viewer.js` are served and tested against `quote_chip.js`, ready to be mounted into the Settings dialog and quote-chip click handlers. Notably, `mode_stages.js:269-320` currently mounts `createSolveWorkspace(...)` without passing `onOpenSource` / `onOpenPageImage`, causing `quote_chip.js:169-170` to disable Solve quote-chip open buttons, and `investigation_v3_shell.js:361` routes `onOpenPageImage` to `hooks?.openPdf` instead of `page_image_viewer.js`.
- **Solve OCR quote metadata & strict schema (`crates/research-tools/src/quote_core.rs:274`, `crates/desktop-server/src/solve_routes_ext.rs:88-96`, `crates/research-tools/src/solve_state.rs:407-421`, `crates/research-tools/src/solve_badge.rs:336-347`, `crates/desktop-server/src/solve_view.rs:239` — Do-Not-Edit `solve_*`):**
  1. `EvidenceQuotePayload` (`quote_core.rs:274`) uses `#[serde(deny_unknown_fields)]` without `ocr`/`page`, while `solve_routes_ext.rs:88-96` injects `"ocr": true` and `"page": N` into the serialized `evidence.quote` payload.
  2. `solve_state.rs:407-421` does not fall back to `source.ingested.pdf_pages` when `evidence.quote` lacks an explicit `"ocr"` field.
  3. `solve_badge.rs:336-337` requires `tier.effective == Tier::Cited` (rather than `tier.own == Tier::Cited`) to keep `DisplayGrade::MatchedQuoteScannedPage` and `TAG_OCR` when an upstream dependency downgrades `effective`.
  4. `solve_view.rs:239` serializes `QuoteRef` without enriching `{ page, quote, ocr, paper }`, leaving Solve quote chips without page numbers or quote text.
- **`crates/research-tools/src/prior_work.rs:221` (out of scope):** `TextBasis::parse` maps `"ocr" | "none" => Some(Self::Ocr)`, which would classify an un-OCRed `"none"` basis as `TextBasis::Ocr`.
- **`f14c-vision-transcribe`:** Disjoint (`lines_sent.rs` untouched). Can reuse `research_tools::pdf_render::PdfPageRenderer` (`render_page` / `render_crop`) and `GET /api/sessions/{id}/sources/{source}/page-image`.

---

## 5. Verification Commands & Exact Results

| Command | Result |
|---|---|
| `id` | `uid=1001(antigravity)` (non-root) |
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | **Pass** — `No touched source file is over 600 lines.` |
| `git diff --stat origin/gemini/work/fixed -- Cargo.lock Cargo.toml crates/agent crates/policy crates/ledger tools/golden ...` | **Pass** — empty diff across all Do-Not-Edit paths |
| `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` | **Pass** — exit 0 across all JS assets |
| `node --test crates/desktop-server/tests/*.test.mjs` | **Pass** — **248 passed, 0 failed, 0 skipped** (9.6 s) |
| `unset RE_SEARCH_PYTHON && cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` | **Pass** — 0 warnings, 0 errors |
| `unset RE_SEARCH_PYTHON && cargo test --workspace --locked --no-fail-fast` | **Pass** — **1350 passed, 0 failed, 2 ignored** across 174 test/doc-test targets (the 2 ignored are pre-existing live-evaluation tests in `crates/agent/tests/eval_harness.rs`; zero new ignored tests) |
| `unset RE_SEARCH_PYTHON && cargo test -p research-tools --locked --test pdf_ocr_pipeline -- --nocapture` | **Pass** — **2 passed, 0 failed, 0 ignored** (real `tesseract 5.3.4` Set P6 measurement: 31/31 sentences matched, 0.00 CER/100, `wrong_page_chip == 0`, `scanned_reported_text_layer == 0`) |

### Pre-existing failures
- None observed on Linux (non-root with `RE_SEARCH_PYTHON` unset, after installing `tesseract-ocr`, `tesseract-ocr-eng`, and Python `sympy`/`mpmath`/`numpy`/`scipy`/`matplotlib` on the test VM per `RULES.md`). Running `cargo clippy` without `-A clippy::manual_unwrap_or` still triggers the known pre-existing warning in `crates/desktop-server/src/evidence.rs:744` documented in `RULES.md`.

### What could not be compiled or run on Linux
- `#[cfg(windows)]` code paths and Windows-only tests, specifically:
  - `crates/worker/src/ocr_helper.rs` Windows `AppContainerProfile` + `grant_icacls_inside` + `win::spawn_extended` (now updated for `WorkerLaunch::job_memory_limit_bytes` and exe canonicalization and audited line-by-line against `appcontainer.rs`, `stage.rs`, `sandbox.rs`, `win.rs`, and `win/launch.rs`, but NOT compiled with `x86_64-pc-windows-msvc` or executed on Windows).
  - Windows `CommandExt::creation_flags(0x0800_0000)` branches in `pdf_ocr_models.rs`, `pdf_render.rs`, and `pdf_ocr.rs`.
  - `crates/desktop-app`, Windows `AppContainerSandbox` / `job_memory_limit` / `python_worker` tests, and Playwright browser tests.
- Live Set P6 measurement on real historical journal PDFs (including CCITT/JBIG2/JPEG scans).

---

## 6. What Does Not Work Yet / Known Limitations

- **Unsupported raster image codecs in pure-Rust renderer:** `RasterXObjectRenderer` does not decode `/DCTDecode` (JPEG), `/JPXDecode` (JPEG2000), `/CCITTFaxDecode` (Group 3/4 fax), `/JBIG2Decode`, or pure vector text pages without an external renderer (`ExternalCommandRenderer::pdftoppm` is implemented in `pdf_render.rs` but intentionally not auto-wired into HTTP routes outside AppContainer). Those pages return a clear error rather than crashing or faking text.
- **HTTP body limit for 61–64 MiB uploads over live HTTP:** As noted in Merge Notes, `http_srv.rs` (untouched per Do-Not-Edit list) caps non-`/upload` bodies at `60 MiB`, so live HTTP uploads between 60 and 64 MiB (or >45 MiB base64 JSON) will need the 1-line `http_srv.rs` allowance at merge time.
- **Windows AppContainer ACL on user-installed `tesseract.exe`:** If a Windows user installs Tesseract under a private directory lacking `ALL APPLICATION PACKAGES (S-1-15-2-1) RX` (such as `%LOCALAPPDATA%\Programs\Tesseract-OCR`), AppContainer launch may fail cleanly with `OcrHelperError::Spawn` rather than weakening containment.
- **Library rebuildability for `paper_text`:** `pin_ocr_outcome_to_session` caches full-document OCR text in F10 `Library` via `upsert_paper_text`, whereas `library_rebuild.rs` rebuilds `paper_text` from `paper.text_extracted` events (not `source.ingested`).
