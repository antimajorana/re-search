# Brief f15-writeup-solve — F15 Solve write-up adapter and shared write-up seam

Build the **first** F15 write-up adapter (`export/src/writeup_solve.rs`) and establish the shared
mode-neutral write-up dispatch seam for desktop and CLI. A Solve write-up is a pure, deterministic
function from verified ledger events (plus read-only library lookup for user citation keys and
`.bib` entries) into F15 core's `Writeup` document model, rendered to TeX (+ `.bib`) and Markdown
with **zero model calls**.

## 1. Package and governing design sections

- **Package:** F15 Solve adapter (`00-overview.md` 2.1 row F15, `solve.md` 9 F15 table).
- **Read first:** `agents/RULES.md`, `00-overview.md` 0.1, 2.1 row F15, 2.5, 2.10, 2.11, 2.12,
  2.13 in full, 3.2 row 20, 6, 7.2 set W, 7.3, 8.1, 8.3 questions 10–11; `solve.md` 4.1, 4.4,
  5.4 (gate G7), 5.6, 6 rules R5/R22/R23, 9 F15 table; reports `f4-f9-quote-budget.md`,
  `f10-library.md`, `f14a-pdf-core.md`, `s2-solve-model.md`, and the F15 core report from
  `lib-f13a-f16a-f15`.
- **Owner decisions of 2026-11-25 (binding):**
  - Lean and physlib are **cut** (`00-overview.md` 0.1, 8.1). No Lean tier or formal citations.
  - `"Verified"` is strictly an exact symbolic check by code (`Tier::Verified`); numeric agreement
    is `"Checked numerically"` (`Tier::CheckedNumerically`, `00-overview.md` 8.3 q11).
  - Provenance is non-removable (`00-overview.md` 2.13 rule 1, 8.3 q10, `solve.md` D22): inline
    badge marks may be toggled off, but the opening status line and closing `"How each statement
    was checked"` table can never be disabled.
  - Quotes are `"matched"` (never `"verified"`), capped at `<= 300` chars (`quote_core::MAX_DISPLAY_QUOTE_CHARS`),
    and tagged `"(OCR)"` when from a scanned page (`00-overview.md` 2.13 rule 6, 8.3 q3).

## 2. Start conditions and build order

- **Adapter build order (`00-overview.md` 2.13):** **1st Solve** (this brief) -> 2nd Explore
  (`draft-f15-writeup-explore.md`) -> 3rd Investigate (`draft-f15-writeup-investigate.md`).
- **Already merged on base `832eab5bf`:** F2 (`paper_key.rs`), F4 (`quote_core.rs`, `lines_sent.rs`),
  F9 (`run_budget.rs`), F10 (`library*.rs`), F14a (`pdf_pages*.rs`, `pdf_match.rs`), S2
  (`solve_model*.rs`, `solve_state.rs`, `solve_status.rs`, `solve_badge.rs`, `proof_closure.rs`).
- **Required merges before starting:**
  - **F15 core** (`lib-f13a-f16a-f15`: `export/src/writeup.rs`, `writeup_tex.rs`, `writeup_md.rs`,
    `writeup_bib.rs`) and **F13a** (`source_item.citekey` populated from user libraries).
  - **F5** (`f5-f7-mode-shell`: `desktop-server/src/session_mode.rs`, `api_dispatch.rs` split out
    of `http_srv.rs`).
  - **S8** (`evidence.quote` Cited steps via F4), **S9** (check-process rendered `solve.expr.latex`
    and `solve.check` records), and **S11** (`solve.done`, gate G7).

## 3. Verified base interfaces vs future-merge interfaces

### Verified on base `832eab5bf`
- `research_tools::solve_state::SolveState::{from_events, entry, tier, badge}` (`solve_state.rs`):
  holds `setup`, `entries: Vec<SolveEntry>`, `handles: BTreeMap<HandleRef, SolveExpr>`, `checks`,
  `quotes: BTreeMap<u64, EvidenceQuoteRecord>`, `tiers`, `run_status`, `done: Option<SolveDone>`.
- `research_tools::proof_closure::{closure_for_target, topological_order}` (`proof_closure.rs`):
  transitive closure and deterministic topological sort over `depends_on ∪ derived_depends_on`.
- `research_tools::solve_status::{Tier, StepTier, StepBadge, badge_for_step, rests_on_summary, wording}`
  (`solve_badge.rs`): single wording table (`BADGE_VERIFIED`, `OWN_CHECK_VERIFIED`,
  `NON_VERIFIED_CONSTANTS`, `FLAG_OCR_SOURCE`, etc.).
- `research_tools::quote_core::{verify_quote, verify_quote_ref, clip_quote, MAX_DISPLAY_QUOTE_CHARS}`
  (`quote_core.rs`): re-verifies quotes against `source.ingested` receipts without an Investigation marker.
- `research_tools::library::Library::{open, get_paper}` (`library.rs`): read-only lookup returning
  `PaperDetail { paper: PaperRecord { bibtex, .. }, source_items: Vec<SourceItemRecord { citekey, .. }>, .. }`.
- `research_tools::pdf_pages::{PageMap, PageBasis, PageRecord}` (`pdf_pages.rs:41-48`, pinned on
  `source.ingested.pdf_pages` in `ingest.rs:298`): has **3 variants on base**: `{TextLayer,
  ScanTextLayer, None}` (`PageBasis::Ocr` exists only in `library_ext_types::PageBasis`, not in
  `pdf_pages.rs` until F14b). Determines page numbers and scanned-page basis (`ScanTextLayer`).
- `ledger::Ledger::read_verified_session` (`crates/ledger/src/verify.rs:84`, used in
  `crates/export/src/latex.rs:135-136`): fail-closed hash-chain verified session read.
- `research_tools::tex_audit` (`tex_audit.rs`): deterministic TeX & citation audit for test validation.
- `crates/export/Cargo.toml`: depends on `ledger`, `tools`, `research-tools` (no `agent` dep).

### Future-merge interfaces to verify before coding
- **F15 core (`export/src/writeup*.rs`):** exact `Writeup`, `Section`, `Block`, `CheckTableRow`,
  `BibEntry`, `WriteupOptions { inline_marks: bool, .. }`, and `render_tex` / `render_md` /
  `render_bib` signatures from `lib-f13a-f16a-f15`.
- **F4/S2 `QuoteRef` unification:** base `832eab5bf` has separate structurally matching
  `solve_model::QuoteRef` and `quote_core::QuoteRef` (`reports/s2-solve-model.md` §4). Verify
  whether merged S8 unified them; if not, map fields explicitly without mutating `solve_model.rs`.
- **S9 check assumptions payload:** confirm the exact JSON key in `solve.check.evidence` carrying
  the assumptions used during symbolic simplification (`solve.md` 4.4 guard (c)) so every
  `Verified` row in the check table prints its assumptions.
- **S11 `solve.done` and G7 helper:** confirm exact `SolveDone` fields and whether S11 exports a
  shared G7 closure-readiness predicate to reuse directly.
- **F5 `api_dispatch.rs` & `session_mode.rs`:** verify exact route dispatch and `Mode::Solve`
  detection entry point.
- **F14b (optional, future):** `pdf_pages::PageBasis::Ocr` is added only when F14b lands. Until
  then, detect scanned/OCR quotes strictly via `pdf_pages::PageBasis::ScanTextLayer`, raw JSON
  `"ocr"` basis strings, and `EvidenceQuoteRecord.ocr == true` so this adapter compiles cleanly
  before F14b.

## 4. Smallest implementation checkpoints

1. **Closure, G7 refusal, and deterministic ordering (`writeup_solve.rs`):**
   - Load verified session events via `Ledger::read_verified_session` (`crates/ledger/src/verify.rs:84`,
     same fail-closed gate as `latex.rs:135-136`).
   - Fold `SolveState::from_events(&events)`. Identify final target (`state.done.final_entry_id`,
     or explicit `target_entry_id` override if provided; refuse if none exists).
   - Compute `closure_for_target` + `topological_order` over declared + derived edges.
   - **Gate G7 refusal (`00-overview.md` 2.13 rule 2):** return a structured error listing offending
     step IDs whenever ANY step in the closure has `Tier::Failed`, `Tier::Stale`, or
     `superseded_by.is_some()`.
   - Order closure sections: **Assumptions first** (`Tier::Given`, `Tier::AssumedByModel`, and
     setup conventions/symbol assumptions), then cited/derived/computed steps in topological
     dependency order ending with the final answer.
2. **Handle formulas (R5), Cited quotes (F4 + OCR), and citations (F10/F13a):**
   - For every `Claim` on a step, read `lhs`/`rhs` LaTeX strictly from `state.handles[&handle].latex`
     (check-process rendering). Refuse if a referenced handle is missing or `!expr.portable`. Never
     read `SolveEntry.latex` or scrape formulas from model prose.
   - Step `summary`/`details` emit as `Block::Prose { basis: "model" }` (or `"user"` for
     `StepOrigin::Setup | Given | User`) and are explicitly marked as model prose in the check table.
   - For `Tier::Cited` steps: re-verify `source_ref` against the session's `source.ingested`
     receipt via `quote_core::verify_quote_ref` (never emit an unverified quote). Clip to
     `<= 300` chars (`clip_quote`). Mark `ocr: true` (`"(OCR)"`) if `EvidenceQuoteRecord.ocr` is
     true OR `source.ingested.pdf_pages` puts the cited lines on `pdf_pages::PageBasis::ScanTextLayer`
     (or raw JSON `"ocr"` basis string from future F14b). Emit `\cite{<key>}` with the quote (and
     line or PDF page locator) in a footnote.
   - Resolve citation keys read-only via `Library::get_paper` (optional; missing `library.db`
     degrades gracefully to ledger metadata): prefer `source_item.citekey` (F13a), else deterministic
     app-generated key; emit matching `BibEntry` for `.bib`.
3. **Status line, non-removable check table, and `"verified"` wording rules (R22, R23):**
   - Opening status line equals the run's real status: `"Done"` only when `solve.done.status ==
     Finished`, `"Done with open items: ..."` when `FinishedOpenItems` (listing open counts), or
     exact F9 `RunState` label if unfinished. Never say the result as a whole is verified unless
     the final step's `effective` tier is `Tier::Verified`.
   - Closing table `"How each statement was checked"` lists every step in the closure with step ID,
     effective badge (`badge_for_step`), own tier / `"own check: verified"` + check assumptions
     when `own == Verified`, prose provenance (`model` vs `user`), and `"rests on"` dependencies.
   - `WriteupOptions.inline_marks` toggles inline step badges ONLY; status line and check table are
     unconditionally emitted.
4. **Shared desktop, CLI, and UI write-up dispatch seam (`WriteupFormat::{Tex, Md, Bib}`):**
   - Add `WriteupFormat::{Tex, Md, Bib}`, `export::writeup_session_bytes`, and
     `export::writeup_session_to_path` in `export/src/lib.rs` (mode dispatch via
     `session_mode_from_events`; initially implementing `Mode::Solve` and returning a clear
     `"write-up adapter not built yet"` error for `Explore`/`Investigate` until their adapters
     land). On disk (`writeup_session_to_path`), `Tex` writes BOTH `<stem>.tex` and sidecar
     `<stem>.bib` automatically; `Bib` writes standalone `.bib`; `Md` writes `.md`.
   - Add `crates/desktop-server/src/routes_writeup.rs` exporting a single dispatcher
     `handle_writeup(state, session_id, query)` for `GET /api/sessions/{id}/writeup?format=tex|md|bib[&inline_marks=0|1]`.
     Keep `/api/sessions/{id}/writeup` strictly **header-token authenticated** (`X-ReSearch-Token` /
     `Authorization: Bearer`, matching `/api/library`, fetched via `api.js` Blob download — zero
     expansion of the `auth.rs:208` stream-cookie route surface).
   - Add `crates/cli/src/cmd_writeup.rs` for `re-search writeup SESSION --format tex|md|bib --out PATH [--no-inline-marks]`
     (writing sidecar `.bib` alongside `.tex` automatically).
   - Add small shared UI helper `crates/desktop-server/assets/writeup_btn.js` (registered in
     `assets.rs`, tested in `tests/writeup_btn.test.mjs`) providing the `"Write up"` menu/button
     (TeX / Markdown / BibTeX + `"Hide inline marks"` toggle over header-token `api.js` Blob
     download), mounted by Solve (`solve_steps.js`, if S13 is merged; else record the exact 1-line
     mount call in Merge notes), and reused unchanged by Explore and Investigate.

## 5. Owned files (all <= 600 lines)

Disjoint from F15 core (`writeup.rs`, `writeup_tex.rs`, `writeup_md.rs`, `writeup_bib.rs`),
S19 (`notebook.rs`, `zip.rs`), V15 (`paper_map*.rs`), X17 (`export/src/idea.rs`), and the other
two write-up adapters (each adapter owns its own disjoint route integration test file):
- `crates/export/src/writeup_solve.rs` (adapter; split helpers into `writeup_solve_cite.rs` if > 500 lines)
- `crates/export/tests/writeup_solve.rs` (entry-point + Set W + property tests)
- `crates/desktop-server/src/routes_writeup.rs` (shared write-up HTTP dispatcher)
- `crates/desktop-server/tests/writeup_solve_routes.rs` (auth + Solve write-up route tests)
- `crates/desktop-server/assets/writeup_btn.js` (shared `"Write up"` UI button + Blob download)
- `crates/desktop-server/tests/writeup_btn.test.mjs` (Node tests for `writeup_btn.js`)
- `crates/cli/src/cmd_writeup.rs` (CLI `writeup` subcommand + tests)

## 6. Shared registration seams and "Do not edit" files

- **Allowed minimal registration edits (isolated commit, documented in report Merge notes):**
  - `crates/export/src/lib.rs` (132 lines): declare `pub mod writeup_solve;` and add
    `WriteupFormat::{Tex, Md, Bib}` + `writeup_session_bytes` / `writeup_session_to_path`.
  - `crates/desktop-server/src/lib.rs`: declare `pub mod routes_writeup;`.
  - `crates/desktop-server/src/assets.rs` (365 lines): register `writeup_btn.js`.
  - `crates/desktop-server/src/api_dispatch.rs` (created by F5): add ONE arm for
    `("GET", ["api", "sessions", id, "writeup"]) => handle_writeup(...)`. If F5's `api_dispatch.rs`
    is not yet merged, leave the exact 1-line arm in `routes_writeup.rs` header + report Merge notes.
  - `crates/cli/src/cmd.rs` (462 lines): add ONE `"writeup"` match arm + usage line.
  - `crates/desktop-server/assets/solve_steps.js` (created by S13): 1-line mount of `writeup_btn.js`
    if S13 is merged; otherwise leave the exact mount snippet in report Merge notes.
- **Do NOT edit:**
  - `crates/export/src/latex.rs` (771 lines — over 600; untouched legacy Research export).
  - `crates/desktop-server/src/http_srv.rs` (702 lines), `routes_tools.rs` (610 lines), and
    `routes_session.rs` (616 lines) — all over 600.
  - `crates/desktop-server/src/auth.rs` (340 lines — keep `/api/sessions/{id}/writeup` strictly
    header-token authenticated; do NOT add it to `is_cookie_route` at line 208).
  - `crates/research-tools/src/solve_badge.rs`, `solve_model.rs`, `proof_closure.rs`, `quote_core.rs`,
    `crates/agent/src/resume.rs`, `Cargo.lock`, `crates/research-tools/tests/golden/*`.

## 7. Tests through the entry point (Evaluation Set W, `00-overview.md` 7.3)

All tests use self-written synthetic fixtures (zero third-party paper text, zero model calls, zero
network calls) and run through the public entry point (`writeup_session_bytes` / `writeup_session_to_path`,
`handle_writeup`, `cmd_writeup`, and `writeup_btn.test.mjs`):
1. **TeX validity (`tex_audit`) & aligned formats (`Tex | Md | Bib`):** generated `.tex` + `.bib`
   passes `research_tools::tex_audit` with zero blockers/errors; every `\cite{...}` resolves to a
   `.bib` entry (testing both user library `source_item.citekey` and fallback app-generated key
   when `library.db` is absent); `Tex`, `Md`, and `Bib` work uniformly across Rust API, HTTP, and CLI.
2. **Formula fidelity (R5):** every formula in TeX and Markdown equals `solve.expr.latex` from
   the check process; tampered/divergent `SolveEntry.latex` is ignored; missing or non-portable
   handle causes refusal.
3. **Closure order & assumptions first:** topological dependency order over declared + derived
   edges, assumptions (`Given`, `Assumed by model`) placed first, unrelated off-closure steps omitted.
4. **Gate G7 refusal:** closure with a `Failed`, `Stale`, or superseded step returns an error
   naming the offending step IDs (both in Rust API and HTTP 409/400 response).
5. **Non-removable provenance (q10):** with `inline_marks: false`, inline step badges disappear
   while the status line and complete `"How each statement was checked"` table remain intact.
6. **Status line fidelity:** matches `solve.done.status` (`Done` vs `Done with open items: ...`)
   and never prints `"Done"` without a `Finished` `solve.done` event.
7. **Wording property test (`00-overview.md` 7.3, R22):** over generated Solve ledgers spanning all
   11 `Tier` variants, `"verified"` (case-insensitive) appears ONLY in the check table on rows
   whose own tier is `Verified` (with assumptions printed) or in final-answer status when the final
   step's effective tier is `Verified`; `"proved"`, `"new"`, `"novel"` never appear in app output.
8. **Cited quote re-verification & OCR tagging:** tampered/mismatched `evidence.quote` is rejected
   (never written); quote > 300 chars is clipped cleanly on UTF-8 boundary; scanned PDF quote
   (`pdf_pages::PageBasis::ScanTextLayer`, raw `"ocr"` basis, or `EvidenceQuoteRecord.ocr == true`)
   carries `"(OCR)"` and its physical PDF page number.
9. **Determinism & header-token auth:** calling twice on the same ledger produces byte-identical
   TeX, `.bib`, and Markdown; unauthenticated and cookie-only HTTP requests to
   `/api/sessions/{id}/writeup` are rejected by `AuthGuard::authorize_api`, while valid
   header-token requests succeed.

## 8. Document conflicts and resolutions

- **Legacy `latex.rs` `(F15)` doc header & 771-line count (`crates/export/src/latex.rs:1` vs
  `00-overview.md` 2.13):** `latex.rs` is the older Research publication export and is over 600
  lines. `00-overview.md` 2.13 wins: legacy exports stay untouched; new F15 write-ups use F15 core
  and `writeup_solve.rs`.
- **Pre-revision `"verified"` = Lean only (`solve.md` section 0, `quote_core.rs:27` vs
  `00-overview.md` 0.1, 2.13 rule 4, 8.1, 8.3 q11):** `00-overview.md` wins: Lean is cut;
  `"Verified"` means exact symbolic check, `"Checked numerically"` means numeric agreement.
- **Desktop export seam (`routes_tools.rs` 610 lines vs `explore.md` 5.4):** `00-overview.md` 2.13
  + `explore.md` 5.4 win: use dedicated `GET /api/sessions/{id}/writeup` in new `routes_writeup.rs`
  with header-token auth (no edit to `auth.rs:208`, `routes_tools.rs`, or `http_srv.rs`).
- **OCR detection split (`reports/s2-solve-model.md` §3.7 `evidence.quote.ocr` vs F14a
  `source.ingested.pdf_pages` with 3-variant `pdf_pages::PageBasis`):** check `EvidenceQuoteRecord.ocr`,
  `pdf_pages::PageBasis::ScanTextLayer`, and raw `"ocr"` payload strings; any triggers `"(OCR)"`.

## 9. Out-of-scope items and hand-back

- **Out of scope:**
  - Explore (`writeup_idea.rs`) and Investigate (`writeup_paper.rs`) adapters.
  - S19 Notebook export (`notebook.rs`) and `zip.rs` replay changes.
  - **Optional `"Polish the prose (model)"` (`00-overview.md` 2.13 rule 5):** NOT built here and
    NEVER duplicated per adapter. Tracked as a single follow-up brief **`f15-writeup-polish`**
    (owning `export/src/writeup_polish.rs` for placeholder locking/restoration +
    `desktop-server/src/writeup_polish.rs` for the model exchange running
    `honesty::lint_honesty_with_ledger` and Explore's V8 phrase check, emitting the mandatory
    `"prose edited by a model"` banner) after all 3 pure adapters merge. This adapter only tags
    each `Block::Prose` with accurate `basis: "app" | "user" | "model"` so the future shared
    polisher locks everything else automatically.
- **Hand-back (`RULES.md`):** run `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`,
  `cargo test -p research-tools --test tool_def_golden`, `cargo clippy --workspace --all-targets`,
  `cargo test --workspace --no-fail-fast`, `node --check crates/desktop-server/assets/writeup_btn.js`,
  and `node --test crates/desktop-server/tests/writeup_btn.test.mjs`; commit
  `docs/design/modes-2026-11/reports/f15-writeup-solve.md` last; push to
  `gemini/modes/f15-writeup-solve`.
