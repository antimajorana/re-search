//! Structural branch diffing across verified session branches (F14).

use crate::{
    latex::{collect_proof_items, ProofItem},
    BranchDiffReport,
};
use ledger::{canonical_json, Event, Ledger};
use serde_json::Value;
use similar::TextDiff;
use std::collections::BTreeMap;

fn extract_proof_entries(events: &[Event], entries: &[ProofItem]) -> Vec<Value> {
    let by_seq: BTreeMap<_, _> = entries.iter().map(|entry| (entry.seq, entry)).collect();
    events
        .iter()
        .filter(|ev| ev.kind == "proof.entry")
        .map(|ev| {
            let mut p = ev.payload.clone();
            if let Some(obj) = p.as_object_mut() {
                obj.insert("seq".into(), Value::from(ev.seq));
                if let Some(entry) = by_seq.get(&ev.seq) {
                    obj.insert("status".into(), Value::from(entry.status.as_str()));
                    if let Some(replacement) = &entry.superseded_by {
                        obj.insert("superseded_by".into(), Value::from(replacement.as_str()));
                    }
                }
            }
            p
        })
        .collect()
}

fn extract_final_answer(events: &[Event], entries: &[ProofItem]) -> Option<String> {
    let mut final_entries = entries
        .iter()
        .filter(|entry| entry.entry_type == "final_answer")
        .peekable();
    if final_entries.peek().is_some() {
        return final_entries
            .filter(|entry| entry.status != "superseded")
            .max_by_key(|entry| entry.seq)
            .and_then(|entry| {
                [
                    Some(entry.details.as_str()),
                    Some(entry.summary.as_str()),
                    entry.latex.as_deref(),
                ]
                .into_iter()
                .flatten()
                .find(|text| !text.trim().is_empty())
                .map(str::to_string)
            });
    }
    events
        .iter()
        .rev()
        .filter(|ev| ev.kind == "model.text")
        .filter_map(|ev| ev.payload["text"].as_str())
        .find(|text| !text.trim().is_empty())
        .map(str::to_string)
}

fn format_branch_summary(entries: &[ProofItem], final_answer: Option<&str>) -> String {
    let mut lines = Vec::new();
    for e in entries {
        let status = match &e.superseded_by {
            Some(replacement) => format!("superseded by {replacement}"),
            None => e.status.clone(),
        };
        lines.push(format!(
            "{} [{}] ({status}) {}: {}",
            e.entry_id, e.entry_type, e.summary, e.details
        ));
        if !e.depends_on.is_empty() {
            lines.push(format!("  Depends on: {}", e.depends_on.join(", ")));
        }
        if let Some(latex) = &e.latex {
            lines.push(format!("  LaTeX: {latex}"));
        }
    }
    if let Some(ans) = final_answer {
        lines.push(format!("FINAL_ANSWER: {ans}"));
    }
    lines.join("\n")
}

pub fn compute_branch_diff(
    ledger: &Ledger,
    base_session: &str,
    target_session: &str,
) -> Result<BranchDiffReport, String> {
    let (_, base_events) = ledger
        .read_verified_session(base_session)
        .map_err(|e| format!("base session `{base_session}` failed verification: {e}"))?;
    let target_events = if target_session == base_session {
        base_events.clone()
    } else {
        ledger
            .read_verified_session(target_session)
            .map_err(|e| format!("target session `{target_session}` failed verification: {e}"))?
            .1
    };
    Ok(diff_events(
        base_session,
        &base_events,
        target_session,
        &target_events,
    ))
}

/// Compare the current branch to the original prefix identified by its saved fork receipt.
pub fn compute_fork_diff(
    ledger: &Ledger,
    target_session: &str,
    fork_seq: u64,
    fork_event_hash: &str,
) -> Result<BranchDiffReport, String> {
    let (_, events) = ledger
        .read_verified_session(target_session)
        .map_err(|e| format!("branch session `{target_session}` failed verification: {e}"))?;
    let index = fork_seq
        .checked_sub(1)
        .and_then(|seq| usize::try_from(seq).ok());
    let index = index
        .filter(|&index| index > 0 && index < events.len())
        .ok_or("the recorded fork point is missing from the branch ledger")?;
    let event = &events[index];
    if event.kind != "session.fork"
        || event.event_hash != fork_event_hash
        || event.payload["parent_seq"].as_u64() != Some(index as u64)
    {
        return Err("the branch ledger does not match its recorded fork receipt".into());
    }
    let parent = event.payload["parent_session"]
        .as_str()
        .filter(|parent| !parent.is_empty())
        .ok_or("the recorded fork point has no parent session")?;
    Ok(diff_events(
        parent,
        &events[..index],
        target_session,
        &events,
    ))
}

fn diff_events(
    base_session: &str,
    base_events: &[Event],
    target_session: &str,
    target_events: &[Event],
) -> BranchDiffReport {
    let mut shared_prefix_len = 0usize;
    for (b, t) in base_events.iter().zip(target_events.iter()) {
        if b.kind == t.kind
            && b.schema_version == t.schema_version
            && canonical_json(&b.payload) == canonical_json(&t.payload)
        {
            shared_prefix_len += 1;
        } else {
            break;
        }
    }

    let divergence_seq =
        if shared_prefix_len < base_events.len() || shared_prefix_len < target_events.len() {
            Some((shared_prefix_len + 1) as u64)
        } else {
            None
        };

    let base_only_events = base_events.len().saturating_sub(shared_prefix_len);
    let target_only_events = target_events.len().saturating_sub(shared_prefix_len);

    let base_proof = collect_proof_items(base_events);
    let target_proof = collect_proof_items(target_events);
    let base_entries = extract_proof_entries(base_events, &base_proof);
    let target_entries = extract_proof_entries(target_events, &target_proof);
    let base_final_answer = extract_final_answer(base_events, &base_proof);
    let target_final_answer = extract_final_answer(target_events, &target_proof);

    let base_text = format_branch_summary(&base_proof, base_final_answer.as_deref());
    let target_text = format_branch_summary(&target_proof, target_final_answer.as_deref());

    let diff = TextDiff::from_lines(&base_text, &target_text);
    let mut udiff = diff.unified_diff();
    udiff
        .context_radius(3)
        .header(&format!("a/{base_session}"), &format!("b/{target_session}"));
    let summary_diff = udiff.to_string();

    BranchDiffReport {
        base_session: base_session.to_string(),
        target_session: target_session.to_string(),
        shared_prefix_len,
        divergence_seq,
        base_only_events,
        target_only_events,
        base_entries,
        target_entries,
        base_final_answer,
        target_final_answer,
        summary_diff,
    }
}
