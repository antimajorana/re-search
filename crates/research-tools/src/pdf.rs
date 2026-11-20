//! Bounded pure-Rust PDF text extraction (`pdf-extract` wrapped in `catch_unwind`).

use serde::{Deserialize, Serialize};
use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};

pub const MAX_PDF_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_PDF_PAGES: usize = 400;
pub const MAX_PDF_EXTRACTED_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PdfExtractStatus {
    Ok,
    EmptyTextLayer,
}

impl PdfExtractStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::EmptyTextLayer => "empty_text_layer",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PdfExtractResult {
    pub status: PdfExtractStatus,
    pub page_count: usize,
    pub text: String,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PdfError {
    TooLarge { bytes: usize, max: usize },
    TooManyPages { pages: usize, max: usize },
    Encrypted,
    Malformed(String),
    ParserPanic,
    OutputTooLarge,
}

impl fmt::Display for PdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { bytes, max } => write!(f, "PDF size ({bytes} bytes) exceeds limit ({max} bytes)"),
            Self::TooManyPages { pages, max } => write!(f, "PDF page count ({pages}) exceeds limit ({max} pages)"),
            Self::Encrypted => write!(f, "encrypted password-protected PDFs are not supported"),
            Self::Malformed(m) => write!(f, "malformed PDF: {m}"),
            Self::ParserPanic => write!(f, "PDF parser trapped an internal panic on malformed input"),
            Self::OutputTooLarge => write!(f, "extracted PDF text exceeds 8 MiB limit"),
        }
    }
}

impl std::error::Error for PdfError {}

/// Extracts plain text from PDF bytes with `<= 16 MiB` input cap, `<= 400` page cap checked
/// before decoding page streams, NUL-byte scrubbing, `EmptyTextLayer` detection for scanned PDFs,
/// and `catch_unwind` isolation against upstream `lopdf`/`pdf-extract` panics.
pub fn extract_pdf_bytes(bytes: &[u8]) -> Result<PdfExtractResult, PdfError> {
    if bytes.len() > MAX_PDF_BYTES {
        return Err(PdfError::TooLarge { bytes: bytes.len(), max: MAX_PDF_BYTES });
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
        if pages.is_empty() {
            return Err(PdfError::Malformed("PDF has 0 pages".into()));
        }
        if pages.len() > MAX_PDF_PAGES {
            return Err(PdfError::TooManyPages { pages: pages.len(), max: MAX_PDF_PAGES });
        }

        let mut page_texts = Vec::with_capacity(pages.len());
        let mut total_out = 0usize;
        for &page_num in pages.keys() {
            let mut buf = String::new();
            {
                let mut dev = pdf_extract::PlainTextOutput::new(&mut buf);
                pdf_extract::output_doc_page(&doc, &mut dev, page_num)
                    .map_err(|e| PdfError::Malformed(format!("page {page_num} decode error: {e}")))?;
            }
            buf.retain(|c| c != '\0');
            total_out = total_out.saturating_add(buf.len());
            if total_out > MAX_PDF_EXTRACTED_BYTES {
                return Err(PdfError::OutputTooLarge);
            }
            page_texts.push((page_num, buf));
        }

        if page_texts.iter().all(|(_, t)| t.trim().is_empty()) {
            return Ok(PdfExtractResult {
                status: PdfExtractStatus::EmptyTextLayer,
                page_count: pages.len(),
                text: String::new(),
                warning: Some(
                    "PDF contains no extractable text layer (likely scanned or image-only).".into(),
                ),
            });
        }

        let mut combined = String::new();
        for (page_num, text) in page_texts {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                if !combined.is_empty() {
                    combined.push_str("\n\n");
                }
                combined.push_str(&format!("--- [Page {page_num}] ---\n{trimmed}"));
            }
        }

        Ok(PdfExtractResult {
            status: PdfExtractStatus::Ok,
            page_count: pages.len(),
            text: combined,
            warning: None,
        })
    }))
    .unwrap_or(Err(PdfError::ParserPanic))
}
