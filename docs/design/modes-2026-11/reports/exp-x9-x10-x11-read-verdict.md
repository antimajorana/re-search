# Report: `exp-x9-x10-x11-read-verdict` — Reading queue (X9), passage reader (X10), and map, overlaps & verdict (X11)

- **Brief:** `docs/design/modes-2026-11/agents/exp-x9-x10-x11-read-verdict.md`
- **Base commit (`origin/gemini/work/fixed`):** `20c6d09938c12951e1c1783ffa4daf244d4a6573`
- **Branch pushed:** `gemini/modes/exp-x9-x10-x11-read-verdict`
- **Commits (chronological, excluding this final report commit):**
  1. `09da473` `X9: wire idea_fetch_papers over F3 paper_fetch with F10 paper_text cache and F13a containment`
  2. `4c7ddc9` `X9: add reading queue orchestration, Attach PDF route, and view projection`
  3. `2bf5e05` `X9: render reading-queue row states, OCR notices, Attach PDF, and SSE sync`
  4. `c21392d` `X10: add passage-pack builder and record_paper_evidence recorder over F4/F14a`
  5. `6f169c5` `X10: wire parallel passage readers, follow-up want_lines, F10 findings, and SSE quotes`
  6. `d344972` `X11: add record_idea_map, F6 GraphView projection, and pure idea_verdict with Rule V9`
  7. `e49215b` `X11: wire Map exchange, atomic map+verdict head chaining, and GraphView/verdict view`

---

## 1. What was implemented (package by package, with exact line counts)

All **36** new/edited source & test files vs `origin/gemini/work/fixed` (`20c6d09`) are strictly `<= 600` lines (`python3 scripts/check_file_length.py --base origin/gemini/work/fixed` exits 0; maximum is `593` lines). Zero changes to `Cargo.lock`, `tool_defs.txt`, `tool_profiles.txt`, `tool_profile.rs`, `paper_passages.rs`, `investigation_anchor.rs`, `paper_read.rs`, `investigation_evidence.rs`, `cli_turn*.rs`, `http_srv.rs`, `routes_session.rs`, `resume.rs`, or `.github/`.

### Package X9 — Reading queue, `idea_fetch_papers`, PDF text cache & Attach PDF

| File | Lines | Role |
|---|---:|---|
| `crates/research-tools/src/idea_fetch.rs` (new) | 580 | Implements `idea_fetch_papers` handler over F3 `paper_fetch::fetch` (`library` -> `user_library` -> `arxiv_eprint` -> `arxiv_pdf`, arXiv PDFs one at a time), enforces `paper_fetch::check_fetch_budget(&events, max_papers)` (Rule P3), ingests every acquired paper into session storage via `ingest_bytes` / `ingest_pdf_pages` (Rule C9: never quote-checks a `user_library` file in place), and enforces F13a user-library containment. |
| `crates/research-tools/src/idea_fetch_cache.rs` (new) | 501 | Integrates F14a `PdfExtractJob` (with page-by-page progress callbacks), F14b `PageBasis::{TextLayer, ScanTextLayer, Ocr, None}` handling, and F10 `paper_text` SQLite cache keyed by `(file_sha256, extractor_id)`. |
| `crates/research-tools/src/idea_fetch_tests.rs` (new) | 489 | 5 unit tests covering F3 fetch order, Rule P3 budget cap, Rule C9 ingest-before-reading (`idea_owned_paper_is_ingested_before_reading`), F10 `paper_text` cache reuse, and PDF/scan `text_basis` classification. |
| `crates/research-tools/src/idea_tools.rs` (edit) | 500 | Replaces the X7 placeholder handler of `idea_fetch_papers` by delegating to `idea_fetch::run_idea_fetch_papers` while keeping `idea_fetch_papers_def()` and `tool_profiles.txt` byte-for-byte unchanged. |
| `crates/research-tools/src/idea_tools_tests.rs` (edit) | 311 | Updated `idea_fetch_papers` test expectations from placeholder error to live F3/F10 fetch behavior (4 unit tests). |
| `crates/desktop-server/src/idea_read.rs` (new) | 551 | Reading queue orchestration (`fetch_papers_phase`, `run_reading_with_router_and_transport`, `handle_attach_pdf`), emitting `idea.paper` (`key`, `version`, `source_id`, `source_seq`, `origin`, `text_basis`, `status`, `reason`, `lines_sent`, `total_lines`, `raw_blob`), folding latest `idea.paper` per key in ledger order so newly attached PDFs override older terminal rows and already-fetched papers are reused. |
| `crates/desktop-server/src/routes_idea_read.rs` (new) | 262 | HTTP routes for `POST /api/sessions/{id}/idea/read` (with optional `fetch_only: true` and X18 CLI-route refusal) and `POST /api/sessions/{id}/idea/attach-pdf` (validates PDF upload via `ingest_bytes` + F14a extraction + `paper_text` cache). |
| `crates/desktop-server/src/routers.rs` (edit) | 256 | Wires `library_db_path`, `user_library_dirs`, and session `data_dir` into `build_idea_router` so `idea_fetch_papers` executes strictly through `Router::try_call` (Rule P5). |
| `crates/desktop-server/src/assets.rs` (edit) | 538 | Registers `/assets/idea_read_ui.js` (staying under the 580-line ceiling). |
| `crates/desktop-server/assets/idea_read_ui.js` (new) | 184 | Pure DOM renderer (zero `innerHTML`, zero Rule S8 banned words) for reading-queue rows: `queued`, `fetching`, `"from your library"`, `"extracting text (page k of n)"`, `reading`, `"read: passages (k of N lines), q quotes"`, `"read: full"`, `"nothing relevant"`, `"unavailable: not on arXiv and not in your library; attach PDF"`, `"scan: quotes are from OCR text"`, and Attach PDF file picker button. |
| `crates/desktop-server/assets/idea_search.js` (edit) | 525 | Mounts `renderReadingQueue` from `idea_read_ui.js` and passes reading-queue rows & Attach PDF callback. |
| `crates/desktop-server/assets/idea_state.js` (edit) | 586 | Adds SSE reducers for `idea.paper`, `idea.read_progress`, and live `evidence.quote` updates; resets reading-queue state on `idea.stated`. |
| `crates/desktop-server/assets/idea.js` (edit) | 593 | Wires `attachPdf` and `runRead` handlers cleanly to `api_idea.js`. |
| `crates/desktop-server/assets/api_idea.js` (edit) | 93 | Adds `readIdea(sessionId, body)` and `attachIdeaPdf(sessionId, key, file)` client helpers. |
| `crates/desktop-server/tests/idea_read_map.rs` (new) | 417 | 3 integration tests for reading queue fetch orchestration, Attach PDF upload/ingest/override, and `idea_view` reading-queue projection. |
| `crates/desktop-server/tests/idea_read_ui.test.mjs` (new) | 189 | 3 Node DOM tests verifying all X9 row states, OCR banner, Attach PDF trigger, SSE reduction, zero `innerHTML`, and zero Rule S8 words. |
| `crates/desktop-server/tests/mode_markers_r2.rs` (edit) | 448 | Updated `idea_fetch_papers` emitted event list to include `source.ingested` alongside `idea.paper`. |

### Package X10 — Passage reader (`record_paper_evidence`), PDF/OCR Rule C8 & parallel streaming

| File | Lines | Role |
|---|---:|---|
| `crates/research-tools/src/idea_evidence_pack.rs` (new) | 512 | `load_reading_source` (verifying `files_sha256` from `source.ingested`, `text.lines()` line counting, conservative `PageBasis` parsing, `all_pages_none()` guard), `build_passage_pack` (calls `investigation_outline::build_outline` + `paper_passages::run_search` unchanged; full file if `<= 500` lines else capped at `500`), `build_followup_pack` (`<= 3` ranges, `<= 400` lines). |
| `crates/research-tools/src/idea_evidence.rs` (new) | 546 | Strict `#[serde(deny_unknown_fields)]` `record_paper_evidence` recorder + JSON schema (`items[<=12]`, `quote` `16..=300` chars, `role: EvidenceRole`, `gist <= 240`, `subclaim_ids`, `want_lines[<=3, <=400]`, `status: done \| nothing_relevant`). Verifies every quote via F4 `accept_quote_with_sent` (Rules C1/C2); on PDFs applies F14a `match_pdf_quote` with window-shifted `PageMap` without mutating stored text; enforces Rule C8 (`PageBasis::None` + `all_pages_none()` + formula transcription refusal & `"OCR"` tagging); partial-saves valid siblings while returning indexed faults with nonce-wrapped `true_text` (only when lines were sent); deduplicates `(paper_key, cited_range/matched_range, quote_sha256)` on retry (Rule S3); refuses mid-read `idea.stated` revision drift. |
| `crates/research-tools/tests/idea_evidence.rs` (new) | 446 | 7 integration tests: `idea_evidence_rejects_fabricated_quote`, `idea_evidence_rejects_lines_not_sent`, `idea_reader_output_is_schema_only`, `idea_pdf_quote_has_page`, `idea_scan_quote_is_tagged_ocr`, `idea_transcription_is_not_evidence`, `idea_evidence_deduplicates_on_retry_and_packs_cap_at_500_lines`. |
| `crates/desktop-server/src/idea_read_passages.rs` (new) | 568 | Parallel `std::thread::scope` passage-reader worker pool (`DEFAULT_READER_CONCURRENCY = 3`) over `agent::exchange::run_exchange`; atomic `SlotGuard` enforcing concurrency + F9 `max_model_calls` + per-paper `calls_per_paper`; appends `context.supplied` before every batch (Rule C2); handles follow-up `want_lines` (`1` Standard, `2` Thorough); finalizes partial-saved quotes on cancel/cap; serializes F10 SQLite `SeenRole::Read` + `FindingRecord` writes under `library_lock`; streams live SSE `evidence.quote`, `idea.paper`, and `idea.read_progress`. |
| `crates/desktop-server/src/idea_read_passages_ctx.rs` (new) | 194 | Prompt assembly & Rule C7 `<PAPER_TEXT_{nonce}>` / `<IDEA_BRIEF_{nonce}>` nonce-delimited data wrapping and shared reading-context helpers for `idea_read_passages.rs`. |
| `crates/desktop-server/tests/idea_read_passages.rs` (new) | 523 | 2 E2E tests: `x10_parallel_passage_readers_stream_quotes_followups_and_update_library` and `x10_thorough_allows_two_followups_and_stops_at_model_call_cap`. |

### Package X11 — Map, overlaps, F6 `GraphView`, and pure `idea_verdict` with Rule V9

| File | Lines | Role |
|---|---:|---|
| `crates/research-tools/src/idea_map.rs` (new) | 520 | `RECORDER_IDEA_MAP`, `MapNodeKind`, `RelationToIdea`, `MapNodeStatus`, `MapEdgeKind` (`to_graph_direction` implementing `explore.md` 4.1 `src`/`dst` table), `MapEdgeBasis`, strict `#[serde(deny_unknown_fields)]` inputs with zero `quote`/`excerpt`/`line`/`start_line`/`end_line`/`path` fields (Rule C4), `record_idea_map_schema()` (`edges[].basis` restricted to `evidence \| model_inferred`, `overlaps[].evidence` `minItems: 1`), Rule V8 `check_explore_prose` wrapper over F12 `check_model_prose_phrases`, and F6 `project_graph_view`. |
| `crates/research-tools/src/idea_map_validate.rs` (new) | 564 | Enforces Rules C5/C6 per node kind (`concept` with `idea_quote` or `xe`/`xa`, `definition` `xe` `Defines`, `known_result` `xe` `StatesResult \| NegativeResult`, `open_question` `xe` `PosesOpenQuestion` with app-filled `stated_year` & `later_work_checked`, `related_work` `xe \| xa` with app-filled `basis: fulltext \| abstract` & verified `paper_key`), rejects other-session evidence, drops failing nodes with reasons and refuses on zero survivors, rejects model-supplied `metadata` edges, validates non-empty overlap evidence matching `paper_key` and `other_name.term`, canonicalizes all `PaperKey`s, rejects duplicate `temp_id`s and same-call `(kind, identity_key)` duplicates, assigns Rule S4 stable IDs, preserves user-noted nodes missing from re-map as `status: "not_in_current_map"`, and synthesizes `cites` edges from `idea.citation_net`. |
| `crates/research-tools/src/idea_verdict.rs` (new) | 337 | Pure `compute(events, preset_limits)` delegating to F12 `compute_prior_work_from_events` with active `idea.stated` revision scoping, dual-path Rule V9 origin-paper exclusion (`handoff.packet.from.paper` or `ask.fields.origin_paper`, including when `from.paper: null` or `from: null`), `origin_cited_keys` extraction (`papers[]` + `ask.fields.seed_papers[]` + aliases, excluding origin), Rule V6 `idea.note.target_id -> subclaim_id` bridge, explicit `preset_limits` Quick `max_papers = 0` enforcement, Rule V10 stripping of `lessons.supplied` & non-confirmation library/lesson marks, and Rule V7 `matches_recomputed`. |
| `crates/research-tools/src/lib.rs` (edit) | 382 | Exports `idea_evidence`, `idea_evidence_pack`, `idea_fetch`, `idea_fetch_cache`, `idea_map`, `idea_map_validate`, `idea_verdict`. |
| `crates/research-tools/tests/idea_map.rs` (new) | 356 | 8 integration tests: `idea_map_refuses_inline_quotes`, `idea_map_node_evidence_rules`, `idea_map_rejects_evidence_from_other_session`, `idea_open_question_needs_author_statement`, `idea_node_ids_stable_across_remap`, `idea_map_graph_view_edge_directions_and_styles`, `idea_prose_rejects_absolute_phrases`, `idea_prose_allows_first_calculation` (plus audit regressions for model `metadata` edges and empty/mismatched overlap evidence). |
| `crates/research-tools/tests/idea_verdict.rs` (new) | 369 | 11 integration tests: `idea_verdict_judged_requires_read_paper`, `idea_verdict_other_name_needs_term`, `idea_verdict_abstract_cap`, `idea_quick_never_claims_none_in_scope`, `idea_verdict_text_is_app_authored`, `idea_verdict_recomputes_from_ledger`, `idea_verdict_excludes_origin_paper` (incl. `from.paper: null`), `idea_origin_references_are_marked`, `idea_verdict_ignores_lessons_and_marks`, `idea_library_does_not_count_as_service`, `idea_verdict_allows_ocr_judged_tagged_ocr`. |
| `crates/desktop-server/src/idea_map_exchange.rs` (new) | 444 | `run_map_with_transport` & `handle_map_op`: Rule P1 stage gate, Rule P2 single-tool `record_idea_map`, Rule P3 F9 budget check, Rule C7 `<IDEA_CONTEXT_{nonce}>` & `<SESSION_EVIDENCE_{nonce}>` wrapping, head-chained `idea.map` + `idea.verdict` via `Ledger::append_at_head` under `ledger_write_lock`, and consistent `Cancelled` (on revision drift) / `Failed` stage transitions. |
| `crates/desktop-server/src/idea_view_map.rs` (new) | 121 | `annotate_origin_citations` (`cited_by_origin: true`), `build_explore_graph_view` (keeps empty `{ nodes: [], edges: [], legend: [] }` for `idea_flow` compat; preserves OCR tagging across session revisions for stale/carried-over nodes), and `build_verdicts_view` (recomputes live verdicts so post-map Rule V6 `confirm_same`/`reject_same` notes update immediately). |
| `crates/desktop-server/src/idea_view.rs` (edit) | 590 | Projects reading-queue rows, full-text evidence (`xe-<seq>` with derived `page` & `ocr_tag`), `cited_by_origin` candidate annotations, F6 `GraphView` (`view["map"]`), and live `verdicts` (`view["verdicts"]`). |
| `crates/desktop-server/src/routes_idea.rs` (edit) | 593 | Wires `"read"`, `"attach-pdf"`, and `"map"` ops and `IdeaStage::Read` / `IdeaStage::Map` in `handle_continue`. |
| `crates/desktop-server/src/lib.rs` (edit) | 279 | Exports `idea_map_exchange`, `idea_read`, `idea_read_passages`, `idea_read_passages_ctx`, `idea_view_map`, `routes_idea_read`. |
| `crates/desktop-server/tests/idea_map_e2e.rs` (new) | 328 | 1 E2E test (`x11_map_appends_map_and_verdict_atomically_and_projects_graph_view_with_v6_v9`) exercising `POST /idea/map`, atomic head chaining, `GraphView` direction/style/OCR, Rule V9 origin exclusion & `cited_by_origin`, and Rule V6 live `confirm_same` note recomputation. |
| `crates/desktop-server/tests/idea_flow.rs` (edit) | 485 | Removed `"read"` and `"map"` from the unbuilt 501 stub loop (leaving `["directions", "handoff", "more"]`). |

---

## 2. Exact JSON Schemas for Downstream Consumers (`X12` & `F15`)

### 2.1 `idea.map` ledger event payload (`IdeaMapPayload`)

```json
{
  "idea_rev": 1,
  "request_seq": 42,
  "nodes": [
    {
      "id": "k-1",
      "kind": "concept | definition | known_result | open_question | related_work",
      "rev": 1,
      "label": "Short title (<= 120 chars)",
      "statement": "Model summary (<= 1400 chars, Rule V8 checked)",
      "evidence": ["xe-14", "xa-9"],
      "relation_to_idea": "same | special_case | generalisation | ingredient | obstruction | analogue | background",
      "subclaim_ids": ["sc-1"],
      "papers": ["arxiv:2501.00001"],
      "year": 2025,
      "status": "active | not_in_current_map | flagged",
      "identity_key": "xe-14",
      "idea_quote": "verbatim substring (concepts only; omitted if None)",
      "aliases": ["optional aliases; omitted if empty"],
      "symbol": "optional; omitted if None",
      "conventions": "optional; omitted if None",
      "hypotheses": ["optional; omitted if empty"],
      "regime": "optional; omitted if None",
      "stated_year": 2025,
      "later_work_checked": true,
      "paper_key": "arxiv:2501.00001",
      "basis": "fulltext | abstract"
    }
  ],
  "edges": [
    {
      "from": "k-1",
      "to": "d-1",
      "kind": "defines | uses | special_case_of | generalises | contradicts | answers | leaves_open | same_as | cites",
      "basis": "evidence | metadata | model_inferred",
      "evidence": "xe-14",
      "reason": "required iff model_inferred (<= 400 chars, Rule V8 checked)"
    }
  ],
  "overlaps": [
    {
      "subclaim_id": "sc-1",
      "paper_key": "arxiv:2501.00001",
      "relation": "same | partial | other_name",
      "evidence": ["xe-14"],
      "difference": "model difference note (<= 600 chars, Rule V8 checked)",
      "other_name": { "term": "required iff relation == other_name" }
    }
  ],
  "dropped": [
    {
      "item_type": "node | edge | overlap",
      "reference": "temp_id or edge/overlap index",
      "reason": "human-readable rejection reason"
    }
  ]
}
```

### 2.2 `idea.verdict` ledger event payload & `view["verdicts"]` (`IdeaVerdictPayload`)

```json
{
  "map_seq": 43,
  "origin_paper": "arxiv:2401.99999",
  "verdicts": [
    {
      "subclaim_id": "sc-1",
      "status": "judged_done | judged_partial | judged_other_name | abstract_only | none_in_scope | inconclusive",
      "paper_keys": ["arxiv:2501.00001"],
      "evidence": ["xe-14"],
      "ocr_tagged": false,
      "other_name": "optional term when status == judged_other_name",
      "confirmed_by_user": null,
      "chip": "app-authored F12 chip label",
      "sentence": "app-authored F12 verdict sentence (zero Rule S8 banned words)"
    }
  ],
  "coverage": {
    "preset": "quick | standard | thorough",
    "services": [ "... F12 ServiceCoverage entries ..." ],
    "screened_count": 12,
    "papers_read": 3,
    "ocr_papers_read": 0,
    "gate_passed": true,
    "gate_reasons": []
  }
}
```

*(Exact inner field names of `verdicts[]` and `coverage` come directly from F12 `research_tools::prior_work::PriorWorkVerdict` and `research_tools::search_coverage::SearchCoverage`. When no `idea.verdict` has been recorded yet for the active revision, `view["verdicts"]` is `null`.)*

### 2.3 F6 `GraphView` projection (`view["map"]`)

When no `idea.map` exists in the active revision, `view["map"]` returns `{ "nodes": [], "edges": [], "legend": [] }` (preserving `tests/idea_flow.rs` compatibility). Once a map exists, `project_graph_view` returns:

```json
{
  "nodes": [
    {
      "id": "k-1",
      "kind": "known_result",
      "rev": 1,
      "label": "...",
      "statement": "...",
      "statement_label": "summary (model)",
      "evidence": ["xe-14"],
      "relation_to_idea": "same",
      "subclaim_ids": ["sc-1"],
      "papers": ["arxiv:2501.00001"],
      "year": 2025,
      "status": "active",
      "display_grade": "matched_quote_full_text | matched_quote_scanned_page | matched_quote_abstract | user_statement | model_statement",
      "style": "solid | dashed",
      "badge": "Found | Found (OCR) | abstract only | you | (model)",
      "ocr_tag": null,
      "idea_quote": null,
      "aliases": [],
      "symbol": null,
      "conventions": null,
      "hypotheses": [],
      "regime": null,
      "stated_year": null,
      "later_work_checked": null,
      "paper_key": null,
      "basis": null
    }
  ],
  "edges": [
    {
      "src": "d-1",
      "dst": "k-1",
      "from": "k-1",
      "to": "d-1",
      "kind": "uses",
      "role": "uses",
      "basis": "evidence | code | model",
      "symmetric": false,
      "directed": true,
      "style": "solid | dashed",
      "evidence": "xe-14",
      "reason": null
    }
  ],
  "legend": [
    { "kind": "concept", "prefix": "c-", "label": "Concept" },
    { "kind": "definition", "prefix": "d-", "label": "Definition" },
    { "kind": "known_result", "prefix": "k-", "label": "Known result" },
    { "kind": "open_question", "prefix": "q-", "label": "Open question" },
    { "kind": "related_work", "prefix": "w-", "label": "Related work" }
  ],
  "overlaps": [ "... MapOverlap[] ..." ],
  "dropped": [ "... DroppedMapItem[] ..." ]
}
```

- **Edge direction (`src` prerequisite -> `dst` dependent):** `defines` `(from, to)`, `uses` `(to, from)`, `special_case_of` `(to, from)`, `generalises` `(from, to)`, `answers` `(to, from)`, `leaves_open` `(from, to)`, `cites` `(to, from)`, `contradicts`/`same_as` `(from, to, symmetric: true, directed: false)`.
- **Edge style:** `"dashed"` iff `basis == "model"` (`ModelInferred`) OR either endpoint node is abstract-only (`!has_xe && has_xa`); otherwise `"solid"`.
- **Candidate Rule V9 annotation:** `view["candidates"][i]["cited_by_origin"] = true` is inserted whenever a candidate's canonical `PaperKey` or any alias ID matches `origin_cited_keys(events)`.

---

## 3. Independent Hard Audits (Carbon / `pro` tier) & Fixes Applied

Every checkpoint underwent an independent read-only audit using a `pro` (`research`) subagent before push:

1. **X9 Carbon Audit (`5b1bf08c`):** Audited `idea_fetch.rs`, `idea_fetch_cache.rs`, `idea_read.rs`, `routes_idea_read.rs`, `idea_read_ui.js`, and F13a/Rule C9/Rule P3 compliance. All findings (PDF ingestion containment, `paper_text` cache key verification, UI state completeness, and SSE updates) were addressed and verified before pushing `2bf5e05`.
2. **X10 Carbon Audit (`65efc5af` — 5 HIGH, 5 MEDIUM, 4 LOW, all resolved before `6f169c5`):**
   - Fixed mid-read `idea.stated` revision drift checks before appending `evidence.quote` / `idea.paper`.
   - Serialized F10 SQLite `SeenRole::Read` + `FindingRecord` writes under `library_lock` across parallel reader workers.
   - Added `all_pages_none()` guard in `idea_evidence_pack.rs` / `idea_evidence.rs` so scanned PDFs with empty page layers never bypass Rule C8.
   - Preserved partial-saved quotes on cancellation/cap via a worker finalization epilogue.
   - Folded latest `idea.paper` per key in ledger order so newly attached PDFs override earlier `unavailable` rows and already-fetched papers are reused.
   - Separated user-cancel vs budget-cap sentinels, nonce-wrapped `true_text` in fault replies (only when lines were actually sent), shifted `PageMap` cleanly for passage windows, and fixed SSE row state resets.
3. **X11 Carbon Audit (`107d331e` — 1 CRITICAL, 4 HIGH, 4 MEDIUM, 4 LOW, all resolved before `e49215b`):**
   - **CRITICAL #1:** Fixed Rule V9 `handoff.packet` backfill in `idea_verdict.rs` when `from.paper` is JSON `null` or `from` is `null` (previously `contains_key("paper")` skipped backfill from `ask.fields.origin_paper`). Added explicit regression assertion in `idea_verdict_excludes_origin_paper`.
   - **HIGH #2:** Rejected model-supplied `MapEdgeBasis::Metadata` in both JSON schema (`enum: ["evidence", "model_inferred"]`) and validator (metadata edges are app-synthesized only from `idea.citation_net`).
   - **HIGH #3:** Enforced non-empty `overlaps[].evidence` (`minItems: 1`) matching `overlap.paper_key`, and validated `other_name.term` when `relation == other_name`.
   - **HIGH #4:** Canonicalized all `PaperKey`s across nodes, overlaps, and `related_work.paper_key`, verifying `related_work.paper_key` against session evidence/candidates.
   - **HIGH #5:** Fixed `idea_map_exchange.rs` to emit `RunState::Cancelled` (409) on revision drift and `RunState::Failed` on post-exchange errors.
   - **MEDIUM #6–#8 & LOW #9–#12:** Rejected duplicate `temp_id`s and separated `temp_to_final` from `active_ids`; fixed Rule V6 `target_id -> subclaim_id` bridge when `subclaim_id: null` and stripped `idea.budget_raised` when explicit `preset_limits` enforces Quick `max_papers = 0`; rejected `idea_quote` on non-concepts; canonicalized symmetric edge dedup (`Contradicts`/`SameAs`); collected OCR evidence IDs across all session revisions in `idea_view_map.rs` so carried-over `not_in_current_map` nodes retain `Found (OCR)` / `"OCR"` tags.

---

## 4. Decisions Taken Autonomously, Deviations & Assumptions

1. **Two-call head-chained `idea.map` + `idea.verdict` append (design note):** `ledger::Ledger` currently exposes single-event `append_at_head` (not a multi-event batch transaction API), and the `ledger` crate is outside this brief's owned files. `idea_map_exchange.rs` acquires `ledger_write_lock` across both appends and chains `head0 -> map_app.event_hash` via `append_at_head`, guaranteeing no concurrent session event can interleave between `idea.map` and `idea.verdict` in normal operation. Only an unexpected I/O crash between the two `fsync`s could leave an `idea.map` without its paired `idea.verdict`; even then, `idea_verdict::compute` is a pure function of ledger events (Rule V7) and can recompute the verdict deterministically.
2. **Kept `evidence.quote` payload pure F4 `EvidenceQuotePayload`:** Rather than adding non-standard fields to F4's `EvidenceQuotePayload`, `idea_evidence.rs` writes standard `evidence.quote` events (`paper_key`, `source_id`, `cited_range`, `matched_range`, `quote`, `quote_sha256`, `role`, `gist`, `subclaim_ids`, `page`, `text_basis`), and `idea_view.rs` / SSE projection derives `ocr_tag: "OCR"` whenever `text_basis` is `"scan_text_layer"` or `"ocr"` (or the paper's latest `idea.paper.text_basis` is OCR/scan).
3. **`fetch_only: true` option on `POST /api/sessions/{id}/idea/read`:** Added an optional boolean `fetch_only` (default `false`) on `/idea/read` so callers/tests can exercise X9 fetch-only orchestration without requiring a configured model transport, while default calls run X9 fetch followed by X10 parallel passage reading.
4. **Did not edit `crates/desktop-server/assets/app.js` (932 lines):** Because `app.js` exceeds the 600-line ceiling (`RULES.md`: "Do not touch a file that is already over 600 lines"), SSE event forwarding for `idea.paper`, `idea.read_progress`, and `evidence.quote` was wired in `idea_state.js` (`applyIdeaSseEvent`), leaving `app.js` untouched (see Merge Notes below).
5. **Conservative `PageBasis` fallback:** When an ingested PDF's page map is absent or every page has `PageBasis::None`, `idea_evidence.rs` treats it fail-closed under Rule C8 (refusing ungrounded quotes and tagging OCR/scan bases cleanly) rather than assuming a clean text layer.
6. **Preserved empty `view["map"]` shape:** When `latest_map` is `None`, `idea_view_map::build_explore_graph_view` returns `{ "nodes": [], "edges": [], "legend": [] }` (omitting `overlaps`/`dropped`) so existing `tests/idea_flow.rs` assertions remain intact without weakening any test.

---

## 5. Merge Notes

### Edited existing files (14 files, 226 insertions / 21 deletions total)

- `crates/research-tools/src/lib.rs` (382L): exported `idea_evidence`, `idea_evidence_pack`, `idea_fetch`, `idea_fetch_cache`, `idea_map`, `idea_map_validate`, `idea_verdict`.
- `crates/research-tools/src/idea_tools.rs` (500L): replaced placeholder body of `idea_fetch_papers` handler with call to `idea_fetch::run_idea_fetch_papers` (`idea_fetch_papers_def()` untouched).
- `crates/research-tools/src/idea_tools_tests.rs` (311L): updated `idea_fetch_papers` unit test from placeholder assertion to real handler check.
- `crates/desktop-server/src/lib.rs` (279L): exported `idea_map_exchange`, `idea_read`, `idea_read_passages`, `idea_read_passages_ctx`, `idea_view_map`, `routes_idea_read`.
- `crates/desktop-server/src/routers.rs` (256L): passed library DB & user-library paths into `build_idea_router`.
- `crates/desktop-server/src/routes_idea.rs` (593L): routed `"read"`, `"attach-pdf"`, and `"map"` ops and `IdeaStage::Read`/`Map` in `handle_continue`. **Remaining 501 stubs:** `"directions"`, `"handoff"`, `"more"` (for X12/X13/X14). Note `routes_idea.rs` is at `593`L — downstream lanes adding routes should delegate to sibling modules (as `routes_idea_read.rs` and `idea_map_exchange.rs` do).
- `crates/desktop-server/src/idea_view.rs` (590L): wired reading-queue rows, full-text evidence (`xe-<seq>`), `annotate_origin_citations`, `build_explore_graph_view`, and `build_verdicts_view`. Note `idea_view.rs` is at `590`L — downstream lanes should put helpers in sibling modules like `idea_view_map.rs`.
- `crates/desktop-server/src/assets.rs` (538L): registered `/assets/idea_read_ui.js`.
- `crates/desktop-server/assets/api_idea.js` (93L): added `readIdea` and `attachIdeaPdf`.
- `crates/desktop-server/assets/idea.js` (593L): wired reading-queue and Attach PDF callbacks (`593`L; X12 map/verdict UI should live in new sibling JS modules).
- `crates/desktop-server/assets/idea_state.js` (586L): added SSE reducers for `idea.paper`, `idea.read_progress`, `evidence.quote`, and revision resets.
- `crates/desktop-server/assets/idea_search.js` (525L): mounted `idea_read_ui.js`.
- `crates/desktop-server/tests/idea_flow.rs` (485L): removed `"read"` and `"map"` from the 501 stub loop.
- `crates/desktop-server/tests/mode_markers_r2.rs` (448L): updated `idea_fetch_papers` emitted event list.

### Downstream integration hooks
- **`app.js` (932L, untouched):** Ensure the SSE dispatcher in `app.js` (when F5/X12 owns a split or hook there) forwards `idea.paper`, `idea.read_progress`, `evidence.quote`, `idea.map`, and `idea.verdict` SSE frames to `idea.js` / `applyIdeaSseEvent`.
- **X12 (Map & verdict UI):** Consumes `view["map"]` (Section 2.3), `view["verdicts"]` (Section 2.2), `view["candidates"][i]["cited_by_origin"]`, and `POST /api/sessions/{id}/idea/map`.
- **F15 (`writeup_idea.rs`):** Consumes persisted `idea.map` (Section 2.1), `idea.verdict` (Section 2.2), and `evidence.quote` (`xe-<seq>`).

---

## 6. Verification Commands & Exact Measured Results

All verification ran on Linux as non-root `uid=1001(antigravity)` with `RE_SEARCH_PYTHON` unset:

| Check | Command | Exact Result |
|---|---|---|
| User check | `id` | `uid=1001(antigravity)` (non-root) |
| File length ceiling | `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | Exit 0 (`No touched source file is over 600 lines.`) |
| Do-Not-Edit & golden diff | `git diff --exit-code origin/gemini/work/fixed -- Cargo.lock crates/research-tools/tests/golden/ crates/research-tools/src/tool_profile.rs crates/research-tools/src/paper_passages.rs crates/research-tools/src/investigation_anchor.rs crates/research-tools/src/paper_read.rs crates/research-tools/src/investigation_evidence.rs crates/desktop-server/src/http_srv.rs crates/desktop-server/src/routes_session.rs crates/agent/src/resume.rs .github/` | Exit 0 (0 bytes changed) |
| Workspace Clippy | `unset RE_SEARCH_PYTHON && cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` | Exit 0 (`0 warnings`) |
| JS syntax check | `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` | Exit 0 (all assets valid) |
| Node test suite | `node --test crates/desktop-server/tests/*.test.mjs` | Exit 0 (`263 passed; 0 failed; 0 skipped`, incl. `idea_read_ui.test.mjs` 3/3) |
| Full Rust workspace tests | `unset RE_SEARCH_PYTHON && cargo test --workspace --locked --no-fail-fast -- --quiet` | Exit 0 (`1442 passed; 0 failed; 2 ignored`) |

### Targeted X9 / X10 / X11 & golden test breakdown (all included in workspace run above)
- `research-tools` unit tests: `idea_fetch_tests` (5/5), `idea_tools_tests` (4/4)
- `research-tools` integration tests: `idea_evidence` (7/7), `idea_map` (8/8), `idea_verdict` (11/11), `tool_def_golden` (3/3), `tool_profile_golden` (3/3)
- `desktop-server` integration tests: `idea_read_map` (3/3), `idea_read_passages` (2/2), `idea_map_e2e` (1/1), `idea_flow` (4/4), `mode_markers_r2` (3/3)
- `desktop-server` Node tests: `idea_read_ui.test.mjs` (3/3)
- Zero `#[ignore]` or skipped tests added in any file touched or created by this brief.

### Pre-existing notices & Linux limitations
- **Pre-existing Clippy lint (documented in `RULES.md`):** `crates/desktop-server/src/evidence.rs:744` (`clippy::manual_unwrap_or`), suppressed via the mandated `-A clippy::manual_unwrap_or` flag; file untouched (`> 600`L).
- **Pre-existing ignored tests (2 total):** Both live in `crates/agent/tests/eval_harness.rs` (untouched by this brief).
- **Not compiled/run on Linux:** Windows-only `cfg(windows)` tests (`worker/tests/appcontainer_sandbox.rs` and `worker/tests/job_memory_limit.rs` report `0 tests` on Linux), and the Windows 11 + `[9/9]` Chromium E2E batch (deferred to the post-`X9–X11`-merge milestone per `RULES.md`).
