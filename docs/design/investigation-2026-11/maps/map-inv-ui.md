# Investigation-mode UX map (inv-ui)

Method: static reading only. Nothing was run (no app, cargo, Playwright or node tests), so every "bug" below is proven by code path, not by execution. Gaps are listed at the end.

## 1. Walkthrough: what a first-time user sees and must do

### Step 1. Choosing the mode
- There is no mode switch. Mode is fixed when the session is created (`crates/desktop-server/src/routes_session.rs:175,264-265`).
- Two entry points:
  - Rail button `#new-investigation-btn`, label "New investigation", tooltip "Read a paper and trace its argument" (`crates/desktop-server/assets/index.html:41`). It sits under `#new-research-btn` "+ New research" and has no "+" glyph (`index.html:37-40`).
  - Empty-state hero button `.project-shortcut.investigation-shortcut` "Investigate a paper →", below the starter chips (`crates/desktop-server/assets/stream.js:190`). It just clicks the rail button (`crates/desktop-server/assets/app.js:124`).
- Either click immediately creates a session with `{mode:'investigation'}` (`crates/desktop-server/assets/investigation.js:33-38`, `app.js:143,493-506`).
- The new session's title is the generic default "Research Session" (`crates/desktop-server/src/state.rs:22`, `routes_session.rs:216-224`). It shows in the `#active-session-title` h1 and in the rail.
- The rail row has no mode marker. `session_rail.js` never reads `s.mode` (`crates/desktop-server/assets/session_rail.js:74-115`).
- The only on-screen sign of the mode is the segmented control `#investigation-nav` (group aria-label "Investigation view") with `#show-paper-map` "Paper map" and `#show-investigation-chat` "Conversation" (`index.html:224-229`). The default view is the map (`investigation.js:232-233`).

### Step 2. The setup form (`#investigation-setup`, `index.html:233-252`)
- Heading "Investigate a paper", then "Trace the authors’ main results back to their assumptions, methods and evidence."
- "Primary paper": `#investigation-file`, accepts `.tex,.pdf,.md,.txt,.zip,.tgz,.gz`. Help text `#investigation-formats`: "PDF, TeX, Markdown, plain text or a TeX archive. Up to 10 MiB."
- "Or choose a paper already in this session": `#investigation-existing`, first option "Select a paper…". It is hidden until the session has a source (`investigation.js:256-261`).
- Fieldset `#investigation-pages`, legend "Paper pages (optional)", help "Leave both blank for the whole PDF. Use the page numbers in the PDF viewer.", inputs "First page" / "Last page". It appears only for a `.pdf` (`investigation.js:61-68`).
- "Reading question (optional)": textarea `#investigation-question`, placeholder "Which assumptions does the main result need?".
- Submit button "Find main results".
- Below the form: `#investigation-original` "Open original PDF" (hidden while a new file is chosen, `investigation.js:64`), `#investigation-status` (role=status), `#investigation-summary`, `<details id="investigation-conventions">`, and `#investigation-map` (`index.html:253-257`).
- In map view, CSS hides `#pipeline-bar, #message-stream, #composer-input, #composer-send, #composer-files, .upload-label` (`crates/desktop-server/assets/investigation.css:8`). The composer dock stays visible with only "Research options", the model select `#composer-model`, and Stop (`index.html:309-343`). That bottom bar is where the model and effort for the investigation are chosen; the form does not say so.
- Client validation messages go to `#investigation-status` (`investigation.js:314-321`):
  - "Choose a primary paper."
  - "This paper exceeds 10 MiB. Choose a smaller file."
  - "Enter both page numbers, with the last page at or after the first."

### Step 3. Submit: upload and ingest
- All controls are disabled and the status reads "Adding paper to this session…" (`investigation.js:330-335`).
- The upload POST ingests synchronously on the server (`crates/desktop-server/src/routes_tools.rs:27-97`). The ~137 s PDF ingest from the live run is spent on this one static line.
- During ingest no turn exists, so Stop is hidden (`crates/desktop-server/assets/composer.js:257-264`) and `#stream-status-pill` still says "Ready".
- Only after the upload returns does the client:
  - reject a PDF with no text layer: "This PDF has no readable text layer. Open the original PDF, or choose a searchable PDF or TeX source to build a map." (`investigation.js:351-353`);
  - check for a model (`investigation.js:354`). With none, the Connect dialog opens and the status shows "Connect a model to run the investigation." (`app.js:150-155`).
- The prompt sent is the reading question, or the default "Identify the paper’s main results." (`crates/desktop-server/assets/investigation_map.js:44`, `investigation.js:355-357`). It is appended as the user's own bubble in the hidden Conversation (`app.js:850`).

### Step 4. While the first pass runs (map view)
- Topbar pill goes "Connecting..." → "Thinking..." → "Streaming..." (`app.js:836,853,687`). Two Stop buttons appear, `#topbar-stop-btn` and `#composer-stop`, and Esc also stops (`composer.js:95-110`).
- The form is hidden (`investigation.js:271`). The panel shows:
  - `#investigation-status`: "The selected model is working. Open Conversation to follow its progress or approve a tool." Once the summary exists but the map does not: "The paper overview is ready. The model is identifying the main results." (`investigation.js:273-274`). After the map is saved it reverts to the generic sentence, even though the conventions review is still running.
  - `#investigation-summary`: h2 "What, Why, and How", then "Reading the paper for a source-backed overview…" (`investigation_map.js:150-156`). Later three rows "What / Why / How", each with text and anchors, or "The reviewed passages do not state this explicitly." / "No source-backed statement is available." (`investigation_map.js:158-170`).
  - `#investigation-conventions` summary: "Conventions · Reading the paper…", then "Conventions · N found so far" (`crates/desktop-server/assets/investigation_conventions.js:29-39,130`).
  - `#investigation-map`: empty until `record_paper_map` succeeds, then the overview from Step 5 renders while the turn is still running.
- Updates arrive by polling the whole session every 4 s while the summary or conventions are pending, plus on `investigation.*` SSE events (`investigation.js:283-295`, `app.js:750`).
- There is no elapsed timer in this view. The only timer, `#pipeline-timer`, lives in the hidden pipeline bar; the browser test asserts the bar is hidden (`crates/desktop-server/tests/investigation_browser.py:160`).
- Usable during the run: read the summary, open anchors, browse conventions found so far, open a result, "Add correction", "Open linked Research".
- Disabled during the run, with no stated reason: "Develop this result", "Check claim"/"Recheck claim", "Explore implications" (`investigation_map.js:340,439,474`).
- Conversation view shows:
  - the pipeline bar "Orchestrator → Proof DAG → Verifier → Strategist" with badge "Working" and an mm:ss timer (`index.html:196-222`, `crates/desktop-server/assets/workbench.js:57-78`);
  - a live `<details class="thinking-accordion">` "Activity & reasoning (N tool calls)" with badge "Activity" (`stream.js:341-360,535`);
  - one card per tool call. Paper reads appear as "Workspace · fs_read" with the first 500 characters of output (`crates/desktop-server/assets/trace_cards.js:146-169,308`). `search_paper_passages` and `record_paper_*` get the generic card: raw tool id, status badge, first 600 characters of the args JSON and first 900 of the output (`trace_cards.js:318-327`).
  - A rejected record call is one such card with badge "error" and the text "Excerpt does not match … lines A–B exactly. Read the lines and retry." (`crates/desktop-server/crates`-relative: `crates/research-tools/src/investigation.rs:511`).
  - Send becomes "Steer", placeholder "Steer the active turn (injected at next step)..." (`composer.js:240-255`).
- Stop: pill "Stopping..." → "Stopped", and a warning line "Turn stopped." with a "Reuse prompt" button in the Conversation (`app.js:811-819,884`). Already-recorded summary, map and convention batches persist. The conventions header becomes "Review stopped before the first batch" or "Incomplete catalog" and a "Continue conventions review" button appears (`investigation_conventions.js:32,36,122-124`).
- Errors (`investigation.js:276-277`):
  - Mapping failure shows the server text, prefixed "Showing the saved main results. The latest reading failed. " if an older map exists.
  - With no finish record: "The main results are unavailable. Try again or read the conversation."
  - Server texts: "The model returned without saving the main results with sources. Retry reading or choose another model." and "The main results are saved. The model finished before its conventions review was complete." (`crates/desktop-server/src/investigation.rs:653,655`).
  - After a failure with no map, the form returns with file, select and pages locked; only the question and "Find main results" are usable (`investigation.js:267-272,365`).
- Completion: pill "Ready" and the status line clears. The model's prose reply is rendered only in the hidden Conversation (`app.js:754-791`).

### Step 5. Overview ("Paper map" tab; `overview()`, `investigation_map.js:175-223`)
- h2 with the map title (fallback "Paper map").
- Details "Source version · arXiv:ID" containing "Original file SHA-256 · …".
- The reading question as a paragraph (`.investigation-question-text`).
- Optional lines: "The model identified no statement that directly answers this reading question."; "PDF pages a–b"; "Model reports paper-wide coverage" or "Partial coverage"; details "Coverage and limitations"; details "Earlier map revisions (N)" with rows titled "Map <seq> · <node_id>".
- h3 "Main results", then `.investigation-index` of `button.investigation-node[data-node]` cards. Each card shows a kind label, the sentence, an optional "Selected for your question" / "Suggested-question text match", and "Explore result →" or "Open saved branch →".
- No graph is shown here. The first pass is asked for an overview that "may have no edges" (`crates/research-tools/src/investigation.rs:47,51`), and the browser test expects 0 graph nodes at this point (`investigation_browser.py:219-220`).

### Step 6. Result detail and branches (`detail()`, `developResult()`, `investigation_map.js:364-506`)
- Buttons "← Main results" and "Back to focused result".
- Article `.investigation-selected`: kind label, h2 `.investigation-claim-title`, and anchors.
- Each anchor is a closed `<details class="investigation-anchor">` with summary "<label|kind> · lines a–b". Inside: the quote, the path, "Open cited lines" (opens `#source-reader-dialog`) and "Open original PDF page" (`investigation_map.js:115-137`).
- Branch form `.investigation-develop`: label "Focus question" on textarea `name="branch-focus"` (required; prefilled with the reading question or "What assumptions, methods and evidence support this result?"), submit "Develop this result". An optional "Saved focus" select appears after two built branches.
- Branch rows are details titled "Developing branch · q", "Saved branch · q", "Branch failed · q" or "Interrupted branch · q". Notices: "Branch requested." and "Showing the saved branch for this focus.". With no branches: "Develop this result to trace its assumptions, evidence and intermediate claims."
- After a branch is built (a second model run), `branchGraph()` renders (`investigation_map.js:225-294`):
  - h3 "Dependencies for this result";
  - layout toggle "Graph" / "Steps in order" (aria-label "Paper map layout");
  - search "Find a statement";
  - toggle "Show all statements" / "Show main results and assumptions";
  - "Collapse expanded branches";
  - "Highlight answers" / "Highlight question text matches".

### Step 7. Graph (`crates/desktop-server/assets/investigation_graph.js:132-280`)
- Legend `.investigation-graph-key`: "Read downward from supporting statements to conclusions. Solid arrows show author-stated links. Dashed arrows include an inferred link. A + count marks hidden statements. Select an arrow to inspect it."
- Scroll region `.investigation-graph-viewport` (role=region, "Paper dependency graph") with "←"/"→" scroll buttons.
- Cards `.investigation-graph-node`, each optionally followed by `.investigation-expand` "Expand N adjacent statement(s)".
- Arrows are `g.investigation-edge[role=button]` with a "+N" badge for hidden statements.
- Clicking an arrow opens `.investigation-edge-info` "Selected connection", with text such as "Depends through N hidden statements ↓ · Includes inferred reasoning" and a button "Expand connecting statements".
- Below the graph: sections "What this statement uses" / "What depends on this statement". Each row has details "Dependency|Support|Citation · Author-stated connection|Inferred rationale" (`investigation_map.js:296-313`). Also details "Open questions".

### Step 8. Checks, corrections, implications (`investigation_map.js:315-362,402-447`)
- Action row `.investigation-actions`: "Add correction", "Check claim" (or "Recheck claim"), "Explore implications", "Open linked Research".
- Each toggles a form `.investigation-action-form`:
  - optional "Applies to" select ("This statement" / "Connection from <sentence>");
  - a textarea labelled "Your correction", "Research question" (prefilled "Under what conditions does this result hold?") or "What should the model examine? (optional)";
  - submit "Save correction" or the action label, plus "Cancel".
- Notices (role=status): "Correction saved beside the original.", "Request submitted. Its result will appear here.", "Linked Research opened."
- Corrections render as `aside.investigation-correction` "Your correction" / "Your correction to a connection".
- Check results appear under h3 "Checks and further implications" as closed details "Claim check|Recheck|Further implications · <status>". Inside: the result text and "Run evidence (N)" with rows "#seq · kind · tool" and a `<pre>` of the raw payload JSON, or "No tool evidence was recorded for this check."

### Step 9. Conventions (`investigation_conventions.js`)
- Closed `<details>` "Conventions · N entries[ · partial coverage]".
- Notes: "Saved catalog from an earlier reading of this paper." and "Conventions review stopped. <error>".
- Search "Find notation, meaning or scope" appears only with 7 or more items (line 137).
- h3 groups: Notation, Terminology, Units, Normalization, Signs, Coordinates, Indices, Ordering, Orientation, Algebra, Geometry, Measurement reporting, Other conventions.
- Each row is `details.investigation-convention`: "label — meaning", "Scope · …", "Stated in the paper" or "Inferred from use", a rationale, and anchors.
- Button "Continue conventions review" or "Find paper conventions".

### Step 10. Claim trace (Conversation only)
- Any saved assistant answer gets "Trace a claim" (`crates/desktop-server/assets/claim_trace.js:114-126`, wired at `stream.js:301`).
- Flow: "Choose a statement or equation to inspect." → panel "Evidence for this claim" → "Review this claim".
- This is the Research proof-graph feature. It is unrelated to the map's "Check claim".

### Step 11. Hand-off to Research
- Submitting the "Open linked Research" form creates a child session titled with the question and navigates to it immediately (`investigation.js:373-376`, `src/investigation.rs:781-806`).
- The child shows `#investigation-origin` as a collapsed `<details>` "From a paper investigation" (`investigation.js:104-110`; collapsed state asserted at `crates/desktop-server/tests/investigation_origin.test.mjs:185`).
- Inside the banner: the claim, the question, "Use research question", "Carried assumptions", "Paper conventions (N)" (8 shown, "Showing 8 of N conventions.", "Open full catalog in Investigation"), "Source passages (N)", and "Open original claim" / "Open original investigation".
- The question is neither sent nor prefilled. The user must expand the banner, click "Use research question", then press Send (`investigation_browser.py:303-311`). The stream meanwhile shows the generic "Start a research question" hero.

### Step 12. Follow-up questions about the paper
- Switch to "Conversation" and use the normal composer (placeholder "Write a research question…", `index.html:302`).
- With a map saved, the server answers from the saved map (`src/investigation.rs:554-568`).
- With no paper yet, the send fails with the toast "Send failed: Add a paper to this Investigation before starting the map." (`composer.js:500`, `src/investigation.rs:442`).

## 2. Findings

### (a) No feedback or misleading feedback during long waits
1. **Ingest (~137 s):** one static line "Adding paper to this session…"; no progress, no cancel, pill "Ready", Stop hidden (`investigation.js:335`, `routes_tools.rs:67-86`, `composer.js:257-264`).
2. **No clock or step counter in the default view.** The timer is hidden by `investigation.css:8`. The status sentence is static for minutes (`investigation.js:273-274`).
3. **Silence during 60–85 s record calls.** A `tool_trace` "running" event is emitted only after the whole tool call has been generated (`crates/desktop-server/src/session_view.rs:111-127`). `model.request` events reach the client but drive nothing visible (`workbench.js:98`, called with kind `session_event` at `app.js:751`).
4. **Rejected record calls are invisible in map view.** In Conversation they are raw-JSON cards with an "error" badge. Nothing says "1 quote did not match, retrying".
5. **Approvals render only in the hidden stream.** In map view the sole cue is the small pill "Awaiting Approval"; the status sentence does not change and the Conversation tab gets no badge (`app.js:611-620`, `investigation.js:273-274`). A "Check claim" that needs a compute approval can wait indefinitely.
6. **Map-started turns show no Stop.** For "Develop this result", checks and the conventions review, the client never calls `setComposerBusy(true)` until the first `text_delta`, `thinking_delta` or approval (`app.js:687-690,735-753`; `investigation.js:378-386`). Until then Stop is hidden, Esc is ignored and the pill says "Ready", while the panel says "The selected model is working". No pipeline timer is started either.
7. **Conventions header uses the session-wide running flag.** It shows "Reading the paper…" or "N found so far", and hides the resume button, during any running turn, including a branch or check (`investigation_conventions.js:32,36,122`, `investigation.js:252`).
8. **Pipeline bar is misleading in Conversation.** It shows "Proof DAG → Verifier → Strategist" with Verifier pending (`workbench.js:66`, default chip pressed at `index.html:277`), but the server forces verification, calculation and strategist off for Investigation (`src/investigation.rs:574-576`).
9. **No completion cue in map view.** The status clears and the prose answer to the reading question is in Conversation only (`app.js:764-779`).
10. **Stale notices.** "Request submitted. Its result will appear here." and "Branch requested." stay after the result arrives, until the user navigates (`investigation_map.js:355-356,498`; cleared only at lines 100, 341, 373, 437).
11. **After Stop with no map,** the panel status is the bare string "stopped by user" (`investigation.js:276`, `src/investigation.rs:651-660`, `crates/desktop-server/src/turn_runner.rs:163`).

### (b) Jargon and inconsistent naming
- **Session vs chat vs mode:** "New investigation", "Investigate a paper", "Paper investigation" (panel aria-label), versus default title "Research Session", "Research options", "Write a research question…", and rail copy "No active chats yet." / "Delete chat". The Research starter chip "Investigate a conjecture" reuses the verb (`stream.js:14`).
- **Map:** the tab "Paper map" shows a list. Elsewhere: "to build a map", "Earlier map revisions", "Map 11 · force", "This map has no marked main results or assumptions.", layout "Graph" / "Steps in order". The primary button is "Find main results", yet the server says "Retry reading" (`src/investigation.rs:653`) and no button has that name. Research separately has "Proof DAG" / "proof graph".
- **Node names:** "statement" ("Show all statements", "This statement"), "claim" ("Check claim", "Open original claim", "Claim missing from this map."), "result" ("Explore result →", "Develop this result"), 13 kind labels plus fallback "Claim" (`investigation_map.js:40-43`), and raw node ids leaking at `investigation_map.js:204`.
- **Edge names:** "connection", "link", "arrow", "Dependency/Support/Citation", "Is a prerequisite for", and three phrasings of one idea: "Inferred rationale" / "Includes inferred reasoning" / "includes inference".
- **Branch has three meanings:**
  - a saved focus run ("Saved branch", "Developing branch");
  - expanded hidden statements ("Collapse expanded branches");
  - Research's explore branches ("N branches", "Pause on Explore … to choose a branch", `trace_cards.js:199-204`, `index.html:279`).
  The same thing is also called "Saved focus".
- **Five kinds of question:** "Reading question", "Focus question", "Research question", "Open questions", "Suggested-question text match" / "Selected for your question", plus "What should the model examine?".
- **Overview:** "What, Why, and How" vs "paper overview" vs "source-backed overview" vs server "summary".
- **Conventions:** the paper catalog ("Conventions · …", "catalog", "entries", "found so far") collides with the user's pinned "Conventions" chip under Research options (`index.html:324`). Both are visible in map view.
- **Check vs review:** map "Check claim / Recheck claim / Claim check" vs conversation "Trace a claim / Review this claim / Claim review".
- **Raw enums:** activity status is shown as "answered", "interrupted", "running", "failed" (`investigation_map.js:409`), whereas branches have friendly labels (line 478).
- **Source links:** "Open cited lines" here vs "Read cited lines" / "Read recorded passage" in Research (`crates/desktop-server/assets/canvas_sources.js:492`, `claim_trace.js:182`).
- **"Workspace · fs_read"** labels reads of an uploaded paper that is not in the workspace.

### (c) Dead ends, hidden affordances, unexplained steps
1. **A section-level question has no home.** The only scoping is physical PDF page numbers, and the in-app viewer is unavailable before upload (`investigation.js:64`). The first pass still builds the whole-paper summary, main results and catalog (`src/investigation.rs:549,553`).
2. **The answer and all follow-ups live in Conversation.** Map view has no input.
3. **The reading question cannot be changed after the map.** The form is hidden once a map exists (`investigation.js:271`). A different paper or page range needs a new Investigation and a re-ingest ("Start a new Investigation for a different primary paper.", "Start another Investigation to map a different page range.", `src/investigation.rs:445,260`).
4. **Model and effort are chosen in the leftover composer bar.** The model check comes after the upload (`investigation.js:334-354`).
5. **Dependencies need a second paid run per result.** "Paper map" shows no graph until then. A built branch with zero edges shows neither a graph nor an explanation (`investigation_map.js:484-485`).
6. **Evidence quotes sit behind closed details** on every anchor. Check results are also closed by default.
7. **Graph affordances are explained only in one legend sentence:** clicking arrows, "+N", "Expand connecting statements".
8. **Linked Research needs three extra steps.** The Investigation keeps no list of its linked Research sessions (`src/investigation.rs:123-133` exposes none).
9. **Corrections are append-only.** There is no edit or delete control.
10. **A retry after a failed first pass re-requests summary, map and conventions together.** With no map, partial conventions have no resume button (`investigation_conventions.js:122`).
11. **Research UI leaks into the Investigation's Conversation.** With no messages it shows the hero "Start a research question" and starter chips. Its "Investigate a paper →" creates another session (`stream.js:141-143,190,199`).
12. **The in-app User Guide has no Investigation content** (`crates/desktop-server/assets/guide.js:3-46`).
13. **"Develop", "Check" and "Explore" are disabled during any run** with no tooltip.

### (d) Accessibility and keyboard gaps
- **Global Esc stops the turn.** The handler exempts only three dialogs (`composer.js:95-110`). The Original PDF dialog is created dynamically and is not exempt (`crates/desktop-server/assets/pdf_preview.js:9-11,97`). Esc there, or in the "Find a statement" / convention search boxes, calls `preventDefault()` and stops a running pass.
- **Graph tab order.** The SVG overlay is the first child of the stage (`investigation_graph.js:155`), so every arrow (`tabindex=0`, line 258) is tabbed before any card.
- **Raw TeX in accessible names and tooltips.** Arrow `aria-label` and `<title>` hold full sentences (lines 255-260). The same applies to "Connection from …" options, "Saved focus" options and branch summaries (`investigation_map.js:334,456,479`).
- **Progress is not announced.** Summary, map and conventions-count updates have no live region. `#message-stream` (aria-live) is `display:none` in map view.
- **Focus is dropped on submit.** The focused submit button's form is hidden (`investigation.js:271`).
- **Validation errors are not tied to fields.** They go to a status node below the form, with no `aria-invalid` or `aria-describedby` and no focus move (`investigation.js:314-321`).
- **Disclosure state is lost on re-render.** `render()` replaces the whole map (`investigation_map.js:508-514`). Only anchors directly under the selected claim are restored (lines 86-88). Other open details and the "Selected connection" panel close.
- **No shortcut switches Paper map / Conversation.** They are `aria-pressed` buttons, not tabs. Enter does not submit the setup form or action textareas.

### (e) Differences from Research a user would find surprising
- The default view hides the conversation, the input, Attach and the pipeline bar, but keeps a composer shell.
- Review mode, Verifier and Strategist chips stay selectable but are overridden server-side.
- The answer is not on the default screen. There is no verdict card.
- Approvals and Steer exist only in the other tab.
- There are two upload paths with different accept lists (`index.html:237` vs `306`), two graphs (center map vs Canvas "Proof DAG"), and two "claim" tools.
- Errors appear inline in the panel, whereas Research uses 3.8 s toasts (`session_rail.js:205-215`).
- The session title and rail row look identical to Research.

### (f) Proven UI bugs (code path; not executed)
1. Stop, Esc and the busy pill are missing for map-started turns until the first delta (a6).
2. Esc in the Original PDF dialog or a search box stops the running turn (d).
3. The conventions header reports "Reading the paper…" / "N found so far" during unrelated turns (a7).
4. The Verifier stage shows as pending for runs where the server disables verification (a8).
5. Model replies for checks and implications (up to 24,000 characters, `src/investigation.rs:634`) are rendered with `renderMathTextInto`, which handles math only (`investigation_map.js:410`, `crates/desktop-server/assets/math.js:207-212`). Markdown markup therefore shows literally.
6. The status is the bare "stopped by user" after a Stop (a11).
7. Notices go stale (a10).
8. The Research hero flashes when opening an Investigation: `app.js:541` sets chat view and line 547 renders the hero before the session loads at 568-570.
9. A new Investigation is titled "Research Session".
10. The model check runs after the costly ingest.
11. Internal identifiers and raw payload JSON are shown to the user (`investigation_map.js:204,419-420`).

## 3. Not read or not verified
- **Read only in part:** `math.js` (lines 150-212 and the function list), `canvas_sources.js` (lines 514-540), `crates/research-tools/src/investigation.rs` (lines 41-101 and 511), `turn_runner.rs` (lines 60-135), `session_view.rs` (lines 60-215), `investigation_origin.test.mjs` (assertions only).
- **Not read:** `hitl.js`, `model_picker.js`, `connect.js`, `subagents.js`, `effort_picker.js`, `canvas_dag*.js`, `projects*.js`.
- **Not verified:**
  - the order in which WebView2 dispatches Esc to a modal dialog versus the document handler;
  - whether `fs_read` or `record_paper_*` ever prompt for approval;
  - whether `investigation.*` ledger events written by tools reach SSE (the 4 s poll covers it either way);
  - whether "Interactive" mode or an active subagent alters an Investigation run;
  - how quickly Stop cancels mid model call.
- `composer.js`, `session_rail.js`, `guide.js`, `trace_cards.js` and `claim_trace.js` contain no "investigation" string; their effects above come from shared code paths.
- Headroom under the 600-line rule: `investigation_map.js` is 525 lines and `investigation.js` 387. `crates/desktop-server/src/investigation.rs` is already 810.
