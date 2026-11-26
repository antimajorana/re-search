use super::*;
use crate::paper_key::PaperKey;

#[test]
fn word_lists_are_strictly_sorted_for_binary_search() {
    for list in [STOP_WORDS, COMMON_SENTENCE_OPENERS] {
        for pair in list.windows(2) {
            assert!(
                pair[0] < pair[1],
                "word list must be strictly sorted and unique: `{}` >= `{}`",
                pair[0],
                pair[1]
            );
        }
    }
}

#[test]
fn preserves_tex_math_and_standalone_commands_without_shattering() {
    let idea = r"Study \mathcal{N}=4 SYM with \mathrm{SU}(N) gauge group, \mathbb{Z}_2 symmetry, \operatorname{Tr}(F^2) operators, \alpha_s running, inline $c=1$, \(b_n \sim \alpha n\), and display $$S = \int d^4x \mathcal{L}$$ plus \[E_n = \hbar\omega_n\].";
    let ex = extract(idea);

    for expected in [
        r"\mathcal{N}=4",
        r"\mathrm{SU}(N)",
        r"\mathbb{Z}_2",
        r"\operatorname{Tr}(F^2)",
        r"\alpha_s",
        "c=1",
        r"b_n \sim \alpha n",
        r"S = \int d^4x \mathcal{L}",
        r"E_n = \hbar\omega_n",
    ] {
        assert!(
            ex.math_tokens.iter().any(|t| t == expected),
            "missing math token `{expected}` in {:?}",
            ex.math_tokens
        );
    }
    for noise in ["mathcal", "mathrm", "mathbb", "operatorname", "alpha"] {
        assert!(
            !ex.keywords.iter().any(|k| k == noise),
            "TeX command `{noise}` leaked into keywords: {:?}",
            ex.keywords
        );
    }
}

#[test]
fn unwraps_formatting_macros_and_strips_comments_safely() {
    let idea = "% secret comment mentioning 2103.01234 and FakeAuthor\n\
        Explore \\emph{topological order} and \\textbf{\\emph{anyon}} braiding at 50\\% filling.\n\
        Also see https://doi.org/10.1000%2Fxyz and \\cite{key2020} for context. % trailing noise";
    let ex = extract(idea);

    assert_eq!(ex.paper_refs.len(), 1);
    assert_eq!(ex.paper_refs[0].key, PaperKey::from_doi("10.1000/xyz").unwrap());
    assert!(ex.keywords.iter().any(|k| k == "topological"));
    assert!(ex.keywords.iter().any(|k| k == "topological order"));
    assert!(ex.keywords.iter().any(|k| k == "anyon"));
    assert!(ex.keywords.iter().any(|k| k == "braiding"));
    assert!(!ex.proper_names.iter().any(|p| p == "FakeAuthor"));
    for noise in ["emph", "textbf", "cite", "key2020", "secret", "trailing"] {
        assert!(!ex.keywords.iter().any(|k| k == noise), "leaked `{noise}`");
    }
}

#[test]
fn extracts_quoted_phrases_in_ascii_latex_and_unicode() {
    let idea = "Compare \"Krylov complexity\" against ``spread complexity'' and \u{201c}area law\u{201d} scaling.";
    let ex = extract(idea);
    assert_eq!(
        ex.quoted_phrases,
        vec![
            "Krylov complexity".to_string(),
            "spread complexity".to_string(),
            "area law".to_string()
        ]
    );
    assert!(ex.terms.iter().any(|t| t.kind == TermKind::QuotedPhrase && t.text == "Krylov complexity"));
}

#[test]
fn extracts_proper_names_and_rejects_sentence_initial_stopwords() {
    let idea = "The Schwinger-Dyson equations in Chern-Simons theory and AdS/CFT. \
        We suppose Kitaev chains with Majorana modes and Wick contractions relate to \
        Bekenstein-Hawking entropy in Kaluza-Klein and Yang-Mills backgrounds. \
        Consider if this holds.";
    let ex = extract(idea);

    for expected in [
        "Schwinger-Dyson",
        "Chern-Simons",
        "AdS/CFT",
        "Kitaev",
        "Majorana",
        "Wick",
        "Bekenstein-Hawking",
        "Kaluza-Klein",
        "Yang-Mills",
    ] {
        assert!(
            ex.proper_names.iter().any(|p| p == expected),
            "missing proper name `{expected}` in {:?}",
            ex.proper_names
        );
    }
    for stop in ["The", "We", "Suppose", "Consider", "If", "This"] {
        assert!(
            !ex.proper_names.iter().any(|p| p.eq_ignore_ascii_case(stop)),
            "stop word `{stop}` leaked into proper_names: {:?}",
            ex.proper_names
        );
        assert!(
            !ex.keywords.iter().any(|k| k.eq_ignore_ascii_case(stop)),
            "stop word `{stop}` leaked into keywords: {:?}",
            ex.keywords
        );
    }
}

#[test]
fn extracts_arxiv_and_doi_refs_via_f2_and_deduplicates_in_first_seen_order() {
    let idea = "See 2103.01234, then https://doi.org/10.1103/PhysRevD.90.084021., \
        also hep-th/9711200 and arXiv:math.AG/0601001; repeat https://arxiv.org/abs/2103.01234v2 \
        and (doi:10.1000/XYZ). Ignore bad ids 2199.01234 and 0704.12345.";
    let ex = extract(idea);

    let keys: Vec<&str> = ex.paper_refs.iter().map(|r| r.key.as_str()).collect();
    assert_eq!(
        keys,
        vec![
            "arxiv:2103.01234",
            "doi:10.1103/physrevd.90.084021",
            "arxiv:hep-th/9711200",
            "arxiv:math/0601001",
            "doi:10.1000/xyz",
        ]
    );
    // Version from later mention of 2103.01234v2 fills in first-seen entry.
    assert_eq!(ex.paper_refs[0].version.as_deref(), Some("v2"));
    assert_eq!(ex.paper_refs[0].ids.arxiv.as_deref(), Some("2103.01234"));
    assert_eq!(ex.paper_refs[1].ids.doi.as_deref(), Some("10.1103/physrevd.90.084021"));

    // URL scaffolding must not pollute keywords.
    for leak in ["https", "arxiv", "org", "abs", "doi"] {
        assert!(!ex.keywords.iter().any(|k| k == leak), "URL token `{leak}` leaked");
    }
}

#[test]
fn escaped_dollar_and_unclosed_dollar_do_not_swallow_prose() {
    let idea = r"Cost is \$5 per run; unclosed $x should still let Majorana braiding appear.";
    let ex = extract(idea);
    assert!(ex.proper_names.iter().any(|p| p == "Majorana"));
    assert!(ex.keywords.iter().any(|k| k == "braiding"));
}

#[test]
fn handles_empty_punctuation_unicode_and_overlong_inputs_deterministically() {
    assert_eq!(extract(""), ExtractedIdea::default());
    assert_eq!(extract("   \n\t  ...,,,;;;()[]{}   "), ExtractedIdea::default());

    let mut long = String::new();
    for _ in 0..700 {
        long.push_str("Majorana braiding — "); // includes multi-byte em-dash
    }
    assert!(long.chars().count() > MAX_IDEA_TEXT_CHARS);
    let a = extract(&long);
    let b = extract(&long);
    assert_eq!(a, b);
    assert_eq!(a.proper_names, vec!["Majorana".to_string()]);
    assert!(a.keywords.contains(&"braiding".to_string()));
}

#[test]
fn worked_example_from_explore_design_is_deterministic_and_informative() {
    let idea = "Use the growth rate of Lanczos coefficients (Krylov complexity) as an order \
        parameter for the measurement-induced entanglement transition in a monitored Majorana \
        chain. Expect b_n ~ alpha*n in the volume-law phase and saturation in the area-law phase.";
    let a = extract(idea);
    let b = extract(idea);
    assert_eq!(a, b);
    assert!(a.proper_names.contains(&"Lanczos".to_string()));
    assert!(a.proper_names.contains(&"Krylov".to_string()));
    assert!(a.proper_names.contains(&"Majorana".to_string()));
    assert!(a.math_tokens.contains(&"b_n".to_string()));
    assert!(a.keywords.contains(&"measurement-induced".to_string()));
    assert!(a.keywords.contains(&"entanglement".to_string()));
    assert!(a.keywords.contains(&"volume-law".to_string()));
    assert!(a.keywords.contains(&"area-law".to_string()));
}
