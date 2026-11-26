use super::*;
use crate::paper_key::PaperKey;
use crate::pdf_pages::{PageQualityMetrics, PageRecord};
use crate::skeleton::{
    build_pdf_skeleton, build_tex_skeleton, PdfSkeletonInput, TexSkeletonInput,
};
use std::collections::BTreeMap;

fn build_tex(tex: &str, version: Option<&str>) -> crate::skeleton::TexSkeletonOutput {
    let mut files = BTreeMap::new();
    files.insert("main.tex".to_string(), tex.to_string());
    build_tex_skeleton(&TexSkeletonInput {
        source_id: "src-b".into(),
        source_seq: 1,
        paper_key: PaperKey::from_arxiv("2601.00055").unwrap(),
        version: version.map(str::to_string),
        files,
        primary_path: Some("main.tex".into()),
        files_sha256: None,
    })
}

fn dummy_metrics() -> PageQualityMetrics {
    PageQualityMetrics {
        char_count: 120,
        alpha_count: 90,
        word_count: 20,
        image_coverage_pct: 0,
        has_invisible_text: false,
        width_pt: 612,
        height_pt: 792,
    }
}

#[test]
fn parse_locator_variants_and_priority() {
    let cases = [
        ("Theorem 2.1", Some(SkNodeKind::Theorem), Some("2.1")),
        ("Thm. 3", Some(SkNodeKind::Theorem), Some("3")),
        ("Lemma 4.2", Some(SkNodeKind::Lemma), Some("4.2")),
        ("Prop. A.1", Some(SkNodeKind::Proposition), Some("A.1")),
        ("Corollary 1", Some(SkNodeKind::Corollary), Some("1")),
        ("Definition 2", Some(SkNodeKind::Definition), Some("2")),
        ("Eq. (3.4)", Some(SkNodeKind::Equation), Some("3.4")),
        ("(2.7)", Some(SkNodeKind::Equation), Some("2.7")),
        ("(S1)", Some(SkNodeKind::Equation), Some("S1")),
        ("Section 3", Some(SkNodeKind::Section), Some("3")),
        ("§ 3.2", Some(SkNodeKind::Section), Some("3.2")),
        ("Appendix A", Some(SkNodeKind::Section), Some("A")),
        ("Section 2, Theorem 2.1", Some(SkNodeKind::Theorem), Some("2.1")),
        ("Theorem 2.1(a)", Some(SkNodeKind::Theorem), Some("2.1")),
        ("see [B26, Lem. 9]", Some(SkNodeKind::Lemma), Some("9")),
        ("2.1", None, Some("2.1")),
        ("introduction", None, None),
        ("", None, None),
    ];
    for (raw, want_kind, want_num) in cases {
        let got = parse_locator(raw);
        assert_eq!(got.kind, want_kind, "kind mismatch for `{raw}`");
        assert_eq!(got.number.as_deref(), want_num, "number mismatch for `{raw}`");
    }
}

#[test]
fn single_shared_counter_tex_yields_found() {
    let tex = concat!(
        "\\documentclass{article}\n",
        "\\newtheorem{theorem}{Theorem}[section]\n",
        "\\newtheorem{lemma}[theorem]{Lemma}\n",
        "\\newtheorem*{remark}{Remark}\n",
        "\\begin{document}\n",
        "\\section{Preliminaries}\n",
        "\\begin{lemma}\\label{lem:p}Base estimate.\\end{lemma}\n",
        "\\section{Main Results}\n",
        "\\begin{theorem}[Spectral Bound]\\label{thm:main}Every gap is positive.\\end{theorem}\n",
        "\\begin{lemma}\\label{lem:follow}Consequence.\\end{lemma}\n",
        "\\begin{remark}Unnumbered note.\\end{remark}\n",
        "\\end{document}\n",
    );
    let out = build_tex(tex, Some("v2"));
    let files = BTreeMap::from([("main.tex".into(), tex.into())]);
    let drift = detect_tex_counter_drift(&files, Some(&out.theorem_table), &out.skeleton.nodes);
    assert!(drift.is_none(), "unexpected drift: {drift:?}");

    let res = locate_in_skeleton_with_context(
        &out.skeleton,
        "Theorem 2.1",
        &LocateContext { counter_drift_reason: drift.as_deref(), ..Default::default() },
    );
    assert_eq!(
        res.version_caveat.as_deref(),
        Some("numbering taken from arXiv v2; the journal version may differ")
    );
    match res.outcome {
        LocateStatus::Found { ref tag, number_basis } => {
            let node = out.skeleton.nodes.iter().find(|n| n.tag.as_deref() == Some(tag)).unwrap();
            assert_eq!(node.kind, SkNodeKind::Theorem);
            assert_eq!(node.number.as_deref(), Some("2.1"));
            assert_eq!(number_basis, NumberBasis::Computed);
        }
        other => panic!("expected Found for single shared counter, got {other:?}"),
    }
    let json = serde_json::to_string(&res).unwrap();
    assert!(!json.to_ascii_lowercase().contains("verified"));
}

#[test]
fn unshared_counters_and_setcounter_drift_must_yield_candidates_never_found() {
    let unshared = concat!(
        "\\documentclass{article}\n",
        "\\newtheorem{theorem}{Theorem}[section]\n",
        "\\newtheorem{lemma}{Lemma}[section]\n",
        "\\begin{document}\n",
        "\\section{First}\n",
        "\\begin{theorem}T1\\end{theorem}\n",
        "\\begin{lemma}L1\\end{lemma}\n",
        "\\section{Second}\n",
        "\\begin{theorem}T2\\end{theorem}\n",
        "\\begin{lemma}L2\\end{lemma}\n",
        "\\end{document}\n",
    );
    let out = build_tex(unshared, Some("v1"));
    let res = locate_in_skeleton(&out.skeleton, "Lemma 2.1");
    assert_eq!(res.outcome.status_str(), CITED_STATUS_CANDIDATES);
    if let LocateStatus::Candidates { candidates, .. } = res.outcome {
        assert!(!candidates.is_empty());
        assert_eq!(candidates[0].kind, SkNodeKind::Lemma);
        assert_eq!(candidates[0].number.as_deref(), Some("2.1"));
    } else {
        panic!("expected Candidates for unshared counters");
    }

    // Off-by-one \setcounter drift must NEVER yield Found even when skeleton numbers look contiguous.
    let shifted = concat!(
        "\\documentclass{article}\n",
        "\\newtheorem{lemma}{Lemma}\n",
        "\\begin{document}\n",
        "\\setcounter{lemma}{1}\n",
        "\\begin{lemma}Actually printed as Lemma 2.\\end{lemma}\n",
        "\\end{document}\n",
    );
    let out_shifted = build_tex(shifted, Some("v1"));
    let files = BTreeMap::from([("main.tex".into(), shifted.into())]);
    let drift = detect_tex_counter_drift(&files, Some(&out_shifted.theorem_table), &out_shifted.skeleton.nodes);
    assert!(drift.is_some());
    let res_shifted = locate_in_skeleton_with_context(
        &out_shifted.skeleton,
        "Lemma 1",
        &LocateContext { counter_drift_reason: drift.as_deref(), ..Default::default() },
    );
    assert_eq!(res_shifted.outcome.status_str(), CITED_STATUS_CANDIDATES);
}

#[test]
fn printed_tag_and_html_confirmed_basis_yield_found_while_unlabelled_eqs_yield_candidates() {
    let tex = concat!(
        "\\documentclass{article}\n",
        "\\begin{document}\n",
        "\\section{Eqs}\n",
        "\\begin{equation}a=1\\end{equation}\n",
        "\\begin{equation}\\label{eq:one}b=2\\end{equation}\n",
        "\\begin{equation}\\label{eq:s1}\\tag{S1}c=3\\end{equation}\n",
        "\\end{document}\n",
    );
    let out = build_tex(tex, Some("v1"));
    // Unlabelled display equation shifts computed equation counter -> Eq. (1) must be Candidates.
    let res_computed_eq = locate_in_skeleton(&out.skeleton, "Eq. (1)");
    assert_eq!(res_computed_eq.outcome.status_str(), CITED_STATUS_CANDIDATES);

    // Explicit \tag{S1} has NumberBasis::Printed -> Found!
    let res_printed = locate_in_skeleton(&out.skeleton, "Eq. (S1)");
    match res_printed.outcome {
        LocateStatus::Found { number_basis, .. } => assert_eq!(number_basis, NumberBasis::Printed),
        other => panic!("expected Found for printed \\tag{{S1}}, got {other:?}"),
    }

    // Confirmed / Html basis on non-PDF skeleton yields Found even if quality is Partial.
    let mut sk = out.skeleton.clone();
    sk.quality = SkeletonQuality::Partial;
    let eq_node = sk
        .nodes
        .iter_mut()
        .find(|n| n.kind == SkNodeKind::Equation && n.number.as_deref() == Some("1"))
        .unwrap();
    eq_node.number_basis = Some(NumberBasis::Html);
    let res_html = locate_in_skeleton(&sk, "(1)");
    match res_html.outcome {
        LocateStatus::Found { number_basis, .. } => assert_eq!(number_basis, NumberBasis::Html),
        other => panic!("expected Found for Html basis, got {other:?}"),
    }
}

#[test]
fn pdf_skeleton_always_yields_candidates_with_physical_pages_and_ocr_flags() {
    let text = concat!(
        "1 Introduction\n",
        "Some opening prose.\n",
        "\x0c\n",
        "Theorem 2.1. Every finite operator is bounded.\n",
        "Proof. Direct.\n",
        "\x0c\n",
        "Lemma 3.1. A scanned auxiliary lemma.\n",
    );
    let page_map = PageMap {
        page_count: 3,
        pages: vec![
            PageRecord { page: 1, basis: PageBasis::TextLayer, first_line: Some(1), last_line: Some(2), metrics: dummy_metrics(), reason: None },
            PageRecord { page: 2, basis: PageBasis::TextLayer, first_line: Some(4), last_line: Some(5), metrics: dummy_metrics(), reason: None },
            PageRecord { page: 3, basis: PageBasis::ScanTextLayer, first_line: Some(7), last_line: Some(7), metrics: dummy_metrics(), reason: Some("scan".into()) },
        ],
    };
    let out = build_pdf_skeleton(&PdfSkeletonInput {
        source_id: "src-pdf".into(),
        source_seq: 2,
        paper_key: PaperKey::from_doi("10.1000/pdf.paper").unwrap(),
        version: None,
        primary_path: "paper.pdf.txt".into(),
        text: text.into(),
        page_map: page_map.clone(),
        extractor_id: Some("pdf-extract-layout/0.12.1".into()),
        files_sha256: None,
        extra_quality_notes: Vec::new(),
    });

    // Exact match on text-layer page 2 MUST still be Candidates (never Found), with start_page = 2.
    let res_thm = locate_in_skeleton_with_context(
        &out.skeleton,
        "Theorem 2.1",
        &LocateContext { page_map: Some(&page_map), ..Default::default() },
    );
    match res_thm.outcome {
        LocateStatus::Candidates { candidates, .. } => {
            assert_eq!(candidates[0].kind, SkNodeKind::Theorem);
            assert_eq!(candidates[0].start_page, Some(2));
            assert!(!candidates[0].ocr);
            assert!(candidates[0].render_math);
        }
        other => panic!("PDF lookup must yield Candidates, got {other:?}"),
    }

    // Scanned page 3 candidate MUST carry ocr = true, render_math = false, start_page = 3.
    let res_lem = locate_in_skeleton_with_context(
        &out.skeleton,
        "Lemma 3.1",
        &LocateContext { page_map: Some(&page_map), ..Default::default() },
    );
    match res_lem.outcome {
        LocateStatus::Candidates { candidates, .. } => {
            assert_eq!(candidates[0].kind, SkNodeKind::Lemma);
            assert_eq!(candidates[0].start_page, Some(3));
            assert!(candidates[0].ocr);
            assert!(!candidates[0].render_math);
        }
        other => panic!("scanned PDF lookup must yield Candidates, got {other:?}"),
    }

    // Also verify compact page_lines fallback when full PageMap is absent.
    let compact = page_map.page_lines();
    let res_compact = locate_in_skeleton_with_context(
        &out.skeleton,
        "Theorem 2.1",
        &LocateContext { page_lines: Some(&compact), ..Default::default() },
    );
    match res_compact.outcome {
        LocateStatus::Candidates { candidates, .. } => assert_eq!(candidates[0].start_page, Some(2)),
        other => panic!("expected Candidates with page_lines, got {other:?}"),
    }
}

#[test]
fn kind_mismatch_missing_locator_and_empty_image_only_pdf() {
    let tex = concat!(
        "\\documentclass{article}\n",
        "\\newtheorem{theorem}{Theorem}[section]\n",
        "\\newtheorem{lemma}[theorem]{Lemma}\n",
        "\\begin{document}\n",
        "\\section{S}\n",
        "\\begin{lemma}Only a lemma.\\end{lemma}\n",
        "\\end{document}\n",
    );
    let out = build_tex(tex, Some("v1"));

    // Kind mismatch: asking for Theorem 1.1 when 1.1 is Lemma -> Candidates with Lemma 1.1 first.
    let mm = locate_in_skeleton(&out.skeleton, "Theorem 1.1");
    match mm.outcome {
        LocateStatus::Candidates { candidates, .. } => {
            assert_eq!(candidates[0].kind, SkNodeKind::Lemma);
            assert_eq!(candidates[0].number.as_deref(), Some("1.1"));
        }
        other => panic!("expected Candidates on kind mismatch, got {other:?}"),
    }

    // Missing locator -> PaperFoundLocatorNotFound with theorem-like manual candidates.
    let miss = locate_in_skeleton(&out.skeleton, "Theorem 99");
    match miss.outcome {
        LocateStatus::PaperFoundLocatorNotFound { candidates, .. } => {
            assert_eq!(candidates.len(), 1);
            assert_eq!(candidates[0].kind, SkNodeKind::Lemma);
        }
        other => panic!("expected PaperFoundLocatorNotFound, got {other:?}"),
    }

    // Empty image-only PDF -> NoTex with reason empty_text_layer.
    let empty_pm = PageMap {
        page_count: 1,
        pages: vec![PageRecord {
            page: 1,
            basis: PageBasis::None,
            first_line: None,
            last_line: None,
            metrics: dummy_metrics(),
            reason: Some("image only".into()),
        }],
    };
    let empty_pdf = build_pdf_skeleton(&PdfSkeletonInput {
        source_id: "src-scan".into(),
        source_seq: 3,
        paper_key: PaperKey::from_file_bytes(b"scan"),
        version: None,
        primary_path: "scan.pdf.txt".into(),
        text: String::new(),
        page_map: empty_pm.clone(),
        extractor_id: Some("pdf-extract-layout/0.12.1".into()),
        files_sha256: None,
        extra_quality_notes: Vec::new(),
    });
    let res_empty = locate_in_skeleton_with_context(
        &empty_pdf.skeleton,
        "Theorem 1",
        &LocateContext { page_map: Some(&empty_pm), ..Default::default() },
    );
    assert_eq!(
        res_empty.outcome,
        LocateStatus::NoTex { reason: NO_TEX_REASON_EMPTY_TEXT_LAYER.into() }
    );
}

#[test]
fn carbon_audit_regressions_scanner_ambiguous_locators_and_disjoint_sections() {
    // 1. Single-line \iffalse...\fi must not hide a real \setcounter after \fi,
    //    and \renewcommand*{\thetheorem} / \def\thelemma must be detected as counter drift.
    let tex_inline_fi = concat!(
        "\\documentclass{article}\n",
        "\\newtheorem{theorem}{Theorem}[section]\n",
        "\\begin{document}\n",
        "\\iffalse \\setcounter{theorem}{9} \\fi \\setcounter{theorem}{3}\n",
        "\\section{S}\n",
        "\\begin{theorem}Shifted.\\end{theorem}\n",
        "\\end{document}\n",
    );
    let out_fi = build_tex(tex_inline_fi, None);
    let files_fi = std::collections::BTreeMap::from([("main.tex".to_string(), tex_inline_fi.to_string())]);
    let drift_fi = detect_tex_counter_drift(&files_fi, None, &out_fi.skeleton.nodes);
    assert!(drift_fi.is_some(), "single-line \\iffalse...\\fi must not hide \\setcounter after \\fi");
    let res_fi = locate_in_skeleton_with_context(
        &out_fi.skeleton,
        "Theorem 1.1",
        &LocateContext { counter_drift_reason: drift_fi.as_deref(), ..Default::default() },
    );
    assert!(matches!(res_fi.outcome, LocateStatus::Candidates { .. }), "got {:?}", res_fi.outcome);

    let tex_star_def = concat!(
        "\\documentclass{article}\n",
        "\\newtheorem{theorem}{Theorem}\n",
        "\\renewcommand*{\\thetheorem}{\\Roman{theorem}}\n",
        "\\begin{document}\n",
        "\\begin{theorem}Roman.\\end{theorem}\n",
        "\\end{document}\n",
    );
    let out_star = build_tex(tex_star_def, None);
    let files_star = std::collections::BTreeMap::from([("main.tex".to_string(), tex_star_def.to_string())]);
    let drift_star = detect_tex_counter_drift(&files_star, None, &out_star.skeleton.nodes);
    assert!(drift_star.is_some(), "\\renewcommand*{{\\thetheorem}} must trigger counter drift");
    let res_star = locate_in_skeleton_with_context(
        &out_star.skeleton,
        "Theorem 1",
        &LocateContext { counter_drift_reason: drift_star.as_deref(), ..Default::default() },
    );
    assert!(matches!(res_star.outcome, LocateStatus::Candidates { .. }), "got {:?}", res_star.outcome);

    // 2. Prepositional / indirect and multi-statement locators must NEVER return Found.
    let tex_clean = concat!(
        "\\documentclass{article}\n",
        "\\newtheorem{theorem}{Theorem}[section]\n",
        "\\newtheorem{lemma}[theorem]{Lemma}\n",
        "\\begin{document}\n",
        "\\section{A}\n",
        "\\begin{theorem}\\label{t1}Main.\\end{theorem}\n",
        "\\begin{lemma}\\label{l2}Aux.\\end{lemma}\n",
        "\\end{document}\n",
    );
    let out_clean = build_tex(tex_clean, None);
    for raw in [
        "Corollary to Theorem 1.1",
        "Remark after Theorem 1.1",
        "Theorem 1.1, Lemma 1.2",
        "Theorem 1.1 and Lemma 1.2",
    ] {
        let r = locate_in_skeleton(&out_clean.skeleton, raw);
        assert!(
            matches!(r.outcome, LocateStatus::Candidates { .. }),
            "ambiguous locator `{raw}` must yield Candidates, got {:?}",
            r.outcome
        );
    }

    // 3. Disjoint sections with two unshared theorem-like kinds without single_counter_confirmed
    //    must NOT return Found even when numbers do not collide (Section 1 has Theorem 1.1, Section 2 has Lemma 2.1).
    let tex_disjoint = concat!(
        "\\documentclass{article}\n",
        "\\newtheorem{theorem}{Theorem}[section]\n",
        "\\newtheorem{lemma}{Lemma}[section]\n",
        "\\begin{document}\n",
        "\\section{One}\n",
        "\\begin{theorem}T1.\\end{theorem}\n",
        "\\section{Two}\n",
        "\\begin{lemma}L2.\\end{lemma}\n",
        "\\end{document}\n",
    );
    let out_disjoint = build_tex(tex_disjoint, None);
    let r_disj = locate_in_skeleton_with_context(
        &out_disjoint.skeleton,
        "Theorem 1.1",
        &LocateContext::default(),
    );
    assert!(
        matches!(r_disj.outcome, LocateStatus::Candidates { .. }),
        "disjoint-section unshared counters must yield Candidates, got {:?}",
        r_disj.outcome
    );
}
