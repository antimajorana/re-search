# Brief exp-x16-cite — Citation Expansion & Thorough Second Round (Package X16)

**Split note:** Split out from `draft-exp-x15-x16-chat-cite.md` into an independent, file-disjoint lane from **`exp-x15-chat`**, **`exp-x12-x13-map-handoff`**, **`exp-x17-library`**, and **`exp-x18-cli`**.

## 1. Package and governing design sections

- **Package:** `X16` (Citation-network builder in Rust via F11 `cites`/`cited_by`, `"Who cited this since"`, `"Cited by your core papers but unread"`, and Thorough Round 2 self-check under preset ceilings).
- **Governing design:**
  - `docs/design/modes-2026-11/00-overview.md`: sections 2.4 (F3 gate & rate limit), 2.8 (F11 `cites` / `cited_by`), 3.1 row 12, 3.2 row 20, and 8.2 decision 1.
  - `docs/design/modes-2026-11/explore.md`: sections 3 (`Thorough` preset: `36` queries, `80` screened, `12` papers, `<= 4` calls/paper, hard cap `55` model calls), 4.1 (`SearchPlan` `cites`/`cited_by`, `Candidate`), 4.2 (`idea.citation_net`), 6 (Rules P3, P5, P6, S2, S8), 9 (row X16), and 11 (LocalCitationNetwork & OpenScholar ideas, re-coded in Rust).
  - `docs/design/modes-2026-11/agents/RULES.md`.

## 2. Start conditions and verified interfaces

- **Required merge on `gemini/work/fixed` before starting:** **`exp-x9-x10-x11-read-verdict`** (brings `X7` `idea_search` & `build_idea_router`, `X8` triage, `X9–X11` read/map/verdict).
- **Already merged on `gemini/work/fixed`:**
  - F3 (`net_rate.rs`), F5 (`api_dispatch.rs`), F9 (`run_budget.rs`), F11 (`lit_providers.rs` `cites`/`cited_by`), F12 (`search_coverage.rs`), X3/X4 (`idea.rs`, `idea_search.js`), X19 (`lit_openalex.rs` — supports `cites` & `cited_by` alongside INSPIRE/Semantic Scholar!).
- **Disjointness & 600-line rules:**
  - **Dedicated dispatcher `routes_idea_net.rs`:** expose `/api/sessions/{id}/idea/more` and `/api/sessions/{id}/idea/cited-since` (plus Round 2 endpoint if separate) behind `AuthGuard::authorize_api` via `api_dispatch.rs` — **do NOT edit `routes_idea.rs`** (owned by `X12/X13`).
  - `crates/desktop-server/src/idea.rs` (~`530`L after X8) & `crates/desktop-server/assets/idea_search.js` (~`466`L+ after X8–X9): extract new Round 2 Rust logic into `crates/desktop-server/src/idea_round2.rs` (`<= 600`L) and citation-network UI into `crates/desktop-server/assets/idea_net_ui.js` (`<= 600`L, registered in `assets.rs`) so existing files stay well below `600` lines and edits stay minimal!
  - **Do NOT edit `idea.js` or `turn_runner.rs`** (owned by `X12/X13` / `X15`).

## 3. Smallest implementation checkpoints

1. **Checkpoint 1 — `idea_net.rs` & HTTP routes (`routes_idea_net.rs`):**
   - `crates/research-tools/src/idea_net.rs` (new, `<= 600`L; sibling `_tests.rs` if needed):
     - Pure deterministic citation-network builder over F11 `cites` / `cited_by` relations of seed & core/read papers, keyed by F2 `PaperKey` (`LocalCitationNetwork` pattern in Rust).
     - Deterministic in-network rank (inbound + outbound citation links within network, invariant to input edge order).
     - Identifies `"cited by your core papers but unread"` candidates, excluding already-read papers and any `handoff.packet.from.paper` origin paper (Rule V9 parity).
   - `crates/desktop-server/src/routes_idea_net.rs` (new, `<= 600`L) + 1–2 line arm in `api_dispatch.rs`:
     - `POST /api/sessions/{id}/idea/cited-since`: `"origin": "app"`, **0 model calls**, calls `Router::try_call("host:idea:<session>", "idea_search", ...)` with `relation: cited_by` per Rule P5 & F3 `net_rate` (Rule P6), appends `idea.citation_net`.
     - `POST /api/sessions/{id}/idea/more`: node `"Search more about this node"` & Thorough second-round trigger.
2. **Checkpoint 2 — Thorough Round 2 (`idea_round2.rs` / `idea.rs`) & UI (`idea_net_ui.js` / `idea_search.js`):**
   - `crates/desktop-server/src/idea_round2.rs` (new, `<= 600`L) + minimal hook in `idea.rs` (`<= 600`L):
     - Lists thin/under-supported statements on the active `idea.map` (OpenScholar self-check pattern), builds targeted retrieval queries, executes `idea_search` via `Router::try_call` strictly within `Thorough` hard ceilings (`36` queries, `80` screened, `12` papers, `<= 4` calls/paper, `55` model calls — stops with `stopped_at_cap` on cap, Rule P3).
     - Appends `idea.citation_net` and updates F12 `search_coverage` `citation_expansion { seeds, hops }` + `idea_view_coverage.rs` / `idea_view.rs` projection (`<= 600`L).
   - `crates/desktop-server/assets/idea_net_ui.js` (new, `<= 600`L, registered in `assets.rs`) + minimal hook in `idea_search.js` (`<= 600`L): renders `"Cited by your core papers but unread"`, `"Who cited this since"` button (with `"cited since" != "answered"` note on open questions, `explore.md` 12 risk 12), and Round 2 / Thorough progress. Zero raw `innerHTML`, zero `"verified"` (Rule S8).

## 4. Owned files & Do-Not-Edit (`<= 600` lines each)

- **New:** `crates/research-tools/src/idea_net.rs` (+ `_tests.rs`), `crates/desktop-server/src/{routes_idea_net,idea_round2}.rs`, `crates/desktop-server/assets/idea_net_ui.js`, `crates/desktop-server/tests/{idea_citation_net.rs,idea_net_ui.test.mjs}`.
- **Small edits:** `crates/research-tools/src/lib.rs`, `crates/desktop-server/src/{lib.rs,api_dispatch.rs,assets.rs,idea.rs,idea_view_coverage.rs}`, `crates/desktop-server/assets/idea_search.js`.
- **Do NOT edit:** `routes_idea.rs`, `idea.js`, `turn_runner.rs`, `idea_chat.rs`, `idea_exchanges*.rs`, `cli_turn*.rs`, `export/*`, `tool_profile.rs`, `resume.rs`, `paper_passages.rs` (605L), `http_srv.rs` (702L), `Cargo.lock`, `tests/golden/{tool_defs,tool_profiles}.txt`.

## 5. Required tests & hand-back

- Cover deterministic ranking invariant to edge order, origin/read exclusion on unread citations, HTTP `/idea/cited-since` (`0` model calls, `AuthGuard` enforced, goes through `Router::try_call("host:idea:...", "idea_search", ...)` + F3 `NetRateGate`), Round 2 `Thorough` caps (`36`/`12`/`55` -> `stopped_at_cap`) & F12 `citation_expansion` update, and JS XSS + `"cited since" != "answered"` UI checks (`0` `"verified"`).
- Verify before push as non-root (`RE_SEARCH_PYTHON` unset): `check_file_length.py`, `node --check` + `node --test`, `cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or`, `cargo test --workspace --locked --no-fail-fast`.
- Write `docs/design/modes-2026-11/reports/exp-x16-cite.md` and push to `gemini/modes/exp-x16-cite`.
