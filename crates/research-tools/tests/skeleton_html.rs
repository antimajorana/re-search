//! Integration tests for Investigate V18 (`html-1` HTML5 / LaTeXML skeleton builder).
//! All fixtures under `tests/fixtures/skeleton_html/` are self-authored synthetic HTML.

use ledger::Ledger;
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::paper_key::PaperKey;
use research_tools::quote_core::verify_quote;
use research_tools::sandbox::SandboxRoot;
use research_tools::skeleton::{
    build_html_skeleton, build_html_skeleton_from_ingested, is_html_receipt,
    is_latexml_html_document, load_ingested_html_source, locate_in_skeleton,
    HtmlSkeletonInput, HtmlSkeletonOutput, LocateStatus, NumberBasis, SkConfidence, SkEdgeKind,
    SkEdgeRole, SkEdgeRule, SkNodeKind, SkeletonBuilder, SkeletonQuality,
    MAX_SKELETON_JSON_BYTES, MAX_SKELETON_NODES,
};
use research_tools::ResearchEnv;
use std::fs;
use std::sync::Mutex;
use tempfile::TempDir;

const FIXTURE_LATEXML: &str = include_str!("fixtures/skeleton_html/01_latexml_paper.html");
const FIXTURE_MALFORMED: &str = include_str!("fixtures/skeleton_html/02_malformed_recovery.html");
const FIXTURE_NOT_LATEXML: &str = include_str!("fixtures/skeleton_html/03_not_latexml_error.html");

fn assert_common_html_invariants(out: &HtmlSkeletonOutput) {
    let sk = &out.skeleton;
    assert_eq!(sk.builder, SkeletonBuilder::Html1);
    for (i, node) in sk.nodes.iter().enumerate() {
        assert_eq!(node.order, i);
        assert_eq!(node.confidence, SkConfidence::HtmlClass);
        assert!(node.tag.as_deref().is_some_and(|t| t.starts_with("n-")), "missing tag on {i}");
        assert_eq!(node.fp.len(), 12);
        assert_eq!(
            node.number_basis,
            node.number.as_ref().map(|_| NumberBasis::Html),
            "number_basis mismatch on {:?}",
            node.title
        );
        if let Some(p_idx) = out.parent_orders[i] {
            let parent = &sk.nodes[p_idx];
            assert_eq!(node.parent, parent.tag);
            assert!(
                parent.spans[0].contains(&node.spans[0]),
                "child {:?} ({:?}) outside parent {:?} ({:?})",
                node.kind,
                node.spans[0],
                parent.kind,
                parent.spans[0]
            );
        }
    }
    for p in sk.nodes.iter().filter(|n| n.kind == SkNodeKind::Proof) {
        let proves = sk
            .edges
            .iter()
            .filter(|e| e.kind == SkEdgeKind::Proves && Some(e.from.as_str()) == p.tag.as_deref())
            .count();
        assert!(proves <= 1, "rule 6.2 violated for proof {:?}", p.tag);
    }
    for e in &sk.edges {
        assert_ne!(e.from, e.to);
    }
}

#[test]
fn latexml_paper_builds_full_html1_skeleton_verifies_quotes_and_locates() {
    let tmp = TempDir::new().unwrap();
    let env = ResearchEnv::new(tmp.path());
    let ledger = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let sid = "s-v18-html1";

    assert!(is_latexml_html_document(FIXTURE_LATEXML));
    let ing = ingest_bytes(
        &env.sandbox(),
        &ledger,
        &IngestRequest {
            session_id: sid,
            file_name: "2601.00099.html",
            origin: "https://arxiv.org/html/2601.00099v1",
            raw_bytes: FIXTURE_LATEXML.as_bytes(),
            source_id_hint: Some("htmlpaper"),
            section_filter: None,
            title: None,
        },
    )
    .unwrap();
    assert_eq!(ing.format, "html");

    let events = ledger.lock().unwrap().read_session(sid).unwrap();
    let receipt = events.iter().find(|e| e.seq == ing.event_seq).unwrap();
    assert!(is_html_receipt(receipt));

    let pk = PaperKey::from_arxiv("2601.00099").unwrap();
    let (out, timing) =
        build_html_skeleton_from_ingested(&env.sandbox(), sid, receipt, pk, Some("v1".into()))
            .unwrap();
    assert_common_html_invariants(&out);
    assert!(timing.node_count > 0);
    assert_eq!(
        out.document_title.as_deref(),
        Some("Synthetic Spectral Bounds on Weighted Lattices")
    );

    let sk = &out.skeleton;
    assert_eq!(sk.quality, SkeletonQuality::Full, "notes: {:?}", out.quality_notes);

    let find_kind_num = |kind: SkNodeKind, num: &str| {
        sk.nodes
            .iter()
            .find(|n| n.kind == kind && n.number.as_deref() == Some(num))
            .unwrap_or_else(|| panic!("missing {kind:?} {num}"))
    };
    let abs = sk.nodes.iter().find(|n| n.kind == SkNodeKind::FrontMatter).unwrap();
    let sec1 = find_kind_num(SkNodeKind::Section, "1");
    let sec2 = find_kind_num(SkNodeKind::Section, "2");
    let sec21 = find_kind_num(SkNodeKind::Section, "2.1");
    let app_a = find_kind_num(SkNodeKind::Section, "A");
    let def11 = find_kind_num(SkNodeKind::Definition, "1.1");
    let eq11 = find_kind_num(SkNodeKind::Equation, "1.1");
    let lem21 = find_kind_num(SkNodeKind::Lemma, "2.1");
    let thm22 = find_kind_num(SkNodeKind::Theorem, "2.2");
    let fig1 = find_kind_num(SkNodeKind::Figure, "1");
    let rem = sk.nodes.iter().find(|n| n.kind == SkNodeKind::Remark).unwrap();
    let proofs: Vec<_> = sk.nodes.iter().filter(|n| n.kind == SkNodeKind::Proof).collect();

    assert_eq!(abs.title.as_deref(), Some("Abstract"));
    assert_eq!(sec1.title.as_deref(), Some("Setup & Notation"));
    assert_eq!(sec1.eq_count, 1, "unnumbered display equation inside section 1");
    assert_eq!(sec21.parent, sec2.tag);
    assert_eq!(app_a.title.as_deref(), Some("Appendix Notes"));
    assert_eq!(def11.title.as_deref(), Some("Spectral weight"));
    assert_eq!(lem21.title.as_deref(), Some("Coercivity bound"));
    assert_eq!(thm22.head_cites, vec!["Kel20"]);
    assert!(thm22.intro_refs, "Abstract refers to Theorem 2.2");
    assert_eq!(rem.number, None);
    assert_eq!(sk.nodes.iter().filter(|n| n.kind == SkNodeKind::Equation).count(), 1);
    assert!(!sk.nodes.iter().any(|n| n.number.as_deref() == Some("99")), "script decoy leaked");
    assert_eq!(proofs.len(), 2);
    assert_eq!(proofs[0].title, None);
    assert_eq!(proofs[0].proof_of, lem21.tag);
    assert_eq!(proofs[1].title.as_deref(), Some("Proof of Theorem 2.2"));
    assert_eq!(proofs[1].proof_of, thm22.tag);
    assert!(fig1.title.as_deref().is_some_and(|t| t.contains("Synthetic decay rate profile")));

    // Check MathML alttext deduplication & entity decoding (`≤`, `&`) via stats word count.
    assert!(sec1.words > 0 && sec1.words < 60, "unexpected sec1 word count: {}", sec1.words);

    // Check bibliography extraction.
    let kel = sk.bib.iter().find(|b| b.key == "Kel20").unwrap();
    assert_eq!(kel.arxiv_id.as_deref(), Some("2601.00055"));
    assert_eq!(kel.title.as_deref(), Some("Weighted Lattice Operators"));
    assert!(kel.locators.iter().any(|l| l.text == "Proposition 2.1"));
    let doe = sk.bib.iter().find(|b| b.key == "Doe21").unwrap();
    assert_eq!(doe.doi.as_deref(), Some("10.1000/synthetic.2021"));

    // Check edges: explicit refs, lifted proof eqref, head_cite, proof cite.
    let has_edge = |from: &str, to: &str, kind: SkEdgeKind, role: SkEdgeRole, rule: SkEdgeRule| {
        sk.edges.iter().any(|e| {
            e.from == from && e.to == to && e.kind == kind && e.role == role && e.rule == rule
        })
    };
    assert!(has_edge(
        def11.tag.as_deref().unwrap(),
        lem21.tag.as_deref().unwrap(),
        SkEdgeKind::Uses,
        SkEdgeRole::Statement,
        SkEdgeRule::Ref,
    ));
    assert!(has_edge(
        eq11.tag.as_deref().unwrap(),
        lem21.tag.as_deref().unwrap(),
        SkEdgeKind::Uses,
        SkEdgeRole::Proof,
        SkEdgeRule::Eqref,
    ));
    assert!(has_edge(
        lem21.tag.as_deref().unwrap(),
        thm22.tag.as_deref().unwrap(),
        SkEdgeKind::Uses,
        SkEdgeRole::Proof,
        SkEdgeRule::Ref,
    ));
    assert!(has_edge(
        "Kel20",
        thm22.tag.as_deref().unwrap(),
        SkEdgeKind::Cites,
        SkEdgeRole::Statement,
        SkEdgeRule::HeadCite,
    ));
    assert!(has_edge(
        "Doe21",
        thm22.tag.as_deref().unwrap(),
        SkEdgeKind::Cites,
        SkEdgeRole::Proof,
        SkEdgeRule::Cite,
    ));

    // Rule 6.9 locator compatibility (`LocateStatus::Found` with `NumberBasis::Html`).
    for (loc, expected) in [
        ("Theorem 2.2", thm22),
        ("Lemma 2.1", lem21),
        ("Eq. (1.1)", eq11),
        ("Section 2.1", sec21),
        ("Figure 1", fig1),
    ] {
        let res = locate_in_skeleton(sk, loc);
        assert_eq!(
            res.outcome,
            LocateStatus::Found {
                tag: expected.tag.clone().unwrap(),
                number_basis: NumberBasis::Html,
            },
            "failed locating `{loc}`"
        );
    }

    // F4 physical line verification (`quote_core::verify_quote`) against ingested HTML file.
    let vq = verify_quote(
        &env.sandbox(),
        sid,
        &events,
        &ing.source_id,
        &thm22.spans[0].path,
        thm22.spans[0].start_line,
        thm22.spans[0].end_line,
        "Synthetic spectral decay holds uniformly across weighted lattices.",
    )
    .expect("theorem span must verify raw HTML line slice via verify_quote");
    assert_eq!(vq.quote_ref.start_line, thm22.spans[0].start_line);

    let vq_eq = verify_quote(
        &env.sandbox(),
        sid,
        &events,
        &ing.source_id,
        &eq11.spans[0].path,
        eq11.spans[0].start_line,
        eq11.spans[0].end_line,
        r#"<span class="ltx_tag ltx_tag_equation">(1.1)</span>"#,
    )
    .expect("equation span must cover tag line");
    assert!(vq_eq.quote_ref.end_line >= eq11.spans[0].start_line);
}

#[test]
fn tags_invariant_across_crlf_comments_and_renumbering() {
    let pk = PaperKey::from_arxiv("2601.00099").unwrap();
    let base = build_html_skeleton(&HtmlSkeletonInput {
        source_id: "src-1".into(),
        source_seq: 1,
        paper_key: pk.clone(),
        version: None,
        primary_path: "src-1/paper.html".into(),
        html: FIXTURE_LATEXML.to_string(),
        files_sha256: None,
    });

    let mutated = FIXTURE_LATEXML
        .replace('\n', "\r\n")
        .replace("<!-- Comment ignored for fingerprint invariance -->", "<!-- Extra comment -->\r\n<!-- Two -->")
        .replace("Theorem 2.2", "Theorem 9.9")
        .replace("ltx_tag_equation\">(1.1)<", "ltx_tag_equation\">(9.9)<");
    let renum = build_html_skeleton(&HtmlSkeletonInput {
        source_id: "src-1".into(),
        source_seq: 1,
        paper_key: pk,
        version: None,
        primary_path: "src-1/paper.html".into(),
        html: mutated,
        files_sha256: None,
    });

    let tag_of = |out: &HtmlSkeletonOutput, kind: SkNodeKind| {
        out.skeleton
            .nodes
            .iter()
            .find(|n| n.kind == kind)
            .and_then(|n| n.tag.clone())
    };
    assert_eq!(tag_of(&base, SkNodeKind::Definition), tag_of(&renum, SkNodeKind::Definition));
    assert_eq!(tag_of(&base, SkNodeKind::Theorem), tag_of(&renum, SkNodeKind::Theorem));
    assert_eq!(tag_of(&base, SkNodeKind::Lemma), tag_of(&renum, SkNodeKind::Lemma));
    assert_eq!(tag_of(&base, SkNodeKind::Equation), tag_of(&renum, SkNodeKind::Equation));
}

#[test]
fn malformed_html5_recovers_with_partial_quality_and_valid_spans() {
    let pk = PaperKey::from_arxiv("2601.00077").unwrap();
    let out = build_html_skeleton(&HtmlSkeletonInput {
        source_id: "src-m".into(),
        source_seq: 2,
        paper_key: pk,
        version: None,
        primary_path: "src-m/broken.html".into(),
        html: FIXTURE_MALFORMED.to_string(),
        files_sha256: None,
    });
    assert_common_html_invariants(&out);
    assert_eq!(out.skeleton.quality, SkeletonQuality::Partial);
    assert!(!out.quality_notes.is_empty());

    let line_count = FIXTURE_MALFORMED.lines().count();
    for node in &out.skeleton.nodes {
        assert!(node.spans[0].start_line >= 1 && node.spans[0].end_line <= line_count);
    }
    assert!(out.skeleton.nodes.iter().any(|n| n.kind == SkNodeKind::Lemma && n.number.as_deref() == Some("1.1")));
    assert!(out.skeleton.nodes.iter().any(|n| n.kind == SkNodeKind::Theorem && n.number.as_deref() == Some("2.1")));
}

#[test]
fn non_latexml_rejected_and_tampered_source_fails_hash_check() {
    assert!(!is_latexml_html_document(""));
    assert!(!is_latexml_html_document("   \n  "));
    assert!(!is_latexml_html_document("<html><body>plain\0nul</body></html>"));
    assert!(!is_latexml_html_document(FIXTURE_NOT_LATEXML));

    let pk = PaperKey::from_arxiv("2601.00055").unwrap();
    let weak = build_html_skeleton(&HtmlSkeletonInput {
        source_id: "src-err".into(),
        source_seq: 1,
        paper_key: pk,
        version: None,
        primary_path: "src-err/err.html".into(),
        html: FIXTURE_NOT_LATEXML.to_string(),
        files_sha256: None,
    });
    assert_eq!(weak.skeleton.quality, SkeletonQuality::Weak);
    assert!(weak.skeleton.nodes.is_empty());

    // Tamper verification on disk.
    let tmp = TempDir::new().unwrap();
    let env = ResearchEnv::new(tmp.path());
    let ledger = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let sid = "s-v18-tamper";
    let ing = ingest_bytes(
        &env.sandbox(),
        &ledger,
        &IngestRequest {
            session_id: sid,
            file_name: "paper.html",
            origin: "test:paper.html",
            raw_bytes: FIXTURE_LATEXML.as_bytes(),
            source_id_hint: None,
            section_filter: None,
            title: None,
        },
    )
    .unwrap();
    let events = ledger.lock().unwrap().read_session(sid).unwrap();
    let receipt = events.iter().find(|e| e.seq == ing.event_seq).unwrap();
    let p = ing.primary_path.as_deref().unwrap();
    let disk = env.sandbox().resolve_existing(SandboxRoot::Sources, sid, p, false).unwrap().canonical_target;
    fs::write(&disk, "tampered").unwrap();

    let err = load_ingested_html_source(&env.sandbox(), sid, receipt).unwrap_err();
    assert!(err.contains("hash mismatch"), "unexpected error: {err}");
}

#[test]
fn caps_truncate_excess_depth_nodes_and_json_cleanly() {
    let pk = PaperKey::from_arxiv("2601.00088").unwrap();

    // 1. Nesting depth cap (>128).
    let mut deep = String::from(
        "<article class=\"ltx_document\"><section class=\"ltx_section\"><h2 class=\"ltx_title\">Top</h2></section>",
    );
    for _ in 0..145 {
        deep.push_str("<div>");
    }
    deep.push_str("<section class=\"ltx_section\"><h2 class=\"ltx_title\">Deep</h2></section>");
    for _ in 0..145 {
        deep.push_str("</div>");
    }
    deep.push_str("</article>");
    let deep_out = build_html_skeleton(&HtmlSkeletonInput {
        source_id: "src-deep".into(),
        source_seq: 1,
        paper_key: pk.clone(),
        version: None,
        primary_path: "src-deep/deep.html".into(),
        html: deep,
        files_sha256: None,
    });
    assert_eq!(deep_out.skeleton.quality, SkeletonQuality::Partial);
    assert!(deep_out.quality_notes.iter().any(|n| n.contains("nesting depth")));

    // 2. Node limit (> MAX_SKELETON_NODES).
    let mut many_nodes = String::from("<article class=\"ltx_document\">");
    for i in 0..6_050 {
        many_nodes.push_str(&format!(
            "<div class=\"ltx_theorem ltx_theorem_remark\" id=\"R{i}\"><h6 class=\"ltx_title\">R{i}</h6></div>\n"
        ));
    }
    many_nodes.push_str("</article>");
    let nodes_out = build_html_skeleton(&HtmlSkeletonInput {
        source_id: "src-many".into(),
        source_seq: 1,
        paper_key: pk.clone(),
        version: None,
        primary_path: "src-many/many.html".into(),
        html: many_nodes,
        files_sha256: None,
    });
    assert_eq!(nodes_out.skeleton.quality, SkeletonQuality::Partial);
    assert_eq!(nodes_out.skeleton.nodes.len(), MAX_SKELETON_NODES);
    assert!(serde_json::to_vec(&nodes_out.skeleton).unwrap().len() <= MAX_SKELETON_JSON_BYTES);

    // 3. JSON byte limit (< 6000 nodes, huge bibliography exceeding 2 MiB serialized JSON).
    let pad = "word ".repeat(240);
    let mut big_json = String::from("<article class=\"ltx_document\"><section class=\"ltx_bibliography\"><ul class=\"ltx_biblist\">");
    for i in 0..2_400 {
        big_json.push_str(&format!(
            "<li class=\"ltx_bibitem\" id=\"bib.b{i}\"><span class=\"ltx_tag\">[{i}]</span> {pad}</li>\n"
        ));
    }
    big_json.push_str("</ul></section></article>");
    let json_out = build_html_skeleton(&HtmlSkeletonInput {
        source_id: "src-json".into(),
        source_seq: 1,
        paper_key: pk,
        version: None,
        primary_path: "src-json/json.html".into(),
        html: big_json,
        files_sha256: None,
    });
    assert_eq!(json_out.skeleton.quality, SkeletonQuality::Partial);
    let serialized_len = serde_json::to_vec(&json_out.skeleton).unwrap().len();
    assert!(
        serialized_len <= MAX_SKELETON_JSON_BYTES,
        "serialized JSON size {serialized_len} exceeded {MAX_SKELETON_JSON_BYTES}"
    );
}
