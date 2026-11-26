# Brief exp-x14-eval — Explore evaluation cases: Set I and Explore's share of Set C (package X14)

## 1. Package and governing design sections

- **Package:** `X14` (Evaluation cases for Explore — Set I, Explore's share of Set C chains, and Explore reader coverage on Set P6 PDF-only papers, built on shared harness `F8`).
- **Governing design:**
  - `docs/design/modes-2026-11/00-overview.md`: sections 0.1 (scope revision: V14/P4 deferred with 2 restated-result cases moved to Set C as P-to-I chains; Set P6 PDF-only papers added; library-only paywalled papers in Set I), 7.1–7.3 (two-tier layout, Set I / Set C / Set P6 specs, measures and pass rules), and 8.2 decision 12 (owner picks real ideas/papers; non-builder writes answer keys).
  - `docs/design/modes-2026-11/explore.md`: sections 9 (row X14: two checkpoints — search-recall half after X7, full eval after X11) and 12 (risks 1, 2, 6).
  - `docs/design/modes-2026-11/agents/RULES.md` (zero third-party paper text in repository; zero live network/model calls in CI tests).

## 2. Start conditions and mandatory gates

- **Staged checkpoints (with F8 harness-expansion prerequisite):**
  - **Prerequisite — F8 harness expansion (before adding fixed `manifest-i.json` / `manifest-c.json` to `tests/fixtures/eval/`):** verify whether `eval-solve-badges` or a dedicated F8 update has already expanded `crates/agent/tests/eval_support/{manifest,measures}.rs` and `crates/agent/tests/eval_harness.rs:35`. If not, run a scoped F8 harness-expansion prerequisite (with explicit owner/coordinator `RULES.md` authorization to update `eval_harness.rs:35`) covering: `Set::{P6, B, W, K}` in `manifest.rs`, post-revision measures in `measures.rs` (P6 PDF measures, `search_recall_with_library` / `search_recall_without_library`, replacing `Set::P4` with `Set::C` at lines 106–110), and relaxing `eval_harness.rs:35` from `[Set::P, Set::Q]` to include the newly populated sets.
  - **Checkpoint 1 (Search-recall half):** starts once **`X7`** (`exp-x6-x7-x8-search-triage`), `F11`, and the F8 harness expansion are merged on `gemini/work/fixed` (base already has `F8` and `F10`).
  - **Checkpoint 2a (Full Set I, 4 `P-to-I` chains in Set C, & Set P6 Explore PDF reader):** starts once **`X11`** (`exp-x9-x10-x11-read-verdict`), **`X13`** + **`F7`**, **`F13a`** (for 2 library-only paywalled key papers), and **`F14a`** (merged in base) are merged.
  - **Checkpoint 2b (2 `I-to-Q` chains in Set C verifying `target_answer`):** gated additionally on **Solve `S11` / `S12`** + **`F7` Solve-prefill** (and coordinated cleanly on the single `tests/fixtures/eval/manifest-c.json` with Solve's `P-to-Q` chains).
- **Mandatory owner & non-builder governance gates (`00-overview.md` 7.2, 8.2 decision 12):**
  1. **Live-tier inputs are owner-supplied:** the owner chooses the 10 real ideas for live Set I (3 done, 2 done under another name, 3 done partly, 2 open as far as known; 2 with a key paper available only from the user's library) and the real P6 PDFs. Live manifests stay OUTSIDE the repository (`00-overview.md` 7.1).
  2. **Non-builder answer keys:** for BOTH fixed-tier synthetic cases and live-tier cases, a reviewer who is NOT the builder of X1–X13 (e.g., an independent reviewer subagent for fixed-tier keys, as in `eval-p-cases.md` / `eval-q-cases.md`, and an independent human/reviewer for live-tier keys) MUST author or verify the answer keys (`parts`, expected key papers, expected status per part) independently of implementation output.
  3. **Single `manifest-c.json` & harness assertion gate:** `crates/agent/tests/eval_support/mod.rs:95-98` and `manifest.rs:232-234` allow strictly **one** fixed manifest per `Set` (`manifest-c.json` for `Set::C`), and `eval_harness.rs:35` currently asserts `assert_eq!(sets, [Set::P, Set::Q])`. Never create a second `manifest-c-*.json` file.

## 3. Base-commit verified interfaces vs future-merge interfaces

### Verified on base commit `832eab5bf`
- **`crates/agent/tests/eval_harness.rs:35` & `eval_support/mod.rs:95-98` (CRITICAL SEAMS):**
  - `fixed_tier_manifests_load_and_validate()` asserts `assert_eq!(sets, [Set::P, Set::Q], "the skeleton ships self-test cases for P and Q only");`.
  - `eval_support/mod.rs:95-98` loads every `manifest-*.json` in `tests/fixtures/eval/` and `manifest.rs:232-234` rejects more than one `(Set, Tier)` manifest — so all fixed Set C cases (`P -> Q`, `P -> I`, `I -> Q`) MUST live in the single `tests/fixtures/eval/manifest-c.json`.
- **`crates/agent/tests/eval_support/manifest.rs` (434 lines):**
  - Already supports `Set::I` (`Mode::Explore`, `Input::Idea { text }`, required key field `"parts"`) and `Set::C` (`Mode::Handover`, `Input::Chain { from, to }`, allowing `P -> Q`, `P -> I`, and `I -> Q`, required key field `"target_answer"`).
  - **Gap on base `832eab5bf`:** `Set` enum only has `{P, P2, P3, P4, P5, I, Q, C, R}` — lacks `Set::P6`, `Set::B`, `Set::W`, `Set::K`.
- **`crates/agent/tests/eval_support/measures.rs` (279 lines):**
  - Defines `time_to_candidates_s` (`< 6.0 s`), `runs_above_cap` (`== 0`), `model_calls_typical`, `tokens_typical`, `earlier_source_found`, `none_in_scope_false_negative_rate`, `judged_same_key_different`, `judged_same_key_different_unreviewed` (`== 0`), `search_recall_keyless`, `search_recall_with_keys`, `quotes_accepted_per_model_call`, `quotes_rejected`, `quotes_surviving_handover_pct` (`== 100.0`), `handover_target_solved`, `forbidden_word_property_failures` (`== 0`).
  - **Gaps on base `832eab5bf` (updated in F8 harness-expansion prerequisite):** lines 106–110 still reference `Set::P4` (instead of `Set::C` P-to-I chains after P4 deferral), lack `search_recall_with_library` / `search_recall_without_library`, and lack Set P6 PDF measures.
- `crates/agent/tests/eval_support/{routed_mock,scripted,sheet}.rs`: loopback `RoutedMock`, `ScriptedModel`, and Markdown evaluation sheet writer.

### Future-merge interfaces to verify before coding
- **F8 harness expansion / `eval-solve-badges`:** verify `manifest.rs`, `measures.rs`, `eval_harness.rs:35`, and whether Solve already created `tests/fixtures/eval/manifest-c.json` with `P -> Q` cases.
- **`X7`, `X11`, `X13`, `F7`, `F13a`, and (for Checkpoint 2b) Solve `S11`/`S12`:** entry points to drive search-only recall (`X7`), full Explore + `P -> I` chains (`X11`, `X13`, `F7`, `F13a`), and `I -> Q` Solve target verification (`S11`/`S12` + `F7` Solve-prefill).

## 4. Smallest implementation checkpoints

1. **Prerequisite (if not already merged) — F8 harness expansion:**
   - With explicit owner/coordinator authorization under `RULES.md`, update `crates/agent/tests/eval_support/manifest.rs` (`Set::{P6, B, W, K}`), `measures.rs` (`Set::C` replacing `Set::P4` at lines 106–110; `search_recall_with_library` / `search_recall_without_library`; Set P6 PDF measures), and `eval_harness.rs:35` so `tests/fixtures/eval/manifest-i.json` and the single `tests/fixtures/eval/manifest-c.json` load and validate cleanly.
2. **Checkpoint 1 (after X7 + F8 expansion) — Fixed-tier Set I corpus & search-recall harness:**
   - Author 10 original synthetic Explore ideas (`I-01`..`I-10`: 3 done, 2 done under another name, 3 done partly, 2 open as far as known; 2 with a key paper present ONLY in a self-made F10/F13a library, not returned by arXiv/INSPIRE) in `tests/fixtures/eval/manifest-i.json` plus self-authored loopback service fixtures (`tests/fixtures/eval/i/*`, `RoutedMock`, frozen recorded responses so `"open as far as known"` never drifts). Zero third-party paper text.
   - Have an independent reviewer subagent write/verify `answer_key.parts` from the synthetic fixtures before running X7 search.
   - Add Checkpoint 1 tests in `crates/desktop-server/tests/idea_eval.rs`: measure time to candidates (`< 6.0 s` on loopback), known-paper recall of search alone before any model call across service sets (keyless; with keys; with and without library), and manifest validation. Push after Checkpoint 1.
3. **Checkpoint 2a (after X11 + X13 + F7 + F13a) — Full Set I, 4 `P-to-I` Set C chains, & Set P6 PDF reader eval:**
   - Add scripted model runs (`ScriptedModel`) for all 10 Set I cases through the real Explore entry points (`Sharpen -> Search -> Triage -> Read -> Map -> Verdict -> Directions`).
   - Append Explore's **4 `P-to-I` chains** to the single `tests/fixtures/eval/manifest-c.json` (merging cleanly with any Solve `P-to-Q` entries already present): 2 results with a known follow-up paper + 2 restated results whose earlier source is known (`"Search wider in Explore"`, exercising Rule V9 origin exclusion — formerly Set P4).
   - Run the Explore passage reader (X10) on self-authored PDF-only & scanned fixtures aligned with Set P6 (checking page-link accuracy = 100%, OCR tagging = 100%, zero wrong page links).
   - Measure and assert every pass-rule metric of `00-overview.md` 7.3 (`runs_above_cap == 0`, `quotes_surviving_handover_pct == 100%`, `forbidden_word_property_failures == 0`) and record `none_in_scope_false_negative_rate`, `judged_same_key_different`, `quotes_accepted_per_model_call`, `quotes_rejected`, and `model_calls_typical` into the F8 sheet format. Push after Checkpoint 2a.
4. **Checkpoint 2b (after Solve `S11`/`S12` + `F7` Solve-prefill) — 2 `I-to-Q` Set C chains:**
   - Append Explore's **2 `I-to-Q` chains** (Explore direction handed off via `F7` to Solve setup and verified end-to-end against `target_answer`) into the single `tests/fixtures/eval/manifest-c.json` and run them in `crates/desktop-server/tests/idea_eval.rs`. Push after Checkpoint 2b.

## 5. Owned files

- `tests/fixtures/eval/manifest-i.json` and `tests/fixtures/eval/i/*` (new, self-authored only)
- `tests/fixtures/eval/c/explore-*` (new, self-authored chain fixtures only)
- `crates/desktop-server/tests/idea_eval.rs` (new, `<= 600` lines; split helper support into `crates/desktop-server/tests/idea_eval_support/mod.rs` if needed)

## 6. Shared registration seams and "Do not edit" list

- **Shared registration seams (coordinate — never edit concurrently with Solve eval briefs):**
  - `tests/fixtures/eval/manifest-c.json`: single fixed Set C manifest shared between Solve (`3 P-to-Q` chains) and Explore (`4 P-to-I` in Checkpoint 2a + `2 I-to-Q` in Checkpoint 2b). Append only Explore's `C-` entries without touching Solve's.
  - `crates/agent/tests/eval_harness.rs:35` & `crates/agent/tests/eval_support/{manifest,measures}.rs`: edited ONLY in the authorized F8 harness-expansion prerequisite (never silently loosened).
- **Do not edit:** production code in `crates/research-tools/src/`, `crates/agent/src/`, `crates/desktop-server/src/`, existing `manifest-p.json`/`manifest-q.json` self-test cases, `tests/golden/tool_defs.txt`, `tests/golden/tool_profiles.txt`, `Cargo.lock`.

## 7. Tests through the entry point

Run `cargo test -p desktop-server --test idea_eval`, `cargo test --locked -p agent --test eval_harness`, `cargo clippy --workspace --all-targets -- -D warnings`, and `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`.

- Zero real network requests (`RoutedMock` loopback only), zero live model calls (`ScriptedModel` only), zero third-party paper text, zero live-tier scores claimed in CI.
- Verify fixed-tier Set I (10 cases, including 2 library-only paywalled key papers), Set C (4 `P-to-I` in Checkpoint 2a + 2 `I-to-Q` in Checkpoint 2b, 100% quote survival across hand-off), Set P6 synthetic PDFs (0 wrong page links, 0 missed OCR tags), cap compliance (0 runs above cap), and wording invariant (0 `"verified"`/`"new"`/`"novel"`).

## 8. Document conflicts and resolutions

1. **Set C P-to-I chain count:** `explore.md` 9 row X14 originally wrote `"2 from a paper node to an idea, 2 from an idea to a Solve problem"` in its first paragraph, then added `"set C has 4 chains from a paper node to an idea, 2 of them restated results..."` in its scope-revision sentence. **Resolution:** `00-overview.md` 0.1 & 7.2 win: Explore's share of Set C is **4 P-to-I chains** (2 follow-ups + 2 restated results from deferred P4) and **2 I-to-Q chains**.
2. **Base F8 harness vs post-revision sets & single `manifest-c.json`:** Base `eval_harness.rs:35` pins `[Set::P, Set::Q]`, base `eval_support/mod.rs:95-98` + `manifest.rs:232-234` allow strictly one `manifest-c.json` per `(Set::C, Fixed)`, and base `measures.rs:106-110` still lists `P4` and omits library-split recall and `P6`. **Resolution:** gate `manifest-i.json` / `manifest-c.json` on an explicit F8 harness-expansion prerequisite, extend the single `manifest-c.json`, and split Checkpoint 2 into `2a` (`P-to-I` + `Set I` + `P6`) and `2b` (`I-to-Q`, gated on Solve `S11`/`S12`).

## 9. Out of scope and hand-back

- **Out of scope:** Live-tier manifest authoring (owner-only, outside repository), running real LLMs or real external APIs, Investigate Set P/P3/P5 and Solve Set Q cases (owned by `eval-p-cases` / `eval-q-cases`), F15 Set W write-up eval (Group 5), F16 Set K corrections eval (Group 4).
- **Hand-back:** Write `docs/design/modes-2026-11/reports/exp-x14-eval.md` (recording independent key review notes, every measured fixed-tier metric, and the live-tier checklist awaiting owner ideas/keys), commit with `Co-Authored-By`, and push after each checkpoint to `gemini/modes/exp-x14-eval`.
