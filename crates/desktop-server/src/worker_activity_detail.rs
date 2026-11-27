//! Lazy detail projection helper for `worker_activity_fold`.

use agent::usage::{extract_usage, TokenUsage};
use ledger::{Event, Ledger};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

use super::{JobBuilder, WorkerJobDetail, WorkerJobSummary, WORKER_SUMMARY_UNVERIFIED_LABEL};

const MAX_BLOB_PREVIEW_CHARS: usize = 2_000;

pub(super) fn extract_end_usage(payload: &Value) -> Option<TokenUsage> {
    if let Some(u) = payload.get("usage") {
        let parsed = extract_usage(u);
        if !parsed.is_unknown() {
            return Some(parsed);
        }
    }
    let inp = payload.get("input_tokens").and_then(Value::as_u64);
    let out = payload.get("output_tokens").and_then(Value::as_u64);
    (inp.is_some() || out.is_some()).then_some(TokenUsage { input: inp, output: out })
}

pub(super) fn scrub_secrets(raw: &str, secrets: &[String]) -> String {
    let mut out = raw.to_string();
    for s in secrets {
        if !s.is_empty() && out.contains(s) {
            out = out.replace(s, "[REDACTED]");
        }
    }
    out
}

pub(super) fn find_owner_job<'a>(
    jobs: &'a mut BTreeMap<String, JobBuilder>,
    order: &[String],
    caller: Option<&str>,
    seq: u64,
) -> Option<&'a mut JobBuilder> {
    if let Some(c) = caller {
        if let Some(id) = order.iter().rev().find(|id| {
            let j = &jobs[*id];
            let in_window = seq >= j.seq_start && (j.ok.is_none() || seq <= j.seq_end);
            j.callers.contains(c) || (in_window && !j.role.is_empty() && c.contains(&j.role))
        }) {
            return jobs.get_mut(id);
        }
    }
    order
        .iter()
        .rev()
        .find(|id| {
            let j = &jobs[*id];
            seq >= j.seq_start && (j.ok.is_none() || seq <= j.seq_end)
        })
        .and_then(|id| jobs.get_mut(id))
}

fn extract_tool_blob_hash(payload: &Value) -> Option<&str> {
    payload
        .get("result_blob")
        .and_then(Value::as_str)
        .or_else(|| payload.get("result").and_then(|r| r.get("blob")).and_then(Value::as_str))
        .or_else(|| payload.get("error").and_then(|e| e.get("blob")).and_then(Value::as_str))
        .filter(|s| !s.is_empty())
}

pub(super) fn build_job_detail(
    summary: &WorkerJobSummary,
    j: &JobBuilder,
    events: &[Event],
    ledger: &Ledger,
    secrets: &[String],
    pinned: &BTreeSet<&str>,
    dismissed: &BTreeMap<&str, Option<&str>>,
) -> WorkerJobDetail {
    let mut artifact_missing = summary.artifact_refs.iter().any(|r| r.missing);

    let read_blob_preview = |hash: &str| -> (bool, Option<String>) {
        match ledger.get_blob(hash) {
            Ok(bytes) => {
                let raw = String::from_utf8_lossy(&bytes);
                let clean = scrub_secrets(&raw, secrets);
                let clipped: String = clean.chars().take(MAX_BLOB_PREVIEW_CHARS).collect();
                (false, Some(clipped))
            }
            Err(_) => (true, None),
        }
    };

    let mut request_blobs = Vec::new();
    for (seq, h) in &j.req_blobs {
        let (missing, preview) = read_blob_preview(h);
        if missing {
            artifact_missing = true;
        }
        request_blobs.push(json!({ "seq": seq, "blob": h, "missing": missing, "preview": preview }));
    }

    let mut response_blobs = Vec::new();
    for (seq, h) in &j.resp_blobs {
        let (missing, preview) = read_blob_preview(h);
        if missing {
            artifact_missing = true;
        }
        response_blobs.push(json!({
            "seq": seq, "blob": h, "missing": missing, "preview": preview,
            "label": WORKER_SUMMARY_UNVERIFIED_LABEL,
        }));
    }

    let worker_summary = if let Some(h) = &j.summary_blob {
        let (missing, preview) = read_blob_preview(h);
        if missing {
            artifact_missing = true;
        }
        json!({
            "blob": h,
            "missing": missing,
            "text": preview,
            "label": WORKER_SUMMARY_UNVERIFIED_LABEL,
        })
    } else {
        json!({
            "blob": null,
            "missing": false,
            "text": j.inline_summary.as_deref().map(|s| scrub_secrets(s, secrets)),
            "label": WORKER_SUMMARY_UNVERIFIED_LABEL,
        })
    };

    let upper_seq = if j.ok.is_none() { u64::MAX } else { j.seq_end.max(j.seq_start) };

    let mut context_supplied = Vec::new();
    for ev in events.iter().filter(|e| e.kind == "context.supplied") {
        let caller = ev.payload.get("caller").and_then(Value::as_str).unwrap_or("");
        let matches_caller = j.callers.contains(caller)
            || (!j.role.is_empty()
                && caller.starts_with(&format!("worker:{}:", j.role))
                && ev.seq >= j.seq_start
                && ev.seq <= upper_seq);
        let matches_run = j.run_id.as_deref().is_some_and(|rid| {
            ev.payload.get("run_id").and_then(Value::as_str) == Some(rid)
                && caller.contains(&j.role)
        });
        if matches_caller || matches_run {
            if let Some(ranges) = ev.payload.get("ranges").and_then(Value::as_array) {
                for r in ranges {
                    let mut obj = r.clone();
                    if let Some(m) = obj.as_object_mut() {
                        m.insert("caller".into(), json!(caller));
                        m.insert("seq".into(), json!(ev.seq));
                    }
                    context_supplied.push(obj);
                }
            }
        }
    }

    let mut req_by_call: BTreeMap<String, (u64, String, String)> = BTreeMap::new();
    let mut tool_receipts = Vec::new();
    for ev in events.iter().filter(|e| e.seq >= j.seq_start && e.seq <= upper_seq) {
        if ev.kind == "tool.request" {
            let cid = ev.payload.get("call_id").and_then(Value::as_str).unwrap_or("").to_string();
            let tool = ev.payload.get("tool").and_then(Value::as_str).unwrap_or("").to_string();
            let caller = ev.payload.get("caller").and_then(Value::as_str).unwrap_or("").to_string();
            if !cid.is_empty()
                && (j.callers.contains(&caller) || (!j.role.is_empty() && caller.contains(&j.role)))
            {
                req_by_call.insert(cid, (ev.seq, tool, caller));
            }
        } else if ev.kind == "tool.result" {
            let cid = ev.payload.get("call_id").and_then(Value::as_str).unwrap_or("");
            if let Some((req_seq, tool, caller)) = req_by_call.get(cid) {
                let blob = extract_tool_blob_hash(&ev.payload);
                let missing = blob.is_some_and(|h| ledger.get_blob(h).is_err());
                if missing {
                    artifact_missing = true;
                }
                tool_receipts.push(json!({
                    "call_id": cid,
                    "tool": tool,
                    "caller": caller,
                    "request_seq": req_seq,
                    "result_seq": ev.seq,
                    "ok": ev.payload.get("ok").and_then(Value::as_bool).unwrap_or(false),
                    "result_blob": blob,
                    "artifact_missing": missing,
                    "exprs_out": ev.payload.get("exprs_out").cloned().unwrap_or_else(|| json!([])),
                }));
            }
        }
    }

    let mut returned_items = Vec::new();
    for item_id in &summary.returned_item_ids {
        let mut row = json!({
            "id": item_id,
            "pinned": pinned.contains(item_id.as_str()),
            "dismissed": dismissed.contains_key(item_id.as_str()),
        });
        if let Some(seq_num) = item_id
            .strip_prefix("ev-")
            .or_else(|| item_id.strip_prefix("xe-"))
            .or_else(|| item_id.strip_prefix("xa-"))
            .and_then(|s| s.parse::<u64>().ok())
        {
            if let Some(ev) = events.iter().find(|e| e.seq == seq_num) {
                row["kind"] = json!(ev.kind);
                row["seq"] = json!(ev.seq);
                if let Some(q) = ev.payload.get("quote").and_then(Value::as_str) {
                    row["quote"] = json!(q);
                }
                if let Some(sid) = ev.payload.get("source_id").and_then(Value::as_str) {
                    row["source_id"] = json!(sid);
                }
            }
        }
        returned_items.push(row);
    }

    WorkerJobDetail {
        job: summary.clone(),
        artifact_missing,
        request_blobs,
        response_blobs,
        worker_summary,
        context_supplied,
        tool_receipts,
        returned_items,
        checks: j.check_rows.clone(),
    }
}
