//! Integration tests for Solve S14 (`POST /api/sessions/{id}/solve/review`, D2 same-model
//! review, AuthGuard enforcement) and S20 (blind compute, Gate G3, `independent_recompute`).

mod common;

use agent::{ProviderEntry, ProviderKeyStore};
use common::{raw_http, test_cfg, MockHttp, MockReply};
use desktop_server::start_desktop_server;
use ledger::Ledger;
use research_tools::solve_model::{
    compute_entry_sha256, compute_math_sha256, event_kinds, ReviewIndependence, ReviewPolicy,
    RoleModel, SolveBudgets, SolveRoles, SolveSetup, SolveTarget, StepEntryType, StepOrigin,
    TargetForm, TaskKind,
};
use research_tools::solve_state::SolveState;
use research_tools::solve_status::Tier;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use tempfile::TempDir;

fn openai_json_tool_call(call_id: &str, name: &str, args: &Value) -> Vec<u8> {
    json!({
        "id": "chatcmpl-test-1",
        "object": "chat.completion",
        "model": "gpt-4.1",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": call_id,
                    "type": "function",
                    "function": { "name": name, "arguments": args.to_string() }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": { "prompt_tokens": 100, "completion_tokens": 40, "total_tokens": 140 }
    })
    .to_string()
    .into_bytes()
}

fn seed_solve_session(data_dir: &std::path::Path, session: &str, solver: RoleModel, verifier: RoleModel) {
    let ledger = Ledger::open(data_dir).unwrap();
    ledger
        .append_with_hash(session, event_kinds::SOLVE_STARTED, 1, &json!({}))
        .unwrap();
    let math_sha = compute_math_sha256(&[], &BTreeMap::new(), &[], "Euclidean");
    let setup = SolveSetup {
        revision: 1,
        statement: "Prove monotonicity of f(x) = x^3".into(),
        task_kind: TaskKind::Prove,
        target: SolveTarget {
            description: "proof".into(),
            form: TargetForm::Proof,
            order: None,
            digits: None,
        },
        done: vec![],
        conventions: "Euclidean".into(),
        symbols: vec![],
        dimensions: BTreeMap::new(),
        assumptions: vec![],
        given_source: None,
        sources: vec![],
        roles: SolveRoles {
            solver,
            verifier,
            compute: None,
        },
        budgets: SolveBudgets::default(),
        review: ReviewPolicy::ProseOnly,
        tool_profile: event_kinds::SOLVE_TOOL_PROFILE_V1.into(),
        origin: None,
        math_sha256: math_sha,
    };
    ledger
        .append_with_hash(
            session,
            event_kinds::SOLVE_SETUP,
            1,
            &serde_json::to_value(&setup).unwrap(),
        )
        .unwrap();
    let e1_sha = compute_entry_sha256(None, "Derivative non-negative", "f'(x) = 3x^2 >= 0 for real x.");
    ledger
        .append_with_hash(
            session,
            event_kinds::PROOF_ENTRY,
            1,
            &json!({
                "entry_id": "e-1",
                "entry_type": StepEntryType::Derivation.as_str(),
                "summary": "Derivative non-negative",
                "details": "f'(x) = 3x^2 >= 0 for real x.",
                "depends_on": [],
                "entry_sha256": e1_sha,
                "origin": StepOrigin::Solver,
            }),
        )
        .unwrap();
}

#[test]
fn solve_review_route_enforces_auth_mode_and_runs_d2_same_model_end_to_end() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    // Mock OpenAI server replies with a valid `report_verdict` tool call.
    let mock = MockHttp::start(vec![MockReply {
        status: 200,
        content_type: "application/json",
        body: openai_json_tool_call(
            "call_v1",
            "report_verdict",
            &json!({
                "verdicts": [{
                    "entry_id": "e-1",
                    "verdict": "accepted",
                    "summary": "Derivative 3x^2 >= 0 holds for all real x.",
                    "checks_cited": []
                }]
            }),
        ),
    }]);

    let mut store = ProviderKeyStore::default();
    store.providers.insert(
        "openai".into(),
        ProviderEntry {
            api_key: Some("test-fake-openai-key".into()),
            base_url: Some(mock.base_url.clone()),
            default_model: None,
        },
    );
    store.save_to_data_dir(&data_dir).unwrap();

    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let auth = [("X-ReSearch-Token", tok.as_str())];

    // 1. Create a Research session and a Solve session.
    let (st, _, _) = raw_http(
        port,
        "POST",
        "/api/sessions",
        &auth,
        br#"{"id":"s-res-only"}"#,
    );
    assert_eq!(st, 200);

    let same_role = RoleModel {
        provider: "openai".into(),
        model: "gpt-4.1".into(),
    };
    seed_solve_session(&data_dir, "s-solve-d2", same_role.clone(), same_role);

    // 2. Unauthenticated POST /solve/review must be refused (401).
    let (st_unauth, _, _) = raw_http(
        port,
        "POST",
        "/api/sessions/s-solve-d2/solve/review",
        &[],
        br#"{"entry_id":"e-1"}"#,
    );
    assert_eq!(st_unauth, 401);

    // 3. Research session must be refused (400).
    let (st_wrong_mode, _, _) = raw_http(
        port,
        "POST",
        "/api/sessions/s-res-only/solve/review",
        &auth,
        br#"{"entry_id":"e-1"}"#,
    );
    assert_eq!(st_wrong_mode, 400);

    // 4. Authenticated POST /solve/review executes D2 same-model review and stamps SameModel.
    let (st_ok, _, raw_ok) = raw_http(
        port,
        "POST",
        "/api/sessions/s-solve-d2/solve/review",
        &auth,
        br#"{"entry_id":"e-1"}"#,
    );
    assert_eq!(
        st_ok,
        200,
        "POST /solve/review failed: {}",
        String::from_utf8_lossy(&raw_ok)
    );
    let body: Value = serde_json::from_slice(&raw_ok).unwrap();
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["outcome"]["status"], json!("completed"));
    assert_eq!(
        body["outcome"]["independence"],
        serde_json::to_value(ReviewIndependence::SameModel).unwrap()
    );
    assert_eq!(body["outcome"]["model_calls_used"], json!(1));

    // Confirm ledger folded e-1 to ModelAgreedSameModel (`"Model agreed (same model)"`).
    let ledger = Ledger::open(&data_dir).unwrap();
    let events = ledger.read_session("s-solve-d2").unwrap();
    let folded = SolveState::from_events(&events);
    let tier = folded.tier("e-1").unwrap();
    assert_eq!(tier.own, Tier::ModelAgreedSameModel);
    assert_eq!(tier.effective, Tier::ModelAgreedSameModel);

    mock.finish();
}
