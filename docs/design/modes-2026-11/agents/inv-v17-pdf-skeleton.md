# Brief inv-v17-pdf-skeleton — Investigate PDF skeleton (V17)

Start from `gemini/work/fixed` after V2 and F14a are merged. Read `RULES.md`,
`00-overview.md` 2.12 and evaluation set P6 in 7.2, `investigate.md` 3 (PDF stage),
4.1–4.2, rule 6.21, and the V17 row in 9. Read the F14a and V2 reports and their
implementation before choosing interfaces.

- Own `crates/research-tools/src/skeleton/pdf.rs`, its helper and test files, and
  the PDF entry point and module additions in `skeleton/mod.rs`. Use the shared
  skeleton types, limits and V2 tag generation. Keep the TeX path byte-identical
  for the same input. If a view or open-route registration depends on V3 or V4,
  give the integrator the precise hook and how to test it in your report.
- Build `pdf-1` from the pinned F14a reading file and its page map. Detect numbered
  headings, theorem-like headings and proofs, margin equation numbers, numbered
  bibliography entries with DOI or arXiv IDs, and numbered citations. Use content
  fingerprints for tags. Compute pages from physical file lines. A page number
  printed in a footer, an image-only page, or an ambiguous reference must never
  create a confirmed location.
- Use `pdf_heuristic` node confidence, `printed` numbers and heuristic
  `text_mention` edges. Keep edges in prerequisite-to-dependent order and out of
  the confirmed skip plan. Quality is always `weak` or `partial`, with a reason
  including scan status when applicable. Preserve the stored text and original
  physical page links. With scan text, preserve the OCR basis for quote display
  and leave formulas as text. An image-only page remains available as an image.
- Use self-authored PDF fixtures. Cover text-layer and scanned or image-only
  pages, a page range beginning after page 1, an ambiguous mention, and tags and
  spans that are stable across a rebuild. Check matched quotes against the pinned
  text and map every sampled anchor back to the expected physical page. Report
  measured fixture results and any limits of heading or reading-order detection.

Keep OCR and model transcription in F14b/F14c. Tool definitions, dependencies,
existing tests and receipts stay unchanged. No model calls or network requests.
Record remaining view integration in the report and push to
`gemini/modes/inv-v17-pdf-skeleton` as directed in `RULES.md`.
