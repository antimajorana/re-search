//! Scripted X6 (Sharpen) -> X7 (SearchPlan via Router) -> X8 (Triage & Rule C3) runner and
//! Rule S8 / Rule P8 / Rule V2 checks for Set I (`idea_eval.rs`, package X14).

use crate::common::{raw_http, test_cfg};
use crate::idea_eval_support::manifest::{Case, Input};
use crate::idea_eval_support::wording_audit::{
    audit_empty_state_messages_rule_s8, audit_explore_view_rule_s8, audit_f12_wording_rule_s8,
    S8AuditCount,
};
use crate::idea_eval_support::{
    deterministic_first_queries, scripted_tool_response, set_i_dir, sync_eval_user_library,
    ScriptedExploreTransport,
};
use desktop_server::idea::EVENT_IDEA_EXCHANGE;
use desktop_server::idea_exchanges::{run_sharpen_with_transport, SharpenRunResult};
use desktop_server::idea_exchanges_triage::{run_triage_with_transport, TriageRunResult};
use desktop_server::{idea_view, start_desktop_server};
use ledger::Ledger;
use research_tools::idea_brief::RECORDER_IDEA_BRIEF;
use research_tools::idea_triage::RECORDER_IDEA_TRIAGE;
use research_tools::run_budget::{count_model_calls, RunScope};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use tempfile::TempDir;

const S2_TEST_KEY: &str = "eval-s2-hermetic-key";
const OA_TEST_KEY: &str = "eval-oa-hermetic-key";

/// Summary of scripted X6 -> X7 -> X8 runs across all 10 Set I cases.
#[derive(Debug, Clone)]
pub struct ExchangeEvalOutcome {
    pub cases_run: usize,
    pub total_model_calls: u64,
    pub runs_above_cap: u64,
    pub abstract_evidence_total: u64,
    pub openalex_non_verbatim_rejected: bool,
    pub quick_preset_zero_papers_verified: bool,
    pub s8_audit: S8AuditCount,
}

/// Run all 10 Set I cases through real X6 Sharpen -> X7 SearchPlan -> X8 Triage with
/// `ScriptedExploreTransport` + `RoutedMock` + F13a user library, enforcing F9 caps,
/// Rule C3 (including OpenAlex `verbatim: false` rejection), Rule P8 secret non-leakage,
/// Rule V2 Quick preset, 1:1 `manifest-i.json` part/reading-list alignment, and Rule S8.
pub fn run_scripted_x6_x7_x8_suite(cases: &[Case], lit_base: &str) -> ExchangeEvalOutcome {
    let script_text = fs::read_to_string(set_i_dir().join("scripts/set_i_exchanges.json"))
        .expect("read set_i_exchanges.json");
    assert!(!script_text.contains('\r'), "set_i_exchanges.json must be LF-only");
    let script_root: Value = serde_json::from_str(&script_text).expect("parse set_i_exchanges.json");
    let script_cases = script_root["cases"].as_object().expect("cases map");

    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    sync_eval_user_library(&data_dir, tmp.path());

    let mut cfg = test_cfg(&data_dir, None, None, Some(lit_base));
    let env = cfg.connection_env.get_or_insert_with(Default::default);
    env.insert("SEMANTIC_SCHOLAR_API_KEY".into(), S2_TEST_KEY.into());
    env.insert("OPENALEX_API_KEY".into(), OA_TEST_KEY.into());
    let srv = start_desktop_server(cfg).expect("start desktop server for scripted X6-X8 eval");
    let port = srv.addr.port();
    let auth = [
        ("X-ReSearch-Token", srv.info.token.as_str()),
        ("Content-Type", "application/json"),
    ];
    let ledger = Ledger::open(&data_dir).unwrap();

    let mut total_model_calls = 0u64;
    let mut runs_above_cap = 0u64;
    let mut abstract_evidence_total = 0u64;
    let mut openalex_non_verbatim_rejected = false;
    let mut s8_audit = audit_empty_state_messages_rule_s8();
    let mut f12_audited = false;

    for case in cases {
        let Input::Idea { text } = &case.input else { panic!("expected Input::Idea") };
        let sid = format!("s-scripted-{}", case.id.to_ascii_lowercase());
        let first_qs: Vec<Value> = deterministic_first_queries(text)
            .into_iter()
            .map(|pq| json!({ "provider": pq.provider.as_str(), "q": pq.q }))
            .collect();

        // 1. Create Explore session (Standard preset, hard cap 24 model calls).
        let create_body = serde_json::to_vec(&json!({
            "id": sid, "mode": "explore", "text": text,
            "preset": "standard", "offline": false, "queries": first_qs
        }))
        .unwrap();
        let (st, _, raw) = raw_http(port, "POST", "/api/sessions", &auth, &create_body);
        assert_eq!(st, 200, "{}: create failed: {}", case.id, String::from_utf8_lossy(&raw));

        // 2. X6 Sharpen via scripted transport; verify 1:1 subclaim alignment with manifest-i.json.
        let c_script = &script_cases[&case.id];
        let t_sharpen = ScriptedExploreTransport::new(vec![scripted_tool_response(
            "tu-sharpen",
            RECORDER_IDEA_BRIEF,
            c_script["sharpen_input"].clone(),
        )]);
        let sh_res = run_sharpen_with_transport(
            &ledger, &sid, &t_sharpen, "scripted-explore-eval-model", None, |_, _| {},
        )
        .unwrap_or_else(|e| panic!("{}: sharpen failed: {e}", case.id));
        let SharpenRunResult::Finished { brief, attempts, .. } = sh_res else {
            panic!("{}: expected Finished sharpen", case.id);
        };
        assert_eq!(attempts, 1);
        assert_eq!(t_sharpen.calls_made(), 1);
        let manifest_parts = case.answer_key["parts"].as_array().unwrap();
        assert_eq!(
            brief.subclaims.len(),
            manifest_parts.len(),
            "{}: Sharpen subclaim count must match manifest-i.json parts",
            case.id
        );
        for (sc, mp) in brief.subclaims.iter().zip(manifest_parts.iter()) {
            assert_eq!(sc.id, mp["id"].as_str().unwrap(), "{}: subclaim id mismatch", case.id);
        }

        // 3. X7 Plan + Router-enforced Search from sharpened perspectives; audit plan_approval Rule S8.
        let (st_p, _, raw_p) =
            raw_http(port, "POST", &format!("/api/sessions/{sid}/idea/plan"), &auth, b"{}");
        assert_eq!(st_p, 200, "{}: plan failed: {}", case.id, String::from_utf8_lossy(&raw_p));
        let plan_json: Value = serde_json::from_slice(&raw_p).unwrap();
        let plan_events = ledger.read_session(&sid).unwrap();
        s8_audit.add(audit_explore_view_rule_s8(&plan_json["idea"], &plan_events));

        let (st_s, _, raw_s) =
            raw_http(port, "POST", &format!("/api/sessions/{sid}/idea/search"), &auth, b"{}");
        assert_eq!(st_s, 200, "{}: search failed: {}", case.id, String::from_utf8_lossy(&raw_s));
        let search_json: Value = serde_json::from_slice(&raw_s).unwrap();
        assert_eq!(search_json["ok"], true);

        // 4. X8 Triage via scripted transport (verifying Rule C3 + OpenAlex rejection on I-05).
        let cands = search_json["idea"]["candidates"].as_array().unwrap();
        let core_map: BTreeMap<String, (Value, Value)> = c_script["triage_core"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| {
                (
                    entry["key"].as_str().unwrap().to_string(),
                    (entry["subclaim_ids"].clone(), entry["abstract_quote"].clone()),
                )
            })
            .collect();

        let build_triage_items = |force_oa_quote: bool| -> (Vec<Value>, Vec<String>) {
            let mut items = Vec::new();
            let mut reading_list = Vec::new();
            for c in cands {
                let k = c["key"].as_str().unwrap().to_string();
                if let Some((scs, aq)) = core_map.get(&k) {
                    reading_list.push(k.clone());
                    let quote_val = if force_oa_quote && k == "doi:10.9999/eval.i05.knabe" {
                        json!("Using the detectability lemma and finite-size Knabe threshold inequalities")
                    } else {
                        aq.clone()
                    };
                    items.push(json!({
                        "key": k, "relevance": "core", "subclaim_ids": scs,
                        "why": "Directly addresses an idea part in the synthetic corpus.",
                        "abstract_quote": quote_val
                    }));
                } else {
                    items.push(json!({
                        "key": k, "relevance": "background", "subclaim_ids": [],
                        "why": "Background context paper in the synthetic corpus.",
                        "abstract_quote": null
                    }));
                }
            }
            (items, reading_list)
        };

        let (ok_items, reading_list) = build_triage_items(false);
        let expected_verbatim_quotes: BTreeSet<String> = core_map
            .iter()
            .filter(|(_, (_, aq))| aq.is_string())
            .map(|(k, _)| k.clone())
            .collect();
        let responses = if case.id == "I-05" {
            let (bad_items, bad_rl) = build_triage_items(true);
            vec![
                scripted_tool_response("tu-tr-bad", RECORDER_IDEA_TRIAGE, json!({ "items": bad_items, "reading_list": bad_rl })),
                scripted_tool_response("tu-tr-ok", RECORDER_IDEA_TRIAGE, json!({ "items": ok_items, "reading_list": reading_list })),
            ]
        } else {
            vec![scripted_tool_response("tu-tr-ok", RECORDER_IDEA_TRIAGE, json!({ "items": ok_items, "reading_list": reading_list }))]
        };

        let t_triage = ScriptedExploreTransport::new(responses);
        let tr_res = run_triage_with_transport(
            &data_dir, &ledger, &sid, &t_triage, "scripted-explore-eval-model", None, |_, _| {},
        )
        .unwrap_or_else(|e| panic!("{}: triage failed: {e}", case.id));
        match tr_res {
            TriageRunResult::Finished { outcome, attempts, .. } => {
                assert_eq!(outcome.abstract_evidence_ids.len(), expected_verbatim_quotes.len(), "{}: abstract evidence count mismatch", case.id);
                abstract_evidence_total += outcome.abstract_evidence_ids.len() as u64;
                if case.id == "I-05" {
                    assert_eq!(attempts, 2, "I-05 OpenAlex rebuilt abstract must fail attempt 1 (Rule C3)");
                    openalex_non_verbatim_rejected = true;
                } else {
                    assert_eq!(attempts, 1);
                }
            }
            other => panic!("{}: expected Finished triage, got {other:?}", case.id),
        }

        // 5. Inspect ledger & view: F9 caps, reading_list coverage of expected key papers,
        // per-case xa-* evidence, I-05 Rule C3 fault event, Rule P8 secret non-leakage, Rule S8.
        let events = ledger.read_session(&sid).unwrap();
        let calls = count_model_calls(&events, RunScope::All);
        total_model_calls += calls;
        let view = idea_view::view(&events, false);
        let cap = view["budget"]["max_model_calls"].as_u64().unwrap_or(24);
        if calls > cap {
            runs_above_cap += 1;
        }

        // Verify every expected key paper (including library-only I-03 & I-08) is triaged core & in reading_list.
        let rl_set: BTreeSet<String> = view["triage"]["reading_list"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        let core_keys: BTreeSet<String> = view["triage"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|it| it["relevance"].as_str() == Some("core"))
            .filter_map(|it| it["key"].as_str().map(str::to_string))
            .collect();
        for kp in case.answer_key["expected_key_papers"].as_array().unwrap() {
            let k = kp["key"].as_str().unwrap();
            assert!(rl_set.contains(k), "{}: expected key paper `{k}` missing from triage.reading_list", case.id);
            assert!(core_keys.contains(k), "{}: expected key paper `{k}` not triaged as `core`", case.id);
        }

        // Verify per-case abstract_evidence `xa-*` pointers match expected verbatim quotes.
        let xa_keys: BTreeSet<String> = view["abstract_evidence"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                assert!(e["id"].as_str().is_some_and(|s| s.starts_with("xa-")));
                e["paper_key"].as_str().unwrap().to_string()
            })
            .collect();
        assert_eq!(xa_keys, expected_verbatim_quotes, "{}: xa-* paper keys mismatch", case.id);

        if case.id == "I-05" {
            let has_oa_fault = events.iter().any(|e| {
                e.kind == EVENT_IDEA_EXCHANGE
                    && e.payload["accepted"] == json!(false)
                    && serde_json::to_string(&e.payload["faults"])
                        .unwrap_or_default()
                        .to_ascii_lowercase()
                        .contains("verbatim")
            });
            assert!(has_oa_fault, "I-05 ledger must record Rule C3 non-verbatim fault on attempt 1");
        }

        // Rule P8: API keys must never leak into ledger events or view JSON.
        let events_json = serde_json::to_string(&events).unwrap();
        let view_json = serde_json::to_string(&view).unwrap();
        for secret in [S2_TEST_KEY, OA_TEST_KEY] {
            assert!(!events_json.contains(secret), "{}: Rule P8 leak in ledger", case.id);
            assert!(!view_json.contains(secret), "{}: Rule P8 leak in view", case.id);
        }

        if !f12_audited {
            s8_audit.add(audit_f12_wording_rule_s8(&events));
            f12_audited = true;
        }
        s8_audit.add(audit_explore_view_rule_s8(&view, &events));
    }

    // 6. Rule V2 Quick preset check + offline empty_state Rule S8 check.
    let quick_verified =
        run_quick_preset_check(port, &auth, &data_dir, &ledger, &cases[0], &script_cases["I-01"]);
    s8_audit.add(audit_offline_empty_state_view(port, &auth, &ledger));
    srv.shutdown();

    ExchangeEvalOutcome {
        cases_run: cases.len(),
        total_model_calls,
        runs_above_cap,
        abstract_evidence_total,
        openalex_non_verbatim_rejected,
        quick_preset_zero_papers_verified: quick_verified,
        s8_audit,
    }
}

fn audit_offline_empty_state_view(port: u16, auth: &[(&str, &str)], ledger: &Ledger) -> S8AuditCount {
    let sid = "s-eval-offline-s8";
    let body = serde_json::to_vec(&json!({
        "id": sid, "mode": "explore",
        "text": "Unmatched synthetic offline topic for empty-state wording audit.",
        "preset": "standard", "offline": true, "queries": []
    }))
    .unwrap();
    assert_eq!(raw_http(port, "POST", "/api/sessions", auth, &body).0, 200);
    let events = ledger.read_session(sid).unwrap();
    let view = idea_view::view(&events, false);
    assert!(view["empty_state"]["message"].is_string(), "offline session with 0 hits must populate empty_state.message");
    audit_explore_view_rule_s8(&view, &events)
}

fn run_quick_preset_check(
    port: u16,
    auth: &[(&str, &str)],
    data_dir: &std::path::Path,
    ledger: &Ledger,
    case: &Case,
    c_script: &Value,
) -> bool {
    let Input::Idea { text } = &case.input else { return false };
    let sid = "s-eval-quick-i01";
    let first_qs: Vec<Value> = deterministic_first_queries(text)
        .into_iter()
        .map(|pq| json!({ "provider": pq.provider.as_str(), "q": pq.q }))
        .collect();
    let create_body = serde_json::to_vec(&json!({
        "id": sid, "mode": "explore", "text": text,
        "preset": "quick", "offline": false, "queries": first_qs
    }))
    .unwrap();
    assert_eq!(raw_http(port, "POST", "/api/sessions", auth, &create_body).0, 200);
    let t_sh = ScriptedExploreTransport::new(vec![scripted_tool_response(
        "tu-q-sh", RECORDER_IDEA_BRIEF, c_script["sharpen_input"].clone(),
    )]);
    run_sharpen_with_transport(ledger, sid, &t_sh, "scripted-explore-eval-model", None, |_, _| {}).unwrap();
    assert_eq!(raw_http(port, "POST", &format!("/api/sessions/{sid}/idea/plan"), auth, b"{}").0, 200);
    let (st, _, raw) = raw_http(port, "POST", &format!("/api/sessions/{sid}/idea/search"), auth, b"{}");
    assert_eq!(st, 200);
    let v: Value = serde_json::from_slice(&raw).unwrap();
    let items: Vec<Value> = v["idea"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let k = c["key"].as_str().unwrap_or("");
            if k == "arxiv:2401.90101" {
                json!({
                    "key": k, "relevance": "related", "subclaim_ids": ["sc-1"],
                    "why": "Quick abstract-only triage.",
                    "abstract_quote": "power-law exponent directly proportional to the Luttinger parameter K"
                })
            } else {
                json!({
                    "key": k, "relevance": "background", "subclaim_ids": [],
                    "why": "Background context in Quick abstract-only triage.",
                    "abstract_quote": null
                })
            }
        })
        .collect();
    let t_tr = ScriptedExploreTransport::new(vec![scripted_tool_response(
        "tu-q-tr", RECORDER_IDEA_TRIAGE, json!({ "items": items, "reading_list": [] }),
    )]);
    run_triage_with_transport(data_dir, ledger, sid, &t_tr, "scripted-explore-eval-model", None, |_, _| {}).unwrap();
    let events = ledger.read_session(sid).unwrap();
    let view = idea_view::view(&events, false);
    view["budget"]["max_papers"].as_u64() == Some(0)
        && view["budget"]["papers_fetched"].as_u64() == Some(0)
        && view["triage"]["quick_abstract_only"].as_bool() == Some(true)
        && view["triage"]["reading_list"].as_array().is_some_and(Vec::is_empty)
}
