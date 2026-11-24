# reSearch reference

## Providers

Open **Settings & Guide → Providers** and select a connected model in the composer.

| Option | Setup and usage |
| --- | --- |
| Anthropic, OpenAI, Gemini | Add an API key in Providers or set `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, or `GEMINI_API_KEY` (`GOOGLE_API_KEY` also works). Keys entered in the desktop app are saved in the local `providers.json`. API calls use the corresponding provider account. |
| Ollama | Start Ollama, then choose **Check again**. The default endpoint is `http://127.0.0.1:11434/v1`. Change it under **Advanced** or set `OLLAMA_API_KEY` when the server requires a key. |
| Sign in with ChatGPT | On Windows, sign in through the browser. The route uses your ChatGPT plan limits and stores tokens with Windows DPAPI. It uses a fixed Responses API endpoint. |
| Claude Code (`claude-cli`) | Install Claude Code and sign in with `claude.exe`. `core.exe` provides reSearch tools. To guard against alternate billing, the route checks for `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_OAUTH_TOKEN`, Claude Code's Bedrock, Vertex and Foundry flags, and the CLI's reported `apiKeySource`. `RE_SEARCH_ALLOW_CLAUDE_ENV_AUTH=1` bypasses that guard. |
| Google Antigravity (`antigravity`) | Install and sign in with `agy.exe`, then acknowledge the notice in Providers. You can withdraw the acknowledgement there. Antigravity governs its own built-in tools and third-party CLI automation may affect account terms. The route checks for `GEMINI_API_KEY`, `GOOGLE_GEMINI_BASE_URL`, and `AGY_ADC_AUTH`. `RE_SEARCH_ALLOW_AGY_ENV_AUTH=1` bypasses that guard. |

The first turn locks the session to its route family. Start a new session to switch between in-core providers (API or ChatGPT), Claude Code, and Antigravity. **Explain** and **Narrate Proof** use a connected in-core model, including when the main session uses a subscription CLI. The two subscription CLI routes replay a bounded transcript for follow-up turns. [Threat model](../THREAT-MODEL.md)

## Review modes and approvals

Choose the review mode under **Research options**.

| Mode | What the in-core route runs |
| --- | --- |
| Direct | A single orchestrator pass with tools. |
| Verified (default) | The orchestrator, calculation guard and independent model review, with up to three rounds. |
| Deep | Verified mode with strategist analysis after rejection to guide a revision. |
| Interactive | A proposed plan and a pause for your input before the main work, with verification enabled. |

Use **Conventions** in Research options to pin up to 2,000 characters of notation, units or assumptions for the active session. **Settings & Guide → Advanced** sets defaults for new sessions. In-core turns support live **Steer** notes. **Stop** applies to every route. **Explain** keeps its own thread and can pin a note to conventions or send it back to the orchestrator.

Under **Settings & Guide → Agents & Models**, assign models to the orchestrator, workers, verifiers, strategist, narrator and Explain. Compute, literature, Lean and explorer workers favor smaller connected models by default. Custom agents use one of the `read_only`, `compute` or `full_research` tool profiles and can be selected under **Research options → Agent**.

The research tool policy requires an approval decision for code execution (`sympy_eval`), workspace writes (`fs_write`, `latex_patch`, `latex_compile`) and network reads (`arxiv_search`, `literature_search`). Workspace reads may run without a prompt. Desktop **Settings & Guide → Advanced → Auto-approve SymPy, workspace writes & literature lookups** is on by default and records eligible approvals in the ledger. Turn it off to inspect requests and approve once, decline or grant access for the session when offered. Session grants cover network reads, workspace writes and code execution with active containment. Revoke them in the session header. The standalone CLI prompts in the terminal for approval-required calls. [Threat model](../THREAT-MODEL.md)

## Desktop launch and configuration

On Windows, launch **reSearch** from the Start menu or run `re-search-app.exe`. The native window uses WebView2. `re-search desktop` uses a sibling native executable when available on Windows, and falls back to `re-search ui` for browser-only flags or when that executable is absent. [Windows installation and packaging](../packaging/windows/README.md)

The native app resolves command-line flags first, followed by environment variables, `re-search-app.json` beside the executable, and defaults. Relative paths in that file resolve beside the executable.

| Flag | Environment variable | JSON key |
| --- | --- | --- |
| `--data-dir DIR` | `RE_SEARCH_DATA` | `data_dir` |
| `--workspace DIR` | — | `workspace` |
| `--python EXE` | `RE_SEARCH_PYTHON` | `python` |
| `--worker-script PY` | `RE_SEARCH_WORKER_SCRIPT` | `worker_script` |
| `--core-exe EXE` | `RE_SEARCH_CORE_EXE` | `core_exe` |
| `--claude EXE` | `RE_SEARCH_CLAUDE` | `claude` |
| `--agy EXE` | `RE_SEARCH_AGY` | `agy` |
| `--port PORT` | — | — |

The native app also recognizes legacy `RESEARCH_CORE_*` forms for `DATA`, `PYTHON`, `WORKER_SCRIPT`, and `CLAUDE`. The Windows data directory defaults to `%LOCALAPPDATA%\re-search`. The workspace and data directory must be disjoint. The installed app discovers `runtime/python.exe`, `python/worker.py` and `core.exe` relative to the executable. Port `0` selects a free port.

For headless browser automation, start the workbench without opening a browser and read the printed URL.

```sh
re-search ui --data-dir ./data --port 0 --no-open true --print-url-json true
```

The server binds to `127.0.0.1`. Protected API requests use `X-ReSearch-Token`. Same-origin event streams and downloads use scoped cookie authentication. Test endpoint overrides include `--api-base`, `--arxiv-base`, `--inspire-base`, `--crossref-base` and `--s2-base`. The API provider override disables live model catalog fetches and leaves the ChatGPT endpoint fixed.

## CLI

`re-search run` defaults to the Claude Code route. To use Anthropic, OpenAI, Gemini or Ollama, specify `--route api --provider PROVIDER` and configure credentials through the environment as needed. Use `--model MODEL` to select a model explicitly. On the API route and `resume`, pass an existing `--workspace DIR` or `--enable-research-tools true` for the research tool set. The minimal policy provides `sympy_eval` and `ledger_note`.

```sh
re-search run --route api --provider anthropic --workspace ./paper --data-dir ./data \
  --verification-mode true --calculation-mode true --prompt 'Derive and verify Lemma 1.'
re-search sessions --data-dir ./data
```

Set `source_session` to an ID from `sessions`, replace `6` with the ledger sequence at which to fork, and set `fork_session` to the ID returned by `fork` before running `diff`. `show` prints the session ledger as newline-delimited JSON.

```sh
re-search show "$source_session" --data-dir ./data
re-search resume "$source_session" --workspace ./paper --data-dir ./data \
  --prompt 'Continue with Lemma 2.'
re-search fork "$source_session" --at-seq 6 --assumption 'Assume k > 0' --data-dir ./data
re-search diff "$source_session" "$fork_session" --data-dir ./data
re-search export "$source_session" --format zip --out ./proof.zip --data-dir ./data
re-search verify "$source_session" --data-dir ./data
```

`export` supports `tex`, `html` and `zip`. `verify` without a session ID checks every session in the data directory. Exit codes are `0` for success, `1` for integrity errors or an unknown session during verification, `2` for usage, configuration or other startup errors, `3` for an aborted turn, `4` for a legacy unchained ledger, and `5` for model refusal on the in-core route.

## Exports and verification

Use **Canvas → LaTeX** to export a report as `.tex`, `.html` or **Reproducible ZIP**. **Explain** exports its own thread as Markdown or HTML. A session ZIP includes `report.tex`, `references.bib`, `report.html`, `ledger.json`, available sources and recorded computation artifacts, with provenance in `replay_manifest.json` and instructions in `REPRODUCE.md`.

Run `python3 run_all_checks.py --verify-only` in the extracted bundle to check archived file hashes. With no flag, the script replays recorded successful computations with your Python privileges. Review the code before executing it and use the dependency versions recorded in the bundle. `re-search verify` checks the stored ledger and artifacts. The proof view shows model review and linked Lean results as separate evidence.

## Development checks

In `crates/`, `desktop-app` hosts WebView2 and `desktop-server` runs the loopback HTTP and event stream. `agent` owns in-core turns, providers and review roles. `cli-route` supervises external provider CLIs. `policy` and `tools` enforce capabilities and approvals, `worker` and `research-tools` supply tool implementations, and `ledger` and `export` record and package the results.

```sh
cargo test --locked --workspace --no-fail-fast
node --test crates/desktop-server/tests/*.test.mjs
python -m unittest discover -s python
```

The Python checks use SymPy, NumPy and mpmath. For desktop, tablet and phone browser regression tests, install Python Playwright and Chromium, build the CLI with `cargo build --locked -p cli`, then run `python crates/desktop-server/tests/workbench_browser.py`. Pass `--chromium PATH` to use system Chromium or `--binary PATH` for a different CLI build. Run the Windows AppContainer probes with `cargo test --locked -p worker --test appcontainer_sandbox` and `RE_SEARCH_PYTHON` pointing to a Python installation with the numerical libraries. Windows also exercises DPAPI and vendor CLI process containment through mocked integrations.

reSearch draws on ideas from [OpenTorus](https://github.com/maximilianbehr/OpenTorus), [Open Math Harness](https://github.com/openmathmodel/open-math-harness) and [Solver Agent](https://github.com/starrfree/solver-agent).
