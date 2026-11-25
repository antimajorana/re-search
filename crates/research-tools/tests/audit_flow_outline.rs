//! Failing audit proof tests (`audit-flow`) for WP10 outline & scope resolution in `research-tools`.

use research_tools::investigation_outline::{build_outline, resolve_scope, ScopeResolution};

#[test]
fn aps_outline_keeps_v_d_4_when_earlier_subsubsection_contains_numbered_list() {
    let text = "\
Symmetry Defects in Two-Dimensional Lattice Models

Abstract
We classify crystalline twist defects and lattice dislocations in topological orders.

I. INTRODUCTION
Topological orders admit symmetry defects whose braiding encodes fractionalization.

II. BULK COHOMOLOGY
Each symmetry action defines an automorphism of the bulk anyon theory.

III. GAUGING AND DEFECT FUSION
Gauging the symmetry promotes defects to dynamical fluxes.

IV. LATTICE REALIZATION
We study commuting-projector Hamiltonians on the square lattice.

V. TWIST DEFECTS AND CRYSTALLINE SYMMETRIES
We now analyze point-group and translation defects.

A. Microscopic Plaquette Terms
Vertex and plaquette stabilizers commute pairwise across the lattice.

B. Flux Attachment Rules
Attaching an anyon shifts the topological spin of the defect.

C. Domain Wall Permutations
Crossing an invertible wall permutes electric and magnetic excitations.

D. Crystalline Twist Defects
We specialize to translation and disclination defects on the lattice.

1. Lattice Translation Symmetry
Translations shift plaquettes along the primitive lattice vectors.
1. Discrete translation invariance along a_x holds on the torus.
2. Periodic boundary conditions quantize total crystal momentum.

2. Genons and Higher-Genus Surfaces
Branch cuts between twist defects emulate higher-genus manifolds.

3. Braiding of Elementary Defects
Exchanging two twist defects generates a projective braid representation.

4. Dislocations and Burgers Vectors
A lattice dislocation with Burgers vector b acts as a translation flux:
  d_1 x d_2 = 1 + psi                                            (12)
The quantum dimension of an isolated dislocation equals sqrt(2).

VI. CONCLUSIONS
We have established the crystalline defect classification.
";
    let outline = build_outline(text, &[]);
    assert_eq!(outline.style, "aps");
    let keys: Vec<&str> = outline.sections.iter().map(|s| s.key.as_str()).collect();
    assert!(
        keys.contains(&"V.D.4"),
        "numbered list inside V.D.1 caused assemble_aps to drop V.D.2..V.D.4; got keys: {keys:?}"
    );
    match resolve_scope(&outline, "Help me understand section V.D.4") {
        ScopeResolution::Resolved { targets, .. } => {
            assert_eq!(targets[0].label, "Section V.D.4");
        }
        other => panic!("expected Section V.D.4 to resolve, got {other:?}"),
    }
}

#[test]
fn tex_multiline_tag_does_not_emit_phantom_equation_or_shift_later_numbers() {
    let tex = r"\documentclass{article}
\begin{document}
\section{Introduction}
We start from the tagged definition of the lattice Hamiltonian:
\begin{equation}
  H = -J \sum_{\langle i, j \rangle} Z_i Z_j \tag{A1}
\end{equation}
\section{Ground State Energy}
The first untagged numbered equation gives the ground-state energy:
\begin{equation}
  E_0 = -N_{\mathrm{bonds}} J
\end{equation}
\end{document}
";
    let outline = build_outline(tex, &[]);
    let labels: Vec<(&str, usize)> = outline
        .equations
        .iter()
        .map(|e| (e.label.as_str(), e.line))
        .collect();
    assert_eq!(
        labels,
        vec![("A1", 6), ("1", 10)],
        "multiline \\tag{{A1}} emitted phantom equation \"1\" and shifted later equation numbers: {labels:?}"
    );
}

#[test]
fn prl_letter_tex_without_two_section_commands_still_indexes_equations_and_figures() {
    let prl_tex = r"\documentclass[prl,twocolumn]{revtex4-2}
\begin{document}
\begin{abstract}
We prove quantization of edge transport in a two-dimensional chiral insulator.
\end{abstract}
\emph{Introduction.}---Chiral edge states carry quantized thermal and electrical currents.
\begin{equation}
  \sigma_{xy} = \frac{e^2}{h} C_1
\end{equation}
\begin{figure}[t]
  \caption{Topological phase diagram as a function of sublattice mass and Haldane flux.}
\end{figure}
\end{document}
";
    let outline = build_outline(prl_tex, &[]);
    assert!(outline.weak, "PRL letter without body sections should remain weak");
    assert_eq!(
        outline.equations.len(),
        1,
        "PRL-style TeX without >=2 \\section commands indexed 0 equations: {outline:?}"
    );
    assert_eq!(
        outline.captions.len(),
        1,
        "PRL-style TeX without >=2 \\section commands indexed 0 captions: {outline:?}"
    );
}

#[test]
fn review_outline_plain_mentioning_documentclass_stays_plain() {
    let text = "\
LaTeX Authoring Notes for Physics Preprints

I. INTRODUCTION
Authors preparing APS manuscripts often start from \\documentclass[aps,prb]{revtex4-2}.
This plain-text guide summarizes section conventions and equation numbering.

II. EQUATION CONVENTIONS
Displayed equations carry right-aligned numbers in parentheses:
  E = hbar * omega                                                (1)
Do not confuse prose mentions of macros with raw TeX source files.

III. CONCLUSIONS
Plain-text papers remain parsed as plain APS documents.
";
    let outline = build_outline(text, &[]);
    assert_eq!(
        outline.style, "aps",
        "plain-text paper mentioning \\documentclass once without \\begin{{document}} was misclassified: {outline:?}"
    );
    assert!(!outline.weak);
    let keys: Vec<&str> = outline.sections.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(keys, vec!["I", "II", "III"]);
    assert_eq!(outline.equations.len(), 1);
    assert_eq!(outline.equations[0].label, "1");
}

#[test]
fn review_outline_tex_multirow_align_notag_and_subequations() {
    let tex = r"\documentclass{article}
\begin{document}
\section{Matrix and Alignment Identities}
\begin{align}
  M &= \begin{pmatrix}
    1 & 0 \\
    0 & 1
  \end{pmatrix} \\
  b &= c + d \nonumber \\
  e &= f + g \notag \\
  h &= i + j \tag{*} \\
\end{align}
\section{Subequations}
\begin{subequations}
\begin{align}
  u &= \alpha x \\
  v &= \beta y \\
\end{align}
\end{subequations}
\begin{equation}
  E = mc^2
\end{equation}
\end{document}
";
    let outline = build_outline(tex, &[]);
    assert_eq!(outline.style, "tex");
    let labels: Vec<(&str, usize)> = outline
        .equations
        .iter()
        .map(|e| (e.label.as_str(), e.line))
        .collect();
    assert_eq!(
        labels,
        vec![("1", 5), ("*", 11), ("2a", 16), ("2b", 17), ("3", 20)],
        "unexpected multi-row/subequations labels: {labels:?}"
    );
    match resolve_scope(&outline, "Explain eq. (2b) and equation (3)") {
        ScopeResolution::Resolved { targets, .. } => {
            let tlabels: Vec<&str> = targets.iter().map(|t| t.label.as_str()).collect();
            assert_eq!(tlabels, vec!["Eq. (2b)", "Eq. (3)"]);
        }
        other => panic!("expected Eq. (2b) and Eq. (3) to resolve, got {other:?}"),
    }
}
