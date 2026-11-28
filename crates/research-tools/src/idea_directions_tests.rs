//! Unit tests for `idea_directions` (package X13: Rules D1, D2, V8, no `lean`, Solve field alignment).

use super::*;
use crate::idea_map::{MapNodeStatus, RelationToIdea, EVENT_IDEA_MAP};
use crate::quote_core::EVIDENCE_QUOTE_EVENT;
use ledger::Event;
use serde_json::json;

fn ev(seq: u64, kind: &str, payload: Value) -> Event {
    Event {
        session: "s-dir-test".into(),
        seq,
        ts_ms: 1000 + seq as i64,
        kind: kind.into(),
        schema_version: 1,
        payload,
        prev_hash: "0".repeat(64),
        event_hash: "a".repeat(64),
    }
}

fn sample_events() -> Vec<Event> {
    vec![
        ev(1, "idea.stated", json!({ "rev": 1, "text": "Lanczos growth in monitored chain" })),
        ev(
            2,
            EVIDENCE_QUOTE_EVENT,
            json!({
                "paper_key": "arxiv:2405.01111",
                "quote": "Lanczos coefficients grow linearly in the free chain.",
                "role": "states_result"
            }),
        ),
        ev(
            3,
            EVIDENCE_QUOTE_EVENT,
            json!({
                "paper_key": "arxiv:2405.01111",
                "quote": "It remains open how bi-Lanczos behaves under projective monitoring.",
                "role": "poses_open_question"
            }),
        ),
        ev(
            4,
            EVENT_IDEA_MAP,
            json!({
                "idea_rev": 1,
                "nodes": [
                    {
                        "id": "k-1",
                        "kind": "known_result",
                        "rev": 1,
                        "label": "Linear Lanczos growth",
                        "statement": "Linear growth without measurements.",
                        "evidence": ["xe-2"],
                        "relation_to_idea": RelationToIdea::SpecialCase,
                        "subclaim_ids": ["sc-1"],
                        "papers": ["arxiv:2405.01111"],
                        "status": MapNodeStatus::Active,
                        "identity_key": "xe-2"
                    },
                    {
                        "id": "q-1",
                        "kind": "open_question",
                        "rev": 1,
                        "label": "Projective monitoring effect",
                        "statement": "Authors ask how monitoring alters Lanczos growth.",
                        "evidence": ["xe-3"],
                        "relation_to_idea": RelationToIdea::Same,
                        "subclaim_ids": ["sc-1"],
                        "papers": ["arxiv:2405.01111"],
                        "status": MapNodeStatus::Active,
                        "identity_key": "xe-3"
                    },
                    {
                        "id": "w-1",
                        "kind": "related_work",
                        "rev": 1,
                        "label": "Abstract-only paper",
                        "statement": "Mentioned in abstract only.",
                        "evidence": ["xa-9"],
                        "relation_to_idea": RelationToIdea::Background,
                        "subclaim_ids": [],
                        "papers": ["arxiv:2405.02222"],
                        "status": MapNodeStatus::Active,
                        "identity_key": "xa-9"
                    }
                ],
                "edges": [],
                "overlaps": [],
                "dropped": []
            }),
        ),
    ]
}

fn valid_concrete_direction() -> Value {
    json!({
        "title": "Bi-Lanczos coefficients along the no-click trajectory",
        "question": "Does the bi-Lanczos slope separate the volume-law and area-law phases?",
        "known": [
            { "text": "Unmonitored chain shows linear growth", "node_ids": ["k-1"] }
        ],
        "missing": [
            { "text": "Monitored bi-Lanczos scaling across transition", "basis": "open_question(q-1)" },
            { "text": "Finite-size crossover exponent", "basis": "no_evidence_in_scope" }
        ],
        "first_calculation": {
            "setup": "For the quadratic no-click effective Hamiltonian on L = 64..256 sites, build the single-particle Krylov basis and compute b_n for the first calculation.",
            "inputs": ["k-1"],
            "method": "numeric",
            "expected_form": "Numeric table of b_n slopes across measurement rate gamma",
            "check": {
                "text": "Zero-measurement limit must reproduce the linear slope of k-1",
                "reproduces": "k-1"
            },
            "effort": "days"
        },
        "risks": [
            { "text": "No-click trajectory may differ from averaged trajectories", "kind": "technical", "node_ids": ["k-1"] }
        ],
        "stop_if": "Slopes coincide within error bars on both sides at L = 256",
        "solve_setup": {
            "statement": "Compute single-particle bi-Lanczos coefficients b_n for the monitored Majorana chain.",
            "task_kind": "calculate",
            "target": {
                "description": "Numeric slope alpha(gamma) of b_n vs n",
                "form": "numeric"
            },
            "conventions": "Natural units hbar = 1, periodic boundary conditions",
            "source_keys": ["arxiv:2405.01111"]
        }
    })
}

#[test]
fn idea_direction_concreteness_is_computed_and_allows_first_calculation() {
    let evs = sample_events();
    let concrete = valid_concrete_direction();

    // 1. Concrete direction with "first calculation" prose passes Rule V8 and computes Concrete.
    let res = validate_record_idea_directions(&evs, Some(5), &json!({ "directions": [concrete] })).unwrap();
    assert_eq!(res.map_seq, 4);
    assert_eq!(res.directions.len(), 1);
    assert_eq!(res.directions[0].id, "r-1");
    assert_eq!(res.directions[0].concreteness, Concreteness::Concrete);
    assert!(res.directions[0].concreteness_reasons.is_empty());
    assert_eq!(res.directions[0].solve_setup.task_kind, TaskKind::Calculate);
    assert_eq!(res.directions[0].solve_setup.target.form, TargetForm::Numeric);

    // 2. Model cannot forge `concreteness` on input (`deny_unknown_fields`).
    let mut forged = valid_concrete_direction();
    forged["concreteness"] = json!("concrete");
    assert!(validate_record_idea_directions(&evs, None, &json!({ "directions": [forged] })).is_err());

    // 3. Each missing D2 condition downgrades Concrete -> Sketch (kept under Loose ideas).
    // (a) Known points only to abstract-only node w-1 (no full-text xe evidence).
    let mut no_xe = valid_concrete_direction();
    no_xe["known"] = json!([{ "text": "Abstract hint", "node_ids": ["w-1"] }]);
    let r_no_xe = validate_record_idea_directions(&evs, None, &json!({ "directions": [no_xe] })).unwrap();
    assert_eq!(r_no_xe.directions[0].concreteness, Concreteness::Sketch);

    // (b) Setup shorter than 40 chars.
    let mut short_setup = valid_concrete_direction();
    short_setup["first_calculation"]["setup"] = json!("Too short setup");
    let r_short = validate_record_idea_directions(&evs, None, &json!({ "directions": [short_setup] })).unwrap();
    assert_eq!(r_short.directions[0].concreteness, Concreteness::Sketch);

    // (c) Empty inputs.
    let mut no_inputs = valid_concrete_direction();
    no_inputs["first_calculation"]["inputs"] = json!([]);
    let r_inputs = validate_record_idea_directions(&evs, None, &json!({ "directions": [no_inputs] })).unwrap();
    assert_eq!(r_inputs.directions[0].concreteness, Concreteness::Sketch);

    // (d) Empty check text.
    let mut no_check = valid_concrete_direction();
    no_check["first_calculation"]["check"]["text"] = json!("");
    let r_check = validate_record_idea_directions(&evs, None, &json!({ "directions": [no_check] })).unwrap();
    assert_eq!(r_check.directions[0].concreteness, Concreteness::Sketch);

    // (e) Empty risks.
    let mut no_risk = valid_concrete_direction();
    no_risk["risks"] = json!([]);
    let r_risk = validate_record_idea_directions(&evs, None, &json!({ "directions": [no_risk] })).unwrap();
    assert_eq!(r_risk.directions[0].concreteness, Concreteness::Sketch);

    // (f) Empty stop_if.
    let mut no_stop = valid_concrete_direction();
    no_stop["stop_if"] = json!("");
    let r_stop = validate_record_idea_directions(&evs, None, &json!({ "directions": [no_stop] })).unwrap();
    assert_eq!(r_stop.directions[0].concreteness, Concreteness::Sketch);
}

#[test]
fn idea_direction_rejects_bad_refs_missing_basis_banned_phrases_and_lean() {
    let evs = sample_events();

    // Unknown node id in known[].node_ids -> rejected (Rule D1).
    let mut bad_known = valid_concrete_direction();
    bad_known["known"][0]["node_ids"] = json!(["k-999"]);
    assert!(validate_record_idea_directions(&evs, None, &json!({ "directions": [bad_known] })).is_err());

    // Unknown node id in first_calculation.inputs -> rejected (Rule D1).
    let mut bad_input = valid_concrete_direction();
    bad_input["first_calculation"]["inputs"] = json!(["d-404"]);
    assert!(validate_record_idea_directions(&evs, None, &json!({ "directions": [bad_input] })).is_err());

    // check.reproduces pointing at non-known_result (q-1) -> rejected (Rule D1).
    let mut bad_repro = valid_concrete_direction();
    bad_repro["first_calculation"]["check"]["reproduces"] = json!("q-1");
    assert!(validate_record_idea_directions(&evs, None, &json!({ "directions": [bad_repro] })).is_err());

    // missing without valid basis -> rejected (Rule D1).
    let mut bad_basis = valid_concrete_direction();
    bad_basis["missing"][0]["basis"] = json!("model_guess");
    assert!(validate_record_idea_directions(&evs, None, &json!({ "directions": [bad_basis] })).is_err());

    // missing pointing at nonexistent open_question(q-99) -> rejected (Rule D1).
    let mut bad_q = valid_concrete_direction();
    bad_q["missing"][0]["basis"] = json!("open_question(q-99)");
    assert!(validate_record_idea_directions(&evs, None, &json!({ "directions": [bad_q] })).is_err());

    // Banned Rule V8 absolute phrase -> rejected.
    let mut banned = valid_concrete_direction();
    banned["question"] = json!("This has never been computed and is novel.");
    assert!(validate_record_idea_directions(&evs, None, &json!({ "directions": [banned] })).is_err());

    // `lean` method strictly rejected (`00-overview.md` 0.1).
    let mut lean = valid_concrete_direction();
    lean["first_calculation"]["method"] = json!("lean");
    assert!(validate_record_idea_directions(&evs, None, &json!({ "directions": [lean] })).is_err());
    let schema_str = record_idea_directions_schema().to_string();
    assert!(!schema_str.contains("\"lean\""), "schema must never offer `lean`");

    // Old Explore name `check` instead of Solve's `check_derivation` rejected (`00-overview.md` 3.1 row 9).
    let mut old_kind = valid_concrete_direction();
    old_kind["solve_setup"]["task_kind"] = json!("check");
    assert!(validate_record_idea_directions(&evs, None, &json!({ "directions": [old_kind] })).is_err());

    // > 5 directions rejected (`explore.md` 5.1).
    let six = vec![valid_concrete_direction(); 6];
    assert!(validate_record_idea_directions(&evs, None, &json!({ "directions": six })).is_err());

    // Mixed batch keeps valid direction and reports rejected item.
    let mut bad_ref = valid_concrete_direction();
    bad_ref["risks"][0]["node_ids"] = json!(["k-404"]);
    let mixed = validate_record_idea_directions(
        &evs,
        None,
        &json!({ "directions": [valid_concrete_direction(), bad_ref] }),
    )
    .unwrap();
    assert_eq!(mixed.directions.len(), 1);
    assert_eq!(mixed.rejected.len(), 1);
    assert_eq!(mixed.rejected[0].index, 1);

    // Invalid open_question(q-0) rejected by MissingBasis parser.
    let mut bad_q0 = valid_concrete_direction();
    bad_q0["missing"][0]["basis"] = json!("open_question(q-0)");
    assert!(validate_record_idea_directions(&evs, None, &json!({ "directions": [bad_q0] })).is_err());
}

#[test]
fn idea_direction_rejects_not_in_current_map_and_prior_revision_xe_does_not_grant_concrete() {
    // 1. A node with status `not_in_current_map` (Rule S4 preserved) is rejected under Rule D1.
    let mut evs = sample_events();
    evs[3].payload["nodes"].as_array_mut().unwrap().push(json!({
        "id": "k-9",
        "kind": "known_result",
        "rev": 1,
        "label": "Dropped old node",
        "statement": "Preserved only because user noted it.",
        "evidence": ["xe-2"],
        "relation_to_idea": RelationToIdea::SpecialCase,
        "subclaim_ids": ["sc-1"],
        "papers": ["arxiv:2405.01111"],
        "status": MapNodeStatus::NotInCurrentMap,
        "identity_key": "xe-2-old"
    }));
    let mut cites_dropped = valid_concrete_direction();
    cites_dropped["known"] = json!([{ "text": "Old node", "node_ids": ["k-9"] }]);
    let err = validate_record_idea_directions(&evs, None, &json!({ "directions": [cites_dropped] })).unwrap_err();
    assert!(err.iter().any(|e| e.contains("not_in_current_map")), "expected not_in_current_map error: {err:?}");

    // 2. An `xe-N` recorded before the latest `idea.stated` revision does NOT grant Concrete.
    let mut rev2_evs = vec![
        ev(1, "idea.stated", json!({ "rev": 1, "text": "Old revision" })),
        ev(2, EVIDENCE_QUOTE_EVENT, json!({ "paper_key": "arxiv:2405.01111", "quote": "Old quote", "role": "states_result" })),
        ev(3, "idea.stated", json!({ "rev": 2, "text": "Revised idea" })),
        ev(4, EVENT_IDEA_MAP, evs[3].payload.clone()),
    ];
    // Remove k-9 from rev2 map; k-1 references xe-2 from rev 1 (before seq 3).
    rev2_evs[3].payload["nodes"].as_array_mut().unwrap().retain(|n| n["id"] != "k-9");
    let res = validate_record_idea_directions(&rev2_evs, None, &json!({ "directions": [valid_concrete_direction()] })).unwrap();
    assert_eq!(res.directions[0].concreteness, Concreteness::Sketch);
}
