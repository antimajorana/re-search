# Report: `exp-x14-eval` — Explore Evaluation Cases: Set I, Set C Chains & Set P6 PDF Readiness (Package X14, Checkpoint 1)

- **Brief:** `docs/design/modes-2026-11/agents/exp-x14-eval.md`
- **Base commit (`origin/gemini/work/fixed`):** `20c6d09938c12951e1c1783ffa4daf244d4a6573`
- **Working branch:** `work/exp-x14-eval`
- **Target push branch:** `gemini/modes/exp-x14-eval`
- **Status:** **Checkpoint 1 COMPLETE & VERIFIED**; **Checkpoint 2 BLOCKED ON UPSTREAM LANES** (`exp-x9-x10-x11-read-verdict` and `exp-x12-x13-map-handoff` are not yet merged on `origin/gemini/work/fixed`, plus an F8 `eval_harness.rs` `Set::C` blocker documented in §6).
- **Commits (chronological, excluding this report commit):**
  1. `dc9b889` `X14 CP1: add synthetic Set I service recordings, user library, and PDF fixtures`
  2. `42a13c5` `X14 CP1: add fixed-tier Set I manifest with Carbon-reviewed answer keys`
  3. `8517284` `X14 CP1: add desktop-server idea_eval harness for Set I recall, X6-X8, PDFs, and F8 sheet`
  4. `50b24dd` `fix(eval): align Set I scripted subclaims and library-only triage_core with manifest-i.json (Carbon HIGH-1)`
  5. `92a8e72` `fix(eval): harden Rule S8/P8/C1/C2 checks, F14a match_pdf_quote, and F8 sheet metadata (Carbon HIGH-2, MEDIUM-1..3, LOW-2/4)`

---

## 1. What Was Implemented (Checkpoint 1, File Inventory & Line Counts)

All **12** added code/fixture files (plus this report) are strictly LF-only (`0` CR bytes) and strictly `<= 600` lines (`python3 scripts/check_file_length.py --base origin/gemini/work/fixed` exits `0`; max Rust file is `372` lines). Zero Do-Not-Edit files (`crates/*/src/`, `Cargo.lock`, `eval_harness.rs`, `eval_support/*`, `manifest-{p,q,b,p5}.json`, `tool_defs.txt`, `tool_profiles.txt`) were touched.

| File | Lines | Purpose |
|---|---:|---|
| `tests/fixtures/eval/manifest-i.json` | 413 | Fixed-tier `Set::I` manifest (`I-01`..`I-10`: `3` done, `2` done under another name, `3` done partly, `2` open as far as known; `2` library-only paywalled key papers) with `answer_key` authored and verified by an independent non-builder Carbon reviewer. Validated cleanly by `eval_harness` with zero edits to `eval_harness.rs`. |
| `tests/fixtures/eval/i/recordings/set_i_services.json` | 566 | 70 frozen self-authored `RoutedMock` HTTP routes across all 5 allowlisted services (`arXiv` Atom XML, `INSPIRE-HEP`, `Crossref` JATS, `Semantic Scholar`, `OpenAlex` inverted-index `verbatim: false`). Strictly excludes both paywalled library-only papers (`I-03`, `I-08`). |
| `tests/fixtures/eval/i/scripts/set_i_exchanges.json` | 581 | Scripted X6 `record_idea_brief` + X8 `record_idea_triage` payloads for `I-01`..`I-10`, aligned 1:1 with `manifest-i.json` `answer_key.parts` and `expected_key_papers` (including library-only `I-03` & `I-08` triaged as `core` into `reading_list`). |
| `tests/fixtures/eval/i/user_library/vance1998_anisotropic_membrane.tex` | 30 | Self-authored synthetic paywalled TeX paper (`doi:10.9999/eval.i03.paywalled`) for `I-03`, indexed via F13a `FolderAdapter`. |
| `tests/fixtures/eval/i/user_library/moretti2004_phonon_drag.pdf` | 83 | Self-authored pure-ASCII 2-page digital text-layer PDF (`doi:10.9999/eval.i08.paywalled`) serving both as `I-08`'s paywalled library paper and as Explore PDF text-layer fixture #1. |
| `tests/fixtures/eval/i/pdfs/i_pdf_01_two_column_journal.pdf` | 88 | Self-authored pure-ASCII 2-page two-column journal PDF (`doi:10.9999/eval.ipdf.twocol`) verifying F14a column reading order (`x=54` vs `x=324`) and physical page mapping. |
| `tests/fixtures/eval/i/pdfs/i_pdf_02_scanned_ocr_1987.pdf` | 99 | Self-authored pure-ASCII 3-page pre-1990 scanned PDF: pp. 1–2 full-page `/Im1 Do` (~83.2% coverage) + `3 Tr` hidden OCR text (`PageBasis::ScanTextLayer`), p. 3 image-only (`PageBasis::None`, `needs_ocr() == true`). |
| `crates/desktop-server/tests/idea_eval.rs` | 344 | 5 integration tests: manifest/non-builder key invariants, 4-regime 0-model-call search-only recall matrix & latency (`< 6.0 s`), scripted X6->X7->X8 + Rule C3/V2/P8/S8 suite, F14a+F4 PDF/scan readiness, and F8 `Sheet` validation/rendering (sharing a `OnceLock` run cache). |
| `crates/desktop-server/tests/idea_eval_support/mod.rs` | 238 | F8 `#[path]` harness bridge (`manifest`, `measures_catalog`, `measures`, `sheet`, `routed_mock`), F13a user-library sync helper, deterministic X1 query helpers, and `ScriptedExploreTransport`. |
| `crates/desktop-server/tests/idea_eval_support/recall.rs` | 229 | Runs all 10 cases across 4 search-only regimes (`keyless-nolib`, `keyed-nolib`, `keyless-lib`, `keyed-lib`) through real `POST /api/sessions`, `POST .../idea/plan`, `POST .../idea/search` (`Router::try_call("host:idea:<sid>", "idea_search")`), asserting `count_model_calls == 0`, Rule V4 (`not_configured`), Rule V10 (library never counted as external service), and Rule P8 key non-leakage in ledger/responses. |
| `crates/desktop-server/tests/idea_eval_support/exchanges.rs` | 372 | Runs scripted X6 Sharpen -> X7 Router SearchPlan -> X8 Triage on all 10 cases, enforcing 1:1 subclaim/key alignment, `expected_key_papers` in `triage.reading_list` as `core`, per-case `xa-*` evidence, Rule C3 OpenAlex `verbatim: false` attempt-1 rejection + ledger fault event on `I-05`, Rule V2 Quick preset (`max_papers == 0`, `papers_fetched == 0`), and Rule P8 ledger/view secret non-leakage. |
| `crates/desktop-server/tests/idea_eval_support/wording_audit.rs` | 220 | Comprehensive Rule S8 forbidden-word property audit across all `idea_view` app-authored labels (real `run_bar.{label,reason,preset_label,cost_line,extras}`, `where_i_looked.{not_searched,scope_summary,library.summary,providers}`, `candidates` badges, `triage` `label_basis`, `plan_approval`, offline `empty_state`), all F12 `ALL_STATIC_CONSTANTS`, and dynamic F12 formatters (`1,131` strings checked, `0` failures). |
| `crates/desktop-server/tests/idea_eval_support/pdf_eval.rs` | 237 | F14a `extract_pdf_pages` + `pdf_match::match_pdf_quote` + `ingest_bytes` + F4 `accept_quote` positive and negative (`QuoteRejection::LinesNotSent`, `QuoteRejection::VerificationFailed`) checks, `\x0c`/cross-page `page_of` separation, OCR tagging, and `scanned_pages_checked`. |

---

## 2. Independent Non-Builder Key Reviewer Notes (`00-overview.md` §7.2 & §8.2 #12)

Before `tests/fixtures/eval/manifest-i.json` was written or any evaluation was run, an independent read-only Carbon reviewer (`research` subagent, `pro` tier) inspected ONLY the 10 raw idea texts, `set_i_services.json`, `user_library/*`, `pdfs/*`, and `eval_support/manifest.rs` (without reading any Explore Rust code or scripted exchanges).

- **Verified clean:** `0` CR bytes across all fixtures, `0` third-party paper text, `0` leaks of `I-03`/`I-08` paywalled papers in `set_i_services.json`, 100% cross-service metadata consistency, exact F2 canonical `PaperKey` priority, and exact PDF stream/page-basis ground truth.
- **Reviewer findings resolved before committing `manifest-i.json`:**
  1. **C.1 (`I-05` `other_name` separation):** Removed `"recursive martingale projections"` from `p_i05_key`'s abstract so it relies solely on the alternative terminology `"detectability lemma"` / `"finite-size Knabe threshold inequalities"`.
  2. **C.2 (`I-06` gate naming):** Aligned `p_i06_key` title/abstract from `"CCZ"` to `"Transversal T Gates"` so `sc-1` matches cleanly while `sc-2` (`linear distance`) remains open (`none_in_scope`).
  3. **C.3 (`I-09` route `[62]` alignment):** Updated `I-09`'s `other_names` (`foliated gauge duality`) route to return `p_i09_bg1` (which discusses foliated excitations) rather than `p_i09_bg2`.
  4. **C.4 (`I-10` lexical alignment):** Added realistic Lieb-Robinson / butterfly-velocity phrasing to `p_i10_bg1` and `p_i10_bg2` (in zero-temperature `alpha > 2` and 1D short-range regimes) while preserving strict separation from `I-10`'s 2D finite-temperature `alpha = 3/2` claim.

---

## 3. Independent Hard Carbon Audit Notes & Resolutions

After committing Checkpoint 1 (`dc9b889`, `42a13c5`, `8517284`), a separate hard read-only Carbon auditor (`research` subagent, `pro` tier) audited the entire diff, fixtures, and tests. Result: **0 CRITICAL, 3 HIGH, 3 MEDIUM, 4 LOW** — every in-scope finding was fixed in commits `50b24dd` and `92a8e72`:

- **HIGH-1 (Fixed in `50b24dd` & `92a8e72`):** Aligned `I-02`, `I-04`, and `I-10` scripted Sharpen `subclaims` 1:1 with `manifest-i.json` `answer_key.parts`, added `I-03` (`doi:10.9999/eval.i03.paywalled`) and `I-08` (`doi:10.9999/eval.i08.paywalled`) paywalled papers to `triage_core`, and asserted 1:1 subclaim and `reading_list` core coverage in `exchanges.rs`.
- **HIGH-2 (Fixed in `92a8e72`):** Replaced broken `run_bar["text"]` check with real `run_bar.{label,reason,preset_label,cost_line,extras[]}` checks and expanded Rule S8 auditing via `wording_audit.rs` across `where_i_looked`, candidate/triage badges, `plan_approval`, offline `empty_state`, and dynamic F12 formatters (`1,131` checked strings, `0` failures).
- **HIGH-3 (Documented in §6):** Verified `X9–X13` are not yet on `origin/gemini/work/fixed` and confirmed `crates/agent/tests/eval_harness.rs:188-193` will reject adding `manifest-c.json` until F8 updates `manifests_are_checked_together`.
- **MEDIUM-1 (Fixed in `92a8e72`):** Added explicit Rule P8 assertions verifying `eval-s2-hermetic-key` and `eval-oa-hermetic-key` never appear in serialized ledger events or HTTP/view payloads in both `recall.rs` and `exchanges.rs`, plus keyless `semantic_scholar == "not_configured"` verification. (Note: `RoutedMock` drops request headers other than `content-length`; header-only forwarding is covered by `research-tools` unit tests `idea_tools_tests.rs`.)
- **MEDIUM-2 (Fixed in `92a8e72`):** Replaced hardcoded date/commit/denominator in `idea_eval.rs` with `today_utc()`, `current_build(&repo_root(), env!("CARGO_PKG_VERSION"))`, measured full `wall_time_s`, max `time_to_candidates_s` across all 4 regimes, `pdf.scanned_pages_checked`, and explanatory `sheet.measure_notes`.
- **MEDIUM-3 (Fixed in `92a8e72`):** Added negative F4 `accept_quote` assertions (`QuoteRejection::LinesNotSent` via `caller_partial` and `QuoteRejection::VerificationFailed` via fabricated quote), production F14a `pdf_match::match_pdf_quote` page/basis assertions, and `\x0c`/cross-page `page_of` checks in `pdf_eval.rs`.
- **LOW-1 (Noted):** `research_tools::idea_queries::QueryService` currently defines 4 variants (`Arxiv`, `Inspire`, `SemanticScholar`, `Crossref`) with no `OpenAlex` variant, so `deterministic_keyed_queries` reuses the plain-text `SemanticScholar` focused keyword query for OpenAlex.
- **LOW-2 & LOW-4 (Fixed in `92a8e72`):** Added per-case `xa-*` checks, `I-05` ledger fault verification, Quick `papers_fetched == 0` assertion, and `OnceLock` shared evaluation cache (cutting `idea_eval` runtime from ~40s to ~28s).
- **LOW-3 (Noted):** `idea_eval` runs with `test_cfg`'s default `auto_approve_tools == true` (still strictly routed through `Router::try_call("host:idea:<sid>", "idea_search")`); interactive `UiApprover` denial + session-grant prompts are already exercised in `crates/desktop-server/tests/idea_search_triage.rs`.

---

## 4. Measured Fixed-Tier F8 Evaluation Sheet (Checkpoint 1)

Measured directly by `cargo test --locked -p desktop-server --test idea_eval -- --nocapture` on commit `92a8e72` (loopback `RoutedMock` + `ScriptedExploreTransport` + F13a user library):

| Measure | Id | Rule | Measured Value | Outcome | Note |
|---|---|---|---:|---|---|
| Explore: time from input to candidates | `time_to_candidates_s` | below 6 | `0.328 s` | **pass** | Max across all 40 loopback sessions (4 regimes × 10 cases). |
| Runs above cap | `runs_above_cap` | must be 0 | `0 count` | **pass** | 10/10 runs strictly below Standard cap (24). |
| Typical model calls per run | `model_calls_typical` | recorded | `2.1 count` | recorded | CP1 Sharpen + Triage only (21 calls / 10 cases, incl. 1 Rule C3 repair on `I-05`); X9–X13 pending CP2. |
| Search recall (keyless services) | `search_recall_keyless` | recorded | `0.6 share` | recorded | `6/10` key papers on arXiv + INSPIRE + Crossref without library. |
| Search recall (with keys) | `search_recall_with_keys` | recorded | `0.8 share` | recorded | `8/10` key papers adding Semantic Scholar + OpenAlex without library. |
| Search recall (without library) | `search_recall_without_library` | recorded | `0.8 share` | recorded | Keyed without library = `8/10 (0.8)`; keyless without library = `6/10 (0.6)`. |
| Search recall (with library) | `search_recall_with_library` | recorded | `1 share` | recorded | Keyed with F10/F13a library = `10/10 (1.0)`; keyless with library = `8/10 (0.8)`. |
| PDF sentence match share | `pdf_sentence_match_share` | recorded | `1 share` | recorded | `7/7` sentences matched via F14a `match_pdf_quote` + F4 `accept_quote`. |
| Quote chip opens wrong page | `pdf_wrong_page_chip` | must be 0 | `0 count` | **pass** | `0/7` wrong physical page chips. |
| Scanned page reported plain text | `pdf_scanned_reported_plain_text` | must be 0 | `0 count` | **pass** | `0/3` scanned/image-only pages misclassified (`2` `ScanTextLayer` + `1` `None`). |
| Two-column wrong reading-order lines | `pdf_two_col_wrong_order_lines` | recorded | `0 count` | recorded | Left column strictly precedes right column on both pages of `i_pdf_01_two_column_journal.pdf`. |
| Forbidden-word property failures | `forbidden_word_property_failures` | must be 0 | `0 count` | **pass** | `0/1131` app-authored view, `run_bar`, `empty_state`, and F12 wording strings. |

**Unmeasured in CP1 (pending Checkpoint 2 / X9–X13):** `earlier_source_found`, `none_in_scope_false_negative_rate`, `judged_same_key_different`, `judged_same_key_different_unreviewed`, `quotes_accepted_per_model_call`, `quotes_rejected`, `quotes_surviving_handover_pct`, `handover_target_solved` (plus non-Explore sets `P`, `P2`, `P3`, `P5`, `B`, `Q`, `W`, `K`, `R`). F8 overall workspace gate stays `INCOMPLETE` as designed (`0` failed).

---

## 5. Live-Tier Owner Checklist (`00-overview.md` §7.1, §7.2, §8.2 #12)

Live-tier manifests and real papers are **never committed** to this repository. Before announcing Explore, the owner runs the live tier outside the repository (`EVAL_LIVE_MANIFEST_DIR=<private_dir>`):

1. **Select 10 real physics/math ideas (`I-01`..`I-10`):** `3` known done, `2` known done under another name, `3` known done partly, `2` open as far as known; ensure `2` ideas rely on a real paywalled paper present ONLY in the owner's local library (F10/F13a).
2. **Independent non-builder answer keys:** Have someone other than the Explore builder write `answer_key.parts`, canonical `PaperKey`s, and expected statuses before running Explore on the live ideas.
3. **Select 4 real `P-to-I` chains** (`2` follow-ups + `2` restated earlier-source results from deferred P4) and **2 real `I-to-Q` chains** (Explore direction -> F7 handoff -> Solve setup/execution).
4. **Select 8 real `Set P6` PDF/scan papers** (`2` journal PDFs with text layer & no arXiv, `2` two-column, `2` pre-1990 scans with publisher text layer, `2` image-only scans) with 20 hand-copied sentences/paper + 30 sampled scan lines.
5. **Validate and file:** Run `EVAL_LIVE_MANIFEST_DIR=<dir> cargo test -p agent --test eval_harness -- --ignored live_manifests_validate` and file the resulting sheet via `file_sheet_from_json` into `docs/design/eval-2026-11/`.

---

## 6. Checkpoint 2 Readiness, Decisions Taken & Merge Notes

### What Blocks Checkpoint 2 Today
1. **Upstream Explore lanes not yet merged on `origin/gemini/work/fixed` (`20c6d09`):**
   - `exp-x9-x10-x11-read-verdict` (`idea_read.rs`, `idea_evidence.rs`, `idea_map.rs`, `idea_verdict.rs`, wired `idea_fetch_papers`, Rule V9 origin exclusion) is absent; `crates/desktop-server/src/routes_idea.rs:330-338` still returns `501` for `read` and `map`.
   - `exp-x12-x13-map-handoff` (`idea_directions.rs`, `idea_handoff.rs`) is absent; `routes_idea.rs:330-338` still returns `501` for `directions` and `handoff`.
2. **Design / Harness Blocker Discovered for `manifest-c.json` (`crates/agent/tests/eval_harness.rs:182-194`):**
   - `eval_harness::manifests_are_checked_together` calls `load_manifests(&fixtures_dir(), Tier::Fixed)` and then unconditionally pushes a synthetic `Set::C` fixed manifest into `all` and asserts `validate_manifests(&all) == []`.
   - `eval_support/manifest.rs:317-319` fails closed with `"more than one fixed manifest for set C"` if `tests/fixtures/eval/manifest-c.json` exists on disk.
   - Because `eval_harness.rs` is Do-Not-Edit for `exp-x14-eval`, a tiny F8 prerequisite fix (`all.retain(|m| m.set != Set::C)` inside `with_chains` in `crates/agent/tests/eval_harness.rs:189`, keeping duplicate-set testing on `Set::P`) MUST land on `gemini/work/fixed` before Checkpoint 2 commits `tests/fixtures/eval/manifest-c.json`.

### Decisions Taken Alone & Assumptions
- **Recall split & service attribution:** Designed the 10 synthetic key papers across `I-01..I-08` so every allowlisted service (`arXiv`, `INSPIRE-HEP`, `Crossref`, `Semantic Scholar`, `OpenAlex`) plus both F13a user-library formats (`TeX` for `I-03`, `PDF` for `I-08`) measurably shift recall (`0.6 -> 0.8 -> 1.0`).
- **Query `origin: "user"` for deterministic search-only recall:** F12 (`crates/research-tools/src/search_coverage_events.rs:155,164`) classifies `origin: "app"` + `relation: "none"` as a seed lookup excluded from `answered_external_services`; deterministic Crossref/S2/OpenAlex keyword recall queries therefore pass `origin: "user"`.
- **Assumption (Windows-only code):** `cfg(windows)` code paths (`appcontainer_sandbox.rs`, `job_memory_limit.rs`, `python_worker.rs`, `re-search-app` WebView2 shell) cannot compile or run on Linux; no Unix-only paths or shell-outs were introduced.

### Merge Notes
- All 13 files on this branch are strictly new files (`manifest-i.json`, `tests/fixtures/eval/i/*`, `crates/desktop-server/tests/idea_eval.rs`, `crates/desktop-server/tests/idea_eval_support/*`, and this report); zero merge conflicts are expected with `X9–X13`.
- When Checkpoint 2 resumes after `X9–X13` + the `eval_harness.rs:189` `Set::C` fix land on `origin/gemini/work/fixed`: merge `origin/gemini/work/fixed` (`--no-ff`), add e-print/PDF fetch routes to `set_i_services.json` (or sibling recording) for the 8 online key papers, extend `idea_eval.rs` through `Read -> Map -> Verdict -> Directions`, add `4 P-to-I` + `2 I-to-Q` chains into `tests/fixtures/eval/manifest-c.json` (`tests/fixtures/eval/c/explore-*`), and run X10 passage reader on the P6-style PDF fixtures.

---

## 7. Non-Root Linux Verification Commands & Measured Outcomes

Environment: Linux x86_64 non-root (`uid=1001(antigravity)`), `RE_SEARCH_PYTHON` unset, `Cargo.lock` and all Do-Not-Edit files untouched.

| Command | Measured Result |
|---|---|
| `id` | `uid=1001(antigravity)` (non-root) |
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | **PASS** (exit `0` — `No touched source file is over 600 lines.`, max `372` lines) |
| `git diff --stat origin/gemini/work/fixed -- Cargo.lock crates/research-tools/tests/golden/ crates/agent/tests/eval_harness.rs crates/agent/tests/eval_support/ tests/fixtures/eval/manifest-p.json tests/fixtures/eval/manifest-q.json tests/fixtures/eval/manifest-b.json tests/fixtures/eval/manifest-p5.json` | **PASS** (empty diff — all Do-Not-Edit files untouched) |
| `for f in crates/desktop-server/assets/*.js; do node --check "$f" \|\| exit 1; done` | **PASS** (exit `0`) |
| `node --test crates/desktop-server/tests/*.test.mjs` | **PASS** (`260 passed, 0 failed, 0 skipped`) |
| `unset RE_SEARCH_PYTHON && cargo test --locked -p agent --test eval_harness` | **PASS** (`16 passed, 0 failed, 2 pre-existing ignored`) |
| `unset RE_SEARCH_PYTHON && cargo test --locked -p desktop-server --test idea_eval` | **PASS** (`5 passed, 0 failed, 0 ignored` in `28.07s`) |
| `unset RE_SEARCH_PYTHON && cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` | **PASS** (exit `0`, `0` warnings) |
| `unset RE_SEARCH_PYTHON && cargo test --workspace --locked --no-fail-fast -- --quiet` | **PASS** (`1410 passed, 0 failed, 2 pre-existing ignored` across `182` targets) |

### Pre-existing / Not Run on Linux
- **2 pre-existing ignored tests:** `#[ignore = "live tier: ..."]` in `crates/agent/tests/eval_harness.rs:468,481` (untouched from `origin/gemini/work/fixed`). Zero skipped or `#[ignore]` tests were added.
- **Pre-existing clippy lint:** `manual_unwrap_or` in `crates/desktop-server/src/evidence.rs:744` (>600 lines, untouched, suppressed via `-A clippy::manual_unwrap_or` per `RULES.md`).
- **Not run on Linux:** Windows-only `cfg(windows)` test suites (`worker/tests/{appcontainer_sandbox,job_memory_limit,python_worker}.rs` run `0` tests on Linux) and real external network / live model calls.
