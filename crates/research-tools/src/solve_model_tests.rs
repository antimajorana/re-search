use super::*;
use crate::paper_key::{Mode, NodeRef, PaperKey};
use crate::proof_dag::ledger_append_def;
use serde_json::json;

#[test]
fn handle_ref_validates_and_serializes_as_plain_string() {
    for raw in ["a@1", "V1_msbar@2", "_tmp_9@42", &format!("{}@1", "A".repeat(64))] {
        let h = HandleRef::parse(raw).unwrap();
        assert_eq!(h.to_string(), raw);
        let v = serde_json::to_value(&h).unwrap();
        assert_eq!(v, Value::String(raw.to_string()));
        let back: HandleRef = serde_json::from_value(v).unwrap();
        assert_eq!(back, h);
    }
    for bad in [
        "",
        "V1",
        "@1",
        "1a@1",
        "a-b@1",
        "a.b@1",
        "a@0",
        "a@01",
        "a@",
        "a@-1",
        "a@1@2",
        "α@1",
        &format!("{}@1", "A".repeat(65)),
    ] {
        assert!(HandleRef::parse(bad).is_err(), "{bad:?} should be rejected");
        assert!(
            serde_json::from_value::<HandleRef>(Value::String(bad.to_string())).is_err(),
            "{bad:?} should fail serde"
        );
    }
}

#[test]
fn step_ids_match_solve_prefix_and_positive_integers() {
    for good in ["e-1", "e-2", "e-10", "e-999"] {
        assert!(is_valid_step_id(good), "{good} should be valid");
        assert!(validate_step_id(good).is_ok());
    }
    for bad in ["", "e-", "e-0", "e-01", "e-1a", "e--1", "n-abc", "xe-1", "1"] {
        assert!(!is_valid_step_id(bad), "{bad:?} should be rejected");
        assert!(validate_step_id(bad).is_err());
    }
}

#[test]
fn step_entry_types_match_ledger_append_schema_exactly() {
    let def = ledger_append_def();
    let expected: Vec<String> = def.input_schema["properties"]["entry_type"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let actual: Vec<String> = StepEntryType::ALL.iter().map(|t| t.as_str().to_string()).collect();
    assert_eq!(actual, expected);
    for t in StepEntryType::ALL {
        let v = serde_json::to_value(t).unwrap();
        assert_eq!(v, Value::String(t.as_str().to_string()));
        assert_eq!(serde_json::from_value::<StepEntryType>(v).unwrap(), t);
    }
}

#[test]
fn solve_setup_and_node_ref_roundtrip_with_default_budgets() {
    let origin = NodeRef {
        session: "sess-1".into(),
        mode: Mode::Investigate,
        id: "n-abcdefghij".into(),
        paper: Some(crate::paper_key::PaperRef {
            key: PaperKey::from_arxiv("2103.01234").unwrap(),
            version: Some("v2".into()),
            ids: crate::paper_key::PaperIds {
                arxiv: Some("2103.01234".into()),
                ..Default::default()
            },
        }),
        node_fp: Some("deadbeef".into()),
    };
    let mut dims = BTreeMap::new();
    let mut phi_dim = BTreeMap::new();
    phi_dim.insert("mass".into(), json!(1));
    dims.insert("phi".into(), phi_dim);

    let symbols = vec![SymbolSpec { name: "phi".into(), assumptions: vec!["real".into()] }];
    let assumptions = vec![SetupAssumption { id: "a-1".into(), text: "Euclidean".into() }];
    let math_sha256 = compute_math_sha256(&symbols, &dims, &assumptions, "hbar = c = 1");

    let setup = SolveSetup {
        revision: 1,
        statement: "Compute one-loop V_1(phi)".into(),
        task_kind: TaskKind::Calculate,
        target: SolveTarget {
            description: "Effective potential".into(),
            form: TargetForm::ClosedForm,
            order: None,
            digits: None,
        },
        done: vec![
            DoneCriterion {
                id: "c1".into(),
                kind: DoneCriterionKind::FinalTierAtLeast,
                params: json!({"tier": "verified"}),
            },
            DoneCriterion {
                id: "c2".into(),
                kind: DoneCriterionKind::NamedCheckPassed,
                params: json!({"check": "dimensions", "on": "final"}),
            },
        ],
        conventions: "hbar = c = 1".into(),
        symbols,
        dimensions: dims,
        assumptions,
        given_source: None,
        sources: vec!["src-1".into()],
        roles: SolveRoles {
            solver: RoleModel { provider: "anthropic".into(), model: "m-solver".into() },
            verifier: RoleModel { provider: "openai".into(), model: "m-verifier".into() },
            compute: None,
        },
        budgets: SolveBudgets::default(),
        review: ReviewPolicy::ProseOnly,
        tool_profile: event_kinds::SOLVE_TOOL_PROFILE_V1.into(),
        origin: Some(origin),
        math_sha256,
    };

    assert_eq!(setup.budgets.max_model_calls, 60);
    assert_eq!(setup.budgets.max_wall_minutes, 30);
    assert_eq!(setup.budgets.max_model_calls_per_leg, 48);
    assert_eq!(setup.budgets.max_review_loops, 6);
    assert_eq!(setup.budgets.max_gate_bounces, 3);

    let val = serde_json::to_value(&setup).unwrap();
    assert_eq!(val["task_kind"], "calculate");
    assert_eq!(val["target"]["form"], "closed_form");
    assert_eq!(val["origin"]["mode"], "investigate");
    let back: SolveSetup = serde_json::from_value(val).unwrap();
    assert_eq!(back, setup);
}

#[test]
fn quote_ref_uses_start_line_and_end_line_field_names() {
    let qref = QuoteRef {
        source_id: "src-1".into(),
        path: "main.tex".into(),
        start_line: 42,
        end_line: 45,
        file_sha256: "a".repeat(64),
        quote_sha256: "b".repeat(64),
        evidence_seq: Some(17),
    };
    let val = serde_json::to_value(&qref).unwrap();
    let obj = val.as_object().unwrap();
    assert_eq!(obj.get("start_line"), Some(&json!(42)));
    assert_eq!(obj.get("end_line"), Some(&json!(45)));
    assert!(!obj.contains_key("first_line"));
    assert!(!obj.contains_key("last_line"));
    assert_eq!(serde_json::from_value::<QuoteRef>(val).unwrap(), qref);
}

#[test]
fn claim_and_entry_hashes_separate_math_from_prose() {
    let lhs = HandleRef::parse("I_d@1").unwrap();
    let rhs = HandleRef::parse("V1_series@1").unwrap();
    let c1 = Claim {
        form: ClaimForm::Series,
        lhs: lhs.clone(),
        rhs: rhs.clone(),
        relation: ClaimRelation::Eq,
        order: Some(SeriesOrder { variable: "eps".into(), point: "0".into(), n: 0 }),
        domain: Some("m > 0".into()),
        how: Some("Expand in eps".into()),
    };
    let mut c2 = c1.clone();
    c2.how = Some("Different prose explanation".into());

    let lookup = |h: &HandleRef| -> Option<&'static str> {
        match h.name.as_str() {
            "I_d" => Some("hash_lhs_1"),
            "V1_series" => Some("hash_rhs_1"),
            _ => None,
        }
    };
    let h1 = compute_claim_sha256(std::slice::from_ref(&c1), lookup).unwrap();
    let h2 = compute_claim_sha256(&[c2], lookup).unwrap();
    // Prose (`how`) does not change `claim_sha256`.
    assert_eq!(h1, h2);

    // Changing handle srepr_sha256 changes `claim_sha256`.
    let lookup_changed = |h: &HandleRef| -> Option<&'static str> {
        match h.name.as_str() {
            "I_d" => Some("hash_lhs_2"),
            "V1_series" => Some("hash_rhs_1"),
            _ => None,
        }
    };
    let h3 = compute_claim_sha256(std::slice::from_ref(&c1), lookup_changed).unwrap();
    assert_ne!(h1, h3);

    // Changing domain changes `claim_sha256`.
    let mut c_dom = c1;
    c_dom.domain = Some("m in R".into());
    let h_dom = compute_claim_sha256(&[c_dom], lookup).unwrap();
    assert_ne!(h1, h_dom);

    // Entry hash binds to prose as well as claim hash.
    let e1 = compute_entry_sha256(Some(&h1), "summary A", "details A");
    let e2 = compute_entry_sha256(Some(&h1), "summary B", "details A");
    assert_ne!(e1, e2);
}

#[test]
fn pause_reasons_and_done_statuses_roundtrip() {
    let reasons = [
        (PauseReason::LegCap, "leg_cap"),
        (PauseReason::Budget, "budget"),
        (PauseReason::GateBounces, "gate_bounces"),
        (PauseReason::VerificationStuck, "verification_stuck"),
        (PauseReason::ApprovalNeeded, "approval_needed"),
        (PauseReason::WorkerRestarted, "worker_restarted"),
        (PauseReason::User, "user"),
        (PauseReason::Interrupted, "interrupted"),
        (PauseReason::TransportError, "transport_error"),
    ];
    for (variant, wire) in reasons {
        let v = serde_json::to_value(variant).unwrap();
        assert_eq!(v, Value::String(wire.to_string()));
        assert_eq!(serde_json::from_value::<PauseReason>(v).unwrap(), variant);
    }
    assert_eq!(serde_json::to_value(SolveDoneStatus::Finished).unwrap(), "finished");
    assert_eq!(
        serde_json::to_value(SolveDoneStatus::FinishedOpenItems).unwrap(),
        "finished_open_items"
    );
}

#[test]
fn quote_ref_is_reexported_from_quote_core() {
    let f4_ref = crate::quote_core::QuoteRef {
        source_id: "src-f4".into(),
        path: "paper.tex".into(),
        start_line: 10,
        end_line: 14,
        file_sha256: "c".repeat(64),
        quote_sha256: "d".repeat(64),
        evidence_seq: Some(5),
    };
    let solve_ref: QuoteRef = f4_ref.clone();
    assert_eq!(solve_ref, f4_ref);
}

#[test]
fn solve_run_status_maps_onto_f9_solve_run_state_and_run_state() {
    use crate::run_budget::{RunState, SolvePauseReason, SolveRunState};

    assert_eq!(SolveRunStatus::Idle.to_solve_run_state(), None);
    assert_eq!(SolveRunStatus::Idle.to_run_state(), None);
    assert_eq!(
        SolveRunStatus::Running.to_solve_run_state(),
        Some(SolveRunState::LegRunning)
    );
    assert_eq!(SolveRunStatus::Running.to_run_state(), Some(RunState::Running));
    assert_eq!(
        SolveRunStatus::Done(SolveDoneStatus::Finished).to_run_state(),
        Some(RunState::Done)
    );
    assert_eq!(
        SolveRunStatus::Done(SolveDoneStatus::FinishedOpenItems).to_run_state(),
        Some(RunState::DoneWithOpenItems)
    );

    let pause_cases = [
        (PauseReason::LegCap, SolvePauseReason::LegCap, RunState::Paused),
        (PauseReason::Budget, SolvePauseReason::Budget, RunState::Paused),
        (PauseReason::GateBounces, SolvePauseReason::GateBounces, RunState::Paused),
        (PauseReason::VerificationStuck, SolvePauseReason::VerificationStuck, RunState::Paused),
        (PauseReason::ApprovalNeeded, SolvePauseReason::ApprovalNeeded, RunState::WaitingForYou),
        (PauseReason::WorkerRestarted, SolvePauseReason::WorkerRestarted, RunState::Paused),
        (PauseReason::User, SolvePauseReason::User, RunState::Paused),
        (PauseReason::Interrupted, SolvePauseReason::Interrupted, RunState::Interrupted),
        (PauseReason::TransportError, SolvePauseReason::TransportError, RunState::Interrupted),
    ];
    for (reason, expected_f9, expected_screen) in pause_cases {
        let converted: SolvePauseReason = reason.into();
        assert_eq!(converted, expected_f9);
        assert_eq!(PauseReason::from(converted), reason);
        assert_eq!(reason.as_str(), expected_f9.as_str());
        let status = SolveRunStatus::Paused(reason);
        assert_eq!(status.to_solve_run_state(), Some(SolveRunState::Paused(expected_f9)));
        assert_eq!(status.to_run_state(), Some(expected_screen));
        assert_ne!(status.to_run_state(), Some(RunState::Failed));
    }
}

