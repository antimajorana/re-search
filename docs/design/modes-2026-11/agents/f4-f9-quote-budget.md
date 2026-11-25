# Brief f4-f9-quote-budget: quote core (F4) and run budget (F9)

Two small independent packages. Use one subagent for each if you can; they share no file.

## F4 quote core

Design: `00-overview.md` section 2.5 and row F4; section 1, first row (why the existing path
cannot be reused); 2.10 for what a matched quote means on screen; `explore.md` section 7 and
`solve.md` 7.1 for the callers.

Deliver `crates/research-tools/src/quote_core.rs` and `lines_sent.rs`:

1. `QuoteRef { source_id, path, start_line, end_line, file_sha256, quote_sha256, evidence_seq:
   Option<u64> }` and `verify_quote(session_events, source_id, path, start_line, end_line, quote)
   -> Result<QuoteRef + excerpt, Fault + true_text>`. It wraps the existing
   `SourceIndex::verify_pointer` (read `investigation_anchor.rs`, `investigation_evidence.rs`,
   `paper_lines.rs`): same relocation radius, same fault text. It needs only a `source.ingested`
   receipt in the session and **no Investigation marker**. Do not change the behaviour of
   `note_evidence` or of any existing Investigation path; the existing tests of those must pass
   untouched.
2. `lines_sent(events, run_id) -> ranges`, computed from one event kind `context.supplied {run_id,
   caller, ranges: [{source_id, path, file_sha256, start_line, end_line}]}` plus the tool results
   that return lines (`read_paper_lines`, `search_paper_passages`, `fs_read`; `read_map_node` does
   not exist yet, leave a named place for it). Read how those results are recorded in the ledger
   today before you decide how to credit them, and say in the report exactly which ledger fields
   you read.
3. The acceptance rule as one function: a quote is accepted only if it verifies **and** its lines
   were sent to the caller that cites it. Return which of the two failed.
4. Payload types and validating constructors for the stored-quote events `evidence.quote` (id
   `xe-<seq>`) and `evidence.abstract` (id `xa-<seq>`), with the optional mode fields (`role`,
   `subclaim_ids`, `gist`, `paper_key`). Types and validation only: no tool, no recorder wired into
   a router, no route. The event-kind strings are provisional (owner decision 10): one constant each.
5. Field names everywhere `start_line`, `end_line`. A helper that clips a quote for display or
   export to at most 300 characters on a character boundary.
6. Tests: a quote that matches; one that relocates within the radius; one outside it; wrong file
   hash; a session with a receipt and no Investigation marker verifies (the case in section 1);
   lines sent to caller A do not let caller B cite them; overlapping and adjacent ranges; the 300
   character clip on multi-byte text.

## F9 run budget and run state

Design: `00-overview.md` row F9, section 3.1 row 11, section 3.3, section 3.2 row 22.

Deliver `crates/research-tools/src/run_budget.rs`:

1. One function that counts the model calls of a run from ledger events. "Model call" = one
   request to the provider. Read how `model.request`, `model.response` and `model.error` are
   written today (`crates/agent/src/runner.rs`, `autonomous.rs`, `delegate.rs`, `paper_read.rs`)
   and how a run, a caller or a task can be told apart in them. If today's events cannot attribute
   a model call to a run, say so in the report and define the smallest field that would, without
   adding it.
2. A budget type: cap, used, remaining, "N of at most M model calls" as data (the sentence itself
   belongs to the UI).
3. The run-state vocabulary of 3.3 as one enum with the seven on-screen states, and the mapping
   from each mode's stored states listed in that table (Investigate task, Explore stage, Solve
   run) as total functions with tests that cover every row. "Partial" is a result, not a state.
4. The `independence` function is not in this brief.

Not in this brief: `run_bar.js` and any UI (F6), pause handling in the runner, any tool.
