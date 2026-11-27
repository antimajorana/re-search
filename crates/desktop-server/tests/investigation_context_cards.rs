//! End-to-end integration tests for Investigate V12 (`record_context_cards`, Prerequisites &
//! Compare hypotheses tasks, Rule 6.8/6.9 provenance + never-downgrade, mapview, and UI asset).

mod common;

use common::{raw_http, test_cfg};
use desktop_server::{start_desktop_server, RunningDesktopServer};
use ledger::Ledger;
use research_tools::investigation_markers::{
    EVENT_INVESTIGATION_CONTEXT, MARKER_TASK_FINISHED, MARKER_TASK_REQUESTED,
};
use research_tools::lines_sent::CONTEXT_SUPPLIED_EVENT;
use research_tools::map_defs::RECORD_CONTEXT_CARDS;
use research_tools::paper_key::PaperKey;
use research_tools::skeleton::{build_tex_skeleton_from_ingested, CITED_STATUS_FOUND};
use research_tools::tool_profile::{MARKER_INVESTIGATION_STARTED, PROFILE_INVESTIGATION_V2};
use research_tools::{ingest_bytes, IngestRequest, ResearchEnv};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PRIMARY_PAPER_TEX: &[u8] = br#"\documentclass{article}
\newtheorem{definition}{Definition}
\newtheorem{theorem}{Theorem}
\begin{document}
\section{Notation}
Let $\gamma > 0$ denote the fixed spectral gap constant.
\begin{definition}\label{def:kernel}
A kernel $K$ is admissible when $\|K\|_{L^1} \le 1$.
\end{definition}
\begin{theorem}\label{thm:main}
By Definition~\ref{def:kernel} and \cite{Smith24}, energy decays at rate $\gamma$.
\end{theorem}
\begin{thebibliography}{9}
\bibitem{Smith24} J. Smith, Exponential contraction estimates, 2024.
\end{thebibliography}
\end{document}
"#;

const EXTERNAL_CITED_TEX: &[u8] = br#"\documentclass{article}
\newtheorem{theorem}{Theorem}
\begin{document}
\section{Contraction}
\begin{theorem}[Uniform Contraction]\label{thm:ext}
Assume spectral gap $\lambda_1 \ge \gamma > 0$ and continuous boundary weight.
Then uniform exponential contraction holds on energy subspaces.
\end{theorem}
\end{document}
"#;

fn start_mock_anthropic_http(replies: Vec<Value>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let queue = Arc::new(Mutex::new(VecDeque::from(replies)));
    std::thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut len = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" || line == "\n" {
                    break;
                }
                if let Some((k, v)) = line.split_once(':') {
                    if k.eq_ignore_ascii_case("content-length") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                }
            }
            if len > 0 {
                let mut buf = vec![0u8; len];
                let _ = reader.read_exact(&mut buf);
            }
            let reply = queue.lock().unwrap().pop_front().unwrap_or_else(|| {
                json!({
                    "id": "msg_done", "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
                    "stop_reason": "end_turn", "content": [{ "type": "text", "text": "Done." }],
                    "usage": { "input_tokens": 8, "output_tokens": 6 }
                })
            });
            let bytes = serde_json::to_vec(&reply).unwrap();
            let hdr = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            );
            let _ = stream.write_all(hdr.as_bytes());
            let _ = stream.write_all(&bytes);
        }
    });
    format!("http://{addr}")
}

fn http_json(srv: &RunningDesktopServer, method: &str, path: &str, body: Option<&Value>) -> (u16, Value, String) {
    let hdr = [("X-ReSearch-Token", srv.info.token.as_str()), ("Content-Type", "application/json")];
    let bytes = body.map(|v| serde_json::to_vec(v).unwrap()).unwrap_or_default();
    let (st, _, raw) = raw_http(srv.addr.port(), method, path, &hdr, &bytes);
    (st, serde_json::from_slice(&raw).unwrap_or(Value::Null), String::from_utf8_lossy(&raw).into_owned())
}

fn wait_for_task_finish(srv: &RunningDesktopServer, sid: &str, expected_finishes: usize) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let (gst, sv, _) = http_json(srv, "GET", &format!("/api/sessions/{sid}"), None);
        assert_eq!(gst, 200);
        let fin_count = sv["events"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| e["kind"] == MARKER_TASK_FINISHED)
            .count();
        if sv["running"] == json!(false) && fin_count >= expected_finishes {
            return sv;
        }
        assert!(Instant::now() < deadline, "timed out waiting for task completion: {sv}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn line_in(bytes: &[u8], needle: &str) -> usize {
    std::str::from_utf8(bytes)
        .unwrap()
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains(needle))
        .map(|(i, _)| i + 1)
        .unwrap()
}

#[test]
fn v12_context_tasks_end_to_end_and_asset_check() {
    let dir = tempfile::tempdir().unwrap();
    let env = ResearchEnv::new(dir.path());
    let sid = "s-v12-context-e2e";

    let db = Mutex::new(Ledger::open(dir.path()).unwrap());
    let pri_ing = ingest_bytes(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: sid,
            file_name: "paper.tex",
            origin: "arxiv:2503.00010v1",
            raw_bytes: PRIMARY_PAPER_TEX,
            source_id_hint: Some("src_pri"),
            section_filter: None,
            title: Some("Primary Energy Decay"),
        },
    )
    .unwrap();
    let ext_ing = ingest_bytes(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: sid,
            file_name: "smith24.tex",
            origin: "arxiv:2403.00020v1",
            raw_bytes: EXTERNAL_CITED_TEX,
            source_id_hint: Some("src_ext"),
            section_filter: None,
            title: Some("Smith24 Contraction"),
        },
    )
    .unwrap();
    let ledger = db.into_inner().unwrap();

    let events = ledger.read_session(sid).unwrap();
    let pri_rec = events.iter().find(|e| e.kind == "source.ingested" && e.payload["source_id"] == pri_ing.source_id).unwrap();
    let ext_rec = events.iter().find(|e| e.kind == "source.ingested" && e.payload["source_id"] == ext_ing.source_id).unwrap();

    let (pri_skel, _) = build_tex_skeleton_from_ingested(
        &env.sandbox(), sid, pri_rec, PaperKey::from_arxiv("2503.00010").unwrap(), Some("v1".into()),
    ).unwrap();
    let (ext_skel, _) = build_tex_skeleton_from_ingested(
        &env.sandbox(), sid, ext_rec, PaperKey::from_arxiv("2403.00020").unwrap(), Some("v1".into()),
    ).unwrap();

    let pri_thm_tag = pri_skel.skeleton.nodes.iter().find(|n| n.label.as_deref() == Some("thm:main")).unwrap().tag.clone().unwrap();
    let ext_thm_tag = ext_skel.skeleton.nodes.iter().find(|n| n.label.as_deref() == Some("thm:ext")).unwrap().tag.clone().unwrap();

    let pri_blob = ledger.put_blob(&serde_json::to_vec(&pri_skel.skeleton).unwrap()).unwrap();
    ledger.append(sid, MARKER_INVESTIGATION_STARTED, 1, &json!({ "tool_profile": PROFILE_INVESTIGATION_V2 })).unwrap();
    ledger.append(sid, "investigation.paper", 1, &json!({ "source_id": pri_ing.source_id })).unwrap();
    let skel_seq = ledger.append(sid, "investigation.skeleton", 1, &json!({
        "source_id": pri_ing.source_id, "source_seq": pri_ing.event_seq, "blob": pri_blob,
    })).unwrap();
    // Seed a located Smith24 citation lookup (V11).
    let last_seed_seq = ledger.append(sid, "investigation.cited_lookup", 1, &json!({
        "tag": pri_thm_tag,
        "bib_key": "Smith24",
        "short_ref": "Smith 2024, Thm. 1",
        "resolved": { "paper_key": "arxiv:2403.00020", "version": "v1" },
        "external_source_id": ext_ing.source_id,
        "external_tag": ext_thm_tag,
        "status": CITED_STATUS_FOUND,
    })).unwrap();

    let pri_path = pri_ing.primary_path.clone().unwrap();
    let ext_path = ext_ing.primary_path.clone().unwrap();
    let gap_line = line_in(PRIMARY_PAPER_TEX, "fixed spectral gap");
    let def_line = line_in(PRIMARY_PAPER_TEX, "admissible when");
    let ext_h_line = line_in(EXTERNAL_CITED_TEX, "continuous boundary weight");
    let ext_c_line = line_in(EXTERNAL_CITED_TEX, "uniform exponential contraction");

    let prereq_req_seq = last_seed_seq + 1;
    let base_url = start_mock_anthropic_http(vec![
        // Turn 1 (Prerequisites task): read_map_node on Section 1 then save paper + model_background cards.
        json!({
            "id": "m1", "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
            "stop_reason": "tool_use",
            "content": [{
                "type": "tool_use", "id": "tu_r1", "name": "read_map_node",
                "input": { "source_id": pri_ing.source_id, "node": "Section 1" }
            }],
            "usage": { "input_tokens": 12, "output_tokens": 12 }
        }),
        json!({
            "id": "m2", "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
            "stop_reason": "tool_use",
            "content": [{
                "type": "tool_use", "id": "tu_c1", "name": RECORD_CONTEXT_CARDS,
                "input": {
                    "request_seq": prereq_req_seq,
                    "source_id": pri_ing.source_id,
                    "tag": pri_thm_tag,
                    "cards": [
                        {
                            "type": "symbol",
                            "title": "Spectral gap gamma",
                            "body": "Positive spectral gap parameter fixed in Section 1.",
                            "anchors": [{
                                "source_id": pri_ing.source_id, "path": pri_path,
                                "start_line": gap_line, "end_line": gap_line,
                                "quote": "fixed spectral gap constant"
                            }]
                        },
                        {
                            "type": "prerequisite",
                            "title": "Standard energy norm decay background",
                            "body": "Gronwall decay estimate on weighted spaces.",
                            "anchors": []
                        }
                    ]
                }
            }],
            "usage": { "input_tokens": 14, "output_tokens": 18 }
        }),
        json!({
            "id": "m3", "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
            "stop_reason": "end_turn",
            "content": [{ "type": "text", "text": "Recorded prerequisites." }],
            "usage": { "input_tokens": 8, "output_tokens": 6 }
        }),
    ]);

    let srv = start_desktop_server(test_cfg(dir.path(), None, Some(base_url), None)).unwrap();
    let (cfg_st, _, cfg_raw) = http_json(&srv, "PUT", "/api/settings/providers", Some(&json!({
        "default_provider": "anthropic", "default_model": "claude-haiku-4-5",
        "auto_approve_tools": true, "keys": { "anthropic": "fixture-key" }
    })));
    assert_eq!(cfg_st, 200, "{cfg_raw}");

    // Static UI asset serves cleanly and obeys wording rules.
    let (ast, _, asset_raw) = raw_http(srv.addr.port(), "GET", "/assets/investigation_context_cards.js", &[], &[]);
    assert_eq!(ast, 200);
    let asset_txt = String::from_utf8_lossy(&asset_raw);
    assert!(asset_txt.contains("PROVENANCE_STYLES"));
    assert!(asset_txt.contains("Hide unchecked"));
    assert!(!asset_txt.to_ascii_lowercase().contains("verified"));

    // Run Task 1: Prerequisites.
    let (st1, res1, raw1) = http_json(
        &srv,
        "POST",
        &format!("/api/sessions/{sid}/investigation/tasks"),
        Some(&json!({ "task": "context", "tags": [pri_thm_tag], "focus": "Prerequisites" })),
    );
    assert_eq!(st1, 202, "{raw1}");
    assert_eq!(res1["cap"], 8);
    assert_eq!(res1["skeleton_seq"], skel_seq);
    let sv1 = wait_for_task_finish(&srv, sid, 1);
    let evs1 = sv1["events"].as_array().unwrap();
    let fin1 = evs1.iter().rfind(|e| e["kind"] == MARKER_TASK_FINISHED).unwrap();
    assert_eq!(fin1["payload"]["status"], "done", "{fin1}");
    assert_eq!(fin1["payload"]["counts"]["items"], 2, "evs1={evs1:?}");
    srv.shutdown();

    // Run Task 2: Compare hypotheses with external cited node + bad anchor non-downgrade check.
    let next_req_seq = evs1.last().unwrap()["seq"].as_u64().unwrap() + 1;
    let base_url2 = start_mock_anthropic_http(vec![
        json!({
            "id": "m4", "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
            "stop_reason": "tool_use",
            "content": [{
                "type": "tool_use", "id": "tu_c2", "name": RECORD_CONTEXT_CARDS,
                "input": {
                    "request_seq": next_req_seq,
                    "source_id": pri_ing.source_id,
                    "tag": pri_thm_tag,
                    "cards": [
                        {
                            "type": "cited_result",
                            "title": "Smith24 Contraction Result",
                            "body": "Requires spectral gap and continuous boundary weight.",
                            "bib_key": "Smith24",
                            "anchors": [{
                                "source_id": ext_ing.source_id, "path": ext_path,
                                "start_line": ext_c_line, "end_line": ext_c_line,
                                "quote": "uniform exponential contraction holds"
                            }],
                            "hypotheses": [{
                                "text": "Continuous boundary weight",
                                "anchor": {
                                    "source_id": ext_ing.source_id, "path": ext_path,
                                    "start_line": ext_h_line, "end_line": ext_h_line,
                                    "quote": "continuous boundary weight"
                                }
                            }]
                        },
                        {
                            "type": "prerequisite",
                            "title": "Bad anchor MUST NOT downgrade",
                            "body": "Short text under 300.",
                            "anchors": [{
                                "source_id": pri_ing.source_id, "path": pri_path,
                                "start_line": def_line, "end_line": def_line,
                                "quote": "nonexistent fabricated quote"
                            }]
                        }
                    ]
                }
            }],
            "usage": { "input_tokens": 16, "output_tokens": 20 }
        }),
        json!({
            "id": "m5", "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
            "stop_reason": "end_turn",
            "content": [{ "type": "text", "text": "Compared hypotheses." }],
            "usage": { "input_tokens": 8, "output_tokens": 6 }
        }),
    ]);

    let srv2 = start_desktop_server(test_cfg(dir.path(), None, Some(base_url2), None)).unwrap();
    let (st2, res2, raw2) = http_json(
        &srv2,
        "POST",
        &format!("/api/sessions/{sid}/investigation/tasks"),
        Some(&json!({ "task": "context", "tags": [pri_thm_tag], "focus": "Compare hypotheses" })),
    );
    assert_eq!(st2, 202, "{raw2}");
    assert_eq!(res2["request_seq"], next_req_seq);
    let sv2 = wait_for_task_finish(&srv2, sid, 2);
    let evs2 = sv2["events"].as_array().unwrap();
    let req2_marker = evs2.iter().find(|e| e["seq"] == next_req_seq && e["kind"] == MARKER_TASK_REQUESTED).unwrap();
    assert_eq!(req2_marker["payload"]["focus"], "compare_hypotheses");
    let run_id_str = next_req_seq.to_string();
    assert!(evs2.iter().any(|e| {
        e["kind"] == CONTEXT_SUPPLIED_EVENT
            && e["payload"]["run_id"].as_str() == Some(run_id_str.as_str())
            && e["payload"]["ranges"].as_array().is_some_and(|rs| rs.iter().any(|r| r["source_id"] == ext_ing.source_id))
    }));
    let fin2 = evs2.iter().rfind(|e| e["kind"] == MARKER_TASK_FINISHED).unwrap();
    assert_eq!(fin2["payload"]["status"], "done");
    assert_eq!(fin2["payload"]["counts"]["items"], 1);
    assert_eq!(fin2["payload"]["counts"]["rejected_items"], 1);

    let ctx_evs: Vec<&Value> = evs2.iter().filter(|e| e["kind"] == EVENT_INVESTIGATION_CONTEXT).collect();
    assert_eq!(ctx_evs.len(), 2);

    // Mapview endpoint projects all 3 accepted context cards with F6 provenance + short_ref + zero excerpts.
    let (mst, mv, mraw) = http_json(&srv2, "GET", &format!("/api/sessions/{sid}/investigation/mapview"), None);
    assert_eq!(mst, 200, "{mraw}");
    assert!(!mraw.to_ascii_lowercase().contains("verified"));
    let thm_mv = mv["nodes"].as_array().unwrap().iter().find(|n| n["id"] == pri_thm_tag).unwrap();
    let cards = thm_mv["context_cards"].as_array().unwrap();
    assert_eq!(cards.len(), 3);
    assert_eq!(cards[0]["origin"], "paper");
    assert_eq!(cards[0]["provenance"], "from_paper");
    assert_eq!(cards[1]["origin"], "model_background");
    assert_eq!(cards[1]["provenance"], "model_background");
    assert_eq!(cards[2]["origin"], "external");
    assert_eq!(cards[2]["provenance"], "outside_paper");
    assert_eq!(cards[2]["short_ref"], "Smith 2024, Thm. 1");
    assert!(cards[2]["anchors"][0].get("excerpt").is_none());
    srv2.shutdown();
}
