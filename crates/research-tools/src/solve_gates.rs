//! Solve runtime gates (`G1`, `G4`, `G5`, `G6`, `G7`) implementing [`tools::Gate`]
//! (`solve.md` 5.1, 5.4, 5.5, rules R11, R12, R15, R22; package S11).
//!
//! Hosted in `research-tools` so both in-core loops (`agent`) and `mcp-server` / `desktop-server`
//! can install the exact same `GateChain` without coupling `mcp-server` to `agent`.

use crate::policy_preset::{SOLVE_CHECK_CALLER, SOLVE_VERIFIER_CALLER};
use crate::run_budget::{count_model_calls, RunScope};
use crate::solve_checks::{check_run_handler, AutoCheckGate, CheckProcessRunner};
use crate::solve_defs::{EXPR_SHOW, SOLVE_FINISH, STEP_RECORD};
use crate::solve_done::validate_solve_finish_preconditions;
use crate::solve_model::{event_kinds, PauseReason, ReviewVerdict, SolvePaused};
use crate::solve_state::SolveState;
use crate::solve_steps::step_record_handler;
use crate::tool_profile::SolveCheckHost;
use crate::ResearchEnv;
use ledger::{Event, Ledger};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tools::{Gate, GateChain, GateDecision, GateRefusal, GateRequest, WorkerEpoch};

const DEFAULT_MAX_GATE_BOUNCES: u32 = 3;
const EVENT_SCHEMA_VERSION: u32 = 1;

/// Tools still allowed by Gate G1 once un-cited `sympy_eval` runs exceed 3 (`solve.md` 5.4).
pub const ALLOW_G1_CLEANUP_TOOLS: &[&str] = &[
    STEP_RECORD,
    EXPR_SHOW,
    "ledger_query",
    "ledger_note",
    "fs_read",
    "fs_list",
    "read_paper_lines",
    "search_paper_passages",
];

/// Pause condition detected by host gates G4, G5, or G6 (`solve.md` 5.4 & 5.5).
#[derive(Debug, Clone, PartialEq)]
pub struct PauseTrigger {
    pub reason: PauseReason,
    pub message: String,
    pub open_items: Vec<Value>,
}

/// Host router gate enforcing Solve rules G1, G4, G5, G6, and G7 before tool execution.
pub struct SolveGate {
    env: Arc<ResearchEnv>,
    run_start_seq: AtomicU64,
    leg_start_seq: AtomicU64,
    started_at: Instant,
    elapsed_fn: Option<Arc<dyn Fn() -> Duration + Send + Sync>>,
}

impl SolveGate {
    pub fn new(env: Arc<ResearchEnv>, run_start_seq: u64, leg_start_seq: u64) -> Self {
        Self {
            env,
            run_start_seq: AtomicU64::new(run_start_seq),
            leg_start_seq: AtomicU64::new(leg_start_seq),
            started_at: Instant::now(),
            elapsed_fn: None,
        }
    }

    pub fn with_elapsed_fn(mut self, f: Arc<dyn Fn() -> Duration + Send + Sync>) -> Self {
        self.elapsed_fn = Some(f);
        self
    }

    pub fn set_leg_start_seq(&self, seq: u64) {
        self.leg_start_seq.store(seq, Ordering::SeqCst);
    }

    pub fn elapsed(&self) -> Duration {
        match &self.elapsed_fn {
            Some(f) => f(),
            None => self.started_at.elapsed(),
        }
    }

    /// Inspect G5 -> G6 -> G4 pause triggers against the current session ledger.
    pub fn check_pause_trigger(&self, session: &str) -> Result<Option<PauseTrigger>, String> {
        let ledger = Ledger::open(&self.env.data_dir).map_err(|e| e.to_string())?;
        let events = ledger.read_session(session).map_err(|e| e.to_string())?;
        let state = SolveState::from_events(&events);
        Ok(inspect_pause_triggers(
            &state,
            &events,
            self.leg_start_seq.load(Ordering::SeqCst),
            self.run_start_seq.load(Ordering::SeqCst),
            self.elapsed(),
        ))
    }

    /// If G5, G6, or G4 is triggered, append `solve.paused` v1 and return the folded [`SolvePaused`].
    pub fn check_and_emit_pause(
        &self,
        session: &str,
        leg: u32,
        turn: u32,
    ) -> Result<Option<SolvePaused>, String> {
        let ledger = Ledger::open(&self.env.data_dir).map_err(|e| e.to_string())?;
        let events = ledger.read_session(session).map_err(|e| e.to_string())?;
        let state = SolveState::from_events(&events);
        let Some(trigger) = inspect_pause_triggers(
            &state,
            &events,
            self.leg_start_seq.load(Ordering::SeqCst),
            self.run_start_seq.load(Ordering::SeqCst),
            self.elapsed(),
        ) else {
            return Ok(None);
        };
        let used = count_model_calls(&events, RunScope::All) as u32;
        let paused = emit_solve_paused(&ledger, session, &trigger, leg, turn, used)?;
        Ok(Some(paused))
    }
}

impl Gate for SolveGate {
    fn before(&self, req: &GateRequest<'_>) -> GateDecision {
        if req.caller == SOLVE_CHECK_CALLER {
            return GateDecision::Allow;
        }
        let Ok(ledger) = Ledger::open(&self.env.data_dir) else {
            return GateDecision::Block(GateRefusal::new(
                "G0",
                "failed to open ledger for Solve gate inspection",
            ));
        };
        let Ok(events) = ledger.read_session(req.session) else {
            return GateDecision::Block(GateRefusal::new(
                "G0",
                "failed to read session ledger for Solve gate inspection",
            ));
        };
        let state = SolveState::from_events(&events);

        // G5 / G6 / G4 loop stop conditions hold back further model actions until paused & resumed.
        if !matches!(req.caller, "user" | SOLVE_CHECK_CALLER) {
            if let Some(trigger) = inspect_pause_triggers(
                &state,
                &events,
                self.leg_start_seq.load(Ordering::SeqCst),
                self.run_start_seq.load(Ordering::SeqCst),
                self.elapsed(),
            ) {
                let gate_id = match trigger.reason {
                    PauseReason::GateBounces => "G5",
                    PauseReason::VerificationStuck => "G6",
                    PauseReason::LegCap | PauseReason::Budget => "G4",
                    _ => "G4",
                };
                return GateDecision::Block(GateRefusal::new(gate_id, trigger.message));
            }
        }

        // G7: closure & Solve R22 wording check on `solve_finish`.
        if req.tool_id == SOLVE_FINISH {
            let final_id = req
                .args
                .get("final_entry_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("");
            let summary = req
                .args
                .get("summary_markdown")
                .and_then(Value::as_str)
                .unwrap_or("");
            if let Err(msg) = validate_solve_finish_preconditions(&state, final_id, summary) {
                return GateDecision::Block(GateRefusal::new("G7", msg));
            }
        }

        // G1: block 4th un-cited `sympy_eval` (and anything outside cleanup allowlist if >3 pending).
        if let Err(msg) = check_g1_pending_runs(&state, req.caller, req.tool_id) {
            return GateDecision::Block(GateRefusal::new("G1", msg));
        }

        GateDecision::Allow
    }
}

/// Evaluate pause triggers G5 -> G6 -> G4 in priority order.
pub fn inspect_pause_triggers(
    state: &SolveState,
    events: &[Event],
    leg_start_seq: u64,
    run_start_seq: u64,
    elapsed: Duration,
) -> Option<PauseTrigger> {
    detect_g5_gate_bounces(state)
        .or_else(|| detect_g6_verification_stuck(state, events))
        .or_else(|| detect_g4_budget(state, events, leg_start_seq, run_start_seq, elapsed))
}

/// G5 (`solve.md` 5.4, rule R12): `max_gate_bounces` consecutive gate refusals pause the run.
pub fn detect_g5_gate_bounces(state: &SolveState) -> Option<PauseTrigger> {
    let limit = state
        .setup
        .as_ref()
        .map(|s| s.budgets.max_gate_bounces)
        .unwrap_or(DEFAULT_MAX_GATE_BOUNCES)
        .max(1);
    if state.gate_bounces >= limit {
        Some(PauseTrigger {
            reason: PauseReason::GateBounces,
            message: format!(
                "{} consecutive guardrail refusals reached limit ({limit}); pause required (`gate_bounces`)",
                state.gate_bounces
            ),
            open_items: vec![json!({ "kind": "gate_bounces", "count": state.gate_bounces, "limit": limit })],
        })
    } else {
        None
    }
}

/// G6 (`solve.md` 5.4): 2 rejected reviews on the same step lineage since the last `verification_stuck` pause.
pub fn detect_g6_verification_stuck(state: &SolveState, events: &[Event]) -> Option<PauseTrigger> {
    let last_stuck_seq = events
        .iter()
        .rev()
        .find(|ev| {
            ev.kind == event_kinds::SOLVE_PAUSED
                && ev.payload.get("reason").and_then(Value::as_str) == Some("verification_stuck")
        })
        .map(|ev| ev.seq)
        .unwrap_or(0);

    let replaced_from: BTreeMap<&str, &str> = state
        .entries
        .iter()
        .filter_map(|e| e.superseded_by.as_deref().map(|rep| (rep, e.entry_id.as_str())))
        .collect();

    let lineage_root = |start: &str| -> String {
        let mut cur = start;
        let mut seen = BTreeSet::new();
        while let Some(&prev) = replaced_from.get(cur) {
            if !seen.insert(cur) {
                break;
            }
            cur = prev;
        }
        cur.to_string()
    };

    let mut rejections_by_lineage: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for rev in &state.reviews {
        if rev.seq <= last_stuck_seq || rev.verdict != ReviewVerdict::Rejected {
            continue;
        }
        let root = lineage_root(&rev.target_entry_id);
        rejections_by_lineage
            .entry(root)
            .or_default()
            .push(rev.target_entry_id.clone());
    }

    for (root, targets) in rejections_by_lineage {
        if targets.len() >= 2 {
            return Some(PauseTrigger {
                reason: PauseReason::VerificationStuck,
                message: format!(
                    "step lineage `{root}` was rejected {} times by verifier ({}); pause required (`verification_stuck`)",
                    targets.len(),
                    targets.join(" -> ")
                ),
                open_items: vec![json!({
                    "kind": "verification_stuck",
                    "lineage_root": root,
                    "rejected_targets": targets,
                })],
            });
        }
    }
    None
}

/// G4 (`solve.md` 5.4, 5.5, package F9): model calls per leg/run & wall-clock limits.
pub fn detect_g4_budget(
    state: &SolveState,
    events: &[Event],
    leg_start_seq: u64,
    run_start_seq: u64,
    elapsed: Duration,
) -> Option<PauseTrigger> {
    let budgets = state.setup.as_ref().map(|s| s.budgets).unwrap_or_default();
    let total_calls = count_model_calls(events, RunScope::AfterSeq(run_start_seq));
    if total_calls >= u64::from(budgets.max_model_calls) {
        return Some(PauseTrigger {
            reason: PauseReason::Budget,
            message: format!(
                "model call budget reached ({total_calls} / {}); pause required (`budget`)",
                budgets.max_model_calls
            ),
            open_items: vec![json!({
                "kind": "budget_model_calls",
                "used": total_calls,
                "max": budgets.max_model_calls,
            })],
        });
    }
    let wall_limit = Duration::from_secs(u64::from(budgets.max_wall_minutes) * 60);
    if elapsed >= wall_limit {
        return Some(PauseTrigger {
            reason: PauseReason::Budget,
            message: format!(
                "wall-clock budget reached ({}m / {}m); pause required (`budget`)",
                elapsed.as_secs() / 60,
                budgets.max_wall_minutes
            ),
            open_items: vec![json!({
                "kind": "budget_wall_minutes",
                "elapsed_secs": elapsed.as_secs(),
                "max_minutes": budgets.max_wall_minutes,
            })],
        });
    }
    let leg_calls = count_model_calls(events, RunScope::AfterSeq(leg_start_seq));
    if leg_calls >= u64::from(budgets.max_model_calls_per_leg) {
        return Some(PauseTrigger {
            reason: PauseReason::LegCap,
            message: format!(
                "per-leg model call cap reached ({leg_calls} / {}); checkpoint & pause required (`leg_cap`)",
                budgets.max_model_calls_per_leg
            ),
            open_items: vec![json!({
                "kind": "leg_cap",
                "used": leg_calls,
                "max": budgets.max_model_calls_per_leg,
            })],
        });
    }
    None
}

/// G1 (`solve.md` 5.4): record or discard exploratory `sympy_eval` runs before running a 4th.
pub fn check_g1_pending_runs(
    state: &SolveState,
    caller: &str,
    tool_id: &str,
) -> Result<(), String> {
    if matches!(caller, SOLVE_CHECK_CALLER | SOLVE_VERIFIER_CALLER | "user") {
        return Ok(());
    }
    let pending = state.pending_runs.len();
    let is_sympy = tool_id == "sympy_eval";
    let cleanup = ALLOW_G1_CLEANUP_TOOLS.contains(&tool_id);
    if (pending >= 3 && is_sympy) || (pending > 3 && !cleanup) {
        return Err(format!(
            "{pending} un-cited `sympy_eval` run(s) pending ({}); record one via `step_record` or discard via `ledger_note` (`\"discard:...\"`) before calling `{tool_id}`",
            state.pending_runs.join(", ")
        ));
    }
    Ok(())
}

/// Append `solve.paused` v1 to `ledger` for a triggered pause condition.
pub fn emit_solve_paused(
    ledger: &Ledger,
    session: &str,
    trigger: &PauseTrigger,
    leg: u32,
    turn: u32,
    model_calls_used: u32,
) -> Result<SolvePaused, String> {
    let payload = json!({
        "reason": trigger.reason,
        "leg": leg,
        "turn": turn,
        "model_calls_used": model_calls_used,
        "open_items": trigger.open_items,
    });
    let ev = ledger
        .append_with_hash(session, event_kinds::SOLVE_PAUSED, EVENT_SCHEMA_VERSION, &payload)
        .map_err(|e| format!("failed to write `solve.paused`: {e}"))?;
    Ok(SolvePaused {
        seq: ev.seq,
        reason: trigger.reason,
        leg,
        turn,
        model_calls_used,
        open_items: trigger.open_items.clone(),
    })
}

/// Convenience helper for S11/S12/MCP: wire `check_run`, `step_record`, and `solve_finish` into [`SolveCheckHost`].
pub fn solve_check_host_v1(
    check_epoch: Arc<dyn WorkerEpoch>,
    env: Arc<ResearchEnv>,
    runner: Arc<dyn CheckProcessRunner>,
) -> SolveCheckHost {
    SolveCheckHost::new(check_epoch)
        .with_check_handler(check_run_handler(Arc::clone(&env), runner))
        .with_step_record_handler(step_record_handler(Arc::clone(&env)))
        .with_solve_finish_handler(crate::solve_done::solve_finish_handler(env))
}

/// Build the composite Solve [`GateChain`] (`SolveGate` pre-hooks + `AutoCheckGate` post-hooks).
pub fn solve_gate_chain(
    env: Arc<ResearchEnv>,
    runner: Arc<dyn CheckProcessRunner>,
    run_start_seq: u64,
    leg_start_seq: u64,
) -> (Arc<SolveGate>, Arc<GateChain>) {
    let solve_gate = Arc::new(SolveGate::new(Arc::clone(&env), run_start_seq, leg_start_seq));
    let auto_check = Arc::new(AutoCheckGate::new(env, runner));
    let chain = Arc::new(GateChain::new(vec![
        solve_gate.clone() as Arc<dyn Gate>,
        auto_check as Arc<dyn Gate>,
    ]));
    (solve_gate, chain)
}

#[cfg(test)]
#[path = "solve_gates_tests.rs"]
mod tests;
