# Brief exp-x17-x18-lib-cli — Library pane & export (X17) and CLI-subscription exchange routes (X18)

## 1. Packages and governing design sections

- **Packages:** `X17` (Library pane UI & JSON/BibTeX export with coverage) and `X18` (CLI-subscription routes for Explore exchanges). `X17` and `X18` own completely disjoint files and may run sequentially (push after each) or independently once their respective gates clear.
- **Governing design:**
  - `docs/design/modes-2026-11/00-overview.md`: sections 2.7 (F10 global library), 2.11 (F13a minimal library list & sources panel — distinct from X17's full Library pane!), 2.13 (F15 write-up — OUT OF SCOPE for X17; owned by Group 5), 3.2 row 20 (`"verified"` forbidden in Explore views/exports), 5.1 (Library entry on home screen), and 7.3 (wording property test).
  - `docs/design/modes-2026-11/explore.md`: sections 4.3 (Library tables), 5.1 (CLI JSON-block exchange path using the exact same validators), 5.4 (F10 `/api/library` routes), 6 (rules V7, S5, S8), 9 (rows X17, X18), 10 (free core vs CLI-subscription access), 12 (risk 10), and 13 (decisions 10, 12).
  - `docs/design/modes-2026-11/agents/RULES.md`.

## 2. Start conditions and mandatory cross-group gates

- **Before X17:**
  - `F10` (merged in base: `library.rs`, `routes_library.rs`), `F5` (`f5-f7-mode-shell`: wires `/api/library` and `api_dispatch.rs`), `X11` (`exp-x9-x10-x11-read-verdict`: needed for `export/src/idea.rs` coverage & verdicts), and **`F13a`** (`lib-f13a-f16a-f15`: inspect its `assets/library_sources.js` seam so X17's `library.js` composes with F13a's sources panel rather than duplicating or overwriting it!).
- **Before X18 (CRITICAL CROSS-GROUP SERIALIZATION GATE WITH SOLVE `S15`):**
  - Checkpoint 2a (Explore pipeline exchanges on CLI routes) requires `X6` (`agent/src/exchange.rs`, `idea_exchanges.rs`) and `X10` (`idea_read.rs`).
  - Checkpoint 2b (Explore Conversation turns on CLI routes) additionally requires `X15` (`idea_chat.rs`, `idea_system_prompt.rs`).
  - **`crates/desktop-server/src/cli_turn.rs` (626 lines — OVER 600!), `crates/cli-route/src/core_tools.rs`, and `crates/mcp-server/src/main.rs` are touched by BOTH `X18` and Solve `S15` (Group 2)!**
    1. **NEVER run `X18` concurrently with Solve `S15` on `cli_turn.rs`, `core_tools.rs`, or `mcp-server/src/main.rs`.**
    2. Before editing `cli_turn.rs`, merge `origin/gemini/work/fixed` and run `wc -l crates/desktop-server/src/cli_turn.rs`.
    3. If `S15` already split `cli_turn.rs` under 560 lines (planned filenames `cli_turn_setup.rs` + `cli_turn_tests.rs`; check the actual merged filenames!), integrate cleanly into those split modules and reuse S15's mode-neutral `CORE_TOOL_PROFILE` wiring.
    4. If `cli_turn.rs` is still `> 600` lines, the **FIRST commit of X18 MUST be a pure zero-behaviour-change split** extracting `crates/desktop-server/src/cli_turn_setup.rs` and `crates/desktop-server/src/cli_turn_tests.rs` (matching Solve S15's split convention) until every file is `<= 560` lines, passing all existing `cli_turn` / `cli_route` tests and `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`, before adding any X18 code!

## 3. Base-commit verified interfaces vs future-merge interfaces

### Verified on base commit `832eab5bf`
- `crates/desktop-server/src/routes_library.rs` (287 lines): `GET /api/library` (search by `q`, `title`, `author`, `year`, `key`, `status`, `role`, `limit`, `offset`), `GET /api/library/{key}`, `POST/DELETE /api/library/{key}/mark`, `POST /api/library/{key}/attach`, `DELETE /api/library/{key}`, `POST /api/library/rebuild`, `GET /api/library/marks/backup`, `POST /api/library/marks/restore`. **Important base note from `reports/f10-library.md`:** `http_srv.rs:645` decodes `'+'` to `' '` in URL paths, so `library.js` MUST wrap every `PaperKey` with `encodeURIComponent(key)`!
- `crates/desktop-server/assets/api.js` (465 lines): `saveProjectSource(id, payload)` already exists (line 123) for `"Save to project"` — DO NOT touch `crates/desktop-server/src/projects.rs` (806 lines!) or `crates/desktop-server/src/routes_tools.rs` (610 lines!).
- `crates/export/src/lib.rs` (132 lines): `ExportFormat`, `export_session_to_path`, `export_session_bytes`.
- `crates/desktop-server/src/cli_turn.rs` (**626 lines — OVER 600!**): `CliRouteKind::{Claude, Antigravity}`, `session_route`, `run_turn`; lines `407-419` currently inject a hardcoded Research prompt block mentioning `search_formal_library` / `sympy_eval`.
- `crates/desktop-server/src/cli_relay.rs` (90 lines): `UiRelay`, `PeerWatch`.
- `crates/cli-route/src/core_tools.rs` (188 lines): `register_core_tools_for_profile`, `core_tool_set_for_profile`, `core_tool_ids_for`, `CORE_TOOL_PROFILE`.
- `crates/mcp-server/src/main.rs` (317 lines): line `184` gates profile lookup on `use_research` — must accept `use_research || CORE_TOOL_PROFILE` for non-Research profiles (`explore/1`, `solve/1`).
- `crates/desktop-server/src/antigravity.rs` (285 lines): `require_acknowledged` consent gate (MUST remain enforced on Antigravity CLI exchanges!).
- `crates/desktop-server/src/assets.rs` (365 lines).

### Future-merge interfaces to verify before coding
- **`F13a` (`assets/library_sources.js`, `routes_sources.rs`):** verify exact exported mount function and routes of F13a's minimal library list / sources panel so X17's full `library.js` mounts it cleanly.
- **`F5`:** confirm `/api/library` and `api_dispatch.rs` registration.
- **`X11`/`F12`:** `idea_verdict::compute`, `search_coverage::compute`, `prior_work_wording` (used by `export/src/idea.rs` for Rule V7 recomputation on export).
- **`S15` (if merged before X18):** verify actual post-split filenames of `cli_turn.rs`, whether `cli_turn.rs:407-419` already strips the Research `search_formal_library`/`sympy_eval` prompt block for non-Research modes, and whether `core_tools.rs` + `mcp-server/src/main.rs:184` already wire mode-neutral `CORE_TOOL_PROFILE`.
- **`X15` (before Checkpoint 2b):** `idea_chat.rs` multi-paper line-citation check and `idea_system_prompt.rs`.

## 4. Smallest implementation checkpoints

1. **Checkpoint 1 — X17 (Library pane UI & JSON/BibTeX export with coverage):**
   - `crates/desktop-server/assets/library.js` (new): full Library pane UI over F10's `/api/library` routes — search bar (`q`, `title`, `author`, `year`, `status`, `role`), paper list & detail drawer (showing `source_item` provenance, `owned` badge, matched findings, sessions where seen), mark controls (`unread | skimmed | key | irrelevant`, tags, notes), `"Attach PDF"` upload action, `"Rebuild index"` button, mark backup download & restore, `"Save to project"` via existing `saveProjectSource`, and embedded F13a `library_sources.js` sources panel. Always `encodeURIComponent(key)`. Zero raw `innerHTML` with untrusted paper text; zero `"verified"`.
   - `crates/export/src/idea.rs` (new): deterministic JSON and BibTeX export of an Explore session:
     - Recomputes F12 `Coverage` and `Verdict` from ledger events on export and verifies agreement with the stored `idea.verdict` event (Rule V7; flags mismatch fail-closed).
     - JSON export contains idea statement, sharpened idea, candidates, papers read (`text_basis`, `origin`), matched quotes (clipped `<= 300` chars), concept map, verdicts, directions, and full F12 `Coverage` record.
     - BibTeX export uses the user's own citation keys from F10/F13 `source_item.citekey` where present (falling back to deterministic keys), plus a header comment/metadata block citing the searched scope without inventing fields.
     - Enforces Rule S8: zero `"verified"`, `"new"`, or `"novel"` anywhere in exported JSON/BibTeX wording fields.
   - `crates/desktop-server/src/routes_idea_export.rs` (new, dedicated dispatcher — keeps X17 100% disjoint from `X12`/`X13` on `routes_idea.rs` and `X16` on `routes_idea_net.rs`, and avoids 610-line `routes_tools.rs`!): serves `GET /api/sessions/{id}/idea/export?format=json|bib` behind `AuthGuard::authorize_api`, registered via a single 1-line arm in `api_dispatch.rs`.
   - Commit and push X17.
2. **Checkpoint 2a — X18 Explore pipeline exchanges on CLI-subscription routes:**
   - First commit (if `cli_turn.rs > 600` lines): pure zero-behaviour-change split of `cli_turn.rs` into `cli_turn.rs` + `cli_turn_setup.rs` + `cli_turn_tests.rs` (all `<= 560` lines), verified by existing tests + `check_file_length.py`.
   - Implement plain prompt-in, JSON-block-out exchange runner over `CliRouteKind::Claude` (`claude-cli`) and `CliRouteKind::Antigravity` (`antigravity`, still enforcing `antigravity::require_acknowledged` — NEVER weaken consent or sandbox rules!), keeping `cli_turn.rs` and `idea_exchanges.rs` under 600 lines (extract `crates/desktop-server/src/idea_cli_exchange.rs` if `idea_exchanges.rs` approaches 520 lines).
   - Pipe every parsed JSON block through the **exact same Rust validators/recorders** (`record_idea_brief`, `record_idea_triage`, `record_paper_evidence`, `record_idea_map`, `record_idea_directions`), with the same `<= 2` repair-attempt loop, F9 budget checks before every CLI invocation (Rule P3), `idea.exchange` audit events (recording route provenance), and `UiRelay` (`cli_relay.rs`) approval handling where applicable.
   - Once X18 lands, CLI-subscription Explore sessions run exchanges instead of refusing `claude-cli`/`antigravity` with X6's `"not supported on CLI route yet"` message.
   - Commit and push Checkpoint 2a.
3. **Checkpoint 2b — X18 Explore Conversation turns on CLI routes (gated on `X15`, serialized with `S15`):**
   - Wire mode-neutral `CORE_TOOL_PROFILE` in `crates/cli-route/src/core_tools.rs` (`core_tool_ids_for`, `core_tool_set_for_profile`) and `crates/mcp-server/src/main.rs:184` (`use_research || CORE_TOOL_PROFILE`) — reusing S15's changes if S15 merged first — and strip the hardcoded Research `search_formal_library`/`sympy_eval` prompt block at `cli_turn.rs:407-419` for non-Research modes.
   - Serve `explore/1` MCP tools via `core.exe` for Explore Conversation turns on `claude-cli` / `antigravity` and run X15's `idea_chat` multi-paper line-citation check (F4 `lines_sent_with_ledger`) on the returned turn.

## 5. Owned files

- **X17 (100% disjoint from X12, X13, X15, X16, X18):**
  - `crates/desktop-server/assets/library.js` (new, `<= 600` lines)
  - `crates/export/src/idea.rs` (new, `<= 600` lines; sibling `idea_tests.rs` if needed)
  - `crates/desktop-server/src/routes_idea_export.rs` (new, `<= 600` lines)
  - `crates/desktop-server/tests/library_ui.test.mjs` (new, `<= 600` lines)
  - `crates/export/tests/idea_export.rs` (new, `<= 600` lines)
- **X18 (disjoint from X15, X16, X17; serialized with Solve `S15` on `cli_turn.rs`, `core_tools.rs`, `mcp-server/src/main.rs`):**
  - `crates/desktop-server/src/cli_turn.rs` (626 lines on base — split first if still `> 600` lines!)
  - `crates/desktop-server/src/cli_turn_setup.rs` & `cli_turn_tests.rs` (new if X18 splits `cli_turn.rs` before S15; otherwise integrate into S15's merged split files)
  - `crates/desktop-server/src/cli_relay.rs` (90 lines)
  - `crates/desktop-server/src/idea_exchanges.rs` (and new `idea_cli_exchange.rs` if needed to stay `<= 600` lines)
  - `crates/cli-route/src/core_tools.rs` (188 lines — Checkpoint 2b, if not already updated by S15)
  - `crates/mcp-server/src/main.rs` (317 lines — Checkpoint 2b line 184, if not already updated by S15)
  - `crates/desktop-server/tests/idea_cli_exchange.rs` (new, `<= 600` lines)

## 6. Shared registration seams and "Do not edit" list

- **Registration seams (1–3 lines each):**
  - `crates/export/src/lib.rs` (132 lines): `pub mod idea;`.
  - `crates/desktop-server/src/assets.rs` (365 lines): register `/assets/library.js`.
  - `crates/desktop-server/src/api_dispatch.rs` (from F5): 1-line arm delegating `/api/sessions/{id}/idea/export` to `routes_idea_export.rs` behind `AuthGuard::authorize_api`.
  - `crates/desktop-server/src/lib.rs`: `pub mod routes_idea_export;`, plus `pub mod idea_cli_exchange;` / `mod cli_turn_setup;` if added.
- **Do not edit:** `crates/desktop-server/src/routes_idea.rs` (owned by X12/X13), `crates/desktop-server/src/routes_tools.rs` (610 lines!), `export/src/writeup_idea.rs` (Group 5 owns F15 Explore write-up!), `export/src/latex.rs` (771 lines!), `crates/desktop-server/src/projects.rs` (806 lines!), `crates/desktop-server/assets/library_sources.js` (owned by F13a — compose with it, do not overwrite it), `crates/desktop-server/assets/app.js` (932), `tool_profile.rs`, `tests/golden/tool_defs.txt`, `tests/golden/tool_profiles.txt`, `Cargo.lock`.

## 7. Tests through the entry point

Run `cargo test -p export --test idea_export`, `cargo test -p desktop-server --test idea_cli_exchange --test library_routes`, `cargo test -p cli-route`, `cargo test -p mcp-server`, `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done`, `node --test crates/desktop-server/tests/*.test.mjs`, `cargo clippy --workspace --all-targets -- -D warnings`, and `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`.

Required tests (fake/mock CLI subprocess & synthetic ledgers only — zero real vendor CLI, zero live network, zero third-party text):
- `idea_export.rs` & HTTP `routes_idea_export`: `GET /api/sessions/{id}/idea/export?format=json|bib` (enforcing `AuthGuard::authorize_api`) recomputes F12 coverage & verdicts from ledger events (Rule V7), rejects tampered `idea.verdict` mismatch, prefers `source_item.citekey` when present, clips quotes at 300 chars, survives `library.db` deletion (Rule S5), and passes Rule S8 forbidden-word check (zero `"verified"`/`"new"`/`"novel"`).
- `library_ui.test.mjs`: search/filter, mark update, `"Save to project"`, URL encoding of slash & `+` in `PaperKey`, F13a sources panel mount, zero `"verified"`, XSS escaping.
- `idea_cli_exchange.rs`: fake Claude CLI & Antigravity CLI returning JSON blocks (valid, malformed -> repair up to 2 attempts, rejected fabricated quote -> fault returned to CLI, budget cap -> `stopped_at_cap`), verifying `antigravity::require_acknowledged` refuses unacknowledged Antigravity exchanges, `idea.exchange` events audit every attempt, and (Checkpoint 2b) `explore/1` MCP tools serve over `core.exe` without the Research `search_formal_library`/`sympy_eval` prompt block and run `idea_chat` multi-paper line-citation checks.

## 8. Document conflicts and resolutions

1. **Markdown export in X17 vs F15 write-up:** Earlier `explore.md` included Markdown export in X17. **Resolution:** `00-overview.md` 0.1 & 2.13 (reconciled in `explore.md` 9 row X17) replace X17's Markdown export with F15's cited summary (`export/src/writeup_idea.rs`, owned by Group 5). X17 delivers ONLY JSON and BibTeX export with coverage via dedicated `routes_idea_export.rs`.
2. **Library pane in X17 vs F13a minimal library list:** `00-overview.md` 6 puts F13a's minimal library list and sources panel in week 6 (`assets/library_sources.js`), while `explore.md` 9 row X17 puts the full Library pane (`assets/library.js`) after X11. **Resolution:** X17 builds `library.js` to compose with F13a's `library_sources.js` rather than duplicating source-management UI.
3. **`cli_turn.rs` > 600 lines, `core_tools.rs`, `mcp-server/src/main.rs` & S15 overlap:** Base `cli_turn.rs` is 626 lines (with hardcoded Research prompt block at lines 407–419), `mcp-server/src/main.rs:184` gates on `use_research`, and all three are shared between Solve `S15` and Explore `X18`. **Resolution:** `RULES.md` 600-line rule mandates checking post-S15 filenames and splitting into `cli_turn_setup.rs` + `cli_turn_tests.rs` in a zero-behaviour-change commit first if still `> 600`, with strict serialization against `S15`.

## 9. Out of scope and hand-back

- **Out of scope:** F15 cited summary (`export/src/writeup_idea.rs`, Group 5), F13b/c BibTeX/Zotero/send-back (Group 4), Solve S15 CLI turns (Group 2), live vendor CLI runs.
- **Hand-back:** Write `docs/design/modes-2026-11/reports/exp-x17-x18-lib-cli.md` (recording exact F13a seam, `cli_turn.rs` split status vs `S15`, and Checkpoint 2a/2b status), commit with `Co-Authored-By`, and push after each checkpoint to `gemini/modes/exp-x17-x18-lib-cli`.
