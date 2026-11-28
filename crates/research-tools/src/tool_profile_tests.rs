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
fn investigation_v1_and_v2_encode_may_gain_and_earlier_defs_as_data_while_research_v1_stays_frozen() {
    let r1 = research_v1();
    assert_eq!(r1.may_gain, LEGACY_MAY_GAIN);
    assert!(!r1.allows_gain(&read_map_node_def()), "research/1 must never gain read_map_node");
    assert!(!r1.allows_gain(&record_map_reading_def()), "research/1 must never gain record_map_reading");

    let p = investigation_v1();
    assert_eq!(p.may_gain.len(), LEGACY_MAY_GAIN.len() + INVESTIGATION_V2_MAY_GAIN.len());
    for id in LEGACY_MAY_GAIN {
        let def = p.def_for(id).unwrap_or_else(|| panic!("missing {id}"));
        assert!(p.allows_gain(def), "{id} should be allowed to gain");
        let mut wrong_effect = def.clone();
        wrong_effect.effect = EffectClass::WriteWorkspace;
        assert!(!p.allows_gain(&wrong_effect), "WriteWorkspace must never be gained");
    }
    assert!(p.allows_gain(&read_map_node_def()), "investigation/1 must allow gaining read_map_node");
    assert!(p.allows_gain(&record_map_reading_def()), "investigation/1 must allow gaining record_map_reading");
    assert!(
        p.allows_gain(&crate::map_context_defs::record_context_cards_def()),
        "investigation/1 must allow gaining record_context_cards"
    );
    let mut bad_v6 = read_map_node_def();
    bad_v6.effect = EffectClass::WriteWorkspace;
    assert!(!p.allows_gain(&bad_v6), "WriteWorkspace V6 tool must never be gained");

    let v2 = investigation_v2();
    assert_eq!(v2.name, PROFILE_INVESTIGATION_V2);
    assert_eq!(v2.defs.len(), r1.defs.len() + 3);
    assert!(v2.def_for(crate::map_defs::READ_MAP_NODE).is_some());
    assert!(v2.def_for(crate::map_defs::RECORD_MAP_READING).is_some());
    assert!(v2.def_for(crate::map_defs::RECORD_CONTEXT_CARDS).is_some());
    assert!(v2.def_for(crate::map_defs::RECORD_PRIOR_WORK_CHECK).is_none());
    for pair in v2.defs.windows(2) {
        assert!(pair[0].id < pair[1].id, "investigation/2 not sorted");
    }
    // New Investigate session (no session.start yet) resolves to investigation/2.
    let new_inv = [dummy_event(1, MARKER_INVESTIGATION_STARTED, json!({}))];
    assert_eq!(profile_for_session_events(&new_inv).unwrap().name, PROFILE_INVESTIGATION_V2);

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
    let unbuilt = [dummy_event(1, "session.start", json!({ "tool_profile": "explore/1" }))];
    let err = profile_for_session_events(&unbuilt).unwrap_err();
    assert!(err.contains("profile not built yet"), "{err}");

    // Explicit `solve/1` resolves cleanly to solve/1 with v0.5-solve policy.
    let solve_explicit = [dummy_event(1, "session.start", json!({ "tool_profile": "solve/1" }))];
    let sp = profile_for_session_events(&solve_explicit).unwrap();
    assert_eq!(sp.name, PROFILE_SOLVE_V1);
    assert_eq!(sp.policy.version, crate::SOLVE_POLICY_VERSION);
    assert_eq!(sp.defs.len(), 16);

    // Non-string tool_profile fails closed.
    let bad = [dummy_event(1, "session.start", json!({ "tool_profile": 42 }))];
    assert!(profile_for_session_events(&bad).is_err());

    // No tool_profile -> latest mode marker.
    let legacy_inv = [
        dummy_event(1, MARKER_INVESTIGATION_STARTED, json!({})),
        dummy_event(2, "session.start", json!({})),
    ];
    assert_eq!(profile_for_session_events(&legacy_inv).unwrap().name, PROFILE_INVESTIGATION_V1);

    // Marker for built Solve mode resolves to solve/1.
    let legacy_solve = [
        dummy_event(1, MARKER_SOLVE_STARTED, json!({})),
        dummy_event(2, "session.start", json!({})),
    ];
    assert_eq!(profile_for_session_events(&legacy_solve).unwrap().name, PROFILE_SOLVE_V1);

    // Marker for unbuilt Explore mode fails closed.
    let legacy_explore = [
        dummy_event(1, MARKER_IDEA_STARTED, json!({})),
        dummy_event(2, "session.start", json!({})),
    ];
    assert!(profile_for_session_events(&legacy_explore).unwrap_err().contains("profile not built yet"));
}

#[test]
fn solve_v1_contains_16_tools_excludes_forbidden_and_preserves_reused_def_hashes() {
    let r1 = research_v1();
    let s1 = solve_v1();
    assert_eq!(s1.name, PROFILE_SOLVE_V1);
    assert_eq!(s1.policy.version, crate::SOLVE_POLICY_VERSION);
    assert_eq!(s1.defs.len(), 16);
    assert!(s1.may_gain.is_empty());
    assert!(s1.earlier_defs.is_empty());

    for pair in s1.defs.windows(2) {
        assert!(pair[0].id < pair[1].id, "not strictly sorted: {} >= {}", pair[0].id, pair[1].id);
    }

    let ids: Vec<&str> = s1.defs.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids, crate::solve_defs::SOLVE_V1_TOOL_IDS);

    // Every reused tool matches research/1 def_hash byte for byte.
    for reused in [
        "sympy_eval",
        "ledger_supersede",
        "ledger_query",
        "ledger_note",
        "read_paper_lines",
        "search_paper_passages",
        "arxiv_search",
        "literature_search",
        "fs_read",
        "fs_list",
        "check_expression_sanity",
        "tex_audit",
    ] {
        assert_eq!(
            s1.def_for(reused).unwrap().def_hash(),
            r1.def_for(reused).unwrap().def_hash(),
            "reused tool `{reused}` def_hash drifted"
        );
    }

    // Explicitly excluded tools are absent.
    for excluded in [
        "doc_ingest",
        "ledger_append",
        "lean_check",
        "search_formal_library",
        "fs_write",
        "latex_patch",
        "latex_compile",
        "record_paper_map",
        "record_paper_summary",
        "record_paper_branch",
        "record_paper_conventions",
        "read_map_node",
        "record_map_reading",
        "record_context_cards",
        "idea_search",
        "idea_fetch_papers",
    ] {
        assert!(s1.def_for(excluded).is_none(), "excluded tool `{excluded}` found in solve/1");
    }
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
            ProfileHostTools::new(Arc::new(DummyEpoch), Arc::clone(&stub)),
        )
        .unwrap();
        assert_eq!(pol.version, profile.policy.version);
        assert!(defs_match_exact(&reg.defs(), &profile.defs));

        let mut mcp_reg = Registry::new();
        let mcp_pol = register_profile(
            &profile,
            &mut mcp_reg,
            Arc::clone(&env),
            ProfileHostTools::new(Arc::new(DummyEpoch), Arc::clone(&stub))
                .with_session_title(Arc::clone(&stub)),
        )
        .unwrap();
        assert_eq!(mcp_pol.version, profile.policy.version);
        assert!(defs_match_exact(&mcp_reg.defs(), &profile.mcp_defs()));
    }
}
