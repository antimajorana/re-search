# Brief exp-x15-x16-chat-cite — Conversation turns (X15) and Citation expansion & second round (X16)

## 1. Packages and governing design sections

- **Packages:** `X15` (Conversation turns, `explore/1` chat tools, multi-paper line-citation check) and `X16` (Citation expansion & second round). Because `X15` and `X16` own completely disjoint files, they may run sequentially in one lane (pushing after each) or in parallel once `X11` + `F11` merge.
- **Governing design:**
  - `docs/design/modes-2026-11/00-overview.md`: sections 2.2 (tool profile `explore/1` frozen 4 tools), 2.4 (F3 network gate & approval rule), 2.5 (F4 `lines_sent` & `context.supplied`), 2.8 (F11 citation relations), 3.1 row 12 (policy per profile), 3.2 row 20 (`"matched"`, never `"verified"`), 3.3, and 8.2 decision 1.
  - `docs/design/modes-2026-11/explore.md`: sections 3 (`Afterwards`, `Thorough` preset: 36 queries, 80 screened, 12 papers, up to 4 calls/paper, hard cap 55 model calls), 4.1 (`SearchPlan` `cites`/`cited_by`, `Candidate`), 4.2 (`idea.citation_net`), 5.2 (`explore/1` tools: `idea_search`, `idea_fetch_papers`, `search_paper_passages`, `read_paper_lines`), 6 (rules C1, C2, C7, P3, P5, P6, P7, S2, S8), 8 (Explore sessions items 3–4: Conversation turns record `tool_profile: "explore/1"`), 9 (rows X15, X16), and 11 (LocalCitationNetwork & OpenScholar ideas, re-coded in Rust).
  - `docs/design/modes-2026-11/agents/RULES.md`.

## 2. Start conditions and gates

- **Required merges on `gemini/work/fixed`:**
  - Before **X15**: `X11` (`exp-x9-x10-x11-read-verdict`, which transitively brings `X7` `explore/1` profile & `build_idea_router` and `X9` real `idea_fetch_papers` handler), **`F5`** (`f5-f7-mode-shell`), AND **Solve `S12`** (`draft-solve-s12-loop`, strictly serializing edits to `crates/desktop-server/src/turn_runner.rs` — NEVER run `X15` concurrently with `F5` or `S12` on `turn_runner.rs`!). Base already has `F1`, `F4`, `F9`, `paper_passages.rs`, `paper_lines.rs`, and `investigation_cites.rs`.
  - Before **X16**: `X11` (`exp-x9-x10-x11-read-verdict`), `X3`/`X4`, `F5` (`api_dispatch.rs`), and `F11` (`lit_providers.rs` citation relations `cites` / `cited_by`).
- **No owner gate blocks X15/X16.** Note: `explore/1` was already frozen in `tool_profile.rs` and `tests/golden/tool_profiles.txt` in `X7` and wired with the real `idea_fetch_papers` handler in `X9`, so `X15` MUST NOT modify `tool_profile.rs`, `tool_defs.txt`, or `tool_profiles.txt`.

## 3. Base-commit verified interfaces vs future-merge interfaces

### Verified on base commit `832eab5bf`
- `crates/research-tools/src/tool_profile.rs` (329 lines): `PROFILE_EXPLORE_V1` (`"explore/1"`), `TOOL_PROFILE_FIELD`.
- `crates/agent/src/resume_profile.rs` (310 lines): `resolve_session_profile`, `verify_router_for_resume`, `insert_tool_profile` (records `tool_profile: "explore/1"` on `session.start` / `session.resume` and verifies fail-closed on resume).
- `crates/desktop-server/src/turn_runner.rs` (463 lines): currently calls `investigation::{prepare, finish, turn_context}` and `investigation_system_prompt::base_system_prompt` unconditionally at lines `71-81, 98, 198, 286, 394` — MUST verify post-`F5`/`S12` gating before adding `Mode::Explore`!
- `crates/research-tools/src/paper_passages.rs` (605 lines — **OVER 600, DO NOT EDIT!**): `SEARCH_PAPER_PASSAGES`, `def()`, `register()`.
- `crates/research-tools/src/paper_lines.rs` (202 lines): `READ_PAPER_LINES`, `def()`, `register()`.
- `crates/research-tools/src/lines_sent.rs` (465 lines): `lines_sent_with_ledger`, `SentLines::{covers, ranges_for}` — already credits `context.supplied`, `read_paper_lines`, `search_paper_passages`, and `fs_read` per `(source_id, path, file_sha256)`!
- `crates/desktop-server/src/investigation_cites.rs` (454 lines): existing single-paper `[Lstart–end]` citation-check pattern for Investigation question turns (reference only; do NOT break Investigation's single-paper behavior).
- `crates/research-tools/src/run_budget.rs` (464 lines): `count_model_calls`, `RunScope`.

### Future-merge interfaces to verify before coding
- **`X7`/`X9`:** `routers.rs::build_idea_router` (carrying `.with_tool_profile("explore/1")`), `idea_tools.rs` (`idea_search`, `idea_fetch_papers`).
- **`X3`/`X4`/`X11`:** `idea.rs`, `idea_view.rs`, `idea_search.js`, `idea.js`, `idea_map.rs` (node IDs & quotes for node context).
- **`F11`:** `lit_providers` `cites` and `cited_by` query runners and parsed citation edges keyed by F2 `PaperKey`.
- **`F5` & Solve `S12`:** `turn_runner.rs` mode dispatch (confirm whether `investigation::{prepare, finish, turn_context}` and `investigation_system_prompt::base_system_prompt` at lines `71-81, 98, 198, 286, 394` are already gated by `Mode::Investigate`, and verify `routes_session.rs` `POST /api/sessions/{id}/messages` entry path and `api_dispatch.rs`).

## 4. Smallest implementation checkpoints

1. **Checkpoint 1 — X15 (Conversation turns, multi-paper line-citation check, & chat citation UI):**
   - `crates/desktop-server/src/idea_system_prompt.rs` (new): Explore conversation system prompt injecting selected map node / idea context wrapped as quoted data (Rule C7), instructing the model to cite paper + line ranges only from lines returned by `search_paper_passages` / `read_paper_lines`, and forbidding novelty/verification claims.
   - `crates/desktop-server/src/idea_chat.rs` (new):
     - Runs Conversation turns entered via the existing **`POST /api/sessions/{id}/messages`** HTTP route (`routes_session.rs` -> `turn_runner.rs` `Mode::Explore` arm -> `idea_chat.rs`) on the Explore router (`build_idea_router`, profile `explore/1`: `idea_search`, `idea_fetch_papers`, `search_paper_passages`, `read_paper_lines`, `research_policy()` — zero `ExecuteCode` or `WriteWorkspace` tools, Rule P7).
     - In `turn_runner.rs`, verify `investigation::{prepare, finish, turn_context}` and `investigation_system_prompt::base_system_prompt` (`turn_runner.rs:71-81, 98, 198, 286, 394`) are gated strictly to `Mode::Investigate`; if F5/S12 did not yet guard them, guard them strictly to `Mode::Investigate` when adding the `Mode::Explore` branch so Explore turns never invoke Investigation hooks!
     - Records `tool_profile: "explore/1"` on `session.start` / `session.resume` via F1 (`resume_profile`) so later Conversation turns resume fail-closed without touching `resume.rs`.
     - Multi-paper line-citation checker: adapts `investigation_cites.rs`'s parsing and marking logic to handle multiple ingested papers (`[paper_key: Lstart-end]` / `[source_id: Lstart-end]`), crediting lines strictly via F4 `lines_sent::lines_sent_with_ledger` across every `(source_id, path, file_sha256)` sent to the conversation caller. Cited ranges NOT in `SentLines` are flagged as unread (never shown as matched).
     - Respects F9 run budget & stop controls; never leaks `"verified"` on screen (Rule S8).
   - `crates/desktop-server/assets/idea_chat_ui.js` (new, mounted in `idea.js`'s Conversation tab slot): renders multi-paper matched line-citation chips (opening source/page via F6 `quote_chip.js`) and explicit unread/unmatched citation warning badges, with zero raw `innerHTML` and zero `"verified"`.
   - Commit and push X15 (or run X16 in parallel on disjoint files).
2. **Checkpoint 2 — X16 (Citation expansion & second round):**
   - `crates/research-tools/src/idea_net.rs` (new): pure citation-network builder over F11 `cites` / `cited_by` relations of seed/core papers (LocalCitationNetwork idea, re-coded in Rust): constructs directed citation graph keyed by F2 `PaperKey`, computes deterministic in-network rank (inbound + outbound links within the network), and identifies `"cited by your core papers but unread"` candidates.
   - `crates/desktop-server/src/idea.rs` (owned sequentially after X3):
     - Adds `"Who cited this since"` (`origin: "app"`, zero model calls, calls `Router::try_call("host:idea:<session>", "idea_search", ...)` with `relation: cited_by` per Rule P5 — never bypasses router or F3 rate gate!).
     - Adds `"Search more about this node"` (`more` action) and Thorough preset citation expansion + second self-check round (OpenScholar idea: lists thin/under-supported map statements, builds targeted retrieval queries, runs under Thorough ceilings: 36 queries, 80 screened, 12 papers, 4 calls/paper, 55 model calls — Rule P3). Appends `idea.citation_net` and updates F12 coverage `citation_expansion { seeds, hops }`.
   - `crates/desktop-server/src/routes_idea_net.rs` (new, dedicated dispatcher — keeps X16 100% disjoint from `X12`/`X13` on `routes_idea.rs`!): exposes `POST /api/sessions/{id}/idea/more` and `POST /api/sessions/{id}/idea/cited-since` (and Round 2 trigger) behind `AuthGuard::authorize_api`, registered via a single 1-line match arm in `api_dispatch.rs`.
   - `crates/desktop-server/assets/idea_search.js` (owned sequentially after X9): renders `"Cited by your core papers but unread"` list, `"Who cited this since"` button (showing `"cited since" != "answered"` note on open questions, `explore.md` 12 risk 12), and Round 2 / Thorough progress.

## 5. Owned files

- **X15 (disjoint from X16, X17, X18; serialized on `turn_runner.rs` strictly after `F5` & `S12`):**
  - `crates/desktop-server/src/idea_chat.rs` (new, `<= 600` lines)
  - `crates/desktop-server/src/idea_system_prompt.rs` (new, `<= 600` lines)
  - `crates/desktop-server/assets/idea_chat_ui.js` (new, `<= 600` lines)
  - `crates/desktop-server/tests/idea_chat.rs` (new, `<= 600` lines)
  - `crates/desktop-server/tests/idea_chat_ui.test.mjs` (new, `<= 600` lines)
- **X16 (disjoint from X12, X13, X15, X17, X18):**
  - `crates/research-tools/src/idea_net.rs` (new, `<= 600` lines; sibling `idea_net_tests.rs` if needed)
  - `crates/desktop-server/src/routes_idea_net.rs` (new, `<= 600` lines)
  - `crates/desktop-server/src/idea.rs` (owned sequentially after X3 — keep `<= 600` lines; split helper into `idea_round2.rs` if approaching 550 lines)
  - `crates/desktop-server/assets/idea_search.js` (owned sequentially after X9 — keep `<= 600` lines; split citation-network UI into `idea_net_ui.js` if approaching 550 lines)
  - `crates/desktop-server/tests/idea_citation_net.rs` (new, `<= 600` lines)

## 6. Shared registration seams and "Do not edit" list

- **Registration seams (1–3 lines each):**
  - `crates/research-tools/src/lib.rs`: `pub mod idea_net;`.
  - `crates/desktop-server/src/lib.rs`: `pub mod idea_chat;`, `pub mod idea_system_prompt;`, `pub mod routes_idea_net;`.
  - `crates/desktop-server/src/api_dispatch.rs` (from F5): 1-line arm delegating `/api/sessions/{id}/idea/more` & `/api/sessions/{id}/idea/cited-since` to `routes_idea_net.rs` behind `AuthGuard::authorize_api`.
  - `crates/desktop-server/src/turn_runner.rs` (463 lines, strictly after `F5` & Solve `S12`): route `Mode::Explore` Conversation turns from `POST /api/sessions/{id}/messages` to `idea_chat.rs`, and guard `investigation::{prepare, finish, turn_context}` & `base_system_prompt` (`lines 71-81, 98, 198, 286, 394`) strictly to `Mode::Investigate`.
  - `crates/desktop-server/src/assets.rs`: register `/assets/idea_chat_ui.js`.
  - `crates/desktop-server/assets/idea.js` (coordinate after `X12`/`X13` if concurrent): mount `idea_chat_ui.js` in the Conversation tab slot.
- **Do not edit:** `crates/desktop-server/src/routes_idea.rs` (owned by X12/X13), `tool_profile.rs`, `tests/golden/tool_defs.txt`, `tests/golden/tool_profiles.txt`, `crates/agent/src/resume.rs`, `paper_passages.rs` (605), `investigation_cites.rs` (keep Investigation untouched), `idea_exchanges.rs` (owned by X18 next), `library.js`, `cli_turn.rs` (626), `Cargo.lock`.

## 7. Tests through the entry point

Run `cargo test -p research-tools --lib idea_net --test tool_def_golden --test tool_profile_golden`, `cargo test -p desktop-server --test idea_chat --test idea_citation_net --test eval_set_r`, `cargo test -p agent --test resume_loop`, `node --test crates/desktop-server/tests/*.test.mjs`, `cargo clippy --workspace --all-targets -- -D warnings`, and `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`.

Required tests (scripted transport + loopback mock only, zero live network/model calls, zero third-party text):
- `idea_chat.rs` & `idea_chat_ui.test.mjs`:
  - End-to-end HTTP `POST /api/sessions/{id}/messages` -> `turn_runner.rs` (`Mode::Explore`) -> `idea_chat.rs` starts under `tool_profile: "explore/1"`, executes `search_paper_passages` and `read_paper_lines` across TWO ingested papers, never runs Investigation hooks (`prepare`/`finish`/`base_system_prompt`), and resumes cleanly on turn 2 via F1 fail-closed resume (`resume_profile`).
  - Multi-paper line-citation check marks lines returned by `read_paper_lines`/`search_paper_passages` as matched and flags unread lines or lines from the wrong paper (`file_sha256` / `source_id` mismatch) as unread (Rule C2); `idea_chat_ui.test.mjs` verifies matched chips + unread warning badges, XSS escaping, and zero `"verified"` (Rule S8).
  - `idea_search` / `idea_fetch_papers` invoked by the model in Conversation turns go through real `UiApprover` (Rule P5) and `explore/1` exposes zero `ExecuteCode`/`WriteWorkspace` tools (Rule P7).
- `idea_net.rs` & `idea_citation_net.rs`:
  - Deterministic in-network ranking invariant to input edge ordering; `"cited by your core papers but unread"` excludes already-read papers and origin paper.
  - HTTP `POST /api/sessions/{id}/idea/cited-since` (`routes_idea_net.rs`, behind `AuthGuard::authorize_api`) costs 0 model calls and routes through `Router::try_call("host:idea:...", "idea_search", ...)` (Rule P5) and F3 `net_rate` (Rule P6).
  - Thorough Round 2 respects hard caps (`36` queries / `12` papers / `55` model calls -> `stopped_at_cap`, Rule P3) and updates F12 coverage `citation_expansion`.

## 8. Document conflicts and resolutions

1. **Tool profile & resume drift in Conversation turns:** Earlier `explore.md` planned `idea_legacy_defs.rs` and custom Explore router checks. **Resolution:** `00-overview.md` 2.2 (F1) moves all schema history into `explore/1`'s `earlier_defs` inside `tool_profile.rs`; `X15` never edits `resume.rs` and creates no `idea_legacy_defs.rs`.
2. **Line crediting across papers:** `investigation_cites.rs` reads `tool.result` directly for one pinned `primary_path`. **Resolution:** `00-overview.md` 2.5 (F4) mandates crediting `read_paper_lines`, `search_paper_passages`, and `context.supplied` through `research_tools::lines_sent::lines_sent_with_ledger` across all ingested papers in the session.

## 9. Out of scope and hand-back

- **Out of scope:** Modifying `explore/1` tool schemas (`X7`/`X9`), CLI-subscription exchange execution (`X18`), full Library pane (`X17`), OpenAlex/TheoremSearch citation endpoints (`X19`), F15 write-up (`writeup_idea.rs`, Group 5).
- **Hand-back:** Write `docs/design/modes-2026-11/reports/exp-x15-x16-chat-cite.md`, commit with `Co-Authored-By`, and push to `gemini/modes/exp-x15-x16-chat-cite`.
