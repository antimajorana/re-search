use super::*;
use crate::{ingest_bytes, IngestRequest, ResearchEnv};
use std::sync::Mutex;

const SID: &str = "anchor-unit";

struct Fixture {
    _tmp: tempfile::TempDir,
    env: ResearchEnv,
    ledger: Ledger,
    primary_id: String,
    primary_path: String,
    ref_id: Option<String>,
    events: Vec<Event>,
}

impl Fixture {
    fn new(primary_text: &str, ref_text: Option<&str>, pdf_pages: Option<(usize, usize, Vec<(usize, usize, usize)>)>) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let env = ResearchEnv::new(tmp.path());
        let db = Mutex::new(Ledger::open(tmp.path()).unwrap());
        let upload = |name: &str, text: &str| {
            ingest_bytes(
                &env.sandbox(),
                &db,
                &IngestRequest {
                    session_id: SID,
                    file_name: name,
                    origin: "test",
                    raw_bytes: text.as_bytes(),
                    source_id_hint: None,
                    section_filter: None,
                    title: None,
                },
            )
            .unwrap()
        };
        let primary = upload("paper.txt", primary_text);
        let reference = ref_text.map(|t| upload("ref.txt", t));
        let ledger = db.into_inner().unwrap();
        if let Some((first, last, page_lines)) = pdf_pages {
            let mut receipt = ledger
                .read_session(SID)
                .unwrap()
                .iter()
                .find(|e| e.seq == primary.event_seq)
                .unwrap()
                .payload
                .clone();
            receipt["format"] = json!("pdf");
            receipt["page_count"] = json!(page_lines.len());
            receipt["pdf_page_lines"] = json!(page_lines);
            ledger.append(SID, "source.ingested", 1, &receipt).unwrap();
            ledger
                .append(
                    SID,
                    "investigation.paper",
                    1,
                    &json!({"source_id": primary.source_id, "page_start": first, "page_end": last}),
                )
                .unwrap();
        }
        let events = ledger.read_session(SID).unwrap();
        Self {
            _tmp: tmp,
            env,
            ledger,
            primary_id: primary.source_id,
            primary_path: primary.primary_path.unwrap(),
            ref_id: reference.map(|r| r.source_id),
            events,
        }
    }

    fn index(&self) -> SourceIndex<'_> {
        let receipts: BTreeMap<_, _> = self
            .events
            .iter()
            .filter(|e| e.kind == "source.ingested")
            .filter_map(|e| Some((e.payload["source_id"].as_str()?, e)))
            .collect();
        let primary = receipts[self.primary_id.as_str()];
        let pages = self
            .events
            .iter()
            .rev()
            .find(|e| e.kind == "investigation.paper")
            .and_then(|paper| SelectedPages::for_paper(&self.ledger, &paper.payload, &primary.payload).unwrap());
        SourceIndex {
            sandbox: self.env.sandbox(),
            session: SID,
            primary_source_id: &self.primary_id,
            receipts,
            files: BTreeMap::new(),
            bytes_left: MAX_TOTAL_BYTES,
            pages,
        }
    }

    fn pointer(&self, start_line: usize, end_line: usize, quote: &str) -> Anchor {
        Anchor {
            source_id: self.primary_id.clone(),
            path: self.primary_path.clone(),
            start_line,
            end_line,
            excerpt: None,
            quote: Some(quote.to_string()),
            evidence: None,
            kind: "passage".into(),
            label: None,
            file_sha256: String::new(),
            source_seq: 0,
            excerpt_basis: None,
            cited_range: None,
            range_basis: Vec::new(),
            excerpt_window: None,
        }
    }
}

#[test]
fn fold_matches_across_line_break() {
    let text = "Line one intro.\nWe consider two-dimensional\nquantum lattice models here.\nLine four.\n";
    let fx = Fixture::new(text, None, None);
    let mut idx = fx.index();
    let mut a = fx.pointer(2, 3, "two-dimensional quantum lattice models");
    let note = idx.verify_any("nodes[n1].anchors[0]", &mut a).unwrap();
    assert_eq!(note.basis, AnchorBasis::CitedLines);
    assert_eq!(a.start_line, 2);
    assert_eq!(a.end_line, 3);
    assert_eq!(a.excerpt_basis.as_deref(), Some("cited_lines"));
    assert_eq!(
        a.excerpt.as_deref(),
        Some("We consider two-dimensional\nquantum lattice models here.")
    );
}

#[test]
fn hyphen_slip_fails() {
    let text = "Context line.\nTherefore after mov-\ning the 4 dislocations into place.\nTail.\n";
    let fx = Fixture::new(text, None, None);
    let mut idx = fx.index();
    // Both a one-char slip and de-hyphenation fail (no de-hyphenation).
    let mut slip = fx.pointer(2, 3, "after mov- ning the 4 dislocations");
    assert_eq!(
        idx.verify_any("nodes[n1].anchors[0]", &mut slip).unwrap_err().code,
        "quote_not_found"
    );
    let mut dehyph = fx.pointer(2, 3, "after moving the 4 dislocations");
    assert_eq!(
        idx.verify_any("nodes[n1].anchors[0]", &mut dehyph).unwrap_err().code,
        "quote_not_found"
    );
}

#[test]
fn in_range_quote_narrows_to_span_plus_two() {
    let lines: Vec<String> = (1..=20)
        .map(|i| format!("Filler sentence number {i} in the paper."))
        .collect();
    let mut lines = lines;
    lines[9] = "The universal fusion rule holds for all defects.".into();
    let fx = Fixture::new(&lines.join("\n"), None, None);
    let mut idx = fx.index();
    let mut a = fx.pointer(1, 20, "universal fusion rule holds for all defects");
    let note = idx.verify_any("what.anchors[0]", &mut a).unwrap();
    assert_eq!((a.start_line, a.end_line), (8, 12));
    assert_eq!(a.cited_range, Some([1, 20]));
    assert_eq!(a.range_basis, vec!["narrowed"]);
    assert!(note.corrected.is_some());
}

#[test]
fn sixteen_char_quote_never_grounds_64_lines() {
    let mut lines: Vec<String> = (1..=64)
        .map(|i| format!("Generic line {i} discussing setup details."))
        .collect();
    lines[30] = "UniqueMarkerAlphaBetaGamma appears here.".into();
    let fx = Fixture::new(&lines.join("\n"), None, None);
    let mut idx = fx.index();
    let mut a = fx.pointer(1, 64, "UniqueMarkerAlpha"); // 17 non-space chars
    idx.verify_any("nodes[n1].anchors[0]", &mut a).unwrap();
    assert!(a.end_line - a.start_line + 1 <= 5);
    assert_eq!((a.start_line, a.end_line), (29, 33));
    assert_eq!(a.cited_range, Some([1, 64]));
    assert_eq!(a.range_basis, vec!["narrowed"]);
}

#[test]
fn whole_line_short_quote_only_when_range_is_that_line() {
    let text = "Line one text.\nE = mc^2\nLine three text.\n";
    let fx = Fixture::new(text, None, None);
    let mut idx = fx.index();
    // Exact whole-line range succeeds.
    let mut ok = fx.pointer(2, 2, "E = mc^2");
    idx.verify_any("nodes[n1].anchors[0]", &mut ok).unwrap();
    assert_eq!((ok.start_line, ok.end_line), (2, 2));
    // Wider range fails.
    let mut wide = fx.pointer(1, 3, "E = mc^2");
    assert_eq!(
        idx.verify_any("nodes[n1].anchors[0]", &mut wide).unwrap_err().code,
        "quote_too_short"
    );
    // Partial line fails.
    let mut part = fx.pointer(2, 2, "E = mc");
    assert_eq!(
        idx.verify_any("nodes[n1].anchors[0]", &mut part).unwrap_err().code,
        "quote_too_short"
    );
    // Shifted range (relocation) fails for short quote.
    let mut shifted = fx.pointer(3, 3, "E = mc^2");
    assert_eq!(
        idx.verify_any("nodes[n1].anchors[0]", &mut shifted).unwrap_err().code,
        "quote_too_short"
    );
}

#[test]
fn relocates_within_12() {
    let mut lines: Vec<String> = (1..=30).map(|i| format!("Filler line {i}.")).collect();
    lines[19] = "The anomaly inflow matches the boundary theory.".into(); // line 20
    let fx = Fixture::new(&lines.join("\n"), None, None);
    let mut idx = fx.index();
    // Cited 10..=11, actual is line 20 (offset +9, within +-12).
    let mut a = fx.pointer(10, 11, "anomaly inflow matches the boundary");
    let note = idx.verify_any("nodes[n1].anchors[0]", &mut a).unwrap();
    assert_eq!((a.start_line, a.end_line), (20, 20));
    assert_eq!(a.cited_range, Some([10, 11]));
    assert_eq!(a.range_basis, vec!["relocated"]);
    let corr = note.corrected.unwrap();
    assert_eq!(corr.lines, [20, 20]);
    assert_eq!(corr.from, [10, 11]);
}

#[test]
fn fails_at_20_with_true_text() {
    let mut lines: Vec<String> = (1..=40).map(|i| format!("Filler line number {i}.")).collect();
    lines[29] = "The far-away theorem appears on line thirty.".into(); // line 30
    let fx = Fixture::new(&lines.join("\n"), None, None);
    let mut idx = fx.index();
    let mut a = fx.pointer(10, 10, "far-away theorem appears on line thirty");
    let err = idx.verify_any("nodes[n1].anchors[0]", &mut a).unwrap_err();
    assert_eq!(err.code, "quote_not_found");
    let tt = err.true_text.expect("expected true_text");
    assert!(tt.contains("8| Filler line number 8."));
    assert!(tt.contains("10| Filler line number 10."));
    assert!(tt.contains("12| Filler line number 12."));
}

#[test]
fn ambiguous_lists_candidates() {
    let text = "Alpha.\nThe topological invariant equals unity.\nBeta.\nThe topological invariant equals unity.\nGamma.\n";
    let fx = Fixture::new(text, None, None);
    let mut idx = fx.index();
    let mut a = fx.pointer(1, 5, "topological invariant equals unity");
    let err = idx.verify_any("nodes[n1].anchors[0]", &mut a).unwrap_err();
    assert_eq!(err.code, "quote_ambiguous");
    assert!(err.message.contains("2") && err.message.contains("4"), "{}", err.message);
}

#[test]
fn label_one_line_above_extends() {
    let text = "Intro line.\nTABLE A2.\nRow one of the cocycle table values.\nRow two of the cocycle table values.\n";
    let fx = Fixture::new(text, None, None);
    let mut idx = fx.index();
    let mut a = fx.pointer(3, 4, "Row one of the cocycle table values");
    a.kind = "table".into();
    a.label = Some("TABLE A2.".into());
    idx.verify_any("items[c1].anchors[0]", &mut a).unwrap();
    assert_eq!((a.start_line, a.end_line), (2, 4));
    assert_eq!(a.cited_range, Some([3, 4]));
    assert!(a.range_basis.contains(&"label_extended".to_string()));
    assert!(a.excerpt.as_deref().unwrap().contains("TABLE A2."));
}

#[test]
fn label_five_above_fails() {
    let text = "TABLE A2.\nL2.\nL3.\nL4.\nL5.\nRow one of the cocycle table values.\n";
    let fx = Fixture::new(text, None, None);
    let mut idx = fx.index();
    let mut a = fx.pointer(6, 6, "Row one of the cocycle table values");
    a.kind = "table".into();
    a.label = Some("TABLE A2.".into());
    let err = idx.verify_any("items[c1].anchors[0]", &mut a).unwrap_err();
    assert_eq!(err.code, "label_not_found");
}

#[test]
fn section_name_label_refused_pointer_only() {
    let text = "Abstract\nWe prove a sharp scaling theorem for lattice models.\nI. Introduction\nDetails follow in section two.\n";
    let fx = Fixture::new(text, None, None);
    let mut idx = fx.index();
    for bad_label in ["Abstract", "Introduction", "I. Introduction", "2. Conclusions"] {
        let mut ptr = fx.pointer(1, 2, "sharp scaling theorem for lattice models");
        ptr.label = Some(bad_label.into());
        assert_eq!(
            idx.verify_any("what.anchors[0]", &mut ptr).unwrap_err().code,
            "label_is_section_name",
            "expected refusal for {bad_label}"
        );
    }
    // Legacy verbatim-excerpt anchor still accepts "Abstract" if present in cited lines.
    let mut legacy = Anchor {
        source_id: fx.primary_id.clone(),
        path: fx.primary_path.clone(),
        start_line: 1,
        end_line: 2,
        excerpt: Some("Abstract\nWe prove a sharp scaling theorem for lattice models.".into()),
        quote: None,
        evidence: None,
        kind: "passage".into(),
        label: Some("Abstract".into()),
        file_sha256: String::new(),
        source_seq: 0,
        excerpt_basis: None,
        cited_range: None,
        range_basis: Vec::new(),
        excerpt_window: None,
    };
    assert!(idx.verify_any("what.anchors[0]", &mut legacy).is_ok());
}

#[test]
fn extension_outside_selected_pages_fails() {
    let text = "TABLE A2.\nRow one of the cocycle table values.\nRow two of the cocycle table values.\n";
    let fx = Fixture::new(text, None, Some((2, 2, vec![(1, 1, 1), (2, 2, 3)])));
    let mut idx = fx.index();
    let mut a = fx.pointer(2, 3, "Row one of the cocycle table values");
    a.kind = "table".into();
    a.label = Some("TABLE A2.".into());
    let err = idx.verify_any("items[c1].anchors[0]", &mut a).unwrap_err();
    assert_eq!(err.code, "outside_selected_pages");
}

#[test]
fn crlf_source() {
    let text = "First line.\r\nWe verify CRLF line endings\r\nacross line breaks cleanly.\r\nLast line.\r\n";
    let fx = Fixture::new(text, None, None);
    let mut idx = fx.index();
    let mut a = fx.pointer(2, 3, "CRLF line endings across line breaks");
    idx.verify_any("nodes[n1].anchors[0]", &mut a).unwrap();
    assert_eq!((a.start_line, a.end_line), (2, 3));
    assert!(!a.excerpt.as_deref().unwrap().contains('\r'));
}

#[test]
fn trailing_blank_trimmed() {
    let text = "\nWe verify the exact sequence here.\n   \n";
    let fx = Fixture::new(text, None, None);
    let mut idx = fx.index();
    let mut a = fx.pointer(1, 3, "verify the exact sequence here");
    idx.verify_any("items[c1].anchors[0]", &mut a).unwrap();
    assert_eq!((a.start_line, a.end_line), (2, 2));
    assert_eq!(a.cited_range, Some([1, 3]));
    assert!(a.range_basis.contains(&"trimmed".to_string()));
}

#[test]
fn long_line_gets_excerpt_window_and_reslices() {
    let prefix = "alpha ".repeat(400); // 2400 bytes
    let mid = "CRITICAL_EQUATION_SIGNATURE_42";
    let suffix = " omega".repeat(400); // 2400 bytes
    let line = format!("{prefix}{mid}{suffix}");
    assert!(line.len() > MAX_EXCERPT_BYTES);
    let fx = Fixture::new(&line, None, None);
    let mut idx = fx.index();
    let mut a = fx.pointer(1, 1, mid);
    idx.verify_any("nodes[n1].anchors[0]", &mut a).unwrap();
    let window = a.excerpt_window.expect("expected excerpt_window");
    let excerpt = a.excerpt.as_deref().unwrap();
    assert!(excerpt.len() <= MAX_EXCERPT_BYTES);
    assert!(excerpt.contains(mid));
    assert_eq!(&line[window[0]..window[1]], excerpt);
}

#[test]
fn true_text_clipped_to_selected_pages() {
    let text = "Page one secret line.\nPage two first line.\nPage two second line.\nPage three secret line.\n";
    let fx = Fixture::new(
        text,
        None,
        Some((2, 2, vec![(1, 1, 1), (2, 2, 3), (3, 4, 4)])),
    );
    let mut idx = fx.index();
    let mut a = fx.pointer(2, 3, "nonexistent phrase for testing clip");
    let err = idx.verify_any("nodes[n1].anchors[0]", &mut a).unwrap_err();
    assert_eq!(err.code, "quote_not_found");
    let tt = err.true_text.expect("expected clipped true_text");
    assert!(tt.contains("2| Page two first line."));
    assert!(tt.contains("3| Page two second line."));
    assert!(!tt.contains("Page one secret"));
    assert!(!tt.contains("Page three secret"));
}

#[test]
fn app_only_field_refused() {
    for &field in APP_ONLY_FIELDS {
        let raw = json!({
            "nodes": [{"anchors": [{"source_id": "p", "kind": "passage", field: "spoofed"}]}]
        });
        assert!(reject_app_only_fields(&raw).is_err(), "expected {field} refused");
    }
    let clean = json!({"nodes": [{"anchors": [{"source_id": "p", "kind": "passage", "quote": "real text here now"}]}]});
    assert!(reject_app_only_fields(&clean).is_ok());
}

#[test]
fn excerpt_and_quote_together_refused() {
    let text = "We assume positive mass throughout this paper.\n";
    let fx = Fixture::new(text, None, None);
    let mut idx = fx.index();
    let mut a = fx.pointer(1, 1, "assume positive mass throughout");
    a.excerpt = Some("We assume positive mass throughout this paper.".into());
    let err = idx.verify_any("nodes[n1].anchors[0]", &mut a).unwrap_err();
    assert_eq!(err.code, "invalid_anchor_form");
    // Evidence anchor is also refused in Phase 1.
    let mut ev = fx.pointer(1, 1, "assume positive mass throughout");
    ev.quote = None;
    ev.evidence = Some("ev-12".into());
    assert_eq!(
        idx.verify_any("nodes[n1].anchors[0]", &mut ev).unwrap_err().code,
        "evidence_unavailable"
    );
}

#[test]
fn path_omitted_primary_filled_reference_refused() {
    let fx = Fixture::new(
        "Primary paper theorem about scaling laws.\n",
        Some("Reference paper theorem about scaling laws.\n"),
        None,
    );
    let mut idx = fx.index();
    let mut primary = fx.pointer(1, 1, "Primary paper theorem about scaling");
    primary.path.clear();
    idx.verify_any("nodes[n1].anchors[0]", &mut primary).unwrap();
    assert_eq!(primary.path, fx.primary_path);

    let mut reference = fx.pointer(1, 1, "Reference paper theorem about scaling");
    reference.source_id = fx.ref_id.clone().unwrap();
    reference.path.clear();
    let err = idx.verify_any("nodes[ref].anchors[1]", &mut reference).unwrap_err();
    assert_eq!(err.code, "path_required");
}
