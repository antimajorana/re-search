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

fn sha_matches(expected_sha: Option<&str>, actual: Option<&str>) -> bool {
    match expected_sha {
        Some(expected) => !expected.is_empty() && actual == Some(expected),
        None => true,
    }
}

/// Collect line ranges received by the model for `primary_path` from `tool.result`
/// events recorded after `request_seq`, together with the maximum `total_lines`
/// reported by any matching `fs_read` or `read_paper_lines` result.
/// When `expected_sha` is `Some`, only results whose `file_sha256` equals `expected_sha`
/// are credited or allowed to update `observed_total_lines`.
pub(crate) fn collect_received_ranges(
    ledger: Option<&Ledger>,
    events: &[Event],
    request_seq: u64,
    primary_path: &str,
    expected_sha: Option<&str>,
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
                        if passage["path"].as_str() == Some(primary_path)
                            && sha_matches(expected_sha, passage["file_sha256"].as_str())
                        {
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
                if meta["path"].as_str() != Some(primary_path)
                    || !sha_matches(expected_sha, meta["file_sha256"].as_str())
                {
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
                if !is_sources_root
                    || val["path"].as_str() != Some(primary_path)
                    || !sha_matches(expected_sha, val["file_sha256"].as_str())
                {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PageLineBounds {
    Unrestricted,
    Allowed(usize, usize),
    Denied,
}

fn selected_page_line_bounds(
    ledger: &Ledger,
    paper: &Value,
    receipt: &Value,
) -> PageLineBounds {
    if paper["page_start"].is_null() && paper["page_end"].is_null() {
        return PageLineBounds::Unrestricted;
    }
    let read_page = |field: &str, default: usize| -> Option<usize> {
        if paper[field].is_null() {
            return Some(default);
        }
        paper[field].as_u64().and_then(|n| usize::try_from(n).ok())
    };
    let Some(count) = receipt["page_count"]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
    else {
        return PageLineBounds::Denied;
    };
    let (Some(first), Some(last)) = (read_page("page_start", 1), read_page("page_end", count))
    else {
        return PageLineBounds::Denied;
    };
    if receipt["format"].as_str() != Some("pdf") || first == 0 || last < first || last > count {
        return PageLineBounds::Denied;
    }
    let Ok(page_lines) = research_tools::pdf::verified_page_lines(receipt, || {
        let blob = receipt["raw_blob"]
            .as_str()
            .ok_or_else(|| "Original PDF unavailable.".to_string())?;
        ledger.get_blob(blob).map_err(|e| e.to_string())
    }) else {
        return PageLineBounds::Denied;
    };
    match page_lines
        .iter()
        .copied()
        .filter(|&(p, _, _)| (first..=last).contains(&p))
        .fold(None::<(usize, usize)>, |acc, (_, a, b)| {
            Some(match acc {
                Some((lo, hi)) => (lo.min(a), hi.max(b)),
                None => (a, b),
            })
        }) {
        Some((min_line, max_line)) => PageLineBounds::Allowed(min_line, max_line),
        None => PageLineBounds::Denied,
    }
}

/// Build the `cites` array (`[{start, end, read}]`) stored on `investigation.answer`.
/// Keeps only ranges with `1 <= start <= end <= total_lines` (when `total_lines` is known).
#[allow(clippy::too_many_arguments)]
pub(crate) fn answer_cites_checked(
    ledger: Option<&Ledger>,
    events: &[Event],
    request_seq: u64,
    primary_path: &str,
    expected_sha: Option<&str>,
    total_lines: Option<usize>,
    allowed_lines: PageLineBounds,
    answer_text: &str,
) -> Vec<Value> {
    let (received, observed_total) =
        collect_received_ranges(ledger, events, request_seq, primary_path, expected_sha);
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
        let in_allowed_pages = match allowed_lines {
            PageLineBounds::Unrestricted => true,
            PageLineBounds::Allowed(min_line, max_line) => min_line <= start && end <= max_line,
            PageLineBounds::Denied => false,
        };
        let read = in_allowed_pages
            && end.saturating_sub(start) <= MAX_CLICKABLE_CITE_SPAN
            && received.iter().any(|&(rs, re)| rs <= start && end <= re);
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
    answer_cites_checked(
        ledger,
        events,
        request_seq,
        primary_path,
        None,
        total_lines,
        PageLineBounds::Unrestricted,
        answer_text,
    )
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
        .and_then(Value::as_str)
        .unwrap_or("");
    let total_lines = source_total_lines(state, session, primary_path, Some(expected_sha));
    let allowed_lines = selected_page_line_bounds(ledger, &paper_ev.payload, &source_ev.payload);
    answer_cites_checked(
        Some(ledger),
        events,
        request_seq,
        primary_path,
        Some(expected_sha),
        total_lines,
        allowed_lines,
        answer_text,
    )
}

#[cfg(test)]
#[path = "investigation_cites_tests.rs"]
mod tests;
