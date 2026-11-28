//! Explore outgoing F7 hand-off builders (`"Open in Solve"` and `"Investigate this paper"`)
//! and read-only child status inspection (package X13).
//!
//! Governance: `00-overview.md` 3.1 row 9, 4.1–4.3 (F7), `explore.md` 4.2, 5.4, 9 (X13).
//! Strictly zero model calls and zero network calls. Delegates child session creation and
//! `handoff.sent` writing to F7's [`crate::handoff::handle_create_handoff`] so every `QuoteRef`
//! is re-matched against the child's own copied sources and badges/verdicts never travel.

use crate::handoff::handle_create_handoff;
use crate::idea::{
    latest_statement_seq, EVENT_IDEA_BRIEF, EVENT_IDEA_CANDIDATES, EVENT_IDEA_DIRECTIONS,
    EVENT_IDEA_MAP, EVENT_IDEA_NOTE, EVENT_IDEA_PAPER,
};
use crate::idea_view;
use crate::routes_session::{bad, internal, not_found, parse_json_body, ApiResult};
use crate::state::DesktopState;
use export::handoff::{
    read_session_handoffs, ContextRole, HandoffAsk, HandoffContextItem, HandoffPacket,
    HandoffRequest, HandoffStatement, InvestigateHandoffAsk, ModelTextEntry, SessionHandoffsView,
    SolveHandoffAsk, MAX_ASK_ITEMS, MAX_CONTEXT_ITEMS, MAX_CONTEXT_TEXT_CHARS,
    MAX_CONVENTIONS_CHARS, MAX_MODEL_TEXT_CHARS, MAX_MODEL_TEXT_ITEMS, MAX_PAPERS,
    MAX_STATEMENT_CHARS, MAX_TITLE_CHARS, MAX_USER_NOTES, MAX_USER_NOTE_CHARS,
};
use ledger::{sha256_hex, Event, Ledger};
use research_tools::idea_directions::{Direction, IdeaDirectionsPayload};
use research_tools::idea_map::{IdeaMapPayload, MapNode};
use research_tools::paper_key::{Mode, NodeRef, PaperIds, PaperKey, PaperRef};
use research_tools::quote_core::{QuoteRef, EVIDENCE_QUOTE_EVENT};
use research_tools::solve_model::{DoneCriterion, DoneCriterionKind, SolveTarget};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// `POST /api/sessions/{id}/idea/handoff`: build an F7 [`HandoffRequest`] for either:
/// - `action: "open_in_solve"` (Concrete or Loose direction `direction_id: "r-N"` -> Solve)
/// - `action: "investigate_paper"` (`paper_key`, optional `node_id` -> Investigate)
///
/// Delegates child session creation + `handoff.sent` directly to F7 (`handoff.rs`).
pub fn handle_idea_handoff_op(
    state: &DesktopState,
    ledger: &Ledger,
    session_id: &str,
    body: &[u8],
) -> ApiResult {
    let val = parse_json_body(body)?;
    let action = val.get("action").or_else(|| val.get("kind")).and_then(Value::as_str).map(str::trim).unwrap_or("");
    let events = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;

    let req = match action {
        "open_in_solve" | "solve" | "direction" => {
            let direction_id = val.get("direction_id").or_else(|| val.get("id"))
                .and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
                .ok_or_else(|| bad("`direction_id` is required for `open_in_solve` hand-off"))?;
            build_open_in_solve_request(session_id, &events, direction_id)?
        }
        "investigate_paper" | "investigate" | "paper" => {
            let paper_key = val.get("paper_key").or_else(|| val.get("key"))
                .and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
                .ok_or_else(|| bad("`paper_key` is required for `investigate_paper` hand-off"))?;
            let node_id = val.get("node_id").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
            build_investigate_paper_request(session_id, &events, paper_key, node_id)?
        }
        other => return Err(bad(format!(
            "unknown Explore handoff action `{other}` (expected `open_in_solve` or `investigate_paper`)"
        ))),
    };

    let req_bytes = serde_json::to_vec(&req).map_err(|e| internal(e.to_string()))?;
    let (status, mut resp) = handle_create_handoff(state, session_id, &req_bytes)?;
    let updated = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
    if let Some(obj) = resp.as_object_mut() {
        obj.insert("idea".into(), idea_view::view(&updated, false));
    }
    Ok((status, resp))
}

/// Strictly read-only child status lookup (`explore.md` 4.2, `00-overview.md` 4.1 Rule 5).
pub fn read_explore_handoffs(ledger: &Ledger, session_id: &str) -> Result<SessionHandoffsView, String> {
    read_session_handoffs(ledger, session_id)
}

/// Deterministically construct an F7 [`HandoffRequest`] to open a Concrete or Loose Explore
/// direction in Solve (`00-overview.md` 3.1 row 9 & 4.2, `explore.md` 4.2).
pub fn build_open_in_solve_request(
    session_id: &str,
    events: &[Event],
    direction_id: &str,
) -> Result<HandoffRequest, (u16, String)> {
    let stmt_seq = latest_statement_seq(events);
    let (direction, dir_map_seq) = find_active_direction(events, stmt_seq, direction_id)?;
    let nodes_by_id = load_map_nodes_at_seq(events, dir_map_seq);
    let evidence_by_id = index_verified_quotes(events, stmt_seq);
    let paper_refs = index_session_paper_refs(events);
    let paper_sources = index_paper_source_ids(events);

    let mut quote_texts: BTreeMap<String, String> = BTreeMap::new();
    let mut context: Vec<HandoffContextItem> = Vec::new();
    let mut extra_source_ids: BTreeSet<String> = BTreeSet::new();
    let mut cited_node_ids: BTreeSet<String> = BTreeSet::new();
    let mut known_paper_keys: Vec<String> = Vec::new();

    for known in &direction.known {
        if context.len() >= MAX_CONTEXT_ITEMS { break; }
        let mut quotes = Vec::new();
        let mut first_node_id = String::new();
        let mut first_paper_ref: Option<PaperRef> = None;
        for nid in &known.node_ids {
            cited_node_ids.insert(nid.clone());
            if first_node_id.is_empty() { first_node_id = nid.clone(); }
            if let Some(node) = nodes_by_id.get(nid.as_str()) {
                for pk in &node.papers {
                    known_paper_keys.push(pk.clone());
                    if first_paper_ref.is_none() {
                        first_paper_ref = resolve_paper_ref(pk, &paper_refs);
                    }
                }
                for ev_id in &node.evidence {
                    if let Some((qref, qtext)) = evidence_by_id.get(ev_id.as_str()) {
                        quote_texts.entry(qref.quote_sha256.clone()).or_insert_with(|| qtext.clone());
                        extra_source_ids.insert(qref.source_id.clone());
                        if !quotes.contains(qref) { quotes.push(qref.clone()); }
                    }
                }
            }
        }
        context.push(HandoffContextItem {
            role: ContextRole::KnownResult,
            from: NodeRef {
                session: session_id.to_string(),
                mode: Mode::Explore,
                id: if first_node_id.is_empty() { direction.id.clone() } else { first_node_id },
                paper: first_paper_ref,
                node_fp: None,
            },
            text: clip_chars(&known.text, MAX_CONTEXT_TEXT_CHARS),
            quotes,
        });
    }
    for nid in &direction.first_calculation.inputs { cited_node_ids.insert(nid.clone()); }
    for r in &direction.risks {
        for nid in &r.node_ids { cited_node_ids.insert(nid.clone()); }
    }

    // Union `solve_setup.source_keys` with papers from cited `known` nodes (deduped, capped at MAX_PAPERS).
    let mut papers: Vec<PaperRef> = Vec::new();
    let mut seen_pkeys: BTreeSet<String> = BTreeSet::new();
    for key_str in direction.solve_setup.source_keys.iter().chain(known_paper_keys.iter()) {
        if papers.len() >= MAX_PAPERS { break; }
        if let Some(pref) = resolve_paper_ref(key_str, &paper_refs) {
            let canonical = pref.key.to_string();
            if seen_pkeys.insert(canonical.clone()) {
                if let Some(sids) = paper_sources.get(&canonical) {
                    for sid in sids { extra_source_ids.insert(sid.clone()); }
                }
                papers.push(pref);
            }
        }
    }

    let mut suggested_done: Vec<DoneCriterion> = Vec::new();
    let mut check_first: Vec<String> = Vec::new();
    let check_text = direction.first_calculation.check.text.trim();
    if !check_text.is_empty() {
        check_first.push(clip_chars(check_text, MAX_MODEL_TEXT_CHARS));
    }
    if let Some(ref repro) = direction.first_calculation.check.reproduces {
        let mut params = json!({
            "check": sanitize_solve_check_name(repro),
            "reproduces": repro,
            "method": direction.first_calculation.method.as_str(),
        });
        if !check_text.is_empty() {
            params["description"] = Value::String(clip_chars(check_text, 400));
        }
        suggested_done.push(DoneCriterion {
            id: "check-first".into(),
            kind: DoneCriterionKind::NamedCheckPassed,
            params,
        });
    } else if !check_text.is_empty() {
        let clean = sanitize_confirm_text(check_text, 400);
        if !clean.is_empty() {
            suggested_done.push(DoneCriterion {
                id: "check-first".into(),
                kind: DoneCriterionKind::UserConfirms,
                params: json!({ "text": clean, "method": direction.first_calculation.method.as_str() }),
            });
        }
    }

    let risks: Vec<String> = direction.risks.iter().take(MAX_ASK_ITEMS)
        .map(|r| clip_chars(&format!("[{}] {}", r.kind.as_str(), r.text), MAX_MODEL_TEXT_CHARS))
        .collect();
    let stop_if: Vec<String> = if direction.stop_if.trim().is_empty() {
        Vec::new()
    } else {
        vec![clip_chars(&direction.stop_if, MAX_MODEL_TEXT_CHARS)]
    };

    // Direction prose travels strictly in `model_text` (shown as `"(model)"`, never as checked facts).
    let mut model_text: Vec<ModelTextEntry> = Vec::new();
    push_model_note(&mut model_text, "Direction question", &direction.question);
    if !direction.first_calculation.setup.trim().is_empty() {
        push_model_note(
            &mut model_text,
            &format!("First calculation ({})", direction.first_calculation.method.as_str()),
            &direction.first_calculation.setup,
        );
    }
    push_model_note(&mut model_text, "Expected form", &direction.first_calculation.expected_form);
    for m in &direction.missing {
        push_model_note(&mut model_text, &format!("Missing ({})", m.basis.format_wire()), &m.text);
    }

    let user_notes = collect_explore_user_notes(
        events,
        stmt_seq,
        Some(direction.id.as_str()),
        &cited_node_ids,
        &seen_pkeys,
    );
    let fp_src = format!("{stmt_seq}:{dir_map_seq}:{}:{}", direction.id, direction.title);
    let node_fp = Some(sha256_hex(fp_src.as_bytes())[..16].to_string());

    let packet = HandoffPacket {
        from: NodeRef {
            session: session_id.to_string(),
            mode: Mode::Explore,
            id: direction.id.clone(),
            paper: papers.first().cloned(),
            node_fp,
        },
        target_mode: Mode::Solve,
        title: non_empty_clip(&direction.title, MAX_TITLE_CHARS, &direction.id),
        statement: HandoffStatement {
            text: clip_chars(&direction.solve_setup.statement, MAX_STATEMENT_CHARS),
            quotes: Vec::new(),
        },
        context,
        papers,
        conventions: clip_chars(&direction.solve_setup.conventions, MAX_CONVENTIONS_CHARS),
        model_text,
        user_notes,
        ask: HandoffAsk::Solve(SolveHandoffAsk {
            task_kind: direction.solve_setup.task_kind,
            target: SolveTarget {
                description: non_empty_clip(
                    &direction.solve_setup.target.description,
                    MAX_STATEMENT_CHARS,
                    &direction.first_calculation.expected_form,
                ),
                form: direction.solve_setup.target.form,
                order: None,
                digits: None,
            },
            symbols: Vec::new(),
            suggested_done,
            check_first,
            risks,
            stop_if,
        }),
    };

    Ok(HandoffRequest {
        packet,
        quote_texts,
        extra_source_ids: extra_source_ids.into_iter().collect(),
    })
}

/// Deterministically construct an F7 [`HandoffRequest`] to investigate a paper from an Explore
/// paper row or map node (`00-overview.md` 4.2, `explore.md` 4.2).
pub fn build_investigate_paper_request(
    session_id: &str,
    events: &[Event],
    paper_key_raw: &str,
    node_id: Option<&str>,
) -> Result<HandoffRequest, (u16, String)> {
    let parsed_key = PaperKey::parse(paper_key_raw.trim())
        .map_err(|_| bad(format!("invalid canonical paper_key `{paper_key_raw}`")))?;
    let canonical_key = parsed_key.to_string();
    let stmt_seq = latest_statement_seq(events);
    let (evidence_floor, active_map_seq) = effective_map_scope(events, stmt_seq);
    let paper_refs = index_session_paper_refs(events);
    let paper_sources = index_paper_source_ids(events);
    let paper_ref = paper_refs.get(&canonical_key).cloned().unwrap_or(PaperRef {
        key: parsed_key,
        version: None,
        ids: PaperIds::default(),
    });

    let mut quote_texts: BTreeMap<String, String> = BTreeMap::new();
    let mut earlier_quotes: Vec<QuoteRef> = Vec::new();
    let mut extra_source_ids: BTreeSet<String> = BTreeSet::new();
    let mut model_text: Vec<ModelTextEntry> = Vec::new();

    if let Some(sids) = paper_sources.get(&canonical_key) {
        for sid in sids { extra_source_ids.insert(sid.clone()); }
    }

    for ev in events.iter().filter(|e| e.kind == EVIDENCE_QUOTE_EVENT && e.seq > evidence_floor) {
        let ev_pk = ev.payload["paper_key"].as_str().and_then(|s| PaperKey::parse(s).ok()).map(|k| k.to_string());
        if ev_pk.as_deref() != Some(canonical_key.as_str()) { continue; }
        if let Some((qref, qtext)) = parse_quote_event(ev) {
            if earlier_quotes.len() < MAX_CONTEXT_ITEMS && !earlier_quotes.contains(&qref) {
                extra_source_ids.insert(qref.source_id.clone());
                quote_texts.entry(qref.quote_sha256.clone()).or_insert(qtext);
                earlier_quotes.push(qref);
            }
            if let Some(gist) = ev.payload["gist"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
                push_model_note(&mut model_text, &format!("Quote gist xe-{}", ev.seq), gist);
            }
        }
    }

    let mut note_nodes = BTreeSet::new();
    if let Some(nid) = node_id {
        note_nodes.insert(nid.to_string());
        if let Some(mseq) = active_map_seq {
            let nodes_by_id = load_map_nodes_at_seq(events, mseq);
            if let Some(node) = nodes_by_id.get(nid) {
                push_model_note(&mut model_text, &format!("Map node {} summary", node.id), &node.statement);
            }
        }
    }

    let focus_claims = collect_idea_focus_claims(events, stmt_seq, evidence_floor);
    let statement_text = focus_claims.first().cloned().unwrap_or_else(|| format!("Investigate {canonical_key}"));
    let title_raw = find_paper_title(events, &canonical_key).unwrap_or_else(|| format!("Investigate {canonical_key}"));
    let mut note_papers = BTreeSet::new();
    note_papers.insert(canonical_key.clone());
    let user_notes = collect_explore_user_notes(events, evidence_floor, None, &note_nodes, &note_papers);

    let packet = HandoffPacket {
        from: NodeRef {
            session: session_id.to_string(),
            mode: Mode::Explore,
            id: node_id.unwrap_or(&canonical_key).to_string(),
            paper: Some(paper_ref.clone()),
            node_fp: None,
        },
        target_mode: Mode::Investigate,
        title: non_empty_clip(&title_raw, MAX_TITLE_CHARS, &canonical_key),
        statement: HandoffStatement { text: clip_chars(&statement_text, MAX_STATEMENT_CHARS), quotes: Vec::new() },
        context: Vec::new(),
        papers: vec![paper_ref.clone()],
        conventions: String::new(),
        model_text,
        user_notes,
        ask: HandoffAsk::Investigate(InvestigateHandoffAsk {
            paper: paper_ref,
            focus_claims,
            earlier_quotes,
        }),
    };

    Ok(HandoffRequest {
        packet,
        quote_texts,
        extra_source_ids: extra_source_ids.into_iter().collect(),
    })
}

/// Resolve the active map sequence and evidence floor: if `map_stale` is true (an `idea.map`
/// exists prior to `stmt_seq` and no newer map exists after `stmt_seq`), preserve that map's
/// revision floor so stale-map `"Investigate this paper"` buttons still carry matched quotes.
fn effective_map_scope(events: &[Event], stmt_seq: u64) -> (u64, Option<u64>) {
    if let Some(cur) = events.iter().rev().find(|e| e.kind == EVENT_IDEA_MAP && e.seq > stmt_seq) {
        return (stmt_seq, Some(cur.seq));
    }
    if let Some(prev_map) = events.iter().rev().find(|e| e.kind == EVENT_IDEA_MAP) {
        let floor = events.iter().rev()
            .find(|e| e.kind == "idea.stated" && e.seq < prev_map.seq)
            .map_or(0, |e| e.seq);
        return (floor, Some(prev_map.seq));
    }
    (stmt_seq, None)
}

fn find_active_direction(events: &[Event], stmt_seq: u64, direction_id: &str) -> Result<(Direction, u64), (u16, String)> {
    let active_map_seq = events.iter().rev()
        .find(|e| e.kind == EVENT_IDEA_MAP && e.seq > stmt_seq)
        .map(|e| e.seq)
        .ok_or_else(|| not_found("no active map in the current Explore revision"))?;
    let dir_ev = events.iter().rev()
        .find(|e| e.kind == EVENT_IDEA_DIRECTIONS && e.seq > active_map_seq)
        .ok_or_else(|| not_found("no directions recorded for the current Explore map"))?;
    let payload: IdeaDirectionsPayload = serde_json::from_value(dir_ev.payload.clone())
        .map_err(|e| internal(format!("stored `idea.directions` payload is unreadable: {e}")))?;
    if payload.map_seq != active_map_seq {
        return Err(not_found("stored directions belong to an older map; re-run Directions"));
    }
    let dir = payload.directions.into_iter().find(|d| d.id == direction_id)
        .ok_or_else(|| not_found(format!("direction `{direction_id}` not found")))?;
    Ok((dir, payload.map_seq))
}

fn load_map_nodes_at_seq(events: &[Event], map_seq: u64) -> BTreeMap<String, MapNode> {
    let Some(map_ev) = events.iter().rev().find(|e| e.kind == EVENT_IDEA_MAP && e.seq == map_seq) else {
        return BTreeMap::new();
    };
    let Ok(payload) = serde_json::from_value::<IdeaMapPayload>(map_ev.payload.clone()) else {
        return BTreeMap::new();
    };
    payload.nodes.into_iter().map(|n| (n.id.clone(), n)).collect()
}

fn index_verified_quotes(events: &[Event], floor_seq: u64) -> BTreeMap<String, (QuoteRef, String)> {
    let mut out = BTreeMap::new();
    for ev in events.iter().filter(|e| e.kind == EVIDENCE_QUOTE_EVENT && e.seq > floor_seq) {
        if let Some(pair) = parse_quote_event(ev) { out.insert(format!("xe-{}", ev.seq), pair); }
    }
    out
}

fn parse_quote_event(ev: &Event) -> Option<(QuoteRef, String)> {
    let p = &ev.payload;
    let source_id = p.get("source_id")?.as_str()?.trim().to_string();
    let path = p.get("path")?.as_str()?.trim().to_string();
    let start_line = p.get("start_line")?.as_u64()? as usize;
    let end_line = p.get("end_line")?.as_u64()? as usize;
    let file_sha256 = p.get("file_sha256")?.as_str()?.trim().to_string();
    let quote_sha256 = p.get("quote_sha256")?.as_str()?.trim().to_string();
    let quote = p.get("quote")?.as_str()?.to_string();
    if source_id.is_empty() || path.is_empty() || file_sha256.is_empty() || quote_sha256.is_empty() {
        return None;
    }
    Some((QuoteRef { source_id, path, start_line, end_line, file_sha256, quote_sha256, evidence_seq: Some(ev.seq) }, quote))
}

fn collect_explore_user_notes(
    events: &[Event],
    floor_seq: u64,
    direction_id: Option<&str>,
    node_ids: &BTreeSet<String>,
    paper_keys: &BTreeSet<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    for ev in events.iter().filter(|e| e.kind == EVENT_IDEA_NOTE && e.seq > floor_seq) {
        if out.len() >= MAX_USER_NOTES { break; }
        let kind = ev.payload.get("target_kind").and_then(Value::as_str).unwrap_or("").trim();
        let target = ev.payload.get("target_id").and_then(Value::as_str).unwrap_or("").trim();
        let action = ev.payload.get("action").and_then(Value::as_str).unwrap_or("").trim();
        let comment = ev.payload.get("comment").and_then(Value::as_str).unwrap_or("").trim();
        let matches = (kind == "direction" && direction_id == Some(target))
            || (kind == "node" && node_ids.contains(target))
            || (kind == "paper" && paper_keys.contains(target));
        if !matches || action.is_empty() { continue; }
        let line = if comment.is_empty() {
            format!("[{kind}:{target} {action}]")
        } else {
            format!("[{kind}:{target} {action}] {comment}")
        };
        let clipped = clip_chars(&line, MAX_USER_NOTE_CHARS);
        if !clipped.is_empty() && !out.contains(&clipped) { out.push(clipped); }
    }
    out
}

fn index_session_paper_refs(events: &[Event]) -> BTreeMap<String, PaperRef> {
    let mut out = BTreeMap::new();
    for ev in events {
        if ev.kind == EVENT_IDEA_CANDIDATES {
            for c in ev.payload.get("candidates").and_then(Value::as_array).into_iter().flatten() {
                let Some(k) = c.get("key").and_then(Value::as_str).and_then(|s| PaperKey::parse(s).ok()) else { continue; };
                let ids: PaperIds = c.get("ids").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
                let version = c.get("version").and_then(Value::as_str).map(str::to_string);
                out.insert(k.to_string(), PaperRef { key: k, version, ids });
            }
        } else if ev.kind == EVENT_IDEA_PAPER {
            if let Some(k) = ev.payload.get("key").and_then(Value::as_str).and_then(|s| PaperKey::parse(s).ok()) {
                let version = ev.payload.get("version").and_then(Value::as_str).map(str::to_string);
                out.entry(k.to_string())
                    .and_modify(|pref| { if pref.version.is_none() { pref.version = version.clone(); } })
                    .or_insert(PaperRef { key: k, version, ids: PaperIds::default() });
            }
        }
    }
    out
}

fn index_paper_source_ids(events: &[Event]) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for ev in events.iter().filter(|e| e.kind == EVENT_IDEA_PAPER) {
        if let (Some(k), Some(sid)) = (
            ev.payload.get("key").and_then(Value::as_str).and_then(|s| PaperKey::parse(s).ok()),
            ev.payload.get("source_id").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()),
        ) {
            let entry = out.entry(k.to_string()).or_default();
            if !entry.iter().any(|x| x == sid) { entry.push(sid.to_string()); }
        }
    }
    out
}

fn resolve_paper_ref(raw_key: &str, known: &BTreeMap<String, PaperRef>) -> Option<PaperRef> {
    let key = PaperKey::parse(raw_key.trim()).ok()?;
    let canon = key.to_string();
    Some(known.get(&canon).cloned().unwrap_or(PaperRef { key, version: None, ids: PaperIds::default() }))
}

fn collect_idea_focus_claims(events: &[Event], stmt_seq: u64, fallback_floor: u64) -> Vec<String> {
    for floor in [stmt_seq, fallback_floor] {
        if let Some(brief_ev) = events.iter().rev().find(|e| e.kind == EVENT_IDEA_BRIEF && e.seq > floor) {
            let claims: Vec<String> = brief_ev.payload.get("subclaims").and_then(Value::as_array)
                .into_iter().flatten()
                .filter_map(|s| s.get("text").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()).map(str::to_string))
                .take(MAX_ASK_ITEMS).collect();
            if !claims.is_empty() { return claims; }
        }
    }
    events.iter().rev().find(|e| e.kind == "idea.stated")
        .and_then(|e| e.payload.get("text").and_then(Value::as_str))
        .map(str::trim).filter(|t| !t.is_empty())
        .map(|t| vec![clip_chars(t, MAX_CONTEXT_TEXT_CHARS)])
        .unwrap_or_default()
}

fn find_paper_title(events: &[Event], canonical_key: &str) -> Option<String> {
    for ev in events.iter().rev().filter(|e| e.kind == EVENT_IDEA_CANDIDATES) {
        for c in ev.payload.get("candidates").and_then(Value::as_array).into_iter().flatten() {
            let k = c.get("key").and_then(Value::as_str).and_then(|s| PaperKey::parse(s).ok()).map(|x| x.to_string());
            if k.as_deref() == Some(canonical_key) {
                if let Some(t) = c.get("title").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
                    return Some(t.to_string());
                }
            }
        }
    }
    None
}

fn sanitize_solve_check_name(repro: &str) -> String {
    let mut s = String::from("reproduce_");
    for ch in repro.chars() {
        if ch.is_ascii_alphanumeric() { s.push(ch.to_ascii_lowercase()); } else { s.push('_'); }
        if s.len() >= 64 { break; }
    }
    s
}

fn sanitize_confirm_text(raw: &str, max_chars: usize) -> String {
    raw.trim().chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\t').take(max_chars).collect()
}

fn push_model_note(out: &mut Vec<ModelTextEntry>, label: &str, text: &str) {
    let trimmed = text.trim();
    if trimmed.is_empty() || out.len() >= MAX_MODEL_TEXT_ITEMS { return; }
    out.push(ModelTextEntry { label: clip_chars(label, 120), text: clip_chars(trimmed, MAX_MODEL_TEXT_CHARS) });
}

fn clip_chars(s: &str, max_chars: usize) -> String { s.trim().chars().take(max_chars).collect() }

fn non_empty_clip(s: &str, max_chars: usize, fallback: &str) -> String {
    let c = clip_chars(s, max_chars);
    if c.is_empty() { clip_chars(fallback, max_chars) } else { c }
}
