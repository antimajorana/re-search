# Report: `f5-f7-mode-shell` (packages F5 — mode shell, and F7 — hand-off packet)

- **Base commit (start of F5):** `4e0fbaaf4ef51cf83e7fa3d65b484cb4709a3431` (`origin/gemini/work/fixed`)
- **Mid-brief merge (before F7, per brief):** `c7ba452` (`origin/gemini/work/fixed`, merged cleanly at `b778b9d` with zero conflicts)
- **Work branch:** `work/f5-f7-mode-shell` (pushed to `gemini/modes/f5-f7-mode-shell`)
- **Toolchain on this machine:** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `cargo 1.100.0 (3acc79c0c 2026-11-05)`, `Node v22.23.5`, `Python 3.12.16` on Linux x86_64
- **Unchanged guard rails:**
  - `Cargo.lock` untouched (0 new dependencies).
  - `crates/research-tools/tests/golden/tool_defs.txt` and `crates/research-tools/tests/golden/tool_profiles.txt` untouched; both golden tests pass.
  - No tool added to any registry and no tool definition modified.
  - `crates/policy/Cargo.toml` and `crates/ledger/Cargo.toml` remain free of `tokio`, `rmcp`, and `windows-sys`.
  - `crates/desktop-server/tests/eval_set_r.rs`, `crates/agent/tests/resume_loop.rs`, `crates/agent/src/resume_drift_tests.rs`, `crates/desktop-server/assets/app.js`, and `crates/desktop-server/src/cli_turn.rs` are untouched.
  - `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` and `--base 4e0fbaaf4ef51cf83e7fa3d65b484cb4709a3431` both exit 0 (`No touched source file is over 600 lines.`).
  - All three room-making targets are under 560 lines: `http_srv.rs` (**385**), `routes_session.rs` (**522**), `assets/index.html` (**550**).

---

## 1. What was implemented

### Package F5 — Mode shell (`crates/desktop-server`)

#### 1.1 Make room first (three zero-behaviour-change commits)
- **`48f0d76` — Extract API dispatch table from `http_srv.rs`:**
  - `crates/desktop-server/src/api_dispatch.rs` (new, 333 lines at extraction; 343 lines after `/api/library` and F7 handoff registrations): holds `dispatch_api` verbatim behind `AuthGuard::authorize_api`.
  - `crates/desktop-server/src/http_srv.rs` (702 -> **385 lines**): retains server loop, request parsing, loopback `Host` + auth/origin gating, and response writers (`ParsedRequest`, `write_api_res`, `write_json`, `write_bytes` exposed `pub(crate)`).
  - `crates/desktop-server/src/lib.rs` (219 lines): declares `mod api_dispatch;`.
- **`7011438` — Extract provider settings handlers from `routes_session.rs`:**
  - `crates/desktop-server/src/routes_settings.rs` (new, **107 lines**): holds `handle_get_settings` and `handle_put_settings` verbatim.
  - `crates/desktop-server/src/routes_session.rs` (616 -> **522 lines** after F5 wiring): re-exports `handle_get_settings` and `handle_put_settings` (`pub use`) so every existing import continues to compile unchanged.
  - `crates/desktop-server/src/lib.rs`: declares `pub mod routes_settings;`.
- **`fbc9c69` — Extract HTML partials from `assets/index.html`:**
  - `crates/desktop-server/assets/index.html` (768 -> **550 lines** after adding the New menu button and `<script type="module" src="/assets/new_menu.js">`).
  - Six extracted partials spliced byte-for-byte identically via `assets::render_index_html()` (verified against original SHA-256 `0901c6e3ac0420d5777747a9d0fa38391c341bd8fd385a5a5a6b72fd966200b0` before the F5 menu edit):
    - `crates/desktop-server/assets/index_investigation.html` (**36 lines**)
    - `crates/desktop-server/assets/index_latex_body.html` (**24 lines**)
    - `crates/desktop-server/assets/index_dag_top.html` (**24 lines**)
    - `crates/desktop-server/assets/index_sources.html` (**62 lines**)
    - `crates/desktop-server/assets/index_source_reader.html` (**18 lines**)
    - `crates/desktop-server/assets/index_dialogs.html` (**66 lines**)
  - `crates/desktop-server/src/assets.rs` (**439 lines**): adds `render_index_html()` with LF/CRLF placeholder support and unit test `render_index_html_expands_all_partials_cleanly`, plus registers `/assets/new_menu.js`.

#### 1.2 Register `/api/library/..` (`e5a04ab`)
- `crates/desktop-server/src/api_dispatch.rs`: registers the exact arm specified in `routes_library.rs`:
  ```rust
  (_, ["api", "library", rest @ ..]) => write_api_res(
      &mut stream,
      crate::routes_library::handle_library(state, method, rest, &req.query, &req.body),
  ),
  ```
  sitting behind `AuthGuard::authorize_api` in `http_srv.rs`.
- `crates/desktop-server/tests/mode_shell.rs` (**279 lines**): `library_routes_require_header_token_and_answer_when_authenticated` proves unauthenticated GET/POST and cookie-only GET return `401`, foreign `Origin` mutation returns `403`, and authenticated `X-ReSearch-Token` GET returns `200` with seeded library entries.

#### 1.3 `session_mode.rs` and mode-to-profile wiring (`46a8692`, `6ad3af9`)
- `crates/desktop-server/src/session_mode.rs` (new, **203 lines**):
  - Single source of truth for desktop session mode detection (`session_mode(events)` delegates directly to `research_tools::tool_profile::session_mode_from_events` — zero duplicate rules).
  - Named constants with comments referencing `00-overview.md` 5.2 / 5.3 / 8.2: `MODE_API_RESEARCH`, `MODE_API_INVESTIGATE` (`"investigation"`), `MODE_API_INVESTIGATE_ALIAS` (`"investigate"`), `MODE_API_EXPLORE`, `MODE_API_SOLVE`, `LABEL_RESEARCH` (`"Research (earlier)"`), `LABEL_INVESTIGATE`, `LABEL_EXPLORE`, `LABEL_SOLVE`, and `HOST_ONLY_MODE_MARKERS`.
  - `profile_for_mode(mode)` and `profile_for_session(events)` resolve built profiles via `research_tools::tool_profile` and return a plain user-facing `"not built yet"` error for unbuilt modes (`Explore`, `Solve`).
  - Unit tests cover latest-marker mode resolution, built vs unbuilt profile lookup, and mode parsing.
- `crates/desktop-server/src/routes_session.rs` (**522 lines**): uses `session_mode` in `handle_list_sessions`, `handle_create_session` (accepts `"investigate"` alias; refuses `"explore"` and `"solve"` with `400` `"not built yet"` creating nothing), and `handle_get_session`.
- `crates/desktop-server/src/turn_runner.rs` (**479 lines**):
  - `spawn_turn_thread` checks `session_mode::profile_for_session(&pre_events)` before reserving or spawning any worker thread (refuses unbuilt-mode turns with `400` and writes zero ledger events).
  - `run_session_turn` resolves the session's `ToolProfile` via `session_mode::profile_for_session(&existing)` and calls `routers::build_desktop_router_for_profile`.
- `crates/desktop-server/src/investigation.rs` (**597 lines**, isolated 1-line commit `6ad3af9`): `is_investigation` delegates to `crate::session_mode::session_mode(events) == Mode::Investigate`.
- `crates/desktop-server/tests/mode_shell.rs`: adds `unbuilt_modes_refuse_session_creation_and_create_nothing`, `unbuilt_mode_sessions_report_mode_and_refuse_turns_starting_nothing`, and `ui_approver_prompts_and_resolves_outside_a_running_model_turn` (settling `00-overview.md` 3.1 row 13).

#### 1.4 Marker test — Rule R2 (`fb7a517`)
- `crates/desktop-server/tests/mode_markers_r2.rs` (new, **426 lines**, 3 tests):
  1. `every_registered_tool_emits_no_host_mode_marker`: enumerates every tool registered across all built profiles (`research/1`, `investigation/1`), MCP `set_session_title`, `build_lean_router`, and `paper_reader::build_reader_router`, verifying that every tool id is mapped in the exhaustive audit table and zero tools emit any of `HOST_ONLY_MODE_MARKERS` (`investigation.started`, `investigation.handoff`, `idea.started`, `solve.started`).
  2. `locally_executed_tool_handlers_emit_no_host_mode_markers`: runs locally executable ledger-writing handlers (`ledger_append`, `ledger_supersede`, `ledger_note`, `record_paper_map`, `record_paper_summary`, `record_paper_branch`, `record_paper_conventions`, `latex_patch`, `set_session_title`) against a real ledger and asserts zero host mode markers are appended.
  3. `tool_handler_sources_do_not_append_host_mode_markers`: static source audit of every `.append` / `.append_with_hash` / `.append_new_session` call across all 17 tool handler source files in `research-tools`, `tools`, `cli-route`, and `desktop-server/src/paper_reader.rs`.

#### 1.5 New menu & session rail mode tags (`a5c9b7f`)
- `crates/desktop-server/assets/new_menu.js` (new, **154 lines**):
  - Zero `innerHTML` writes (DOM built exclusively with `createElement` + `textContent`).
  - `MODE_MENU_ENTRIES` (`Investigate` enabled; `Explore` and `Solve` disabled with `NOT_BUILT_YET_LABEL = 'not built yet'` and inert even if `.disabled` is removed programmatically).
  - `New research` placed under `More` (`<details id="rail-more-menu">`).
  - Self-binds `#rail-new-menu` on DOM load without editing the 932-line `assets/app.js`.
- `crates/desktop-server/assets/session_rail.js` (**246 lines**): adds `MODE_TAG_LABELS`, `formatModeTag(rawMode)` (`Investigate`, `Explore`, `Solve`, `Research (earlier)`), escaped mode rail pill (`session-mode-tag`), and mode label in rail search haystack.
- `crates/desktop-server/assets/index.html` (**550 lines**) and `crates/desktop-server/assets/investigation.css` (**106 lines**): adds the `#rail-new-menu` container and styling for mode pills and disabled `"not built yet"` menu rows.
- `crates/desktop-server/tests/new_menu_and_rail.test.mjs` (new, **291 lines**, 4 tests): tests `new_menu.js`, `session_rail.js`, XSS escaping, and static wiring.

---

### Package F7 — Hand-off packet (`crates/export` and `crates/desktop-server`)

#### 1.6 Shared packet, quote re-verification, prefills, quoted wrapping, and read-only status (`f884f11`, `3c9d067`)
- `crates/export/src/fork.rs` (**280 lines**, isolated 4-line visibility commit `f884f11`): exposes `generate_session_id`, `StagedSources` (with `pub(crate) committed`), and `copy_session_sources` as `pub(crate)`. **`research_from_paper` and `fork_session` are untouched byte for byte.**
- `crates/export/src/handoff_types.rs` (new, **341 lines**):
  - Named constants referencing `00-overview.md` 4.1 / 8.2 decision 10: `HANDOFF_PACKET_EVENT` (`"handoff.packet"`), `HANDOFF_SENT_EVENT` (`"handoff.sent"`), `HANDOFF_SCHEMA_VERSION` (`1`), `MODEL_TEXT_SUFFIX` (`"(model)"`).
  - `HandoffPacket`, `HandoffStatement`, `ContextRole`, `HandoffContextItem`, `ModelTextEntry` (`display_label()` ensuring `"(model)"`), `HandoffAsk` (`#[serde(tag = "kind", content = "fields")]` with `SolveHandoffAsk` using `research_tools::solve_model` field names, `ExploreHandoffAsk` with `ExploreQuestionKind::{Extended, PriorWork}`, `origin_paper`, and `seed_papers`, and `InvestigateHandoffAsk`), `HandoffRequest`, `DroppedQuote`, `HandoffReceipt`.
  - Every input struct carries `#[serde(deny_unknown_fields)]` so badges, tiers, and verdicts cannot cross a hand-over (Rule 2).
  - Reuses F2 `PaperRef`, `NodeRef`, `Mode` and F4 `QuoteRef`.
- `crates/export/src/handoff_view.rs` (new, **454 lines**):
  - Deterministic zero-model prefills (`build_prefill`, `prefill_solve_setup`, `prefill_explore`, `prefill_investigate`): Solve prefill emits only `PrefillStepKind::Cited` (when a quote matched in the child) or `PrefillStepKind::Given` (otherwise), never `Verified` or `Numerically checked` (Rule 2); Explore prefill preserves `excluded_origin_paper` and `seed_papers`; Investigate prefill collects child-verified quotes.
  - Rule 3: `wrap_packet_as_quoted_data` wraps packet text in a per-call random `<HANDOFF_DATA_<16 hex>>` boundary and neutralizes any colliding boundary tokens inside untrusted text.
  - Rule 5: `read_handoff_child_status` and `read_session_handoffs` inspect child sessions (`solve.done`, `solve.paused`, `idea.verdict`, `investigation.map`, `investigation.skeleton`) strictly read-only without writing anything to the origin session, and recover `quotes_dropped` read-only from origin `handoff.sent` when reloading a child prefill.
- `crates/export/src/handoff.rs` (new, **334 lines**):
  - `execute_handoff(data_dir, req)`: verifies origin ledger via `read_verified_session`, copies needed verified source files via `fork::copy_session_sources`, re-matches every `QuoteRef` against the child's own copied files and receipts via `research_tools::quote_core::verify_quote_ref` (Rule 1: drops failed quotes into `dropped_quotes` and clears parent `evidence_seq` to `None`), writes child events in exact order `[source.ingested..., handoff.packet, <target_marker>]`, and appends `handoff.sent` to origin.
- `crates/export/src/handoff_tests.rs` (new, **486 lines**, 5 tests) and `crates/export/src/lib.rs` (**135 lines**): covers all three target payloads, exact event ordering, `Ledger::verify_all`, Rule 1 quote re-matching/dropping/`evidence_seq` clearing, Rule 2 `deny_unknown_fields` and Cited/Given-only prefill, Rule 3 random boundary wrapping, Rule 5 read-only status propagation, and rejection of `Mode::Research` / mismatched `ask.kind`.

#### 1.7 Desktop HTTP hand-off routes (`1b06104`)
- `crates/desktop-server/src/handoff.rs` (new, **86 lines**):
  - `handle_create_handoff` (`POST /api/sessions/{id}/handoff`): validates `session_id`, verifies session exists (`404` if missing), deserializes `HandoffRequest` (`400` on invalid/unknown fields), enforces `req.packet.from.session == session_id`, calls `export::handoff::execute_handoff`, builds deterministic prefill via `build_prefill`, saves child `SessionMeta` (inheriting workspace/provider/model settings, `allow_model_title: Some(false)`), attaches child to origin's project if any, and returns `201 Created` with `{ session_id, mode, mode_label, target_mode_built, receipt, prefill }`.
  - `handle_get_handoffs` (`GET /api/sessions/{id}/handoffs`): validates `session_id`, verifies session exists, reads incoming packet/prefill and live read-only outgoing child statuses via `read_session_handoffs`, returns `200 OK`.
- `crates/desktop-server/src/api_dispatch.rs` (**343 lines**) and `crates/desktop-server/src/lib.rs` (**219 lines**): registers both routes behind `AuthGuard::authorize_api` and exposes `pub mod handoff;`.
- `crates/desktop-server/tests/handoff_routes.rs` (new, **350 lines**, 2 tests): verifies auth (`401` without token / cookie-only, `403` foreign Origin), packet validation (`from.session` mismatch, smuggled `badge`), full Investigate -> Solve / Investigate -> Explore / Explore -> Investigate flows, project inheritance, quote re-verification, unbuilt-mode turn refusal on Solve children, and strictly read-only status flow-back.

---

## 2. Deviations, decisions taken alone, and assumptions

### Deviations from the design
1. **Wire `"mode"` value for Investigate kept as `"investigation"` on GET responses (`session_mode::MODE_API_INVESTIGATE`):**
   - *Design (`00-overview.md` 5.2):* on-screen label `"Investigate"`, internal prefix `investigation.*`; `paper_key::Mode::as_str()` is `"investigate"`.
   - *Why kept as `"investigation"` on `/api/sessions` GET:* existing API tests (`crates/desktop-server/tests/investigation_flow.rs:52,244`, which must not be weakened) and existing `assets/investigation.js` assert `mode === "investigation"`. To honour both the existing contract and F5's new mode names: `POST /api/sessions` accepts both `"investigation"` and `"investigate"`, `GET /api/sessions` returns `"investigation"` (while `handoff.packet.target_mode` uses `paper_key::Mode` = `"investigate" | "explore" | "solve"` per `00-overview.md` 4.1), and `session_rail.js` renders `"Investigate"`, `"Explore"`, `"Solve"`, and `"Research (earlier)"`.
2. **Solve packet prefill in F7 does not depend on S4 (not yet merged):**
   - *Design (`00-overview.md` 4.3):* "The prefill from a packet is built in F7 on top of S4."
   - *Why:* S4 (`solve.setup` UI/route) is on a parallel brief not present in this checkout. Per `RULES.md` ("Do not wait for it and do not implement it. Code against the interface the design defines"), F7 builds `export::handoff::prefill_solve_setup` (`SolvePrefill`) directly against S2's `research_tools::solve_model::{TaskKind, SolveTarget, SymbolSpec, DoneCriterion}` types (already merged in this checkout) and returns it from `POST /api/sessions/{id}/handoff` and `GET /api/sessions/{id}/handoffs` so S4 can consume it with zero adapter glue.
3. **HTML partials kept out of `assets/index.html` while retaining all IDs checked by existing static regex tests:**
   - Existing Node tests (`math_render.test.mjs`, `hitl_ui.test.mjs`, `projects_ui.test.mjs`, `source_links.test.mjs`) read `assets/index.html` directly from disk and assert specific element IDs. Moving 6 unasserted blocks into `index_*.html` partials shrunk `index.html` from 768 to 550 lines (< 560) while keeping every existing Node test passing untouched and serving the exact rendered HTML via `assets::render_index_html()`.

### Decisions taken alone
1. **Did not touch `assets/app.js` (932 lines) or `src/cli_turn.rs` (627 lines):** both exceed the 600-line ceiling (`RULES.md` forbids editing files already over 600 lines). `assets/new_menu.js` self-binds `#rail-new-menu` via `<script type="module" src="/assets/new_menu.js">`, and `cli_turn.rs` automatically inherits `session_mode` through `investigation::is_investigation`.
2. **Settled `00-overview.md` 3.1 row 13 (approver outside a running model turn):** inspected `approver.rs`, `routes_session.rs`, and `assets/app.js:693-710` and added `mode_shell.rs::ui_approver_prompts_and_resolves_outside_a_running_model_turn`. Result: **yes**, `UiApprover::request` prompts and resolves via `GET /api/sessions/{id}/approvals`, `POST /api/sessions/{id}/approvals/{call_id}`, and SSE `approval_requested`/`approval_resolved` with no running model turn (`is_turn_running == false`), and `app.js` subscribes to SSE on session selection regardless of `running`.
3. **Kept `lean_router` on frozen `investigation/1` and `research/1`:** `investigation_flow.rs:552` (`check_action_keeps_workers`) explicitly verifies `delegate_lean` is offered on an `investigation/1` recheck turn; matches `00-overview.md` 2.2.
4. **Hand-off HTTP routes (`POST /api/sessions/{id}/handoff`, `GET /api/sessions/{id}/handoffs`):** placed under `/api/sessions/{id}/...` matching existing `/api/sessions/{id}/fork`, `/api/sessions/{id}/investigation/research`, and `/api/sessions/{id}/diff/{other}`.
5. **Quote verbatim text recovery during child re-verification (`execute_handoff`):** checks (a) optional `req.quote_texts` map keyed by `quote_sha256`, (b) origin ledger `evidence.quote` / `investigation.evidence` / `investigation.{map,branch,summary}` events, and (c) read-only `SandboxRoot::Sources` line slice of the child's own copied file accepted **only** if `quote_sha256(slice) == qref.quote_sha256` before passing through `verify_quote_ref`. Any mismatch or missing source drops the quote and records it in `dropped_quotes`.
6. **Hand-off route allows creating child sessions for unbuilt target modes (`solve`, `explore`), while `POST /api/sessions` and `POST /api/sessions/{id}/messages` refuse them:** allows F7 hand-overs and deterministic setup prefills (`POST /api/sessions/{id}/handoff`) to be tested and inspected end-to-end (zero model calls, Rule 4), while `target_mode_built: false` tells the UI the mode is not runnable yet and any turn attempt fails closed with `400` `"not built yet"` writing zero events.

### Assumptions
- None regarding external network services (zero network calls in F5 and F7).
- Assumed `solve.done` carries `payload["outcome"]` (`"finished"` / `"finished_open_items"`), `solve.paused` carries `payload["reason"]`, and `idea.verdict` carries `payload["status"]` (or `payload["summary"]`) as described in `00-overview.md` 3.3 and 4.1 Rule 5. If Explore or Solve finalize a different field name in their packages, `export/src/handoff_view.rs::read_handoff_child_status` is the single place to update.

---

## 3. Merge notes

### Existing files edited (small, local, in separate commits)
- `crates/desktop-server/src/http_srv.rs`: extracted `dispatch_api` into `api_dispatch.rs` (`48f0d76`), switched `/` to serve `assets::render_index_html()` (`fbc9c69`).
- `crates/desktop-server/src/routes_session.rs`: extracted settings handlers into `routes_settings.rs` (`7011438`), wired `session_mode` into session list/create/get (`46a8692`).
- `crates/desktop-server/src/turn_runner.rs`: pre-turn `session_mode::profile_for_session` gate in `spawn_turn_thread` and `build_desktop_router_for_profile` in `run_session_turn` (`46a8692`).
- `crates/desktop-server/src/investigation.rs`: 1-line edit (`6ad3af9`) delegating `is_investigation` to `session_mode::session_mode(events) == Mode::Investigate`.
- `crates/desktop-server/src/assets.rs`: added `render_index_html()` (`fbc9c69`) and registered `/assets/new_menu.js` (`a5c9b7f`).
- `crates/desktop-server/src/lib.rs`: added module declarations `mod api_dispatch;`, `pub mod routes_settings;`, `pub mod session_mode;`, `pub mod handoff;`.
- `crates/desktop-server/assets/index.html`: extracted 6 partials (`fbc9c69`), added `#rail-new-menu` and `<script type="module" src="/assets/new_menu.js">` (`a5c9b7f`).
- `crates/desktop-server/assets/session_rail.js`: added `MODE_TAG_LABELS`, `formatModeTag`, escaped mode pill (`a5c9b7f`).
- `crates/desktop-server/assets/investigation.css`: added 6 lines of mode-pill and disabled menu-row styling (`a5c9b7f`).
- `crates/export/src/fork.rs`: visibility-only change (`f884f11`) making `generate_session_id`, `StagedSources`, and `copy_session_sources` `pub(crate)`.
- `crates/export/src/lib.rs`: added `pub mod handoff; mod handoff_types; mod handoff_view;` (`3c9d067`).

### Integration hooks for later briefs / packages
- **Adding new mode routes (`api_dispatch.rs`):** `crates/desktop-server/src/api_dispatch.rs` (343 lines) has > 250 lines of headroom; add one-line arms for Explore (`/api/sessions/{id}/explore/...`), Solve (`/api/sessions/{id}/solve/...`), and V16/X13/S4 endpoints directly in `dispatch_api` (all automatically behind `AuthGuard::authorize_api`).
- **Enabling Explore (`explore/1`) and Solve (`solve/1`) once built:**
  - `crates/desktop-server/src/session_mode.rs` automatically picks them up as soon as `research_tools::tool_profile::lookup` returns `Ok(ToolProfile)` for `explore/1` / `solve/1` — zero edits needed in `session_mode.rs`, `routes_session.rs`, or `turn_runner.rs`.
  - Flip the `built: false` flag on `Explore` / `Solve` in `crates/desktop-server/assets/new_menu.js` (`MODE_MENU_ENTRIES`) once their UI views ship.
- **V16 (Investigate hand-over buttons), X13 (Explore hand-over buttons), S4 (Solve setup card from packet):**
  - Call `POST /api/sessions/{id}/handoff` with a `HandoffRequest` (`export::handoff::HandoffPacket`), render the returned `prefill` (`SolvePrefill` / `ExplorePrefill` / `InvestigatePrefill`), and poll/read `GET /api/sessions/{origin}/handoffs` to render the live read-only status chips (`child_status.label`) on origin nodes.
  - Before injecting any `HandoffPacket` into a model prompt, wrap it with `export::handoff::wrap_packet_as_quoted_data(&packet)` (Rule 3).
- **Adding any new tool handler:** update the exhaustive `EXPECTED_EMITTED_EVENTS` table in `crates/desktop-server/tests/mode_markers_r2.rs` (and `TOOL_HANDLER_SOURCE_FILES` if a new handler file is added) so Rule R2 continues to guard every registered tool.

---

## 4. Verification commands and exact results

| Command | Result |
|---|---|
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | Exit 0: `No touched source file is over 600 lines.` |
| `python3 scripts/check_file_length.py --base 4e0fbaaf4ef51cf83e7fa3d65b484cb4709a3431` | Exit 0: `No touched source file is over 600 lines.` |
| `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` | Exit 0 (all JS files valid syntax) |
| `node --test crates/desktop-server/tests/*.test.mjs` | Exit 0: **161 passed, 0 failed, 0 skipped** (includes 4 new tests in `new_menu_and_rail.test.mjs`) |
| `cargo test -p research-tools --test tool_def_golden --test tool_profile_golden` | Exit 0: **6 passed (3 + 3), 0 failed, 0 ignored** (`tool_defs.txt` and `tool_profiles.txt` unchanged) |
| `cargo test -p export` | Exit 0: **13 passed (5 new `handoff::tests` + 4 `export_integrity` + 4 `fork_diff_export`), 0 failed, 0 ignored** |
| `cargo test -p desktop-server --test handoff_routes --test mode_shell --test mode_markers_r2 --test eval_set_r` | Exit 0: **14 passed (2 `handoff_routes` + 4 `mode_shell` + 3 `mode_markers_r2` + 5 `eval_set_r`), 0 failed, 0 ignored** |
| `cargo test -p agent --test resume_loop` | Exit 0: **6 passed, 0 failed, 0 ignored** |
| `cargo test -p agent --lib resume` | Exit 0: **12 passed, 0 failed, 0 ignored** (includes all `resume::drift_tests` and `resume_profile::tests`) |
| `RE_SEARCH_PYTHON=$(command -v python3) cargo test -p worker -p export --no-fail-fast` | Exit 0: **14 passed (13 `export` + 1 `worker` unit), 0 failed, 0 ignored** |
| `cargo test --workspace --no-fail-fast` (without `RE_SEARCH_PYTHON`) | Exit 0: **796 passed, 0 failed, 2 ignored** (pre-existing live-tier `eval_harness.rs:420,433`) |
| `cargo clippy -p export --all-targets -- -D warnings` | Exit 0 (**0 warnings**) |
| `cargo clippy -p desktop-server --all-targets -- -D warnings -A clippy::manual_unwrap_or` | Exit 0 (**0 warnings**) |
| `cargo clippy --workspace --all-targets -- --no-deps` | Exit 0 (**0 warnings** in any file touched by F5/F7; only the 6 pre-existing Linux warnings below) |

---

## 5. Pre-existing failures/lints on Linux, unrun Windows paths, and bugs noticed

### Pre-existing clippy warnings under `rustc 1.100.0` on Linux (untouched files, also documented in `reports/f1-tool-profiles.md`)
- `crates/desktop-server/src/evidence.rs:744`: `clippy::manual_unwrap_or` (listed in `RULES.md`; file is > 600 lines and untouched; note that clippy's suggestion would drop the status allowlist match).
- `crates/agent/src/guardrails.rs:335`: `clippy::items_after_test_module` (`mod computation_tests` precedes `check_final_answer_gate` / `run_research_loop`).
- `crates/agent/tests/eval_support/manifest.rs:289`: `clippy::match_like_matches_macro`.
- `crates/agent/tests/eval_support/sheet.rs:102`: `clippy::unnecessary_map_or`.
- `crates/research-tools/tests/sandbox_and_fs.rs:6,9`: unused imports `is_symlink_or_junction`, `TEX_AUDIT` on non-Windows.
- `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6`: unused imports `BufRead`, `BufReader` on non-Windows.

### Pre-existing Linux test interaction noticed (not fixed; outside scope)
- `crates/cli/tests/desktop_and_ext.rs:149-168`: as already documented in `reports/f1-tool-profiles.md`, exporting `RE_SEARCH_PYTHON` globally on Linux causes the spawned `re-search run` binary to attempt Windows `AppContainerSandbox` initialization and exit with code 2 before `ScriptedMock` accepts a connection, hanging `mock.finish()`. Running `cargo test --workspace --no-fail-fast` without `RE_SEARCH_PYTHON` and `RE_SEARCH_PYTHON=$(command -v python3) cargo test -p worker -p export --no-fail-fast` separately exercises both cleanly.

### Could not be compiled or run on Linux
- All `#[cfg(windows)]` code paths and tests: `AppContainerSandbox` / `JobObjectSandbox` (`crates/worker/tests/appcontainer_sandbox.rs`, `crates/worker/tests/python_worker.rs`, `crates/cli/tests/contained_router.rs`), Windows WebView2 runtime (`crates/desktop-app`), and Windows-only `chatgpt-auth` vault lock tests.
- `crates/agent/tests/eval_harness.rs:420,433`: live-tier evaluation tests (`#[ignore]`).

### Bugs / design tensions noticed (not fixed; outside scope)
- **`crates/desktop-server/assets/app.js` (932 lines) and `crates/desktop-server/src/cli_turn.rs` (627 lines):** both exceed the 600-line ceiling and will need splitting before any future brief edits them. In particular, `cli_turn.rs` handles subscription-CLI turns (`claude`, `codex`, `antigravity`) and will eventually need the same `session_mode::profile_for_session` guard for unbuilt modes once subscription-CLI turns support modes beyond Research/Investigate (today `cli_turn.rs` already inherits `session_mode` for Investigation vs Research via `investigation::is_investigation`).
- **`crates/desktop-server/src/evidence.rs:744`:** clippy's `manual_unwrap_or` auto-fix would silently remove the status allowlist guard (`"verified" | "failed" | ...`); it should be silenced with an explicit `#[allow(clippy::manual_unwrap_or)]` (not auto-fixed) if `evidence.rs` is ever split under 600 lines.
- **Wire `"investigation"` vs `"investigate"` naming:** `00-overview.md` 5.2 specifies `"Investigate"` on screen and `paper_key::Mode::Investigate.as_str() == "investigate"`, whereas existing `GET /api/sessions` tests (`investigation_flow.rs:52,244`) and `investigation.js` expect `"investigation"`. Noted in Section 2; if a future brief unifies the wire value to `"investigate"`, `session_mode::MODE_API_INVESTIGATE` is the single constant to change alongside those tests and `investigation.js`.
