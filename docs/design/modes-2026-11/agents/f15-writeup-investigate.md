# Brief f15-writeup-investigate — F15 Investigate Write-Up Adapter (Reading Notes)

**Status:** Queued right behind **`f15-writeup-solve`** (once Investigate `V2–V8, V10, V17` are merged; prioritized ahead of Explore!). Zero `Cargo.lock` edits!
**Push branch:** `gemini/modes/f15-writeup-investigate`

---

## 1. Package & Governing Design

- **Package:** **F15 Investigate adapter** (`export/src/writeup_paper.rs` reading notes: outline, main results, gists `"(model)"`, user notes, bibliography, quality status line, and non-removable `"How each statement was checked"` table with zero model calls), wired via `Mode::Investigate` into `f15-writeup-solve`'s shared dispatcher (`export/src/lib.rs`, `routes_writeup.rs`, `cmd_writeup.rs`, `writeup_btn.js`).
- **Governing design:** `00-overview.md` 0.1, 2.13, 3.2 rows 19–24, 7.2 Set W, 7.3, 8.1, 8.3 Q3 & Q10; `investigate.md` 3 (`L'`), 4.1–4.4, 6 (rules 6.2, 6.5, 6.6, 6.12, 6.21, 6.22), 12, D9; `agents/RULES.md`.

---

## 2. Verified Interfaces & Disjointness

- **Disjoint from V15 Share:** V15 owns `.rsmap` and standalone `.html` (`/investigation/export`); F15 owns readable `.tex` / `.md` / `.bib` reading notes (`/api/sessions/{id}/writeup` via `f15-writeup-solve`'s `routes_writeup.rs`).
- **Inputs read-only:** `Ledger::read_verified_session`, `investigation.skeleton` (`PaperSkeleton`, `builder`, `quality`), `map_fold` (Orient + Open + V12 context cards if present + user `investigation.note` pins & text notes + F16a `investigation.override`), `quote_core::verify_quote_ref` + `clip_quote`, `pdf_pages::PageMap` (`ScanTextLayer` / `"ocr"`), `Library::get_paper` (licence check: both ledger `"licence"`/`"license"` and `PaperFlags.license` — fail-closed `<= 300` chars unless explicit permissive `CC-BY*` / `CC0*`), `tex_audit`.
- **Priority / ordering note:** Starts immediately after `f15-writeup-solve` without blocking on `f15-writeup-explore` (`Mode::Investigate` arm in `export/src/lib.rs` is disjoint from `Mode::Explore`).

---

## 3. Checkpoints

1. **Checkpoint 1 (`crates/export/src/writeup_paper.rs` [+ `writeup_paper_meta.rs` if > 500L] + `tests/writeup_paper.rs`):**
   - Works on skeleton-only sessions AND full Orient/Open/notes/pins/F16a sessions (user pins override model main-result labels; overrides tagged `"corrected by you"`).
   - Enforces licence-gated quote cap (default arXiv `nonexclusive-distrib` / missing -> `<= 300` chars; `CC-BY` -> full), physical PDF page numbers, `"(OCR)"` tagging with zero OCR math delimiters, `"(model)"` gists/attributions (`"Authors: <New|Restated|Background|Not stated> (model)"`), zero `"verified"`, `"new"` ONLY in explicit `"Authors: New (model)"`, and non-removable check table (`inline_marks: false` keeps status + table).
2. **Checkpoint 2 (`export/src/lib.rs`, `tests/writeup_investigate_routes.rs`):**
   - Wire `Mode::Investigate => writeup_paper::build_writeup(...)` in `export/src/lib.rs` + note/mount `assets/writeup_btn.js` cleanly on Investigate UI (`investigation_handoff_ui.js` or `investigation_v3_shell.js` keeping `<= 600`L).
   - Verify `tex_audit` clean, determinism, header-token `AuthGuard::authorize_api`, all Set W checks. Write `docs/design/modes-2026-11/reports/f15-writeup-investigate.md` and push to `gemini/modes/f15-writeup-investigate`. End with `PUSHED`.

---

## 4. Owned Files & Do-Not-Edit (`<= 600` Lines Each)

- **New:** `crates/export/src/writeup_paper.rs` (+ `writeup_paper_meta.rs`), `crates/export/tests/writeup_paper.rs`, `crates/desktop-server/tests/writeup_investigate_routes.rs`.
- **Small edits:** `crates/export/src/lib.rs` (+1 `Mode::Investigate` arm & `pub mod writeup_paper;`), optional 1-line `writeup_btn` UI mount.
- **Do NOT edit:** `paper_map*.rs`, `investigation_share.rs`, `latex.rs` (`771`L), `writeup_solve.rs`, `writeup_idea.rs`, `http_srv.rs`, `routes_session.rs`, `auth.rs`, `Cargo.lock`, `tool_defs.txt`, `tool_profiles.txt`.
