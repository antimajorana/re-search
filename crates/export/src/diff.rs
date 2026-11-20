//! Structural branch diffing across verified session branches (F14).

use crate::BranchDiffReport;
use ledger::{canonical_json, Event, Ledger};
use serde_json::Value;
use similar::TextDiff;

fn extract_proof_entries(events: &[Event]) -> Vec<Value> {
    events
        .iter()
        .filter(|ev| ev.kind == "proof.entry")
        .map(|ev| {
            let mut p = ev.payload.clone();
            if let Some(obj) = p.as_object_mut() {
                obj.insert("seq".into(), Value::from(ev.seq));
            }
            p
        })
        .collect()
}

fn extract_final_answer(events: &[Event]) -> Option<String> {
    let from_proof = events
        .iter()
        .rev()
        .find(|ev| {
            ev.kind == "proof.entry"
                && ev.payload.get("entry_type").and_then(Value::as_str) == Some("final_answer")
        })
        .and_then(|ev| {
            ev.payload
                .get("details")
                .or_else(|| ev.payload.get("summary"))
                .or_else(|| ev.payload.get("latex"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    if from_proof.is_some() {
        return from_proof;
    }
    events
        .iter()
        .rev()
        .find(|ev| ev.kind == "model.text")
        .and_then(|ev| ev.payload.get("text").and_then(Value::as_str).map(str::to_string))
}

fn format_branch_summary(entries: &[Value], final_answer: Option<&str>) -> String {
    let mut lines = Vec::new();
    for e in entries {
        let id = e.get("entry_id").and_then(Value::as_str).unwrap_or("?");
        let ty = e.get("entry_type").and_then(Value::as_str).unwrap_or("entry");
        let sum = e.get("summary").and_then(Value::as_str).unwrap_or("");
        let det = e.get("details").and_then(Value::as_str).unwrap_or("");
        lines.push(format!("{id} [{ty}] {sum}: {det}"));
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
    ledger
        .verify_session(base_session)
        .map_err(|e| format!("base session `{base_session}` failed verification: {e}"))?;
    ledger
        .verify_session(target_session)
        .map_err(|e| format!("target session `{target_session}` failed verification: {e}"))?;

    let base_events = ledger
        .read_session(base_session)
        .map_err(|e| format!("failed to read `{base_session}`: {e}"))?;
    let target_events = ledger
        .read_session(target_session)
        .map_err(|e| format!("failed to read `{target_session}`: {e}"))?;

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

    let divergence_seq = if shared_prefix_len < base_events.len() || shared_prefix_len < target_events.len() {
        Some((shared_prefix_len + 1) as u64)
    } else {
        None
    };

    let base_only_events = base_events.len().saturating_sub(shared_prefix_len);
    let target_only_events = target_events.len().saturating_sub(shared_prefix_len);

    let base_entries = extract_proof_entries(&base_events);
    let target_entries = extract_proof_entries(&target_events);
    let base_final_answer = extract_final_answer(&base_events);
    let target_final_answer = extract_final_answer(&target_events);

    let base_text = format_branch_summary(&base_entries, base_final_answer.as_deref());
    let target_text = format_branch_summary(&target_entries, target_final_answer.as_deref());

    let diff = TextDiff::from_lines(&base_text, &target_text);
    let mut udiff = diff.unified_diff();
    udiff
        .context_radius(3)
        .header(&format!("a/{base_session}"), &format!("b/{target_session}"));
    let summary_diff = udiff.to_string();

    Ok(BranchDiffReport {
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
    })
}
