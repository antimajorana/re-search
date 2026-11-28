//! End-to-end tests for Explore reading queue (X9), passage reader (X10), and map/verdict (X11).
//!
//! Zero live network/model calls, zero third-party paper text, zero skipped tests.

mod common;

use common::{raw_http, test_cfg};
use desktop_server::idea::{
    append_explore_session_start, build_idea_statement, IdeaPreset, IdeaStage,
    EVENT_IDEA_BRIEF, EVENT_IDEA_CANDIDATES, EVENT_IDEA_PAPER, EVENT_IDEA_STAGE,
    EVENT_IDEA_TRIAGE,
};
use desktop_server::idea_read::{run_reading_queue_with_router, ReadQueueResult};
use desktop_server::start_desktop_server;
use ledger::{sha256_hex, Ledger};
use research_tools::idea_tools::{
    idea_fetch_papers_def, idea_fetch_papers_handler_with_gate, register_explore_v1_tools,
};
use research_tools::library::{
    FulltextRecord, Library, LibraryIds, PaperFlags, PaperRecord, ORIGIN_USER_LIBRARY,
};
use research_tools::net_rate::{FakeRateClock, NetRateGate};
use research_tools::paper_key::PaperKey;
use research_tools::quote_core::verify_quote;
use research_tools::{research_policy, ResearchEnv};
use serde_json::{json, Value};
use std::io::Write;
use std::sync::Arc;
use tempfile::TempDir;
use tools::{ApprovalReply, ApprovalRequest, Approver, Registry, Router};

struct ApproveAll;
impl Approver for ApproveAll {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        ApprovalReply {
            approved: true,
            digest: req.digest.clone(),
            by: "idea-read-map-test".into(),
        }
    }
}

struct DenyAll;
impl Approver for DenyAll {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        ApprovalReply {
            approved: false,
            digest: req.digest.clone(),
            by: "idea-read-map-test".into(),
        }
    }
}

fn build_synthetic_pdf(pages: &[&str]) -> Vec<u8> {
    let n = pages.len().max(1);
    let mut objs: Vec<(usize, Vec<u8>)> = Vec::new();
    let first_page_id = 4usize;
    let first_stream_id = first_page_id + n;
    let kids: String = (0..n)
        .map(|i| format!("{} 0 R", first_page_id + i))
        .collect::<Vec<_>>()
        .join(" ");
    objs.push((1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()));
    objs.push((2, format!("<< /Type /Pages /Kids [{kids}] /Count {n} >>").into_bytes()));
    objs.push((3, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec()));
    for i in 0..n {
        let pid = first_page_id + i;
        let sid = first_stream_id + i;
        let dict = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents {sid} 0 R >>"
        );
        objs.push((pid, dict.into_bytes()));
    }
    for (i, text) in pages.iter().enumerate() {
        let sid = first_stream_id + i;
        let escaped = text.replace('\\', "\\\\").replace('(', "\\(").replace(')', "\\)");
        let body = format!("BT /F1 12 Tf 72 720 Td ({escaped}) Tj ET");
        let obj = format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len());
        objs.push((sid, obj.into_bytes()));
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
    writeln!(
        out,
        "trailer\n<< /Size {total_xref} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF"
    )
    .unwrap();
    out
}

fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { TABLE[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[(n & 63) as usize] as char } else { '=' });
    }
    out
}

fn seed_triaged_session(data_dir: &std::path::Path, sid: &str, preset: IdeaPreset, reading_keys: &[&str]) {
    let ledger = Ledger::open(data_dir).unwrap();
    let stmt = build_idea_statement(
        1,
        "Study Lanczos coefficients and Krylov complexity in a monitored Majorana chain.",
        &[],
        &[],
    )
    .unwrap();
    append_explore_session_start(&ledger, sid, preset, false, &stmt).unwrap();
    ledger
        .append(
            sid,
            EVENT_IDEA_BRIEF,
            1,
            &json!({
                "idea_rev": 1,
                "restatement": "Lanczos growth in a monitored Majorana chain.",
                "assumptions": ["free fermions"],
                "subclaims": [{ "id": "sc-1", "text": "b_n grows linearly", "idea_quote": "Lanczos coefficients" }],
                "terms": [{ "term": "Lanczos coefficients", "aliases": ["Krylov complexity"] }],
                "perspectives": [
                    { "id": "p-1", "kind": "exact", "question": "Exact?", "queries": [{ "provider": "arxiv", "q": "Lanczos Majorana" }] },
                    { "id": "p-2", "kind": "other_names", "question": "Other?", "queries": [{ "provider": "inspire", "q": "Krylov complexity" }] }
                ],
                "questions": [],
                "exclusions": []
            }),
        )
        .unwrap();
    ledger
        .append(sid, EVENT_IDEA_STAGE, 1, &json!({ "stage": IdeaStage::Sharpen.as_str(), "state": "finished" }))
        .unwrap();
    ledger
        .append(sid, EVENT_IDEA_STAGE, 1, &json!({ "stage": IdeaStage::Search.as_str(), "state": "finished" }))
        .unwrap();
    let cands: Vec<Value> = reading_keys
        .iter()
        .map(|k| json!({ "key": k, "title": format!("Fixture {k}"), "authors": ["Ada Example"], "year": 2025 }))
        .collect();
    ledger
        .append(sid, EVENT_IDEA_CANDIDATES, 1, &json!({ "round": 1, "candidates": cands }))
        .unwrap();
    let items: Vec<Value> = reading_keys
        .iter()
        .map(|k| json!({ "key": k, "relevance": "core", "subclaim_ids": ["sc-1"], "why": "Relevant fixture." }))
        .collect();
    ledger
        .append(
            sid,
            EVENT_IDEA_TRIAGE,
            1,
            &json!({ "items": items, "reading_list": reading_keys }),
        )
        .unwrap();
    ledger
        .append(sid, EVENT_IDEA_STAGE, 1, &json!({ "stage": IdeaStage::Triage.as_str(), "state": "finished" }))
        .unwrap();
}

#[test]
fn x9_reading_queue_runs_through_router_enforces_denial_and_records_idea_paper() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&vault).unwrap();

    let key_owned = "doi:10.5555/owned-x9-2025";
    let tex_path = vault.join("owned.tex");
    let tex_body = "\\section{Result}\nKrylov complexity saturates in the area-law phase for monitored chains.\n";
    std::fs::write(&tex_path, tex_body).unwrap();

    let lib = Library::open(&data_dir).unwrap();
    lib.upsert_source(&research_tools::library::SourceRecord {
        id: "src-vault-x9".into(),
        kind: research_tools::library::SourceKind::Folder,
        root: vault.to_string_lossy().to_string(),
        settings: json!({}),
        added_ms: 1,
        last_sync_ms: Some(1),
        state: "synced".into(),
    })
    .unwrap();
    let pkey = PaperKey::parse(key_owned).unwrap();
    lib.upsert_paper(&PaperRecord {
        key: pkey.clone(),
        ids: LibraryIds { doi: Some("10.5555/owned-x9-2025".into()), ..Default::default() },
        title: "Owned X9 Paper".into(),
        authors: vec!["Ada Example".into()],
        year: Some(2025),
        abstract_text: String::new(),
        bibtex: String::new(),
        flags: PaperFlags { owned: true, ..Default::default() },
        first_seen_ms: 1,
        last_seen_ms: 1,
    })
    .unwrap();
    lib.upsert_user_library_fulltext(&FulltextRecord {
        key: pkey,
        version: None,
        raw_blob: None,
        format: "tex".into(),
        bytes: tex_body.len() as u64,
        origin: ORIGIN_USER_LIBRARY.into(),
        fetched_ms: 1,
        file_path: Some(tex_path.to_string_lossy().to_string()),
        file_size: Some(tex_body.len() as u64),
        file_mtime_ms: Some(1),
        file_sha256: Some(sha256_hex(tex_body.as_bytes())),
    })
    .unwrap();

    let sid = "s-x9-queue";
    seed_triaged_session(&data_dir, sid, IdeaPreset::Standard, &[key_owned]);

    // 1. Denied router approval refuses reading queue without bypassing Router::try_call (Rule P5).
    let env = Arc::new(ResearchEnv::new(&data_dir));
    let gate = Arc::new(NetRateGate::new(Arc::new(FakeRateClock::new())));
    let mut deny_reg = Registry::new();
    register_explore_v1_tools(
        &mut deny_reg,
        Arc::clone(&env),
        Some(idea_fetch_papers_handler_with_gate(Arc::clone(&env), Some(Arc::clone(&gate)))),
    );
    let deny_router = Router::new(
        deny_reg,
        research_policy(),
        Ledger::open(&data_dir).unwrap(),
        sid,
        Arc::new(DenyAll),
    );
    let ledger = Ledger::open(&data_dir).unwrap();
    let denied = run_reading_queue_with_router(&ledger, &deny_router, sid, None, None, |_, _| {}).unwrap();
    assert!(matches!(denied, ReadQueueResult::Denied { .. }), "expected Denied, got {denied:?}");

    // 2. Approved router executes `idea_fetch_papers`, ingests owned file, writes `idea.paper`.
    let mut ok_reg = Registry::new();
    ok_reg.register(
        idea_fetch_papers_def(),
        idea_fetch_papers_handler_with_gate(Arc::clone(&env), Some(gate)),
    );
    let ok_router = Router::new(
        ok_reg,
        research_policy(),
        Ledger::open(&data_dir).unwrap(),
        sid,
        Arc::new(ApproveAll),
    );
    let mut sse_kinds = Vec::new();
    let res = run_reading_queue_with_router(&ledger, &ok_router, sid, None, None, |kind, _| {
        sse_kinds.push(kind.to_string());
    })
    .unwrap();
    let ReadQueueResult::Finished { papers, ready_count, .. } = res else {
        panic!("expected Finished, got {res:?}");
    };
    assert_eq!(ready_count, 1);
    assert_eq!(papers[0].origin, ORIGIN_USER_LIBRARY);
    assert_eq!(papers[0].text_basis, "tex");
    assert!(papers[0].source_id.is_some());
    assert!(sse_kinds.iter().any(|k| k == EVENT_IDEA_PAPER));

    let events = ledger.read_session(sid).unwrap();
    assert!(events.iter().any(|e| e.kind == EVENT_IDEA_PAPER));
    assert!(events.iter().any(|e| e.kind == "tool.request" && e.payload["tool"] == "idea_fetch_papers"));
}

#[test]
fn x9_http_attach_pdf_ingests_bytes_and_updates_idea_view() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let cfg = test_cfg(&data_dir, None, None, None);
    let server = start_desktop_server(cfg).unwrap();
    let port = server.addr.port();
    let auth = [("X-ReSearch-Token", server.info.token.as_str())];

    let sid = "s-x9-attach";
    // Create session metadata via desktop state helper pattern by writing meta + ledger events.
    seed_triaged_session(&data_dir, sid, IdeaPreset::Standard, &["doi:10.5555/paywalled-pdf"]);
    let meta_dir = data_dir.join("sessions").join(sid);
    std::fs::create_dir_all(&meta_dir).unwrap();
    std::fs::write(
        meta_dir.join("meta.json"),
        serde_json::to_vec(&json!({
            "id": sid,
            "title": "X9 Attach PDF",
            "created_ms": 1
        }))
        .unwrap(),
    )
    .unwrap();

    let pdf = build_synthetic_pdf(&[
        "Page one synthetic fixture with Lanczos coefficient scaling law.",
        "Page two synthetic fixture with area-law saturation discussion.",
    ]);
    let body = serde_json::to_vec(&json!({
        "attach_pdf": {
            "paper_key": "doi:10.5555/paywalled-pdf",
            "file_name": "paywalled_fixture.pdf",
            "bytes_base64": encode_base64(&pdf),
        }
    }))
    .unwrap();
    let (status, _, raw) = raw_http(port, "POST", &format!("/api/sessions/{sid}/idea/read"), &auth, &body);
    assert_eq!(status, 200, "body: {}", String::from_utf8_lossy(&raw));
    let resp: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(resp["ok"], true);
    assert_eq!(resp["paper"]["origin"], "upload");
    assert_eq!(resp["paper"]["text_basis"], "pdf_text");
    let source_id = resp["paper"]["source_id"].as_str().unwrap().to_string();
    assert!(!source_id.is_empty());
    assert_eq!(resp["idea"]["papers"][0]["key"], "doi:10.5555/paywalled-pdf");

    // Verify quote verification works on the attached PDF's session-ingested copy (Rule C9).
    let ledger = Ledger::open(&data_dir).unwrap();
    let events = ledger.read_session(sid).unwrap();
    let src_ev = events
        .iter()
        .find(|e| e.kind == "source.ingested" && e.payload["source_id"] == source_id)
        .unwrap();
    let primary_path = src_ev.payload["primary_path"].as_str().unwrap();
    let sandbox = ResearchEnv::new(&data_dir).sandbox();
    let vq = verify_quote(
        &sandbox,
        sid,
        &events,
        &source_id,
        primary_path,
        1,
        1,
        "Page one synthetic fixture with Lanczos coefficient scaling law.",
    )
    .expect("attached PDF must verify against session-ingested copy");
    assert_eq!(vq.quote_ref.start_line, 1);
}

#[test]
fn x9_read_recovers_after_interrupted_stage_and_preserves_completed_reads() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let sid = "s-x9-crash-recover";
    let key = "doi:10.5555/already-read";
    seed_triaged_session(&data_dir, sid, IdeaPreset::Standard, &[key]);

    let ledger = Ledger::open(&data_dir).unwrap();
    // Simulate a paper that already finished X10 reading and a subsequent crash mid-Read stage.
    ledger
        .append(
            sid,
            EVENT_IDEA_PAPER,
            1,
            &json!({
                "key": key,
                "source_id": "src-existing-1",
                "source_seq": 5,
                "origin": "upload",
                "text_basis": "tex",
                "status": "read_passages",
                "lines_sent": 140,
                "total_lines": 500,
                "raw_blob": "a".repeat(64),
            }),
        )
        .unwrap();
    ledger
        .append(
            sid,
            EVENT_IDEA_STAGE,
            1,
            &json!({ "stage": "read", "state": "requested", "request_id": "crashed-1" }),
        )
        .unwrap();

    let env = Arc::new(ResearchEnv::new(&data_dir));
    let gate = Arc::new(NetRateGate::new(Arc::new(FakeRateClock::new())));
    let mut reg = Registry::new();
    reg.register(
        idea_fetch_papers_def(),
        idea_fetch_papers_handler_with_gate(env, Some(gate)),
    );
    let router = Router::new(
        reg,
        research_policy(),
        Ledger::open(&data_dir).unwrap(),
        sid,
        Arc::new(ApproveAll),
    );
    let res = run_reading_queue_with_router(&ledger, &router, sid, None, None, |_, _| {}).unwrap();
    let ReadQueueResult::Finished { papers, ready_count, .. } = res else {
        panic!("expected crash-recovered Finished, got {res:?}");
    };
    assert_eq!(ready_count, 1);
    assert_eq!(papers[0].status, "read_passages");
    assert_eq!(papers[0].lines_sent, 140);
}
