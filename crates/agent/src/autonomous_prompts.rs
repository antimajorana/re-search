//! Prompts, Proof-DAG final-answer synthesis, and Strategist escalation helpers
//! for [`crate::autonomous`].

use crate::autonomous::{AutonomousConfig, AutonomousRoles};
use crate::honesty::HonestyReport;
use crate::runner::Audit;
use crate::strategist::{
    consult_strategist_cancellable, count_strategist_consultations, mandatory_strategist_required,
    MAX_STRATEGIST_CONSULTATIONS,
};
use crate::transport::{CancelToken, ContentBlock, Message, Role, Transport};
use crate::verifiers::VerificationReport;
use research_tools::{read_proof_dag, ProofDag, ProofStatus, LEDGER_APPEND, LEDGER_SUPERSEDE};
use serde_json::{json, Value};
use tools::{CallOutcome, Router};

/// Clean-room default system prompt for the Orchestrator, teaching convention pinning,
/// worker delegation (`delegate_compute`, `delegate_literature`, `delegate_lean`,
/// `delegate_explore`), honesty (`evidence != proof`), Proof-DAG discipline, and synthesis.
pub const RESEARCH_SYSTEM_PROMPT: &str = "\
You are re:Search, the Orchestrator of a rigorous mathematical and theoretical-physics research \
workspace backed by an auditable, hash-chained ledger.\n\n\
Workflow & Tool Discipline:\n\
1. Pin Conventions First: State conventions explicitly (metric signature, Fourier/normalization \
factors, operator grading, lattice/indexing conventions) and record foundational assumptions via \
`ledger_append` (`entry_type: \"assumption\"`).\n\
2. Delegate Specialist Work (or use direct tools when delegation is absent): \
Delegate heavy symbolic/numerical Python scripts, traceback retries, and `check_expression_sanity` \
to `delegate_compute` (or call `sympy_eval` / `check_expression_sanity` directly). \
In `sympy_eval`, use `check_symbolic_identity(lhs, rhs, variable=x, domain=...)` to record the residual, \
exact branch tests, and any `limits=[(point, direction)]` or `dimensions={symbol: {base: power}}`. \
Use `check_convergence(run, levels, axis=..., atol=..., rtol=..., reference=...)` for precision, resolution, \
or basis refinement. Set the levels, tolerances and independent reference when available before running. \
Return the report as the final Python expression, and cite the resulting computation entry. State its scope, \
especially excluded points, counterexamples, or convergence failures. Agreement over sampled levels is empirical. \
Use `set_random_seed(seed)` before stochastic Python calculations. Record the precision, \
tolerance, and relevant assumptions for numerical checks. \
Delegate arXiv/literature searches, ingestion, and bulky `.tex` reading under `sources` to \
`delegate_literature` (or use `arxiv_search`, `literature_search`, `fs_list`, `fs_read` directly \
rather than guessing citations from memory). \
Use `search_paper_passages` to retrieve exact file and line references from ingested sources. \
Quote only passages that support the claim, and include the source path, lines, and tool reference. \
For formal physics, use `search_formal_library` in the attached workspace to find existing Physlib or Lean declarations. \
Match the declaration's assumptions and statement to the current claim before calling `lean_check`. \
Delegate Lean 4 formalization and `lean_check` machine-checked proofs to `delegate_lean`. \
Use `delegate_explore` to test 2..=4 competing strategies or Ansaetze concurrently before committing.\n\
3. Honesty & Evidence Discipline (`evidence != proof`): Never claim `SymPy confirmed`, \
`Lean verified`, `Lean 4 verified`, or `rigorously proved` / `Q.E.D.` without matching \
ledger-backed verification (a succeeding `sympy_eval`, a `lean_check` with status `\"verified\"`, \
or an accepted full Verifier review).\n\
4. Maintain the Proof DAG: Record assumptions (`assumption`), derivations (`derivation`), \
verified calculations (`symbolic_computation` / `numerical_computation`), interpreted conclusions \
(`result` citing dependencies in `depends_on`) via `ledger_append`, and your complete concluding \
synthesis (`final_answer`). Store the returned `tool_call_ref` on each proof entry recording a \
successful SymPy calculation or Lean check. Link the relevant evidence through `depends_on` in your \
final answer or cite its proof entry ID in the answer text. For a literature claim, use a `reference_lookup` \
entry with the relevant source read or search result's `tool_call_ref` and record the source location \
when known. Give EVERY `ledger_append` entry a clear one-line `summary` and a \
self-contained `details` explanation spelling out the exact steps, equations, and why the result \
follows from `depends_on` (so inspecting any node in the Proof DAG is immediately readable). \
Supersede flawed entries explicitly with `ledger_supersede`.\n\
5. Synthesize Self-Contained Exposition: Write clear, pedagogical Markdown and LaTeX \
(`$...$`, `$$...$$`, `\\begin{align*}...\\end{align*}`) stating hypotheses, intuition, definitions, \
intermediate steps without hand-waving, and conclusions. If a target claim fails or hits an \
obstruction, prove clearly what IS valid under modified hypotheses or provide an explicit counterexample.";

pub(crate) fn ensure_active_final_answer_entry(
    router: &Router,
    audit: &mut Audit<'_>,
    leg_start_seq: u64,
    round: u32,
    last_text: &str,
) -> Result<String, String> {
    if last_text.trim().is_empty() {
        return Err("the solver produced no final answer text to verify".to_string());
    }
    let dag = read_proof_dag(audit.ledger, audit.session)?;
    let fresh_final = dag.entries.iter().rev().find(|e| {
        e.seq > leg_start_seq && e.entry_type == "final_answer" && e.status != ProofStatus::Superseded
    });
    let new_id = if let Some(entry) = fresh_final {
        entry.entry_id.clone()
    } else {
        let deps: Vec<_> = dag
            .entries
            .iter()
            .filter(|e| e.entry_type != "final_answer" && e.status != ProofStatus::Superseded)
            .map(|e| &e.entry_id)
            .collect();
        let details: String = last_text.chars().take(15_500).collect();
        append_supervisor_entry(router, audit, &json!({
            "entry_type": "final_answer",
            "summary": format!("Candidate solution (verification round {round})"),
            "details": details,
            "depends_on": deps,
        }))?
    };

    let rejected: Vec<_> = dag
        .entries
        .iter()
        .filter(|e| e.seq <= leg_start_seq && e.entry_type == "final_answer" && e.status == ProofStatus::Rejected)
        .map(|e| &e.entry_id)
        .collect();
    if rejected.is_empty() {
        return Ok(new_id);
    }
    let reason = format!("Revised candidate recorded in {new_id}");
    let correction_id = append_supervisor_entry(router, audit, &json!({
        "entry_type": "correction",
        "summary": "Replace rejected final answer",
        "details": reason,
        "depends_on": [new_id],
    }))?;
    for entry_id in rejected {
        let outcome = router.try_call(
            "supervisor:autonomous",
            LEDGER_SUPERSEDE,
            &json!({ "entry_id": entry_id, "replaced_by": correction_id, "reason": reason }),
        )?;
        audit.flush_interleaved()?;
        if let CallOutcome::Denied(err) | CallOutcome::Error(err) = outcome {
            return Err(format!("could not supersede rejected final answer {entry_id}: {err}"));
        }
    }
    Ok(new_id)
}

fn append_supervisor_entry(router: &Router, audit: &mut Audit<'_>, args: &Value) -> Result<String, String> {
    let outcome = router.try_call("supervisor:autonomous", LEDGER_APPEND, args)?;
    audit.flush_interleaved()?;
    match outcome {
        CallOutcome::Ok(value) => value["entry_id"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| "ledger_append returned no entry ID".into()),
        CallOutcome::Denied(err) | CallOutcome::Error(err) => Err(format!("could not record proof entry: {err}")),
    }
}

pub(crate) fn maybe_run_strategist(
    solver_transport: &dyn Transport,
    solver_model: &str,
    roles: &AutonomousRoles<'_>,
    audit: &mut Audit<'_>,
    auto_cfg: &AutonomousConfig,
    report: &VerificationReport,
    cancel: Option<&CancelToken>,
) -> Result<Option<String>, String> {
    let _ = mandatory_strategist_required;
    if !auto_cfg.use_strategist {
        return Ok(None);
    }
    let used = count_strategist_consultations(audit.ledger, audit.session).unwrap_or(0);
    if used >= MAX_STRATEGIST_CONSULTATIONS {
        return Ok(None);
    }
    let (s_transport, s_model) = roles.strategist.unwrap_or((solver_transport, solver_model));
    audit.sink.on_thinking_delta(&format!(
        "[Strategist ({s_model}): analyzing verifier rejection & formulating repair plan...]\n"
    ));
    let issue_list: Vec<String> = report
        .issues
        .iter()
        .map(|i| format!("- [{}] {}", i.severity, i.message))
        .collect();
    let question = format!(
        "Independent Verifier rejected `{}`.\nSummary: {}\nCounterexample: {}\nIssues:\n{}\n\
         Propose a concrete mathematical repair strategy and checks the Solver should run next.",
        report.target_entry_id,
        report.summary,
        report.counterexample.as_deref().unwrap_or("none"),
        if issue_list.is_empty() { "(none listed)".into() } else { issue_list.join("\n") }
    );
    match consult_strategist_cancellable(s_transport, audit.ledger, audit.session, &question, s_model, cancel) {
        Ok(adv) => {
            audit.flush_interleaved()?;
            Ok(Some(adv.advice))
        }
        Err(_) => {
            audit.flush_interleaved()?;
            Ok(None)
        }
    }
}

pub(crate) fn build_revision_prompt(
    report: &VerificationReport,
    round: u32,
    max_rounds: u32,
    strategist_advice: Option<&str>,
    honesty: &HonestyReport,
) -> String {
    let mut out = format!(
        "<verification_feedback round=\"{round}\" max_rounds=\"{max_rounds}\" target=\"{}\">\n\
         The independent stateless Verifier REJECTED your candidate solution.\n\
         Verifier summary: {}\n",
        report.target_entry_id, report.summary
    );
    if let Some(cx) = &report.counterexample {
        out.push_str(&format!("Counterexample: {cx}\n"));
    }
    if !report.issues.is_empty() {
        out.push_str("Issues to resolve:\n");
        for iss in &report.issues {
            out.push_str(&format!("- [{}] {}\n", iss.severity, iss.message));
        }
    }
    let honesty_errs: Vec<_> = honesty.findings.iter().filter(|f| f.severity == "error").collect();
    if !honesty_errs.is_empty() {
        out.push_str("Honesty Linter errors to resolve:\n");
        for f in honesty_errs {
            out.push_str(&format!("- `{}`: {} ({})\n", f.claim, f.reason, f.suggestion));
        }
    }
    if let Some(advice) = strategist_advice.filter(|a| !a.trim().is_empty()) {
        out.push_str(&format!(
            "\n<strategist_unverified_advice>\n{advice}\n</strategist_unverified_advice>\n"
        ));
    }
    out.push_str(
        "\nInstructions for revision:\n\
         1. Diagnose whether the rejection is (a) a fixable gap / target mismatch on the right theorem, or \
         (b) a structural obstruction in your current approach.\n\
         2. If fixable, repair every Verifier issue rigorously (use `sympy_eval`, `arxiv_search`, or `fs_read` \
         as needed). If your current proof strategy is obstructed, pivot to a **genuinely different approach** \
         (you may call `delegate_explore` to test competing strategies concurrently).\n\
         3. Supersede flawed Proof-DAG entries with `ledger_supersede` and record your revised `final_answer` \
         via `ledger_append`.\n\
         4. If the claim cannot hold under the stated hypotheses, prove an explicit counterexample or the \
         sharpest theorem that IS valid under stated additional assumptions, and explain clearly what is possible.\n\
         5. Output a complete, self-contained revised exposition (never a bare diff against the rejected draft).\n\
         </verification_feedback>",
    );
    out
}

pub(crate) fn format_unverified_banner(report: &VerificationReport, rounds: u32) -> String {
    let mut msg = format!(
        "> **Unverified Draft (Independent Verifier still raised issues after {rounds} round(s)):** {}\n",
        report.summary
    );
    for iss in report.issues.iter().take(5) {
        msg.push_str(&format!("> - **[{}]** {}\n", iss.severity, iss.message));
    }
    msg
}

pub(crate) fn build_unverified_recovery(
    report: &VerificationReport,
    rounds: u32,
    dag: &ProofDag,
    strategist_advice: Option<&str>,
) -> String {
    let verified_ids: Vec<String> = dag
        .entries
        .iter()
        .filter(|e| e.status == ProofStatus::Accepted && e.entry_type != "final_answer")
        .map(|e| format!("`{}`", e.entry_id))
        .collect();
    let compute_ids: Vec<String> = dag
        .entries
        .iter()
        .filter(|e| {
            e.status != ProofStatus::Superseded
                && matches!(e.entry_type.as_str(), "symbolic_computation" | "numerical_computation")
        })
        .map(|e| format!("`{}`", e.entry_id))
        .collect();
    let mut parts = Vec::new();
    if !verified_ids.is_empty() || !compute_ids.is_empty() {
        let mut foundations = Vec::new();
        if !verified_ids.is_empty() {
            foundations.push(format!("accepted proof steps ({})", verified_ids.join(", ")));
        }
        if !compute_ids.is_empty() {
            foundations.push(format!("recorded computations ({})", compute_ids.join(", ")));
        }
        parts.push(format!("**Solid foundation available:** {}.", foundations.join("; ")));
    }
    if let Some(advice) = strategist_advice.map(str::trim).filter(|s| !s.is_empty()) {
        let clipped: String = advice.chars().take(600).collect();
        parts.push(format!("**Suggested repair direction:** {clipped}"));
    } else if rounds >= 1 {
        parts.push(format!(
            "**Why verification paused after {rounds} round(s):** `{}` did not close verification cleanly ({}).",
            report.target_entry_id, report.summary
        ));
    }
    parts.push(
        "**What you can do next:** Choose **Try Another Approach** to test an alternative proof strategy, \
         **Fix Flagged Issues** to repair the specific gap, or **Discuss in Explain** to examine modified hypotheses."
            .to_string(),
    );
    parts.join("\n\n")
}

pub(crate) fn inject_feedback(
    audit: &mut Audit<'_>,
    messages: &mut Vec<Message>,
    round: u32,
    target: &str,
    text: &str,
) -> Result<(), String> {
    audit.emit(
        "autonomous.feedback",
        json!({ "round": round, "target": target, "text": text }),
    )?;
    messages.push(Message {
        role: Role::User,
        content: vec![ContentBlock::Text { text: text.to_string() }],
    });
    Ok(())
}
