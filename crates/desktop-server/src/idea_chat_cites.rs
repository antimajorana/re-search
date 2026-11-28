//! Multi-paper line-citation parser and F4 `lines_sent_with_ledger` checker for Explore
//! Conversation answers (package X15).
//!
//! Governance: `00-overview.md` 2.5 (F4), 3.2 row 20 (`"matched"`, never `"verified"`);
//! `explore.md` 5.2, 6 (Rules C1, C2, C8, S8), 9 (row X15).

use crate::idea::EVENT_IDEA_PAPER;
use crate::idea_system_prompt::EXPLORE_CHAT_CITE_MATCHED_WORD;
use ledger::{Event, Ledger};
use research_tools::lines_sent::lines_sent_with_ledger;
use research_tools::paper_key::PaperKey;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

/// Additive ledger event kind recording checked multi-paper line citations for an Explore
/// Conversation answer (`explore.md` 5.2, 6 Rules C1/C2/S8; `00-overview.md` 2.5, 3.2 row 20).
pub const EVENT_IDEA_CHAT_CITATIONS: &str = "idea.chat_citations";

/// Owner-accepted matched citation label (`00-overview.md` 3.2 row 20, Rule S8).
pub const CHAT_CITE_STATUS_MATCHED: &str = EXPLORE_CHAT_CITE_MATCHED_WORD;

/// Unread / unmatched citation label (`explore.md` Rule C2).
pub const CHAT_CITE_STATUS_UNREAD: &str = "unread";

/// One parsed `[paper_ref: Lstart-end]` line citation extracted from assistant prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedPaperCitation {
    pub raw: String,
    pub paper_ref: String,
    pub start_line: usize,
    pub end_line: usize,
}

/// Checked citation record produced strictly against F4 `lines_sent_with_ledger` and pinned
/// `source.ingested` file hashes (`explore.md` Rules C1, C2, C8, S8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckedPaperCitation {
    pub raw: String,
    pub paper_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub start_line: usize,
    pub end_line: usize,
    pub matched: bool,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr_tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_basis: Option<String>,
}

/// Summary report for all citations parsed from one Explore Conversation answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExploreChatCitationReport {
    pub caller: String,
    pub matched_count: usize,
    pub unread_count: usize,
    pub citations: Vec<CheckedPaperCitation>,
}

#[derive(Debug, Clone)]
struct IngestedPaperInfo {
    source_id: String,
    paper_key: Option<String>,
    primary_path: String,
    files: Vec<(String, String)>,
    pdf_pages: Vec<(usize, usize, usize, String)>,
    text_basis: String,
}

fn parse_one_line_range(part: &str) -> Option<(usize, usize)> {
    let rest = part.trim().strip_prefix('L')?;
    if rest.is_empty() {
        return None;
    }
    match rest.find(['-', '–']) {
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

fn parse_segment_paper_cites(segment: &str) -> Option<Vec<ParsedPaperCitation>> {
    let trimmed = segment.trim();
    let colon = trimmed.rfind(':')?;
    let paper_ref = trimmed[..colon].trim();
    let ranges_raw = trimmed[colon + 1..].trim();
    if paper_ref.is_empty()
        || paper_ref.contains('[')
        || paper_ref.contains(']')
        || paper_ref.contains('\n')
        || !ranges_raw.starts_with('L')
    {
        return None;
    }
    let mut out = Vec::new();
    for raw_part in ranges_raw.split(',') {
        let part = raw_part.trim();
        let (start_line, end_line) = parse_one_line_range(part)?;
        out.push(ParsedPaperCitation {
            raw: format!("[{paper_ref}: {part}]"),
            paper_ref: paper_ref.to_string(),
            start_line,
            end_line,
        });
    }
    (!out.is_empty()).then_some(out)
}

/// Parse multi-paper line citations of the form `[paper_key: Lstart-end]`,
/// `[source_id: Lstart-end]`, `[paper_key: L12, L18-20]`, or semicolon-grouped
/// `[paperA: L1-4; paperB: L7-9]` from Explore Conversation prose.
pub fn parse_multi_paper_line_citations(text: &str) -> Vec<ParsedPaperCitation> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            if let Some(rel_end) = text[i + 1..].find(']') {
                let inner = &text[i + 1..i + 1 + rel_end];
                if !inner.is_empty() && !inner.contains('[') && !inner.contains('\n') {
                    let parsed = if inner.contains(';') {
                        let mut combined = Vec::new();
                        let mut all_ok = true;
                        for seg in inner.split(';') {
                            if let Some(items) = parse_segment_paper_cites(seg) {
                                combined.extend(items);
                            } else {
                                all_ok = false;
                                break;
                            }
                        }
                        if all_ok && !combined.is_empty() {
                            Some(combined)
                        } else {
                            parse_segment_paper_cites(inner)
                        }
                    } else {
                        parse_segment_paper_cites(inner)
                    };
                    if let Some(items) = parsed {
                        out.extend(items);
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

fn collect_ingested_papers(events: &[Event]) -> Vec<IngestedPaperInfo> {
    let mut meta_by_sid: BTreeMap<String, (Option<String>, String)> = BTreeMap::new();
    for ev in events {
        if ev.kind == EVENT_IDEA_PAPER {
            if let Some(sid) = ev.payload.get("source_id").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                let key = ev
                    .payload
                    .get("key")
                    .and_then(Value::as_str)
                    .map(|k| PaperKey::parse(k).map_or_else(|_| k.to_string(), |pk| pk.as_str().to_string()));
                let basis = ev
                    .payload
                    .get("text_basis")
                    .and_then(Value::as_str)
                    .unwrap_or("tex")
                    .to_string();
                meta_by_sid.insert(sid.to_string(), (key, basis));
            }
        }
    }

    let mut by_sid: BTreeMap<String, IngestedPaperInfo> = BTreeMap::new();
    for ev in events {
        if ev.kind != "source.ingested" {
            continue;
        }
        let Some(sid) = ev.payload.get("source_id").and_then(Value::as_str).filter(|s| !s.is_empty())
        else {
            continue;
        };
        let primary_path = ev
            .payload
            .get("primary_path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let mut files: Vec<(String, String)> = Vec::new();
        if let Some(obj) = ev.payload.get("files_sha256").and_then(Value::as_object) {
            for (path, val) in obj {
                if let Some(sha) = val.as_str().filter(|s| s.len() == 64) {
                    files.push((path.clone(), sha.to_string()));
                }
            }
        }
        if files.is_empty() {
            if let Some(sha) = ev.payload.get("file_sha256").and_then(Value::as_str).filter(|s| s.len() == 64) {
                if !primary_path.is_empty() {
                    files.push((primary_path.clone(), sha.to_string()));
                }
            }
        }
        let pdf_pages: Vec<(usize, usize, usize, String)> = ev
            .payload
            .get("pdf_pages")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| {
                        Some((
                            usize::try_from(p.get("page")?.as_u64()?).ok()?,
                            usize::try_from(p.get("first_line")?.as_u64()?).ok()?,
                            usize::try_from(p.get("last_line")?.as_u64()?).ok()?,
                            p.get("basis")
                                .and_then(Value::as_str)
                                .unwrap_or("text_layer")
                                .to_string(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let (idea_key, idea_basis) = meta_by_sid
            .get(sid)
            .cloned()
            .unwrap_or_else(|| (None, "tex".to_string()));
        let fallback_key = ev
            .payload
            .get("paper_key")
            .or_else(|| ev.payload.get("origin"))
            .and_then(Value::as_str)
            .and_then(|s| PaperKey::parse(s).ok().map(|pk| pk.as_str().to_string()));
        let paper_key = idea_key.or(fallback_key);
        let text_basis = if !pdf_pages.is_empty() && idea_basis == "tex" {
            "pdf_text".to_string()
        } else {
            idea_basis
        };

        by_sid.insert(
            sid.to_string(),
            IngestedPaperInfo {
                source_id: sid.to_string(),
                paper_key,
                primary_path,
                files,
                pdf_pages,
                text_basis,
            },
        );
    }

    by_sid.into_values().collect()
}

fn resolve_paper_ref<'a>(
    papers: &'a [IngestedPaperInfo],
    raw_ref: &str,
) -> Option<&'a IngestedPaperInfo> {
    let trimmed = raw_ref.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(found) = papers.iter().find(|p| p.source_id == trimmed) {
        return Some(found);
    }
    let canon = PaperKey::parse(trimmed).ok().map(|pk| pk.as_str().to_string());
    if let Some(ref key) = canon {
        if let Some(found) = papers
            .iter()
            .find(|p| p.paper_key.as_deref() == Some(key.as_str()))
        {
            return Some(found);
        }
    }
    let lowered = trimmed.to_ascii_lowercase();
    papers.iter().find(|p| {
        p.paper_key
            .as_deref()
            .is_some_and(|k| k.to_ascii_lowercase() == lowered)
    })
}

fn unread_citation(parsed: ParsedPaperCitation, reason: &str) -> CheckedPaperCitation {
    CheckedPaperCitation {
        raw: parsed.raw,
        paper_ref: parsed.paper_ref,
        paper_key: None,
        source_id: None,
        path: None,
        start_line: parsed.start_line,
        end_line: parsed.end_line,
        matched: false,
        status: CHAT_CITE_STATUS_UNREAD.to_string(),
        reason: Some(reason.to_string()),
        page: None,
        ocr_tag: None,
        text_basis: None,
    }
}

/// Check every `[paper_key: Lstart-end]` / `[source_id: Lstart-end]` citation in `answer_text`
/// strictly via F4 [`research_tools::lines_sent::lines_sent_with_ledger`] across every
/// `(source_id, path, file_sha256)` sent to `caller` during Conversation turns (`session.start`
/// onward). Unread ranges, wrong-paper ranges, and uningested papers are explicitly flagged
/// `status: "unread"` (`matched: false`), never `"matched"` and never `"verified"`.
pub fn check_explore_answer_citations(
    ledger: Option<&Ledger>,
    events: &[Event],
    caller: &str,
    answer_text: &str,
) -> ExploreChatCitationReport {
    let papers = collect_ingested_papers(events);
    let chat_slice = events
        .iter()
        .position(|e| e.kind == "session.start")
        .map_or(events, |idx| &events[idx..]);
    let sent = lines_sent_with_ledger(chat_slice, ledger, None);

    let mut seen: HashSet<(String, usize, usize)> = HashSet::new();
    let mut citations = Vec::new();
    let mut matched_count = 0usize;
    let mut unread_count = 0usize;

    for parsed in parse_multi_paper_line_citations(answer_text) {
        let dedup_key = (
            parsed.paper_ref.to_ascii_lowercase(),
            parsed.start_line,
            parsed.end_line,
        );
        if !seen.insert(dedup_key) {
            continue;
        }
        if parsed.start_line == 0 || parsed.start_line > parsed.end_line {
            unread_count += 1;
            citations.push(unread_citation(parsed, "invalid line range"));
            continue;
        }

        let Some(paper) = resolve_paper_ref(&papers, &parsed.paper_ref) else {
            unread_count += 1;
            citations.push(unread_citation(
                parsed,
                "cited paper is not ingested in this session",
            ));
            continue;
        };

        let matched_path = if caller.trim().is_empty() {
            None
        } else {
            paper
                .files
                .iter()
                .find(|(path, sha)| {
                    sent.covers(
                        caller,
                        &paper.source_id,
                        path,
                        sha,
                        parsed.start_line,
                        parsed.end_line,
                    )
                })
                .map(|(path, _)| path.clone())
        };

        let (page, page_basis) = paper
            .pdf_pages
            .iter()
            .find(|(_, first, last, _)| parsed.start_line >= *first && parsed.start_line <= *last)
            .map(|(pg, _, _, basis)| (Some(*pg), Some(basis.as_str())))
            .unwrap_or((None, None));
        let eff_basis = page_basis.unwrap_or(paper.text_basis.as_str());
        let ocr_tag = matches!(eff_basis, "scan_text_layer" | "ocr").then(|| "OCR".to_string());

        if let Some(path) = matched_path {
            matched_count += 1;
            citations.push(CheckedPaperCitation {
                raw: parsed.raw,
                paper_ref: parsed.paper_ref,
                paper_key: paper.paper_key.clone(),
                source_id: Some(paper.source_id.clone()),
                path: Some(path),
                start_line: parsed.start_line,
                end_line: parsed.end_line,
                matched: true,
                status: CHAT_CITE_STATUS_MATCHED.to_string(),
                reason: None,
                page,
                ocr_tag,
                text_basis: Some(eff_basis.to_string()),
            });
        } else {
            unread_count += 1;
            citations.push(CheckedPaperCitation {
                raw: parsed.raw,
                paper_ref: parsed.paper_ref,
                paper_key: paper.paper_key.clone(),
                source_id: Some(paper.source_id.clone()),
                path: (!paper.primary_path.is_empty()).then(|| paper.primary_path.clone()),
                start_line: parsed.start_line,
                end_line: parsed.end_line,
                matched: false,
                status: CHAT_CITE_STATUS_UNREAD.to_string(),
                reason: Some(
                    "lines not read from this paper in this conversation".to_string(),
                ),
                page,
                ocr_tag,
                text_basis: Some(eff_basis.to_string()),
            });
        }
    }

    ExploreChatCitationReport {
        caller: caller.to_string(),
        matched_count,
        unread_count,
        citations,
    }
}

/// Extract all persisted `idea.chat_citations` payloads from ledger events for view consumers.
pub fn chat_citations_from_events(events: &[Event]) -> Vec<Value> {
    events
        .iter()
        .filter(|e| e.kind == EVENT_IDEA_CHAT_CITATIONS)
        .map(|e| {
            let mut v = e.payload.clone();
            if let Some(obj) = v.as_object_mut() {
                obj.insert("seq".to_string(), json!(e.seq));
                obj.insert("ts_ms".to_string(), json!(e.ts_ms));
            }
            v
        })
        .collect()
}
