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

fn caller_allowed(caller: Option<&Value>) -> bool {
    match caller {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => !s.starts_with("worker:") && !s.starts_with("user:"),
        Some(_) => false,
    }
}

fn push_passage_range(
    out: &mut Vec<(usize, usize)>,
    passage: &Value,
    source_bytes: Option<&[u8]>,
    total_lines: Option<usize>,
) {
    let (Some(s), Some(e)) = (
        passage["start_line"].as_u64(),
        passage["end_line"].as_u64(),
    ) else {
        return;
    };
    let (Ok(mut a), Ok(mut b)) = (usize::try_from(s), usize::try_from(e)) else {
        return;
    };
    if a < 1 || a > b {
        return;
    }
    if let Some(excerpt) = passage["excerpt"].as_str() {
        let start_byte = passage["start_byte"]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok());
        let end_byte = passage["end_byte"]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok());
        if let (Some(bytes), Some(sb), Some(eb)) = (source_bytes, start_byte, end_byte) {
            if sb > 0 && bytes.get(sb - 1) != Some(&b'\n') {
                a = a.saturating_add(1);
            }
            if eb < bytes.len() && eb > 0 && bytes.get(eb - 1) != Some(&b'\n') {
                b = b.saturating_sub(1);
            }
        } else {
            if start_byte.is_some_and(|sb| sb > 0) {
                a = a.saturating_add(1);
            }
            if !excerpt.ends_with('\n')
                && (total_lines.is_some_and(|t| b < t)
                    || (total_lines.is_none() && excerpt.len() >= 900))
            {
                b = b.saturating_sub(1);
            }
        }
    }
    if a >= 1 && a <= b {
        out.push((a, b));
    }
}

/// Collect line ranges received by the model for `primary_path` from `tool.result` and
/// `context.supplied` events recorded after `request_seq`, together with the maximum `total_lines`
/// reported by any matching `fs_read` or `read_paper_lines` result.
pub(crate) fn collect_received_ranges_with_sha(
    ledger: Option<&Ledger>,
    events: &[Event],
    request_seq: u64,
    primary_path: &str,
    expected_sha: Option<&str>,
    source_bytes: Option<&[u8]>,
    known_total_lines: Option<usize>,
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
    let mut pending_passages: Vec<Value> = Vec::new();
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
        if !caller_allowed(caller) {
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
                            pending_passages.push(passage.clone());
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
                let start = meta["start_line"].as_u64();
                let mut end = meta["end_line"].as_u64();
                if meta["line_truncated"].as_bool() == Some(true) {
                    end = end.map(|e| e.saturating_sub(1));
                }
                push_range(&mut ranges, start, end);
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
                        if let Some(text) = m["text"].as_str() {
                            if text.chars().count() > 500
                                && (text.starts_with("...") || text.ends_with("..."))
                            {
                                continue;
                            }
                        }
                        let line = m["line"].as_u64();
                        push_range(&mut ranges, line, line);
                    }
                } else {
                    if val["returned_chars"].as_u64() == Some(0)
                        || val["content"].as_str().is_some_and(str::is_empty)
                    {
                        continue;
                    }
                    push_range(
                        &mut ranges,
                        val["start_line"].as_u64(),
                        val["end_line"].as_u64(),
                    );
                }
            }
            _ => {}
        }
    }

    let effective_total = known_total_lines.or(observed_total_lines);
    for passage in &pending_passages {
        push_passage_range(&mut ranges, passage, source_bytes, effective_total);
    }

    // Rule 6.17: credit `context.supplied` and `read_map_node` results through F4's shared
    // `lines_sent_with_ledger`, bounded to this request window and checking caller, run_id, and
    // pinned `file_sha256`.
    let end_seq = events
        .iter()
        .find(|e| {
            e.seq > request_seq
                && research_tools::investigation_markers::is_investigation_marker(&e.kind)
        })
        .map(|e| e.seq);
    let f4_events: Vec<Event> = events
        .iter()
        .filter(|e| {
            e.seq > request_seq
                && end_seq.is_none_or(|end| e.seq < end)
                && (e.kind == research_tools::lines_sent::CONTEXT_SUPPLIED_EVENT
                    || (e.kind == "tool.request"
                        && e.payload["tool"]
                            .as_str()
                            .or_else(|| e.payload["tool_id"].as_str())
                            == Some(research_tools::lines_sent::READ_MAP_NODE_TOOL))
                    || (e.kind == "tool.result"
                        && e.payload["call_id"]
                            .as_str()
                            .and_then(|cid| requests.get(cid))
                            .is_some_and(|(tool, _)| {
                                *tool == research_tools::lines_sent::READ_MAP_NODE_TOOL
                            })))
        })
        .cloned()
        .collect();
    if !f4_events.is_empty() {
        let req_id = request_seq.to_string();
        let sent =
            research_tools::lines_sent::lines_sent_with_ledger(&f4_events, ledger, Some(&req_id));
        for (key, line_ranges) in sent.all_entries() {
            if key.path != primary_path {
                continue;
            }
            if !caller_allowed(Some(&Value::String(key.caller.clone()))) {
                continue;
            }
            if let Some(sha) = expected_sha {
                if key.file_sha256 != sha {
                    continue;
                }
            }
            for r in line_ranges {
                ranges.push((r.start_line, r.end_line));
            }
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
#[cfg(test)]
pub(crate) fn answer_cites_with_bytes(
    ledger: Option<&Ledger>,
    events: &[Event],
    request_seq: u64,
    primary_path: &str,
    total_lines: Option<usize>,
    source_bytes: Option<&[u8]>,
    answer_text: &str,
) -> Vec<Value> {
    answer_cites_with_bytes_and_sha(
        ledger,
        events,
        request_seq,
        primary_path,
        None,
        total_lines,
        source_bytes,
        answer_text,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn answer_cites_with_bytes_and_sha(
    ledger: Option<&Ledger>,
    events: &[Event],
    request_seq: u64,
    primary_path: &str,
    expected_sha: Option<&str>,
    total_lines: Option<usize>,
    source_bytes: Option<&[u8]>,
    answer_text: &str,
) -> Vec<Value> {
    let (received, observed_total) = collect_received_ranges_with_sha(
        ledger,
        events,
        request_seq,
        primary_path,
        expected_sha,
        source_bytes,
        total_lines,
    );
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
        let read = received.iter().any(|&(rs, re)| rs <= start && end <= re);
        cites.push(json!({ "start": start, "end": end, "read": read }));
    }
    cites
}

#[cfg(test)]
pub(crate) fn answer_cites(
    ledger: Option<&Ledger>,
    events: &[Event],
    request_seq: u64,
    primary_path: &str,
    total_lines: Option<usize>,
    answer_text: &str,
) -> Vec<Value> {
    answer_cites_with_bytes(
        ledger,
        events,
        request_seq,
        primary_path,
        total_lines,
        None,
        answer_text,
    )
}

fn verified_source_info(
    state: &DesktopState,
    session: &str,
    primary_path: &str,
    expected_sha: Option<&str>,
) -> (Option<usize>, Option<Vec<u8>>) {
    let Some(expected_sha) = expected_sha.filter(|s| !s.is_empty()) else {
        return (None, None);
    };
    let Ok(env) = state.research_env_for_session(session) else {
        return (None, None);
    };
    let Ok(file) = env
        .sandbox()
        .resolve_existing(SandboxRoot::Sources, session, primary_path, false)
    else {
        return (None, None);
    };
    if !file.metadata.is_file() || file.rel_path != primary_path {
        return (None, None);
    }
    let mut bytes = Vec::new();
    let Ok(fh) = std::fs::File::open(file.canonical_target) else {
        return (None, None);
    };
    if fh
        .take(MAX_FS_READ_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return (None, None);
    }
    if bytes.len() as u64 > MAX_FS_READ_FILE_BYTES {
        return (None, None);
    }
    if ledger::sha256_hex(&bytes) != expected_sha {
        return (None, None);
    }
    let total = std::str::from_utf8(&bytes)
        .ok()
        .map(|text| if text.is_empty() { 0 } else { text.lines().count() });
    (total, Some(bytes))
}

pub(crate) fn answer_cites_for_session(
    state: &DesktopState,
    ledger: &Ledger,
    session: &str,
    events: &[Event],
    request_seq: u64,
    answer_text: &str,
) -> Vec<Value> {
    let Some(paper_ev) = events
        .iter()
        .rev()
        .find(|e| e.kind == "investigation.paper")
    else {
        return Vec::new();
    };
    let Some(source_id) = paper_ev.payload["source_id"].as_str() else {
        return Vec::new();
    };
    let Some(source_ev) = events
        .iter()
        .find(|e| e.kind == "source.ingested" && e.payload["source_id"].as_str() == Some(source_id))
    else {
        return Vec::new();
    };
    let Some(primary_path) = source_ev.payload["primary_path"].as_str() else {
        return Vec::new();
    };
    let expected_sha = source_ev.payload["files_sha256"]
        .get(primary_path)
        .and_then(Value::as_str);
    let (total_lines, source_bytes) =
        verified_source_info(state, session, primary_path, expected_sha);
    answer_cites_with_bytes_and_sha(
        Some(ledger),
        events,
        request_seq,
        primary_path,
        expected_sha,
        total_lines,
        source_bytes.as_deref(),
        answer_text,
    )
}

#[cfg(test)]
#[path = "investigation_cites_tests.rs"]
mod tests;
