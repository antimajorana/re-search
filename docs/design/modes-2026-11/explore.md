# re:Search - Explore mode design

Reconciled with 00-overview.md on 2026-11-25; shared packages are referred to by their F ids.

Scope revision of 2026-11-25 (owner decisions, overview section 0.1), applied below:

- **Bring your own papers.** The library gains the user's own sources (F13: a Kandinsky vault, a
  folder, a `.bib` file, later Zotero), kept in step by sync. Explore searches them and can read
  papers the user owns, paywalled ones included.
- **Old and PDF-only papers are essential.** Reading a PDF is a normal path on the shared PDF
  package F14 (page-broken text, page links, scans tagged "OCR").
- **A write-up:** the cited summary (F15). **Corrections are reused** (F16): marks, confirmed and
  rejected "same" judgements, and notes the user chooses to keep.
- **Explore is now Investigate's only path to earlier literature**, through "Search wider in
  Explore" (Investigate's own prior-work check, V14, is deferred).
- **Lean is cut**: `lean` is no longer a method of a first calculation, and the word "verified" is
  shown nowhere.
- In the revised build order the shared literature services (F11) and coverage (F12) come in
  weeks 9 to 11, so Explore's first screen moves to about week 11 (overview section 6).

Written 2026-11-25. One design, merged from two proposals (researcher-first and architecture-first)
after a hostile review of both. Code facts were checked read-only on `origin/gemini/work/fixed`
(commit 824e7c4); nothing was built or run. All waiting times and model-call counts below are
estimates until the evaluation set (shared harness F8, set I; Explore's part is package X14)
measures them.

Terms used throughout:

- **Shared package (F0 to F16)**: a piece of code that all three modes (Investigate, Explore, Solve)
  use and that is built once. The packages are defined in `00-overview.md` section 2 ("the
  overview" below). This document names them by id and describes only what is specific to Explore.
- **Exchange** (previously called "leg" in this document): one model call with one job (for example
  "sharpen the idea"). Its only accepted output is one structured record that the app validates. An
  exchange has a fixed maximum price: 1 model call plus at most 2 repair calls. The word "leg" now
  keeps the meaning it has in the code and in Solve (a stretch of up to 48 turns) and is not used
  for Explore.
- **Model call**: one request to the model provider. Every cap and every screen counts model calls.
- **Recorder**: the validator that writes the ledger. In Explore a recorder is the single output of
  an exchange (in Investigate the same kind of validator is offered as a registry tool). Faults go
  back to the model at once as an error message, never as a prompt reminder.
- **Ledger**: the existing hash-chained, append-only event store (`crates/ledger`).
- **Evidence**: a verbatim quote that the app found, by itself, in text it fetched and pinned by
  sha256. Two grades: *full-text* (lines of an ingested paper) and *abstract* (the abstract returned
  by a search service).
- **Matched**: the word shown on screen when the app found a quote at the cited place ("quote
  matched at lines 212-219", "4 quotes could not be matched"). The word "verified" is reserved for
  a machine-checked formal proof and, since Lean was cut, is shown nowhere. Internal names
  (functions, tests) may keep "verify". A quote from a PDF names its page; a quote found in the
  text of a scanned page is "matched in OCR text" and its chip opens the page image (F14).
- **Coverage**: the app-computed record of where the search looked, what failed and what was read
  (shared package F12).
- **Paper key**: one stable name per paper across services and sessions (for example
  `arxiv:2103.01234`). Defined once in shared package F2.

---

## 1. What this mode is for

1. You write an idea the way you would in a notebook. The app sharpens it into a few checkable parts.
2. It searches the literature from several angles, including other names for the same thing, and says
   for each part: found (same), found (partly), found under another name, or not found in the searched scope.
3. Every statement about a paper is tied to a quote the app matched in text it actually fetched.
   "Not found" is a sentence written by the app from the coverage record, never by a model.
4. It draws a concept map of what is known and open around the idea, and proposes a few concrete ways
   to pursue it, each with a first calculation that can be opened in Solve.
5. Papers found are kept in a local library, so the next exploration starts faster and costs less.
   The library also holds the papers you already own, read from where you keep them and kept in
   step (F13), so a paywalled or old paper in your collection can be searched and read.
6. You can take the result out as a cited summary (F15): what was found for each part, with the
   quotes and citations, where the search looked, and the directions.

Explore has no proof duty and no verifier verdict. It is a staged pipeline run by the app, not a
free-running agent.

---

## 2. A worked example

Paper contents below are invented placeholders ([P1], [P2], ...) to show the flow. They are not claims
about the real literature.

**Idea typed by the user:** "Use the growth rate of Lanczos coefficients (Krylov complexity) as an order
parameter for the measurement-induced entanglement transition in a monitored Majorana chain. Expect
b_n ~ alpha*n in the volume-law phase and saturation in the area-law phase."

| Step | What happens | Model calls |
|---|---|---|
| Typing | On the Explore card of the home screen, below the text box, with no network: term chips (Krylov complexity, Lanczos coefficients, measurement-induced, monitored Majorana chain), guessed arXiv categories, "7 papers in your library mention Krylov complexity", and the 4 exact queries that will be sent to arXiv and INSPIRE-HEP, as editable chips. | 0 |
| "Explore" button (Ctrl+Enter), preset Standard | Session page opens. The 4 shown queries run. In a few seconds: 38 candidates with abstracts, the user's terms highlighted, badge "not read". The sharpen exchange runs in parallel. | 1 |
| Sharpen returns | Restatement. Three parts: sc-1 "b_n grows linearly in the volume-law phase", sc-2 "b_n saturates in the area-law phase", sc-3 "the growth rate changes at the transition and can serve as an order parameter". Assumptions (free fermions; operator complexity). Aliases (spread complexity, operator growth, bi-Lanczos). Five perspectives with 14 queries. Two questions with defaults already applied ("projective or continuous monitoring? default: both"). | - |
| Plan approval | One card lists the 14 queries and the services. "Allow for this session". | 0 |
| Search | 14 queries run under per-service rate limits. 121 candidates after merging. "Where I looked" fills in: arXiv ok, INSPIRE ok, Crossref ok, Semantic Scholar rate-limited (2 of 4 queries failed, in red). | 0 |
| Triage | Top 40 by rank get a label and a reason, each with a phrase the app located in the stored abstract. Groups: core (2), related (7), background (12), unrelated (19). Reading list: 6 papers, editable. | 1 |
| Read | 4 TeX sources fetched (3 s apart), 1 comes from the fetch cache, 1 ([P4], a journal paper with no arXiv version) is a PDF in the user's own library and is read from there. One reader per paper. [P1] turns "read: passages (410 of 2,300 lines), 4 quotes". [P4] turns "read: passages, 2 quotes (p. 3, p. 7)"; its text was extracted once, when the library indexed it. | 11 |
| Map and overlaps | 23 nodes. Per part: sc-1 "Model judges: partly the same: [P1] computes Lanczos coefficients for a non-Hermitian chain without measurements across the transition; quote matched at lines 212-219" (after the user's confirming click: "found (partly)"); sc-2 "Model judges: same under another name: 'spread complexity saturation', [P2]; quote"; sc-3 "not found in the searched scope". | 1 |
| Coverage line next to sc-3 | Written by the app: "Not found in 18 queries on arXiv, INSPIRE-HEP, Crossref; 40 abstracts screened; 6 papers read (5 in passages, 1 in full). Semantic Scholar was rate-limited, so the search is incomplete. Not searched: books, talks, theses outside arXiv, paywalled full text." The chip reads "not found (incomplete search)" because one service failed. Button: "Retry Semantic Scholar". | 0 |
| Directions | 4 cards. One is marked Concrete: "Bi-Lanczos coefficients along the no-click trajectory. Known: [k-3], [k-5]. Missing: no evidence in the searched scope for b_n across the transition. First calculation: the effective Hamiltonian is quadratic, so the Krylov problem is single-particle of size L; compute b_n for L = 64..512. Check first: the zero-measurement limit must reproduce [k-2]. Risk: trajectory average differs from averaged dynamics [k-7]. Stop if: profiles coincide within error on both sides at L = 512. [Open in Solve]". Two others lack a check and sit under "Loose ideas". | 1 |

Total: 15 model calls, about 3 to 5 minutes, useful from the first seconds. Reopening the session
costs nothing: everything is rebuilt from the ledger.

---

## 3. What the user sees, stage by stage

A stage rail is always visible: Idea - Sharpen - Search - Triage - Read - Map - Directions. Each stage
is idle or in one of the run states that all three modes share (overview 3.3, shared package F9):
Running (elapsed time, model calls used), Waiting for you (an approval or an answer is needed),
Paused: (reason) with Continue (stopped at a budget cap or by the Stop button; one click resumes,
nothing is lost), Interrupted with Retry (the app or the provider went away mid-run; never re-run
unasked), Failed with its reason and Retry, or Done. A cost line is always visible, drawn by the
shared run bar (`run_bar.js`, F6): "Standard - 9 of 18 queries - 3 of 6 papers - 7 of at most 24
model calls - Stop".

| Stage | On screen | Typical wait | Model calls |
|---|---|---|---|
| Before submit | The Explore card on the home screen (one of three cards: Investigate, Explore, Solve; overview 5.1, built in F5). Text box (up to 4,000 characters, TeX allowed), optional "papers you already know" (arXiv IDs, DOIs), preset dial with its price. Live local preview: term chips, detected IDs, library matches, the exact first queries and services. An "offline: library only" switch. The button reads "Explore". Text that is only an arXiv id, a DOI or an arXiv link moves the focus to the Investigate card; nothing else is guessed. | under 0.3 s per keystroke batch, no network | 0 |
| First screen | Idea card (verbatim), stage rail, cost line, library matches. | under 1 s | 0 |
| First results | Candidate list from arXiv and INSPIRE-HEP for the shown queries: title, authors, year, abstract with term highlights, badge "not read". Coverage counter starts. | 2 to 6 s | 0 |
| Sharpen | "Sharpened idea" card (the data record is still called IdeaBrief; the word "brief" is not shown, because Solve uses "resume brief" for something else): restatement, assumptions, parts (each highlighting the span of the user's own text it comes from), terms and aliases, perspectives with queries, up to 2 questions with defaults applied. All editable. | 5 to 20 s, in parallel with the first search | 1 |
| Search | One approval card for the whole plan. Then candidates merge in; "Where I looked" updates per service. | 10 to 60 s (arXiv allows 1 request per 3 s) | 0 |
| Triage | List regroups: core, related, background, unrelated (collapsed). Each label is marked "from abstract". Reading list shown and editable. | 10 to 25 s | 1 |
| Read | Per paper: queued, fetching, "from your library", "extracting text (page k of n)", reading, "read: passages (k of N lines), q quotes" or "read: full", "nothing relevant", "unavailable: not on arXiv and not in your library; attach PDF", "scan: quotes are from OCR text". Quotes appear under the row as they are matched; clicking one opens the source lines, or the page for a PDF. | 10 to 30 s per TeX paper; for a PDF whose text is not yet in the library, the extraction time measured by F14a comes on top (one logged run with today's extractor: 30 to 140 s) | 1 to 3 per paper |
| Map and overlaps | Table with one row per part of the idea: idea sentence, status chip, matched quote with paper and line link, "what differs (model)". Concept map as a grouped list and a graph kept in sync (the shared graph view, F6). | 15 to 40 s | 1 |
| Directions | Cards in two groups: Concrete and Loose ideas (collapsed). Fixed layout: Question, Known (quote chips), Missing, First calculation with "Check first", Effort, Risks, Stop if. Free text is tagged "suggestion (model)". | 10 to 30 s | 1 |
| Afterwards | Ask about a node (Conversation tab), "Search more about this node", "Who cited this since" (no model), flag a node, revise the idea (map marked stale, not deleted), Open in Solve, Investigate this paper, Write up (the cited summary, F15; no model), export. After a mark or a rejection: "Remember this?" (F16). | - | per action |

Presets (ceilings are enforced in code; "typical" is the estimate):

| Preset | Queries | Screened | Papers read | Calls per paper | Typical calls | Hard cap | Extra |
|---|---|---|---|---|---|---|---|
| Quick | 8 | 25 | 0 | - | 3 | 5 | Map from abstracts only; best headline is "possibly related (abstracts only)" |
| Standard | 18 | 40 | 6 | up to 3 | 15 | 24 | - |
| Thorough | 36 | 80 | 12 | up to 4 | 35 | 55 | Citation expansion (no model) and one self-check round |

With no model configured, the first three rows of the stage table still work (preview, search of shown
queries, candidates, library). The page says: "Search and library work without a model. Sharpening,
reading and mapping need one." Until Explore is usable its card on the home screen is hidden behind
a setting, not shown greyed out.

Wording of model text follows the two patterns all modes share (overview 3.2 row 24): a verdict by a
model starts "Model judges:"; any other model text carries the suffix "(model)", as in
"what differs (model)", "summary (model)" and "suggestion (model)". Badges are always text plus icon,
never colour alone.

Empty and failure states are specific: "Offline: showing library matches only; nothing was searched";
"All services failed: no result is shown"; "Zero candidates: try fewer or broader terms" (no verdict is
produced); "Model call failed at Triage: candidates are still listed; Retry".

---

## 4. Data model

All records are ledger events in the Explore session. Explore-only records have the kind prefix
`idea.` (not `explore.`: that prefix is already used by `delegate_explore`, see Appendix A). Records
that every mode writes use the shared neutral kinds of the overview: `evidence.quote`,
`evidence.abstract`, `context.supplied` (F4), `handoff.packet`, `handoff.sent` (F7),
`literature.fetched` and `source.ingested`. Rust types for Explore-only code live in
`crates/research-tools/src/idea_*.rs` with `#[serde(deny_unknown_fields)]` on recorder inputs. Views
read events through `serde_json::Value` so older events stay readable.

### 4.1 Structures

**PaperKey** (string). Defined in shared package F2 (`research-tools/src/paper_key.rs`), not here.
First available of `arxiv:<base id, no version>`, `doi:<normalised>`, `inspire:<recid>`,
`s2:<paperId>`, `openalex:<W id>`, `file:<first 16 hex of sha256 of the raw file>` (the last form is
for dropped or attached files). Records sharing any identifier are merged. Where a version or the
other identifiers are needed, Explore uses F2's `PaperRef` {`key`, `version`, `ids`}. A pointer into
another session is always F2's `NodeRef` {`session`, `mode`, `id`, `paper`, `node_fp`}, never a bare id.

**IdeaStatement** - `rev`, `text` (verbatim, <= 4,000 chars), `text_sha256`, `fields[]` (arXiv
categories), `seeds[]` (PaperKey the user already knows), `author: "user"`.

**IdeaBrief** - `idea_rev`, `restatement` (<= 1,200), `assumptions[]` (<= 8),
`subclaims[1..4]` {`id: sc-N`, `text`, `idea_quote` (exact substring of the idea text)},
`terms[]` {`term`, `aliases[]`}, `perspectives[2..6]`, `questions[0..2]` {`question`, `options[]`,
`default`}, `exclusions[]`.

**Perspective** - `id: p-N`, `kind`: exact | other_names | generalisation | special_case | method |
obstruction | adjacent_field, `question`, `queries[1..4]` {`provider`, `q` (<= 300)}.
`exact` and `other_names` are required by code. `obstruction` is optional (forcing it produces junk
queries for many ideas).

**SearchPlan** - `round`, `queries[]` {`qid`, `perspective_id` or null, `provider`, `q`, `relation`
(none | cites | cited_by + key), `origin`: user | model | app}. `user` = strings the user saw in the
form; `app` = citation expansion and seed lookups.

**Candidate** - `key`, `ids{}`, `title`, `authors[<=8]`, `year`, `abstract` (optional, <= 3,000),
`abstract_ref` {`raw_blob`, `provider`, `verbatim: bool`}, `found_by[qid]`, `perspectives[]`, `score`,
`citation_count`, `withdrawn_or_retracted`, `in_library`, `owned` (app-filled: a file of this paper is
in one of the user's own sources, so it can be read without a fetch), `user_seed`, `cited_by_origin` (app-filled;
true when the session was opened from an Investigate node and the origin paper cites this one, see
rule V9). Score is deterministic:
reciprocal-rank fusion over (query, service) lists, plus number of distinct perspectives, plus a
small log(citations) term. Cap 200 per round. `verbatim` is false for abstracts rebuilt from an
index (OpenAlex); those can be shown and triaged but can never be evidence.

**Triage** - `items[]` {`key`, `relevance`: core | related | background | unrelated, `subclaim_ids[]`,
`why` (<= 200), `abstract_quote`}, `reading_list[PaperKey]`.

**PaperRead** - `key`, `version` (for example v2), `source_id`, `source_seq`, `origin`: arxiv_eprint |
arxiv_pdf | library (the fetch cache) | user_library (a file of the user's own sources, F13) | upload,
`text_basis` (app-filled, from the source receipt: tex | pdf_text | scan_text_layer | ocr; F14), `status`: read_full | read_passages | nothing_relevant | unavailable |
failed, `reason`, `lines_sent` (a count; the ranges themselves are in `context.supplied` events, F4),
`total_lines`, `raw_blob`.

**Evidence** - id `xe-<ledger seq>` (full text) or `xa-<ledger seq>` (abstract). Stored in the shared
event kinds of F4, not in Explore-only kinds.
Full text (`evidence.quote`): F4's `QuoteRef` (`source_id`, `path`, `start_line`, `end_line`,
`file_sha256`, `quote_sha256`) with `cited_range`, `quote` (at most 300 characters), `excerpt`,
`source_seq`, `caller`, plus `paper_key` and the Explore fields, which are optional in the shared
payload (the page of a PDF quote and the tag "OCR" are not stored: the view computes them from the
lines and from the source's `text_basis`): `role`: states_result | defines | poses_open_question | method | limitation |
claims_novelty | negative_result, `subclaim_ids[]`, `gist` (<= 240, model text, always shown next to
the quote).
Abstract (`evidence.abstract`): `paper_key`, `quote`, `start`, `end` (offsets filled by the app),
`raw_blob`, `provider`.

**MapNode** - `id`, `kind`, `rev`, `label` (<= 120), `statement` (<= 1,400, shown as "summary (model)"
beside the quotes), `evidence[]`, `relation_to_idea`: same | special_case | generalisation |
ingredient | obstruction | analogue | background, `subclaim_ids[]`, `papers[]` and `year` (app-filled),
`status`: active | not_in_current_map | flagged.

| Kind | Id prefix | Evidence required | Extra fields |
|---|---|---|---|
| concept | c- | >= 1 xe or xa, or an `idea_quote` from the idea text | `aliases[]` |
| definition | d- | >= 1 xe with role defines | `symbol`, `conventions` |
| known_result | k- | >= 1 xe with role states_result or negative_result | `hypotheses[]`, `regime` |
| open_question | q- | >= 1 xe with role poses_open_question | `stated_year` (app-filled), `later_work_checked` (app-filled) |
| related_work | w- | >= 1 xe or xa | `paper_key`, `basis`: fulltext or abstract (app-computed) |
| direction | r- | see Direction | - |

Node ids are sequential and assigned by the app. Identity across re-mapping is the pair (kind, first
listed evidence id): the same pair gets the same id; a new pair gets a new id; two nodes with the
same pair in one call are rejected as duplicates. A node that had a user note and is missing from a
new map is kept with status `not_in_current_map`, never deleted.

"definition" is also a node kind in Investigate, where code finds it in the paper; here it needs a
matched quote with role defines. The same word is kept; the graph legend is per mode. What a node
rests on is shown by the display grade all modes share (overview 2.10): coverage and verdict text are
"by code"; a node on `xe` evidence is "matched quote, full text" (solid, quote chip), or "matched quote, scanned page" with the tag
"OCR" when the paper is a scan; a node on `xa`
evidence only is "matched quote, abstract" (dashed, "abstract only"); summaries, differences and
direction text are "model statement" (dashed or tagged "(model)"); the idea text, marks and
confirmations are "user statement" (plain, "you").

**MapEdge** - `from`, `to`, `kind`: defines | uses | special_case_of | generalises | contradicts |
answers | leaves_open | same_as | cites, `basis`: evidence | metadata | model_inferred, `evidence`
(optional xe), `reason` (<= 400, required when model_inferred). `cites` edges are built by the app
from service data.

An edge is read as the sentence "`from` `kind` `to`". The recorder schema states this reading for
every kind, so the model cannot swap the ends silently. The shared graph view (F6) wants every edge
as `src` (the thing relied on, drawn above) and `dst` (the thing that relies on it, drawn below).
`idea_view` maps each kind to that form with one fixed table:

| Kind | Reading | `src` (relied on) | `dst` (relies on it) |
|---|---|---|---|
| defines | definition `from` defines concept `to` | `from` | `to` |
| uses | `from` uses `to` | `to` | `from` |
| special_case_of | `from` is a special case of `to` | `to` (the general statement) | `from` |
| generalises | `from` generalises `to` | `from` (the general statement) | `to` |
| answers | result `from` answers open question `to` | `to` (the question) | `from` |
| leaves_open | result `from` leaves open question `to` | `from` | `to` |
| cites | paper `from` cites paper `to` | `to` | `from` |
| contradicts, same_as | symmetric | none: drawn without an arrowhead, both nodes on one layer | - |

In the GraphView the edge `basis` becomes "evidence" (basis evidence), "code" (basis metadata) or
"model" (basis model_inferred). Line style is the one all modes share: solid means found by code or
backed by a matched full-text quote; dashed means model-inferred, abstract-only or unchecked.

**Overlap** (model-supplied) - `subclaim_id`, `paper_key`, `relation`: same | partial | other_name,
`evidence[xe]`, `difference` (<= 600), `other_name` {`term`}.

**Verdict** (app-computed, per part) - `subclaim_id`, `status`: the shared `PriorWork` status of F12
(`judged_same` | `judged_partial` | `judged_other_name` | `abstract_only` | `none_in_scope` |
`incomplete`), `overlaps[]`, `confirmed_by_user: bool`, `text` (app template from F12's wording
module). There is no value meaning "new" or "novel". The same enum answers Investigate's question
"is this result earlier".

| Shared status | Previously in this document | Chip on screen |
|---|---|---|
| `judged_same` | found_same | "Model judges: same" until the user confirms, then "found (same)" |
| `judged_partial` | found_partial | "Model judges: partly the same" until the user confirms, then "found (partly)" |
| `judged_other_name` | found_other_name | "Model judges: same under another name" until the user confirms, then "found under another name" |
| `abstract_only` | possibly_related_abstract_only | "possibly related (abstracts only)" |
| `none_in_scope` | not_found_in_scope | "not found in the searched scope" |
| `incomplete` | search_incomplete, no_close_abstract | "not found (incomplete search)"; the sentence lists the failing gate conditions (for Quick: "no close match among N abstracts; nothing was read") |

All three `judged` statuses rest on a relation the model typed, so each is shown as a model verdict
until the user confirms it (overview 2.9 and 3.2 row 24; Investigate does the same). The exact
sentences come from F12's wording module; the chips above are provisional until F12 fixes them.

**Coverage** (the shared record of F12, `search_coverage.rs`, which Investigate adopts too;
app-computed from ledger events only) - `providers[]` {`name`, `state`: ok | partial |
failed | rate_limited | not_configured | disabled, `queries_ok`, `queries_failed`, `cached`},
`library` {`papers`, `with_full_text`, `hits`} (the user's own papers and the fetch cache, searched
locally; always listed, never counted as a service by the gate),
`perspectives[]` {`id`, `kind`, `queries_ok`, `hits`}, `candidates_total`, `screened`,
`papers` {read_full, read_passages, nothing_relevant, unavailable}, `unavailable_core[]`,
`year_range`, `citation_expansion` {seeds, hops}, `known_paper_recall` {given, found_by_search, missed[]},
`not_searched` (fixed app text: books and monographs, theses not on arXiv, talks and unpublished notes,
non-English literature, most work before 1991 without a DOI unless it is in your library, paywalled
full text that is not in your library), `gate` {each condition, `passed`}.

**Direction** - `id: r-N`, `title`, `question`, `known[]` {`text`, `node_ids[]`}, `missing[]` {`text`,
`basis`: no_evidence_in_scope | open_question(q-N)}, `first_calculation` {`setup`, `inputs[node ids]`,
`method`: by_hand | sympy | numeric | literature (`lean` was removed with Lean), `expected_form` (the shape of the answer,
not its value), `check` {`text`, `reproduces`: optional k-N}, `effort`: hours | days | weeks},
`risks[]` {`text`, `kind`: already_done | technical | no_go | low_payoff, `node_ids[]`}, `stop_if`,
`solve_setup` {`statement`, `task_kind`: calculate | prove | check_derivation, `target` {`description`,
`form`: closed_form | series | numeric | bound | proof | per_step_verdict}, `conventions`,
`source_keys[]`}, `concreteness`: concrete | sketch (app-computed). `solve_setup` uses the field names
and values of Solve's `solve.setup` (overview 3.1 row 9); `target.description` and `target.form` are
filled from `first_calculation.expected_form`. How a direction travels to Solve is at the end of 4.2.

**Budget** - `preset`, `max_queries`, `max_screened`, `max_papers`, `calls_per_paper`,
`max_model_calls`, `used` {by stage}, `state`: running | stopped_at_cap | done. Model calls are
counted from ledger events by the shared function of F9 (`run_budget.rs`), not by Explore code. The
stored states are shown with the shared run-state words: running as "Running", stopped_at_cap as
"Paused: (reason)" with Continue, done as "Done".

### 4.2 Ledger events (all new, additive, schema_version 1)

Kinds without the `idea.` prefix are shared with the other modes and defined in the overview (F4, F7,
F11); they are listed here because Explore writes them.

| Kind | Written by | Payload |
|---|---|---|
| `idea.started` | app (host only) | preset, budget, pipeline_version. The mode marker: a session is Explore when its latest host-written mode marker is `idea.started` (8, F5). It is the first event of a session created on the Explore card; in a session opened by a hand-over it follows the copied `source.ingested` events and the `handoff.packet`. |
| `idea.stated` | user action | IdeaStatement, one per revision |
| `idea.brief` | sharpen recorder | IdeaBrief, request_seq |
| `idea.answer` | user action | question id, answer; brief edits by the user |
| `idea.search_planned` | app | SearchPlan |
| `idea.search_finished` | app | round, per-query status: ok, failed, rate_limited, not_configured, skipped_budget; http status |
| `literature.fetched` | app (existing kind, written by F11) | as today plus optional `qid`, `run_id` and `cached_from_ms`; raw response stays in `raw_blob` |
| `idea.candidates` | app | round, merged Candidate list |
| `idea.citation_net` | app | seeds, nodes, edges, provider |
| `idea.triage` | triage recorder | Triage, request_seq |
| `evidence.abstract` (shared, F4; previously `idea.abstract_evidence`) | app | one per matched abstract quote, id `xa-<seq>` |
| `idea.paper` | app | PaperRead, one per paper |
| `source.ingested` | existing ingest | written when a paper enters the session |
| `context.supplied` (shared, F4) | app | `run_id`, `caller`, `ranges[]` {`source_id`, `path`, `file_sha256`, `start_line`, `end_line`}: the lines the app put in front of a reader. One per message that carries paper lines. Lines returned by `read_paper_lines` and `search_paper_passages` in Conversation turns are credited by the same F4 function. |
| `evidence.quote` (shared, F4; previously `idea.evidence`) | reader recorder | one per matched full-text quote, id `xe-<seq>` |
| `idea.map` | map recorder | nodes, edges, overlaps, dropped items with reasons |
| `idea.verdict` | app | Verdict list + Coverage, written at the same head as the map |
| `idea.directions` | directions recorder | Direction list, map_seq |
| `idea.note` | user action | target (node, direction, candidate, verdict), action: wrong | not_relevant | pin | key_paper | confirm_same | reject_same | comment. These are the corrections F16 reads (4.4) |
| `lessons.supplied` (shared, F16) | app | `run_id`, `lesson_ids`, `blob`: the notes from the user's earlier corrections that an exchange was given |
| `idea.stage` | app | stage, state: requested | finished | failed | stopped_at_cap | cancelled, request id. Shown with the shared run-state words (overview 3.3): requested is "Running" (or "Waiting for you" while the plan approval is open), finished is "Done", failed is "Failed", stopped_at_cap and cancelled are "Paused: (reason)" with Continue; a stage left at requested by a crash is "Interrupted" with Retry (rule S3). |
| `idea.exchange` (previously `idea.leg`) | app | exchange name, attempt, model, tokens in and out, request and response blob hashes, accepted, faults |
| `handoff.sent` (shared, F7; previously `idea.handoff`) | app | written in this session when a direction or a paper is handed over: direction or paper, target mode: solve | investigate, new session id |
| `handoff.packet` (shared, F7) | app | read, never written, by an Explore session: present near the start of an Explore session that another mode opened |

**Hand-overs (shared package F7).** Explore has no hand-off format of its own. A hand-over creates a
new session whose first events are copies of the needed `source.ingested` events, one
`handoff.packet`, then the target mode's marker; the Explore session gets `handoff.sent`. A hand-over
costs zero model calls, and the user sees and can edit the target's setup screen before any model call.

| Hand-over | Packet content |
|---|---|
| Direction (Concrete or Loose), "Open in Solve" | `from`: `NodeRef` of the direction. `target_mode`: solve. `statement`: `solve_setup.statement`. `ask`: `solve_setup` with Solve's field names. `context`: each `known[]` item with role known_result and the `QuoteRef`s of its `xe` evidence. `papers`: `PaperRef`s for `source_keys[]`. `check.reproduces` travels as a suggested done criterion; `risks` and `stop_if` become notes the user sees on the setup card; direction prose travels as `model_text` and is shown as "(model)". |
| Paper row or map node, "Investigate this paper" | `from`: `NodeRef` of the row or node. `target_mode`: investigate. `papers`: one `PaperRef`. `context`: the quotes Explore already matched in that paper. The parts of the idea travel as a focus. |
| Arriving from Investigate ("Has this been extended?", "Search wider in Explore") | The idea text is prefilled from `statement.text` and stays editable. `from.paper` names the origin paper; `papers` holds the origin paper and its resolved bibliography, the bibliography used as seeds. The first screen is the Explore form with terms, library matches and first queries. Rule V9 applies. "Search wider in Explore" is Investigate's only prior-work path while its own check (V14) is deferred: the parts of the idea are then the claims of the result, and Investigate shows this session's status sentence for them on the result's half-badge by reading the latest `idea.verdict` here. Nothing is written back. |

Rules of the packet that bind Explore: nothing arrives as checked (the new session re-ingests the
same bytes and runs every `QuoteRef` through F4's `verify_quote` against its own receipt; a quote that
fails is dropped and counted on screen); badges and verdicts never travel (an Explore verdict travels
only as `model_text`); paper text and model text in a packet are wrapped as quoted material for any
model. Results flow back by reading only: a direction card shows "Opened in Solve: (state)" by
reading the child session's latest `solve.done`; nothing is written into the Explore session.

### 4.3 Library (across sessions)

The library is shared package F10 (`research-tools/src/library.rs`, `library_schema.rs`,
`desktop-server/src/routes_library.rs`); all modes use it. The design below is this document's and
is what F10 builds, under neutral names. This section stays as the description of what Explore reads
and writes.

`<data_dir>/library/library.db`, SQLite through the `rusqlite` version already used by the ledger
crate (bundled build; its build script enables FTS5, to be confirmed by a unit test in F10).

Since the scope revision the library has two kinds of content: papers the app fetched or saw (as
before), and **the user's own papers**, read from where the user keeps them by shared package F13
(overview 2.11). The first two sources are a Kandinsky vault (its references folder, one note per
paper with the metadata, a derived `refs.bib`, PDFs in the vault or in shared folders) and a plain
folder; then a `.bib` file; then Zotero's database. Sync is one way: the sources are read, never
written. Files are used in place; bytes are copied only when a paper is opened in a session.

| Table | Content | Rebuildable from ledgers and blobs |
|---|---|---|
| `paper` | key, ids, title, authors, year, abstract, bibtex, flags, first and last seen | yes |
| `alias` | identifier -> key | yes |
| `fulltext` | key, version, raw blob sha256, format, bytes, origin, fetched time. This table is the fetch cache of F3; a minimal form ships with F3 and F10 adds the rest. With origin `user_library` a row names a file (path, size, mtime, and sha256 once read) instead of a blob | yes |
| `source` (F13) | id, kind (kandinsky_vault, folder, bibtex, zotero), root, settings, last sync, state | no (the user's settings) |
| `source_item` (F13) | source, external id (the user's citation key, or the path), key (empty until the file has an identifier or a hash), ids, files with size and mtime, state (present, changed, missing, possible_duplicate) | yes, by scanning the source again |
| `paper_text` (F14) | key, version, file sha256, extractor, text blob, page basis: PDF text extracted once per file | yes |
| `seen` | key, session, role: candidate, triaged, read (written by Explore); investigated, cited_lookup, solve_source (written by the other modes) | yes |
| `skeleton` | key, version, builder, blob sha256: the structure of a cited paper, built once by Investigate. Explore does not write it | yes |
| `finding` | key, session, evidence seq, role, gist, quote, cited range | yes |
| `search_cache` | service, url sha256, raw blob, fetched time (7-day reuse) | no (cache only) |
| `paper_fts` | FTS5 over title, abstract, gists, and the full text where `paper_text` has it | yes |
| `user_mark` | key, status: unread, skimmed, key, irrelevant; tags; note | no (user data; included in backups) |

Full-text bytes are not copied: they are the ledger's content-addressed blobs. Reusing a paper in a
new session means re-ingesting the blob through `ingest_bytes`, which writes a fresh `source.ingested`
event, so the new session checks quotes against its own pinned copy. The same holds for a file of
the user's own sources: opening it ingests its bytes into the session, so moving, editing or
deleting the file later changes nothing in that session. The library is an index and a
cache. It is never evidence.

### 4.4 What the user's own papers and corrections change in Explore

- **Preview and candidates.** Library matches include the user's own papers. A candidate that the
  user owns is marked "in your library" (`owned`) and costs no fetch.
- **Reading.** The reading queue takes a paper from the library first (F3's order). A paywalled
  paper that the user owns is read like any other. PDF text comes from F14: quotes name the page;
  quotes from a scan are tagged "OCR"; a `judged` status may rest on them, tagged so (overview 8.3,
  question 3).
- **Coverage.** The library is listed under "Where I looked" with its counts. It is not a service
  for the gate (rule V3 still needs two literature services), because a private collection is not
  an index of the literature.
- **Seeds.** "Papers you already know" can be picked from the library; the user's collections and
  tags are shown as filters only and are never sent to a service.
- **Write-up.** The cited summary cites with the user's own citation keys where the paper is in
  their library, so it drops into a document that already uses their `refs.bib`.
- **Corrections (F16).** By code: "irrelevant" demotes a paper in later sessions and "key paper"
  makes it a seed when its terms match (decision 11); a confirmed "same under another name" adds the
  two names to the alias list used when later queries are built; a rejected one is not proposed as
  the same again. As notes for the model: only when the user says "Remember this", in their own
  words, supplied to the sharpen, triage and map exchanges as quoted data and recorded in
  `lessons.supplied`. Neither changes a verdict: rule V10.

---

## 5. Tools

Three kinds of callable things exist. Only the second kind has tool definitions with a `def_hash`.

### 5.1 Exchange recorders (app functions; no tool definition, no def_hash)

On API routes the exchange offers the model exactly one tool spec built from the recorder's schema. On
CLI routes (later, X18) the same record arrives as a JSON block. Either way the same validator runs and
every attempt is audited as `idea.exchange`. Any other tool call in an exchange is answered with an
error.

| Recorder | Input (from the model) | Output | Called in |
|---|---|---|---|
| `record_idea_brief` | IdeaBrief without ids | brief seq, or faults per field | Sharpen exchange |
| `record_idea_triage` | Triage | accepted items, rejected items with fault | Triage exchange |
| `record_paper_evidence` | `items[<=12]` {start_line, end_line, quote (16..300 chars), role, gist, subclaim_ids}, `want_lines[<=3 ranges, <=400 lines]`, `status`: done or nothing_relevant | per item: xe id, or fault with the true text at those lines | Reader exchange, one paper each |
| `record_idea_map` | nodes, edges (each read as "`from` `kind` `to`", 4.1), overlaps, citing evidence by id only | map seq, node ids, dropped items, faults | Map exchange |
| `record_idea_directions` | Direction list without ids (<= 5) | ids, concreteness per id, rejected items | Directions exchange |

The reader is a *passage reader*, not a tool loop. Its first message already contains the sharpened
idea (IdeaBrief), the paper outline (`build_outline`), and the top passages for the terms and aliases
with line numbers (lexical search from `paper_passages.rs`, at most 500 lines; the whole file if it is
shorter). If it asks for more lines it gets one further model call (two in Thorough). Every message
that carries paper lines is recorded as a `context.supplied` event. A quote is accepted only if it
passes F4's `verify_quote` at the cited lines and F4's `lines_sent` shows those lines were actually
sent to this reader. This is the rule all modes share: a quote counts only if it matches and its
lines were sent to the caller that cites it.

### 5.2 Explore tools: profile `explore/1` (registered only on the Explore router)

Which tools a session has is fixed by a tool profile (shared package F1): a named, frozen tool list
recorded in the session at start and at resume. Explore's profile is `explore/1` and holds exactly the
four tools below. It is used in Conversation turns. Nothing is added to the shared list `research/1`.

| Tool | Input | Output | Effect | Who may call |
|---|---|---|---|---|
| `idea_search` (new) | `queries[1..24]` {qid, provider: arxiv, inspire, semantic_scholar, crossref, openalex, theoremsearch; q (<= 300); relation: none, cites, cited_by + key; max_results 1..25} | per query: status, returned, new papers | NetworkRead, policy Ask, one approval covers the listed batch | the app (caller `host:idea:<session>`) for model-written plans; the model in Conversation turns |
| `idea_fetch_papers` (new; a thin tool over F3's `paper_fetch::fetch`) | `keys[1..8]` | per key: source_id, status, total_lines, origin | NetworkRead, Ask | the app for the reading list; the model in Conversation turns |
| `search_paper_passages` | unchanged | unchanged | ReadWorkspace | model in Conversation turns |
| `read_paper_lines` | unchanged | unchanged | ReadWorkspace | model in Conversation turns |

The provider enum lists all six services from day one, including ones that are off, so enabling a
service later does not change the schema (and therefore not the `def_hash`). A disabled service
returns status `disabled`. The Explore router has no code-execution, file-write or LaTeX tool.

### 5.3 App functions (not callable by a model)

Explore-only: `idea_terms::extract`, `idea_queries::first_queries`, `idea_rank::merge_and_rank`,
`idea_verdict::compute` (per-part verdicts from overlaps, using F12's status enum and gate),
`idea_net::build`.

Shared, called by Explore and referred to by package id: `paper_key::*` (F2), `lit_providers::run_query`
(F11), the per-host network gate `net_rate` (F3: arXiv 1 per 3 s, INSPIRE-HEP 15 per 5 s, Semantic
Scholar 1 per s, Crossref 1 per s; every outgoing request of every mode passes it), `paper_fetch::fetch`
and `fetched_sources` (F3; the session limit is the preset's `max_papers`), `quote_core::verify_quote`
and `lines_sent` (F4), `run_budget` (F9), `library::*` (F10), `search_coverage::compute` (F12),
`prior_work_wording::*` (F12; the only place user-facing verdict sentences are produced),
`paper_sources::*` (F13; the user's own papers, read through the library), `pdf_pages::*` and
`pdf_match::*` (F14; page-broken PDF text, the page of a line, matching that ignores ligatures and
line-end hyphens), `writeup::*` (F15), `lessons::select` and `lessons::record_supplied` (F16).

### 5.4 HTTP routes (new file `routes_idea.rs`; `http_srv.rs` gets two delegating match arms)

- `POST /api/idea/preview` {text} -> terms, ids, library matches, first queries. No network, no session.
- `POST /api/sessions` with `mode: "explore"` {text, seeds, preset, queries as shown} -> creates the
  session, appends `idea.started` and `idea.stated`, runs the shown queries, starts the sharpen exchange.
- `POST /api/sessions/{id}/idea/{op}` with op = revise | answer | plan | search | triage | read | map |
  directions | continue | stop | note | handoff | more.
  The `handoff` op builds a `handoff.packet` and passes it to the shared hand-off route of F7.
- `GET /api/sessions/{id}` gains `"idea": idea_view::view(events, running)`. The map part of the view
  has the `GraphView` shape that the shared graph component (F6) draws.
- Library routes belong to F10 (`routes_library.rs`), not to `routes_idea.rs`: `GET /api/library?q=`,
  `GET /api/library/{key}`, `POST /api/library/{key}/mark`, `POST /api/library/{key}/attach`,
  `DELETE /api/library/{key}`, `POST /api/library/rebuild`. The routes for the user's own sources
  belong to F13 (`routes_sources.rs`): list, add (with the first-sync report), sync now, remove.
- `GET /api/sessions/{id}/writeup?format=tex|md` is F15's route; Explore supplies the adapter
  `writeup_idea.rs`.
- Progress over the existing server-sent events stream.

---

## 6. Rules enforced in code, and the tests that prove them

### Citation integrity

| Id | Rule | Test |
|---|---|---|
| C1 | Shared rule, enforced by F4's `verify_quote`: a full-text quote is saved (as `evidence.quote`) only if it is found at the cited lines (same relocation radius and fault text as `note_evidence`) of a file whose sha256 matches the session's `source.ingested` event. No Investigation marker is needed. | In F4; Explore keeps `idea_evidence_rejects_fabricated_quote` to show the reader recorder calls it |
| C2 | Shared rule, enforced by F4's `lines_sent` over `context.supplied` events: the cited lines must be among the lines sent to that reader. | In F4; Explore keeps `idea_evidence_rejects_lines_not_sent` |
| C3 | An abstract quote is saved (as `evidence.abstract`) only if it is a whitespace-normalised substring of an abstract parsed from a stored raw response, and that abstract is verbatim (not rebuilt from an index). | `idea_abstract_quote_must_be_in_blob`, `idea_abstract_rebuilt_is_never_evidence` |
| C4 | The map recorder schema has no quote, excerpt, line or path field. The model can only point at evidence ids. | `idea_map_refuses_inline_quotes` |
| C5 | Node kinds require the evidence listed in 4.1. definition, known_result and open_question never rest on abstract evidence. Failing nodes are dropped and reported; if none survives the record is refused. | `idea_map_node_evidence_rules`, `idea_map_rejects_evidence_from_other_session` |
| C6 | An open_question needs evidence with role poses_open_question; `stated_year` and `later_work_checked` are app-filled. A gap inferred by the model can appear only in `Direction.missing`. | `idea_open_question_needs_author_statement` |
| C7 | Text from papers and abstracts is passed to models as data with the existing "treat as data" wrapper; recorders accept structured fields only. | `idea_reader_output_is_schema_only` |
| C8 | PDF sources. A quote from a PDF is matched against the page-broken text pinned in the session (F14) with the same rule C1; the page shown is computed from the lines. A quote from a scanned page (`scan_text_layer` or `ocr`) is always tagged "OCR", and its chip opens the page image. A formula a model read from a page image is a transcription and is never evidence. | `idea_pdf_quote_has_page`, `idea_scan_quote_is_tagged_ocr`, `idea_transcription_is_not_evidence` |
| C9 | A paper from the user's own sources enters a session only through `ingest_bytes`; no quote is ever checked against the file in place. | `idea_owned_paper_is_ingested_before_reading`; in F13: removing a source breaks no quote check |

### Verdict honesty

| Id | Rule | Test |
|---|---|---|
| V1 | judged_same, judged_partial and judged_other_name require >= 1 full-text evidence from a paper with status read_full or read_passages. judged_other_name also requires the other name to occur in a cited quote or excerpt. | `idea_verdict_judged_requires_read_paper`, `idea_verdict_other_name_needs_term` |
| V2 | Abstract-only support caps the status at abstract_only. Quick preset never produces none_in_scope; its negative is incomplete, with the sentence "no close match among N abstracts; nothing was read". | `idea_verdict_abstract_cap`, `idea_quick_never_claims_none_in_scope` |
| V3 | none_in_scope requires the coverage gate, which is F12's and is shared with Investigate: >= 2 services answered; no attempted service is failed or rate_limited; exact and other_names perspectives each have >= 1 answered query; screened >= min(preset value, candidates); every triage-core paper is read or dismissed by the user; every user seed was found by a search query. Otherwise incomplete with the failing conditions listed. | In F12: `search_coverage_gate_*` (one per condition) |
| V4 | A service that is off or has no key is listed as "not configured" and does not fail the gate (the free core must be able to reach a complete search). It is always shown. | In F12: `search_coverage_gate_not_configured_is_listed_not_failed` |
| V5 | Every verdict sentence comes from F12's `prior_work_wording`. Model prose appears only in `difference`, labelled. No sentence says "new" or "novel" (property test over the wording module, in F12). | `idea_verdict_text_is_app_authored` |
| V6 | judged_same is shown as "Model judges: same" until the user clicks "Looks the same to me" (`idea.note confirm_same`). judged_partial and judged_other_name are shown as "Model judges: ..." in the same way until the same click (the note action `confirm_same` covers all three `judged` statuses). | `idea_same_needs_user_confirmation`, `idea_judged_statuses_need_user_confirmation` (UI state tests) |
| V7 | Coverage and verdicts are pure functions of ledger events: recomputing equals the stored payload. Also checked on export. | `search_coverage_recomputes_from_ledger` (F12), `idea_verdict_recomputes_from_ledger` |
| V8 | Phrase check on model prose fields (`difference`, direction texts): "has not been done", "has never been", "no prior work", "nobody has", "first to", "for the first time", "novel", "unexplored" are rejected with a request for a scoped statement. Phrases, not single words such as "new" or "first". A backstop behind V1 to V5. | `idea_prose_rejects_absolute_phrases`, `idea_prose_allows_first_calculation` |
| V10 | Corrections and lessons never decide a verdict. Marks change ranking only; a confirmed alias changes later queries only; notes supplied to a model (F16) are data in the prompt. `idea_verdict::compute` and the coverage gate read none of them. The user's library is listed in coverage and is never a service for the gate. | `idea_verdict_ignores_lessons_and_marks` (same events with and without lessons give the same verdicts and gate), `idea_library_does_not_count_as_service` |
| V9 | Exclude the origin paper. When the session was opened by a `handoff.packet` whose `from.paper` is set (an Investigate node asked "Has this been extended?"), that paper never supports judged_same, judged_partial, judged_other_name or abstract_only for any part: `idea_verdict::compute` skips overlaps whose `paper_key` equals the origin's `PaperKey`. The origin's own references stay candidates and are marked "cited by the origin" (`cited_by_origin`). Without this rule every such session would report "found" by pointing at the paper the question came from. | `idea_verdict_excludes_origin_paper`, `idea_origin_references_are_marked` |

### Directions

| Id | Rule | Test |
|---|---|---|
| D1 | Rejected: unknown node ids, a `missing` item without basis, banned phrases. | `idea_direction_rejects_bad_refs` |
| D2 | `concreteness` is computed: concrete needs >= 1 known item naming a node with full-text evidence, a first calculation with setup (>= 40 chars) and resolvable inputs, a check, >= 1 risk and `stop_if`. Otherwise sketch. Sketches are kept and shown under "Loose ideas". | `idea_direction_concreteness_is_computed` |
| D3 | Only node chips count as citations; free text is tagged "suggestion (model)". | `idea_direction_marks_suggestions` (.mjs) |

### Pipeline, cost, network

| Id | Rule | Test |
|---|---|---|
| P1 | Stage order is one transition table (`next_stage(events)`); a recorder called outside its stage returns an error. | `idea_recorder_refused_outside_stage` |
| P2 | An exchange accepts only its recorder. | `idea_exchange_rejects_other_tool_calls` |
| P3 | Budget caps (queries, screened, papers, calls per paper, total model calls) are checked before every network request and every model call. Model calls are counted by F9's function over ledger events; papers fetched by F3's `fetched_sources` with the preset's `max_papers` as the limit. At a cap the stage ends as stopped_at_cap, shown as "Paused: (reason)" with "Continue (+N)"; raising a limit is a user click recorded in the ledger. Never a silent cut. | `idea_budget_stops_search_at_cap`, `idea_budget_stops_exchanges_at_cap` |
| P4 | Queries with origin user run without an approval prompt only if they are exactly the strings returned by the preview and sent back in the create request, only at session creation, at most 6, only to arXiv and INSPIRE-HEP. This is an instance of the approval rule all modes share (overview 2.4): a network request runs without an approval card only when the user's own click or keystroke named exactly that request. | `idea_user_queries_are_exactly_shown_ones` |
| P5 | Every other query goes through `Router::try_call("host:idea:...", "idea_search", ...)`, so the real policy engine, approver, grants and audit apply. No direct call path exists. | `idea_model_queries_need_approval`, `idea_search_has_no_bypass` |
| P6 | Provided by F3 (`net_rate.rs`), not by Explore code: one gate per host with an injectable clock; process-wide, shared by all sessions and all modes, so Explore and Investigate together cannot exceed arXiv's limit. Explore makes no request that bypasses it. | In F3: `net_rate_gate_spaces_arxiv_calls`; Explore keeps `idea_requests_all_pass_the_gate` |
| P7 | `research_policy()` is used unchanged and is named in profile `explore/1` (F1), so resume compares the right policy version; the Explore router registers no ExecuteCode or WriteWorkspace tool. | `idea_router_has_no_exec_or_write_tools` |
| P8 | API keys are sent only as headers to their own host; the User-Agent stays the generic `re-search/<version>`; no contact address unless the user sets one. | `idea_requests_carry_no_identity` |

### Stability

| Id | Rule | Test |
|---|---|---|
| S1 | Provided by F0: (id, def_hash, order) of every tool registered today is pinned by one golden test (profile `research/1`). | In F0: `research-tools/tests/tool_def_golden.rs` |
| S2 | Provided by F1: one golden file per profile. (id, def_hash, order) of `explore/1` is pinned; earlier definitions of a changed tool go into the profile's `earlier_defs`. | In F1: the golden file of `explore/1` |
| S3 | A stage interrupted between requested and finished is shown as "Interrupted" with Retry after restart. It is not re-run automatically (it would spend model calls unasked). Re-running does not duplicate evidence (key: paper, cited range, quote hash). | `idea_stage_resumes_after_crash` |
| S4 | Node ids are stable across re-mapping; noted nodes are never deleted. | `idea_node_ids_stable_across_remap` |
| S5 | Attaching a library paper re-ingests the same bytes; deleting `library.db` does not break quote checking in any session. | In F10: `library_attach_reingests_same_bytes`, `library_loss_does_not_break_verify` |
| S6 | Provided by F0: no source file touched by these packages exceeds 600 lines (CI check). | In F0: workflow step in `workbench-checks.yml` |
| S7 | Only the host writes the mode marker `idea.started`; no tool or recorder can append it. | In F5: the test that lists what each tool can append |
| S8 | No Explore view, export or write-up shows "verified" at all, or "new", "novel" or "proved" outside the allowed places. | Property test in CI (overview 7.3) |
| S9 | The cited summary (F15) is a pure function of ledger events; its status sentences are the stored verdict texts; the coverage paragraph cannot be left out; every citation resolves to a library entry. | Evaluation set W; `idea_writeup_recomputes_from_ledger` |

---

## 7. What is reused, changed and retired

### Reused unchanged

| What | Path |
|---|---|
| Ledger: append, append_at_head, append_new_session, blobs with sha256 re-check | `crates/ledger/src/lib.rs` |
| Registry, Router, approval store, `def_hash` | `crates/tools/src/{registry,router,approval}.rs` |
| Policy (NetworkRead is always clamped to Ask) | `crates/policy/src/lib.rs`, `research_policy()` |
| Approver with per-session network grant | `crates/desktop-server/src/{approver,session_lifecycle}.rs` |
| Allowlisted fetch | `crates/research-tools/src/http_client.rs` (`fetch_bytes_allowlisted`), reached through F3 |
| TeX and PDF ingestion, arXiv e-print unpacking | `crates/research-tools/src/{ingest,arxiv,pdf}.rs` |
| Quote verification and relocation | `crates/research-tools/src/investigation_anchor.rs` (`SourceIndex::verify_pointer`), reached through F4's `verify_quote` |
| Outline and passage search | `crates/research-tools/src/{investigation_outline,paper_passages,paper_lines}.rs` |
| Tool defs `arxiv_search`, `literature_search`, `note_evidence`, `search_paper_passages`, `read_paper_lines` | not edited (their definitions stay byte for byte), not re-registered elsewhere except the last two in profile `explore/1` |
| Transport trait for model calls | `crates/agent/src/transport.rs` |
| Source chips, math rendering | `assets/{source_links,math}.js` |
| Mode-marker pattern, linked-session pattern | `crates/desktop-server/src/investigation.rs` |

### Shared packages Explore builds on (defined in the overview, section 2)

This document used to build its own version of each piece below. Each is now one shared package.

| Package | What Explore takes from it | Replaces, in the earlier text of this document |
|---|---|---|
| F0 Guard rails | Golden test of today's tool definitions; 600-line CI check; `routers.rs` split out of `turn_runner.rs` | Package X0; rules S1 and S6 |
| F1 Tool profiles | Profile `explore/1`; resume from the recorded profile; `earlier_defs` | Section 8, Explore sessions, item 4; `idea_legacy_defs.rs`; rule S2 |
| F2 Paper key | `PaperKey`, `PaperRef`, `NodeRef`, id-prefix table | `idea_keys.rs` (part of X1) |
| F3 Network gate and paper fetch | `net_rate.rs`, `paper_fetch.rs`, `fetched_sources` | `idea_rate.rs` (part of X2), `idea_fetch.rs` (part of X9); rule P6 |
| F4 Quote core | `verify_quote`, `lines_sent`, events `context.supplied`, `evidence.quote`, `evidence.abstract` | `idea.evidence`, `idea.abstract_evidence`; rules C1 and C2; the open point on `SourceIndex` |
| F5 Mode shell | `session_mode(events)`, home screen with three cards, rail tags, mode to profile to router | Three-valued mode in X3; the edits to `routes_session.rs`, `index.html`, `app.js` |
| F6 Graph view and shared UI parts | `graph_layout.js`, `graph_view.js`, `ui_badge.js`, `run_bar.js`, `quote_chip.js` | Layout extraction in X12 |
| F7 Hand-off packet | `handoff.packet`, `handoff.sent`, receiving route | `idea.handoff`; the hand-off half of X13 |
| F8 Evaluation harness | Manifest and sheet formats, loopback mock, scripted model | The harness of X14 |
| F9 Run budget and run state | Model-call counting, run-state words, run bar | Budget accounting in X3 |
| F10 Library | `library.rs`, `library_schema.rs`, `routes_library.rs` | Package X5; `idea_library.rs`, `idea_library_schema.rs` |
| F11 Literature services | `lit_providers.rs`, `lit_abstracts.rs` | Package X2; `idea_providers.rs`, `idea_abstracts.rs` |
| F12 Coverage, verdict and wording | `search_coverage.rs`, `prior_work.rs`, `prior_work_wording.rs` | Coverage and wording parts of X7; `idea_coverage.rs`, `idea_wording.rs` |
| F13 Paper sources and sync | The user's own papers in the library: `paper_sources.rs`, adapters, `routes_sources.rs` | The "Zotero link" add-on of section 10; "attach PDF" as the only way to read a paywalled paper |
| F14 PDF and old papers | Page-broken PDF text kept once per file, page of a line, scan detection, matching for PDF text, later own OCR | The PDF handling inside X9 and X10 |
| F15 Write-up | Document model and the TeX, Markdown and BibTeX writers; Explore's adapter `writeup_idea.rs` | The Markdown export of X17 |
| F16 Corrections and lessons | Marks and aliases reused by code; notes kept on request | Decision 11 as an Explore-only rule |

The prefix `idea_` stays for Explore-only code.

### Changed (small, behaviour-preserving)

| Path (lines today) | Change |
|---|---|
| `crates/desktop-server/src/routers.rs` (new in F0) | Add `build_idea_router`, built from profile `explore/1`. The move of the existing builders out of `turn_runner.rs` (586) is F0's; the function that maps session mode to profile to router is F5's. |
| `crates/desktop-server/src/routes_session.rs` (616, already over) | No edit by Explore packages. F5 adds `session_mode` (four values) and shrinks the file; the Explore create body lives in `idea.rs`. |
| `crates/desktop-server/src/http_srv.rs` (702, already over) | Two delegating match arms only. The foundation lane owns every edit to this file (overview section 6), so the two arms are merged through that lane. |
| `crates/research-tools/src/investigation_anchor.rs` (597) | No edit by Explore. F4's `verify_quote` wraps `SourceIndex::verify_pointer` and needs only a `source.ingested` receipt; the point left open here earlier is settled there. |

No longer changed by Explore packages: `turn_runner.rs` (F0), `arxiv.rs` and `literature.rs`
(behaviour-only edits in F3 and F11; no schema change, so no def_hash change),
`research-tools/Cargo.toml` (`rusqlite` is added by F10), `assets/index.html` and `assets/app.js`
(the home screen with its three cards is F5's `new_menu.js`; Explore supplies the content of its
card in `idea_setup.js`).

Not changed, on purpose: `paper_read.rs` and `investigation_evidence.rs` (bound to Investigation
markers; Explore uses its own reader recorder on the shared quote core F4), `resume.rs` (edited once,
in F1; Explore packages never edit it).

### New files

Explore-only files:
`crates/research-tools/src/`: `idea_terms.rs`, `idea_queries.rs`, `idea_rank.rs`, `idea_brief.rs`,
`idea_triage.rs`, `idea_evidence.rs`, `idea_map.rs`, `idea_map_validate.rs`, `idea_verdict.rs`,
`idea_directions.rs`, `idea_net.rs`, `idea_tools.rs`.
`crates/agent/src/exchange.rs` (the exchange runner; previously planned as `leg.rs`).
`crates/desktop-server/src/`: `idea.rs`, `idea_view.rs`, `idea_exchanges.rs` (previously planned as
`idea_legs.rs`), `idea_read.rs`, `idea_handoff.rs` (fills the packet of F7), `idea_chat.rs`,
`idea_system_prompt.rs`, `routes_idea.rs`.
`crates/desktop-server/assets/`: `idea.js`, `idea_state.js`, `idea_setup.js`, `idea_search.js`,
`idea_map.js`, `idea_directions.js`, `library.js`, `idea.css`.
`crates/export/src/idea.rs`.

Files that moved to shared packages under neutral names (overview 3.1 row 15) and are no longer
created here: `idea_keys.rs` (now `paper_key.rs`, F2), `idea_rate.rs` (`net_rate.rs`, F3),
`idea_fetch.rs` (`paper_fetch.rs`, F3), `idea_providers.rs` (`lit_providers.rs`, F11),
`idea_abstracts.rs` (`lit_abstracts.rs`, F11), `idea_library.rs` and `idea_library_schema.rs`
(`library.rs`, `library_schema.rs`, F10), `idea_coverage.rs` (`search_coverage.rs`, F12),
`idea_wording.rs` (`prior_work_wording.rs`, F12), `idea_legacy_defs.rs` (the `earlier_defs` of
profile `explore/1`, F1), `routers.rs` (F0).

### Retired

Nothing. Two overlaps are handled without touching hashes: `delegate_explore` keeps its id and
description and only its UI label becomes "Try strategies in parallel"; Research mode keeps accepting
idea-like prompts and gains a one-line hint "This reads like an idea: open it in Explore" (UI only).

---

## 8. Migration of existing sessions and def_hash

**Existing Research and Investigation sessions: no change.** `check_tool_defs_drift`
(`crates/agent/src/resume.rs:191`) compares the recorded tool list, in order, with the registry. It
tolerates a new tool only if its id is in a hard-coded list and its effect is ReadWorkspace. Shared
package F1 moves that list into data (tool profiles) and is the only package that edits
`resume.rs`. So:

1. No existing tool definition is edited, and nothing is added to `register_research_tools`. With
   tool profiles (F1) today's list is frozen as profile `research/1`.
2. Shared package F0 pins all current hashes in a golden test before any Explore code is merged; F1
   adds one golden file per profile. (This was package X0.)
3. `LiteratureEndpoints` gains no field in the first release (no new host). When OpenAlex is added
   (X19) the new fields get serde defaults.
4. Sessions with no mode marker are tagged "Research (earlier)" and keep resuming under `research/1`.

**Explore sessions:**

1. Mode is derived from the ledger by the one shared function `session_mode(events)` (F5): the
   latest of the host-written markers `investigation.started`, `idea.started` and `solve.started`
   decides; no marker means "research". Mode therefore has four values: research, investigation,
   explore, solve. A rule "the first event is `idea.started`" would be wrong for a session opened by
   a hand-over, which begins with copied `source.ingested` events and a `handoff.packet` before its
   marker. Only the host may write a marker (rule S7).
2. Pipeline exchanges record no tool definitions and never resume a model loop. They restart from
   ledger state. Changing a recorder schema later cannot strand an old session; old events stay
   readable because views tolerate missing and unknown fields.
3. Only Conversation turns record a tool profile: `session.start` and `session.resume` record
   `tool_profile: "explore/1"` (four tools), and resume builds the expected definitions from the
   recorded profile and stays fail-closed. The two new NetworkRead tools are still made broad from
   day one (all providers in the enum, citation relations included), so the profile rarely has to
   change.
4. When one of the four changes, its previous definition goes into the `earlier_defs` of profile
   `explore/1` (F1). No Explore-specific special case is added to `resume.rs` and no
   `idea_legacy_defs.rs` is created: F1 edits `resume.rs` once for all modes, with a recorded ledger
   fixture of each existing session type that must still resume.
5. `pipeline_version` in `idea.started` lets a later app version show "made with an older pipeline;
   re-run to update" instead of misreading old events.
6. Library (F10): created empty on first use. An optional button backfills it from existing sessions'
   `source.ingested` events (pointers to existing blobs; no bytes move). The user's own sources
   (F13) are added in Settings; the first sync of a source shows a report before anything is indexed. A `schema_version` table
   guards upgrades; the derived tables can be rebuilt at any time.
7. A session started after F1 records a profile and will not resume in an older build (said once in
   the release notes).

---

## 9. Work packages

Sizes: S = up to 2 days, M = up to 1 week, L = 1 to 2 weeks (one developer or agent, with tests), as
in the overview.

Three packages of the earlier text are now shared packages and are no longer built here: X0 is F0,
X2 is F3 plus F11, X5 is F10. Their rows are kept so the numbering stays stable. Parts of X1, X3,
X7, X9, X12, X13, X14 and X15 moved to shared packages as well; their rows list only what is
specific to Explore.

**X1, X3 and X4 are the first three Explore packages to build.** X1 needs no model, no network and
no UI decision. X3 and X4 give the first slice that works without a model. Before them the shared
foundation must exist: F0, F1, F2, F3, F5, F9, F10 and F11 for X3, and also F6 for X4. Order of work, as in the overview's revised build
order (section 6): the library (F10), the user's own sources (F13a) and the PDF text core (F14a)
land inside the first two months; F11, F12 and X1 follow in weeks 9 to 11; then X3 and X4 (first
screen at about week 11, a planning estimate; a fifth worker brings it back to about week 9);
then X6 to X8; then X9 to X13. X14 to X19 follow, with the search-recall half of X14 as soon as X7
is done. The cited summary (F15's Explore adapter) comes after X11.

| Id | Title | What | Files | Size | Depends on |
|---|---|---|---|---|---|
| X0 | (now F0) Pin hashes, split routers | Nothing is left in Explore. The golden test, the move of the router builders to `routers.rs` and the 600-line CI check are F0. | none | - | - |
| **X1** | **Terms, queries, rank** | Pure code: TeX-aware term and id extraction; first-query builder per service; reciprocal-rank fusion and dedupe (on F2's `PaperKey` and alias merge, which are no longer part of this package). Fixture tests. | `idea_terms.rs`, `idea_queries.rs`, `idea_rank.rs` | S | F2 |
| X2 | (now F3 and F11) Services, abstracts, rate gate | Nothing is left as a package. The query runners, abstracts, citation relations and `literature.fetched` with `qid` are F11; the process-wide rate gate is F3. The one Explore-specific item, writing `idea.candidates` from merged results, moves to X3. | none | - | - |
| **X3** | **Session, stage machine, view, routes** | `idea.started` as host-written marker (mode detection itself is F5), `next_stage`, budget caps on top of F9's model-call count, `idea.candidates` from F11 results merged by X1, `idea_view` (map part in the `GraphView` shape), preview and create routes, user-shown queries (P4), interrupted-stage handling, stage states mapped to the shared run-state words. | `idea.rs`, `idea_view.rs`, `routes_idea.rs`, `http_srv.rs` (two arms, through the foundation lane), `desktop-server/tests/idea_flow.rs` | M | X1, F1, F5, F9, F10, F11 |
| **X4** | **First screen** | Content of the Explore card on F5's home screen (form with live preview), session page (idea card, stage rail, cost line on the shared run bar), candidate list with highlights, "Where I looked" skeleton. First shippable slice: no model needed. | `idea.js`, `idea_state.js`, `idea_setup.js`, `idea_search.js`, `idea.css`, `api.js`, `tests/idea_state.test.mjs` | M | X3, F5, F6 |
| X5 | (now F10) Library store | Nothing is left in Explore. Explore writes `seen` roles candidate, triaged, read and `finding` rows through F10. | none | - | - |
| X6 | Exchange runner, budget, Sharpen | `agent::exchange` (one model call, validate, up to 2 repairs, `idea.exchange` audit, budget callback into F9); `record_idea_brief`; "Sharpened idea" card with editable queries and answers. | `agent/src/exchange.rs`, `idea_brief.rs`, `idea_exchanges.rs`, `idea_setup.js` | M | X3, F9 |
| X7 | Plan execution, approval | `idea_search` tool in profile `explore/1`, called by the app through the Router; one batched approval on the shared approval card, which lists each grant it will record; calls into F12 for coverage, gate and wording (no longer built here); known-paper recall input; "Where I looked" panel. | `idea_tools.rs`, `routers.rs` (`build_idea_router`), `idea_exchanges.rs`, `idea_search.js` | M | X6, F1, F11, F12 |
| X8 | Triage and abstract evidence | `record_idea_triage`, abstract-quote location on F11's verbatim abstracts, `evidence.abstract`, grouped list, reading-list ticks, Quick preset path. | `idea_triage.rs`, `idea_exchanges.rs`, `idea_search.js` | M | X7, F4 |
| X9 | Reading queue | `idea_fetch_papers` as a thin tool over F3's `paper_fetch` (library first, which includes the user's own files and needs no network for them; then arXiv e-print; arXiv PDF last and one at a time; the fetcher itself is no longer built here); PDF text from F14, taken from the library when it was extracted before; `idea.paper` with `text_basis`; Attach PDF through the existing upload; row states. | `idea_tools.rs`, `idea_read.rs`, `idea_search.js` | S | X7, F3, F10 |
| X10 | Passage reader | Works the same on TeX and on page-broken PDF text (rule C8: page on the chip, "OCR" tag for scans). Passage pack builder, `record_paper_evidence` on the shared quote core (F4), `context.supplied` for every message with paper lines, `evidence.quote`, parallel readers up to the configured limit, live quotes. | `idea_evidence.rs`, `idea_read.rs`, `research-tools/tests/idea_evidence.rs` | L | X6, X9, F4 |
| X11 | Map, overlaps, verdict | `record_idea_map` (edge reading per kind), node rules, stable ids, `idea_verdict` on F12's status enum and gate, exclude-origin rule (V9), phrase check, map and verdict written at one head. | `idea_map.rs`, `idea_map_validate.rs`, `idea_verdict.rs`, `research-tools/tests/idea_map.rs` | L | X8, X10, F12 |
| X12 | Map and verdict UI | Verdict table with side-by-side quote, confirm control, grouped list and graph through the shared graph view (an adapter, no layout code of its own), quote chips and badges from F6, node drawer, flags, stale banner. | `idea_map.js`, `tests/idea_map.test.mjs` | M | X11, F6 |
| X13 | Directions and hand-over buttons | `record_idea_directions`, concreteness, cards, `solve_setup` with Solve's names; two buttons on top of F7: "Open in Solve" (opens Solve's setup card) and "Investigate this paper"; `handoff.sent`; reading the child's state back onto the card. The packet format and the receiving route are F7. | `idea_directions.rs`, `idea_handoff.rs`, `idea_directions.js` | M | X11, F7 |
| X14 | Evaluation cases for Explore | Set I of the shared evaluation set (overview section 7): ten ideas (3 done, 2 done under another name, 3 done partly, 2 open as far as known), frozen as recorded responses, in F8's manifest format, with answer keys (key papers and expected status per part) written by someone other than the builder. Explore's share of set C (chains): 2 from a paper node to an idea, 2 from an idea to a Solve problem. Measures: time to first content (candidates under 6 s), model calls against the cap, known-paper recall of the search alone per service set, "none in scope" shown for a part with a known earlier source (the false-negative rate that decides whether the mode is announced), "judged same" where the key says different, quotes accepted and rejected per model call, quotes surviving re-matching after a hand-over (must be 100 percent). Added by the scope revision: for 2 of the ideas a key paper is available only from the library (paywalled), and recall is recorded with and without the library; the Explore reader runs on the PDF-only papers of set P6; set C has 4 chains from a paper node to an idea, 2 of them restated results whose earlier source is known ("Search wider in Explore"), which is where Investigate's deferred prior-work check is now measured. The harness itself is F8. The search-recall half can start after X7. | `tests/fixtures/eval/` (set I), `desktop-server/tests/idea_eval.rs` | S | F8, X7, then X11 |
| X15 | Conversation turns | Explore system prompt, node context, the four tools of `explore/1` in chat, line-citation check for answers (logic of `investigation_cites.rs` for several papers; lines are credited through F4). The golden file and the earlier definitions are F1. | `idea_chat.rs`, `idea_system_prompt.rs` | M | X11, F1, F4 |
| X16 | Citation expansion, second round | References and citations of the key papers (relations from F11), in-network ranking, "cited by your core papers but unread", "Who cited this since", Thorough self-check round. | `idea_net.rs`, `idea.rs`, `idea_search.js` | M | X11, F11 |
| X17 | Library pane and export | Library view, search, marks, Save to project (on F10's routes), JSON and BibTeX export with coverage. A minimal library list and the sources panel are built earlier, with F13a (week 6). The readable output is the cited summary of F15 (`writeup_idea.rs`: per part the status sentence, quotes and citations, "what differs (model)", the coverage paragraph, the directions), built after X11; it replaces the Markdown export that was planned here. | `library.js`, `export/src/idea.rs` | M | F10, X11; F15 for the cited summary |
| X18 | CLI-subscription routes | Run exchanges as plain prompt-in, JSON-out calls through the Claude CLI and Antigravity routes, with the same validators. | `cli_turn.rs`, `cli_relay.rs`, `idea_exchanges.rs` | L | X6, X10 |
| X19 | Optional services | OpenAlex (new allowlist host, its own entry in F3's network gate, key in settings, threat-model note) and TheoremSearch (off by default, own consent text). | `lit_providers.rs`, `net_rate.rs`, `http_client.rs`, `lib.rs` | S each | F3, F11 |

---

## 10. Free core versus optional add-ons

| Free core (no paid service, no key) | Works better with a free key | Optional add-ons (never required) |
|---|---|---|
| Idea preview, library, first search on arXiv and INSPIRE-HEP, Crossref | Semantic Scholar (higher limits), OpenAlex (wider field coverage) | Hosted model access for users with none |
| Complete-search gate reachable with arXiv + INSPIRE-HEP alone | - | Larger citation databases, higher limits |
| All exchanges with the user's own model access (API key; CLI subscription after X18) | Cheaper model for readers | Heavy PDF parsing (Docling, GROBID) for PDF-only papers |
| arXiv full text (TeX first), user-attached PDFs, PDF-only and scanned papers with page-linked quotes (F14) | - | Open-access full-text fetch from publisher hosts |
| The user's own papers from a Kandinsky vault, a folder or a `.bib` file, kept in step (F13) | - | - |
| Concept map, verdicts, directions, cited summary (F15), export, library | - | Shared library of maps |

With no model at all the app still does the first search, shows abstracts and keeps the library.
The app says plainly which service was "not configured"; this never blocks a verdict.

---

## 11. What is borrowed from where

| Source | Licence | Idea or code | Used for |
|---|---|---|---|
| STORM / Co-STORM | MIT | idea | Search from several named perspectives; other_names required in code |
| dzhng/deep-research | MIT | idea, re-coded in Rust | Queries with goals, breadth and depth knobs; paid web search replaced by scholarly services |
| Ai2 Scholar QA | Apache-2.0 | idea | Quotes first, structure second, prose last |
| PaperQA2 | Apache-2.0 | idea | Evidence gathered per question from pre-retrieved passages (the passage reader) |
| OpenScholar | Apache-2.0 | idea | Self-check round listing thin statements and retrieving for each (Thorough) |
| OpenNovelty, UKPLab novelty assessment | Apache-2.0 | idea | Per-part labels with a required evidence snippet; role claims_novelty kept apart from the comparison |
| langextract | Apache-2.0 | idea | Align every extraction to offsets and flag what cannot be aligned (abstract quotes) |
| gpt-researcher | Apache-2.0 | idea | Visible, editable plan before execution |
| asta-paper-finder | Apache-2.0 | idea | Content criteria apart from metadata criteria (categories, dates) in queries |
| local-deep-research | MIT | idea | Found sources become a persistent searchable library |
| LocalCitationNetwork | GPL-3.0 | idea, re-coded in Rust | Seed papers, references and citations, rank by links inside the network (network calls must stay behind the allowlist) |
| Head Start | MIT | idea | Map and list kept in sync |
| arxiv.py | MIT | idea | Paging and 3-second spacing for the arXiv API |
| leanblueprint | Apache-2.0 | idea | Border style shows evidence grade |
| Gerby / Stacks Project | MIT | idea | Permanent node tags independent of numbering |
| Solver agent / re-Solve | AGPL-3.0 | lesson | Rules checked per call and returned as errors; prompt-only reminders are ignored |
| TheoremSearch | no licence | optional public service only, never its code | Off by default |
| Kandinsky (the owner's notes app) | AGPL-3.0, own | approach; code if the owner allows (overview 8.3, question 2) | The user's papers as files with a derived `refs.bib`; PDFs used in place; scan with a dry-run report; moved files recognised by hash; duplicates by DOI, arXiv id, file hash, ISBN and fuzzy title; citation keys kept; PDF text cached per file |
| re:Search itself | GPL-3.0 -> AGPL-3.0 | code | Ledger, Router, policy, approver, ingest, quote verification, outline, passage search, mode-marker and legacy-defs patterns |

---

## 12. Risks

1. **A false "not found".** Lexical search misses work phrased differently or not indexed. The design
   limits the claim, not the miss rate: scoped wording, required other-names perspective, citation
   expansion, known-paper recall, incomplete-search downgrade. Only the evaluation set (F8 with set
   I, X14) can measure the real rate; it should exist before the mode is announced.
2. **Thin evidence from passage reading.** The reader sees the passages that lexical search picks. A
   relevant section with different vocabulary can be missed, and quotes from TeX with macros can fail
   to match. Mitigations: aliases in the passage query, one or two follow-up line requests, true
   text returned on a near miss, honest "read: passages (k of N lines)" label, a "Read more of this
   paper" action. X14 tracks quotes accepted per model call of a reader.
3. **"Same result" is a model judgement across notations.** Shown as "Model judges: same" with the
   quote beside the idea sentence until the user confirms.
4. **Field coverage.** INSPIRE-HEP is strong in high-energy physics and partial elsewhere. For
   condensed matter or quantum information without keys the search rests on arXiv plus Crossref. The
   gate can pass while coverage is weaker than it looks; the per-service counts are always shown.
5. **Rate limits.** There is no rate limiter in the code today; F3 adds the one gate that all modes
   share, so an Investigate session fetching at the same time slows Explore down. arXiv at 1 request per 3 s makes an
   18-query plan take up to a minute and six full texts at least 18 s.
6. **PDF-only and old papers.** Now a normal path (F14), and often the papers that matter most for
   "has this been done": older work is where an idea is most likely to exist under another name.
   What remains weak: extraction with today's extractor was logged at 34 to 138 s per paper (F14a
   measures a second one, and the text is kept once per file); formulas in PDF text are poor and in
   OCR text useless, so a result stated mainly as a formula may be quoted only by its surrounding
   sentence; pages with no text at all cannot be read before own OCR exists (F14b).
7. **Privacy of an unpublished idea.** Terms go to public search services and the text to the user's
   model provider. Queries are shown before sending; an offline switch exists; TheoremSearch is off.
   Once "Allow for this session" is given, later model-written queries run without a prompt (existing
   grant behaviour); they are always listed on screen.
8. **Approval of app-run plans.** If the app called the services directly it would bypass the policy
   engine. Rule P5 and its test guard this. Whether the approver can prompt outside a running turn
   was not verified; the overview assigns this to F5 (it is open in all three designs), and X7
   cannot start before it is settled.
9. **Files near or over 600 lines.** `turn_runner.rs` 586, `routes_session.rs` 616, `http_srv.rs` 702,
   `investigation_anchor.rs` 597, `delegate.rs` 594, `paper_passages.rs` 605. F0 and F5 exist to make
   room, and one lane makes every edit to the first three.
10. **CLI-subscription users.** Exchanges need X18 there. Until then they get the deterministic part only.
11. **Well-formed but poor directions.** The concreteness gate checks form and references, not physics
    sense. The "check first" step and the Solve hand-off are where a direction gets tested.
12. **Open questions go stale.** Stated year and "later work checked" help; "cited since" is not "answered".
13. **Unverified assumptions here.** Semantic Scholar and INSPIRE field names for abstracts and
    citations are from general knowledge (now F11's concern); `SourceIndex` construction outside
    Investigation (now settled in F4) and the CLI plain-prompt path were not read in full; all
    timings are estimates.
14. **Waiting for the foundation.** Explore depends on nine shared packages before its first
    screen (F0, F1, F2, F3, F5, F6, F9, F10, F11). In the overview's revised plan the library, the
    user's own sources and the PDF text core come first and F11 follows in weeks 9 to 10, so the
    Explore screen is at about week 11 (planning estimate).
15. **The library is not the literature.** A hit in the user's own papers is strong evidence that
    something exists; no hit there says nothing. The coverage sentence keeps the two apart and the
    gate never counts the library as a service.
16. **Privacy of the library.** Collections, tags and notes of the user's library stay local. Only
    terms and identifiers go to search services, as before.
17. **Stale lessons.** A note kept from an old correction can be wrong for a new idea. Each use is
    shown, each note has a scope and a use count, and deleting one is one click.

---

## 13. Decisions for the owner

| # | Question | Recommended default |
|---|---|---|
| 1 | May the first queries (built from your own words and shown in the form) run on submit without a second approval prompt? | Yes: at most 6, exactly as shown, arXiv and INSPIRE-HEP only, with an "offline: library only" switch. All later queries need the approval card. |
| 2 | Pause after Sharpen and after Triage, or run through? | Run through in every preset, with the plan and the reading list editable while it runs and Stop always visible. A setting "pause before reading" for those who prefer it. |
| 3 | Default preset and caps? | Standard: 18 queries, 40 screened, 6 papers, hard cap 24 model calls. Quick one click away, Thorough opt-in. |
| 4 | Is a matched quote from an abstract enough to put something on the map? | Yes for related_work and concept nodes, drawn dashed and labelled "abstract only". Never for a known result, definition or open question, and never for a "found" verdict. |
| 5 | Does a service with no key block "not found in the searched scope"? | No. It is listed as "not configured". Only a service that was tried and failed makes the search incomplete. |
| 6 | Does "found (same)" need your confirmation? | Yes. It reads "Model judges: same" until you click "Looks the same to me". The shared status table (overview 2.9) applies the same to "found (partly)" and "found under another name". (Overview section 8, decision 9 asks whether Investigate adopts the same click.) |
| 7 | How much of a paper must be read to count as read? | Passage reading counts, labelled "read: passages (k of N lines)" and counted separately in coverage. "Read more" is one click. |
| 8 | Which services are on at first release? | arXiv, INSPIRE-HEP, Crossref, Semantic Scholar (all already on the allowlist). OpenAlex after sign-off on the new host (X19). TheoremSearch off until its terms are known. |
| 9 | Full text beyond arXiv? | Not fetched by the free core (keeps the allowlist closed). It comes from your own library (F13: decided, added) or you attach the PDF. A wider fetcher could be an add-on. |
| 10 | (Now a shared decision: overview section 8, decision 5, one library for all modes.) | What stays Explore's: projects keep their curated list and get "Save to project". |
| 11 | Should your marks (key paper, irrelevant) change later rankings? | Yes, visibly: "irrelevant" demotes but still lists; "key" is added as a seed when its terms match. Now part of the shared package F16 (decided, added), together with confirmed and rejected "same" judgements. |
| 12 | Explore on CLI-subscription routes in the first release? | Deterministic part only, with a clear message. X18 moves up if most of your own use is through a CLI subscription. |
| 13 | Revising the idea after a map exists? | Keep the old map marked stale; "Re-map against revision n" reuses all evidence at no reader cost. |
| 14 | Send a contact address to Crossref for its faster pool? | Off. A settings field you may fill in yourself. |
| 15 | (Now a shared decision: overview section 8, decision 2, the two-month priority.) | See the order of work in section 9. |
| 16 | (Settled by the overview, section 5.2, and section 8, decisions 10 and 11.) | Explore-only events, files and tools use `idea`; shared event kinds are `evidence.quote`, `evidence.abstract`, `context.supplied`, `handoff.packet`, `handoff.sent`; "leg" is renamed "exchange"; the mode is called "Explore" on screen; `delegate_explore` keeps its id and gets the label "Try strategies in parallel". |

Other shared decisions that bind this document and are not repeated here: overview section 8,
decision 1 (tool profiles; Explore is `explore/1`), 4 (dashed always means "not checked by code"),
6 (home screen with three cards), 7 ("New research" stays under "More" until X13 and S13 work), 8
(hand-overs re-match every quote and carry no badge) and 12 (who writes the answer keys of the
evaluation set). Decided on 2026-11-25 (overview 8.1): "verified" is shown nowhere and quotes are
"matched"; bring your own papers; PDF-only and old papers essential; write-up; corrections. New
questions that touch Explore (overview 8.3): 1 (the revised order), 3 (a `judged` status on OCR
text), 5 and 6 (sync direction and first sources), 7 (lessons).

---

## Appendix A. What the review found

Facts checked in the code:

| Disputed point | Finding |
|---|---|
| Ledger prefix `explore.` | `crates/agent/src/explore.rs` emits `explore.start`, `explore.branch`, `explore.end`; `honesty.rs:273`, `resume.rs:331`, `session_view.rs:502` and `session_summary.rs:138` match on them. The researcher proposal's `explore.*` events and `src/explore/` folder would share that namespace. Use `idea.*`. |
| Adding tools breaks resume | Confirmed (`resume.rs:191-245`): ordered comparison, tolerated additions are a fixed id list with effect ReadWorkspace only. |
| User-typed search needs no approval | Confirmed (`routes_tools.rs:374`, `handle_literature` calls the search functions directly). But it covers strings the user typed, not app-built ones; hence rule P4. |
| Session network grant | Keyed by (session, effect) in `session_lifecycle.rs`: one "allow for session" covers every later network read in that session. |
| Abstracts from INSPIRE and Semantic Scholar | Not available today: `LiteratureHit` has no abstract and the Semantic Scholar request omits it. Only arXiv results carry `abstract_text`. |
| FTS5 | The bundled SQLite build script (`libsqlite3-sys` 0.30.1, `build.rs:129`) sets `SQLITE_ENABLE_FTS5`. `rusqlite` is a dependency of the ledger crate only. |
| "Thin wrapper over the worker loop" | The worker loop is inline in `run_delegation_call` (`delegate.rs:389-594`); there is nothing to wrap. The exchange runner is new code on the `Transport` trait. |
| Reader reuse | `paper_read.rs` hard-wires `note_evidence` and the caller prefix; `find_active_marker` requires Investigation markers. Parameterising them risks Investigation. |
| Rate limiting | None exists for any literature service today. |
| Network allowlist | Five hosts: export.arxiv.org, arxiv.org, inspirehep.net, api.crossref.org, api.semanticscholar.org. OpenAlex is not on it. |

Rejected or corrected from the researcher-first proposal: the `explore.*` prefix; a reserved ledger
session named "library" (it would surface in every session list and per-session code path); abstract
evidence allowed under known results "unless marked so"; `delegate_read_papers` as an agent-side tool
inside an app-driven pipeline (uncounted orchestration calls); refactoring `literature.rs` internals;
a mode-aware change to `resume.rs` up front; model-written queries running under the first grant with
no plan card; "search incomplete" whenever a keyless service is skipped.

Rejected or corrected from the architecture-first proposal: about 50 model calls as the Standard
default; abstracts usable for nothing (Quick preset would have no map and paywalled papers no
presence); a deny-list on the single words "new" and "first" (it rejects "first calculation"); a
mandatory obstruction perspective; node ids hashed from the first evidence (collide when two nodes
share a quote, change when evidence is reordered); refusing directions outright instead of ranking
them as sketches; pausing by default after Sharpen; the "=" plus numeral guard on expected_form;
recorders as registry tools on a router (unneeded, and it blocks the CLI path).

Two points of this appendix are superseded by the overview. First, "a mode-aware change to
`resume.rs` up front" was rejected here as an Explore-only edit; it is now made once for all modes
as tool profiles (F1), before any mode adds a tool. Second, the module names with the `idea_` prefix
that the review settled on are kept only for Explore-only code; code that all modes use has the
neutral names listed in section 7.
