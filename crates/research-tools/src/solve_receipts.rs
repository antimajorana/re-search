//! Pure Solve receipt view over `tool.request`/`tool.result` pairs and harvested
//! `solve.expr` handles (`solve.md` 4.1 & 9 row S8).

use crate::solve_state::extract_exprs_out;
use crate::proof_dag::tool_reference_matches;
use crate::solve_model::{event_kinds, HandleRef, ReceiptProcess, SolveExpr, SolveReceipt};
use ledger::{Event, Ledger};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

/// Enriched view over a single `tool.request`/`tool.result` pair in a Solve session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolReceiptRecord {
    pub call_id: String,
    pub tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_seq: Option<u64>,
    pub result_seq: u64,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wall_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<ReceiptProcess>,
    #[serde(default)]
    pub exprs_out: Vec<String>,
    #[serde(default)]
    pub harvested: Vec<HandleRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_blob: Option<String>,
    /// True when the result blob hash could not be read or verified (`00-overview.md` 3.4).
    #[serde(default)]
    pub result_missing: bool,
    /// True iff `ok`, payload decoded without missing blob, `value` is present, and `error` is null.
    pub sympy_succeeded: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoded_result: Option<Value>,
}

impl ToolReceiptRecord {
    /// Convert to the compact `SolveReceipt` shape used by `SolveState`.
    pub fn into_solve_receipt(&self) -> SolveReceipt {
        SolveReceipt {
            call_id: self.call_id.clone(),
            tool: self.tool.clone(),
            request_seq: self.request_seq,
            result_seq: self.result_seq,
            ok: self.ok,
            code_sha256: self.code_sha256.clone(),
            wall_ms: self.wall_ms,
            process: self.process,
            exprs_out: self.exprs_out.clone(),
            env: self.env.clone(),
            result_blob: self.result_blob.clone(),
        }
    }
}

struct PendingReq {
    tool: String,
    caller: Option<String>,
    seq: u64,
}

/// Build the pure `call_id -> ToolReceiptRecord` map over `events`, optionally reading
/// blob-stored `tool.result` payloads through `ledger` without ever failing on missing blobs
/// (`00-overview.md` 3.4).
pub fn build_receipts(
    events: &[Event],
    ledger: Option<&Ledger>,
) -> BTreeMap<String, ToolReceiptRecord> {
    let mut requests: HashMap<String, PendingReq> = HashMap::new();
    let mut receipts: BTreeMap<String, ToolReceiptRecord> = BTreeMap::new();
    let mut pending_exprs: Vec<(String, u64, HandleRef)> = Vec::new();

    for ev in events {
        let p = &ev.payload;
        match ev.kind.as_str() {
            event_kinds::TOOL_REQUEST => {
                if let (Some(call_id), Some(tool)) = (
                    p.get("call_id").and_then(Value::as_str),
                    p.get("tool").and_then(Value::as_str),
                ) {
                    requests.insert(
                        call_id.to_string(),
                        PendingReq {
                            tool: tool.to_string(),
                            caller: p.get("caller").and_then(Value::as_str).map(str::to_string),
                            seq: ev.seq,
                        },
                    );
                }
            }
            event_kinds::TOOL_RESULT => {
                let Some(call_id) = p.get("call_id").and_then(Value::as_str) else {
                    continue;
                };
                let ok = p.get("ok").and_then(Value::as_bool).unwrap_or(false);
                let req = requests.remove(call_id);
                let tool = req
                    .as_ref()
                    .map(|r| r.tool.clone())
                    .or_else(|| p.get("tool").and_then(Value::as_str).map(str::to_string))
                    .unwrap_or_default();
                let caller = req.as_ref().and_then(|r| r.caller.clone());
                let request_seq = req.map(|r| r.seq);

                let result = p.get("result");
                let result_blob = result
                    .and_then(|r| r.get("blob"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let (decoded_result, result_missing) = decode_result_payload(result, ledger);
                let meta = decoded_result.as_ref().unwrap_or(p);

                let code_sha256 = meta
                    .get("code_sha256")
                    .or_else(|| p.get("code_sha256"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let wall_ms = meta
                    .get("wall_ms")
                    .or_else(|| p.get("wall_ms"))
                    .and_then(Value::as_u64);
                let process: Option<ReceiptProcess> = meta
                    .get("process")
                    .or_else(|| p.get("process"))
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                let exprs_out = extract_exprs_out(meta);
                let env = meta
                    .get("env")
                    .or_else(|| p.get("env"))
                    .or_else(|| p.get("worker_execution").and_then(|w| w.get("environment")))
                    .cloned();

                let value = decoded_result.as_ref().and_then(|v| v.get("value")).cloned();
                let error = decoded_result
                    .as_ref()
                    .and_then(|v| v.get("error"))
                    .filter(|e| !e.is_null())
                    .and_then(|e| {
                        e.as_str()
                            .map(str::to_string)
                            .or_else(|| Some(e.to_string()))
                    })
                    .or_else(|| {
                        p.get("error")
                            .filter(|e| !e.is_null())
                            .and_then(|e| e.as_str().map(str::to_string))
                    });

                let sympy_succeeded = ok
                    && !result_missing
                    && match &decoded_result {
                        Some(v) => v.get("value").is_some() && v["error"].is_null(),
                        None => false,
                    };

                receipts.insert(
                    call_id.to_string(),
                    ToolReceiptRecord {
                        call_id: call_id.to_string(),
                        tool,
                        caller,
                        request_seq,
                        result_seq: ev.seq,
                        ok,
                        code_sha256,
                        wall_ms,
                        process,
                        exprs_out,
                        harvested: Vec::new(),
                        env,
                        result_blob,
                        result_missing,
                        sympy_succeeded,
                        value,
                        error,
                        decoded_result,
                    },
                );
            }
            event_kinds::SOLVE_EXPR => {
                if let Ok(expr) = serde_json::from_value::<SolveExpr>(p.clone()) {
                    if let Ok(handle) = expr.handle() {
                        pending_exprs.push((
                            expr.produced_by.call_id,
                            expr.produced_by.result_seq,
                            handle,
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    for (call_id, rseq, handle) in pending_exprs {
        if let Some(rec) = receipts.get_mut(&call_id) {
            if rseq == 0 || rec.result_seq == rseq {
                rec.harvested.push(handle);
            }
        }
    }

    receipts
}

/// Find a receipt matching `reference` (`call_id`, integer `seq`, `"seq:N"`, or `"call_id@seq:N"`).
pub fn find_receipt<'a>(
    receipts: &'a BTreeMap<String, ToolReceiptRecord>,
    reference: &str,
) -> Option<&'a ToolReceiptRecord> {
    let trimmed = reference.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(direct) = receipts.get(trimmed) {
        return Some(direct);
    }
    receipts
        .values()
        .find(|rec| tool_reference_matches(trimmed, &rec.call_id, rec.result_seq))
}

/// Resolve every `tool_call_ref` against `receipts`, failing closed on empty or unknown refs.
pub fn resolve_cited_receipts<'a>(
    receipts: &'a BTreeMap<String, ToolReceiptRecord>,
    refs: &[String],
) -> Result<Vec<&'a ToolReceiptRecord>, String> {
    let mut out = Vec::with_capacity(refs.len());
    for raw in refs {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err("`tool_call_refs` entries must not be empty".to_string());
        }
        let rec = find_receipt(receipts, trimmed)
            .ok_or_else(|| format!("`tool_call_refs` cites unknown tool call `{trimmed}`"))?;
        out.push(rec);
    }
    Ok(out)
}

/// True iff at least one resolved receipt is a verified successful `sympy_eval` execution.
pub fn has_successful_sympy_receipt(recs: &[&ToolReceiptRecord]) -> bool {
    recs.iter()
        .any(|r| r.ok && !r.result_missing && r.tool == "sympy_eval" && r.sympy_succeeded)
}

fn decode_result_payload(result: Option<&Value>, ledger: Option<&Ledger>) -> (Option<Value>, bool) {
    let Some(res) = result else {
        return (None, false);
    };
    if let Some(inline) = res.get("inline").and_then(Value::as_str) {
        return (serde_json::from_str::<Value>(inline).ok(), false);
    }
    if let Some(hash) = res.get("blob").and_then(Value::as_str) {
        let Some(ledger_ref) = ledger else {
            return (None, false);
        };
        return match ledger_ref.get_blob(hash) {
            Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                Ok(val) => (Some(val), false),
                Err(_) => (None, true),
            },
            Err(_) => (None, true),
        };
    }
    if res.is_object() {
        return (Some(res.clone()), false);
    }
    (None, false)
}

#[cfg(test)]
#[path = "solve_receipts_tests.rs"]
mod tests;
