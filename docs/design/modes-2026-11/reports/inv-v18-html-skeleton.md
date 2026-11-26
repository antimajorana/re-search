# Inv-V18 Report — arXiv HTML5 / LaTeXML Skeleton & Number Confirmation

**Brief:** `docs/design/modes-2026-11/agents/inv-v18-html-skeleton.md`
**Base commit:** `2da21b7` (`origin/gemini/work/fixed`)
**Branch:** `gemini/modes/inv-v18-html-skeleton`
**Checkpoint 1 commit:** `e900734` (`inv-v18: add pure-Rust LaTeXML HTML5 skeleton builder (html-1)`)

---

## 1. Summary of Implemented Packages

Implemented Package `V18` with **zero new external crate dependencies**, **zero edits to `Cargo.lock` or forbidden files**, and **zero third-party paper text** in code or tests:

### Checkpoint 1 — `research-tools` Pure HTML5 / LaTeXML Skeleton Builder (`html-1`)
- `crates/research-tools/src/skeleton/html_entities.rs` (`116` lines): bounded pure-Rust decoder for HTML5 named math/typography character references (`&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;`, `&nbsp;`, inequalities, arrows, operators) and decimal/hex numeric entities (`&#...;`, `&#x...;`) with invalid codepoint sanitization.
- `crates/research-tools/src/skeleton/html_lex.rs` (`521` lines): bounded zero-dependency HTML5 lexer (`MAX_HTML_BYTES = 16 MiB`, `MAX_HTML_TOKENS = 500_000`, `MAX_HTML_ATTRS_PER_TAG = 128`) tracking exact 1-based physical source line numbers (`LineIndex`), handling void tags (`<br>`, `<hr>`, `<img>`, `<meta>`, `<link>`, `<input>`, etc.), unquoted/single/double-quoted attributes, HTML comments, CDATA, `<script>`/`<style>` raw-text isolation, RCDATA `<title>`, and unescaped inline-math `<` characters (`a < b`).
- `crates/research-tools/src/skeleton/html_classify.rs` (`253` lines): `is_latexml_html_document` detector (requiring `ltx_*` structural classes outside `<script>`/`<style>`/comments or `<meta name="generator" content="...LaTeXML...">`, rejecting NUL bytes and arXiv error pages cleanly), section/structural node classifiers, heading/tag/title extraction helpers, equation `(tag)` stripping (`"(1.1)" -> "1.1"`), and normalized body fingerprinting (`compute_html_body_fp`).
- `crates/research-tools/src/skeleton/html_walk.rs` (`539` lines) & `crates/research-tools/src/skeleton/html_finalize.rs` (`221` lines): bounded stack DOM walker (`MAX_HTML_DEPTH = 128`) with unclosed sibling/EOF tag recovery (`SkeletonQuality::Partial`), `<math alttext="...">` capture without duplicating `<annotation>`/`<annotation-xml>` children, equation & `ltx_equationgroup` handling (emitting `SkNodeKind::Equation` only when numbered/labelled and counting unlabelled equations in `eq_count`), citation/ref/bibliography span capture, proof header & adjacent proof tracking, and `MAX_SKELETON_NODES` (`6_000`) enforcement.
- `crates/research-tools/src/skeleton/html_edges.rs` (`412` lines) & `crates/research-tools/src/skeleton/html_bib.rs` (`347` lines): rule 6.2 unique `Proves` edges, explicit `Ref` / `Eqref` and heuristic `TextMention` `Uses` edges with theorem/proof lifting and `intro_refs` marking, `BibRef` arXiv/DOI/title/locator extraction, `MAX_SKELETON_EDGES` (`20_000`) enforcement, and `compute_html_stats`.
- `crates/research-tools/src/skeleton/html.rs` (`347` lines): `IngestedHtmlSource`, `HtmlSkeletonInput`, `HtmlSkeletonOutput`, `is_html_receipt`, `load_ingested_html_source` (sandbox-resolved + SHA-256 verified), `build_html_skeleton`, `build_html_skeleton_timed`, and `build_html_skeleton_from_ingested` (producing `SkeletonBuilder::Html1`, `SkConfidence::HtmlClass`, `NumberBasis::Html`, permanent V2 `n-...` tags via `tags::assign_tags`, and `MAX_SKELETON_JSON_BYTES` (`2 MiB`) truncation).
- `crates/research-tools/src/ingest.rs` (`590` lines): added `"html"`, `"htm"`, `"xhtml"` to `TEXT_EXTENSIONS` so `ingest_bytes` accepts ingested HTML5 papers without altering any MCP tool schema or golden hash.
- `crates/research-tools/src/skeleton/mod.rs` (`311` lines): exposed `pub mod html`, `html_lex::MAX_HTML_BYTES`, and public builders/loaders.
- Synthetic fixtures & tests:
  - `crates/research-tools/tests/fixtures/skeleton_html/01_latexml_paper.html` (`96` lines)
  - `crates/research-tools/tests/fixtures/skeleton_html/02_malformed_recovery.html` (`14` lines)
  - `crates/research-tools/tests/fixtures/skeleton_html/03_not_latexml_error.html` (`13` lines)
  - `crates/research-tools/tests/skeleton_html.rs` (`432` lines)

### Checkpoint 2 — Number Cross-Confirmation & F3 Fallback in `desktop-server`
- `crates/research-tools/src/skeleton/html_confirm.rs` (`287` lines, exported in `skeleton/mod.rs`): `confirm_tex_numbers_with_html`, `ConfirmReport`, and `NumberDisagreement`. Matches TeX nodes to `html-1` nodes conservatively (unique explicit `label`, unique `(kind, fp)`, or unique normalized `(kind, title)`), upgrades `NumberBasis::Computed -> NumberBasis::Confirmed` ONLY on verbatim trimmed number equality when HTML node carries `NumberBasis::Html`, and records mismatches without overwriting TeX numbers.
- `crates/desktop-server/src/investigation_open_html.rs` (`428` lines, exported in `crates/desktop-server/src/lib.rs` at `240` lines): implements the arXiv fallback cascade (`LibraryLookup` cache -> `EprintTex` -> `HtmlFallback` on `<arxiv_eprint_base>/html/<target_id>` -> `PdfFallback`) strictly through `fetch_response_with_gate(..., Some(Service::Arxiv), gate)`. Rejects HTTP non-200 or non-LaTeXML HTML cleanly so PDF fallback still runs.
- `crates/desktop-server/src/investigation_open.rs` (`522` lines): added `OpenStep::HtmlFallback`, delegated Step 2 source fetch cleanly to `investigation_open_html::fetch_source_with_html_fallback`.
- `crates/desktop-server/src/investigation_skeleton.rs` (`543` lines): thin dispatch hook adding `is_html_receipt` in `is_skeleton_receipt` and `build_html_skeleton_from_ingested` in `build_skeleton_from_receipt`.
- `crates/desktop-server/src/investigation_mapview.rs` (`588` lines): thin 6-line dispatch hook in `read_verified_node_text` supporting `SkeletonBuilder::Html1` via `load_ingested_html_source`.
- `crates/desktop-server/tests/investigation_open_html.rs` (`424` lines): loopback `RoutedMock` + `FakeRateClock` integration tests covering missing-TeX -> `/html/<id>` via `Service::Arxiv` -> `html-1` + `mapview` + `node-text`; `/html/` 404 and non-LaTeXML 200 HTML clean fallthrough to `pdf-1`; and non-arXiv DOI + `FulltextCache` preservation with zero live network calls or model requests.

---

## 2. Decisions & Inferred LaTeXML Assumptions

1. **Decision 12 (Zero External HTML Dependencies):** Built a small bounded pure-Rust scanner/DOM walker (`html_entities`, `html_lex`, `html_walk`, `html_finalize`). Fixtures exercise well-formed LaTeXML structures, nested `<math alttext="...">` + `<annotation>`, equation groups, citations/bib items, and malformed HTML5 unclosed/void/unquoted/stray-`<` recovery without needing `html5ever`.
2. **Inferred LaTeXML Class Conventions:** Documented in `skeleton/html.rs`:
   - Sections/front matter/bibliography: `ltx_part`, `ltx_chapter`, `ltx_section`, `ltx_appendix`, `ltx_subsection`, `ltx_subsubsection`, `ltx_abstract`, `ltx_bibliography`, `ltx_bibitem`.
   - Theorem-like & proof blocks: `ltx_theorem_<kind>`, `ltx_theorem` (with tag-prefix inference), `ltx_algorithm`, `ltx_proof`.
   - Equations: outer `ltx_equation` / `ltx_equationgroup` (nested `<tr class="ltx_equation">` inside outer `PendingEquation` inherits rather than double-emits; unnumbered display equations increment `eq_count` once on parent).
   - Headings & tags: `ltx_title*`, `ltx_caption`, `ltx_tag*`, `ltx_eqn_eqno`, `ltx_ref`, `ltx_cite`.
3. **FulltextCache & Frozen `paper_fetch.rs` Isolation:** Because `research-tools/src/paper_fetch.rs` is frozen and its `cached_file_name` maps unknown format keys (other than `"pdf"`, `"tex"`, `"tar.gz"`, `"zip"`) to `.gz`, `investigation_open_html.rs` does NOT write raw `"html"` bytes into `FulltextCache`. Ingested HTML enters the session cleanly via `ingest_bytes` (`source.ingested`, `format: "html"`), while e-print TeX and fallback PDF responses continue to populate `FulltextCache`.

---

## 3. Deviations From Brief & Merge Notes

- **`investigation_mapview.rs` thin hook (`588` lines):** The older markdown brief (`agents/inv-v18-html-skeleton.md`) listed `investigation_mapview.rs` in Do-Not-Edit, whereas the updated execution instructions specifically authorized a thin hook in `investigation_mapview::read_verified_node_text` while keeping `investigation_mapview.rs <= 600` lines and excluding `investigation_mapview.rs` from the forbidden list. Adding the 6-line `SkeletonBuilder::Html1` branch brought `investigation_mapview.rs` from `582` to `588` lines (`<= 600`), allowing `GET /investigation/node-text` to read verified HTML node/proof text directly.
- **`ingest.rs` extension list (`590` lines):** Added `"html"`, `"htm"`, `"xhtml"` to `TEXT_EXTENSIONS` in `crates/research-tools/src/ingest.rs` so `ingest_bytes` ingests downloaded `arxiv_*.html` files without failing format detection.
- **Module splitting for `<= 600`-line compliance:** Split the HTML scanner and walker across small focused modules (`html_entities`, `html_lex`, `html_classify`, `html_walk`, `html_finalize`, `html_edges`, `html_bib`, `html`, `html_confirm`), keeping all touched files between `116` and `590` lines.
- **Merge note for `FulltextCache` + `paper_fetch.rs`:** If cross-session caching of `/html/<id>` downloads in `FulltextCache` is desired later, add `"html" => "html"` to `paper_fetch::cached_file_name` and call `cache_put(..., "html", ...)` in `investigation_open_html.rs`.

---

## 4. Known Limitations & Items Not Changed

- **Single-file HTML ingestion:** `load_ingested_html_source` verifies and reads `primary_path`; multi-chapter split HTML trees or external image/CSS assets are not fetched or assembled.
- **`<math>` without `alttext`:** Falls back to concatenated MathML child character text rather than reconstructing TeX markup.
- **Standalone HTML receipt persistence in `FulltextCache`:** Omitted as noted in Section 3 to avoid changing frozen `paper_fetch.rs`.
- **Two live-eval tests remain `#[ignore]` pre-existing:** `file_sheet_from_json` (`live tier: needs EVAL_SHEET_JSON`) and `live_manifests_validate` (`live tier: needs EVAL_LIVE_MANIFEST_DIR`).

---

## 5. Verification Commands & Observed Results

Executed on Windows (`check_file_length.py`) and non-root WSL2 Ubuntu (`uid=1000`, `RE_SEARCH_PYTHON` unset, `flock /var/tmp/research-cargo.lock`):

1. `py -3 scripts/check_file_length.py --base origin/gemini/work/fixed` -> `No touched source file is over 600 lines.` (exit `0`; max touched file `590` lines).
2. `cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` -> exit `0` (`0` warnings/errors).
3. `cargo test --workspace --locked --no-fail-fast -j 2` -> exit `0`:
   - **Workspace totals:** `138` test suites run, `1128` passed, `0` failed, `2` ignored (pre-existing live-eval gated tests).
   - **Notable V18 suites:**
     - `research-tools` lib unittests: `311 passed; 0 failed; 0 ignored` (includes `skeleton::html_lex::tests::lexer_handles_void_unquoted_comments_scripts_entities_and_stray_lt` and `skeleton::html_confirm::tests::confirms_verbatim_matches_and_never_overwrites_mismatches`)
     - `research-tools` `tests/skeleton_html.rs`: `5 passed; 0 failed; 0 ignored`
     - `desktop-server` `tests/investigation_open_html.rs`: `3 passed; 0 failed; 0 ignored`
     - `desktop-server` `tests/investigation_open.rs`: `4 passed; 0 failed; 0 ignored`
     - `desktop-server` `tests/investigation_v3_routes.rs`: `2 passed; 0 failed; 0 ignored`
     - `research-tools` `tests/tool_def_golden.rs` (`3 passed`) & `tests/tool_profile_golden.rs` (`3 passed`)
