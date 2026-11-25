//! Failing audit proof tests (`audit-flow`) for PDF page-range scope clipping,
//! multi-target reader slice planning, `max_parallel` provider selection (Decision 5),
//! and auto-enable fallback gating (Decision 7) in `desktop-server`.

mod common;

use agent::ProviderKind;
use common::{raw_http, test_cfg};
use desktop_server::auth::AuthGuard;
use desktop_server::chatgpt::{refusing_opener, ChatGptHub, ChatGptTestHooks};
use desktop_server::paper_reader::prepare_for_turn;
use desktop_server::sse::SseHub;
use desktop_server::state::DesktopState;
use desktop_server::{start_desktop_server, RunningDesktopServer};
use ledger::Ledger;
use research_tools::{ingest_bytes, IngestRequest, ResearchEnv};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;

type Handler = Box<dyn Fn(&str, &Value) -> Value + Send + Sync>;

struct MockServer {
    base_url: String,
    seen: Arc<Mutex<Vec<(String, Value)>>>,
}

impl MockServer {
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
        Self {
            base_url: format!("http://{addr}"),
            seen,
        }
    }

    fn captured(&self) -> Vec<(String, Value)> {
        self.seen.lock().unwrap().clone()
    }
}

fn ant_text(model: &str, text: &str) -> Value {
    json!({
        "id": "msg_t", "type": "message", "role": "assistant", "model": model,
        "stop_reason": "end_turn", "content": [{ "type": "text", "text": text }],
        "usage": { "input_tokens": 12, "output_tokens": 18 }
    })
}

fn ant_tool(model: &str, id: &str, name: &str, input: Value) -> Value {
    json!({
        "id": "msg_u", "type": "message", "role": "assistant", "model": model,
        "stop_reason": "tool_use", "content": [{ "type": "tool_use", "id": id, "name": name, "input": input }],
        "usage": { "input_tokens": 12, "output_tokens": 18 }
    })
}

fn ant_tool_results(body: &Value) -> Vec<Value> {
    body["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|m| m["content"].as_array().into_iter().flatten())
        .filter(|b| b["type"] == "tool_result")
        .cloned()
        .collect()
}

fn has_tool(body: &Value, name: &str) -> bool {
    body["tools"]
        .as_array()
        .is_some_and(|a| a.iter().any(|t| t["name"] == name || t["function"]["name"] == name))
}

struct Client {
    port: u16,
    tok: String,
}

impl Client {
    fn new(srv: &RunningDesktopServer) -> Self {
        Self {
            port: srv.addr.port(),
            tok: srv.info.token.clone(),
        }
    }

    fn req(&self, method: &str, path: &str, body: Option<&Value>) -> (u16, Value, String) {
        let hdr = [
            ("X-ReSearch-Token", self.tok.as_str()),
            ("Content-Type", "application/json"),
        ];
        let bytes = body
            .map(|v| serde_json::to_vec(v).unwrap())
            .unwrap_or_default();
        let (st, _, raw) = raw_http(self.port, method, path, &hdr, &bytes);
        (
            st,
            serde_json::from_slice(&raw).unwrap_or(Value::Null),
            String::from_utf8_lossy(&raw).into_owned(),
        )
    }

    fn wait_idle(&self, sid: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let (st, sv, raw) = self.req("GET", &format!("/api/sessions/{sid}"), None);
            assert_eq!(st, 200, "GET session failed: {raw}");
            if sv["running"] == json!(false) {
                return sv;
            }
            assert!(Instant::now() < deadline, "timed out waiting for idle: {sv}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn boot_readers(mock_url: String) -> (TempDir, std::path::PathBuf, RunningDesktopServer, Client) {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, Some(mock_url), None)).unwrap();
    let client = Client::new(&srv);
    client.req(
        "PUT",
        "/api/settings/providers",
        Some(&json!({
            "keys": { "anthropic": "placeholder-offline-test-value" },
            "paper_readers_enabled": true
        })),
    );
    (tmp, data_dir, srv, client)
}

/// Builds a valid multi-page PDF where each page renders its slice of lines top-to-bottom.
fn build_multi_page_pdf(pages: &[&[&str]]) -> Vec<u8> {
    let n = pages.len();
    let first_stream_id = 4usize;
    let first_page_id = first_stream_id + n;
    let kids: String = (0..n)
        .map(|i| format!("{} 0 R", first_page_id + i))
        .collect::<Vec<_>>()
        .join(" ");

    let mut objs: Vec<(usize, Vec<u8>)> = vec![
        (1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()),
        (2, format!("<< /Type /Pages /Kids [{kids}] /Count {n} >>").into_bytes()),
        (3, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec()),
    ];
    for (i, lines) in pages.iter().enumerate() {
        let mut ops = String::from("BT /F1 12 Tf 72 720 Td\n");
        for (li, ln) in lines.iter().enumerate() {
            let esc = ln.replace('\\', "\\\\").replace('(', "\\(").replace(')', "\\)");
            if li > 0 {
                ops.push_str("0 -18 Td\n");
            }
            ops.push_str(&format!("({esc}) Tj\n"));
        }
        ops.push_str("ET");
        let stream_obj = format!("<< /Length {} >>\nstream\n{}\nendstream", ops.len(), ops);
        objs.push((first_stream_id + i, stream_obj.into_bytes()));
    }
    for i in 0..n {
        let (pid, sid) = (first_page_id + i, first_stream_id + i);
        objs.push((
            pid,
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents {sid} 0 R >>").into_bytes(),
        ));
    }

    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0usize; objs.len() + 1];
    for (id, body) in &objs {
        offsets[*id] = out.len();
        writeln!(out, "{id} 0 obj").unwrap();
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref_pos = out.len();
    let total_xref = objs.len() + 1;
    writeln!(out, "xref\n0 {total_xref}\n0000000000 65535 f ").unwrap();
    for off in offsets.iter().skip(1) {
        writeln!(out, "{off:010} 00000 n ").unwrap();
    }
    writeln!(out, "trailer\n<< /Size {total_xref} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF")
        .unwrap();
    out
}

#[test]
fn pdf_page_range_clips_overlapping_section_for_scope_and_reader_slices() {
    let mock = MockServer::start(Box::new(|_, body| {
        if has_tool(body, "note_evidence") {
            ant_text("claude-sonnet-4-6", "Reader done.")
        } else if ant_tool_results(body).is_empty() {
            ant_tool("claude-sonnet-4-6", "c_read", "delegate_read_paper", json!({ "focus": "Section II" }))
        } else {
            ant_text("claude-sonnet-4-6", "Done.")
        }
    }));
    let (_tmp, data_dir, _srv, client) = boot_readers(mock.base_url.clone());
    let sid = "inv-pdf-clip";
    client.req("POST", "/api/sessions", Some(&json!({ "id": sid, "mode": "investigation" })));

    let pdf_bytes = build_multi_page_pdf(&[
        &[
            "Chiral Defect Modes in Two-Page Selection",
            "Abstract",
            "We analyze defects restricted to pages one and two.",
            "I. INTRODUCTION",
            "Page one establishes the bulk lattice model.",
        ],
        &[
            "II. DEFECT SPECTRUM",
            "Page two begins Section II within the selected page range.",
            "Theorem 1 holds on physical page two.",
        ],
        &[
            "Page three continues Section II outside the selected PDF pages.",
            "Excluded page three corollary that must not be read or cited.",
            "III. CONCLUSIONS",
            "Summary remarks on excluded page three.",
        ],
    ]);

    let env = ResearchEnv::new(data_dir.clone());
    let db = Mutex::new(Ledger::open(&data_dir).unwrap());
    let src = ingest_bytes(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: sid,
            file_name: "paper.pdf",
            origin: "upload",
            raw_bytes: &pdf_bytes,
            source_id_hint: Some("pdf-clip"),
            section_filter: None,
            title: Some("Chiral Defect Modes"),
        },
    )
    .unwrap();
    drop(db);

    let ledger = Ledger::open(&data_dir).unwrap();
    let evs_before = ledger.read_session(sid).unwrap();
    let ing = evs_before.iter().find(|e| e.kind == "source.ingested").unwrap();
    let page_lines: Vec<(usize, usize, usize)> =
        serde_json::from_value(ing.payload["pdf_page_lines"].clone()).unwrap();
    let page_2_max_line = page_lines.iter().find(|&&(p, _, _)| p == 2).unwrap().2;

    let (st, _, raw) = client.req(
        "POST",
        &format!("/api/sessions/{sid}/messages"),
        Some(&json!({
            "prompt": "Help me understand section II",
            "investigation_source_id": src.source_id,
            "investigation_page_start": 1,
            "investigation_page_end": 2,
            "provider": "anthropic",
            "model": "claude-sonnet-4-6"
        })),
    );
    assert_eq!(st, 202, "{raw}");
    client.wait_idle(sid);

    let evs = ledger.read_session(sid).unwrap();
    let scope_ev = evs.iter().find(|e| e.kind == "investigation.scope").unwrap();
    let scope_end = scope_ev.payload["ranges"][0][1].as_u64().unwrap() as usize;
    let d_end = evs.iter().find(|e| e.kind == "delegate.end" && e.payload["worker"] == "paper_reader").unwrap();
    let slice_end = d_end.payload["readers"][0]["range"][1].as_u64().unwrap() as usize;

    assert!(
        scope_end <= page_2_max_line && slice_end <= page_2_max_line,
        "selected PDF pages 1..=2 end at line {page_2_max_line}, but Section II remained unclipped into excluded page 3 (scope_end={scope_end}, reader_slice_end={slice_end}): scope={}, readers={}",
        scope_ev.payload,
        d_end.payload["readers"]
    );
}

#[test]
fn multi_range_scope_does_not_span_entire_paper_between_targets() {
    let mock = MockServer::start(Box::new(|_, body| {
        if has_tool(body, "note_evidence") {
            ant_text("claude-sonnet-4-6", "Reader done.")
        } else if ant_tool_results(body).is_empty() {
            ant_tool("claude-sonnet-4-6", "c_read_m", "delegate_read_paper", json!({ "focus": "Section I and Appendix A" }))
        } else {
            ant_text("claude-sonnet-4-6", "Done.")
        }
    }));
    let (_tmp, data_dir, _srv, client) = boot_readers(mock.base_url.clone());
    let sid = "inv-multi-scope";
    client.req("POST", "/api/sessions", Some(&json!({ "id": sid, "mode": "investigation" })));

    let mut paper = String::from(
        "Title: Multi-Target Scope Paper\nAbstract\nShort abstract.\n\nI. INTRODUCTION\nShort intro line one.\nShort intro line two.\n\nII. BULK CALCULATIONS\n",
    );
    for i in 1..=260 {
        paper.push_str(&format!(
            "Unscoped middle bulk derivation line {i:03} that is neither Section I nor Appendix A.\n"
        ));
    }
    paper.push_str("\nAppendix A: Boundary Algebra\nAppendix A line one.\nAppendix A line two.\n");

    let env = ResearchEnv::new(data_dir.clone());
    let db = Mutex::new(Ledger::open(&data_dir).unwrap());
    let src = ingest_bytes(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: sid,
            file_name: "multi.txt",
            origin: "upload",
            raw_bytes: paper.as_bytes(),
            source_id_hint: Some("multi-1"),
            section_filter: None,
            title: Some("Multi-Target Scope Paper"),
        },
    )
    .unwrap();
    drop(db);

    let (st, _, raw) = client.req(
        "POST",
        &format!("/api/sessions/{sid}/messages"),
        Some(&json!({
            "prompt": "Map section I and Appendix A",
            "investigation_source_id": src.source_id,
            "provider": "anthropic",
            "model": "claude-sonnet-4-6"
        })),
    );
    assert_eq!(st, 202, "{raw}");
    client.wait_idle(sid);

    let ledger = Ledger::open(&data_dir).unwrap();
    let evs = ledger.read_session(sid).unwrap();
    let d_end = evs.iter().find(|e| e.kind == "delegate.end" && e.payload["worker"] == "paper_reader").unwrap();
    let total_planned_lines: u64 = d_end.payload["readers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["range"][1].as_u64().unwrap() - r["range"][0].as_u64().unwrap() + 1)
        .sum();
    assert!(
        total_planned_lines <= 30,
        "disjoint scope ranges (Section I + Appendix A, ~9 lines) collapsed to min..max and assigned {total_planned_lines} lines across the entire paper: {}",
        d_end.payload["readers"]
    );
}

#[test]
fn reader_max_parallel_uses_reader_provider_not_orchestrator_provider() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let mut cfg = test_cfg(&data_dir, None, Some("http://127.0.0.1:9".into()), None);
    cfg.chatgpt_test = Some(ChatGptTestHooks {
        mock_base: "http://127.0.0.1:9".into(),
        opener: refusing_opener(),
        login_timeout: Duration::from_secs(1),
    });
    let auth = AuthGuard::new(19999).unwrap();
    let sse = Arc::new(SseHub::new());
    let chatgpt = ChatGptHub::new(&cfg).unwrap();
    let state = Arc::new(DesktopState::new(cfg, auth, sse, chatgpt));

    let settings_body = serde_json::to_vec(&json!({
        "keys": { "anthropic": "placeholder-offline-test-value" },
        "paper_readers_enabled": true,
        "paper_reader_max_parallel": null,
        "agent_models": {
            "paper_reader": { "provider": "anthropic", "model": "claude-haiku-4-5" }
        }
    }))
    .unwrap();
    desktop_server::routes_session::handle_put_settings(&state, &settings_body).unwrap();

    let sid = "inv-max-par";
    let env = Arc::new(ResearchEnv::new(&data_dir));
    let db = Mutex::new(Ledger::open(&data_dir).unwrap());
    let big_text: String = (1..=2500)
        .map(|i| format!("Line {i:04}: Long physical derivation paragraph with boundary terms and topological invariants.\n"))
        .collect();
    let src = ingest_bytes(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: sid,
            file_name: "big.txt",
            origin: "upload",
            raw_bytes: big_text.as_bytes(),
            source_id_hint: Some("big-1"),
            section_filter: None,
            title: Some("Big Paper"),
        },
    )
    .unwrap();
    let ledger = db.into_inner().unwrap();
    ledger.append(sid, "investigation.started", 1, &json!({})).unwrap();
    ledger.append(sid, "investigation.paper", 1, &json!({"source_id": src.source_id})).unwrap();
    let marker_seq = ledger
        .append(sid, "investigation.mapping_requested", 1, &json!({"source_id": src.source_id}))
        .unwrap();

    let ui = state.load_ui_settings();
    let req = json!({ "_paper_map_request": marker_seq });
    let prepared = prepare_for_turn(&state, &env, &ui, &req, sid, ProviderKind::ChatGpt, "gpt-5.4")
        .unwrap()
        .expect("paper reader should be prepared");
    let pr_cfg = prepared.as_config();
    assert_eq!(pr_cfg.worker.model, "claude-haiku-4-5");
    assert_eq!(
        pr_cfg.max_parallel, 4,
        "Paper Reader runs on Anthropic API key (`claude-haiku-4-5`), yet `prepare_for_turn` computed `max_parallel` from Orchestrator's ChatGpt provider (got {})",
        pr_cfg.max_parallel
    );
}

#[test]
fn unconfigured_paper_reader_provider_does_not_auto_enable_primary_model_readers() {
    let mock = MockServer::start(Box::new(|_, _| ant_text("claude-sonnet-4-6", "Done.")));
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let srv = start_desktop_server(test_cfg(&data_dir, None, Some(mock.base_url.clone()), None)).unwrap();
    let client = Client::new(&srv);
    let (st, _, raw) = client.req(
        "PUT",
        "/api/settings/providers",
        Some(&json!({
            "keys": { "anthropic": "placeholder-offline-test-value" },
            "paper_readers_enabled": null,
            "agent_models": { "paper_reader": { "provider": "openai", "model": "gpt-4.1-mini" } }
        })),
    );
    assert_eq!(st, 200, "{raw}");

    let sid = "inv-unusable-reader";
    client.req("POST", "/api/sessions", Some(&json!({ "id": sid, "mode": "investigation" })));
    let env = ResearchEnv::new(data_dir.clone());
    let db = Mutex::new(Ledger::open(&data_dir).unwrap());
    let src = ingest_bytes(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: sid,
            file_name: "paper.txt",
            origin: "upload",
            raw_bytes: b"Title: Short Paper\nTheorem 1: Bulk-boundary correspondence holds.\n",
            source_id_hint: Some("short-1"),
            section_filter: None,
            title: Some("Short Paper"),
        },
    )
    .unwrap();
    drop(db);

    let (st_m, _, raw_m) = client.req(
        "POST",
        &format!("/api/sessions/{sid}/messages"),
        Some(&json!({
            "prompt": "Map the whole paper",
            "investigation_source_id": src.source_id,
            "provider": "anthropic",
            "model": "claude-sonnet-4-6"
        })),
    );
    assert_eq!(st_m, 202, "{raw_m}");
    client.wait_idle(sid);

    assert!(
        !has_tool(&mock.captured()[0].1, "delegate_read_paper"),
        "unusable `agent_models.paper_reader` (missing OpenAI key) auto-enabled parallel readers on the primary model (`isolated_primary_model`) with `paper_readers_enabled: null`, violating Decision 7"
    );
}
