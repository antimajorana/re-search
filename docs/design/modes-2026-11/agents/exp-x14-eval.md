# Brief exp-x14-eval — Explore Evaluation Cases: Set I, Set C Chains & Set P6 PDF Coverage (Package X14)

## 1. Package and governing design sections

- **Package:** `X14` (Fixed-tier Set I `10` synthetic Explore cases, Explore's share of Set C chains — `4` `P-to-I` + `2` `I-to-Q` in `tests/fixtures/eval/manifest-c.json` — and Explore reader evaluation on self-authored P6-style PDF/scan fixtures, built on merged F8 harness).
- **Governing design:**
  - `docs/design/modes-2026-11/00-overview.md`: sections 0.1 (`P4 -> C`, `Set P6`, library-only paywalled papers in Set I), 7.1–7.3, and 8.2 decision 12 (owner supplies real live-tier ideas/papers; non-builder reviewer subagent writes/verifies answer keys).
  - `docs/design/modes-2026-11/explore.md`: sections 9 (row X14: Checkpoint 1 search-recall after `X7`, Checkpoint 2 full eval after `X11`/`X13`), and 12 (risks 1, 2, 6).
  - `docs/design/modes-2026-11/agents/RULES.md` (zero third-party paper text in repo; zero live network/model calls in CI tests).

## 2. Start conditions and verified F8 harness state

- **ALREADY MERGED ON `gemini/work/fixed`:**
  - **F8 harness expansion (`f8-harness-sets` PR #54):** `eval_support/manifest.rs` (`536`L, `Set::{P6, B, W, K}` + `Set::I` + `Set::C`), `eval_support/measures.rs` (`279`L) & `measures_catalog.rs` (`272`L, post-revision Set I/C/P6 measures + library-split recall + Rule S8 wording checks), `eval_harness.rs` (`493`L, `required` list `[Set::P, Set::Q, Set::B, Set::P5]` ALREADY permits adding `manifest-i.json` and `manifest-c.json` with ZERO changes to `eval_harness.rs`!).
  - **F7** (`handoff.rs`), **F10/F13a** (user library), **F11 + X19** (`lit_providers.rs` + `lit_openalex.rs`), **F14a** (`pdf_pages.rs` + `pdf_match.rs`), **Solve S11/S12** (setup + execution loop + `Set Q`).
- **Staged Checkpoints (push after each, merging `origin/gemini/work/fixed` before CP2):**
  - **Checkpoint 1 (Search-recall half — `I-01..I-10`):** launchable as soon as **`exp-x6-x7-x8`** is merged on `gemini/work/fixed`.
  - **Checkpoint 2 (`Set I` full + `4 P-to-I` + `2 I-to-Q` in `manifest-c.json` + `Set P6` Explore PDF reader):** requires **`exp-x9-x10-x11-read-verdict`** AND **`exp-x12-x13-map-handoff`** merged on `gemini/work/fixed`. (Check whether Solve `eval-q-cases` / any PR already created `tests/fixtures/eval/manifest-c.json` and merge cleanly into the single `manifest-c.json` — **NEVER create a second `manifest-c-*.json`**, because `eval_support/mod.rs` rejects duplicate fixed manifests for `Set::C`!)

## 3. Mandatory governance rules (`00-overview.md` 7.1–7.3, 8.2 #12)

1. **Non-builder answer keys:** Spawn a separate reviewer subagent (`model: pro` / independent read-only review) who did NOT implement Explore (`X1–X13`) to write or verify `answer_key.parts`, expected key papers, expected per-part statuses, and Set C `target_answer` keys strictly from the synthetic fixtures before running evaluation.
2. **Self-authored fixtures only:** Zero third-party paper text, zero live external network requests (`RoutedMock` loopback only), zero live model calls (`ScriptedModel` only), and zero live-tier manifests committed to the repo (document the live-tier owner checklist in the report instead).

## 4. Smallest implementation checkpoints (push after each!)

1. **Checkpoint 1 — Fixed-tier Set I corpus & search-recall harness:**
   - Author 10 original synthetic Explore ideas (`I-01..I-10`: `3` done, `2` done under another name, `3` done partly, `2` open as far as known; `2` having a key paper ONLY inside a self-made F10/F13a user library, absent from arXiv/INSPIRE/OpenAlex/Crossref) in `tests/fixtures/eval/manifest-i.json` + loopback service fixtures (`tests/fixtures/eval/i/*`, `RoutedMock` frozen responses so `"open as far as known"` never drifts).
   - Have an independent reviewer subagent author/verify `answer_key` from fixtures.
   - `crates/desktop-server/tests/idea_eval.rs` (`<= 600`L; helpers in `crates/desktop-server/tests/idea_eval_support/mod.rs` if needed): run X7 search-only recall across service sets (keyless; with keys; with and without library), verify `time_to_candidates_s < 6.0` on loopback, and validate `manifest-i.json` via `eval_harness`. Push Checkpoint 1 to `gemini/modes/exp-x14-eval`.

2. **Checkpoint 2 — Full Set I, Set C (`4 P-to-I` + `2 I-to-Q`) & Set P6 Explore PDF reader eval:**
   - Merge `origin/gemini/work/fixed` (`X9–X13`) cleanly.
   - Add scripted runs (`ScriptedModel`) for all 10 Set I cases through real Explore entry points (`Sharpen -> Search -> Triage -> Read -> Map -> Verdict -> Directions`).
   - Add Explore's **4 `P-to-I` chains** (`2` follow-ups + `2` restated earlier-source results from deferred P4, exercising Rule V9 origin exclusion) and **2 `I-to-Q` chains** (Explore direction -> F7 handoff -> Solve setup + execution against `target_answer`) into the single `tests/fixtures/eval/manifest-c.json` (`tests/fixtures/eval/c/explore-*`).
   - Evaluate Explore passage reader (X10) on self-authored PDF-only & scanned fixtures aligned with Set P6 (`pdf_wrong_page_chip == 0`, `pdf_scanned_reported_plain_text == 0`, OCR tagging `100%`).
   - Assert every §7.3 pass rule (`runs_above_cap == 0`, `judged_same_key_different_unreviewed == 0`, `quotes_surviving_handover_pct == 100.0`, `forbidden_word_property_failures == 0`, `pdf_wrong_page_chip == 0`, `pdf_scanned_reported_plain_text == 0`) and record all §7.3 recorded measures into an F8 `Sheet` report inside `docs/design/modes-2026-11/reports/exp-x14-eval.md`.

## 5. Owned files & Do-Not-Edit (`<= 600` lines each)

- **New:** `tests/fixtures/eval/manifest-i.json`, `tests/fixtures/eval/i/*`, `tests/fixtures/eval/manifest-c.json` (merge carefully if created earlier — only one Set C manifest!), `tests/fixtures/eval/c/explore-*`, `crates/desktop-server/tests/idea_eval.rs` (+ optional `idea_eval_support/mod.rs`).
- **Do NOT edit:** production code under `crates/*/src/`, `crates/agent/tests/eval_harness.rs`, `crates/agent/tests/eval_support/*`, `manifest-{p,q,b,p5}.json`, `Cargo.lock`, `tests/golden/{tool_defs,tool_profiles}.txt`.

## 6. Mandatory verification & hand-back

- Run `cargo test --locked -p desktop-server --test idea_eval`, `cargo test --locked -p agent --test eval_harness`, `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`, `cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or`, and `cargo test --workspace --locked --no-fail-fast` as non-root with `RE_SEARCH_PYTHON` unset.
- Write `docs/design/modes-2026-11/reports/exp-x14-eval.md` (with independent key-reviewer notes, fixed-tier F8 sheet metrics, and live-tier owner checklist) and push to `gemini/modes/exp-x14-eval`.
