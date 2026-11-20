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
| `crates/mcp-server` | `core`: the stdio MCP server (registers `research-tools` automatically when `CORE_WORKSPACE` or session `sources/` exists). |
| `crates/cli` | `re-search`: CLI supervisor supporting `run`, `resume`, `ui`, `desktop`, `fork`, `diff`, `export`, `show`, `sessions`, and `verify`. |

## Build and test

Core unit, research-tool, agent, export, desktop-server, and CLI route tests run on both **Linux and Windows** against in-test loopback mock servers:

```bash
cargo build
cargo test -p policy -p ledger -p tools -p research-tools -p agent -p export -p desktop-server -p cli
```

Windows AppContainer containment probes require **Windows 11 + Python (SymPy and NumPy)**:

```powershell
$env:RE_SEARCH_PYTHON = "C:\path\to\python.exe"
cargo test -p worker -- --nocapture
cargo test -p cli --test contained_router -- --nocapture
```

## Desktop UI (`re-search ui` / `re-search desktop`)

Launch the local CSRF-protected desktop server with offline KaTeX rendering, live SSE proof-DAG inspector, LaTeX canvas, literature/sources drawer, counterfactual fork/diff, and isolated Side-Talk:

```bash
re-search ui --workspace ./paper --data-dir ./data
re-search desktop --workspace ./paper --data-dir ./data
```

Useful headless/automation flags: `--port PORT`, `--no-open true`, `--print-url-json true`, `--api-base URL`, `--arxiv-base URL`, `--inspire-base URL`, `--crossref-base URL`.

## CLI Research Mode, Fork, Diff, Export & Verify

Attach `--workspace DIR` (or `--enable-research-tools true`) on `--route api` / `resume` to enable the Stage 6-7 research policy (`v0.3-research`), console unified-diff approvals for `fs_write`/`latex_patch`, pinned conventions, and verifier/calculation guardrails:

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
