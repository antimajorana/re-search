//! Consistency audit tests: section-scoped conventions (`ux-rust` D4 vs `split-1` WP1b vs
//! `split-3` WP11), PDF `selected_pages` + boundary section scope & reader slices, unstripped
//! `_paper_scope` injection, section-scoped `conventions_resumed` reader auto-enable, stale scope
//! chip after whole-paper expansion, and question-only Research handoff (Design 02 P1b).

mod common;

use common::{raw_http, test_cfg};
use desktop_server::paper_reader::plan_slices;
use desktop_server::{start_desktop_server, RunningDesktopServer};
use ledger::Ledger;
use research_tools::investigation::{record, record_conventions};
use research_tools::{ingest_bytes, IngestRequest, IngestedSource, ResearchEnv};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
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
        Self {
            base_url: format!("http://{addr}"),
            seen,
        }
    }

    fn captured(&self) -> Vec<(String, Value)> {
        self.seen.lock().unwrap().clone()
    }
}

fn ant_text(text: &str) -> Value {
    json!({
        "id": "msg_ok",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-6",
        "stop_reason": "end_turn",
        "content": [{ "type": "text", "text": text }],
        "usage": { "input_tokens": 15, "output_tokens": 10 }
    })
}

fn has_tool(body: &Value, name: &str) -> bool {
    body["tools"]
        .as_array()
        .is_some_and(|a| a.iter().any(|t| t["name"] == name || t["function"]["name"] == name))
}

fn req(srv: &RunningDesktopServer, method: &str, path: &str, body: Option<&Value>) -> (u16, Value) {
    let hdr = [
        ("X-ReSearch-Token", srv.info.token.as_str()),
        ("Content-Type", "application/json"),
    ];
    let bytes = body.map(|v| serde_json::to_vec(v).unwrap()).unwrap_or_default();
    let (st, _, raw) = raw_http(srv.addr.port(), method, path, &hdr, &bytes);
    (st, serde_json::from_slice(&raw).unwrap_or(Value::Null))
}

fn wait_idle(srv: &RunningDesktopServer, sid: &str, min_ends: usize) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let (st, sv) = req(srv, "GET", &format!("/api/sessions/{sid}"), None);
        assert_eq!(st, 200);
        let ends = sv["events"]
            .as_array()
            .map(|evs| {
                evs.iter()
                    .filter(|e| e["kind"] == "session.end" || e["kind"] == "session.abort")
                    .count()
            })
            .unwrap_or(0);
        if sv["running"] == json!(false) && ends >= min_ends {
            return sv;
        }
        assert!(Instant::now() < deadline, "timed out waiting for turn: {sv}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

struct InvBoot {
    _tmp: TempDir,
    data_dir: PathBuf,
    env: ResearchEnv,
    srv: RunningDesktopServer,
}

impl InvBoot {
    fn start(mock_url: &str) -> Self {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        let srv =
            start_desktop_server(test_cfg(&data_dir, None, Some(mock_url.to_string()), None)).unwrap();
        req(
            &srv,
            "PUT",
            "/api/settings/providers",
            Some(&json!({
                "keys": { "anthropic": "test-ant-key" },
                "paper_readers_enabled": null
            })),
        );
        let env = ResearchEnv::new(&data_dir);
        Self { _tmp: tmp, data_dir, env, srv }
    }

    fn new_session(&self, sid: &str, file_name: &str, bytes: &[u8]) -> IngestedSource {
        req(
            &self.srv,
            "POST",
            "/api/sessions",
            Some(&json!({ "id": sid, "mode": "investigation" })),
        );
        let db = Mutex::new(Ledger::open(&self.data_dir).unwrap());
        ingest_bytes(
            &self.env.sandbox(),
            &db,
            &IngestRequest {
                session_id: sid,
                file_name,
                origin: "upload",
                raw_bytes: bytes,
                source_id_hint: None,
                section_filter: None,
                title: Some("Audit Paper"),
            },
        )
        .unwrap()
    }
}

const SECTIONED_PAPER: &str = "\
1. Introduction\n\
Throughout this paper c = 1.\n\
We introduce a boundary functional.\n\
2. Boundary Terms\n\
In Section 2, eta denotes the outward normal.\n\
Theorem 2: The boundary variation vanishes for fixed endpoints.\n\
3. Conclusion\n\
Future work treats moving boundaries.\n";

fn build_two_page_pdf(page1_lines: &[&str], page2_lines: &[&str]) -> Vec<u8> {
    let stream_for = |lines: &[&str]| -> Vec<u8> {
        let mut s = String::from("BT /F1 12 Tf 72 720 Td 14 TL ");
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                s.push_str("T* ");
            }
            s.push_str(&format!("({line}) Tj "));
        }
        s.push_str("ET");
        format!("<< /Length {} >>\nstream\n{}\nendstream", s.len(), s).into_bytes()
    };
    let objs: Vec<(usize, Vec<u8>)> = vec![
        (1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()),
        (2, b"<< /Type /Pages /Kids [6 0 R 7 0 R] /Count 2 >>".to_vec()),
        (3, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec()),
        (4, stream_for(page1_lines)),
        (5, stream_for(page2_lines)),
        (6, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents 4 0 R >>".to_vec()),
        (7, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents 5 0 R >>".to_vec()),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0usize; objs.len() + 1];
    for (id, body) in &objs {
        offsets[*id] = out.len();
        writeln!(out, "{id} 0 obj").unwrap();
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref_pos = out.len();
    let total = objs.len() + 1;
    writeln!(out, "xref\n0 {total}\n0000000000 65535 f ").unwrap();
    for off in offsets.iter().skip(1) {
        writeln!(out, "{off:010} 00000 n ").unwrap();
    }
    writeln!(out, "trailer\n<< /Size {total} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF").unwrap();
    out
}

#[test]
fn section_scoped_map_then_conventions_builds_section_catalog_before_expanding_to_paper() {
    let calls = Arc::new(Mutex::new(0usize));
    let calls_cl = Arc::clone(&calls);
    let src_holder: Arc<Mutex<Option<(String, String)>>> = Arc::new(Mutex::new(None));
    let src_cl = Arc::clone(&src_holder);
    let mock = LoopbackApi::start(Box::new(move |_, _| {
        let mut n = calls_cl.lock().unwrap();
        *n += 1;
        if *n == 1 {
            let (source_id, primary_path) = src_cl.lock().unwrap().clone().unwrap();
            let anchor6 = json!({
                "source_id": source_id, "path": primary_path,
                "start_line": 6, "end_line": 6,
                "quote": "The boundary variation vanishes for fixed endpoints.", "kind": "passage"
            });
            json!({
                "id": "msg_map",
                "type": "message",
                "role": "assistant",
                "model": "claude-sonnet-4-6",
                "stop_reason": "tool_use",
                "content": [
                    {
                        "type": "tool_use",
                        "id": "tu_sum",
                        "name": "record_paper_summary",
                        "input": {
                            "source_id": source_id,
                            "what": { "text": "Boundary terms vanish.", "anchors": [anchor6] },
                            "why": { "unstated": true },
                            "how": { "unstated": true }
                        }
                    },
                    {
                        "type": "tool_use",
                        "id": "tu_map",
                        "name": "record_paper_map",
                        "input": {
                            "source_id": source_id, "title": "Boundary Terms", "coverage": "partial",
                            "limitations": ["Scoped to Section 2."],
                            "nodes": [{
                                "id": "thm-2", "kind": "result", "main": true,
                                "sentence": "The boundary variation vanishes for fixed endpoints.", "anchors": [anchor6]
                            }],
                            "edges": []
                        }
                    }
                ],
                "usage": { "input_tokens": 15, "output_tokens": 25 }
            })
        } else {
            ant_text("Done.")
        }
    }));
    let boot = InvBoot::start(&mock.base_url);
    let sid = "s-sec-conv";
    let src = boot.new_session(sid, "sections.md", SECTIONED_PAPER.as_bytes());
    *src_holder.lock().unwrap() = Some((src.source_id.clone(), src.primary_path.clone().unwrap()));

    let (st, _) = req(
        &boot.srv,
        "POST",
        &format!("/api/sessions/{sid}/messages"),
        Some(&json!({
            "prompt": "Help me understand section 2",
            "investigation_source_id": src.source_id,
            "investigation_scope": "map",
            "provider": "anthropic",
            "model": "claude-sonnet-4-6"
        })),
    );
    assert_eq!(st, 202);
    let after_turn1 = wait_idle(&boot.srv, sid, 1);
    let start_ctx = after_turn1["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == "session.start")
        .and_then(|e| e["payload"]["project_context"].as_str())
        .unwrap()
        .to_string();
    let map_seq = after_turn1["investigation"]["map_seq"].as_u64().unwrap();
    assert!(after_turn1["investigation"]["conventions"].is_null());

    // Click "Find paper conventions" when catalog is still null (per D4).
    let (conv_st, _) = req(
        &boot.srv,
        "POST",
        &format!("/api/sessions/{sid}/investigation/conventions"),
        Some(&json!({ "map_seq": map_seq, "provider": "anthropic", "model": "claude-sonnet-4-6" })),
    );
    assert_eq!(conv_st, 202);
    let after_turn2 = wait_idle(&boot.srv, sid, 2);
    let events = after_turn2["events"].as_array().unwrap();
    let conv_resumed = events.iter().rev().find(|e| e["kind"] == "investigation.conventions_resumed").unwrap();
    let resume_ctx = events
        .iter()
        .rev()
        .find(|e| e["kind"] == "session.resume")
        .and_then(|e| e["payload"]["project_context"].as_str())
        .unwrap();

    assert_eq!(
        (
            start_ctx.contains("(lines 4–6) (lines 4-6)"),
            start_ctx.contains("record conventions for the scope"),
            &conv_resumed["payload"]["scope"],
            resume_ctx.contains("Expand the section conventions catalog"),
        ),
        (false, false, &json!("section"), false),
        "Section-scoped map + conventions bugs:\nstart_ctx={start_ctx}\nconv_resumed={conv_resumed}\nresume_ctx={resume_ctx}"
    );
}

#[test]
fn incomplete_section_conventions_resume_stays_section_scoped_and_auto_enables_readers() {
    let mock = LoopbackApi::start(Box::new(|_, _| ant_text("Done.")));
    let boot = InvBoot::start(&mock.base_url);
    let sid = "s-inc-sec-conv";
    let src = boot.new_session(sid, "sections.md", SECTIONED_PAPER.as_bytes());
    let ledger = Ledger::open(&boot.data_dir).unwrap();
    ledger.append(sid, "investigation.paper", 1, &json!({ "source_id": src.source_id })).unwrap();
    let map_req_seq = ledger
        .append(
            sid,
            "investigation.mapping_requested",
            1,
            &json!({
                "source_id": src.source_id,
                "conventions_scope": "section",
                "scope": {
                    "status": "resolved",
                    "label": "Section 2 (lines 4–6)",
                    "targets": [{ "kind": "section", "label": "Section 2", "start_line": 4, "end_line": 6 }],
                    "ranges": [[4, 6]]
                }
            }),
        )
        .unwrap();
    let anchor5 = json!({
        "source_id": src.source_id, "path": src.primary_path,
        "start_line": 5, "end_line": 5,
        "quote": "eta denotes the outward normal.", "kind": "passage"
    });
    let map_seq = record(
        &boot.env,
        &ledger,
        sid,
        &json!({
            "source_id": src.source_id, "title": "Boundary Terms", "coverage": "partial",
            "limitations": ["Scoped to Section 2."],
            "nodes": [{ "id": "thm-2", "kind": "result", "main": true, "sentence": "Boundary vanishes.", "anchors": [anchor5] }],
            "edges": []
        }),
    )
    .unwrap()["map_seq"]
        .as_u64()
        .unwrap();

    record_conventions(
        &boot.env,
        &ledger,
        sid,
        &json!({
            "request_seq": map_req_seq, "source_id": src.source_id,
            "batch_index": 1, "done": false, "coverage": "partial", "limitations": ["Scoped to Section 2."],
            "items": [{
                "id": "normal-eta", "category": "notation", "label": "eta",
                "meaning": "Outward normal", "scope": "Section 2", "basis": "explicit", "anchors": [anchor5]
            }]
        }),
    )
    .unwrap();

    let (st, _) = req(
        &boot.srv,
        "POST",
        &format!("/api/sessions/{sid}/investigation/conventions"),
        Some(&json!({ "map_seq": map_seq, "provider": "anthropic", "model": "claude-sonnet-4-6" })),
    );
    assert_eq!(st, 202);
    let sv = wait_idle(&boot.srv, sid, 1);
    let conv_resumed = sv["events"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|e| e["kind"] == "investigation.conventions_resumed")
        .unwrap();

    assert_eq!(
        (
            &conv_resumed["payload"]["scope"],
            mock.captured().iter().any(|(_, b)| has_tool(b, "delegate_read_paper")),
        ),
        (&json!("section"), true),
        "Incomplete section catalog prematurely expanded to paper AND section_scoped_marker checked `conventions_scope` instead of `scope`: {conv_resumed}"
    );
}

#[test]
fn pdf_selected_pages_clips_boundary_section_scope_and_reader_slices() {
    let mock = LoopbackApi::start(Box::new(|_, _| ant_text("Summary on page 2.")));
    let boot = InvBoot::start(&mock.base_url);
    let sid = "s-pdf-clip";
    let pdf_bytes = build_two_page_pdf(
        &["1. INTRODUCTION", "Page one intro.", "2. BOUNDARY TERMS", "Page one setup of section two."],
        &["Page two continuation of section two.", "3. CONCLUSION", "Page two closing."],
    );
    let extracted = research_tools::pdf::extract_pdf_bytes(&pdf_bytes).unwrap();
    let page2_first_line = extracted
        .page_lines
        .iter()
        .find(|&&(p, _, _)| p == 2)
        .map(|&(_, first, _)| first)
        .expect("2-page PDF has page 2 lines");
    let src = boot.new_session(sid, "two_page.pdf", &pdf_bytes);

    let (st, _) = req(
        &boot.srv,
        "POST",
        &format!("/api/sessions/{sid}/messages"),
        Some(&json!({
            "prompt": "Help me understand section 2",
            "investigation_source_id": src.source_id,
            "investigation_page_start": 2,
            "investigation_page_end": 2,
            "investigation_scope": "map",
            "provider": "anthropic",
            "model": "claude-sonnet-4-6"
        })),
    );
    assert_eq!(st, 202);
    let sv = wait_idle(&boot.srv, sid, 1);

    let scope = &sv["investigation"]["scope"];
    let range_start = scope["ranges"][0][0].as_u64().unwrap() as usize;
    let range_end = scope["ranges"][0][1].as_u64().unwrap() as usize;
    let slices = plan_slices(
        &extracted.text,
        &extracted.page_lines,
        Some((2, 2)),
        Some((range_start, range_end)),
        2,
    );

    assert!(
        range_start >= page2_first_line && slices.iter().all(|sl| sl.start_line >= page2_first_line),
        "clip_to_selected_pages left page-1 line {range_start} (< page 2 start {page2_first_line}) in ScopeResolution ({scope}) and plan_slices ({slices:?})"
    );
}

#[test]
fn external_client_cannot_inject_internal_paper_scope_and_question_only_can_open_research() {
    let mock = LoopbackApi::start(Box::new(|_, _| ant_text("Answer from lines [L1].")));
    let boot = InvBoot::start(&mock.base_url);
    let sid = "s-spoof";
    let src = boot.new_session(sid, "flat.txt", b"Line 1: Fixed endpoints.\nLine 2: Zero boundary term.\nLine 3: Done.\n");

    let (st, _) = req(
        &boot.srv,
        "POST",
        &format!("/api/sessions/{sid}/messages"),
        Some(&json!({
            "prompt": "Map the whole paper",
            "investigation_source_id": src.source_id,
            "investigation_scope": "map",
            "_paper_scope": { "resolved": true, "start_line": 1, "end_line": 2 },
            "provider": "anthropic",
            "model": "claude-sonnet-4-6"
        })),
    );
    assert_eq!(st, 202);
    wait_idle(&boot.srv, sid, 1);
    let turn1_has_readers = mock.captured().iter().any(|(_, b)| has_tool(b, "delegate_read_paper"));

    let sid_q = "s-question-only";
    let src_q = boot.new_session(sid_q, "flat.txt", b"Line 1: Fixed endpoints.\nLine 2: Zero boundary term.\n");
    req(
        &boot.srv,
        "POST",
        &format!("/api/sessions/{sid_q}/messages"),
        Some(&json!({
            "prompt": "What vanishes?",
            "investigation_source_id": src_q.source_id,
            "investigation_scope": "question",
            "provider": "anthropic",
            "model": "claude-sonnet-4-6"
        })),
    );
    let sv_q = wait_idle(&boot.srv, sid_q, 1);
    assert_eq!(sv_q["investigation"]["answers"].as_array().unwrap().len(), 1);
    let (handoff_st, handoff_body) = req(
        &boot.srv,
        "POST",
        &format!("/api/sessions/{sid_q}/investigation/research"),
        Some(&json!({ "question": "Extend to moving endpoints" })),
    );

    assert_eq!(
        (turn1_has_readers, handoff_st),
        (false, 201),
        "Spoofed `_paper_scope` enabled readers ({turn1_has_readers}) AND question-only Research handoff failed ({handoff_st}: {handoff_body})"
    );
}
