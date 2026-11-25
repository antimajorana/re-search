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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formal: Option<RoleModel>,
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
    FormalStatement,
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

/// Minimal Solve-internal run lifecycle state (mapped to F9's shared `RunState` on merge).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SolveRunStatus {
    #[default]
    Idle,
    Running,
    Paused(PauseReason),
    Done(SolveDoneStatus),
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
