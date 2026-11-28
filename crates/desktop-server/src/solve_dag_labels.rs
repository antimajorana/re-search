//! Relabel `/api/sessions/{id}/dag` entries with Solve checking tiers (`R1-F1`).
//!
//! Pure projection over ledger events: non-Solve sessions are left untouched,
//! superseded entries remain `"superseded"`, and every active Solve entry gets its
//! authoritative S2 badge label in `status` plus the serialized effective `Tier`
//! in `solve_tier` so Solve steps never render as `"pending"` or green `"accepted"`.

use crate::session_mode::session_mode;
use ledger::Event;
use research_tools::paper_key::Mode;
use research_tools::solve_model::event_kinds;
use research_tools::solve_state::SolveState;
use research_tools::solve_status::{compute_all_badges, wording, Tier};
use serde_json::Value;

fn is_solve_session(events: &[Event]) -> bool {
    session_mode(events) == Mode::Solve
        || events.iter().any(|ev| ev.kind == event_kinds::SOLVE_SETUP)
}

fn is_superseded_entry(obj: &serde_json::Map<String, Value>) -> bool {
    obj.get("status").and_then(Value::as_str) == Some("superseded")
        || obj
            .get("superseded_by")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
}

fn tier_value(tier: Tier) -> Value {
    serde_json::to_value(tier).unwrap_or_else(|_| Value::String("unchecked".to_string()))
}

/// Replace non-superseded Proof DAG entry statuses with Solve tier labels and
/// attach `solve_tier` when `events` belong to a Solve session.
pub fn relabel_for_solve(dag: &mut Value, events: &[Event]) {
    if !is_solve_session(events) {
        return;
    }
    let state = SolveState::from_events(events);
    let badges = compute_all_badges(&state);
    let Some(entries) = dag.get_mut("entries").and_then(Value::as_array_mut) else {
        return;
    };
    for entry in entries {
        let Some(obj) = entry.as_object_mut() else {
            continue;
        };
        if is_superseded_entry(obj) {
            continue;
        }
        let entry_id = obj.get("entry_id").and_then(Value::as_str).unwrap_or("");
        let (label, tier) = match badges.get(entry_id) {
            Some(badge) => (badge.label.clone(), badge.effective),
            None => (wording::BADGE_UNCHECKED.to_string(), Tier::Unchecked),
        };
        obj.insert("status".to_string(), Value::String(label));
        obj.insert("solve_tier".to_string(), tier_value(tier));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use research_tools::solve_model::{
        compute_claim_sha256, compute_entry_sha256, compute_math_sha256, CheckOutcome,
        CheckRequester, Claim, ClaimForm, ClaimRelation, HandleRef, ReceiptRef, ReviewPolicy,
        RoleModel, SolveBudgets, SolveCheck, SolveExpr, SolveRoles, SolveSetup, SolveTarget,
        StepEntryType, StepOrigin, TargetForm, TaskKind,
    };
    use serde_json::json;
    use std::collections::BTreeMap;

    fn mk_event(seq: u64, kind: &str, payload: Value) -> Event {
        Event {
            session: "s-dag-unit".into(),
            seq,
            ts_ms: (seq as i64) * 100,
            kind: kind.into(),
            schema_version: 1,
            payload,
            prev_hash: "0".repeat(64),
            event_hash: "1".repeat(64),
        }
    }

    fn setup_event(seq: u64) -> (Event, String) {
        let math_sha256 = compute_math_sha256(&[], &BTreeMap::new(), &[], "");
        let setup = SolveSetup {
            revision: 1,
            statement: "Compute x + y".into(),
            task_kind: TaskKind::Calculate,
            target: SolveTarget {
                description: "sum".into(),
                form: TargetForm::ClosedForm,
                order: None,
                digits: None,
            },
            done: vec![],
            conventions: String::new(),
            symbols: vec![],
            dimensions: BTreeMap::new(),
            assumptions: vec![],
            given_source: None,
            sources: vec![],
            roles: SolveRoles {
                solver: RoleModel { provider: "anthropic".into(), model: "claude-sonnet".into() },
                verifier: RoleModel { provider: "openai".into(), model: "gpt-4.1".into() },
                compute: None,
            },
            budgets: SolveBudgets::default(),
            review: ReviewPolicy::ProseOnly,
            tool_profile: event_kinds::SOLVE_TOOL_PROFILE_V1.into(),
            origin: None,
            math_sha256: math_sha256.clone(),
        };
        (mk_event(seq, event_kinds::SOLVE_SETUP, serde_json::to_value(setup).unwrap()), math_sha256)
    }

    fn mk_expr(seq: u64, name: &str, sha: &str) -> (Event, HandleRef) {
        let expr = SolveExpr {
            name: name.into(),
            version: 1,
            srepr_blob: format!("blob-{name}"),
            srepr_sha256: sha.into(),
            latex: name.into(),
            str_preview: name.into(),
            free_symbols: vec![],
            produced_by: ReceiptRef { call_id: "c1".into(), result_seq: 3 },
            owner_entry: None,
            portable: true,
        };
        let handle = expr.handle().unwrap();
        (mk_event(seq, event_kinds::SOLVE_EXPR, serde_json::to_value(expr).unwrap()), handle)
    }

    fn mk_check(
        seq: u64,
        claim_sha: &str,
        entry_id: &str,
        math_sha: &str,
        outcome: CheckOutcome,
    ) -> Event {
        let chk = SolveCheck {
            check_id: format!("chk-{seq}"),
            claim_sha256: claim_sha.into(),
            claim_index: 0,
            entry_id: Some(entry_id.into()),
            math_sha256: math_sha.into(),
            name: "identity_symbolic".into(),
            registry_sha256: "reg-v1".into(),
            params: json!({}),
            outcome,
            asked_by: CheckRequester::Host,
            receipt: ReceiptRef { call_id: "c-chk".into(), result_seq: seq },
            evidence: json!({ "residual": "0" }),
            evidence_sha256: "ev-sha".into(),
        };
        mk_event(seq, event_kinds::SOLVE_CHECK, serde_json::to_value(chk).unwrap())
    }

    #[test]
    fn relabels_solve_tiers_and_preserves_superseded_entries() {
        let (setup_ev, math_sha) = setup_event(2);
        let (expr_a, h_a) = mk_expr(4, "A", "sha_a");
        let (expr_b, h_b) = mk_expr(5, "B", "sha_b");
        let (expr_c, h_c) = mk_expr(6, "C", "sha_c");
        let claim_ok = Claim {
            form: ClaimForm::Equation,
            lhs: h_a.clone(),
            rhs: h_b.clone(),
            relation: ClaimRelation::Eq,
            order: None,
            domain: None,
            how: None,
        };
        let claim_bad = Claim {
            form: ClaimForm::Equation,
            lhs: h_a,
            rhs: h_c,
            relation: ClaimRelation::Eq,
            order: None,
            domain: None,
            how: None,
        };
        let ok_claim_sha = compute_claim_sha256(std::slice::from_ref(&claim_ok), |h| {
            if h.name == "A" { Some("sha_a") } else { Some("sha_b") }
        })
        .unwrap();
        let bad_claim_sha = compute_claim_sha256(std::slice::from_ref(&claim_bad), |h| {
            if h.name == "A" { Some("sha_a") } else { Some("sha_c") }
        })
        .unwrap();

        let e_same_sha = compute_entry_sha256(None, "Same-model review", "details");
        let e_ver_sha = compute_entry_sha256(Some(&ok_claim_sha), "Exact identity", "details");
        let e_fail_sha = compute_entry_sha256(Some(&bad_claim_sha), "Broken identity", "details");
        let e_old_sha = compute_entry_sha256(None, "Old step", "details");
        let e_unck_sha = compute_entry_sha256(None, "Unchecked step", "details");

        let events = vec![
            mk_event(1, event_kinds::SOLVE_STARTED, json!({ "origin": "new" })),
            setup_ev,
            mk_event(3, event_kinds::TOOL_RESULT, json!({ "call_id": "c1", "ok": true, "result": {} })),
            expr_a,
            expr_b,
            expr_c,
            mk_event(7, event_kinds::PROOF_ENTRY, json!({
                "entry_id": "e-unck", "entry_type": StepEntryType::Derivation,
                "summary": "Unchecked step", "details": "details",
                "depends_on": [], "entry_sha256": e_unck_sha, "origin": StepOrigin::Solver
            })),
            mk_event(8, event_kinds::PROOF_ENTRY, json!({
                "entry_id": "e-same", "entry_type": StepEntryType::Derivation,
                "summary": "Same-model review", "details": "details",
                "depends_on": [], "entry_sha256": e_same_sha, "origin": StepOrigin::Solver
            })),
            mk_event(9, event_kinds::PROOF_VERIFICATION, json!({
                "target_entry_id": "e-same", "verdict": "accepted",
                "independence": "same_model", "entry_sha256": e_same_sha
            })),
            mk_event(10, event_kinds::PROOF_ENTRY, json!({
                "entry_id": "e-ver", "entry_type": StepEntryType::Result,
                "summary": "Exact identity", "details": "details",
                "depends_on": [], "claims": [claim_ok],
                "claim_sha256": ok_claim_sha, "entry_sha256": e_ver_sha, "origin": StepOrigin::Solver
            })),
            mk_check(11, &ok_claim_sha, "e-ver", &math_sha, CheckOutcome::Pass),
            mk_event(12, event_kinds::PROOF_ENTRY, json!({
                "entry_id": "e-fail", "entry_type": StepEntryType::Result,
                "summary": "Broken identity", "details": "details",
                "depends_on": [], "claims": [claim_bad],
                "claim_sha256": bad_claim_sha, "entry_sha256": e_fail_sha, "origin": StepOrigin::Solver
            })),
            mk_check(13, &bad_claim_sha, "e-fail", &math_sha, CheckOutcome::Fail),
            mk_event(14, event_kinds::PROOF_ENTRY, json!({
                "entry_id": "e-old", "entry_type": StepEntryType::Derivation,
                "summary": "Old step", "details": "details",
                "depends_on": [], "entry_sha256": e_old_sha, "origin": StepOrigin::Solver
            })),
            mk_event(15, event_kinds::PROOF_SUPERSEDE, json!({
                "entry_id": "e-old", "replaced_by": "e-ver", "reason": "replaced"
            })),
        ];

        let mut dag = json!({
            "session": "s-dag-unit",
            "entries": [
                { "entry_id": "e-unck", "status": "pending", "superseded_by": null },
                { "entry_id": "e-same", "status": "accepted", "superseded_by": null },
                { "entry_id": "e-ver", "status": "pending", "superseded_by": null },
                { "entry_id": "e-fail", "status": "pending", "superseded_by": null },
                { "entry_id": "e-old", "status": "superseded", "superseded_by": "e-ver" }
            ]
        });

        relabel_for_solve(&mut dag, &events);
        let entries = dag["entries"].as_array().unwrap();
        assert_eq!(entries[0]["status"], wording::BADGE_UNCHECKED);
        assert_eq!(entries[0]["solve_tier"], "unchecked");
        assert_eq!(entries[1]["status"], wording::BADGE_MODEL_AGREED_SAME_MODEL);
        assert_eq!(entries[1]["solve_tier"], "model_agreed_same_model");
        assert_eq!(entries[2]["status"], wording::BADGE_VERIFIED);
        assert_eq!(entries[2]["solve_tier"], "verified");
        assert_eq!(entries[3]["status"], wording::BADGE_FAILED);
        assert_eq!(entries[3]["solve_tier"], "failed");
        assert_eq!(entries[4]["status"], "superseded");
        assert_eq!(entries[4]["superseded_by"], "e-ver");
        assert!(entries[4].get("solve_tier").is_none());
    }

    #[test]
    fn leaves_non_solve_research_session_untouched() {
        let events = vec![
            mk_event(1, "session.start", json!({ "prompt": "Research question" })),
            mk_event(2, event_kinds::PROOF_ENTRY, json!({
                "entry_id": "e-1", "entry_type": "derivation", "summary": "Step", "details": "d"
            })),
            mk_event(3, event_kinds::PROOF_VERIFICATION, json!({
                "target_entry_id": "e-1", "verdict": "accepted"
            })),
        ];
        let mut dag = json!({
            "session": "s-research",
            "entries": [
                { "entry_id": "e-1", "status": "accepted", "superseded_by": null },
                { "entry_id": "e-2", "status": "pending", "superseded_by": null }
            ]
        });
        let before = dag.clone();
        relabel_for_solve(&mut dag, &events);
        assert_eq!(dag, before);
        assert!(dag["entries"][0].get("solve_tier").is_none());
        assert!(dag["entries"][1].get("solve_tier").is_none());
    }
}
