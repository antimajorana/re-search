//! Stage 13 integration tests:
//! - Default-on `auto_approve_tools` + manual opt-out toggle in settings
//! - Per-agent model assignments (`agent_models`), validation, and routing for Narrator / Explain
//! - Explain conversation export (`format=explain_md`, `format=explain_html`) + XSS escaping + cookie access
//! - Proof Narrator bug fixes (empty-session non-bricking rejection, Direct-mode derivation fallback, persistence)
//! - Multi-turn bug fixes (upload before Turn 1, Turn 2+ Verified mode prompt propagation)

mod common;

use common::{launch_cookie, raw_http, test_cfg};
use desktop_server::start_desktop_server;
use ledger::Ledger;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;

type CapturedHandler = Box<dyn Fn(&str, &Value) -> Value + Send + Sync>;

struct ScriptedMock {
    base_url: String,
    seen: Arc<Mutex<Vec<(String, Value)>>>,
}

impl ScriptedMock {
    fn start(handler: CapturedHandler) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_cl = Arc::clone(&seen);
        let handler = Arc::new(handler);

        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let handler = Arc::clone(&handler);
                let seen_inner = Arc::clone(&seen_cl);
                std::thread::spawn(move || {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                    let mut reader = BufReader::new(stream);
                    let mut req_line = String::new();
                    if reader.read_line(&mut req_line).unwrap_or(0) == 0 {
                        return;
                    }
                    let path = req_line.split_whitespace().nth(1).unwrap_or("").to_string();
                    let mut content_len = 0usize;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                            break;
                        }
                        if let Some((k, v)) = line.split_once(':') {
                            if k.eq_ignore_ascii_case("content-length") {
                                content_len = v.trim().parse().unwrap_or(0);
                            }
                        }
                    }
                    let mut buf = vec![0u8; content_len];
                    let _ = reader.read_exact(&mut buf);
                    let body: Value = serde_json::from_slice(&buf).unwrap_or(Value::Null);
                    seen_inner.lock().unwrap().push((path.clone(), body.clone()));
                    let reply = handler(&path, &body);
                    let bytes = serde_json::to_vec(&reply).unwrap();
                    let mut s = reader.into_inner();
                    let hdr = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        bytes.len()
                    );
                    let _ = s.write_all(hdr.as_bytes());
                    let _ = s.write_all(&bytes);
                    let _ = s.flush();
                });
            }
        });

        Self {
            base_url: format!("http://{addr}"),
            seen,
        }
    }

    fn captured(&self) -> Vec<(String, Value)> {
        self.seen.lock().unwrap().clone()
    }
}

fn anthropic_text(model: &str, text: &str) -> Value {
    json!({
        "id": "msg_1",
        "type": "message",
        "role": "assistant",
        "model": model,
        "stop_reason": "end_turn",
        "content": [{ "type": "text", "text": text }],
        "usage": { "input_tokens": 12, "output_tokens": 18 }
    })
}

fn anthropic_tool(model: &str, call_id: &str, name: &str, input: Value) -> Value {
    json!({
        "id": "msg_t",
        "type": "message",
        "role": "assistant",
        "model": model,
        "stop_reason": "tool_use",
        "content": [{ "type": "tool_use", "id": call_id, "name": name, "input": input }],
        "usage": { "input_tokens": 12, "output_tokens": 18 }
    })
}

fn openai_text(model: &str, text: &str) -> Value {
    json!({
        "id": "chatcmpl_1",
        "model": model,
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": text },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 15, "completion_tokens": 25 }
    })
}

fn clear_provider_env() {
    for p in [
        agent::ProviderKind::Anthropic,
        agent::ProviderKind::OpenAi,
        agent::ProviderKind::Gemini,
        agent::ProviderKind::Ollama,
    ] {
        for name in p.key_env_vars() {
            std::env::remove_var(name);
        }
    }
    std::env::remove_var("RE_SEARCH_API_KEY");
    std::env::remove_var("RESEARCH_CORE_API_KEY");
}

#[test]
fn default_auto_approve_runs_tool_without_prompt_and_toggle_off_restores_cards() {
    clear_provider_env();
    let seq = Arc::new(Mutex::new(0usize));
    let seq_cl = Arc::clone(&seq);
    let mock = ScriptedMock::start(Box::new(move |_path, body| {
        let has_tool_result = body["messages"]
            .as_array()
            .and_then(|arr| arr.last())
            .and_then(|m| m["content"].as_array())
            .is_some_and(|blocks| blocks.iter().any(|b| b["type"] == "tool_result"));
        if has_tool_result {
            return anthropic_text("claude-haiku-4-5", "Write completed cleanly.");
        }
        let mut idx = seq_cl.lock().unwrap();
        *idx += 1;
        if *idx == 1 {
            anthropic_tool(
                "claude-haiku-4-5",
                "tu_auto",
                "fs_write",
                json!({ "path": "derived.tex", "content": "Auto\n" }),
            )
        } else {
            anthropic_tool(
                "claude-haiku-4-5",
                "tu_manual",
                "fs_write",
                json!({ "path": "manual.tex", "content": "Manual\n" }),
            )
        }
    }));

    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ws_dir = tmp.path().join("ws");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&ws_dir).unwrap();
    std::fs::write(
        data_dir.join("providers.json"),
        serde_json::to_vec(&json!({ "providers": { "anthropic": { "api_key": "sk-ant-stage13-auto" } } })).unwrap(),
    )
    .unwrap();

    let srv = start_desktop_server(test_cfg(
        &data_dir,
        Some(ws_dir.clone()),
        Some(mock.base_url.clone()),
        None,
    ))
    .unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let cookie = launch_cookie(port, &tok);
    let hdr = [("X-ReSearch-Token", tok.as_str()), ("Content-Type", "application/json")];

    // 1. Default setting is true.
    let (st_set, _, set_raw) = raw_http(port, "GET", "/api/settings/providers", &hdr, b"");
    assert_eq!(st_set, 200);
    assert_eq!(serde_json::from_slice::<Value>(&set_raw).unwrap()["auto_approve_tools"], json!(true));

    let (_, _, cr) = raw_http(
        port,
        "POST",
        "/api/sessions",
        &hdr,
        serde_json::to_vec(&json!({ "title": "Auto Approve", "workspace_root": ws_dir }))
            .unwrap()
            .as_slice(),
    );
    let sid = serde_json::from_slice::<Value>(&cr).unwrap()["id"].as_str().unwrap().to_string();

    let mut sse = TcpStream::connect(("127.0.0.1", port)).unwrap();
    sse.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(
        sse,
        "GET /api/sessions/{sid}/events HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nCookie: {cookie}\r\n\r\n"
    )
    .unwrap();
    let mut reader = BufReader::new(sse);

    // 2. Turn 1: auto-approved -> zero approval_requested events, file written, turn_complete arrives.
    let m1 = serde_json::to_vec(&json!({
        "prompt": "Write derived.tex",
        "provider": "anthropic",
        "model": "claude-haiku-4-5"
    }))
    .unwrap();
    assert_eq!(raw_http(port, "POST", &format!("/api/sessions/{sid}/messages"), &hdr, &m1).0, 202);

    let mut saw_auto_resolved = false;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(!line.contains("event: approval_requested"), "unexpected approval prompt while auto_approve_tools=true: {line}");
        if line.contains("\"auto_approved\":true") {
            saw_auto_resolved = true;
        }
        if line.contains("event: turn_complete") {
            break;
        }
    }
    assert!(saw_auto_resolved, "expected SSE approval_resolved with auto_approved: true");
    assert_eq!(std::fs::read_to_string(ws_dir.join("derived.tex")).unwrap(), "Auto\n");

    // 3. Turn off auto_approve_tools -> Turn 2 requires explicit UI approval again.
    let (st_put, _, put_raw) = raw_http(
        port,
        "PUT",
        "/api/settings/providers",
        &hdr,
        br#"{"auto_approve_tools":false}"#,
    );
    assert_eq!(st_put, 200);
    assert_eq!(serde_json::from_slice::<Value>(&put_raw).unwrap()["auto_approve_tools"], json!(false));

    let m2 = serde_json::to_vec(&json!({
        "prompt": "Write manual.tex",
        "provider": "anthropic",
        "model": "claude-haiku-4-5"
    }))
    .unwrap();
    assert_eq!(raw_http(port, "POST", &format!("/api/sessions/{sid}/messages"), &hdr, &m2).0, 202);

    let mut saw_manual_prompt = false;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if let Some(rest) = line.strip_prefix("data: ") {
            if let Ok(v) = serde_json::from_str::<Value>(rest.trim()) {
                if v.get("tool_id") == Some(&json!("fs_write")) && v.get("call_id").is_some() && v.get("edit_preview").is_some() {
                    saw_manual_prompt = true;
                    let cid = v["call_id"].as_str().unwrap();
                    let dig = v["digest"].as_str().unwrap();
                    let ap_body = serde_json::to_vec(&json!({ "digest": dig, "approved": true })).unwrap();
                    assert_eq!(
                        raw_http(port, "POST", &format!("/api/sessions/{sid}/approvals/{cid}"), &hdr, &ap_body).0,
                        200
                    );
                }
                if v.get("text").is_some() && saw_manual_prompt && line.contains("Write completed cleanly.") {
                    break;
                }
            }
        }
        if line.contains("event: turn_complete") && saw_manual_prompt {
            break;
        }
    }
    assert!(saw_manual_prompt);
    assert_eq!(std::fs::read_to_string(ws_dir.join("manual.tex")).unwrap(), "Manual\n");

    let ledger = Ledger::open_existing(&data_dir).unwrap();
    assert!(ledger.verify_session(&sid).is_ok());
    srv.shutdown();
}

#[test]
fn per_agent_models_explain_export_and_narrator_lifecycle() {
    clear_provider_env();
    let mock_ant = ScriptedMock::start(Box::new(|_path, body| {
        let model = body["model"].as_str().unwrap_or("claude-haiku-4-5");
        anthropic_text(model, "Direct derivation: $E = \\hbar \\omega$ in step 1.")
    }));
    let mock_oai = ScriptedMock::start(Box::new(|_path, body| {
        let model = body["model"].as_str().unwrap_or("gpt-4.1-mini");
        let sys = body["messages"][0]["content"].as_str().unwrap_or("");
        if sys.contains("Proof Narrator") {
            openai_text(model, "Walkthrough narrative of the harmonic oscillator energy.")
        } else {
            openai_text(model, "Explain: $\\hbar \\omega$ comes from ladder operators in `e-1`. <script>safe</script>")
        }
    }));

    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, None, None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let cookie = launch_cookie(port, &tok);
    let hdr = [("X-ReSearch-Token", tok.as_str()), ("Content-Type", "application/json")];

    // 1. Validate agent_models rejections.
    assert_eq!(
        raw_http(port, "PUT", "/api/settings/providers", &hdr, br#"{"agent_models":{"bogus":{"provider":"openai","model":"gpt-4.1"}}}"#).0,
        400
    );
    assert_eq!(
        raw_http(port, "PUT", "/api/settings/providers", &hdr, br#"{"agent_models":{"explain":{"provider":"claude-cli","model":"sonnet"}}}"#).0,
        400
    );

    // 2. Configure providers + per-agent models.
    let put_cfg = serde_json::to_vec(&json!({
        "keys": {
            "anthropic": "sk-ant-role-test",
            "openai": "sk-oai-role-test"
        },
        "base_urls": {
            "anthropic": mock_ant.base_url,
            "openai": mock_oai.base_url
        },
        "agent_models": {
            "orchestrator": { "provider": "anthropic", "model": "claude-haiku-4-5" },
            "narrator": { "provider": "openai", "model": "gpt-4.1-mini" },
            "explain": { "provider": "openai", "model": "gpt-4.1-nano" }
        }
    }))
    .unwrap();
    let (st_put, _, put_b) = raw_http(port, "PUT", "/api/settings/providers", &hdr, &put_cfg);
    assert_eq!(st_put, 200, "{}", String::from_utf8_lossy(&put_b));
    let put_v: Value = serde_json::from_slice(&put_b).unwrap();
    assert_eq!(put_v["agent_models"]["narrator"]["model"], "gpt-4.1-mini");
    assert_eq!(put_v["agent_models"]["explain"]["model"], "gpt-4.1-nano");

    // 3. Narrate on empty session returns 400 and does NOT brick Turn 1.
    let (_, _, cr) = raw_http(port, "POST", "/api/sessions", &hdr, br#"{"title":"Explain <Test>"}"#);
    let sid = serde_json::from_slice::<Value>(&cr).unwrap()["id"].as_str().unwrap().to_string();

    let (st_nar0, _, nar0_b) = raw_http(port, "POST", &format!("/api/sessions/{sid}/narrate"), &hdr, b"{}");
    assert_eq!(st_nar0, 400, "{}", String::from_utf8_lossy(&nar0_b));
    assert!(String::from_utf8_lossy(&nar0_b).contains("Run at least one derivation turn"));

    // 4. Turn 1 succeeds (session was not bricked!) in Direct mode without Proof DAG entries.
    let msg = serde_json::to_vec(&json!({ "prompt": "Derive E", "verification_mode": false })).unwrap();
    assert_eq!(raw_http(port, "POST", &format!("/api/sessions/{sid}/messages"), &hdr, &msg).0, 202);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let (_, _, sb) = raw_http(port, "GET", &format!("/api/sessions/{sid}"), &hdr, b"");
        let sv: Value = serde_json::from_slice(&sb).unwrap();
        if sv["running"] == json!(false) && sv["messages"].as_array().unwrap().len() >= 2 {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "Turn 1 did not finish");
        std::thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(mock_ant.captured()[0].1["model"], "claude-haiku-4-5");

    // 5. Proof Narrator uses assigned `narrator` model (gpt-4.1-mini on OpenAI mock), sees Direct derivation fallback, and persists across reload!
    let (st_nar, _, nar_b) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{sid}/narrate"),
        &hdr,
        br#"{"provider":"anthropic","model":"claude-haiku-4-5"}"#,
    );
    assert_eq!(st_nar, 200, "{}", String::from_utf8_lossy(&nar_b));
    let nar_v: Value = serde_json::from_slice(&nar_b).unwrap();
    assert!(nar_v["narrative"].as_str().unwrap().contains("harmonic oscillator energy"));

    let (_, _, sess_after_nar) = raw_http(port, "GET", &format!("/api/sessions/{sid}"), &hdr, b"");
    let sess_v: Value = serde_json::from_slice(&sess_after_nar).unwrap();
    assert!(
        sess_v["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["narrator"] == json!(true) && m["text"].as_str().unwrap().contains("harmonic oscillator energy")),
        "narrator.report should persist in session messages: {sess_v}"
    );

    // 6. Explain (`sidetalk`) uses assigned `explain` model (`gpt-4.1-nano`) and exports cleanly to `.md` & `.html`!
    let (st_ex, _, ex_b) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{sid}/sidetalk"),
        &hdr,
        br#"{"question":"Why $\\hbar \\omega$? <script>alert(1)</script>","provider":"anthropic","model":"claude-haiku-4-5"}"#,
    );
    assert_eq!(st_ex, 200, "{}", String::from_utf8_lossy(&ex_b));

    let oai_reqs = mock_oai.captured();
    assert_eq!(oai_reqs.len(), 2);
    assert_eq!(oai_reqs[0].1["model"], "gpt-4.1-mini");
    assert!(oai_reqs[0].1["messages"][1]["content"].as_str().unwrap().contains("session_derivations"));
    assert_eq!(oai_reqs[1].1["model"], "gpt-4.1-nano");

    // Cookie-authenticated Explain export `.md` & `.html`
    let (st_md, hdr_md, md_bytes) = raw_http(
        port,
        "GET",
        &format!("/api/sessions/{sid}/export?format=explain_md"),
        &[("Cookie", cookie.as_str())],
        b"",
    );
    assert_eq!(st_md, 200);
    assert!(hdr_md.iter().any(|(k, v)| k == "content-type" && v.contains("text/markdown")));
    let md_str = String::from_utf8(md_bytes).unwrap();
    assert!(md_str.contains("# Explain Conversation: Explain <Test>"));
    assert!(md_str.contains("Why $\\hbar \\omega$?"));
    assert!(md_str.contains("`e-1`"));

    let (st_ht, hdr_ht, ht_bytes) = raw_http(
        port,
        "GET",
        &format!("/api/sessions/{sid}/export?format=explain_html"),
        &[("Cookie", cookie.as_str())],
        b"",
    );
    assert_eq!(st_ht, 200);
    assert!(hdr_ht.iter().any(|(k, v)| k == "content-type" && v.contains("text/html")));
    let ht_str = String::from_utf8(ht_bytes).unwrap();
    assert!(!ht_str.contains("<script>"));
    assert!(ht_str.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(ht_str.contains("&lt;script&gt;safe&lt;/script&gt;"));

    srv.shutdown();
}

#[test]
fn explicit_claim_reviews_stay_with_the_original_answer_and_step() {
    let (entered_tx, entered) = std::sync::mpsc::channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let release_rx = Mutex::new(release_rx);
    let mock = ScriptedMock::start(Box::new(move |_, req| {
        let input = req.to_string();
        if input.contains("<selected_claim>x is positive [e-2].</selected_claim>") {
            entered_tx.send(()).unwrap();
            release_rx.lock().unwrap().recv_timeout(Duration::from_secs(8)).unwrap();
        }
        let text = if input.contains("<selected_claim>no verdict</selected_claim>") {
            "I cannot provide the requested result."
        } else {
            r#"{"verdict":"accepted","summary":"Follows under the stated assumption.","counterexample":null,"issues":[]}"#
        };
        openai_text("gpt-4.1-mini", text)
    }));
    let temp = TempDir::new().unwrap();
    let server = start_desktop_server(test_cfg(temp.path(), None, None, None)).unwrap();
    let auth = [("X-ReSearch-Token", server.info.token.as_str())];
    let call = |method: &str, path: &str, data: Value| {
        let (code, _, bytes) = raw_http(server.addr.port(), method, path, &auth, &serde_json::to_vec(&data).unwrap());
        (code, serde_json::from_slice::<Value>(&bytes).unwrap())
    };
    let (_, session) = call("POST", "/api/sessions", json!({"title":"Two turns"}));
    let (status, agents) = call("GET", "/api/subagents", Value::Null);
    assert_eq!(status, 200);
    let step_role = agents["builtin"].as_array().unwrap().iter().find(|a| a["id"] == "step-verifier").unwrap();
    assert_eq!(step_role["role_key"], "step_verifier");
    let sid = session["id"].as_str().unwrap();
    let route = format!("/api/sessions/{sid}/claim-review");
    let ledger = Ledger::open(temp.path()).unwrap();
    let rec = |kind: &str, data: Value| ledger.append_with_hash(sid, kind, 1, &data).unwrap().seq;
    rec("session.start", json!({"prompt":"FIRST_PROBLEM","pinned_conventions":"x>0"}));
    rec("proof.entry", json!({"entry_id":"e-1","entry_type":"assumption","summary":"x>0","depends_on":[]}));
    rec("tool.request", json!({"call_id":"probe","tool":"sympy_eval","args":{"inline":"{\"code\":\"factorial(3)\"}"}}));
    rec("tool.result", json!({"call_id":"probe","ok":true,"result":{"inline":"{\"value\":\"RECORDED_EXACT_VALUE\"}"}}));
    rec("proof.entry", json!({"entry_id":"e-2","entry_type":"symbolic_computation","summary":"Calculation",
        "tool_call_ref":"probe","depends_on":["e-1"]}));
    rec("proof.entry", json!({"entry_id":"e-3","entry_type":"derivation","summary":"UNRELATED_ENTRY","depends_on":[]}));
    rec("model.response", json!({"stop_reason":"end_turn"}));
    let seq = rec("model.text", json!({"text":"The result:"}));
    let repeated = format!("FIRST_OCCURRENCE x is positive [e-2]. {} SECOND_OCCURRENCE x is positive [e-2]. no verdict", "λ".repeat(1600));
    rec("model.text", json!({"text":repeated}));
    rec("session.end", json!({"outcome":"completed"}));
    rec("session.resume", json!({"prompt":"LATER_UNRELATED_PROBLEM","pinned_conventions":"x<0"}));
    rec("model.text", json!({"text":"x is negative."}));
    rec("session.end", json!({"outcome":"completed"}));
    assert_eq!(call("POST", &route, json!({"claim":"x is negative.","answer_seq":seq})).0,400);
    assert_eq!(call("POST", &route, json!({"claim":"x is positive.","answer_seq":seq,"entry_id":"e-1"})).0,400);
    assert_eq!(call("POST", &route, json!({"entry_id":"e-missing"})).0,404);
    assert_eq!(call("POST", &route, json!({"claim":"x is positive [e-2].","answer_seq":seq,"occurrence":2})).0,400);
    assert!(mock.captured().is_empty());
    assert_eq!(call("PUT", "/api/settings/providers", json!({
        "keys":{"openai":"sk-claim-test"},"base_urls":{"openai":mock.base_url},
        "agent_models":{"step_verifier":{"provider":"openai","model":"gpt-4.1-mini"}}
    })).0,200);
    let (code, claim) = std::thread::scope(|scope| {
        let pending = scope.spawn(|| call("POST", &route, json!({"claim":"x is positive [e-2].","answer_seq":seq,"cited_steps":["e-2"],"occurrence":1})));
        entered.recv_timeout(Duration::from_secs(8)).unwrap();
        assert_eq!(call("POST", &route, json!({"entry_id":"e-1"})).0,409);
        assert_eq!(call("POST", &format!("/api/sessions/{sid}/messages"), json!({"prompt":"Busy!"})).0,409);
        release.send(()).unwrap();
        pending.join().unwrap()
    });
    assert_eq!(code, 200, "{claim}");
    assert_eq!(claim["scope"], "claim");
    assert_eq!(claim["verdict"], "accepted");
    let request = mock.captured()[0].1.to_string();
    assert!(request.contains("RECORDED_EXACT_VALUE") && request.contains("factorial(3)"));
    assert!(request.contains("SECOND_OCCURRENCE") && !request.contains("FIRST_OCCURRENCE"));
    assert!(!request.contains("UNRELATED_ENTRY"));
    assert_eq!(claim["cited_steps"], json!(["e-2"]));
    rec("proof.verification", json!({"kind":"step","target_entry_id":"e-2","verdict":"rejected"}));
    let (code, retry) = call("POST", &route, json!({"entry_id":"e-2"}));
    assert_eq!(code, 200, "{retry}");
    let retry_request = mock.captured().last().unwrap().1.to_string();
    assert!(retry_request.contains("RECORDED_EXACT_VALUE") && retry_request.contains("factorial(3)"));
    assert!(!retry_request.contains("UNRELATED_ENTRY"));
    let (code, step) = call("POST", &route, json!({"entry_id":"e-1"}));
    assert_eq!(code, 200, "{step}");
    assert_eq!(step["scope"], "step");
    assert_eq!(step["verdict"], "accepted");
    for (_, request) in mock.captured() {
        let message = request.to_string();
        assert!(message.contains("FIRST_PROBLEM"));
        assert!(!message.contains("LATER_UNRELATED_PROBLEM"));
    }
    assert_eq!(call("POST", &route, json!({"claim":"no verdict","answer_seq":seq})).0,400);
    let reopened = call("GET", &format!("/api/sessions/{sid}"), Value::Null).1;
    assert!(reopened["messages"].as_array().unwrap().iter().all(|m| m["verified_status"] != "verified"));
    let events = reopened["events"].as_array().unwrap();
    let selections = events.iter().filter(|e| e["kind"] == "claim.review").collect::<Vec<_>>();
    assert_eq!(selections.len(), 1);
    assert_eq!(selections[0]["payload"]["answer_seq"], seq);
    assert_eq!(selections[0]["payload"]["claim"], "x is positive [e-2].");
    assert_eq!(selections[0]["payload"]["occurrence"], 1);
    assert_eq!(selections[0]["payload"]["answer_sha256"], ledger::sha256_hex(format!("The result:\n{repeated}").as_bytes()));
    rec("session.resume", json!({"prompt":"Is the latest answer justified?"}));
    rec("proof.entry", json!({"entry_id":"e-4","entry_type":"final_answer","summary":"Bound","depends_on":[]}));
    rec("model.text", json!({"text":"The final bound."}));
    rec("proof.verification", json!({"kind":"full","target_entry_id":"e-4","verdict":"rejected","summary":"Full review found a gap.",
        "answer_sha256":ledger::sha256_hex(b"The final bound.")}));
    rec("session.end", json!({"outcome":"completed_unverified"}));
    let (code, scoped) = call("POST", &route, json!({"entry_id":"e-4"}));
    assert_eq!(code, 200, "{scoped}");
    assert_eq!(scoped["verdict"], "accepted");
    let reopened = call("GET", &format!("/api/sessions/{sid}"), Value::Null).1;
    assert_eq!(reopened["messages"].as_array().unwrap().last().unwrap()["verified_status"], "unverified");
    assert_eq!(reopened["stats"]["step_checks"], 1);
    for format in ["html","tex"] {
        let (code, _, bytes) = raw_http(server.addr.port(), "GET", &format!("/api/sessions/{sid}/export?format={format}"), &auth, b"");
        assert_eq!(code, 200);
        let report = String::from_utf8(bytes).unwrap();
        assert!(report.contains("Selected claim from answer event") && report.contains("Proof step e-1"));
        assert!(report.contains("occurrence 2"));
        assert!(report.contains("Full-solution review targeting e-4") && report.contains("Full review found a gap."));
        assert!(report.contains("Proof step e-4"));
        assert!(report.contains("Follows under the stated assumption."));
    }
    assert!(ledger.verify_session(sid).is_ok());
    server.shutdown();
}

#[test]
fn prestart_upload_does_not_brick_turn1_and_turn2_verifier_sees_new_question() {
    clear_provider_env();
    let mock = ScriptedMock::start(Box::new(|_path, body| {
        let sys = body["system"].as_str().unwrap_or("");
        if sys.contains("full_verification") {
            return anthropic_text(
                "claude-haiku-4-5",
                r#"{"verdict":"accepted","summary":"Verified latest question","counterexample":null,"issues":[]}"#,
            );
        }
        anthropic_text("claude-haiku-4-5", "Candidate mathematical solution.")
    }));

    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, Some(mock.base_url.clone()), None)).unwrap();
    let port = srv.addr.port();
    let tok = srv.info.token.clone();
    let hdr = [("X-ReSearch-Token", tok.as_str()), ("Content-Type", "application/json")];

    raw_http(
        port,
        "PUT",
        "/api/settings/providers",
        &hdr,
        br#"{"keys":{"anthropic":"sk-ant-multiturn"}}"#,
    );
    let (_, _, cr) = raw_http(port, "POST", "/api/sessions", &hdr, br#"{"title":"MultiTurn"}"#);
    let sid = serde_json::from_slice::<Value>(&cr).unwrap()["id"].as_str().unwrap().to_string();

    // 1. Upload file BEFORE Turn 1 (`source.ingested` is seq 1 in ledger!).
    let b64 = "XHNlY3Rpb257UHJlfQpIZWxsbwo=";
    let (st_up, _, _) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{sid}/upload"),
        &hdr,
        serde_json::to_vec(&json!({ "files": [{ "name": "pre.tex", "bytes_base64": b64 }] }))
            .unwrap()
            .as_slice(),
    );
    assert_eq!(st_up, 200);

    // 2. Turn 1 in Verified mode succeeds cleanly.
    let wait_turn = |prompt: &str, expected_msgs: usize| {
        let body = serde_json::to_vec(&json!({
            "prompt": prompt,
            "provider": "anthropic",
            "model": "claude-haiku-4-5",
            "verification_mode": true
        }))
        .unwrap();
        let (st, _, b) = raw_http(port, "POST", &format!("/api/sessions/{sid}/messages"), &hdr, &body);
        assert_eq!(st, 202, "{}", String::from_utf8_lossy(&b));
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let (_, _, sb) = raw_http(port, "GET", &format!("/api/sessions/{sid}"), &hdr, b"");
            let sv: Value = serde_json::from_slice(&sb).unwrap();
            if sv["running"] == json!(false) && sv["messages"].as_array().unwrap().len() >= expected_msgs {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "timed out waiting for `{prompt}`");
            std::thread::sleep(Duration::from_millis(25));
        }
    };

    wait_turn("TURN1_LEMMA_ALPHA", 2);
    wait_turn("TURN2_INTEGRAL_BETA", 4);

    // Inspect Verifier calls: 2nd verification must show TURN2_INTEGRAL_BETA as Latest user request!
    let verifier_reqs: Vec<String> = mock
        .captured()
        .into_iter()
        .filter(|(_, b)| b["system"].as_str().unwrap_or("").contains("full_verification"))
        .map(|(_, b)| {
            b["messages"][0]["content"][0]["text"]
                .as_str()
                .or_else(|| b["messages"][0]["content"].as_str())
                .unwrap_or("")
                .to_string()
        })
        .collect();
    assert_eq!(verifier_reqs.len(), 2);
    assert!(verifier_reqs[0].contains("TURN1_LEMMA_ALPHA"));
    assert!(verifier_reqs[1].contains("Latest user request to verify:\nTURN2_INTEGRAL_BETA"));
    assert!(verifier_reqs[1].contains("TURN1_LEMMA_ALPHA"));

    let ledger = Ledger::open_existing(&data_dir).unwrap();
    assert!(ledger.verify_all().is_ok());
    srv.shutdown();
}
