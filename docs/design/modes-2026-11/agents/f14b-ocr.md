# Brief f14b-ocr — Free Default Tesseract (Apache-2.0) OCR Text Layer, Local Detection & AppContainer Helper (F14b)

**Status:** Unblocked on `gemini/work/fixed` (`F4`, `F6`, `F10`, `F14a`, `S2`, `V17` are merged, and **Owner Decision 2026-11-26** explicitly approved F14b with **Tesseract Apache-2.0 as the free, default OCR text layer**). Zero `Cargo.lock` edits!
**Package:** `F14b` (pure-Rust page-image extraction/rendering interface `pdf_render.rs`, 64 MiB scan ingest path `ingest_ocr.rs` + `original_pdf.rs`, local `tesseract` + `tessdata` detector/SHA-256 provenance `pdf_ocr_models.rs`, `AppContainerSandbox` Tesseract runner `worker::ocr_helper`, default free OCR pipeline `pdf_ocr.rs` producing `\x0c`-broken `PageBasis::Ocr` text cached in F10 `paper_text`, plus `routes_ocr.rs` & UI).
**Delivery Choice (2026-11-26 Coordinator Clarification):** Use **local system Tesseract detection** (do NOT bundle a separate Tesseract installer or add any `tessdata` host to `ALLOWLISTED_HTTPS_HOSTS`, and **NEVER invent SHA-256 hashes or download binaries/models over the network**). When local `tesseract` or `eng.traineddata` is missing, surface `"Install Tesseract to enable OCR"`.
**Governing design:**
- `00-overview.md`: 2.7 (`paper_text`), 2.10, 2.12 (F14b, 4 scanned-text honesty rules), 7.2 Set P6, 7.3 (`wrong_page_chip == 0`, `scanned_reported_text_layer == 0`).
- `agents/RULES.md` (zero `Cargo.lock` changes, `<= 600`-line limit, no `#[ignore]` tests, non-root Linux workspace Clippy & tests, push after each checkpoint).

---

## 1. Verified Interfaces & Disjointness on `gemini/work/fixed`

- **`crates/research-tools/src/pdf_pages.rs` (`569`L):** `PageBasis` has `{ TextLayer, ScanTextLayer, None }`, while `library_schema::PageBasis` already has `Ocr`. Add `PageBasis::Ocr` (`<= 12` lines in `pdf_pages.rs`, keeping `<= 600`L!) and update any exhaustive match arms in `crates/research-tools/src/skeleton/pdf*.rs` (disjoint from `V18`'s `skeleton/html*.rs`).
- **Strictly Do NOT edit over-limit / tight files:** `ingest.rs` (`590`L), `pdf_match.rs` (`589`L), `routes_tools.rs` (`610`L), `lines_sent.rs` (`503`L — owned by `f14c-vision-transcribe`!), `net_gate.rs` / `ALLOWLISTED_HTTPS_HOSTS`, or anything under `crates/agent/`. Put 64 MiB scan ingest in `ingest_ocr.rs` + `routes_ocr.rs` (`MAX_USER_PDF_BYTES = 64 MiB` in `original_pdf.rs`).
- **UI components already ready:** `crates/desktop-server/assets/quote_chip.js` ALREADY renders `"matched in OCR text, p. N"`, `"OCR"` tag, and `openPageImage` without KaTeX math!

---

## 2. Checkpoints — Push to `gemini/modes/f14b-ocr` After Each!

### Checkpoint 1 — Renderer Interface, Local Tesseract/Tessdata Detector, `OcrEngine` Helper & `PageBasis::Ocr` Core
1. `crates/research-tools/src/pdf_render.rs` (+ `pdf_render_tests.rs`):
   - Define `PdfPageRenderer` (`render_page(&self, pdf_bytes: &[u8], page_1based: usize, dpi: u16) -> Result<RenderedPage, String>`, `render_crop(...) -> Result<RenderedPage, String>`) with dimension/byte/timeout caps and pure-Rust PNG + PDF raster-XObject extraction (using `flate2` already in workspace `Cargo.toml`) PLUS an injectable / external helper adapter so synthetic PDF tests and real PNG scan pages render cleanly with **zero `Cargo.lock` changes**!
2. `crates/worker/src/ocr_helper.rs` (+ `ocr_helper_tests.rs`, exported in `worker/src/lib.rs` `486`L):
   - Runs locally installed Apache-2.0 `tesseract` inside `AppContainerSandbox` on `#[cfg(windows)]` (`network_restricted: true`, `filesystem_restricted: true`, `KILL_ON_JOB_CLOSE`), staging ONLY the single rendered PNG page image + read-only local `tessdata` directory, enforcing per-page timeout + output byte caps.
   - Exposes a platform-neutral `OcrEngine` trait (`TesseractCliEngine` + test `FakeOcrEngine`) so unit tests run deterministically with zero skips!
3. `crates/research-tools/src/pdf_ocr_models.rs`:
   - **Local detection & honest status (no network downloads, no invented hashes):**
     - Discover `tesseract` executable and local `tessdata` directory (`eng.traineddata` required for English default) across: explicit settings/override, `RE_SEARCH_TESSERACT` / `TESSDATA_PREFIX`, `PATH`, Windows standard install (`C:\Program Files\Tesseract-OCR\tesseract.exe` + `tessdata`), and Unix standard locations (`/usr/bin/tesseract`, `/usr/share/tesseract-ocr/*/tessdata`, `/opt/homebrew/...`).
     - Expose `ocr_status()` reporting `{ available: bool, tesseract_path, tessdata_dir, langs, message }`. When `tesseract` or `eng.traineddata` is missing, return `"Install Tesseract to enable OCR"` cleanly (never fake results or download files).
     - **Default-free-on** config helper (`DEFAULT_OCR_ENABLED = true`, overridable in user settings without mutating existing receipts).
     - At OCR runtime, compute the **actual SHA-256 of the local `.traineddata` file on disk** that was read so `source.ingested.ocr_models` records real, observed hashes rather than hardcoded or invented constants.
4. Run mandatory checks and push Checkpoint 1 to `gemini/modes/f14b-ocr`.

### Checkpoint 2 — OCR Pipeline, 64 MiB Scan Ingest, Routes, UI & Real Set P6 Coverage Measurement
1. `crates/research-tools/src/pdf_pages.rs` (`PageBasis::Ocr`, `is_scan() == true`, `needs_ocr() == false`), `pdf_ocr.rs`, `pdf_ocr_job.rs`, `ingest_ocr.rs`, `pdf_ocr_tests.rs`, `crates/research-tools/tests/pdf_ocr_pipeline.rs`:
   - Automatically OCRs `PageBasis::None` pages by default when local `tesseract` + `eng.traineddata` are present (leaving `TextLayer` & `ScanTextLayer` untouched!), splices `\x0c`-separated pages into the reading file, pins `ocr_engine` + computed `ocr_models[{lang, sha256}]` in `source.ingested` (using `PdfExtractOptions::user_supplied()` up to `64 MiB`), caches per-file text in F10 `paper_text`, never mutates existing receipts, and leaves `pdf_match` / `quote_core` working seamlessly on OCR text (`"OCR source"` flag in Solve, `"OCR"` tag in quotes, zero wrong page links).
   - When `tesseract` is missing on a scan upload, keep the `PageBasis::None` basis honest and return/surface `"Install Tesseract to enable OCR"` (never report a text layer!).
2. `crates/desktop-server/src/original_pdf.rs` (`43`L: raise `MAX_PDF_BYTES` -> `MAX_USER_PDF_BYTES`), `crates/desktop-server/src/routes_ocr.rs` (+ `tests/ocr_routes.rs`), and 1-line wiring in `lib.rs` / `api_dispatch.rs`:
   - Authenticated routes for 64 MiB user scan upload/ingest, OCR settings (`default free` toggle + local Tesseract status / `"Install Tesseract to enable OCR"` prompt + custom local `tesseract`/`tessdata` path config), background OCR job status/cancel, and `GET /api/sessions/{id}/sources/{source}/page-image?page=N&crop=...` (serving `image/png` with `X-Content-Type-Options: nosniff`).
3. `crates/desktop-server/assets/{ocr_settings,page_image_viewer}.js` (+ `tests/ocr_ui.test.mjs`) + `/assets/...` arms in `assets.rs`:
   - Safe DOM rendering (zero `innerHTML`, XSS-escaped), shows `"Install Tesseract to enable OCR"` when local Tesseract is unavailable, and integrates with `quote_chip.js`'s `onOpenPageImage` callback to show the original page image alongside `"matched in OCR text, p. N"`.
4. **Mandatory Real Set P6 Measurement Before Done (`00-overview.md` §7.2 Set P6):**
   - Measure **real OCR coverage on Set P6 (fax / JBIG2 / scanned PDF fixtures)** using real `tesseract-ocr` (report exact fixture count, pages OCR'd vs skipped, character/token recovery or match rate, `wrong_page_chip == 0`, and `scanned_reported_text_layer == 0` in `docs/design/modes-2026-11/reports/f14b-ocr.md`) BEFORE declaring F14b done!
5. Run full checks (`check_file_length.py`, `node --check`, `node --test`, non-root Linux workspace Clippy & `cargo test --workspace --locked`), write `docs/design/modes-2026-11/reports/f14b-ocr.md`, and push to `gemini/modes/f14b-ocr`.

---

## 3. Do-Not-Edit List & Verification (`RULES.md`)

- **Do NOT edit:** `Cargo.lock`, `Cargo.toml`, `crates/research-tools/src/{ingest,pdf_match,lines_sent,lit_*,http_client,net_rate,solve_*,skeleton/html*}.rs`, `crates/agent/*`, `crates/desktop-server/src/{routes_tools,http_srv,routes_session,state}.rs`, `tools/golden/*.txt`.
- Run on Linux **as non-root (`id -u != 0`) with `RE_SEARCH_PYTHON` unset**:
  ```bash
  python3 scripts/check_file_length.py --base origin/gemini/work/fixed
  for f in crates/desktop-server/assets/*.js; do node --check "$f"; done
  node --test crates/desktop-server/tests/*.test.mjs
  unset RE_SEARCH_PYTHON
  cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or
  cargo test --workspace --locked --no-fail-fast
  ```
- Push **only** via `git push origin HEAD:refs/heads/gemini/modes/f14b-ocr`. Never squash, never open/merge PRs, and end with `PUSHED`.
