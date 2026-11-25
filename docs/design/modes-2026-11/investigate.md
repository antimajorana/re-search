# re:Search INVESTIGATE mode: design

Reconciled with 00-overview.md on 2026-11-25; shared packages are referred to by their F ids.

Scope revision of 2026-11-25 (owner decisions, `00-overview.md` section 0.1), applied below:

- **Old and PDF-only papers are essential.** The PDF skeleton (V17) moves into the first two months
  and builds on the shared PDF package F14 (page-broken text, page links, scans). A PDF-only paper
  is a normal case, with its quality stated, not a fallback.
- **Bring your own papers.** A paper can be opened from the user's own library (F13: a Kandinsky
  vault, a folder, a `.bib` file), with no network.
- **Prior work inside Investigate (V14) is deferred.** The right half-badge is the button "Search
  wider in Explore". Everything below that describes the prior-work task, its tool, its statuses
  and rule 6.7 is kept as the design for later and marked "deferred".
- **Version tracking is deferred** (the version half of V10, stage M, evaluation set P2). A session
  stays on the version it opened.
- **Added:** reading notes as a write-up (F15); corrections the reader makes are reused (F16).
- **Lean is cut.** The word "verified" is shown nowhere; quotes are "matched".

Written 2026-11-25. Builds on `reSearch-modes-gaps-and-prior-art-2026-11-25.md` (the agreed gap analysis)
and on two independent proposals (researcher-first and architecture-first). Code facts were checked
read-only on `origin/gemini/work/fixed`; nothing was built, run or fetched. All waiting times are
targets to be measured, not measurements. Section 0 records what the review of the two proposals
found; sections 1 to 13 are the design.

Terms used throughout:

- **Skeleton**: the structure of the paper computed by code with no model (sections, theorem-like
  statements, proofs, labelled equations, figures, bibliography, and who refers to whom).
- **Interpretation**: everything a model adds, always attached to a skeleton node.
- **Tag**: the permanent identifier of a node, independent of its printed number.
- **Anchor**: a pointer `{source_id, path, start_line, end_line, quote}` that the app verifies against
  the hash-pinned source file (existing mechanism in `investigation_anchor.rs`, reached through
  `verify_quote` of shared package F4).
- **Matched**: a quote or anchor that passed that verification. On screen and in this document a
  quote is "matched", never "verified"; the word "verified" is reserved for a machine-checked formal
  proof and, since Lean was cut, is shown nowhere. Internal names such as `verify_quote` keep the
  old verb. A quote found in the text of a scanned page is "matched in OCR text" and its chip opens
  the page image (F14).
- **Task**: one scoped model run started by the reader (orient, open a node, context; prior work is deferred).
- **Prior work**: the comparison of one result of the paper with earlier literature. Previously
  called "check" in this document; "check" now belongs to Solve mode, where it means a computed test.
- **Recorder**: the validator that checks records and writes them to the ledger. In this mode every
  recorder is offered to the model as a registry tool.
- **Model call**: one request to the model provider. Every cap in this document counts model calls.
- **def_hash**: the hash of a tool definition, stored at session start and compared on resume.
- **Shared package**: one of F0 to F16 in `00-overview.md` section 2, built once for all three
  modes. This document refers to them by id and builds no version of its own.
- **Tool profile**: the frozen tool list of one mode, recorded in the session (F1). New Investigate
  sessions use the profile `investigation/2`. While V14 is deferred it adds three tools, not four:
  `record_prior_work_check` is named under the profile's `may_gain` and joins when V14 is built.
- **PaperKey**: the one name of a paper in every mode (F2), for example `arxiv:1102.0842`.
- **Names**: on screen the mode is "Investigate"; events, files and tool ids keep the older word
  (`investigation.*`, `investigation_*`, `map_*`).

## 0. Review of the two proposals (what was wrong, what was checked)

### 0.1 Facts checked in the code

| Fact | Where | Consequence |
|---|---|---|
| Anchors into the assembled TeX view are refused on purpose ("generated text and different line numbers") | `research-tools/src/investigation_anchor.rs` `SourceIndex::load`; `paper_lines.rs` near line 97 | The architecture proposal's package I-05 (make the assembled file the reading file) would reverse a deliberate honesty rule and touch a 597-line file. Rejected. Node spans point at the physical files. |
| An anchor may point at any file listed in the source receipt, with its own digest | same function (`files` and `files_sha256` checks) | Multi-file TeX already verifies per file. No verifier change is needed for spans across `\input`. |
| The "lines the model received" check credits only `fs_read`, `read_paper_lines`, `search_paper_passages`, and only for `primary_path` | `desktop-server/src/investigation_cites.rs` `collect_received_ranges` | The researcher proposal's "pre-injected lines are logged as received" has no mechanism today. A host event and a line-credit function are needed. Both are shared (F4): the event `context.supplied` and the function `lines_sent` (section 4.4, rule 6.17). |
| Recorder gating uses a hard-coded list of marker kinds, in three places | `research-tools/src/investigation.rs` lines 386-401; `investigation_conventions.rs` 192; `investigation_evidence.rs` 140 and 151 | A new marker kind that is not added to all three lists is invisible, so the old recorders would stay open during a new task. One shared constant fixes it. |
| Resume lets a session gain a tool only if its id is on a fixed list and its effect is `ReadWorkspace`; a missing recorded tool is rejected ("tools were removed or reordered") | `agent/src/resume.rs` `check_tool_defs_drift` | This hand-written list is replaced by tool profiles (F1): the four new tools live in the profile `investigation/2`, and an older Investigation session gains them on its next resume. The tool list still cannot vary per task, so the researcher proposal's per-scope subsets are not possible. |
| All research tools are one sorted list, offered in every mode | `research-tools/src/lib.rs` `all_research_tool_defs` | Without profiles every added tool would cost tokens in Explore and Solve too. With F1 the four new tools are offered only in Investigate sessions and are not added to this shared list. Fewer tools is still better: this design adds 4, not 6. |
| `core.exe` registers tools through `register_research_tools` | `cli-route/src/core_tools.rs` | F1 edits `core_tools.rs` once so that `core.exe` takes its tool list from the session's profile. The architecture proposal's separate "add four ids to core_tools.rs" is still not needed. |
| Reaching `max_turns` aborts the session | `agent/src/runner.rs` lines 410-414 | "A cap ends the task as partial" must be done in the host finish hook, which already exists per request kind (`investigation_finish.rs`). `max_turns` is already a per-request field (`turn_runner.rs` 184-189). On screen this state is "Paused" (section 3). After Solve package S12 makes the runner outcome `Paused` mode-neutral, this mode switches to it and drops the repair in the hook. |
| The host already fetches from arXiv on a user action without a model | `desktop-server/src/routes_tools.rs` `handle_literature` calling `run_arxiv_search` | A host-side "open by arXiv id" route follows an existing pattern and does not weaken approvals. |
| No arXiv rate limiter exists; e-print cap is 32 MiB; at most 2 e-prints per search call | `http_client.rs`, `arxiv.rs` (`MAX_EPRINT_PAPERS`) | The 3-second spacing is new code, built once for all modes in F3 (`net_rate.rs`). |
| Allow-listed hosts: export.arxiv.org, arxiv.org, inspirehep.net, api.crossref.org, api.semanticscholar.org | `http_client.rs` | arxiv.org/html is reachable. OpenCitations and ar5iv are not; this design does not need them. |
| Blob storage pinned by hash exists | `Ledger::put_blob` / `get_blob` | The skeleton is stored as a blob named in its ledger event, not as a loose file beside the sources (which the receipt would not pin). |
| The only XML parser in the crate is `roxmltree` | `research-tools/Cargo.toml` | arXiv HTML is HTML5, not XML. The HTML builder needs its own tag scanner or a new dependency (decision 12). |
| PDF text extraction runs inside ingestion, synchronously | `ingest.rs` `prepare_pdf_ingestion` | A route that "returns only after the skeleton is appended" would block for the 34 to 138 s seen in the logged run. PDF needs a background job with progress. |

### 0.2 Researcher-first proposal: faults

1. **Cited-result lookup can show the wrong theorem as fact.** It finds "Theorem 2.1" in the cited
   paper by a number the app computed by simulating LaTeX counters, the least reliable part of the
   scanner, then shows that statement verbatim under "what the cited result states". Fixed in 6.9: a
   located statement is shown as established only when the number is confirmed; otherwise the reader
   picks from candidates.
2. **"Attributed by the authors to [NS06]" from a citation in a theorem heading** overstates: a heading
   citation can also mean "generalises". Wording fixed to "cites [NS06] in its heading".
3. **Six new tools and per-scope tool lists** cannot work with resume (0.1). Reduced to four, always
   present in the Investigate tool profile (F1), gated by the recorder.
4. **Skeleton stored as a file beside the source** is not covered by the receipt hash. Replaced by a blob.
5. **Fuzzy tag carry-over at 0.8 with no margin** can attach a note or a NEW label to a different
   lemma. The stricter rule (unique best, runner-up below 0.7) is adopted, and labels on changed nodes
   lose their colour.
6. **Legacy edges given role "proof" by default** is a guess shown as structure. They are shown with
   role "unknown (older map)".
7. **23 packages; nothing visible until the ninth.** Reordered so the third package puts the skeleton
   on screen.
8. **HTML export with node cards** would redistribute full statements of papers under the default
   arXiv licence. Statements are exported only when the paper's licence allows or the reader ticks a box.
9. **The 6-second first screen** ignores that the limiter does not exist yet and that metadata and
   e-print are two requests. Kept as a target with the measurement placed in the first packages.

### 0.3 Architecture-first proposal: faults

1. **I-05 (assembled reading file)**: rejected, see 0.1.
2. **One `record_reading` call with up to 48 items and 128 edges** repeats today's failure mode (one
   long call late in a 48-turn run). Caps cut to 16 items and 32 edges per call, several calls per task.
3. **Model node ids from `hash(request_seq, id)`** change on every re-run, so re-running duplicates
   every model node and the orphan problem returns one level down. Replaced by a content-based id.
4. **"Latest record wins"** lets a later run silently replace a label the reader has already relied on.
   Kept, but the node card shows "changed since you last opened this" with the earlier value, and
   reader pins and notes always win.
5. **Paper route returns only after the skeleton exists**: blocks on PDF (0.1). Split into immediate
   response plus job progress.
6. **"No earlier source found" is refused unless a `literature.fetched` event follows the request**:
   a search the reader runs by hand writes the same event. Searches are counted by tool call id of
   the task's own main-agent calls.
7. **Abstracts ingested as a new source format** changes `ingest.rs` (565 lines) and lets an abstract
   support "same statement". Dropped from the first version: evidence must be full text; hits known
   only from an abstract are listed as `abstract_only` with the verbatim abstract quote and never
   support more than "possibly related" (F12).
8. **"By Lemma 3" text mentions resolved through computed numbers** are heuristic. Kept, flagged, and
   excluded from skip dimming unless the number is confirmed.
9. **Every unlabelled equation is a node, then dropped to fit caps.** Simpler: an equation is a node
   only if it has a label; others are counted on their parent.
10. **Retiring the old recorders** needs a new resume rule and gives the reader nothing. Moved to the
    end as an owner decision.

### 0.4 Faults shared by both

1. **A matched quote pair does not make "same statement" true.** Both proposals let the model type
   the relation between this paper's result and an earlier one and then show "Earlier statement
   found". The quotes are matched; the relation is a model judgement. The UI must say so until the
   reader confirms the judgement (6.7).
2. **"Main result" is a model label that drives what is dimmed as skippable.** The reader must be
   able to pin and unpin main results, and the skip plan must state its basis.
3. **Neither handles the three marker lists** (0.1).
4. **Question-turn citations into `\input` files are not credited** because the received-lines check
   is primary-path only. `read_map_node` results must be credited per (path, file hash); the shared
   `lines_sent` function of F4 does this.
5. **DOI entry for a paper with no arXiv version** cannot fetch anything (publisher hosts are not
   allow-listed). The app must say so and ask for the file.

## 1. What this mode is for

- You give it a paper (arXiv id, DOI, link, file, or a paper from your own library); it shows the
  paper's own structure as a map before any model call: within seconds from TeX, after text
  extraction from a PDF. Old papers and papers that exist only as PDF are a normal case.
- You open any node, to any depth, and the model explains that node only; you pay only for what you open.
- Each result says whether the authors call it new, restated or background, with the quote. What
  earlier literature says is a separate question: one click hands the result to Explore ("Search
  wider in Explore") and Explore's answer is shown next to the result. (Investigate's own
  prior-work check is deferred.)
- Context you lack (definition of a symbol, what a cited theorem states, prerequisites) appears next
  to the node, visibly marked as from this paper, from outside it, or unchecked model knowledge.
- It tells you which sections no main result refers to, so you can skip them, and hands a node to
  Explore or Solve.
- You can take your reading out as notes (outline, main results, your notes, quotes with pages), and
  a fix you make (a node the scanner got wrong, a main result you pinned) is kept.

## 2. Worked example

A mathematical physicist needs the quasi-locality estimate for the spectral flow from Bachmann,
Michalakis, Nachtergaele, Sims, "Automorphic equivalence within gapped phases of quantum lattice
systems" (arXiv:1102.0842), to adapt it to a different decay function. Section and theorem numbers
below are illustrative; they were not checked against the source.

| Time (target) | Reader does | App shows | Model calls |
|---|---|---|---|
| 0:00 | Pastes `1102.0842` into the Investigate card of the home screen, Enter | "Fetching TeX source" | 0 |
| 0:02 | | Metadata card: title, authors, abstract (rendered), versions, licence | 0 |
| 0:04-0:08 | | Outline with reading minutes per section; "Results in this paper": every theorem, lemma, proposition, corollary with its verbatim statement; reading plan "the statements of all theorems refer to Sections 1, 2.1, 3.1 (about 14 min); no theorem refers to Section 6 or Appendix B (about 11 min)"; header "Structure read from TeX source, v3" | 0 |
| 0:08 | (keeps reading) | Thin line "Model is identifying main results" | orient task starts |
| 0:40-1:20 | | Item by item: 4 to 6 results marked "Main result (model)" with a one-sentence gist; left half-badge on each result "Authors: new / restated from [x] / background / not stated", each with the suffix "(model)" and its quote | 3 to 6 model calls |
| 1:20 | Clicks the quasi-locality theorem | At once: verbatim statement; "Statement uses" (decay-function definition, gap assumption, eq. (2.7)); "Proof uses" (Lieb-Robinson lemma, weight-function lemma); "Used by"; buttons Source lines, PDF page, Proof | 0 |
| 1:20-2:00 | | Below, streaming: gist, hypotheses checklist (each with a quote), proof split into 3 to 7 steps as child nodes | open task, 3 to 6 model calls |
| 2:00 | Clicks the symbol F | "Nearest definition above: Section 2.1, line 412" with the sentence. Marked "found by pattern" | 0 |
| 2:30 | Clicks citation chip "[NS06, Theorem 2.1]" | "Looking up [NS06]"; then the cited paper's Theorem 2.1 verbatim in the "Outside this paper" style, with "numbering taken from arXiv v2; the journal version may differ". If the number cannot be confirmed: a list of that paper's theorems to pick from | 0 |
| 4:00 | Clicks right half-badge "Earlier work: search wider in Explore" | An Explore session opens from one hand-off packet (F7): the statement as the idea, this paper as origin (never counted as a find), its bibliography as seeds; reader comes back and continues | 0 here; Explore's own budget there |
| 5:30 | | The half-badge reads "Opened in Explore: Model judges: partly the same, [HW05]" or "Opened in Explore: not found in the searched scope", read from the Explore session; one click opens it | 0 |
| 6:00 | Types "Does the proof need the gap to be uniform in the system size?" with the node selected | Answer card under the node with line citations (existing question turn) | existing |
| 8:00 | Presses M | Whole map: theorem-like nodes in section lanes, heavy solid arrows for statement uses, light solid arrows for proof uses (a toggle shows either set; a dashed arrow would mean a link inferred by the model); nodes no main result needs are dimmed | 0 |
| 9:00 | Writes a note, clicks "Derive in Solve" | Solve setup card opens from one hand-off packet (F7) with the statement, the definitions and assumptions it uses (each listed as Cited once its quote matches again in the new session), and the note | 0 here |
| Next week | Opens the session | Everything is as it was, on the version first opened (v3). A newer arXiv version is not looked for: version tracking is deferred | 0 |

The same reader with an old paper: a 1972 article that exists only as a scanned PDF in their own
library. They pick it from "Library" on the Investigate card. No network is used. Page text is
extracted with page progress; the outline, theorem-like headings found by pattern, numbered
equations and the reference list appear with the quality line "estimated from PDF text (scan);
boundaries may be wrong". Every statement shows its page, and a click opens the page image. Formulas
from the scan are shown as the page crop, not as rendered mathematics. Orient and node explanations
work as above; quotes are tagged "OCR".

Click and wait budget (acceptance targets): paste to readable structure in one action and under 10 s
for a TeX paper with zero model calls; open a node in one click with the statement visible in under
200 ms; symbol definition under 100 ms; cited statement under 10 s when the cited paper has TeX on
arXiv.

## 3. What the user sees, stage by stage

Layout: left outline tree (always visible); centre is the results list, one node card, or the whole
map; right context drawer; bottom composer scoped to the selected node. Single column under 900 px.

| Stage | Trigger | Wait (target) | Model | On screen | If it fails |
|---|---|---|---|---|---|
| A. Entry | Paste id / DOI / link, drop a file, or pick from the library | 0 | no | The Investigate card of the home screen (F5): one input; recent papers from the shared library (F10) and the papers of the user's own sources (F13). An arXiv id, DOI or arXiv link pasted into any card's box focuses this card | "This DOI has no arXiv version and is not in your library; drop the PDF or TeX" |
| B. Metadata | automatic | 1-3 s | no | Title, authors, abstract, versions, licence | Skipped for dropped files; title from the file |
| C. Skeleton, TeX | automatic | 3-8 s after B | no | Outline, results list, reading plan, quality line | Falls to arXiv HTML (later package), then PDF |
| C'. Skeleton, PDF | automatic | extraction time as measured by F14a (one logged run with today's extractor: 30-140 s); done once per file and kept in the library; outline fills as pages arrive | no | "Extracting page 12 of 41"; abstract from stage B stays readable; outline, theorem-like statements, numbered equations and references with page links (V17); quality line "estimated from PDF text; theorem boundaries may be wrong", and "(scan)" when the pages are scanned | Sections-only outline; a page with no text at all shows as its image until own OCR exists (F14b) |
| D. Orient | automatic if a model is connected and the setting is on; else a button | 30-90 s, items arrive one by one | yes, cap 8 model calls | Main-result markers, gists, authors' half-badges, recomputed reading plan | "Paused: 3 results annotated" with Continue; the skeleton stays fully usable |
| E. Open node | click | statement at once; explanation in 10-40 s | deterministic part no; explanation yes, cap 10 | Node card blocks A (skeleton) and B (model) | Block B shows the error and Retry; block A unaffected |
| F. Symbol | click a symbol | under 100 ms | no | Nearest earlier definition, marked "found by pattern" | "No definition found above this line" |
| G. Cited result | click a citation chip | 3-10 s | no | Verbatim statement, "Outside this paper" style | Named states, see 6.9 |
| H. Context | "Prerequisites" or "Compare hypotheses" button | 15-40 s | yes, cap 8 | Cards in three provenance styles | Partial list |
| I. Earlier work | click the right half-badge "Search wider in Explore" | under 1 s to open Explore | no (Explore spends its own budget) | An Explore session opened from a hand-off packet; afterwards the half-badge shows Explore's status for this result, read from that session | "Opened in Explore: not finished" |
| I'. Prior work inside Investigate (deferred, V14) | - | - | - | The task with its two depths, statuses and coverage, as designed below, is not built now | - |
| J. Whole map | key M | under 1 s | no | Layered graph, filters (shared graph view, F6) | Above 150 visible nodes, one box per section |
| K. Skip plan | always on once a main result is known | 0 | no | Dimmed sections, two reading times, basis line | Nothing dimmed until a main result exists |
| L. Export / import | menu | under 2 s; import re-matches every quote | no | File saved; "Imported, 4 quotes could not be matched" | Read-only with banner |
| L'. Reading notes | menu "Write up" | under 2 s | no | Markdown or TeX: outline, main results with quotes and pages, gists marked "(model)", the reader's notes, a bibliography with the reader's own citation keys (F15) | - |
| M. New version (deferred) | - | - | - | Not built now. A session stays on the version it opened | - |

Task states. Every task shows the shared run bar (F6) with the one run-state vocabulary of all modes
(F9): "Running" with elapsed time and "3 of at most 8 model calls"; "Waiting for you" when an
approval is needed; "Paused: (reason)" with Continue when a task stopped at a cap or was stopped by
the reader, nothing lost; "Interrupted" with Retry when the app or the provider went away mid-run
(never re-run unasked); "Failed" with Retry and the reason; "Done". "Partial" is a word for a
result, not a state: a paused task reads "Paused: 3 results annotated".

Node card. The card uses the shared node-card shell of F6 (instant block from code on top, model
block below, each labelled). Block A (instant, skeleton): header (kind, number, title, tag), verbatim statement,
"Statement uses" and "Proof uses" as two chip lists, "Used by", citations with locators, buttons
Source lines, PDF page, Proof, Note, Ask, Derive in Solve, Has this been extended? (Explore).
Block B (model): gist, hypotheses checklist with quotes, proof steps as child nodes (each can be
opened the same way), open questions. With no model connected, block B reads "Connect a model to
get an explanation" and block A is complete.

Novelty row. Two half-badges that never merge; each is text plus icon, never colour alone (F6).
Left: "Authors: New / Restated / Background / Not stated", each with the suffix "(model)"; click
shows the quote, plus any pattern cue ("cites [12] in its heading", found by code). Right, while
V14 is deferred: the button "Earlier work: search wider in Explore", and after it was used
"Opened in Explore: (Explore's status sentence for this result)", read from the child session's
latest `idea.verdict`; nothing is written into this session except `handoff.sent`. The sentence is
Explore's and comes from the shared wording module (F12), so the words "new" and "novel" do not
exist on the right side.

Deferred with V14: the right half-badge as a task of this mode ("Prior work: Not run / Running /
Model judges: earlier statement / Model judges: partly earlier / Model judges: earlier under
another name / Possibly related (abstract only) / No earlier statement found in: (list) /
Incomplete: (conditions not met)", with a Confirm button on a "Model judges" status).

Three provenance styles, set by code, used everywhere in this mode:

| Style | Meaning | Look |
|---|---|---|
| From this paper | every anchor matched in the investigated source | plain, with a line reference |
| Outside this paper | at least one anchor matched in another ingested source | coloured left border, header "Outside this paper: (short reference)" |
| Model background, not checked | no anchors | dashed border, header as named; one toggle hides all of them |

Line and border style is the same in every mode (F6): solid means found by code or backed by a
matched full-text quote; dashed means inferred by the model, abstract only, or not checked. In the
whole map, "statement uses" and "proof uses" are both solid when found by code and differ by line
weight (heavy and light) and a toggle. Any model text that is not a verdict carries the suffix
"(model)"; a verdict starts "Model judges:".

## 4. Data model

### 4.1 Layer 1: skeleton (deterministic)

```
PaperSkeleton {
  schema: 1, builder: "tex-1" | "html-1" | "pdf-1",
  source_id, source_seq,              // the source.ingested receipt this was built from
  files_root_sha256,                  // hash over the receipt's files_sha256 map
  paper_key,                          // a PaperKey (F2): "arxiv:<base id, no version>", else "doi:<normalised>",
                                      // else "file:<first 16 hex of sha256 of the raw file>"; previously called `family`
  version: Option<String>,            // "v3"
  quality: "full" | "partial" | "weak", quality_reason,
  nodes: Vec<SkNode>, edges: Vec<SkEdge>, bib: Vec<BibRef>, symbols: Vec<SymbolDef>,
  macros: Vec<(name, arity, body)>,   // passed to the renderer; never expanded in the text
  stats: per section { words, read_minutes, counts by kind },
  carry: Option<Carry>
}
SkNode {
  tag, kind, number: Option<String>, number_basis: "computed" | "html" | "printed" | "confirmed",
  env_name, title, label: Option<String>, parent: Option<tag>, order,
  spans: Vec<Span>,                   // Span { path, start_line, end_line } in PHYSICAL files
  proof_of: Option<tag>,              // proof nodes only
  head_cites: Vec<bib key>,           // citations in the environment heading
  intro_refs: bool,                   // abstract or introduction refers to it
  fp,                                 // 12 hex of sha256 of the comment- and whitespace-normalised body
  words, eq_count, read_minutes, confidence: "tex_env" | "tex_heading" | "html_class" | "pdf_heuristic"
}
kinds: front_matter, section, definition, assumption, theorem, lemma, proposition, corollary,
       conjecture, claim, proof, example, remark, equation (labelled only), figure, table,
       algorithm, bibliography
SkEdge {
  from,                               // the thing used
  to,                                 // the thing that uses it (same direction as today's maps)
  kind: "contains" | "uses" | "proves" | "cites",
  role: "statement" | "proof" | "prose",   // where the reference sits, decided by position only
  site: Span, rule: "ref" | "eqref" | "cref" | "proof_header" | "proof_adjacent" | "cite"
        | "head_cite" | "text_mention" | "lifted",
  confidence: "explicit" | "heuristic"
}
BibRef { key, raw, arxiv_id, doi, title, cited_at: Vec<Span>, locators: Vec<{site, text}> }
SymbolDef { symbol_tex, norm, span, in_node, how: "newcommand" | "define_phrase" }
Carry { prev_skeleton_seq, matched: [(tag, basis: "label" | "text" | "similar", changed)],
        retired: [tag], added: [tag] }
```

Rules that keep this honest and small:

- Spans point at the physical source files, so the existing anchor verifier applies unchanged.
  `\input` flattening happens in memory with an origin map (flattened line to file and line).
- "Statement uses" and "proof uses" come only from where the `\ref` sits: inside the statement's
  span, or inside the span of a proof whose `proof_of` is the target. A "lifted" edge moves a
  reference to a hidden kind (an equation) up to its enclosing node, so equations can be folded.
- `text_mention` edges ("by Lemma 3") are heuristic and resolve through printed numbers; they are
  drawn but do not count for the skip plan unless `number_basis` is "confirmed".
- Limits: 6,000 nodes, 20,000 edges, 2 MiB of JSON. Above that, quality is "partial" with the reason.
- Shared names (F2): `paper_key` is the `PaperKey` every mode uses, so the same paper has the same
  tags whichever mode opened it. A `BibRef` that is fetched becomes a `PaperRef {key, version, ids}`.
  A pointer to a node from another session is always a `NodeRef {session, mode, id, paper, node_fp}`,
  never a bare tag. The prefixes `n-`, `m-` and `legacy:` are reserved for this mode in the prefix
  table of `paper_key.rs`.
- Edge direction is the one fixed for all modes: `from` is the prerequisite, `to` the dependent.
  The mapview returns the shared `GraphView` shape (F6), where they are named `src` and `dst`.
- PDF sources (builder "pdf-1", package V17 on the shared PDF text of F14): the physical file a
  span points at is the page-broken text stored with the source receipt, so anchors verify
  unchanged. The page of a span is looked up from its lines, never stored. Every PDF node has
  `confidence: "pdf_heuristic"`; printed numbers have `number_basis: "printed"`; edges found from
  "Theorem 2.1" or "Eq. (3.4)" in the text are `text_mention`, `heuristic`. Quality is "weak" or
  "partial", never "full". When the pages are scans the quality reason says so.
- `carry` is always empty for now: version tracking is deferred. The field stays in schema 1 so
  that adding it later needs no schema change.

### 4.2 Stable tags

- First build: `tag = "n-" + base32(sha256(PaperKey | kind class | key))[..10]`, where `PaperKey`
  is the skeleton's `paper_key` (F2; it must exist before the first tag is written, because tags are
  permanent) and key is
  `label:<\label>` when the node has a label, else `fp:<fp>` plus `#k` for the k-th identical body
  under the same parent. Printed numbers are never part of identity. Same files and builder give
  byte-identical output.
- **Deferred (version tracking).** Until it is built, a session keeps the skeleton it was made
  with: the blob is pinned in the ledger, no newer arXiv version is fetched into the session, and a
  newer builder is used for new sessions only. So no tag is ever carried and nothing can be attached
  to the wrong node. The design for later: on a
  new paper version or new builder, the builder receives the previous skeleton and reuses an old tag
  when a node matches by, in order: same label and kind class; same fp; same kind class with token
  similarity at least 0.8, a unique best match and the runner-up below 0.7. A carried node whose fp
  differs is `changed`. Unmatched old tags are `retired`; their records stay readable in a tray.
- Every layer-2 record stores `node_fp`. The view computes `stale = (node_fp != current fp)`. Stale
  records are shown with a diff and lose their badge colour; nothing is silently re-attached.
- Model-created nodes: `tag = "m-" + base32(sha256(attach_to | kind | normalised first quote))[..10]`.
  The same claim found again in a re-run gets the same tag, so a re-run updates instead of duplicating.

### 4.3 Layer 2: interpretation (small, additive records keyed by tag)

| Record | Fields | Written by |
|---|---|---|
| Reading item | `tag` (or `new {id, attach_to, kind}`), `role` (main_result, supporting, technical, background, standard_tool), `gist` (<= 400 chars), `purpose` (<= 300), `hypotheses [{text, anchor}]`, `anchors` (1-3, at least one inside the node's spans), `node_fp` | model, `record_map_reading` |
| Authors' claim | `tag`, `label` (new, restated, background, not_stated), `relation_note`, `attributed_to [bib key]`, `anchors` (required unless not_stated; this paper only), app-filled `cues` | model, same tool |
| Model edge | `from`, `to`, `role` (statement, proof, argument), `basis` (explicit, inferred), `reason`, `anchor` | model, same tool |
| Context card | `tag`, `type` (symbol, prerequisite, cited_result), `title`, `body` (<= 900), `anchors`, `hypotheses [{text, anchor}]`, `bib_key`, app-filled `origin` | model, `record_context_cards`; host for deterministic cited results |
| Reader override (F16a) | `tag`, `fix`: `kind` (for example "this is a theorem"), `span` (where the statement or the proof ends), `not_a_node`, `number`; `node_fp` | reader. Applied by the view on top of the skeleton, which itself stays a pure function of the files. Shown as "corrected by you" |
| Prior-work record (deferred, V14) | `tag`, `status` (the shared `PriorWork` values of F12: judged_same, judged_partial, judged_other_name, abstract_only, none_in_scope, incomplete), `evidence [{paper_anchor, external_anchor, relation}]`, `note`, app-filled `coverage` (the shared `Coverage` record of F12: what was searched and what was read), app-filled `abstract_hits [{paper: PaperRef, quote}]`, app-filled `checker {provider, model, independence}` (different_provider, different_model, same_model) | model, `record_prior_work_check` |
| Cited lookup | `bib_key`, `locator`, `resolved` (a `PaperRef`), `external_source_id`, `external_tag`, `status`, `number_basis` | host |
| Note / pin | `tag`, `text`, or `pin: main_result | not_main_result` (and, deferred with V14, `confirm_prior_work`: the reader confirms a "Model judges" status) | reader |

Status names in the prior-work record changed with the reconciliation: `earlier_statement` is now
`judged_same`, `partly_earlier` is `judged_partial`, `none_found` is `none_in_scope`, `inconclusive`
is `incomplete`; `judged_other_name` is added; the `found_not_read` list is now `abstract_hits`,
shown under the status `abstract_only`; `checker.same_as_reader` is now `checker.independence`.

Derived, never stored (pure functions over the skeleton and the records):

- **Map view**: latest record per (tag, field); earlier values kept as history and shown on request.
  Reader pins override the model's `role`.
- **Display badges**: the authors' side from `status_badge(authors_claim)`; the other side is the
  status sentence of the Explore session this result was handed to, read from that session (it was
  written there by the shared wording module of F12, whose output never contains "new" or
  "novel"). No merged verdict is ever computed.
- **Skip plan**: statement closure and proof closure of the main results over `uses` edges, mapped
  to sections: "needed to read the main statements", "needed to check the proofs", "not referred
  to by any main result". Carries a basis: "by explicit references" or "includes model-inferred
  links". Empty when no main result is known.
- **Symbol lookup**: last `SymbolDef` with the same `norm` before the reading position.

### 4.4 Ledger events (all new kinds; no existing kind changes)

Storage stays per mode; two kinds in this table are shared with the other modes and are defined by
shared packages: `context.supplied` (F4) and `handoff.sent` (F7).

| Kind | Writer | Payload (main fields) |
|---|---|---|
| `investigation.skeleton` | host | source_id, source_seq, builder, paper_key, version, quality, counts, `blob` (hash of the skeleton JSON via `put_blob`), prev_skeleton_seq, carry counts |
| `investigation.task_requested` | host | task (orient, open, context, prior_work), tags, depth, focus, source_id, skeleton_seq |
| `investigation.task_finished` | host | request_seq, status (done, partial, failed, stopped, interrupted), counts, error. On screen: done is "Done", partial and stopped are "Paused: (reason)" with Continue, interrupted is "Interrupted" with Retry, failed is "Failed" with Retry (F9) |
| `context.supplied` (shared kind, F4; previously `investigation.context_supplied`) | host | run_id (here the task's request_seq), caller, ranges `[{source_id, path, file_sha256, start_line, end_line}]` of the text the app put in front of the model |
| `investigation.reading` | `record_map_reading` | request_seq, skeleton_seq, items, edges, rejected, anchor_report |
| `investigation.context` | `record_context_cards` or host | request_seq, skeleton_seq, tag, cards with origin |
| `investigation.prior_work` (previously `investigation.check`; deferred with V14, kind name reserved) | `record_prior_work_check` | request_seq, skeleton_seq, tag, status, evidence, app-filled fields |
| `investigation.override` (F16a) | host, on a reader action | the reader override record of 4.3 |
| `lessons.supplied` (shared kind, F16) | host, at the start of a task | `{run_id, lesson_ids, blob}`: the notes from the reader's earlier corrections that were put in front of the model |
| `investigation.cited_lookup` | host | Cited lookup record |
| `investigation.note` | host | tag, node_fp, text or pin or confirmation of a prior-work judgement |
| `handoff.sent` (shared kind, F7) | host | the `NodeRef` of the node handed over, target mode, child session |
| `investigation.version_linked` (deferred with version tracking) | host | from/to source ids and skeleton seqs |
| `investigation.exported`, `investigation.imported` | host | format, sha256; on import: source_match, items_matched, items_dropped |

Unchanged and still read: `investigation.started`, `.paper`, `.scope`, `.summary`, `.map`,
`.branch*`, `.conventions*`, `.question_requested`, `.answer`, `.evidence`, `.requested`, `.result`,
`.correction`, `.linked`, `.handoff`. The old `.handoff` kind is not extended: a new hand-over
writes one shared `handoff.packet` (F7) into the child session and `handoff.sent` into this one.
Sessions created by an old `investigation.handoff` stay readable and stay Research sessions.

## 5. Tools

### 5.1 Model tools (3 new ids now, a 4th deferred; effect `ReadWorkspace`, same approval settings as the existing recorders)

The tools live in the tool profile `investigation/2` (F1), which is today's research list plus
the first three below. `record_prior_work_check` is deferred with V14: it is not in the profile's
list, its id is named under `may_gain`, and it joins when V14 is built, without a new profile.
None is added to the shared list (`register_research_tools`), so Explore and Solve
sessions never carry them. A session with the older profile `investigation/1` gains the three on its
next resume.

| Tool | Input | Output | Used in |
|---|---|---|---|
| `read_map_node` | `source_id`, `node` (a tag, or a printed reference such as "Theorem 4.5"), `include` (statement, proof, uses, used_by, symbols), `max_chars` | tag, kind, number, spans, line-numbered text per file with file hash, uses by role, used_by, symbols, truncated flag | every task; the question turn |
| `record_map_reading` | `request_seq`, `source_id`, `items` (<= 16), `edges` (<= 32), `limitations` (<= 8), `done` | per item: accepted, or `{index, code, message, true_text}`; assigned tags for new nodes | orient, open |
| `record_context_cards` | `request_seq`, `source_id`, `tag`, `cards` (<= 8) | per card: accepted with the origin the app assigned, or the reason | context, open |
| `record_prior_work_check` (deferred with V14; the tool id keeps its name) | `request_seq`, `source_id`, `tag`, `status`, `evidence` (<= 6), `note` | accepted or faults; the `coverage` record as the app filled it | prior work |

Several recorder calls per task are expected (save early, save often). Existing tools used as they
are: `read_paper_lines`, `search_paper_passages`, `arxiv_search`, `literature_search` (network,
through the existing approval path and the shared network gate of F3; their definitions stay byte
for byte under F11; the approval is asked on the one approval card all modes share, which lists
each grant it will record, overview 3.1 row 13), `note_evidence`, the question turn. The four old recorders stay
in the profile and closed during new tasks. With V14 deferred no task of this mode calls the two
search tools; they stay in the profile for the question turn.

What the app injects so the common case needs no read calls: for orient, the abstract, introduction,
conclusion and the list of theorem-like headings; for open, the node's statement, its proof and the
statements of what it uses. All injected text is line-numbered per file, wrapped as quoted source
material, and logged in the shared event `context.supplied` (F4).

### 5.2 Host routes (user-initiated, no model)

| Route | Does |
|---|---|
| `POST /api/sessions/{id}/investigation/open` `{arxiv_id / doi / url / source_id / library_key}` | Returns at once with a job id. Job steps, each visible: metadata, e-print TeX, ingest, skeleton. Fetching is the shared `paper_fetch` (F3: the library first, which includes files of the user's own sources (F13) and uses no network for them; then arXiv e-print, then arXiv PDF; a DOI with no arXiv version and no library file returns `needs_file`). A library entry that has the unpacked arXiv TeX beside its PDF is opened from the TeX. PDF text extraction is the shared job of F14a, on a background thread with page progress, kept once per file |
| `GET .../investigation/skeleton?seq=` | Skeleton blob, ETag = blob hash. Never part of the status poll |
| `GET .../investigation/mapview` | Folded records, badges, skip plan, task states, carry tray, in the shared `GraphView` shape (F6); no node text; ETag = ledger head |
| `POST .../investigation/tasks` `{task, tags, depth?, focus?}` | Starts one task through the existing turn thread, with the task's model-call cap as `max_turns` |
| `POST .../investigation/cited` `{tag, bib_key, locator}` | Deterministic cited-result lookup (6.9) |
| `POST .../investigation/notes` | Note or pin |
| `POST .../investigation/version` (deferred) | Fetch another arXiv version, build with carry |
| `POST .../investigation/overrides` | A reader override (4.3) |
| `GET .../investigation/export?format=rsmap / md / html`, `POST .../investigation/import` | Section 6.11 |

Approval rule, shared by all modes (overview 2.4): a network request runs without an approval card
only when the reader's own click or keystroke named exactly that request. The `open` and `cited`
routes are such requests. Anything a model composes goes through the real approver.
Hand-overs to and from other modes use the routes of F7, not routes of this mode.

### 5.3 Model-call budget

| Action | Model calls (cap) |
|---|---|
| First screen, node open (block A), symbol, cited statement, skip plan, whole map, export | 0 |
| Orient | 8 |
| Open node | 10 |
| Context | 8 |
| "Search wider in Explore" | 0 in this mode; Explore's preset decides the rest |
| Prior work inside Investigate (deferred, V14): references only | 10 |
| Prior work inside Investigate (deferred, V14): search wider | 16, and at most 6 search calls and 3 fetched papers |

Model calls of a task are counted from ledger events by the shared function of F9; this mode has no
counter of its own.

## 6. Rules enforced in code, and the tests that prove them

None of these depends on prompt wording.

| # | Rule | Where | Test |
|---|---|---|---|
| 6.1 | Deterministic first: no task can be requested until an `investigation.skeleton` exists for the pinned source | task route | After `open`, the ledger has a skeleton event and zero `model.request` events |
| 6.2 | Skeleton is a pure function of (files, builder, previous skeleton). The blob hash is in the ledger; a mismatch triggers a rebuild from hash-pinned sources; a rebuild that differs under the same builder is an error and no interpretation is drawn on it | builder, mapview | Two builds byte-identical; CRLF vs LF and comment-only edits keep tags; spans nest; each proof proves at most one node |
| 6.3 | Recorder gating: a new recorder accepts only while the newest marker is a `task_requested` of a matching task with the same `request_seq`; old recorders are closed then, and the reverse. The marker kinds live in one shared constant used by all three existing lists | `investigation_markers.rs` (new), the three lists | Table test: 4 old and 3 new recorders against every marker kind |
| 6.4 | Tag scope: items may name only tags in the skeleton pinned by the request; for open, context and prior work, only the requested tag, its descendants and (for edges) the nodes it uses. Unknown tag: that item is dropped, with nearby valid tags in the message | recorders | Stale tag after a version change is rejected per item |
| 6.5 | Containment: a reading item needs one matched anchor inside the node's spans; `main_result` is refused on proof, remark, example, figure, table, bibliography; a model edge with role "proof" needs its anchor inside a proof of the target (role "argument" otherwise), role "statement" inside the target's statement | `record_map_reading` | Same edge with the anchor moved out of the proof is rejected (`scope_mismatch`) |
| 6.6 | Authors' claim: new, restated or background needs a matched quote from this paper; `attributed_to` keys must exist in the skeleton bibliography; the app records which cue words occur in the quoted lines and the UI shows "no explicit cue in the quoted lines" when none does | recorder, view | Each rejection; cue list computed |
| 6.7 | **Deferred with V14; kept as the design for later.** Prior work: the status is one of the shared `PriorWork` values (F12). `coverage`, `abstract_hits` and `checker` are filled by the app from this task's own tool calls (matched by call id and main-agent caller) and are app-only fields; `checker.independence` is computed by the one host function of F12. `judged_same`, `judged_partial` and `judged_other_name` need at least one pair with both anchors matched in full text, the external one in an ingested non-primary source, and for `judged_other_name` the other name must occur in the quote. A hit known only from its abstract never supports a `judged` status: it is listed under `abstract_only` with the verbatim abstract quote and the words "possibly related". `none_in_scope` is accepted only when the shared coverage gate of F12 passes (for the "references only" depth: every reference is read, or listed as not available); when the gate fails the status is `incomplete` and the sentence lists the conditions that failed. The badge text is produced by the shared wording module: it starts "Model judges:" for a `judged` status until the reader confirms it with one click (stored as one note event), and lists what was searched for a negative one | recorder, F12 (`search_coverage.rs`, `prior_work.rs`, `prior_work_wording.rs`), notes route | Each rejection; the F12 property test that no wording contains "new" or "novel"; a hand-run search does not satisfy the gate; an abstract-only hit cannot produce a `judged` status; a confirmation changes the badge text and no other record |
| 6.8 | Provenance is assigned by code. Origin is "paper", "external" or "model_background" from where the anchors matched. An item whose anchors fail is dropped, never downgraded. `model_background` is allowed only for type prerequisite, at most 300 characters and 5 per node | `record_context_cards` | Each branch |
| 6.9 | (When the cited paper exists only as PDF, from the library or from arXiv, the lookup runs on its PDF skeleton: a number printed in the PDF text counts as "printed", but because the statement's end is estimated the result is shown as `candidates` with the page, never as located.) Cited results: a `cited_result` card needs a bib key in the skeleton, a `cited_lookup` for that key, and a matched anchor inside the located external node; each hypothesis needs its own anchor there. The deterministic lookup shows a statement as located only when the cited paper's number is "confirmed" (printed in the source, or computed and equal to the arXiv HTML number, or the only node of that kind with that number under a single shared counter); otherwise status is `candidates` and the reader picks. Statuses: found, candidates, paper_found_locator_not_found, no_tex, unresolved | cited route, recorder | Fixture where the computed number is off by one must yield `candidates`, not a statement |
| 6.10 | Per-item acceptance everywhere: one bad item never loses a call; the result names the index, the code and the true text of the cited lines | recorders (existing AnchorReport pattern) | Mixed-validity call saves the valid items |
| 6.11 | Import is re-matching: the importing app obtains the same source (arXiv id and version, or a file with the same hash), rebuilds the skeleton, and passes every record through the same recorder checks; failures are dropped and counted; imported items carry an "imported" mark. With no matching source the map opens read-only under "quotes not matched on this machine" | import route | Round trip; tampered quote is dropped |
| 6.12 | Additive records: nothing is deleted or replaced; a re-run cannot remove notes, pins, model nodes, labels or cards. A changed value is shown with its earlier value | fold | Orient run twice with different results; run-1 notes and children still attached |
| 6.13 | **Deferred with version tracking.** Until then the rule is: a session never changes its skeleton (4.2). Test now: opening a session made with builder "tex-1" in a build that has "tex-2" shows the stored skeleton unchanged. For later: Carry-over is conservative (thresholds in 4.2); labels on changed nodes show "written for v3, statement changed" and lose colour | tags, view | Label renamed; theorem reordered; split in two; one-word edit; deleted; builder bump |
| 6.14 | Skip plan honesty: text always says "by cross-references"; basis switches to "includes model-inferred links" when one changes a section's status; heuristic text mentions do not count; nothing is dimmed with no main result | `skip_plan` | Each state |
| 6.15 | Caps: `max_turns` per task from 5.3; model calls are counted from ledger events by the shared function of F9 (`run_budget.rs`). The finish hook writes `partial` when the run ended at the cap or aborted but at least one record with that `request_seq` exists, and the screen then shows "Paused: (reason)" with Continue. After Solve package S12 makes the runner outcome `Paused` mode-neutral, this mode uses it and the repair in the hook is dropped. Search and fetch counts per prior-work task are enforced inside `arxiv_search` and `literature_search` by counting this task's call ids through F9 (behaviour only, no schema change) | finish hook, F9, tools | Capped run with 2 saved items ends `partial` and shows "Paused"; 7th search is refused |
| 6.16 | External papers per session: default 8 fetched sources, counted by the shared `fetched_sources(events)` of F3 with the limit as a parameter, visible counter, raised only by a reader click (recorded as a note event). Hosts stay the existing allow-list; every outgoing request passes the one process-wide network gate of F3 (arXiv: 1 request per 3 s); existing size caps. This mode has no limiter and no fetcher of its own | F3 (`net_rate.rs`, `paper_fetch.rs`) | 9th fetch refused until raised; gate spacing (F3's own test) |
| 6.17 | Lines sent: a quote is accepted only if it matches and its lines were sent to the caller that cites it. This is the shared `lines_sent` function of F4; it credits `read_map_node` results and `context.supplied` ranges, matched by path and file hash. This mode adds no credit code of its own | F4 (`lines_sent.rs`), called from `investigation_cites.rs` | A citation to injected lines is accepted; one with a wrong hash is not |
| 6.18 | Tool-definition stability: the golden file of the profile `investigation/2` (F0, F1) pins id, def_hash and order of the three new tools and the four old recorders (the fourth new tool joins through `may_gain` when V14 is built); any later schema change must add the previous definition to the profile's `earlier_defs` or the test fails | F0, F1 | Golden test per profile; changed schema rejected on resume |
| 6.19 | Paper text is data: injected text is wrapped as quoted source material; the existing Investigation system prompt rule stays | prompts | Existing test extended to injected text |
| 6.20 | Files at or under 600 lines: all new code in new files. The line-count check in CI is built once in F0 | F0 | F0's check |
| 6.21 | PDF honesty: a skeleton built from PDF text is never quality "full" and always shows its quality line; a node's page is computed from its lines; a quote from a scanned page is shown as "matched in OCR text" with the page image one click away; nothing from OCR text is drawn as mathematics; a formula a model read from a page image is "transcription (model), unconfirmed" and is never a matched quote | `skeleton/pdf.rs`, F14 (`pdf_pages.rs`, `pdf_match.rs`), view | On the PDF fixtures (our own TeX compiled to PDF, and the same as page images): the page of every sampled node is right; a scan page is never reported as a text layer; a hand-copied sentence matches; a model transcription cannot be stored as an anchor |
| 6.22 | Corrections never decide a label: a reader override changes how a node is shown and is marked "corrected by you", and lessons from earlier corrections (F16) change only what the model is told; neither can set an authors' label, a role or an anchor result | view, F16 | A session folded with and without overrides and lessons gives the same recorder verdicts for the same model output; an override on a tag that no longer exists is shown in the tray, not applied |

## 7. Reused, changed and retired in today's code

| Path (under `crates/`) | Lines today | Fate |
|---|---|---|
| `research-tools/src/investigation_anchor.rs` | 597 | Reused unchanged (anchor verification, relocation, app-only field rejection); the new recorders reach it through `verify_quote` of F4 |
| `research-tools/src/investigation.rs` | 576 | Changed by a few lines: its marker list moves to the shared constant. Old recorders stay |
| `research-tools/src/investigation_conventions.rs`, `investigation_evidence.rs` | 381, 398 | Marker lists use the shared constant |
| `research-tools/src/investigation_legacy_defs.rs` | 168 | Reused. F1 moves today's tolerated additions and special cases into the data of the profile `investigation/1`; earlier definitions of the new tools go into `earlier_defs` of `investigation/2`. No new legacy-defs file in this mode |
| `research-tools/src/tex_scan.rs`, `tex_bundle.rs` | 419, 433 | Reused: comment stripping, labels, refs, cites, bib parsing, arXiv id and DOI scanners, entry-point scoring, include resolution |
| `research-tools/src/investigation_outline*.rs` | 564, 285, 582 | Reused as is for question scope; the plain builder feeds the PDF skeleton |
| `research-tools/src/arxiv.rs` | 528 | Reused. Fetching by id is the shared `paper_fetch.rs` of F3, which also makes the behaviour-only edit that puts this file's requests behind the network gate. This mode adds nothing here (the file must not grow) |
| `research-tools/src/paper_lines.rs`, `paper_passages.rs` | 202, - | Unchanged |
| `research-tools/src/lib.rs` | - | Not changed by this mode: the four tools are not added to `register_research_tools`. Their definitions live in `map_defs.rs` and are listed by the profile `investigation/2` (F1) |
| `agent/src/resume.rs` | 579 | Edited only by F1 (tool profiles), once for all modes. This mode makes no edit here; its resume tests are the profile fixtures of F1 |
| `desktop-server/src/investigation.rs` | 579 | Not grown; new routes live in new files, dispatched from `http_srv.rs` |
| `desktop-server/src/investigation_cites.rs` | 472 | Calls the shared `lines_sent` of F4 instead of extending its own list (6.17) |
| `desktop-server/src/investigation_finish.rs` | 105 | New branch for task requests |
| `desktop-server/src/investigation_view.rs` | 217 | Small hook to the new mapview |
| `desktop-server/assets/investigation.js`, `investigation_map.js` | 542, 594 | Not grown; `investigation.js` delegates to new modules; the old map module serves the "Earlier map" section only |
| `desktop-server/assets/investigation_graph.js` | 280 | F6 extracts its layering into the shared `graph_layout.js`; this mode does not extract it a second time |
| `export/src/fork.rs` | 280 | Not changed by this mode. Hand-overs use the shared packet of F7 (`export/src/handoff.rs`), which reuses the source-copy mechanism of this file |
| Old routes `/investigation/branches`, `/rechecks`, `/conventions`; old prompts | | Kept. Retired only by the last package, if the owner agrees |

New Rust files: `research-tools/src/skeleton/{mod, model, flatten, tex_env, tex_theorems, edges,
number, tags, stats, skip, symbols, bib, locate, pdf, html}.rs`,
`investigation_markers.rs`, `map_node_read.rs`, `map_reading.rs`, `map_context.rs`,
`map_prior_work.rs` (previously planned as `map_check.rs`; deferred with V14), `map_defs.rs`, `map_fold.rs`,
`status_badge.rs` (authors' side only); `desktop-server/src/investigation_{open, skeleton,
tasks, task_context, task_prompts, cited, notes, share, legacy_view, handoff}.rs`;
`export/src/paper_map*.rs`.
New UI modules: `investigation_{entry, outline, results, node, node_model, badges, context,
wholemap, skip}.js` and `investigation_shell.css`.

Used from the shared foundation, not built here: `tool_profile.rs` and `resume_profile.rs` (F1),
`paper_key.rs` (F2), `net_rate.rs` and `paper_fetch.rs` (F3; they replace the `arxiv_fetch.rs` this
document planned before), `quote_core.rs` and `lines_sent.rs` (F4), `session_mode.rs` and
`new_menu.js` (F5), `graph_layout.js`, `graph_view.js`, `ui_badge.js`, `run_bar.js` and
`quote_chip.js` (F6), `handoff.rs` (F7), `run_budget.rs` (F9), `library.rs` (F10),
`paper_sources.rs` (F13: opening a paper from the user's own library), `pdf_pages.rs`,
`pdf_firstpage.rs`, `pdf_match.rs` and later `pdf_ocr.rs` (F14: the text, pages and scan handling
that `skeleton/pdf.rs` reads), `writeup.rs` with the adapter `writeup_paper.rs` (F15: reading
notes), `lessons.rs` (F16). `search_coverage.rs`, `prior_work.rs` and `prior_work_wording.rs`
(F12) are used by this mode only when V14 is built. `investigation_wholemap.js`
and `investigation_badges.js` are thin adapters on the F6 parts.

## 8. Migration of existing sessions and def_hash

1. **Events.** Only new kinds are added. The ledger stays append-only; no old session is rewritten.
2. **Tools.** The four existing recorders keep their definitions byte for byte, so their def_hash
   values and the golden constants stay valid. The four new tools are new ids with effect
   `ReadWorkspace` in the tool profile `investigation/2` (F1); three of them now, the fourth
   (`record_prior_work_check`) under `may_gain` until V14 is built. An Investigation session started
   before this change has no profile field and is treated as `investigation/1`; that profile lists
   the four ids under `may_gain`, so the session resumes and can use the new tasks. Resume builds
   the expected definitions from the recorded profile and stays fail-closed. Tools are sorted by id,
   so the order is deterministic. Nothing is removed, so "tools were removed or reordered" cannot
   fire. The four tools are not in the profile `research/1`, so that list stays frozen as Solve
   requires. Which mode a session has is decided by the one shared function `session_mode` (F5):
   the latest host-written marker among `investigation.started`, `idea.started` and `solve.started`
   wins; none means Research. Only the host may write a marker.
3. **Downgrade.** A session started after F1 records its tool profile and will not resume in an
   older build. State this in the release notes once; the same note covers all modes.
4. **Old sessions on open.** If the pinned paper has no skeleton event, the host builds one (no
   model) and appends it, provided the source files still match the receipt hashes; otherwise the
   session opens in the old view with a note.
5. **Old maps.** A read-side adapter attaches each node of `investigation.map` and
   `investigation.branch` to the innermost skeleton node whose spans contain its first anchor,
   under an "Earlier map" heading, with display id `legacy:<map_seq>:<id>`. Old edges are shown with
   role "unknown (older map)". `investigation.correction` appears as a note. Nodes that cannot be
   placed go to "Unplaced (older map)". The old overview and conventions panels stay.
6. **Buttons.** In old sessions "Find main results" is relabelled "Rebuild earlier map"; the default
   action is Orient, which only adds records and so cannot orphan anything. The menu button "New
   investigation" becomes the Investigate card of the home screen (F5); the session rail tags these
   sessions "Investigate".
7. **Later schema changes** to the new tools add the previous definition to the profile's
   `earlier_defs` (F1); the golden test fails otherwise.
8. **Builder upgrades** ("tex-1" to "tex-2"): while version tracking is deferred a session keeps
   the skeleton it was made with, and a better parser is used for new sessions only. Later a
   builder upgrade becomes a version step with a carry block, so tags and everything attached
   survive a better parser.
9. **Tests required before merge:** resume of the recorded fixture ledgers of evaluation set R
   (research, investigation before pointer citations, investigation current, hand-off child; kept
   in the fixed tier of F8 and run in CI) with all new code present; mapview of the two
   investigation ledgers shows attached legacy nodes and the same content as before; table test of
   recorder gating.

## 9. Work packages, in order

Sizes are in days, as in the overview: S up to 2 days, M up to 1 week, L 1 to 2 weeks. **The first
three are marked.** They are still V1, V2 and V3. Two shared packages must be merged before them:
the paper key (F2) before V2, because tags are permanent and are hashed from it, and the shared
graph and UI parts (F6) before the screen half of V3.

Packages F0 to F16 are the shared foundation of `00-overview.md` section 2. They are built once for
all modes and are not repeated in this table; the "Depends on" column names them. Order across the
build lanes follows overview section 6: V1 (weeks 1 and 2), V2 (week 3), V3 (weeks 3 and 4, first
milestone: skeleton on screen with zero model calls), V4 and V5 (week 5), V6 (weeks 6 and 7),
V17 (the PDF skeleton, week 7, built by the foundation lane on F14a) with V13 beside it, V7
(week 8), then V8, V9, V10 (notes only), V11 and the rest. V14 and the version half of V10 are
deferred and are not in the order. Edits to `http_srv.rs`,
`resume.rs`, `routes_session.rs` and `turn_runner.rs` are made by the foundation lane, so a package
below that needs a route registered hands that one edit to that lane.

| Id | Title | What | Main files | Size | Depends on |
|---|---|---|---|---|---|
| **V1 (first)** | Skeleton core for TeX | Types; in-memory `\input` flattening with origin map; comment-safe scanner with environment stack: sectioning, `\newtheorem` / `\declaretheorem` table mapped to kinds, theorem-like environments with title and heading citations, proofs, labelled equations, figures, tables. 8 self-written TeX fixtures with expected JSON (amsthm shared counters, custom names, multi-file, revtex with no theorems, detached proof, unlabelled theorems, two versions) and a timing helper | `skeleton/{mod, model, flatten, tex_env, tex_theorems}.rs`, `tests/fixtures/skeleton/*`, `tests/skeleton_tex.rs` | L | none |
| **V2 (second)** | Edges, tags, reading stats | Positional `uses` edges (KnowTeX rules), proof linking, lifted edges, counter simulation with `number_basis`, first-build tags hashed from the `PaperKey` of F2, per-section words and minutes, bibliography with ids and locators | `skeleton/{edges, number, tags, stats, bib}.rs`, tests | M | V1, F2 |
| **V3 (third)** | Skeleton on screen | `investigation.skeleton` event with blob; build on pin and on opening an old session; skeleton and mapview routes (mapview in the shared `GraphView` shape); UI on the shared node-card shell and badges of F6: outline tree, results list with rendered statements, node card block A, quality line. Works on any already ingested TeX source. Measures build time on the fixtures | `investigation_skeleton.rs`, `http_srv.rs` (route registration only, made through the foundation lane), `investigation_{outline, results, node}.js`, `investigation_shell.css`, mjs tests | L | V2, F6 |
| V4 | Open by arXiv id, TeX first | Only the mode-specific remainder: the `open` route with job progress (it also accepts a library key, so a paper of the user's own sources opens with no network once F13a exists); the input box of the Investigate card. Moving PDF extraction to a background job is F14a, not this package. Fetching itself (id normalisation, metadata, versions, e-print by id through the existing ingest, DOI to arXiv id, the 3-second network gate, the cache by id and version) is F3 and is not built here | `investigation_open.rs`, `investigation_entry.js` | M | V3, F3 |
| V5 | Skip plan, symbols | `skip_plan` with basis; symbol definitions and nearest-definition lookup; reading-plan strip; symbol popovers | `skeleton/{skip, symbols}.rs`, `investigation_skip.js` | M | V2, V3 |
| V6 | Task markers, `read_map_node`, `record_map_reading` | Shared marker constant; task requested / finished; context injection logged as `context.supplied` (F4); the two tools, listed in the profile `investigation/2` (F1); rules 6.3-6.5, 6.10, 6.12, 6.15 (the finish hook and the caps; the counting is F9); fold. No longer in this package: the `resume.rs` edit (F1), rule 6.17 (F4), rule 6.18 (F0 and F1), model-call counting (F9) | `investigation_markers.rs`, `map_node_read.rs`, `map_reading.rs`, `map_defs.rs`, `map_fold.rs`, `investigation_tasks.rs`, `investigation_task_context.rs` | L | V3, F1, F4, F9 |
| V7 | Orient | Deterministic cues; orient prompt; main-result markers, gists, authors' half-badge with quote; reader pins; auto-start setting | `investigation_task_prompts.rs`, `status_badge.rs` (authors' side), `investigation_badges.js`, `investigation_notes.rs` | M | V6 |
| V8 | Open any node, any depth | Open prompt; model nodes with content-based tags; block B with streaming and the shared run bar (Stop, Paused, Continue); breadcrumb | `map_reading.rs`, `investigation_node_model.js` | M | V7 |
| V9 | Old sessions | Legacy adapter, "Earlier map", relabelled button, fixture ledgers | `investigation_legacy_view.rs`, tests | M | V6 |
| V10 | Notes now; stable tags across versions **deferred** | Now: notes on a node (text, shown on the card and in the reading notes). Deferred with version tracking: carry matching, version route (versions fetched through F3), change summary, stale banners and diff | `investigation_notes.rs`, UI; later `skeleton/tags.rs`, `investigation_open.rs` | S now; M later | V7 |
| V11 | Cited result without a model | (A cited paper found in the user's library is used from there, with no fetch; one that exists only as PDF is located on its PDF skeleton and shown as candidates with the page, rule 6.9.) Resolve the bib entry to a `PaperRef`, fetch TeX through F3, build that skeleton once and keep it in the library's `skeleton` table (F10) so it is not rebuilt per session, locate with rule 6.9, all statuses, picker, fetch budget | `investigation_cited.rs`, `skeleton/locate.rs`, `investigation_context.js` | M | V4, F10 |
| V12 | Context cards | `record_context_cards` with rule 6.8; "Prerequisites" and "Compare hypotheses" tasks; drawer; hide-unchecked toggle | `map_context.rs`, UI | M | V8, V11 |
| V13 | Whole map | Adapter from the mapview to the shared graph view; layering, lanes, folding above 150 nodes and drawing are F6 and are not built here. Mode-specific: line weight and a toggle for statement uses and proof uses, novelty toggles (these wait for V7), dimming, section collapse | `investigation_wholemap.js` | M | V5, F6 |
| V14 | Prior work: **deferred** (owner decision of 2026-11-25; the "Search wider in Explore" button moved to V16) | Not in the build order. When it returns: `record_prior_work_check` with rule 6.7; two depths; search and fetch caps; one-click confirmation; right half-badge and side-by-side view; "Search wider in Explore" button on a result (a packet of F7 that carries the coverage and the earlier candidates); batch with a cost note. Coverage record and gate, status values and wording are F12 and are not built here | `map_prior_work.rs`, `investigation_badges.js` | L | V7, V11, F12; F7 for the Explore button |
| V15 | Export and import | `.rsmap`, standalone HTML; import with rule 6.11. The readable Markdown or TeX output is the reading-notes write-up of F15 (`writeup_paper.rs`), not this package | `export/src/paper_map*.rs`, `investigation_share.rs` | M | V8 |
| V16 | Hand-off to Explore and Solve | Three buttons on top of the shared hand-off packet (F7): "Derive in Solve", "Has this been extended?" and, on the right half-badge of a result, "Search wider in Explore" (moved here from V14; it is this mode's only path to earlier literature for now, and the half-badge afterwards shows Explore's status sentence by reading the child's `idea.verdict`). This package only fills the packet from a node: the statement with its quotes; the statement closure (definitions, assumptions, equations it uses) as context; symbols; the paper's conventions; the reader's notes; gists only as model text; for Explore the paper as origin and its resolved bibliography as seeds. The task kind sent to Solve uses Solve's names: `prove` for a theorem-like node, `calculate` for an equation, `check_derivation` for a proof node. Receiving side: a packet whose target is Investigate opens the paper, and each earlier quote that matches again appears as a note on the innermost node whose span contains it. Nothing is written back: the node shows a chip such as "Opened in Solve: Done with open items" by reading the child session. Order note: the overview's week-8 milestone (one packet opens a Solve setup card from an Investigate node) comes before V8, so the smallest filler for "Derive in Solve" (statement, statement closure, symbols and conventions of a skeleton node; no notes, no model text) is written with F7 in week 8, in `investigation_handoff.rs`; this package adds the rest | `investigation_handoff.rs`, `investigation_node.js` | S | V8, F7 |
| V17 | PDF skeleton (**moved up: week 7; essential**) | On the page-broken text of F14a: section headings by numbering pattern, theorem-like headings and "Proof." by pattern, numbered equations at the margin, the reference list as bibliography entries with any DOI or arXiv id found, "[12]" as citations, "Theorem 2.1" and "Eq. (3.4)" as heuristic uses; page for every node; quality "weak" or "partial" with its reason, "(scan)" for scanned pages; rule 6.21. Fixtures: our own TeX fixtures compiled to PDF and the same as page images. Pages with no text at all wait for F14b | `skeleton/pdf.rs`, tests | M | V2, F14a |
| V18 | arXiv HTML skeleton | Second source when TeX is missing or weak; also confirms computed numbers | `skeleton/html.rs` | M | V4, decision 12 |
| V19 | Retire old recorders | A later Investigate tool profile that no longer lists the old recorders (profile data of F1, not a new hand-written rule in `resume.rs`), then removal of old map and branch paths | profile data, the old files | M | F1, one release after V9, decision 11 |

Evaluation. This mode has no acceptance set of its own any more; it uses the shared evaluation set
of `00-overview.md` section 7, run by the shared harness (F8). The parts that test this mode:

| Set | Content | Pass rule or measure |
|---|---|---|
| P. Papers, structure | 10 arXiv ids: at least 4 physics papers with no theorem environments, 1 over 80 pages, 1 multi-file with custom theorem macros (the PDF-only case is now its own set, P6) | Time from input to skeleton (target under 10 s for TeX); recall of theorem-like statements and of sections (below 0.9 on TeX papers with theorem environments blocks the first milestone); precision of 30 sampled "uses" edges, statement and proof roles separately |
| P6. PDF-only and old papers | 8 papers: 2 journal PDFs with a text layer and no arXiv version, 2 two-column, 2 scans from before 1990 with a publisher's text layer, 2 scans with no text (one heavy with formulas) | A hand-copied sentence matches in the stored text (recorded per page basis); a quote chip opening the wrong page must be 0; a scanned page reported as a plain text layer must be 0; recall of sections and theorem-like statements, recorded and shown beside the quality line; character errors on sampled scan lines; reading-order errors on two-column pages |
| P2. Versions | deferred with version tracking | - |
| P3. Authors' labels | 5 of the papers in P, 6 to 10 results each | Agreement of the authors' label with the key; share of labels whose quote contains a cue |
| P4. Restated results | deferred with V14. Two of its cases are now chains in set C: a restated result handed to Explore by "Search wider in Explore" | - |
| P5. Cited statements | 10 citations with a locator | A wrong statement shown as located must be 0 (a candidates list is acceptable) |
| C. Chains | 3 from a node of an evaluation paper to a Solve problem; 4 from a result to an Explore idea (2 with a known follow-up paper, 2 restated results whose earlier source is known) | Quotes surviving re-matching after a hand-over of unchanged bytes must be 100 percent; for the two restated results, whether Explore finds the known earlier source |
| B. Own library | A self-made vault, folder and `.bib` file with planted cases | A paper opens from the library with no network; the rest is F13's (overview 7.3) |
| W, K | Reading notes of one paper; scripted corrections | Overview 7.3 |
| R. Old sessions | 4 ledgers: research, investigation before pointer citations, investigation current, hand-off child | Must resume and show the same content; in CI |

Two tiers, as in the overview: a fixed tier inside the repository (`tests/fixtures/eval/`:
self-written TeX, recorded service responses, a scripted model, old ledgers; no third-party paper
text; runs in CI) and a live tier outside the repository (a manifest of arXiv ids with hand-made
answer keys; papers are fetched, never stored in the repository; run by hand before each milestone,
one sheet per run in `docs/design/eval-2026-11/`). The owner chooses the papers; someone who is not
the builder writes the answer keys. Model calls and tokens per task are recorded against the caps
of 5.3, and a property test checks that no view, export or write-up shows "verified" at all, or
"new", "novel" or "proved" outside the allowed places. The 8 self-written fixtures of V1 stay with V1 as unit tests.

## 10. Free core versus optional add-ons

| Free core (no paid service) | Optional add-on |
|---|---|
| Skeleton from TeX, arXiv HTML and PDF text, scanned old papers included (page links, own OCR once F14b exists); outline, results list, node cards, whole map, skip plan, symbols | Heavy PDF parsing (Docling, GROBID): no longer needed for the normal case |
| Papers from the user's own library: a Kandinsky vault, a folder, a `.bib` file (F13) | |
| Cited-result lookup from arXiv TeX | Publisher full text through an institution's access |
| Orient, open and context tasks on the user's own model access | Hosted model access for users with none |
| Earlier work through "Search wider in Explore" (Explore's free services) | Optional free Semantic Scholar key field; larger citation databases; a theorem-search service |
| Notes, reading notes as a write-up (F15), export, import, file sharing | A hosted library of maps for well-known papers |

With no model connected the mode still delivers stages A to C, F, G, J, K and L.

## 11. What is borrowed from where

| Source | Licence | What | How |
|---|---|---|---|
| KnowTeX | Apache-2.0 | Rules turning `\ref`, `\eqref`, "Proof of Theorem ..." into dependency edges | Re-coded in Rust in `skeleton/edges.rs`, credited in the file header |
| leanblueprint | Apache-2.0 | "Uses" on the statement separate from "uses" on the proof; status from ancestors; dependency-graph page | Idea |
| Gerby / Stacks Project | MIT | Permanent tags independent of numbering | Idea |
| arxiv-to-prompt | MIT | `\input` flattening with a line-origin map | Idea (entry-point scoring already exists in `tex_bundle.rs`) |
| arxiv.py | MIT | Three-second spacing and backoff | Idea, built once for all modes in F3 |
| arXiv HTML (LaTeXML output) | service | Second structure source; confirmation of theorem numbers | Called |
| ScholarPhi | Apache-2.0 | Click a symbol, see the nearest definition; dim what is irrelevant | Idea |
| NodeGraph | MIT | Prompt shape for the few contributions only this paper makes | Idea |
| UKPLab novelty assessment | Apache-2.0 | Authors' claims kept apart from the comparison; the paper's own references as the first pool | Idea |
| OpenNovelty | Apache-2.0 | Per-claim label with a required evidence snippet | Idea, enforced by the recorder |
| PaperQA2 | Apache-2.0 | Reading the cited paper against the citing sentence | Idea, for "Compare hypotheses" |
| SciCite | Apache-2.0 | Citation intent, extended with "source of a restated result" | Idea |
| langextract | Apache-2.0 | Every extraction aligned to the source, the rest flagged | Already the app's anchor rule; extended to per-item acceptance |
| INSPIRE-HEP, arXiv, Crossref, Semantic Scholar | services | Reference resolution and search | Existing tools and allow-list |
| re:Search itself | own | Anchor verification, recorder gating, golden def_hash tests, `read_paper_lines`, question turn, graph layering, TeX scanners | Reused |
| Kandinsky (the owner's notes app) | AGPL-3.0, own | Papers kept as files with a derived `refs.bib`; PDFs used in place; first-page metadata for PDFs with no identifier; PDF text cached per file with page breaks; column geometry; Tesseract in a local helper for scans | Approach now; code if the owner allows (overview 8.3, question 2). Shared packages F13 and F14 |

Not taken: texlab and tree-sitter-latex (a small own scanner avoids a C grammar build on Windows;
revisit only if the fixtures show it failing); latexml-oxide (needs a TeX installation); Docling and
GROBID (add-ons); TheoremSearch (no licence or terms). Nothing here depends on the move to AGPL-3.0.

## 12. Risks

| Risk | Effect | Mitigation | What remains |
|---|---|---|---|
| TeX in the wild: macros wrapping theorem environments, plain TeX, class-dependent numbering | Missing or misnumbered nodes | Quality line, `number_basis`, fixtures first, HTML as second source | Some papers get sections and equations only |
| PDF text in the wild: two-column reading order, running headers, footnotes inside sentences, formulas as scattered glyphs | Wrong statement boundaries; quotes that do not match | F14a measures two extractors and picks one; matching ignores ligatures and line-end hyphens; quality line always shown; set P6 | Formula-heavy statements stay poor in PDF text; the page image is the fallback |
| Scans | OCR errors pass as the paper's words; formulas are garbage | Tag "OCR" on every quote; page image one click away; nothing from OCR drawn as mathematics; a model's reading of a formula is a marked transcription | Pages with no text at all show only as images until F14b; Tesseract in the Windows installer is not verified |
| The user's library on a synced drive | Files not downloaded yet; a file changes after a session used it | Listing reads no content; a session pins its own copy on open | A first full-text index of a large library downloads everything; it is an explicit action |
| Physics papers with no theorem environments (most of hep-th) | Empty results list until Orient | Outline, minutes, labelled equations and abstract still appear at once; Orient creates result nodes | The weakest case; it is in evaluation set P |
| Dependencies are only cross-references | "Skippable" can be wrong | Wording "by cross-references"; never "not needed"; model-inferred edges update the plan and change the basis line | Prose dependence is missed |
| Earlier work only through Explore (V14 deferred) | A second session for a question the reader asks at a node; no "references only" comparison against the paper's own bibliography | One click; the bibliography travels as seeds; Explore's status is shown back on the result | The quick references-only check is missing until V14. The false-negative rate is measured on the P-to-I chains of set C |
| Cited-result numbering differs between arXiv and journal | Wrong statement shown | Rule 6.9; version note; picker | The reader may still have to pick |
| No version tracking (deferred) | A reader on v3 is not told that v4 exists; notes cannot move to a new version | The version is recorded and shown in the header; the paper can be opened again as a new session | Notes stay with the old session |
| Three more tool schemas (a fourth with V14) | Tokens on every model call of an Investigate session | Kept small; they live in the profile `investigation/2` (F1), so Explore and Solve do not carry them | This mode cannot add a tool before F1 is merged |
| Auto-started Orient | Cost surprises on the user's own account | Per-task caps, estimate before batch, setting to turn auto-start off | |
| arXiv limits and outages | Queued fetches (a cited lookup can take 6 to 10 s) | Fetch cache by id and version in the shared library (F3, F10), one network gate for all modes, visible queue, fetch budget | The gate is shared, so a busy Explore search can delay a cited lookup |
| Files near 600 lines | Merge friction | New files only; shared marker constant removes lines from the old ones | |
| Copyright in exports | Redistributing statements | 300-character quote cap; statements exported only under a permitting licence or on an explicit tick | |
| Large papers and books | Slow SVG and KaTeX in the WebView | Labelled equations only, folding, section collapse above 150 nodes, lazy rendering | Not measured |
| Two generations of map in old sessions | Confusing view | "Earlier map" heading; "Unplaced" group | Large "Unplaced" group for old PDF sessions |
| Timings are estimates | Promises not kept | Measured in V1, V3, V4, V7 and filed as evaluation sheets (F8) before any figure is shown to users | |

## 13. Decisions for the owner

Decisions that concern all three modes are no longer asked here; they are in `00-overview.md`
section 8. Decided on 2026-11-25 (overview 8.1): PDF-only and old papers are essential; bring your
own papers; write-up; corrections; V14 and version tracking deferred; Lean cut, "verified" shown
nowhere, quotes are "matched". Still open and shaping this mode (overview 8.2): 1 (tool profiles),
4 (dashed means not checked by code), 5 (one library), 6 (entry screen), 8 (hand-overs re-match
every quote), 10 (shared event kinds), 11 ("check" renamed to "prior work"), 12 (who writes the
evaluation answer keys); and from overview 8.3: 3 (quotes from OCR text), 4 (own OCR).

| # | Question | Recommended default |
|---|---|---|
| 1 | Should Orient start by itself when a paper opens and a model is connected? | Yes, with a setting to turn it off. The skeleton is on screen first either way |
| 2 | Labels on the authors' side | New / Restated / Background / Not stated, with "extends [x]" as a note on New |
| 3 | May the prior-work side ever show a positive word such as "likely new"? | No. It shows Explore's sentence, which has no such word. (The task of this mode is deferred with V14.) |
| 4 | Wording of a positive prior-work finding | Deferred with V14. For later: "Model judges: earlier statement" / "partly earlier" / "earlier under another name", always with both quotes visible |
| 5 | Unsourced model background in context cards | Allowed for prerequisites only, at most 5 short items per node, dashed style, one toggle hides them; never in the cited-result slot |
| 6 | External papers fetched per session without asking | 8, with a visible counter and one click to allow more |
| 7 | Default depth of the prior-work comparison on first click | Decided for now: the click opens Explore ("Search wider in Explore"). "References only" returns with V14 |
| 8 | A cited theorem whose number cannot be confirmed | Show candidates and let the reader pick; do not guess |
| 9 | What an exported map contains | Structure, gists, labels, quotes up to 300 characters; full statements only when the paper's licence permits or a box is ticked; private notes only when ticked |
| 10 | Old sessions | Upgrade in place on first open (skeleton added, old map under "Earlier map"); nothing recorded changes |
| 11 | Retire the old map and branch recorders (V19)? | Not now. Decide after one release with the new view |
| 12 | arXiv HTML parsing: own tag scanner or a new HTML parser dependency | Own small scanner first; add a dependency only if the fixtures show it failing |
| 13 | PDF-only papers in the free core | Decided: essential. Skeleton from PDF text with page links and a stated quality (V17, F14a), scans included (F14b); heavy parsing stays an optional add-on and is not needed |
| 14 | New arXiv version | Deferred with version tracking. For later: ask, showing what changed; never switch automatically; the old version stays openable |
| 15 | Optional Semantic Scholar key | Keyless by default; an optional key field in settings |
| 16 | A different model for the prior-work comparison than for the reading | Deferred with V14 |
| 19 | A node the scanner got wrong | The reader can fix its kind and its boundaries; the fix is a reader override, marked "corrected by you", and is kept (F16a) |
| 17 | Per-mode tool sets (so Investigate tools are not sent in Explore and Solve) | No longer decided here: it is overview section 8, decision 1 (tool profiles, F1), recommended yes |
| 18 | User-facing word for opening a node | The node opens on click; the model part is headed "Explanation". The word "branch" is dropped in the new view |
