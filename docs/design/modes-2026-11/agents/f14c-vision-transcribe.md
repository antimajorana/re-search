# Brief f14c-vision-transcribe — Opt-In Model Transcription of Equation Pages ("model-transcribed") (F14c)

**Status:** Unblocked on `gemini/work/fixed` (`F4`, `F9`, `F14a` are merged, and **Owner Decision 2026-11-26** explicitly approved F14c with strict honesty constraints). Zero `Cargo.lock` edits!
**Owner Decision 2026-11-26 (NON-NEGOTIABLE RULES):**
1. **Off by default:** `DEFAULT_EQUATION_TRANSCRIPTION_ENABLED: bool = false`. User must explicitly opt in before equation-page images are sent to their own vision model.
2. **Labelled `"model-transcribed"`:** `pub const MODEL_TRANSCRIBED_LABEL: &str = "model-transcribed";` (on both stored records and UI cards beside the page crop).
3. **Never used as a source for matched quotes unless the text layer agrees:** a `"model-transcribed"` transcription can NEVER mint a `QuoteRef`, NEVER produce an `evidence.quote` event, NEVER credit `SentLines::covers` from `page_ranges`, and NEVER modify `source.ingested` reading text; any quote cited from that page MUST independently match via `quote_core::accept_quote` / `pdf_match` against the **pinned text layer** (`TextLayer`, `ScanTextLayer`, or `Ocr`) with text lines supplied in `context.supplied`!

---

## 1. Verified Interfaces & Disjointness on `gemini/work/fixed`

- **`crates/agent/src/responses.rs` (`587`L):** only `13` lines below the `600`-line limit because inline `#[cfg(test)] mod tests` sits at lines `541–587`. **Commit 1 MUST pre-split `responses.rs` lines `541–587` into `crates/agent/src/responses_tests.rs` with zero behaviour change!**
- **`crates/agent/src/{transport.rs (372L), wire.rs (207L), gemini.rs (405L), openai.rs (492L)}`:** `ContentBlock` currently has NO `Image` variant. Exclusive to `f14c-vision-transcribe` across all parallel lanes!
- **`crates/research-tools/src/lines_sent.rs` (`503`L):** `ContextSuppliedPayload` currently uses `#[serde(deny_unknown_fields)]` and holds only `ranges: Vec<SuppliedRange>`. Exclusive to `f14c-vision-transcribe` (`f14b-ocr` does NOT touch `lines_sent.rs`).
- **Decoupled from `f14b-ocr` (`pdf_render.rs`):** accept caller-supplied or callback/trait-supplied PNG/JPEG/WebP page-crop bytes in `routes_pdf_vision.rs` (plus list the exact 1-line hook to `pdf_render::PdfPageRenderer` in Merge notes) so `f14b-ocr` and `f14c-vision-transcribe` touch ZERO overlapping files!

---

## 2. Checkpoints — Push to `gemini/modes/f14c-vision-transcribe` After Each!

### Checkpoint 1 — Pre-Split `responses.rs` + Multimodal `ContentBlock::Image` (`crates/agent`)
1. **Commit 1 (zero behaviour change):** extract `crates/agent/src/responses.rs:541-587` into `crates/agent/src/responses_tests.rs` (`responses.rs` drops from `587` to ~`540` lines).
2. **Commit 2 (`vision.rs`, `vision_tests.rs`, `transport.rs`, `wire.rs`, `openai.rs`, `gemini.rs`, `responses.rs`, `agent/src/lib.rs`):**
   - Add `ContentBlock::Image { media_type: String, data_base64: String }` (`image/png`, `image/jpeg`, `image/webp` only; validated base64 + size cap; ~40-line pure-Rust base64 encoder in `vision.rs` with zero `Cargo.lock` change).
   - Add provider wire encoders (`wire.rs` Anthropic `type: "image"`, `openai.rs` OpenAI/Ollama `image_url` data URI, `gemini.rs` `inlineData`, `responses.rs` `input_image`) AND an explicit `(ProviderKind, model)` vision-capability check that **fails closed** (`TransportErrorKind::Config`) when the selected model is not vision-capable (never silently dropping the image!).
   - Test all 4 providers' serialized request JSON and fail-closed non-vision rejection in `vision_tests.rs`.
3. Push Checkpoint 1 to `gemini/modes/f14c-vision-transcribe`.

### Checkpoint 2 — `context.supplied` Page Ranges, Honesty Gate, Routes & UI
1. **Extend `crates/research-tools/src/lines_sent.rs` (`503`L -> ~`545`L, staying `<= 600`L!):**
   - Add `SuppliedPageRange { source_id, raw_blob_sha256, start_page, end_page, crop: Option<[u32; 4]> }` and optional `#[serde(default, skip_serializing_if = "Vec::is_empty")] pub page_ranges: Vec<SuppliedPageRange>` on `ContextSuppliedPayload`. Keep existing line-only `context.supplied` JSON 100% backward-compatible and guarantee **`SentLines::covers` receives zero credit from `page_ranges`**.
2. **Create `crates/research-tools/src/pdf_vision.rs` (+ `pdf_vision_tests.rs`, exported in `research-tools/src/lib.rs`):**
   - Define `DEFAULT_EQUATION_TRANSCRIPTION_ENABLED = false`, `MODEL_TRANSCRIBED_LABEL = "model-transcribed"`, `EVENT_PDF_TRANSCRIPTION = "pdf.transcription"`, and `FormulaTranscription { source_id, raw_blob_sha256, page, crop, model, latex_draft, label, agreed_with_text_layer: bool }`.
   - Enforce the **text-layer agreement gate** (`verify_transcription_against_text_layer`): a `"model-transcribed"` draft NEVER produces a `QuoteRef` / `evidence.quote` on its own; only if the pinned `source.ingested` text layer (`TextLayer`, `ScanTextLayer`, or `Ocr`) agrees and its lines were supplied in `context.supplied` can `quote_core::accept_quote` accept a matched quote from that page (tested explicitly for both disagreeing/missing text layer -> refused, and agreeing text layer -> matched).
3. **Create `crates/desktop-server/src/routes_pdf_vision.rs` (+ `tests/pdf_vision_routes.rs`):**
   - `POST /api/sessions/{id}/sources/{source}/transcribe-equations` (registered via 1-line arm in `api_dispatch.rs`, exported in `desktop-server/src/lib.rs`): refuses unless explicit per-request/setting opt-in is enabled (off by default!), validates PNG/JPEG/WebP crop image + session `source.ingested`, checks F9 `run_budget`, records `context.supplied` `page_ranges`, calls vision transport, and records `pdf.transcription` labelled `"model-transcribed"`.
4. **Create `crates/desktop-server/assets/formula_transcription.js` (+ `tests/formula_transcription.test.mjs`):**
   - Renders the page crop beside `"model-transcribed"` LaTeX draft (zero `innerHTML`, XSS-escaped, visibly marked off-by-default & non-quotable unless text layer agrees) and registers `/assets/formula_transcription.js` in `assets.rs`.
5. Run full mandatory checks (`check_file_length.py`, `node --check`, `node --test`, non-root Linux workspace Clippy & `cargo test --workspace --locked`), write `docs/design/modes-2026-11/reports/f14c-vision-transcribe.md`, and push to `gemini/modes/f14c-vision-transcribe`.

---

## 3. Do-Not-Edit List & Verification (`RULES.md`)

- **Do NOT edit:** `Cargo.lock`, `Cargo.toml`, `crates/research-tools/src/{pdf_pages,pdf_ocr*,pdf_render,ingest*,pdf_match,http_client,net_rate,lit_*,solve_*,skeleton/*}.rs`, `crates/worker/*`, `crates/desktop-server/src/{routes_ocr,original_pdf,lit_settings,routes_worker_activity,solve*,state,http_srv,routes_session}.rs`, `tools/golden/*.txt`.
- Run on Linux **as non-root (`id -u != 0`) with `RE_SEARCH_PYTHON` unset**:
  ```bash
  python3 scripts/check_file_length.py --base origin/gemini/work/fixed
  for f in crates/desktop-server/assets/*.js; do node --check "$f"; done
  node --test crates/desktop-server/tests/*.test.mjs
  unset RE_SEARCH_PYTHON
  cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or
  cargo test --workspace --locked --no-fail-fast
  ```
- Push **only** via `git push origin HEAD:refs/heads/gemini/modes/f14c-vision-transcribe`. Never squash, never open/merge PRs, and end with `PUSHED`.
