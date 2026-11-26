use super::*;
use crate::auth::AuthGuard;
use crate::chatgpt::{offline_test_hooks, ChatGptHub};
use crate::investigation_mapview::build_mapview_json_with_fold;
use crate::investigation_mapview_v7::enrich_mapview_with_v7;
use crate::investigation_skeleton::{pin_source_and_build_skeleton, EVENT_INVESTIGATION_STARTED};
use crate::sse::SseHub;
use crate::DesktopConfig;
use research_tools::{ingest_bytes, IngestRequest, LiteratureEndpoints, ResearchEnv};
use std::sync::{Arc, Mutex};

const TEX: &[u8] = br#"\documentclass{article}
\newtheorem{lemma}{Lemma}[section]
\newtheorem{theorem}[lemma]{Theorem}
\begin{document}
\section{Setup}\label{sec:setup}
\begin{lemma}[Base Bound]\label{lem:base}
Every finite vector satisfies $\|x\| \ge 0$.
\end{lemma}
\section{Main}\label{sec:main}
\begin{theorem}[Main Decay]\label{thm:main}
Under Lemma~\ref{lem:base}, solutions decay exponentially.
\end{theorem}
\section{Digression}\label{sec:dig}
This section is never referenced by the main theorem.
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
    DesktopState::new(cfg, AuthGuard::new(0).unwrap(), Arc::new(SseHub::new()), chatgpt)
}

#[test]
fn reader_pin_and_note_validate_tag_fp_and_recompute_skip_plan() {
    let tmp = tempfile::tempdir().unwrap();
    let state = mk_state(tmp.path());
    let env = ResearchEnv::new(tmp.path().to_path_buf());
    let ledger = Mutex::new(state.open_ledger().unwrap());
    let sid = "inv-notes-1";
    ledger.lock().unwrap().append(sid, EVENT_INVESTIGATION_STARTED, 1, &json!({})).unwrap();
    let src = ingest_bytes(
        &env.sandbox(),
        &ledger,
        &IngestRequest {
            session_id: sid,
            file_name: "paper.tex",
            origin: "https://arxiv.org/e-print/2402.00001v1",
            raw_bytes: TEX,
            source_id_hint: None,
            section_filter: None,
            title: Some("Notes Test"),
        },
    )
    .unwrap();
    drop(ledger);

    let rec = pin_source_and_build_skeleton(&state, sid, &src.source_id, None, None, None).unwrap();
    let thm = rec.skeleton.nodes.iter().find(|n| n.kind == research_tools::skeleton::SkNodeKind::Theorem).unwrap();
    let thm_tag = thm.tag.as_deref().unwrap();

    // Unknown tag rejected (400).
    let err = handle(&state, sid, serde_json::to_vec(&json!({
        "tag": "n-doesnotexist", "pin": "main_result"
    })).unwrap().as_slice()).unwrap_err();
    assert_eq!(err.0, 400);

    // Mismatched node_fp rejected (409).
    let fp_err = handle(&state, sid, serde_json::to_vec(&json!({
        "tag": thm_tag, "node_fp": "deadbeef0000", "pin": "main_result"
    })).unwrap().as_slice()).unwrap_err();
    assert_eq!(fp_err.0, 409);

    // Empty body rejected (400).
    let empty_err = handle(&state, sid, serde_json::to_vec(&json!({
        "tag": thm_tag
    })).unwrap().as_slice()).unwrap_err();
    assert_eq!(empty_err.0, 400);

    // Valid pin accepted (201) and drives V5 skip_plan dimming of Section 3.
    let (st, pin_out) = handle(&state, sid, serde_json::to_vec(&json!({
        "tag": thm_tag, "node_fp": thm.fp, "pin": "main_result"
    })).unwrap().as_slice()).unwrap();
    assert_eq!(st, 201);
    assert_eq!(pin_out["pin"], "main_result");

    // Valid note accepted (201) and preserved alongside pin.
    let (st2, note_out) = handle(&state, sid, serde_json::to_vec(&json!({
        "tag": thm_tag, "text": "Check uniformity of the decay constant."
    })).unwrap().as_slice()).unwrap();
    assert_eq!(st2, 201);
    assert_eq!(note_out["text"], "Check uniformity of the decay constant.");

    // Fold and enrich mapview: theorem is pinned main result, Section 3 dimmed, note visible.
    let events = state.open_ledger().unwrap().read_session(sid).unwrap();
    let folded = fold_map(&rec.skeleton, &events, false);
    let mut mv = build_mapview_json_with_fold(&rec.skeleton, rec.event_seq, &rec.blob_hash, "head", None, Some(&folded));
    enrich_mapview_with_v7(&mut mv, &rec.skeleton, &folded, true);

    assert_eq!(mv["main_result_tags"], json!([thm_tag]));
    assert_eq!(mv["skip_plan"]["empty"], json!(false));
    let nodes = mv["nodes"].as_array().unwrap();
    let thm_mv = nodes.iter().find(|n| n["id"] == thm_tag).unwrap();
    assert_eq!(thm_mv["pinned"], json!(true));
    assert_eq!(thm_mv["pin"], json!("main_result"));
    assert_eq!(thm_mv["notes"][0]["text"], json!("Check uniformity of the decay constant."));
    let dig_mv = nodes.iter().find(|n| n["title"] == "Digression").unwrap();
    assert_eq!(dig_mv["dim"], json!(true));

    // Unpinning via not_main_result restores zero dimming (Rule 6.14).
    let (st3, _) = handle(&state, sid, serde_json::to_vec(&json!({
        "tag": thm_tag, "pin": "not_main_result"
    })).unwrap().as_slice()).unwrap();
    assert_eq!(st3, 201);
    let events2 = state.open_ledger().unwrap().read_session(sid).unwrap();
    let folded2 = fold_map(&rec.skeleton, &events2, false);
    let mut mv2 = build_mapview_json_with_fold(&rec.skeleton, rec.event_seq, &rec.blob_hash, "head2", None, Some(&folded2));
    enrich_mapview_with_v7(&mut mv2, &rec.skeleton, &folded2, true);
    assert_eq!(mv2["main_result_tags"], json!([]));
    assert_eq!(mv2["skip_plan"]["empty"], json!(true));
    assert!(mv2["nodes"].as_array().unwrap().iter().all(|n| n["dim"] == json!(false)));
    // Note stayed intact across pin toggle (Rule 6.12).
    let thm_mv2 = mv2["nodes"].as_array().unwrap().iter().find(|n| n["id"] == thm_tag).unwrap();
    assert_eq!(thm_mv2["notes"].as_array().unwrap().len(), 1);
}
