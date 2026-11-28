//! Shared run budget and run-state vocabulary (package F9).
//!
//! One pure ledger-event counter for provider model calls across all modes, a structured budget
//! type (`ModelCallBudget`), and the shared seven-state run vocabulary with total mappings from
//! Investigate, Explore and Solve stored states (`docs/design/modes-2026-11/00-overview.md`, 3.3).
//!
//! Nothing here touches I/O or formats user-facing budget sentences; on-screen budget prose
//! belongs to `run_bar.js` (package F6).

use ledger::Event;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;

/// Recommended attribution field for provider-request events (`model.request`, `verifier.request`,
/// `strategist.request`, `narrator.request`, `delegate.turn`, `delegate.end`, `explore.branch`,
/// `explore.end`, `idea.exchange`). Today's `runner.rs`, `autonomous.rs`, `delegate.rs` and
/// `paper_read.rs` events do not emit `run_id` yet; callers scoping a run before emitters adopt it
/// can use [`RunScope::AfterSeq`] or [`RunScope::SeqRange`].
pub const RUN_ID_FIELD: &str = "run_id";

/// Explore exchange provider-call event kind.
/// Provisional pending owner decision 11 in `docs/design/modes-2026-11/00-overview.md` (Explore
/// renamed "leg" to "exchange", section 3.2 row 17).
pub const IDEA_EXCHANGE_EVENT: &str = "idea.exchange";

/// On-screen labels for the seven [`RunState`] variants (overview section 3.3).
pub const RUN_STATE_LABEL_RUNNING: &str = "Running";
pub const RUN_STATE_LABEL_WAITING_FOR_YOU: &str = "Waiting for you";
pub const RUN_STATE_LABEL_PAUSED: &str = "Paused";
pub const RUN_STATE_LABEL_INTERRUPTED: &str = "Interrupted";
pub const RUN_STATE_LABEL_FAILED: &str = "Failed";
pub const RUN_STATE_LABEL_DONE: &str = "Done";
pub const RUN_STATE_LABEL_DONE_WITH_OPEN_ITEMS: &str = "Done with open items";

/// Which subset of ledger events belongs to the run being measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunScope<'a> {
    /// Every event in the supplied slice.
    All,
    /// Events strictly after `seq` (`ev.seq > seq`).
    AfterSeq(u64),
    /// Events with `start_exclusive < ev.seq && ev.seq <= end_inclusive`.
    SeqRange { start_exclusive: u64, end_inclusive: u64 },
    /// Events whose `payload["run_id"]` matches the given id (JSON string or unsigned integer
    /// equal to its canonical decimal string).
    RunId(&'a str),
}

/// Provider model calls broken down by emitter family.
///
/// One model call equals one request sent to a provider (`00-overview.md` section 3.2 row 22).
/// `model.response` and `model.error` are excluded: `runner.rs` emits `model.request` immediately
/// before every `send_prepared_streaming` call, whereas a `prepare_request` failure before
/// `model.request` sends no request to the provider.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCallBreakdown {
    /// Main loop requests (`model.request`).
    pub main_loop: u64,
    /// Verifier requests (`verifier.request`).
    pub verifier: u64,
    /// Strategist requests (`strategist.request`).
    pub strategist: u64,
    /// Narrator requests (`narrator.request`).
    pub narrator: u64,
    /// Delegated worker calls (`delegate.turn` / `delegate.end` and `explore.branch` /
    /// `explore.end`), deduplicated per `call_id` via `max(turn_events, end_turns)` so a failed
    /// final worker turn is counted without double-counting completed turns.
    pub delegated: u64,
    /// Explore exchange calls (`idea.exchange`, one per provider call attempt).
    pub exchange: u64,
}

impl ModelCallBreakdown {
    /// Total provider calls across all families (saturating sum).
    pub fn total(&self) -> u64 {
        self.main_loop
            .saturating_add(self.verifier)
            .saturating_add(self.strategist)
            .saturating_add(self.narrator)
            .saturating_add(self.delegated)
            .saturating_add(self.exchange)
    }
}

fn matches_scope(ev: &Event, scope: RunScope<'_>) -> bool {
    match scope {
        RunScope::All => true,
        RunScope::AfterSeq(seq) => ev.seq > seq,
        RunScope::SeqRange { start_exclusive, end_inclusive } => {
            ev.seq > start_exclusive && ev.seq <= end_inclusive
        }
        RunScope::RunId(target) => match ev.payload.get(RUN_ID_FIELD) {
            Some(Value::String(s)) => s == target,
            Some(Value::Number(n)) => n.as_u64().is_some_and(|v| v.to_string() == target),
            _ => false,
        },
    }
}

fn non_empty_str<'a>(payload: &'a Value, key: &str) -> Option<&'a str> {
    payload.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn payload_u64(payload: &Value, key: &str) -> u64 {
    payload.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn add_call_turns<'a>(
    calls: &mut BTreeMap<&'a str, (u64, u64)>,
    unkeyed: &mut u64,
    payload: &'a Value,
    turns: u64,
    is_end: bool,
) {
    match non_empty_str(payload, "call_id") {
        Some(id) => {
            let e = calls.entry(id).or_default();
            if is_end { e.1 = e.1.max(turns); } else { e.0 = e.0.saturating_add(turns); }
        }
        None => *unkeyed = unkeyed.saturating_add(turns),
    }
}

/// Count provider model calls within `scope`, broken down by emitter family.
///
/// Events are filtered by `scope` first; per-`call_id` deduplication for delegated workers then
/// runs strictly over the matching events.
pub fn count_model_calls_breakdown(events: &[Event], scope: RunScope<'_>) -> ModelCallBreakdown {
    let mut out = ModelCallBreakdown::default();
    // Per `call_id`: `(observed_turns, reported_end_turns)`.
    let mut delegate_calls: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
    let mut explore_calls: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
    let mut unkeyed_delegated = 0u64;

    for ev in events {
        if !matches_scope(ev, scope) {
            continue;
        }
        match ev.kind.as_str() {
            "model.request" => out.main_loop = out.main_loop.saturating_add(1),
            "verifier.request" => out.verifier = out.verifier.saturating_add(1),
            "strategist.request" => out.strategist = out.strategist.saturating_add(1),
            "narrator.request" => out.narrator = out.narrator.saturating_add(1),
            IDEA_EXCHANGE_EVENT => out.exchange = out.exchange.saturating_add(1),
            "delegate.turn" => {
                add_call_turns(&mut delegate_calls, &mut unkeyed_delegated, &ev.payload, 1, false);
            }
            "delegate.end" => {
                let turns = payload_u64(&ev.payload, "turns");
                add_call_turns(&mut delegate_calls, &mut unkeyed_delegated, &ev.payload, turns, true);
            }
            "explore.branch" => {
                let turns = payload_u64(&ev.payload, "turns");
                add_call_turns(&mut explore_calls, &mut unkeyed_delegated, &ev.payload, turns, false);
            }
            "explore.end" => {
                let turns = payload_u64(&ev.payload, "turns");
                add_call_turns(&mut explore_calls, &mut unkeyed_delegated, &ev.payload, turns, true);
            }
            _ => {}
        }
    }

    let mut delegated = unkeyed_delegated;
    for &(turn_events, end_turns) in delegate_calls.values() {
        delegated = delegated.saturating_add(turn_events.max(end_turns));
    }
    for &(branch_turns, end_turns) in explore_calls.values() {
        delegated = delegated.saturating_add(branch_turns.max(end_turns));
    }
    out.delegated = delegated;
    out
}

/// Count total provider model calls within `scope` from ledger events.
pub fn count_model_calls(events: &[Event], scope: RunScope<'_>) -> u64 {
    count_model_calls_breakdown(events, scope).total()
}

/// Whether a run's model-call budget still has headroom or has reached its cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetStatus { WithinCap, AtCap }

/// Structured model-call budget ("N of at most M model calls").
/// Holds only counts (`cap`, `used`, `remaining`); UI prose belongs to `run_bar.js` (F6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCallBudget {
    pub cap: u64,
    pub used: u64,
    pub remaining: u64,
}

impl ModelCallBudget {
    pub fn new(cap: u64, used: u64) -> Self {
        Self { cap, used, remaining: cap.saturating_sub(used) }
    }

    pub fn from_events(events: &[Event], scope: RunScope<'_>, cap: u64) -> Self {
        Self::new(cap, count_model_calls(events, scope))
    }

    pub fn status(&self) -> BudgetStatus {
        if self.used >= self.cap { BudgetStatus::AtCap } else { BudgetStatus::WithinCap }
    }

    pub fn is_exhausted(&self) -> bool { self.used >= self.cap }

    pub fn can_spend(&self, calls: u64) -> bool { self.remaining >= calls }

    /// Return a new budget with `cap` raised by `extra_calls` (saturating) and `remaining`
    /// recomputed against `used`.
    pub fn with_raised_cap(&self, extra_calls: u64) -> Self {
        Self::new(self.cap.saturating_add(extra_calls), self.used)
    }
}

/// Primary user action paired with a non-active, non-terminal [`RunState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStateAction { Continue, Retry }

impl RunStateAction {
    pub fn label(self) -> &'static str {
        match self { Self::Continue => "Continue", Self::Retry => "Retry" }
    }
}

/// Shared seven-state run vocabulary across Investigate, Explore and Solve (overview 3.3).
/// "Partial" is a result summary, never a state: a partially completed run is [`RunState::Paused`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState { Running, WaitingForYou, Paused, Interrupted, Failed, Done, DoneWithOpenItems }

impl RunState {
    pub const ALL: [RunState; 7] = [
        Self::Running, Self::WaitingForYou, Self::Paused, Self::Interrupted,
        Self::Failed, Self::Done, Self::DoneWithOpenItems,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Running => RUN_STATE_LABEL_RUNNING,
            Self::WaitingForYou => RUN_STATE_LABEL_WAITING_FOR_YOU,
            Self::Paused => RUN_STATE_LABEL_PAUSED,
            Self::Interrupted => RUN_STATE_LABEL_INTERRUPTED,
            Self::Failed => RUN_STATE_LABEL_FAILED,
            Self::Done => RUN_STATE_LABEL_DONE,
            Self::DoneWithOpenItems => RUN_STATE_LABEL_DONE_WITH_OPEN_ITEMS,
        }
    }

    pub fn action(self) -> Option<RunStateAction> {
        match self {
            Self::Paused => Some(RunStateAction::Continue),
            Self::Interrupted | Self::Failed => Some(RunStateAction::Retry),
            Self::Running | Self::WaitingForYou | Self::Done | Self::DoneWithOpenItems => None,
        }
    }

    pub fn is_terminal(self) -> bool { matches!(self, Self::Done | Self::DoneWithOpenItems) }
    pub fn is_active(self) -> bool { matches!(self, Self::Running | Self::WaitingForYou) }
}

impl fmt::Display for RunState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(self.label()) }
}

/// Stored Investigate task states (`investigate.md` 4.4 and overview 3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvestigateTaskState { Requested, Approval, Partial, Stopped, Interrupted, Failed, Done }

impl InvestigateTaskState {
    pub const ALL: [InvestigateTaskState; 7] = [
        Self::Requested, Self::Approval, Self::Partial, Self::Stopped,
        Self::Interrupted, Self::Failed, Self::Done,
    ];

    pub fn from_stored(s: &str) -> Option<Self> {
        match s {
            "requested" => Some(Self::Requested),
            "approval" => Some(Self::Approval),
            "partial" => Some(Self::Partial),
            "stopped" => Some(Self::Stopped),
            "interrupted" => Some(Self::Interrupted),
            "failed" => Some(Self::Failed),
            "done" => Some(Self::Done),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Approval => "approval",
            Self::Partial => "partial",
            Self::Stopped => "stopped",
            Self::Interrupted => "interrupted",
            Self::Failed => "failed",
            Self::Done => "done",
        }
    }
}

/// Total mapping from an Investigate task's stored state to [`RunState`].
pub fn investigate_task_run_state(state: InvestigateTaskState) -> RunState {
    match state {
        InvestigateTaskState::Requested => RunState::Running,
        InvestigateTaskState::Approval => RunState::WaitingForYou,
        InvestigateTaskState::Partial | InvestigateTaskState::Stopped => RunState::Paused,
        InvestigateTaskState::Interrupted => RunState::Interrupted,
        InvestigateTaskState::Failed => RunState::Failed,
        InvestigateTaskState::Done => RunState::Done,
    }
}

impl From<InvestigateTaskState> for RunState {
    fn from(state: InvestigateTaskState) -> Self { investigate_task_run_state(state) }
}

/// Stored Explore stage states (`explore.md` 4.2 and overview 3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExploreStageState {
    Requested, PlanApproval, StoppedAtCap, Cancelled, Interrupted, Failed, Finished,
}

impl ExploreStageState {
    pub const ALL: [ExploreStageState; 7] = [
        Self::Requested, Self::PlanApproval, Self::StoppedAtCap, Self::Cancelled,
        Self::Interrupted, Self::Failed, Self::Finished,
    ];

    pub fn from_stored(s: &str) -> Option<Self> {
        match s {
            "requested" => Some(Self::Requested),
            "plan_approval" => Some(Self::PlanApproval),
            "stopped_at_cap" => Some(Self::StoppedAtCap),
            "cancelled" => Some(Self::Cancelled),
            "interrupted" => Some(Self::Interrupted),
            "failed" => Some(Self::Failed),
            "finished" => Some(Self::Finished),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::PlanApproval => "plan_approval",
            Self::StoppedAtCap => "stopped_at_cap",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
            Self::Failed => "failed",
            Self::Finished => "finished",
        }
    }
}

/// Total mapping from an Explore stage's stored state to [`RunState`].
pub fn explore_stage_run_state(state: ExploreStageState) -> RunState {
    match state {
        ExploreStageState::Requested => RunState::Running,
        ExploreStageState::PlanApproval => RunState::WaitingForYou,
        ExploreStageState::StoppedAtCap | ExploreStageState::Cancelled => RunState::Paused,
        ExploreStageState::Interrupted => RunState::Interrupted,
        ExploreStageState::Failed => RunState::Failed,
        ExploreStageState::Finished => RunState::Done,
    }
}

impl From<ExploreStageState> for RunState {
    fn from(state: ExploreStageState) -> Self { explore_stage_run_state(state) }
}

/// All nine `solve.paused` reasons (`solve.md` sections 4.1 and 5.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SolvePauseReason {
    LegCap, Budget, GateBounces, VerificationStuck, ApprovalNeeded,
    WorkerRestarted, User, Interrupted, TransportError,
}

impl SolvePauseReason {
    pub const ALL: [SolvePauseReason; 9] = [
        Self::LegCap, Self::Budget, Self::GateBounces, Self::VerificationStuck,
        Self::ApprovalNeeded, Self::WorkerRestarted, Self::User,
        Self::Interrupted, Self::TransportError,
    ];

    pub fn from_stored(s: &str) -> Option<Self> {
        match s {
            "leg_cap" => Some(Self::LegCap),
            "budget" => Some(Self::Budget),
            "gate_bounces" => Some(Self::GateBounces),
            "verification_stuck" => Some(Self::VerificationStuck),
            "approval_needed" => Some(Self::ApprovalNeeded),
            "worker_restarted" => Some(Self::WorkerRestarted),
            "user" => Some(Self::User),
            "interrupted" => Some(Self::Interrupted),
            "transport_error" => Some(Self::TransportError),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::LegCap => "leg_cap",
            Self::Budget => "budget",
            Self::GateBounces => "gate_bounces",
            Self::VerificationStuck => "verification_stuck",
            Self::ApprovalNeeded => "approval_needed",
            Self::WorkerRestarted => "worker_restarted",
            Self::User => "user",
            Self::Interrupted => "interrupted",
            Self::TransportError => "transport_error",
        }
    }
}

/// Total mapping from a `solve.paused` reason to [`RunState`].
pub fn solve_pause_run_state(reason: SolvePauseReason) -> RunState {
    match reason {
        SolvePauseReason::LegCap
        | SolvePauseReason::Budget
        | SolvePauseReason::GateBounces
        | SolvePauseReason::VerificationStuck
        | SolvePauseReason::WorkerRestarted
        | SolvePauseReason::User => RunState::Paused,
        SolvePauseReason::ApprovalNeeded => RunState::WaitingForYou,
        SolvePauseReason::Interrupted | SolvePauseReason::TransportError => RunState::Interrupted,
    }
}

impl From<SolvePauseReason> for RunState {
    fn from(reason: SolvePauseReason) -> Self { solve_pause_run_state(reason) }
}

/// Stored Solve run states (`solve.md` 5.5 and overview 3.3).
/// Solve runs never map to [`RunState::Failed`]; "Failed" in Solve is a step tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SolveRunState { LegRunning, Paused(SolvePauseReason), Finished, FinishedOpenItems }

/// Total mapping from a Solve run state to [`RunState`].
pub fn solve_run_state(state: SolveRunState) -> RunState {
    match state {
        SolveRunState::LegRunning => RunState::Running,
        SolveRunState::Paused(reason) => solve_pause_run_state(reason),
        SolveRunState::Finished => RunState::Done,
        SolveRunState::FinishedOpenItems => RunState::DoneWithOpenItems,
    }
}

impl From<SolveRunState> for RunState {
    fn from(state: SolveRunState) -> Self { solve_run_state(state) }
}

#[cfg(test)]
#[path = "run_budget_tests.rs"]
mod tests;
