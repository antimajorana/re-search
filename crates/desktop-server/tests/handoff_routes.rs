//! Loopback HTTP integration tests for package F7 (`desktop-server/src/handoff.rs`):
//! - Header-token auth and Origin enforcement on `POST /api/sessions/{id}/handoff`
//!   and `GET /api/sessions/{id}/handoffs`;
//! - Receiving-route rules 1..5 across Investigate -> Solve, Investigate -> Explore,
//!   and Explore -> Investigate;
//! - Project inheritance, unbuilt-mode turn refusal on Solve/Explore children,
//!   and strictly read-only status flow-back to the origin session.

mod common;

use common::{launch_cookie, raw_http, test_cfg};
use desktop_server::start_desktop_server;
use ledger::{sha256_hex, Ledger};
use research_tools::quote_core::quote_sha256;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn seed_origin_with_source(
    data_dir: &Path,
    port: u16,
    tok: &str,
    session_id: &str,
    source_id: &str,
    content: &str,
) -> (String, String) {
    let auth = [("X-ReSearch-Token", tok)];
    let body = serde_json::to_vec(&json!({ "id": session_id, "mode": "investigate" })).unwrap();
    let (st, _, raw) = raw_http(port, "POST", "/api/sessions", &auth, &body);
    assert_eq!(st, 200, "create origin session failed: {}", String::from_utf8_lossy(&raw));

    let rel_path = format!("{source_id}/paper.tex");
    let full = data_dir
        .join("sessions")
        .join(session_id)
        .join("sources")
        .join(source_id)
        .join("paper.tex");
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(&full, content).unwrap();
    let file_hash = sha256_hex(content.as_bytes());

    let ledger = Ledger::open_existing(data_dir).unwrap();
    ledger
        .append(
            session_id,
            "source.ingested",
            1,
            &json!({
                "source_id": source_id,
                "kind": "tex",
                "primary_path": rel_path,
                "files": [rel_path],
                "files_sha256": { rel_path.clone(): file_hash.clone() },
                "file_sha256": file_hash.clone(),
                "bytes": content.len(),
            }),
        )
        .unwrap();
    (rel_path, file_hash)
}

#[test]
fn handoff_routes_enforce_auth_origin_and_packet_validation() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let cookie = launch_cookie(port, &tok);
    let auth = [("X-ReSearch-Token", tok.as_str())];

    let _ = seed_origin_with_source(&data_dir, port, &tok, "s-auth-origin", "src-a", "Line 1: A.\n");

    // 401 without token or with stream cookie alone.
    let (st, _, _) = raw_http(port, "GET", "/api/sessions/s-auth-origin/handoffs", &[], b"");
    assert_eq!(st, 401);
    let (st, _, _) = raw_http(
        port,
        "GET",
        "/api/sessions/s-auth-origin/handoffs",
        &[("Cookie", cookie.as_str())],
        b"",
    );
    assert_eq!(st, 401);
    let (st, _, _) = raw_http(port, "POST", "/api/sessions/s-auth-origin/handoff", &[], b"{}");
    assert_eq!(st, 401);

    // 403 on foreign Origin mutation.
    let (st, _, _) = raw_http(
        port,
        "POST",
        "/api/sessions/s-auth-origin/handoff",
        &[("X-ReSearch-Token", tok.as_str()), ("Origin", "http://evil.invalid")],
        b"{}",
    );
    assert_eq!(st, 403);

    // 404 for unknown session.
    let (st, _, _) = raw_http(port, "GET", "/api/sessions/s-does-not-exist/handoffs", &auth, b"");
    assert_eq!(st, 404);

    // 400 when `from.session` does not match route session id.
    let mismatched = serde_json::to_vec(&json!({
        "packet": {
            "from": { "session": "s-other", "mode": "investigate", "id": "n-1" },
            "target_mode": "solve",
            "title": "Mismatch",
            "statement": { "text": "x = 1", "quotes": [] },
            "ask": {
                "kind": "solve",
                "fields": {
                    "task_kind": "prove",
                    "target": { "description": "prove x = 1", "form": "proof" }
                }
            }
        }
    }))
    .unwrap();
    let (st, _, raw) = raw_http(port, "POST", "/api/sessions/s-auth-origin/handoff", &auth, &mismatched);
    assert_eq!(st, 400, "{}", String::from_utf8_lossy(&raw));

    // 400 when a caller tries to smuggle a badge or tier field (Rule 2).
    let smuggled = serde_json::to_vec(&json!({
        "packet": {
            "from": { "session": "s-auth-origin", "mode": "investigate", "id": "n-1" },
            "target_mode": "solve",
            "title": "Smuggled badge",
            "badge": "Verified",
            "statement": { "text": "x = 1", "quotes": [] },
            "ask": {
                "kind": "solve",
                "fields": {
                    "task_kind": "prove",
                    "target": { "description": "prove x = 1", "form": "proof" }
                }
            }
        }
    }))
    .unwrap();
    let (st, _, _) = raw_http(port, "POST", "/api/sessions/s-auth-origin/handoff", &auth, &smuggled);
    assert_eq!(st, 400, "smuggled badge field must be rejected");
}

#[test]
fn handoff_end_to_end_across_modes_projects_quote_reverification_and_readonly_status() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let auth = [("X-ReSearch-Token", tok.as_str())];

    let origin = "s-inv-f7";
    let paper_text = concat!(
        "Line 1: Define kernel K(x) = exp(-x).\n",
        "Line 2: Theorem 1: Integral of K(x) on [0, inf) equals 1.\n",
        "Line 3: End of synthetic fixture.\n"
    );
    let (rel_path, file_hash) =
        seed_origin_with_source(&data_dir, port, &tok, origin, "src-f7", paper_text);

    // Attach origin to a project to verify child sessions inherit project membership.
    let proj_body = serde_json::to_vec(&json!({ "title": "F7 Project" })).unwrap();
    let (st, _, proj_raw) = raw_http(port, "POST", "/api/projects", &auth, &proj_body);
    assert_eq!(st, 201, "{}", String::from_utf8_lossy(&proj_raw));
    let project_id = serde_json::from_slice::<Value>(&proj_raw).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (st, _, _) = raw_http(
        port,
        "PUT",
        &format!("/api/projects/{project_id}/sessions/{origin}"),
        &auth,
        b"",
    );
    assert_eq!(st, 200);

    let good_quote_text = "Line 1: Define kernel K(x) = exp(-x).";
    let bad_quote_text = "Nonexistent fabricated quote line.";

    // 1. Investigate -> Solve hand-over (1 matching quote + 1 tampered quote).
    let solve_req = serde_json::to_vec(&json!({
        "packet": {
            "from": { "session": origin, "mode": "investigate", "id": "n-thm1" },
            "target_mode": "solve",
            "title": "Compute kernel integral",
            "statement": {
                "text": "Integral of K(x) on [0, inf) equals 1.",
                "quotes": []
            },
            "context": [
                {
                    "role": "definition",
                    "from": { "session": origin, "mode": "investigate", "id": "n-def1" },
                    "text": "K(x) = exp(-x)",
                    "quotes": [{
                        "source_id": "src-f7", "path": rel_path,
                        "start_line": 1, "end_line": 1,
                        "file_sha256": file_hash, "quote_sha256": quote_sha256(good_quote_text),
                        "evidence_seq": 42
                    }]
                },
                {
                    "role": "assumption",
                    "from": { "session": origin, "mode": "investigate", "id": "n-a1" },
                    "text": "Unverified assumption",
                    "quotes": [{
                        "source_id": "src-f7", "path": rel_path,
                        "start_line": 2, "end_line": 2,
                        "file_sha256": file_hash, "quote_sha256": quote_sha256(bad_quote_text),
                        "evidence_seq": 43
                    }]
                }
            ],
            "papers": [{ "key": "arxiv:2401.00001", "version": "v1", "ids": { "arxiv": "2401.00001" } }],
            "conventions": "Real x >= 0.",
            "model_text": [{ "label": "Investigate gist", "text": "Direct antiderivative." }],
            "user_notes": ["Check x -> inf limit."],
            "ask": {
                "kind": "solve",
                "fields": {
                    "task_kind": "calculate",
                    "target": { "description": "Compute integral of K(x)", "form": "closed_form" },
                    "symbols": [{ "name": "x", "assumptions": ["real", "nonnegative"] }]
                }
            }
        }
    }))
    .unwrap();
    let (st, _, raw) = raw_http(port, "POST", &format!("/api/sessions/{origin}/handoff"), &auth, &solve_req);
    assert_eq!(st, 201, "{}", String::from_utf8_lossy(&raw));
    let solve_res: Value = serde_json::from_slice(&raw).unwrap();
    let solve_child = solve_res["session_id"].as_str().unwrap().to_string();
    assert_eq!(solve_res["mode"], "solve");
    assert_eq!(solve_res["target_mode_built"], true);
    assert_eq!(solve_res["receipt"]["quotes_matched"], 1);
    assert_eq!(solve_res["receipt"]["quotes_dropped"], 1);
    assert_eq!(solve_res["prefill"]["steps"][0]["kind"], "cited");
    assert_eq!(solve_res["prefill"]["steps"][1]["kind"], "given");
    assert_eq!(solve_res["prefill"]["model_notes"][0]["label"], "Investigate gist (model)");
    assert!(solve_res["receipt"]["packet"]["context"][0]["quotes"][0]["evidence_seq"].is_null());

    // Verify child reports `mode: "solve"`, inherits project, and refuses model turns cleanly.
    let (st, _, child_raw) = raw_http(port, "GET", &format!("/api/sessions/{solve_child}"), &auth, b"");
    assert_eq!(st, 200);
    let child_val: Value = serde_json::from_slice(&child_raw).unwrap();
    assert_eq!(child_val["mode"], "solve");
    assert_eq!(child_val["project_id"], project_id);

    let ledger = Ledger::open_existing(&data_dir).unwrap();
    let child_len_before = ledger.read_session(&solve_child).unwrap().len();
    let msg_body = serde_json::to_vec(&json!({ "text": "Start solving" })).unwrap();
    let (st, _, msg_raw) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{solve_child}/messages"),
        &auth,
        &msg_body,
    );
    assert_eq!(st, 400, "unconfigured Solve child must refuse turns before setup");
    assert!(String::from_utf8_lossy(&msg_raw).contains("setup"));
    assert_eq!(ledger.read_session(&solve_child).unwrap().len(), child_len_before);

    // 2. Investigate -> Explore (PriorWork) with origin_paper excluded.
    let explore_req = serde_json::to_vec(&json!({
        "packet": {
            "from": { "session": origin, "mode": "investigate", "id": "n-thm1" },
            "target_mode": "explore",
            "title": "Prior work on kernel integral",
            "statement": {
                "text": "Integral of K(x) on [0, inf) equals 1.",
                "quotes": [{
                    "source_id": "src-f7", "path": rel_path,
                    "start_line": 1, "end_line": 1,
                    "file_sha256": file_hash, "quote_sha256": quote_sha256(good_quote_text)
                }]
            },
            "ask": {
                "kind": "explore",
                "fields": {
                    "idea_text": "Is Theorem 1 already in the literature?",
                    "question_kind": "prior_work",
                    "origin_paper": { "key": "arxiv:2401.00001", "version": "v1", "ids": { "arxiv": "2401.00001" } },
                    "seed_papers": [{ "key": "arxiv:2312.00002", "version": "v1", "ids": { "arxiv": "2312.00002" } }]
                }
            }
        }
    }))
    .unwrap();
    let (st, _, raw) = raw_http(port, "POST", &format!("/api/sessions/{origin}/handoff"), &auth, &explore_req);
    assert_eq!(st, 201, "{}", String::from_utf8_lossy(&raw));
    let exp_res: Value = serde_json::from_slice(&raw).unwrap();
    let exp_child = exp_res["session_id"].as_str().unwrap().to_string();
    assert_eq!(exp_res["mode"], "explore");
    assert_eq!(exp_res["prefill"]["excluded_origin_paper"]["key"], "arxiv:2401.00001");

    // 3. Explore -> Investigate hand-over (target mode IS built).
    let inv_req = serde_json::to_vec(&json!({
        "packet": {
            "from": { "session": exp_child, "mode": "explore", "id": "xp-1" },
            "target_mode": "investigate",
            "title": "Investigate matched paper",
            "statement": { "text": "Inspect kernel definition.", "quotes": [] },
            "ask": {
                "kind": "investigate",
                "fields": {
                    "paper": { "key": "arxiv:2401.00001", "version": "v1", "ids": { "arxiv": "2401.00001" } },
                    "focus_claims": ["Theorem 1"],
                    "earlier_quotes": [{
                        "source_id": "src-f7", "path": rel_path,
                        "start_line": 1, "end_line": 1,
                        "file_sha256": file_hash, "quote_sha256": quote_sha256(good_quote_text)
                    }]
                }
            }
        }
    }))
    .unwrap();
    let (st, _, raw) = raw_http(port, "POST", &format!("/api/sessions/{exp_child}/handoff"), &auth, &inv_req);
    assert_eq!(st, 201, "{}", String::from_utf8_lossy(&raw));
    let inv_res: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(inv_res["mode"], "investigation");
    assert_eq!(inv_res["target_mode_built"], true);
    assert_eq!(inv_res["receipt"]["quotes_matched"], 1);

    // 4. Rule 5: child status flows back to origin strictly read-only.
    let origin_before = ledger.read_session(origin).unwrap();
    let head_before = origin_before.last().unwrap().event_hash.clone();
    ledger
        .append(&solve_child, "solve.done", 1, &json!({ "outcome": "finished" }))
        .unwrap();
    ledger
        .append(&exp_child, "idea.verdict", 1, &json!({ "status": "judged_partial" }))
        .unwrap();

    let (st, _, hs_raw) = raw_http(port, "GET", &format!("/api/sessions/{origin}/handoffs"), &auth, b"");
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&hs_raw));
    let hs: Value = serde_json::from_slice(&hs_raw).unwrap();
    assert_eq!(hs["outgoing"].as_array().unwrap().len(), 2);
    assert_eq!(hs["outgoing"][0]["child_status"]["label"], "Opened in Solve: Done");
    assert_eq!(hs["outgoing"][1]["child_status"]["label"], "Opened in Explore: judged_partial");

    // Origin ledger is untouched by reading status.
    let origin_after = ledger.read_session(origin).unwrap();
    assert_eq!(origin_after.len(), origin_before.len());
    assert_eq!(origin_after.last().unwrap().event_hash, head_before);
}
