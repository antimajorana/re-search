//! Task-kind system prompts for Solve mode (`solve.md` 5.2, 5.4, 9 S12).
//!
//! Kept strictly separate from [`crate::RESEARCH_SYSTEM_PROMPT`] so Research and
//! Investigate sessions remain byte-identical.

use research_tools::solve_model::{SolveSetup, TaskKind};

/// Short snake_case label matching `SolveSetup.task_kind`.
pub fn task_kind_label(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::Calculate => "calculate",
        TaskKind::Prove => "prove",
        TaskKind::CheckDerivation => "check_derivation",
    }
}

/// Focused operational guidance tailored to `TaskKind`.
pub fn task_kind_instructions(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::Calculate => "\
Task Kind: calculate\n\
- Compute the target expression or numerical value cleanly and check domain, limiting cases, \
dimensions, and symmetries before concluding.\n\
- Assign intermediate and final expressions via `save_expr(name, expr)` inside `sympy_eval` so \
later steps reference versioned handles rather than re-typed text.\n\
- Derive LHS and RHS independently: keep the expression under test in unevaluated form \
(`UnevaluatedExpr(...)`, `Integral(...)`, `Sum(...)`, `Derivative(...)`, `Limit(...)`, `MatMul(...)`, \
`Trace(...)`, or an unevaluated string `save_expr(\"lhs\", \"(x + 1)**2\", locals=...)`) against an \
independently constructed RHS. Never save the same expected constant or simplified result on both sides: \
identical-side claims return `trivial` and earn no tier.\n\
- Record computation steps (`symbolic_computation` or `numerical_computation`) citing the \
`sympy_eval` `tool_call_ref`, build supporting `derivation`/`result` steps, and conclude with a \
`final_answer` step matching the requested target form, order, or digits.",
        TaskKind::Prove => "\
Task Kind: prove\n\
- Record foundational hypotheses and setup assumptions first (`assumption`), then construct a \
complete dependency-linked chain (`derivation`, `symbolic_computation`, `result`) reaching the \
theorem statement (`final_answer`).\n\
- For every quantitative equality, inequality, series relation, or counterexample search, derive \
both sides independently in `sympy_eval` and attach structured `claims` on `step_record`. Keep the \
left-hand expression unevaluated (via `UnevaluatedExpr`, `Integral`/`Sum`/`Derivative`/`MatMul`/`Trace`, \
or a string expression in `save_expr(\"lhs\", \"...\", locals=...)`) rather than pre-evaluating it to the \
expected constant: never write the expected result on both sides, because identical-side claims return \
`trivial` and earn no tier.\n\
- Do not skip edge cases, branch cuts, or quantifier restrictions; record counterexamples or \
needed refinements immediately if a check fails.",
        TaskKind::CheckDerivation => "\
Task Kind: check_derivation\n\
- Audit the supplied derivation step-by-step against the attached sources (`fs_read`, \
`read_paper_lines`, `search_paper_passages`) and algebraic checks.\n\
- Extract each key claimed identity, limit, or transformation into distinct LHS and RHS SymPy handles \
via `sympy_eval` (preserving the unevaluated form via `UnevaluatedExpr`, `Integral`/`Sum`/`Derivative`, \
or string expressions in `save_expr` — never saving the same expected constant on both sides, which \
returns `trivial` and earns no tier), record each audited step with explicit `claims`, and run \
domain/limit/dimension/counterexample checks (`check_run`).\n\
- Record any gap or sign/factor/regime error with `correction` or superseded steps, and make the \
`final_answer` deliver clear per-step verdicts matching the target.",
    }
}

/// Build the Solve system prompt from a [`SolveSetup`] and optional extra sections.
pub fn build_solve_system_prompt(setup: &SolveSetup, extra_sections: &[&str]) -> String {
    let mut out = String::new();
    out.push_str(
        "You are re:Search Solver operating in `solve/1` mode over a tamper-evident ledger.\n\n",
    );
    out.push_str(task_kind_instructions(setup.task_kind));
    out.push_str("\n\nSolve Discipline & Host Gates:\n");
    out.push_str(
        "1. Handles First & Independent Sides: Python variables do not survive across calls or worker \
restarts; only handles saved with `save_expr(name, expr)` do. Derive LHS and RHS independently and \
keep the expression under test unevaluated (`UnevaluatedExpr`, `Integral`/`Sum`/`Derivative`/`MatMul`/`Trace`, \
or an unevaluated string passed to `save_expr(name, expr_str, locals=...)`). Never write the expected \
constant/result on both sides — identical-side claims return `trivial` and earn no tier. Inspect stored \
handles with `expr_show`.\n",
    );
    out.push_str(
        "2. Gate G1 (Interpret Runs): Never leave more than 3 successful `sympy_eval` runs \
un-cited. Either cite each run in `step_record.tool_call_refs` or discard exploratory runs via \
`ledger_note` with text starting `\"discard: \"`.\n",
    );
    out.push_str(
        "3. Structured Proof Graph (`step_record`): Cite `depends_on` explicitly (host also tracks \
handle provenance in `derived_depends_on`). Never manufacture formulas in prose when a handle claim \
applies; supersede flawed steps with `ledger_supersede` rather than abandoning them or re-rooting.\n",
    );
    out.push_str(
        "4. Host Checks & Verdicts: `step_record` automatically runs `AutoCheckGate` on claims; use \
`check_run` for targeted `series_match`, `vanishes_to_order`, `counterexample_search`, \
`dimensions`, `limit_case`, `symmetry`, `conservation`, `convergence`, or `matrix_property` checks. \
If an auto-check returns `trivial` or `inconclusive`, inspect the residual/attention note and repair.\n",
    );
    out.push_str(
        "5. R22 Honesty & `solve_finish`: Only use the word \"verified\" for steps whose effective \
tier is `Verified`, and never claim the overall solution is verified or proved unless the final \
step's effective tier is `Verified`. Call `solve_finish` once ready.\n\n",
    );

    out.push_str("## Problem Setup\n");
    out.push_str(&format!("Task kind: {}\n", task_kind_label(setup.task_kind)));
    out.push_str(&format!("Target ({:?}): {}\n", setup.target.form, setup.target.description.trim()));
    if let Some(order) = &setup.target.order {
        out.push_str(&format!("Target order: {order}\n"));
    }
    if let Some(digits) = setup.target.digits {
        out.push_str(&format!("Target digits: {digits}\n"));
    }
    out.push_str("Statement:\n");
    out.push_str(setup.statement.trim());
    out.push('\n');

    if !setup.conventions.trim().is_empty() {
        out.push_str("\nConventions:\n");
        out.push_str(setup.conventions.trim());
        out.push('\n');
    }
    if !setup.symbols.is_empty() {
        out.push_str("\nSymbols:\n");
        for sym in &setup.symbols {
            if sym.assumptions.is_empty() {
                out.push_str(&format!("- {}\n", sym.name));
            } else {
                out.push_str(&format!("- {} ({})\n", sym.name, sym.assumptions.join(", ")));
            }
        }
    }
    if !setup.dimensions.is_empty() {
        out.push_str("\nDimensions:\n");
        for (sym, dims) in &setup.dimensions {
            let formatted = serde_json::to_string(dims).unwrap_or_else(|_| "{}".into());
            out.push_str(&format!("- {sym}: {formatted}\n"));
        }
    }
    if !setup.assumptions.is_empty() {
        out.push_str("\nSetup Assumptions:\n");
        for a in &setup.assumptions {
            out.push_str(&format!("- [{}] {}\n", a.id, a.text));
        }
    }
    if !setup.sources.is_empty() {
        out.push_str("\nAttached Sources:\n");
        for src in &setup.sources {
            out.push_str(&format!("- {src}\n"));
        }
    }
    if !setup.done.is_empty() {
        out.push_str("\nDone Checklist:\n");
        for c in &setup.done {
            out.push_str(&format!("- {}: {:?} ({})\n", c.id, c.kind, c.params));
        }
    }

    for section in extra_sections {
        let trimmed = section.trim();
        if !trimmed.is_empty() {
            out.push_str("\n\n");
            out.push_str(trimmed);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use research_tools::solve_model::{
        compute_math_sha256, DoneCriterion, DoneCriterionKind, ReviewPolicy, RoleModel,
        SetupAssumption, SolveBudgets, SolveRoles, SolveTarget, SymbolSpec, TargetForm,
    };
    use research_tools::tool_profile::PROFILE_SOLVE_V1;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn sample_setup(kind: TaskKind) -> SolveSetup {
        let symbols = vec![SymbolSpec {
            name: "x".into(),
            assumptions: vec!["real".into(), "positive".into()],
        }];
        let assumptions = vec![SetupAssumption {
            id: "A1".into(),
            text: "x > 0".into(),
        }];
        let dimensions = BTreeMap::new();
        let conventions = "Natural units hbar = c = 1.".to_string();
        let math_sha256 = compute_math_sha256(&symbols, &dimensions, &assumptions, &conventions);
        SolveSetup {
            revision: 1,
            statement: "Evaluate the Gaussian integral.".into(),
            task_kind: kind,
            target: SolveTarget {
                description: "Exact closed-form integral".into(),
                form: TargetForm::ClosedForm,
                order: None,
                digits: None,
            },
            done: vec![DoneCriterion {
                id: "c1".into(),
                kind: DoneCriterionKind::FinalTierAtLeast,
                params: json!({ "tier": "verified" }),
            }],
            conventions,
            symbols,
            dimensions,
            assumptions,
            given_source: None,
            sources: vec!["notes.tex".into()],
            roles: SolveRoles {
                solver: RoleModel { provider: "anthropic".into(), model: "claude-sonnet-4-5".into() },
                verifier: RoleModel { provider: "openai".into(), model: "gpt-4.1".into() },
                compute: None,
            },
            budgets: SolveBudgets::default(),
            review: ReviewPolicy::default(),
            tool_profile: PROFILE_SOLVE_V1.into(),
            origin: None,
            math_sha256,
        }
    }

    #[test]
    fn prompts_cover_all_task_kinds_and_keep_research_prompt_untouched() {
        let calc = build_solve_system_prompt(&sample_setup(TaskKind::Calculate), &[]);
        let prove = build_solve_system_prompt(&sample_setup(TaskKind::Prove), &[]);
        let check = build_solve_system_prompt(
            &sample_setup(TaskKind::CheckDerivation),
            &["Extra focus: branch cuts"],
        );
        assert!(calc.contains("Task Kind: calculate") && calc.contains("Python variables do not survive"));
        assert!(prove.contains("Task Kind: prove") && prove.contains("solve_finish"));
        assert!(check.contains("Task Kind: check_derivation") && check.contains("Extra focus: branch cuts"));
        assert_ne!(calc, prove);
        assert_ne!(prove, check);
        assert!(crate::RESEARCH_SYSTEM_PROMPT.contains("You are re:Search, the Orchestrator"));
    }
}
