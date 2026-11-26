# Brief f15-writeup-explore — F15 Explore write-up adapter (cited summary)

Build the **second** F15 write-up adapter (`export/src/writeup_idea.rs`) and register it in the
shared write-up dispatch seam. An Explore write-up is a pure, deterministic cited summary built
from verified ledger events (plus read-only library lookup for user citation keys and `.bib`
entries) into F15 core's `Writeup` document model, rendered to TeX (+ `.bib`) and Markdown with
**zero model calls**.

## 1. Package and governing design sections

- **Package:** F15 Explore adapter (`00-overview.md` 2.1 row F15, `explore.md` 9 X17 & F15 notes).
- **Read first:** `agents/RULES.md`, `00-overview.md` 0.1, 2.1 row F15, 2.5, 2.9, 2.10, 2.11,
  2.12, 2.13 in full, 3.2 rows 17–24, 6, 7.2 set W, 7.3, 8.1, 8.3 questions 3 & 10; `explore.md`
  4.1–4.4, 5.4, 6 rules C1–C9 / V1–V10 / D1–D3 / S8–S9, 7, 9 X17; reports `f4-f9-quote-budget.md`,
  `f10-library.md`, `f14a-pdf-core.md`, F15 core (`lib-f13a-f16a-f15`), and `f15-writeup-solve.md`.
- **Owner decisions of 2026-11-25 (binding):**
  - `"verified"` is **never** shown anywhere in Explore views, exports, or write-ups (`00-overview.md`
    0.1, 3.2 row 20, 7.3, `explore.md` rule S8). Quotes are `"matched"`.
  - Provenance and scope are non-removable (`00-overview.md` 2.13 rule 1, 8.3 q10, `explore.md`
    rule S9): inline marks may be toggled off, but the status line, the `"Where I looked"` coverage
    paragraph, and the `"How each statement was checked"` table can never be removed.
  - A `judged` status may rest on OCR text from a scanned page, always tagged `"(OCR)"` with page
    number (`00-overview.md` 2.13 rule 6, 8.3 q3, `explore.md` rule C8).
  - User library papers use the user's own citation key when present (`00-overview.md` 2.13 rule 3,
    `explore.md` 4.4); the user's library is listed in coverage and never counted as a literature
    service for the coverage gate (`explore.md` rule V10).

## 2. Start conditions and build order

- **Adapter build order (`00-overview.md` 2.13):** 1st Solve (`draft-f15-writeup-solve.md`) ->
  **2nd Explore** (this brief) -> 3rd Investigate (`draft-f15-writeup-investigate.md`).
- **Already merged on base `832eab5bf`:** F2 (`paper_key.rs`), F4 (`quote_core.rs`, `lines_sent.rs`),
  F9 (`run_budget.rs`), F10 (`library*.rs`), F14a (`pdf_pages*.rs`, `pdf_match.rs`).
- **Required merges before starting:**
  - **F15 core** (`lib-f13a-f16a-f15`) and **F13a** (`source_item.citekey`).
  - **F15 Solve adapter** (`f15-writeup-solve`), which establishes `export::writeup_session_*`,
    `desktop-server/src/routes_writeup.rs`, and `cli/src/cmd_writeup.rs`.
  - **F5** (`session_mode.rs`), **F12** (`search_coverage.rs`, `prior_work.rs`,
    `prior_work_wording.rs`), **X10** (`evidence.quote`, `idea.paper` with `text_basis`), and
    **X11** (`idea.map`, `idea.verdict`, `idea_verdict::compute`, V8 phrase checker).
  - **Optional merge (graceful if absent):** **X13** (`idea.directions`). The adapter must omit
    the Directions section cleanly when no `idea.directions` event exists in the session.

## 3. Verified base interfaces vs future-merge interfaces

### Verified on base `832eab5bf`
- `research_tools::paper_key::{PaperKey, PaperRef}` (`paper_key.rs`): canonical paper keys and
  Explore ID prefixes (`sc-`, `p-`, `c-`, `d-`, `k-`, `q-`, `w-`, `r-`, `xe-`, `xa-`).
- `research_tools::quote_core::{verify_quote_ref, clip_quote, EVIDENCE_QUOTE_EVENT,
  EVIDENCE_ABSTRACT_EVENT, MAX_DISPLAY_QUOTE_CHARS}` (`quote_core.rs`): full-text re-verification
  and 300-char UTF-8 clipping.
- `research_tools::library::Library::{open, get_paper}` (`library.rs`): read-only lookup for
  `PaperDetail.source_items[].citekey` and `PaperDetail.paper.bibtex`.
- `research_tools::pdf_pages::{PageMap, PageBasis}` (`pdf_pages.rs:41-48`, `source.ingested.pdf_pages`):
  has **3 variants on base**: `{TextLayer, ScanTextLayer, None}` (`PageBasis::Ocr` exists only in
  `library_ext_types::PageBasis`, not in `pdf_pages.rs` until F14b). Computes physical page numbers
  and scanned-page basis (`ScanTextLayer`).
- `ledger::Ledger::read_verified_session` (`crates/ledger/src/verify.rs:84`, used in
  `crates/export/src/latex.rs:135-136`): fail-closed hash-chain verified session read.
- `research_tools::run_budget::{ExploreStageState, explore_stage_run_state, RunState}` (`run_budget.rs`):
  shared on-screen run-state vocabulary for the opening status line.
- `research_tools::tex_audit` (`tex_audit.rs`): deterministic TeX & citation validation for tests.

### Future-merge interfaces to verify before coding
- **F15 core (`export/src/writeup*.rs`) and `f15-writeup-solve` seam:** verify `Writeup` document
  model types, `WriteupOptions`, `WriteupFormat::{Tex, Md, Bib}`, `writeup_session_bytes` /
  `writeup_session_to_path`, and `assets/writeup_btn.js`.
- **F12 (`search_coverage.rs`, `prior_work.rs`, `prior_work_wording.rs`):** verify exact
  `PriorWorkStatus` enum variants (`judged_same`, `judged_partial`, `judged_other_name`,
  `abstract_only`, `none_in_scope`, `incomplete`), `Coverage` recomputation function, and
  `prior_work_wording` sentence/coverage-paragraph builders.
- **X10 / X11 / X13 ledger payloads (`idea_evidence.rs`, `idea_map.rs`, `idea_verdict.rs`,
  `idea_directions.rs`):** verify exact JSON field names on `idea.stated`, `idea.brief`,
  `idea.paper` (`text_basis`), `evidence.quote`, `evidence.abstract`, `idea.map`, `idea.verdict`,
  `idea.directions`, and `idea.note` (`confirm_same` / `reject_same`), plus `idea_verdict::compute`
  and the V8 phrase-check helper.
- **F14b (optional, future):** `pdf_pages::PageBasis::Ocr` is added only when F14b lands. Until
  then, detect scanned/OCR quotes via `pdf_pages::PageBasis::ScanTextLayer`, raw JSON `"ocr"` /
  `"scan_text_layer"` strings (`idea.paper.text_basis` or `pdf_pages`), and `evidence.quote.ocr`
  so this adapter compiles cleanly before F14b.

## 4. Smallest implementation checkpoints

1. **Ledger fold, verdict recomputation, and status line (`writeup_idea.rs`):**
   - Load verified session events via `Ledger::read_verified_session` (`crates/ledger/src/verify.rs:84`).
     Fold `idea.started`, latest `idea.stated`, `idea.brief`, `idea.paper`, `evidence.quote` (`xe-*`),
     `evidence.abstract` (`xa-*`), `idea.map`, `idea.verdict`, `idea.directions` (optional),
     `idea.note`, and `idea.stage`.
   - **Rule V7 & V10 honesty check:** recompute `Coverage` (F12) and per-subclaim `Verdict`
     (`idea_verdict::compute`, respecting origin-paper exclusion V9 and user `confirm_same` notes
     V6, reading zero F16 lessons or library marks). Fail closed if stored `idea.verdict` disagrees
     with recomputation, or if no `idea.stated` / `idea.brief` exists to summarize.
   - Opening status line reflects actual pipeline run state via F9 (`explore_stage_run_state`) and
     coverage completeness (e.g. `"Done — complete search"` vs `"Done — incomplete search"` or
     `"Paused: ..."`), never claiming novelty or verification.
2. **Per-part findings, matched quotes (`xe`/`xa`), citations, and OCR tagging:**
   - For each subclaim (`sc-N`) in `idea.brief`:
     - Status sentence strictly from F12's `prior_work_wording` (showing `"Model judges: ..."` for
       unconfirmed `judged_*` statuses, confirmed wording only after `idea.note confirm_same`,
       `"possibly related (abstracts only)"` for `abstract_only`, scoped `"not found"` only when
       F12 gate passes).
     - Matched quotes (`xe` full-text and `xa` abstract): re-verify every `xe` quote through F4
       `quote_core::verify_quote_ref` against `source.ingested` and every `xa` quote against its
       pinned `raw_blob` (never emit an unverified quote). Clip quotes to `<= 300` chars.
     - Tag every `xa` quote `"abstract only"`; tag every PDF quote with its computed page; tag
       every quote from `idea.paper.text_basis` in `{"scan_text_layer", "ocr"}`,
       `pdf_pages::PageBasis::ScanTextLayer` (or raw JSON `"ocr"` basis), or `evidence.quote.ocr`
       as `"(OCR)"` (never render OCR text as mathematics).
     - Emit `"What differs (model)"` from `Overlap.difference` as `Block::Prose { basis: "model" }`,
       validated against Explore's V8 banned-phrase list.
   - Resolve citations via read-only `Library::get_paper`: prefer `source_item.citekey` (F13a),
     else deterministic app-generated key; emit complete `BibEntry` records so every `\cite{...}`
     resolves in `.bib`.
3. **Non-removable coverage paragraph, optional directions, and check table:**
   - **Coverage paragraph (`"Where I looked"`):** built strictly by F12 app code (services queried
     and their states, queries ok/failed, abstracts screened, papers read full/passages/unavailable,
     user library counts listed separately from the 2-service gate, fixed `not_searched` text, and
     any failed gate conditions). Cannot be toggled off even when `inline_marks: false`.
   - **Directions section (optional, when `idea.directions` exists):** group Concrete first, then
     Loose ideas (`concreteness == sketch`); render Known (cited `xe` chips), Missing (with basis),
     First calculation + Check first, Risks, and Stop if; mark free prose `basis: "model"`
     (`"suggestion (model)"`). Omit section cleanly if `idea.directions` is absent.
   - **Check table (`"How each statement was checked"`):** non-removable closing table listing each
     subclaim, its F12 status + confirmation state, evidence basis (`matched full-text quote`,
     `matched OCR quote`, `abstract only`, `coverage gate`), and `"what differs (model)"` /
     direction prose provenance.
4. **Register in shared dispatch & UI seams (1-line hooks only):**
   - Wire `Mode::Explore => writeup_idea::build_writeup(...)` in `export/src/lib.rs` (if
     `f15-writeup-investigate` merges first, add only the `pub mod writeup_idea;` + `Mode::Explore`
     arm without touching `writeup_paper.rs`).
   - Verify `WriteupFormat::{Tex, Md, Bib}` works across `writeup_session_bytes`,
     `routes_writeup.rs` (`GET /api/sessions/{id}/writeup?format=tex|md|bib`), and `cmd_writeup.rs`
     with zero extra route boilerplate.
   - Mount `assets/writeup_btn.js` (from `f15-writeup-solve`) in Explore's `assets/idea_map.js`
     if X12 is already merged; otherwise record the exact 1-line mount snippet in report Merge notes.

## 5. Owned files (all <= 600 lines)

Disjoint from F15 core, `writeup_solve.rs`, `writeup_paper.rs`, `writeup_solve_routes.rs`,
`writeup_investigate_routes.rs`, and X17's machine-readable `export/src/idea.rs`:
- `crates/export/src/writeup_idea.rs` (Explore write-up adapter; split helpers into
  `writeup_idea_cite.rs` if > 500 lines)
- `crates/export/tests/writeup_idea.rs` (entry-point + Set W + wording property tests)
- `crates/desktop-server/tests/writeup_explore_routes.rs` (Explore HTTP route + auth tests)

## 6. Shared registration seams and "Do not edit" files

- **Allowed minimal registration edits (isolated commit):**
  - `crates/export/src/lib.rs`: declare `pub mod writeup_idea;` and add the `Mode::Explore` arm in
    the shared `writeup_session_*` dispatcher.
  - `crates/desktop-server/assets/idea_map.js` (created by X12): 1-line mount of `writeup_btn.js`
    if X12 is merged; otherwise leave the exact mount snippet in report Merge notes.
- **Do NOT edit:**
  - `crates/export/src/idea.rs` (X17 JSON/BibTeX machine export — stays separate).
  - `crates/export/src/latex.rs` (771 lines), `writeup_solve.rs`, `writeup_paper.rs`.
  - `crates/desktop-server/src/http_srv.rs` (702 lines), `routes_tools.rs` (610 lines),
    `routes_session.rs` (616 lines), `auth.rs` (340 lines), `tests/writeup_solve_routes.rs`,
    `tests/writeup_investigate_routes.rs`.
  - `crates/research-tools/src/prior_work_wording.rs`, `search_coverage.rs`, `quote_core.rs`,
    `Cargo.lock`, `crates/research-tools/tests/golden/*`.

## 7. Tests through the entry point (Evaluation Set W, `00-overview.md` 7.3)

All tests use self-written synthetic fixtures (zero third-party paper text, zero model calls, zero
network calls) and run through `writeup_session_bytes` / `writeup_session_to_path` and `handle_writeup`:
1. **TeX validity (`tex_audit`) & `.bib` resolution (`Tex | Md | Bib`):** generated `.tex` + `.bib`
   passes `tex_audit` with zero blockers/errors; every `\cite{...}` resolves to a `.bib` entry using
   `source_item.citekey` when present and deterministic fallback when `library.db` is absent; all
   three formats (`Tex`, `Md`, `Bib`) succeed end to end.
2. **All F12 statuses & user confirmation (V5, V6):** fixture covering `judged_same` (unconfirmed
   `"Model judges: ..."` vs confirmed via `idea.note confirm_same`), `judged_partial`,
   `judged_other_name`, `abstract_only` (`xa` quote tagged `"abstract only"`), `none_in_scope`,
   and `incomplete` (failing gate conditions listed).
3. **Verdict recomputation & lesson invariance (V7, V10):** tampered `idea.verdict` payload fails
   closed; adding `lessons.supplied` or library marks does not alter any status sentence or gate.
4. **Non-removable provenance & coverage (q10, S9):** with `inline_marks: false`, status line,
   `"Where I looked"` coverage paragraph (with library counts + `not_searched`), and `"How each
   statement was checked"` table are all preserved.
5. **Directions optionality:** renders Concrete + Loose ideas with `"(model)"` tags when
   `idea.directions` is present, and succeeds cleanly omitting the section when absent (post-X11
   or Quick preset).
6. **Wording property test (`00-overview.md` 7.3, S8, V8):** across all generated status/coverage
   combinations, `"verified"` NEVER appears anywhere in app/model text or tables; `"new"`, `"novel"`,
   and `"proved"` never appear in app-generated status/table/coverage chrome; V8 banned phrases in
   model prose are rejected/caught.
7. **Quote re-verification & OCR tagging (C1, C8):** tampered `xe` quote is never written; quote
   > 300 chars is clipped on UTF-8 boundary; scanned PDF quote (`pdf_pages::PageBasis::ScanTextLayer`
   or raw `"ocr"` / `"scan_text_layer"` basis) carries page number and `"(OCR)"` without math rendering.
8. **Determinism & header-token auth:** repeated calls produce byte-identical output;
   unauthenticated and cookie-only HTTP requests are rejected by `AuthGuard::authorize_api`.

## 8. Document conflicts and resolutions

- **X17 Markdown export vs F15 Explore adapter (`explore.md` 7 & 9 X17 vs `00-overview.md` 2.13):**
  earlier Explore drafts put Markdown export inside X17 (`export/src/idea.rs`). `00-overview.md`
  2.13 wins (already reflected in reconciled `explore.md` 9 X17): `export/src/idea.rs` is strictly
  machine JSON/BibTeX; human-readable cited summaries live in `export/src/writeup_idea.rs`.
- **Old event/status names (`idea.evidence`, `idea.abstract_evidence`, `found_same`,
  `not_found_in_scope` in older passages vs `00-overview.md` 2.5 & 2.9):** `00-overview.md` wins.
  Read shared `evidence.quote` / `evidence.abstract` and F12 `PriorWork` statuses exclusively.
- **Directions timing (`00-overview.md` 2.13 "after X11" vs "Then the directions"):** X13 may not
  have run yet when a user writes up an X11 session; render Directions when present and omit cleanly
  when absent.

## 9. Out-of-scope items and hand-back

- **Out of scope:**
  - Machine-readable JSON/BibTeX export (`export/src/idea.rs`, package X17).
  - Solve (`writeup_solve.rs`) and Investigate (`writeup_paper.rs`) adapters.
  - **Optional `"Polish the prose (model)"` (`00-overview.md` 2.13 rule 5):** NOT implemented or
    duplicated here. Tracked as a single follow-up brief **`f15-writeup-polish`** (owning
    `export/src/writeup_polish.rs` + `desktop-server/src/writeup_polish.rs`, with placeholder
    locking, `honesty` lint, Explore V8 phrase check, and `"prose edited by a model"` banner)
    after all 3 pure adapters merge. This adapter marks only `"what differs (model)"` and
    direction suggestions as `Block::Prose { basis: "model" }`, keeping status sentences, quotes,
    citations, coverage paragraph, and check table locked.
- **Hand-back (`RULES.md`):** run `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`,
  `cargo test -p research-tools --test tool_def_golden`, `cargo clippy --workspace --all-targets`,
  and `cargo test --workspace --no-fail-fast` (plus `node --check` / `node --test` if `idea_map.js`
  was edited); commit `docs/design/modes-2026-11/reports/f15-writeup-explore.md` last; push to
  `gemini/modes/f15-writeup-explore`.
