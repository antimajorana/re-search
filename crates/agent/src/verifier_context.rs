//! Context assembly, ancestor slicing, XML formatting, linked check summaries,
//! and deterministic TeX audit runner shared by the stateless Research verifiers.

use crate::guardrails::materialize_proof_entries;
use crate::honesty::{recorded_tool_checks, recorded_value};
use crate::resume::fork_assumption;
use ledger::{Event, Ledger};
use research_tools::{
    read_proof_dag, tex_audit::run_tex_audit_with_definitions, AuditOptions, AuditReport,
    ProofEntry, ProofStatus, ResearchEnv,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// The caller resolves the answer text and sequence from its own session transcript.
#[derive(Clone, Copy)]
pub struct AnswerClaim<'a> {
    pub answer_seq: u64,
    pub answer: &'a str,
    pub claim: &'a str,
    pub occurrence: usize,
    pub cited_steps: &'a [String],
}

impl AnswerClaim<'_> {
    pub(crate) fn context_xml(&self) -> Result<String, String> {
        let (offset, _) = self
            .answer
            .match_indices(self.claim)
            .nth(self.occurrence)
            .ok_or("The selected occurrence could not be found in the saved answer.")?;
        let before = &self.answer[..offset];
        let after = &self.answer[offset + self.claim.len()..];
        let start = before.char_indices().rev().nth(1499).map_or(0, |(i, _)| i);
        let end = after.char_indices().nth(1500).map_or(after.len(), |(i, _)| i);
        Ok(format!(
            "<answer_context><preceding>{}</preceding><selected_claim>{}</selected_claim><following>{}</following></answer_context>\n\n",
            xml_text(&before[start..]),
            xml_text(self.claim),
            xml_text(&after[..end])
        ))
    }
}

pub(crate) fn load_context_and_entries(
    ledger: &Ledger,
    session: &str,
) -> Result<(String, Vec<ProofEntry>, Vec<Event>), String> {
    let events = ledger
        .read_session(session)
        .map_err(|e| format!("cannot read session `{session}`: {e}"))?;
    let context = problem_context(&events);
    let dag = read_proof_dag(ledger, session)?;
    let entries = if !dag.entries.is_empty() {
        dag.entries
    } else {
        materialize_proof_entries(&events)
    };
    Ok((context, entries, events))
}

pub(crate) fn problem_context(events: &[Event]) -> String {
    let turns = events
        .iter()
        .filter(|e| e.kind == "session.start" || e.kind == "session.resume");
    // Each turn records its own conventions, including when the setting is cleared.
    let latest_turn = turns.clone().next_back();
    let pinned_conventions = latest_turn
        .and_then(|e| e.payload.get("pinned_conventions"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty());
    let prompts: Vec<&str> = turns
        .filter_map(|e| e.payload.get("prompt").and_then(Value::as_str))
        .filter(|p| !p.trim().is_empty())
        .collect();
    let problem_prompt = match prompts.as_slice() {
        [] => "(no session prompt recorded)".to_string(),
        [single] => (*single).to_string(),
        [earlier @ .., latest] => {
            let context = earlier
                .iter()
                .enumerate()
                .map(|(i, p)| format!("{}. {p}", i + 1))
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "Latest user request to verify:\n{latest}\n\nEarlier session prompts (context):\n{context}"
            )
        }
    };
    let mut context = format!(
        "<problem_prompt>\n{}\n</problem_prompt>",
        xml_text(&problem_prompt)
    );
    if let Some(project) = latest_turn.and_then(|event| event.payload["project_context"].as_str()) {
        context.push_str(&format!(
            "\n\n<project_context>\n{}\n</project_context>",
            xml_text(project)
        ));
    }
    let fork_assumptions: Vec<_> = events
        .iter()
        .filter_map(fork_assumption)
        .map(xml_text)
        .collect();
    if !fork_assumptions.is_empty() {
        context.push_str(&format!(
            "\n\n<fork_assumptions>\n{}\n</fork_assumptions>",
            fork_assumptions.join("\n\n"),
        ));
    }
    if let Some(conventions) = pinned_conventions {
        context.push_str(&format!(
            "\n\n<pinned_conventions>\n{}\n</pinned_conventions>",
            xml_text(conventions)
        ));
    }
    context
}

pub(crate) fn xml_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub(crate) fn collect_ancestor_slice(
    entries: &[ProofEntry],
    target_id: &str,
) -> Result<Vec<ProofEntry>, String> {
    if !entries.iter().any(|entry| entry.entry_id == target_id) {
        return Err(format!("proof entry `{target_id}` not found"));
    }
    Ok(ancestor_entries(entries, &[target_id]))
}

pub(crate) fn ancestor_entries(entries: &[ProofEntry], targets: &[&str]) -> Vec<ProofEntry> {
    let by_id: BTreeMap<&str, &ProofEntry> =
        entries.iter().map(|e| (e.entry_id.as_str(), e)).collect();
    let mut visited = BTreeSet::new();
    let mut stack: Vec<_> = targets.iter().map(|id| id.to_string()).collect();
    while let Some(cur) = stack.pop() {
        if !visited.insert(cur.clone()) {
            continue;
        }
        if let Some(entry) = by_id.get(cur.as_str()) {
            for dep in &entry.depends_on {
                if !visited.contains(dep) {
                    stack.push(dep.clone());
                }
            }
        }
    }
    entries
        .iter()
        .filter(|e| visited.contains(&e.entry_id))
        .cloned()
        .collect()
}

pub(crate) fn format_entries_xml(entries: &[ProofEntry]) -> String {
    let mut out = String::from("<proof_entries>\n");
    for e in entries {
        let deps = e.depends_on.join(", ");
        let latex = e.latex.as_deref().unwrap_or("");
        let tool_ref = e.tool_call_ref.as_deref().unwrap_or("");
        out.push_str(&format!(
            "  <entry id=\"{}\" type=\"{}\" status=\"{:?}\" depends_on=\"{}\">\n    \
             <summary>{}</summary>\n    <details>{}</details>\n    <latex>{}</latex>\n    <tool_call_ref>{}</tool_call_ref>\n  </entry>\n",
            xml_text(&e.entry_id).replace('"', "&quot;"),
            xml_text(&e.entry_type).replace('"', "&quot;"),
            e.status,
            xml_text(&deps).replace('"', "&quot;"),
            xml_text(&e.summary),
            xml_text(&e.details),
            xml_text(latex),
            xml_text(tool_ref),
        ));
    }
    out.push_str("</proof_entries>");
    out
}

pub(crate) fn format_linked_checks(
    ledger: &Ledger,
    events: &[Event],
    ancestors: &[ProofEntry],
    include_rejected: bool,
) -> String {
    let mut lean = Vec::new();
    let mut validation = Vec::new();
    let mut sources = Vec::new();
    let preview = |field: &Value, max: usize| -> String {
        let text = field
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| field.to_string());
        if let Some((end, _)) = text.char_indices().nth(max) {
            format!("{}…", &text[..end])
        } else {
            text
        }
    };
    for check in recorded_tool_checks(
        events,
        Some(ledger),
        &[
            "lean_check",
            "sympy_eval",
            "check_expression_sanity",
            "search_paper_passages",
            "read_paper_lines",
            "search_formal_library",
        ],
    )
    .into_iter()
    .rev()
    {
        let relevant_ids: Vec<_> = ancestors
            .iter()
            .filter(|entry| {
                entry.status != ProofStatus::Superseded
                    && (include_rejected || entry.status != ProofStatus::Rejected)
            })
            .filter(|entry| {
                check.seq < entry.seq
                    && entry
                        .tool_call_ref
                        .as_deref()
                        .is_some_and(|reference| check.matches_ref(reference))
            })
            .map(|entry| &entry.entry_id)
            .collect();
        if relevant_ids.is_empty() {
            continue;
        }
        if check.tool == "lean_check" {
            let args = recorded_value(check.args, Some(ledger)).unwrap_or(Value::Null);
            lean.push(json!({
                "proof_entries": relevant_ids,
                "tool_call_ref": check.call_id,
                "result_seq": check.seq,
                "status": check.result["status"],
                "axioms": check.result["axioms"],
                "source_sha256": check.result["source_sha256"],
                "path": args["path"],
                "inline_code": args["code"],
            }));
        } else if check.tool == "sympy_eval" && validation.len() < 6 {
            let report = &check.result["validation"];
            if report["schema_version"] != 1
                || !matches!(
                    report["kind"].as_str(),
                    Some("symbolic_identity" | "numerical_convergence")
                )
            {
                let args = recorded_value(check.args, Some(ledger)).unwrap_or(Value::Null);
                validation.push(json!({
                    "proof_entries": relevant_ids,
                    "tool_call_ref": check.call_id,
                    "result_seq": check.seq,
                    "kind": "sympy_execution",
                    "code": preview(&args["code"], 1800),
                    "value": preview(&check.result["value"], 1200),
                    "stdout": preview(&check.result["stdout"], 600),
                    "stderr": preview(&check.result["stderr"], 400),
                    "error": preview(&check.result["error"], 400),
                    "worker_truncated": check.result["truncated"],
                    "validation_truncated": check.result["validation_truncated"],
                }));
                continue;
            }
            let notable_samples: Vec<_> = report["samples"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|row| {
                    row["status"] == "counterexample"
                        || row["status"] == "undefined"
                        || row["status"] == "outside_domain"
                        || row["passes"] == false
                        || row["reference_passes"] == false
                        || !row["error"].is_null()
                })
                .take(3)
                .map(|row| preview(row, 400))
                .collect();
            let recent_samples: Vec<_> = report["samples"]
                .as_array()
                .into_iter()
                .flatten()
                .rev()
                .take(3)
                .map(|row| preview(row, 400))
                .collect();
            validation.push(json!({
                "proof_entries": relevant_ids,
                "tool_call_ref": check.call_id,
                "result_seq": check.seq,
                "kind": report["kind"],
                "status": report["status"],
                "assumptions": preview(&report["assumptions"], 350),
                "domain": preview(&report["domain"], 400),
                "residual": preview(&report["residual"], 250),
                "dimensions": preview(&report["dimensions"], 300),
                "limits": preview(&report["limits"], 600),
                "axis": report["axis"],
                "atol": preview(&report["atol"], 80),
                "rtol": preview(&report["rtol"], 80),
                "reference": preview(&report["reference"], 100),
                "notable_samples": notable_samples,
                "recent_samples": recent_samples,
            }));
        } else if check.tool == "check_expression_sanity" && validation.len() < 6 {
            validation.push(json!({
                "proof_entries": relevant_ids,
                "tool_call_ref": check.call_id,
                "result_seq": check.seq,
                "kind": "expression_sanity",
                "lhs": preview(&check.result["lhs"], 600),
                "rhs": preview(&check.result["rhs"], 600),
                "domain": check.result["domain"],
                "delimiter_balanced": check.result["delimiter_balanced"],
                "numeric_eval_supported": check.result["numeric_eval_supported"],
                "numeric_match": check.result["numeric_match"],
                "samples_checked": check.result["samples_checked"],
                "max_abs_diff": check.result["max_abs_diff"],
                "issues": preview(&check.result["issues"], 350),
            }));
        } else if sources.len() < 5
            && matches!(
                check.tool,
                "search_paper_passages" | "read_paper_lines" | "search_formal_library"
            )
        {
            let rows: Vec<_> = if check.tool == "read_paper_lines" {
                let meta = &check.result["a_meta"];
                vec![json!({
                    "path": preview(&meta["path"], 180),
                    "first_line": meta["start_line"].as_u64(),
                    "last_line": meta["end_line"].as_u64(),
                    "file_sha256": preview(&meta["file_sha256"], 64),
                    "source_text": preview(&check.result["text"], 700),
                })]
            } else {
                let paper = check.tool == "search_paper_passages";
                check.result[if paper { "passages" } else { "results" }]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(2)
                    .map(|row| {
                        json!({
                            "path": preview(&row["path"], 180),
                            "first_line": row[if paper { "start_line" } else { "line" }].as_u64(),
                            "last_line": row["end_line"].as_u64(),
                            "file_sha256": preview(&row[if paper { "file_sha256" } else { "source_sha256" }], 64),
                            "source_text": preview(&row[if paper { "excerpt" } else { "signature" }], 700),
                        })
                    })
                    .collect()
            };
            sources.push(json!({
                "proof_entries": relevant_ids,
                "tool": check.tool,
                "tool_call_ref": check.call_id,
                "result_seq": check.seq,
                "records": rows,
            }));
        }
    }
    lean.reverse();
    let mut context = String::new();
    for (tag, checks) in [
        ("lean_checks", lean),
        ("computational_checks", validation),
        ("research_sources", sources),
    ] {
        if !checks.is_empty() {
            context.push_str(&format!(
                "<{tag}>\n{}\n</{tag}>\n\n",
                xml_text(&Value::Array(checks).to_string())
            ));
        }
    }
    context
}

pub(crate) fn run_candidate_tex_audit(
    env: &ResearchEnv,
    ledger: &Ledger,
    session: &str,
    entries: &[ProofEntry],
    delivered_answer: Option<&str>,
    selected: Option<AnswerClaim<'_>>,
) -> Result<AuditReport, String> {
    let mut candidate_texts: Vec<_> = entries
        .iter()
        .map(|e| {
            let body = match &e.latex {
                Some(l) if !l.trim().is_empty() => format!("{}\n{}\n{}", e.summary, e.details, l),
                _ => format!("{}\n{}", e.summary, e.details),
            };
            (format!("proof_entry:{}", e.entry_id), body)
        })
        .collect();
    if let Some(claim) = selected {
        candidate_texts.push(("selected_claim".into(), claim.claim.into()));
    } else if let Some(answer) = delivered_answer {
        candidate_texts.push(("final_answer_text".into(), answer.into()));
    }
    let opts = AuditOptions {
        candidate_texts,
        candidate_only: true,
        ..AuditOptions::default()
    };
    run_tex_audit_with_definitions(env, session, Some(ledger), &opts, selected.map(|c| c.answer))
}
