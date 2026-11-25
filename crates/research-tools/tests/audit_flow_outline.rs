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
fn resolve_scope_handles_lowercase_space_separated_and_mixed_separator_aps_refs() {
    let text = "\
I. INTRODUCTION
Introductory discussion of the lattice Hamiltonian.

II. MODEL
Definition of the coupling constants.

III. SYMMETRIES
Global and crystalline symmetry actions.

IV. GAUGING
Promotion of global symmetries to dynamical gauge fields.

V. DEFECTS
Overview of crystalline defects.

A. Plaquette Stabilizers
Commuting local projectors.

B. Flux Attachment
Shifted topological spin rules.

C. Domain Walls
Braided autoequivalences across walls.

D. Dislocations
Sub-overview of dislocations.

1. Translation Symmetry
Elementary lattice shifts.

2. Genons
Branch cuts on the plane.

3. Braiding
Projective braid group representations.

4. Burgers Vector Algebra
Detailed derivation of the Burgers vector fusion rule.
";
    let outline = build_outline(text, &[]);
    assert!(
        outline.sections.iter().any(|s| s.key == "V.D.4"),
        "precondition: V.D.4 must exist in outline: {:?}",
        outline.sections
    );

    for prompt in [
        "walk me through sec. v d 4",
        "Help me understand section V.D 4",
    ] {
        match resolve_scope(&outline, prompt) {
            ScopeResolution::Resolved { targets, .. } => {
                assert_eq!(
                    targets[0].label, "Section V.D.4",
                    "prompt `{prompt}` silently truncated to wrong parent section: {targets:?}"
                );
            }
            other => panic!("prompt `{prompt}` failed to resolve: {other:?}"),
        }
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
