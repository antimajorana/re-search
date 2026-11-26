# Brief solve-s15-cli-parity — Solve CLI & MCP Parity (S15)

**Status:** Queued: **CP1** starts behind **`S11 + S12`**; **CP2** (API-verifier review) finishes after **`S14`**. **Cross-group seam rule:** Coordinate with Explore `X18` (`cli_turn.rs` / `core_tools.rs` / `mcp-server/src/main.rs`) — whichever merges first splits `cli_turn.rs` (`597`L) into `cli_turn_setup.rs` + `cli_turn_tests.rs`. Zero `Cargo.lock` edits (`mcp-server` stays free of `crates/agent`!)!
**Push branch:** `gemini/modes/solve-s15-cli-parity`

---

## 1. Package & Governing Design

- **Package:** **S15** — Subscription-CLI & MCP parity for `solve/1` (`cli_turn_setup.rs`, `cli-route/src/core_tools.rs`, `mcp-server/src/main.rs` + optional `solve_wire.rs`, binding `research_tools::solve_gates::SolveGate` + S9 auto-checks over MCP `core.exe`), gating Research prompt instructions strictly to `Mode::Research`, ledger-driven result-sheet honesty (`Not done: N steps unchecked`), and CP2 D16 review handling after S14.
- **Governing design:** `00-overview.md` 2.2, 3.1 row 12, 8.2 D1; `solve.md` 0.1, 0.3, 5.1, 5.4, 6 (R1, R11, R12, R15, R18, R20, R22), 7.2, 9 (`S15`), 12 (risks 13, 14), D16; `agents/RULES.md`.

---

## 2. Checkpoints — Push After Each!

1. **Checkpoint S15a — Zero-behaviour-change split (`cli_turn.rs` `597`L -> `cli_turn_setup.rs` + `cli_turn_tests.rs`):** Commit alone if not already split by X18.
2. **Checkpoint S15b (`core_tools.rs` + `cli_turn.rs`):**
   - `cli-route/src/core_tools.rs::core_tool_ids_for`: honor `CORE_TOOL_PROFILE` first via `core_tool_set_for_profile` (tolerating `NoWorker` cleanly during tool-ID enumeration for Antigravity `enabledTools` = 17 `solve/1.mcp_defs()`, clean fallback on invalid profile), else legacy `CORE_ENABLE_RESEARCH_TOOLS`.
   - `cli_turn.rs`: detect mode (`session_mode_from_events`), gate Research `search_formal_library`/`sympy_eval` prompt block strictly to `Mode::Research`, pass `CORE_TOOL_PROFILE` + check-worker env vars in `core.env`, build Solve prompt from setup + `resume_brief`, record `"tool_profile": "solve/1"` on `session.start`/`session.resume`.
3. **Checkpoint S15c (`mcp-server/src/main.rs` + `solve_wire.rs` + MCP gate tests):**
   - Fix `main.rs` `use_profile` check so `env("CORE_TOOL_PROFILE").is_some()` activates without `CORE_ENABLE_RESEARCH_TOOLS` or a workspace.
   - Attach solver + check `WorkerEpoch` bindings, **`research_tools::solve_gates::SolveGate`** (G1/G4/G5/G6/G7), and S9 auto-check hook (`host:check` through `TcpApprover`, zero auto-approve!). Keep `mcp-server/Cargo.toml` free of `agent`.
4. **Checkpoint S15d (CP2 after S14 merges — D16 API-transport review):** Invoke S14 `verifier_loop` over configured API `roles.verifier` (respecting `agent_models.rs` helper-role rule against CLI helper roles) or surface D16 notice on setup card.

---

## 3. Owned Files & Do-Not-Edit (`<= 600` Lines Each)

- **New:** `crates/desktop-server/src/cli_turn_setup.rs`, `cli_turn_tests.rs`, `crates/mcp-server/src/solve_wire.rs`, `crates/mcp-server/tests/solve_mcp_gates.rs`.
- **Small edits:** `crates/desktop-server/src/{cli_turn,lib}.rs`, `crates/cli-route/src/core_tools.rs`, `crates/mcp-server/src/main.rs`.
- **Do NOT edit:** `Cargo.lock`, `crates/mcp-server/Cargo.toml`, `tool_profile.rs`, `tool_defs.txt`, `tool_profiles.txt`, `http_srv.rs`, `autonomous.rs`.

---

## 4. Mandatory Verification Before Every Push (`RULES.md`)

Run on Linux **as non-root (`id -u != 0`) with `RE_SEARCH_PYTHON` unset**:
```bash
python3 scripts/check_file_length.py --base origin/gemini/work/fixed
unset RE_SEARCH_PYTHON
cargo clippy --workspace --all-targets --locked -- -D warnings -A clippy::manual_unwrap_or
cargo test --workspace --locked --no-fail-fast
```
Write `docs/design/modes-2026-11/reports/solve-s15-cli-parity.md` and push to `gemini/modes/solve-s15-cli-parity`. End with `PUSHED`.
