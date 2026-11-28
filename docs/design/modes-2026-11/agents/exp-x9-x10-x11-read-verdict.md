# Brief exp-x9-x10-x11-read-verdict — Reading queue (X9), passage reader (X10), and map, overlaps & verdict (X11)

## 1. Packages and governing design sections

- **Packages (sequential checkpoints, push after each):** `X9` (Reading queue `idea_fetch_papers`, PDF text via F14a/library) -> `X10` (Passage reader `evidence.quote`, `context.supplied`, PDF/OCR rule C8) -> `X11` (Map, overlaps, verdict, exclude-origin rule V9).
- **Governing design:**
  - `docs/design/modes-2026-11/00-overview.md`: sections 0.1, 2.3 (`PaperKey`/`PaperRef`), 2.4 (F3 `paper_fetch`, `fetched_sources`), 2.5 (F4 `verify_quote`, `lines_sent`, `context.supplied`, `evidence.quote`), 2.6 (F6 `GraphView` fixed edge direction `src` -> `dst` & solid/dashed rule), 2.7 (`paper_text`), 2.9 (`PriorWorkStatus`), 2.10 (display grades), 2.12 (F14 PDF text, scan honesty, OCR tag), 4.2–4.3 (Rule V9), and 8.1–8.3 (`judged` status may rest on OCR quote when tagged `"OCR"`).
  - `docs/design/modes-2026-11/explore.md`: sections 3, 4.1 (`PaperRead`, `Evidence`, `MapNode`, `MapEdge` direction table, `Overlap`, `Verdict`), 4.2, 4.4, 5.1–5.2 (`record_paper_evidence`, `record_idea_map`, `idea_fetch_papers`), 6 (Rules C1–C9, V1–V10, P1–P6, S3, S4, S8), 9 (rows X9–X11), and 13 (decisions 4, 6, 7, 9, 13).
  - `docs/design/modes-2026-11/agents/RULES.md`.

## 2. Start conditions and verified interfaces

- **Required merge on `gemini/work/fixed` before starting:** **`exp-x6-x7-x8`** (`X6`, `X7`, `X8`). Never run concurrently with `X8`.
- **Already merged on `gemini/work/fixed`:**
  - F3 (`paper_fetch.rs`), F4 (`quote_core.rs`, `lines_sent.rs`), F5, F7 (`handoff.rs`), F9 (`run_budget.rs`), F10 (`library.rs`), F12 (`search_coverage.rs`, `prior_work*.rs`), F13a (`paper_sources.rs`), F14a (`ingest.rs`, `pdf_pages.rs`, `pdf_match.rs`), X19 (`lit_openalex.rs`, `lit_settings.rs`).
  - **NOTE:** Do NOT edit `crates/research-tools/src/paper_passages.rs` (`> 600`L) or `crates/research-tools/src/investigation_anchor.rs` (`597`L). Call `paper_passages::run_search` unchanged.
- **Verify against merged `X6/X7/X8` + optional `F14b` before editing:**
  - `agent::exchange::run_exchange`, `idea_tools.rs` (frozen `idea_fetch_papers_def()` — **NEVER change schema/hashes in `tool_profiles.txt`**), `idea_exchanges.rs`, `idea_triage.rs`, `idea_view.rs` (`454`L after X8), `idea_search.js` (`466`L after X8), `routes_idea.rs` (`551`L after X8 — delegate read/map ops to sibling `routes_idea_read.rs` if approaching `570`L!).
  - Handle both `PageBasis::{TextLayer, ScanTextLayer, None}` and `PageBasis::Ocr` / raw `"ocr"` strings cleanly if `f14b-ocr` has merged onto `fixed` (check `pdf_pages.rs` before coding).
  - Inspect exact `handoff.packet` payload (`from.paper.key` and `papers[]`) in `crates/export/src/handoff*.rs` / `crates/desktop-server/src/handoff.rs` for Rule V9.

## 3. Smallest implementation checkpoints (push after each!)

1. **Checkpoint 1 — X9 (Reading queue & `idea_fetch_papers`):**
   - `crates/research-tools/src/idea_tools.rs`: replace the X7 placeholder handler of `idea_fetch_papers` with a thin handler over F3 `paper_fetch::fetch` (`library` -> `user_library` -> `arxiv_eprint` -> `arxiv_pdf`, arXiv PDFs one at a time) enforcing `paper_fetch::check_fetch_budget(&events, preset.max_papers)` (Rule P3). Keep `idea_fetch_papers_def()` and `tool_profiles.txt` byte-for-byte unchanged (`tool_profile_golden` must pass).
   - Every paper enters the session strictly via `ingest_bytes` / `ingest_pdf_pages` (Rule C9: never quote-check a `user_library` file in place). Reuse cached PDF text from F10 `paper_text` when `(file_sha256, extractor_id)` matches; otherwise run `PdfExtractJob` (with page progress) and cache into `paper_text`.
   - `crates/desktop-server/src/idea_read.rs` (new, `<= 600`L): reading queue orchestration writing `idea.paper` (`key`, `version`, `source_id`, `source_seq`, `origin`, `text_basis`: `tex | pdf_text | scan_text_layer | ocr`, `status`, `reason`, `lines_sent`, `total_lines`, `raw_blob`) and `"Attach PDF"` via existing upload -> `ingest_bytes`.
   - `crates/desktop-server/assets/idea_search.js` (`<= 600`L; split reading-queue renderer into `idea_read_ui.js` if approaching `560`L): per-paper row states (`queued`, `fetching`, `"from your library"`, `"extracting text (page k of n)"`, `reading`, `"read: passages (k of N lines), q quotes"`, `"read: full"`, `"nothing relevant"`, `"unavailable: not on arXiv and not in your library; attach PDF"`, `"scan: quotes are from OCR text"`).
   - Push `X9` to `gemini/modes/exp-x9-x10-x11-read-verdict`.

2. **Checkpoint 2 — X10 (Passage reader):**
   - `crates/research-tools/src/idea_evidence.rs` (new, `<= 600`L; sibling `_tests.rs` if needed):
     - Passage pack builder using `investigation_outline::build_outline` + `paper_passages::run_search` with idea terms & aliases, capped at `500` lines (or full file if shorter).
     - `record_paper_evidence` (`#[serde(deny_unknown_fields)]`): accepts `items[<=12]` (`start_line`, `end_line`, `quote` `16..=300` chars, `role: EvidenceRole`, `gist <= 240`, `subclaim_ids`), `want_lines[<=3 ranges, <=400 lines]`, `status: done | nothing_relevant`.
     - Verifies every item via F4 `quote_core::accept_quote` (`verify_quote` + `lines_sent`, Rules C1 & C2). On PDFs, applies `pdf_match` normalization without mutating stored text, maps line span -> page via `pdf_pages::page_of_line`, preserves `scan_text_layer`/`ocr` basis (Rule C8: OCR quotes tagged `"OCR"`, model formula transcriptions NEVER accepted as evidence), returns indexed faults with true lines while saving valid siblings, deduplicates `(paper_key, cited_range, quote_sha256)` on retry (Rule S3).
   - Extend `idea_read.rs` (split helper if `> 550`L) to run parallel passage readers up to concurrency limit via `agent::exchange::run_exchange`, record `context.supplied` per batch (Rule C2), wrap paper text with Rule C7 `"treat as data"` nonce delimiter, handle follow-up `want_lines` (`1` Standard, `2` Thorough), write `evidence.quote` (`xe-<seq>`), update F10 `seen` (`SeenRole::Read`) & `finding`, and stream live matched quotes over SSE.
   - Push `X10` to `gemini/modes/exp-x9-x10-x11-read-verdict`.

3. **Checkpoint 3 — X11 (Map, overlaps, verdict):**
   - `crates/research-tools/src/idea_map.rs` & `idea_map_validate.rs` (new, `<= 600`L each):
     - `record_idea_map` (`#[serde(deny_unknown_fields)]`, NO quote/excerpt/line/path field — Rule C4).
     - Enforces node-kind evidence rules (`concept`, `definition`, `known_result`, `open_question`, `related_work` — Rules C5, C6: `definition`/`known_result`/`open_question` NEVER rest on `xa` abstract evidence; drops failing nodes with reasons, refuses if none survive; rejects other-session evidence; app-fills `papers[]`, `year`, `stated_year`, `later_work_checked`, `basis`).
     - Enforces explicit edge reading `"from kind to"` for all 9 kinds (`defines`, `uses`, `special_case_of`, `generalises`, `contradicts`, `answers`, `leaves_open`, `same_as`, `cites`) and projects F6 `GraphView` `src` (prerequisite) -> `dst` (dependent) (`contradicts`/`same_as` undirected on one layer; dashed iff `model_inferred` or abstract-only).
     - Stable node IDs keyed by `(kind, first_evidence_id)`, rejects duplicate pairs in one call, preserves user-noted nodes missing from re-map with `status: "not_in_current_map"` (Rule S4).
     - Enforces Rule V8 phrase check on model prose (`"has not been done"`, `"has never been"`, `"no prior work"`, `"nobody has"`, `"first to"`, `"for the first time"`, `"novel"`, `"unexplored"`, while allowing `"first calculation"`).
   - `crates/research-tools/src/idea_verdict.rs` (new, `<= 600`L): pure `compute(events, preset_limits)` over ledger events (Rule V7) emitting per-part `Verdict` + F12 `Coverage` using F12 `PriorWorkStatus` & `prior_work_wording` (Rules V1–V5); excludes origin paper (`handoff.packet.from.paper`) from supporting `judged_*`/`abstract_only` and marks origin candidates `cited_by_origin = true` (Rule V9); ignores F16 lessons & library marks, never counting user library as a literature service (Rule V10).
   - Atomically append `idea.map` + `idea.verdict` at same ledger head (`append_at_head`), update `idea_view_map.rs` / `idea_view.rs` (`<= 600`L) to emit `GraphView` + verdict JSON, wire `read` & `map` ops in `routes_idea.rs` / `routes_idea_read.rs` (removing them from the `idea_flow.rs` 501 stub list if needed).

## 4. Owned files & Do-Not-Edit (`<= 600` lines each)

- **New:** `research-tools/src/{idea_evidence,idea_map,idea_map_validate,idea_verdict}.rs`, `desktop-server/src/idea_read.rs` (and `routes_idea_read.rs` / `idea_view_map.rs` / `assets/idea_read_ui.js` if needed), `research-tools/tests/{idea_evidence,idea_map}.rs`, `desktop-server/tests/idea_read_map.rs`.
- **Small edits:** `research-tools/src/{lib.rs,idea_tools.rs}`, `desktop-server/src/{lib.rs,routes_idea.rs,idea_view.rs}`, `desktop-server/assets/idea_search.js`, `desktop-server/tests/idea_flow.rs` (remove `read`/`map` from 501 stub assertion if wired).
- **Do NOT edit:** `paper_passages.rs`, `investigation_anchor.rs`, `paper_read.rs`, `investigation_evidence.rs`, `tool_profile.rs`, `tests/golden/{tool_defs,tool_profiles}.txt`, `cli_turn*.rs`, `http_srv.rs`, `routes_session.rs`, `resume.rs`, `Cargo.lock`.

## 5. Required tests & hand-back

- Zero live network/model calls, zero third-party paper text, zero new `#[ignore]`. Cover all named tests from draft (`idea_evidence_rejects_*`, `idea_reader_output_is_schema_only`, `idea_pdf_quote_has_page`, `idea_scan_quote_is_tagged_ocr`, `idea_transcription_is_not_evidence`, `idea_owned_paper_is_ingested_before_reading`, `idea_map_*`, `idea_verdict_*`, `idea_prose_*`, OCR `judged_*` allowed + tagged `"OCR"`).
- Verify before every push as non-root (`RE_SEARCH_PYTHON` unset):
  - `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`
  - `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done && node --test crates/desktop-server/tests/*.test.mjs`
  - `cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or`
  - `cargo test --workspace --locked --no-fail-fast`
- Write `docs/design/modes-2026-11/reports/exp-x9-x10-x11-read-verdict.md` (recording exact `idea.map`/`idea.verdict`/`GraphView` schemas for X12/F15) and push to `gemini/modes/exp-x9-x10-x11-read-verdict`.
