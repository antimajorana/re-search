use super::*;
use crate::quote_core::EvidenceAbstractPayload;

#[test]
fn strips_jats_xml_and_unescapes_entities() {
    let raw = "<jats:title>Abstract</jats:title>\
               <jats:p>We analyse <jats:italic>non-Hermitian</jats:italic> chains with \
               &alpha; &lt; 1 &amp; &beta; &gt; 0 and H<sub>2</sub>O&#x2013;like modes.</jats:p>\
               <jats:p>Numerical checks confirm &Delta; &ge; 0.</jats:p>";
    let clean = normalize_abstract(raw, true).expect("non-empty");
    assert_eq!(
        clean,
        "Abstract We analyse non-Hermitian chains with \u{03B1} < 1 & \u{03B2} > 0 and H2O\u{2013}like modes. Numerical checks confirm \u{0394} \u{2265} 0."
    );
}

#[test]
fn preserves_plain_text_inequalities_and_unknown_entities() {
    let raw = "For x < 1 and y > 0 with a < b + c we keep R&D; and &#65; intact.";
    let clean = normalize_abstract(raw, true).expect("non-empty");
    assert_eq!(clean, "For x < 1 and y > 0 with a < b + c we keep R&D; and A intact.");
}

#[test]
fn handles_xml_comments_and_cdata() {
    let raw = "<!-- hidden --><jats:p>Keep <![CDATA[a < b & c]]> text.</jats:p>";
    let clean = normalize_abstract(raw, true).expect("non-empty");
    assert_eq!(clean, "Keep a < b & c text.");
}

#[test]
fn caps_abstract_at_3000_unicode_chars() {
    let long: String = "α".repeat(3_500);
    let capped = normalize_abstract(&long, false).expect("non-empty");
    assert_eq!(capped.chars().count(), MAX_ABSTRACT_CHARS);
    assert!(capped.is_char_boundary(capped.len()));
}

#[test]
fn whitespace_only_or_empty_tags_yield_none() {
    assert_eq!(normalize_abstract("   \n\t  ", false), None);
    assert_eq!(normalize_abstract("<jats:p>   </jats:p>", true), None);
}

#[test]
fn locate_abstract_quote_enforces_rule_c3_and_returns_exact_offsets() {
    let abs = "We prove   that the spectral\n gap closes\tlinearly at λ = 1.";
    let m = locate_abstract_quote(abs, "spectral gap closes linearly at λ = 1", true)
        .expect("should match");
    assert_eq!(&abs[m.start..m.end], "spectral\n gap closes\tlinearly at λ = 1");
    assert_eq!(m.matched_text, "spectral\n gap closes\tlinearly at λ = 1");
    assert_eq!(m.quote_sha256, quote_sha256("spectral gap closes linearly at λ = 1"));

    // Non-verbatim (rebuilt-from-index) abstract MUST be rejected unconditionally.
    let err = locate_abstract_quote(abs, "spectral gap closes", false).unwrap_err();
    assert!(err.contains("non-verbatim"), "unexpected error: {err}");

    // Fabricated quote MUST be rejected.
    let miss = locate_abstract_quote(abs, "exponentially fast decay", true).unwrap_err();
    assert!(miss.contains("not found"), "unexpected error: {miss}");

    // Empty quote MUST be rejected.
    assert!(locate_abstract_quote(abs, "   \n ", true).is_err());
}

#[test]
fn locate_abstract_quote_integrates_with_quote_core_payload() {
    let abs = "Fictional summary showing quadratic scaling of entanglement entropy.";
    let m = locate_abstract_quote(abs, "quadratic scaling of entanglement", true).unwrap();
    let blob = ledger::sha256_hex(b"synthetic-blob");
    let key = PaperKey::from_arxiv("2501.00001").unwrap();
    let payload = EvidenceAbstractPayload::new(
        key,
        m.matched_text.clone(),
        m.start,
        m.end,
        blob,
        "arxiv",
        None,
        vec!["sc-1".into()],
        Some("Entanglement scaling claim".into()),
    )
    .expect("valid evidence.abstract payload");
    assert_eq!(payload.start, m.start);
    assert_eq!(payload.end, m.end);
    assert_eq!(payload.quote_sha256, m.quote_sha256);
}

#[test]
fn extract_and_locate_abstract_from_raw_crossref_and_arxiv_blobs() {
    let crossref_blob = br#"{
        "message": {
            "items": [{
                "DOI": "10.9999/synth.2025.01",
                "title": ["Synthetic Lattice Study"],
                "abstract": "<jats:p>We derive a <jats:italic>sharp bound</jats:italic> on edge currents.</jats:p>"
            }]
        }
    }"#;
    let doi_key = PaperKey::from_doi("10.9999/synth.2025.01").unwrap();
    let m = locate_abstract_quote_in_blob(
        LitProvider::Crossref,
        crossref_blob,
        &doi_key,
        "sharp bound on edge currents",
    )
    .expect("quote in crossref blob");
    assert_eq!(m.matched_text, "sharp bound on edge currents");

    let arxiv_blob = br#"<?xml version="1.0" encoding="UTF-8"?>
    <feed xmlns="http://www.w3.org/2005/Atom">
      <entry>
        <id>http://arxiv.org/abs/2501.00042v1</id>
        <title>Synthetic Spin Model</title>
        <summary>  We construct a solvable   dimer Hamiltonian on a hex grid. </summary>
      </entry>
    </feed>"#;
    let arxiv_key = PaperKey::from_arxiv("2501.00042").unwrap();
    let m2 = locate_abstract_quote_in_blob(
        LitProvider::Arxiv,
        arxiv_blob,
        &arxiv_key,
        "solvable dimer Hamiltonian",
    )
    .expect("quote in arxiv blob");
    assert_eq!(m2.matched_text, "solvable dimer Hamiltonian");
}
