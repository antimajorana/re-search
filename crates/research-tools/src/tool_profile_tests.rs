use super::*;
use crate::investigation::{
    GOLDEN_BRANCH_7C36A0E, GOLDEN_CONVENTIONS_7C36A0E, GOLDEN_MAP_7C36A0E,
    GOLDEN_MAP_NO_ANSWER_INDEX_7C36A0E, GOLDEN_SUMMARY_7C36A0E, RECORD_PAPER_BRANCH,
    RECORD_PAPER_CONVENTIONS, RECORD_PAPER_MAP, RECORD_PAPER_SUMMARY,
};
use serde_json::json;

struct DummyEpoch;

impl WorkerEpoch for DummyEpoch {
    fn live_epoch(&self) -> Option<u64> {
        None
    }

    fn is_contained(&self) -> bool {
        false
    }
}

fn dummy_event(seq: u64, kind: &str, payload: Value) -> Event {
    Event {
        session: "s-test".to_string(),
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
fn lookup_returns_built_profiles_and_clear_errors_for_reserved_or_unknown() {
    let r1 = lookup(PROFILE_RESEARCH_V1).expect("research/1 must exist");
    assert_eq!(r1.name, "research/1");
    assert_eq!(r1.policy.version, crate::RESEARCH_POLICY_VERSION);

    let i1 = lookup(PROFILE_INVESTIGATION_V1).expect("investigation/1 must exist");
    assert_eq!(i1.name, "investigation/1");
    assert_eq!(i1.policy.version, crate::RESEARCH_POLICY_VERSION);
    assert_eq!(r1.defs, i1.defs);

    for reserved in RESERVED_PROFILE_NAMES {
        let err = lookup(reserved).expect_err("reserved profile must return Err, not panic");
        assert_eq!(err, ProfileError::NotBuiltYet { name: (*reserved).to_string() });
        assert!(err.to_string().contains("profile not built yet"), "unexpected error: {err}");
    }

    let unknown = lookup("custom/99").unwrap_err();
    assert_eq!(unknown, ProfileError::Unknown { name: "custom/99".to_string() });
    assert!(unknown.to_string().contains("unknown tool profile"));
}

#[test]
fn research_v1_contains_all_research_tools_plus_host_tools_in_id_order() {
    let p = research_v1();
    let expected_len = all_research_tool_defs().len() + 2;
    assert_eq!(p.defs.len(), expected_len);
    for pair in p.defs.windows(2) {
        assert!(pair[0].id < pair[1].id, "not strictly sorted: {} >= {}", pair[0].id, pair[1].id);
    }
    assert!(p.def_for(builtin::LEDGER_NOTE).is_some());
    assert!(p.def_for(builtin::SYMPY_EVAL).is_some());
    assert!(p.def_for(builtin::SET_SESSION_TITLE).is_none());
    let mcp = p.mcp_defs();
    assert_eq!(mcp.len(), expected_len + 1);
    assert!(mcp.iter().any(|d| d.id == builtin::SET_SESSION_TITLE));
}

#[test]
fn investigation_v1_encodes_legacy_may_gain_and_earlier_defs_as_data() {
    let p = investigation_v1();
    assert_eq!(p.may_gain, LEGACY_MAY_GAIN);
    assert_eq!(p.may_gain.len(), 7);
    for id in LEGACY_MAY_GAIN {
        let def = p.def_for(id).unwrap_or_else(|| panic!("missing {id}"));
        assert!(p.allows_gain(def), "{id} should be allowed to gain");
        let mut wrong_effect = def.clone();
        wrong_effect.effect = EffectClass::WriteWorkspace;
        assert!(!p.allows_gain(&wrong_effect), "WriteWorkspace must never be gained");
    }

    let map_hashes: Vec<String> = p.earlier_defs_for(RECORD_PAPER_MAP).iter().map(ToolDef::def_hash).collect();
    assert_eq!(map_hashes.len(), 3);
    assert!(map_hashes.contains(&GOLDEN_MAP_7C36A0E.to_string()));
    assert!(map_hashes.contains(&GOLDEN_MAP_NO_ANSWER_INDEX_7C36A0E.to_string()));
    assert_eq!(p.earlier_defs_for(RECORD_PAPER_SUMMARY)[0].def_hash(), GOLDEN_SUMMARY_7C36A0E);
    assert_eq!(p.earlier_defs_for(RECORD_PAPER_BRANCH)[0].def_hash(), GOLDEN_BRANCH_7C36A0E);
    assert_eq!(p.earlier_defs_for(RECORD_PAPER_CONVENTIONS)[0].def_hash(), GOLDEN_CONVENTIONS_7C36A0E);
}

#[test]
fn mode_detection_uses_latest_host_written_marker() {
    assert_eq!(session_mode_from_events(&[]), Mode::Research);
    assert_eq!(
        session_mode_from_events(&[dummy_event(1, MARKER_INVESTIGATION_STARTED, json!({}))]),
        Mode::Investigate
    );
    assert_eq!(
        session_mode_from_events(&[
            dummy_event(1, "source.ingested", json!({})),
            dummy_event(2, MARKER_INVESTIGATION_HANDOFF, json!({})),
        ]),
        Mode::Research
    );
    assert_eq!(
        session_mode_from_events(&[
            dummy_event(1, MARKER_INVESTIGATION_STARTED, json!({})),
            dummy_event(2, MARKER_INVESTIGATION_HANDOFF, json!({})),
        ]),
        Mode::Research
    );
    assert_eq!(
        session_mode_from_events(&[
            dummy_event(1, "source.ingested", json!({})),
            dummy_event(2, MARKER_IDEA_STARTED, json!({})),
        ]),
        Mode::Explore
    );
    assert_eq!(
        session_mode_from_events(&[
            dummy_event(1, "source.ingested", json!({})),
            dummy_event(2, MARKER_SOLVE_STARTED, json!({})),
        ]),
        Mode::Solve
    );
}

#[test]
fn profile_for_session_events_prefers_recorded_field_and_fails_closed_on_invalid_or_unbuilt() {
    // Explicit profile wins over markers.
    let explicit = [
        dummy_event(1, MARKER_INVESTIGATION_STARTED, json!({})),
        dummy_event(2, "session.start", json!({ "tool_profile": "research/1" })),
    ];
    assert_eq!(profile_for_session_events(&explicit).unwrap().name, PROFILE_RESEARCH_V1);

    // Unbuilt explicit profile fails closed.
    let unbuilt = [dummy_event(1, "session.start", json!({ "tool_profile": "solve/1" }))];
    let err = profile_for_session_events(&unbuilt).unwrap_err();
    assert!(err.contains("profile not built yet"), "{err}");

    // Non-string tool_profile fails closed.
    let bad = [dummy_event(1, "session.start", json!({ "tool_profile": 42 }))];
    assert!(profile_for_session_events(&bad).is_err());

    // No tool_profile -> latest mode marker.
    let legacy_inv = [
        dummy_event(1, MARKER_INVESTIGATION_STARTED, json!({})),
        dummy_event(2, "session.start", json!({})),
    ];
    assert_eq!(profile_for_session_events(&legacy_inv).unwrap().name, PROFILE_INVESTIGATION_V1);

    // Marker for unbuilt mode fails closed.
    let legacy_solve = [
        dummy_event(1, MARKER_SOLVE_STARTED, json!({})),
        dummy_event(2, "session.start", json!({})),
    ];
    assert!(profile_for_session_events(&legacy_solve).unwrap_err().contains("profile not built yet"));
}

#[test]
fn register_profile_matches_profile_defs_and_mcp_defs() {
    let env = Arc::new(ResearchEnv::new(std::env::temp_dir().join("re-search-profile-reg-test")));
    let stub: Arc<dyn ToolHandler> = Arc::new(|_: &tools::CallCtx<'_>, _: &Value| Ok(json!({})));

    for profile in built_profiles() {
        let mut reg = Registry::new();
        let pol = register_profile(
            &profile,
            &mut reg,
            Arc::clone(&env),
            ProfileHostTools {
                sympy_epoch: Arc::new(DummyEpoch),
                sympy_handler: Arc::clone(&stub),
                session_title_handler: None,
            },
        )
        .unwrap();
        assert_eq!(pol.version, profile.policy.version);
        assert!(defs_match_exact(&reg.defs(), &profile.defs));

        let mut mcp_reg = Registry::new();
        let mcp_pol = register_profile(
            &profile,
            &mut mcp_reg,
            Arc::clone(&env),
            ProfileHostTools {
                sympy_epoch: Arc::new(DummyEpoch),
                sympy_handler: Arc::clone(&stub),
                session_title_handler: Some(Arc::clone(&stub)),
            },
        )
        .unwrap();
        assert_eq!(mcp_pol.version, profile.policy.version);
        assert!(defs_match_exact(&mcp_reg.defs(), &profile.mcp_defs()));
    }
}
