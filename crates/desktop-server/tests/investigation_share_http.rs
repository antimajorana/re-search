//! Loopback HTTP and auth integration tests for Investigate V15 share export/import routes.

mod common;

use common::{launch_cookie, raw_http, test_cfg};
use desktop_server::start_desktop_server;
use export::paper_map::PaperMapExport;
use ledger::{FulltextCache, Ledger};
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::library::{Library, LibraryIds, PaperFlags, PaperRecord};
use research_tools::map_defs::{
    AuthorsClaimLabel, AuthorsClaimRecord, MapReadingEventPayload, ReadingItemRecord,
    ReadingRejected, ReadingRole, VerifiedMapAnchor,
};
use research_tools::paper_key::PaperKey;
use research_tools::quote_core::verify_quote;
use research_tools::skeleton::SkNodeKind;
use research_tools::ResearchEnv;
use serde_json::{json, Value};
use std::sync::Mutex;

const SAMPLE_TEX: &[u8] = br#"\documentclass{article}
\newtheorem{lemma}{Lemma}[section]
\newtheorem{theorem}[lemma]{Theorem}
\begin{document}
\section{Framework}\label{sec:fw}
\begin{lemma}[Positivity]\label{lem:pos}
For every $x \ne 0$, $Q(x) > 0$.
\end{lemma}
\begin{theorem}[Stability]\label{thm:stab}
Under Lemma~\ref{lem:pos}, trajectories remain globally bounded.
\end{theorem}
\begin{proof}
Our theorem improves prior bounds under strict positivity.
\end{proof}
\end{document}
"#;

fn header_val<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    let lower = name.to_ascii_lowercase();
    headers.iter().find(|(k, _)| k.to_ascii_lowercase() == lower).map(|(_, v)| v.as_str())
}

fn create_investigate_session(port: u16, tok: &str, title: &str) -> String {
    let (st, _, created) = raw_http(
        port,
        "POST",
        "/api/sessions",
        &[("X-ReSearch-Token", tok), ("Content-Type", "application/json")],
        &serde_json::to_vec(&json!({ "mode": "investigate", "title": title })).unwrap(),
    );
    assert_eq!(st, 200, "create session failed: {}", String::from_utf8_lossy(&created));
    serde_json::from_slice::<Value>(&created).unwrap()["id"].as_str().unwrap().to_string()
}

fn seed_mapped_session(data_dir: &std::path::Path, port: u16, tok: &str, licence: Option<&str>) -> String {
    let sid = create_investigate_session(port, tok, "Seed <script>alert(1)</script>");
    let key = PaperKey::from_arxiv("2402.00001").unwrap();
    let lib = Library::open(data_dir).unwrap();
    lib.upsert_paper(&PaperRecord {
        key: key.clone(),
        ids: LibraryIds { arxiv: Some("2402.00001".into()), ..Default::default() },
        title: "Stability <script>alert(1)</script>".to_string(),
        authors: vec!["Ada Lovelace".into()],
        year: Some(2024),
        abstract_text: "Stability note.".into(),
        bibtex: String::new(),
        flags: PaperFlags { license: licence.map(str::to_string), ..Default::default() },
        first_seen_ms: 1,
        last_seen_ms: 1,
    })
    .unwrap();
    FulltextCache::open(data_dir)
        .unwrap()
        .put_bytes(key.as_str(), Some("v2"), "tex", "https://arxiv.org/e-print/2402.00001v2", SAMPLE_TEX)
        .unwrap();

    let env = ResearchEnv::new(data_dir.to_path_buf());
    let ledger_mutex = Mutex::new(Ledger::open_existing(data_dir).unwrap());
    let ing = ingest_bytes(
        &env.sandbox(),
        &ledger_mutex,
        &IngestRequest {
            session_id: &sid,
            file_name: "stability.tex",
            origin: "https://arxiv.org/e-print/2402.00001v2",
            raw_bytes: SAMPLE_TEX,
            source_id_hint: None,
            section_filter: None,
            title: Some("Stability <script>alert(1)</script>"),
        },
    )
    .unwrap();
    drop(ledger_mutex);

    let (pin_st, _, pin_raw) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{sid}/investigation/skeleton"),
        &[("X-ReSearch-Token", tok), ("Content-Type", "application/json")],
        &serde_json::to_vec(&json!({ "source_id": ing.source_id, "pin": true })).unwrap(),
    );
    assert_eq!(pin_st, 200, "pin skeleton failed: {}", String::from_utf8_lossy(&pin_raw));
    let pin_val: Value = serde_json::from_slice(&pin_raw).unwrap();
    let skel_seq = pin_val["seq"].as_u64().unwrap();
    let skel: research_tools::skeleton::PaperSkeleton =
        serde_json::from_value(pin_val["skeleton"].clone()).unwrap();

    let ledger = Ledger::open_existing(data_dir).unwrap();
    let events = ledger.read_session(&sid).unwrap();
    let path = ing.primary_path.as_deref().unwrap();
    let lem = skel.nodes.iter().find(|n| n.kind == SkNodeKind::Lemma).unwrap();
    let thm = skel.nodes.iter().find(|n| n.kind == SkNodeKind::Theorem).unwrap();
    let prf = skel.nodes.iter().find(|n| n.kind == SkNodeKind::Proof).unwrap();

    let lem_quote = "For every $x \\ne 0$, $Q(x) > 0$.";
    let lem_line = lem.spans[0].start_line + 1;
    let vq_lem = verify_quote(&env.sandbox(), &sid, &events, &ing.source_id, path, lem_line, lem_line, lem_quote).unwrap();

    let claim_quote = "Our theorem improves prior bounds under strict positivity.";
    let prf_line = prf.spans[0].start_line + 1;
    let vq_claim = verify_quote(&env.sandbox(), &sid, &events, &ing.source_id, path, prf_line, prf_line, claim_quote).unwrap();

    let to_va = |vq: research_tools::quote_core::VerifiedQuote, raw: &str| VerifiedMapAnchor {
        source_id: vq.quote_ref.source_id, path: vq.quote_ref.path,
        start_line: vq.quote_ref.start_line, end_line: vq.quote_ref.end_line,
        file_sha256: vq.quote_ref.file_sha256, quote_sha256: vq.quote_ref.quote_sha256,
        quote: raw.to_string(), excerpt: vq.excerpt, source_seq: vq.source_seq,
        cited_range: vq.cited_range, range_basis: vq.range_basis, excerpt_window: vq.excerpt_window,
    };

    let payload = MapReadingEventPayload {
        schema_version: 1,
        request_seq: skel_seq,
        source_id: skel.source_id.clone(),
        skeleton_seq: skel_seq,
        done: true,
        items: vec![
            ReadingItemRecord {
                tag: lem.tag.clone().unwrap(), node_fp: lem.fp.clone(), attach_to: None, model_kind: None,
                role: Some(ReadingRole::Supporting), gist: Some("Positive definite quadratic form".into()),
                purpose: None, hypotheses: vec![], anchors: vec![to_va(vq_lem, lem_quote)], authors_claim: None,
            },
            ReadingItemRecord {
                tag: thm.tag.clone().unwrap(), node_fp: thm.fp.clone(), attach_to: None, model_kind: None,
                role: Some(ReadingRole::MainResult), gist: Some("Global boundedness result".into()),
                purpose: None, hypotheses: vec![], anchors: vec![],
                authors_claim: Some(AuthorsClaimRecord {
                    label: AuthorsClaimLabel::New, relation_note: None, attributed_to: vec![],
                    anchors: vec![to_va(vq_claim, claim_quote)], cues: vec!["improves".into()],
                }),
            },
        ],
        edges: vec![],
        limitations: vec![],
        rejected: ReadingRejected::default(),
        anchor_report: json!({ "dropped": [], "corrected": [] }),
    };
    ledger.append(&sid, "investigation.reading", 1, &serde_json::to_value(payload).unwrap()).unwrap();
    ledger.append(&sid, "investigation.note", 1, &json!({
        "source_id": skel.source_id, "skeleton_seq": skel_seq, "tag": thm.tag, "node_fp": thm.fp,
        "text": "Private reader note: re-check constant.",
    })).unwrap();
    sid
}

#[test]
fn investigation_share_http_auth_gates_export_roundtrip_tamper_and_readonly_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let srv = start_desktop_server(test_cfg(dir.path(), None, None, None)).unwrap();
    let port = srv.info.port;
    let tok = srv.info.token.clone();
    let tok_hdr = ("X-ReSearch-Token", tok.as_str());

    let sid = seed_mapped_session(
        dir.path(),
        port,
        &tok,
        Some("http://arxiv.org/licenses/nonexclusive-distrib/1.0/"),
    );

    // 1. Auth & Origin checks: export requires token (not stream cookie); import checks foreign Origin.
    let (unauth_exp, _, _) = raw_http(port, "GET", &format!("/api/sessions/{sid}/investigation/export?format=rsmap"), &[], b"");
    assert_eq!(unauth_exp, 401);
    let cookie = launch_cookie(port, &tok);
    let (cookie_exp, _, _) = raw_http(
        port,
        "GET",
        &format!("/api/sessions/{sid}/investigation/export?format=rsmap"),
        &[("Cookie", cookie.as_str())],
        b"",
    );
    assert_eq!(cookie_exp, 401, "investigation/export must require X-ReSearch-Token, not stream cookie");

    let (bad_origin, _, _) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{sid}/investigation/import"),
        &[tok_hdr, ("Origin", "https://evil.example"), ("Content-Type", "application/json")],
        b"{}",
    );
    assert_eq!(bad_origin, 403);

    // 2. Owner decision: format=md returns 400 pointing to F15 writeup_paper.rs.
    let (md_st, _, md_raw) = raw_http(
        port,
        "GET",
        &format!("/api/sessions/{sid}/investigation/export?format=md"),
        &[tok_hdr],
        b"",
    );
    assert_eq!(md_st, 400);
    let md_msg = String::from_utf8_lossy(&md_raw);
    assert!(md_msg.contains("writeup_paper"), "unexpected format=md message: {md_msg}");

    // 3. Default .rsmap & HTML export strip full statements & private notes under non-open licence.
    let (rs_st, rs_hdrs, rs_bytes) = raw_http(
        port,
        "GET",
        &format!("/api/sessions/{sid}/investigation/export?format=rsmap"),
        &[tok_hdr],
        b"",
    );
    assert_eq!(rs_st, 200);
    assert!(header_val(&rs_hdrs, "Content-Disposition").is_some_and(|v| v.contains(".rsmap")));
    let rs_text = String::from_utf8_lossy(&rs_bytes);
    assert!(!rs_text.contains("Private reader note"));
    assert!(!rs_text.contains("trajectories remain globally bounded"));
    assert!(!rs_text.to_ascii_lowercase().contains("verified"));

    let (ht_st, ht_hdrs, ht_bytes) = raw_http(
        port,
        "GET",
        &format!("/api/sessions/{sid}/investigation/export?format=html&include_statements=1&include_notes=1"),
        &[tok_hdr],
        b"",
    );
    assert_eq!(ht_st, 200);
    assert!(header_val(&ht_hdrs, "Content-Disposition").is_some_and(|v| v.contains(".html")));
    let html = String::from_utf8_lossy(&ht_bytes);
    assert!(!html.to_ascii_lowercase().contains("<script"));
    assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(html.contains("Private reader note"));
    assert!(html.contains("trajectories remain globally bounded"));
    assert!(!html.to_ascii_lowercase().contains("verified"));

    let src_events = Ledger::open_existing(dir.path()).unwrap().read_session(&sid).unwrap();
    let exp_ev = src_events.iter().rfind(|e| e.kind == "investigation.exported").unwrap();
    assert_eq!(exp_ev.payload["format"], "html");
    assert_eq!(exp_ev.payload["sha256"], ledger::sha256_hex(&ht_bytes));

    // 4. Import .rsmap into another Investigate session in same data_dir (local source matched).
    let dst_sid = create_investigate_session(port, &tok, "Import target");
    let (imp_st, _, imp_raw) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{dst_sid}/investigation/import"),
        &[tok_hdr, ("Content-Type", "application/json")],
        &rs_bytes,
    );
    assert_eq!(imp_st, 200, "import failed: {}", String::from_utf8_lossy(&imp_raw));
    let imp_val: Value = serde_json::from_slice(&imp_raw).unwrap();
    assert_eq!(imp_val["source_match"], true);
    assert_eq!(imp_val["items_dropped"], 0);
    assert!(!String::from_utf8_lossy(&imp_raw).to_ascii_lowercase().contains("verified"));

    // 5. Tampered quote is dropped and counted in banner + ledger event.
    let mut tampered: PaperMapExport = serde_json::from_slice(&rs_bytes).unwrap();
    tampered.gists[0].anchors[0].quote = "Forged non-matching quote.".into();
    let tamper_sid = create_investigate_session(port, &tok, "Tamper target");
    let (t_st, _, t_raw) = raw_http(
        port,
        "POST",
        &format!("/api/sessions/{tamper_sid}/investigation/import"),
        &[tok_hdr, ("Content-Type", "application/json")],
        &serde_json::to_vec(&tampered).unwrap(),
    );
    assert_eq!(t_st, 200);
    let t_val: Value = serde_json::from_slice(&t_raw).unwrap();
    assert_eq!(t_val["items_dropped"], 1);
    assert_eq!(t_val["banner"], "Imported, 1 quotes could not be matched");

    // 6. Fresh server without local source falls back to read-only banner "quotes not matched on this machine".
    let empty_dir = tempfile::tempdir().unwrap();
    let srv_ro = start_desktop_server(test_cfg(empty_dir.path(), None, None, None)).unwrap();
    let ro_tok = srv_ro.info.token.clone();
    let ro_sid = create_investigate_session(srv_ro.info.port, &ro_tok, "RO target");
    let (ro_st, _, ro_raw) = raw_http(
        srv_ro.info.port,
        "POST",
        &format!("/api/sessions/{ro_sid}/investigation/import"),
        &[("X-ReSearch-Token", ro_tok.as_str()), ("Content-Type", "application/json")],
        &rs_bytes,
    );
    assert_eq!(ro_st, 200);
    let ro_val: Value = serde_json::from_slice(&ro_raw).unwrap();
    assert_eq!(ro_val["source_match"], false);
    assert_eq!(ro_val["read_only"], true);
    assert_eq!(ro_val["banner"], "quotes not matched on this machine");
    assert!(!String::from_utf8_lossy(&ro_raw).to_ascii_lowercase().contains("verified"));
    let ro_evs = Ledger::open_existing(empty_dir.path()).unwrap().read_session(&ro_sid).unwrap();
    assert!(!ro_evs.iter().any(|e| e.kind == "source.ingested" || e.kind == "investigation.skeleton"));
    assert!(ro_evs.iter().any(|e| e.kind == "investigation.imported" && e.payload["source_match"] == false));
}
