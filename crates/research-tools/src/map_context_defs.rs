//! Tool definition, constants, and shared Layer-2 record types for `record_context_cards` (V12).

use crate::investigation_markers::{active_investigation_slice, EVENT_INVESTIGATION_CONTEXT};
use crate::map_defs::{
    pointer_anchor_schema, str_schema, HypothesisRecord, IndexedFault, VerifiedMapAnchor,
    RECORD_CONTEXT_CARDS,
};
use ledger::Event;
use policy::EffectClass;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tools::{ApprovalScope, ToolDef};

pub const CONTEXT_SCHEMA_VERSION: u32 = 1;
pub const MAX_CONTEXT_CARDS: usize = 8;
pub const MAX_CONTEXT_CARD_ANCHORS: usize = 6;
pub const MAX_CONTEXT_CARD_HYPOTHESES: usize = 8;
pub const MAX_CONTEXT_CARD_TITLE_CHARS: usize = 160;
pub const MAX_GROUNDED_BODY_CHARS: usize = 900;
pub const MAX_MODEL_BACKGROUND_BODY_CHARS: usize = 300;
pub const MAX_MODEL_BACKGROUND_PER_NODE: usize = 5;
pub const MAX_CONTEXT_HYPOTHESIS_CHARS: usize = 400;
pub const EVENT_INVESTIGATION_CITED_LOOKUP: &str = "investigation.cited_lookup";

/// Kind of a context card stored on an Investigate map node (`investigate.md` §4.3).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextCardType {
    Symbol,
    Prerequisite,
    CitedResult,
}

impl ContextCardType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Symbol => "symbol",
            Self::Prerequisite => "prerequisite",
            Self::CitedResult => "cited_result",
        }
    }
}

/// Code-assigned provenance of a context card (`investigate.md` §4.3, rule 6.8).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextCardOrigin {
    Paper,
    External,
    ModelBackground,
}

impl ContextCardOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Paper => "paper",
            Self::External => "external",
            Self::ModelBackground => "model_background",
        }
    }
}

/// Persisted Layer-2 context card stored inside `investigation.context` (`investigate.md` §4.3).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextCardRecord {
    #[serde(rename = "type")]
    pub card_type: ContextCardType,
    pub title: String,
    pub body: String,
    pub origin: ContextCardOrigin,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<VerifiedMapAnchor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hypotheses: Vec<HypothesisRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bib_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_fp: Option<String>,
}

/// Persisted payload of an `investigation.context` event (`investigate.md` §4.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapContextEventPayload {
    pub schema_version: u32,
    pub request_seq: u64,
    pub source_id: String,
    pub skeleton_seq: u64,
    pub tag: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cards: Vec<ContextCardRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejected: Vec<IndexedFault>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub anchor_report: Value,
}

/// Tool definition for `record_context_cards` (`investigate.md` §5.1, profile `investigation/2`).
pub fn record_context_cards_def() -> ToolDef {
    ToolDef {
        id: RECORD_CONTEXT_CARDS.into(),
        description: "Record validated context cards (`symbol`, `prerequisite`, or `cited_result`) \
            for one node `tag` during an active `context` or `open` task. Supply `anchors` from \
            matched quotes you received; code assigns `origin` (`paper`, `external`, or \
            `model_background` only for `prerequisite` <= 300 chars and <= 5 per node). Never \
            supply `origin` or app-only anchor digest/excerpt fields. Failed anchors drop their \
            card rather than downgrading. `cited_result` requires `bib_key` present in the \
            skeleton and located in `investigation.cited_lookup`, with at least one anchor and \
            all `hypotheses` anchored inside that located external node."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "request_seq": { "type": "integer", "minimum": 1 },
                "source_id": str_schema(1, 64),
                "tag": str_schema(1, 64),
                "cards": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MAX_CONTEXT_CARDS,
                    "items": {
                        "type": "object",
                        "properties": {
                            "type": {
                                "type": "string",
                                "enum": ["symbol", "prerequisite", "cited_result"]
                            },
                            "title": str_schema(1, MAX_CONTEXT_CARD_TITLE_CHARS),
                            "body": str_schema(1, MAX_GROUNDED_BODY_CHARS),
                            "anchors": {
                                "type": "array",
                                "maxItems": MAX_CONTEXT_CARD_ANCHORS,
                                "items": pointer_anchor_schema()
                            },
                            "hypotheses": {
                                "type": "array",
                                "maxItems": MAX_CONTEXT_CARD_HYPOTHESES,
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "text": str_schema(1, MAX_CONTEXT_HYPOTHESIS_CHARS),
                                        "anchor": pointer_anchor_schema()
                                    },
                                    "required": ["text", "anchor"],
                                    "additionalProperties": false
                                }
                            },
                            "bib_key": str_schema(1, 128)
                        },
                        "required": ["type", "title", "body", "anchors"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["request_seq", "source_id", "tag", "cards"],
            "additionalProperties": false
        }),
        effect: EffectClass::ReadWorkspace,
        approval_scope: ApprovalScope::Once,
    }
}

/// Count existing `model_background` context cards recorded for `(source_id, tag)` in the active session slice.
pub fn count_model_background_cards(events: &[Event], source_id: &str, tag: &str) -> usize {
    let Ok((slice, _)) = active_investigation_slice(events) else {
        return 0;
    };
    slice
        .iter()
        .filter(|ev| ev.kind == EVENT_INVESTIGATION_CONTEXT)
        .filter_map(|ev| serde_json::from_value::<MapContextEventPayload>(ev.payload.clone()).ok())
        .filter(|p| p.source_id == source_id && p.tag == tag)
        .flat_map(|p| p.cards)
        .filter(|c| c.origin == ContextCardOrigin::ModelBackground)
        .count()
}
