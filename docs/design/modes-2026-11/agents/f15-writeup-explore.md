# Brief f15-writeup-explore — F15 Explore Write-Up Adapter (Cited Summary)

Build the Explore F15 write-up adapter (`crates/export/src/writeup_idea.rs`) and register its `Mode::Explore` arm in `crates/export/src/lib.rs`. An Explore write-up is a pure, deterministic cited summary built from hash-chain-verified ledger events (plus read-only `Library` lookup for user citekeys and `.bib`) into F15 core's `Writeup` document model (`Tex`, `Md`, `Bib`) with **zero model calls**.

## 1. Package & governing design

- **Governing design:** `00-overview.md` 0.1, 2.1 row F15, 2.5, 2.9, 2.10, 2.11, 2.12, 2.13 in full, 3.2 rows 17–24, 6, 7.2 Set W, 7.3, 8.1, 8.3 #3 & #10; `explore.md` 4.1–4.4, 5.4, 6 (C1–C9, V1–V10, D1–D3, S8–S9), 7, 9 X17; `agents/RULES.md`.
- **Binding wording & honesty rules:**
  - `"verified"` **never** appears anywhere in Explore views, exports, or write-ups (`00-overview.md` 0.1, 3.2 row 20, 7.3, `explore.md` S8). Quotes are `"matched"`.
  - Non-removable status line, `"Where I looked"` coverage paragraph, and `"How each statement was checked"` table (`inline_marks: false` only toggles inline marks).
  - `judged_*` resting on OCR text from scanned pages is always tagged `"(OCR)"` with physical page number (`00-overview.md` 2.13 rule 6, 8.3 q3, rule C8).
  - Prefer `source_item.citekey` via read-only `Library::open_existing` / `Library::get_paper` (use `writeup_solve.rs` pattern — zero writes if `library.db` absent); user library counts appear in `"Where I looked"` and never satisfy the 2-service coverage gate (Rule V10).

## 2. Start conditions and verified interfaces

- **Required merge on `gemini/work/fixed` before starting:** **`exp-x9-x10-x11-read-verdict`** (for `evidence.quote`, `idea.paper` `text_basis`, `idea.map`, `idea.verdict`, `idea_verdict::compute`, V8 phrase checker).
- **Optional merge (handle gracefully if absent):** **`exp-x12-x13-map-handoff`** (`idea.directions` rendered when present and omitted cleanly when absent; only add `writeup_btn.js` mount in `idea_map.js` if `X12` is already merged — otherwise record the exact mount snippet in Merge notes).
- **Already merged on `gemini/work/fixed`:**
  - F15 core (`export/src/writeup{,_bib,_md,_tex}.rs`), `f15-writeup-solve` (`export/src/lib.rs::build_session_writeup`, `routes_writeup.rs`, `cmd_writeup.rs`, `assets/writeup_btn.js`), F2 (`paper_key.rs`), F4 (`quote_core.rs`), F9 (`explore_stage_run_state`), F10/F13a (`Library`), F12 (`search_coverage.rs`, `prior_work*.rs`), F14a (`pdf_pages.rs` + handle `PageBasis::Ocr` OR raw `"ocr"`/`"scan_text_layer"` cleanly whether `f14b-ocr` has merged or not), `tex_audit.rs`.
  - Note: if `f15-writeup-investigate` already wired `Mode::Investigate` in `export/src/lib.rs`, add ONLY `pub mod writeup_idea;` and fill `Mode::Explore` without altering `writeup_paper.rs` or `writeup_solve.rs`.

## 3. Smallest implementation checkpoints

1. **Ledger fold, verdict recomputation & status line (`crates/export/src/writeup_idea.rs`, `<= 600`L; split `writeup_idea_cite.rs` if `> 500`L):**
   - Load verified session via `Ledger::read_verified_session` (already done in `export/src/lib.rs`). Fold `idea.started`, latest `idea.stated`, `idea.brief`, `idea.paper`, `evidence.quote` (`xe-*`), `evidence.abstract` (`xa-*`), `idea.map`, `idea.verdict`, optional `idea.directions`, `idea.note`, `idea.stage`.
   - **Rules V7 & V10 fail-closed check:** recompute F12 `Coverage` and per-subclaim `Verdict` (`idea_verdict::compute`, respecting origin-paper exclusion V9 & `confirm_same` V6, ignoring F16 lessons & library marks). Refuse fail-closed if stored `idea.verdict` disagrees or if no `idea.stated` / `idea.brief` exists.
   - Opening status line reflects `explore_stage_run_state` + coverage completeness, zero novelty/verification claims.
2. **Per-part findings, matched quotes (`xe`/`xa`), citations, OCR & optional directions:**
   - Per subclaim (`sc-N`): F12 `prior_work_wording` status (`"Model judges: ..."` until `confirm_same`, `"possibly related (abstracts only)"`, scoped `"not found"` only when gate passes), F4 `verify_quote_ref` / pinned `raw_blob` checks (drop unverified, clip `<= 300` UTF-8 chars), `"abstract only"` on `xa`, page + `"(OCR)"` on `scan_text_layer`/`ocr` (no math rendering), and `"What differs (model)"` as `Block::Prose { basis: "model" }` checked by V8 banned-phrase validator.
   - Citations via `writeup_bib::resolve_bib_entry` (prefer `source_item.citekey` from read-only Library, deterministic fallback if absent).
   - Non-removable `"Where I looked"` coverage paragraph (F12 app code: services, queries ok/fail, abstracts/papers, separate library counts, `not_searched`, failing gates).
   - Optional Directions (`Concrete` first, then `Sketch` under `"Loose ideas"`, free prose `basis: "model"`).
   - Non-removable closing `"How each statement was checked"` table.
3. **Register & verify (`export/src/lib.rs` + Set W tests):**
   - Wire `Mode::Explore => writeup_idea::build_explore_writeup_from_verified(data_dir, session, &events)` in `export/src/lib.rs`. Confirm `WriteupFormat::{Tex, Md, Bib}`, `routes_writeup.rs` (`GET /api/sessions/{id}/writeup`), and `cmd_writeup.rs` work end-to-end.

## 4. Owned files & Do-Not-Edit (`<= 600` lines each)

- **New:** `crates/export/src/writeup_idea.rs` (+ optional `writeup_idea_cite.rs`), `crates/export/tests/writeup_idea.rs`, `crates/desktop-server/tests/writeup_explore_routes.rs`.
- **Small edits:** `crates/export/src/lib.rs` (`pub mod writeup_idea;` + `Mode::Explore` arm), plus optional 1-line `writeup_btn.js` mount in `assets/idea_map.js` ONLY if `X12` is already merged (`<= 600`L).
- **Do NOT edit:** `crates/export/src/{idea.rs,latex.rs,writeup_solve*.rs,writeup_paper*.rs}`, `http_srv.rs` (702L), `routes_tools.rs` (610L), `routes_session.rs` (616L), `auth.rs`, `search_coverage.rs`, `prior_work_wording.rs`, `quote_core.rs`, `Cargo.lock`, `tests/golden/*`.
- **Out of scope:** `"Polish the prose (model)"` (`f15-writeup-polish`, tracked separately after all 3 pure adapters merge).

## 5. Tests through entry points (Set W, `00-overview.md` 7.3) & hand-back

- Zero third-party text, zero model/network calls, zero `#[ignore]`. Cover all 8 Set W tests (`tex_audit` 0 blockers/errors, `.bib` `citekey` + fallback across `Tex`/`Md`/`Bib`, all 6 F12 statuses + `confirm_same`, Rule V7/V10 tampered verdict + lesson invariance, `inline_marks: false` non-removable provenance/coverage/table, Directions optionality, Rule S8/V8 wording property test with zero `"verified"`/`"new"`/`"novel"`/`"proved"`, tampered `xe` quote drop + 300-char UTF-8 clip + `(OCR)` tag, determinism + `AuthGuard::authorize_api`).
- Verify before push as non-root (`RE_SEARCH_PYTHON` unset): `check_file_length.py`, (`node --check` + `node --test` if JS edited), `cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or`, `cargo test --workspace --locked --no-fail-fast`.
- Write `docs/design/modes-2026-11/reports/f15-writeup-explore.md` and push to `gemini/modes/f15-writeup-explore`.
