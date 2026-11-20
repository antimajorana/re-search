# re:Search

A small Rust core for a local mathematics and theoretical-physics research
assistant. On Windows 11, SymPy/NumPy code runs inside a zero-capability
Windows AppContainer sandbox; every turn, approval, and tool call is recorded in
an append-only, SHA-256 hash-chained SQLite ledger (`--route cli` via the
Claude Code CLI, or `--route api` via the in-core agent loop over Anthropic,
OpenAI, Ollama, or Gemini HTTP transports, with verified `re-search resume`).

**Read [THREAT-MODEL.md](THREAT-MODEL.md) for what AppContainer contains and what remains outside the boundary.**

## Layout

| Crate | What it is |
| --- | --- |
| `crates/policy` | Pure allow / ask / deny engine. No I/O; depends on `serde` only. |
| `crates/ledger` | Append-only, SHA-256 hash-chained SQLite event store and SHA-256 blob directory. |
| `crates/tools` | Tool registry, router (policy, approval, ledger), digest-bound and epoch-bound approvals. |
| `crates/worker` | Python worker under Windows `AppContainerSandbox` / `JobObjectSandbox`, `STARTUPINFOEXW` handle scoping, fingerprinted runtime staging, JSON lines on two pipes. |
| `crates/agent` | `Transport` trait, `AnthropicTransport`, `OpenAiTransport` (OpenAI/Ollama), `GeminiTransport`, `SecretKey`, in-core `AgentLoop`, and verified session resume. |
| `crates/mcp-server` | `core.exe`, the stdio MCP server (the only crate that uses `rmcp`). |
| `crates/cli` | `re-search`, the supervisor that runs/resumes sessions, enforces `.active.lock`, and manages console approvals. |
| `python/worker.py` | The worker script (SymPy / NumPy). |

## Build and test

`policy`, `ledger`, `tools`, `agent`, and `cli` unit and API/resume route tests
run on both **Windows and Linux** (against in-test local mock HTTP servers,
never calling paid external APIs):

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

## Run and resume a session

Run this in a real terminal, because the approval prompt reads from it.
`re-search run` supports two routes via `--route cli|api` (or
`RE_SEARCH_ROUTE`):

- **`--route cli` (default, `"external loop, partial provenance"`):** launches
  `claude.exe` headless in a Job Object wired to `core.exe` over stdio MCP.
  Requires `--python` (or `RE_SEARCH_PYTHON`), `claude.exe`, and `core.exe`;
  `--model` defaults to `haiku`.
- **`--route api` (Stage 4–5, `"in-core loop, full provenance"`):** runs the
  in-core agent loop directly over `--provider anthropic|openai|ollama|gemini`
  (or `RE_SEARCH_PROVIDER`, default `anthropic`) with no `claude.exe` or
  `core.exe`. Supports `--model`, `--max-turns`, `--max-tokens`, and
  `--api-base` (`RE_SEARCH_API_BASE`). `--python` is optional: when provided on
  Windows, `sympy_eval` uses `AppContainerSandbox`; when omitted, `ledger_note`
  stays active and `sympy_eval` stays `Once`-scoped (erroring if invoked).

Provider defaults and key lookup (`RE_SEARCH_API_KEY` -> `RESEARCH_CORE_API_KEY` -> provider env):

| `--provider` | Default model | Default `--api-base` | Fallback key env |
| --- | --- | --- | --- |
| `anthropic` | `claude-haiku-4-5` (`haiku`) | `https://api.anthropic.com` | `ANTHROPIC_API_KEY` |
| `openai` | `gpt-4.1-mini` | `https://api.openai.com` | `OPENAI_API_KEY` |
| `ollama` | `qwen2.5-coder:7b` | `http://127.0.0.1:11434/v1` | `OLLAMA_API_KEY` (keyless on loopback) |
| `gemini` | `gemini-2.5-flash` | `https://generativelanguage.googleapis.com` | `GEMINI_API_KEY` / `GOOGLE_API_KEY` |

```powershell
# Default CLI route (Windows 11 + Claude Code CLI + Python):
$env:RE_SEARCH_PYTHON = "C:\path\to\python.exe"
.\target\debug\re-search.exe run --prompt "Use sympy_eval to integrate x*sin(x) with respect to x, then state the result."

# In-core API route across providers (cross-platform without --python):
$env:OPENAI_API_KEY = "sk-..."
.\target\debug\re-search.exe run --route api --provider openai --data-dir .\data --prompt "Use ledger_note to record a greeting, then summarize."

# Local keyless Ollama over loopback HTTP:
.\target\debug\re-search.exe run --route api --provider ollama --data-dir .\data --prompt "Summarize the task using ledger_note."

# Gemini via x-goog-api-key header:
$env:GEMINI_API_KEY = "..."
.\target\debug\re-search.exe run --route api --provider gemini --data-dir .\data --prompt "Record a short note with ledger_note."

# Continue a verified --route api session (inherits last provider/model unless overridden):
.\target\debug\re-search.exe resume s-<id> --data-dir .\data --prompt "Follow up on the prior result."
.\target\debug\re-search.exe resume s-<id> --data-dir .\data --provider anthropic --prompt "Continue with Claude."
```

`re-search resume SESSION --prompt "..."` verifies the session hash chain and
blobs, checks policy/tool compatibility, acquires an atomic `.active.lock`,
reconstructs prior turns losslessly (stripping provider-specific blocks when
switching providers), and starts with a fresh `ApprovalStore` and worker epoch.

Exit codes for `run` and `resume`: `0` = completed, `1` = tampered, corrupt, or
unknown session on `resume`, `2` = usage, configuration, drift, active lock,
unsupported `--route cli` resume, or missing database, `3` = aborted
(init/transport/protocol error, `max_tokens`, or `max_turns`), `4` = legacy
unchained v0 database on `resume`, `5` = model refusal.

On first Windows worker run, the supervisor stages a read-only copy of the
Python runtime plus `sympy`, `mpmath`, and `numpy` into
`<data-dir>\runtime\<fingerprint>\` and creates a per-session AppContainer
profile (`re-search.worker.<session>`) with write access limited to
`sessions\<id>\scratch` and `sessions\<id>\pycache`. You will see a terminal
approval prompt showing the code, containment status, and scope
(`worker-session`: once approved, further `sympy_eval` calls reuse the grant
until that worker epoch exits or times out).

Inspect and verify sessions offline with:

```powershell
.\target\debug\re-search.exe sessions --data-dir .\data
.\target\debug\re-search.exe show s-<id> --data-dir .\data
.\target\debug\re-search.exe verify --data-dir .\data
.\target\debug\re-search.exe verify s-<id> --data-dir .\data
```

`re-search verify [SESSION] [--data-dir DIR]` checks SHA-256 chains, canonical
JSON, schema triggers, and blob integrity offline (`0` = valid, `1` = tampered /
unknown session, `2` = usage or missing database, `4` = legacy v0 schema).

Data lives in `%LOCALAPPDATA%\re-search` on Windows unless you pass `--data-dir`
(or set `RE_SEARCH_DATA`; on Linux, pass `--data-dir` or set `RE_SEARCH_DATA`).

## Requirements

- Rust (stable) on Linux or Windows (`crates/agent`, `policy`, `ledger`,
  `tools`, and `cli` `--route api` / `resume` without `--python`).
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
