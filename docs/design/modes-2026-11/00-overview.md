# re:Search - three modes, one foundation: cross-mode review

Written 2026-11-25. Reviews `investigate.md`, `explore.md` and `solve.md` in this folder against each other
and against `reSearch-modes-gaps-and-prior-art-2026-11-25.md`. Code facts were re-checked read-only on
`origin/gemini/work/fixed` with `git show` and `git grep`. Nothing was checked out, built or run. All
durations are planning estimates.

**Revised 2026-11-25 after the owner's scope decisions.** Section 0.1 lists what was added, cut and
deferred. Where an older passage of this document or of the three mode documents disagrees with
section 0.1, section 0.1 wins. Kandinsky (`github.com/antimajorana/kandinsky`, the owner's own
notes app, AGPL-3.0-or-later) was read at commit `353368b` for the library design in 2.11; none of
its code was run.

**Licence, decided the same day:** re:Search is AGPL-3.0. `main` already carries it (PR 5, merged
on 2026-11-25: `Cargo.toml` says `AGPL-3.0-only`); the work branches cut before that merge pick it
up from `main`. So Kandinsky's code may be ported directly where it helps (2.11, "Porting").

Package ids: `V..` Investigate, `X..` Explore, `S..` Solve (as in the three documents), `F..` shared
foundation (new in this document). `F0` to `F12` are the original thirteen; `F13` to `F16` were
added by the scope revision.

---

## 0. One-page summary

**The three designs are sound one by one and agree on the principles** (code decides every label; the
model only proposes records that code checks; deterministic content first; old sessions keep resuming).
**They disagree on the plumbing.** Each one built its own version of the same six things. If built as
written, the app would have three resume mechanisms, two arXiv rate limiters (which together break
arXiv's limit), two literature search stacks, three quote-checking entry points, three graph views and
three hand-off formats.

**What to do about it: build small shared packages first or alongside, then the modes.** There were
13 (F0 to F12). The scope revision adds four (F13 to F16), so there are now 17. Section 2.1 lists them.

| Shared piece | Replaces | Why it cannot wait |
|---|---|---|
| F1 tool profiles (one frozen tool list per mode, recorded in the session) | Investigate's "gained tools" edit, Explore's separate router rule, Solve's S1 | Three different edits to the same 579-line resume file; the first mode to add a tool decides for all |
| F2 paper key (one name per paper) | Investigate `family`, Explore `PaperKey` | Investigate's permanent node tags are hashed from it; changing it later changes every tag |
| F3 one network gate and one paper fetcher | Investigate `arxiv_fetch.rs`, Explore `idea_rate.rs` and `idea_fetch.rs` | Two limiters at 1 request per 3 s each is 2 per 3 s |
| F4 quote core (verify a quote, record which lines a model was shown) | Three per-mode paths | Solve's "Cited" path as written does not work: the code it reuses refuses to run outside an Investigation request |
| F6 one graph view | Three graph modules, one of them already over 600 lines | Same picture three times |
| F7 one hand-off packet | Three hand-off payloads | The modes are meant to open each other |

**The contradictions that matter most** (full list in section 3):

1. Solve freezes today's tool list ("fails on any edit"); Investigate adds 4 tools to that same list. Fix: profiles (F1).
2. "Leg" means one model exchange in Explore and a 48-turn stretch in Solve and in today's code. Fix: Explore says "exchange".
3. Solve reserves the word "verified" for Lean; the other two show "verified quote" everywhere. Fix: quotes are "matched". (Lean is now cut, and the owner decided that an exact symbolic check by code is what "verified" means: section 0.1.)
4. A dashed line means "used in the proof" in Investigate and "not checked" in Explore. Fix: dashed always means "not checked by code".
5. Mode is detected three different ways. Fix: the latest host-written mode marker wins.
6. Task kind is `check` in Explore's hand-off and `check_derivation` in Solve. Fix: Solve's names.

**Two months, four lanes** (three Rust lanes, one UI lane; section 6):

| End of week | What works |
|---|---|
| 2 | Profiles, paper key, quote core, graph component and evaluation harness exist. Skeleton scanner passes its fixtures. Solve badge function passes its property tests. |
| 4 | **Investigate: skeleton of an ingested TeX paper on screen with zero model calls.** Solve setup card. PDF text core measured on the PDF fixtures. |
| 6 | **Investigate: paste an arXiv id, structure in under 10 s, skip plan, symbols.** **Open a paper from your own library folder (a Kandinsky vault or a plain folder), TeX or PDF, with no network.** Solve: a recorded step with a claim gets "Verified" from the check process in tests. |
| 8 | **Investigate: Orient (main results, authors' new / restated labels). A PDF-only paper gets a skeleton and page-linked quotes.** Solve: checks, gates and finish pass the evaluation set headless. One hand-off packet. |

Honest limits: a Solve run a user can watch (S12, S13) and "open any node" in Investigate (V8) land in
weeks 9 to 11, not in month 2, unless a fifth worker is added. The scope revision puts the library
sources (F13) and the PDF path (F14a, V17) inside the two months; the price is that the shared
literature services (F11) and coverage (F12) move to weeks 9 to 11, and Explore's first screen from
about week 9 to about week 11 (planning estimate). The write-up (F15) and corrections (F16) come
after week 8, because they need the modes' records to exist.

**Decisions needed from the owner** (section 8): the scope decisions of 2026-11-25 are recorded in
8.1; on the same day the owner accepted every recommended answer of 8.2 and 8.3 in one message
(tool profiles, one global library, the two-month order above, where "New research" goes, and the
questions that the added packages raise).

### 0.1 Scope revision of 2026-11-25 (owner decisions)

| Decision | What it means in this plan | Where |
|---|---|---|
| **Add: bring your own papers, with sync** | New package F13. The app reads paper libraries the user already keeps (first a Kandinsky vault and a plain folder, then a BibTeX file, then a Zotero database) and keeps its own library index in step with them. It never writes into them unless asked | 2.11 |
| **Add, essential: old and PDF-only papers** | New package F14 (PDF text with page links, scans, first-page metadata) and Investigate's PDF skeleton V17 moved into the two months. A PDF-only paper is a normal case in every mode, not a fallback | 2.12 |
| **Add: a write-up output** | New package F15. A cited summary from Explore, a clean TeX derivation from Solve, reading notes from Investigate. Assembled by code from the records; how each statement was checked always travels with it | 2.13 |
| **Add: learning from user corrections** | New package F16. Corrections the user makes are kept across sessions and used again: by code where a rule exists, and as short notes shown to the model where it does not. They never change a badge, a tier or a verdict | 2.14 |
| **Skip: numerics in Solve ahead of Lean** | Nothing added. The numeric checks already in the plan (`identity_numeric`, S18 SciPy) stay as they were | - |
| **Cut: Lean and physlib** | No Lean tier, no `formal_check`, no engine download, no formal-library search in any new mode. Packages S16 cut; S22 loses its Lean part. The top tier in Solve is Verified, an exact symbolic check by code (next row). Existing Lean and physlib code stays untouched for earlier Research sessions (their tool list is frozen as `research/1`); removing that code is a separate decision (8.3) | solve.md; 2.2; 2.10 |
| **The word "verified"** | Owner, 2026-11-25: "things can still be verified through SymPy, SciPy and other Python computations". This overrules the editor's first reading (the word shown nowhere). **"Verified"** is the badge of a Solve step whose every claim passed an exact symbolic check by code; **"Checked numerically"** is the badge when the agreement is numeric (sampled points, quadrature, any SciPy result): evidence, not proof. A step is never shown above the weakest step it rests on. Quotes stay "matched"; a model review stays "Model agreed"; Investigate and Explore do not use the word. The split into two words is the editor's; it was accepted with question 11 of 8.3 | solve.md 4.3, 4.4; 2.10; 3.2 row 20; 7.3; 8.1 |
| **Defer: Cadabra2 and FORM** | Package S22 (engine manager, `engine_run`, profile `solve/2`) is deferred as a whole. No tool is reserved for them in `solve/1` | solve.md |
| **Defer: V14, Investigate's own prior-work check** | Investigate does not compare a result with earlier literature itself. The right half-badge is a button, "Search wider in Explore", which hands the result to Explore (F7) and later shows Explore's answer by reading the child session. The tool `record_prior_work_check` leaves the profile `investigation/2` (three new tools, not four) and is listed under `may_gain`. Evaluation set P4 is deferred; two of its cases become P-to-I chains in set C | investigate.md; 2.2; 2.9; 4.2 |
| **Defer: version tracking (evaluation set P2)** | A session stays on the arXiv version it opened. No "new version" banner, no carrying of notes and labels to another version, no builder upgrade inside an existing session. Tags stay permanent and content-based, and the version is still recorded, so this can be added later without changing stored data. Package V10 keeps only notes | investigate.md |

---

## 1. Facts re-checked for this review

| Fact | Where | Effect on the designs |
|---|---|---|
| `note_evidence` and the reader path require an open Investigation request marker (`find_active_marker`) | `research-tools/src/investigation_evidence.rs` 123-155 | Explore is right not to reuse it. Solve 7.1 lists it as "reused unchanged" for Cited steps: that cannot work in a Solve session. F4 fixes it. |
| A session is Investigation only if its latest marker is `investigation.started`; a session whose marker is `investigation.handoff` is Research | `desktop-server/src/investigation.rs` 23-34, 264 | Mode is "latest marker", not "first event". Explore's "first event `idea.started`" rule breaks for hand-off children, which begin with copied `source.ingested` events (`export/src/fork.rs` 75-80). |
| Two different grant stores exist: a per-session grant keyed by (session, effect) in the UI approver, and a Router grant keyed by caller, tool, def_hash, policy version and worker epoch for `WorkerSession` tools | `desktop-server/src/session_lifecycle.rs` 30-40; `tools/src/router.rs` 170-196 | Explore's "one click covers the session" (network reads) and Solve's "one grant per caller" (code execution) are both true. One approval card design must show both kinds. |
| The drift check tolerates 7 named `ReadWorkspace` tool ids and has two hand-written special cases | `agent/src/resume.rs` 191-240 | Confirmed. All three designs want to edit this function. |
| Reaching `max_turns` aborts the session | `agent/src/runner.rs` 409-415 | Confirmed. Investigate and Solve both need "pause", in two different ways today. |
| The menu has two buttons, "New research" and "New investigation" | `desktop-server/assets/index.html` 37-41 | Section 5. |
| Both `math.js` and `math_katex.js` exist; `canvas_dag.js` is 688 lines, `investigation_graph.js` 280 | `desktop-server/assets/` | The two graph modules are separate code today. |

---

## 2. Shared foundation

Rule for all of it: **storage stays per mode, the plumbing is shared.** Each mode keeps its own ledger
events because its integrity rules differ. What is shared is identity, fetching, quote checking, search,
the library, resume, the graph picture and the hand-off.

### 2.1 Packages

Sizes: S up to 2 days, M up to 1 week, L 1 to 2 weeks.

| Id | Title | Content | Files (new unless noted) | Size | Depends on | Absorbs |
|---|---|---|---|---|---|---|
| F0 | Guard rails | Golden test pinning (id, def_hash, order) of every tool registered today; CI check failing on any touched file over 600 lines; move `build_desktop_router` and `build_lean_router` out of `turn_runner.rs` (586) | `desktop-server/src/routers.rs`, `research-tools/tests/tool_def_golden.rs`, `workbench-checks.yml` | S | - | X0; rule 6.20; rule R20 |
| F1 | Tool profiles | 2.2 below | `research-tools/src/tool_profile.rs`, `agent/src/resume_profile.rs`; edits `resume.rs`, `autonomous.rs`, `routers.rs`, `mcp-server/src/main.rs`, `cli-route/src/core_tools.rs` | M | F0 | S1; the resume edit in V6; Explore 8.4; Investigate decision 17 |
| F2 | Paper key | 2.3 below | `research-tools/src/paper_key.rs` | S | - | `idea_keys.rs` from X1; `family` in V2 |
| F3 | Network gate and paper fetch | 2.4 below | `research-tools/src/net_rate.rs`, `paper_fetch.rs`; behaviour-only edits in `arxiv.rs`, `literature.rs`, `http_client.rs` | M | F2 | `arxiv_fetch.rs` from V4; `idea_rate.rs` from X2; `idea_fetch.rs` from X9 |
| F4 | Quote core | 2.5 below | `research-tools/src/quote_core.rs`, `lines_sent.rs` | M | - | rule 6.17; rules C1, C2; rule R17; the Cited path of S8 |
| F5 | Mode shell | One function `session_mode(events)`; New menu; rail tags; mode to profile to router; shrink `routes_session.rs` (616) | `desktop-server/src/session_mode.rs`, `assets/new_menu.js`; edits `routes_session.rs`, `session_rail.js`, `index.html` | S | F1 | mode parts of X3, S4, S5 |
| F6 | Graph view and shared UI parts | 2.6 below | `assets/graph_layout.js`, `graph_view.js`, `ui_badge.js`, `run_bar.js`, `quote_chip.js` | M | - | layout extraction in V13 and X12; `canvas_dag.js` slimming in S13 |
| F7 | Hand-off packet | Section 4 | `export/src/handoff.rs`, `desktop-server/src/handoff.rs` | S | F4, F5 | V16; hand-off halves of X13 and S21 |
| F8 | Evaluation harness | Section 7 | `crates/agent/tests/eval_support/`, `tests/fixtures/eval/`, `docs/design/eval-2026-11/` | S | - | S0; harness of X14; the Investigate acceptance set |
| F9 | Run budget and run state | One function counting model calls of a run from ledger events; one run-state vocabulary (3.3); one run bar | `research-tools/src/run_budget.rs`; uses `run_bar.js` | S | - | budget parts of X3, V6 (6.15), S11 (G4) |
| F10 | Library | 2.7 below | `research-tools/src/library.rs`, `library_schema.rs`; `desktop-server/src/routes_library.rs` | M | F2 | X5 |
| F11 | Literature services | 2.8 below | `research-tools/src/lit_providers.rs`, `lit_abstracts.rs` | L | F2, F3 | X2 |
| F12 | Coverage, verdict and wording | 2.9 below. Built for Explore; the Investigate side waits with V14 (deferred) | `research-tools/src/search_coverage.rs`, `prior_work.rs`, `prior_work_wording.rs` | M | F11 | coverage and wording parts of X7; check side of `status_badge.rs` (V7, V14) |
| F13 | Paper sources and sync (bring your own papers) | 2.11 below. a: source model, sync, Kandinsky-vault and folder adapters. b: BibTeX and Zotero adapters. c: sending papers back (opt-in) | `research-tools/src/paper_sources.rs`, `source_sync.rs`, `source_kandinsky.rs`, `source_folder.rs`, `source_bibtex.rs`, `source_zotero.rs`, `source_dedupe.rs`; `desktop-server/src/routes_sources.rs`; `assets/library_sources.js` | a: M; b: S each; c: S | F2, F10; F14a for PDFs with no identifiers | "Zotero link" add-on of Explore 10; "attach PDF" as the only way in for a paywalled paper |
| F14 | PDF and old papers | 2.12 below. a: PDF text core (extractor chosen by measurement, page map, scan detection, first-page metadata, background job). b: own OCR for pages with no text. c: page images to a model for formulas | `research-tools/src/pdf_pages.rs`, `pdf_firstpage.rs`, `pdf_match.rs`, `pdf_ocr.rs`; edits `pdf.rs`, `ingest.rs` (565) | a: M; b: M; c: S | a: none; b and c: a | PDF handling inside V4 and X9; the "heavy PDF parsing" add-on as the only answer for scans |
| F15 | Write-up | 2.13 below. Core (document model, TeX, Markdown and BibTeX writers) plus one small adapter per mode | `export/src/writeup.rs`, `writeup_tex.rs`, `writeup_md.rs`, `writeup_bib.rs`, `writeup_solve.rs`, `writeup_idea.rs`, `writeup_paper.rs`; `export/src/latex.rs` (771, over: split first) | core M; adapters S each | F4, F10; Solve adapter S9 and S11; Explore adapter X11 and F12; Investigate adapter V8 | the TeX output of Solve stage I; the Markdown part of X17; the Markdown part of V15 |
| F16 | Corrections and lessons | 2.14 below. a: corrections reused by code. b: lessons shown to a model, approved by the user | `research-tools/src/lessons.rs`, `lessons_schema.rs`, `lessons_select.rs`; `desktop-server/src/routes_lessons.rs`; `assets/lessons.js` | a: M; b: M | a: F10; b: F9 and at least one mode's recorders | Explore decision 11 (marks change rankings) |

Cut or deferred by the scope revision (section 0.1), so not in the build order: S16 (formal tier,
cut), S22 (engine manager, deferred), V14 (prior work in Investigate, deferred), the version half of
V10 (deferred). V17 (PDF skeleton) moves up.

After this, X1 is only terms, queries and ranking; V4 is only the open route, the job with progress and
the entry box; S0, S1, X0, X2 and X5 have no work of their own left (their rows stay in the three
documents as one-line pointers, so the numbering is unchanged).

### 2.2 Tool profiles (F1)

One mechanism for "which tools does this session have", used at start, at resume, in the desktop
router, in the MCP server and in `core.exe`.

| Profile | Tool list | Used by |
|---|---|---|
| `research/1` | Exactly today's `all_research_tool_defs()`, frozen by the golden test | Sessions with no profile field and no mode marker |
| `investigation/1` | Same list; today's 7 tolerated additions and the 2 special cases, moved into data | Investigation sessions with no profile field |
| `investigation/2` | `research/1` plus `read_map_node`, `record_map_reading`, `record_context_cards`. `record_prior_work_check` is not in the list while V14 is deferred; its id is named under `may_gain`, so it can be added later without a new profile | New Investigate sessions; an `investigation/1` session gains these three (all `ReadWorkspace`) on its next resume |
| `explore/1` | `idea_search`, `idea_fetch_papers`, `search_paper_passages`, `read_paper_lines` | Explore conversation turns |
| `solve/1` | As in solve.md 5.1 | Solve |

Each profile is data: `{name, defs, may_gain: [id], earlier_defs: {id: [ToolDef]}, policy}`.
`session.start` and `session.resume` record `tool_profile`. Resume builds the expected definitions from
the recorded profile and stays fail-closed.

Consequences, each resolving a conflict between documents:

- No Lean or physlib tool is offered to a model in Explore or Solve: `explore/1` and `solve/1` list none (`lean_check`, `search_formal_library` and the Lean worker are left out). `research/1` and `investigation/1` keep them, because those lists are frozen for old sessions. `investigation/2` is "`research/1` plus three" and so still carries the two definitions; dropping them from it is a small F1 choice that costs a second golden list (8.3, question 8).
- Investigate's new tools (three while V14 is deferred, four after) are **not** added to `register_research_tools`. They live in `investigation/2`. So they cost no tokens in Explore or Solve (Investigate decision 17 is answered: yes), and Solve's "`research/1` fails on any edit" holds.
- Explore's "nothing is added to the shared list" holds, and its future schema changes use the profile's `earlier_defs` instead of a new special case in `resume.rs`.
- Solve's rule "a profile changes only at a host-written brief resume" stays Solve-only for now.
- Tests: golden file per profile; a recorded ledger fixture of each existing session type (research, investigation before pointers, investigation current, hand-off child) resumes with all new code present.

### 2.3 Paper key and node references (F2)

```
PaperKey   = "arxiv:<base id, no version>" | "doi:<normalised>" | "inspire:<recid>"
           | "s2:<paperId>" | "openalex:<W id>" | "file:<first 16 hex of sha256 of the raw file>"
PaperRef   = { key: PaperKey, version: Option<String>, ids: {arxiv, doi, inspire, s2} }
NodeRef    = { session, mode, id, paper: Option<PaperRef>, node_fp: Option<String> }
```

- First available identifier in that order (Explore's rule). `file:` is added for dropped files (Investigate's `sha:` case).
- Investigate's `family` field **is** a `PaperKey`. Tags are `"n-" + base32(sha256(PaperKey | kind class | key))[..10]`. This must be fixed before V2, because tags are permanent.
- Ids inside a session keep their per-mode form. No prefixes collide today:

| Mode | Ids | Scope |
|---|---|---|
| Investigate | `n-..` skeleton tag, `m-..` model node, `legacy:<seq>:<id>` | `n-` is stable across sessions and versions of one paper; the rest per session |
| Explore | `sc-N` part of the idea, `p-N` perspective, `c- d- k- q- w- r-` + N map nodes, `xe-<seq>` `xa-<seq>` evidence | per session |
| Solve | `e-N` step, `name@version` handle, check id | per session |

- A citation key from the user's own library (`maier2005quantum`) is not a `PaperKey`. It is kept beside the key in the library's `source_item` table (2.11) and used by the write-up (2.13), so `PaperKey` and the tags hashed from it do not change.
- A file from a user library with no DOI and no arXiv id gets its `file:` key when its bytes are first hashed (2.11). Until then it is listed without a key and cannot be cited.
- A cross-session pointer is always a `NodeRef`, never a bare id. Reserve the prefixes in one table in `paper_key.rs` with a test that no two modes share one.

### 2.4 Network gate and paper fetch (F3)

- `net_rate.rs`: one process-wide gate per host with an injectable clock. arXiv 1 per 3 s, INSPIRE-HEP 15 per 5 s, Semantic Scholar 1 per s, Crossref 1 per s. **Every** outgoing request passes it, including the existing `arxiv_search` and `literature_search` handlers (behaviour only, no schema change, so no def_hash change).
- `paper_fetch.rs`: `fetch(PaperRef, want: metadata | versions | source) -> job`. Order: library (a blob already fetched, or a file in one of the user's own libraries, which F13 registers in the same `fulltext` table), arXiv e-print (TeX), arXiv PDF. A DOI with no arXiv version and no library file returns `needs_file`. Because F13 goes through the library table, the F3 brief already in work needs no change. Result enters a session only through the existing `ingest_bytes`, which writes that session's own `source.ingested`.
- Session fetch counter: one function `fetched_sources(events)`; the limit is a parameter (Investigate 8 per session, Explore the preset's `max_papers`). Raising it is a user click recorded in the ledger.
- Rule for approvals, stated once for all modes: **a network request runs without an approval card only when the user's own click or keystroke named exactly that request** (an arXiv id typed in, a citation chip clicked, a query string shown in the form). Anything a model or the app composes goes through `Router::try_call` and the real approver. Investigate's `open` and `cited` routes and Explore's rule P4 are all instances of this.

### 2.5 Quote core (F4)

```
QuoteRef { source_id, path, start_line, end_line, file_sha256, quote_sha256, evidence_seq: Option<u64> }
verify_quote(session_events, source_id, path, start_line, end_line, quote) -> Result<QuoteRef + excerpt, Fault + true_text>
lines_sent(events, run_id) -> ranges              // from one event kind, below
```

- `verify_quote` wraps the existing `SourceIndex::verify_pointer` (same relocation radius, same fault text) and needs only a `source.ingested` receipt, no Investigation marker. Settles the open point in Explore 7 ("to be settled in X10") once.
- **One event for "text the app put in front of a model":** `context.supplied {run_id, caller, ranges: [{source_id, path, file_sha256, start_line, end_line}]}`. Investigate's `investigation.context_supplied` and Explore's "lines sent to this reader" are the same fact. Tool results that return lines (`read_paper_lines`, `search_paper_passages`, `read_map_node`, `fs_read`) are credited by the same function. Rule in all modes: a quote is accepted only if it verifies **and** its lines were sent to the caller that cites it.
- **One event for a stored quote outside Investigation maps:** `evidence.quote` (id `xe-<seq>`) and `evidence.abstract` (id `xa-<seq>`), instead of Explore's `idea.evidence` / `idea.abstract_evidence` and an unnamed third kind in Solve. Mode-specific fields (`role`, `subclaim_ids`, `gist`) stay optional in the payload. Investigate's new records keep embedding anchors (they are checked by the same function); the existing `investigation.evidence` kind is untouched.
- Solve's `source_ref` on a Cited step is a `QuoteRef` (field names `start_line`, `end_line`, not `first_line`, `last_line`) whose `evidence_seq` names an `evidence.quote` event in the same session.
- Field name everywhere: `start_line`, `end_line`. Quote length shown or exported: at most 300 characters.

### 2.6 Graph view and shared UI parts (F6)

One picture component, three adapters. The Rust view of each mode (`mapview`, `idea_view`, `solve_view`)
returns the same JSON shape:

```
GraphView {
  nodes: [{ id, kind, label, group,        // group = section lane / node kind / none
            parent,                        // for folding
            grade,                         // 2.10
            badges: [{text, icon, tone}], flags: [text], stale: bool, dim: bool }],
  edges: [{ src,                           // the thing relied on
            dst,                           // the thing that relies on it
            kind, role, basis: "code" | "evidence" | "model" }],
  legend: [...]
}
```

- **Edge direction is fixed:** `src` is the prerequisite, `dst` the dependent, drawn top to bottom. Investigate already stores edges this way. Solve's `depends_on` maps directly. Explore must state the direction per edge kind (for `uses`, `special_case_of`, `generalises`, `answers`) in its recorder schema; today it does not.
- **Line and border style is fixed:** solid = found by code or backed by a matched full-text quote; dashed = model-inferred, abstract-only or unchecked. Investigate's "statement uses" versus "proof uses" is shown by line weight and a toggle, not by dashes.
- **Badges** are always text plus icon, never colour alone (Solve's rule, applied to all).
- `graph_layout.js` (pure: layering, lanes, folding above 150 nodes) is extracted from `investigation_graph.js` and `canvas_dag.js`; `graph_view.js` draws, pans, selects, filters. `canvas_dag.js` drops under 600 lines as a result.
- Also shared, small: `quote_chip.js` (quote, paper, lines, opens source), `ui_badge.js`, `run_bar.js` (state, elapsed, "7 of at most 24 model calls", Stop, Continue), the node-card shell (instant block from code on top, model block below, each labelled).

### 2.7 Library (F10)

Explore's X5 design, with neutral names and two additions. `<data_dir>/library/library.db`, an index over
ledger blobs, rebuildable, never evidence.

| Change to X5 | Reason |
|---|---|
| Module `library.rs`, routes `/api/library/..`, not `idea_library` | All modes use it |
| `seen.role` gains `investigated`, `cited_lookup`, `solve_source` | "Recent papers" in Investigate; sources in Solve |
| New rebuildable table `skeleton(key, version, builder, blob_sha256)` | A cited paper's skeleton (V11) is built once, not once per session |
| `fulltext` is the fetch cache of F3 | Investigate's "cache by id and version" is this table |
| `fulltext.origin` gains `user_library`; such a row names a file path, size, mtime and (once read) sha256 instead of a blob | Papers the user already owns are used in place (2.11) |
| New tables `source` and `source_item` (2.11), rebuildable by scanning the sources again | Bring your own papers |
| New table `paper_text` (key, version, file sha256, extractor, text blob) and `paper_fts` extended to full text where it exists | Search inside the user's own papers; PDF text is extracted once per file, not once per session (2.12) |

A minimal `fulltext` table ships with F3; F10 adds the rest. The tables of F16 (2.14) live in a
separate file, because they hold user data that cannot be rebuilt.

### 2.8 Literature services (F11)

Explore's X2 provider layer under neutral names: one query runner per service with abstracts and citation
relations, raw response stored as a blob, `literature.fetched` gains optional `qid` and `run_id`.
The existing `arxiv_search` and `literature_search` tools keep their definitions byte for byte.
Investigate's prior-work task may keep calling those two tools; its caps are counted from the task's own
call ids (rule 6.7) through F9.

### 2.9 Coverage, verdict and wording (F12)

"Has this been done" (Explore) and "is this result earlier" (Investigate) are one question about
different objects. One enum, one gate, one wording module.

| Shared `PriorWork` status | Investigate today | Explore today | Needs |
|---|---|---|---|
| `judged_same` | `earlier_statement` | `found_same` | Matched full-text quote on both sides; shown "Model judges: ..." until the user confirms |
| `judged_partial` | `partly_earlier` | `found_partial` | Same |
| `judged_other_name` | (missing) | `found_other_name` | Same, and the other name occurs in the quote |
| `abstract_only` | `found_not_read` list | `possibly_related_abstract_only` | Verbatim abstract quote; never more than "possibly related" |
| `none_in_scope` | `none_found` | `not_found_in_scope` | Coverage gate passed; sentence lists what was searched |
| `incomplete` | `inconclusive` | `search_incomplete`, `no_close_abstract` | Gate failed; sentence lists the failing conditions |

- Since the scope revision only Explore asks this question. The Investigate column of the table is kept for the day V14 is built; until then Investigate hands the result to Explore and shows Explore's status by reading the child session (4.2).
- The user's own library (2.11) is listed in the coverage record as a searched place ("your library: 412 papers, 380 with full text"). It never counts towards the gate condition "at least 2 services answered", because a private collection is not a literature index.
- No value means "new" or "novel" in either mode. Property test over the wording module.
- Deferred with V14: Investigate adopts Explore's `Coverage` record and gate (its own rule "refused when the task made no search or read" is weaker); the "references only" depth has its own gate (every reference is read, or listed as not available); Investigate adds the same one-click confirmation Explore has for a "same" judgement.
- `independence` of a second model is computed by one host function with Solve's three values (`different_provider`, `different_model`, `same_model`). Solve's S14 is its only user for now, so the function is built with S14 (it was listed under F12).

### 2.10 One evidence scale for display

Not a shared storage field; a shared mapping so the same look means the same thing in every mode.

| Grade | Investigate | Explore | Solve | Look |
|---|---|---|---|---|
| By code | Skeleton node or edge | Coverage, verdict text | Verified (exact symbolic check), Checked numerically (numeric agreement) | Solid, plain; the badge text tells the two apart |
| Matched quote, full text | From this paper / Outside this paper | `xe` evidence | Cited | Solid, quote chip; for a PDF the chip names the page |
| Matched quote, scanned page (OCR text) | Same, from a scan | `xe` evidence from a scan | Cited, flag "OCR source" | Solid, quote chip tagged "OCR", which opens the page image (2.12) |
| Matched quote, abstract | - | `xa` evidence | - | Dashed, "abstract only" |
| Model statement | Gist, role, model edge, model background; a formula read by a model from a page image | Summary, difference, direction text | Model agreed, Assumed by model | Dashed or tagged "(model)" |
| User statement | Note, pin | Idea text, marks, confirmation | Given, Checked by you | Plain, "you" |

There is no grade above "By code". The formal tier (Lean) was cut.

### 2.11 Paper sources and sync: bring your own papers (F13)

**What it is for.** The user already keeps papers somewhere: in a Kandinsky vault, in folders of
PDFs, in a `.bib` file, in Zotero. The app reads those places, lists the papers in its own library
(F10), keeps the list in step when files change, and uses the files in place. This is also the main
way a paywalled or old paper gets into the app at all.

**What Kandinsky does** (read in its repository at `353368b`: `docs/guides/ZOTERO.md`,
`docs/guides/PAPERS-PDF-SOURCES.md`, `plugins/beta/catalog-papers/`, `crates/kandinsky-core/src/
{external_roots, pdf_text, pdf_columns}.rs`):

| Kandinsky | Taken into re:Search | How |
|---|---|---|
| A reference is one folder `refs/papers/<citekey>/` with a note `<citekey>.md` whose properties hold the metadata (Zotero field names); `refs/refs.bib` is rewritten from them after every change | Yes | The Kandinsky-vault adapter reads exactly these. The files stay the truth; the app's library is an index that can be rebuilt |
| PDFs stay where they are: in the entry's folder, or in shared folders outside the vault (`external_pdf_roots`), never copied, no symlinks or hard links, no path that leaves a listed folder | Yes | Files are indexed in place. Bytes are copied only when a paper is opened in a session (below) |
| Scanning a folder: PDFs only, identifiers and first-page metadata, collections from subfolders, a moved file recognised by SHA-256, a dry-run report first, rate-limited lookups | Yes | The folder adapter and the sync loop |
| Duplicates by five signals: normalised DOI, arXiv id without version, PDF SHA-256, ISBN, fuzzy title plus first author plus year | Yes, without merging | The first four merge index rows. The fuzzy one only shows "possible duplicate" |
| Citation keys are kept (pinned Better BibTeX keys first) | Yes | Stored per source item; the write-up cites with them |
| The exact arXiv version is kept (`arXiv:XXXX.YYYYYvN`); the unpacked arXiv TeX source may sit in `arxiv_source_dir/<id>/` | Yes | Version goes into `PaperRef.version`. When the TeX source is there, the app reads TeX, not the PDF |
| Zotero import straight from `zotero.sqlite`, opened read-only, from a snapshot when Zotero holds a lock | Later (F13b) | Zotero users are already covered by a BibTeX export or by Kandinsky's own import |
| Answering the Zotero browser extension; a drop inbox; merging duplicates; reading notes; PDF annotations | No | These write into the library. They stay Kandinsky's job |
| PDF text read in Rust, cached per file, pages separated by a form feed; first-page metadata by shape; column geometry; OCR with Tesseract compiled into a helper | Yes, as approach | 2.12 |

**Model.**

```
PaperSource { id, kind: "kandinsky_vault" | "folder" | "bibtex" | "zotero",
              root, settings, added, last_sync, state }
SourceItem  { source_id, external_id,            // the citation key, or the path under the root
              key: Option<PaperKey>, ids: {doi, arxiv, isbn}, version,
              title, authors, year, item_kind, collections, tags,
              files: [{ path, role: "pdf" | "tex" | "note", size, mtime, sha256: Option }],
              state: "present" | "changed" | "missing" | "possible_duplicate" }
```

One trait, `scan(source) -> items`, read-only, one adapter per kind:

| Adapter | Reads | Needs to read the PDF for metadata |
|---|---|---|
| `kandinsky_vault` | The references folder (default `refs/papers`): each `<key>/<key>.md`, or the older `info.yaml`; the `pdf` property; the shared PDF folders; the arXiv TeX folder. Where the vault's settings file names these folders is to be confirmed against Kandinsky when the adapter is written; the user can also set them by hand | No |
| `folder` | Any folder: PDFs, TeX bundles, arXiv archives. Identifiers found in the first pages, then title, authors and year by shape, then the file name. Collections from subfolders | Yes (F14a) |
| `bibtex` | One `.bib` file and the `file` field of each entry. Works with an automatic export from Zotero, with JabRef and with Kandinsky's own `refs.bib` | No |
| `zotero` | `zotero.sqlite`, read-only | No |

**Sync.** One direction by default: the source is read, never written.

1. When: at app start, on "Sync now", and every few minutes while the app is open. By polling, with no file watcher (Kandinsky does the same).
2. Listing compares path, size and modification time only. No file content is read, so files that a cloud drive has not downloaded are not pulled down.
3. For new and changed items the metadata is read. A file is hashed when it is first opened, or in a background pass the user turns on ("Index the full text of my library").
4. The same SHA-256 at a new path is the same item (moved). A file that is gone is marked "missing" and stays listed while any session refers to it.
5. The first sync of a source shows a report first (items, with PDF, with TeX, duplicates, unreadable). Nothing is written to the index before the user accepts it.
6. Removing a source removes its rows from the index and touches no file and no session.

**Using a paper.** Opening a library paper in any mode runs the existing `ingest_bytes` on the file:
the session gets its own `source.ingested` receipt and its own pinned copy. So every quote is checked
against the session's copy, and editing, moving or deleting the file later cannot change what an old
session shows. The library is still never evidence.

**What the modes gain.** Investigate: "Open from library", with no network. Explore: the library is
a searched place (title and abstract at once, full text where indexed); a paper the user owns can
be read even when it is paywalled, so "unavailable: attach PDF" becomes rarer; library papers can
be seeds. Solve: "Cite a paper" picks from it. Write-up: citations use the user's own keys.

**Sending papers back (F13c, opt-in, later).** "Export BibTeX" writes a file the user picks. "Send
to Kandinsky" goes through Kandinsky's own entry point (`kandinsky capture-paper <id or url>` or
the `kandinsky://capture-paper` link), so the vault keeps one writer. The app never writes vault
files itself.

**Network and privacy.** Scanning and syncing are local. Looking up metadata for files that carry no
identifier (Crossref by title) is a request the app composes, so it needs the approval card (2.4),
states how many lookups and to which host, passes the gate of F3, and is off by default.

**Porting (decided 2026-11-25: re:Search is AGPL-3.0, so Kandinsky's code is ported directly where
useful).** Kandinsky is AGPL-3.0-or-later; re:Search is AGPL-3.0-only on `main` (PR 5). Code under
"version 3 or later" may be used under version 3, so the direction Kandinsky to re:Search is
allowed. The reverse is not automatic: code first written here is "3.0 only" unless the owner
licenses it otherwise.

| Kandinsky file (at `353368b`) | Lines | Port into | Note |
|---|---|---|---|
| `plugins/beta/catalog-papers/engine/src/firstpage.rs` | 257 | `pdf_firstpage.rs` (F14a) | Title, authors, year from the first page by shape. Nearly self-contained |
| `.../pdf_meta.rs` | 441 | `pdf_firstpage.rs` or beside it (F14a) | DOI, arXiv id, ISBN in the first pages. No Kandinsky helper used |
| `.../entry.rs` | 393 | `source_kandinsky.rs` (F13a) | Reads a reference's note and the older `info.yaml`. Only the reading half is ported |
| `.../external.rs` and `crates/kandinsky-core/src/external_roots.rs` | 499, 410 | `source_kandinsky.rs`, `source_folder.rs` (F13a) | Resolving a PDF inside a listed folder with no way out of it. The annotation sidecar parts are left out |
| `.../scan_roots.rs` | 1337 | `source_folder.rs`, `source_sync.rs` (F13a) | The folder scan, moved-file detection by hash, the dry-run report. Split on the way in |
| `.../inbox_dedupe.rs` | 912 | `source_dedupe.rs` (F13a) | The five duplicate signals. The inbox and the merge that rewrites notes are left out |
| `.../zotero_import.rs` | 982 | `source_zotero.rs` (F13b) | Read-only access to `zotero.sqlite`, citation keys, attachment paths. The parts that write notes and annotations are left out |
| `crates/kandinsky-core/src/pdf_text.rs` | - | `pdf_pages.rs` (F14a) | Per-file text cache with page breaks, if F14a chooses `pdf_oxide` |
| `crates/kandinsky-core/src/pdf_columns.rs` | - | `pdf_pages.rs` (F14a) | Column geometry, only if F14a finds that it is needed |

Rules for a port:

1. Every ported file starts with the header already used for the maths rendering, with the commit: `// Ported in part from Kandinsky (github.com/antimajorana/kandinsky, 353368b), the same author's project.`
2. Kandinsky's `docs/THIRD-PARTY.md` says Kandinsky is based on Atlas by Alneb. Whether any file in the table carries Atlas code was not checked. Each port checks its file's history first and keeps any notice it finds; a file with another author's notice is ported with that notice or re-coded.
3. Not copied: vendored libraries, anything under `plugins/obsidian/`, the plugin SDK glue. The engine uses two Kandinsky helper crates (`kandinsky-text` for note properties and safe paths, `kandinsky-plugin` for its host calls); the port replaces those calls with re:Search's own sandbox helpers and a small front-matter reader. Kandinsky is not added as a dependency.
4. Three of the files are over this repository's 600-line limit. They are split on the way in; the limit is not waived.
5. New dependencies a port brings are named in its package and need the owner's yes as any other: `pdf_oxide` (only if F14a picks it), `serde_yaml` or another YAML reader for note properties. `rusqlite`, `tar`, `flate2` and `sha2` are already in the workspace.
6. What is ported is the reading side. Everything in Kandinsky that writes into the library stays there (the table of "What Kandinsky does" above).
7. A port keeps Kandinsky's tests for the part it takes.

Effect on size: F13a and F13b are mostly adaptation, not new design, which makes the one-week
estimate for F13a more believable. It is still an estimate.

**Rules and tests.** Fixtures: a small self-made vault, a folder and a `.bib` file with self-made
PDFs. Syncing twice changes nothing; a moved file keeps its item; an edited note updates the
metadata; a deleted file becomes "missing"; a path that leaves the root, and a symlink or junction
out of it, are refused (the existing sandbox helpers); after a sync the source folder is
byte-identical; removing the source or deleting the index breaks no quote check in any session.

### 2.12 PDF and old papers (F14)

**Why it is essential.** Papers before the early 1990s, most journal-only papers and most books have
no TeX source. Before this revision the plan treated them as a slow fallback with a weak outline,
left scans to an optional add-on, and built the PDF outline late (V17).

**Today in the code:** `research-tools/src/pdf.rs` extracts text with `pdf-extract`, limited to
16 MiB, 400 pages and 8 MiB of text, and reports `ok` or `empty_text_layer`; extraction runs inside
ingestion; one logged run took 34 to 138 s per paper.

**F14a, PDF text core.**

- One function from bytes to page-broken text with the extractor's name and version. Two extractors are measured on the PDF fixtures and on evaluation set P6 before one is chosen: `pdf-extract` (today) and `pdf_oxide` (what Kandinsky uses for the same job, with `pdftotext` as a fallback when installed). Measured: time, failures, reading order on two-column pages. No speed figure is promised before that.
- The reading file is the extracted text with a form feed between pages, pinned in the source receipt together with the extractor id. Anchors stay `{path, start_line, end_line}`, so the quote core (F4) works unchanged, and `page_of(line)` is a lookup. How today's ingestion stores the extracted text is to be confirmed at the start of F14a. A better extractor never rewrites the text of an existing session.
- Every quote chip from a PDF source names the page and opens it.
- Each page gets a basis: `text_layer`, `scan_text_layer` (a full-page image with text behind it, which is how publishers ship most scanned old papers) or `none`. When the detection is unsure the page counts as a scan. The detection rule is fixed on the fixtures.
- Matching a quote against PDF text ignores what PDF extraction adds: ligatures, soft hyphens, a hyphen at a line end. The stored text is never changed; only the comparison is normalised (`pdf_match.rs`), and the chip shows the text as stored.
- Two-column pages: if the chosen extractor mixes the columns, the page is split by column geometry before the text is written (Kandinsky's `pdf_columns.rs` shows the method on character boxes).
- First-page metadata by shape: title, authors, year, DOI, arXiv id, ISBN. Ported from Kandinsky's `firstpage.rs` and `pdf_meta.rs` (2.11, "Porting").
- The 16 MiB limit is too small for many scans. It is raised for files the user supplies, to a value set from measurement. A file over the page limit is opened by a page range the user picks.
- Extraction is a background job with page progress, and its result is kept once per file in the library (`paper_text`), not once per session.

**F14b, own OCR for pages with no text.** A local helper process inside the existing sandbox, with
no network, running Tesseract (Apache-2.0). Language models are downloaded once as a user action in
Settings, pinned by hash; English first. The OCR text becomes the reading file with basis `ocr` and
the engine and model hashes in the receipt. Not verified: that Tesseract builds into the Windows
installer, and which library renders pages to images (question 4 of 8.3).

**Honesty for scanned text** (both `scan_text_layer` and `ocr`):

1. A quote found there is shown as "matched in OCR text, p. 7". The chip opens the page image, so the reader sees the original.
2. Nothing from OCR text is drawn as mathematics. A statement with formulas is shown as text with the page crop beside it.
3. **F14c.** A model that accepts images may be given page images to read formulas. What it returns is "transcription (model), unconfirmed" with the crop beside it, the pattern Solve already uses for a derivation the user supplies. It is never a matched quote. The pages sent are recorded in `context.supplied` (a range may name pages of a file).
4. Solve: a Cited step whose quote comes from a scan carries the flag "OCR source". Explore: a `judged` status may rest on OCR full text, tagged so; the user's confirming click is needed as for any other (question 3 of 8.3).

**PDF skeleton (Investigate V17, moved into the two months).** From the page-broken text: section
headings by numbering pattern, theorem-like headings and "Proof." by pattern, equation numbers at
the margin, the reference list as bibliography entries with any DOI or arXiv id found, "[12]" as a
citation, "Theorem 2.1" and "Eq. (3.4)" as heuristic uses. Numbers are `printed`. Quality is
`weak` or `partial` and the screen says so. Tags are content-based, as for unlabelled TeX nodes.

**Not in this package:** Solve's "check a derivation" from a PDF (TeX only at first); Docling and
GROBID (still optional add-ons, no longer needed for the normal case).

**Tests and fixtures.** The self-written TeX fixtures are compiled once to PDF (text exactly known)
and the same pages are turned into images (a scan with exactly known text); both are committed, since
the content is our own. Tests: a sentence copied by hand from the page matches; a wrong page link
never occurs; a scan page is never reported as `text_layer`; the same bytes and extractor give the
same text.

### 2.13 Write-up (F15)

**What it is for.** Something the user can read, send or paste into a paper: prose with citations
and formulas. The existing exports (ZIP, notebook, `.rsmap`, JSON) are for machines and for
re-import; they stay.

**How it is made.** A pure function from ledger events to one document model, then a writer per
format (TeX with a `.bib` file, Markdown). Zero model calls.

```
Writeup { title, status_line, sections: [Section], bibliography: [BibEntry], check_table }
Block   = Prose { text, basis: "app" | "user" | "model" }
        | Formula { latex, from: handle }             // Solve: rendered by the check process
        | Quote { text (<= 300 chars), cite, page or lines, ocr: bool }
        | Statement { text, how_checked }
```

| Mode | Write-up | Content |
|---|---|---|
| Solve | Clean derivation in TeX | The steps the final answer rests on, in dependency order. Assumptions first (Given, Assumed by model). Each formula comes from its handle, rendered by the check process, so the formula written equals the formula checked (R5). Step prose is the solver's text, marked as model text in the check table. Cited steps become `\cite` with the quote in a footnote |
| Explore | Cited summary | Per part of the idea: the status sentence from the wording module (F12), the matched quotes with citations, "what differs (model)". Then the coverage paragraph ("where I looked"), written by the app. Then the directions |
| Investigate | Reading notes | Outline, main results (statements within the quote limit unless the paper's licence allows more), gists marked as model text, the user's notes |

**Rules.**

1. **How it was checked always travels.** Every write-up ends with a table "How each statement was checked" (step or claim, badge, what it rests on) and starts with a status line ("Done with open items: 2 steps unchecked"). Inline badge marks can be turned off; the table and the status line cannot. Explore's coverage paragraph cannot be removed either.
2. A Solve write-up is refused while the closure of the final answer has a Failed or Stale step (the rule of gate G7).
3. Citations are `PaperRef`s. The key is the user's own citation key when the paper is in one of their libraries (2.11), else one made by the app; the `.bib` file comes from the library.
4. The words "verified", "proved", "new" and "novel" follow the same rule as on screen; the property test of 7.3 covers the writers. In a Solve write-up "verified" appears only in the check table, on rows whose own tier is Verified, each with its assumptions; the text never calls the result as a whole verified unless the final step's effective tier is Verified.
5. **Optional "Polish the prose (model)"**: one exchange that may only rewrite prose blocks. Formulas, quotes, citations, status sentences and the check table are locked placeholders; the result is refused if one is missing or altered, and is checked by the existing honesty lint and Explore's phrase rule (V8). The output says "prose edited by a model".
6. A quote from a scan is marked "(OCR)".

Order of building: the core and the Solve adapter first (after S11; a derivation is what a physicist
most often wants to keep), then Explore's (after X11), then Investigate's.

### 2.14 Corrections and lessons (F16)

**What it is for.** When the user corrects the app, the same mistake should not come back in the
next session. Nothing here trains a model. It is a local store the user can read, edit and delete.

**Corrections already in the three designs:** pins and notes in Investigate; "wrong", "not
relevant", "key paper", "looks the same to me" and its rejection in Explore, and edits to the
sharpened idea and the queries; "Correct this step", acknowledgements and setup revisions in Solve.
F16 adds no new way to correct. It reads these events.

**F16a, corrections reused by code** (no model involved):

| Correction | Reused as |
|---|---|
| Paper marked "irrelevant" or "key paper" | Later rankings: demoted but still listed; a key paper becomes a seed when its terms match (Explore decision 11) |
| "Same under another name" confirmed or rejected | A confirmed pair of names is added to the alias list used for later queries; a rejected pair is not proposed as the same again |
| Conventions, symbol assumptions and dimensions set or corrected in a Solve setup | Offered as defaults the next time the same symbols appear |
| A skeleton node the reader fixed ("this is a theorem", "the proof ends here") | A reader override record keyed by the node's tag. The skeleton itself stays a pure function of the files |
| A line of OCR text the reader corrected | An overlay for that file's hash, shown as "corrected by you". The pinned text is unchanged |

**F16b, lessons shown to a model.**

- A lesson is a short text with a scope: `{id, text (<= 300 chars), scope: global | field | paper | mode, from: NodeRef, created, uses}`.
- A lesson is made only when the user says so. After a correction the app offers "Remember this?" with the user's own words filled in and editable. Nothing is learnt silently. A model-written summary of the correction is an option, never the default.
- Lessons are chosen for a run by code: scope match, then most recent, up to a fixed number and size. No model picks them.
- They are given to the model as quoted data under the heading "Notes from the user's earlier corrections". The exact text supplied is stored in the run's ledger (`lessons.supplied {run_id, lesson_ids, blob}`), so a resume or a replay sees the same text even if the store changed. The screen shows "used 2 of your notes" with the two notes.
- A page "What I have learnt" lists every lesson with its origin and its use count, with edit and delete.

**Rule: a lesson never decides anything.** Badges, tiers, verdicts, coverage gates and quote
matching are computed by code from records, as before. A lesson changes only what a model is told
and, in F16a, a ranking or a default the user can see. Property test: for the same events, with and
without lessons, every tier, verdict and gate result is identical.

Storage: `<data_dir>/library/lessons.db`. It is user data: not rebuildable, included in backups,
local only, and exported only when the user ticks it.

Order of building: F16a with the library (F10) and Explore's notes; F16b after at least one mode
records corrections on screen (Solve's "Correct this step" in S13).

---

## 3. Contradictions and recommended resolutions

### 3.1 Mechanisms

| # | Conflict | Resolution |
|---|---|---|
| 1 | Investigate adds 4 tools to the shared list and to the "may be gained" list. Explore adds nothing to the shared list and keeps its own router. Solve freezes the shared list as `research/1` and rebuilds resume around profiles. | F1. Profiles are the only mechanism. |
| 2 | Three golden def_hash tests (6.18, S1 and S2 of Explore, R1 of Solve) | One golden file per profile, in F0 and F1. |
| 3 | Mode detection: "first event is `idea.started`" (Explore); "latest marker" (Investigation code); `solve.started` "at creation or after a fork" (Solve). Explore says mode is three-valued; there are four values. | `session_mode` = the latest of `investigation.started`, `idea.started`, `solve.started`; none means `research`. Only the host may write a marker (test enumerates what each tool can append, as in Solve R2). |
| 4 | Two arXiv limiters and two fetchers | F3. |
| 5 | Paper identity: `family` versus `PaperKey` | F2. |
| 6 | Solve reuses `investigation_evidence.rs` unchanged for Cited steps; that code refuses to run without an Investigation request marker | F4. Solve cites `evidence.quote` records. |
| 7 | "Lines the model received": `investigation.context_supplied` versus Explore's per-reader rule | F4, one event `context.supplied`. |
| 8 | Hand-off: Investigate extends `investigation.handoff`; Explore writes `idea.handoff`; Solve reads `origin` in `solve.setup` | F7, one `handoff.packet` (section 4). Old `investigation.handoff` sessions stay readable and stay Research. |
| 9 | Task kind: Explore `solve_setup.kind` = `calculate, prove, check`; Solve `task_kind` = `calculate, prove, check_derivation`. Explore `target` is a string; Solve `target` is a record. | Solve's names and record. Explore's `expected_form` fills `target.form` and `target.description`. |
| 10 | Pause at a cap: Investigate lets the runner abort and repairs the status in the finish hook; Solve adds `SessionOutcome::Paused` | Keep both for now. S12 makes `Paused` mode-neutral in the runner; Investigate switches to it afterwards and drops the repair. |
| 11 | Model-call counting is written three times (6.15, P3, G4) | F9. |
| 12 | Policy: Explore uses `research_policy()` unchanged; Solve adds `solve_policy()` with its own version | Fine, but the policy belongs to the profile (F1) so resume compares the right version. |
| 13 | Approvals: "allow for this session" (Explore) versus "one grant per caller, tool and worker epoch" (Solve) | Both are true (section 1). One approval card component that lists each grant it will record, used by Solve's setup card, Explore's plan card and Investigate's "search wider". No new scope. Whether the approver can prompt outside a running model turn is unverified in all three documents: settle it in F5. |
| 14 | Abstracts as evidence: dropped in Investigate, allowed for two node kinds in Explore | Consistent after F12: abstract-only never supports a "judged" status in either mode. Investigate shows such hits as `abstract_only` with the quote instead of a bare "found, not read" list. |
| 15 | Explore module names (`idea_keys`, `idea_rate`, `idea_providers`, `idea_fetch`, `idea_library`) for code all modes use | Neutral names as in 2.1. The `idea_` prefix stays for Explore-only code. |
| 16 | Three packages edit `routes_session.rs` (616), three edit `resume.rs` (579), two edit `turn_runner.rs` (586), two edit `canvas_dag.js` or `investigation_graph.js` | Done once each, in F5, F1, F0 and F6, by one lane, before the mode packages that need them. |

### 3.2 Words

| # | Word | Meanings found | Resolution |
|---|---|---|---|
| 17 | Leg | Explore: one model call plus up to 2 repairs. Solve and today's code (`run_turn_leg`): a stretch of up to 48 turns. | "Leg" keeps the code's meaning. Explore's unit is an **exchange**: `agent/src/exchange.rs`, event `idea.exchange`. |
| 18 | Brief | Explore: the sharpened idea (`idea.brief`). Solve: the deterministic text a resumed leg starts from. | Explore's data name stays; its screen says "Sharpened idea". Solve's is the **resume brief** (`resume_brief.rs`). |
| 19 | Check | Investigate: the prior-work comparison (`investigation.check`, "Check: not run"). Solve: a computed test (`solve.check`). | "Check" belongs to Solve. Investigate: event `investigation.prior_work`, badge "Prior work: not run". The tool id `record_prior_work_check` may stay. |
| 20 | Verified | Solve: only with Lean. Investigate and Explore: "verified quote", "quotes not verified on this machine". | The word belongs to Solve: a step whose every claim passed an exact symbolic check by code is "Verified"; numeric agreement is "Checked numerically" (section 0.1, solve.md 4.4). It is used for nothing else: not for a quote, a model review, a paper or a run. Quotes are "matched" ("quote matched at lines 212-219", "4 quotes could not be matched"). Internal names may keep "verify". |
| 21 | Recorder | Investigate: a registry tool. Explore: an app function with no tool definition. | One definition: a recorder is the validator that writes the ledger. It may be offered as a registry tool or as the single output of an exchange. |
| 22 | Model call, request, turn | Caps are in "requests" (Investigate), "model calls" (Explore), "model calls" and "turns" (Solve) | "Model call" = one request to the provider, in every cap and on every screen. "Turn" is internal. |
| 23 | Run states | Investigate: done, partial, failed, stopped. Explore: requested, finished, failed, stopped_at_cap, cancelled, interrupted. Solve: paused (9 reasons), finished, finished_open_items. | 3.3. |
| 24 | Model-judgement labels | "Model judges: ...", "(model)", "as read by the model", "Model agreed", "suggestion (model)" | Two patterns only: a verdict starts "Model judges:" or "Model agreed"; any other model text carries the suffix "(model)". |
| 25 | Dashed | Investigate whole map: proof uses. Investigate cards and Explore: not checked or abstract only. | 2.6. |
| 26 | Definition | A node kind in Investigate (found by code in the paper) and in Explore (needs a matched quote with role `defines`) | Same word is fine; the graph legend is per mode and the grade (2.10) shows the difference. |
| 27 | Package sizes | Investigate sizes are in lines (S up to 300 lines); Explore and Solve in days | This document uses days for all. Investigate's L packages (V1, V3, V6, V14) are 1 to 2 weeks. |

### 3.3 One run-state vocabulary (F9)

| State on screen | Meaning | Investigate task | Explore stage | Solve run |
|---|---|---|---|---|
| Running | Work in progress, with elapsed time and model calls used | requested | requested | leg running |
| Waiting for you | An approval or an answer is needed | approval | plan approval | `approval_needed` |
| Paused: (reason) + Continue | Stopped at a limit; one click resumes; nothing lost | `partial`, `stopped` | `stopped_at_cap`, `cancelled` | every `solve.paused` reason except the next two |
| Interrupted + Retry | The app or the provider went away mid-run; never re-run unasked | (add) | interrupted | `interrupted`, `transport_error` |
| Failed + Retry | An error with its reason | `failed` | failed | - |
| Done | Finished with nothing open | `done` | finished | `finished` |
| Done with open items | Finished; a list says what is open | - | - | `finished_open_items` |

"Partial" describes a result, not a state: a paused Investigate task shows "Paused: 3 results annotated".

---

## 4. Hand-overs between modes

### 4.1 One packet (F7)

A hand-over creates a new session whose first events are, in order: copies of the needed
`source.ingested` events (existing mechanism in `export/src/fork.rs`), one `handoff.packet`, then the
target mode's marker. The source session gets a `handoff.sent` event (existing `investigation.linked`
pattern).

```
handoff.packet v1 {
  from: NodeRef,                       // session, mode, id, paper, node_fp
  target_mode: "investigate" | "explore" | "solve",
  title,
  statement: { text, quotes: [QuoteRef] },          // verbatim text and where it came from
  context: [{ role: "definition" | "assumption" | "uses" | "known_result" | "convention",
              from: NodeRef, text, quotes: [QuoteRef] }],
  papers: [PaperRef],                  // sources and seeds
  conventions: text,                   // from the paper's conventions catalog or the Solve setup
  model_text: [{ label, text }],       // gists, summaries, direction prose: always shown as "(model)"
  user_notes: [text],
  ask: { kind, fields }                // target-specific, see 4.2
}
```

Rules, enforced in the receiving route:

1. **Nothing arrives as checked.** The child re-ingests the same bytes and runs every `QuoteRef` through `verify_quote` against its own receipt. A quote that fails is dropped and counted on screen.
2. **Badges and tiers never travel.** A Solve step created from a packet is Given (user text) or Cited (quote matched in the child). An Investigate label or an Explore verdict travels only as `model_text`.
3. The packet is data. Paper text and model text in it are wrapped as quoted material for any model.
4. The user sees and can edit the target's setup screen before any model call. A hand-over costs zero model calls.
5. Result flows back by reading, not by writing: the origin node shows "Opened in Solve: Done with open items" by reading the child's latest `solve.done` or `idea.verdict`.

### 4.2 The hand-overs

| From | Action | To | What travels in `ask` and `context` | First screen in the target |
|---|---|---|---|---|
| Investigate node (theorem, equation, proof step) | "Derive in Solve" | Solve | `kind`: `prove` for theorem-like nodes, `calculate` for an equation, `check_derivation` for a proof node. `context`: the statement closure (definitions, assumptions, equations it uses) with their spans; symbols from `SymbolDef`; the paper's conventions | Setup card: statement filled, each closure item listed as a Cited step once its quote matches, symbols table filled, origin chip back to the node |
| Investigate node | "Has this been extended?" | Explore | Idea text prefilled from the statement (editable). `papers`: this paper as `origin` and its resolved bibliography as seeds | Explore form with terms, library matches and first queries. **The origin paper is excluded from "found" verdicts** and its own references are marked "cited by the origin" |
| Investigate result, right half-badge | "Search wider in Explore" (this is Investigate's only prior-work path while V14 is deferred) | Explore | Same as above; the idea text is the result's statement and the question is "is this earlier", so the sharpened parts are the result's claims | Same. Back in Investigate the half-badge reads "Opened in Explore: (Explore's status for the part)" by reading the child's latest `idea.verdict` |
| Explore direction (Concrete or Loose) | "Open in Solve" | Solve | `solve_setup` with Solve's field names (3.1 row 9). `known[]` items become `context` with their `xe` quotes; `check.reproduces` becomes a suggested done criterion; `risks` and `stop_if` become user-visible notes | Setup card; known results listed as Cited once matched; "Check first" shown above the done checklist |
| Explore paper row or map node | "Investigate this paper" | Investigate | `papers`: one `PaperRef`. `context`: the quotes Explore already matched in it; the parts of the idea as a focus | Skeleton from the library blob in seconds; each earlier quote appears as a note on the innermost node whose span contains it |
| Solve step | "Cite a paper" (attach a source to an assumption) | within Solve, library | A `PaperRef` picked from the library or an arXiv id; the user or the model points at lines; `verify_quote`; the step becomes Cited | Cited chip; click opens the paper in Investigate at the node whose span contains the lines (a `NodeRef` computed by span containment, no model) |
| Solve run | finished | origin session | Nothing is written into the origin; it reads the child's status | "Solved" chip on the Investigate node or the Explore direction |
| Solve step | "Has this been computed before?" | Explore | Statement rendered from the step's handles | Later; not in the two-month plan |

### 4.3 What each design must change for this

- investigate.md 4.4: `.handoff` "gains optional fields" becomes "old kind unchanged; new hand-overs write `handoff.packet`". V16 becomes three buttons ("Derive in Solve", "Has this been extended?", "Search wider in Explore"), the filling of the packet from a node and the receiving side, on top of F7. The third button moved here from V14, which is deferred.
- explore.md: `idea.handoff` becomes `handoff.sent`; `Direction.solve_setup` uses Solve's names; add the "exclude origin paper" rule to the verdict function with a test.
- solve.md 8.4: the prefill reads a packet; `solve.setup.origin` is a `NodeRef`. The prefill from a packet is built in F7 on top of S4 (milestone M3 needs it in week 8); S21 keeps only "Open in Solve" for earlier Research sessions, which is a fork plus `solve.started` (5.3), not a packet.

---

## 5. How the app presents three modes

### 5.1 Entry screen

One home screen with three cards. Each card has one input and shows deterministic content before any
model call.

| Card | Input | What appears with no model call | Verb on the button |
|---|---|---|---|
| **Investigate** a paper | arXiv id, DOI, link, a dropped file, or a paper picked from the library | Metadata, then the skeleton (from TeX, or from PDF text with the quality line of 2.12) | Open |
| **Explore** an idea | Free text, TeX allowed | Term chips, library matches, the exact first queries | Explore |
| **Solve** a problem | Statement, TeX allowed | Kind guess, symbols table, default done checklist | Run |

- Paste detection: text that is only an arXiv id, DOI or arXiv link focuses the Investigate card whichever box it lands in. Nothing else is guessed; the user picks the mode.
- Below the cards: recent sessions with a mode tag, and "Library" (the papers the app has seen plus the user's own sources, 2.11, with their sync state).
- A card for a mode that is not usable yet is hidden behind a setting, not shown greyed out.
- With no model connected, each card still says what works: Investigate skeleton; Explore search and library; Solve setup, and re-running checks of earlier sessions. The library, its sync and every write-up also work with no model.

### 5.2 Names

| On screen | Internal (events, files, tool ids) | Note |
|---|---|---|
| Investigate | `investigation.*`, `investigation_*`, `map_*` | The old word "Investigation" stays in code; the menu says "Investigate". The word "branch" disappears from the new view |
| Explore | `idea.*`, `idea_*` | `explore.*` stays reserved for `delegate_explore`, whose label becomes "Try strategies in parallel" |
| Solve | `solve.*`, `solve_*`, `proof.*` | Steps remain `proof.entry` |
| Research (earlier) | no marker | Tag for sessions with no mode marker |
| Shared | `handoff.*`, `evidence.*`, `context.supplied`, `literature.fetched`, `source.ingested` | New neutral prefixes come only from this document |

### 5.3 What happens to today's sessions

| Today | After | Detail |
|---|---|---|
| Investigation session | Opens in Investigate, upgraded in place | Skeleton appended on first open if the source files still match their hashes; old map under "Earlier map"; nothing recorded changes; profile `investigation/1` gains the three new tools on resume (the fourth with V14) |
| Research session | Stays as it is, tagged "Research (earlier)" | Resumable under `research/1` with its recorded prompt; gains "Open in Solve" (fork plus `solve.started`) once S21 exists |
| Research session created by an old Investigation hand-off | Research (earlier) | Unchanged |
| "New investigation" button | Investigate card | From F5 on |
| "New research" button | Stays, under "More", until Solve can run a problem end to end (S13) and Explore can map an idea (X13); then hidden behind a setting | Until Explore exists, an idea typed into Research keeps working as today, with the one-line hint to Explore once it ships |

A session started after F1 records a profile and will not resume in an older build. Say so in the release notes once.

---

## 6. One build order for the first two months

Revised on 2026-11-25 for the scope decisions (section 0.1). Done so far: F0, F2, F8. In work as
agent briefs: F3, F4 with F9, V1, S2.

Assumptions: four lanes (A Investigate Rust, B Solve Rust and Python, C foundation Rust, D UI in
plain JS), one integrator who merges. On the 14.8 GB reference laptop only one heavy cargo build
runs at a time, so lanes that build locally take turns or build elsewhere; lane D needs no cargo build.
Lane C owns every edit to `resume.rs`, `routes_session.rs`, `turn_runner.rs` and `http_srv.rs`, so those
files are never edited by two lanes in the same week.

Items inside one numbered step can run in parallel. A step starts when the listed dependencies are merged.

1. **Week 1.** `F0` (C) | `F2` (C) | `F8` harness plus the Solve problem set, former S0 (B) | `V1` starts (A) | `S2` (B) | `F6` (D).
2. **Week 2.** `F1` (C, needs F0) | `F4` with `F9` (C) | `V1` finishes (A) | `S3` then `S6` (B) | `F5` UI part (D).
3. **Week 3.** `F5` Rust part (C, needs F1) | `F3` (C, needs F2) | `V2` (A, needs V1, F2) | `S4` (B, needs S2, F5) | `V3` UI against fixture JSON (D, needs F6).
4. **Week 4.** `V3` Rust part and joining (A, needs V2) | `S7` (B, needs F1, S2, S6) | `F14a` PDF text core (C) | `S5` (D, needs S4).
   **Milestone M1: skeleton of an ingested TeX paper on screen, zero model calls; build time measured on the fixtures. The two PDF extractors measured on the PDF fixtures and one chosen.**
5. **Week 5.** `V4` (A, needs V3, F3) | `S8` (B, needs S7, F4) | `F10` library (C, needs F2) | `V5` (D with A for the Rust half, needs V2, V3).
6. **Week 6.** `V6` starts (A, needs V3, F1, F4, F9) | `S9` starts (B, needs S3, S8) | `F13a` sources and sync, Kandinsky-vault and folder adapters (C, needs F10, F14a) | library list and sources panel, the UI half of F13a (D).
   **Milestone M2: paste an arXiv id, structure in under 10 s on the evaluation papers; skip plan and symbols; a Solve step with a claim reaches Verified in tests; a paper from the user's own library folder opens with no network, and a second sync changes nothing.**
7. **Week 7.** `V6` finishes (A) | `S9` finishes (B) | `V17` PDF skeleton (C, in Investigate's skeleton module, needs V2, F14a) | `V13` whole map without novelty toggles (D, needs V5, F6).
8. **Week 8.** `V7` Orient (A, needs V6) | `S11` gates and finish (B, needs S3, S9, F9) | `F7` hand-off packet (C, needs F4, F5, S4) | `V7` badges UI and the page link on the quote chip (D).
   **Milestone M3: Orient shows main results and the authors' labels with quotes on the evaluation papers; a PDF-only paper of set P6 gets a skeleton with its quality line, and quotes from it match and open the right page; the Solve evaluation set passes headless (planted errors end Failed, never Verified or Checked numerically); one hand-off packet opens a Solve setup card from an Investigate node.**

Next, in order, after week 8: `S12` and `S13` (a Solve run on screen), `V8` (open any node), `F11`
(literature services), `F15` core with the Solve adapter (a derivation as TeX), `F14b` (own OCR),
`F12`, `X1`, `X3` and `X4` (Explore session and first screen, model-free), `F16a`, `S10`, `V9`,
`V10` (notes only), `V11`, `S14`, `X6` to `X8`, `S15`, `X9` to `X13`, the Explore adapter of `F15`,
`F16b`, `F13b` (BibTeX, Zotero), `F14c`, `F13c`.

Not in the order: `S16` (cut), `S22`, `V14` and the version half of `V10` (deferred).

What the revision costs inside the two months (planning estimate): lane C's weeks 4 to 7 used to
hold the literature services (F11), the library and coverage (F12). They now hold the PDF text
core, the library, the sources and the PDF skeleton. F11 and F12 follow in weeks 9 to 11, so
Explore's first screen moves from about week 9 to about week 11. Nothing is taken from the
Investigate or Solve lanes. X1 (pure code, small) no longer fits in week 7.

What can move:

| If | Then |
|---|---|
| Only three lanes | Drop lane D as a separate lane; UI packages follow their Rust halves; every milestone slips about two weeks. Keep the order |
| Only two lanes | Lanes A and C only. Solve starts after M2. F1 still comes first |
| A fifth worker | `F11` and `F12` in weeks 5 to 8, which brings Explore's first screen (`X3`, `X4`) back to about week 9 |
| V1 fixtures show the own TeX scanner failing | Stop and decide on texlab or tree-sitter before V2; everything after V2 waits |
| S6 shows the restricted parser rejecting most real expressions | Stop S7 to S9 and widen the allowlist from the logged rejections first; a badge that almost never appears is not worth the rest |
| F14a shows that neither extractor gives a usable reading order on two-column pages | Do the column split before V17; V17 waits, the rest does not |
| F13a does not fit in one week (it is the tightest new item) | Ship the Kandinsky-vault adapter at M2 and the folder adapter with V17 |

Gate before each milestone: the evaluation set (section 7) is run and its sheet is filed. No timing or
quality figure is quoted to users before that.

---

## 7. Evaluation set

Revised on 2026-11-25: sets P2 and P4 are deferred; sets P6, B, W and K are new.

### 7.1 Layout (F8)

Two tiers.

| Tier | Where | Content | Runs |
|---|---|---|---|
| Fixed | In the repository: `tests/fixtures/eval/` | Self-written TeX fixtures, the same compiled to PDF and turned into page images (2.12); a self-made paper library (2.11); recorded service responses served by a loopback mock; a scripted model transport; ledger fixtures of old sessions. No third-party paper text | In CI, on every change |
| Live | Outside the repository: a manifest of arXiv ids, papers named by DOI, ideas and problems with hand-made answer keys; papers are fetched or taken from the owner's library, never stored in the repository | Real papers, real services, the user's real models and real library | By hand before each milestone; results in `docs/design/eval-2026-11/` as one sheet per run |

One manifest format for all modes: `{id, mode, input, answer_key, notes}`. One sheet format:
date, build, models, model calls, wall time, the measures below.

### 7.2 Cases

| Set | Size | Content | Answer key made by hand, once |
|---|---|---|---|
| P. Papers, structure | 10 arXiv ids, at least 4 physics papers with no theorem environments, 1 over 80 pages, 1 multi-file with custom theorem macros | Investigate skeleton from TeX | List of theorem-like statements and sections; 30 sampled "uses" edges marked right or wrong |
| P2. Versions | deferred with version tracking (section 0.1) | - | - |
| P3. Authors' labels | 5 of the papers in P, 6 to 10 results each | Orient | Each result marked new, restated, background or not stated, with the sentence that says so |
| P4. Restated results | deferred with V14. Two of its cases (a known restatement whose earlier source is on arXiv) move to set C as chains from a paper to an idea | - | - |
| P5. Cited statements | 10 citations with a locator ("[x, Theorem 2.1]"), at least 3 of them into a paper that exists only as PDF | Cited lookup | The right statement in the cited paper |
| **P6. PDF-only and old papers** | 8 papers: 2 journal PDFs with a text layer and no arXiv version; 2 two-column; 2 scans from before 1990 that carry a publisher's text layer; 2 scans with no text, one of them heavy with formulas | PDF text core, PDF skeleton, quote matching, page links (F14, V17); the Explore reader on a PDF | Sections and theorem-like statements; 20 sentences per paper copied by hand, each with its page; for the four scans, 30 sampled lines typed by hand |
| **B. Own library** | Fixed tier: a self-made vault of 30 entries, a folder and a `.bib` file, with planted cases (a moved file, an edited note, a deleted file, a duplicate by DOI, a duplicate by file hash, one paper as preprint and as journal version, a path that leaves the root). Live tier: the owner's real library | Sources and sync (F13) | The expected item list and the expected state of each planted case. For the real library only counts are recorded |
| I. Ideas | 10 ideas: 3 done, 2 done under another name, 3 done partly, 2 open as far as known. For the first three groups the key papers are given. For 2 of the ideas a key paper is available only from the library (paywalled) | Explore | Key papers per part of the idea; expected status per part |
| Q. Problems | 10 calculations with known closed forms (including the effective-potential example), 3 derivations with one planted error each, 2 prose-heavy proofs | Solve | Final answer; expected tier per step; the planted step |
| **W. Write-ups** | The write-up of 3 runs of Q, 2 runs of I and 1 paper of P | F15 | Nothing by hand: the pass rules of 7.3 |
| **K. Corrections** | 6 scripted pairs of sessions: a correction in the first, its effect in the second (a confirmed other name is used in queries; a rejected pair is not proposed again; an irrelevant paper is demoted; a convention is offered as default; a lesson is supplied and shown; a deleted lesson is not supplied). And 3 problems of Q, each run with and without one lesson that names its typical error | F16 | The expected effect per pair |
| C. Chains | 3 from P to Q: a node of an evaluation paper whose equation is one of the Q problems. 4 from P to I: 2 results with a known follow-up paper, and 2 restated results whose earlier source is known (from the former P4; this is "Search wider in Explore"). 2 from I to Q: an idea whose first calculation is a Q problem | Hand-overs | The answer of the target case |
| R. Old sessions | 4 ledgers: research, investigation before pointer citations, investigation current, hand-off child | Resume and migration | Must resume; must show the same content |

The owner chooses the real papers and ideas; a reviewer who is not the builder makes the answer keys.
Set I is frozen as recorded responses so "open as far as known" cannot change under the test.

### 7.3 What is measured

| Measure | Applies to | Target or pass rule |
|---|---|---|
| Time from input to first deterministic content | All modes | Investigate TeX under 10 s; Explore candidates under 6 s; Solve setup under 1 s. Investigate from a PDF with a text layer: recorded; a target is set after the first F14a sheet |
| Model calls and tokens per run, against the cap | All | Never above the cap; typical value recorded |
| Theorem-like statement recall; section recall | P, P6 | Recorded; below 0.9 on TeX papers with theorem environments blocks M1. For PDF the figure is recorded and shown beside the quality line; no blocking value before the first sheet |
| "Uses" edge precision, statement and proof roles separately | P | Recorded |
| Authors' label agreement with the key; share of labels whose quote contains a cue | P3 | Recorded |
| Known earlier source found; "none in scope" shown for a result that has a known earlier source | I, and the P-to-I chains of C | The second number is the false-negative rate that decides whether the feature is announced |
| "Judged same" where the key says different | I, C | Recorded; every such case reviewed |
| Wrong statement shown as located | P5 | Must be 0 (candidates list is acceptable) |
| Known-paper recall of the search alone, before any model call | I | Recorded per service set (keyless; with keys; with and without the library) |
| Quotes accepted per model call; quotes rejected and why | Investigate tasks, Explore readers | Recorded |
| A sentence copied by hand from the page matches in the stored text | P6 | Recorded per page basis (text layer, scan with text layer, own OCR) |
| A quote chip opens the wrong page | P6 | Must be 0 |
| A scanned page reported as a plain text layer | P6 and the PDF fixtures | Must be 0 |
| Character errors per 100 characters on the sampled lines of scans; lines in the wrong reading order on two-column pages | P6 | Recorded |
| Sync: a second sync changes nothing; every planted case ends in its expected state; the source folder is byte-identical afterwards; no quote check breaks when a source is removed | B | Must pass; in CI |
| Planted error ends Failed | Q | Must be 100 percent; ending Verified or Checked numerically is a release blocker |
| Share of steps Verified, Checked numerically, Model agreed, Unchecked | Q | Recorded; decides whether SymPy's reach is enough. With Lean cut, a prose proof cannot rise above Model agreed; the two proofs of Q show how much that costs |
| Final answer equals the key | Q | Recorded |
| Write-up: the TeX compiles; every formula equals the check process's rendering of its handle; every citation resolves to a bibliography entry; the check table lists every step the answer rests on with its badge; the status line equals the run's status | W | Must pass |
| With and without lessons, every tier, verdict and gate result is identical for the same events | K | Must pass; in CI |
| The corrected error returns, with the lesson and without it | K | Recorded |
| Quotes surviving re-matching after a hand-over of unchanged bytes | C | Must be 100 percent |
| Target case solved correctly when started from a hand-over | C | Recorded |
| Old ledgers resume; views equal before and after | R | Must pass; in CI |
| Peak memory and build time on the reference laptop | All | Recorded at each milestone |
| "Verified" appears only tied to a Solve step whose own tier is Verified, and as a step badge only when the effective tier is Verified; no Investigate or Explore view, export or write-up shows the word; "new", "novel" and "proved" stay inside their allowed places | All | Property tests in CI |
| A step whose only passing identity check is numeric is never shown as Verified; a Verified equation that disagrees at the numeric guard points ends Failed | Q | Must be 100 percent; property test on generated ledgers plus two planted cases in set Q (one identity true only on a sampled domain, one wrong simplification across a branch cut) |

---

## 8. Decisions for the owner

### 8.1 Decided on 2026-11-25

| Topic | Decision | Recorded in |
|---|---|---|
| Bring your own papers, with sync | Add | 0.1, 2.11 (F13) |
| Old and PDF-only papers | Add; essential | 0.1, 2.12 (F14, V17 moved up) |
| Write-up output | Add | 0.1, 2.13 (F15) |
| Learning from corrections | Add | 0.1, 2.14 (F16) |
| Numerics in Solve ahead of Lean | Skip | 0.1 |
| Lean and physlib | Cut | 0.1; solve.md |
| The word "verified" (was question 3) | Decided by the owner: computations in Python can verify. "Verified" is a Solve step with an exact symbolic check; "Checked numerically" is numeric agreement; quotes are "matched". (The editor's first reading, "shown nowhere", was overruled.) | 0.1, 3.2 row 20, 7.3, solve.md 4.4 |
| Cadabra2 and FORM | Defer | 0.1; solve.md S22 |
| V14, Investigate's own prior-work check | Defer; "Search wider in Explore" is the path | 0.1, 4.2; investigate.md |
| Version tracking (set P2) | Defer | 0.1; investigate.md V10 |
| Licence | re:Search is AGPL-3.0. Already on `main` (PR 5). Kandinsky's code may be ported directly where useful | 2.11 "Porting" |

### 8.2 From the first version: recommended answers accepted

**Accepted on 2026-11-25.** The owner approved the open items in one message ("Do everything all at
once, I give my approval"). Every answer in the "Recommended" column below is therefore the decision,
and the documents may be built on it. Any single row can still be reversed by the owner.

| # | Question | Recommended |
|---|---|---|
| 1 | Adopt tool profiles (F1) as the single way a session's tools are fixed, before any mode adds a tool? | Yes. It settles Investigate decision 17 and replaces S1 and X0. |
| 2 | Two-month priority | Revised: Investigate through Orient, including PDF-only papers; the Solve engine headless; the library with the user's own sources. The shared literature service and Explore's first screen follow in weeks 9 to 11. A fifth worker brings Explore back to about week 9 (section 6). |
| 3 | "Verified" | Decided, see 8.1. |
| 4 | Is a dashed line or border always "not checked by code"? | Yes; statement versus proof use is shown by weight and a toggle. |
| 5 | One library for all modes, with the fetch cache, cited-paper skeletons, PDF text and the user's own sources in it? | Yes, global, rebuildable; user marks and lessons backed up. |
| 6 | Entry screen | Three cards with one input each and paste detection for paper ids; no mode guessing beyond that. |
| 7 | "New research" | Under "More" until Solve (S13) and Explore (X13) both work; then behind a setting. Earlier sessions stay resumable. |
| 8 | Hand-overs re-match every quote and never carry a badge | Yes. |
| 9 | One-click confirmation of Investigate's "earlier statement" judgement | No longer asked: deferred with V14. Explore keeps the click. |
| 10 | Shared event kinds `handoff.packet`, `evidence.quote`, `evidence.abstract`, `context.supplied` instead of one per mode? | Yes; none of the per-mode kinds they replace has shipped. |
| 11 | Rename Explore's "leg" to "exchange" and Investigate's "check" to "prior work"? | Yes, now, while it costs nothing. |
| 12 | Who makes the answer keys of the evaluation set, and which real papers and ideas go in? | The owner picks, now including the eight PDF-only and old papers of set P6; someone other than the builder writes the keys before M1. |

### 8.3 Raised by the revision: recommended answers accepted

**Accepted on 2026-11-25.** The owner approved the open items in one message ("Do everything all at
once, I give my approval"). Every answer in the "Recommended" column below is therefore the decision,
and the documents may be built on it. Two rows still need something from the owner that an approval cannot supply: the papers and ideas of the evaluation set (8.2, question 12) and the measurement that question 4 waits for. Any single row can still be reversed by the owner.

| # | Question | Recommended |
|---|---|---|
| 1 | The revised two-month order puts PDF and the library first and moves Explore's first screen about two weeks later. Accept? | Yes. Both were asked for, and Explore reads better once paywalled papers from the library can be read. |
| 2 | Porting Kandinsky's code and the licence of this repository | Decided on 2026-11-25, see 8.1: AGPL-3.0, port directly. Left open inside it: Kandinsky is "3.0 or later" and this repository "3.0 only", so code flows from Kandinsky to here but not back unless the owner says so. Should this repository say "or later" too? Recommended: yes, if code written here should ever go back into Kandinsky. |
| 3 | May a "judged" status in Explore and a Cited step in Solve rest on a quote matched in OCR text? | Yes, always tagged "OCR", with the page image one click away. Formulas from OCR are never drawn as mathematics. |
| 4 | Own OCR (F14b): Tesseract in a local helper, language models downloaded once from Settings, and a page renderer added as a dependency? | Yes, but decide after F14a has measured how many pages of set P6 really have no text. If publishers' text layers cover the owner's old papers, F14b can wait. |
| 5 | Sync direction | The app only reads the user's libraries. Sending a paper back goes through Kandinsky's own entry point or a BibTeX file the user picks; the app never writes vault files. |
| 6 | Which sources first | A Kandinsky vault and a plain folder (F13a), then a BibTeX file, then Zotero's database (F13b). |
| 7 | Lessons from corrections | Kept only when the user says "Remember this", in the user's own words; a model-written summary is an option. Default scope: this mode and this field. They never change a badge or a verdict. |
| 8 | Remove the existing Lean and physlib code (the `lean_check` and `search_formal_library` tools, the Lean worker, the formal-library panel)? Earlier Research sessions resume with a frozen tool list that names them. | Not now. No new mode offers them, and their screen entry points are hidden for new sessions when F5 is built. Removal waits until earlier Research sessions are retired. Also: leave them out of `investigation/2` if the tool list check in F1 confirms they are in the research list. |
| 9 | With Lean cut, a proof in prose can reach at most "Model agreed" or "Checked by you". Keep "prove" as a Solve task kind? | Keep it, and say so on the setup card: "a proof in prose is reviewed by a model; only calculations are computed". Its default done checklist asks for a review by a different model. |
| 10 | May a write-up be produced without the table "How each statement was checked"? | No. Inline marks can be turned off; the table and the status line stay. |
| 11 | The owner said computations in SymPy, SciPy and other Python code can verify. SciPy is numeric. Is a numeric agreement "Verified", or a weaker word? | A weaker word: "Checked numerically", one tier below "Verified" (exact symbolic check). Agreement at 12 points does not exclude a difference elsewhere, and one word for both would hide which steps are exact. The alternative, if the owner prefers the word everywhere, is "Verified numerically" with the same tier order; it changes two strings and no logic. |

---

## 9. Edits to make in the three documents

Applied on 2026-11-25. `investigate.md`, `explore.md` and `solve.md` were edited to agree with this
document and now refer to the shared packages by their F ids; each carries a "Reconciled" line under
its title. A second pass on the same day checked the three documents against each other.

- Sections 0, 1 and 3 above describe the three documents as they were before these edits. They are
  kept as the record of why the shared packages exist.
- Removed packages keep their rows as one-line pointers so that numbering stays stable: X0 (now F0),
  X2 (now F3 and F11), X5 (now F10), S0 (now inside F8), S1 (now F1).
- Kept on purpose: stored field and state names that are not shown on screen (`from` and `to` on
  stored edges, `brief_blob`, `stopped`, `stopped_at_cap`, `finished_open_items`), the tool id
  `record_prior_work_check`, and internal uses of "verify" and "turn".
- Still open for the owner or the foundation lane: which package builds the shared approval card
  (3.1 row 13); whether `incomplete` needs a reason code; whether F7 stays size S now that it also
  holds the Solve prefill from a packet and the smallest "Derive in Solve" filler.

**Scope revision, same day.** After the owner's decisions (section 0.1) this document gained 0.1,
2.11 to 2.14, a new build order, a new evaluation set and a split decisions table, and the three
documents were edited again; each carries a "Scope revision" line under its title.

- Cut and deferred packages keep their rows, marked "cut" or "deferred", so numbering stays stable:
  S16 (cut), S22, V14 and the version half of V10 (deferred).
- Passages of the three documents that describe the review of the earlier proposals (their
  sections 0 and appendix) still mention Lean, Cadabra2 and FORM as history. They were not rewritten.
- Settled by the revision: the `independence` function is built with S14, its only user for now.
- Licence: the passages that said "GPL-3.0 only, AGPL assumed" were replaced after the owner's
  decision; `main` was merged into this branch so that `Cargo.toml` and `LICENSE` here say AGPL-3.0.
- For the briefs already in work: the S2 brief was changed twice (no Lean-checked tier; then the
  two computed tiers, Checked numerically and Verified, with the rule for where "verified" may
  appear). The F3 brief needs no change (2.4). The V1 brief still
  lists a two-version fixture; it is harmless and unused until version tracking returns.
