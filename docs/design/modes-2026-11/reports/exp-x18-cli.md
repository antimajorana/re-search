# Report: `exp-x18-cli` — Explore CLI-Subscription Exchanges & `explore/1` Conversation Turns (Package X18)

## Metadata

- **Brief:** `docs/design/modes-2026-11/agents/exp-x18-cli.md`
- **Branch pushed:** `gemini/modes/exp-x18-cli` (local `work/exp-x18-cli`)
- **Initial base commit (`origin/gemini/work/fixed` at branch start):** `ffee92f4bb0372a362b2e2781bb3c8da13b7cdaa` (`Merge pull request #79 from antimajorana/gemini/modes/exp-x9-x10-x11-read-verdict`)
- **Latest `origin/gemini/work/fixed` merged (`--no-ff`):** `7b1c191` (`Merge pull request #80 from antimajorana/gemini/modes/f8-set-c-harness`) -> merge commit `279310b`
- **Scope guardrails verified:** `Cargo.lock`, `crates/research-tools/tests/golden/{tool_defs,tool_profiles}.txt`, `tool_profile.rs`, `resume.rs`, `session_mode.rs`, `turn_runner.rs`, `http_srv.rs`, `routes_tools.rs`, `routes_idea*.rs`, `assets/library.js`, and `crates/export/src/idea.rs` are untouched. No new dependencies, no skipped/`#[ignore]` tests added, and every touched source file is `<= 600` lines.

---

## 1. What was implemented

### Checkpoint 1 — Explore pipeline exchanges over subscription CLIs (`7c4bc4d`, `aa8d9bc`, `73169aa`)

Implemented plain prompt-in, JSON-block-out structured exchanges for Explore over both `claude-cli` (`ClaudeCode`) and `antigravity` (`Antigravity`), plugging directly into `agent::exchange::run_exchange` so every Explore pipeline stage (`Sharpen`, `Triage`, `Passage reader`, `Map`, and future `Directions` once X13 merges) runs the **exact same Rust validators**, `<= 2` repair rounds, F9 pre-call budget checks (Rule P3), cooperative cancellation, and `idea.exchange` provenance auditing as API routes:

- **`crates/desktop-server/src/idea_cli_exchange.rs` (592 lines, new):**
  - Thread-safe `CliExchangeTransport` implementing `agent::Transport`, safe for concurrent parallel X10 passage readers via isolated per-attempt `AttemptDirGuard` directories (`ex-<random_hex>`).
  - `build_cli_exchange_transport`: enforces `antigravity::require_acknowledged` (`403` when unacknowledged), subscription billing safety (`env_auth` opt-in read strictly from real `std::env::var_os`, never spoofable via `connection_env`), `locate`, `pick_model`, serialized cached `--version` probing (`Mutex<Option<String>>` seeded from `DesktopState::{claude_version, agy_version}`), and `env_auth_vars_allowed` provenance recording in request/response audit blobs.
  - Cross-platform `ExchangeChild` (`worker::sandbox::JobObjectSandbox` on `cfg(windows)`, `std::process::Child` on non-Windows, kill-on-drop), bounded `BufReader::new(stdout.take(4 MiB + 1))` safety ceiling, cooperative `CancelToken` polling (`50 ms`), immediate process kill on Rule P2 unauthorized tool calls while still returning `ModelResponse { ToolUse }` so `run_exchange` records `ExchangeError::UnexpectedToolCall`, and non-blocking cleanup on force-kill/abort/timeout/cancel.
- **`crates/desktop-server/src/idea_cli_exchange_parse.rs` (563 lines, new):**
  - Pinned flags `exchange_claude_args` (`-p --model ... --output-format stream-json --verbose --tools "" --strict-mcp-config --mcp-config <empty> --setting-sources "" --disable-slash-commands --no-session-persistence --permission-prompts none`, optional `--effort`, prompt strictly on stdin) and `exchange_agy_args` / `exchange_agy_settings` (`agentMode: ""`, `toolPermission: "request-review"`, `enableTerminalSandbox: true`, `allowNonWorkspaceAccess: false`, `allow: []`, `deny: [all AGY_DENIED_KINDS(*), "mcp(*)"]`).
  - Fail-closed handshakes (`check_claude_exchange_init` requiring `tools == []`, `mcp_servers == []`, and `apiKeySource == "none"` unless opted in; `agy_check_init` with `&[]`).
  - `extract_json_object`: CommonMark 3+ backtick fence extraction, unclosed EOF fence flush, strict no-prose-fallback when a ```` ```json ```` block is present but malformed, plus fallback to generic fenced / raw / balanced `{...}` object when no ```` ```json ```` block exists.
  - `ExchangeStreamState`: multi-block assistant text accumulation (`text_blocks`), Claude `model.tool_use` Rule P2 detection, and Antigravity `vendor.builtin_tool` pending/terminal step tracking (catches completed non-denied AND unresolved active-only built-in tools while allowing cleanly soft-denied `denied_or_failed: true` steps).
- **`crates/desktop-server/src/idea_exchanges.rs` (557 lines, edited):**
  - Removed X6's `CLI_ROUTE_UNSUPPORTED_MSG` refusal; wired `resolve_explore_transport` to `build_cli_exchange_transport` for `claude-cli` and `antigravity`.
  - Enforced `409 Conflict` cross-route mixing refusal across `meta.route`, `session.start` (`route` and `provider`), `session.resume`, and `idea.exchange`; prioritized `explicit request > session_default > orchestrator role > ui_default`; persisted `meta.route` on both explicit and implicit CLI selection.
- **`crates/desktop-server/src/lib.rs` (281 lines, edited):** exported `pub mod idea_cli_exchange;` and `pub mod cli_turn_explore;`.
- **CP1 Tests:**
  - `crates/desktop-server/tests/idea_cli_support/mod.rs` (259 lines, new): cross-platform `rustc`-compiled `MultiAttemptCli` test harness (zero Unix shell-outs, supports per-attempt NDJSON scripts, concurrent `script.ndjson`, `--version` call counting, per-attempt CWD logging, `sleep_ms`, `huge_line_bytes`).
  - `crates/desktop-server/tests/idea_cli_exchange.rs` (514 lines, new): 4 integration tests covering pure flags/handshakes/JSON extraction, HTTP `/idea/sharpen` on fake Claude & Antigravity CLIs + `403` unacknowledged Antigravity + `ANTHROPIC_API_KEY` billing guard, malformed JSON repair -> 2-repair cap -> F9 budget `StoppedAtCap` (0 CLI spawns) -> Rule P2, and full Sharpen -> Triage (Antigravity) -> Passage Reader (fabricated quote fault returned with true lines -> repair) -> Map pipeline.
  - `crates/desktop-server/tests/idea_cli_exchange_audit.rs` (503 lines, new): 5 Carbon-audit regression tests covering `mcp(*)` deny, Antigravity `vendor.builtin_tool` Rule P2 (DONE, unresolved ACTIVE, cleanly soft-denied), JSON fence edge cases, multi-block Claude text, live bad-init abort, cooperative `CancelToken`, 4 MiB newline-free ceiling, parallel X10 passage readers (`concurrency = 2`, distinct attempt dirs, `--version` called once), and `API -> CLI` / `claude-cli -> antigravity` `409` refusals.
  - `crates/desktop-server/tests/idea_search_triage.rs` (596 lines, edited): updated only the two obsolete X6 placeholder assertions that checked `"not supported on CLI route yet"` to verify X18 behavior (`403` unacknowledged Antigravity, `400` missing Claude CLI, `409` cross-route refusal).

### Checkpoint 2 — Explore `explore/1` Conversation turns & MCP parity (`c260568`, `6d9c405`)

- **`crates/mcp-server/src/main.rs` (452 lines, edited):**
  - Resolved `resolved_profile` **before** `build_worker` so `explore/1` (`is_explore`) never spawns or requires a Python compute worker even when `CORE_PYTHON`/`CORE_WORKER_SCRIPT` are set.
  - Added `session_has_mode_profile` so a mode-tagged session (`idea.started`, `solve.started`, `investigation.started`, or explicit `session.start.tool_profile`) without `sources/` and without `CORE_TOOL_PROFILE` resolves its mode profile instead of silently falling back to v0 `sympy_eval` + `ledger_note`.
  - Added `EXPLORE_CORE_INSTRUCTIONS` for `CoreServer::get_info` when `router.tool_profile() == Some("explore/1")` (names only `explore/1` tools; zero `sympy_eval`, `ledger_note`, `search_formal_library`, or `"verified"`).
  - Forwarded `RE_SEARCH_OPENALEX_BASE` in `build_research_env`.
- **`crates/desktop-server/src/cli_turn_explore.rs` (598 lines, new):**
  - `EVENT_IDEA_CHAT_CITATIONS = "idea.chat_citations"` and `EXPLORE_CLI_SYSTEM_GUIDANCE` (named constants for easy alignment with X15).
  - `resolve_explore_v1_profile` and `build_explore_cli_prompt`: wraps pinned conventions, latest `idea.stated`, current-revision `idea.brief` / `idea.map` (gated by `e.seq > latest_stmt_seq` so rev-2 statements hide stale rev-1 brief/map), ingested papers (listing `primary_path`, never `assembled_path`), matched quotes, and bounded frozen `lessons.supplied` (max 8 notes / 2,000 chars) inside Rule C7 `wrap_untrusted_data("EXPLORE_CONTEXT", ...)`. Deliberately excludes Research `project_context` (`check_convergence`) and Research tool/Lean/`"verified"` wording.
  - `parse_multi_paper_citations`, `check_explore_cli_citations`, and `record_explore_cli_citations`: parses `[paper_key: Lstart-end]`, `[paper_key: Lstart–Lend]`, `[doi:...: L1-3]`, `[paper_key L1-3]`, comma lists, and bare `[L1-3]`; iterates `source.ingested` in reverse (latest receipt wins); checks F4 `lines_sent_with_ledger` across all readable non-assembled `(path, file_sha256)` files (primary + secondary `.tex` files, never `assembled_path`); resolves mid-conversation `idea_fetch_papers` `paper_key`s from both inline and blob-backed `tool.result` events even when no `idea.paper` event exists; flags invalid/zero/inverted ranges as `unread` (`"invalid line range"`); enforces caller isolation and multi-paper unqualified ambiguity; persists `idea.chat_citations` with `"matched"`/`"unread"` (Rule S8: never `"verified"`).
- **`crates/desktop-server/src/cli_turn_setup.rs` (589 lines, edited) & `crates/desktop-server/src/cli_turn.rs` (346 lines, edited):**
  - `core_env_for_profile`: gated `CORE_ENABLE_RESEARCH_TOOLS=1` to `Mode::Research | Mode::Investigate` and omitted `CORE_PYTHON`/`CORE_WORKER_SCRIPT` for `Mode::Explore`.
  - `cli_turn.rs`: excluded `Mode::Explore` from Research `subagent` prompt injection, Research `project_context`, and `needs_title`; routed `Mode::Explore` prompt construction (including `focus_node_id` / `node_id`) to `build_explore_cli_prompt`; called `record_explore_cli_citations` after completed Explore CLI turns (`UiRelay` with session network grant already active).
  - Kept `crates/desktop-server/src/session_mode.rs::DESKTOP_MODE_EXPLORE_ENABLED = false` and `resolve_cli_turn_profile` gate untouched so existing `cli_turn::tests::resolve_cli_turn_profile_maps_modes_and_rejects_unbuilt_explore` stays green until `exp-x15-chat` merges.
- **CP2 Tests:**
  - `crates/mcp-server/tests/explore_mcp_parity.rs` (409 lines, new): 2 end-to-end tests spawning the compiled `core` binary over stdio MCP verifying `explore/1` instructions, exact 5 `explore/1.mcp_defs()` tools (even with `CORE_ENABLE_RESEARCH_TOOLS=1` + `CORE_WORKSPACE` + bogus `CORE_PYTHON`), `read_paper_lines` and `search_paper_passages` F4 crediting, fail-closed `DENIED` on `idea_search` and `idea_fetch_papers` without relay, approved `idea_search` and `idea_fetch_papers` over loopback `cli_route::relay` + mock arXiv, `CORE_CALLER_ROUTE=antigravity` caller isolation, and sourceless `idea.started` sessions resolving to `explore/1`.
  - `crates/desktop-server/tests/cli_turn_explore.rs` (431 lines, new): 4 tests covering `core_env_for_profile` + `core_tool_ids_for`, Explore CLI prompt + focus node + zero Research/verified wording, 2-paper F4 caller isolation + `idea.chat_citations` persistence, and TeX multi-file (`assembled_path` != `primary_path`, secondary `app.tex`) + mid-turn blob `idea_fetch_papers` key resolution + revision staleness + Rule C7 `conventions` + stripped `project_context` + DOI/inverted/zero range parsing.

---

## 2. Independent Carbon Audits & Fixes Applied

Two focused independent Carbon (`pro`) read-only audits were executed (one for CP1, one for CP2). All CRITICAL and HIGH findings in owned files were fixed and covered by new regression tests:

- **CP1 Audit Fixes (`73169aa`):**
  1. Added `"mcp(*)"` to `"deny"` in `exchange_agy_settings()` (closes user-level global/plugin MCP allow-rule bypass during zero-tool plain exchanges).
  2. Added pending + terminal step tracking for Antigravity `vendor.builtin_tool` Rule P2 violations and immediate child kill on mid-stream unauthorized tool calls (while still recording `UnexpectedToolCall` in `idea.exchange`).
  3. Bounded `stdout` `BufReader` with `Read::take(4 MiB + 1)` to prevent OOM on newline-free output, and avoided blocking on detached pipe joins during force-kill cleanup.
  4. Hardened `extract_json_object` (3+ backtick CommonMark fences, unclosed EOF flush, no stray prose fallback on malformed ```` ```json ```` blocks) and accumulated multi-block assistant text.
  5. Serialized cached `--version` probes via `Mutex<Option<String>>` across concurrent passage readers, recorded `env_auth_vars_allowed` provenance, checked `session.start` `provider` in `recorded_exchange_route`, and fixed `session_default` priority + `meta.route` persistence in `resolve_explore_transport`.
- **CP2 Audit Fixes (folded into `c260568` & `6d9c405`):**
  1. Fixed `collect_session_papers` to use `primary_path` + all non-assembled `files_sha256` entries (never `assembled_path`, which `read_paper_lines` refuses and `search_paper_passages` skips), and reversed `source.ingested` traversal so latest receipt wins.
  2. Added mid-conversation `idea_fetch_papers` `tool.result` scanning (inline JSON string + blob-backed) so papers fetched during `explore/1` Conversation turns resolve by `<paper_key>`.
  3. Removed Research `project_context` (`check_convergence`) and Research `subagent` injection from `Mode::Explore`, and moved `conventions` inside the Rule C7 `EXPLORE_CONTEXT` wrapper.
  4. Gated `idea.brief` / `idea.map` on `e.seq > latest_stmt_seq`, bounded `lessons.supplied` (8 notes / 2,000 chars), scanned colons right-to-left (`rmatch_indices`), accepted `Lstart-Lend`, and flagged zero/inverted line ranges as `unread`.
  5. Added `session_has_mode_profile` in `mcp-server/src/main.rs` so sourceless Explore sessions without `CORE_TOOL_PROFILE` never fall back to `sympy_eval`.

---

## 3. Deviations, Decisions Taken Alone, and Assumptions

- **`exp-x15-chat` and `exp-x13` (`idea_directions.rs`) are not yet merged on `origin/gemini/work/fixed`:**
  - Per `RULES.md` ("do not wait for it and do not implement it"), `DESKTOP_MODE_EXPLORE_ENABLED` in `crates/desktop-server/src/session_mode.rs` was left `false`, `turn_runner.rs` was not touched, and `cli_turn_explore.rs` implements a self-contained Explore CLI prompt + F4 multi-paper citation checker against the `explore.md` §5.2 contract with explicit merge seams for X15.
  - Because `DESKTOP_MODE_EXPLORE_ENABLED` is `false`, `cli_turn::run_cli_turn` on an Explore session still returns the gated error from `resolve_cli_turn_profile` until X15 flips that flag; every CP2 component (`core` MCP binary, `core_env_for_profile`, `core_tool_ids_for`, `build_explore_cli_prompt`, `check_explore_cli_citations`, `record_explore_cli_citations`) is tested directly, and `cli_turn::run_cli_turn` will activate automatically as soon as X15 flips `DESKTOP_MODE_EXPLORE_ENABLED = true`.
- **`crates/cli-route/src/core_tools.rs` required zero edits:** S15's mode-neutral `core_tool_set_for_profile` / `core_tool_ids_for` already served `explore/1` cleanly (verified by tests in `cli_turn_explore.rs` and `explore_mcp_parity.rs`).
- **`crates/agent/src/exchange.rs` was not edited:** `CliExchangeTransport` integrates via `agent::Transport`, keeping `exchange.rs` untouched and storing detailed CLI provenance (`route`, `provenance`, `confinement`, `cli_version`, `init_verified`, `env_auth_vars_allowed`, `raw_ndjson_lines`) inside the `idea.exchange` request/response blobs while `idea.exchange.provider` records `"claude-code-cli"` or `"antigravity"`.
- **Assumptions:**
  - `EVENT_IDEA_CHAT_CITATIONS = "idea.chat_citations"` and citation bracket formats (`[paper_key: Lstart-end]`, `[paper_key Lstart-end]`, `[source_id: Lstart-end]`, `[Lstart-end]`) are placed in single named constants/functions in `cli_turn_explore.rs` so X15 can unify them in one place if its event name differs.
  - Claude Code and Antigravity CLI flags and NDJSON schemas follow the pinned repository fixtures `crates/cli-route/fixtures/{claude_help_2_1_318.txt, agy_help_1_5_14.txt, fake_cli.rs, fake_agy.rs}`.

---

## 4. Merge Notes

Files edited that other concurrent briefs may touch, plus exact integration seams:

1. **`crates/desktop-server/src/idea_exchanges.rs` (557L):**
   - `resolve_explore_transport` now returns `CliExchangeTransport` for `claude-cli` / `antigravity`.
   - **Seam with `exp-x13` (`idea_directions.rs`):** once X13 merges, route `Directions` through `crate::idea_exchanges::resolve_explore_transport` (just like `Sharpen`, `Triage`, `Passage reader`, and `Map`) to get CLI-subscription support automatically.
2. **`crates/desktop-server/src/cli_turn_explore.rs` (598L, new):**
   - **Seam with `exp-x15-chat` (`idea_system_prompt.rs`, `idea_chat.rs`):** once X15 merges and flips `DESKTOP_MODE_EXPLORE_ENABLED = true` in `session_mode.rs`, `build_explore_cli_prompt` can delegate prompt/context construction to X15's `idea_system_prompt`, and `check_explore_cli_citations` / `record_explore_cli_citations` can delegate to X15's `idea_chat` multi-paper citation checker (unifying `EVENT_IDEA_CHAT_CITATIONS`).
3. **`crates/desktop-server/src/cli_turn.rs` (346L) & `crates/desktop-server/src/cli_turn_setup.rs` (589L):**
   - Small, localized `Mode::Explore` branches (`core_env_for_profile`, `build_cli_prompt`, `needs_title`, `subagent`/`project_context` exclusion, post-turn `record_explore_cli_citations`). Research, Investigate, and Solve (`S15`) paths are preserved intact.
4. **`crates/mcp-server/src/main.rs` (452L):**
   - Profile resolution hoisted before `build_worker`; `EXPLORE_CORE_INSTRUCTIONS` added; `session_has_mode_profile` added; `RE_SEARCH_OPENALEX_BASE` forwarded.
5. **`crates/desktop-server/src/lib.rs` (281L):**
   - Added `pub mod cli_turn_explore;` and `pub mod idea_cli_exchange;`.
6. **`crates/desktop-server/tests/idea_search_triage.rs` (596L):**
   - Updated the 2 X6 `"awaiting X18"` placeholder assertions to assert X18's consent/CLI/409 behavior.

---

## 5. Verification Commands & Exact Results

All checks ran on Linux as non-root (`uid=1001(antigravity)`) on the merged head (`279310b`, including `origin/gemini/work/fixed` `7b1c191`):

| Command | Result |
| --- | --- |
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | Exit 0 — `No touched source file is over 600 lines.` (max touched file: `cli_turn_explore.rs` at 598L; `idea_search_triage.rs` at 596L; `idea_cli_exchange.rs` at 592L; `cli_turn_setup.rs` at 589L) |
| `git diff origin/gemini/work/fixed -- Cargo.lock crates/research-tools/tests/golden/` | Exit 0 — empty diff (untouched) |
| `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` | Exit 0 — all JS files valid |
| `node --test crates/desktop-server/tests/*.test.mjs` | Exit 0 — `263 passed, 0 failed, 0 skipped` |
| `unset RE_SEARCH_PYTHON && CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or` | Exit 0 — 0 warnings, 0 errors |
| `unset RE_SEARCH_PYTHON && CARGO_INCREMENTAL=0 cargo test --workspace --locked --no-fail-fast` | Exit 0 — `1460 passed, 0 failed, 2 ignored` (the 2 ignored are pre-existing live-eval tests in `crates/agent/tests/eval_harness.rs`: `file_sheet_from_json` and `live_manifests_validate`; includes `idea_cli_exchange` 4/4, `idea_cli_exchange_audit` 5/5, `idea_search_triage` 4/4, `cli_turn_explore` 4/4, `explore_mcp_parity` 2/2, `solve_mcp_gates` 2/2, `tool_def_golden` 3/3, `tool_profile_golden` 3/3) |
| `RE_SEARCH_PYTHON="$(command -v python3)" CARGO_INCREMENTAL=0 cargo test --locked -p worker -p mcp-server -p research-tools` | Exit 0 — `659 passed, 0 failed, 0 ignored` |

- **Pre-existing failures:** None observed on Linux as non-root `uid=1001`.
- **Could not compile or run on Linux:**
  - `cfg(windows)` code paths (`worker::sandbox::JobObjectSandbox` inside `ExchangeChild::spawn`/`kill_tree` in `idea_cli_exchange.rs`, `AppContainer` sandbox tests in `crates/worker/tests/`).
  - Live vendor `claude` and `agy` CLI binaries (all tests use compiled fake CLIs per `RULES.md`).
  - Full live `cli_turn::run_cli_turn` on an Explore session (gated by `session_mode.rs::DESKTOP_MODE_EXPLORE_ENABLED = false` until `exp-x15-chat` merges).

---

## 6. Bugs Noticed in Non-Owned Files (Not Fixed)

1. **`crates/desktop-server/src/antigravity.rs:81`:** `POST /api/connections/antigravity/withdraw` calls `state.stop_cli_turns_of(CliRouteKind::Antigravity)` which stops running `cli_turn` turns in `state.cli_turns`, but does not cancel `state.api_turns` for in-flight Explore `ActiveExploreGuard` CLI exchanges running on Antigravity.
2. **`crates/desktop-server/src/routes_idea_read.rs:46-51`:** calls `resolve_explore_transport` before `ActiveExploreGuard::acquire` (unlike `idea_exchanges.rs`, `idea_exchanges_triage.rs`, and `idea_map_exchange.rs`). A concurrent `POST /idea/read` with `{"provider": "claude-cli"}` on an already-active unbound session could persist `meta.route` before returning `409 session already has an active run`.
3. **`crates/research-tools/src/idea_tools.rs:376-390`:** `register_explore_v1_tools` uses `default_provider_config()` (environment variables only) rather than reading `lit_settings.json` from `CORE_DATA_DIR`, so an OpenAlex API key stored only in `lit_settings.json` is not used by `explore/1` MCP `idea_search` calls.
4. **`crates/desktop-server/src/cli_turn_setup.rs:106-120`:** `session_route` inspects `session.start`/`session.resume` and `meta.route`, not `idea.exchange`; mitigated for Explore exchanges by persisting `meta.route` in `resolve_explore_transport` for both explicit and implicit CLI selection.
