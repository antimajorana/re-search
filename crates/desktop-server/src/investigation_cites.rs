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

const MAX_CLICKABLE_CITE_SPAN: usize = 512;

fn is_main_agent_caller(caller: &str) -> bool {
    caller.starts_with("agent:")
        || caller.starts_with("claude-code-cli:")
        || caller.starts_with("antigravity:")
}

/// Collect line ranges received by the model for `primary_path` from `tool.result`
/// events recorded after `request_seq`, together with the maximum `total_lines`
/// reported by any matching `fs_read` or `read_paper_lines` result.
pub(crate) fn collect_received_ranges(
    ledger: Option<&Ledger>,
    events: &[Event],
    request_seq: u64,
    primary_path: &str,
) -> (Vec<(usize, usize)>, Option<usize>) {
    let mut requests: HashMap<&str, (&str, Option<&Value>)> = HashMap::new();
    for ev in events.iter().filter(|e| e.seq > request_seq && e.kind == "tool.request") {
        if let (Some(call_id), Some(tool)) = (
            ev.payload["call_id"].as_str(),
            ev.payload["tool"].as_str().or_else(|| ev.payload["tool_id"].as_str()),
        ) {
            requests.insert(call_id, (tool, ev.payload.get("caller")));
        }
    }

    let mut ranges = Vec::new();
    let mut observed_total_lines: Option<usize> = None;

    for ev in events.iter().filter(|e| {
        e.seq > request_seq
            && e.kind == "tool.result"
            && e.payload["ok"].as_bool() == Some(true)
    }) {
        let Some((tool, caller)) = ev.payload["call_id"]
            .as_str()
            .and_then(|cid| requests.get(cid).copied())
        else {
            continue;
        };
        if !caller.is_none_or(|v| v.as_str().is_some_and(is_main_agent_caller)) {
            continue;
        }
        if !matches!(tool, "fs_read" | "read_paper_lines" | "search_paper_passages") {
            continue;
        }
        let Some(val) = recorded_value(ledger, &ev.payload["result"]) else {
            continue;
        };

        match tool {
            "search_paper_passages" => {
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
            }
            "read_paper_lines" => {
                let meta = &val["a_meta"];
                if meta["path"].as_str() != Some(primary_path) {
                    continue;
                }
                if let Some(total) = meta["total_lines"]
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                {
                    observed_total_lines =
                        Some(observed_total_lines.map_or(total, |prev| prev.max(total)));
                }
                if meta["line_truncated"].as_bool() != Some(true) {
                    push_range(
                        &mut ranges,
                        meta["start_line"].as_u64(),
                        meta["end_line"].as_u64(),
                    );
                }
            }
            "fs_read" => {
                let is_sources_root = val["root"].as_str().is_none_or(|r| r == "sources");
                if !is_sources_root || val["path"].as_str() != Some(primary_path) {
                    continue;
                }
                if let Some(total) = val["total_lines"]
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                {
                    observed_total_lines =
                        Some(observed_total_lines.map_or(total, |prev| prev.max(total)));
                }
                if let Some(matches) = val["matches"].as_array() {
                    for m in matches {
                        let line = m["line"].as_u64();
                        push_range(&mut ranges, line, line);
                    }
                } else {
                    if val["returned_chars"].as_u64() == Some(0)
                        || val["content"].as_str().is_some_and(str::is_empty)
                    {
                        continue;
                    }
                    let mut start = val["start_line"].as_u64();
                    let mut end = val["end_line"].as_u64();
                    if val["offset"].as_u64().is_some_and(|o| o > 0) {
                        start = start.and_then(|s| s.checked_add(1));
                    }
                    if val["truncated"].as_bool() == Some(true)
                        && !val["content"].as_str().is_some_and(|s| s.ends_with('\n'))
                    {
                        end = end.map(|e| e.saturating_sub(1));
                    }
                    push_range(&mut ranges, start, end);
                }
            }
            _ => {}
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
        let read = end.saturating_sub(start) <= MAX_CLICKABLE_CITE_SPAN
            && received.iter().any(|&(rs, re)| rs <= start && end <= re);
        cites.push(json!({ "start": start, "end": end, "read": read }));
    }
    cites
}

pub(crate) fn source_total_lines(
    state: &DesktopState,
    session: &str,
    primary_path: &str,
    expected_sha: Option<&str>,
) -> Option<usize> {
    let expected_sha = expected_sha.filter(|s| !s.is_empty())?;
    let env = state.research_env_for_session(session).ok()?;
    let file = env
        .sandbox()
        .resolve_existing(SandboxRoot::Sources, session, primary_path, false)
        .ok()?;
    if !file.metadata.is_file() || file.rel_path != primary_path {
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
    if ledger::sha256_hex(&bytes) != expected_sha {
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
    let source_ev = events
        .iter()
        .find(|e| e.kind == "source.ingested" && e.payload["source_id"].as_str() == Some(source_id));
    let Some(source_ev) = source_ev else {
        return Vec::new();
    };
    let Some(primary_path) = source_ev.payload["primary_path"].as_str() else {
        return Vec::new();
    };
    let expected_sha = source_ev.payload["files_sha256"]
        .get(primary_path)
        .and_then(Value::as_str);
    let total_lines = source_total_lines(state, session, primary_path, expected_sha);
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

    #[test]
    fn read_paper_lines_marks_cites_read() {
        let path = "src-1/paper.txt";
        let events = vec![
            ev(10, "investigation.question_requested", json!({"source_id": "src-1"})),
            ev(11, "tool.request", json!({"call_id": "r1", "tool": "read_paper_lines"})),
            ev(
                12,
                "tool.result",
                json!({"call_id": "r1", "ok": true, "result": {"inline": serde_json::to_string(&json!({
                    "a_meta": {
                        "path": path,
                        "start_line": 15,
                        "end_line": 28,
                        "total_lines": 50
                    },
                    "text": "15\tline"
                })).unwrap()}}),
            ),
        ];
        let cites = answer_cites(
            None,
            &events,
            10,
            path,
            None,
            "Read [L16-20], unread [L30], out of bounds [L55].",
        );
        assert_eq!(
            cites,
            vec![
                json!({"start": 16, "end": 20, "read": true}),
                json!({"start": 30, "end": 30, "read": false}),
            ]
        );
    }

    #[test]
    fn caller_allowlist_and_unmatched_results() {
        let path = "src-1/paper.txt";
        let res = |cid: &str, s: u64, e: u64| {
            ev(20, "tool.result", json!({"call_id": cid, "ok": true, "result": {"inline":
                serde_json::to_string(&json!({"root":"sources","path":path,"start_line":s,"end_line":e,"total_lines":100})).unwrap()}}))
        };
        let events = vec![
            ev(11, "tool.request", json!({"call_id": "w", "tool": "fs_read", "caller": "worker:lit"})),
            res("w", 1, 5),
            ev(12, "tool.request", json!({"call_id": "u", "tool": "fs_read", "caller": "user:canvas"})),
            res("u", 6, 10),
            ev(13, "tool.request", json!({"call_id": "e", "tool": "fs_read", "caller": ""})),
            res("e", 11, 15),
            ev(14, "tool.request", json!({"call_id": "n", "tool": "fs_read", "caller": 42})),
            res("n", 16, 20),
            res("unmatched", 21, 25),
            ev(15, "tool.request", json!({"call_id": "a", "tool": "fs_read", "caller": "agent:s1"})),
            res("a", 30, 32),
            ev(16, "tool.request", json!({"call_id": "c", "tool": "fs_read", "caller": "claude-code-cli:s1"})),
            res("c", 40, 42),
            ev(17, "tool.request", json!({"call_id": "g", "tool": "fs_read", "caller": "antigravity:s1"})),
            res("g", 50, 52),
        ];
        let cites = answer_cites(
            None, &events, 10, path, Some(100),
            "[L2] [L7] [L12] [L17] [L22] [L31] [L41] [L51]",
        );
        let flags: Vec<bool> = cites.iter().map(|c| c["read"].as_bool().unwrap()).collect();
        assert_eq!(flags, vec![false, false, false, false, false, true, true, true]);
    }

    #[test]
    fn truncated_empty_and_partial_reads_excluded() {
        let path = "src-1/paper.txt";
        let events = vec![
            ev(11, "tool.request", json!({"call_id": "rpl", "tool": "read_paper_lines", "caller": "agent:s1"})),
            ev(12, "tool.result", json!({"call_id": "rpl", "ok": true, "result": {"inline": serde_json::to_string(&json!({
                "a_meta": {"path": path, "start_line": 1, "end_line": 1, "total_lines": 30, "line_truncated": true}
            })).unwrap()}})),
            ev(13, "tool.request", json!({"call_id": "eof", "tool": "fs_read", "caller": "agent:s1"})),
            ev(14, "tool.result", json!({"call_id": "eof", "ok": true, "result": {"inline": serde_json::to_string(&json!({
                "root": "sources", "path": path, "start_line": 30, "end_line": 30, "returned_chars": 0, "content": ""
            })).unwrap()}})),
            ev(15, "tool.request", json!({"call_id": "mid", "tool": "fs_read", "caller": "agent:s1"})),
            ev(16, "tool.result", json!({"call_id": "mid", "ok": true, "result": {"inline": serde_json::to_string(&json!({
                "root": "sources", "path": path, "start_line": 5, "end_line": 8, "offset": 12, "truncated": true, "content": "tail\n6\n7\nhead"
            })).unwrap()}})),
            ev(17, "tool.request", json!({"call_id": "nl", "tool": "fs_read", "caller": "agent:s1"})),
            ev(18, "tool.result", json!({"call_id": "nl", "ok": true, "result": {"inline": serde_json::to_string(&json!({
                "root": "sources", "path": path, "start_line": 10, "end_line": 12, "offset": 0, "truncated": true, "content": "10\n11\n12\n"
            })).unwrap()}})),
        ];
        let cites = answer_cites(
            None, &events, 10, path, None,
            "[L1] [L30] [L5] [L6-7] [L8] [L10-12] [L31]",
        );
        let got: Vec<(u64, u64, bool)> = cites
            .iter()
            .map(|c| (c["start"].as_u64().unwrap(), c["end"].as_u64().unwrap(), c["read"].as_bool().unwrap()))
            .collect();
        assert_eq!(got, vec![(1, 1, false), (30, 30, false), (5, 5, false), (6, 7, true), (8, 8, false), (10, 12, true)]);
    }

    #[test]
    fn wide_citation_above_513_lines_stays_unread() {
        let path = "src-1/wide.txt";
        let events = vec![
            ev(11, "tool.request", json!({"call_id": "a", "tool": "read_paper_lines", "caller": "agent:s1"})),
            ev(12, "tool.result", json!({"call_id": "a", "ok": true, "result": {"inline": serde_json::to_string(&json!({
                "a_meta": {"path": path, "start_line": 1, "end_line": 600, "total_lines": 600}
            })).unwrap()}})),
        ];
        let cites = answer_cites(None, &events, 10, path, Some(600), "[L1-513] [L1-514]");
        assert_eq!(cites, vec![json!({"start": 1, "end": 513, "read": true}), json!({"start": 1, "end": 514, "read": false})]);
    }
}
