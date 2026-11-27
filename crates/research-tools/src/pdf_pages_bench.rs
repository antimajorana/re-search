//! Reusable PDF extractor measurement & comparison harness (package F14a, 00-overview.md 2.12).
//!
//! Any [`PdfExtractor`] (today's `pdf-extract` plain/layout modes, or a future second extractor
//! such as `pdf_oxide` or `pdftotext`) can be evaluated against the same fixture set and compared
//! on time, failures, two-column reading order, hyphenation, ligatures, empty/scan pages, and
//! page-link accuracy.

use crate::pdf_match::match_pdf_quote;
use crate::pdf_pages::{PageBasis, PdfExtractOptions, PdfExtractor};
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// One expected sentence and its ground-truth 1-based physical page.
#[derive(Debug, Clone)]
pub struct ExpectedSentence {
    pub text: String,
    pub expected_page: usize,
    /// Optional category tag (`"plain"`, `"two_col"`, `"hyphenation"`, `"ligature"`, `"scan_text"`).
    pub category: String,
}

/// Ordered sequence of phrases that must appear in non-decreasing byte order (reading-order check).
#[derive(Debug, Clone)]
pub struct OrderedSequence {
    pub page: usize,
    pub phrases: Vec<String>,
}

/// One self-authored PDF fixture for extractor evaluation.
#[derive(Debug, Clone)]
pub struct ExtractorFixture {
    pub name: String,
    pub pdf_bytes: Vec<u8>,
    pub expected_page_count: usize,
    pub expected_bases: Vec<(usize, PageBasis)>,
    pub expected_sentences: Vec<ExpectedSentence>,
    pub reading_order_checks: Vec<OrderedSequence>,
}

/// Measurement results for one fixture under one [`PdfExtractor`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractorCaseMetrics {
    pub fixture: String,
    pub extractor_id: String,
    pub elapsed_us: u128,
    pub failed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub page_count: usize,
    pub sentences_total: usize,
    pub sentences_matched: usize,
    pub wrong_page_links: usize,
    pub scans_reported_as_text_layer: usize,
    pub empty_pages_correct: usize,
    pub empty_pages_total: usize,
    pub reading_order_pairs_total: usize,
    pub reading_order_pairs_in_order: usize,
    pub reading_order_errors: usize,
    pub hyphenation_matched: usize,
    pub hyphenation_total: usize,
    pub ligatures_matched: usize,
    pub ligatures_total: usize,
}

/// Aggregate comparison report for one [`PdfExtractor`] across a fixture suite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractorBenchmarkReport {
    pub extractor_id: String,
    pub cases: Vec<ExtractorCaseMetrics>,
    pub total_elapsed_us: u128,
    pub failures: usize,
    pub total_sentences_matched: usize,
    pub total_sentences: usize,
    pub total_wrong_page_links: usize,
    pub total_scans_reported_as_text_layer: usize,
    pub total_reading_order_errors: usize,
}

/// Measure `extractor` against every fixture in `fixtures` using `opts`.
pub fn measure_extractor<E: PdfExtractor>(
    extractor: &E,
    fixtures: &[ExtractorFixture],
    opts: &PdfExtractOptions,
) -> ExtractorBenchmarkReport {
    let mut cases = Vec::with_capacity(fixtures.len());
    let mut total_elapsed_us = 0u128;
    let mut failures = 0usize;
    let mut total_sentences_matched = 0usize;
    let mut total_sentences = 0usize;
    let mut total_wrong_page_links = 0usize;
    let mut total_scans_reported_as_text_layer = 0usize;
    let mut total_reading_order_errors = 0usize;

    for fx in fixtures {
        let start = Instant::now();
        let res = extractor.extract(&fx.pdf_bytes, opts);
        let elapsed_us = start.elapsed().as_micros();
        total_elapsed_us += elapsed_us;

        let metrics = match res {
            Err(err) => {
                failures += 1;
                ExtractorCaseMetrics {
                    fixture: fx.name.clone(),
                    extractor_id: extractor.id().to_string(),
                    elapsed_us,
                    failed: true,
                    error: Some(err.to_string()),
                    page_count: 0,
                    sentences_total: fx.expected_sentences.len(),
                    sentences_matched: 0,
                    wrong_page_links: 0,
                    scans_reported_as_text_layer: 0,
                    empty_pages_correct: 0,
                    empty_pages_total: fx
                        .expected_bases
                        .iter()
                        .filter(|(_, b)| *b == PageBasis::None)
                        .count(),
                    reading_order_pairs_total: fx
                        .reading_order_checks
                        .iter()
                        .map(|s| s.phrases.len().saturating_sub(1))
                        .sum(),
                    reading_order_pairs_in_order: 0,
                    reading_order_errors: fx
                        .reading_order_checks
                        .iter()
                        .map(|s| s.phrases.len().saturating_sub(1))
                        .sum(),
                    hyphenation_matched: 0,
                    hyphenation_total: fx
                        .expected_sentences
                        .iter()
                        .filter(|s| s.category == "hyphenation")
                        .count(),
                    ligatures_matched: 0,
                    ligatures_total: fx
                        .expected_sentences
                        .iter()
                        .filter(|s| s.category == "ligature")
                        .count(),
                }
            }
            Ok(doc) => {
                let mut sentences_matched = 0usize;
                let mut wrong_page_links = 0usize;
                let mut hyphenation_total = 0usize;
                let mut hyphenation_matched = 0usize;
                let mut ligatures_total = 0usize;
                let mut ligatures_matched = 0usize;

                for sent in &fx.expected_sentences {
                    if sent.category == "hyphenation" {
                        hyphenation_total += 1;
                    } else if sent.category == "ligature" {
                        ligatures_total += 1;
                    }
                    if let Some(m) = match_pdf_quote(&doc.text, &doc.page_map, &sent.text) {
                        sentences_matched += 1;
                        if sent.category == "hyphenation" {
                            hyphenation_matched += 1;
                        } else if sent.category == "ligature" {
                            ligatures_matched += 1;
                        }
                        if m.start_page != Some(sent.expected_page) {
                            wrong_page_links += 1;
                        }
                    }
                }

                let mut scans_reported_as_text_layer = 0usize;
                let mut empty_pages_total = 0usize;
                let mut empty_pages_correct = 0usize;
                for &(page_num, expected_basis) in &fx.expected_bases {
                    let actual = doc.page_map.record(page_num).map(|r| r.basis);
                    if expected_basis == PageBasis::None {
                        empty_pages_total += 1;
                        if actual == Some(PageBasis::None) {
                            empty_pages_correct += 1;
                        }
                    }
                    if expected_basis.is_scan() && actual == Some(PageBasis::TextLayer) {
                        scans_reported_as_text_layer += 1;
                    }
                }

                let mut pairs_total = 0usize;
                let mut pairs_in_order = 0usize;
                let mut order_errors = 0usize;
                for seq in &fx.reading_order_checks {
                    let positions: Vec<Option<usize>> = seq
                        .phrases
                        .iter()
                        .map(|phrase| {
                            match_pdf_quote(&doc.text, &doc.page_map, phrase)
                                .filter(|m| m.start_page == Some(seq.page))
                                .map(|m| m.start_byte)
                        })
                        .collect();
                    for win in positions.windows(2) {
                        pairs_total += 1;
                        match (win[0], win[1]) {
                            (Some(a), Some(b)) if a < b => pairs_in_order += 1,
                            _ => order_errors += 1,
                        }
                    }
                }

                ExtractorCaseMetrics {
                    fixture: fx.name.clone(),
                    extractor_id: doc.extractor_id,
                    elapsed_us,
                    failed: false,
                    error: None,
                    page_count: doc.page_count,
                    sentences_total: fx.expected_sentences.len(),
                    sentences_matched,
                    wrong_page_links,
                    scans_reported_as_text_layer,
                    empty_pages_correct,
                    empty_pages_total,
                    reading_order_pairs_total: pairs_total,
                    reading_order_pairs_in_order: pairs_in_order,
                    reading_order_errors: order_errors,
                    hyphenation_matched,
                    hyphenation_total,
                    ligatures_matched,
                    ligatures_total,
                }
            }
        };

        total_sentences_matched += metrics.sentences_matched;
        total_sentences += metrics.sentences_total;
        total_wrong_page_links += metrics.wrong_page_links;
        total_scans_reported_as_text_layer += metrics.scans_reported_as_text_layer;
        total_reading_order_errors += metrics.reading_order_errors;
        cases.push(metrics);
    }

    ExtractorBenchmarkReport {
        extractor_id: extractor.id().to_string(),
        cases,
        total_elapsed_us,
        failures,
        total_sentences_matched,
        total_sentences,
        total_wrong_page_links,
        total_scans_reported_as_text_layer,
        total_reading_order_errors,
    }
}
