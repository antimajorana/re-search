//! End-to-end HTTP + SSE integration tests for Investigate final-turn submission,
//! clamped preamble cap, `Partial` non-error completion, `orient_call_cap` settings,
//! `continue_from` resumption, and pinned-paper title preservation.

mod common;

use common::{launch_cookie, raw_http, test_cfg};
use desktop_server::{start_desktop_server, RunningDesktopServer};
use ledger::Ledger;
use research_tools::investigation_markers::{MARKER_TASK_FINISHED, MARKER_TASK_REQUESTED};
use research_tools::map_defs::{READ_MAP_NODE, RECORD_CONTEXT_CARDS, RECORD_MAP_READING};
use research_tools::{ingest_bytes, IngestRequest, ResearchEnv};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

const PAPER_TEX: &[u8] = br#"\documentclass{article}
\newtheorem{lemma}{Lemma}[section]
\newtheorem{theorem}[lemma]{Theorem}
\begin{document}
\begin{abstract}
We prove a sharp contraction bound for finite chains.
\end{abstract}
\section{Introduction}\label{sec:intro}
We establish Theorem~\ref{thm:main} using Lemma~\ref{lem:step}.
\begin{lemma}[Step Bound]\label{lem:step}
Every step contracts the weighted seminorm by a factor $\theta < 1$.
\end{lemma}
\begin{theorem}[Main Contraction]\label{thm:main}
We prove that under Lemma~\ref{lem:step}, the chain converges geometrically.
\end{theorem}
\section{Conclusion}\label{sec:conc}
Geometric convergence holds uniformly across finite chains.
\end{document}
"#;

struct ScriptedAnthropic {
    base_url: String,
    replies: Arc<Mutex<VecDeque<Value>>>,
    seen_requests: Arc<Mutex<Vec<Value>>>,
}

impl ScriptedAnthropic {
    fn start(initial: Vec<Value>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let queue = Arc::new(Mutex::new(VecDeque::from(initial)));
        let q_bg = Arc::clone(&queue);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_bg = Arc::clone(&seen);
        std::thread::spawn(move || {
            while let Ok((mut stream, _)) = listener.accept() {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut content_len = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" || line == "\n" {
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
                let body_str = String::from_utf8_lossy(&body).into_owned();
                let parsed: Value = serde_json::from_str(&body_str).unwrap_or(Value::Null);
                seen_bg.lock().unwrap().push(parsed);
                let mut reply = q_bg.lock().unwrap().pop_front().unwrap_or_else(|| ant_text("Done."));
                if let Some(rseq) = extract_request_seq(&body_str) {
                    patch_zero_request_seq(&mut reply, rseq);
                }
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
        Self { base_url: format!("http://{addr}"), replies: queue, seen_requests: seen }
    }

    fn enqueue(&self, more: Vec<Value>) {
        self.replies.lock().unwrap().extend(more);
    }
}

fn extract_request_seq(body: &str) -> Option<u64> {
    let idx = body.rfind("request_seq=")?;
    body[idx + "request_seq=".len()..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()
}

fn patch_zero_request_seq(val: &mut Value, seq: u64) {
    match val {
        Value::Object(map) => {
            if map.get("request_seq").and_then(Value::as_u64) == Some(0) {
                map.insert("request_seq".into(), json!(seq));
            }
            for child in map.values_mut() {
                patch_zero_request_seq(child, seq);
            }
        }
        Value::Array(items) => {
            for child in items {
                patch_zero_request_seq(child, seq);
            }
        }
        _ => {}
    }
}

fn ant_text(text: &str) -> Value {
    json!({
        "id": "msg_txt", "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
        "stop_reason": "end_turn", "content": [{ "type": "text", "text": text }],
        "usage": { "input_tokens": 10, "output_tokens": 12 }
    })
}

fn ant_tool(id: &str, name: &str, input: Value) -> Value {
    json!({
        "id": format!("msg_{id}"), "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
        "stop_reason": "tool_use", "content": [{ "type": "tool_use", "id": id, "name": name, "input": input }],
        "usage": { "input_tokens": 16, "output_tokens": 24 }
    })
}

fn req(srv: &RunningDesktopServer, method: &str, path: &str, body: Option<&Value>) -> (u16, Value, String) {
    let hdr = [("X-ReSearch-Token", srv.info.token.as_str()), ("Content-Type", "application/json")];
    let bytes = body.map(|v| serde_json::to_vec(v).unwrap()).unwrap_or_default();
    let (st, _, raw) = raw_http(srv.addr.port(), method, path, &hdr, &bytes);
    (st, serde_json::from_slice(&raw).unwrap_or(Value::Null), String::from_utf8_lossy(&raw).into_owned())
}

fn open_sse(srv: &RunningDesktopServer, sid: &str) -> mpsc::Receiver<(String, Value)> {
    let port = srv.addr.port();
    let cookie = launch_cookie(port, &srv.info.token);
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
    let http = format!("GET /api/sessions/{sid}/events HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nCookie: {cookie}\r\n\r\n");
    stream.write_all(http.as_bytes()).unwrap();
    stream.flush().unwrap();
    let (tx, rx) = mpsc::channel::<(String, Value)>();
    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" || line == "\n" {
                break;
            }
        }
        let _ = ready_tx.send(());
        let mut ev_name = String::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if let Some(rest) = trimmed.strip_prefix("event:") {
                ev_name = rest.trim().to_string();
            } else if let Some(rest) = trimmed.strip_prefix("data:") {
                let data: Value = serde_json::from_str(rest.trim()).unwrap_or(Value::Null);
                if tx.send((ev_name.clone(), data)).is_err() {
                    break;
                }
            }
        }
    });
    ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    rx
}

fn wait_sse_turn_end(rx: &mpsc::Receiver<(String, Value)>) -> (String, Value) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "timed out waiting for SSE turn end");
        let (ev, data) = rx.recv_timeout(left).unwrap();
        if ev == "turn_complete" || ev == "turn_error" {
            return (ev, data);
        }
    }
}

fn wait_task_finish(srv: &RunningDesktopServer, sid: &str, min_finished: usize) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let (st, sv, raw) = req(srv, "GET", &format!("/api/sessions/{sid}"), None);
        assert_eq!(st, 200, "{raw}");
        let count = sv["events"].as_array().into_iter().flatten()
            .filter(|e| e["kind"] == MARKER_TASK_FINISHED).count();
        if sv["running"] == json!(false) && count >= min_finished {
            return sv;
        }
        assert!(Instant::now() < deadline, "timed out waiting for finished task: {sv}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn line_of(needle: &str) -> usize {
    std::str::from_utf8(PAPER_TEX).unwrap().lines().enumerate()
        .find_map(|(i, l)| l.contains(needle).then_some(i + 1))
        .unwrap()
}

struct Harness {
    _dir: tempfile::TempDir,
    provider: ScriptedAnthropic,
    srv: RunningDesktopServer,
    sid: String,
    source_id: String,
    primary_path: String,
    thm_tag: String,
    lem_tag: String,
}

fn boot(id: &str) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let provider = ScriptedAnthropic::start(vec![]);
    let srv = start_desktop_server(test_cfg(dir.path(), None, Some(provider.base_url.clone()), None)).unwrap();
    let (st, _, raw) = req(&srv, "PUT", "/api/settings/providers", Some(&json!({
        "default_provider": "anthropic", "default_model": "claude-haiku-4-5",
        "auto_approve_tools": true, "keys": { "anthropic": "fixture-key" }
    })));
    assert_eq!(st, 200, "{raw}");
    let (st, created, raw) = req(&srv, "POST", "/api/sessions", Some(&json!({ "id": id, "mode": "investigate" })));
    assert_eq!(st, 200, "{raw}");
    let sid = created["id"].as_str().unwrap().to_string();

    let env = ResearchEnv::new(dir.path());
    let ledger_m = Mutex::new(Ledger::open(dir.path()).unwrap());
    let ing = ingest_bytes(&env.sandbox(), &ledger_m, &IngestRequest {
        session_id: &sid, file_name: "contraction.tex",
        origin: "https://arxiv.org/e-print/2501.00077v1", raw_bytes: PAPER_TEX,
        source_id_hint: None, section_filter: None, title: Some("Contraction Bound"),
    }).unwrap();
    drop(ledger_m);

    let (st, pin_val, raw) = req(
        &srv, "POST", &format!("/api/sessions/{sid}/investigation/skeleton"),
        Some(&json!({ "source_id": ing.source_id, "pin": true })),
    );
    assert_eq!(st, 200, "{raw}");
    let nodes = pin_val["skeleton"]["nodes"].as_array().unwrap();
    let find_tag = |lbl: &str| nodes.iter().find(|n| n["label"].as_str() == Some(lbl)).unwrap()["tag"].as_str().unwrap().to_string();

    Harness {
        _dir: dir, provider, srv, sid, source_id: ing.source_id,
        primary_path: ing.primary_path.unwrap(),
        thm_tag: find_tag("thm:main"), lem_tag: find_tag("lem:step"),
    }
}

#[test]
fn orient_max_turns_one_dispatches_final_record_map_reading_and_marks_done_with_turn_complete() {
    let h = boot("s-final-turn-1");
    let sse = open_sse(&h.srv, &h.sid);
    let thm_l = line_of("We prove that under Lemma");
    let anchor = json!({
        "source_id": h.source_id, "path": h.primary_path,
        "start_line": thm_l, "end_line": thm_l, "quote": "We prove that under Lemma"
    });
    h.provider.enqueue(vec![ant_tool("t_final", RECORD_MAP_READING, json!({
        "request_seq": 0,
        "source_id": h.source_id,
        "items": [{
            "tag": h.thm_tag, "role": "main_result",
            "gist": "Establishes geometric convergence of finite chains.",
            "anchors": [anchor.clone()],
            "authors_claim": { "label": "new", "anchors": [anchor] }
        }],
        "edges": [], "limitations": [], "done": true
    }))]);

    let (st, started, raw) = req(
        &h.srv, "POST", &format!("/api/sessions/{}/investigation/tasks", h.sid),
        Some(&json!({ "task": "orient", "max_turns": 1 })),
    );
    assert_eq!(st, 202, "{raw}");
    assert_eq!(started["cap"], json!(1));

    let (ev_name, ev_data) = wait_sse_turn_end(&sse);
    assert_eq!(ev_name, "turn_complete", "final-turn done must not emit turn_error: {ev_data}");
    assert_eq!(ev_data["text"], json!(""));

    let sv = wait_task_finish(&h.srv, &h.sid, 1);
    let fin = sv["events"].as_array().unwrap().iter().rev()
        .find(|e| e["kind"] == MARKER_TASK_FINISHED).unwrap();
    assert_eq!(fin["payload"]["status"], json!("done"));
    assert_eq!(fin["payload"]["run_state"], json!("done"));
    assert!(fin["payload"]["error"].is_null());

    // Final-turn provider request filters tools strictly to final_turn_tools and omits model title tool.
    let seen = h.provider.seen_requests.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let tool_names: Vec<&str> = seen[0]["tools"].as_array().unwrap().iter()
        .filter_map(|t| t["name"].as_str()).collect();
    assert_eq!(tool_names.len(), 2, "unexpected tools on final turn: {tool_names:?}");
    assert!(tool_names.contains(&RECORD_MAP_READING) && tool_names.contains(&RECORD_CONTEXT_CARDS));
    let req_str = seen[0].to_string();
    assert!(req_str.contains("call_cap=1") && req_str.contains("cap 1 model calls"));

    let (st, mv, _) = req(&h.srv, "GET", &format!("/api/sessions/{}/investigation/mapview", h.sid), None);
    assert_eq!(st, 200);
    assert_eq!(mv["tasks"][0]["run_state"], json!("done"));
    assert_eq!(mv["orient_call_cap"], json!(8));
    h.srv.shutdown();
}

#[test]
fn orient_cap_two_settings_partial_non_error_and_continue_from() {
    let h = boot("s-final-turn-2");

    // 1. Settings: default orient_call_cap=8, update to 12, reject out-of-range 3 and 25.
    let (st, s0, _) = req(&h.srv, "GET", "/api/settings/investigate", None);
    assert_eq!(st, 200);
    assert_eq!(s0["orient_call_cap"], json!(8));
    assert_eq!(req(&h.srv, "PUT", "/api/settings/investigate", Some(&json!({ "orient_call_cap": 3 }))).0, 400);
    assert_eq!(req(&h.srv, "PUT", "/api/settings/investigate", Some(&json!({ "orient_call_cap": 25 }))).0, 400);
    assert_eq!(req(&h.srv, "PUT", "/api/settings/investigate", Some(&json!({ "orient_call_cap": "12" }))).0, 400);
    let (st, s1, _) = req(&h.srv, "PUT", "/api/settings/investigate", Some(&json!({ "orient_call_cap": 12 })));
    assert_eq!(st, 200);
    assert_eq!(s1["orient_call_cap"], json!(12));
    let (_, mv0, _) = req(&h.srv, "GET", &format!("/api/sessions/{}/investigation/mapview", h.sid), None);
    assert_eq!(mv0["orient_call_cap"], json!(12));

    // 2. max_turns=2 Orient run saves 1 item with done:false then hits cap -> Partial + turn_complete.
    let sse = open_sse(&h.srv, &h.sid);
    let thm_l = line_of("We prove that under Lemma");
    let lem_l = line_of("Every step contracts");
    let thm_a = json!({ "source_id": h.source_id, "path": h.primary_path, "start_line": thm_l, "end_line": thm_l, "quote": "We prove that under Lemma" });
    let lem_a = json!({ "source_id": h.source_id, "path": h.primary_path, "start_line": lem_l, "end_line": lem_l, "quote": "Every step contracts" });
    h.provider.enqueue(vec![
        ant_tool("t_p1", RECORD_MAP_READING, json!({
            "request_seq": 0, "source_id": h.source_id,
            "items": [{ "tag": h.thm_tag, "role": "main_result", "gist": "Main theorem.", "anchors": [thm_a] }],
            "edges": [], "limitations": [], "done": false
        })),
        ant_tool("t_p2", READ_MAP_NODE, json!({ "source_id": h.source_id, "node": h.lem_tag })),
    ]);

    let (st, started, raw) = req(
        &h.srv, "POST", &format!("/api/sessions/{}/investigation/tasks", h.sid),
        Some(&json!({ "task": "orient", "max_turns": 2 })),
    );
    assert_eq!(st, 202, "{raw}");
    assert_eq!(started["cap"], json!(2));
    let partial_seq = started["request_seq"].as_u64().unwrap();

    let (ev_name, ev_data) = wait_sse_turn_end(&sse);
    assert_eq!(ev_name, "turn_complete", "Partial must emit turn_complete, not turn_error: {ev_data}");
    assert_eq!(ev_data["text"], json!(""));
    let sv1 = wait_task_finish(&h.srv, &h.sid, 1);
    let fin1 = sv1["events"].as_array().unwrap().iter().rev()
        .find(|e| e["kind"] == MARKER_TASK_FINISHED).unwrap();
    assert_eq!(fin1["payload"]["status"], json!("partial"));
    assert_eq!(fin1["payload"]["run_state"], json!("paused"));
    assert!(fin1["payload"]["error"].as_str().unwrap_or("").starts_with("reached max_turns ("));

    // Verify injected preamble printed `call_cap=2` and `cap 2 model calls`, and pinned paper omits `set_session_title` even when max_turns > 1.
    let first_req = h.provider.seen_requests.lock().unwrap()[0].clone();
    let first_req_str = first_req.to_string();
    assert!(first_req_str.contains("call_cap=2") && first_req_str.contains("cap 2 model calls"));
    assert!(!first_req_str.contains("call_cap=8") && !first_req_str.contains("call_cap=12"));
    let first_tools: Vec<&str> = first_req["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(
        !first_tools.contains(&"set_session_title"),
        "pinned-paper Investigate turn with max_turns=2 must omit set_session_title: {first_tools:?}"
    );

    // 3. continue_from validation: invalid rejected 400; valid partial accepted 202 & recorded.
    assert_eq!(req(&h.srv, "POST", &format!("/api/sessions/{}/investigation/tasks", h.sid), Some(&json!({ "task": "orient", "continue_from": 0 }))).0, 400);
    assert_eq!(req(&h.srv, "POST", &format!("/api/sessions/{}/investigation/tasks", h.sid), Some(&json!({ "task": "orient", "continue_from": 99999 }))).0, 400);

    h.provider.enqueue(vec![ant_tool("t_c1", RECORD_MAP_READING, json!({
        "request_seq": 0, "source_id": h.source_id,
        "items": [{ "tag": h.lem_tag, "role": "supporting", "gist": "Step contraction.", "anchors": [lem_a] }],
        "edges": [], "limitations": [], "done": true
    }))]);
    let (st, cont_started, raw) = req(
        &h.srv, "POST", &format!("/api/sessions/{}/investigation/tasks", h.sid),
        Some(&json!({ "task": "orient", "continue_from": partial_seq, "max_turns": 1 })),
    );
    assert_eq!(st, 202, "{raw}");
    let cont_seq = cont_started["request_seq"].as_u64().unwrap();
    let (ev2, _) = wait_sse_turn_end(&sse);
    assert_eq!(ev2, "turn_complete");
    let sv2 = wait_task_finish(&h.srv, &h.sid, 2);

    let cont_req_ev = sv2["events"].as_array().unwrap().iter()
        .find(|e| e["seq"] == json!(cont_seq) && e["kind"] == MARKER_TASK_REQUESTED).unwrap();
    assert_eq!(cont_req_ev["payload"]["continue_from"], json!(partial_seq));

    let cont_provider_req = h.provider.seen_requests.lock().unwrap().last().unwrap().to_string();
    assert!(cont_provider_req.contains("Already saved readings for this paper"));
    assert!(cont_provider_req.contains(&h.thm_tag));

    // Continuing from a `done` Orient task must be rejected 400.
    assert_eq!(
        req(&h.srv, "POST", &format!("/api/sessions/{}/investigation/tasks", h.sid), Some(&json!({ "task": "orient", "continue_from": cont_seq }))).0,
        400
    );
    h.srv.shutdown();
}
