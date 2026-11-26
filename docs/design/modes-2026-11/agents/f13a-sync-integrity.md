# Brief f13a-sync-integrity — review and harden the F13a checkpoint

Start from the current `gemini/work/fixed`, then merge only the F13a checkpoint
`eea6e2cc4ef11d8e7e95e038b3f6c444d50865d0` from
`origin/gemini/modes/lib-f13a-f16a-f15`. The original agent is continuing F16a
and F15. Leave those packages and its branch alone.

- Trace the library index writes in `source_sync.rs` and `library_db_ext.rs`.
  Make a sync update atomic across paper/source rows, aliases, fulltext and the
  last-sync marker. Verify how two syncs of a source can interleave and keep
  old scan results from overwriting a more recent update. Keep expensive file
  reads outside the write transaction. Surface errors that otherwise leave
  a partial result with a synced marker.
- Unregistering one source must preserve a fulltext path still indexed by
  another registered source. Check overlapping folder and vault entries and
  preserve fulltext from unrelated origins. Keep the operation confined to
  the rebuildable library database and handle retries safely.
- Add focused synthetic tests for overlap and interrupted or concurrent sync.
  Preserve existing dry-run, path containment, moved-file, duplicate and
  no-writes-to-source tests. Run the touched-crate checks. No dependencies,
  new tools, UI or routing changes; the integrator will register the dispatcher
  after F5. Never use real user folders, notes or paper files.
- In the report, identify the exact F13a code and test checkpoints. Keep the
  attribution to Kandinsky `353368b` and note that the source files have mixed
  contributor history. The integrator has already checked the source files.

Write `reports/f13a-sync-integrity.md` and push only to
`gemini/modes/f13a-sync-integrity` using `RULES.md`.
