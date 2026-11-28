//! Mandatory WP1 real-session replay regression test against sanitized fixture
//! `crates/research-tools/tests/fixtures/s-ce81c873ea592808.solve.jsonl`.

use ledger::Event;
use research_tools::proof_closure::closure_for_target;
use research_tools::solve_done::{evaluate_done_criteria, validate_solve_finish_preconditions};
use research_tools::solve_state::SolveState;
use research_tools::solve_status::{badge_for_step, wording, Tier};
use serde_json::Value;

fn load_replay_events() -> Vec<Event> {
    let raw = include_str!("fixtures/s-ce81c873ea592808.solve.jsonl");
    let mut events = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let obj: Value = serde_json::from_str(trimmed)
            .unwrap_or_else(|e| panic!("invalid JSONL on line {}: {e}", idx + 1));
        events.push(Event {
            session: "s-ce81c873ea592808".to_string(),
            seq: obj["seq"].as_u64().expect("seq"),
            ts_ms: obj["ts_ms"].as_i64().unwrap_or(0),
            kind: obj["kind"].as_str().expect("kind").to_string(),
            schema_version: 1,
            payload: obj["payload"].clone(),
            prev_hash: String::new(),
            event_hash: String::new(),
        });
    }
    events
}

#[test]
fn replay_session_blocks_vacuous_self_comparisons_and_invalidates_trivial_reviews() {
    let events = load_replay_events();

    // Replay up to seq 355 (right after e-6 self-comparison checks at seq 350/355, before seq 382 review).
    let pre_review_e6: Vec<Event> = events.iter().filter(|e| e.seq <= 355).cloned().collect();
    let state_355 = SolveState::from_events(&pre_review_e6);
    let e6_pre = state_355.tier("e-6").expect("e-6 tier at seq 355");
    assert!(
        !e6_pre.own.is_computed(),
        "e-6 self-comparisons at seq 331-355 must NEVER earn Verified/CheckedNumerically; got {:?}",
        e6_pre.own
    );
    assert_eq!(e6_pre.own, Tier::Unchecked);
    assert!(
        e6_pre
            .check_note
            .as_deref()
            .is_some_and(|n| n.contains("both sides identical")),
        "e-6 must surface trivial check_note; got {:?}",
        e6_pre.check_note
    );

    // Replay full session.
    let state = SolveState::from_events(&events);

    // Self-comparisons on e-6, e-8, e-9 NEVER earn Verified or CheckedNumerically.
    for eid in ["e-6", "e-8", "e-9"] {
        let tier = state.tier(eid).unwrap_or_else(|| panic!("missing tier for {eid}"));
        assert!(
            tier.own != Tier::Verified && tier.own != Tier::CheckedNumerically,
            "step `{eid}` self-comparison must NEVER earn a computed tier; got {:?}",
            tier.own
        );
    }

    // Historical reviews at seq 537 (e-8) and seq 538 (e-9) cited vacuous self-comparison checks,
    // so they must NOT grant ModelAgreed / ModelAgreedSameModel.
    for eid in ["e-8", "e-9"] {
        let tier = state.tier(eid).unwrap();
        assert_eq!(
            tier.own,
            Tier::Unchecked,
            "step `{eid}` review citing trivial checks must be invalidated -> Unchecked; got {:?}",
            tier.own
        );
    }
}

#[test]
fn replay_session_e3_has_check_note_and_no_symbolic_sublabel_while_unchecked() {
    let events = load_replay_events();
    let state = SolveState::from_events(&events);

    let e3_tier = state.tier("e-3").expect("e-3 tier");
    let e3_badge = badge_for_step(&state, "e-3").expect("e-3 badge");
    assert_eq!(e3_tier.own, Tier::Unchecked);
    assert_eq!(e3_tier.effective, Tier::Unchecked);
    assert!(
        !e3_tier.sublabels.iter().any(|s| s == wording::SUBLABEL_SYMBOLIC),
        "Unchecked step e-3 must NOT carry `symbolic` sublabel; got {:?}",
        e3_tier.sublabels
    );
    assert!(
        !e3_badge.sublabels.iter().any(|s| s == wording::SUBLABEL_SYMBOLIC),
        "Unchecked badge e-3 must NOT carry `symbolic` sublabel; got {:?}",
        e3_badge.sublabels
    );
    assert!(
        e3_tier
            .check_note
            .as_deref()
            .is_some_and(|n| n.contains("both sides identical on 4 of 5 claims")),
        "e-3 check_note should report its 4 of 5 identical-side claims; got {:?}",
        e3_tier.check_note
    );
    assert_eq!(e3_badge.check_note, e3_tier.check_note);
}

#[test]
fn replay_session_refuses_rerooted_second_finish_and_fails_tier_criteria() {
    let events = load_replay_events();
    let state = SolveState::from_events(&events);
    let setup = state.setup.as_ref().expect("setup");

    // Both historical solve.done records (seq 402 on e-7 and seq 562 on e-10) are preserved.
    assert_eq!(state.done_history.len(), 2);
    assert_eq!(state.done_history[0].final_entry_id, "e-7");
    assert_eq!(state.done_history[1].final_entry_id, "e-10");
    assert!(!state.done_history[1].summary_markdown.is_empty());
    assert!(!state.done_history[1].open_items.is_empty());

    // Pre-seq-562 state right before the second finish on e-10: e-7 is still active & outside closure.
    let pre_562: Vec<Event> = events.iter().filter(|e| e.seq < 562).cloned().collect();
    let state_pre_562 = SolveState::from_events(&pre_562);
    let err = validate_solve_finish_preconditions(
        &state_pre_562,
        "e-10",
        "Counterexample summary without overclaiming.",
    )
    .expect_err("second finish on e-10 must be refused while active e-7 is outside closure");
    assert!(
        err.contains("earlier final answer `e-7` is still active"),
        "expected active earlier final answer refusal, got: {err}"
    );

    // Replaying evaluation of the historical re-rooted solve.done at seq 562 (e-10):
    // FinalTierAtLeast and ClosureTierAtLeast MUST report met: false with `re_rooted_past_active_final`.
    let closure_e10 = closure_for_target(&state.entries, "e-10").expect("e-10 closure");
    let outcomes = evaluate_done_criteria(setup, &state, "e-10", &closure_e10).expect("criteria");

    let final_crit = outcomes
        .iter()
        .find(|c| c.id == "final_different_model_review")
        .expect("final_different_model_review");
    assert!(
        !final_crit.met,
        "re-rooted e-10 must NOT meet final_different_model_review: {final_crit:?}"
    );
    assert!(
        final_crit.evidence.get("re_rooted_past_active_final").is_some(),
        "evidence must include re_rooted_past_active_final: {:?}",
        final_crit.evidence
    );

    let closure_crit = outcomes
        .iter()
        .find(|c| c.id == "closure_model_agreed")
        .expect("closure_model_agreed");
    assert!(
        !closure_crit.met,
        "re-rooted e-10 must NOT meet closure_model_agreed: {closure_crit:?}"
    );
    assert!(
        closure_crit.evidence.get("re_rooted_past_active_final").is_some(),
        "evidence must include re_rooted_past_active_final: {:?}",
        closure_crit.evidence
    );
}
