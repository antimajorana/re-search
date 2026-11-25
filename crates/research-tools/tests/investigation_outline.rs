use research_tools::investigation_outline::{
    build_outline, format_outline_for_prompt, resolve_scope, ScopeResolution,
};

fn aps_paper_fixture() -> (String, Vec<(usize, usize, usize)>) {
    let text = "\
Symmetry-Enriched Topological Phases and Twist Defects
A. Author and B. Collaborator

Abstract
We classify two-dimensional symmetry-enriched phases and study the braiding and fusion
of symmetry defects and lattice dislocations in Abelian and non-Abelian orders.

I. INTRODUCTION
Topologically ordered phases host fractionalized excitations with nontrivial braiding.
Earlier work by J. Smith, Phys. Rev. B 80, 012345 (2009) examined Abelian examples.
In this paper we construct a general framework for symmetry defects and dislocations.

II. SYMMETRY FRACTIONALIZATION
We review the cohomology classification of symmetry fractionalization on anyons.
For a symmetry group G, the fractionalization class is specified by a 2-cocycle.
  w(g, h) = exp(i phi(g, h))                                      (1)
This equation fixes the projective representation carried by each bulk quasiparticle.

III. GAUGING THE GLOBAL SYMMETRY
Gauging G promotes symmetry defects to deconfined dynamical fluxes in the enlarged theory.
FIG. 1. Phase diagram showing the confined, Higgs, and gauged topological regimes.
The partition function on a closed manifold depends on the flat background bundle.

IV. ALGEBRAIC THEORY OF DEFECTS
Defects form a G-crossed braided tensor category with fusion and associativity data.
We collect the consistency conditions for the F-symbols and R-symbols.

V. SYMMETRY DEFECTS AND LATTICE MODELS
In this section we realize the abstract defect classes in explicit lattice models.

A. Microscopic Hamiltonian
Consider a commuting-projector Hamiltonian on a square lattice with plaquette terms.
Each vertex carries a local Hilbert space of dimension d.

B. Flux Attachment
Attaching a bulk anyon to a symmetry defect shifts its topological spin.
The shifted spin satisfies a coboundary identity in group cohomology.

C. Domain Walls and Permutations
Invertible domain walls implement braided autoequivalences of the modular tensor category.
When a quasiparticle crosses the wall it is permuted according to the action of g.

D. Twist Defects and Lattice Geometry
We now specialize to lattice translations and crystalline point-group symmetries.

1. Lattice Translation Symmetry
Translations act by shifting plaquettes and can permute electric and magnetic charges.
We denote the elementary lattice vectors by a_x and a_y.

2. Genons and Higher-Genus Manifolds
Branch cuts connecting twist defects emulate higher-genus topology on the plane.
The ground-state degeneracy grows exponentially with the number of genons.

3. Braiding of Elementary Defects
Exchanging two twist defects induces a projective representation of the braid group.
The topological spin of a composite defect determines the phase under a full twist.

4. Dislocations and Twist Defects
A lattice dislocation with Burgers vector b acts as a symmetry defect for translation.
We verify three properties of isolated dislocations:
1. Locality of the defect core
2. Quantization of the Burgers vector
3. Protection by the bulk energy gap
Below, we demonstrate the fusion rule for two elementary dislocations:
  d_1 x d_2 = sum_a N_{12}^a a                                    (52)
FIG. 3. Braiding path of four lattice dislocations around a plaquette.
The quantum dimension of each dislocation equals sqrt(2) in the Z_2 toric code.

VI. CONCLUSIONS
We have classified symmetry defects and demonstrated their lattice realizations.
Future directions include three-dimensional fracton models and non-invertible symmetries.

Acknowledgments
We thank C. Colleague for helpful discussions on cohomology tables.

Appendix A: Group Cohomology Conventions
We summarize the homogeneous and inhomogeneous cochain conventions used in the text.
  d omega(g, h, k) = 1                                            (A3)
TABLE A2. Explicit 3-cocycle representatives for Z_2 x Z_2 symmetry.

Appendix B: Derivation of the Dislocation Fusion Rule
In this appendix we prove the fusion rule in Eq. (52) from the stabilizer algebra.

B.1 Stabilizer Reduction
We reduce the plaquette operators around a pair of Burgers vectors to a local algebra.
The resulting commutation relations reproduce the Ising fusion channels.

References
1. A. Kitaev, Ann. Phys. 303, 2 (2003).
2. M. Barkeshli, P. Bonderson, M. Cheng, and Z. Wang, Phys. Rev. B 100, 115147 (2019).
";
    let total = text.lines().count();
    let page_lines = vec![
        (1, 1, 25),
        (2, 26, 55),
        (3, 56, 82),
        (4, 83, total),
    ];
    (text.to_string(), page_lines)
}

#[test]
fn aps_fixture_detects_v_d_4_appendices_equations_captions_and_skips_lists() {
    let (text, pages) = aps_paper_fixture();
    let outline = build_outline(&text, &pages);

    assert_eq!(outline.style, "aps");
    assert!(!outline.weak);

    let keys: Vec<&str> = outline.sections.iter().map(|s| s.key.as_str()).collect();
    for expected in [
        "Abstract", "I", "II", "III", "IV", "V", "V.A", "V.B", "V.C", "V.D",
        "V.D.1", "V.D.2", "V.D.3", "V.D.4", "VI", "A", "B", "B.1", "References",
    ] {
        assert!(keys.contains(&expected), "missing section {expected}; got {keys:?}");
    }

    // Stray author initials ("J. Smith"), numbered list inside V.D.4, and numbered references
    // must NOT be promoted to headings.
    assert!(!keys.iter().any(|k| k.ends_with(".J") || *k == "V.D.5"));
    assert_eq!(keys.iter().filter(|&&k| k == "V.D.1").count(), 1);
    assert!(!outline.sections.iter().any(|s| s.title.contains("Kitaev")));
    assert!(!outline.sections.iter().any(|s| s.title.contains("Locality of the defect core")));

    let vd4 = outline.sections.iter().find(|s| s.key == "V.D.4").unwrap();
    assert_eq!(vd4.level, 3);
    assert!(vd4.title.contains("Dislocations"));
    assert!(vd4.end_line > vd4.start_line);
    let vi = outline.sections.iter().find(|s| s.key == "VI").unwrap();
    assert_eq!(vd4.end_line, vi.start_line - 1);
    assert_eq!(vd4.page_start, Some(3));

    // Parent V.D encompasses V.D.1..V.D.4.
    let vd = outline.sections.iter().find(|s| s.key == "V.D").unwrap();
    assert!(vd.start_line < vd4.start_line);
    assert_eq!(vd.end_line, vd4.end_line);

    // Equations & captions indexed.
    assert!(outline.equations.iter().any(|e| e.label == "52"));
    assert!(outline.equations.iter().any(|e| e.label == "A3"));
    assert!(outline.captions.iter().any(|c| c.kind == "figure" && c.label == "3"));
    assert!(outline.captions.iter().any(|c| c.kind == "table" && c.label == "A2"));
    // Years in References must not be indexed as equations.
    assert!(!outline.equations.iter().any(|e| e.label == "2003" || e.label == "2019"));

    let prompt_outline = format_outline_for_prompt(&outline, 1536);
    assert!(prompt_outline.len() <= 1536);
    assert!(prompt_outline.contains("V.D.4"));
}

#[test]
fn scope_parsing_handles_v_d_4_variants_appendix_eq_fig_table_and_unresolved() {
    let (text, pages) = aps_paper_fixture();
    let outline = build_outline(&text, &pages);
    let vd4 = outline.sections.iter().find(|s| s.key == "V.D.4").unwrap();

    for q in [
        "Help me understand section V.D.4",
        "What does § V.D.4 prove?",
        "Explain §V.D.4 carefully",
        "Summarize sec. V.D.4",
        "Walk me through Sec. V D 4 please",
        "help me understand section v.d.4",
    ] {
        match resolve_scope(&outline, q) {
            ScopeResolution::Resolved { targets, ranges, front_matter, .. } => {
                assert_eq!(targets.len(), 1, "question={q}");
                assert_eq!(targets[0].label, "Section V.D.4", "question={q}");
                assert_eq!(ranges, vec![[vd4.start_line, vd4.end_line]], "question={q}");
                assert!(front_matter.is_some());
            }
            ScopeResolution::Unresolved { reason } => {
                panic!("expected resolved for '{q}', got unresolved: {reason}");
            }
        }
    }

    // Appendix B and B.1
    let app_b = outline.sections.iter().find(|s| s.key == "B").unwrap();
    match resolve_scope(&outline, "Explain the derivation in appendix B") {
        ScopeResolution::Resolved { targets, ranges, .. } => {
            assert_eq!(targets[0].kind, "appendix");
            assert_eq!(targets[0].label, "Appendix B");
            assert_eq!(ranges, vec![[app_b.start_line, app_b.end_line]]);
        }
        other => panic!("expected appendix B, got {other:?}"),
    }

    match resolve_scope(&outline, "Check App. B.1") {
        ScopeResolution::Resolved { targets, .. } => {
            assert_eq!(targets[0].label, "Appendix B.1");
        }
        other => panic!("expected Appendix B.1, got {other:?}"),
    }

    // Equation (52)
    let eq52 = outline.equations.iter().find(|e| e.label == "52").unwrap();
    match resolve_scope(&outline, "Where does eq. (52) come from?") {
        ScopeResolution::Resolved { targets, .. } => {
            assert_eq!(targets[0].kind, "equation");
            assert_eq!(targets[0].label, "Eq. (52)");
            assert!(targets[0].start_line <= eq52.line && eq52.line <= targets[0].end_line);
        }
        other => panic!("expected Eq. (52), got {other:?}"),
    }

    // Figure 3 and Table A2
    match resolve_scope(&outline, "Explain fig. 3 in detail") {
        ScopeResolution::Resolved { targets, .. } => {
            assert_eq!(targets[0].kind, "figure");
            assert_eq!(targets[0].label, "Fig. 3");
        }
        other => panic!("expected Fig. 3, got {other:?}"),
    }
    match resolve_scope(&outline, "What are the axes of Table A2?") {
        ScopeResolution::Resolved { targets, .. } => {
            assert_eq!(targets[0].kind, "table");
            assert_eq!(targets[0].label, "Table A2");
        }
        other => panic!("expected Table A2, got {other:?}"),
    }

    // Unresolved: no reference or missing target
    for q in [
        "What are the main results of this paper?",
        "Explain the section in detail without citing a number",
        "Help me understand section IX.Z.9",
        "What is in Appendix Z?",
        "Derive Eq. (999)",
    ] {
        assert!(
            matches!(resolve_scope(&outline, q), ScopeResolution::Unresolved { .. }),
            "expected unresolved for '{q}'"
        );
    }
}

#[test]
fn dotted_arabic_fixture_builds_hierarchy_and_resolves() {
    let text = "\
Abstract
We analyze boundary variations in effective field theory.

1. Introduction
Boundary terms control the variational principle in finite volume.
We review prior formulations and summarize our results.

2. Variational Setup
We consider an action functional on a manifold with boundary.

2.1 Bulk Equations
Varying the bulk fields yields the Euler-Lagrange equations.
  delta S / delta phi = 0                                         (2.1)

2.2 Boundary Conditions
Imposing Dirichlet boundary conditions eliminates the endpoint variation.

2.2.1 Fixed Endpoints
When the endpoint variation vanishes, the boundary term equals zero.

3. Discussion
We compare Dirichlet and Neumann boundary conditions.

Appendix A: Notation
We use natural units with hbar = c = 1.
";
    let outline = build_outline(text, &[]);
    assert_eq!(outline.style, "dotted");
    assert!(!outline.weak);

    let keys: Vec<&str> = outline.sections.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(
        keys,
        vec!["Abstract", "1", "2", "2.1", "2.2", "2.2.1", "3", "A"]
    );

    match resolve_scope(&outline, "Explain section 2.2.1 and eq. (2.1)") {
        ScopeResolution::Resolved { targets, .. } => {
            assert_eq!(targets.len(), 2);
            assert_eq!(targets[0].label, "Section 2.2.1");
            assert_eq!(targets[1].label, "Eq. (2.1)");
        }
        other => panic!("expected resolved dotted targets, got {other:?}"),
    }
}

#[test]
fn tex_fixture_parses_sections_appendices_equations_and_captions() {
    let text = r#"\documentclass{article}
\begin{document}
\begin{abstract}
A concise summary of the model.
\end{abstract}
% \section{Commented Out Section}
\section{Introduction}
We study the spectrum of a harmonic oscillator.
\section{Model and Spectrum}
\subsection{Hamiltonian}
\begin{equation}
H = \hbar \omega (a^\dagger a + 1/2)
\end{equation}
\subsubsection{Ladder Operators}
The creation and annihilation operators satisfy $[a, a^\dagger] = 1$.
\begin{figure}
\caption{Energy levels of the harmonic oscillator.}
\end{figure}
\begin{table}
\caption{First four eigenvalues.}
\end{table}
\appendix
\section{Normalization Details}
\begin{equation}\tag{A1}
\langle n | m \rangle = \delta_{nm}
\end{equation}
\begin{thebibliography}{9}
\bibitem{ref1} Standard textbook.
\end{thebibliography}
\end{document}
"#;
    let outline = build_outline(text, &[]);
    assert_eq!(outline.style, "tex");
    assert!(!outline.weak);

    let keys: Vec<&str> = outline.sections.iter().map(|s| s.key.as_str()).collect();
    assert!(keys.contains(&"Abstract"));
    assert!(keys.contains(&"1"));
    assert!(keys.contains(&"2"));
    assert!(keys.contains(&"2.1"));
    assert!(keys.contains(&"2.1.1"));
    assert!(keys.contains(&"A"));
    assert!(keys.contains(&"References"));
    assert!(!outline.sections.iter().any(|s| s.title.contains("Commented Out")));

    assert!(outline.equations.iter().any(|e| e.label == "1"));
    assert!(outline.equations.iter().any(|e| e.label == "A1"));
    assert!(outline.captions.iter().any(|c| c.kind == "figure" && c.label == "1"));
    assert!(outline.captions.iter().any(|c| c.kind == "table" && c.label == "1"));

    match resolve_scope(&outline, "Help me understand section 2.1.1") {
        ScopeResolution::Resolved { targets, .. } => {
            assert_eq!(targets[0].label, "Section 2.1.1");
            assert_eq!(targets[0].title.as_deref(), Some("Ladder Operators"));
        }
        other => panic!("expected Section 2.1.1, got {other:?}"),
    }
    match resolve_scope(&outline, "Check appendix A and Eq. (A1)") {
        ScopeResolution::Resolved { targets, .. } => {
            assert_eq!(targets[0].label, "Appendix A");
            assert_eq!(targets[1].label, "Eq. (A1)");
        }
        other => panic!("expected Appendix A and Eq. (A1), got {other:?}"),
    }
}

#[test]
fn numbered_list_false_positives_are_rejected() {
    let text = "\
I. INTRODUCTION
We summarize our assumptions as follows:
1. Locality of interactions
2. Uniform energy gap
3. Finite correlation length

Another list without a colon and with sentences.
1. The ground state is unique on the sphere.
2. The modular matrices are unitary.
3. The chiral central charge is quantized.

Wrapped list items should also be rejected.
1. The entanglement entropy obeys an area law with
a universal subleading topological correction.
2. Anyonic excitations obey ribbon braiding statistics that
depend only on the topological class.

Packed lettered items:
A. First choice
B. Second choice

II. CONCLUSION
All three properties hold across the phase.
";
    let outline = build_outline(text, &[]);
    assert_eq!(outline.style, "aps");
    assert!(!outline.weak);
    let keys: Vec<&str> = outline.sections.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(keys, vec!["I", "II"]);
}

#[test]
fn weak_outline_for_unstructured_text_still_indexes_equations_and_returns_unresolved_for_sections() {
    let text = "\
Notes on a boundary calculation
We assume a fixed boundary throughout this short note.
The endpoint variation vanishes identically:
  delta S = 0                                                     (7)
No section headings appear anywhere in this document.
";
    let outline = build_outline(text, &[]);
    assert!(outline.weak);
    assert_eq!(outline.style, "weak");
    assert!(outline.sections.is_empty());
    assert_eq!(outline.equations.len(), 1);
    assert_eq!(outline.equations[0].label, "7");

    assert!(matches!(
        resolve_scope(&outline, "Explain section V.D.4"),
        ScopeResolution::Unresolved { .. }
    ));
    match resolve_scope(&outline, "Explain eq. (7)") {
        ScopeResolution::Resolved { targets, .. } => {
            assert_eq!(targets[0].label, "Eq. (7)");
        }
        other => panic!("expected Eq. (7) to resolve even on weak outline, got {other:?}"),
    }
}
