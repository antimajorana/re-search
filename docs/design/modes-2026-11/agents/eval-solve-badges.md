# Brief eval-solve-badges — align the F8 Q gate with the accepted Solve tiers

S2 is already merged. Read `00-overview.md` 0.1, the M3 line in 6, and 7.3,
`solve.md` 4.4, and `crates/research-tools/src/solve_badge.rs`. F8 predates
the scope revision and still calls a passing check `Computed`. Fix the
fixed-tier evaluation amounts and labels without changing production Solve.

- Own the Q measures in `crates/agent/tests/eval_support/measures.rs`, focused
  tests in `crates/agent/tests/eval_harness.rs`, and the corresponding generated
  template entries if needed. Use the documented regeneration command. These
  files are approved exceptions to the no-design-docs rule in `RULES.md`.
- Require 100 percent of planted errors to finish `Failed`. The gate must
  also fail on any planted error ending `Verified` or `Checked numerically`.
  Record the two step shares separately. Leave `Model agreed` and `Unchecked`
  measures intact. Cover the zero-case path so an empty sample stays unmeasured.
- Review the effect of changing metric keys on saved F8 sheets. Preserve
  historical meaning or report precisely which old input would need updating.
  Do not imply an old `Computed` datum distinguishes exact and numeric checks.
- The Q fixtures are owned by `eval-q-cases`. Leave that manifest, scripts,
  and existing case entries untouched. Tell the integrator what wording in the
  two harness-only case keys still needs to be aligned after both branches
  merge. Keep every existing assertion's strength or strengthen it when the
  accepted design adds a requirement.

Run `cargo test --locked -p agent --test eval_harness`. No production, tool
definition, dependency, private paper or live network changes. Write
`reports/eval-solve-badges.md` and push to `gemini/modes/eval-solve-badges`.
