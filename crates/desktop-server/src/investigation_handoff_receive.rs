//! Receiving side for `target_mode: "investigate"` hand-off packets
//! (package V16, `00-overview.md` §4, `investigate.md` §9 V16).
//!
//! Opens/ingests `ask.paper` (copied source first, then Library cache — zero network calls on
//! cache hit), builds/loads the deterministic skeleton, re-verifies `QuoteRef`s via F4
//! `verify_quote` / `verify_quote_ref` when newly ingested from Library, and attaches each
//! surviving quote as an `investigation.note` on the innermost enclosing skeleton node.

use crate::investigation::latest;
use crate::investigation_cited_support::{find_source_receipt, resolve_library_canonical};
use crate::investigation_notes::MAX_NOTE_TEXT_CHARS;
use crate::investigation_skeleton::{
    is_skeleton_receipt, pin_source_and_build_skeleton, SkeletonRecord,
};
use crate::routes_session::internal;
use crate::state::DesktopState;
use export::handoff::{
    DroppedQuote, HandoffAsk, HandoffPacket, HandoffRequest, HandoffReceipt,
    HANDOFF_PACKET_EVENT, HANDOFF_SENT_EVENT,
};
use ledger::Event;
use research_tools::investigation_markers::EVENT_INVESTIGATION_NOTE;
use research_tools::library::Library;
use research_tools::paper_key::{PaperKey, PaperRef};
use research_tools::quote_core::{
    clip_quote, quote_sha256, verify_quote, verify_quote_ref, QuoteRef, EVIDENCE_QUOTE_EVENT,
};
use research_tools::skeleton::{PaperSkeleton, SkNode, SkNodeKind, Span};
use research_tools::SandboxRoot;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::sync::Mutex;

/// Summary of child-side Investigate setup after receiving a `target_mode: "investigate"` handoff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigateReceiveOutcome {
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skeleton_seq: Option<u64>,
    pub source_origin: String,
    pub attached_quote_notes: usize,
    #[serde(default)]
    pub note_targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_file_reason: Option<String>,
}

/// Complete Investigate child initialization on `target_mode: "investigate"`:
/// library-first `ask.paper` open/ingest, skeleton build/load, F4 quote re-verification if newly
/// attached from library, and attaching surviving quotes onto innermost enclosing nodes.
pub fn receive_investigate_handoff(
    state: &DesktopState,
    parent_session: &str,
    req: &HandoffRequest,
    receipt: &mut HandoffReceipt,
) -> Result<InvestigateReceiveOutcome, (u16, String)> {
    let HandoffAsk::Investigate(ref inv_ask) = req.packet.ask else {
        return Err(internal("expected Investigate ask in receive_investigate_handoff"));
    };

    let ledger = state.open_ledger().map_err(|e| internal(e.to_string()))?;
    let parent_events = ledger.read_session(parent_session).map_err(|e| internal(e.to_string()))?;
    let mut child_events = ledger
        .read_session(&receipt.child_session)
        .map_err(|e| internal(e.to_string()))?;
    drop(ledger);

    let ResolvedPaperSource {
        source_id,
        source_origin,
        library_attached,
        canonical_key,
    } = resolve_or_attach_paper_source(state, receipt, &child_events, &inv_ask.paper)?;

    let Some(source_id) = source_id else {
        return Ok(InvestigateReceiveOutcome {
            status: "needs_file".into(),
            source_id: None,
            skeleton_seq: None,
            source_origin,
            attached_quote_notes: 0,
            note_targets: Vec::new(),
            needs_file_reason: Some(format!(
                "No cached full text found in library for `{}`; attach PDF or TeX in the child session.",
                inv_ask.paper.key
            )),
        });
    };

    let quote_lookup = build_quote_lookup(&req.quote_texts, &parent_events);

    if library_attached {
        child_events = state
            .open_ledger()
            .map_err(|e| internal(e.to_string()))?
            .read_session(&receipt.child_session)
            .map_err(|e| internal(e.to_string()))?;
        reverify_against_newly_attached_source(
            state,
            req,
            receipt,
            &child_events,
            &source_id,
            &quote_lookup,
        )?;
    }

    let child_receipt = find_source_receipt(&child_events, &source_id)
        .ok_or_else(|| internal(format!("missing source receipt `{source_id}` in child session")))?;
    if !is_skeleton_receipt(child_receipt) {
        return Ok(InvestigateReceiveOutcome {
            status: "attached_non_skeleton".into(),
            source_id: Some(source_id),
            skeleton_seq: None,
            source_origin,
            attached_quote_notes: 0,
            note_targets: Vec::new(),
            needs_file_reason: None,
        });
    }

    let focus_question = inv_ask
        .focus_claims
        .first()
        .cloned()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| (!req.packet.title.trim().is_empty()).then(|| req.packet.title.clone()));
    let skel_rec = pin_source_and_build_skeleton(
        state,
        &receipt.child_session,
        &source_id,
        Some(canonical_key),
        inv_ask.paper.version.clone(),
        focus_question.as_deref(),
    )?;

    let surviving = collect_surviving_quotes(&receipt.packet);
    let note_targets = attach_quote_notes_to_innermost_nodes(
        state,
        &receipt.child_session,
        &source_id,
        &skel_rec,
        &surviving,
        &quote_lookup,
    )?;

    Ok(InvestigateReceiveOutcome {
        status: "skeleton_ready".into(),
        source_id: Some(source_id),
        skeleton_seq: Some(skel_rec.event_seq),
        source_origin,
        attached_quote_notes: note_targets.len(),
        note_targets,
        needs_file_reason: None,
    })
}

struct ResolvedPaperSource {
    source_id: Option<String>,
    source_origin: String,
    library_attached: bool,
    canonical_key: PaperKey,
}

fn resolve_or_attach_paper_source(
    state: &DesktopState,
    receipt: &HandoffReceipt,
    child_events: &[Event],
    paper: &PaperRef,
) -> Result<ResolvedPaperSource, (u16, String)> {
    let lib = Library::open(&state.cfg.data_dir).map_err(|e| internal(e.to_string()))?;
    let canonical = resolve_library_canonical(&lib, paper)?;

    // 1. Prefer copied sources already in child_session (F7 Rule 1 copy from origin session).
    if let Some(existing) = child_events
        .iter()
        .find(|e| e.kind == "source.ingested" && source_matches_paper(e, paper, &canonical))
        .or_else(|| child_events.iter().find(|e| e.kind == "source.ingested" && is_skeleton_receipt(e)))
        .and_then(|e| e.payload["source_id"].as_str().map(str::to_string))
    {
        return Ok(ResolvedPaperSource {
            source_id: Some(existing),
            source_origin: "handoff_copy".into(),
            library_attached: false,
            canonical_key: canonical,
        });
    }

    // 2. Otherwise consult Library cache first with ZERO network calls.
    let cached = lib
        .read_fulltext_bytes(&canonical, paper.version.as_deref())
        .map_err(|e| internal(e.to_string()))?;
    if cached.is_some() {
        let env = state
            .research_env_for_session(&receipt.child_session)
            .map_err(internal)?;
        let ledger_mutex = Mutex::new(state.open_ledger().map_err(|e| internal(e.to_string()))?);
        let _lock = state
            .ledger_write_lock
            .lock()
            .map_err(|_| internal("ledger lock poisoned"))?;
        let ingested = lib
            .attach_to_session(
                &env.sandbox(),
                &ledger_mutex,
                &receipt.child_session,
                &canonical,
                paper.version.as_deref(),
                None,
            )
            .map_err(internal)?;
        return Ok(ResolvedPaperSource {
            source_id: Some(ingested.source_id),
            source_origin: "library".into(),
            library_attached: true,
            canonical_key: canonical,
        });
    }

    Ok(ResolvedPaperSource {
        source_id: None,
        source_origin: "needs_file".into(),
        library_attached: false,
        canonical_key: canonical,
    })
}

fn source_matches_paper(ev: &Event, paper: &PaperRef, canonical: &PaperKey) -> bool {
    let p = &ev.payload;
    let cover = p["arxiv_cover_id"].as_str().unwrap_or("");
    let origin = p["origin"].as_str().unwrap_or("");
    let key_str = canonical.as_str();
    let val = canonical.value();
    cover == val
        || origin.contains(key_str)
        || origin.contains(val)
        || paper.ids.arxiv.as_deref().is_some_and(|a| cover == a || origin.contains(a))
}

fn reverify_against_newly_attached_source(
    state: &DesktopState,
    raw_req: &HandoffRequest,
    receipt: &mut HandoffReceipt,
    child_events: &[Event],
    source_id: &str,
    quote_lookup: &BTreeMap<String, String>,
) -> Result<(), (u16, String)> {
    let child_receipt = find_source_receipt(child_events, source_id)
        .ok_or_else(|| internal("library-attached source receipt missing"))?;
    let primary_path = child_receipt.payload["primary_path"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let env = state
        .research_env_for_session(&receipt.child_session)
        .map_err(internal)?;
    let sandbox = env.sandbox();

    let mut matched = 0usize;
    let mut dropped = Vec::new();

    let mut verify_one = |loc: String, qref: &QuoteRef| -> Option<QuoteRef> {
        let target_path = if qref.path.starts_with(&format!("{source_id}/")) {
            qref.path.clone()
        } else if let Some((_, rest)) = qref.path.split_once('/') {
            let remapped = format!("{source_id}/{rest}");
            if sandbox.resolve_existing(SandboxRoot::Sources, &receipt.child_session, &remapped, false).is_ok() {
                remapped
            } else {
                primary_path.clone()
            }
        } else {
            primary_path.clone()
        };
        let mut candidate = qref.clone();
        candidate.source_id = source_id.to_string();
        candidate.path = target_path.clone();

        // Reject if caller pinned a specific file_sha256 that differs from the child's actual file bytes.
        let expected_hash = find_child_file_hash(child_receipt, &target_path);
        if !qref.file_sha256.is_empty() && !expected_hash.is_empty() && qref.file_sha256 != expected_hash {
            dropped.push(DroppedQuote {
                location: loc,
                source_id: qref.source_id.clone(),
                path: qref.path.clone(),
                start_line: qref.start_line,
                end_line: qref.end_line,
                reason: "file_sha256 does not match child library source".into(),
            });
            return None;
        }

        let text = quote_lookup
            .get(&qref.quote_sha256)
            .cloned()
            .or_else(|| read_matching_lines(&sandbox, &receipt.child_session, &target_path, qref));
        let Some(text) = text else {
            dropped.push(DroppedQuote {
                location: loc,
                source_id: qref.source_id.clone(),
                path: qref.path.clone(),
                start_line: qref.start_line,
                end_line: qref.end_line,
                reason: "verbatim quote text unavailable or lines do not match quote_sha256 in child".into(),
            });
            return None;
        };

        let check = if candidate.file_sha256.is_empty() {
            verify_quote(
                &sandbox,
                &receipt.child_session,
                child_events,
                source_id,
                &target_path,
                qref.start_line,
                qref.end_line,
                &text,
            )
        } else {
            verify_quote_ref(&sandbox, &receipt.child_session, child_events, &candidate, &text)
        };
        match check {
            Ok(v) => {
                let mut clean = v.quote_ref;
                clean.evidence_seq = None;
                matched += 1;
                Some(clean)
            }
            Err(fault) => {
                dropped.push(DroppedQuote {
                    location: loc,
                    source_id: qref.source_id.clone(),
                    path: qref.path.clone(),
                    start_line: qref.start_line,
                    end_line: qref.end_line,
                    reason: fault.to_string(),
                });
                None
            }
        }
    };

    receipt.packet.statement.quotes = raw_req
        .packet
        .statement
        .quotes
        .iter()
        .enumerate()
        .filter_map(|(i, q)| verify_one(format!("statement.quotes[{i}]"), q))
        .collect();
    for (ci, item) in receipt.packet.context.iter_mut().enumerate() {
        let orig_quotes = &raw_req.packet.context[ci].quotes;
        item.quotes = orig_quotes
            .iter()
            .enumerate()
            .filter_map(|(qi, q)| verify_one(format!("context[{ci}].quotes[{qi}]"), q))
            .collect();
    }
    if let (HandoffAsk::Investigate(ref mut dst), HandoffAsk::Investigate(ref src)) =
        (&mut receipt.packet.ask, &raw_req.packet.ask)
    {
        dst.earlier_quotes = src
            .earlier_quotes
            .iter()
            .enumerate()
            .filter_map(|(i, q)| verify_one(format!("ask.earlier_quotes[{i}]"), q))
            .collect();
    }

    receipt.quotes_matched = matched;
    receipt.quotes_dropped = dropped.len();
    receipt.dropped_quotes = dropped;
    Ok(())
}

fn find_child_file_hash(receipt: &Event, rel_path: &str) -> String {
    if let Some(map) = receipt.payload.get("files_sha256").and_then(|v| v.as_object()) {
        if let Some(h) = map.get(rel_path).and_then(|v| v.as_str()) {
            return h.to_string();
        }
        let suffix = rel_path.split_once('/').map(|(_, rest)| rest).unwrap_or(rel_path);
        if let Some(h) = map.get(suffix).and_then(|v| v.as_str()) {
            return h.to_string();
        }
    }
    receipt.payload["file_sha256"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn read_matching_lines(
    sandbox: &research_tools::WorkspaceSandbox,
    session_id: &str,
    path: &str,
    qref: &QuoteRef,
) -> Option<String> {
    if qref.start_line == 0 || qref.end_line < qref.start_line {
        return None;
    }
    let resolved = sandbox
        .resolve_existing(SandboxRoot::Sources, session_id, path, false)
        .ok()?;
    let text = fs::read_to_string(&resolved.canonical_target).ok()?;
    let lines: Vec<&str> = text.lines().collect();
    if qref.end_line > lines.len() {
        return None;
    }
    let slice = lines[qref.start_line - 1..qref.end_line].join("\n");
    (quote_sha256(&slice) == qref.quote_sha256).then_some(slice)
}

fn collect_surviving_quotes(packet: &HandoffPacket) -> Vec<QuoteRef> {
    let mut quotes = Vec::new();
    if let HandoffAsk::Investigate(ref ask) = packet.ask {
        quotes.extend(ask.earlier_quotes.iter().cloned());
    }
    quotes.extend(packet.statement.quotes.iter().cloned());
    for item in &packet.context {
        quotes.extend(item.quotes.iter().cloned());
    }
    quotes
}

/// Find the innermost enclosing skeleton node whose span covers `(qref.path, qref.start_line..=qref.end_line)`.
pub fn find_innermost_enclosing_node<'a>(
    skeleton: &'a PaperSkeleton,
    qref: &QuoteRef,
) -> Option<&'a SkNode> {
    let target = Span::new(&qref.path, qref.start_line, qref.end_line);
    let mut parent_by_tag: HashMap<&str, Option<&str>> = HashMap::new();
    for node in &skeleton.nodes {
        if let Some(tag) = node.tag.as_deref() {
            parent_by_tag.insert(tag, node.parent.as_deref());
        }
    }
    let depth_of = |tag: &str| -> usize {
        let mut d = 0usize;
        let mut cur = Some(tag);
        while let Some(t) = cur {
            if d > 64 { break; }
            cur = parent_by_tag.get(t).copied().flatten();
            if cur.is_some() { d += 1; }
        }
        d
    };

    // Rank candidates strictly enclosing the quote span:
    // 1. Structural depth (deeper parent chain wins, e.g. Proof/Equation > Theorem > Section)
    // 2. Non-Section/non-FrontMatter nodes over structural containers
    // 3. Narrowest containing span width (end - start)
    // 4. Highest document order
    skeleton
        .nodes
        .iter()
        .filter(|n| n.tag.is_some())
        .filter_map(|node| {
            let best_width = node
                .spans
                .iter()
                .filter(|s| s.contains(&target))
                .map(|s| s.end_line.saturating_sub(s.start_line))
                .min()?;
            let tag = node.tag.as_deref()?;
            let is_leaf_kind = !matches!(node.kind, SkNodeKind::Section | SkNodeKind::FrontMatter | SkNodeKind::Bibliography);
            Some((
                (depth_of(tag), is_leaf_kind, std::cmp::Reverse(best_width), node.order),
                node,
            ))
        })
        .max_by_key(|(rank, _)| *rank)
        .map(|(_, node)| node)
}

fn attach_quote_notes_to_innermost_nodes(
    state: &DesktopState,
    child_session: &str,
    source_id: &str,
    skel_rec: &SkeletonRecord,
    quotes: &[QuoteRef],
    quote_lookup: &BTreeMap<String, String>,
) -> Result<Vec<String>, (u16, String)> {
    if quotes.is_empty() {
        return Ok(Vec::new());
    }
    let mut attached_tags = Vec::new();
    let ledger = state.open_ledger().map_err(|e| internal(e.to_string()))?;
    let _lock = state
        .ledger_write_lock
        .lock()
        .map_err(|_| internal("ledger lock poisoned"))?;

    let mut seen_note_keys = std::collections::BTreeSet::new();
    for qref in quotes {
        let Some(node) = find_innermost_enclosing_node(&skel_rec.skeleton, qref) else {
            continue;
        };
        let Some(tag) = node.tag.as_deref() else {
            continue;
        };
        if !seen_note_keys.insert((tag.to_string(), qref.quote_sha256.clone(), qref.start_line, qref.end_line)) {
            continue;
        }
        let raw_text = quote_lookup
            .get(&qref.quote_sha256)
            .cloned()
            .or_else(|| {
                state
                    .research_env_for_session(child_session)
                    .ok()
                    .and_then(|env| read_matching_lines(&env.sandbox(), child_session, &qref.path, qref))
            })
            .unwrap_or_default();
        let note_body = if raw_text.trim().is_empty() {
            format!("[Handoff quote {}:{}-{}]", qref.path, qref.start_line, qref.end_line)
        } else {
            format!(
                "[Handoff quote {}:{}-{}] \"{}\"",
                qref.path,
                qref.start_line,
                qref.end_line,
                clip_quote(raw_text.trim())
            )
        };
        let text: String = note_body.chars().take(MAX_NOTE_TEXT_CHARS).collect();
        let payload = json!({
            "source_id": source_id,
            "skeleton_seq": skel_rec.event_seq,
            "tag": tag,
            "node_fp": node.fp,
            "text": text,
        });
        ledger
            .append(child_session, EVENT_INVESTIGATION_NOTE, 1, &payload)
            .map_err(|e| internal(e.to_string()))?;
        attached_tags.push(tag.to_string());
    }

    let _ = (latest, HANDOFF_PACKET_EVENT, HANDOFF_SENT_EVENT);
    Ok(attached_tags)
}

fn build_quote_lookup(caller: &BTreeMap<String, String>, parent_events: &[Event]) -> BTreeMap<String, String> {
    let mut map = caller.clone();
    for ev in parent_events {
        if ev.kind == EVIDENCE_QUOTE_EVENT {
            if let (Some(h), Some(q)) = (ev.payload["quote_sha256"].as_str(), ev.payload["quote"].as_str()) {
                map.entry(h.to_string()).or_insert_with(|| q.to_string());
            }
        } else if ev.kind == "investigation.evidence" {
            if let Some(q) = ev.payload["quote"].as_str() {
                map.entry(quote_sha256(q)).or_insert_with(|| q.to_string());
            }
        }
    }
    map
}
