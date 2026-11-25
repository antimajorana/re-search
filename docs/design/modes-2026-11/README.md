# Three modes, one foundation (2026-11)

Plan and working files for the Investigate, Explore and Solve modes and the shared foundation
packages F0 to F12. The work branch is `claude/project-thread-caszr2`, based on
`gemini/work/fixed`.

| File | What it is |
|---|---|
| `00-overview.md` | **The authoritative plan.** Shared packages, contradictions resolved, build order (section 6), evaluation set (section 7), owner decisions (section 8) |
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
| F8 evaluation harness skeleton | in progress (coordinator) | - |
| F3 network gate and paper fetch | brief `agents/f3-net-gate.md` | `gemini/modes/f3-net-gate` |
| F4 quote core, F9 run budget | brief `agents/f4-f9-quote-budget.md` | `gemini/modes/f4-f9-quote-budget` |
| V1 skeleton core for TeX | brief `agents/v1-skeleton.md` | `gemini/modes/v1-skeleton` |
| S2 Solve model, state fold, badge function | brief `agents/s2-solve-model.md` | `gemini/modes/s2-solve-model` |
| F1 tool profiles | **not started: waits for the owner's decision 1** | - |

## Owner decisions

Section 8 of the overview is not answered yet. The recommended options are provisional. No agent
implements F1 (tool profiles), adds a tool to any registry, or changes a tool definition.
