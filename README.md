# re:Search

A small Rust core for a local mathematics and theoretical-physics research
assistant. On Windows 11, SymPy/NumPy code runs inside a zero-capability
Windows AppContainer sandbox; every turn, approval, and tool call is recorded in
an append-only, SHA-256 hash-chained SQLite ledger (`--route cli` via the
Claude Code CLI, or `--route api` via the in-core agent loop over Anthropic
Messages HTTP).

**Read [THREAT-MODEL.md](THREAT-MODEL.md) for what AppContainer contains and what remains outside the boundary.**

## Layout

| Crate | What it is |
| --- | --- |
| `crates/policy` | Pure allow / ask / deny engine. No I/O; depends on `serde` only. |
| `crates/ledger` | Append-only, SHA-256 hash-chained SQLite event store and SHA-256 blob directory. |
| `crates/tools` | Tool registry, router (policy, approval, ledger), digest-bound and epoch-bound approvals. |
| `crates/worker` | Python worker under Windows `AppContainerSandbox` / `JobObjectSandbox`, `STARTUPINFOEXW` handle scoping, fingerprinted runtime staging, JSON lines on two pipes. |
| `crates/agent` | `Transport` trait, hardened `AnthropicTransport` (`SecretKey`), and in-core `AgentLoop` (`POST /v1/messages`). |
| `crates/mcp-server` | `core.exe`, the stdio MCP server (the only crate that uses `rmcp`). |
| `crates/cli` | `re-search`, the supervisor that runs CLI or API sessions and manages console approvals. |
| `python/worker.py` | The worker script (SymPy / NumPy). |

## Build and test

`policy`, `ledger`, `tools`, `agent`, and `cli` unit and API-route tests run on
both **Windows and Linux** (against an in-test local mock HTTP server, never
calling paid external APIs):

```bash
cargo build
cargo test -p policy -p ledger -p tools -p agent -p cli
```

The `worker` crate, `AppContainerSandbox` containment probes, and end-to-end
contained router tests require **Windows 11 + Python (SymPy and NumPy)**:

```powershell
$env:RE_SEARCH_PYTHON = "C:\path\to\python.exe"
cargo test -p worker -- --nocapture
cargo test -p cli --test contained_router -- --nocapture
```

(`RESEARCH_CORE_*` environment variables are also accepted as fallbacks when
the corresponding `RE_SEARCH_*` variable is unset.)

## Run a session

Run this in a real terminal, because the approval prompt reads from it.
`re-search run` supports two routes via `--route cli|api` (or
`RE_SEARCH_ROUTE`):

- **`--route cli` (default, `"external loop, partial provenance"`):** launches
  `claude.exe` headless in a Job Object wired to `core.exe` over stdio MCP.
  Requires `--python` (or `RE_SEARCH_PYTHON`), `claude.exe`, and `core.exe`;
  `--model` defaults to `haiku`.
- **`--route api` (Stage 4, `"in-core loop, full provenance"`):** runs the
  in-core agent loop directly against `POST /v1/messages` using
  `RE_SEARCH_API_KEY` (or `ANTHROPIC_API_KEY`) with no `claude.exe` or
  `core.exe`. `--model` defaults to `claude-haiku-4-5` (`haiku` maps to
  `claude-haiku-4-5`), and supports `--max-turns`, `--max-tokens`, and
  `--api-base` (or `RE_SEARCH_API_BASE`, default `https://api.anthropic.com`).
  `--python` is optional on `--route api`: when provided on Windows, `sympy_eval`
  uses `AppContainerSandbox`; when omitted, `ledger_note` stays active and
  `sympy_eval` remains registered as `Once`-scoped (returning an error if
  invoked).

```powershell
# Default CLI route (Windows 11 + Claude Code CLI + Python):
$env:RE_SEARCH_PYTHON = "C:\path\to\python.exe"
.\target\debug\re-search.exe run --prompt "Use sympy_eval to integrate x*sin(x) with respect to x, then state the result."

# In-core API route (optional --python on Windows; cross-platform without --python):
$env:RE_SEARCH_API_KEY = "sk-ant-..."
.\target\debug\re-search.exe run --route api --data-dir .\data --prompt "Use ledger_note to record a greeting, then summarize."
```

On first Windows worker run, the supervisor stages a read-only copy of the
Python runtime plus `sympy`, `mpmath`, and `numpy` into
`<data-dir>\runtime\<fingerprint>\` and creates a per-session AppContainer
profile (`re-search.worker.<session>`) with write access limited to
`sessions\<id>\scratch` and `sessions\<id>\pycache`. You will see a terminal
approval prompt showing the code, containment status, and scope
(`worker-session`: once approved, further `sympy_eval` calls reuse the grant
until that worker epoch exits or times out).

`re-search run` exit codes: `0` = completed, `2` = usage, configuration, or fatal
ledger error, `3` = aborted (init check, transport/protocol error, `max_tokens`
truncation, or `max_turns`), `5` = model refusal (`stop_reason: "refusal"`).
Inspect and verify sessions offline with:

```powershell
.\target\debug\re-search.exe sessions --data-dir .\data
.\target\debug\re-search.exe show s-<id> --data-dir .\data
.\target\debug\re-search.exe verify --data-dir .\data
.\target\debug\re-search.exe verify s-<id> --data-dir .\data
```

`re-search verify [SESSION] [--data-dir DIR]` verifies SHA-256 hash chains,
canonical JSON payloads, schema triggers, and blob integrity offline without
creating missing files or modifying schema metadata (exit `0` = valid, `1` =
tampered, corrupt, or unknown session, `2` = usage or missing database, `4` =
legacy unchained v0 schema — pass a fresh `--data-dir` when starting new
hash-chained sessions if an older Stage 1/2 directory exists).

Data (ledger, blobs, staged runtimes, per-session folders, and bytecode caches)
lives in `%LOCALAPPDATA%\re-search` on Windows unless you pass `--data-dir` (or
set `RE_SEARCH_DATA`; on Linux, pass `--data-dir` or set `RE_SEARCH_DATA`).

## Requirements

- Rust (stable) on Linux or Windows (`crates/agent`, `policy`, `ledger`,
  `tools`, and `cli` `--route api` without `--python`).
- Windows 11 + Python 3 with SymPy and NumPy for `AppContainerSandbox` worker
  execution (`worker` crate, `mcp-server`, and `--python`).
- Claude Code CLI, logged in, for `--route cli` (developed against `2.1.318`;
  the version is checked on start and at `system/init`, and mismatches warn on
  stderr and are recorded in the ledger).

## Licence

Dual-licensed under either of [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE), at your option. `rmcp` (Apache-2.0) is a
dependency of `mcp-server` only and is kept out of `policy`, `ledger`, and
`agent`.
