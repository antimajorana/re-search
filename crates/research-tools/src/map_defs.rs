//! Tool definitions and shared Layer-2 record types for Investigate v2 (package V6, `investigate.md` §4.3, §5.1).
//!
//! Only `read_map_node` and `record_map_reading` (`ReadWorkspace`) are defined and registered in V6,
//! exclusively through tool profile `investigation/2` (never added to `register_research_tools`).
//! `record_context_cards` (V12) and `record_prior_work_check` (deferred V14) are kept as reserved
//! ID constants under `may_gain` only.

use crate::skeleton::SkNodeKind;
use crate::ResearchEnv;
use policy::EffectClass;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tools::{ApprovalScope, CallCtx, Registry, ToolDef};

/// Investigate v2 node reader tool ID (`investigate.md` §5.1, `00-overview.md` §2.2).
pub const READ_MAP_NODE: &str = "read_map_node";
/// Investigate v2 reading/edge recorder tool ID (`investigate.md` §5.1, `00-overview.md` §2.2).
pub const RECORD_MAP_READING: &str = "record_map_reading";
/// Reserved ID for V12 context-card recorder (not registered in V6; listed in `may_gain`).
pub const RECORD_CONTEXT_CARDS: &str = "record_context_cards";
/// Reserved ID for deferred V14 prior-work recorder (not registered in V6; listed in `may_gain`).
pub const RECORD_PRIOR_WORK_CHECK: &str = "record_prior_work_check";

/// Investigate v2 `ReadWorkspace` tool IDs permitted under `may_gain` for `investigation/1` and
/// `investigation/2` (never added to `research/1`).
pub const INVESTIGATION_V2_MAY_GAIN: &[&str] = &[
    READ_MAP_NODE,
    RECORD_MAP_READING,
    RECORD_CONTEXT_CARDS,
    RECORD_PRIOR_WORK_CHECK,
];

/// Maximum items per `record_map_reading` call (`investigate.md` §5.1).
pub const MAX_READING_ITEMS: usize = 16;
/// Maximum model edges per `record_map_reading` call (`investigate.md` §5.1).
pub const MAX_READING_EDGES: usize = 32;
/// Maximum limitation notes per `record_map_reading` call (`investigate.md` §5.1).
pub const MAX_READING_LIMITATIONS: usize = 8;
/// Maximum hypotheses per reading item.
pub const MAX_READING_HYPOTHESES: usize = 12;
/// Maximum anchors per reading item (`investigate.md` §4.3).
pub const MAX_READING_ANCHORS: usize = 3;
/// Maximum gist length in Unicode scalar values (`investigate.md` §4.3).
pub const MAX_GIST_CHARS: usize = 400;
/// Maximum purpose length in Unicode scalar values (`investigate.md` §4.3).
pub const MAX_PURPOSE_CHARS: usize = 300;
/// Default character budget for `read_map_node`.
pub const DEFAULT_READ_NODE_MAX_CHARS: usize = 12_000;
/// Minimum allowed `max_chars` for `read_map_node`.
pub const MIN_READ_NODE_MAX_CHARS: usize = 500;
/// Maximum allowed `max_chars` for `read_map_node`.
pub const MAX_READ_NODE_MAX_CHARS: usize = 24_000;

/// Model-assigned role of a skeleton node (`investigate.md` §4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadingRole {
    MainResult,
    Supporting,
    Technical,
    Background,
    StandardTool,
}

impl ReadingRole {
    /// Enforce rule 6.5: `main_result` is refused on proof, remark, example, figure, table,
    /// and bibliography nodes.
    pub fn allowed_for_kind(self, kind: SkNodeKind) -> bool {
        if self != Self::MainResult {
            return true;
        }
        !matches!(
            kind,
            SkNodeKind::Proof
                | SkNodeKind::Remark
                | SkNodeKind::Example
                | SkNodeKind::Figure
                | SkNodeKind::Table
                | SkNodeKind::Bibliography
        )
    }
}

/// Authors' own claim classification (`investigate.md` §4.3, rule 6.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorsClaimLabel {
    New,
    Restated,
    Background,
    NotStated,
}

/// Role of a model-proposed dependency edge (`investigate.md` §4.3, rule 6.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelEdgeRole {
    Statement,
    Proof,
    Argument,
}

/// Evidence basis of a model-proposed edge (`investigate.md` §4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelEdgeBasis {
    Explicit,
    Inferred,
}

/// Matched, hash-pinned source anchor stored on Layer-2 Investigate records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedMapAnchor {
    pub source_id: String,
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub file_sha256: String,
    pub quote_sha256: String,
    pub quote: String,
    pub excerpt: String,
    pub source_seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cited_range: Option<[usize; 2]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub range_basis: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt_window: Option<[usize; 2]>,
}

/// One matched hypothesis attached to a reading item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HypothesisRecord {
    pub text: String,
    pub anchor: VerifiedMapAnchor,
}

/// Stored authors'-claim sub-record with app-filled `cues` (`investigate.md` §4.3, rule 6.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorsClaimRecord {
    pub label: AuthorsClaimLabel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation_note: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributed_to: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<VerifiedMapAnchor>,
    #[serde(default)]
    pub cues: Vec<String>,
}

/// One accepted reading item persisted inside an `investigation.reading` ledger event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadingItemRecord {
    pub tag: String,
    pub node_fp: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attach_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<ReadingRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gist: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hypotheses: Vec<HypothesisRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<VerifiedMapAnchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authors_claim: Option<AuthorsClaimRecord>,
}

/// One accepted model edge persisted inside an `investigation.reading` ledger event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelEdgeRecord {
    pub from: String,
    pub to: String,
    pub role: ModelEdgeRole,
    pub basis: ModelEdgeBasis,
    pub reason: String,
    pub anchor: VerifiedMapAnchor,
}

/// Indexed per-item or per-edge validation fault (`investigate.md` §5.1, rule 6.10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedFault {
    pub index: usize,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub true_text: Option<String>,
}

/// Per-call rejected items and edges recorded alongside accepted siblings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadingRejected {
    #[serde(default)]
    pub items: Vec<IndexedFault>,
    #[serde(default)]
    pub edges: Vec<IndexedFault>,
}

/// Stored payload of an `investigation.reading` ledger event (`investigate.md` §4.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapReadingEventPayload {
    pub schema_version: u32,
    pub request_seq: u64,
    pub source_id: String,
    pub skeleton_seq: u64,
    pub done: bool,
    #[serde(default)]
    pub items: Vec<ReadingItemRecord>,
    #[serde(default)]
    pub edges: Vec<ModelEdgeRecord>,
    #[serde(default)]
    pub limitations: Vec<String>,
    #[serde(default)]
    pub rejected: ReadingRejected,
    pub anchor_report: Value,
}

fn str_schema(min: usize, max: usize) -> Value {
    json!({ "type": "string", "minLength": min, "maxLength": max })
}

fn pointer_anchor_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "source_id": str_schema(1, 64),
            "path": str_schema(1, 1024),
            "start_line": { "type": "integer", "minimum": 1 },
            "end_line": { "type": "integer", "minimum": 1 },
            "quote": str_schema(3, 300)
        },
        "required": ["source_id", "start_line", "end_line", "quote"],
        "additionalProperties": false
    })
}

/// Frozen `ToolDef` for `read_map_node` (`investigate.md` §5.1, profile `investigation/2`).
pub fn read_map_node_def() -> ToolDef {
    ToolDef {
        id: READ_MAP_NODE.into(),
        description: "Read a node from the pinned paper skeleton by stable tag (for example `n-...`), TeX label, or printed reference (for example `Theorem 4.5`). Returns line-numbered physical source text with file SHA-256 hashes, structural dependencies grouped by role, dependents, and symbols, truncated cleanly at line boundaries to fit `max_chars`.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "source_id": str_schema(1, 64),
                "node": str_schema(1, 160),
                "include": {
                    "type": "array",
                    "maxItems": 5,
                    "items": {
                        "type": "string",
                        "enum": ["statement", "proof", "uses", "used_by", "symbols"]
                    }
                },
                "max_chars": {
                    "type": "integer",
                    "minimum": MIN_READ_NODE_MAX_CHARS,
                    "maximum": MAX_READ_NODE_MAX_CHARS
                }
            },
            "required": ["source_id", "node"],
            "additionalProperties": false
        }),
        effect: EffectClass::ReadWorkspace,
        approval_scope: ApprovalScope::Once,
    }
}

/// Frozen `ToolDef` for `record_map_reading` (`investigate.md` §4.3, §5.1, profile `investigation/2`).
pub fn record_map_reading_def() -> ToolDef {
    let anchor = pointer_anchor_schema();
    let hypothesis = json!({
        "type": "object",
        "properties": {
            "text": str_schema(1, 400),
            "anchor": anchor
        },
        "required": ["text", "anchor"],
        "additionalProperties": false
    });
    let authors_claim = json!({
        "type": "object",
        "properties": {
            "label": { "type": "string", "enum": ["new", "restated", "background", "not_stated"] },
            "relation_note": str_schema(1, 600),
            "attributed_to": {
                "type": "array",
                "maxItems": 16,
                "items": str_schema(1, 128)
            },
            "anchors": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_READING_ANCHORS,
                "items": anchor
            }
        },
        "required": ["label"],
        "additionalProperties": false
    });
    let new_node = json!({
        "type": "object",
        "properties": {
            "id": str_schema(1, 64),
            "attach_to": str_schema(1, 64),
            "kind": str_schema(1, 48)
        },
        "required": ["id", "attach_to", "kind"],
        "additionalProperties": false
    });
    let item = json!({
        "type": "object",
        "properties": {
            "tag": str_schema(1, 64),
            "new": new_node,
            "role": {
                "type": "string",
                "enum": ["main_result", "supporting", "technical", "background", "standard_tool"]
            },
            "gist": str_schema(1, MAX_GIST_CHARS),
            "purpose": str_schema(1, MAX_PURPOSE_CHARS),
            "hypotheses": {
                "type": "array",
                "maxItems": MAX_READING_HYPOTHESES,
                "items": hypothesis
            },
            "anchors": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_READING_ANCHORS,
                "items": anchor
            },
            "authors_claim": authors_claim
        },
        "additionalProperties": false
    });
    let edge = json!({
        "type": "object",
        "properties": {
            "from": str_schema(1, 64),
            "to": str_schema(1, 64),
            "role": { "type": "string", "enum": ["statement", "proof", "argument"] },
            "basis": { "type": "string", "enum": ["explicit", "inferred"] },
            "reason": str_schema(1, 600),
            "anchor": anchor
        },
        "required": ["from", "to", "role", "basis", "reason", "anchor"],
        "additionalProperties": false
    });
    ToolDef {
        id: RECORD_MAP_READING.into(),
        description: "Record additive reading annotations and model dependency edges on nodes of the pinned paper skeleton during an active orient or open task. Every quote must match lines already shown to this caller and satisfy node/proof span rules. Valid siblings in a call are saved even when other items fail with indexed faults; call multiple times to save early and often.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "request_seq": { "type": "integer", "minimum": 1 },
                "source_id": str_schema(1, 64),
                "items": { "type": "array", "maxItems": MAX_READING_ITEMS, "items": item },
                "edges": { "type": "array", "maxItems": MAX_READING_EDGES, "items": edge },
                "limitations": {
                    "type": "array",
                    "maxItems": MAX_READING_LIMITATIONS,
                    "items": str_schema(1, 500)
                },
                "done": { "type": "boolean" }
            },
            "required": ["request_seq", "source_id", "items", "edges", "limitations", "done"],
            "additionalProperties": false
        }),
        effect: EffectClass::ReadWorkspace,
        approval_scope: ApprovalScope::Once,
    }
}

/// Register ONLY the two V6 Investigate tools (`read_map_node` and `record_map_reading`) on a
/// profile-built `investigation/2` registry. Never called by `register_research_tools`.
pub fn register_map_tools(registry: &mut Registry, env: Arc<ResearchEnv>) {
    let read_env = Arc::clone(&env);
    registry.register(
        read_map_node_def(),
        Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
            let ledger = ctx.ledger.lock().map_err(|_| "ledger lock poisoned")?;
            crate::map_node_read::read_map_node(&read_env, &ledger, ctx.session, args)
        }),
    );
    registry.register(
        record_map_reading_def(),
        Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
            let ledger = ctx.ledger.lock().map_err(|_| "ledger lock poisoned")?;
            crate::map_reading::record_map_reading(&env, &ledger, ctx.session, ctx.caller, args)
        }),
    );
}
