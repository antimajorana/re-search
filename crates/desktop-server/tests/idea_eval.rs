//! Explore Evaluation Suite — Set I (Checkpoint 1, package X14).
//!
//! Governance: `00-overview.md` §0.1, §7.1–7.3, §8.2 #12; `explore.md` §9 & §12;
//! `docs/design/modes-2026-11/agents/exp-x14-eval.md`.
//!
//! Runs strictly on self-authored fixed-tier fixtures (`tests/fixtures/eval/manifest-i.json`,
//! `tests/fixtures/eval/i/*`), loopback `RoutedMock` recordings, and `ScriptedExploreTransport`:
//! zero third-party paper text, zero external network calls, zero live model calls.

mod common;
mod idea_eval_support;

use idea_eval_support::exchanges::{run_scripted_x6_x7_x8_suite, ExchangeEvalOutcome};
use idea_eval_support::manifest::{Input, Tier};
use idea_eval_support::measures::{count_in, evaluate, share, GateVerdict, Outcome};
use idea_eval_support::pdf_eval::{evaluate_pdf_fixture, PdfEvalMetrics, PdfSentenceExpectation};
use idea_eval_support::recall::{run_search_recall_regime, RecallOutcome, RecallRegime};
use idea_eval_support::sheet::{
    current_build, render_markdown, today_utc, validate_sheet, Sheet,
};
use idea_eval_support::{load_set_i_cases, repo_root, set_i_dir, start_set_i_mock};
use research_tools::paper_key::PaperKey;
use research_tools::pdf_pages::PageBasis;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::sync::OnceLock;
use std::time::Instant;

struct SharedEvalRun {
    keyless_no_lib: RecallOutcome,
    keyed_no_lib: RecallOutcome,
    keyless_with_lib: RecallOutcome,
    keyed_with_lib: RecallOutcome,
    exchanges: ExchangeEvalOutcome,
    pdf: PdfEvalMetrics,
    wall_time_s: f64,
}

fn shared_eval_run() -> &'static SharedEvalRun {
    static RUN: OnceLock<SharedEvalRun> = OnceLock::new();
    RUN.get_or_init(|| {
        let t0 = Instant::now();
        let cases = load_set_i_cases();
        let mock = start_set_i_mock();

        let keyless_no_lib = run_search_recall_regime(
            &cases, &mock.base_url,
            RecallRegime { label: "keyless-nolib", with_keys: false, with_library: false },
        );
        let keyed_no_lib = run_search_recall_regime(
            &cases, &mock.base_url,
            RecallRegime { label: "keyed-nolib", with_keys: true, with_library: false },
        );
        let keyless_with_lib = run_search_recall_regime(
            &cases, &mock.base_url,
            RecallRegime { label: "keyless-lib", with_keys: false, with_library: true },
        );
        let keyed_with_lib = run_search_recall_regime(
            &cases, &mock.base_url,
            RecallRegime { label: "keyed-lib", with_keys: true, with_library: true },
        );
        let exchanges = run_scripted_x6_x7_x8_suite(&cases, &mock.base_url);
        mock.assert_no_unexpected();
        let pdf = run_explore_pdf_suite();
        let wall_time_s = t0.elapsed().as_secs_f64();

        SharedEvalRun {
            keyless_no_lib, keyed_no_lib, keyless_with_lib, keyed_with_lib,
            exchanges, pdf, wall_time_s,
        }
    })
}

#[test]
fn set_i_manifest_structure_and_non_builder_key_invariants() {
    let cases = load_set_i_cases();
    assert_eq!(cases.len(), 10, "Set I must contain exactly 10 Explore cases");

    let recording_text = fs::read_to_string(set_i_dir().join("recordings/set_i_services.json"))
        .expect("read set_i_services.json");
    assert!(!recording_text.contains('\r'), "set_i_services.json must be LF-only");

    let mut categories: BTreeMap<&str, usize> = BTreeMap::new();
    let mut lib_only_cases = 0usize;
    let mut total_key_papers = 0usize;

    for (idx, case) in cases.iter().enumerate() {
        assert_eq!(case.id, format!("I-{:02}", idx + 1));
        let Input::Idea { text } = &case.input else {
            panic!("{}: Set I input must be Idea", case.id);
        };
        let cat = case.answer_key["category"].as_str().unwrap();
        *categories.entry(cat).or_insert(0) += 1;
        let is_lib_only = case.answer_key["library_only_key_paper"].as_bool().unwrap();
        if is_lib_only {
            lib_only_cases += 1;
        }

        let parts = case.answer_key["parts"].as_array().unwrap();
        assert!(!parts.is_empty(), "{}: answer_key.parts must be non-empty", case.id);
        let mut part_ids = BTreeSet::new();
        for p in parts {
            let pid = p["id"].as_str().unwrap();
            assert!(part_ids.insert(pid), "{}: duplicate part id {pid}", case.id);
            let quote = p["idea_quote"].as_str().unwrap();
            assert!(
                text.contains(quote),
                "{}.{pid}: idea_quote `{quote}` must be an exact substring of input.text",
                case.id
            );
            let status = p["expected_status"].as_str().unwrap();
            assert!(
                matches!(
                    status,
                    "judged_same" | "judged_other_name" | "judged_partial" | "none_in_scope"
                ),
                "{}.{pid}: invalid expected_status `{status}`",
                case.id
            );
            if status == "judged_other_name" {
                assert!(
                    p["other_name"].as_str().is_some_and(|s| !s.trim().is_empty()),
                    "{}.{pid}: judged_other_name requires non-empty other_name",
                    case.id
                );
            }
        }

        let kps = case.answer_key["expected_key_papers"].as_array().unwrap();
        if cat == "open_as_far_as_known" {
            assert!(kps.is_empty(), "{}: open case must have empty expected_key_papers", case.id);
        } else {
            assert!(!kps.is_empty(), "{}: non-open case must list expected_key_papers", case.id);
        }
        let mut any_lib_only_paper = false;
        for kp in kps {
            total_key_papers += 1;
            let raw_key = kp["key"].as_str().unwrap();
            let parsed = PaperKey::parse(raw_key).expect("valid F2 canonical PaperKey");
            assert_eq!(parsed.as_str(), raw_key);
            let lib_only = kp["library_only"].as_bool().unwrap();
            let avail: Vec<&str> = kp["available_in"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            if lib_only {
                any_lib_only_paper = true;
                assert_eq!(avail, ["user_library"]);
                assert!(
                    !recording_text.contains(parsed.value()),
                    "{}: library-only key `{raw_key}` leaked into external service recording",
                    case.id
                );
            } else {
                assert!(!avail.is_empty() && !avail.contains(&"user_library"));
            }
        }
        assert_eq!(is_lib_only, any_lib_only_paper, "{}: library_only_key_paper mismatch", case.id);
    }

    // Exact 00-overview.md §7.2 breakdown: 3 done, 2 other name, 3 partly, 2 open; 2 library-only.
    assert_eq!(categories.get("done").copied().unwrap_or(0), 3);
    assert_eq!(categories.get("done_other_name").copied().unwrap_or(0), 2);
    assert_eq!(categories.get("done_partly").copied().unwrap_or(0), 3);
    assert_eq!(categories.get("open_as_far_as_known").copied().unwrap_or(0), 2);
    assert_eq!(lib_only_cases, 2);
    assert_eq!(total_key_papers, 10);
}

#[test]
fn set_i_search_only_recall_matrix_and_latency() {
    let run = shared_eval_run();

    // Known ground-truth counts across the 10 key papers in I-01..I-08:
    // - Keyless (arXiv + INSPIRE + Crossref), no library: 6/10 = 0.60
    // - With keys (+ Semantic Scholar + OpenAlex), no library: 8/10 = 0.80
    // - Keyless + F10/F13a user library: 8/10 = 0.80
    // - With keys + F10/F13a user library: 10/10 = 1.00
    assert_eq!((run.keyless_no_lib.found, run.keyless_no_lib.total), (6, 10));
    assert_eq!((run.keyed_no_lib.found, run.keyed_no_lib.total), (8, 10));
    assert_eq!((run.keyless_with_lib.found, run.keyless_with_lib.total), (8, 10));
    assert_eq!((run.keyed_with_lib.found, run.keyed_with_lib.total), (10, 10));

    assert!(run.keyed_no_lib.share() > run.keyless_no_lib.share());
    assert!(run.keyless_with_lib.share() > run.keyless_no_lib.share());
    assert!(run.keyed_with_lib.share() > run.keyed_no_lib.share());

    for lib_key in ["doi:10.9999/eval.i03.paywalled", "doi:10.9999/eval.i08.paywalled"] {
        assert!(run.keyless_with_lib.library_owned_hits.contains(lib_key));
        assert!(run.keyed_with_lib.library_owned_hits.contains(lib_key));
        assert!(!run.keyless_no_lib.library_owned_hits.contains(lib_key));
        assert!(!run.keyed_no_lib.library_owned_hits.contains(lib_key));
    }

    let max_t = run
        .keyless_no_lib
        .max_time_to_candidates_s
        .max(run.keyed_no_lib.max_time_to_candidates_s)
        .max(run.keyless_with_lib.max_time_to_candidates_s)
        .max(run.keyed_with_lib.max_time_to_candidates_s);
    assert!(max_t < 6.0, "max time_to_candidates_s {max_t:.3}s >= 6.0s");
}

#[test]
fn set_i_scripted_sharpen_search_triage_and_rule_s8() {
    let out = &shared_eval_run().exchanges;

    assert_eq!(out.cases_run, 10);
    assert_eq!(out.runs_above_cap, 0);
    // 10 Sharpen + 10 Triage + 1 Rule C3 repair on I-05 OpenAlex rebuilt abstract = 21 calls.
    assert_eq!(out.total_model_calls, 21);
    assert_eq!(out.abstract_evidence_total, 7, "7 verbatim abstract quotes across I-01,02,04,06,07(x2),08");
    assert!(out.openalex_non_verbatim_rejected, "I-05 OpenAlex rebuilt abstract must be rejected by Rule C3");
    assert!(out.quick_preset_zero_papers_verified, "Quick preset must enforce max_papers == 0 and papers_fetched == 0");
    assert!(out.s8_audit.checked >= 150, "expected >= 150 Rule S8 wording checks, got {}", out.s8_audit.checked);
    assert_eq!(out.s8_audit.failures, 0, "Rule S8 forbidden-word check must have 0 failures");
}

#[test]
fn set_i_explore_pdf_and_scan_fixtures_f14_readiness() {
    let m = &shared_eval_run().pdf;
    assert_eq!(m.sentences_total, 7);
    assert_eq!(m.sentences_matched, 7);
    assert_eq!(m.wrong_page_chips, 0);
    assert_eq!(m.scanned_pages_checked, 3);
    assert_eq!(m.scanned_reported_plain_text, 0);
    assert_eq!(m.scanned_quotes_total, 2);
    assert_eq!(m.scanned_quotes_tagged_ocr, 2);
    assert_eq!(m.two_col_wrong_order_lines, 0);
    assert_eq!(m.image_only_pages_needing_ocr, 1);
    assert_eq!(m.lines_not_sent_rejections, 3);
    assert_eq!(m.fabricated_quote_rejections, 3);
}

#[test]
fn set_i_f8_sheet_evaluation_and_markdown_render() {
    let cases = load_set_i_cases();
    let run = shared_eval_run();
    let max_t = run
        .keyless_no_lib
        .max_time_to_candidates_s
        .max(run.keyed_no_lib.max_time_to_candidates_s)
        .max(run.keyless_with_lib.max_time_to_candidates_s)
        .max(run.keyed_with_lib.max_time_to_candidates_s);

    let mut sheet = Sheet::blank(Tier::Fixed);
    sheet.date = today_utc();
    sheet.build = current_build(&repo_root(), env!("CARGO_PKG_VERSION"));
    sheet.models = vec!["scripted-explore-eval-model".into()];
    sheet.model_calls = Some(run.exchanges.total_model_calls);
    sheet.wall_time_s = Some(run.wall_time_s);
    sheet.cases_run = cases.iter().map(|c| c.id.clone()).collect();

    sheet.set_value("time_to_candidates_s", max_t);
    sheet.set_value_opt("runs_above_cap", count_in(run.exchanges.runs_above_cap, run.exchanges.cases_run as u64));
    sheet.set_value("model_calls_typical", run.exchanges.total_model_calls as f64 / run.exchanges.cases_run as f64);
    sheet.set_value_opt("search_recall_keyless", share(run.keyless_no_lib.found, run.keyless_no_lib.total));
    sheet.set_value_opt("search_recall_with_keys", share(run.keyed_no_lib.found, run.keyed_no_lib.total));
    sheet.set_value_opt("search_recall_without_library", share(run.keyed_no_lib.found, run.keyed_no_lib.total));
    sheet.set_value_opt("search_recall_with_library", share(run.keyed_with_lib.found, run.keyed_with_lib.total));
    sheet.set_value_opt("pdf_sentence_match_share", share(run.pdf.sentences_matched, run.pdf.sentences_total));
    sheet.set_value_opt("pdf_wrong_page_chip", count_in(run.pdf.wrong_page_chips, run.pdf.sentences_total));
    sheet.set_value_opt("pdf_scanned_reported_plain_text", count_in(run.pdf.scanned_reported_plain_text, run.pdf.scanned_pages_checked));
    sheet.set_value("pdf_two_col_wrong_order_lines", run.pdf.two_col_wrong_order_lines as f64);
    sheet.set_value_opt(
        "forbidden_word_property_failures",
        count_in(run.exchanges.s8_audit.failures, run.exchanges.s8_audit.checked),
    );
    sheet.measure_notes.insert("time_to_candidates_s".into(), "Max across all 40 loopback sessions (4 regimes x 10 cases).".into());
    sheet.measure_notes.insert("model_calls_typical".into(), "CP1 Sharpen + Triage only (21 calls / 10 cases, incl. 1 Rule C3 repair on I-05); X9-X13 pending CP2.".into());
    sheet.measure_notes.insert("search_recall_without_library".into(), "Keyed without library = 8/10 (0.8); keyless without library = 6/10 (0.6).".into());
    sheet.measure_notes.insert("search_recall_with_library".into(), "Keyed with F10/F13a library = 10/10 (1.0); keyless with library = 8/10 (0.8).".into());
    sheet.measure_notes.insert("pdf_scanned_reported_plain_text".into(), format!("Checked {} scanned/image-only pages across i_pdf_02_scanned_ocr_1987.pdf.", run.pdf.scanned_pages_checked));
    sheet.measure_notes.insert("forbidden_word_property_failures".into(), format!("Checked {} app-authored view, run_bar, empty_state, and F12 wording strings.", run.exchanges.s8_audit.checked));
    sheet.remarks = "X14 Checkpoint 1 fixed-tier Explore evaluation (search-recall, X6-X8 scripted, F14a PDF readiness).".into();

    assert_eq!(validate_sheet(&sheet), Vec::<String>::new());
    let report = evaluate(&sheet.values).expect("F8 gate evaluate");
    assert_eq!(report.verdict, GateVerdict::Incomplete);
    for id in [
        "time_to_candidates_s",
        "runs_above_cap",
        "pdf_wrong_page_chip",
        "pdf_scanned_reported_plain_text",
        "forbidden_word_property_failures",
    ] {
        assert_eq!(report.outcome_of(id), Some(Outcome::Pass), "measure `{id}` must Pass");
    }
    for id in [
        "search_recall_keyless",
        "search_recall_with_keys",
        "search_recall_without_library",
        "search_recall_with_library",
        "pdf_sentence_match_share",
        "model_calls_typical",
    ] {
        assert_eq!(report.outcome_of(id), Some(Outcome::RecordedOnly), "measure `{id}` must be RecordedOnly");
    }
    let md = render_markdown(&sheet).expect("render F8 sheet markdown");
    assert!(md.contains("search_recall_keyless"));
    assert!(md.contains("| `time_to_candidates_s` | All modes | below 6 |"));
    println!("=== F8 CP1 SHEET ===\n{md}");
}

fn run_explore_pdf_suite() -> PdfEvalMetrics {
    let mut m = PdfEvalMetrics::default();
    evaluate_pdf_fixture(
        &set_i_dir().join("user_library/moretti2004_phonon_drag.pdf"),
        &[PageBasis::TextLayer, PageBasis::TextLayer],
        &[
            PdfSentenceExpectation { quote: "acoustic phonon drag corrections to thermal Hall conductivity", expected_page: 1, expected_basis: PageBasis::TextLayer },
            PdfSentenceExpectation { quote: "Theorem 2.1 (Clean Phonon Drag Correction).", expected_page: 2, expected_basis: PageBasis::TextLayer },
        ],
        &[],
        &mut m,
    );
    evaluate_pdf_fixture(
        &set_i_dir().join("pdfs/i_pdf_01_two_column_journal.pdf"),
        &[PageBasis::TextLayer, PageBasis::TextLayer],
        &[
            PdfSentenceExpectation { quote: "Left-column sentence alpha defines the", expected_page: 1, expected_basis: PageBasis::TextLayer },
            PdfSentenceExpectation { quote: "Proposition 2.1 (Edge Flux).", expected_page: 1, expected_basis: PageBasis::TextLayer },
            PdfSentenceExpectation { quote: "Theorem 3.1 (Two-Column Decay).", expected_page: 2, expected_basis: PageBasis::TextLayer },
        ],
        &[
            ("Left-column sentence alpha", "Proposition 2.1 (Edge Flux)"),
            ("Theorem 3.1 (Two-Column Decay)", "Right-column sentence gamma"),
        ],
        &mut m,
    );
    evaluate_pdf_fixture(
        &set_i_dir().join("pdfs/i_pdf_02_scanned_ocr_1987.pdf"),
        &[PageBasis::ScanTextLayer, PageBasis::ScanTextLayer, PageBasis::None],
        &[
            PdfSentenceExpectation { quote: "flexural curvature tensor", expected_page: 1, expected_basis: PageBasis::ScanTextLayer },
            PdfSentenceExpectation { quote: "Lemma 2.1 (Scanned Strain Bound).", expected_page: 2, expected_basis: PageBasis::ScanTextLayer },
        ],
        &[],
        &mut m,
    );
    m
}
