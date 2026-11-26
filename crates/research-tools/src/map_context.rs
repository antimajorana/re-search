//! Investigate v2 context-card recorder (`record_context_cards`, package V12, rules 6.3, 6.4, 6.8, 6.9, 6.10).

use crate::investigation::anchor::valid_id;
use crate::investigation_markers::{
    active_investigation_slice, check_new_recorder_gate, NewRecorderKind,
    EVENT_INVESTIGATION_CONTEXT, EVENT_INVESTIGATION_SKELETON,
};
use crate::lines_sent::{lines_sent_with_ledger, SentLines};
use crate::map_context_defs::{
    count_model_background_cards, record_context_cards_def, ContextCardOrigin, ContextCardRecord,
    ContextCardType, MapContextEventPayload, CONTEXT_SCHEMA_VERSION,
    EVENT_INVESTIGATION_CITED_LOOKUP, MAX_CONTEXT_CARDS, MAX_CONTEXT_CARD_ANCHORS,
    MAX_CONTEXT_CARD_HYPOTHESES, MAX_CONTEXT_CARD_TITLE_CHARS, MAX_CONTEXT_HYPOTHESIS_CHARS,
    MAX_GROUNDED_BODY_CHARS, MAX_MODEL_BACKGROUND_BODY_CHARS, MAX_MODEL_BACKGROUND_PER_NODE,
};
use crate::map_defs::{HypothesisRecord, IndexedFault, VerifiedMapAnchor, RECORD_CONTEXT_CARDS};
use crate::map_model_nodes::{collect_existing_model_nodes, ModelNodeRef};
use crate::map_reading::validate::{
    fault, span_contains_anchor, verify_anchor_with_fallback, AnchorErr, InputAnchor,
    InputHypothesis, ReadingAnchorReport,
};
use crate::map_reading::{
    compute_open_scopes, load_pinned_skeleton, reject_app_only_fields_with, rejected_outcome,
};
use crate::paper_key::PaperKey;
use crate::skeleton::{
    build_html_skeleton_from_ingested, build_pdf_skeleton_from_ingested,
    build_tex_skeleton_from_ingested, derive_receipt_paper_identity, is_html_receipt,
    is_pdf_receipt, is_tex_receipt, PaperSkeleton, SkNode, Span, CITED_STATUS_FOUND,
};
use crate::{resolve_sources_session_id, ResearchEnv, WorkspaceSandbox};
use ledger::{Event, Ledger};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use tools::{CallCtx, Registry};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawContextArgs {
    request_seq: u64,
    source_id: String,
    tag: String,
    cards: Vec<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputContextCard {
    #[serde(rename = "type")]
    card_type: ContextCardType,
    title: String,
    body: String,
    anchors: Vec<InputAnchor>,
    #[serde(default)]
    hypotheses: Vec<InputHypothesis>,
    #[serde(default)]
    bib_key: Option<String>,
}

struct ContextValidationEnv<'a> {
    ledger: &'a Ledger,
    sandbox: WorkspaceSandbox,
    session: &'a str,
    caller: &'a str,
    primary_source_id: &'a str,
    events: &'a [Event],
    active_slice: &'a [Event],
    sent: &'a SentLines,
    receipt_primary_paths: BTreeMap<String, String>,
    bib_keys: BTreeSet<&'a str>,
    target_tag: &'a str,
    target_node_fp: Option<String>,
    model_bg_count: usize,
    ext_skeletons: BTreeMap<String, PaperSkeleton>,
}

struct CitedExternalTarget {
    source_id: String,
    tag: String,
    spans: Vec<Span>,
}

/// Register the `record_context_cards` tool on an `investigation/2` profile registry.
pub fn register_context_cards_tool(registry: &mut Registry, env: Arc<ResearchEnv>) {
    registry.register(
        record_context_cards_def(),
        Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
            let ledger = ctx.ledger.lock().map_err(|_| "ledger lock poisoned")?;
            record_context_cards(&env, &ledger, ctx.session, ctx.caller, args)
        }),
    );
}

/// Validate and append grounded or bounded `model_background` context cards (`investigation.context`).
pub fn record_context_cards(
    env: &ResearchEnv,
    ledger: &Ledger,
    session: &str,
    caller: &str,
    args: &Value,
) -> Result<Value, String> {
    let session = resolve_sources_session_id(session).map_err(|e| e.to_string())?;
    reject_app_only_fields_with(args, &["origin"])?;
    tools::registry::check_args(&record_context_cards_def().input_schema, args)?;

    let parsed: RawContextArgs = serde_json::from_value(args.clone())
        .map_err(|e| format!("Invalid `record_context_cards` input: {e}"))?;
    if !valid_id(&parsed.source_id, 64) {
        return Err("Invalid `source_id`.".into());
    }
    if parsed.cards.is_empty() || parsed.cards.len() > MAX_CONTEXT_CARDS {
        return Err(format!("`cards` must contain 1..={MAX_CONTEXT_CARDS} entries."));
    }

    let (_verified, events) = ledger
        .read_verified_session(session)
        .map_err(|_| "The session ledger or a recorded artifact could not be verified.")?;
    let (active_slice, _) = active_investigation_slice(&events)?;
    let (marker, task_kind) =
        check_new_recorder_gate(active_slice, NewRecorderKind::ContextCards, parsed.request_seq)?;
    if marker.payload["source_id"].as_str() != Some(parsed.source_id.as_str()) {
        return Err(format!(
            "`source_id` `{}` does not match the active task's source `{}`.",
            parsed.source_id,
            marker.payload["source_id"].as_str().unwrap_or("")
        ));
    }

    let (skeleton_seq, skeleton) =
        load_pinned_skeleton(ledger, active_slice, marker, &parsed.source_id)?;
    let nodes_by_tag: BTreeMap<&str, &SkNode> =
        skeleton.nodes.iter().filter_map(|n| Some((n.tag.as_deref()?, n))).collect();
    let model_nodes = collect_existing_model_nodes(&skeleton, &events);
    let target_tag = parsed.tag.trim();
    if target_tag.is_empty() {
        return Err("`tag` must be non-empty.".into());
    }
    let target_node_fp = validate_target_tag_and_scope(
        task_kind,
        marker,
        &skeleton,
        &nodes_by_tag,
        &model_nodes,
        target_tag,
    )?;

    let mut receipt_primary_paths = BTreeMap::new();
    for ev in &events {
        if ev.kind == "source.ingested" {
            if let (Some(sid), Some(path)) = (
                ev.payload["source_id"].as_str().filter(|s| !s.is_empty()),
                ev.payload["primary_path"].as_str().filter(|s| !s.is_empty()),
            ) {
                receipt_primary_paths.insert(sid.to_string(), path.to_string());
            }
        }
    }

    let marker_global_idx = events
        .iter()
        .position(|e| e.seq == marker.seq)
        .ok_or("Active task marker is missing from session events.")?;
    let task_events = &events[marker_global_idx..];
    let req_seq_str = parsed.request_seq.to_string();
    let sent = lines_sent_with_ledger(task_events, Some(ledger), Some(&req_seq_str));
    let initial_bg = count_model_background_cards(&events, &parsed.source_id, target_tag);

    let mut vctx = ContextValidationEnv {
        ledger,
        sandbox: env.sandbox(),
        session,
        caller,
        primary_source_id: &parsed.source_id,
        events: &events,
        active_slice,
        sent: &sent,
        receipt_primary_paths,
        bib_keys: skeleton.bib.iter().map(|b| b.key.as_str()).collect(),
        target_tag,
        target_node_fp,
        model_bg_count: initial_bg,
        ext_skeletons: BTreeMap::new(),
    };

    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    let mut outcomes = Vec::with_capacity(parsed.cards.len());
    let mut report = ReadingAnchorReport::default();

    for (idx, raw) in parsed.cards.into_iter().enumerate() {
        match validate_one_card(&mut vctx, idx, raw, &mut report) {
            Ok(card) => {
                outcomes.push(json!({
                    "index": idx,
                    "status": "accepted",
                    "type": card.card_type.as_str(),
                    "origin": card.origin.as_str(),
                    "tag": target_tag,
                }));
                accepted.push(card);
            }
            Err(f) => {
                outcomes.push(rejected_outcome(&f));
                rejected.push(f);
            }
        }
    }

    let anchor_report_val = report.to_value();
    let context_seq = if !accepted.is_empty() {
        let payload = MapContextEventPayload {
            schema_version: CONTEXT_SCHEMA_VERSION,
            request_seq: parsed.request_seq,
            source_id: parsed.source_id.clone(),
            skeleton_seq,
            tag: target_tag.to_string(),
            cards: accepted.clone(),
            rejected: rejected.clone(),
            anchor_report: anchor_report_val.clone(),
        };
        let payload_val = serde_json::to_value(payload)
            .map_err(|e| format!("Failed to serialize `investigation.context`: {e}"))?;
        let seq = ledger
            .append(session, EVENT_INVESTIGATION_CONTEXT, CONTEXT_SCHEMA_VERSION, &payload_val)
            .map_err(|e| format!("Failed to append `investigation.context`: {e}"))?;
        Some(seq)
    } else {
        None
    };

    let status = if context_seq.is_some() { "recorded" } else { "no_valid_records" };
    Ok(json!({
        "tool": RECORD_CONTEXT_CARDS,
        "status": status,
        "request_seq": parsed.request_seq,
        "source_id": parsed.source_id,
        "skeleton_seq": skeleton_seq,
        "tag": target_tag,
        "context_seq": context_seq,
        "accepted": accepted,
        "rejected": rejected,
        "outcomes": outcomes,
        "anchor_report": anchor_report_val,
    }))
}

fn validate_target_tag_and_scope(
    task: crate::investigation_markers::InvestigationTaskKind,
    marker: &Event,
    skeleton: &PaperSkeleton,
    nodes_by_tag: &BTreeMap<&str, &SkNode>,
    model_nodes: &BTreeMap<String, ModelNodeRef>,
    target_tag: &str,
) -> Result<Option<String>, String> {
    let node_fp = if let Some(sk) = nodes_by_tag.get(target_tag) {
        Some(sk.fp.clone())
    } else if let Some(mn) = model_nodes.get(target_tag) {
        Some(mn.node_fp.clone())
    } else {
        return Err(format!(
            "Target tag `{target_tag}` does not exist in pinned skeleton `{}` or existing model nodes.",
            skeleton.source_id
        ));
    };

    let in_scope = match task {
        crate::investigation_markers::InvestigationTaskKind::Open => {
            let (item_scope, _) = compute_open_scopes(marker, skeleton, nodes_by_tag, model_nodes);
            item_scope.contains(target_tag)
        }
        crate::investigation_markers::InvestigationTaskKind::Context => marker
            .payload
            .get("tags")
            .and_then(Value::as_array)
            .is_some_and(|arr| arr.iter().filter_map(Value::as_str).any(|t| t == target_tag)),
        other => {
            return Err(format!(
                "`{RECORD_CONTEXT_CARDS}` is not allowed during task kind `{}`.",
                other.as_str()
            ));
        }
    };
    if !in_scope {
        return Err(format!(
            "Target tag `{target_tag}` is outside the scope of the active `{}` task.",
            task.as_str()
        ));
    }
    Ok(node_fp)
}

fn validate_one_card(
    ctx: &mut ContextValidationEnv<'_>,
    index: usize,
    raw: Value,
    report: &mut ReadingAnchorReport,
) -> Result<ContextCardRecord, IndexedFault> {
    let parsed: InputContextCard = serde_json::from_value(raw)
        .map_err(|e| fault(index, "invalid_card", format!("Invalid context card: {e}"), None))?;

    let title = parsed.title.trim();
    if title.is_empty() || title.chars().count() > MAX_CONTEXT_CARD_TITLE_CHARS {
        return Err(fault(
            index,
            "invalid_card",
            format!("Card `title` must be 1..={MAX_CONTEXT_CARD_TITLE_CHARS} characters."),
            None,
        ));
    }
    let body = parsed.body.trim();
    if body.is_empty() || body.chars().count() > MAX_GROUNDED_BODY_CHARS {
        return Err(fault(
            index,
            "invalid_card",
            format!("Card `body` must be 1..={MAX_GROUNDED_BODY_CHARS} characters."),
            None,
        ));
    }
    if parsed.anchors.len() > MAX_CONTEXT_CARD_ANCHORS
        || parsed.hypotheses.len() > MAX_CONTEXT_CARD_HYPOTHESES
    {
        return Err(fault(index, "invalid_card", "Card anchors or hypotheses exceed schema cap.", None));
    }

    let raw_bib = parsed.bib_key.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if parsed.card_type != ContextCardType::CitedResult && parsed.bib_key.is_some() {
        return Err(fault(
            index,
            "unexpected_bib_key",
            "Only `cited_result` context cards may specify `bib_key`.",
            None,
        ));
    }

    let cited_target = if parsed.card_type == ContextCardType::CitedResult {
        let key = raw_bib.ok_or_else(|| {
            fault(index, "missing_bib_key", "`cited_result` card requires `bib_key`.", None)
        })?;
        Some(resolve_cited_external_target(ctx, index, key)?)
    } else {
        None
    };

    // Unsourced card path (`anchors` empty and `hypotheses` empty).
    if parsed.anchors.is_empty() && parsed.hypotheses.is_empty() {
        if parsed.card_type == ContextCardType::CitedResult {
            return Err(fault(
                index,
                "missing_anchors",
                "`cited_result` cards cannot be `model_background` and require an anchor inside the cited node.",
                None,
            ));
        }
        if parsed.card_type != ContextCardType::Prerequisite {
            return Err(fault(
                index,
                "missing_anchors",
                format!(
                    "`{}` card requires at least one verified anchor; only `prerequisite` may use `model_background`.",
                    parsed.card_type.as_str()
                ),
                None,
            ));
        }
        if body.chars().count() > MAX_MODEL_BACKGROUND_BODY_CHARS {
            return Err(fault(
                index,
                "model_background_too_long",
                format!(
                    "Unsourced `model_background` prerequisite exceeds {MAX_MODEL_BACKGROUND_BODY_CHARS} characters ({}).",
                    body.chars().count()
                ),
                None,
            ));
        }
        if ctx.model_bg_count >= MAX_MODEL_BACKGROUND_PER_NODE {
            return Err(fault(
                index,
                "model_background_limit_exceeded",
                format!(
                    "Node `{}` already has {MAX_MODEL_BACKGROUND_PER_NODE} `model_background` prerequisite cards.",
                    ctx.target_tag
                ),
                None,
            ));
        }
        ctx.model_bg_count += 1;
        return Ok(ContextCardRecord {
            card_type: ContextCardType::Prerequisite,
            title: title.to_string(),
            body: body.to_string(),
            origin: ContextCardOrigin::ModelBackground,
            anchors: Vec::new(),
            hypotheses: Vec::new(),
            bib_key: None,
            external_source_id: None,
            external_tag: None,
            node_fp: ctx.target_node_fp.clone(),
        });
    }

    if parsed.anchors.is_empty() {
        return Err(fault(
            index,
            "missing_anchors",
            "Grounded context cards with hypotheses require at least one top-level anchor in `anchors`.",
            None,
        ));
    }

    // Grounded card: verify every anchor and hypothesis; drop the card on ANY anchor failure (Rule 6.8).
    let mut anchors = Vec::with_capacity(parsed.anchors.len());
    let mut pending_corr = Vec::new();
    for (ai, raw_a) in parsed.anchors.iter().enumerate() {
        let owner = format!("cards[{index}].anchors[{ai}]");
        let fallback = ctx.receipt_primary_paths.get(&raw_a.source_id).map(String::as_str);
        let va = verify_card_anchor(ctx, index, owner, raw_a, fallback, &mut pending_corr, report)?;
        anchors.push(va);
    }

    let mut hypotheses = Vec::with_capacity(parsed.hypotheses.len());
    for (hi, hyp) in parsed.hypotheses.iter().enumerate() {
        let htext = hyp.text.trim();
        if htext.is_empty() || htext.chars().count() > MAX_CONTEXT_HYPOTHESIS_CHARS {
            return Err(fault(
                index,
                "invalid_hypothesis",
                format!("Hypothesis text must be 1..={MAX_CONTEXT_HYPOTHESIS_CHARS} characters."),
                None,
            ));
        }
        let owner = format!("cards[{index}].hypotheses[{hi}]");
        let fallback = ctx.receipt_primary_paths.get(&hyp.anchor.source_id).map(String::as_str);
        let va =
            verify_card_anchor(ctx, index, owner, &hyp.anchor, fallback, &mut pending_corr, report)?;
        hypotheses.push(HypothesisRecord { text: htext.to_string(), anchor: va });
    }

    // Rule 6.9 enforcement for `cited_result`.
    if let Some(target) = &cited_target {
        let has_ext_anchor = anchors
            .iter()
            .any(|a| a.source_id == target.source_id && span_contains_anchor(&target.spans, a));
        if !has_ext_anchor {
            return Err(fault(
                index,
                "anchor_outside_cited_node",
                format!(
                    "`cited_result` card needs at least one verified anchor inside `{}` node `{}`.",
                    target.source_id, target.tag
                ),
                None,
            ));
        }
        for hyp in &hypotheses {
            if hyp.anchor.source_id != target.source_id
                || !span_contains_anchor(&target.spans, &hyp.anchor)
            {
                return Err(fault(
                    index,
                    "hypothesis_outside_cited_node",
                    format!(
                        "Every hypothesis of `cited_result` `{}` must anchor inside `{}` node `{}`.",
                        raw_bib.unwrap_or(""),
                        target.source_id,
                        target.tag
                    ),
                    None,
                ));
            }
        }
    }

    report.corrected.extend(pending_corr);

    // Rule 6.8 provenance assignment by code from where anchors actually matched.
    let uses_external = anchors
        .iter()
        .chain(hypotheses.iter().map(|h| &h.anchor))
        .any(|a| a.source_id != ctx.primary_source_id);
    let origin = if uses_external {
        ContextCardOrigin::External
    } else {
        ContextCardOrigin::Paper
    };

    Ok(ContextCardRecord {
        card_type: parsed.card_type,
        title: title.to_string(),
        body: body.to_string(),
        origin,
        anchors,
        hypotheses,
        bib_key: raw_bib.map(str::to_string),
        external_source_id: cited_target.as_ref().map(|t| t.source_id.clone()),
        external_tag: cited_target.as_ref().map(|t| t.tag.clone()),
        node_fp: ctx.target_node_fp.clone(),
    })
}

fn verify_card_anchor(
    ctx: &ContextValidationEnv<'_>,
    index: usize,
    owner: String,
    raw: &InputAnchor,
    fallback_path: Option<&str>,
    pending_corr: &mut Vec<Value>,
    report: &mut ReadingAnchorReport,
) -> Result<VerifiedMapAnchor, IndexedFault> {
    match verify_anchor_with_fallback(
        &ctx.sandbox,
        ctx.session,
        ctx.events,
        ctx.sent,
        ctx.caller,
        owner,
        raw,
        fallback_path,
    ) {
        Ok((va, corr)) => {
            if let Some(c) = corr {
                pending_corr.push(c);
            }
            Ok(va)
        }
        Err(AnchorErr { owner, code, message, true_text }) => {
            let mut dropped = json!({
                "owner": owner,
                "source_id": raw.source_id,
                "code": code,
                "message": message,
            });
            if let Some(t) = &true_text {
                dropped["true_text"] = json!(t);
            }
            report.dropped.push(dropped);
            Err(fault(index, code, message, true_text))
        }
    }
}

fn resolve_cited_external_target(
    ctx: &mut ContextValidationEnv<'_>,
    index: usize,
    bib_key: &str,
) -> Result<CitedExternalTarget, IndexedFault> {
    if !ctx.bib_keys.contains(bib_key) {
        return Err(fault(
            index,
            "unknown_bib_key",
            format!("Citation key `{bib_key}` is not in the pinned skeleton bibliography."),
            None,
        ));
    }

    let lookup_ev = ctx
        .active_slice
        .iter()
        .rev()
        .find(|e| {
            e.kind == EVENT_INVESTIGATION_CITED_LOOKUP
                && e.payload["tag"].as_str() == Some(ctx.target_tag)
                && e.payload["bib_key"].as_str() == Some(bib_key)
        })
        .or_else(|| {
            ctx.active_slice.iter().rev().find(|e| {
                e.kind == EVENT_INVESTIGATION_CITED_LOOKUP
                    && e.payload["bib_key"].as_str() == Some(bib_key)
            })
        })
        .ok_or_else(|| {
            fault(
                index,
                "missing_cited_lookup",
                format!("No `investigation.cited_lookup` record exists for `{bib_key}`."),
                None,
            )
        })?;

    let status = lookup_ev.payload["status"].as_str().unwrap_or("unresolved");
    let ext_source_id = lookup_ev.payload["external_source_id"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let ext_tag = lookup_ev.payload["external_tag"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let (Some(ext_source_id), Some(ext_tag)) = (ext_source_id, ext_tag) else {
        return Err(fault(
            index,
            "cited_result_not_located",
            format!("Citation `{bib_key}` has not resolved to one external node (`status`: `{status}`)."),
            None,
        ));
    };
    if status != CITED_STATUS_FOUND {
        return Err(fault(
            index,
            "cited_result_not_located",
            format!("Citation `{bib_key}` status is `{status}`, not `{CITED_STATUS_FOUND}`."),
            None,
        ));
    }
    if ext_source_id == ctx.primary_source_id {
        return Err(fault(
            index,
            "cited_source_is_primary",
            "Cited external source cannot equal the primary paper `source_id`.",
            None,
        ));
    }

    let spans = load_or_build_external_node_spans(ctx, index, ext_source_id, ext_tag, lookup_ev)?;
    Ok(CitedExternalTarget {
        source_id: ext_source_id.to_string(),
        tag: ext_tag.to_string(),
        spans,
    })
}

fn load_or_build_external_node_spans(
    ctx: &mut ContextValidationEnv<'_>,
    index: usize,
    ext_source_id: &str,
    ext_tag: &str,
    lookup_ev: &Event,
) -> Result<Vec<Span>, IndexedFault> {
    if !ctx.ext_skeletons.contains_key(ext_source_id) {
        let skel = load_external_skeleton(ctx, ext_source_id, lookup_ev).map_err(|msg| {
            fault(index, "external_source_unavailable", msg, None)
        })?;
        ctx.ext_skeletons.insert(ext_source_id.to_string(), skel);
    }
    let skel = ctx.ext_skeletons.get(ext_source_id).expect("inserted above");
    let node = skel
        .nodes
        .iter()
        .find(|n| n.tag.as_deref() == Some(ext_tag))
        .ok_or_else(|| {
            fault(
                index,
                "external_node_not_found",
                format!("Located external tag `{ext_tag}` not found in source `{ext_source_id}`."),
                None,
            )
        })?;
    Ok(node.spans.clone())
}

fn load_external_skeleton(
    ctx: &ContextValidationEnv<'_>,
    ext_source_id: &str,
    lookup_ev: &Event,
) -> Result<PaperSkeleton, String> {
    if let Some(skel_ev) = ctx.active_slice.iter().rev().find(|e| {
        e.kind == EVENT_INVESTIGATION_SKELETON
            && e.payload["source_id"].as_str() == Some(ext_source_id)
    }) {
        if let Some(blob) = skel_ev.payload["blob"].as_str().filter(|s| !s.is_empty()) {
            let bytes = ctx
                .ledger
                .get_blob(blob)
                .map_err(|_| format!("External skeleton blob `{blob}` could not be read."))?;
            if ledger::sha256_hex(&bytes) != blob {
                return Err(format!("External skeleton blob `{blob}` failed digest verification."));
            }
            let skel: PaperSkeleton = serde_json::from_slice(&bytes)
                .map_err(|e| format!("Invalid external skeleton JSON: {e}"))?;
            if skel.source_id == ext_source_id {
                return Ok(skel);
            }
        }
    }

    let receipt = ctx
        .events
        .iter()
        .rev()
        .find(|e| {
            e.kind == "source.ingested"
                && e.payload["source_id"].as_str() == Some(ext_source_id)
                && matches!(
                    e.payload["status"].as_str().unwrap_or(""),
                    "ok" | "pdf_only" | "empty_text_layer"
                )
        })
        .ok_or_else(|| format!("No usable `source.ingested` receipt for `{ext_source_id}`."))?;

    let key_override = lookup_ev.payload["resolved"]["paper_key"]
        .as_str()
        .and_then(|s| PaperKey::parse(s).ok());
    let ver_override = lookup_ev.payload["resolved"]["version"]
        .as_str()
        .map(str::to_string);
    let (paper_key, version) = derive_receipt_paper_identity(receipt, key_override, ver_override)?;

    if is_tex_receipt(receipt) {
        Ok(build_tex_skeleton_from_ingested(&ctx.sandbox, ctx.session, receipt, paper_key, version)?.0.skeleton)
    } else if is_html_receipt(receipt) {
        Ok(build_html_skeleton_from_ingested(&ctx.sandbox, ctx.session, receipt, paper_key, version)?.0.skeleton)
    } else if is_pdf_receipt(receipt) {
        Ok(build_pdf_skeleton_from_ingested(&ctx.sandbox, ctx.session, receipt, paper_key, version)?.0.skeleton)
    } else {
        Err(format!("Ingested source `{ext_source_id}` does not have a supported skeleton format."))
    }
}

#[cfg(test)]
#[path = "map_context_tests.rs"]
mod tests;
