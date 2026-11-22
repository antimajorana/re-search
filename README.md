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
| `crates/agent` | Multi-provider HTTP transports (`Anthropic`, `OpenAI`, `Ollama`, `Gemini`, and the Responses-API `ChatGPT` plan route), Stage 7 guardrails, Step & Full Verifiers, Narrator, Side-Talk, and verified resume. |
| `crates/export` | Verified counterfactual `fork`, structural branch `diff`, and `.tex`/`.html`/`.zip` `export`. |
| `crates/desktop-server` | Loopback `127.0.0.1` HTTP + SSE desktop server (`std::net` + `httparse`, zero `tokio`), `UiApprover` diff broker, and embedded offline KaTeX UI. |
| `crates/desktop-app` | `re-search-app`: the re:Search window. One process runs the desktop server and one locked-down WebView2 window (`wry` + `tao`, Windows only); the token is delivered in a request header, never in a URL. The only crate with GUI dependencies. |
| `crates/mcp-server` | `core`: the stdio MCP server (registers `research-tools` automatically when `CORE_WORKSPACE`, `CORE_ENABLE_RESEARCH_TOOLS=1` or session `sources/` exists). |
| `crates/chatgpt-auth` | Sign in with ChatGPT (OpenAI's preview flow for open-source apps): PKCE sign-in with a one-shot `127.0.0.1` callback, RS256 ID-token checks against OpenAI's JWKS, DPAPI-encrypted token storage, serialized refresh with rotating refresh tokens, revocation. No `tokio`. |
| `crates/cli-route` | The vendor-CLI routes shared by the console and the desktop: drives the unmodified Claude Code CLI (`claude.exe`) and the labelled Antigravity CLI (`agy.exe`) — job object, prompt on stdin, init check, stream mapping, loopback approval relay with caller check, `core` tool set, environment-credential refusal and history replay. No `tokio`, no `rmcp`. |
| `crates/cli` | `re-search`: CLI supervisor supporting `run`, `resume`, `ui`, `desktop`, `fork`, `diff`, `export`, `show`, `sessions`, and `verify`. |

## Build and test

Core unit, research-tool, agent, export, desktop-server, desktop-app (pure config and lock-down rules; no display or WebView2 needed), and CLI route tests run on both **Linux and Windows** against in-test loopback mock servers:

```bash
cargo build
cargo test -p policy -p ledger -p tools -p research-tools -p agent -p chatgpt-auth -p export -p cli-route -p desktop-server -p desktop-app -p cli
```

The Sign in with ChatGPT flow tests (`chatgpt-auth` `tests/flow.rs` DPAPI suite, desktop-server `tests/chatgpt_route.rs`) run against a loopback mock of OpenAI's auth and API hosts (no network, no browser) and need Windows, because tokens are stored with DPAPI; non-Windows unit/integration tests verify fail-closed refusal and pure OAuth/JWT/callback logic on Linux too.

The subscription-CLI routes are tested with scripted fake CLIs (`cli_route::testkit`, compiled with `rustc` at test time); the desktop-server tests that spawn a CLI inside a Windows Job Object (`claude_cli_route`, `claude_cli_env`, `claude_cli_guards`, and the four turn-spawning tests in `antigravity_route`) run on Windows only, while the remaining `antigravity_route` gate/env/model/label tests run on both Linux and Windows. Desktop Markdown/LaTeX/CSS rendering and XSS escaping are tested against the vendored offline KaTeX `0.19.1` bundle via `node --test crates/desktop-server/tests/math_render.test.mjs` (also run by `cargo test -p desktop-server --test ui_math`).

Windows AppContainer containment probes require **Windows 11 + Python (SymPy and NumPy)**:

```powershell
$env:RE_SEARCH_PYTHON = "C:\path\to\python.exe"
cargo test -p worker -- --nocapture
cargo test -p cli --test contained_router -- --nocapture
```

## Desktop UI (`re-search-app`, `re-search desktop`, `re-search ui`)

The local CSRF-protected desktop server offers offline KaTeX rendering, a live SSE proof-DAG inspector, LaTeX canvas, literature/sources drawer, counterfactual fork/diff, and isolated Side-Talk. There are two ways to show it:

- **Its own window (Windows 11, WebView2 runtime).** `re-search-app.exe` starts the server and opens a window titled *re:Search*; closing the window stops the server. It needs no console and no browser (Microsoft Edge itself need not be installed, only the WebView2 Evergreen runtime). Only one instance runs per logon session; a second launch exits quietly. `re-search desktop` starts `re-search-app.exe` from its own directory when it is there and only `--data-dir`/`--workspace`/`--python`/`--worker-script`/`--port`/`--core-exe`/`--claude`/`--agy` were given; otherwise it behaves like `re-search ui`.
- **The default browser.** `re-search ui` opens `http://127.0.0.1:<port>/?launch=…` in the default browser using a single-use, 60-second launch nonce (never the real API token); the first page load redeems it for the token in `<meta>` and sets the stream cookie, and any replay returns `403`.

```bash
cargo build -p desktop-app -p cli          # target/debug/re-search-app.exe and re-search.exe
re-search-app --workspace ./paper --data-dir ./data --python C:\path\to\python.exe
re-search desktop --workspace ./paper --data-dir ./data
re-search ui --workspace ./paper --data-dir ./data
```

`re-search-app` settings, in order of precedence: the flags above (plus `--core-exe EXE` and `--claude EXE` for the Claude subscription route, `--agy EXE` for the Antigravity route); then `RE_SEARCH_PYTHON`, `RE_SEARCH_DATA`, `RE_SEARCH_WORKER_SCRIPT`, `RE_SEARCH_CLAUDE` (each with a `RESEARCH_CORE_*` fallback), plus `RE_SEARCH_CORE_EXE` and `RE_SEARCH_AGY`; then an optional `re-search-app.json` next to the executable (keys `python`, `worker_script`, `data_dir`, `workspace`, `core_exe`, `claude`, `agy`; relative paths are relative to that directory); then the defaults (`%LOCALAPPDATA%\re-search`, `python/worker.py` next to the executable when a Python is set, `core.exe` next to the executable, `claude.exe` from `PATH` or the npm install location, and `agy.exe` from `PATH` or WinGet's folders). A Start-menu shortcut gets no useful environment, so use the JSON file there. Startup errors appear in a message box. WebView2 keeps its (InPrivate) profile folder in `%LOCALAPPDATA%\re-search\webview`.

The server listens on `127.0.0.1` only. Every `/api/` route except `/api/health` needs the per-launch token in the `X-ReSearch-Token` header; a separate cookie opens only the event stream and export downloads. A workspace may not overlap the data directory. See [THREAT-MODEL.md](THREAT-MODEL.md).

Useful headless/automation flags: `--port PORT`, `--no-open true`, `--print-url-json true`, `--api-base URL`, `--arxiv-base URL`, `--inspire-base URL`, `--crossref-base URL`, `--core-exe EXE`, `--claude EXE`, `--agy EXE`.

### Connecting a model and picking one

Open **Connect** (bottom of the left rail). It lists every way re:Search can reach a model, each with a green *Connected* badge or one thing to do:

| Provider | How to connect | What re:Search checks |
| --- | --- | --- |
| ChatGPT (your subscription, Sign in with ChatGPT) | Press **Continue with ChatGPT**, then *Open in browser* (or *Copy link* and paste it into a browser), sign in to ChatGPT and allow re:Search to use your plan. Windows only for now. | Signed in, the ID token checked, and ChatGPT plan use (`chatgpt.tokens.use.direct`) granted; the status shows the account. |
| Claude (your subscription, via Claude Code) | Install Claude Code and sign in once by running `claude` in a terminal. No key. | `claude.exe` is found (`--claude`, `RE_SEARCH_CLAUDE`, `PATH`, npm location) and no environment credential would bill instead of the subscription; the status shows `claude --version`. |
| Antigravity (your Google plan) — vendor-sandboxed, not confined to re:Search tools | Install the Antigravity CLI and sign in once by running `agy` in a terminal. Then read the warning on its Connect row and press *I understand the risk: enable this route*. No key. | `agy.exe` is found (`--agy`, `RE_SEARCH_AGY`, `PATH`, WinGet's folders), no environment credential would replace your Google login, and you acknowledged the warning; `agy --version` runs only after acknowledgement. |
| Anthropic API | Paste an API key and press Save (or set `ANTHROPIC_API_KEY`). | A key is saved or set. |
| OpenAI API | Paste an API key and press Save (or set `OPENAI_API_KEY`). | A key is saved or set. |
| Google Gemini API | Paste an API key and press Save (or set `GEMINI_API_KEY` / `GOOGLE_API_KEY`). | A key is saved or set. |
| Ollama (models on this computer) | Install and start Ollama; press *Check again*. | Its model list answers at `http://127.0.0.1:11434` (or the address under Advanced). |

A saved key is never shown again; *Remove saved key* deletes it from `providers.json`. The status says whether the key a turn would use is the saved one or comes from the environment (`RE_SEARCH_API_KEY` takes precedence for every API provider in turns). A provider counts as connected only with its own key (saved, or its own variable): the generic `RE_SEARCH_API_KEY` is tied to a provider only when `RE_SEARCH_PROVIDER` names it, and it is never sent to list another provider's models. When re:Search is started with `--api-base` / `RE_SEARCH_API_BASE`, no model list is fetched (that one address would receive every provider's key); the dropdown then shows re:Search's built-in model names. Addresses (base URLs), role presets and pinned conventions are under **Advanced**.

The composer has one model dropdown, grouped by connected provider; choosing an entry sets both the provider and the model of the next message. The lists are live: `GET /api/models` asks each connected provider for its current models (Anthropic, OpenAI and Gemini `…/models`, Ollama `/api/tags`; OpenAI's list is narrowed to chat models), keeps them for 10 minutes (`?refresh=1` asks again; saving or removing a key or address clears them), and marks one recommended model per provider. A provider whose list call fails keeps its last list (or re:Search's default model) with a note that gives only the kind of failure (for example `auth (HTTP 401)` or `timeout`), never the provider's message. The Claude subscription offers the CLI's aliases (`fable`, `opus`, `sonnet`, `haiku`), which always mean the latest model of that name. With nothing connected the dropdown shows *No model connected* and a **Connect** button appears beside it. Once a session's first turn has started, it stays on its route: models on other routes are greyed out in the dropdown (start a new session to switch between subscription-CLI routes and API providers). The last choice is remembered per session in the browser's `localStorage` (no secret is stored there). Sessions on the Claude route show "runs Claude Code (your login)" beside the dropdown; ChatGPT-plan sessions show "runs on your ChatGPT plan" and run on re:Search's own (API) route. Both routes (`GET /api/connections`, `GET /api/models`) need the token like every other `/api/` route.

### Desktop sessions on your ChatGPT plan (`chatgpt`)

re:Search can use your ChatGPT subscription through **Sign in with ChatGPT**, OpenAI's official flow for open-source and locally hosted apps ([documentation](https://developers.openai.com/siwc/token-sharing-open-source)). OpenAI describes it as a **preview**; its rules and limits may change. Usage counts toward your ChatGPT plan's limits (for Plus, the five-hour limit is shared with every other app on your plan); review it at chatgpt.com/settings/usage.

- **Connect.** In **Connect**, press *Continue with ChatGPT*. re:Search shows the sign-in link with *Open in browser* (re:Search asks Windows to open it in your default browser, only when you click) and *Copy link*. Sign in, allow re:Search to use your plan, and the browser returns to a one-time address on `127.0.0.1`; the Connect row turns green. The link stays valid for 5 minutes. Declining plan use keeps the sign-in without using your plan and offers *Allow ChatGPT plan* (which repeats OAuth with `prompt=consent`) alongside *Sign out*.
- **Use.** Pick a model under *ChatGPT (your subscription, Sign in with ChatGPT)* in the model dropdown (the list is your account's, from `GET https://api.openai.com/v1/models`, entries marked for display). The model runs in re:Search's own loop through the Responses API (`store: false`, streaming), so tools, policy, approval cards and the ledger work exactly as with an API key. The composer shows "runs on your ChatGPT plan". A session is recorded as provider `chatgpt-responses` with the model; no token is ever recorded.
- **What is stored where.** `%LOCALAPPDATA%\re-search\chatgpt\registration.dpapi` (this computer's host ID and re:Search's issued client ID) and `credentials.dpapi` (the ID, access and refresh tokens, expiry and granted scopes), both encrypted with Windows DPAPI for your Windows user. Nothing is in `providers.json`, the browser, the ledger or the session folders. Tokens are renewed automatically shortly before they expire (within 5 minutes, or half the token's lifetime if that is shorter, and not before the server's `earliest_refresh_at` unless the token has expired), and retried once on HTTP 401, serialized across processes via `<data_dir>/chatgpt/.lock`. That is an operating-system file lock, released automatically when its holder exits; while another re:Search process holds it, the operation fails after 10 s with "another ChatGPT sign-in operation is in progress".
- **Sign out.** *Sign out* in Connect first deletes the tokens on this computer (the host ID and client ID are kept for the next sign-in), then revokes the session at OpenAI (discovering `revocation_endpoint` once from `https://auth.openai.com/.well-known/openid-configuration`, pinned to `auth.openai.com`, with the documented URL as fallback; at most 45 s in all, with no lock held, so the Connect panel and other sessions are not blocked). If OpenAI does not confirm, Connect says so; you can also disconnect re:Search in ChatGPT Settings.
- **Limits.** Windows only for now (other systems refuse to store the tokens). The tokens go only to `auth.openai.com` and `api.openai.com`: `--api-base` and saved addresses do not apply to this route, and it takes no API key. The console (`re-search run --route api`) does not offer it.

### Desktop sessions on your Claude subscription (`claude-cli`)

A desktop session can run on the Claude subscription you are signed into in the Claude Code CLI instead of an API key: create the session or send the message with `{"provider": "claude-cli", "model": "<alias>"}` (default model `sonnet`). re:Search then starts the unmodified `claude.exe` for each turn, exactly as `re-search run --route cli` does on the console: the model gets only re:Search's tools (served by `core.exe` over MCP, with the session's workspace and the research policy `v0.4-research`), every tool call goes through the router in `core.exe`, and approvals appear as the usual approval cards. The UI receives the same events as on the API route. Such sessions are recorded as route `claude-code-cli`, "external loop, partial provenance", and `GET /api/sessions/{id}` reports the route. A session stays on the route it started with.

- **Billing safety.** If `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_OAUTH_TOKEN`, `CLAUDE_CODE_USE_BEDROCK`, `CLAUDE_CODE_USE_VERTEX` or `CLAUDE_CODE_USE_FOUNDRY` is set in re:Search's environment, the turn is refused with an error naming the variable, because the CLI would use it instead of your subscription. Unset it, use the API-key route, or set `RE_SEARCH_ALLOW_CLAUDE_ENV_AUTH=1` to proceed anyway. The turn is also aborted when Claude Code itself reports an `apiKeySource` other than `"none"` (for example an `apiKeyHelper` in managed settings), with the same opt-in. The same rules apply to `re-search run --route cli`. A session whose first turn was refused before it started can still be used with an API model.
- **Narrate proof and side questions** need an in-core API-route provider (Anthropic, OpenAI, Gemini, Ollama, or signed-in ChatGPT): they use the model picked in the dropdown when it is on the API route; with a subscription-CLI model (`claude-cli` or `antigravity`) picked they use the first configured API provider, and say so when there is none.
- **Prompt on stdin.** The prompt is written to the CLI's standard input, never put on its command line (both routes).
- **Later turns.** The CLI keeps no state between turns (`--no-session-persistence`), so the second and later turns start with a delimited transcript of this session's earlier messages and final replies, read from the ledger (the most recent turns up to about 24,000 characters; the prompt says when older turns were left out). Tool calls and results are not replayed.
- **Stop.** `POST /api/sessions/{id}/stop` (token required) kills the running turn's process tree and records `session.abort`; closing the app window does the same for every running turn.

### Desktop sessions on your Google Antigravity plan (`antigravity`) — vendor-sandboxed, not confined to re:Search tools

A desktop session can run on the Google plan you are signed into in the Antigravity CLI (`agy`). **Read this before you use it.**

- **Terms risk.** Google's terms for consumer plans may treat driving the Antigravity CLI from third-party software such as re:Search as unsupported or as a breach, which could put your Google account at risk. re:Search does not decide this for you: the route stays off until you acknowledge the warning on its Connect row (stored in `ui_settings.json` as `antigravity_ack`; *Withdraw acknowledgement* turns it off again and stops any running Antigravity turn). `agy.exe` is not spawned (`--version` or `models`) until you acknowledge. The builder of this route read no Google terms document; the warning paraphrases what the user was told and quotes nothing.
- **What "vendor-sandboxed, not confined to re:Search tools" means.** On the Claude route the model has *only* re:Search's tools. Here the Antigravity CLI keeps its own built-in tools (terminal commands, file reads and writes, web fetches, a browser) and they cannot be removed. re:Search passes a per-session `--settings` object that selects Default agent mode (nothing auto-approved; not `auto`), `toolPermission: request-review`, the terminal sandbox, no access outside the workspace, **denies** `command(*)`, `read_file(*)`, `write_file(*)`, `read_url(*)`, `execute_url(*)`, and **allows** only `mcp(core/*)`; it also passes `--sandbox`. Whether those rules hold is up to the CLI. Concretely, these re:Search guarantees do **not** hold for anything the CLI runs itself: re:Search's router and policy, its approval cards, its ledger record of the call and result, and the AppContainer around re:Search's Python worker. re:Search's own tools (served by `core.exe`) still go through the router, the policy `v0.4-research` and your approval cards, exactly as on the Claude route.
- **Seen, never hidden.** Every built-in or non-core tool step the CLI reports (`step_update` for anything outside the verified `core` tool set) is recorded as a `vendor.builtin_tool` ledger event and shown as a warning line in the conversation, whether the CLI reports it running, denied, failed or finished. re:Search only sees what the CLI prints.
- **Other MCP servers.** There is no per-session MCP flag. re:Search writes `sessions/<id>/agy/.agents/mcp_config.json` (the CLI's workspace config; it holds the turn's session token, is written only after version/stop checks, and is deleted when the turn ends, is stopped or fails, and swept when left behind) naming only `core`. MCP servers from your global Antigravity configuration still load; their tools have no rule in re:Search's settings and default to *Ask*, which print mode soft-denies, unless your own Antigravity settings allow them.
- **Use.** Pick a model under *Antigravity (your Google plan) — vendor-sandboxed, not confined to re:Search tools* in the dropdown (from `agy models`, cached for 10 minutes; a small built-in list, marked as such, if it never answered). The composer shows "runs the Antigravity CLI (your Google login) — vendor-sandboxed, not confined to re:Search tools" (and keeps the confinement label when reopening the session even if Antigravity is later withdrawn or disconnected). The prompt goes on the CLI's standard input as one stream-json `user` event (`--input-format stream-json`; never `-p`). Sessions are recorded as route `antigravity`, confinement `vendor-sandboxed`, with the exact flags; `GET /api/sessions/{id}` reports the route, confinement and label. A session stays on its route; later turns replay a bounded transcript, as on the Claude route; *Stop* works the same way. Desktop only: the console (`re-search run`) does not offer this route.
- **Credentials.** re:Search never reads or copies the CLI's login. If `GEMINI_API_KEY`, `GOOGLE_GEMINI_BASE_URL` or `AGY_ADC_AUTH` is set in re:Search's environment, the turn is refused (naming the variable, never its value), because the CLI could then use an API key, another endpoint or Application Default Credentials instead of your plan; `RE_SEARCH_ALLOW_AGY_ENV_AUTH=1` proceeds anyway. Note that `GEMINI_API_KEY` is also the Gemini API route's key variable.
- **Updates.** The Antigravity CLI updates itself (observed while building this route: 1.5.13 became 1.5.14 after `agy models`). The route was written against 1.5.14; a different version is recorded as `cli.version_warning`.
- **Not verified:** no live turn was run (no prompt was ever sent to a model through `agy`); every test uses a scripted fake `agy` (the Windows-only `JobObjectSandbox` and DPAPI test suites have been run on Windows 11; WebView2 has no display test). See the "Not verified" lists in [THREAT-MODEL.md](THREAT-MODEL.md).

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
