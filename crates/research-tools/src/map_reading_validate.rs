//! Per-item and per-edge validation helpers for `record_map_reading` (package V6).

use crate::investigation::anchor::bounded;
use crate::lines_sent::SentLines;
use crate::map_authors_claim::{validate_authors_claim, ClaimAnchorFault, InputAuthorsClaim};
use crate::map_defs::{
    HypothesisRecord, IndexedFault, ModelEdgeBasis, ModelEdgeRecord, ModelEdgeRole,
    ReadingItemRecord, ReadingRole, VerifiedMapAnchor, MAX_GIST_CHARS, MAX_PURPOSE_CHARS,
    MAX_READING_ANCHORS, MAX_READING_HYPOTHESES,
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
pub(super) struct InputAnchor {
    pub source_id: String,
    #[serde(default)]
    pub path: Option<String>,
    pub start_line: usize,
    pub end_line: usize,
    pub quote: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputHypothesis { text: String, anchor: InputAnchor }

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputNewNode {
    #[allow(dead_code)] id: String,
    #[allow(dead_code)] attach_to: String,
    #[allow(dead_code)] kind: String,
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
pub(super) struct ReadingAnchorReport {
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
    pub open_item_scope: Option<BTreeSet<&'a str>>,
    pub open_edge_scope: Option<BTreeSet<&'a str>>,
}

struct AnchorErr { owner: String, code: String, message: String, true_text: Option<String> }

fn fault(index: usize, code: impl Into<String>, message: impl Into<String>, true_text: Option<String>) -> IndexedFault {
    IndexedFault { index, code: code.into(), message: message.into(), true_text }
}

pub(super) fn span_contains_anchor(spans: &[Span], anchor: &VerifiedMapAnchor) -> bool {
    spans.iter().any(|s| s.path == anchor.path && s.start_line <= anchor.start_line && anchor.end_line <= s.end_line)
}

fn nearby_valid_tags(ctx: &ValidationCtx<'_>, missing: &str, scope: Option<&BTreeSet<&str>>) -> String {
    let mut scored: Vec<(usize, usize, &str)> = ctx.skeleton.nodes.iter().filter_map(|n| {
        let tag = n.tag.as_deref()?;
        if scope.is_some_and(|allowed| !allowed.contains(tag)) { return None; }
        let prefix = tag.bytes().zip(missing.bytes()).take_while(|(a, b)| a == b).count();
        let shared = missing.bytes().filter(|b| tag.as_bytes().contains(b)).count();
        Some((prefix * 4 + shared, n.order, tag))
    }).collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let picked: Vec<&str> = scored.into_iter().take(6).map(|(_, _, t)| t).collect();
    if picked.is_empty() { "none".to_string() } else { picked.join(", ") }
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

fn verify_one_anchor(
    ctx: &ValidationCtx<'_>, owner: String, anchor: &InputAnchor, default_path: Option<&str>,
) -> Result<(VerifiedMapAnchor, Option<Value>), AnchorErr> {
    if anchor.source_id != ctx.source_id {
        return Err(AnchorErr {
            owner, code: "non_primary_source".into(),
            message: format!("Anchor source_id `{}` must match primary paper `{}`.", anchor.source_id, ctx.source_id),
            true_text: None,
        });
    }
    let path = anchor.path.as_deref().filter(|s| !s.is_empty()).or(default_path).or(ctx.receipt_primary_path)
        .ok_or_else(|| AnchorErr {
            owner: owner.clone(), code: "missing_path".into(),
            message: "Anchor is missing `path` and no default source file path is available.".into(),
            true_text: None,
        })?;
    match accept_quote_with_sent(
        &ctx.sandbox, ctx.session, ctx.events, ctx.sent, ctx.caller,
        &anchor.source_id, path, anchor.start_line, anchor.end_line, &anchor.quote,
    ) {
        Ok(vq) => {
            let corr = vq.cited_range.map(|from| json!({
                "owner": owner, "lines": [vq.quote_ref.start_line, vq.quote_ref.end_line],
                "from": from, "basis": vq.range_basis,
            }));
            Ok((to_verified_map_anchor(vq, &anchor.quote), corr))
        }
        Err(QuoteRejection::VerificationFailed(qf)) => {
            Err(AnchorErr { owner, code: qf.code, message: qf.message, true_text: qf.true_text })
        }
        Err(rej @ QuoteRejection::LinesNotSent { .. }) => {
            Err(AnchorErr { owner, code: "lines_not_sent".into(), message: rej.to_string(), true_text: None })
        }
    }
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

pub(super) fn validate_item(
    ctx: &ValidationCtx<'_>, index: usize, raw: Value, report: &mut ReadingAnchorReport,
) -> Result<ReadingItemRecord, IndexedFault> {
    let item: InputReadingItem = serde_json::from_value(raw)
        .map_err(|e| fault(index, "invalid_item", format!("Invalid reading item: {e}"), None))?;
    if item.new.is_some() {
        return Err(fault(index, "unsupported_model_node", "Model-created nodes (`new`) are not supported in V6.", None));
    }
    let tag = item.tag.as_deref().filter(|s| !s.is_empty()).ok_or_else(|| {
        let nearby = nearby_valid_tags(ctx, "", ctx.open_item_scope.as_ref());
        fault(index, "unknown_tag", format!("Missing item `tag`. Nearby valid tags: {nearby}"), None)
    })?;
    let node = ctx.nodes_by_tag.get(tag).copied().ok_or_else(|| {
        let nearby = nearby_valid_tags(ctx, tag, ctx.open_item_scope.as_ref());
        fault(index, "unknown_tag", format!("Unknown tag `{tag}`. Nearby valid tags: {nearby}"), None)
    })?;
    if let Some(scope) = &ctx.open_item_scope {
        if !scope.contains(tag) {
            let nearby = nearby_valid_tags(ctx, tag, Some(scope));
            return Err(fault(index, "out_of_scope_tag", format!("Tag `{tag}` is outside open scope. In-scope: {nearby}"), None));
        }
    }

    let has_reading = item.role.is_some() || item.gist.is_some() || item.purpose.is_some() || !item.hypotheses.is_empty();
    if !has_reading && item.authors_claim.is_none() {
        return Err(fault(index, "empty_item", "Item needs at least one of role, gist, purpose, hypotheses, or authors_claim.", None));
    }
    if let Some(role) = item.role {
        if !role.allowed_for_kind(node.kind) {
            return Err(fault(index, "invalid_role_for_kind", format!("Role `{role:?}` is forbidden on kind `{:?}`.", node.kind), None));
        }
    }
    if item.gist.as_deref().is_some_and(|g| !bounded(g, MAX_GIST_CHARS))
        || item.purpose.as_deref().is_some_and(|p| !bounded(p, MAX_PURPOSE_CHARS))
        || item.hypotheses.len() > MAX_READING_HYPOTHESES
        || item.hypotheses.iter().any(|h| !bounded(&h.text, MAX_HYPOTHESIS_CHARS))
        || item.anchors.len() > MAX_READING_ANCHORS
    {
        return Err(fault(index, "invalid_item", "Item field exceeds length or count bounds.", None));
    }

    let default_path = node.spans.first().map(|s| s.path.as_str());
    let (mut pending_dropped, mut pending_corrected) = (Vec::new(), Vec::new());
    let (kept_anchors, anchor_faults) =
        verify_anchor_list(ctx, &format!("items[{index}].anchors"), &item.anchors, default_path, &mut pending_corrected);
    if has_reading {
        if item.anchors.is_empty() {
            return Err(fault(index, "missing_anchors", "Reading fields require 1..=3 matched anchors inside the node.", None));
        }
        if kept_anchors.is_empty() {
            return Err(match anchor_faults.into_iter().next() {
                Some(first) => fault(index, first.code, first.message, first.true_text),
                None => fault(index, "missing_anchors", "No anchor matched for this item.", None),
            });
        }
        if !kept_anchors.iter().any(|a| span_contains_anchor(&node.spans, a)) {
            return Err(fault(index, "anchor_outside_node", format!("No matched anchor lies inside node `{tag}` spans."), None));
        }
        pending_dropped.extend(anchor_faults.iter().map(|e| json!({ "owner": e.owner, "code": e.code })));
    }

    let mut verified_hyps = Vec::with_capacity(item.hypotheses.len());
    for (hi, hyp) in item.hypotheses.into_iter().enumerate() {
        let (va, corr) = verify_one_anchor(ctx, format!("items[{index}].hypotheses[{hi}].anchor"), &hyp.anchor, default_path)
            .map_err(|e| fault(index, e.code, e.message, e.true_text))?;
        if let Some(c) = corr { pending_corrected.push(c); }
        verified_hyps.push(HypothesisRecord { text: hyp.text.trim().to_string(), anchor: va });
    }

    let mut item_anchor_faults: Vec<ClaimAnchorFault> = anchor_faults
        .into_iter()
        .map(|e| ClaimAnchorFault {
            owner: e.owner,
            code: e.code,
            message: e.message,
            true_text: e.true_text,
        })
        .collect();
    let authors_claim = if let Some(ac) = item.authors_claim {
        let claim = validate_authors_claim(
            index,
            ac,
            ctx.source_id,
            &ctx.bib_keys,
            &kept_anchors,
            &mut item_anchor_faults,
            &mut pending_dropped,
            |raw_anchors| {
                let (kept, faults) = verify_anchor_list(
                    ctx,
                    &format!("items[{index}].authors_claim.anchors"),
                    raw_anchors,
                    default_path,
                    &mut pending_corrected,
                );
                let mapped = faults
                    .into_iter()
                    .map(|e| ClaimAnchorFault {
                        owner: e.owner,
                        code: e.code,
                        message: e.message,
                        true_text: e.true_text,
                    })
                    .collect();
                (kept, mapped)
            },
        )?;
        Some(claim)
    } else {
        None
    };

    if !has_reading && !item_anchor_faults.is_empty() {
        pending_dropped.extend(item_anchor_faults.iter().map(|e| json!({ "owner": e.owner, "code": e.code })));
    }
    report.dropped.extend(pending_dropped);
    report.corrected.extend(pending_corrected);

    Ok(ReadingItemRecord {
        tag: tag.to_string(), node_fp: node.fp.clone(), role: item.role,
        gist: item.gist.map(|s| s.trim().to_string()),
        purpose: item.purpose.map(|s| s.trim().to_string()),
        hypotheses: verified_hyps, anchors: kept_anchors, authors_claim,
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
    let from_node = ctx.nodes_by_tag.get(edge.from.as_str()).copied().ok_or_else(|| {
        let nearby = nearby_valid_tags(ctx, &edge.from, ctx.open_edge_scope.as_ref());
        fault(index, "unknown_tag", format!("Unknown `from` tag `{}`. Nearby: {nearby}", edge.from), None)
    })?;
    let to_node = ctx.nodes_by_tag.get(edge.to.as_str()).copied().ok_or_else(|| {
        let nearby = nearby_valid_tags(ctx, &edge.to, ctx.open_edge_scope.as_ref());
        fault(index, "unknown_tag", format!("Unknown `to` tag `{}`. Nearby: {nearby}", edge.to), None)
    })?;
    if edge.from == edge.to {
        return Err(fault(index, "self_edge", format!("Edge cannot connect `{}` to itself.", edge.from), None));
    }
    if let (Some(edge_scope), Some(item_scope)) = (&ctx.open_edge_scope, &ctx.open_item_scope) {
        let ok_ends = edge_scope.contains(edge.from.as_str()) && item_scope.contains(edge.to.as_str());
        if !ok_ends {
            return Err(fault(index, "out_of_scope_tag", format!("Edge `{} -> {}` is outside open scope.", edge.from, edge.to), None));
        }
    }

    let default_path = to_node.spans.first().or_else(|| from_node.spans.first()).map(|s| s.path.as_str());
    let (verified, corr) = verify_one_anchor(ctx, format!("edges[{index}].anchor"), &edge.anchor, default_path)
        .map_err(|e| fault(index, e.code, e.message, e.true_text))?;

    match edge.role {
        ModelEdgeRole::Statement if !span_contains_anchor(&to_node.spans, &verified) => {
            return Err(fault(index, "scope_mismatch", format!("`statement` edge anchor must lie inside `{}`.", edge.to), None));
        }
        ModelEdgeRole::Proof => {
            let in_target = to_node.kind == SkNodeKind::Proof && span_contains_anchor(&to_node.spans, &verified);
            let in_proof = ctx.proofs_for_target.get(edge.to.as_str())
                .is_some_and(|ps| ps.iter().any(|p| span_contains_anchor(&p.spans, &verified)));
            if !in_target && !in_proof {
                return Err(fault(index, "scope_mismatch", format!("`proof` edge anchor must lie inside a proof of `{}`.", edge.to), None));
            }
        }
        ModelEdgeRole::Statement | ModelEdgeRole::Argument => {}
    }
    if let Some(c) = corr { report.corrected.push(c); }
    Ok(ModelEdgeRecord {
        from: edge.from, to: edge.to, role: edge.role, basis: edge.basis,
        reason: edge.reason.trim().to_string(), anchor: verified,
    })
}
