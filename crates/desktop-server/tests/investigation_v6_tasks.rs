//! End-to-end desktop HTTP integration tests for Investigate V6 tasks
//! (`POST /api/sessions/{id}/investigation/tasks`, rules 6.1, 6.15, 6.17, 6.19).

mod common;

use common::{raw_http, test_cfg};
use desktop_server::{start_desktop_server, RunningDesktopServer};
use ledger::Ledger;
use research_tools::investigation_markers::{
    EVENT_INVESTIGATION_READING, MARKER_TASK_FINISHED, MARKER_TASK_REQUESTED,
};
use research_tools::lines_sent::CONTEXT_SUPPLIED_EVENT;
use research_tools::map_defs::{READ_MAP_NODE, RECORD_MAP_READING};
use research_tools::{ingest_bytes, IngestRequest, ResearchEnv};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SAMPLE_TEX: &[u8] = br#"\documentclass{article}
\newtheorem{lemma}{Lemma}[section]
\newtheorem{theorem}[lemma]{Theorem}
\begin{document}
\begin{abstract}
We prove spectral stability for a discrete damped wave operator.
\end{abstract}
\section{Introduction}\label{sec:intro}
This section states the discrete energy norm and setup.
\begin{lemma}[Discrete Coercivity]\label{lem:dcoer}
For every mesh vector $v$, the discrete energy satisfies $Q(v) \ge \alpha \|v\|^2$.
\end{lemma}
\begin{theorem}[Spectral Stability]\label{thm:sstab}
Under Lemma~\ref{lem:dcoer}, all eigenmodes decay exponentially in time.
\end{theorem}
\begin{proof}
Combining Lemma~\ref{lem:dcoer} with the discrete damping identity yields exponential mode decay.
\end{proof}
\section{Conclusion}\label{sec:conc}
Discrete coercivity implies unconditional spectral stability.
\end{document}
"#;

struct ScriptedProvider {
    base_url: String,
    seen_requests: Arc<Mutex<Vec<Value>>>,
}

impl ScriptedProvider {
    fn start(replies: Vec<Value>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_bg = Arc::clone(&seen);
        let queue = Arc::new(Mutex::new(std::collections::VecDeque::from(replies)));
        std::thread::spawn(move || {
            while let Ok((mut stream, _)) = listener.accept() {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut content_len = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    if line == "\r\n" || line == "\n" {
                        break;
                    }
                    if let Some((k, v)) = line.split_once(':') {
                        if k.eq_ignore_ascii_case("content-length") {
                            content_len = v.trim().parse().unwrap_or(0);
                        }
                    }
                }
                let mut body = vec![0u8; content_len];
                if content_len > 0 {
                    let _ = reader.read_exact(&mut body);
                }
                if let Ok(val) = serde_json::from_slice::<Value>(&body) {
                    seen_bg.lock().unwrap().push(val);
                }
                let reply = queue
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or_else(|| ant_text("Done."));
                let bytes = serde_json::to_vec(&reply).unwrap();
                let hdr = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                );
                let _ = stream.write_all(hdr.as_bytes());
                let _ = stream.write_all(&bytes);
                let _ = stream.flush();
            }
        });
        Self { base_url: format!("http://{addr}"), seen_requests: seen }
    }
}

fn ant_text(text: &str) -> Value {
    json!({
        "id": "msg_txt", "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
        "stop_reason": "end_turn", "content": [{ "type": "text", "text": text }],
        "usage": { "input_tokens": 12, "output_tokens": 18 }
    })
}

fn ant_tool(id: &str, name: &str, input: Value) -> Value {
    json!({
        "id": format!("msg_{id}"), "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
        "stop_reason": "tool_use", "content": [{ "type": "tool_use", "id": id, "name": name, "input": input }],
        "usage": { "input_tokens": 15, "output_tokens": 25 }
    })
}

fn req(srv: &RunningDesktopServer, method: &str, path: &str, body: Option<&Value>) -> (u16, Value, String) {
    let hdr = [("X-ReSearch-Token", srv.info.token.as_str()), ("Content-Type", "application/json")];
    let bytes = body.map(|v| serde_json::to_vec(v).unwrap()).unwrap_or_default();
    let (st, _, raw) = raw_http(srv.addr.port(), method, path, &hdr, &bytes);
    (st, serde_json::from_slice(&raw).unwrap_or(Value::Null), String::from_utf8_lossy(&raw).into_owned())
}

fn wait_idle(srv: &RunningDesktopServer, sid: &str, min_finished: usize) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let (st, sv, raw) = req(srv, "GET", &format!("/api/sessions/{sid}"), None);
        assert_eq!(st, 200, "GET session failed: {raw}");
        let fin_count = sv["events"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| e["kind"] == MARKER_TASK_FINISHED)
            .count();
        if sv["running"] == json!(false) && fin_count >= min_finished {
            return sv;
        }
        assert!(Instant::now() < deadline, "timed out waiting for task finish: {sv}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn configure_mock_anthropic(srv: &RunningDesktopServer) {
    let (st, _, raw) = req(
        srv,
        "PUT",
        "/api/settings/providers",
        Some(&json!({
            "default_provider": "anthropic",
            "default_model": "claude-haiku-4-5",
            "auto_approve_tools": true,
            "keys": {
                "anthropic": "fixture-ant-key"
            }
        })),
    );
    assert_eq!(st, 200, "provider config failed: {raw}");
}

#[test]
fn v6_tasks_enforce_auth_skeleton_first_and_end_to_end_done_partial_and_mapview() {
    let dir = tempfile::tempdir().unwrap();

    // First start server with no provider needed to verify auth + Rule 6.1 + spoofing guard.
    let srv_gate = start_desktop_server(test_cfg(dir.path(), None, None, None)).unwrap();
    let (st, created, raw) = req(
        &srv_gate,
        "POST",
        "/api/sessions",
        Some(&json!({ "id": "s-v6-http", "mode": "investigate" })),
    );
    assert_eq!(st, 200, "{raw}");
    let sid = created["id"].as_str().unwrap().to_string();

    // 1. Auth guard: 401 without token, 403 on foreign Origin.
    let (unauth, _, _) = raw_http(
        srv_gate.addr.port(),
        "POST",
        &format!("/api/sessions/{sid}/investigation/tasks"),
        &[("Content-Type", "application/json")],
        br#"{"task":"orient"}"#,
    );
    assert_eq!(unauth, 401);
    let (bad_origin, _, _) = raw_http(
        srv_gate.addr.port(),
        "POST",
        &format!("/api/sessions/{sid}/investigation/tasks"),
        &[
            ("X-ReSearch-Token", srv_gate.info.token.as_str()),
            ("Origin", "https://evil.example"),
            ("Content-Type", "application/json"),
        ],
        br#"{"task":"orient"}"#,
    );
    assert_eq!(bad_origin, 403);

    // 2. Rule 6.1: refuse task before pinned skeleton exists; zero task_requested / model.request written.
    let (no_skel_st, _, no_skel_raw) = req(
        &srv_gate,
        "POST",
        &format!("/api/sessions/{sid}/investigation/tasks"),
        Some(&json!({ "task": "orient" })),
    );
    assert_eq!(no_skel_st, 400, "{no_skel_raw}");
    let (pw_st, _, _) = req(
        &srv_gate,
        "POST",
        &format!("/api/sessions/{sid}/investigation/tasks"),
        Some(&json!({ "task": "prior_work" })),
    );
    assert_eq!(pw_st, 400);

    // Ingest and pin TeX skeleton (zero model calls).
    let env = ResearchEnv::new(dir.path());
    let ledger_m = Mutex::new(Ledger::open(dir.path()).unwrap());
    let ing = ingest_bytes(
        &env.sandbox(),
        &ledger_m,
        &IngestRequest {
            session_id: &sid,
            file_name: "spectral.tex",
            origin: "https://arxiv.org/e-print/2406.00099v1",
            raw_bytes: SAMPLE_TEX,
            source_id_hint: None,
            section_filter: None,
            title: Some("Spectral stability"),
        },
    )
    .unwrap();
    drop(ledger_m);

    let (pin_st, pin_val, pin_raw) = req(
        &srv_gate,
        "POST",
        &format!("/api/sessions/{sid}/investigation/skeleton"),
        Some(&json!({ "source_id": ing.source_id, "pin": true })),
    );
    assert_eq!(pin_st, 200, "{pin_raw}");
    let nodes = pin_val["skeleton"]["nodes"].as_array().unwrap();
    let thm = nodes.iter().find(|n| n["kind"] == "theorem").unwrap().clone();
    let lem = nodes.iter().find(|n| n["kind"] == "lemma").unwrap().clone();
    let proof = nodes.iter().find(|n| n["kind"] == "proof").unwrap().clone();
    let thm_tag = thm["tag"].as_str().unwrap().to_string();
    let lem_tag = lem["tag"].as_str().unwrap().to_string();
    let primary_path = ing.primary_path.clone().unwrap();

    // Validation errors on pinned skeleton: unknown tag & empty tags for open.
    let (bad_tag_st, _, bad_tag_raw) = req(
        &srv_gate,
        "POST",
        &format!("/api/sessions/{sid}/investigation/tasks"),
        Some(&json!({ "task": "open", "tags": ["n-doesnotexist"] })),
    );
    assert_eq!(bad_tag_st, 400, "{bad_tag_raw}");
    assert!(bad_tag_raw.contains(&thm_tag) || bad_tag_raw.contains("Valid tags"), "{bad_tag_raw}");

    let (empty_open_st, _, _) = req(
        &srv_gate,
        "POST",
        &format!("/api/sessions/{sid}/investigation/tasks"),
        Some(&json!({ "task": "open", "tags": [] })),
    );
    assert_eq!(empty_open_st, 400);

    // Check ledger still has zero task_requested and zero model.request.
    let pre_evs = Ledger::open(dir.path()).unwrap().read_session(&sid).unwrap();
    assert!(!pre_evs.iter().any(|e| e.kind == MARKER_TASK_REQUESTED || e.kind == "model.request"));
    srv_gate.shutdown();

    // 3. Start server with scripted mock provider for real end-to-end Orient (done) + Open (partial at cap)!
    let expected_orient_seq = pre_evs.last().unwrap().seq + 1;
    let thm_start = thm["spans"][0]["start_line"].as_u64().unwrap();
    let thm_end = thm["spans"][0]["end_line"].as_u64().unwrap();
    let proof_start = proof["spans"][0]["start_line"].as_u64().unwrap();
    let proof_end = proof["spans"][0]["end_line"].as_u64().unwrap();

    // Script for Run 1 (orient, 3 model calls -> done):
    // Turn 1: call read_map_node on Theorem 1.2
    // Turn 2: call record_map_reading with 1 valid theorem item + 1 valid proof edge + 1 bad sibling
    // Turn 3: final text reply
    let replies = vec![
        ant_tool("tu_read", READ_MAP_NODE, json!({ "source_id": ing.source_id, "node": "Theorem 1.2" })),
        ant_tool(
            "tu_rec",
            RECORD_MAP_READING,
            json!({
                "request_seq": expected_orient_seq,
                "source_id": ing.source_id,
                "items": [
                    {
                        "tag": thm_tag,
                        "role": "main_result",
                        "gist": "Discrete coercivity yields exponential mode decay.",
                        "anchors": [{
                            "source_id": ing.source_id,
                            "path": primary_path,
                            "start_line": thm_start,
                            "end_line": thm_end,
                            "quote": "all eigenmodes decay exponentially in time"
                        }]
                    },
                    {
                        "tag": "n-bogus00000",
                        "role": "supporting",
                        "gist": "Should be rejected per-item.",
                        "anchors": [{
                            "source_id": ing.source_id,
                            "path": primary_path,
                            "start_line": thm_start,
                            "end_line": thm_end,
                            "quote": "all eigenmodes decay exponentially in time"
                        }]
                    }
                ],
                "edges": [{
                    "from": lem_tag,
                    "to": thm_tag,
                    "role": "proof",
                    "basis": "explicit",
                    "reason": "Proof combines Lemma 1.1 with damping identity.",
                    "anchor": {
                        "source_id": ing.source_id,
                        "path": primary_path,
                        "start_line": proof_start,
                        "end_line": proof_end,
                        "quote": "Combining Lemma~\\ref{lem:dcoer} with the discrete damping identity"
                    }
                }],
                "limitations": [],
                "done": true
            }),
        ),
        ant_text("Orient completed and saved Theorem 1.2."),
    ];
    let mock = ScriptedProvider::start(replies);
    let srv = start_desktop_server(test_cfg(dir.path(), None, Some(mock.base_url.clone()), None)).unwrap();
    configure_mock_anthropic(&srv);

    let (t_st, t_val, t_raw) = req(
        &srv,
        "POST",
        &format!("/api/sessions/{sid}/investigation/tasks"),
        Some(&json!({ "task": "orient", "focus": "Find the main stability result" })),
    );
    assert_eq!(t_st, 202, "{t_raw}");
    assert_eq!(t_val["request_seq"].as_u64(), Some(expected_orient_seq));
    assert_eq!(t_val["cap"].as_u64(), Some(8));

    let sv1 = wait_idle(&srv, &sid, 1);
    let evs1 = sv1["events"].as_array().unwrap();
    // Verify context.supplied was logged with quoted source material sent to the provider.
    let supplied = evs1.iter().find(|e| e["kind"] == CONTEXT_SUPPLIED_EVENT).expect("context.supplied logged");
    assert_eq!(supplied["payload"]["run_id"], expected_orient_seq.to_string());
    assert!(supplied["payload"]["caller"].as_str().unwrap().starts_with("agent:"));
    assert!(!supplied["payload"]["ranges"].as_array().unwrap().is_empty());

    // Verify provider received quoted_source_material framing (Rule 6.19) and investigation/2 tools!
    let seen = mock.seen_requests.lock().unwrap();
    assert!(!seen.is_empty());
    let req0_str = seen[0].to_string();
    assert!(req0_str.contains("quoted_source_material"), "injected context must be wrapped");
    assert!(req0_str.contains(READ_MAP_NODE));
    assert!(req0_str.contains(RECORD_MAP_READING));
    drop(seen);

    // Verify task_finished recorded status "done" and F9 model_calls == 3.
    let fin1 = evs1.iter().rev().find(|e| e["kind"] == MARKER_TASK_FINISHED).unwrap();
    assert_eq!(fin1["payload"]["status"], "done", "{fin1}");
    assert_eq!(fin1["payload"]["run_state"], "done");
    assert_eq!(fin1["payload"]["counts"]["items"], 1);
    assert_eq!(fin1["payload"]["counts"]["edges"], 1);
    assert_eq!(fin1["payload"]["counts"]["rejected_items"], 1);
    assert_eq!(fin1["payload"]["counts"]["model_calls"], 3);
    assert_eq!(fin1["payload"]["counts"]["cap"], 8);

    // Verify mapview projects folded task, node reading, and model edge!
    let (mv_st, mv_val, mv_raw) = req(&srv, "GET", &format!("/api/sessions/{sid}/investigation/mapview"), None);
    assert_eq!(mv_st, 200, "{mv_raw}");
    assert_eq!(mv_val["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(mv_val["tasks"][0]["run_state"], "done");
    let thm_mv = mv_val["nodes"].as_array().unwrap().iter().find(|n| n["id"] == thm_tag).unwrap();
    assert_eq!(thm_mv["reading"]["role"]["current"]["value"], "main_result");
    assert!(mv_val["edges"].as_array().unwrap().iter().any(|e| e["model_edge"] == true));
    srv.shutdown();

    // 4. Rule 6.15 cap test: open task with max_turns=2 saves 1 item on turn 1, hits cap on turn 2 ->
    //    finishes `partial` / `run_state: "paused"`, preserving the saved record!
    let pre_open_last = Ledger::open(dir.path()).unwrap().read_session(&sid).unwrap().last().unwrap().seq;
    let expected_open_seq = pre_open_last + 1;
    let cap_replies = vec![
        ant_tool(
            "tu_cap_1",
            RECORD_MAP_READING,
            json!({
                "request_seq": expected_open_seq,
                "source_id": ing.source_id,
                "items": [{
                    "tag": thm_tag,
                    "purpose": "Main spectral stability theorem from injected open context.",
                    "anchors": [{
                        "source_id": ing.source_id,
                        "path": primary_path,
                        "start_line": thm_start,
                        "end_line": thm_end,
                        "quote": "all eigenmodes decay exponentially in time"
                    }]
                }],
                "edges": [],
                "limitations": [],
                "done": false
            }),
        ),
        // Turn 2 still asks for a tool -> hits max_turns=2 cap!
        ant_tool("tu_cap_2", READ_MAP_NODE, json!({ "source_id": ing.source_id, "node": lem_tag })),
    ];
    let mock2 = ScriptedProvider::start(cap_replies);
    let srv2 = start_desktop_server(test_cfg(dir.path(), None, Some(mock2.base_url.clone()), None)).unwrap();

    let (o_st, o_val, o_raw) = req(
        &srv2,
        "POST",
        &format!("/api/sessions/{sid}/investigation/tasks"),
        Some(&json!({ "task": "open", "tags": [thm_tag], "max_turns": 2 })),
    );
    assert_eq!(o_st, 202, "{o_raw}");
    assert_eq!(o_val["request_seq"].as_u64(), Some(expected_open_seq));
    assert_eq!(o_val["cap"].as_u64(), Some(2));

    let sv2 = wait_idle(&srv2, &sid, 2);
    let evs2 = sv2["events"].as_array().unwrap();
    let fin2 = evs2.iter().rev().find(|e| e["kind"] == MARKER_TASK_FINISHED).unwrap();
    assert_eq!(fin2["payload"]["request_seq"], expected_open_seq);
    assert_eq!(fin2["payload"]["status"], "partial", "{fin2}");
    assert_eq!(fin2["payload"]["run_state"], "paused");
    assert_eq!(fin2["payload"]["counts"]["items"], 1);
    assert_eq!(fin2["payload"]["counts"]["model_calls"], 2);
    assert_eq!(fin2["payload"]["counts"]["cap"], 2);
    assert_eq!(
        evs2.iter().filter(|e| e["kind"] == EVENT_INVESTIGATION_READING).count(),
        2,
        "saved record from capped run must be preserved"
    );

    // 5. Spoofing guard: /messages with _investigation_task_spec cannot create an investigation.task_requested.
    let before_spoof = evs2.iter().filter(|e| e["kind"] == MARKER_TASK_REQUESTED).count();
    let (sp_st, _, _) = req(
        &srv2,
        "POST",
        &format!("/api/sessions/{sid}/messages"),
        Some(&json!({
            "prompt": "What is Lemma 1.1?",
            "_investigation_task_spec": { "task": "orient", "source_id": ing.source_id, "skeleton_seq": 1 }
        })),
    );
    assert_eq!(sp_st, 202);
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let (_, sv, _) = req(&srv2, "GET", &format!("/api/sessions/{sid}"), None);
        if sv["running"] == json!(false) {
            let after_spoof = sv["events"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|e| e["kind"] == MARKER_TASK_REQUESTED)
                .count();
            assert_eq!(after_spoof, before_spoof, "/messages must strip _investigation_task_spec");
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    srv2.shutdown();
}

#[test]
fn question_turn_credits_read_map_node_and_context_supplied_with_hash_check() {
    let dir = tempfile::tempdir().unwrap();
    let sid = "s-v6-q-cites";

    let env = ResearchEnv::new(dir.path());
    let ledger_m = Mutex::new(Ledger::open(dir.path()).unwrap());
    ledger_m.lock().unwrap().append(sid, "investigation.started", 1, &json!({})).unwrap();
    let ing = ingest_bytes(
        &env.sandbox(),
        &ledger_m,
        &IngestRequest {
            session_id: sid,
            file_name: "spectral.tex",
            origin: "https://arxiv.org/e-print/2406.00100v1",
            raw_bytes: SAMPLE_TEX,
            source_id_hint: None,
            section_filter: None,
            title: Some("Spectral stability"),
        },
    )
    .unwrap();
    drop(ledger_m);

    let srv_pin = start_desktop_server(test_cfg(dir.path(), None, None, None)).unwrap();
    let (pin_st, pin_val, pin_raw) = req(
        &srv_pin,
        "POST",
        &format!("/api/sessions/{sid}/investigation/skeleton"),
        Some(&json!({ "source_id": ing.source_id, "pin": true })),
    );
    assert_eq!(pin_st, 200, "{pin_raw}");
    let thm = pin_val["skeleton"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["kind"] == "theorem")
        .unwrap()
        .clone();
    let thm_start = thm["spans"][0]["start_line"].as_u64().unwrap();
    let thm_end = thm["spans"][0]["end_line"].as_u64().unwrap();
    srv_pin.shutdown();

    // Run a question turn that uses `read_map_node` to read Theorem 1.2 and cites both
    // the read lines and an unread line [L1].
    let replies = vec![
        ant_tool("tu_q_rmn", READ_MAP_NODE, json!({ "source_id": ing.source_id, "node": "Theorem 1.2" })),
        ant_text(&format!(
            "Theorem 1.2 establishes exponential decay [L{thm_start}-{thm_end}] while line [L1] was not read."
        )),
    ];
    let mock = ScriptedProvider::start(replies);
    let srv = start_desktop_server(test_cfg(dir.path(), None, Some(mock.base_url.clone()), None)).unwrap();
    configure_mock_anthropic(&srv);

    let (m_st, _, m_raw) = req(
        &srv,
        "POST",
        &format!("/api/sessions/{sid}/messages"),
        Some(&json!({ "prompt": "What does Theorem 1.2 prove?" })),
    );
    assert_eq!(m_st, 202, "{m_raw}");

    let deadline = Instant::now() + Duration::from_secs(15);
    let answer_ev = loop {
        let (_, sv, _) = req(&srv, "GET", &format!("/api/sessions/{sid}"), None);
        if sv["running"] == json!(false) {
            if let Some(ans) = sv["events"]
                .as_array()
                .unwrap()
                .iter()
                .rev()
                .find(|e| e["kind"] == "investigation.answer")
            {
                break ans.clone();
            }
        }
        assert!(Instant::now() < deadline, "timed out waiting for investigation.answer");
        std::thread::sleep(Duration::from_millis(25));
    };
    let cites = answer_ev["payload"]["cites"].as_array().unwrap();
    let thm_cite = cites
        .iter()
        .find(|c| c["start"].as_u64() == Some(thm_start) && c["end"].as_u64() == Some(thm_end))
        .expect("theorem cite present");
    assert_eq!(thm_cite["read"], json!(true), "read_map_node must mark cited lines read: {cites:?}");
    let l1_cite = cites.iter().find(|c| c["start"].as_u64() == Some(1)).expect("L1 cite present");
    assert_eq!(l1_cite["read"], json!(false), "unread line must stay read=false: {cites:?}");

    srv.shutdown();
}
