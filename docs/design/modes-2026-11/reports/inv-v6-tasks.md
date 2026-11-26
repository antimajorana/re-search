# Report: `inv-v6-tasks` (Investigate V6 — reader, reading recorder, `investigation/2`, fold, and host task lifecycle)

- **Base commit:** `ae87d4ab4769b62d6b7d3a79abead67b8dc73f38` (`origin/gemini/work/fixed`)
- **Branch:** `gemini/modes/inv-v6-tasks`
- **Toolchain on Linux VM:** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `cargo 1.100.0 (3acc79c0c 2026-11-05)`, `Python 3.12.16`, `node v22.23.5`
- **Guard rails:**
  - `Cargo.lock` untouched (no new dependencies).
  - `crates/research-tools/tests/golden/tool_defs.txt` untouched; `all_research_tool_defs()`, `register_research_tools()`, and `[profile research/1]` unchanged.
  - `crates/research-tools/tests/golden/tool_profiles.txt` extended **only** for `investigation/1`'s scoped `may_gain` compatibility additions and `[profile investigation/2]`.
  - `crates/agent/src/resume.rs`, `crates/policy/`, `crates/ledger/`, `.github/`, and all other files under `docs/design/` untouched.
  - Zero `.unwrap()` or `.expect()` calls in non-test V6 code.
  - Zero `"verified"` / `"Verified"` strings in user-facing Investigate strings or task prompt copy (quotes are `"matched"`).
  - `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` exits 0 (`No touched source file is over 600 lines.`).

---

## 1. What was implemented

### 1.1 Shared Investigation markers & bidirectional gate (`research-tools`, commit `2f5cbe5`)

| File | Lines | Contents |
|---|---:|---|
| `crates/research-tools/src/investigation_markers.rs` | 275 | Single 14-marker table `INVESTIGATION_REQUEST_MARKERS` (10 legacy markers + `investigation.task_requested` + `investigation.task_finished` + `session.end` + `session.abort`), named constants for Layer-2 event kinds (`investigation.skeleton`, `investigation.reading`, `investigation.context`, `investigation.prior_work`, `investigation.note`, `investigation.override`), `InvestigationTaskKind` (`orient` cap 8, `open` cap 8, `context` cap 10, `prior_work` cap 12), `OldRecorderKind`, `NewRecorderKind`, `is_investigation_marker`, `active_investigation_slice`, `check_old_recorder_gate`, `check_new_recorder_gate` |
| `crates/research-tools/src/investigation_markers_tests.rs` | 154 | Exhaustive table tests covering all 4 old recorders × all 14 marker kinds and all 3 new recorders × all 14 marker kinds × all 4 task kinds × wrong `request_seq` |
| `crates/research-tools/src/investigation.rs` | 575 | Replaced hardcoded legacy marker match with `investigation_markers::is_investigation_marker`; preserved all 4 legacy recorder definitions, gates, and saved event shapes unchanged |
| `crates/research-tools/src/investigation_evidence.rs` | 388 | Replaced hardcoded legacy marker match in `find_active_marker` with `investigation_markers::is_investigation_marker` |

### 1.2 V6 tool definitions, node reader, reading recorder, additive fold, and F4 `lines_sent` (`research-tools`, commit `2f5cbe5`)

| File | Lines | Contents |
|---|---:|---|
| `crates/research-tools/src/map_defs.rs` | 396 | Tool ID constants (`READ_MAP_NODE`, `RECORD_MAP_READING`, reserved `RECORD_CONTEXT_CARDS`, reserved `RECORD_PRIOR_WORK_CHECK`), `INVESTIGATION_V2_MAY_GAIN`, frozen `ToolDef` builders (`read_map_node_def`, `record_map_reading_def` — both `EffectClass::ReadWorkspace`, `ApprovalScope::Once`), `register_map_tools`, and Layer-2 record structs (`ReadingRole`, `AuthorsClaimLabel`, `ModelEdgeRole`, `ModelEdgeBasis`, `VerifiedMapAnchor`, `HypothesisRecord`, `AuthorsClaimRecord`, `ReadingItemRecord`, `ModelEdgeRecord`, `IndexedFault`, `ReadingRejected`, `MapReadingEventPayload`) |
| `crates/research-tools/src/map_node_read.rs` | 471 | `load_pinned_skeleton` (resolves active task's `skeleton_seq` or latest skeleton for `source_id`, verifies ledger blob SHA-256) and `read_map_node` (lookup by tag, TeX `\label`, or printed reference e.g. `"Theorem 1.2"` / `"Eq. (3)"` with ambiguity and nearby-tag diagnostics; `SourceIndex` hash verification; selective `include`; strict line-boundary `max_chars` binary search; `files[]` metadata carrying exact delivered `{source_id, path, file_sha256, start_line, end_line}` for F4 `lines_sent`; attached proof, `uses` grouped by role, `used_by`, and `symbols`) |
| `crates/research-tools/src/map_node_read_tests.rs` | 369 | 6 unit tests: lookup by tag/label/printed ref, ambiguity & unknown node diagnostics, statement/proof/edges/symbols & selective `include`, strict `max_chars` line-boundary truncation & F4 `lines_sent` crediting of delivered lines only, pinned `skeleton_seq` during active task, tampered skeleton blob and tampered source file fail-closed |
| `crates/research-tools/src/map_reading.rs` | 363 | `record_map_reading` handler: recursive rejection of app-only fields (`excerpt_basis`, `cited_range`, `range_basis`, `excerpt_window`, `file_sha256`, `quote_sha256`, `excerpt`, `cues`, `node_fp`), rule 6.3 marker gate (`orient`/`open` + exact `request_seq`), pinned skeleton & `open` scope computation (item scope = requested tags + descendants + attached proofs; edge scope = item scope + direct `uses` prerequisites), F4 `lines_sent_with_ledger` over the active task slice, per-item/per-edge acceptance saving valid siblings while returning indexed faults + `true_text`, vacuous empty `done: true` refusal unless `limitations` is non-empty, multi-call `append_at_head` |
| `crates/research-tools/src/map_reading_validate.rs` | 365 | Per-item and per-edge validation: F4 `accept_quote_with_sent`, span containment (rule 6.5), `role` restrictions (`main_result` on theorem-like kinds only; `standard_tool` on theorem-like/definition/algorithm), `authors_claim` validation (non-empty `bib_key` present in skeleton `bib` + `quote_matches_node` / `differs_from_node` / `unclear`), `open` scope checks (`edge.from` in `edge_scope` and `edge.to` in `item_scope`), edge anchor rules (`proof` in target proof or proof node, `statement` in target statement, `argument` anywhere in pinned source, self-edge refusal), and rejection of V8 `new` model nodes (`unsupported_model_node`) |
| `crates/research-tools/src/map_reading_tests.rs` | 450 | 6 unit tests: mixed validity saving valid siblings with `true_text` + multi-call append, span containment / `role` / `authors_claim` rules, edge role containment & self-edge refusal, F4 `lines_sent` enforcement (wrong hash, wrong caller, `read_map_node` credit), `open` task scope / unknown tag / `new` node rejection / app-only fields / closed marker gate, plus regression test for `open` edge destination scope & vacuous empty `done: true` refusal |
| `crates/research-tools/src/map_fold.rs` | 395 | Pure additive fold `fold_map(&PaperSkeleton, &[Event], running)` (rule 6.12): chronological `FieldSlot<T>` history per node field (`gist`, `role`, `purpose`, `hypotheses`, `anchors`, `authors_claim`), model edge history keyed by `(from, to, role)` never overwriting Layer-1 skeleton edges, `node_fp` staleness detection, reader notes (`investigation.note`) and reader role pins (`investigation.override`), and task history fold using F9 `count_model_calls` + `investigate_task_run_state` |
| `crates/research-tools/src/map_fold_tests.rs` | 397 | 3 unit tests: running Orient twice preserves chronological history, untouched fields, reader notes, and pins; reader pin overrides model role & `node_fp` mismatch flags `stale`; task fold uses F9 `count_model_calls` and maps all run states |
| `crates/research-tools/src/lines_sent.rs` | 503 | Implemented `credit_read_map_node_result` (replacing F4's placeholder) to credit hash-pinned `files[]` blocks from `read_map_node` results (`start_line..=end_line`, adjusting for `line_truncated`) |
| `crates/research-tools/src/lines_sent_tests.rs` | 482 | Replaced placeholder test with full unit test verifying `read_map_node` credits exact delivered hash-pinned lines, respects `line_truncated`, rejects invalid hashes/ranges, and isolates callers |
| `crates/research-tools/src/lib.rs` | 310 | Exported `investigation_markers`, `map_defs`, `map_fold`, `map_node_read`, `map_reading` (left `all_research_tool_defs()` and `register_research_tools()` completely untouched) |

### 1.3 `investigation/2` tool profile, compatibility `may_gain`, and goldens (`research-tools`, `agent`, `mcp-server`, `desktop-server`, commit `46a8bc9`)

| File | Lines | Contents |
|---|---:|---|
| `crates/research-tools/src/tool_profile.rs` | 370 | Added `investigation_v2()` (24 tools in sorted id order: all 22 `research/1` tools + `read_map_node` + `record_map_reading`; `v0.4-research` policy; `INVESTIGATION_V1_MAY_GAIN` = 7 legacy + 4 V2 `ReadWorkspace` IDs); extended `investigation/1.may_gain` (never `research/1`); removed `investigation/2` from `RESERVED_PROFILE_NAMES`; updated `profile_for_session_events` so new Investigate sessions without `session.start` resolve to `investigation/2` while existing sessions with `session.start` lacking `tool_profile` remain on `investigation/1`; wired `register_map_tools` in `register_profile` exclusively for `investigation/2` |
| `crates/research-tools/src/tool_profile_tests.rs` | 223 | Added assertions for `investigation/2` lookup, 24-tool sorted set, `investigation/1` vs `research/1` `may_gain` split, and `profile_for_session_events` (new Investigate -> `investigation/2`, legacy pre-F1 Investigate -> `investigation/1`) |
| `crates/research-tools/tests/tool_profile_golden.rs` | 155 | Extended cross-check to cover `investigation/2` (asserting both V6 tools are `EffectClass::ReadWorkspace` while all 22 shared tools match `tool_defs.txt` hashes verbatim) and updated `RESERVED_PROFILE_NAMES` check |
| `crates/research-tools/tests/golden/tool_profiles.txt` | 127 | Extended **only** with `investigation/1` `may_gain` entries 7–10 and `[profile investigation/2]` (`read_map_node` hash `e23958bd...`, `record_map_reading` hash `3349b66c...`) |
| `crates/research-tools/tests/investigation_v6_markers_and_tools.rs` | 441 | 3 integration tests: router-level bidirectional marker gate closing legacy recorders during `task_requested` and `record_map_reading` during legacy markers; end-to-end `read_map_node` -> `record_map_reading` -> re-run -> `fold_map` preserving history & reader notes; profile compatibility proving `investigation/1` allows `ReadWorkspace` V6 gain while `research/1` rejects it |
| `crates/agent/src/resume_profile_tests.rs` | 455 | Updated `investigation/2` from reserved to built; kept untolerated-tool drift test with `unrelated_read_tool` and added explicit assertions that `research/1` rejects gaining `read_map_node` while `investigation/1` accepts it only with `EffectClass::ReadWorkspace` |
| `crates/desktop-server/tests/mode_markers_r2.rs` | 428 | Added `read_map_node` (`&[]`) and `record_map_reading` (`&["investigation.reading"]`) to the exhaustive R2 mode-marker audit inventory |
| `crates/mcp-server/src/main.rs` | 331 | When `CORE_TOOL_PROFILE` is unset and `session.start` has no explicit `tool_profile`, resolves via `legacy_profile_name_for_events` so untouched `cli_turn.rs` (627 lines) does not suffer a tool-list mismatch on Claude CLI / Antigravity |

### 1.4 Host task context, task lifecycle, mapview fold, F4 cites, and HTTP route (`desktop-server`, commits `bf665a0` & `4520ea6`)

| File | Lines | Contents |
|---|---:|---|
| `crates/desktop-server/src/investigation_task_context.rs` | 403 | `build_task_context` for `orient` (abstract, intro, conclusion, theorem/definition headings), `open` (requested tags, attached proofs, direct `uses` prerequisites), and `context` (requested tags + prerequisites + citations), wrapped in `<quoted_source_material ...>` with explicit DATA ONLY framing (rule 6.19), strict line-boundary `max_chars` truncation, exact hash-pinned `SuppliedRange` collection, and `record_context_supplied` |
| `crates/desktop-server/src/investigation_task_context_tests.rs` | 394 | 6 unit tests covering Orient/Open/Context assembly, tight `max_chars` boundary without crediting omitted lines, `record_context_supplied` caller/hash isolation, task route validation & all rule 6.15 finish states, plus regression tests for all Carbon audit guards (C1 `investigation/1` refusal, C2 CLI route refusal, C3 bounded F4 cites with hash check, M1 spawn-error closure, M2 vacuous empty `done: true` refusal, M5 mapview excerpt stripping) |
| `crates/desktop-server/src/investigation_tasks.rs` | 498 | `handle` for `POST /api/sessions/{id}/investigation/tasks` (rule 6.1 pinned skeleton required, `investigation/2` profile required, tag validation with nearby suggestions, cap clamping, rejects deferred `prior_work`), `prepare_task`, `fail_task_on_spawn_error`, `record_supplied_for_caller`, `finish_task` (F9 `count_model_calls` + rule 6.15 `done`/`partial`/`stopped`/`interrupted`/`failed` preserving saved records) |
| `crates/desktop-server/src/turn_runner.rs` | 538 | Smallest host lifecycle hook: `spawn_task_turn_thread`, `strip_internal_task_fields` on untrusted `spawn_turn_thread`, pre-check refusing CLI-route v2 tasks before `wants_cli_route` can mutate metadata, `prepare_task`, `fail_task_on_spawn_error`, `record_supplied_for_caller` with exact `agent:{provider}:{session}` caller string, disabling non-Investigate workers during task turns, and `finish_with_cancel` |
| `crates/desktop-server/src/investigation_finish.rs` | 112 | Added `finish_with_cancel` dispatching v6 task turns to `investigation_tasks::finish_task` while preserving legacy finish paths and `#[cfg(test)] finish` wrapper |
| `crates/desktop-server/src/investigation.rs` | 598 | Gated `pub(crate) use crate::investigation_finish::finish;` with `#[cfg(test)]` |
| `crates/desktop-server/src/investigation_mapview.rs` | 459 | Added `build_mapview_json_with_fold` projecting folded `node.reading`, `stale`, model edges (`model_edge: true`), and `tasks` into `mapview`, plus `strip_anchor_excerpts` removing 3.2 KiB raw source `excerpt` fields while keeping matched `quote` chips |
| `crates/desktop-server/src/investigation_skeleton.rs` | 486 | Wired `GET /api/sessions/{id}/investigation/mapview` to fold with `map_fold::fold_map` and call `build_mapview_json_with_fold` |
| `crates/desktop-server/src/investigation_cites.rs` | 534 | Added `collect_received_ranges_with_sha` and `answer_cites_with_bytes_and_sha` crediting `context.supplied` and `read_map_node` through F4 `lines_sent_with_ledger`, bounded strictly to `(request_seq, next_investigation_marker)` with `run_id` and pinned `file_sha256` checks (rule 6.17) |
| `crates/desktop-server/src/investigation_view.rs` | 391 | Added `investigation.task_requested` and `investigation.task_finished` to `mark_last_running` and `branch_history` closing marker checks |
| `crates/desktop-server/src/paper_reader.rs` | 589 | Wired `active_marker` to shared `investigation_markers::is_investigation_marker` |
| `crates/desktop-server/src/lib.rs` | 222 | Exported `investigation_task_context` and `investigation_tasks` |
| `crates/desktop-server/src/api_dispatch.rs` | 372 | Registered `("POST", ["api", "sessions", id, "investigation", "tasks"])` behind `AuthGuard::authorize_api` (commit `4520ea6`) |
| `crates/desktop-server/tests/investigation_v6_tasks.rs` | 565 | 2 end-to-end HTTP integration tests: auth (401 missing/wrong token, 403 cross-site origin), rule 6.1 skeleton-first refusal, scripted mock Anthropic Orient (`done`, `context.supplied`, `mapview` folded readings & tasks), Open hitting cap (`partial`, saved records preserved), `/messages` `_investigation_task_spec` spoofing guard, and question-turn `read_map_node` citation crediting |

---

## 2. Independent adversarial Carbon audit findings (all fixed)

After initial implementation, an independent Carbon (`pro`) audit reviewed every diff and surfaced the following findings, all of which were fixed and locked with regression tests before committing:

- **C1 (Critical): `investigation/1` sessions starting v2 tasks.** Because `resume.rs` is frozen and `resume_profile.rs:154-160` requires `active_router_profile == resolved.profile.name`, an existing `investigation/1` session cannot register `read_map_node`/`record_map_reading`. Fixed by checking `profile_for_session(&events)` in `investigation_tasks::handle` and `prepare_task` and failing closed with HTTP `400` before appending `investigation.task_requested`.
- **C2 (Critical): CLI route (`claude-cli` / `antigravity`) tool mismatch.** `cli_turn.rs` (627 lines, untouchable under `RULES.md`) hardcodes `core_tool_set` (`research/1`, 23 tools) without setting `CORE_TOOL_PROFILE`. Fixed by (a) falling back to `legacy_profile_name_for_events` in `mcp-server/src/main.rs` when `CORE_TOOL_PROFILE` and explicit `session.start.tool_profile` are absent, and (b) rejecting CLI-route v2 task requests with HTTP `400` in `turn_runner.rs` before `wants_cli_route` can provisionally bind metadata or open `investigation.task_requested`.
- **C3 (Critical): Unbounded F4 `lines_sent` window in `investigation_cites.rs`.** Fixed by bounding `f4_events` strictly to `(request_seq, next_investigation_marker)` and passing `run_id = Some(&request_seq.to_string())` so later turns' `context.supplied` / `read_map_node` results can never leak backward.
- **M1 (Major): Thread spawn failure leaving dangling `investigation.task_requested`.** Fixed by adding `investigation_tasks::fail_task_on_spawn_error` and calling it from `turn_runner.rs` if `std::thread::Builder::spawn` fails.
- **M2 (Major): Vacuous empty `done: true` earning `Done`.** Fixed in `map_reading.rs` (require non-empty `limitations` for an item/edge-free `done: true` append) and `investigation_tasks.rs` (`has_saved` requires `saved_items > 0 || saved_edges > 0`; `has_documented_done` requires `any_done && (has_saved || saved_limitations > 0)`).
- **M3 (Major): Over-broad `open` edge destination scope.** Tightened `map_reading_validate.rs` so `open` edges require `edge_scope.contains(edge.from) && item_scope.contains(edge.to)` (previously allowed `edge_scope` on both endpoints).
- **M4 (Major): Legacy `investigation_view.rs` missing V6 task markers.** Added `investigation.task_requested` / `investigation.task_finished` to `mark_last_running` and `branch_history`.
- **M5 (Major): 3.2 KiB anchor `excerpt` bloat in `mapview`.** Added `strip_anchor_excerpts` in `investigation_mapview.rs` so `mapview` omits raw `excerpt` fields while retaining matched `quote` chips.
- **Minor #1 & #2:** Removed two `.unwrap()` calls in `map_reading_validate.rs`, replaced `"verified"` with `"matched"` in `anchor_report` and task prompt copy, unconditionally tracked `item_anchor_faults` in `pending_dropped`, and fixed a `clippy::match_like_matches_macro` lint in `investigation_markers_tests.rs`.

---

## 3. Deviations, decisions taken alone, and assumptions

### Deviations from the design
1. **CLI-route v2 tasks temporarily refused (HTTP 400) until `cli_turn.rs` is split below 600 lines:** `crates/desktop-server/src/cli_turn.rs` is 627 lines on base `ae87d4ab` and cannot be edited under `RULES.md`. Rather than allowing a silent `system/init` tool-list crash on Claude CLI / Antigravity, v2 tasks fail closed with HTTP 400 on CLI routes while API and ChatGPT routes run end-to-end. Exact 4-line integration patch is in Section 4.
2. **Older `investigation/1` sessions cannot run v2 tasks directly:** `crates/agent/src/resume.rs` cannot be edited in this brief, and `resume_profile.rs:154-160` enforces exact profile-name equality (`active_router_profile == resolved.profile.name`). Although `investigation/1.may_gain` includes `read_map_node` and `record_map_reading` per F1/V6 compatibility rules, resuming an `investigation/1` session under an `investigation/2` router is currently rejected by `resume_profile.rs`. `investigation_tasks` therefore rejects `investigation/1` sessions with a clear HTTP 400 directing the user to open a new Investigate session (`investigation/2`).
3. **`record_map_reading` and `read_map_node` are `ReadWorkspace` / `ApprovalScope::Once`:** Matches `investigate.md` §5.1 and `00-overview.md` §2.2; neither mutates user workspace files.
4. **Deferred/out-of-scope items intentionally left to their owning briefs:** Orient presentation & authors' labels (V7), model-created `new` nodes & Open presentation (V8 — `new` currently returns structured fault code `unsupported_model_node`), legacy view (V9), `record_context_cards` (V12), and `record_prior_work_check` / `prior_work` task (deferred V14 — HTTP 400 on `POST .../tasks`).

### Decisions taken alone
1. **Exact caller string for `context.supplied`:** Recorded `format!("agent:{}:{}", transport.info().provider, session_id)` on the InCore route (matching `crates/agent/src/runner.rs:246`) and `format!("{}:{}", kind.ledger_route(), session_id)` on the CLI route (matching `crates/mcp-server/src/main.rs:268`) so `record_map_reading`'s `ctx.caller` matches `lines_sent_with_ledger` identically without weakening caller isolation.
2. **Named constants for accepted overview §8 decisions:** Event kinds (`investigation.task_requested`, `investigation.task_finished`, `investigation.skeleton`, `investigation.reading`, `investigation.context`, `investigation.prior_work`, `investigation.note`, `investigation.override`), tool IDs (`read_map_node`, `record_map_reading`, `record_context_cards`, `record_prior_work_check`), wrapper tag (`quoted_source_material`), and mode/badge labels are isolated in named constants with decision comments in `investigation_markers.rs`, `map_defs.rs`, `investigation_task_context.rs`, and `investigation_mapview.rs`.
3. **Vacuous `done: true` rule:** Calling `record_map_reading` with `items: []`, `edges: []`, `limitations: []`, `done: true` returns `status: "no_valid_records"` and does not append an empty ledger event; calling with non-empty `limitations` and `done: true` appends and allows the task to finish `done`.
4. **HTML/PDF skeletons:** `read_map_node` and `build_task_context` currently load source files via `load_ingested_tex_source` / `SourceIndex`. Because V1/V3 only produce TeX skeletons on this base, this handles all skeletons produced today and fails closed on hash mismatch or missing source files.

### Assumptions
- Assumed `investigation.note` (`{ tag, text }` or `{ tag, note }`) and `investigation.override` (`{ tag, role }`) event schemas for `map_fold.rs` match `investigate.md` §4.4; if V7/V8 choose slightly different payload keys, `map_fold.rs:212-260` is the single place to update.
- Assumed default `max_chars` budgets of `12_000` (clamped `500..=24_000`) for `read_map_node` and `28_000` for `build_task_context`.

---

## 4. Merge notes

### Existing files edited (each kept small and local)
- `crates/research-tools/src/lib.rs` (`2f5cbe5`): +5 `pub mod` lines (`investigation_markers`, `map_defs`, `map_fold`, `map_node_read`, `map_reading`).
- `crates/research-tools/src/investigation.rs` & `investigation_evidence.rs` (`2f5cbe5`), `crates/desktop-server/src/paper_reader.rs` (`bf665a0`): replaced duplicated 12-marker `matches!` arms with `investigation_markers::is_investigation_marker`.
- `crates/research-tools/src/lines_sent.rs` & `lines_sent_tests.rs` (`2f5cbe5`): populated F4's `credit_read_map_node_result` placeholder.
- `crates/research-tools/src/tool_profile.rs`, `tool_profile_tests.rs`, `tests/tool_profile_golden.rs`, `tests/golden/tool_profiles.txt` (`46a8bc9`): added `investigation/2` and `investigation/1.may_gain` entries; trivial to merge with Explore/Solve briefs adding `explore/1` or `solve/1`.
- `crates/agent/src/resume_profile_tests.rs` & `crates/desktop-server/tests/mode_markers_r2.rs` (`46a8bc9`): updated for built `investigation/2` and the two V6 tools.
- `crates/mcp-server/src/main.rs` (`46a8bc9`): legacy fallback when `CORE_TOOL_PROFILE` and explicit `session.start.tool_profile` are absent.
- `crates/desktop-server/src/lib.rs`, `investigation.rs`, `investigation_finish.rs`, `investigation_mapview.rs`, `investigation_skeleton.rs`, `investigation_cites.rs`, `investigation_view.rs`, `turn_runner.rs` (`bf665a0`): smallest lifecycle, fold, and F4 citation hooks.
- `crates/desktop-server/src/api_dispatch.rs` (`4520ea6`): 3-line route registration.

### Exact desktop route registration lines (`crates/desktop-server/src/api_dispatch.rs:285-287`)
If the mode shell (`f5-f7-mode-shell`) refactors `api_dispatch.rs`, place this exact arm **before** the generic `("POST", ["api", "sessions", id, "investigation", operation])` catch-all:

```rust
        ("POST", ["api", "sessions", id, "investigation", "tasks"]) => {
            write_api_res(&mut stream, crate::investigation_tasks::handle(state, id, &req.body));
        }
```

### Enabling CLI routes (`claude-cli` / `antigravity`) for v2 tasks
Once `crates/desktop-server/src/cli_turn.rs` (currently 627 lines) is split below 600 lines:
1. In `crates/desktop-server/src/cli_turn.rs` around lines 453–456, replace `let tools = core_tool_set(Some(Arc::clone(&research)));` with:
   ```rust
   let profile = crate::session_mode::profile_for_session(&existing)?;
   core.env.insert("CORE_TOOL_PROFILE".into(), profile.name.into());
   let tools = cli_route::core_tools::core_tool_set_for_profile(Arc::clone(&research), profile.name)?;
   ```
   and include `"tool_profile": profile.name` in `start_fields` (line 456).
2. Remove the temporary `allow_task_spec` CLI-route refusal in `crates/desktop-server/src/turn_runner.rs:67-98`. (`turn_runner.rs:148-156` already calls `record_supplied_for_caller` with the exact `{ledger_route}:{session}` caller.)

### Upgrading older `investigation/1` sessions to `investigation/2`
`investigation/1.may_gain` already whitelists `read_map_node` and `record_map_reading` (`ReadWorkspace`). When a future brief updates `crates/agent/src/resume_profile.rs:154-160` to permit a declared profile upgrade (`investigation/1` -> `investigation/2`), relax `require_map_tools_profile` in `crates/desktop-server/src/investigation_tasks.rs:321` accordingly.

### Handoff interfaces for downstream briefs
- **V7 (Orient presentation & authors' labels):** Reads `mapview.nodes[].reading` (`gist`, `role`, `purpose`, `anchors`, `authors_claim` + chronological `history`) and `mapview.tasks[]` from `GET /api/sessions/{id}/investigation/mapview`; posts `{"task":"orient"}` to `POST /api/sessions/{id}/investigation/tasks`.
- **V8 (Open presentation & model-created nodes):** Posts `{"task":"open","tags":[...]}` to `POST /api/sessions/{id}/investigation/tasks`; replaces the `unsupported_model_node` branch in `crates/research-tools/src/map_reading_validate.rs:197-205` with model-node validation.
- **V12 (`record_context_cards`):** Uses `investigation_markers::NewRecorderKind::ContextCards` (already wired to `InvestigationTaskKind::Context` in the shared marker gate) and reserved ID `map_defs::RECORD_CONTEXT_CARDS` (already in `investigation/1` and `investigation/2` `may_gain`).

---

## 5. Verification commands and exact results

| Command | Exit | Exact result |
|---|---:|---|
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | 0 | `No touched source file is over 600 lines.` |
| `cargo test -p research-tools --test tool_def_golden` | 0 | **3 passed; 0 failed; 0 ignored** |
| `cargo test -p research-tools --test tool_profile_golden` | 0 | **3 passed; 0 failed; 0 ignored** |
| `cargo test -p research-tools --test investigation_v6_markers_and_tools` | 0 | **3 passed; 0 failed; 0 ignored** |
| `cargo test -p research-tools --test investigation --test investigation_pointers --test investigation_scope --test investigation_evidence` | 0 | **37 passed; 0 failed; 0 ignored** (9 + 13 + 1 + 14) |
| `cargo test -p research-tools --lib map_` | 0 | **20 passed; 0 failed; 0 ignored** (includes `map_node_read`, `map_reading`, `map_fold`, `lines_sent::read_map_node`) |
| `cargo test -p agent --lib resume` | 0 | **12 passed; 0 failed; 0 ignored** (includes all 6 `resume::drift_tests` + 4 `resume_profile::tests` + 2 `paper_read` resume tests) |
| `cargo test -p agent --test resume_loop` | 0 | **6 passed; 0 failed; 0 ignored** |
| `cargo test -p desktop-server --lib investigation_task_context` | 0 | **6 passed; 0 failed; 0 ignored** |
| `cargo test -p desktop-server --lib investigation_mapview` | 0 | **3 passed; 0 failed; 0 ignored** |
| `cargo test -p desktop-server --test eval_set_r --test mode_markers_r2 --test investigation_v3_routes --test investigation_v6_tasks --test investigation_flow --test audit_honesty_cites --test audit_consistency_investigation` | 0 | **25 passed; 0 failed; 0 ignored** (`eval_set_r`: 5, `mode_markers_r2`: 3, `investigation_v3_routes`: 1, `investigation_v6_tasks`: 2, `investigation_flow`: 4, `audit_honesty_cites`: 4, `audit_consistency_investigation`: 6) |
| `cargo test --workspace --no-fail-fast` | 0 | 113 test binaries/doc-tests: **908 passed; 0 failed; 2 ignored** (the 2 ignored are pre-existing live-tier tests in `crates/agent/tests/eval_harness.rs`) |
| `node --test crates/desktop-server/tests/*.test.mjs` | 0 | **163 passed; 0 failed; 0 skipped** |
| `cargo clippy -p research-tools --lib --test tool_def_golden --test tool_profile_golden --test investigation_v6_markers_and_tools --test investigation --test investigation_pointers --test investigation_scope --test investigation_evidence -- -D warnings` | 0 | 0 warnings, 0 errors |
| `cargo clippy -p mcp-server --all-targets -- -D warnings` | 0 | 0 warnings, 0 errors |
| `cargo clippy -p agent --lib --test resume_loop -- -D warnings` | 0 | 0 warnings, 0 errors |
| `cargo clippy -p desktop-server --lib --test eval_set_r --test mode_markers_r2 --test investigation_v3_routes --test investigation_v6_tasks -- -D warnings -A clippy::manual_unwrap_or` | 0 | 0 warnings, 0 errors (only allowing pre-existing `evidence.rs:744`) |
| `cargo clippy --workspace --all-targets -- -D warnings` | 101 | Fails solely on pre-existing untouched warnings on Linux/Rust 1.100.0 (Section 6); 0 warnings in any file touched or created in this brief |

---

## 6. Pre-existing warnings/failures on Linux (with proof)

Confirmed on untouched files (already documented in `RULES.md` and `reports/f4-f9-quote-budget.md` on base commit `ae87d4ab`):
1. `crates/desktop-server/src/evidence.rs:744` — `clippy::manual_unwrap_or` (file is 1,057 lines, untouched).
2. `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` — `unused_imports`: `BufRead`, `BufReader` (used only in `#[cfg(windows)]` tests).
3. `crates/research-tools/tests/sandbox_and_fs.rs:6,9` — `unused_imports` on non-Windows.
4. `crates/agent/tests/eval_support/sheet.rs:102` (`clippy::unnecessary_map_or`) and `crates/agent/tests/eval_support/manifest.rs:289` (`clippy::match_like_matches_macro`).
5. `paper_read::tests::try_call_err_aborts_all_and_fails_turn` passed on this VM because tests ran as non-root user `antigravity`.

---

## 7. What could not be compiled or run on Linux

- All `#[cfg(windows)]` code paths and Windows-only integration tests:
  - `crates/desktop-app` (Windows WebView2 host)
  - `crates/worker/tests/appcontainer_sandbox.rs` and `crates/worker/tests/python_worker.rs`
  - `crates/cli/tests/contained_router.rs` and `crates/cli/tests/init_abort.rs`
  - Windows DPAPI vault tests in `crates/chatgpt-auth`
- Live vendor CLI or real-network model runs (forbidden by `RULES.md`; all provider turns tested with in-process `ScriptedProvider` loopback mocks).

---

## 8. Bugs noticed (not fixed, outside brief scope)

- `crates/desktop-server/src/cli_turn.rs:454` (627 lines, untouchable under `RULES.md`): hardcodes `core_tool_set` instead of `core_tool_set_for_profile` and does not set `CORE_TOOL_PROFILE` in `core_env`, which prevents subscription-CLI routes from serving `investigation/2` (and future `explore/1`, `solve/1`) until split below 600 lines.
- `crates/desktop-server/src/cli_turn.rs:145-152`: `wants_cli_route` writes provisional `m.route` to `SessionMeta` before turn validation finishes; `turn_runner.rs` now guards v2 tasks before calling `wants_cli_route`.
- `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` and `crates/research-tools/tests/sandbox_and_fs.rs:6,9`: Windows-only imports lack `#[cfg(windows)]`, breaking `cargo clippy --workspace --all-targets -- -D warnings` on Linux.

---

## 9. Contradictions or gaps noticed in the design

- **`investigation/1` -> `investigation/2` compatibility vs strict `resume_profile.rs` profile equality:** `inv-v6-tasks.md` states *"Use only F1's profile data to support older `investigation/1` sessions ... Do not edit `resume.rs`."* While `investigation/1.may_gain` now authorizes gaining `read_map_node` and `record_map_reading` (`ReadWorkspace`), `crates/agent/src/resume_profile.rs:154-160` still fails closed whenever `active_router_profile != resolved.profile.name`. Until a later brief allows controlled profile upgrades in `resume_profile.rs`, older `investigation/1` sessions continue resuming cleanly on `investigation/1` (proven by `eval_set_r`), while v2 tasks require a new `investigation/2` session.
- **Sequential `run_id` attribution gap in F4/F9:** As noted in `reports/f4-f9-quote-budget.md`, `Router::call_inner` (`tool.request`) and `runner.rs` (`model.request`) do not yet emit `run_id`, so V6 bounds F4 and F9 queries by slicing from `request_seq` to the next Investigation marker (`RunScope::AfterSeq(request_seq)` / active task slice). This is exact today because `active_turns` enforces at most one running turn per session.
