# Report: `f15-writeup-solve` — F15 Solve Write-Up Adapter & Shared Seam

- **Brief:** `docs/design/modes-2026-11/agents/f15-writeup-solve.md`
- **Base branch & commit:** `origin/gemini/work/fixed` @ `24be58e6797679cfa423dcc16ea2073b54288714`
- **Local branch:** `work/f15-writeup-solve`
- **Push branch:** `gemini/modes/f15-writeup-solve`
- **Environment:** Linux non-root (`uid=1001(antigravity)`), `rustc 1.100.0 (0ef1bc386 2026-11-09)`, Node `v22.23.5`, Python 3
- **Protected files untouched:** `Cargo.lock`, `crates/research-tools/tests/golden/tool_defs.txt`, `crates/research-tools/tests/golden/tool_profiles.txt`, `crates/export/src/{latex,notebook,zip,paper_map*,writeup_paper,writeup_idea}.rs`, `crates/desktop-server/src/{http_srv,auth,routes_tools,routes_session}.rs`

---

## 1. What Was Implemented (Package by Package)

All 17 touched files are `<= 600` lines (`python3 scripts/check_file_length.py --base origin/gemini/work/fixed` passes cleanly). Commits were kept separate per package and per audit fix (never squashed).

### Checkpoint 1 — F15 Solve Write-Up Adapter (`export`)

- **Commits:**
  - `4edfce3` `feat(export): add F15 Solve write-up adapter (writeup_solve)` (pushed as Checkpoint 1)
  - `8b9093d` `fix(export): address audit findings in writeup_solve and writeup_solve_cite`
- **Files:**
  - `crates/export/src/writeup_solve.rs` (`572` lines, new) — Deterministic `build_solve_writeup(data_dir, session_id, opts) -> Result<Writeup, String>`:
    - Reads the hash-chain-verified ledger via `Ledger::read_verified_session` (refusing tampered chains fail-closed).
    - Folds `SolveState::from_events`, determines the target step (prioritizing `state.run_status` over stale `state.done` when `run_status` is set, then `solve.done.final_entry_id`, then latest `FinalAnswer`, then latest active entry), computes `closure_for_target` and `topological_order`.
    - Enforces Gate **G7 refusal** whenever any closure step has `Tier::Failed`, `Tier::Stale`, or `superseded_by.is_some()`.
    - Enforces Rule **R5** formula provenance: renders every `Claim` exclusively from check-process `solve.expr.latex` keyed by `HandleRef` (refusing missing, non-portable, or empty-latex handles on any claim; ignores `SolveEntry.latex`).
    - Places assumptions first (`Given`, `AssumedByModel`, setup statement/conventions/assumptions), followed by topological step derivation ending in the final answer.
    - Tags prose `ProseBasis::User` (user-supplied `Given` entries and setup text) vs `ProseBasis::Model` (solver/verifier steps).
    - Enforces Rule **R22** on model-authored step text via `solve_done::lint_solve_summary_r22` (rejecting forbidden overclaiming words such as `"verified"` / `"proved"` when `effective_tier != Tier::Verified`), emits the unconditional opening status line and closing `"How each statement was checked"` table, and reserves the word `"verified"` strictly for `Tier::Verified` rows/status.
  - `crates/export/src/writeup_solve_cite.rs` (`541` lines, new) — Citation, quote verification, OCR, and BibTeX resolution helper:
    - Re-verifies every `Cited` quote through F4/F15 `build_verified_quote` against the session workspace sandbox (`<= 300` chars; unmatched/tampered quotes refused or dropped cleanly).
    - Detects OCR from `EvidenceQuoteRecord.ocr`, raw event payload `"ocr"` flag, and `pdf_pages::PageBasis::ScanTextLayer` / `empty_text_layer` (including fallback page lookup across `start_line..=end_line`), emitting `ocr: true` -> `"(OCR)"` and physical PDF page locators.
    - Resolves `.bib` entries via `source_item.citekey` first, then `Library::get_paper` (only opening `library.db` when `data_dir/library/library.db` already exists, keeping export read-only), then fallback citation metadata.
  - `crates/export/tests/common_writeup_solve/mod.rs` (`198` lines, new) — Synthetic Solve session ledger/workspace/library fixtures (zero third-party paper text).
  - `crates/export/tests/writeup_solve.rs` (`360` lines, new) — Set W tests: `audit_tex_sources` (`tex_audit`) clean pass, R5 formula provenance & refusal of missing/non-portable handles, `source_item.citekey`-first `.bib` resolution, deterministic byte-for-byte output, Gate G7 refusal (`Failed`, `Stale`, `superseded_by`), OCR tag + physical PDF page + tamper refusal.
  - `crates/export/tests/writeup_solve_r22.rs` (`395` lines, new) — R22 property sweep across all 11 `Tier` variants (with `inline_marks` on and off), `own_tier == Verified` resting on `Unchecked` honesty check, raw `"ocr"` flag, `> 300`-char quote cap, WAL-aware ledger hash-chain tamper refusal, and post-audit regression tests (`run_status` precedence, R22 step-prose lint, non-ASCII hex prefix UTF-8 safety, OCR page-number fallback).

### Checkpoint 2 — Shared Write-Up Seam, HTTP Route, CLI, and UI Button

- **Shared Seam (`export`):**
  - Commits: `dacb565` `feat(export): add shared WriteupFormat and writeup_session seam`, `c9b9da8` `fix(export): sanitize bib_stem and guard .bib sidecar collision`.
  - Files:
    - `crates/export/src/lib.rs` (`318` lines, edited) — Exports `pub mod writeup_solve;`, `WriteupFormat::{Tex, Md, Bib}`, `writeup_session_bytes`, and `writeup_session_to_path`. Detects session mode from the verified ledger (`Mode::Solve` wired to `build_solve_writeup`; `Mode::Explore` and `Mode::Investigate` return `"write-up adapter not built yet"`). For `WriteupFormat::Tex`, `writeup_session_to_path` sanitizes `bib_stem`, refuses a `.bib` primary output path (preventing self-overwrite collision), and writes `<stem>.tex` alongside its `<stem>.bib` sidecar.
    - `crates/export/tests/writeup_seam.rs` (`160` lines, new) — Tests format parsing, mode dispatch (`Solve`, `Explore`, `Investigate`, legacy/Research), `writeup_session_bytes`, `writeup_session_to_path` sidecar `.bib` generation, `bib_stem` sanitization, and `.bib` collision refusal.
- **Asset Refactor (`desktop-server`, per `RULES.md` line 89):**
  - Commit: `595b389` `refactor(desktop-server): extract KaTeX font arms into assets_fonts.rs`.
  - Files:
    - `crates/desktop-server/src/assets_fonts.rs` (`74` lines, new) — Standalone zero-behavior-change extraction of KaTeX font match arms.
    - `crates/desktop-server/src/assets.rs` (`494` lines, edited; reduced from 571L to 490L, then 494L with `writeup_btn.js`).
    - `crates/desktop-server/src/lib.rs` (`252` lines, edited; declares `assets_fonts` and `routes_writeup`).
- **Authenticated HTTP Route (`desktop-server`):**
  - Commits: `466ca02` `feat(desktop-server): add GET /api/sessions/{id}/writeup route`, `321c9fb` `fix(desktop-server): strip leading dots in writeup download filename`.
  - Files:
    - `crates/desktop-server/src/routes_writeup.rs` (`144` lines, new) — `GET /api/sessions/{id}/writeup?format=tex|md|bib&inline_marks=0|1`. Strictly header-token authenticated via `AuthGuard::authorize_api` in `http_srv.rs` (never added to `auth.rs` stream-cookie routes). Serves `Content-Disposition: attachment`, `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`, `Content-Security-Policy: sandbox`. Maps Gate G7 to HTTP `409 Conflict`, unbuilt mode adapters to HTTP `501`, missing session to `404`, bad query/id to `400`. Sanitizes download filenames and strips leading `.` characters.
    - `crates/desktop-server/src/api_dispatch.rs` (`484` lines, edited) — 3-line route dispatch arm for `("GET", [id, "writeup"])`.
    - `crates/desktop-server/tests/writeup_solve_routes.rs` (`247` lines, new) — End-to-end TCP HTTP tests verifying header-token auth (401 without token and 401 with cookie only), `tex`/`md`/`bib` downloads, `inline_marks=0|1`, security headers, Gate G7 409 refusal, and unbuilt Explore/Investigate 501 response.
- **CLI Subcommand (`cli`):**
  - Commits: `a314678` `feat(cli): add re-search writeup subcommand`, `61a73df` `fix(cli): accept --no-inline-marks=bool and log canonical format`.
  - Files:
    - `crates/cli/src/cmd_writeup.rs` (`212` lines, new) — `re-search writeup SESSION --format tex|md|bib --out PATH [--no-inline-marks] [--data-dir DIR]`, including unit tests for `tex` + sidecar `.bib`, `md`, `bib`, `--no-inline-marks`, `--no-inline-marks=false`, G7 refusal, unbuilt Explore refusal, and rejection of `--no-inline-marks` on unrelated subcommands.
    - `crates/cli/src/cmd.rs` (`474` lines, edited) — Declares `#[path = "cmd_writeup.rs"] mod cmd_writeup;`, adds `USAGE` line, `"writeup"` dispatch arm, and boolean flag handling for `--no-inline-marks` and `--no-inline-marks=VALUE`.
- **Desktop UI Write-Up Button (`desktop-server/assets`):**
  - Commits: `4d625e1` `feat(desktop-ui): add writeup_btn.js module and Node tests`, `6ca934b` `fix(desktop-ui): guard stale session write-up downloads on session switch`.
  - Files:
    - `crates/desktop-server/assets/writeup_btn.js` (`347` lines, new) — Pure-DOM (`document.createElement` / `textContent`, zero `innerHTML`) `mountWriteupButton`, `fetchWriteup`, `writeupUrl`, format selector (`tex`/`md`/`bib`), `inline_marks` toggle, `X-ReSearch-Token` header auth, blob download + `URL.revokeObjectURL` cleanup, and monotonic `reqSeq` + `targetSession` stale-response guard on `update({ sessionId })` and `destroy()`.
    - `crates/desktop-server/tests/writeup_btn.test.mjs` (`285` lines, new) — 5 Node tests covering URL/filename helpers, header-token fetch + blob download + zero `innerHTML`, XSS-safe Gate G7 error rendering, `/assets/writeup_btn.js` registration + `auth.rs` non-cookie assertion, and stale in-flight download cancellation on session switch.

---

## 2. Independent Carbon Audit & Fixes

An independent audit (`pro` subagent) reviewed the entire branch against `00-overview.md`, `solve.md`, `f15-writeup-solve.md`, and `RULES.md`. It reported **7 Major** and **6 Minor** findings:

| # | Severity | Finding | Resolution |
|---|---|---|---|
| 1 | Major | Stale `state.done` could shadow newer `state.run_status` when selecting target/status | Fixed in `8b9093d`: `state.run_status` now takes precedence over `state.done`; added regression test in `writeup_solve_r22.rs`. |
| 2 | Major | Model-authored step `summary`/`details` were not checked by `lint_solve_summary_r22` for R22 banned wording | Fixed in `8b9093d`: enforced `solve_done::lint_solve_summary_r22` on non-`Verified` model prose; added regression test. |
| 3 | Major | `file_key_from_hex_prefix` sliced `&s[..16]` without ASCII/hex guard (UTF-8 panic risk) | Fixed in `8b9093d`: guarded `is_ascii()` and ASCII hex check before slicing; added non-ASCII regression test. |
| 4 | Major | OCR fallback checked only exact `start_line` and case-sensitive basis string | Fixed in `8b9093d`: scans `start_line..=end_line` with page-number fallback and case-insensitive `ScanTextLayer` / `empty_text_layer` matching; added regression test. |
| 5 | Major | `writeup_session_to_path` accepted unsanitized `bib_stem` and allowed `.bib` output path collision under `WriteupFormat::Tex` | Fixed in `c9b9da8`: added `sanitize_bib_stem`, aligned sidecar `.bib` filename, and refused `.bib` primary output path for `Tex`; added regression tests in `writeup_seam.rs`. |
| 6 | Major | (a) `writeup_btn.js` in-flight download race on `update({ sessionId })`; (b) button not yet mounted in `solve_steps.js` | (a) Fixed in `6ca934b` via monotonic `reqSeq` + `targetSession` check + Node regression test. (b) S13 `solve_steps.js` is not yet merged on base; documented exact 1-line mount call under **Merge notes** per brief line 36. |
| 7 | Major | Final report `docs/design/modes-2026-11/reports/f15-writeup-solve.md` not yet written | Written and committed last (this file). |
| M1 | Minor | Dangling dependency IDs dropped from `rests_on` | Fixed in `8b9093d`: preserved dangling deps in `rests_on`. |
| M2 | Minor | `http_srv.rs` lacks `501 => "Not Implemented"` reason phrase (emits numeric `501` with fallback `"Internal Server Error"`) | Not fixed because `http_srv.rs` is Do-Not-Edit; recorded in §6 below. |
| M3 | Minor | `setup.assumptions` omitted from Setup/Assumptions section | Fixed in `8b9093d`: rendered `setup.assumptions` alongside conventions. |
| M4 | Minor | `safe_download_filename` could leave leading `.` (e.g., `.hidden.tex`) | Fixed in `321c9fb` + unit test in `routes_writeup.rs`. |
| M5 | Minor | CLI only handled bare `--no-inline-marks`, not `--no-inline-marks=false`, and logged raw `fmt_raw` | Fixed in `61a73df` + regression assertion in `cmd_writeup.rs`. |
| M6 | Minor | `WriteupBlock::Quote` renders via F15 core `quote` environment rather than LaTeX footnotes | Intentional compatibility with golden-locked F15 core `writeup_tex.rs`; recorded in §3 below. |

---

## 3. Deviations from the Design & Rationale

1. **Standalone `writeup_btn.js` pending S13 (`solve_steps.js`):**
   - `solve_steps.js` (owned by S13) does not exist on `origin/gemini/work/fixed`, and `mode_stages.js` is outside this brief's owned files. Following brief line 36 (`note 1-line mount call in solve_steps.js if S13 not yet merged`), `writeup_btn.js` is registered at `/assets/writeup_btn.js` and tested standalone, with the exact 1-line mount call recorded under **Merge notes**.
2. **`#[path = "cmd_writeup.rs"] mod cmd_writeup;` inside `crates/cli/src/cmd.rs`:**
   - The brief lists `crates/cli/src/cmd.rs` (not `crates/cli/src/main.rs`) as an edited existing file. Declaring `cmd_writeup` from `cmd.rs` keeps `crates/cli/src/main.rs` completely untouched while keeping `cmd.rs` at 474 lines.
3. **TeX Quote Environment (`writeup_tex.rs`):**
   - `solve.md` mentions footnote-style citations in passing, whereas F15 core (`crates/export/src/writeup_tex.rs`, already merged and outside our owned files) renders `WriteupBlock::Quote` as a verified `\begin{quote}...\end{quote}` block with `\cite{key}` and `"(OCR)"` locator tags. We emit standard `WriteupBlock::Quote` entries so the Solve adapter stays 100% compliant with F15 core and passes `audit_tex_sources`.

---

## 4. Decisions Taken Alone & Assumptions

- **HTTP status mapping in `routes_writeup.rs`:**
  - Missing/invalid `session_id` or bad `format`/`inline_marks` -> `400 Bad Request`.
  - Unknown session -> `404 Not Found`.
  - Gate G7 refusal (`Failed`, `Stale`, or `superseded_by.is_some()` in closure) -> `409 Conflict`.
  - Unbuilt adapters (`Mode::Explore`, `Mode::Investigate`) -> `501` (`"write-up adapter not built yet"`).
  - Unauthenticated or cookie-only request -> `401 Unauthorized` (enforced by `AuthGuard::authorize_api` before dispatch).
- **Read-only library lookup:** `writeup_solve_cite.rs` checks `data_dir.join("library").join("library.db").is_file()` before calling `Library::open`, so exporting a session that has no library database never creates files in `data_dir`.
- **Claimless non-portable handles (`solve.md` 4.3):** Any `Claim` referencing a missing, non-portable, or empty-latex handle is refused fail-closed (Rule R5). Non-portable handles that are not referenced by any `Claim` (for example, auxiliary solver scratch expressions) are ignored rather than blocking a valid proof closure.
- **Mode detection in `export/src/lib.rs`:** Inspects session events using `research_tools::tool_profile` mode markers (`SOLVE_STARTED`/`SOLVE_SETUP`, `MARKER_IDEA_STARTED`, `MARKER_INV_STARTED`); sessions with no Solve/Explore/Investigate marker return a clear error rather than mis-rendering a Research session as a Solve proof.
- **`bib_stem` sanitization (`export/src/lib.rs`):** Strips directory separators and non-ASCII/unsafe TeX characters from `bib_stem` so `\bibliography{<stem>}` always matches the emitted `<stem>.bib` sidecar filename and cannot traverse directories or collide when `--out` ends with `.bib`.
- **Assumption:** On Windows (`cfg(windows)`), `std::path::absolute` and path joins in `cmd_writeup.rs` and `export/src/lib.rs` work identically to Linux because no Unix-specific paths, symlinks, or shell invocations were used.

---

## 5. Merge Notes

### Edits to Existing Files

| File | Lines | Change | Merge Risk / Coordination |
|---|---:|---|---|
| `crates/export/src/lib.rs` | 318 | Added `pub mod writeup_solve;`, `WriteupFormat`, `writeup_session_bytes`, `writeup_session_to_path`, and `build_session_writeup` dispatcher. | **Explore / Investigate F15 adapters:** wire `Mode::Explore` and `Mode::Investigate` arms inside `build_session_writeup` in `crates/export/src/lib.rs` (currently returning `Err("write-up adapter not built yet".into())`). |
| `crates/desktop-server/src/lib.rs` | 252 | Added `mod assets_fonts;` and `pub mod routes_writeup;`. | Tiny 2-line additive diff; trivial to merge. |
| `crates/desktop-server/src/api_dispatch.rs` | 484 | Added `("GET", [id, "writeup"]) => { routes_writeup::serve_writeup(stream, state, id, query); true }` arm. | 3-line additive arm in `dispatch_session_route`. |
| `crates/desktop-server/src/assets.rs` | 494 | Extracted KaTeX font arms into `assets_fonts.rs` (commit `595b389`) and registered `"/assets/writeup_btn.js"` (commit `4d625e1`). | Gives other lanes ~86 lines of headroom under the 580L/600L ceiling. |
| `crates/cli/src/cmd.rs` | 474 | Declared `#[path = "cmd_writeup.rs"] mod cmd_writeup;`, added `"writeup"` arm, `USAGE` entry, and `--no-inline-marks[=VALUE]` parsing. | Localized to `USAGE`, `parse_args`, and `run_args` match; `cli/src/main.rs` untouched. |

### 1-Line Mount Call for S13 (`solve_steps.js`)

When S13 (`crates/desktop-server/assets/solve_steps.js`) merges, import and mount the write-up button in the Solve header/toolbar container:

```js
import { mountWriteupButton } from '/assets/writeup_btn.js';
// Inside Solve steps view init (pass sessionId and header token accessor):
const writeupCtrl = mountWriteupButton(headerActionsEl, { sessionId, getToken: () => window.__reSearchToken });
// On session switch: writeupCtrl.update({ sessionId: nextSessionId });
// On unmount: writeupCtrl.destroy();
```

---

## 6. Verification Commands & Exact Measured Results

All commands were executed as non-root (`uid=1001(antigravity)`) with `RE_SEARCH_PYTHON` unset after the final code commit (`61a73df`):

1. **Identity & Protected Files & File Lengths:**
   ```bash
   id
   python3 scripts/check_file_length.py --base origin/gemini/work/fixed
   git diff origin/gemini/work/fixed -- Cargo.lock crates/research-tools/tests/golden/tool_defs.txt crates/research-tools/tests/golden/tool_profiles.txt
   ```
   - Result: `uid=1001(antigravity)`, `No touched source file is over 600 lines.` (max touched file: `crates/export/src/writeup_solve.rs` at `572` lines), zero diff on `Cargo.lock`, `tool_defs.txt`, and `tool_profiles.txt`.

2. **Full Workspace Clippy:**
   ```bash
   unset RE_SEARCH_PYTHON
   cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or
   ```
   - Result: **Passed (`exit 0`, 0 warnings, 0 errors).**

3. **Full Workspace Test Suite:**
   ```bash
   unset RE_SEARCH_PYTHON
   cargo test --workspace --locked --no-fail-fast
   ```
   - Result: **Passed (`exit 0`) across 155 Rust test binaries/doc-test suites: `1208 passed; 0 failed; 2 ignored`.**
   - New/touched test suites included in the run:
     - `crates/export/tests/writeup_solve.rs`: `4 passed; 0 failed; 0 ignored`
     - `crates/export/tests/writeup_solve_r22.rs`: `4 passed; 0 failed; 0 ignored`
     - `crates/export/tests/writeup_seam.rs`: `2 passed; 0 failed; 0 ignored`
     - `crates/desktop-server/tests/writeup_solve_routes.rs`: `1 passed; 0 failed; 0 ignored`
     - `desktop-server` unit test `routes_writeup::tests::safe_download_filename_strips_leading_dots`: `1 passed; 0 failed`
     - `cli` unit test `cmd::cmd_writeup::tests::cli_writeup_writes_tex_sidecar_bib_md_and_respects_no_inline_marks`: `1 passed; 0 failed`
     - `crates/research-tools/tests/tool_def_golden.rs`: `3 passed; 0 failed`
     - `crates/research-tools/tests/tool_profile_golden.rs`: `3 passed; 0 failed`

4. **Node Asset Syntax & Test Suite:**
   ```bash
   for f in crates/desktop-server/assets/*.js; do node --check "$f" || exit 1; done
   node --test crates/desktop-server/tests/*.test.mjs
   ```
   - Result: **Passed (`exit 0`): all `assets/*.js` syntax-checked cleanly; `231 passed; 0 failed; 0 skipped` (including 5 new tests in `crates/desktop-server/tests/writeup_btn.test.mjs`).**

---

## 7. Pre-Existing Notes

- **2 pre-existing `#[ignore]` live-tier tests in `crates/agent`** (present on `origin/gemini/work/fixed` base commit `24be58e6797679cfa423dcc16ea2073b54288714`):
  - `file_sheet_from_json` (`ignored, live tier: needs EVAL_SHEET_JSON`)
  - `live_manifests_validate` (`ignored, live tier: needs EVAL_LIVE_MANIFEST_DIR`)
  - Zero `#[ignore]` or skipped tests were added by this brief.
- **Pre-existing Clippy lint:** `crates/desktop-server/src/evidence.rs:744` (`clippy::manual_unwrap_or`), already documented in `RULES.md` and left untouched.

---

## 8. What Could Not Be Compiled or Run on Linux

- `cfg(windows)` code paths (AppContainer sandbox, Win32 job objects, WebView2 desktop shell) cannot be compiled or executed on this Linux cloud VM.
- Live browser/WebView2 rendering of `writeup_btn.js` and real `pdflatex` compilation of generated `.tex` files were not run; `writeup_btn.js` was verified via Node mock-DOM tests (`writeup_btn.test.mjs`) and `.tex` output was verified via `research_tools::audit_tex_sources` (`tex_audit`).

---

## 9. Bugs Noticed But Not Fixed (Out of Scope / Do-Not-Edit)

1. **`crates/desktop-server/src/http_srv.rs:358-372`:** `write_bytes`'s HTTP status reason-phrase `match` lacks an explicit `501 => "Not Implemented"` arm, so any `501` response (both existing `routes_idea.rs` and new `routes_writeup.rs`) sends status line `HTTP/1.1 501 Internal Server Error` (numeric status `501` is intact). Not fixed because `http_srv.rs` is explicitly Do-Not-Edit under `RULES.md` and this brief.
2. **`crates/export/src/writeup.rs:439-440`:** `build_verified_quote` returns `verified.excerpt` (the full matched line range from F4 `verify_quote_ref`) rather than clamping back to the exact quoted substring when a caller passes a shorter substring on a long source line; if a source line exceeds 300 characters, `verified.excerpt` can exceed 300 characters even though `quote_text` passed the `<= 300` check on line 426. Not fixed because `writeup.rs` is F15 core (already merged and not in our owned files).

---

## 10. Plain Statement of Current Limitations

- `Mode::Explore` and `Mode::Investigate` write-up adapters are intentionally not implemented in this brief and return `"write-up adapter not built yet"` (CLI error / HTTP `501`) until their respective F15 adapter lanes merge.
- `writeup_btn.js` is served at `/assets/writeup_btn.js` and unit-tested, but is not yet mounted into the live Solve UI because S13 (`solve_steps.js`) is not yet merged on `origin/gemini/work/fixed` (see §5 **Merge notes**).

---

## 11. Post-Push Carbon (`pro`) Independent Audit Hardening

An independent Carbon (`pro`) audit identified two additional correctness/read-only edge cases that were fixed on top of `de44a2a` without rewriting history:

1. **Commit `95d76ef` (`crates/export/src/writeup_solve.rs` `577`L, `crates/export/tests/writeup_solve.rs` `396`L):** Live `harvest_exprs_out` stores `owner_entry: None` on `solve.expr` events; `build_step_formulas` now matches `entry.tool_call_ref` / `entry.tool_call_refs` via `research_tools::proof_dag::tool_reference_matches` when `owner_entry` is `None` (mirroring `solve_steps.rs::handles_with_effective_owners`), with regression test `claimless_step_resolves_live_owner_entry_none_handles_via_tool_call_refs`.
2. **Commit `64773bc` (`crates/desktop-server/src/routes_writeup.rs` `157`L, `crates/desktop-server/tests/writeup_solve_routes.rs` `262`L):** Replaced `state.session_exists` (which opens `Ledger::open` and creates `ledger.db` / `blobs/` on disk) with a read-only `Ledger::open_existing` existence check so `GET /api/sessions/{id}/writeup` performs zero disk writes on `404`, and mapped `"ledger verification failed"` to `409 Conflict`, with regression test `writeup_route_404_is_strictly_readonly_before_ledger_exists`.
3. **Additional out-of-scope note:** `crates/research-tools/src/library.rs:37` / `crates/ledger/src/library_db.rs:31` (`Library::open`) currently lacks a read-only opener (`Library::open_existing_readonly`), so when `library.db` already exists on disk, SQLite opens it read-write (WAL/migrations). `writeup_solve.rs` avoids creating `library.db` when absent; adding a read-only opener is left for the library lane.

**Final non-root verification after `64773bc` (`uid=1001(antigravity)`, `RE_SEARCH_PYTHON` unset):**
- `cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or`: **Passed (`exit 0`, 0 warnings/errors)**
- `cargo test --workspace --locked --no-fail-fast`: **Passed (`exit 0`) across 155 test binaries: `1210 passed; 0 failed; 2 ignored`** (`export/writeup_solve`: `5 passed`, `desktop-server/writeup_solve_routes`: `2 passed`)
- `for f in crates/desktop-server/assets/*.js; do node --check "$f" || exit 1; done && node --test crates/desktop-server/tests/*.test.mjs`: **Passed (`exit 0`, `231 passed; 0 failed; 0 skipped`)**
- `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`: **Passed (`No touched source file is over 600 lines.`)**

