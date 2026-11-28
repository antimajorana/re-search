# Report: `audit-mode-foundations` — Independent Carbon Review of F1, F4/F9, and F14a Ahead of V6, V17, and F5

- **Base commit audited:** `c5c89e64806c2430440ef4cd05b1b7fa86274078` (`origin/gemini/work/fixed`)
- **Review tier:** Independent Carbon (`pro`) multi-pass audit (Parallel Review A + Parallel Review B) plus full Linux test/lint verification
- **Toolchain on Linux audit runner:** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `cargo 1.100.0 (3acc79c0c 2026-11-05)`, `Python 3.12.16`, `node v22.23.5`
- **Repository changes in this branch:** Only this report (`docs/design/modes-2026-11/reports/audit-mode-foundations.md`); zero Rust, JS, test, golden, or `Cargo.lock` edits

---

## 1. Executive Verdict & Blocking Matrix

F1 (`f1-tool-profiles`), F4/F9 (`f4-f9-quote-budget`), and F14a (`f14a-pdf-core`) pass all existing unit, golden, integration, JS, and Set R tests on `c5c89e64806c2430440ef4cd05b1b7fa86274078` (**771 passed, 0 failed** in workspace Rust tests; **8 passed** in `export` with Python; **157 passed** in Node tests). However, independent end-to-end tracing across real entry points, caller strings, and receipt formats uncovered **four plan blockers / high-impact integration breaks** and **ten concrete production or evaluation defects** that must be accounted for as F5, V6, V17, and V4/V12 land.

| ID | Severity | Area | Summary of Concrete Impact on Plan | Smallest Owning Package |
|---|---|---|---|---|
| **A-1** | **Blocker (V6/F5)** | F1 resume & profile upgrade | `investigation/1 -> investigation/2` upgrade on resume is rejected in **four** places (`tool_profile.rs` + `resume_profile.rs`); V6 cannot support older `investigation/1` sessions using data-only `tool_profile.rs` edits. | F1 follow-up (`tool_profile.rs`, `resume_profile.rs`) + F5 (`turn_runner.rs`) |
| **A-2** | **High (V6/V12/Q8)** | `investigation/2` staged evolution | (a) V6 omits `record_context_cards` (added in V12), so V6 `investigation/2` sessions will fail resume in V12 unless `"record_context_cards"` is in `may_gain`. (b) Dropping `search_formal_library` per Overview 8.3 Q8 breaks `investigation/1 -> investigation/2` resume unless `ToolProfile` carries `retired_defs` (also needed in V19). | V6 (`tool_profile.rs`) + F1 follow-up (`retired_defs` in `resume_profile.rs`) |
| **A-3** | **High (F5/V6 CLI)** | Subscription CLI tool-profile split-brain | `cli_turn.rs` + `cli-route/src/core_tools.rs` hardcode `"research/1"`, while `mcp-server` (`core.exe`) serves the session profile; any profile differing from `research/1` (`investigation/2`, `explore/1`, `solve/1`) aborts Claude Code / Antigravity turns at `check_init`. | F5 / CLI-profile prep (`cli-route/src/core_tools.rs`, split `cli_turn.rs`) |
| **A-4** | **High (V6 quotes)** | Task-scoped `lines_sent` vs `accept_quote` | `Router::call_inner` omits `"run_id"` on `tool.request`, and `lines_sent.rs` lets `ReqRunId::Absent` pass `Some(run_id)`. Passing full `&events` to `accept_quote` leaks Task 1 tool reads into Task 2; passing `&events[req_idx..]` breaks `verify_quote` (`source_unavailable`, missing seq-1 `source.ingested`). | V6 workaround (`accept_quote_with_sent`) now; F4 follow-up (`after_seq` in `quote_core.rs` / `lines_sent.rs`) |
| **A-5** | **High (V6 host)** | `context.supplied` `caller` wire string | `SentLines::covers` requires exact `caller` match (`"agent:<wire_provider>:<sid>"` or `"claude-code-cli:<sid>"` / `"antigravity:<sid>"`), which is unknown inside `investigation::prepare` and changes if the user switches provider before Continue. | V6 (`investigation_tasks.rs` / `turn_runner.rs` post-transport hook) |
| **A-6** | **Medium (F4/V6)** | `lines_sent` over-credit, `true_text` leak & V6 stub | (a) `credit_fs_read` credits whole lines even when char `offset`/`max_chars` truncates mid-line; (b) `credit_search_paper_passages` credits unseen prefix of split `>1800`-byte lines > 1; (c) `accept_quote` leaks +/-2 unsent lines via `QuoteFault.true_text`; (d) `credit_read_map_node_result` is an empty stub `{}` that `investigate.md` forbids V6 from editing. | F4 follow-up (`lines_sent.rs`, `quote_core.rs`) + authorize V6 stub fill |
| **A-7** | **Medium (V6/F9)** | Desktop marker lists & CLI `run_budget` | (a) `paper_reader.rs:390-411` (600 lines) and `investigation_view.rs:165-178, 223-236` have marker lists omitted from `investigate.md` 0.1 row 4; (b) `count_model_calls` returns `0` on subscription CLI routes (`cli.result`, no `model.request`) and `0` for `RunScope::RunId`. | V6 (`investigation_markers.rs`, `investigation_view.rs`) + F9 follow-up (`run_budget.rs`) |
| **B-1** | **Confirmed (Test)** | `pdf_preview_browser.py` post-F14a breakage | Confirmed F14a report: `marked_pdf()` (`[text, blank, text]`) fails both `?line=2` (`\x0c` separator -> empty `X-ReSearch-Pdf-Page`) and Python `.splitlines()` splitting on `\x0c` (`line=7` instead of `5` -> falls back to Page 1 red instead of Page 3 green). Also `pdf_pages_tests.rs:20-62` hand-crafts a 2-line blank-page gap (`6..=8`) instead of `assemble_reading_file`'s 3-line gap (`7..=9`). | `desktop-server` (`pdf_preview_browser.py`) + F14a (`pdf_pages_tests.rs`) |
| **B-2** | **High (Prod PDF)** | `.gz` (arXiv PDF-only) & archive PDFs break `original-pdf` | For single-PDF `.gz` payloads (`arxiv_<id>.gz`), `ingest.rs:278` stores gzipped bytes (`1f 8b`) in `raw_blob` with `format = "pdf"`; `original_pdf.rs:32` fails `%PDF-` check (HTTP 400) and fallback `verified_page_lines` fails. For `.zip`/`.tar.gz` single PDFs (`ingest.rs:540-547`), `format` becomes `"zip"`/`"tar.gz"`, returning HTTP 404. | F14a / V4 (`ingest.rs`) |
| **B-3** | **High (V17/V4)** | `page_range > 1` & PDF byte/page cap mismatches | `original_pdf.rs:32` enforces 16 MiB (`MAX_PDF_BYTES`) vs 64 MiB (`MAX_USER_PDF_BYTES`); `routes_tools.rs:21` caps uploads at 10 MiB; `ingest_pdf_pages` omits `page_range` from `source.ingested`; `pdf_firstpage.rs:261` hardcodes `cur_page = 1` (misnumbering `401..=403` as `1..=3` in V17). | F14a / V4 / V17 (`original_pdf.rs`, `ingest.rs`, `pdf.rs`, `pdf_firstpage.rs`) |
| **B-4** | **Medium (Prod/V17)** | `investigation_outline.rs` `page_end = None` on `\x0c` | `finalize_ranges` sets `end_line = next.start_line - 1`, landing on `\x0c` whenever the next section starts a page, dropping `page_end` to `None`. V17 `skeleton/pdf.rs` must also trim trailing `\x0c`/blank lines before `page_of`. | `research-tools` (`investigation_outline.rs`) + V17 (`skeleton/pdf.rs`) |
| **B-5** | **High (P6 Scan)** | `pdf_pages_scan.rs` scan-false-negative paths | Violates P6 zero-tolerance rule (*"scanned page reported as plain text layer: Must be 0"*, Overview 7): Form XObject `/Matrix` & `depth >= 2` ignored (`~0%` coverage); `3 Tr` counted by op count rather than char count and not saved/restored across `q`/`Q`; full-page inline images (`BI`) with `>=100` OCR chars classified `TextLayer`. | F14a follow-up (`pdf_pages_scan.rs`) |
| **B-6** | **Medium (Provenance)** | Cross-page & `PageBasis::None` scan label loss + `\x0c` hyphen | `PdfQuoteMatch` checks only `start_line` basis (missing `TextLayer -> ScanTextLayer` spans); `<15`-alpha-char text on `PageBasis::None` serializes as `"none"` -> UI `'fulltext'` ("matched") in `quote_chip.js:53`; `pdf_match.rs:90-110` fails `"word-\n\x0c\nrest"` across page breaks; `quote_core.rs:196` does not call `pdf_match`. | F4/F14a (`pdf_match.rs`, `quote_core.rs`) + F6 (`quote_chip.js`) |
| **B-7** | **Medium (UI Honesty)** | KaTeX math rendering on scanned PDF text | Overview 2.12 rule 2 forbids silent LaTeX math rendering on scanned text; `quote_chip.js:136` uses plain text, but `canvas_sources.js:187,197,734,841` and `investigation_map.js:199` pass `.extracted.txt` / anchor excerpts through KaTeX without checking `pdf_pages` scan basis. | F6 / V14 (`canvas_sources.js`, `investigation_map.js`) |

---

## 2. Parallel Review A — Tool Profiles, Resume, Caller-Scoped `lines_sent`, and V6 Readiness

### 2.1 End-to-End Trace Through Actual Entry Points

1. **Brand-new Investigate session (Desktop HTTP -> Turn Runner -> Agent Loop):**
   - `POST /api/sessions` (`crates/desktop-server/src/routes_session.rs:235-269`) creates the session directory and, when `kind == "investigation"`, calls `research_tools::start_investigation` (`259-262`), appending **only** seq 1 `investigation.started` (no `session.start` yet).
   - On the first turn (`crates/desktop-server/src/turn_runner.rs:77, 162, 232-259`), `investigation::prepare` appends stage events (`scope_proposed`, etc.), and `build_desktop_router` (`crates/desktop-server/src/routers.rs:72-81`) calls `profile_for_session_events(&existing_events)` (`crates/research-tools/src/tool_profile.rs:254-267`).
   - Because `existing_events` has no `session.start`, `profile_for_session_events` falls through to `session_mode_from_events` -> `Mode::Investigate` -> `legacy_profile_name_for_mode(Mode::Investigate)` -> **`"investigation/1"`** (`tool_profile.rs:245`).
   - `turn_runner.rs:412-431` calls `inspect_session_for_resume(&ledger, &sid, None)` -> `StartFresh`, then `start_autonomous_research_loop_with_opts` -> `runner::run_with_cancel` (`crates/agent/src/runner.rs:150-169`), where `insert_tool_profile` (`crates/agent/src/resume_profile.rs:278-286`) stamps `"tool_profile": "investigation/1"` onto `session.start`.
2. **Existing Investigation ledger — Case 1: Pre-F1 legacy ledger (no `tool_profile` on `session.start`):**
   - `profile_for_session_events` (`tool_profile.rs:254-267`) sees `session.start` without `"tool_profile"`, detects `investigation.started` via `session_mode_from_events`, and returns `investigation/1`.
   - `resolve_session_profile` (`crates/agent/src/resume_profile.rs:64-75`) returns `ResolvedSessionProfile { profile: investigation/1, explicit: false }`.
   - `verify_router_for_resume` (`crates/agent/src/resume.rs:321-334` -> `resume_profile.rs:146-243`) requires `router.profile_name() == Some("investigation/1")` (`154-161`) and checks historical `session.start.tool_defs` via `check_tool_defs_drift_for_profile` (`113-138`, allowing additions listed in `investigation/1.may_gain`). On `Continue`, `runner::resume_with_cancel` stamps `"tool_profile": "investigation/1"` onto `session.resume` (`runner.rs:201-214`).
3. **Existing Investigation ledger — Case 2: Post-F1 `investigation/1` ledger (`"tool_profile": "investigation/1"` on `session.start`):**
   - `profile_for_session_events` (`tool_profile.rs:256-260`) returns `investigation/1` immediately from `session.start.payload.tool_profile`.
   - `resolve_session_profile` returns `explicit: true`. `verify_profile_policy_and_defs` (`resume_profile.rs:201-212`) enforces `defs_match_exact(&expected_defs, &resolved.profile.defs)` against the frozen 22-tool `investigation/1` list (`may_gain` is **not** consulted against the active router when `explicit == true`).
4. **CLI (`crates/cli/src/runtime.rs`) and Subscription CLI (`cli_turn.rs` + `mcp-server`):**
   - `re-search run` (`runtime.rs:92-100`) hardcodes `PROFILE_RESEARCH_V1`; `re-search resume` (`runtime.rs:165-171`) resolves via `profile_for_session_events(&events)`.
   - Subscription CLI turns (`crates/desktop-server/src/cli_turn.rs:449-495`) spawn `core.exe` (`crates/mcp-server/src/main.rs:229-245`) without setting `CORE_TOOL_PROFILE` and without recording `"tool_profile"` on `session.start`, while `cli_turn.rs:453` validates `cli.init` against `cli_route::core_tool_set` (`crates/cli-route/src/core_tools.rs:55-63`), which hardcodes `"research/1"`.

*(Performance/design note in `turn_runner.rs:424-430` + `autonomous.rs:195`: `turn_runner` calls `inspect_session_for_resume(&ledger, &session_id, None)` with `active_router_profile = None`, and `resume_autonomous_research_loop` immediately calls `verify_router_for_resume` again, duplicating full hash-chain verification and blob rehydration on every resumed desktop turn while hiding profile mismatches from `inspect_session_for_resume`.)*

---

### 2.2 Confirmed Findings in Parallel Review A

#### Finding A-1 [Blocker for V6 & F5] — `investigation/1 -> investigation/2` resume upgrade is blocked in four places
- **Exact locations:**
  1. `crates/research-tools/src/tool_profile.rs:238-267` (`legacy_profile_name_for_mode`, `profile_for_session_events`)
  2. `crates/agent/src/resume_profile.rs:154-161` (active-router profile name equality check)
  3. `crates/agent/src/resume_profile.rs:201-212` (`explicit == true` exact `defs_match_exact` check against `resolved.profile.defs`)
  4. `crates/agent/src/resume_profile.rs:77-92, 278-306` (Rule R2 `session.resume` profile-change rejection)
- **Design contradiction:** `docs/design/modes-2026-11/00-overview.md:152-153` and `docs/design/modes-2026-11/investigate.md:425-426, 562-565` specify that pre-F1 Investigation ledgers map to `investigation/1`, new Investigate sessions after F5/V6 start on `investigation/2`, and an `investigation/1` session upgrades on its next resume to gain `read_map_node` and `record_map_reading`. Meanwhile `docs/design/modes-2026-11/agents/inv-v6-tasks.md:21-24` instructs V6: *"Use only F1's profile data to support older `investigation/1` sessions... Do not edit `resume.rs`."*
- **Why data-only edits fail:**
  - Mutating `investigation_v1().defs` to include V6 tools violates Overview 2.2 (`investigation/1` = today's 22-tool surface) and breaks `crates/desktop-server/tests/eval_set_r.rs:204-206, 251-253` (which builds a 22-tool router with `PROFILE_INVESTIGATION_V1`).
  - Leaving `investigation/1` at 22 tools and having F5 (`f5-f7-mode-shell.md:30`) build `investigation/2` for `Mode::Investigate` causes `resume_profile.rs:154-161` to reject **every** pre-F1 and post-F1 `investigation/1` session with `ResumeRejectKind::Drift` (`"active router profile 'investigation/2' differs from session profile 'investigation/1'"`). Even if `154-161` were relaxed, `201-212` rejects post-F1 `investigation/1` sessions (`defs_match_exact`: 24 tools != 22 tools), and `85-90` rejects the subsequent leg (`"tool_profile changed from investigation/1 to investigation/2"`).
  - Also on brand-new sessions, `routes_session.rs:259-262` writes `[investigation.started]` before `session.start` exists; `profile_for_session_events` (`tool_profile.rs:254-267`) cannot distinguish an unstarted session (`[investigation.started]`, no `session.start`) from a pre-F1 legacy session (`[investigation.started, session.start{no tool_profile}]`) unless it checks whether `session.start` is present (or F5 passes the explicit target profile on fresh start).
- **Minimal synthetic example:** Append `investigation.started` + `session.start` (`"tool_profile": "investigation/1"`, 22 defs) + `turn.end`, then call `verify_router_for_resume` with a router built from `investigation/2` (24 defs) -> `Err(ResumeRejection { kind: Drift, reason: "active router profile 'investigation/2' differs from session profile 'investigation/1'" })`.
- **Smallest owning package:** Small F1 follow-up in `crates/research-tools/src/tool_profile.rs` (`upgrades_to: Option<&'static str>`, e.g. `investigation/1 -> Some("investigation/2")`, plus `profile_for_session_events` distinguishing missing `session.start` from legacy `session.start`) and `crates/agent/src/resume_profile.rs` (allowing `resolved -> active` along the `upgrades_to` chain when `active.defs` equals `resolved.defs` plus tools permitted by `may_gain`, and allowing monotonic upgrade in Rule R2 without touching `crates/agent/src/resume.rs`).

#### Finding A-2 [High — V6/V12 & Overview Q8/V19] — `investigation/2` staged additions and `search_formal_library` removal
- **Exact locations:** `crates/research-tools/src/tool_profile.rs:146-168`; `crates/agent/src/resume_profile.rs:113-138, 209-225`; `crates/research-tools/tests/golden/tool_profiles.txt:25, 63`; `crates/desktop-server/src/turn_runner.rs:260, 322-335`.
- **Consequence:**
  1. **V6 -> V12 `record_context_cards` break:** `00-overview.md:153` lists `read_map_node`, `record_map_reading`, and `record_context_cards` in `investigation/2` with only `record_prior_work_check` in `may_gain`, whereas `inv-v6-tasks.md:19, 34` adds only the first two in V6 and defers `record_context_cards` to V12. Because V6 sessions have `explicit == true`, `resume_profile.rs:214-225` checks historical `session.start.tool_defs` via `check_tool_defs_drift_for_profile(&session_defs, active_profile)`. Unless `"record_context_cards"` is included in `investigation/2.may_gain` (alongside `"record_prior_work_check"`), every session started in V6 will fail to resume once V12 lands.
  2. **Overview 8.3 Q8 (`00-overview.md:865`) `search_formal_library` removal:** `lean_check` is already mounted separately via `build_lean_router` (`turn_runner.rs:260, 322-335`), whereas `search_formal_library` is inside `research_tool_defs()` (`tool_profiles.txt:25, 63`). If `investigation/2` omits `search_formal_library`, `check_tool_defs_drift_for_profile` (`resume_profile.rs:113-138`) rejects every upgrading `investigation/1` session (`"tools were removed or reordered"` / `"tool definition drift"`). Additionally, `turn_runner.rs:260, 322-335` builds `lean_router` and `lean_worker` unconditionally for all non-`paper_reading_turn` turns; F5/S7 must gate `lean_router` on mode/profile.
- **Smallest owning package:** V6 (`tool_profile.rs`: keep `search_formal_library` in `investigation/2` for V6 compatibility OR pair with `retired_defs: &'static [&'static ToolDef]` in `tool_profile.rs` + `resume_profile.rs`, which V19 also needs; include `"record_context_cards"` and `"record_prior_work_check"` in `investigation/2.may_gain`); F5 (`turn_runner.rs`: gate `lean_router` off in `Mode::Investigate` / `Mode::Explore`).

#### Finding A-3 [High — Subscription CLI] — `cli_turn.rs` / `cli-route` hardcode `"research/1"` while `core.exe` serves the session profile
- **Exact locations:** `crates/desktop-server/src/cli_turn.rs:449-495` (file is 627 lines, over the 600-line F0 limit); `crates/cli-route/src/core_tools.rs:55-63, 118-135`; `crates/cli-route/src/claude.rs:148`; `crates/cli-route/src/antigravity.rs:185-188`; `crates/mcp-server/src/main.rs:229-245`.
- **Consequence:** Today `research/1` and `investigation/1` expose the exact same 23 MCP tools (`22 + set_session_title`). As soon as `investigation/2` (25 MCP tools), `explore/1`, or `solve/1` is active:
  - `core.exe` (`mcp-server/src/main.rs:236`) resolves `investigation/2` from session events and offers 25 tools to Claude Code / Antigravity.
  - `cli_turn.rs:453` computes expected tools via `core_tool_set(Some(...))` -> `PROFILE_RESEARCH_V1` (23 tools), and `claude::check_init` / `antigravity::check_init` abort every subscription CLI turn with `tool list mismatch` / `unexpected core tools offered`.
  - `cli_turn.rs:456-495` also never writes `"tool_profile"` onto `session.start`, leaving CLI-started post-F1 sessions in `explicit == false` legacy mode.
- **Minimal synthetic example:** Start an `investigation/2` session and run a turn on `claude-cli`; `core.exe` advertises `mcp__core__read_map_node`, `claude::check_init` compares 25 MCP tools against `core_tool_set`'s 23 `research/1` tools and fails the turn before any model output.
- **Smallest owning package:** F5 (or a small CLI-profile prep commit): add profile-aware `core_tool_set_for_profile(profile_name, ...)` / `core_tool_ids_for_profile(...)` in `crates/cli-route/src/core_tools.rs`, split `crates/desktop-server/src/cli_turn.rs` (627 lines) under 600 lines, pass `CORE_TOOL_PROFILE`, and stamp `"tool_profile"` in `cli_turn`'s `start_fields`.

#### Finding A-4 [High — Task-Scoped `lines_sent` & `accept_quote`] — Cross-task tool-read leakage vs `source_unavailable` catch-22
- **Exact locations:** `crates/tools/src/router.rs:140-156` (`Router::call_inner`); `crates/research-tools/src/lines_sent.rs:309-312, 329-335`; `crates/research-tools/src/quote_core.rs:178-182, 221-247`; `crates/research-tools/src/quote_core_tests.rs:196-208`.
- **Mechanism (verified against caller, test, and receipt):**
  - `Router::call_inner` (`router.rs:145-156`) emits `tool.request` with `{call_id, caller, tool, tool_def_hash, effect, approval_scope, args}` — **never** `"run_id"`.
  - Because `tool.request` has no `"run_id"`, `lines_sent.rs:331-335` intentionally lets `ReqRunId::Absent` pass even when `run_id = Some(want)` (pinned by `quote_core_tests.rs:196-208`).
  - `accept_quote` (`quote_core.rs:221-230`) passes the **same** `session_events` slice to both `verify_quote` (which scans `session_events` for seq-1 `source.ingested` at `178-182`) and `lines_sent_with_ledger(session_events, ledger, run_id)`.
- **Consequence for V6:**
  - If V6 calls `accept_quote(..., &full_events, Some(&ledger), Some(&task2_run_id), caller, ...)` in Task 2, `lines_sent` credits **every** `read_paper_lines` / `fs_read` / `search_paper_passages` / `read_map_node` call made by `caller` in Task 1 or earlier question turns, violating Overview 2.5's task-scoped proof (`"within this task"`).
  - Conversely, if V6 slices `&full_events[task2_req_idx..]` and passes that slice to `accept_quote`, `verify_quote` misses seq-1 `source.ingested` and rejects every quote with `QuoteFault { code: "source_unavailable" }`.
- **Immediate V6 workaround + permanent fix:**
  - **In V6 without touching F4:** Slice `let task_events = &events[req_idx..];`, compute `let sent = lines_sent_with_ledger(task_events, Some(ledger), Some(&run_id));`, and call `accept_quote_with_sent(sandbox, session_id, &events, &sent, caller, ...)` (`quote_core.rs:236-247`) with full `&events` for `source.ingested` lookup and `task_events` for `sent`!
  - **Smallest owning package for permanent fix:** F4 follow-up (`lines_sent.rs` + `quote_core.rs`: accept `after_seq: Option<u64>` or `RunScope` alongside `run_id`).

#### Finding A-5 [High — V6 Host `context.supplied`] — Exact `caller` wire identity and provider-switch Continue hazard
- **Exact locations:** `crates/research-tools/src/lines_sent.rs:129-144` (`SentLines::covers`); `crates/agent/src/runner.rs:245-246`; `crates/mcp-server/src/main.rs:268-274`; `crates/desktop-server/src/turn_runner.rs:77, 167-172, 201`; `crates/agent/src/autonomous.rs:198-199`.
- **Consequence:** `SentLines::covers` checks `k.caller == caller` by exact string equality. Actual `ToolContext.caller` strings passed to recorder handlers are:
  - In-core agent loop (`runner.rs:245-246`): `"agent:<transport.info().provider>:<session_id>"`, where `info.provider` is the transport wire ID (`"anthropic-messages"`, `"openai-chat"`, `"ollama-chat"`, `"gemini-generate-content"`, `"chatgpt-responses"` — **not** UI route names like `"anthropic"` or `"openai"`).
  - MCP `core.exe` (`mcp-server/src/main.rs:268-274`): `"claude-code-cli:<session_id>"` or `"antigravity:<session_id>"` (**not** `"claude-cli"`).
  - In `turn_runner.rs:77`, `investigation::prepare` runs **before** `run_session_turn` resolves settings and constructs `transport` (`167-201`). Emitting `context.supplied` inside `prepare` with a role label (`"main_agent"`, `"investigator"`) or UI provider name causes 100% of quotes from host-injected prompt ranges to fail with `LinesNotSent`. Furthermore, if a user pauses a task and resumes (`Continue`) under a different provider (`autonomous.rs:198-199`), the new leg has a new `caller` string and needs a fresh `context.supplied` receipt for that `caller` and `run_id`.
- **Smallest owning package:** V6 (`investigation_tasks.rs` + `turn_runner.rs` / `cli_turn.rs`: emit `context.supplied` after transport/CLI route resolution using a single shared helper that computes the exact `caller` string, and re-emit on Continue if `caller` changed).

#### Finding A-6 [Medium — `lines_sent` Soundness, `true_text` Leak, and V6 `read_map_node` Stub]
- **Exact locations:**
  1. `crates/research-tools/src/lines_sent.rs:442-450` vs `crates/research-tools/src/fs_tools.rs:285-343` (`credit_fs_read`)
  2. `crates/research-tools/src/lines_sent.rs:398-405` vs `crates/research-tools/src/paper_passages.rs:329-406` (`credit_search_paper_passages`)
  3. `crates/research-tools/src/quote_core.rs:226-229` vs `crates/research-tools/src/investigation_anchor.rs:335-372, 546-557` (`QuoteFault.true_text`)
  4. `crates/research-tools/src/lines_sent.rs:456-461` (`credit_read_map_node_result` empty stub)
- **Consequences & synthetic examples:**
  1. **`fs_read` partial-line over-credit:** `fs_read` slice mode slices by character `offset` and `max_chars`, then reports line numbers containing the slice (`fs_tools.rs:306-318`). Calling `fs_read(path, offset=0, max_chars=5)` on a 200-char line 1 reports `start_line: 1, end_line: 1, truncated: true`, and `credit_fs_read` credits all of line 1 so the caller can cite unseen text at columns `6..200`.
  2. **`search_paper_passages` split-line over-credit:** `lines_sent.rs:398` adjusts `start_line` only when `start_line == 1 && start_byte > 0`. When `next_excerpt` (`paper_passages.rs:365-377`) splits a `>1800`-byte line at `line = 20` (`breaks == 0`), chunk 2 has `start_line: 20, start_byte > 0`; `lines_sent` credits all of line 20 including the unseen prefix (compare `investigation_cites.rs:89-105`).
  3. **`accept_quote` `true_text` oracle on unsent lines:** `accept_quote` runs `verify_quote` first (needed so relocated quotes check `sent_lines.covers` at the relocated span, tested at `quote_core_tests.rs:189-193`, and `VerificationFailed` precedence is asserted at `quote_core_tests.rs:162-164`). However, on `quote_not_found` for unsent lines `100..=100`, `QuoteFault.true_text` returns raw lines `98..=102` (`investigation_anchor.rs:338`), leaking 5 unsent lines to an unauthorized caller unless `accept_quote_with_sent` redacts `fault.true_text = None` when `!sent_lines.covers(caller, path, &file_sha256, start_line, end_line)`.
  4. **Unowned `credit_read_map_node_result` stub:** `lines_sent.rs:456-461` is `{}` (no-op). `inv-v6-tasks.md:17` expects `read_map_node` results to be credited for `record_map_reading`, whereas `investigate.md:504, 546` says V6 adds no credit code and does not own `lines_sent.rs`.
- **Smallest owning package:** F4 follow-up (`lines_sent.rs`, `quote_core.rs`) + explicit authorization in V6 (`inv-v6-tasks`) to implement `credit_read_map_node_result` in `lines_sent.rs` (handling `truncated`).

#### Finding A-7 [Medium — Desktop Marker Lists & F9 `run_budget.rs` Ahead of V6]
- **Exact locations:** `crates/desktop-server/src/paper_reader.rs:390-411` (600 lines); `crates/desktop-server/src/investigation_view.rs:165-178, 223-236`; `crates/research-tools/src/run_budget.rs:87-100, 141-163`.
- **Consequence:**
  1. **Marker lists omitted from `investigate.md` 0.1 row 4:** Beyond the 3 `research-tools` marker arrays (`investigation.rs:386-407`, `investigation_conventions.rs:225-235`, `investigation_evidence.rs:134-154`), `desktop-server` has `paper_reader.rs:390-411` (at the 600-line ceiling) and `investigation_view.rs:165-178, 223-236`. In `investigation_view.rs`, `mark_last_running` (`165-178`) checks whether a turn is open after the last stage marker; omitting `investigation.task_requested` / `investigation.task_finished` makes a live V6 task turn resurrect an older crashed `"interrupted"` stage item as `"running"` in the UI.
  2. **`count_model_calls` blind to subscription CLI & `RunId`:** Subscription CLI turns (`cli_turn.rs` / `claude.rs` / `antigravity.rs`) never emit `"model.request"` (they emit `"cli.init"`, `"model.tool_use"`, and `"cli.result"` carrying `"num_turns"`), so `count_model_calls` always returns **`0`** on `claude-cli` and `antigravity`. `RunScope::RunId` also returns **`0`** everywhere today because `runner.rs` does not stamp `"run_id"` on `model.request`, and `RunScope::AfterSeq(request_seq)` overcounts across Pause/Continue or subsequent turns unless bounded per leg via `RunScope::SeqRange`.
- **Smallest owning package:** V6 (export `STAGE_MARKERS` from `investigation_markers.rs` and update `investigation_view.rs`; split `paper_reader.rs` if touched) + F9 follow-up (`run_budget.rs`: count `cli.result.payload.num_turns` or `model.tool_use` turns when `model.request` is absent).

### 2.3 Independent Verification of Golden & Set R Checks (Why A-1..A-3 Passed)
- `crates/research-tools/tests/tool_def_golden.rs` (3/3) and `crates/research-tools/tests/golden/tool_defs.txt` are intact and byte-identical to pre-F1.
- `crates/research-tools/tests/tool_profile_golden.rs` (3/3) verifies `tool_profiles.txt`, **but** line 43 snapshots only `policy.version` (not policy rules or `mcp_defs()`), and tests no CLI `core_tools` parity.
- `crates/desktop-server/tests/eval_set_r.rs` (5/5) uses a private test helper `build_profile_router` (`crates/desktop-server/tests/eval_set_r_support/mod.rs:109-133`) rather than `desktop_server::routers::build_desktop_router` or `turn_runner`, and exercises only synthetic `"test/investigation-2"` upgrades from **pre-F1** (`explicit == false`) ledgers — never testing post-F1 (`explicit == true`) `investigation/1 -> investigation/2` upgrades or multi-leg `session.resume` after an upgrade.

---

## 3. Parallel Review B — PDF Receipt, Page Breaks & 1-Based Lookups, Scan Provenance, and `original-pdf`

### 3.1 End-to-End Trace (Upload/Ingest -> `.extracted.txt` -> Blank Page -> `verified_page_lines` -> `original-pdf`)

Tracing a 3-page PDF with page texts `["A\nB", "", "C"]` (text page 1, blank page 2, text page 3):

1. **Extraction & Reading-File Assembly (`crates/research-tools/src/pdf_pages.rs:506-536`):**
   - `assemble_reading_file` joins pages with `"\n\x0c\n"`, incrementing `cur_line += 1` at `idx > 0` (`line 515`) for the `\x0c` line and `cur_line += 1` on `text.is_empty()` (`line 518`) for the empty blank-page line.
   - Resulting `.extracted.txt` bytes: `"A\nB\n\x0c\n\n\x0c\nC"` (10 bytes, 5 `'\n'` bytes -> **6** 1-based `\n`-delimited lines):

| 1-Based Line | Raw Line Content | `PageMap::page_of(line)` | `verified_page_lines` -> `page_for_line` | `/original-pdf?line=N` (`X-ReSearch-Pdf-Page`, `#page=`) |
|---|---|---|---|---|
| `1` | `"A"` | `Some(1)` | `Some(1)` | `"1"`, `#page=1` |
| `2` | `"B"` | `Some(1)` | `Some(1)` | `"1"`, `#page=1` |
| `3` | `"\x0c"` (P1->P2 break) | `None` | `None` | `""` (empty header; viewer opens page 1) |
| `4` | `""` (blank Page 2) | `None` | `None` | `""` (empty header; viewer opens page 1) |
| `5` | `"\x0c"` (P2->P3 break) | `None` | `None` | `""` (empty header; viewer opens page 1) |
| `6` | `"C"` | `Some(3)` | `Some(3)` | `"3"`, `#page=3` |

2. **Receipt Fields (`crates/research-tools/src/ingest.rs:285-336`):**
   - `source.ingested` records `format: "pdf"`, `pdf_extractor: "f14a/1"`, `pdf_pages: [{page:1, start_line:1, end_line:2, char_count:3, image_coverage:..., basis:"text_layer"}, {page:2, start_line:0, end_line:0, char_count:0, ..., basis:"none"}, {page:3, start_line:6, end_line:6, char_count:1, ..., basis:"text_layer"}]`, and `pdf_page_lines: [[1, 1, 2], [3, 6, 6]]`.
3. **1-Based Line Consumers (Verified Consistent on `'\n'`):**
   - `read_paper_lines` (`paper_lines.rs:143`), `VerifiedFile::from_bytes` (`investigation_anchor.rs:227-233`), `routes_tools.rs:183` (`read_source_text`), `verified_page_lines` (`pdf.rs:73-122`), and `citedLineRange` (`crates/desktop-server/assets/canvas_sources.js:795-813`) all split strictly on `'\n'` (`b'\n'`), agreeing on L1..2 -> P1, L3..5 -> `None`, L6 -> P3.
4. **Legacy Pre-F14a Receipt Fallback & Forking (`crates/research-tools/src/pdf.rs:79-121, 134-211`, `crates/export/src/fork.rs:77, 158`):**
   - When `pdf_extractor` is absent on `source.ingested`, `verified_page_lines` verifies `.extracted.txt` against `file_sha256` and returns stored `pdf_page_lines` (`79-96`), or re-runs `extract_pdf_bytes_legacy` (`113-116`, logic-identical to pre-F14a `extract_pdf_bytes`, emitting `--- [Page N] ---` headers) when `pdf_page_lines` is also absent.
   - `export/src/fork.rs:77, 158` copies full `source.ingested` payloads verbatim, preserving `pdf_extractor`, `pdf_pages`, and `pdf_page_lines`.

---

### 3.2 Confirmed Findings in Parallel Review B

#### Finding B-1 [Confirmed Test Defect] — `pdf_preview_browser.py` fails on two `\x0c` assumptions; `pdf_pages_tests.rs` hand-crafts wrong blank-page gap
- **Exact locations:** `crates/desktop-server/tests/pdf_preview_browser.py:52-67`; `crates/research-tools/src/pdf_pages_tests.rs:20-62`.
- **Verification of F14a report finding:** Confirmed 100% accurate. `marked_pdf()` (`crates/desktop-server/tests/audit_flow_readers_and_pages.rs:342-403`) is itself a 3-page `[text, blank, text]` PDF (`"--- [Page 3] ---"` decoy on P1, blank P2, `"Figure 5. Green rectangle"` on P3). Under `f14a/1`, `.extracted.txt` is `"--- [Page 3] ---\n\x0c\n\n\x0c\nFigure 5. Green rectangle"` (`pdf_page_lines = [[1, 1, 1], [3, 5, 5]]`):
  1. `pdf_preview_browser.py:57` fetches `/original-pdf?line=2` and asserts `X-ReSearch-Pdf-Page == "1"`. Line 2 is now `"\x0c"`, so `page_for_line` returns `None` and the header is `""`.
  2. `pdf_preview_browser.py:52` runs `extracted.read_text("utf-8").splitlines()` in Python, which splits on `\x0c` (`U+000C`) as well as `\n`, computing 1-based `line = 7` instead of `5` for `"Figure 5. Green rectangle"`. Passing `startLine: 7` to `openOriginalPdf` resolves `page = None`, loads Page 1 (red) instead of Page 3 (green), and fails `check_color(3, 1)`.
- **Additional unit-test defect found:** `crates/research-tools/src/pdf_pages_tests.rs:20-62` (`pagemap_lookup_with_blank_middle_page`) hand-constructs `PageMap` with P1 `1..=3`, blank P2 `0..=0`, and P3 `6..=8` (a 2-line gap), whereas `assemble_reading_file` on a 3-line P1 + blank P2 + 3-line P3 actually produces P3 at **`7..=9`** (a 3-line gap: `\x0c`, blank line, `\x0c`).
- **Smallest owning package:** `desktop-server` (`pdf_preview_browser.py`: use `.split("\n")` at line 52 and `line=1` at line 57) + F14a (`pdf_pages_tests.rs`: test `PageMap` via `assemble_reading_file` or `7..=9`).

#### Finding B-2 [High — Separate Production Defect] — `.gz`-wrapped PDFs (arXiv PDF-only e-prints) and `.zip`/`.tar.gz` PDFs break `original_pdf.rs` and `verified_page_lines` fallback
- **Exact locations:** `crates/research-tools/src/ingest.rs:278, 433-439, 540-547`; `crates/research-tools/src/arxiv.rs:527-541`; `crates/desktop-server/src/original_pdf.rs:27, 32`; `crates/research-tools/src/pdf.rs:76, 105, 113`.
- **Mechanism & user-visible consequence:**
  1. **arXiv PDF-only `.gz` deliveries:** When arXiv serves a PDF-only e-print at `/e-print/<id>` as gzip-compressed PDF bytes (`arxiv_<id>.gz`, `arxiv.rs:527-540`), `extract_archive_bytes` returns `ArchivePayload::RawPdf(pdf_bytes)` (`ingest.rs:433-439`), setting `format = "pdf"` and extracting `pdf_pages` from decompressed `pdf_bytes`. **However**, `finalize_ingestion` (`ingest.rs:278`) writes `req.raw_bytes` (the **gzip-compressed** bytes starting with `1f 8b`, not `%PDF-`) into `raw_blob`! When the user clicks "Open PDF" (`GET /api/sessions/:id/sources/:source_id/original-pdf`), `original_pdf.rs:32` checks `pdf_bytes.starts_with(b"%PDF-")` on `raw_blob` and fails with **HTTP 400** (`"The stored document is not a supported PDF."`). Any fallback call to `verified_page_lines` without `pdf_page_lines` (`pdf.rs:105, 113`) likewise fails with `"missing %PDF- header"`.
  2. **Single-PDF `.zip` / `.tar.gz` archives:** `extract_tar_or_zip` (`ingest.rs:540-547`) extracts `pdf_pages` from the inner `.pdf` entry, then overwrites `p.format = format.to_string()` (`"zip"` or `"tar.gz"`). `original_pdf.rs:27` requires `rec.format == "pdf"` -> **HTTP 404** (`"An original PDF was not stored for this source."`), and `verified_page_lines` (`pdf.rs:76`) errors out (`"source is 'zip', not pdf"`).
- **Minimal synthetic example:** Gzip-compress `synthetic_three_page_pdf()` bytes and pass to `ingest_bytes` with `filename: "arxiv_2501.00001.gz"`; `source.ingested` has `format == "pdf"`, `ledger.get_blob(&raw_blob)` starts with `[0x1f, 0x8b]`, and `GET .../original-pdf` returns `400 Bad Request`.
- **Smallest owning package:** F14a follow-up / V4 (`crates/research-tools/src/ingest.rs`: store decompressed `pdf_bytes` as `raw_blob` when `ArchivePayload::RawPdf(pdf_bytes)` or single-PDF archive is ingested, or record a dedicated `pdf_blob` hash).

#### Finding B-3 [High — V17 & User PDFs] — Partial `page_range > 1` & byte/page cap mismatches across ingest, firstpage, upload, and `original-pdf`
- **Exact locations:** `crates/desktop-server/src/original_pdf.rs:32, 38`; `crates/desktop-server/src/routes_tools.rs:21`; `crates/research-tools/src/ingest.rs:220-229, 296-299, 473`; `crates/research-tools/src/pdf.rs:103-110`; `crates/research-tools/src/pdf_firstpage.rs:261-265`.
- **Consequences:**
  1. **Cap mismatches:** `pdf_pages.rs:36` defines `MAX_USER_PDF_BYTES = 64 MiB` (and `MAX_USER_PDF_PAGES = 2000`), but `routes_tools.rs:21` rejects desktop file uploads > **10 MiB** (`MAX_UPLOAD_BYTES_PER_FILE`), `ingest.rs:473` extracts uploaded PDFs with `PdfExtractOptions::default()` (**16 MiB**, **400 pages**), and `original_pdf.rs:32` rejects serving any stored PDF > **16 MiB** (`MAX_PDF_BYTES`) with HTTP 400! (`original_pdf.rs:38` also silently swallows `verified_page_lines` errors via `.unwrap_or_default()`, losing `#page=` navigation on verification failure.)
  2. **Partial `page_range` (`401..=403`) in V17 (`inv-v17-pdf-skeleton.md:26`):** `ingest_pdf_pages` (`ingest.rs:220-229`) does not persist `page_range` in `source.ingested`, so fallback re-extraction (`pdf.rs:103-110`) runs `page_range: None` with 16 MiB / 400-page defaults and fails. Worse, `pdf_firstpage.rs:261-265` (`split_numbered_pages`) initializes `cur_page = 1` instead of reading `PageMap` page numbers, mislabeling physical page `401` as `page: Some(1)` in metadata/first-page extraction.
- **Smallest owning package:** F14a follow-up / V4 / V17 (`original_pdf.rs`: `MAX_USER_PDF_BYTES`; `routes_tools.rs` + `ingest.rs`: user PDF options & `pdf_page_range` receipt field; `pdf_firstpage.rs`: thread `PageMap` page numbers instead of `cur_page = 1`).

#### Finding B-4 [Medium — Production Outline & V17 Hazard] — `investigation_outline.rs` `finalize_ranges` lands on `\x0c`, dropping `page_end` to `None`
- **Exact locations:** `crates/research-tools/src/investigation_outline.rs:148-152, 363-371, 481-482`; `docs/design/modes-2026-11/agents/inv-v17-pdf-skeleton.md:16-17`.
- **Mechanism & consequence:** `finalize_ranges` (`investigation_outline.rs:363-371`) sets `sections[i].end_line = next_start - 1`. Whenever section `i + 1` starts on line 1 of a new PDF page, `next_start - 1` is the `\x0c` separator line (or blank-page gap `k+1..=k+3`), where `page_for_line(page_lines, end_line)` returns `None`. Both `format_outline_for_prompt` (`148-152`: `(Some(a), Some(b))`) and `ScopeTarget` (`481-482`) then drop the end page (or whole page label) in Investigate scope proposals. V17 (`skeleton/pdf.rs`) will hit the identical bug unless node `end_line` trims trailing `\x0c` and blank-page lines before calling `PageMap::page_of`.
- **Minimal synthetic example:** Page 1 lines `1..=2` (`"1 Introduction\nBody"`), line 3 `"\x0c"`, Page 2 lines `4..=5` (`"2 Methods\nMore"`). Section 1 gets `start_line: 1, end_line: 3` -> `page_start: Some(1), page_end: None` -> outline omits page range.
- **Smallest owning package:** `research-tools` (`investigation_outline.rs`: step `end_line` back past `\x0c`/empty separator lines before `page_for_line`) + V17 (`skeleton/pdf.rs`).

#### Finding B-5 [High — P6 Scan Provenance] — Three scan-false-negative paths in `pdf_pages_scan.rs`
- **Exact locations:** `crates/research-tools/src/pdf_pages_scan.rs:58-60, 188-201, 222-252, 337`; `docs/design/modes-2026-11/00-overview.md:793` (Set P6).
- **Consequence (violates Set P6 hard gate: *"A scanned page reported as a plain text layer: Must be 0"*):**
  1. **Form XObject `/Matrix` & depth:** `walk_ops` (`222-236`) recurses into `/Subtype /Form` XObjects without multiplying the Form stream's `/Matrix` into `ctm` and stops at `depth < 2`. Scanned PDFs that place page images inside a scaled Form XObject (e.g. `1 0 0 1 0 0 cm` outside, `[612 0 0 792 0 0]` in Form `/Matrix`) compute `1x1 pt` image area (`image_coverage ~ 0.0002%`) -> misclassified as `PageBasis::TextLayer`.
  2. **`3 Tr` invisible OCR text ratio & graphics state:** `q`/`Q` (`188-201`) saves/restores `ctm` only, not `text_render_mode` (`Tr`), and `has_invisible_text` (`248`: `invisible_text_ops * 2 >= total_text_ops`) counts PDF operators instead of rendered characters. A scanned page with 1 `TJ` op holding 3,000 invisible (`3 Tr`) OCR characters plus 2 visible (`0 Tr`) running-header/page-number `Tj` ops has `1 * 2 < 3` (`has_invisible_text == false`); if its scan image is <75% page area (cropped margin scan at 65% area), it is misclassified as `PageBasis::TextLayer`.
  3. **Full-page inline images (`BI`):** `walk_ops` (`239-241`) sets `has_inline_image = true` with `0.0` added to `image_area`, and `classify_page` (`337`) triggers scan on `has_inline_image` only when `chars_per_std_page < 100.0`. Any scanned page stored as an inline image with >=100 visible OCR chars is misclassified as `PageBasis::TextLayer`.
- **Smallest owning package:** F14a follow-up (`crates/research-tools/src/pdf_pages_scan.rs`: apply Form `/Matrix`, push/pop `Tr` on `q`/`Q`, weight `3 Tr` by char count, and compute `BI` `/W`/`/H` CTM area or treat `3 Tr` > 50 chars as scan).

#### Finding B-6 [Medium — Quote Provenance & PDF Matching] — Cross-page scan basis loss, `PageBasis::None` -> `"matched"` UI label, `\x0c` hyphenation, and `quote_core` disconnect
- **Exact locations:** `crates/research-tools/src/pdf_match.rs:36-37, 90-110, 201-212`; `crates/desktop-server/assets/quote_chip.js:50-55`; `crates/research-tools/src/quote_core.rs:196`.
- **Consequences:**
  1. **Cross-page scan loss:** `match_pdf_quote` (`pdf_match.rs:201-208`) inspects only `record_of_line(start_line)`. A quote starting at the bottom of Page 1 (`TextLayer`) and continuing onto Page 2 (`ScanTextLayer`) reports `basis: TextLayer, is_scan: false`.
  2. **`PageBasis::None` text labeled `"matched"` in UI:** Pages with `< 15` alphabetic chars (`pdf_pages_scan.rs:20, 329`, e.g. `"Theorem 1: x = 0"` = 8 alpha chars over a full-page scan image) get `PageBasis::None` (`basis: "none"`), yet their text IS written to `.extracted.txt` (`pdf_pages.rs:517-526`). `quote_chip.js:50-55` (`basisKind`) maps only `'scan_text_layer'` and `'ocr'` to `'ocr'`, defaulting `'none'` to `'fulltext'` (**"matched"** instead of **"OCR — check page"**).
  3. **Page-break hyphenation:** `fold_pdf_file_for_match` (`pdf_match.rs:90-110`) folds `word-\nrest` only when `chars[j]` after optional horizontal whitespace is alphanumeric; across a page break (`"electro-\n\x0c\nmagnetic"`), `chars[j]` is `'\x0c'`, leaving `"electro- magnetic"` and failing `"electromagnetic"`.
  4. **`quote_core.rs` does not call `pdf_match`:** `verify_quote` (`quote_core.rs:196`) delegates solely to `SourceIndex::verify_pointer` (`fold_with_map`), so PDF ligatures (`ﬁ`), soft hyphens (`U+00AD`), and line-end hyphens are not normalized during `verify_quote` / `accept_quote`, and `VerifiedQuote` / `QuoteRef` carry no `page` or `basis`.
- **Smallest owning package:** F4/F14a (`pdf_match.rs`: check all pages in `start_line..=end_line` and skip `\n\x0c\n` in hyphen folding; `quote_core.rs`: integrate `pdf_match` for `f14a/1` PDFs) + F6 (`quote_chip.js:53`: treat `'none'` as non-fulltext / scan-unverified).

#### Finding B-7 [Medium — Overview 2.12 Rule 2] — KaTeX math rendering on scanned PDF text in source & map views
- **Exact locations:** `crates/desktop-server/assets/canvas_sources.js:187, 197, 734, 841`; `crates/desktop-server/assets/investigation_map.js:199` (contrast `crates/desktop-server/assets/quote_chip.js:136-137`).
- **Consequence:** Overview 2.12 rule 2 (`00-overview.md:315-318`) mandates that scanned text is never silently rendered as LaTeX math. While `quote_chip.js` renders plain `textContent`, `canvas_sources.js` (`renderHighlightedReadingInto` / `renderRichInto`) and `investigation_map.js` (`renderMathTextInto`) render `.extracted.txt` lines and anchor excerpts through KaTeX without checking `pdf_pages` scan basis.
- **Smallest owning package:** F6 / V14 (`canvas_sources.js`, `investigation_map.js`).

---

## 4. Hypotheses Checked and Ruled Out (Verified Clean)

1. **`\x0c` line-count drift between Rust and JS (`paper_lines.rs`, `investigation_anchor.rs`, `routes_tools.rs`, `canvas_sources.js`):** Checked every reader. None use `.lines()` where `\r` or `\x0c` would desynchronize indices; Rust `VerifiedFile::from_bytes` (`b'\n'`), `split('\n')`, and JS `text.split('\n')` (`canvas_sources.js:801`) stay 1-based aligned across `\x0c` and blank pages.
2. **Legacy pre-F14a PDF checksum breakage:** Verified `extract_pdf_bytes_legacy` (`pdf.rs:134-211`) matches pre-F14a `extract_pdf_bytes` and passes `pdf_core::legacy_receipt_still_verifies_without_f14a_fields`.
3. **Session fork losing F14a receipt fields:** Verified `crates/export/src/fork.rs:77, 158` copies `source.ingested` JSON payloads intact.
4. **Relocated quote bypassing `lines_sent`:** Verified `quote_core.rs:239-245` checks `sent_lines.covers` against `verified.quote_ref.start_line..=end_line` (the relocated span, not the model's claimed span), covered by `quote_core_tests.rs:189-193`.

---

## 5. Concrete V6 & F5 Integration Recipe (Without Breaking Frozen `tool_defs.txt`)

To unblock `inv-v6-tasks`, `f5-f7-mode-shell`, and `inv-v17-pdf-skeleton` cleanly:
1. **Keep `tool_defs.txt` untouched:** Register `read_map_node` and `record_map_reading` (`EffectClass::ReadWorkspace`) in `investigation_extra_defs()` (`crates/research-tools/src/investigation_markers.rs` / new sibling module), keeping `research_tool_defs()` frozen at 22 tools (`tool_def_golden` stays green).
2. **Enable `investigation/1 -> investigation/2` resume (`tool_profile.rs` + `resume_profile.rs`):** Add `upgrades_to: Option<&'static str>` (`investigation/1 -> Some("investigation/2")`) and `retired_defs: &'static [&'static ToolDef]` on `ToolProfile`; include `"record_context_cards"` and `"record_prior_work_check"` in `investigation/2.may_gain`; distinguish unstarted `[investigation.started]` (no `session.start`) from pre-F1 `[investigation.started, session.start]` in `profile_for_session_events`.
3. **Wire CLI profile parity (`cli-route/src/core_tools.rs` + split `cli_turn.rs`):** Pass `CORE_TOOL_PROFILE` to `core.exe`, validate `cli.init` against that profile's MCP tool list, and record `"tool_profile"` on CLI `session.start`.
4. **Task-scoped quote proof in V6 (`record_map_reading`):** Compute `sent = lines_sent_with_ledger(&events[task_req_idx..], Some(ledger), Some(&run_id))` on the post-`task_requested` slice and pass `&sent` + full `&events` to `accept_quote_with_sent`; fill `credit_read_map_node_result` in `lines_sent.rs` (skipping `truncated == true` partial tail lines); emit `context.supplied` **after** transport/CLI resolution with the exact `caller` wire string (`"agent:<wire_provider>:<sid>"` or `"claude-code-cli:<sid>"` / `"antigravity:<sid>"`).

---

## 6. Decisions Taken Alone, Assumptions, and Merge Notes

### Decisions Taken Unattended
1. **Strict read-only audit:** Per `agents/audit-mode-foundations.md` (*"Change only your report at `reports/audit-mode-foundations.md`"*), left all Rust/JS/test files untouched, even for trivial test fixes (`pdf_preview_browser.py`, `pdf_pages_tests.rs`).
2. **Scoped report to actionable plan risks:** Prioritized defects that block V6, V17, F5, Set R, Set P6, or corrupt user-visible provenance/PDF navigation, and summarized ruled-out hypotheses separately in Section 4 so downstream briefs do not re-investigate clean paths.
3. **Test runner environment isolation:** Ran workspace `cargo test` with `RE_SEARCH_PYTHON` unset (`crates/cli/tests/desktop_and_ext.rs` spawns long-lived processes when set on Linux, as noted in `HANDOFF.md:105-106`), and ran `RE_SEARCH_PYTHON=$(command -v python3) cargo test --locked -p export --no-fail-fast` separately so every Python-backed export/fork integrity test ran and passed.

### Assumptions
- Assumed Overview 8.3 Q8 (`00-overview.md:865`, accepted 2026-11-25) applies to `investigation/2` either in V6 (requiring `retired_defs` for `search_formal_library`) or in V19 alongside old stage-recorder retirement; flagged both options in Finding A-2.
- Assumed arXiv's `/e-print/<id>` endpoint continues serving single-file `.gz`-compressed PDFs (`ArchivePayload::RawPdf`, already handled in `arxiv.rs:527-540` and `ingest.rs:433-439`), making Finding B-2 reachable on standard PDF-only arXiv papers.

### Merge Notes
- **Files added:** `docs/design/modes-2026-11/reports/audit-mode-foundations.md` (this file only; zero conflict with V2–V5, F5/F7, V6, or V17).
- **Recommended follow-up ownership before/during V6 & V17:**
  - `f1-followup` (`tool_profile.rs`, `resume_profile.rs`): Finding A-1, A-2 (`upgrades_to`, `retired_defs`, unstarted-session `profile_for_session_events`).
  - `f5-f7-mode-shell` (`turn_runner.rs`, `cli-route/src/core_tools.rs`, split `cli_turn.rs`): Finding A-2 (`lean_router` mode gate), A-3 (subscription CLI profile parity).
  - `f4-f9-followup` (`lines_sent.rs`, `quote_core.rs`, `run_budget.rs`): Finding A-4 (`after_seq`), A-6 (`fs_read`/`search_paper_passages` partial-line bounds, `true_text` redaction), A-7 (`cli.result` model-call counting).
  - `inv-v6-tasks`: Finding A-2 (`may_gain` includes `record_context_cards`), A-4 (`accept_quote_with_sent` slice recipe), A-5 (`context.supplied` post-transport `caller`), A-6 (`credit_read_map_node_result`), A-7 (`investigation_markers.rs` + `investigation_view.rs`).
  - `f14a-followup` / `inv-v4` / `inv-v17`: Finding B-1 (`pdf_preview_browser.py`, `pdf_pages_tests.rs`), B-2 (`ingest.rs` decompressed PDF `raw_blob`), B-3 (64 MiB cap & `page_range` in `ingest.rs`/`pdf_firstpage.rs`/`original_pdf.rs`), B-4 (`investigation_outline.rs` + `skeleton/pdf.rs` trailing `\x0c` trim), B-5 (`pdf_pages_scan.rs`), B-6 (`pdf_match.rs` + `quote_core.rs` + `quote_chip.js`).

---

## 7. Verification Commands, Counts, Pre-Existing Lints, and Linux Limits

### Commands Run & Exact Counts on `c5c89e64806c2430440ef4cd05b1b7fa86274078`
1. `cargo test --locked -p research-tools --test tool_def_golden --test tool_profile_golden --test pdf_core --test pdf_core_bench_and_match --test ingest_archive_pdf --test investigation --test paper_lines --no-fail-fast` -> **35 passed, 0 failed, 0 ignored** (`ingest_archive_pdf` 6, `investigation` 9, `paper_lines` 7, `pdf_core` 5, `pdf_core_bench_and_match` 2, `tool_def_golden` 3, `tool_profile_golden` 3).
2. `cargo test --locked --lib -p research-tools -p agent -p cli-route -p desktop-server --no-fail-fast` -> **306 passed, 0 failed, 0 ignored** (`research-tools` 128, `desktop-server` 72, `cli-route` 57, `agent` 49).
3. `cargo test --locked -p agent --test resume_loop --no-fail-fast` -> **6 passed, 0 failed, 0 ignored**.
4. `cargo test --locked -p desktop-server --test eval_set_r --test server_api --test investigation_flow --test audit_flow_readers_and_pages --no-fail-fast` -> **18 passed, 0 failed, 0 ignored** (`eval_set_r` 5, `server_api` 5, `investigation_flow` 4, `audit_flow_readers_and_pages` 4).
5. `RE_SEARCH_PYTHON="$(command -v python3)" cargo test --locked -p export --no-fail-fast` -> **8 passed, 0 failed, 0 ignored** (`export_integrity` 4, `fork_diff_export` 4).
6. `cargo test --locked --workspace --no-fail-fast` -> **771 passed, 0 failed, 2 ignored** across all workspace targets (the 2 ignored are live-provider tests in `crates/agent/tests/eval_harness.rs`; `paper_read_tests.rs:227` passed because the runner is non-root).
7. `for f in crates/desktop-server/assets/*.js; do node --check "$f"; done` -> **0 syntax errors** across all asset JS files.
8. `node --test crates/desktop-server/tests/*.test.mjs` -> **157 passed, 0 failed, 0 skipped**.
9. `cargo clippy --locked -p tools -p research-tools -p agent -p cli-route -p mcp-server --lib --bins -- -D warnings` -> **0 warnings**; `cargo clippy --locked -p desktop-server --lib --bins -- -D warnings -A clippy::manual_unwrap_or` -> **0 warnings**.
10. `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` -> **Exit 0** (`No touched source file is over 600 lines.`).

### Pre-Existing Clippy Warnings on Untouched Base Commit `c5c89e64806c2430440ef4cd05b1b7fa86274078` (Rust 1.100.0 on Linux)
Running `cargo clippy --locked --workspace --all-targets -- --no-deps` on the clean base commit before any edits reported 6 warnings (left untouched per `RULES.md`):
1. `crates/desktop-server/src/evidence.rs:744` — `clippy::manual_unwrap_or` (documented in `RULES.md`)
2. `crates/agent/src/guardrails.rs:335` — `clippy::items_after_test_module` (fires under `--all-targets` with Rust 1.100)
3. `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` — `unused_imports` (`BufRead`, `BufReader`, gated under `cfg(windows)` consumers)
4. `crates/research-tools/tests/sandbox_and_fs.rs:6, 9` — `unused_imports` (`is_symlink_or_junction`, `TEX_AUDIT`, gated under `cfg(windows)` tests)
5. `crates/agent/tests/eval_support/manifest.rs:289` — `clippy::match_like_matches_macro`
6. `crates/agent/tests/eval_support/sheet.rs:102` — `clippy::unnecessary_map_or`

### What Linux Cannot Compile or Confirm
- **`cfg(windows)` modules & desktop shell:** Windows AppContainer / JobObject sandboxing (`crates/sandbox` Windows backend), DPAPI/Windows vault locking (`crates/chatgpt-auth` Windows paths), and WebView2 native window hosting (`crates/desktop`).
- **Playwright + built PDF.js viewer (`crates/desktop-server/tests/pdf_preview_browser.py`):** Verified analytically against `marked_pdf()`, `original_pdf.rs`, and `canvas_sources.js`, but not executed in a live Chromium/WebView2 browser with bundled PDF.js canvas pixel sampling.
- **Set P6 real-corpus evaluation (`00-overview.md` Section 7):** Per `audit-mode-foundations.md`, user libraries and external papers were excluded; scan heuristics (`pdf_pages_scan.rs`) and ligature/hyphen folding (`pdf_match.rs`) were audited against parser internals and synthetic fixtures rather than external publisher/scan PDFs.
