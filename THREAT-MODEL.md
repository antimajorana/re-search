# Threat model (Stage 3: AppContainer isolation & hash-chained ledger)

This file says plainly what re:Search currently protects against and what it
does not. Read the remaining limits before approving anything.

## Architecture overview

A supervisor (`re-search`) starts the Claude Code CLI headless inside a
Windows Job Object. The CLI runs the model's agent loop and is given exactly
two tools, both served by our MCP server `core`:

- `sympy_eval` runs Python code in a local worker process confined inside a
  zero-capability Windows `AppContainerSandbox`.
- `ledger_note` appends a note to the session ledger.

Every tool call passes through one router: record the request, ask the policy,
ask the human if the policy says "ask" (reusing an active `worker-session`
approval only while the contained worker epoch remains alive), run, record the
result.

## What Stage 2 contains

- **Zero-capability Windows AppContainer for the Python worker.** When
  `CORE_PYTHON` and `CORE_WORKER_SCRIPT` are configured, `core` fails closed
  unless `AppContainerSandbox` initializes cleanly (requiring per-session
  `CORE_SCRATCH` and `CORE_PYCACHE`). The worker runs with `TokenIsAppContainer
  == 1`, Low integrity (`S-1-16-4096`), and `CapabilityCount = 0` (no LPAC).
  Network egress (loopback TCP/UDP, local-NIC UDP, DNS resolution, external
  datagrams) and named pipes to the host are blocked by the AppContainer/WFP
  boundary, and `JobObjectBasicUIRestrictions` blocks clipboard read/write,
  external USER handles, desktop creation/switching, global atoms, display or
  system parameter changes, and `ExitWindows`.
- **Scoped worker handle inheritance (`STARTUPINFOEXW`).** Both
  `AppContainerSandbox` and `JobObjectSandbox` launch workers via
  `CreateProcessW` with `STARTUPINFOEXW` + `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`
  (`core` spawns no other child processes). Only the worker's two dedicated
  pipe handles and parent-opened `\\.\NUL` stdio handles are inherited by the
  worker.
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
  (`CORE_SESSION_TOKEN`), `ledger.db`, other sessions' scratch directories, and
  user files outside `scratch`/`pycache` are denied (`ERROR_ACCESS_DENIED` /
  `EACCES`).
- **Containment-gated `worker-session` approval reuse and epoch binding.**
  Every human approval is bound by SHA-256 digest over the effective scope,
  caller, tool id, tool-definition hash, canonical arguments, policy version,
  and a fresh nonce. `WorkerEpochAdapter` enables `worker-session` reuse only
  when `containment().is_contained()` is `true`: the first `sympy_eval` call in
  a live worker epoch prompts for `worker-session` approval, and subsequent
  calls by the same caller with the same tool-definition hash and policy version
  reuse that grant only while that exact OS worker epoch stays alive. Timeout,
  crash, or restart ends the epoch and forces a fresh prompt. Any uncontained
  fallback or `once` tool remains pinned to per-call `once`, and the supervisor
  rejects `worker-session` requests unless they declare `contained: true`.
- **The model gets only our tools.** The supervisor starts the CLI with
  built-in tools off, user and project settings off, and only our MCP config,
  and verifies `system/init` (`mcp__core__ledger_note`, `mcp__core__sympy_eval`,
  and only `core` connected) before the model acts.
- **Code execution always asks.** `execute-code` tools can never be configured
  to `allow`: the policy engine clamps `allow` to `ask`.
- **No approval without a person.** If stdin or stderr is not a terminal, or
  the token, channel version, session, scope, or containment check fails, the
  answer is "no". There is no auto-approve switch. Stray console input outside
  an active prompt is drained and discarded.
- **Approval channel timeout & ACK.** Disconnecting before a decision closes
  the prompt and records `approval.expired`; valid answers require an explicit
  ACK from `core` (`approval.delivered` / `approval.undelivered`).
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
- **The agent loop is outside the core (Stage 4).** `claude.exe` runs
  uncontained as you (inside a kill-on-close Job Object) and controls what the
  model sees and when it calls tools; sessions are recorded as `"external loop,
  partial provenance"`. Prompts and tool results leave the machine to Anthropic
  through the CLI. Any unrelated program running as you can also read
  `sessions/<id>/mcp.json` (`CORE_SESSION_TOKEN`).
- **Nothing is verified.** `sympy_eval` returns computations, and `ledger_note`
  records `unverified` notes.
