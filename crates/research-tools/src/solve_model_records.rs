//! Setup sub-records, review/ack/lifecycle payloads, and receipt views for Solve mode.
//! Re-exported from `solve_model` so callers import everything from `research_tools::solve_model`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Calculate,
    Prove,
    CheckDerivation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetForm {
    ClosedForm,
    Series,
    Numeric,
    Bound,
    Proof,
    PerStepVerdict,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolveTarget {
    pub description: String,
    pub form: TargetForm,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digits: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoneCriterionKind {
    FinalTierAtLeast,
    ClosureTierAtLeast,
    NamedCheckPassed,
    NoOpenFlags,
    UserConfirms,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DoneCriterion {
    pub id: String,
    pub kind: DoneCriterionKind,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolSpec {
    pub name: String,
    #[serde(default)]
    pub assumptions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupAssumption {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GivenSource {
    pub source_id: String,
    pub file_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleModel {
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolveRoles {
    pub solver: RoleModel,
    pub verifier: RoleModel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compute: Option<RoleModel>,
}

/// Default Solve budgets (`solve.md` 4.1 & D8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolveBudgets {
    pub max_model_calls: u32,
    pub max_wall_minutes: u32,
    pub max_model_calls_per_leg: u32,
    pub max_review_loops: u32,
    pub max_gate_bounces: u32,
}

impl Default for SolveBudgets {
    fn default() -> Self {
        Self {
            max_model_calls: 60,
            max_wall_minutes: 30,
            max_model_calls_per_leg: 48,
            max_review_loops: 6,
            max_gate_bounces: 3,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewPolicy {
    #[default]
    ProseOnly,
    All,
    FinalOnly,
    Off,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanItem {
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolvePlan {
    #[serde(default)]
    pub items: Vec<PlanItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptRef {
    pub call_id: String,
    pub result_seq: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewIndependence {
    DifferentProvider,
    DifferentModel,
    SameModel,
}

fn canonical_provider_family(raw: &str) -> String {
    let norm = raw.trim().to_ascii_lowercase();
    match norm.as_str() {
        "anthropic" | "anthropic-messages" | "claude-cli" | "claude" => "anthropic".to_string(),
        "openai" | "openai-chat" | "chatgpt" | "chatgpt-responses" => "openai".to_string(),
        "gemini" | "gemini-generate-content" | "google" | "antigravity" => "gemini".to_string(),
        "ollama" | "ollama-chat" => "ollama".to_string(),
        _ => norm,
    }
}

fn canonical_model_id(provider_family: &str, raw: &str) -> String {
    let mut norm = raw.trim().to_ascii_lowercase();
    if let Some(rest) = norm.strip_prefix("models/") {
        norm = rest.trim().to_string();
    } else {
        for prefix in ["anthropic/", "openai/", "chatgpt/", "gemini/", "google/", "ollama/"] {
            if let Some(rest) = norm.strip_prefix(prefix) {
                norm = rest.trim().to_string();
                break;
            }
        }
    }
    // Keep Anthropic shorthand aliases in sync with `agent::transport::ProviderKind::resolve_model`.
    if provider_family == "anthropic" && norm == "haiku" {
        return "claude-haiku-4-5".to_string();
    }
    norm
}

/// Shared host-computed review independence (`00-overview.md` 2.9 & 9; `solve.md` 4.1, 4.4 rule 4,
/// rules R3 & R13). Never trusts model-supplied labels and fails closed to
/// [`ReviewIndependence::SameModel`] whenever either role has an empty or ambiguous provider/model.
pub fn compute_review_independence(
    solver: &RoleModel,
    reviewer: &RoleModel,
) -> ReviewIndependence {
    let sp = canonical_provider_family(&solver.provider);
    let rp = canonical_provider_family(&reviewer.provider);
    let sm = canonical_model_id(&sp, &solver.model);
    let rm = canonical_model_id(&rp, &reviewer.model);
    if sp.is_empty() || rp.is_empty() || sm.is_empty() || rm.is_empty() {
        return ReviewIndependence::SameModel;
    }
    if sp != rp {
        return ReviewIndependence::DifferentProvider;
    }
    if sm != rm {
        return ReviewIndependence::DifferentModel;
    }
    ReviewIndependence::SameModel
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewVerdict {
    Accepted,
    Rejected,
    Incomplete,
    Pending,
}

/// Folded `proof.verification` record (`solve.md` 4.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolveReview {
    pub seq: u64,
    pub target_entry_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_sha256: Option<String>,
    pub verdict: ReviewVerdict,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub independence: Option<ReviewIndependence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verifier_model: Option<String>,
    #[serde(default)]
    pub checks_cited: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_calls_used: Option<u32>,
    #[serde(default)]
    pub issues: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counterexample: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AckKind {
    Assumption,
    Transcription,
    HandChecked,
}

/// Folded `solve.ack` v1 record (`solve.md` 4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolveAck {
    pub seq: u64,
    pub entry_id: String,
    pub kind: AckKind,
    pub entry_sha256: String,
}

/// Stored pause reasons for `solve.paused` (`solve.md` 4.1 & 5.5).
/// Mapping to the shared 7-state on-screen vocabulary belongs to package F9 (`run_budget.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseReason {
    LegCap,
    Budget,
    GateBounces,
    VerificationStuck,
    ApprovalNeeded,
    WorkerRestarted,
    User,
    Interrupted,
    TransportError,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolvePaused {
    pub seq: u64,
    pub reason: PauseReason,
    pub leg: u32,
    pub turn: u32,
    pub model_calls_used: u32,
    #[serde(default)]
    pub open_items: Vec<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointReason {
    Continue,
    Context,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolveCheckpoint {
    pub seq: u64,
    pub leg: u32,
    pub upto_seq: u64,
    pub brief_blob: String,
    pub brief_sha256: String,
    pub reason: CheckpointReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SolveDoneStatus {
    Finished,
    FinishedOpenItems,
}

impl From<PauseReason> for crate::run_budget::SolvePauseReason {
    fn from(reason: PauseReason) -> Self {
        match reason {
            PauseReason::LegCap => Self::LegCap,
            PauseReason::Budget => Self::Budget,
            PauseReason::GateBounces => Self::GateBounces,
            PauseReason::VerificationStuck => Self::VerificationStuck,
            PauseReason::ApprovalNeeded => Self::ApprovalNeeded,
            PauseReason::WorkerRestarted => Self::WorkerRestarted,
            PauseReason::User => Self::User,
            PauseReason::Interrupted => Self::Interrupted,
            PauseReason::TransportError => Self::TransportError,
        }
    }
}

impl From<crate::run_budget::SolvePauseReason> for PauseReason {
    fn from(reason: crate::run_budget::SolvePauseReason) -> Self {
        match reason {
            crate::run_budget::SolvePauseReason::LegCap => Self::LegCap,
            crate::run_budget::SolvePauseReason::Budget => Self::Budget,
            crate::run_budget::SolvePauseReason::GateBounces => Self::GateBounces,
            crate::run_budget::SolvePauseReason::VerificationStuck => Self::VerificationStuck,
            crate::run_budget::SolvePauseReason::ApprovalNeeded => Self::ApprovalNeeded,
            crate::run_budget::SolvePauseReason::WorkerRestarted => Self::WorkerRestarted,
            crate::run_budget::SolvePauseReason::User => Self::User,
            crate::run_budget::SolvePauseReason::Interrupted => Self::Interrupted,
            crate::run_budget::SolvePauseReason::TransportError => Self::TransportError,
        }
    }
}

impl PauseReason {
    pub fn as_str(self) -> &'static str {
        crate::run_budget::SolvePauseReason::from(self).as_str()
    }
}

/// Minimal Solve-internal run lifecycle state mapped onto F9's shared `SolveRunState` / `RunState`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SolveRunStatus {
    #[default]
    Idle,
    Running,
    Paused(PauseReason),
    Done(SolveDoneStatus),
}

impl SolveRunStatus {
    /// Map an active/paused/done Solve lifecycle status onto F9's [`crate::run_budget::SolveRunState`].
    /// Returns `None` for [`SolveRunStatus::Idle`] (before a Solve run has started).
    pub fn to_solve_run_state(self) -> Option<crate::run_budget::SolveRunState> {
        match self {
            Self::Idle => None,
            Self::Running => Some(crate::run_budget::SolveRunState::LegRunning),
            Self::Paused(reason) => Some(crate::run_budget::SolveRunState::Paused(reason.into())),
            Self::Done(SolveDoneStatus::Finished) => Some(crate::run_budget::SolveRunState::Finished),
            Self::Done(SolveDoneStatus::FinishedOpenItems) => {
                Some(crate::run_budget::SolveRunState::FinishedOpenItems)
            }
        }
    }

    /// Map onto F9's shared 7-state on-screen [`crate::run_budget::RunState`] (`00-overview.md` 3.3).
    pub fn to_run_state(self) -> Option<crate::run_budget::RunState> {
        self.to_solve_run_state().map(crate::run_budget::solve_run_state)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CriterionOutcome {
    pub id: String,
    pub met: bool,
    #[serde(default)]
    pub evidence: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeakestDep {
    pub entry_id: String,
    pub tier: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestsOnSummary {
    #[serde(default)]
    pub given: Vec<String>,
    #[serde(default)]
    pub cited: Vec<String>,
    #[serde(default)]
    pub assumed_by_model: Vec<String>,
    #[serde(default)]
    pub weakest: Vec<WeakestDep>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCallsBreakdown {
    pub solver: u32,
    pub verifier: u32,
    pub workers: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolveDone {
    pub seq: u64,
    pub final_entry_id: String,
    pub own_tier: String,
    pub effective_tier: String,
    #[serde(default)]
    pub criteria: Vec<CriterionOutcome>,
    pub rests_on: RestsOnSummary,
    pub status: SolveDoneStatus,
    pub model_calls: ModelCallsBreakdown,
    #[serde(default)]
    pub summary_markdown: String,
    #[serde(default)]
    pub open_items: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptProcess {
    Solver,
    Check,
    Verifier,
}

/// View over a `tool.request`/`tool.result` pair (`solve.md` 4.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolveReceipt {
    pub call_id: String,
    pub tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_seq: Option<u64>,
    pub result_seq: u64,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wall_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<ReceiptProcess>,
    #[serde(default)]
    pub exprs_out: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_blob: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolveArtifact {
    pub artifact_id: String,
    pub name: String,
    pub mime: String,
    pub blob: String,
    pub sha256: String,
    pub bytes: u64,
    pub call_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardrailBlocked {
    pub seq: u64,
    pub gate: String,
    pub tool: String,
    pub call_id: String,
    pub message: String,
}

#[cfg(test)]
#[path = "solve_model_records_tests.rs"]
mod tests;
