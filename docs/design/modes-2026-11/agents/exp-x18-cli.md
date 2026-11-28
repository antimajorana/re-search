# Brief exp-x18-cli — Explore CLI-Subscription Exchanges & `explore/1` Conversation Turns (Package X18)

**Split note:** Split out from `draft-exp-x17-x18-lib-cli.md` into an independent lane from **`exp-x17-library`** and serialized strictly **AFTER Solve `S15`** (`solve-s15-cli-parity`) + **`exp-x9-x10-x11-read-verdict`** (+ **`exp-x15-chat`** for Checkpoint 2).

## 1. Package and governing design sections

- **Package:** `X18` — CLI-subscription routes (`claude-cli`, `antigravity`) for Explore:
  - **Checkpoint 1:** Explore pipeline exchanges (`Sharpen`, `Triage`, `Passage reader`, `Map`, `Directions`) via plain JSON-block prompts + existing Rust validators & F9 budget checks.
  - **Checkpoint 2:** Explore Conversation turns (`explore/1` over `core.exe` MCP server + X15 `idea_chat` multi-paper line-citation check, reusing `S15`'s mode-neutral `CORE_TOOL_PROFILE` plumbing in `cli_turn*` / `core_tools.rs` / `mcp-server`).
- **Governing design:**
  - `docs/design/modes-2026-11/00-overview.md`: sections 2.2 (`explore/1`), 2.5 (F4 `lines_sent`), 3.1 row 12, 3.2 row 20, and 8.2 decision 1.
  - `docs/design/modes-2026-11/explore.md`: sections 5.1 (CLI JSON-block exchange path using exact same validators), 5.2 (`explore/1`), 6 (Rules P3, P5, S8), 9 (row X18), and 10 (free core vs CLI-subscription access).
  - `docs/design/modes-2026-11/agents/RULES.md` (includes scoped `exp-x18-cli` seam authorization).

## 2. Start conditions and verified post-S15 seams

- **Required merges on `gemini/work/fixed`:**
  - **Checkpoint 1:** **`solve-s15-cli-parity`** AND **`exp-x9-x10-x11-read-verdict`** (and transitively `X6` `exchange.rs` + `idea_exchanges.rs`). (Checkpoints 1 & 2 should wait until **`exp-x15-chat`** also merges if launched together, or push CP1 first and CP2 after `X15`.)
- **Verified `solve-s15-cli-parity` architecture on `gemini/work/fixed`:**
  - `S15` ALREADY split `crates/desktop-server/src/cli_turn.rs` (now `~295`L) into `cli_turn_setup.rs` (`~547`L), `cli_turn_solve.rs` (`~247`L), and `cli_turn_tests.rs` (`~321`L), and updated `crates/cli-route/src/core_tools.rs` (`~257`L) and `crates/mcp-server/src/{main.rs,solve_wire.rs}` (`386`L / `445`L) for mode-neutral `CORE_TOOL_PROFILE`!
  - **Line-count headroom warning:** `cli_turn_setup.rs` is near `547`L! Keep edits to `cli_turn.rs` / `cli_turn_setup.rs` tiny (`<= 15`L) or place Explore CLI helpers in `crates/desktop-server/src/idea_cli_exchange.rs` (`<= 600`L) / `cli_turn_explore.rs` (`<= 600`L) so every file stays `<= 600`L.
  - **Never weaken sandbox guards:** `UiRelay` (`cli_relay.rs`) MUST remain enforced on Antigravity / Claude CLI routes.

## 3. Smallest implementation checkpoints (push after each!)

1. **Checkpoint 1 — Explore pipeline exchanges over CLI routes:**
   - `crates/desktop-server/src/idea_cli_exchange.rs` (new, `<= 600`L) + minimal hook in `idea_exchanges.rs` (and `idea_exchanges_triage.rs` / `idea_read.rs` where exchanges dispatch):
     - Implements plain prompt-in, JSON-block-out exchange runner over `CliRouteKind::Claude` and `CliRouteKind::Antigravity`.
     - Pipes extracted JSON through the **exact same Rust validators/recorders** (`record_idea_brief`, `record_idea_triage`, `record_paper_evidence`, `record_idea_map`, `record_idea_directions`), preserving `<= 2` repair rounds, F9 budget checks before every CLI call (Rule P3), `idea.exchange` audit events with CLI route provenance, and `UiRelay` (`cli_relay.rs`) approval handling.
     - Removes X6's `"not supported on CLI route yet"` refusal for Explore exchanges when running on `claude-cli` / `antigravity`.
     - Push Checkpoint 1 to `gemini/modes/exp-x18-cli`.

2. **Checkpoint 2 — Explore `explore/1` Conversation turns on CLI routes (requires `exp-x15-chat`):**
   - `crates/desktop-server/src/cli_turn_explore.rs` (new, `<= 600`L) + minimal hooks in `cli_turn.rs` / `cli_turn_setup.rs`:
     - Verify S15's `CORE_TOOL_PROFILE` wiring serves `explore/1` cleanly in `cli-route/src/core_tools.rs` (`core_tool_ids_for`, `core_tool_set_for_profile`) and `crates/mcp-server/src/main.rs` without injecting Research-only `search_formal_library`/`sympy_eval` prompts (`cli_turn_setup.rs`).
     - Route `Mode::Explore` CLI Conversation turns using X15's `idea_system_prompt` and post-turn multi-paper line-citation check (`idea_chat` + F4 `lines_sent_with_ledger`).
     - Push Checkpoint 2 to `gemini/modes/exp-x18-cli`.

## 4. Owned files & Do-Not-Edit (`<= 600` lines each)

- **New:** `crates/desktop-server/src/idea_cli_exchange.rs`, `crates/desktop-server/src/cli_turn_explore.rs` (if needed), `crates/desktop-server/tests/idea_cli_exchange.rs`.
- **Small edits:** `crates/desktop-server/src/{lib.rs,idea_exchanges.rs,idea_exchanges_triage.rs,idea_read.rs,cli_turn.rs,cli_turn_setup.rs,cli_relay.rs}`, and only if `explore/1` needs parity fixes: `crates/cli-route/src/core_tools.rs`, `crates/mcp-server/src/main.rs`.
- **Do NOT edit:** `crates/desktop-server/assets/library.js`, `crates/export/src/idea.rs`, `routes_idea_export.rs`, `routes_idea.rs`, `routes_idea_net.rs`, `routes_tools.rs` (610L), `http_srv.rs` (702L), `tool_profile.rs`, `resume.rs`, `Cargo.lock`, `tests/golden/{tool_defs,tool_profiles}.txt`.

## 5. Required tests & hand-back

- Cover in `idea_cli_exchange.rs` (fake/mock CLI subprocess + synthetic ledgers only, zero live vendor CLI/network): fake Claude & Antigravity CLIs returning JSON blocks (valid, malformed -> up to 2 repairs, fabricated quote fault returned to CLI, budget cap -> `stopped_at_cap`), `idea.exchange` provenance auditing, and (CP2) `explore/1` MCP tools serving without Research `sympy_eval`/`search_formal_library` prompt block + running `idea_chat` multi-paper line-citation check.
- Verify before push as non-root (`RE_SEARCH_PYTHON` unset): `check_file_length.py`, `cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or`, `cargo test --workspace --locked --no-fail-fast`, plus targeted `RE_SEARCH_PYTHON="$(command -v python3)" cargo test --locked -p worker -p mcp-server -p research-tools`.
- Write `docs/design/modes-2026-11/reports/exp-x18-cli.md` and push to `gemini/modes/exp-x18-cli`.
