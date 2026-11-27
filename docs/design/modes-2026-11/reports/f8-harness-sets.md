# Report: F8 Harness Expansion, Fixed Set B, and Fixed Set P5 (`f8-harness-sets`)

- **Branch:** `gemini/modes/f8-harness-sets`
- **Base Commit:** `d1b865c39dc934a3432c366f5e7c0328fbee7531` (`origin/gemini/work/fixed` at branch creation)
- **Authoring Scope:** Post-revision F8 harness expansion (CP1), fixed-tier Set B synthetic library sync fixtures + tests (CP2), and fixed-tier Set P5 synthetic cited-statement fixtures + tests (CP3). Zero third-party paper text and zero real user paths.

---

## 1. Checkpoints & File Inventory

### Checkpoint 1 — `c3e01b54b570b172ba3346baee9cc65eab4ac33b`
*`feat(eval): expand F8 harness for sets P6, B, W, K and §7.3 measures`*

| File | Lines | Purpose |
|---|---:|---|
| `crates/agent/tests/eval_support/manifest.rs` | 572 | Added `Set::{P6, B, W, K}`, `Mode::{Library, Writeup, Corrections}`, `LibrarySourceKind`, `MAX_LESSON_CHARS = 300`, `Input::{LibraryFixture, Library, Writeup, Correction}`, directory/auxiliary path helpers, and cross-manifest checks for `C`, `W`, `K`. |
| `crates/agent/tests/eval_support/measures_catalog.rs` | 272 | Extracted pure static catalog from `measures.rs`: replaced deferred `P4` measure references with `Set::C` (`I, C`), added `Set::P6` to statement/section recall, and added all 15 §7.3 measures (total: 55 measures, 24 pass rules, 31 recorded). |
| `crates/agent/tests/eval_support/measures.rs` | 296 | Re-exports catalog items (`#[allow(unused_imports)]` for sibling test binaries) and runs sheet evaluation / pass-rule gating cleanly under `<= 600` lines. |
| `crates/agent/tests/eval_support/mod.rs` | 128 | Exposes `measures_catalog`, checks `fixture_dir_paths` (`is_dir`) and `fixture_auxiliary_paths` (`exists`). |
| `crates/agent/tests/eval_support/f8_expansion_tests.rs` | 236 | Validates `P6, B, W, K` schemas, cross-manifest references, `P4 -> C` migration, and all 55 §7.3 measures. |
| `crates/agent/tests/eval_harness.rs` | 488 | Wired `f8_expansion_tests`, updated expected pass-rule table to 24 rules, replaced hardcoded `[Set::P, Set::Q]` equality with uniqueness + required-set check (`492` lines at CP2, `493` at CP3). |
| `docs/design/eval-2026-11/sheet-template.md` | 90 | Regenerated via `EVAL_BLESS=1`. |

### Checkpoint 2 — `5b2ab17dccdd2c32a4d964d1ab4375a16e7d271d`
*`feat(eval): add fixed-tier Set B library fixtures and sync tests`*

| File | Lines | Purpose |
|---|---:|---|
| `tests/fixtures/eval/manifest-b.json` | 466 | 3 fixed Set B cases: `B-eval-vault`, `B-eval-folder`, `B-eval-bibtex`. |
| `tests/fixtures/eval/b/...` | — | Self-made LF-only Kandinsky vault (30 entries in `refs/papers/<key>/<key>.md`, `refs/refs.bib`, `.kandinsky/plugin-settings/catalog-papers.toml`, `.kandinsky/cache/arxiv-src/2501.00029v1/main.tex`, `b/shared_pdfs/Vega2023_shared.pdf`), plain folder (`b/folder/`), and standalone `.bib` + PDFs (`b/standalone/`), covering all 7 planted cases. |
| `crates/desktop-server/tests/eval_set_b_support/mod.rs` | 68 | Harness `#[path]` bridge (`../../../agent/tests/eval_support/...`) and deterministic read-only directory hash/copy helpers (`hash_tree`). |
| `crates/desktop-server/tests/eval_set_b.rs` | 343 | Executes F10/F13a/F13b sync + desktop-server `/api/sources` route across vault/folder/standalone `.bib`: dry-run preview, 0 changes on second sync (`sync_second_changes`), all 7 planted states (`sync_planted_case_mismatches`), byte-identical source dirs (`sync_source_folder_diffs`), quote survival after `remove_source` + deleted DB/source tree (`sync_removed_source_quote_breaks`), and F8 `evaluate` pass. |
| `crates/agent/tests/eval_harness.rs` | 492 | Added `Set::B` to `required` fixed manifests and matched `Set::P` by predicate in `manifests_are_checked_together`. |

### Checkpoint 3 — `0226250faa7999d81dd015a9d624bb754fcf788d`
*`feat(eval): add fixed-tier Set P5 cited-statement fixtures and tests`*

| File | Lines | Purpose |
|---|---:|---|
| `tests/fixtures/eval/manifest-p5.json` | 267 | 10 fixed Set P5 cases (`P5-eval-01`..`P5-eval-10`): 7 TeX/HTML + 3 PDF-only. |
| `tests/fixtures/eval/p5/*` | — | 10 original LF-only synthetic cited-paper fixtures: TeX shared-counter theorem (`p5_01`), `\tag{A1}` printed equation (`p5_02`), HTML lemma (`p5_03`), HTML Roman `Theorem II` queried via Arabic `Theorem 2` (`p5_04`), TeX ambiguous compound locator (`p5_05`), TeX missing locator (`p5_06`), TeX counter drift + `\setcounter` (`p5_07`), PDF multi-page text layer (`p5_08`), PDF two-column APS layout (`p5_09`), PDF scanned OCR `3 Tr` + full-page XObject + image-only page 3 (`p5_10`). |
| `crates/research-tools/tests/eval_set_p5_support/mod.rs` | 115 | Harness `#[path]` bridge and non-builder ground-truth key checks against raw source text/lines and extracted PDF page records. |
| `crates/research-tools/tests/eval_set_p5.rs` | 428 | Ingests fixtures via `ingest_bytes`, builds `Tex1`/`Html1`/`Pdf1` skeletons, runs V11 `locate_in_skeleton_with_context` + `detect_tex_counter_drift`, verifies physical page/column order/F14a `ScanTextLayer` vs `None` honesty and `wrong_statement_located == 0` pass rule (`evaluate`), plus standalone image-only `NoTex(empty_text_layer)` check. |
| `crates/agent/tests/eval_harness.rs` | 493 | Added `Set::P5` to `required` fixed manifests and asserted `10` cases. |

---

## 2. Deviations, Design Decisions & Unfixed Observations

1. **`Mode::{Library, Writeup, Corrections}` & `LibrarySourceKind`:** Added cleanly to `manifest.rs` matching `00-overview.md` §7.2 modes (`B`, `W`, `K`). Existing `P`, `Q`, `I`, `C`, `R` modes, key fields, and assertions remain unchanged.
2. **`P4 -> C` Measure Retargeting:** Retargeted `earlier_source_found`, `none_in_scope_false_negative_rate`, `judged_same_key_different`, `judged_same_key_different_unreviewed`, and `INVESTIGATE_AND_EXPLORE` from deferred `Set::P4` to `Set::C` (`00-overview.md` §0.1, §7.3).
3. **`measures_catalog.rs` Split & Line-Length Compliance:** Moved the static measure table and `solve_wording` macros out of `measures.rs` (`530` -> `296` lines) into `measures_catalog.rs` (`272` lines) so every code file stays under the 600-line ceiling (`max = 572` lines in `manifest.rs`).
4. **Preprint + Journal Version in Set B:** Modeled with distinct canonical keys (`arxiv:` vs `doi:`) and matching normalized title/author/year so `b2025preprint` and `b2025journal` surface as `SourceItemState::PossibleDuplicate` / `DuplicateConfidence::TitleAuthorYear` without merging.
5. **`manifests_are_checked_together` Ordering Fix (`eval_harness.rs`):** `load_manifests` sorts files alphabetically, so once `manifest-b.json` was added it preceded `manifest-p.json` at index `0`. Fixed `manifests_are_checked_together` to find `Set::P` by predicate instead of `fixed.manifests[0]`.
6. **Observed V11 Gap (`crates/research-tools/src/skeleton/locate.rs:299`, left untouched per Do-Not-Edit rule):** In `P5-eval-04`, querying Arabic `"Theorem 2"` on an HTML paper with Roman `"Theorem II"` triggers `locate.rs:299` (`numbers_equal` compares `a.trim().eq_ignore_ascii_case(b.trim())` without Roman↔Arabic normalization). V11 safely returns `PaperFoundLocatorNotFound` with candidates `[Theorem I, Theorem II]` (zero wrong `Found`, keeping `wrong_statement_located == 0`), while direct `"Theorem II"` returns `Found`. Adding Roman↔Arabic equivalence to `numbers_equal` is left for a V11 bugfix lane outside our Do-Not-Edit boundary.
7. **Two-Column & Scanned Synthetic PDFs (`p5_09`, `p5_10`):** Pure-ASCII `/ASCIIHexDecode` 1x1 Image XObjects (scaled to ~83% of `612x792`) plus `3 Tr` hidden text operators keep fixtures deterministic, compact, and 100% LF-only (`0` CR bytes).

---

## 3. Verification Results (WSL2 Non-Root Linux, `RE_SEARCH_PYTHON` unset)

Full verification executed on committed CP3 state `0226250faa7999d81dd015a9d624bb754fcf788d` via `/var/tmp/research-cargo.sh f8`:

- **Clippy (`--workspace`):**
  `cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` -> **exit `0`** (0 warnings/errors workspace-wide).
- **Clippy (touched crates):**
  `cargo clippy -p agent -p desktop-server -p research-tools --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` -> **exit `0`**.
- **Workspace Cargo Tests:**
  `cargo test --workspace --locked --no-fail-fast -j 2` -> **exit `0`** across `142` test-binary/doc-test runs: **`1143 passed; 0 failed; 2 ignored`**:
  - `agent/tests/eval_harness.rs`: `16 passed; 0 failed; 2 ignored` (the 2 ignored tests are opt-in env-gated live-tier checks `file_sheet_from_json` and `live_manifests_validate`).
  - `agent/tests/eval_p_cases.rs`: `3 passed; 0 failed`.
  - `agent/tests/eval_q_cases.rs`: `1 passed; 0 failed`.
  - `desktop-server/tests/eval_set_b.rs`: `2 passed; 0 failed`.
  - `desktop-server/tests/eval_set_r.rs`: `5 passed; 0 failed`.
  - `research-tools/tests/eval_set_p5.rs`: `2 passed; 0 failed`.
  - **Linux `#[cfg(windows)]` skipped suites:** `worker/tests/appcontainer_sandbox.rs` (`#![cfg(windows)]`) and `worker/tests/python_worker.rs` (`#![cfg(windows)]`) ran `0` tests on Linux (`0 passed; 0 failed; 0 ignored`).
- **JS Syntax & Unit Tests (`node` in WSL2):**
  - `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` -> **exit `0`**.
  - `node --test crates/desktop-server/tests/*.test.mjs` -> **`214 passed; 0 failed`**.
- **Python Unit Tests (`python3` in WSL2):**
  - `PYTHONPATH=python python3 -m unittest discover -s python` -> **`33 tests` OK (`Ran 33 tests in 14.357s`)**.
- **Hygiene & Scope Checks:**
  - `py -3 scripts/check_file_length.py --base d1b865c` & `--base origin/gemini/work/fixed` -> `No touched source file is over 600 lines.` (exit `0`).
  - Frozen-file diff vs `d1b865c` (`manifest-p.json`, `manifest-q.json`, `tests/fixtures/eval/p/*`, `crates/{agent,research-tools,desktop-server}/src/*`, `eval_set_r.rs`, `tool_defs.txt`, `tool_profiles.txt`, `Cargo.lock`, `f9b_dual_scripted.rs`) -> **empty**.
  - Zero `\r` bytes across all generated `b/`, `p5/`, and `manifest-{b,p5}.json` files.

---

## 4. Merge & Downstream Lane Notes

- Downstreams (`Set P6`, `Set W`, `Set K`, `Set C` live/fixed lanes) can build directly on `Set::{P6, B, W, K}` and the 55-measure catalog in `measures_catalog.rs`, and follow the `#[path = "../../../agent/tests/eval_support/..."]` bridge pattern shown in `eval_set_b_support/mod.rs` and `eval_set_p5_support/mod.rs`.
- Note: `origin/gemini/work/fixed` moved forward from `d1b865c` to `a36c11a3352d3329c4f336e80d813aeff62ed3b7` during parallel merges; diffs scoped to `d1b865c..HEAD` or merge-base `origin/gemini/work/fixed...HEAD` cleanly isolate all F8/B/P5 changes with zero frozen-file conflicts.
