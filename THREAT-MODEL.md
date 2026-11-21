# Threat model (Stages 1–8: AppContainer isolation, hash-chained ledger, multi-provider loop, research tools & loopback desktop UI)

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
  alternate data streams and symlinks escaping root. The sandbox has no
  per-file deny-list; instead `ResearchEnv::with_workspace` refuses to attach a
  workspace that is this launch's data directory, contains it, or lies inside
  it, so that launch's `providers.json`, `ledger.db` and `blobs/` are not
  under its workspace root. CLI `--workspace`, the MCP server and the desktop
  server (at launch and on `POST /api/sessions`) all go through it; the check
  itself and the desktop route are tested. It compares against the data
  directory in use only: a workspace containing *another* re:Search data
  directory (for example the default `%LOCALAPPDATA%\re-search` while
  running with `--data-dir` elsewhere) is not refused. Archive
  (`tar.gz`/`zip`) and PDF
  ingestion enforce anti-bomb byte/file/page caps, reject archive symlinks, and
  record `"source.ingested"` `"raw_blob"` digests.
- **Three new effect classes and how the policy treats them.** Stage 6 adds
  `read-workspace` (`fs_list`, `fs_read`, `doc_ingest`, `tex_audit`),
  `write-workspace` (`fs_write`, `latex_patch`) and `network-read`
  (`arxiv_search`, `literature_search`). The research policy `v0.4-research`
  allows `read-workspace` and `observe-local` without asking, asks for
  `write-workspace`, `execute-code` (`sympy_eval`, `latex_compile`) and
  `network-read`, and denies everything else (including `network-egress`).
- **Unconfigurable human approvals for writes, code and literature queries.**
  `policy::evaluate` hard-clamps `allow` to `ask` for `execute-code`,
  `write-workspace` and `network-read`, so no rule or preset can switch these
  prompts off; a `deny` rule stays `deny`. For `write-workspace`,
  `ConsoleApprover` and `UiApprover` display a unified diff preview
  (`preview_workspace_edit`) and SHA-256 request digest, store pre-write file
  backups in `blobs/` (`"workspace.backup"`), and record `"approval.answered"`
  before modifying any workspace file.
- **Literature queries ask every time (user decision, 2026-11-20).** The query
  text of `arxiv_search` / `literature_search` is chosen by the model, so it is
  a channel by which a manipulated model could carry workspace content to a
  search host. Every model-initiated call is therefore approved per call
  (`once` scope; there is no per-session reuse), and the console banner and the
  desktop approval card show the full arguments, including the query, before
  anything is sent. A refused call makes no network request (tested). Searches
  the person types into the desktop UI's literature drawer
  (`POST /api/sessions/{id}/literature`, token-protected) are human-authored and
  run without a second prompt.
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
- **Loopback `127.0.0.1` desktop server (`crates/desktop-server`).** `re-search
  ui` / `desktop` binds exclusively to `127.0.0.1` (`std::net` + `httparse`,
  zero `tokio`) and serves the UI, an HTTP/JSON API and an SSE event stream.
  Credentials and checks, per request:
  - *Host.* Every request must carry a loopback `Host` (`127.0.0.1[:port]` or
    `localhost[:port]`), which defeats DNS-rebinding pages.
  - *Two secrets per launch.* A 256-bit `getrandom` **token** and a separate
    256-bit **stream cookie** value. `re-search ui` opens the browser at
    `/?token=…`; the `re-search-app` window instead requests `/` with the
    token in the `X-ReSearch-Token` request header (see below). Only a load
    of `/` that already carries a valid token (query or header) gets the token
    written into the page (`<meta>` tag), and it sets the
    `HttpOnly; SameSite=Strict` cookie `__research_stream_<port>`. A page on
    another origin cannot attach that header to a navigation, and a `fetch`
    carrying it needs a CORS preflight the server never grants.
    The UI removes the token from the address bar and keeps it in
    `sessionStorage` (per origin, port included, per tab). A reload carries
    only the cookie and is served the page *without* the token, so the cookie
    can never be exchanged for the token; `/` without either answers 403.
  - *Every `/api/` route except `GET /api/health`* (which returns only `ok`
    and the version) requires the token in the `X-ReSearch-Token` or
    `Authorization: Bearer` header, compared in constant time; otherwise 401.
    This covers listing sessions, session contents, workspace listings and file
    contents, the proof DAG, diffs, Side-Talk, provider settings and all
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
  - *Approvals.* Model tool calls that need approval appear as cards in the UI
    (`UiApprover`), bound to the request digest and recorded in the ledger
    (fail-closed) before the tool runs; answering needs the header token.
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
  - KaTeX `0.19.1` is served offline from the binary; the UI loads nothing
    from the network.

## The desktop window (`re-search-app`, `crates/desktop-app`)

- **One process, no browser.** `re-search-app.exe` starts the loopback
  server in-process (`open_window: false`, `print_url_json: false`) and runs
  one WebView2 window (`wry 0.57.3` + `tao 0.37.3`) on the main thread.
  Closing the window shuts the server down and exits the process, which also
  ends child processes held in kill-on-close Job Objects. `re-search desktop`
  starts it from its own directory (passing only `--data-dir`, `--workspace`,
  `--python`, `--worker-script`, `--port`, never a token); with any other flag,
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
- **Where the desktop token is exposed (`re-search ui`, and `re-search
  desktop` when it falls back).** It is in the launch URL, which
  appears on the command line of the browser process that is started (and of
  `cmd.exe` when the default browser is used), readable by other accounts on
  Linux via `/proc` and by programs running as you on Windows, and possibly
  in browser history, although the UI removes it from the address bar on
  load. With `--print-url-json true` the URL and token are also written to
  stdout. It then lives in the page's `sessionStorage`. Any program running
  as you that can read the browser profile or process memory can obtain it.
  The server speaks plain HTTP on loopback.
- **An approved LaTeX compile can read files outside the workspace.**
  `latex_compile` runs with `-no-shell-escape`, but TeX's own file-read
  restrictions (`openin_any`) are left at the distribution's default, so a
  `.tex` file can `\input` any file you can read, including a data
  directory's `providers.json`, and place it in the PDF or log. Compiling
  always asks first; read what will be compiled.
- **Literature endpoints can be redirected to loopback.** The HTTPS host
  allowlist is fixed, but `--arxiv-base` / `--inspire-base` /
  `--crossref-base` and the matching `RE_SEARCH_*_BASE` environment variables
  accept `http://127.0.0.1:<port>` (used by the tests); whoever controls the
  launch environment can point searches at a local service.
- **Desktop server resource use.** One thread per connection with no cap, and
  an authenticated request may carry a 60 MiB body; a local process can tie up
  threads by opening many connections (local denial of service only).
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
- **Symlink escapes are tested on Unix only.** The workspace sandbox's
  symlink-escape tests are `#[cfg(unix)]`; on Windows the `..`, absolute-path,
  drive, alternate-data-stream and reserved-name rejections are tested, but
  symlink and junction escapes are not.
- **Heuristic & LLM checks are not formal kernel proofs.** `sympy_eval`,
  `check_expression_sanity`, `tex_audit`, and LLM Step/Full Verifiers catch many
  algebraic and citation errors, not formal theorem-prover guarantees.
