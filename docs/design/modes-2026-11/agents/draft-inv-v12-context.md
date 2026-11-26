# Brief draft-inv-v12-context — Investigate Context Cards and `record_context_cards` (V12)

## 1. Packages and governing design sections

- **Package:** **V12** (Context cards: `record_context_cards` tool added to `investigation/2` profile with rule 6.8 provenance enforcement and rule 6.9 cited-result card validation, `"Prerequisites"` and `"Compare hypotheses"` tasks, right context drawer model-cards panel, and `"Hide unchecked"` toggle).
- **Governing design:**
  - `00-overview.md`: 0.1 (scope revision: V14 deferred, so `investigation/2` reaches its full 3-tool active set here), 2.2 (F1 tool profiles & `investigation/2`), 2.5 (F4 `verify_quote`, `lines_sent`, `context.supplied`), 2.6 & 2.10 (F6 provenance styles & evidence grade scale), 3.2 (rows 20, 24, 25), 3.3 (F9 run-state vocabulary), 7.3 (wording property tests), 8.2 (#1 profiles, #4 dashed = unchecked).
  - `investigate.md`: 3 (stage H Context, three provenance styles), 4.3 (`Context card` record), 4.4 (`investigation.context`), 5.1 (`record_context_cards` tool, effect `ReadWorkspace`), 5.3 (Context cap: 8 model calls), rules 6.3, 6.4, 6.8, 6.9, 6.10, 6.12, 6.15, 6.17–6.19, sections 7–9 (row V12), section 13 (decision 5: unsourced model background allowed for prerequisites only, max 5 short items per node, dashed style, one toggle hides them, never in cited-result slot).
- **Owner decisions (2026-11-25) applied:**
  - **V6 -> V12 profile seam (CRITICAL):** `inv-v6-tasks.md` intentionally added ONLY `read_map_node` and `record_map_reading` to `investigation/2` and deferred `record_context_cards` to V12. V12 completes the 3-tool `investigation/2` profile while keeping `record_prior_work_check` out of active defs (under `may_gain` only, waiting for deferred V14).
  - **`tool_defs.txt` and `research/1` stay completely frozen.** Only `tool_profile.rs` (`investigation/2` and `investigation/1`/`investigation/2` `may_gain`) and `tests/golden/tool_profiles.txt` are updated for `record_context_cards`.

## 2. Start conditions

- **Required merges on `gemini/work/fixed` before coding:**
  - Already merged on base `832eab5bf`: **F0, F1, F2, F4, F6, F8, F9, V1**.
  - Must be merged before starting V12: **V6** (`inv-v6-tasks`, for `investigation_markers.rs`, `map_defs.rs`, `map_fold.rs`, `investigation_tasks.rs`, and `investigation/2`), **V7 & V8** (`draft-inv-v7-v8-v10`, for `investigation_task_prompts.rs`, `m-` nodes, and Open task), and **V11** (`draft-inv-v11-cited`, for `investigation.cited_lookup` records and `investigation_context.js` drawer shell).
- **Owner / coordinator hard gate (DO NOT DISPATCH V12 BEFORE THIS CLEARS):**
  - `RULES.md:57-59` currently authorizes **only** `inv-v6-tasks` for tool/profile additions. Before dispatching V12, the coordinator/owner **MUST** extend `RULES.md:57-59` to explicitly authorize V12's `record_context_cards` definition and `tool_profiles.txt` extension (while keeping `research/1` and `crates/research-tools/tests/golden/tool_defs.txt` completely frozen).

## 3. Base-commit verified interfaces vs future-merge interfaces

### Verified on base `832eab5bf`
- **F1 (`crates/research-tools/src/tool_profile.rs`, 329 lines; `tests/golden/tool_profiles.txt`, 79 lines):** `ToolProfile`, `PROFILE_INVESTIGATION_V1`, `PROFILE_INVESTIGATION_V2`, `register_profile`, `defs_match_exact`, `allows_gain` (requires `EffectClass::ReadWorkspace`). `tests/tool_def_golden.rs` pins `tests/golden/tool_defs.txt` (must remain untouched).
- **F4 (`crates/research-tools/src/quote_core.rs`, `lines_sent.rs`):** `verify_quote`, `accept_quote_with_sent` (note `quote_core.rs:234-242` requires exact `caller` equality between `context.supplied.caller` / `tool.request.caller` and `Router::try_call`'s `ctx.caller`), `lines_sent_with_ledger`, `CONTEXT_SUPPLIED_EVENT`.
- **F9 (`crates/research-tools/src/run_budget.rs`):** `count_model_calls`, `ModelCallBudget`, `InvestigateTaskState`.
- **F6 (`crates/desktop-server/assets/ui_badge.js`, `quote_chip.js`, `run_bar.js`):** `PROVENANCE_STYLES` (`from_paper`, `outside_paper`, `model_background`), `createNodeCardShell`, `createQuoteChip`, `createRunBar`, `BORDER_DASHED`, `MODEL_TEXT_SUFFIX`.

### Future-merge interfaces to verify before coding (not yet in `832eab5bf`)
- **V6 (`inv-v6-tasks`):** `investigation_markers.rs` (shared marker list), `map_defs.rs` (`read_map_node` + `record_map_reading` defs), `map_fold.rs`, `investigation_tasks.rs`, `investigation_task_context.rs` (including its exact `caller` string), and `investigation_v2()` in `tool_profile.rs` (verify whether V6 already placed `record_context_cards` in `investigation/1.may_gain` and/or `investigation/2.may_gain`). **Do NOT remove any tool present in V6's `investigation/2.defs`** (including `search_formal_library` if V6 kept it): removing a recorded tool breaks `crates/agent/src/resume_profile.rs:134-138` fail-closed drift checks; V12 ONLY adds `record_context_cards` (`ReadWorkspace`, permitted in `may_gain`).
- **V8 (`draft-inv-v7-v8-v10`):** `m-` model-node tags in `map_fold.rs` and `investigation_task_prompts.rs`.
- **V11 (`draft-inv-v11-cited`):** `investigation.cited_lookup` event schema (`bib_key`, `external_source_id`, `external_tag`, `status`) and `investigation_context.js`'s `mountContextSection` drawer seam.

## 4. Smallest implementation checkpoints

1. **Checkpoint 1 — `record_context_cards` recorder, marker gating, and `investigation/2` profile update (`crates/research-tools`):**
   - Add `record_context_cards` `ToolDef` (`EffectClass::ReadWorkspace`, `ApprovalScope::Once`, `additionalProperties: false`, schema `{ request_seq, source_id, tag, cards (<= 8) }`) in `map_defs.rs` (or a new sibling `map_context_defs.rs` if `map_defs.rs` is near 500 lines). Never add it to `all_research_tool_defs()` or `register_research_tools()`.
   - Implement `crates/research-tools/src/map_context.rs`:
     - **Rule 6.3 gating:** accept ONLY when the newest marker in the shared `investigation_markers` table is `investigation.task_requested` with `task` in `{"context", "open"}` and matching `request_seq` (closed during `orient`, legacy tasks, and idle). Reject caller-supplied app-only field `origin`.
     - **Rule 6.4 tag scope:** `tag` must exist in the pinned skeleton (or valid `m-` node) and equal the requested tag (or descendant for `open`).
     - **Rule 6.8 provenance assignment by code:**
       - Every supplied anchor is verified via F4 `verify_quote` AND checked against F4 `lines_sent_with_ledger` (rule 6.17), matching V6's exact `caller` string (`context.supplied.caller == ctx.caller`, `quote_core.rs:234-242`).
       - **Never downgrade a failed anchor:** if a card supplies anchors and any anchor fails verification or lines-sent, drop that card with an indexed fault `{ index, code, message, true_text }` (rule 6.10); save valid sibling cards in the same call.
       - All anchors matched in primary investigated source -> app fills `origin = "paper"`.
       - At least one anchor matched in another ingested source -> app fills `origin = "external"`.
       - Zero anchors -> allowed **ONLY** when `type == "prerequisite"`, `body` <= 300 chars (grounded cards <= 900 chars), and total `model_background` cards on `tag` (including previously saved ones in the ledger) stay <= 5 -> app fills `origin = "model_background"`. Otherwise reject that card!
     - **Rule 6.9 cited-result card validation:** when `type == "cited_result"`, require `bib_key` present in pinned `skeleton.bib`, an existing `investigation.cited_lookup` event for that `bib_key` with a located external node, at least one matched anchor inside that located external node, and every hypothesis `{ text, anchor }` anchored inside that located external node. Never allow `model_background` in the `cited_result` slot (`investigate.md` 13 #5).
     - Append `investigation.context { request_seq, skeleton_seq, tag, cards }` and fold into `map_fold.rs` (additive, rule 6.12).
   - Update `crates/research-tools/src/tool_profile.rs` and `crates/research-tools/tests/golden/tool_profiles.txt`:
     - **ONLY add** `record_context_cards` to `investigation_v2().defs` (sorted by id) and wire its handler in `register_profile` for `PROFILE_INVESTIGATION_V2`; **NEVER remove** any tool already present in V6's `investigation_v2().defs` (`resume_profile.rs:134-138`).
     - Ensure `record_context_cards` is in BOTH `investigation_v1().may_gain` AND `investigation_v2().may_gain` (so both pre-F1 `investigation/1` sessions AND V6-era 2-tool `investigation/2` sessions gain `record_context_cards` cleanly on resume without drift!).
     - Keep `research/1` and `tests/golden/tool_defs.txt` **byte-for-byte unchanged**. Never edit `crates/agent/src/resume.rs`.
2. **Checkpoint 2 — `"Prerequisites"` and `"Compare hypotheses"` tasks (`crates/desktop-server`):**
   - Add `crates/desktop-server/src/investigation_context_tasks.rs` and wire thin dispatch/prompt hooks in V6/V7 `investigation_tasks.rs`, `investigation_task_context.rs`, and `investigation_task_prompts.rs` (keeping every file <= 600 lines) for `task: "context"` with `focus: "prerequisites" | "compare_hypotheses"` and cap **8 model calls** (5.3, counted via F9).
   - For `"Compare hypotheses"`, inject the citing node's statement/hypotheses AND the located cited node's text from V11's ingested external source (wrapped as quoted source material and logged via F4 `context.supplied` using the exact same `caller` string as `Router::try_call`'s `ctx.caller` and exact `(source_id, path, file_sha256, start_line, end_line)`).
3. **Checkpoint 3 — Context cards drawer UI & hide-unchecked toggle (`crates/desktop-server/assets`):**
   - Create `crates/desktop-server/assets/investigation_context_cards.js` mounted into V11's `investigation_context.js` drawer seam:
     - Triggers `"Prerequisites"` and `"Compare hypotheses"` tasks with F6 `createRunBar` (Stop / `"Paused: (reason)"` + Continue / Retry).
     - Renders cards in the three F6 provenance styles (`from_paper`, `outside_paper`, `model_background` with dashed border and `"Model background, not checked"` header), using `createQuoteChip` for every anchor and hypothesis quote.
     - One persistent `"Hide unchecked"` toggle that hides all `origin === "model_background"` cards at once (`investigate.md` 3, 13 #5).

## 5. Owned files (all <= 600 lines)

- **New Rust files:**
  - `crates/research-tools/src/map_context.rs` (+ `map_context_tests.rs`)
  - `crates/desktop-server/src/investigation_context_tasks.rs` (context task prompt & external-source injection helpers)
  - `crates/desktop-server/tests/investigation_context_cards.rs`
- **Scoped edits to existing/V6/V7 files (keeping all <= 600 lines):**
  - `crates/research-tools/src/map_defs.rs`, `map_fold.rs`, `tool_profile.rs` (329 lines on base), `lib.rs` (300 lines)
  - `crates/research-tools/tests/golden/tool_profiles.txt` (79 lines on base — ONLY `investigation/2` and `investigation/1`/`investigation/2` `may_gain`)
  - `crates/desktop-server/src/investigation_tasks.rs`, `investigation_task_context.rs`, and `investigation_task_prompts.rs` (thin dispatch/prompt hook into `investigation_context_tasks.rs` for `task: "context"`)
- **New UI & test files:**
  - `crates/desktop-server/assets/investigation_context_cards.js`
  - `crates/desktop-server/tests/investigation_context_cards.test.mjs`

## 6. Shared registration seams and "Do not edit" files

- **Registration seams:**
  - `crates/research-tools/src/tool_profile.rs` + `tests/golden/tool_profiles.txt`: scoped `record_context_cards` addition to `investigation/2` + `may_gain` (gated on `RULES.md:57-59` extension).
  - `crates/research-tools/src/lib.rs`: export `pub mod map_context;`.
  - `crates/desktop-server/src/lib.rs` (215 lines): declare `mod investigation_context_tasks;`.
  - `crates/desktop-server/src/investigation_tasks.rs` & `investigation_task_prompts.rs` (from V6/V7): route `task: "context"` to `investigation_context_tasks.rs`.
  - `crates/desktop-server/src/assets.rs` (365 lines): register `/assets/investigation_context_cards.js`.
  - V11 UI seam (`investigation_context.js`): mount `investigation_context_cards.js` into the drawer via `mountContextSection`.
- **Do NOT edit:**
  - `crates/research-tools/tests/golden/tool_defs.txt` (MUST remain byte-for-byte frozen; `tool_def_golden` untouched!).
  - `research/1` section of `tool_profiles.txt` and `all_research_tool_defs()` / `register_research_tools()`.
  - `crates/agent/src/resume.rs`, `crates/agent/src/runner.rs` (626), `http_srv.rs` (702), `routes_session.rs` (616), `desktop-server/src/investigation.rs` (597), `index.html` (768), `app.js` (932), `Cargo.lock`.

## 7. Tests through the entry point

- **Recorder & profile tests (`crates/research-tools` + `crates/desktop-server/tests/investigation_context_cards.rs`, zero network, scripted transport only):**
  - **Rule 6.3 marker table test:** extend V6's table test to cover 4 old + 2 new recorders (`record_map_reading`, `record_context_cards`) + `read_map_node` against every marker kind (`orient`, `open`, `context`, legacy mapping/branch/conventions/question, finished/idle).
  - **Rule 6.8 branch matrix:** (a) paper-only anchors -> `origin: "paper"`; (b) external ingested anchor -> `origin: "external"`; (c) zero anchors + `prerequisite` + <= 300 chars + <= 5 -> `origin: "model_background"`; (d) > 300 chars or 6th `model_background` card rejected; (e) zero anchors on `symbol` or `cited_result` rejected; (f) bad anchor or unsent lines dropped with `{ index, code, true_text }`, **never** downgraded to `model_background`; (g) mixed-validity call saves valid siblings (rule 6.10); (h) caller-supplied `origin` rejected.
  - **Rule 6.9 `cited_result` card:** rejected without `investigation.cited_lookup` or with anchor outside located external node or hypothesis missing external anchor; accepted when matched against V11's ingested external source.
  - **Profile & resume compatibility:** `tool_def_golden` passes untouched; `tool_profile_golden` passes; `eval_set_r`, `resume_loop`, `resume_drift_tests` pass untouched; a synthetic V6-era 2-tool `investigation/2` ledger fixture resumes cleanly and gains `record_context_cards` via `may_gain`!
  - **Wording check:** zero `"verified"` in any recorder output, prompt, or rendered card.
- **Node UI tests (`node --test`):** three provenance styles rendered via F6 `PROVENANCE_STYLES`, `"Hide unchecked"` toggle hides ONLY `model_background` cards, `run_bar.js` cap/pause/continue, XSS escaping on card titles/bodies/hypotheses.

## 8. Document conflicts & resolutions

- **`inv-v6-tasks.md` vs `00-overview.md` 2.2 / `investigate.md` 5.1 (`record_context_cards` timing):** Overview 2.2 lists 3 new tools in `investigation/2`, while `inv-v6-tasks.md` adds only the first 2 (`read_map_node`, `record_map_reading`) in V6 and assigns `record_context_cards` to V12. Resolution: V12 adds `record_context_cards` to `investigation/2` AND lists it in `investigation/2.may_gain` (and `investigation/1.may_gain`) so sessions started on V6's 2-tool `investigation/2` resume fail-closed without drift.
- **`investigate.md` 4.3 vs 6.8 (`body` length limit):** 4.3 lists `body (<= 900)` while 6.8 restricts `model_background` to `<= 300` chars. Both hold simultaneously: grounded (`paper`/`external`) cards cap at 900 chars; `model_background` prerequisites cap at 300 chars and at most 5 per node.

## 9. Out-of-scope / deferred items and hand-back

- **Out of scope / deferred:** `record_prior_work_check` (V14, stays out of `investigation/2.defs` and under `may_gain` only); deterministic cited lookup (V11); F16b lessons injection.
- **Hand-back (`RULES.md`):** Write `docs/design/modes-2026-11/reports/inv-v12-context.md` and push to `gemini/modes/inv-v12-context`.
