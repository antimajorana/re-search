# Evaluation sheet: not recorded, live tier

Format: one sheet per run (overview section 7.1). Measures and targets: section 7.3.

## Run

| Field | Value |
|---|---|
| Date | not recorded |
| Tier | live |
| Gate for milestone | none |
| Build: git commit | not recorded |
| Build: crate version | not recorded |
| Models | none |
| Model calls | not measured |
| Wall time | not measured |
| Cases run (0) | none |

## Gate: INCOMPLETE

PASS needs every measure that has a pass rule to be measured and to pass. A measure without a value counts as not measured, never as a pass.

- Failed (0): none
- Pass rule, not measured (15): `time_to_skeleton_tex_s`, `time_to_candidates_s`, `time_to_solve_setup_s`, `runs_above_cap`, `statement_recall_tex_theorem_envs`, `section_recall_tex_theorem_envs`, `wrong_carry_overs`, `judged_same_key_different_unreviewed`, `wrong_statement_located`, `planted_error_ends_failed_pct`, `planted_error_ends_computed`, `quotes_surviving_handover_pct`, `old_ledgers_not_resuming`, `old_ledgers_view_differs`, `forbidden_word_property_failures`

## Measures

| Measure | Id | Applies to | Rule | Value | Outcome | Says in 7.3 | Note |
|---|---|---|---|---|---|---|---|
| Investigate: time from input to skeleton, TeX paper | `time_to_skeleton_tex_s` | All modes | below 10 | not measured | not measured | Investigate TeX under 10 s |  |
| Explore: time from input to candidates | `time_to_candidates_s` | All modes | below 6 | not measured | not measured | Explore candidates under 6 s |  |
| Solve: time from input to setup | `time_to_solve_setup_s` | All modes | below 1 | not measured | not measured | Solve setup under 1 s |  |
| Runs whose model calls or tokens went above the cap | `runs_above_cap` | All | must be 0 | not measured | not measured | Never above the cap |  |
| Typical model calls per run | `model_calls_typical` | All | recorded | not measured | not measured | typical value recorded |  |
| Typical tokens per run | `tokens_typical` | All | recorded | not measured | not measured | typical value recorded |  |
| Theorem-like statement recall, all papers of P | `statement_recall` | P | recorded | not measured | not measured |  |  |
| Section recall, all papers of P | `section_recall` | P | recorded | not measured | not measured |  |  |
| Theorem-like statement recall, TeX papers with theorem environments | `statement_recall_tex_theorem_envs` | P | at least 0.9 | not measured | not measured | below 0.9 on TeX papers with theorem environments blocks M1 |  |
| Section recall, TeX papers with theorem environments | `section_recall_tex_theorem_envs` | P | at least 0.9 | not measured | not measured | below 0.9 on TeX papers with theorem environments blocks M1 |  |
| "Uses" edge precision, statement role | `uses_edge_precision_statement` | P | recorded | not measured | not measured |  |  |
| "Uses" edge precision, proof role | `uses_edge_precision_proof` | P | recorded | not measured | not measured |  |  |
| Tags unchanged for unchanged statements | `tags_unchanged_share` | P2 | recorded | not measured | not measured |  |  |
| Wrong carry-overs | `wrong_carry_overs` | P2 | must be 0 | not measured | not measured | Wrong carry-overs must be 0 |  |
| Authors' label agreement with the key | `author_label_agreement` | P3 | recorded | not measured | not measured |  |  |
| Share of labels whose quote contains a cue | `label_quote_cue_share` | P3 | recorded | not measured | not measured |  |  |
| Known earlier source found | `earlier_source_found` | P4, I | recorded | not measured | not measured |  |  |
| "None in scope" shown for a result that has a known earlier source | `none_in_scope_false_negative_rate` | P4, I | recorded | not measured | not measured | The second number is the false-negative rate that decides whether the feature is announced |  |
| "Judged same" where the key says different | `judged_same_key_different` | P4, I | recorded | not measured | not measured | Recorded; every such case reviewed |  |
| Of those, cases not yet reviewed | `judged_same_key_different_unreviewed` | P4, I | must be 0 | not measured | not measured | every such case reviewed |  |
| Wrong statement shown as located | `wrong_statement_located` | P5 | must be 0 | not measured | not measured | Must be 0 (candidates list is acceptable) |  |
| Known-paper recall of the search alone, keyless services | `search_recall_keyless` | I | recorded | not measured | not measured | Recorded per service set (keyless; with keys) |  |
| Known-paper recall of the search alone, services with keys | `search_recall_with_keys` | I | recorded | not measured | not measured | Recorded per service set (keyless; with keys) |  |
| Quotes accepted per model call | `quotes_accepted_per_model_call` | Investigate tasks, Explore readers | recorded | not measured | not measured |  |  |
| Quotes rejected (reasons in the note) | `quotes_rejected` | Investigate tasks, Explore readers | recorded | not measured | not measured |  |  |
| Planted error ends Failed | `planted_error_ends_failed_pct` | Q | must equal 100 | not measured | not measured | Must be 100 percent |  |
| Planted error ends Computed | `planted_error_ends_computed` | Q | must be 0 | not measured | not measured | ending Computed is a release blocker |  |
| Share of steps Computed | `steps_computed_share` | Q | recorded | not measured | not measured | decides whether SymPy's reach is enough |  |
| Share of steps Model agreed | `steps_model_agreed_share` | Q | recorded | not measured | not measured | decides whether SymPy's reach is enough |  |
| Share of steps Unchecked | `steps_unchecked_share` | Q | recorded | not measured | not measured | decides whether SymPy's reach is enough |  |
| Final answer equals the key | `final_answer_equals_key` | Q | recorded | not measured | not measured |  |  |
| Quotes surviving re-matching after a hand-over of unchanged bytes | `quotes_surviving_handover_pct` | C | must equal 100 | not measured | not measured | Must be 100 percent |  |
| Target case solved correctly when started from a hand-over | `handover_target_solved` | C | recorded | not measured | not measured |  |  |
| Old ledgers that do not resume | `old_ledgers_not_resuming` | R | must be 0 | not measured | not measured | Must pass; in CI |  |
| Old ledgers whose views differ before and after | `old_ledgers_view_differs` | R | must be 0 | not measured | not measured | Must pass; in CI |  |
| Peak memory on the reference laptop | `peak_memory_mb` | All | recorded | not measured | not measured | Recorded at each milestone |  |
| Build time on the reference laptop | `build_time_s` | All | recorded | not measured | not measured | Recorded at each milestone |  |
| Failing property tests for "new", "novel", "proved", "verified" outside the allowed places | `forbidden_word_property_failures` | All | must be 0 | not measured | not measured | Property tests in CI |  |

38 measures: 15 with a pass rule, 23 recorded only.

## Remarks

None.
