# Independent review of the audit findings (coordinator, Claude reviewers)

Read-only second opinion on each audit finding, judged against `origin/gemini/work/merged`, the designs and the
README decisions. Verdicts: **real** = fix it; **by_design** = the designs ask for this, do not change production
code; **not_a_bug** = the auditor misread, do not change production code; **unclear** = leave, report.
`fix risk` says what a careless fix would break. `Missed` lists bugs the auditors did not report: treat as findings.

## audit-honesty

Of the 15 findings in the pushed report, 10 hold up. Only F1 is serious: worker and Canvas reads, told apart only by the `caller` field the router already logs, turn Orchestrator answer cites into read:true. That breaks README decision 6, so I rate it major, not critical. F3, F4, F5 and F6 are real but minor: F3 and F4 need contrived inputs, and F5 and F6 still fail closed or leave the pending items visible. F7, F10, F11, F14 and F15 are real nits. F2 and F12 are by design, because design 01 §2.1 spells out `[min(s, start-3), start-1]` and `quote` 12–300 chars. The auditor's F2 fix would contradict the design, and its F12 fix would break resume via def_hash drift unless legacy hashes are added. F13 is by design because §2.5 fixes the return shape. F8 is not a bug: request overrides are the existing pattern for every role, and only the token-authenticated user sends them. F9 is not a bug: it needs tampering with the sources directory and only ever drops cites. On fixes: F10 must not be fixed by marking cites unread, F14 must not drop the app's pending note, and F1 should use a denylist of worker: and user: callers rather than an allowlist of main-agent prefixes. I also found three more ways false read:true can arise (missed list): grep matches on lines over 500 chars, search_paper_passages excerpts cut short, and the `tool.is_none()` fallbacks. All of this is read-only review: I did not run the proof tests or build anything.

### F1: **real** (claimed critical, judged major)

- Where: `crates/desktop-server/src/investigation_cites.rs:66-90`
- Why: collect_received_ranges builds `requests: HashMap<call_id, tool>` and never looks at the `caller` field. The router logs that field on every tool.request (tools/src/router.rs:136). Worker calls (`worker:{role}:{provider}:{session}`, delegate.rs:410) and Canvas calls (`router.try_call("user:canvas", ...)`, routes_tools.rs:303) land in the same session ledger after request_seq. So a delegate_literature worker's fs_read makes `[L20]` count as `read:true` even though the Orchestrator never saw that line. This breaks README decision 6 ("only for lines the model actually received") and the Investigation prompt rule "read the cited lines yourself". Question turns keep the workers on, because `paper_reading_turn` (turn_runner.rs:294) excludes them.
- Fix risk: The auditor's allowlist (`agent:`, `claude-code-cli:`, `antigravity:`) fails closed but will silently turn every cite to plain text on any other main-route caller prefix, current or added later. A denylist of `worker:` and `user:` callers is safer. Dropping the `tool.is_none()` fallbacks is fine. Keep the D9 sources (fs_read, grep matches, search_paper_passages, read_paper_lines) and do not loosen the request_seq and primary_path filters.

### F2: **by_design** (claimed critical, judged none (design question))

- Where: `crates/research-tools/src/investigation_anchor.rs:396`
- Why: The code `above_lo = s.min(final_start.saturating_sub(LABEL_ABOVE_RADIUS))` is design 01 §2.1 step 8 word for word: "look for a unique occurrence in `[min(s, start-3), start-1]`". The label has to be a unique literal string inside the model's own cited range, and the 64-line cap still applies afterwards. The tension with step 6's "16-char quote can't ground 64 lines" is real, but it is in the design, not a code defect. The split-1 report's prose is what is wrong.
- Fix risk: The proposed `final_start-3` window would reject labels the design allows (theorem header 4 or more lines above the quoted sentence, inside the cited range). Recorders would then return label_not_found and drop anchors or items. Change the design first if this is wanted.

### F3: **real** (claimed critical, judged minor)

- Where: `crates/desktop-server/src/investigation_cites.rs:124-128,148-152; crates/research-tools/src/fs_tools.rs:338-341`
- Why: Past EOF, `if !found_start { return ("", total, total, line_no, line_no) }`. On a file with no trailing newline, line_no is the last line, so a 0-char fs_read credits that line. read_paper_lines results with `a_meta.line_truncated: true` are credited by `push_range(start_line,end_line)` with no check, although paper_lines.rs:184-188 delivered only a prefix of the line. Both are real false 'received' claims, but they need an absurd offset or a line over 14k chars. A mid-line fs_read slice counting its whole boundary lines is what D9 specifies ("fs_read start_line..end_line"), so that part is by design.
- Fix risk: If you make a truncated line count as unread, do it only for that line. Do not drop the whole result, or ordinary multi-line reads lose their read state. Skip `returned_chars == 0` and do not touch observed_total_lines handling. Dropping boundary lines on every offset>0 read would make most fs_read cites unread, against D9.

### F4: **real** (claimed major, judged minor)

- Where: `crates/research-tools/src/investigation_evidence.rs:370; crates/research-tools/src/investigation_anchor.rs:427-436`
- Why: (2) is the realistic half. verify_pointer checks the label against all final lines (`file.lines_text(final_start, final_end).contains(label)`). resolve_evidence instead checks `!derived.contains(*l)`, where derived is the ≤3,200-byte window. So a reader record whose label sits at the top of a span over 3.2 KB (theorem header plus long statement) cannot be cited by the recorder with that label. (1) is real but contrived: if the raw span of the quote is over 3,200 bytes (whitespace runs), window_around skips the `q_len <= max_bytes` branch and stores an excerpt without the full quote. That breaks design step 10 ("a window that contains the quote").
- Fix risk: Checking the label against full_lines in resolve_evidence is safe, because full_lines is re-derived from the hash-verified file. Do not relax the `derived != rec.excerpt` re-derivation check or the quote-in-derived fold check. For (1), reject the anchor; do not widen the window past 3,200 bytes.

### F5: **real** (claimed major, judged minor)

- Where: `crates/research-tools/src/investigation_conventions.rs:195-200,109; crates/desktop-server/src/investigation_scope.rs:230`
- Why: The done/limitations gate only fails when `prior_ids.is_empty() && batch.items.is_empty()` (or coverage is partial). A batch 2 with `done:true, coverage:"paper", items:[], limitations:[]` therefore finalizes a catalog whose rejected item was never resolved. paper_conventions then returns `latest.payload.clone()`, which drops the app note "N items awaiting valid citations". That defeats README decision 2 ("mark the catalog unfinished when some citations fail") with one more call. The UI still shows "1 citation pending", so the user is not misled outright; resume is blocked by the design's done gate. Dropping limitations from earlier batches is pre-existing behavior (main also clones latest.payload).
- Fix risk: Forcing done=false while pending_items exist can trap a run: `finish` requires a done catalog when conventions_requested, and the model may never manage to cite the item. Give it an explicit way to abandon pending items, recorded as a limitation. Keep the section-upgrade logic (coverage_scope/paper_resume) intact.

### F6: **real** (claimed major, judged minor)

- Where: `crates/desktop-server/src/investigation_scope.rs:181-205; crates/desktop-server/src/paper_reader.rs:188`
- Why: clip_to_selected_pages only filters on `a <= last_page && b >= first_page` and keeps `[t.start_line, t.end_line]` and front_matter unchanged. The prompt therefore points the model at lines on pages the user did not select, and plan_slices ignores selected_pages whenever `scope_lines.is_some()`. Recorders and note_evidence still fail closed with outside_selected_pages, so this is wasted reader tokens and an off-selection answer basis, not a fabricated anchor.
- Fix risk: Clamping requires the verified page_lines from compute_scope. Raw receipt pdf_page_lines must not be used for this. If the intersection is empty, return Unresolved rather than an empty range, or plan_slices could fall back to the whole paper.

### F7: **real** (claimed major, judged nit)

- Where: `crates/research-tools/src/investigation_anchor.rs:379,443`
- Why: Narrowing sets `cited_range = Some([s, e])`. Trimming plus label_extended can restore exactly `[s,e]`, but cited_range is never cleared. The UI then shows "lines corrected by the app (cited 10–13)" on lines 10–13. The design defines cited_range as "model's original range when changed". This is cosmetic.
- Fix risk: Clear only when `[final_start, final_end] == [s, e]`, and clear cited_range and the correction together so anchor_report stays consistent. Evidence records copy rec.cited_range, so old records still show the note. The JS guard alone is the lowest-risk fix.

### F8: **not_a_bug** (claimed minor, judged nit)

- Where: `crates/desktop-server/src/agent_models.rs:342; crates/desktop-server/src/paper_reader.rs:115`
- Why: Per-request role overrides are the existing convention for every role (`req.get(role).and_then(parse_role_obj)` at agent_models.rs:320 and the verifier at :251). The doc comment says "unless explicitly assigned in settings or request overrides". The request comes from the token-authenticated local client, which means the user, and no UI asset sends `paper_reader`. An explicit per-turn assignment is consent in the sense of decision 4.
- Fix risk: Removing it only for paper_reader makes it inconsistent with the other roles and may break API/CLI clients that set it.

### F9: **not_a_bug** (claimed minor, judged nit)

- Where: `crates/desktop-server/src/investigation_cites.rs:239,262-290; crates/desktop-server/src/paper_reader.rs:422-431`
- Why: This only matters if someone tampers with the app-managed sources directory mid-turn. The result fails closed: `total_lines.or(observed_total)` can only drop cites, and `read` is computed solely from tool.result ranges. prepare_for_turn uses the unverified text only to plan slices; readers then read through read_paper_lines, which is hash-pinned. A hash check would be nice hardening but is not an honesty failure.
- Fix risk: If added, a sha mismatch must fall back to observed_total, not to `None` with no limit. Otherwise out-of-range cites would stop being filtered.

### F10: **real** (claimed minor, judged nit)

- Where: `crates/desktop-server/src/investigation_cites.rs:254-256; crates/desktop-server/src/routes_tools.rs:163; crates/desktop-server/assets/investigation.js:43-48`
- Why: A received span such as [L1–550] becomes a `.line-cite` button. openCite passes start/end straight to openSource, and the window endpoint rejects `last - first > 512`, so the click shows an error. This is UX, not honesty: those lines really were received.
- Fix risk: Do not fix it by marking such cites read:false. That would mix up "received" with "openable" and weaken decision 6's meaning. Clamp in openCite, or open start..start+512 and show a note.

### F11: **real** (claimed minor, judged nit)

- Where: `crates/desktop-server/assets/investigation_map.js:42-66`
- Why: mathSpans opens inline math at any `$` followed by a non-space and has no currency rule. math_scan.js's findClosingInlineDollar has isCurrencyBody and a digit-after-close rule; mathSpans has neither. So `$5 ... 0$` becomes a span and foldSearchSpan widens `<mark>` over unquoted prose. The renderer itself (math.js) would treat `$5` as currency, so highlight and rendering disagree. The highlight is a visual aid, not a verification claim.
- Fix risk: The excerpt pieces still go through renderMathTextInto (math()); keep that path and never build the mark with innerHTML from excerpt text. Reusing findClosingInlineDollar is the minimal change.

### F12: **by_design** (claimed minor, judged nit)

- Where: `crates/research-tools/src/investigation_anchor.rs:41`
- Why: Design 01 §2.1 sets the schema explicitly: "`quote` (12–300 chars)", changed once so that def_hash changes only once. The 3-char whole-line exception lives in the runtime verifier. The mismatch with note_evidence_def (minLength 3) comes from the design, not from an implementation mistake.
- Fix risk: Changing anchor_schema changes the record_paper_* def_hash. Without adding the old hashes to investigation_legacy_defs earlier_defs, every recorded Investigation session fails resume drift checks.

### F13: **by_design** (claimed nit, judged nit)

- Where: `crates/agent/src/paper_read.rs:320-322`
- Why: The return shape `{e,l,pg,k,r,lab?,q,g}` is specified verbatim in design 01 §2.5, without source_id. resolve_evidence fails closed on `rec.source_id != anchor.source_id`. A wrong `pg` on a non-primary evidence row is at most a nit.
- Fix risk: Adding fields changes the delegate_read_paper return that resume replays from raw_blob; old blobs are fine, but tests that pin the shape will need updating.

### F14: **real** (claimed nit, judged nit)

- Where: `crates/research-tools/src/investigation_conventions.rs:131,294-302`
- Why: Input is capped at `batch.limitations.len() > 12`, but `batch.limitations.push(note)` can store 13, and `"{} items awaiting..."` prints "1 items". Nothing visible enforces 12 on stored payloads.
- Fix risk: Do not satisfy the cap by dropping the app note. It is the honesty signal for decision 2. Replace the model's last limitation, or exempt app notes from the cap.

### F15: **real** (claimed nit, judged nit)

- Where: `crates/desktop-server/src/investigation_scope.rs:73,92`
- Why: build_scope_label (investigation_outline.rs:541) and clip_to_selected_pages:196 already format a single target as `"{label} (lines a–b)"`, and both suffixes append `({range_str})` again. The result is "Section II (lines 51–110) (lines 51-110)". The change is cosmetic, inside the prompt only.
- Fix risk: The label is also stored in investigation.scope and shown in the UI. Fix it in the suffix, not by changing the stored label, or the UI and tests that read `label` will change.

### Missed by the auditor

- **M1**: crates/desktop-server/src/investigation_cites.rs:142-146 with crates/research-tools/src/fs_tools.rs:384,402,419-434: grep-mode fs_read credits `matches[].line` as fully received, but each match text is `excerpt_around_match(line, ..., 500)`. On a line over 500 chars the model saw at most a 500-char window, yet the cite is read:true. This is the same false-receipt class as F3, and more likely to happen than the past-EOF case.
- **M2**: crates/desktop-server/src/investigation_cites.rs:95-110: search_paper_passages ranges are credited whole, but paper_passages::next_excerpt caps each excerpt at MAX_EXCERPT_BYTES/MAX_EXCERPT_LINES and may cut an oversized physical line at whitespace. If the passage's start_line/end_line covers the full lines while the text was cut, the cut tail counts as received. Worth checking in paper_passages.rs before fixing F3.
- **M3**: crates/desktop-server/src/investigation_cites.rs:86-90,112,132: when a tool.result has no matching tool.request after request_seq (`tool == None`), any result object with `path == primary_path` and start_line/end_line is credited, including results of tools outside the fs_read/read_paper_lines/search list. The `tool.is_none()` fallbacks should be removed together with the F1 caller filter.

## audit-consistency

Of the 12 findings, 7 are real: C-01, C-02, C-03, C-04 (item 1 only), C-05 (items 1-2 only, and item 2 is a consequence of C-02), C-06 and C-10. C-09 is real but pre-existing on origin/main. C-11 is a real docs nit. C-04 item 2 (the '$ does not render' sentence) and C-12 are by_design: design 02 D3 prescribes that sentence, and README LaTeX decision 8 defers the auto-render removal. C-07 and C-05 item 3 are not bugs: the chip tracks the map/question scope, and a checkbox matches decision 7's 'opt-in setting'. C-08 is outside the spec: the design only specifies the stopped status for mapping, and its investigation.result half predates the merge. The most important fixes are C-02 and C-06. C-02: conventions_expanding_to_paper ignores a null or unfinished catalog, so a section catalog can never be built, and the fix must write an explicit scope:"section" or the recorder loses coverage_scope. C-06: question-only sessions cannot hand off to Research, against design 02 P1b. For C-01, the router, evidence.rs and linked_sources changes must land together, with sha-pinned, scrubbed excerpts. None of the real findings weakens the design 01 §2.7 honesty guarantees, because SelectedPages, the marker and the quote checks still hold. The proof tests for C-05 item 2 and C-07 assert behaviour that no design requires.

### C-01: **real** (claimed major, judged minor)

- Where: `crates/tools/src/router.rs:401-410; crates/desktop-server/src/evidence.rs:39-42,677-686; crates/agent/src/verifiers.rs:294; crates/agent/src/delegate.rs:119,134`
- Why: read_paper_lines is in the main registry, but Router only injects tool_call_ref for `"sympy_eval" | ... | "fs_read" | ... | "search_paper_passages" ...`, and evidence.rs maps any other tool name to `_ => "unavailable"` / `unsupported_reference`. So a Research or claim-check Proof DAG entry cannot cite a line read. Today that path is nearly unreachable, because the model never sees the router call_id. The worker allowlists (delegate.rs:119/134) and the verifier source context also leave it out. The prompt part (item 5) is only a nit: design 02 D3 specifies the system-prompt wording, and paper_map_instructions already names read_paper_lines.
- Fix risk: If only router.rs is fixed, models can cite read_paper_lines refs, and every such entry then shows 'Unresolved evidence link'. Router, evidence.rs and linked_session_sources must land together. The evidence handler must take its excerpt from the recorded result and check a_meta.file_sha256 against the source receipt, as source_read_link does for fs_read, and the text must go through self.scrub.field before the UI puts it in innerHTML. Adding read_paper_lines to the compute/lean profiles is not asked for.

### C-02: **real** (claimed major, judged major)

- Where: `crates/desktop-server/src/investigation_scope.rs:234-245 (also :73); crates/desktop-server/src/investigation.rs:304-323`
- Why: `conventions_expanding_to_paper` returns true when `catalog["coverage_scope"] == "section"` OR the mapping_requested has `conventions_scope == "section"`. It never checks `catalog.is_null()` or `catalog["done"] == true`, although the gate on the line above (`done != true || coverage_scope == section`) and design 01 section 2.6 allow paper expansion only from a finished section catalog. Two results follow. With conventions off in the first pass (README UX decision 1), the first click on a section-scoped map writes `scope:"paper"` and sends 'Expand the section conventions catalog… Prior batches: null', so a section catalog is never built. An unfinished section catalog (pending_items) is also forced to whole-paper coverage. Item 3 is right too: `record conventions for the scope plus global notation` contradicts D4 / `conventions_requested:false`. Design 01 section 2.6 has that sentence, but the later user decision supersedes it.
- Fix risk: The recorder takes coverage_scope from the conventions_resumed marker (investigation_conventions.rs:150-156; 'section' only if `active_marker.payload["scope"]` says so). A fix must therefore write an explicit `scope:"section"` on the first or continuing resume. If it does not, the batch gets no coverage_scope, and conventions_finish_done / the 'Catalog all conventions' button stop working. Do not loosen the `done==true` finalized check in the recorder.

### C-03: **real** (claimed major, judged minor)

- Where: `crates/desktop-server/src/investigation_scope.rs:182-207; crates/desktop-server/src/paper_reader.rs:188-198 (plan_slices); investigation_scope.rs:73,92`
- Why: clip_to_selected_pages only filters targets by page overlap (`a <= last_page && b >= first_page`) and keeps `[t.start_line, t.end_line]` unchanged. plan_slices skips page filtering whenever `scope_lines` is Some (`if scope_lines.is_none() && !page_lines.is_empty()`). Readers and the model are therefore sent to lines that SelectedPages will refuse: wasted turns and outside_selected_pages faults, with no honesty loss. The label already contains `(lines a–b)` (build_scope_label / clip path), and the suffix appends `({range_str})` again, so the prompt repeats the range (cosmetic).
- Fix risk: Clip ranges to the selected pages using the verified page_lines. Never widen them, and never drop SelectedPages::check_range in note_evidence/recorders to make page-1 lines 'work'. If clipping empties a target, return Unresolved, not a 1..total fallback.

### C-04: **real** (claimed major, judged minor)

- Where: `crates/agent/src/autonomous.rs:91-97; crates/agent/src/delegate.rs:189-193; crates/desktop-server/src/turn_runner.rs:294-296`
- Why: Item 1 is real. format_delegation_prompt_section always prepends 'Act as the high-level Orchestrator: … (`ledger_append` with `entry_type: "result"` …), and synthesize the `final_answer`'. Because a question turn is not a `paper_reading_turn`, the section is frozen into session.start of every Investigation session that starts with a question. Design 01 section 2.4 says that push 'must be gated off for Investigation sessions at session.start', and README UX decision 3 says no proof-graph duty. Item 2 (the '$…$ does not render' sentence) is by_design and at most a stale nit: design 02 section 2.4 D3 prescribes exactly 'Use \(...\) and $$...$$ … because single $ does not render', and following it still renders correctly.
- Fix risk: Gate only the Orchestrator/final_answer lead-in for Investigation sessions. Do not strip the honesty Rule 3 text, and keep the Research-mode prompt and its drift/resume behaviour unchanged: resume reuses the stored prompt, and autonomous.rs:216 skips the section when 'Orchestrator -> Worker Delegation:' is already present, so a changed header can duplicate the section on resume.

### C-05: **real** (claimed major, judged minor)

- Where: `crates/desktop-server/src/investigation.rs:253-262; crates/desktop-server/src/paper_reader.rs:127-139,396-420; assets/subagents.js:400-408`
- Why: Item 1 is real but low impact. `_paper_scope` is missing from the strip list, and nothing on the server ever writes it, so it can only come from the client. It can only switch readers on or move slices; the client can already send `paper_readers_enabled` directly (paper_reader.rs:151), and note_evidence still enforces the marker and SelectedPages. Item 2 is real only as a consequence of C-02: conventions_resumed never carries a section scope or ranges, so `m["conventions_scope"]`/`m["scope"]["resolved"]` cannot match, and scope_lines falls back to the whole paper. The diagnosis that the check uses 'conventions_scope instead of scope' is wrong, because `scope` there is the string "paper". Item 3 is not_a_bug: README decision 7 asks for 'an opt-in setting', and a checkbox for always-on vs automatic does exactly that.
- Fix risk: When fixing item 2, copy the map's resolved ranges into the conventions_resumed payload (or look them up by for_request_seq). Do not trust any request field. Adding a force-off UI is a scope change, not a fix.

### C-06: **real** (claimed major, judged major)

- Where: `crates/desktop-server/src/investigation.rs:56 (selection→current_map); crates/export/src/fork.rs:27-29`
- Why: Design 02 P1b requires that 'a session whose only answer is a question turn still opens a handoff Research session'. Instead, `latest(events, "investigation.map").ok_or_else(|| bad("Map the paper first."))` and fork.rs's `"Paper map revision missing."` block it. ux-rust left this undone.
- Fix risk: The handoff context should carry only the primary source receipt and cites with `read:true` (the honesty rule for clickable citations, README decision 6), not unverified [L…] ranges or model prose presented as evidence. It must keep fork.rs's hash pinning of whatever event it references (e.g. answer seq + event_hash), and must not loosen the map_hash check for map-based handoffs.

### C-07: **not_a_bug** (claimed minor, judged nit)

- Where: `crates/desktop-server/src/investigation_view.rs:105-112`
- Why: The chip shows the scope of the latest map or question request (`mapping_requested | question_requested`), and the map stays section-scoped after conventions are expanded. Conventions coverage is labelled separately ('· section catalog' in investigation_conventions.js:43). No design asks the chip to follow the conventions coverage.
- Fix risk: Tying the chip to conventions_resumed would make it say 'whole paper' while the map and answers are still section-scoped.

### C-08: **unclear** (claimed minor, judged nit)

- Where: `crates/desktop-server/assets/investigation_state.js:24-32,92-100; crates/desktop-server/src/investigation_finish.rs:62-67`
- Why: Design 02's status table only specifies 'Mapping stopped' ('Stopped. Anything already saved is kept below.'), which isMappingStopped implements. The design does not cover a stop during conventions, branch or check turns, so the empty status there is a gap in the spec, not a contradiction. Item 2 (`Err(t) => ("failed", t)` for `_paper_request`) is unchanged from origin/main (investigation.rs:634), so it is pre-existing.
- Fix risk: Do not mark the conventions catalog done or rewrite investigation.result outcomes after the fact; only the status copy should change.

### C-09: **real** (claimed minor, judged pre_existing (minor))

- Where: `crates/desktop-server/src/session_summary.rs:132-135`
- Why: `u.get("input_tokens")`/`u.get("output_tokens")` reads raw provider usage, while openai.rs:329 stores Chat Completions `usage` (prompt_tokens/completion_tokens) and gemini.rs:271 stores `usageMetadata`. As a result the totals are 0 for those orchestrators. This code is not touched by the merge (no diff vs origin/main), so it is outside this merge's scope.
- Fix risk: Low. Reuse the same extractor delegate.rs uses. Do not double-count Responses-API usage, which already uses input_tokens.

### C-10: **real** (claimed minor, judged minor)

- Where: `crates/desktop-server/assets/source_links.js:326-360`
- Why: sessionSourcesUsed counts map, summary, conventions, handoff and proof-entry anchors, but never `investigation.answers[].cites`. A question-only Investigation, which is now the default path, therefore shows an empty 'Cited in this session' even when it has verified [L…] cites.
- Fix risk: Count only cites with `read === true` (what the app verified the model received). Do not parse [L…] from the answer text.

### C-11: **real** (claimed minor, judged nit)

- Where: `docs/reference.md:99-103; README.md:26; crates/desktop-server/assets/guide.js:16`
- Why: These files are unchanged by the merge. reference.md still says 'The first reading gives a short What, Why and How summary…', which is wrong for question-first turns (README UX decisions 1-2). The THREAT-MODEL.md:225 'Stage 6 adds…' line is a historical description and not wrong as written.
- Fix risk: None for code. Do not claim reader or citation guarantees beyond design 01 section 2.7.

### C-12: **by_design** (claimed nit, judged nit)

- Where: `crates/desktop-server/assets/index.html:765; crates/desktop-server/src/assets.rs:255`
- Why: README LaTeX decision 8: 'Remove auto-render.min.js later (WP6), together with index.html, assets.rs and the browser test.' No caller of renderMathInElement remains, but the file is deliberately deferred.
- Fix risk: Removing it without the browser test and CSP/asset updates breaks the deferred-together plan.

### Missed by the auditor

- **M1**: crates/desktop-server/src/paper_reader.rs:396-420 (prepare_for_turn scope_lines): for a multi-target scope, `min_s`/`max_e` over all `ranges` collapse disjoint ranges (e.g. Section 2 + Appendix B) into one span that covers everything in between. The readers then read the whole intervening paper, which defeats section scoping (decision 7) and costs reader turns.
- **M2**: crates/desktop-server/src/investigation_scope.rs:189-206 (clip_to_selected_pages): when targets are dropped for lying off the selected pages, `ranges` and `targets` are rebuilt but the original `front_matter` is kept unclipped. On a PDF page selection that excludes page 1, the prompt still says 'take the summary from front matter (front matter lines 1-N)', and those lines fail SelectedPages.
- **M3**: crates/desktop-server/src/paper_reader.rs:151 (should_enable_readers): the per-request body field `paper_readers_enabled` overrides the user's saved setting, and prepare() does not strip it. The local UI is the only client, so impact is low, but a forced `false` from the client silently disables readers that decision 7 would turn on.

## audit-ui F1-F9

I judged F1-F9 against origin/gemini/work/merged and ran the latex proof tests with node only (no build). All 7 latex proofs for F1-F7 fail as the auditor reported, and the F8/F9 tests assert real requirements. All nine findings are real, but four of the auditor's four 'critical' ratings are too high: F1-F4 are major, F5-F7 are minor, F8 is minor (only part 1 is a bug; part 2, the missing macro reset on reconnect, matches design 03), and F9 is major. Two of the auditor's proposed fixes would cause harm. F3's 'replay through KaTeX' fails because vendored KaTeX 0.19.1 rejects `\\newcommand[2][2]` (I probed it). F4's `para.push(rest)` would break the designed `$$$$` close-and-reopen behaviour pinned by math_parity case 18. For F8, resetting macros inside renderSessionStream would undo the per-formula cache during live turns.

### F1: **real** (claimed critical, judged major)

- Where: `crates/desktop-server/assets/markdown.js:284-299 (matchBlockDisplay at :107-113)`
- Why: I reproduced it with node. The closing `$$` sits at the start of a line, so `/^ {0,3}\$\$/` in matchBlockDisplay treats it as a new opener, and `findDisplayClose` pairs it with the next equation's opener. Every later display then flips: prose becomes math and the formulas stay as text with literal `$$`. Design 03 asks that a mid-paragraph `$$…$$` become a `span.math-display`. main pairs these delimiters correctly (it extracts math before block parsing), so this is a regression. The output is visibly broken rather than silently wrong, which is why I rate it major, not critical.
- Fix risk: Deciding whether `para` holds an unclosed opener must reuse extractMath's own scan state: escape-run check, currency guard, `$$$$`, and the stop at the next ATX heading. A plain regex would swallow `\$$` or `$5 … $$`, or join displays across a heading. Unclosed `$$` must still stay as text (design: same as Kandinsky), and during streaming a half-arrived display must still show as source.

### F2: **real** (claimed critical, judged major)

- Where: `crates/desktop-server/assets/math_katex.js:99-105`
- Why: `/\\text\{([^{}]*)\}/` matches `\text{if $x^2 < 1$ and $x_i > 0$}` because the body has no braces, and every `^`/`_` inside the nested `$…$` is rewritten to `\^{}`/`\_`. KaTeX then renders a circumflex and a literal underscore with no error, so the formula is silently wrong. Design 03 does specify the brace-free `\text` repair, but its purpose is bare text-mode `_`/`^`. Characters inside nested math are not text-mode, and before the repair KaTeX rendered this input correctly (I checked in a probe).
- Fix risk: Toggling on `$` must ignore `\$` (odd backslash run) and must also handle `\(…\)`. It must not reintroduce the text-mode `_` parse errors that the repair exists to fix. The `data-tex`/`aria-label` attributes must keep the raw source, escaped.

### F3: **real** (claimed critical, judged major)

- Where: `crates/desktop-server/assets/tex_preview.js:58-67, 77-83`
- Why: The `[argc]` and `[default]` groups are read and then dropped, and only `macros[name] = body.content` is stored. KaTeX works out arity from the highest `#n` in a string macro, so `\newcommand{\norm}[2][2]{…}` binds `+` as #2, and `[1]{}` with an unused argument leaks that argument. This affects the TeX-file preview only, not model answers. The auditor's proposed fix is wrong: I probed vendored KaTeX 0.19.1, and `\newcommand{\norm}[2][2]{…}` throws `Expected '}', got '#'`. Replaying the definitions through KaTeX therefore cannot handle optional defaults.
- Fix risk: Replaying the whole preamble in one `renderToString` call would lose every macro as soon as one definition is unsupported (optional args, delimited `\def`). Each definition needs its own try/catch with a fallback. A correct fix must store `{numArgs}`-style objects, or expand optional args itself. `\providecommand` must not overwrite seed macros.

### F4: **real** (claimed critical, judged major)

- Where: `crates/desktop-server/assets/markdown.js:294-296`
- Why: `lines[disp.closeLine] = disp.rest.trimStart(); i = disp.closeLine - 1;` sends the trailing text back through block dispatch. `$$ > 0 …` becomes a `<blockquote>`, which deletes the `>`, and `$$ - 2 = -1.` becomes a `<ul>`. Design 03 says text after a mid-line closer becomes a following paragraph. Silently deleting a relation sign makes this an honesty problem.
- Fix risk: The auditor's fix (`para.push(rest)`) would break the documented `$$$$` behaviour (design 03: closes one display and opens the next). That behaviour depends on the re-dispatch and is pinned by math_parity.test.mjs:244 (case 18). The fix needs to re-check only for a display opener, or for a `$$`/`\[`/`\begin` at the start of `rest`, and otherwise push to `para`. It must not offer list, quote, heading or table parsing.

### F5: **real** (claimed major, judged minor)

- Where: `crates/desktop-server/assets/math_equations.js:218 and :131`
- Why: `tok.prepared = `${body} \\tag{${tagText}}`` and `row = `${row.trimEnd()} \\tag{…}`` add the tag to the same line as a trailing `% comment`, so KaTeX comments out the tag. In align, the `\\` row break is commented out as well. The counter still advances, so `\eqref` points to an equation that shows no number. The TeX preview already strips comments (tex_preview.js:11-23), so only model Markdown is affected, and unescaped `%` there is uncommon.
- Fix risk: The easiest safe fix is to put a newline before the tag (`${body}\n\\tag{n}`). Any comment stripping must keep `\%` (percent signs are common in prose math), and must leave the raw `data-tex` source unchanged.

### F6: **real** (claimed major, judged minor)

- Where: `crates/desktop-server/assets/math_equations.js:173, 92-102, 213-218`
- Why: After `subequations` is stripped (:173), two `equation` blocks are not a MULTI_ROW match. normalizeSingleDisplayBody deletes both wrappers with no separator, and both labels map to the single `next()` number. The result is `a = 1 b = 2 (1)`, with two `\eqref`s reading `(1)`. This is rare in chat but realistic in the TeX preview. main also passed this input straight to KaTeX without care, so it is not a clean regression.
- Fix risk: Converting to `gather*` rows must keep the inner `\label`→tag mapping and not number `equation*` children. The existing `math_render.test.mjs:174` case (subequations wrapping one `align` with an outer `\label`) must still map the outer label to the first row.

### F7: **real** (claimed major, judged minor)

- Where: `crates/desktop-server/assets/math_equations.js:6; crates/desktop-server/assets/math_katex.js:109`
- Why: `OWN_TAG_RE = /\\tag\*?\s*\{([^{}]*)\}/` does not match `\tag{\mathrm{I}}`. The display is therefore treated as untagged and gets a second `\tag{1}`, and KaTeX shows a red 'Multiple \tag' error. stripTagCommands has the same `[^{}]*` limit, which breaks inlineOnly surfaces. Design 03 says a display that already has `\tag` keeps it. The failure is visible, not silent, and nested-brace tags are uncommon.
- Fix risk: The tag text then contains TeX. It flows into `data-eq-id` (escaped with escapeHtml, fine) and into replaceEqrefsInTex/prose `\eqref` labels, where escapeTexText or escapeHtml will show `\mathrm{I}` literally. That is cosmetic, but no path may stop escaping before innerHTML.

### F8: **real** (claimed major, judged minor)

- Where: `crates/desktop-server/assets/app.js:572 vs :584; crates/desktop-server/assets/investigation_answer.js:64-73, 102-121`
- Why: Part 1 is real. After `resetMathMacros()` (app.js:542), `showInvestigationSession(id, sess)` (:572) renders the Answer card before `renderSessionStream` (:584). The card renders only `newest` and leaves `older` unrendered (and reversed when shown), so a macro defined in Q1 shows as a red undefined command in Q2. The `signature` check then blocks the re-render after the stream has defined it. Design 03 chose persistent per-session macros. Part 2 (no reset on reconnect, turn_complete or resetToBlankState) is not a bug: design 03 asks for a reset only on session switch, and the next selectSession resets anyway because `changed` is true after the blank state.
- Fix risk: Do not add `resetMathMacros()` inside renderSessionStream. It also runs during live turns, and resetting there would clear macros the Answer card just used and wipe the per-formula cache, which brings back full re-typesetting. Rendering older answers in chronological order must keep the lazy `<details>` behaviour, or at least avoid inserting hidden HTML that skips escaping.

### F9: **real** (claimed major, judged major)

- Where: `crates/desktop-server/assets/investigation_state.js:59-60; crates/desktop-server/assets/investigation.js:82-86, 165-167, 355`
- Why: `approvalWaiting = Boolean(sess.approvalWaiting) || (Array.isArray(sess.pending_approvals) && sess.pending_approvals.length > 0)` lets the stale load-time `session.pending_approvals` override the explicit `waiting:false` that app.js:705 dispatches. app.js sends a count (approver.rs:276 `pending_for_session(...).len()`), so `waiting:false` is reliable. The status stays on 'waiting for your approval' with the timer stopped until the next `investigation.*` refetch, which can be minutes away during a reading turn.
- Fix risk: investigation.js always passes a boolean `approvalWaiting`, so preferring it is safe there. Other callers and tests of paperViewState that pass only `pending_approvals` must keep working, so prefer the explicit value only when `typeof === 'boolean'`. Do not clear `session.pending_approvals` when `waiting` is true or when another approval is still pending.

### Missed by the auditor

- **M1**: crates/desktop-server/assets/tex_preview.js:54-67: `\providecommand` is harvested exactly like `\renewcommand`, so it overwrites existing seed macros (createMathState spreads `{...SEED_MACROS, ...extraMacros}`). In LaTeX, `\providecommand` must not override a command that already exists.
- **M2**: crates/desktop-server/assets/tex_preview.js:77: the `\def` regex `([^{]*)` is not bounded to the line. A `\def\foo` with a delimited or unusual body can swallow text up to the next `{` anywhere later in the preamble, so the wrong group becomes the body.

## audit-ui F10-F18

I agree with all nine findings (F10-F18) that the code behaves as claimed on origin/gemini/work/merged, but three severities are too high and two sub-claims are not bugs. F10 and F11 drop from major to minor: both are display or silent-failure problems that never misstate what was read. F13 and F15 drop to nit. F13 sub-claim 2 is by design: 02-investigation-ux.md:124 asks 'Find main results' to send the newest question. F15 sub-claim 2 (pass-2 cache miss) is a performance detail, since Design 03 cases 44a/44b require only 'no error'. In F12 the auditor named the wrong endpoint; the failing request goes through /workspace/file?root=sources, but the 513-line HTTP 400 is real. F17 misquotes the CSS rule, but the double line break holds. Fix risks to watch: F16 must not bound the stop check at mapping_finished, because stopped mapping turns also write mapping_finished; and F11 must not show the stale mapping-era conventions_error. In F10/F14 nothing may move model or paper text into innerHTML before escaping, and the F12 fix must not loosen the backend read flag.

### F10: **real** (claimed major, judged minor)

- Where: `crates/desktop-server/assets/investigation_map.js:42-66, 74-77, 85-88`
- Why: mathSpans only recognises $, $$, \( and \[ (`text[i + 1] === '(' || text[i + 1] === '['`), so foldSearchSpan never widens a quote that sits inside \begin{equation}…\end{equation}. renderAnchorExcerptInto then splits the excerpt into pre/mark/post and renders each with renderMathTextInto, leaving raw `\begin{equation}` / `\end{equation}` text and an unrendered body. The single-$ branch also has no currency guard (`$5 and $10` becomes a span). Only display is affected: the anchor text is still verified and still escaped, so this is minor, not major.
- Fix risk: The fix must keep all three pieces going through renderMathTextInto/textContent. Never build the <mark> with innerHTML string concatenation. A widened span must still contain the verified quote, which only changes how much is highlighted. A shared math_scan span helper must not change extractMath's behaviour for markdown.js, or it would silently move the Design 03 parity cases.

### F11: **real** (claimed major, judged minor)

- Where: `crates/desktop-server/assets/investigation_conventions.js:42, 124, 142`
- Why: The backend sends `conventions_expanding` (investigation_view.rs:126), but the catalog widget never reads it: `if (!catalog.done) return (running ? …` returns no running text once the section catalog is done, and `inv.conventions_error && !running && (!catalog?.done || inv.conventions_is_current === false)` hides a failed paper expansion's error behind 'N entries · section catalog'. The header still truthfully says 'section catalog' and the global status line polls (investigation.js:403), so nothing is misrepresented. The failure is silent, which is why I rate it minor.
- Fix risk: `conventions_error` falls back to the mapping turn's `finished.conventions_error` (investigation_view.rs `.or_else(...)`). Relaxing the gate to `|| sectionDone` can therefore show a stale mapping-era error under a good section catalog. The fix should gate on the expansion request, not only on sectionDone. Adding conventions_expanding to the signature is safe.

### F12: **real** (claimed minor, judged minor)

- Where: `crates/desktop-server/assets/paper_cites.js:22,47 (with src/routes_tools.rs:163)`
- Why: buildAllowedSet accepts any read range with `start >= 1 && start <= end`, while the source window route rejects `last - first > 512` with 'Select a passage of up to 513 lines.' A read:true cite wider than 513 lines therefore becomes a button whose click fails. The auditor named the wrong endpoint: the request goes through api.readSourceFile to /workspace/file?root=sources&start_line&end_line (api.js:266-271), not /excerpt. The substance holds.
- Fix risk: Clamping openCite to the first 513 lines is the safer choice, because the button text still shows the model's range while the reader opens only part of it. Do not change the backend `read` flag or merge_ranges to make the range 'fit'. That flag is the honesty signal that the turn actually received those lines.

### F13: **real** (claimed minor, judged nit)

- Where: `crates/desktop-server/assets/investigation.js:20,38`
- Why: `createScopeChip(byId('investigation-status'))` inserts the chip right after the status line, then `byId('investigation-status').after(answerHost)` inserts the answer before it. The resulting order is status → answer → chip, which puts 'Scoped to …' under the Answer card. No design text fixes the chip's position, so this is a placement nit. Sub-claim 2 is by_design: 02-investigation-ux.md:124 says 'Find main results' 'sends the newest question as the prompt', which is exactly `onFindMainResults(String(newest?.question || ''))`.
- Fix risk: Low. Insert answerHost after scopeChip.element instead. Do not 'fix' sub-claim 2 by sending inv.question, which would contradict design 02.

### F14: **real** (claimed minor, judged minor)

- Where: `crates/desktop-server/assets/markdown.js:16`
- Why: Design 03 Decision 4 is 'strip outside code', but the fence pattern `^ {0,3}(`{3,}|~{3,})` misses fences after `> ` and fences indented four or more spaces inside list items. The inline-code pattern `(`+)[^`\n]*?\3` also disagrees with renderInlineMarkdown's `(`+)(?!`)([^\n]+?)(?<!`)\1(?!`)`, so `<!-- -->` is deleted inside code that the renderer later shows as code. This code is new in the merge (it is absent on origin/main).
- Fix risk: If stripping moves to after tokenisation, comment text must still never reach innerHTML unescaped. Today it is removed before escapeHtml, and a reordering that leaves `<!--` in the escaped stream is fine. One that leaves it after placeholder substitution is not. Also keep the Design 03 rule that comments inside formulas are stripped.

### F15: **real** (claimed minor, judged nit)

- Where: `crates/desktop-server/assets/math_katex.js:80-83`
- Why: isDefined probes `renderToString('\\' + name)` in math mode and treats 'Undefined control sequence' as undefined. KaTeX's \newcommand guard also counts text-mode symbols (\ae, \ss, \o), so `\newcommand{\ae}` errors, and Design 03 §192 asks for \renewcommand for any KaTeX built-in. This is a real but rare edge case. Sub-claim 2, the pass-2 cache miss, is not a bug: Design 03 cases 44a/44b require only 'no error either time', and an extra KaTeX call is a performance detail.
- Fix risk: Run any \newcommand probe with a fresh `macros: {}` and throwOnError:true. Never run it against state.macros, which would define or clobber user macros. Canonicalising postKey must not cache output whose macros were used before definition (`usedBeforeDefinition`), or streaming would show stale renders.

### F16: **real** (claimed minor, judged minor)

- Where: `crates/desktop-server/assets/investigation_state.js:24-31, 101-104`
- Why: isMappingStopped returns true for any `session.abort` with `stopped === true` and `ev.seq > req.seq`, unless a newer mapping or question request exists. A user stop of a later conventions-resume, develop or check turn therefore replaces the failed mapping's `data.mapping_error` with 'Stopped. Anything already saved is kept below.' That hides a still-current failure and keeps hiding it until the next mapping request.
- Fix risk: finish() writes `mapping_finished` (status failed, with an error) even when the mapping turn itself is stopped (investigation_finish.rs:92). Bounding the search at mapping_finished could therefore turn a genuine user stop into a scary error, depending on event order. Bound the search by the next `*_requested` or `conventions_resumed` event after req, as latestRequestEvent does, rather than by mapping_finished.

### F17: **real** (claimed minor, judged minor)

- Where: `crates/desktop-server/assets/investigation.css:14 (with markdown.js:100-101)`
- Why: `.investigation-inner p { margin: .35rem 0 .8rem; white-space: pre-line; }` applies to the Answer card's `.investigation-answer-body.rich-prose` paragraphs, and renderParagraphLines emits `<br>\n`. Under pre-line both the <br> and the preserved newline break the line, so every hard line break shows a blank line. The auditor quoted the CSS rule slightly wrong (`margin: 0; overflow-wrap…`), but the conclusion holds.
- Fix risk: Prefer a scoped `.investigation-inner .rich-prose p { white-space: normal }`. Dropping the `\n` in markdown.js changes markdownToHtml output shared with Research mode and with the math_parity/markdown tests. Removing pre-line globally would break other investigation paragraphs (questions, limitations) that rely on it.

### F18: **real** (claimed nit, judged nit)

- Where: `crates/desktop-server/assets/math_katex.js:222,237,253-254; markdown.js:11; investigation_map.js:36-40`
- Why: `if (threw) return glue ? …` is byte-identical to the following return, so `threw` is dead. CITATION_BODY_RE (markdown.js:11 vs math_scan.js:16, not exported) and isEscaped/isEscapedAt are duplicated. Item 4, re-wrapping .katex-error text in renderMathInContainer, is hypothetical: its only callers (stream.js:197,217) run once on fresh DOM.
- Fix risk: Pure cleanup. If renderMathInContainer gains a closest() skip, keep the CODE/PRE skip. The error branch must keep `escapeHtml(source)`.

### Missed by the auditor

- **M1**: crates/desktop-server/src/investigation_view.rs (conventions_error = catalog_finish error .or_else(finished.conventions_error)) combined with investigation_conventions.js:142: when catalog_finish has no error, a stale mapping-era conventions_error still reaches the widget. It is shown today whenever the catalog is not done or not current, even if a newer resume turn is running or has stopped without its own error. This is not a separate visible bug today, but any fix to F11 makes it visible.
- **M2**: crates/desktop-server/assets/investigation_map.js:53-57: the single-$ branch of mathSpans has no currency guard, while math_scan/extractMath has one. An excerpt such as 'costs $5 and $10' produces a fake span that widens the <mark> past the verified quote. Same root cause as F10, but it fails in the opposite direction (over-highlighting non-math text).

## Pre-existing tests weakened (main -> merged)

I found one real weakening and nothing else in the pre-existing test files. Of the 9 test files that existed on origin/main and were modified (none were deleted, none renamed), only investigation_browser.py loses assertions. The other 8 are additive or stub-only changes. I also checked tests inside src files that lost lines (resume.rs, desktop-server investigation.rs); both were moved verbatim, not dropped. No `#[ignore]`, skip markers, lowered counts, changed error codes or raised timeouts appear in the merged diff. The one real weakening: the browser test dropped its `.question-match` assertions, including the check that the badge sits on the matched statement (lines[1]). The design keeps that badge, so those assertions should be restored. I read diffs only and ran nothing. I did not verify that the merged tests pass.

### W1: **real** (claimed n/a, judged low)

- Where: `crates/desktop-server/tests/investigation_browser.py:~235 (main flow after refreshInvestigation)`
- Why: Two assertions were deleted: chart .question-match contains 'Selected for your question', and .question-match contains lines[1] (the matched statement text). The design (02) does not remove the 'Selected for your question' badge; investigation_graph.js and investigation_map.js still emit it, and the 'answer_node_ids' selection is kept. The new question-first test only re-asserts the badge text on '.investigation-index'. It does not check that the badge sits on the right node (lines[1]) or the .question-match class. Part of the loss is justified because the main flow now sends the fixed prompt 'Identify the paper's main results.', so the old question text no longer applies. The node-level assertion should have been rewritten for the new flow, not dropped. Restore it, for example expect(chart.locator('.question-match')).to_contain_text(lines[1]) with answer_node_ids set in the fixture.
- Fix risk: low

### W2: **by_design** (claimed n/a, judged none)

- Where: `crates/desktop-server/tests/investigation_browser.py:~160 (Reading question fill) and ~214, ~229, ~236, ~278`
- Why: The 'Reading question' field is replaced by 'Your question'. Design 02 (question-first) says a typed question gets an answer only, and an empty question gets 'Find main results' with prompt 'Identify the paper's main results.' So the assertions were changed to new defaults: send prompt 'Identify the paper's main results.', investigation_scope == 'map', and Focus question default 'What assumptions, methods and evidence support this result?'. The change is consistent. The old-question variants are covered by check_question_first_investigation, where sends[0]['prompt'] == question, scope 'question', and the map 'Find main results' send keeps the question and has a focus-question value of question. No coverage is lost.
- Fix risk: none

### W3: **by_design** (claimed n/a, judged none)

- Where: `crates/desktop-server/tests/investigation_browser.py:~296`
- Why: The click on '.investigation-selected summary' became expect(...details first).to_have_attribute('open',''). The assertion is equivalent or stronger. Design 02 says the selected main result's first anchor opens by default (openFirst) and that investigation_browser.py:184,192 stays valid. The old click would now close the details, so the test had to change.
- Fix risk: none

### W4: **not_a_bug** (claimed n/a, judged none)

- Where: `crates/desktop-server/tests/investigation_browser.py:~203-215 (conventions focus)`
- Why: inferred.focus() was moved after the refresh, and the fixture now dispatches turn_complete. New assertions were added: Stop button visible, Esc inside the searchbox does not call /stop, and the receipts['stops'] list stays empty. This follows the Esc design (composer.js ignores Esc in #investigation-panel inputs). The result is stronger, not weaker. The state.update gained mapping_requested=True, which is justified by the new gating (summary box needs mapping_requested or map).
- Fix risk: none

### W5: **not_a_bug** (claimed n/a, judged none)

- Where: `crates/agent/src/resume.rs:245 (test earlier_map_tool_can_gain_only_its_optional_answer_index)`
- Why: The test was moved verbatim to crates/agent/src/resume_drift_tests.rs, which resume.rs now includes via #[path] mod drift_tests. Both assertions (ok for the earlier schema, err for a changed nodes type) are intact. It has new golden 7c36a0e hash tests beside it.
- Fix risk: none

### W6: **not_a_bug** (claimed n/a, judged none)

- Where: `crates/desktop-server/src/investigation.rs (test long_model_context_keeps_source_locations_and_reports_omissions)`
- Why: The test was moved verbatim to the new src/investigation_tests.rs (line 63). All six assertions are the same (len<8192, node_id, start_line, excerpt null, additional_saved_rows>0, stored excerpt unchanged).
- Fix risk: none

### W7: **not_a_bug** (claimed n/a, judged none)

- Where: `crates/desktop-server/tests/investigation_origin.test.mjs:94-125`
- Why: Only harness stubs changed: added stubs for investigation_answer.js, investigation_scope.js and setComposerPlaceholder, an after() shim, and an href rewrite for investigation_state.js. No assertion was removed or loosened. Stubbing the new modules is needed because investigation.js imports them. Their behaviour has its own tests (investigation_answer.test.mjs, investigation_scope.test.mjs).
- Fix risk: none

### W8: **not_a_bug** (claimed n/a, judged none)

- Where: `crates/desktop-server/tests/investigation_map.test.mjs:1-18`
- Why: The loader changed: the real math.js is now imported, with KaTeX, instead of stripping the import, because investigation_map.js now renders math excerpts. Existing assertions are untouched, and about 115 lines of new tests were added.
- Fix risk: none

### W9: **not_a_bug** (claimed n/a, judged none)

- Where: `crates/desktop-server/tests/investigation_conventions.test.mjs:7; math_render.test.mjs:120; ui_math.rs:31; investigation_flow.rs:246; resume_loop.rs:547; research-tools/tests/investigation.rs:669`
- Why: These are additive only. The conventions import gained createConventionCatalog. math_render's CSS audit gained 3 container selectors (.math-display, .math-glue, .table-wrap), which makes it stricter. ui_math.rs gained a second suite test. The other three files only had tests appended. There are no removals, no #[ignore], no skips, no timeout changes and no lowered counts.

## Fix commits reviewed so far

So far origin/gemini/work/fixed has one commit beyond merged (8bf50bb), and it changes only crates/desktop-server/tests/investigation_browser.py. The race the fix agent describes is real and is in the test, not the app. Playwright's set_input_files filled the hidden #investigation-file before createAndSelectSession -> selectSession -> showInvestigationSession(id, null) reset the form. buildMap then stopped at 'Choose a primary paper.' before the model check, so the Settings dialog never opened. A user cannot pick a file before that reset: the investigation panel stays hidden until after it, and the later showInvestigationSession(id, sess) call does not reset. The fix waits for #investigation-setup to be visible, gives each install its own data directory, and waits for each POST to /messages before checking it. These changes only add synchronization; nothing was weakened or deleted, and no app, honesty, policy or escaping code changed. Verdict: the commit is fine. One small older race (picking a file in an empty investigation's form while 'New investigation' is still creating the next session) is noted under missed items and is unrelated to this fix.

### 8bf50bb: **not_a_bug** (claimed test-only fix for Windows CI failure, judged none (legitimate test race fix))

- Where: `crates/desktop-server/tests/investigation_browser.py:347-489`
- Why: The diagnosis holds against the JS on origin/gemini/work/fixed. Clicking 'Investigate a paper' runs createAndSelectSession({mode:'investigation'}) (app.js:494), which awaits api.createSession, fileNewSession and refreshSessionList before selectSession(id). selectSession then calls showInvestigationSession(id, null). Because sessionId changes, that call runs byId('investigation-setup').reset() (investigation.js:334), which clears #investigation-file. The old test called set_input_files straight after the click. Playwright's setInputFiles does not wait for visibility, so it could fill the still-hidden input before that reset. The reset then cleared the file. buildMap (investigation.js) checks for a file before it checks for a model ('if (!file && !sourceId) { showStatus("Choose a primary paper."); return; }'), so it showed a status line and never reached callbacks.requestOptions() -> openConnect(). That is exactly '#settings-dialog expected visible but hidden'.

A real user cannot hit this path. From the blank or hero state, #investigation-panel is hidden (index.html has the hidden attribute, and selectView only shows it once session.mode==='investigation'). It becomes visible only in the second showInvestigationSession(id, sess) call, after the reset. That call has the same id, so no reset runs. A file the user picks in the visible form is therefore never cleared by this path. Once a file is chosen, the no-model path does open Settings. Waiting for #investigation-setup to be visible matches what a user can actually do.

The other changes are also sound. Giving each open_install its own data directory removes leaked state: the first install creates an investigation session, which could otherwise be returned as listSessions().sessions[0] and would replace the empty hero. The '#composer-model option:checked' == 'Gemini fixture' waits only add synchronization. Wrapping the three sends in expect_response('**/<sid>/messages') fixes a second real test race, where sends[n] was asserted right after the click; route.fulfill still emits a response event, so the wait resolves. No assertion was removed or loosened, no proof test was touched, and no app, policy, sandbox, ledger, escaping or honesty code changed. The diff touches only this one test file.
- Fix risk: Low. Test-only change; all edits add synchronization or isolation.

### Missed by the auditor

- **M1**: Separate, older race the commit does not touch (in origin/gemini/work/merged as well; not the CI cause, low severity): #new-investigation-btn sits in the rail, so it can be clicked while an empty investigation A is open and A's setup form is visible. The button is disabled during createAndSelectSession, but A's file input is not. A file picked in A's form during that window is cleared when selectSession(B) resets the form. setupDrafts saves only the question, source and page fields, never the file. Possible fix: disable the setup inputs while a create is pending, or carry the File over to the new session.
- **M2**: UX observation, not a bug: buildMap checks for a file before checking for a model. A user with no model and no file sees 'Choose a primary paper.' instead of the connect-a-model prompt. This ordering is why the test race showed up as 'settings dialog hidden'.
