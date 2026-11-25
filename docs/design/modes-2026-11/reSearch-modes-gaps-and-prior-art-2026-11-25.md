# re:Search: three modes, what is missing, and what to borrow

Written 2026-11-25. Based on a read of the code on branch `gemini/work/merged` (nothing was run) and a
search for open-source prior art. Every repository below was confirmed to exist and its licence was
checked on GitHub that day. What each one contains comes from its README, not from running it.

## 1. The three modes

Research mode is split in two, so each mode can be designed for one job.

| Mode | You give it | It gives back | Closest thing today |
|---|---|---|---|
| **Investigate** | A paper | A map of the paper that you explore with the AI, one node or all of it | Investigation mode |
| **Explore** | An idea, written down | Whether it has been done, what has been done, and possible ways to pursue it | Nothing: Research mode treats it as a proof |
| **Solve** | A calculation or proof, set up | The worked result, with each step checked | Research mode's calculation side |

Principles that apply to all three:

- **The core is free.** Every mode must do its job with no paid service. Paid services are optional add-ons.
- **Nothing is cited that was not read.** The existing rule (verified quotes, hash-chained ledger) stays.
- **Show something at once.** Anything that can be computed without a model is shown before the model starts.
- **The modes hand over to each other.** A node in Investigate can open Explore ("has this been extended?")
  or Solve ("derive equation 14"). An Explore direction can open Solve. Today the modes share ingestion but
  not the map.

## 2. Investigate

Goal: dissect a paper so it is easier to digest. Map it, explore the map progressively, fill in missing
context, save time, and say up front which results are new.

### Where it stands

| Goal | State | Detail |
|---|---|---|
| Break the paper into a map | Partial | The map is a graph of claim sentences (10 node kinds, 3 edge kinds), not a map of the paper. A section, equation and figure outline (`build_outline`) is computed on every turn and never shown. No theorem, lemma, proposition, proof, example or remark kinds. |
| Explore progressively | Strongest part | Overview, then a result, then a focused branch, then a statement, then check or hand-off. But only main results can be developed, so depth stops at two levels. No whole-map view. Re-running "Find main results" orphans all branches. |
| Fill in context | Weak | Only the paper's own conventions catalog, off by default, and forbidden from adding anything the paper does not say. No prerequisites per node. Cited results are named but never looked up. One paper per session. |
| Save time | Partial | First screen costs PDF extraction (34 to 138 s in the logged run) plus a model turn (summary about 85 s, map about 180 s). Only a timer is visible meanwhile. Nothing says what can be skipped. Parallel readers are off unless configured. |
| New versus known | Missing | No field, prompt, tool or badge. |
| Keep, share, export | Partial | Maps persist and reopen. No export (Markdown, JSON, HTML), no sharing, no import. |
| Input | Partial | No "paste an arXiv ID or DOI". No OCR for scanned papers. 10 MiB cap. |

### What to build

1. **Two-layer map.**
   - Layer 1, the skeleton: sections, definitions, theorems, equations, figures and who refers to whom.
     Read straight from the TeX source or arXiv HTML with no model, shown within seconds.
   - Layer 2, the interpretation: what the paper contributes and what each part is for. Built by the
     model, grounded in verified quotes, attached to skeleton nodes.
2. **Node types for mathematics and physics:** theorem, lemma, proposition, corollary, definition, proof,
   example, remark, plus the existing kinds. Separate "the statement uses X" from "the proof uses X".
3. **Open any node, to any depth.** Remove the main-results-only limit. Add a whole-map view.
4. **Stable node identifiers**, so notes and branches survive a re-run and a new arXiv version.
5. **New, restated or background, per result.** Two separate labels shown side by side:
   what the authors claim is new, and what a comparison with prior work supports.
6. **Context card per node:** prerequisites, the nearest definition of each symbol, and for a cited result
   what the cited paper actually states, with its hypotheses. Clearly labelled as coming from outside the paper.
7. **"What can I skip":** length and reading time per section, and which sections no main result depends on.
8. **arXiv ID entry, TeX source first, PDF as fallback.**
9. **Export and share a map.**

### What to borrow

| Project | Licence | Use | Borrow |
|---|---|---|---|
| [KnowTeX](https://github.com/ElifUskuplu/KnowTex) | Apache-2.0 | Port | Eight inference rules that turn `\ref`, `\eqref` and "Proof of Theorem ..." into dependency edges. Small enough to port to Rust. |
| [texlab](https://github.com/latex-lsp/texlab) | GPL-3.0 | Include code | Its `syntax`, `parser` and `symbols` crates: sections, environments, labels, references. Does not expand macros. |
| [tree-sitter-latex](https://github.com/latex-lsp/tree-sitter-latex) | MIT | Include code | Lighter alternative to texlab's parser. |
| [latexml-oxide](https://github.com/dginev/latexml-oxide) | CC0-1.0 | Run as a tool | Rust port of LaTeXML with full macro expansion. Beta, one maintainer, needs a TeX installation. |
| [ar5iv](https://github.com/dginev/ar5iv) and arXiv HTML | MIT | Call | HTML that already marks sections, theorems and equations. Not every paper converts. |
| [arxiv.py](https://github.com/lukasschwab/arxiv.py) | MIT | Idea | Paging and backoff for the arXiv API (one request per three seconds). |
| [arxiv-to-prompt](https://github.com/takashiishida/arxiv-to-prompt) | MIT | Idea | Main-file detection and `\input` flattening while keeping a line-origin map. |
| [unarXive](https://github.com/IllDepence/unarXive) | MIT (code) | Idea | Record shape for paragraphs with citation and formula markers. |
| [pdf.js](https://github.com/mozilla/pdf.js) | Apache-2.0 | Include code | Text with coordinates and the PDF's own bookmark outline, for papers without TeX. |
| [Docling](https://github.com/docling-project/docling) | MIT | Optional tool | Structured PDF parsing. Heavy (PyTorch, model downloads): add-on. |
| [GROBID](https://github.com/grobidOrg/grobid) | Apache-2.0 | Optional tool | Reference parsing resolved to DOIs. Needs a JVM: add-on. |
| [leanblueprint](https://github.com/PatrickMassot/leanblueprint) | Apache-2.0 | Idea | Node schema and status colours; "uses" on statement versus on proof. |
| [NodeGraph](https://github.com/Jeong-jin-Han/NodeGraph) | MIT | Idea | Prompt design for finding the 4 to 8 contributions only this paper makes. |
| [Gerby](https://github.com/gerby-project/gerby-website) (Stacks Project) | MIT | Idea | Permanent tags per node, independent of numbering. |
| [ScholarPhi](https://github.com/allenai/scholarphi) | Apache-2.0 | Idea | Click a symbol, see the nearest definition above it; fade what is irrelevant. |
| [OpenNovelty](https://github.com/january-blue/OpenNovelty) | Apache-2.0 | Idea | Phased novelty check with per-claim labels and a required evidence snippet. |
| [UKPLab novelty assessment](https://github.com/UKPLab/eacl2026-assessing-paper-novelty) | Apache-2.0 | Idea | Separate the authors' novelty claims from the comparison; use the paper's own citations as the first pool. |
| [TheoremSearch](https://github.com/uw-math-ai/TheoremSearch) | No licence | Optional service | Search over millions of theorem statements to spot restated results. No terms or limits published. |
| [PaperQA2](https://github.com/Future-House/paper-qa) | Apache-2.0 | Idea | "Gather evidence": score and summarise chunks of the cited paper against the citing sentence. |
| [Ai2 Scholar QA](https://github.com/allenai/ai2-scholarqa-lib) | Apache-2.0 | Idea | Quotes first, outline second, prose last, for background notes. |
| Semantic Scholar API | Service | Call | Per reference: abstract, short summary, citation context and intent in one call. Free key recommended. |
| [INSPIRE-HEP API](https://github.com/inspirehep/rest-api-doc) | Service | Call | Physics references and abstracts, no key. 15 requests per 5 seconds. |
| OpenAlex, OpenCitations, Crossref | Services | Call | Fallbacks. OpenAlex expects a free key since February 2026. |
| [SciCite](https://github.com/allenai/scicite) | Apache-2.0 | Idea | Citation labels (background, method, result); add "source of a restated result". |

**No prior art found:** labelling each result inside one paper as new, restated or folklore; novelty
assessment tuned for theoretical physics or mathematics; typed theorem and proof extraction with
proof-to-statement linking; dependency graphs for PDF-only papers; ready-made prerequisite retrieval.
These would be original work.

## 3. Explore

Goal: you write down an idea. It finds out whether it has been done, what has been done, and offers
possible ways to pursue it.

### Where it stands

Nothing is designed for this. Research mode has one prompt and one pipeline, so an idea is treated as a
claim to be "rigorously solved" and sent to the verifier. Sources are limited to arXiv, INSPIRE-HEP,
Crossref and Semantic Scholar metadata, plus arXiv TeX source and attached files; there is no general web
access. There is no concept map: ledger entry types are proof-shaped. `delegate_explore` sounds like this
mode but is strategy testing for proofs.

### What to build

1. **Its own pipeline and prompt**, with no proof duty and no "rigorously solves" verdict.
2. **Sharpen the idea first:** restate it, list its assumptions, ask one or two clarifying questions.
3. **Has it been done?** Search from several perspectives, read what is found, and report with quotes:
   done exactly, done partly, done under other names, or not found. "Not found" states where it looked.
4. **What has been done:** a concept map whose nodes are concepts, definitions, known results and open
   questions, each linked to its sources. A local citation network of the key papers.
5. **Ways to pursue it:** a short list of directions, each with what is known, what is missing, what a
   first calculation would be, and a button that opens it in Solve.
6. **A library that grows:** papers found while exploring are kept and reused in later sessions.
7. **New ledger entry types:** concept, definition, known_result, open_question, related_work, direction.

### What to borrow

| Project | Licence | Use | Borrow |
|---|---|---|---|
| [STORM / Co-STORM](https://github.com/stanford-oval/storm) | MIT | Idea | Questions from several perspectives; a mind map built during the conversation. |
| [Ai2 Scholar QA](https://github.com/allenai/ai2-scholarqa-lib) | Apache-2.0 | Idea | Quote extraction, then clustering, then writing. Fits the citation rules. |
| [PaperQA2](https://github.com/Future-House/paper-qa) | Apache-2.0 | Idea | Evidence summaries scored per question; contradiction detection. |
| [dzhng/deep-research](https://github.com/dzhng/deep-research) | MIT | Port | The whole loop is small: queries with goals, learnings, breadth and depth. Replace its paid search with scholarly APIs. |
| [gpt-researcher](https://github.com/assafelovic/gpt-researcher) | Apache-2.0 | Idea | Planner and executor split; a visible tree of sub-questions. |
| [local-deep-research](https://github.com/LearningCircuit/local-deep-research) | MIT | Idea | Found sources become a persistent, searchable library. |
| [asta-paper-finder](https://github.com/allenai/asta-paper-finder) | Apache-2.0 | Idea | Split a query into content and metadata criteria; route to different search plans. Unmaintained. |
| [OpenScholar](https://github.com/AkariAsai/OpenScholar) | Apache-2.0 | Idea | Self-feedback pass: list unsupported claims, retrieve for each, revise. |
| [LocalCitationNetwork](https://github.com/LocalCitationNetwork/LocalCitationNetwork.github.io) | GPL-3.0 | Include code | Plain JavaScript: seed papers, fetch references and citations, rank by links within the network. Its OpenAlex calls need updating. |
| [Head Start](https://github.com/OpenKnowledgeMaps/Headstart) | MIT | Idea | Topic bubbles with a synchronised list. |
| [arxiv-mcp-server](https://github.com/blazickjp/arxiv-mcp-server) | Apache-2.0 | Idea | Tool surface: get outline, read section, get citations. |
| [zotero-mcp](https://github.com/54yyyu/zotero-mcp) | MIT | Idea | Talk to the running Zotero app over its local HTTP API. |
| [langextract](https://github.com/google/langextract) | Apache-2.0 | Idea | Align every extraction to character offsets; flag what cannot be aligned. |
| The novelty projects and APIs in section 2 | | | "Has it been done" is the same machinery as "new versus known". |

**No prior art found:** concept maps for an idea beyond Co-STORM; idea exploration tuned for mathematics
or theoretical physics; an agent light enough to embed in a Rust desktop app (all are Python or TypeScript).

## 4. Solve

Goal: you set up a calculation or a proof and it carries it out. This is very similar to Solver agent
(`re-Solve`, the author's fork of `starrfree/solver-agent`, AGPL-3.0), which was read for the comparison
below. Nothing in either repository was built or run for it.

### Compared with Solver agent

**Recommendation: Solve mode is re:Search's engine with Solver agent's ideas.** Do not embed Solver agent
and do not keep two apps. re:Search already has its own versions of most of Solver agent's design (proof
graph, completion gate, write-ahead verification markers, strategist, pinned conventions, sanity harness,
TeX audit, `solver_calc`, side-talk, narrator, fork, diff, export). It is stronger where Solver agent is
weakest: a real Windows sandbox, a hash-chained ledger, and no Node, MongoDB or WSL to install. Solver
agent is TypeScript and re:Search is Rust, so everything is re-coded anyway.

| Area | Better today | Why |
|---|---|---|
| Step record and dependency graph | re:Search | Hash-chained SQLite ledger with sequence numbers; Solver agent orders by millisecond timestamps in MongoDB. |
| Python sandbox | re:Search | Per-session Windows AppContainer; Solver agent's own comments say its wrapper is not filesystem isolation. |
| Verifier | Solver agent | Its step and full verifiers run a 12-turn loop with compute, sanity-check and ledger-read tools, as separately routed model roles. |
| "Interpret every computation" gate | Solver agent | Enforced on every tool call, so the model is corrected at once; re:Search checks only at the end of a leg. |
| Compute workers | Solver agent, narrowly | A "success" with no zero-exit run is downgraded; the orchestrator may not state the expected result. |
| Expression passing between steps | Solver agent | Both ship `solver_calc`, but only Solver agent's host harvests named exact expressions and stages them into later runs and verifiers. |
| Lean | Solver agent, slightly | `#print axioms` audit and three honest states (verified, unchecked draft, rejected). Neither bundles Lean. |
| Final-answer gate, strategist, conventions, masking, fork, export | Tie | Same designs. re:Search also checks the delivered answer text against what the verifier saw. |

**What to take from Solver agent, in order:**

1. Verifier with tools, on a different model by default; label "same model" as weaker when it is not. (medium)
2. Move the computation-interpretation gate to each tool call, returning failures to the model as tool errors. (small)
3. Per-step verification: let the solver call `verify_step`, require it on every result entry, write the
   verdict onto the step, and compute each step's badge from its ancestors. Solver agent leaves this optional
   and leaves no mark on an accepted step, so re:Search should go further. (medium)
4. Hitting the turn cap pauses with "Continue" instead of aborting; rebuild history deterministically from
   the ledger and give the verifier only the final answer's dependency closure. (medium)
5. Setup dialog: statement, kind of task (calculate, prove, check a given derivation), target, conventions,
   sources, per-role models. Solver agent has no "kind of task" or "what counts as done" field. (small)
6. Wire the expression bus into the host; add execution receipts and the "never state the expected result"
   rule. (medium)
7. Heavier compute tier with SciPy, plots and an artifacts panel; then optional engines. (medium to large)
8. Lean axiom audit and result states now; bundling as an optional download later. (small now)
9. Notebook export. Neither app has it.

**Lessons from Solver agent's own change log:** text-only advisers (critic, simplifier, context
synthesiser, subject specialists) were tried and removed; prompt-only reminders were ignored (4 strategist
calls in 871), so rules must be enforced in code.

**Ownership:** git history cannot separate upstream from the fork's additions (all commits are the fork
owner's, and the first already contains upstream). The standalone files the change log describes as added
(`solver_calc.py`, the expression bus, the sanity tool, the TeX audit, the Lean plugin) are the fork
owner's. The agent loop, verifier agents and prompts are upstream. If re:Search moves to AGPL-3.0 (draft
pull requests were opened on 2026-11-25), upstream code may also be copied, not only its ideas.

### Where it stands

The strongest code in the app: a sandboxed SymPy, NumPy and mpmath worker, validation helpers
(`check_symbolic_identity`, `check_convergence`), a typed proof graph with per-step status, fork from a
step, and export to TeX, HTML and a reproducible ZIP.

| Weakness | Detail |
|---|---|
| "Verified" is a second opinion, not a check | The verifier has no tools and 8,192 tokens. It cannot re-run a computation or test a counterexample. With no verifier model set, it is the same model as the solver. |
| Only the final answer is reviewed | After a verified run nearly every step is still "pending", so the graph cannot show which steps to trust. |
| Dependency edges can be manufactured | When the model links no evidence, the final answer is made to depend on every entry. |
| Lean is not there | Not bundled or installed: a fresh install returns `lean_not_found`. The cheat check scans text for `sorry` instead of `#print axioms`. |
| Thin toolbox | No SciPy, no plots, no tensor or field-theory algebra, no access to the user's data files. |
| Long work is cut off | 48 turns, then the session aborts. No checkpoint and continue, no summarising. Workers get 8 turns. |
| No notebook export | The ledger already holds every code cell and output. |
| CLI routes are weaker | Claude CLI and Antigravity routes skip the verifier, strategist and delegation. |
| Windows only | No compute worker elsewhere. |

### What to build

1. **A setup step:** state the problem, conventions, assumptions and what counts as done, before any work.
2. **Two labelled tiers of checking.**
   - Tier 1, computed: each step re-derived by computer algebra and tested numerically, plus named physics
     checks (dimensions, limits, conservation laws, symmetries).
   - Tier 2, formal: Lean, as an optional download.
   A step's badge says which tier passed. "A model agreed" is a third, weaker label.
3. **Check every step, and compute status from ancestors:** proved but resting on something unproved.
4. **A verifier with tools** that can re-run code and try counterexamples, on a different model by default.
5. **Expression handles:** the model creates an expression once and passes its key, so nothing is retyped.
6. **More engines:** SciPy and plotting in the worker; Cadabra2 and FORM as optional engines.
7. **Long runs:** checkpoint, summarise, continue in the background.
8. **Notebook export** and reading the user's data files.

### What to borrow

| Project | Licence | Use | Borrow |
|---|---|---|---|
| [SymPy](https://github.com/sympy/sympy) and mpmath | BSD-3-Clause | In the worker | Three-rung equality check: simplify to zero, then numeric at random points, then series comparison. |
| [Get Physics Done](https://github.com/psi-oss/get-physics-done) | Claimed Apache-2.0; GitHub does not confirm | Idea | A registry of 24 named checks (dimensional analysis, limiting cases, conservation laws, Ward identities). Read its licence file before taking more than the idea. |
| [sympy-mcp](https://github.com/sdiehl/sympy-mcp) | Apache-2.0 | Port | Expression keys and assumptions attached when a variable is created. |
| [Lean 4 REPL](https://github.com/leanprover-community/repl) | Apache-2.0 | Run as a tool | JSON over a child process, no Python. Import Mathlib once and reuse the state. Several GB: optional download. |
| [lean-lsp-mcp](https://github.com/oOo0oOo/lean-lsp-mcp) | MIT | Idea | Tool list for the model: goal at position, diagnostics, try several tactics. |
| [PyPantograph](https://github.com/stanford-centaur/PyPantograph) | Apache-2.0 | Idea | A proof as a tree of goal states; the same graph component could draw it. |
| [physlib](https://github.com/leanprover-community/physlib) | Apache-2.0 | Optional dependency | Physics definitions in Lean. Coverage is thin. |
| [leanblueprint](https://github.com/PatrickMassot/leanblueprint) | Apache-2.0 | Idea | Status computed from a node and its ancestors. |
| [Cadabra2](https://github.com/kpeeters/cadabra2) | GPL-3.0 | Optional engine | Tensor and field-theory algebra with LaTeX-like input. |
| [FORM](https://github.com/form-dev/form) | GPL-3.0 | Optional engine | Heavy perturbative calculations. Check for a native Windows build. |
| [Mathics3](https://github.com/Mathics3/mathics-core) | GPL-3.0 | Optional engine | Runs simple Mathematica snippets without a licence. |

**No prior art found:** a standalone checker that verifies a written LaTeX derivation step by step;
reliable LaTeX-to-algebra parsing for physics notation; autoformalisation of physics good enough to trust
unattended; open prover models with licences that allow bundling; a free hosted Lean checker.

## 5. Free core and paid add-ons

| Free core | Optional add-ons |
|---|---|
| Skeleton map from TeX and arXiv HTML | Heavy PDF parsing for papers without TeX (Docling, GROBID) |
| Model-built map, branches, questions (user's own model access) | Hosted model access for users with none |
| Novelty and context from INSPIRE-HEP, arXiv, Crossref, OpenCitations | Larger citation databases; higher API limits |
| Computed checks with SymPy and mpmath | Hosted Lean and Mathlib checking; Cadabra2 and FORM bundles |
| Export and sharing of maps | A shared library of maps for well-known papers |

Some free services expect a free key per user (Semantic Scholar, OpenAlex).

## 6. Suggested order

1. **Investigate:** show the outline at once and attach claim nodes to sections.
2. **Investigate:** new versus known, first from the authors' own claims, then checked against prior work.
3. **Investigate:** per-node context cards and cited-result lookup.
4. **Solve:** computed checks on every step and a verifier with tools.
5. **Explore:** the new pipeline. It reuses the search and novelty machinery from steps 2 and 3.
6. **Solve:** Lean and the physics engines as optional downloads.

Before any of this, an evaluation set of real papers and problems would show whether each step helps.

## 7. Licence notes

- re:Search is GPL-3.0-only. Code under MIT, BSD, Apache-2.0, CC0 and GPL-3.0 can be included.
- "Idea" means reimplement the approach; no code is copied, so the licence does not constrain it.
- TheoremSearch has no licence: use its public service only, never its code.
- pandoc is reported by GitHub as GPL-2.0; run it as a separate program only, or avoid it.
- AI-Scientist's novelty check has a custom licence that is not compatible; it is not listed above.
