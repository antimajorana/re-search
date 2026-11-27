//! Unit tests for `solve_derivation` (package S17a).

use super::{
    reject_if_pdf, split_tex_derivation, split_tex_derivation_bytes, MAX_DERIVATION_TEX_BYTES,
    PDF_REJECT_MESSAGE,
};

const SAMPLE_ANHARMONIC_TEX: &str = r#"\documentclass{article}
\begin{document}
\section{Hamiltonian}
Start from the perturbed oscillator Hamiltonian:
\begin{equation}
  H = H_0 + \epsilon m \omega^2 x^4 \label{eq:ham}
\end{equation}
\section{First-Order Shift}
Using the fourth moment of the ground state:
\begin{align}
  \langle x^4 \rangle_0 &= \frac{3}{4 m^2 \omega^2} \label{eq:x4} \\
  \Delta E_1 &= \epsilon m \omega^2 \langle x^4 \rangle_0 = \frac{3\epsilon}{4m} \tag{A.1}
\end{align}
% Commented out equation must be ignored:
% \begin{equation} H_{fake} = 0 \end{equation}
And the starred display check:
\[
  E_0(\epsilon) = \frac{\omega}{2} + \frac{3\epsilon}{4m} + O(\epsilon^2)
\]
\end{document}
"#;

#[test]
fn splits_numbered_align_and_bracket_equations_deterministically_under_1s() {
    let split = split_tex_derivation(SAMPLE_ANHARMONIC_TEX).expect("valid TeX should split");
    assert_eq!(split.outline_style, "tex");
    assert_eq!(split.section_count, 2);
    assert!(split.elapsed_ms < 1000, "split took {} ms", split.elapsed_ms);
    assert_eq!(split.equations.len(), 4);

    assert_eq!(split.equations[0].index, 1);
    assert_eq!(split.equations[0].label, "1");
    assert_eq!(split.equations[0].tex_label.as_deref(), Some("eq:ham"));
    assert_eq!(split.equations[0].section.as_deref(), Some("Hamiltonian"));
    assert!(split.equations[0].raw_tex.contains(r"\epsilon m \omega^2 x^4"));

    assert_eq!(split.equations[1].index, 2);
    assert_eq!(split.equations[1].label, "2");
    assert_eq!(split.equations[1].tex_label.as_deref(), Some("eq:x4"));
    assert_eq!(split.equations[1].section.as_deref(), Some("First-Order Shift"));

    assert_eq!(split.equations[2].index, 3);
    assert_eq!(split.equations[2].label, "A.1");
    assert!(split.equations[2].raw_tex.contains(r"\frac{3\epsilon}{4m}"));

    assert_eq!(split.equations[3].index, 4);
    assert_eq!(split.equations[3].env, "bracket");
    assert_eq!(split.equations[3].label, "eq-4");

    assert_eq!(split.pairs.len(), 3);
    assert_eq!(split.pairs[0].from_index, 1);
    assert_eq!(split.pairs[0].to_index, 2);
    assert_eq!(split.pairs[2].from_index, 3);
    assert_eq!(split.pairs[2].to_index, 4);

    let again = split_tex_derivation(SAMPLE_ANHARMONIC_TEX).unwrap();
    assert_eq!(split.source_sha256, again.source_sha256);
    assert_eq!(split.equations, again.equations);
}

#[test]
fn splits_bare_tex_fragment_with_nested_matrices_multiline_intertext_and_dollars() {
    let bare = r#"
\begin{align*}
  M &= \begin{pmatrix} 1 & 0 \\ 0 & -1 \end{pmatrix} \\
  \shortintertext{Notice the bracketed commutator and
  multi-line prose with \{escaped\} braces:}
  [A, B] &= C \\ [2pt]
  M^2 &= I_2
\end{align*}
$$ \operatorname{Tr}(M^2) = 2 $$
"#;
    let split = split_tex_derivation(bare).expect("bare TeX fragment should split");
    assert_eq!(split.outline_style, "tex");
    assert_eq!(split.equations.len(), 4);
    assert!(split.equations[0].raw_tex.contains(r"\begin{pmatrix} 1 & 0 \\ 0 & -1 \end{pmatrix}"));
    // `[A, B]` must NOT be eaten as an optional `\\ [...]` spacing argument!
    assert_eq!(split.equations[1].raw_tex, "[A, B] &= C");
    // `[2pt]` IS a genuine TeX length spec and must be stripped cleanly.
    assert_eq!(split.equations[2].raw_tex, "M^2 &= I_2");
    assert_eq!(split.equations[3].env, "dollar");
    assert_eq!(split.equations[3].raw_tex, r"\operatorname{Tr}(M^2) = 2");
    assert_eq!(split.pairs.len(), 3);
}

#[test]
fn handles_nonumber_same_line_commented_docclass_crlf_bom_and_label_spacing() {
    let tex = "\u{FEFF}%\\documentclass{article}\r\n%\\begin{document}\r\n\\section{Test}\r\n\\begin{align} x = 1 \\nonumber \\\\ y = 2 \\label {eq:y} \\end{align}\r\n";
    let split = split_tex_derivation(tex).expect("CRLF+BOM TeX should split");
    assert_eq!(split.equations.len(), 2);
    // Nonumber row must NOT steal outline label "1" from the numbered row on the same line.
    assert_eq!(split.equations[0].label, "eq-1");
    assert_eq!(split.equations[1].label, "1");
    assert_eq!(split.equations[1].tex_label.as_deref(), Some("eq:y"));
    assert_eq!(split.equations[1].section.as_deref(), Some("Test"));
}

#[test]
fn rejects_unclosed_delimiters_and_ignores_escaped_begin_and_verbatim() {
    assert!(split_tex_derivation("\\begin{equation}\n a = b \n\\end{document}").is_err());
    assert!(split_tex_derivation("\\[\n a = b \n").is_err());
    assert!(split_tex_derivation("$$\n a = b \n").is_err());

    let src = r#"\begin{verbatim}
\section{Fake}
\begin{equation}
  x = 999
\end{equation}
\end{verbatim}
Line break then literal \\begin{equation} not math
\begin{equation}
  y = 42 \mylabel{not_real}
\end{equation}
"#;
    let split = split_tex_derivation(src).unwrap();
    assert_eq!(split.section_count, 0);
    assert_eq!(split.equations.len(), 1);
    assert_eq!(split.equations[0].label, "1");
    assert_eq!(split.equations[0].tex_label, None);
    assert!(split.equations[0].raw_tex.starts_with("y = 42"));
}

#[test]
fn rejects_pdf_empty_oversize_nul_control_or_equationless_tex() {
    assert_eq!(
        reject_if_pdf(Some("notes.PDF"), b"\\begin{equation}x=1\\end{equation}").unwrap_err(),
        PDF_REJECT_MESSAGE
    );
    assert_eq!(
        split_tex_derivation_bytes(Some("derivation.tex"), b"%PDF-1.7\n1 0 obj\n").unwrap_err(),
        PDF_REJECT_MESSAGE
    );
    assert_eq!(
        split_tex_derivation_bytes(None, b"\xEF\xBB\xBF  %PDF-1.4\n").unwrap_err(),
        PDF_REJECT_MESSAGE
    );
    assert!(split_tex_derivation("   \n\t  ").is_err());
    assert!(split_tex_derivation("Just prose without display math: $x = 1$.").is_err());
    assert!(split_tex_derivation("\\begin{equation}x=1\0\\end{equation}").is_err());
    assert!(split_tex_derivation("\\begin{equation}x=1\x07\\end{equation}").is_err());
    assert!(split_tex_derivation_bytes(Some("bad.tex"), &[0xFF, 0xFE, 0x00]).is_err());
    let huge = vec![b'a'; MAX_DERIVATION_TEX_BYTES + 1];
    assert!(split_tex_derivation_bytes(Some("huge.tex"), &huge).is_err());
}
