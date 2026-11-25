# Audit Report — Lens: `consistency`

- **Brief**: `audit-consistency`
- **Audited commit (`origin/gemini/work/merged`)**: `351b1287c9a54a700b0e33111b3891f195afd6d1`
- **Baseline (`origin/main`)**: `7c36a0e2547e8e10edcbfb2260649738ea5bad28` (92 files changed)
- **Governance (`origin/gemini/runner`)**: `ca495a361116f52150f5484e08d63b6bfc7fd470`
- **Proving test files added** (production code and existing tests untouched):
  - `crates/desktop-server/tests/audit_consistency_evidence_and_tools.rs` (4 failing tests)
  - `crates/desktop-server/tests/audit_consistency_investigation.rs` (4 failing tests)
  - `crates/desktop-server/tests/audit_consistency_ui.test.mjs` (5 failing tests)

## Summary of Findings

| ID | Severity | Summary | Proof |
|---|---|---|---|
| C-01 | `major` | `read_paper_lines` omitted from `Router` `tool_call_ref` injection, Proof DAG `evidence.rs` & `linked_sources`, Verifier `<research_sources>`, `literature_worker` / `read_only` subagent tool lists, and reading prompts | Proven (`audit_consistency_evidence_and_tools.rs`) |
| C-02 | `major` | Section-scoped conventions broken by `ux-rust` D4 + `split-1` WP1b + `split-3` WP11 conflict: first pass skips `"section"` scope and asks to expand `null` catalog; incomplete (`done: false`) section catalog prematurely expands to `"paper"`; scoped map prompt contradicts `conventions_requested: false` | Proven (`audit_consistency_investigation.rs`) |
| C-03 | `major` | PDF `selected_pages` + section scope leaves out-of-page lines unclipped in `ScopeResolution` and `paper_reader::plan_slices` (violating `SelectedPages::check_range`), and scoped prompts duplicate `(lines A–B) (lines A-B)` | Proven (`audit_consistency_investigation.rs`) |
| C-04 | `major` | In-core Investigation freezes Research Proof-DAG / `ledger_append` / `final_answer` delegation instructions into `session.start.system_prompt` alongside `"Do not build a Proof DAG"`, and `INVESTIGATION_SYSTEM_PROMPT` falsely claims `$...$` math does not render in Paper view | Proven (`audit_consistency_evidence_and_tools.rs`, `audit_consistency_ui.test.mjs`) |
| C-05 | `major` | `paper_reader` contract mismatches: unstripped `_paper_scope` allows client spoofing; `section_scoped_marker` checks `"conventions_scope"` instead of `"scope"` on `conventions_resumed` (breaking reader auto-enable); UI `#paper-readers-enabled` checkbox cannot represent or save `false` | Proven (`audit_consistency_investigation.rs`, `audit_consistency_ui.test.mjs`) |
| C-06 | `major` | Question-only Investigation sessions cannot open a linked Research handoff (`400 "Map the paper first."`) despite Design 02 P1b | Proven (`audit_consistency_investigation.rs`) |
| C-07 | `minor` | `#investigation-scope-chip` stays stuck on prior section scope during and after whole-paper conventions expansion or branch turns | Proven (`audit_consistency_ui.test.mjs`) |
| C-08 | `minor` | Stop & running status mismatches on Conventions, Branch, and Claim-check turns (`paperViewState` drops `"Stopped..."` once mapped; `investigation_finish.rs` records `"failed"` instead of `"stopped"`) | Proven (`audit_consistency_ui.test.mjs`) |
| C-09 | `minor` | `session_summary.rs` reports `0` Orchestrator `input_tokens`/`output_tokens` on OpenAI (`prompt_tokens`/`completion_tokens`) and Gemini (`promptTokenCount`/`candidatesTokenCount`) while Worker events count all three | Proven (`audit_consistency_evidence_and_tools.rs`) |
| C-10 | `minor` | `sessionSourcesUsed` (`source_links.js`) omits papers cited via `[Lstart–end]` in question-first `investigation.answer` turns, leaving `#used-sources-list` empty | Proven (`audit_consistency_ui.test.mjs`) |
| C-11 | `minor` | Stale user, reference, and security docs (`docs/reference.md`, `README.md`, `assets/guide.js`, `THREAT-MODEL.md`) contradict question-first Investigation, `paper_reader`, `read_paper_lines`, and split `math_*.js` | Unproven (static doc/copy check) |
| C-12 | `nit` | Orphaned `/vendor/katex/auto-render.min.js` (`index.html`, `assets.rs`) still served and loaded on startup with zero callers | Unproven (static asset check; deferred to WP6) |

---

## Detailed Findings (Most Severe First)

### C-01 (`major`) — `read_paper_lines` omitted across `Router`, Proof DAG `evidence.rs`, Verifier `<research_sources>`, worker/subagent tool profiles, and reading prompts

- **Locations**:
  - `crates/tools/src/router.rs:401-410`
  - `crates/desktop-server/src/evidence.rs:39-44, 677-686, 884`
  - `crates/agent/src/verifiers.rs:294-311`
  - `crates/agent/src/delegate.rs:119, 134`
  - `crates/desktop-server/src/investigation_system_prompt.rs:10-12`, `crates/desktop-server/src/investigation_question.rs:58-61`, `crates/desktop-server/src/turn_runner.rs:322`, `crates/desktop-server/src/cli_turn.rs:408`, `crates/agent/src/autonomous_prompts.rs:41`
- **What is wrong**: `split-1` (WP2) registered `read_paper_lines` in `research_tools::register_research_tools` (`crates/research-tools/src/lib.rs:233`) and `resume.rs:207`, and `ux-rust` added it to `investigation_cites.rs:87, 112`, but five cross-crate consumers that handle `fs_read` and `search_paper_passages` were not updated:
  1. `Router::call` (`router.rs:401-410`) injects `"tool_call_ref": call_id` into returned JSON for `"fs_read"` and `"search_paper_passages"`, omitting `"read_paper_lines"`.
  2. When a `proof.entry` cites a `read_paper_lines` `call_id` in `tool_call_ref` (in Research mode, a linked Research handoff, or an Investigation claim-check turn), `SessionEvidence::describe_tool` (`evidence.rs:685, 884`) returns `"kind": "unavailable", "status": "unsupported_reference"` (rendered in `canvas_dag_detail.js:118, 141` as *"Unresolved evidence link · unsupported tool record"*), and `linked_session_sources` (`evidence.rs:41`) drops it from `session.linked_sources` and Claim Trace.
  3. `verification_evidence_context` (`verifiers.rs:294`) builds `<research_sources>` only from `"search_paper_passages" | "search_formal_library"`, withholding `read_paper_lines` excerpts from Step/Full Verifiers.
  4. `literature_worker_tools()` (`delegate.rs:119`) and custom subagent profile `"read_only"` (`delegate.rs:134`) include `"fs_read"` and `"search_paper_passages"`, omitting `"read_paper_lines"`.
  5. Question-first and Research reading prompts (`investigation_system_prompt.rs:10-12`, `investigation_question.rs:58-61`, `turn_runner.rs:322`, `cli_turn.rs:408`, `autonomous_prompts.rs:41`) tell the model to locate passages with `fs_read` or `search_paper_passages` and *"take line numbers directly from search or query results rather than counting"*, omitting `read_paper_lines` even though `fs_read` offset reads do not number individual lines whereas `read_paper_lines` prefixes every line with `N\t`.
- **Concrete input / sequence**: Ingest `boundary.txt`, call `read_paper_lines({source_id, start_line: 2, end_line: 2})` via `Router`, call `ledger_append` with `tool_call_ref: <call_id>`, then `GET /api/sessions/{id}/evidence?entry_id=e-1` and `GET /api/sessions/{id}`.
- **Proposed fix**: Add `"read_paper_lines"` to `router.rs:403`; handle `"read_paper_lines"` in `evidence.rs` (`linked_session_sources` `:41` and `describe_tool` `:681, 750`, reading `result["a_meta"]` + `result["text"]`) and `verifiers.rs:294`; add `"read_paper_lines"` to `literature_worker_tools()` and `"read_only"` in `delegate.rs`; name `read_paper_lines` alongside `search_paper_passages` in the five prompt builders.
- **Proof (`audit_consistency_evidence_and_tools.rs`)**:
  ```text
  thread 'read_paper_lines_populates_tool_call_ref_and_resolves_in_proof_evidence_and_linked_sources' panicked at crates/desktop-server/tests/audit_consistency_evidence_and_tools.rs:216:5:
  assertion `left == right` failed: read_paper_lines missing from Router::call tool_call_ref injection ({"a_meta":{"end_line":2,"file_sha256":"a062e820266947419bd30ac82b9ef727739b3f6f870ca47d695bfc28b5af9813","pages":[],"path":"src-boundary-68d2bf22d5b880d5/boundary.txt","source_seq":1,"start_line":2,"total_lines":2},"text":"2\tLine 2: The boundary variation vanishes.\n"}), evidence.rs::describe_tool ({"kind":"unavailable","request_seq":3,"result_seq":5,"status":"unsupported_reference","tool":"read_paper_lines","tool_call_ref":"e8eab6fc79330ab9","via_entry_id":"e-1"}), and linked_session_sources ([])
    left: (Null, String("unavailable"), String("unsupported_reference"), 0)
   right: (String("e8eab6fc79330ab9"), String("source_read"), String("recorded"), 1)

  thread 'literature_worker_and_read_only_subagent_expose_read_paper_lines' panicked at crates/desktop-server/tests/audit_consistency_evidence_and_tools.rs:254:5:
  literature_worker_tools has fs_read & search_paper_passages but omits read_paper_lines: ["arxiv_search", "literature_search", "search_paper_passages", "doc_ingest", "fs_list", "fs_read", "ledger_query", "ledger_note"]
  ```

### C-02 (`major`) — Section-scoped conventions catalog broken by three-way cross-brief disagreement (`ux-rust` D4 + `split-1` WP1b + `split-3` WP11)

- **Locations**:
  - `crates/desktop-server/src/investigation_scope.rs:72-74, 234-245, 255-266, 311-315`
  - `crates/desktop-server/src/investigation.rs:304-323`
  - `crates/desktop-server/src/investigation_prompts.rs:69-79`
  - `crates/desktop-server/assets/investigation_conventions.js:129-133`
- **What is wrong**:
  1. **First conventions pass skips `"section"` scope and asks to expand a non-existent catalog**: `ux-rust` (D4) set `"conventions_requested": false` in `prepare_map` (`investigation_scope.rs:312`) so initial map turns record no conventions (`catalog == Value::Null`). `split-3` (WP11) wrote `"conventions_scope": "section"` on `investigation.mapping_requested` (`:313`) assuming the map turn built a section catalog. When the user clicks **"Find paper conventions"** (`POST /api/sessions/{id}/investigation/conventions`), `conventions_expanding_to_paper` (`:239-244`) checks `e.payload["conventions_scope"] == "section"` on `mapping_requested` **even when `catalog.is_null()`**, returning `true`! Consequently `investigation.conventions_resumed` is emitted with `"scope": "paper"` instead of `"section"`, and `_paper_context` tells the model `"Expand the section conventions catalog to cover the whole paper. ... Prior batches follow as data: null"`. A section-scoped conventions catalog can never be built from the UI.
  2. **Continuing an incomplete section catalog (`done: false`) prematurely forces whole-paper expansion**: When `catalog` has `coverage_scope: "section", done: false` (e.g. `pending_items` from `split-1` WP1b), `investigation_conventions.js:129-133` labels the button **"Continue conventions review"** (`sectionDone` requires `catalog.done`), yet `conventions_expanding_to_paper` (`:239`) checks `catalog["coverage_scope"] == "section"` without checking `catalog["done"] == true`, emitting `"scope": "paper"` and failing `conventions_finish_done` (`:262`) unless `coverage_scope == "paper"`.
  3. **Contradictory scoped map prompt**: `prepare_map` calls `paper_map_instructions(..., false)` (omitting `record_paper_conventions`), then appends `mapping_instructions_suffix` (`:73`), instructing `"record conventions for the scope plus global notation"`.
- **Concrete input / sequence**: Ingest a numbered-section paper, `POST /api/sessions/{id}/messages` with `{"prompt": "Help me understand section 2", "investigation_source_id": ..., "investigation_scope": "map"}`, let the model record summary + map, then `POST /api/sessions/{id}/investigation/conventions` with `{"map_seq": map_seq}`.
- **Proposed fix**: In `conventions_expanding_to_paper` (`investigation_scope.rs:234-245`), return `true` **only** when `!catalog.is_null() && catalog["coverage_scope"].as_str() == Some("section") && catalog["done"].as_bool() == Some(true)` (and when `catalog.is_null()` on a section-scoped map, emit `"scope": "section"` so the first click builds the section catalog and the second click expands to `"paper"`). Remove `"record conventions for the scope plus global notation"` from `mapping_instructions_suffix` when `conventions_requested == false`.
- **Proof (`audit_consistency_investigation.rs`)**:
  ```text
  thread 'section_scoped_map_then_conventions_builds_section_catalog_before_expanding_to_paper' panicked at crates/desktop-server/tests/audit_consistency_investigation.rs:344:5:
  assertion `left == right` failed: Section-scoped map + conventions bugs:
  start_ctx=... Question scope: Section 2 (lines 4–6) (lines 4-6). ... map only the evidenced main results; record conventions for the scope plus global notation. ...
  conv_resumed={"kind":"investigation.conventions_resumed","payload":{"for_request_seq":4,"map_seq":19,"scope":"paper","source_id":"sections-4a3d22f7999daf10"},...}
  resume_ctx=Expand the section conventions catalog to cover the whole paper. ... Prior batches follow as data:
  null
    left: (true, true, String("paper"), true)
   right: (false, false, String("section"), false)
  ```

### C-03 (`major`) — PDF `selected_pages` + section scope leaves out-of-page lines unclipped in `ScopeResolution` and `paper_reader::plan_slices`, and duplicates `(lines A–B)` in prompts

- **Locations**:
  - `crates/desktop-server/src/investigation_scope.rs:57-75, 85-94, 169-215`
  - `crates/desktop-server/src/paper_reader.rs:188-204`
  - `crates/research-tools/src/investigation_outline.rs:541-548`
- **What is wrong**:
  1. **Unclipped boundary-section lines violate `SelectedPages`**: When a PDF has `investigation_page_start = 2, investigation_page_end = 2` and Section 2 starts on physical page 1 (line 4) and continues onto page 2 (lines 7–8), `clip_to_selected_pages` (`investigation_scope.rs:184-187`) checks page overlap (`a <= last_page && b >= first_page`) and preserves the unclipped `start_line: 4, end_line: 8` (`ranges: [[4, 8]]`). Then `paper_reader::plan_slices` (`paper_reader.rs:188-193`) takes the `(Some((s, e)), _)` branch and **skips `selected_pages` clipping whenever `scope_lines` is `Some`**, assigning reader slice `4..=8`. Any `note_evidence` or `record_paper_*` call on lines `4..=6` then fails `SelectedPages::check_range` (`outside_selected_pages`).
  2. **Duplicated line range in scoped prompt strings**: `resolve_scope` (`investigation_outline.rs:544`) already formats single-target `ScopeResolution::Resolved.label` as `"Section 2 (lines 4–6)"`. Both `mapping_instructions_suffix` (`:73`) and `question_instructions_suffix` (`:92`) format `"Question scope: {label} ({range_str})."`, emitting `"Question scope: Section 2 (lines 4–6) (lines 4-6)."` on every single-target resolved scope.
- **Concrete input / sequence**: Ingest a 2-page PDF where `"2. BOUNDARY TERMS"` begins on page 1 (line 4) and continues onto page 2 (lines 7–8); start a turn with `investigation_page_start: 2, investigation_page_end: 2, prompt: "Help me understand section 2"`.
- **Proposed fix**: Intersect each retained `ScopeTarget`'s `[start_line, end_line]` (and `front_matter`) with the line span of `[first_page..=last_page]` from `page_lines` inside `clip_to_selected_pages`, and intersect `scope_lines` with `selected_pages` in `paper_reader::plan_slices`. Avoid re-appending `({range_str})` in `mapping_instructions_suffix` / `question_instructions_suffix` when `label` already ends with `(lines ...)`.
- **Proof (`audit_consistency_investigation.rs`)**:
  ```text
  thread 'pdf_selected_pages_clips_boundary_section_scope_and_reader_slices' panicked at crates/desktop-server/tests/audit_consistency_investigation.rs:488:5:
  clip_to_selected_pages left page-1 line 4 (< page 2 start 7) in ScopeResolution ({"front_matter":[1,1],"label":"Section 2 (lines 4–8)",...,"ranges":[[4,8]],...,"targets":[{"end_line":8,"kind":"section","label":"Section 2","page_end":2,"page_start":1,"start_line":4,"title":"BOUNDARY TERMS"}]}) and plan_slices ([PaperReaderSlice { start_line: 4, end_line: 8, goal: "Read lines 4–8" }])
  ```

### C-04 (`major`) — In-core Investigation freezes Research Proof-DAG / `final_answer` delegation instructions into `session.start.system_prompt` alongside `"Do not build a Proof DAG"`, and `INVESTIGATION_SYSTEM_PROMPT` contradicts `$...$` math rendering

- **Locations**:
  - `crates/agent/src/autonomous.rs:91-97, 215-223`
  - `crates/agent/src/delegate.rs:189-203, 294-296`
  - `crates/desktop-server/src/turn_runner.rs:293-301, 566-573`
  - `crates/desktop-server/src/investigation_system_prompt.rs:18-21`
  - `crates/desktop-server/assets/math.js:21, 62-76, 105-108`
- **What is wrong**:
  1. **Self-contradictory `system_prompt` frozen into `session.start`**: Design 01 §2.4 (`01-reader-recorder-split.md:153`) required gating off the Research prompt's `ledger_append`/`final_answer` push at `session.start` for Investigation sessions. `ux-rust` added `"Do not build a Proof DAG or use proof-step citation tags on reading turns."` to `INVESTIGATION_SYSTEM_PROMPT` (`investigation_system_prompt.rs:18-19`), **but** `run_autonomous_research_loop` (`autonomous.rs:91-97`) unconditionally appends `format_delegation_prompt_section(dcfg)` (`delegate.rs:191-200`) and freezes the combined string into `session.start.payload["system_prompt"]` (`autonomous.rs:220`). Worse, on a **question-first** Investigation turn (`Scope::Question`, now the default whenever a reading question is entered), `paper_reading_turn` (`turn_runner.rs:294-296`) is `false`, so all four Research workers are active on Turn 1 — freezing `"interpret computation outputs (ledger_append with entry_type: \"result\" / \"derivation\"), and synthesize the final_answer"` plus `delegate_compute` / `delegate_lean` / `delegate_explore` instructions into `session.start` for the entire lifetime of the Investigation session (including later map/conventions/branch resume turns). In addition, `delegate_literature`'s worker prompt (`delegate.rs:295`) instructs the worker to record `reference_lookup` Proof DAG entries on Investigation turns.
  2. **`INVESTIGATION_SYSTEM_PROMPT` contradicts `latex` single-dollar rendering**: `investigation_system_prompt.rs:20-21` tells the model *"Write inline math with `\(...\)` and display math with `$$...$$`, because single dollar delimiters do not render in the Paper view."* Brief `latex` (WP2/WP3, `math.js:21, 62-76, 105-108`) enabled `$...$` single-dollar KaTeX rendering across both `renderRichInto` (Answer cards) and `renderMathTextInto` (Summary, Results, Conventions, node details) — and `investigation.js:233` itself wraps bare TeX labels in `'$' + term + '$'`.
- **Concrete input / sequence**: Create an Investigation session, ingest `paper.txt`, `POST /api/sessions/{id}/messages` with `{"prompt": "What happens to the boundary term?", "investigation_source_id": ..., "investigation_scope": "question"}`, and inspect `session.start.payload.system_prompt`.
- **Proposed fix**: Omit the Proof-DAG/`final_answer` orchestrator lead-in from `format_delegation_prompt_section` (and `reference_lookup` DAG instructions in `delegate_literature`, plus calculation/explore workers on non-action Investigation turns) when `is_investigation_session` is true; update `INVESTIGATION_SYSTEM_PROMPT` item 5 to state accurate math delimiter support (`$...$`, `\(...\)`, `$$...$$`, `\[...\]`).
- **Proof (`audit_consistency_evidence_and_tools.rs` & `audit_consistency_ui.test.mjs`)**:
  ```text
  thread 'investigation_session_start_system_prompt_does_not_contradict_itself_or_math_renderer' panicked at crates/desktop-server/tests/audit_consistency_evidence_and_tools.rs:332:5:
  Investigation session.start freezes Research Proof-DAG/final_answer delegation prompt AND stale single-dollar claim:
  ...
  4. Ledger Use: Use `ledger_append` only when a claim-check turn runs an explicit calculation. Do not build a Proof DAG or use proof-step citation tags on reading turns.
  5. Math Formatting: Write inline math with `\(...\)` and display math with `$$...$$`, because single dollar delimiters do not render in the Paper view.
  ...
  Orchestrator -> Worker Delegation:
  - Act as the high-level Orchestrator: plan derivations, interpret computation outputs (`ledger_append` with `entry_type: "result"` / `"derivation"`), and synthesize the `final_answer`.

  ✖ Paper view math renderer renders single-dollar $...$ inline math despite INVESTIGATION_SYSTEM_PROMPT claiming it does not
    AssertionError [ERR_ASSERTION]: INVESTIGATION_SYSTEM_PROMPT falsely claims single-dollar delimiters do not render in Paper view
  ```

### C-05 (`major`) — `paper_reader` contract mismatches: unstripped `_paper_scope` injection, `"conventions_scope"` vs `"scope"` field-name mismatch on `conventions_resumed`, and 2-state UI checkbox vs 3-state `Option<bool>`

- **Locations**:
  - `crates/desktop-server/src/investigation.rs:253-262, 321`
  - `crates/desktop-server/src/paper_reader.rs:127-159, 321-343, 396-420`
  - `crates/desktop-server/assets/subagents.js:399-419`
- **What is wrong**:
  1. **Unstripped `_paper_scope` on `POST /api/sessions/{id}/messages`**: `investigation::prepare` (`investigation.rs:253-262`) strips `_paper_context`, `_paper_request`, `_paper_map_request`, `_paper_branch_request`, `_paper_conventions_request`, and `_paper_question_request` from client JSON, **omitting `_paper_scope`**, which `split-2` added and `paper_reader.rs:128-133, 396-406` trusts. Any caller posting `"_paper_scope": {"resolved": true, "start_line": 1, "end_line": 2}` can force-enable `delegate_read_paper` (when `paper_readers_enabled` is `null`) and override slice line bounds.
  2. **Field-name mismatch breaks reader auto-enable and slice scoping on `conventions_resumed` (and `branch_requested`)**: `investigation.rs:321` emits `investigation.conventions_resumed` with `"scope": "section" | "paper"`, whereas `paper_reader::section_scoped_marker` (`paper_reader.rs:145`) checks `ev.payload.get("conventions_scope") == Some("section")`. Consequently `should_enable_readers` (`:158`) always returns `false` on section-scoped conventions turns (and `branch_requested` turns, which carry neither field), and `scope_lines` (`:396-420`) returns `None`.
  3. **Settings UI `#paper-readers-enabled` cannot represent or save `paper_readers_enabled: false`**: Rust's `UiSettings.paper_readers_enabled` is `Option<bool>` (`Some(true)` = force on, `Some(false)` = force off even with a dedicated smaller reader model or section scope, `None` = automatic). `subagents.js:400, 408` uses a 2-state checkbox (`cb.checked = paperReadersEnabled === true`; unchecking sends `null`), so users cannot disable automatic reader fan-out from the UI without removing their reader model, and a saved `false` value renders identically to `null` (unchecked) and is overwritten with `null` on toggle.
- **Concrete input / sequence**: (1) Post `{"prompt": "Map the whole paper", "investigation_source_id": ..., "investigation_scope": "map", "_paper_scope": {"resolved": true, "start_line": 1, "end_line": 2}}` with `paper_readers_enabled: null`; (2) resume a section-scoped conventions turn with `paper_readers_enabled: null`; (3) load Settings when `paper_readers_enabled: false` and toggle `#paper-readers-enabled`.
- **Proposed fix**: Add `"_paper_scope"` to the stripped fields in `investigation.rs:253-262`; check `ev.payload.get("scope") == Some("section")` (and look up the parent map's resolved `scope` ranges) in `paper_reader.rs:138-146, 396-420`; replace `#paper-readers-enabled` checkbox in `subagents.js:397-419` with a 3-option `<select>` (`Automatic (smaller model or section-scoped)` -> `null`, `Always on` -> `true`, `Off` -> `false`).
- **Proof (`audit_consistency_investigation.rs` & `audit_consistency_ui.test.mjs`)**:
  ```text
  thread 'incomplete_section_conventions_resume_stays_section_scoped_and_auto_enables_readers' panicked at crates/desktop-server/tests/audit_consistency_investigation.rs:418:5:
    left: (String("paper"), false)
   right: (String("section"), true)

  thread 'external_client_cannot_inject_internal_paper_scope_and_question_only_can_open_research' panicked at crates/desktop-server/tests/audit_consistency_investigation.rs:530:5:
  assertion `left == right` failed: Spoofed `_paper_scope` enabled readers (true) AND question-only Research handoff failed (400: {"error":"Map the paper first."})
    left: (true, 400)
   right: (false, 201)

  ✖ Settings UI #paper-readers-enabled represents 3-state Option<bool> including explicit false override
    AssertionError [ERR_ASSERTION]: UI checkbox cannot represent or save paper_readers_enabled=false (sent null instead)
  ```

### C-06 (`major`) — Question-only Investigation sessions cannot open a linked Research handoff despite Design 02 P1b

- **Locations**:
  - `docs/design/investigation-2026-11/02-investigation-ux.md:275`
  - `crates/desktop-server/src/investigation.rs:56, 473, 551`
  - `crates/export/src/fork.rs:27-29`
  - `crates/desktop-server/assets/investigation_answer.js:34-76`
- **What is wrong**: Design 02 P1b (`02-investigation-ux.md:275`) specifies that *"a session whose only answer is a question turn still opens a handoff Research session."* `ux-ui` did not touch `export::fork` (per file-ownership boundaries), and `ux-rust` left `POST /api/sessions/{id}/investigation/research` (`investigation.rs:473`) gated on `selection` -> `current_map` (`400 "Map the paper first."`) and `export::fork::research_from_paper` (`fork.rs:27-29`) gated on `investigation.map` (acknowledged in `reports/ux-rust.md:53, 98`). Answer cards in `investigation_answer.js` also expose no handoff action. Because question-first turns are now the default path whenever a user supplies a reading question, users cannot hand off an Answer into Research without first mapping the paper and selecting a map node.
- **Concrete input / sequence**: Run a question-first turn (`investigation_scope: "question"`), receive `investigation.answer`, then `POST /api/sessions/{id}/investigation/research` with `{"question": "Extend to moving endpoints"}`.
- **Proposed fix**: Allow `POST /api/sessions/{id}/investigation/research` and `export::fork::research_from_paper` to accept `answer_seq` (or latest `investigation.answer`) when no map node is selected, seeding `investigation.handoff` with the primary paper source receipt, verified `cites`, and answer text, and add an `"Open linked Research"` button on completed Answer cards in `investigation_answer.js`.
- **Proof (`audit_consistency_investigation.rs`)**: Quoted under C-05 above (`400: {"error":"Map the paper first."}`).

### C-07 (`minor`) — `#investigation-scope-chip` stays stuck on prior section scope during and after whole-paper conventions expansion or branch turns

- **Locations**:
  - `crates/desktop-server/src/investigation_view.rs:105-112`
  - `crates/desktop-server/assets/investigation.js:361`
- **What is wrong**: `investigation_view.rs:105-112` computes `investigation.scope` from the latest `mapping_requested | question_requested` event and ignores later `investigation.conventions_resumed` (with `"scope": "paper"`) or `investigation.branch_requested` events; `investigation.js:361` passes `inv.scope` straight to `scopeChip.update(inv.scope)`. After a section-scoped turn (`"Scoped to Section 2 · lines 4–6"`), clicking `"Catalog all conventions"` expands coverage to the whole paper (`conventions.coverage_scope == "paper"`), yet the header chip continues claiming `"Scoped to Section 2"`.
- **Concrete input / sequence**: Map Section 2, complete section conventions, click `"Catalog all conventions"`, inspect `#investigation-scope-chip`.
- **Proposed fix**: Clear or supersede `investigation.scope` in `investigation_view.rs:105-112` when a newer `investigation.conventions_resumed` with `"scope": "paper"` (or whole-paper turn) has occurred, or reflect whole-paper conventions coverage in `scopeChip`.
- **Proof (`audit_consistency_ui.test.mjs`)**:
  ```text
  ✖ investigation.js scopeChip update displays stale section scope after whole-paper conventions expansion
    AssertionError [ERR_ASSERTION]: #investigation-scope-chip continues displaying section scope ("Scoped to Section 2") after whole-paper conventions expansion
    + { detail: 'lines 4–6', text: 'Scoped to Section 2', tooltip: 'Reading scoped to Section 2' }
    - null
  ```

### C-08 (`minor`) — Stop and running status inconsistencies on Conventions, Branch, and Claim-check turns

- **Locations**:
  - `crates/desktop-server/assets/investigation_state.js:24-32, 69-77, 94-103`
  - `crates/desktop-server/src/investigation_finish.rs:64-68, 148-158`
- **What is wrong**:
  1. In `investigation_state.js:26-31`, `isMappingStopped` checks only `investigation.mapping_requested` and returns `false` whenever `inv?.mapping_status === 'mapped'`. Pressing **Stop** during a Conventions, Branch, or Claim-check turn on an already-mapped paper produces `statusText: ""` instead of `"Stopped. Anything already saved is kept below."` Conversely, while a calculation `"check"` action turn is running on a mapped paper, `paperViewState` (`:99`) displays `"Reading the paper · m:ss"`.
  2. In `investigation_finish.rs:64-68`, pressing **Stop** on an `_paper_request` action turn records `investigation.result` with `"status": "failed"` (`"stopped by user"`), whereas `finish_question` (`:148-158`) records `"status": "stopped"`.
- **Concrete input / sequence**: Complete a paper map, click `"Find paper conventions"`, press **Stop** (`session.abort` with `stopped: true`), inspect `#investigation-status` (`statusText === ""`).
- **Proposed fix**: Compute `stopped` in `investigation_state.js` against `latestRequestEvent(events)` (which already includes `conventions_resumed`, `branch_requested`, and `investigation.requested`), distinguish `"Checking selected claim · m:ss"` when `latestReq.kind === 'investigation.requested'`, and emit `"status": "stopped"` in `investigation_finish.rs:64-68` when `abort.payload["stopped"] == true`.
- **Proof (`audit_consistency_ui.test.mjs`)**:
  ```text
  ✖ paperViewState shows Stopped status when a conventions or branch turn is stopped on a mapped paper
    AssertionError [ERR_ASSERTION]: Stopping a conventions turn after a map exists loses the Stopped status message: {...,"hasMap":true,...,"statusText":"",...}
  ```

### C-09 (`minor`) — `session_summary.rs` reports `0` Orchestrator tokens on OpenAI and Gemini sessions while counting Worker tokens

- **Locations**:
  - `crates/desktop-server/src/session_summary.rs:132-135`
  - `crates/agent/src/runner.rs:347`, `crates/agent/src/openai.rs:329`, `crates/agent/src/gemini.rs:271`, `crates/agent/src/delegate.rs:223-225`
- **What is wrong**: `runner.rs:347` records raw provider `usage` on `model.response` (`prompt_tokens`/`completion_tokens` on OpenAI Chat Completions; `promptTokenCount`/`candidatesTokenCount` on Gemini). Worker/Explorer/Paper-Reader delegation (`delegate.rs:223-225`) normalizes all three provider schemas into `worker_input_tokens`/`worker_output_tokens`, whereas `session_summary.rs:133-134` reads only `u["input_tokens"]` and `u["output_tokens"]`, leaving `stats.input_tokens == 0` and `stats.output_tokens == 0` for every OpenAI and Gemini Orchestrator turn.
- **Concrete input / sequence**: Run any OpenAI (`gpt-5.4`) or Gemini Orchestrator turn and read `GET /api/sessions/{id}` `stats`.
- **Proposed fix**: Share `agent::delegate::extract_usage_tokens` (or check `input_tokens` / `prompt_tokens` / `promptTokenCount` and `output_tokens` / `completion_tokens` / `candidatesTokenCount`) in `session_summary.rs:132-135`.
- **Proof (`audit_consistency_evidence_and_tools.rs`)**:
  ```text
  thread 'openai_orchestrator_turn_records_nonzero_input_and_output_tokens_in_session_stats' panicked at crates/desktop-server/tests/audit_consistency_evidence_and_tools.rs:380:5:
  assertion `left == right` failed: session_summary.rs ignores OpenAI prompt_tokens on model.response: {"accepted_entries":0,"campaign_phase":"COMPLETE","delegation_count":0,"honesty_warnings":0,"input_tokens":0,"model":"gpt-5.4","output_tokens":0,...}
    left: Number(0)
   right: Number(42)
  ```

### C-10 (`minor`) — `sessionSourcesUsed` (`source_links.js`) omits papers cited via `[Lstart–end]` in question-first `investigation.answer` turns

- **Locations**: `crates/desktop-server/assets/source_links.js:326-360`
- **What is wrong**: `sessionSourcesUsed` marks an ingested paper as used when referenced in `investigation.summary`, `investigation.conventions`, `investigation.map`, `investigation.handoff`, `linked_sources` (`proof.entry`), or by full title (`>= 12` chars containing a space) / explicit `arxiv:`/`doi:` URL in `finalAnswerTexts`. On a question-first Investigation (`Scope::Question`), none of the map/summary/conventions/DAG events exist, so a local paper cited via `[L2]` in `investigation.answer.cites` is excluded from `#used-sources-list` (*"Cited in this session"*).
- **Concrete input / sequence**: Ingest `notes.txt` (title `"Notes"`), run a question-first Investigation turn that cites `[L2]` (`cites: [{source_id, path, start: 2, end: 2, read: true}]`), open **Sources & Literature**.
- **Proposed fix**: Iterate `session?.investigation?.answers` (or `investigation.answer` events) in `sessionSourcesUsed` (`source_links.js:328`) and mark `bySourceId.get(cite.source_id)` when `cite.read` is true.
- **Proof (`audit_consistency_ui.test.mjs`)**:
  ```text
  ✖ sessionSourcesUsed counts papers cited via [path:Lstart-end] in question-first investigation answers
    AssertionError [ERR_ASSERTION]: Question-first Investigation answer citations do not mark the primary paper as used in Sources & Literature
    0 !== 1
  ```

### C-11 (`minor`, unproven — static documentation audit) — User, reference, and threat-model docs contradict current Investigation, `paper_reader`, `read_paper_lines`, and `math_*.js` behaviour

- **Locations**:
  - `docs/reference.md:30, 99-103`
  - `README.md:26`
  - `crates/desktop-server/assets/guide.js:16`
  - `THREAT-MODEL.md:225, 351-356`
- **What is wrong**:
  1. `docs/reference.md:99-103` states: *"Choose **New investigation**, add a searchable PDF, text or LaTeX file, and optionally enter a reading question. The first reading gives a short **What, Why and How** summary with passages you can open, followed by the paper’s main results."* Entering a reading question now runs a question-only turn (Answer card with `[Lstart–end]` citations, no summary, no map), and conventions are deferred until requested.
  2. `docs/reference.md:30` and in-app `guide.js:16` list built-in specialist agents (*Compute, Literature, Lean, Ultramath Explorer, Step/Full Verifiers, Strategist, Narrator, Explain*) and state that workers favor smaller connected models by default, omitting **Paper Reader** (`paper_reader`, which defaults to isolated Primary Model, `"low"` wire effort, and opt-in / max-parallel controls).
  3. `README.md:26` describes only the pre-redesign Summary -> main result -> Develop flow, omitting question-first answers and checked line citations.
  4. `THREAT-MODEL.md:225` lists `read-workspace` tools as `(`fs_list`, `fs_read`, `doc_ingest`, `tex_audit`)`, omitting `read_paper_lines`, `search_paper_passages`, `search_formal_library`, `record_paper_*`, and worker-only `note_evidence` / isolated `paper_reader` router; `THREAT-MODEL.md:351-356` still describes monolithic `math.js` prior to its split into `math.js`, `math_katex.js`, `math_scan.js`, `math_equations.js`, `markdown.js`, `tex_preview.js`, and `code_highlight.js`.
- **Proposed fix**: Update those four documents/copy blocks to match the merged implementation.

### C-12 (`nit`, unproven — static asset audit) — Orphaned `/vendor/katex/auto-render.min.js` still served and loaded on startup

- **Locations**: `crates/desktop-server/assets/index.html:765`, `crates/desktop-server/src/assets.rs:255`
- **What is wrong**: `math_scan.js` replaced `window.renderMathInElement` across the entire frontend; zero references to `renderMathInElement` remain in `crates/desktop-server/assets/*.js`, yet `auto-render.min.js` is still embedded in the binary and `<script defer>`-loaded on startup (noted as deferred to WP6 in `reports/latex.md`).
- **Proposed fix**: Remove `<script defer src="/vendor/katex/auto-render.min.js"></script>` from `index.html:765`, its route in `assets.rs:255`, and the vendored file when WP6 lands.

---

## Checked & Found Consistent (Including `reports/merge.md` Cross-Brief Wiring)

1. **`finish_question` -> `answer_cites_for_session` -> `investigation_view` -> `paper_cites.js` (`ux-rust` + `ux-ui` + `split-1`)**: Verified `investigation_cites.rs:87, 112` recognizes `read_paper_lines` (`a_meta`), `fs_read`, and `search_paper_passages` from the turn's `tool.result`s; `investigation_view.rs:91-96` populates `answers[i].cites` (`{source_id, path, start, end, read}`), matching `investigation_answer.js:46` and `paper_cites.js:88-109` (`linkLineCitations` linking only `read === true` ranges and passing `{source_id, path, start_line, end_line}` to `openSessionSource`).
2. **`routes_session.rs` `is_turn_running` snapshot ordering (`merge.md` race fix)**: Verified `routes_session.rs:148` samples `state.is_turn_running(&id)` *before* `ledger.read_session(&id)` so a turn finishing mid-request cannot pair pre-finish events with `running: false`.
3. **Poll sanitization of `investigation.evidence` (`split-2`)**: Verified `routes_session.rs:212-222` (`poll_event_payload`) strips `excerpt` and `quote` from `investigation.evidence` on live poll responses while keeping ledger blobs intact.
4. **Pointer-first anchor verification & `resume.rs` drift compatibility (`split-1`)**: Verified `note_evidence` is registered exclusively on the isolated `paper_reader` router (`paper_reader.rs:167`, absent from `all_research_tool_defs` and `cli_route::core_tool_set`), `earlier_defs` golden tool-definition hashes in `investigation_legacy_defs.rs` match pre-split schemas for `resume.rs`, and `resume.rs:207` allows adding `read_paper_lines` on resumed pre-WP2 sessions.
5. **`AgentModelSpec` & `RESERVED_IDS` for `paper_reader` (`split-2`)**: Verified `agent_models.rs:41-47` (`DefaultModelMode::IsolatedPrimaryModel`, `can_disable: false`), `subagents.rs:24` (`RESERVED_IDS` blocks custom subagent ID `"paper_reader"`), `"low"` wire effort on `paper_reader` (`paper_reader.rs:251`), and no wire effort on the four pre-existing workers (`turn_runner.rs:344-368`).
6. **Math macro reset & module split (`latex` + `ux-ui`)**: Verified `app.js:322, 616` calls `resetMathMacros()` on session switch and creation, every split asset (`math_katex.js`, `math_scan.js`, `math_equations.js`, `markdown.js`, `tex_preview.js`, `code_highlight.js`, `paper_cites.js`, `investigation_*.js`) is registered in `assets.rs` with matching paths, and `export` / `ledger` hash-chain verification passes on sessions containing the new `investigation.*` events.

## Decisions Taken Unattended

1. **Test placement without touching production files**: Because `RULES.md` and `audit.md` forbid editing `src/lib.rs` or any existing file, all Rust proofs were written as public-API/HTTP integration tests under `crates/desktop-server/tests/audit_consistency_*.rs` using real `Ledger`, `Router`, `ResearchEnv`, `RunningDesktopServer`, genuine 2-page PDF bytes, and loopback model HTTP stubs.
2. **Severity of C-06 (Question-only Research handoff)**: Graded `major` rather than `minor` because Design 02 P1b (`02-investigation-ux.md:275`) explicitly specifies question-only Research handoff and question-first turns are now the default entry path whenever a user enters a question, even though `reports/ux-rust.md:53, 98` documented leaving `export::fork` untouched.
3. **File-length discipline**: Kept all three new test files (`391`, `546`, `245` lines) and this report (`<= 600` lines) strictly within the 600-line ceiling in `RULES.md`.

## Verification Commands & Counts

- **Baseline Rust tests (`origin/gemini/work/merged`)**:
  - `RE_SEARCH_PYTHON="$(command -v python3)" cargo test --workspace --no-fail-fast -- --skip cli_research_workspace_read_fork_diff_export_and_verify` -> **544 passed, 0 failed, 0 ignored** (82 test targets).
  - `env -u RE_SEARCH_PYTHON cargo test -p cli --test desktop_and_ext cli_research_workspace_read_fork_diff_export_and_verify` -> **1 passed, 0 failed** (hangs on Linux only when `RE_SEARCH_PYTHON` is set, pre-existing per `reports/merge.md`).
- **Baseline JS checks (`origin/gemini/work/merged`)**:
  - `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` -> exit 0.
  - `node --test $(ls crates/desktop-server/tests/*.test.mjs | grep -v audit_consistency)` -> **109 passed, 0 failed**.
- **Clippy**:
  - `cargo clippy --workspace --all-targets -- -D warnings -A clippy::manual_unwrap_or -A clippy::items_after_test_module -A unused_imports` -> exit 0 (**0 warnings** in `audit_consistency_*.rs`; 4 pre-existing Linux/`rustc 1.100.0` warnings in untouched files `evidence.rs:737`, `guardrails.rs:335`, `sandbox_and_fs.rs:6`, `vault_lock_multiprocess.rs:6`).
- **New proving audit tests (expected failures)**:
  - `cargo test -p desktop-server --test audit_consistency_evidence_and_tools --test audit_consistency_investigation --no-fail-fast` -> **0 passed, 8 failed** (proving C-01 through C-06 and C-09).
  - `node --test crates/desktop-server/tests/audit_consistency_ui.test.mjs` -> **0 passed, 5 failed** (proving C-04, C-05, C-07, C-08, C-10).

## What Was Not Reached / Windows-Only

- **Windows-only paths**: `crates/desktop-app` WebView2 shell/CSP host rules, Windows AppContainer worker token sandboxing (`crates/worker/tests/appcontainer_sandbox.rs` Windows branches), and live vendor CLI binaries (`claude`, `antigravity`) were not executed on this Linux VM.
- **Visual browser layout**: DOM/KaTeX/state logic was verified in Node (`node:test`), not in a live WebView2/Chromium layout engine.
