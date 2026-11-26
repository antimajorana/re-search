use super::*;
use ledger::Ledger;
use policy::{Decision, EffectClass, Policy};
use research_tools::investigation::{
    branch_def, conventions_def, def as map_def, summary_def, GOLDEN_BRANCH_7C36A0E,
    GOLDEN_CONVENTIONS_7C36A0E, GOLDEN_MAP_7C36A0E, GOLDEN_MAP_NO_ANSWER_INDEX_7C36A0E,
    GOLDEN_SUMMARY_7C36A0E, RECORD_PAPER_BRANCH, RECORD_PAPER_CONVENTIONS, RECORD_PAPER_MAP,
    RECORD_PAPER_SUMMARY,
};
use research_tools::tool_profile::{
    investigation_v1, research_v1, LEGACY_MAY_GAIN, MARKER_IDEA_STARTED,
    MARKER_INVESTIGATION_HANDOFF, MARKER_INVESTIGATION_STARTED, MARKER_SOLVE_STARTED,
    PROFILE_EXPLORE_V1, PROFILE_INVESTIGATION_V1, PROFILE_INVESTIGATION_V2, PROFILE_RESEARCH_V1,
    PROFILE_SOLVE_V1,
};
use serde_json::json;
use std::collections::BTreeMap;
use tools::{builtin, ApprovalScope, ToolDef};

/// Exact pre-F1 `check_tool_defs_drift` from commit `e1517477` (`crates/agent/src/resume.rs:191-244`),
/// retained here strictly as a test oracle for table-driven equivalence verification.
fn legacy_check_tool_defs_drift_oracle(
    session: &str,
    payload: &Value,
    expected_defs: &[ToolDef],
) -> Result<(), ResumeReject> {
    let recorded = payload
        .get("tool_defs")
        .and_then(Value::as_array)
        .ok_or_else(|| drift(format!("session `{session}` missing `tool_defs`")))?;
    let mut prior = recorded.iter().peekable();
    for exp in expected_defs {
        let same_id = prior.peek().and_then(|rec| rec["id"].as_str()) == Some(exp.id.as_str());
        if !same_id
            && matches!(
                exp.id.as_str(),
                "search_formal_library"
                    | "search_paper_passages"
                    | "read_paper_lines"
                    | "record_paper_map"
                    | "record_paper_summary"
                    | "record_paper_branch"
                    | "record_paper_conventions"
            )
            && exp.effect == EffectClass::ReadWorkspace
        {
            continue;
        }
        let hash = prior.next().and_then(|rec| rec["def_hash"].as_str());
        let compatible = same_id
            && (hash == Some(exp.def_hash().as_str())
                || {
                    let mut earlier = exp.clone();
                    earlier.id == "record_paper_map"
                        && earlier.input_schema["properties"]
                            .as_object_mut()
                            .is_some_and(|p| p.remove("answer_node_ids").is_some())
                        && hash == Some(earlier.def_hash().as_str())
                }
                || {
                    research_tools::investigation::is_current_recorder_def(exp)
                        && research_tools::investigation::earlier_defs(&exp.id)
                            .iter()
                            .any(|d| hash == Some(d.def_hash().as_str()))
                });
        if !compatible {
            return Err(drift(format!(
                "cannot resume session `{session}`: tool definition drift for `{}`",
                exp.id
            )));
        }
    }
    if prior.next().is_some() {
        return Err(drift(format!(
            "cannot resume session `{session}`: tools were removed or reordered"
        )));
    }
    Ok(())
}

fn recorded_json(defs: &[ToolDef]) -> Value {
    json!({
        "tool_defs": defs.iter().map(|d| json!({ "id": d.id, "def_hash": d.def_hash() })).collect::<Vec<_>>()
    })
}

fn recorded_pairs(pairs: &[(&str, &str)]) -> Value {
    json!({
        "tool_defs": pairs.iter().map(|(id, h)| json!({ "id": id, "def_hash": h })).collect::<Vec<_>>()
    })
}

fn dummy_event(seq: u64, kind: &str, payload: Value) -> Event {
    Event {
        session: "s-prof".to_string(),
        seq,
        ts_ms: 0,
        kind: kind.to_string(),
        schema_version: 1,
        payload,
        prev_hash: "0".repeat(64),
        event_hash: "0".repeat(64),
    }
}

#[test]
fn table_driven_equivalence_against_legacy_drift_oracle() {
    let r1 = research_v1();
    let i1 = investigation_v1();
    let full = r1.defs.clone();

    let mut map_no_answer_idx = map_def();
    map_no_answer_idx.input_schema["properties"]
        .as_object_mut()
        .unwrap()
        .remove("answer_node_ids");

    let mut full_without_all_7: Vec<ToolDef> = full
        .iter()
        .filter(|d| !LEGACY_MAY_GAIN.contains(&d.id.as_str()))
        .cloned()
        .collect();
    full_without_all_7.sort_by(|a, b| a.id.cmp(&b.id));

    let full_with_7c36a0e_recorders: Value = {
        let items: Vec<Value> = full
            .iter()
            .map(|d| {
                let h = match d.id.as_str() {
                    RECORD_PAPER_MAP => GOLDEN_MAP_7C36A0E.to_string(),
                    RECORD_PAPER_SUMMARY => GOLDEN_SUMMARY_7C36A0E.to_string(),
                    RECORD_PAPER_BRANCH => GOLDEN_BRANCH_7C36A0E.to_string(),
                    RECORD_PAPER_CONVENTIONS => GOLDEN_CONVENTIONS_7C36A0E.to_string(),
                    _ => d.def_hash(),
                };
                json!({ "id": d.id, "def_hash": h })
            })
            .collect();
        json!({ "tool_defs": items })
    };

    let mut wrong_effect_gain = research_tools::paper_lines::def();
    wrong_effect_gain.effect = EffectClass::WriteWorkspace;
    let mut expected_wrong_effect = vec![builtin::ledger_note_def(), wrong_effect_gain];
    expected_wrong_effect.sort_by(|a, b| a.id.cmp(&b.id));

    let mut unknown_read_tool = research_tools::paper_lines::def();
    unknown_read_tool.id = "unrelated_read_tool".into();
    let mut expected_unknown_gain = vec![builtin::ledger_note_def(), unknown_read_tool];
    expected_unknown_gain.sort_by(|a, b| a.id.cmp(&b.id));

    let mut v6_read_map_node = research_tools::map_defs::read_map_node_def();
    let mut expected_v6_gain = vec![builtin::ledger_note_def(), v6_read_map_node.clone()];
    expected_v6_gain.sort_by(|a, b| a.id.cmp(&b.id));
    v6_read_map_node.effect = EffectClass::WriteWorkspace;
    let mut expected_v6_wrong_effect = vec![builtin::ledger_note_def(), v6_read_map_node];
    expected_v6_wrong_effect.sort_by(|a, b| a.id.cmp(&b.id));

    let mut mutated_summary = summary_def();
    mutated_summary.input_schema["properties"]["what"]["type"] = json!("string");

    let mut mutated_sympy = builtin::sympy_eval_def();
    mutated_sympy.description.push_str(" tampered");

    let mut mutated_scope = builtin::ledger_note_def();
    mutated_scope.approval_scope = ApprovalScope::WorkerSession;

    let mut cases: Vec<(&str, Value, Vec<ToolDef>, bool)> = vec![
        ("exact full research/1 match", recorded_json(&full), full.clone(), true),
        ("exact builtin 2-tool match", recorded_json(&builtin::defs()), builtin::defs(), true),
        ("missing tool_defs key", json!({}), full.clone(), false),
        ("non-array tool_defs", json!({ "tool_defs": "bad" }), full.clone(), false),
        ("gain all 7 historical ReadWorkspace tools", recorded_json(&full_without_all_7), full.clone(), true),
        ("full set with 7c36a0e recorders", full_with_7c36a0e_recorders, full.clone(), true),
        (
            "current record_paper_map without answer_node_ids",
            recorded_json(std::slice::from_ref(&map_no_answer_idx)),
            vec![map_def()],
            true,
        ),
        (
            "7c36a0e record_paper_map without answer_node_ids",
            recorded_pairs(&[(RECORD_PAPER_MAP, GOLDEN_MAP_NO_ANSWER_INDEX_7C36A0E)]),
            vec![map_def()],
            true,
        ),
        (
            "7c36a0e all four recorders",
            recorded_pairs(&[
                (RECORD_PAPER_BRANCH, GOLDEN_BRANCH_7C36A0E),
                (RECORD_PAPER_CONVENTIONS, GOLDEN_CONVENTIONS_7C36A0E),
                (RECORD_PAPER_MAP, GOLDEN_MAP_7C36A0E),
                (RECORD_PAPER_SUMMARY, GOLDEN_SUMMARY_7C36A0E),
            ]),
            vec![branch_def(), conventions_def(), map_def(), summary_def()],
            true,
        ),
        (
            "gained id with non-ReadWorkspace effect rejected",
            recorded_json(&[builtin::ledger_note_def()]),
            expected_wrong_effect,
            false,
        ),
        (
            "untolerated ReadWorkspace tool unrelated_read_tool rejected",
            recorded_json(&[builtin::ledger_note_def()]),
            expected_unknown_gain,
            false,
        ),
        (
            "mutated expected summary rejected against 7c36a0e hash",
            recorded_pairs(&[(RECORD_PAPER_SUMMARY, GOLDEN_SUMMARY_7C36A0E)]),
            vec![mutated_summary],
            false,
        ),
        (
            "bogus recorder hash rejected",
            recorded_pairs(&[(RECORD_PAPER_MAP, &"0".repeat(64))]),
            vec![map_def()],
            false,
        ),
        (
            "reordered defs rejected",
            recorded_pairs(&[
                (RECORD_PAPER_MAP, GOLDEN_MAP_7C36A0E),
                (RECORD_PAPER_BRANCH, GOLDEN_BRANCH_7C36A0E),
            ]),
            vec![branch_def(), map_def()],
            false,
        ),
        (
            "removed tool rejected",
            recorded_json(&full),
            full[..full.len() - 1].to_vec(),
            false,
        ),
        (
            "mutated sympy_eval description rejected",
            recorded_json(&[builtin::sympy_eval_def()]),
            vec![mutated_sympy],
            false,
        ),
        (
            "mutated approval_scope rejected",
            recorded_json(&[builtin::ledger_note_def()]),
            vec![mutated_scope],
            false,
        ),
    ];

    // Also test gaining each of the 7 historical tools individually.
    for id in LEGACY_MAY_GAIN {
        let without_one: Vec<ToolDef> = full.iter().filter(|d| d.id != *id).cloned().collect();
        cases.push((id, recorded_json(&without_one), full.clone(), true));
    }

    for (name, recorded, expected, want_ok) in cases {
        let oracle_res = legacy_check_tool_defs_drift_oracle("s-eq", &recorded, &expected);
        assert_eq!(oracle_res.is_ok(), want_ok, "oracle disagreed with expectation on `{name}`");
        for profile in [&r1, &i1] {
            let new_res = check_tool_defs_drift_for_profile("s-eq", &recorded, &expected, profile);
            assert_eq!(
                new_res.is_ok(),
                oracle_res.is_ok(),
                "profile `{}` diverged from legacy oracle on `{name}`: new={new_res:?}, oracle={oracle_res:?}",
                profile.name
            );
        }
    }

    // V6 profile split: `research/1` stays frozen and rejects gaining `read_map_node`,
    // while `investigation/1` accepts gaining `read_map_node` ONLY with `ReadWorkspace` effect.
    let rec_note = recorded_json(&[builtin::ledger_note_def()]);
    assert!(check_tool_defs_drift_for_profile("s-eq", &rec_note, &expected_v6_gain, &r1).is_err());
    assert!(check_tool_defs_drift_for_profile("s-eq", &rec_note, &expected_v6_gain, &i1).is_ok());
    assert!(check_tool_defs_drift_for_profile("s-eq", &rec_note, &expected_v6_wrong_effect, &i1).is_err());
}

#[test]
fn non_canonical_map_without_answer_index_is_stricter_than_legacy_oracle() {
    // Documented fail-closed tightening: if `expected` itself has a non-canonical description
    // for `record_paper_map`, data-driven `earlier_defs` refuses even when `recorded` equals
    // `expected` minus `answer_node_ids`.
    let mut non_canon = map_def();
    non_canon.description.push_str(" (mutated)");
    let mut recorded_def = non_canon.clone();
    recorded_def.input_schema["properties"]
        .as_object_mut()
        .unwrap()
        .remove("answer_node_ids");
    let recorded = recorded_json(&[recorded_def]);

    assert!(legacy_check_tool_defs_drift_oracle("s-strict", &recorded, std::slice::from_ref(&non_canon)).is_ok());
    let err = check_tool_defs_drift_for_profile("s-strict", &recorded, &[non_canon], &investigation_v1()).unwrap_err();
    assert_eq!(err.kind, ResumeRejectKind::Drift);
}

#[test]
fn resolve_session_profile_handles_explicit_legacy_unbuilt_and_r2_drift() {
    // Legacy research (no marker) -> research/1.
    let evs = [dummy_event(1, "session.start", json!({ "route": "api" }))];
    let r = resolve_session_profile("s-1", &evs).unwrap();
    assert_eq!(r.profile.name, PROFILE_RESEARCH_V1);
    assert!(!r.explicit);

    // Legacy investigation -> investigation/1.
    let evs = [
        dummy_event(1, MARKER_INVESTIGATION_STARTED, json!({})),
        dummy_event(2, "session.start", json!({ "route": "api" })),
    ];
    let r = resolve_session_profile("s-2", &evs).unwrap();
    assert_eq!(r.profile.name, PROFILE_INVESTIGATION_V1);
    assert!(!r.explicit);

    // Hand-off child -> research/1.
    let evs = [
        dummy_event(1, "source.ingested", json!({})),
        dummy_event(2, MARKER_INVESTIGATION_HANDOFF, json!({})),
        dummy_event(3, "session.start", json!({ "route": "api" })),
    ];
    let r = resolve_session_profile("s-3", &evs).unwrap();
    assert_eq!(r.profile.name, PROFILE_RESEARCH_V1);
    assert!(!r.explicit);

    // Built investigation/2 resolves cleanly as an explicit profile.
    let evs_v2 = [dummy_event(1, "session.start", json!({ "tool_profile": PROFILE_INVESTIGATION_V2 }))];
    let r_v2 = resolve_session_profile("s-inv2", &evs_v2).unwrap();
    assert_eq!(r_v2.profile.name, PROFILE_INVESTIGATION_V2);
    assert!(r_v2.explicit);

    // Reserved / unbuilt profiles fail closed with Drift ("profile not built yet").
    for name in [PROFILE_EXPLORE_V1] {
        let evs = [dummy_event(1, "session.start", json!({ "tool_profile": name }))];
        let err = resolve_session_profile("s-res", &evs).unwrap_err();
        assert_eq!(err.kind, ResumeRejectKind::Drift);
        assert!(err.message.contains("profile not built yet"), "{}", err.message);
    }

    // Explicit `solve/1` and `solve.started` now resolve to built profile `solve/1` (`v0.5-solve`).
    let solve_explicit = [dummy_event(1, "session.start", json!({ "tool_profile": PROFILE_SOLVE_V1 }))];
    let r = resolve_session_profile("s-solve-exp", &solve_explicit).unwrap();
    assert_eq!(r.profile.name, PROFILE_SOLVE_V1);
    assert_eq!(r.profile.policy.version, research_tools::SOLVE_POLICY_VERSION);
    assert!(r.explicit);

    let solve_marker = [
        dummy_event(1, MARKER_SOLVE_STARTED, json!({})),
        dummy_event(2, "session.start", json!({})),
    ];
    let r = resolve_session_profile("s-solve-m", &solve_marker).unwrap();
    assert_eq!(r.profile.name, PROFILE_SOLVE_V1);
    assert!(!r.explicit);

    // Legacy markers for unbuilt modes also fail closed.
    for marker in [MARKER_IDEA_STARTED] {
        let evs = [
            dummy_event(1, marker, json!({})),
            dummy_event(2, "session.start", json!({})),
        ];
        let err = resolve_session_profile("s-m", &evs).unwrap_err();
        assert_eq!(err.kind, ResumeRejectKind::Drift);
        assert!(err.message.contains("profile not built yet"), "{}", err.message);
    }

    // Non-string tool_profile fails closed.
    let evs = [dummy_event(1, "session.start", json!({ "tool_profile": null }))];
    assert_eq!(resolve_session_profile("s-null", &evs).unwrap_err().kind, ResumeRejectKind::Drift);

    // Rule R2: replay resume cannot change profile.
    let evs = [
        dummy_event(1, "session.start", json!({ "tool_profile": PROFILE_RESEARCH_V1 })),
        dummy_event(2, "session.resume", json!({ "tool_profile": PROFILE_INVESTIGATION_V1 })),
    ];
    let err = resolve_session_profile("s-r2", &evs).unwrap_err();
    assert_eq!(err.kind, ResumeRejectKind::Drift);
    assert!(err.message.contains("changed"), "{}", err.message);
}

#[test]
fn policy_belongs_to_profile_and_is_enforced_on_resume() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(dir.path()).unwrap();
    let r1 = research_v1();
    let defs_json: Vec<Value> = r1
        .defs
        .iter()
        .map(|d| json!({ "id": d.id, "def_hash": d.def_hash() }))
        .collect();

    // Session recording research/1 with a mismatched policy version must be rejected.
    ledger
        .append_with_hash(
            "s-bad-pol",
            "session.start",
            1,
            &json!({
                "route": "api",
                "provider": "anthropic-messages",
                "model": "claude-haiku-4-5",
                "prompt": "hi",
                "policy_version": "v0.2",
                "tool_profile": PROFILE_RESEARCH_V1,
                "tool_defs": defs_json,
            }),
        )
        .unwrap();
    let err = inspect_session_with_profile(&ledger, "s-bad-pol").unwrap_err();
    assert_eq!(err.kind, ResumeRejectKind::Drift);
    assert!(err.message.contains("policy_version"), "{}", err.message);

    // Session recording research/1 with matching policy passes inspect_session_with_profile,
    // and fails if a caller passes a different active policy or router profile.
    ledger
        .append_with_hash(
            "s-good",
            "session.start",
            1,
            &json!({
                "route": "api",
                "provider": "anthropic-messages",
                "model": "claude-haiku-4-5",
                "prompt": "hi",
                "policy_version": r1.policy.version,
                "tool_profile": PROFILE_RESEARCH_V1,
                "tool_defs": defs_json,
            }),
        )
        .unwrap();
    let (insp, resolved) = inspect_session_with_profile(&ledger, "s-good").unwrap();
    assert_eq!(resolved.profile.name, PROFILE_RESEARCH_V1);
    assert!(resolved.explicit);
    assert_eq!(insp.tool_profile, PROFILE_RESEARCH_V1);

    // Wrong active policy version -> Drift.
    let err = crate::resume::inspect_session_for_resume(&ledger, "s-good", "v9.9-wrong", &r1.defs).unwrap_err();
    assert_eq!(err.kind, ResumeRejectKind::Drift);

    // Wrong active router profile -> Drift (Rule R2).
    let err = verify_and_read_with_profile(
        &ledger,
        "s-good",
        &r1.policy.version,
        &r1.defs,
        Some(PROFILE_INVESTIGATION_V1),
    )
    .unwrap_err();
    assert_eq!(err.kind, ResumeRejectKind::Drift);
    assert!(err.message.contains("active router profile"), "{}", err.message);

    // Custom profile carries its own policy version and drift table independently.
    let custom = ToolProfile {
        name: "custom/1",
        defs: vec![builtin::ledger_note_def()],
        may_gain: Vec::new(),
        earlier_defs: BTreeMap::new(),
        policy: Policy {
            version: "v9.0-custom".to_string(),
            rules: Vec::new(),
            default: Decision::Deny,
        },
    };
    assert_eq!(custom.policy.version, "v9.0-custom");
    assert!(check_tool_defs_drift_for_profile(
        "s-custom",
        &recorded_json(&[builtin::ledger_note_def()]),
        &custom.defs,
        &custom,
    )
    .is_ok());

    // `solve/1` binds its own `v0.5-solve` policy and 16 frozen tool defs on resume.
    let s1 = research_tools::tool_profile::solve_v1();
    let s1_defs: Vec<Value> = s1.defs.iter().map(|d| json!({ "id": d.id, "def_hash": d.def_hash() })).collect();
    ledger
        .append_with_hash(
            "s-solve-ok",
            "session.start",
            1,
            &json!({
                "route": "api",
                "provider": "anthropic-messages",
                "model": "claude-haiku-4-5",
                "prompt": "solve",
                "policy_version": s1.policy.version,
                "tool_profile": PROFILE_SOLVE_V1,
                "tool_defs": s1_defs,
            }),
        )
        .unwrap();
    let (insp, resolved) = inspect_session_with_profile(&ledger, "s-solve-ok").unwrap();
    assert_eq!(resolved.profile.name, PROFILE_SOLVE_V1);
    assert_eq!(insp.tool_profile, PROFILE_SOLVE_V1);
    let err = crate::resume::inspect_session_for_resume(
        &ledger,
        "s-solve-ok",
        research_tools::RESEARCH_POLICY_VERSION,
        &s1.defs,
    )
    .unwrap_err();
    assert_eq!(err.kind, ResumeRejectKind::Drift);
}
