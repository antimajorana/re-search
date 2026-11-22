//! Prompts, Proof-DAG final-answer synthesis, and Strategist escalation helpers
//! for [`crate::autonomous`].

use crate::autonomous::{AutonomousConfig, AutonomousRoles};
use crate::runner::Audit;
use crate::strategist::{
    consult_strategist_cancellable, count_strategist_consultations, mandatory_strategist_required,
    MAX_STRATEGIST_CONSULTATIONS,
};
use crate::transport::{CancelToken, ContentBlock, Message, Role, Transport};
use crate::verifiers::VerificationReport;
use research_tools::{read_proof_dag, ProofStatus, LEDGER_APPEND, LEDGER_SUPERSEDE};
use serde_json::{json, Value};
use tools::{CallOutcome, Router};

/// Clean-room default system prompt for research sessions, teaching convention pinning,
/// primary-literature source verification, symbolic/numerical sanity checks, and Proof-DAG discipline.
pub const RESEARCH_SYSTEM_PROMPT: &str = "\
You are re:Search, a rigorous mathematical and theoretical-physics research assistant operating \
inside an auditable, hash-chained workspace.\n\n\
Workflow & Tool Discipline:\n\
1. Pin Conventions First: State conventions explicitly (metric signature, Fourier/normalization \
factors, operator grading, lattice/indexing conventions) and record foundational assumptions via \
`ledger_append` (`entry_type: \"assumption\"`).\n\
2. Ground Primary Literature: When citing theorems, definitions, or papers, verify exact statements \
using `arxiv_search` (supports inline prefixes such as `au:Author AND ti:Keyword`, `cat:math-ph`, \
or bare arXiv IDs; pass `include_source: true` and `section_filter` to unpack `.tar.gz`/`.gz` LaTeX \
e-prints into `sources`) and `literature_search`. Read unpacked TeX files via `fs_list` / `fs_read` \
(`root: \"sources\"`) rather than guessing citations from memory.\n\
3. Compute & Sanity-Check: Use `sympy_eval` and `check_expression_sanity` to verify algebraic \
identities, commutators, signs, prefactors, dimensional consistency, small-system exact checks, \
and limiting cases. Never claim a symbolic check succeeded unless `sympy_eval` actually ran.\n\
4. Maintain the Proof DAG: Record key derivations (`derivation`), verified calculations \
(`symbolic_computation` / `numerical_computation`), interpreted conclusions (`result` citing prior \
steps in `depends_on`), and your complete concluding synthesis (`final_answer` via `ledger_append`). \
If an earlier Proof-DAG entry is flawed, supersede it explicitly with `ledger_supersede`.\n\
5. Self-Contained Exposition: Write clear, publication-grade Markdown and LaTeX (`$...$`, `$$...$$`, \
`\\begin{align*}...\\end{align*}`) stating hypotheses, definitions, intermediate lemmas, and conclusions \
without hand-waving.";

pub(crate) fn ensure_active_final_answer_entry(
    router: &Router,
    audit: &mut Audit<'_>,
    leg_start_seq: u64,
    round: u32,
    last_text: &str,
) -> Result<(), String> {
    let dag = read_proof_dag(audit.ledger, audit.session)?;
    let has_fresh_final = dag.entries.iter().any(|e| {
        e.seq > leg_start_seq && e.entry_type == "final_answer" && e.status != ProofStatus::Superseded
    });
    if has_fresh_final {
        return Ok(());
    }
    let prior_finals: Vec<String> = dag
        .entries
        .iter()
        .filter(|e| e.entry_type == "final_answer" && e.status != ProofStatus::Superseded)
        .map(|e| e.entry_id.clone())
        .collect();
    let deps: Vec<String> = dag
        .entries
        .iter()
        .filter(|e| e.entry_type != "final_answer" && e.status != ProofStatus::Superseded)
        .map(|e| e.entry_id.clone())
        .collect();
    let details: String = if last_text.trim().is_empty() {
        "(empty candidate response)".to_string()
    } else {
        last_text.chars().take(15_500).collect()
    };
    let out = router.try_call(
        "supervisor:autonomous",
        LEDGER_APPEND,
        &json!({
            "entry_type": "final_answer",
            "summary": format!("Candidate solution (verification round {round})"),
            "details": details,
            "depends_on": deps,
        }),
    )?;
    audit.flush_interleaved()?;
    if let CallOutcome::Ok(val) = out {
        if let Some(new_id) = val.get("entry_id").and_then(Value::as_str) {
            for old_id in prior_finals {
                let _ = router.try_call(
                    "supervisor:autonomous",
                    LEDGER_SUPERSEDE,
                    &json!({
                        "entry_id": old_id,
                        "replaced_by": new_id,
                        "reason": format!("Superseded by revised candidate in round {round}"),
                    }),
                )?;
            }
            audit.flush_interleaved()?;
        }
    }
    Ok(())
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
    if let Some(advice) = strategist_advice.filter(|a| !a.trim().is_empty()) {
        out.push_str(&format!(
            "\n<strategist_unverified_advice>\n{advice}\n</strategist_unverified_advice>\n"
        ));
    }
    out.push_str(
        "\nInstructions for revision:\n\
         1. Address every Verifier issue and counterexample rigorously (use `sympy_eval` or \
         `arxiv_search`/`fs_read` if needed to verify claims or citations).\n\
         2. Supersede any flawed Proof-DAG entries with `ledger_supersede` and append your \
         corrected `final_answer` via `ledger_append`.\n\
         3. Output a complete, self-contained revised answer for the user (do not merely write \
         a diff against your rejected draft).\n\
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
