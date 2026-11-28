//! Explore conceptual map, overlap recorder (`record_idea_map`), and F6 `GraphView`
//! projection (package X11).
//!
//! Governance: `explore.md` 4.1 (`MapNode`, `MapEdge` direction table, `Overlap`), 4.2 (`idea.map`),
//! 5.1 (`record_idea_map`), 6 (Rules C4, C5, C6, S4, V8), `00-overview.md` 2.6 (`GraphView`) & 2.10.

use crate::idea_map_validate::{validate_and_build_map, MapValidationInput};
use crate::paper_key::PaperKey;
use crate::prior_work::OverlapRelation;
use crate::prior_work_wording::check_model_prose_phrases;
use ledger::Event;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Recorder tool name offered in the Map exchange (`explore.md` 5.1).
pub const RECORDER_IDEA_MAP: &str = "record_idea_map";
/// Ledger event kind for recorded Explore maps (`explore.md` 4.2).
pub const EVENT_IDEA_MAP: &str = "idea.map";
/// Ledger event kind for citation network edges (`explore.md` 4.2).
pub const EVENT_IDEA_CITATION_NET: &str = "idea.citation_net";
/// Maximum label characters for a `MapNode` (`explore.md` 4.1).
pub const MAX_NODE_LABEL_CHARS: usize = 120;
/// Maximum statement characters for a `MapNode` (`explore.md` 4.1).
pub const MAX_NODE_STATEMENT_CHARS: usize = 1_400;
/// Maximum reason characters for a `model_inferred` `MapEdge` (`explore.md` 4.1).
pub const MAX_EDGE_REASON_CHARS: usize = 400;
/// Maximum difference characters for an `Overlap` (`explore.md` 4.1).
pub const MAX_OVERLAP_DIFF_CHARS: usize = 600;
/// Maximum nodes in a single map call.
pub const MAX_MAP_NODES: usize = 64;
/// Maximum edges in a single map call.
pub const MAX_MAP_EDGES: usize = 160;
/// Maximum overlaps in a single map call.
pub const MAX_MAP_OVERLAPS: usize = 64;

/// Explore map node kinds (`explore.md` 4.1 table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapNodeKind {
    Concept,
    Definition,
    KnownResult,
    OpenQuestion,
    RelatedWork,
}

impl MapNodeKind {
    pub const fn id_prefix(self) -> &'static str {
        match self {
            Self::Concept => "c-",
            Self::Definition => "d-",
            Self::KnownResult => "k-",
            Self::OpenQuestion => "q-",
            Self::RelatedWork => "w-",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Concept => "concept",
            Self::Definition => "definition",
            Self::KnownResult => "known_result",
            Self::OpenQuestion => "open_question",
            Self::RelatedWork => "related_work",
        }
    }
}

/// Relation of a map node to the user's idea (`explore.md` 4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationToIdea {
    Same,
    SpecialCase,
    Generalisation,
    Ingredient,
    Obstruction,
    Analogue,
    Background,
}

/// Lifecycle status of a map node (`explore.md` 4.1, Rule S4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapNodeStatus {
    Active,
    NotInCurrentMap,
    Flagged,
}

/// Explore map edge kinds (`explore.md` 4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapEdgeKind {
    Defines,
    Uses,
    SpecialCaseOf,
    Generalises,
    Contradicts,
    Answers,
    LeavesOpen,
    SameAs,
    Cites,
}

impl MapEdgeKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Defines => "defines",
            Self::Uses => "uses",
            Self::SpecialCaseOf => "special_case_of",
            Self::Generalises => "generalises",
            Self::Contradicts => "contradicts",
            Self::Answers => "answers",
            Self::LeavesOpen => "leaves_open",
            Self::SameAs => "same_as",
            Self::Cites => "cites",
        }
    }

    /// Map `(from, to)` ("`from` `kind` `to`") to F6 `GraphView` `(src, dst, symmetric)` where
    /// `src` is the prerequisite/relied-on node drawn above and `dst` relies on it (`explore.md` 4.1).
    pub fn to_graph_direction<'a>(self, from: &'a str, to: &'a str) -> (&'a str, &'a str, bool) {
        match self {
            Self::Defines => (from, to, false),
            Self::Uses => (to, from, false),
            Self::SpecialCaseOf => (to, from, false),
            Self::Generalises => (from, to, false),
            Self::Answers => (to, from, false),
            Self::LeavesOpen => (from, to, false),
            Self::Cites => (to, from, false),
            Self::Contradicts | Self::SameAs => (from, to, true),
        }
    }
}

/// Provenance basis of a `MapEdge` (`explore.md` 4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapEdgeBasis {
    Evidence,
    Metadata,
    ModelInferred,
}

impl MapEdgeBasis {
    pub const fn graph_view_basis(self) -> &'static str {
        match self {
            Self::Evidence => "evidence",
            Self::Metadata => "code",
            Self::ModelInferred => "model",
        }
    }
}

/// Strict model input for one map node (Rule C4: NO `quote`/`excerpt`/`line`/`start_line`/`end_line`/`path`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapNodeInput {
    pub temp_id: String,
    pub kind: MapNodeKind,
    pub label: String,
    pub statement: String,
    #[serde(default)]
    pub evidence: Vec<String>,
    pub relation_to_idea: RelationToIdea,
    #[serde(default)]
    pub subclaim_ids: Vec<String>,
    /// Allowed ONLY for `concept` nodes: verbatim substring of active `IdeaStatement.text`.
    #[serde(default)]
    pub idea_quote: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub conventions: Option<String>,
    #[serde(default)]
    pub hypotheses: Vec<String>,
    #[serde(default)]
    pub regime: Option<String>,
    #[serde(default)]
    pub paper_key: Option<String>,
}

/// Strict model input for one map edge (`from` `kind` `to`, Rule C4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapEdgeInput {
    pub from: String,
    pub to: String,
    pub kind: MapEdgeKind,
    pub basis: MapEdgeBasis,
    #[serde(default)]
    pub evidence: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Structured `other_name` field on an overlap (`explore.md` 4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OtherNameInput {
    pub term: String,
}

/// Strict model input for one prior-work overlap (`explore.md` 4.1, Rule C4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlapInput {
    pub subclaim_id: String,
    pub paper_key: String,
    pub relation: OverlapRelation,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub difference: String,
    #[serde(default)]
    pub other_name: Option<OtherNameInput>,
}

/// Top-level input payload of `record_idea_map` (`#[serde(deny_unknown_fields)]`, Rule C4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordIdeaMapInput {
    pub nodes: Vec<MapNodeInput>,
    #[serde(default)]
    pub edges: Vec<MapEdgeInput>,
    #[serde(default)]
    pub overlaps: Vec<OverlapInput>,
}

/// App-validated, ID-assigned conceptual map node persisted in `idea.map`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapNode {
    pub id: String,
    pub kind: MapNodeKind,
    pub rev: u32,
    pub label: String,
    pub statement: String,
    pub evidence: Vec<String>,
    pub relation_to_idea: RelationToIdea,
    #[serde(default)]
    pub subclaim_ids: Vec<String>,
    #[serde(default)]
    pub papers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    pub status: MapNodeStatus,
    /// Stable identity token (`first_evidence_id` or `idea_quote:<sha256>`) used across re-maps (Rule S4).
    pub identity_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idea_quote: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conventions: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hypotheses: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stated_year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub later_work_checked: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basis: Option<String>,
}

/// App-validated edge persisted in `idea.map`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapEdge {
    pub from: String,
    pub to: String,
    pub kind: MapEdgeKind,
    pub basis: MapEdgeBasis,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// App-validated overlap persisted in `idea.map` (100% compatible with F12 `prior_work_events`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapOverlap {
    pub subclaim_id: String,
    pub paper_key: PaperKey,
    pub relation: OverlapRelation,
    pub evidence: Vec<String>,
    pub difference: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other_name: Option<OtherNameInput>,
}

/// Dropped node/edge/overlap record with explicit reason (`explore.md` 4.2, Rule C5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DroppedMapItem {
    pub item_type: String,
    pub reference: String,
    pub reason: String,
}

/// Persisted `idea.map` ledger payload (`explore.md` 4.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeaMapPayload {
    pub idea_rev: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_seq: Option<u64>,
    pub nodes: Vec<MapNode>,
    pub edges: Vec<MapEdge>,
    pub overlaps: Vec<MapOverlap>,
    #[serde(default)]
    pub dropped: Vec<DroppedMapItem>,
}

/// Rule V8 prose check wrapper over F12 [`check_model_prose_phrases`].
/// Rejects banned absolute-claim phrases while allowing phrases like `"first calculation"`.
pub fn check_explore_prose(text: &str) -> Result<(), Vec<String>> {
    check_model_prose_phrases(text).map_err(|v| v.into_iter().map(str::to_string).collect())
}

/// Strict JSON Schema offered as the sole tool in the Map exchange (`explore.md` 4.1, 5.1, Rule C4).
pub fn record_idea_map_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["nodes", "edges", "overlaps"],
        "properties": {
            "nodes": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_MAP_NODES,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["temp_id", "kind", "label", "statement", "evidence", "relation_to_idea"],
                    "properties": {
                        "temp_id": { "type": "string", "minLength": 1, "maxLength": 64 },
                        "kind": { "type": "string", "enum": ["concept", "definition", "known_result", "open_question", "related_work"] },
                        "label": { "type": "string", "minLength": 1, "maxLength": MAX_NODE_LABEL_CHARS },
                        "statement": { "type": "string", "minLength": 1, "maxLength": MAX_NODE_STATEMENT_CHARS },
                        "evidence": { "type": "array", "items": { "type": "string", "pattern": "^x[ea]-[1-9][0-9]*$" } },
                        "relation_to_idea": { "type": "string", "enum": ["same", "special_case", "generalisation", "ingredient", "obstruction", "analogue", "background"] },
                        "subclaim_ids": { "type": "array", "items": { "type": "string", "pattern": "^sc-[1-9][0-9]*$" } },
                        "idea_quote": { "type": ["string", "null"] },
                        "aliases": { "type": "array", "items": { "type": "string" } },
                        "symbol": { "type": ["string", "null"] },
                        "conventions": { "type": ["string", "null"] },
                        "hypotheses": { "type": "array", "items": { "type": "string" } },
                        "regime": { "type": ["string", "null"] },
                        "paper_key": { "type": ["string", "null"] }
                    }
                }
            },
            "edges": {
                "type": "array",
                "maxItems": MAX_MAP_EDGES,
                "description": "Every edge is read as the sentence `<from> <kind> <to>`: defines (`from` defines `to`), uses (`from` uses `to`), special_case_of (`from` is a special case of `to`), generalises (`from` generalises `to`), answers (result `from` answers open question `to`), leaves_open (result `from` leaves open question `to`), cites (paper `from` cites paper `to`), contradicts/same_as (symmetric).",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["from", "to", "kind", "basis"],
                    "properties": {
                        "from": { "type": "string", "minLength": 1, "maxLength": 64 },
                        "to": { "type": "string", "minLength": 1, "maxLength": 64 },
                        "kind": { "type": "string", "enum": ["defines", "uses", "special_case_of", "generalises", "contradicts", "answers", "leaves_open", "same_as", "cites"] },
                        "basis": { "type": "string", "enum": ["evidence", "model_inferred"] },
                        "evidence": { "type": ["string", "null"], "pattern": "^xe-[1-9][0-9]*$" },
                        "reason": { "type": ["string", "null"], "maxLength": MAX_EDGE_REASON_CHARS }
                    }
                }
            },
            "overlaps": {
                "type": "array",
                "maxItems": MAX_MAP_OVERLAPS,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["subclaim_id", "paper_key", "relation", "evidence", "difference"],
                    "properties": {
                        "subclaim_id": { "type": "string", "pattern": "^sc-[1-9][0-9]*$" },
                        "paper_key": { "type": "string", "minLength": 1 },
                        "relation": { "type": "string", "enum": ["same", "partial", "other_name"] },
                        "evidence": { "type": "array", "minItems": 1, "items": { "type": "string", "pattern": "^xe-[1-9][0-9]*$" } },
                        "difference": { "type": "string", "maxLength": MAX_OVERLAP_DIFF_CHARS },
                        "other_name": {
                            "type": ["object", "null"],
                            "additionalProperties": false,
                            "required": ["term"],
                            "properties": { "term": { "type": "string", "minLength": 1, "maxLength": 160 } }
                        }
                    }
                }
            }
        }
    })
}

/// Validate a raw `record_idea_map` JSON input strictly against this session's ledger `events`,
/// enforcing Rules C4, C5, C6, S4, and V8, assigning stable sequential IDs, preserving user-noted
/// nodes as `not_in_current_map`, and synthesizing `cites` edges from `idea.citation_net`.
pub fn validate_record_idea_map(
    events: &[Event],
    idea_rev: u32,
    idea_text: &str,
    stmt_seq: u64,
    valid_subclaim_ids: &[String],
    request_seq: Option<u64>,
    raw_input: &Value,
) -> Result<IdeaMapPayload, Vec<String>> {
    validate_and_build_map(
        MapValidationInput {
            events,
            idea_rev,
            idea_text,
            stmt_seq,
            valid_subclaim_ids,
            request_seq,
        },
        raw_input,
    )
}

/// Project an [`IdeaMapPayload`] into the shared F6 `GraphView` JSON shape (`00-overview.md` 2.6,
/// `explore.md` 4.1 edge direction table & solid/dashed line rule).
pub fn project_graph_view(map: &IdeaMapPayload, ocr_evidence_ids: &BTreeSet<String>) -> Value {
    let mut node_abstract_only: BTreeMap<&str, bool> = BTreeMap::new();
    let mut nodes_json = Vec::with_capacity(map.nodes.len());

    for n in &map.nodes {
        let has_xe = n.evidence.iter().any(|e| e.starts_with("xe-"));
        let has_xa = n.evidence.iter().any(|e| e.starts_with("xa-"));
        let has_ocr = n.evidence.iter().any(|e| ocr_evidence_ids.contains(e));
        let abstract_only = !has_xe && has_xa;
        let user_only = !has_xe && !has_xa && n.idea_quote.is_some();
        node_abstract_only.insert(n.id.as_str(), abstract_only);

        let (display_grade, style, badge, ocr_tag) = if has_xe && has_ocr {
            ("matched_quote_scanned_page", "solid", "Found (OCR)", Some("OCR"))
        } else if has_xe {
            ("matched_quote_full_text", "solid", "Found", None)
        } else if abstract_only {
            ("matched_quote_abstract", "dashed", "abstract only", None)
        } else if user_only {
            ("user_statement", "solid", "you", None)
        } else {
            ("model_statement", "dashed", "(model)", None)
        };

        nodes_json.push(json!({
            "id": n.id,
            "kind": n.kind.as_str(),
            "rev": n.rev,
            "label": n.label,
            "statement": n.statement,
            "statement_label": "summary (model)",
            "evidence": n.evidence,
            "relation_to_idea": n.relation_to_idea,
            "subclaim_ids": n.subclaim_ids,
            "papers": n.papers,
            "year": n.year,
            "status": n.status,
            "display_grade": display_grade,
            "style": style,
            "badge": badge,
            "ocr_tag": ocr_tag,
            "idea_quote": n.idea_quote,
            "aliases": n.aliases,
            "symbol": n.symbol,
            "conventions": n.conventions,
            "hypotheses": n.hypotheses,
            "regime": n.regime,
            "stated_year": n.stated_year,
            "later_work_checked": n.later_work_checked,
            "paper_key": n.paper_key,
            "basis": n.basis,
        }));
    }

    let mut edges_json = Vec::with_capacity(map.edges.len());
    for e in &map.edges {
        let (src, dst, symmetric) = e.kind.to_graph_direction(&e.from, &e.to);
        let endpoint_abstract = node_abstract_only.get(e.from.as_str()).copied().unwrap_or(false)
            || node_abstract_only.get(e.to.as_str()).copied().unwrap_or(false);
        let dashed = e.basis == MapEdgeBasis::ModelInferred || endpoint_abstract;
        edges_json.push(json!({
            "src": src,
            "dst": dst,
            "from": e.from,
            "to": e.to,
            "kind": e.kind.as_str(),
            "role": e.kind.as_str(),
            "basis": e.basis.graph_view_basis(),
            "symmetric": symmetric,
            "directed": !symmetric,
            "style": if dashed { "dashed" } else { "solid" },
            "evidence": e.evidence,
            "reason": e.reason,
        }));
    }

    json!({
        "nodes": nodes_json,
        "edges": edges_json,
        "legend": [
            { "kind": "concept", "prefix": "c-", "label": "Concept" },
            { "kind": "definition", "prefix": "d-", "label": "Definition" },
            { "kind": "known_result", "prefix": "k-", "label": "Known result" },
            { "kind": "open_question", "prefix": "q-", "label": "Open question" },
            { "kind": "related_work", "prefix": "w-", "label": "Related work" }
        ],
        "overlaps": map.overlaps,
        "dropped": map.dropped,
    })
}
