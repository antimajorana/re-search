# Brief f8-harness-sets — Post-Revision F8 Harness Expansion, Fixed Set B (Own Library), and Fixed Set P5 (Cited Statements)

**Status:** Unblocked on `gemini/work/fixed` (`F8`, `F10`, `F13a`, `F13b`, `F13c`, `V11`, `V17` are merged; `RULES.md` explicitly authorizes this brief to extend `eval_support/{manifest,measures}.rs`, `eval_harness.rs`, and `docs/design/eval-2026-11/sheet-template.md`).
**Packages:** `F8` harness expansion (`Set::{P6, B, W, K}`, `P4 -> C` measure update, library/PDF/write-up/lessons measures, `sheet-template.md`), **`Set B`** fixed-tier own-library sync evaluation (`tests/fixtures/eval/manifest-b.json`), and **`Set P5`** fixed-tier cited-statement locator evaluation (`tests/fixtures/eval/manifest-p5.json`).
**Governing design:**
- `00-overview.md`: 0.1 (P2/P4 deferred; P6/B/W/K added; two P4 cases moved to Set C), 7.1–7.3 (exact sets, answer-key fields, measures, pass rules), 8.2 decision 12.
- `agents/RULES.md` (strict prohibition on loosening existing P/Q/measures assertions or `eval_set_r.rs`, `<= 600`-line limit, zero third-party paper text, push after each checkpoint).

---

## 1. Verified Interfaces & Constraints on `gemini/work/fixed`

- **`crates/agent/tests/eval_support/manifest.rs` (`433` lines):** `Set` currently only has `{P, P2, P3, P4, P5, I, Q, C, R}` (`ALL` length `9`). Missing `Set::{P6, B, W, K}` and their `Input` / `required_key_fields` variants:
  - `Set::P6` (`Mode::Investigate`): PDF-only and old-paper fixtures (`["statements", "sections", "sampled_sentences"]` or matching §7.2).
  - `Set::B` (`Mode::Library` / library mode? Wait: check `manifest.rs` `Mode` enum! Add `Mode::Library`, `Mode::Writeup`, `Mode::Corrections` ONLY if clean, or map per §7.2 — document any `Mode` additions clearly in `manifest.rs` & tests so `P`, `Q`, `I`, `C`, `R` remain unchanged!). For `Set::B` fixed tier: self-made Kandinsky vault / folder / `.bib` planted fixture dir (`Input::LibraryFixture` or `Input::Fixture`), `required_key_fields`: `["items", "planted_cases"]`.
  - `Set::W` (`Mode::Writeup` / `Handover` or new `Mode::Writeup`): `required_key_fields`: `["compiles", "check_table"]`.
  - `Set::K` (`Mode::Corrections`): `required_key_fields`: `["expected_effect"]`.
- **`crates/agent/tests/eval_support/measures.rs` (`530` lines):** already close to the 600-line ceiling! Extract static `MEASURES` catalog / constants into a pure sibling `crates/agent/tests/eval_support/measures_catalog.rs` first if needed so every file stays strictly `<= 600` lines!
  - Replace deferred `Set::P4` references in `measures.rs` (`INVESTIGATE_AND_EXPLORE`, `earlier_source_found`, `none_in_scope_false_negative_rate`, `judged_same_key_different`, `judged_same_key_different_unreviewed`) with `Set::C` (`C (P-to-I), I` per `00-overview.md` §0.1 & §7.2–7.3).
  - Add missing §7.3 measures: `search_recall_with_library`, `search_recall_without_library`, `time_to_skeleton_pdf_s`, `pdf_sentence_match_share`, `pdf_wrong_page_chip` (`Rule::MustBeZero`), `pdf_scanned_reported_plain_text` (`Rule::MustBeZero`), `pdf_scan_char_errors_per_100`, `pdf_two_col_wrong_order_lines`, `sync_second_changes` (`Rule::MustBeZero`), `sync_planted_case_mismatches` (`Rule::MustBeZero`), `sync_source_folder_diffs` (`Rule::MustBeZero`), `sync_removed_source_quote_breaks` (`Rule::MustBeZero`), `writeup_failures` (`Rule::MustBeZero` covering §7.3 W pass rule), `lessons_gate_drift` (`Rule::MustBeZero` covering §7.3 K CI rule), `lessons_error_recurrence`.
- **`crates/agent/tests/eval_harness.rs` (`474` lines, line `86–87`):** currently hardcodes `assert_eq!(sets, [Set::P, Set::Q], ...)` (notice it moved from line 35 to line 87 after PR 33!). Update this cleanly so it asserts all expected required sets present on the branch (`P`, `Q`, and `B`, `P5` once added in CP2/CP3) while also validating *any* additional `manifest-{p3,p6,i,c,w,k}.json` files that future lanes add (`all.contains(&Set::P) && all.contains(&Set::Q) ...` + uniqueness check), **never relaxing any P/Q validation**!
- **Pre-existing external callers:** Check `crates/research-tools/tests/eval_q_cases.rs` or any other test including `eval_support/manifest.rs` or `eval_support/measures.rs` via `#[path]` before pushing — mandatory full `--workspace --all-targets` builds catch all callers!

---

## 2. Checkpoints — Push to `gemini/modes/f8-harness-sets` After Each!

### Checkpoint 1 — F8 Harness Expansion (`P6`, `B`, `W`, `K`, `P4 -> C`, §7.3 Measures, `sheet-template.md`)
1. Extend `manifest.rs`, `measures.rs` (splitting `measures_catalog.rs` if `> 540` lines), `eval_harness.rs`, and `eval_support/sheet.rs` if needed, keeping all existing Set P/Q/measures tests passing and regenerating `docs/design/eval-2026-11/sheet-template.md` via `EVAL_BLESS=1`.
2. Push Checkpoint 1 immediately so `exp-x14-eval` and future `P6`/`W`/`K` lanes unblock as soon as CP1 merges!

### Checkpoint 2 — Fixed-Tier Set `B` (Own Library Sync Eval)
1. Author a self-made synthetic library fixture tree under `tests/fixtures/eval/b/` (LF line endings per `.gitattributes`, zero third-party paper text, zero real user paths):
   - Self-made Kandinsky vault (30 entries in `refs/papers/<key>/<key>.md` + `refs/refs.bib`), plain folder of synthetic PDFs/TeX bundles, and standalone `.bib` file covering **all 7 planted cases** (`00-overview.md` §7.2): moved file (recognised by SHA-256), edited note, deleted/missing file, duplicate by DOI, duplicate by file hash, one paper as preprint + journal version (fuzzy possible duplicate), and path escaping root (rejected!).
2. Add `tests/fixtures/eval/manifest-b.json` and `crates/desktop-server/tests/eval_set_b.rs` running F10/F13a/F13b sync and asserting: second sync changes `0` rows, all 7 planted cases land in expected state, source folder bytes before == after, and removing a source leaves existing session `source.ingested` quote checks passing (`verify_quote`).
3. Push Checkpoint 2 to `gemini/modes/f8-harness-sets`.

### Checkpoint 3 — Fixed-Tier Set `P5` (10 Cited-Statement Lookups)
1. Author 10 self-made synthetic `P5` locator cases (`P5-eval-01`..`P5-eval-10`) in `tests/fixtures/eval/manifest-p5.json` + `tests/fixtures/eval/p5/*`:
   - 7 TeX/HTML cases (exact theorem/lemma/equation locator, Roman vs Arabic equivalence, ambiguous compound locator, missing locator -> `paper_found_locator_not_found`, drifted custom theorem counter -> `candidates`, never wrong `found`) + **at least 3 PDF-only cited-paper cases** (`00-overview.md` §7.2, resolving physical page candidates via F14a/V17). Zero third-party paper text!
2. Add `crates/research-tools/tests/eval_set_p5.rs` executing V11 `locate_in_skeleton_with_context` across all 10 cases, recording F8 metrics (`wrong_statement_located == 0` mandatory pass rule), and validating independent non-builder key consistency.
3. Write `docs/design/modes-2026-11/reports/f8-harness-sets.md`, run full checks, and push to `gemini/modes/f8-harness-sets`.

---

## 3. Do-Not-Edit List & Verification (`RULES.md`)

- **Do NOT edit:** existing `manifest-p.json`, `manifest-q.json`, `tests/fixtures/eval/p/*`, `crates/agent/src/*`, `crates/research-tools/src/*`, `crates/desktop-server/src/*`, `crates/desktop-server/tests/eval_set_r.rs`, `tool_defs.txt`, `tool_profiles.txt`, `Cargo.lock`.
- Run on Linux **as non-root (`id -u != 0`) with `RE_SEARCH_PYTHON` unset**:
  ```bash
  python3 scripts/check_file_length.py --base origin/gemini/work/fixed
  EVAL_BLESS=1 cargo test -p agent --test eval_harness --locked sheet_template_is_the_rendering_of_a_blank_sheet
  unset RE_SEARCH_PYTHON
  cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or
  cargo test --workspace --locked --no-fail-fast
  ```
- Push **only** via `git push origin HEAD:refs/heads/gemini/modes/f8-harness-sets`. Never squash, never open/merge PRs, and end with `PUSHED`.
