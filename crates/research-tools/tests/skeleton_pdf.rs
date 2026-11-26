//! End-to-end integration tests for Investigate V17 (`skeleton/pdf.rs`, builder `pdf-1`):
//! multi-page text-layer PDFs with footers, quotes/anchors, two-column APS layout, and legacy
//! pre-F14a PDF receipts. Strictly self-authored fixtures generated via `pdf_support`.

mod pdf_support;

use ledger::Ledger;
use pdf_support::{build_pdf, PageSpec, PdfBuildOptions};
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::paper_key::PaperKey;
use research_tools::pdf_pages::{PdfExtractOptions, PdfExtractPlain, PdfExtractor, PAGE_BREAK_CHAR};
use research_tools::quote_core::verify_quote;
use research_tools::skeleton::{
    build_pdf_skeleton_from_ingested, match_pdf_quote_for_display, page_range_for_node,
    page_range_for_span, EdgeConfidence, NumberBasis, PdfSkeletonOutput, SkConfidence, SkEdgeKind,
    SkEdgeRole, SkEdgeRule, SkNodeKind, SkeletonBuilder, SkeletonQuality, PDF_QUALITY_REASON_BASE,
    PDF_QUALITY_SCAN_TAG,
};
use research_tools::ResearchEnv;
use serde_json::json;
use std::fs;
use std::sync::Mutex;
use tempfile::TempDir;

fn assert_common_pdf_invariants(out: &PdfSkeletonOutput) {
    let sk = &out.skeleton;
    assert_eq!(sk.builder, SkeletonBuilder::Pdf1);
    assert_ne!(sk.quality, SkeletonQuality::Full, "PDF skeleton must never be quality=full");
    let reason = sk.quality_reason.as_deref().expect("PDF skeleton always shows quality_reason");
    assert!(reason.contains(PDF_QUALITY_REASON_BASE), "missing base quality line: {reason}");

    for (i, node) in sk.nodes.iter().enumerate() {
        assert_eq!(node.order, i);
        assert_eq!(node.confidence, SkConfidence::PdfHeuristic);
        assert!(node.label.is_none(), "PDF nodes must use content fingerprints, never labels");
        assert!(node.tag.as_deref().is_some_and(|t| t.starts_with("n-")));
        assert_eq!(node.fp.len(), 12);
        if node.number.is_some() {
            assert_eq!(node.number_basis, Some(NumberBasis::Printed));
        } else {
            assert_eq!(node.number_basis, None);
        }
        if let Some(p_idx) = out.parent_orders[i] {
            let parent = &sk.nodes[p_idx];
            assert_eq!(node.parent, parent.tag);
            assert!(
                parent.spans[0].contains(&node.spans[0]),
                "child span {:?} must sit inside parent span {:?}",
                node.spans[0],
                parent.spans[0]
            );
        }
    }

    for edge in &sk.edges {
        assert_eq!(edge.confidence, EdgeConfidence::Heuristic);
        assert_ne!(edge.from, edge.to);
    }

    for p in sk.nodes.iter().filter(|n| n.kind == SkNodeKind::Proof) {
        let proves_count = sk
            .edges
            .iter()
            .filter(|e| e.kind == SkEdgeKind::Proves && Some(e.from.as_str()) == p.tag.as_deref())
            .count();
        assert!(proves_count <= 1);
    }
}

#[test]
fn multi_page_text_layer_pdf_skeleton_footers_quotes_and_anchors() {
    let tmp = TempDir::new().unwrap();
    let env = ResearchEnv::new(tmp.path());
    let ledger = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let sid = "s-v17-multi";

    let pdf_bytes = build_pdf(
        &[
            PageSpec::single_column(&[
                "A. Author -- Spectral Decay",
                "Weighted Graph Contraction Estimates",
                "Abstract",
                "We prove spectral decay for weighted Laplacians in Theorem 2.2.",
                "1. Introduction",
                "Earlier bounds appear in [1] and [2, Theorem 3.1].",
                "Definition 1.1. A graph weight w is strictly positive on every edge.",
                "- 101 -",
            ]),
            PageSpec::single_column(&[
                "A. Author -- Spectral Decay",
                "2. Main Results",
                "2.1 Auxiliary Estimates",
                "Lemma 2.1. Under Definition 1.1, the Dirichlet energy satisfies",
                "E(u) = sum w_e (du)^2 >= 0 (1)",
                "Proof. Expand each squared difference term directly. Q.E.D.",
                "Theorem 2.2 ([1]). Combining Lemma 2.1 and Eq. (1) gives",
                "||P^k f|| <= exp(-c k) ||f|| (2.1)",
                "- 102 -",
            ]),
            PageSpec::single_column(&[
                "A. Author -- Spectral Decay",
                "3. Proofs",
                "Proof of Theorem 2.2. Apply Lemma 2.1 and Eq. (2.1) inductively.",
                "This closes the decay estimate in all dimensions. Q.E.D.",
                "Figure 1: Decay constant versus edge weight ratio.",
                "3",
            ]),
            PageSpec::single_column(&[
                "A. Author -- Spectral Decay",
                "References",
                "[1] A. Researcher, \"Discrete Laplacians,\" arXiv:2201.05432v2 (2022).",
                "[2] B. Analyst, \"Weighted Energy Bounds,\" doi:10.1000/spectral.2022 (2021).",
                "(4)",
            ]),
        ],
        &PdfBuildOptions::default(),
    );

    let ing = ingest_bytes(
        &env.sandbox(),
        &ledger,
        &IngestRequest {
            session_id: sid,
            file_name: "spectral.pdf",
            origin: "upload:spectral.pdf",
            raw_bytes: &pdf_bytes,
            source_id_hint: Some("spectral"),
            section_filter: None,
            title: None,
        },
    )
    .unwrap();

    let events = ledger.lock().unwrap().read_session(sid).unwrap();
    let receipt = events.iter().find(|e| e.seq == ing.event_seq).unwrap();
    let primary_path = ing.primary_path.as_deref().unwrap();
    let disk_path = env
        .sandbox()
        .resolve_existing(research_tools::sandbox::SandboxRoot::Sources, sid, primary_path, false)
        .unwrap()
        .canonical_target;
    let stored_before = fs::read_to_string(&disk_path).unwrap();

    let paper_key = PaperKey::from_file_bytes(&pdf_bytes);
    let (out, _timing) =
        build_pdf_skeleton_from_ingested(&env.sandbox(), sid, receipt, paper_key, None).unwrap();
    assert_common_pdf_invariants(&out);

    assert_eq!(fs::read_to_string(&disk_path).unwrap(), stored_before);
    let sk = &out.skeleton;
    assert_eq!(sk.quality, SkeletonQuality::Partial);
    assert!(!sk.quality_reason.as_deref().unwrap().contains(PDF_QUALITY_SCAN_TAG));

    let sec = |num: &str| {
        sk.nodes
            .iter()
            .find(|n| n.kind == SkNodeKind::Section && n.number.as_deref() == Some(num))
            .unwrap_or_else(|| panic!("missing section {num}"))
    };
    assert_eq!(page_range_for_node(&out.page_map, sec("1")), Some((1, 1)));
    assert_eq!(page_range_for_node(&out.page_map, sec("2")), Some((2, 2)));
    assert_eq!(page_range_for_node(&out.page_map, sec("2.1")), Some((2, 2)));
    assert_eq!(sec("2.1").parent, sec("2").tag);
    assert_eq!(page_range_for_node(&out.page_map, sec("3")), Some((3, 3)));

    let def11 = sk.nodes.iter().find(|n| n.kind == SkNodeKind::Definition).unwrap();
    let lem21 = sk.nodes.iter().find(|n| n.kind == SkNodeKind::Lemma).unwrap();
    let thm22 = sk.nodes.iter().find(|n| n.kind == SkNodeKind::Theorem).unwrap();
    let eq1 = sk.nodes.iter().find(|n| n.kind == SkNodeKind::Equation && n.number.as_deref() == Some("1")).unwrap();
    let eq21 = sk.nodes.iter().find(|n| n.kind == SkNodeKind::Equation && n.number.as_deref() == Some("2.1")).unwrap();
    let fig1 = sk.nodes.iter().find(|n| n.kind == SkNodeKind::Figure).unwrap();
    let proofs: Vec<_> = sk.nodes.iter().filter(|n| n.kind == SkNodeKind::Proof).collect();

    assert_eq!(page_range_for_node(&out.page_map, def11), Some((1, 1)));
    assert_eq!(page_range_for_node(&out.page_map, lem21), Some((2, 2)));
    assert_eq!(page_range_for_node(&out.page_map, thm22), Some((2, 2)));
    assert_eq!(page_range_for_node(&out.page_map, eq1), Some((2, 2)));
    assert_eq!(page_range_for_node(&out.page_map, eq21), Some((2, 2)));
    assert_eq!(page_range_for_node(&out.page_map, fig1), Some((3, 3)));
    assert_eq!(proofs.len(), 2);
    assert_eq!(proofs[0].proof_of, lem21.tag);
    assert_eq!(proofs[1].proof_of, thm22.tag);
    assert_eq!(page_range_for_node(&out.page_map, proofs[1]), Some((3, 3)));
    assert!(thm22.intro_refs, "Abstract refers to Theorem 2.2");
    assert_eq!(thm22.head_cites, vec!["1"]);
    assert_eq!(sk.nodes.iter().filter(|n| n.kind == SkNodeKind::Equation).count(), 2);

    let has_edge = |from: &str, to: &str, kind: SkEdgeKind, role: SkEdgeRole, rule: SkEdgeRule| {
        sk.edges.iter().any(|e| {
            e.from == from && e.to == to && e.kind == kind && e.role == role && e.rule == rule
        })
    };
    assert!(has_edge(def11.tag.as_deref().unwrap(), lem21.tag.as_deref().unwrap(), SkEdgeKind::Uses, SkEdgeRole::Statement, SkEdgeRule::TextMention));
    assert!(has_edge(lem21.tag.as_deref().unwrap(), thm22.tag.as_deref().unwrap(), SkEdgeKind::Uses, SkEdgeRole::Statement, SkEdgeRule::TextMention));
    assert!(has_edge(eq1.tag.as_deref().unwrap(), thm22.tag.as_deref().unwrap(), SkEdgeKind::Uses, SkEdgeRole::Statement, SkEdgeRule::TextMention));
    assert!(has_edge(lem21.tag.as_deref().unwrap(), thm22.tag.as_deref().unwrap(), SkEdgeKind::Uses, SkEdgeRole::Proof, SkEdgeRule::TextMention));
    assert!(has_edge("1", thm22.tag.as_deref().unwrap(), SkEdgeKind::Cites, SkEdgeRole::Statement, SkEdgeRule::HeadCite));

    assert_eq!(sk.bib.len(), 2);
    assert_eq!(sk.bib[0].key, "1");
    assert_eq!(sk.bib[0].arxiv_id.as_deref(), Some("2201.05432"));
    assert_eq!(sk.bib[1].key, "2");
    assert_eq!(sk.bib[1].doi.as_deref(), Some("10.1000/spectral.2022"));
    assert_eq!(sk.bib[1].locators.len(), 1);
    assert_eq!(sk.bib[1].locators[0].text, "Theorem 3.1");

    let samples = [
        ("A graph weight w is strictly positive on every edge.", 1usize),
        ("Combining Lemma 2.1 and Eq. (1) gives", 2usize),
        ("This closes the decay estimate in all dimensions.", 3usize),
    ];
    for (quote, expected_page) in samples {
        let disp = match_pdf_quote_for_display(primary_path, &stored_before, &out.page_map, quote)
            .unwrap_or_else(|| panic!("quote did not match: {quote}"));
        assert_eq!(disp.start_page, expected_page);
        assert_eq!(page_range_for_span(&out.page_map, &disp.span), Some((expected_page, expected_page)));
        assert!(!disp.is_ocr);
        assert!(disp.render_math);
        let verified = verify_quote(
            &env.sandbox(),
            sid,
            &events,
            &ing.source_id,
            &disp.span.path,
            disp.span.start_line,
            disp.span.end_line,
            &disp.stored_excerpt,
        )
        .unwrap();
        assert_eq!(verified.quote_ref.start_line, disp.span.start_line);
    }
}

#[test]
fn two_column_interleaved_physics_pdf_reading_order_and_aps_sections() {
    let tmp = TempDir::new().unwrap();
    let env = ResearchEnv::new(tmp.path());
    let ledger = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let sid = "s-v17-twocol";

    let pdf_bytes = build_pdf(
        &[PageSpec::two_column(
            &["Coupled Oscillator Chain"],
            &[
                "I. INTRODUCTION",
                "We study a 1D chain as in [1].",
                "II. MODEL",
                "A. Lattice Setup",
                "H = sum p_j^2 + q_j^2 (1)",
            ],
            &[
                "B. Spectral Gap",
                "From Eq. (1) we compute the gap.",
                "delta = E_1 - E_0 > 0 (2)",
                "References",
                "[1] C. Physicist, doi:10.1103/physrevb.99.010101.",
            ],
            true,
        )],
        &PdfBuildOptions::default(),
    );

    let ing = ingest_bytes(
        &env.sandbox(),
        &ledger,
        &IngestRequest {
            session_id: sid,
            file_name: "chain.pdf",
            origin: "upload:chain.pdf",
            raw_bytes: &pdf_bytes,
            source_id_hint: Some("chain"),
            section_filter: None,
            title: None,
        },
    )
    .unwrap();
    let events = ledger.lock().unwrap().read_session(sid).unwrap();
    let receipt = events.iter().find(|e| e.seq == ing.event_seq).unwrap();
    let (out, _) = build_pdf_skeleton_from_ingested(
        &env.sandbox(),
        sid,
        receipt,
        PaperKey::from_file_bytes(&pdf_bytes),
        None,
    )
    .unwrap();
    assert_common_pdf_invariants(&out);

    let sk = &out.skeleton;
    let sec_nums: Vec<_> = sk
        .nodes
        .iter()
        .filter(|n| n.kind == SkNodeKind::Section)
        .filter_map(|n| n.number.as_deref())
        .collect();
    assert_eq!(sec_nums, vec!["I", "II", "II.A", "II.B"]);
    assert_eq!(sk.nodes.iter().filter(|n| n.kind == SkNodeKind::Equation).count(), 2);
    assert_eq!(sk.bib.len(), 1);
    assert_eq!(sk.bib[0].doi.as_deref(), Some("10.1103/physrevb.99.010101"));

    let eq1 = sk.nodes.iter().find(|n| n.kind == SkNodeKind::Equation && n.number.as_deref() == Some("1")).unwrap();
    let sec_b = sk.nodes.iter().find(|n| n.number.as_deref() == Some("II.B")).unwrap();
    assert!(sk.edges.iter().any(|e| {
        e.from == eq1.tag.as_deref().unwrap()
            && e.to == sec_b.tag.as_deref().unwrap()
            && e.kind == SkEdgeKind::Uses
            && e.rule == SkEdgeRule::TextMention
            && e.confidence == EdgeConfidence::Heuristic
    }));
}

#[test]
fn legacy_pre_f14a_pdf_receipt_loads_and_notes_unknown_scan_status() {
    let tmp = TempDir::new().unwrap();
    let env = ResearchEnv::new(tmp.path());
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-v17-legacy";

    let pdf_bytes = build_pdf(
        &[
            PageSpec::single_column(&["1. Intro", "Theorem 1. Legacy theorem text."]),
            PageSpec::single_column(&["2. End", "By Theorem 1 we conclude."]),
        ],
        &PdfBuildOptions::default(),
    );
    let plain = PdfExtractPlain.extract(&pdf_bytes, &PdfExtractOptions::default()).unwrap();
    let p1 = plain.text.split(PAGE_BREAK_CHAR).next().unwrap().trim();
    let p2 = plain.text.split(PAGE_BREAK_CHAR).nth(1).unwrap().trim();
    let legacy_text = format!("--- [Page 1] ---\n{p1}\n\n--- [Page 2] ---\n{p2}");
    let p1_last = 1 + p1.lines().count();
    let p2_first = p1_last + 2;
    let p2_last = p2_first + p2.lines().count();
    let src_id = "legacy-v17-1234567890abcdef";
    let rel_path = format!("{src_id}/legacy.pdf.extracted.txt");
    let dir = tmp.path().join("sessions").join(sid).join("sources").join(src_id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("legacy.pdf.extracted.txt"), &legacy_text).unwrap();

    let sha = ledger::sha256_hex(legacy_text.as_bytes());
    let raw_blob = ledger.put_blob(&pdf_bytes).unwrap();
    let seq = ledger
        .append(
            sid,
            "source.ingested",
            1,
            &json!({
                "source_id": src_id,
                "origin": "upload",
                "format": "pdf",
                "status": "ok",
                "bytes": pdf_bytes.len(),
                "raw_blob": raw_blob,
                "primary_path": rel_path,
                "assembled_path": null,
                "files": [rel_path],
                "files_sha256": { rel_path.clone(): sha },
                "page_count": 2,
                "pdf_page_lines": [[1, 1, p1_last], [2, p2_first, p2_last]],
            }),
        )
        .unwrap();
    let events = ledger.read_session(sid).unwrap();
    let receipt = events.iter().find(|e| e.seq == seq).unwrap();
    let (out, _) = build_pdf_skeleton_from_ingested(
        &env.sandbox(),
        sid,
        receipt,
        PaperKey::from_file_bytes(&pdf_bytes),
        None,
    )
    .unwrap();
    assert_common_pdf_invariants(&out);
    assert!(out.skeleton.quality_reason.as_deref().unwrap().contains("pre-F14a"));
    let thm = out.skeleton.nodes.iter().find(|n| n.kind == SkNodeKind::Theorem).unwrap();
    assert_eq!(page_range_for_node(&out.page_map, thm), Some((1, 1)));
}
