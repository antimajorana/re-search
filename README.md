# research-core (working name)

A small Rust core for a local mathematics and theoretical-physics research
assistant on Windows 11. v0 is a first working slice: a model (through the
Claude Code CLI, on your own login) can ask to run SymPy code; you approve
each run; everything is written to an append-only ledger.

**v0 has no sandbox. Read [THREAT-MODEL.md](THREAT-MODEL.md) first.**

## Layout

| Crate | What it is |
| --- | --- |
| `crates/policy` | Pure allow / ask / deny engine. No I/O; depends on `serde` only. |
| `crates/ledger` | Append-only SQLite event store and SHA-256 blob directory. |
| `crates/tools` | Tool registry, router (policy, approval, ledger), single-use approvals. |
| `crates/worker` | Python worker under a Windows Job Object, JSON lines on two pipes. |
| `crates/mcp-server` | `core.exe`, the stdio MCP server (the only crate that uses `rmcp`). |
| `crates/cli` | `research-core.exe`, the supervisor that runs a session. |
| `python/worker.py` | The worker script (needs SymPy). |

## Build and test

```
cargo build
cargo test -p policy -p ledger -p tools -p cli
```

The worker test needs a Python with SymPy:

```
$env:RESEARCH_CORE_PYTHON = "C:\path\to\python.exe"
cargo test -p worker -- --nocapture
```

## Run a session

Run this in a real terminal, because the approval prompt reads from it:

```
$env:RESEARCH_CORE_PYTHON = "C:\path\to\python.exe"
.\target\debug\research-core.exe run --prompt "Use sympy_eval to integrate x*sin(x) with respect to x, then state the result."
```

You will see the model's request, then a prompt showing the exact code.
Type `y` to run it once. Afterwards:

```
.\target\debug\research-core.exe sessions
.\target\debug\research-core.exe show s-<id>
```

Data (ledger, blobs, per-session folders) lives in
`%LOCALAPPDATA%\research-core` unless you pass `--data-dir`. The model
defaults to `haiku`; change it with `--model`.

## Requirements

- Windows 11, Rust (stable, MSVC).
- Claude Code CLI, logged in. Developed against 2.1.318; the version in use
  is recorded in every session.
- Python 3 with SymPy.

## Licence

Dual-licensed under either of [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE), at your option. `rmcp` (Apache-2.0) is a
dependency of `mcp-server` only and is kept out of `policy` and `ledger`.
