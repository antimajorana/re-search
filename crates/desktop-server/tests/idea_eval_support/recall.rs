//! Deterministic search-only recall evaluation across service sets and F10/F13a user library
//! (package X14 Checkpoint 1, `00-overview.md` §7.3, `exp-x14-eval.md` §4).

use crate::common::{raw_http, test_cfg};
use crate::idea_eval_support::manifest::{Case, Input};
use crate::idea_eval_support::{
    deterministic_crossref_query, deterministic_first_queries, deterministic_keyed_queries,
    sync_eval_user_library,
};
use desktop_server::start_desktop_server;
use research_tools::run_budget::{count_model_calls, RunScope};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::time::Instant;
use tempfile::TempDir;

/// One search-only evaluation configuration (zero model calls).
#[derive(Debug, Clone, Copy)]
pub struct RecallRegime {
    pub label: &'static str,
    pub with_keys: bool,
    pub with_library: bool,
}

/// Measured search-only recall and latency summary for one regime across all 10 Set I cases.
#[derive(Debug, Clone)]
pub struct RecallOutcome {
    pub found: u64,
    pub total: u64,
    pub max_time_to_candidates_s: f64,
    pub library_owned_hits: BTreeSet<String>,
}

impl RecallOutcome {
    pub fn share(&self) -> f64 {
        assert!(self.total > 0);
        self.found as f64 / self.total as f64
    }
}

/// Execute all 10 Set I cases through real Explore HTTP entry points (`POST /api/sessions` for
/// Rule P4 first search + `POST /api/sessions/{id}/idea/plan` + `POST /api/sessions/{id}/idea/search`
/// via `Router::try_call("host:idea:<session>", "idea_search", ...)`) with ZERO model calls.
pub fn run_search_recall_regime(
    cases: &[Case],
    lit_base: &str,
    regime: RecallRegime,
) -> RecallOutcome {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    if regime.with_library {
        let synced = sync_eval_user_library(&data_dir, tmp.path());
        assert_eq!(synced, 2, "expected 2 F13a user-library papers");
    }

    let mut cfg = test_cfg(&data_dir, None, None, Some(lit_base));
    if regime.with_keys {
        let env = cfg.connection_env.get_or_insert_with(Default::default);
        env.insert("SEMANTIC_SCHOLAR_API_KEY".into(), "eval-s2-hermetic-key".into());
        env.insert("OPENALEX_API_KEY".into(), "eval-oa-hermetic-key".into());
    }
    let srv = start_desktop_server(cfg).expect("start desktop server for recall eval");
    let port = srv.addr.port();
    let auth = [
        ("X-ReSearch-Token", srv.info.token.as_str()),
        ("Content-Type", "application/json"),
    ];

    let mut found = 0u64;
    let mut total = 0u64;
    let mut max_elapsed = 0.0f64;
    let mut library_owned_hits = BTreeSet::new();

    for case in cases {
        let Input::Idea { text } = &case.input else {
            panic!("{}: Set I case must have Input::Idea", case.id);
        };
        let sid = format!("s-eval-{}-{}", regime.label, case.id.to_ascii_lowercase());
        let first_qs: Vec<Value> = deterministic_first_queries(text)
            .into_iter()
            .map(|pq| json!({ "provider": pq.provider.as_str(), "q": pq.q }))
            .collect();
        assert!(!first_qs.is_empty(), "{}: X1 must generate preview queries", case.id);

        // 1. First deterministic search (Rule P4 user-shown arXiv + INSPIRE + F10 library).
        let t0 = Instant::now();
        let create_body = serde_json::to_vec(&json!({
            "id": sid,
            "mode": "explore",
            "text": text,
            "preset": "standard",
            "offline": false,
            "queries": first_qs
        }))
        .unwrap();
        let (st, _, raw) = raw_http(port, "POST", "/api/sessions", &auth, &create_body);
        let elapsed_s = t0.elapsed().as_secs_f64();
        assert_eq!(st, 200, "{}: create failed: {}", case.id, String::from_utf8_lossy(&raw));
        if elapsed_s > max_elapsed {
            max_elapsed = elapsed_s;
        }
        assert!(
            elapsed_s < 6.0,
            "{}: time_to_candidates_s {elapsed_s:.3}s exceeded 6.0s target",
            case.id
        );

        // 2. Extended deterministic search-only queries via X7 `idea_search` (Router::try_call).
        let mut ext_queries = Vec::new();
        if let Some(cr_q) = deterministic_crossref_query(text) {
            ext_queries.push(json!({
                "qid": "q-det-crossref",
                "provider": "crossref",
                "q": cr_q,
                "origin": "user"
            }));
        }
        if regime.with_keys {
            for (idx, (prov, q)) in deterministic_keyed_queries(text).into_iter().enumerate() {
                ext_queries.push(json!({
                    "qid": format!("q-det-keyed-{idx}"),
                    "provider": prov.as_str(),
                    "q": q,
                    "origin": "user"
                }));
            }
        }
        let plan_body = serde_json::to_vec(&json!({ "queries": ext_queries })).unwrap();
        let (st_p, _, raw_p) = raw_http(
            port,
            "POST",
            &format!("/api/sessions/{sid}/idea/plan"),
            &auth,
            &plan_body,
        );
        assert_eq!(st_p, 200, "{}: plan failed: {}", case.id, String::from_utf8_lossy(&raw_p));

        let (st_s, _, raw_s) = raw_http(
            port,
            "POST",
            &format!("/api/sessions/{sid}/idea/search"),
            &auth,
            b"{}",
        );
        assert_eq!(st_s, 200, "{}: search failed: {}", case.id, String::from_utf8_lossy(&raw_s));
        let resp: Value = serde_json::from_slice(&raw_s).unwrap();
        assert_eq!(resp["ok"], true, "{}: search returned not ok: {resp}", case.id);

        // Verify zero model calls in search-only recall (F9 count_model_calls == 0).
        let ledger = ledger::Ledger::open(&data_dir).unwrap();
        let events = ledger.read_session(&sid).unwrap();
        assert_eq!(
            count_model_calls(&events, RunScope::All),
            0,
            "{}: search-only recall must spend 0 model calls",
            case.id
        );

        // Rule V10: user library is listed in coverage, never counted as an external service.
        let ext_answered = resp["idea"]["coverage"]["gate"]["answered_external_services"]
            .as_u64()
            .unwrap_or(0);
        let expected_services = if regime.with_keys { 5 } else { 3 };
        assert_eq!(
            ext_answered, expected_services,
            "{}: answered_external_services mismatch (regime {})",
            case.id, regime.label
        );
        if regime.with_library {
            assert!(
                resp["idea"]["coverage"]["library"]["papers"].as_u64().unwrap_or(0) >= 2,
                "{}: F12 coverage must report user library papers",
                case.id
            );
        }

        // Collect canonical candidate keys (plus owned library flags).
        let cands = resp["idea"]["candidates"].as_array().unwrap();
        let mut cand_keys = BTreeSet::new();
        for c in cands {
            if let Some(k) = c["key"].as_str() {
                cand_keys.insert(k.to_string());
                if c["in_library"].as_bool() == Some(true) && c["owned"].as_bool() == Some(true) {
                    library_owned_hits.insert(k.to_string());
                }
            }
        }

        // Score against non-builder answer_key.expected_key_papers.
        let expected = case.answer_key["expected_key_papers"].as_array().unwrap();
        for kp in expected {
            let key = kp["key"].as_str().unwrap();
            let lib_only = kp["library_only"].as_bool().unwrap();
            total += 1;
            let hit = cand_keys.contains(key);
            if hit {
                found += 1;
            }
            if lib_only && !regime.with_library {
                assert!(
                    !hit,
                    "{}: library-only paywalled key `{key}` must NOT be found without user library",
                    case.id
                );
            }
            if lib_only && regime.with_library {
                assert!(
                    hit,
                    "{}: library-only paywalled key `{key}` MUST be found with F10/F13a user library",
                    case.id
                );
            }
        }
    }

    srv.shutdown();
    RecallOutcome {
        found,
        total,
        max_time_to_candidates_s: max_elapsed,
        library_owned_hits,
    }
}
