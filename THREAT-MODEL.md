# Threat model (v0)

This file says plainly what v0 protects against and what it does not. Read
the second section before approving anything.

## What v0 is

A supervisor (`research-core`) starts the Claude Code CLI headless. The CLI
runs the model's agent loop and is given exactly two tools, both served by
our MCP server `core`:

- `sympy_eval` runs Python code in a local worker process.
- `ledger_note` appends a note to the session ledger.

Every tool call passes through one router: record the request, ask the
policy, ask the human if the policy says "ask", run, record the result.

## What v0 does NOT contain

**The Python worker is not sandboxed.** Code approved for `sympy_eval` runs
as you, with your rights. It can:

- read, change and delete any file you can (documents, keys, browser
  profiles, this ledger),
- use the network (send your files anywhere, download and run anything),
- start other programs.

**Your approval is the only gate.** There is no second line of defence
behind it. Read the code in the prompt before typing `y`. Code that looks
like mathematics can still open files or sockets.

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
  killed before the model acts. The CLI version is recorded, because its
  headless defaults are expected to change.
- **Code execution always asks.** `execute-code` tools can never be
  "allow": the policy engine turns that into "ask" whatever the rules say.
- **An approval is for one exact call.** It is bound to a SHA-256 digest
  over the caller, the tool id, the hash of the tool definition, the
  canonical arguments, the policy version and a fresh nonce. It is used up
  once, and it is checked again immediately before the code is sent to the
  worker. Approving a call does not approve the next one, even an identical
  one. (The design allowed "one approval per worker session"; v0 chooses the
  stricter per-call rule.)
- **No approval without a person.** If the supervisor's stdin is not a
  terminal, or the token or session does not match, the answer is "no".
  There is no auto-approve switch.
- **The prompt says what is not contained.** It shows the code, the effect
  class and the sandbox backend's own statement.
- **Process lifetime is contained.** The CLI and the worker each run in a
  Windows Job Object with kill-on-close. A call that exceeds its timeout
  (default 120 s) gets the whole worker tree killed; the next call starts a
  fresh worker. When the supervisor exits, everything it started dies.
- **The worker protocol cannot be corrupted by prints.** It runs on two
  dedicated pipes, not stdin/stdout.
- **Worker clutter is redirected.** TEMP/TMP and the matplotlib config go
  to a per-session scratch directory, and user site-packages are off. The
  Python bytecode cache goes to one directory shared by all sessions
  (`<data>/cache/pycache`), because a fresh cache costs about 15 s per
  worker start. This is hygiene, not containment, and the shared cache has
  a cost: code approved in one session can alter cached bytecode that later
  sessions load.
- **Failing closed.** If the ledger cannot record a step, the call is
  refused.

## Known gaps to close later

- The CLI version is recorded but not enforced: a version other than the
  one this was developed against (2.1.318) runs without a warning. The
  init check still applies to it.
- A stale approval prompt: if nobody answers within 15 minutes the call is
  denied, but the prompt stays on screen. A late `y` is then logged by the
  supervisor as approved although nothing runs (the core's own record says
  denied).

- A real sandbox backend (AppContainer or similar) behind the existing
  `Sandbox` trait, so that approval stops being the only gate.
- A hash chain or external anchor for the ledger.
- An in-core agent loop (API-key route) for full provenance.
- Handle inheritance: the worker's two pipe handles are inheritable for a
  moment, so another process started by the same program at the same instant
  could inherit them too. v0 starts one worker at a time.
