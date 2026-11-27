# Report: `f15-writeup-investigate` (F15 Investigate Write-Up Adapter — Reading Notes)

- **Base commit:** `24be58e6797679cfa423dcc16ea2073b54288714` (`origin/gemini/work/fixed`)
- **Work branch:** `work/f15-writeup-investigate`
- **Push branch:** `gemini/modes/f15-writeup-investigate` (pushed incrementally after Checkpoint 1, Checkpoint 2 + audit hardening, and final report)
- **Toolchain on this machine:** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `cargo 1.100.0 (3acc79c0c 2026-11-05)`, Node `v22.23.5`, Python `3.12.16`, non-root `uid=1001(antigravity)` on Linux x86_64
- **Unchanged guard rails:** `Cargo.lock` untouched (0 new dependencies); `crates/research-tools/tests/golden/tool_defs.txt` and `tool_profiles.txt` untouched (`tool_def_golden`: 3 passed, 0 failed); `paper_map*.rs`, `investigation_share.rs`, `latex.rs` (771L), `http_srv.rs`, `routes_session.rs`, `auth.rs` untouched; zero `#[ignore]` or skipped tests added; `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` passes (every touched/added source file <= 600 lines).

---

## 1. What was implemented (checkpoint by checkpoint)

### Checkpoint 1 — F15 Investigate reading-notes adapter & Set W tests (`a44df9f`)
- `crates/export/src/writeup_paper.rs` (398 lines, new): public entry points `build_writeup`, `build_writeup_with_options`, `PaperWriteupOptions { include_notes: bool }` (default `true`), `PaperWriteupBuild`, `pub(crate) fn build_writeup_from_verified`, `pub use is_permissive_licence`, fail-closed ledger verification (`Ledger::read_verified_session`), active Investigate slice & `investigation.skeleton` blob SHA-256 verification, exact `source_seq` receipt lookup, unified paper metadata resolution, and mandatory opening status line builder (`build_status_line`). Zero model calls, zero network calls.
- `crates/export/src/writeup_paper_meta.rs` (589 lines, new): named constants tied to owner decisions (`CORRECTED_BY_YOU_LABEL`, `PINNED_MAIN_RESULT_NOTE`, `SKELETON_ONLY_STATUS`, `FALLBACK_RESULTS_NOTE`), fail-closed tokenized `is_permissive_licence` allow-list + deterministic licence resolution (`investigation.paper` -> `source.ingested` -> read-only `Library::get_paper`, supporting both UK `"licence"` and US `"license"` spellings), `PdfPageIndex` supporting F14a `PageBasis` (`text_layer`, `scan_text_layer`, `none`) and future F14b raw `"ocr"` basis strings, F16a override folding (`lessons.db` when present + ledger `investigation.override`/`investigation.node_override` with `node_fp` and valid-tag filtering per rule 6.22), fail-closed physical source verification (`files_root_sha256` + per-file SHA-256), safe same-line/multi-file TeX span extraction, OCR math-delimiter neutralization for full statements, and F10/F13a bibliography resolution (`resolve_bibliography` preferring user `source_item.citekey`).
- `crates/export/src/writeup_paper_nodes.rs` (520 lines, new): deterministic `"Outline"`, `"Main Results"`, and `"Additional Annotated Nodes and Reader Notes"` section builders; user pins (`ReaderPin::MainResult` / `NotMainResult`) override model `ReadingRole` and fallback eligibility (including `Equation` fallback); deduplicated `m-*` model nodes + orphan `m-*` node inclusion; F16a overrides tagged `"corrected by you"` without mutating `PaperSkeleton`; licence-gated `<= 300`-char clipped quotes vs full `CC-BY`/`CC0` statements with inline provenance; user notes (`include_notes` toggle); non-removable check table rows.
- `crates/export/src/writeup_paper_anchors.rs` (299 lines, new): F4 `quote_core::verify_quote_ref` re-verification of every Layer-2 anchor (`reading.anchors`, `authors_claim.anchors`, `hypotheses`, `context_cards.anchors`, `context_cards.hypotheses`) **before** emitting grounded claim/card prose; deterministic cue recomputation strictly from verified anchors (`status_badge`); refusal to misattribute external anchors to the primary paper; 64-line span probe narrowing for long (`>= 64`-line) theorems.
- `crates/export/tests/writeup_paper.rs` (566 lines, new) + `crates/export/tests/writeup_paper_support/mod.rs` (565 lines, new): 7 self-authored synthetic Set W integration/property tests (zero third-party paper text, zero network, zero model calls) covering `tex_audit` cleanliness (0 blockers, 0 errors), user `source_item.citekey` + no-library fallback, skeleton-only & full Orient/Open/V12/notes/pins/F16a sessions, `include_inline_badges: false` non-removable provenance (Q10), `include_notes: false` opt-out, fail-closed licence gate (including adversarial NC/ND/SA/negated strings & UK/US ledger + library `CC-BY-4.0`), PDF physical page numbers + `ScanTextLayer` & raw `"ocr"` `(OCR)` tagging + zero OCR math delimiters, wording property across all 4 `AuthorsClaimLabel` variants + byte-determinism, tamper refusal (modified source file, corrupted skeleton blob, forged anchor omitting ungrounded `Authors: Background (model)`), and shared `writeup_session_bytes`/`writeup_session_to_path` dispatch.

### Checkpoint 2 — Audit hardening, shared dispatcher seam, and desktop-server route/auth tests (`9f6953e`, `a1b5599`)
- `crates/export/src/lib.rs` (284 lines, +142): declared `pub mod writeup_paper;` (+ private helpers) and wired `WriteupFormat::{Tex, Md, Bib}`, `build_session_writeup` (`Mode::Investigate => writeup_paper::build_writeup_from_verified(...)`; other modes return `"write-up adapter not built yet for mode ..."`), `writeup_session_bytes` (aligned `{session}-writeup` `bib_stem`), and `writeup_session_to_path` (`Tex` automatically writes sidecar `<stem>.bib`).
- `crates/desktop-server/tests/writeup_investigate_routes.rs` (335 lines, new): 2 integration tests verifying (1) real desktop-server Investigate session setup (`POST /api/sessions`, `ingest_bytes`, `POST /investigation/skeleton`, `POST /investigation/notes`) exported through `writeup_session_bytes` & `writeup_session_to_path` (`Tex`/`Md`/`Bib`, `tex_audit` clean, determinism, zero `"verified"`), plus V15 disjointness (`GET /investigation/export?format=md` -> HTTP 400 pointing to `writeup_paper.rs`; `format=rsmap` -> HTTP 200), and (2) strict header-token `AuthGuard::authorize_api` enforcement on `/api/sessions/{id}/writeup` (unauthenticated & cookie-only rejected with 401 both in unit and live loopback HTTP; `X-ReSearch-Token` and `Authorization: Bearer` accepted).

---

## 2. Independent adversarial audit summary

An independent read-only audit subagent (`pro`) audited Checkpoint 1 + Checkpoint 2 against `RULES.md`, `f15-writeup-investigate.md`, `draft-f15-writeup-investigate.md`, `00-overview.md`, and `investigate.md`. All actionable in-scope findings were fixed in commit `9f6953e` and re-verified across the full workspace:
- **Fixed:** eliminated double ledger verification in `lib.rs` via `build_writeup_from_verified` and aligned default `bib_stem` in `writeup_session_bytes` with `{session}-writeup.bib`; verified previously skipped `reading.anchors` and `context_card.hypotheses`; replaced substring licence checks with strict tokenized SPDX/CC URL allow-list rejecting `NC`/`ND`/`SA`/negated strings; verified anchors *before* emitting `authors_claim` and `Paper`/`External` context cards and refused external->primary misattribution; deduplicated `m-*` child nodes and included orphan `m-*` nodes; fixed `>= 64`-line theorem verification, same-line `\begin{...}` preservation, and multi-file span loading; enforced non-empty `files_root_sha256`, exact `source_seq` receipt matching, verified binary asset tolerance, `node_fp` override checks, `Equation` `NotMainResult` pin exclusion, `not_a_node` section/node check rows, OCR math neutralization + inline provenance on full `CC-BY` statements, and PDF status-line deduplication.
- **Documented (out-of-scope / dependent on `f15-writeup-solve`):** live HTTP `/api/sessions/{id}/writeup` routing (`routes_writeup.rs`, `api_dispatch.rs`), `assets/writeup_btn.js`, `writeup.rs`/`writeup_tex.rs`/`writeup_md.rs`, and read-only SQLite constructors in `library.rs`/`lessons.rs` (see Sections 4 & 6 below).

---

## 3. Decisions taken alone, deviations, and assumptions

### Decisions taken alone
1. **Fail-closed `CC-BY-SA` exclusion:** `is_permissive_licence` rejects `CC-BY-SA*` (ShareAlike copyleft) alongside `CC-BY-NC*`, `CC-BY-ND*`, and `nonexclusive-distrib`, permitting `> 300`-char full statements strictly for permissive `CC-BY` (without NC/ND/SA), `CC0`, and Public Domain.
2. **Representing permissive `> 300`-char statements via `WriteupBlock::Statement` + inline source line:** F15 core (`crates/export/src/writeup.rs:392`, out of scope) enforces a hard 300-char cap on `WriteupBlock::Quote`. For permissive `CC-BY`/`CC0` statements exceeding 300 chars, `writeup_paper_nodes.rs` verifies the full file SHA-256 + opening excerpt via `quote_core::verify_quote`, emits the full contiguous statement as `WriteupBlock::Statement` (neutralizing math delimiters when `ocr == true`), and emits an adjacent `Source: [cite_key] (locator) (OCR) — full statement permitted by <licence>` line so full statements remain contiguous and preserve inline provenance without modifying F15 core.
3. **Providing `WriteupFormat` / `writeup_session_bytes` / `writeup_session_to_path` in `crates/export/src/lib.rs`:** `f15-writeup-solve` has not yet merged on `gemini/work/fixed` (`24be58e6`), so the shared dispatcher functions were added in `crates/export/src/lib.rs` matching `f15-writeup-solve.md`'s exact interface (wired to `Mode::Investigate`, returning `"write-up adapter not built yet"` for other modes) so Checkpoint 2 compiles and passes end-to-end immediately.
4. **Did not create a stub `routes_writeup.rs` or `assets/writeup_btn.js`:** Both files are explicitly owned by `f15-writeup-solve`. Per `RULES.md` §Scope and `draft-f15-writeup-investigate.md` §4.4/§6, we did not implement `f15-writeup-solve`'s files or add a dangling JS import; exact merge snippets are recorded below.
5. **Direct `WriteupQuote` construction from `quote_core::verify_quote` / `verify_quote_ref`:** Avoids `writeup.rs:440`'s full-line `verified.excerpt` overwrite bug when clipping lines > 300 chars (see Section 6).
6. **Consistent metadata precedence (`investigation.paper` -> `source.ingested` -> `Library`):** Applied uniformly across title, authors, and year so session-pinned metadata wins over mutable library rows.

### Deviations from the design
- Split `writeup_paper.rs` into four modules (`writeup_paper.rs`, `writeup_paper_meta.rs`, `writeup_paper_nodes.rs`, `writeup_paper_anchors.rs`) so every file stays strictly `<= 600` lines (max 589 lines).
- In `writeup_paper_meta.rs:241, 398`, `open_existing_library` and `collect_reader_overrides` guard `.is_file()` before calling `Library::open` / `LessonsStore::open` so missing DBs are never created, but existing DBs still open via those crates' current `READ_WRITE` constructors (see Section 6).

### Assumptions
- Future F14b OCR receipts will record `"basis": "ocr"` (or `"ocr": true`) on `source.ingested.pdf_pages[]` entries; `PdfPageIndex::from_receipt` parses raw JSON strings so both F14a `"scan_text_layer"`/`"none"` and future F14b `"ocr"` work without schema breakage.
- `f15-writeup-solve`'s `routes_writeup.rs` and `cmd_writeup.rs` will call `export::writeup_session_bytes` / `export::writeup_session_to_path` with `WriteupFormat::{Tex, Md, Bib}` and `WriteupOptions`; `include_notes=0` support in `routes_writeup.rs` can additionally call `writeup_paper::build_writeup_with_options` for `Mode::Investigate`.

---

## 4. Merge notes

### 1. Reconciling `crates/export/src/lib.rs` with `f15-writeup-solve` (and later `f15-writeup-explore`)
When merging `f15-writeup-solve` alongside this branch, keep the single `WriteupFormat`, `build_session_writeup`, `writeup_session_bytes`, and `writeup_session_to_path` definitions in `crates/export/src/lib.rs` and add `f15-writeup-solve`'s `Mode::Solve` arm in `build_session_writeup`:
```rust
research_tools::paper_key::Mode::Solve => writeup_solve::build_solve_writeup(data_dir, session),
research_tools::paper_key::Mode::Investigate => writeup_paper::build_writeup_from_verified(
    data_dir, session, &ledger, &events, writeup_paper::PaperWriteupOptions::default(),
).map(|b| b.writeup),
```
For `GET /api/sessions/{id}/writeup?...&include_notes=0` on Investigate sessions, `routes_writeup.rs` (owned by `f15-writeup-solve`) can pass `PaperWriteupOptions { include_notes }` into `writeup_paper::build_writeup_with_options`.

### 2. UI mount snippet once `f15-writeup-solve` delivers `crates/desktop-server/assets/writeup_btn.js`
Note `crates/desktop-server/assets/investigation_v3_shell.js` is at **597 lines** (3 lines under the 600-line ceiling). Mount `writeup_btn.js` cleanly in `crates/desktop-server/assets/investigation_share_ui.js` (268 lines, inside `mountInvestigationShare` right after appending `group` to `toolbar`):
```js
import { mountWriteupButton } from './writeup_btn.js';
// Inside mountInvestigationShare({ toolbar, apiMod, sessionId, ... }):
if (toolbar && sessionId) mountWriteupButton({ container: toolbar, sessionId, apiMod, mode: 'investigate' });
```

### 3. Edits to existing files on this branch
- `crates/export/src/lib.rs` (commits `a44df9f`, `9f6953e`): declared `pub mod writeup_paper;` (+ private helper modules) and added `WriteupFormat`, `build_session_writeup`, `writeup_session_bytes`, and `writeup_session_to_path`.

---

## 5. Verification, exact test counts, and Linux limits

### Commands run (non-root `uid=1001(antigravity)`, `RE_SEARCH_PYTHON` unset)

| Command | Result |
|---|---|
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | **Pass** — `No touched source file is over 600 lines.` (max touched file: `crates/export/src/writeup_paper_meta.rs` at 589 lines) |
| `unset RE_SEARCH_PYTHON && cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` | **Pass** — 0 warnings, 0 errors |
| `unset RE_SEARCH_PYTHON && cargo test --workspace --locked --no-fail-fast` | **Pass** — **1204 passed, 0 failed, 2 ignored** (`export/tests/writeup_paper.rs`: 7/7; `desktop-server/tests/writeup_investigate_routes.rs`: 2/2; the 2 ignored are pre-existing live eval tests in `crates/agent/tests/eval_harness.rs`) |
| `unset RE_SEARCH_PYTHON && cargo test -p research-tools --test tool_def_golden --locked` | **Pass** — **3 passed, 0 failed, 0 ignored** |
| `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` | **Pass** — 0 syntax errors |
| `node --test crates/desktop-server/tests/*.test.mjs` | **Pass** — **226 passed, 0 failed, 0 skipped** |
| `git diff --stat origin/gemini/work/fixed -- Cargo.lock crates/research-tools/tests/golden/ crates/export/src/latex.rs crates/export/src/paper_map*.rs crates/desktop-server/src/investigation_share.rs crates/desktop-server/src/http_srv.rs crates/desktop-server/src/routes_session.rs crates/desktop-server/src/auth.rs` | **Pass** — empty diff |

### Pre-existing items
- **2 ignored tests** in `cargo test --workspace`: pre-existing live-network eval tests in `crates/agent/tests/eval_harness.rs`. Zero new ignored or skipped tests were added.
- **1 pre-existing clippy lint** suppressed via `-A clippy::manual_unwrap_or` per `RULES.md` (`crates/desktop-server/src/evidence.rs:744`).

### What could not be compiled or run on Linux
- `crates/desktop-app` and `cfg(windows)` code paths/tests.
- Running an external `pdflatex` binary on generated `.tex` (verified deterministically with `research_tools::audit_tex_sources` / `tex_audit`: 0 blockers, 0 errors, citations & verbatim quotes verified).

---

## 6. Honest status & out-of-scope bugs noticed (not fixed)

### What does not work yet (waiting on `f15-writeup-solve` merge)
- Live HTTP `GET /api/sessions/{id}/writeup` currently returns HTTP 401 when unauthenticated/cookie-only (verified) and HTTP 404 when authenticated, because `routes_writeup.rs` and its `api_dispatch.rs` arm belong to `f15-writeup-solve`.
- CLI `re-search writeup` (`cmd_writeup.rs`) and UI `"Write up"` menu (`assets/writeup_btn.js`) belong to `f15-writeup-solve` and are not yet mounted.
- `Mode::Solve` and `Mode::Explore` arms in `export::build_session_writeup` return `"write-up adapter not built yet"` until their adapters merge.

### Bugs / oddities noticed in out-of-scope files (left untouched per scope rules)
1. **`crates/export/src/writeup.rs:440`:** `build_verified_quote` stores `text: verified.excerpt` (the full physical line returned by `SourceIndex::verify_pointer`) alongside `quote_ref.quote_sha256` (which hashes the clipped `quote_text`). If a single source line exceeds 300 chars and a clipped `<= 300`-char quote is passed to `build_verified_quote` or `filter_verified_quotes`, `Writeup::validate()` rejects it for both length `> 300` and `quote_sha256` mismatch.
2. **`crates/export/src/writeup_tex.rs:93` vs `crates/research-tools/src/tex_audit.rs:426-437`:** `writeup_tex.rs` runs `escape_latex` inside `\begin{quote}...\end{quote}` while `tex_audit` performs literal normalized substring matching without unescaping LaTeX escapes (`\$`, `\textbackslash{}`). Quotes containing raw TeX math commands therefore trigger `unverified_quote` in `tex_audit`.
3. **`crates/export/src/writeup_md.rs:61`:** `WriteupBlock::Quote` emits raw `q.text` without neutralizing `$`/`\[`/`\(` even when `q.ocr == true`, which could allow a Markdown math renderer to typeset literal `$` delimiters present in a scanned page's text layer.
4. **`crates/export/src/writeup_tex.rs:58` & `crates/export/src/writeup_md.rs:39`:** `include_inline_badges: false` omits the inline `"(model)"` suffix on `ProseBasis::Model` blocks (the closing check table still labels them `"Model statement (model)"`, and `"Authors: ... (model)"` stays intact).
5. **`crates/research-tools/src/library.rs:37` & `crates/research-tools/src/lessons.rs:24`:** Lack `SQLITE_OPEN_READ_ONLY` constructors; opening an *existing* `library.db` or `lessons.db` opens SQLite read-write with WAL and may run migrations or `user_marks` sync.
6. **`crates/desktop-server/assets/investigation_v3_shell.js` (597 lines) & `crates/desktop-server/src/assets.rs` (570 lines):** Near the 600-line limit; see Merge notes above for mounting `writeup_btn.js` in `investigation_share_ui.js` (268 lines) instead.
