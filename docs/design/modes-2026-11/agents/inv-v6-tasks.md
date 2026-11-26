# Brief inv-v6-tasks — Investigate reader and task lifecycle (V6)

Start from `gemini/work/fixed` after V3, F1, F4 and F9 are merged. Read `RULES.md`,
`investigate.md` 4.3–4.4, 5.1–5.3, rules 6.3–6.5, 6.10, 6.12, 6.15, 6.17–6.19,
and sections 7–9. Read `00-overview.md` 2.2, 2.5, 3.3, and the F1, F4/F9 and V3
reports. Own the V6 row. When useful, delegate the node reader, record validation
and fold, and host task lifecycle to separate Neon subagents on disjoint files.

- Own the V6 marker, reader, recorder, profile definitions, fold, task and context
  modules in the package row. Share the marker table across old and new recorders.
  A new recorder writes only for the matching active task and request sequence.
  Preserve the four old recorder definitions, their legacy gates and their saved
  events. Cover both directions of the old/new marker gate in table tests.
- A request pins its source and skeleton. Validate tags and relation scope against
  that snapshot. Reuse F4's verification and caller-scoped lines-sent logic,
  including `read_map_node` results and `context.supplied` ranges with file hashes.
  Require matched, in-span anchors and enforce the edge rules in code. Return
  indexed faults and save valid siblings in the same call. Keep history on re-run.
- Add only `read_map_node` and `record_map_reading` (`ReadWorkspace`) through
  `investigation/2`. Keep `research/1` and existing tool hashes and order intact.
  Use only F1's profile data to support older `investigation/1` sessions. The
  legacy research registry stays frozen. Extend the profile golden only for the
  approved profile and compatibility additions. Prove the old ledger fixtures
  and both drift suites pass untouched. Do not edit `resume.rs`.
- Own the host task module and the smallest lifecycle hook in `turn_runner.rs`.
  The task host writes the requested and finished markers and logs the exact
  source text it supplied, wrapped as quoted source material. Use F9 to count
  model calls. Preserve saved records when a run pauses, stops or hits its cap.
  Use the desktop dispatch seam available on the base; give the integrator exact
  registration lines if the mode shell is still in progress. Restrict the tested
  routes to session authorization and require the pinned skeleton before starting.

Leave Orient's presentation and authors' labels to V7, model-created nodes and
Open's presentation to V8, the legacy view to V9, and context cards to V12. V14
and version tracking are deferred. Run the old Investigation suites, the
profile goldens, `eval_set_r`, `resume_loop` and `resume_drift_tests`, and include
the results and integration instructions in your report. Push to
`gemini/modes/inv-v6-tasks` as directed in `RULES.md`.
