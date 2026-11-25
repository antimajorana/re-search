use super::*;
use serde_json::json;

fn ev(seq: u64, kind: &str, payload: Value) -> Event {
    Event {
        session: "s-test".to_string(),
        seq,
        ts_ms: (seq as i64) * 1000,
        kind: kind.to_string(),
        schema_version: 1,
        payload,
        prev_hash: ledger::GENESIS_HASH.to_string(),
        event_hash: ledger::GENESIS_HASH.to_string(),
    }
}

#[test]
fn empty_events_count_is_zero() {
    let breakdown = count_model_calls_breakdown(&[], RunScope::All);
    assert_eq!(breakdown, ModelCallBreakdown::default());
    assert_eq!(breakdown.total(), 0);
    assert_eq!(count_model_calls(&[], RunScope::All), 0);
    assert_eq!(count_model_calls(&[], RunScope::AfterSeq(0)), 0);
    assert_eq!(
        count_model_calls(&[], RunScope::SeqRange { start_exclusive: 0, end_inclusive: 10 }),
        0
    );
    assert_eq!(count_model_calls(&[], RunScope::RunId("run-1")), 0);
}

#[test]
fn counts_model_request_only_not_response_or_error() {
    let events = vec![
        ev(1, "session.start", json!({})),
        // Normal turn: request + response = 1 provider call.
        ev(2, "model.request", json!({ "turn": 1 })),
        ev(3, "model.response", json!({ "turn": 1, "stop_reason": "tool_use" })),
        ev(4, "tool.request", json!({ "id": "t1" })),
        ev(5, "tool.result", json!({ "id": "t1" })),
        // Transport failure after request was sent: request + error = 1 provider call.
        ev(6, "model.request", json!({ "turn": 2 })),
        ev(7, "model.error", json!({ "turn": 2, "error": "connection reset" })),
        // prepare_request error before any model.request: 0 provider calls.
        ev(8, "model.error", json!({ "turn": 3, "error": "invalid payload" })),
        // Refusal follows model.request + model.response: still only 1 provider call.
        ev(9, "model.request", json!({ "turn": 4 })),
        ev(10, "model.response", json!({ "turn": 4, "stop_reason": "refusal" })),
        ev(11, "model.refusal", json!({ "turn": 4 })),
    ];

    let breakdown = count_model_calls_breakdown(&events, RunScope::All);
    assert_eq!(breakdown.main_loop, 3);
    assert_eq!(breakdown.total(), 3);
    assert_eq!(count_model_calls(&events, RunScope::All), 3);
}

#[test]
fn counts_mixed_emitter_families_exactly() {
    let events = vec![
        ev(1, "model.request", json!({ "turn": 1 })),
        ev(2, "model.response", json!({ "turn": 1 })),
        ev(3, "verifier.request", json!({ "kind": "step" })),
        ev(4, "verifier.response", json!({ "kind": "step" })),
        ev(5, "verifier.request", json!({ "kind": "full" })),
        ev(6, "strategist.request", json!({ "consultation_number": 1 })),
        ev(7, "narrator.request", json!({ "entry_count": 4 })),
        ev(8, IDEA_EXCHANGE_EVENT, json!({ "exchange": "sharpen", "attempt": 1 })),
        ev(9, IDEA_EXCHANGE_EVENT, json!({ "exchange": "sharpen", "attempt": 2 })),
        ev(10, "model.request", json!({ "turn": 2 })),
    ];

    let breakdown = count_model_calls_breakdown(&events, RunScope::All);
    assert_eq!(
        breakdown,
        ModelCallBreakdown {
            main_loop: 2,
            verifier: 2,
            strategist: 1,
            narrator: 1,
            delegated: 0,
            exchange: 2,
        }
    );
    assert_eq!(breakdown.total(), 8);
    assert_eq!(count_model_calls(&events, RunScope::All), 8);
}

#[test]
fn deduplicates_delegated_and_explore_worker_calls_per_call_id() {
    let events = vec![
        // Completed delegate call: 2 turn events + delegate.end(turns: 2) -> 2.
        ev(1, "delegate.start", json!({ "call_id": "c-ok" })),
        ev(2, "delegate.turn", json!({ "call_id": "c-ok", "turn": 1 })),
        ev(3, "delegate.turn", json!({ "call_id": "c-ok", "turn": 2 })),
        ev(4, "delegate.end", json!({ "call_id": "c-ok", "turns": 2, "ok": true })),
        // Failed final turn: 1 delegate.turn emitted before turn 2 errored, delegate.end reports 2 -> 2.
        ev(5, "delegate.turn", json!({ "call_id": "c-fail-last", "turn": 1 })),
        ev(6, "delegate.end", json!({ "call_id": "c-fail-last", "turns": 2, "ok": false })),
        // Mid-run delegate without delegate.end yet: 3 turn events -> 3.
        ev(7, "delegate.turn", json!({ "call_id": "c-live", "turn": 1 })),
        ev(8, "delegate.turn", json!({ "call_id": "c-live", "turn": 2 })),
        ev(9, "delegate.turn", json!({ "call_id": "c-live", "turn": 3 })),
        // Parallel explore with matching branch sum and end turns: (2 + 3) vs 5 -> 5.
        ev(10, "explore.start", json!({ "call_id": "e-ok" })),
        ev(11, "explore.branch", json!({ "call_id": "e-ok", "index": 0, "turns": 2 })),
        ev(12, "explore.branch", json!({ "call_id": "e-ok", "index": 1, "turns": 3 })),
        ev(13, "explore.end", json!({ "call_id": "e-ok", "branch_count": 2, "turns": 5 })),
        // Parallel explore where explore.end reports a higher turn count than branches -> 4.
        ev(14, "explore.branch", json!({ "call_id": "e-err", "index": 0, "turns": 2 })),
        ev(15, "explore.end", json!({ "call_id": "e-err", "branch_count": 2, "turns": 4 })),
        // Mid-run explore with one branch recorded and no explore.end yet -> 2.
        ev(16, "explore.branch", json!({ "call_id": "e-live", "index": 0, "turns": 2 })),
    ];

    let breakdown = count_model_calls_breakdown(&events, RunScope::All);
    assert_eq!(breakdown.delegated, 2 + 2 + 3 + 5 + 4 + 2);
    assert_eq!(breakdown.total(), 18);
}

#[test]
fn unkeyed_delegated_events_still_count_without_colliding() {
    let events = vec![
        ev(1, "delegate.end", json!({ "worker": "compute_worker", "turns": 2 })),
        ev(2, "delegate.end", json!({ "worker": "paper_reader", "turns": 3 })),
        ev(3, "explore.end", json!({ "branch_count": 2, "turns": 4 })),
        ev(4, "delegate.turn", json!({ "turn": 1 })),
    ];
    assert_eq!(count_model_calls_breakdown(&events, RunScope::All).delegated, 10);
}

#[test]
fn run_scope_filters_seq_and_run_id_accurately() {
    let events = vec![
        ev(1, "model.request", json!({ "run_id": "r1" })),
        ev(2, "verifier.request", json!({ "run_id": "r1" })),
        ev(3, "delegate.turn", json!({ "call_id": "c1", "run_id": "r1" })),
        ev(4, "delegate.end", json!({ "call_id": "c1", "turns": 3, "run_id": "r2" })),
        ev(5, "model.request", json!({ "run_id": 42 })),
        ev(6, IDEA_EXCHANGE_EVENT, json!({ "run_id": 42 })),
        ev(7, "model.request", json!({})), // no run_id
        ev(8, "model.request", json!({ "run_id": -42 })), // negative number ignored
        ev(9, "model.request", json!({ "run_id": 42.5 })), // float ignored
        ev(10, "model.request", json!({ "run_id": true })), // non-string/non-u64 ignored
    ];

    // All: 6 model.request + 1 verifier.request + 1 idea.exchange + max(1, 3) for c1 = 11.
    assert_eq!(count_model_calls(&events, RunScope::All), 11);

    // AfterSeq(5): seq 6..=10 -> 1 idea.exchange + 4 model.request = 5.
    assert_eq!(count_model_calls(&events, RunScope::AfterSeq(5)), 5);
    assert_eq!(count_model_calls(&events, RunScope::AfterSeq(10)), 0);

    // SeqRange: 1 < seq <= 5 -> seq 2 (verifier), seq 3 (c1 turn=1), seq 4 (c1 end=3), seq 5 (model) = 5.
    assert_eq!(
        count_model_calls(&events, RunScope::SeqRange { start_exclusive: 1, end_inclusive: 5 }),
        5
    );
    // Inverted / empty SeqRange yields 0.
    assert_eq!(
        count_model_calls(&events, RunScope::SeqRange { start_exclusive: 5, end_inclusive: 5 }),
        0
    );
    assert_eq!(
        count_model_calls(&events, RunScope::SeqRange { start_exclusive: 6, end_inclusive: 2 }),
        0
    );

    // RunId("r1"): seq 1 (model), seq 2 (verifier), seq 3 (c1 turn=1; seq 4 belongs to r2 so excluded).
    let r1 = count_model_calls_breakdown(&events, RunScope::RunId("r1"));
    assert_eq!(r1.main_loop, 1);
    assert_eq!(r1.verifier, 1);
    assert_eq!(r1.delegated, 1);
    assert_eq!(r1.total(), 3);

    // RunId("r2"): only seq 4 (c1 end=3) matches.
    let r2 = count_model_calls_breakdown(&events, RunScope::RunId("r2"));
    assert_eq!(r2.delegated, 3);
    assert_eq!(r2.total(), 3);

    // RunId("42"): matches JSON unsigned integer 42 on seq 5 and seq 6, ignores -42, 42.5, missing.
    let r42 = count_model_calls_breakdown(&events, RunScope::RunId("42"));
    assert_eq!(r42.main_loop, 1);
    assert_eq!(r42.exchange, 1);
    assert_eq!(r42.total(), 2);

    // Non-canonical decimal with leading zero does not match JSON integer 42.
    assert_eq!(count_model_calls(&events, RunScope::RunId("042")), 0);
    assert_eq!(count_model_calls(&events, RunScope::RunId("unknown")), 0);
}

#[test]
fn model_call_budget_operations_and_serde() {
    let events = vec![
        ev(1, "model.request", json!({})),
        ev(2, "verifier.request", json!({})),
        ev(3, "model.request", json!({})),
    ];

    let b = ModelCallBudget::from_events(&events, RunScope::All, 10);
    assert_eq!(b, ModelCallBudget { cap: 10, used: 3, remaining: 7 });
    assert_eq!(b.status(), BudgetStatus::WithinCap);
    assert!(!b.is_exhausted());
    assert!(b.can_spend(0));
    assert!(b.can_spend(7));
    assert!(!b.can_spend(8));

    let raised = b.with_raised_cap(5);
    assert_eq!(raised, ModelCallBudget { cap: 15, used: 3, remaining: 12 });
    assert!(raised.can_spend(12));

    // Exactly at cap.
    let at_cap = ModelCallBudget::new(4, 4);
    assert_eq!(at_cap.remaining, 0);
    assert_eq!(at_cap.status(), BudgetStatus::AtCap);
    assert!(at_cap.is_exhausted());
    assert!(at_cap.can_spend(0));
    assert!(!at_cap.can_spend(1));

    // Used exceeds cap: remaining saturates at 0, raising cap accounts for actual used.
    let over = ModelCallBudget::new(5, 8);
    assert_eq!(over.remaining, 0);
    assert_eq!(over.status(), BudgetStatus::AtCap);
    assert!(over.is_exhausted());
    assert!(!over.can_spend(1));
    assert_eq!(over.with_raised_cap(2), ModelCallBudget { cap: 7, used: 8, remaining: 0 });
    assert_eq!(over.with_raised_cap(5), ModelCallBudget { cap: 10, used: 8, remaining: 2 });

    // Zero cap is immediately exhausted.
    let zero = ModelCallBudget::new(0, 0);
    assert_eq!(zero.remaining, 0);
    assert_eq!(zero.status(), BudgetStatus::AtCap);
    assert!(zero.is_exhausted());
    assert!(zero.can_spend(0));
    assert!(!zero.can_spend(1));

    // Overflow-safe cap raising.
    let max_cap = ModelCallBudget::new(u64::MAX - 1, 10).with_raised_cap(100);
    assert_eq!(max_cap.cap, u64::MAX);
    assert_eq!(max_cap.remaining, u64::MAX - 10);

    // JSON serde round-trip.
    let encoded = serde_json::to_string(&b).unwrap();
    let decoded: ModelCallBudget = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, b);
    assert_eq!(serde_json::to_value(BudgetStatus::WithinCap).unwrap(), json!("within_cap"));
    assert_eq!(serde_json::to_value(BudgetStatus::AtCap).unwrap(), json!("at_cap"));
    assert_eq!(
        serde_json::from_value::<BudgetStatus>(json!("within_cap")).unwrap(),
        BudgetStatus::WithinCap
    );
    assert_eq!(
        serde_json::from_value::<BudgetStatus>(json!("at_cap")).unwrap(),
        BudgetStatus::AtCap
    );
}

#[test]
fn run_state_vocabulary_labels_actions_flags_and_serde() {
    let expected = [
        (RunState::Running, "Running", "running", None, true, false),
        (RunState::WaitingForYou, "Waiting for you", "waiting_for_you", None, true, false),
        (RunState::Paused, "Paused", "paused", Some(RunStateAction::Continue), false, false),
        (RunState::Interrupted, "Interrupted", "interrupted", Some(RunStateAction::Retry), false, false),
        (RunState::Failed, "Failed", "failed", Some(RunStateAction::Retry), false, false),
        (RunState::Done, "Done", "done", None, false, true),
        (
            RunState::DoneWithOpenItems,
            "Done with open items",
            "done_with_open_items",
            None,
            false,
            true,
        ),
    ];

    assert_eq!(RunState::ALL.len(), expected.len());
    for ((state, label, wire, action, active, terminal), listed) in
        expected.into_iter().zip(RunState::ALL)
    {
        assert_eq!(state, listed);
        assert_eq!(state.label(), label);
        assert_eq!(state.to_string(), label);
        assert_eq!(state.action(), action);
        assert_eq!(state.is_active(), active);
        assert_eq!(state.is_terminal(), terminal);
        // "Partial" describes a result, never a run state (overview section 3.3).
        assert!(!state.label().to_ascii_lowercase().contains("partial"));
        assert_eq!(serde_json::to_value(state).unwrap(), json!(wire));
        assert_eq!(serde_json::from_value::<RunState>(json!(wire)).unwrap(), state);
    }

    assert_eq!(RunStateAction::Continue.label(), "Continue");
    assert_eq!(RunStateAction::Retry.label(), "Retry");
    assert_eq!(serde_json::to_value(RunStateAction::Continue).unwrap(), json!("continue"));
    assert_eq!(serde_json::to_value(RunStateAction::Retry).unwrap(), json!("retry"));
}

#[test]
fn investigate_task_states_map_totally_to_run_state() {
    let cases = [
        (InvestigateTaskState::Requested, "requested", RunState::Running),
        (InvestigateTaskState::Approval, "approval", RunState::WaitingForYou),
        (InvestigateTaskState::Partial, "partial", RunState::Paused),
        (InvestigateTaskState::Stopped, "stopped", RunState::Paused),
        (InvestigateTaskState::Interrupted, "interrupted", RunState::Interrupted),
        (InvestigateTaskState::Failed, "failed", RunState::Failed),
        (InvestigateTaskState::Done, "done", RunState::Done),
    ];

    assert_eq!(InvestigateTaskState::ALL.len(), cases.len());
    for ((state, stored, expected_run), listed) in cases.into_iter().zip(InvestigateTaskState::ALL)
    {
        assert_eq!(state, listed);
        assert_eq!(state.as_str(), stored);
        assert_eq!(InvestigateTaskState::from_stored(stored), Some(state));
        assert_eq!(investigate_task_run_state(state), expected_run);
        assert_eq!(RunState::from(state), expected_run);
        assert_eq!(serde_json::to_value(state).unwrap(), json!(stored));
        assert_eq!(serde_json::from_value::<InvestigateTaskState>(json!(stored)).unwrap(), state);
    }
    assert_eq!(InvestigateTaskState::from_stored(""), None);
    assert_eq!(InvestigateTaskState::from_stored("finished"), None);
}

#[test]
fn explore_stage_states_map_totally_to_run_state() {
    let cases = [
        (ExploreStageState::Requested, "requested", RunState::Running),
        (ExploreStageState::PlanApproval, "plan_approval", RunState::WaitingForYou),
        (ExploreStageState::StoppedAtCap, "stopped_at_cap", RunState::Paused),
        (ExploreStageState::Cancelled, "cancelled", RunState::Paused),
        (ExploreStageState::Interrupted, "interrupted", RunState::Interrupted),
        (ExploreStageState::Failed, "failed", RunState::Failed),
        (ExploreStageState::Finished, "finished", RunState::Done),
    ];

    assert_eq!(ExploreStageState::ALL.len(), cases.len());
    for ((state, stored, expected_run), listed) in cases.into_iter().zip(ExploreStageState::ALL) {
        assert_eq!(state, listed);
        assert_eq!(state.as_str(), stored);
        assert_eq!(ExploreStageState::from_stored(stored), Some(state));
        assert_eq!(explore_stage_run_state(state), expected_run);
        assert_eq!(RunState::from(state), expected_run);
        assert_eq!(serde_json::to_value(state).unwrap(), json!(stored));
        assert_eq!(serde_json::from_value::<ExploreStageState>(json!(stored)).unwrap(), state);
    }
    assert_eq!(ExploreStageState::from_stored(""), None);
    assert_eq!(ExploreStageState::from_stored("partial"), None);
}

#[test]
fn solve_pause_reasons_and_run_states_map_totally_and_never_fail() {
    let pause_cases = [
        (SolvePauseReason::LegCap, "leg_cap", RunState::Paused),
        (SolvePauseReason::Budget, "budget", RunState::Paused),
        (SolvePauseReason::GateBounces, "gate_bounces", RunState::Paused),
        (SolvePauseReason::VerificationStuck, "verification_stuck", RunState::Paused),
        (SolvePauseReason::ApprovalNeeded, "approval_needed", RunState::WaitingForYou),
        (SolvePauseReason::WorkerRestarted, "worker_restarted", RunState::Paused),
        (SolvePauseReason::User, "user", RunState::Paused),
        (SolvePauseReason::Interrupted, "interrupted", RunState::Interrupted),
        (SolvePauseReason::TransportError, "transport_error", RunState::Interrupted),
    ];

    assert_eq!(SolvePauseReason::ALL.len(), 9);
    assert_eq!(SolvePauseReason::ALL.len(), pause_cases.len());

    let mut all_solve_states = vec![
        (SolveRunState::LegRunning, RunState::Running),
        (SolveRunState::Finished, RunState::Done),
        (SolveRunState::FinishedOpenItems, RunState::DoneWithOpenItems),
    ];

    for ((reason, stored, expected_run), listed) in
        pause_cases.into_iter().zip(SolvePauseReason::ALL)
    {
        assert_eq!(reason, listed);
        assert_eq!(reason.as_str(), stored);
        assert_eq!(SolvePauseReason::from_stored(stored), Some(reason));
        assert_eq!(solve_pause_run_state(reason), expected_run);
        assert_eq!(RunState::from(reason), expected_run);
        assert_eq!(serde_json::to_value(reason).unwrap(), json!(stored));
        assert_eq!(serde_json::from_value::<SolvePauseReason>(json!(stored)).unwrap(), reason);
        all_solve_states.push((SolveRunState::Paused(reason), expected_run));
    }

    assert_eq!(SolvePauseReason::from_stored(""), None);
    assert_eq!(SolvePauseReason::from_stored("failed"), None);

    for (solve_state, expected_run) in all_solve_states {
        let mapped = solve_run_state(solve_state);
        assert_eq!(mapped, expected_run);
        assert_eq!(RunState::from(solve_state), expected_run);
        // Solve runs never use RunState::Failed ("Failed" is a step tier in Solve).
        assert_ne!(mapped, RunState::Failed);
    }
}
