//! Evaluation plan Set P5 (00-overview.md §7.2, §7.3; packages V11, V17, V18, F14a):
//!
//! Runs deterministic skeleton construction (`tex-1`, `html-1`, `pdf-1`) and V11
//! `locate_in_skeleton_with_context` across all 10 fixed-tier cited-statement cases in
//! `tests/fixtures/eval/manifest-p5.json` (`tests/fixtures/eval/p5/*`): 7 TeX/HTML cases plus 3
//! PDF-only cases, validating non-builder raw source consistency, PDF physical-page and F14a scan
//! honesty, two-column reading order, and mandatory F8 `wrong_statement_located == 0`.

mod eval_set_p5_support;

use eval_set_p5_support::manifest::{parse_manifest, validate_manifest, Case, Input, Set, Tier};
use eval_set_p5_support::measures::{evaluate, Outcome};
use eval_set_p5_support::{eval_fixtures_dir, verify_statement_key_against_raw_source};
use ledger::Ledger;
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::paper_key::PaperKey;
use research_tools::pdf_pages::PageBasis;
use research_tools::sandbox::WorkspaceSandbox;
use research_tools::skeleton::{
    build_html_skeleton_from_ingested, build_pdf_skeleton_from_ingested,
    build_tex_skeleton_from_ingested, detect_tex_counter_drift, load_ingested_pdf_source,
    load_ingested_tex_source, locate_in_skeleton_with_context, LocateCandidate, LocateContext,
    LocateResult, LocateStatus, NumberBasis, PaperSkeleton, PdfSkeletonOutput, SkNode, SkNodeKind,
    NO_TEX_REASON_EMPTY_TEXT_LAYER,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use tempfile::TempDir;

struct BuiltCase {
    skeleton: PaperSkeleton,
    pdf: Option<PdfSkeletonOutput>,
    drift_reason: Option<String>,
    single_counter_confirmed: bool,
}

fn load_set_p5_cases() -> Vec<Case> {
    let text = fs::read_to_string(eval_fixtures_dir().join("manifest-p5.json")).unwrap();
    let m = parse_manifest(&text).unwrap();
    assert_eq!(m.set, Set::P5);
    assert_eq!(m.tier, Tier::Fixed);
    assert_eq!(validate_manifest(&m), Vec::<String>::new());
    m.cases
}

fn build_case_skeleton(data_dir: &Path, session_id: &str, rel_file: &str, format: &str) -> BuiltCase {
    let raw_bytes = fs::read(eval_fixtures_dir().join(rel_file)).unwrap();
    let file_name = Path::new(rel_file).file_name().unwrap().to_str().unwrap();
    let key = PaperKey::from_file_bytes(&raw_bytes);
    let ledger = Mutex::new(Ledger::open(data_dir).unwrap());
    let sandbox = WorkspaceSandbox::new(data_dir, None);
    let ing = ingest_bytes(
        &sandbox,
        &ledger,
        &IngestRequest {
            session_id,
            file_name,
            origin: "p5_fixture",
            raw_bytes: &raw_bytes,
            source_id_hint: None,
            section_filter: None,
            title: None,
        },
    )
    .unwrap();
    let events = ledger.lock().unwrap().read_session(session_id).unwrap();
    let receipt = events.iter().find(|e| e.seq == ing.event_seq).unwrap();

    match format {
        "tex" => {
            let tex = load_ingested_tex_source(&sandbox, session_id, receipt).unwrap();
            let (out, _) =
                build_tex_skeleton_from_ingested(&sandbox, session_id, receipt, key, None).unwrap();
            let drift =
                detect_tex_counter_drift(&tex.files, Some(&out.theorem_table), &out.skeleton.nodes);
            let confirmed = drift.is_none();
            BuiltCase {
                skeleton: out.skeleton,
                pdf: None,
                drift_reason: drift,
                single_counter_confirmed: confirmed,
            }
        }
        "html" => {
            let (out, _) =
                build_html_skeleton_from_ingested(&sandbox, session_id, receipt, key, None).unwrap();
            BuiltCase {
                skeleton: out.skeleton,
                pdf: None,
                drift_reason: None,
                single_counter_confirmed: false,
            }
        }
        "pdf" => {
            let (out, _) =
                build_pdf_skeleton_from_ingested(&sandbox, session_id, receipt, key, None).unwrap();
            let _ = load_ingested_pdf_source(&sandbox, session_id, receipt).unwrap();
            BuiltCase {
                skeleton: out.skeleton.clone(),
                pdf: Some(out),
                drift_reason: None,
                single_counter_confirmed: false,
            }
        }
        other => panic!("unsupported Set P5 fixture format `{other}`"),
    }
}

fn locate_case(built: &BuiltCase, locator: &str) -> LocateResult {
    locate_in_skeleton_with_context(
        &built.skeleton,
        locator,
        &LocateContext {
            page_map: built.pdf.as_ref().map(|p| &p.page_map),
            page_lines: None,
            counter_drift_reason: built.drift_reason.as_deref(),
            single_counter_confirmed: built.single_counter_confirmed,
        },
    )
}

fn find_node<'a>(skeleton: &'a PaperSkeleton, tag: &str) -> &'a SkNode {
    skeleton
        .nodes
        .iter()
        .find(|n| n.tag.as_deref() == Some(tag))
        .unwrap_or_else(|| panic!("missing node tag `{tag}`"))
}

fn parse_kind(raw: &str) -> SkNodeKind {
    match raw {
        "theorem" => SkNodeKind::Theorem,
        "lemma" => SkNodeKind::Lemma,
        "proposition" => SkNodeKind::Proposition,
        "corollary" => SkNodeKind::Corollary,
        "definition" => SkNodeKind::Definition,
        "equation" => SkNodeKind::Equation,
        other => panic!("unsupported expected kind `{other}`"),
    }
}

fn parse_basis(raw: &str) -> NumberBasis {
    match raw {
        "computed" => NumberBasis::Computed,
        "printed" => NumberBasis::Printed,
        "html" => NumberBasis::Html,
        other => panic!("unsupported expected number_basis `{other}`"),
    }
}

fn candidate_matches(c: &LocateCandidate, kind: SkNodeKind, number: Option<&str>, title: Option<&str>) -> bool {
    c.kind == kind
        && number.is_none_or(|n| c.number.as_deref() == Some(n))
        && title.is_none_or(|t| c.title.as_deref().is_some_and(|got| got.contains(t)))
}

#[test]
fn eval_set_p5_ten_cited_statement_cases_pass() {
    let cases = load_set_p5_cases();
    assert_eq!(cases.len(), 10, "Set P5 requires 10 cited-statement cases");
    let pdf_count = cases
        .iter()
        .filter(|c| c.answer_key["statement"]["format"] == "pdf")
        .count();
    assert!(pdf_count >= 3, "Set P5 requires at least 3 PDF-only cases, got {pdf_count}");

    let tmp = TempDir::new().unwrap();
    let mut wrong_statement_located = 0usize;
    let mut pdf_scanned_reported_plain_text = 0usize;
    let mut pdf_two_col_wrong_order_lines = 0usize;

    for (idx, case) in cases.iter().enumerate() {
        let (rel_file, locator) = match &case.input {
            Input::Fixture { files, locator } => (
                files[0].as_str(),
                locator.as_deref().expect("Set P5 case requires input.locator"),
            ),
            other => panic!("{}: unexpected input `{other:?}`", case.id),
        };
        let stmt = &case.answer_key["statement"];
        verify_statement_key_against_raw_source(&case.id, rel_file, stmt);

        let format = stmt["format"].as_str().unwrap();
        let data_dir = tmp.path().join(format!("case_{idx}"));
        let sid = format!("s-p5-{:02}", idx + 1);
        let built = build_case_skeleton(&data_dir, &sid, rel_file, format);
        let res = locate_case(&built, locator);
        let want_kind = parse_kind(stmt["kind"].as_str().unwrap());

        if case.id == "P5-eval-04" {
            let want_num = stmt["number"].as_str().unwrap();
            let want_title = stmt["title"].as_str();
            let wrong_num = stmt["wrong_number_must_not_locate"].as_str().unwrap();
            match &res.outcome {
                LocateStatus::Found { tag, .. } => {
                    let node = find_node(&built.skeleton, tag);
                    if node.number.as_deref() == Some(wrong_num) || node.number.as_deref() != Some(want_num) {
                        wrong_statement_located += 1;
                    }
                }
                LocateStatus::Candidates { candidates, .. }
                | LocateStatus::PaperFoundLocatorNotFound { candidates, .. } => {
                    assert!(
                        candidates.iter().any(|c| candidate_matches(c, want_kind, Some(want_num), want_title)),
                        "P5-eval-04 candidates must include `{want_num}`"
                    );
                }
                other => panic!("P5-eval-04 unexpected outcome `{other:?}`"),
            }
            let roman_direct = locate_case(&built, "Theorem II");
            match &roman_direct.outcome {
                LocateStatus::Found { tag, number_basis } => {
                    assert_eq!(*number_basis, NumberBasis::Html);
                    let node = find_node(&built.skeleton, tag);
                    assert_eq!(node.number.as_deref(), Some("II"));
                    assert_eq!(node.title.as_deref(), want_title);
                }
                other => panic!("P5-eval-04 direct Roman lookup expected Found, got `{other:?}`"),
            }
            continue;
        }

        let expected_status = stmt["expected_status"].as_str().unwrap();
        match expected_status {
            "found" => {
                let want_basis = parse_basis(stmt["number_basis"].as_str().unwrap());
                let want_num = stmt["number"].as_str().unwrap();
                match &res.outcome {
                    LocateStatus::Found { tag, number_basis } => {
                        let node = find_node(&built.skeleton, tag);
                        let label_ok = stmt
                            .get("label")
                            .and_then(Value::as_str)
                            .is_none_or(|l| node.label.as_deref() == Some(l));
                        if node.kind != want_kind || node.number.as_deref() != Some(want_num) || !label_ok {
                            wrong_statement_located += 1;
                        }
                        assert_eq!(*number_basis, want_basis, "{}: number_basis", case.id);
                    }
                    other => panic!("{}: expected Found, got `{other:?}`", case.id),
                }
            }
            "paper_found_locator_not_found" => match &res.outcome {
                LocateStatus::Found { tag, .. } => {
                    panic!("{}: missing locator must never return Found (`{tag}`)", case.id);
                }
                LocateStatus::PaperFoundLocatorNotFound { candidates, .. } => {
                    let want_labels: Vec<&str> = stmt["candidate_labels"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_str().unwrap())
                        .collect();
                    for lbl in want_labels {
                        assert!(
                            candidates
                                .iter()
                                .any(|c| find_node(&built.skeleton, &c.tag).label.as_deref() == Some(lbl)),
                            "{}: missing fallback candidate `{lbl}`",
                            case.id
                        );
                    }
                }
                other => panic!("{}: expected PaperFoundLocatorNotFound, got `{other:?}`", case.id),
            },
            "candidates" => match &res.outcome {
                LocateStatus::Found { tag, .. } => {
                    panic!("{}: ambiguous/drifted/PDF case must never return Found (`{tag}`)", case.id);
                }
                LocateStatus::Candidates { candidates, .. } => {
                    assert!(!candidates.is_empty(), "{}: candidates must be non-empty", case.id);
                    if stmt.get("counter_drifted").and_then(Value::as_bool) == Some(true) {
                        assert!(built.drift_reason.is_some(), "{}: expected detected counter drift", case.id);
                        let lbl = stmt["label"].as_str().unwrap();
                        assert!(
                            candidates
                                .iter()
                                .any(|c| find_node(&built.skeleton, &c.tag).label.as_deref() == Some(lbl)),
                            "{}: missing `{lbl}`",
                            case.id
                        );
                    } else if let Some(cand_labels) = stmt.get("candidate_labels").and_then(Value::as_array) {
                        for v in cand_labels {
                            let lbl = v.as_str().unwrap();
                            assert!(
                                candidates
                                    .iter()
                                    .any(|c| find_node(&built.skeleton, &c.tag).label.as_deref() == Some(lbl)),
                                "{}: missing `{lbl}`",
                                case.id
                            );
                        }
                    } else {
                        let want_num = stmt.get("number").and_then(Value::as_str);
                        let want_title = stmt.get("title").and_then(Value::as_str);
                        let matched = candidates
                            .iter()
                            .find(|c| candidate_matches(c, want_kind, want_num, want_title))
                            .unwrap_or_else(|| panic!("{}: target candidate missing from `{candidates:?}`", case.id));
                        let want_page = stmt["page"].as_u64().unwrap() as usize;
                        assert_eq!(matched.start_page, Some(want_page), "{}: start_page", case.id);
                        assert_eq!(matched.end_page, Some(want_page), "{}: end_page", case.id);
                        assert_eq!(matched.ocr, stmt["ocr"].as_bool().unwrap(), "{}: ocr flag", case.id);
                        assert_eq!(
                            matched.render_math,
                            stmt["render_math"].as_bool().unwrap(),
                            "{}: render_math flag",
                            case.id
                        );

                        let pdf = built.pdf.as_ref().unwrap();
                        let avail = pdf.pages.iter().find(|p| p.page == want_page).unwrap();
                        let want_basis = match stmt["page_basis"].as_str().unwrap() {
                            "text_layer" => PageBasis::TextLayer,
                            "scan_text_layer" => PageBasis::ScanTextLayer,
                            other => panic!("unexpected page_basis `{other}`"),
                        };
                        if want_basis == PageBasis::ScanTextLayer && avail.basis == PageBasis::TextLayer {
                            pdf_scanned_reported_plain_text += 1;
                        }
                        assert_eq!(avail.basis, want_basis, "{}: page basis", case.id);

                        if let Some(exp_sec) = stmt.get("expected_section").and_then(Value::as_str) {
                            let thm_pos = built.skeleton.nodes.iter().position(|n| n.tag.as_deref() == Some(&matched.tag)).unwrap();
                            let sec_pos = built.skeleton.nodes.iter().position(|n| n.title.as_deref() == Some(exp_sec));
                            let parent_tag = find_node(&built.skeleton, &matched.tag).parent.as_deref();
                            let parent_title = parent_tag.and_then(|pt| find_node(&built.skeleton, pt).title.as_deref());
                            if sec_pos.is_none_or(|sp| sp >= thm_pos) || parent_title != Some(exp_sec) {
                                pdf_two_col_wrong_order_lines += 1;
                            }
                            assert_eq!(parent_title, Some(exp_sec), "{}: two-column section parent", case.id);
                        }
                        if let Some(img_page) = stmt.get("image_only_page").and_then(Value::as_u64) {
                            let img_avail = pdf.pages.iter().find(|p| p.page == img_page as usize).unwrap();
                            assert_eq!(img_avail.basis, PageBasis::None, "{}: image-only page basis", case.id);
                        }
                    }
                }
                other => panic!("{}: expected Candidates, got `{other:?}`", case.id),
            },
            other => panic!("{}: unsupported expected_status `{other}`", case.id),
        }
    }

    assert_eq!(wrong_statement_located, 0, "F8 Set P5 requires wrong_statement_located == 0");
    assert_eq!(pdf_scanned_reported_plain_text, 0);
    assert_eq!(pdf_two_col_wrong_order_lines, 0);

    let values = BTreeMap::from([
        ("wrong_statement_located".to_string(), wrong_statement_located as f64),
        ("pdf_scanned_reported_plain_text".to_string(), pdf_scanned_reported_plain_text as f64),
        ("pdf_two_col_wrong_order_lines".to_string(), pdf_two_col_wrong_order_lines as f64),
    ]);
    let report = evaluate(&values).unwrap();
    assert_eq!(report.outcome_of("wrong_statement_located"), Some(Outcome::Pass));
    assert_eq!(report.outcome_of("pdf_scanned_reported_plain_text"), Some(Outcome::Pass));
}

fn build_image_only_pdf() -> Vec<u8> {
    let objs: [(usize, &[u8]); 5] = [
        (1, b"<< /Type /Catalog /Pages 2 0 R >>"),
        (2, b"<< /Type /Pages /Kids [4 0 R] /Count 1 >>"),
        (3, b"<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /ASCIIHexDecode /Length 7 >>\nstream\n808080>\nendstream"),
        (4, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Im1 3 0 R >> >> /Contents 5 0 R >>"),
        (5, b"<< /Length 31 >>\nstream\nq 560 0 0 720 26 36 cm /Im1 Do Q\nendstream"),
    ];
    let mut out = Vec::from(&b"%PDF-1.4\n"[..]);
    let mut offsets = [0usize; 6];
    for (id, body) in objs {
        offsets[id] = out.len();
        out.extend_from_slice(format!("{id} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref_pos = out.len();
    out.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for off in &offsets[1..] {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n").as_bytes());
    out
}

#[test]
fn empty_text_layer_image_only_pdf_resolves_to_no_tex() {
    let raw = build_image_only_pdf();
    let tmp = TempDir::new().unwrap();
    let ledger = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let sandbox = WorkspaceSandbox::new(tmp.path(), None);
    let ing = ingest_bytes(
        &sandbox,
        &ledger,
        &IngestRequest {
            session_id: "s-empty-pdf",
            file_name: "image_only.pdf",
            origin: "p5_edge",
            raw_bytes: &raw,
            source_id_hint: None,
            section_filter: None,
            title: None,
        },
    )
    .unwrap();
    assert_eq!(ing.status, NO_TEX_REASON_EMPTY_TEXT_LAYER);
    let events = ledger.lock().unwrap().read_session("s-empty-pdf").unwrap();
    let receipt = events.iter().find(|e| e.seq == ing.event_seq).unwrap();
    let (pdf_out, _) = build_pdf_skeleton_from_ingested(
        &sandbox,
        "s-empty-pdf",
        receipt,
        PaperKey::from_file_bytes(&raw),
        None,
    )
    .unwrap();
    assert_eq!(pdf_out.pages[0].basis, PageBasis::None);
    let res = locate_in_skeleton_with_context(
        &pdf_out.skeleton,
        "Theorem 1.1",
        &LocateContext { page_map: Some(&pdf_out.page_map), ..LocateContext::default() },
    );
    match res.outcome {
        LocateStatus::NoTex { reason } => assert_eq!(reason, NO_TEX_REASON_EMPTY_TEXT_LAYER),
        other => panic!("expected NoTex(empty_text_layer), got `{other:?}`"),
    }
}
