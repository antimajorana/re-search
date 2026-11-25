# Audit report: `audit-honesty` (lens: `honesty`)

- **Base commit (`origin/gemini/work/merged`):** `351b1287c9a54a700b0e33111b3891f195afd6d1`
- **Instructions commit (`origin/gemini/runner`):** `ea6b5e84436ac91a3f5b660b3dcec754a31a06a1`
- **Branch:** `work/audit-honesty` -> `gemini/work/audit-honesty`
- **Production / existing-test edits:** None (read-only audit; only new `audit_honesty_*` test files and this report added).
- **New proof test files added (all `<= 600` lines, offline, zero clippy warnings):**
  1. `crates/research-tools/tests/audit_honesty_anchors.rs` (382 lines, 6 failing tests)
  2. `crates/desktop-server/tests/audit_honesty_cites.rs` (570 lines, 6 failing tests)
  3. `crates/desktop-server/tests/audit_honesty_ui.test.mjs` (207 lines, 3 failing tests)

---

## 1. Summary of findings (most severe first)

| # | Severity | Location | Summary | Proof |
|---|---|---|---|---|
| F1 | `critical` | `crates/desktop-server/src/investigation_cites.rs:65-90,96,112,132` | Subagent/worker and UI Canvas reads mark Orchestrator Answer-card `[L…]` citations as `read: true` even when the answer-writing model never received those lines | Proven (`audit_honesty_cites`) |
| F2 | `critical` | `crates/research-tools/src/investigation_anchor.rs:378-409` | `verify_pointer` label lookback (`s.min(final_start - 3)`) defeats Step 6 narrowing across up to 63 unquoted lines and contradicts `reports/split-1.md:35` | Proven (`audit_honesty_anchors`) |
| F3 | `critical` | `crates/desktop-server/src/investigation_cites.rs:112-130,148-153`; `crates/research-tools/src/fs_tools.rs:338-341` | Zero-char past-EOF `fs_read`, mid-line `fs_read` slices, and `line_truncated: true` `read_paper_lines` results mark unread text as `read: true` on Answer cards | Proven (`audit_honesty_cites`) |
| F4 | `major` | `crates/research-tools/src/investigation_anchor.rs:427-437,515-532`; `crates/research-tools/src/investigation_evidence.rs:370-372` | `window_around` (>3,200 bytes) can cut off part of `quote` from stored `anchor.excerpt` (breaking Guarantee #2) and slice off `label` so `resolve_evidence` fails with `label_not_found` | Proven (`audit_honesty_anchors`, 2 tests) |
| F5 | `major` | `crates/research-tools/src/investigation_conventions.rs:82-114,195-200,285-310`; `crates/desktop-server/src/investigation_scope.rs:229-231`; `crates/desktop-server/assets/investigation_conventions.js:129-131` | Later conventions batch can finalize `done: true, coverage: "paper"` and wipe `limitations` while `pending_items` remain, permanently locking out `"Continue conventions review"` | Proven (`audit_honesty_anchors` + `audit_honesty_ui`) |
| F6 | `major` | `crates/desktop-server/src/investigation_scope.rs:57-75,169-215`; `crates/desktop-server/src/paper_reader.rs:188-204,396-420` | `clip_to_selected_pages` does not clamp partially overlapping section ranges or `front_matter`, sending `investigation.scope` and `paper_reader` slices outside selected PDF pages | Proven (`audit_honesty_cites`) |
| F7 | `major` | `crates/research-tools/src/investigation_anchor.rs:378-409,443-449`; `crates/desktop-server/assets/investigation_map.js:196-199` | Narrowing/trimming followed by `label_extended` back to `[s, e]` leaves `cited_range == [start_line, end_line]`, rendering a false `"lines corrected by the app"` note | Proven (`audit_honesty_anchors` + `audit_honesty_ui`) |
| F8 | `minor` | `crates/desktop-server/src/paper_reader.rs:115-117`; `crates/desktop-server/src/agent_models.rs:342-346` | Request-body `paper_reader` override auto-enables readers and sends paper text to a provider never assigned in settings (violating Decision #4) | Proven (`audit_honesty_cites`) |
| F9 | `minor` | `crates/desktop-server/src/investigation_cites.rs:239,262-290`; `crates/desktop-server/src/paper_reader.rs:384-388,422-431` | Unverified on-disk reads in `source_total_lines` and `paper_reader::prepare_for_turn` skip `files_sha256` verification and override hash-verified `tool.result` metadata | Proven (`audit_honesty_cites`) |
| F10 | `minor` | `crates/desktop-server/src/investigation_cites.rs:43-50,242-258`; `crates/desktop-server/src/routes_tools.rs:163` | Answer-card `[Lstart–end]` citations spanning >513 lines are marked `read: true` (clickable `.line-cite` button) but fail with HTTP 400 on click | Proven (`audit_honesty_cites`) |
| F11 | `minor` | `crates/desktop-server/assets/investigation_map.js:42-66,68-79` | `foldSearchSpan` lacks `math_scan.js`'s currency `$` guard, pairing `$5` with later inline math `$` and widening `<mark>` across unquoted prose | Proven (`audit_honesty_ui`) |
| F12 | `minor` | `crates/research-tools/src/investigation_anchor.rs:18,41,350-355`; `crates/research-tools/src/investigation_evidence.rs:87` | `anchor_schema()` sets `quote.minLength: 12`, contradicting the 3-char whole-line rule in `verify_pointer`/`resolve_evidence` and `note_evidence_def()` (`minLength: 3`) | Proven (`audit_honesty_anchors`) |
| F13 | `nit` | `crates/agent/src/paper_read.rs:320-322` | `collect_evidence_rows` omits `source_id` and computes `pg` from primary PDF `page_lines` even on external-source evidence | Unproven (code inspection) |
| F14 | `nit` | `crates/research-tools/src/investigation_conventions.rs:131,294-302` | Appending pending-items note when `batch.limitations.len() == 12` produces 13 limitations (above the 12-item schema cap) and formats `"1 items"` | Unproven (code inspection) |
| F15 | `nit` | `crates/desktop-server/src/investigation_scope.rs:73,198` | `mapping_instructions_suffix` duplicates single-target line spans (`"Section II (lines 51–110) (lines 51-110)"`) | Unproven (code inspection) |

---

## 2. Detailed findings

### F1 (`critical`) — Subagent/worker and UI Canvas reads mark Answer-card `[L…]` citations as `read: true` when the answer-writing Orchestrator never received those lines

- **Location:** `crates/desktop-server/src/investigation_cites.rs:65-90, 96, 112, 132` (with Question-turn delegation active in `crates/desktop-server/src/turn_runner.rs:294-296, 316-372`).
- **What is wrong:** `README.md` Decision #6 (*"Clickable citations (phase 3): only for lines the model actually received"*), Design 02 §2.10 (D9), and `INVESTIGATION_SYSTEM_PROMPT` Rule 6 (*"use workers only to locate candidate passages, then read the cited lines yourself"*) require an Answer-card `[Lstart–end]` citation to become clickable (`read: true`) only if the answer-writing model itself received those lines. On Question turns (`_paper_question_request`), `paper_reading_turn` (`turn_runner.rs:294-296`) is `false`, so `delegate_literature`, `delegate_explore`, and `delegate_subagent` are enabled and invoke `router.try_call("worker:...", ...)` on the shared session `Router` (`crates/agent/src/delegate.rs:361`, `explore.rs:183`), appending `tool.request` and `tool.result` to the session ledger. Concurrent UI Canvas actions (`caller: "user:canvas"`) also write to the same session ledger. `collect_received_ranges` never inspects `ev.payload["caller"]` on `tool.request` (and even accepts `tool.is_none()` at lines 96, 112, and 132), crediting worker or UI reads to the Orchestrator's answer even when the worker returned only a vague summary without line 20's text.
- **Concrete input / sequence:** Start an Investigation on a 30-line paper and post a Question turn. Have the Orchestrator call `delegate_literature`, have `literature_worker` call `fs_read` on `paper.txt` (lines 1–30) and return `"Relevant derivation is around line 20."` without the line text, and have the Orchestrator answer `"By [L20], the derivation holds."` without ever calling `read_paper_lines`, `fs_read`, or `search_paper_passages`. `answer.cites` records `[{"start": 20, "end": 20, "read": true}]`.
- **Proof (`crates/desktop-server/tests/audit_honesty_cites.rs`):**
  ```text
  thread 'subagent_read_must_not_mark_orchestrator_answer_citation_as_read' panicked at crates/desktop-server/tests/audit_honesty_cites.rs:222:5:
  assertion `left == right` failed: orchestrator never received line 20 (only literature_worker ran fs_read), yet cites=[{"end":20,"read":true,"start":20}]
    left: Bool(true)
   right: Bool(false)
  ```
- **Proposed fix:** In `collect_received_ranges`, record `(tool, caller)` from `tool.request`, require `Some((tool, caller))` (dropping `tool.is_none()` fallbacks), and restrict credited reads to main-agent callers (`caller.starts_with("agent:") || caller.starts_with("claude-code-cli:") || caller.starts_with("antigravity:")`).

---

### F2 (`critical`) — `verify_pointer` label lookback (`s.min(final_start - 3)`) defeats Step 6 narrowing across up to 63 unquoted lines

- **Location:** `crates/research-tools/src/investigation_anchor.rs:378-409`.
- **What is wrong:** Design 01 §2.1 Step 6 narrows non-relocated matches to `[max(s, a - 2), min(e, b + 2)]` specifically so *"a 64-line citation therefore can never be grounded by a 16-char quote."* However, Step 8 (`line 396`) computes `above_lo = s.min(final_start.saturating_sub(LABEL_ABOVE_RADIUS)).max(1)`. Because it takes `s.min(...)` instead of `final_start.saturating_sub(LABEL_ABOVE_RADIUS)`, whenever `s` is far above `final_start` (up to 63 lines above), `verify_pointer` searches the entire narrowed-away prefix `s..=(final_start - 1)` for `label` and resets `final_start = label_line`. A 16-char quote on line 61 paired with `label: "Lemma 1."` on line 1 grounds all 61 lines (`1..=61`), copying 59 unquoted lines into the verified `anchor.excerpt`. Note also that `reports/split-1.md:35` inaccurately stated that label lookback *"extends up by up to 2 lines when a theorem/definition/lemma/equation/etc. label sits 1–2 lines above."*
- **Concrete input / sequence:** Ingest a 70-line paper with `"Lemma 1. Setup."` on line 1, 59 unquoted filler lines on lines 2–60, and `"Energy is strictly conserved here."` on line 61. Call `record_paper_map` with `{source_id, start_line: 1, end_line: 61, kind: "passage", label: "Lemma 1.", quote: "Energy is strictly conserved here."}`. `verify_pointer` narrows to `59..=61` and then extends `final_start` back to `1`, saving `start_line: 1, end_line: 61` with all 61 lines in `excerpt`.
- **Proof (`crates/research-tools/tests/audit_honesty_anchors.rs`):**
  ```text
  thread 'label_lookback_must_not_defeat_narrowing_across_60_unquoted_lines' panicked at crates/research-tools/tests/audit_honesty_anchors.rs:110:9:
  16+ char quote on line 61 plus label on line 1 grounded 61 lines (1..=61), absorbing 59 unquoted lines into verified excerpt
  ```
- **Proposed fix:** Replace `s.min(final_start.saturating_sub(LABEL_ABOVE_RADIUS)).max(1)` at `investigation_anchor.rs:396` with `final_start.saturating_sub(LABEL_ABOVE_RADIUS).max(1)` so label extension never looks more than `LABEL_ABOVE_RADIUS` (3) lines above `final_start`.

---

### F3 (`critical`) — Zero-character past-EOF `fs_read`, mid-line `fs_read` slices, and `line_truncated` `read_paper_lines` mark unread text as `read: true` on the Answer card

- **Location:** `crates/desktop-server/src/investigation_cites.rs:112-130, 148-153` and `crates/research-tools/src/fs_tools.rs:317-342`.
- **What is wrong:**
  1. When `fs_read` is called with `offset >= total_chars` on a file without a trailing newline, `slice_utf8_chars` (`fs_tools.rs:338-341`) returns `("", total, total, line_no, line_no)`, producing `returned_chars: 0, start_line: total_lines, end_line: total_lines`. `collect_received_ranges` (`investigation_cites.rs:148-152`) ignores `returned_chars == 0` and credits `total_lines..=total_lines` as read when zero characters were delivered.
  2. When a single long line (such as an unwrapped `.tex` paragraph) exceeds `MAIN_BUDGET_CHARS` (14,000 chars), `read_paper_lines` (`paper_lines.rs:184-188`) truncates the line mid-sentence and sets `a_meta.line_truncated = true`. `collect_received_ranges` (`investigation_cites.rs:124-128`) ignores `line_truncated == true` and marks `[L1]` as `read: true` even when the cited claim sits in the unread tail of line 1. (Similarly, `fs_read` character-offset slices that start or stop mid-line report whole boundary line numbers, and `merge_ranges` merges adjacent line numbers across unread mid-line gaps.)
- **Concrete input / sequence:** Ingest a 3-line paper (no trailing `\n`) whose line 1 is ~16,500 chars ending with `"TAIL_FACT_NEVER_SENT"` and line 3 is `"Line three final conclusion never read"`. On a Question turn, call `read_paper_lines(start_line: 1, max_lines: 1)` (`a_meta.line_truncated == true`) and `fs_read(root: "sources", path: "long.txt", offset: 999999, max_chars: 100)` (`returned_chars == 0, start_line: 3, end_line: 3`). Answer citing `[L1]` and `[L3]`: both are marked `read: true`.
- **Proof (`crates/desktop-server/tests/audit_honesty_cites.rs`):**
  ```text
  thread 'zero_char_past_eof_fs_read_and_truncated_read_paper_lines_must_not_mark_cites_read' panicked at crates/desktop-server/tests/audit_honesty_cites.rs:285:5:
  assertion `left == right` failed: line_truncated=true on L1 and 0-char past-EOF fs_read on L3 were marked read:true: [Object {"end": Number(1), "read": Bool(true), "start": Number(1)}, Object {"end": Number(3), "read": Bool(true), "start": Number(3)}]
    left: (Some(true), Some(true))
   right: (Some(false), Some(false))
  ```
- **Proposed fix:** In `collect_received_ranges`, skip `read_paper_lines` results with `meta["line_truncated"] == true`, skip `fs_read` results with `returned_chars == 0` (or empty `content`), and exclude partially read boundary lines when `offset > 0` or `truncated == true` in `fs_read`.

---

### F4 (`major`) — `verify_pointer`'s `window_around` (>3,200 bytes) can drop part of `quote` from `anchor.excerpt` and slice off `label`, breaking Guarantee #2 and `resolve_evidence`

- **Location:** `crates/research-tools/src/investigation_anchor.rs:427-437, 515-532` and `crates/research-tools/src/investigation_evidence.rs:370-372`.
- **What is wrong:**
  1. `fold_with_map` collapses arbitrary whitespace runs into a single `' '`. When a matched `quote` spans more than `MAX_EXCERPT_BYTES` (3,200 bytes) in the raw source (for example across padded table columns or blank space), `window_around` (`lines 520-531`) returns a 3,200-byte window `[w0, w1]` with `w1 < q_end`. `verify_pointer` succeeds and saves an `anchor.excerpt` that **omits the end of `quote`**, violating Design 01 §2.1 Step 10 (*"a 3,200-byte UTF-8 window centred on the matched quote"*) and §2.7 Guarantee #2 (*"Every stored `excerpt` comes from the file"* containing the verified quote). Downstream, UI `foldSearchSpan` returns `null`, and any `note_evidence` item saved this way can never be cited via `resolve_evidence` (`!folded_derived.contains(&folded_quote)` -> `evidence_quote_mismatch`).
  2. When `full_lines.len() > 3200` has a valid `label` (such as `"Theorem 1"`) near the top of the cited lines and `quote` >3,200 bytes later, `verify_pointer` validates `label` against `full_lines` (`line 393`) and then windows `excerpt` around `quote`, slicing `"Theorem 1"` out of `rec.excerpt`. `note_evidence` succeeds and exposes `"lab": "Theorem 1"` to the Orchestrator (`paper_read.rs:325`), but when the Orchestrator echoes `{"evidence": "ev-N", "label": "Theorem 1", "quote": "..."}`, `resolve_evidence` (`investigation_evidence.rs:370`) checks `derived.contains(label)` instead of `full_lines.contains(label)` and rejects with `label_not_found`.
- **Concrete input / sequence:**
  1. Line 1 has `"Alpha boundary condition"` + 3,400 spaces, line 2 has `"holds for every mode."`; cite `1..=2` with `quote: "Alpha boundary condition holds for every mode."` -> stored `excerpt` contains `"Alpha boundary condition"` + spaces and drops `"holds for every mode."`.
  2. Line 1 has `"Theorem 1 ( Spectral Gap )."`, lines 2–21 have ~3,600 bytes of hypotheses, line 22 has `"Therefore the spectral gap remains strictly positive in the limit."`; `note_evidence` records `ev-7` with `label: "Theorem 1"`, then `record_paper_map` cites `ev-7` with `label: "Theorem 1"` -> fails with `label_not_found`.
- **Proof (`crates/research-tools/tests/audit_honesty_anchors.rs`):**
  ```text
  thread 'excerpt_window_must_contain_the_full_matched_quote_or_reject' panicked at crates/research-tools/tests/audit_honesty_anchors.rs:209:9:
  stored excerpt window dropped part of the verbatim quote: got `Alpha boundary condition`

  thread 'evidence_anchor_with_reader_label_before_excerpt_window_must_not_fail_label_not_found' panicked at crates/research-tools/tests/audit_honesty_anchors.rs:281:5:
  resolve_evidence rejected the reader's own verified label `Theorem 1` because excerpt_window sliced it off: Err("{\"error\":\"anchors_failed\",\"faults\":[{\"code\":\"label_not_found\",\"message\":\"Label `Theorem 1` not found in evidence `ev-7`.\",\"owner\":\"nodes[thm1].anchors[0]\"}],\"more_faults\":0}")
  ```
- **Proposed fix:** In `verify_pointer`, reject with `quote_span_too_wide` when `q_end - q_start > MAX_EXCERPT_BYTES` so every stored `excerpt` contains the full matched `quote`; in `resolve_evidence` (`investigation_evidence.rs:370`), check `full_lines.contains(label)` (matching `verify_pointer:393`) instead of `derived.contains(label)`.

---

### F5 (`major`) — `record_paper_conventions` lets a later batch finalize `done: true, coverage: "paper"` while stranding `pending_items`, wiping prior `limitations`, and locking out UI resume

- **Location:** `crates/research-tools/src/investigation_conventions.rs:82-114, 195-200, 285-310`, `crates/desktop-server/src/investigation_scope.rs:229-231`, and `crates/desktop-server/assets/investigation_conventions.js:45, 129-131`.
- **What is wrong:** When Batch 1 includes a valid item (`good-metric`) and an item whose pointer quote fails (`bad-sign`), Batch 1 forces `done: false, coverage: "partial"` and appends `"1 items awaiting valid citations: bad-sign"` to `limitations` (`README.md` Decision #2; Design 01 §2.2). In Batch 2, however, the model can submit `batch_index: 2, done: true, coverage: "paper", items: [], limitations: []`:
  - `payload()` (`line 195`) allows `items: []` with `limitations: []` because `!prior_ids.is_empty()`, and skips line 274 because `had_items == false` and `rejected_items.is_empty()`.
  - `paper_conventions()` (`lines 109-113`) clones `latest.payload`, returning `done: true`, `coverage: "paper"`, and `limitations: []` (wiping Batch 1's model-reported AND app-generated limitations!) while simultaneously computing `pending_items: ["bad-sign"]`.
  - Both `conventions_resume_allows` (`investigation_scope.rs:230`) and `investigation_conventions.js:130` check `done == true` without checking `pending_items`, hiding/blocking `"Continue conventions review"` while the UI header permanently reads `"Conventions · 1 entry · 1 citation pending"`.
- **Concrete input / sequence:** Submit Batch 1 with `good-metric` (valid quote) + `bad-sign` (non-existent quote) + `limitations: ["Appendix B was not inspected."]`; then submit Batch 2 with `done: true, coverage: "paper", items: [], limitations: []`.
- **Proof (`crates/research-tools/tests/audit_honesty_anchors.rs` & `crates/desktop-server/tests/audit_honesty_ui.test.mjs`):**
  ```text
  thread 'conventions_later_batch_must_not_finalize_done_true_or_wipe_limitations_while_pending_items_remain' panicked at crates/research-tools/tests/audit_honesty_anchors.rs:356:9:
  catalog finalized with done=true and coverage="paper" while pending_items=[String("bad-sign")] remain unresolved

  not ok 3 - audit_honesty: convention catalog with pending_items must not lock out Continue button when done is true
    error: |-
      catalog shows "Conventions · 1 entry · 1 citation pending" yet hides the Continue button so pending citations can never be resolved
      true !== false
  ```
- **Proposed fix:** Compute unresolved `pending_items` across prior batches inside `investigation_conventions::payload`: while unresolved `pending_items` remain (unless explicitly superseded/cleared), force `done = false, coverage = "partial"` and carry forward the pending-items limitation; merge distinct `limitations` across all batches in `paper_conventions()`; and allow resuming whenever `pending_items` is non-empty in both `conventions_resume_allows` and `investigation_conventions.js:130`.

---

### F6 (`major`) — `clip_to_selected_pages` fails to clip partially overlapping section line ranges and `front_matter`, sending `investigation.scope` and `paper_reader` outside selected PDF pages

- **Location:** `crates/desktop-server/src/investigation_scope.rs:57-75, 169-215` and `crates/desktop-server/src/paper_reader.rs:188-204, 396-420`.
- **What is wrong:** When a user pins a PDF with `page_start..=page_end` (e.g., pages 2–3, lines 21–80), `SelectedPages::check_range` rejects any primary-paper anchor outside lines 21–80. Yet `clip_to_selected_pages` only filters targets by `a <= last_page && b >= first_page` without clamping `start_line`, `end_line`, `page_start`, `page_end`, `ranges`, or `front_matter` to the line bounds of `first_page..=last_page`. Furthermore, `paper_reader::plan_slices` (`line 188`: `if scope_lines.is_none()`) ignores `selected_pages` whenever `scope_lines.is_some()`. Consequently, asking `"Explain section II"` when Section II spans pages 3–4 (lines 51–110) records `ranges: [[51, 110]]` and `front_matter: [1, 20]` (page 1), assigns `paper_reader` slices up to line 110 (page 4, where `note_evidence` fails every anchor with `outside_selected_pages`), and instructs the Orchestrator to summarize `front matter lines 1-20` (also rejected by `SelectedPages`).
- **Concrete input / sequence:** Ingest a 4-page PDF (`page 1: 1..=20`, `page 2: 21..=50`, `page 3: 51..=80`, `page 4: 81..=110`) where `"II. MAIN DERIVATION"` spans lines 51–110. Start Investigation with `page_start: 2, page_end: 3` and `question: "Explain section II"`.
- **Proof (`crates/desktop-server/tests/audit_honesty_cites.rs`):**
  ```text
  thread 'selected_pdf_pages_must_clip_resolved_section_scope_and_reader_slices' panicked at crates/desktop-server/tests/audit_honesty_cites.rs:452:9:
  user selected PDF pages 2–3 (lines 21–80), yet investigation.scope resolved unclipped range 51..=110 (scope={"front_matter":[1,20],"label":"Section II (lines 51–110)","outline_style":"aps","outline_weak":false,"question":"Explain section II","ranges":[[51,110]],"request_seq":4,"section_count":4,"source_id":"pdf-scoped-1","status":"resolved","targets":[{"end_line":110,"kind":"section","label":"Section II","page_end":4,"page_start":3,"start_line":51,"title":"MAIN DERIVATION"}]})
  ```
- **Proposed fix:** Pass `&page_lines` into `clip_to_selected_pages`, clamp each kept target's `[start_line, end_line]` (and `page_start`/`page_end`) plus `front_matter` to the `[min_line, max_line]` of `first_page..=last_page` (dropping `front_matter` if disjoint), and intersect `scope_lines` with `selected_pages` line bounds in `paper_reader::plan_slices`.

---

### F7 (`major`) — False `"lines corrected by the app"` (`cited_range == [start_line, end_line]`) when narrowing/trimming + `label_extended` restores the exact cited range

- **Location:** `crates/research-tools/src/investigation_anchor.rs:378-409, 443-449` and `crates/desktop-server/assets/investigation_map.js:196-199`.
- **What is wrong:** When a model accurately cites `start_line: 10, end_line: 13` where line 10 holds `"Eq. (1)"`, lines 11–12 are blank, and line 13 holds the quoted equation, Step 6 narrows to `11..=13` (setting `cited_range = Some([10, 13])`), Step 7 trims blanks to `13..=13`, and Step 8 extends `final_start` back to line 10 for `"Eq. (1)"`. Because `verify_pointer` never clears `cited_range` when `[final_start, final_end] == [s, e]`, it stores `start_line: 10, end_line: 13, cited_range: [10, 13]`, emits an `AnchorCorrection` from `[10, 13]` to `[10, 13]`, and `investigation_map.js` renders `"lines 10–13"` together with `"lines corrected by the app (cited 10–13)"`.
- **Concrete input / sequence:** Paper with line 10 `"Eq. (1)"`, blank lines 11–12, line 13 `"H = p^2 / (2m) + V(x) is the system Hamiltonian."`. Call `record_paper_map` with `start_line: 10, end_line: 13, label: "Eq. (1)", quote: "H = p^2 / (2m) + V(x) is the system Hamiltonian."`.
- **Proof (`crates/research-tools/tests/audit_honesty_anchors.rs` & `crates/desktop-server/tests/audit_honesty_ui.test.mjs`):**
  ```text
  thread 'label_extension_restoring_original_range_must_not_emit_false_cited_range_correction' panicked at crates/research-tools/tests/audit_honesty_anchors.rs:163:5:
  final range 10..=13 equals cited 10..=13, yet cited_range was set to Some(Array [Number(10), Number(13)]) (basis: Some(Array [String("narrowed"), String("trimmed"), String("label_extended")]), report: {"corrected":[{"basis":["narrowed","trimmed","label_extended"],"from":[10,13],"lines":[10,13],"owner":"nodes[eq1].anchors[0]"}],"dropped":[]})

  not ok 2 - audit_honesty: renderAnchors must not claim lines were corrected when cited_range equals final range
    error: |-
      unchanged range 10–13 was falsely labelled corrected: lines corrected by the app (cited 10–13)
      1 !== 0
  ```
- **Proposed fix:** In `verify_pointer` (before line 438), if `[final_start, final_end] == [s, e]`, clear `cited_range = None` and `range_basis.clear()`; also guard `investigation_map.js:196` with `(a !== anchor.start_line || b !== anchor.end_line)`.

---

### F8 (`minor`) — Request-body `paper_reader` override auto-enables readers and sends paper text to a provider never assigned in settings

- **Location:** `crates/desktop-server/src/paper_reader.rs:115-117` and `crates/desktop-server/src/agent_models.rs:342-346`.
- **What is wrong:** `README.md` Decision #4 and Design 01 §5 #4 specify: *"Reader model when none is assigned: the primary provider; another provider only if assigned in settings (that sends paper text there)."* Both `has_dedicated_reader_model` and `resolve_paper_reader_transport` honor `req.get("paper_reader")` before `ui.agent_models.get("paper_reader")`, allowing a request payload to auto-enable `delegate_read_paper` (`should_enable_readers == true` even with `ui.paper_readers_enabled == None` and empty `ui.agent_models`) and route paper text to a different provider (`routing_reason: "request_override"`).
- **Concrete input / sequence:** Leave `ui.agent_models` empty and primary provider set to Anthropic; send `POST /api/sessions/{id}/turns` with `"paper_reader": {"provider": "openai", "model": "gpt-5.4-mini"}` during a mapping turn where the Orchestrator calls `delegate_read_paper`.
- **Proof (`crates/desktop-server/tests/audit_honesty_cites.rs`):**
  ```text
  thread 'request_body_paper_reader_override_must_not_send_paper_to_unassigned_provider' panicked at crates/desktop-server/tests/audit_honesty_cites.rs:565:5:
  paper_reader was NOT assigned in settings, yet request-body override auto-enabled readers and sent 1 call(s) to OpenAI
  ```
- **Proposed fix:** Remove `req.get("paper_reader")` from `has_dedicated_reader_model` and `resolve_paper_reader_transport` (or restrict cross-provider `paper_reader` routing strictly to `ui.agent_models.get("paper_reader")` persisted in settings).

---

### F9 (`minor`) — Unverified on-disk reads in `investigation_cites::source_total_lines` and `paper_reader::prepare_for_turn` skip `files_sha256` verification

- **Location:** `crates/desktop-server/src/investigation_cites.rs:239, 262-290` and `crates/desktop-server/src/paper_reader.rs:384-388, 422-431`.
- **What is wrong:** Unlike `SourceIndex::load`, `read_paper_lines`, and `compute_scope`, `source_total_lines` reads `primary_path` directly from disk without checking `source.ingested.payload["files_sha256"]`, and `answer_cites` (`line 239`: `total_lines.or(observed_total)`) gives that unverified disk count priority over `observed_total` extracted from hash-verified `read_paper_lines`/`fs_read` `tool.result`s. Similarly, `paper_reader::prepare_for_turn` reads `primary_path` via `String::from_utf8_lossy` without checking `files_sha256` and reads `receipt.payload["pdf_page_lines"]` directly instead of `research_tools::pdf::verified_page_lines`.
- **Concrete input / sequence:** Ingest a 20-line paper, let the Orchestrator read lines 1–20 via `read_paper_lines` (recording hash-verified `total_lines: 20`), truncate the on-disk file to 2 lines before `finish_question` runs, and have the Orchestrator answer citing `[L15]`. `source_total_lines` returns `Some(2)` and silently drops `[L15]` from `answer.cites`.
- **Proof (`crates/desktop-server/tests/audit_honesty_cites.rs`):**
  ```text
  thread 'unverified_disk_mutation_must_not_override_verified_total_lines_in_answer_cites' panicked at crates/desktop-server/tests/audit_honesty_cites.rs:504:5:
  assertion `left == right` failed: unverified on-disk truncation overrode hash-verified tool.result total_lines=20 and dropped [L15]: cites=[]
    left: Array []
   right: Array [Object {"end": Number(15), "read": Bool(true), "start": Number(15)}]
  ```
- **Proposed fix:** Verify `ledger::sha256_hex(&bytes) == expected_sha` against the `source.ingested` receipt in both `source_total_lines` (returning `None` on mismatch so `answer_cites` uses `observed_total`) and `paper_reader::prepare_for_turn` (and use `verified_page_lines` there).

---

### F10 (`minor`) — Checked Answer-card `[Lstart–end]` citations spanning >513 lines are marked `read: true` but fail with HTTP 400 on click

- **Location:** `crates/desktop-server/src/investigation_cites.rs:43-50, 242-258` vs `crates/desktop-server/src/routes_tools.rs:163`.
- **What is wrong:** `collect_received_ranges` merges adjacent `read_paper_lines` calls (e.g., `1..=350` + `351..=600` -> `1..=600`). When the answer cites `[L1–550]`, `answer_cites` marks it `read: true`, so `paper_cites.js` renders a clickable `.line-cite` button; clicking it calls `GET /api/sessions/{id}/files?root=sources&path=...&start_line=1&end_line=550`, which `routes_tools.rs:163` (`last - first > 512`) rejects with HTTP 400 `"Select a passage of up to 513 lines."`.
- **Concrete input / sequence:** Ingest a 600-line paper, have the Orchestrator read `1..=350` and `351..=600` via `read_paper_lines`, and answer `"Across [L1–550], the derivation is self-contained."`, then open the cited span via the source-window endpoint.
- **Proof (`crates/desktop-server/tests/audit_honesty_cites.rs`):**
  ```text
  thread 'checked_cite_exceeding_513_lines_must_either_open_or_not_be_marked_read' panicked at crates/desktop-server/tests/audit_honesty_cites.rs:347:9:
  assertion `left == right` failed: [L1–550] was marked read:true (clickable .line-cite button), but clicking it fails with HTTP 400: {"error":"Select a passage of up to 513 lines."}
    left: 400
   right: 200
  ```
- **Proposed fix:** Either require `end - start <= 512` for `read: true` in `answer_cites` (and `paper_cites.js`) so >513-line ranges stay plain text, or clamp `end_line` to `start_line + 512` in `openCite` (`crates/desktop-server/assets/investigation.js:47`).

---

### F11 (`minor`) — `investigation_map.js` `foldSearchSpan` pairs currency `$` with later math `$`, widening `<mark>` across unquoted prose

- **Location:** `crates/desktop-server/assets/investigation_map.js:42-66, 68-79`.
- **What is wrong:** `mathSpans` in `investigation_map.js` reimplements inline `$...$` scanning without `math_scan.js`'s currency guard. On an excerpt containing currency `$5` before inline math `$\Delta > 0$`, `mathSpans` treats `$5` as an inline-math opener, ignores `$\Delta` (preceded by a space), and pairs `$5` with `0$`, creating a bogus math span `'$5 per sample, the gap $\\Delta > 0$'`. `foldSearchSpan` then widens `<mark>` backwards to `$5`, highlighting unquoted prose as part of the model's verbatim quote.
- **Concrete input / sequence:** Call `foldSearchSpan('At $5 per sample, the gap $\\Delta > 0$ persists.', 'the gap $\\Delta > 0$ persists')`.
- **Proof (`crates/desktop-server/tests/audit_honesty_ui.test.mjs`):**
  ```text
  not ok 1 - audit_honesty: foldSearchSpan must not widen <mark> across earlier currency $ into unquoted prose
    error: |-
      currency $5 before the quote must not be absorbed into the highlighted quote span
      + actual - expected

      + '$5 per sample, the gap $\\Delta > 0$ persists'
      - 'the gap $\\Delta > 0$ persists'
  ```
- **Proposed fix:** Import and use `math_scan.js`'s tokenizer (or apply its currency `$` rule) inside `investigation_map.js` instead of maintaining a separate `mathSpans` scanner.

---

### F12 (`minor`) — `anchor_schema()` sets `quote.minLength: 12`, contradicting the 3-char whole-line rule in `verify_pointer`, `resolve_evidence`, and `note_evidence_def()`

- **Location:** `crates/research-tools/src/investigation_anchor.rs:18, 41, 350-355` vs `crates/research-tools/src/investigation_evidence.rs:87`.
- **What is wrong:** Design 01 §2.1 contains an internal mismatch between its schema table (`line 41`: `12–300 chars`) and Step 2 (`line 66`: `MIN_WHOLE_LINE_NON_SPACE = 3` for exact whole-line matches). `note_evidence_def()` uses `"minLength": 3`, allowing a reader to record a short whole-line equation such as `"E = mc^2"` (8 chars) as `ev-N`, whereas `anchor_schema()` on all four `record_paper_*` tools advertises `"minLength": 12`, causing strict schema-validating providers to block the Orchestrator from supplying that same 8-char `quote` on `{"evidence": "ev-N", "quote": "E = mc^2"}` or on direct whole-line pointer citations.
- **Concrete input / sequence:** Compare `note_evidence_def().input_schema["properties"]["items"]["items"]["properties"]["quote"]["minLength"]` (`3`) against `record_paper_map` `input_schema["properties"]["nodes"]["items"]["properties"]["anchors"]["items"]["properties"]["quote"]["minLength"]` (`12`).
- **Proof (`crates/research-tools/tests/audit_honesty_anchors.rs`):**
  ```text
  thread 'recorder_anchor_schema_min_length_must_not_contradict_3_char_whole_line_quote_rule' panicked at crates/research-tools/tests/audit_honesty_anchors.rs:377:5:
  record_paper_* anchor_schema sets quote.minLength=12, blocking 3–11 char whole-line quotes (such as `E = mc^2`) that verify_pointer, resolve_evidence, and note_evidence_def (minLength=3) allow
  ```
- **Proposed fix:** Set `quote.minLength` to `3` in `anchor_schema()` and append the current recorder `ToolDef` hashes to `crates/research-tools/src/investigation_legacy_defs.rs::earlier_defs` so sessions recorded before the schema update still pass resume drift checks.

---

### Unproven / code-inspected `nit` findings

- **F13 (`nit`, unproven) — `crates/agent/src/paper_read.rs:320-322`:** `collect_evidence_rows` omits `rec.source_id` from the compact `{e, l, pg, k, r, lab, q, g}` object returned by `delegate_read_paper` and computes `pg` from the primary PDF's `page_lines` without checking `rec.source_id == primary_source_id`. (`resolve_evidence` at `investigation_evidence.rs:334` still fails closed with `evidence_source_mismatch` if misattributed.)
- **F14 (`nit`, unproven) — `crates/research-tools/src/investigation_conventions.rs:131, 294-302`:** Input validation caps `batch.limitations.len() <= 12`, but when `rejected_items` is non-empty, line 302 pushes an extra app note (`"{} items awaiting valid citations: ..."`), producing 13 items (and plural `"1 items"` for 1 rejected item).
- **F15 (`nit`, unproven) — `crates/desktop-server/src/investigation_scope.rs:73, 198`:** `build_scope_label` and `clip_to_selected_pages` already format single-target labels as `"Section II (lines 51–110)"`, and `mapping_instructions_suffix` (`line 73`) / `question_instructions_suffix` (`line 92`) append `({range_str})` a second time, producing `"Question scope: Section II (lines 51–110) (lines 51-110)."`.

---

## 3. What was checked and found correct

1. **Core security, policy, approval, sandbox, and ledger invariants (`git diff origin/main...HEAD`):**
   - Zero diff vs `origin/main` in `Cargo.toml`, `Cargo.lock`, all `crates/*/Cargo.toml`, `crates/policy`, `crates/ledger`, `crates/tools`, `crates/worker`, `crates/mcp-server`, `crates/cli`, `crates/cli-route`, `crates/chatgpt-auth`, `crates/export`, and `crates/desktop-app`.
   - `policy` (`deny > ask > allow`, `execute-code` / `write-workspace` / `network-read` clamps), `UiApprover` / `ConsoleApprover` (zero auto-approve paths), `WorkspaceSandbox` / `AppContainerSandbox`, SQLite append-only triggers, SHA-256 hash chain (`read_verified_session`), and `policy`/`ledger` crate dependency isolation (no `tokio`, `rmcp`, or `windows-sys`) are completely intact.
2. **Design 01 §2.7 honesty mechanisms that hold:**
   - **Legacy `excerpt` path:** `SourceIndex::verify` (`investigation_anchor.rs:265-307`) preserves exact byte-for-byte substring checking against hash-verified source files, and all four recorders abort the entire call immediately when `fault.legacy == true`.
   - **Mutual exclusion & app-only field rejection:** `{excerpt, quote}`, `{excerpt, evidence}`, and `{evidence, start_line/end_line/path}` (when conflicting) fail closed; `reject_app_only_fields` (`investigation_anchor.rs:48-61`) recursively rejects `excerpt_basis`, `cited_range`, `range_basis`, and `excerpt_window` on raw JSON before serde deserialization.
   - **Relocation, ambiguity, hyphen/NFKC strictness, 64-line cap, and assembled-TeX refusal:** Relocation (`±12` lines) requires >=16 non-space chars and unique match (`0` -> `quote_not_found`, `>=2` -> `quote_ambiguous` with candidate lines); hyphens and Unicode compatibility characters are never folded; final ranges >=64 lines fail with `range_too_long`; `SourceIndex::load` and `read_paper_lines` refuse `assembled_path` TeX flattens.
   - **Reader isolation & `resolve_evidence`:** `note_evidence` enforces `caller.starts_with("worker:paper_reader:")`, is absent from the main `Registry`, and retries `append_at_head` up to 3 times on `SessionChanged`. `EvidenceRecord` uses `#[serde(deny_unknown_fields)]`; `resolve_evidence` verifies `marker_seq`, `source_id`, `file_sha256`, `source_seq`, `SelectedPages`, source re-derivation against `rec.excerpt`, and the recorder's own `quote`. `delegate_read_paper` enforces the 3-tool whitelist (`search_paper_passages`, `read_paper_lines`, `note_evidence`), restricts the final synthesis turn to `note_evidence` only (`paper_read.rs:230-241`), aborts on `router.try_call` `Err`, and persists output via `delegate.end` `raw_blob` for resume replay.
   - **Sanitized live poll & honesty naming:** `poll_event_payload` (`routes_session.rs:584-617`) strips `excerpt` and `quote`; `anchors_verified` is never displayed in any UI asset (`rg anchors_verified crates/desktop-server/assets/` = 0 hits).
3. **Audit of all existing test changes (`git diff origin/main...HEAD -- '*test*' 'crates/*/tests'` + inline `#[test]`/`#[ignore]`):**
   - Zero `#[ignore]` attributes added; zero existing Rust or Node tests deleted, skipped, or weakened.
   - Moved unit tests (`earlier_map_tool_can_gain_only_its_optional_answer_index` -> `resume_drift_tests.rs`; `long_model_context_keeps_source_locations_and_reports_omissions` -> `investigation_tests.rs`) are verbatim.
   - In `crates/desktop-server/tests/investigation_browser.py` (the only existing test file with net deleted lines), leaving setup question blank in the main journey per Design 02 P4 moved `.question-match` coverage to `check_question_first_investigation` (note minor coverage gap at `investigation_browser.py:285,395`: `:395` checks `"Selected for your question"` text on `#investigation-map .investigation-index` without re-asserting `.question-match` + `lines[1]`, and `:285` asserts `to_have_attribute("open", "")` because `openFirst: true` now opens the first anchor by default).

---

## 4. Decisions taken unattended

1. **Test file split (`<= 600` lines):** Split Rust proof tests into `crates/research-tools/tests/audit_honesty_anchors.rs` (anchor/evidence/conventions/schema unit-level proofs) and `crates/desktop-server/tests/audit_honesty_cites.rs` (HTTP/session/turn-runner/cites/scope/provider-routing proofs), plus `crates/desktop-server/tests/audit_honesty_ui.test.mjs` for JS DOM/scanner proofs, keeping every file under 600 lines.
2. **Fixture placeholders:** Used plain fixture strings (`"fixture-ant"`, `"fixture-oai"`) in `audit_honesty_cites.rs` so no provider-key-prefixed dummy strings appear in audit tests or logs.
3. **`RE_SEARCH_PYTHON` verification:** Verified Python 3.12 (`sympy 1.14.0`, `numpy`, `mpmath`) was installed and ran the full `export` replay/integrity suite with `RE_SEARCH_PYTHON="$(command -v python3)"`, isolating `cli_research_workspace_read_fork_diff_export_and_verify` into its own run without `RE_SEARCH_PYTHON` so all 545 baseline Rust tests executed and passed.

---

## 5. Test commands run and exact counts

Environment: Linux (Ubuntu 24.04), `rustc 1.100.0`, `cargo 1.100.0`, `node v22.23.5`, Python 3.12.3 (`sympy 1.14.0`, `numpy`, `mpmath`).

### Baseline suites (verified on `351b1287c9a54a700b0e33111b3891f195afd6d1` and untouched after adding `audit_honesty_*`)

| Command | Result | Exact counts |
|---|---|---|
| `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` | **PASS** | 32 JS assets checked, exit 0 |
| `node --check crates/desktop-server/tests/audit_honesty_ui.test.mjs` | **PASS** | Syntax check exit 0 |
| `node --test crates/desktop-server/tests/*.test.mjs` (excluding `audit_honesty_ui.test.mjs`) | **PASS** | **109 passed, 0 failed, 0 skipped** |
| `RE_SEARCH_PYTHON="$(command -v python3)" cargo test --workspace --no-fail-fast -- --skip cli_research_workspace_read_fork_diff_export_and_verify` (excluding `audit_honesty_*`) | **PASS** | **544 passed, 0 failed, 0 ignored, 1 skipped** (including `export`: 8/8 passed) |
| `cargo test -p cli --test desktop_and_ext cli_research_workspace_read_fork_diff_export_and_verify` | **PASS** | **1 passed, 0 failed** (545/545 baseline Rust tests pass) |
| `cargo clippy -p research-tools --test audit_honesty_anchors -- -D warnings` | **PASS** | **0 warnings, 0 errors** |
| `cargo clippy -p desktop-server --test audit_honesty_cites --no-deps -- -D warnings -A clippy::manual_unwrap_or` | **PASS** | **0 warnings, 0 errors** |
| `cargo clippy --workspace --all-targets -- -D warnings -A clippy::manual_unwrap_or -A clippy::items_after_test_module -A unused_imports` | **PASS** | **0 warnings, 0 errors** across all 13 crates |

### New failing honesty proof suites (expected FAIL — proving F1–F12)

| Command | Result | Exact counts |
|---|---|---|
| `cargo test -p research-tools --test audit_honesty_anchors -- --test-threads=1` | **FAIL (expected)** | **0 passed, 6 failed, 0 ignored** |
| `cargo test -p desktop-server --test audit_honesty_cites -- --test-threads=1` | **FAIL (expected)** | **0 passed, 6 failed, 0 ignored** |
| `node --test crates/desktop-server/tests/audit_honesty_ui.test.mjs` | **FAIL (expected)** | **0 passed, 3 failed, 0 skipped** |

---

## 6. Pre-existing Linux diagnostics and Windows-only code not compiled/run

1. **Pre-existing `cargo clippy --workspace --all-targets -- -D warnings` diagnostics on Linux / `rustc 1.100.0`** (verified on base commit `351b1287` in untouched files):
   - `crates/desktop-server/src/evidence.rs:737` (`clippy::manual_unwrap_or` on status allowlist match).
   - `crates/agent/src/guardrails.rs:335` (`clippy::items_after_test_module`).
   - `crates/research-tools/tests/sandbox_and_fs.rs:6` (`unused_imports` on `#[cfg(windows)]`-only imports).
   - `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` (`unused_imports` on `#[cfg(windows)]`-only imports).
2. **Pre-existing Linux env interaction in `crates/cli/tests/desktop_and_ext.rs::cli_research_workspace_read_fork_diff_export_and_verify`:** Hangs only when `RE_SEARCH_PYTHON` is exported globally because `re-search run --route api` rejects non-Windows Python worker configuration before `ScriptedMock` completes `accept()`; passes (`1 passed, 0 failed`) when invoked without `RE_SEARCH_PYTHON`.
3. **Not compiled or run on Linux (Windows-only):** `#[cfg(windows)]` modules (`crates/desktop-app` WebView2 shell, Windows AppContainer / JobObject worker sandboxing in `crates/worker`, DPAPI vault paths in `crates/chatgpt-auth`, Windows symlink/junction tests in `sandbox_and_fs.rs`) and Playwright browser journeys (`crates/desktop-server/tests/investigation_browser.py`, out of scope per `RULES.md`).
