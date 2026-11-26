# Brief f15-writeup-solve — F15 Solve Write-Up Adapter & Shared Seam

**Status:** Queued right behind **`S8 + S9 + S11`** (`solve-s8-s9-s11`). F15 core (`writeup.rs`, `writeup_tex.rs`, `writeup_md.rs`, `writeup_bib.rs`) and F13a are already merged on `gemini/work/fixed`. Zero `Cargo.lock` edits! File-disjoint from `solve-s10-physics-checks`, `solve-s19-export-replay`, `solve-s12-loop`, and `solve-s18a-artifacts`.
**Push branch:** `gemini/modes/f15-writeup-solve`

---

## 1. Package & Governing Design

- **Package:** **F15 Solve adapter (1st adapter)** — deterministic `export::writeup_solve::build_solve_writeup`, shared `WriteupFormat::{Tex, Md, Bib}` + `writeup_session_bytes` / `writeup_session_to_path` in `crates/export/src/lib.rs`, `routes_writeup.rs` (`GET /api/sessions/{id}/writeup` header-token authenticated), `cli/src/cmd_writeup.rs`, and `assets/writeup_btn.js` + Set W tests.
- **Governing design:** `00-overview.md` 0.1, 2.5, 2.10, 2.13, 3.2 row 20, 7.2 Set W, 7.3, 8.1, 8.3 Q10–11; `solve.md` 4.1, 4.4, 5.4 (G7), 5.6, 6 (R5, R22, R23), 9; `agents/RULES.md`.

---

## 2. Verified Interfaces on `gemini/work/fixed`

- **F15 core (`crates/export/src/writeup.rs` 512L, `writeup_tex.rs` 212L, `writeup_md.rs` 161L, `writeup_bib.rs` 200L):** `Writeup`, `WriteupMode`, `ProseBasis`, `HowChecked`, `QuoteLocator`, `WriteupQuote`, `WriteupBlock`, `WriteupSection`, `BibEntry`, `CheckTableRow`, `WriteupOptions`, `writeup_wording`, `build_verified_quote`, `filter_verified_quotes`, `resolve_bib_entry`, `render_bibtex`, `render_writeup_tex`, `render_writeup_markdown`. (`latex.rs` 771L stays untouched.)
- **F10/F13a & S2 (`library.rs`, `solve_state.rs`, `proof_closure.rs`, `solve_badge.rs`):** `Library::open`, `Library::get_paper`, `SolveState::from_events`, `closure_for_target`, `topological_order`, `badge_for_step`, `research_tools::audit_tex_sources`.
- **`crates/desktop-server/src/assets.rs` (`546`L):** Tight headroom! Add compact helper `fn asset(&str, &[u8])` or extract KaTeX font arms into `assets_fonts.rs` first if registering new assets pushes `assets.rs > 570`L.

---

## 3. Checkpoints — Push After Each!

1. **Checkpoint 1 (`crates/export/src/writeup_solve.rs` + `tests/writeup_solve.rs`):**
   - Load verified ledger via `Ledger::read_verified_session`, fold `SolveState::from_events`, compute `closure_for_target` + `topological_order`.
   - Enforce Gate **G7 refusal** whenever any closure step has `Tier::Failed`, `Tier::Stale`, or `superseded_by.is_some()`.
   - Place assumptions (`Given`, `AssumedByModel`, setup conventions) first, followed by topological step derivation ending with final answer.
   - Read formulas **exclusively** from check-process `solve.expr.latex` (refusing missing/non-portable handles; ignore `SolveEntry.latex`, R5). Tag prose `ProseBasis::Model` vs `ProseBasis::User`.
   - Re-verify every Cited quote via `build_verified_quote` (<= 300 chars; `ocr: true` -> `"(OCR)"` + physical PDF page from `pdf_pages::PageBasis::ScanTextLayer` / raw `"ocr"` / `EvidenceQuoteRecord.ocr`).
   - Emit unconditional opening status line and closing `"How each statement was checked"` table (`inline_marks` toggles inline badges only; `"verified"` only on `Tier::Verified` rows/status, R22). Resolve `.bib` via `source_item.citekey` first.
2. **Checkpoint 2 (Shared seam: `export/src/lib.rs`, `routes_writeup.rs`, `cmd_writeup.rs`, `assets/writeup_btn.js`):**
   - Add `WriteupFormat::{Tex, Md, Bib}`, `writeup_session_bytes`, `writeup_session_to_path` (`Mode::Solve` implemented; `Explore`/`Investigate` return `"write-up adapter not built yet"`). `Tex` writes `<stem>.tex` + sidecar `<stem>.bib`.
   - Add `crates/desktop-server/src/routes_writeup.rs` (`GET /api/sessions/{id}/writeup?format=tex|md|bib&inline_marks=0|1`, strictly header-token `AuthGuard::authorize_api`, NEVER added to `auth.rs` stream-cookie list).
   - Add `crates/cli/src/cmd_writeup.rs` (`re-search writeup SESSION --format tex|md|bib --out PATH [--no-inline-marks]`) + 1-line arm in `cli/src/cmd.rs` (`434`L).
   - Add `crates/desktop-server/assets/writeup_btn.js` + `tests/writeup_btn.test.mjs` + `/assets/writeup_btn.js` in `assets.rs` (note 1-line mount call in `solve_steps.js` if S13 not yet merged).
   - Run full checks + Set W tests (`tex_audit`, R5, R22 property sweep over all 11 tiers, OCR tag, G7 refusal, auth), write `reports/f15-writeup-solve.md`, push to `gemini/modes/f15-writeup-solve`. End with `PUSHED`.

---

## 4. Owned Files & Do-Not-Edit (`<= 600` Lines Each)

- **New:** `crates/export/src/writeup_solve.rs` (+ `_cite.rs` if >500L), `crates/export/tests/writeup_solve.rs`, `crates/desktop-server/src/routes_writeup.rs`, `crates/desktop-server/tests/writeup_solve_routes.rs`, `crates/desktop-server/assets/writeup_btn.js`, `crates/desktop-server/tests/writeup_btn.test.mjs`, `crates/cli/src/cmd_writeup.rs`.
- **Small edits:** `crates/export/src/lib.rs` (`128`L), `crates/desktop-server/src/{lib,api_dispatch,assets}.rs`, `crates/cli/src/cmd.rs` (`434`L).
- **Do NOT edit:** `crates/export/src/{latex,notebook,zip,paper_map*,writeup_paper,writeup_idea}.rs`, `crates/desktop-server/src/{http_srv,auth,routes_tools,routes_session}.rs`, `Cargo.lock`, `tool_defs.txt`, `tool_profiles.txt`.
