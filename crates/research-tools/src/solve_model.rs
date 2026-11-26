//! Solve mode data model (package S2): types, serde schemas, identifier validators,
//! and deterministic hash helpers. Pure code, no I/O, no ledger writes, no tool defs.
//!
//! Shared cross-session references (`NodeRef`, `PaperRef`, `PaperKey`) are imported from
//! `crate::paper_key` (package F2). `QuoteRef` uses the exact schema of package F4
//! (`00-overview.md` section 2.5) with `start_line` and `end_line`.

use crate::paper_key::{mode_of_id, Mode, NodeRef};
pub use crate::solve_model_records::*;
use ledger::{canonical_json, sha256_hex};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fmt;

/// Event kinds written or read by Solve mode.
/// Names governed by owner decisions in `00-overview.md` section 8 (accepted 2026-11-25)
/// remain single named constants so they can change in one place.
pub mod event_kinds {
    pub const SOLVE_STARTED: &str = "solve.started";
    pub const SOLVE_SETUP: &str = "solve.setup";
    pub const SOLVE_PLAN: &str = "solve.plan";
    pub const PROOF_ENTRY: &str = "proof.entry";
    pub const PROOF_SUPERSEDE: &str = "proof.supersede";
    pub const PROOF_VERIFICATION: &str = "proof.verification";
    pub const SOLVE_EXPR: &str = "solve.expr";
    /// Owner decision 11 ("check" belongs to Solve).
    pub const SOLVE_CHECK: &str = "solve.check";
    pub const SOLVE_ACK: &str = "solve.ack";
    pub const SOLVE_PAUSED: &str = "solve.paused";
    pub const SOLVE_CHECKPOINT: &str = "solve.checkpoint";
    pub const SOLVE_DONE: &str = "solve.done";
    pub const SOLVE_ARTIFACT: &str = "solve.artifact";
    /// Owner decision 10 (shared stored-quote event kind, package F4).
    pub const EVIDENCE_QUOTE: &str = "evidence.quote";
    /// Owner decision 10 (shared supplied-context event kind, package F4).
    pub const CONTEXT_SUPPLIED: &str = "context.supplied";
    /// Owner decision 10 (shared hand-off packet event kind, package F7).
    pub const HANDOFF_PACKET: &str = "handoff.packet";
    /// Shared supplied-lessons event kind (package F16; never affects tiers, R24).
    pub const LESSONS_SUPPLIED: &str = "lessons.supplied";
    pub const GUARDRAIL_BLOCKED: &str = "guardrail.blocked";
    pub const TOOL_REQUEST: &str = "tool.request";
    pub const TOOL_RESULT: &str = "tool.result";
    pub const NOTE: &str = "note";
    pub const SESSION_RESUME: &str = "session.resume";
    /// Owner decision 1 (tool profiles, package F1).
    pub const SOLVE_TOOL_PROFILE_V1: &str = "solve/1";
}

pub const MAX_EXPR_NAME_LEN: usize = 64;
pub const MAX_STATEMENT_CHARS: usize = 16_000;
pub const MAX_CONVENTIONS_CHARS: usize = 2_000;
pub const MAX_STR_PREVIEW_CHARS: usize = 400;

/// Validate an expression handle name against `^[A-Za-z_][A-Za-z0-9_]{0,63}$`.
pub fn validate_expr_name(name: &str) -> Result<(), String> {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_EXPR_NAME_LEN {
        return Err(format!(
            "expression name must be 1..={MAX_EXPR_NAME_LEN} ASCII characters (got {})",
            bytes.len()
        ));
    }
    if !(bytes[0].is_ascii_alphabetic() || bytes[0] == b'_') {
        return Err(format!("expression name `{name}` must start with an ASCII letter or `_`"));
    }
    if !bytes[1..].iter().all(|b| b.is_ascii_alphanumeric() || *b == b'_') {
        return Err(format!(
            "expression name `{name}` may contain only ASCII letters, digits, and `_`"
        ));
    }
    Ok(())
}

/// Validate a canonical Solve step id `e-N` (positive integer, no leading zero).
pub fn is_valid_step_id(id: &str) -> bool {
    if mode_of_id(id) != Some(Mode::Solve) {
        return false;
    }
    let Some(rest) = id.strip_prefix("e-") else {
        return false;
    };
    !rest.is_empty()
        && !rest.starts_with('0')
        && rest.bytes().all(|b| b.is_ascii_digit())
        && rest.parse::<u64>().is_ok_and(|n| n >= 1)
}

pub fn validate_step_id(id: &str) -> Result<(), String> {
    if is_valid_step_id(id) {
        Ok(())
    } else {
        Err(format!("invalid Solve step id `{id}` (expected `e-N` with positive integer N)"))
    }
}

/// Versioned expression handle `name@version` (serialized as a plain JSON string).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HandleRef {
    pub name: String,
    pub version: u32,
}

impl HandleRef {
    pub fn new(name: impl Into<String>, version: u32) -> Result<Self, String> {
        let name = name.into();
        validate_expr_name(&name)?;
        if version == 0 {
            return Err(format!("handle version for `{name}` must be >= 1"));
        }
        Ok(Self { name, version })
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        let (name, ver_str) = raw
            .split_once('@')
            .ok_or_else(|| format!("handle `{raw}` must have form `name@version`"))?;
        validate_expr_name(name)?;
        if ver_str.is_empty()
            || ver_str.starts_with('0')
            || !ver_str.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(format!("handle `{raw}` has invalid version `{ver_str}`"));
        }
        let version: u32 =
            ver_str.parse().map_err(|_| format!("handle `{raw}` version overflows u32"))?;
        Ok(Self { name: name.to_string(), version })
    }
}

impl fmt::Display for HandleRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.name, self.version)
    }
}

impl std::str::FromStr for HandleRef {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for HandleRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for HandleRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// Shared quote reference re-exported from package F4 (`quote_core::QuoteRef`, `00-overview.md` 2.5).
pub use crate::quote_core::QuoteRef;

/// Folded representation of a shared `evidence.quote` event (`00-overview.md` 2.5 & 2.10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceQuoteRecord {
    pub seq: u64,
    pub source_id: String,
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub file_sha256: String,
    pub quote_sha256: String,
    /// True when the quote was matched in OCR text extracted from a scanned page.
    #[serde(default)]
    pub ocr: bool,
}

impl EvidenceQuoteRecord {
    pub fn matches_ref(&self, qref: &QuoteRef) -> bool {
        qref.evidence_seq == Some(self.seq)
            && self.source_id == qref.source_id
            && self.path == qref.path
            && self.start_line == qref.start_line
            && self.end_line == qref.end_line
            && self.file_sha256 == qref.file_sha256
            && self.quote_sha256 == qref.quote_sha256
    }
}

/// Payload of `solve.setup` v1 (`solve.md` 4.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolveSetup {
    pub revision: u32,
    pub statement: String,
    pub task_kind: TaskKind,
    pub target: SolveTarget,
    #[serde(default)]
    pub done: Vec<DoneCriterion>,
    #[serde(default)]
    pub conventions: String,
    #[serde(default)]
    pub symbols: Vec<SymbolSpec>,
    #[serde(default)]
    pub dimensions: BTreeMap<String, BTreeMap<String, Value>>,
    #[serde(default)]
    pub assumptions: Vec<SetupAssumption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub given_source: Option<GivenSource>,
    #[serde(default)]
    pub sources: Vec<String>,
    pub roles: SolveRoles,
    #[serde(default)]
    pub budgets: SolveBudgets,
    #[serde(default)]
    pub review: ReviewPolicy,
    pub tool_profile: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<NodeRef>,
    pub math_sha256: String,
}

/// The 9 existing `proof.entry` types (`proof_dag.rs`, none added).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepEntryType {
    Assumption,
    Derivation,
    Result,
    SymbolicComputation,
    NumericalComputation,
    ReferenceLookup,
    Verification,
    Correction,
    FinalAnswer,
}

impl StepEntryType {
    pub const ALL: [Self; 9] = [
        Self::Assumption,
        Self::Derivation,
        Self::Result,
        Self::SymbolicComputation,
        Self::NumericalComputation,
        Self::ReferenceLookup,
        Self::Verification,
        Self::Correction,
        Self::FinalAnswer,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Assumption => "assumption",
            Self::Derivation => "derivation",
            Self::Result => "result",
            Self::SymbolicComputation => "symbolic_computation",
            Self::NumericalComputation => "numerical_computation",
            Self::ReferenceLookup => "reference_lookup",
            Self::Verification => "verification",
            Self::Correction => "correction",
            Self::FinalAnswer => "final_answer",
        }
    }

    pub fn is_computation(self) -> bool {
        matches!(self, Self::SymbolicComputation | Self::NumericalComputation)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepOrigin {
    Solver,
    Setup,
    Given,
    User,
    Worker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimForm {
    Equation,
    Inequality,
    Value,
    Series,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimRelation {
    Eq,
    Le,
    Ge,
    Approx,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeriesOrder {
    pub variable: String,
    pub point: String,
    pub n: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    pub form: ClaimForm,
    pub lhs: HandleRef,
    pub rhs: HandleRef,
    pub relation: ClaimRelation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<SeriesOrder>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub how: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptionMeta {
    pub source_tex_sha256: String,
}

/// Folded `proof.entry` with Solve additive fields (`solve.md` 4.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolveEntry {
    pub entry_id: String,
    pub seq: u64,
    pub entry_type: StepEntryType,
    pub summary: String,
    pub details: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latex: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub derived_depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_ref: Option<String>,
    #[serde(default)]
    pub tool_call_refs: Vec<String>,
    #[serde(default)]
    pub claims: Vec<Claim>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<StepOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<QuoteRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcription: Option<TranscriptionMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
}

/// Payload of `solve.expr` v1 (`solve.md` 4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolveExpr {
    pub name: String,
    pub version: u32,
    pub srepr_blob: String,
    pub srepr_sha256: String,
    pub latex: String,
    pub str_preview: String,
    #[serde(default)]
    pub free_symbols: Vec<SymbolSpec>,
    pub produced_by: ReceiptRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_entry: Option<String>,
    pub portable: bool,
}

impl SolveExpr {
    pub fn handle(&self) -> Result<HandleRef, String> {
        HandleRef::new(self.name.clone(), self.version)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckOutcome {
    Pass,
    Fail,
    Inconclusive,
    NotRun,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckRequester {
    Host,
    Solver,
    Verifier,
    User,
}

/// Payload of `solve.check` v1 (`solve.md` 4.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolveCheck {
    pub check_id: String,
    pub claim_sha256: String,
    pub claim_index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<String>,
    pub math_sha256: String,
    pub name: String,
    pub registry_sha256: String,
    #[serde(default)]
    pub params: Value,
    pub outcome: CheckOutcome,
    pub asked_by: CheckRequester,
    pub receipt: ReceiptRef,
    #[serde(default)]
    pub evidence: Value,
    pub evidence_sha256: String,
}

/// Hash of the setup's mathematical context (`symbols`, `dimensions`, `assumptions`, `conventions`).
pub fn compute_math_sha256(
    symbols: &[SymbolSpec],
    dimensions: &BTreeMap<String, BTreeMap<String, Value>>,
    assumptions: &[SetupAssumption],
    conventions: &str,
) -> String {
    let val = json!({
        "v": 1,
        "symbols": symbols,
        "dimensions": dimensions,
        "assumptions": assumptions,
        "conventions": conventions,
    });
    sha256_hex(canonical_json(&val).as_bytes())
}

/// Hash of a step's claims with the `srepr_sha256` of every handle they name.
/// Excludes prose (`how`, summary, details): checks test mathematics only (`solve.md` 4.1).
pub fn compute_claim_sha256<'a, F>(claims: &[Claim], mut srepr_sha256_of: F) -> Result<String, String>
where
    F: FnMut(&HandleRef) -> Option<&'a str>,
{
    let mut items = Vec::with_capacity(claims.len());
    for claim in claims {
        let lhs_hash = srepr_sha256_of(&claim.lhs)
            .ok_or_else(|| format!("unknown handle `{}` in claim lhs", claim.lhs))?;
        let rhs_hash = srepr_sha256_of(&claim.rhs)
            .ok_or_else(|| format!("unknown handle `{}` in claim rhs", claim.rhs))?;
        items.push(json!({
            "form": claim.form,
            "lhs": claim.lhs,
            "lhs_srepr_sha256": lhs_hash,
            "rhs": claim.rhs,
            "rhs_srepr_sha256": rhs_hash,
            "relation": claim.relation,
            "order": claim.order,
            "domain": claim.domain,
        }));
    }
    let val = json!({ "v": 1, "claims": items });
    Ok(sha256_hex(canonical_json(&val).as_bytes()))
}

/// Hash of a step's claims, summary, and details (`solve.md` 4.1).
/// Reviews and acknowledgements bind to this hash because a reviewer judges prose too.
pub fn compute_entry_sha256(claim_sha256: Option<&str>, summary: &str, details: &str) -> String {
    let val = json!({
        "v": 1,
        "claim_sha256": claim_sha256,
        "summary": summary,
        "details": details,
    });
    sha256_hex(canonical_json(&val).as_bytes())
}

#[cfg(test)]
#[path = "solve_model_tests.rs"]
mod tests;
