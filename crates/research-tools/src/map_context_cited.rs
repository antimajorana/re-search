//! Rule 6.9 external citation lookup and located-node resolution for `record_context_cards`.

use super::{CitedExternalTarget, ContextValidationEnv};
use crate::investigation_markers::EVENT_INVESTIGATION_SKELETON;
use crate::map_context_defs::EVENT_INVESTIGATION_CITED_LOOKUP;
use crate::map_defs::IndexedFault;
use crate::map_reading::validate::fault;
use crate::paper_key::PaperKey;
use crate::skeleton::{
    build_html_skeleton_from_ingested, build_pdf_skeleton_from_ingested,
    build_tex_skeleton_from_ingested, derive_receipt_paper_identity, is_html_receipt,
    is_pdf_receipt, is_tex_receipt, PaperSkeleton, Span, CITED_STATUS_FOUND,
};
use ledger::Event;

pub(super) fn resolve_cited_external_target(
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
