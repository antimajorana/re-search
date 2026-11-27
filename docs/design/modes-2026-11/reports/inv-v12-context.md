# Report: `inv-v12-context` (Investigate V12 — Context Cards and `record_context_cards`)

- **Base commit:** `0ec99d316c70025a2acc73c2f9ea12414ea59a45` (`origin/gemini/work/fixed` at lane start)
- **Branch:** `gemini/modes/inv-v12-context`
- **Toolchain on this machine (WSL2 Ubuntu):** `rustc 1.100.0`, `cargo 1.100.0`, `node` (nvm), `Python 3.12.3`
- **Guard rails:**
  - `Cargo.lock` untouched (0 new dependencies)
  - `crates/research-tools/tests/golden/tool_defs.txt` byte-for-byte untouched (`record_context_cards` never added to `all_research_tool_defs()` or `register_research_tools()`)
  - `crates/research-tools/tests/golden/tool_profiles.txt` updated **only** for `[profile investigation/2]` (`def 15 record_context_cards`); `[profile research/1]`, `[profile investigation/1]`, `[profile solve/1]`, `may_gain`, and `earlier_defs` preserved
  - Frozen shared files (`crates/agent/src/{resume.rs, runner.rs}`, `crates/desktop-server/src/{turn_runner.rs, agent_models.rs, http_srv.rs, routes_session.rs, investigation.rs}`) untouched; file-disjoint from V15 (`paper_map*`, `investigation_share*`) and V16 (`investigation_handoff*`, `handoff.rs`)
  - `crates/desktop-server/assets/investigation_v3_shell.js` kept at `595` lines (`<= 600`, `-3` net lines vs base)
  - Zero occurrences of `"verified"` (case-insensitive) in V12 assets, prompts, or API responses; zero `innerHTML` writes in V12 JS UI (`textContent` / DOM builders only)
  - `py -3 scripts/check_file_length.py --base origin/gemini/work/fixed` passes (`No touched source file is over 600 lines.`)

---

## 1. Summary of Changes by Checkpoint

### Checkpoint 1 — `record_context_cards`, `investigation/2` registration, fold projection (commit `2363910`, split in CP2)

| File | Lines | Status | Responsibility |
|---|---:|---|---|
| `crates/research-tools/src/map_context_defs.rs` | 179 | new | Defines `record_context_cards_def()` (`EffectClass::ReadWorkspace`, `ApprovalScope::Once`), `ContextCardType` (`symbol`, `prerequisite`, `cited_result`), `ContextCardOrigin` (`paper`, `external`, `model_background`), `ContextCardRecord`, `MapContextEventPayload`, schema caps (`<= 8` cards, `<= 900` grounded chars, `<= 300` `model_background` chars, `<= 5` `model_background` per node), and `count_model_background_cards`. |
| `crates/research-tools/src/map_context.rs` | 547 | new | Implements `record_context_cards` with Rule 6.3 marker gating (`NewRecorderKind::ContextCards`: active `context` or `open`), Rule 6.4 tag scope check (`tags` for `context`, `compute_open_scopes` for `open`, including `m-` model children), caller `origin`/app-only field rejection, Rule 6.8 `verify_anchor_with_fallback` + `lines_sent_with_ledger` provenance assignment (`paper` / `external` / bounded `model_background`) with strict never-downgrade on failed anchors, Rule 6.10 per-item faults (`true_text` + `anchor_report`), and `investigation.context` event append. |
| `crates/research-tools/src/map_context_cited.rs` | 181 | new | Implements Rule 6.9 `cited_result` validation: verifies `bib_key` in pinned `skeleton.bib`, requires located `investigation.cited_lookup` (`status == "found"` with non-primary `external_source_id` & `external_tag`), loads/builds the external `PaperSkeleton` (`tex-1`, `html-1`, or `pdf-1`) from its verified receipt/blob, and enforces external node span containment on `>= 1` top-level anchor and every hypothesis `{ text, anchor }`. |
| `crates/research-tools/src/map_context_tests.rs` | 569 | new | Unit tests for marker gating, app-only/`origin` rejection, scope rules (`context` vs `open` + `m-` child), Rule 6.8 provenance & never-downgrade, `model_background` 300-char & 5-per-node cumulative caps, `map_fold` integration, and profile exclusion/inclusion (`all_research_tool_defs` vs `investigation/2`). |
| `crates/research-tools/src/map_context_cited_tests.rs` | 97 | new | Rule 6.9 `cited_result` containment tests (candidates-only rejection, wrong external node rejection, hypothesis outside located node rejection, primary-only rejection, and accepted `external` provenance). |
| `crates/research-tools/src/map_fold.rs` | 501 | modified | Adds `context_cards: Vec<VersionedValue<ContextCardRecord>>` to `FoldedNodeReading`, folds `EVENT_INVESTIGATION_CONTEXT` additively with node/skeleton staleness checks, and counts context cards in `FoldAcc::task_counts`. |
| `crates/research-tools/src/{lib.rs, map_defs.rs, map_reading.rs, map_reading_validate.rs, skeleton/mod.rs, tool_profile.rs, tool_profile_tests.rs}` | — | modified | Exposes shared receipt detection & anchor verification (`AnchorSessionCtx`, `verify_anchor_with_fallback`, `reject_app_only_fields_with`), registers `record_context_cards` in `register_map_tools` & `investigation_v2().defs`, keeps `research/1` frozen, and updates profile unit/golden/marker/resume tests. |
| `crates/research-tools/tests/{golden/tool_profiles.txt, tool_profile_golden.rs, investigation_v6_markers_and_tools.rs}` & `crates/desktop-server/tests/{mode_markers_r2.rs, investigation_v1_resume_upgrade.rs}` | — | modified | Updates `investigation/2` golden section, `tool_profile_golden` allowlist, `mode_markers_r2` (`("record_context_cards", &["investigation.context"][..])` & `map_context.rs`), and `investigation_v1_resume_upgrade` (`record_context_cards_def()`). |

### Checkpoint 2 — Context Tasks, External Cited Context Injection & `mapview` Projection (commit `9304eea`)

| File | Lines | Status | Responsibility |
|---|---:|---|---|
| `crates/desktop-server/src/investigation_context_tasks.rs` | 555 | new | Normalizes context task focus (`prerequisites` / `compare_hypotheses`), generates task prompts/instructions for `record_context_cards` (cap 8, Rule 6.19 `DATA ONLY`, zero `"verified"`), injects target node reading metadata + located cited external node text wrapped in `<quoted_source_material>` during `Compare hypotheses` (and logs exact delivered lines via F4 `context.supplied`), sums `investigation.context` counts in `finish_task`, and projects `context_cards` onto `mapview` (`n-...` & `m-...` nodes, F6 `provenance` keys `from_paper` / `outside_paper` / `model_background`, `short_ref`, zero `excerpt` leaks, and primary/external PDF `page`/`ocr` metadata). |
| `crates/desktop-server/src/investigation_context_tasks_tests.rs` | 494 | new | Unit tests covering focus canonicalization, Rule 6.19 prompt guards, `Compare hypotheses` external node injection & F4 `SuppliedRange` crediting, `finish_task` status/counts, and `mapview` projection. |
| `crates/desktop-server/src/{lib.rs, investigation_tasks.rs, investigation_task_context.rs, investigation_task_prompts.rs, investigation_mapview_v7.rs, investigation_skeleton.rs}` | — | modified | Thin hooks for context focus, `Compare hypotheses` external block injection, `investigation.context` accounting in `finish_task`, `record_context_cards` profile check, shared `attach_pdf_pages_to_anchor_obj`, and `mapview` projection/enrichment. |

### Checkpoint 3 — Context Cards Drawer UI, E2E HTTP Tests & Verification

| File | Lines | Status | Responsibility |
|---|---:|---|---|
| `crates/desktop-server/assets/investigation_context_cards.js` | 357 | new | Context cards drawer section mounted into V11's Context drawer via `mountContextSection('context-cards', ...)`: `"Prerequisites"` and `"Compare hypotheses"` buttons, `createRunBar` for active/past `context` tasks, persistent `"Hide unchecked"` toggle hiding only `origin === "model_background"` cards (surviving re-mounts), F6 `PROVENANCE_STYLES` (`from_paper`, `outside_paper`, dashed `model_background` with `"Model background, not checked"`), and `createQuoteChip` for every anchor & hypothesis quote. |
| `crates/desktop-server/assets/{investigation_v3_shell.js, investigation_shell.css}` & `crates/desktop-server/src/assets.rs` | — | modified | Registers `/assets/investigation_context_cards.js`, adds `.inv-ctx-card*` dashed/solid CSS rules, and wires `syncContextCardsDrawer` into `investigation_v3_shell.js` (staying at `595` lines). |
| `crates/desktop-server/tests/investigation_context_cards.test.mjs` | 292 | new | 3 Node tests covering provenance styles, quote chips, hypothesis links, `"Hide unchecked"` toggle + persistence across remounts, RunBar actions, XSS `textContent` escaping (`innerHtmlWrites === 0`), and zero `"verified"` wording. |
| `crates/desktop-server/tests/investigation_context_cards.rs` | 388 | new | End-to-end HTTP + scripted mock LLM integration test covering `"Prerequisites"` (`paper` + `model_background`), `"Compare hypotheses"` (located V11 citation injection + F4 `context.supplied` + `cited_result` `external` + never-downgrade on bad anchor), `/investigation/mapview` projection, and static asset serving (`/assets/investigation_context_cards.js`). |

---

## 2. Verification Summary

- `py -3 scripts/check_file_length.py --base origin/gemini/work/fixed`: passed (`No touched source file is over 600 lines.`)
- `node --check crates/desktop-server/assets/*.js`: passed
- `node --test crates/desktop-server/tests/*.test.mjs`: `211/211` passed (`3/3` in `investigation_context_cards.test.mjs`)
- Targeted Rust tests passed across CP1/CP2/CP3: `map_context` (`5/5`), `map_fold` (`3/3`), `map_reading` (`7/7`), `tool_profile` (`7/7`), `tool_def_golden` (`3/3`), `tool_profile_golden` (`3/3`), `investigation_v6_markers_and_tools` (`3/3`), `agent::resume_profile` (`5/5`), `desktop-server::investigation_context_tasks` (`2/2`), `desktop-server::investigation_task*` (`7/7`), `desktop-server::investigation_mapview*` (`5/5`), `desktop-server::investigation_v6_tasks` (`2/2`), `desktop-server::investigation_v7_orient_badges` (`2/2`), `desktop-server::investigation_v7_v8_v10` (`2/2`), `desktop-server::mode_markers_r2` (`3/3`), `desktop-server::investigation_v1_resume_upgrade` (`3/3`), `desktop-server::investigation_context_cards` (`1/1`)
- Full workspace Clippy & workspace test suite executed on the final CP3 commit and reported with exact counts in the final handoff response.
