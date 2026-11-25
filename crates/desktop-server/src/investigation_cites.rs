//! Checked line-citation extraction for Investigation question answers (P5).
//! Marks cited `[Lstart–end]` ranges as `read: true` only when the turn actually
//! received those lines for the pinned paper's `primary_path` via `tool.result`.

use crate::state::DesktopState;
use ledger::{Event, Ledger};
use research_tools::fs_tools::MAX_FS_READ_FILE_BYTES;
use research_tools::SandboxRoot;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::Read;

fn recorded_value(ledger: Option<&Ledger>, slot: &Value) -> Option<Value> {
    if let Some(text) = slot["inline"].as_str().or_else(|| slot.as_str()) {
        serde_json::from_str(text).ok()
    } else if let Some(hash) = slot["blob"].as_str() {
        let bytes = ledger?.get_blob(hash).ok()?;
        serde_json::from_slice(&bytes).ok()
    } else {
        slot.as_object().map(|_| slot.clone())
    }
}

fn push_range(out: &mut Vec<(usize, usize)>, start: Option<u64>, end: Option<u64>) {
    let (Some(s), Some(e)) = (start, end) else {
        return;
    };
    let (Ok(s), Ok(e)) = (usize::try_from(s), usize::try_from(e)) else {
        return;
    };
    if s >= 1 && s <= e {
        out.push((s, e));
    }
}

fn merge_ranges(mut ranges: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    if ranges.is_empty() {
        return ranges;
    }
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
    for (start, end) in ranges {
        if let Some(last) = merged.last_mut() {
            if last.1.saturating_add(1) >= start {
                if end > last.1 {
                    last.1 = end;
                }
                continue;
            }
        }
        merged.push((start, end));
    }
    merged
}

/// Collect line ranges received by the model for `primary_path` from `tool.result`
/// events recorded after `request_seq`, together with the maximum `total_lines`
/// reported by any matching `fs_read` result.
pub(crate) fn collect_received_ranges(
    ledger: Option<&Ledger>,
    events: &[Event],
    request_seq: u64,
    primary_path: &str,
) -> (Vec<(usize, usize)>, Option<usize>) {
    let mut requests: HashMap<&str, &str> = HashMap::new();
    for ev in events.iter().filter(|e| e.seq > request_seq && e.kind == "tool.request") {
        if let (Some(call_id), Some(tool)) = (
            ev.payload["call_id"].as_str(),
            ev.payload["tool"].as_str().or_else(|| ev.payload["tool_id"].as_str()),
        ) {
            requests.insert(call_id, tool);
        }
    }

    let mut ranges = Vec::new();
    let mut observed_total_lines: Option<usize> = None;

    for ev in events.iter().filter(|e| {
        e.seq > request_seq
            && e.kind == "tool.result"
            && e.payload["ok"].as_bool() == Some(true)
    }) {
        let tool = ev.payload["call_id"]
            .as_str()
            .and_then(|cid| requests.get(cid).copied());
        if let Some(name) = tool {
            if !matches!(name, "fs_read" | "search_paper_passages") {
                continue;
            }
        }
        let Some(val) = recorded_value(ledger, &ev.payload["result"]) else {
            continue;
        };

        if tool == Some("search_paper_passages")
            || (tool.is_none() && val["passages"].is_array())
        {
            if let Some(passages) = val["passages"].as_array() {
                for passage in passages {
                    if passage["path"].as_str() == Some(primary_path) {
                        push_range(
                            &mut ranges,
                            passage["start_line"].as_u64(),
                            passage["end_line"].as_u64(),
                        );
                    }
                }
            }
            continue;
        }

        let is_sources_root = val["root"].as_str().is_none_or(|r| r == "sources");
        if !is_sources_root || val["path"].as_str() != Some(primary_path) {
            continue;
        }
        if let Some(total) = val["total_lines"]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
        {
            observed_total_lines = Some(observed_total_lines.map_or(total, |prev| prev.max(total)));
        }
        if let Some(matches) = val["matches"].as_array() {
            for m in matches {
                let line = m["line"].as_u64();
                push_range(&mut ranges, line, line);
            }
        } else {
            push_range(
                &mut ranges,
                val["start_line"].as_u64(),
                val["end_line"].as_u64(),
            );
        }
    }

    (merge_ranges(ranges), observed_total_lines)
}

fn parse_one_cite_part(part: &str) -> Option<(usize, usize)> {
    let rest = part.strip_prefix('L')?;
    if rest.is_empty() {
        return None;
    }
    let sep_pos = rest.find(['-', '–']);
    match sep_pos {
        None => {
            if !rest.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let n = rest.parse::<usize>().ok()?;
            Some((n, n))
        }
        Some(pos) => {
            let left = &rest[..pos];
            let sep_len = rest[pos..].chars().next()?.len_utf8();
            let right = &rest[pos + sep_len..];
            if left.is_empty()
                || right.is_empty()
                || !left.bytes().all(|b| b.is_ascii_digit())
                || !right.bytes().all(|b| b.is_ascii_digit())
            {
                return None;
            }
            Some((left.parse::<usize>().ok()?, right.parse::<usize>().ok()?))
        }
    }
}

/// Parse `[L12]`, `[L3–5]`, and `[L3-5, L9]` groups from prose.
pub(crate) fn parse_line_citations(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            if let Some(rel_end) = text[i + 1..].find(']') {
                let inner = &text[i + 1..i + 1 + rel_end];
                if !inner.is_empty() && inner.starts_with('L') {
                    let mut group = Vec::new();
                    let mut valid = true;
                    for (idx, raw_part) in inner.split(',').enumerate() {
                        let part = if idx == 0 {
                            raw_part
                        } else {
                            raw_part.trim_start_matches(|c: char| c.is_ascii_whitespace())
                        };
                        if let Some(pair) = parse_one_cite_part(part) {
                            group.push(pair);
                        } else {
                            valid = false;
                            break;
                        }
                    }
                    if valid {
                        out.extend(group);
                        i += rel_end + 2;
                        continue;
                    }
                }
            }
        }
        i += text[i..].chars().next().map_or(1, char::len_utf8);
    }
    out
}

/// Build the `cites` array (`[{start, end, read}]`) stored on `investigation.answer`.
/// Keeps only ranges with `1 <= start <= end <= total_lines` (when `total_lines` is known).
pub(crate) fn answer_cites(
    ledger: Option<&Ledger>,
    events: &[Event],
    request_seq: u64,
    primary_path: &str,
    total_lines: Option<usize>,
    answer_text: &str,
) -> Vec<Value> {
    let (received, observed_total) =
        collect_received_ranges(ledger, events, request_seq, primary_path);
    let max_lines = total_lines.or(observed_total);
    let mut seen = HashSet::new();
    let mut cites = Vec::new();
    for (start, end) in parse_line_citations(answer_text) {
        if start < 1 || start > end {
            continue;
        }
        if let Some(limit) = max_lines {
            if end > limit {
                continue;
            }
        }
        if !seen.insert((start, end)) {
            continue;
        }
        let read = received
            .iter()
            .any(|&(rs, re)| rs <= start && end <= re);
        cites.push(json!({ "start": start, "end": end, "read": read }));
    }
    cites
}

pub(crate) fn source_total_lines(
    state: &DesktopState,
    session: &str,
    primary_path: &str,
) -> Option<usize> {
    let env = state.research_env_for_session(session).ok()?;
    let file = env
        .sandbox()
        .resolve_existing(SandboxRoot::Sources, session, primary_path, false)
        .ok()?;
    if !file.metadata.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(file.canonical_target)
        .ok()?
        .take(MAX_FS_READ_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_FS_READ_FILE_BYTES {
        return None;
    }
    let text = std::str::from_utf8(&bytes).ok()?;
    Some(if text.is_empty() {
        0
    } else {
        text.lines().count()
    })
}

pub(crate) fn answer_cites_for_session(
    state: &DesktopState,
    ledger: &Ledger,
    session: &str,
    events: &[Event],
    request_seq: u64,
    answer_text: &str,
) -> Vec<Value> {
    let source_id = events
        .iter()
        .rev()
        .find(|e| e.kind == "investigation.paper")
        .and_then(|e| e.payload["source_id"].as_str());
    let Some(source_id) = source_id else {
        return Vec::new();
    };
    let primary_path = events
        .iter()
        .find(|e| e.kind == "source.ingested" && e.payload["source_id"].as_str() == Some(source_id))
        .and_then(|e| e.payload["primary_path"].as_str());
    let Some(primary_path) = primary_path else {
        return Vec::new();
    };
    let total_lines = source_total_lines(state, session, primary_path);
    answer_cites(
        Some(ledger),
        events,
        request_seq,
        primary_path,
        total_lines,
        answer_text,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(seq: u64, kind: &str, payload: Value) -> Event {
        Event {
            session: "s1".to_string(),
            seq,
            ts_ms: (seq * 1000) as i64,
            kind: kind.to_string(),
            schema_version: 1,
            payload,
            prev_hash: String::new(),
            event_hash: String::new(),
        }
    }

    #[test]
    fn cites_marked_read_only_inside_received_ranges() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ledger = Ledger::open(dir.path()).expect("open ledger");
        let path = "src-1/paper.tex";
        let blob_payload = serde_json::to_vec(&json!({
            "root": "sources",
            "path": path,
            "start_line": 21,
            "end_line": 30,
            "total_lines": 100
        }))
        .unwrap();
        let blob_hash = ledger.put_blob(&blob_payload).expect("put_blob");

        let events = vec![
            // Prior turn read (before request_seq=10) must be ignored.
            ev(5, "tool.request", json!({"call_id": "old", "tool": "fs_read"})),
            ev(
                6,
                "tool.result",
                json!({"call_id": "old", "ok": true, "result": {"inline": serde_json::to_string(&json!({
                    "root": "sources", "path": path, "start_line": 1, "end_line": 5, "total_lines": 100
                })).unwrap()}}),
            ),
            ev(10, "investigation.question_requested", json!({"source_id": "src-1"})),
            // Workspace-root fs_read with same relative path must be ignored.
            ev(11, "tool.request", json!({"call_id": "ws", "tool": "fs_read"})),
            ev(
                12,
                "tool.result",
                json!({"call_id": "ws", "ok": true, "result": {"inline": serde_json::to_string(&json!({
                    "root": "workspace", "path": path, "start_line": 80, "end_line": 90, "total_lines": 100
                })).unwrap()}}),
            ),
            // Offset-mode fs_read (inline) lines 10..20 + blob lines 21..30 -> merged 10..30.
            ev(13, "tool.request", json!({"call_id": "r1", "tool": "fs_read"})),
            ev(
                14,
                "tool.result",
                json!({"call_id": "r1", "ok": true, "result": {"inline": serde_json::to_string(&json!({
                    "root": "sources", "path": path, "start_line": 10, "end_line": 20, "total_lines": 100
                })).unwrap()}}),
            ),
            ev(15, "tool.request", json!({"call_id": "r2", "tool": "fs_read"})),
            ev(
                16,
                "tool.result",
                json!({"call_id": "r2", "ok": true, "result": {"blob": blob_hash}}),
            ),
            // Grep-mode fs_read: lines 40 and 42 (gap at 41!).
            ev(17, "tool.request", json!({"call_id": "g1", "tool": "fs_read"})),
            ev(
                18,
                "tool.result",
                json!({"call_id": "g1", "ok": true, "result": {"inline": serde_json::to_string(&json!({
                    "root": "sources", "path": path, "matches": [
                        {"line": 40, "text": "hit a"},
                        {"line": 42, "text": "hit b"}
                    ], "total_lines": 100
                })).unwrap()}}),
            ),
            // search_paper_passages: lines 60..65 on primary_path, 70..75 on another paper.
            ev(19, "tool.request", json!({"call_id": "s1", "tool": "search_paper_passages"})),
            ev(
                20,
                "tool.result",
                json!({"call_id": "s1", "ok": true, "result": {"inline": serde_json::to_string(&json!({
                    "passages": [
                        {"path": path, "start_line": 60, "end_line": 65},
                        {"path": "src-2/other.tex", "start_line": 70, "end_line": 75}
                    ]
                })).unwrap()}}),
            ),
        ];

        let answer = concat!(
            "Offset [L12] and merged [L18–25]. ",
            "Grep [L40, L42] vs gap [L40-42]. ",
            "Search [L61-64] vs other paper [L72]. ",
            "Prior turn [L3], workspace [L85], unread [L50]. ",
            "Invalid: out of range [L101], reversed [L20-15], zero [L0]."
        );

        let cites = answer_cites(Some(&ledger), &events, 10, path, Some(100), answer);
        assert_eq!(
            cites,
            vec![
                json!({"start": 12, "end": 12, "read": true}),
                json!({"start": 18, "end": 25, "read": true}),
                json!({"start": 40, "end": 40, "read": true}),
                json!({"start": 42, "end": 42, "read": true}),
                json!({"start": 40, "end": 42, "read": false}),
                json!({"start": 61, "end": 64, "read": true}),
                json!({"start": 72, "end": 72, "read": false}),
                json!({"start": 3, "end": 3, "read": false}),
                json!({"start": 85, "end": 85, "read": false}),
                json!({"start": 50, "end": 50, "read": false}),
            ]
        );
    }
}
