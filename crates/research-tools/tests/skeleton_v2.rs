//! Integration tests for Investigate V2 skeleton enrichment (`edges`, `number`, `tags`,
//! `stats`, `bib`).
//!
//! Every expected edge, proof link, number, and bibliography entry below was derived by hand
//! from reading each fixture's `.tex` source files line by line.

use research_tools::paper_key::{node_tag, PaperKey};
use research_tools::skeleton::{
    build_tex_skeleton, build_tex_skeleton_timed, kind_class, EdgeConfidence, NumberBasis,
    SkEdgeKind, SkEdgeRole, SkEdgeRule, SkNodeKind, SkeletonQuality, Span, TexSkeletonInput,
    TexSkeletonOutput,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skeleton")
}

fn collect_tex_files(dir: &Path) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    fn rec(base: &Path, cur: &Path, out: &mut BTreeMap<String, String>) {
        for entry in std::fs::read_dir(cur).expect("read fixture dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                rec(base, &path, out);
            } else if matches!(path.extension().and_then(|e| e.to_str()), Some("tex" | "bib" | "bbl")) {
                let rel = path.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
                out.insert(rel, std::fs::read_to_string(&path).expect("read fixture file"));
            }
        }
    }
    rec(dir, dir, &mut files);
    files
}

fn build_fixture(rel_dir: &str, paper_key: &str) -> (TexSkeletonOutput, research_tools::skeleton::BuildTiming) {
    let dir = fixtures_root().join(rel_dir);
    let files = collect_tex_files(&dir);
    build_tex_skeleton_timed(&TexSkeletonInput {
        source_id: format!("src-v2-{}", rel_dir.replace('/', "-")),
        source_seq: 1,
        paper_key: PaperKey::parse(paper_key).expect("valid paper key"),
        version: Some("v1".into()),
        files,
        primary_path: None,
        files_sha256: None,
    })
}

/// Hand-specified expected edge using node order indices (or bib key for `Cites`).
#[derive(Debug, PartialEq, Eq)]
struct HandEdge {
    from: HandEndpoint,
    to_order: usize,
    kind: SkEdgeKind,
    role: SkEdgeRole,
    site: Span,
    rule: SkEdgeRule,
    confidence: EdgeConfidence,
}

#[derive(Debug, PartialEq, Eq)]
enum HandEndpoint {
    Node(usize),
    Bib(&'static str),
}

fn assert_hand_edges(out: &TexSkeletonOutput, expected: &[HandEdge], label: &str) {
    let tag_of = |ord: usize| out.skeleton.nodes[ord].tag.clone().expect("assigned tag");
    let got: Vec<_> = out
        .skeleton
        .edges
        .iter()
        .map(|e| {
            (
                e.from.clone(),
                e.to.clone(),
                e.kind,
                e.role,
                e.site.clone(),
                e.rule,
                e.confidence,
            )
        })
        .collect();
    let want: Vec<_> = expected
        .iter()
        .map(|e| {
            let from = match e.from {
                HandEndpoint::Node(o) => tag_of(o),
                HandEndpoint::Bib(k) => k.to_string(),
            };
            (from, tag_of(e.to_order), e.kind, e.role, e.site.clone(), e.rule, e.confidence)
        })
        .collect();
    assert_eq!(got, want, "{label}: edges mismatch");
}

fn assert_proof_links(out: &TexSkeletonOutput, expected: &[(usize, usize)], label: &str) {
    let mut actual = Vec::new();
    for (idx, node) in out.skeleton.nodes.iter().enumerate() {
        if let Some(target_tag) = node.proof_of.as_deref() {
            assert_eq!(node.kind, SkNodeKind::Proof, "{label}: non-proof node {idx} has proof_of");
            let target_ord = out
                .skeleton
                .nodes
                .iter()
                .position(|n| n.tag.as_deref() == Some(target_tag))
                .expect("proof_of tag exists");
            actual.push((idx, target_ord));
        }
    }
    assert_eq!(actual, expected, "{label}: proof_of links mismatch");
    // Rule 6.2: each proof proves at most one node.
    let mut seen_proofs = BTreeSet::new();
    for edge in out.skeleton.edges.iter().filter(|e| e.kind == SkEdgeKind::Proves) {
        assert!(seen_proofs.insert(edge.from.as_str()), "{label}: proof `{}` proves >1 node", edge.from);
    }
}

#[test]
fn hand_checked_v2_edges_proofs_bib_and_stats_on_all_v1_fixtures() {
    use EdgeConfidence::*;
    use HandEndpoint::*;
    use SkEdgeKind::*;
    use SkEdgeRole::*;
    use SkEdgeRule::*;

    // 01_amsthm_shared_counters
    let (out1, t1) = build_fixture("01_amsthm_shared_counters", "arxiv:2601.00001");
    assert_proof_links(&out1, &[(5, 4), (9, 7)], "01");
    assert_hand_edges(
        &out1,
        &[
            HandEdge { from: Node(5), to_order: 4, kind: Proves, role: Proof, site: Span::new("main.tex", 36, 38), rule: ProofAdjacent, confidence: Explicit },
            HandEdge { from: Node(9), to_order: 7, kind: Proves, role: Proof, site: Span::new("main.tex", 46, 48), rule: ProofAdjacent, confidence: Explicit },
            HandEdge { from: Bib("keller2020"), to_order: 4, kind: Cites, role: Statement, site: Span::new("main.tex", 30, 35), rule: HeadCite, confidence: Explicit },
        ],
        "01",
    );
    assert_eq!(out1.skeleton.bib.len(), 1);
    assert_eq!(out1.skeleton.bib[0].key, "keller2020");
    assert_eq!(out1.skeleton.bib[0].cited_at, vec![Span::new("main.tex", 30, 30)]);
    assert_eq!(out1.skeleton.bib[0].locators.len(), 1);
    assert_eq!(out1.skeleton.bib[0].locators[0].text, "Proposition 2.1");
    // Stats: top-level (abstract) + 2 sections.
    assert_eq!(out1.skeleton.stats.len(), 3);
    assert_eq!(out1.skeleton.stats[0].section_tag, None);
    assert_eq!(out1.skeleton.stats[1].section_tag, out1.skeleton.nodes[1].tag);
    assert_eq!(out1.skeleton.stats[2].section_tag, out1.skeleton.nodes[6].tag);
    assert!(out1.skeleton.stats.iter().all(|s| s.words > 0 && s.read_minutes > 0.0));

    // 02_custom_theorem_names
    let (out2, t2) = build_fixture("02_custom_theorem_names", "arxiv:2601.00002");
    assert_proof_links(&out2, &[(3, 2)], "02");
    assert_hand_edges(
        &out2,
        &[HandEdge { from: Node(3), to_order: 2, kind: Proves, role: Proof, site: Span::new("main.tex", 19, 21), rule: ProofAdjacent, confidence: Explicit }],
        "02",
    );
    assert!(out2.skeleton.bib.is_empty());

    // 03_multi_file
    let (out3, t3) = build_fixture("03_multi_file", "arxiv:2601.00003");
    assert_proof_links(&out3, &[(4, 3)], "03");
    assert_hand_edges(
        &out3,
        &[HandEdge { from: Node(4), to_order: 3, kind: Proves, role: Proof, site: Span::new("sections/sub_lemma.tex", 4, 6), rule: ProofAdjacent, confidence: Explicit }],
        "03",
    );
    assert_eq!(out3.skeleton.bib.len(), 1);
    assert_eq!(out3.skeleton.bib[0].key, "refchain");
    assert!(out3.skeleton.bib[0].raw.contains("Finite Chains and Edge States"));

    // 04_revtex_physics
    let (out4, t4) = build_fixture("04_revtex_physics", "arxiv:2601.00004");
    assert_proof_links(&out4, &[], "04");
    assert!(out4.skeleton.edges.is_empty());
    assert!(out4.skeleton.bib.is_empty());

    // 05_detached_proof
    let (out5, t5) = build_fixture("05_detached_proof", "arxiv:2601.00005");
    assert_proof_links(&out5, &[(5, 1), (6, 2), (7, 3), (10, 9)], "05");
    assert_hand_edges(
        &out5,
        &[
            HandEdge { from: Node(5), to_order: 1, kind: Proves, role: Proof, site: Span::new("main.tex", 19, 21), rule: ProofHeader, confidence: Explicit },
            HandEdge { from: Node(6), to_order: 2, kind: Proves, role: Proof, site: Span::new("main.tex", 22, 24), rule: ProofHeader, confidence: Heuristic },
            HandEdge { from: Node(7), to_order: 3, kind: Proves, role: Proof, site: Span::new("main.tex", 25, 27), rule: ProofHeader, confidence: Explicit },
            HandEdge { from: Node(10), to_order: 9, kind: Proves, role: Proof, site: Span::new("main.tex", 32, 34), rule: ProofAdjacent, confidence: Explicit },
            HandEdge { from: Bib("seneta2006"), to_order: 3, kind: Cites, role: Proof, site: Span::new("main.tex", 25, 27), rule: HeadCite, confidence: Explicit },
        ],
        "05",
    );
    assert_eq!(out5.skeleton.bib.len(), 1);
    assert_eq!(out5.skeleton.bib[0].key, "seneta2006");

    // 06_unlabelled_theorems
    let (out6, t6) = build_fixture("06_unlabelled_theorems", "arxiv:2601.00006");
    assert_proof_links(&out6, &[(4, 3)], "06");
    assert_ne!(out6.skeleton.nodes[5].tag, out6.skeleton.nodes[6].tag);
    assert_hand_edges(
        &out6,
        &[HandEdge { from: Node(4), to_order: 3, kind: Proves, role: Proof, site: Span::new("main.tex", 21, 23), rule: ProofAdjacent, confidence: Explicit }],
        "06",
    );

    // 07_two_versions (permanent tag stability across renumbering!)
    let (out7a, t7a) = build_fixture("07_two_versions/v1", "arxiv:2601.00007");
    let (out7b, t7b) = build_fixture("07_two_versions/v2", "arxiv:2601.00007");
    let tag_by_label = |out: &TexSkeletonOutput, lbl: &str| {
        out.skeleton.nodes.iter().find(|n| n.label.as_deref() == Some(lbl)).and_then(|n| n.tag.clone()).unwrap()
    };
    let num_by_label = |out: &TexSkeletonOutput, lbl: &str| {
        out.skeleton.nodes.iter().find(|n| n.label.as_deref() == Some(lbl)).and_then(|n| n.number.clone()).unwrap()
    };
    // Numbers shifted between v1 and v2, yet permanent tags stayed identical!
    assert_eq!(num_by_label(&out7a, "prop:gap"), "1.2");
    assert_eq!(num_by_label(&out7b, "prop:gap"), "1.3");
    assert_eq!(tag_by_label(&out7a, "prop:gap"), tag_by_label(&out7b, "prop:gap"));
    assert_eq!(num_by_label(&out7a, "thm:sup"), "1.3");
    assert_eq!(num_by_label(&out7b, "thm:sup"), "1.4");
    assert_eq!(tag_by_label(&out7a, "thm:sup"), tag_by_label(&out7b, "thm:sup"));
    assert_eq!(tag_by_label(&out7a, "thm:decay"), tag_by_label(&out7b, "thm:decay"));

    // 08_hard_edge_cases
    let (out8, t8) = build_fixture("08_hard_edge_cases", "arxiv:2601.00008");
    assert_proof_links(&out8, &[], "08");
    assert_hand_edges(
        &out8,
        &[HandEdge { from: Bib("hairer2006"), to_order: 2, kind: Cites, role: Statement, site: Span::new("main.tex", 35, 40), rule: HeadCite, confidence: Explicit }],
        "08",
    );
    assert_eq!(out8.skeleton.bib.len(), 1);
    assert_eq!(out8.skeleton.bib[0].key, "hairer2006");
    assert_eq!(out8.skeleton.bib[0].locators[0].text, "Theorem 3.2");

    for (name, t) in [
        ("01_amsthm_shared_counters", &t1),
        ("02_custom_theorem_names", &t2),
        ("03_multi_file", &t3),
        ("04_revtex_physics", &t4),
        ("05_detached_proof", &t5),
        ("06_unlabelled_theorems", &t6),
        ("07_two_versions/v1", &t7a),
        ("07_two_versions/v2", &t7b),
        ("08_hard_edge_cases", &t8),
    ] {
        eprintln!(
            "fixture timing {name}: lines={}, nodes={}, json_bytes={}, flatten={}us, scan={}us, total={}us",
            t.flattened_lines, t.node_count, t.json_bytes, t.flatten_us, t.scan_us, t.total_us
        );
    }
}

#[test]
fn cross_reference_paper_tests_knowtex_roles_lifted_edges_text_mentions_appendix_and_bib() {
    let main_tex = r#"\documentclass{article}
\usepackage{amsmath,amsthm,cleveref}
\numberwithin{equation}{section}
\newtheorem{theorem}{Theorem}[section]
\newtheorem{lemma}[theorem]{Lemma}
\newtheorem{definition}[theorem]{Definition}
\begin{document}
\begin{abstract}
We prove Theorem~\ref{thm:main} on decay of discrete correlations.
\end{abstract}
\section{Introduction}\label{sec:intro}
Our main estimate is established in \cref{thm:main}.
\section{Preliminaries}\label{sec:prelim}
\begin{definition}[Weighted energy]\label{def:energy}
Define the discrete energy functional
\begin{equation}\label{eq:energy}
E(f) = \sum_{e} w_e |df(e)|^2.
\end{equation}
\end{definition}
\begin{lemma}[Coercivity]\label{lem:coercive}
For mean-zero $f$, $E(f) \ge c \|f\|_2^2$ by Definition~\ref{def:energy}.
\end{lemma}
\begin{proof}
Follows from the spectral gap \cite[Theorem 4.1]{spectral2021}.
\end{proof}
\section{Main result}\label{sec:main}
\begin{theorem}[Exponential decay]\label{thm:main}
Under Definition~\ref{def:energy} and \eqref{eq:energy}, the semigroup decays exponentially.
\end{theorem}
\begin{proof}
Apply Lemma 2.2 and \eqref{eq:energy} to bound the time derivative \citep[see][Lemma 3]{decay2022}.
\end{proof}
\appendix
\section{Auxiliary identities}\label{sec:app}
\begin{lemma}[Summation by parts]\label{lem:sbp}
Discrete integration by parts holds on finite graphs.
\end{lemma}
\end{document}
"#;
    let refs_bib = r#"
@article{spectral2021,
  author = {A. Researcher},
  title = {Spectral {G}aps on {W}eighted {G}raphs},
  eprint = {2104.01234v2},
  doi = {10.1000/spectral.2021},
  year = {2021}
}
@article{decay2022,
  author = {B. Author},
  title = {Heat Kernel Decay Estimates},
  eprint = {arXiv:2205.05678},
  year = {2022}
}
"#;
    let mut files = BTreeMap::new();
    files.insert("main.tex".into(), main_tex.into());
    files.insert("refs.bib".into(), refs_bib.into());

    let pk = PaperKey::parse("arxiv:2601.00042").unwrap();
    let out = build_tex_skeleton(&TexSkeletonInput {
        source_id: "crossref".into(),
        source_seq: 1,
        paper_key: pk.clone(),
        version: Some("v1".into()),
        files,
        primary_path: Some("main.tex".into()),
        files_sha256: None,
    });

    let find = |lbl: &str| out.skeleton.nodes.iter().find(|n| n.label.as_deref() == Some(lbl)).expect(lbl);
    let def_e = find("def:energy");
    let eq_e = find("eq:energy");
    let lem_c = find("lem:coercive");
    let thm_m = find("thm:main");
    let sec_app = find("sec:app");
    let lem_sbp = find("lem:sbp");

    // Verify permanent tag matches paper_key::node_tag directly.
    assert_eq!(
        thm_m.tag.as_deref(),
        Some(node_tag(&pk, kind_class(SkNodeKind::Theorem), "label:thm:main").unwrap().as_str())
    );
    // Verify intro_refs (abstract + introduction both cite thm:main).
    assert!(thm_m.intro_refs);
    assert!(!lem_sbp.intro_refs);

    // Verify \numberwithin{equation}{section} and \appendix lettering.
    assert_eq!(eq_e.number.as_deref(), Some("2.1"));
    assert_eq!(eq_e.number_basis, Some(NumberBasis::Computed));
    assert_eq!(lem_c.number.as_deref(), Some("2.2"));
    assert_eq!(sec_app.number.as_deref(), Some("A"));
    assert_eq!(lem_sbp.number.as_deref(), Some("A.1"));

    let has_edge = |from: &str, to: &str, kind: SkEdgeKind, role: SkEdgeRole, rule: SkEdgeRule, conf: EdgeConfidence| {
        out.skeleton.edges.iter().any(|e| {
            e.from == from && e.to == to && e.kind == kind && e.role == role && e.rule == rule && e.confidence == conf
        })
    };
    let dt = def_e.tag.as_deref().unwrap();
    let et = eq_e.tag.as_deref().unwrap();
    let lt = lem_c.tag.as_deref().unwrap();
    let tt = thm_m.tag.as_deref().unwrap();

    // Statement uses: def:energy -> thm:main (Ref), eq:energy -> thm:main (Eqref), def:energy -> thm:main (Lifted).
    assert!(has_edge(dt, tt, SkEdgeKind::Uses, SkEdgeRole::Statement, SkEdgeRule::Ref, EdgeConfidence::Explicit));
    assert!(has_edge(et, tt, SkEdgeKind::Uses, SkEdgeRole::Statement, SkEdgeRule::Eqref, EdgeConfidence::Explicit));
    assert!(has_edge(dt, tt, SkEdgeKind::Uses, SkEdgeRole::Statement, SkEdgeRule::Lifted, EdgeConfidence::Explicit));

    // Proof uses on thm:main: Lemma 2.2 (TextMention, Heuristic), eq:energy (Eqref, Explicit), lifted def:energy (Lifted, Explicit).
    assert!(has_edge(lt, tt, SkEdgeKind::Uses, SkEdgeRole::Proof, SkEdgeRule::TextMention, EdgeConfidence::Heuristic));
    assert!(has_edge(et, tt, SkEdgeKind::Uses, SkEdgeRole::Proof, SkEdgeRule::Eqref, EdgeConfidence::Explicit));
    assert!(has_edge(dt, tt, SkEdgeKind::Uses, SkEdgeRole::Proof, SkEdgeRule::Lifted, EdgeConfidence::Explicit));

    // Bibliography parsing from refs.bib with normalized arXiv ID, DOI, title, and locators.
    let sp = out.skeleton.bib.iter().find(|b| b.key == "spectral2021").expect("spectral2021");
    assert_eq!(sp.arxiv_id.as_deref(), Some("2104.01234"));
    assert_eq!(sp.doi.as_deref(), Some("10.1000/spectral.2021"));
    assert_eq!(sp.title.as_deref(), Some("Spectral Gaps on Weighted Graphs"));
    assert_eq!(sp.locators.len(), 1);
    assert_eq!(sp.locators[0].text, "Theorem 4.1");

    let dc = out.skeleton.bib.iter().find(|b| b.key == "decay2022").expect("decay2022");
    assert_eq!(dc.arxiv_id.as_deref(), Some("2205.05678"));
    assert_eq!(dc.locators.len(), 1);
    assert_eq!(dc.locators[0].text, "Lemma 3");
}

#[test]
fn timing_generated_80_page_paper() {
    // Realistic 80-page paper (~60 lines/page = ~4,800 lines, 20 sections, definitions, lemmas,
    // theorems, proofs, equations, cross-references, citations, and bibliography).
    let mut tex = String::with_capacity(300_000);
    tex.push_str("\\documentclass{article}\n\\usepackage{amsmath,amsthm}\n");
    tex.push_str("\\numberwithin{equation}{section}\n");
    tex.push_str("\\newtheorem{theorem}{Theorem}[section]\n");
    tex.push_str("\\newtheorem{lemma}[theorem]{Lemma}\n");
    tex.push_str("\\newtheorem{definition}[theorem]{Definition}\n");
    tex.push_str("\\begin{document}\n\\begin{abstract}\n");
    tex.push_str("We establish main estimates in Theorem~\\ref{thm:20} across an 80-page monograph.\n");
    tex.push_str("\\end{abstract}\n");

    for sec in 1..=20 {
        tex.push_str(&format!("\\section{{Section {sec}: Spectral Analysis}}\\label{{sec:{sec}}}\n"));
        tex.push_str(&format!(
            "\\begin{{definition}}[Operator $A_{{{sec}}}$]\\label{{def:{sec}}}\nLet $A_{{{sec}}}$ act on $\\ell^2(\\mathbb{{Z}}^d)$ via\n\\begin{{equation}}\\label{{eq:def-{sec}}}\nA_{{{sec}}} u = \\sum_{{j=1}}^{{d}} \\partial_j^* \\partial_j u.\n\\end{{equation}}\n\\end{{definition}}\n"
        ));
        for k in 1..=5 {
            let lid = (sec - 1) * 5 + k;
            tex.push_str(&format!(
                "\\begin{{lemma}}[Estimate {sec}.{k}]\\label{{lem:{lid}}}\nUnder Definition~\\ref{{def:{sec}}} and \\eqref{{eq:def-{sec}}}, the resolvent bound holds.\n\\end{{lemma}}\n"
            ));
            tex.push_str(&format!(
                "\\begin{{proof}}\nCombine \\eqref{{eq:def-{sec}}} with standard commutator bounds \\cite[Section {k}]{{ref{sec}}}.\n\\end{{proof}}\n"
            ));
            for p in 0..30 {
                tex.push_str(&format!(
                    "Expository discussion line {p} in section {sec} explaining the asymptotic regime of Estimate {sec}.{k}.\n"
                ));
            }
        }
        let prev_lem = (sec - 1) * 5 + 5;
        tex.push_str(&format!(
            "\\begin{{theorem}}[Main Result {sec}]\\label{{thm:{sec}}}\nUsing Lemma~\\ref{{lem:{prev_lem}}}, the spectral gap stays uniform.\n\\end{{theorem}}\n"
        ));
        tex.push_str(&format!(
            "\\begin{{proof}}\nApply Lemma~\\ref{{lem:{prev_lem}}} together with \\eqref{{eq:def-{sec}}}.\n\\end{{proof}}\n"
        ));
    }
    tex.push_str("\\begin{thebibliography}{99}\n");
    for sec in 1..=20 {
        tex.push_str(&format!(
            "\\bibitem{{ref{sec}}} A. Author, \\emph{{Monograph Part {sec}}}, arXiv:2401.{:05} (2024).\n",
            sec
        ));
    }
    tex.push_str("\\end{thebibliography}\n\\end{document}\n");

    let mut files = BTreeMap::new();
    files.insert("main.tex".into(), tex);
    let (out, timing) = build_tex_skeleton_timed(&TexSkeletonInput {
        source_id: "bench-80page".into(),
        source_seq: 1,
        paper_key: PaperKey::parse("arxiv:2601.00080").unwrap(),
        version: Some("v1".into()),
        files,
        primary_path: Some("main.tex".into()),
        files_sha256: None,
    });

    eprintln!(
        "80-page paper timing: lines={}, nodes={}, edges={}, bib={}, json_bytes={}, flatten={}us, scan={}us, total={}us",
        timing.flattened_lines,
        timing.node_count,
        out.skeleton.edges.len(),
        out.skeleton.bib.len(),
        timing.json_bytes,
        timing.flatten_us,
        timing.scan_us,
        timing.total_us
    );

    assert_eq!(out.skeleton.quality, SkeletonQuality::Full);
    assert!(timing.flattened_lines >= 3_800);
    assert_eq!(out.skeleton.bib.len(), 20);
    assert!(!out.skeleton.edges.is_empty());
    assert!(timing.total_duration().as_secs() < 10);
}
