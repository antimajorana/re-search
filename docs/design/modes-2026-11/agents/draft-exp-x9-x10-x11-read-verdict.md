# Brief exp-x9-x10-x11-read-verdict — Reading queue (X9), passage reader (X10), and map, overlaps & verdict (X11)

## 1. Packages and governing design sections

- **Packages (in order):** `X9` (Reading queue `idea_fetch_papers`, PDF text from F14/library) -> `X10` (Passage reader `evidence.quote`, `context.supplied`, PDF/OCR rule C8) -> `X11` (Map, overlaps, verdict, exclude-origin rule V9).
- **Governing design:**
  - `docs/design/modes-2026-11/00-overview.md`: sections 0.1 (scope revision of 2026-11-25), 2.3 (F2 `PaperKey`/`PaperRef`), 2.4 (F3 `paper_fetch`, `fetched_sources`), 2.5 (F4 `verify_quote`, `lines_sent`, `context.supplied`, `evidence.quote`), 2.6 (F6 `GraphView` fixed edge direction `src` -> `dst` & solid/dashed rule), 2.7 (`paper_text`), 2.9 (`PriorWorkStatus`), 2.10 (display grades), 2.12 (F14 PDF text, scan honesty, OCR tag), 4.2–4.3 (exclude-origin rule V9), and 8.1–8.3 (8.3 question 3: `judged` status may rest on OCR quote when tagged `"OCR"`).
  - `docs/design/modes-2026-11/explore.md`: sections 3 (Read, Map and overlaps), 4.1 (`PaperRead`, `Evidence`, `MapNode`, `MapEdge` direction table, `Overlap`, `Verdict`), 4.2, 4.4, 5.1–5.2 (`record_paper_evidence`, `record_idea_map`, `idea_fetch_papers`), 6 (rules C1–C9, V1–V10, P1–P6, S3, S4, S8), 9 (rows X9, X10, X11), and 13 (decisions 4, 6, 7, 9, 13).
  - `docs/design/modes-2026-11/agents/RULES.md`.

## 2. Start conditions and gates

- **Required merges on `gemini/work/fixed`:**
  - Before **X9**: the entire **`exp-x6-x7-x8-search-triage` brief (`X6`, `X7`, and `X8`) complete and merged**! Never run `X9` concurrently with `X8`, because both touch `crates/desktop-server/assets/idea_search.js` (and `exp-x6-x7-x8-search-triage` also owns `idea_tools.rs` and `idea_exchanges.rs`). Base already has `F3`, `F10`, `F14a`. (If `F13a` is merged, `user_library` files flow through F10/F3 with zero network; X9 works cleanly whether or not `user_library` rows exist.)
  - Before **X10**: `X9` committed & pushed (and `X6` already merged). Base already has `F4` (`quote_core.rs`, `lines_sent.rs`) and `F14a` (`pdf_pages.rs`, `pdf_match.rs`).
  - Before **X11**: `X10` committed & pushed, `F12` (`exp-f12-coverage`), and `X8` (already merged before X9). (Verify `F7` `handoff.packet` JSON schema if merged, or match `00-overview.md` 4.1 `from.paper.key` for Rule V9.)
- **No new owner gate blocks X9–X11** (owner accepted 8.3 question 3 on OCR quotes on 2026-11-25).

## 3. Base-commit verified interfaces vs future-merge interfaces

### Verified on base commit `832eab5bf`
- `crates/research-tools/src/paper_fetch.rs` (492 lines): `fetch`, `fetch_with_gate`, `FetchWant::Source`, `FetchOutcome::{Source, NeedsFile}`, `SourceOrigin::{Library, ArxivEprint, ArxivPdf}`, `FetchedSource::ingest_into_session`, `fetched_sources`, `check_fetch_budget`.
- `crates/research-tools/src/ingest.rs` (590 lines): `ingest_bytes`, `ingest_pdf_pages` (pins `"pdf_extractor"` and `"pdf_pages"` on PDF receipts).
- `crates/research-tools/src/pdf_pages.rs` (569 lines): `PageBasis::{TextLayer, ScanTextLayer, None}`, `PageMap::{page_of, record_of_line}`, `page_of_line`, `PdfExtractJob`, `PdfExtractOptions`.
- `crates/research-tools/src/pdf_match.rs` (589 lines): `match_pdf_quote`, `quotes_match_pdf` (normalizes ligatures, soft hyphens, line-end hyphens without mutating stored text).
- `crates/research-tools/src/quote_core.rs` (472 lines): `EVIDENCE_QUOTE_EVENT`, `EvidenceRole`, `EvidenceQuotePayload`, `QuoteRef`, `verify_quote`, `accept_quote`, `quote_sha256`, `clip_quote` (300 chars).
- `crates/research-tools/src/lines_sent.rs` (465 lines): `CONTEXT_SUPPLIED_EVENT`, `ContextSuppliedPayload`, `SuppliedRange`, `lines_sent`, `lines_sent_with_ledger`.
- `crates/research-tools/src/investigation_outline.rs` (564 lines): `build_outline`, `format_outline_for_prompt`.
- `crates/research-tools/src/paper_passages.rs` (**605 lines — OVER 600, DO NOT EDIT!**): `run_search` (call unchanged).
- `crates/research-tools/src/library.rs` (532 lines): `Library::{upsert_paper_text, attach_to_session, record_seen, upsert_finding}`, `SeenRole::Read`.

### Future-merge interfaces to verify before coding
- **`X6`/`X7`/`X8`:** `agent::exchange` runner, `idea_tools.rs` (`idea_fetch_papers_def()` frozen in X7), `idea_exchanges.rs`, `idea_triage.rs`, `idea_search.js`, `routes_idea.rs`.
- **`F12`:** `PriorWorkStatus`, `search_coverage::compute`, `prior_work_wording`.
- **`F7`:** exact `handoff.packet` payload shape (`from.paper.key` and `papers[]` for Rule V9).
- **`F13a`:** `Library` reader for `user_library` files so X9 ingests owned files strictly through `ingest_bytes` (Rule C9).

## 4. Smallest implementation checkpoints

Push after each package (`X9`, `X10`, `X11`), merging `origin/gemini/work/fixed` between packages:

1. **Checkpoint 1 — X9 (Reading queue & `idea_fetch_papers`):**
   - `crates/research-tools/src/idea_tools.rs`: replace the X7 placeholder handler of `idea_fetch_papers` with the real thin handler over F3 `paper_fetch::fetch` (`library` -> `user_library` -> `arxiv_eprint` -> `arxiv_pdf`, fetching arXiv PDFs one at a time) enforcing `paper_fetch::check_fetch_budget(&events, preset.max_papers)` (Rule P3). **Do NOT alter `idea_fetch_papers_def()`** (`tool_profile_golden` must pass with `tool_profiles.txt` untouched!).
   - Every paper enters the session ONLY via `ingest_bytes` / `ingest_pdf_pages` (Rule C9: never check a quote against a `user_library` file in place). Reuse cached PDF text from F10 `paper_text` when `(file_sha256, extractor_id)` matches; otherwise run F14a `PdfExtractJob` with page progress and cache into `paper_text`.
   - `crates/desktop-server/src/idea_read.rs` (new): reading queue orchestration, writes `idea.paper` (`key`, `version`, `source_id`, `source_seq`, `origin`, `text_basis`: `tex | pdf_text | scan_text_layer | ocr`, `status`, `reason`, `lines_sent`, `total_lines`, `raw_blob`), wires `"Attach PDF"` through existing upload -> `ingest_bytes`.
   - `crates/desktop-server/assets/idea_search.js`: per-paper row states (`queued`, `fetching`, `"from your library"`, `"extracting text (page k of n)"`, `reading`, `"read: passages (k of N lines), q quotes"`, `"read: full"`, `"nothing relevant"`, `"unavailable: not on arXiv and not in your library; attach PDF"`, `"scan: quotes are from OCR text"`).
2. **Checkpoint 2 — X10 (Passage reader):**
   - `crates/research-tools/src/idea_evidence.rs` (new):
     - Passage pack builder using `investigation_outline::build_outline` + `paper_passages::run_search` (unchanged!) with idea terms and aliases, capped at 500 lines (or full file if shorter).
     - `record_paper_evidence` (`#[serde(deny_unknown_fields)]`): accepts `items[<=12]` (`start_line`, `end_line`, `quote` 16..=300 chars, `role: EvidenceRole`, `gist <= 240`, `subclaim_ids`), `want_lines[<=3 ranges, <=400 lines]`, `status: done | nothing_relevant`.
     - Verifies every item through F4 `quote_core::accept_quote` (`verify_quote` + `lines_sent`, Rules C1 & C2). For PDF sources, applies F14a `pdf_match` normalization without mutating stored text, maps line span to page via `pdf_pages::page_of_line`, and preserves `scan_text_layer`/`ocr` basis (Rule C8: OCR quotes tagged `"OCR"`, model formula transcriptions are NEVER accepted as evidence). Returns indexed faults with true text at those lines while saving valid siblings; deduplicates on `(paper_key, cited_range, quote_sha256)` on retry (Rule S3).
   - `crates/desktop-server/src/idea_read.rs`: runs parallel passage readers up to configured concurrency limit via `agent::exchange`, writes `context.supplied` for every message carrying paper lines (Rule C2), wraps paper text with "treat as data" wrapper (Rule C7), supports follow-up `want_lines` exchanges within `calls_per_paper` cap (1 in Standard, 2 in Thorough), writes `evidence.quote` (`xe-<seq>`), updates F10 `seen` (`Read`) and `finding`, streams live matched quotes over SSE.
3. **Checkpoint 3 — X11 (Map, overlaps, verdict):**
   - `crates/research-tools/src/idea_map.rs` & `idea_map_validate.rs` (new):
     - `record_idea_map` (`#[serde(deny_unknown_fields)]`, NO quote, excerpt, line, or path field — Rule C4).
     - Enforces node-kind evidence rules (`concept`, `definition`, `known_result`, `open_question`, `related_work` — Rules C5, C6: `definition`/`known_result`/`open_question` NEVER rest on `xa` abstract evidence; drops failing nodes with reasons, refuses if none survives; rejects evidence from other sessions; app-fills `papers[]`, `year`, `stated_year`, `later_work_checked`, `basis`).
     - Enforces explicit edge reading `"from kind to"` in recorder schema for every kind (`defines`, `uses`, `special_case_of`, `generalises`, `contradicts`, `answers`, `leaves_open`, `same_as`, `cites`) and provides the exact `GraphView` `src` (relied on) -> `dst` (relies on) projection table of `explore.md` 4.1 (`contradicts` and `same_as` undirected on one layer; `basis`: `evidence` -> `"evidence"`, `metadata` -> `"code"`, `model_inferred` -> `"model"`, dashed iff `model_inferred` or abstract-only).
     - Assigns sequential stable node IDs keyed by `(kind, first_evidence_id)`, rejects duplicate pairs in one call, preserves user-noted nodes missing from a re-map with `status: "not_in_current_map"` (Rule S4).
     - Enforces phrase check (Rule V8) on model prose (`difference`, statements): rejects `"has not been done"`, `"has never been"`, `"no prior work"`, `"nobody has"`, `"first to"`, `"for the first time"`, `"novel"`, `"unexplored"`, while allowing `"first calculation"`.
   - `crates/research-tools/src/idea_verdict.rs` (new):
     - Pure `compute(events, preset_limits)` over ledger events (Rule V7), producing per-part `Verdict` + F12 `Coverage` using F12 `PriorWorkStatus` and `prior_work_wording` (Rules V1–V5).
     - **Rule V9 (exclude origin paper):** when session started from `handoff.packet` with `from.paper`, skips all overlaps whose `paper_key` equals origin's `PaperKey` (origin never supports `judged_*` or `abstract_only`), and marks origin bibliography candidates `cited_by_origin = true`.
     - **Rule V10:** ignores F16 lessons and library marks; user library never counts as a service.
   - Atomically write `idea.map` and `idea.verdict` at the same ledger head (`append_at_head`) and wire `idea_view.rs` map projection to emit the `GraphView` JSON of `00-overview.md` 2.6.

## 5. Owned files

- `crates/research-tools/src/idea_tools.rs` (owned sequentially after X7 — handler body of `idea_fetch_papers` only)
- `crates/research-tools/src/idea_evidence.rs` (new, `<= 600` lines)
- `crates/research-tools/src/idea_map.rs` (new, `<= 600` lines)
- `crates/research-tools/src/idea_map_validate.rs` (new, `<= 600` lines)
- `crates/research-tools/src/idea_verdict.rs` (new, `<= 600` lines)
- `crates/desktop-server/src/idea_read.rs` (new, `<= 600` lines)
- `crates/desktop-server/src/idea_view.rs` (add `GraphView` map & verdict projection; keep `<= 600` lines)
- `crates/desktop-server/assets/idea_search.js` (owned sequentially after X8 — reading queue row states)
- Tests: `crates/research-tools/tests/idea_evidence.rs`, `crates/research-tools/tests/idea_map.rs`, `crates/desktop-server/tests/idea_read_map.rs`.

## 6. Shared registration seams and "Do not edit" list

- **Registration seams:**
  - `crates/research-tools/src/lib.rs`: `pub mod idea_evidence;`, `pub mod idea_map;`, `pub mod idea_map_validate;`, `pub mod idea_verdict;`.
  - `crates/desktop-server/src/lib.rs`: `pub mod idea_read;`.
  - `crates/desktop-server/src/routes_idea.rs`: wire `read` and `map` ops.
- **Do not edit:** `paper_passages.rs` (605 lines!), `investigation_anchor.rs` (597 lines), `paper_read.rs`, `investigation_evidence.rs`, `tool_profile.rs`, `tests/golden/tool_defs.txt`, `tests/golden/tool_profiles.txt`, `cli_turn.rs` (626), `Cargo.lock`.

## 7. Tests through the entry point

Run `cargo test -p research-tools --test idea_evidence --test idea_map --test tool_def_golden --test tool_profile_golden`, `cargo test -p desktop-server --test idea_read_map`, `node --test crates/desktop-server/tests/*.test.mjs`, `cargo clippy --workspace --all-targets -- -D warnings`, and `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`.

Required named tests (Rules C1–C9, V1–V10, S3, S4, S8 — synthetic fixtures + scripted transport + loopback mock only):
- `idea_evidence_rejects_fabricated_quote` (C1), `idea_evidence_rejects_lines_not_sent` (C2), `idea_reader_output_is_schema_only` (C7).
- `idea_pdf_quote_has_page`, `idea_scan_quote_is_tagged_ocr`, `idea_transcription_is_not_evidence` (C8 — self-authored PDFs via F14a `pdf_support` pattern).
- `idea_owned_paper_is_ingested_before_reading` (C9 — deleting/moving library source afterwards breaks no quote verification).
- `idea_map_refuses_inline_quotes` (C4), `idea_map_node_evidence_rules` & `idea_map_rejects_evidence_from_other_session` (C5), `idea_open_question_needs_author_statement` (C6), `idea_node_ids_stable_across_remap` (S4), edge-direction table test mapping every `MapEdge.kind` to `GraphView` `src`/`dst`.
- `idea_verdict_judged_requires_read_paper`, `idea_verdict_other_name_needs_term` (V1), `idea_verdict_abstract_cap`, `idea_quick_never_claims_none_in_scope` (V2), `idea_verdict_text_is_app_authored` (V5), `idea_verdict_recomputes_from_ledger` (V7), `idea_prose_rejects_absolute_phrases`, `idea_prose_allows_first_calculation` (V8), `idea_verdict_excludes_origin_paper`, `idea_origin_references_are_marked` (V9), `idea_verdict_ignores_lessons_and_marks`, `idea_library_does_not_count_as_service` (V10).
- OCR-backed `judged_*` verdict test (`00-overview.md` 8.3 question 3): allowed, tagged `"OCR"`, still requires user confirmation.

## 8. Document conflicts and resolutions

1. **Full-text evidence & context events:** Earlier `explore.md` used `idea.evidence` and a per-reader sent-lines store. **Resolution:** `00-overview.md` 2.5 (owner decision 10) mandates shared `evidence.quote` (`xe-<seq>`) and `context.supplied`, checked via F4 `quote_core::accept_quote`.
2. **Edge direction in `GraphView`:** Earlier `explore.md` left `from`/`to` vs `src`/`dst` implicit. **Resolution:** `00-overview.md` 2.6 and reconciled `explore.md` 4.1 mandate explicit `"from kind to"` in the recorder schema and the fixed 9-kind `src` (prerequisite) -> `dst` (dependent) mapping table in `idea_view`.
3. **PDF & OCR handling:** Earlier `explore.md` deferred PDF/scan handling to an add-on. **Resolution:** `00-overview.md` 0.1, 2.12, and 8.3 question 3 make PDF text (`F14a`) a normal path in X9/X10 and permit `judged_*` verdicts on OCR-tagged quotes.
4. **Origin paper in Investigate -> Explore hand-offs:** Earlier `explore.md` lacked an origin exclusion rule. **Resolution:** `00-overview.md` 4.2 & 4.3 (reconciled in `explore.md` Rule V9) require `idea_verdict::compute` to skip overlaps matching `handoff.packet.from.paper`.

## 9. Out of scope and hand-back

- **Out of scope:** Map/verdict UI (`X12`), directions & hand-over buttons (`X13`), F15 cited summary (`writeup_idea.rs`, Group 5), Conversation turns (`X15`), citation expansion (`X16`), F14b Tesseract OCR / F14c page-image model transcription (Group 4).
- **Hand-back:** Write `docs/design/modes-2026-11/reports/exp-x9-x10-x11-read-verdict.md` (recording exact `idea.map` / `idea.verdict` schemas and `GraphView` payload fixture for X12), commit with `Co-Authored-By`, and push after each checkpoint to `gemini/modes/exp-x9-x10-x11-read-verdict`.
