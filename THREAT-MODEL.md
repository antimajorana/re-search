# Threat model (v0)

This file says plainly what v0 protects against and what it does not. Read
the second section before approving anything.

## What v0 is

A supervisor (`re-search`) starts the Claude Code CLI headless. The CLI
runs the model's agent loop and is given exactly two tools, both served by
our MCP server `core`:

- `sympy_eval` runs Python code in a local worker process.
- `ledger_note` appends a note to the session ledger.

Every tool call passes through one router: record the request, ask the
policy, ask the human if the policy says "ask" (on every call in v0, or
reusing an active `worker-session` approval only when the worker runs
inside a contained sandbox), run, record the result.

## What v0 does NOT contain

**The Python worker is not sandboxed.** Code approved for `sympy_eval` runs
as you, with your rights. It can:

- read, change and delete any file you can (documents, keys, browser
  profiles, this ledger),
- use the network (send your files anywhere, download and run anything),
- start other programs.

**Your approval is the only gate.** There is no second line of defence
behind it. Read the code in the prompt before typing `y`. Code that looks
like mathematics can still open files or sockets. Because v0 runs under
the uncontained `JobObjectSandbox` backend, `sympy_eval` asks for your
approval on every call (`once`); `worker-session` reuse activates only
under a contained sandbox backend (`AppContainer`, Stage 2).

Other things v0 does not do:

- **The policy engine is policy and audit, not a security boundary.** It
  decides what the core will dispatch. It cannot restrain code once that
  code is running.
- **The ledger is append-only, not tamper-proof.** The schema rejects
  UPDATE, DELETE, REPLACE and out-of-order sequence numbers, also for other
  programs that open the database. But anyone who can write the file can
  drop the triggers or swap the file, and so can approved worker code. There
  is no hash chain in v0.
- **The agent loop is outside the core.** The Claude Code CLI decides what
  the model sees and when it calls tools. The core records the CLI's output
  stream and its own tool calls, not the model's full context. Sessions are
  therefore recorded as "external loop, partial provenance".
- **Other local programs are trusted.** The approval channel is a loopback
  TCP port protected by a random per-session token that is stored in the
  session's `mcp.json`. Any program running as you can read that file. Such
  a program could also just run code itself, so this adds nothing new, but
  the token is not a defence against it.
- **Nothing is verified.** `sympy_eval` returns a computation. Notes are
  stored as `unverified`. No tool can mark a claim as verified, and v0 has
  no checker; the only statuses are `unverified` and `tested` (bounded
  evidence).
- **Prompts and results leave the machine.** They go to Anthropic through
  the CLI, under your own subscription login.

## What v0 does do

- **The model gets only our tools.** The supervisor starts the CLI with
  built-in tools off, user and project settings off, and only our MCP
  config. It then checks the CLI's `system/init` event: the tool list must
  be exactly `mcp__core__ledger_note` and `mcp__core__sympy_eval`, and
  `core` must be the only MCP server. On any difference the session is
  killed before the model acts. The CLI version is checked both via
  `--version` and at `system/init`; mismatches emit a stderr warning and a
  `cli.version_warning` ledger event while keeping the `system/init` check
  active.
- **Code execution always asks.** `execute-code` tools can never be
  "allow": the policy engine turns that into "ask" whatever the rules say.
- **Containment-gated approval scope and epoch binding.** Every human
  approval is bound by SHA-256 digest over the effective scope, caller,
  tool id, tool-definition hash, canonical arguments, policy version, and a
  fresh nonce. While v0 runs uncontained (`JobObjectSandbox`), the router
  downgrades `sympy_eval` to `once` so every call requires a single-use
  human approval. When a contained sandbox backend (`AppContainer`, Stage 2,
  restricting both filesystem and network access) is active,
  `worker-session` reuse activates: the first `sympy_eval` call (and the
  first call after any worker crash, timeout, or restart) requires a fresh
  human approval, and further `sympy_eval` calls by the same caller with
  the same tool-definition hash and policy version reuse that approval
  only while that exact OS worker epoch remains alive (enforced under the
  worker mutex). `Once`-scoped tools always require a single-use approval
  per call.
- **No approval without a person.** If the supervisor's stdin or stderr is
  not a terminal, or the token, session, channel version, or scope does not
  match, the answer is "no". There is no auto-approve switch. Stray lines
  typed on the console while no prompt is active are drained and discarded.
- **Approval channel timeout & ACK.** If `core` disconnects or times out
  while waiting for a decision, the supervisor closes the prompt and logs
  `approval.expired` instead of leaving a stale prompt open; valid answers
  require an explicit ACK from `core` and record `approval.delivered`
  (meaning `core` received the reply prior to digest re-check, not that
  the code ran) or `approval.undelivered`. If a late ACK crosses the 5 s
  timeout, `approval.consumed` and `tool.result` in the ledger are
  authoritative over `approval.undelivered`.
- **The prompt says what is not contained.** It shows the code, the effect
  class, the approval scope, and the sandbox backend's own statement.
- **Process lifetime is contained.** The CLI and the worker each run in a
  Windows Job Object with kill-on-close. A call that exceeds its timeout
  (default 120 s) gets the whole worker tree killed; the next call starts a
  fresh worker (and invalidates any prior worker-session approval when
  running contained). When the supervisor exits, everything it started dies.
- **The worker protocol cannot be corrupted by prints.** It runs on two
  dedicated pipes, not stdin/stdout.
- **Worker clutter and bytecode caches are isolated per session.** TEMP/TMP,
  the matplotlib config, and the Python bytecode cache (`PYTHONPYCACHEPREFIX`)
  go to per-session directories (`sessions/<id>/scratch`,
  `sessions/<id>/pycache`), and user site-packages are off. Each session
  therefore pays a ~15 s cold import cost on its first worker start rather
  than sharing compiled bytecode across sessions.
- **Failing closed.** If the ledger cannot record a step (in either the
  tool router or the approval server), the call is refused.

## Known gaps to close later

- A real sandbox backend (`AppContainer`, Stage 2) behind the existing
  `Sandbox` trait, so that approval stops being the only gate and
  `worker-session` reuse can safely activate.
- A hash chain or external anchor for the ledger.
- An in-core agent loop (API-key route) for full provenance.
- Handle inheritance: the worker's two pipe handles are inheritable for a
  moment, so another process started by the same program at the same instant
  could inherit them too. v0 starts one worker at a time.
