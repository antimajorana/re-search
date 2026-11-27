# Brief inv-v12-context — Investigate Context Cards and `record_context_cards` (V12)

**Status:** Ready / Unblocked (`V6`, `V7`, `V8`, `V10`, `V11`, `V13`, `V17` merged on `gemini/work/fixed`; `RULES.md` explicitly authorizes `inv-v12-context` for `record_context_cards` in `investigation/2` and `tool_profiles.txt`). Zero `Cargo.lock` edits!
**Push branch:** `gemini/modes/inv-v12-context`

---

## 1. Packages and governing design sections

- **Package:** **V12** (`record_context_cards` tool added to `investigation/2` profile with rule 6.8 provenance enforcement and rule 6.9 cited-result card validation, `"Prerequisites"` and `"Compare hypotheses"` tasks, right context drawer model-cards panel, and `"Hide unchecked"` toggle).
- **Governing design:**
  - `00-overview.md`: 0.1 (`V14` deferred; `investigation/2` reaches its full active set here), 2.2 (F1 tool profiles & `investigation/2`), 2.5 (F4 `verify_quote`, `lines_sent`, `context.supplied`), 2.6 & 2.10 (F6 provenance styles & evidence grade scale), 3.2 (rows 20, 24, 25), 3.3 (F9 run-state vocabulary), 7.3 (wording property tests), 8.2 (#1 profiles, #4 dashed = unchecked).
  - `investigate.md`: 3 (stage H Context, three provenance styles), 4.3 (`Context card` record), 4.4 (`investigation.context`), 5.1 (`record_context_cards` tool, effect `ReadWorkspace`), 5.3 (Context cap: 8 model calls), rules 6.3, 6.4, 6.8, 6.9, 6.10, 6.12, 6.15, 6.17–6.19, sections 7–9 (V12), section 13 (decision 5: unsourced model background allowed for prerequisites only, max 5 short items per node, dashed style, one toggle hides them, never in cited-result slot).
  - `agents/RULES.md` (`tool_defs.txt` and `research/1` stay byte-for-byte frozen; `<= 600`-line rule; zero `#[ignore]`; zero `"verified"` in Investigate outputs; zero real network calls/model calls in tests).

---

## 2. Verified Interfaces on `gemini/work/fixed`

- **Profiles (`crates/research-tools/src/tool_profile.rs` `467`L, `tests/golden/tool_profiles.txt`):** `investigation/2` (`policy v0.4-research`) already contains `may_gain ... record_context_cards` alongside `search_formal_library`, `read_map_node`, `record_map_reading`, etc., and `may_gain record_prior_work_check` (deferred V14). **ONLY add** `record_context_cards` to `investigation_v2().defs` (sorted by id) and wire it in `register_profile` for `PROFILE_INVESTIGATION_V2`; ensure `record_context_cards` remains in `investigation/{1,2}.may_gain`; **NEVER remove** any existing V6 tool from `investigation/2.defs` (`resume_profile.rs` drift checks fail closed!).
- **Map & Fold (`crates/research-tools/src/{map_defs.rs (378L), map_fold.rs (422L), map_model_nodes.rs, investigation_markers.rs}`):** `InvestigationTaskKind::Context`, `EVENT_INVESTIGATION_READING`, `record_supplied_for_caller` (`turn_runner.rs` passes caller such as `"orchestrator"` matching `Router::try_call`'s `ctx.caller`). Keep `map_reading.rs` / `map_reading_validate.rs` semantics intact; put new defs in `map_context_defs.rs` if `map_defs.rs` approaches 600L.
- **Tasks & Prompts (`crates/desktop-server/src/{investigation_tasks.rs, investigation_task_context.rs (409L), investigation_task_prompts.rs (187L)}`):** Put `"Prerequisites"` & `"Compare hypotheses"` task prep and prompt/context builders in a new sibling `investigation_context_tasks.rs` with thin dispatch hooks from `investigation_tasks.rs` / `investigation_task_context.rs` / `investigation_task_prompts.rs` (keeping every file `<= 600`L).
- **Cited lookup & Context drawer (`crates/desktop-server/src/investigation_cited.rs`, `assets/investigation_context.js` `543`L):** `investigation.cited_lookup` records `{ bib_key, external_source_id, external_tag, status }`; `investigation_v3_shell.js` mounts drawers via `contextDrawer.mountContextSection(id, element)`. Put V12's drawer UI in a new `crates/desktop-server/assets/investigation_context_cards.js` rather than growing 543-line `investigation_context.js` past 600L!

---

## 3. Smallest Implementation Checkpoints (Push to `gemini/modes/inv-v12-context` After Each!)

1. **Checkpoint 1 — `record_context_cards` recorder, marker gating & `investigation/2` profile (`crates/research-tools`):**
   - Define `record_context_cards` `ToolDef` (`EffectClass::ReadWorkspace`, `ApprovalScope::Once`, `additionalProperties: false`, schema `{ request_seq, source_id, tag, cards (<= 8) }`); NEVER add it to `all_research_tool_defs()` or `register_research_tools()`.
   - Implement `crates/research-tools/src/map_context.rs` (+ `map_context_tests.rs`):
     - **Rule 6.3 marker gating:** accept ONLY when newest marker in `investigation_markers` is `investigation.task_requested` with `task` in `{"context", "open"}` and matching `request_seq` (refuse during `orient`, legacy tasks, or idle). Reject caller-supplied `origin`.
     - **Rule 6.4 tag scope:** `tag` must exist in pinned skeleton (or valid `m-` model node) and equal requested tag (or descendant for `open`).
     - **Rule 6.8 provenance assignment by code:** verify every supplied anchor via F4 `verify_quote` + `lines_sent_with_ledger` (matching `ctx.caller`). **Never downgrade a failed anchor** — drop any card whose anchor fails with `{ index, code, message, true_text }` (rule 6.10) while saving valid siblings. All anchors in primary source -> `origin = "paper"`; any anchor in another ingested source -> `origin = "external"`; zero anchors -> allowed **ONLY** when `type == "prerequisite"`, `body <= 300` chars (grounded cards `<= 900` chars), and cumulative `model_background` cards on `tag` stay `<= 5` -> `origin = "model_background"`; otherwise reject that card.
     - **Rule 6.9 cited-result card validation:** when `type == "cited_result"`, require `bib_key` in pinned `skeleton.bib`, an existing `investigation.cited_lookup` event for `bib_key` with a located external node, at least one matched anchor inside that external node, and every hypothesis `{ text, anchor }` anchored inside that external node (never `model_background` in `cited_result`).
     - Append `investigation.context { request_seq, skeleton_seq, tag, cards }` and fold additively into `map_fold.rs` (projected into `investigation_mapview_v7.rs` / `v8.rs`).
   - Update `tool_profile.rs` + `tests/golden/tool_profiles.txt` (`investigation/2` defs + `investigation/{1,2}` `may_gain` ONLY; keep `tool_defs.txt` & `research/1` untouched!). Run checks & push CP1.
2. **Checkpoint 2 — `"Prerequisites"` / `"Compare hypotheses"` tasks (`crates/desktop-server`):**
   - `crates/desktop-server/src/investigation_context_tasks.rs`: handles `task: "context"` with `focus: "prerequisites" | "compare_hypotheses"`, cap **8 model calls** (counted via F9). For `"Compare hypotheses"`, injects both citing node's statement/hypotheses AND located cited node's text from V11's external source as quoted source material logged via F4 `context.supplied`.
3. **Checkpoint 3 — Context cards drawer UI & `"Hide unchecked"` toggle (`crates/desktop-server/assets`):**
   - `crates/desktop-server/assets/investigation_context_cards.js` (registered in `assets.rs`, mounted via `mountContextSection` in `investigation_v3_shell.js` — add `<= 6` lines in `investigation_v3_shell.js` keeping it `<= 600`L! Better yet, expose a helper or mount hook directly inside `investigation_context_cards.js` / tiny 4-line mount in `investigation_v3_shell.js`!).
   - Renders F6 `PROVENANCE_STYLES` (`from_paper`, `outside_paper`, `model_background` dashed + `"Model background, not checked"`), `createQuoteChip` for every anchor/hypothesis quote, `createRunBar`, and one persistent `"Hide unchecked"` toggle hiding only `origin === "model_background"` cards. Zero `innerHTML`!

---

## 4. Verification & Do-Not-Edit (`RULES.md`)

- **Do NOT edit:** `crates/research-tools/tests/golden/tool_defs.txt`, `research/1` in `tool_profiles.txt`, `crates/agent/src/{resume.rs, runner.rs}`, `crates/desktop-server/src/{turn_runner.rs, agent_models.rs, http_srv.rs, routes_session.rs, investigation.rs}`, `Cargo.lock`.
- **Tests:** `cargo test -p research-tools --test tool_def_golden --test tool_profile_golden` + `eval_set_r`, `resume_loop`, `resume_drift_tests` + `crates/desktop-server/tests/investigation_context_cards.rs` (Rule 6.3 marker matrix, Rule 6.8 branch matrix, Rule 6.9 cited-result checks, V6 2-tool `investigation/2` resume compatibility via `may_gain`, zero `"verified"`), `crates/desktop-server/tests/investigation_context_cards.test.mjs`, `python3 scripts/check_file_length.py --base origin/gemini/work/fixed`, non-root Linux `clippy --workspace --all-targets --locked` + `cargo test --workspace --locked`, write `docs/design/modes-2026-11/reports/inv-v12-context.md`, and push to `gemini/modes/inv-v12-context`.
