# Report: `exp-x16-cite` — Citation Expansion & Thorough Second Round (Package X16)

- **Brief:** `docs/design/modes-2026-11/agents/exp-x16-cite.md`
- **Base commit (branch start):** `ffee92f4bb0372a362b2e2781bb3c8da13b7cdaa` (`origin/gemini/work/fixed` at start, after PR #79 `exp-x9-x10-x11-read-verdict`)
- **Merged `origin/gemini/work/fixed` commit:** `575a72c466fff1b4651c76f174a85a09f32c1c37` (merged `--no-ff` in `fa1ad10`, bringing PR #80 `f8-set-c-harness` and PR #82 `exp-x15-chat`)
- **Push branch:** `gemini/modes/exp-x16-cite`
- **Governing design:** `00-overview.md` (§2.4, §2.8, §3.1 row 12, §3.2 row 20, §8.2 decision 1); `explore.md` (§3, §4.1, §4.2, §6 Rules P3/P5/P6/S2/S8/V9, §9 row X16, §11, §12 risk 12); `agents/RULES.md`.

---

## 1. Commits (Separate Per Checkpoint & Fix — Never Squashed)

1. `a1bf9c7` — **Checkpoint 1a (`research-tools`):** Add pure deterministic citation-network builder `idea_net.rs` and unit tests `idea_net_tests.rs`.
2. `132f316` — **Checkpoint 1b (`desktop-server`):** Add `/idea/cited-since` and `/idea/more` HTTP routes (`routes_idea_net.rs`, `idea_net_exec.rs`), route wiring in `api_dispatch.rs`, and integration tests (`tests/idea_citation_net.rs`).
3. `2b8ccc8` — **Checkpoint 1c (test update):** Remove wired `/idea/more` from the 501 `"not built yet"` list in `crates/desktop-server/tests/idea_flow.rs` (matching X9/X11 precedent). *(Pushed Checkpoint 1.)*
4. `46a4758` — **Checkpoint 2a (Thorough Round 2 & coverage projection):** Add `idea_round2.rs`, minimal Round-2 helpers in `idea.rs`, F12 `citation_expansion` / `citation_net` / `round2` projection in `idea_view_coverage.rs`, `/idea/round2` route dispatch, and Round-2 cap/S8 tests in `tests/idea_citation_net.rs`.
5. `799f1de` — **Checkpoint 2b (UI & Node tests):** Add citation-network and Thorough Round 2 UI in `assets/idea_net_ui.js`, register `/assets/idea_net_ui.js` in `assets.rs`, hook `idea_search.js::renderWhereILooked` / `renderCandidateList`, and add Node DOM/XSS/S8 tests in `tests/idea_net_ui.test.mjs`.
6. `2a227fb` — **Audit Fix A (`research-tools`):** Preserve alias `PaperIds` across `build_from_events` reload, fold latest `idea.triage` relevance per paper (no non-core read promotion when triage exists), stop counting unexpanded `idea.stated` seeds as expanded citation seeds (Audit M2, M3), and add `audit_m3_alias_reload_triage_downgrade_and_rev2_isolation`.
7. `46f0cab` — **Audit Fix B (`desktop-server` + UI + regression tests):** Prevent post-Triage/post-Map citation runs from overwriting Round-1 `search_plan`/`stages` or regressing `next_stage` (Audit C1), enforce all 5 Thorough caps including `calls_per_paper=4` via F4 `context.supplied` (`reader:<sid>:<paper_key>`) and `screened=80` (Audit C2, L3), acquire `ActiveExploreGuard` before ledger reads + serialize writes with `ledger_write_lock` (Audit H3), validate providers / exclude `"unsupported"` from budget / separate 24-query batch slice from F9 budget cap / reject non-allowlisted providers and ambiguous `/idea/more` `{}` (Audit H4, H5, M4), filter `citing_papers` by `since_year` while retaining all edges and avoid inflating seeds/hops on zero-ok runs (Audit M1, M2), guard JS year parsing and running Round-2 button state (Audit M5), fix `clippy::len_zero` (Audit H1), and add `tests/idea_citation_net_audit.rs` + M1/M5 assertions in `tests/idea_net_ui.test.mjs`. *(Pushed Checkpoint 2.)*
8. `fa1ad10` — **Base Sync:** `Merge origin/gemini/work/fixed (575a72c) into work/exp-x16-cite` (`--no-ff`; resolved 1-line module-list conflict in `crates/desktop-server/src/lib.rs` keeping both `pub mod idea_round2;` and `pub mod idea_system_prompt;`).
9. *(Final commit)* — This report (`docs/design/modes-2026-11/reports/exp-x16-cite.md`).

---

## 2. Package X16 Inventory & Line Counts (`<= 600` Lines Every File)

Measured on `fa1ad10` (`python3 scripts/check_file_length.py --base origin/gemini/work/fixed` passed):

| File | Status | Lines | Purpose |
| --- | --- | ---: | --- |
| `crates/research-tools/src/idea_net.rs` | New | 574 | Pure deterministic citation-network builder (`build`, `build_from_events`, `supported_citation_providers`, `default_citation_provider`) over F2 `PaperKey`/`PaperIds` and F11 `cites`/`cited_by` edges; order-invariant in-network ranking (`inbound + outbound`); `"cited by your core papers but unread"` ranking excluding read papers and `handoff.packet.from.paper` origin paper across aliases (Rule V9). |
| `crates/research-tools/src/idea_net_tests.rs` | New | 349 | Unit tests covering order-invariant ranking, alias-aware origin/read exclusion, active-revision folding, provider capability matrix (F11 + X19 OpenAlex), and Audit M3 alias-reload / triage-downgrade / rev-2 isolation. |
| `crates/research-tools/src/lib.rs` | Modified | 383 | Exports `pub mod idea_net;`. |
| `crates/desktop-server/src/routes_idea_net.rs` | New | 496 | Dedicated HTTP handlers for `POST /api/sessions/{id}/idea/{cited-since,more,round2}` mounted behind `AuthGuard::authorize_api`; request parsing & strict provider/action validation; `"Search more about this node"` and explicit citation-expansion plan builders. |
| `crates/desktop-server/src/idea_net_exec.rs` | New | 436 | Shared Rule P3/P5/P6 app-originated batch executor (`run_cited_since_with_router`, `execute_app_search_batch`) routing every query through `Router::try_call("host:idea:<session>", "idea_search", ...)` + F3 `NetRateGate`; zero model calls; `since_year` filtering on `citing_papers`; `ledger_write_lock` around multi-event ledger writes; non-corrupting `"origin": "app"` event emission. |
| `crates/desktop-server/src/idea_round2.rs` | New | 522 | Thorough Round 2 self-check (`find_thin_statements`, `build_round2_plan`, `run_round2_with_router`, `handle_round2_op`, `round2_summary`, `reader_calls_by_paper`): identifies thin/under-supported map nodes and uncovered subclaims, builds targeted retrieval + citation queries, enforces all 5 Thorough hard ceilings (`36` queries, `80` screened, `12` papers, `<= 4` calls/paper via F4 `context.supplied` + `idea.exchange`, `55` model calls -> `StoppedAtCap`, Rule P3), and emits `idea.citation_net`. |
| `crates/desktop-server/src/api_dispatch.rs` | Modified | 530 | Dispatches `POST /api/sessions/{id}/idea/{cited-since,more,round2}` directly to `routes_idea_net::handle_idea_net_op` behind `AuthGuard::authorize_api` without touching `routes_idea.rs`. |
| `crates/desktop-server/src/idea.rs` | Modified | 589 | Adds `EVENT_IDEA_CITATION_NET` constant, `supports_thorough_round2`, excludes `"unsupported"` rows from `queries_used`, and ignores `"origin": "app"` `idea.candidates` updates in `next_stage` so post-Triage citation runs do not regress `next_stage` back to `Triage`. |
| `crates/desktop-server/src/idea_view_coverage.rs` | Modified | 409 | Projects `where_i_looked.citation_expansion` (from F12 `compute_coverage_from_events`), `where_i_looked.citation_net` (seeds, hops, unread core citations, cited-since runs with open-question caveat), and `where_i_looked.round2` (status, ceilings, usage, thin statements). |
| `crates/desktop-server/src/lib.rs` | Modified | 284 | Exports `pub mod idea_round2;` and `pub mod routes_idea_net;`. |
| `crates/desktop-server/src/assets.rs` | Modified | 546 | Registers static asset `/assets/idea_net_ui.js` (remains below the 580-line `assets_fonts.rs` split threshold). |
| `crates/desktop-server/assets/idea_net_ui.js` | New | 306 | Renders `"Who cited this since"` (0 model calls, mandatory `"cited since" != "answered"` note on open questions), `"Cited by your core papers but unread"`, cited-since result lists (filtering `matches_since_year !== false`), and Thorough Round 2 progress/ceilings/thin-statement controls. Zero `innerHTML`, zero `"verified"`/`"novel"`/`"proved"` (Rule S8). |
| `crates/desktop-server/assets/idea_search.js` | Modified | 544 | Minimal hooks calling `renderCitedSinceControl` in `renderCandidateList` (when `onCitedSince` is supplied) and `renderCitationNetPanel` in `renderWhereILooked`. |
| `crates/desktop-server/tests/idea_citation_net.rs` | New | 570 | HTTP + Router integration tests covering `AuthGuard`, interactive `UiApprover` deny/approve, `0` model calls, `host:idea:<session>` audit events, open-question caveat, `since_year` filtering, `/idea/more` node expansion, F3 `NetRateGate` spacing + query cap stop, Round-2 thin-statement detection + Rule S8 + F12 `citation_expansion` update, and Thorough hard caps (`36`/`12`/`55`). |
| `crates/desktop-server/tests/idea_citation_net_audit.rs` | New | 299 | Dedicated regression tests for Audit C1 (stage/`search_plan`/`next_stage` preservation), C2 (`calls_per_paper=4` via F4 `context.supplied`), L3 (`screened=80`), H4 (unsupported provider refusal & budget exclusion), M2 (zero-ok runs do not inflate `seeds`/`hops`), and M4 (HTTP 400 on non-allowlisted provider `theoremsearch` and ambiguous `/idea/more` `{}`). |
| `crates/desktop-server/tests/idea_flow.rs` | Modified | 485 | Removed `"more"` from the 501 `"not built yet"` route list at line 417 (`["directions", "handoff"]`), matching X9/X11 precedent. |
| `crates/desktop-server/tests/idea_net_ui.test.mjs` | New | 282 | Node DOM tests verifying `0 model calls` badge, `"cited since" != "answered"` caveat, unread core citations, Round 2 progress/buttons, XSS escaping (`innerHTML` throws), static zero-`innerHTML` & Rule S8 forbidden-word audit, and Audit M1/M5 regressions. |

**Frozen / Do-Not-Edit files verified untouched (`git diff --name-only origin/gemini/work/fixed` empty):**
`Cargo.lock`, `crates/research-tools/src/tool_profile.rs`, `crates/research-tools/tests/golden/tool_defs.txt`, `crates/research-tools/tests/golden/tool_profiles.txt`, `crates/desktop-server/src/routes_idea.rs`, `crates/desktop-server/assets/idea.js`, `crates/desktop-server/src/turn_runner.rs`, `crates/desktop-server/src/idea_chat.rs`, `crates/desktop-server/src/idea_exchanges*.rs`, `crates/desktop-server/src/cli_turn*.rs`, `crates/export/*`, `crates/agent/src/resume.rs`, `crates/research-tools/src/paper_passages.rs`, `crates/desktop-server/src/http_srv.rs`, `.github/`.

---

## 3. Deviations, Solo Decisions & Assumptions

1. **Deviation (test-only, precedent-matched) — `crates/desktop-server/tests/idea_flow.rs:417` (`2b8ccc8`):**
   - *Reason:* `idea_flow.rs` previously asserted that `["more", "directions", "handoff"]` returned HTTP `501 Not Implemented` while those endpoints were unbuilt. Wiring `POST /api/sessions/{id}/idea/more` in `api_dispatch.rs` (explicitly required by `exp-x16-cite.md` §3.1) caused `"more"` to return a live handler response instead of `501`. Removed `"more"` from that placeholder array while keeping `"directions"` and `"handoff"` asserted as `501`, matching the same pattern used when X9–X11 wired `/idea/read`, `/idea/map`, and `/idea/verdict`.
2. **Solo decision — Split `idea_net_exec.rs` out of `routes_idea_net.rs`:**
   - Separating the HTTP route handlers/plan builders (`routes_idea_net.rs`, `496`L) from the Router batch executor (`idea_net_exec.rs`, `436`L, included via `#[path = "idea_net_exec.rs"]`) kept both files well under the 600-line cap while keeping the public API under `routes_idea_net`.
3. **Solo decision — Left `crates/desktop-server/src/idea_view.rs` (`591`L) untouched:**
   - `idea_view.rs` already computes `coverage` via F12 `compute_coverage_from_events` (which automatically reads `citation_expansion { seeds, hops }` from `idea.citation_net`) and delegates `where_i_looked` construction to `idea_view_coverage.rs::build_where_i_looked`. Projecting `citation_expansion`, `citation_net`, and `round2` inside `idea_view_coverage.rs` (`409`L) avoided touching `idea_view.rs` near the 600-line ceiling.
4. **Solo decision — Separate `/idea/round2` endpoint alongside `/idea/more`:**
   - Exposed `POST /api/sessions/{id}/idea/round2` directly (explicitly permitted by `exp-x16-cite.md` §2: `"plus Round 2 endpoint if separate"`) AND kept `POST /api/sessions/{id}/idea/more` dispatching to Round 2 when `action: "round2"` / `round2: true` / `mode: "round2"` (or on an empty `{}` body only when the session is Thorough and already has a map/verdict). On non-Thorough pre-map sessions, `{}` returns HTTP `400` rather than guessing an intent (Audit M4).
5. **Solo decision — Thorough Round 2 scope (deterministic retrieval & citation self-check, `0` model calls):**
   - `idea_round2.rs` implements the deterministic OpenScholar-style self-check pass (`find_thin_statements`), constructs targeted lexical + citation queries (`cites`/`cited_by`), executes them through `Router::try_call("host:idea:<session>", "idea_search", ...)` + F3 `NetRateGate` under all 5 Thorough hard ceilings (`36` queries, `80` screened, `12` papers, `<= 4` calls/paper, `55` model calls), and persists `idea.citation_net` + `idea.candidates` + `idea.search_finished`. It does **not** automatically invoke LLM triage/read/re-map exchanges (`idea_exchanges*.rs` are Do-Not-Edit in X16); those remain user-triggered or follow-up integration (see §6).
6. **Solo decision — `since_year` filtering semantics:**
   - For `"Who cited this since"` (`cited_by`), `citing_papers` filters out items whose known publication year is strictly less than `since_year` (`matches_since_year == false`), while keeping items with `year: None` (unknown year) and retaining all returned citation edges in the underlying `CitationNetwork` graph.
7. **Named decision constants (per `RULES.md` §Scope):**
   - `EVENT_IDEA_CITATION_NET = "idea.citation_net"` (`idea.rs`, `idea_net.rs`)
   - `CITED_SINCE_NOT_ANSWERED_NOTE` (`routes_idea_net.rs`, `assets/idea_net_ui.js` — `explore.md` §12 risk 12)
   - `THOROUGH_MAX_QUERIES = 36`, `THOROUGH_MAX_SCREENED = 80`, `THOROUGH_MAX_PAPERS = 12`, `THOROUGH_CALLS_PER_PAPER = 4`, `THOROUGH_MAX_MODEL_CALLS = 55` (`idea_round2.rs` — `explore.md` §3)
   - `UNREAD_CORE_HEADING`, `CITED_SINCE_BUTTON_LABEL`, `CITED_SINCE_ZERO_CALLS_HINT`, `SEARCH_MORE_NODE_LABEL`, `ROUND2_HEADING`, `ROUND2_TRIGGER_LABEL` (`assets/idea_net_ui.js`)
8. **Assumptions:**
   - Provider `cites`/`cited_by` support matrix in `idea_net.rs::supported_citation_providers` was verified directly against `crates/research-tools/src/lit_providers_url.rs` and `lit_openalex.rs` (`INSPIRE`: arXiv/DOI/INSPIRE; `OpenAlex`: DOI/OpenAlex; `SemanticScholar`: arXiv/DOI/S2; `arXiv`/`Crossref`: none; `TheoremSearch`: unallowlisted/disabled).
   - CSS styling for `.idea-cited-since-*`, `.idea-open-question-caveat`, `.idea-net-*`, and `.idea-round2-*` classes is deferred to the Explore UI integration lane because `idea.css` / `style.css` are outside X16's owned files.

---

## 4. Independent Carbon Audit Findings & Fixes

An independent read-only audit (`pro` subagent) reviewed the full X16 diff against `origin/gemini/work/fixed` and identified 2 Critical, 5 High, 5 Medium, and 3 Low findings. All actionable code defects were fixed in commits `2a227fb` and `46f0cab` with regression tests:

- **C1 (Stage / `search_plan` / `next_stage` corruption):** Post-Triage/post-Map `/idea/cited-since`, `/idea/more`, and `/idea/round2` runs previously emitted `idea.stage` (`Search`), overwrote `idea.search_planned`, and appended untagged `idea.candidates` causing `next_stage` to regress to `Triage`. Fixed by embedding `"origin": "app"` + `"plan_queries"` on `idea.search_finished` / `idea.citation_net`, stopping emission of `idea.stage` (`Search`) / `idea.search_planned` from app citation batches, and ignoring `"origin": "app"` `idea.candidates` in `idea.rs::next_stage`; regression in `audit_c1_post_map_cited_since_and_round2_do_not_corrupt_round1_stage_or_next_stage`.
- **C2 (Incomplete `calls_per_paper` enforcement):** X10 (`idea_read_passages.rs`) records per-paper reader calls in F4 `context.supplied` (`caller = "reader:<sid>:<paper_key>"`), not on `idea.exchange`. Added `reader_calls_by_paper` counting both F4 `context.supplied` and `idea.exchange.paper_key`, and enforced all 5 Thorough caps (`model_calls=55`, `papers=12`, `screened=80`, `calls_per_paper=4`, `queries=36`) with `round2_stopped_at_cap` persistence; regression in `audit_c2_and_l3_round2_enforces_calls_per_paper_via_f4_context_supplied_and_screened_80`.
- **H1 (`clippy::len_zero`):** Fixed `.len() >= 1` -> `!.is_empty()` in `tests/idea_citation_net.rs:518`.
- **H2 (Duplicate `"more"` arm in `routes_idea.rs:336`):** Left `routes_idea.rs` untouched (Do-Not-Edit in X16) and documented the shadowed 501 arm under Merge Notes (§5).
- **H3 (Concurrent guard & ledger write serialization):** Moved `ActiveExploreGuard::acquire` before ledger reads/plan building in `routes_idea_net.rs` and `idea_round2.rs`, passed `Some(&state.ledger_write_lock)` held strictly around multi-event ledger writes (never across `Router::try_call`), and re-verified active `stmt_seq` before writing.
- **H4 (Provider validation & `"unsupported"` budget accounting):** Validated `supported_citation_providers` and `is_allowlisted()` inside `run_cited_since_with_router`, excluded `"unsupported"` rows from `idea.rs::budget_snapshot::queries_used` and `idea_net_exec.rs` failed counts; regression in `audit_h4_m2_m4_*`.
- **H5 (`MAX_IDEA_SEARCH_QUERIES` vs F9 budget cap):** Separated 24-query batch slicing from F9 `max_queries` budget skips so batch truncation is not misreported as `skipped_budget` / `stopped_at_cap`.
- **M1 (`since_year` filtering):** Filtered `citing_papers` by `matches_since_year != false` in both Rust (`idea_net_exec.rs`) and JS (`idea_net_ui.js`), updated missing `year` when a second provider supplies one, and kept all edges in `CitationNetwork`; regressions in `idea_citation_net.rs` and `idea_net_ui.test.mjs`.
- **M2 (Zero-ok seed/hop inflation & hop honesty):** Avoided passing `extra_seeds`/`hops` when `ok_count == 0`, stopped treating unexpanded `idea.stated` seeds as expanded citation seeds in `idea_net.rs::build_from_events`, skipped appending `idea.citation_net` on 100%-failed `cited_since` batches, and kept single-batch `hops <= 1`; regression in `audit_h4_m2_m4_*`.
- **M3 (`idea_net.rs` alias reload & triage downgrade):** Persisted and merged `PaperIds` on `CitationNetNode` so cross-provider aliases survive `build_from_events` reload, and folded latest `idea.triage` relevance per paper instead of unioning historical `"core"` labels; regression in `audit_m3_alias_reload_triage_downgrade_and_rev2_isolation`.
- **M4 (Strict HTTP validation):** Rejected explicit non-allowlisted providers (`theoremsearch`) in `parse_cited_since_request` and ambiguous `{}` on non-Thorough pre-map `/idea/more` with HTTP `400`; regression in `audit_h4_m2_m4_*`.
- **M5 (`idea_net_ui.js` year parsing & running button):** Added strict `parseValidYear` (`Number.isInteger`) so `""`/non-numeric `since_year` never renders `NaN` or `"since "`, and disabled `"Run Thorough Round 2"` when `status === 'running'`; regression in `idea_net_ui.test.mjs`.
- **L1 (`idea.rs` line headroom):** Moved `max_reader_calls_per_paper` into `idea_round2.rs`, keeping `idea.rs` at `589`L (`<= 600`L).
- **L2 (`handle_continue` `extra_screened`):** Noted under Merge Notes (§5) because `routes_idea.rs` is Do-Not-Edit in X16.
- **L3 (`screened=80` and `rev=2` isolation tests):** Added both in `idea_citation_net_audit.rs` and `idea_net_tests.rs`.

---

## 5. Merge Notes

Exact integration seams for concurrent/downstream lanes:

- **`crates/desktop-server/assets/idea.js` (`593`L, Do-Not-Edit in X16 — owned by `X12/X13`):**
  - Pass `{ onCitedSince, onRunRound2, onSearchMoreNode }` callbacks into `renderCandidateList` and `renderWhereILooked` (from `idea_search.js`) and wire them to:
    - `POST /api/sessions/{id}/idea/cited-since` with `{ paper_key, node_id, since_year, is_open_question }`
    - `POST /api/sessions/{id}/idea/more` with `{ node_id }` (for node expansion) or `{ action: "expand_citations", seeds }`
    - `POST /api/sessions/{id}/idea/round2` with `{}` (for Thorough Round 2)
  - On map open-question cards (`kind === 'open_question'`), call `renderCitedSinceControl(node, { onCitedSince })` from `/assets/idea_net_ui.js` so the mandatory `"cited since" != "answered"` note renders directly on map nodes too.
- **`crates/desktop-server/assets/idea_state.js` (owned by `X12/X13`):**
  - Add an SSE reducer arm for `idea.citation_net` (or re-fetch `GET /api/sessions/{id}/idea` on `idea.citation_net`) so live citation expansion and Round 2 updates refresh `where_i_looked.citation_net` and `where_i_looked.round2`.
- **`crates/desktop-server/assets/idea.css` / `style.css`:**
  - Style the semantic classes emitted by `idea_net_ui.js`: `idea-cited-since-box`, `idea-open-question-caveat`, `idea-net-panel`, `idea-net-section`, `idea-net-summary`, `idea-net-list`, `idea-net-item`, `idea-round2-panel`, `idea-round2-thin-list`, `idea-round2-thin-item`.
- **`crates/desktop-server/src/routes_idea.rs:336-338` (Do-Not-Edit in X16 — owned by `X12/X13`):**
  - The `"more"` arm returning `501 Not Implemented` is now shadowed by `api_dispatch.rs`'s earlier dispatch to `routes_idea_net::handle_idea_net_op`; remove that dead arm when `routes_idea.rs` is next edited.
  - Note also `routes_idea.rs::handle_continue` (line 487) does not currently bump `extra_screened` when extending budgets; wire that if Round-2 continuation needs additional screening headroom.
- **Localized edits in existing files (conflict awareness):**
  - `crates/research-tools/src/lib.rs` (1 `pub mod idea_net;` line)
  - `crates/desktop-server/src/lib.rs` (2 `pub mod` lines)
  - `crates/desktop-server/src/api_dispatch.rs` (3-route match arm before `routes_idea`)
  - `crates/desktop-server/src/assets.rs` (`546`L — 1 `/assets/idea_net_ui.js` arm)
  - `crates/desktop-server/src/idea.rs` (`589`L — `EVENT_IDEA_CITATION_NET`, `supports_thorough_round2`, `"unsupported"` budget skip, `"origin": "app"` `next_stage` guard; do not add > 10 lines without extracting)
  - `crates/desktop-server/src/idea_view_coverage.rs` (`409`L — appends `citation_expansion`, `citation_net`, `round2` to `where_i_looked`)
  - `crates/desktop-server/assets/idea_search.js` (`544`L — imports & calls `renderCitedSinceControl` and `renderCitationNetPanel`)
  - `crates/desktop-server/tests/idea_flow.rs:417` (`["directions", "handoff"]`)

---

## 6. Verification Commands & Exact Measured Results

All commands ran on Linux as non-root (`uid=1001(antigravity)`, `gid=1001(antigravity)`), `rustc 1.100.0 (0ef1bc386 2026-11-09)`, Node `v22.23.5`, Python `3.12.16`, on merged commit `fa1ad10`:

1. **Non-root check & file-length gate:**
   - `id` -> `uid=1001(antigravity) gid=1001(antigravity)`
   - `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` -> **PASS** (`No touched source file is over 600 lines.`)
   - `git diff --name-only origin/gemini/work/fixed -- Cargo.lock crates/research-tools/tests/golden/` -> empty (untouched)
2. **JavaScript syntax & Node DOM test suite:**
   - `for f in crates/desktop-server/assets/*.js; do node --check "$f" || exit 1; done` -> **PASS** (exit 0)
   - `node --test crates/desktop-server/tests/*.test.mjs` -> **270 passed, 0 failed, 0 skipped** (including `idea_net_ui.test.mjs`: 4/4 passed)
3. **Workspace Clippy:**
   - `unset RE_SEARCH_PYTHON && cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` -> **PASS** (0 warnings, 0 errors)
4. **Workspace Rust tests:**
   - `unset RE_SEARCH_PYTHON && cargo test --workspace --locked --no-fail-fast -j 2` -> **1465 passed, 0 failed, 2 ignored**
   - Includes:
     - `research-tools` lib `idea_net::tests`: **5 passed, 0 failed**
     - `research-tools` `tool_def_golden` & `tool_profile_golden`: **6 passed, 0 failed**
     - `desktop-server` `idea_citation_net`: **4 passed, 0 failed**
     - `desktop-server` `idea_citation_net_audit`: **3 passed, 0 failed**
     - `desktop-server` `idea_flow`: **4 passed, 0 failed**
     - `desktop-server` `idea_eval`: **5 passed, 0 failed**
   - *Note on `-j 2`:* Running `cargo test --workspace` with default full concurrency was previously killed by the container memory limit during parallel linking; `-j 2` ran the entire workspace cleanly to completion with zero failures.
   - *Pre-existing ignored tests (2):* Live-API eval tests in `crates/agent/tests/eval_harness.rs` (untouched by X16). Zero skipped or `#[ignore]` tests were added.

---

## 7. What Could Not Run on Linux & Known Limitations

- **Not compiled or run on Linux:**
  - Windows-only `cfg(windows)` code and tests (`worker/tests/appcontainer_sandbox.rs`, `worker/tests/job_memory_limit.rs`, `worker/tests/python_worker.rs`, `desktop-server/tests/claude_cli_*.rs` all report `0 tests` on Linux).
  - Real Chromium browser E2E checks (no browser in headless Linux container; UI was verified via Node `MiniEl` DOM tests).
  - Live external literature endpoints (INSPIRE-HEP, OpenAlex, Semantic Scholar); all tests use loopback HTTP mocks per `RULES.md`.
- **Plain statement of what is not wired / does not happen yet:**
  - `idea_round2.rs` performs deterministic thin-statement detection + Router-gated retrieval/citation expansion (`0` model calls) under all 5 Thorough hard caps, but does **not** automatically launch follow-up LLM triage, passage-reading, or map-rewriting exchanges (those remain separate user-driven or future integration steps).
  - Live UI buttons in `assets/idea.js` do not yet call `/idea/{cited-since,more,round2}` and `assets/idea_state.js` does not yet reduce `idea.citation_net` SSE events, because both files are Do-Not-Edit in X16 (documented in §5 Merge Notes).
  - `crates/desktop-server/src/routes_idea.rs:336-338` still contains the now-shadowed 501 `"more"` arm because `routes_idea.rs` is Do-Not-Edit in X16.
