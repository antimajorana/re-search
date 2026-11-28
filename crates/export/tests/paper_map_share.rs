//! Integration tests for Investigate `.rsmap` + standalone HTML export and fail-closed import (V15).

use export::paper_map::{
    build_paper_map_export, is_open_licence, serialize_paper_map_json, PaperMapExport,
    PaperMapExportOptions, RSMAP_SCHEMA_VERSION,
};
use export::paper_map_html::render_paper_map_html;
use export::paper_map_import::{import_paper_map, READ_ONLY_QUOTES_BANNER};
use ledger::{sha256_hex, FulltextCache, Ledger};
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::investigation_markers::{EVENT_INVESTIGATION_NOTE, EVENT_INVESTIGATION_READING, EVENT_INVESTIGATION_SKELETON};
use research_tools::library::{Library, LibraryIds, PaperFlags, PaperRecord};
use research_tools::map_defs::{
    AuthorsClaimLabel, AuthorsClaimRecord, MapReadingEventPayload, ModelEdgeBasis, ModelEdgeRecord,
    ModelEdgeRole, ReadingItemRecord, ReadingRejected, ReadingRole, VerifiedMapAnchor,
};
use research_tools::map_fold::fold_map;
use research_tools::paper_key::PaperKey;
use research_tools::quote_core::verify_quote;
use research_tools::skeleton::{build_tex_skeleton_from_ingested, PaperSkeleton, SkNodeKind};
use research_tools::ResearchEnv;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Mutex;

const SAMPLE_TEX: &[u8] = br#"\documentclass{article}
\newtheorem{lemma}{Lemma}[section]
\newtheorem{theorem}[lemma]{Theorem}
\begin{document}
\section{Main Framework}\label{sec:fw}
We establish new bounds on positive trajectories \cite{Smith2020}.
\begin{lemma}[Positivity]\label{lem:pos}
For every nonzero state vector, the energy functional remains strictly positive along all admissible smooth trajectories in the domain.
The proof is elementary and self-contained and spans more than three hundred characters when padded cleanly across multiple lines of source text so we can test strict quote clipping inside both rsmap and standalone HTML exports cleanly.
\end{lemma}
\begin{theorem}[Stability]\label{thm:stab}
Under Lemma~\ref{lem:pos}, trajectories remain globally bounded in norm.
\end{theorem}
\begin{proof}
Our main theorem improves \cite{Smith2020} by avoiding smallness assumptions cleanly.
\end{proof}
\begin{thebibliography}{9}
\bibitem{Smith2020} J. Smith, Bounded Flows, 2020.
\end{thebibliography}
\end{document}
"#;

fn setup_investigate_session(
    data_dir: &Path,
    session_id: &str,
    licence: Option<&str>,
    include_reading_and_note: bool,
) -> (String, PaperSkeleton) {
    let ledger_mutex = Mutex::new(Ledger::open(data_dir).unwrap());
    ledger_mutex
        .lock()
        .unwrap()
        .append(session_id, "investigation.started", 1, &json!({}))
        .unwrap();

    let key = PaperKey::from_arxiv("2402.00001").unwrap();
    let lib = Library::open(data_dir).unwrap();
    lib.upsert_paper(&PaperRecord {
        key: key.clone(),
        ids: LibraryIds { arxiv: Some("2402.00001".into()), ..Default::default() },
        title: "Stability <script>alert(1)</script> Note".to_string(),
        authors: vec!["Ada Lovelace".into()],
        year: Some(2024),
        abstract_text: "Stability analysis.".into(),
        bibtex: String::new(),
        flags: PaperFlags { license: licence.map(str::to_string), ..Default::default() },
        first_seen_ms: 1,
        last_seen_ms: 1,
    })
    .unwrap();

    let ft = FulltextCache::open(data_dir).unwrap();
    ft.put_bytes(
        key.as_str(),
        Some("v2"),
        "tex",
        "https://arxiv.org/e-print/2402.00001v2",
        SAMPLE_TEX,
    )
    .unwrap();

    let env = ResearchEnv::new(data_dir.to_path_buf());
    let ing = ingest_bytes(
        &env.sandbox(),
        &ledger_mutex,
        &IngestRequest {
            session_id,
            file_name: "stability.tex",
            origin: "https://arxiv.org/e-print/2402.00001v2",
            raw_bytes: SAMPLE_TEX,
            source_id_hint: None,
            section_filter: None,
            title: Some("Stability <script>alert(1)</script> Note"),
        },
    )
    .unwrap();

    let ledger = ledger_mutex.into_inner().unwrap();
    let events = ledger.read_session(session_id).unwrap();
    let receipt = events
        .iter()
        .find(|e| e.kind == "source.ingested" && e.payload["source_id"].as_str() == Some(ing.source_id.as_str()))
        .unwrap()
        .clone();
    ledger
        .append(
            session_id,
            "investigation.paper",
            1,
            &json!({ "source_id": ing.source_id, "question": "Does positivity imply global stability?" }),
        )
        .unwrap();

    let (out, _) = build_tex_skeleton_from_ingested(
        &env.sandbox(),
        session_id,
        &receipt,
        key,
        Some("v2".into()),
    )
    .unwrap();
    let skel = out.skeleton;
    let blob = ledger.put_blob(&serde_json::to_vec(&skel).unwrap()).unwrap();
    let skel_seq = ledger
        .append(
            session_id,
            EVENT_INVESTIGATION_SKELETON,
            1,
            &json!({
                "source_id": skel.source_id,
                "source_seq": skel.source_seq,
                "builder": skel.builder,
                "files_root_sha256": skel.files_root_sha256,
                "paper_key": skel.paper_key,
                "version": skel.version,
                "quality": skel.quality,
                "quality_reason": skel.quality_reason,
                "blob": blob,
                "raw_blob": blob,
            }),
        )
        .unwrap();

    if include_reading_and_note {
        let events = ledger.read_session(session_id).unwrap();
        let path = ing.primary_path.as_deref().unwrap();
        let lem = skel.nodes.iter().find(|n| n.kind == SkNodeKind::Lemma).unwrap();
        let thm = skel.nodes.iter().find(|n| n.kind == SkNodeKind::Theorem).unwrap();
        let prf = skel.nodes.iter().find(|n| n.kind == SkNodeKind::Proof).unwrap();

        let lem_quote = "For every nonzero state vector, the energy functional remains strictly positive along all admissible smooth trajectories in the domain.";
        let lem_line = lem.spans[0].start_line + 1;
        let vq_lem = verify_quote(&env.sandbox(), session_id, &events, &ing.source_id, path, lem_line, lem_line, lem_quote).unwrap();

        let claim_quote = "Our main theorem improves \\cite{Smith2020} by avoiding smallness assumptions cleanly.";
        let prf_line = prf.spans[0].start_line + 1;
        let vq_claim = verify_quote(&env.sandbox(), session_id, &events, &ing.source_id, path, prf_line, prf_line, claim_quote).unwrap();

        let to_va = |vq: research_tools::quote_core::VerifiedQuote, raw: &str| VerifiedMapAnchor {
            source_id: vq.quote_ref.source_id,
            path: vq.quote_ref.path,
            start_line: vq.quote_ref.start_line,
            end_line: vq.quote_ref.end_line,
            file_sha256: vq.quote_ref.file_sha256,
            quote_sha256: vq.quote_ref.quote_sha256,
            quote: raw.to_string(),
            excerpt: vq.excerpt,
            source_seq: vq.source_seq,
            cited_range: vq.cited_range,
            range_basis: vq.range_basis,
            excerpt_window: vq.excerpt_window,
        };

        let payload = MapReadingEventPayload {
            schema_version: 1,
            request_seq: skel_seq,
            source_id: skel.source_id.clone(),
            skeleton_seq: skel_seq,
            done: true,
            items: vec![
                ReadingItemRecord {
                    tag: lem.tag.clone().unwrap(),
                    node_fp: lem.fp.clone(),
                    attach_to: None,
                    model_kind: None,
                    role: Some(ReadingRole::Supporting),
                    gist: Some("Energy stays positive <img src=x onerror=alert(1)>".into()),
                    purpose: Some("Provides coercivity".into()),
                    hypotheses: vec![],
                    anchors: vec![to_va(vq_lem, lem_quote)],
                    authors_claim: None,
                },
                ReadingItemRecord {
                    tag: thm.tag.clone().unwrap(),
                    node_fp: thm.fp.clone(),
                    attach_to: None,
                    model_kind: None,
                    role: Some(ReadingRole::MainResult),
                    gist: Some("Global boundedness theorem".into()),
                    purpose: None,
                    hypotheses: vec![],
                    anchors: vec![],
                    authors_claim: Some(AuthorsClaimRecord {
                        label: AuthorsClaimLabel::New,
                        relation_note: Some("Avoids smallness".into()),
                        attributed_to: vec!["Smith2020".into()],
                        anchors: vec![to_va(vq_claim.clone(), claim_quote)],
                        cues: vec!["improves".into()],
                    }),
                },
            ],
            edges: vec![ModelEdgeRecord {
                from: lem.tag.clone().unwrap(),
                to: thm.tag.clone().unwrap(),
                role: ModelEdgeRole::Statement,
                basis: ModelEdgeBasis::Explicit,
                reason: "Theorem invokes Positivity lemma.".into(),
                anchor: to_va(vq_claim, claim_quote),
            }],
            limitations: vec![],
            rejected: ReadingRejected::default(),
            anchor_report: json!({ "dropped": [], "corrected": [] }),
        };
        ledger
            .append(session_id, EVENT_INVESTIGATION_READING, 1, &serde_json::to_value(payload).unwrap())
            .unwrap();
        ledger
            .append(
                session_id,
                EVENT_INVESTIGATION_NOTE,
                1,
                &json!({
                    "source_id": skel.source_id,
                    "skeleton_seq": skel_seq,
                    "tag": thm.tag,
                    "node_fp": thm.fp,
                    "text": "Private draft note: double-check boundary term.",
                }),
            )
            .unwrap();
    }

    (ing.source_id, skel)
}

#[test]
fn export_enforces_copyright_privacy_quotes_clipping_and_xss_safety() {
    assert!(!is_open_licence(None));
    assert!(!is_open_licence(Some("http://arxiv.org/licenses/nonexclusive-distrib/1.0/")));
    assert!(is_open_licence(Some("https://creativecommons.org/licenses/by/4.0/")));
    assert!(is_open_licence(Some("CC0-1.0")));

    let dir = tempfile::tempdir().unwrap();
    setup_investigate_session(
        dir.path(),
        "sess-src",
        Some("http://arxiv.org/licenses/nonexclusive-distrib/1.0/"),
        true,
    );

    // Default export under non-open licence excludes statements AND private notes.
    let exp_closed = build_paper_map_export(dir.path(), "sess-src", PaperMapExportOptions::default()).unwrap();
    assert_eq!(exp_closed.schema_version, RSMAP_SCHEMA_VERSION);
    assert!(!exp_closed.open_licence);
    assert!(!exp_closed.statements_included);
    assert!(!exp_closed.notes_included);
    assert!(exp_closed.notes.is_none());
    assert!(exp_closed.context_cards.is_empty());
    assert!(exp_closed.skeleton.nodes.iter().all(|n| n.statement.is_none()));
    for g in &exp_closed.gists {
        for a in &g.anchors {
            assert!(a.quote.chars().count() <= 300, "quote exceeded 300 chars: {}", a.quote.len());
        }
    }

    let json_text = String::from_utf8(serialize_paper_map_json(&exp_closed).unwrap()).unwrap();
    assert!(!json_text.to_ascii_lowercase().contains("verified"));
    assert!(!json_text.contains("Private draft note"));
    assert!(!json_text.contains("trajectories remain globally bounded"));
    assert!(json_text.contains("\"context_cards\": []"));

    let html_closed = render_paper_map_html(&exp_closed);
    assert!(!html_closed.to_ascii_lowercase().contains("<script"));
    assert!(!html_closed.to_ascii_lowercase().contains("verified"));
    assert!(!html_closed.contains("https://fonts."));
    assert!(html_closed.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(html_closed.contains("&lt;img src=x onerror=alert(1)&gt;"));
    assert!(html_closed.contains("(model)"));
    assert!(html_closed.contains("border: 1.5px dashed"));
    assert!(html_closed.contains("border-left: 4px solid"));
    assert!(!html_closed.contains("Private draft note"));
    assert!(!html_closed.contains("trajectories remain globally bounded"));

    // Verify HTML renderer strictly clips quotes longer than 300 chars if fed an unclipped anchor.
    let mut long_exp = exp_closed.clone();
    long_exp.gists[0].anchors[0].quote = "q".repeat(360);
    let long_html = render_paper_map_html(&long_exp);
    assert!(long_html.contains(&"q".repeat(300)));
    assert!(!long_html.contains(&"q".repeat(301)));

    // Opt-in statements + notes includes both cleanly.
    let exp_open_opt = build_paper_map_export(
        dir.path(),
        "sess-src",
        PaperMapExportOptions { include_full_statements: true, include_notes: true },
    )
    .unwrap();
    assert!(exp_open_opt.statements_included);
    assert!(exp_open_opt.notes_included);
    assert_eq!(exp_open_opt.notes.as_ref().unwrap().len(), 1);
    assert!(exp_open_opt.skeleton.nodes.iter().any(|n| {
        n.statement.as_deref().is_some_and(|s| s.contains("trajectories remain globally bounded"))
    }));
    let html_opt = render_paper_map_html(&exp_open_opt);
    assert!(html_opt.contains("trajectories remain globally bounded"));
    assert!(html_opt.contains("Private draft note"));
    assert!(!html_opt.to_ascii_lowercase().contains("verified"));

    // Open CC-BY licence automatically allows statements even without opt-in flag.
    let open_dir = tempfile::tempdir().unwrap();
    setup_investigate_session(
        open_dir.path(),
        "sess-ccby",
        Some("https://creativecommons.org/licenses/by/4.0/"),
        false,
    );
    let exp_ccby = build_paper_map_export(open_dir.path(), "sess-ccby", PaperMapExportOptions::default()).unwrap();
    assert!(exp_ccby.open_licence);
    assert!(exp_ccby.statements_included);
    assert!(exp_ccby.skeleton.nodes.iter().any(|n| n.statement.is_some()));
}

#[test]
fn import_roundtrip_revalidates_drops_tampered_keeps_zero_orphan_blobs_and_handles_missing_source() {
    let src_dir = tempfile::tempdir().unwrap();
    setup_investigate_session(src_dir.path(), "sess-src", Some("CC-BY-4.0"), true);
    let exported = build_paper_map_export(
        src_dir.path(),
        "sess-src",
        PaperMapExportOptions { include_full_statements: true, include_notes: true },
    )
    .unwrap();
    let rsmap_bytes = serialize_paper_map_json(&exported).unwrap();

    // Case A: Destination machine HAS local cached arXiv:2402.00001 v2 in FulltextCache.
    let dst_dir = tempfile::tempdir().unwrap();
    let dst_ft = FulltextCache::open(dst_dir.path()).unwrap();
    dst_ft
        .put_bytes(
            "arxiv:2402.00001",
            Some("v2"),
            "tex",
            "https://arxiv.org/e-print/2402.00001v2",
            SAMPLE_TEX,
        )
        .unwrap();
    Ledger::open(dst_dir.path())
        .unwrap()
        .append("sess-dst", "investigation.started", 1, &json!({}))
        .unwrap();

    let rep = import_paper_map(dst_dir.path(), "sess-dst", &rsmap_bytes).unwrap();
    assert!(rep.source_match);
    assert!(!rep.read_only);
    assert_eq!(rep.sha256, sha256_hex(&rsmap_bytes));
    assert_eq!(rep.items_dropped, 0);
    assert!(rep.items_matched >= 5);
    assert!(!rep.banner.to_ascii_lowercase().contains("verified"));

    let verify_rep = Ledger::open_existing(dst_dir.path()).unwrap().verify_all().unwrap();
    assert_eq!(verify_rep.unreferenced_blobs, 0, "import must leave zero unreferenced blobs");

    let ledger = Ledger::open_existing(dst_dir.path()).unwrap();
    let dst_events = ledger.read_session("sess-dst").unwrap();
    let reading_ev = dst_events.iter().find(|e| e.kind == EVENT_INVESTIGATION_READING).unwrap();
    assert_eq!(reading_ev.payload["imported"], json!(true));
    let folded = fold_map(
        &serde_json::from_slice(&ledger.get_blob(rep.skeleton_blob.as_deref().unwrap()).unwrap()).unwrap(),
        &dst_events,
        false,
    );
    assert_eq!(folded.edges.len(), 1);

    // Case B: Tampered quote in .rsmap is dropped cleanly while valid siblings survive.
    let mut tampered: PaperMapExport = serde_json::from_slice(&rsmap_bytes).unwrap();
    tampered.gists[0].anchors[0].quote = "This forged quote does not appear in the local TeX source.".into();
    let tampered_bytes = serde_json::to_vec(&tampered).unwrap();
    Ledger::open(dst_dir.path())
        .unwrap()
        .append("sess-tamper", "investigation.started", 1, &json!({}))
        .unwrap();
    let rep_tamper = import_paper_map(dst_dir.path(), "sess-tamper", &tampered_bytes).unwrap();
    assert!(rep_tamper.source_match);
    assert_eq!(rep_tamper.items_dropped, 1);
    assert_eq!(rep_tamper.banner, "Imported, 1 quotes could not be matched");
    assert_eq!(Ledger::open_existing(dst_dir.path()).unwrap().verify_all().unwrap().unreferenced_blobs, 0);

    // Case C: Missing local source -> read-only banner "quotes not matched on this machine" and zero ingest.
    let empty_dir = tempfile::tempdir().unwrap();
    Ledger::open(empty_dir.path())
        .unwrap()
        .append("sess-ro", "investigation.started", 1, &json!({}))
        .unwrap();
    let rep_ro = import_paper_map(empty_dir.path(), "sess-ro", &rsmap_bytes).unwrap();
    assert!(!rep_ro.source_match);
    assert!(rep_ro.read_only);
    assert_eq!(rep_ro.banner, READ_ONLY_QUOTES_BANNER);
    assert_eq!(rep_ro.banner, "quotes not matched on this machine");
    let ro_events = Ledger::open_existing(empty_dir.path()).unwrap().read_session("sess-ro").unwrap();
    assert!(!ro_events.iter().any(|e| e.kind == "source.ingested" || e.kind == EVENT_INVESTIGATION_SKELETON));
    let raw_v: Value = serde_json::to_value(&rep_ro).unwrap();
    assert!(!raw_v.to_string().to_ascii_lowercase().contains("verified"));
}
