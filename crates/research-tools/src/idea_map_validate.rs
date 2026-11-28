//! Validation, node-kind evidence rules (C4/C5/C6), stable node IDs (S4), Rule V8 prose check,
//! and `cites` edge synthesis for `record_idea_map` (package X11).

use crate::idea_map::{
    check_explore_prose, DroppedMapItem, IdeaMapPayload, MapEdge, MapEdgeBasis, MapEdgeKind,
    MapNode, MapNodeInput, MapNodeKind, MapNodeStatus, MapOverlap, RecordIdeaMapInput,
    EVENT_IDEA_CITATION_NET, EVENT_IDEA_MAP, MAX_EDGE_REASON_CHARS, MAX_MAP_EDGES, MAX_MAP_NODES,
    MAX_MAP_OVERLAPS, MAX_NODE_LABEL_CHARS, MAX_NODE_STATEMENT_CHARS, MAX_OVERLAP_DIFF_CHARS,
    RECORDER_IDEA_MAP,
};
use crate::paper_key::PaperKey;
use crate::prior_work::OverlapRelation;
use crate::quote_core::{
    parse_abstract_id, parse_quote_id, EvidenceRole, EVIDENCE_ABSTRACT_EVENT, EVIDENCE_QUOTE_EVENT,
};
use ledger::{sha256_hex, Event};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Context passed into [`validate_and_build_map`].
pub struct MapValidationInput<'a> {
    pub events: &'a [Event],
    pub idea_rev: u32,
    pub idea_text: &'a str,
    pub stmt_seq: u64,
    pub valid_subclaim_ids: &'a [String],
    pub request_seq: Option<u64>,
}

#[derive(Debug, Clone)]
struct SessionEvidenceEntry {
    is_full_text: bool,
    paper_key: Option<String>,
    role: Option<EvidenceRole>,
}

struct SessionMapContext {
    evidence: BTreeMap<String, SessionEvidenceEntry>,
    paper_years: BTreeMap<String, i32>,
    max_searched_year: Option<i32>,
    prior_id_by_identity: BTreeMap<(MapNodeKind, String), String>,
    prior_nodes_by_id: BTreeMap<String, MapNode>,
    noted_node_ids: BTreeSet<String>,
    max_seq_by_kind: BTreeMap<MapNodeKind, u64>,
    citation_pairs: BTreeSet<(String, String)>,
}

pub(crate) fn validate_and_build_map(
    ctx: MapValidationInput<'_>,
    raw_input: &Value,
) -> Result<IdeaMapPayload, Vec<String>> {
    let input: RecordIdeaMapInput = serde_json::from_value(raw_input.clone())
        .map_err(|e| vec![format!("invalid `{RECORDER_IDEA_MAP}` payload: {e}")])?;

    let mut top_errors = Vec::new();
    if input.nodes.is_empty() { top_errors.push("`nodes` must contain at least one node".into()); }
    if input.nodes.len() > MAX_MAP_NODES {
        top_errors.push(format!("`nodes` exceeds cap of {MAX_MAP_NODES} (got {})", input.nodes.len()));
    }
    if input.edges.len() > MAX_MAP_EDGES {
        top_errors.push(format!("`edges` exceeds cap of {MAX_MAP_EDGES} (got {})", input.edges.len()));
    }
    if input.overlaps.len() > MAX_MAP_OVERLAPS {
        top_errors.push(format!("`overlaps` exceeds cap of {MAX_MAP_OVERLAPS} (got {})", input.overlaps.len()));
    }
    if !top_errors.is_empty() { return Err(top_errors); }

    let mut smc = build_session_map_context(ctx.events, ctx.stmt_seq);
    let valid_sc: BTreeSet<&str> = ctx.valid_subclaim_ids.iter().map(String::as_str).collect();

    // Reject duplicate (kind, identity_key) pairs, duplicate `temp_id`s (Audit MEDIUM #7), and Rule V8 banned phrases.
    let mut batch_identities: BTreeSet<(MapNodeKind, String)> = BTreeSet::new();
    let mut batch_temp_ids: BTreeSet<String> = BTreeSet::new();
    let mut batch_errors = Vec::new();
    for (idx, n) in input.nodes.iter().enumerate() {
        let tid = n.temp_id.trim();
        if tid.is_empty() || !batch_temp_ids.insert(tid.to_string()) {
            batch_errors.push(format!("nodes[{idx}] has empty or duplicate temp_id `{tid}`"));
        }
        if let Some(id_key) = compute_node_identity(n) {
            if !batch_identities.insert((n.kind, id_key.clone())) {
                batch_errors.push(format!("nodes[{idx}] duplicates identity ({}, `{id_key}`) in one call (Rule S4)", n.kind.as_str()));
            }
        }
        if let Err(banned) = check_explore_prose(&n.statement) {
            batch_errors.push(format!("nodes[{idx}].statement contains banned absolute phrase(s) [{}]; use a scoped statement (Rule V8)", banned.join(", ")));
        }
    }
    for (idx, e) in input.edges.iter().enumerate() {
        if let Some(reason) = e.reason.as_deref() {
            if let Err(banned) = check_explore_prose(reason) {
                batch_errors.push(format!("edges[{idx}].reason contains banned absolute phrase(s) [{}]; use a scoped statement (Rule V8)", banned.join(", ")));
            }
        }
    }
    for (idx, o) in input.overlaps.iter().enumerate() {
        if let Err(banned) = check_explore_prose(&o.difference) {
            batch_errors.push(format!("overlaps[{idx}].difference contains banned absolute phrase(s) [{}]; use a scoped statement (Rule V8)", banned.join(", ")));
        }
    }
    if !batch_errors.is_empty() { return Err(batch_errors); }

    let mut dropped: Vec<DroppedMapItem> = Vec::new();
    let mut temp_to_final: BTreeMap<String, String> = BTreeMap::new();
    let mut active_ids: BTreeSet<String> = BTreeSet::new();
    let mut nodes: Vec<MapNode> = Vec::new();

    for (idx, raw_node) in input.nodes.into_iter().enumerate() {
        let ref_label = format!("nodes[{idx}] ({})", raw_node.temp_id);
        match validate_node(&raw_node, &ctx, &mut smc, &valid_sc) {
            Ok(node) => {
                temp_to_final.insert(raw_node.temp_id.trim().to_string(), node.id.clone());
                active_ids.insert(node.id.clone());
                nodes.push(node);
            }
            Err(reason) => dropped.push(DroppedMapItem { item_type: "node".into(), reference: ref_label, reason }),
        }
    }

    if nodes.is_empty() {
        let reasons: Vec<String> = dropped.iter().map(|d| format!("{}: {}", d.reference, d.reason)).collect();
        return Err(vec![format!("record_idea_map refused: zero valid nodes survived validation ({})", reasons.join("; "))]);
    }

    // Rule S4: preserve any previously user-noted node missing from the new map as `not_in_current_map`.
    for noted_id in &smc.noted_node_ids {
        if !active_ids.contains(noted_id) {
            if let Some(prev) = smc.prior_nodes_by_id.get(noted_id) {
                let mut kept = prev.clone();
                kept.status = MapNodeStatus::NotInCurrentMap;
                active_ids.insert(kept.id.clone());
                nodes.push(kept);
            }
        }
    }

    let mut edges: Vec<MapEdge> = Vec::new();
    let mut edge_dedupe: BTreeSet<(String, String, MapEdgeKind)> = BTreeSet::new();
    for (idx, e) in input.edges.into_iter().enumerate() {
        let ref_label = format!("edges[{idx}] ({} {} {})", e.from, e.kind.as_str(), e.to);
        match validate_edge(&e, &temp_to_final, &active_ids, &smc) {
            Ok(edge) => {
                let key = canonical_edge_key(&edge.from, &edge.to, edge.kind);
                if edge_dedupe.insert(key) { edges.push(edge); }
            }
            Err(reason) => dropped.push(DroppedMapItem { item_type: "edge".into(), reference: ref_label, reason }),
        }
    }

    synthesize_citation_edges(&nodes, &smc.citation_pairs, &mut edges, &mut edge_dedupe);

    let mut overlaps: Vec<MapOverlap> = Vec::new();
    for (idx, o) in input.overlaps.into_iter().enumerate() {
        let ref_label = format!("overlaps[{idx}] ({}/{})", o.subclaim_id, o.paper_key);
        match validate_overlap(o, &smc, &valid_sc) {
            Ok(ov) => overlaps.push(ov),
            Err(reason) => dropped.push(DroppedMapItem { item_type: "overlap".into(), reference: ref_label, reason }),
        }
    }

    Ok(IdeaMapPayload { idea_rev: ctx.idea_rev, request_seq: ctx.request_seq, nodes, edges, overlaps, dropped })
}

fn canonical_edge_key(from: &str, to: &str, kind: MapEdgeKind) -> (String, String, MapEdgeKind) {
    if matches!(kind, MapEdgeKind::Contradicts | MapEdgeKind::SameAs) && to < from {
        (to.to_string(), from.to_string(), kind)
    } else {
        (from.to_string(), to.to_string(), kind)
    }
}

fn is_valid_subclaim_id(s: &str) -> bool {
    s.strip_prefix("sc-")
        .is_some_and(|d| !d.is_empty() && !d.starts_with('0') && d.bytes().all(|b| b.is_ascii_digit()))
}

fn canonical_paper_key(raw: &str) -> Option<String> {
    PaperKey::parse(raw.trim()).ok().map(|k| k.to_string())
}

fn compute_node_identity(n: &MapNodeInput) -> Option<String> {
    if let Some(first_ev) = n.evidence.first().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        return Some(first_ev.to_string());
    }
    if n.kind == MapNodeKind::Concept {
        if let Some(iq) = n.idea_quote.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            return Some(format!("idea_quote:{}", sha256_hex(iq.as_bytes())));
        }
    }
    None
}

fn validate_node(
    raw: &MapNodeInput,
    ctx: &MapValidationInput<'_>,
    smc: &mut SessionMapContext,
    valid_sc: &BTreeSet<&str>,
) -> Result<MapNode, String> {
    let label = raw.label.trim();
    if label.is_empty() || label.chars().count() > MAX_NODE_LABEL_CHARS {
        return Err(format!("label must be 1..={MAX_NODE_LABEL_CHARS} chars"));
    }
    let statement = raw.statement.trim();
    if statement.is_empty() || statement.chars().count() > MAX_NODE_STATEMENT_CHARS {
        return Err(format!("statement must be 1..={MAX_NODE_STATEMENT_CHARS} chars"));
    }
    if raw.kind != MapNodeKind::Concept && raw.idea_quote.as_deref().is_some_and(|s| !s.trim().is_empty()) {
        return Err(format!("`idea_quote` is only permitted on `concept` nodes, not `{}`", raw.kind.as_str()));
    }

    let mut clean_sc = Vec::new();
    let mut seen_sc = BTreeSet::new();
    for sc in &raw.subclaim_ids {
        let t = sc.trim();
        if !is_valid_subclaim_id(t) { return Err(format!("invalid subclaim_id `{t}`")); }
        if !valid_sc.is_empty() && !valid_sc.contains(t) { return Err(format!("unknown subclaim_id `{t}`")); }
        if seen_sc.insert(t.to_string()) { clean_sc.push(t.to_string()); }
    }

    let mut clean_ev: Vec<String> = Vec::new();
    let mut seen_ev = BTreeSet::new();
    let mut entries: Vec<&SessionEvidenceEntry> = Vec::new();
    for raw_id in &raw.evidence {
        let id = raw_id.trim();
        if parse_quote_id(id).is_none() && parse_abstract_id(id).is_none() {
            return Err(format!("invalid evidence id `{id}` (expected `xe-<seq>` or `xa-<seq>`)"));
        }
        let Some(entry) = smc.evidence.get(id) else {
            return Err(format!("evidence id `{id}` does not exist in this session's ledger (Rule C5)"));
        };
        if seen_ev.insert(id.to_string()) {
            clean_ev.push(id.to_string());
            entries.push(entry);
        }
    }

    let mut papers_set = BTreeSet::new();
    for e in &entries {
        if let Some(ref pk) = e.paper_key { papers_set.insert(pk.clone()); }
    }

    let mut clean_idea_quote: Option<String> = None;
    let mut stated_year: Option<i32> = None;
    let mut later_work_checked: Option<bool> = None;
    let mut basis: Option<String> = None;
    let mut paper_key_out: Option<String> = None;

    match raw.kind {
        MapNodeKind::Concept => {
            if entries.is_empty() {
                let iq = raw.idea_quote.as_deref().map(str::trim).filter(|s| !s.is_empty())
                    .ok_or_else(|| "concept node requires >= 1 session evidence id or verbatim `idea_quote`".to_string())?;
                if !ctx.idea_text.contains(iq) {
                    return Err("concept `idea_quote` must be an exact verbatim substring of the active IdeaStatement".into());
                }
                clean_idea_quote = Some(iq.to_string());
            } else if let Some(iq) = raw.idea_quote.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                if !ctx.idea_text.contains(iq) {
                    return Err("concept `idea_quote` must be an exact verbatim substring of the active IdeaStatement".into());
                }
                clean_idea_quote = Some(iq.to_string());
            }
        }
        MapNodeKind::Definition => {
            if !entries.iter().any(|e| e.is_full_text && e.role == Some(EvidenceRole::Defines)) {
                return Err("definition node requires >= 1 full-text `xe-*` quote with role `defines` (Rule C5)".into());
            }
        }
        MapNodeKind::KnownResult => {
            if !entries.iter().any(|e| e.is_full_text && matches!(e.role, Some(EvidenceRole::StatesResult | EvidenceRole::NegativeResult))) {
                return Err("known_result node requires >= 1 full-text `xe-*` quote with role `states_result` or `negative_result` (Rule C5)".into());
            }
        }
        MapNodeKind::OpenQuestion => {
            let q_entry = entries
                .iter()
                .find(|e| e.is_full_text && e.role == Some(EvidenceRole::PosesOpenQuestion))
                .ok_or_else(|| "open_question node requires >= 1 full-text `xe-*` quote with role `poses_open_question` (Rule C6)".to_string())?;
            stated_year = q_entry.paper_key.as_deref().and_then(|pk| smc.paper_years.get(pk).copied());
            later_work_checked = Some(match (stated_year, smc.max_searched_year) {
                (Some(sy), Some(my)) => my > sy,
                _ => false,
            });
        }
        MapNodeKind::RelatedWork => {
            if entries.is_empty() {
                return Err("related_work node requires >= 1 session `xe-*` or `xa-*` evidence id (Rule C5)".into());
            }
            let has_ft = entries.iter().any(|e| e.is_full_text);
            basis = Some(if has_ft { "fulltext" } else { "abstract" }.to_string());
            // Audit HIGH #4: `paper_key` MUST agree with cited evidence papers.
            if let Some(raw_pk) = raw.paper_key.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                let parsed = canonical_paper_key(raw_pk)
                    .ok_or_else(|| format!("related_work `paper_key` `{raw_pk}` is not a canonical PaperKey"))?;
                if !papers_set.is_empty() && !papers_set.contains(&parsed) {
                    return Err(format!("related_work `paper_key` `{parsed}` does not match any cited evidence paper"));
                }
                paper_key_out = Some(parsed);
            } else {
                paper_key_out = entries.iter().find_map(|e| e.paper_key.clone());
            }
        }
    }

    let papers: Vec<String> = papers_set.into_iter().collect();
    let year = papers.iter().filter_map(|pk| smc.paper_years.get(pk).copied()).min();
    let identity_key = if let Some(first) = clean_ev.first() {
        first.clone()
    } else if let Some(ref iq) = clean_idea_quote {
        format!("idea_quote:{}", sha256_hex(iq.as_bytes()))
    } else {
        return Err("node has no verifiable identity key".into());
    };
    let id = assign_stable_node_id(raw.kind, &identity_key, smc);

    Ok(MapNode {
        id, kind: raw.kind, rev: ctx.idea_rev, label: label.to_string(), statement: statement.to_string(),
        evidence: clean_ev, relation_to_idea: raw.relation_to_idea, subclaim_ids: clean_sc,
        papers, year, status: MapNodeStatus::Active, identity_key, idea_quote: clean_idea_quote,
        aliases: raw.aliases.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
        symbol: raw.symbol.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string),
        conventions: raw.conventions.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string),
        hypotheses: raw.hypotheses.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
        regime: raw.regime.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string),
        stated_year, later_work_checked, paper_key: paper_key_out, basis,
    })
}

fn assign_stable_node_id(kind: MapNodeKind, identity_key: &str, smc: &mut SessionMapContext) -> String {
    if let Some(existing) = smc.prior_id_by_identity.get(&(kind, identity_key.to_string())) {
        return existing.clone();
    }
    let next = smc.max_seq_by_kind.entry(kind).or_insert(0);
    *next += 1;
    let id = format!("{}{}", kind.id_prefix(), *next);
    smc.prior_id_by_identity.insert((kind, identity_key.to_string()), id.clone());
    id
}

fn resolve_edge_target(
    raw_ref: &str,
    temp_to_final: &BTreeMap<String, String>,
    active_ids: &BTreeSet<String>,
) -> Option<String> {
    let trimmed = raw_ref.trim();
    if let Some(mapped) = temp_to_final.get(trimmed) {
        return Some(mapped.clone());
    }
    active_ids.contains(trimmed).then(|| trimmed.to_string())
}

fn validate_edge(
    e: &crate::idea_map::MapEdgeInput,
    temp_to_final: &BTreeMap<String, String>,
    active_ids: &BTreeSet<String>,
    smc: &SessionMapContext,
) -> Result<MapEdge, String> {
    // Audit HIGH #2: `metadata` edges are strictly app-built (e.g. `idea.citation_net`), never model-supplied.
    if e.basis == MapEdgeBasis::Metadata {
        return Err("`metadata` edges are app-built from citation data; model edges must use `evidence` or `model_inferred`".into());
    }
    let from = resolve_edge_target(&e.from, temp_to_final, active_ids)
        .ok_or_else(|| format!("edge `from` target `{}` does not resolve to a surviving node", e.from))?;
    let to = resolve_edge_target(&e.to, temp_to_final, active_ids)
        .ok_or_else(|| format!("edge `to` target `{}` does not resolve to a surviving node", e.to))?;
    if from == to { return Err("self-loop edge is not allowed".into()); }

    let reason = e.reason.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    if let Some(ref r) = reason {
        if r.chars().count() > MAX_EDGE_REASON_CHARS {
            return Err(format!("edge reason exceeds {MAX_EDGE_REASON_CHARS} chars"));
        }
    }
    if e.basis == MapEdgeBasis::ModelInferred && reason.is_none() {
        return Err("`model_inferred` edge requires a non-empty `reason`".into());
    }

    let evidence = match e.evidence.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(ev_id) => {
            if parse_quote_id(ev_id).is_none() {
                return Err(format!("edge evidence `{ev_id}` must be a full-text `xe-<seq>` id"));
            }
            if !smc.evidence.get(ev_id).is_some_and(|x| x.is_full_text) {
                return Err(format!("edge evidence `{ev_id}` does not exist in this session's ledger"));
            }
            Some(ev_id.to_string())
        }
        None => None,
    };
    if e.basis == MapEdgeBasis::Evidence && evidence.is_none() {
        return Err("`evidence` edge requires a valid session `xe-<seq>` evidence id".into());
    }
    Ok(MapEdge { from, to, kind: e.kind, basis: e.basis, evidence, reason })
}

fn validate_overlap(
    o: crate::idea_map::OverlapInput,
    smc: &SessionMapContext,
    valid_sc: &BTreeSet<&str>,
) -> Result<MapOverlap, String> {
    let sc = o.subclaim_id.trim();
    if !is_valid_subclaim_id(sc) || (!valid_sc.is_empty() && !valid_sc.contains(sc)) {
        return Err(format!("invalid or unknown subclaim_id `{sc}`"));
    }
    let paper_key = PaperKey::parse(o.paper_key.trim())
        .map_err(|e| format!("invalid overlap paper_key `{}`: {e}", o.paper_key))?;
    let pk_str = paper_key.to_string();
    if o.difference.chars().count() > MAX_OVERLAP_DIFF_CHARS {
        return Err(format!("overlap difference exceeds {MAX_OVERLAP_DIFF_CHARS} chars"));
    }
    if o.relation == OverlapRelation::OtherName
        && !o.other_name.as_ref().is_some_and(|n| !n.term.trim().is_empty())
    {
        return Err("overlap with relation `other_name` requires non-empty `other_name.term`".into());
    }
    // Audit HIGH #3: require >= 1 session `xe-*` quote belonging to `paper_key`.
    if o.evidence.is_empty() {
        return Err("overlap requires >= 1 full-text `xe-<seq>` evidence id".into());
    }
    let mut clean_ev = Vec::new();
    let mut seen = BTreeSet::new();
    for raw_id in o.evidence {
        let id = raw_id.trim();
        if parse_quote_id(id).is_none() {
            return Err(format!("overlap evidence `{id}` must be a full-text `xe-<seq>` id"));
        }
        let Some(entry) = smc.evidence.get(id).filter(|x| x.is_full_text) else {
            return Err(format!("overlap evidence `{id}` does not exist in this session's ledger"));
        };
        if let Some(ref ev_pk) = entry.paper_key {
            if ev_pk != &pk_str {
                return Err(format!("overlap evidence `{id}` belongs to `{ev_pk}`, not `{pk_str}`"));
            }
        }
        if seen.insert(id.to_string()) { clean_ev.push(id.to_string()); }
    }
    Ok(MapOverlap {
        subclaim_id: sc.to_string(), paper_key, relation: o.relation,
        evidence: clean_ev, difference: o.difference.trim().to_string(), other_name: o.other_name,
    })
}

fn synthesize_citation_edges(
    nodes: &[MapNode],
    citation_pairs: &BTreeSet<(String, String)>,
    edges: &mut Vec<MapEdge>,
    edge_dedupe: &mut BTreeSet<(String, String, MapEdgeKind)>,
) {
    if citation_pairs.is_empty() { return; }
    let mut nodes_by_paper: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for n in nodes {
        if let Some(ref pk) = n.paper_key {
            nodes_by_paper.entry(pk.as_str()).or_default().push(n.id.as_str());
        } else if n.kind == MapNodeKind::RelatedWork {
            for pk in &n.papers {
                nodes_by_paper.entry(pk.as_str()).or_default().push(n.id.as_str());
            }
        }
    }
    for (citing, cited) in citation_pairs {
        let (Some(from_nodes), Some(to_nodes)) = (nodes_by_paper.get(citing.as_str()), nodes_by_paper.get(cited.as_str())) else { continue };
        for &fn_id in from_nodes {
            for &tn_id in to_nodes {
                if fn_id != tn_id && edge_dedupe.insert((fn_id.to_string(), tn_id.to_string(), MapEdgeKind::Cites)) {
                    edges.push(MapEdge {
                        from: fn_id.to_string(), to: tn_id.to_string(), kind: MapEdgeKind::Cites,
                        basis: MapEdgeBasis::Metadata, evidence: None, reason: None,
                    });
                }
            }
        }
    }
}

fn build_session_map_context(events: &[Event], stmt_seq: u64) -> SessionMapContext {
    let mut evidence: BTreeMap<String, SessionEvidenceEntry> = BTreeMap::new();
    let mut paper_years: BTreeMap<String, i32> = BTreeMap::new();
    let mut max_searched_year: Option<i32> = None;
    let mut prior_id_by_identity: BTreeMap<(MapNodeKind, String), String> = BTreeMap::new();
    let mut prior_nodes_by_id: BTreeMap<String, MapNode> = BTreeMap::new();
    let mut noted_node_ids: BTreeSet<String> = BTreeSet::new();
    let mut max_seq_by_kind: BTreeMap<MapNodeKind, u64> = BTreeMap::new();
    let mut citation_pairs: BTreeSet<(String, String)> = BTreeSet::new();

    for ev in events {
        match ev.kind.as_str() {
            EVIDENCE_QUOTE_EVENT if ev.seq > stmt_seq => {
                let id = format!("xe-{}", ev.seq);
                let paper_key = ev.payload["paper_key"].as_str().and_then(canonical_paper_key);
                let role = ev.payload.get("role").and_then(|v| serde_json::from_value::<EvidenceRole>(v.clone()).ok());
                evidence.insert(id, SessionEvidenceEntry { is_full_text: true, paper_key, role });
            }
            EVIDENCE_ABSTRACT_EVENT if ev.seq > stmt_seq => {
                let id = format!("xa-{}", ev.seq);
                let paper_key = ev.payload["paper_key"].as_str().and_then(canonical_paper_key);
                evidence.insert(id, SessionEvidenceEntry { is_full_text: false, paper_key, role: None });
            }
            "idea.candidates" if ev.seq > stmt_seq => {
                if let Some(arr) = ev.payload["candidates"].as_array() {
                    for c in arr {
                        let yr = c["year"].as_i64().map(|y| y as i32);
                        if let Some(y) = yr {
                            max_searched_year = Some(max_searched_year.map_or(y, |m| m.max(y)));
                            if let Some(k) = c["key"].as_str().and_then(canonical_paper_key) {
                                paper_years.insert(k, y);
                            }
                        }
                    }
                }
            }
            EVENT_IDEA_CITATION_NET if ev.seq > stmt_seq => {
                if let Some(arr) = ev.payload["edges"].as_array() {
                    for e in arr {
                        let from = e["from"].as_str().or_else(|| e["citing"].as_str()).and_then(canonical_paper_key);
                        let to = e["to"].as_str().or_else(|| e["cited"].as_str()).and_then(canonical_paper_key);
                        if let (Some(kf), Some(kt)) = (from, to) { citation_pairs.insert((kf, kt)); }
                    }
                }
            }
            EVENT_IDEA_MAP => {
                if let Ok(prev_map) = serde_json::from_value::<IdeaMapPayload>(ev.payload.clone()) {
                    for n in prev_map.nodes {
                        let id_key = if !n.identity_key.is_empty() {
                            n.identity_key.clone()
                        } else if let Some(first) = n.evidence.first() {
                            first.clone()
                        } else if let Some(ref iq) = n.idea_quote {
                            format!("idea_quote:{}", sha256_hex(iq.as_bytes()))
                        } else {
                            continue;
                        };
                        if let Some(num) = n.id.strip_prefix(n.kind.id_prefix()).and_then(|d| d.parse::<u64>().ok()) {
                            let entry = max_seq_by_kind.entry(n.kind).or_insert(0);
                            *entry = (*entry).max(num);
                        }
                        prior_id_by_identity.insert((n.kind, id_key), n.id.clone());
                        prior_nodes_by_id.insert(n.id.clone(), n);
                    }
                }
            }
            "idea.note" => {
                let is_node_target = ev.payload["target_kind"].as_str() == Some("node")
                    || ev.payload["target"].as_str().is_some_and(looks_like_node_id);
                if is_node_target {
                    if let Some(tid) = ev.payload["target_id"].as_str().or_else(|| ev.payload["target"].as_str()) {
                        noted_node_ids.insert(tid.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    SessionMapContext {
        evidence, paper_years, max_searched_year, prior_id_by_identity,
        prior_nodes_by_id, noted_node_ids, max_seq_by_kind, citation_pairs,
    }
}

fn looks_like_node_id(s: &str) -> bool {
    ["c-", "d-", "k-", "q-", "w-"]
        .iter()
        .any(|p| s.strip_prefix(p).is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())))
}
