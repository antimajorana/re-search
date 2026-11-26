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
| F8 Solve fixed case set Q | Neon launched | `gemini/modes/eval-q-cases` |
| F8 Investigate fixed case set P | Neon launched | `gemini/modes/eval-p-cases` |
| F1/F4/F14a integration review | Carbon launched | `gemini/modes/audit-mode-foundations` |
| Later lane briefs | Neon planning launched | `gemini/modes/draft-next-lanes` |
| F8 Solve evaluation gate and badge names | brief ready | `agents/eval-solve-badges.md` |
| F9b Orchestrator and smaller worker routing | design recorded; Carbon review brief ready | `agents/audit-orchestration-routing.md` |
| Solve evidence labels | brief ready; S2 merged | `agents/solve-evidence-labels.md` |
| Solve comparison evidence | brief ready; S2 merged | `agents/solve-comparison-evidence.md` |
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
| F12 coverage gate and prior-work wording | brief ready | `agents/f12-coverage.md` |
| V2 Investigate dependencies, tags, numbering and bibliography | reviewed and merged | `crates/research-tools/src/skeleton/`; checkpoint `afea25b` |
| V3 Investigate skeleton on screen | reviewed, wired and merged; Windows browser verified | `investigation_{skeleton,mapview}.rs`, `assets/investigation_{outline,results,node,v3_shell}.js`; checkpoint `0f7cbdf` |
| Investigate lane A: V4, V5 in order | V4 checkpoint received; V5 in progress | `gemini/modes/inv-a-v2-v5` |
| F5 mode shell and library route registration | reviewed and merged; Windows browser verified | `session_mode.rs`, `api_dispatch.rs`, `assets/new_menu.js`; checkpoint `a5c9b7f` |
| F7 hand-off packet | in progress | `gemini/modes/f5-f7-mode-shell` |
| Library lane: F13a sources and sync, F16a corrections by code, F15 write-up core | F13a checkpoint received; review in progress | `gemini/modes/lib-f13a-f16a-f15` |
| F13a library sync integrity | brief ready against checkpoint | `agents/f13a-sync-integrity.md` |
| V17 Investigate PDF skeleton | brief ready; V2 and F14a merged | `agents/inv-v17-pdf-skeleton.md` |
| V6 Investigate reader and task lifecycle | unblocked by V3; launching | `agents/inv-v6-tasks.md` |
| F13b, F13c, F14b, F14c, F16b, mode adapters of F15 | planned, no brief yet | - |
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
