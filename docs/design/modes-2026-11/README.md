# Three modes, one foundation (2026-11)

Plan and working files for the Investigate, Explore and Solve modes and the shared foundation
packages F0 to F16 (F13 to F16 were added by the scope revision of 2026-11-25). The base branch is `gemini/work/fixed`.
Priority (owner, 2026-11-25): Investigate first, with the foundation pieces it needs.

| File | What it is |
|---|---|
| `00-overview.md` | **The authoritative plan.** Scope revision (section 0.1), shared packages (section 2), contradictions resolved, build order (section 6), evaluation set (section 7), owner decisions (section 8) |
| `investigate.md`, `explore.md`, `solve.md` | The three mode designs, already reconciled to the overview. Where one of them and the overview disagree, the overview wins |
| `reSearch-modes-gaps-and-prior-art-2026-11-25.md` | Background: gaps and prior art |
| `agents/RULES.md` | Rules for every Gemini agent on this work |
| `agents/<brief>.md` | One brief per agent |
| `reports/<brief>.md` | Each agent's report |

## Status

| Package | State | Where |
|---|---|---|
| F0 guard rails | done | work branch |
| F2 paper key | done | work branch, `crates/research-tools/src/paper_key.rs` |
| F8 evaluation harness skeleton | done | work branch, `crates/agent/tests/eval_support/`, `tests/fixtures/eval/`, `docs/design/eval-2026-11/` |
| F8 Solve fixed case set Q | reviewed and merged | `tests/fixtures/eval/manifest-q.json`, `crates/agent/tests/eval_q_cases.rs`; report `reports/eval-q-cases.md` |
| F8 Investigate fixed case set P | reviewed and merged | `tests/fixtures/eval/manifest-p.json`, `tests/fixtures/eval/p/`, `crates/agent/tests/eval_p_cases.rs`; report `reports/eval-p-cases.md` |
| F1/F4/F14a integration review | Carbon review recorded | `reports/audit-mode-foundations.md` |
| Later lane briefs | reviewed and merged (30 gate-aligned drafts) | `agents/draft-*.md`; report `reports/draft-next-lanes.md` |
| F8 Solve evaluation gate and badge names | reviewed and merged | `crates/agent/tests/eval_support/{measures,solve_q_tests}.rs`, `docs/design/eval-2026-11/sheet-template.md`; report `reports/eval-solve-badges.md` |
| F9b Orchestrator and smaller worker routing | design and Carbon review recorded | `reports/audit-orchestration-routing.md` |
| Solve evidence labels | reviewed and merged | `solve_badge.rs`, `tests/solve_evidence_labels.rs`; report `reports/solve-evidence-labels.md` |
| Solve comparison evidence | reviewed and merged | `solve_own_tier*.rs`; report `reports/solve-comparison-evidence.md` |
| F3 network gate and paper fetch | done, reviewed and merged | `net_rate.rs`, `paper_fetch*.rs`, `ledger/src/fulltext_cache.rs`; report `reports/f3-net-gate.md` |
| F4 quote core, F9 run budget | done, reviewed and merged | `quote_core.rs`, `lines_sent.rs`, `run_budget.rs`; report `reports/f4-f9-quote-budget.md` |
| V1 skeleton core for TeX | done, reviewed and merged | `crates/research-tools/src/skeleton/`; report `reports/v1-skeleton.md` |
| S2 Solve model, state fold, badge function | done, reviewed and merged | `solve_model*.rs`, `solve_state.rs`, `solve_status.rs`, `solve_badge.rs`, `solve_own_tier.rs`, `proof_closure.rs`; report `reports/s2-solve-model.md` |
| F1 tool profiles | done, reviewed and merged | `tool_profile.rs`, `resume_profile.rs`; report `reports/f1-tool-profiles.md`. Sessions started after this do not resume in older builds |
| F6 graph view and shared UI parts | done, reviewed and merged | `assets/graph_layout.js`, `graph_view.js`, `ui_badge.js`, `run_bar.js`, `quote_chip.js`; report `reports/f6-graph-ui.md` |
| F14a PDF text core | done, reviewed and merged; local page-link probe passed on seven user-selected PDFs | `pdf_pages*.rs`, `pdf_firstpage*.rs`, `pdf_match.rs`; report `reports/f14a-pdf-core.md`. Full P6 evaluation pending |
| F10 library | done, reviewed and merged; `/api/library` registered by F5 | `library*.rs`, `ledger/src/library_*.rs`, `routes_library.rs`; report `reports/f10-library.md` |
| F11 literature services, X1 terms and ranking, S3 router gate | reviewed and merged | `lit_providers.rs`, `lit_abstracts.rs`, `idea_{terms,queries,rank}.rs`, `tools/src/gate.rs` |
| S6 check worker protocol | protocol and contained check role verified on Windows and Linux; installer verification pending | `codex/modes-worker-windows` |
| F12 coverage gate and prior-work wording | reviewed and merged | `search_coverage*.rs`, `prior_work*.rs`; report `reports/f12-coverage.md` |
| V2 Investigate dependencies, tags, numbering and bibliography | reviewed and merged | `crates/research-tools/src/skeleton/`; checkpoint `afea25b` |
| V3 Investigate skeleton on screen | reviewed, wired and merged; Windows browser verified | `investigation_{skeleton,mapview}.rs`, `assets/investigation_{outline,results,node,v3_shell}.js`; checkpoint `0f7cbdf` |
| V4 Investigate paper opener, V5 skip plan and symbols | reviewed, wired and merged; Windows verified | `investigation_open.rs`, `skeleton/{skip,symbols}.rs`, `assets/investigation_{entry,skip}.js`; report `reports/inv-a-v2-v5.md` |
| F5 mode shell and library route registration | reviewed and merged; Windows browser verified | `session_mode.rs`, `api_dispatch.rs`, `assets/new_menu.js`; checkpoint `a5c9b7f` |
| F7 hand-off packet | reviewed and merged; Windows verified | `crates/export/src/handoff*.rs`, `crates/desktop-server/src/handoff.rs`; report `reports/f5-f7-mode-shell.md` |
| Library lane: F13a sources and sync, F16a corrections by code, F15 write-up core | reviewed, wired and merged (`/api/sources` and `/api/lessons` registered) | `paper_sources.rs`, `source_*.rs`, `lessons*.rs`, `export/src/writeup*.rs`; report `reports/lib-f13a-f16a-f15.md` |
| F13a library sync integrity | reviewed and merged | `library_db_sync*.rs`, `source_sync*.rs`, `tests/source_sync_integrity.rs`; report `reports/f13a-sync-integrity.md` |
| V17 Investigate PDF skeleton | reviewed, wired and merged; Windows verified | `crates/research-tools/src/skeleton/pdf*.rs`; report `reports/inv-v17-pdf-skeleton.md` |
| V6 Investigate reader and task lifecycle | Neon launched | `gemini/modes/inv-v6-tasks` |
| V11 Investigate cited result without a model | Neon launched | `gemini/modes/inv-v11-cited` |
| V13 Investigate focused and whole-paper graph views | Neon launched | `gemini/modes/inv-v13-wholemap` |
| X3 Explore session and stage machine, X4 model-free first screen | Neon launched | `gemini/modes/exp-x3-x4-shell` |
| S4 Solve setup and view, S5 first screen and shared approval card | Neon launched | `gemini/modes/solve-s4-s5-setup` |
| S7 Solve `solve/1` profile freeze and expression store | Neon launched | `gemini/modes/solve-s7-exprs` |
| F13b BibTeX/Zotero adapters, F13c opt-in Kandinsky write-back | Neon launched | `gemini/modes/f13bc-sources-ext` |
| F14b, F14c, F16b, mode adapters of F15 | gate-aligned draft briefs in `agents/draft-*.md` | `reports/draft-next-lanes.md` |
| S16 formal tier (Lean) | cut | - |
| S22 engine manager (Cadabra2, FORM), V14 prior work in Investigate, version tracking (half of V10) | deferred | - |

## Scope revision of 2026-11-25

The owner decided: add a bring-your-own-papers library with sync (designed on the owner's Kandinsky
app), PDF-only and old papers (essential), a write-up output and learning from corrections; skip
numerics ahead of Lean; cut Lean and physlib; defer Cadabra2, FORM, V14 and version tracking.
`00-overview.md` section 0.1 says what each decision means; the three mode documents carry a "Scope
revision" block under their titles. For briefs in work: S2 has no Lean-checked tier (brief updated);
F3 and V1 are unaffected.

Licence: re:Search is AGPL-3.0 (owner decision of 2026-11-25; on `main` since PR 5). Kandinsky's
code may be ported directly; the rules and the list of files are in overview 2.11, "Porting".

## Owner decisions

The scope decisions are recorded in section 8.1 of the overview. Sections 8.2 (the first questions)
and 8.3 (questions raised by the revision) were accepted in bulk by the owner on 2026-11-25: every
recommended answer there is the decision. This lifts the earlier hold on F1 (tool profiles), which
is now approved as the single way a session's tools are fixed; tools are still added only through
a profile. In Solve, "Verified" means an exact symbolic check by code and "Checked numerically"
means numeric agreement (overview 0.1, `solve.md` 4.4).
