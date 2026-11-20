# Threat model (Stage 5: AppContainer isolation, hash-chained ledger, multi-provider loop & verified resume)

This file says plainly what re:Search currently protects against and what it
does not. Read the remaining limits before approving anything.

## Architecture overview

The `re-search` supervisor supports two execution routes:

- **`--route cli` (default, `"external loop, partial provenance"`).**
  Starts the Claude Code CLI (`claude.exe`) headless inside a Windows Job
  Object. The CLI runs the model's agent loop and is given exactly two tools
  served over stdio by our MCP server `core.exe` (via `sessions/<id>/mcp.json`
  carrying `CORE_SESSION_TOKEN`): `sympy_eval` (Python/SymPy in a
  zero-capability Windows `AppContainerSandbox`) and `ledger_note`.
- **`--route api` (Stage 4–5, `"in-core loop, full provenance"`).** Runs the
  in-core agent loop (`crates/agent`) directly inside `re-search` over the
  `Transport` trait (`AnthropicTransport`, `OpenAiTransport` for OpenAI/Ollama,
  or `GeminiTransport`), and supports continuing verified sessions via
  `re-search resume`. Tool calls are dispatched in-process through
  `tools::Router::try_call` and `ConsoleApprover` with no `claude.exe`,
  `core.exe`, or `sessions/<id>/mcp.json`. When `--python` is configured on
  Windows, `sympy_eval` runs inside `AppContainerSandbox`; when omitted,
  `ledger_note` remains available and `sympy_eval` stays registered as
  `Once`-scoped (returning an error if invoked after approval).

Every tool call passes through one router: record the request, ask the policy,
ask the human if the policy says "ask" (reusing an active `worker-session`
approval only while the contained worker epoch remains alive), run, record the
result.

## What Stage 2 contains

- **Zero-capability Windows AppContainer for the Python worker.** When
  `--python` and `worker.py` are configured (`CORE_PYTHON` /
  `CORE_WORKER_SCRIPT` on `--route cli`), startup fails closed unless
  `AppContainerSandbox` initializes cleanly (requiring per-session `scratch`
  and `pycache` directories). The worker runs with `TokenIsAppContainer == 1`,
  Low integrity (`S-1-16-4096`), and `CapabilityCount = 0` (no LPAC). Network
  egress (loopback TCP/UDP, local-NIC UDP, DNS resolution, external datagrams)
  and named pipes to the host are blocked by the AppContainer/WFP boundary, and
  `JobObjectBasicUIRestrictions` blocks clipboard read/write, external USER
  handles, desktop creation/switching, global atoms, display or system parameter
  changes, and `ExitWindows`.
- **Scoped worker handle inheritance (`STARTUPINFOEXW`).** Both
  `AppContainerSandbox` and `JobObjectSandbox` launch workers via
  `CreateProcessW` with `STARTUPINFOEXW` + `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`
  (no other child processes are spawned by the worker layer). Only the worker's
  two dedicated pipe handles and parent-opened `\\.\NUL` stdio handles are
  inherited by the worker.
- **Clean minimal child environment.** Host environment variables are not
  inherited; the worker receives only `SystemRoot`, `LOCALAPPDATA`, `APPDATA`,
  `USERPROFILE`, `HOME`, `TEMP`, `TMP`, `MPLCONFIGDIR`, `PYTHONPYCACHEPREFIX`,
  `PYTHONPATH`, `PYTHONNOUSERSITE`, and `PYTHONIOENCODING`.
- **Fingerprinted read-only staged Python runtime.** The base interpreter,
  selected math packages (`sympy`, `mpmath`, `numpy`, `numpy.libs`), and
  `worker.py` are staged into `<data_dir>/runtime/<fingerprint>/` under a lock
  with `*S-1-15-2-1:(OI)(CI)RX` (read/execute only, no write permission).
- **Per-session filesystem isolation and token protection.** Each session gets
  an ephemeral AppContainer profile (`re-search.worker.<session>`) granted
  Modify (`(OI)(CI)M`) strictly on `sessions/<id>/scratch` and
  `sessions/<id>/pycache`. `sessions/<id>/work`, `sessions/<id>/mcp.json`
  (`CORE_SESSION_TOKEN` on `--route cli`), `ledger.db`, other sessions' scratch
  directories, and user files outside `scratch`/`pycache` are denied
  (`ERROR_ACCESS_DENIED` / `EACCES`).
- **Containment-gated `worker-session` approval reuse and epoch binding.**
  Every human approval is bound by SHA-256 digest over the effective scope,
  caller, tool id, tool-definition hash, canonical arguments, policy version,
  and a fresh nonce. `WorkerEpochAdapter` enables `worker-session` reuse only
  when `containment().is_contained()` is `true`: the first `sympy_eval` call in
  a live worker epoch prompts for `worker-session` approval, and subsequent
  calls by the same caller with the same tool-definition hash and policy version
  reuse that grant only while that exact OS worker epoch stays alive. Timeout,
  crash, or restart ends the epoch and forces a fresh prompt. Any uncontained
  fallback or `once` tool remains pinned to per-call `once`, and both approval
  backends reject `worker-session` requests unless `contained: true`.
- **The model gets only our tools.** On `--route cli`, the supervisor starts the
  CLI with built-in tools off, user and project settings off, and only our MCP
  config, and verifies `system/init` (`mcp__core__ledger_note`,
  `mcp__core__sympy_eval`, and only `core` connected) before the model acts. On
  `--route api`, `AgentLoop` advertises only `router.registry().defs()`.
- **Code execution always asks.** `execute-code` tools can never be configured
  to `allow`: the policy engine clamps `allow` to `ask`.
- **No approval without a person.** If stdin or stderr is not a terminal, or
  the token, channel version, session, scope, or containment check fails, the
  answer is "no". There is no auto-approve switch. Stray console input outside
  an active prompt is drained and discarded.
- **Approval channel timeout & ACK.** On `--route cli`, disconnecting before a
  decision closes the prompt and records `approval.expired`; valid answers
  require an explicit ACK from `core` (`approval.delivered` /
  `approval.undelivered`).
- **Process lifetime and profile cleanup.** Both the CLI (`JobObjectSandbox`)
  and worker (`WinChild`) run in `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` Job
  Objects. On abort or exit, the supervisor kills the tree and deletes the
  session's AppContainer profile (`delete_session_profile`), and sweeps stale
  `re-search.worker.*` profiles at startup (`sweep_stale_profiles`).
- **Failing closed.** If staging, AppContainer initialization, or ledger
  recording fails, the call or startup is refused.

## What Stage 3 adds (hash-chained ledger & verification)

- **Per-session SHA-256 hash chaining.** Every event links `prev_hash` to the
  prior event's `event_hash` (anchored at 64-zero `GENESIS_HASH` for `seq = 1`).
  `event_hash` is a SHA-256 digest over 8-byte little-endian length-prefixed
  UTF-8 fields (`"re-search.event.v1"`, `session`, `seq`, `ts_ms`, `kind`,
  `schema_version`, canonical JSON `payload`, and `prev_hash`).
- **Canonical JSON with float roundtrip.** Payloads are serialized with UTF-8
  byte-sorted keys and `serde_json`'s `float_roundtrip` feature so re-parsing
  during verification preserves exact floating-point representation.
- **Database schema guards (`PRAGMA user_version = 3`).** SQLite `CHECK`
  constraints require 64-character lowercase hex digests, `event_no_update` and
  `event_no_delete` reject edits/deletes, and `event_insert_guard` enforces
  `seq = max(seq)+1` before checking `prev_hash` continuity on `INSERT`.
- **NDJSON chain receipts (`Recorder::emit`).** Supervisor stdout NDJSON lines
  include `ts_ms`, `prev_hash`, and `event_hash` so callers can capture an
  external head anchor.
- **Offline verification (`re-search verify [SESSION] [--data-dir DIR]`).**
  Opens via `Ledger::open_existing` inside a SQLite `BEGIN DEFERRED` read
  snapshot and checks Genesis continuity, contiguous `seq = 1..N`, canonical
  JSON roundtrip, `event_hash` recomputation, and fixed ledger-controlled blob
  slots (`raw_blob`; `tool.request.args`; `tool.result.result` and
  `tool.result.error`) without scanning arbitrary `model.tool_use.input`. Whole-ledger
  verification also walks `blobs/<prefix>/<file>`, verifying every blob's
  SHA-256 filename and content (allowing valid unreferenced orphan blobs, e.g.
  from skipped CLI `user` lines).
- **Fail-closed legacy handling.** Legacy v0 databases lacking `prev_hash` /
  `event_hash` remain readable via `show` / `sessions` (`read_session` /
  `sessions`), while `append*` and `verify*` refuse them
  (`Error::LegacyUnchainedSchema`, CLI exit code `4`).

## What Stage 4 adds (transport trait & in-core agent loop)

- **In-core conversation loop and wire-level provenance (`crates/agent`).**
  `prepare_request` serializes the exact outgoing JSON body, stores it in
  `blobs/`, and records `model.request` (`raw_blob`) *before* any network call;
  `model.response` (`raw_blob` of the exact incoming JSON body) and content
  block events are recorded *before* any tool call runs. `verify_session` and
  `verify_all` check all `raw_blob` digests with zero unreferenced blobs.
- **Router-only tool path and fail-closed ledger checks.** `crates/agent` does
  not depend on `worker`; tool definitions come from `router.registry().defs()`,
  and every `tool_use` block calls `Router::try_call` with a core-assigned
  caller (`agent:<provider>:<session>`). Any ledger or blob recording failure
  halts the loop immediately before network egress or tool dispatch.
- **In-process terminal approvals (`ConsoleApprover`).** Shares `StdinHub`, TTY
  checks, the 150 ms anti-buffer window, control-char escaping, containment
  scope enforcement (`worker-session` requires `contained == true`), and
  fail-closed `approval.answered` ledger recording before returning
  `ApprovalReply` (answering `approved: false` if the ledger write fails).
- **API key hygiene and transport hardening (`AnthropicTransport`).** Reads
  `RE_SEARCH_API_KEY` -> `RESEARCH_CORE_API_KEY` -> `ANTHROPIC_API_KEY` at
  runtime into `SecretKey` (zeroized on drop, `[REDACTED]` `Debug`, control
  characters rejected); records only the env var *name* (`api_key_env`) and
  sanitized endpoint (`scheme://host[:port]/v1/messages`). Enforces `https://`
  outside loopback (`127.0.0.1`, `[::1]`, `localhost`), disables HTTP
  redirects (`max_redirects(0)`), bounds response sizes (`16 MiB` response /
  `1 MiB` error body), and scrubs any accidental reflection of raw key bytes
  from error strings and HTTP bodies before returning or storing blobs.
- **Explicit refusal and truncation handling.** Model `stop_reason: "refusal"`
  dispatches zero tools, records `model.refusal` + `session.end`
  (`outcome: "refusal"`), and exits `5`; `max_tokens` truncation,
  malformed/unexpected `tool_use` blocks, transport errors (`model.error`), and
  `max_turns` exhaustion dispatch zero offending tools, record `session.abort`,
  and exit `3`.

## What Stage 5 adds (multi-provider transports & verified session resume)

- **Multi-provider auth and endpoint rules (`OpenAiTransport`, `GeminiTransport`).**
  `--provider openai` sends `Authorization: Bearer` using `RE_SEARCH_API_KEY` ->
  `RESEARCH_CORE_API_KEY` -> `OPENAI_API_KEY`; `--provider ollama` reads
  `RE_SEARCH_API_KEY` -> `RESEARCH_CORE_API_KEY` -> `OLLAMA_API_KEY` and
  permits keyless (`api_key_env: "none"`) operation *only* for `--provider
  ollama` on plaintext loopback (`127.0.0.1`, `[::1]`, `localhost`);
  `--provider gemini` sends the key strictly in the `x-goog-api-key` header
  (never in `?key=` query parameters, using `GEMINI_API_KEY` / `GOOGLE_API_KEY`
  after `RE_SEARCH_API_KEY` / `RESEARCH_CORE_API_KEY`). All providers enforce
  `https://` off loopback, `max_redirects(0)`, bounded payloads, and `SecretKey`
  scrubbing.
- **Pre-resume integrity, drift, and lock gates.** `re-search resume` opens the
  ledger with `Ledger::open_existing`, verifies the entire session hash chain
  and every referenced blob via `verify_session` (rejecting tamper with exit
  `1`, legacy v0 schemas with exit `4`, and `--route cli` or
  `policy_version`/`tool_defs` drift with exit `2` before writing any event),
  and acquires an atomic `create_new` `.active.lock` per session (reclaiming a
  stale lock at most once only when the recorded PID is verified dead).
- **Lossless history reconstruction and cross-provider sanitization.** Resume
  replays verbatim turns directly from verified `model.request` / `model.response`
  blobs and `model.tool_result` events while stripping provider-specific blocks
  (such as `thinking` and `redacted_thinking`) when switching providers,
  appending `session.resume` contiguously onto the existing hash chain.
- **Fresh worker epoch and empty `ApprovalStore` on resume.** Resuming creates
  a brand-new `Router` with an empty `ApprovalStore` and a fresh
  `Worker`/`AppContainerSandbox` epoch, so prior `worker-session` grants never
  carry across process boundaries and `sympy_eval` always re-prompts.

## What Stages 6–8 add (workspace/LaTeX tools, proof guardrails & loopback desktop UI)

- **Workspace path sandboxing & bounded ingestion (`crates/research-tools`).**
  `WorkspaceSandbox` canonicalizes paths strictly inside the attached
  `workspace_root` and `sessions/<id>/sources`, rejecting `..`, NUL bytes,
  alternate data streams, symlinks escaping root, and writes targeting
  `ledger.db`, `blobs/`, or `providers.json`. Archive (`tar.gz`/`zip`) and PDF
  ingestion enforce anti-bomb byte/file/page caps, reject archive symlinks, and
  record `"source.ingested"` `"raw_blob"` digests.
- **Unconfigurable `write-workspace` unified-diff human approvals.** Both
  `ExecuteCode` (`sympy_eval`, `latex_compile`) and `WriteWorkspace` (`fs_write`,
  `latex_patch`) are hard-clamped from `allow` to `ask` in `policy::evaluate`.
  `ConsoleApprover` and `UiApprover` display a unified diff preview
  (`preview_workspace_edit`) and SHA-256 request digest, store pre-write file
  backups in `blobs/` (`"workspace.backup"`), and record `"approval.answered"`
  before modifying any workspace file.
- **Allowlisted literature egress & deterministic `tex_audit`.** `arxiv_search`
  and `literature_search` restrict remote hosts to allowlisted HTTPS science
  endpoints (`arxiv.org`, `export.arxiv.org`, `inspirehep.net`,
  `api.crossref.org`, `api.semanticscholar.org`, plus loopback mock overrides in
  tests), reject XXE/DTD feeds, and record `"literature.fetched"` `"raw_blob"`s.
  `tex_audit` deterministically checks `\label`/`\ref`/`\cite`, verbatim TeX
  quotes, and blocks reliance on withdrawn arXiv papers or retracted DOIs.
- **Stateless verifiers, isolated Side-Talk & verified fork/export
  (`crates/agent`, `crates/export`).** Step and Full-Solution Verifiers see only
  the problem prompt, `depends_on` DAG ancestors, and `<deterministic_tex_audit>`
  (forcing `"rejected"` if `tex_audit` has blockers). Side-Talk writes only to
  `sidetalk:<id>` without mutating the solver chain; `fork` and `export` verify
  session hash chains before cloning prefixes or writing `.tex`/`.html`/`.zip`
  bundles.
- **Loopback `127.0.0.1` desktop server & CSRF protection
  (`crates/desktop-server`).** Binds exclusively to `127.0.0.1` (`std::net` +
  `httparse`, zero `tokio`), checks `Host` and `Origin` headers, issues a
  per-launch 256-bit `getrandom` token (`HttpOnly; SameSite=Strict` cookie +
  `X-ReSearch-Token` header on mutating routes), serves KaTeX `0.19.1` offline,
  and stores `<data_dir>/providers.json` outside git (`0600` on Unix, redacted on
  `GET`/`PUT /api/settings/providers`).

## Remaining limits & gaps

- **Windows system and `ALL APPLICATION PACKAGES` files remain readable.**
  Standard Windows OS libraries and directories readable by `S-1-15-2-1`
  (`ALL APPLICATION PACKAGES`) can still be read inside the AppContainer.
- **Interactive desktop connection.** To initialize `_ctypes` / `user32.dll`
  cleanly across terminal and runner desktops, the headless AppContainer worker
  connects to the logon session's default desktop (`lpDesktop = ""`,
  `CREATE_NO_WINDOW`) under `JobObjectBasicUIRestrictions` (clipboard, external
  USER handles, global atoms, and desktop switching denied). GDI screen-DC
  queries that Windows permits on the shared session desktop without cross-job
  `HWND` access are not separately virtualized.
- **Path-existence oracle on restricted directories.** Attempting to open a
  missing file vs a restricted existing file outside `scratch` can still
  distinguish `ERROR_FILE_NOT_FOUND` (2) from `ERROR_ACCESS_DENIED` (5).
- **Loopback port-state and UDP send oracle.** Connecting TCP to a closed
  loopback port fails immediately with `WSAECONNREFUSED` (10061), whereas a
  listening/filtered loopback port blocks until timeout or fails with
  `WSAEACCES` (10013). UDP `sendto()` may return success at the socket layer
  while Windows Filtering Platform drops the datagram so zero packets reach
  local or remote receivers.
- **Policy engine is policy and audit, not a security boundary.** Once a
  `worker-session` grant is active for an epoch, further `sympy_eval` calls in
  that epoch execute inside the AppContainer without re-prompting. Read the
  initial code and remember that approved computations share Python state in
  that epoch.
- **Tamper-evident, not tamper-proof without an external anchor.** Stage 3
  detects post-hoc row edits, non-canonical rewrites, deleted prefix or interior
  events, broken hash links, and corrupted or missing blobs (while the
  AppContainer worker still cannot write `ledger.db`). However, without an
  external anchor hash (such as a saved head digest from stdout NDJSON or
  `re-search verify SESSION`), an uncontained local process running as you can
  drop triggers and truncate the latest tail events of a session, delete an
  entire session, or rewrite history and recompute the full hash chain from
  Genesis to head.
- **Prompts and tool outputs still leave the machine to the model provider.**
  Model inference happens off-box (over HTTPS to Anthropic, OpenAI, or Gemini
  on `--route api`, through a local Ollama server, or through `claude.exe` on
  `--route cli`), and the remote provider's weights and sampling remain
  untrusted external computation.
- **External CLI route (`--route cli`) retains partial provenance and
  `mcp.json` token exposure.** On `--route cli`, `claude.exe` runs uncontained
  in a kill-on-close Job Object, controls the external loop, and reads
  `sessions/<id>/mcp.json` (`CORE_SESSION_TOKEN`).
- **Supervisor & LaTeX compiler run in the user's logon context.** While the
  Python worker runs inside the zero-capability AppContainer on Windows, host
  supervisor operations (`fs_read`, approved `fs_write`/`latex_patch`, approved
  `latex_compile` invoking `pdflatex`/`latexmk` with `-no-shell-escape`, and
  allowlisted literature HTTP calls) execute as the `re-search` user, and any
  uncontained local process running as the same OS user shares access to
  `providers.json`, loopback ports, and environment memory.
- **Heuristic & LLM checks are not formal kernel proofs.** `sympy_eval`,
  `check_expression_sanity`, `tex_audit`, and LLM Step/Full Verifiers catch many
  algebraic and citation errors, not formal theorem-prover guarantees.
