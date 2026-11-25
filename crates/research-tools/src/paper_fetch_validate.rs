//! Download validation helpers for [`crate::paper_fetch`].
//!
//! Refuses empty, oversized, HTML error pages, corrupt archives, symlink/bomb archives,
//! and malformed/encrypted/oversized PDFs without panicking.

use crate::archive::{extract_gzip_or_tar_gz, ArchivePayload};
use crate::http_client::MAX_EPRINT_BYTES;
use crate::paper_fetch::SourceOrigin;
use crate::pdf::{MAX_PDF_BYTES, MAX_PDF_PAGES};
use std::panic::{catch_unwind, AssertUnwindSafe};

const MAX_PLAIN_TEX_BYTES: usize = 16 * 1024 * 1024;

pub(crate) struct ValidatedDownload {
    pub(crate) origin: SourceOrigin,
    pub(crate) format: String,
    pub(crate) file_name: String,
    pub(crate) origin_url: String,
    pub(crate) raw_bytes: Vec<u8>,
}

pub(crate) fn safe_id_slug(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub(crate) fn looks_like_html(content_type: Option<&str>, bytes: &[u8]) -> bool {
    if content_type.is_some_and(|ct| ct.to_ascii_lowercase().contains("text/html")) {
        return true;
    }
    let trimmed = bytes
        .iter()
        .skip_while(|b| b.is_ascii_whitespace())
        .take(32)
        .copied()
        .collect::<Vec<_>>();
    let head = String::from_utf8_lossy(&trimmed).to_ascii_lowercase();
    head.starts_with("<!doctype html") || head.starts_with("<html")
}

pub(crate) fn validate_eprint_download(
    target_id: &str,
    final_url: &str,
    content_type: Option<&str>,
    bytes: Vec<u8>,
) -> Result<ValidatedDownload, String> {
    if bytes.is_empty() {
        return Err("malformed arXiv e-print download: empty response body".into());
    }
    if bytes.len() > MAX_EPRINT_BYTES {
        return Err(format!(
            "arXiv e-print download exceeds {MAX_EPRINT_BYTES} byte limit"
        ));
    }
    if looks_like_html(content_type, &bytes) {
        return Err(
            "malformed arXiv e-print download: received HTML instead of paper source".into(),
        );
    }
    let slug = safe_id_slug(target_id);
    if bytes[..bytes.len().min(1024)]
        .windows(5)
        .any(|w| w == b"%PDF-")
    {
        validate_pdf_download(None, &bytes)?;
        return Ok(ValidatedDownload {
            origin: SourceOrigin::ArxivPdf,
            format: "pdf".into(),
            file_name: format!("arxiv_{slug}.pdf"),
            origin_url: final_url.to_string(),
            raw_bytes: bytes,
        });
    }
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let hint = format!("arxiv_{slug}.gz");
        let payload = extract_gzip_or_tar_gz(&bytes, &hint, false)
            .map_err(|e| format!("malformed arXiv e-print archive: {e}"))?;
        return match payload {
            ArchivePayload::Files(entries) => {
                if entries.is_empty() {
                    return Err("malformed arXiv e-print archive: contains 0 files".into());
                }
                Ok(ValidatedDownload {
                    origin: SourceOrigin::ArxivEprint,
                    format: "tar.gz".into(),
                    file_name: hint,
                    origin_url: final_url.to_string(),
                    raw_bytes: bytes,
                })
            }
            ArchivePayload::SingleGzipText { .. } => Ok(ValidatedDownload {
                origin: SourceOrigin::ArxivEprint,
                format: "gz".into(),
                file_name: hint,
                origin_url: final_url.to_string(),
                raw_bytes: bytes,
            }),
            ArchivePayload::RawPdf(pdf_bytes) => {
                validate_pdf_download(None, &pdf_bytes)?;
                Ok(ValidatedDownload {
                    origin: SourceOrigin::ArxivPdf,
                    format: "pdf".into(),
                    file_name: format!("arxiv_{slug}.pdf"),
                    origin_url: final_url.to_string(),
                    raw_bytes: pdf_bytes,
                })
            }
        };
    }
    if bytes.len() <= MAX_PLAIN_TEX_BYTES && !bytes.contains(&0u8) {
        if let Ok(text) = std::str::from_utf8(&bytes) {
            let looks_tex = text.contains("\\documentclass")
                || text.contains("\\begin{document}")
                || text.contains("\\section")
                || text.contains("\\documentstyle");
            if looks_tex {
                return Ok(ValidatedDownload {
                    origin: SourceOrigin::ArxivEprint,
                    format: "tex".into(),
                    file_name: format!("arxiv_{slug}.tex"),
                    origin_url: final_url.to_string(),
                    raw_bytes: bytes,
                });
            }
        }
    }
    Err("malformed arXiv e-print download: unrecognized or corrupt payload".into())
}

pub(crate) fn validate_pdf_download(content_type: Option<&str>, bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() {
        return Err("malformed PDF download: empty response body".into());
    }
    if bytes.len() > MAX_PDF_BYTES {
        return Err(format!(
            "PDF download ({} bytes) exceeds {MAX_PDF_BYTES} byte limit",
            bytes.len()
        ));
    }
    if looks_like_html(content_type, bytes) {
        return Err("malformed PDF download: received HTML instead of PDF".into());
    }
    let header_window = &bytes[..bytes.len().min(1024)];
    if !header_window.windows(5).any(|w| w == b"%PDF-") {
        return Err("malformed PDF download: missing %PDF- header".into());
    }
    catch_unwind(AssertUnwindSafe(|| {
        let mut doc = pdf_extract::Document::load_mem(bytes)
            .map_err(|e| format!("malformed PDF structure: {e}"))?;
        if doc.is_encrypted() && doc.decrypt("").is_err() {
            return Err("encrypted password-protected PDFs are not supported".into());
        }
        let pages = doc.get_pages();
        if pages.is_empty() {
            return Err("malformed PDF download: 0 pages".into());
        }
        if pages.len() > MAX_PDF_PAGES {
            return Err(format!(
                "PDF page count ({}) exceeds {MAX_PDF_PAGES} page limit",
                pages.len()
            ));
        }
        Ok(())
    }))
    .unwrap_or_else(|_| Err("malformed PDF download: parser trapped internal panic".into()))
}
