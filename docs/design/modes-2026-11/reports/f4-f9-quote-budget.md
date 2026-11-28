# Report: `f4-f9-quote-budget` (F4 quote core & F9 run budget / run state)

- **Base commit:** `49edc18dd66b85044046dacac186af3593a44de3` (`origin/claude/project-thread-caszr2`)
- **Branch:** `work/f4-f9-quote-budget` (pushed to `gemini/modes/f4-f9-quote-budget`)
- **Toolchain on Linux VM:** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `cargo 1.100.0`, `python3 3.12.16` (`sympy 1.14.0`, `numpy 2.5.5`, `mpmath 1.3.0`), `node v22.23.5`
- **Guard rails:** `Cargo.lock` untouched; `crates/research-tools/tests/golden/tool_defs.txt` untouched; no tool added or modified; `policy` and `ledger` untouched; `check_file_length.py` passes (all touched/added source files <= 600 lines).

---

## 1. What was implemented

### Package F4 — Quote core (`crates/research-tools/src/quote_core.rs`, `lines_sent.rs`)

| File | Lines | Contents |
|---|---:|---|
| `crates/research-tools/src/quote_core.rs` | 472 | `QuoteRef`, `VerifiedQuote`, `QuoteFault`, `QuoteRejection`, `quote_sha256`, `clip_to_chars`, `clip_quote`, `verify_quote`, `verify_quote_ref`, `accept_quote`, `accept_quote_with_sent`, `EvidenceRole`, `EvidenceModeMeta`, `EvidenceQuotePayload`, `EvidenceAbstractPayload`, `quote_id`/`parse_quote_id` (`xe-<seq>`), `abstract_id`/`parse_abstract_id` (`xa-<seq>`), provisional constants (`EVIDENCE_QUOTE_EVENT`, `EVIDENCE_ABSTRACT_EVENT`, `QUOTE_MATCHED_WORD`) |
| `crates/research-tools/src/quote_core_tests.rs` | 335 | 5 unit tests covering exact match, relocation within radius 12, out-of-radius `quote_not_found` + `true_text`, wrong file hash (both on disk and expected `file_sha256`), no Investigation marker required, `accept_quote` verification vs `LinesNotSent` (including caller isolation, wrong hash, relocated-unsent, and `read_paper_lines` tool results), `evidence.quote`/`evidence.abstract` validation & round-trip & IDs, and 300-char UTF-8 boundary clipping on multi-byte text |
| `crates/research-tools/src/lines_sent.rs` | 465 | `CONTEXT_SUPPLIED_EVENT`, `READ_MAP_NODE_TOOL`, `LineRange`, `SuppliedRange`, `ContextSuppliedPayload`, `SentFileKey`, `SentLines` (`ranges_for`, `covers`, `all_entries`, `is_empty`, `merge_ranges`), `lines_sent`, `lines_sent_with_ledger`, crediting `context.supplied` + `read_paper_lines` + `search_paper_passages` + `fs_read` + placeholder `credit_read_map_node_result` |
| `crates/research-tools/src/lines_sent_tests.rs` | 426 | 5 unit tests covering overlapping/adjacent range merging, caller/hash/`run_id` isolation (string and integer `run_id`), `read_paper_lines` inline + blob + `line_truncated` + `ok: false` + `run_id` mismatch, `search_paper_passages` + `fs_read` (slice, query clipping, `workspace` root exclusion, empty slice) + `read_map_node` placeholder, and `ContextSuppliedPayload` validation & `deny_unknown_fields` |

### Package F9 — Run budget and run state (`crates/research-tools/src/run_budget.rs`)

| File | Lines | Contents |
|---|---:|---|
| `crates/research-tools/src/run_budget.rs` | 464 | `RUN_ID_FIELD`, `IDEA_EXCHANGE_EVENT`, seven `RUN_STATE_LABEL_*` constants, `RunScope`, `ModelCallBreakdown`, `count_model_calls_breakdown`, `count_model_calls`, `BudgetStatus`, `ModelCallBudget`, `RunStateAction`, `RunState` (7 states), `InvestigateTaskState` + `investigate_task_run_state`, `ExploreStageState` + `explore_stage_run_state`, `SolvePauseReason` (all 9 reasons) + `solve_pause_run_state`, `SolveRunState` + `solve_run_state` (`independence` intentionally omitted per brief) |
| `crates/research-tools/src/run_budget_tests.rs` | 397 | 11 unit tests covering empty slices, `model.request` vs `model.response`/`model.error`/`model.refusal`, mixed emitter families, `delegate.*` and `explore.*` per-`call_id` deduplication (completed, failed final turn, mid-run, unkeyed), all `RunScope` variants, `ModelCallBudget` operations/saturation/serde, and exhaustive total-mapping tests over every variant of `RunState::ALL`, `InvestigateTaskState::ALL`, `ExploreStageState::ALL`, `SolvePauseReason::ALL`, and `SolveRunState` |

### Wiring edit

| File | Lines | Change |
|---|---:|---|
| `crates/research-tools/src/lib.rs` | 272 | Added `pub mod lines_sent;`, `pub mod quote_core;`, and `pub mod run_budget;` (3 lines, in its own commit). |

---

## 2. Ledger audit: exact fields read by F4 (`lines_sent`) and F9 (`run_budget`)

### F4 — How line-returning tool results are recorded today (`crates/tools/src/router.rs`)

`Router::call_inner` (`crates/tools/src/router.rs:132-143`) records `tool.request` with:
- `call_id` (string), `caller` (string), `tool` (string), `tool_def_hash`, `effect`, `approval_scope`, `args` (`{"inline": "<canonical json>"}` or `{"blob": "<sha256>", "bytes": N}`).
- **Note:** `tool.request` does **not** carry a `run_id` field today.

`Router::dispatch_auth` (`crates/tools/src/router.rs:411-423`) records `tool.result` with:
- `call_id` (string), `ok` (`bool`), `result` (`{"inline": "<canonical json>"}` or `{"blob": "<sha256>", "bytes": N}` when canonical JSON exceeds `INLINE_LIMIT = 4096` bytes — common for `read_paper_lines`, `search_paper_passages`, and `fs_read`).

Exact fields read by `lines_sent_with_ledger` in `crates/research-tools/src/lines_sent.rs`:
1. **`context.supplied`** (provisional constant `CONTEXT_SUPPLIED_EVENT`):
   - `payload.run_id` (string or unsigned integer matching canonical decimal string)
   - `payload.caller` (non-empty string)
   - `payload.ranges[]`: `{ source_id, path, file_sha256, start_line, end_line }`
2. **`tool.request`**:
   - `payload.call_id`, `payload.tool` (fallback `payload.tool_id`), `payload.caller`, optional `payload.run_id`, optional `payload.args` (inline/blob/object).
3. **`tool.result`** (only when `payload.ok == true`):
   - `payload.call_id`, `payload.result` (decoded from `inline`, `blob` via `Option<&Ledger>`, or direct JSON object).
   - **`read_paper_lines`** (`crates/research-tools/src/paper_lines.rs:140-155`): reads `a_meta.path`, `a_meta.file_sha256`, `a_meta.start_line`, `a_meta.end_line`, `a_meta.line_truncated` (if `true`, subtracts 1 from `end_line` because the last line was cut mid-line). Because `a_meta` omits `source_id`, `source_id` is resolved in order: `a_meta.source_id` -> decoded `tool.request.args.source_id` -> prefix before `/` in `a_meta.path`.
   - **`search_paper_passages`** (`crates/research-tools/src/paper_passages.rs:61-82,152-157`): reads `passages[]` elements `{ source_id, path, file_sha256, start_line, end_line, start_byte, excerpt }`. If `start_line == 1 && start_byte > 0`, `start_line` advances to 2 (line 1 started mid-line). If `excerpt` does not end with `\n` and `excerpt.len() >= 900`, `end_line` decrements by 1 (last line may have been cut at `next_excerpt`'s `MAX_EXCERPT_BYTES / 2` boundary).
   - **`fs_read`** (`crates/research-tools/src/fs_tools.rs:281,288-301,405-416`): requires `root` to be `"sources"` or absent (rejects `"workspace"`), reads `path`, `file_sha256`, and `source_id` (or prefix before `/` in `path`). In query mode (`matches[]`), credits `(line, line)` per match unless `text.chars().count() > 500 && (text.starts_with("...") || text.ends_with("..."))`. In slice mode, requires `returned_chars != Some(0)` and non-empty `content`, crediting `(start_line, end_line)`.
   - **`read_map_node`** (`READ_MAP_NODE_TOOL`): named match arm and placeholder `credit_read_map_node_result` (no-op until Investigate package V6 defines the tool's output schema).

### F9 — How provider calls are recorded today (`runner.rs`, `autonomous.rs`, `delegate.rs`, `paper_read.rs`, etc.)

Audited every `send_prepared` / `send_prepared_streaming` call site in `crates/agent/src/`:
1. **`crates/agent/src/runner.rs`**:
   - `model.request` (line 310): `{ turn, model, message_count, max_tokens, raw_blob }` — emitted immediately *before* `send_prepared_streaming`.
   - `model.response` (line 340): `{ turn, response_id, model, stop_reason, usage, duration_ms, raw_blob }`.
   - `model.error` (line 467): `{ turn, kind, status, message, raw_blob? }` — emitted either on `prepare_request` failure (before `model.request`, zero network calls) or on `send_prepared_streaming` failure (after `model.request`).
   - **Attribution gap:** None of `model.request`, `model.response`, or `model.error` carries any `run_id`, `caller`, `task`, or `request_seq` field.
2. **`crates/agent/src/autonomous.rs`**:
   - Calls `run_turn_leg` (emits `model.request`), `verifiers::verify_delivered_answer` (emits `verifier.request { target_entry_id, kind, model, raw_blob }` at `verifiers.rs:574`), and `strategist::consult_strategist` (emits `strategist.request { consultation_number, model, question, raw_blob }` at `strategist.rs:120`). None carries `run_id` or `caller`.
3. **`crates/agent/src/delegate.rs`** and **`crates/agent/src/paper_read.rs`**:
   - Neither emits `model.request`, `model.response`, or `model.error`.
   - Instead they emit `delegate.start { call_id, role, ..., marker_seq? }`, `delegate.turn { call_id, role, reader?, turn, model, stop_reason, usage, tool_calls, raw_blob }` (only after a successful response), and `delegate.end { call_id, ..., turns }`, where `turns` counts attempted turns (including a final failed/cancelled turn that emitted no `delegate.turn`). `delegate.turn` has `call_id` and `role`, but no `run_id`.
4. **`crates/agent/src/explore.rs`** (`delegate_explore`):
   - Emits `explore.start`, `explore.branch { call_id, index, approach, ok, turns, ... }`, and `explore.end { call_id, ..., turns }` — no `model.request` and no `run_id`.
5. **`crates/agent/src/narrator.rs`**:
   - Emits `narrator.request { model, entry_count, raw_blob }` (no `run_id`).
6. **`crates/agent/src/sidetalk.rs`**:
   - Emits `sidetalk.request` on a separate session key `sidetalk:<base_session>`, so it never mixes into the main session ledger.

**Conclusion on attribution & smallest field to add (not added in this brief):**
- Today's events **cannot** attribute a provider call to a specific run, task, or caller by payload alone when multiple runs interleave in one session; they can only be scoped sequentially by ledger sequence number (`RunScope::AfterSeq` / `RunScope::SeqRange`) when runs do not overlap.
- **Smallest field that solves attribution:** one optional string field `"run_id"` (`run_budget::RUN_ID_FIELD`) added to `model.request` (and to `verifier.request`, `strategist.request`, `narrator.request`, `delegate.turn`, `delegate.end`, `explore.branch`, `explore.end`, and `idea.exchange`), matching the `run_id` already specified on `context.supplied` in F4. If per-caller attribution (e.g. Solve's `{solver, verifier, workers}`) is also desired on `model.request`, a second optional `"caller"` field suffices. Neither field was added to `crates/agent/` in this brief.

---

## 3. Deviations, decisions taken alone, and assumptions

### Deviations from the brief's pseudocode signatures
- **`verify_quote` signature:** The brief wrote `verify_quote(session_events, source_id, path, start_line, end_line, quote)`. Because `SourceIndex::load` reads and SHA-256-verifies the source file from disk inside `SandboxRoot::Sources`, `verify_quote` takes `(sandbox: &WorkspaceSandbox, session: &str, session_events: &[Event], source_id: &str, path: &str, start_line: usize, end_line: usize, quote: &str)`. Passing `session` explicitly also handles empty `session_events` cleanly without panicking or guessing the session ID.
- **`lines_sent` signature:** Provided both `lines_sent(events: &[Event], run_id: Option<&str>) -> SentLines` (pure over events) and `lines_sent_with_ledger(events: &[Event], ledger: Option<&Ledger>, run_id: Option<&str>) -> SentLines` so tool results larger than `INLINE_LIMIT` (4,096 bytes), which `Router` stores in `blobs/`, are resolved when a `Ledger` is available.

### Decisions taken alone
1. **`quote_sha256` normalization:** Defined `quote_sha256(quote)` as `ledger::sha256_hex(fold_with_map(quote).0.as_bytes())` (whitespace runs collapsed to one space, trimmed). This matches `SourceIndex::verify_pointer` and Explore's deduplication rule (S3) so line-wrap differences do not change the hash.
2. **Legacy `tool.request` without `run_id` in `lines_sent`:** When `run_id` is `Some(rid)`, `context.supplied` events must match `rid`, and `tool.request` events carrying `run_id` must match `rid`; `tool.request` events *without* a `run_id` field (today's `Router`) are credited within the supplied `events` slice (still strictly partitioned by `caller`). Callers working on a multi-run session before `Router` adds `run_id` should pass the run's event slice.
3. **`search_paper_passages` boundary heuristic:** Unlike `desktop-server/src/investigation_cites.rs` (which drops `start_line` whenever `start_byte > 0` without disk bytes, even for passages cleanly starting on line 20), `lines_sent.rs` only advances `start_line` when `start_line == 1 && start_byte > 0`, and decrements `end_line` only when `excerpt` lacks a trailing `\n` and `excerpt.len() >= 900`.
4. **`EvidenceRole` enum:** Modeled `role` on `EvidenceQuotePayload` and `EvidenceAbstractPayload` as a typed enum covering all 7 Explore roles (`explore.md` 4.1) rather than an unchecked string. If a future mode needs another role, it extends `EvidenceRole` in one place.
5. **`subclaim_ids` format:** Validated each entry in `subclaim_ids` strictly as `sc-<positive digits>` (no zero, no leading zero), matching `paper_key::ID_PREFIXES`.
6. **Delegated worker call deduplication in F9:** Counted `delegate.turn` / `delegate.end` and `explore.branch` / `explore.end` per `call_id` via `max(observed_turns, end_turns)` so completed calls are never double-counted, mid-run calls count turns already emitted, and failed final turns recorded in `*.end.turns` are still charged to the budget.
7. **Provisional constants (Section 8 of `00-overview.md`):** Isolated into named constants with decision comments: `EVIDENCE_QUOTE_EVENT`, `EVIDENCE_ABSTRACT_EVENT`, `CONTEXT_SUPPLIED_EVENT` (decision 10); `QUOTE_MATCHED_WORD` (decision 3); `IDEA_EXCHANGE_EVENT` (decision 11); and `RUN_STATE_LABEL_*` (section 3.3).

### Assumptions
- Assumed `evidence.abstract` offsets `start` and `end` satisfy `start < end` (non-empty span) and `raw_blob` is a 64-character lowercase SHA-256 hex digest pointing to a ledger blob (consistent with `explore.md` 4.1 and `ledger::Ledger::put_blob`).
- Assumed `idea.exchange` records one event per provider call attempt (1 initial + up to 2 repairs, `explore.md` 4.2), so each `idea.exchange` event counts as 1 model call in `run_budget`.

---

## 4. Merge notes

### Existing file edited
- `crates/research-tools/src/lib.rs` (commit `eee499f`): added 3 `pub mod` lines (`lines_sent`, `quote_core`, `run_budget`). Trivial to merge with other briefs adding modules to `research-tools/src/lib.rs` (e.g. F3, V1, S2).

### Interfaces awaiting other briefs / later packages
- `crates/research-tools/src/lines_sent.rs` — `READ_MAP_NODE_TOOL` and `fn credit_read_map_node_result` (lines 19, 343, 456–461): **Investigate package V6** delivers `read_map_node` and its result schema, and will implement `credit_read_map_node_result` to credit the line-numbered file spans returned by `read_map_node`.
- `crates/desktop-server/src/investigation_cites.rs` — `collect_received_ranges`: **Investigate package V6** (or follow-up wiring) will switch question-turn citation crediting to `research_tools::lines_sent::lines_sent_with_ledger` (rule 6.17).
- `crates/research-tools/src/quote_core.rs` — `accept_quote`, `verify_quote`, `EvidenceQuotePayload`, `EvidenceAbstractPayload`: called by **Explore X8/X10** (reader/triage recorders), **Solve S8** (`step_record` Cited path + attach-source host route), and **F7** (`handoff.packet` quote re-matching).
- `crates/research-tools/src/run_budget.rs` — `RUN_ID_FIELD`, `count_model_calls`, `ModelCallBudget`, `RunState`:
  - **Future runner/router integration (F5 / V6 / X3 / X6 / S11 / S12):** add optional `"run_id"` (using `RUN_ID_FIELD`) to `model.request`, `verifier.request`, `strategist.request`, `delegate.turn`/`delegate.end`, `idea.exchange`, and `tool.request` so concurrent or multi-run sessions can be attributed without sequential slicing.
  - **F6 (`run_bar.js`):** renders `ModelCallBudget` (`cap`, `used`, `remaining`) and `RunState` (`label`, `action`) on screen.

---

## 5. Verification commands and exact results

| Command | Exit | Exact result |
|---|---:|---|
| `cargo test -p research-tools --test tool_def_golden` | 0 | 3 passed; 0 failed; 0 ignored |
| `cargo clippy -p research-tools --lib -- -D warnings` | 0 | 0 warnings, 0 errors |
| `cargo test -p research-tools --lib -- quote_core lines_sent run_budget` | 0 | 21 passed; 0 failed; 0 ignored |
| `cargo test --workspace --no-fail-fast` | 0 | 96 test binaries/doc-tests: **660 passed; 0 failed; 2 ignored** (the 2 ignored are pre-existing in `crates/agent/tests/eval_harness.rs`) |
| `python3 scripts/check_file_length.py --base origin/claude/project-thread-caszr2` | 0 | `No touched source file is over 600 lines.` |
| `cargo clippy --workspace --all-targets -- -D warnings` | 101 | Fails on pre-existing warnings only (see Section 6); 0 warnings in any file we added or edited |

---

## 6. Pre-existing failures / warnings on Linux (with proof)

1. **`crates/desktop-server/src/evidence.rs:744` (`clippy::manual_unwrap_or`)** — **Pre-existing.** Confirmed by running `cargo clippy --workspace --all-targets -- -D warnings` on base commit `49edc18dd66b85044046dacac186af3593a44de3` before any edits (`error: this pattern reimplements Option::unwrap_or` at `crates/desktop-server/src/evidence.rs:744:40`). Also documented in `RULES.md`.
2. **Additional pre-existing clippy warnings under Rust 1.100.0** (visible via `cargo clippy --workspace --all-targets -- -D warnings --cap-lints warn`; none in files touched by this brief):
   - `crates/research-tools/tests/sandbox_and_fs.rs:6` — `unused_imports`: `TEX_AUDIT` and `is_symlink_or_junction` (used only in `#[cfg(windows)]` tests).
   - `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` — `unused_imports`: `BufRead` and `BufReader` (used only in `#[cfg(windows)]` tests).
   - `crates/agent/tests/eval_harness.rs` — 2 pre-existing clippy lints (`match expression looks like matches! macro`, `this map_or can be simplified`).
   - `crates/agent` lib tests — 1 pre-existing clippy lint (`items after a test module`).
3. **Note on `paper_read::tests::try_call_err_aborts_all_and_fails_turn`:** Passed on this VM because commands ran as non-root user `antigravity`.

---

## 7. What could not be compiled or run on Linux

- All `#[cfg(windows)]` code paths and Windows-only integration tests:
  - `crates/worker/tests/appcontainer_sandbox.rs` (0 tests executed on Linux)
  - `crates/worker/tests/python_worker.rs` (0 tests executed on Linux)
  - `crates/cli/tests/contained_router.rs` (0 tests executed on Linux)
  - `crates/cli/tests/init_abort.rs` (0 tests executed on Linux)
  - `crates/desktop-app` (Windows WebView2 crate, excluded from default Linux workspace members / `#[cfg(windows)]`)
  - Windows DPAPI vault tests in `crates/chatgpt-auth`
- No assets under `crates/desktop-server/assets/` were touched, so `node --check` / `node --test` were not required.

---

## 8. Bugs noticed (not fixed, outside brief scope)

- **`crates/cli/tests/desktop_and_ext.rs:149-168`:** When `RE_SEARCH_PYTHON` is exported in the environment on Linux, `re-search run --route api` refuses `--python` on non-Windows and exits immediately before contacting `ScriptedMock`. Because `mock.finish()` (line 168) is called *before* checking `out.status` (line 169), the test blocks indefinitely waiting for 3 HTTP requests that will never arrive. Checking `out.status` (or dropping/signaling the mock on non-zero exit) before `mock.finish()` would fail fast instead of hanging.
- **`crates/research-tools/tests/sandbox_and_fs.rs:6` and `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6`:** Windows-only test imports are not gated with `#[cfg(windows)]`, causing `-D warnings` clippy failures on Linux.

---

## 9. Contradictions or gaps noticed in the design

- **Run attribution in `tool.request` and `model.request`:** Both F4 (`lines_sent(events, run_id)`) and F9 (`count_model_calls` of a run) assume provider requests and line-returning tool calls can be attributed to a run, yet neither `Router::call_inner` (`tool.request`) nor `runner.rs` (`model.request`) currently records `run_id`. Until emitters write `run_id`, both F4 and F9 must rely on sequential slicing (`RunScope::AfterSeq`/`SeqRange` or pre-sliced event slices) for legacy events, which breaks if two runs interleave in one session.
- **No other contradictions found** between `00-overview.md`, `explore.md` section 7, `solve.md` section 7.1, `investigate.md`, and `f4-f9-quote-budget.md`.
