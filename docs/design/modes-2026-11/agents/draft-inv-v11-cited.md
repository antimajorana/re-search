# Brief draft-inv-v11-cited — Investigate Cited Result Without a Model (V11)

## 1. Packages and governing design sections

- **Package:** **V11** (Cited result without a model: resolve a citation chip `[bib_key, locator]` to a `PaperRef`, reuse the local F10 library or fetch via F3 under the per-session fetch budget, reuse/cache the cited paper's skeleton in F10, locate the cited statement strictly under rule 6.9, handle PDF-only cited papers via V17 with page-linked candidates, and open a focused Investigate view of the cited paper at the located node with a return link to the original claim).
- **Governing design:**
  - `00-overview.md`: 2.3 (F2 `PaperKey`, `PaperRef`, `NodeRef`), 2.4 (F3 network gate, `paper_fetch`, user-initiated approval rule, session fetch budget), 2.5 (F4 `verify_quote`), 2.7 (F10 library `skeleton` cache, `seen.role = cited_lookup`), 2.10 (evidence scale: `"Outside this paper"`), 2.12 (F14a PDF text & page links), 7.2–7.3 (Evaluation Set P5: **wrong statement shown as located MUST be 0**), 8.1–8.2 (accepted decisions).
  - `investigate.md`: 0.2 #1 (do not show unconfirmed computed theorem numbers as fact), 2 (worked example 2:30), 3 ("Follow the argument" cited-result flow, stage G, provenance styles), 4.1 (`BibRef`, `NumberBasis`), 4.3 (`Cited lookup` record), 4.4 (`investigation.cited_lookup`), 5.2 (`POST .../investigation/cited`), 5.3 (0 model calls), rules 6.9, 6.16, 6.21, sections 7 & 9 (row V11), section 13 (decisions 6 and 8).
- **Owner decisions (2026-11-25) applied:**
  - **Zero model calls.** Purely deterministic resolution, fetch, skeleton build/cache, and locator matching.
  - **PDF-only papers are essential:** when the cited paper exists only as PDF (from library or arXiv), run lookup on its V17 PDF skeleton (`pdf-1`) and return `candidates` with physical page links — **never** `found` (because PDF statement boundaries are estimated, rule 6.9).
  - **Fetch budget (`investigate.md` 6.16, 13 #6):** default 8 fetched external sources per session (`paper_fetch::DEFAULT_INVESTIGATE_FETCH_LIMIT`), visible counter, raised only by an explicit user click recorded in the ledger.

## 2. Start conditions

- **Required merges on `gemini/work/fixed` before coding:**
  - Already merged on base `832eab5bf`: **F0, F1, F2, F3, F4, F6, F8, F9, F10, F14a, V1**.
  - Must be merged before starting V11: **V2, V3, V4** (`inv-a-v2-v5`, for `skeleton/bib.rs`, `skeleton/number.rs`, `investigation_skeleton.rs`, `investigation_open.rs`), **F5** (`f5-f7-mode-shell`, for `api_dispatch.rs`), and **V17** (`inv-v17-pdf-skeleton`, for the PDF-only cited-paper path; V11 Checkpoint 1–2 can start on TeX once V4 merges if V17 is still in flight, then wire V17 in Checkpoint 3).
  - Optional synergy: **F13a** (`lib-f13a-f16a-f15`) — once merged, user-library files registered in F10 resolve locally with zero network.
- **Owner gates:** None (decisions 6 and 8 accepted; Set P5 fixed-tier fixtures are self-authored).

## 3. Base-commit verified interfaces vs future-merge interfaces

### Verified on base `832eab5bf`
- **F2 (`crates/research-tools/src/paper_key.rs`):** `PaperKey`, `PaperIds`, `PaperRef::from_ids_or`, `NodeRef`, `Mode::Investigate`, `split_arxiv_id`, `canonical_doi`.
- **F3 (`crates/research-tools/src/paper_fetch.rs`):** `fetch` / `fetch_with_gate`, `FetchWant::{Metadata, Source}`, `FetchOutcome::{Source, NeedsFile}`, `STATUS_NEEDS_FILE` (`"needs_file"`), `FetchedSource::ingest_into_session`, `fetched_sources(events)`, `check_fetch_budget(events, limit)`, `DEFAULT_INVESTIGATE_FETCH_LIMIT` (8). Doc comment enforces user-initiated-only unapproved network rule (`POST .../investigation/cited` is triggered directly by clicking a citation chip).
- **F10 (`crates/research-tools/src/library.rs`, `library_schema.rs`):** `Library::open`, `resolve_key`, `read_fulltext_bytes(key, version)`, `upsert_skeleton`, `get_skeleton(key, version, builder)`, `record_seen` with `SeenRole::CitedLookup`.
- **F14a (`crates/research-tools/src/pdf_pages.rs`):** `PageMap::page_of(&self, line)` (`pdf_pages.rs:106`, when a full `PageMap` is loaded), standalone `page_of_line(ranges, line)` (`pdf_pages.rs:150`, over `source.ingested`'s `pdf_page_lines` ranges), and `PageBasis` (for OCR tagging on scanned cited PDFs).
- **V1 (`crates/research-tools/src/skeleton/model.rs`):** `PaperSkeleton`, `SkNode`, `NumberBasis::{Computed, Html, Printed, Confirmed}`, `BibRef`, `BibLocator`, `SkeletonBuilder::{Tex1, Html1, Pdf1}`.
- **F6 (`crates/desktop-server/assets/`):** `ui_badge.js` (`PROVENANCE_STYLES.outside_paper`, `createNodeCardShell`, `createBadge`), `quote_chip.js` (`createQuoteChip` with `page` and `ocr` support).

### Future-merge interfaces to verify before coding (not yet in `832eab5bf`)
- **V2 (`inv-a-v2-v5`):** `skeleton/bib.rs` (`BibRef` extraction of `arxiv_id`, `doi`, `title`, `locators`) and `skeleton/number.rs` (single-shared-counter flag / `NumberBasis` rules).
- **V3/V4 (`inv-a-v2-v5`):** `desktop-server/src/investigation_skeleton.rs` (building/loading `PaperSkeleton` blobs) and `investigation_open.rs` (opening a paper into Investigate; used by the optional "Open full paper in new Investigate session" button).
- **V17 (`inv-v17-pdf-skeleton`):** `skeleton/pdf.rs` entry point building `SkeletonBuilder::Pdf1` from an ingested PDF receipt.
- **F5 (`f5-f7-mode-shell`):** `desktop-server/src/api_dispatch.rs`.

## 4. Smallest implementation checkpoints

1. **Checkpoint 1 — Pure locator engine (`crates/research-tools/src/skeleton/locate.rs`):**
   - Parse citation locators (`"Theorem 2.1"`, `"Thm. 3"`, `"Lemma 4.2"`, `"Prop. A.1"`, `"Corollary 1"`, `"Definition 2"`, `"Eq. (3.4)"`, `"(2.7)"`, `"Section 3"`, etc.) into normalized `(Option<SkNodeKind>, Option<String> number, raw_locator)`.
   - Implement rule 6.9 strictly (`locate_in_skeleton(skeleton, locator) -> LocateResult`):
     - Return `LocateStatus::Found { tag, number_basis }` **ONLY** when the skeleton is non-PDF (`Tex1` or `Html1`), exactly one node matches kind + number, AND `number_basis` is confirmed: `NumberBasis::Confirmed`, `NumberBasis::Html`, `NumberBasis::Printed` (in TeX/HTML, e.g. `\tag{...}`), OR `NumberBasis::Computed` when the skeleton proves it is the unique node of that kind with that number under a single shared counter (no ambiguous unshared/custom counter drift).
     - Return `LocateStatus::Candidates { candidates: Vec<LocateCandidate>, reason }` whenever the number is unconfirmed (`Computed` with possible counter drift, multiple matches, kind mismatch, or **any** `Pdf1` skeleton — carrying physical page numbers via `PageMap::page_of(&self, line)` or `page_of_line(ranges, line)` for PDFs).
     - Return `LocateStatus::PaperFoundLocatorNotFound` when the skeleton has nodes but none match the locator (still returning theorem-like nodes so the user can pick manually).
     - Return `LocateStatus::NoTex { reason }` (serialized as `status: "no_tex"`, carrying `reason: paper_fetch::STATUS_NEEDS_FILE` i.e. `"needs_file"` when paywalled/missing source, or `"empty_text_layer"` for image-only scans without OCR) when no usable TeX/HTML/PDF skeleton could be built.
     - Return `LocateStatus::Unresolved` when the `BibRef` has no resolvable identifier (`arxiv_id`, `doi`, or library match).
2. **Checkpoint 2 — Desktop route & F10 skeleton caching (`crates/desktop-server/src/investigation_cited.rs`):**
   - Implement a **single** route `POST /api/sessions/{id}/investigation/cited` (1 arm in `api_dispatch.rs`) dispatched by JSON `"action"`:
     - `"lookup"` (default when omitted): `{ tag, bib_key, locator }` — deterministic cited-result lookup.
     - `"pick"`: `{ tag, bib_key, locator, external_source_id, external_tag }` — records the user's explicit candidate selection into `investigation.cited_lookup`.
     - `"raise_budget"`: `{ new_limit }` — records the user's explicit click as an `investigation.note` event raising the per-session fetch budget (rule 6.16).
   - Resolution order for `"lookup"` (zero model calls):
     1. Validate session is Investigate, pinned skeleton exists, `tag` and `bib_key` exist in pinned skeleton.
     2. Resolve `BibRef` -> `PaperRef` (using `arxiv_id` / `doi` / library alias lookup; if DOI-only, let F3 `paper_fetch` try library first, then DOI->arXiv via F3's gated resolvers).
     3. Check F10 library (`Library::read_fulltext_bytes` and `Library::get_skeleton`) first — **zero network** on hit!
     4. On cache miss, enforce `paper_fetch::check_fetch_budget(&events, effective_limit)` BEFORE any network call; if exceeded, return a structured budget-limit response (409/429 JSON with current count and limit) so the UI offers the one-click `"raise_budget"` action.
     5. Fetch via F3 (`paper_fetch::fetch`, gated by `net_rate::global_gate()`); ingest into session via `FetchedSource::ingest_into_session` (writing `source.ingested` inside the current session so F4 `verify_quote` works on the cited paper in V12!). Record `SeenRole::CitedLookup` and cache the built skeleton (`Tex1` or V17 `Pdf1`) in F10 via `Library::upsert_skeleton`.
     6. Run `skeleton::locate::locate_in_skeleton`, append `investigation.cited_lookup` (`bib_key`, `locator`, `resolved: PaperRef`, `external_source_id`, `external_tag`, `status`, `number_basis`), and return the verbatim statement (only on `Found`) or candidate list + version disclaimer (`"numbering taken from arXiv vN; the journal version may differ"`), or bibliographic reference + `"attach PDF / TeX"` prompt on `no_tex` (`reason: "needs_file"`) / `unresolved`.
3. **Checkpoint 3 — Context drawer UI, V17 PDF path & in-session cross-paper overlay (`crates/desktop-server/assets/investigation_context.js`):**
   - Wire V17 `pdf-1` skeleton lookup once V17 merges so PDF-only cited papers return `candidates` with physical page links and `"OCR"` badges on scanned pages.
   - Render cited-result card in F6 `"Outside this paper"` provenance style (`PROVENANCE_STYLES.outside_paper`, coloured left border, header `"Outside this paper: <short ref>"`, `createQuoteChip` with line/page link and `"OCR"` badge when from a scanned PDF).
   - Render all 5 named statuses cleanly: `found` (verbatim statement + version caveat), `candidates` (interactive picker list with page numbers for PDFs), `paper_found_locator_not_found`, `no_tex` (bib reference + attach-file trigger), `unresolved` (bib reference + attach-file trigger).
   - Show visible fetch counter (`"3 / 8 external papers fetched"`) with one-click `"raise_budget"` button.
   - Implement "Follow the argument" jump (`investigate.md` 3): button `"Inspect in cited paper"` opens an **in-session focused Investigate overlay** of the cited paper at the located node backed directly by F10's cached skeleton and the external `source.ingested` already pinned in the current session (preserving the original `NodeRef` so returning restores exact selection with zero model/network calls and without colliding with V16's F7 `"Investigate this paper"` child-session creation), plus an optional `"Open full paper in new Investigate session"` button delegating to V4/F7 when available.

## 5. Owned files (all <= 600 lines)

- **New Rust files:**
  - `crates/research-tools/src/skeleton/locate.rs` (+ `locate_tests.rs` if needed)
  - `crates/desktop-server/src/investigation_cited.rs` (+ `investigation_cited_tests.rs`)
  - `crates/desktop-server/tests/investigation_cited_route.rs`
- **New UI & test files:**
  - `crates/desktop-server/assets/investigation_context.js` (cited-result card & drawer shell; keep a clean slot/API so V12 mounts model context cards into the same drawer without merge conflict)
  - `crates/desktop-server/tests/investigation_context.test.mjs`

## 6. Shared registration seams and "Do not edit" files

- **Registration seams:**
  - `crates/research-tools/src/skeleton/mod.rs` (210 lines): add `pub mod locate;` and re-export locate types.
  - `crates/desktop-server/src/lib.rs` (215 lines): add `mod investigation_cited;`.
  - `crates/desktop-server/src/api_dispatch.rs` (from F5): add **exactly 1 arm** for `("POST", ["api", "sessions", id, "investigation", "cited"])` behind `AuthGuard::authorize_api` (handling `"lookup"`, `"pick"`, and `"raise_budget"` via JSON `"action"`).
  - `crates/desktop-server/src/assets.rs` (365 lines): register `/assets/investigation_context.js`.
  - V3/V4 UI seam: clicking a citation chip in `investigation_node.js` calls `investigation_context.js`'s `openCitedLookup({ tag, bibKey, locator })`.
- **Do NOT edit:**
  - `crates/research-tools/src/net_rate.rs`, `paper_fetch.rs`, `arxiv.rs`, `literature.rs` (use F3 as-is; never build a second limiter or fetcher).
  - `crates/research-tools/tests/golden/tool_defs.txt`, `tool_profiles.txt`, `tool_profile.rs`, `Cargo.lock`.
  - `http_srv.rs` (702), `routes_session.rs` (616), `desktop-server/src/investigation.rs` (597), `index.html` (768), `app.js` (932).

## 7. Tests through the entry point

- **Eval Set P5 & "Follow the argument" integration (`crates/desktop-server/tests/investigation_cited_route.rs`, loopback mock + fake clock only, zero real network, zero model calls):**
  - **Two self-authored papers:** Paper A cites `[B26, Theorem 2.1]` and `[B26, Lemma 9]` (ambiguous/off-by-one counter).
    - Confirmed match in Paper B (TeX): status `found`, exact verbatim statement and physical lines match Paper B, zero `model.request` events, F10 `skeleton` cache populated; second lookup hits F10 with **zero** loopback HTTP requests.
    - Off-by-one / unconfirmed computed number fixture (rule 6.9): MUST yield `candidates`, **never** `found` (Set P5 invariant: wrong statement shown as located == **0**).
    - **CP3 Set P5 PDF test (runs once V17 merges):** PDF-only cited Paper C (via V17 `pdf-1`, self-authored PDF from `pdf_support`): MUST yield `candidates` with exact physical page numbers (and `"OCR"` flag on scanned pages), **never** `found`.
    - Missing locator -> `paper_found_locator_not_found`; DOI with no arXiv & no library file -> `status: "no_tex"` with `reason: "needs_file"`; bare citation with no IDs -> `unresolved`.
    - Fetch budget (rule 6.16): 9th external fetch refused until `"raise_budget"` action appends the user-click note, after which 9th succeeds.
    - Cross-paper overlay & return: opening Paper B in the focused in-session overlay at the cited node preserves return `NodeRef` to Paper A's original claim and restores selection on return.
    - Auth check: unauthenticated `POST .../investigation/cited` returns 401/403.
  - **Wording check:** zero occurrences of `"verified"` anywhere in API responses or JS DOM.
- **Node UI tests (`node --test`):** all 5 statuses, `"Outside this paper"` provenance styling, version caveat text, candidate picker selection, fetch-budget counter & raise click, in-session overlay & return-link restoration, and XSS escaping on cited paper titles/statements.

## 8. Document conflicts & resolutions

- **`investigate.md` 6.9 (`no_tex` status vs PDF-only papers):** The original 6.9 status list includes `no_tex`, while the 2026-11-25 scope revision added the parenthetical rule that PDF-only cited papers use the V17 PDF skeleton and return `candidates` with page links. Resolution (`00-overview.md` 0.1 & 2.12 win): try TeX -> PDF (`pdf-1`, yielding `candidates` with pages), and emit `status: "no_tex"` (with `reason: "needs_file"` from `paper_fetch::STATUS_NEEDS_FILE`, or `"empty_text_layer"`) only when neither TeX nor extractable PDF text is available.
- **Cross-paper focused Investigate view (`investigate.md` 3 "Follow the argument" vs V12/V16 session boundaries):** Opening a brand-new child session on every citation click would collide with V16's F7 `"Investigate this paper"` hand-off and leave the external source out of the current session where V12 `"Compare hypotheses"` needs it. Resolution: V11 opens the cited node in an **in-session focused Investigate overlay** backed by F10's cached skeleton + the external `source.ingested` pinned in the current session (instant return to original `NodeRef`), with an optional `"Open full paper in new Investigate session"` button delegating to V4/F7.
- **V11 vs V12 ownership of `investigation_context.js`:** `investigate.md` 9 lists `investigation_context.js` under V11 while V12 also populates the right context drawer. Resolution: V11 owns `investigation_context.js` (context drawer shell + deterministic cited-result cards) and exposes a clean `mountContextSection(id, element)` seam so V12 puts model context cards and the `"Hide unchecked"` toggle in a disjoint `investigation_context_cards.js`.

## 9. Out-of-scope / deferred items and hand-back

- **Out of scope:** Model-generated context cards and `record_context_cards` (V12); arXiv HTML number confirmation (V18, though `locate.rs` already honors `NumberBasis::Html` and `NumberBasis::Confirmed` so V18 plugs in cleanly); F14b Tesseract OCR for empty-text-layer scans; V14 prior-work search.
- **Hand-back (`RULES.md`):** Write `docs/design/modes-2026-11/reports/inv-v11-cited.md` and push to `gemini/modes/inv-v11-cited`.
