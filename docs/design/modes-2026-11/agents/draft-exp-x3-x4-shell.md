# Brief exp-x3-x4-shell — Explore session, stage machine, view, routes (X3) and model-free first screen (X4)

## 1. Packages and governing design sections

- **Packages (in order):** `X3` (Session, stage machine, view, routes) -> `X4` (First screen, model-free shippable slice).
- **Governing design:**
  - `docs/design/modes-2026-11/00-overview.md`: sections 0.1 (scope decisions of 2026-11-25), 2.1–2.7, 2.10–2.11, 3.1 rows 3/15/16, 3.2 rows 17–25, 3.3 (run-state vocabulary), 5.1–5.3 (entry screen, names, existing sessions), and 8.1–8.3.
  - `docs/design/modes-2026-11/explore.md`: sections 3 (Before submit, First screen, First results, Presets, no-model banner, empty/failure states), 4.1 (`IdeaStatement`, `SearchPlan`, `Candidate`, `Budget`), 4.2 (ledger events), 4.4 (library preview & owned papers), 5.3–5.4 (app functions & HTTP routes), 6 (rules P1, P3, P4, P6, S3, S7, S8), 7–8 (migration, latest-marker mode detection), 9 (rows X3, X4), and 13 (decisions 1, 2, 3, 12, 16).
  - `docs/design/modes-2026-11/agents/RULES.md` (and `00-overview.md` wins on any discrepancy).

## 2. Start conditions and gates

- **Required merges on `gemini/work/fixed` before starting X3:**
  - `F5` (`f5-f7-mode-shell`: `session_mode.rs`, `api_dispatch.rs` split from `http_srv.rs`, `routes_session.rs` shrunk under 560 lines, `index.html` shrunk under 560 lines, `assets/new_menu.js`).
  - `F11` (`f11-lit-services`: `lit_providers.rs`, `lit_abstracts.rs`, `literature.fetched` with optional `qid`/`run_id`).
  - `X1` (`x1-idea-terms`: `idea_terms.rs`, `idea_queries.rs`, `idea_rank.rs`).
  - Already merged on base `832eab5bf`: `F0`, `F1`, `F2`, `F3`, `F6`, `F9`, `F10`, `F14a`.
- **Optional enrichment (verify if merged):** `F13a` (`lib-f13a-f16a-f15`) populates `user_library` rows in F10's `fulltext` table (`owned: true`). X3/X4 must work identically whether `user_library` rows are present or empty.
- **No owner gate blocks X3/X4** (owner accepted all section 8 recommendations on 2026-11-25, including `explore.md` decision 1 for user-shown first queries). Note: keep the Explore home card behind a visibility/settings flag until Explore is usable (`explore.md` 3, `00-overview.md` 5.1) — never show it greyed out.

## 3. Base-commit verified interfaces vs future-merge interfaces

### Verified on base commit `832eab5bf`
- `crates/research-tools/src/tool_profile.rs` (329 lines): `MARKER_IDEA_STARTED` (`"idea.started"`), `session_mode_from_events`, `PROFILE_EXPLORE_V1` (`"explore/1"` — stays `NotBuiltYet` until X7; X3/X4 make zero model calls and do not activate `explore/1`).
- `crates/research-tools/src/paper_key.rs`: `PaperKey`, `PaperRef`, `ID_PREFIXES` (`sc-`, `p-`, `c-`, `d-`, `k-`, `q-`, `w-`, `r-`, `xe-`, `xa-`).
- `crates/research-tools/src/run_budget.rs` (464 lines): `count_model_calls`, `ModelCallBudget`, `ExploreStageState`, `explore_stage_run_state`, `RUN_STATE_LABEL_*`.
- `crates/research-tools/src/paper_fetch.rs` (492 lines): `fetched_sources`, `check_fetch_budget`.
- `crates/research-tools/src/library.rs` (532 lines): `Library::{open, search, get_paper, record_seen}`, `SeenRole::Candidate`, `ORIGIN_USER_LIBRARY`.
- `crates/desktop-server/assets/run_bar.js` (283 lines), `ui_badge.js` (387 lines), `quote_chip.js` (178 lines), `api.js` (465 lines), `crates/desktop-server/src/assets.rs` (365 lines).
- **Files verified OVER 600 lines on base (DO NOT EDIT without split):** `http_srv.rs` (702 — F5 moves dispatch to `api_dispatch.rs`), `routes_session.rs` (616 — F5 shrinks <560), `session_view.rs` (753!), `index.html` (768 — F5 shrinks <560), `app.js` (932!).

### Future-merge interfaces to verify before coding
- **`F5`:** exact `session_mode.rs` helper, `api_dispatch.rs` match signature, shrunk `routes_session.rs` create/get-session hooks, and `assets/new_menu.js` card mount contract (including paste detection that focuses Investigate on bare arXiv ID / DOI / arXiv URL).
- **`X1`:** exact function signatures in `idea_terms.rs` (`extract`), `idea_queries.rs` (`first_queries`), and `idea_rank.rs` (`merge_and_rank`).
- **`F11`:** exact query runner signature in `lit_providers.rs` and raw-blob storage into `literature.fetched`.

## 4. Smallest implementation checkpoints

Push after X3, then merge `origin/gemini/work/fixed` before starting X4:

1. **Checkpoint 1 — X3 Rust session, stage machine, view, and routes:**
   - `idea.rs`: host-only `idea.started` writer (`preset`, `budget`, `pipeline_version: 1`; never emitted by any tool — Rule S7), `idea.stated` writer (`IdeaStatement`, `text <= 4000`, `text_sha256`, `fields`, `seeds`, `author: "user"`), deterministic stage transition table `next_stage(events)` (Rule P1), interrupted-stage detection (`idea.stage` left at `requested` on restart -> `ExploreStageState::Interrupted`, never auto-re-run — Rule S3), preset budget definitions (`Quick`, `Standard`, `Thorough` ceilings from `explore.md` 3) layered on F9 `run_budget::count_model_calls` and F3 `fetched_sources` (Rule P3).
   - First-search execution under **Rule P4**: at session creation only, verify submitted `queries` match the exact strings produced by `idea_queries::first_queries` (at most 6, providers `arxiv` and `inspire` only); reject any tampered/extra query or provider with HTTP 400; respect `offline: true` (zero network requests, library matches only). Run permitted queries through F11 (gated by F3 `net_rate`, Rule P6), merge/rank via X1 `idea_rank::merge_and_rank` (cap 200), annotate `in_library` and `owned` from F10 `Library`, write `idea.search_planned`, `literature.fetched`, `idea.search_finished`, `idea.candidates`, and record F10 `seen` (`Candidate`).
   - `idea_view.rs`: pure `view(events, running)` reading events via `serde_json::Value` (tolerating missing/unknown fields for migration safety), mapping stage states through F9 `explore_stage_run_state`, returning cost-line payload for `run_bar.js`, candidate list, `"Where I looked"` skeleton, and `GraphView`-shaped map placeholder (`{ nodes: [], edges: [], legend: [] }`).
   - `routes_idea.rs`: single dispatcher implementing `POST /api/idea/preview` (no network, no session, calls X1 + F10 library search), `create_explore_session` (for `POST /api/sessions` with `mode: "explore"`), and `POST /api/sessions/{id}/idea/{op}` (`revise | answer | plan | search | triage | read | map | directions | continue | stop | note | handoff | more`, returning explicit 501 / `"not built yet"` for ops owned by X6–X16). Stream progress over existing SSE (`sse.rs`). All routes sit behind `AuthGuard::authorize_api`.
   - Commit and push X3 to `gemini/modes/exp-x3-x4-shell`.
2. **Checkpoint 2 — X4 Model-free first screen UI:**
   - `idea_state.js`: pure client state reducer & selectors (stage rail, run-bar model, candidate filters, empty/failure states).
   - `idea_setup.js`: Explore card content mounted in F5's home screen (`new_menu.js`): idea textarea (`<= 4000` chars, TeX allowed), optional `"papers you already know"` (arXiv IDs / DOIs), preset dial with price ceilings, `"offline: library only"` switch, debounced local preview (`POST /api/idea/preview`: term chips, detected IDs, library matches with `"in your library"` badge, editable first queries), `"Explore"` button (Ctrl+Enter).
   - `idea_search.js`: candidate list (title, authors, year, abstract with term highlights, `"not read"` badge, `"in your library"` badge) and `"Where I looked"` skeleton.
   - `idea.js` & `idea.css`: Explore session page shell — verbatim Idea card, stage rail (`Idea - Sharpen - Search - Triage - Read - Map - Directions`), shared F6 `run_bar.js` cost line (`"Standard - 9 of 18 queries - 3 of 6 papers - 7 of at most 24 model calls - Stop"`), no-model notice (`"Search and library work without a model. Sharpening, reading and mapping need one."`), and exact empty/failure states (`explore.md` 3).
   - `api.js` (465 lines on base — add small Explore helpers if `< 540` lines after prior merges; otherwise place helpers in a new `assets/api_idea.js` so `api.js` stays safely `<= 600` lines).
   - Commit and push X4 to `gemini/modes/exp-x3-x4-shell`.

## 5. Owned files

- **X3:**
  - `crates/desktop-server/src/idea.rs` (new, `<= 600` lines)
  - `crates/desktop-server/src/idea_view.rs` (new, `<= 600` lines)
  - `crates/desktop-server/src/routes_idea.rs` (new, `<= 600` lines)
  - `crates/desktop-server/tests/idea_flow.rs` (new, `<= 600` lines)
- **X4:**
  - `crates/desktop-server/assets/idea.js` (new, `<= 600` lines)
  - `crates/desktop-server/assets/idea_state.js` (new, `<= 600` lines)
  - `crates/desktop-server/assets/idea_setup.js` (new, `<= 600` lines)
  - `crates/desktop-server/assets/idea_search.js` (new, `<= 600` lines)
  - `crates/desktop-server/assets/idea.css` (new, `<= 600` lines)
  - `crates/desktop-server/assets/api.js` (or new `api_idea.js` if `api.js` approaches 550 lines)
  - `crates/desktop-server/tests/idea_state.test.mjs` (new, `<= 600` lines)

## 6. Shared registration seams and "Do not edit" list

- **Registration seams (keep each edit 1–3 lines, in its own commit):**
  - `crates/desktop-server/src/lib.rs` (215 lines): `pub mod idea;`, `pub mod idea_view;`, `pub mod routes_idea;`.
  - `crates/desktop-server/src/api_dispatch.rs` (created by F5 from `http_srv.rs`): 2 delegating match arms for `/api/idea/preview` and `/api/sessions/{id}/idea/{op}` behind `AuthGuard::authorize_api`.
  - `crates/desktop-server/src/routes_session.rs` (shrunk `< 560` by F5): delegate `mode: "explore"` on `POST /api/sessions` to `routes_idea::create_explore_session`, and attach `"idea": idea_view::view(&events, running)` on `GET /api/sessions/{id}` when `session_mode == Explore`. (Do NOT edit `session_view.rs` — 753 lines!)
  - `crates/desktop-server/src/assets.rs` (365 lines): register `/assets/idea.js`, `idea_state.js`, `idea_setup.js`, `idea_search.js`, `idea.css` (and `api_idea.js` if created).
  - `crates/desktop-server/assets/new_menu.js` (created by F5): mount Explore card (`idea_setup.js`) behind visibility flag.
- **Do not edit:** `session_view.rs` (753), `http_srv.rs` (702, unless F5 already shrunk it below 560), `app.js` (932), `index.html` (768, owned by F5), `routes_tools.rs` (610), `resume.rs`, `tool_profile.rs`, `tests/golden/tool_defs.txt`, `tests/golden/tool_profiles.txt`, `Cargo.lock`.

## 7. Tests through the entry point

Run `cargo test -p desktop-server --test idea_flow`, `cargo test --workspace --no-fail-fast`, `cargo clippy --workspace --all-targets -- -D warnings`, `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done`, `node --test crates/desktop-server/tests/*.test.mjs`, and `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`.

Required tests (loopback mock only, zero live network, zero model calls, zero third-party paper text):
- `idea_preview_is_local_and_deterministic`: `POST /api/idea/preview` makes 0 HTTP requests, creates 0 sessions, returns terms, IDs, library matches (`owned` flag), and exact first queries.
- `idea_user_queries_are_exactly_shown_ones` (Rule P4): create accepts exact preview queries (`<= 6`, `arxiv`/`inspire` only) without approval; rejects mutated query string, 7th query, or `crossref`/`semantic_scholar` provider with 400; `offline: true` sends 0 network requests.
- `idea_recorder_refused_outside_stage` (Rule P1) and `idea_stage_resumes_after_crash` (Rule S3): interrupted `idea.stage` (`requested` without `finished`) renders `"Interrupted"` with Retry and does NOT auto-re-run on reopen.
- `idea_requests_all_pass_the_gate` (Rule P6) & auth check: unauthenticated `/api/idea/preview` and `/api/sessions/{id}/idea/*` are rejected by `AuthGuard::authorize_api`.
- `idea_state.test.mjs`: tests stage rail, F9 run-state labels, no-model banner, all empty/failure states, XSS escaping (zero raw `innerHTML` injection from paper titles/abstracts), and Rule S8 (zero `"verified"`, `"new"`, or `"novel"` strings in rendered Explore DOM).

## 8. Document conflicts and resolutions

1. **Mode detection rule:** Earlier `explore.md` stated "first event is `idea.started`" and a three-valued mode enum. **Resolution:** `00-overview.md` 1 & 3.1 row 3 (reconciled in `explore.md` 8) mandate 4-valued `session_mode_from_events` where the latest host-written marker wins (`idea.started` may follow copied `source.ingested` and `handoff.packet` in hand-off children).
2. **`session_view.rs` line limit:** `explore.md` 5.4 says `GET /api/sessions/{id}` gains `"idea": idea_view::view(...)`, which naively points at `session_view.rs` (753 lines, over 600). **Resolution:** attach `"idea"` in `routes_session.rs` (shrunk by F5) or split `session_view.rs` in a separate zero-behaviour-change commit before touching it (`RULES.md` 600-line rule).
3. **Run-state words & "exchange" naming:** Earlier `explore.md` used `"leg"` and mode-local stage words (`stopped_at_cap`, `cancelled`). **Resolution:** `00-overview.md` 3.2 row 17 & 3.3 (F9) standardize on `"exchange"` and the 7 shared UI labels (`Running`, `Waiting for you`, `Paused: (reason)` + Continue, `Interrupted` + Retry, `Failed` + Retry, `Done`).

## 9. Out of scope and hand-back

- **Out of scope:** Model exchanges (`X6`–`X11`), `explore/1` profile activation and `idea_search`/`idea_fetch_papers` tools (`X7`, `X9`), graph map UI (`X12`), directions/hand-offs (`X13`), Conversation turns (`X15`), citation expansion (`X16`), full Library pane (`X17`), CLI routes (`X18`).
- **Hand-back:** Write `docs/design/modes-2026-11/reports/exp-x3-x4-shell.md` (with exact merge notes for `X6` and `X7`), commit with `Co-Authored-By`, and push checkpoints to `gemini/modes/exp-x3-x4-shell`.
