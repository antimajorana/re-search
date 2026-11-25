# Three modes, one foundation (2026-11)

Plan and working files for the Investigate, Explore and Solve modes and the shared foundation
packages F0 to F16 (F13 to F16 were added by the scope revision of 2026-11-25). The work branch is `claude/project-thread-caszr2`, based on
`gemini/work/fixed`.

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
| F3 network gate and paper fetch | brief `agents/f3-net-gate.md` | `gemini/modes/f3-net-gate` |
| F4 quote core, F9 run budget | brief `agents/f4-f9-quote-budget.md` | `gemini/modes/f4-f9-quote-budget` |
| V1 skeleton core for TeX | brief `agents/v1-skeleton.md` | `gemini/modes/v1-skeleton` |
| S2 Solve model, state fold, badge function | brief `agents/s2-solve-model.md` | `gemini/modes/s2-solve-model` |
| F1 tool profiles | **not started: waits for the owner's decision 1** | - |
| F13 paper sources and sync, F14 PDF and old papers, F15 write-up, F16 corrections and lessons | planned, no brief yet (overview 2.11 to 2.14; F14a in week 4, F13a in week 6) | - |
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
and 8.3 (new questions raised by the revision) are not answered yet. The recommended options are provisional. No agent
implements F1 (tool profiles), adds a tool to any registry, or changes a tool definition.
