//! Deterministic property-style tests for the V1 TeX skeleton scanner (no extra dependencies).
//!
//! Verifies termination, panic freedom, parent/child span containment, and origin-map
//! fidelity across random interleavings of `\begin`, `\end`, `%`, braces, brackets,
//! verbatim blocks, `\iffalse...\fi`, sections, and multi-file `\input`.

use research_tools::paper_key::PaperKey;
use research_tools::skeleton::{build_tex_skeleton, TexSkeletonInput};
use std::collections::{BTreeMap, BTreeSet};

struct Rng64(u64);

impl Rng64 {
    fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn usize(&mut self, bound: usize) -> usize {
        (self.next_u64() as usize) % bound
    }
}

const FRAGMENTS: &[&str] = &[
    "\\begin{theorem}[Title \\cite{k1}]",
    "\\begin{theorem}[Unclosed title",
    "\\end{theorem}",
    "\\begin{lemma}",
    "\\end{lemma}",
    "\\begin{proof}[Proof of Theorem 1]",
    "\\end{proof}",
    "\\begin{equation}\\label{eq:a} x = y \\end{equation}",
    "\\begin{align}\na &= b \\label{eq:al1} \\\\\nc &= d \\label{eq:al2}\n\\end{align}",
    "\\begin{figure}\\caption{Cap}\\label{fig:1}\\end{figure}",
    "\\begin{table}\\caption{Tab}\\end{table}",
    "\\begin{center}",
    "\\end{center}",
    "\\begin{verbatim}\n\\begin{theorem} fake \\end{theorem}\n\\end{verbatim}",
    "\\begin{verbatim}\nunclosed verbatim",
    "\\section{Heading}",
    "\\subsection{Sub {nested} heading}",
    "\\section{Broken heading",
    "\\label{lbl:x}",
    "\\iffalse",
    "\\else",
    "\\fi",
    "% comment with \\begin{theorem} and }",
    "50\\% literal percent % trailing comment",
    "\\verb|\\begin{lemma}|",
    "\\[ a + b \\]",
    "$$ c + d $$",
    "{ unbalanced open brace",
    "} stray close brace",
    "[ unbalanced bracket",
    "] stray close bracket",
    "\\input{sub1}",
    "\\input{sub2}",
    "\\input{missing}",
    "\\input{main}",
    "\\newtheorem{mythm}[theorem]{My Theorem}",
    "\\declaretheorem[name=Claim, sibling=theorem]{myclm}",
    "\\newcommand{\\foo}[1]{#1^2}",
    "Plain prose with $x + y = z$.",
    "",
];

fn gen_file(rng: &mut Rng64, line_count: usize, wrap_doc: bool) -> String {
    let mut out = String::new();
    if wrap_doc {
        out.push_str("\\documentclass{article}\n\\newtheorem{theorem}{Theorem}\n\\begin{document}\n");
    }
    for _ in 0..line_count {
        let frag = FRAGMENTS[rng.usize(FRAGMENTS.len())];
        out.push_str(frag);
        if rng.usize(5) == 0 {
            // Occasionally join two fragments on the same physical line.
            out.push(' ');
            out.push_str(FRAGMENTS[rng.usize(FRAGMENTS.len())]);
        }
        out.push('\n');
    }
    if wrap_doc && rng.usize(4) != 0 {
        out.push_str("\\end{document}\n");
    }
    out
}

#[test]
fn property_random_tex_interleavings_terminate_nest_and_map_origins() {
    let paper_key = PaperKey::parse("arxiv:2601.00099").unwrap();

    for seed in 1u64..=200 {
        let mut rng = Rng64::new(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut files = BTreeMap::new();
        let wrap_doc = seed % 5 != 0;
        let n_main = 15 + rng.usize(35);
        let n_sub1 = 4 + rng.usize(12);
        let n_sub2 = 4 + rng.usize(12);
        files.insert("main.tex".to_string(), gen_file(&mut rng, n_main, wrap_doc));
        files.insert("sub1.tex".to_string(), gen_file(&mut rng, n_sub1, false));
        files.insert("sub2.tex".to_string(), gen_file(&mut rng, n_sub2, false));

        let line_counts: BTreeMap<String, usize> = files
            .iter()
            .map(|(p, text)| (p.clone(), text.lines().count()))
            .collect();

        let out = build_tex_skeleton(&TexSkeletonInput {
            source_id: format!("prop-{seed}"),
            source_seq: seed,
            paper_key: paper_key.clone(),
            version: None,
            files: files.clone(),
            primary_path: Some("main.tex".into()),
            files_sha256: None,
        });

        // Origin-map entries must point to real files and valid 1-based physical lines.
        let mut origin_set: BTreeSet<(&str, usize)> = BTreeSet::new();
        for fl in &out.flattened.lines {
            let max_ln = *line_counts
                .get(&fl.origin.path)
                .unwrap_or_else(|| panic!("seed {seed}: unknown origin path `{}`", fl.origin.path));
            assert!(
                fl.origin.line >= 1 && fl.origin.line <= max_ln,
                "seed {seed}: origin line {} out of 1..={max_ln} for {}",
                fl.origin.line,
                fl.origin.path
            );
            origin_set.insert((fl.origin.path.as_str(), fl.origin.line));
        }

        // Check every node invariant.
        assert_eq!(out.skeleton.nodes.len(), out.parent_orders.len());
        for (idx, node) in out.skeleton.nodes.iter().enumerate() {
            assert_eq!(node.order, idx, "seed {seed}: order mismatch");
            assert!(node.tag.is_none(), "seed {seed}: V1 tag must be None");
            assert!(node.parent.is_none(), "seed {seed}: V1 parent tag must be None");
            assert!(node.proof_of.is_none(), "seed {seed}: V1 proof_of tag must be None");
            assert_eq!(node.fp.len(), 12, "seed {seed}: fp length");
            assert!(!node.spans.is_empty(), "seed {seed}: node {idx} has empty spans");

            // Every span must map back through the origin map to a real physical file and line range.
            for sp in &node.spans {
                let max_ln = *line_counts
                    .get(&sp.path)
                    .unwrap_or_else(|| panic!("seed {seed}: node {idx} span path `{}` unknown", sp.path));
                assert!(
                    sp.start_line >= 1 && sp.start_line <= sp.end_line && sp.end_line <= max_ln,
                    "seed {seed}: node {idx} invalid span {sp:?} (max {max_ln})"
                );
                for ln in sp.start_line..=sp.end_line {
                    assert!(
                        origin_set.contains(&(sp.path.as_str(), ln)),
                        "seed {seed}: node {idx} span {sp:?} line {ln} not in origin map"
                    );
                }
            }

            // Every child span must lie inside some span of its parent.
            if let Some(p_idx) = out.parent_orders[idx] {
                assert!(p_idx < idx, "seed {seed}: parent {p_idx} must precede child {idx}");
                let p_spans = &out.skeleton.nodes[p_idx].spans;
                for c_sp in &node.spans {
                    assert!(
                        p_spans.iter().any(|p_sp| p_sp.contains(c_sp)),
                        "seed {seed}: child node {idx} span {c_sp:?} not inside parent {p_idx} spans {p_spans:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn property_same_line_multi_file_spans_preserve_nesting_and_origins() {
    let mut files = BTreeMap::new();
    files.insert(
        "main.tex".into(),
        "\\begin{document}\n\\section{Mix} \\input{a} \\input{b}\n\\end{document}\n".into(),
    );
    files.insert(
        "a.tex".into(),
        "\\begin{theorem}[From A]\\label{thm:a}\nStatement A.\n\\end{theorem}\n".into(),
    );
    files.insert(
        "b.tex".into(),
        "\\begin{proof}\nProof B.\n\\end{proof}\n".into(),
    );

    let out = build_tex_skeleton(&TexSkeletonInput {
        source_id: "same-line".into(),
        source_seq: 1,
        paper_key: PaperKey::parse("arxiv:2601.00100").unwrap(),
        version: None,
        files,
        primary_path: Some("main.tex".into()),
        files_sha256: None,
    });

    assert_eq!(out.skeleton.nodes.len(), 3);
    for (idx, node) in out.skeleton.nodes.iter().enumerate() {
        if let Some(p) = out.parent_orders[idx] {
            for sp in &node.spans {
                assert!(
                    out.skeleton.nodes[p].spans.iter().any(|psp| psp.contains(sp)),
                    "child {idx} span {sp:?} not inside parent {p} spans {:?}",
                    out.skeleton.nodes[p].spans
                );
            }
        }
    }
}

#[test]
fn single_line_iffalse_and_verb_do_not_expand_includes() {
    let mut files = BTreeMap::new();
    files.insert(
        "main.tex".into(),
        concat!(
            "\\begin{document}\n",
            "\\section{Dormant}\n",
            "\\iffalse \\input{dead} \\fi \\input{alive}\n",
            "\\verb|\\input{dead}| \\begin{verbatim}\\input{dead}\\end{verbatim}\n",
            "\\end{document}\n",
        )
        .into(),
    );
    files.insert(
        "dead.tex".into(),
        "\\begin{theorem}[Dead]\nMust never be inlined.\n\\end{theorem}\n".into(),
    );
    files.insert(
        "alive.tex".into(),
        "\\begin{lemma}[Alive]\\label{lem:alive}\nMust be inlined.\n\\end{lemma}\n".into(),
    );

    let out = build_tex_skeleton(&TexSkeletonInput {
        source_id: "dormant".into(),
        source_seq: 1,
        paper_key: PaperKey::parse("arxiv:2601.00101").unwrap(),
        version: None,
        files,
        primary_path: Some("main.tex".into()),
        files_sha256: None,
    });

    assert_eq!(out.flattened.inlined_files, vec!["alive.tex".to_string()]);
    assert_eq!(out.skeleton.nodes.len(), 2);
    assert_eq!(out.skeleton.nodes[1].label.as_deref(), Some("lem:alive"));
}

#[test]
fn node_cap_truncates_with_partial_quality() {
    use research_tools::skeleton::{SkeletonQuality, MAX_SKELETON_NODES};
    let mut tex = String::from("\\begin{document}\n\\section{Many}\n");
    for i in 0..(MAX_SKELETON_NODES + 50) {
        tex.push_str(&format!("\\begin{{lemma}}\nL{i}.\n\\end{{lemma}}\n"));
    }
    tex.push_str("\\end{document}\n");
    let mut files = BTreeMap::new();
    files.insert("main.tex".into(), tex);

    let out = build_tex_skeleton(&TexSkeletonInput {
        source_id: "cap".into(),
        source_seq: 1,
        paper_key: PaperKey::parse("arxiv:2601.00102").unwrap(),
        version: None,
        files,
        primary_path: Some("main.tex".into()),
        files_sha256: None,
    });

    assert_eq!(out.skeleton.nodes.len(), MAX_SKELETON_NODES);
    assert_eq!(out.skeleton.quality, SkeletonQuality::Partial);
    assert!(out.quality_notes.iter().any(|n| n.contains("node limit")));
}
