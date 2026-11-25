# re:Search - three modes, one foundation: cross-mode review

Written 2026-11-25. Reviews `investigate.md`, `explore.md` and `solve.md` in this folder against each other
and against `reSearch-modes-gaps-and-prior-art-2026-11-25.md`. Code facts were re-checked read-only on
`origin/gemini/work/fixed` with `git show` and `git grep`. Nothing was checked out, built or run. All
durations are planning estimates.

Package ids: `V..` Investigate, `X..` Explore, `S..` Solve (as in the three documents), `F..` shared
foundation (new in this document).

---

## 0. One-page summary

**The three designs are sound one by one and agree on the principles** (code decides every label; the
model only proposes records that code checks; deterministic content first; old sessions keep resuming).
**They disagree on the plumbing.** Each one built its own version of the same six things. If built as
written, the app would have three resume mechanisms, two arXiv rate limiters (which together break
arXiv's limit), two literature search stacks, three quote-checking entry points, three graph views and
three hand-off formats.

**What to do about it: build 13 small shared packages (F0 to F12) first or alongside, then the modes.**

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
3. Solve reserves the word "verified" for Lean; the other two show "verified quote" everywhere. Fix: quotes are "matched".
4. A dashed line means "used in the proof" in Investigate and "not checked" in Explore. Fix: dashed always means "not checked by code".
5. Mode is detected three different ways. Fix: the latest host-written mode marker wins.
6. Task kind is `check` in Explore's hand-off and `check_derivation` in Solve. Fix: Solve's names.

**Two months, four lanes** (three Rust lanes, one UI lane; section 6):

| End of week | What works |
|---|---|
| 2 | Profiles, paper key, quote core, graph component and evaluation harness exist. Skeleton scanner passes its fixtures. Solve badge function passes its property tests. |
| 4 | **Investigate: skeleton of an ingested TeX paper on screen with zero model calls.** Solve setup card. |
| 6 | **Investigate: paste an arXiv id, structure in under 10 s, skip plan, symbols, whole map.** Solve: a recorded step with a claim gets "Computed" from the check process in tests. Literature service and library exist. |
| 8 | **Investigate: Orient (main results, authors' new / restated labels).** Solve: checks, gates and finish pass the evaluation set headless. One hand-off packet. Explore: search service, coverage and wording done; its screen is the first work of month 3. |

Honest limit: a Solve run a user can watch (S12, S13) and "open any node" in Investigate (V8) land in
weeks 9 to 11, not in month 2, unless a fifth worker is added.

**Decisions needed from the owner before work starts** (section 8): adopt tool profiles; one global
library; "verified" reserved for Lean; the two-month priority above; where "New research" goes.

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
| F12 | Coverage, verdict and wording | 2.9 below | `research-tools/src/search_coverage.rs`, `prior_work.rs`, `prior_work_wording.rs` | M | F11 | coverage and wording parts of X7; check side of `status_badge.rs` (V7, V14) |

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
| `investigation/2` | `research/1` plus `read_map_node`, `record_map_reading`, `record_context_cards`, `record_prior_work_check` | New Investigate sessions; an `investigation/1` session gains these four (all `ReadWorkspace`) on its next resume |
| `explore/1` | `idea_search`, `idea_fetch_papers`, `search_paper_passages`, `read_paper_lines` | Explore conversation turns |
| `solve/1` | As in solve.md 5.1 | Solve |

Each profile is data: `{name, defs, may_gain: [id], earlier_defs: {id: [ToolDef]}, policy}`.
`session.start` and `session.resume` record `tool_profile`. Resume builds the expected definitions from
the recorded profile and stays fail-closed.

Consequences, each resolving a conflict between documents:

- Investigate's 4 tools are **not** added to `register_research_tools`. They live in `investigation/2`. So they cost no tokens in Explore or Solve (Investigate decision 17 is answered: yes), and Solve's "`research/1` fails on any edit" holds.
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

- A cross-session pointer is always a `NodeRef`, never a bare id. Reserve the prefixes in one table in `paper_key.rs` with a test that no two modes share one.

### 2.4 Network gate and paper fetch (F3)

- `net_rate.rs`: one process-wide gate per host with an injectable clock. arXiv 1 per 3 s, INSPIRE-HEP 15 per 5 s, Semantic Scholar 1 per s, Crossref 1 per s. **Every** outgoing request passes it, including the existing `arxiv_search` and `literature_search` handlers (behaviour only, no schema change, so no def_hash change).
- `paper_fetch.rs`: `fetch(PaperRef, want: metadata | versions | source) -> job`. Order: library blob, arXiv e-print (TeX), arXiv PDF. DOI with no arXiv version returns `needs_file`. Result enters a session only through the existing `ingest_bytes`, which writes that session's own `source.ingested`.
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

A minimal `fulltext` table ships with F3; F10 adds the rest.

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

- No value means "new" or "novel" in either mode. Property test over the wording module.
- Investigate adopts Explore's `Coverage` record and gate (its own rule "refused when the task made no search or read" is weaker). The "references only" depth has its own gate: every reference is read, or listed as not available.
- Investigate adds the same one-click confirmation Explore has for a "same" judgement. Cost: one note event.
- `independence` of a second model is computed by one host function with Solve's three values (`different_provider`, `different_model`, `same_model`); Investigate's `checker.same_as_reader` becomes that field.

### 2.10 One evidence scale for display

Not a shared storage field; a shared mapping so the same look means the same thing in every mode.

| Grade | Investigate | Explore | Solve | Look |
|---|---|---|---|---|
| By code | Skeleton node or edge | Coverage, verdict text | Computed, Lean-checked | Solid, plain |
| Matched quote, full text | From this paper / Outside this paper | `xe` evidence | Cited | Solid, quote chip |
| Matched quote, abstract | - | `xa` evidence | - | Dashed, "abstract only" |
| Model statement | Gist, role, model edge, model background | Summary, difference, direction text | Model agreed, Assumed by model | Dashed or tagged "(model)" |
| User statement | Note, pin | Idea text, marks, confirmation | Given, Checked by you | Plain, "you" |

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
| 20 | Verified | Solve: only with Lean. Investigate and Explore: "verified quote", "quotes not verified on this machine". | On screen, in every mode, "verified" is used only for Lean. Quotes are "matched" ("quote matched at lines 212-219", "4 quotes could not be matched"). Internal names may keep "verify". |
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
| Investigate node | "Has this been extended?" | Explore | Idea text prefilled from the statement (editable). `papers`: this paper as `origin` and its resolved bibliography as seeds. If a prior-work check ran: its coverage, so shown queries are not repeated | Explore form with terms, library matches and first queries. **The origin paper is excluded from "found" verdicts** and its own references are marked "cited by the origin" |
| Investigate prior-work result | "Search wider in Explore" | Explore | Same as above plus the earlier candidates | Same |
| Explore direction (Concrete or Loose) | "Open in Solve" | Solve | `solve_setup` with Solve's field names (3.1 row 9). `known[]` items become `context` with their `xe` quotes; `check.reproduces` becomes a suggested done criterion; `risks` and `stop_if` become user-visible notes | Setup card; known results listed as Cited once matched; "Check first" shown above the done checklist |
| Explore paper row or map node | "Investigate this paper" | Investigate | `papers`: one `PaperRef`. `context`: the quotes Explore already matched in it; the parts of the idea as a focus | Skeleton from the library blob in seconds; each earlier quote appears as a note on the innermost node whose span contains it |
| Solve step | "Cite a paper" (attach a source to an assumption) | within Solve, library | A `PaperRef` picked from the library or an arXiv id; the user or the model points at lines; `verify_quote`; the step becomes Cited | Cited chip; click opens the paper in Investigate at the node whose span contains the lines (a `NodeRef` computed by span containment, no model) |
| Solve run | finished | origin session | Nothing is written into the origin; it reads the child's status | "Solved" chip on the Investigate node or the Explore direction |
| Solve step | "Has this been computed before?" | Explore | Statement rendered from the step's handles | Later; not in the two-month plan |

### 4.3 What each design must change for this

- investigate.md 4.4: `.handoff` "gains optional fields" becomes "old kind unchanged; new hand-overs write `handoff.packet`". V16 becomes two buttons, the filling of the packet from a node and the receiving side, on top of F7; the third button ("Search wider in Explore") is in V14.
- explore.md: `idea.handoff` becomes `handoff.sent`; `Direction.solve_setup` uses Solve's names; add the "exclude origin paper" rule to the verdict function with a test.
- solve.md 8.4: the prefill reads a packet; `solve.setup.origin` is a `NodeRef`. The prefill from a packet is built in F7 on top of S4 (milestone M3 needs it in week 8); S21 keeps only "Open in Solve" for earlier Research sessions, which is a fork plus `solve.started` (5.3), not a packet.

---

## 5. How the app presents three modes

### 5.1 Entry screen

One home screen with three cards. Each card has one input and shows deterministic content before any
model call.

| Card | Input | What appears with no model call | Verb on the button |
|---|---|---|---|
| **Investigate** a paper | arXiv id, DOI, link, or a dropped file | Metadata, then the skeleton | Open |
| **Explore** an idea | Free text, TeX allowed | Term chips, library matches, the exact first queries | Explore |
| **Solve** a problem | Statement, TeX allowed | Kind guess, symbols table, default done checklist | Run |

- Paste detection: text that is only an arXiv id, DOI or arXiv link focuses the Investigate card whichever box it lands in. Nothing else is guessed; the user picks the mode.
- Below the cards: recent sessions with a mode tag, and "Library".
- A card for a mode that is not usable yet is hidden behind a setting, not shown greyed out.
- With no model connected, each card still says what works: Investigate skeleton; Explore search and library; Solve setup, and re-running checks of earlier sessions.

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
| Investigation session | Opens in Investigate, upgraded in place | Skeleton appended on first open if the source files still match their hashes; old map under "Earlier map"; nothing recorded changes; profile `investigation/1` gains the four tools on resume |
| Research session | Stays as it is, tagged "Research (earlier)" | Resumable under `research/1` with its recorded prompt; gains "Open in Solve" (fork plus `solve.started`) once S21 exists |
| Research session created by an old Investigation hand-off | Research (earlier) | Unchanged |
| "New investigation" button | Investigate card | From F5 on |
| "New research" button | Stays, under "More", until Solve can run a problem end to end (S13) and Explore can map an idea (X13); then hidden behind a setting | Until Explore exists, an idea typed into Research keeps working as today, with the one-line hint to Explore once it ships |

A session started after F1 records a profile and will not resume in an older build. Say so in the release notes once.

---

## 6. One build order for the first two months

Assumptions: four lanes (A Investigate Rust, B Solve Rust and Python, C foundation and literature Rust,
D UI in plain JS), one integrator who merges. On the 14.8 GB reference laptop only one heavy cargo build
runs at a time, so lanes that build locally take turns or build elsewhere; lane D needs no cargo build.
Lane C owns every edit to `resume.rs`, `routes_session.rs`, `turn_runner.rs` and `http_srv.rs`, so those
files are never edited by two lanes in the same week.

Items inside one numbered step can run in parallel. A step starts when the listed dependencies are merged.

1. **Week 1.** `F0` (C) | `F2` (C) | `F8` harness plus the Solve problem set, former S0 (B) | `V1` starts (A) | `S2` (B) | `F6` (D).
2. **Week 2.** `F1` (C, needs F0) | `F4` (C) | `V1` finishes (A) | `S3` then `S6` (B) | `F5` UI part (D).
3. **Week 3.** `F5` Rust part (C, needs F1) | `F3` (C, needs F2) | `V2` (A, needs V1, F2) | `S4` (B, needs S2, F5) | `V3` UI against fixture JSON (D, needs F6).
4. **Week 4.** `V3` Rust part and joining (A, needs V2) | `S7` (B, needs F1, S2, S6) | `F11` starts (C, needs F2, F3) | `S5` (D, needs S4).
   **Milestone M1: skeleton of an ingested TeX paper on screen, zero model calls; build time measured on the fixtures.**
5. **Week 5.** `V4` (A, needs V3, F3) | `S8` (B, needs S7, F4) | `F11` continues (C) | `V5` (D with A for the Rust half, needs V2, V3).
6. **Week 6.** `V6` starts (A, needs V3, F1, F4, F9) | `S9` starts (B, needs S3, S8) | `F9` then `F10` (C, needs F2) | `V13` whole map without novelty toggles (D, needs V5, F6).
   **Milestone M2: paste an arXiv id, structure in under 10 s on the evaluation papers; skip plan and symbols; a Solve step with a claim reaches Computed in tests; literature service answers against the loopback mock.**
7. **Week 7.** `V6` finishes (A) | `S9` finishes (B) | `F12` (C, needs F11) | `X1` terms, queries, rank (C or D, needs F2) | `S13` starts against fixture views (D, needs S5).
8. **Week 8.** `V7` Orient (A, needs V6) | `S11` gates and finish (B, needs S3, S9, F9) | `F7` hand-off packet (C, needs F4, F5, S4) | `V7` badges UI (D).
   **Milestone M3: Orient shows main results and the authors' labels with quotes on the evaluation papers; the Solve evaluation set passes headless (planted errors end Failed, never Computed); one hand-off packet opens a Solve setup card from an Investigate node.**

Next, in order, after week 8: `S12` and `S13` (a Solve run on screen), `V8` (open any node), `X3` and
`X4` (Explore session and first screen, model-free), `S10`, `V9`, `V11`, `S14`, `X6` to `X8`, `V14` (on
F12), `S15`, `X9` to `X13`.

What can move:

| If | Then |
|---|---|
| Only three lanes | Drop lane D as a separate lane; UI packages follow their Rust halves; every milestone slips about two weeks. Keep the order |
| Only two lanes | Lanes A and C only. Solve starts after M2. F1 still comes first |
| A fifth worker | `X3` and `X4` in weeks 6 to 8 (after F11), giving a model-free Explore screen at M3 |
| V1 fixtures show the own TeX scanner failing | Stop and decide on texlab or tree-sitter before V2; everything after V2 waits |
| S6 shows the restricted parser rejecting most real expressions | Stop S7 to S9 and widen the allowlist from the logged rejections first; a badge that almost never appears is not worth the rest |

Gate before each milestone: the evaluation set (section 7) is run and its sheet is filed. No timing or
quality figure is quoted to users before that.

---

## 7. Evaluation set

### 7.1 Layout (F8)

Two tiers.

| Tier | Where | Content | Runs |
|---|---|---|---|
| Fixed | In the repository: `tests/fixtures/eval/` | Self-written TeX fixtures; recorded service responses served by a loopback mock; a scripted model transport; ledger fixtures of old sessions. No third-party paper text | In CI, on every change |
| Live | Outside the repository: a manifest of arXiv ids, ideas and problems with hand-made answer keys; papers are fetched, never stored in the repository | Real papers, real services, the user's real models | By hand before each milestone; results in `docs/design/eval-2026-11/` as one sheet per run |

One manifest format for all modes: `{id, mode, input, answer_key, notes}`. One sheet format:
date, build, models, model calls, wall time, the measures below.

### 7.2 Cases

| Set | Size | Content | Answer key made by hand, once |
|---|---|---|---|
| P. Papers, structure | 10 arXiv ids, at least 4 physics papers with no theorem environments, 1 over 80 pages, 1 PDF-only, 1 multi-file with custom theorem macros | Investigate skeleton | List of theorem-like statements and sections; 30 sampled "uses" edges marked right or wrong |
| P2. Versions | 3 papers in two arXiv versions each | Tag carry-over | Which statements are the same, changed, added, removed |
| P3. Authors' labels | 5 of the papers in P, 6 to 10 results each | Orient | Each result marked new, restated, background or not stated, with the sentence that says so |
| P4. Restated results | 5 results that are known restatements with the earlier source on arXiv; 5 results with no known earlier source | Prior-work check | The earlier paper and statement, or "none known" |
| P5. Cited statements | 10 citations with a locator ("[x, Theorem 2.1]") | Cited lookup | The right statement in the cited paper |
| I. Ideas | 10 ideas: 3 done, 2 done under another name, 3 done partly, 2 open as far as known. For the first three groups the key papers are given | Explore | Key papers per part of the idea; expected status per part |
| Q. Problems | 10 calculations with known closed forms (including the effective-potential example), 3 derivations with one planted error each, 2 prose-heavy proofs | Solve | Final answer; expected tier per step; the planted step |
| C. Chains | 3 from P to Q: a node of an evaluation paper whose equation is one of the Q problems. 2 from P to I: a result with a known follow-up paper. 2 from I to Q: an idea whose first calculation is a Q problem | Hand-overs | The answer of the target case |
| R. Old sessions | 4 ledgers: research, investigation before pointer citations, investigation current, hand-off child | Resume and migration | Must resume; must show the same content |

The owner chooses the real papers and ideas; a reviewer who is not the builder makes the answer keys.
Set I is frozen as recorded responses so "open as far as known" cannot change under the test.

### 7.3 What is measured

| Measure | Applies to | Target or pass rule |
|---|---|---|
| Time from input to first deterministic content | All modes | Investigate TeX under 10 s; Explore candidates under 6 s; Solve setup under 1 s |
| Model calls and tokens per run, against the cap | All | Never above the cap; typical value recorded |
| Theorem-like statement recall; section recall | P | Recorded; below 0.9 on TeX papers with theorem environments blocks M1 |
| "Uses" edge precision, statement and proof roles separately | P | Recorded |
| Tags unchanged for unchanged statements; wrong carry-overs | P2 | Wrong carry-overs must be 0 |
| Authors' label agreement with the key; share of labels whose quote contains a cue | P3 | Recorded |
| Known earlier source found; "none in scope" shown for a result that has a known earlier source | P4, I | The second number is the false-negative rate that decides whether the feature is announced |
| "Judged same" where the key says different | P4, I | Recorded; every such case reviewed |
| Wrong statement shown as located | P5 | Must be 0 (candidates list is acceptable) |
| Known-paper recall of the search alone, before any model call | I | Recorded per service set (keyless; with keys) |
| Quotes accepted per model call; quotes rejected and why | Investigate tasks, Explore readers | Recorded |
| Planted error ends Failed | Q | Must be 100 percent; ending Computed is a release blocker |
| Share of steps Computed, Model agreed, Unchecked | Q | Recorded; decides whether SymPy's reach is enough |
| Final answer equals the key | Q | Recorded |
| Quotes surviving re-matching after a hand-over of unchanged bytes | C | Must be 100 percent |
| Target case solved correctly when started from a hand-over | C | Recorded |
| Old ledgers resume; views equal before and after | R | Must pass; in CI |
| Peak memory and build time on the reference laptop | All | Recorded at each milestone |
| A view never shows "new", "novel", "proved" or "verified" outside the allowed places | All | Property tests in CI |

---

## 8. Decisions for the owner

| # | Question | Recommended |
|---|---|---|
| 1 | Adopt tool profiles (F1) as the single way a session's tools are fixed, before any mode adds a tool? | Yes. It settles Investigate decision 17 and replaces S1 and X0. |
| 2 | Two-month priority | Investigate through Orient, the Solve engine headless, the shared literature service; Explore's screen first thing in month 3. A fifth worker buys the model-free Explore screen in month 2. |
| 3 | Is "verified" reserved for Lean on every screen, with quotes called "matched"? | Yes. |
| 4 | Is a dashed line or border always "not checked by code"? | Yes; statement versus proof use is shown by weight and a toggle. |
| 5 | One library for all modes, with the fetch cache and cited-paper skeletons in it? | Yes, global, rebuildable; user marks backed up. |
| 6 | Entry screen | Three cards with one input each and paste detection for paper ids; no mode guessing beyond that. |
| 7 | "New research" | Under "More" until Solve (S13) and Explore (X13) both work; then behind a setting. Earlier sessions stay resumable. |
| 8 | Hand-overs re-match every quote in the new session and never carry a badge | Yes. |
| 9 | Should Investigate's "earlier statement" judgement need the same one-click confirmation as Explore's "same"? | Yes; until then it reads "Model judges: ...". |
| 10 | Shared event kinds `handoff.packet`, `evidence.quote`, `evidence.abstract`, `context.supplied` instead of one per mode? | Yes; none of the per-mode kinds they replace has shipped. |
| 11 | Rename Explore's "leg" to "exchange" and Investigate's "check" to "prior work"? | Yes, now, while it costs nothing. |
| 12 | Who makes the answer keys of the evaluation set, and which real papers and ideas go in? | The owner picks; someone other than the builder writes the keys before M1. |

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
  holds the Solve prefill from a packet and the smallest "Derive in Solve" filler; whether the
  `independence` function should live in F9 instead of F12, since Solve's S14 needs it.
