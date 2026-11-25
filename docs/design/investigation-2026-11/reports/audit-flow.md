# Audit Report — Lens: `flow` (`audit-flow`)

- **Brief:** `docs/design/investigation-2026-11/agents/audit.md` (`flow` lens)
- **Audited branch / base commit:** `origin/gemini/work/merged` (`351b1287c9a54a700b0e33111b3891f195afd6d1`)
- **Audit branch:** `gemini/work/audit-flow`
- **Scope:** Rust turn flow in `crates/agent`, `crates/research-tools`, and `crates/desktop-server/src` (plus cross-checks in `crates/cli-route` and `crates/export`): question-first turns, scope resolution, outline extraction, conventions gating, parallel Paper Reader fan-out, session-read vs running-turn races, error paths, turn limits, Research mode invariance, and every decision in `docs/design/investigation-2026-11/README.md`.
- **Environment:** Linux (Ubuntu 24.04), `rustc 1.100.0`, `cargo 1.100.0`, `node v22.23.5`, `python3 3.12` (`sympy`, `numpy`, `mpmath`).

---

## 1. Executive Summary

- **Critical findings (sandbox, approval, policy engine, ledger hash chain, or unverified paper quote saved as verified):** **0**. Policy precedence (`deny > ask > allow`), tool approval digests, workspace/source sandboxing, ledger SHA-256 hash chaining, and fail-closed recorder verification remain intact.
- **Proven bugs (16 failing tests across 4 new `audit_flow_*.rs` files, 0 production or existing test edits):** **6 `major`**, **3 `minor`** (plus **1 `minor` unproven** design/race group and **1 `nit`**). Most stem from seams between the parallel waves (`ux-rust`, `split-1`, `split-2`, `split-3`).

---

## 2. Findings (Ordered Most Severe First)

### F1 (`major`) — Checked answer citations (`investigation.answer.payload["cites"]`) mark isolated worker reads and 0-char past-EOF `fs_read` calls as `read: true`

- **Locations:**
  - `crates/desktop-server/src/investigation_cites.rs:66-93` (`collect_received_ranges` ignores `tool.request.payload["caller"]`)
  - `crates/desktop-server/src/investigation_cites.rs:148-153` (`fs_read` branch ignores `returned_chars == 0` / empty `content`)
  - `crates/research-tools/src/fs_tools.rs:338-341` (past-EOF `fs_read` on a file without trailing `\n` returns `content: ""`, `returned_chars: 0`, `start_line: total_lines`, `end_line: total_lines`)
- **What is wrong:**
  1. UX Decision 6 (`README.md:39`) requires clickable citations *only for lines the model actually received*, and `INVESTIGATION_SYSTEM_PROMPT` Rule 6 (`crates/desktop-server/src/investigation_system_prompt.rs:30-32`) tells the Orchestrator to use workers only to locate passages and read cited lines itself before citing them. On Question turns, delegated workers (`delegate_literature`, `delegate_explore`, `delegate_subagent`) execute through the same session `Router` (`crates/agent/src/delegate.rs:361`), recording `tool.request` with `caller: "worker:<role>:..."` and `tool.result` in the session ledger while returning only a summary to the Orchestrator. `collect_received_ranges` never filters `tool.request` by `caller` (`agent:...`), so lines read exclusively inside an isolated worker are stamped `read: true` on the Orchestrator's answer.
  2. When the Orchestrator calls `fs_read` with `offset >= total_chars` on a source without a trailing newline, `fs_read` returns `returned_chars: 0` and `content: ""` alongside `start_line = end_line = total_lines`. `collect_received_ranges` pushes `(start_line, end_line)` unconditionally, marking the last line `read: true` even though zero characters were delivered.
- **Concrete input / sequence:** On a 19-line paper (no trailing `\n`), run a Question turn where `delegate_literature` calls `fs_read(offset=0, max_chars=60)` (reading line 1 inside the isolated worker) and the Orchestrator calls `fs_read(offset=99999)` (0 chars returned, `start_line=19, end_line=19`), then answers `"The chain is introduced [L1] and protected against disorder [L19]."`. Both citations are recorded as `read: true`.
- **Proving test:** `crates/desktop-server/tests/audit_flow_turns.rs` — `checked_answer_cites_ignore_isolated_worker_reads_and_eof_zero_char_fs_read`:
  ```text
  thread 'checked_answer_cites_ignore_isolated_worker_reads_and_eof_zero_char_fs_read' panicked at crates/desktop-server/tests/audit_flow_turns.rs:532:5:
  assertion `left == right` failed: line 1 was read ONLY inside isolated literature_worker (never received by Orchestrator), yet marked read=true: [Object {"end": Number(1), "read": Bool(true), "start": Number(1)}, Object {"end": Number(19), "read": Bool(true), "start": Number(19)}]
    left: Bool(true)
   right: false
  ```
- **Proposed fix:** In `collect_received_ranges`, accept `tool.request` entries only when `caller` is absent (legacy fixtures) or starts with `"agent:"` (excluding `"worker:"` and `"paper_reader"` callers), and in the `fs_read` slice branch skip pushing `(start_line, end_line)` when `val["returned_chars"].as_u64() == Some(0)` or `val["content"].as_str() == Some("")`.

---

### F2 (`major`) — New Investigation sessions freeze Research `ledger_append` / `final_answer` synthesis duties into `session.start.payload["system_prompt"]` and expose Research workers on Question turns

- **Locations:**
  - `crates/agent/src/autonomous.rs:91-97` (`run_autonomous_research_loop`)
  - `crates/agent/src/delegate.rs:189-212` (`format_delegation_prompt_section`)
  - `crates/desktop-server/src/turn_runner.rs:294-374, 402-428` (`paper_reading_turn`, `roles.delegation`)
  - `crates/desktop-server/src/cli_turn.rs:407-419` (`formal_library_instructions` / `compute_evidence_instructions` prepended unconditionally on CLI routes)
- **What is wrong:** Design 01 §2.4 ("The Research prompt's `ledger_append`/`final_answer` push must be gated off for Investigation sessions at `session.start`") and UX Decision 3 (`README.md:36`: "New Investigation sessions use their own system prompt (no proof-graph duty, `[L…]` line citations)") were split across `split-1` and `ux-rust`. While `base_system_prompt(&existing)` selects `INVESTIGATION_SYSTEM_PROMPT`, `turn_runner.rs:381` always passes `roles.delegation = Some(delegation_cfg)`, and `run_autonomous_research_loop` (`autonomous.rs:91-97`) unconditionally appends `format_delegation_prompt_section(dcfg)` (`delegate.rs:190-194`) before emitting `session.start`. This injects `"- Act as the high-level Orchestrator: plan derivations, interpret computation outputs (\`ledger_append\` with \`entry_type: \"result\"\` / \`\"derivation\"\`), and synthesize the \`final_answer\`."` directly into the wire system prompt and freezes it permanently into `session.start.payload["system_prompt"]` for all subsequent resumed turns. In addition, `paper_reading_turn` (`turn_runner.rs:294-296`) omits `_paper_question_request`, so question-first turns expose `delegate_compute`, `delegate_lean`, `delegate_explore`, and `delegate_subagent` and freeze all four `Prefer delegate_*` lines into `session.start`.
- **Concrete input / sequence:** Create `POST /api/sessions` with `{"mode": "investigation"}`, ingest a paper, and send a question-first turn (`POST /api/sessions/{id}/messages` with `investigation_scope: "question"`). Inspect `session.start.payload["system_prompt"]`.
- **Proving test:** `crates/desktop-server/tests/audit_flow_turns.rs` — `investigation_turn_system_prompt_excludes_final_answer_and_research_workers`:
  ```text
  thread 'investigation_turn_system_prompt_excludes_final_answer_and_research_workers' panicked at crates/desktop-server/tests/audit_flow_turns.rs:311:5:
  Investigation session.start froze Research Proof-DAG/final_answer synthesis instructions into system_prompt: You are re:Search, a reading assistant helping the user understand one scientific paper.
  ...
  Orchestrator -> Worker Delegation:
  - Act as the high-level Orchestrator: plan derivations, interpret computation outputs (`ledger_append` with `entry_type: "result"` / `"derivation"`), and synthesize the `final_answer`.
  - Prefer `delegate_compute` (claude-sonnet-4-6) for iterative `sympy_eval` scripts, traceback retries, and `check_expression_sanity`.
  ```
- **Proposed fix:** Pass a mode-aware header flag (or suppress the Proof-DAG / `final_answer` lead bullet in `format_delegation_prompt_section` when `cfg.system_prompt` is not `RESEARCH_SYSTEM_PROMPT`), include `req.get("_paper_question_request").is_some()` in `paper_reading_turn` (retaining `literature_worker` if passage search delegation is desired, or gating workers to `rechecks` turns), and skip `formal_library_instructions` / `compute_evidence_instructions` in `cli_turn.rs:407-419` on non-`rechecks` Investigation turns.

---

### F3 (`major`) — Section-scoped conventions lifecycle breaks when conventions are off on initial map (UX Decision 1) or partially accepted (Split Decision 2)

- **Locations:**
  - `crates/desktop-server/src/investigation_scope.rs:72-74` (`PreparedScope::mapping_instructions_suffix`)
  - `crates/desktop-server/src/investigation_scope.rs:234-245` (`conventions_expanding_to_paper`)
  - `crates/desktop-server/src/investigation_scope.rs:269-283` (`conventions_view_expanding`, called at `crates/desktop-server/src/investigation_view.rs:113-119`)
  - `crates/desktop-server/src/investigation.rs:299-333` & `crates/desktop-server/src/investigation_prompts.rs:69-75`
- **What is wrong:** Four linked defects at the merge seam between `ux-rust` (UX D1: `conventions_requested: false` on `Scope::Map`) and `split-3` (section-scoped conventions):
  1. **Contradictory map prompt without `request_seq`:** `prepare_map` (`investigation_scope.rs:311, 319`) sets `conventions_requested: false` and calls `paper_map_instructions(false)` (omitting `record_paper_conventions` and `request_seq`), but `mapping_instructions_suffix` (`:73`) still appends `"record conventions for the scope plus global notation."` on every resolved scope.
  2. **Null catalog skips section scope:** When the user later clicks `"Find paper conventions"` on that section-scoped map (`catalog == Value::Null`), `conventions_expanding_to_paper` (`:239-244`) returns `true` solely because `mapping_requested.payload["conventions_scope"] == "section"`. `prepare` emits `investigation.conventions_resumed { scope: "paper" }` and prompts `"Expand the section conventions catalog to cover the whole paper. ... Prior batches follow as data:\nnull"`, making a section-scoped conventions catalog impossible to build via the button.
  3. **Incomplete section catalog (`done: false`) prematurely upgrades to `scope: "paper"`:** `conventions_expanding_to_paper` (`:239`) checks `catalog["coverage_scope"] == "section"` without checking `catalog["done"] == true`. When batch 1 has a rejected citation (`done: false`, `pending_items` non-empty), clicking `"Continue conventions review"` upgrades `scope` to `"paper"` instead of finishing the section catalog.
  4. **`conventions_expanding` stuck `true` after failed/stopped turn:** `conventions_view_expanding` (`:269-283`) checks only whether the latest `conventions_resumed` had `scope == "paper"` and `coverage_scope != "paper"`, ignoring `running`, `investigation.conventions_finished`, `session.end`, and `session.abort`. If an expansion turn stops or finishes incomplete, `GET /api/sessions/{id}` reports `conventions_expanding: true` permanently while `running: false`.
- **Concrete input / sequence:** Map `"Help me understand section II.A"`, then `POST /api/sessions/{id}/investigation/conventions`; or record batch 1 with 1 valid item + 1 bad quote (`done: false`) and resume conventions, letting the turn finish without `coverage_scope: "paper"`.
- **Proving tests:** `crates/desktop-server/tests/audit_flow_turns.rs`:
  - `section_scoped_map_prompt_does_not_ask_for_conventions_when_conventions_off`:
    ```text
    section-scoped map with conventions_requested=false still instructs model to record conventions without request_seq: ... Question scope: Section II.A (lines 11–14) (lines 11-14). ... record conventions for the scope plus global notation.
    ```
  - `first_conventions_run_on_section_map_starts_at_section_scope_not_paper_expansion`:
    ```text
    assertion `left == right` failed: with null conventions catalog on a section-scoped map, POST /investigation/conventions skipped section scope and jumped straight to paper expansion: {"for_request_seq":4,"map_seq":5,"scope":"paper","source_id":"aps-1-60aece91308fbe1d"}
      left: String("paper")
     right: "section"
    ```
  - `incomplete_section_conventions_resume_stays_section_and_failed_expansion_clears_flag`:
    ```text
    incomplete section catalog (done=false) upgraded to scope="paper" and left conventions_expanding=true after turn finished (running=false): resumed={"for_request_seq":4,"map_seq":5,"scope":"paper","source_id":"aps-1-60aece91308fbe1d"}
    ```
- **Proposed fix:** Drop `"record conventions for the scope plus global notation"` from `mapping_instructions_suffix` when `conventions_requested == false`; change `conventions_expanding_to_paper` to `catalog["done"] == true && catalog["coverage_scope"] == "section"` (and stamp `resumed_payload["scope"] = "section"` when `!expanding` on a section-scoped map); gate `conventions_view_expanding` on `running` and absence of a newer `conventions_finished` / `session.end` / `session.abort`.

---

### F4 (`major`) — PDF `clip_to_selected_pages` leaves overlapping section and `±20`-line equation/figure ranges unclipped, and multi-target scopes plan reader slices across the entire paper

- **Locations:**
  - `crates/desktop-server/src/investigation_scope.rs:169-215` (`clip_to_selected_pages`)
  - `crates/desktop-server/src/paper_reader.rs:188-205, 396-420` (`plan_slices`, `prepare_for_turn`)
  - `crates/research-tools/src/investigation_outline.rs:513-524` (`item_target` sets `page_start = page_end = item.page` while expanding `start_line..end_line` by `±20` lines across page boundaries)
- **What is wrong:**
  1. When a PDF Investigation is pinned to physical pages `first_page..=last_page`, `clip_to_selected_pages` (`investigation_scope.rs:182-188`) only drops completely disjoint targets (`a <= last_page && b >= first_page`) and leaves `t.start_line`, `t.end_line`, `t.page_start`, `t.page_end`, `ranges`, and `front_matter` **unclipped** against `page_lines`. Meanwhile `plan_slices` (`paper_reader.rs:188, 198-200`) ignores `selected_pages` whenever `scope_lines.is_some()`. As a result, a section spanning pages 2–3 under `page_start=1, page_end=2` (or an equation near the bottom of page 2 whose `+20` window crosses onto page 3) produces `investigation.scope` and Paper Reader slices extending into excluded page 3, where `note_evidence` and `record_paper_*` reject every citation with `outside_selected_pages`.
  2. When a question names two disjoint targets (e.g., `"Map section I and Appendix A"` at lines `5..8` and `271..273`), `prepare_for_turn` (`paper_reader.rs:407-419`) collapses `scope.ranges` to `(min_s, max_e) = (5, 273)`, assigning Paper Readers to read the entire unscoped paper in between. (Conversely, for a single-section map with `overview_only: true`, `scope_lines` omits `scope.front_matter`, so readers are not assigned front-matter lines needed by `record_paper_summary`.)
- **Concrete input / sequence:** (1) Ingest a 3-page PDF where Section II spans pages 2–3 (lines 9–15, page 2 ends at line 11), pin pages `1..=2`, and send `"Help me understand section II"` with `paper_readers_enabled: true`. (2) Ingest a 273-line paper and send `"Map section I and Appendix A"`.
- **Proving tests:** `crates/desktop-server/tests/audit_flow_readers_and_pages.rs`:
  - `pdf_page_range_clips_overlapping_section_for_scope_and_reader_slices`:
    ```text
    selected PDF pages 1..=2 end at line 11, but Section II remained unclipped into excluded page 3 (scope_end=15, reader_slice_end=15): scope={"front_matter":[1,4],"label":"Section II (lines 9–15)",...,"ranges":[[9,15]],...,"targets":[{"end_line":15,"kind":"section","label":"Section II","page_end":3,"page_start":2,"start_line":9,"title":"DEFECT SPECTRUM"}]}, readers=[{"gap_lines":[[9,15]],"i":0,"in_tok":12,"ok":true,"out_tok":18,"pointers":0,"range":[9,15],"turns":1}]
    ```
  - `multi_range_scope_does_not_span_entire_paper_between_targets`:
    ```text
    disjoint scope ranges (Section I + Appendix A, ~9 lines) collapsed to min..max and assigned 269 lines across the entire paper: [{"gap_lines":[[5,273]],"i":0,"in_tok":12,"ok":true,"out_tok":18,"pointers":0,"range":[5,273],"turns":1}]
    ```
- **Proposed fix:** Pass `&page_lines` into `clip_to_selected_pages` to clamp every kept target's `start_line..=end_line` (and `front_matter`) to `[sel_min_line, sel_max_line]`, dropping targets that become empty; in `paper_reader::plan_slices`, always intersect `scope_lines` with `selected_pages` and plan per disjoint range in `scope.ranges` (including `front_matter` on `Scope::Map` turns) rather than taking `(min_s, max_e)`.

---

### F5 (`major`) — Paper Reader `max_parallel` uses Orchestrator provider instead of Reader provider (Decision 5), unconfigured `agent_models.paper_reader` auto-enables primary-model readers (Decision 7), and `delegate_read_paper` `sections` is unbounded

- **Locations:**
  - `crates/desktop-server/src/paper_reader.rs:49-58, 104-125, 141-159, 432`
  - `crates/desktop-server/src/agent_models.rs:335-358`
  - `crates/agent/src/paper_read.rs:73-85, 403-415`
- **What is wrong:**
  1. **Wrong provider for `max_parallel` (Split Decision 5):** `prepare_for_turn` (`paper_reader.rs:432`) calls `effective_max_parallel(ui, req, primary_provider)` with the **Orchestrator's** `primary_provider` instead of `built.provider` (the Paper Reader's actual transport). When Orchestrator = `ChatGpt` and `paper_reader` = `Anthropic` API key, readers are throttled to `2` instead of `4`; conversely, when Orchestrator = `Anthropic`/`OpenAi` API key and `paper_reader` = `ChatGpt`, it launches `4` concurrent streams against the ChatGPT subscription route instead of `2`.
  2. **Silent fallback violates Split Decision 7:** `has_dedicated_reader_model` (`paper_reader.rs:118-123`) checks only `ui.agent_models.get("paper_reader")` syntax without checking whether `state.build_transport(p, &m)` succeeds (and `handle_put_settings` at `routes_session.rs:559-570` retains `ui.agent_models` when an API key is cleared). `should_enable_readers` returns `true` when `paper_readers_enabled` is `None`, and `resolve_paper_reader_transport` (`agent_models.rs:347-357`) falls back to `isolated_primary_model`, running parallel readers on the primary model without user opt-in.
  3. **Unbounded `sections` fan-out:** Unlike `delegate_explore` (which caps `approaches` at `MAX_EXPLORE_BRANCHES = 4`), `paper_read_tool_spec` and `parse_input` (`paper_read.rs:73-85, 403-415`) accept arbitrarily large `sections` arrays, running all `N` up to 12-turn reader loops (`N * 12` model calls) inside a single Orchestrator tool turn.
- **Proving tests:**
  - `crates/desktop-server/tests/audit_flow_readers_and_pages.rs` — `reader_max_parallel_uses_reader_provider_not_orchestrator_provider`:
    ```text
    assertion `left == right` failed: Paper Reader runs on Anthropic API key (`claude-haiku-4-5`), yet `prepare_for_turn` computed `max_parallel` from Orchestrator's ChatGpt provider (got 2)
      left: 2
     right: 4
    ```
  - `crates/desktop-server/tests/audit_flow_readers_and_pages.rs` — `unconfigured_paper_reader_provider_does_not_auto_enable_primary_model_readers`:
    ```text
    unusable `agent_models.paper_reader` (missing OpenAI key) auto-enabled parallel readers on the primary model (`isolated_primary_model`) with `paper_readers_enabled: null`, violating Decision 7
    ```
  - `crates/agent/tests/audit_flow_reader.rs` — `delegate_read_paper_caps_explicit_sections_fan_out`:
    ```text
    explicit `sections` array with 25 slices was neither rejected nor capped (ran 25 reader loops in one tool call)
    ```
- **Proposed fix:** Pass `built.provider` to `effective_max_parallel`; check `resolve_paper_reader_transport` before auto-enabling when `ui.paper_readers_enabled.is_none()` and require `built.routing_reason != "isolated_primary_model"` unless section-scoped; cap `sections.len()` (e.g., `<= MAX_PAPER_READER_SLICES` = 8 or `16`) in `paper_read_tool_spec` (`maxItems`) and `parse_input`.

---

### F6 (`major`) — APS outline drops subsequent subsubsections (`V.D.2`..`V.D.4`) after any body numbered list, `resolve_scope` truncates `"sec. v d 4"` to `Section V`, and multi-line TeX `\tag{...}` shifts later equation numbers

- **Locations:**
  - `crates/research-tools/src/investigation_outline_plain.rs:313-353` (`assemble_aps`)
  - `crates/research-tools/src/investigation_outline.rs:415-419` (`SEC_RE`)
  - `crates/research-tools/src/investigation_outline_tex.rs:97-123` (`build_tex_outline`)
- **What is wrong:**
  1. **APS subsubsection drop:** In `assemble_aps` (`investigation_outline_plain.rs:318-336`), when scanning `CandKind::Dotted([n])` under `V.D`, a sentence-punctuated numbered list (`1. Discrete translation invariance...\n2. Periodic boundary conditions...`) inside `V.D.1` produces `CandKind::Dotted([1])` with `ends_with_dot: true`. Line 322 executes `if n == 1 && last_n >= 1 { break; }` *before* checking `!cands[j].ends_with_dot`, terminating the subsubsection scan at `run = [V.D.1]` and setting `l3_done_for_parent = true`, which permanently drops `V.D.2`, `V.D.3`, and `V.D.4`. (The existing `aps_paper_fixture` placed its numbered list inside `V.D.4`, the *last* subsubsection.)
  2. **Silent scope truncation on lowercase / mixed separators:** `SEC_RE` (`investigation_outline.rs:417`) uses `(?:\.[A-Za-z](?:\.\d+)?|\s+[A-Z](?:\s+\d+)?)?`. Because `[A-Z]` in the whitespace branch is case-sensitive and separators cannot mix, `"walk me through sec. v d 4"` matches `"sec. v"` at `\b` and silently resolves to all of **`Section V`** instead of **`Section V.D.4`** (`"section V.D 4"` likewise truncates to **`Section V.D`**).
  3. **TeX multi-line `\tag{...}` phantom equation & off-by-one shift:** `build_tex_outline` (`investigation_outline_tex.rs:97-123`) processes line-by-line: `\begin{equation}` without `\tag` on the same line increments `eq_num` and emits equation `"1"`, and `\tag{A1}` on the next line emits a second equation `"A1"`, shifting every subsequent untagged `\begin{equation}` off by `+1`.
- **Proving tests:** `crates/research-tools/tests/audit_flow_outline.rs`:
  - `aps_outline_keeps_v_d_4_when_earlier_subsubsection_contains_numbered_list`:
    ```text
    numbered list inside V.D.1 caused assemble_aps to drop V.D.2..V.D.4; got keys: ["Abstract", "I", "II", "III", "IV", "V", "V.A", "V.B", "V.C", "V.D", "V.D.1", "VI"]
    ```
  - `resolve_scope_handles_lowercase_space_separated_and_mixed_separator_aps_refs`:
    ```text
    assertion `left == right` failed: prompt `walk me through sec. v d 4` silently truncated to wrong parent section: [ScopeTarget { kind: "section", label: "Section V", title: Some("DEFECTS"), start_line: 13, end_line: 38, page_start: None, page_end: None }]
      left: "Section V"
     right: "Section V.D.4"
    ```
  - `tex_multiline_tag_does_not_emit_phantom_equation_or_shift_later_numbers`:
    ```text
    assertion `left == right` failed: multiline \tag{A1} emitted phantom equation "1" and shifted later equation numbers: [("1", 5), ("A1", 6), ("2", 10)]
      left: [("1", 5), ("A1", 6), ("2", 10)]
     right: [("A1", 6), ("1", 10)]
    ```
- **Proposed fix:** In `assemble_aps`, ignore `cands[j].ends_with_dot` before the `n == 1 && last_n >= 1` break (or skip restarting `1..k` packed list blocks while continuing until the next `Letter`/`Roman` heading); change `SEC_RE`'s sub-key group to `(?:[.\s]+[A-Za-z](?:[.\s]+\d+)?)?`; in `build_tex_outline`, scan from `\begin{equation|align|gather|eqnarray}` through `\end{...}` as an environment block before assigning `eq_num` vs `\tag{...}` (and skip starred `\begin{equation*}` unless `\tag` is present).

---

### F7 (`minor`) — PRL-style `.tex` manuscripts without `>= 2` `\section` commands index 0 equations and 0 captions

- **Location:** `crates/research-tools/src/investigation_outline_tex.rs:7-22` (`looks_like_tex`)
- **What is wrong:** `looks_like_tex` requires at least two lines containing `\section`, `\subsection`, `\subsubsection`, or `\chapter`. Standard `revtex` / PRL letters use `\documentclass[prl]{revtex4-2}` and `\emph{Introduction.}---` paragraphs with zero `\section` commands, so `build_outline` falls through to `build_plain_outline`, missing every `\begin{equation}` and `\caption{...}`.
- **Proving test:** `crates/research-tools/tests/audit_flow_outline.rs` — `prl_letter_tex_without_two_section_commands_still_indexes_equations_and_figures`:
  ```text
  assertion `left == right` failed: PRL-style TeX without >=2 \section commands indexed 0 equations: PaperOutline { total_lines: 13, style: "weak", weak: true, sections: [], equations: [], captions: [] }
    left: 0
   right: 1
  ```
- **Proposed fix:** Also recognize `\documentclass` / `\begin{document}` (or `>= 1` TeX math/figure environment paired with `\begin{abstract}` / `\caption{`) in `looks_like_tex`.

---

### F8 (`minor`) — Second `delegate_read_paper` call in a turn masks 100% reader failure (`0/N` slices ok) as `ok: true`, and schema requires `"focus"` for `mode: "list"`

- **Locations:**
  - `crates/agent/src/paper_read.rs:525-537` (`any_ok = outcomes.iter().any(|o| o.ok) || ev_count > 0`)
  - `crates/agent/src/paper_read.rs:89, 394-395` (`"required": ["focus"]` vs `if mode == "read" && focus.is_empty()`)
- **What is wrong:** `collect_evidence_rows` (`:525`) gathers all `investigation.evidence` events recorded under `pr.marker_seq` across the entire turn, not just pointers noted during the current `delegate_read_paper` call. If call #1 saves `ev-1` on lines 1–10 and call #2 targeting lines 11–20 fails on every slice (e.g., HTTP 503 / timeout / max_turns), `ev_count` is `1`, so `any_ok` evaluates to `true`, `emit_end` records `delegate.end { ok: true }`, and `ToolResult.is_error` is `false`. Separately, `paper_read_tool_spec` (`:89`) marks `"focus"` as JSON-schema `required` even though `parse_input` (`:395`) makes `focus` optional when `mode == "list"`.
- **Proving test:** `crates/agent/tests/audit_flow_reader.rs` — `second_delegate_read_paper_call_reports_error_when_all_its_slices_fail`:
  ```text
  assertion `left == right` failed: second delegate_read_paper call had 0/1 slices succeed, but prior evidence under marker_seq masked total failure as ok=true: {"call_id":"call_slice_2","coverage":{"gaps":[[11,20]],"lines_read_pct":0},...,"evidence_count":1,...,"ok":true,...,"readers":[{"error":"http_status (HTTP 503): upstream reader unavailable",...,"ok":false,"pointers":0,"range":[11,20],"turns":1}],...}
    left: Bool(true)
   right: false
  ```
- **Proposed fix:** Compute `call_pointers: usize = outcomes.iter().map(|o| o.pointers).sum()` (or filter `events` with `ev.seq > call_start_seq`) so `any_ok = outcomes.iter().any(|o| o.ok) || call_pointers > 0`, and drop `"focus"` from top-level schema `required` (keeping the runtime non-empty check for `mode == "read"`).

---

### F9 (`minor`) — Session-read vs running-turn status glitches and pre-loop user-stop classification

- **Locations:**
  - `crates/desktop-server/src/investigation_view.rs:136-148` (`mark_last_running`, proven)
  - `crates/desktop-server/src/routes_session.rs:302-306` & `crates/desktop-server/src/turn_runner.rs:104-113` (unproven narrow torn-read windows)
  - `crates/desktop-server/src/turn_runner.rs:170-172`, `crates/desktop-server/src/cli_turn.rs:370-372`, `crates/cli-route/src/turn.rs:141-144`, `crates/desktop-server/src/investigation_question.rs:149-153` (unproven narrow pre-start stop window)
- **What is wrong:**
  1. **Proven (`mark_last_running`):** Unlike `branch_history` (`investigation_view.rs:190-201`), `mark_last_running` (`:136-148`) checks only `"session.end" | "session.abort"` after `row["seq"]`. If an earlier question or `rechecks` turn crashed before writing `session.end` / `session.abort`, starting a subsequent mapping, conventions, or branch turn (`running == true`) resurrects that stale `answers` / `activities` entry from `"interrupted"` back to `"running"`.
  2. **Unproven (torn-read windows):** `handle_get_session` (`routes_session.rs:302-306`) samples `running = is_turn_running(...)` *before* `ledger.read_session(...)`. If a new turn inserts into `active_turns` and appends `question_requested` / `branch_requested` between those two lines, `view(&events, false)` flashes `"interrupted"`. Conversely, during teardown (`turn_runner.rs:104-113`), `run_session_turn` appends `session.end` *before* `investigation::finish` appends `investigation.answer` and before `active_turns.remove`, so a poll in that gap calls `view(&events, true)` with `session.end` present and also flashes `"interrupted"`.
  3. **Unproven (pre-loop `POST /stop`):** `ApiTurnGuard` (`turn_control.rs:12-15`) registers `CancelToken` before spawning the worker thread specifically so an immediate `POST /stop` after `202 Accepted` cancels the turn. However, `turn_runner.rs:170-172` returns `Err("stopped by user")` *before* `run_turn_leg` emits `session.abort { stopped: true }`, so `finish_question` (`investigation_question.rs:149-153`) records `investigation.answer { status: "failed" }` instead of `"stopped"`.
- **Proving test (for item 1):** `crates/desktop-server/tests/audit_flow_turns.rs` — `interrupted_question_does_not_resurrect_as_running_during_later_mapping_turn`:
  ```text
  assertion `left == right` failed: earlier crashed question turn resurrected as `running` during a subsequent mapping turn: [{"cites":[],"error":null,"question":"Crashed earlier question?","seq":4,"status":"running","text":null}]
    left: String("running")
   right: "interrupted"
  ```
- **Proposed fix:** Add `"investigation.mapping_requested" | "investigation.conventions_resumed" | "investigation.branch_requested" | "investigation.question_requested" | "investigation.requested"` to `mark_last_running`; sample `is_turn_running` both before and after `ledger.read_session` (`let running = r1 || r2;`) and keep a turn-phase marker until `investigation::finish` completes; emit `session.abort { stopped: true }` (or check `cancel.is_cancelled()` in `investigation::finish`) when cancelled before `session.start`.

---

### F10 (`minor`, unproven — design/prompt discrepancies) — Question-only handoff to Research still requires a map, and Question prompts omit `read_paper_lines`

- **Locations:**
  - `crates/desktop-server/src/investigation.rs:52-63, 473, 551-577` & `crates/export/src/fork.rs:27-29` (vs `docs/design/investigation-2026-11/02-investigation-ux.md:275`)
  - `crates/desktop-server/src/investigation_question.rs:58` & `crates/desktop-server/src/investigation_system_prompt.rs:10`
- **What is wrong:**
  1. Confirming the claim in `reports/ux-rust.md:53, 98`: `02-investigation-ux.md:275` states *"a session whose only answer is a question turn still opens a handoff Research session"*, whereas `POST /api/sessions/{id}/investigation/research` calls `selection(&events, &val)` (`investigation.rs:473` -> `current_map` `"Map the paper first."`) and `export::fork::research_from_paper` (`fork.rs:27-29`) requires a matching `investigation.map` event.
  2. `question_context` (`investigation_question.rs:58`) and `INVESTIGATION_SYSTEM_PROMPT` (`investigation_system_prompt.rs:10`) tell the model to read passages with `fs_read` and `search_paper_passages`, never mentioning `read_paper_lines`, even though `question_instructions_suffix` (`investigation_scope.rs:92`) supplies 1-based line intervals (`lines A-B`) while `fs_read` offset mode uses character offsets and omits per-line `L<n>:` prefixes.
- **Proposed fix:** Either add an answer-seeded handoff path in `investigation.rs` + `export::fork::research_from_paper` (keyed by `answer_seq` + `cites`) or update `02-investigation-ux.md:275` if map-only handoff is intentional; mention `read_paper_lines` alongside `fs_read` and `search_paper_passages` in `question_context` and `INVESTIGATION_SYSTEM_PROMPT`.

---

### F11 (`nit`) — Duplicated line-range parenthetical in scoped prompt suffixes

- **Locations:** `crates/desktop-server/src/investigation_scope.rs:73, 92` & `crates/research-tools/src/investigation_outline.rs:541-545`
- **What is wrong:** `build_scope_label` (`investigation_outline.rs:544`) and `clip_to_selected_pages` (`investigation_scope.rs:198`) already format single-target labels as `"Section II.A (lines 11–14)"`. Both `mapping_instructions_suffix` and `question_instructions_suffix` then format `"\nQuestion scope: {label} ({range_str})."`, emitting `"Question scope: Section II.A (lines 11–14) (lines 11-14)."` (visible in the quoted output of `section_scoped_map_prompt_does_not_ask_for_conventions_when_conventions_off` above).
- **Proposed fix:** Append `({range_str})` only when `targets.len() > 1` (where `label` joins target labels without line numbers).

---

## 3. Verification of `README.md` Decisions Against the Code

| Decision (`docs/design/investigation-2026-11/README.md`) | Verdict | Evidence in code |
|---|---|---|
| **Split 1.** Relocate citations within `±12` lines, narrow to quote `±2`, mark `"relocated"` / `"quote"` | **Correct** | `crates/research-tools/src/investigation_pointers.rs:16-17, 134-212`; `crates/research-tools/src/investigation.rs:344-416` |
| **Split 2.** Partial conventions acceptance (`done: false`, `pending_items`) | **Core correct; section resume broken (F3)** | `crates/research-tools/src/investigation_conventions.rs:96-214` saves valid items and tracks `pending_items`; `investigation_scope.rs:239` mishandles `done: false` on section catalogs |
| **Split 3.** Keep legacy `excerpt` anchor for one release; stop asking for it in prompts | **Correct** | Legacy branch preserved in `crates/research-tools/src/investigation.rs:384-415`; prompts in `research-tools/src/investigation_prompts.rs` and `desktop-server/src/investigation_prompts.rs` ask only for `quote` / `evidence` |
| **Split 4.** Reader model defaults to primary provider; other provider only if assigned | **Correct** | `crates/desktop-server/src/agent_models.rs:335-358` (`resolve_paper_reader_transport` never cross-provider auto-picks) |
| **Split 5.** Reader parallelism: 2 on ChatGPT-plan routes, 4 on API keys, configurable | **Partial bug (F5a)** | Constants & clamp (`1..=8`) in `paper_reader.rs:23-26, 42-58` are right, but `prepare_for_turn:432` passes `primary_provider` instead of `built.provider` |
| **Split 6.** Reader reasoning effort: `low` | **Correct** | `paper_reader.rs:300` passes `Some("low")` only to `paper_reader`; other workers pass `None` (`turn_runner.rs:313, 326, 339, 351`) |
| **Split 7.** Readers on by default only when smaller reader model set or section-scoped; else opt-in | **Partial bug (F5b)** | `should_enable_readers` (`paper_reader.rs:141-159`) implements the rule, but `has_dedicated_reader_model` (`:104-125`) ignores missing credentials and falls back to `isolated_primary_model` |
| **Split 8.** Question-scoped first pass with full conventions catalog on click | **Broken with UX D1 (F3)** | Works only if a section catalog already exists (`done: true, coverage_scope: "section"`); with UX D1 (`conventions_requested: false`), clicking the button skips section scope and leaves `conventions_expanding` stuck on failure |
| **Split 9.** Licence `GPL-3.0-only`, no incompatible new dependencies | **Correct** | `Cargo.toml` unchanged; no new dependencies added; `policy` and `ledger` remain free of `tokio`, `rmcp`, and `windows-sys` |
| **UX 1.** Conventions catalog off in first pass; existing button builds it | **Full-paper correct; section-scoped broken (F3)** | `prepare_map` sets `conventions_requested: false` (`investigation_scope.rs:311`), `finish` succeeds without catalog (`investigation_finish.rs:48-68`), `Scope::Conventions` builds full-paper catalog |
| **UX 2.** Typed question gets answer only, with `"Find main results"` offered | **Correct** | `investigation_question.rs:16-33` routes plain sends to `Scope::Question`; `finish_question` (`:132-173`) records `investigation.answer` |
| **UX 3.** New Investigation sessions use their own system prompt (no proof-graph duty, `[L…]`) | **Partial bug (F2)** | `INVESTIGATION_SYSTEM_PROMPT` (`investigation_system_prompt.rs`) is clean, but `autonomous.rs:91-97` appends `format_delegation_prompt_section` (`ledger_append` / `final_answer`) into `session.start` |
| **UX 4.** Tab names `"Paper / Conversation"` | **Out of `flow` lens (UI)** | Checked in passing (`index.html` / `investigation.js`); owned by `ui` / `consistency` lenses |
| **UX 5.** Keep `DEFAULT_MAX_TURNS` for question turns until measured | **Correct** | `turn_runner.rs:184-189` uses `DEFAULT_MAX_TURNS` (`24`) for question turns unless overridden in request |
| **UX 6.** Clickable citations only for lines the model actually received | **Broken for worker & EOF reads (F1)** | Direct `read_paper_lines` / `search_paper_passages` / `fs_read` checks work, but `collect_received_ranges` leaks worker reads and 0-char EOF `fs_read` |
| **UX 7.** Leave the word `"branch"` unchanged | **Correct** | `investigation.branch*` events, `/investigation/branches`, and `record_paper_branch` unchanged |

---

## 4. Checked and Found Correct

- **Research mode untouched:** Sessions without `investigation.started` (`marker(&events)` is `None`) bypass `investigation::prepare` (`investigation.rs:265-267`), `investigation::finish` (`investigation_finish.rs:15-17`), `investigation_view::view` (`investigation_view.rs:7-9` -> `Value::Null`), and `paper_reader::prepare_for_turn` (`paper_reader.rs:356-363`), keeping `RESEARCH_SYSTEM_PROMPT`, Proof-DAG guardrails, Verifier/Strategist legs, and session title order unchanged. Handoff Research sessions (`investigation.handoff`) inject `_paper_context` while running as full Research sessions.
- **Recorder window gating:** `record_document` (`crates/research-tools/src/investigation.rs:260-310`) closes the map/summary recording window on `investigation.question_requested`, `investigation.answer`, `investigation.mapping_finished`, `session.end`, and `session.abort`, preventing Question or Recheck turns from mutating the saved map.
- **Paper Reader isolation, concurrency, retries, and cancellation:**
  - `register_reader_tools` (`crates/research-tools/src/investigation_evidence.rs:84-90`) registers strictly `read_paper_lines`, `search_paper_passages`, and `note_evidence`; `note_evidence` is absent from `register_research_tools` and rejects callers not starting with `"worker:paper_reader:"`.
  - `note_evidence` verifies SHA-256 of the primary source, enforces `selected_pages`, checks verbatim `quote` containment (with normalized whitespace fallback and `±12` relocation), reslices long lines cleanly on UTF-8 char boundaries (`<= 3200` bytes), and retries `ledger::Error::SessionChanged` up to 8 times under concurrent reader threads.
  - `run_paper_readers` (`crates/agent/src/paper_read.rs:462-538`) bounds worker threads to `slices.len().min(pr.max_parallel.max(1))`, restricts the final turn (`wturn == max_turns`) to `note_evidence` only, propagates `CancelToken` cleanly, trims return payloads to `<= 24 KiB` deterministically, and preserves the ledger hash chain with zero unreferenced blobs.
- **`read_paper_lines`:** Clamps `end_line - start_line + 1` to `120` lines and `32_000` chars, uses 1-based line numbers (`L<n>: `), rejects non-`sources` paths and symlinks via `WorkspaceSandbox`, and places `a_meta` first in canonical JSON so Orchestrator tool-output clipping (`16_000` chars) never truncates `start_line`/`end_line`/`total_lines`.
- **Resume integrity:** `inspect_session_for_resume` (`crates/agent/src/resume.rs`) reuses `session.start.payload["system_prompt"]` without false `SystemPromptDrift` on custom/Investigation prompts while still rejecting policy-version or tool-definition hash drift.

---

## 5. Verification Commands, Counts, Pre-Existing Issues & Platform Limits

### Commands run and exact results

1. **Baseline JS syntax & unit tests (untouched):**
   - `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` -> **exit 0**
   - `node --test crates/desktop-server/tests/*.test.mjs` -> **109 passed, 0 failed**
2. **Baseline Rust workspace tests on `351b1287c9a54a700b0e33111b3891f195afd6d1` (untouched):**
   - `RE_SEARCH_PYTHON="$(command -v python3)" cargo test --workspace --no-fail-fast -- --skip cli_research_workspace_read_fork_diff_export_and_verify` -> **83 test targets, 544 passed, 0 failed**
   - `cargo test --test desktop_and_ext` (without `RE_SEARCH_PYTHON`) -> **2 passed, 0 failed** (**545 total baseline Rust tests passing**)
3. **New `audit_flow_*` proof test binaries (expected failures proving F1–F9):**
   - `cargo test -p research-tools --test audit_flow_outline --no-fail-fast` -> **0 passed, 4 failed**
   - `cargo test -p agent --test audit_flow_reader --no-fail-fast` -> **0 passed, 2 failed**
   - `cargo test -p desktop-server --test audit_flow_turns --no-fail-fast` -> **0 passed, 6 failed**
   - `cargo test -p desktop-server --test audit_flow_readers_and_pages --no-fail-fast` -> **0 passed, 4 failed**
   - Total new proof assertions: **16 failing tests across 4 files** (`195`, `431`, `588`, and `527` lines; all `<= 600` lines).
4. **Clippy check on all new test targets:**
   - `cargo clippy -p research-tools --test audit_flow_outline -p agent --test audit_flow_reader -p desktop-server --test audit_flow_turns -p desktop-server --test audit_flow_readers_and_pages -- -D warnings -A clippy::manual_unwrap_or -A clippy::items_after_test_module` -> **exit 0 (0 warnings in new files)**.

### Pre-existing Linux issues (verified on base `351b1287c9a54a700b0e33111b3891f195afd6d1`)

1. `cargo clippy --workspace --all-targets -- -D warnings` fails on base `351b1287` under `rustc 1.100.0` at `crates/desktop-server/src/evidence.rs:737:40` (`clippy::manual_unwrap_or` false positive on a guarded status allowlist match), plus pre-existing warnings at `crates/agent/src/guardrails.rs:335` (`items_after_test_module`), `crates/research-tools/tests/sandbox_and_fs.rs:6` (`unused_imports` on non-Windows), and `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` (`unused_imports` on non-Windows).
2. Setting `RE_SEARCH_PYTHON` globally on Linux causes `crates/cli/tests/desktop_and_ext.rs::cli_research_workspace_read_fork_diff_export_and_verify` to block in `ScriptedMock::finish()` because `re-search run --route api` rejects non-Windows Python worker configuration before connecting to the mock; running `cargo test --test desktop_and_ext` with `RE_SEARCH_PYTHON` unset passes (`2 passed, 0 failed`).

### What could not be compiled or run on Linux

- `#[cfg(windows)]` targets and code paths: `crates/desktop-app` (WebView2 shell), `crates/worker` (`AppContainerSandbox`, `JobObjectSandbox`, `python_worker`), `crates/chatgpt-auth` DPAPI vault (`crates/desktop-server/tests/chatgpt_route.rs`), `crates/desktop-server/tests/claude_cli_route.rs` / `antigravity_route.rs` Windows process-tree paths, and `crates/cli` Windows containment tests.
- Windows Playwright browser tests (`crates/desktop-server/tests/*_browser.py`).

### Decisions taken unattended

1. **Test file layout:** Split the 16 failing proof tests across four focused integration binaries (`crates/research-tools/tests/audit_flow_outline.rs`, `crates/agent/tests/audit_flow_reader.rs`, `crates/desktop-server/tests/audit_flow_turns.rs`, `crates/desktop-server/tests/audit_flow_readers_and_pages.rs`) so every file stays comfortably below the 600-line rule without touching any existing test or production file.
2. **Linux-safe proof of ChatGPT/API `max_parallel` mismatch (F5a):** Because the `chatgpt` token vault uses Windows DPAPI, proved the `effective_max_parallel` provider bug on Linux by calling `desktop_server::paper_reader::prepare_for_turn` with `primary_provider = ProviderKind::ChatGpt` and `agent_models.paper_reader` assigned to `anthropic` (`claude-haiku-4-5`), exercising the exact mismatch deterministically offline.
3. **Non-flaky race reporting (F9b):** Rather than adding sleep-based timing races to hit narrow torn-read and pre-thread-start `CancelToken` windows, proved the deterministic `mark_last_running` bug with a failing test and labeled the narrow timing windows `"unproven"` per `audit.md`.
