# Worker delegation and model routing in research-core (main @ 7c36a0e) — read-only map

Scope read in full: `crates/agent/src/{delegate,explore,autonomous,runner,lib,transport,model_list}.rs`, `crates/desktop-server/src/{agent_models,subagents,routes_subagents,model_catalog,turn_runner}.rs`, `crates/desktop-server/tests/{stage14_worker_delegation,subagents_and_effort}.rs`. Read in part: `agent/src/{resume,guardrails,interaction,responses,openai,wire,gemini,honesty,autonomous_prompts}.rs`, `tools/src/{router,registry,canon}.rs`, `ledger/src/lib.rs`, `research-tools/src/{fs_tools,paper_passages,policy_preset,lib,proof_dag,investigation*}.rs`, `desktop-server/src/{chatgpt,connections,routes_session,session_view,session_summary,state}.rs`, `assets/{trace_cards,subagents}.js`. Nothing was built or run except one `node` one-off in the scratchpad that re-implements two pure functions (marked below). No network, no user data dir, no ledger of the live session was read.

## 1. How a `delegate_*` call runs, end to end

**Per-turn setup (desktop-server).** `run_session_turn` resolves four worker transports (`crates/desktop-server/src/turn_runner.rs:208-239`), custom subagents (`:241-249`), a separate Lean router (`:250`), and builds `DelegationConfig` unconditionally (`:274-342`) — there is no Investigation-mode awareness here, although `investigation::prepare` has already run and stamped `req["_paper_*"]` flags (`turn_runner.rs:81`; `crates/desktop-server/src/investigation.rs:478-576`). `make_worker_spec` hard-codes `max_tokens: WORKER_MAX_TOKENS` and `reasoning_effort: None` (`turn_runner.rs:133-153`).

**Offering the tools.** The autonomous loop appends an "Orchestrator -> Worker Delegation" section to the system prompt (`crates/agent/src/autonomous.rs:91-97`, resume `:215-223`; text in `crates/agent/src/delegate.rs:184-207`) and passes the config into `LoopParams` (`autonomous.rs:350`). `run_turn_leg` appends `delegation_tool_specs` to the registry tools (`crates/agent/src/runner.rs:247-255`). Delegation tools are **not** in the router registry, so they are not in `session.start.tool_defs` and are not subject to the resume drift check.

**Dispatch.** `dispatch_tool_calls` is a sequential `for` over the turn's `tool_use` blocks (`runner.rs:549-590`); delegation names go to `run_delegation_call` (`runner.rs:559-568`). Several `delegate_*` calls in one orchestrator turn therefore run one after another.

**`run_delegation_call`** (`delegate.rs:376-488`):
1. `delegate_explore` branches off to `run_parallel_exploration` (`:385-387`).
2. Validates `task`; resolves the worker (`resolve_target_worker`, `:231-259`).
3. Whitelist = `allowed_tools` minus delegation tools (`:397`); tool specs taken from the main router, plus `lean_check` from `lean_router` for the Lean worker only (`:312-324`).
4. Caller id `worker:<role>:<wire provider>:<session>` (`:402`); system prompt = role prompt + pinned conventions + "Delegated Worker Rules" (`:261-275`); user prompt = `Task:` + optional `Relevant Context:` + a role-specific suffix (`:277-293`).
5. Emits `delegate.start` (`:406-410`).
6. Loop `1..=max_turns` (`:417-463`): fresh `ModelRequest` with the full un-masked worker history (`:420-423`); streaming send with a cancel-only observer (`:425-430`); raw response body to a blob + `delegate.turn` (`:431-443`); `validate_turn` (a `max_tokens` stop is a hard failure, `runner.rs:495-501`); `EndTurn` ends OK (`:446`); `ToolUse` on the last turn aborts with "worker reached max_turns" and keeps only the text of that turn (`:449`); otherwise each tool call goes through `call_worker_tool` → `router.try_call(worker_caller, …)` (`:327-373`), which denies non-whitelisted tools (`:340-342`) and `ledger_append` with `entry_type: "final_answer"` (`:343-345`).
7. Final text → blob; `fit_return_json`; `delegate.end`; returns `(is_error, canonical JSON)` (`:465-487`).

**Limits and where they live**

| Limit | Value | Defined | Applied |
|---|---|---|---|
| Worker turns (default) | 8 | `delegate.rs:24` | `turn_runner.rs:289,301,313`; custom subagents `delegate.rs:252` |
| Worker turns (hard clamp) | 12 | `delegate.rs:25` | `delegate.rs:400` (unreachable today: every caller passes 8) |
| Worker max output tokens | 8,192 | `delegate.rs:26` | `turn_runner.rs:150`, `delegate.rs:253`. **Not sent on the ChatGPT route** (`crates/agent/src/responses.rs:193-194`); sent as `max_completion_tokens` (`openai.rs:196`) / `maxOutputTokens` (`gemini.rs:156`) |
| Worker tool-output clip | 8,000 chars of canonical JSON | `delegate.rs:28` | `delegate.rs:368` |
| Return summary clip | 2,800 chars | `delegate.rs:27` | `delegate.rs:295-306` |
| Return JSON cap | 3,800 **bytes**; summary limit halves (2800→1400→700→…→120) until it fits | `delegate.rs:29` | `delegate.rs:301-304` |
| `task` in `delegate.start` | 500 chars | — | `delegate.rs:409` |
| Explore branches | 2..=4 | `explore.rs:18` | `explore.rs:280` |
| Explore turns per branch | 6 | `explore.rs:19` | `explore.rs:313` |
| Explore tool-output clip | 6,000 chars | `explore.rs:20` | `explore.rs:185` |
| Explore return cap | 3,800 bytes; per-branch summary budget 2,400/N chars, approach 220 | `explore.rs:21` | `explore.rs:220-263` |
| Ledger copy of any tool result | 4,000 chars + `truncated` flag | `runner.rs:20` | `runner.rs:578-588` (the model still receives the full string, `:589`) |
| Orchestrator defaults | 48 turns / 16,384 tokens | `runner.rs:14-15` | `turn_runner.rs:176-187` |
| Orchestrator tool-output masking | keep newest 6 tool-result messages, mask older in strides of 8 (48-char stub) | `guardrails.rs:17-18` | `guardrails.rs:115-165`, `autonomous.rs:347` |
| `fs_read` | default 8,000 / max 12,000 chars | `research-tools/src/fs_tools.rs:20-21` | — |
| `search_paper_passages` | default 5 / max 10 hits, ≤1,800 bytes or 8 lines each | `research-tools/src/paper_passages.rs:21-28` | — |
| ChatGPT request timeout | 600 s per request (connect 15 s) | `responses.rs:66-67` | — |

There is no wall-clock budget for a delegation and no retry/backoff: any transport error ends the worker (`delegate.rs:429`, `explore.rs:138-141`).

## 2. What the orchestrator sees vs. what stays in the ledger

**Returned to the orchestrator** (one tool result, ≤ ~3.8 kB): canonical JSON with `worker, worker_name, provider, model, routing_reason, ok, turns, input_tokens, output_tokens, tools_used, proof_entries, raw_blob, summary` and `error` on failure (`delegate.rs:468-487`). On cancel it is the plain string `DENIED: stopped by user` (`:483`). For explore: totals plus `branches[]` each with a clipped `summary` and `raw_blob` (`explore.rs:233-247`).

`raw_blob` is a dead reference for the model: no registered tool reads a blob by hash (`get_blob` appears only inside handlers at `research-tools/src/investigation.rs:355`, `proof_dag.rs:534`, `tex_audit_sources.rs:184`). The only channel wider than `summary` is Proof-DAG entries the worker writes (`proof_entries`, collected at `delegate.rs:361-367`), readable through `ledger_query`.

**Ledger/blob only:**
- `delegate.start`: role, model, provider, routing reason, task (≤500), exposed tools, max turns (`delegate.rs:406-410`).
- `delegate.turn` per worker model turn: usage, stop reason, tool-call ids/names, `raw_blob` = full provider response (`:431-443`).
- Each worker tool call: `tool.request` / `policy.decision` / approval events / `tool.result` under the `worker:…` caller with full args and results, inline or blob (`crates/tools/src/router.rs:92-164`).
- `delegate.end`: the return payload + `call_id`, `duration_ms`, `error`; `raw_blob` = the worker's full unclipped final text (`delegate.rs:466-481`).
- `model.tool_result` for the orchestrator: text clipped to 4,000 chars (`runner.rs:578-588`).
- Explore: `explore.start`, one `explore.branch` per branch (final text blob only), `explore.end` (`explore.rs:315-397`).

**UI only (not ledger):** thinking deltas such as `[Worker (model): task]`, `[Worker -> tool]` (`delegate.rs:346,411,482`). The session view turns `delegate.end`/`explore.end` into one status line (`session_view.rs:487-510`) and counts tokens (`session_summary.rs:137-141`); the tool card shows model, turns, tokens, routing reason, tools (`assets/trace_cards.js:179-209`).

**Not recorded anywhere:** worker request bodies (the orchestrator's are, `runner.rs:309-319`), hence the worker's system prompt and effort; the full `task`/`context` survive only in the orchestrator's `model.tool_use` event (`runner.rs:484-486`).

## 3. How worker models are chosen

`resolve_worker_transport` (`desktop-server/src/agent_models.rs:311-330`), in order, each step falling through silently if the transport cannot be built:
1. **Request override** — `req[<role>] = {provider, model}` → `request_override` (`:319-323`). API-only: no UI code sends role objects in a message body (only `assets/subagents.js:7` mentions the role keys, for the settings page) — inference from grep.
2. **Settings role** — `ui.agent_models[<role>]` → `assigned_in_settings` (`:324-328`); saved through `PUT /api/settings/providers` (`routes_session.rs:536-539`); CLI routes are rejected for helper roles (`agent_models.rs:114-120`).
3. **Auto smaller** — `catalog.pick_smaller_model(primary_provider.cli_name(), primary_model)`, **same provider only** → `auto_smaller_catalog_model` (`:293-301`).
4. **Fallback** — primary provider and model in a fresh isolated context → `isolated_primary_model` (`:303-308`).

Custom subagents: own `provider`+`model` → `subagent_assigned_model`, else steps 3-4 (`:332-352`).

**`pick_smaller_model`** (`desktop-server/src/model_catalog.rs:124-157`): requires a healthy in-memory entry for the provider (`ok_ms` set and no `last_error`, `:137-140`), then returns the first listed id ≠ primary that contains, in tier order, `haiku`, `mini`, `flash`, `nano` (`:134`) and none of `embedding/embed/tts/whisper/image/audio/realtime` (`:125-133`).

**There is no static catalog data.** Entries are filled only as a side effect of `GET /api/models` (`model_catalog.rs:369-414`; ChatGPT at `:318-350`) and, for Ollama, the connections check (`connections.rs:316`). The whole catalog is cleared when keys or base URLs are saved (`routes_session.rs:611-614`), when the ChatGPT login generation changes (`chatgpt.rs:112-123`) and on logout (`chatgpt.rs:269`). `pick_smaller_model` ignores the cache TTL.

**Why a ChatGPT-sign-in session on `gpt-6.2-sol` gets `isolated_primary_model`.** Steps 1 and 2 did not apply (no override, no role assigned), and step 3 returned `None`. Provable from code: the ChatGPT list is `slug`s with `visibility == "list"` from `GET /v1/models` (`agent/src/model_list.rs:247-251`); a slug only qualifies if it contains `haiku|mini|flash|nano`; `gpt-6.2-sol` does not, and it is the only production ChatGPT id anywhere in the repo (`agent/src/transport.rs:20`; it is also the entire fallback list, `model_catalog.rs:277-285`). Step 3 never looks at another provider, so a connected Anthropic or OpenAI key with a haiku/mini model does not help unless assigned in settings. **Not verified:** the live list itself — I cannot tell whether the plan listed no tier-named sibling (most likely, inference from the `-sol` naming) or the catalog entry was missing/cleared/errored at turn start; both produce the same reason string.

Which providers can get a smaller model by this rule: Anthropic (`haiku`), OpenAI (`mini`, then `nano`), Ollama (only if a local tag happens to contain a tier word), ChatGPT plan (only if a slug contains `mini`/`nano`), Gemini (matches, but wrongly — see bug 3).

## 4. Concurrency today, and what N parallel section readers would need

**Today.** Only `delegate_explore` is parallel. `run_parallel_exploration` uses `std::thread::scope`, one OS thread per approach, joined in index order (`explore.rs:335-353`). Each branch (`run_branch`, `explore.rs:85-218`) shares `&dyn Transport` (trait is `Send + Sync`, `transport.rs:353`), `&Router`, and the `CancelToken`; it has its own caller id `worker:explorer_b<N>:…` (`:101`). Branches do **not** get the `Audit`: `Audit` holds `&mut dyn EventSink` and `&Ledger` (`runner.rs:64-69`), and `Ledger` wraps a SQLite connection (`ledger/src/lib.rs:244-249`) that the router guards with a `Mutex` (`router.rs:52`). So branches return a `BranchOutcome`, and the main thread writes blobs and `explore.branch` events after the join, then forwards the router's interleaved events to the sink with `flush_interleaved` (`explore.rs:354-373`). Tests prove real concurrency with a `Barrier` stub (`explore.rs:428-493`) and an HTTP mock (`stage14_worker_delegation.rs:353-404`).

All other delegation is sequential (`runner.rs:549-590`).

**What N paper-section readers need:**
1. A fan-out entry point. Either a delegation tool taking a `sections[]` array (same shape as `approaches`), or an app-driven pre-pass before the first orchestrator turn. The second needs a new public function in `agent`, because `Audit` and `run_delegation_call` are `pub(crate)` (`runner.rs:64`, `delegate.rs:376`).
2. The same "threads return data, main thread records" pattern. To keep per-turn provenance (which explore lacks, bug 7), branches must return their raw response bodies for blobbing after the join, or the recorder must be restructured.
3. A cap on N (precedent: 4) — each reader holds one streaming request; a plan usage-limit error fails the branch with no retry (`explore.rs:138-141`; hint text `responses.rs:327-330`).
4. Live progress. The UI receives nothing between the "launching" line and the join (`explore.rs:327-354`); a channel drained by the main thread would be needed.
5. Tool whitelist for readers. `fs_read` and `search_paper_passages` are `ReadWorkspace` → policy `Allow` (`policy_preset.rs:27`; `fs_tools.rs:73`; `paper_passages.rs:46`), so parallel readers raise no approval prompts. Note `search_paper_passages` holds the router's ledger mutex for the whole scan and re-verifies the session chain on every call (`paper_passages.rs:57,135`), so concurrent searches serialize; `fs_read` takes no lock (`fs_tools.rs:85`).
6. A larger per-reader return than explore's 2,400/N chars (`explore.rs:221`).

Cancellation already works across threads (`transport.rs:313-328`; checks at `explore.rs:111,172`).

## 5. Adding a `paper_reader` role with a whitelist and a bigger structured return

**agent crate**
- `delegate.rs` is 578 lines (22 under the 600 limit): put the role in a new module, as `explore.rs` did. Hooks in `delegate.rs`: tool-name const (`:20-23`), `is_delegation_tool` (`:104-109`), a `paper_reader_tools()` list (pattern `:111-124`), `DelegationConfig` field (`:57-65`), `delegation_tool_specs` (`:160-182`), `format_delegation_prompt_section` (`:184-207`), `resolve_target_worker` (`:231-259`), `build_worker_user_prompt` (`:277-293`), the "keep your final reply concise" rule (`:271`), and the caps `DELEGATE_RETURN_SUMMARY_CHARS`/`MAX_RETURN_JSON_LEN`/`WORKER_TOOL_OUTPUT_CHARS` (`:27-29`) with `fit_return_json` (`:295-306`).
- `lib.rs:45-51` re-exports.
- `runner.rs:559-568` needs no change if `is_delegation_tool` covers the new name. `runner.rs:589` passes the returned string through uncapped, so the 2,800 limit is purely `delegate.rs`.
- **Resume, critical:** once a return exceeds 4,000 chars the ledger copy is truncated (`runner.rs:20,620-626`) and resume rebuilds it from the nearest preceding `delegate.end`/`explore.end`, requiring that the event payload minus `call_id`/`duration_ms` canonicalizes to exactly the returned string, or that `raw_blob` equals it (`resume.rs:321-345`). Otherwise the session becomes un-resumable ("output reconstruction mismatch"). A new event kind needs a new arm there. No test covers a delegated return above 4,000 chars (`delegate.rs:554` asserts `< 3900`).
- `honesty.rs:274-278` maps `delegate.start` roles to campaign phases; add the new role (tests `:434-437`).
- `autonomous_prompts.rs:38-40` and `delegate.rs:166,194` describe literature delegation as ".tex reading"; nothing mentions PDFs or Investigation.
- Masking: a pointer bundle in orchestrator context is replaced by a 48-char stub once 6 newer tool-result messages plus a stride of 8 accumulate (`guardrails.rs:115-165`), so pointers must be persisted (ledger) rather than only held in context.

**Structured return.** A worker's result is free text (`collect_text`, `delegate.rs:446`); nothing enforces a schema. The robust option is a worker-only tool whose schema-checked arguments (`router.rs:148`) are the pointers `(source_id, path, start_line, end_line, label)`; its handler validates the range against the extracted text and stores/returns the app-derived excerpt and hash, and the delegation return is assembled from those records rather than from model prose. This is the piece that lets record calls accept pointers and keep excerpts honest by construction.

**Worker-only tool exposure.** Every main-registry tool is offered to the orchestrator (`runner.rs:247-252`) and enters `tool_defs`. Two consequences:
- Registering a new tool (or changing the description/schema of any existing one, e.g. relaxing `record_paper_*` excerpts) changes `def_hash` (`tools/src/registry.rs:26-35`) and makes every existing session fail resume with "tool definition drift" unless it is added to the allow-list at `resume.rs:202-206` (new id + `ReadWorkspace`) or given a compatibility shim like `resume.rs:208-213`.
- The existing precedent for a worker-only tool is the separate `lean_router` (`turn_runner.rs:540-550`; `delegate.rs:312-324,350-357`), currently special-cased on `DELEGATE_LEAN`/`lean_check`; generalizing it to a per-worker extra router avoids drift entirely and still routes every call through `Router::try_call`.
- Letting a reader call `record_paper_*` directly is only a whitelist edit: those handlers do not inspect the caller (no `caller` reference in `research-tools/src/investigation*.rs`).

**desktop-server**
- `agent_models.rs:18-30` `AGENT_ROLE_KEYS` (tests `:427-487`).
- `turn_runner.rs:208-239` (resolve transport), `:133-153` (`make_worker_spec`: add effort and token parameters), `:274-342` (config; this is where Investigation-only gating on `req["_paper_*"]` would go). File is 550 lines.
- `subagents.rs:21-39` `RESERVED_IDS`, `:67-169` `builtin_agents_json`.
- `assets/subagents.js:7` `WORKER_ROLE_KEYS`; `assets/trace_cards.js:171-177,303-306` titles and card routing.
- `session_view.rs:487-510`, `session_summary.rs:137` only if new event kinds are introduced.
- `research-tools/src/lib.rs:225-258` if a tool is registered in the main registry.

**Tests to touch or copy**
- `agent/src/delegate.rs:529-577` (whitelist, `final_answer` guard, lean-router routing) — add reader guards.
- `agent/src/explore.rs:455-493` — concurrency pattern for N readers.
- `agent/tests/autonomous_and_streaming.rs:542-547` and the literals at `delegate.rs:526`, `explore.rs:467-472`, `turn_runner.rs:141-152` break if `DelegatedWorkerSpec` gains a field.
- `desktop-server/tests/stage14_worker_delegation.rs`: roster and CLI-provider rejection (`:443-448`), role settings (`:449-457`); model the reader scenario on the literature test (`:296-351`).
- `desktop-server/tests/stage15_interactive_hitl.rs:433-435` (worker tool lists must not contain `ask_user`).
- `desktop-server/tests/subagents_and_effort.rs:120-129` (builtin list) and `:206` (pattern for asserting effort on the wire).
- `agent/src/resume.rs` tests for a >4,000-char delegated return and, if applicable, the drift allow-list (`:226-235`).
- `desktop-server/src/model_catalog.rs:486-539` if the smaller-model picker is fixed.
- Not read: `desktop-server/tests/investigation_flow.rs`, `agent/tests/effort_wire.rs` (grep only).

## 6. Reasoning effort for workers

Confirmed `None` for all four built-in workers: `turn_runner.rs:151` → `DelegatedWorkerSpec.reasoning_effort` → `ModelRequest` (`delegate.rs:422`, `explore.rs:122`). With `None` every codec omits the field (`responses.rs:220-223`, `openai.rs:197-203`, `wire.rs:45-48`, `gemini.rs:157-164`), so workers run at the provider's default effort. The session effort (`subagents.rs:376-405`) reaches only the orchestrator (`turn_runner.rs:383,419` → `runner.rs:300`). The same `None` applies to verifier, strategist, narrator and explain (`verifiers.rs:554`, `strategist.rs:111`, `narrator.rs:194`, `sidetalk.rs:175`).

A lower effort for readers is expressible:
- In code, trivially: the field exists and is honoured; only `make_worker_spec` pins it to `None`.
- Today without code changes: a custom subagent with `tools_profile: "read_only"`, `reasoning_effort: "low"` and its own model (`subagents.rs:284`, `turn_runner.rs:338`, `delegate.rs:254`); it gets `fs_list, fs_read, arxiv_search, literature_search, search_paper_passages, search_formal_library, doc_ingest, ledger_query, tex_audit` (`delegate.rs:132`), but still 8 turns and the 2,800-char return.
- Floor is `low`: only `low|medium|high|xhigh|max` are accepted (`transport.rs:200-206`). Ollama never sends effort (`openai.rs:205-207`). Settings have no per-role effort — `AgentModelSpec` is provider + model only (`agent_models.rs:12-16`).

## 7. Proven bugs and inconsistencies

1. **Literature worker is told to do what its whitelist forbids.** Its user prompt says to record each result as a `reference_lookup` proof entry and return the entry id (`delegate.rs:289-290`); that is a `ledger_append` entry type (`research-tools/src/proof_dag.rs:15-25`), and `ledger_append` is not in `literature_worker_tools` (`delegate.rs:116-119`), so the call is denied (`delegate.rs:340-342`). Not covered: the stage14 mock never tries it (`stage14_worker_delegation.rs:303-308`).
2. **A worker's default `fs_read` is always cut, and loses its line metadata.** `fs_read` returns 8,000 content chars by default (`fs_tools.rs:20`) inside an object whose canonical form sorts keys, putting `content` first (`fs_tools.rs:287-300`; `tools/src/canon.rs:17-20`); workers then clip that JSON to 8,000 chars (`delegate.rs:368`) or 6,000 (`explore.rs:185`). Result: the tail of the text plus `end_line`, `end_offset`, `start_line`, `total_lines`, `truncated` are replaced by `...`. My `node` re-implementation gives 7,985 content chars kept and no `start_line` for an unescaped 8,000-char read; LaTeX escapes make it worse. A reader that must report line ranges is blind unless it asks for a smaller `max_chars`. Five default `search_paper_passages` hits (≤1,800 bytes each) overflow the same clip.
3. **`mini` matches `gemini`.** `lower.contains("mini")` (`model_catalog.rs:134,151`) is true for every Gemini id, so the "smaller" model is simply the first listed non-primary one (re-implementation: primary `gemini-2.5-flash` with a list starting `gemini-3-pro-preview` returns the pro model). The unit test only exercises the errored-Gemini case (`model_catalog.rs:524-530`).
4. **"Smaller" is not checked against the primary.** Only equality is excluded (`model_catalog.rs:145`), so primary `gpt-4.1-nano` yields `gpt-4.1-mini` (re-implementation), and a primary that is already `…-mini` yields some other `mini`.
5. **Auto-smaller routing depends on incidental in-memory state.** It works only if `GET /api/models` succeeded since the last catalog clear (see section 3), so the same session can route to the primary on one turn and a smaller model on the next. The settings label promises "Default (Auto Smaller Model)" (`assets/subagents.js:345`). No test asserts either `auto_smaller_catalog_model` or `isolated_primary_model`.
6. **Summary budget mixes chars and bytes.** Clipping counts chars (`delegate.rs:209-216`) but the fit test is the byte length of JSON-escaped text (`:301`) with coarse halving (`:304`), so replies heavy in backslashes, newlines or non-ASCII math drop from 2,800 to 1,400 or 700 chars.
7. **Explorer branches have no per-turn provenance.** No raw response blobs and no per-turn events (`explore.rs:85-218`); only the final text is stored (`explore.rs:356-373`), unlike `delegate.turn` (`delegate.rs:431-443`). The module doc claims every turn is logged (`delegate.rs:4-7`).
8. **Running out of turns discards the worker's work.** On the last turn a tool request ends the worker with only that turn's text, usually empty (`delegate.rs:449`, `explore.rs:162-166`); there is no forced wrap-up turn. With 8 turns a reader has at most 7 tool rounds.
9. **Conflicting guidance about who reads.** The base prompt and the delegation section push `delegate_literature` for "bulky `.tex` reading" (`autonomous_prompts.rs:38-40`, `delegate.rs:194`), while the Investigation instructions tell the orchestrator itself to use `fs_read` and `search_paper_passages` and to supply verbatim excerpts (`research-tools/src/investigation.rs:43-50`). The literature worker prompt asks for "exact passages, source paths, line ranges" (`turn_runner.rs:297-299`) while the shared worker rules demand a concise reply (`delegate.rs:271`) that is then cut to 2,800 chars. This matches the live run's zero delegations (interpretation).
10. **Custom subagent delegation ignores per-agent limits**: always 8 turns and 8,192 tokens (`delegate.rs:252-253`).
11. **File-length rule already exceeded by `wc -l`**: `agent/src/runner.rs` 626, `agent/src/verifiers.rs` 867, `desktop-server/src/investigation.rs` 810, `research-tools/src/investigation.rs` 771, `desktop-server/src/session_view.rs` 753. I did not find how the project measures the 600-line rule, so this may be counted differently.

Checked and found sound: the `final_answer` guard (`delegate.rs:343-345`) against the exact-match entry-type check in `proof_dag.rs:135`; recursion bar via `is_delegation_tool` filtering (`delegate.rs:397`, `explore.rs:301`, `delegate.rs:134`); resume reconstruction for today's returns, which stay under 4,000 chars including the explore branch-picker addition (`interaction.rs:11,162-193`).