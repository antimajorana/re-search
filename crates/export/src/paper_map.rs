//! `.rsmap` (schema v1) Investigate paper map export builder (`investigate.md` §3 Stage L, §12, §13).
//!
//! Exports paper identity, licence metadata, skeleton summary + deterministic nodes/edges/bib/
//! symbols, Layer-2 gists, authors' claims, model nodes/edges, clipped quotes (`<= 300` chars),
//! empty `context_cards: []` (V12 ready), and optional statement bodies / private notes behind
//! explicit copyright and privacy gates.

use ledger::{sha256_hex, Event, Ledger};
use research_tools::investigation_markers::EVENT_INVESTIGATION_SKELETON;
use research_tools::library::Library;
use research_tools::map_defs::{
    AuthorsClaimLabel, ModelEdgeBasis, ModelEdgeRole, ReadingRole, VerifiedMapAnchor,
};
use research_tools::map_fold::{fold_map, FoldedMap, FoldedNodeReading, ReaderPin};
use research_tools::paper_key::Mode;
use research_tools::quote_core::clip_quote;
use research_tools::sandbox::{SandboxRoot, WorkspaceSandbox};
use research_tools::skeleton::{
    BibRef, NumberBasis, PaperSkeleton, SkConfidence, SkEdge, SkNode, SkNodeKind, SkeletonBuilder,
    SkeletonQuality, Span, SymbolDef,
};
use research_tools::tool_profile::session_mode_from_events;
use research_tools::validate_session_id;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::{fmt, fs, path::Path};

pub const RSMAP_SCHEMA_VERSION: u32 = 1;
pub const RSMAP_FORMAT: &str = "rsmap";
pub const MAX_EXPORT_QUOTE_CHARS: usize = 300;
pub const EVENT_INVESTIGATION_PAPER: &str = "investigation.paper";
const EVENT_SOURCE_INGESTED: &str = "source.ingested";

/// Options controlling copyrighted full statements and private reader notes in `.rsmap`/`.html`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PaperMapExportOptions {
    pub include_full_statements: bool,
    pub include_notes: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaperMapError {
    BadRequest(String),
    NotFound(String),
    Internal(String),
}

impl PaperMapError {
    pub fn bad(msg: impl Into<String>) -> Self { Self::BadRequest(msg.into()) }
    pub fn not_found(msg: impl Into<String>) -> Self { Self::NotFound(msg.into()) }
    pub fn internal(msg: impl Into<String>) -> Self { Self::Internal(msg.into()) }
}

impl fmt::Display for PaperMapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadRequest(m) | Self::NotFound(m) | Self::Internal(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for PaperMapError {}

/// Portable quote anchor capped at 300 characters (`clip_quote`), omitting surrounding excerpts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportAnchor {
    pub source_id: String,
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub quote: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cited_range: Option<[usize; 2]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub range_basis: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportHypothesis {
    pub text: String,
    pub anchor: ExportAnchor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportSkeletonSummary {
    pub quality: SkeletonQuality,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality_reason: Option<String>,
    pub node_count: usize,
    pub edge_count: usize,
    pub bib_count: usize,
    pub sections: usize,
    pub results: usize,
    pub equations: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportSkeletonNode {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    pub kind: SkNodeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number_basis: Option<NumberBasis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub order: usize,
    #[serde(default)]
    pub spans: Vec<Span>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof_of: Option<String>,
    #[serde(default)]
    pub head_cites: Vec<String>,
    #[serde(default)]
    pub intro_refs: bool,
    pub fp: String,
    #[serde(default)]
    pub words: u32,
    #[serde(default)]
    pub eq_count: u32,
    #[serde(default)]
    pub read_minutes: f64,
    pub confidence: SkConfidence,
    /// Full verbatim statement body, gated by open licence or explicit export opt-in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub statement: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportSkeleton {
    pub summary: ExportSkeletonSummary,
    #[serde(default)]
    pub nodes: Vec<ExportSkeletonNode>,
    #[serde(default)]
    pub edges: Vec<SkEdge>,
    #[serde(default)]
    pub bib: Vec<BibRef>,
    #[serde(default)]
    pub symbols: Vec<SymbolDef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportNodeReading {
    pub tag: String,
    pub node_fp: String,
    #[serde(default)]
    pub stale: bool,
    #[serde(default)]
    pub effective_is_main_result: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<ReadingRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gist: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<ReaderPin>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hypotheses: Vec<ExportHypothesis>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<ExportAnchor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportClaimBody {
    pub label: AuthorsClaimLabel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation_note: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributed_to: Vec<String>,
    #[serde(default)]
    pub cues: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<ExportAnchor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportAuthorsClaim {
    pub tag: String,
    pub node_fp: String,
    #[serde(default)]
    pub stale: bool,
    pub label: AuthorsClaimLabel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation_note: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributed_to: Vec<String>,
    #[serde(default)]
    pub cues: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<ExportAnchor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportModelNode {
    pub tag: String,
    pub attach_to: String,
    pub model_kind: String,
    pub node_fp: String,
    #[serde(default)]
    pub stale: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<ReadingRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gist: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hypotheses: Vec<ExportHypothesis>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<ExportAnchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authors_claim: Option<ExportClaimBody>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportModelEdge {
    pub from: String,
    pub to: String,
    pub role: ModelEdgeRole,
    pub basis: ModelEdgeBasis,
    #[serde(default)]
    pub stale: bool,
    pub reason: String,
    pub anchor: ExportAnchor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportNote {
    pub tag: String,
    pub node_fp: String,
    #[serde(default)]
    pub stale: bool,
    pub text: String,
}

/// Complete portable `.rsmap` schema v1 document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaperMapExport {
    pub schema_version: u32,
    pub format: String,
    pub paper_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    pub builder: SkeletonBuilder,
    pub source_id: String,
    pub files_root_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub licence: Option<String>,
    pub open_licence: bool,
    pub statements_included: bool,
    pub notes_included: bool,
    pub skeleton: ExportSkeleton,
    #[serde(default)]
    pub gists: Vec<ExportNodeReading>,
    #[serde(default)]
    pub authors_claims: Vec<ExportAuthorsClaim>,
    #[serde(default)]
    pub model_nodes: Vec<ExportModelNode>,
    #[serde(default)]
    pub model_edges: Vec<ExportModelEdge>,
    #[serde(default)]
    pub limitations: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<Vec<ExportNote>>,
    /// Reserved empty list for V12 context cards.
    #[serde(default)]
    pub context_cards: Vec<Value>,
}

/// Returns `true` only when `licence` clearly indicates CC-BY / CC0 / Public Domain rights.
pub fn is_open_licence(licence: Option<&str>) -> bool {
    let Some(raw) = licence.map(str::trim).filter(|s| !s.is_empty()) else { return false };
    let lower = raw.to_ascii_lowercase();
    if lower.contains("nonexclusive") || lower.contains("all-rights-reserved") || lower.contains("all rights reserved") {
        return false;
    }
    if lower.contains("cc0") || lower.contains("publicdomain/zero") || lower.contains("public-domain") || lower.contains("public domain") || lower == "pd" {
        return true;
    }
    let has_by = lower.contains("creativecommons.org/licenses/by/")
        || lower.contains("creativecommons.org/licenses/by-sa")
        || lower.starts_with("cc-by")
        || lower.starts_with("cc by")
        || lower.contains("cc-by-")
        || lower.contains("cc by ");
    has_by && !lower.contains("by-nd") && !lower.contains("by-nc")
}

pub(crate) fn clip_anchor(a: &VerifiedMapAnchor) -> ExportAnchor {
    ExportAnchor {
        source_id: a.source_id.clone(),
        path: a.path.clone(),
        start_line: a.start_line,
        end_line: a.end_line,
        quote: clip_quote(a.quote.trim()).to_string(),
        cited_range: a.cited_range,
        range_basis: a.range_basis.clone(),
    }
}

fn non_empty(v: Option<&str>) -> Option<String> {
    v.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn find_receipt<'a>(events: &'a [Event], source_id: &str, source_seq: u64) -> Option<&'a Event> {
    events.iter().rev().find(|e| {
        e.kind == EVENT_SOURCE_INGESTED && e.seq == source_seq && e.payload["source_id"].as_str() == Some(source_id)
    }).or_else(|| {
        events.iter().rev().find(|e| e.kind == EVENT_SOURCE_INGESTED && e.payload["source_id"].as_str() == Some(source_id))
    })
}

fn load_verified_source_files(sandbox: &WorkspaceSandbox, session_id: &str, receipt: &Event) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    let Some(hashes) = receipt.payload["files_sha256"].as_object() else { return out };
    for (rel_path, expected_val) in hashes {
        let Some(expected) = expected_val.as_str() else { continue };
        let Ok(resolved) = sandbox.resolve_existing(SandboxRoot::Sources, session_id, rel_path, false) else { continue };
        let Ok(bytes) = fs::read(&resolved.canonical_target) else { continue };
        if sha256_hex(&bytes) != expected { continue; }
        if let Ok(text) = String::from_utf8(bytes) {
            out.insert(rel_path.clone(), text.lines().map(str::to_string).collect());
        }
    }
    out
}

fn extract_node_statement(node: &SkNode, files: &BTreeMap<String, Vec<String>>) -> Option<String> {
    if !(node.kind.is_theorem_like() || node.kind == SkNodeKind::Equation) || node.spans.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    for span in &node.spans {
        let lines = files.get(&span.path)?;
        if span.start_line == 0 || span.end_line < span.start_line || span.end_line > lines.len() {
            return None;
        }
        parts.extend(lines[span.start_line - 1..span.end_line].iter().cloned());
    }
    let joined = parts.join("\n").trim().to_string();
    (!joined.is_empty()).then_some(joined)
}

fn resolve_node_fp(node: &FoldedNodeReading, skeleton_fps: &BTreeMap<String, String>) -> String {
    if let Some(fp) = skeleton_fps.get(&node.tag) {
        return fp.clone();
    }
    node.anchors.current.as_ref().map(|v| v.node_fp.as_str())
        .or_else(|| node.gist.current.as_ref().map(|v| v.node_fp.as_str()))
        .or_else(|| node.purpose.current.as_ref().map(|v| v.node_fp.as_str()))
        .or_else(|| node.role.current.as_ref().map(|v| v.node_fp.as_str()))
        .or_else(|| node.authors_claim.current.as_ref().map(|v| v.node_fp.as_str()))
        .unwrap_or_default().to_string()
}

fn build_export_skeleton(skeleton: &PaperSkeleton, source_files: Option<&BTreeMap<String, Vec<String>>>) -> ExportSkeleton {
    let nodes = skeleton.nodes.iter().map(|n| ExportSkeletonNode {
        tag: n.tag.clone(), kind: n.kind, number: n.number.clone(), number_basis: n.number_basis,
        env_name: n.env_name.clone(), title: n.title.clone(), label: n.label.clone(),
        parent: n.parent.clone(), order: n.order, spans: n.spans.clone(), proof_of: n.proof_of.clone(),
        head_cites: n.head_cites.clone(), intro_refs: n.intro_refs, fp: n.fp.clone(), words: n.words,
        eq_count: n.eq_count, read_minutes: n.read_minutes, confidence: n.confidence,
        statement: source_files.and_then(|f| extract_node_statement(n, f)),
    }).collect();
    let summary = ExportSkeletonSummary {
        quality: skeleton.quality, quality_reason: skeleton.quality_reason.clone(),
        node_count: skeleton.nodes.len(), edge_count: skeleton.edges.len(), bib_count: skeleton.bib.len(),
        sections: skeleton.nodes.iter().filter(|n| n.kind == SkNodeKind::Section).count(),
        results: skeleton.nodes.iter().filter(|n| n.kind.is_theorem_like()).count(),
        equations: skeleton.nodes.iter().filter(|n| n.kind == SkNodeKind::Equation).count(),
    };
    ExportSkeleton { summary, nodes, edges: skeleton.edges.clone(), bib: skeleton.bib.clone(), symbols: skeleton.symbols.clone() }
}

struct FoldedExportSections {
    gists: Vec<ExportNodeReading>,
    authors_claims: Vec<ExportAuthorsClaim>,
    model_nodes: Vec<ExportModelNode>,
    model_edges: Vec<ExportModelEdge>,
    notes: Option<Vec<ExportNote>>,
}

fn build_folded_sections(skeleton: &PaperSkeleton, folded: &FoldedMap, include_notes: bool) -> FoldedExportSections {
    let skeleton_fps: BTreeMap<String, String> = skeleton.nodes.iter().filter_map(|n| Some((n.tag.clone()?, n.fp.clone()))).collect();
    let valid_tags: BTreeSet<String> = skeleton_fps.keys().cloned().chain(folded.nodes.keys().cloned()).collect();
    let (mut gists, mut authors_claims, mut model_nodes, mut notes) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());

    for node in folded.nodes.values() {
        let node_fp = resolve_node_fp(node, &skeleton_fps);
        let hypotheses: Vec<ExportHypothesis> = node.hypotheses.current.as_ref().map(|v| {
            v.value.iter().map(|h| ExportHypothesis { text: h.text.clone(), anchor: clip_anchor(&h.anchor) }).collect()
        }).unwrap_or_default();
        let anchors: Vec<ExportAnchor> = node.anchors.current.as_ref().map(|v| v.value.iter().map(clip_anchor).collect()).unwrap_or_default();
        let role = node.role.current.as_ref().map(|v| v.value);
        let gist = node.gist.current.as_ref().map(|v| v.value.clone());
        let purpose = node.purpose.current.as_ref().map(|v| v.value.clone());
        let claim = node.authors_claim.current.as_ref().map(|v| &v.value);

        if node.is_model_node {
            if let (Some(attach_to), Some(model_kind)) = (node.attach_to.as_ref(), node.model_kind.as_ref()) {
                model_nodes.push(ExportModelNode {
                    tag: node.tag.clone(), attach_to: attach_to.clone(), model_kind: model_kind.clone(),
                    node_fp: node_fp.clone(), stale: node.any_stale, role, gist, purpose, hypotheses, anchors,
                    authors_claim: claim.map(|c| ExportClaimBody {
                        label: c.label, relation_note: c.relation_note.clone(), attributed_to: c.attributed_to.clone(),
                        cues: c.cues.clone(), anchors: c.anchors.iter().map(clip_anchor).collect(),
                    }),
                });
            }
        } else {
            if role.is_some() || gist.is_some() || purpose.is_some() || node.pin.is_some() || !hypotheses.is_empty() || !anchors.is_empty() {
                gists.push(ExportNodeReading {
                    tag: node.tag.clone(), node_fp: node_fp.clone(), stale: node.any_stale,
                    effective_is_main_result: node.effective_is_main_result, role, gist, purpose,
                    pin: node.pin, hypotheses, anchors,
                });
            }
            if let Some(c) = claim {
                authors_claims.push(ExportAuthorsClaim {
                    tag: node.tag.clone(), node_fp: node_fp.clone(),
                    stale: node.authors_claim.current.as_ref().is_some_and(|v| v.stale),
                    label: c.label, relation_note: c.relation_note.clone(), attributed_to: c.attributed_to.clone(),
                    cues: c.cues.clone(), anchors: c.anchors.iter().map(clip_anchor).collect(),
                });
            }
        }
        if include_notes {
            for note in &node.notes {
                notes.push(ExportNote { tag: note.tag.clone(), node_fp: note.node_fp.clone(), stale: note.stale, text: note.text.clone() });
            }
        }
    }

    let model_edges = folded.edges.iter().filter(|e| valid_tags.contains(&e.from) && valid_tags.contains(&e.to)).map(|e| {
        ExportModelEdge {
            from: e.from.clone(), to: e.to.clone(), role: e.role, basis: e.current.value.basis,
            stale: e.current.stale, reason: e.current.value.reason.clone(), anchor: clip_anchor(&e.current.value.anchor),
        }
    }).collect();
    FoldedExportSections {
        gists,
        authors_claims,
        model_nodes,
        model_edges,
        notes: include_notes.then_some(notes),
    }
}

/// Build a complete `.rsmap` schema v1 document from a persisted Investigation session.
pub fn build_paper_map_export(data_dir: &Path, session_id: &str, opts: PaperMapExportOptions) -> Result<PaperMapExport, PaperMapError> {
    validate_session_id(session_id).map_err(|e| PaperMapError::bad(e.to_string()))?;
    let ledger = Ledger::open_existing(data_dir).map_err(|e| PaperMapError::internal(e.to_string()))?;
    let events = ledger.read_session(session_id).map_err(|e| PaperMapError::not_found(format!("session `{session_id}` not found: {e}")))?;
    if session_mode_from_events(&events) != Mode::Investigate {
        return Err(PaperMapError::bad("Export is only supported for Investigate sessions."));
    }

    let paper_ev = events.iter().rev().find(|e| e.kind == EVENT_INVESTIGATION_PAPER);
    let pinned_source = paper_ev.and_then(|e| e.payload["source_id"].as_str());
    let skel_ev = events.iter().rev()
        .find(|e| e.kind == EVENT_INVESTIGATION_SKELETON && pinned_source.is_none_or(|sid| e.payload["source_id"].as_str() == Some(sid)))
        .ok_or_else(|| PaperMapError::not_found("No paper skeleton has been built for this session."))?;

    let blob_hash = skel_ev.payload["blob"].as_str().filter(|s| !s.is_empty())
        .ok_or_else(|| PaperMapError::internal("stored investigation.skeleton missing `blob` hash"))?;
    let skel_bytes = ledger.get_blob(blob_hash).map_err(|e| PaperMapError::internal(format!("failed to read skeleton blob: {e}")))?;
    if sha256_hex(&skel_bytes) != blob_hash {
        return Err(PaperMapError::internal("skeleton blob hash mismatch"));
    }
    let skeleton: PaperSkeleton = serde_json::from_slice(&skel_bytes).map_err(|e| PaperMapError::internal(format!("invalid skeleton blob: {e}")))?;

    let receipt = find_receipt(&events, &skeleton.source_id, skeleton.source_seq);
    let raw_sha256 = receipt.and_then(|r| non_empty(r.payload["raw_blob"].as_str()));
    let receipt_title = receipt.and_then(|r| non_empty(r.payload["title"].as_str()));
    let receipt_licence = receipt.and_then(|r| non_empty(r.payload["license"].as_str()));

    let lib_detail = Library::open(data_dir).ok().and_then(|lib| lib.get_paper(&skeleton.paper_key).ok().flatten());
    let lib_licence = lib_detail.as_ref().and_then(|d| non_empty(d.paper.flags.license.as_deref()));
    let lib_title = lib_detail.as_ref().and_then(|d| non_empty(Some(d.paper.title.as_str())));
    let licence = lib_licence.or(receipt_licence);
    let open_licence = is_open_licence(licence.as_deref());
    let statements_included = open_licence || opts.include_full_statements;
    let source_files = if statements_included {
        receipt.map(|r| load_verified_source_files(&WorkspaceSandbox::new(data_dir.to_path_buf(), None), session_id, r))
    } else { None };

    let folded = fold_map(&skeleton, &events, false);
    let sections = build_folded_sections(&skeleton, &folded, opts.include_notes);
    Ok(PaperMapExport {
        schema_version: RSMAP_SCHEMA_VERSION, format: RSMAP_FORMAT.to_string(),
        paper_key: skeleton.paper_key.to_string(), version: skeleton.version.clone(),
        title: receipt_title.or(lib_title), question: paper_ev.and_then(|e| non_empty(e.payload["question"].as_str())),
        builder: skeleton.builder, source_id: skeleton.source_id.clone(),
        files_root_sha256: skeleton.files_root_sha256.clone(), raw_sha256, licence,
        open_licence, statements_included, notes_included: opts.include_notes,
        skeleton: build_export_skeleton(&skeleton, source_files.as_ref()),
        gists: sections.gists, authors_claims: sections.authors_claims, model_nodes: sections.model_nodes,
        model_edges: sections.model_edges, limitations: folded.limitations,
        notes: sections.notes, context_cards: Vec::new(),
    })
}

/// Serialize a [`PaperMapExport`] into pretty UTF-8 JSON bytes with a trailing newline.
pub fn serialize_paper_map_json(export: &PaperMapExport) -> Result<Vec<u8>, PaperMapError> {
    let mut bytes = serde_json::to_vec_pretty(export).map_err(|e| PaperMapError::internal(format!("failed to serialize .rsmap: {e}")))?;
    bytes.push(b'\n');
    Ok(bytes)
}
