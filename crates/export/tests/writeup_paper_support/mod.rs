//! Synthetic self-authored TeX and PDF fixtures for F15 Investigate write-up tests (Set W).
//! Zero third-party paper text, zero model calls, zero network calls.

use ledger::Ledger;
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::investigation_markers::{
    EVENT_INVESTIGATION_CONTEXT, EVENT_INVESTIGATION_NOTE, EVENT_INVESTIGATION_OVERRIDE,
    EVENT_INVESTIGATION_READING, EVENT_INVESTIGATION_SKELETON, MARKER_TASK_FINISHED,
    MARKER_TASK_REQUESTED,
};
use research_tools::library::{
    Library, LibraryIds, PaperFlags, PaperRecord, SourceItemRecord, SourceItemState, SourceKind,
    SourceRecord,
};
use research_tools::map_context_defs::{
    ContextCardOrigin, ContextCardRecord, ContextCardType, MapContextEventPayload,
    CONTEXT_SCHEMA_VERSION,
};
use research_tools::map_defs::{
    AuthorsClaimLabel, AuthorsClaimRecord, HypothesisRecord, MapReadingEventPayload,
    ReadingItemRecord, ReadingRejected, ReadingRole, VerifiedMapAnchor,
};
use research_tools::paper_key::{PaperIds, PaperKey};
use research_tools::pdf_pages::{PageBasis, PageMap, PageQualityMetrics, PageRecord};
use research_tools::quote_core::verify_quote;
use research_tools::sandbox::WorkspaceSandbox;
use research_tools::skeleton::{
    build_pdf_skeleton, build_tex_skeleton, PaperSkeleton, PdfSkeletonInput, SkNodeKind,
    TexSkeletonInput,
};
use research_tools::tool_profile::MARKER_INVESTIGATION_STARTED;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

/// Self-authored synthetic TeX paper with a short lemma and a >360-char theorem statement
/// so both the 300-char quote cap and permissive CC-BY full-statement rendering are tested.
pub const LONG_THEOREM_BODY: &str = "For every finite admissible chain of even length at least four with strictly positive boundary coupling weights, the discrete resolvent remains uniformly bounded across all spectral parameters away from the real axis, and the boundary sum converges monotonically to the exact seven twelfths limit without oscillatory corrections in the thermodynamic regime for every admissible family of weights.";

pub const SHORT_LEMMA_BODY: &str = "Every admissible boundary coupling weight is strictly positive and bounded above by two.";

pub const PROOF_CUE_LINE: &str = "Here we establish the global stability estimate from the local positivity bound and prior work.";

pub fn synthetic_tex_source() -> String {
    format!(
        r"\documentclass{{article}}
\newtheorem{{definition}}{{Definition}}[section]
\newtheorem{{lemma}}[definition]{{Lemma}}
\newtheorem{{theorem}}[definition]{{Theorem}}
\begin{{document}}
\section{{Framework}}\label{{sec:fw}}
\begin{{definition}}[Admissible Chain]\label{{def:chain}}
An admissible chain is a finite sequence of coupled lattice sites with positive weights.
\end{{definition}}
\section{{Main Estimates}}\label{{sec:est}}
\begin{{lemma}}[Local Positivity]\label{{lem:pos}}
{SHORT_LEMMA_BODY}
\end{{lemma}}
\begin{{theorem}}[Global Stability]\label{{thm:stab}}
{LONG_THEOREM_BODY}
\end{{theorem}}
\begin{{proof}}
{PROOF_CUE_LINE}
\end{{proof}}
\begin{{thebibliography}}{{9}}
\bibitem{{keller2022}} M. Keller, Local Chain Bounds, arXiv:2201.00456 (2022).
\end{{thebibliography}}
\end{{document}}
"
    )
}

pub struct SeededTexSession {
    pub session_id: String,
    pub source_id: String,
    pub primary_path: String,
    pub skeleton: PaperSkeleton,
    pub skeleton_seq: u64,
    pub skeleton_blob: String,
    pub lemma_tag: String,
    pub theorem_tag: String,
    pub def_tag: String,
}

pub struct TexSeedConfig<'a> {
    pub session_id: &'a str,
    pub ledger_licence: Option<&'a str>,
    pub use_us_spelling: bool,
    pub library_licence: Option<&'a str>,
    pub user_citekey: Option<&'a str>,
}

pub fn seed_tex_session(data_dir: &Path, cfg: &TexSeedConfig<'_>) -> SeededTexSession {
    let raw = synthetic_tex_source();
    let paper_key = PaperKey::from_arxiv("2403.05555").unwrap();
    let ledger = Ledger::open(data_dir).unwrap();
    ledger.append(cfg.session_id, "session.created", 1, &json!({ "title": "Synthetic Stability Study" })).unwrap();
    ledger.append(cfg.session_id, MARKER_INVESTIGATION_STARTED, 1, &json!({ "profile": "investigation/2" })).unwrap();

    let sandbox = WorkspaceSandbox::new(data_dir.to_path_buf(), None);
    let ledger_mutex = Mutex::new(Ledger::open_existing(data_dir).unwrap());
    let ing = ingest_bytes(
        &sandbox,
        &ledger_mutex,
        &IngestRequest {
            session_id: cfg.session_id,
            file_name: "stability.tex",
            origin: "https://arxiv.org/e-print/2403.05555v2",
            raw_bytes: raw.as_bytes(),
            source_id_hint: Some("src-tex-1"),
            section_filter: None,
            title: Some("Synthetic Stability of Admissible Chains"),
        },
    )
    .unwrap();
    drop(ledger_mutex);

    let primary_path = ing.primary_path.clone().unwrap_or_else(|| ing.files[0].clone());
    let mut files = BTreeMap::new();
    files.insert(primary_path.clone(), raw.clone());
    let mut hashes = BTreeMap::new();
    hashes.insert(primary_path.clone(), ledger::sha256_hex(raw.as_bytes()));

    let skel = build_tex_skeleton(&TexSkeletonInput {
        source_id: ing.source_id.clone(),
        source_seq: ing.event_seq,
        paper_key: paper_key.clone(),
        version: Some("v2".into()),
        primary_path: Some(primary_path.clone()),
        files,
        files_sha256: Some(hashes),
    })
    .skeleton;

    let skel_bytes = serde_json::to_vec(&skel).unwrap();
    let blob_hash = ledger.put_blob(&skel_bytes).unwrap();

    let mut paper_payload = json!({
        "source_id": ing.source_id,
        "title": "Synthetic Stability of Admissible Chains",
        "authors": ["Lin, Clara", "Bauer, Otto"],
        "year": 2024,
        "paper_key": paper_key.as_str(),
    });
    if let Some(lic) = cfg.ledger_licence {
        let key = if cfg.use_us_spelling { "license" } else { "licence" };
        paper_payload[key] = json!(lic);
    }
    ledger.append(cfg.session_id, "investigation.paper", 1, &paper_payload).unwrap();
    let skeleton_seq = ledger
        .append(
            cfg.session_id,
            EVENT_INVESTIGATION_SKELETON,
            1,
            &json!({
                "source_id": ing.source_id,
                "source_seq": ing.event_seq,
                "builder": skel.builder.as_str(),
                "paper_key": paper_key.as_str(),
                "version": "v2",
                "quality": skel.quality,
                "blob": blob_hash,
            }),
        )
        .unwrap();

    if cfg.library_licence.is_some() || cfg.user_citekey.is_some() {
        let lib = Library::open(data_dir).unwrap();
        lib.upsert_paper(&PaperRecord {
            key: paper_key.clone(),
            ids: LibraryIds::from_paper_ids(&PaperIds {
                arxiv: Some("2403.05555".into()),
                ..Default::default()
            }),
            title: "Synthetic Stability of Admissible Chains".into(),
            authors: vec!["Lin, Clara".into(), "Bauer, Otto".into()],
            year: Some(2024),
            abstract_text: "Synthetic abstract.".into(),
            bibtex: String::new(),
            flags: PaperFlags {
                owned: true,
                venue: Some("Journal of Synthetic Models".into()),
                license: cfg.library_licence.map(str::to_string),
                ..Default::default()
            },
            first_seen_ms: 100,
            last_seen_ms: 100,
        })
        .unwrap();
        if let Some(ck) = cfg.user_citekey {
            lib.upsert_source(&SourceRecord {
                id: "vault-1".into(),
                kind: SourceKind::KandinskyVault,
                root: "/tmp/synth-vault".into(),
                settings: json!({}),
                added_ms: 100,
                last_sync_ms: Some(100),
                state: "synced".into(),
            })
            .unwrap();
            lib.upsert_source_item(&SourceItemRecord {
                source_id: "vault-1".into(),
                external_id: ck.into(),
                key: Some(paper_key.clone()),
                citekey: Some(ck.into()),
                ids: LibraryIds::from_paper_ids(&PaperIds {
                    arxiv: Some("2403.05555".into()),
                    ..Default::default()
                }),
                version: Some("v2".into()),
                title: "Synthetic Stability of Admissible Chains".into(),
                authors: vec!["Lin, Clara".into(), "Bauer, Otto".into()],
                year: Some(2024),
                item_kind: "journalArticle".into(),
                collections: vec![],
                tags: vec![],
                files: vec![],
                state: SourceItemState::Present,
                updated_ms: 100,
            })
            .unwrap();
        }
    }

    let lemma_tag = skel.nodes.iter().find(|n| n.kind == SkNodeKind::Lemma).unwrap().tag.clone().unwrap();
    let theorem_tag = skel.nodes.iter().find(|n| n.kind == SkNodeKind::Theorem).unwrap().tag.clone().unwrap();
    let def_tag = skel.nodes.iter().find(|n| n.kind == SkNodeKind::Definition).unwrap().tag.clone().unwrap();

    SeededTexSession {
        session_id: cfg.session_id.to_string(),
        source_id: ing.source_id,
        primary_path,
        skeleton: skel,
        skeleton_seq,
        skeleton_blob: blob_hash,
        lemma_tag,
        theorem_tag,
        def_tag,
    }
}

pub fn make_anchor(
    data_dir: &Path,
    seeded: &SeededTexSession,
    line: usize,
    quote: &str,
) -> VerifiedMapAnchor {
    let ledger = Ledger::open_existing(data_dir).unwrap();
    let events = ledger.read_session(&seeded.session_id).unwrap();
    let sandbox = WorkspaceSandbox::new(data_dir.to_path_buf(), None);
    let v = verify_quote(
        &sandbox,
        &seeded.session_id,
        &events,
        &seeded.source_id,
        &seeded.primary_path,
        line,
        line,
        quote,
    )
    .unwrap();
    VerifiedMapAnchor {
        source_id: v.quote_ref.source_id,
        path: v.quote_ref.path,
        start_line: v.quote_ref.start_line,
        end_line: v.quote_ref.end_line,
        file_sha256: v.quote_ref.file_sha256,
        quote_sha256: v.quote_ref.quote_sha256,
        quote: quote.to_string(),
        excerpt: v.excerpt,
        source_seq: v.source_seq,
        cited_range: v.cited_range,
        range_basis: v.range_basis,
        excerpt_window: v.excerpt_window,
    }
}

pub fn append_full_layer2(
    data_dir: &Path,
    seeded: &SeededTexSession,
    authors_label: AuthorsClaimLabel,
    pin_override: bool,
) {
    let ledger = Ledger::open_existing(data_dir).unwrap();
    let thm_node = seeded.skeleton.nodes.iter().find(|n| n.tag.as_deref() == Some(&seeded.theorem_tag)).unwrap();
    let lem_node = seeded.skeleton.nodes.iter().find(|n| n.tag.as_deref() == Some(&seeded.lemma_tag)).unwrap();
    let def_node = seeded.skeleton.nodes.iter().find(|n| n.tag.as_deref() == Some(&seeded.def_tag)).unwrap();
    let prf_node = seeded.skeleton.nodes.iter().find(|n| n.kind == SkNodeKind::Proof).unwrap();

    let req_orient = ledger
        .append(
            &seeded.session_id,
            MARKER_TASK_REQUESTED,
            1,
            &json!({ "task": "orient", "tags": [], "source_id": seeded.source_id, "skeleton_seq": seeded.skeleton_seq }),
        )
        .unwrap();

    let thm_anchor = make_anchor(data_dir, seeded, thm_node.spans[0].start_line + 1, "the discrete resolvent remains uniformly bounded across all spectral parameters");
    let lem_anchor = make_anchor(data_dir, seeded, lem_node.spans[0].start_line + 1, SHORT_LEMMA_BODY);
    let cue_anchor = make_anchor(data_dir, seeded, prf_node.spans[0].start_line + 1, PROOF_CUE_LINE);

    let child_tag = "m-proofstep01".to_string();
    let reading_payload = MapReadingEventPayload {
        schema_version: 1,
        request_seq: req_orient,
        source_id: seeded.source_id.clone(),
        skeleton_seq: seeded.skeleton_seq,
        done: true,
        items: vec![
            ReadingItemRecord {
                tag: seeded.theorem_tag.clone(),
                node_fp: thm_node.fp.clone(),
                attach_to: None,
                model_kind: None,
                role: Some(ReadingRole::MainResult),
                gist: Some("Establishes uniform resolvent bounds and convergence to seven twelfths.".into()),
                purpose: Some("Primary convergence estimate of the paper.".into()),
                hypotheses: vec![HypothesisRecord {
                    text: "Chain length is even and at least four.".into(),
                    anchor: thm_anchor.clone(),
                }],
                anchors: vec![thm_anchor.clone()],
                authors_claim: Some(AuthorsClaimRecord {
                    label: authors_label,
                    relation_note: Some("extends earlier finite-chain bounds".into()),
                    attributed_to: vec!["keller2022".into()],
                    anchors: if authors_label == AuthorsClaimLabel::NotStated { vec![] } else { vec![cue_anchor.clone()] },
                    cues: vec![],
                }),
            },
            ReadingItemRecord {
                tag: seeded.lemma_tag.clone(),
                node_fp: lem_node.fp.clone(),
                attach_to: None,
                model_kind: None,
                role: Some(ReadingRole::Supporting),
                gist: Some("Bounds each boundary coupling weight cleanly.".into()),
                purpose: None,
                hypotheses: vec![],
                anchors: vec![lem_anchor.clone()],
                authors_claim: None,
            },
            ReadingItemRecord {
                tag: child_tag,
                node_fp: thm_node.fp.clone(),
                attach_to: Some(seeded.theorem_tag.clone()),
                model_kind: Some("proof_step".into()),
                role: None,
                gist: Some("Combines local positivity with the boundary identity.".into()),
                purpose: None,
                hypotheses: vec![],
                anchors: vec![cue_anchor],
                authors_claim: None,
            },
        ],
        edges: vec![],
        limitations: vec![],
        rejected: ReadingRejected::default(),
        anchor_report: json!({}),
    };
    ledger.append(&seeded.session_id, EVENT_INVESTIGATION_READING, 1, &serde_json::to_value(&reading_payload).unwrap()).unwrap();
    ledger
        .append(&seeded.session_id, MARKER_TASK_FINISHED, 1, &json!({ "request_seq": req_orient, "status": "done" }))
        .unwrap();

    // V12 Context card on theorem.
    let req_ctx = ledger
        .append(
            &seeded.session_id,
            MARKER_TASK_REQUESTED,
            1,
            &json!({ "task": "context", "tags": [seeded.theorem_tag], "source_id": seeded.source_id, "skeleton_seq": seeded.skeleton_seq }),
        )
        .unwrap();
    let ctx_payload = MapContextEventPayload {
        schema_version: CONTEXT_SCHEMA_VERSION,
        request_seq: req_ctx,
        source_id: seeded.source_id.clone(),
        skeleton_seq: seeded.skeleton_seq,
        tag: seeded.theorem_tag.clone(),
        cards: vec![ContextCardRecord {
            card_type: ContextCardType::Prerequisite,
            title: "Spectral gap intuition".into(),
            body: "Positive coupling weights imply uniform spectral separation.".into(),
            origin: ContextCardOrigin::ModelBackground,
            anchors: vec![],
            hypotheses: vec![],
            bib_key: None,
            external_source_id: None,
            external_tag: None,
            node_fp: Some(thm_node.fp.clone()),
        }],
        rejected: vec![],
        anchor_report: Value::Null,
    };
    ledger.append(&seeded.session_id, EVENT_INVESTIGATION_CONTEXT, 1, &serde_json::to_value(&ctx_payload).unwrap()).unwrap();
    ledger.append(&seeded.session_id, MARKER_TASK_FINISHED, 1, &json!({ "request_seq": req_ctx, "status": "done" })).unwrap();

    // User text notes on lemma and definition.
    ledger
        .append(
            &seeded.session_id,
            EVENT_INVESTIGATION_NOTE,
            1,
            &json!({
                "source_id": seeded.source_id,
                "tag": seeded.lemma_tag,
                "node_fp": lem_node.fp,
                "text": "Check whether odd chain lengths keep the upper bound of two.",
            }),
        )
        .unwrap();
    ledger
        .append(
            &seeded.session_id,
            EVENT_INVESTIGATION_NOTE,
            1,
            &json!({
                "source_id": seeded.source_id,
                "tag": seeded.def_tag,
                "node_fp": def_node.fp,
                "text": "Admissibility excludes zero weights at the boundary.",
            }),
        )
        .unwrap();

    // Optional pin override: user demotes theorem and promotes lemma!
    if pin_override {
        ledger
            .append(
                &seeded.session_id,
                EVENT_INVESTIGATION_NOTE,
                1,
                &json!({ "source_id": seeded.source_id, "tag": seeded.theorem_tag, "node_fp": thm_node.fp, "pin": "not_main_result" }),
            )
            .unwrap();
        ledger
            .append(
                &seeded.session_id,
                EVENT_INVESTIGATION_NOTE,
                1,
                &json!({ "source_id": seeded.source_id, "tag": seeded.lemma_tag, "node_fp": lem_node.fp, "pin": "main_result" }),
            )
            .unwrap();
    }

    // F16a reader override on definition.
    ledger
        .append(
            &seeded.session_id,
            EVENT_INVESTIGATION_OVERRIDE,
            1,
            &json!({
                "source_id": seeded.source_id,
                "tag": seeded.def_tag,
                "node_fp": def_node.fp,
                "fix": { "kind": "Assumption", "note": "Boundary positivity is assumed throughout." }
            }),
        )
        .unwrap();
}

pub fn seed_pdf_ocr_session(data_dir: &Path, session_id: &str, raw_ocr_basis: bool) -> String {
    let reading_text = concat!(
        "1. Introduction\n",
        "Theorem 1.1. The clean digital page states that every finite coupling sum is positive.\n",
        "\x0c\n",
        "2. Scanned Estimates\n",
        "Theorem 2.1. On scanned page two the OCR text records E = mc^2 with residual error below 0.01.\n",
    );
    let paper_key = PaperKey::from_file_bytes(reading_text.as_bytes());
    let ledger = Ledger::open(data_dir).unwrap();
    ledger.append(session_id, "session.created", 1, &json!({ "title": "Synthetic Scanned PDF" })).unwrap();
    ledger.append(session_id, MARKER_INVESTIGATION_STARTED, 1, &json!({ "profile": "investigation/2" })).unwrap();

    let sandbox = WorkspaceSandbox::new(data_dir.to_path_buf(), None);
    let rel_path = "src-pdf-1/paper.txt";
    let dst = sandbox
        .canonical_root(research_tools::sandbox::SandboxRoot::Sources, session_id, true)
        .unwrap()
        .unwrap()
        .join(rel_path);
    std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
    std::fs::write(&dst, reading_text.as_bytes()).unwrap();
    let sha = ledger::sha256_hex(reading_text.as_bytes());

    let metrics = PageQualityMetrics {
        char_count: 100, alpha_count: 80, word_count: 18,
        image_coverage_pct: 90, has_invisible_text: true, width_pt: 612, height_pt: 792,
    };
    let page_map = PageMap {
        page_count: 2,
        pages: vec![
            PageRecord { page: 1, basis: PageBasis::TextLayer, first_line: Some(1), last_line: Some(2), metrics: metrics.clone(), reason: None },
            PageRecord { page: 2, basis: PageBasis::ScanTextLayer, first_line: Some(4), last_line: Some(5), metrics: metrics.clone(), reason: Some("scan".into()) },
        ],
    };
    let pdf_pages_json = if raw_ocr_basis {
        json!([
            { "page": 1, "basis": "text_layer", "first_line": 1, "last_line": 2, "metrics": metrics },
            { "page": 2, "basis": "ocr", "first_line": 4, "last_line": 5, "metrics": metrics }
        ])
    } else {
        serde_json::to_value(&page_map.pages).unwrap()
    };

    let source_seq = ledger
        .append(
            session_id,
            "source.ingested",
            1,
            &json!({
                "source_id": "src-pdf-1",
                "title": "Synthetic Scanned Archive Note",
                "format": "pdf",
                "status": "pdf_only",
                "primary_path": rel_path,
                "files": [rel_path],
                "files_sha256": { rel_path: sha },
                "pdf_extractor": "pdf-extract-layout/0.12.1",
                "pdf_pages": pdf_pages_json,
            }),
        )
        .unwrap();

    let out = build_pdf_skeleton(&PdfSkeletonInput {
        source_id: "src-pdf-1".into(),
        source_seq,
        paper_key: paper_key.clone(),
        version: None,
        primary_path: rel_path.into(),
        text: reading_text.into(),
        page_map,
        extractor_id: Some("pdf-extract-layout/0.12.1".into()),
        files_sha256: Some(BTreeMap::from([(rel_path.to_string(), sha)])),
        extra_quality_notes: vec![],
    });
    let blob = ledger.put_blob(&serde_json::to_vec(&out.skeleton).unwrap()).unwrap();
    ledger
        .append(
            session_id,
            "investigation.paper",
            1,
            &json!({ "source_id": "src-pdf-1", "title": "Synthetic Scanned Archive Note" }),
        )
        .unwrap();
    ledger
        .append(
            session_id,
            EVENT_INVESTIGATION_SKELETON,
            1,
            &json!({
                "source_id": "src-pdf-1",
                "source_seq": source_seq,
                "builder": "pdf-1",
                "paper_key": paper_key.as_str(),
                "quality": out.skeleton.quality,
                "blob": blob,
            }),
        )
        .unwrap();
    session_id.to_string()
}
