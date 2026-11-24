//! Claim-linked evidence is bounded, redacted, and tied to verified ledger records.

mod common;

use common::{raw_http, test_cfg};
use desktop_server::start_desktop_server;
use ledger::{sha256_hex, Ledger};
use serde_json::{json, Value};

#[test]
fn evidence_route_links_checks_and_citations_to_the_selected_claim() {
    let dir = tempfile::tempdir().unwrap();
    let leaked = "redact-this-fixture-value";
    let mut cfg = test_cfg(dir.path(), None, None, None);
    cfg.connection_env
        .as_mut()
        .unwrap()
        .insert("OPENAI_API_KEY".into(), leaked.into());
    let srv = start_desktop_server(cfg).unwrap();
    let auth = [("X-ReSearch-Token", srv.info.token.as_str())];
    let call = |method: &str, path: &str, body: Value| {
        let (status, _, bytes) = raw_http(
            srv.addr.port(),
            method,
            path,
            &auth,
            &serde_json::to_vec(&body).unwrap(),
        );
        (status, serde_json::from_slice::<Value>(&bytes).unwrap())
    };

    let (status, session) = call("POST", "/api/sessions", json!({"title":"Evidence"}));
    assert_eq!(status, 200, "{session}");
    let sid = session["id"].as_str().unwrap();
    let path = format!("/api/sessions/{sid}/evidence");
    assert_eq!(call("GET", &path, Value::Null).1["entries"], json!([]));

    let ledger = Ledger::open(dir.path()).unwrap();
    let record = |kind, payload| ledger.append_with_hash(sid, kind, 1, &payload).unwrap().seq;
    let put_json = |data: Value| {
        let bytes = serde_json::to_vec(&data).unwrap();
        json!({"blob": ledger.put_blob(&bytes).unwrap(), "bytes": bytes.len()})
    };
    record("session.start", json!({"prompt":"Check 2+2"}));
    record(
        "tool.request",
        json!({
            "call_id":"compute","tool":"sympy_eval","args":{"inline":"{\"code\":\"2+2\"}"}
        }),
    );
    let compute_seq = record(
        "tool.result",
        json!({
            "call_id":"compute","ok":true,"result":put_json(json!({
                "value":"4", "latex":"4", "stdout":format!("RESULT {leaked} {}", "α".repeat(3000)),
                "validation":{
                    "schema_version":1,"kind":"symbolic_identity","status":"symbolic_agreement",
                    "lhs":"(2*x + 2*x)/x","rhs":"4","residual":"0",
                    "assumptions":{"x":{"positive":true,"real":true,"unrecognized":leaked}},
                    "domain":{"requested":"Reals","effective":"Interval.open(0, oo)",
                              "common":"Interval.open(0, oo)","status":"complete"},
                    "samples":[{"point":"2","lhs":"4","rhs":"4","status":"agreement","private":leaked}],
                    "limits":[{"point":"0","direction":"+","lhs":"4","rhs":"4","status":"agreement"}],
                    "dimensions":{"status":"match","lhs":"dimensionless","rhs":"dimensionless"}
                }
            }))
        }),
    );
    record(
        "proof.entry",
        json!({
            "entry_id":"e-1","entry_type":"symbolic_computation","tool_call_ref":format!("compute@seq:{compute_seq}")
        }),
    );
    let lean_code = "theorem arithmetic : 2 + 2 = 4 := by decide";
    record(
        "tool.request",
        json!({
            "call_id":"lean","tool":"lean_check","args":{"inline":json!({"code":lean_code}).to_string()}
        }),
    );
    record(
        "tool.result",
        json!({
            "call_id":"lean","ok":true,"result":{"inline":json!({
                "ok":true,"status":"verified","axioms":["propext"],"source_sha256":sha256_hex(lean_code.as_bytes())
            }).to_string()}
        }),
    );
    record(
        "tool.request",
        json!({
            "call_id":"refine","tool":"sympy_eval","args":{"inline":"{}"}
        }),
    );
    let numerical_levels: Vec<_> = (1..=12)
        .map(|index| {
            let value = 1.0 / f64::from(index).powi(2);
            json!({"level": index.to_string(), "work_dps": 40, "value": {
            "real": format!("{value:.10}"), "imag": "0", "unrecognized": leaked
        }, "delta": "0.001", "threshold": "0.01", "passes": index > 6,
        "reference_error": format!("{value:.10}"), "reference_threshold": "0.02",
        "reference_passes": index > 7})
        })
        .collect();
    record(
        "tool.result",
        json!({
            "call_id":"refine","ok":true,"result":put_json(json!({
                "value":"0.0069444444",
                "validation":{
                    "schema_version":1,"kind":"numerical_convergence","axis":"resolution","status":"within_tolerance",
                    "stable":true,"within_tolerance":true,"reference_within_tolerance":true,"tail":5,
                    "analysis_dps":40,"atol":"0.01","rtol":"0","reference":"0","samples":numerical_levels
                }
            }))
        }),
    );
    record(
        "proof.entry",
        json!({
            "entry_id":"e-5","entry_type":"numerical_computation","tool_call_ref":"refine"
        }),
    );
    record(
        "proof.entry",
        json!({
            "entry_id":"e-2","entry_type":"verification","tool_call_ref":"lean"
        }),
    );
    let source = b"Counting\nA finite set with two pairs has four elements.\n";
    let source_blob = ledger.put_blob(source).unwrap();
    let source_dir = dir.path().join(format!("sessions/{sid}/sources/notes-1"));
    std::fs::create_dir_all(&source_dir).unwrap();
    std::fs::write(source_dir.join("counting.txt"), source).unwrap();
    let source_seq = record(
        "source.ingested",
        json!({
            "source_id":"notes-1","format":"txt","status":"ok","title":"Finite Sets",
            "raw_blob":source_blob,"primary_path":"notes-1/counting.txt",
            "files":["notes-1/counting.txt"], "files_sha256":{"notes-1/counting.txt":source_blob}
        }),
    );
    record(
        "tool.request",
        json!({"call_id":"find-counting","tool":"search_paper_passages","args":{"inline":"{}"}}),
    );
    record(
        "tool.result",
        json!({"call_id":"find-counting","ok":true,"result":put_json(json!({
            "passages":[{
                "source_id":"notes-1","source_seq":source_seq,"source_sha256":source_blob,
                "path":"notes-1/counting.txt","file_sha256":source_blob,"start_line":2,"end_line":2,
                "excerpt":"A finite set with two pairs has four elements."
            }]
        }))}),
    );
    record(
        "proof.entry",
        json!({"entry_id":"e-6","entry_type":"reference_lookup","tool_call_ref":"find-counting"}),
    );
    record(
        "proof.entry",
        json!({
            "entry_id":"e-3","entry_type":"final_answer","depends_on":["e-1","e-2","e-6"]
        }),
    );
    record(
        "proof.verification",
        json!({
            "target_entry_id":"e-3","kind":"full","verdict":"accepted","summary":"Finite sets check out.",
            "answer_sha256":sha256_hex(b"Two plus two is four.")
        }),
    );
    record(
        "proof.entry",
        json!({
            "entry_id":"e-4","entry_type":"reference_lookup","tool_call_ref":"unrelated"
        }),
    );
    record(
        "tool.request",
        json!({
            "call_id":"unrelated","tool":"fs_read","args":{"inline":"{}"}
        }),
    );
    record(
        "tool.result",
        json!({
            "call_id":"unrelated","ok":true,"result":{"inline":"{\"content\":\"OUT_OF_SCOPE\"}"}
        }),
    );

    let (status, project) = call("POST", "/api/projects", json!({"title":"Finite sets"}));
    assert_eq!(status, 201, "{project}");
    let pid = project["id"].as_str().unwrap();
    assert_eq!(
        call(
            "PUT",
            &format!("/api/projects/{pid}/sessions/{sid}"),
            json!({})
        )
        .0,
        200
    );
    let (status, project) = call(
        "POST",
        &format!("/api/projects/{pid}/items"),
        json!({
            "kind":"finding","text":"Two pairs contain four elements.","session_id":sid,"entry_id":"e-3",
            "source_id":"notes-1","location":"Section 1","quote":"A finite set with two pairs has four elements."
        }),
    );
    assert_eq!(status, 200, "{project}");

    let (status, _, _) = raw_http(srv.addr.port(), "GET", &path, &[], b"");
    assert_eq!(status, 401);
    let (status, doc) = call("GET", &format!("{path}?entry_id=e-3"), Value::Null);
    assert_eq!(status, 200, "{doc}");
    assert_eq!(doc["ledger_verified"], true);
    let entries = doc["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["model_reviews"][0]["verdict"], "accepted");
    assert_eq!(entries[0]["model_reviews"][0]["scope"], "full");
    let evidence = entries[0]["support"].as_array().unwrap();
    let find = |kind: &str| evidence.iter().find(|e| e["kind"] == kind).unwrap();
    let computation = find("computation");
    assert_eq!(computation["status"], "completed");
    assert_eq!(computation["via_entry_id"], "e-1");
    assert_eq!(computation["value"], "4");
    let identity = &computation["validation"];
    assert_eq!(identity["status"], "symbolic_agreement");
    assert_eq!(
        identity["assumptions"]["x"],
        json!({ "positive": true, "real": true })
    );
    assert_eq!(identity["domain"]["common"], "Interval.open(0, oo)");
    assert_eq!(identity["samples"][0]["point"], "2");
    assert!(identity["samples"][0].get("private").is_none());
    assert_eq!(identity["limits"][0]["direction"], "+");
    let stdout = computation["stdout"].as_str().unwrap();
    assert!(stdout.contains("[REDACTED]") && stdout.ends_with('…') && stdout.len() < 1300);
    assert!(!doc.to_string().contains(leaked));
    assert!(!doc.to_string().contains("OUT_OF_SCOPE"));
    let lean = find("lean");
    assert_eq!(lean["via_entry_id"], "e-2");
    assert_eq!(lean["code"], lean_code);
    assert_eq!(lean["axioms"], json!(["propext"]));
    let citation = find("citation");
    assert_eq!(citation["status"], "researcher_recorded");
    assert_eq!(citation["location"], "Section 1");
    assert_eq!(citation["source"]["raw_sha256"], source_blob);
    assert_eq!(citation["source"]["title"], "Finite Sets");
    assert_eq!(citation["source"]["reader"]["source_id"], "notes-1");
    let reader = &find("paper_passages")["passages"][0]["reader"];
    assert_eq!(reader["path"], "notes-1/counting.txt");
    assert_eq!(reader["file_sha256"], source_blob);
    assert_eq!(reader["start_line"], 2);
    assert_eq!(reader["end_line"], 2);
    let (status, session_view) = call("GET", &format!("/api/sessions/{sid}"), Value::Null);
    assert_eq!(status, 200);
    assert_eq!(session_view["linked_sources"], json!([{ "entry_id": "e-6", "reader": reader }]));
    let read_path =
        format!("/api/sessions/{sid}/workspace/file?root=sources&path=notes-1/counting.txt");
    let (status, preview) = call("GET", &read_path, Value::Null);
    assert_eq!(status, 200, "{preview}");
    assert_eq!(preview["file_sha256"], reader["file_sha256"]);
    std::fs::write(source_dir.join("counting.txt"), b"new contents\n").unwrap();
    let (status, changed) = call("GET", &read_path, Value::Null);
    assert_eq!(status, 200, "{changed}");
    assert_ne!(changed["file_sha256"], reader["file_sha256"]);
    assert_eq!(
        call("GET", &format!("{path}?entry_id=missing"), Value::Null).0,
        404
    );
    assert_eq!(
        call("GET", &format!("{path}?entry_id=e-4"), Value::Null).1["entries"][0]["support"][0]
            ["status"],
        "unknown_reference"
    );
    let (_, refined) = call("GET", &format!("{path}?entry_id=e-5"), Value::Null);
    let convergence = &refined["entries"][0]["support"][0]["validation"];
    assert_eq!(convergence["axis"], "resolution");
    assert_eq!(convergence["within_tolerance"], true);
    assert_eq!(convergence["rows_omitted"], true);
    let rows = convergence["samples"].as_array().unwrap();
    assert_eq!(rows.len(), 8);
    assert_eq!(rows[0]["level"], "1");
    assert_eq!(rows[7]["level"], "12");
    assert_eq!(rows[7]["value"], json!({"real":"0.0069444444","imag":"0"}));
    assert!(!refined.to_string().contains(leaked));
}
