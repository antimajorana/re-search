# reSearch

> **Core philosophy:** *Models propose; deterministic computations, independent verifiers, and tamper-evident ledgers decide.*

**reSearch** is a small, tamper-evident Rust research core and loopback desktop workbench built for mathematics and theoretical physics. Instead of trusting a single chat thread to write proofs, run uncontained scripts on your machine, and grade its own homework, reSearch separates **generating ideas** from **executing code**, **auditing manuscripts**, and **certifying steps**.

Every Python/SymPy calculation runs inside a **zero-capability Windows 11 AppContainer sandbox** (`sympy_eval`), while local `.tex`/`.bib` workspace inspection, arXiv/literature ingestion, deterministic TeX/citation auditing, Lean 4 type-checking, hash-chained Proof DAGs, counterfactual branching, interactive human checkpoints, and the offline KaTeX UI are supervised by the Rust core.

**Security boundary first:** Read [THREAT-MODEL.md](THREAT-MODEL.md) for what the AppContainer isolates, how CSRF and workspace approvals work, and what remains outside the boundary.

---

## Contents

1. [Why reSearch Exists (In Plain English)](#1-why-research-exists-in-plain-english)
2. [Quick Start (3 Minutes)](#2-quick-start-3-minutes)
3. [Mental Model: Follow One Derivation Through All 5 Layers](#3-mental-model-follow-one-derivation-through-all-5-layers)
4. [Working Interactively: Modes, Live Steering & Explain](#4-working-interactively-modes-live-steering--explain)
5. [Meet the Agents (Built-in Roles & Custom Subagents)](#5-meet-the-agents-built-in-roles--custom-subagents)
6. [Tool & Computation Approvals](#6-tool--computation-approvals)
7. [Connecting Providers, Billing Safety & Confinement](#7-connecting-providers-billing-safety--confinement)
8. [Desktop Launch, Configuration & Headless Flags](#8-desktop-launch-configuration--headless-flags)
9. [CLI Reference (`run`, `fork`, `diff`, `export`, `verify`)](#9-cli-reference-run-fork-diff-export-verify)
10. [Codebase Map (13 Crates) & Running Tests](#10-codebase-map-13-crates--running-tests)
11. [Licence](#11-licence)

---

## 1. Why reSearch Exists (In Plain English)

Ordinary AI coding/chat assistants fail on serious mathematical and physics derivations in four predictable ways:

| Failure Mode in Standard Chats | How reSearch Solves It |
| --- | --- |
| **Context bloat:** long SymPy tracebacks, huge arXiv `.tex` sources, and failed Lean attempts pollute the main conversation. | **Context-isolated Workers:** heavy SymPy retries, paper reading, Lean proofs, and parallel explorations run in clean short-lived worker contexts and return only concise verified summaries + `e-N` Proof-DAG IDs. |
| **Self-grading bias:** the same context that dropped a minus sign also "verifies" its own final answer. | **Zero-bias Stateless Verifiers:** in `Verified`, `Deep`, and `Interactive` modes, a fresh Verifier sees only the prompt, the Proof DAG, `<pinned_conventions>`, Lean results, and deterministic `tex_audit` XML — never the solver's scratch reasoning. |
| **"Evidence" dressed up as "proof":** saying *"SymPy confirmed / q.e.d."* when a check failed or never ran. | **Typed Proof DAG + Honesty Linter:** symbolic/numerical checks, literature facts, and proof steps have separate node types, and `honesty.lint` cross-checks final prose against actual ledger & artifact evidence. |
| **Silent execution / local edits:** arbitrary Python or file writes happen behind your back. | **Fail-closed Policy + AppContainer:** every tool call crosses the Rust router (`policy`), is SHA-256 hash-chained into `ledger.db`, and Python runs inside a zero-capability AppContainer (no network, no user files, no DPAPI, no env secrets). |

---

## 2. Quick Start (3 Minutes)

1. **Launch the desktop window:**
   Run `re-search-app.exe` (or `re-search desktop` / Start menu shortcut **reSearch** on Windows 11, or `re-search ui` to open a single-use `http://127.0.0.1:<port>/?launch=...` tab in your default browser).
2. **Connect at least one provider:**
   Click **Settings & Guide → Providers** (bottom-left) and connect Sign in with ChatGPT, Claude Code, Anthropic/OpenAI/Gemini API, local Ollama, or (if explicitly acknowledged) Google Antigravity.
3. **Set your math conventions & rigor:**
   Click **Conventions** next to the composer to pin metric signatures, natural units, or Fourier prefactors, and pick a mode: **`Direct`**, **`✓ Verified`** (recommended default), **`✦ Deep`**, or **`Interactive`**.
4. **Optionally attach a paper folder:**
   Click **Workspace** to point the session at a local `.tex`/`.bib` directory (or drop `.tex`, `.bib`, `.pdf`, `.zip` files directly with **Attach**).
5. **Run & inspect:**
   Watch the live pipeline stepper (**Orchestrator → Proof DAG → Verifier → Strategist**), inspect equations in **Canvas → LaTeX / Proof DAG**, ask clarifying questions safely in **Explain**, or steer mid-turn.

---

## 3. Mental Model: Follow One Derivation Through All 5 Layers

reSearch synthesizes three architectural patterns — **OpenTorus** (fail-closed policy clamps, AppContainer sandbox, loopback WebView2 isolation, SHA-256 ledger), **open-math-harness** (Orchestrator → Worker delegation, Ultramath parallel exploration, Lean 4 anti-cheat), and **solver-agent** (Campaign Phases, artifact-aware `honesty.lint`, zero-bias verification):

```text
┌────────────────────────────────────────────────────────────────────────────┐
│ 1. Desktop Shell & Loopback Server (desktop-app + desktop-server)          │
│    WebView2 locked to 127.0.0.1 (header CSRF + Origin) · Offline KaTeX UI  │
│    Live Pipeline Bar · HITL Cards · Proof DAG / LaTeX / Sources / Explain  │
└───────────────────────────────────┬────────────────────────────────────────┘
                                    │ spawns / supervises turn
┌───────────────────────────────────▼────────────────────────────────────────┐
│ 2. Orchestrator, Workers & Zero-Bias Verification (agent / cli-route)      │
│    Phases  │ INGEST -> EXPLORE -> COMPUTE -> SYNTHESIZE -> VERIFY ->       │
│            │ REVISE -> COMPLETE                                            │
│    Workers │ Compute · Literature · Lean Prover · Parallel Explorer (2-4x) │
│            │ + Custom Subagents (isolated contexts; cannot finalize)       │
│    Gates   │ lean_check (anti-cheat) · honesty.lint (evidence != proof)    │
│            │ Stateless Step/Full Verifiers (<=3x) + Strategist (Deep mode) │
└───────────────┬──────────────────────────────────────────┬─────────────────┘
                │ every tool call goes through Router      │ append-only events
┌───────────────▼──────────────────────────┐   ┌───────────▼─────────────────┐
│ 3. Fail-Closed Policy & Approval Router  │   │ 4. SHA-256 Proof-DAG Ledger │
│    policy (Allow / Ask / Deny clamps)    │   │    Shared DAG · Provenance  │
│    Desktop auto-approve or per-call card │   │    delegate.* + honesty.lint│
└───────────────┬──────────────────────────┘   └─────────────────────────────┘
                │ sympy_eval only (EffectClass::ExecuteCode)
┌───────────────▼────────────────────────────────────────────────────────────┐
│ 5. Windows 11 Zero-Capability AppContainer Python Sandbox (worker)         │
│    No network · No user files/DPAPI/env · Kill-on-close Job Object         │
└────────────────────────────────────────────────────────────────────────────┘
```

Here is what happens step-by-step during a typical derivation:

1. **Structured Campaign Phases:** The Orchestrator moves explicitly through `INGEST -> EXPLORE -> COMPUTE -> SYNTHESIZE -> VERIFY -> REVISE -> COMPLETE`.
2. **Delegation Without Context Pollution:** Instead of doing brute-force algebra inline, the Orchestrator calls `delegate_compute` (or `delegate_literature` / `delegate_lean` / `delegate_explore` / `delegate_subagent`). Each worker starts with a clean context, retries its own errors, logs `delegate.start` / `delegate.turn` / `delegate.end` with `routing_reason` and token usage, and writes durable `e-N` nodes (`symbolic_computation`, `numerical_computation`, etc.) into SQLite `ledger.db` (plus full transcripts as content-addressed SHA-256 blobs). Workers are **barred in Rust** from calling `delegate_*` recursively or writing `final_answer`.
3. **Router & Sandbox Enforcement:** Whenever Python runs (`sympy_eval`), the Router hashes the canonical request + policy version + worker epoch, records an approval decision, and sends the code to a single `worker` running `sympy`, `mpmath`, and `numpy` inside an AppContainer that cannot touch the network or your disk.
4. **Honesty Check (`honesty.lint`):** After the Orchestrator produces prose, the artifact-aware Honesty Linter scans claims like *"Lean 4 verified"*, *"SymPy confirmed"*, or *"q.e.d."* against actual passing Proof-DAG entries, `lean_check` verdicts, and accepted verification events, displaying a warning banner whenever proof is overstated.
5. **Independent Verification & Revision (`<= 3` rounds):** In `Verified`, `Deep`, and `Interactive` modes, calculation guardrails ensure computations are interpreted by a `result` entry (`depends_on: ["e-1"]`), then the stateless **Full Solution Verifier** accepts or rejects with concrete issues/counterexamples. In **Deep** mode, rejection automatically invokes the **Research Strategist** to plan repairs before revision.

---

## 4. Working Interactively: Modes, Live Steering & Explain

### Choosing Your Rigor Preset
| Preset | Best For | What Runs |
| --- | --- | --- |
| **`Direct`** | Quick look-ups, brainstorms, formatting `.tex` | Orchestrator + tools (single pass; no post-turn Verifier loop). |
| **`✓ Verified`** *(default)* | Derivations, symbolic checks, paper claims | Orchestrator + Calculation Guard + Stateless Full Verifier (up to 3 rounds). |
| **`✦ Deep`** | Hard open problems likely to need course-correction | Everything in `Verified` + **Research Strategist** diagnosis after any rejection. |
| **`Interactive`** | High-stakes projects where you want to steer the roadmap | Turns on **Verifier** + **Pause on Explore** and instructs the Orchestrator to propose a numbered plan via `ask_user` (`kind: "plan"`) before heavy execution. |

*Tip:* You can customize active agents directly in the **Conventions** popover (`Verifier`, `Strategist`, `Pause on Explore`). All presets also pair with the compact **Reasoning Effort** selector (`Default`, `Low`, `Medium`, `High`, `Extra High`, `Max`) — when set to `Default`, reSearch omits provider reasoning parameters.

### Interactive Human-in-the-Loop Controls
- **Mid-Turn Live Steering (`POST /api/sessions/{id}/steer`):** During active in-core API or ChatGPT turns, the Send button switches to **`Steer`**. Submitting a note queues a `human.steer` ledger event, shows a `Steer queued -> ...` badge, and injects `<human_steer>...</human_steer>` at the Orchestrator's very next step (without killing in-progress work). *Note:* Live steering is refused with HTTP `409` on subscription-CLI turns (`claude-cli` / `antigravity`).
- **High-Level Decision Checkpoints (`ask_user`):** The Orchestrator can pause to ask mathematical, convention, or plan questions — never for low-level tool permissions. Checkpoints appear inline with quick-pick option buttons and a free-text note field (`human.checkpoint_requested` / `human.decision`), and clicking **Stop** unblocks any waiting checkpoint cleanly.
- **Ultramath Branch Picker (`Pause on Explore`):** When enabled, `delegate_explore` evaluates 2–4 competing Ansaetze concurrently in isolated branches, then pauses to display side-by-side **KaTeX-rendered branch cards** so you can adopt one branch, combine ideas, or add steering notes (`human_selection`).
- **Isolated `Explain` Tab (`Canvas -> Explain`):** Ask clarifying questions about notation or specific `e-1` steps on a **completely isolated side-talk thread** (`sidetalk:<session>`) without mutating the Proof DAG. Export conversations as Markdown (`.md`) or standalone HTML (`.html`), or promote any insight back to the main workflow:
  - **Pin to Conventions** appends to the session's pinned conventions (`<= 2000` chars; long answers auto-trim cleanly).
  - **Send to Orchestrator** injects directly as a live steer if an API/ChatGPT turn is running, or loads the note into the main composer when idle.
- **Session vs Global Conventions:** Clicking **Conventions** edits and clears conventions for the **active session** (`PUT /api/sessions/{id}/conventions`), which persist across reloads and are upserted into the Orchestrator prompt on resumed turns as well as all workers and verifiers. When no session is active (or under **Settings → Advanced**), you edit global default conventions for new sessions.

---

## 5. Meet the Agents (Built-in Roles & Custom Subagents)

Configure models per agent anytime under **Settings & Guide → Agents & Models**:

| Agent | Role Key | Default Model Policy | Core Responsibility |
| --- | --- | --- | --- |
| **Orchestrator** | `orchestrator` | User-selected model | Leads Campaign Phases, delegates subtasks, writes `result`/`final_answer` DAG steps and LaTeX patches. |
| **Compute Worker** | `compute_worker` | **Auto Smaller Model** | Iterative `sympy_eval` + `check_expression_sanity` inside AppContainer + computation DAG nodes. |
| **Literature Worker** | `literature_worker` | **Auto Smaller Model** | `arxiv_search`, `literature_search`, `doc_ingest`, `fs_read` on bulky papers/TeX files. |
| **Lean Prover Worker** | `lean_worker` | **Auto Smaller Model** | Formalizes claims in Lean 4 without `sorry`, `admit`, or custom axioms; iterates on `lean_check` diagnostics. |
| **Parallel Explorer** | `explorer_worker` | **Auto Smaller Model** | Ultramath multi-branch runner (`delegate_explore`, 2–4 concurrent branches, read/compute-only). |
| **Step / Full Verifier** | `step_verifier`, `full_verifier` | Follow Orchestrator | Zero-bias stateless auditors checking DAG ancestors, `tex_audit` XML, and Lean checks. |
| **Research Strategist** | `strategist` | Follow Orchestrator | Diagnoses rejected proof attempts in `Deep` mode to plan structural revisions. |
| **Proof Narrator / Explain / Builder** | `narrator`, `sidetalk`, `subagent_builder` | Follow Orchestrator | `Narrate Proof` sheets, isolated **Explain** Q&A, and generating custom specialist profiles. |

- **Automatic Smaller-Model Routing:** By default, worker roles pick a faster, cheaper model from connected live catalogs (`Catalog::pick_smaller_model`), recording the choice in `routing_reason`. Override any role explicitly to pin your preferred model.
- **Custom Subagents (`<data_dir>/subagents.json`):** Describe a specialist in plain English in **Agents & Models** and click **Generate with AI Agent** to draft a role (`read_only`, `compute`, or `full_research` tool profile, optional model, effort, and rigor including `interactive`). Invokable directly from the composer pill (`Agent: ...`) or via `delegate_subagent` — still strictly bound by `v0.4-research` policy and AppContainer isolation.

---

## 6. Tool & Computation Approvals

- **Policy Clamps (`crates/policy`):** `ExecuteCode` (`sympy_eval`), `WriteWorkspace` (`fs_write`, `latex_patch`, `latex_compile`), and `NetworkRead` (`arxiv_search`, `literature_search`) are statically clamped to `Ask`.
- **Desktop Default (`auto_approve_tools = true`):** Out of the box, the desktop app enables **Settings → Advanced → Auto-approve computations, workspace writes & literature lookups** so contained SymPy scripts, TeX patches, and arXiv lookups run smoothly while still logging every decision to `ledger.db` (`approval.answered`) before execution.
- **Manual Review Mode:** Uncheck `auto_approve_tools` anytime to inspect live syntax-highlighted Python code and unified `.tex` diffs on inline approval cards, choosing **Decline**, **Approve Once**, or **Approve for Session** (revocable via the topbar `Session allowed: ... ×` pill).
- **Console (`re-search run`):** Never auto-approves; every `Ask`-classed tool prompts interactively on the terminal.

---

## 7. Connecting Providers, Billing Safety & Confinement

| Provider | How to Connect | Security / Billing Highlights |
| --- | --- | --- |
| **ChatGPT Plan** *(Windows)* | **Continue with ChatGPT** → sign in in browser → grant plan use. | Official OpenAI preview flow; DPAPI-encrypted tokens (`%LOCALAPPDATA%\re-search\chatgpt\*.dpapi`), cross-process `.lock`, Responses API (`store: false`). Shared with your ChatGPT plan limits; **does not apply `--api-base`**. |
| **Claude Subscription** (`claude-cli`) | Install Claude Code (`claude.exe`) and sign in via terminal. | Drives unmodified `claude.exe` on `fable`/`opus`/`sonnet`/`haiku`; tools served strictly by `core.exe`. **Billing guard:** refuses to run if `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, Bedrock/Vertex/Foundry vars, or non-`"none"` `apiKeySource` is present unless `RE_SEARCH_ALLOW_CLAUDE_ENV_AUTH=1`. |
| **Google Antigravity** (`antigravity`) | Sign in to `agy.exe`, read the Connect warning, click **I understand the risk**. | **Vendor-sandboxed, NOT confined to reSearch tools.** Third-party CLI automation may conflict with Google consumer terms. reSearch passes `--settings` denying built-in commands/files/URLs plus `--sandbox`, shows all non-core steps as `vendor.builtin_tool` warnings, and refuses if `GEMINI_API_KEY`/`GOOGLE_GEMINI_BASE_URL`/`AGY_ADC_AUTH` is set unless `RE_SEARCH_ALLOW_AGY_ENV_AUTH=1`. *Live turns not verified (fake-`agy` test suite only; see [THREAT-MODEL.md](THREAT-MODEL.md)).* |
| **Anthropic / OpenAI / Gemini API** | Paste API key in Providers (or set `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`/`GOOGLE_API_KEY`). | Stored locally in `providers.json`, never re-displayed or sent to browser `localStorage`; scrubbed from HTTP errors and blobs. |
| **Ollama (Local)** | Start Ollama at `http://127.0.0.1:11434` (configurable in Advanced) → **Check again**. | Runs 100% locally via OpenAI-compatible API; zero external traffic. |

**Important route rules:**
- **Route lock:** Once a session starts its first turn, it stays on its route (API/ChatGPT vs `claude-cli` vs `antigravity`); start a new session to switch routes.
- **Side-features on CLI routes:** **Narrate Proof** and **Explain** always require an API-route model (configured per-agent or first available API provider). On `claude-cli` and `antigravity`, later turns replay a bounded transcript (~24,000 chars) on stdin rather than native tool history.
- **Safe Literature Links:** External links (`Open Paper ↗`, `PDF ↗`, `DOI ↗`) are strictly allowlisted (`arxiv.org`, `doi.org`, `inspirehep.net`, `semanticscholar.org`, `crossref.org`) and open in your OS handler without navigating the WebView2 window off `127.0.0.1`; archives open inside the **Read Source** modal (`Rendered` KaTeX / `Raw`).

---

## 8. Desktop Launch, Configuration & Headless Flags

Settings precedence for `re-search-app.exe` (and desktop UI launches):
1. **CLI flags:** `--workspace DIR`, `--data-dir DIR`, `--python EXE`, `--worker-script PY`, `--port PORT`, `--core-exe EXE`, `--claude EXE`, `--agy EXE`
2. **Environment variables:** `RE_SEARCH_PYTHON`, `RE_SEARCH_DATA`, `RE_SEARCH_WORKER_SCRIPT`, `RE_SEARCH_CLAUDE` (plus `RESEARCH_CORE_*` fallbacks), `RE_SEARCH_CORE_EXE`, `RE_SEARCH_AGY`
3. **`re-search-app.json` next to the executable** *(used by Start-menu launches where env vars aren't set; relative paths resolve next to the binary)* — keys: `python`, `worker_script`, `data_dir`, `workspace`, `core_exe`, `claude`, `agy`
4. **Defaults:** `%LOCALAPPDATA%\re-search`, sibling `python/worker.py` + `core.exe`, and `claude.exe` / `agy.exe` from `PATH` or default install folders

Headless & test automation flags (`re-search ui`): `--port PORT`, `--no-open true`, `--print-url-json true`, `--api-base URL` (disables `/models` fetches so keys aren't leaked to a single mock endpoint), `--arxiv-base URL`, `--inspire-base URL`, `--crossref-base URL`. Server binds strictly to `127.0.0.1`, requires `X-ReSearch-Token` on `/api/*`, uses a scoped stream cookie for SSE/exports, and refuses `--workspace` overlapping `--data-dir`.

---

## 9. CLI Reference (`run`, `fork`, `diff`, `export`, `verify`)

Pass `--workspace DIR` (or `--enable-research-tools true`) on `--route api` / `resume` to enable the full `v0.4-research` research policy (minimal `v0.2` uses `sympy_eval` and `ledger_note` only):

```bash
re-search run --route api --provider anthropic --workspace ./paper --data-dir ./data \
  --verification-mode true --calculation-mode true --prompt "Derive and verify Lemma 1."
re-search resume s-<id> --workspace ./paper --data-dir ./data --prompt "Continue with Lemma 2."
re-search fork s-<id> --at-seq 6 --assumption "Assume k > 0" --data-dir ./data
re-search diff s-<id> s-<fork_id> --data-dir ./data
re-search export s-<id> --format zip --out ./out/proof.zip --data-dir ./data
re-search verify s-<id> --data-dir ./data
```

Also available: `re-search desktop`, `re-search ui`, `re-search show <session>`, `re-search sessions`, and `POST /api/sessions/{id}/stop` (universal **Stop** button across API, ChatGPT, and CLI routes).

**Exit codes:** `0` = completed · `1` = tampered/missing/corrupt on `resume`/`verify` · `2` = CLI usage, config, billing-guard, drift, or lock error · `3` = aborted turn/guardrail/transport error · `4` = legacy unchained schema · `5` = model refusal.

---

## 10. Codebase Map (13 Crates) & Running Tests

| Crate | Purpose |
| --- | --- |
| `crates/policy` | Pure `Allow / Ask / Deny` clamps (`serde` only; zero I/O, zero `tokio`/`rmcp`/`windows-sys`). |
| `crates/ledger` | Append-only SHA-256 hash-chained SQLite store (`user_version = 3`) + content-addressed blob directory. |
| `crates/tools` | Tool registry, router, canonical JSON, digest/epoch-bound approvals, session grants. |
| `crates/research-tools` | `fs_list`/`fs_read`/`fs_write`, `doc_ingest`, `latex_patch`/`latex_compile`/`tex_audit`, literature clients, `ledger_*`, `lean_check`, sanity checks. |
| `crates/worker` | Python worker (`sympy`, `mpmath`, `numpy`) in Windows `AppContainerSandbox` / `JobObjectSandbox`. |
| `crates/agent` | Streaming provider transports, Campaign Phases, Orchestrator–Verifier–Strategist loop, Workers (`delegate`/`explore`), `honesty.lint`, `interaction` HITL, Narrator, Explain, lossless resume. |
| `crates/export` | Counterfactual `fork`, structural branch `diff`, `.tex`/`.html`/`.zip` `export`. |
| `crates/desktop-server` | Loopback `127.0.0.1` HTTP + SSE server (`std::net` + `httparse`, zero `tokio`), `HitlHub`, `UiApprover`, offline KaTeX UI. |
| `crates/desktop-app` | `re-search-app`: WebView2 (`wry` + `tao`, Windows only) locked to loopback with header CSRF token. Only crate with GUI deps. |
| `crates/mcp-server` | `core`: stdio MCP server (**only** crate depending on `rmcp`). |
| `crates/chatgpt-auth` | ChatGPT PKCE OAuth, RS256 JWKS verification, DPAPI token store, cross-process `VaultLock`. |
| `crates/cli-route` | Unmodified `claude.exe` and `agy.exe` supervisors (stdin prompts, init checks, loopback approvals, bounded history replay). |
| `crates/cli` | `re-search` CLI binary (`run`, `resume`, `ui`, `desktop`, `fork`, `diff`, `export`, `show`, `sessions`, `verify`). |

```bash
# Core Rust suite (Linux & Windows against in-test loopback mocks)
cargo build
cargo test -p policy -p ledger -p tools -p research-tools -p agent -p chatgpt-auth -p export -p cli-route -p desktop-server -p desktop-app -p cli

# Offline KaTeX 0.19.1, XSS, Trace Cards & HITL UI tests
node --test crates/desktop-server/tests/math_render.test.mjs crates/desktop-server/tests/hitl_ui.test.mjs
```

Windows 11 AppContainer sandbox probes require Python with `sympy` and `numpy`:

```powershell
$env:RE_SEARCH_PYTHON = "C:\path\to\python.exe"
cargo test --workspace --no-fail-fast
```

Windows-only integration suites cover DPAPI `VaultLock` (`chatgpt-auth`, `desktop-server/chatgpt_route`) and Job-Object CLI route spawning (`claude_cli_*`, `antigravity_route` turn tests) using loopback mocks and scripted fake CLIs.

---

## 11. Licence

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE). `rmcp` (Apache-2.0) is isolated strictly in `crates/mcp-server` and excluded from all other crates.
