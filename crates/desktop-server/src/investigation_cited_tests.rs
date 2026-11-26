use super::*;
use crate::auth::AuthGuard;
use crate::chatgpt::{offline_test_hooks, ChatGptHub};
use crate::investigation_skeleton::{
    pin_source_and_build_skeleton, EVENT_INVESTIGATION_STARTED,
};
use crate::sse::SseHub;
use crate::DesktopConfig;
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::library::{FulltextRecord, LibraryIds, PaperFlags, PaperRecord};
use research_tools::net_rate::{FakeRateClock, NetRateGate};
use research_tools::paper_key::PaperKey;
use research_tools::LiteratureEndpoints;
use std::sync::Arc;

const PAPER_A_TEX: &[u8] = concat!(
    "\\documentclass{article}\n",
    "\\newtheorem{theorem}{Theorem}[section]\n",
    "\\begin{document}\n",
    "\\section{Main}\n",
    "\\begin{theorem}\\label{thm:a}\n",
    "By \\cite[Theorem 2.1]{B26}, \\cite[Theorem 99]{B26}, and \\cite[Prop. 1]{BareRef}, the bound holds.\n",
    "\\end{theorem}\n",
    "\\begin{thebibliography}{9}\n",
    "\\bibitem{B26} B. Author, \\emph{Cached Bound}, arXiv:2601.00055v2.\n",
    "\\bibitem{BareRef} C. Unknown, Unpublished manuscript, 1984.\n",
    "\\end{thebibliography}\n",
    "\\end{document}\n",
)
.as_bytes();

const PAPER_B_TEX: &[u8] = concat!(
    "\\documentclass{article}\n",
    "\\newtheorem{theorem}{Theorem}[section]\n",
    "\\newtheorem{lemma}[theorem]{Lemma}\n",
    "\\begin{document}\n",
    "\\section{Setup}\n",
    "\\begin{lemma}\\label{lem:b1}Base norm inequality.\\end{lemma}\n",
    "\\section{Bound}\n",
    "\\begin{theorem}[Spectral Decay]\\label{thm:b21}EXACT_CITED_THM_2_1_SENTINEL.\\end{theorem}\n",
    "\\end{document}\n",
)
.as_bytes();

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
fn cited_lookup_library_hit_unresolved_pick_and_raise_budget() {
    let tmp = tempfile::tempdir().unwrap();
    let state = mk_state(tmp.path());
    let env = research_tools::ResearchEnv::new(tmp.path().to_path_buf());
    let sid = "inv-cited-unit";

    let ledger = Mutex::new(state.open_ledger().unwrap());
    ledger
        .lock()
        .unwrap()
        .append(sid, EVENT_INVESTIGATION_STARTED, 1, &json!({}))
        .unwrap();
    let src_a = ingest_bytes(
        &env.sandbox(),
        &ledger,
        &IngestRequest {
            session_id: sid,
            file_name: "paper_a.tex",
            origin: "upload",
            raw_bytes: PAPER_A_TEX,
            source_id_hint: None,
            section_filter: None,
            title: Some("Paper A"),
        },
    )
    .unwrap();
    drop(ledger);

    let rec_a = pin_source_and_build_skeleton(&state, sid, &src_a.source_id, None, None, None).unwrap();
    let thm_a_tag = rec_a
        .skeleton
        .nodes
        .iter()
        .find(|n| n.kind == research_tools::skeleton::SkNodeKind::Theorem)
        .unwrap()
        .tag
        .clone()
        .unwrap();

    // Pre-populate Paper B in F10 library (zero network needed).
    let ledger_raw = state.open_ledger().unwrap();
    let raw_blob = ledger_raw.put_blob(PAPER_B_TEX).unwrap();
    let lib = Library::open(tmp.path()).unwrap();
    let key_b = PaperKey::from_arxiv("2601.00055").unwrap();
    lib.upsert_paper(&PaperRecord {
        key: key_b.clone(),
        ids: LibraryIds { arxiv: Some("2601.00055".into()), ..Default::default() },
        title: "Cached Bound".into(),
        authors: vec!["B. Author".into()],
        year: Some(2026),
        abstract_text: String::new(),
        bibtex: String::new(),
        flags: PaperFlags::default(),
        first_seen_ms: 1,
        last_seen_ms: 1,
    })
    .unwrap();
    lib.upsert_fulltext(&FulltextRecord {
        key: key_b.clone(),
        version: Some("v2".into()),
        raw_blob: Some(raw_blob),
        format: "tex".into(),
        bytes: PAPER_B_TEX.len() as u64,
        origin: "arxiv_eprint".into(),
        fetched_ms: 1,
        file_path: None,
        file_size: None,
        file_mtime_ms: None,
        file_sha256: None,
    })
    .unwrap();

    let gate = NetRateGate::new(Arc::new(FakeRateClock::new()));

    // 1. Bare citation -> unresolved.
    let body_unres = serde_json::to_vec(&json!({
        "tag": thm_a_tag, "bib_key": "BareRef", "locator": "Prop. 1"
    }))
    .unwrap();
    let (st, val) = handle_with_gate(&state, sid, &body_unres, &gate).unwrap();
    assert_eq!(st, 200);
    assert_eq!(val["status"], CITED_STATUS_UNRESOLVED);

    // 2. Confirmed library lookup -> Found, zero network, populates F10 skeleton cache.
    let body_found = serde_json::to_vec(&json!({
        "tag": thm_a_tag, "bib_key": "B26", "locator": "Theorem 2.1"
    }))
    .unwrap();
    let (st, val) = handle_with_gate(&state, sid, &body_found, &gate).unwrap();
    assert_eq!(st, 200);
    assert_eq!(val["status"], CITED_STATUS_FOUND);
    assert_eq!(val["from_cache"], json!(true));
    assert_eq!(val["skeleton_cache_hit"], json!(false));
    assert!(val["statement"]["text"]
        .as_str()
        .unwrap()
        .contains("EXACT_CITED_THM_2_1_SENTINEL"));
    assert_eq!(
        val["version_caveat"],
        json!("numbering taken from arXiv v2; the journal version may differ")
    );
    assert_eq!(val["return_ref"]["id"], json!(thm_a_tag));
    assert!(!serde_json::to_string(&val).unwrap().to_ascii_lowercase().contains("verified"));

    // 3. Second lookup hits F10 cached skeleton directly.
    let (st2, val2) = handle_with_gate(&state, sid, &body_found, &gate).unwrap();
    assert_eq!(st2, 200);
    assert_eq!(val2["skeleton_cache_hit"], json!(true));

    // 4. Missing locator -> paper_found_locator_not_found + manual pick succeeds.
    let body_miss = serde_json::to_vec(&json!({
        "tag": thm_a_tag, "bib_key": "B26", "locator": "Theorem 99"
    }))
    .unwrap();
    let (st3, val3) = handle_with_gate(&state, sid, &body_miss, &gate).unwrap();
    assert_eq!(st3, 200);
    assert_eq!(val3["status"], "paper_found_locator_not_found");
    let picked_tag = val3["candidates"][0]["tag"].as_str().unwrap().to_string();
    let ext_sid = val3["external_source_id"].as_str().unwrap().to_string();

    let body_pick = serde_json::to_vec(&json!({
        "action": "pick", "tag": thm_a_tag, "bib_key": "B26", "locator": "Theorem 99",
        "external_source_id": ext_sid, "external_tag": picked_tag
    }))
    .unwrap();
    let (st4, val4) = handle_with_gate(&state, sid, &body_pick, &gate).unwrap();
    assert_eq!(st4, 200);
    assert_eq!(val4["status"], CITED_STATUS_FOUND);
    assert_eq!(val4["picked_by_user"], json!(true));

    // 5. Raise fetch budget records investigation.note and updates effective limit.
    let body_raise = serde_json::to_vec(&json!({
        "action": "raise_budget", "new_limit": 16, "tag": thm_a_tag
    }))
    .unwrap();
    let (st5, val5) = handle_with_gate(&state, sid, &body_raise, &gate).unwrap();
    assert_eq!(st5, 200);
    assert_eq!(val5["fetch_budget"]["limit"], json!(16));
    let events = state.open_ledger().unwrap().read_session(sid).unwrap();
    assert_eq!(effective_fetch_limit(&events), 16);
    assert!(!events.iter().any(|e| e.kind == "model.request"));

    // 6. Invalid `new_limit` values must return HTTP 400 (never silently bump).
    for bad_limit in [json!(-5), json!("bad"), json!(0), json!(12.5)] {
        let body_bad = serde_json::to_vec(&json!({
            "action": "raise_budget", "new_limit": bad_limit, "tag": thm_a_tag
        }))
        .unwrap();
        let err = handle_with_gate(&state, sid, &body_bad, &gate).unwrap_err();
        assert_eq!(err.0, 400, "bad new_limit {bad_limit} should be 400");
    }

    // 7. Picking the pinned primary source as an external cited paper must return HTTP 400.
    let body_self_pick = serde_json::to_vec(&json!({
        "action": "pick", "tag": thm_a_tag, "bib_key": "B26", "locator": "Theorem 2.1",
        "external_source_id": src_a.source_id, "external_tag": thm_a_tag
    }))
    .unwrap();
    let err_self = handle_with_gate(&state, sid, &body_self_pick, &gate).unwrap_err();
    assert_eq!(err_self.0, 400, "picking pinned primary source must be rejected");
}
