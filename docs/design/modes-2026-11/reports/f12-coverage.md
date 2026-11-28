# Report: `f12-coverage` (Coverage gate and prior-work wording — package F12)

- **Brief:** `docs/design/modes-2026-11/agents/f12-coverage.md`
- **Base commit:** `7e3dac2055b5f27f5dbc1548cb50e240e9ad82af` (`origin/gemini/work/fixed`)
- **Work branch:** `work/f12-coverage` -> pushed to `gemini/modes/f12-coverage`
- **Toolchain (Linux VM):** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `cargo 1.100.0 (3acc79c0c 2026-11-05)`, `python3 3.12.16` (`sympy 1.14.0`, `numpy 2.5.5`, `mpmath 1.3.0`), `node v22.23.5`
- **Guard rails:** `Cargo.lock` and all `Cargo.toml` files untouched; `crates/research-tools/tests/golden/tool_defs.txt` untouched (`tool_def_golden` passes 3/3); zero tool definitions or profiles added/modified; zero network/model calls or routes; `policy` and `ledger` untouched; zero `.unwrap()`, `.expect()`, `panic!`, `todo!`, `unimplemented!`, `unreachable!`, or `unsafe` in non-test code; `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` passes (max 548 lines <= 600).

---

## 1. What was implemented (package F12)

Pure, deterministic Rust modules in `crates/research-tools/src/` built on F2 (`paper_key`), F4 (`quote_core`, `lines_sent`), F11 (`lit_providers`, `lit_abstracts`), and S2's shared display scale (`solve_status::{DisplayGrade, BorderStyle}`).

### `search_coverage` — Coverage record, Explore presets, and completeness gate

| File | Lines | Contents |
|---|---:|---|
| `crates/research-tools/src/search_coverage.rs` | 206 | Public facade re-exporting `search_coverage_types::*`, event kind & scope constants (`MIN_ANSWERED_EXTERNAL_SERVICES = 2`, `IDEA_*_EVENT`, `IDEA_CORE_DISMISSED_EVENT`, `NOTE_ACTION_DISMISS_CORE`, `NOT_SEARCHED_DEFAULT`), input structs (`CoverageInput`, `QueryCoverageRecord`, `CandidateCoverageRecord`, `TriageCoverageRecord`, `PaperReadCoverageRecord`), `compute_coverage`, `compute_coverage_from_events`, `coverage_matches_recomputed` (rule V7), and `paper_ref_matches_key` (delegating to F11's `lit_abstracts::paper_matches_key`). |
| `crates/research-tools/src/search_coverage_types.rs` | 446 | Private helper re-exported by `search_coverage`: `ExplorePreset` (`Quick`/`Standard`/`Thorough`) & `PresetLimits` matching `explore.md` section 3, `PerspectiveKind`, `QueryOutcomeStatus` (implementing `From<lit_providers::QueryStatus>` + `SkippedBudget`), `ProviderCoverageState`, `ProviderCoverage` (explicitly tracking `queries_ok`, `queries_zero_results`, `queries_failed`, `queries_rate_limited`, `queries_skipped_budget`, `queries_unsupported`, `seed_lookup_queries_ok`, `cached`, `hits`), `LibraryCoverage`, `PerspectiveCoverage`, `TriageRelevance`, `PaperReadStatus`, `PaperReadCounts`, `CorePaperGap`, `UnresolvedCorePaper`, `CitationExpansionCoverage`, `KnownPaperRecall`, `GateFailure` (9 tagged variants naming every unsatisfied condition), `CoverageGate`, `SearchCoverage`. |
| `crates/research-tools/src/search_coverage_eval.rs` | 388 | Private aggregation and gate evaluation: `aggregate_providers` (excluding direct `is_seed_lookup` queries from search-answer credit and handling explicit `Partial` states cleanly), `aggregate_perspectives`, `summarize_candidates`, `summarize_reads_and_triage`, `compute_seed_recall` (alias-aware via F2 `PaperRef`), and `evaluate_gate` (evaluating all conditions without short-circuiting). |
| `crates/research-tools/src/search_coverage_events.rs` | 504 | Private tolerant `serde_json::Value` ledger extractor for `compute_coverage_from_events`: folds `idea.started`, `idea.stated`, `idea.brief`, `idea.search_planned`, `idea.search_finished`, `literature.fetched`, `idea.candidates`, `idea.citation_net`, `idea.triage`, `idea.paper`, `idea.core_dismissed`, and `dismiss_core`/`undismiss_core` notes; ignores `lessons.supplied` and F16 ranking marks (rule V10); preserves `literature.fetched` hit/cache counts across event orders and merges incremental multi-round triage/candidate records. |
| `crates/research-tools/src/search_coverage_tests.rs` | 492 | 12 unit tests covering exact preset limits, F11 `QueryStatus` mapping, V4 `not_configured`/`disabled` listing, Quick preset rejection, private library exclusion from external service minimum, failed/rate-limited/partial attempted services, `exact` & `other_names` perspective gates, `screened >= min(preset, candidates)`, core paper reading vs explicit dismissal, seed recall (including alias matching and non-crediting of seed lookups), zero results vs no answer, and V7/V10 ledger recomputation + serde round-trip. |
| `crates/research-tools/src/search_coverage_audit_tests.rs` | 250 | 5 adversarial regression tests from the independent Carbon audit: `literature.fetched` -> `idea.search_finished` ordering & INSPIRE/S2 `id` alias extraction, direct seed lookup exclusion from service/perspective minimums, explicit `Partial` provider state failing `no_attempted_service_failed`, multi-round triage preserving Round 1 `Core` papers / merging candidate `PaperIds` / rejecting `budget.max_screened: 0` / `Unsupported` & `SkippedBudget` counters / 0-seed citation hops, and re-sharpened `idea.brief` invalidating stale perspective bindings. |

### `prior_work` — Shared prior-work statuses and pure verdict evaluator

| File | Lines | Contents |
|---|---:|---|
| `crates/research-tools/src/prior_work.rs` | 504 | Shared 6-state `PriorWorkStatus` (`judged_same`, `judged_partial`, `judged_other_name`, `abstract_only`, `none_in_scope`, `incomplete`), re-exported `DisplayGrade` & `BorderStyle` (`00-overview.md` 2.10), typed F4 `EvidenceId` (`xe-<seq>` / `xa-<seq>`, rejecting `seq == 0`), `TextBasis` (`Tex`/`PdfText`/`ScanTextLayer`/`Ocr`), `OverlapRelation`, evidence/overlap/verdict/input/report structs, `OverlapRejectionReason`, `origin_ref_from_key`, `is_origin_paper`, `quote_contains_term`, `evaluate_prior_work`, `compute_prior_work_from_events`, and `report_matches_recomputed` (rule V7). |
| `crates/research-tools/src/prior_work_eval.rs` | 210 | Private per-subclaim evaluator enforcing V1 (full-text `xe-*` from `read_full`/`read_passages` paper; `judged_other_name` term in full-text quote/excerpt), V2 (verbatim abstract cap at `abstract_only`; Quick preset never `none_in_scope`), V3 (`none_in_scope` iff `coverage.gate.passed`), V6 (`confirmed_by_user` only on `judged_*`), V8 (`MAX_DIFFERENCE_CHARS` & banned phrase rejection on `difference`), V9 (alias-aware origin exclusion), M2 (primary status/OCR scoping), and M6/R1 (rejecting unknown/dangling/mismatched `xe-*` and `xa-*` IDs). |
| `crates/research-tools/src/prior_work_events.rs` | 493 | Private ledger extractor reading `handoff.packet`, `idea.stated`, `idea.brief`, `idea.candidates`, `literature.fetched`, `source.ingested` (`pdf_pages` & `files_sha256`), `idea.paper`, `evidence.quote`, `evidence.abstract`, `idea.map`, and `confirm_same`/`reject_same` notes. Performs hybrid-PDF per-page OCR detection via `source.ingested.pdf_pages`, validates F4 quote/abstract digests and `source_id/` path prefix, enriches origin aliases from candidates/fetches, and invalidates stale `confirm_same` on `idea.stated`, re-sharpened `idea.brief`, or changed `idea.map` overlap signatures. |
| `crates/research-tools/src/prior_work_tests.rs` | 474 | 6 unit tests covering wire names & `EvidenceId` parsing, V1 read-paper & full-text requirements + confirmation transitions, `judged_other_name` full-text term requirement (rejecting abstract/difference-only terms), V2 abstract cap & non-verbatim exclusion & Quick preset negative, V9 origin exclusion & OCR tagging, and V7/V10 ledger recomputation + `confirm_same`/`reject_same`. |
| `crates/research-tools/src/prior_work_audit_tests.rs` | 484 | 6 adversarial regression tests from the Carbon audit: alias-based V9 origin exclusion (via `handoff.packet` and later `idea.candidates`), hybrid-PDF per-page OCR detection from `source.ingested.pdf_pages` on standard F4 `evidence.quote` payloads, malformed F4 event / zero `EvidenceId` / dangling `xe-*` rejection, stale `confirm_same` invalidation on changed `idea.map` and `idea.stated`, mixed-relation overlap isolation (`Same` TeX + secondary `Partial` OCR) & V8 `difference` enforcement, and dangling/cross-paper `xa-*` rejection + stale map clearing on `idea.stated`. |

### `prior_work_wording` — App-authored wording, scope disclosure, attribution, and guards

| File | Lines | Contents |
|---|---:|---|
| `crates/research-tools/src/prior_work_wording.rs` | 548 | Named `wording` constant table (chips, `"Model judges:"`, `"(model)"`, `"Reader confirmed:"`, 2.10 display-scale labels, `"OCR"`, `"possible relevance"`, V8 banned phrases, forbidden generated words, `ALL_STATIC_CONSTANTS`), `provider_display_name`, `status_chip`, `display_grade_label`, `display_grade_border`, `format_provider_coverage`, `format_library_coverage`, `format_gate_failure`, `format_coverage_scope`, `format_verdict_sentence` (separating primary vs secondary overlaps, disclosing failing gate conditions on both `Incomplete` and `AbstractOnly`, and guarding Quick's `"No close match among N abstracts; nothing was read"` against failed/unsearched runs), `format_abstract_evidence_note` (300-char UTF-8 safe clipping via F4 `clip_quote`), and `format_model_text`. |
| `crates/research-tools/src/prior_work_wording_check.rs` | 89 | Private helper re-exported by `prior_work_wording`: `check_model_prose_phrases` (rule V8 with zero-width/soft-hyphen/combining-mark stripping and Latin typographic ligature expansion) and `contains_forbidden_prior_work_word` (masking only strict canonical F2 `PaperKey` tokens via `PaperKey::parse`). |
| `crates/research-tools/src/prior_work_wording_tests.rs` | 376 | 8 unit and exhaustive property tests covering all static constants, exhaustive combinations over all 6 statuses x 2 confirmation states x 3 presets x 2 OCR states x 7 provider states x 6 providers x all 10 `GateFailure` fixtures verifying zero `"new"`, `"novel"`, `"novelty"`, `"verified"`, `"proved"`, or `"unexplored"` outputs, scope/zero-results/no-answer/failing-condition disclosures, Quick failed search vs `AbstractOnly` incomplete gate & DOI masking, singular/plural & `NotAttempted` details, 300-char UTF-8 abstract clipping, model attribution & 2.10 display scale, and V8 absolute-phrase + Unicode bypass checks. |

### Wiring edit (separate commit)

| File | Lines | Change |
|---|---:|---|
| `crates/research-tools/src/lib.rs` | 308 (+3) | Added `pub mod prior_work;`, `pub mod prior_work_wording;`, and `pub mod search_coverage;` in commit `3b66cba`. |

---

## 2. Independent Carbon audit summary

An independent read-only adversarial audit (Carbon / `pro` model) reviewed all F12 code and tests in two rounds:
- **Round 1 findings (all fixed):** H1 (`literature.fetched` -> `idea.search_finished` overwriting `hits`/`cached`), H2 (V9 origin exclusion missing F2 aliases), H3 (per-page OCR detection from `source.ingested.pdf_pages` on real F4 `evidence.quote` payloads), H4 (unvalidated F4 `evidence.quote`/`evidence.abstract` events and `EvidenceId(0)`), H5 (stale `confirm_same` persisting across changed `idea.map` / `idea.stated`), M1 (`is_seed_lookup` counting toward external service & perspective minimums), M2 (mixed-relation verdict wording & OCR flag conflation), M3 (explicit `Partial` provider state bypassing `no_attempted_service_failed`), M4 (multi-round `idea.triage` wiping Round 1 core papers, candidate `PaperIds` merging, INSPIRE/S2 `id` extraction, `budget.max_screened: 0`, re-sharpened `idea.brief`), M5 (Unicode ZWSP/ligature V8 bypasses, `validate_overlap` enforcing V8/`MAX_DIFFERENCE_CHARS`, Quick-mode failed wording, `AbstractOnly` incomplete disclosure, DOI masking), M6 (dangling `xe-*` skipped), and L1 (pluralization, `unavailable_core` vs `unresolved_core`, `NotAttempted` details, `citation_net` 0-seed hops, `lit_abstracts::paper_matches_key` reuse).
- **Round 2 re-audit remaining items (all fixed):** R1 (rejecting dangling/cross-paper `xa-*` in `validate_overlap`), R2 (`idea.stated` and re-sharpened `idea.brief` clearing stale `overlaps`/`prev_sigs`/`current_map_seq` and including `difference` in overlap signatures), R3b (`path` prefix `{source_id}/` and `source.ingested.files_sha256` verification), R3c (clearing `planned` perspective bindings on re-sharpening and preserving non-zero `literature.fetched` hits over `0`), L1 (replaced stub with real `idea.candidates` alias-enrichment test), L2 (strict `PaperKey::parse` in `mask_paper_key_tokens`), L3 (filtering `screened_limit_override >= 1` in `compute_coverage`), L4 (requiring `ProviderCoverageState::Ok` in `answered_with_zero_results`).

---

## 3. Deviations, decisions taken alone, and assumptions

### Deviations from the design
1. **Private helper submodules for the 600-line rule (`RULES.md`):** Split `search_coverage` into `search_coverage_{types,eval,events}.rs`, `prior_work` into `prior_work_{eval,events}.rs`, and `prior_work_wording` into `prior_work_wording_check.rs`, matching the existing `lit_providers_*` / `solve_*` pattern. External callers still import exclusively from `research_tools::{search_coverage, prior_work, prior_work_wording}`.
2. **`ProviderCoverageState::NotAttempted` added:** `explore.md` 4.1 lists `ok | partial | failed | rate_limited | not_configured | disabled`. Added `not_attempted` so a configured service whose queries were all skipped by budget, unsupported, seed-lookup-only, or unrun is unambiguously distinguished from both `ok` (answered with 0 results) and `not_configured`/`failed`.
3. **Explicit core-paper dismissal contract (`IDEA_CORE_DISMISSED_EVENT` / `NOTE_ACTION_DISMISS_CORE`):** `explore.md` rule V3 allows explicit user dismissal of an unavailable triage-core paper, while rule V10 states that F16 marks (`not_relevant`, `pin`, `key_paper`, `wrong`, `comment`) change ranking only and are never read by the coverage gate. Separating explicit core dismissal (`idea.core_dismissed` or `idea.note` with `action: "dismiss_core"` / `"undismiss_core"`) satisfies both V3 and V10 without ambiguity.

### Decisions taken alone
1. **Zero results vs no answer:** Any non-seed-lookup search query returning `QueryOutcomeStatus::Ok` with `hits == 0` counts as answered (`queries_ok += 1`, `queries_zero_results += 1`, `answered() == true`) and can pass the service and perspective gates when no failures occur (`zero_candidates_from_answered_search == true`). Failed, rate-limited, skipped, disabled, unsupported, not-configured, or unrun queries never count as answered (`has_no_answer() == true`).
2. **`PaperReadStatus::NothingRelevant`:** Satisfies rule V3 (full text was fetched and read, resolving a triage-core paper) while `can_support_judged_overlap` remains strictly `ReadFull | ReadPassages` per rule V1.
3. **Overlap status precedence & secondary wording:** When a subclaim has multiple valid full-text overlaps, status precedence is `JudgedSame > JudgedOtherName > JudgedPartial`; primary status wording and `has_ocr_evidence` reflect only overlaps matching `status`, while other accepted overlaps appear under `"Additional recorded overlap: ..."`.
4. **F4 `evidence.abstract` `verbatim` compatibility:** Because F4's `EvidenceAbstractPayload` (`crates/research-tools/src/quote_core.rs:369-381`) has `#[serde(deny_unknown_fields)]` and omits a `verbatim` field (enforcing rule C3 at write time), `prior_work_events` rejects explicit `verbatim: false` and validates all F4 fields (`start < end`, 64-hex `raw_blob`, matching `quote_sha256`, non-empty `provider`), treating F4-valid `evidence.abstract` records as verbatim while rejecting ungrounded records.
5. **Confirmed verdict sentence prefix:** Used `"Reader confirmed:"` (`wording::READER_CONFIRMED_PREFIX`) with chip `"found (same)"` / `"found (partly)"` / `"found under another name"`, keeping `"Model judges:"` strictly for unconfirmed model judgments.
6. **Overlap V8 enforcement at readback:** `validate_overlap` enforces `MAX_DIFFERENCE_CHARS = 600` and `check_model_prose_phrases` (rule V8). Downstream X11 (`record_idea_map`) must reject overlaps failing V8/length before appending `idea.map`, because rejecting an already-stored `Same` overlap on readback would drop it from the verdict.
7. **S14 (`independence`) and V14 (Investigate prior work):** Intentionally omitted per `f12-coverage.md` and `00-overview.md` 2.9.

### Assumptions
- Assumed Explore's `idea.search_planned` distinguishes direct seed lookups from citation expansion by `seed_lookup: true` or (`origin == "app"` and `relation` absent/`"none"`).
- Assumed `source.ingested.pdf_pages` records `basis` values matching F14a `PageBasis` (`"text_layer"`, `"scan_text_layer"`, `"none"`).

---

## 4. Merge notes

### Existing file edited
- `crates/research-tools/src/lib.rs` (commit `3b66cba`, +3 lines): added `pub mod prior_work;`, `pub mod prior_work_wording;`, `pub mod search_coverage;`.

### Downstream interfaces (X7–X11, F15, S14, future V14)
- **X7 (Plan execution & "Where I looked"):** Call `search_coverage::compute_coverage` or `search_coverage::compute_coverage_from_events`, and render disclosures via `prior_work_wording::{format_coverage_scope, format_provider_coverage, format_library_coverage, format_gate_failure}`. When emitting `idea.search_finished`, include `qid`, `provider`, `status` (matching `QueryOutcomeStatus`), and optional `providers` (`not_configured`/`disabled`) and `library` summaries; direct seed lookups should set `origin: "app"` (with `relation: "none"`) or `seed_lookup: true`. For explicit reader dismissal of an unavailable triage-core paper (V3), emit `idea.core_dismissed` (`{ paper_key }`) or `idea.note` with `action: "dismiss_core"`.
- **X8 / X10 (Triage & passage reader):** Continue writing F4's `evidence.quote` and `evidence.abstract` events (`quote_core`) and F14a's `source.ingested` (`pdf_pages`, `files_sha256`). F12 automatically infers per-page OCR provenance from `source.ingested.pdf_pages` even when `evidence.quote` has no `text_basis` field.
- **X11 (`record_idea_map` and `idea_verdict`):** Validate model prose (`difference`, directions) with `prior_work_wording::check_model_prose_phrases` before writing `idea.map`. Compute verdicts and coverage at one head via `prior_work::evaluate_prior_work` or `prior_work::compute_prior_work_from_events`. Display chips/sentences directly from `PriorWorkVerdict::{chip, text, display_grade, border_style, has_ocr_evidence}` and attribute model `difference` prose with `prior_work_wording::format_model_text`.
- **X17 / F15 (Write-up & export):** Verify stored `idea.verdict` payloads on export with `search_coverage::coverage_matches_recomputed` and `prior_work::report_matches_recomputed` (rule V7).
- **S14 (Second-model independence):** Owns the `independence` (`different_provider | different_model | same_model`) host function (`00-overview.md` 2.9).
- **V14 (Deferred Investigate prior-work check):** Will reuse `PriorWorkStatus`, `SearchCoverage`, `CoverageGate`, and `prior_work_wording` directly when un-deferred.

---

## 5. Verification commands and exact results

| Command | Exit | Exact result |
|---|---:|---|
| `cargo test -p research-tools --lib -- search_coverage prior_work` | 0 | **37 passed; 0 failed; 0 ignored** (163 filtered out) |
| `cargo test -p research-tools --test tool_def_golden` | 0 | **3 passed; 0 failed; 0 ignored** |
| `cargo clippy -p research-tools --lib -- -D warnings` | 0 | **0 warnings, 0 errors** |
| `cargo test --workspace --no-fail-fast` | 0 | 106 test suites: **867 passed; 0 failed; 2 ignored** (pre-existing ignores in `crates/agent/tests/eval_harness.rs`) |
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | 0 | `No touched source file is over 600 lines.` |
| `cargo clippy --workspace --all-targets -- -D warnings` | 101 | Fails solely on pre-existing Linux warnings (Section 6); **0 warnings** in any file added or touched by F12 (confirmed via `--cap-lints warn`) |

---

## 6. Pre-existing clippy warnings on Linux (unchanged from base)

Confirmed via `cargo clippy --workspace --all-targets -- --cap-lints warn` (identical to those already recorded on the base branch in `docs/design/modes-2026-11/reports/f4-f9-quote-budget.md` section 6; none are in files touched by this brief):
1. `crates/desktop-server/src/evidence.rs:744` — `clippy::manual_unwrap_or` (also documented in `RULES.md`).
2. `crates/research-tools/tests/sandbox_and_fs.rs:6` — `unused_imports`: `TEX_AUDIT`, `is_symlink_or_junction` (used only under `#[cfg(windows)]`).
3. `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` — `unused_imports`: `BufRead`, `BufReader` (used only under `#[cfg(windows)]`).
4. `crates/agent/tests/eval_harness.rs` — 2 pre-existing clippy lints (`match` looks like `matches!`, `map_or` simplification).
5. `crates/agent` lib tests — 1 pre-existing clippy lint (`items after a test module`).

Note: Running `cargo test --workspace` with `RE_SEARCH_PYTHON` pointed at another workspace's virtualenv caused `cli::tests::desktop_and_ext::cli_research_workspace_read_fork_diff_export_and_verify` to stall past 60 s; running `cargo test --workspace --no-fail-fast` directly completed cleanly with 867 passed, 0 failed.

---

## 7. What could not be compiled or run on Linux

- All `#[cfg(windows)]` code paths and Windows-only integration tests (`crates/worker/tests/{appcontainer_sandbox,python_worker}.rs`, `crates/cli/tests/{contained_router,init_abort}.rs`, `crates/desktop-app`, and Windows DPAPI vault tests in `crates/chatgpt-auth`).
- No files under `crates/desktop-server/assets/` were touched, so `node --check` / `node --test` were not required.

---

## 8. Design tensions and observations noted (not modified)

1. **`explore.md` V3 vs V10 on `idea.note`:** Rule V3 allows dismissing an unavailable triage-core paper while rule V10 forbids `coverage` / `verdict` from reading F16 marks (`not_relevant`, `pin`, `key_paper`, `wrong`, `comment`). F12 resolves this by keeping explicit core dismissal on its own event/action (`idea.core_dismissed` / `dismiss_core`) separate from ranking marks.
2. **F4 `#[serde(deny_unknown_fields)]` vs `explore.md` 4.1:** `quote_core::EvidenceQuotePayload` has no `text_basis` field and `EvidenceAbstractPayload` has no `verbatim` field. F12 handles this cleanly without editing F4 by deriving per-page PDF/OCR provenance from `source.ingested.pdf_pages` and validating full F4 invariants on `evidence.abstract`.
3. **Single-pass origin alias enrichment (`prior_work_events.rs`):** Origin aliases are enriched in ledger order from `handoff.packet`, `idea.candidates`, and `literature.fetched`. Two out-of-order multi-hop aliases (e.g. `A->B` discovered before `origin->A`) would require a union-find pass if ever needed in multi-round sessions.
