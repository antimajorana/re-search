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
| `crates/mcp-server` | `core`: the stdio MCP server (registers `research-tools` automatically when `CORE_WORKSPACE`, `CORE_ENABLE_RESEARCH_TOOLS=1` or session `sources/` exists). |
| `crates/cli-route` | The vendor-CLI route shared by the console and the desktop: drives the unmodified Claude Code CLI (job object, prompt on stdin, `system/init` check, stream mapping), the loopback approval relay, the tool set `core` serves, the environment-credential refusal and history replay. No `tokio`, no `rmcp`. |
| `crates/cli` | `re-search`: CLI supervisor supporting `run`, `resume`, `ui`, `desktop`, `fork`, `diff`, `export`, `show`, `sessions`, and `verify`. |

## Build and test

Core unit, research-tool, agent, export, desktop-server, desktop-app (pure config and lock-down rules; no display or WebView2 needed), and CLI route tests run on both **Linux and Windows** against in-test loopback mock servers:

```bash
cargo build
cargo test -p policy -p ledger -p tools -p research-tools -p agent -p export -p cli-route -p desktop-server -p desktop-app -p cli
```

The Claude subscription route is tested with a scripted fake CLI (`cli_route::testkit`, compiled with `rustc` at test time); those desktop-server tests (`claude_cli_route`, `claude_cli_env`) run on Windows only, because the CLI is started in a Windows job object.

Windows AppContainer containment probes require **Windows 11 + Python (SymPy and NumPy)**:

```powershell
$env:RE_SEARCH_PYTHON = "C:\path\to\python.exe"
cargo test -p worker -- --nocapture
cargo test -p cli --test contained_router -- --nocapture
```

## Desktop UI (`re-search-app`, `re-search desktop`, `re-search ui`)

The local CSRF-protected desktop server offers offline KaTeX rendering, a live SSE proof-DAG inspector, LaTeX canvas, literature/sources drawer, counterfactual fork/diff, and isolated Side-Talk. There are two ways to show it:

- **Its own window (Windows 11, WebView2 runtime).** `re-search-app.exe` starts the server and opens a window titled *re:Search*; closing the window stops the server. It needs no console and no browser (Microsoft Edge itself need not be installed, only the WebView2 Evergreen runtime). Only one instance runs per logon session; a second launch exits quietly. `re-search desktop` starts `re-search-app.exe` from its own directory when it is there and only `--data-dir`/`--workspace`/`--python`/`--worker-script`/`--port`/`--core-exe`/`--claude` were given; otherwise it behaves like `re-search ui`.
- **The default browser.** `re-search ui` opens `http://127.0.0.1:<port>/?token=…` in the default browser, as before.

```bash
cargo build -p desktop-app -p cli          # target/debug/re-search-app.exe and re-search.exe
re-search-app --workspace ./paper --data-dir ./data --python C:\path\to\python.exe
re-search desktop --workspace ./paper --data-dir ./data
re-search ui --workspace ./paper --data-dir ./data
```

`re-search-app` settings, in order of precedence: the flags above (plus `--core-exe EXE` and `--claude EXE` for the Claude subscription route); then `RE_SEARCH_PYTHON`, `RE_SEARCH_DATA`, `RE_SEARCH_WORKER_SCRIPT`, `RE_SEARCH_CORE_EXE`, `RE_SEARCH_CLAUDE` (with the `RESEARCH_CORE_*` fallbacks); then an optional `re-search-app.json` next to the executable (keys `python`, `worker_script`, `data_dir`, `workspace`, `core_exe`, `claude`; relative paths are relative to that directory); then the defaults (`%LOCALAPPDATA%\re-search`, `python/worker.py` next to the executable when a Python is set, `core.exe` next to the executable, and `claude.exe` from `PATH` or the npm install location). A Start-menu shortcut gets no useful environment, so use the JSON file there. Startup errors appear in a message box. WebView2 keeps its (InPrivate) profile folder in `%LOCALAPPDATA%\re-search\webview`.

The server listens on `127.0.0.1` only. Every `/api/` route except `/api/health` needs the per-launch token in the `X-ReSearch-Token` header; a separate cookie opens only the event stream and export downloads. A workspace may not overlap the data directory. See [THREAT-MODEL.md](THREAT-MODEL.md).

Useful headless/automation flags: `--port PORT`, `--no-open true`, `--print-url-json true`, `--api-base URL`, `--arxiv-base URL`, `--inspire-base URL`, `--crossref-base URL`, `--core-exe EXE`, `--claude EXE`.

### Connecting a model and picking one

Open **Connect** (bottom of the left rail). It lists every way re:Search can reach a model, each with a green *Connected* badge or one thing to do:

| Provider | How to connect | What re:Search checks |
| --- | --- | --- |
| Claude (your subscription, via Claude Code) | Install Claude Code and sign in once by running `claude` in a terminal. No key. | `claude.exe` is found (`--claude`, `RE_SEARCH_CLAUDE`, `PATH`, npm location) and no environment credential would bill instead of the subscription; the status shows `claude --version`. |
| Anthropic API | Paste an API key and press Save (or set `ANTHROPIC_API_KEY`). | A key is saved or set. |
| OpenAI API | Paste an API key and press Save (or set `OPENAI_API_KEY`). | A key is saved or set. |
| Google Gemini API | Paste an API key and press Save (or set `GEMINI_API_KEY` / `GOOGLE_API_KEY`). | A key is saved or set. |
| Ollama (models on this computer) | Install and start Ollama; press *Check again*. | Its model list answers at `http://127.0.0.1:11434` (or the address under Advanced). |

A saved key is never shown again; *Remove saved key* deletes it from `providers.json`. The status says whether the key a turn would use is the saved one or comes from the environment (`RE_SEARCH_API_KEY` takes precedence for every API provider in turns). A provider counts as connected only with its own key (saved, or its own variable): the generic `RE_SEARCH_API_KEY` is tied to a provider only when `RE_SEARCH_PROVIDER` names it, and it is never sent to list another provider's models. When re:Search is started with `--api-base` / `RE_SEARCH_API_BASE`, no model list is fetched (that one address would receive every provider's key); the dropdown then shows re:Search's built-in model names. Addresses (base URLs), role presets and pinned conventions are under **Advanced**.

The composer has one model dropdown, grouped by connected provider; choosing an entry sets both the provider and the model of the next message. The lists are live: `GET /api/models` asks each connected provider for its current models (Anthropic, OpenAI and Gemini `…/models`, Ollama `/api/tags`; OpenAI's list is narrowed to chat models), keeps them for 10 minutes (`?refresh=1` asks again; saving or removing a key or address clears them), and marks one recommended model per provider. A provider whose list call fails keeps its last list (or re:Search's default model) with a note that gives only the kind of failure (for example `auth (HTTP 401)` or `timeout`), never the provider's message. The Claude subscription offers the CLI's aliases (`fable`, `opus`, `sonnet`, `haiku`), which always mean the latest model of that name. With nothing connected the dropdown shows *No model connected* and a **Connect** button appears beside it. Once a session's first turn has started, it stays on its route: the other route's models are greyed out in the dropdown (start a new session to switch between the subscription and an API provider). The last choice is remembered per session in the browser's `localStorage` (no secret is stored there). Sessions on the subscription route show "runs Claude Code (your login)" beside the dropdown. Both routes (`GET /api/connections`, `GET /api/models`) need the token like every other `/api/` route.

### Desktop sessions on your Claude subscription (`claude-cli`)

A desktop session can run on the Claude subscription you are signed into in the Claude Code CLI instead of an API key: create the session or send the message with `{"provider": "claude-cli", "model": "<alias>"}` (default model `sonnet`). re:Search then starts the unmodified `claude.exe` for each turn, exactly as `re-search run --route cli` does on the console: the model gets only re:Search's tools (served by `core.exe` over MCP, with the session's workspace and the research policy `v0.4-research`), every tool call goes through the router in `core.exe`, and approvals appear as the usual approval cards. The UI receives the same events as on the API route. Such sessions are recorded as route `claude-code-cli`, "external loop, partial provenance", and `GET /api/sessions/{id}` reports the route. A session stays on the route it started with.

- **Billing safety.** If `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_OAUTH_TOKEN`, `CLAUDE_CODE_USE_BEDROCK`, `CLAUDE_CODE_USE_VERTEX` or `CLAUDE_CODE_USE_FOUNDRY` is set in re:Search's environment, the turn is refused with an error naming the variable, because the CLI would use it instead of your subscription. Unset it, use the API-key route, or set `RE_SEARCH_ALLOW_CLAUDE_ENV_AUTH=1` to proceed anyway. The turn is also aborted when Claude Code itself reports an `apiKeySource` other than `"none"` (for example an `apiKeyHelper` in managed settings), with the same opt-in. The same rules apply to `re-search run --route cli`. A session whose first turn was refused before it started can still be used with an API model.
- **Narrate proof and side questions** need an API-key provider for now: they use the model picked in the dropdown when it is an API model; with a Claude subscription model picked they use the first API provider that has a key, and say so when there is none.
- **Prompt on stdin.** The prompt is written to the CLI's standard input, never put on its command line (both routes).
- **Later turns.** The CLI keeps no state between turns (`--no-session-persistence`), so the second and later turns start with a delimited transcript of this session's earlier messages and final replies, read from the ledger (the most recent turns up to about 24,000 characters; the prompt says when older turns were left out). Tool calls and results are not replayed.
- **Stop.** `POST /api/sessions/{id}/stop` (token required) kills the running turn's process tree and records `session.abort`; closing the app window does the same for every running turn.

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

When neither `--workspace` nor `--enable-research-tools true` is passed on `--route api`, `re-search` uses the minimal `v0.2` built-in policy (`sympy_eval`, `ledger_note`). `--route cli` (the default; your Claude subscription through the unmodified Claude Code CLI) always uses the `v0.2` tools and policy and passes the prompt on stdin.

**Console behaviour change (Stage 9b):** `re-search run --route cli` now refuses to start while `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_OAUTH_TOKEN`, `CLAUDE_CODE_USE_BEDROCK`, `CLAUDE_CODE_USE_VERTEX` or `CLAUDE_CODE_USE_FOUNDRY` is set (earlier versions ignored them, and the Claude CLI then billed that credential instead of your subscription). It exits with code `2` before starting `claude.exe`, naming the variable (never its value) on stderr. Unset the variable, use `--route api`, or set `RE_SEARCH_ALLOW_CLAUDE_ENV_AUTH=1` to proceed anyway. A turn whose `system/init` reports an `apiKeySource` other than `"none"` is aborted (exit `3`) unless the same opt-in is set.

Exit codes: `0` = completed, `1` = tampered/missing/corrupt on `resume`/`verify`, `2` = CLI usage, config, drift, or lock error, `3` = aborted turn/guardrail/transport error, `4` = legacy unchained schema, `5` = model refusal.

## Licence

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE). `rmcp` (Apache-2.0) is used strictly in `crates/mcp-server` and stays excluded from all other crates.
