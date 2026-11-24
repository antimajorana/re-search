//! Experiment contracts, comparison, and evidence persistence through the normal HTTP API.

mod common;

use common::{raw_http, test_cfg};
use desktop_server::{start_desktop_server, RunningDesktopServer};
use ledger::Ledger;
use serde_json::{json, Value};

fn request(server: &RunningDesktopServer, method: &str, path: &str, body: Value) -> (u16, Value) {
    let auth = [("X-ReSearch-Token", server.info.token.as_str())];
    let (status, _, bytes) = raw_http(
        server.addr.port(),
        method,
        path,
        &auth,
        &serde_json::to_vec(&body).unwrap(),
    );
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[test]
fn experiment_rankings_keep_failures_and_use_verified_fork_receipts() {
    let directory = tempfile::tempdir().unwrap();
    let config = test_cfg(directory.path(), None, None, None);
    let server = start_desktop_server(config.clone()).unwrap();
    let (status, project) = request(
        &server,
        "POST",
        "/api/projects",
        json!({"title":"Eigenvalue benchmark"}),
    );
    assert_eq!(status, 201, "{project}");
    let project = project["id"].as_str().unwrap();
    let base = "s-benchmark-base";
    let foreign = "s-unlinked";
    assert_eq!(
        request(
            &server,
            "POST",
            "/api/sessions",
            json!({"id":base,"project_id":project})
        )
        .0,
        200
    );
    assert_eq!(
        request(&server, "POST", "/api/sessions", json!({"id":foreign})).0,
        200
    );
    let ledger = Ledger::open(directory.path()).unwrap();
    ledger
        .append(
            base,
            "session.start",
            1,
            &json!({"prompt":"Estimate an eigenvalue"}),
        )
        .unwrap();
    ledger
        .append(
            base,
            "tool.request",
            1,
            &json!({"call_id":"inherited","tool":"sympy_eval"}),
        )
        .unwrap();
    let anchor = ledger
        .append_with_hash(
            base,
            "proof.entry",
            1,
            &json!({
                "entry_id":"e-1", "entry_type":"assumption", "summary":"Fixed boundary conditions"
            }),
        )
        .unwrap();
    ledger
        .append(base, "session.end", 1, &json!({"status":"completed"}))
        .unwrap();
    let experiments = format!("/api/projects/{project}/experiments");
    assert_eq!(
        raw_http(server.addr.port(), "GET", &experiments, &[], b"").0,
        401
    );
    let plan = |session_id, at_seq, limit| {
        json!({
            "question":"Which method estimates the gap under the same budget?",
            "metric":{"name":"absolute error","unit":"dimensionless","direction":"minimize"},
            "budget":{"limit":limit,"unit":"evaluations"},
            "baseline":{"session_id":session_id,"at_seq":at_seq}
        })
    };
    assert_eq!(
        request(&server, "POST", &experiments, plan(foreign, 1, 10)).0,
        404
    );
    assert_eq!(
        request(&server, "POST", &experiments, plan(base, 999, 10)).0,
        400
    );
    assert_eq!(
        request(&server, "POST", &experiments, plan(base, anchor.seq, 0)).0,
        400
    );
    let (status, frozen) = request(&server, "POST", &experiments, plan(base, anchor.seq, 10));
    assert_eq!(status, 201, "{frozen}");
    assert_eq!(frozen["baseline"]["event_hash"], anchor.event_hash);
    assert_eq!(frozen["baseline"]["entry_id"], "e-1");
    let branch_path = format!("{experiments}/{}/branches", frozen["id"].as_str().unwrap());

    let fork = |at_seq| {
        let (status, response) = request(
            &server,
            "POST",
            &format!("/api/sessions/{base}/fork"),
            json!({"at_seq":at_seq}),
        );
        assert_eq!(status, 200, "{response}");
        assert_eq!(response["project_id"], project);
        response["new_session"].as_str().unwrap().to_string()
    };
    let wrong_fork = fork(anchor.seq + 1);
    assert_eq!(
        request(
            &server,
            "POST",
            &branch_path,
            json!({"session_id":wrong_fork,"label":"Different cutoff"})
        )
        .0,
        409
    );
    let best = fork(anchor.seq);
    let expensive = fork(anchor.seq);
    let failed = fork(anchor.seq);
    for (session_id, label) in [
        (&best, "Spectral"),
        (&expensive, "Finite differences"),
        (&failed, "Variational"),
    ] {
        let (status, registered) = request(
            &server,
            "POST",
            &branch_path,
            json!({"session_id":session_id,"label":label}),
        );
        assert_eq!(status, 201, "{registered}");
    }
    assert_eq!(
        request(
            &server,
            "POST",
            &branch_path,
            json!({"session_id":best,"label":"Spectral"})
        )
        .0,
        200
    );
    let tool = ledger
        .append(
            &best,
            "tool.request",
            1,
            &json!({"call_id":"spectral","tool":"sympy_eval"}),
        )
        .unwrap();
    let result = ledger
        .append(
            &best,
            "tool.result",
            1,
            &json!({"call_id":"spectral","ok":true,"result":{"inline":"{}"}}),
        )
        .unwrap();
    let evidence = ledger
        .append_with_hash(
            &best,
            "proof.entry",
            1,
            &json!({
                "entry_id":"e-2", "entry_type":"numerical_computation", "summary":"Gap measured"
            }),
        )
        .unwrap();
    ledger
        .append(
            &best,
            "tool.request",
            1,
            &json!({"call_id":"later","tool":"fs_read"}),
        )
        .unwrap();
    ledger
        .append(base, "proof.entry", 1, &json!({
            "entry_id":"e-2", "entry_type":"derivation", "summary":"Parent advanced after experiment freeze"
        }))
        .unwrap();
    let diff_path = format!("{branch_path}/{best}/diff");
    assert_eq!(
        raw_http(server.addr.port(), "GET", &diff_path, &[], b"").0,
        401
    );
    let (status, diff) = request(&server, "GET", &diff_path, Value::Null);
    assert_eq!(status, 200, "{diff}");
    assert_eq!(diff["base_session"], base);
    assert_eq!(diff["shared_prefix_len"], anchor.seq);
    assert_eq!(diff["base_only_events"], 0);
    let diff_text = diff["summary_diff"].as_str().unwrap();
    assert!(diff_text.contains("Gap measured"));
    assert!(!diff_text.contains("Parent advanced"));
    assert_eq!(
        request(
            &server,
            "GET",
            &format!("{branch_path}/{wrong_fork}/diff"),
            Value::Null
        )
        .0,
        404
    );

    let record = |session_id: &str, payload| {
        request(
            &server,
            "POST",
            &format!("{branch_path}/{session_id}/observations"),
            payload,
        )
    };
    assert_eq!(
        record(&best, json!({"status":"completed","actual_budget":8})).0,
        400
    );
    assert_eq!(
        record(
            &best,
            json!({
                "status":"completed","actual_budget":8,"score":0.2,"at_seq":anchor.seq
            })
        )
        .0,
        409
    );
    let (status, observed) = record(
        &best,
        json!({
            "status":"completed","actual_budget":8,"score":0.2,"at_seq":evidence.seq,"note":"Matched boundary conditions"
        }),
    );
    assert_eq!(status, 200, "{observed}");
    let receipt = &observed["branches"][0]["observations"][0]["evidence"];
    assert_eq!(receipt["event_hash"], evidence.event_hash);
    assert_eq!(receipt["entry_id"], "e-2");
    assert_eq!(
        receipt["tools"],
        json!([{
            "tool":"sympy_eval","call_id":"spectral","request_seq":tool,"result_seq":result,"result":"recorded_ok"
        }])
    );
    assert_eq!(
        record(
            &expensive,
            json!({"status":"completed","actual_budget":11,"score":0.01})
        )
        .0,
        200
    );
    let (status, observed) = record(
        &failed,
        json!({
            "status":"failed","actual_budget":5,"note":"Convergence stalled"
        }),
    );
    assert_eq!(status, 200, "{observed}");
    assert_eq!(
        observed["comparison"],
        json!({
            "completed_within_budget":1,"completed_over_budget":1,"failed":1,"pending":0,
            "best_score":0.2,"best_session_ids":[best]
        })
    );

    let (status, revised) = record(
        &failed,
        json!({"status":"completed","actual_budget":10,"score":0.1}),
    );
    assert_eq!(status, 200, "{revised}");
    assert_eq!(revised["comparison"]["best_session_ids"], json!([failed]));
    assert_eq!(
        revised["branches"][2]["observations"][0]["status"],
        "failed"
    );

    server.shutdown();
    let restarted = start_desktop_server(config).unwrap();
    let (status, loaded) = request(&restarted, "GET", &experiments, Value::Null);
    assert_eq!(status, 200, "{loaded}");
    assert_eq!(loaded["experiments"][0], revised);
}
