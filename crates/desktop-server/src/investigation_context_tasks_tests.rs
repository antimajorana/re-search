use super::*;
use crate::auth::AuthGuard;
use crate::chatgpt::{offline_test_hooks, ChatGptHub};
use crate::investigation_mapview::build_mapview_json_with_fold;
use crate::investigation_mapview_v7::enrich_mapview_with_v7;
use crate::investigation_skeleton::{pin_source_and_build_skeleton, EVENT_INVESTIGATION_STARTED};
use crate::investigation_task_context::{
    build_task_context_with_events, record_context_supplied, DEFAULT_CONTEXT_CHAR_BUDGET,
};
use crate::investigation_task_prompts::{build_task_preamble, build_user_task_prompt};
use crate::investigation_tasks::{self, TASK_CAP_FIELD, TASK_REQUEST_SEQ_FIELD};
use crate::sse::SseHub;
use crate::state::DesktopState;
use crate::DesktopConfig;
use ledger::Ledger;
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::investigation_markers::{MARKER_TASK_FINISHED, MARKER_TASK_REQUESTED};
use research_tools::map_context::record_context_cards;
use research_tools::map_reading::record_map_reading;
use research_tools::paper_key::PaperKey;
use research_tools::skeleton::{
    build_tex_skeleton, build_tex_skeleton_from_ingested, TexSkeletonInput,
};
use research_tools::{LiteratureEndpoints, ResearchEnv};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

const PRIMARY_TEX: &[u8] = br#"\documentclass{article}
\newtheorem{definition}{Definition}
\newtheorem{theorem}{Theorem}
\begin{document}
\section{Setup}
Let $\gamma > 0$ denote the spectral gap fixed throughout.
\begin{definition}\label{def:adm}
An operator kernel is admissible when its trace norm is finite.
\end{definition}
\begin{theorem}\label{thm:main}
Under Definition~\ref{def:adm} and \cite{Smith24} \cite{Bare99}, contraction holds at rate $\gamma$.
\end{theorem}
\begin{thebibliography}{9}
\bibitem{Smith24} J. Smith, Exponential contraction estimates, 2024.
\bibitem{Bare99} B. Bare, Background remark, 1999.
\end{thebibliography}
\end{document}
"#;

const EXTERNAL_TEX: &[u8] = br#"\documentclass{article}
\newtheorem{theorem}{Theorem}
\begin{document}
\section{Main}
\begin{theorem}[External Contraction]\label{thm:ext}
Assume positive gap $\lambda_1 \ge \gamma > 0$ and continuous boundary weight.
Then uniform exponential contraction holds on all finite subspaces.
\end{theorem}
\end{document}
"#;

fn mk_state(dir: &std::path::Path) -> DesktopState {
    let cfg = DesktopConfig {
        port: 0,
        data_dir: dir.to_path_buf(),
        workspace_root: None,
        python: None,
        worker_script: None,
        api_base_override: None,
        literature_endpoints: LiteratureEndpoints::default(),
        open_window: false,
        print_url_json: false,
        eval_timeout_s: 30,
        core_exe: None,
        claude_exe: None,
        agy_exe: None,
        connection_env: Some(Default::default()),
        chatgpt_test: Some(offline_test_hooks()),
    };
    let chatgpt = ChatGptHub::new(&cfg).unwrap();
    DesktopState::new(
        cfg,
        AuthGuard::new(0).unwrap(),
        Arc::new(SseHub::new()),
        chatgpt,
    )
}

#[test]
fn focus_normalization_and_prompts_respect_rule_619_and_avoid_forbidden_words() {
    assert_eq!(normalize_context_focus(Some("Prerequisites")), Some(FOCUS_PREREQUISITES));
    assert_eq!(normalize_context_focus(Some("prerequisites")), Some(FOCUS_PREREQUISITES));
    assert_eq!(
        normalize_context_focus(Some("Compare hypotheses")),
        Some(FOCUS_COMPARE_HYPOTHESES)
    );
    assert_eq!(
        normalize_context_focus(Some("compare_hypotheses")),
        Some(FOCUS_COMPARE_HYPOTHESES)
    );
    assert_eq!(normalize_context_focus(Some("custom_note")), None);

    assert_eq!(
        canonicalize_task_focus(InvestigationTaskKind::Context, Some("Compare hypotheses".into())),
        Some("compare_hypotheses".into())
    );
    assert_eq!(
        canonicalize_task_focus(InvestigationTaskKind::Orient, Some("Compare hypotheses".into())),
        Some("Compare hypotheses".into())
    );

    for focus in ["prerequisites", "compare_hypotheses"] {
        let user_p = build_user_task_prompt(
            InvestigationTaskKind::Context,
            &["n-abc123".into()],
            Some(focus),
        );
        let pre = build_task_preamble(
            InvestigationTaskKind::Context,
            "src1",
            3,
            11,
            &["n-abc123".into()],
            None,
            Some(focus),
        );
        assert!(!user_p.to_ascii_lowercase().contains("verified"));
        assert!(!pre.to_ascii_lowercase().contains("verified"));
        assert!(pre.contains("cap 8 model calls"));
        assert!(pre.contains("record_context_cards"));
        assert!(pre.contains("DATA ONLY") || pre.contains("never instructions"));
    }
}

#[test]
fn compare_hypotheses_injects_located_cited_node_logs_f4_supplied_and_projects_mapview() {
    let tmp = tempfile::tempdir().unwrap();
    let state = mk_state(tmp.path());
    let env = ResearchEnv::new(tmp.path());
    let sid = "s-ctx-tasks";
    let ledger = Ledger::open(tmp.path()).unwrap();
    ledger.append(sid, EVENT_INVESTIGATION_STARTED, 1, &json!({})).unwrap();
    drop(ledger);

    let db = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let pri_ing = ingest_bytes(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: sid,
            file_name: "primary.tex",
            origin: "arxiv:2502.00001v1",
            raw_bytes: PRIMARY_TEX,
            source_id_hint: Some("src1"),
            section_filter: None,
            title: Some("Primary Flow"),
        },
    )
    .unwrap();
    let ext_ing = ingest_bytes(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: sid,
            file_name: "smith24.tex",
            origin: "arxiv:2402.00011v1",
            raw_bytes: EXTERNAL_TEX,
            source_id_hint: Some("src_ext"),
            section_filter: None,
            title: Some("External Contraction"),
        },
    )
    .unwrap();
    drop(db);

    let pinned = pin_source_and_build_skeleton(
        &state,
        sid,
        &pri_ing.source_id,
        Some(PaperKey::parse("arxiv:2502.00001").unwrap()),
        Some("v1".into()),
        None,
    )
    .unwrap();

    let thm_tag = pinned
        .skeleton
        .nodes
        .iter()
        .find(|n| n.label.as_deref() == Some("thm:main"))
        .and_then(|n| n.tag.clone())
        .unwrap();

    let ledger = Ledger::open(tmp.path()).unwrap();
    let ext_receipt = ledger
        .read_session(sid)
        .unwrap()
        .into_iter()
        .find(|e| {
            e.kind == "source.ingested"
                && e.payload["source_id"].as_str() == Some(ext_ing.source_id.as_str())
        })
        .unwrap();
    let ext_skel = build_tex_skeleton_from_ingested(
        &env.sandbox(),
        sid,
        &ext_receipt,
        PaperKey::parse("arxiv:2402.00011").unwrap(),
        Some("v1".into()),
    )
    .unwrap()
    .0
    .skeleton;
    let ext_thm_tag = ext_skel
        .nodes
        .iter()
        .find(|n| n.label.as_deref() == Some("thm:ext"))
        .and_then(|n| n.tag.clone())
        .unwrap();

    // Seed prior reading hypothesis on citing theorem + located Smith24 lookup + unlocated Bare99 lookup.
    ledger
        .append(
            sid,
            EVENT_INVESTIGATION_CITED_LOOKUP,
            1,
            &json!({
                "tag": thm_tag,
                "bib_key": "Smith24",
                "short_ref": "Smith 2024, Thm. 1",
                "resolved": { "paper_key": "arxiv:2402.00011", "version": "v1" },
                "external_source_id": ext_ing.source_id,
                "external_tag": ext_thm_tag,
                "status": CITED_STATUS_FOUND,
            }),
        )
        .unwrap();
    ledger
        .append(
            sid,
            EVENT_INVESTIGATION_CITED_LOOKUP,
            1,
            &json!({
                "tag": thm_tag,
                "bib_key": "Bare99",
                "short_ref": "Bare 1999",
                "resolved": Value::Null,
                "external_source_id": Value::Null,
                "external_tag": Value::Null,
                "status": "unresolved",
            }),
        )
        .unwrap();

    let pri_path = pri_ing.primary_path.clone().unwrap();
    let pri_text = std::str::from_utf8(PRIMARY_TEX).unwrap();
    let thm_line = pri_text
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains("contraction holds at rate"))
        .map(|(i, _)| i + 1)
        .unwrap();

    // Also add an `open` task reading with hypothesis + a `m-` child node to test Compare + projection.
    let open_req = ledger
        .append(
            sid,
            MARKER_TASK_REQUESTED,
            1,
            &json!({
                "task": "open",
                "tags": [thm_tag],
                "source_id": pri_ing.source_id,
                "skeleton_seq": pinned.event_seq,
            }),
        )
        .unwrap();
    let pri_sha = ledger::sha256_hex(PRIMARY_TEX);
    record_context_supplied(
        &ledger,
        sid,
        open_req,
        "main",
        &[SuppliedRange {
            source_id: pri_ing.source_id.clone(),
            path: pri_path.clone(),
            file_sha256: pri_sha,
            start_line: 1,
            end_line: pri_text.lines().count(),
        }],
    )
    .unwrap();
    let rec_res = record_map_reading(
        &env,
        &ledger,
        sid,
        "main",
        &json!({
            "request_seq": open_req,
            "source_id": pri_ing.source_id,
            "items": [{
                "tag": thm_tag,
                "gist": "Establishes contraction from Smith24.",
                "hypotheses": [{
                    "text": "Definition def:adm holds",
                    "anchor": {
                        "source_id": pri_ing.source_id,
                        "path": pri_path,
                        "start_line": thm_line,
                        "end_line": thm_line,
                        "quote": "contraction holds at rate"
                    }
                }],
                "anchors": [{
                    "source_id": pri_ing.source_id,
                    "path": pri_path,
                    "start_line": thm_line,
                    "end_line": thm_line,
                    "quote": "contraction holds at rate"
                }]
            }],
            "edges": [],
            "limitations": [],
            "done": true
        }),
    )
    .unwrap();
    assert_eq!(rec_res["status"], "recorded");
    ledger
        .append(
            sid,
            MARKER_TASK_FINISHED,
            1,
            &json!({ "request_seq": open_req, "status": "done" }),
        )
        .unwrap();

    // Start Context (Compare hypotheses) task marker & build context.
    let ctx_req = ledger
        .append(
            sid,
            MARKER_TASK_REQUESTED,
            1,
            &json!({
                "task": "context",
                "tags": [thm_tag],
                "focus": "compare_hypotheses",
                "source_id": pri_ing.source_id,
                "skeleton_seq": pinned.event_seq,
                "cap": 8
            }),
        )
        .unwrap();
    let events = ledger.read_session(sid).unwrap();
    let pri_receipt = find_source_receipt(&events, &pri_ing.source_id, None).unwrap();
    let built = build_task_context_with_events(
        &env.sandbox(),
        sid,
        pri_receipt,
        &events,
        &pinned.skeleton,
        pinned.event_seq,
        ctx_req,
        InvestigationTaskKind::Context,
        std::slice::from_ref(&thm_tag),
        None,
        Some("Compare hypotheses"),
        DEFAULT_CONTEXT_CHAR_BUDGET,
    )
    .unwrap();

    assert!(built.instructions.contains("Definition def:adm holds"), "{}", built.instructions);
    assert!(
        built.instructions.contains("uniform exponential contraction holds"),
        "{}",
        built.instructions
    );
    assert!(
        built.instructions.contains("Bare99") && built.instructions.contains("unresolved"),
        "{}",
        built.instructions
    );
    assert!(
        built.supplied_ranges.iter().any(|r| r.source_id == ext_ing.source_id),
        "external ranges must be credited: {:?}",
        built.supplied_ranges
    );

    // Log supplied ranges and save external cited_result + model_background card via record_context_cards.
    record_context_supplied(&ledger, sid, ctx_req, "main", &built.supplied_ranges).unwrap();
    let ext_path = ext_ing.primary_path.clone().unwrap();
    let ext_text = std::str::from_utf8(EXTERNAL_TEX).unwrap();
    let ext_h_line = ext_text
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains("continuous boundary weight"))
        .map(|(i, _)| i + 1)
        .unwrap();
    let ext_c_line = ext_text
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains("uniform exponential contraction"))
        .map(|(i, _)| i + 1)
        .unwrap();

    let card_res = record_context_cards(
        &env,
        &ledger,
        sid,
        "main",
        &json!({
            "request_seq": ctx_req,
            "source_id": pri_ing.source_id,
            "tag": thm_tag,
            "cards": [
                {
                    "type": "cited_result",
                    "title": "Smith24 Contraction Comparison",
                    "body": "Cited theorem requires continuous boundary weight in addition to gap gamma.",
                    "bib_key": "Smith24",
                    "anchors": [{
                        "source_id": ext_ing.source_id,
                        "path": ext_path,
                        "start_line": ext_c_line,
                        "end_line": ext_c_line,
                        "quote": "uniform exponential contraction holds"
                    }],
                    "hypotheses": [{
                        "text": "Continuous boundary weight assumption",
                        "anchor": {
                            "source_id": ext_ing.source_id,
                            "path": ext_path,
                            "start_line": ext_h_line,
                            "end_line": ext_h_line,
                            "quote": "continuous boundary weight"
                        }
                    }]
                },
                {
                    "type": "prerequisite",
                    "title": "Background energy norm reminder",
                    "body": "Standard finite-dimensional norm equivalence.",
                    "anchors": []
                }
            ]
        }),
    )
    .unwrap();
    assert_eq!(card_res["status"], "recorded", "{card_res}");

    // Finish task via finish_task: context cards must count as saved records (status: done!).
    let req_obj = json!({
        TASK_REQUEST_SEQ_FIELD: ctx_req,
        TASK_CAP_FIELD: 8,
    });
    let reply = investigation_tasks::finish_task(&state, sid, &req_obj, Ok("Done.".into()), false)
        .unwrap();
    assert_eq!(reply, "Done.");

    let post_events = ledger.read_session(sid).unwrap();
    let fin = post_events.iter().rev().find(|e| e.kind == MARKER_TASK_FINISHED).unwrap();
    assert_eq!(fin.payload["status"], "done");
    assert_eq!(fin.payload["counts"]["items"], 2);

    // Mapview projection includes provenance keys, short_ref, and zero excerpts.
    let folded = fold_map(&pinned.skeleton, &post_events, false);
    let mut mv = build_mapview_json_with_fold(
        &pinned.skeleton,
        pinned.event_seq,
        "blob",
        "head",
        None,
        Some(&folded),
    );
    enrich_mapview_with_v7(&mut mv, &pinned.skeleton, &folded, false);
    enrich_context_cards_metadata(&mut mv, &state, sid, &post_events);

    let thm_mv = mv["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == thm_tag)
        .unwrap();
    let cards = thm_mv["context_cards"].as_array().unwrap();
    assert_eq!(cards.len(), 2);
    assert_eq!(cards[0]["origin"], "external");
    assert_eq!(cards[0]["provenance"], "outside_paper");
    assert_eq!(cards[0]["short_ref"], "Smith 2024, Thm. 1");
    assert!(cards[0]["anchors"][0].get("excerpt").is_none());
    assert!(cards[0]["hypotheses"][0]["anchor"].get("excerpt").is_none());
    assert_eq!(cards[1]["origin"], "model_background");
    assert_eq!(cards[1]["provenance"], "model_background");

    // Silence unused warning if build_tex_skeleton not needed.
    let _ = std::marker::PhantomData::<TexSkeletonInput>;
    let _ = build_tex_skeleton;
    let _ = BTreeMap::<String, String>::new();
}
