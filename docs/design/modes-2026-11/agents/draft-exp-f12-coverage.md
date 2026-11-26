# Brief exp-f12-coverage — Coverage, verdict status and wording (package F12)

## 1. Package and governing design sections

- **Package:** `F12` (Coverage, verdict and wording — shared foundation built for Explore; Investigate column kept as a future hook while V14 is deferred).
- **Governing design:**
  - `docs/design/modes-2026-11/00-overview.md`: sections 0.1 (owner scope decisions of 2026-11-25), 2.1 (row F12), 2.9 (shared `PriorWork` status, coverage gate, library exclusion, `independence` moved to S14), 2.10 (display grades), 2.11 (user library never counts as a service), 3.1 row 14 (abstracts as evidence), 3.2 rows 20 & 24 (`"verified"` forbidden outside Solve; `"Model judges:"` / `"(model)"` patterns), 7.3 (forbidden-word property test), 8.1–8.3, and 9 (open item: `incomplete` reason code; `independence` moved to S14).
  - `docs/design/modes-2026-11/explore.md`: sections 4.1 (`Verdict`, `Coverage`), 4.4 (library & corrections interaction), 6 (rules V1–V5, V7, V10, S8), 9 (row F12 / X7 split), and 13 (decisions 4, 5, 6).
  - `docs/design/modes-2026-11/agents/RULES.md` (binds all work; `00-overview.md` wins over mode documents).

## 2. Start conditions and gates

- **Required merges on `gemini/work/fixed` before dispatch:**
  - `F11` (`f11-lit-services` / `q-f11-x1-s3-s6`: `lit_providers.rs`, `lit_abstracts.rs`, `literature.fetched` with optional `qid`/`run_id`).
  - Already merged on base `832eab5bf`: `F0`, `F2` (`paper_key.rs`), `F3` (`net_rate.rs`, `paper_fetch.rs`), `F4` (`quote_core.rs`, `lines_sent.rs`), `F9` (`run_budget.rs`), `F10` (`library.rs`, `library_schema.rs`).
- **Owner / design gate to confirm before dispatch:**
  - `00-overview.md` section 9 marks one item still open: **whether `PriorWorkStatus::Incomplete` carries a typed reason code** (e.g., `IncompleteReason` enum covering `QuickAbstractOnly`, `InsufficientServices`, `ServiceFailedOrRateLimited`, `MissingRequiredPerspective`, `UnscreenedCandidates`, `UnreadCorePapers`, `MissedUserSeed`) alongside the human-readable failing conditions list. Default if not overruled before start: emit a typed `IncompleteReason` enum (snake_case, additive) inside `CoverageGate` / `Verdict` so UI and Explore/Investigate callers never parse prose strings, and note the choice in the report.

## 3. Base-commit verified interfaces vs future-merge interfaces

### Verified on base commit `832eab5bf`
- `crates/research-tools/src/paper_key.rs`: `PaperKey`, `PaperRef`, `PaperIds`, `Mode`, `ID_PREFIXES`.
- `crates/research-tools/src/quote_core.rs` (472 lines): `EVIDENCE_QUOTE_EVENT` (`"evidence.quote"`), `EVIDENCE_ABSTRACT_EVENT` (`"evidence.abstract"`), `QUOTE_MATCHED_WORD` (`"matched"`), `EvidenceRole`, `EvidenceQuotePayload`, `EvidenceAbstractPayload`.
- `crates/research-tools/src/net_rate.rs` (447 lines): `Service::{Arxiv, Inspire, SemanticScholar, Crossref}` and `Service::as_str`.
- `crates/research-tools/src/library_schema.rs` (594 lines): `PaperSummary`, `ORIGIN_USER_LIBRARY`.
- `crates/research-tools/src/lib.rs` (300 lines): module exports.

### Future-merge interfaces to verify before coding
- **`F11` (`lit_providers.rs`, `lit_abstracts.rs`, `literature.fetched`):** verify the exact provider status enum/strings (`ok | partial | failed | rate_limited | not_configured | disabled`), exact `literature.fetched` payload fields (`qid`, `run_id`, `cached_from_ms`, provider name), and `abstract_ref.verbatim` flag. Do not duplicate F11 provider enums if F11 exports a canonical `ProviderName` or status type.

## 4. Smallest implementation checkpoints

1. **Checkpoint 1 — `prior_work.rs` (shared status enum & pure verdict helpers):**
   - Define `PriorWorkStatus`: `JudgedSame`, `JudgedPartial`, `JudgedOtherName`, `AbstractOnly`, `NoneInScope`, `Incomplete` (with serde snake_case names matching `00-overview.md` 2.9).
   - Provide the mapping table / constants documenting the future Investigate column (`V14` deferred) without implementing V14 logic.
   - Define `PriorWorkInput` / `OverlapSupport` types enforcing rule V1 (`judged_*` requires `>= 1` full-text evidence from a read paper; `judged_other_name` requires the other name in quote or excerpt) and rule V2 (`abstract_only` cap; Quick preset never yields `none_in_scope`).
2. **Checkpoint 2 — `search_coverage.rs` (pure ledger coverage & gate):**
   - Implement `Coverage`, `ProviderCoverage`, `ProviderState`, `LibraryCoverage { papers, with_full_text, hits }`, `PerspectiveCoverage`, `PapersReadCounts`, `CitationExpansionCoverage`, `KnownPaperRecall`, `CoverageGate`, `IncompleteReason`, and fixed `NOT_SEARCHED_SCOPE` lines (`explore.md` 4.1).
   - Implement pure `search_coverage::compute(events, preset_limits)` reading ledger events via `serde_json::Value` (Rule V7: deterministic recomputation from ledger events only).
   - Implement the shared gate (Rules V3, V4, V10): `>= 2` literature services answered; zero attempted services `failed` or `rate_limited`; `exact` and `other_names` perspectives each have `>= 1` answered query; `screened >= min(preset_max_screened, candidates_total)`; every triage-core paper read (`read_full`, `read_passages`, `nothing_relevant`) or dismissed by user; every user seed found by a search query. `not_configured` and `disabled` services are listed and never fail the gate (V4). `library` is always listed (`"your library: N papers, M with full text"`) and NEVER counted toward `>= 2` services (V10). Gate reads NO F16 lessons or marks (V10).
3. **Checkpoint 3 — `prior_work_wording.rs` (single wording source & property tests):**
   - Implement every user-facing status chip and verdict/coverage sentence in one module (Rule V5):
     - `judged_same` -> `"Model judges: same"` until `confirmed_by_user == true`, then `"found (same)"` (same pattern for `judged_partial` and `judged_other_name`, Rule V6).
     - `abstract_only` -> `"possibly related (abstracts only)"`.
     - `none_in_scope` -> `"not found in the searched scope"` + app sentence naming searched services, query/abstract/paper counts, library counts, and fixed `not_searched` list.
     - `incomplete` -> `"not found (incomplete search)"` + app sentence listing every failing gate condition (and for Quick: `"no close match among N abstracts; nothing was read"`).
   - Isolate provisional chip strings in named constants with decision comments (`00-overview.md` 2.9, 3.2 rows 20 & 24, `RULES.md`).

## 5. Owned files

- `crates/research-tools/src/prior_work.rs` (new, `<= 600` lines)
- `crates/research-tools/src/search_coverage.rs` (new, `<= 600` lines)
- `crates/research-tools/src/prior_work_wording.rs` (new, `<= 600` lines)
- Sibling unit tests (`*_tests.rs`) and/or `crates/research-tools/tests/prior_work_coverage.rs` (new, `<= 600` lines each)

No existing file over 600 lines is touched.

## 6. Shared registration seams and "Do not edit" list

- **Registration seam:** `crates/research-tools/src/lib.rs` (300 lines) — add `pub mod prior_work;`, `pub mod prior_work_wording;`, `pub mod search_coverage;` in a small, isolated commit.
- **Do not edit:** `solve_badge.rs` (Solve tiers), `run_budget.rs`, `tool_profile.rs`, `tests/golden/tool_defs.txt`, `tests/golden/tool_profiles.txt`, `lit_providers.rs`, `library.rs`, `Cargo.lock`, or any `desktop-server` / `assets` file.

## 7. Tests through the entry point

Run `cargo test -p research-tools --test prior_work_coverage`, `cargo test -p research-tools --lib`, `cargo test -p research-tools --test tool_def_golden`, `cargo test -p research-tools --test tool_profile_golden`, `cargo clippy --workspace --all-targets -- -D warnings`, and `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`.

Required test coverage (all pure, zero network, zero model calls, zero third-party text):
- **`search_coverage_gate_*` (Rule V3):** one dedicated test per gate condition failing in isolation (only 1 service answered; 1 attempted service `failed`; 1 attempted service `rate_limited`; missing `exact` answered query; missing `other_names` answered query; `screened < min(preset, candidates)`; unread triage-core paper not dismissed; user seed missed by search) plus the passing case with arXiv + INSPIRE-HEP alone.
- **`search_coverage_gate_not_configured_is_listed_not_failed` (Rule V4):** Semantic Scholar / OpenAlex `not_configured` or `disabled` appear in `providers[]` and do not fail the gate.
- **`idea_library_does_not_count_as_service` (Rule V10):** 1 live service + large user library (`412` papers, `380` with full text) still fails the `>= 2 services` gate while rendering `"your library: 412 papers, 380 with full text"` in coverage output.
- **`search_coverage_recomputes_from_ledger` (Rule V7):** deterministic recomputation on synthetic ledger streams, invariant to unknown/extra fields and ignoring F16 lessons/marks events.
- **Quick preset & abstract cap (Rule V2):** Quick preset with 0 papers read produces `Incomplete` (`QuickAbstractOnly`) with `"no close match among N abstracts; nothing was read"`, never `NoneInScope`.
- **Forbidden-word property test (Rules V5, S8, `00-overview.md` 7.3):** deterministic generator exhausting every `PriorWorkStatus`, `confirmed_by_user` (`false`/`true`), `IncompleteReason` subset, OCR tag, provider list, and library count; asserts case-insensitive word-boundary absence of `"verified"`, `"new"`, `"novel"`, and `"proved"` across every generated chip and sentence, and asserts all unconfirmed `judged_*` statuses start with `"Model judges:"`.

## 8. Document conflicts and resolutions

1. **`independence` function placement:** `00-overview.md` 2.1 (historical note) and earlier notes placed second-model `independence` in F12. **Resolution:** `00-overview.md` 2.9 and section 9 (scope revision of 2026-11-25) explicitly move `independence` to Solve `S14`, its only current user. Do NOT build `independence` in F12.
2. **Legacy Explore status names:** `explore.md` earlier text used `found_same`, `found_partial`, `found_other_name`, `possibly_related_abstract_only`, `not_found_in_scope`, `search_incomplete`, `no_close_abstract`. **Resolution:** `00-overview.md` 2.9 and reconciled `explore.md` 4.1 adopt the single 6-value `PriorWorkStatus` enum (`judged_same`, `judged_partial`, `judged_other_name`, `abstract_only`, `none_in_scope`, `incomplete`), with `no_close_abstract` folded into `incomplete`.
3. **Investigate `status_badge.rs` / V14 hooks:** `00-overview.md` 2.1 row F12 mentions `status_badge.rs` (V7, V14). **Resolution:** `00-overview.md` 0.1 and 2.9 defer V14; Investigate uses "Search wider in Explore" and reads Explore's child verdict. F12 provides the shared types and wording for Explore now and keeps the Investigate status mapping ready as data only.

## 9. Out of scope and hand-back

- **Out of scope / deferred:** `independence` (Solve `S14`), Investigate V14 prior-work task & "references only" depth gate (deferred), Explore-specific `idea_verdict::compute` and exclude-origin rule V9 (`X11`), UI panels (`X7`, `X12`), F15 write-up (`writeup_idea.rs`).
- **Hand-back:** Follow `RULES.md` — write `docs/design/modes-2026-11/reports/exp-f12-coverage.md` (including exact `IncompleteReason` decision and merge notes for `X7`/`X11`), commit with the `Co-Authored-By` trailer, and push only to `gemini/modes/exp-f12-coverage`.
