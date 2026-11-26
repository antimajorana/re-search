//! PDF text core: page-broken reading files (`\x0c`), page map (`page_of`), scan detection,
//! pluggable [`PdfExtractor`] trait, and background extraction jobs (package F14a, 00-overview.md 2.12).

use crate::pdf::{
    PdfError, PdfExtractResult, PdfExtractStatus, MAX_PDF_BYTES, MAX_PDF_EXTRACTED_BYTES,
    MAX_PDF_PAGES,
};
use crate::pdf_pages_layout::LayoutOutputDev;
use crate::pdf_pages_scan::{classify_page_basis, inspect_page_signals};
use serde::{Deserialize, Serialize};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub use crate::pdf_pages_bench::{
    measure_extractor, ExpectedSentence, ExtractorBenchmarkReport, ExtractorCaseMetrics,
    ExtractorFixture, OrderedSequence,
};
pub use crate::pdf_pages_job::{PdfExtractJob, PdfExtractJobSnapshot, PdfExtractStage};

/// Form-feed page separator in extracted PDF reading files (`00-overview.md` section 2.12).
pub const PAGE_BREAK_CHAR: char = '\x0c';

/// Extractor id for `pdf-extract`'s stock `PlainTextOutput` (pinned in `source.ingested`).
pub const EXTRACTOR_PDF_EXTRACT_PLAIN: &str = "pdf-extract/0.12.1";

/// Extractor id for the column-aware `OutputDev` over `pdf-extract` (pinned in `source.ingested`).
pub const EXTRACTOR_PDF_EXTRACT_LAYOUT: &str = "pdf-extract-layout/0.12.1";

/// Default extractor id used by new PDF ingestions (chosen by fixture measurement in F14a;
/// `00-overview.md` section 2.12).
pub const DEFAULT_PDF_EXTRACTOR_ID: &str = EXTRACTOR_PDF_EXTRACT_LAYOUT;

/// Raised input byte cap for user-supplied PDFs (such as scanned old papers; `00-overview.md` 2.12).
/// Tunable once live P6 scan sizes are measured.
pub const MAX_USER_PDF_BYTES: usize = 64 * 1024 * 1024;

/// Per-page text quality classification (`00-overview.md` section 2.12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageBasis {
    /// Clean embedded digital text layer.
    TextLayer,
    /// Full-page scan image with a publisher/OCR text layer behind it (or uncertain scan).
    ScanTextLayer,
    /// No usable text layer (blank, image-only, or too little text for its page size).
    None,
}

impl PageBasis {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TextLayer => "text_layer",
            Self::ScanTextLayer => "scan_text_layer",
            Self::None => "none",
        }
    }

    /// True whenever the page is scanned or lacks a trustworthy digital text layer.
    pub fn is_scan(self) -> bool {
        !matches!(self, Self::TextLayer)
    }

    /// True when the page has no usable text and needs OCR (F14b).
    pub fn needs_ocr(self) -> bool {
        matches!(self, Self::None)
    }
}

/// Quantitative signals recorded per page to explain its [`PageBasis`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageQualityMetrics {
    pub char_count: usize,
    pub alpha_count: usize,
    pub word_count: usize,
    pub image_coverage_pct: u8,
    pub has_invisible_text: bool,
    pub width_pt: u32,
    pub height_pt: u32,
}

/// Quality and line-range record for one physical PDF page (1-based page and line numbers).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageRecord {
    pub page: usize,
    pub basis: PageBasis,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_line: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_line: Option<usize>,
    pub metrics: PageQualityMetrics,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Map between 1-based lines in the reading file and 1-based physical PDF pages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageMap {
    pub page_count: usize,
    pub pages: Vec<PageRecord>,
}

impl PageMap {
    /// Look up the 1-based physical page containing 1-based `line` in the reading file.
    /// Returns `None` for out-of-range lines or inter-page `\x0c` separator lines.
    pub fn page_of(&self, line: usize) -> Option<usize> {
        self.record_of_line(line).map(|r| r.page)
    }

    /// Look up the full [`PageRecord`] containing 1-based `line`.
    pub fn record_of_line(&self, line: usize) -> Option<&PageRecord> {
        if line == 0 {
            return None;
        }
        self.pages.iter().find(|r| match (r.first_line, r.last_line) {
            (Some(first), Some(last)) => (first..=last).contains(&line),
            _ => false,
        })
    }

    /// Look up a [`PageRecord`] by 1-based physical page number.
    pub fn record(&self, page: usize) -> Option<&PageRecord> {
        self.pages.iter().find(|r| r.page == page)
    }

    /// Compact `(page, first_line, last_line)` tuples matching `source.ingested.pdf_page_lines`.
    pub fn page_lines(&self) -> Vec<(usize, usize, usize)> {
        self.pages
            .iter()
            .filter_map(|r| Some((r.page, r.first_line?, r.last_line?)))
            .collect()
    }

    /// True if any extracted page was classified as `scan_text_layer` or `none`.
    pub fn has_scanned_pages(&self) -> bool {
        self.pages.iter().any(|r| r.basis.is_scan())
    }

    /// 1-based physical page numbers that have no usable text layer (need F14b OCR).
    pub fn pages_needing_ocr(&self) -> Vec<usize> {
        self.pages
            .iter()
            .filter(|r| r.basis.needs_ocr())
            .map(|r| r.page)
            .collect()
    }
}

/// Lookup helper over a compact `(page, first_line, last_line)` table (such as `pdf_page_lines`).
pub fn page_of_line(page_lines: &[(usize, usize, usize)], line: usize) -> Option<usize> {
    if line == 0 {
        return None;
    }
    page_lines
        .iter()
        .find(|&&(_, first, last)| (first..=last).contains(&line))
        .map(|&(page, _, _)| page)
}

/// Bounds and page-range options for PDF extraction.
#[derive(Debug, Clone)]
pub struct PdfExtractOptions {
    pub max_bytes: usize,
    pub max_pages: usize,
    pub max_extracted_bytes: usize,
    /// Optional 1-based inclusive `(start_page, end_page)` for files exceeding `max_pages`.
    pub page_range: Option<(usize, usize)>,
    pub(crate) cancel: Option<Arc<AtomicBool>>,
}

impl Default for PdfExtractOptions {
    fn default() -> Self {
        Self {
            max_bytes: MAX_PDF_BYTES,
            max_pages: MAX_PDF_PAGES,
            max_extracted_bytes: MAX_PDF_EXTRACTED_BYTES,
            page_range: None,
            cancel: None,
        }
    }
}

impl PdfExtractOptions {
    /// Options for user-supplied local PDFs (higher byte cap for scans; `00-overview.md` 2.12).
    pub fn user_supplied() -> Self {
        Self {
            max_bytes: MAX_USER_PDF_BYTES,
            ..Self::default()
        }
    }

    /// Restrict extraction to 1-based inclusive pages `start_page..=end_page`.
    pub fn with_page_range(mut self, start_page: usize, end_page: usize) -> Self {
        self.page_range = Some((start_page, end_page));
        self
    }
}

/// Per-page progress event delivered during extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageProgress {
    pub page: usize,
    pub total_pages: usize,
    pub basis: PageBasis,
    pub chars: usize,
}

/// Full extraction result from a [`PdfExtractor`], pinned in `source.ingested`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PdfDocumentPages {
    pub extractor_id: String,
    pub status: PdfExtractStatus,
    pub page_count: usize,
    pub page_map: PageMap,
    pub text: String,
    pub warning: Option<String>,
}

impl PdfDocumentPages {
    /// Convert to the legacy [`PdfExtractResult`] shape used by existing callers.
    pub fn to_extract_result(&self) -> PdfExtractResult {
        PdfExtractResult {
            status: self.status,
            page_count: self.page_count,
            page_lines: self.page_map.page_lines(),
            text: self.text.clone(),
            warning: self.warning.clone(),
        }
    }
}

/// Pluggable PDF extractor interface (`00-overview.md` section 2.12).
///
/// Any future second extractor (e.g. `pdf_oxide` or `pdftotext`) implements this trait so it can
/// be measured on the exact same fixture suite via [`measure_extractor`].
pub trait PdfExtractor {
    /// Stable extractor identifier pinned in `source.ingested`.
    fn id(&self) -> &'static str;

    /// Extract page-broken text (`\x0c` between pages) and per-page quality records.
    fn extract_with_progress(
        &self,
        bytes: &[u8],
        opts: &PdfExtractOptions,
        on_page: &mut dyn FnMut(PageProgress),
    ) -> Result<PdfDocumentPages, PdfError>;

    /// Convenience synchronous extraction without progress notifications.
    fn extract(
        &self,
        bytes: &[u8],
        opts: &PdfExtractOptions,
    ) -> Result<PdfDocumentPages, PdfError> {
        let mut noop = |_| {};
        self.extract_with_progress(bytes, opts, &mut noop)
    }
}

/// Stock `pdf-extract` plain-text extractor (`PlainTextOutput`).
#[derive(Debug, Clone, Copy, Default)]
pub struct PdfExtractPlain;

impl PdfExtractor for PdfExtractPlain {
    fn id(&self) -> &'static str {
        EXTRACTOR_PDF_EXTRACT_PLAIN
    }

    fn extract_with_progress(
        &self,
        bytes: &[u8],
        opts: &PdfExtractOptions,
        on_page: &mut dyn FnMut(PageProgress),
    ) -> Result<PdfDocumentPages, PdfError> {
        run_pdf_extract(bytes, opts, self.id(), false, on_page)
    }
}

/// Column-aware layout extractor over `pdf-extract`'s `OutputDev`.
#[derive(Debug, Clone, Copy, Default)]
pub struct PdfExtractLayout;

impl PdfExtractor for PdfExtractLayout {
    fn id(&self) -> &'static str {
        EXTRACTOR_PDF_EXTRACT_LAYOUT
    }

    fn extract_with_progress(
        &self,
        bytes: &[u8],
        opts: &PdfExtractOptions,
        on_page: &mut dyn FnMut(PageProgress),
    ) -> Result<PdfDocumentPages, PdfError> {
        run_pdf_extract(bytes, opts, self.id(), true, on_page)
    }
}

/// Return the default extractor selected for F14a.
pub fn default_extractor() -> PdfExtractLayout {
    PdfExtractLayout
}

/// Extract a PDF using the default extractor and options.
pub fn extract_pdf_pages(
    bytes: &[u8],
    opts: &PdfExtractOptions,
) -> Result<PdfDocumentPages, PdfError> {
    default_extractor().extract(bytes, opts)
}

/// Extract a PDF using the default extractor with a per-page progress callback.
pub fn extract_pdf_pages_with_progress(
    bytes: &[u8],
    opts: &PdfExtractOptions,
    on_page: &mut dyn FnMut(PageProgress),
) -> Result<PdfDocumentPages, PdfError> {
    default_extractor().extract_with_progress(bytes, opts, on_page)
}

/// Extract with a specific pinned extractor id (used when verifying receipts that name an extractor).
pub fn extract_pdf_pages_by_id(
    extractor_id: &str,
    bytes: &[u8],
    opts: &PdfExtractOptions,
) -> Result<PdfDocumentPages, PdfError> {
    match extractor_id {
        EXTRACTOR_PDF_EXTRACT_PLAIN => PdfExtractPlain.extract(bytes, opts),
        EXTRACTOR_PDF_EXTRACT_LAYOUT => PdfExtractLayout.extract(bytes, opts),
        other => Err(PdfError::Malformed(format!("unknown PDF extractor id `{other}`"))),
    }
}

fn sanitize_page_text(raw: &str) -> String {
    let mut cleaned = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\0' => {}
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                cleaned.push('\n');
            }
            PAGE_BREAK_CHAR => cleaned.push('\n'),
            _ => cleaned.push(c),
        }
    }
    cleaned.trim().to_string()
}

fn run_pdf_extract(
    bytes: &[u8],
    opts: &PdfExtractOptions,
    extractor_id: &'static str,
    use_layout: bool,
    on_page: &mut dyn FnMut(PageProgress),
) -> Result<PdfDocumentPages, PdfError> {
    if bytes.is_empty() {
        return Err(PdfError::Malformed("empty PDF input".into()));
    }
    if bytes.len() > opts.max_bytes {
        return Err(PdfError::TooLarge {
            bytes: bytes.len(),
            max: opts.max_bytes,
        });
    }
    let header_window = &bytes[..bytes.len().min(1024)];
    if !header_window.windows(5).any(|w| w == b"%PDF-") {
        return Err(PdfError::Malformed("missing %PDF- header".into()));
    }

    catch_unwind(AssertUnwindSafe(|| {
        let mut doc = pdf_extract::Document::load_mem(bytes)
            .map_err(|e| PdfError::Malformed(format!("invalid PDF structure: {e}")))?;
        if doc.is_encrypted() && doc.decrypt("").is_err() {
            return Err(PdfError::Encrypted);
        }
        let pages = doc.get_pages();
        let total_pages = pages.len();
        if total_pages == 0 {
            return Err(PdfError::Malformed("PDF has 0 pages".into()));
        }

        let (start_page, end_page) = match opts.page_range {
            Some((s, e)) => {
                if s == 0 || e < s || e > total_pages {
                    return Err(PdfError::InvalidPageRange {
                        start: s,
                        end: e,
                        total_pages,
                    });
                }
                let span = e - s + 1;
                if span > opts.max_pages {
                    return Err(PdfError::TooManyPages {
                        pages: span,
                        max: opts.max_pages,
                    });
                }
                (s, e)
            }
            None => {
                if total_pages > opts.max_pages {
                    return Err(PdfError::TooManyPages {
                        pages: total_pages,
                        max: opts.max_pages,
                    });
                }
                (1, total_pages)
            }
        };

        let selected: Vec<(u32, pdf_extract::ObjectId)> = pages
            .into_iter()
            .filter(|&(num, _)| {
                let p = num as usize;
                (start_page..=end_page).contains(&p)
            })
            .collect();
        let target_count = selected.len();

        let mut extracted_pages = Vec::with_capacity(target_count);
        let mut total_out = 0usize;

        for (idx, (page_num, page_id)) in selected.into_iter().enumerate() {
            if opts
                .cancel
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::Relaxed))
            {
                return Err(PdfError::Cancelled);
            }
            let raw_text = if use_layout {
                let mut dev = LayoutOutputDev::new();
                pdf_extract::output_doc_page(&doc, &mut dev, page_num)
                    .map_err(|e| PdfError::Malformed(format!("page {page_num} decode error: {e}")))?;
                dev.into_text()
            } else {
                let mut buf = String::new();
                {
                    let mut dev = pdf_extract::PlainTextOutput::new(&mut buf);
                    pdf_extract::output_doc_page(&doc, &mut dev, page_num).map_err(|e| {
                        PdfError::Malformed(format!("page {page_num} decode error: {e}"))
                    })?;
                }
                buf
            };
            let clean = sanitize_page_text(&raw_text);
            total_out = total_out.saturating_add(clean.len());
            if total_out > opts.max_extracted_bytes {
                return Err(PdfError::OutputTooLarge);
            }
            let signals = inspect_page_signals(&doc, page_id);
            let (basis, metrics, reason) = classify_page_basis(&clean, &signals);
            on_page(PageProgress {
                page: idx + 1,
                total_pages: target_count,
                basis,
                chars: metrics.char_count,
            });
            extracted_pages.push((page_num as usize, clean, basis, metrics, reason));
        }

        Ok(assemble_reading_file(
            extractor_id,
            total_pages,
            extracted_pages,
        ))
    }))
    .unwrap_or(Err(PdfError::ParserPanic))
}

fn assemble_reading_file(
    extractor_id: &'static str,
    total_pages: usize,
    extracted_pages: Vec<(usize, String, PageBasis, crate::pdf_pages::PageQualityMetrics, Option<String>)>,
) -> PdfDocumentPages {
    let any_text = extracted_pages.iter().any(|(_, t, _, _, _)| !t.is_empty());
    if !any_text {
        let records = extracted_pages
            .into_iter()
            .map(|(page, _, basis, metrics, reason)| PageRecord {
                page,
                basis,
                first_line: None,
                last_line: None,
                metrics,
                reason,
            })
            .collect();
        return PdfDocumentPages {
            extractor_id: extractor_id.to_string(),
            status: PdfExtractStatus::EmptyTextLayer,
            page_count: total_pages,
            page_map: PageMap {
                page_count: total_pages,
                pages: records,
            },
            text: String::new(),
            warning: Some(
                "PDF contains no extractable text layer (likely scanned or image-only).".into(),
            ),
        };
    }

    let mut combined = String::new();
    let mut records = Vec::with_capacity(extracted_pages.len());
    let mut cur_line = 1usize;

    for (idx, (page, text, basis, metrics, reason)) in extracted_pages.into_iter().enumerate() {
        if idx > 0 {
            combined.push('\n');
            combined.push(PAGE_BREAK_CHAR);
            combined.push('\n');
            cur_line += 1;
        }
        let (first_line, last_line) = if text.is_empty() {
            cur_line += 1;
            (None, None)
        } else {
            let line_count = text.split('\n').count();
            let first = cur_line;
            let last = first + line_count - 1;
            combined.push_str(&text);
            cur_line = last + 1;
            (Some(first), Some(last))
        };
        records.push(PageRecord {
            page,
            basis,
            first_line,
            last_line,
            metrics,
            reason,
        });
    }

    let scan_count = records.iter().filter(|r| r.basis.is_scan()).count();
    let none_count = records.iter().filter(|r| r.basis.needs_ocr()).count();
    let warning = if none_count > 0 {
        Some(format!(
            "{none_count} of {} extracted page(s) have no usable text layer (OCR needed).",
            records.len()
        ))
    } else if scan_count > 0 {
        Some(format!(
            "{scan_count} of {} extracted page(s) appear to be scanned with an embedded text layer.",
            records.len()
        ))
    } else {
        None
    };

    PdfDocumentPages {
        extractor_id: extractor_id.to_string(),
        status: PdfExtractStatus::Ok,
        page_count: total_pages,
        page_map: PageMap {
            page_count: total_pages,
            pages: records,
        },
        text: combined,
        warning,
    }
}

#[cfg(test)]
#[path = "pdf_pages_tests.rs"]
mod tests;
