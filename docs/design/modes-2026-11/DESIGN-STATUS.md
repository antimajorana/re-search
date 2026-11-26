# re:Search three-mode design and implementation status

**Snapshot** 26 November 2026. Repository `antimajorana/reSearch`, integration branch `gemini/work/fixed`, verified at `35ceb1d524be50c8bed721a13b41046daa4c728a` (PR 28). Refresh the remote refs before acting. The detailed specification and package dependencies live in `docs/design/modes-2026-11/00-overview.md`, `investigate.md`, `explore.md`, and `solve.md`. The companion takeover prompt contains the local Gemini access details.

## Product intent

re:Search is a Windows-first research workbench for mathematics and physics. The researcher chooses the question, controls the models and budgets, inspects what the models and tools actually produced, and can redirect work without losing the record. Investigate has first priority. Explore and Solve can advance independently as their foundations land. The old Research experience and its saved sessions remain supported through the established compatibility path.

### Investigate a paper

Start from an arXiv ID, DOI, TeX bundle, PDF, or library entry. Resolve locally first and use the shared fetcher for a named request. Parse the source into a pinned, deterministic skeleton with sections, results, equations, bibliography, source spans and dependency edges. The first usable view needs zero model calls. A PDF uses pinned page-broken text and physical page links. Show the quality of a PDF-derived outline, preserve scan provenance, and use the page image for OCR formulas.

Begin with the outline and the main results. Orient can identify candidate main results, and the researcher can pin results that matter. Selecting a result shows its exact statement and source, **Statement uses**, **Proof uses**, and **Used by**. Arrows point from prerequisites to dependents. A focused view follows incoming and outgoing edges, and a whole-paper view remains available. Open only the selected proof branch and stream supported steps into it. Keep deterministic evidence and earlier accepted steps visible during model work, pause or retry.

Follow a named citation to the exact statement and locator in the cited paper. Reuse the library and pinned skeleton, render candidate pages when the locator is ambiguous, and let the researcher open the cited node in its own Investigate view and return to the original selection. An unavailable source leaves the bibliographic reference and a way to supply the file. The acceptance walkthrough uses two original fixtures with a statement prerequisite, a proof prerequisite, a downstream claim, one expanded proof child, and a round trip across papers.

### Explore an idea

Decompose an idea into concrete subclaims and search perspectives. Show or refine the initial queries, search through the shared literature services, merge/rank candidates, triage abstracts, read applicable full text or passages, and map overlaps and promising directions. Output records the evidence and the scope searched. A verbatim abstract can support **Found in abstract** and possible relevance. It cannot support a full-text overlap judgment. The private library is reported in coverage and does not count as an external literature service.

The coverage gate conditions any negative finding on actual answered services, answered exact and alternative-name perspectives, sufficient screening, disposition of core papers, and seed coverage. Missing evidence stays incomplete. Use host-generated scoped wording. Model judgments stay attributable, and the originating paper in an Investigate handoff cannot support its own novelty result.

### Solve a derivation

Start with a calculation, proof, or derivation check. Pin the task, target, assumptions, conventions and sources. Record typed steps and dependencies in the ledger. Pass versioned expression handles between bounded workers and the independent check process. Keep successful computations tied to the result that interprets them. The check process runs exact and numerical checks through the existing sandbox, with separate model review where useful. Save work on pause and continue from a checkpoint. Finish with either a supported derivation or named open items.

The reviewed Solver agent inspiration is the tool-capable verifier, the per-call interpretation gate and the reusable named expressions with real code/output. Implement these in re:Search’s existing Rust/Python engine. Use the reference fork at `antimajorana/re-Solve` commit `5d68b0115a42f294b9ff066f2d2eb854e45c46ed` if needed. Tell the owner before opening other repositories for inspiration. The owner confirmed that Kandinsky is their repo and its approved code may be merged into re:Search.

### Evidence and control

| Label or display | Requirement |
| --- | --- |
| **Found** | A matched passage in an accessible source with its locator. An abstract-only find is labeled as such. |
| **Verified** | A successful exact symbolic check on the current claim and assumptions with an inspectable computation receipt. |
| **Numerically checked** | An actual numeric comparison against the claim. Show the domain, samples or seed, precision, tolerance and residual or counterexample. Missing comparison evidence cannot earn this label. |
| **Review verdict** | Record what a verifier judged, with its model and independence. Agreement alone earns a review verdict. |
| **Inconclusive**, **Not run**, **Failed**, **Stale** | Distinguish timeout or undecidable calculation, skipped check, counterexample and changed claim/assumptions. Keep old receipts accessible. |

Show the step’s own check and its dependency-limited effective status together when they differ. A step can say **Own check: Verified · Rests on: Lemma 2, unchecked** and link to the dependency. The receipt opens the exact expressions, assumptions/domain, engine/version, output and rerun action. A rerun creates a fresh record. The worker view shows its assigned task, model, inputs, actual returned records and artifacts, tools, usage, and acceptance or rejection reasons alongside the evidence. The researcher can pin or dismiss a proposal, adjust focus, pause the next stage, stop or continue, and choose assignments and budgets for a subsequent run.

### Frontier Orchestrator, smaller workers and code

The user chooses the effective frontier Orchestrator. It plans, assesses evidence, decides the reasoning route and synthesizes. Smaller models handle bounded passage location, extraction, triage, and calculation preparation or retries. Deterministic code takes source-only work, quote matching, canonicalization, symbolic and numeric checks, coverage and badge decisions. Prefer an explicit worker assignment, then a compatible smaller model on the connected provider. Show the effective model and avoid silent frontier fallback for a worker. Reuse the existing provider roles, transport, sandbox and tool policies.

Workers receive scoped inputs and return compact evidence and receipts. Batch the relevant evidence for the Orchestrator rather than replaying worker histories. F4 credits the actual citing caller for the passages it saw. F9/F9b reserve shared call capacity before dispatch, account once per provider request, and show model usage by role. Missing tokens stay unknown. Measure frontier and total tokens, latency, and evidence quality on paired held-out runs before claiming savings.

## Implementation status

| Area | Merged or verified | Pending |
| --- | --- | --- |
| Shared safety and data | F0 guardrails, F1 tool profiles, F2 paper identifiers, F3 shared network gate/fetch, F4 quote core, F5 mode shell (PR 27, Windows browser verified), F6 graph UI components, F8 evaluation harness, F9 run counter, F10 library storage, F14a PDF text core | F7 handoffs, F9b routing/activity, F12 coverage, F13 sources and write-back, F14b OCR and F14c transcription, F15 write-up, F16 corrections/lessons |
| Investigate | V1 TeX scanner; V2 dependencies, numbering, stable tags, statistics and bibliography in PR 26; V3 deterministic first screen, HTTP dispatcher and zero-model-call TeX entry verified on Windows. Duplicate TeX labels stay unresolved. | V4 paper opener pushed (`eafe8a1`), V5 skip plan/symbols underway. V17 PDF skeleton running after V2. V6 tasks unblocked by V3, then V7–V9. |
| Explore | F11 literature services and X1 terms, query building and ranking in PR 24. F11 citation replay and error handling fixes verified. | F12 running. X3 is unblocked after F5, followed by X4, then X6–X19 as gated in the plan. |
| Solve | S2 step model and badge fold, S3 router gate in PR 24, S6 contained check worker in PR 28. | Evidence wording and numeric-evidence fixes are assigned. S4/S5 and S7–S15 follow their dependency gates. |
| Local Windows baseline | 824 Rust tests passed, 2 ignored by the existing suite. Eight browser journeys passed with installed vendor CLIs isolated. PR 21 fixed the PDF preview fixture’s line anchors. | Re-run relevant suites on each new integrated candidate. GitHub’s release PR into `main` must pass Windows CI. |
| Real PDF probe | Seven user-supplied PDFs, zero model calls and all 31 sampled original-page links passed. The first 58 pages of the Mermin file were detected as scanned pages with a text layer. | This checks F14a extraction and links. V17 skeleton quality and the full P6 milestone evaluation remain open. Keep these PDFs and paper-text answer keys local. |

### Next checkpoints

1. **F5 merged in PR 27.** Linux desktop, mode, auth, math and resume checks passed, along with 163 JavaScript tests and all eight Windows headless Chromium journeys (with installed vendor CLIs isolated). Clippy required an allowance for the unchanged warning in `desktop-server/src/evidence.rs:744`. F7 has pushed on `gemini/modes/f5-f7-mode-shell` and is queued for review.
2. **S6 merged in PR 28.** The worker uses Windows AppContainer startup and staging. All 19 Windows worker tests and 33 Python tests passed, along with Linux touched-crate checks, strict Clippy and resume tests. The Windows installer definition includes the check files. Building and installing it remains pending.
3. **V3 merged; V4, V5 and V6 next.** V3 (`0f7cbdf`) is wired behind `AuthGuard::authorize_api` (`/api/sessions/{id}/investigation/{skeleton,mapview,node-text}`) and the normal Investigate UI entry so TeX papers open immediately into the outline, shared graph, results list and Block A node card with zero model calls (even with no model connected). V6 (`inv-v6-tasks`) is now unblocked; V4 (`eafe8a1`) has pushed and V5 is in progress on `gemini/modes/inv-a-v2-v5`. V17 is running against merged V2/F14a.
4. **F13a.** Inspect the `0db46e3` checkpoint, which includes the length split after `eea6e2c`. Integrate the independent fix for atomic sync, safe retries and preservation of a path shared across source roots. Register the source dispatcher behind F5’s API authorization. The original library lane has pushed F16a and is continuing F15 core. The Kandinsky port is authorized by the owner.
5. **Other completed work** — review each branch as it appears, prioritizing the two Solve badge fixes and the fixed P/Q case sets. Carbon audits are evidence, while the coordinator still verifies candidate diffs and tests.

## Remaining package order

- **Investigate:** V3 → V4/V5; V3 → V6 → V7 → V8; V9 legacy adapter after V6; V10 reader notes after V7; V11 cited lookup after V4/F10 with the PDF path after V17; V12 context cards after V8/V11; V13 graph after V5/F6 with main-result filters after V7; V15 import/export after V8; V16 handoffs after V8/F7; V18 arXiv HTML after V4; V19 retirement only after the V9 compatibility release. V17 PDF skeleton follows V2/F14a. V14 and the version-tracking half of V10 are deferred.
- **Explore:** X3 session and X4 first screen after F5 using merged F11/X1; X6 runner, X7 plan and approvals after F12, X8 evidence and X9 queue, X10 passage reader, X11 verdict/map, X12 UI, X13 handoffs, X14 fixed/live evaluation, X15 conversation, X16 citation expansion, X17 library/export, X18 subscription-route parity, X19 optional services. X0/X2/X5 were absorbed into shared packages.
- **Solve:** S4/S5 setup and first screen as F5 and the worker foundations land; S7 expression store, S8 typed steps, S9 algebra checks, S10 physics checks, S11 finish gate, S12 resumable runner, S13 step/receipt UI, S14 tool-capable verifier, S15 CLI parity; then S17 derivation intake, S18 numerical/artifact tools, S19 notebook export, S20 blind recomputation, S21 legacy workflow integration. F8 owns S0, F1 owns S1. S16 Lean/physlib is cut. S22 Cadabra2/FORM is deferred.
- **Shared:** F7 handoff packets carry exact typed sources, context and provenance. F13b handles BibTeX/Zotero after F13a; F13c handles opt-in write-back. F14b local OCR for image-only pages follows measured P6 need and the owner’s dependency decision. F14c optional image transcription is visibly unconfirmed. F15 core has a receipt-aware TeX/Markdown/BibTeX model and adapters after their mode records exist. F16a corrections change display or ranking under code rules, and F16b supplies lessons as user-controlled model context. Corrections and lessons cannot decide a verification or prior-work verdict.

**Release gate:** The next visible milestone is a TeX paper opening into its source-built Investigate structure. Subsequent gates cover the progressive two-paper argument walkthrough, real P/P6 extracts, Explore evidence/coverage, Solve planted failures, cross-mode handoffs, and old ledger resume. Keep `main` for the final Windows-verified promotion.
