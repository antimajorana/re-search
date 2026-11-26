# Report: `exp-x3-x4-shell` (packages X3 — Explore session, stage machine, view, routes; X4 — model-free first screen)

- **Base commit (start of X3):** `28ac1111f99e49eab151799c629c091b7996b8c2` (`origin/gemini/work/fixed`)
- **Mid-brief merge (between X3 and X4, per brief section 4):** `d174ab6a3eb62244cc4e3adef831c0cd3df4bf44` (`origin/gemini/work/fixed`, merged at `e8727ab` after resolving trivial 2-line conflicts in `crates/desktop-server/src/lib.rs` and `crates/desktop-server/src/api_dispatch.rs`)
- **Work branch:** `work/exp-x3-x4-shell` (pushed to `gemini/modes/exp-x3-x4-shell`)
- **Toolchain on this machine:** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `cargo 1.100.0`, `Node v22.23.5`, `Python 3.12.16` on Linux x86_64
- **Unchanged guard rails:**
  - `Cargo.lock` untouched (0 new dependencies).
  - `crates/research-tools/tests/golden/tool_defs.txt` and `crates/research-tools/tests/golden/tool_profiles.txt` untouched; both golden tests pass.
  - Zero tools added or modified; `explore/1` remains `NotBuiltYet` (zero model calls in X3 and X4).
  - `crates/policy/Cargo.toml` and `crates/ledger/Cargo.toml` remain free of `tokio`, `rmcp`, and `windows-sys`.
  - Do-Not-Edit files untouched: `session_view.rs`, `http_srv.rs`, `app.js`, `index.html`, `routes_tools.rs`, `resume.rs`, `tool_profile.rs`, `Cargo.lock`, `tool_defs.txt`, `tool_profiles.txt`.
  - Zero third-party paper text anywhere in code or tests (all fixtures are synthetic).
  - `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` exits 0 (`No touched source file is over 600 lines.`).

---

## 1. What was implemented

### Package X3 — Explore session, stage machine, view, and routes (`crates/desktop-server`)

#### 1.1 Core session model, stage machine, budgets, view, and routes (`1dd6c89`)
- `crates/desktop-server/src/idea.rs` (new, **580 lines**):
  - Named constants referencing `00-overview.md` 8.2 / `explore.md` 4.2 & 13: `PIPELINE_VERSION = 1`, `MAX_IDEA_CHARS = 4000`, `MAX_USER_PREVIEW_QUERIES = 6`, `MAX_CANDIDATES = 200`, `EVENT_IDEA_STARTED`, `EVENT_IDEA_STATED`, `EVENT_IDEA_STAGE`, `EVENT_IDEA_SEARCH_PLANNED`, `EVENT_IDEA_SEARCH_FINISHED`, `EVENT_IDEA_CANDIDATES`, `EVENT_IDEA_BUDGET`, `EVENT_IDEA_NOTE`, `EVENT_IDEA_BRIEF`, `EVENT_IDEA_MAP`.
  - Host-only `write_idea_started` and `write_idea_stated` (SHA-256 hash, Unicode scalar length validation `<= 4000`, normalized `fields`, canonicalized `seeds`, `author: "user"`). Never emitted by any tool (Rule S7; verified by `mode_markers_r2`).
  - Preset budget table (`Quick`, `Standard`, `Thorough` from `explore.md` 3) layered directly on F9 `run_budget::count_model_calls` + `ModelCallBudget` and F3 `paper_fetch::fetched_sources` / `check_fetch_budget` (Rule P3), with additive budget bumps via `idea.budget` (`extra_queries`, `extra_papers`, `extra_model_calls`).
  - Deterministic revision-scoped stage transition table `resolve_stages` / `next_stage` / `validate_recorder_stage` (Rule P1): resets post-`Idea` stages on a new `idea.stated` revision, enforces latest-stage prerequisite ordering (`Idea -> Sharpen -> Search -> Triage -> Read -> Map -> Directions`), and flags interrupted stages (`idea.stage` left at `requested` when not actively running -> `ExploreStageState::Interrupted`, never auto-re-run on restart — Rule S3).
  - `ActiveExploreGuard` and `CancelToken` tracking so an in-flight first search reports `Running` (not `Interrupted`) and `POST /api/sessions/{id}/idea/stop` cancels cooperatively.
- `crates/desktop-server/src/idea_search.rs` (new, **511 lines**, private submodule of `idea.rs`):
  - Local-only `preview_idea` (zero HTTP requests, zero session creation): calls X1 `idea_terms::extract`, `idea_queries::first_queries`, parses bare-ID focus detection (`focus_investigate`), canonicalizes seeds (reporting `invalid_seeds`), and runs F10 `Library` search (per top term + exact key lookups, deduplicated by canonical `PaperKey`, annotating `owned` via `ORIGIN_USER_LIBRARY` / `source_items`).
  - Rule P4 first-query validation (`validate_user_first_queries`): verifies online submitted queries are non-empty, `<= 6`, `arxiv`/`inspire` only, and exact matches against `idea_queries::first_queries` (rejecting tampered strings, extras, or disallowed providers like `crossref`/`semantic_scholar` with HTTP 400).
  - `run_first_search`: runs permitted queries through F11 `lit_providers` (all HTTP requests gated by F3 `NetRateGate` — Rule P6), merges and ranks via X1 `idea_rank::merge_and_rank` (capped at 200), merges offline/online library matches, annotates `in_library` and `owned` from F10 `Library`, writes `idea.search_planned`, `literature.fetched`, `idea.search_finished`, `idea.candidates`, records F10 `seen` (`SeenRole::Candidate`), and emits SSE progress events.
- `crates/desktop-server/src/idea_view.rs` (new, **593 lines**):
  - Pure `view(events, running)` reading ledger payloads via `serde_json::Value` (tolerating missing/unknown fields for migration safety).
  - Maps stage states through F9 `explore_stage_run_state` (`Running`, `Waiting for you`, `Paused`, `Interrupted`, `Failed`, `Done`), builds the F6 `run_bar.js`-compatible cost-line payload (both `snake_case` and `camelCase` `modelCallsUsed`/`modelCallsMax`), formats candidate cards with `"not read"` / `"in your library"` / `"owned"` badges, builds `"Where I looked"` (including honest `"not_attempted"` for Crossref and `"not_configured"` for Semantic Scholar), computes exact empty/failure states (`offline_library_only`, `all_services_failed`, `zero_candidates`, `stage_failed`), translates F6 map edge kinds, and returns the `GraphView`-shaped placeholder `{ nodes: [], edges: [], legend: [] }`.
  - Rule S8 compliance: zero `"verified"`, `"new"`, `"novel"`, or `"proved"` strings in any view label.
- `crates/desktop-server/src/routes_idea.rs` (new, **576 lines**):
  - `handle_idea_preview` (`POST /api/idea/preview`): local preview behind `AuthGuard::authorize_api`.
  - `create_explore_session` (called from `POST /api/sessions` with `mode: "explore"` and `text`): validates input, checks live and deleted-session ID conflicts, writes `meta.json` (rolling back on start failure), executes Rule P4 first search (or offline library-only search), emits SSE progress, returns `201 Created` with full `idea` view.
  - `handle_idea_op` (`POST /api/sessions/{id}/idea/{op}`): validates session existence and Explore mode; implements `revise` (acquires `ledger_write_lock`, appends next `idea.stated` rev), `stop` (sets cancel token, records interrupted stage under lock), `note` (`idea.note`), and `continue` (validates strict integer budget bumps when paused; returns explicit `501 Not Implemented` for unbuilt model-stage continuations); returns explicit `501` `"not built yet"` for ops owned by X6–X16 (`answer`, `plan`, `search`, `triage`, `read`, `map`, `directions`, `handoff`, `more`).

#### 1.2 Registration seams (`b132b64`)
- `crates/desktop-server/src/lib.rs` (**227 lines**): `pub mod idea; pub mod idea_view; pub mod routes_idea;`.
- `crates/desktop-server/src/api_dispatch.rs` (**395 lines**): 2 delegating match arms for `POST /api/idea/preview` and `POST /api/sessions/{id}/idea/{op}` behind `AuthGuard::authorize_api`.
- `crates/desktop-server/src/routes_session.rs` (**532 lines**): delegates `POST /api/sessions` with `mode: "explore"` + `text` to `routes_idea::create_explore_session` (preserving the F5 400 `"not built yet"` contract for bare Explore sessions without `text`), and attaches `"idea": idea_view::view(&events, running)` on `GET /api/sessions/{id}` when `session_mode == Mode::Explore`.

#### 1.3 X3 integration tests (`55f73f7`)
- `crates/desktop-server/tests/idea_flow.rs` (new, **485 lines**, 4 tests):
  1. `idea_preview_is_local_and_deterministic`: proves `POST /api/idea/preview` makes 0 HTTP requests, creates 0 sessions, extracts terms/IDs, surfaces F10 library matches with `owned` flag, and returns exact first queries.
  2. `idea_user_queries_are_exactly_shown_ones` (Rule P4): proves exact preview queries (`<= 6`, `arxiv`/`inspire` only) execute without approval; mutated query string, 7th query, omitted/empty online queries, and `crossref`/`semantic_scholar` providers return HTTP 400; `offline: true` sends 0 network requests.
  3. `idea_recorder_refused_outside_stage_and_resumes_after_crash` (Rules P1, S3): proves out-of-order stage recorders are refused, interrupted `idea.stage` renders `"Interrupted"` with Retry without auto-re-running on reopen, and `revise` advances revision cleanly.
  4. `idea_requests_all_pass_the_gate_and_auth` (Rule P6): proves every outbound literature request goes through `NetRateGate` (`FakeClock` accounting) and unauthenticated `/api/idea/preview` and `/api/sessions/{id}/idea/*` are rejected by `AuthGuard::authorize_api` (`401`/`403`).

---

### Package X4 — Model-free first screen UI (`crates/desktop-server`)

#### 1.4 Explore UI assets (`82bafb3`)
- `crates/desktop-server/assets/api_idea.js` (new, **78 lines**): Explore REST helpers (`previewIdea`, `createExploreSession`, `getExploreSession`, `postIdeaOp`, `resolveIdeaToken` with `sessionStorage` fallback; keeps `api.js` untouched).
- `crates/desktop-server/assets/idea_state.js` (new, **382 lines**): pure reducer (`SET_IDEA_VIEW`, `SET_HAS_MODEL`, `SET_BUSY`, `SET_ERROR`, `SET_NOTICE`, `SET_FILTER`, `RESET_FILTERS`, `APPLY_SSE_EVENT` syncing `stages`, `run_bar`, `candidates` badges, `map_stale`, and `where_i_looked`), selectors (`selectStageRail`, `selectRunBarModel` with `nonNegInt` preserving Quick `max_papers: 0` + `openItems`, `selectFilteredCandidates` with `qid -> provider` lookup, `selectEmptyState`), `formatCostLine`, Rust-aligned `isBarePaperIdentifier`, and regex-safe `highlightTermsSegments` (capped at 32 terms).
- `crates/desktop-server/assets/idea_setup.js` (new, **466 lines**): Explore setup card (`<= 4000` Unicode scalar chars, TeX allowed, optional seeds with `invalid_seeds` feedback, Quick/Standard/Thorough preset dial with ceilings, `"Offline: library only"` switch, debounced local preview with `previewSeq`/`lastPreviewKey` stale-reply protection, editable first-query checkboxes, immediate paste & input bare-ID redirection to Investigate, Ctrl+Enter submission with Rule P4 zero-query guard, zero `innerHTML`).
- `crates/desktop-server/assets/idea_search.js` (new, **256 lines**): decoupled `renderCandidateCards` (focus-preserving in-place patching) + `renderCandidateList` (filter bar, `<mark>` term highlights, `"not read"` / `"in your library"` / `"owned"` badges) and `renderWhereILooked` (provider status badges, formatted query statuses, library counts, `"Not searched"` list), zero `innerHTML`.
- `crates/desktop-server/assets/idea.js` (new, **345 lines**): `renderExploreSession` (7-stage rail, shared F6 `run_bar.js`, no-model notice, status/error/empty-failure banners, verbatim Idea card, candidate list, `"Where I looked"`, map placeholder) and `createExploreSessionController` (focus-preserving filter patching, `loadSeq` session-switch race guard, SSE envelope normalization, graceful HTTP 501 status notice on unbuilt Retry/Continue ops, paused budget-bump payload), zero `innerHTML`.
- `crates/desktop-server/assets/idea.css` (new, **332 lines**): styling using exclusively `tokens.css` variables (verified for dark/light theme parity; zero `--surface-*` undefined tokens) plus `.ui-run-bar-extra::after` cost-line separator.

#### 1.5 Registration seams (`d443b84`)
- `crates/desktop-server/src/assets.rs` (**490 lines**): registers `/assets/api_idea.js`, `/assets/idea_state.js`, `/assets/idea_setup.js`, `/assets/idea_search.js`, `/assets/idea.js`, `/assets/idea.css`.
- `crates/desktop-server/assets/new_menu.js` (**213 lines**): exports `isExploreCardVisible` and `mountExploreCard` (hides and clears container when flag is off — never greyed out; injects `/assets/idea.css` once in real browser DOM; defaults `onFocusInvestigate` to focusing `#investigation-v4-entry-input` or `#new-investigation-btn`; auto-mounts `#home-explore-card` when present). Default `MODE_MENU_ENTRIES` left unchanged so F5's `new_menu_and_rail.test.mjs` stays 100% green.

#### 1.6 X4 UI & state tests (`3361558`)
- `crates/desktop-server/tests/idea_state.test.mjs` (new, **521 lines**, 5 test groups): covers stage rail, F9 run-state labels, F6 `run_bar` cost line including Quick `max_papers: 0` (`0 of 0 papers`) and `done_open_items`, `qid -> provider` filtering, no-model banner, all 4 empty/failure states, Rule S8 across textContent and `title`/`placeholder`/`aria-label` attributes (`\b(verified|new|novel|proved)\b`), visibility flag hiding, Rust-aligned bare paper ID validation, stale-preview race protection, Ctrl+Enter zero-query guard, focus-preserving filter updates, `loadSession` race guard, SSE envelope updates, HTTP 501 status notice handling, XSS zero-`innerHTML` safety, `assets.rs` registration, and `tokens.css` variable validation for `idea.css`.

---

## 2. Independent audits performed

Two independent hard audits (using separate read-only subagents) were executed before committing each checkpoint:
1. **X3 Audit (8 HIGH/MEDIUM findings — all fixed in `1dd6c89`–`55f73f7`):**
   - Rejected omitted or empty `queries` in online Explore creation (Rule P4).
   - Scoped `resolve_stages` / `next_stage` / `validate_recorder_stage` to the latest `idea.stated` revision, enforced latest-stage prerequisite ordering, and routed post-`Sharpen` transitions to `Search` (Rule P1).
   - Added `ActiveExploreGuard` + `CancelToken` so an active first search does not look `"Interrupted"` and `/idea/stop` cancels cooperatively (Rule S3).
   - Shared `normalize_fields`, trimmed preview text, merged library matches into online candidate pools, and checked aliases + `source_items` for `owned` ownership.
   - Added `camelCase` `modelCallsUsed`/`modelCallsMax` in `idea_view::build_run_bar`, reported `"not_attempted"` for Crossref, guarded `"candidates are still listed"`, and added the F6 edge translation table.
   - Checked deleted-session ID conflicts, rolled back `meta.json` on creation failure, guarded `revise`/`stop` with `ledger_write_lock`, and enforced strict integer validation on `continue` budget bumps.
2. **X4 Audit (3 CRITICAL, 5 HIGH, 7 MEDIUM findings — all fixed before committing `82bafb3`–`3361558`):**
   - Fixed stale-preview race (`lastPreviewKey` + `previewSeq` invalidation on input/error), enforced Rule P4 zero-query guard inside `submitExplore` (including Ctrl+Enter and empty `first_queries`), added immediate `paste` listener + Unicode scalar `[...trimmed].length` counting + `invalid_seeds` display (`idea_setup.js`).
   - Fixed Quick `max_papers: 0` truthy fallback bug (`0 || 6` -> `nonNegInt`), synced `APPLY_SSE_EVENT` with `stages`/`run_bar`/`candidates` badges/`map_stale`, added `qid -> provider` lookup and `openItems`, and aligned `isBarePaperIdentifier` with Rust's `parse_paper_input`/`split_arxiv_id` (`idea_state.js`).
   - Replaced undefined `--surface-*` tokens with verified `tokens.css` variables for dark/light theme parity (`idea.css`).
   - Decoupled `renderCandidateCards` for focus-preserving filter typing, added `loadSeq` session-switch race guard, normalized SSE envelopes, and handled HTTP 501 as a clean status notice (`idea_search.js`, `idea.js`).
   - Added `sessionStorage` CSRF fallback (`api_idea.js`), moved imports to top of `new_menu.js`, injected `/assets/idea.css` once on mount, and upgraded `idea_state.test.mjs` fake DOM to browser-accurate `join('')` + full attribute Rule S8 scanning.

---

## 3. Deviations, decisions taken alone, and assumptions

### Deviations from the design
1. **Split `idea_search.rs` (511 lines) out of `idea.rs` (580 lines):**
   - *Design (`exp-x3-x4-shell.md` 5):* listed `idea.rs`, `idea_view.rs`, `routes_idea.rs`.
   - *Why:* keeping preview, Rule P4 query verification, F11 execution, and F10 library matching in `idea.rs` exceeded the 600-line ceiling (1052 lines). Splitting them into private submodule `mod idea_search;` keeps every file under 600 lines with zero public API change.
2. **Created `assets/api_idea.js` (78 lines) instead of editing `assets/api.js`:**
   - *Brief allowance (`exp-x3-x4-shell.md` 4 & 5):* explicitly permitted `api_idea.js` if `api.js` approached 550 lines. `api.js` is already 502 lines after prior merges; `api_idea.js` keeps `api.js` untouched and avoids merge conflicts with parallel lanes.
3. **Preserved F5 bare-Explore 400 behavior on `POST /api/sessions` without `text`:**
   - *Why:* existing F5 test `mode_shell::unbuilt_modes_refuse_session_creation_and_create_nothing` posts `{"id":"s-explore","mode":"explore"}` (no `text`) and asserts HTTP 400 containing `"Explore"` and `"not built yet"`. `routes_session.rs` delegates `mode: "explore"` with `text` to `routes_idea::create_explore_session`, and returns `400` `"Explore sessions require an idea \`text\` at creation; bare Explore sessions (tool profile \`explore/1\`) are not built yet."` when `text` is absent.
4. **Synchronous first-search execution inside `POST /api/sessions`:**
   - *Design (`explore.md` 3):* notes the first screen opens quickly (`< 1 s`) with candidates streaming in over SSE.
   - *Why:* in X3, `create_explore_session` runs the first-search queries synchronously before returning `201` (still emitting SSE progress events during execution), which makes `POST /api/sessions` deterministic for tests and immediately returns the populated `idea` view. Moving first-search execution to a background worker thread once `app.js` navigates immediately on `201` is straightforward because `ActiveExploreGuard` and `CancelToken` are already in place (see Merge Notes).

### Decisions taken alone
1. **FTS5 library preview strategy (`idea_search::search_library_matches`):** `library_db_query::build_safe_fts_query` joins terms with spaces (implicit FTS5 AND), which misses partial overlaps on multi-term ideas. Preview queries F10 `Library::search` per top extracted term (up to 6) plus exact `get_paper` lookups for seeds/paper_refs, deduplicating by canonical `PaperKey`.
2. **Visibility flag default off (`EXPLORE_VISIBILITY_STORAGE_KEY = 're-search-explore-enabled'`):** `isExploreCardVisible` defaults to `false` unless explicitly enabled via options, `window.__RE_SEARCH_EXPLORE_ENABLED__`, or `localStorage`. When off, `mountExploreCard` sets `hidden = true` and clears children (never greyed out).
3. **No edits to `index.html` or `app.js`:** both are on the Do-Not-Edit list (`app.js` is 932 lines; `index.html` is owned by F5). `new_menu.js` exports `mountExploreCard` and auto-mounts `#home-explore-card` when present.

### Assumptions
- Assumed future X6–X11 stages will emit `idea.stage` events matching `idea::StageId` (`sharpen`, `search`, `triage`, `read`, `map`, `directions`) and `ExploreStageState` strings (`requested`, `plan_approval`, `finished`, `stopped_at_cap`, `failed`, `interrupted`), and will read `idea::next_stage` / `idea::validate_recorder_stage` before mutating stage records.
- Assumed X11/X12 map events (`idea.map`) will populate `nodes` and `edges` whose edge kinds map via `idea_view`'s F6 edge translation table (`supports`, `contradicts`, `uses`, `extends`, `related`).
- Assumed `app.js` (once split under 600 lines) will mount `#home-explore-card` in `index.html` and route opened `mode === "explore"` sessions to `createExploreSessionController`.

---

## 4. Merge notes

### Existing files edited (small, local, in separate commits)
- `crates/desktop-server/src/lib.rs` (**227 lines**, commit `b132b64`): declared `pub mod idea; pub mod idea_view; pub mod routes_idea;`.
- `crates/desktop-server/src/api_dispatch.rs` (**395 lines**, commit `b132b64`): added 2 arms for `POST /api/idea/preview` and `POST /api/sessions/{id}/idea/{op}` behind `AuthGuard::authorize_api`.
- `crates/desktop-server/src/routes_session.rs` (**532 lines**, commit `b132b64`): delegated `POST /api/sessions` (`mode: "explore"` + `text`) to `routes_idea::create_explore_session`; attached `"idea": idea_view::view(&events, running)` on `GET /api/sessions/{id}` for Explore sessions.
- `crates/desktop-server/src/assets.rs` (**490 lines**, commit `d443b84`): registered 6 static Explore assets (`/assets/api_idea.js`, `idea_state.js`, `idea_setup.js`, `idea_search.js`, `idea.js`, `idea.css`).
- `crates/desktop-server/assets/new_menu.js` (**213 lines**, commit `d443b84`): exported `isExploreCardVisible` and `mountExploreCard`; auto-mounts `#home-explore-card` if present.

### Integration hooks for X6, X7, X11/X12, and foundation UI shell
- **For X6 (Sharpen & Search plan):**
  - Wire `op == "answer"` and `op == "plan"` in `crates/desktop-server/src/routes_idea.rs::handle_idea_op` (currently explicit `501`).
  - Gate stage entry/recording through `idea::next_stage` and `idea::validate_recorder_stage` (Rule P1), and check `idea::check_explore_budgets` before running model calls (Rule P3).
- **For X7 (`explore/1` tool profile & model-driven search):**
  - Activate `PROFILE_EXPLORE_V1` (`"explore/1"`) in `research-tools/src/tool_profile.rs` and update `tests/golden/tool_profiles.txt` + `tests/mode_markers_r2.rs::EXPECTED_EMITTED_EVENTS` for new Explore tools.
  - Wire `op == "search"` and `"continue"` (for model stages) in `routes_idea.rs::handle_idea_op`.
  - Optionally move `idea_search::run_first_search` onto a background thread inside `create_explore_session` once the frontend navigates immediately on `201` (`ActiveExploreGuard` and `CancelToken` are already wired).
- **For X11 / X12 (`idea_view.rs` headroom):**
  - `crates/desktop-server/src/idea_view.rs` is at **593 lines** (7 lines of headroom). Before expanding `idea_view.rs` for Triage/Read/Map/Directions projections in X8–X13, split map/coverage projection helpers into a sibling `idea_view_map.rs` submodule.
- **For foundation / UI shell integration (`index.html` and `app.js`):**
  - Add `<div id="home-explore-card" hidden></div>` to `index.html` (or its home-screen partial) so `new_menu.js` mounts the Explore setup card automatically when `re-search-explore-enabled` is enabled.
  - Once `app.js` (932 lines) is split under 600 lines, wire opened sessions with `session.mode === "explore"` to `createExploreSessionController(hostEl, { hasModel })` from `/assets/idea.js`, forwarding SSE `idea.*` events to `controller.handleSseEvent(envelope)`, and pass `onCreated` to `mountExploreCard` to open the newly created session.

---

## 5. Verification commands and exact results

| Command | Result |
|---|---|
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | Exit 0: `No touched source file is over 600 lines.` |
| `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` | Exit 0 (all JS files valid syntax) |
| `node --test crates/desktop-server/tests/idea_state.test.mjs` | Exit 0: **5 passed, 0 failed, 0 skipped** |
| `node --test crates/desktop-server/tests/*.test.mjs` | Exit 0: **172 passed, 0 failed, 0 skipped** (includes 5 new tests in `idea_state.test.mjs`) |
| `cargo test -p desktop-server --test idea_flow --test mode_shell --test mode_markers_r2 --test handoff_routes` | Exit 0: **13 passed (4 `idea_flow` + 4 `mode_shell` + 3 `mode_markers_r2` + 2 `handoff_routes`), 0 failed, 0 ignored** |
| `cargo test -p research-tools --test tool_def_golden --test tool_profile_golden` | Exit 0: **6 passed (3 + 3), 0 failed, 0 ignored** (`tool_defs.txt` and `tool_profiles.txt` unchanged) |
| `cargo test --workspace --no-fail-fast` (without `RE_SEARCH_PYTHON`) | Exit 0: **1002 passed, 0 failed, 2 ignored** (pre-existing live-tier `eval_harness.rs:420,433`) |
| `cargo clippy -p desktop-server --all-targets -- -D warnings -A clippy::manual_unwrap_or` | Exit 0 (**0 warnings**) |
| `cargo clippy --workspace --all-targets -- -D warnings` | Exit 101 solely on pre-existing untouched `crates/desktop-server/src/evidence.rs:744` (`clippy::manual_unwrap_or`) |
| `cargo clippy --workspace --all-targets -- --no-deps` | Exit 0 (**0 warnings** in any file touched by X3/X4; only the 6 pre-existing untouched Linux warnings below) |

---

## 6. Pre-existing failures/lints on Linux, unrun Windows paths, and bugs noticed

### Pre-existing clippy warnings under `rustc 1.100.0` on Linux (untouched files; also documented in `reports/f5-f7-mode-shell.md`)
- `crates/desktop-server/src/evidence.rs:744`: `clippy::manual_unwrap_or` (listed in `RULES.md`; file is > 600 lines and untouched).
- `crates/agent/src/guardrails.rs:335`: `clippy::items_after_test_module`.
- `crates/agent/tests/eval_support/manifest.rs:289`: `clippy::match_like_matches_macro`.
- `crates/agent/tests/eval_support/sheet.rs:102`: `clippy::unnecessary_map_or`.
- `crates/research-tools/tests/sandbox_and_fs.rs:6,9`: unused imports on non-Windows.
- `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6`: unused imports on non-Windows.

### Pre-existing Linux test interaction noticed (not fixed; outside scope)
- `crates/cli/tests/desktop_and_ext.rs:149-168`: exporting `RE_SEARCH_PYTHON` globally on Linux causes the spawned `re-search run` binary to attempt Windows `AppContainerSandbox` initialization and exit with code 2 before `ScriptedMock` connects (documented in `reports/f1-tool-profiles.md` and `reports/f5-f7-mode-shell.md`). Workspace tests are run without `RE_SEARCH_PYTHON`, and `RE_SEARCH_PYTHON=$(command -v python3) cargo test -p worker -p export` is run separately if needed.

### Could not be compiled or run on Linux
- All `#[cfg(windows)]` code paths and tests: `AppContainerSandbox` / `JobObjectSandbox`, Windows WebView2 runtime (`crates/desktop-app`), and Windows-only `chatgpt-auth` vault lock tests.
- `crates/agent/tests/eval_harness.rs:420,433`: live-tier evaluation tests (`#[ignore]`).
- Real browser rendering of `idea.js` / `idea_setup.js` / `idea.css` (all JS logic, DOM creation, focus preservation, XSS safety, and CSS token validity were verified under Node 22 with a browser-accurate fake DOM, not a live browser engine).

### Bugs / design tensions noticed (not fixed; outside scope)
- **`crates/desktop-server/assets/app.js` (932 lines) and `crates/desktop-server/src/session_view.rs` (753 lines):** both exceed the 600-line ceiling and prevented direct runtime mounting in `app.js` within this brief. `routes_session.rs` attaches `"idea"` directly without touching `session_view.rs`, and `new_menu.js` exports `mountExploreCard` for `index.html`/`app.js` integration once split.
- **`crates/desktop-server/tests/mode_shell.rs:86` vs `POST /api/sessions` (`mode: "explore"`):** F5's test expects `POST /api/sessions` with `{"id":"s-explore","mode":"explore"}` (no `text`) to fail with HTTP 400 `"not built yet"`. Once `explore/1` is built in X7 and Explore becomes generally enabled, that F5 test will need updating alongside `new_menu.js::MODE_MENU_ENTRIES`.
