# Threat model (Stages 1–9: AppContainer isolation, hash-chained ledger, multi-provider loop, research tools, loopback desktop UI & subscription routes)

This file says plainly what re:Search currently protects against and what it
does not. Read the remaining limits before approving anything.

## Architecture overview

The `re-search` supervisor supports two console execution routes and two
additional desktop subscription routes:

- **`--route cli` (default, `"external loop, partial provenance"`).**
  Starts the Claude Code CLI (`claude.exe`) headless inside a Windows Job
  Object. The CLI runs the model's agent loop and is given exactly two tools
  served over stdio by our MCP server `core.exe` (via `sessions/<id>/mcp.json`
  carrying `CORE_SESSION_TOKEN`): `sympy_eval` (Python/SymPy in a
  zero-capability Windows `AppContainerSandbox`) and `ledger_note`. The
  desktop server runs the same route for sessions bound to `claude-cli` (see
  "The Claude subscription route in the desktop" below), with the research
  tools added, and also offers an opt-in, labelled Antigravity CLI route
  (`antigravity`, `agy.exe`, `"vendor-sandboxed, not confined to re:Search
  tools"`). Both use `crates/cli-route`.
- **`--route api` (Stage 4–5, `"in-core loop, full provenance"`).** Runs the
  in-core agent loop (`crates/agent`) directly inside `re-search` over the
  `Transport` trait (`AnthropicTransport`, `OpenAiTransport` for OpenAI/Ollama,
  `GeminiTransport`, and—on the desktop only—`ChatGptTransport` over the
  Responses API), and supports continuing verified sessions via
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
- **No approval without a person on the console route.** If stdin or stderr is not a terminal, or
  the token, channel version, session, scope, or containment check fails, the
  answer is "no". There is no auto-approve switch on the console route (see
  `auto_approve_tools` in the desktop section below). Stray console input outside
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
  alternate data streams, symlinks, and Windows NTFS directory junctions
  escaping root. The sandbox has no
  per-file deny-list; instead `ResearchEnv::with_workspace` refuses to attach a
  workspace that is this launch's data directory, contains it, or lies inside
  it, so that launch's `providers.json`, `ledger.db` and `blobs/` are not
  under its workspace root. CLI `--workspace`, the MCP server and the desktop
  server (at launch, on `POST /api/sessions`, and on in-place
  `PUT /api/sessions/{id}/workspace`) all go through it; the check
  itself and the desktop routes are tested. It compares against the data
  directory in use only: a workspace containing *another* re:Search data
  directory (for example the default `%LOCALAPPDATA%\re-search` while
  running with `--data-dir` elsewhere) is not refused. Archive
  (`tar.gz`/`zip`) and PDF
  ingestion enforce anti-bomb byte/file/page caps, reject archive symlinks, and
  record `"source.ingested"` `"raw_blob"` digests.
- **Three new effect classes and how the policy treats them.** Stage 6 adds
  `read-workspace` (`fs_list`, `fs_read`, `read_paper_lines`,
  `search_paper_passages`, `search_formal_library`, `doc_ingest`, `tex_audit`,
  `record_paper_summary`, `record_paper_map`, `record_paper_branch`,
  `record_paper_conventions`, plus worker-only `note_evidence` registered
  exclusively on the isolated `paper_reader` router),
  `write-workspace` (`fs_write`, `latex_patch`) and `network-read`
  (`arxiv_search`, `literature_search`). The research policy `v0.4-research`
  allows `read-workspace` and `observe-local` without asking, asks for
  `write-workspace`, `execute-code` (`sympy_eval`, `latex_compile`) and
  `network-read`, and denies everything else (including `network-egress`).
- **Policy-layer clamps vs desktop `auto_approve_tools` (`UiApprover`).**
  `policy::evaluate` in `crates/policy` still hard-clamps `allow` to `ask` for
  `execute-code`, `write-workspace` and `network-read` at the `policy` layer; a
  `deny` rule stays `deny`, and the CLI route always requires an interactive
  terminal decision (`ConsoleApprover`). On the desktop server, `UiApprover`
  checks `auto_approve_tools` (`Settings -> Advanced`, stored in provider
  settings and **default `true`**) before showing UI approval cards: when on,
  `UiApprover` approves immediately while still fail-closed recording
  `"approval.answered"` in `ledger.db` (denying if the ledger write fails) and
  keeping `sympy_eval` inside the Windows AppContainer and `write-workspace`
  pre-write backups in `blobs/` (`"workspace.backup"`). Unchecking
  `auto_approve_tools` switches desktop sessions back to digest-bound UI cards
  (`preview_workspace_edit` unified diff for writes, SHA-256 request digest)
  before execution.
- **Literature queries & optional human session opt-in.**
  The query text of `arxiv_search` / `literature_search` is chosen by the model,
  so it is a channel by which a manipulated model could carry workspace content
  to a search host. `policy::evaluate` hard-clamps `network-read` to `ask`, so
  on the CLI route (and on the desktop when `auto_approve_tools` is unchecked),
  the first model-initiated literature lookup in a session prompts the
  human with the full arguments (including the query) before any network request
  is made. When manually prompted on the desktop in-core route, the human may
  choose *Approve Once* (single-use digest) or *Approve for Session*
  (`approve_for_session: true`), which binds a `NetworkReadGrant` strictly for
  `EffectClass::NetworkRead` tools under the same `(session_id, caller,
  policy_version)` (recorded as `approval.session_bound` /
  `approval.session_reused`); `write-workspace` and `execute-code` can never
  use a `NetworkReadGrant`, and grants never carry across sessions or server
  restarts. A refused call makes no network request (tested). Searches the
  person types into the desktop UI's literature drawer
  (`POST /api/sessions/{id}/literature`, token-protected) are human-authored and
  run without a second prompt.
- **Allowlisted literature egress & deterministic `tex_audit`.** `arxiv_search`,
  `literature_search`, and the unified F11/X19 literature runner restrict remote
  hosts to allowlisted HTTPS science endpoints (`arxiv.org`, `export.arxiv.org`,
  `inspirehep.net`, `api.crossref.org`, `api.semanticscholar.org`, and
  `api.openalex.org` [Owner Decision 2026-11-26, X19 Part 1], plus loopback mock
  overrides in tests), pace every request through the process-wide F3 rate gate,
  send optional per-user API keys strictly via request headers (`x-api-key` for
  Semantic Scholar, `Authorization: Bearer <key>` for OpenAlex — never in URLs
  or ledger payloads, and stripped on cross-origin redirects), reject XXE/DTD
  feeds, mark OpenAlex inverted-index abstracts non-verbatim (`verbatim: false`
  so they never count as `evidence.abstract`), and record `"literature.fetched"`
  `"raw_blob"`s. TheoremSearch remains excluded/disabled (not allowlisted).
  `tex_audit` deterministically checks `\label`/`\ref`/`\cite`, verbatim TeX
  quotes, and blocks reliance on withdrawn arXiv papers or retracted DOIs.
- **Stateless verifiers, isolated Explain & verified fork/export
  (`crates/agent`, `crates/export`).** Step and Full-Solution Verifiers see only
  the problem prompt, `depends_on` DAG ancestors, and `<deterministic_tex_audit>`
  (forcing `"rejected"` if `tex_audit` has blockers). Explain writes only to
  `sidetalk:<id>` without mutating the Orchestrator chain; `fork` and `export`
  verify session hash chains before cloning prefixes or writing `.tex`/`.html`/`.zip`
  and Explain `.md`/`.html` (`explain_md`/`explain_html`) bundles.
- **Loopback `127.0.0.1` desktop server (`crates/desktop-server`).** `re-search
  ui` / `desktop` binds exclusively to `127.0.0.1` (`std::net` + `httparse`,
  zero `tokio`) and serves the UI, an HTTP/JSON API and an SSE event stream.
  Credentials and checks, per request:
  - *Host.* Every request must carry a loopback `Host` (`127.0.0.1[:port]` or
    `localhost[:port]`), which defeats DNS-rebinding pages.
  - *Three secrets per launch.* A 256-bit `getrandom` **token**, a separate
    256-bit **stream cookie** value, and a separate single-use 256-bit
    **launch nonce** (60 s TTL). `re-search ui` opens the browser at
    `/?launch=…` (never with the live API token; `/?token=…` is rejected with
    403); the `re-search-app` window instead requests `/` with the token in the
    `X-ReSearch-Token` request header (see below). Only a load of `/` carrying
    a valid header token or redeeming the unconsumed, unexpired launch nonce
    gets the token written into the page (`<meta>` tag) and sets the
    `HttpOnly; SameSite=Strict` cookie `__research_stream_<port>`. The nonce is
    consumed atomically on the first matching load (wrong guesses do not burn
    it; replays and expired nonces answer 403, and the nonce is never accepted
    on `/api/` routes). A page on another origin cannot attach the header to a
    navigation, and a `fetch` carrying it needs a CORS preflight the server
    never grants. The UI strips `launch`/`token` from the address bar and keeps
    the token in `sessionStorage` (per origin, port included, per tab). A
    reload carries only the cookie and is served the page *without* the token,
    so the cookie can never be exchanged for the token; `/` without a valid
    header, fresh nonce, or cookie answers 403.
  - *Every `/api/` route except `GET /api/health`* (which returns only `ok`
    and the version) requires the token in the `X-ReSearch-Token` or
    `Authorization: Bearer` header, compared in constant time; otherwise 401.
    This covers listing sessions, session contents, workspace listings and file
    contents, the proof DAG, diffs, Explain, provider/agent settings and all
    mutations. Credentials are checked from the request headers before any
    body is read.
  - *What the cookie opens.* Only `GET /api/sessions/{id}/events` (SSE) and
    `GET /api/sessions/{id}/export`, the two requests a browser cannot attach
    a header to, and only when the browser does not mark the request as coming
    from elsewhere: `Sec-Fetch-Site`, when present, must be `same-origin`, and
    `Origin`, when present, must be this server. Browsers send a `127.0.0.1`
    cookie to *every* port on that host, so another local web service can
    capture it; with it alone that service cannot list sessions, browse the
    workspace, obtain the token or mutate anything. A non-browser client
    replaying the cookie could read the event stream or an export of a
    session whose id it already knows (ids created by re:Search carry 64
    random bits); those carry what the session touched: prompts, tool
    arguments and results, and diff previews of workspace files.
  - *Framing.* The page is served with `X-Frame-Options: DENY` and
    `frame-ancestors 'none'`. A tab opened without the launch token (new tab,
    bookmark) shows a notice instead of the app.
  - *Mutations.* Every non-GET request needs the header token and, when an
    `Origin` is present, this server's own origin. No CORS headers are sent,
    so a page on any other origin can neither set the header nor read a
    response; a cross-site page cannot send the cookie either.
  - *No request-supplied provider endpoint.* The base URL a provider call (and
    with it the provider key) goes to comes only from launch configuration or
    saved settings, never from a request body.
  - *Side effects of reads.* GET handlers only read: `export` renders
    `.tex`/`.html`/`.zip` in memory and writes nothing to disk (the CLI
    `re-search export --out` is what writes files). Opening the ledger can
    create an empty `ledger.db` in the data directory if none exists.
  - *Approvals.* Model tool calls routed through `crates/policy` (`execute-code`,
    `write-workspace`, `network-read`) hit `UiApprover`. When `auto_approve_tools`
    is enabled (desktop default in `Settings -> Advanced`), `UiApprover`
    auto-approves after fail-closed recording `approval.answered` in `ledger.db`
    before the tool runs (keeping AppContainer isolation and workspace backups
    active). When `auto_approve_tools` is off, calls pause for digest-bound UI
    approval cards and answering requires the header token.
  - *Provider keys.* Keys are stored in `<data_dir>/providers.json` outside git
    (`0600` on Unix; on Windows no ACL is set, so the file inherits the data
    directory's permissions: private under the default
    `%LOCALAPPDATA%\re-search`, but not if `--data-dir` points at a shared
    folder). The key is plaintext on disk. `GET`/`PUT
    /api/settings/providers` return only a redacted view (`configured`,
    `base_url`, `default_model` per provider, plus UI settings); its type has
    no key field. Saving a `base_url` without an `http(s)://` scheme, or with
    `user@`, a query string, a fragment or a backslash, is refused, and a
    stored value that fails the same check is not echoed.
  - *Literature settings (X19 Part 1).* Optional OpenAlex credentials and
    polite-pool preferences are stored outside git in `<data_dir>/lit_settings.json`
    (`0600` on Unix; inherits data-directory ACLs on Windows, symlinks refused,
    with `OPENALEX_API_KEY` environment-variable fallback). `GET`/`PUT
    /api/settings/literature` require `AuthGuard::authorize_api` header token
    auth, return only a masked key (`****<last4>` or `****`) plus source
    provenance, fail closed on corrupt files, and never write keys to URLs,
    `Debug` logs, or `ledger.db`.
  - KaTeX `0.19.1` is served offline from the binary; the UI loads nothing
    from the network. Markdown/LaTeX rendering (`math.js`, `math_katex.js`,
    `math_scan.js`, `math_equations.js`, `markdown.js`, `tex_preview.js`, and
    `code_highlight.js`) strips NUL bytes, passes all untrusted prose, TeX
    commands, and link URLs through `escapeHtml` (allowing only `http(s)://`
    links), is tested against the vendored KaTeX bundle by
    `tests/math_render.test.mjs` and `tests/math_parity.test.mjs`
    (`tests/ui_math.rs`), and has been visually verified inside `re-search-app`
    on Windows 11 (WebView2 158; `cargo test` itself opens no WebView2 display
    window).

## The desktop window (`re-search-app`, `crates/desktop-app`)

- **One process, no browser.** `re-search-app.exe` starts the loopback
  server in-process (`open_window: false`, `print_url_json: false`) and runs
  one WebView2 window (`wry 0.57.3` + `tao 0.37.3`) on the main thread.
  Closing the window shuts the server down and exits the process, which also
  ends child processes held in kill-on-close Job Objects. `re-search desktop`
  starts it from its own directory (passing only `--data-dir`, `--workspace`,
  `--python`, `--worker-script`, `--port`, `--core-exe`, `--claude`, `--agy`,
  never a token); with any other flag,
  or when `re-search-app.exe` is absent, it falls back to `re-search ui`.
- **Token delivery without a URL.** The first navigation is `GET /` with the
  token in the `X-ReSearch-Token` request header (WebView2
  `NavigateWithWebResourceRequest`). The token is never placed in a URL, on
  the command line of `re-search-app` or of the WebView2 processes (checked
  on this machine with WebView2 158: no `msedgewebview2.exe` command line
  carried it), or in the readiness JSON. WebView2 may drop such a header
  silently, so after the first load a probe script reports only `ok` /
  `no-token` / `elsewhere`. On `no-token`, a fallback script is run in the
  current top-level document, only if `window.location.origin` is exactly
  this server's `http://127.0.0.1:<port>`: it stores the token in the
  `sessionStorage` key the UI reads (`re-search-token`), fetches `/` with the
  header (which sets the stream cookie) and reloads `/` (served without the
  token because only the cookie is sent). On the test machine the header path
  worked and the fallback was not needed.
- **WebView2 profile.** The window runs InPrivate (`with_incognito(true)`,
  WebView2 `IsInPrivateModeEnabled`), intended to keep cookies (including the
  stream cookie), `sessionStorage` (holding the token) and the HTTP cache for
  the session only. Microsoft's reference page for that property does not
  spell out what InPrivate stores where, so this rests on InPrivate's general
  browser semantics plus the on-disk search below. Cookies and
  `sessionStorage` are what the event stream (`EventSource` with the
  `SameSite=Strict` cookie) and export downloads rely on; the test run showed
  the page authenticated (the token reached the page), but the event stream
  and an export download were not exercised inside the window. The user data
  folder is
  `%LOCALAPPDATA%\re-search\webview` (never under a data directory's
  `sessions/` or inside the workspace; the app refuses to start otherwise).
  WebView2 still writes browser component data there (`Local State`,
  crash-report settings, variations seeds, shader caches, an empty `Default`
  profile skeleton). After a test run, a search of that folder found neither
  the stream-cookie name, the server URL nor the page's token `<meta>`; that
  search was not exhaustive.
- **Lock-down.** A navigation handler allows only URLs on exactly
  `http://127.0.0.1:<port>` (the character after the port must be `/`, `?`,
  `#` or the end) plus `about:blank`; look-alikes such as
  `http://127.0.0.1:<port>.evil.example` or another port are refused. Every
  new-window request (`window.open`, `target="_blank"` links such as arXiv
  links in rendered text) is denied, so external links do not open from the
  window. A download is allowed only from
  `http://127.0.0.1:<port>/api/sessions/<id>/export`, saved into
  `%USERPROFILE%\Downloads` under a sanitized name that never overwrites an
  existing file (`name (1).ext`, `name (2).ext`, ...). The navigation rule
  sees top-level navigations only, so the page's own policy (`frame-src
  'none'`, `object-src 'none'`, `base-uri 'none'`, sent with `/`) is what
  keeps frames and plugins out. DevTools are off unless the app is started
  with `RE_SEARCH_DEVTOOLS=1`.
- **Identity and single instance.** Window and taskbar icons are embedded,
  the process sets the AppUserModelID `ReSearch.Desktop` before creating its
  window, and a named mutex `Local\ReSearch.Desktop.SingleInstance` makes a
  second launch exit without touching the running one.
- **No `--app=<url>` browser mode any more.** The former Edge/Chrome
  `--app=<url>` launch path of `re-search desktop` was removed because it put
  the token on the browser's command line.

## The Claude subscription route in the desktop (`claude-cli`)

- **What runs.** For a session created or messaged with `provider:
  "claude-cli"`, each turn starts the unmodified `claude.exe` the user signed
  into (re:Search never reads or passes its login token, never passes
  `--bare`, and adds no sign-in flow) in a kill-on-close Job Object, with the
  console route's flags (`-p`, `--output-format stream-json`, `--verbose`,
  `--tools ""`, `--strict-mcp-config`, `--mcp-config <sessions/<id>/mcp.json>`,
  `--setting-sources ""`, `--disable-slash-commands`,
  `--no-session-persistence`, `--allowedTools mcp__core__*`) plus
  `--permission-prompts none` (listed by `claude --help` of 2.1.318: anything
  that would prompt inside the CLI is denied; our tools are pre-allowed). The
  same flags are now used by `re-search run --route cli`.
- **Only our tools, checked before the model acts.** `core.exe` gets the
  session id, a fresh 256-bit session token, the loopback relay address, the
  data directory, the session's workspace (`CORE_WORKSPACE`) and
  `CORE_ENABLE_RESEARCH_TOOLS=1`, so it serves `sympy_eval`, `ledger_note` and
  the research tools under `v0.4-research`, as the API route does. The list
  the `system/init` event must show is computed by the same registration code
  `core.exe` uses (`cli_route::core_tools`); any extra or missing tool, or any
  MCP server besides `core`, aborts the turn (`session.abort`) before any
  model output is accepted, and kills the process tree.
- **Approvals.** `core.exe` asks over the loopback relay, which checks the
  channel version, session token, session, scope and containment (an
  uncontained `worker-session` request is refused) and then shows the request
  as the normal approval card (`UiApprover`: digest-bound, `approval.answered`
  recorded fail-closed before the answer is sent). Only
  `POST /api/sessions/{id}/approvals/{call_id}` with the header token and the
  matching digest approves; a wrong digest is refused. If `core.exe`
  disconnects while the card is open, or the turn is stopped, the card is
  denied (`"denied: supervisor: ..."`), never approved. ACK bookkeeping
  (`approval.delivered` / `approval.undelivered` / `approval.expired`) is the
  console route's.
- **What "partial provenance" means here.** The model's requests and
  responses are not seen by re:Search: the ledger holds what the CLI printed
  (each raw stream-json line as a blob, mapped to `cli.*` / `model.*` events),
  the exact prompt written to the CLI (the start event's `raw_blob`), and,
  from `core.exe`, every tool request, policy decision, approval and result.
  The system prompt, sampling, any CLI-internal retries and what the vendor
  sends to its servers are not recorded. Start events carry route
  `claude-code-cli` and `"external loop, partial provenance"`;
  `GET /api/sessions/{id}` reports both. A session never mixes this route
  with the in-core API route.
- **Billing safety.** `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`,
  `CLAUDE_CODE_OAUTH_TOKEN`, `CLAUDE_CODE_USE_BEDROCK`,
  `CLAUDE_CODE_USE_VERTEX` and `CLAUDE_CODE_USE_FOUNDRY` take precedence over
  the subscription login in non-interactive mode. re:Search does not remove
  them (that would restrict how the CLI authenticates); when any is set (non-
  empty) it refuses to start the turn, naming the variables but never their
  values, on the desktop and on `re-search run --route cli`. On the desktop the
  check reads the environment the CLI really inherits (re:Search's process
  environment); a configured `DesktopConfig::connection_env` map can only add
  variables to the check, never hide one the child would see (tested on
  Windows in `claude_cli_env.rs`).
  `RE_SEARCH_ALLOW_CLAUDE_ENV_AUTH=1` proceeds anyway and records the names in
  the start event (`env_auth_vars_allowed`). Second check, on the CLI's own
  report: after a verified `system/init`, a turn whose `apiKeySource` is
  present and not `"none"` (for example `"apiKeyHelper"` from managed
  settings, which `--setting-sources ""` does not switch off, or a key the CLI
  found elsewhere) is aborted at once (`session.abort` with the reason, process
  tree killed) unless `RE_SEARCH_ALLOW_CLAUDE_ENV_AUTH=1` is set. The live
  2.1.318 recording (`crates/cli/src/fixtures/stream_cli_2_1_318.ndjson`)
  shows `"apiKeySource":"none"` for a subscription login. The model may have
  been loaded by then but has not acted (the abort comes on the init line,
  before any assistant event is accepted). Not covered: a third-party
  provider chosen in managed settings that the CLI does not report in
  `apiKeySource`, and an inherited `CLAUDE_CODE_SIMPLE=1` (bare mode), which
  makes the CLI ignore the subscription login (the Connect panel warns).
- **Prompt on stdin.** The prompt is written to the CLI's standard input and
  stdin is then closed; it is not on the command line (not visible in the
  process list, no length limit). Both routes.
- **History replay.** The CLI is one-shot, so turns after the first start
  with a delimited transcript of this session's earlier user messages and
  final assistant replies, read from the ledger after `verify_session`
  passes (most recent turns up to about 24,000 characters; the prompt says
  how many older turns were left out; tool calls and results are not
  replayed). The start event records `history_replayed`, `history_chars`,
  `history_turns` and `history_turns_dropped`. Every tag and the new-message
  line carry a random marker chosen per prompt, and tag- or marker-like text
  inside replayed messages (`<turn`, `</assistant`, `[New message from the
  user`, ...) is rewritten into an inert, visibly quoted form, so text an
  earlier turn printed (for example quoted from a paper) cannot pose as a new
  user message. Turns that ended in `session.abort` are marked as aborted. The
  transcript is still text the model reads: this keeps the structure
  unambiguous, it does not make the model ignore what the text says; the
  approval cards gate every effect either way.
- **`mcp.json` lifetime.** `sessions/<id>/mcp.json` holds the turn's
  `CORE_SESSION_TOKEN`. It is written right before the CLI is started (after
  the start event) and deleted when the turn ends on every path (completion,
  abort, stop, error, server shutdown: closing the window waits up to 5 s for
  running turns to finish their cleanup). If writing it or starting the CLI
  fails after the start event, the turn records `session.abort` with the
  reason, so the ledger never keeps a start without an end (tested in
  `cli-route` `turn_tests.rs`). While the turn runs it is readable by any process running
  as you; a crash or kill of re:Search itself leaves it behind (the token is
  useless once its relay listener is gone) until the next server start or
  until the session is opened: both delete `sessions/*/mcp.json` unless a
  live turn owns it (one of this process, or another running process named
  in the session's `.active.lock`).
- **Stop and shutdown.** `POST /api/sessions/{id}/stop` (header token) kills
  the turn's process tree and records `session.abort` (`stopped: true`); the
  stop handle is registered before the turn's thread starts, so a stop right
  after the message's 202 ends the turn before anything is started (nothing
  is recorded then). `claude --version` runs with a 5 s deadline, without a
  console window and with stdin closed, and the stop is checked right after;
  stopping the server (closing the app window) stops every running turn, and
  process exit closes the Job Object, which kills the tree in any case.
- **Not verified, because no live model call was made.** That `claude -p`
  2.1.318 reads the prompt from stdin when no prompt argument is given (the
  documented headless use; `--help` says only "useful for pipes"); how
  `--permission-prompts none` behaves in practice (every flag re:Search
  passes, this one included, appears in the captured 2.1.318 `--help`
  excerpt `crates/cli-route/fixtures/claude_help_2_1_318.txt`, which a unit
  test checks, and the fake CLIs reject unknown options; a supervised live
  run is still needed before merge); what `apiKeySource` the real CLI prints
  under an `apiKeyHelper`; the `system/init` tool list
  the real CLI prints when `core.exe` serves the research tools; and that a
  real turn's stream maps to the SSE events as the scripted fake's does. All
  route tests use a scripted fake CLI and never start `core.exe`.
- **Output read errors.** The CLI's stdout is read as bytes: a line that is
  not valid UTF-8 is kept as a blob and recorded as `cli.unparsed`; a read
  error (other than the pipe closing after a stop) ends the turn with
  `session.abort`, never with a clean `session.end`.

## Connection status and live model lists (`/api/connections`, `/api/models`)

- **Same credential rule as every API route.** Both are `GET` routes under
  `/api/` and need the header token (tested in the route matrix of
  `tests/api_auth.rs`); the stream cookie does not open them.
- **Which hosts are called, with which credential.** These calls are made
  by the desktop on its own (when the app opens, on refresh), not because the
  person chose a provider, so each uses only that provider's own credential.
  `GET /api/models` makes one read-only `GET` per provider that has one, only
  to that provider's saved `base_url`, else its default (never a request
  body): Anthropic `{base}/v1/models?limit=1000` with `x-api-key` and
  `anthropic-version`, OpenAI `{base}/v1/models` with `Authorization:
  Bearer`, Gemini `{base}/v1beta/models?pageSize=1000` with the key in the
  `x-goog-api-key` header (never in the URL), Ollama
  `{base without /v1}/api/tags`. The key is the provider's own variable
  (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`/`GOOGLE_API_KEY`,
  `OLLAMA_API_KEY`), else its saved key. The generic `RE_SEARCH_API_KEY` /
  `RESEARCH_CORE_API_KEY`, which a turn reads first for whatever provider it
  runs on, is used for a list call only when `RE_SEARCH_PROVIDER` (or
  `RESEARCH_CORE_PROVIDER`) names that one provider; otherwise it is never
  sent to list models, and a provider whose only key is the generic one is
  shown as not connected, with a status that says why. The Ollama probe
  never carries `OPENAI_API_KEY` (the Ollama transport's fallback) or any
  other provider's key: only `OLLAMA_API_KEY` or a saved Ollama key, else no
  key. With `--api-base` / `RE_SEARCH_API_BASE` at launch, every provider's
  calls would go to that one address, so **no list call is made at all**:
  providers with their own key are shown connected with re:Search's
  built-in model names and a status saying the list was not fetched, and
  Ollama is shown as not checked. (Turns still use the launch override and
  the turn key order; that is the person's explicit choice per message.)
  Tested with loopback mocks that record the `Authorization`, `x-api-key`
  and `x-goog-api-key` headers each host received
  (`tests/models_api.rs`). The calls use the transports' rules
  (`agent::model_list`): `https://` except on loopback, no redirects, a
  4 MiB response limit, connect and total timeouts (10 s; 3 s for Ollama).
  **Errors carry only a status class** (`auth (HTTP 401)`,
  `rate limit (HTTP 429)`, `HTTP n`, `timeout`, `network`,
  `oversized response`, `unexpected response`): the body of an error
  answer is never read, so a provider message that quotes part of a key
  (for example a masked `sk-proj-****abcd`) cannot reach the cache, the
  JSON responses or the UI (tested). `GET /api/connections` calls no API
  provider; it only asks Ollama's `/api/tags` (to say whether it is
  running) and reports, for each provider, whether a key is saved and
  whether the key a turn would use comes from the environment (with the
  variable's name), never the key.
- **The Claude CLI is never given a prompt here.** Connection status locates
  `claude.exe` the same way a turn does, applies the same billing-safety
  check (naming variables, never values) and runs `claude --version` at most
  once per server launch (5 s timeout, no console window, nesting markers
  removed). The model choices for this route are the CLI's own aliases
  (`fable`, `opus`, `sonnet`, `haiku`); no list endpoint is called.
- **What is cached.** Model lists (ids, labels, times, the last error)
  stay in the server's memory for 10 minutes (a failed call for 30 s),
  never on disk; `?refresh=1` bypasses the cache and saving or removing a
  key or base URL clears it; a list call that began before such a change
  does not store its result (a generation counter). Keys are not cached by
  this code (they are
  read from the environment or `providers.json` for each call). The browser
  keeps only the last chosen provider and model id per session in
  `localStorage`.
- **Model names are untrusted text.** Ids and labels come from the
  provider (or a server at a configured base URL). The server keeps only
  ids of at most 160 characters without whitespace, control characters or
  a leading `-` (Gemini ids also pass the transport's model-name check),
  strips control characters from labels and caps their length and the list
  (500 models); the UI inserts them only as option text, never as HTML,
  and a chosen id is sent back as the `model` of a message, where the
  transports and the CLI route validate it again.
- **Not covered.** A base URL saved by whoever holds the token decides
  where the list call (and the key) goes, as for turns (see "The desktop
  token is as powerful as your provider keys"). Ollama's status call goes
  to its address without asking.

## Sign in with ChatGPT (`chatgpt`, `crates/chatgpt-auth`)

The `chatgpt` provider runs a session on the person's ChatGPT plan through
OpenAI's "Sign in with ChatGPT" flow for open-source and locally hosted apps
(documentation: <https://developers.openai.com/siwc/token-sharing-open-source>
and its pages *Registration and sign-in*, *Accounts and sessions*, *Models and
inference*, *Token reference*, *Errors and recovery*, *Preview limitations*;
read 2026-11-21). OpenAI calls the programme a preview. The model runs in
re:Search's own loop (`agent::ChatGptTransport`, Responses API), so every tool
call goes through the router, the policy, the approval cards and the ledger
exactly as on the API-key route; the session records `route: "api"`,
`provider: "chatgpt-responses"`, the endpoint, the model and
`api_key_env: "chatgpt-sign-in"` (a label, never a token).

- **Sign-in.** `POST /api/connections/chatgpt/login` (launch token required,
  like every `/api/` route) persists this host's `ext_agent_host_id`
  (`urn:uuid:` + a random UUIDv4, created once) and starts a one-shot listener
  on `127.0.0.1` with a system-chosen port and the fixed path `/auth/callback`
  (the documentation lets only the port vary and forbids `localhost`). The
  authorize URL carries `response_type=code`, `client_id`
  (`dynamic_agent_client` the first time, then the issued `oaiapp_…` ID),
  `redirect_uri`, `scope=openid profile email offline_access resource.invoke
  chatgpt.tokens.use.direct`, `resource=https://api.openai.com/v1`, a fresh
  random `state` and `nonce`, `code_challenge` (S256) and
  `ext_agent_host_id`; `agent_name_hint=re:Search` only on first
  registration, and `prompt=consent` only when re-authorizing after an earlier
  declined plan scope (never on ordinary sign-in). **No `id_token_hint`** is
  sent: the URL is shown in the UI and may be copied, and a token must not
  travel in a URL. The UI shows the URL with *Open in browser* and *Copy link*
  (and restores it if Connect is reopened while pending). *Open in browser*
  asks the server (`POST …/open`) to open **its own pending URL** (never a URL
  from the request, and only one starting with the authorize endpoint) with the
  system's default handler (`ShellExecuteW` with STA COM initialization, no
  shell, so `&` in the URL cannot start a command); nothing opens a browser
  unless the person clicks. Server shutdown cancels any pending login listener.
  In-process tests never hold the real opener: the shared desktop-server test
  configuration and the unit-test configurations use an opener that refuses
  (`test: browser opening disabled`) and endpoints on a closed `127.0.0.1`
  port, and in `cfg(test)` builds of `chatgpt-auth` and `desktop-server` the
  real opener is compiled out (it returns the same refusal). A test drives
  sign-in and *Open in browser* with a valid token and asserts the refusal
  (`api_auth.rs`; the sign-in part runs on Windows only). The CLI's
  end-to-end test starts the real `re-search ui --no-open` binary, whose
  configuration is the production one; it never calls the open route.
- **The callback is unauthenticated by nature.** The listener enforces a 5 s
  total wall-clock read deadline (defeating slowloris stalls), parses only
  headers before `\r\n\r\n`, and answers only `GET /auth/callback` (`HTTP/1.0`
  or `HTTP/1.1`) with a single exact `Host: 127.0.0.1:<port>` header, no
  duplicate OAuth query parameters, and the attempt's exact `state`
  (constant-time comparison). Anything else gets a short static page and the
  listener keeps waiting (at most 32 such requests, then the attempt fails).
  The first request with the right `state` ends the attempt whatever its
  outcome, and the listener closes, so a replayed callback finds nothing
  listening (tested). `error=access_denied` stops the attempt without an
  exchange; `error=invalid_client` on a returning client forgets the saved
  `client_id` and `subject` (keeping `ext_agent_host_id`). A new registration
  must return an issued `client_id` (never `dynamic_agent_client`); on
  reauthorization a different `client_id` is rejected. The attempt expires
  after 5 minutes. Pages are static text (no inline styles) with
  `Cache-Control: no-store`, `Referrer-Policy: no-referrer`,
  `Content-Security-Policy: default-src 'none'; frame-ancestors 'none'`,
  `X-Content-Type-Options: nosniff`, and `X-Frame-Options: DENY`; no code,
  token or claim is echoed.
- **Exchange and validation.** The code is exchanged at
  `https://auth.openai.com/api/accounts/oauth/token` (form-encoded
  `authorization_code` grant with the issued `client_id`, `code`,
  `code_verifier`, the same `redirect_uri` and `resource`; no client secret).
  An in-flight sign-in checks cancellation under the vault lock before saving,
  so signing out or starting a new sign-in during code exchange cannot be
  overwritten. The ID token is checked before anything is stored: canonical
  base64url decoding, RS256 signature against
  `https://auth.openai.com/.well-known/jwks.json` (`ring`, already in the
  dependency tree through rustls; `crit` and non-string `kid` rejected), `iss`
  exactly `https://auth.openai.com`, `aud` containing the issued client ID (and
  `azp` equal to it whenever present, and required whenever `aud` has multiple
  entries), `exp` and optional `nbf` with 5 s skew (saturating arithmetic;
  fail-closed clock check), `iat` present, `nonce` equal to the attempt's,
  non-empty `sub`. An email is kept only when `email_verified` is `true` (with
  control characters stripped). A returning sign-in whose `sub` differs from
  the saved account is rejected. ChatGPT plan use requires
  `chatgpt.tokens.use.direct` in the token answer's `scope`: without it the
  sign-in is kept (as the documentation advises) with an *Allow ChatGPT plan*
  re-consent action alongside *Sign out*, and no turn, model list or token use
  happens. Tested: bad signature, wrong audience/azp/multi-aud, wrong nonce,
  expired and not-yet-valid tokens, missing `iat`, `crit`, non-canonical
  base64url, missing scope, declined consent, missing/dynamic/mismatched
  client ID, wrong state, slowloris deadline, body/duplicate Host, duplicate
  query params, wrong path/method/host, and `MAX_BAD_REQUESTS`. Missing scope
  and *Allow ChatGPT plan* re-consent, declined consent, the returning-sign-in
  `sub` mismatch, `invalid_client` handling, a missing refresh token and the
  refresh and sign-out behaviour are tested only in the Windows-only suites
  (`chatgpt-auth` `tests/flow.rs`, `desktop-server` `tests/chatgpt_route.rs`);
  the ID-token, callback and client-ID checks are unit tests that run on
  Linux and Windows.
- **Storage.** `<data_dir>/chatgpt/registration.dpapi` (host ID, issued
  client ID, account subject) and `<data_dir>/chatgpt/credentials.dpapi` (ID,
  access and refresh tokens, scopes, expiry), each encrypted with Windows
  DPAPI (`CryptProtectData`, current user, `CRYPTPROTECT_UI_FORBIDDEN`, an
  app-specific entropy, volatile zeroization of decrypted buffers, 1 MiB read
  cap) and replaced atomically (new temp file, `sync`, rename with brief
  sharing-violation retry on Windows). Never in `sessions/`, the workspace
  (which may not overlap the data directory), `providers.json`, browser storage
  or the ledger. On other systems nothing is written: sign-in fails closed with
  "not supported yet" instead of storing tokens in plain text (tested on Linux).
  DPAPI protects against other Windows users and copies of the files to another
  account or machine, **not** against code running as the same user.
- **Refresh.** An access token is refreshed (`grant_type=refresh_token`,
  issued `client_id`, `refresh_token`, `resource`, no `scope`) when it is
  expired or within 10 s of expiry, or when it is inside the refresh margin of
  `min(5 minutes, half its issued lifetime)` and the server's
  `earliest_refresh_at` (when given) has passed; so a short-lived token is not
  refreshed on every request (unit-tested in `manager.rs`). The refresh runs
  under both an in-process mutex and a cross-process lockfile
  (`<data_dir>/chatgpt/.lock`) that also cover sign-in completion and the
  storage part of sign-out; credentials are re-read under the lock, so
  concurrent callers share one refresh and a rotated refresh token is stored
  before anyone uses it. The lock is an operating-system
  file lock (`File::try_lock`: `LockFileEx` on Windows) held on an open handle
  to that file; the system releases it when the handle closes, including when
  the holding process dies, so nothing is judged by PID or age and no lock is
  ever taken over from a live holder. A waiter fails closed with "another
  ChatGPT sign-in operation is in progress" after a 10 s wait. The file is
  never deleted and its content is ignored. Tested on Windows both in-process
  (exclusion, release on drop, leftover files of any content) and across two
  real OS processes (`tests/vault_lock_multiprocess.rs`: exclusion, 10 s
  fail-closed timeout, kernel release on abrupt process termination,
  fail-closed sign-out preserving `credentials.dpapi`, and single shared DPAPI
  refresh across racing processes). If sign-out cannot take the lock, nothing
  is deleted and the answer says the sign-in is still on this computer. Status reads (Connect panel, sign-in polling)
  take neither lock. On HTTP 401 from Responses, the transport calls
  `force_refresh` and retries once (if the token changed). On a transient
  refresh failure while the current access token still has more than 10 s left,
  the still-valid token is returned instead of failing immediately.
  `invalid_grant`, `invalid_refresh_token`, `token_expired`,
  `refresh_token_expired`, `refresh_token_invalidated` and
  `refresh_token_reused` delete the tokens and mark the connection disconnected
  with that code; `invalid_client` (on refresh, exchange, or returning
  callback) also forgets the saved `client_id`/`subject` while keeping
  `ext_agent_host_id`; network or 5xx failures keep the sign-in.
- **Sign-out.** `POST /api/connections/chatgpt/logout` first deletes
  `credentials.dpapi` under both locks (keeping the registration for the next
  sign-in), so no refresh in this or another process can save rotated tokens
  afterwards. Then, with no lock held and from an in-memory copy of the refresh
  token (zeroized on drop), it discovers `revocation_endpoint` once from
  `{issuer}/.well-known/openid-configuration` (pinned to the exact issuer
  origin `https://auth.openai.com`, falling back to
  `https://auth.openai.com/api/accounts/oauth/revoke`) and revokes the refresh
  token (`token`, `token_type_hint=refresh_token`, `client_id`; retried twice
  on network or 5xx failures), all within a 45 s overall budget. Status reads,
  sign-in and turn starts are not blocked meanwhile. If revocation is not
  confirmed the UI says so and points to ChatGPT Settings. When a completed
  code exchange is discarded (the attempt was cancelled, the account differs
  from the saved one, the ID token fails its checks, or storage fails), its
  refresh token is revoked once, best effort, within 10 s and outside the
  locks. The ordering, the single discovery, the non-blocking status read, the
  budget and the discarded-exchange revocation are tested only in the
  Windows-only `tests/flow.rs` suite (DPAPI).
- **Where the token goes.** Only to `auth.openai.com` (refresh, revocation)
  and as `Authorization: Bearer` to `https://api.openai.com/v1/responses` and
  `/v1/models`. `agent::ChatGptTransport` and `agent::model_list` refuse any
  other host or path prefix except plaintext `http://127.0.0.1:<port>` (tests;
  `localhost` and `[::1]` are rejected). The ChatGPT route ignores
  `--api-base`, `RE_SEARCH_API_BASE` and saved base URLs; `PUT
  /api/settings/providers` refuses a key or address for `chatgpt` (including
  inside `roles`, and `redacted_view` strips any unsafe or `chatgpt` role
  `base_url`); no API-key variable (not even `RE_SEARCH_API_KEY` with
  `RE_SEARCH_PROVIDER=chatgpt`) is read for it. Test endpoints can be set only
  through `DesktopConfig::chatgpt_test`, a field the CLI and the app always
  leave empty, and only to `http://127.0.0.1:<port>`.
- **Tokens never cross the HTTP API, SSE, ledger or error strings.** Status,
  login and logout answers carry only states, fixed messages, OAuth error
  codes from a known list and the verified email. Transport errors are scrubbed
  of the token; `ChatGptTransport` HTTP errors keep only the HTTP status,
  `error.code`, `error.param` and documented recovery hints (never raw server
  `error.message` or `detail` strings). Temporary `Bearer` header strings and
  token-exchange buffers are zeroized on drop. The desktop test signs in, lists
  models, runs a turn with an approved `fs_write` and signs out, then checks
  every API answer, every SSE frame and every file under the data directory
  (ledger, blobs, metadata; the DPAPI files are ciphertext) for the tokens and
  the authorization code. Nothing in these paths writes a log.
- **Inference rules followed.** `store: false`, `stream: true`, the full
  history in `input` (no `previous_response_id`), the system prompt as
  `instructions`, function tools grouped in one `namespace` tool (`research`)
  with `"strict": false` and wire-time array `"items"` normalization (so
  schemas stay valid under OpenAI's Responses validator without altering
  `def_hash`), and none of the fields the preview rejects (`max_output_tokens`
  among them, so re:Search's `max_tokens` is not sent). A turn succeeds only on
  `response.completed` (with `status == "completed"` when present);
  `response.failed`, `response.incomplete`, an `error` event, or a stream that
  ends early return transport errors and abort the turn (tested, including SSE
  `error` with `error.param`, BOM/keepalive handling, and unterminated EOF
  rejection). Reasoning items are parsed into `Thinking` and not replayed in
  later requests.
- **Not verified.** No live sign-in, token exchange, refresh, revocation,
  model list or inference against OpenAI has been performed; all of it is
  tested against loopback mocks written from the documentation. The
  `#[cfg(windows)]` ChatGPT code and tests (DPAPI encryption/retry, the
  `chatgpt-auth` `tests/flow.rs` DPAPI suite, `desktop-server`
  `tests/chatgpt_route.rs`) have been compiled and run on Windows 11 with the
  sign-out/lock fixes; `ShellExecuteW` + COM initialization is compiled but
  never executed by any test (tests never open a browser). Not
  checked live: that the issued client accepts a callback port different from
  the one used at registration (the documentation says only the port may
  vary), the exact `revocation_endpoint` returned by live discovery, the exact
  shape of `function_call` output items under a `namespace` tool (the transport
  reads `call_id`, `name`, `arguments` and accepts an optional `namespace`),
  whether `namespace` must be repeated on replayed `function_call` input items
  (re:Search sends it), whether the model-list answer has more fields, and
  `ShellExecuteW` opening the browser. **Whether OpenAI requires approval for a
  local open-source app is unverified:** the documentation says these pages
  cover "open-source and locally hosted apps" and asks paid or remotely hosted
  apps to use an interest form; it states no approval step for local
  open-source apps, and mentions `force_reconsent=true` "only after OpenAI
  confirms deployment for your integration". Note also: signing out keeps the
  issued `client_id` and `subject` (per OpenAI's guidance for re-signing into
  the same account), so switching to a *different* ChatGPT account currently
  requires removing `<data_dir>/chatgpt/registration.dpapi` (no account-switch
  UI exists yet).

## The Antigravity route in the desktop (`antigravity`) — vendor-sandboxed, not confined to re:Search tools

The user chose to build this route knowing two things it cannot fix: Google's terms
for consumer plans may treat third-party wrapping of the Antigravity CLI as
unsupported or as a breach (account risk, the user's own), and the CLI's built-in
tools are outside re:Search's control. The Connect row, model group, composer
note, start event and session API identify the route as vendor-sandboxed.

- **Gate.** `POST /api/connections/antigravity/acknowledge` (header token) stores
  `antigravity_ack {acknowledged_ms, notice_version}` in `ui_settings.json`
  under a mutex with unique per-write temp files (so concurrent settings saves
  cannot resurrect a withdrawn acknowledgement). The UI calls it only when the
  person clicks the button under the full warning and confirms a dialog that
  repeats it. Without it (or for an older notice version), `POST
  /api/sessions/{id}/messages` refuses synchronously with HTTP 403 before
  mutating `meta.json` or starting a turn, `GET /api/models` omits the group,
  and **`agy.exe` is never spawned** (neither `--version` nor `models` runs
  before acknowledgement; tested). `PUT /api/settings/providers` cannot set it;
  `POST /api/connections/antigravity/withdraw` clears it, invalidates any
  in-flight `agy models` cache write via a generation counter, and stops any
  running Antigravity turn. Reopening an Antigravity session after withdrawal
  or disconnect still shows its confinement label in the session API and
  composer. Any holder of the launch token could call the endpoint, as with
  approvals; re:Search itself never does.
- **What runs.** Each turn starts the unmodified `agy.exe` (found via `--agy`,
  `RE_SEARCH_AGY`, `PATH`, WinGet's `Links` and `Packages\Google.AntigravityCLI_*`)
  in a kill-on-close Job Object with working directory `sessions/<id>/agy` and
  `--input-format stream-json --output-format stream-json --model <slug> --sandbox
  --disable-slash-commands --settings <json>` (no `-p`). The prompt goes on stdin as
  one stream-json `user` event (`{"event":"user","message":{"content":...}}`), then
  stdin is closed; per the "Headless mode" page the CLI finishes that turn and exits,
  and a prompt given with `-p` would be argv (and is dropped in this mode). The
  ledger's start-event blob is exactly that stdin line. Events are parsed in the
  documented shape, fields nested under the event's name (`{"event":"init",
  "init":{...}}`); an `init` with its fields at the top level is not accepted. Print
  mode was chosen over `agy acp`: its NDJSON stream fits the existing turn loop; core
  tools are allowed by rule, so the CLI never needs to ask about them (`core.exe`
  asks the person through re:Search's relay); and ACP's permission channel would only
  ever ask about tools this route wants denied. Print mode soft-denies anything that
  needs an approval it cannot get (documented), which fails closed. `--mode` is not
  passed: its values are `accept-edits`, `auto` and `plan` (the Settings page: `plan`
  prepends `/plan` to the prompt; the others approve edits), none stricter than
  Default; `agentMode: ""` in `--settings` selects Default, in which file edits
  need review and commands follow `toolPermission`. If `agentMode` were unset,
  headless runs use `auto` where available, which auto-approves workspace edits.
  `toolPermission: strict` is not used: it prompts even when an allow rule matches,
  so core tools could never run headless.
- **The settings (the only thing holding built-ins back).** `toolPermission:
  request-review`, `enableTerminalSandbox: true`, `allowNonWorkspaceAccess: false`,
  `permissions.deny = [command(*), read_file(*), write_file(*), read_url(*),
  execute_url(*)]`, `permissions.allow = [mcp(core/*)]`. Per Google's permission docs,
  deny > ask > allow, so a user-level allow rule for those kinds cannot win, and MCP
  calls without a rule default to Ask, which print mode soft-denies. `mcp(*)` cannot
  be denied (it would beat the core allow), so other servers' tools rely on that
  default and on the user's own settings not allowing them.
- **Tool config & caller binding.** `sessions/<id>/agy/.agents/mcp_config.json`
  names only `core` (with the same env and session token as the Claude route's
  `mcp.json`, plus explicit `CORE_CALLER_ROUTE=antigravity` — and Claude turns
  now explicitly set `CORE_CALLER_ROUTE=claude-code-cli` — so `core.exe` cannot
  be mislabelled by host environment variables, and the approval relay checks
  that `request["caller"]` matches the turn's expected caller), `enabledTools`
  = exactly the tools `core.exe` serves, each `eager` so the model sees it as
  `mcp_core_<tool>` and `init` lists it. Written only *after* `version()` and
  pre-spawn stop checks (with mode `0o600` on Unix), deleted on every exit path
  (`ToolConfigGuard`, after `ProcGuard` kills the process tree), and swept when
  stale (`TOOL_CONFIG_FILES`; checks live PID on Windows and `/proc/<pid>` on
  Linux) at server start and when a session is opened. Global MCP servers
  (`~/.gemini/config/mcp_config.json`) still load; re:Search never reads that
  file.
- **Handshake: "core tools reachable + settings applied".** Built-ins cannot be
  removed, so the `init` check is: `permission_mode == "request-review"` (missing, or
  `always-proceed`, aborts), `tools` contains only strings, every expected
  `mcp_core_*` tool is offered, and no other `mcp_core_*` tool is offered. Anything
  else (or exiting before a verified `init`) aborts (`session.abort`) before model
  output is accepted. `cli.init` records the full tool list, the built-in tools, the
  permission mode and the cwd. Route binding inspects the first `session.start` or
  `session.resume` with a `route` (and emits `session.start` even if files were
  uploaded before Turn 1), preventing route mixing after pre-Turn-1 uploads.
- **Built-in tool use is recorded, never hidden.** Once `init` is verified,
  only exact expected `mcp_core_<tool>` names (or `call_mcp_tool` with
  `ServerName: core` and an expected `ToolName`, remembered per `step_index`
  across frames) are treated as core tools, preventing lazy global MCP servers
  from spoofing the `mcp_core_` prefix. Every other tool `step_update`
  (including non-`"tool"` steps carrying `tool_name` or `tool_info`, steps
  without `step_index`, and terminal states such as `DONE`, `FAILED`, `ERROR`,
  `DENIED`, `CANCELLED`) is recorded as `vendor.builtin_tool` (first sighting
  and terminal/error frames, with state, name, clipped parameters and any
  error) and shown as a warning line live and when the session is reopened.
  Only what the CLI prints is seen: a built-in the CLI runs without reporting
  it is invisible to re:Search.
- **Guarantees that do NOT hold on this route** (for the CLI's own tools): routing
  through re:Search's router and policy, person-given approval cards, the ledger
  record of the call and its result, the AppContainer worker, and the "only our
  tools" check. They still hold for re:Search's own tools served by `core.exe`.
  Partial provenance as on the Claude route (the start event's `raw_blob` is the exact
  stdin text; every raw line is a blob).
- **Credentials.** re:Search never reads, copies or passes the CLI's cached login and
  never reads files under the user's Antigravity/Gemini folders. With
  `GEMINI_API_KEY`, `GOOGLE_GEMINI_BASE_URL` or `AGY_ADC_AUTH` set (from the CLI's
  changelog and Settings page) the turn is refused, names only (tested in both
  `cli-route` and `desktop-server`); `RE_SEARCH_ALLOW_AGY_ENV_AUTH=1` proceeds
  and records the names. The `init` event reports no credential source, so
  there is no second check.
- **Not verified (no live turn has been run; every test uses a scripted fake
  `agy`; the fake-CLI suites have been run on Windows 11):** that the CLI honours
  the deny rules and `agentMode` given through `--settings` (merge semantics
  with the user's `settings.json` are undocumented); that the workspace
  `mcp_config.json` is loaded in an untrusted, non-git folder under the data
  directory in headless mode; that eager tools appear in `init` as
  `mcp_core_<tool>`; that `--sandbox` works on this Windows build; the exact
  live shape of `step_update` for tools (`tool_name`, `tool_info.error`) and
  whether the final `agent_response` frame repeats the whole text (both
  handled); that `--input-format stream-json` without `-p` runs headless as the
  Headless page says, and that `--disable-slash-commands` (documented for print
  mode) is accepted with it; that the inline `--settings` JSON survives Windows
  command-line quoting as one argument; the exit code when a built-in is
  soft-denied (documented as 0); and whether `agy models` and `--version` (run
  after acknowledgement) trigger a self-update (observed once, 1.5.13 →
  1.5.14).

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
  Model inference happens off-box (over HTTPS to Anthropic, OpenAI—including
  the ChatGPT-plan Responses route—or Gemini on the API route, through a local
  Ollama server, or through `claude.exe`/`agy.exe` on the subscription-CLI
  routes), and the remote provider's weights and sampling remain untrusted
  external computation.
- **External CLI routes (`--route cli`, desktop `claude-cli` and `antigravity`)
  retain partial provenance and tool-config token exposure.** `claude.exe` and
  `agy.exe` run uncontained in a kill-on-close Job Object (they can read and
  write what you can, and use the network, through their own code), control the
  external loop, and read `sessions/<id>/mcp.json` or
  `sessions/<id>/agy/.agents/mcp_config.json` (`CORE_SESSION_TOKEN`) during the
  turn. On `antigravity`, built-in tools also remain outside re:Search's
  control (see above).
- **Supervisor & LaTeX compiler run in the user's logon context.** While the
  Python worker runs inside the zero-capability AppContainer on Windows, host
  supervisor operations (`fs_read`, approved `fs_write`/`latex_patch`, approved
  `latex_compile` invoking `pdflatex`/`latexmk` with `-no-shell-escape`,
  `openin_any=p`, `openout_any=p`, and `shell_escape`/`shell_escape_commands`
  removed, and allowlisted literature HTTP calls) execute as the `re-search`
  user, and any uncontained local process running as the same OS user shares
  access to `providers.json`, loopback ports, and environment memory.
- **The desktop token is as powerful as your provider keys.** Whoever holds
  the token can change a provider's saved `base_url` and then start a turn,
  which sends that provider's stored key to the new host; it can also attach
  any directory outside the data directory as a workspace, which the model
  then reads without asking. Treat the token like the keys.
- **The app window's residual limits.** In `re-search-app` the token is in
  the memory of `re-search-app.exe` and of the WebView2 renderer, in the
  page's `<meta>` tag and `sessionStorage`; any program running as you can
  read process memory. If WebView2 drops the header, the fallback hands the
  token to the page through `ExecuteScript` (guarded by an exact-origin check,
  but it is a second path). WebView2 is an Evergreen component that Windows
  updates outside re:Search, so its behaviour (including header navigation)
  can change without a re:Search release. Downloads go to
  `%USERPROFILE%\Downloads`; a Downloads folder moved elsewhere is not
  followed. Permission prompts (camera, microphone, ...) use WebView2's
  default dialog. A second launch exits quietly without focusing the
  running window. Further limits: on a WebView2 runtime too old to support
  InPrivate, the library used (`wry`) silently creates a normal profile, in
  which case the token's `sessionStorage` copy and the stream cookie would be
  written under `%LOCALAPPDATA%\re-search\webview` (the app does not check
  this; the Evergreen runtime on Windows 11 supports it). If the WebView2
  process crashes, its crash dump in that folder can contain the per-launch
  token. The app reads `re-search-app.json` beside the executable and looks
  for `python/worker.py` beside it and in up to three parent folders, so the
  install folder and its parents must not be writable by other users (the
  default `%LOCALAPPDATA%\Programs\re-search` is not).
- **Dependencies of the app window.** `wry` and `tao` are compiled only into
  `re-search-app`, on Windows only. `tao` is Apache-2.0 only. The Windows
  build includes three Unicode data crates under "(MIT OR Apache-2.0) AND
  Unicode-3.0", and `Cargo.lock` pins `idna_adapter` to `1.1.0` to keep the
  larger ICU set (Unicode-3.0 only) out; a plain `cargo update` would bring
  it back. `Cargo.lock` also lists MPL-2.0 crates (`cssparser`, `selectors`,
  and others) that `wry` uses only on Linux and macOS; they are never
  compiled on Windows.
- **Where the desktop launch credential is exposed (`re-search ui`, and
  `re-search desktop` when it falls back).** `re-search ui` no longer places
  the live API token in the launch URL; it opens `/?launch=<nonce>` using a
  single-use, 60-second launch nonce (`/?token=` is rejected). Once the
  browser redeems it on first load (or after 60 s), the URL in browser history
  or process command lines (`cmd.exe` / browser argv) is dead (replays return
  403). Residual exposure: a local process able to read command-line arguments
  (via `/proc` on Linux or process inspection as the same user on Windows)
  during the brief window *before* the browser redeems the nonce could race it
  to obtain the token; `--print-url-json true` still writes the token to
  stdout; and after redemption the token lives in the page's `sessionStorage`
  (readable by same-user processes inspecting the browser profile or memory).
  The server speaks plain HTTP on loopback. Header-token authentication and
  visual KaTeX rendering in `re-search-app` (WebView2 158) and the
  `re-search ui --no-open --print-url-json` `/?launch=<nonce>` redemption flow
  are verified on Windows 11; the live `cmd /C start` default-browser spawn in
  `launcher.rs` is compiled on Windows (and disabled under `#[cfg(test)]`)
  without opening a browser in automated tests.
- **Approved LaTeX compilation relies on TeX's `openin_any=p` / `openout_any=p`
  enforcement.** `latex_compile` runs `pdflatex`/`latexmk` with
  `-no-shell-escape`, sets `openin_any=p` and `openout_any=p` (paranoid mode:
  forbids reading or writing absolute paths or `..` parent paths outside the
  working directory / TEXMF), and removes `shell_escape`,
  `shell_escape_commands`, and provider API-key variables from the child
  environment (verified on both Linux and Windows with a `rustc`-compiled mock
  `pdflatex` binary in `write_and_compile.rs`; real TeX Live/MiKTeX enforcement
  of `openin_any=p` / `openout_any=p` on Windows is not verified here). Because
  the compiler runs outside AppContainer as your OS user, any bug or
  distribution-specific bypass of `openin_any=p` would still run with your user
  permissions; compiling always asks first, so read what will be compiled.
- **Literature endpoints can be redirected to loopback.** The HTTPS host
  allowlist is fixed, but `--arxiv-base` / `--inspire-base` /
  `--crossref-base` and the matching `RE_SEARCH_*_BASE` environment variables
  accept `http://127.0.0.1:<port>` (used by the tests); whoever controls the
  launch environment can point searches at a local service.
- **Desktop server resource use.** One thread per connection with no cap.
  Authenticated requests may carry 60 MiB bodies, with a larger limit for up to
  five Base64-encoded 10 MiB uploads plus framing. A local process can tie up
  threads by opening many connections.
- **MCP route without an approval relay.** When `core.exe` runs without the
  supervisor's approval channel, every tool that must ask (code execution,
  workspace writes, and now literature search) is denied; nothing falls back to
  allow.
- **Policy version.** The ask-every-time rule for `network-read` ships as
  policy `v0.4-research`. A session recorded under an earlier research policy
  string (CLI or desktop) cannot be resumed or continued under it (drift
  check, exit `2`).
- **Literature queries go to third parties once approved.** After you approve
  a query, its text is sent to the search host and recorded in the ledger;
  read the query before approving.
- **Unprivileged Windows symbolic links vs NTFS junctions in tests.** On
  Windows, `sandbox_and_fs.rs` and `write_and_compile.rs` unconditionally test
  `..`, absolute-path, drive, alternate-data-stream, reserved-name, and NTFS
  directory junction escapes (escaping, broken, and intra-workspace loop
  junctions across `WorkspaceSandbox`, `fs_list`, `fs_write`, `latex_patch`,
  `latex_compile`, and `tex_audit`). Creating Windows file/directory symbolic
  links (`symlink_file`) requires Developer Mode or
  `SeCreateSymbolicLinkPrivilege` and runs opportunistically when permitted on
  Windows (and unconditionally under `#[cfg(unix)]`).
- **Heuristic & LLM checks are not formal kernel proofs.** `sympy_eval`,
  `check_expression_sanity`, `tex_audit`, and LLM Step/Full Verifiers catch many
  algebraic and citation errors, not formal theorem-prover guarantees.
