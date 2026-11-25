# Report: split-1

- **Brief**: `docs/design/investigation-2026-11/agents/split-1.md`
- **Design**: `docs/design/investigation-2026-11/01-reader-recorder-split.md`, Phase 1 (WP1a, WP1b, WP2, WP3, WP4, WP5; sections 2.0–2.4, 2.7; user decisions 1–3 in `README.md`)
- **Base commit**: `91753b86b4392814cbe0fe4dd213c9be1d15fc80` (`origin/gemini/runner`)
- **Branch**: `work/split-1` (pushed to `refs/heads/gemini/work/split-1`)
- **Dependencies added**: none (`Cargo.toml`, `Cargo.lock`, `crates/*/Cargo.toml` unchanged; `policy` and `ledger` crates untouched)
- **Legacy test suite**: `crates/research-tools/tests/investigation.rs` has 0 diff and all 6 tests pass.

## Commits

| Commit | Package | Summary |
|---|---|---|
| `2ca071e` | WP1a | Pointer-citation anchor verifier core (`investigation_anchor.rs`) and 19 unit tests (`investigation_anchor_tests.rs`) |
| `4dfbdf0` | WP1b | Partial pointer acceptance, fault reporting, conventions exclusion & pending aggregation, and 13 integration tests |
| `aed1d76` | WP2 | `read_paper_lines` tool (`paper_lines.rs`), registration in `research-tools`, and 7 integration tests |
| `74654f0` | WP3 | `earlier_defs` / `is_current_recorder_def` with golden `7c36a0e` hashes, resume drift compatibility, and 6 tests |
| `1a5ba63` | WP4 | Route-aware prompts (`investigation_prompts.rs`), pointer context strip, conventions pending resume, Phase 1b `DelegationConfig` gating, and 5 tests |
| `9655bc2` | WP5 | Quote `<mark>` highlighting with math-span widening, corrected-range note, pending count in conventions UI, and 5 Node tests |

---

## Package-by-package implementation

### WP1a — Anchor verifier core (`2ca071e`)
- **New files**:
  - `crates/research-tools/src/investigation_anchor.rs` (584 lines)
  - `crates/research-tools/src/investigation_anchor_tests.rs` (446 lines)
- **Wired into**: `crates/research-tools/src/investigation.rs` (`mod anchor`, `verify` / `verify_any` / `anchor_schema` / `AnchorBasis` / `AnchorFault`).
- **Behaviour**:
  - Supports legacy excerpt form (`excerpt`, byte-identical verification & serialization), pointer form (`quote` with no `excerpt`), and explicitly refuses `evidence` form in Phase 1 with `evidence_unavailable`.
  - Refuses app-owned fields (`excerpt_basis`, `cited_range`, `excerpt_window`, `evidence_hash`) and mixed `excerpt` + `quote` inputs.
  - Whitespace-folded substring match (`fold_text`) across line breaks and CRLF, exact on hyphens/punctuation.
  - Quote rules: `< 16` folded chars allowed only when `start_line == end_line` and the quote equals the entire folded line (`>= 2` chars); otherwise `>= 16` folded chars required; max 400 chars.
  - Range correction: narrows in-range matches to `[match_start, match_end + 2]` clamped to the cited range (or full cited range when within 2 lines); relocates unique matches within `+-12` lines (refuses ambiguous with candidate line ranges; refuses `> 12` lines with `true_text` clipped to selected pages); trims blank edges; extends up by up to 2 lines when a theorem/definition/lemma/equation/etc. label sits 1–2 lines above (refusing section names as labels on pointer anchors and never extending outside selected PDF pages).
  - Long-line windowing: lines `> 800` chars produce a `<= 600`-char excerpt centered on the quote with `excerpt_window: [start, end]` byte offsets re-verified against the source line.
  - Primary paper `path` defaults to `primary_path` when omitted; reference anchors require explicit `path`.
- **Tests (19/19 passing)**: `fold_matches_across_line_break`, `hyphen_slip_fails`, `in_range_quote_narrows_to_span_plus_two`, `sixteen_char_quote_never_grounds_64_lines`, `whole_line_short_quote_only_when_range_is_that_line`, `relocates_within_12`, `fails_at_20_with_true_text`, `ambiguous_lists_candidates`, `label_one_line_above_extends`, `label_five_above_fails`, `section_name_label_refused_pointer_only`, `extension_outside_selected_pages_fails`, `crlf_source`, `trailing_blank_trimmed`, `long_line_gets_excerpt_window_and_reslices`, `true_text_clipped_to_selected_pages`, `app_only_field_refused`, `excerpt_and_quote_together_refused`, `path_omitted_primary_filled_reference_refused`.

### WP1b — Partial acceptance, fault reporting, and recorder updates (`4dfbdf0`)
- **Edited files**:
  - `crates/research-tools/src/investigation.rs` (582 lines)
  - `crates/research-tools/src/investigation_summary.rs` (188 lines)
  - `crates/research-tools/src/investigation_conventions.rs` (290 lines)
  - `crates/research-tools/src/investigation_branch.rs` (196 lines)
- **New test file**:
  - `crates/research-tools/tests/investigation_pointers.rs` (405 lines)
- **Behaviour**:
  - `record_paper_map` and `record_paper_branch`: drops individual bad pointer anchors on nodes if `>= 1` verified primary anchor remains on that node; fails whole call if a node loses its last primary anchor or any edge anchor fails; reports `dropped_anchors`, `anchor_basis` (`{model, app, evidence}`), and `anchors_verified`.
  - `record_paper_conventions`: excludes items with invalid pointer anchors (`rejected_items` + `pending_items`), forces `done: false` when any item is rejected, aggregates `pending_items` across batches (removing an item once accepted in a later batch), prevents label-collision rejection against previously excluded items, handles idempotent resend cleanly (returns same `seq` and `rejected_items`), refuses empty-after-exclusion batches without appending an event, and appends an app limitation string `"{N} items awaiting valid citations: {ids}"` (clipped to 500 chars).
  - `record_paper_summary`: reports per-part pointer faults cleanly while preserving legacy error strings for legacy excerpt faults.
  - Multi-fault error formatting caps combined error output at 3,500 chars; legacy excerpt faults return legacy error messages even when pointers are present in the same call.
- **Tests (13/13 passing)**: `map_node_one_good_one_bad_pointer_saved_with_report`, `drop_leaving_no_primary_anchor_fails`, `bad_edge_pointer_fails`, `multiple_faults_one_error_capped_3500`, `legacy_fault_returns_legacy_message_even_with_pointers`, `conventions_three_items_one_bad_saved_two_done_false`, `rejected_item_resent_in_next_batch_no_label_collision`, `identical_resend_returns_same_seq_and_rejected_items`, `all_items_bad_no_append`, `pending_items_aggregated_across_batches`, `summary_bad_part_reports_specific_error`, `fabricated_quote_no_new_event`, `regression_five_live_failure_classes`.

### WP2 — `read_paper_lines` tool (`aed1d76`)
- **New files**:
  - `crates/research-tools/src/paper_lines.rs` (200 lines)
  - `crates/research-tools/tests/paper_lines.rs` (254 lines)
- **Edited file**: `crates/research-tools/src/lib.rs` (261 lines; exports `pub mod paper_lines`, adds `paper_lines::def()` to `all_research_tool_defs()` and registers handler in `register_research_tools()`).
- **Behaviour**:
  - Effect class `ReadWorkspace` (policy Allow).
  - Resolves `source_id` against verified `investigation.source` ledger records, pins file content via `sha256` before reading, refuses `assembled_path` TeX views, caps calls at 400 lines (`MAX_LINES_PER_CALL`), formats `"{line_no}| {text}"`, and packs whole lines into an exact canonical-JSON character budget (`MAIN_BUDGET_CHARS = 14_000` for main registry, `READER_BUDGET_CHARS = 7_000` via `register_with_budget` for reader workers; hard-truncates oversize single lines with `truncated: true`).
  - Sets `a_meta.next_line = delivered_end + 1` when undelivered lines remain (omitted at EOF) and attaches `a_meta.pages` derived from `pdf_page_lines` filtered to overlapping pages.
  - CLI route compatibility verified: `crates/desktop-server/src/cli_turn.rs` and `crates/cli-route/src/turn.rs` record `tool_defs`/`def_hash` on `session.start`/`session.resume` without any `def_hash` drift check against prior turns (`cli-route` 56/56 tests pass).
- **Tests (7/7 passing)**: `numbering_and_line_boundary_cut`, `next_line_is_first_undelivered`, `budget_7000_survives_worker_clip_unchanged`, `oversize_single_line_truncated_flag`, `rewritten_file_refused_by_hash_pin`, `tex_assembled_view_refused`, `pages_meta_from_pdf_page_lines`.

### WP3 — Resume drift compatibility (`74654f0`)
- **New files**:
  - `crates/research-tools/src/investigation_legacy_defs.rs` (168 lines)
  - `crates/agent/src/resume_drift_tests.rs` (103 lines)
- **Edited files**:
  - `crates/research-tools/src/investigation.rs` (exports `earlier_defs`, `is_current_recorder_def`, and golden hash constants)
  - `crates/agent/src/resume.rs` (579 lines)
- **Golden `def_hash` values at `7c36a0e`** (computed in a temporary detached worktree at `7c36a0e`, then hard-coded and verified by unit test `earlier_defs_reproduce_golden_7c36a0e_hashes`):
  - `record_paper_map`: `b58425d3f6b5b16425a9873830c0c5b39454dca3948418ef9ef77236d0475b17`
  - `record_paper_map` (without `answer_node_ids`): `2eb56f1c1bd5ea54bed3995e95eeb98f5e4c8a30697ef3da0bc6f0ed0f6d5fcf`
  - `record_paper_summary`: `c1757b2c8712e12435273a4e655486f7566432380ff13b79c04a3dfcaae91b27`
  - `record_paper_branch`: `c1a960a090f0481b8e2277b08d92a164feb6894af79cdb70cde24b6d84cabf88`
  - `record_paper_conventions`: `ff8682073cb81ffa3f20270b2d9da2a589175e5f739f30d4347f6e7579a2031d`
- **Behaviour**:
  - `check_tool_defs_drift` permits newly added `read_paper_lines` (with `ReadWorkspace` effect) and permits pre-pointer (`7c36a0e`) hashes for all four recorders only when `is_current_recorder_def(exp)` matches the exact current definition; preserves the dynamic `answer_node_ids` shim; rejects modified schemas and reordered tool definitions.
- **Tests (7/7 passing)**: `investigation_legacy_defs::tests::earlier_defs_reproduce_golden_7c36a0e_hashes`, `resume::drift_tests::earlier_map_tool_can_gain_only_its_optional_answer_index` (moved verbatim from `resume.rs`), `golden_7c36a0e_recorder_hashes_accepted`, `answer_index_shim_kept`, `modified_schema_rejected`, `read_paper_lines_may_be_added`, `reordered_defs_rejected`.

### WP4 — Route-aware prompts, context strip, pending resume, and Phase 1b worker gating (`1a5ba63`)
- **New file**: `crates/desktop-server/src/investigation_prompts.rs` (180 lines)
- **Edited files**:
  - `crates/desktop-server/src/investigation.rs` (821 lines — deliberately not split to `<= 600` lines; owned by `ux-rust`)
  - `crates/desktop-server/src/turn_runner.rs` (564 lines)
  - `crates/desktop-server/tests/investigation_flow.rs` (437 lines)
- **Behaviour**:
  - Adds `PaperRoute::{InCore, Cli}` passed from `turn_runner::spawn_turn_thread` into `investigation::prepare`.
  - Prompts instruct the model to cite paper lines using `{source_id, start_line, end_line, quote}` pointers (app fills `excerpt`) and use `read_paper_lines` for line-numbered reads; `PaperRoute::Cli` omits `"Other worker delegation is unavailable during paper reading."` and never mentions `delegate_read_paper`.
  - `context_text` strips `excerpt`, `quote`, `excerpt_basis`, `cited_range`, and `excerpt_window` from anchor objects before forwarding prior reading context.
  - `conventions_resume_instructions` lists any `pending_items` awaiting valid citations when resuming a conventions review.
  - Phase 1b `DelegationConfig` gating in `turn_runner::run_session_turn`: on `_paper_map_request`, `_paper_branch_request`, and `_paper_conventions_request` turns, `compute_worker`, `lean_worker`, `lean_router`, `explorer_worker`, and `custom_subagents` are disabled (`None` / `Vec::new()`), while `literature_worker` stays available; `"check"` turns (`_paper_request`) retain all workers.
  - Research `final_answer` prompt gating was left to `ux-rust`'s Investigation system prompt as instructed in `agents/split-1.md`.
- **Tests (5/5 passing)**: `investigation_prompts::tests::cli_route_context_never_mentions_delegate_read_paper`, `context_text_strips_pointer_fields`, `resume_prompt_lists_pending_items`; `tests/investigation_flow.rs::mapping_turn_offers_no_compute_lean_explore_subagent`, `check_action_keeps_workers` (plus existing `corrected_paper_claim_opens_fresh_research_with_its_evidence_and_assumptions` and all 5 `stage15_interactive_hitl` tests).

### WP5 — UI quote highlighting, corrected-range note, and pending count (`9655bc2`)
- **Edited files**:
  - `crates/desktop-server/assets/investigation_map.js` (599 lines)
  - `crates/desktop-server/assets/investigation_conventions.js` (163 lines)
  - `crates/desktop-server/tests/investigation_map.test.mjs` (224 lines)
  - `crates/desktop-server/tests/investigation_conventions.test.mjs` (37 lines)
- **Behaviour**:
  - `foldSearchSpan(excerpt, quote)` locates `quote` in `excerpt` under whitespace folding and widens the `[start, end]` span to enclose any `$...$`, `$$...$$`, `\(...\)`, or `\[...\]` math delimiter it partially intersects.
  - `renderAnchorExcerptInto(el, excerpt, quote)` renders prefix, `<mark>` quote, and suffix via `renderMathTextInto` (never building HTML strings).
  - When `anchor.cited_range` is present, appends `<small class="investigation-anchor-note">lines corrected by the app (cited A–B)</small>` (or single line `A`).
  - `investigation_conventions.js` adds `pendingCitationsLabel(count)` (`"1 citation pending"` / `"{N} citations pending"`) and displays it in `conventionStatus()` and in the expanded notes block when `catalog.pending_items.length > 0` on the current catalog.
  - Confirmed `rg -n "anchors_verified" crates/desktop-server/assets/` returns 0 matches.
- **Tests (5/5 new tests passing; 68/68 total Node tests passing)**: `quote_mark_uses_text_nodes`, `quote_with_html_chars_escaped`, `mark_widened_to_math_span`, `corrected_note_rendered`, `pending_count_shown`.

---

## Honesty guarantees (section 2.7) mapped to proving tests

| Section 2.7 guarantee | Proving test(s) & verification |
|---|---|
| 1. A stored excerpt is always the exact final lines of the hash-pinned file, or a window of them that is re-checked by its offsets. | `investigation::anchor::tests::fold_matches_across_line_break`, `crlf_source`, `long_line_gets_excerpt_window_and_reslices`; `tests/paper_lines.rs::rewritten_file_refused_by_hash_pin`; `tests/investigation.rs::map_is_grounded_and_scoped_to_the_selected_paper_and_session_mode` |
| 2. A verbatim quote supplied by the recorder or reader was found inside those lines. | `investigation::anchor::tests::fold_matches_across_line_break`, `hyphen_slip_fails`, `fails_at_20_with_true_text`; `tests/investigation_pointers.rs::fabricated_quote_no_new_event` |
| 3. A citation is never accepted from line numbers alone. | `investigation::anchor::tests::excerpt_and_quote_together_refused`, `whole_line_short_quote_only_when_range_is_that_line`, `sixteen_char_quote_never_grounds_64_lines` |
| 4. Every range change is stored next to the model's range, with its basis. | `investigation::anchor::tests::in_range_quote_narrows_to_span_plus_two`, `relocates_within_12`, `trailing_blank_trimmed`, `label_one_line_above_extends`, `app_only_field_refused`; `tests/investigation_map.test.mjs::corrected_note_rendered` |
| 5. Evidence anchors also require the recorder's own quote. | Phase 2 (`split-2`) implements full `evidence` form; in Phase 1 `investigation::anchor::tests::excerpt_and_quote_together_refused` proves `evidence` form is refused fail-closed with `evidence_unavailable`. |
| 6. `anchors_verified` keeps its meaning: "text exists at the cited lines". It goes to the model only; no asset renders it. | `tests/investigation_pointers.rs::map_node_one_good_one_bad_pointer_saved_with_report`; `tests/investigation.rs` (all 6 tests unchanged & passing); `rg -n "anchors_verified" crates/desktop-server/assets/` (0 matches). |

---

## Merge notes

Every edit made by `split-1` to shared files listed in `PARALLEL.md` is enumerated below so the coordinator can merge `ux-rust`, `ux-ui`, `split-2`, `split-3`, and `latex` cleanly:

1. **`crates/research-tools/src/investigation.rs`** (582 lines; main owner `ux-rust` for P0 split; also touched by `split-2`):
   - **Module declarations & re-exports**: added `pub(crate) mod anchor;` (`#[path = "investigation_anchor.rs"]`) and `mod legacy_defs;` (`#[path = "investigation_legacy_defs.rs"]`), re-exporting `earlier_defs`, `is_current_recorder_def`, and the 5 `GOLDEN_*_7C36A0E` constants.
   - **Functions changed**: `paper_map_instructions()`, `def()` (schema now uses `anchor::anchor_schema(false)`), `register()` / `record()` / `record_summary()` / `record_conventions()` / `record_branch()`, `verify_graph_anchors()` (partial pointer drop + fault reporting), `record_document()` (returns `dropped_anchors`, `anchor_basis`, `anchors_verified`).
   - **What other briefs will change**: `ux-rust` will relocate code out of `investigation.rs` during P0 (anchor code is already isolated in `investigation_anchor.rs`, `investigation_anchor_tests.rs`, `investigation_legacy_defs.rs`); `split-2` will add the `evidence` anchor form inside `investigation_anchor.rs::verify_any()` where Phase 1 currently returns `evidence_unavailable`.

2. **`crates/research-tools/src/lib.rs`** (261 lines):
   - **Lines changed**: added `pub mod paper_lines;`, `paper_lines::def()` in `all_research_tool_defs()`, and `paper_lines::register(registry, env.clone())` in `register_research_tools()`.
   - **What other briefs will change**: `split-2` may add `delegate_read_paper` tool registration/defs nearby; keep tool ordering consistent with `crates/agent/src/resume.rs`.

3. **`crates/desktop-server/src/investigation.rs`** (821 lines; main owner `ux-rust`; also touched by `split-3`):
   - **Deliberately NOT shrunk to `<= 600` lines** per `agents/split-1.md` (`ux-rust` owns the split).
   - **Module declaration**: added `#[path = "investigation_prompts.rs"] mod prompts; pub(crate) use prompts::PaperRoute;`.
   - **Call sites changed**:
     - `context_text()`: replaced `object.remove("excerpt")` / `anchor.remove("excerpt")` with `prompts::strip_anchor_fields(...)`.
     - `prepare()`: added `route: PaperRoute` parameter; delegated `instructions` construction to `prompts::map_base_instructions()`, `prompts::conventions_resume_instructions()`, and `prompts::branch_turn_instructions()`.
   - **What other briefs will change**: `ux-rust` will split `desktop-server/src/investigation.rs`, add question turns and the Investigation system prompt (including Research `final_answer` gating); `split-3` will add outline/question scope handling in `prepare()`. Keep the `route: PaperRoute` parameter and `prompts::*` calls when moving `prepare()` / `context_text()`.

4. **`crates/desktop-server/src/turn_runner.rs`** (564 lines; main owner `split-2`; also touched by `ux-rust`):
   - **`spawn_turn_thread()`**: computes `PaperRoute::Cli` vs `PaperRoute::InCore` from `on_cli_route.is_some()` and passes it to `crate::investigation::prepare(...)`.
   - **`run_session_turn()`**: computes `paper_reading_turn` from `_paper_map_request | _paper_branch_request | _paper_conventions_request` and gates `compute_worker`, `lean_worker`, `lean_router`, `explorer_worker`, and `custom_subagents` to `None`/`Vec::new()` in `DelegationConfig` (retaining `literature_worker` and keeping all workers on `"check"` / `_paper_request` turns).
   - **What other briefs will change**: `split-2` will wire the reader worker (`delegate_read_paper`) into `DelegationConfig` on paper reading turns; `ux-rust` will inject the Investigation system prompt (and Research `final_answer` gating).

5. **`crates/agent/src/resume.rs`** (579 lines):
   - **`check_tool_defs_drift()`**: added `"read_paper_lines"` to the allow-list of newly added `ReadWorkspace` tools; added the `is_current_recorder_def(exp) && earlier_defs(&exp.id)` compatibility branch while keeping the `answer_node_ids` shim; moved inline `#[test] fn earlier_map_tool_can_gain_only_its_optional_answer_index` into `crates/agent/src/resume_drift_tests.rs` (included via `#[cfg(test)] #[path = "resume_drift_tests.rs"] mod drift_tests;` at bottom of `resume.rs`).
   - **What other briefs will change**: `split-2` will extend the allow-list if `delegate_read_paper` is added as a session tool or bump `earlier_defs` if Phase 2 adds `evidence` to the recorder schema.

6. **`crates/desktop-server/assets/investigation_map.js`** (599 lines; main owner `split-1`; also touched by `ux-ui`, `split-2`):
   - **Added**: `foldWithIndexMap`, `isEscaped`, `mathSpans`, exported `foldSearchSpan`, exported `renderAnchorExcerptInto`.
   - **Changed**: `createPaperMap()` anchor rendering calls `renderAnchorExcerptInto(quote, anchor.excerpt, anchor.quote)` and appends `<small class="investigation-anchor-note">lines corrected by the app (cited ...)</small>` when `anchor.cited_range` is a 2-element array.
   - **Interface contract**: uses only existing `renderMathTextInto` export from `math.js` (compatible with `latex` rewriting its internals); `ux-ui` can add `openFirst` / header gate and `split-2` can add reader label alongside `cited_range`.

7. **`crates/desktop-server/assets/investigation_conventions.js`** (163 lines; main owner `split-1`; also touched by `ux-ui`):
   - **Added**: exported `pendingCitationsLabel(count)`.
   - **Changed**: `conventionStatus()` appends ` · N citation(s) pending` when `catalog.pending_items.length > 0` on current catalogs; `createConventionCatalog()` renders `pendingCitationsLabel(pendingCount)` in the expanded `notes` block.

---

## Deviations from the design and rationale

1. **Optional `path` parameter on `read_paper_lines`** (`crates/research-tools/src/paper_lines.rs`): Added an optional `path` argument (defaulting to `primary_path`) so multi-file TeX bundles can read non-primary TeX files and so requests targeting `assembled_path` can be explicitly exercised and refused (`tex_assembled_view_refused`).
2. **Research `final_answer` gating deferred to `ux-rust`**: Per `agents/split-1.md`, WP4 implements route-aware wording, pointer-citation instructions, and Phase 1b `DelegationConfig` gating, leaving Research `final_answer` gating to `ux-rust`'s Investigation system prompt.
3. **`crates/desktop-server/src/investigation.rs` remains 821 lines**: Per `agents/split-1.md`, `split-1` only changed call sites and mod lines in `desktop-server/src/investigation.rs` and left the `<= 600`-line split to `ux-rust`.

---

## Decisions taken unattended

1. **Legacy anchor byte-identity**: `verify()` leaves `excerpt_basis: None` so legacy anchor JSON serialization is byte-identical to pre-pointer records, while `verify_any()` still reports `AnchorBasis::Model` so `anchor_basis.model` counts accurately.
2. **PDF `meta.pages` filtering in `read_paper_lines`**: Filtered `pdf_page_lines` entries to pages overlapping the delivered `[start_line, end_line]` range so large PDFs do not crowd out the 7,000-char line budget; `next_line` is `delivered_end + 1` whenever `delivered_end < total_lines` and omitted at EOF.
3. **Drift check tightening (`WP3`)**: Guarded `earlier_defs(&exp.id)` in `check_tool_defs_drift` with `is_current_recorder_def(exp)` while retaining the dynamic `answer_node_ids` shim so both `modified_schema_rejected` and `earlier_map_tool_can_gain_only_its_optional_answer_index` pass cleanly.
4. **Prompt module wiring**: Declared `investigation_prompts.rs` as `#[path = "investigation_prompts.rs"] mod prompts;` inside `crates/desktop-server/src/investigation.rs` so `crates/desktop-server/src/lib.rs` required zero edits.
5. **Pending citations wording**: Conventions app limitation uses `"{N} items awaiting valid citations: {ids}"` (clipped to 500 chars) per section 2.2; UI `pendingCitationsLabel` uses singular `"1 citation pending"` and plural `"{N} citations pending"` in both `conventionStatus()` and the expanded notes block.
6. **Phase 1b worker gating**: Gated `compute_worker`, `lean_worker`, `lean_router`, `explorer_worker`, and `custom_subagents` to `None`/`Vec::new()` on `_paper_map_request`, `_paper_branch_request`, and `_paper_conventions_request` turns while keeping `literature_worker`, and keeping all workers enabled on `"check"` (`_paper_request`) turns.
7. **`PaperRoute::Cli` wording**: Omitted `"Other worker delegation is unavailable during paper reading."` on CLI route turns and ensured `delegate_read_paper` is never mentioned in CLI-route context.
8. **CLI route drift check audit**: Verified `crates/desktop-server/src/cli_turn.rs` and `crates/cli-route/src/turn.rs` perform no `def_hash` drift check across turns (`cli-route` 56/56 tests pass), so adding `read_paper_lines` and updating recorder schemas does not break resumed CLI sessions.
9. **Math-span test harness**: Because `crates/desktop-server/assets/math.js` checks `window.katex`, `investigation_map.test.mjs` sets and restores `globalThis.window = { katex }` inside its fake-DOM helper without altering `math.js` (owned by `latex`).

---

## Test commands and exact results

Environment: Linux (Ubuntu 24.04), `rustc 1.100.0`, `cargo 1.100.0`, `node v22.23.5`, `RE_SEARCH_PYTHON="$(command -v python3)"`.

| Command | Result |
|---|---|
| `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` | Exit 0 (all assets valid JS) |
| `node --test crates/desktop-server/tests/*.test.mjs` | **68 passed, 0 failed** |
| `RE_SEARCH_PYTHON="$(command -v python3)" cargo test --workspace --exclude cli --no-fail-fast` | **440 passed, 2 failed, 0 ignored** (both failures pre-existing in `export`; see proof below) |
| `RE_SEARCH_PYTHON="$(command -v python3)" cargo test -p cli --bins` | **28 passed, 0 failed, 0 ignored** |
| `RE_SEARCH_PYTHON="$(command -v python3)" cargo test -p cli --test api_route --test resume_route --test desktop_and_ext` | **8 passed, 0 failed, 0 ignored** (`api_route`: 2, `resume_route`: 4, `desktop_and_ext`: 2; `contained_router` and `init_abort` compile to 0 tests on Linux because `cfg(windows)`) |
| **Combined Rust workspace total** | **476 passed, 2 pre-existing failed, 0 ignored** |
| `cargo clippy --workspace --all-targets -- -D warnings` | Fails only on 3 pre-existing errors in untouched files (see below) |
| `cargo clippy --workspace --all-targets -- -D warnings -A clippy::manual_unwrap_or -A clippy::items_after_test_module -A unused_imports` | **Exit 0, 0 warnings across all 13 crates** |

---

## Pre-existing Linux failures (proven on base commit `91753b8`)

Checked out base commit `91753b86b4392814cbe0fe4dd213c9be1d15fc80` in a detached worktree (`/tmp/proof-base`, removed after verification) and ran the failing tests and clippy:

1. **2 `export` integration tests fail with `ModuleNotFoundError: No module named 'sympy'`** (system `python3` on this Linux VM does not have `sympy` installed):
   - `crates/export/tests/export_integrity.rs::handoff_checks_source_hashes_and_compares_seeded_values`
   - `crates/export/tests/fork_diff_export.rs::export_tex_html_and_reproducible_zip_and_replay_checks`
   - Identical failure and error output reproduced on base commit `91753b8`.
2. **3 pre-existing `cargo clippy --workspace --all-targets -- -D warnings` errors in untouched files** under `rustc 1.100.0` / Linux:
   - `crates/desktop-server/src/evidence.rs:737:40` (`clippy::manual_unwrap_or`, false positive on `Some(status @ (...))`) — reproduced on `91753b8`.
   - `crates/agent/src/guardrails.rs:335:1` (`clippy::items_after_test_module`) — reproduced on `91753b8`.
   - `crates/research-tools/tests/sandbox_and_fs.rs:6` (`unused_imports` for `is_symlink_or_junction` and `TEX_AUDIT`, which are used only inside `#[cfg(windows)]` tests on Linux).

---

## Could not compile or run on Linux

- All `#[cfg(windows)]` code and tests, including:
  - `desktop-app` WebView2 host (`crates/desktop-app`)
  - `worker` AppContainer sandboxing (`crates/worker`)
  - `crates/cli/tests/contained_router.rs` and `crates/cli/tests/init_abort.rs`
  - Windows-only `claude_cli_*` tests in `crates/desktop-server` and `#[cfg(windows)]` tests in `crates/research-tools/tests/sandbox_and_fs.rs` and `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs`
- Playwright / browser UI tests (out of scope on Linux per `RULES.md`).

---

## Pre-existing bugs / lint warnings noticed (not fixed)

- `crates/desktop-server/src/evidence.rs:737` — `clippy::manual_unwrap_or` fails under `rustc 1.100.0` with `-D warnings`.
- `crates/agent/src/guardrails.rs:335` — `clippy::items_after_test_module` fails under `rustc 1.100.0` with `-D warnings`.
- `crates/research-tools/tests/sandbox_and_fs.rs:6` — `is_symlink_or_junction` and `TEX_AUDIT` imports are not gated by `#[cfg(windows)]`, causing `unused_imports` on Linux.
- `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` — `BufRead` and `BufReader` imports are not gated by `#[cfg(windows)]`, causing `unused_imports` on Linux.
