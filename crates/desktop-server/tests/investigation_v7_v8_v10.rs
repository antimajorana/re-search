//! End-to-end desktop HTTP integration tests for Investigate V7 (Orient, Rule 6.6 cues,
//! authors' half-badge, reader pins, auto-start setting), V8 (Open any node at arbitrary depth,
//! content-based `m-` tags, branch-only isolation, Rule 6.12 diff, Rule 6.15 Paused), and
//! V10 (additive reader notes surviving re-runs). Zero third-party paper text.

mod common;

use common::{raw_http, test_cfg};
use desktop_server::{start_desktop_server, RunningDesktopServer};
use ledger::Ledger;
use research_tools::investigation_markers::MARKER_TASK_FINISHED;
use research_tools::map_defs::{READ_MAP_NODE, RECORD_MAP_READING};
use research_tools::{ingest_bytes, IngestRequest, ResearchEnv};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SYNTHETIC_PAPER_TEX: &[u8] = br#"\documentclass{article}
\newtheorem{definition}{Definition}[section]
\newtheorem{lemma}[definition]{Lemma}
\newtheorem{theorem}[definition]{Theorem}
\begin{document}
\begin{abstract}
We prove an exponential decay bound for a toy damped lattice chain.
\end{abstract}
\section{Introduction}\label{sec:intro}
We recall the standard norm from \cite{Toy95} and establish our main decay theorem.
\begin{definition}[Lattice Energy]\label{def:energy}
For every chain state $u$, define the discrete energy $E(u) = \|u\|^2$.
\end{definition}
\begin{lemma}[\cite{Toy95}]\label{lem:coercive}
Recalled from \cite{Toy95}, the damping operator satisfies $D(u) \ge \gamma E(u)$.
\end{lemma}
\begin{theorem}[Main Decay Bound]\label{thm:main}
We prove that under Definition~\ref{def:energy} and Lemma~\ref{lem:coercive}, $E(u(t)) \le e^{-\gamma t} E(u(0))$.
\end{theorem}
\begin{proof}
Differentiating $E(u(t))$ along the flow gives $dE/dt = -2 D(u)$.
Applying Lemma~\ref{lem:coercive} yields $dE/dt \le -2\gamma E(u)$, and Gronwall completes the estimate.
\end{proof}
\section{Auxiliary Remarks}\label{sec:aux}
\begin{lemma}[Standalone Identity]\label{lem:unused}
Every finite diagonal matrix commutes with itself.
\end{lemma}
\section{Conclusion}\label{sec:conc}
Our main theorem gives unconditional decay on finite chains.
\begin{thebibliography}{9}
\bibitem{Toy95} A. Toy, Discrete damping identities, 1995.
\bibitem{Old88} B. Old, Matrix remarks, 1988.
\end{thebibliography}
\end{document}
"#;

struct ScriptedAnthropic {
    base_url: String,
    replies: Arc<Mutex<VecDeque<Value>>>,
}

impl ScriptedAnthropic {
    fn start(initial: Vec<Value>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let queue = Arc::new(Mutex::new(VecDeque::from(initial)));
        let q_bg = Arc::clone(&queue);
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
                let body_str = String::from_utf8_lossy(&body);
                let parsed_req_seq = extract_request_seq(&body_str);
                let mut reply = q_bg
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or_else(|| ant_text("Completed."));
                if let Some(rseq) = parsed_req_seq {
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
        Self { base_url: format!("http://{addr}"), replies: queue }
    }

    fn enqueue(&self, more: Vec<Value>) {
        self.replies.lock().unwrap().extend(more);
    }
}

fn extract_request_seq(body: &str) -> Option<u64> {
    let idx = body.rfind("request_seq=")?;
    let digits: String = body[idx + "request_seq=".len()..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
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
        "usage": { "input_tokens": 12, "output_tokens": 14 }
    })
}

fn ant_tool(id: &str, name: &str, input: Value) -> Value {
    json!({
        "id": format!("msg_{id}"), "type": "message", "role": "assistant", "model": "claude-haiku-4-5",
        "stop_reason": "tool_use", "content": [{ "type": "tool_use", "id": id, "name": name, "input": input }],
        "usage": { "input_tokens": 18, "output_tokens": 28 }
    })
}

fn req(srv: &RunningDesktopServer, method: &str, path: &str, body: Option<&Value>) -> (u16, Value, String) {
    let hdr = [("X-ReSearch-Token", srv.info.token.as_str()), ("Content-Type", "application/json")];
    let bytes = body.map(|v| serde_json::to_vec(v).unwrap()).unwrap_or_default();
    let (st, _, raw) = raw_http(srv.addr.port(), method, path, &hdr, &bytes);
    (st, serde_json::from_slice(&raw).unwrap_or(Value::Null), String::from_utf8_lossy(&raw).into_owned())
}

fn wait_task_finish(srv: &RunningDesktopServer, sid: &str, min_finished: usize) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let (st, sv, raw) = req(srv, "GET", &format!("/api/sessions/{sid}"), None);
        assert_eq!(st, 200, "GET session failed: {raw}");
        let count = sv["events"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| e["kind"] == MARKER_TASK_FINISHED)
            .count();
        if sv["running"] == json!(false) && count >= min_finished {
            return sv;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {min_finished} finished tasks: {sv}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn line_of(needle: &str) -> usize {
    std::str::from_utf8(SYNTHETIC_PAPER_TEX)
        .unwrap()
        .lines()
        .enumerate()
        .find_map(|(i, l)| l.contains(needle).then_some(i + 1))
        .unwrap_or_else(|| panic!("missing line containing `{needle}`"))
}

struct Setup {
    _dir: tempfile::TempDir,
    provider: ScriptedAnthropic,
    srv: RunningDesktopServer,
    sid: String,
    source_id: String,
    primary_path: String,
    thm_tag: String,
    thm_fp: String,
    lem_tag: String,
    lem_fp: String,
    unused_tag: String,
    def_line: usize,
}

fn boot_investigate_session(id: &str) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let provider = ScriptedAnthropic::start(vec![]);
    let srv = start_desktop_server(test_cfg(dir.path(), None, Some(provider.base_url.clone()), None)).unwrap();

    let (st, _, raw) = req(&srv, "PUT", "/api/settings/providers", Some(&json!({
        "default_provider": "anthropic",
        "default_model": "claude-haiku-4-5",
        "auto_approve_tools": true,
        "keys": { "anthropic": "fixture-key" }
    })));
    assert_eq!(st, 200, "{raw}");

    let (st, created, raw) = req(&srv, "POST", "/api/sessions", Some(&json!({ "id": id, "mode": "investigate" })));
    assert_eq!(st, 200, "{raw}");
    let sid = created["id"].as_str().unwrap().to_string();

    let env = ResearchEnv::new(dir.path());
    let ledger_m = Mutex::new(Ledger::open(dir.path()).unwrap());
    let ing = ingest_bytes(&env.sandbox(), &ledger_m, &IngestRequest {
        session_id: &sid,
        file_name: "toy_decay.tex",
        origin: "https://arxiv.org/e-print/2407.00123v1",
        raw_bytes: SYNTHETIC_PAPER_TEX,
        source_id_hint: None,
        section_filter: None,
        title: Some("Toy decay bound"),
    }).unwrap();
    drop(ledger_m);

    let (pin_st, pin_val, pin_raw) = req(
        &srv, "POST", &format!("/api/sessions/{sid}/investigation/skeleton"),
        Some(&json!({ "source_id": ing.source_id, "pin": true })),
    );
    assert_eq!(pin_st, 200, "{pin_raw}");
    let nodes = pin_val["skeleton"]["nodes"].as_array().unwrap();
    let find_lbl = |lbl: &str| {
        let n = nodes.iter().find(|n| n["label"].as_str() == Some(lbl)).unwrap();
        (n["tag"].as_str().unwrap().to_string(), n["fp"].as_str().unwrap().to_string())
    };
    let (thm_tag, thm_fp) = find_lbl("thm:main");
    let (lem_tag, lem_fp) = find_lbl("lem:coercive");
    let (unused_tag, _) = find_lbl("lem:unused");

    Setup {
        _dir: dir,
        provider,
        srv,
        sid,
        source_id: ing.source_id,
        primary_path: ing.primary_path.unwrap(),
        thm_tag,
        thm_fp,
        lem_tag,
        lem_fp,
        unused_tag,
        def_line: line_of("define the discrete energy"),
    }
}

fn find_node<'a>(mv: &'a Value, id: &str) -> &'a Value {
    mv["nodes"].as_array().unwrap().iter().find(|n| n["id"].as_str() == Some(id)).unwrap()
}

#[test]
fn set_p3_scripted_orient_rule_66_pins_skip_plan_and_settings() {
    let s = boot_investigate_session("s-v7-p3");

    // 1. Auth guard on settings & notes routes (401 no token, 403 foreign origin).
    let (unauth_s, _, _) = raw_http(s.srv.addr.port(), "GET", "/api/settings/investigate", &[], b"");
    assert_eq!(unauth_s, 401);
    let (unauth_n, _, _) = raw_http(
        s.srv.addr.port(), "POST", &format!("/api/sessions/{}/investigation/notes", s.sid),
        &[("Content-Type", "application/json")], b"{}",
    );
    assert_eq!(unauth_n, 401);
    let (bad_orig, _, _) = raw_http(
        s.srv.addr.port(), "PUT", "/api/settings/investigate",
        &[("X-ReSearch-Token", s.srv.info.token.as_str()), ("Origin", "https://evil.example"), ("Content-Type", "application/json")],
        br#"{"orient_auto_start":false}"#,
    );
    assert_eq!(bad_orig, 403);

    // 2. Orient auto-start setting defaults true and toggles cleanly.
    let (st, gs, _) = req(&s.srv, "GET", "/api/settings/investigate", None);
    assert_eq!(st, 200);
    assert_eq!(gs["orient_auto_start"], json!(true));
    let (st, ps, _) = req(&s.srv, "PUT", "/api/settings/investigate", Some(&json!({ "orient_auto_start": false })));
    assert_eq!(st, 200);
    assert_eq!(ps["orient_auto_start"], json!(false));

    // Rule 6.14 before Orient: skip_plan is empty and nothing is dimmed.
    let (st, mv0, _) = req(&s.srv, "GET", &format!("/api/sessions/{}/investigation/mapview", s.sid), None);
    assert_eq!(st, 200);
    assert_eq!(mv0["orient_auto_start"], json!(false));
    assert_eq!(mv0["skip_plan"]["empty"], json!(true));
    assert_eq!(find_node(&mv0, &s.unused_tag)["dim"], json!(false));

    // 3. Scripted Set P3 Orient run: cues present, heading cites, no-cue fallback, and bad bib rejection.
    let thm_l = line_of("We prove that under Definition");
    let lem_l = line_of("Recalled from");
    let unused_l = line_of("Every finite diagonal matrix");
    let anchor = |line: usize, quote: &str| json!({
        "source_id": s.source_id, "path": s.primary_path,
        "start_line": line, "end_line": line, "quote": quote
    });

    // Determine next request_seq by reading session events (skeleton + paper + start + source = 4).
    let (_, sv_before, _) = req(&s.srv, "GET", &format!("/api/sessions/{}", s.sid), None);
    let next_req_seq = sv_before["events"].as_array().unwrap().last().unwrap()["seq"].as_u64().unwrap() + 1;

    s.provider.enqueue(vec![
        ant_tool("t_read_unused", READ_MAP_NODE, json!({
            "source_id": s.source_id,
            "node": s.unused_tag
        })),
        ant_tool("t_orient", RECORD_MAP_READING, json!({
            "request_seq": next_req_seq,
            "source_id": s.source_id,
            "items": [
                {
                    "tag": s.thm_tag,
                    "role": "main_result",
                    "gist": "Proves exponential decay of discrete energy.",
                    "anchors": [anchor(thm_l, "We prove that under Definition")],
                    "authors_claim": {
                        "label": "new",
                        "relation_note": "extends Toy95 to finite chains",
                        "attributed_to": ["Toy95"],
                        "anchors": [anchor(thm_l, "We prove that under Definition")]
                    }
                },
                {
                    "tag": s.lem_tag,
                    "role": "supporting",
                    "gist": "Coercivity lower bound on damping.",
                    "anchors": [anchor(lem_l, "Recalled from \\cite{Toy95}, the damping operator")],
                    "authors_claim": {
                        "label": "restated",
                        "attributed_to": ["Toy95"],
                        "anchors": [anchor(lem_l, "Recalled from \\cite{Toy95}, the damping operator")]
                    }
                },
                {
                    "tag": s.unused_tag,
                    "role": "background",
                    "gist": "Diagonal matrices commute with themselves.",
                    "anchors": [anchor(unused_l, "Every finite diagonal matrix commutes with itself.")],
                    "authors_claim": {
                        "label": "background",
                        "anchors": [anchor(unused_l, "Every finite diagonal matrix commutes with itself.")]
                    }
                },
                {
                    "tag": s.lem_tag,
                    "authors_claim": {
                        "label": "restated",
                        "attributed_to": ["MISSING99"],
                        "anchors": [anchor(lem_l, "Recalled from \\cite{Toy95}, the damping operator")]
                    }
                }
            ],
            "edges": [],
            "limitations": [],
            "done": true
        })),
        ant_text("Orient complete."),
    ]);

    let (st, started, raw) = req(
        &s.srv, "POST", &format!("/api/sessions/{}/investigation/tasks", s.sid),
        Some(&json!({ "task": "orient" })),
    );
    assert_eq!(st, 202, "{raw}");
    assert_eq!(started["cap"], json!(8));
    wait_task_finish(&s.srv, &s.sid, 1);

    let (st, mv1, raw1) = req(&s.srv, "GET", &format!("/api/sessions/{}/investigation/mapview", s.sid), None);
    assert_eq!(st, 200);

    // Check Set P3 authors' badges, Rule 6.6 cues, and Rule 6.14 skip dimming.
    let thm_n = find_node(&mv1, &s.thm_tag);
    assert_eq!(thm_n["role"], json!("main_result"));
    assert_eq!(thm_n["effective_is_main_result"], json!(true));
    assert_eq!(thm_n["authors_claim"]["text"], json!("Authors: New (model)"));
    assert!(thm_n["authors_claim"]["cues"].as_array().unwrap().iter().any(|c| c == "we prove"));

    let lem_n = find_node(&mv1, &s.lem_tag);
    assert_eq!(lem_n["authors_claim"]["text"], json!("Authors: Restated (model)"));
    assert_eq!(lem_n["authors_claim"]["heading_cite_line"], json!("cites [Toy95] in its heading"));
    assert!(lem_n["authors_claim"]["cues"].as_array().unwrap().iter().any(|c| c == "recalled from"));

    let unused_n = find_node(&mv1, &s.unused_tag);
    assert_eq!(unused_n["authors_claim"]["cue_line"], json!("no explicit cue in the quoted lines"));
    assert_eq!(mv1["skip_plan"]["empty"], json!(false));
    assert_eq!(unused_n["dim"], json!(true));

    // 4. Reader pins override model role & recompute skip plan; wrong node_fp returns 409.
    let (bad_fp_st, _, _) = req(
        &s.srv, "POST", &format!("/api/sessions/{}/investigation/notes", s.sid),
        Some(&json!({ "tag": s.thm_tag, "node_fp": "wrongfp", "pin": "not_main_result" })),
    );
    assert_eq!(bad_fp_st, 409);

    let (st, _, raw) = req(
        &s.srv, "POST", &format!("/api/sessions/{}/investigation/notes", s.sid),
        Some(&json!({ "tag": s.thm_tag, "node_fp": s.thm_fp, "pin": "not_main_result" })),
    );
    assert_eq!(st, 201, "{raw}");
    let (_, mv_unpin, _) = req(&s.srv, "GET", &format!("/api/sessions/{}/investigation/mapview", s.sid), None);
    assert_eq!(find_node(&mv_unpin, &s.thm_tag)["effective_is_main_result"], json!(false));
    assert_eq!(mv_unpin["skip_plan"]["empty"], json!(true));
    assert_eq!(find_node(&mv_unpin, &s.unused_tag)["dim"], json!(false));

    let (st, _, _) = req(
        &s.srv, "POST", &format!("/api/sessions/{}/investigation/notes", s.sid),
        Some(&json!({ "tag": s.lem_tag, "node_fp": s.lem_fp, "pin": "main_result" })),
    );
    assert_eq!(st, 201);
    let (_, mv_repin, _) = req(&s.srv, "GET", &format!("/api/sessions/{}/investigation/mapview", s.sid), None);
    assert_eq!(find_node(&mv_repin, &s.lem_tag)["effective_is_main_result"], json!(true));
    assert_eq!(find_node(&mv_repin, &s.unused_tag)["dim"], json!(true));

    // Wording property tests on mapview JSON.
    let lower = raw1.to_ascii_lowercase();
    assert!(!lower.contains("verified"), "forbidden word `verified` in mapview");
    assert!(!lower.contains("attributed by the authors to"));
}

#[test]
fn v8_arbitrary_depth_open_rerun_stability_pause_and_v10_notes_persistence() {
    let s = boot_investigate_session("s-v8-v10");
    let thm_l = line_of("We prove that under Definition");
    let proof_l1 = line_of("Differentiating $E(u(t))$");
    let proof_l2 = line_of("Applying Lemma");
    let anchor = |line: usize, quote: &str| json!({
        "source_id": s.source_id, "path": s.primary_path,
        "start_line": line, "end_line": line, "quote": quote
    });

    // 1. Open thm:main (verify default cap = 10), creating 2 proof steps + 1 open question + hypothesis.
    let (_, sv0, _) = req(&s.srv, "GET", &format!("/api/sessions/{}", s.sid), None);
    let req1 = sv0["events"].as_array().unwrap().last().unwrap()["seq"].as_u64().unwrap() + 1;

    s.provider.enqueue(vec![
        ant_tool("t_open1", RECORD_MAP_READING, json!({
            "request_seq": req1,
            "source_id": s.source_id,
            "items": [
                {
                    "tag": s.thm_tag,
                    "gist": "Energy decays at rate 2 gamma.",
                    "purpose": "Main stability bound.",
                    "hypotheses": [{
                        "text": "Energy is defined as squared norm.",
                        "anchor": anchor(s.def_line, "define the discrete energy")
                    }],
                    "anchors": [anchor(thm_l, "We prove that under Definition")]
                },
                {
                    "new": { "id": "step-1", "attach_to": s.thm_tag, "kind": "proof_step" },
                    "gist": "Differentiate energy along the flow.",
                    "anchors": [anchor(proof_l1, "Differentiating $E(u(t))$ along the flow")]
                },
                {
                    "new": { "id": "step-2", "attach_to": s.thm_tag, "kind": "proof_step" },
                    "gist": "Apply coercivity and Gronwall.",
                    "anchors": [anchor(proof_l2, "Gronwall completes the estimate.")]
                },
                {
                    "new": { "id": "oq-1", "attach_to": s.thm_tag, "kind": "open_question" },
                    "gist": "Does gamma depend on chain length?",
                    "anchors": [anchor(thm_l, "We prove that under Definition")]
                }
            ],
            "edges": [],
            "limitations": [],
            "done": true
        })),
        ant_text("Opened main theorem."),
    ]);

    let (st, started1, raw) = req(
        &s.srv, "POST", &format!("/api/sessions/{}/investigation/tasks", s.sid),
        Some(&json!({ "task": "open", "tags": [s.thm_tag] })),
    );
    assert_eq!(st, 202, "{raw}");
    assert_eq!(started1["cap"], json!(10));
    wait_task_finish(&s.srv, &s.sid, 1);

    let (_, mv1, raw1) = req(&s.srv, "GET", &format!("/api/sessions/{}/investigation/mapview", s.sid), None);
    assert!(!raw1.contains("\"excerpt\""), "mapview must strip raw anchor excerpts");
    let children1: Vec<String> = find_node(&mv1, &s.thm_tag)["children"]
        .as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    assert_eq!(children1.len(), 3);
    let m_step1 = children1[0].clone();
    let m_step2 = children1[1].clone();
    assert!(m_step1.starts_with("m-") && m_step2.starts_with("m-"));
    assert_eq!(find_node(&mv1, &m_step1)["model_node"], json!(true));
    assert_eq!(find_node(&mv1, &m_step1)["attach_to"], json!(s.thm_tag));

    // 2. V10: Save reader notes on both skeleton thm:main and model node m_step1.
    for (tag, text) in [(&s.thm_tag, "Note on main theorem."), (&m_step1, "Note on proof step 1.")] {
        let (st, _, raw) = req(
            &s.srv, "POST", &format!("/api/sessions/{}/investigation/notes", s.sid),
            Some(&json!({ "tag": tag, "node_fp": s.thm_fp, "text": text })),
        );
        assert_eq!(st, 201, "{raw}");
    }

    // 3. Re-run Open on thm:main capped at max_turns=2 (Rule 6.15 -> Paused, NEVER "Partial" label)
    //    with the same first quote for step-1 -> identical m_step1 tag + Rule 6.12 diff + notes intact.
    s.provider.enqueue(vec![
        ant_tool("t_open2", RECORD_MAP_READING, json!({
            "request_seq": 0,
            "source_id": s.source_id,
            "items": [{
                "new": { "id": "rerun-s1", "attach_to": s.thm_tag, "kind": "proof_step" },
                "gist": "Refined differentiation step.",
                "anchors": [anchor(proof_l1, "Differentiating $E(u(t))$ along the flow")]
            }],
            "edges": [],
            "limitations": [],
            "done": false
        })),
        ant_tool("t_open2_cap", READ_MAP_NODE, json!({ "source_id": s.source_id, "node": s.lem_tag })),
    ]);
    let (st, _, raw) = req(
        &s.srv, "POST", &format!("/api/sessions/{}/investigation/tasks", s.sid),
        Some(&json!({ "task": "open", "tags": [s.thm_tag], "max_turns": 2 })),
    );
    assert_eq!(st, 202, "{raw}");
    wait_task_finish(&s.srv, &s.sid, 2);

    let (_, mv2, _) = req(&s.srv, "GET", &format!("/api/sessions/{}/investigation/mapview", s.sid), None);
    assert_eq!(find_node(&mv2, &s.thm_tag)["children"].as_array().unwrap().len(), 3, "re-run must not duplicate m- node");
    let step1_v2 = find_node(&mv2, &m_step1);
    assert_eq!(step1_v2["gist"], json!("Refined differentiation step."));
    assert_eq!(step1_v2["changed_since_last"]["label"], json!("Changed since you last opened this"));
    assert_eq!(step1_v2["notes"][0]["text"], json!("Note on proof step 1."));
    assert_eq!(find_node(&mv2, &s.thm_tag)["notes"][0]["text"], json!("Note on main theorem."));
    let task2 = mv2["tasks"].as_array().unwrap().last().unwrap();
    assert_eq!(task2["run_state"], json!("paused"));
    assert_eq!(task2["run_state_label"], json!("Paused"));
    assert_ne!(task2["run_state_label"], json!("Partial"));

    // 4. Depth-2 Open targeting ONLY m_step1: expands m_step1, rejects sibling m_step2 (Rule 6.4).
    s.provider.enqueue(vec![
        ant_tool("t_open3", RECORD_MAP_READING, json!({
            "request_seq": 0,
            "source_id": s.source_id,
            "items": [
                {
                    "new": { "id": "sub-1a", "attach_to": m_step1, "kind": "substep" },
                    "gist": "Product rule for squared norm derivative.",
                    "anchors": [anchor(proof_l1, "along the flow gives $dE/dt = -2 D(u)$.")]
                },
                {
                    "new": { "id": "sib-bad", "attach_to": m_step2, "kind": "substep" },
                    "gist": "Must be rejected outside m_step1 scope.",
                    "anchors": [anchor(proof_l2, "Gronwall completes the estimate.")]
                }
            ],
            "edges": [],
            "limitations": [],
            "done": true
        })),
        ant_text("Depth-2 step done."),
    ]);
    let (st, _, raw) = req(
        &s.srv, "POST", &format!("/api/sessions/{}/investigation/tasks", s.sid),
        Some(&json!({ "task": "open", "tags": [m_step1] })),
    );
    assert_eq!(st, 202, "{raw}");
    wait_task_finish(&s.srv, &s.sid, 3);

    let (_, mv3, raw3) = req(&s.srv, "GET", &format!("/api/sessions/{}/investigation/mapview", s.sid), None);
    let sub_children = find_node(&mv3, &m_step1)["children"].as_array().unwrap();
    assert_eq!(sub_children.len(), 1);
    let m_sub1a = sub_children[0].as_str().unwrap();
    assert!(m_sub1a.starts_with("m-"));
    assert_eq!(find_node(&mv3, m_sub1a)["attach_to"], json!(m_step1));
    assert_eq!(find_node(&mv3, m_sub1a)["root_skeleton_tag"], json!(s.thm_tag));
    assert_eq!(find_node(&mv3, m_sub1a)["stale"], json!(false));
    assert_eq!(find_node(&mv3, &m_step2)["children"].as_array().unwrap().len(), 0);
    assert!(!raw3.to_ascii_lowercase().contains("verified"));

    // 5. V7/V8/V10 static JS assets are served over HTTP cleanly.
    for asset in [
        "/assets/investigation_badges.js",
        "/assets/investigation_node_model.js",
        "/assets/investigation_notes_ui.js",
    ] {
        let (st, _, raw) = raw_http(s.srv.addr.port(), "GET", asset, &[], b"");
        assert_eq!(st, 200, "asset {asset} missing");
        let body = String::from_utf8_lossy(&raw);
        assert!(!body.is_empty());
    }
}
