# re:Search

A small, tamper-evident Rust research core and loopback desktop workbench for mathematics and theoretical physics. Symbolic/numerical checks run inside a zero-capability Windows 11 AppContainer sandbox (`sympy_eval`), while workspace inspection, document ingestion, arXiv/literature lookup, deterministic TeX/citation auditing, hash-chained proof graphs, counterfactual forking, reproducible exports, and the embedded Gemini-inspired desktop UI are orchestrated by the core supervisor.

**Read [THREAT-MODEL.md](THREAT-MODEL.md) for what AppContainer contains, how CSRF/workspace approvals work, and what remains outside the boundary.**

## Layout

| Crate | What it is |
| --- | --- |
| `crates/policy` | Pure allow / ask / deny engine (`ReadWorkspace`, `WriteWorkspace`, `NetworkRead`, `ExecuteCode`). No I/O; `serde` only. |
| `crates/ledger` | Append-only, SHA-256 hash-chained SQLite event store (`PRAGMA user_version = 3`) and content-addressed blob directory. |
| `crates/tools` | Tool registry, policy router, digest- and epoch-bound approvals. |
| `crates/research-tools` | Stage 6 tools: `fs_list`, `fs_read`, `doc_ingest`, `fs_write`, `latex_patch`, `latex_compile`, `tex_audit`, `arxiv_search`, `literature_search`, `ledger_append`, `ledger_supersede`, `ledger_query`, `check_expression_sanity`. |
| `crates/worker` | Python worker (`sympy`, `mpmath`, `numpy`) under Windows `AppContainerSandbox` / `JobObjectSandbox`. |
| `crates/agent` | Multi-provider HTTP transports (`Anthropic`, `OpenAI`, `Ollama`, `Gemini`), Stage 7 guardrails, Step & Full Verifiers, Narrator, Side-Talk, and verified resume. |
| `crates/export` | Verified counterfactual `fork`, structural branch `diff`, and `.tex`/`.html`/`.zip` `export`. |
| `crates/desktop-server` | Loopback `127.0.0.1` HTTP + SSE desktop server (`std::net` + `httparse`, zero `tokio`), `UiApprover` diff broker, and embedded offline KaTeX UI. |
| `crates/desktop-app` | `re-search-app`: the re:Search window. One process runs the desktop server and one locked-down WebView2 window (`wry` + `tao`, Windows only); the token is delivered in a request header, never in a URL. The only crate with GUI dependencies. |
| `crates/mcp-server` | `core`: the stdio MCP server (registers `research-tools` automatically when `CORE_WORKSPACE` or session `sources/` exists). |
| `crates/cli` | `re-search`: CLI supervisor supporting `run`, `resume`, `ui`, `desktop`, `fork`, `diff`, `export`, `show`, `sessions`, and `verify`. |

## Build and test

Core unit, research-tool, agent, export, desktop-server, desktop-app (pure config and lock-down rules; no display or WebView2 needed), and CLI route tests run on both **Linux and Windows** against in-test loopback mock servers:

```bash
cargo build
cargo test -p policy -p ledger -p tools -p research-tools -p agent -p export -p desktop-server -p desktop-app -p cli
```

Windows AppContainer containment probes require **Windows 11 + Python (SymPy and NumPy)**:

```powershell
$env:RE_SEARCH_PYTHON = "C:\path\to\python.exe"
cargo test -p worker -- --nocapture
cargo test -p cli --test contained_router -- --nocapture
```

## Desktop UI (`re-search-app`, `re-search desktop`, `re-search ui`)

The local CSRF-protected desktop server offers offline KaTeX rendering, a live SSE proof-DAG inspector, LaTeX canvas, literature/sources drawer, counterfactual fork/diff, and isolated Side-Talk. There are two ways to show it:

- **Its own window (Windows 11, WebView2 runtime).** `re-search-app.exe` starts the server and opens a window titled *re:Search*; closing the window stops the server. It needs no console and no browser (Microsoft Edge itself need not be installed, only the WebView2 Evergreen runtime). Only one instance runs per logon session; a second launch exits quietly. `re-search desktop` starts `re-search-app.exe` from its own directory when it is there and only `--data-dir`/`--workspace`/`--python`/`--worker-script`/`--port` were given; otherwise it behaves like `re-search ui`.
- **The default browser.** `re-search ui` opens `http://127.0.0.1:<port>/?token=…` in the default browser, as before.

```bash
cargo build -p desktop-app -p cli          # target/debug/re-search-app.exe and re-search.exe
re-search-app --workspace ./paper --data-dir ./data --python C:\path\to\python.exe
re-search desktop --workspace ./paper --data-dir ./data
re-search ui --workspace ./paper --data-dir ./data
```

`re-search-app` settings, in order of precedence: the flags above; then `RE_SEARCH_PYTHON`, `RE_SEARCH_DATA`, `RE_SEARCH_WORKER_SCRIPT` (with the `RESEARCH_CORE_*` fallbacks); then an optional `re-search-app.json` next to the executable (keys `python`, `worker_script`, `data_dir`, `workspace`; relative paths are relative to that directory); then the defaults (`%LOCALAPPDATA%\re-search`, and `python/worker.py` next to the executable when a Python is set). A Start-menu shortcut gets no useful environment, so use the JSON file there. Startup errors appear in a message box. WebView2 keeps its (InPrivate) profile folder in `%LOCALAPPDATA%\re-search\webview`.

The server listens on `127.0.0.1` only. Every `/api/` route except `/api/health` needs the per-launch token in the `X-ReSearch-Token` header; a separate cookie opens only the event stream and export downloads. A workspace may not overlap the data directory. See [THREAT-MODEL.md](THREAT-MODEL.md).

Useful headless/automation flags: `--port PORT`, `--no-open true`, `--print-url-json true`, `--api-base URL`, `--arxiv-base URL`, `--inspire-base URL`, `--crossref-base URL`.

## CLI Research Mode, Fork, Diff, Export & Verify

Attach `--workspace DIR` (or `--enable-research-tools true`) on `--route api` / `resume` to enable the Stage 6-7 research policy (`v0.4-research`), console unified-diff approvals for `fs_write`/`latex_patch`, a per-call approval for every model-initiated `arxiv_search`/`literature_search` (showing the query), pinned conventions, and verifier/calculation guardrails:

```bash
re-search run --route api --provider anthropic --workspace ./paper --data-dir ./data \
  --verification-mode true --calculation-mode true --prompt "Derive and verify Lemma 1."
re-search resume s-<id> --workspace ./paper --data-dir ./data --prompt "Continue with Lemma 2."
re-search fork s-<id> --at-seq 6 --assumption "Assume k > 0" --data-dir ./data
re-search diff s-<id> s-<fork_id> --data-dir ./data
re-search export s-<id> --format zip --out ./out/proof.zip --data-dir ./data
re-search verify s-<id> --data-dir ./data
```

When neither `--workspace` nor `--enable-research-tools true` is passed on `--route api`, `re-search` uses the minimal `v0.2` built-in policy (`sympy_eval`, `ledger_note`).

Exit codes: `0` = completed, `1` = tampered/missing/corrupt on `resume`/`verify`, `2` = CLI usage, config, drift, or lock error, `3` = aborted turn/guardrail/transport error, `4` = legacy unchained schema, `5` = model refusal.

## Licence

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE). `rmcp` (Apache-2.0) is used strictly in `crates/mcp-server` and stays excluded from all other crates.
