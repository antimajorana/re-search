//! Integration tests for Investigate V16 Cross-Mode Hand-Off (`derive_solve`,
//! `extend_explore`, `prior_work_explore`, read-only child status projection,
//! incoming `target_mode: "investigate"` receiver, and Set R untouched).

mod common;

use desktop_server::auth::AuthGuard;
use desktop_server::chatgpt::ChatGptHub;
use desktop_server::handoff::{handle_create_handoff, handle_get_handoffs};
use desktop_server::investigation_handoff::{
    build_derive_in_solve_request, build_extend_explore_request,
    build_search_wider_explore_request, child_handoff_statuses, handle_build_handoff_packet,
    parse_skeleton_from_event, sanitize_f12_status_label,
};
use desktop_server::investigation_skeleton::{
    handle as handle_skeleton, pin_source_and_build_skeleton,
};
use desktop_server::sse::SseHub;
use desktop_server::state::{DesktopState, SessionMeta};
use export::fork::research_from_paper;
use export::handoff::{ExploreQuestionKind, HandoffAsk, HandoffPacket, HandoffRequest, HandoffStatement};
use research_tools::ingest_bytes;
use research_tools::investigation_markers::{
    EVENT_INVESTIGATION_NOTE, EVENT_INVESTIGATION_READING, MARKER_TASK_REQUESTED,
};
use research_tools::library::{FulltextRecord, Library, LibraryIds, PaperFlags, PaperRecord};
use research_tools::map_fold::fold_map;
use research_tools::paper_key::{Mode, NodeRef, PaperIds, PaperKey, PaperRef};
use research_tools::prior_work_wording::wording::FORBIDDEN_GENERATED_WORDS;
use research_tools::quote_core::{quote_sha256, QuoteRef};
use research_tools::skeleton::SkNodeKind;
use research_tools::solve_model::{TargetForm, TaskKind};
use research_tools::{IngestRequest, WorkspaceSandbox};
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

const TEX_PAPER: &str = r#"\documentclass{article}
\newcommand{\Lap}{\mathcal{L}}
\begin{document}
\begin{thebibliography}{9}
\bibitem{Toy95} A. Toy, Earlier Spectral Notes, arXiv:2101.01234v2.
\end{thebibliography}

\section{Setup}\label{sec:setup}
\begin{definition}[Kernel Operator]\label{def:kernel}
For weight $w > 0$, define the normalized Laplacian $\Lap(w)$ on finite graphs.
\end{definition}

\begin{assumption}[Uniform Connectivity]\label{asm:conn}
Every edge satisfies $w_{uv} \ge \alpha > 0$ under Definition~\ref{def:kernel}.
\end{assumption}

\begin{equation}\label{eq:gap}
\lambda_2(\Lap) = \frac{\alpha}{n^2}.
\end{equation}

\begin{theorem}[Spectral Lower Bound \cite{Toy95}]\label{thm:bound}
Under Definition~\ref{def:kernel}, Assumption~\ref{asm:conn}, and~\eqref{eq:gap}, we prove $\lambda_2(\Lap) \ge \alpha / n^2$.
\end{theorem}

\begin{proof}[Proof of Theorem~\ref{thm:bound}]\label{prf:bound}
Combine Definition~\ref{def:kernel} with~\eqref{eq:gap} directly by discrete integration.
\end{proof}
\end{document}
"#;

const TARGET_TEX: &str = r#"\documentclass{article}
\begin{document}
\section{Earlier Results}\label{sec:earlier}
\begin{theorem}[Toy Bound]\label{thm:toy}
For every finite connected graph, the gap bound $\lambda_2 \ge c / n^2$ holds unconditionally.
\end{theorem}
\end{document}
"#;

fn test_state(dir: &Path) -> DesktopState {
    let cfg = common::test_cfg(dir, None, None, None);
    let chatgpt = ChatGptHub::new(&cfg).expect("chatgpt hub");
    let state = DesktopState::new(cfg, AuthGuard::new(0).unwrap(), Arc::new(SseHub::new()), chatgpt);
    let _ = state.open_ledger().expect("init ledger schema");
    state
}

fn create_investigate_session(
    state: &DesktopState,
    session_id: &str,
    tex: &str,
    paper_key: &str,
    version: Option<&str>,
) -> (String, String, Vec<String>) {
    let sandbox = WorkspaceSandbox::new(&state.cfg.data_dir, None);
    let ledger = Mutex::new(state.open_ledger().expect("open ledger"));
    {
        let g = ledger.lock().unwrap();
        g.append(session_id, "investigation.started", 1, &json!({})).unwrap();
    }
    state
        .save_meta(&SessionMeta {
            id: session_id.into(),
            title: "V16 Paper Investigation".into(),
            pinned_conventions: Some("Use natural units alpha > 0.".into()),
            ..Default::default()
        })
        .unwrap();
    let ing = ingest_bytes(
        &sandbox,
        &ledger,
        &IngestRequest {
            session_id,
            file_name: "paper.tex",
            origin: "upload:paper.tex",
            raw_bytes: tex.as_bytes(),
            source_id_hint: Some("src-paper"),
            section_filter: None,
            title: Some("Spectral Paper"),
        },
    )
    .expect("ingest paper");
    drop(ledger);

    let key = PaperKey::parse(paper_key).expect("paper key");
    let rec = pin_source_and_build_skeleton(
        state,
        session_id,
        &ing.source_id,
        Some(key),
        version.map(str::to_string),
        Some("Check Theorem 1"),
    )
    .expect("pin & build skeleton");

    let tags: Vec<String> = rec
        .skeleton
        .nodes
        .iter()
        .filter_map(|n| n.tag.clone())
        .collect();
    (ing.source_id, rec.blob_hash, tags)
}

fn find_tag(state: &DesktopState, session_id: &str, kind: SkNodeKind) -> String {
    let ledger = state.open_ledger().unwrap();
    let evs = ledger.read_session(session_id).unwrap();
    let sk_ev = evs.iter().rfind(|e| e.kind == "investigation.skeleton").unwrap();
    let rec = parse_skeleton_from_event(&ledger, sk_ev).unwrap();
    rec.nodes
        .into_iter()
        .find(|n| n.kind == kind)
        .and_then(|n| n.tag)
        .expect("matching node kind tag")
}

#[test]
fn v16_set_c_derive_in_solve_packet_closure_symbols_notes_and_full_quote_survival() {
    let tmp = TempDir::new().unwrap();
    let state = test_state(tmp.path());
    let sid = "inv-v16-solve";
    let (source_id, _, _) = create_investigate_session(&state, sid, TEX_PAPER, "arxiv:2401.10001", Some("v1"));

    let thm_tag = find_tag(&state, sid, SkNodeKind::Theorem);
    let eq_tag = find_tag(&state, sid, SkNodeKind::Equation);
    let prf_tag = find_tag(&state, sid, SkNodeKind::Proof);

    // Add Orient-like model gist + authors_claim + reader note onto Theorem 1 + conventions.
    let ledger = state.open_ledger().unwrap();
    let evs = ledger.read_session(sid).unwrap();
    let req_seq = ledger.append(sid, MARKER_TASK_REQUESTED, 1, &json!({"task": "orient", "source_id": source_id})).unwrap();
    let sk_ev = evs.iter().rfind(|e| e.kind == "investigation.skeleton").unwrap();
    let sk = parse_skeleton_from_event(&ledger, sk_ev).unwrap();
    let thm_fp = sk.nodes.iter().find(|n| n.tag.as_deref() == Some(&thm_tag)).unwrap().fp.clone();

    ledger.append(sid, EVENT_INVESTIGATION_READING, 1, &json!({
        "schema_version": 1,
        "request_seq": req_seq,
        "source_id": source_id,
        "skeleton_seq": sk_ev.seq,
        "done": true,
        "anchor_report": {},
        "items": [{
            "tag": thm_tag,
            "node_fp": thm_fp,
            "role": "main_result",
            "gist": "Lower bound holds on connected finite graphs.",
            "authors_claim": { "label": "new", "cues": ["we prove"] }
        }]
    })).unwrap();
    ledger.append(sid, EVENT_INVESTIGATION_NOTE, 1, &json!({
        "source_id": source_id,
        "skeleton_seq": sk_ev.seq,
        "tag": thm_tag,
        "node_fp": thm_fp,
        "text": "Check factor of alpha/n^2 carefully."
    })).unwrap();
    ledger.append(sid, "investigation.conventions", 1, &json!({
        "request_seq": req_seq,
        "items": [{ "term": "\\Lap", "convention": "Unnormalized graph Laplacian" }],
        "done": true
    })).unwrap();
    drop(ledger);

    // Build packets for Theorem -> Prove, Equation -> Calculate, Proof -> CheckDerivation.
    let thm_req = build_derive_in_solve_request(&state, sid, &thm_tag).expect("thm solve req");
    let eq_req = build_derive_in_solve_request(&state, sid, &eq_tag).expect("eq solve req");
    let prf_req = build_derive_in_solve_request(&state, sid, &prf_tag).expect("prf solve req");

    let HandoffAsk::Solve(ref thm_ask) = thm_req.packet.ask else { panic!("expected solve ask"); };
    assert_eq!(thm_ask.task_kind, TaskKind::Prove);
    assert_eq!(thm_ask.target.form, TargetForm::Proof);
    assert!(thm_ask.symbols.iter().any(|s| s.name.contains("\\Lap")), "expected \\Lap symbol");

    let HandoffAsk::Solve(ref eq_ask) = eq_req.packet.ask else { panic!("expected eq solve ask"); };
    assert_eq!(eq_ask.task_kind, TaskKind::Calculate);
    assert_eq!(eq_ask.target.form, TargetForm::ClosedForm);

    let HandoffAsk::Solve(ref prf_ask) = prf_req.packet.ask else { panic!("expected prf solve ask"); };
    assert_eq!(prf_ask.task_kind, TaskKind::CheckDerivation);
    assert_eq!(prf_ask.target.form, TargetForm::PerStepVerdict);

    // Closure includes Definition, Assumption, Equation verbatim with quotes.
    assert!(thm_req.packet.context.len() >= 2, "statement closure should carry used defs/assumptions/equations");
    assert!(!thm_req.packet.statement.quotes.is_empty(), "statement should carry F4 quote refs");
    assert!(thm_req.packet.conventions.contains("natural units") && thm_req.packet.conventions.contains("Unnormalized graph Laplacian"));
    assert_eq!(thm_req.packet.user_notes, vec!["Check factor of alpha/n^2 carefully.".to_string()]);
    assert!(thm_req.packet.model_text.iter().any(|m| m.label.contains("(model)") && m.text.contains("Lower bound")));

    // F7 Rule 2: serialize packet JSON and ensure no badge/tier/authors_claim terms appear in payload fields.
    let serialized = serde_json::to_string(&thm_req.packet).unwrap();
    assert!(!serialized.contains("authors_claim"));
    assert!(!serialized.contains("main_result"));

    // HTTP build-packet endpoint matches direct builder cleanly.
    let req_bytes = serde_json::to_vec(&json!({ "action": "derive_solve", "tag": thm_tag })).unwrap();
    let (st_pkt, body_pkt) = handle_build_handoff_packet(&state, sid, &req_bytes).unwrap();
    assert_eq!(st_pkt, 200);
    assert_eq!(body_pkt["packet"]["target_mode"], json!("solve"));

    // Set C (P-to-Q): execute handoff over unchanged source -> 100% quote survival (quotes_dropped == 0).
    let (st_create, created) = handle_create_handoff(&state, sid, &serde_json::to_vec(&thm_req).unwrap()).unwrap();
    assert_eq!(st_create, 201);
    assert_eq!(created["mode"], json!("solve"));
    assert_eq!(created["receipt"]["quotes_dropped"], json!(0), "Set C P-to-Q must have 100% quote survival");
    assert!(created["receipt"]["quotes_matched"].as_u64().unwrap() >= 1);
}

#[test]
fn v16_extend_and_search_wider_carry_origin_exclusion_and_bib_seeds() {
    let tmp = TempDir::new().unwrap();
    let state = test_state(tmp.path());
    let sid = "inv-v16-explore";
    create_investigate_session(&state, sid, TEX_PAPER, "arxiv:2401.10002", Some("v1"));

    // Seed Library with Toy95 cited in TEX_PAPER (`arXiv:2101.01234`).
    let lib = Library::open(&state.cfg.data_dir).unwrap();
    let toy_key = PaperKey::from_arxiv("2101.01234").unwrap();
    lib.upsert_paper(&PaperRecord {
        key: toy_key.clone(),
        ids: LibraryIds { arxiv: Some("2101.01234".into()), ..Default::default() },
        title: "Earlier Spectral Notes".into(),
        authors: vec!["A. Toy".into()],
        year: Some(2021),
        abstract_text: String::new(),
        bibtex: String::new(),
        flags: PaperFlags::default(),
        first_seen_ms: 10,
        last_seen_ms: 10,
    }).unwrap();

    let thm_tag = find_tag(&state, sid, SkNodeKind::Theorem);

    let ext_req = build_extend_explore_request(&state, sid, &thm_tag).expect("extend explore");
    let prior_req = build_search_wider_explore_request(&state, sid, &thm_tag).expect("search wider");

    for (req, expected_kind) in [
        (&ext_req, ExploreQuestionKind::Extended),
        (&prior_req, ExploreQuestionKind::PriorWork),
    ] {
        assert_eq!(req.packet.target_mode, Mode::Explore);
        let HandoffAsk::Explore(ref ask) = req.packet.ask else { panic!("expected explore ask"); };
        assert_eq!(ask.question_kind, expected_kind);
        assert_eq!(ask.origin_paper.as_ref().map(|p| &p.key), Some(&PaperKey::from_arxiv("2401.10002").unwrap()));
        assert!(ask.seed_papers.iter().any(|p| p.key == toy_key), "resolved bibliography seed should travel");
        assert!(!ask.seed_papers.iter().any(|p| p.key == PaperKey::from_arxiv("2401.10002").unwrap()), "origin must stay excluded from seeds");
    }
    assert!(prior_req.packet.ask.expected_target_mode() == Mode::Explore);
    assert!(serde_json::to_string(&prior_req.packet).unwrap().to_lowercase().contains("earlier"));

    // Execute both handoffs with zero model calls & zero network.
    let (st_e, _) = handle_create_handoff(&state, sid, &serde_json::to_vec(&ext_req).unwrap()).unwrap();
    let (st_p, _) = handle_create_handoff(&state, sid, &serde_json::to_vec(&prior_req).unwrap()).unwrap();
    assert_eq!(st_e, 201);
    assert_eq!(st_p, 201);
}

#[test]
fn v16_child_status_projection_is_readonly_and_enforces_f12_wording() {
    let tmp = TempDir::new().unwrap();
    let state = test_state(tmp.path());
    let sid = "inv-v16-status";
    create_investigate_session(&state, sid, TEX_PAPER, "arxiv:2401.10003", Some("v1"));
    let thm_tag = find_tag(&state, sid, SkNodeKind::Theorem);

    let solve_req = build_derive_in_solve_request(&state, sid, &thm_tag).unwrap();
    let prior_req = build_search_wider_explore_request(&state, sid, &thm_tag).unwrap();
    let (_, solve_created) = handle_create_handoff(&state, sid, &serde_json::to_vec(&solve_req).unwrap()).unwrap();
    let (_, prior_created) = handle_create_handoff(&state, sid, &serde_json::to_vec(&prior_req).unwrap()).unwrap();
    let solve_child = solve_created["session_id"].as_str().unwrap().to_string();
    let prior_child = prior_created["session_id"].as_str().unwrap().to_string();

    let ledger = state.open_ledger().unwrap();
    let parent_before = ledger.read_session(sid).unwrap();
    let parent_len = parent_before.len();
    let parent_head = parent_before.last().unwrap().event_hash.clone();

    // Initial statuses before child finishes.
    let initial = child_handoff_statuses(&ledger, &parent_before).unwrap();
    assert_eq!(initial.len(), 2);
    assert_eq!(initial[0].label, "Opened in Solve: Not run yet");
    assert_eq!(initial[1].label, "Opened in Explore: not finished");

    // Append simulated `solve.done` and `idea.verdict` in child sessions ONLY.
    ledger.append(&solve_child, "solve.done", 1, &json!({ "outcome": "finished" })).unwrap();
    ledger.append(&prior_child, "idea.verdict", 1, &json!({ "verdicts": [{ "status": "judged_partial" }] })).unwrap();

    // Verify F12 wording guard strips any forbidden word ("new", "novel", "verified").
    for w in FORBIDDEN_GENERATED_WORDS {
        let sanitized = sanitize_f12_status_label(&format!("Opened in Explore: {w} claim"));
        assert!(!sanitized.to_ascii_lowercase().split(|c: char| !c.is_ascii_alphanumeric()).any(|t| t == *w));
    }

    // Read via API endpoints and verify parent ledger saw ZERO mutations.
    let (st_ho, ho_json) = handle_get_handoffs(&state, sid).unwrap();
    assert_eq!(st_ho, 200);
    let statuses = ho_json["child_statuses"].as_array().unwrap();
    assert_eq!(statuses[0]["label"], json!("Opened in Solve: Done"));
    assert_eq!(statuses[1]["label"], json!("Opened in Explore: Model judges: partly the same"));
    assert_eq!(statuses[1]["action_kind"], json!("prior_work_explore"));

    let (st_mv, mv_json) = handle_skeleton(&state, "GET", sid, "mapview", &HashMap::new(), &[]).unwrap();
    assert_eq!(st_mv, 200);
    assert_eq!(mv_json["handoff_statuses"].as_array().unwrap().len(), 2);

    let parent_after = ledger.read_session(sid).unwrap();
    assert_eq!(parent_after.len(), parent_len, "Rule 5: status reads must NEVER write to parent ledger");
    assert_eq!(parent_after.last().unwrap().event_hash, parent_head);
}

#[test]
fn v16_incoming_investigate_handoff_library_ingest_drops_tampered_and_notes_innermost_node() {
    let tmp = TempDir::new().unwrap();
    let state = test_state(tmp.path());
    let origin = "explore-origin";
    {
        let ledger = state.open_ledger().unwrap();
        ledger.append(origin, "idea.started", 1, &json!({})).unwrap();
    }
    state.save_meta(&SessionMeta { id: origin.into(), title: "Explore Origin".into(), ..Default::default() }).unwrap();

    // Store TARGET_TEX in Library only (NOT in origin session) with cached full-text bytes.
    let lib_key = PaperKey::from_arxiv("2001.55555").unwrap();
    let lib = Library::open(&state.cfg.data_dir).unwrap();
    lib.upsert_paper(&PaperRecord {
        key: lib_key.clone(),
        ids: LibraryIds { arxiv: Some("2001.55555".into()), ..Default::default() },
        title: "Toy Bound Paper".into(),
        authors: vec!["A. Toy".into()],
        year: Some(2020),
        abstract_text: String::new(),
        bibtex: String::new(),
        flags: PaperFlags::default(),
        first_seen_ms: 1,
        last_seen_ms: 1,
    }).unwrap();
    let ledger_blob = state.open_ledger().unwrap();
    let blob_sha = ledger_blob.put_blob(TARGET_TEX.as_bytes()).unwrap();
    lib.upsert_fulltext(&FulltextRecord {
        key: lib_key.clone(),
        version: Some("v1".into()),
        raw_blob: Some(blob_sha),
        format: "tex".into(),
        bytes: TARGET_TEX.len() as u64,
        origin: "library:arxiv:2001.55555".into(),
        fetched_ms: 1,
        file_path: None,
        file_size: None,
        file_mtime_ms: None,
        file_sha256: None,
    }).unwrap();

    // Build 1 valid quote on line 5 (`For every finite connected graph...` inside Theorem) + 1 tampered quote.
    let valid_quote_str = r"For every finite connected graph, the gap bound $\lambda_2 \ge c / n^2$ holds unconditionally.";
    let valid_qref = QuoteRef {
        source_id: "library-ref".into(),
        path: "library-ref/paper.tex".into(),
        start_line: 5,
        end_line: 5,
        file_sha256: String::new(),
        quote_sha256: quote_sha256(valid_quote_str),
        evidence_seq: None,
    };
    let tampered_qref = QuoteRef {
        source_id: "library-ref".into(),
        path: "library-ref/paper.tex".into(),
        start_line: 5,
        end_line: 5,
        file_sha256: String::new(),
        quote_sha256: quote_sha256("Tampered statement that does not exist in library paper."),
        evidence_seq: None,
    };

    let packet = HandoffPacket {
        from: NodeRef { session: origin.into(), mode: Mode::Explore, id: "c-toy".into(), paper: None, node_fp: None },
        target_mode: Mode::Investigate,
        title: "Inspect Toy Bound".into(),
        statement: HandoffStatement { text: valid_quote_str.into(), quotes: vec![] },
        context: vec![],
        papers: vec![PaperRef { key: lib_key.clone(), version: Some("v1".into()), ids: PaperIds { arxiv: Some("2001.55555".into()), ..Default::default() } }],
        conventions: String::new(),
        model_text: vec![],
        user_notes: vec![],
        ask: HandoffAsk::Investigate(export::handoff::InvestigateHandoffAsk {
            paper: PaperRef { key: lib_key, version: Some("v1".into()), ids: PaperIds { arxiv: Some("2001.55555".into()), ..Default::default() } },
            focus_claims: vec!["Toy Bound theorem".into()],
            earlier_quotes: vec![valid_qref.clone(), tampered_qref.clone()],
        }),
    };
    let mut quote_texts = std::collections::BTreeMap::new();
    quote_texts.insert(valid_qref.quote_sha256.clone(), valid_quote_str.into());
    quote_texts.insert(tampered_qref.quote_sha256.clone(), "Tampered statement that does not exist in library paper.".into());

    let req = HandoffRequest { packet, quote_texts, extra_source_ids: vec![] };
    let (st, res) = handle_create_handoff(&state, origin, &serde_json::to_vec(&req).unwrap()).unwrap();
    assert_eq!(st, 201);
    assert_eq!(res["mode"], json!("investigation"));
    assert_eq!(res["receipt"]["quotes_matched"], json!(1));
    assert_eq!(res["receipt"]["quotes_dropped"], json!(1));
    assert_eq!(res["investigate"]["status"], json!("skeleton_ready"));
    assert_eq!(res["investigate"]["source_origin"], json!("library"));
    assert_eq!(res["investigate"]["attached_quote_notes"], json!(1));

    let child_id = res["session_id"].as_str().unwrap();
    let ledger = state.open_ledger().unwrap();
    let child_events = ledger.read_session(child_id).unwrap();
    let sk_ev = child_events.iter().rfind(|e| e.kind == "investigation.skeleton").unwrap();
    let sk = parse_skeleton_from_event(&ledger, sk_ev).unwrap();
    let folded = fold_map(&sk, &child_events, false);
    let thm_tag = sk.nodes.iter().find(|n| n.kind == SkNodeKind::Theorem).unwrap().tag.clone().unwrap();
    let sec_tag = sk.nodes.iter().find(|n| n.kind == SkNodeKind::Section).unwrap().tag.clone().unwrap();

    // Innermost enclosing node (Theorem, NOT outer Section) must receive the quote note.
    assert!(!folded.nodes.get(&thm_tag).unwrap().notes.is_empty(), "Theorem node should hold attached quote note");
    assert!(folded.nodes.get(&sec_tag).is_none_or(|n| n.notes.is_empty()), "Outer Section must not hold inner quote note");

    // Cache-miss paper in offline mode returns clean needs_file without crashing or network calls.
    let missing_key = PaperKey::from_arxiv("2412.99999").unwrap();
    let missing_req = HandoffRequest {
        packet: HandoffPacket {
            from: NodeRef { session: origin.into(), mode: Mode::Explore, id: "c-miss".into(), paper: None, node_fp: None },
            target_mode: Mode::Investigate,
            title: "Missing paper".into(),
            statement: HandoffStatement { text: "Check missing paper.".into(), quotes: vec![] },
            context: vec![],
            papers: vec![],
            conventions: String::new(),
            model_text: vec![],
            user_notes: vec![],
            ask: HandoffAsk::Investigate(export::handoff::InvestigateHandoffAsk {
                paper: PaperRef { key: missing_key, version: None, ids: PaperIds::default() },
                focus_claims: vec![],
                earlier_quotes: vec![],
            }),
        },
        quote_texts: Default::default(),
        extra_source_ids: vec![],
    };
    let (st_m, res_m) = handle_create_handoff(&state, origin, &serde_json::to_vec(&missing_req).unwrap()).unwrap();
    assert_eq!(st_m, 201);
    assert_eq!(res_m["investigate"]["status"], json!("needs_file"));

    // Verify legacy Set R (`research_from_paper`) stays completely untouched.
    let leg_inv = "inv-set-r";
    let (leg_src_id, _, _) = create_investigate_session(&state, leg_inv, TEX_PAPER, "arxiv:2401.10009", None);
    let map_seq = ledger.append(leg_inv, "investigation.map", 1, &json!({
        "source_id": leg_src_id,
        "title": "Leg Map",
        "coverage": "paper",
        "limitations": [],
        "nodes": [],
        "edges": [],
    })).unwrap();
    let map_hash = ledger.read_session(leg_inv).unwrap().last().unwrap().event_hash.clone();
    let set_r_id = research_from_paper(
        tmp.path(),
        leg_inv,
        &json!({ "map_seq": map_seq, "map_hash": map_hash, "prompt": "Explore freely" }),
    ).unwrap();
    let set_r_evs = ledger.read_session(&set_r_id).unwrap();
    assert!(set_r_evs.iter().any(|e| e.kind == "investigation.handoff"));
}
