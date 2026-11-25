//! Ledger derivation of which source lines were shown to a caller (package F4).
//!
//! Combines host-recorded `context.supplied` events with successful read-tool results
//! (`read_paper_lines`, `search_paper_passages`, `fs_read`, and a reserved placeholder for
//! `read_map_node`) into merged, sorted 1-based line ranges keyed by
//! `(caller, source_id, path, file_sha256)`.

use crate::investigation::anchor::valid_id;
use ledger::{Event, Ledger};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Provisional shared event kind recording source line ranges shown to a model outside tool calls
/// (owner decision 10 in `docs/design/modes-2026-11/00-overview.md`, section 8).
pub const CONTEXT_SUPPLIED_EVENT: &str = "context.supplied";

/// Reserved tool name placeholder for Investigate package V6 (`read_map_node`).
pub const READ_MAP_NODE_TOOL: &str = "read_map_node";

const MAX_FS_READ_MATCH_CHARS: usize = 500;
const PASSAGE_UNTERMINATED_MIN_BYTES: usize = 900;

/// Inclusive 1-based line range within a single source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LineRange {
    pub start_line: usize,
    pub end_line: usize,
}

/// One hash-pinned source file slice recorded inside a [`ContextSuppliedPayload`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuppliedRange {
    pub source_id: String,
    pub path: String,
    pub file_sha256: String,
    pub start_line: usize,
    pub end_line: usize,
}

/// Payload of a [`CONTEXT_SUPPLIED_EVENT`] (`context.supplied`) ledger event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextSuppliedPayload {
    pub run_id: String,
    pub caller: String,
    pub ranges: Vec<SuppliedRange>,
}

impl ContextSuppliedPayload {
    /// Construct and validate a `context.supplied` payload.
    pub fn new(
        run_id: impl Into<String>,
        caller: impl Into<String>,
        ranges: Vec<SuppliedRange>,
    ) -> Result<Self, String> {
        let payload = Self { run_id: run_id.into(), caller: caller.into(), ranges };
        payload.validate()?;
        Ok(payload)
    }

    /// Validate all fields of a `context.supplied` payload.
    pub fn validate(&self) -> Result<(), String> {
        if self.run_id.trim().is_empty() {
            return Err("context.supplied requires a non-empty run_id".into());
        }
        if self.caller.trim().is_empty() {
            return Err("context.supplied requires a non-empty caller".into());
        }
        for (idx, r) in self.ranges.iter().enumerate() {
            if !valid_id(&r.source_id, 64) {
                return Err(format!("ranges[{idx}] has invalid source_id"));
            }
            if r.path.is_empty()
                || r.path.len() > 1024
                || r.path.split_once('/').map(|(id, _)| id) != Some(r.source_id.as_str())
            {
                return Err(format!(
                    "ranges[{idx}] path must be non-empty, <= 1024 bytes, and belong to source_id"
                ));
            }
            if !is_sha256_hex(&r.file_sha256) {
                return Err(format!("ranges[{idx}] file_sha256 must be 64 lower-case hex characters"));
            }
            if r.start_line == 0 || r.end_line < r.start_line {
                return Err(format!("ranges[{idx}] must satisfy 1 <= start_line <= end_line"));
            }
        }
        Ok(())
    }
}

/// Lookup key identifying a specific caller and hash-pinned source file.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SentFileKey {
    pub caller: String,
    pub source_id: String,
    pub path: String,
    pub file_sha256: String,
}

/// Merged, sorted 1-based line ranges sent to each `(caller, source_id, path, file_sha256)`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SentLines {
    entries: BTreeMap<SentFileKey, Vec<LineRange>>,
}

impl SentLines {
    /// Return the sorted, merged line ranges sent to `caller` for the given file revision.
    pub fn ranges_for(
        &self,
        caller: &str,
        source_id: &str,
        path: &str,
        file_sha256: &str,
    ) -> &[LineRange] {
        let key = SentFileKey {
            caller: caller.to_string(),
            source_id: source_id.to_string(),
            path: path.to_string(),
            file_sha256: file_sha256.to_string(),
        };
        self.entries.get(&key).map_or(&[], Vec::as_slice)
    }

    /// Return `true` iff every line in `start_line..=end_line` was sent to `caller` for the exact
    /// `(source_id, path, file_sha256)`.
    pub fn covers(
        &self,
        caller: &str,
        source_id: &str,
        path: &str,
        file_sha256: &str,
        start_line: usize,
        end_line: usize,
    ) -> bool {
        if start_line == 0 || end_line < start_line {
            return false;
        }
        self.ranges_for(caller, source_id, path, file_sha256)
            .iter()
            .any(|r| r.start_line <= start_line && end_line <= r.end_line)
    }

    /// Inspect all credited file keys and their merged line ranges.
    pub fn all_entries(&self) -> &BTreeMap<SentFileKey, Vec<LineRange>> { &self.entries }

    /// Return `true` when no line ranges have been credited.
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }

    /// Sort `ranges` and merge all overlapping and adjacent (`last.end_line + 1 >= next.start_line`)
    /// valid 1-based ranges.
    pub fn merge_ranges(mut ranges: Vec<LineRange>) -> Vec<LineRange> {
        ranges.retain(|r| r.start_line >= 1 && r.start_line <= r.end_line);
        ranges.sort();
        let mut merged: Vec<LineRange> = Vec::with_capacity(ranges.len());
        for range in ranges {
            if let Some(last) = merged.last_mut() {
                if last.end_line.saturating_add(1) >= range.start_line {
                    last.end_line = last.end_line.max(range.end_line);
                    continue;
                }
            }
            merged.push(range);
        }
        merged
    }
}

/// Derive which source lines were sent to each caller from inline ledger events alone.
pub fn lines_sent(events: &[Event], run_id: Option<&str>) -> SentLines {
    lines_sent_with_ledger(events, None, run_id)
}

/// Derive which source lines were sent to each caller, optionally resolving blob-backed tool
/// requests and results through `ledger`.
pub fn lines_sent_with_ledger(
    events: &[Event],
    ledger: Option<&Ledger>,
    run_id: Option<&str>,
) -> SentLines {
    let mut requests: BTreeMap<&str, ToolRequestInfo<'_>> = BTreeMap::new();
    let mut raw: BTreeMap<SentFileKey, Vec<LineRange>> = BTreeMap::new();

    for event in events {
        match event.kind.as_str() {
            CONTEXT_SUPPLIED_EVENT => credit_context_supplied(&event.payload, run_id, &mut raw),
            "tool.request" => record_tool_request(&event.payload, &mut requests),
            "tool.result" => credit_tool_result(&event.payload, &requests, ledger, run_id, &mut raw),
            _ => {}
        }
    }

    let entries = raw
        .into_iter()
        .filter_map(|(key, ranges)| {
            let merged = SentLines::merge_ranges(ranges);
            (!merged.is_empty()).then_some((key, merged))
        })
        .collect();
    SentLines { entries }
}

pub(crate) fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

enum ReqRunId { Absent, Present(String), Invalid }

struct ToolRequestInfo<'a> {
    tool: &'a str,
    caller: &'a str,
    run_id: ReqRunId,
    args_slot: Option<&'a Value>,
}

fn value_run_id(value: &Value) -> Option<String> {
    if let Some(s) = value.as_str() {
        return (!s.is_empty()).then(|| s.to_string());
    }
    value.as_u64().map(|n| n.to_string())
}

fn json_usize(value: Option<&Value>) -> Option<usize> {
    value.and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok())
}

fn decode_payload_slot(slot: &Value, ledger: Option<&Ledger>) -> Option<Value> {
    if let Some(inline) = slot.get("inline").and_then(Value::as_str) {
        return serde_json::from_str(inline).ok();
    }
    if let Some(blob_hash) = slot.get("blob").and_then(Value::as_str) {
        let bytes = ledger?.get_blob(blob_hash).ok()?;
        return serde_json::from_slice(&bytes).ok();
    }
    slot.is_object().then(|| slot.clone())
}

fn push_range(
    acc: &mut BTreeMap<SentFileKey, Vec<LineRange>>,
    caller: &str,
    source_id: &str,
    path: &str,
    file_sha256: &str,
    start_line: usize,
    end_line: usize,
) {
    if caller.is_empty()
        || source_id.is_empty()
        || path.is_empty()
        || !is_sha256_hex(file_sha256)
        || start_line == 0
        || end_line < start_line
    {
        return;
    }
    let key = SentFileKey {
        caller: caller.to_string(),
        source_id: source_id.to_string(),
        path: path.to_string(),
        file_sha256: file_sha256.to_string(),
    };
    acc.entry(key).or_default().push(LineRange { start_line, end_line });
}

fn credit_context_supplied(
    payload: &Value,
    run_id: Option<&str>,
    acc: &mut BTreeMap<SentFileKey, Vec<LineRange>>,
) {
    if let Some(want) = run_id {
        let Some(have) = payload.get("run_id").and_then(value_run_id) else { return };
        if have != want { return; }
    }
    let Some(caller) = payload.get("caller").and_then(Value::as_str).filter(|s| !s.is_empty()) else {
        return;
    };
    let Some(ranges) = payload.get("ranges").and_then(Value::as_array) else { return };
    for item in ranges {
        let (Some(source_id), Some(path), Some(file_sha256)) = (
            item.get("source_id").and_then(Value::as_str),
            item.get("path").and_then(Value::as_str),
            item.get("file_sha256").and_then(Value::as_str),
        ) else { continue };
        let (Some(start_line), Some(end_line)) =
            (json_usize(item.get("start_line")), json_usize(item.get("end_line")))
        else { continue };
        push_range(acc, caller, source_id, path, file_sha256, start_line, end_line);
    }
}

fn record_tool_request<'a>(
    payload: &'a Value,
    requests: &mut BTreeMap<&'a str, ToolRequestInfo<'a>>,
) {
    let Some(call_id) = payload.get("call_id").and_then(Value::as_str).filter(|s| !s.is_empty()) else {
        return;
    };
    let Some(tool) = payload
        .get("tool")
        .or_else(|| payload.get("tool_id"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    else { return };
    let Some(caller) = payload.get("caller").and_then(Value::as_str).filter(|s| !s.is_empty()) else {
        return;
    };
    let run_id = match payload.get("run_id") {
        None | Some(Value::Null) => ReqRunId::Absent,
        Some(v) => value_run_id(v).map_or(ReqRunId::Invalid, ReqRunId::Present),
    };
    requests.insert(call_id, ToolRequestInfo { tool, caller, run_id, args_slot: payload.get("args") });
}

fn credit_tool_result(
    payload: &Value,
    requests: &BTreeMap<&str, ToolRequestInfo<'_>>,
    ledger: Option<&Ledger>,
    run_id: Option<&str>,
    acc: &mut BTreeMap<SentFileKey, Vec<LineRange>>,
) {
    if payload.get("ok").and_then(Value::as_bool) != Some(true) {
        return;
    }
    let Some(req) = payload.get("call_id").and_then(Value::as_str).and_then(|id| requests.get(id)) else {
        return;
    };
    if let Some(want) = run_id {
        match &req.run_id {
            ReqRunId::Absent => {}
            ReqRunId::Present(have) if have == want => {}
            ReqRunId::Present(_) | ReqRunId::Invalid => return,
        }
    }
    let Some(result) = payload.get("result").and_then(|s| decode_payload_slot(s, ledger)) else {
        return;
    };
    match req.tool {
        "read_paper_lines" => credit_read_paper_lines(req, &result, ledger, acc),
        "search_paper_passages" => credit_search_paper_passages(req.caller, &result, acc),
        "fs_read" => credit_fs_read(req.caller, &result, acc),
        READ_MAP_NODE_TOOL => credit_read_map_node_result(req.caller, &result, acc),
        _ => {}
    }
}

fn credit_read_paper_lines(
    req: &ToolRequestInfo<'_>,
    result: &Value,
    ledger: Option<&Ledger>,
    acc: &mut BTreeMap<SentFileKey, Vec<LineRange>>,
) {
    let Some(meta) = result.get("a_meta") else { return };
    let (Some(path), Some(file_sha256), Some(start_line), Some(mut end_line)) = (
        meta.get("path").and_then(Value::as_str).filter(|s| !s.is_empty()),
        meta.get("file_sha256").and_then(Value::as_str),
        json_usize(meta.get("start_line")),
        json_usize(meta.get("end_line")),
    ) else { return };
    if meta.get("line_truncated").and_then(Value::as_bool) == Some(true) {
        end_line = end_line.saturating_sub(1);
    }
    if start_line == 0 || start_line > end_line {
        return;
    }
    let source_id = meta
        .get("source_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            req.args_slot
                .and_then(|slot| decode_payload_slot(slot, ledger))
                .and_then(|args| args.get("source_id").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string))
        })
        .or_else(|| path.split_once('/').map(|(id, _)| id.to_string()).filter(|s| !s.is_empty()));
    if let Some(source_id) = source_id {
        push_range(acc, req.caller, &source_id, path, file_sha256, start_line, end_line);
    }
}

fn credit_search_paper_passages(
    caller: &str,
    result: &Value,
    acc: &mut BTreeMap<SentFileKey, Vec<LineRange>>,
) {
    let Some(passages) = result.get("passages").and_then(Value::as_array) else { return };
    for passage in passages {
        let (Some(source_id), Some(path), Some(file_sha256)) = (
            passage.get("source_id").and_then(Value::as_str),
            passage.get("path").and_then(Value::as_str),
            passage.get("file_sha256").and_then(Value::as_str),
        ) else { continue };
        let (Some(mut start_line), Some(mut end_line)) =
            (json_usize(passage.get("start_line")), json_usize(passage.get("end_line")))
        else { continue };
        if start_line == 1 && json_usize(passage.get("start_byte")).unwrap_or(0) > 0 {
            start_line = start_line.saturating_add(1);
        }
        if let Some(excerpt) = passage.get("excerpt").and_then(Value::as_str) {
            if !excerpt.ends_with('\n') && excerpt.len() >= PASSAGE_UNTERMINATED_MIN_BYTES {
                end_line = end_line.saturating_sub(1);
            }
        }
        push_range(acc, caller, source_id, path, file_sha256, start_line, end_line);
    }
}

fn credit_fs_read(caller: &str, result: &Value, acc: &mut BTreeMap<SentFileKey, Vec<LineRange>>) {
    match result.get("root") {
        None | Some(Value::Null) => {}
        Some(Value::String(root)) if root == "sources" => {}
        _ => return,
    }
    let Some(path) = result.get("path").and_then(Value::as_str).filter(|s| !s.is_empty()) else {
        return;
    };
    let Some(file_sha256) = result.get("file_sha256").and_then(Value::as_str).filter(|s| is_sha256_hex(s)) else {
        return;
    };
    let Some(source_id) = result
        .get("source_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| path.split_once('/').map(|(id, _)| id).filter(|s| !s.is_empty()))
    else { return };

    if let Some(matches) = result.get("matches").and_then(Value::as_array) {
        for item in matches {
            let Some(line) = json_usize(item.get("line")).filter(|&n| n >= 1) else { continue };
            let text = item.get("text").and_then(Value::as_str).unwrap_or("");
            let clipped = text.chars().count() > MAX_FS_READ_MATCH_CHARS
                && (text.starts_with("...") || text.ends_with("..."));
            if !clipped {
                push_range(acc, caller, source_id, path, file_sha256, line, line);
            }
        }
        return;
    }

    let returned_zero = result.get("returned_chars").and_then(Value::as_u64) == Some(0);
    let content_non_empty = result.get("content").and_then(Value::as_str).is_some_and(|c| !c.is_empty());
    if !returned_zero && content_non_empty {
        if let (Some(start_line), Some(end_line)) =
            (json_usize(result.get("start_line")), json_usize(result.get("end_line")))
        {
            push_range(acc, caller, source_id, path, file_sha256, start_line, end_line);
        }
    }
}

/// Placeholder for crediting line ranges returned by `read_map_node`.
///
/// Investigate package V6 will populate this function once `read_map_node`'s result schema lands.
fn credit_read_map_node_result(
    _caller: &str,
    _result: &Value,
    _acc: &mut BTreeMap<SentFileKey, Vec<LineRange>>,
) {
}

#[cfg(test)]
#[path = "lines_sent_tests.rs"]
mod tests;
