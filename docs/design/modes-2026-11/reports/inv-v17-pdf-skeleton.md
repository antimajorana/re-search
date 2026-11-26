# Report: `inv-v17-pdf-skeleton` (Investigate package V17 — PDF skeleton `pdf-1`)

- **Base commit:** `cf2544dc9c98718ab7b311e7a28609fee745062a` (`origin/gemini/work/fixed`)
- **Work branch:** `work/inv-v17-pdf-skeleton` (pushed to `gemini/modes/inv-v17-pdf-skeleton`)
- **Toolchain on this machine:** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `cargo 1.100.0 (3acc79c0c 2026-11-05)` on Linux x86_64
- **Guard rails:** `Cargo.lock` untouched (0 new dependencies); `crates/research-tools/tests/golden/tool_defs.txt` and `tool_profiles.txt` untouched; no tool definitions added or modified; `crates/policy` and `crates/ledger` untouched; TeX skeleton modules untouched and byte-identical; `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` passes (`No touched source file is over 600 lines.`).

---

## 1. What was implemented (Package V17 — `pdf-1`)

### Library modules (`crates/research-tools/src/skeleton/`)

| File | Lines | Status | Responsibility |
|---|---:|---|---|
| `pdf.rs` | 348 | new | Public `pdf-1` orchestrator: `PdfSkeletonInput`, `PdfSkeletonOutput`, `build_pdf_skeleton`, `build_pdf_skeleton_timed`, `build_pdf_skeleton_from_ingested`, page availability summaries, `Weak`/`Partial` quality classification (never `Full`), `quality_reason` with `PDF_QUALITY_REASON_BASE` and `"(scan)"` status, and strict `MAX_SKELETON_JSON_BYTES` (2 MiB) enforcement over both `nodes` and `bib`. |
| `pdf_honesty.rs` | 524 | new | Fail-closed `load_ingested_pdf_source` (verifying `source.ingested`, PDF status, `files` membership, `resolved.rel_path`, 64-hex SHA-256 digests, reading file hash, and monotonic/in-bounds `PageMap` / legacy `pdf_page_lines`) plus Rule 6.21 helpers: named constants (`PDF_QUALITY_REASON_BASE`, `PDF_QUALITY_SCAN_TAG`, `PDF_OCR_QUOTE_LABEL`, `PDF_MODEL_TRANSCRIPTION_LABEL`), `PdfTextOrigin`, `PdfPageAvailability`, `PdfQuoteDisplay`, dynamic physical page lookups (`page_range_for_span`, `page_range_for_node`, `node_page_basis`), `render_formulas_as_math` (`false` for `ScanTextLayer`/`None`), and `match_pdf_quote_for_display` (non-mutating, skips `PageBasis::None`, rejects ambiguous multi-page hits). |
| `pdf_scan.rs` | 561 | new | Structural state-machine scanner (`scan_pdf_lines`, `compute_pdf_body_fp`, `PdfProofHint`, `PdfScanOutput`): emits `FrontMatter`, nested `Section`s, theorem-like nodes, `Proof`s (with QED termination), `Figure`/`Table` captions, margin `Equation`s (with `min_eq_start_line` floor), and `Bibliography`; enforces bottom-up parent-span containment, splits spans around intervening `PageBasis::None` pages so no node claims an image-only page, computes whitespace-normalised 12-hex content fingerprints (`fp`, preserving `%`), and enforces `MAX_SKELETON_NODES` (6,000). |
| `pdf_sections.rs` | 529 | new | Numbered/APS/appendix/fixed section heading selection (`select_sections`): dotted Arabic (`1.`, `2.1`, `2.1.3`), APS Roman + letter subsections (`I.`, `II.A`), appendices (`Appendix A`, `A.1`), numbered `References`/`Bibliography` headings (`5. References`, `VI. REFERENCES`), page-range-after-page-1 support based on the first active non-`None` page, packed-list and active-only colon guards, and author-initial guards. |
| `pdf_patterns.rs` | 484 | new | Line-level matchers (`map_pdf_kind_word`, `match_theorem_heading`, `match_proof_heading`, `has_qed_marker`, `match_caption_heading`, `match_margin_equation`, `looks_math_line`, `is_display_math_candidate`) plus running header/footer and standalone page-number detection (`detect_headers_footers`, `compute_active_lines`) covering top-2/bottom-2 page numbers, alternating odd/even page-number headers (`"2   Title"` / `"Title   3"`), nested parentheses in theorem titles, flexible `[cite]`/`(Title)` ordering, EOL lowercase prose-wrap guards, and prose-vs-equation disambiguation before bottom `(N)`. |
| `pdf_bib.rs` | 449 | new | Numbered bibliography (`[1] ...` and `1. ...`) and body citation (`[12]`, `[1, 2, 5]`, `[1-3]`, `[12, Theorem 2.1]`) extraction: repairs line-wrapped DOIs/arXiv IDs (slashes, hyphens, and dot-splits) solely for ID scanning without mutating stored text; extracts canonical DOIs and arXiv IDs (including `10.48550/arXiv.*` and old-style `math/0501234`) via `tex_scan` and F2 `paper_key`; marks duplicate bibliography keys ambiguous; guards against math indexing (`f[1]`) and intervals (`x ∈ [1, 2]`, `[1, 2] × [3, 4]`); populates `BibRef::cited_at`, `BibRef::locators`, and `SkNode::head_cites`. |
| `pdf_edges.rs` | 563 | new | Heuristic edge construction (`build_pdf_edges`) and PDF reading stats (`compute_pdf_stats`): resolves unambiguous `(kind, printed_number)` targets only (ambiguous targets emit 0 edges and `proof_of = None`), emits prerequisite-to-dependent `Uses` (`TextMention` and `Lifted`), `Proves` (`ProofHeader` and `ProofAdjacent`, at most one per proof), and `Cites` (`HeadCite` and `Cite`) all with `EdgeConfidence::Heuristic` (out of confirmed skip plan), sets `intro_refs`, deduplicates in reading order, and enforces `MAX_SKELETON_EDGES` (20,000). |
| `pdf_tests.rs` | 484 | new | 9 unit tests covering footers/prose rejection, ambiguous numbers & bib keys, image-only page with stray footer, page range after page 1 + math bracket guards, CRLF/rebuild determinism, nested-paren titles + wrapped DOIs + citation ranges + plural mentions, intervening `PageBasis::None` span splitting + ambiguous multi-page quote refusal, alternating running headers + numbered `5. References` + `<` equations + EOL wrap guards, and `MAX_SKELETON_NODES` / `MAX_SKELETON_JSON_BYTES` (including huge `bib`) caps. |
| `mod.rs` | 267 (+15) | edited (isolated commit `3ba8846`) | Declares `pub mod pdf;` and private submodules `pdf_{bib,edges,honesty,patterns,scan,sections}` and re-exports the `pdf-1` public API. Zero changes to any TeX type or function. |

### Integration tests (`crates/research-tools/tests/`)

| File | Lines | Status | Coverage |
|---|---:|---|---|
| `skeleton_pdf.rs` | 376 | new | 3 end-to-end tests using pure-Rust self-authored PDFs (`pdf_support`): (1) 4-page text-layer math paper with running headers, printed footers (`- 101 -`, `- 102 -`, `3`, `(4)`), sections/subsections, Definition/Lemma/Theorem/Proofs (adjacent + detached), margin equations, Figure, citations with locators, DOI/arXiv bibliography, and quote verification via both `quote_core::verify_quote` and `match_pdf_quote_for_display`; (2) interleaved two-column APS physics PDF (`I.`, `II.`, `II.A`, `II.B`); (3) legacy pre-F14a PDF receipt with `pdf_page_lines` (unknown scan status noted). |
| `skeleton_pdf_honesty.rs` | 435 | new | 4 end-to-end tests: (1) mixed `ScanTextLayer` (`3 Tr`) + `PageBasis::None` image-only + `TextLayer` PDF plus all-image-only PDF verifying Rule 6.21 (`(scan)`, OCR display label, `render_math == false`, image availability, `ModelImageTranscription` non-anchor); (2) 6-page PDF opened with `with_page_range(4, 6)` starting after page 1 (sections 4 & 5, physical pages 4–6, footer 40–42 ignored); (3) ambiguous duplicate `Lemma 1` / `Eq. (1)` / `Proof of Lemma 1` + rebuild byte-identical stability + fail-closed reading-file tamper refusal; (4) fail-closed receipt validation (missing both `pdf_pages` and `pdf_page_lines`, missing `files` membership, out-of-bounds `pdf_page_lines`). |

---

## 2. Measured fixture results and known detection limits

### Measured CI fixture results (self-authored PDFs via `pdf_support`, debug build, Linux x86_64)

| Fixture test | Pages / bases | Nodes & structure measured | Quotes & physical pages | Quality / scan status |
|---|---|---|---|---|
| `multi_page_text_layer_pdf_skeleton_footers_quotes_and_anchors` | 4 pp (`TextLayer`), footers `-101-`, `-102-`, `3`, `(4)` | 4/4 sections (`1`, `2`, `2.1`, `3`), 1/1 Def, 1/1 Lem, 1/1 Thm (`head_cites: ["1"]`, `intro_refs: true`), 2/2 Proofs (both linked), 2/2 Eqs (footer `(4)` rejected), 1/1 Fig, 2/2 Bib (`arXiv` + `DOI` + locator) | 3/3 quotes matched via both `quote_core::verify_quote` and `match_pdf_quote_for_display`; 0 wrong pages | `partial`; no `(scan)` |
| `two_column_interleaved_physics_pdf_reading_order_and_aps_sections` | 1 p two-column interleaved (`TextLayer`) | 4/4 APS sections (`I`, `II`, `II.A`, `II.B`), 2/2 Eqs, 1/1 Bib (`DOI`), `Uses(Eq 1 -> II.B)` | 100% column reading order preserved | `partial`; no `(scan)` |
| `mixed_scan_text_layer_and_image_only_pages_enforce_rule_6_21` | 3 pp (`ScanTextLayer`, `None`, `TextLayer`) + 2 pp all-`None` | Mixed: 2/2 sections, 1/1 Thm (p.1), 1/1 Eq (p.1), 1/1 Cor (p.3), 0 nodes on image-only p.2. All-image: 0 nodes, `image_only_pages = [1, 2]` | Scan quote matched on p.1 with `"matched in OCR text, p. 1"`, `render_math = false`, `page_image_available = true`; 0 scans reported as `text_layer` | Mixed: `partial` with `(scan)`; All-image: `weak` with `(scan)` |
| `page_range_beginning_after_page_one_maps_physical_pages_and_sections` | 6-p PDF, range `4..=6` (`TextLayer`), footers `40`, `41`, `42` | 2/2 sections (`4`, `5`), 1/1 Thm, 1/1 Eq, 1/1 Proof, 1/1 Lem, 1/1 Bib (`[7]` + DOI) | All anchors mapped to physical pages `4`, `5`, `6` (0 shifted to `1..=3`, 0 footer matches) | `partial`; notes `extracted page range 4..=6 of 6` |
| `ambiguous_mentions_rebuild_stability_and_tamper_rejection` | 1 p (`TextLayer`) | 2 duplicate `Lemma 1`, 2 duplicate `Eq (1)`, 1 unique `Prop 3`, 1 `Thm 2`, 1 ambiguous `Proof of Lemma 1` | 0 edges for ambiguous `Lemma 1` / `Eq (1)`, `proof_of = None`, 1/1 edge for `Prop 3`; rebuild JSON byte-identical; tampered file rejected fail-closed | `partial` |

### Local `pdflatex` probe on self-authored V1 TeX fixtures (not committed, cleaned from `/tmp`)

To measure behavior on real LaTeX-compiled PDFs beyond `pdf_support`, two existing self-authored V1 fixtures were compiled locally with `pdflatex` and run through F14a `PdfExtractLayout` + `build_pdf_skeleton`:
- **`05_detached_proof/main.tex` -> 1-page PDF:** 11/11 nodes detected (100%: 3 sections, Lemma 1.1, Theorem 1.2, Corollary 1.3, Proposition 3.1, 4 proofs), 4/4 proof links resolved (3 detached `ProofHeader` + 1 attached `ProofAdjacent`), footer `"1"` filtered cleanly.
- **`01_amsthm_shared_counters/main.tex` -> 2-page PDF:** 11/12 nodes detected (FrontMatter, 2 sections, Definition 1.1, Equation (1), Theorem 2.1, Equation (2), 2 proofs including Proof of Theorem 2.1 across page 1 -> page 2, Corollary 2.2, Remark), footers `"1"` and `"2"` filtered cleanly. One node (`Lemma 1.2`) was missed because `01_amsthm_shared_counters/main.tex` references an undefined `\cite[Proposition 2.1]{keller2020}` inside the theorem heading, causing `pdflatex` to render a malformed unclosed parenthetical `Lemma 1.2 (Dirichlet form identity . keller2020]` without a closing `)`. That probe also revealed nested-parenthesis titles (e.g. `Theorem 2.1 (Bound on L^2(M)).`), which was subsequently fixed and covered by unit tests.

### Known limits of heading, boundary, and reading-order detection

1. **Theorem / proof end boundaries without blank lines or QED glyphs:** F14a's default `PdfExtractLayout` joins visual lines with single `\n` and does not emit blank lines between paragraphs on the same page. A theorem-like block not followed by `Proof.` or another heading/caption/section extends to the next structural boundary (matching Stage C' quality wording `"theorem boundaries may be wrong"`). Likewise, a proof whose QED box is drawn as a vector rectangle (no Unicode `□`/`∎` glyph) extends until the next heading/section.
2. **Multi-line theorem headings and unclosed parens:** Headings whose `(Title)` wraps across two physical lines or lacks a closing `)` are not matched by single-line `THM_RE`.
3. **Unnumbered prose-only papers and `\paragraph`-only papers:** Papers with no numbered/Roman/fixed headings yield only `FrontMatter` (or `Weak` quality) plus any margin equations and captions.
4. **Two-column layout limits inherited from F14a (`pdf_pages_layout.rs`):** Reading order relies on F14a's vertical gutter detector (`[0.35W, 0.65W]`); three-column layouts, rotated pages (`/Rotate`), floating footnotes interwoven with bottom columns, or wide displayed equations breaking column alignment can disturb reading order.
5. **Formula-heavy / unquoted bibliography entries:** `extract_pdf_bib_title` populates `BibRef::title` only for quoted titles (`"..."` / `“...”`) and leaves unquoted journal entries (such as APS `Phys. Rev. B 99, 010101`) with `title: None` rather than guessing. Unnumbered author-year bibliographies (without `[1]` or `1.` prefixes) are not split into `BibRef` items.
6. **Live set P6 not measured:** No real third-party journal PDFs or pre-1990 scans were accessed or stored in this unattended run.

---

## 3. Independent Carbon audit summary

An independent adversarial audit (`research` subagent on `pro` tier) reviewed all 11 touched/added files against `RULES.md`, `inv-v17-pdf-skeleton.md`, `00-overview.md` 2.12 & 7.2 P6, and `investigate.md` 3, 4.1–4.2, 6.21, V17. Every critical, high, and medium finding raised was fixed before committing:
- **C1:** Extended the 2 MiB `MAX_SKELETON_JSON_BYTES` truncation loop in `pdf.rs` to also truncate `skeleton.bib` (pruning affected `Cites` edges) so huge bibliographies cannot exceed 2 MiB; added stress test.
- **H1:** Split `SkNode::spans` around intervening `PageBasis::None` pages in `pdf_scan.rs` (preserving parent-span containment), made `page_range_for_span` skip `PageBasis::None`, updated `match_pdf_quote_for_display` to skip `PageBasis::None` hits (preventing shadowing) and reject multi-page ambiguous quotes, and hardened `load_ingested_pdf_source` (`status`, `files` membership, `resolved.rel_path`, 64-hex SHA-256 checks).
- **H2:** Expanded `detect_headers_footers` to top-2/bottom-2 standalone page numbers, normalized alternating odd/even running headers (`"2   Title"` / `"Title   3"`), tightened `is_display_math_candidate` / `looks_like_plain_prose_sentence` so prose like `"We set N = 4..."` above `"(12)"` never creates an equation, and added `<`, `>`, `·`, `∞`, `∼`, `∥`, `∀`, `∃`, `||` to `looks_math_line`.
- **H3:** Computed `starts_after_page_one` from the first non-`None` page, constrained subsection `valid_step` after `l1 > 0`, recognized numbered `References`/`Bibliography` headings (`5. References`, `VI. REFERENCES`), gated dotted `A.1` subsections on active appendix context, filtered `preceded_by_list_colon` to active lines, and added author-initial guards on `LETTER_RE`.
- **H4:** Added `min_eq_start_line` floor in `pdf_scan.rs` so standalone `(N)` equations cannot bleed backward onto headings, captions, or QED-closed proofs.
- **H5:** Made `load_ingested_pdf_source` fail closed on pre-F14a receipts lacking both `pdf_pages` and `pdf_page_lines` (while constructing all-`None` `PageMap` cleanly for `"empty_text_layer"` receipts).
- **M1–M5 / L1–L2:** Added wrapped dot-split DOI joining (`10.1103/PhysRevB.99.\n010101`), interval suffix/prefix guards (`[1, 2] × [3, 4]`), `Observation`/`Hypothesis`/`Axiom`/`Appendix` mentions and `|[A-Z]` in `EXTRA_NUM_RE`, EOL lowercase prose-wrap guards on bare `Theorem N` / `Proof`, flexible `[cite]`/`(Title)` ordering, joint-proof recognition (`target: None`), `quote_core::clip_to_chars` reuse, and dead-variable cleanup.

---

## 4. Decisions taken alone, deviations, and assumptions

### Decisions
1. **PDF body fingerprint (`compute_pdf_body_fp`):** Uses SHA-256 over whitespace-normalised text (12 lowercase hex chars, invariant to CRLF/LF and `\x0c`) **without** LaTeX `%` comment stripping, because `%` is literal content in PDF text (e.g., `"50% bound"`). Identical to `compute_body_fp` whenever `%` is absent.
2. **All PDF edges carry `EdgeConfidence::Heuristic`:** `Uses` (`TextMention`, `Lifted`), `Proves` (`ProofHeader`, `ProofAdjacent`), and `Cites` (`HeadCite`, `Cite`) all use `EdgeConfidence::Heuristic` so no heuristic PDF link enters the confirmed skip plan (`investigate.md` 4.1, 6.14).
3. **Ambiguous quote locations refused in `match_pdf_quote_for_display`:** If a query matches on more than one distinct non-`None` physical page, `match_pdf_quote_for_display` returns `None` rather than guessing the first page.
4. **Pure-Rust PDF fixtures in CI (`pdf_support`):** All committed tests generate self-authored PDF 1.4 documents in pure Rust via `tests/pdf_support/mod.rs` (same approach as F14a), avoiding Windows/Linux CI dependence on `pdflatex` or committed binary PDFs.
5. **Named UI/status constants (`RULES.md`):** Placed `PDF_QUALITY_REASON_BASE`, `PDF_QUALITY_SCAN_TAG`, `PDF_OCR_QUOTE_LABEL`, and `PDF_MODEL_TRANSCRIPTION_LABEL` in `skeleton/pdf_honesty.rs` (re-exported from `skeleton::pdf` and `skeleton`).

### Deviations from the design
- Split `skeleton/pdf.rs` across `pdf.rs`, `pdf_honesty.rs`, `pdf_scan.rs`, `pdf_sections.rs`, `pdf_patterns.rs`, `pdf_bib.rs`, `pdf_edges.rs`, and `pdf_tests.rs` (all `<= 563` lines) to keep every file strictly under the 600-line limit. All public items are re-exported from `skeleton::pdf` and `skeleton`.
- Did not commit compiled binary PDFs or rasterized page-image PDFs generated from `pdflatex` (used pure-Rust `pdf_support` fixtures in CI tests and ran `pdflatex` locally as a probe on `01_amsthm_shared_counters` and `05_detached_proof`).

### Assumptions
- F14a `source.ingested` PDF receipts always carry `page_count` and either `pdf_pages` (F14a) or `pdf_page_lines` (pre-F14a with `pdf_page_lines`), or `status: "empty_text_layer"`. Older pre-F14a receipts lacking both `pdf_pages` and `pdf_page_lines` require the caller to reconstruct verified ranges from `raw_blob` via `pdf::verified_page_lines` before constructing `PdfSkeletonInput`.
- Original raw PDF bytes remain accessible via `source.ingested.raw_blob` (and desktop server's original-PDF page viewer) so `PdfPageAvailability::image_available` is `true` for every physical page in `PageMap`.

---

## 5. Merge notes & V3/V4 view/open-route integration hooks

### Edits to existing files
- **`crates/research-tools/src/skeleton/mod.rs`** (commit `3ba8846`, +15 lines): added `pub mod pdf; mod pdf_{bib,edges,honesty,patterns,scan,sections};` and `pub use pdf::{...};`. No other existing file was edited.

### Precise hooks for V3 (`desktop-server/src/investigation_skeleton.rs`) and V4 (`investigation_open.rs` + UI)
1. **Dispatch in `investigation_skeleton.rs` (V3):**
   - Given a pinned `source.ingested` `Event`:
     - If TeX source (`format` is `"tex"` / archive with `.tex` files and `status != "pdf_only"`): call `skeleton::build_tex_skeleton_from_ingested(...)` (unchanged).
     - If PDF source (`format == "pdf"` || `status == "pdf_only"` || `payload["pdf_extractor"].is_string()` || `payload["pdf_pages"].is_array()`): call
       ```rust
       research_tools::skeleton::build_pdf_skeleton_from_ingested(
           &sandbox,
           session_id,
           receipt,
           paper_key,
           version,
       )
       ```
       Store `output.skeleton` JSON via `Ledger::put_blob` and append `investigation.skeleton` with `builder: "pdf-1"`, `quality`, `quality_reason`, and blob hash — identical to TeX!
     - If a pre-F14a PDF receipt lacks both `pdf_pages` and `pdf_page_lines`, reconstruct verified page lines first via `research_tools::pdf::verified_page_lines(&receipt.payload, || ledger.get_blob(raw_blob))` and call `skeleton::build_pdf_skeleton_timed(&PdfSkeletonInput { ... })`.
2. **Page links, quality line, OCR badge, and image-only pages in mapview / node card (V3/V4 UI):**
   - **Never store pages on `SkNode`:** compute each node's physical page range on the fly when serializing the mapview response using `skeleton::page_range_for_node(&page_map, node)` and `skeleton::node_page_basis(&page_map, node)`.
   - **Always display `skeleton.quality_reason`** (which starts with `PDF_QUALITY_REASON_BASE` and includes `"(scan)"` when any page is scanned/image-only).
   - **Formulas & OCR quotes (rule 6.21):** Use `skeleton::render_formulas_as_math(basis)` (`false` for `ScanTextLayer`/`None`) to disable KaTeX/math rendering on OCR nodes/quotes; use `skeleton::match_pdf_quote_for_display(primary_path, &stored_text, &page_map, query)` to emit `"matched in OCR text, p. N"` with a link to the original PDF page image endpoint.
   - **Image-only pages:** Expose `output.image_only_pages` / `output.pages` (`PdfPageAvailability`) in the mapview/open response so image-only pages remain clickable as page images while awaiting F14b OCR.
   - **Background extraction (V4):** After `pdf_pages::PdfExtractJob` completes, pin via `ingest::ingest_pdf_pages(...)` and immediately run `build_pdf_skeleton_from_ingested(...)` on the resulting `source.ingested` receipt.
3. **How to test the V3/V4 integration once merged:**
   - Ingest a 3-page mixed PDF (`ScanTextLayer`, `PageBasis::None`, `TextLayer`) via `ingest_bytes` (or `PdfExtractJob` + `ingest_pdf_pages`), invoke the V3 skeleton builder route, and assert: (a) `investigation.skeleton` event records `builder == "pdf-1"`, `quality != "full"`, `quality_reason` contains `"(scan)"`; (b) mapview JSON attaches physical page `1` (`basis: "scan_text_layer"`, `render_math: false`) to page-1 nodes, page `3` to page-3 nodes, and lists page `2` in `image_only_pages`; (c) zero `model.request` events occur during open/skeleton build (rule 6.1).

---

## 6. Verification, test counts, and pre-existing findings

### Commands run and exact results

| Command | Result |
|---|---|
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | **Pass** — `No touched source file is over 600 lines.` |
| `git diff --stat origin/gemini/work/fixed -- Cargo.lock crates/research-tools/tests/golden/ crates/policy crates/ledger` | **Pass** — empty diff |
| `cargo test --locked -p research-tools --lib skeleton::pdf` | **Pass** — **9 passed, 0 failed, 0 ignored** |
| `cargo test --locked -p research-tools --test skeleton_pdf --test skeleton_pdf_honesty` | **Pass** — **7 passed, 0 failed, 0 ignored** (`3` in `skeleton_pdf`, `4` in `skeleton_pdf_honesty`) |
| `cargo test --locked -p research-tools --test skeleton_tex --test skeleton_v2 --test skeleton_props --test tool_def_golden --test tool_profile_golden` | **Pass** — **26 passed, 0 failed, 0 ignored** (`12` + `4` + `4` + `3` + `3`) |
| `cargo clippy --locked -p research-tools --lib --test skeleton_pdf --test skeleton_pdf_honesty --test skeleton_tex --test skeleton_v2 --test skeleton_props --test tool_def_golden -- -D warnings` | **Pass** — **0 warnings, 0 errors** |
| `cargo clippy --locked --workspace --all-targets --no-deps` | **6 pre-existing warnings only** (0 warnings in any touched/added file; see below) |
| `cargo test --locked --workspace --no-fail-fast` | **Pass** — **866 passed, 0 failed, 2 ignored** across 109 test/doc-test targets (the 2 ignored are pre-existing live-evaluation tests in `crates/agent/tests/eval_harness.rs`) |
| `RE_SEARCH_PYTHON="$(command -v python3)" cargo test --locked -p export --no-fail-fast` | **Pass** — **8 passed, 0 failed, 0 ignored** |

### Pre-existing clippy warnings (unchanged from base commit `cf2544dc9c98718ab7b311e7a28609fee745062a`)

Running `cargo clippy --locked --workspace --all-targets --no-deps` under `rustc 1.100.0` produces only the exact 6 pre-existing warnings already documented in `RULES.md` and `reports/f14a-pdf-core.md`:
1. `crates/desktop-server/src/evidence.rs:744` — `clippy::manual_unwrap_or` (documented in `RULES.md`).
2. `crates/agent/tests/eval_support/manifest.rs:289` — `clippy::match_like_matches_macro`.
3. `crates/agent/tests/eval_support/sheet.rs:102` — `clippy::unnecessary_map_or`.
4. `crates/agent/src/guardrails.rs:335` — `clippy::items_after_test_module`.
5. `crates/research-tools/tests/sandbox_and_fs.rs:6,9` — unused imports on Linux (`TEX_AUDIT`, `is_symlink_or_junction`).
6. `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` — unused imports on Linux (`BufRead`, `BufReader`).

### What could not be compiled or run on Linux
- `#[cfg(windows)]` code paths and tests (`crates/desktop-app`, `crates/worker` AppContainer/Python worker, `crates/chatgpt-auth` DPAPI, `crates/research-tools/src/sandbox.rs` NTFS junctions, Windows Playwright browser tests). `skeleton/pdf*.rs` itself is 100% pure Rust with no platform-specific paths or shell-outs.
- Live external PDFs (evaluation set P6).

---

## 7. Bugs noticed (not fixed), design notes, and what does not work yet

- **Existing note in `crates/research-tools/src/pdf_match.rs:194` (F14a, untouched):** `match_pdf_quote` uses a single `.find(&needle)` on normalized text, so callers needing disambiguation across multiple pages or skipping `PageBasis::None` hits should call `skeleton::match_pdf_quote_for_display` (which scans all occurrences, skips `PageBasis::None`, and rejects ambiguous multi-page matches).
- **Existing note in `crates/research-tools/src/pdf_pages_layout.rs:294` (F14a, untouched):** `LayoutOutputDev` emits single newlines between all visual lines (no blank paragraph lines) and joins horizontally separated pieces on the same `y`-band with a single space. `skeleton/pdf_patterns.rs` accounts for this, though theorem end boundaries without a following `Proof.` or heading remain coarse estimates.
- **Design contradictions found:** None. The interface between F14a (`pdf_pages`, `pdf_match`, `ingest`), V2 (`skeleton::{model, tags, stats}`), and V17 (`skeleton/pdf.rs`) aligns cleanly.
- **What does not work yet (out of V17 scope):** OCR for `PageBasis::None` pages (F14b), model formula transcription from page images (F14c), V3/V4 desktop-server route wiring and UI rendering (depends on V3/V4 merging; hooks documented in Section 5), and live P6 evaluation on real journal PDFs and pre-1990 scans.
