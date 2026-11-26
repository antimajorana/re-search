//! Unit tests for `skeleton/pdf.rs` and its private PDF helper modules (Investigate V17).

use super::*;
use crate::pdf_pages::{PageQualityMetrics, PageRecord};
use crate::skeleton::model::{EdgeConfidence, NumberBasis, SkConfidence, SkEdgeKind, SkEdgeRole, SkEdgeRule};

fn mk_metrics(chars: usize) -> PageQualityMetrics {
    PageQualityMetrics {
        char_count: chars,
        alpha_count: chars,
        word_count: chars / 5,
        image_coverage_pct: 0,
        has_invisible_text: false,
        width_pt: 612,
        height_pt: 792,
    }
}

fn page_map_from_pages(total: usize, pages: Vec<(usize, PageBasis, &str)>) -> (String, PageMap) {
    let mut text = String::new();
    let mut recs = Vec::new();
    let mut cur = 1usize;
    for (idx, (page, basis, body)) in pages.into_iter().enumerate() {
        if idx > 0 {
            text.push_str("\n\x0c\n");
            cur += 1;
        }
        let (first, last) = if body.is_empty() {
            cur += 1;
            (None, None)
        } else {
            let cnt = body.split('\n').count();
            let f = cur;
            let l = f + cnt - 1;
            text.push_str(body);
            cur = l + 1;
            (Some(f), Some(l))
        };
        recs.push(PageRecord {
            page,
            basis,
            first_line: first,
            last_line: last,
            metrics: mk_metrics(body.len()),
            reason: None,
        });
    }
    (text, PageMap { page_count: total, pages: recs })
}

fn base_input(text: String, page_map: PageMap) -> PdfSkeletonInput {
    PdfSkeletonInput {
        source_id: "src-pdf-unit".into(),
        source_seq: 1,
        paper_key: PaperKey::parse("doi:10.1000/unit.v17").unwrap(),
        version: None,
        primary_path: "src-pdf-unit/paper.pdf.extracted.txt".into(),
        text,
        page_map,
        extractor_id: Some("pdf-extract-layout/0.12.1".into()),
        files_sha256: None,
        extra_quality_notes: Vec::new(),
    }
}

#[test]
fn footer_page_numbers_and_prose_sentences_never_become_nodes_or_locations() {
    let p1 = concat!(
        "Spectral Decay Note\n",
        "1. Introduction\n",
        "Theorem 2.1 shows that decay holds in year (2024).\n",
        "Setting x = y in Eq. (1) yields a bound.\n",
        "Proof techniques are standard.\n",
        "- 42 -",
    );
    let p2 = concat!(
        "2. Main Bound\n",
        "Theorem 2.1. Every positive contraction obeys\n",
        "||T x|| <= c ||x|| (1)\n",
        "Proof. Follow Eq. (1) directly. Q.E.D.\n",
        "(99)",
    );
    let (text, page_map) = page_map_from_pages(
        2,
        vec![(1, PageBasis::TextLayer, p1), (2, PageBasis::TextLayer, p2)],
    );
    let out = build_pdf_skeleton(&base_input(text, page_map));
    let sk = &out.skeleton;

    assert_eq!(sk.builder, SkeletonBuilder::Pdf1);
    assert_eq!(sk.quality, SkeletonQuality::Partial);
    assert!(sk.quality_reason.as_deref().unwrap().contains(PDF_QUALITY_REASON_BASE));
    assert!(!sk.quality_reason.as_deref().unwrap().contains(PDF_QUALITY_SCAN_TAG));

    // Only real Theorem 2.1 and Equation (1) exist (not prose "Theorem 2.1 shows", "(2024)", "in Eq. (1)", or footer "(99)").
    let thms: Vec<_> = sk.nodes.iter().filter(|n| n.kind == SkNodeKind::Theorem).collect();
    assert_eq!(thms.len(), 1);
    assert_eq!(thms[0].number.as_deref(), Some("2.1"));
    assert_eq!(thms[0].number_basis, Some(NumberBasis::Printed));
    assert_eq!(thms[0].confidence, SkConfidence::PdfHeuristic);
    assert_eq!(page_range_for_node(&out.page_map, thms[0]), Some((2, 2)));
    assert!(thms[0].intro_refs, "Introduction mentions Theorem 2.1");

    let eqs: Vec<_> = sk.nodes.iter().filter(|n| n.kind == SkNodeKind::Equation).collect();
    assert_eq!(eqs.len(), 1);
    assert_eq!(eqs[0].number.as_deref(), Some("1"));
    assert_eq!(page_range_for_node(&out.page_map, eqs[0]), Some((2, 2)));

    let proofs: Vec<_> = sk.nodes.iter().filter(|n| n.kind == SkNodeKind::Proof).collect();
    assert_eq!(proofs.len(), 1);
    assert_eq!(proofs[0].proof_of, thms[0].tag);
    assert!(sk.edges.iter().all(|e| e.confidence == EdgeConfidence::Heuristic));
}

#[test]
fn ambiguous_numbers_and_ambiguous_bib_keys_emit_zero_edges() {
    let p1 = concat!(
        "1. First Part\n",
        "Lemma 1. First lemma.\n",
        "x + y = 1 (1)\n",
        "2. Second Part\n",
        "Lemma 1. Duplicate numbered lemma.\n",
        "a + b = 2 (1)\n",
        "Theorem 2. By Lemma 1 and Eq. (1), see [1] and [2].\n",
        "Proof of Lemma 1. Ambiguous target proof. Q.E.D.\n",
        "References\n",
        "[1] First ref one. doi:10.1000/dup.a\n",
        "[1] Duplicate key one. doi:10.1000/dup.b\n",
        "[2] Clean ref two. arXiv:2301.01234v2",
    );
    let (text, page_map) = page_map_from_pages(1, vec![(1, PageBasis::TextLayer, p1)]);
    let out = build_pdf_skeleton(&base_input(text, page_map));
    let sk = &out.skeleton;

    // Duplicate Lemma 1 and Equation (1) must never create Uses edges or proof_of links.
    assert!(!sk.edges.iter().any(|e| e.kind == SkEdgeKind::Uses));
    assert!(!sk.edges.iter().any(|e| e.kind == SkEdgeKind::Proves));
    assert!(sk.nodes.iter().filter(|n| n.kind == SkNodeKind::Proof).all(|p| p.proof_of.is_none()));

    // Ambiguous bib key [1] emits no Cites edge; unambiguous [2] does.
    assert!(!sk.edges.iter().any(|e| e.kind == SkEdgeKind::Cites && e.from == "1"));
    assert!(sk.edges.iter().any(|e| {
        e.kind == SkEdgeKind::Cites
            && e.from == "2"
            && e.role == SkEdgeRole::Statement
            && e.rule == SkEdgeRule::Cite
            && e.confidence == EdgeConfidence::Heuristic
    }));
    let bib2 = sk.bib.iter().find(|b| b.key == "2").expect("bib entry 2");
    assert_eq!(bib2.arxiv_id.as_deref(), Some("2301.01234"));
}

#[test]
fn image_only_page_with_stray_footer_emits_no_nodes_and_remains_available_as_image() {
    let p1 = "1. Setup\nLemma 1.1. Let A > 0 be symmetric.\nA v = w (1.1)";
    let p2_stray = "2"; // Stray footer on an image-only scan page (`PageBasis::None`).
    let p3 = "2. Main Result\nTheorem 2.1. From Lemma 1.1 and Eq. (1.1), the bound holds.";
    let (text, page_map) = page_map_from_pages(
        3,
        vec![
            (1, PageBasis::ScanTextLayer, p1),
            (2, PageBasis::None, p2_stray),
            (3, PageBasis::TextLayer, p3),
        ],
    );
    let out = build_pdf_skeleton(&base_input(text.clone(), page_map.clone()));
    let sk = &out.skeleton;

    assert_eq!(sk.quality, SkeletonQuality::Partial);
    assert!(sk.quality_reason.as_deref().unwrap().contains(PDF_QUALITY_SCAN_TAG));
    assert_eq!(out.image_only_pages, vec![2]);
    assert_eq!(out.scan_text_pages, vec![1]);
    assert!(out.pages[1].image_available && !out.pages[1].has_text && !out.pages[1].render_math);

    // No node may map to image-only page 2.
    for node in &sk.nodes {
        let range = page_range_for_node(&out.page_map, node).expect("mapped page");
        assert!(range.0 != 2 && range.1 != 2, "node {:?} leaked onto image-only page 2", node.title);
    }

    // Scan quote on page 1 preserves OCR basis and disables math rendering.
    let q_scan = match_pdf_quote_for_display(
        "src-pdf-unit/paper.pdf.extracted.txt",
        &text,
        &page_map,
        "Let A > 0 be symmetric.",
    )
    .expect("scan quote matches");
    assert_eq!(q_scan.start_page, 1);
    assert!(q_scan.is_ocr);
    assert!(!q_scan.render_math);
    assert!(q_scan.page_image_available);
    assert!(q_scan.display_label.contains(PDF_OCR_QUOTE_LABEL));
    assert_eq!(q_scan.origin, PdfTextOrigin::ScanOcrText);

    // Stray text on image-only page 2 must NOT match as a confirmed quote.
    assert!(match_pdf_quote_for_display(
        "src-pdf-unit/paper.pdf.extracted.txt",
        &text,
        &page_map,
        "2",
    )
    .is_none_or(|m| m.start_page != 2));

    // Model image transcription can never be a matched anchor or rendered as math (rule 6.21).
    assert!(!PdfTextOrigin::ModelImageTranscription.can_be_matched_anchor());
    assert!(!PdfTextOrigin::ModelImageTranscription.render_formulas_as_math());
    assert_eq!(
        PdfTextOrigin::ModelImageTranscription.display_prefix(),
        PDF_MODEL_TRANSCRIPTION_LABEL
    );
}

#[test]
fn page_range_after_page_one_and_math_bracket_guards() {
    let p10 = concat!(
        "4. Asymptotics\n",
        "For x ∈ [1, 2] and f[1] = 0, see [7, Theorem 3.2].\n",
        "Theorem 4.1. Bounds follow from [7].\n",
        "10",
    );
    let p11 = concat!(
        "5. Discussion\n",
        "By Theorem 4.1 the tail decays.\n",
        "11",
    );
    let (text, page_map) = page_map_from_pages(
        20,
        vec![(10, PageBasis::TextLayer, p10), (11, PageBasis::TextLayer, p11)],
    );
    let out = build_pdf_skeleton(&base_input(text, page_map));
    let sk = &out.skeleton;

    let secs: Vec<_> = sk.nodes.iter().filter(|n| n.kind == SkNodeKind::Section).collect();
    assert_eq!(secs.len(), 2);
    assert_eq!(secs[0].number.as_deref(), Some("4"));
    assert_eq!(secs[1].number.as_deref(), Some("5"));
    assert_eq!(page_range_for_node(&out.page_map, secs[0]), Some((10, 10)));
    assert_eq!(page_range_for_node(&out.page_map, secs[1]), Some((11, 11)));

    // Math expressions `x ∈ [1, 2]` and `f[1]` must NOT produce bib entries or Cites edges.
    assert!(sk.bib.iter().all(|b| b.key == "7"));
    assert_eq!(sk.bib.len(), 1);
    assert_eq!(sk.bib[0].locators.len(), 1);
    assert_eq!(sk.bib[0].locators[0].text, "Theorem 3.2");
    assert!(sk.quality_reason.as_deref().unwrap().contains("10..=11 of 20"));
}

#[test]
fn crlf_and_rebuild_produce_byte_identical_tags_and_spans() {
    let lf = "1. Intro\nTheorem 1.1. A 50% bound holds.\n2. Next\nBy Theorem 1.1 we finish.";
    let crlf = lf.replace('\n', "\r\n");
    let (_, pm) = page_map_from_pages(1, vec![(1, PageBasis::TextLayer, lf)]);
    let a = build_pdf_skeleton(&base_input(lf.into(), pm.clone()));
    let a2 = build_pdf_skeleton(&base_input(lf.into(), pm.clone()));
    let b = build_pdf_skeleton(&base_input(crlf, pm));

    assert_eq!(
        serde_json::to_string(&a.skeleton).unwrap(),
        serde_json::to_string(&a2.skeleton).unwrap()
    );
    let tags_a: Vec<_> = a.skeleton.nodes.iter().map(|n| (&n.tag, &n.fp, &n.spans)).collect();
    let tags_b: Vec<_> = b.skeleton.nodes.iter().map(|n| (&n.tag, &n.fp, &n.spans)).collect();
    assert_eq!(tags_a, tags_b);
}

#[test]
fn nested_paren_titles_wrapped_dois_citation_ranges_and_plural_mentions() {
    let p1 = concat!(
        "1. Setup\n",
        "Lemma 1.1 (Bound on L^2(M)). Every u satisfies\n",
        "||u||^2 = E(u) (1)\n",
        "||v||^2 = F(v) (2)\n",
        "2. Main\n",
        "Theorem 2.1 (Spectral gap on H^1(Omega)). By Eqs. (1) and (2) and [1-3], the gap is positive.\n",
        "References\n",
        "1. A. First, \"First Paper,\" https://doi.org/10.1000/\n",
        "wrapped.doi (2020).\n",
        "2. B. Second, \"Preprint,\" doi:10.48550/arXiv.2402.09876 (2024).\n",
        "3. C. Third, \"Third Paper,\" math/0501234 (2005).",
    );
    let (text, page_map) = page_map_from_pages(1, vec![(1, PageBasis::TextLayer, p1)]);
    let out = build_pdf_skeleton(&base_input(text, page_map));
    let sk = &out.skeleton;

    let lem = sk.nodes.iter().find(|n| n.kind == SkNodeKind::Lemma).expect("lemma 1.1");
    let thm = sk.nodes.iter().find(|n| n.kind == SkNodeKind::Theorem).expect("theorem 2.1");
    assert_eq!(lem.title.as_deref(), Some("Bound on L^2(M)"));
    assert_eq!(thm.title.as_deref(), Some("Spectral gap on H^1(Omega)"));

    // Both Eqs. (1) and (2) produce TextMention Uses edges to Theorem 2.1.
    let eq_uses = sk
        .edges
        .iter()
        .filter(|e| {
            e.kind == SkEdgeKind::Uses
                && e.rule == SkEdgeRule::TextMention
                && e.to == thm.tag.as_deref().unwrap()
        })
        .count();
    assert_eq!(eq_uses, 2);

    // Dotted numbered bibliography parsed with wrapped DOI, 10.48550/arXiv, and old-style arXiv ID.
    assert_eq!(sk.bib.len(), 3);
    assert_eq!(sk.bib[0].doi.as_deref(), Some("10.1000/wrapped.doi"));
    assert_eq!(sk.bib[1].doi.as_deref(), Some("10.48550/arxiv.2402.09876"));
    assert_eq!(sk.bib[1].arxiv_id.as_deref(), Some("2402.09876"));
    assert_eq!(sk.bib[2].arxiv_id.as_deref(), Some("math/0501234"));

    // Citation range `[1-3]` expands to Cites edges from keys 1, 2, 3.
    let cited: Vec<_> = sk
        .edges
        .iter()
        .filter(|e| e.kind == SkEdgeKind::Cites && e.to == thm.tag.as_deref().unwrap())
        .map(|e| e.from.as_str())
        .collect();
    assert_eq!(cited, vec!["1", "2", "3"]);
}

#[test]
fn intervening_image_only_page_splits_node_spans_and_ambiguous_quotes_are_refused() {
    let p1 = concat!(
        "1. Continuous Argument\n",
        "Repeated phrase on multiple pages.\n",
        "Unique phrase on page one.",
    );
    let p2_none = "Unique phrase on page three."; // Stray OCR fragment on an image-only page!
    let p3 = concat!(
        "Theorem 1.1. Unique phrase on page three.\n",
        "Repeated phrase on multiple pages.",
    );
    let (text, page_map) = page_map_from_pages(
        3,
        vec![
            (1, PageBasis::TextLayer, p1),
            (2, PageBasis::None, p2_none),
            (3, PageBasis::TextLayer, p3),
        ],
    );
    let out = build_pdf_skeleton(&base_input(text.clone(), page_map.clone()));
    let sec1 = out
        .skeleton
        .nodes
        .iter()
        .find(|n| n.kind == SkNodeKind::Section)
        .expect("section 1");
    assert_eq!(sec1.spans.len(), 2, "Section 1 must split around image-only page 2");
    assert_eq!(node_page_basis(&out.page_map, sec1), Some(PageBasis::TextLayer));
    for sp in &sec1.spans {
        let r = page_range_for_span(&out.page_map, sp).expect("mapped span");
        assert!(r.0 != 2 && r.1 != 2, "span must not claim image-only page 2");
    }

    // Quote appearing on PageBasis::None page 2 AND TextLayer page 3 must skip page 2 and resolve to page 3.
    let q3 = match_pdf_quote_for_display(
        "src-pdf-unit/paper.pdf.extracted.txt",
        &text,
        &page_map,
        "Unique phrase on page three.",
    )
    .expect("must not be shadowed by image-only page 2");
    assert_eq!(q3.start_page, 3);

    // Quote appearing on BOTH page 1 and page 3 is ambiguous and must NOT confirm a location.
    assert!(
        match_pdf_quote_for_display(
            "src-pdf-unit/paper.pdf.extracted.txt",
            &text,
            &page_map,
            "Repeated phrase on multiple pages.",
        )
        .is_none(),
        "ambiguous multi-page quote must not return a confirmed page"
    );
}

#[test]
fn alternating_headers_numbered_references_wrapped_dot_doi_and_interval_guards() {
    let p1 = concat!(
        "Spectral Decay   1\n",
        "1. Introduction\n",
        "Observation 1. Strict bound holds:\n",
        "||T u|| < 1 (1)\n",
        "We set N = 4 in every finite dimension.\n",
        "(12)",
    );
    let p2 = concat!(
        "2   Spectral Decay\n",
        "Theorem 2.1\n",
        "shows why wrapped prose must not create a theorem.\n",
        "Theorem 2.2 [1] (Main Result). On [1, 2] × [3, 4], by Observation 1 and Eq. (1) and [2].\n",
        "5. References\n",
        "[1] A. One, doi:10.1103/PhysRevB.99.\n",
        "010101 (2019).\n",
        "[2] B. Two, arXiv:2305.01234 (2023).",
    );
    let (text, page_map) = page_map_from_pages(
        2,
        vec![(1, PageBasis::TextLayer, p1), (2, PageBasis::TextLayer, p2)],
    );
    let out = build_pdf_skeleton(&base_input(text, page_map));
    let sk = &out.skeleton;

    // Alternating running header "2   Spectral Decay" must NOT become Section 2.
    assert!(
        !sk.nodes
            .iter()
            .any(|n| n.kind == SkNodeKind::Section && n.number.as_deref() == Some("2")),
        "alternating page-number running header must not create Section 2"
    );
    // Prose "N = 4." above "(12)" must NOT create Equation (12); strict `<` equation `(1)` MUST match.
    let eq_nums: Vec<_> = sk
        .nodes
        .iter()
        .filter(|n| n.kind == SkNodeKind::Equation)
        .filter_map(|n| n.number.as_deref())
        .collect();
    assert_eq!(eq_nums, vec!["1"]);

    // Wrapped prose "Theorem 2.1\nshows..." must NOT create a Theorem 2.1 node.
    let thms: Vec<_> = sk.nodes.iter().filter(|n| n.kind == SkNodeKind::Theorem).collect();
    assert_eq!(thms.len(), 1);
    assert_eq!(thms[0].number.as_deref(), Some("2.2"));
    assert_eq!(thms[0].title.as_deref(), Some("Main Result"));
    assert_eq!(thms[0].head_cites, vec!["1"]);

    // Numbered "5. References" parsed as Bibliography with dot-wrapped DOI, and `[1, 2] × [3, 4]`
    // did not generate bogus citation to `[1]` in body (only HeadCite `[1]` and body Cite `[2]`).
    assert!(sk.nodes.iter().any(|n| n.kind == SkNodeKind::Bibliography));
    assert_eq!(sk.bib[0].doi.as_deref(), Some("10.1103/physrevb.99.010101"));
    assert_eq!(sk.bib[1].arxiv_id.as_deref(), Some("2305.01234"));
    assert!(!sk.edges.iter().any(|e| e.kind == SkEdgeKind::Cites && e.rule == SkEdgeRule::Cite && e.from == "1"));
    assert!(sk.edges.iter().any(|e| e.kind == SkEdgeKind::Cites && e.rule == SkEdgeRule::HeadCite && e.from == "1"));
    assert!(sk.edges.iter().any(|e| e.kind == SkEdgeKind::Cites && e.rule == SkEdgeRule::Cite && e.from == "2"));

    // Observation 1 mention resolves cleanly.
    let obs1 = sk.nodes.iter().find(|n| n.kind == SkNodeKind::Claim).expect("Observation 1");
    assert!(sk.edges.iter().any(|e| {
        e.from == obs1.tag.as_deref().unwrap()
            && e.to == thms[0].tag.as_deref().unwrap()
            && e.kind == SkEdgeKind::Uses
    }));
}

#[test]
fn node_cap_and_json_cap_truncate_safely_without_promoting_to_full() {
    // Generate > 6,300 numbered equations (exceeding MAX_SKELETON_NODES even after 1900..=2099 year filter).
    let mut lines = vec!["1. Stress Section".to_string()];
    for i in 1..=6_300 {
        lines.push(format!("x_{i} = y_{i} + z_{i} ({i})"));
    }
    let body = lines.join("\n");
    let (text, page_map) = page_map_from_pages(1, vec![(1, PageBasis::TextLayer, &body)]);
    let (out, timing) = build_pdf_skeleton_timed(&base_input(text, page_map));
    assert_ne!(out.skeleton.quality, SkeletonQuality::Full);
    assert!(out.skeleton.nodes.len() <= crate::skeleton::MAX_SKELETON_NODES);
    assert!(timing.json_bytes <= crate::skeleton::MAX_SKELETON_JSON_BYTES);
    assert!(out
        .skeleton
        .quality_reason
        .as_deref()
        .unwrap()
        .contains("MAX_SKELETON_NODES"));

    // Huge bibliography alone exceeding 2 MiB must also truncate to <= MAX_SKELETON_JSON_BYTES.
    let mut bib_lines = vec!["References".to_string()];
    let filler = "A".repeat(1_500);
    for i in 1..=1_600 {
        bib_lines.push(format!("[{i}] Author {i}, \"{filler}\" (2020)."));
    }
    let bib_body = bib_lines.join("\n");
    let (b_text, b_pm) = page_map_from_pages(1, vec![(1, PageBasis::TextLayer, &bib_body)]);
    let (b_out, b_timing) = build_pdf_skeleton_timed(&base_input(b_text, b_pm));
    assert_ne!(b_out.skeleton.quality, SkeletonQuality::Full);
    assert!(b_timing.json_bytes <= crate::skeleton::MAX_SKELETON_JSON_BYTES);
    assert!(b_out
        .skeleton
        .quality_reason
        .as_deref()
        .unwrap()
        .contains("exceeded"));
}


