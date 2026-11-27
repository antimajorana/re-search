//! Rule 6.11 fail-closed `.rsmap` importer (zero network calls).
//!
//! Locates a matching local paper source via arXiv + version in `Library` or matching
//! `raw_sha256` / `files_root_sha256` across session receipts, re-ingests host-side into
//! `source.ingested` and rebuilds `investigation.skeleton` (with `"raw_blob"` so
//! `Ledger::verify_all().unreferenced_blobs == 0`), re-validates every quote via `quote_core`,
//! drops mismatched items, or leaves a read-only fallback `"quotes not matched on this machine"`.

use crate::paper_map::{
    ExportAnchor, ExportClaimBody, ExportHypothesis, ExportModelNode,
    ExportNodeReading, PaperMapError, PaperMapExport, EVENT_INVESTIGATION_PAPER, RSMAP_FORMAT,
    RSMAP_SCHEMA_VERSION,
};
use ledger::{sha256_hex, Event, Ledger};
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::investigation_markers::{
    EVENT_INVESTIGATION_NOTE, EVENT_INVESTIGATION_READING, EVENT_INVESTIGATION_SKELETON,
};
use research_tools::library::Library;
use research_tools::map_defs::{
    AuthorsClaimLabel, AuthorsClaimRecord, HypothesisRecord, MapReadingEventPayload,
    ModelEdgeRecord, ReadingItemRecord, ReadingRejected, VerifiedMapAnchor,
};
use research_tools::map_model_nodes::{model_node_tag, normalize_model_kind, role_allowed_for_model_kind};
use research_tools::paper_key::{split_arxiv_id, Mode, PaperKey};
use research_tools::quote_core::{clip_quote, verify_quote, VerifiedQuote};
use research_tools::sandbox::WorkspaceSandbox;
use research_tools::skeleton::{
    build_html_skeleton_from_ingested, build_pdf_skeleton_from_ingested,
    build_tex_skeleton_from_ingested, compute_files_root_sha256, is_html_receipt, PaperSkeleton,
    SkNode, SkNodeKind, Span,
};
use research_tools::status_badge::detect_cues_from_anchors;
use research_tools::tool_profile::session_mode_from_events;
use research_tools::validate_session_id;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Mutex;

pub const READ_ONLY_QUOTES_BANNER: &str = "quotes not matched on this machine";
const MAX_RSMAP_BYTES: usize = 8 * 1024 * 1024;
const EVENT_SOURCE_INGESTED: &str = "source.ingested";

/// Result summary returned after attempting a Rule 6.11 `.rsmap` import.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportPaperMapReport {
    pub sha256: String,
    pub paper_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub source_match: bool,
    pub read_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skeleton_seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skeleton_blob: Option<String>,
    pub items_matched: usize,
    pub items_dropped: usize,
    pub banner: String,
    #[serde(default)]
    pub dropped_reasons: Vec<String>,
    pub imported_map: PaperMapExport,
}

enum LocalSourceCandidate {
    ExistingReceipt { source_id: String },
    RawBytes { file_name: String, origin: String, title: Option<String>, raw_bytes: Vec<u8> },
}

fn validate_export_shape(raw: &[u8]) -> Result<PaperMapExport, PaperMapError> {
    if raw.is_empty() || raw.len() > MAX_RSMAP_BYTES {
        return Err(PaperMapError::bad("`.rsmap` payload must be between 1 byte and 8 MiB."));
    }
    let map: PaperMapExport = serde_json::from_slice(raw)
        .map_err(|e| PaperMapError::bad(format!("Invalid `.rsmap` JSON: {e}")))?;
    if map.schema_version != RSMAP_SCHEMA_VERSION || map.format != RSMAP_FORMAT {
        return Err(PaperMapError::bad("Unsupported `.rsmap` schema version or format."));
    }
    PaperKey::parse(&map.paper_key)
        .map_err(|e| PaperMapError::bad(format!("Invalid `paper_key` in `.rsmap`: {e}")))?;
    if map.files_root_sha256.len() != 64
        || !map.files_root_sha256.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(PaperMapError::bad("Invalid `files_root_sha256` in `.rsmap`."));
    }
    Ok(map)
}

fn ext_for_format(format: &str) -> &'static str {
    match format {
        "pdf" => "pdf",
        "tex" | "latex" => "tex",
        "tar_gz" | "tar.gz" | "eprint" => "tar.gz",
        "zip" => "zip",
        "html" => "html",
        _ => "txt",
    }
}

fn is_tex_receipt(receipt: &Event) -> bool {
    let primary = receipt.payload["primary_path"].as_str();
    let assembled = receipt.payload["assembled_path"].as_str();
    receipt.payload["files"].as_array().is_some_and(|files| {
        files.iter().filter_map(Value::as_str).any(|rel| {
            !(assembled == Some(rel) && primary != Some(rel))
                && (rel.to_ascii_lowercase().ends_with(".tex") || rel.to_ascii_lowercase().ends_with(".ltx"))
        })
    })
}

fn is_pdf_receipt(receipt: &Event) -> bool {
    if is_tex_receipt(receipt) { return false; }
    let p = &receipt.payload;
    let status = p["status"].as_str().unwrap_or("");
    matches!(status, "ok" | "pdf_only" | "empty_text_layer")
        && (p["format"].as_str() == Some("pdf") || status == "pdf_only"
            || p.get("pdf_extractor").is_some_and(Value::is_string)
            || p.get("pdf_pages").is_some_and(Value::is_array))
}

fn receipt_matches_key(receipt: &Event, target_key: &PaperKey, target_ver: Option<&str>) -> bool {
    ["arxiv_cover_id", "origin"].iter().any(|field| {
        receipt.payload[field].as_str().is_some_and(|raw| {
            split_arxiv_id(raw).is_ok_and(|(base, ver)| {
                PaperKey::from_arxiv(&base).ok().as_ref() == Some(target_key)
                    && target_ver.is_none_or(|tv| ver.as_deref() == Some(tv))
            })
        })
    })
}

fn receipt_matches_raw_or_files(receipt: &Event, map: &PaperMapExport) -> bool {
    if map.raw_sha256.as_deref().is_some_and(|w| receipt.payload["raw_blob"].as_str() == Some(w)) {
        return true;
    }
    let Some(hashes) = receipt.payload["files_sha256"].as_object() else { return false };
    let abs_map: BTreeMap<String, String> = hashes.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect();
    compute_files_root_sha256(&abs_map) == map.files_root_sha256
}

fn find_local_candidate(
    data_dir: &Path, ledger: &Ledger, target_events: &[Event], map: &PaperMapExport,
) -> Result<Option<LocalSourceCandidate>, PaperMapError> {
    let paper_key = PaperKey::parse(&map.paper_key).map_err(PaperMapError::bad)?;
    if let Some(ev) = target_events.iter().rev().find(|e| {
        e.kind == EVENT_SOURCE_INGESTED && (receipt_matches_raw_or_files(e, map) || receipt_matches_key(e, &paper_key, map.version.as_deref()))
    }) {
        if let Some(sid) = ev.payload["source_id"].as_str() {
            return Ok(Some(LocalSourceCandidate::ExistingReceipt { source_id: sid.to_string() }));
        }
    }

    if let Ok(lib) = Library::open(data_dir) {
        if let Ok(Some((rec, bytes))) = lib.read_fulltext_bytes(&paper_key, map.version.as_deref()) {
            let slug: String = paper_key.as_str().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
            let ver_sfx = map.version.as_deref().unwrap_or("");
            let title = lib.get_paper(&paper_key).ok().flatten().map(|d| d.paper.title).filter(|t| !t.trim().is_empty());
            return Ok(Some(LocalSourceCandidate::RawBytes {
                file_name: format!("{slug}.{}", ext_for_format(&rec.format)),
                origin: format!("library:{}{ver_sfx}", paper_key.as_str()),
                title, raw_bytes: bytes,
            }));
        }
    }

    for (sid, _) in ledger.sessions().map_err(|e| PaperMapError::internal(e.to_string()))? {
        let Ok(evs) = ledger.read_session(&sid) else { continue };
        for ev in evs.iter().rev().filter(|e| e.kind == EVENT_SOURCE_INGESTED) {
            if !(receipt_matches_raw_or_files(ev, map) || receipt_matches_key(ev, &paper_key, map.version.as_deref())) {
                continue;
            }
            let Some(blob) = ev.payload["raw_blob"].as_str() else { continue };
            let Ok(raw_bytes) = ledger.get_blob(blob) else { continue };
            if sha256_hex(&raw_bytes) != blob { continue; }
            let file_name = ev.payload["primary_path"].as_str().and_then(|p| p.rsplit('/').next()).filter(|s| !s.is_empty()).unwrap_or("paper.tex").to_string();
            let origin = ev.payload["origin"].as_str().filter(|s| !s.trim().is_empty()).unwrap_or("imported:local").to_string();
            let title = ev.payload["title"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
            return Ok(Some(LocalSourceCandidate::RawBytes { file_name, origin, title, raw_bytes }));
        }
    }
    Ok(None)
}

fn build_skeleton_from_receipt(
    sandbox: &WorkspaceSandbox, session_id: &str, receipt: &Event, paper_key: PaperKey, version: Option<String>,
) -> Result<PaperSkeleton, String> {
    if is_tex_receipt(receipt) {
        Ok(build_tex_skeleton_from_ingested(sandbox, session_id, receipt, paper_key, version)?.0.skeleton)
    } else if is_html_receipt(receipt) {
        Ok(build_html_skeleton_from_ingested(sandbox, session_id, receipt, paper_key, version)?.0.skeleton)
    } else if is_pdf_receipt(receipt) {
        Ok(build_pdf_skeleton_from_ingested(sandbox, session_id, receipt, paper_key, version)?.0.skeleton)
    } else {
        Err("local source is neither TeX, LaTeXML HTML, nor a page-mapped PDF".to_string())
    }
}

fn map_anchor_path(
    anchor: &ExportAnchor,
    local_source_id: &str,
    fallback_path: Option<&str>,
    receipt_files: &BTreeSet<&str>,
) -> Option<String> {
    if let Some((_, rest)) = anchor.path.split_once('/') {
        if !rest.is_empty() {
            let candidate = format!("{local_source_id}/{rest}");
            if receipt_files.is_empty() || receipt_files.contains(candidate.as_str()) {
                return Some(candidate);
            }
        }
    }
    (receipt_files.len() <= 1)
        .then_some(fallback_path)
        .flatten()
        .map(str::to_string)
}

fn to_verified_anchor(vq: VerifiedQuote, raw_quote: &str) -> VerifiedMapAnchor {
    VerifiedMapAnchor {
        source_id: vq.quote_ref.source_id, path: vq.quote_ref.path,
        start_line: vq.quote_ref.start_line, end_line: vq.quote_ref.end_line,
        file_sha256: vq.quote_ref.file_sha256, quote_sha256: vq.quote_ref.quote_sha256,
        quote: clip_quote(raw_quote.trim()).to_string(), excerpt: vq.excerpt, source_seq: vq.source_seq,
        cited_range: vq.cited_range, range_basis: vq.range_basis, excerpt_window: vq.excerpt_window,
    }
}

fn span_contains(spans: &[Span], anchor: &VerifiedMapAnchor) -> bool {
    spans.iter().any(|s| s.path == anchor.path && s.start_line <= anchor.start_line && anchor.end_line <= s.end_line)
}

struct ImportVerifyCtx<'a> {
    sandbox: &'a WorkspaceSandbox,
    session_id: &'a str,
    events: &'a [Event],
    local_source_id: &'a str,
    fallback_path: Option<&'a str>,
    receipt_files: BTreeSet<&'a str>,
    nodes_by_tag: BTreeMap<&'a str, &'a SkNode>,
    proofs_for_target: BTreeMap<&'a str, Vec<&'a SkNode>>,
    bib_keys: BTreeSet<&'a str>,
}

impl<'a> ImportVerifyCtx<'a> {
    fn check_anchor(&self, root_node: Option<&SkNode>, anchor: &ExportAnchor) -> Result<VerifiedMapAnchor, String> {
        let mapped_path = map_anchor_path(anchor, self.local_source_id, self.fallback_path, &self.receipt_files)
            .ok_or_else(|| "anchor is missing a valid source path".to_string())?;
        let vq = verify_quote(
            self.sandbox, self.session_id, self.events, self.local_source_id,
            &mapped_path, anchor.start_line, anchor.end_line, &anchor.quote,
        ).map_err(|f| format!("quote check failed at `{mapped_path}` lines {}-{}: {}", anchor.start_line, anchor.end_line, f.message))?;
        let va = to_verified_anchor(vq, &anchor.quote);
        if let Some(root) = root_node {
            let root_tag = root.tag.as_deref().unwrap_or("");
            let in_root = span_contains(&root.spans, &va)
                || self.proofs_for_target.get(root_tag).is_some_and(|ps| ps.iter().any(|p| span_contains(&p.spans, &va)));
            if !in_root { return Err(format!("quote `{}` falls outside node `{root_tag}` span", anchor.path)); }
        }
        Ok(va)
    }

    fn verify_hyps_and_anchors(
        &self, root: &SkNode, raw_hyps: &[ExportHypothesis], raw_anchors: &[ExportAnchor], tag_label: &str,
    ) -> Result<(Vec<HypothesisRecord>, Vec<VerifiedMapAnchor>), String> {
        let mut hyps = Vec::with_capacity(raw_hyps.len());
        for h in raw_hyps {
            let text = h.text.trim();
            if text.is_empty() { return Err(format!("empty hypothesis on node `{tag_label}`")); }
            hyps.push(HypothesisRecord { text: text.to_string(), anchor: self.check_anchor(Some(root), &h.anchor)? });
        }
        let mut anchors = Vec::with_capacity(raw_anchors.len());
        for a in raw_anchors {
            anchors.push(self.check_anchor(Some(root), a)?);
        }
        Ok((hyps, anchors))
    }

    fn verify_claim_body(&self, root: &SkNode, body: &ExportClaimBody, tag_label: &str) -> Result<AuthorsClaimRecord, String> {
        for k in &body.attributed_to {
            if !self.bib_keys.contains(k.as_str()) {
                return Err(format!("bibliography key `{k}` missing on local skeleton for `{tag_label}`"));
            }
        }
        let mut anchors = Vec::with_capacity(body.anchors.len());
        for a in &body.anchors { anchors.push(self.check_anchor(Some(root), a)?); }
        if matches!(body.label, AuthorsClaimLabel::New | AuthorsClaimLabel::Restated | AuthorsClaimLabel::Background) && anchors.is_empty() {
            return Err(format!("authors_claim `{:?}` on `{tag_label}` requires a matched quote anchor", body.label));
        }
        let cues = detect_cues_from_anchors(&anchors);
        Ok(AuthorsClaimRecord {
            label: body.label, relation_note: body.relation_note.clone(),
            attributed_to: body.attributed_to.clone(), anchors, cues,
        })
    }
}

fn verify_gist_item(
    ctx: &ImportVerifyCtx<'_>, g: &ExportNodeReading,
) -> Result<(ReadingItemRecord, Option<research_tools::map_fold::ReaderPin>), String> {
    let sk = ctx.nodes_by_tag.get(g.tag.as_str()).copied().ok_or_else(|| format!("unknown node `{}`", g.tag))?;
    if sk.fp != g.node_fp { return Err(format!("node `{}` fingerprint mismatch on local source", g.tag)); }
    if let Some(role) = g.role {
        if !role.allowed_for_kind(sk.kind) { return Err(format!("role `{role:?}` not allowed on node `{}`", g.tag)); }
    }
    let (hypotheses, anchors) = ctx.verify_hyps_and_anchors(sk, &g.hypotheses, &g.anchors, &g.tag)?;
    Ok((ReadingItemRecord {
        tag: g.tag.clone(), node_fp: sk.fp.clone(), attach_to: None, model_kind: None,
        role: g.role, gist: g.gist.clone(), purpose: g.purpose.clone(), hypotheses, anchors, authors_claim: None,
    }, g.pin))
}

fn verify_model_node_item<'a>(
    ctx: &ImportVerifyCtx<'a>, m: &ExportModelNode, model_roots: &BTreeMap<String, (&'a SkNode, String)>,
) -> Result<(ReadingItemRecord, &'a SkNode), String> {
    let norm_kind = normalize_model_kind(&m.model_kind)?;
    if let Some(role) = m.role {
        if !role_allowed_for_model_kind(role, &norm_kind) {
            return Err(format!("role `{role:?}` invalid for model kind `{norm_kind}`"));
        }
    }
    let root = ctx.nodes_by_tag.get(m.attach_to.as_str()).copied()
        .or_else(|| model_roots.get(&m.attach_to).map(|(sk, _)| *sk))
        .ok_or_else(|| format!("unknown attach_to parent `{}` for model node `{}`", m.attach_to, m.tag))?;
    let (hypotheses, anchors) = ctx.verify_hyps_and_anchors(root, &m.hypotheses, &m.anchors, &m.tag)?;
    if anchors.is_empty() { return Err(format!("model node `{}` requires at least one matched anchor", m.tag)); }
    let derived_tag = model_node_tag(&m.attach_to, &norm_kind, &anchors[0].quote)?;
    let authors_claim = m.authors_claim.as_ref().map(|ac| ctx.verify_claim_body(root, ac, &derived_tag)).transpose()?;
    Ok((ReadingItemRecord {
        tag: derived_tag, node_fp: root.fp.clone(), attach_to: Some(m.attach_to.clone()),
        model_kind: Some(norm_kind), role: m.role, gist: m.gist.clone(), purpose: m.purpose.clone(),
        hypotheses, anchors, authors_claim,
    }, root))
}

fn resolve_or_ingest_local_source(
    data_dir: &Path, sandbox: &WorkspaceSandbox, session_id: &str, events: &[Event], map: &PaperMapExport,
) -> Result<Option<String>, PaperMapError> {
    let ledger = Ledger::open_existing(data_dir).map_err(|e| PaperMapError::internal(format!("failed to open ledger: {e}")))?;
    let Some(candidate) = find_local_candidate(data_dir, &ledger, events, map)? else { return Ok(None); };
    drop(ledger);
    match candidate {
        LocalSourceCandidate::ExistingReceipt { source_id } => Ok(Some(source_id)),
        LocalSourceCandidate::RawBytes { file_name, origin, title, raw_bytes } => {
            let mutex = Mutex::new(Ledger::open_existing(data_dir).map_err(|e| PaperMapError::internal(e.to_string()))?);
            let ing = ingest_bytes(sandbox, &mutex, &IngestRequest {
                session_id, file_name: &file_name, origin: &origin, raw_bytes: &raw_bytes,
                source_id_hint: None, section_filter: None, title: title.as_deref(),
            }).map_err(PaperMapError::internal)?;
            Ok(Some(ing.source_id))
        }
    }
}

struct RebuiltSkeleton {
    seq: u64,
    blob: String,
    skeleton: PaperSkeleton,
    events: Vec<Event>,
    fallback_path: Option<String>,
}

fn rebuild_and_persist_skeleton(
    ledger: &Ledger, sandbox: &WorkspaceSandbox, session_id: &str, local_source_id: &str, map: &PaperMapExport,
) -> Result<RebuiltSkeleton, PaperMapError> {
    let events = ledger.read_session(session_id).map_err(|e| PaperMapError::internal(e.to_string()))?;
    let receipt = events.iter().rev()
        .find(|e| e.kind == EVENT_SOURCE_INGESTED && e.payload["source_id"].as_str() == Some(local_source_id))
        .ok_or_else(|| PaperMapError::internal("ingested source receipt missing after import"))?
        .clone();
    if let Some(pinned) = events.iter().rev().find(|e| e.kind == EVENT_INVESTIGATION_PAPER) {
        if pinned.payload["source_id"].as_str() != Some(local_source_id) {
            return Err(PaperMapError::bad("Start a new Investigation before importing a different paper."));
        }
    } else {
        ledger.append(session_id, EVENT_INVESTIGATION_PAPER, 1, &json!({ "source_id": local_source_id, "question": map.question }))
            .map_err(|e| PaperMapError::internal(e.to_string()))?;
    }
    let paper_key = PaperKey::parse(&map.paper_key).map_err(PaperMapError::bad)?;
    let skeleton = build_skeleton_from_receipt(sandbox, session_id, &receipt, paper_key, map.version.clone())
        .map_err(PaperMapError::internal)?;
    let skel_bytes = serde_json::to_vec(&skeleton).map_err(|e| PaperMapError::internal(e.to_string()))?;
    let skel_blob = ledger.put_blob(&skel_bytes).map_err(|e| PaperMapError::internal(e.to_string()))?;
    let skel_payload = json!({
        "source_id": skeleton.source_id, "source_seq": skeleton.source_seq, "builder": skeleton.builder,
        "files_root_sha256": skeleton.files_root_sha256, "paper_key": skeleton.paper_key,
        "version": skeleton.version, "quality": skeleton.quality, "quality_reason": skeleton.quality_reason,
        "counts": {
            "nodes": skeleton.nodes.len(), "edges": skeleton.edges.len(), "bib": skeleton.bib.len(),
            "sections": skeleton.nodes.iter().filter(|n| n.kind == SkNodeKind::Section).count(),
            "results": skeleton.nodes.iter().filter(|n| n.kind.is_theorem_like()).count(),
            "equations": skeleton.nodes.iter().filter(|n| n.kind == SkNodeKind::Equation).count(),
        },
        "blob": skel_blob, "raw_blob": skel_blob, "prev_skeleton_seq": Value::Null, "carry_counts": Value::Null, "imported": true,
    });
    let skel_seq = ledger.append(session_id, EVENT_INVESTIGATION_SKELETON, 1, &skel_payload)
        .map_err(|e| PaperMapError::internal(e.to_string()))?;
    let refreshed = ledger.read_session(session_id).map_err(|e| PaperMapError::internal(e.to_string()))?;
    let fallback_path = receipt.payload["primary_path"].as_str().map(str::to_string);
    Ok(RebuiltSkeleton { seq: skel_seq, blob: skel_blob, skeleton, events: refreshed, fallback_path })
}

fn verify_and_persist_layer2(
    ledger: &Ledger, sandbox: &WorkspaceSandbox, session_id: &str, local_source_id: &str,
    rebuilt: &RebuiltSkeleton, map: &PaperMapExport,
) -> Result<(usize, usize, Vec<String>), PaperMapError> {
    let skel_seq = rebuilt.seq;
    let skeleton = &rebuilt.skeleton;
    let events: &[Event] = &rebuilt.events;
    let fallback_path = rebuilt.fallback_path.as_deref();
    let mut nodes_by_tag: BTreeMap<&str, &SkNode> = BTreeMap::new();
    let mut proofs_for_target: BTreeMap<&str, Vec<&SkNode>> = BTreeMap::new();
    for node in &skeleton.nodes {
        if let Some(tag) = node.tag.as_deref() { nodes_by_tag.insert(tag, node); }
        if node.kind == SkNodeKind::Proof {
            if let Some(target) = node.proof_of.as_deref() { proofs_for_target.entry(target).or_default().push(node); }
        }
    }
    let bib_keys: BTreeSet<&str> = skeleton.bib.iter().map(|b| b.key.as_str()).collect();
    let receipt_files: BTreeSet<&str> = events.iter().rev()
        .find(|e| e.kind == EVENT_SOURCE_INGESTED && e.payload["source_id"].as_str() == Some(local_source_id))
        .and_then(|e| e.payload["files"].as_array())
        .map(|arr| arr.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let ctx = ImportVerifyCtx {
        sandbox, session_id, events, local_source_id, fallback_path, receipt_files,
        nodes_by_tag: nodes_by_tag.clone(), proofs_for_target, bib_keys,
    };

    let (mut matched, mut dropped, mut reasons) = (0usize, 0usize, Vec::new());
    let (mut reading_items, mut pins, mut item_by_tag) = (Vec::new(), Vec::new(), BTreeMap::new());
    for g in &map.gists {
        match verify_gist_item(&ctx, g) {
            Ok((rec, pin)) => {
                matched += 1;
                if let Some(p) = pin { pins.push((rec.tag.clone(), rec.node_fp.clone(), p)); }
                item_by_tag.insert(rec.tag.clone(), reading_items.len());
                reading_items.push(rec);
            }
            Err(r) => { dropped += 1; reasons.push(r); }
        }
    }
    for ac in &map.authors_claims {
        let Some(&sk) = nodes_by_tag.get(ac.tag.as_str()).filter(|sk| sk.fp == ac.node_fp) else {
            dropped += 1; reasons.push(format!("authors_claim target `{}` missing or changed", ac.tag)); continue;
        };
        let body = ExportClaimBody {
            label: ac.label, relation_note: ac.relation_note.clone(), attributed_to: ac.attributed_to.clone(),
            cues: ac.cues.clone(), anchors: ac.anchors.clone(),
        };
        match ctx.verify_claim_body(sk, &body, &ac.tag) {
            Ok(rec) => {
                matched += 1;
                if let Some(&idx) = item_by_tag.get(&ac.tag) { reading_items[idx].authors_claim = Some(rec); }
                else {
                    reading_items.push(ReadingItemRecord {
                        tag: ac.tag.clone(), node_fp: sk.fp.clone(), attach_to: None, model_kind: None,
                        role: None, gist: None, purpose: None, hypotheses: Vec::new(), anchors: Vec::new(), authors_claim: Some(rec),
                    });
                }
            }
            Err(r) => { dropped += 1; reasons.push(r); }
        }
    }
    let mut model_roots: BTreeMap<String, (&SkNode, String)> = BTreeMap::new();
    for m in &map.model_nodes {
        match verify_model_node_item(&ctx, m, &model_roots) {
            Ok((rec, root)) => {
                matched += 1;
                model_roots.insert(m.tag.clone(), (root, rec.node_fp.clone()));
                model_roots.insert(rec.tag.clone(), (root, rec.node_fp.clone()));
                reading_items.push(rec);
            }
            Err(r) => { dropped += 1; reasons.push(r); }
        }
    }
    let surviving_tags: BTreeSet<String> = nodes_by_tag.keys().map(|s| s.to_string()).chain(model_roots.keys().cloned()).collect();
    let mut surviving_edges = Vec::new();
    for e in &map.model_edges {
        if !surviving_tags.contains(&e.from) || !surviving_tags.contains(&e.to) {
            dropped += 1; reasons.push(format!("edge `{} -> {}` references missing node", e.from, e.to)); continue;
        }
        match ctx.check_anchor(None, &e.anchor) {
            Ok(anchor) => {
                matched += 1;
                surviving_edges.push(ModelEdgeRecord {
                    from: e.from.clone(), to: e.to.clone(), role: e.role, basis: e.basis, reason: e.reason.clone(), anchor,
                });
            }
            Err(r) => { dropped += 1; reasons.push(r); }
        }
    }

    if !reading_items.is_empty() || !surviving_edges.is_empty() || !map.limitations.is_empty() {
        let mut payload = serde_json::to_value(MapReadingEventPayload {
            schema_version: 1, request_seq: skel_seq, source_id: local_source_id.to_string(),
            skeleton_seq: skel_seq, done: true, items: reading_items, edges: surviving_edges,
            limitations: map.limitations.clone(), rejected: ReadingRejected::default(),
            anchor_report: json!({ "dropped": [], "corrected": [], "imported": true }),
        }).map_err(|e| PaperMapError::internal(e.to_string()))?;
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("imported".into(), json!(true));
            obj.insert("origin_tag".into(), json!("imported"));
            for key in ["items", "edges"] {
                if let Some(arr) = obj.get_mut(key).and_then(Value::as_array_mut) {
                    for entry in arr {
                        if let Some(m) = entry.as_object_mut() { m.insert("imported".into(), json!(true)); }
                    }
                }
            }
        }
        ledger.append(session_id, EVENT_INVESTIGATION_READING, 1, &payload).map_err(|e| PaperMapError::internal(e.to_string()))?;
    }
    for (tag, node_fp, pin) in pins {
        ledger.append(session_id, EVENT_INVESTIGATION_NOTE, 1, &json!({
            "source_id": local_source_id, "skeleton_seq": skel_seq, "tag": tag, "node_fp": node_fp, "pin": pin, "imported": true,
        })).map_err(|e| PaperMapError::internal(e.to_string()))?;
    }
    if let Some(notes) = map.notes.as_ref() {
        for note in notes {
            let valid_fp = nodes_by_tag.get(note.tag.as_str()).map(|sk| sk.fp.as_str())
                .or_else(|| model_roots.get(&note.tag).map(|(_, fp)| fp.as_str()));
            if valid_fp != Some(note.node_fp.as_str()) {
                dropped += 1; reasons.push(format!("note target `{}` missing or fingerprint mismatch", note.tag)); continue;
            }
            matched += 1;
            ledger.append(session_id, EVENT_INVESTIGATION_NOTE, 1, &json!({
                "source_id": local_source_id, "skeleton_seq": skel_seq, "tag": note.tag, "node_fp": note.node_fp, "text": note.text, "imported": true,
            })).map_err(|e| PaperMapError::internal(e.to_string()))?;
        }
    }
    Ok((matched, dropped, reasons))
}

/// Perform a Rule 6.11 fail-closed `.rsmap` import into `session_id` without any network calls.
pub fn import_paper_map(data_dir: &Path, session_id: &str, rsmap_bytes: &[u8]) -> Result<ImportPaperMapReport, PaperMapError> {
    validate_session_id(session_id).map_err(|e| PaperMapError::bad(e.to_string()))?;
    let sha256 = sha256_hex(rsmap_bytes);
    let map = validate_export_shape(rsmap_bytes)?;
    let events = Ledger::open_existing(data_dir)
        .map_err(|e| PaperMapError::internal(format!("failed to open ledger: {e}")))?
        .read_session(session_id)
        .map_err(|e| PaperMapError::not_found(format!("session `{session_id}` not found: {e}")))?;
    if session_mode_from_events(&events) != Mode::Investigate {
        return Err(PaperMapError::bad("Import requires an active Investigate session."));
    }

    let sandbox = WorkspaceSandbox::new(data_dir.to_path_buf(), None);
    let Some(local_source_id) = resolve_or_ingest_local_source(data_dir, &sandbox, session_id, &events, &map)? else {
        let total = map.gists.len() + map.authors_claims.len() + map.model_nodes.len()
            + map.model_edges.len() + map.notes.as_ref().map_or(0, Vec::len);
        return Ok(ImportPaperMapReport {
            sha256, paper_key: map.paper_key.clone(), version: map.version.clone(),
            source_match: false, read_only: true, source_id: None, skeleton_seq: None, skeleton_blob: None,
            items_matched: 0, items_dropped: total, banner: READ_ONLY_QUOTES_BANNER.to_string(),
            dropped_reasons: vec![READ_ONLY_QUOTES_BANNER.to_string()], imported_map: map,
        });
    };

    let ledger = Ledger::open_existing(data_dir).map_err(|e| PaperMapError::internal(e.to_string()))?;
    let rebuilt = rebuild_and_persist_skeleton(&ledger, &sandbox, session_id, &local_source_id, &map)?;
    let (items_matched, items_dropped, dropped_reasons) = verify_and_persist_layer2(
        &ledger, &sandbox, session_id, &local_source_id, &rebuilt, &map,
    )?;
    let banner = if items_dropped > 0 {
        format!("Imported, {items_dropped} quotes could not be matched")
    } else {
        format!("Imported ({items_matched} items matched)")
    };
    Ok(ImportPaperMapReport {
        sha256, paper_key: map.paper_key.clone(), version: map.version.clone(),
        source_match: true, read_only: false, source_id: Some(local_source_id),
        skeleton_seq: Some(rebuilt.seq), skeleton_blob: Some(rebuilt.blob),
        items_matched, items_dropped, banner, dropped_reasons, imported_map: map,
    })
}
