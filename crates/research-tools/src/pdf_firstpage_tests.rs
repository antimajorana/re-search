//! Unit tests for `pdf_firstpage.rs` using strictly self-authored synthetic fixtures.

use super::*;

#[test]
fn normalizes_ligatures_pua_spaces_and_diacritics() {
    assert_eq!(
        normalize_pdf_unicode("ﬁeld ﬂow ﬃ ﬄ ﬅ ﬆ ﬀ"),
        "field flow ffi ffl ft st ff"
    );
    assert_eq!(
        normalize_pdf_unicode("\u{F731}\u{F739}\u{F761}\u{F77A}\u{F721}\u{F724}\u{F726}\u{F73F}"),
        "19az!$&?"
    );
    assert_eq!(
        normalize_pdf_unicode("ＩＳＢＮ ９７８－０－３０６－４０６１５－７"),
        "ISBN 978-0-306-40615-7"
    );
    assert_eq!(
        normalize_pdf_unicode("soft\u{00AD}hyphen\u{00A0}space\u{2009}thin\u{202F}nbsp"),
        "softhyphen space thin nbsp"
    );
    assert_eq!(
        fold_diacritics("Cvetković Łukasiewicz Sørensen Différence İnce"),
        "cvetkovic lukasiewicz sorensen difference ince"
    );
    assert_eq!(
        fold_diacritics("e\u{0301}tude Groß Æther Œuvre Þór ð"),
        "etude gross aether oeuvre thor d"
    );
}

#[test]
fn filters_junk_titles_parses_dates_utf16_and_validates_isbns() {
    assert!(is_junk_pdf_title("Microsoft Word - notes.docx"));
    assert!(is_junk_pdf_title("Untitled"));
    assert!(is_junk_pdf_title("dvips"));
    assert!(is_junk_pdf_title("main.tex"));
    assert!(is_junk_pdf_title("arXiv:2401.12345v1"));
    assert!(!is_junk_pdf_title("Spectral Bounds for Lattice Hamiltonians"));

    assert_eq!(parse_pdf_date_year("D:20240115103000Z").as_deref(), Some("2024"));
    assert_eq!(parse_pdf_date_year("1988-05-01").as_deref(), Some("1988"));

    let mut be = vec![0xFE, 0xFF];
    for u in "Boundary Modes".encode_utf16() {
        be.extend_from_slice(&u.to_be_bytes());
    }
    assert_eq!(decode_pdf_string(&be), "Boundary Modes");

    assert_eq!(normalize_valid_isbn("978-0-306-40615-7").as_deref(), Some("9780306406157"));
    assert_eq!(normalize_valid_isbn("0-306-40615-2").as_deref(), Some("0306406152"));
    assert_eq!(normalize_valid_isbn("978-0-306-40615-8"), None);
}

#[test]
fn extracts_title_authors_year_doi_and_arxiv_with_exact_line_numbers_never_certain() {
    let reading = concat!(
        "arXiv:2402.00123v2 [math-ph] 14 Feb 2024\n",
        "Journal of Synthetic Mechanics, Vol. 12\n",
        "Spectral Stability of Coupled Oscillators\n",
        "on finite metric graphs\n",
        "Ada Example*1, Clara Lin†2 and Viktor Novak\n",
        "Department of Applied Mathematics\n",
        "Dated: March 2024\n",
        "DOI: 10.1000/synthetic.2024.42\n",
        "ISBN: 978-0-306-40615-7\n",
    );
    let c = extract_first_page_candidates(reading, Some("paper.pdf"));

    let title = c.title.expect("title candidate");
    assert_eq!(title.value, "Spectral Stability of Coupled Oscillators on finite metric graphs");
    assert_eq!(title.line, Some(3));
    assert_eq!(title.end_line, Some(4));
    assert_eq!(title.page, Some(1));
    assert!(!title.certain);

    let authors = c.authors.expect("authors candidate");
    assert_eq!(authors.value, vec!["Ada Example", "Clara Lin", "Viktor Novak"]);
    assert_eq!(authors.line, Some(5));
    assert_eq!(authors.page, Some(1));
    assert!(!authors.certain);

    let year = c.year.expect("year candidate");
    assert_eq!(year.value, "2024");
    assert_eq!(year.line, Some(7));
    assert_eq!(year.page, Some(1));
    assert!(!year.certain);

    assert_eq!(c.arxiv_ids.len(), 1);
    assert_eq!(c.arxiv_ids[0].value.id, "2402.00123");
    assert_eq!(c.arxiv_ids[0].value.version.as_deref(), Some("v2"));
    assert_eq!(c.arxiv_ids[0].line, Some(1));
    assert_eq!(c.arxiv_ids[0].page, Some(1));
    assert!(!c.arxiv_ids[0].certain);

    assert_eq!(c.dois.len(), 1);
    assert_eq!(c.dois[0].value, "10.1000/synthetic.2024.42");
    assert_eq!(c.dois[0].line, Some(8));
    assert_eq!(c.dois[0].page, Some(1));
    assert!(!c.dois[0].certain);

    assert_eq!(c.isbns.len(), 1);
    assert_eq!(c.isbns[0].value, "9780306406157");
    assert_eq!(c.isbns[0].line, Some(9));
    assert!(!c.isbns[0].certain);
}

#[test]
fn skips_blank_and_repository_cover_pages_and_rejects_broken_ocr() {
    let blank = "   \n  1  ";
    let cover = "Institutional Repository Cover\nThis version is available at https://repo.example.org/99\nAccepted Manuscript\nAda Example";
    let real = "© 2023 Example Press\nDiscrete Curvature Bounds on Trees\nClara Lin∗¹, Viktor Novak²\nLast compiled: May 2024";
    let doc = format!("{blank}\n\x0c\n{cover}\n\x0c\n{real}");

    let c = extract_first_page_candidates(&doc, Some("notes.pdf"));
    let title = c.title.expect("title on page 3");
    assert_eq!(title.value, "Discrete Curvature Bounds on Trees");
    assert_eq!(title.page, Some(3));
    assert_eq!(title.line, Some(10));
    assert!(!title.certain);

    let authors = c.authors.expect("authors on page 3");
    assert_eq!(authors.value, vec!["Clara Lin", "Viktor Novak"]);
    assert_eq!(authors.line, Some(11));

    let year = c.year.expect("year on page 3");
    assert_eq!(year.value, "2024");
    assert_eq!(year.line, Some(12));

    let broken_ocr = "ON TH E\nPROPERTIES O F A N\nEXAMPLE S Y S T E M\nBY\nA . S M I T H\n";
    let ocr_c = extract_first_page_candidates(broken_ocr, Some("scan_1972.pdf"));
    assert_eq!(ocr_c.title, None);
    assert_eq!(ocr_c.authors, None);
    let y = ocr_c.year.expect("filename year fallback");
    assert_eq!(y.value, "1972");
    assert_eq!(y.line, None);
    assert_eq!(y.source, CandidateSource::FileName);
    assert!(!y.certain);
}

#[test]
fn arxiv_doi_maps_to_arxiv_candidate_without_duplicating_doi() {
    let text = "Published preprint doi:10.48550/arXiv.2305.00421\n";
    let c = extract_first_page_candidates(text, None);
    assert!(c.dois.is_empty());
    assert_eq!(c.arxiv_ids.len(), 1);
    assert_eq!(c.arxiv_ids[0].value.id, "2305.00421");
    assert_eq!(c.arxiv_ids[0].line, Some(1));
}
