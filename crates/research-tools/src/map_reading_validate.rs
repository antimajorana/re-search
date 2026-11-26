//! Per-item and per-edge validation helpers for `record_map_reading` (package V6).

use crate::investigation::anchor::bounded;
use crate::lines_sent::SentLines;
use crate::map_authors_claim::{validate_authors_claim, ClaimAnchorFault, InputAuthorsClaim};
use crate::map_defs::{
    HypothesisRecord, IndexedFault, ModelEdgeBasis, ModelEdgeRecord, ModelEdgeRole,
    ReadingItemRecord, ReadingRole, VerifiedMapAnchor, MAX_GIST_CHARS, MAX_PURPOSE_CHARS,
    MAX_READING_ANCHORS, MAX_READING_HYPOTHESES,
};
use crate::map_model_nodes::{
    model_node_tag, normalize_model_kind, role_allowed_for_model_kind, ModelNodeRef,
    MAX_MODEL_NODE_TEMP_ID_CHARS,
};
use crate::quote_core::{accept_quote_with_sent, QuoteRejection, VerifiedQuote};
use crate::skeleton::{PaperSkeleton, SkNode, SkNodeKind, Span};
use crate::WorkspaceSandbox;
use ledger::Event;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const MAX_HYPOTHESIS_CHARS: usize = 400;
const MAX_EDGE_REASON_CHARS: usize = 600;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InputAnchor {
    pub source_id: String,
    #[serde(default)]
    pub path: Option<String>,
    pub start_line: usize,
    pub end_line: usize,
    pub quote: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InputHypothesis {
    pub text: String,
    pub anchor: InputAnchor,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputNewNode {
    id: String,
    attach_to: String,
    kind: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputReadingItem {
    #[serde(default)] tag: Option<String>,
    #[serde(default)] new: Option<InputNewNode>,
    #[serde(default)] role: Option<ReadingRole>,
    #[serde(default)] gist: Option<String>,
    #[serde(default)] purpose: Option<String>,
    #[serde(default)] hypotheses: Vec<InputHypothesis>,
    #[serde(default)] anchors: Vec<InputAnchor>,
    #[serde(default)] authors_claim: Option<InputAuthorsClaim<InputAnchor>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputModelEdge {
    from: String,
    to: String,
    role: ModelEdgeRole,
    basis: ModelEdgeBasis,
    reason: String,
    anchor: InputAnchor,
}

#[derive(Debug, Default)]
pub(crate) struct ReadingAnchorReport {
    pub dropped: Vec<Value>,
    pub corrected: Vec<Value>,
}

impl ReadingAnchorReport {
    pub fn to_value(&self) -> Value { json!({ "dropped": self.dropped, "corrected": self.corrected }) }
}

pub(super) struct ValidationCtx<'a> {
    pub sandbox: WorkspaceSandbox,
    pub session: &'a str,
    pub caller: &'a str,
    pub source_id: &'a str,
    pub receipt_primary_path: Option<&'a str>,
    pub events: &'a [Event],
    pub sent: &'a SentLines,
    pub skeleton: &'a PaperSkeleton,
    pub nodes_by_tag: BTreeMap<&'a str, &'a SkNode>,
    pub proofs_for_target: BTreeMap<&'a str, Vec<&'a SkNode>>,
    pub bib_keys: BTreeSet<&'a str>,
    pub model_nodes: BTreeMap<String, ModelNodeRef>,
    pub temp_id_map: BTreeMap<String, String>,
    pub open_item_scope: Option<BTreeSet<String>>,
    pub open_edge_scope: Option<BTreeSet<String>>,
}

pub(super) struct ValidatedItem {
    pub record: ReadingItemRecord,
    pub temp_id: Option<String>,
}

pub(crate) struct AnchorErr {
    pub owner: String,
    pub code: String,
    pub message: String,
    pub true_text: Option<String>,
}

pub(crate) fn fault(
    index: usize,
    code: impl Into<String>,
    message: impl Into<String>,
    true_text: Option<String>,
) -> IndexedFault {
    IndexedFault { index, code: code.into(), message: message.into(), true_text }
}

pub(crate) fn span_contains_anchor(spans: &[Span], anchor: &VerifiedMapAnchor) -> bool {
    spans.iter().any(|s| s.path == anchor.path && s.start_line <= anchor.start_line && anchor.end_line <= s.end_line)
}

fn anchor_in_node_or_proof(ctx: &ValidationCtx<'_>, root_tag: &str, root_node: &SkNode, anchor: &VerifiedMapAnchor) -> bool {
    if span_contains_anchor(&root_node.spans, anchor) {
        return true;
    }
    ctx.proofs_for_target
        .get(root_tag)
        .is_some_and(|ps| ps.iter().any(|p| span_contains_anchor(&p.spans, anchor)))
}

fn nearby_valid_tags(ctx: &ValidationCtx<'_>, missing: &str, scope: Option<&BTreeSet<String>>) -> String {
    let mut scored: Vec<(usize, usize, &str)> = ctx.skeleton.nodes.iter().filter_map(|n| {
        let tag = n.tag.as_deref()?;
        if scope.is_some_and(|allowed| !allowed.contains(tag)) { return None; }
        let prefix = tag.bytes().zip(missing.bytes()).take_while(|(a, b)| a == b).count();
        let shared = missing.bytes().filter(|b| tag.as_bytes().contains(b)).count();
        Some((prefix * 4 + shared, n.order, tag))
    }).collect();
    for (idx, mtag) in ctx.model_nodes.keys().enumerate() {
        if scope.is_some_and(|allowed| !allowed.contains(mtag.as_str())) { continue; }
        let prefix = mtag.bytes().zip(missing.bytes()).take_while(|(a, b)| a == b).count();
        let shared = missing.bytes().filter(|b| mtag.as_bytes().contains(b)).count();
        scored.push((prefix * 4 + shared, ctx.skeleton.nodes.len() + idx, mtag.as_str()));
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let picked: Vec<&str> = scored.into_iter().take(6).map(|(_, _, t)| t).collect();
    if picked.is_empty() { "none".to_string() } else { picked.join(", ") }
}

fn resolve_tag_or_temp(ctx: &ValidationCtx<'_>, raw: &str) -> String {
    let trimmed = raw.trim();
    ctx.temp_id_map
        .get(trimmed)
        .cloned()
        .unwrap_or_else(|| trimmed.to_string())
}

fn resolve_owning_skeleton<'a>(ctx: &'a ValidationCtx<'a>, tag: &str) -> Option<(&'a str, &'a SkNode)> {
    if let Some((&sk_tag, &sk_node)) = ctx.nodes_by_tag.get_key_value(tag) {
        return Some((sk_tag, sk_node));
    }
    let m = ctx.model_nodes.get(tag)?;
    let (&sk_tag, &sk_node) = ctx.nodes_by_tag.get_key_value(m.root_skeleton_tag.as_str())?;
    Some((sk_tag, sk_node))
}

fn to_verified_map_anchor(vq: VerifiedQuote, quote: &str) -> VerifiedMapAnchor {
    VerifiedMapAnchor {
        source_id: vq.quote_ref.source_id, path: vq.quote_ref.path,
        start_line: vq.quote_ref.start_line, end_line: vq.quote_ref.end_line,
        file_sha256: vq.quote_ref.file_sha256, quote_sha256: vq.quote_ref.quote_sha256,
        quote: quote.to_string(), excerpt: vq.excerpt, source_seq: vq.source_seq,
        cited_range: vq.cited_range, range_basis: vq.range_basis, excerpt_window: vq.excerpt_window,
    }
}

pub(crate) fn verify_anchor_with_fallback(
    sandbox: &WorkspaceSandbox,
    session: &str,
    events: &[Event],
    sent: &SentLines,
    caller: &str,
    owner: String,
    anchor: &InputAnchor,
    fallback_path: Option<&str>,
) -> Result<(VerifiedMapAnchor, Option<Value>), AnchorErr> {
    let path = anchor
        .path
        .as_deref()
        .filter(|s| !s.is_empty())
        .or(fallback_path)
        .ok_or_else(|| AnchorErr {
            owner: owner.clone(),
            code: "missing_path".into(),
            message: "Anchor is missing `path` and no default source file path is available.".into(),
            true_text: None,
        })?;
    match accept_quote_with_sent(
        sandbox,
        session,
        events,
        sent,
        caller,
        &anchor.source_id,
        path,
        anchor.start_line,
        anchor.end_line,
        &anchor.quote,
    ) {
        Ok(vq) => {
            let corr = vq.cited_range.map(|from| {
                json!({
                    "owner": owner,
                    "lines": [vq.quote_ref.start_line, vq.quote_ref.end_line],
                    "from": from,
                    "basis": vq.range_basis,
                })
            });
            Ok((to_verified_map_anchor(vq, &anchor.quote), corr))
        }
        Err(QuoteRejection::VerificationFailed(qf)) => Err(AnchorErr {
            owner,
            code: qf.code,
            message: qf.message,
            true_text: qf.true_text,
        }),
        Err(rej @ QuoteRejection::LinesNotSent { .. }) => Err(AnchorErr {
            owner,
            code: "lines_not_sent".into(),
            message: rej.to_string(),
            true_text: None,
        }),
    }
}

fn verify_one_anchor(
    ctx: &ValidationCtx<'_>,
    owner: String,
    anchor: &InputAnchor,
    default_path: Option<&str>,
) -> Result<(VerifiedMapAnchor, Option<Value>), AnchorErr> {
    if anchor.source_id != ctx.source_id {
        return Err(AnchorErr {
            owner,
            code: "non_primary_source".into(),
            message: format!("Anchor source_id `{}` must match primary paper `{}`.", anchor.source_id, ctx.source_id),
            true_text: None,
        });
    }
    verify_anchor_with_fallback(
        &ctx.sandbox,
        ctx.session,
        ctx.events,
        ctx.sent,
        ctx.caller,
        owner,
        anchor,
        default_path.or(ctx.receipt_primary_path),
    )
}

fn verify_anchor_list(
    ctx: &ValidationCtx<'_>, prefix: &str, anchors: &[InputAnchor], default_path: Option<&str>,
    pending_corrected: &mut Vec<Value>,
) -> (Vec<VerifiedMapAnchor>, Vec<AnchorErr>) {
    let (mut kept, mut faults) = (Vec::new(), Vec::new());
    for (ai, raw_a) in anchors.iter().enumerate() {
        match verify_one_anchor(ctx, format!("{prefix}[{ai}]"), raw_a, default_path) {
            Ok((va, corr)) => {
                kept.push(va);
                if let Some(c) = corr { pending_corrected.push(c); }
            }
            Err(err) => faults.push(err),
        }
    }
    (kept, faults)
}

enum TargetKind<'a> {
    Skeleton { tag: String, node: &'a SkNode },
    ExistingModel { tag: String, attach_to: String, kind: String, root_tag: &'a str, root_node: &'a SkNode },
    NewModel { temp_id: String, attach_to: String, kind: String, root_tag: &'a str, root_node: &'a SkNode },
}

fn resolve_item_target<'a>(
    ctx: &'a ValidationCtx<'a>, index: usize, item: &InputReadingItem,
) -> Result<TargetKind<'a>, IndexedFault> {
    if item.tag.is_some() && item.new.is_some() {
        return Err(fault(index, "invalid_item", "Specify either `tag` or `new`, not both.", None));
    }
    if let Some(ref nn) = item.new {
        let temp_id = nn.id.trim();
        if temp_id.is_empty() || temp_id.chars().count() > MAX_MODEL_NODE_TEMP_ID_CHARS || temp_id.chars().any(char::is_control) {
            return Err(fault(index, "invalid_item", "Model node `new.id` must be 1..=64 printable characters.", None));
        }
        let norm_kind = normalize_model_kind(&nn.kind)
            .map_err(|msg| fault(index, "invalid_item", msg, None))?;
        let attach_to = resolve_tag_or_temp(ctx, &nn.attach_to);
        let (root_tag, root_node) = resolve_owning_skeleton(ctx, &attach_to).ok_or_else(|| {
            let nearby = nearby_valid_tags(ctx, &attach_to, ctx.open_item_scope.as_ref());
            fault(index, "unknown_tag", format!("Unknown `attach_to` tag `{attach_to}`. Nearby valid tags: {nearby}"), None)
        })?;
        if let Some(scope) = &ctx.open_item_scope {
            if !scope.contains(attach_to.as_str()) {
                let nearby = nearby_valid_tags(ctx, &attach_to, Some(scope));
                return Err(fault(index, "out_of_scope_tag", format!("Parent tag `{attach_to}` is outside open scope. In-scope: {nearby}"), None));
            }
        }
        if let Some(role) = item.role {
            if !role_allowed_for_model_kind(role, &norm_kind) {
                return Err(fault(index, "invalid_role_for_kind", format!("Role `{role:?}` is forbidden on model kind `{norm_kind}`."), None));
            }
        }
        return Ok(TargetKind::NewModel {
            temp_id: temp_id.to_string(), attach_to, kind: norm_kind, root_tag, root_node,
        });
    }

    let raw_tag = item.tag.as_deref().filter(|s| !s.trim().is_empty()).ok_or_else(|| {
        let nearby = nearby_valid_tags(ctx, "", ctx.open_item_scope.as_ref());
        fault(index, "unknown_tag", format!("Missing item `tag`. Nearby valid tags: {nearby}"), None)
    })?;
    let resolved = resolve_tag_or_temp(ctx, raw_tag);
    if let Some(&node) = ctx.nodes_by_tag.get(resolved.as_str()) {
        if let Some(scope) = &ctx.open_item_scope {
            if !scope.contains(resolved.as_str()) {
                let nearby = nearby_valid_tags(ctx, &resolved, Some(scope));
                return Err(fault(index, "out_of_scope_tag", format!("Tag `{resolved}` is outside open scope. In-scope: {nearby}"), None));
            }
        }
        if let Some(role) = item.role {
            if !role.allowed_for_kind(node.kind) {
                return Err(fault(index, "invalid_role_for_kind", format!("Role `{role:?}` is forbidden on kind `{:?}`.", node.kind), None));
            }
        }
        return Ok(TargetKind::Skeleton { tag: resolved, node });
    }
    if let Some(m) = ctx.model_nodes.get(&resolved) {
        if let Some(scope) = &ctx.open_item_scope {
            if !scope.contains(resolved.as_str()) {
                let nearby = nearby_valid_tags(ctx, &resolved, Some(scope));
                return Err(fault(index, "out_of_scope_tag", format!("Tag `{resolved}` is outside open scope. In-scope: {nearby}"), None));
            }
        }
        let (&root_tag, &root_node) = ctx.nodes_by_tag.get_key_value(m.root_skeleton_tag.as_str()).ok_or_else(|| {
            fault(index, "unknown_tag", format!("Owning skeleton tag `{}` missing.", m.root_skeleton_tag), None)
        })?;
        if let Some(role) = item.role {
            if !role_allowed_for_model_kind(role, &m.kind) {
                return Err(fault(index, "invalid_role_for_kind", format!("Role `{role:?}` is forbidden on model kind `{}`.", m.kind), None));
            }
        }
        return Ok(TargetKind::ExistingModel {
            tag: resolved, attach_to: m.attach_to.clone(), kind: m.kind.clone(), root_tag, root_node,
        });
    }
    let nearby = nearby_valid_tags(ctx, &resolved, ctx.open_item_scope.as_ref());
    Err(fault(index, "unknown_tag", format!("Unknown tag `{resolved}`. Nearby valid tags: {nearby}"), None))
}

pub(super) fn validate_item(
    ctx: &mut ValidationCtx<'_>, index: usize, raw: Value, report: &mut ReadingAnchorReport,
) -> Result<ValidatedItem, IndexedFault> {
    let item: InputReadingItem = serde_json::from_value(raw)
        .map_err(|e| fault(index, "invalid_item", format!("Invalid reading item: {e}"), None))?;

    let has_reading = item.role.is_some() || item.gist.is_some() || item.purpose.is_some() || !item.hypotheses.is_empty();
    if !has_reading && item.authors_claim.is_none() {
        return Err(fault(index, "empty_item", "Item needs at least one of role, gist, purpose, hypotheses, or authors_claim.", None));
    }
    if item.gist.as_deref().is_some_and(|g| !bounded(g, MAX_GIST_CHARS))
        || item.purpose.as_deref().is_some_and(|p| !bounded(p, MAX_PURPOSE_CHARS))
        || item.hypotheses.len() > MAX_READING_HYPOTHESES
        || item.hypotheses.iter().any(|h| !bounded(&h.text, MAX_HYPOTHESIS_CHARS))
        || item.anchors.len() > MAX_READING_ANCHORS
    {
        return Err(fault(index, "invalid_item", "Item field exceeds length or count bounds.", None));
    }

    let (is_new, target_tag_hint, root_tag, root_fp, default_path, attach_info) = {
        let target = resolve_item_target(ctx, index, &item)?;
        match target {
            TargetKind::Skeleton { tag, node } => (
                false, Some(tag), None, node.fp.clone(),
                node.spans.first().map(|s| s.path.clone()), None,
            ),
            TargetKind::ExistingModel { tag, attach_to, kind, root_tag, root_node } => (
                false, Some(tag), Some(root_tag.to_string()), root_node.fp.clone(),
                root_node.spans.first().map(|s| s.path.clone()),
                Some((None, attach_to, kind)),
            ),
            TargetKind::NewModel { temp_id, attach_to, kind, root_tag, root_node } => (
                true, None, Some(root_tag.to_string()), root_node.fp.clone(),
                root_node.spans.first().map(|s| s.path.clone()),
                Some((Some(temp_id), attach_to, kind)),
            ),
        }
    };

    let (mut pending_dropped, mut pending_corrected) = (Vec::new(), Vec::new());
    let (kept_anchors, anchor_faults) = verify_anchor_list(
        ctx, &format!("items[{index}].anchors"), &item.anchors, default_path.as_deref(), &mut pending_corrected,
    );
    let needs_anchors = has_reading || is_new;
    if needs_anchors {
        if item.anchors.is_empty() {
            return Err(fault(index, "missing_anchors", "Reading fields and new model nodes require 1..=3 matched anchors.", None));
        }
        if kept_anchors.is_empty() {
            return Err(match anchor_faults.into_iter().next() {
                Some(first) => fault(index, first.code, first.message, first.true_text),
                None => fault(index, "missing_anchors", "No anchor matched for this item.", None),
            });
        }
        let contained = match (&root_tag, &target_tag_hint) {
            (Some(rtag), _) => {
                let rnode = ctx.nodes_by_tag.get(rtag.as_str()).copied();
                rnode.is_some_and(|rn| kept_anchors.iter().any(|a| anchor_in_node_or_proof(ctx, rtag, rn, a)))
            }
            (None, Some(sk_tag)) => {
                let snode = ctx.nodes_by_tag.get(sk_tag.as_str()).copied();
                snode.is_some_and(|sn| kept_anchors.iter().any(|a| span_contains_anchor(&sn.spans, a)))
            }
            _ => false,
        };
        if !contained {
            let label = target_tag_hint.as_deref().or(root_tag.as_deref()).unwrap_or("node");
            return Err(fault(index, "anchor_outside_node", format!("No matched anchor lies inside node `{label}` spans."), None));
        }
        pending_dropped.extend(anchor_faults.iter().map(|e| json!({ "owner": e.owner, "code": e.code })));
    }

    let mut verified_hyps = Vec::with_capacity(item.hypotheses.len());
    for (hi, hyp) in item.hypotheses.into_iter().enumerate() {
        let (va, corr) = verify_one_anchor(ctx, format!("items[{index}].hypotheses[{hi}].anchor"), &hyp.anchor, default_path.as_deref())
            .map_err(|e| fault(index, e.code, e.message, e.true_text))?;
        if let Some(c) = corr { pending_corrected.push(c); }
        verified_hyps.push(HypothesisRecord { text: hyp.text.trim().to_string(), anchor: va });
    }

    let mut item_anchor_faults: Vec<ClaimAnchorFault> = anchor_faults.into_iter()
        .map(|e| ClaimAnchorFault { owner: e.owner, code: e.code, message: e.message, true_text: e.true_text })
        .collect();
    let authors_claim = if let Some(ac) = item.authors_claim {
        let claim = validate_authors_claim(
            index, ac, ctx.source_id, &ctx.bib_keys, &kept_anchors,
            &mut item_anchor_faults, &mut pending_dropped,
            |raw_anchors| {
                let (kept, faults) = verify_anchor_list(
                    ctx, &format!("items[{index}].authors_claim.anchors"), raw_anchors,
                    default_path.as_deref(), &mut pending_corrected,
                );
                let mapped = faults.into_iter()
                    .map(|e| ClaimAnchorFault { owner: e.owner, code: e.code, message: e.message, true_text: e.true_text })
                    .collect();
                (kept, mapped)
            },
        )?;
        Some(claim)
    } else {
        None
    };

    let (final_tag, temp_id, attach_to, model_kind) = match attach_info {
        Some((Some(tid), parent, kind)) => {
            let assigned = model_node_tag(&parent, &kind, &kept_anchors[0].quote)
                .map_err(|msg| fault(index, "invalid_item", msg, None))?;
            if let Some(rtag) = root_tag {
                ctx.model_nodes.insert(assigned.clone(), ModelNodeRef {
                    tag: assigned.clone(), attach_to: parent.clone(), kind: kind.clone(),
                    root_skeleton_tag: rtag, node_fp: root_fp.clone(),
                });
            }
            ctx.temp_id_map.insert(tid.clone(), assigned.clone());
            if let Some(scope) = ctx.open_item_scope.as_mut() { scope.insert(assigned.clone()); }
            if let Some(scope) = ctx.open_edge_scope.as_mut() { scope.insert(assigned.clone()); }
            (assigned, Some(tid), Some(parent), Some(kind))
        }
        Some((None, parent, kind)) => (
            target_tag_hint.unwrap_or_default(), None, Some(parent), Some(kind),
        ),
        None => (target_tag_hint.unwrap_or_default(), None, None, None),
    };

    if !needs_anchors && !item_anchor_faults.is_empty() {
        pending_dropped.extend(item_anchor_faults.iter().map(|e| json!({ "owner": e.owner, "code": e.code })));
    }
    report.dropped.extend(pending_dropped);
    report.corrected.extend(pending_corrected);

    Ok(ValidatedItem {
        record: ReadingItemRecord {
            tag: final_tag, node_fp: root_fp, attach_to, model_kind,
            role: item.role, gist: item.gist.map(|s| s.trim().to_string()),
            purpose: item.purpose.map(|s| s.trim().to_string()),
            hypotheses: verified_hyps, anchors: kept_anchors, authors_claim,
        },
        temp_id,
    })
}

pub(super) fn validate_edge(
    ctx: &ValidationCtx<'_>, index: usize, raw: Value, report: &mut ReadingAnchorReport,
) -> Result<ModelEdgeRecord, IndexedFault> {
    let edge: InputModelEdge = serde_json::from_value(raw)
        .map_err(|e| fault(index, "invalid_edge", format!("Invalid model edge: {e}"), None))?;
    if !bounded(&edge.reason, MAX_EDGE_REASON_CHARS) {
        return Err(fault(index, "invalid_edge", format!("Edge `reason` must be 1..={MAX_EDGE_REASON_CHARS} chars."), None));
    }
    let resolved_from = resolve_tag_or_temp(ctx, &edge.from);
    let resolved_to = resolve_tag_or_temp(ctx, &edge.to);
    let (_, from_node) = resolve_owning_skeleton(ctx, &resolved_from).ok_or_else(|| {
        let nearby = nearby_valid_tags(ctx, &resolved_from, ctx.open_edge_scope.as_ref());
        fault(index, "unknown_tag", format!("Unknown `from` tag `{resolved_from}`. Nearby: {nearby}"), None)
    })?;
    let (to_root_tag, to_node) = resolve_owning_skeleton(ctx, &resolved_to).ok_or_else(|| {
        let nearby = nearby_valid_tags(ctx, &resolved_to, ctx.open_edge_scope.as_ref());
        fault(index, "unknown_tag", format!("Unknown `to` tag `{resolved_to}`. Nearby: {nearby}"), None)
    })?;
    if resolved_from == resolved_to {
        return Err(fault(index, "self_edge", format!("Edge cannot connect `{resolved_from}` to itself."), None));
    }
    if let (Some(edge_scope), Some(item_scope)) = (&ctx.open_edge_scope, &ctx.open_item_scope) {
        let ok_ends = edge_scope.contains(resolved_from.as_str()) && item_scope.contains(resolved_to.as_str());
        if !ok_ends {
            return Err(fault(index, "out_of_scope_tag", format!("Edge `{resolved_from} -> {resolved_to}` is outside open scope."), None));
        }
    }

    let default_path = to_node.spans.first().or_else(|| from_node.spans.first()).map(|s| s.path.as_str());
    let (verified, corr) = verify_one_anchor(ctx, format!("edges[{index}].anchor"), &edge.anchor, default_path)
        .map_err(|e| fault(index, e.code, e.message, e.true_text))?;

    match edge.role {
        ModelEdgeRole::Statement if !span_contains_anchor(&to_node.spans, &verified) => {
            return Err(fault(index, "scope_mismatch", format!("`statement` edge anchor must lie inside `{resolved_to}`."), None));
        }
        ModelEdgeRole::Proof => {
            let in_target = to_node.kind == SkNodeKind::Proof && span_contains_anchor(&to_node.spans, &verified);
            let in_proof = ctx.proofs_for_target.get(to_root_tag)
                .is_some_and(|ps| ps.iter().any(|p| span_contains_anchor(&p.spans, &verified)));
            if !in_target && !in_proof {
                return Err(fault(index, "scope_mismatch", format!("`proof` edge anchor must lie inside a proof of `{resolved_to}`."), None));
            }
        }
        ModelEdgeRole::Statement | ModelEdgeRole::Argument => {}
    }
    if let Some(c) = corr { report.corrected.push(c); }
    Ok(ModelEdgeRecord {
        from: resolved_from, to: resolved_to, role: edge.role, basis: edge.basis,
        reason: edge.reason.trim().to_string(), anchor: verified,
    })
}
