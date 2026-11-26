//! Rule 6.21 honesty, scan/image-only pages, page ranges beginning after page 1, ambiguous
//! mentions, rebuild stability, and tamper-rejection integration tests for `pdf-1` (Investigate V17).

mod pdf_support;

use ledger::Ledger;
use pdf_support::{build_pdf, PageSpec, PdfBuildOptions};
use research_tools::ingest::{ingest_bytes, ingest_pdf_pages, IngestRequest};
use research_tools::paper_key::PaperKey;
use research_tools::pdf_pages::{extract_pdf_pages, PageBasis, PdfExtractOptions};
use research_tools::skeleton::{
    build_pdf_skeleton_from_ingested, match_pdf_quote_for_display, node_page_basis,
    page_range_for_node, render_formulas_as_math, EdgeConfidence, NumberBasis, PdfSkeletonOutput,
    PdfTextOrigin, SkConfidence, SkEdgeKind, SkEdgeRule, SkNodeKind, SkeletonBuilder,
    SkeletonQuality, PDF_MODEL_TRANSCRIPTION_LABEL, PDF_OCR_QUOTE_LABEL, PDF_QUALITY_REASON_BASE,
    PDF_QUALITY_SCAN_TAG,
};
use research_tools::ResearchEnv;
use std::fs;
use std::sync::Mutex;
use tempfile::TempDir;

fn assert_common_pdf_invariants(out: &PdfSkeletonOutput) {
    let sk = &out.skeleton;
    assert_eq!(sk.builder, SkeletonBuilder::Pdf1);
    assert_ne!(sk.quality, SkeletonQuality::Full);
    assert!(sk
        .quality_reason
        .as_deref()
        .is_some_and(|r| r.contains(PDF_QUALITY_REASON_BASE)));
    for (i, node) in sk.nodes.iter().enumerate() {
        assert_eq!(node.confidence, SkConfidence::PdfHeuristic);
        assert!(node.label.is_none());
        assert!(node.tag.as_deref().is_some_and(|t| t.starts_with("n-")));
        if node.number.is_some() {
            assert_eq!(node.number_basis, Some(NumberBasis::Printed));
        }
        if let Some(p_idx) = out.parent_orders[i] {
            assert!(sk.nodes[p_idx].spans[0].contains(&node.spans[0]));
        }
    }
    for edge in &sk.edges {
        assert_eq!(edge.confidence, EdgeConfidence::Heuristic);
        assert_ne!(edge.from, edge.to);
    }
}

#[test]
fn mixed_scan_text_layer_and_image_only_pages_enforce_rule_6_21() {
    let tmp = TempDir::new().unwrap();
    let env = ResearchEnv::new(tmp.path());
    let ledger = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let sid = "s-v17-scan";

    let mixed_pdf = build_pdf(
        &[
            PageSpec::scan_with_text(
                &[
                    "1. Historical Setup",
                    "Theorem 1.1. The scanned operator satisfies",
                    "T^2 - I = 0 (1)",
                    "Every involution splits the space into two eigenspaces.",
                ],
                true,
            ),
            PageSpec::full_page_image_only(),
            PageSpec::single_column(&[
                "2. Modern Corollary",
                "Corollary 2.1. By Theorem 1.1 and Eq. (1), the spectrum is finite.",
            ]),
        ],
        &PdfBuildOptions::default(),
    );

    let ing = ingest_bytes(
        &env.sandbox(),
        &ledger,
        &IngestRequest {
            session_id: sid,
            file_name: "scanned_mixed.pdf",
            origin: "upload:scanned_mixed.pdf",
            raw_bytes: &mixed_pdf,
            source_id_hint: Some("scanned"),
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
        PaperKey::from_file_bytes(&mixed_pdf),
        None,
    )
    .unwrap();
    assert_common_pdf_invariants(&out);

    let sk = &out.skeleton;
    assert_eq!(sk.quality, SkeletonQuality::Partial);
    assert!(sk.quality_reason.as_deref().unwrap().contains(PDF_QUALITY_SCAN_TAG));
    assert_eq!(out.scan_text_pages, vec![1]);
    assert_eq!(out.image_only_pages, vec![2]);
    assert!(out.pages[1].image_available && !out.pages[1].has_text && !out.pages[1].render_math);

    let thm11 = sk.nodes.iter().find(|n| n.kind == SkNodeKind::Theorem).unwrap();
    let cor21 = sk.nodes.iter().find(|n| n.kind == SkNodeKind::Corollary).unwrap();
    assert_eq!(page_range_for_node(&out.page_map, thm11), Some((1, 1)));
    assert_eq!(node_page_basis(&out.page_map, thm11), Some(PageBasis::ScanTextLayer));
    assert!(!render_formulas_as_math(PageBasis::ScanTextLayer));
    assert_eq!(page_range_for_node(&out.page_map, cor21), Some((3, 3)));
    assert_eq!(node_page_basis(&out.page_map, cor21), Some(PageBasis::TextLayer));

    // Quote from scanned page 1 keeps OCR basis and disables math rendering.
    let primary_path = ing.primary_path.as_deref().unwrap();
    let stored = fs::read_to_string(
        env.sandbox()
            .resolve_existing(research_tools::sandbox::SandboxRoot::Sources, sid, primary_path, false)
            .unwrap()
            .canonical_target,
    )
    .unwrap();
    let q = match_pdf_quote_for_display(
        primary_path,
        &stored,
        &out.page_map,
        "Every involution splits the space into two eigenspaces.",
    )
    .expect("scan quote matches");
    assert_eq!(q.start_page, 1);
    assert!(q.is_ocr);
    assert!(!q.render_math);
    assert!(q.page_image_available);
    assert_eq!(q.display_label, format!("{PDF_OCR_QUOTE_LABEL}, p. 1"));
    assert!(!PdfTextOrigin::ModelImageTranscription.can_be_matched_anchor());
    assert_eq!(
        PdfTextOrigin::ModelImageTranscription.display_prefix(),
        PDF_MODEL_TRANSCRIPTION_LABEL
    );

    // All-image-only PDF yields quality=weak with "(scan)" and preserves all pages as images.
    let all_img_pdf = build_pdf(
        &[PageSpec::full_page_image_only(), PageSpec::full_page_image_only()],
        &PdfBuildOptions::default(),
    );
    let ing_img = ingest_bytes(
        &env.sandbox(),
        &ledger,
        &IngestRequest {
            session_id: sid,
            file_name: "all_img.pdf",
            origin: "upload:all_img.pdf",
            raw_bytes: &all_img_pdf,
            source_id_hint: Some("allimg"),
            section_filter: None,
            title: None,
        },
    )
    .unwrap();
    let events2 = ledger.lock().unwrap().read_session(sid).unwrap();
    let rec2 = events2.iter().find(|e| e.seq == ing_img.event_seq).unwrap();
    let (out_img, _) = build_pdf_skeleton_from_ingested(
        &env.sandbox(),
        sid,
        rec2,
        PaperKey::from_file_bytes(&all_img_pdf),
        None,
    )
    .unwrap();
    assert_common_pdf_invariants(&out_img);
    assert_eq!(out_img.skeleton.quality, SkeletonQuality::Weak);
    assert!(out_img.skeleton.nodes.is_empty());
    assert!(out_img.skeleton.quality_reason.as_deref().unwrap().contains(PDF_QUALITY_SCAN_TAG));
    assert_eq!(out_img.image_only_pages, vec![1, 2]);
}

#[test]
fn page_range_beginning_after_page_one_maps_physical_pages_and_sections() {
    let tmp = TempDir::new().unwrap();
    let env = ResearchEnv::new(tmp.path());
    let ledger = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let sid = "s-v17-range";

    let pdf_bytes = build_pdf(
        &[
            PageSpec::single_column(&["1. Earlier Intro", "Page one text."]),
            PageSpec::single_column(&["2. Background", "Page two text."]),
            PageSpec::single_column(&["3. Setup", "Page three text."]),
            PageSpec::single_column(&[
                "4. Spectral Bounds",
                "Theorem 4.1. Every finite truncation obeys",
                "||H_N u|| <= c ||u|| (4.1)",
                "Proof. Apply Cauchy-Schwarz directly. Q.E.D.",
                "40",
            ]),
            PageSpec::single_column(&[
                "5. Applications",
                "Lemma 5.1. By Theorem 4.1 and Eq. (4.1), see [7].",
                "41",
            ]),
            PageSpec::single_column(&[
                "References",
                "[7] D. Writer, \"Truncation Bounds,\" doi:10.1000/range.7 (2020).",
                "42",
            ]),
        ],
        &PdfBuildOptions::default(),
    );

    let extracted =
        extract_pdf_pages(&pdf_bytes, &PdfExtractOptions::default().with_page_range(4, 6)).unwrap();
    let ing = ingest_pdf_pages(
        &env.sandbox(),
        &ledger,
        &IngestRequest {
            session_id: sid,
            file_name: "long_book.pdf",
            origin: "upload:long_book.pdf",
            raw_bytes: &pdf_bytes,
            source_id_hint: Some("longbook"),
            section_filter: None,
            title: None,
        },
        extracted,
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
    assert_eq!(sec_nums, vec!["4", "5"]);

    let thm41 = sk.nodes.iter().find(|n| n.kind == SkNodeKind::Theorem).unwrap();
    let lem51 = sk.nodes.iter().find(|n| n.kind == SkNodeKind::Lemma).unwrap();
    let bib_node = sk.nodes.iter().find(|n| n.kind == SkNodeKind::Bibliography).unwrap();
    assert_eq!(page_range_for_node(&out.page_map, thm41), Some((4, 4)));
    assert_eq!(page_range_for_node(&out.page_map, lem51), Some((5, 5)));
    assert_eq!(page_range_for_node(&out.page_map, bib_node), Some((6, 6)));
    assert!(sk.quality_reason.as_deref().unwrap().contains("4..=6 of 6"));

    let primary_path = ing.primary_path.as_deref().unwrap();
    let stored = fs::read_to_string(
        env.sandbox()
            .resolve_existing(research_tools::sandbox::SandboxRoot::Sources, sid, primary_path, false)
            .unwrap()
            .canonical_target,
    )
    .unwrap();
    let q = match_pdf_quote_for_display(primary_path, &stored, &out.page_map, "By Theorem 4.1 and Eq. (4.1)")
        .expect("quote matches");
    assert_eq!(q.start_page, 5);
}

#[test]
fn ambiguous_mentions_rebuild_stability_and_tamper_rejection() {
    let tmp = TempDir::new().unwrap();
    let env = ResearchEnv::new(tmp.path());
    let ledger = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let sid = "s-v17-ambig";

    let pdf_bytes = build_pdf(
        &[PageSpec::single_column(&[
            "1. Section One",
            "Lemma 1. First duplicate lemma.",
            "u + v = 1 (1)",
            "Proposition 3. Unique helper proposition.",
            "2. Section Two",
            "Lemma 1. Second duplicate lemma.",
            "p + q = 2 (1)",
            "Theorem 2. Uses Lemma 1, Eq. (1), and Proposition 3.",
            "Proof of Lemma 1. Cannot know which Lemma 1 this proves. Q.E.D.",
        ])],
        &PdfBuildOptions::default(),
    );

    let ing = ingest_bytes(
        &env.sandbox(),
        &ledger,
        &IngestRequest {
            session_id: sid,
            file_name: "ambig.pdf",
            origin: "upload:ambig.pdf",
            raw_bytes: &pdf_bytes,
            source_id_hint: Some("ambig"),
            section_filter: None,
            title: None,
        },
    )
    .unwrap();
    let events = ledger.lock().unwrap().read_session(sid).unwrap();
    let receipt = events.iter().find(|e| e.seq == ing.event_seq).unwrap();
    let pk = PaperKey::from_file_bytes(&pdf_bytes);

    let (out1, _) = build_pdf_skeleton_from_ingested(&env.sandbox(), sid, receipt, pk.clone(), None).unwrap();
    let (out2, _) = build_pdf_skeleton_from_ingested(&env.sandbox(), sid, receipt, pk.clone(), None).unwrap();
    assert_common_pdf_invariants(&out1);

    assert_eq!(
        serde_json::to_string(&out1.skeleton).unwrap(),
        serde_json::to_string(&out2.skeleton).unwrap()
    );

    let sk = &out1.skeleton;
    let prop3 = sk.nodes.iter().find(|n| n.kind == SkNodeKind::Proposition).unwrap();
    let thm2 = sk.nodes.iter().find(|n| n.kind == SkNodeKind::Theorem).unwrap();
    let pf = sk.nodes.iter().find(|n| n.kind == SkNodeKind::Proof).unwrap();

    assert!(pf.proof_of.is_none());
    assert_eq!(sk.edges.iter().filter(|e| e.kind == SkEdgeKind::Uses).count(), 1);
    assert!(sk.edges.iter().any(|e| {
        e.from == prop3.tag.as_deref().unwrap()
            && e.to == thm2.tag.as_deref().unwrap()
            && e.kind == SkEdgeKind::Uses
            && e.rule == SkEdgeRule::TextMention
    }));

    let disk = env
        .sandbox()
        .resolve_existing(
            research_tools::sandbox::SandboxRoot::Sources,
            sid,
            ing.primary_path.as_deref().unwrap(),
            false,
        )
        .unwrap()
        .canonical_target;
    fs::write(&disk, "tampered reading text\n").unwrap();
    let err = build_pdf_skeleton_from_ingested(&env.sandbox(), sid, receipt, pk, None)
        .expect_err("tampered reading file must fail closed");
    assert!(err.contains("hash mismatch"), "unexpected error: {err}");
}

#[test]
fn receipt_loader_fails_closed_on_missing_page_map_or_invalid_receipt_fields() {
    let tmp = TempDir::new().unwrap();
    let env = ResearchEnv::new(tmp.path());
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-v17-failclosed";

    let src_id = "bad-1234567890abcdef";
    let rel_path = format!("{src_id}/paper.pdf.extracted.txt");
    let dir = tmp.path().join("sessions").join(sid).join("sources").join(src_id);
    fs::create_dir_all(&dir).unwrap();
    let body = "1. Intro\nTheorem 1. Text.";
    fs::write(dir.join("paper.pdf.extracted.txt"), body).unwrap();
    let sha = ledger::sha256_hex(body.as_bytes());
    let pk = PaperKey::from_file_bytes(body.as_bytes());

    // 1. Pre-F14a receipt missing BOTH `pdf_pages` and `pdf_page_lines` must fail closed.
    let seq1 = ledger
        .append(
            sid,
            "source.ingested",
            1,
            &serde_json::json!({
                "source_id": src_id,
                "format": "pdf",
                "status": "ok",
                "primary_path": rel_path,
                "files": [rel_path],
                "files_sha256": { rel_path.clone(): sha.clone() },
                "page_count": 1,
            }),
        )
        .unwrap();
    // 2. Receipt where `primary_path` is omitted from `files` must fail closed.
    let seq2 = ledger
        .append(
            sid,
            "source.ingested",
            1,
            &serde_json::json!({
                "source_id": src_id,
                "format": "pdf",
                "status": "ok",
                "primary_path": rel_path,
                "files": [],
                "files_sha256": { rel_path.clone(): sha.clone() },
                "page_count": 1,
                "pdf_page_lines": [[1, 1, 2]],
            }),
        )
        .unwrap();
    // 3. Receipt with out-of-bounds `pdf_page_lines` (`last_line > line_count`) must fail closed.
    let seq3 = ledger
        .append(
            sid,
            "source.ingested",
            1,
            &serde_json::json!({
                "source_id": src_id,
                "format": "pdf",
                "status": "ok",
                "primary_path": rel_path,
                "files": [rel_path],
                "files_sha256": { rel_path.clone(): sha },
                "page_count": 1,
                "pdf_page_lines": [[1, 1, 99]],
            }),
        )
        .unwrap();

    let events = ledger.read_session(sid).unwrap();
    let e1 = events.iter().find(|e| e.seq == seq1).unwrap();
    let e2 = events.iter().find(|e| e.seq == seq2).unwrap();
    let e3 = events.iter().find(|e| e.seq == seq3).unwrap();

    assert!(build_pdf_skeleton_from_ingested(&env.sandbox(), sid, e1, pk.clone(), None)
        .unwrap_err()
        .contains("pdf_page_lines"));
    assert!(build_pdf_skeleton_from_ingested(&env.sandbox(), sid, e2, pk.clone(), None)
        .unwrap_err()
        .contains("not listed in receipt `files`"));
    assert!(build_pdf_skeleton_from_ingested(&env.sandbox(), sid, e3, pk, None)
        .unwrap_err()
        .contains("invalid line range"));
}

