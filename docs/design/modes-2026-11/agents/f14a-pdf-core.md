# Brief f14a-pdf-core: PDF text core (package F14a)

The owner calls PDF-only and old papers essential. Investigate's PDF skeleton (V17) and the
library's own-papers import (F13a) are built on this package.

Design: `00-overview.md` section 2.12 (all of it; you build part a only), row F14 of 2.1, the
row about F14a in the "What can move" table of section 6, 7.2 set P6 and the PDF rows of 7.3.
`investigate.md`: the facts table row on PDF extraction running synchronously inside ingestion,
and package V17 for what will consume your output.

## Deliver

`crates/research-tools/src/pdf_pages.rs`, `pdf_firstpage.rs`, `pdf_match.rs`; edits in `pdf.rs`
and `ingest.rs` (565 lines: it must stay at or under 600; move code out if you need room).

1. The reading file and page map as 2.12 defines them: extracted text with a form feed between
   pages, pinned in the source receipt with the extractor id; anchors stay `{path, start_line,
   end_line}`; `page_of(line)` is a lookup. **An existing `source.ingested` receipt and its
   reading file must keep meaning what they mean today**: a PDF ingested before your change still
   opens, its recorded hashes still verify, and quotes recorded against it still match. Test this
   with a receipt built by the code as it is at your base.
2. Extractor choice by measurement, as the design says. Only what is already a dependency may be
   used (`pdf-extract`, and whatever `pdf.rs` uses today); the design may name a second
   extractor: if it is not already in `Cargo.lock`, do not add it. Build the comparison harness
   and the fixture set so that a second extractor can be measured later, measure the one you
   have, and report the numbers (reading order on two-column pages, hyphenation, ligatures,
   empty pages).
3. Scan detection (a page with no text layer or with too little text for its size) and the
   per-page quality record. No OCR (that is F14b).
4. First-page metadata: title, authors, year, DOI and arXiv id candidates found by code on the
   first page, each with the line it came from, never presented as certain. `pdf_match.rs`:
   matching those candidates to a `PaperKey` (F2) without the network; the network lookup is a
   function that takes a fetcher, tested against a mock (F3's `paper_fetch` is in your checkout).
5. The background job: extraction of a large PDF must not block the caller. Deliver the job type
   with progress per page that a later route runs on a thread (see `paper_fetch_job.rs` for the
   shape F3 chose) and keep the existing synchronous path working.
6. PDF fixtures: generate them in the test from text you wrote (a minimal PDF writer in the test
   helper is fine: one column, two columns, a page with no text, a first page with title and
   DOI). No third-party PDF in the repository. Malformed, encrypted and huge inputs are refused
   without panic (see `paper_fetch_validate.rs`).

## Not in this brief

OCR (F14b), page images to a model (F14c), the PDF skeleton (V17), routes and UI.
