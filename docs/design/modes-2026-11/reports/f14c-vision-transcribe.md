# Report: `f14c-vision-transcribe` — Opt-In Model Transcription of Equation Pages ("model-transcribed") (F14c)

- **Branch:** `work/f14c-vision-transcribe` -> `origin/gemini/modes/f14c-vision-transcribe`
- **Base commit (`origin/gemini/work/fixed`):** `6491394e28ef390f5f40e5fe56ac3aec8c1a97d7` (merged `--no-ff` at `f3e0fd7` on top of prior Checkpoint 1 head `2d615f520e008d55c2ca96fcd73cc308a98ff84b`, which branched from `e9d8342ee051ca255fbec409a8d7ea7b563a526a`)
- **`Cargo.lock` / `Cargo.toml` / golden tests:** untouched (`0` external dependencies added; `tool_def_golden` and `tool_profile_golden` green)

---

## 1. Summary by Package (Files & Line Counts)

Every touched source file is `<= 600` lines (`python3 scripts/check_file_length.py --base origin/gemini/work/fixed` exits `0`; max touched file is `pdf_vision.rs` at `583` lines).

### Checkpoint 1 — Multimodal `ContentBlock::Image`, Provider Codecs & Fail-Closed Vision Gate (`crates/agent`)
Commits: `5a30c2e`, `32feed4`, `2d615f5`, `0e8f3d3` (audit hardening).
- `crates/agent/src/responses.rs` (`559`L) & `crates/agent/src/responses_tests.rs` (`44`L): zero-behaviour-change inline test split, followed by Responses API `type: "input_image"` encoding + fail-closed vision validation.
- `crates/agent/src/runner.rs` (`533`L) & `crates/agent/src/runner_blocks.rs` (`107`L): zero-behaviour-change pre-split of block/validation helpers so adding `ContentBlock::Image` keeps `runner.rs <= 600` lines.
- `crates/agent/src/transport.rs` (`376`L): added `ContentBlock::Image { media_type: String, data_base64: String }`.
- `crates/agent/src/vision.rs` (`393`L) & `crates/agent/src/vision_tests.rs` (`342`L):
  - Pure-Rust RFC 4648 base64 encoder & strict decoder (`encode_base64`, `decode_base64_strict`), rejecting whitespace, invalid padding, non-canonical trailing bits, and oversized payloads before allocation.
  - Strict MIME allowlist (`image/png`, `image/jpeg`, `image/webp`), magic-byte header verification, per-image 5 MiB cap (`MAX_IMAGE_BYTES`), per-request 8-image / 16 MiB caps, and user-role-only enforcement.
  - Explicit fail-closed `(ProviderKind, model)` vision-capability gate (`supports_vision`, `validate_request_images`) returning `TransportErrorKind::Config` for unknown/text/audio/realtime/tts/transcribe/search models (including `gpt-4o-audio-*`, `gpt-4o-realtime-*`, `gpt-4o-mini-tts`, `gpt-4o-transcribe`, `gpt-4o-search-*`, `gpt-4-turbo-preview`, `gpt-4-turbo-0125-preview`, `o1-mini`, `o1-preview`, `o3-mini`, `gemma3:1b`, `qwen2.5-coder:7b`, `claude-2.1`, `gemini-1.0-pro`).
- `crates/agent/src/wire.rs` (`211`L), `crates/agent/src/openai.rs` (`511`L), `crates/agent/src/gemini.rs` (`409`L), `crates/agent/src/lib.rs` (`103`L): wired Anthropic (`type: "image"`), OpenAI/Ollama (`image_url` data URI), Gemini (`inlineData`), and ChatGPT Responses (`input_image`) wire codecs with fail-closed validation.

### Checkpoint 2.1 — `context.supplied` Page Ranges (`crates/research-tools`)
Commit: `d28baaa`.
- `crates/research-tools/src/lines_sent.rs` (`551`L) & `crates/research-tools/src/lines_sent_tests.rs` (`560`L):
  - Added `SuppliedPageRange { source_id, raw_blob_sha256, start_page, end_page, crop: Option<[u32; 4]> }` (`#[serde(deny_unknown_fields)]`) and optional `#[serde(default, skip_serializing_if = "Vec::is_empty")] pub page_ranges: Vec<SuppliedPageRange>` on `ContextSuppliedPayload`, plus `ContextSuppliedPayload::new_with_pages`.
  - Preserved 100% backward compatibility for existing line-only `context.supplied` JSON (deserializes with empty `page_ranges` and omits `page_ranges` on re-serialization).
  - Guaranteed `SentLines::covers` receives **zero** credit from `page_ranges` (only `ranges` is read by `credit_context_supplied`).

### Checkpoint 2.2 — Opt-In Schema & Text-Layer Agreement Gate (`crates/research-tools`)
Commits: `8c38081`, `ce14095`, `d9e10e1`.
- `crates/research-tools/src/pdf_vision.rs` (`583`L), `crates/research-tools/src/pdf_vision_tests.rs` (`459`L), `crates/research-tools/src/lib.rs` (`353`L):
  - Constants (Owner Decision 2026-11-26): `DEFAULT_EQUATION_TRANSCRIPTION_ENABLED = false`, `MODEL_TRANSCRIBED_LABEL = "model-transcribed"`, `EVENT_PDF_TRANSCRIPTION = "pdf.transcription"`.
  - `is_equation_transcription_opted_in(setting_enabled, request_opt_in)`: off by default; explicit per-request `Some(false)` overrides saved opt-in.
  - `FormulaTranscription { source_id, raw_blob_sha256, page, crop, model, latex_draft, label, agreed_with_text_layer: bool }` (`#[serde(deny_unknown_fields)]`), unconditionally stamping and validating `label == "model-transcribed"`.
  - `check_text_layer_agreement` (read-only advisory) and `verify_transcription_against_text_layer`:
    - Verifies pinned `source.ingested` PDF receipt, `raw_blob` SHA-256, regular-file 16 MiB bounded read, and `files_sha256` integrity.
    - Accepts only `TextLayer`, `ScanTextLayer`, or `Ocr` (refuses `PageBasis::None` / empty text layer; conservatively classifies pre-F14a PDF receipts lacking scan metadata as `ScanTextLayer`).
    - Matches via `pdf_match::match_pdf_quote` on the pinned page's line slice with `<= 300`-char (`MAX_DISPLAY_QUOTE_CHARS`), `>= 16`-non-ws / full-line parity with `quote_core`, token-boundary checks, and page-level uniqueness (fails closed on duplicate occurrences).
    - Delegates final acceptance to `quote_core::accept_quote` (requiring text lines supplied in `context.supplied` `ranges` / read tools — never `page_ranges`) and verifies the final `QuoteRef` remains on `transcription.page`. Never mints `QuoteRef`/`evidence.quote` directly and never alters reading text.

### Checkpoint 2.3 — Opt-In HTTP Route (`crates/desktop-server`)
Commits: `0065efa`, `271f48e`.
- `crates/desktop-server/src/routes_pdf_vision.rs` (`557`L), `crates/desktop-server/tests/pdf_vision_routes.rs` (`516`L), `crates/desktop-server/src/api_dispatch.rs` (`488`L, 1-line arm), `crates/desktop-server/src/lib.rs` (`254`L, 1-line module export):
  - `POST /api/sessions/{id}/sources/{source}/transcribe-equations` behind header-token `AuthGuard::authorize_api` (not added to `auth.rs` cookie exceptions).
  - Refuses with `403` unless explicit per-request `opt_in: true` or persisted `<data_dir>/pdf_vision_settings.json` (`equation_transcription_enabled: true`, saved atomically under mutex with `0600` mode on Unix) is enabled; explicit `opt_in: false` always refuses.
  - Accepts caller-supplied PNG/JPEG/WebP base64 crop OR decoupled `PageCropProvider` trait (F14b merge seam; returns `crop_image_media_type` + `crop_image_base64` on 200 when rendered server-side; loads `raw_blob` lazily only when `PageCropProvider` is invoked).
  - Enforces fail-closed `page_count` bounds, `(ProviderKind, model)` vision capability check (`400`), clamped F9 `run_budget` across BOTH `RunScope::All` (`DEFAULT_VISION_BUDGET_CAP = 64`) and optional `RunScope::RunId` (`429`), cooperative cancel check before writing `context.supplied`, and fail-closed `StopReason` checks (`StopReason::Refusal` -> `422`, `StopReason::MaxTokens`/other -> `502`, never persisting refusal/truncated text).
  - Records `context.supplied` `page_ranges` (zero `SentLines` credit) and `pdf.transcription` labelled `"model-transcribed"`, returning `quotable: false`, `requires_text_layer_agreement: true`, and updated F9 `budget`.

### Checkpoint 2.4 — Side-by-Side Crop & Draft UI (`crates/desktop-server/assets`)
Commits: `57de222`, `9e5e5e4`.
- `crates/desktop-server/assets/formula_transcription.js` (`427`L), `crates/desktop-server/tests/formula_transcription.test.mjs` (`395`L), `crates/desktop-server/src/assets.rs` (`498`L, 4-line static registration):
  - Strictly DOM-based (`createElement` / `textContent` / `setAttribute`; zero `innerHTML`, `outerHTML`, `insertAdjacentHTML`).
  - Exports `DEFAULT_EQUATION_TRANSCRIPTION_ENABLED = false`, `MODEL_TRANSCRIBED_LABEL = 'model-transcribed'`, `safeCropDataUrl` (PNG/JPEG/WebP + strict base64 only), `validateCropRect` (strict finite non-negative integer `[x, y, w >= 1, h >= 1]`), `transcribeEquationsUrl`, `requestEquationTranscription`, `renderFormulaTranscriptionCard`, and `mountFormulaTranscriptionPanel` (off-by-default opt-in checkbox, wired button `'click'` handler, custom-document CSRF resolution, server-returned `crop_image_*` fallback, and stale session/source guards).

---

## 2. Independent Audit Findings & Resolutions

An independent adversarial audit (`research` / `pro` subagent) audited Checkpoint 1 + Checkpoint 2 and reported 4 BLOCKERs and 4 MEDIUMs. Every actionable code finding was fixed in dedicated commits:
1. **BLOCKER 1 (`formula_transcription.js` dead click button):** wired `transcribeBtn`'s `'click'` handler (`9e5e5e4`) with `getParams`/`defaultParams` + DOM click test. Note: `canvas_sources.js` (`842`L) and `app.js` (`932`L) are both already over the 600-line ceiling (`RULES.md` forbids touching files `> 600` lines), so mounting into the reader shell is documented in Merge notes below.
2. **BLOCKER 2 (`routes_pdf_vision.rs` ignored `resp.stop_reason`):** added fail-closed `StopReason` checks (`Refusal` -> `422`, `MaxTokens`/other -> `502`) + integration tests (`271f48e`).
3. **BLOCKER 3 (F9 `run_budget` cap bypass & pre-cancel write):** clamped client `budget_cap` to `DEFAULT_VISION_BUDGET_CAP`, enforced BOTH `RunScope::All` and `RunScope::RunId`, and added pre-cancel check before `context.supplied` append (`271f48e`).
4. **BLOCKER 4 (missing report & unpushed Checkpoint 2):** resolved by this report and pushing to `origin/gemini/modes/f14c-vision-transcribe`.
5. **MEDIUM 1 (`vision.rs` allowlist false positives):** rejected `-audio`/`-realtime`/`-tts`/`-transcribe`/`-search`, text-only `gpt-4-turbo-preview`/`gpt-4-turbo-0125-preview`, and Ollama `gemma3:1b` (`0e8f3d3`).
6. **MEDIUM 2 (`pdf_vision.rs` parity, ambiguity, legacy scan basis, bounded reads):** added `is_file()` + 16 MiB `.take(...)`, conservative `ScanTextLayer` fallback for pre-F14a receipts, `<= 300`-char + `>= 16`-non-ws/full-line parity, and duplicate-match rejection (`ce14095`, `d9e10e1`).
7. **MEDIUM 3 (`routes_pdf_vision.rs` page bounds, lazy blob load, server-rendered crop, settings lock/`0600`):** fixed all four (`271f48e`).
8. **MEDIUM 4 (`formula_transcription.js` crop validation & `resolveToken`):** added `validateCropRect` and `resolveDoc` in `resolveToken` (`9e5e5e4`).

---

## 3. Deviations, Autonomous Decisions & Assumptions

- **Deviation — `runner.rs` pre-split (`32feed4`):** `runner.rs` had an exhaustive match on `ContentBlock` and was already `626` lines on `e9d8342`; Checkpoint 1 extracted block/validation helpers into `runner_blocks.rs` (`107`L) so `runner.rs` dropped to `533` lines (`<= 600`).
- **Decision — settings sidecar (`pdf_vision_settings.json`):** `state.rs` is on the Do-Not-Edit list (`599`L). Following `investigation_settings.rs`, persisted opt-in defaults to `DEFAULT_EQUATION_TRANSCRIPTION_ENABLED = false` in `<data_dir>/pdf_vision_settings.json` without editing `state.rs`.
- **Decision — F9 vision call accounting without editing `run_budget.rs`:** `routes_pdf_vision.rs` counts prior vision calls via `context.supplied` events carrying non-empty `page_ranges` (1 per dispatched vision request, so failed/refused attempts are still charged) on top of `run_budget::count_model_calls`, avoiding any edit to `run_budget.rs` or polluting `resume.rs`'s main-loop `model.request` state machine.
- **Decision — standalone UI module (`formula_transcription.js`):** `canvas_sources.js` (`842`L) and `app.js` (`932`L) exceed 600 lines and cannot be edited under `RULES.md`. Like `writeup_btn.js` (F15), `formula_transcription.js` is self-contained, registered at `/assets/formula_transcription.js`, and ready to mount once those files are split.
- **Assumption — provider vision model-family allowlists (`vision.rs:212-302`):** model-family prefix/tag matching (`claude-3/4/sonnet/opus/haiku-4`, `gpt-4o/4.1/4.5/5/6/o1/o3/o4-mini`, `gemini-1.5/2/3/4/pro-vision/exp`, Ollama vision families) is based on published vendor model naming; unknown model IDs fail closed by design.

---

## 4. Merge Notes

1. **F14b (`pdf_render::PdfPageRenderer`) 1-line hook:** In `crates/desktop-server/src/routes_pdf_vision.rs`, implement `PageCropProvider` over F14b's `pdf_render::PdfPageRenderer` and change `handle_transcribe_equations` (line 324) from `handle_transcribe_with_provider(state, session, source, body, None)` to `handle_transcribe_with_provider(state, session, source, body, Some(&renderer))`. Zero overlapping files with `f14b-ocr`.
2. **Existing files edited (small, local seams):**
   - `crates/agent/src/lib.rs`: `+1` line (`pub mod vision;`)
   - `crates/agent/src/{transport.rs,wire.rs,openai.rs,gemini.rs,responses.rs,runner.rs}`: `ContentBlock::Image` + pre-splits (`responses_tests.rs`, `runner_blocks.rs`)
   - `crates/research-tools/src/lib.rs`: `+1` line (`pub mod pdf_vision;`)
   - `crates/research-tools/src/lines_sent.rs`: additive `SuppliedPageRange` + optional `page_ranges` field
   - `crates/desktop-server/src/lib.rs`: `+1` line (`pub mod routes_pdf_vision;`)
   - `crates/desktop-server/src/api_dispatch.rs`: `+1` line (`POST /api/sessions/{sid}/sources/{source}/transcribe-equations`)
   - `crates/desktop-server/src/assets.rs`: `+4` lines (`"/assets/formula_transcription.js"`)
3. **Optional coordinator follow-up hooks (out of F14c scope):**
   - `crates/research-tools/src/run_budget.rs`: if `solve_gates::detect_g4_budget` / `solve_done` should also charge vision calls against Solve budgets, add `count_vision_calls` (or a vision counter) inside `count_model_calls_breakdown`.
   - `crates/desktop-server/src/api_dispatch.rs`: add `GET`/`PUT /api/settings/pdf-vision` wired to `load_pdf_vision_settings` / `save_pdf_vision_settings` when Settings UI exposes the persistent opt-in toggle.
   - `crates/desktop-server/assets/canvas_sources.js`: after splitting below 600 lines, import `mountFormulaTranscriptionPanel` from `./formula_transcription.js` and mount inside the PDF source reader pane.

---

## 5. Verification Results (Non-Root Linux, `uid=1001(antigravity)`, `RE_SEARCH_PYTHON` unset)

| Command | Result |
| --- | --- |
| `id` | `uid=1001(antigravity)` (non-root) |
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | PASS (`No touched source file is over 600 lines.`) |
| `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` | PASS (0 errors across all JS assets) |
| `node --test crates/desktop-server/tests/*.test.mjs` | PASS (**237 passed, 0 failed, 0 skipped**; includes 6/6 in `formula_transcription.test.mjs`) |
| `unset RE_SEARCH_PYTHON && cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` | PASS (0 warnings, exit 0) |
| `unset RE_SEARCH_PYTHON && cargo test --workspace --locked --no-fail-fast` | PASS (**1,225 passed, 0 failed, 2 pre-existing ignored**) |

- **Pre-existing ignored tests (not added or touched by F14c):** `file_sheet_from_json` (`live tier: needs EVAL_SHEET_JSON`) and `live_manifests_validate` (`live tier: needs EVAL_LIVE_MANIFEST_DIR`) in `crates/agent/tests/eval_harness.rs`. Zero tests skipped or ignored in any F14c code.
- **Not compiled/run on Linux:** `cfg(windows)` code paths (e.g. `AppContainerSandbox`, Windows launcher/COM helpers), real browser rendering of `formula_transcription.js`, and live network calls against vendor vision endpoints (all provider wire formats and HTTP routes were verified offline via unit tests and loopback `MockHttp`).
- **Untouched bugs noticed but not fixed (Do-Not-Edit files):**
  - `crates/desktop-server/src/http_srv.rs:358-372`: status reason-phrase table omits `429` (`Too Many Requests`), `422` (`Unprocessable Entity`), and `502` (`Bad Gateway`), falling back to `"Internal Server Error"` as the HTTP reason phrase while still returning the accurate numeric status codes `429`/`422`/`502`. (`http_srv.rs` is on the Do-Not-Edit list.)
  - `crates/desktop-server/src/evidence.rs:744`: pre-existing `clippy::manual_unwrap_or` documented in `RULES.md` (file `> 600` lines, untouched).
