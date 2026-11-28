# Brief exp-x15-chat — Explore Conversation Turns & Multi-Paper Line Citations (Package X15)

**Split note:** Split out from `draft-exp-x15-x16-chat-cite.md` into an independent, file-disjoint lane from **`exp-x16-cite`**, **`exp-x17-library`**, and **`exp-x18-cli`**.

## 1. Package and governing design sections

- **Package:** `X15` (Conversation turns on `POST /api/sessions/{id}/messages`, `explore/1` chat tools via `build_idea_router`, multi-paper line-citation check via `lines_sent_with_ledger`, enabling `DESKTOP_MODE_EXPLORE_ENABLED`, and chat citation UI).
- **Governing design:**
  - `docs/design/modes-2026-11/00-overview.md`: sections 2.2 (frozen 4-tool `explore/1`), 2.4 (F3 network gate & approval rule), 2.5 (F4 `lines_sent` & `context.supplied`), 3.1 row 12, 3.2 row 20 (`"matched"`, never `"verified"`), 3.3, and 8.2 decision 1.
  - `docs/design/modes-2026-11/explore.md`: sections 3 (`Afterwards`), 5.2 (`explore/1`: `idea_search`, `idea_fetch_papers`, `search_paper_passages`, `read_paper_lines`), 6 (Rules C1, C2, C7, P3, P5, P6, P7, S2, S8), 8 (items 3–4: Conversation turns record `tool_profile: "explore/1"`), and 9 (row X15).
  - `docs/design/modes-2026-11/agents/RULES.md` (includes scoped `exp-x15-chat` seam authorization).

## 2. Start conditions and verified interfaces

- **Required merge on `gemini/work/fixed` before starting:** **`exp-x9-x10-x11-read-verdict`** (brings `X7` `explore/1` & `build_idea_router`, `X9` real `idea_fetch_papers`, `X10`/`X11` evidence & map).
- **Already merged on `gemini/work/fixed`:**
  - F1 (`tool_profile.rs`, `resume_profile.rs`), F4 (`lines_sent.rs`), F5 (`session_mode.rs`, `routes_session.rs`, `routers.rs`), F9 (`run_budget.rs`), `investigation_cites.rs` (single-paper reference pattern — do NOT break Investigate!), Solve `S12` (already gated `Mode::Solve` in `turn_runner.rs`).
- **CRITICAL 600-line & safety checks on `origin/gemini/work/fixed` before editing:**
  1. `crates/desktop-server/src/turn_runner.rs` is **580 lines** after `S12`! Do NOT push it over 600 lines. If gating Investigate helpers and dispatching `Mode::Explore` would approach 595 lines, extract a helper (e.g. `run_explore_turn` in `idea_chat.rs` or split Research/Investigate turn setup into `turn_runner_research.rs`) in a clean commit.
  2. Gate `investigation::prepare`, `investigation_finish::finish_with_cancel`, `investigation::turn_context`, and `investigation_system_prompt::base_system_prompt` in `turn_runner.rs` strictly away from `Mode::Explore` (just as `S12` excluded `Mode::Solve`) so Explore Conversation turns NEVER invoke Investigate hooks!
  3. **`explore/1` is already frozen** in `tool_profile.rs` & `tests/golden/tool_profiles.txt`, and wired with the real `idea_fetch_papers` handler in `X9`: **do NOT edit `tool_profile.rs`, `tool_defs.txt`, `tool_profiles.txt`, or `resume.rs`.**
  4. Check `crates/desktop-server/src/session_mode.rs`, `tests/mode_shell.rs`, and `tests/handoff_routes.rs`: flip `DESKTOP_MODE_EXPLORE_ENABLED = true` only when Explore `explore/1` turns work end-to-end through `POST /api/sessions/{id}/messages`, and update only the Explore `"not built yet"` assertions in `mode_shell.rs` / `handoff_routes.rs` honestly (disclosing in report & commit). Leave `"not built yet"` on bare `POST /api/sessions { mode: "explore" }` without `text` intact if `X3`'s `routes_session.rs` contract still expects `text`, or verify existing tests!

## 3. Smallest implementation checkpoints

1. **Checkpoint 1 — `idea_system_prompt.rs` & `idea_chat.rs`:**
   - `crates/desktop-server/src/idea_system_prompt.rs` (new, `<= 600`L): builds Explore Conversation system prompt injecting selected map node / idea context wrapped as quoted untrusted data (Rule C7), instructing the model to cite paper + line ranges ONLY from lines returned by `search_paper_passages` / `read_paper_lines`, forbidding novelty/verification prose (Rule S8).
   - `crates/desktop-server/src/idea_chat.rs` (new, `<= 600`L):
     - Executes Conversation turns entered via `POST /api/sessions/{id}/messages` -> `turn_runner.rs` (`Mode::Explore`) on `routers::build_idea_router` (or `build_desktop_router_for_profile(..., PROFILE_EXPLORE_V1)`), enforcing `research_policy()` (`0` `ExecuteCode`/`WriteWorkspace` tools, Rule P7), F9 `run_budget` & `CancelToken`, and `session.start` / `session.resume` `tool_profile: "explore/1"` via F1 (`resume_profile`) so subsequent turns resume fail-closed.
     - Multi-paper line-citation checker: parses `[paper_key: Lstart-end]` / `[source_id: Lstart-end]` across multiple ingested papers and credits lines strictly via F4 `research_tools::lines_sent::lines_sent_with_ledger` across every `(source_id, path, file_sha256)` sent to the conversation caller; flags unread lines or wrong-paper lines as unread (never matched).
     - Preserves `"matched"` wording and zero `"verified"` on screen (Rule S8).
2. **Checkpoint 2 — `turn_runner.rs` / `session_mode.rs` wiring + `idea_chat_ui.js`:**
   - Wire `Mode::Explore` dispatch & Investigate-hook exclusion cleanly in `turn_runner.rs` (`<= 600`L), flip `DESKTOP_MODE_EXPLORE_ENABLED = true` in `session_mode.rs`, and update `mode_shell.rs` / `handoff_routes.rs` Explore-enabled assertions honestly.
   - `crates/desktop-server/assets/idea_chat_ui.js` (new, `<= 600`L): renders multi-paper matched line-citation chips (integrating with F6 `quote_chip.js`) and explicit unread/unmatched citation warning badges, zero raw `innerHTML`, zero `"verified"`.
   - Register `/assets/idea_chat_ui.js` in `crates/desktop-server/src/assets.rs`; if `idea.js` (`<= 600`L) has room, add the minimal mount call in Conversation tab, OR export a standalone mounter documented in `Merge notes` if `idea.js` is at `> 590`L (never exceed `600`L or conflict needlessly with `exp-x12-x13-map-handoff`).

## 4. Owned files & Do-Not-Edit (`<= 600` lines each)

- **New:** `crates/desktop-server/src/{idea_chat,idea_system_prompt}.rs`, `crates/desktop-server/assets/idea_chat_ui.js`, `crates/desktop-server/tests/idea_chat.rs`, `crates/desktop-server/tests/idea_chat_ui.test.mjs`.
- **Small edits:** `desktop-server/src/{lib.rs,turn_runner.rs,session_mode.rs,routers.rs,assets.rs}`, `desktop-server/tests/{mode_shell,handoff_routes}.rs`, minimally `assets/idea.js` only if `<= 590`L after mount.
- **Do NOT edit:** `idea_net.rs`, `routes_idea_net.rs`, `idea.rs`, `idea_search.js`, `routes_idea.rs`, `idea_exchanges*.rs`, `cli_turn*.rs`, `core_tools.rs`, `mcp-server/*`, `export/*`, `tool_profile.rs`, `resume.rs`, `paper_passages.rs` (605L), `investigation_cites.rs`, `routes_tools.rs` (610L), `http_srv.rs` (702L), `routes_session.rs` (616L), `Cargo.lock`, `tests/golden/{tool_defs,tool_profiles}.txt`.

## 5. Required tests & hand-back

- Cover in `idea_chat.rs` & `idea_chat_ui.test.mjs` (scripted transport + loopback mock only): HTTP `POST /api/sessions/{id}/messages` -> `turn_runner.rs` (`Mode::Explore`) -> `idea_chat.rs` starting under `tool_profile: "explore/1"`, calling `search_paper_passages` + `read_paper_lines` across 2 ingested papers, zero Investigation hooks run, turn-2 fail-closed F1 resume, multi-paper line crediting + unread/wrong-paper flagging, `UiApprover` gating on `idea_search`/`idea_fetch_papers`, Rule P7 (`0` code/write tools), XSS escaping, and Rule S8 (zero `"verified"`).
- Verify before push as non-root (`RE_SEARCH_PYTHON` unset): `check_file_length.py`, `node --check` + `node --test`, `cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or`, `cargo test --workspace --locked --no-fail-fast`.
- Write `docs/design/modes-2026-11/reports/exp-x15-chat.md` and push to `gemini/modes/exp-x15-chat`.
