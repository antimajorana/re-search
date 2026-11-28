//! Deterministic skeleton persistence, pinning, and HTTP handlers for Investigate (package V3).
//!
//! Zero model calls and zero network calls happen here: skeletons are built deterministically
//! from hash-checked ingested TeX files, stored in the blob store, and recorded in the ledger.
//!
//! Registration note (`http_srv.rs` is over the 600-line ceiling and not edited by V3):
//! add one delegating arm before the catch-all `investigation` POST arm in `dispatch_api`:
//! ```text
//! (method, ["api", "sessions", id, "investigation", op @ ("skeleton" | "mapview" | "node-text")]) => {
//!     write_api_res(&mut stream, crate::investigation_skeleton::handle(state, method, id, op, &req.query, &req.body));
//! }
//! ```

use crate::investigation::{is_investigation, latest};
use crate::investigation_mapview::{attach_pdf_page_metadata, build_mapview_json_with_fold, read_verified_node_text};
use crate::routes_session::{bad, conflict, internal, not_found, parse_json_body, ApiResult};
use crate::state::DesktopState;
use ledger::{Event, Ledger};
use research_tools::paper_key::{split_arxiv_id, PaperKey};
use research_tools::skeleton::{
    build_html_skeleton_from_ingested, build_pdf_skeleton_from_ingested,
    build_tex_skeleton_from_ingested, is_html_receipt, load_ingested_pdf_source, BuildTiming,
    PaperSkeleton, SkNodeKind, SkeletonBuilder,
};
use research_tools::validate_session_id;
use serde_json::{json, Value};
use std::collections::HashMap;

/// Persisted deterministic skeleton event (`investigate.md` §4.4; `00-overview.md` §8). Host-only.
pub const EVENT_INVESTIGATION_SKELETON: &str = "investigation.skeleton";
/// Primary paper pin event (`investigate.md` §4.4; preserved unchanged per `00-overview.md` §8).
pub const EVENT_INVESTIGATION_PAPER: &str = "investigation.paper";
/// Investigation mode marker event (`investigate.md` §4.4; accepted in `00-overview.md` §8).
pub const EVENT_INVESTIGATION_STARTED: &str = "investigation.started";
const EVENT_SOURCE_INGESTED: &str = "source.ingested";

/// Loaded or newly built deterministic skeleton plus its ledger sequence and blob digest.
#[derive(Debug, Clone)]
pub struct SkeletonRecord {
    pub event_seq: u64,
    pub blob_hash: String,
    pub skeleton: PaperSkeleton,
    pub timing: Option<BuildTiming>,
    pub rebuilt: bool,
}

/// True iff `receipt.payload["files"]` contains a `.tex`/`.ltx` path (excluding distinct `assembled_path`).
pub fn is_tex_receipt(receipt: &Event) -> bool {
    let primary = receipt.payload["primary_path"].as_str();
    let assembled = receipt.payload["assembled_path"].as_str();
    receipt.payload["files"].as_array().is_some_and(|files| {
        files.iter().filter_map(Value::as_str).any(|rel| {
            if assembled == Some(rel) && primary != Some(rel) {
                return false;
            }
            let lower = rel.to_ascii_lowercase();
            lower.ends_with(".tex") || lower.ends_with(".ltx")
        })
    })
}

/// True iff `receipt` is a PDF `source.ingested` receipt eligible for V17 `pdf-1` skeleton extraction.
pub fn is_pdf_receipt(receipt: &Event) -> bool {
    if is_tex_receipt(receipt) {
        return false;
    }
    let p = &receipt.payload;
    let status = p["status"].as_str().unwrap_or("");
    matches!(status, "ok" | "pdf_only" | "empty_text_layer")
        && (p["format"].as_str() == Some("pdf")
            || status == "pdf_only"
            || p.get("pdf_extractor").is_some_and(Value::is_string)
            || p.get("pdf_pages").is_some_and(Value::is_array))
}

/// True iff `receipt` supports deterministic skeleton building (`tex-1`, `html-1`, or `pdf-1`).
pub fn is_skeleton_receipt(receipt: &Event) -> bool {
    is_tex_receipt(receipt) || is_html_receipt(receipt) || is_pdf_receipt(receipt)
}

/// Derives `(PaperKey, version)` from overrides, `arxiv_cover_id`, `origin`, `raw_blob`, or `source_id`.
pub fn derive_paper_identity(
    receipt: &Event, key_override: Option<PaperKey>, version_override: Option<String>,
) -> Result<(PaperKey, Option<String>), String> {
    let ver_ov = version_override.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    if let Some(key) = key_override { return Ok((key, ver_ov)); }
    let p = &receipt.payload;
    for field in ["arxiv_cover_id", "origin"] {
        if let Some((base, ver)) = non_empty(&p[field]).and_then(|r| split_arxiv_id(r).ok()) {
            return Ok((PaperKey::from_arxiv(&base)?, ver_ov.or(ver)));
        }
    }
    if let Some(raw_blob) = non_empty(&p["raw_blob"]).filter(|s| is_lower_hex_prefix(s, 16)) {
        return Ok((PaperKey::parse(&format!("file:{}", &raw_blob[..16]))?, ver_ov));
    }
    let sid = non_empty(&p["source_id"])
        .ok_or_else(|| "source.ingested receipt missing `source_id`".to_string())?;
    Ok((PaperKey::from_file_bytes(sid.as_bytes()), ver_ov))
}

/// Loads the existing skeleton for `(source_id, receipt.seq)` (Rule 6.13 / Rule 6.2 repair) or
/// builds and appends a new `investigation.skeleton` event with zero model calls.
pub fn build_and_record_skeleton(
    state: &DesktopState, session_id: &str, source_id: &str,
    key_override: Option<PaperKey>, version_override: Option<String>,
) -> Result<SkeletonRecord, (u16, String)> {
    let source_id = source_id.trim();
    if source_id.is_empty() { return Err(bad("Provide source_id.")); }
    let (ledger, events) = open_and_read(state, session_id)?;
    let receipt = find_source_receipt(&events, source_id, None).ok_or_else(|| {
        not_found(format!("source `{source_id}` has not been ingested in session `{session_id}`"))
    })?;
    if let Some(ev) = latest_skeleton_for_source(&events, source_id, receipt.seq) {
        return load_or_repair_skeleton(state, &ledger, session_id, ev, Some(receipt));
    }
    build_and_append_new(state, &ledger, session_id, receipt, key_override, version_override, false)?
        .ok_or_else(|| internal("skeleton build returned no record"))
}

/// Verifies Investigation mode, pins `investigation.paper` (409 on conflict), and builds its skeleton.
pub fn pin_source_and_build_skeleton(
    state: &DesktopState, session_id: &str, source_id: &str,
    key_override: Option<PaperKey>, version_override: Option<String>, question: Option<&str>,
) -> Result<SkeletonRecord, (u16, String)> {
    let source_id = source_id.trim();
    if source_id.is_empty() { return Err(bad("Provide source_id.")); }
    let (ledger, events) = open_and_read(state, session_id)?;
    if !is_investigation(&events) {
        return Err(bad("Open an Investigation session before pinning a paper."));
    }
    if let Some(pinned) = latest(&events, EVENT_INVESTIGATION_PAPER) {
        if pinned.payload["source_id"].as_str() != Some(source_id) {
            return Err(conflict("Start a new Investigation for a different primary paper."));
        }
    } else {
        if find_source_receipt(&events, source_id, None).is_none() {
            return Err(not_found(format!("source `{source_id}` has not been ingested in session `{session_id}`")));
        }
        let payload = json!({ "source_id": source_id, "question": question.map(str::trim).filter(|s| !s.is_empty()) });
        let _lock = state.ledger_write_lock.lock().map_err(|_| internal("ledger lock poisoned"))?;
        ledger.append(session_id, EVENT_INVESTIGATION_PAPER, 1, &payload).map_err(storage)?;
    }
    drop(ledger);
    build_and_record_skeleton(state, session_id, source_id, key_override, version_override)
}

/// Ensures a pinned TeX Investigation has a skeleton on open (`investigate.md` §8.4).
/// Propagates Rule 6.2 corruption errors; returns `Ok(None)` if an old session's files changed.
pub fn ensure_skeleton_on_open(
    state: &DesktopState, session_id: &str,
) -> Result<Option<SkeletonRecord>, (u16, String)> {
    let (ledger, events) = open_and_read(state, session_id)?;
    if !is_investigation(&events) { return Ok(None); }
    let Some(source_id) =
        latest(&events, EVENT_INVESTIGATION_PAPER).and_then(|e| e.payload["source_id"].as_str())
    else { return Ok(None); };
    let Some(receipt) = find_source_receipt(&events, source_id, None).filter(|r| is_skeleton_receipt(r))
    else { return Ok(None); };
    if let Some(ev) = latest_skeleton_for_source(&events, source_id, receipt.seq) {
        return load_or_repair_skeleton(state, &ledger, session_id, ev, Some(receipt)).map(Some);
    }
    build_and_append_new(state, &ledger, session_id, receipt, None, None, true)
}

/// HTTP route dispatcher for `/api/sessions/{id}/investigation/{skeleton|mapview|node-text}`.
pub fn handle(
    state: &DesktopState, method: &str, session_id: &str, op: &str,
    query: &HashMap<String, String>, body: &[u8],
) -> ApiResult {
    handle_with_etag(state, method, session_id, op, query, body, None).map(|(st, _, v)| (st, v))
}

/// HTTP route dispatcher returning `(status, optional_etag, body)` with `If-None-Match` support.
pub fn handle_with_etag(
    state: &DesktopState, method: &str, session_id: &str, op: &str,
    query: &HashMap<String, String>, body: &[u8], if_none_match: Option<&str>,
) -> Result<(u16, Option<String>, Value), (u16, String)> {
    match (method, op) {
        ("GET", "skeleton") => {
            let rec = load_skeleton_for_query(state, session_id, parse_opt_u64(query, "seq")?)?
                .ok_or_else(|| not_found("no skeleton available for this session"))?;
            if etag_matches(if_none_match, &rec.blob_hash) { return Ok((304, Some(rec.blob_hash), Value::Null)); }
            let etag = rec.blob_hash.clone();
            Ok((200, Some(etag.clone()), json!({
                "seq": rec.event_seq, "blob": rec.blob_hash, "etag": etag, "skeleton": rec.skeleton,
            })))
        }
        ("POST", "skeleton") => {
            validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
            let val = parse_json_body(body)?;
            let source_id = non_empty(&val["source_id"]).ok_or_else(|| bad("Provide source_id."))?;
            let paper_key = non_empty(&val["paper_key"]).map(PaperKey::parse).transpose().map_err(bad)?;
            let version = non_empty(&val["version"]).map(str::to_string);
            let rec = if val.get("pin").and_then(Value::as_bool).unwrap_or(true) {
                pin_source_and_build_skeleton(state, session_id, source_id, paper_key, version, val["question"].as_str())?
            } else {
                build_and_record_skeleton(state, session_id, source_id, paper_key, version)?
            };
            let etag = rec.blob_hash.clone();
            Ok((200, Some(etag.clone()), json!({
                "seq": rec.event_seq, "blob": rec.blob_hash, "etag": etag,
                "timing": rec.timing, "skeleton": rec.skeleton,
            })))
        }
        ("GET", "mapview") => {
            let mut rec = ensure_skeleton_on_open(state, session_id)?;
            let (ledger, events) = open_and_read(state, session_id)?;
            if let Some(ev) = latest(&events, EVENT_INVESTIGATION_SKELETON)
                .filter(|_| rec.is_none() && latest(&events, EVENT_INVESTIGATION_PAPER).is_none())
            {
                rec = Some(load_or_repair_skeleton(state, &ledger, session_id, ev, receipt_for_skeleton_event(&events, ev))?);
            }
            let head = events.last().map_or_else(String::new, |e| e.event_hash.clone());
            if etag_matches(if_none_match, &head) { return Ok((304, Some(head), Value::Null)); }
            if let Some(rec) = rec {
                let running = state
                    .active_turns
                    .lock()
                    .ok()
                    .is_some_and(|active| active.contains(session_id));
                let folded = research_tools::map_fold::fold_map(&rec.skeleton, &events, running);
                let mut mv = build_mapview_json_with_fold(
                    &rec.skeleton,
                    rec.event_seq,
                    &rec.blob_hash,
                    &head,
                    None,
                    Some(&folded),
                );
                let inv_settings = crate::investigation_settings::load_settings(&state.cfg.data_dir);
                crate::investigation_mapview_v7::enrich_mapview_with_v7(
                    &mut mv,
                    &rec.skeleton,
                    &folded,
                    inv_settings.orient_auto_start,
                );
                mv["orient_call_cap"] = json!(inv_settings.orient_call_cap);
                crate::investigation_context_tasks::enrich_context_cards_metadata(
                    &mut mv,
                    state,
                    session_id,
                    &events,
                );
                if rec.skeleton.builder == SkeletonBuilder::Pdf1 {
                    if let Some(receipt) = find_source_receipt(&events, &rec.skeleton.source_id, Some(rec.skeleton.source_seq))
                        .or_else(|| find_source_receipt(&events, &rec.skeleton.source_id, None))
                    {
                        if let Ok(env) = state.research_env_for_session(session_id) {
                            if let Ok(pdf) = load_ingested_pdf_source(&env.sandbox(), session_id, receipt) {
                                attach_pdf_page_metadata(&mut mv, &rec.skeleton, &pdf);
                                crate::investigation_mapview_v7::attach_v7_pdf_quote_pages(&mut mv, &pdf);
                            }
                        }
                    }
                }
                crate::investigation_handoff::attach_child_statuses_to_mapview(&ledger, &events, &mut mv);
                crate::investigation_legacy_view::embed_legacy_into_mapview(&mut mv, &events, Some(&rec.skeleton));
                return Ok((200, Some(head), mv));
            }
            let mut mv = json!({
                "etag": head.clone(), "skeleton_seq": Value::Null, "nodes": [], "edges": [],
                "legend": [], "fallback_note": fallback_note(&events),
            });
            crate::investigation_legacy_view::embed_legacy_into_mapview(&mut mv, &events, None);
            Ok((200, Some(head), mv))
        }
        ("GET", "node-text") => {
            let tag = query.get("tag").map(String::as_str).map(str::trim).filter(|s| !s.is_empty())
                .ok_or_else(|| bad("Provide `tag` query parameter."))?;
            let rec = load_skeleton_for_query(state, session_id, parse_opt_u64(query, "seq")?)?
                .ok_or_else(|| not_found("no skeleton available for this session"))?;
            let (_, events) = open_and_read(state, session_id)?;
            let receipt = find_source_receipt(&events, &rec.skeleton.source_id, Some(rec.skeleton.source_seq))
                .or_else(|| find_source_receipt(&events, &rec.skeleton.source_id, None))
                .ok_or_else(|| not_found(format!("source receipt `{}` not found", rec.skeleton.source_id)))?;
            let env = state.research_env_for_session(session_id).map_err(internal)?;
            let val = read_verified_node_text(&env.sandbox(), session_id, receipt, &rec.skeleton, tag)
                .map_err(|m| if m.to_ascii_lowercase().contains("not found") { not_found(m) } else { internal(m) })?;
            Ok((200, Some(rec.blob_hash), val))
        }
        _ => Err(not_found(format!(
            "unknown investigation skeleton endpoint: {method} /api/sessions/{session_id}/investigation/{op}"
        ))),
    }
}

fn load_skeleton_for_query(
    state: &DesktopState, session_id: &str, requested_seq: Option<u64>,
) -> Result<Option<SkeletonRecord>, (u16, String)> {
    let (ledger, events) = open_and_read(state, session_id)?;
    if let Some(seq) = requested_seq {
        let ev = events.iter().find(|e| e.seq == seq && e.kind == EVENT_INVESTIGATION_SKELETON)
            .ok_or_else(|| not_found(format!("no skeleton at seq {seq}")))?;
        return load_or_repair_skeleton(state, &ledger, session_id, ev, receipt_for_skeleton_event(&events, ev)).map(Some);
    }
    if latest(&events, EVENT_INVESTIGATION_PAPER).is_some() {
        drop(ledger);
        return ensure_skeleton_on_open(state, session_id);
    }
    match latest(&events, EVENT_INVESTIGATION_SKELETON) {
        Some(ev) => load_or_repair_skeleton(state, &ledger, session_id, ev, receipt_for_skeleton_event(&events, ev)).map(Some),
        None => Ok(None),
    }
}

pub(crate) fn build_skeleton_from_receipt(
    sandbox: &research_tools::sandbox::WorkspaceSandbox,
    session_id: &str,
    receipt: &Event,
    paper_key: PaperKey,
    version: Option<String>,
) -> Result<(PaperSkeleton, BuildTiming), String> {
    if is_tex_receipt(receipt) {
        let (out, timing) = build_tex_skeleton_from_ingested(sandbox, session_id, receipt, paper_key, version)?;
        Ok((out.skeleton, timing))
    } else if is_html_receipt(receipt) {
        let (out, timing) = build_html_skeleton_from_ingested(sandbox, session_id, receipt, paper_key, version)?;
        Ok((out.skeleton, timing))
    } else if is_pdf_receipt(receipt) {
        let (out, timing) = build_pdf_skeleton_from_ingested(sandbox, session_id, receipt, paper_key, version)?;
        Ok((out.skeleton, timing))
    } else {
        Err("source is neither TeX, LaTeXML HTML, nor PDF with an extracted page map".to_string())
    }
}

fn build_and_append_new(
    state: &DesktopState, ledger: &Ledger, session_id: &str, receipt: &Event,
    key_override: Option<PaperKey>, version_override: Option<String>, swallow_source_err: bool,
) -> Result<Option<SkeletonRecord>, (u16, String)> {
    if !is_skeleton_receipt(receipt) {
        return Err((422, "source is neither TeX nor a page-mapped PDF".to_string()));
    }
    let (paper_key, version) = derive_paper_identity(receipt, key_override, version_override).map_err(bad)?;
    let env = state.research_env_for_session(session_id).map_err(internal)?;
    let (skel, timing) =
        match build_skeleton_from_receipt(&env.sandbox(), session_id, receipt, paper_key, version) {
            Ok(pair) => pair,
            Err(_) if swallow_source_err => return Ok(None),
            Err(err) => return Err(internal(err)),
        };
    let json_bytes = serde_json::to_vec(&skel).map_err(storage)?;
    let blob_hash = ledger.put_blob(&json_bytes).map_err(storage)?;
    let sections = skel.nodes.iter().filter(|n| n.kind == SkNodeKind::Section).count();
    let results = skel.nodes.iter().filter(|n| n.kind.is_theorem_like()).count();
    let equations = skel.nodes.iter().filter(|n| n.kind == SkNodeKind::Equation).count();
    let payload = json!({
        "source_id": skel.source_id, "source_seq": skel.source_seq, "builder": skel.builder,
        "files_root_sha256": skel.files_root_sha256, "paper_key": skel.paper_key,
        "version": skel.version, "quality": skel.quality, "quality_reason": skel.quality_reason,
        "counts": {
            "nodes": skel.nodes.len(), "edges": skel.edges.len(), "bib": skel.bib.len(),
            "sections": sections, "results": results, "equations": equations,
        },
        "blob": blob_hash, "prev_skeleton_seq": Value::Null, "carry_counts": Value::Null,
    });
    let _lock = state.ledger_write_lock.lock().map_err(|_| internal("ledger lock poisoned"))?;
    let event_seq = ledger.append(session_id, EVENT_INVESTIGATION_SKELETON, 1, &payload).map_err(storage)?;
    Ok(Some(SkeletonRecord { event_seq, blob_hash, skeleton: skel, timing: Some(timing), rebuilt: false }))
}

fn load_or_repair_skeleton(
    state: &DesktopState, ledger: &Ledger, session_id: &str, ev: &Event, receipt: Option<&Event>,
) -> Result<SkeletonRecord, (u16, String)> {
    let blob_hash = ev.payload["blob"].as_str()
        .ok_or_else(|| internal("stored investigation.skeleton missing `blob` hash"))?;
    // Rule 6.13: keep stored skeleton unchanged when its blob matches the recorded digest.
    if let Some(bytes) = ledger.get_blob(blob_hash).ok().filter(|b| ledger::sha256_hex(b) == blob_hash) {
        let skeleton: PaperSkeleton = serde_json::from_slice(&bytes).map_err(storage)?;
        return Ok(SkeletonRecord {
            event_seq: ev.seq, blob_hash: blob_hash.to_string(), skeleton, timing: None, rebuilt: false,
        });
    }
    // Rule 6.2: missing or corrupt blob triggers a deterministic rebuild from hash-pinned files.
    let receipt = receipt.ok_or_else(|| {
        internal("skeleton blob mismatch: source.ingested receipt missing for rebuild")
    })?;
    let (paper_key, version) = match ev.payload["paper_key"].as_str().and_then(|s| PaperKey::parse(s).ok()) {
        Some(k) => (k, ev.payload["version"].as_str().map(str::to_string)),
        None => derive_paper_identity(receipt, None, None).map_err(internal)?,
    };
    let env = state.research_env_for_session(session_id).map_err(internal)?;
    let (rebuilt_skel, timing) =
        build_skeleton_from_receipt(&env.sandbox(), session_id, receipt, paper_key, version)
            .map_err(|e| internal(format!("skeleton blob mismatch and rebuild failed: {e}")))?;
    let rebuilt_bytes = serde_json::to_vec(&rebuilt_skel).map_err(storage)?;
    if ledger::sha256_hex(&rebuilt_bytes) != blob_hash {
        return Err(internal(
            "skeleton blob mismatch: rebuild from hash-pinned source produced a different digest",
        ));
    }
    if blob_hash.len() == 64 && is_lower_hex_prefix(blob_hash, 64) {
        let _ = std::fs::remove_file(state.cfg.data_dir.join("blobs").join(&blob_hash[..2]).join(blob_hash));
    }
    ledger.put_blob(&rebuilt_bytes).map_err(storage)?;
    Ok(SkeletonRecord {
        event_seq: ev.seq, blob_hash: blob_hash.to_string(),
        skeleton: rebuilt_skel, timing: Some(timing), rebuilt: true,
    })
}

pub(crate) fn open_and_read(state: &DesktopState, session_id: &str) -> Result<(Ledger, Vec<Event>), (u16, String)> {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    let ledger = state.open_ledger().map_err(internal)?;
    let events = ledger.read_session(session_id).map_err(storage)?;
    Ok((ledger, events))
}
fn storage(err: impl ToString) -> (u16, String) { internal(err.to_string()) }
fn non_empty(v: &Value) -> Option<&str> { v.as_str().map(str::trim).filter(|s| !s.is_empty()) }
fn is_lower_hex_prefix(s: &str, n: usize) -> bool {
    s.len() >= n && s.as_bytes()[..n].iter().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}
pub(crate) fn find_source_receipt<'a>(events: &'a [Event], source_id: &str, seq: Option<u64>) -> Option<&'a Event> {
    events.iter().rev().find(|e| {
        e.kind == EVENT_SOURCE_INGESTED && e.payload["source_id"].as_str() == Some(source_id) && seq.is_none_or(|s| e.seq == s)
    })
}
fn receipt_for_skeleton_event<'a>(events: &'a [Event], ev: &Event) -> Option<&'a Event> {
    let sid = ev.payload["source_id"].as_str()?;
    ev.payload["source_seq"].as_u64()
        .and_then(|seq| find_source_receipt(events, sid, Some(seq)))
        .or_else(|| find_source_receipt(events, sid, None))
}
fn latest_skeleton_for_source<'a>(events: &'a [Event], source_id: &str, seq: u64) -> Option<&'a Event> {
    events.iter().rev().find(|e| {
        e.kind == EVENT_INVESTIGATION_SKELETON
            && e.payload["source_id"].as_str() == Some(source_id)
            && e.payload["source_seq"].as_u64() == Some(seq)
    })
}
fn parse_opt_u64(query: &HashMap<String, String>, key: &str) -> Result<Option<u64>, (u16, String)> {
    match query.get(key).map(String::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => raw.parse::<u64>().map(Some).map_err(|_| bad(format!("`{key}` must be a positive integer"))),
        None => Ok(None),
    }
}
fn etag_matches(if_none_match: Option<&str>, etag: &str) -> bool {
    let Some(raw) = if_none_match.filter(|_| !etag.is_empty()) else { return false };
    raw.split(',').any(|part| {
        let t = part.trim();
        t == "*" || t.strip_prefix("W/").or_else(|| t.strip_prefix("w/")).unwrap_or(t).trim().trim_matches('"') == etag
    })
}
pub(crate) fn fallback_note(events: &[Event]) -> &'static str {
    let Some(sid) = latest(events, EVENT_INVESTIGATION_PAPER).and_then(|e| e.payload["source_id"].as_str()) else {
        return "No primary paper is pinned to this Investigation yet.";
    };
    match find_source_receipt(events, sid, None) {
        None => "Pinned source receipt is missing; showing the earlier view.",
        Some(r) if !is_skeleton_receipt(r) => "Pinned source is plain text or lacks a PDF page map; showing the earlier view.",
        Some(_) => "Ingested source files changed or are missing on disk; showing the earlier view.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthGuard;
    use crate::chatgpt::{offline_test_hooks, ChatGptHub};
    use crate::sse::SseHub;
    use crate::DesktopConfig;
    use research_tools::{ingest_bytes, IngestRequest, LiteratureEndpoints, ResearchEnv};
    use std::sync::{Arc, Mutex};

    const TEX: &[u8] = b"\\documentclass{article}\n\\newtheorem{theorem}{Theorem}\n\\begin{document}\n\\section{Intro}\\label{sec:i}\n\\begin{theorem}\\label{thm:m}\nBounded sequences have convergent subsequences.\n\\end{theorem}\n\\end{document}\n";

    fn mk_state(dir: &std::path::Path) -> DesktopState {
        let cfg = DesktopConfig {
            port: 0, data_dir: dir.to_path_buf(), workspace_root: None, python: None,
            worker_script: None, api_base_override: None,
            literature_endpoints: LiteratureEndpoints::default(), open_window: false,
            print_url_json: false, eval_timeout_s: 30, core_exe: None, claude_exe: None,
            agy_exe: None, connection_env: Some(Default::default()),
            chatgpt_test: Some(offline_test_hooks()),
        };
        let chatgpt = ChatGptHub::new(&cfg).unwrap();
        DesktopState::new(cfg, AuthGuard::new(0).unwrap(), Arc::new(SseHub::new()), chatgpt)
    }

    #[test]
    fn helpers_identity_and_etag() {
        let ev = |payload| Event {
            session: "s".into(), seq: 1, ts_ms: 0, kind: EVENT_SOURCE_INGESTED.into(),
            schema_version: 1, payload, prev_hash: String::new(), event_hash: String::new(),
        };
        let tex = ev(json!({
            "source_id": "s1", "primary_path": "p/main.tex", "assembled_path": "p/all.tex",
            "files": ["p/main.tex", "p/all.tex"], "origin": "https://arxiv.org/e-print/2103.01234v2",
        }));
        assert!(is_tex_receipt(&tex));
        let (k, v) = derive_paper_identity(&tex, None, None).unwrap();
        assert_eq!((k.as_str(), v.as_deref()), ("arxiv:2103.01234", Some("v2")));
        assert_eq!(derive_paper_identity(&tex, None, Some("v3".into())).unwrap().1.as_deref(), Some("v3"));
        let non = ev(json!({
            "source_id": "s2", "primary_path": "p/doc.txt", "assembled_path": "p/all.tex",
            "files": ["p/doc.txt", "p/all.tex"], "raw_blob": "a".repeat(64),
        }));
        assert!(!is_tex_receipt(&non));
        assert_eq!(derive_paper_identity(&non, None, None).unwrap().0.as_str(), format!("file:{}", "a".repeat(16)));
        assert!(etag_matches(Some("W/\"abc\""), "abc") && !etag_matches(Some("\"abc\""), ""));
    }

    #[test]
    fn pin_repair_fallback_and_routes() {
        let tmp = tempfile::tempdir().unwrap();
        let state = mk_state(tmp.path());
        let env = ResearchEnv::new(tmp.path().to_path_buf());
        let ledger = Mutex::new(state.open_ledger().unwrap());
        let (sid, old_sid) = ("inv-skel-1", "inv-old-1");
        ledger.lock().unwrap().append(sid, EVENT_INVESTIGATION_STARTED, 1, &json!({})).unwrap();
        let src = ingest_bytes(&env.sandbox(), &ledger, &IngestRequest {
            session_id: sid, file_name: "paper.tex", origin: "https://arxiv.org/e-print/2401.00001v1",
            raw_bytes: TEX, source_id_hint: None, section_filter: None, title: Some("T"),
        }).unwrap();
        let txt = ingest_bytes(&env.sandbox(), &ledger, &IngestRequest {
            session_id: sid, file_name: "note.txt", origin: "upload",
            raw_bytes: b"plain", source_id_hint: None, section_filter: None, title: None,
        }).unwrap();
        ledger.lock().unwrap().append(old_sid, EVENT_INVESTIGATION_STARTED, 1, &json!({})).unwrap();
        let old = ingest_bytes(&env.sandbox(), &ledger, &IngestRequest {
            session_id: old_sid, file_name: "paper.tex", origin: "upload",
            raw_bytes: TEX, source_id_hint: None, section_filter: None, title: None,
        }).unwrap();
        ledger.lock().unwrap().append(old_sid, EVENT_INVESTIGATION_PAPER, 1, &json!({"source_id": old.source_id})).unwrap();
        drop(ledger);

        assert_eq!(build_and_record_skeleton(&state, sid, &txt.source_id, None, None).unwrap_err().0, 422);
        let rec = pin_source_and_build_skeleton(&state, sid, &src.source_id, None, None, Some("Q")).unwrap();
        assert!(!rec.rebuilt && pin_source_and_build_skeleton(&state, sid, &txt.source_id, None, None, None).unwrap_err().0 == 409);
        assert!(!state.open_ledger().unwrap().read_session(sid).unwrap().iter().any(|e| e.kind == "model.request"));

        let blob = tmp.path().join("blobs").join(&rec.blob_hash[..2]).join(&rec.blob_hash);
        std::fs::write(&blob, b"corrupt").unwrap();
        assert!(build_and_record_skeleton(&state, sid, &src.source_id, None, None).unwrap().rebuilt);
        assert_eq!(handle_with_etag(&state, "GET", sid, "skeleton", &HashMap::new(), &[], Some(&rec.blob_hash)).unwrap().0, 304);
        let (st, head, mv) = handle_with_etag(&state, "GET", sid, "mapview", &HashMap::new(), &[], None).unwrap();
        assert_eq!(st, 200);
        assert!(mv["nodes"].is_array() && handle_with_etag(&state, "GET", sid, "mapview", &HashMap::new(), &[], head.as_deref()).unwrap().0 == 304);
        let tag = rec.skeleton.nodes.iter().find_map(|n| n.tag.clone()).unwrap();
        assert_eq!(handle(&state, "GET", sid, "node-text", &HashMap::from([("tag".into(), tag)]), &[]).unwrap().0, 200);

        let p = tmp.path().join("sessions").join(old_sid).join("sources").join(old.primary_path.as_ref().unwrap());
        let orig = std::fs::read(&p).unwrap();
        std::fs::write(&p, b"tampered").unwrap();
        assert!(ensure_skeleton_on_open(&state, old_sid).unwrap().is_none());
        let (_, _, fmv) = handle_with_etag(&state, "GET", old_sid, "mapview", &HashMap::new(), &[], None).unwrap();
        assert!(fmv["skeleton_seq"].is_null() && fmv["fallback_note"].is_string());
        std::fs::write(&p, &orig).unwrap();
        state.open_ledger().unwrap().append(old_sid, EVENT_INVESTIGATION_SKELETON, 1, &json!({
            "source_id": old.source_id, "source_seq": old.event_seq, "blob": "0".repeat(64),
        })).unwrap();
        assert_eq!(ensure_skeleton_on_open(&state, old_sid).unwrap_err().0, 500);
    }
}
