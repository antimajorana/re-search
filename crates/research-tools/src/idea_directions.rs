//! Explore Directions recorder (`record_idea_directions`), Rules D1/D2/V8 validation, and
//! Solve-aligned `SolveSetup` schema (package X13).
//!
//! Governance: `explore.md` 4.1 (`Direction`, `FirstCalculation`, `SolveSetup`), 4.2
//! (`idea.directions`), 5.1 (`record_idea_directions`), 6 (Rules D1, D2, V8, S8);
//! `00-overview.md` 0.1 (no `lean` method, no `"verified"`), 3.1 row 9 (`solve_setup` uses
//! Solve's exact `task_kind` and structured `target: { description, form }`).

use crate::solve_model::{TargetForm, TaskKind};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Value};

pub use crate::idea_directions_validate::validate_record_idea_directions;

/// Recorder tool name offered in the Directions exchange (`explore.md` 5.1).
pub const RECORDER_IDEA_DIRECTIONS: &str = "record_idea_directions";
/// Ledger event kind for recorded Explore directions (`explore.md` 4.2).
pub const EVENT_IDEA_DIRECTIONS: &str = "idea.directions";
/// Maximum directions allowed in a single call (`explore.md` 5.1).
pub const MAX_DIRECTIONS: usize = 5;
/// Minimum character length of `first_calculation.setup` for `Concreteness::Concrete` (Rule D2).
pub const MIN_CONCRETE_SETUP_CHARS: usize = 40;
/// Maximum text bounds to prevent unbounded payloads.
pub const MAX_DIRECTION_TITLE_CHARS: usize = 200;
pub const MAX_DIRECTION_TEXT_CHARS: usize = 2_000;
pub const MAX_SOLVE_STATEMENT_CHARS: usize = 4_000;
pub const MAX_SOLVE_CONVENTIONS_CHARS: usize = 2_000;

/// Permitted first-calculation methods (`explore.md` 4.1 & `00-overview.md` 0.1: NO `lean`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FirstCalcMethod {
    ByHand,
    Sympy,
    Numeric,
    Literature,
}

impl FirstCalcMethod {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ByHand => "by_hand",
            Self::Sympy => "sympy",
            Self::Numeric => "numeric",
            Self::Literature => "literature",
        }
    }
}

/// Estimated effort for a direction's first calculation (`explore.md` 4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffortLevel {
    Hours,
    Days,
    Weeks,
}

/// Risk taxonomy on a direction (`explore.md` 4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskKind {
    AlreadyDone,
    Technical,
    NoGo,
    LowPayoff,
}

impl RiskKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AlreadyDone => "already_done",
            Self::Technical => "technical",
            Self::NoGo => "no_go",
            Self::LowPayoff => "low_payoff",
        }
    }
}

/// App-computed concreteness classification (Rule D2; never accepted on model input).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Concreteness {
    Concrete,
    Sketch,
}

/// Basis required on every `Direction.missing` entry (`explore.md` 4.1 & Rule D1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissingBasis {
    NoEvidenceInScope,
    OpenQuestion(String),
}

impl MissingBasis {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let trimmed = raw.trim();
        if trimmed == "no_evidence_in_scope" {
            return Ok(Self::NoEvidenceInScope);
        }
        if let Some(inner) = trimmed
            .strip_prefix("open_question(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let qid = inner.trim();
            if is_open_question_id(qid) {
                return Ok(Self::OpenQuestion(qid.to_string()));
            }
            return Err(format!(
                "invalid open_question id `{qid}` in missing.basis (expected `open_question(q-N)`)"
            ));
        }
        Err(format!(
            "invalid missing.basis `{trimmed}` (expected `no_evidence_in_scope` or `open_question(q-N)`)"
        ))
    }

    pub fn format_wire(&self) -> String {
        match self {
            Self::NoEvidenceInScope => "no_evidence_in_scope".to_string(),
            Self::OpenQuestion(qid) => format!("open_question({qid})"),
        }
    }
}

impl Serialize for MissingBasis {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.format_wire())
    }
}

impl<'de> Deserialize<'de> for MissingBasis {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// One known-fact anchor on a direction (`explore.md` 4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectionKnown {
    pub text: String,
    #[serde(default)]
    pub node_ids: Vec<String>,
}

/// One missing-piece item with mandatory basis (`explore.md` 4.1 & Rule D1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectionMissing {
    pub text: String,
    pub basis: MissingBasis,
}

/// Sanity/reproduction check on a first calculation (`explore.md` 4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirstCalcCheck {
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reproduces: Option<String>,
}

/// Concrete or sketch first calculation proposed for a direction (`explore.md` 4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirstCalculation {
    pub setup: String,
    #[serde(default)]
    pub inputs: Vec<String>,
    pub method: FirstCalcMethod,
    pub expected_form: String,
    pub check: FirstCalcCheck,
    pub effort: EffortLevel,
}

/// Explicit risk attached to a direction (`explore.md` 4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectionRisk {
    pub text: String,
    pub kind: RiskKind,
    #[serde(default)]
    pub node_ids: Vec<String>,
}

/// Structured Solve target using Solve's exact field names (`00-overview.md` 3.1 row 9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolveSetupTarget {
    pub description: String,
    pub form: TargetForm,
}

/// Solve hand-off setup using Solve's exact `solve.setup` field names (`00-overview.md` 3.1 row 9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolveSetup {
    pub statement: String,
    pub task_kind: TaskKind,
    pub target: SolveSetupTarget,
    #[serde(default)]
    pub conventions: String,
    #[serde(default)]
    pub source_keys: Vec<String>,
}

/// Model-supplied direction before app ID assignment and concreteness computation.
/// `#[serde(deny_unknown_fields)]` blocks forged `id`, `concreteness`, or unknown fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectionInput {
    pub title: String,
    pub question: String,
    #[serde(default)]
    pub known: Vec<DirectionKnown>,
    #[serde(default)]
    pub missing: Vec<DirectionMissing>,
    pub first_calculation: FirstCalculation,
    #[serde(default)]
    pub risks: Vec<DirectionRisk>,
    #[serde(default)]
    pub stop_if: String,
    pub solve_setup: SolveSetup,
}

/// Top-level model input for `record_idea_directions` (`#[serde(deny_unknown_fields)]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordIdeaDirectionsInput {
    pub directions: Vec<DirectionInput>,
}

/// App-validated, ID-assigned direction persisted in `idea.directions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Direction {
    pub id: String,
    pub title: String,
    pub question: String,
    pub known: Vec<DirectionKnown>,
    pub missing: Vec<DirectionMissing>,
    pub first_calculation: FirstCalculation,
    pub risks: Vec<DirectionRisk>,
    pub stop_if: String,
    pub solve_setup: SolveSetup,
    pub concreteness: Concreteness,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub concreteness_reasons: Vec<String>,
}

/// One rejected direction entry reported back to the caller/exchange.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectedDirection {
    pub index: usize,
    pub title: String,
    pub reasons: Vec<String>,
}

/// Persisted `idea.directions` ledger payload (`explore.md` 4.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeaDirectionsPayload {
    pub map_seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_seq: Option<u64>,
    pub directions: Vec<Direction>,
    #[serde(default)]
    pub rejected: Vec<RejectedDirection>,
}

/// Strict JSON Schema offered as the sole tool in the Directions exchange (`explore.md` 5.1).
pub fn record_idea_directions_schema() -> Value {
    let first_calc_schema = json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["setup", "inputs", "method", "expected_form", "check", "effort"],
        "properties": {
            "setup": { "type": "string", "maxLength": MAX_DIRECTION_TEXT_CHARS },
            "inputs": { "type": "array", "items": { "type": "string", "minLength": 1 } },
            "method": { "type": "string", "enum": ["by_hand", "sympy", "numeric", "literature"] },
            "expected_form": { "type": "string", "minLength": 1, "maxLength": MAX_DIRECTION_TEXT_CHARS },
            "check": {
                "type": "object",
                "additionalProperties": false,
                "required": ["text"],
                "properties": {
                    "text": { "type": "string", "maxLength": MAX_DIRECTION_TEXT_CHARS },
                    "reproduces": { "type": ["string", "null"], "pattern": "^k-[1-9][0-9]*$" }
                }
            },
            "effort": { "type": "string", "enum": ["hours", "days", "weeks"] }
        }
    });
    let solve_setup_schema = json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["statement", "task_kind", "target", "conventions", "source_keys"],
        "properties": {
            "statement": { "type": "string", "minLength": 1, "maxLength": MAX_SOLVE_STATEMENT_CHARS },
            "task_kind": { "type": "string", "enum": ["calculate", "prove", "check_derivation"] },
            "target": {
                "type": "object",
                "additionalProperties": false,
                "required": ["description", "form"],
                "properties": {
                    "description": { "type": "string", "minLength": 1, "maxLength": MAX_DIRECTION_TEXT_CHARS },
                    "form": { "type": "string", "enum": ["closed_form", "series", "numeric", "bound", "proof", "per_step_verdict"] }
                }
            },
            "conventions": { "type": "string", "maxLength": MAX_SOLVE_CONVENTIONS_CHARS },
            "source_keys": { "type": "array", "items": { "type": "string", "minLength": 1 } }
        }
    });
    let item_schema = json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["title", "question", "known", "missing", "first_calculation", "risks", "stop_if", "solve_setup"],
        "properties": {
            "title": { "type": "string", "minLength": 1, "maxLength": MAX_DIRECTION_TITLE_CHARS },
            "question": { "type": "string", "minLength": 1, "maxLength": MAX_DIRECTION_TEXT_CHARS },
            "known": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["text", "node_ids"],
                    "properties": {
                        "text": { "type": "string", "minLength": 1, "maxLength": MAX_DIRECTION_TEXT_CHARS },
                        "node_ids": { "type": "array", "items": { "type": "string", "minLength": 1 } }
                    }
                }
            },
            "missing": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["text", "basis"],
                    "properties": {
                        "text": { "type": "string", "minLength": 1, "maxLength": MAX_DIRECTION_TEXT_CHARS },
                        "basis": { "type": "string", "description": "Either `no_evidence_in_scope` or `open_question(q-N)` naming a map open_question node." }
                    }
                }
            },
            "first_calculation": first_calc_schema,
            "risks": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["text", "kind", "node_ids"],
                    "properties": {
                        "text": { "type": "string", "minLength": 1, "maxLength": MAX_DIRECTION_TEXT_CHARS },
                        "kind": { "type": "string", "enum": ["already_done", "technical", "no_go", "low_payoff"] },
                        "node_ids": { "type": "array", "items": { "type": "string", "minLength": 1 } }
                    }
                }
            },
            "stop_if": { "type": "string", "maxLength": MAX_DIRECTION_TEXT_CHARS },
            "solve_setup": solve_setup_schema
        }
    });
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["directions"],
        "properties": {
            "directions": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_DIRECTIONS,
                "items": item_schema
            }
        }
    })
}

fn is_open_question_id(s: &str) -> bool {
    s.strip_prefix("q-")
        .is_some_and(|rest| !rest.is_empty() && !rest.starts_with('0') && rest.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
#[path = "idea_directions_tests.rs"]
mod tests;
