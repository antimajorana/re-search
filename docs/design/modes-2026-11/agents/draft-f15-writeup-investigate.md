# Brief f15-writeup-investigate — F15 Investigate write-up adapter (reading notes)

Build the **third** F15 write-up adapter (`export/src/writeup_paper.rs`) and register it in the
shared write-up dispatch seam. An Investigate write-up produces structured reading notes (outline,
main results, gists marked `"(model)"`, user notes, bibliography, status line, and check/basis
table) as a pure, deterministic function of verified ledger events (plus read-only library lookup)
into F15 core's `Writeup` document model, rendered to TeX (+ `.bib`) and Markdown with **zero model
calls**.

## 1. Package and governing design sections

- **Package:** F15 Investigate adapter (`00-overview.md` 2.1 row F15, `investigate.md` 3 stage L',
  7, 9 V15).
- **Read first:** `agents/RULES.md`, `00-overview.md` 0.1, 2.1 row F15, 2.5, 2.10, 2.11, 2.12,
  2.13 in full, 3.2 rows 19–24, 6, 7.2 set W, 7.3, 8.1, 8.3 questions 3 & 10; `investigate.md`
  3 (stages C/C'/D/E/L'), 4.1–4.4, 5.2, 6 rules 6.2/6.5/6.6/6.12/6.21/6.22, 9 V15, 12 (copyright
  in exports), 13 decision 9; reports `f4-f9-quote-budget.md`, `f10-library.md`, `f14a-pdf-core.md`,
  F15 core (`lib-f13a-f16a-f15`), and `f15-writeup-solve.md`.
- **Owner decisions of 2026-11-25 (binding):**
  - `"verified"` is **never** shown anywhere in Investigate views, exports, or write-ups
    (`00-overview.md` 0.1, 3.2 row 20, 7.3). Quotes are `"matched"`.
  - Provenance is non-removable (`00-overview.md` 2.13 rule 1, 8.3 q10): inline marks may be
    toggled off, but the opening status line (including skeleton builder/quality) and closing
    `"How each statement was checked"` table can never be removed.
  - **V14 (prior work inside Investigate)** and **version tracking (half of V10)** are **deferred**
    (`00-overview.md` 0.1). Do not depend on `investigation.prior_work` or `Carry`.
  - **PDF-only and scanned papers are normal cases (`00-overview.md` 2.12, `investigate.md` 6.21):**
    every PDF quote names its physical page; every scan quote is tagged `"(OCR)"`; nothing from
    OCR text is rendered as mathematics.
  - **Quote cap & copyright (`00-overview.md` 2.13, `investigate.md` 12 & D9):** statements are
    capped at `<= 300` chars (`quote_core::MAX_DISPLAY_QUOTE_CHARS`) unless the paper's recorded
    licence explicitly permits longer redistribution (fail-closed by default).

## 2. Start conditions and build order

- **Adapter build order (`00-overview.md` 2.13):** 1st Solve (`draft-f15-writeup-solve.md`) ->
  2nd Explore (`draft-f15-writeup-explore.md`) -> **3rd Investigate** (this brief).
- **Already merged on base `832eab5bf`:** F2 (`paper_key.rs`), F4 (`quote_core.rs`, `lines_sent.rs`),
  F9 (`run_budget.rs`), F10 (`library*.rs`), F14a (`pdf_pages*.rs`, `pdf_match.rs`), V1
  (`skeleton/model.rs`, TeX core).
- **Required merges before starting:**
  - **F15 core** (`lib-f13a-f16a-f15`) and **F13a** (`source_item.citekey`).
  - **F15 Solve adapter** (`f15-writeup-solve`), which establishes `WriteupFormat::{Tex, Md, Bib}`,
    `export::writeup_session_*`, `desktop-server/src/routes_writeup.rs`, `assets/writeup_btn.js`,
    and `cli/src/cmd_writeup.rs`.
  - **F15 Explore adapter** (`f15-writeup-explore`) — preferred serialization on `export/src/lib.rs`
    per `00-overview.md` 2.13 (1st Solve -> 2nd Explore -> 3rd Investigate). If Explore lags
    Investigate, this adapter may still merge cleanly because its test files are completely disjoint
    and it edits only a 1-line `pub mod writeup_paper;` + `Mode::Investigate` match arm in
    `export/src/lib.rs` (leaving Explore to add its 1-line arm later without touching `writeup_paper.rs`).
  - **F5** (`session_mode.rs`), **V2/V3** (`investigation.skeleton` event + blob), **V6/V7**
    (`investigation.reading`, Orient main results, gists, authors' claims, `map_fold.rs`),
    **V8** (open node readings, hypotheses, proof children), **V10** (user notes & pins in
    `investigation.note`), and **V17** (`skeleton/pdf.rs` `pdf-1` skeleton).
  - **Optional merge (graceful if absent):** **F16a** (`investigation.override` reader overrides).

## 3. Verified base interfaces vs future-merge interfaces

### Verified on base `832eab5bf`
- `research_tools::skeleton::model::{PaperSkeleton, SkNode, SkNodeKind, SkeletonBuilder,
  SkeletonQuality, Span, BibRef}` (`skeleton/model.rs`): exact layer-1 skeleton types.
- `research_tools::paper_fetch_meta::PaperMetadata.licence: Option<String>` (`paper_fetch_meta.rs:36`,
  UK spelling, extracted from arXiv Atom `<license>` / `<link rel="license">`).
- `research_tools::library_schema::{PaperDetail, PaperRecord, PaperFlags}` (`library_schema.rs:227`,
  US spelling `PaperFlags.license: Option<String>`, plus `PaperRecord.bibtex` and
  `PaperDetail.source_items[].citekey`).
- `research_tools::ingest` (`ingest.rs:280-299`): confirmed that `source.ingested` pins
  `source_id`, `title`, `format`, `files_sha256`, `pdf_extractor`, and `pdf_pages: Vec<PageRecord>`,
  and does **NOT** currently store a `licence`/`license` field on base `832eab5bf`.
- `research_tools::quote_core::{verify_quote, verify_quote_ref, clip_quote, MAX_DISPLAY_QUOTE_CHARS}`
  (`quote_core.rs`).
- `research_tools::pdf_pages::{PageMap, PageBasis}` (`pdf_pages.rs:41-48`): has **3 variants on
  base**: `{TextLayer, ScanTextLayer, None}` (`PageBasis::Ocr` exists only in
  `library_ext_types::PageBasis`, not in `pdf_pages.rs` until F14b).
- `ledger::Ledger::read_verified_session` (`crates/ledger/src/verify.rs:84`, used in
  `crates/export/src/latex.rs:135-136`): fail-closed hash-chain verified session read.
- `research_tools::tex_audit` (`tex_audit.rs`): deterministic TeX & citation validation for tests.

### Future-merge interfaces to verify before coding
- **F15 core (`export/src/writeup*.rs`) and `f15-writeup-solve` seam:** verify `Writeup` document
  model types, `WriteupOptions`, `WriteupFormat::{Tex, Md, Bib}`, `writeup_session_*` dispatch
  signatures, and `assets/writeup_btn.js`.
- **V3 / V4 paper metadata & licence persistence:** verify whether V4 (`investigation_open.rs`)
  writes `licence` / `license` onto `investigation.paper`, `source.ingested`, or `Library::upsert_paper`
  (`PaperFlags.license`), and confirm the exact blob format of `investigation.skeleton`.
- **V6 / V7 / V8 / V10 / F16a event payloads (`map_fold.rs`, `status_badge.rs`,
  `investigation_notes.rs`):** verify exact folded structs for `investigation.reading` (main
  results, gists, hypotheses, authors' claims, cues, model proof children), `investigation.note`
  (user text notes + `main_result` / `not_main_result` pins), and `investigation.override`
  (`kind`, `span`, `not_a_node`, `number`).
- **V17 (`skeleton/pdf.rs`):** verify `pdf-1` `quality_reason` and span-to-page lookup helper.
- **F14b (optional, future):** `pdf_pages::PageBasis::Ocr` is added only when F14b lands. Until
  then, detect scanned/OCR pages via `pdf_pages::PageBasis::ScanTextLayer` and raw JSON `"ocr"`
  basis strings so this adapter compiles cleanly before F14b.

## 4. Smallest implementation checkpoints

1. **Skeleton load, fold, and status line (`writeup_paper.rs`):**
   - Load verified session events via `Ledger::read_verified_session` (`crates/ledger/src/verify.rs:84`).
     Load pinned `investigation.skeleton` blob (re-verify blob hash; fail closed if missing or
     corrupt per rule 6.2).
   - Fold layer-2 records via `map_fold` (V6/V7/V8): latest reading items per `(tag, field)`,
     authors' claims, open-node explanations, reader pins (`investigation.note` pins override model
     `main_result` role), user notes, and any F16a `investigation.override` records (marked
     `"corrected by you"`, never mutating the skeleton or altering authors' claims/anchors per 6.22).
   - Opening status line states the paper title/key/version, skeleton builder & quality (e.g.
     `"Structure from TeX (tex-1, full)"` or `"Structure estimated from PDF text (pdf-1, weak —
     scanned pages)"`), and what tasks ran (Orient, N opened nodes, or `"Skeleton only — no model
     tasks run"`).
2. **Outline, main results, licence-gated statement length, and OCR safety:**
   - **Outline section:** hierarchical section tree from `PaperSkeleton` with reading minutes and
     result counts per section (works with zero model tasks!).
   - **Main results section:** all nodes pinned `main_result` by the user or labelled `main_result`
     by Orient (falling back to theorem-like skeleton nodes if Orient has not run, clearly noted).
     For each result:
     - **Statement excerpt:** read verbatim from the hash-verified physical source file at the
       node's `Span` via F4 (`quote_core`) — never emit an unverified excerpt.
     - **Licence check (fail-closed):** inspect recorded licence in deterministic order: (a)
       session ledger metadata (`investigation.paper` / `source.ingested`, checking both `"licence"`
       and `"license"`), (b) read-only `Library::get_paper(&skeleton.paper_key)` -> `paper.flags.license`.
       Allow full statements `> 300` chars **only** for an explicit allow-listed permissive licence
       (e.g. `CC-BY*`, `CC0*`, `creativecommons.org/licenses/by/`, `creativecommons.org/publicdomain/zero/`,
       explicitly rejecting arXiv's default `nonexclusive-distrib` and unknown/absent licences);
       otherwise cap strictly at `<= 300` chars (`clip_quote`).
     - **PDF & OCR rules (`00-overview.md` 2.12, 2.13 rule 6, `investigate.md` 6.21):** compute
       physical page via `PageMap::page_of` for PDF sources; if any span line lies on
       `pdf_pages::PageBasis::ScanTextLayer` (or raw JSON `"ocr"` basis string from future F14b),
       tag `"(OCR)"` and emit strictly as plain text quote (never inside TeX math delimiters or
       `Block::Formula`).
     - **Model interpretation:** gists, purposes, open-node explanations, and proof steps emitted
       as `Block::Prose { basis: "model" }` marked `"(model)"`; authors' novelty claim shown
       strictly as `"Authors: <New|Restated|Background|Not stated> (model)"` with its matched quote
       and deterministic cue (`investigate.md` 6.6).
3. **User notes, bibliography (F10/F13a), and non-removable check/basis table:**
   - **User notes:** include `investigation.note` text notes attached to their nodes as
     `Block::Prose { basis: "user" }` (`"Your note"`). (Optional opt-out query flag
     `include_notes=0` when sharing notes publicly, defaulting to `true` for reading notes.)
   - **Bibliography:** cite the investigated paper itself plus any cited references referenced in
     reading notes/authors' attributions, resolving `source_item.citekey` (F13a) via read-only
     `Library::get_paper` first and falling back to deterministic app-generated keys + `.bib`
     entries from `PaperRecord.bibtex` or `PaperSkeleton.bib`.
   - **Closing table (`"How each statement was checked"`):** non-removable table listing each
     included section/result/note, its provenance basis (`skeleton by code (tex-1/pdf-1)`,
     `matched full-text quote`, `matched OCR quote`, `model gist (model)`, `authors' claim (model)`,
     `user note / pin`, `corrected by you`), locator (file:lines or PDF page), and licence cap status.
     `WriteupOptions.inline_marks` toggles inline tags only; status line and check table stay.
4. **Register in shared dispatch & UI seams (1-line hooks only):**
   - Wire `Mode::Investigate => writeup_paper::build_writeup(...)` in `export/src/lib.rs`.
   - Verify `WriteupFormat::{Tex, Md, Bib}` works across `writeup_session_bytes`,
     `routes_writeup.rs` (`GET /api/sessions/{id}/writeup?format=tex|md|bib`), and `cmd_writeup.rs`
     with zero extra route boilerplate.
   - Mount `assets/writeup_btn.js` (from `f15-writeup-solve`) in Investigate's
     `assets/investigation_handoff_ui.js` or `assets/investigation_node.js` if merged; otherwise
     record the exact 1-line mount snippet in report Merge notes.

## 5. Owned files (all <= 600 lines)

Disjoint from F15 core, `writeup_solve.rs`, `writeup_idea.rs`, `writeup_solve_routes.rs`,
`writeup_explore_routes.rs`, and V15's `.rsmap`/HTML export (`export/src/paper_map*.rs`,
`desktop-server/src/investigation_share.rs`):
- `crates/export/src/writeup_paper.rs` (Investigate reading-notes adapter; split licence/bib
  helpers into `writeup_paper_meta.rs` if > 500 lines)
- `crates/export/tests/writeup_paper.rs` (entry-point + Set W + licence + OCR + wording tests)
- `crates/desktop-server/tests/writeup_investigate_routes.rs` (Investigate HTTP route + auth tests)

## 6. Shared registration seams and "Do not edit" files

- **Allowed minimal registration edits (isolated commit):**
  - `crates/export/src/lib.rs`: declare `pub mod writeup_paper;` and add the `Mode::Investigate`
    arm in the shared `writeup_session_*` dispatcher.
  - `crates/desktop-server/assets/investigation_handoff_ui.js` or `investigation_node.js`: 1-line
    mount of `writeup_btn.js` if merged; otherwise leave the exact mount snippet in Merge notes.
- **Do NOT edit:**
  - `crates/export/src/paper_map*.rs` and `desktop-server/src/investigation_share.rs` (V15 `.rsmap`
    and standalone HTML export — stays separate).
  - `crates/export/src/latex.rs` (771 lines), `writeup_solve.rs`, `writeup_idea.rs`.
  - `crates/desktop-server/src/http_srv.rs` (702 lines), `routes_tools.rs` (610 lines),
    `routes_session.rs` (616 lines), `investigation.rs` (597 lines — only 3 lines under 600!),
    `auth.rs` (340 lines), `tests/writeup_solve_routes.rs`, `tests/writeup_explore_routes.rs`.
  - `crates/research-tools/src/skeleton/*`, `ingest.rs`, `paper_fetch_meta.rs`, `quote_core.rs`,
    `Cargo.lock`, `crates/research-tools/tests/golden/*`.

## 7. Tests through the entry point (Evaluation Set W, `00-overview.md` 7.3)

All tests use self-written synthetic TeX and PDF fixtures (zero third-party paper text, zero model
calls, zero network calls) and run through `writeup_session_bytes` / `writeup_session_to_path` and
`handle_writeup`:
1. **TeX validity (`tex_audit`) & `.bib` resolution (`Tex | Md | Bib`):** generated `.tex` + `.bib`
   passes `tex_audit` with zero blockers/errors; every `\cite{...}` resolves to a `.bib` entry (user
   `source_item.citekey` when present, deterministic fallback when `library.db` is absent); all
   three formats (`Tex`, `Md`, `Bib`) succeed end to end.
2. **Skeleton-only & full-reading paths:** succeeds with zero model tasks (outline + theorem-like
   results + quality line) and with Orient + Open + user notes + pins + F16a overrides (pins win
   over model roles; overrides marked `"corrected by you"`).
3. **Licence-gated statement cap:** default arXiv `nonexclusive-distrib`, missing licence, and
   unknown licence all cap statements at `<= 300` chars; explicit `CC-BY-4.0` (tested from both
   ledger `"licence"` and library `PaperFlags.license`) permits full statement length.
4. **PDF & OCR honesty (6.21, rule 6):** `pdf-1` skeleton prints its `weak`/`partial` quality line;
   quotes carry physical PDF page numbers; scanned pages (`pdf_pages::PageBasis::ScanTextLayer` or
   raw `"ocr"` basis) tag quotes `"(OCR)"` and never wrap OCR text in TeX math delimiters.
5. **Non-removable provenance (q10):** with `inline_marks: false`, status line and complete
   `"How each statement was checked"` table remain present.
6. **Wording property test (`00-overview.md` 7.3):** `"verified"` NEVER appears anywhere in app or
   model output; `"proved"` never appears in app chrome; `"new"` appears ONLY inside explicit
   `"Authors: New (model)"` attribution lines, never as an app novelty verdict; gists are always
   marked `"(model)"`.
7. **Quote re-verification & tamper refusal:** corrupted/mismatched source hash or tampered anchor
   is rejected (never emitted silently).
8. **Determinism & header-token auth:** repeated calls produce byte-identical output;
   unauthenticated and cookie-only HTTP requests are rejected by `AuthGuard::authorize_api`.

## 8. Document conflicts and resolutions

- **V15 `/investigation/export?format=rsmap|md|html` (`investigate.md` 5.2) vs F15 reading notes
  (`00-overview.md` 2.13, `investigate.md` 3 L' & 9 V15):** `00-overview.md` 2.13 wins. V15 owns
  `.rsmap` and standalone HTML; F15 (`writeup_paper.rs` at `GET /api/sessions/{id}/writeup`) owns
  readable Markdown and TeX reading notes.
- **Licence field spelling & storage gap (`paper_fetch_meta.rs:36` `licence`, `library_schema.rs:227`
  `PaperFlags.license`, `ingest.rs:280` omits both):** read both spellings from ledger metadata
  events and `Library::get_paper -> paper.flags.license`, failing closed to `<= 300` chars when
  absent or non-permissive.
- **Deferred V14 prior work & V10 version tracking (`investigate.md` 4.2–4.4 vs `00-overview.md`
  0.1):** `00-overview.md` 0.1 wins. Ignore `investigation.prior_work` and `Carry`; keep the
  adapter strictly pure over the current session's pinned skeleton and ledger.
- **User notes default (`00-overview.md` 2.13 reading notes includes "the user's notes" vs
  `investigate.md` D9 "private notes only when ticked" for shared maps):** `00-overview.md` 2.13
  wins for personal reading notes (include user notes by default, with an opt-out `include_notes=0`
  toggle for sharing), while keeping full-statement redistribution strictly fail-closed on licence.

## 9. Out-of-scope items and hand-back

- **Out of scope:**
  - `.rsmap` export/import and standalone HTML export (`export/src/paper_map*.rs`, package V15).
  - Solve (`writeup_solve.rs`) and Explore (`writeup_idea.rs`) adapters.
  - V14 prior-work reading from child Explore sessions and V10 version tracking (deferred).
  - **Optional `"Polish the prose (model)"` (`00-overview.md` 2.13 rule 5):** NOT built or
    duplicated here. Tracked as a single follow-up brief **`f15-writeup-polish`** (owning
    `export/src/writeup_polish.rs` + `desktop-server/src/writeup_polish.rs`, locking quotes,
    citations, status, and check table, running `honesty` lint and V8 phrase check, and emitting
    `"prose edited by a model"`) after all 3 pure adapters merge. This adapter tags gists and
    model explanations as `Block::Prose { basis: "model" }` and user notes as `basis: "user"` so
    the future shared polisher only touches eligible prose.
- **Hand-back (`RULES.md`):** run `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`,
  `cargo test -p research-tools --test tool_def_golden`, `cargo clippy --workspace --all-targets`,
  and `cargo test --workspace --no-fail-fast` (plus `node --check` / `node --test` if Investigate
  UI JS was edited); commit `docs/design/modes-2026-11/reports/f15-writeup-investigate.md` last;
  push to `gemini/modes/f15-writeup-investigate`.
