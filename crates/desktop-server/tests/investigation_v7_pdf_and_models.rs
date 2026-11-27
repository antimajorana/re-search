//! V7 integration tests for PDF-1 quote page + OCR metadata on `authors_claim.quotes`,
//! OpenAI-only model resolution without `default_provider`, CLI-route task rejection, and
//! `attributed_to` re-run diff tracking.

mod common;

use common::{raw_http, test_cfg};
use desktop_server::investigation_skeleton::EVENT_INVESTIGATION_SKELETON;
use desktop_server::{start_desktop_server, RunningDesktopServer};
use ledger::{sha256_hex, Ledger};
use research_tools::pdf_pages::{PageBasis, PageQualityMetrics, PageRecord};
use research_tools::ResearchEnv;
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;

type Handler = Box<dyn Fn(&str, &Value) -> Value + Send + Sync>;

struct LoopbackApi {
    base_url: String,
    seen: Arc<Mutex<Vec<(String, Value)>>>,
}

impl LoopbackApi {
    fn start(handler: Handler) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (seen_cl, handler) = (Arc::clone(&seen), Arc::new(handler));
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (h, s_inner) = (Arc::clone(&handler), Arc::clone(&seen_cl));
                std::thread::spawn(move || {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                    let mut reader = BufReader::new(stream);
                    let mut req_line = String::new();
                    if reader.read_line(&mut req_line).unwrap_or(0) == 0 {
                        return;
                    }
                    let path = req_line
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or("")
                        .split('?')
                        .next()
                        .unwrap_or("")
                        .to_string();
                    let mut len = 0usize;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                            break;
                        }
                        if let Some((k, v)) = line.split_once(':') {
                            if k.eq_ignore_ascii_case("content-length") {
                                len = v.trim().parse().unwrap_or(0);
                            }
                        }
                    }
                    let mut buf = vec![0u8; len];
                    let _ = reader.read_exact(&mut buf);
                    let body: Value = serde_json::from_slice(&buf).unwrap_or(Value::Null);
                    s_inner.lock().unwrap().push((path.clone(), body.clone()));
                    let bytes = serde_json::to_vec(&h(&path, &body)).unwrap();
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
        Self { base_url: format!("http://{addr}"), seen }
    }

    fn captured(&self) -> Vec<(String, Value)> {
        self.seen.lock().unwrap().clone()
    }
}

fn req(srv: &RunningDesktopServer, method: &str, path: &str, body: Option<&Value>) -> (u16, Value, String) {
    let hdr = [("X-ReSearch-Token", srv.info.token.as_str()), ("Content-Type", "application/json")];
    let bytes = body.map(|v| serde_json::to_vec(v).unwrap()).unwrap_or_default();
    let (st, _, raw) = raw_http(srv.addr.port(), method, path, &hdr, &bytes);
    (st, serde_json::from_slice(&raw).unwrap_or(Value::Null), String::from_utf8_lossy(&raw).into_owned())
}

fn wait_idle(srv: &RunningDesktopServer, sid: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let (st, sv, raw) = req(srv, "GET", &format!("/api/sessions/{sid}"), None);
        assert_eq!(st, 200, "GET session failed: {raw}");
        if sv["running"] == json!(false) {
            return sv;
        }
        assert!(Instant::now() < deadline, "timed out waiting for idle: {sv}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn ant_tool(id: &str, name: &str, input: Value) -> Value {
    json!({
        "id": "msg_tool", "type": "message", "role": "assistant", "model": "claude-sonnet-4-6",
        "stop_reason": "tool_use", "content": [{ "type": "tool_use", "id": id, "name": name, "input": input }],
        "usage": { "input_tokens": 10, "output_tokens": 20 }
    })
}

fn ant_text(text: &str) -> Value {
    json!({
        "id": "msg_done", "type": "message", "role": "assistant", "model": "claude-sonnet-4-6",
        "stop_reason": "end_turn", "content": [{ "type": "text", "text": text }],
        "usage": { "input_tokens": 10, "output_tokens": 12 }
    })
}

fn ant_has_tool_result(body: &Value) -> bool {
    body["messages"].as_array().into_iter().flatten()
        .flat_map(|m| m["content"].as_array().into_iter().flatten())
        .any(|b| b["type"] == "tool_result")
}

fn openai_tool(id: &str, name: &str, args: &Value) -> Value {
    json!({
        "id": "chatcmpl_tool", "model": "gpt-5.4",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant", "content": Value::Null,
                "tool_calls": [{
                    "id": id, "type": "function",
                    "function": { "name": name, "arguments": serde_json::to_string(args).unwrap() }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": { "prompt_tokens": 12, "completion_tokens": 18, "total_tokens": 30 }
    })
}

fn openai_text(text: &str) -> Value {
    json!({
        "id": "chatcmpl_done", "model": "gpt-5.4",
        "choices": [{ "index": 0, "message": { "role": "assistant", "content": text }, "finish_reason": "stop" }],
        "usage": { "prompt_tokens": 12, "completion_tokens": 10, "total_tokens": 22 }
    })
}

fn extract_request_seq(body: &Value) -> u64 {
    let s = body.to_string();
    let idx = s.rfind("request_seq=").expect("request_seq in prompt");
    s[idx + "request_seq=".len()..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap()
}

#[test]
fn pdf1_v7_mapview_attaches_physical_page_and_ocr_to_authors_claim_quotes() {
    let reading_text = concat!(
        "1. Introduction\n",
        "Theorem 1. We prove a spectral gap bound.\n",
        "\x0c\n",
        "Lemma 2. By scanning old notes, we show positivity.\n",
        "\x0c\n",
        "[scan page 3]\n"
    );
    let q1_line = "Theorem 1. We prove a spectral gap bound.";
    let q2_line = "Lemma 2. By scanning old notes, we show positivity.";
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let node_ref_cell = Arc::new(Mutex::new((String::new(), String::new())));
    let node_ref = Arc::clone(&node_ref_cell);
    let srv_mock = LoopbackApi::start(Box::new(move |_, body| {
        if ant_has_tool_result(body) {
            return ant_text("Orient complete.");
        }
        let (thm_tag, lem_tag) = node_ref.lock().unwrap().clone();
        let req_seq = extract_request_seq(body);
        ant_tool("c1", "record_map_reading", json!({
            "request_seq": req_seq,
            "source_id": "src-pdf-v7",
            "items": [
                {
                    "tag": thm_tag,
                    "role": "main_result",
                    "gist": "Main spectral gap claim.",
                    "anchors": [
                        { "source_id": "src-pdf-v7", "path": "src-pdf-v7/paper.txt", "start_line": 2, "end_line": 2, "quote": q1_line }
                    ],
                    "authors_claim": {
                        "label": "new",
                        "relation_note": "Authors explicitly say they prove this bound.",
                        "anchors": [
                            { "source_id": "src-pdf-v7", "path": "src-pdf-v7/paper.txt", "start_line": 2, "end_line": 2, "quote": q1_line }
                        ]
                    }
                },
                {
                    "tag": lem_tag,
                    "role": "supporting",
                    "gist": "Scanned OCR positivity lemma.",
                    "anchors": [
                        { "source_id": "src-pdf-v7", "path": "src-pdf-v7/paper.txt", "start_line": 4, "end_line": 4, "quote": q2_line }
                    ],
                    "authors_claim": {
                        "label": "new",
                        "relation_note": "Authors say they show positivity in scan text.",
                        "anchors": [
                            { "source_id": "src-pdf-v7", "path": "src-pdf-v7/paper.txt", "start_line": 4, "end_line": 4, "quote": q2_line }
                        ]
                    }
                }
            ],
            "edges": [],
            "limitations": [],
            "done": true
        }))
    }));
    let srv = start_desktop_server(test_cfg(&data_dir, None, Some(srv_mock.base_url.clone()), None)).unwrap();
    let (st, _, _) = req(&srv, "PUT", "/api/settings/providers", Some(&json!({
        "keys": { "anthropic": "sk-ant-offline" },
        "default_provider": "anthropic",
        "default_model": "claude-sonnet-4-6"
    })));
    assert_eq!(st, 200);

    let sid = "s-pdf-v7";
    assert_eq!(req(&srv, "POST", "/api/sessions", Some(&json!({ "id": sid, "mode": "investigation" }))).0, 200);
    let rel_path = "src-pdf-v7/paper.txt";
    let abs_file = data_dir.join("sessions").join(sid).join("sources").join(rel_path);
    fs::create_dir_all(abs_file.parent().unwrap()).unwrap();
    fs::write(&abs_file, reading_text.as_bytes()).unwrap();
    let sha = sha256_hex(reading_text.as_bytes());
    let metrics = PageQualityMetrics {
        char_count: 40, alpha_count: 30, word_count: 6, image_coverage_pct: 0,
        has_invisible_text: false, width_pt: 612, height_pt: 792,
    };
    let pages = vec![
        PageRecord { page: 1, basis: PageBasis::TextLayer, first_line: Some(1), last_line: Some(2), metrics: metrics.clone(), reason: None },
        PageRecord { page: 2, basis: PageBasis::ScanTextLayer, first_line: Some(4), last_line: Some(4), metrics: metrics.clone(), reason: Some("ocr scan".into()) },
        PageRecord { page: 3, basis: PageBasis::None, first_line: None, last_line: None, metrics, reason: Some("image only".into()) },
    ];
    let ledger = Ledger::open(&data_dir).unwrap();
    ledger.append(sid, "source.ingested", 1, &json!({
        "source_id": "src-pdf-v7",
        "title": "Scanned Gap Paper",
        "origin": "paper.pdf",
        "format": "pdf",
        "status": "ok",
        "primary_path": rel_path,
        "files": [rel_path],
        "files_sha256": { rel_path: sha },
        "page_count": 3,
        "pdf_extractor": "pdf-extract-layout/0.12.1",
        "pdf_pages": pages,
    })).unwrap();
    drop(ledger);

    let (st_open, _, raw_open) = req(&srv, "POST", &format!("/api/sessions/{sid}/investigation/open"), Some(&json!({ "source_id": "src-pdf-v7" })));
    assert_eq!(st_open, 200, "{raw_open}");
    let (st_mv0, mv0, raw_mv0) = req(&srv, "GET", &format!("/api/sessions/{sid}/investigation/mapview"), None);
    assert_eq!(st_mv0, 200, "{raw_mv0}");
    assert_eq!(mv0["builder"], "pdf-1", "{mv0}");
    let nodes0 = mv0["nodes"].as_array().unwrap();
    let thm0 = nodes0.iter().find(|n| n["kind"] == "theorem").expect("theorem node in pdf-1 mapview");
    let lem0 = nodes0.iter().find(|n| n["kind"] == "lemma").expect("lemma node in pdf-1 mapview");
    *node_ref_cell.lock().unwrap() = (
        thm0["id"].as_str().unwrap().to_string(),
        lem0["id"].as_str().unwrap().to_string(),
    );

    let (st_t, _, raw_t) = req(&srv, "POST", &format!("/api/sessions/{sid}/investigation/tasks"), Some(&json!({ "task": "orient" })));
    assert_eq!(st_t, 202, "{raw_t}");
    wait_idle(&srv, sid);

    let (st_mv, mv, raw_mv) = req(&srv, "GET", &format!("/api/sessions/{sid}/investigation/mapview"), None);
    assert_eq!(st_mv, 200, "{raw_mv}");
    let nodes = mv["nodes"].as_array().unwrap();
    let thm = nodes.iter().find(|n| n["kind"] == "theorem").unwrap();
    let lem = nodes.iter().find(|n| n["kind"] == "lemma").unwrap();
    let q_thm = thm["authors_claim"]["quotes"].as_array().unwrap();
    let q_lem = lem["authors_claim"]["quotes"].as_array().unwrap();
    assert_eq!(q_thm.len(), 1, "{thm}");
    assert_eq!(q_thm[0]["page"], json!(1), "{q_thm:?}");
    assert!(q_thm[0].get("ocr").is_none(), "{q_thm:?}");
    assert_eq!(q_lem.len(), 1, "{lem}");
    assert_eq!(q_lem[0]["page"], json!(2), "{q_lem:?}");
    assert_eq!(q_lem[0]["ocr"], json!(true), "{q_lem:?}");
    assert_eq!(q_lem[0]["page_basis"], json!("scan_text_layer"), "{q_lem:?}");
    let (st_js, _, raw_js) = req(&srv, "GET", "/assets/investigation_orient_ui.js", None);
    assert_eq!(st_js, 200);
    assert!(raw_js.contains("mountOrientControls"), "{raw_js}");
    srv.shutdown();
}

#[test]
fn openai_only_orient_works_without_default_provider_and_rejects_cli_route() {
    let call_count = Arc::new(Mutex::new(0usize));
    let cc = Arc::clone(&call_count);
    let src_id_cell = Arc::new(Mutex::new(String::new()));
    let src_path_cell = Arc::new(Mutex::new(String::new()));
    let node_ref_cell = Arc::new(Mutex::new((String::new(), String::new())));
    let (sid_ref, path_ref, node_ref) = (
        Arc::clone(&src_id_cell),
        Arc::clone(&src_path_cell),
        Arc::clone(&node_ref_cell),
    );
    let mock = LoopbackApi::start(Box::new(move |path, body| {
        assert!(path.contains("chat/completions"), "unexpected path: {path}");
        let is_tool_reply = body["messages"]
            .as_array()
            .and_then(|a| a.last())
            .is_some_and(|m| m["role"] == "tool");
        if is_tool_reply {
            return openai_text("Orient finished.");
        }
        let mut guard = cc.lock().unwrap();
        *guard += 1;
        let run_idx = *guard;
        let (tag, _) = node_ref.lock().unwrap().clone();
        let attr = if run_idx == 1 { vec!["Doe98"] } else { vec!["Smith01"] };
        let src_id = sid_ref.lock().unwrap().clone();
        let src_path = path_ref.lock().unwrap().clone();
        let req_seq = extract_request_seq(body);
        openai_tool("call_orient", "record_map_reading", &json!({
            "request_seq": req_seq,
            "source_id": src_id,
            "items": [{
                "tag": tag,
                "role": "supporting",
                "gist": "Recalled decay lemma.",
                "anchors": [{
                    "source_id": src_id,
                    "path": src_path,
                    "start_line": 3,
                    "end_line": 3,
                    "quote": "This bound is known from earlier work."
                }],
                "authors_claim": {
                    "label": "restated",
                    "relation_note": "Recalls earlier decay bound.",
                    "attributed_to": attr,
                    "anchors": [{
                        "source_id": src_id,
                        "path": src_path,
                        "start_line": 3,
                        "end_line": 3,
                        "quote": "This bound is known from earlier work."
                    }]
                }
            }],
            "edges": [],
            "limitations": [],
            "done": true
        }))
    }));
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, Some(mock.base_url.clone()), None)).unwrap();
    // Connect ONLY OpenAI without setting `default_provider`.
    let (st_set, _, raw_set) = req(&srv, "PUT", "/api/settings/providers", Some(&json!({ "keys": { "openai": "sk-oai-offline" } })));
    assert_eq!(st_set, 200, "{raw_set}");

    let sid = "s-oai-orient";
    assert_eq!(req(&srv, "POST", "/api/sessions", Some(&json!({ "id": sid, "mode": "investigation" }))).0, 200);
    let tex = concat!(
        "\\section{Setup}\n",
        "\\begin{theorem}\n",
        "This bound is known from earlier work.\n",
        "\\end{theorem}\n",
        "\\begin{thebibliography}{9}\n",
        "\\bibitem{Doe98} J. Doe, 1998.\n",
        "\\bibitem{Smith01} A. Smith, 2001.\n",
        "\\end{thebibliography}\n",
    );
    let env = ResearchEnv::new(&data_dir);
    let ledger = Mutex::new(Ledger::open(&data_dir).unwrap());
    let src = research_tools::ingest_bytes(&env.sandbox(), &ledger, &research_tools::IngestRequest {
        session_id: sid, file_name: "main.tex", origin: "upload", raw_bytes: tex.as_bytes(),
        source_id_hint: Some("tex-oai"), section_filter: None, title: Some("OpenAI Orient Test"),
    }).unwrap();
    *src_id_cell.lock().unwrap() = src.source_id.clone();
    *src_path_cell.lock().unwrap() = src.primary_path.clone().expect("primary_path");
    drop(ledger);

    assert_eq!(req(&srv, "POST", &format!("/api/sessions/{sid}/investigation/open"), Some(&json!({ "source_id": src.source_id }))).0, 200);
    let (st_mv0, mv0, _) = req(&srv, "GET", &format!("/api/sessions/{sid}/investigation/mapview"), None);
    assert_eq!(st_mv0, 200);
    let thm0 = mv0["nodes"].as_array().unwrap().iter().find(|n| n["kind"] == "theorem").expect("theorem node in tex-1 mapview");
    *node_ref_cell.lock().unwrap() = (
        thm0["id"].as_str().unwrap().to_string(),
        thm0["fp"].as_str().unwrap().to_string(),
    );

    // Reject CLI route cleanly at validation time.
    let (st_cli, _, raw_cli) = req(&srv, "POST", &format!("/api/sessions/{sid}/investigation/tasks"), Some(&json!({ "task": "orient", "provider": "claude-cli" })));
    assert_eq!(st_cli, 400, "{raw_cli}");

    // Omit provider/model when only OpenAI is connected and default_provider is unset:
    // must fail cleanly with a "Choose a provider" error and NEVER fall back silently.
    let (st_missing, _, raw_missing) = req(&srv, "POST", &format!("/api/sessions/{sid}/investigation/tasks"), Some(&json!({ "task": "orient" })));
    assert_eq!(st_missing, 400, "{raw_missing}");
    assert!(raw_missing.contains("Choose a provider"), "expected clear choose-a-provider error, got: {raw_missing}");
    assert!(mock.captured().is_empty(), "OpenAI mock must NOT be called without an explicit provider choice");
    assert_eq!(
        Ledger::open(&data_dir).unwrap().read_session(sid).unwrap().iter().filter(|e| e.kind == "investigation.task_requested").count(),
        0,
        "refused task must not append investigation.task_requested"
    );

    // Explicitly choose OpenAI (as the UI toolbar / Orient controls do): succeeds cleanly.
    let (st_1, _, raw_1) = req(&srv, "POST", &format!("/api/sessions/{sid}/investigation/tasks"), Some(&json!({ "task": "orient", "provider": "openai" })));
    assert_eq!(st_1, 202, "{raw_1}");
    wait_idle(&srv, sid);
    assert!(!mock.captured().is_empty(), "OpenAI mock must have been called");

    // Re-run Orient changing ONLY `attributed_to` -> must flag `changed` & `changed_since_last`.
    let (st_2, _, raw_2) = req(&srv, "POST", &format!("/api/sessions/{sid}/investigation/tasks"), Some(&json!({ "task": "orient", "provider": "openai" })));
    assert_eq!(st_2, 202, "{raw_2}");
    wait_idle(&srv, sid);

    let (st_mv, mv, raw_mv) = req(&srv, "GET", &format!("/api/sessions/{sid}/investigation/mapview"), None);
    assert_eq!(st_mv, 200, "{raw_mv}");
    let thm = mv["nodes"].as_array().unwrap().iter().find(|n| n["authors_claim"].is_object()).unwrap_or_else(|| panic!("expected authors_claim in mv: {mv}"));
    assert_eq!(thm["authors_claim"]["changed"], json!(true), "{thm}");
    assert!(thm["changed_since_last"]["changes"].as_array().unwrap().iter().any(|c| c["field"] == "authors_claim"), "{thm}");
    assert_eq!(Ledger::open(&data_dir).unwrap().read_session(sid).unwrap().iter().filter(|e| e.kind == EVENT_INVESTIGATION_SKELETON).count(), 1);
    srv.shutdown();
}
