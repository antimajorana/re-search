use super::*;
use crate::map_defs::VerifiedMapAnchor;

fn mk_anchor(quote: &str, excerpt: &str) -> VerifiedMapAnchor {
    VerifiedMapAnchor {
        source_id: "src-1".into(),
        path: "main.tex".into(),
        start_line: 10,
        end_line: 12,
        file_sha256: "a".repeat(64),
        quote_sha256: "b".repeat(64),
        quote: quote.into(),
        excerpt: excerpt.into(),
        source_seq: 2,
        cited_range: None,
        range_basis: Vec::new(),
        excerpt_window: None,
    }
}

#[test]
fn cue_detector_finds_explicit_novelty_restatement_and_background_cues() {
    let c1 = detect_authors_claim_cues(&["In Section 3, we prove our main decay estimate."]);
    assert_eq!(c1, vec!["we prove", "our main"]);

    let c2 = detect_authors_claim_cues(&["This bound is recalled from [NS06] and restated below."]);
    assert_eq!(c2, vec!["recalled from", "restated"]);

    let c3 = detect_authors_claim_cues(&["It is well-known (see e.g. [K99]) that the operator is compact."]);
    assert_eq!(c3, vec!["well-known", "see e.g."]);
    assert!(!c3.contains(&"known".to_string()), "subsumed cue should not duplicate");

    let c4 = detect_authors_claim_cues(&["Here we show that the unknown constant satisfies C > 0."]);
    assert_eq!(c4, vec!["here we show"]);
    assert!(!c4.contains(&"known".to_string()), "word boundary must reject `unknown`");
    assert!(!c4.contains(&"we show".to_string()), "subsumed `we show` should not duplicate `here we show`");
}

#[test]
fn cue_detector_returns_empty_and_no_explicit_cue_line_when_absent() {
    let cues = detect_authors_claim_cues(&["Let H be a self-adjoint operator with spectral gap g > 0."]);
    assert!(cues.is_empty());
    assert_eq!(format_cue_line(&cues), NO_EXPLICIT_CUE_TEXT);
}

#[test]
fn heading_cites_uses_exact_honest_wording_never_attributed() {
    assert_eq!(format_heading_cites(&[]), None);
    assert_eq!(format_heading_cites(&["  ".into()]), None);
    let line = format_heading_cites(&["NS06".into(), "HW05".into(), "NS06".into()]).unwrap();
    assert_eq!(line, "cites [NS06, HW05] in its heading");
    assert!(!line.to_ascii_lowercase().contains("attributed"));
    assert!(!line.to_ascii_lowercase().contains("verified"));
}

#[test]
fn status_badge_builds_all_four_labels_clips_quotes_and_never_emits_verified() {
    let long_quote = format!("We prove {}", "x".repeat(400));
    let anchor = mk_anchor(&long_quote, &long_quote);
    let cases = [
        (AuthorsClaimLabel::New, AUTHORS_LABEL_NEW, "info"),
        (AuthorsClaimLabel::Restated, AUTHORS_LABEL_RESTATED, "neutral"),
        (AuthorsClaimLabel::Background, AUTHORS_LABEL_BACKGROUND, "muted"),
        (AuthorsClaimLabel::NotStated, AUTHORS_LABEL_NOT_STATED, "muted"),
    ];
    for (label, expected_text, expected_tone) in cases {
        let rec = AuthorsClaimRecord {
            label,
            relation_note: Some("extends [NS06]".into()),
            attributed_to: vec!["NS06".into()],
            anchors: vec![anchor.clone()],
            cues: Vec::new(),
        };
        let badge = status_badge(Some(&rec), &["NS06".into()]);
        assert!(badge.present);
        assert_eq!(badge.label, Some(label));
        assert_eq!(badge.text, expected_text);
        assert_eq!(badge.tone, expected_tone);
        assert_eq!(badge.border, BORDER_DASHED);
        assert_eq!(badge.cues, vec!["we prove"]);
        assert_eq!(badge.cue_line, "Cues in quoted lines: we prove");
        assert_eq!(badge.heading_cite_line.as_deref(), Some("cites [NS06] in its heading"));
        assert_eq!(badge.quotes[0].quote.chars().count(), 300);

        let serialized = serde_json::to_string(&badge).unwrap().to_ascii_lowercase();
        assert!(!serialized.contains("verified"), "forbidden word in {serialized}");
        assert!(!serialized.contains("attributed by the authors"), "forbidden phrase in {serialized}");
        if label != AuthorsClaimLabel::New {
            assert!(!badge.text.to_ascii_lowercase().contains("new"));
        }
    }
}

#[test]
fn status_badge_without_claim_preserves_heading_cites_only() {
    let badge = status_badge(None, &["Toy98".into()]);
    assert!(!badge.present);
    assert_eq!(badge.label, None);
    assert!(badge.text.is_empty());
    assert_eq!(badge.heading_cite_line.as_deref(), Some("cites [Toy98] in its heading"));
    assert_eq!(badge.cue_line, NO_EXPLICIT_CUE_TEXT);
}

#[test]
fn badge_quote_omits_pdf_page_metadata_unless_present() {
    let plain = BadgeQuote {
        source_id: "s1".into(),
        path: "main.tex".into(),
        start_line: 10,
        end_line: 12,
        quote: "We prove the theorem.".into(),
        page: None,
        end_page: None,
        ocr: None,
    };
    let plain_json = serde_json::to_value(&plain).unwrap();
    assert!(plain_json.get("page").is_none());
    assert!(plain_json.get("end_page").is_none());
    assert!(plain_json.get("ocr").is_none());

    let pdf_ocr = BadgeQuote {
        page: Some(4),
        end_page: Some(5),
        ocr: Some(true),
        ..plain
    };
    let pdf_json = serde_json::to_value(&pdf_ocr).unwrap();
    assert_eq!(pdf_json["page"], 4);
    assert_eq!(pdf_json["end_page"], 5);
    assert_eq!(pdf_json["ocr"], true);
}
