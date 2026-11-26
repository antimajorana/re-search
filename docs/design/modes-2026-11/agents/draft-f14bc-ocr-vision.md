# Draft brief: `f14bc-ocr-vision` — Own OCR for `PageBasis::None` pages (F14b) and page images to vision models (F14c)

> **Status:** Draft brief prepared against base commit `832eab5bf856cec90b3a4a2f08f95827ec9d1acd` (`work/draft-next-lanes`). **BLOCKED ON EXPLICIT OWNER MEASUREMENT AND DEPENDENCY APPROVAL GATES** (Section 2). Do not launch implementation agents until those gates are cleared.

---

## 1. Packages and governing design sections

- **F14b — Own OCR for pages with no usable text layer (`PageBasis::None`):**
  - Renders `PageBasis::None` pages to images and runs a local Tesseract (Apache-2.0) helper process inside the existing Windows `AppContainerSandbox` with **no network**.
  - Language models (`tessdata`, English first) are downloaded once as an explicit user action in Settings and verified against pinned SHA-256 hashes before use.
  - Produces a `\x0c`-separated reading file with `basis: ocr`, pinning both the OCR engine version and language-model SHA-256 hash(es) in the `source.ingested` receipt and caching extracted text per file in F10's `paper_text` table.
  - Enforces all four scanned-text honesty rules (`00-overview.md` 2.12): quotes show `"matched in OCR text, p. N"` and open the page image; nothing from OCR text is drawn as mathematics; Solve Cited steps carry `"OCR source"`; Explore `judged` statuses from OCR text are tagged `"OCR"` and still require the user's confirming click.
- **F14c — Page images to vision-capable models for formulas:**
  - Adds image content-block support to `crates/agent` transports (explicitly gated by provider/model vision capability; fail-closed if unsupported) and renders user/app-selected PDF page images or crops.
  - Records every page sent to a model in `context.supplied` (extending `context.supplied` to allow page-range entries alongside line ranges, without ever crediting `SentLines` text-quote coverage).
  - Tags model output strictly as **`"transcription (model), unconfirmed"`** shown beside the page crop. **A model transcription from a page image is never a matched quote, never stored as `evidence.quote` / `QuoteRef`, and never an anchor.**
- **Governing design sections (with owner decisions of 2026-11-25):**
  - `00-overview.md` sections `0.1`, `2.1` (row F14), `2.5` (`context.supplied`), `2.7` (`paper_text`), `2.10` (evidence grades: "Matched quote, scanned page (OCR text)" and "Model statement"), `2.12` (F14b, F14c, honesty rules 1–4, tests & fixtures), `6` (build order), `7.2` Set P6 & `7.3` PDF/OCR pass rules, `8.1`, `8.2` #10 & #12, `8.3` #3 (OCR quotes in Explore/Solve, no OCR math rendering) & #4 (**OCR measurement decision**).
  - `investigate.md` rule `6.21` (PDF honesty), section `9` (F14, V17), section `10` (scan risks, unverified Tesseract packaging).
  - `explore.md` section `4.1` (`text_basis: ocr`), rule `C8` (OCR tag, page image, transcription is never evidence).
  - `solve.md` sections `4.4` (`OCR source` flag), `7.1`, `9` (`D13` transcription unconfirmed pattern).
  - `agents/RULES.md`, `agents/f14a-pdf-core.md`, and `reports/f14a-pdf-core.md`.

---

## 2. Start conditions — EXPLICIT OWNER MEASUREMENT & DEPENDENCY GATES

F14b and F14c **cannot start** under `RULES.md`'s no-`Cargo.lock`-change rule until the owner explicitly clears the gates below. Keep F14b and F14c independently dispatchable once their respective gates clear.

### Gate A — Explicit OCR measurement decision (`00-overview.md` 8.3 question 4 & 8.2 question 12)
**F14b MUST NOT START until all three steps complete:**
1. **Live P6 supplied by owner:** The owner supplies the 8 real papers of evaluation set P6 (2 journal PDFs with text layer and no arXiv version, 2 two-column PDFs, 2 pre-1990 scans with publisher text layers, and 2 scans with no text layer — one formula-heavy) plus independent answer keys (`00-overview.md` 7.2, 8.2 #12).
2. **F14a measured on live P6:** Run F14a (`pdf_pages.rs` / `measure_extractor`) on live P6 and file the measurement sheet in `docs/design/eval-2026-11/`, reporting the exact count and share of pages classified as `PageBasis::None` (no usable text layer) vs `ScanTextLayer` vs `TextLayer`, plus `pdf_pages_scan.rs` false-negative/false-positive checks and real scan file sizes (to confirm whether `MAX_USER_PDF_BYTES = 64 MiB` is sufficient).
3. **Explicit owner GO on F14b:** Per `00-overview.md` 8.3 #4 (*"decide after F14a has measured how many pages of set P6 really have no text. If publishers' text layers cover the owner's old papers, F14b can wait"*), the owner reviews the P6 sheet and explicitly authorizes starting F14b (or defers it).

### Gate B — Explicit owner approval of EVERY new dependency and packaging choice (`Cargo.lock`)
Verified on base `832eab5bf`: `Cargo.toml` / `Cargo.lock` currently contain **zero** PDF rasterizer, **zero** OCR helper, **zero** image encoder, **zero** `pdf_oxide`, and **zero** `base64` crate (only `pdf-extract = "0.12.1"`). Before coding, the owner must explicitly approve each item needed:

| Item | Needed by | Decision left with the owner |
|---|---|---|
| **1. PDF page renderer** | F14b (full pages) and F14c (pages & crops) | Choose and approve the renderer crate or helper binary (e.g., `pdfium-render` + Apache-2.0 PDFium DLL, Poppler/pdftoppm, MuPDF [AGPL-compatible], or a pure-Rust renderer) and its `Cargo.lock` / Windows packaging footprint. |
| **2. Tesseract helper & Windows installer packaging** | F14b | Marked **"Not verified"** in `00-overview.md` 2.12 and `investigate.md` 10. Owner approves how Tesseract (Apache-2.0) is built/bundled into the Windows installer, how the helper binary is staged for `AppContainerSandbox`, and the curated `tessdata` download URLs + pinned SHA-256 table (English first). |
| **3. `pdf_oxide` revisit (optional)** | F14a follow-up (if triggered by P6 sheet) | Only if live P6 measurement shows `pdf-extract-layout/0.12.1` fails on real two-column journal PDFs (`00-overview.md` 6 "What can move"). Plugs into F14a's existing `PdfExtractor` trait and `measure_extractor` harness. |
| **4. Image encoding / Base64** | F14b, F14c | Base64 encoding requires **no crate** (~40 lines of pure Rust). PNG/crop encoding either comes directly from the approved page renderer or requires an owner-approved lightweight PNG crate (`Cargo.lock`). |

### Gate C — Required package merges
- **Already merged on base `832eab5bf`:** F4 (`quote_core.rs`, `lines_sent.rs`), F6 (`quote_chip.js`), F10 (`library.rs`, `paper_text`), F14a (`pdf_pages*.rs`, `pdf_match.rs`, `ingest_pdf_pages`), S2 (`solve_state.rs`, `solve_status.rs` OCR flag handling).
- **Recommended merges before wiring:** F5 (`api_dispatch.rs`) for HTTP route registration; V17 (`skeleton/pdf.rs`) before end-to-end F14b-to-skeleton verification; at least one mode UI (V3/V8 or S13) before mounting F14c's side-by-side crop card in the main view.

---

## 3. Verified base interfaces vs future-merge interfaces

### Verified on base commit `832eab5bf` (code facts confirmed)
- **F14a (`crates/research-tools/src/pdf_pages.rs`, 569 lines):**
  - `PageMap::pages_needing_ocr(&self) -> Vec<usize>`, `PageMap::has_scanned_pages()`, `PageMap::page_of(line)`, `PdfExtractOptions::user_supplied()` (`MAX_USER_PDF_BYTES = 64 MiB`), `PdfDocumentPages`, `PdfExtractJob` (`pdf_pages_job.rs`, 232 lines), `PAGE_BREAK_CHAR = '\x0c'`.
  - **Gap found:** `pdf_pages::PageBasis` (line 41) has **3 variants** (`TextLayer`, `ScanTextLayer`, `None` — no `Ocr`), whereas F10's `library_schema::PageBasis` (originating in `crates/research-tools/src/library_ext_types.rs:76`, re-exported by `library_schema.rs:9`) already has **4 variants** (`TextLayer`, `ScanTextLayer`, `None`, `Ocr`).
- **F14a (`crates/research-tools/src/ingest.rs`, 590 lines; `pdf_match.rs`, 589 lines):**
  - `ingest::ingest_pdf_pages` pins `pdf_extractor` and `pdf_pages` on `source.ingested`. Both files are **10–11 lines below the 600-line ceiling** — do NOT grow them past 600 lines.
  - **End-to-end 64 MiB scan-cap gaps found:** `ingest.rs:24,114,148` caps `MAX_INGEST_INPUT_BYTES` at `32 MiB`, `crates/desktop-server/src/routes_tools.rs:21,54` (610 lines — already over 600, must not be edited here) caps HTTP uploads at `MAX_UPLOAD_BYTES_PER_FILE = 10 MiB`, and `crates/desktop-server/src/original_pdf.rs:32` (43 lines) caps serving original PDFs at `MAX_PDF_BYTES = 16 MiB`. All three sit below F14a's `MAX_USER_PDF_BYTES = 64 MiB`.
- **F10 (`crates/research-tools/src/library.rs`, 532 lines; `crates/ledger/src/library_schema.rs`, 315 lines):**
  - `paper_text(paper_key, version, file_sha256, extractor, text_blob, page_basis_json, extracted_ms)` and `Library::upsert_paper_text` are already live and index full text into `paper_fts`.
- **F4 (`crates/research-tools/src/lines_sent.rs`, 465 lines):**
  - `ContextSuppliedPayload` and `SuppliedRange` currently carry `#[serde(deny_unknown_fields)]` and accept **only** line ranges `{ source_id, path, file_sha256, start_line, end_line }`.
- **F6 (`crates/desktop-server/assets/quote_chip.js`, 178 lines):**
  - **Already implements** `"matched in OCR text, p. N"`, `"OCR"` tag, solid border, `openPageImage: true`, `options.onOpenPageImage` callback, and `textContent`-only rendering (never KaTeX/math).
- **S2 (`crates/research-tools/src/solve_state.rs`, 517 lines; `solve_status.rs`, 230 lines):**
  - Already parses `ocr: true` / `scanned: true` / `source_kind: "ocr"` on `evidence.quote` and emits flag `"OCR source"` and `DisplayGrade::MatchedQuoteScannedPage`.
- **Windows sandbox (`crates/worker/src/sandbox.rs`, 142 lines; `appcontainer.rs`, 377 lines):**
  - `Sandbox` trait (`WorkerLaunch`, `Contained`, `Containment`) and `AppContainerSandbox` (zero-capability Windows AppContainer + `KILL_ON_JOB_CLOSE` Job Object) already exist for spawning no-network helper processes.
- **Agent transports (`crates/agent/src/transport.rs`, 372 lines; `wire.rs`, `openai.rs`, `gemini.rs`, `responses.rs`, 587 lines):**
  - **Critical finding:** `ContentBlock` (`transport.rs:158–183`) currently has **NO `Image` variant** (only `Text`, `Thinking`, `RedactedThinking`, `ToolUse`, `ToolResult`, `Other`), and `openai.rs`, `gemini.rs`, and `responses.rs` silently ignore `ContentBlock::Other`. F14c MUST add first-class `ContentBlock::Image` encoding per provider with explicit vision-capability checks.
  - **Line-ceiling alert:** `crates/agent/src/responses.rs` is **587 lines** (only 13 lines below 600, with inline `#[cfg(test)] mod tests` at lines 541–587) and MUST be pre-split before adding `ContentBlock::Image` encoding.

### Future-merge interfaces (verify before coding)
- **F5 (`f5-f7-mode-shell`):** `crates/desktop-server/src/api_dispatch.rs` route registration seam and Settings route namespace (`/api/settings/...`).
- **V17 (`inv-v17-pdf-skeleton`):** `crates/research-tools/src/skeleton/pdf.rs` consumption of `pdf_pages::PageBasis` (ensure adding `PageBasis::Ocr` compiles cleanly with V17's match arms and preserves OCR basis on quote display).
- **Explore X10 (`idea_evidence.rs`) & Solve S8 (`solve_steps.rs`):** verify exact OCR metadata field passed on `evidence.quote` and how page-image viewers are opened from mode cards.

---

## 4. Smallest implementation checkpoints

### Part 1 — F14b (Own OCR; ONLY after Gates A and B clear)
1. **Checkpoint B1 — Page renderer abstraction (`pdf_render.rs`) + 64 MiB scan path end-to-end (`original_pdf.rs`, `ingest_ocr.rs`, `routes_ocr.rs`):**
   - Define a platform-neutral `PdfPageRenderer` trait (`render_page(bytes, page, dpi) -> RenderedPage`, `render_crop(bytes, page, rect_norm, dpi) -> RenderedPage`) with strict byte/dimension/timeout bounds and `catch_unwind` isolation, backed by the owner-approved renderer.
   - Lift `crates/desktop-server/src/original_pdf.rs:32` from `MAX_PDF_BYTES` (16 MiB) to `MAX_USER_PDF_BYTES` (64 MiB, confirmed after live P6 measurement).
   - Deliver an end-to-end 64 MiB user-supplied local-scan upload/ingest path strictly through `routes_ocr.rs` + `ingest_ocr.rs` (passing `PdfExtractOptions::user_supplied()` and respecting `MAX_USER_PDF_BYTES`), **without** growing 590-line `ingest.rs` or touching 610-line `routes_tools.rs` (general upload routes in `routes_tools.rs` stay untouched here and can only be lifted after S19 splits `routes_tools.rs` under 600 lines).
2. **Checkpoint B2 — Pinned language-model manager (`pdf_ocr_models.rs`) + sandboxed Tesseract helper (`worker::ocr_helper`):**
   - Curated `tessdata` registry (English `eng.traineddata` first) with exact pinned SHA-256 digests and sizes; user-initiated download in Settings routed through `http_client` allowlist, verified by SHA-256 in a temp file before atomic install into `<data_dir>/ocr/tessdata/`. Refuse any model file whose hash does not match.
   - `crates/worker/src/ocr_helper.rs`: invoke the owner-approved Tesseract helper inside `AppContainerSandbox` on Windows (`filesystem_restricted: true`, `network_restricted: true`, `KILL_ON_JOB_CLOSE`), staging ONLY the single rendered page image + read-only verified `tessdata` dir; enforce per-page wall timeout and output byte cap. Provide an injectable `OcrEngine` trait so Linux CI tests run deterministically with a fake engine without requiring a live Tesseract binary.
3. **Checkpoint B3 — OCR pipeline, `PageBasis::Ocr`, receipt pinning, and background job (`pdf_ocr.rs`, `pdf_ocr_job.rs`, `ingest_ocr.rs`):**
   - Reconcile `pdf_pages::PageBasis` with `library_schema::PageBasis` (`library_ext_types.rs:76`) by adding `PageBasis::Ocr` (`is_scan() == true`, `needs_ocr() == false`).
   - Run OCR only on pages returned by `PageMap::pages_needing_ocr()` (or user-selected range), splice OCR text into the `\x0c`-separated reading file while preserving existing `TextLayer`/`ScanTextLayer` pages untouched, and record `ocr_engine` (id + version) and `ocr_models` (`[{lang, sha256}]`) in the `source.ingested` receipt via `ingest_ocr.rs` (accepting user-supplied scans up to `MAX_USER_PDF_BYTES` via `PdfExtractOptions::user_supplied()` while keeping `ingest.rs <= 600` lines).
   - Never mutate an earlier session's `source.ingested` receipt or reading file; cache per-file OCR text in F10's `paper_text` with `extractor = "<base>+<ocr_engine>+<model_sha_prefix>"`.
4. **Checkpoint B4 — Routes & UI (`routes_ocr.rs`, `ocr_settings.js`, `page_image_viewer.js`):**
   - Authenticated endpoints for user-supplied scan ingest up to `MAX_USER_PDF_BYTES`, Settings listing/downloading/removing pinned OCR language packs (explicit user click only), OCR job start/status/cancel, and `GET /api/sessions/{id}/sources/{source}/page-image?page=N&crop=...` (returns PNG/WebP for `quote_chip.js`'s `onOpenPageImage`).

### Part 2 — F14c (Vision formulas; ONLY after Gate B item 1 clears; independent of Tesseract)
5. **Checkpoint C1 — Pre-split `responses.rs` + multimodal `ContentBlock::Image` in `crates/agent` (`vision.rs` + minimal transport arms):**
   - **Required pre-split commit (zero behaviour change):** move `crates/agent/src/responses.rs`'s inline `#[cfg(test)] mod tests` (`lines 541–587`) into a new `crates/agent/src/responses_tests.rs` (dropping `responses.rs` from 587 to ~540 lines) and verify all existing tests pass BEFORE adding `ContentBlock::Image` encoding.
   - Add `ContentBlock::Image { media_type: String, data_base64: String }` (strictly allowlisting `image/png`, `image/jpeg`, `image/webp` and a strict per-image / per-request byte cap) in `crates/agent/src/transport.rs`.
   - Implement wire encoding in `crates/agent/src/vision.rs` called by minimal match arms in `wire.rs` (Anthropic `type: "image", source: {type: "base64", ...}`), `openai.rs` (OpenAI/Ollama `image_url` data URI), `gemini.rs` (`inlineData`), and `responses.rs` (`input_image`).
   - Implement explicit vision-capability check per `(ProviderKind, model)`; **fail closed** with a clear `TransportErrorKind::Config` error when a model/provider is not vision-capable (never silently drop the image).
6. **Checkpoint C2 — `context.supplied` page-range extension & transcription record (`pdf_vision.rs`):**
   - Extend `crates/research-tools/src/lines_sent.rs` (465 lines, room for ~40 lines) with additive optional `#[serde(default, skip_serializing_if = "Vec::is_empty")] pub page_ranges: Vec<SuppliedPageRange>` (`{ source_id, raw_blob_sha256, start_page, end_page, crop? }`) on `ContextSuppliedPayload`, keeping old line-only events 100% compatible. **Invariant:** `SentLines::covers(...)` MUST NEVER credit line coverage from `page_ranges`.
   - Define `FormulaTranscription` (`source_id`, `raw_blob_sha256`, `page`, `crop`, `model`, `latex_draft`, `label: "transcription (model), unconfirmed"`, `confirmed: false`) and a compile-time/runtime guard preventing any `FormulaTranscription` from being stored as `evidence.quote` or `QuoteRef`.
7. **Checkpoint C3 — Vision transcription route & crop card (`routes_pdf_vision.rs`, `formula_transcription.js`):**
   - Endpoint to send a user-selected page/crop image to a vision-capable model (behind `AuthGuard`, counting against F9's `run_budget` and recording `context.supplied` with the exact pages sent).
   - UI card (`formula_transcription.js`, zero `innerHTML`) showing the page crop beside `"transcription (model), unconfirmed"`, never styled as a matched quote.

---

## 5. Owned files (disjoint, <= 600 lines each)

| Package | Owned files (new unless noted) | Notes |
|---|---|---|
| Shared B/C | `crates/research-tools/src/pdf_render.rs`, `pdf_render_tests.rs` | Pluggable page/crop renderer over owner-approved backend. |
| Shared B/C | `crates/desktop-server/src/original_pdf.rs` (43 lines, edit) | Raise `MAX_PDF_BYTES` -> `MAX_USER_PDF_BYTES` on line 32. |
| F14b | `crates/worker/src/ocr_helper.rs`, `ocr_helper_tests.rs` | Sandboxed Tesseract runner inside `AppContainerSandbox` (no network). |
| F14b | `crates/research-tools/src/pdf_ocr.rs`, `pdf_ocr_models.rs`, `pdf_ocr_job.rs`, `pdf_ocr_tests.rs`, `ingest_ocr.rs` | OCR pipeline, SHA-256 model verification, background job, 64 MiB user-scan ingest & receipt pinning (keeps `ingest.rs <= 600`). |
| F14b | `crates/desktop-server/src/routes_ocr.rs`, `crates/desktop-server/tests/ocr_routes.rs` | 64 MiB scan upload/ingest, Settings model download, OCR job, and page-image serving routes (avoids 610-line `routes_tools.rs`). |
| F14b | `crates/desktop-server/assets/ocr_settings.js`, `assets/page_image_viewer.js`, `crates/desktop-server/tests/ocr_ui.test.mjs` | Settings model controls & page image viewer for `quote_chip.js`. |
| F14b | `crates/research-tools/tests/pdf_ocr_pipeline.rs` | End-to-end Set P6 synthetic scan OCR & receipt immutability tests. |
| F14c | `crates/agent/src/responses_tests.rs` | Extracted from `responses.rs` lines 541–587 in a dedicated zero-behaviour-change pre-split commit before editing `responses.rs`. |
| F14c | `crates/agent/src/vision.rs`, `vision_tests.rs` | Provider image encoding, base64, capability gate, size bounds. |
| F14c | `crates/research-tools/src/pdf_vision.rs`, `pdf_vision_tests.rs` | Transcription record, honesty guard, `context.supplied` page-range builder. |
| F14c | `crates/desktop-server/src/routes_pdf_vision.rs`, `crates/desktop-server/tests/pdf_vision_routes.rs` | Vision formula-read endpoint (counts in F9 budget). |
| F14c | `crates/desktop-server/assets/formula_transcription.js`, `crates/desktop-server/tests/formula_transcription.test.mjs` | Side-by-side crop + `"transcription (model), unconfirmed"` card. |

---

## 6. Shared registration seams and "Do not edit" files

### Minimal registration seams (small isolated commits, documented in Merge notes)
- `crates/research-tools/src/pdf_pages.rs` (569 lines): add `PageBasis::Ocr` variant (+ `as_str`, `is_scan`, `needs_ocr`) — <= 10 lines added (must stay `<= 600`).
- `crates/research-tools/src/lines_sent.rs` (465 lines): add `SuppliedPageRange` and optional `page_ranges` field on `ContextSuppliedPayload` (validation only; `SentLines::covers` untouched).
- `crates/agent/src/transport.rs` (372 lines), `wire.rs`, `openai.rs`, `gemini.rs`, and `responses.rs` (**only after pre-splitting lines 541–587 into `responses_tests.rs`**): add `ContentBlock::Image` variant and delegate encoding to `crate::vision::*` (every touched file stays `<= 600` lines).
- `crates/research-tools/src/lib.rs`, `crates/worker/src/lib.rs`, `crates/agent/src/lib.rs`, `crates/desktop-server/src/lib.rs`: module exports.
- `crates/desktop-server/src/assets.rs` (365 lines): register the 3 new JS assets.
- `crates/desktop-server/src/api_dispatch.rs` (from F5): delegating arms for `routes_ocr` and `routes_pdf_vision` (or document exact lines in report if F5 is not merged).

### Explicit "Do not edit" files
- `Cargo.lock` / `Cargo.toml` **until the owner explicitly approves the exact dependencies in Gate B**.
- `crates/research-tools/tests/golden/tool_defs.txt` and `tool_profiles.txt` (zero tool definitions added or changed).
- `crates/research-tools/src/ingest.rs` (590 lines — put 64 MiB scan ingest and OCR receipt helper in `ingest_ocr.rs`), `pdf_match.rs` (589 lines), `library_schema.rs` (594 lines).
- `crates/desktop-server/src/routes_tools.rs` (610 lines — over 600; 64 MiB scan upload goes through `routes_ocr.rs`, while general upload lifting waits for S19's split of `routes_tools.rs`).
- `crates/desktop-server/src/http_srv.rs` (702 lines), `routes_session.rs` (616 lines), `assets/index.html` (768 lines), `assets/app.js` (932 lines), `crates/agent/src/resume.rs`, `turn_runner.rs`.
- `crates/policy/` and `crates/ledger/` (keep free of `tokio`, `rmcp`, `windows-sys`; `paper_text` already exists in `ledger`).
- Note for integrator (from `reports/f14a-pdf-core.md` section 6): `crates/desktop-server/tests/pdf_preview_browser.py:52-58` still assumes pre-F14a `--- [Page N] ---` headers and `.splitlines()` splitting on `\x0c`; fix separately in F14a/UI integration, not inside F14b/c.

---

## 7. Tests through the entry point

All CI tests use strictly self-authored synthetic PDFs / page images (zero third-party paper text/PDFs, zero live network or model calls):

1. **Set P6 synthetic OCR tests (`pdf_ocr_pipeline.rs`, `pdf_ocr_tests.rs`):**
   - Generate a self-authored PDF containing a `TextLayer` page, a `ScanTextLayer` page, and a `PageBasis::None` image-only page; run F14b against a deterministic fake `OcrEngine`. `RULES.md` forbids adding `#[ignore]` tests and forbids real external binary or network execution in CI; all CI tests must run and pass deterministically with the fake engine.
   - Assert: only `PageBasis::None` pages are sent to OCR; resulting reading file uses `\x0c` page breaks; `PageMap::page_of(line)` maps every sampled line to the exact physical page (`wrong page link == 0`); `PageBasis::Ocr.is_scan() == true` (never reported as `text_layer`); receipt pins `ocr_engine` and `ocr_models` SHA-256; re-running on identical bytes + engine + model produces byte-identical reading text.
   - **Existing receipt immutability:** a pre-F14b `source.ingested` receipt still verifies unchanged after F14b lands.
   - **SHA-256 model-pinning security test (`pdf_ocr_models.rs`):** tampered `tessdata` download fails hash verification and is deleted before install; unpinned language id is rejected.
   - **Sandbox containment test (`ocr_helper_tests.rs`):** verify `WorkerLaunch` config requests `AppContainerSandbox` (`network_restricted: true`, `filesystem_restricted: true`), stages only the single page image + verified `tessdata`, rejects path traversal, and kills the helper on timeout.
2. **Honesty & quote-chip tests (`ocr_routes.rs`, `ocr_ui.test.mjs`, `pdf_vision_tests.rs`):**
   - Quote matched in `PageBasis::Ocr` text is accepted by `quote_core`, displayed by `quote_chip.js` as `"matched in OCR text, p. N"` with `"OCR"` tag and `openPageImage: true`, never passed to KaTeX/math rendering, and opens `GET .../page-image`.
   - **Solve/Explore OCR flags:** Solve `solve_status` tags a Cited step from an OCR page with `"OCR source"`.
   - **F14c non-evidence invariant (`pdf_vision_tests.rs`):**
     - Sending pages `[2..=3]` writes `context.supplied` with `page_ranges: [{start_page: 2, end_page: 3, ...}]`.
     - `lines_sent(...)` returns **empty** `SentLines` for those pages; `quote_core::accept_quote` **rejects** any attempt to cite a model transcription as a matched quote (`LinesNotSent` / `VerificationFailed`).
     - Transcription record always carries `"transcription (model), unconfirmed"` and cannot be converted into `EvidenceQuotePayload` / `QuoteRef`.
3. **Multimodal transport tests (`crates/agent/src/vision_tests.rs`, `responses_tests.rs`):**
   - Request-body golden assertions for Anthropic (`wire.rs`), OpenAI/Ollama (`openai.rs`), Gemini (`gemini.rs`), and ChatGPT Responses (`responses.rs`) against a scripted/loopback mock transport.
   - Non-vision model fails closed with `TransportErrorKind::Config` (zero HTTP requests sent).
   - Oversized image or disallowed MIME type (`image/svg+xml`, `text/html`) is rejected before serialization.
4. **End-to-end 64 MiB local-scan & UI XSS tests:**
   - Upload/ingest a >32 MiB, <=64 MiB synthetic scan PDF through `routes_ocr.rs` + `ingest_ocr.rs` and serve it back via `original_pdf::read`; assert >64 MiB is rejected cleanly.
   - XSS payloads in model transcription text and language metadata produce zero `innerHTML` writes in `formula_transcription.js` and `ocr_settings.js`.

---

## 8. Document conflicts and resolutions

| # | Conflict | Exact sources | Resolution (`00-overview.md` wins) |
|---|---|---|---|
| 1 | `pdf_pages::PageBasis` has 3 variants vs `library_schema::PageBasis` has 4 variants (`Ocr`) | `crates/research-tools/src/pdf_pages.rs:41` vs `crates/research-tools/src/library_ext_types.rs:76` (re-exported at `library_schema.rs:9`) vs `00-overview.md` 2.12 | Add `PageBasis::Ocr` to `pdf_pages::PageBasis` (or unify on one shared enum) with `is_scan() == true`, `needs_ocr() == false`. |
| 2 | `routes_tools.rs:21,54` (10 MiB, 610 lines), `original_pdf.rs:32` (16 MiB), and `ingest.rs:24,114,148` (32 MiB, 590 lines) vs `pdf_pages.rs:36` (`MAX_USER_PDF_BYTES = 64 MiB`) | `routes_tools.rs:21,54`, `original_pdf.rs:32`, `ingest.rs:24,114,148` vs `00-overview.md` 2.12 ("16 MiB is too small for scans") & `reports/f14a-pdf-core.md` §6 | Lift `original_pdf.rs:32` to `MAX_USER_PDF_BYTES` and route user-supplied local-scan upload/ingest end-to-end through `routes_ocr.rs` + `ingest_ocr.rs` (`PdfExtractOptions::user_supplied()`, `MAX_USER_PDF_BYTES`, confirmed after P6 measurement) without growing 590-line `ingest.rs` or touching 610-line `routes_tools.rs` (general uploads wait for S19's split of `routes_tools.rs`). |
| 3 | `context.supplied` is line-only with `deny_unknown_fields` vs page-range entries needed for F14c | `crates/research-tools/src/lines_sent.rs:33–49` vs `00-overview.md` 2.12 honesty rule 3 ("a range may name pages of a file") | Extend `ContextSuppliedPayload` additively with optional `page_ranges` (old line-only events remain valid); `SentLines::covers` remains strictly line-only. |
| 4 | `evidence.quote` OCR indicator: computed from receipt vs stored field | `explore.md:212` (view computes page & `"OCR"` from source receipt) vs `reports/s2-solve-model.md` §8 #4 (`solve_state.rs` checks `ocr: true` / `scanned: true` / `source_kind: "ocr"`) | Derive truth from pinned `source.ingested.pdf_pages[p].basis` (`00-overview.md` 2.12) AND emit `"ocr": true` when writing `evidence.quote` from scanned/OCR pages so both views and `solve_state.rs` agree. |
| 5 | Unverified Tesseract/renderer dependencies vs `RULES.md` frozen `Cargo.lock` | `00-overview.md` 2.12 & 8.3 #4 vs `agents/RULES.md` ("No new dependency and no change to `Cargo.lock`") | Explicit owner measurement + dependency approval gate (Section 2) MUST clear before F14b or F14c is dispatched. |

---

## 9. Out of scope and hand-back

- **Out of scope:** PDF text extraction core and first-page metadata (F14a, merged); PDF skeleton `pdf-1` (V17); Solve "check a derivation" from a PDF (TeX-only first, `00-overview.md` 2.12); Docling/GROBID add-ons; user OCR text-line correction overlays (F16a, `00-overview.md` 2.14).
- **Hand-back:** Run `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`, `cargo test -p research-tools --test tool_def_golden`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --no-fail-fast`, and `node --check` / `node --test` on touched JS files. Record every owner-approved dependency, Windows-only `AppContainerSandbox` coverage limits on Linux, and P6 fixture metrics in `docs/design/modes-2026-11/reports/f14bc-ocr-vision.md`, and push only to `gemini/modes/f14bc-ocr-vision` under `agents/RULES.md`.
