# Report: `f15-writeup-explore` — F15 Explore Write-Up Adapter (Cited Summary)

- **Branch:** `gemini/modes/f15-writeup-explore` (local `work/f15-writeup-explore`)
- **Initial base commit:** `ffee92f4bb0372a362b2e2781bb3c8da13b7cdaa` (`origin/gemini/work/fixed`, PR #79 `exp-x9-x10-x11-read-verdict`; includes `f15-writeup-solve`, `f15-writeup-investigate`, `f14b-ocr`, `exp-x19-openalex`, `exp-x6-x7-x8`, `exp-x9-x10-x11-read-verdict`)
- **Final synced base commit (merged via `--no-ff`, commit `63c316d`):** `575a72c466fff1b4651c76f174a85a09f32c1c37` (`origin/gemini/work/fixed`, PR #82 `exp-x15-chat`, also includes PR #80 `f8-set-c-harness`)
- **Optional merge status (`exp-x12-x13-map-handoff` / `assets/idea_map.js`):** Not yet merged on `origin/gemini/work/fixed` at `575a72c` → `idea.directions` is rendered when present and omitted cleanly when absent; `assets/idea_map.js` was not created/edited and the exact `writeup_btn.js` mount snippet is provided in **§4 Merge notes**.
- **Date:** 2026-11-28
- **Status:** Complete — Explore cited-summary write-up adapter implemented, wired into `export::build_session_writeup` (`Mode::Explore`), verified through `Tex`/`Md`/`Bib`, `routes_writeup.rs` live HTTP/auth, `cmd_writeup.rs` CLI, and `tex_audit`, hardened against all independent Gemini 4 Carbon audit findings (`C1–C2`, `H1–H5`, `M1–M3`, `L1–L3`), and synced cleanly with `origin/gemini/work/fixed`.

---

## 1. What was implemented (by package/checkpoint & file)

### Checkpoint 1 — Core Explore cited-summary adapter & `Mode::Explore` registration (`549ec1b`, `633ae43`)
- `crates/export/src/writeup_idea.rs` (**512 lines**): Public entrypoints (`build_explore_writeup`, `build_explore_writeup_from_verified`, `build_explore_writeup_detailed`, `ExploreWriteupBuild`, centralized `explore_writeup_wording` constants). Folds the active `idea.stated` revision, `idea.brief`, user-edited `idea.answer` provenance (`ProseBasis::User` vs `ProseBasis::Model`), per-subclaim verdicts, matched quotes (`xe-*` and `xa-*`), `"What differs (model)"` prose, optional `idea.directions`, opening status line, `"Where I looked"` coverage paragraph, and closing `"How each statement was checked"` table with zero model calls and zero network calls.
- `crates/export/src/writeup_idea_cite.rs` (**524 lines**): Stateful per-writeup citation resolver (`IdeaCiteResolver`) and read-only `open_existing_library` (checks `<data_dir>/library/library.db` existence before opening so zero files are created when absent). Pre-verifies `xe-*` quotes via F4 `verify_quote_ref` (with `paper_key` match, `source_id` ownership, `quote_sha256` match, UTF-8 `<= 300`-char clipping, and full unclipped tail verification against the `file_sha256`-verified physical file) and `xa-*` abstract quotes strictly via F11 `extract_abstract_from_blob` (`verbatim == true`, exact `start..end` slice hash match, zero raw-byte fallback). Registers `.bib` entries only for surviving quotes via `writeup_bib::resolve_bib_entry`, preferring user `source_item.citekey` and merging metadata across aliased `PaperKey`s (`doi:` ↔ `arxiv:`).
- `crates/export/src/writeup_idea_ocr.rs` (**449 lines**): Physical 1-based PDF page and OCR provenance detector (`detect_ocr_and_page`) across `idea.paper.text_basis`, `evidence.quote`, and `source.ingested` (`pdf_pages` with `PageBasis::Ocr`/`ScanTextLayer`/`None` or raw strings, `pdf_page_lines`, and PDF scan flags), preferring scanned pages over clean pages on multi-page quotes. Also folds paper metadata/aliases/source ownership from `source.ingested`, `literature.fetched`, `idea.candidates`, and `idea.paper`, plus `QuoteRef` / `PaperRef` / full-quote tail helpers.
- `crates/export/src/writeup_idea_sections.rs` (**560 lines**): Optional `idea.directions` builder (`Concrete` under `"Directions"` first, then `Sketch` under `"Loose ideas"`, strict `map_seq`/staleness binding, ID deduplication, Concrete-before-Sketch check-table ordering, and full V8+S8 validation across all direction fields), F12 `"Where I looked"` coverage paragraph builder (`build_coverage_paragraph`), F9 `explore_stage_run_state` + coverage status line (`build_status_line`, mapping open `requested` stages to `Interrupted` per Rule S3), and Rule V8 / word-boundary Rule S8 validators.
- `crates/export/src/writeup_idea_verify.rs` (**211 lines**): Rule V7/V10 fail-closed stored `idea.verdict` parity check (`verify_stored_verdict_parity` at the stored verdict's own ledger head), Audit C2 iterative fixed-point pre-verification & verdict recomputation loop (`verify_and_recompute_verdicts`) that drops unverified `xe-*`/`xa-*` quotes before emitting prose or `.bib`, and full-document Rule S8 honesty gate (`enforce_explore_honesty`).
- `crates/export/src/lib.rs` (**383 lines**): Registered `pub mod writeup_idea;` and wired `Mode::Explore => writeup_idea::build_explore_writeup_from_verified(data_dir, session, &events)` inside `build_session_writeup` (leaving `Mode::Solve`, `Mode::Investigate`, and `Mode::Research` untouched).
- `crates/export/tests/writeup_seam.rs` (**160 lines**), `crates/export/tests/writeup_paper.rs` (**580 lines**), `crates/desktop-server/tests/writeup_solve_routes.rs` (**265 lines**), `crates/cli/src/cmd_writeup.rs` (**212 lines**): Updated bare Explore marker session assertions from placeholder `"write-up adapter not built yet"` (HTTP 501) to live fail-closed `"idea.stated"` (HTTP 400), matching the exact Investigate precedent from commit `1056c05`.

### Checkpoint 2 — Set W & live HTTP/auth integration tests (`5ee3372`, `151763a`)
- `crates/export/tests/common_writeup_idea/mod.rs` (**409 lines**): Synthetic, zero-third-party-text fixture builders for Explore sessions, arXiv Atom & OpenAlex inverted-index blobs, F4 `evidence.quote` & `evidence.abstract`, `PageBasis::Ocr` receipts, honest `idea.map`/`idea.verdict`, `confirm_same` notes, and read-only library `citekey` setup.
- `crates/export/tests/writeup_idea.rs` (**378 lines**): 3 Set W tests covering `tex_audit` (0 blockers, 0 errors), library `citekey` + read-only fallback (`library.db` never created when absent) across `Tex`/`Md`/`Bib`, all 6 F12 `PriorWorkStatus` states + Rule V6 `confirm_same` + Rule V9 origin exclusion, and Rule V7 tampered verdict / missing brief refusal + Rule V10 F16 lesson & mark invariance.
- `crates/export/tests/writeup_idea_audit.rs` (**479 lines**): 3 Set W tests covering `inline_marks: false` non-removable status/coverage/check-table + Directions optionality & Concrete-before-Loose ordering + Rule V8 refusal, tampered `xe` drop & C2 downgrade + non-verbatim OpenAlex `xa` drop & C2 downgrade + `> 300`-char multi-byte UTF-8 clip + `(OCR)` + physical page + neutralized OCR math delimiters, and Rule S8 wording property across all 7 `ExploreStageState`s and `Tex`/`Md`/`Bib` + byte determinism.
- `crates/desktop-server/tests/writeup_explore_routes.rs` (**453 lines**): Live HTTP/auth route integration test against `GET /api/sessions/{id}/writeup` verifying `AuthGuard::authorize_api` (cookie-only 401, `X-ReSearch-Token` and `Authorization: Bearer` 200), `format=tex|md|bib`, `inline_marks=0`, download/CSP/nosniff/no-store headers, byte-for-byte determinism, `tex_audit` 0 blockers/0 errors on served `.tex` + `.bib`, and fail-closed tampered ledger refusal (HTTP 409).

### Checkpoint 3 — Independent Gemini 4 Carbon Audit & Hardening (`1174c0e`, `e60205b`, `44bba91`, `7461068`)
- `crates/export/src/writeup.rs` (**566 lines**), `crates/export/src/writeup_tex.rs` (**241 lines**), `crates/export/src/writeup_md.rs` (**197 lines**): Added additive `WriteupBlock::AbstractQuote { evidence_id, cite_key, provider_label, text }` (Audit H3) so abstract quotes render real `\cite{cite_key}` in TeX without `\begin{quote}` (keeping `tex_audit` at 0 errors while resolving BibTeX citations) and `[@cite_key]` in Markdown.
- `crates/export/tests/writeup_idea_hardening.rs` (**468 lines**): 4 targeted regression tests covering every Carbon audit finding (`C1`, `C2`, `H1–H5`, `M1–M3`, `L3`).

---

## 2. Independent Gemini 4 Carbon Audit & Fixes Applied

An independent Gemini 4 Carbon (`pro`) audit reviewed all implementation and test files against `origin/gemini/work/fixed` and identified 2 `CRITICAL`, 5 `HIGH`, 3 `MEDIUM`, and 3 `LOW` findings. All findings were fixed and regression-tested:

1. **C1 — Raw-byte fallback in `verify_abstract_quote` bypassed Rule C3 (CRITICAL):** Removed the raw-blob fallback completely (`e60205b`); `verify_abstract_quote` now strictly requires `LitProvider::parse` + `extract_abstract_from_blob` with `ext.verbatim == true` (rejecting malformed OpenAlex blobs, disabled `theoremsearch`, and unknown providers).
2. **C2 — Dropped `xe`/`xa` quotes left stale verdicts, prose, and `.bib` entries behind (CRITICAL):** Added iterative pre-verification + `idea_verdict::compute` recomputation in `writeup_idea_verify.rs` (`e60205b`), splitting `verify_*` and `commit_*` so dropped quotes honestly downgrade subclaim verdicts, remove stale `"What differs"` prose, and never register dropped papers in `.bib`.
3. **H1 — Missing `evidence.quote` paper/source ownership & over-300-char tail verification (HIGH):** Added `event_paper_matches`, `source_belongs_to_paper` (against `source.ingested` origins and `idea.paper.source_id`), and `full_quote_present_in_verified_file` against the `file_sha256`-verified file (`e60205b`).
4. **H2 — Missing `evidence.abstract` `start..end` slice & `paper_key` validation (HIGH):** Added exact `start < end`, `ext.text.get(start..end)`, `quote_sha256(slice) == qsha`, and `event_paper_matches` checks (`e60205b`).
5. **H3 — `xa-*` abstract quotes emitted Markdown `[@cite_key]` syntax into TeX (HIGH):** Added `WriteupBlock::AbstractQuote` across `writeup.rs`, `writeup_tex.rs`, and `writeup_md.rs` (`1174c0e`) so TeX emits `\cite{cite_key}` without `\begin{quote}` and passes `tex_audit` with 0 blockers/0 errors.
6. **H4 — Stage reason `"unverified"` self-crash, incomplete direction field validation, and title/session `"verified"` leak (HIGH):** Filtered `idea.stage.reason` against substring `"verified"` in `build_status_line`, added word-boundary `contains_ascii_word_ci` for Rule S8, validated every direction field (`id`, `method`, `effort`, `node_ids`, `inputs`, `reproduces`, `open_question`), guarded `derive_title` against `"verified"` in `session`, and expanded `enforce_explore_honesty` across titles, sections, app/model prose, statements, and check-table rows (`e60205b`).
7. **H5 — User-edited `idea.answer` mislabeled as `Model` and rejected by Rule V8 (HIGH):** Tracked `restatement_by_user` and `assumptions_by_user` in `ActiveBriefView` so `idea.answer` edits emit `ProseBasis::User` and skip model-prose V8/S8 rejection, while model-authored brief fields check Rule S8 only (Rule V8 applies to `overlap.difference` and `idea.directions`) (`e60205b`).
8. **M1 — Multi-page clean+OCR quote page choice & secondary OCR overlap promotion (MEDIUM):** Tracked `scanned_pages` separately in `detect_ingested_ocr_and_page` to prefer scanned physical pages on multi-page quotes, and restricted subclaim-level `any_ocr`/`ocr_pages` promotion to primary overlaps matching `verdict.status` (matching `prior_work_eval.rs`) (`e60205b`).
9. **M2 — Aliased `PaperKey` (`doi:` vs `arxiv:`) lookup in bibliography and `"What differs"` (MEDIUM):** Added `key_aliases` and symmetric `all_paper_keys` / `lookup_meta` metadata merging across aliased keys (`e60205b`, `44bba91`).
10. **M3 — `idea.directions` staleness, check-table ordering, ID dedup, and malformed items (MEDIUM):** Tightened `map_seq` + `e.seq > lm` staleness filtering, emitted `check_table` rows in Concrete-then-Sketch document order, deduplicated direction IDs, and skipped non-object/empty direction entries (`e60205b`).
11. **L1/L2/L3 — Single `Ledger::open_existing` per build, alias-aware `is_paper_ocr_basis`, and full regression tests (LOW):** Fixed in `e60205b` and verified in `writeup_idea_audit.rs` + `writeup_idea_hardening.rs` (`7461068`).

---

## 3. Decisions, Deviations, and Assumptions

### Decisions taken independently
1. **Additive `WriteupBlock::AbstractQuote` variant:** Adding a dedicated block variant in `writeup.rs` / `writeup_tex.rs` / `writeup_md.rs` was the cleanest way to emit real TeX `\cite{key}` for `xa-*` abstract quotes without triggering `tex_audit`'s `\begin{quote}` full-text-on-disk requirement (since abstracts live in content-addressed `raw_blob`s rather than `source.ingested` files). Existing `Solve` and `Investigate` serialization and outputs are completely unaffected.
2. **Pre-verification & recomputation on dropped quotes (C2):** When a stored `evidence.quote` or `evidence.abstract` passes ledger-level `idea.verdict` parity (Rule V7) but fails physical disk verification (e.g., source modified on disk after `idea.verdict`) or strict F11 `raw_blob` extraction, the adapter drops the quote, recomputes `idea_verdict::compute` without it so the subclaim honestly downgrades, omits the dropped paper from `.bib`, and records `"N quote(s) could not be matched and omitted"` in the check-table note.
3. **Rule V8 vs Rule S8 scope on brief fields:** Rule V8 (`check_model_prose_phrases`) governs `overlap.difference` and `idea.directions`; model-authored `idea.brief` (`restatement`, `assumptions`, `subclaims`) enforces Rule S8 (`"verified"` word-boundary check), while user-authored `idea.stated` and user-edited `idea.answer` fields are tagged `ProseBasis::User` and preserved verbatim.
4. **Placeholder assertion updates:** Updated the 4 existing tests that asserted `"write-up adapter not built yet"` on bare Explore marker sessions (`writeup_seam.rs`, `writeup_paper.rs`, `writeup_solve_routes.rs`, `cmd_writeup.rs`) to assert live fail-closed `"idea.stated"` (HTTP 400), following `f15-writeup-investigate`'s precedent (`1056c05`).

### Deviations from the design
- Split the Explore adapter into five focused modules (`writeup_idea.rs`, `writeup_idea_cite.rs`, `writeup_idea_ocr.rs`, `writeup_idea_sections.rs`, `writeup_idea_verify.rs`) so every file stays comfortably under the 600-line ceiling (max 560 lines).
- Edited `crates/export/src/writeup.rs`, `writeup_tex.rs`, and `writeup_md.rs` (`1174c0e`) strictly to add `WriteupBlock::AbstractQuote` (documented in Merge notes below).

### Assumptions
- `exp-x12-x13-map-handoff` will emit `idea.directions` events with optional `map_seq` and `directions[]` items matching `explore.md` §4.3 (`id`, `title`, `question`, `concreteness`, `known`, `missing`, `first_calculation`, `risks`, `stop_if`). The adapter handles absent, empty, or partial `idea.directions` gracefully.

---

## 4. Merge notes

### Edits to existing files on this branch (vs `origin/gemini/work/fixed` `575a72c`)
- `crates/export/src/lib.rs`: added `pub mod writeup_idea;` and wired `Mode::Explore` into `build_session_writeup`.
- `crates/export/src/writeup.rs`, `crates/export/src/writeup_tex.rs`, `crates/export/src/writeup_md.rs`: added `WriteupBlock::AbstractQuote` for TeX `\cite{}` and Markdown `[@]` abstract quotes (zero changes to existing `Prose`, `Statement`, `Quote`, `CodeCheck`, or `Table` rendering).
- `crates/export/tests/writeup_seam.rs`, `crates/export/tests/writeup_paper.rs`, `crates/desktop-server/tests/writeup_solve_routes.rs`, `crates/cli/src/cmd_writeup.rs`: updated bare Explore session assertions from placeholder 501 `"write-up adapter not built yet"` to live fail-closed 400 `"idea.stated"`.

### Deferred UI mount snippet for `crates/desktop-server/assets/idea_map.js` (when `exp-x12-x13-map-handoff` merges)
`assets/idea_map.js` does not exist on `origin/gemini/work/fixed` (`575a72c`). Once `exp-x12-x13-map-handoff` merges, mount `assets/writeup_btn.js` in the Explore map toolbar using the same stale-download-safe lifecycle proven in `crates/desktop-server/assets/investigation_share_ui.js` (commit `f7a703f`):

```js
import { mountWriteupButton } from './writeup_btn.js';

let activeExploreWriteupCtrl = null;

export function destroyExploreWriteup() {
  if (activeExploreWriteupCtrl) {
    activeExploreWriteupCtrl.destroy?.();
    activeExploreWriteupCtrl = null;
  }
}

// Call inside the Explore map toolbar render when a session is active (and call
// destroyExploreWriteup() when leaving Explore mode or hiding the map shell):
function mountExploreWriteup(toolbar, { sessionId, getToken, fetchFn, urlApi, scheduleRevoke }) {
  destroyExploreWriteup();
  if (!toolbar || !sessionId) return null;
  const slot = toolbar.ownerDocument.createElement('div');
  slot.className = 'explore-writeup-slot';
  slot.setAttribute('data-testid', 'explore-writeup-slot');
  toolbar.appendChild(slot);
  activeExploreWriteupCtrl = mountWriteupButton(slot, {
    sessionId,
    format: 'tex',
    inlineMarks: true,
    getToken,
    fetchFn,
    urlApi,
    scheduleRevoke,
  });
  return activeExploreWriteupCtrl;
}
```

---

## 5. Verification, exact test counts, and Linux limits

### Commands run on merged head (`63c316d`, non-root `uid=1001(antigravity)`, `RE_SEARCH_PYTHON` unset)

| Command | Result |
|---|---|
| `id` | **Non-root** (`uid=1001(antigravity)`) |
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | **Pass** — `No touched source file is over 600 lines.` (max touched file: `crates/export/tests/writeup_paper.rs`, 580 lines; max new file: `crates/export/src/writeup_idea_sections.rs`, 560 lines) |
| Forbidden-file diff check vs `origin/gemini/work/fixed` (`Cargo.lock`, `tests/golden/`, `idea.rs`, `latex.rs`, `writeup_solve.rs`, `writeup_paper.rs`, `http_srv.rs`, `routes_tools.rs`, `routes_session.rs`, `auth.rs`, `search_coverage.rs`, `prior_work_wording.rs`, `quote_core.rs`, `.github/`) | **Pass** — empty diff |
| `for f in crates/desktop-server/assets/*.js; do node --check "$f" \|\| exit 1; done` | **Pass** — 0 syntax errors |
| `node --test crates/desktop-server/tests/*.test.mjs` | **Pass** — **266 passed, 0 failed, 0 skipped** |
| `unset RE_SEARCH_PYTHON && cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` | **Pass** — 0 warnings, 0 errors |
| `unset RE_SEARCH_PYTHON && cargo test --workspace --locked --no-fail-fast` | **Pass** — **195 suites: 1464 passed, 0 failed, 2 ignored** (`writeup_idea`: 3/3, `writeup_idea_audit`: 3/3, `writeup_idea_hardening`: 4/4, `writeup_seam`: 2/2, `writeup_paper`: 7/7, `writeup_explore_routes`: 1/1, `writeup_solve_routes`: 2/2, `tool_def_golden`: 3/3, `tool_profile_golden`: 3/3) |

### Pre-existing items
- **2 ignored tests** in `cargo test --workspace`: pre-existing live-network eval tests in `crates/agent/tests/eval_harness.rs` (`file_sheet_from_json`, `live_manifests_validate`). Zero new ignored or skipped tests added.
- **1 pre-existing clippy lint** suppressed via `-A clippy::manual_unwrap_or` per `RULES.md` (`crates/desktop-server/src/evidence.rs:744`).

### What could not be compiled or run on Linux
- `crates/desktop-app` and `cfg(windows)` code paths/tests (CI runs on Windows 11).
- Running an external `pdflatex` binary on generated `.tex` (verified deterministically with `research_tools::audit_tex_sources` / `tex_audit`: 0 blockers, 0 errors across both pure `xe-*` and mixed `xe-*`+`xa-*` Explore write-ups).

---

## 6. Honest status & out-of-scope bugs/limitations noticed (not fixed)

1. **`crates/research-tools/src/library.rs:37`:** `Library` has no `SQLITE_OPEN_READ_ONLY` constructor (`open_existing`). `writeup_idea_cite.rs::open_existing_library` checks `<data_dir>/library/library.db` with `.is_file()` before opening so absent DBs are never created, but existing DBs still open via SQLite's read-write constructor.
2. **`crates/export/src/writeup_tex.rs:93` vs `crates/research-tools/src/tex_audit.rs:426-437`:** As also noted in `f15-writeup-investigate.md`, `writeup_tex.rs` escapes LaTeX characters inside `\begin{quote}...\end{quote}` while `tex_audit` matches normalized substrings without unescaping TeX commands, so quotes containing raw TeX metacharacters (`\`, `$`, `_`) can trigger `unverified_quote` in `tex_audit`.
3. **`crates/desktop-server/assets/idea_map.js`:** Not yet on `origin/gemini/work/fixed`; the Explore UI `"Write up"` button mount is deferred to integration with `exp-x12-x13-map-handoff` using the snippet in §4.
