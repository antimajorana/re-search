# re:Search

A small Rust core for a local mathematics and theoretical-physics research
assistant on Windows 11. A model (through the Claude Code CLI, on your own
login) can ask to run SymPy/NumPy code inside a zero-capability Windows
AppContainer sandbox; everything is written to an append-only SQLite ledger.

**Read [THREAT-MODEL.md](THREAT-MODEL.md) for what AppContainer contains and what remains outside the boundary.**

## Layout

| Crate | What it is |
| --- | --- |
| `crates/policy` | Pure allow / ask / deny engine. No I/O; depends on `serde` only. |
| `crates/ledger` | Append-only SQLite event store and SHA-256 blob directory. |
| `crates/tools` | Tool registry, router (policy, approval, ledger), digest-bound and epoch-bound approvals. |
| `crates/worker` | Python worker under Windows `AppContainerSandbox` / `JobObjectSandbox`, `STARTUPINFOEXW` handle scoping, fingerprinted runtime staging, JSON lines on two pipes. |
| `crates/mcp-server` | `core.exe`, the stdio MCP server (the only crate that uses `rmcp`). |
| `crates/cli` | `re-search.exe`, the supervisor that runs a session and manages console approvals. |
| `python/worker.py` | The worker script (SymPy / NumPy). |

## Build and test

```powershell
cargo build
cargo test -p policy -p ledger -p tools -p cli
```

The worker containment probes and end-to-end contained router tests need a
Python with SymPy and NumPy:

```powershell
$env:RE_SEARCH_PYTHON = "C:\path\to\python.exe"
cargo test -p worker -- --nocapture
cargo test -p cli --test contained_router -- --nocapture
```

(`RESEARCH_CORE_*` environment variables are also accepted as fallbacks when
the corresponding `RE_SEARCH_*` variable is unset.)

## Run a session

Run this in a real terminal, because the approval prompt reads from it:

```powershell
$env:RE_SEARCH_PYTHON = "C:\path\to\python.exe"
.\target\debug\re-search.exe run --prompt "Use sympy_eval to integrate x*sin(x) with respect to x, then state the result."
```

On first run, `core.exe` stages a read-only copy of the Python runtime plus
`sympy`, `mpmath`, and `numpy` into `<data-dir>\runtime\<fingerprint>\` and
creates a per-session AppContainer profile (`re-search.worker.<session>`) with
write access limited to `sessions\<id>\scratch` and `sessions\<id>\pycache`.
You will see a terminal approval prompt showing the code, containment status,
and scope (`worker-session`: once approved, further `sympy_eval` calls reuse
the grant until that worker epoch exits or times out). Afterwards:

```powershell
.\target\debug\re-search.exe sessions
.\target\debug\re-search.exe show s-<id>
```

Data (ledger, blobs, staged runtimes, per-session folders, and bytecode caches)
lives in `%LOCALAPPDATA%\re-search` unless you pass `--data-dir`. The model
defaults to `haiku`; change it with `--model`.

## Requirements

- Windows 11, Rust (stable, MSVC).
- Claude Code CLI, logged in. Developed against `2.1.318`; the version in
  use is checked on start and at `system/init` (mismatches warn on stderr
  and are recorded in the ledger).
- Python 3 with SymPy and NumPy.

## Licence

Dual-licensed under either of [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE), at your option. `rmcp` (Apache-2.0) is a
dependency of `mcp-server` only and is kept out of `policy` and `ledger`.
