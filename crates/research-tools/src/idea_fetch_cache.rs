//! F10 `paper_text` PDF cache reuse, `PdfExtractJob` runner, and F13a-contained `user_library`
//! lookup helpers for [`crate::idea_fetch`] (package X9, Rules C8 & C9).

use crate::ingest::{ingest_bytes, ingest_pdf_pages, IngestRequest, IngestedSource};
use crate::library::{
    Library, LibraryIds, PageBasis as LibPageBasis, PaperFlags, PaperRecord, PaperTextRecord,
    SourceItemState, SourceRecord, ORIGIN_USER_LIBRARY,
};
use crate::paper_key::{PaperKey, PaperRef};
use crate::paper_sources::{
    format_from_path_and_role, read_contained_bytes, resolve_contained_abs, FILE_ROLE_NOTE,
    FILE_ROLE_PDF, FILE_ROLE_TEX, MAX_SOURCE_TEXT_BYTES,
};
use crate::pdf::PdfExtractStatus;
use crate::pdf_pages::{
    PageBasis, PageMap, PageQualityMetrics, PageRecord, PdfDocumentPages, PdfExtractJob,
    PdfExtractOptions, DEFAULT_PDF_EXTRACTOR_ID, MAX_USER_PDF_BYTES, PAGE_BREAK_CHAR,
};
use crate::sandbox::WorkspaceSandbox;
use ledger::library_db::LibraryDb;
use ledger::{sha256_hex, FulltextCache, Ledger};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const PDF_JOB_POLL_MS: u64 = 5;

/// Cached non-`user_library` paper bytes retrieved from F10/F3 fulltext cache.
#[derive(Debug, Clone)]
pub(crate) struct CachedLibraryPaper {
    pub version: Option<String>,
    pub format: String,
    pub raw_blob: String,
    pub bytes: Vec<u8>,
    pub title: Option<String>,
}

/// F13a-contained user-owned paper file read safely from a registered source root.
#[derive(Debug, Clone)]
pub(crate) struct UserLibraryFile {
    pub version: Option<String>,
    pub format: String,
    pub file_name: String,
    pub bytes: Vec<u8>,
    pub title: Option<String>,
}

pub(crate) fn is_pdf_bytes(file_name: &str, format: &str, raw_bytes: &[u8]) -> bool {
    format.eq_ignore_ascii_case("pdf")
        || file_name.to_ascii_lowercase().ends_with(".pdf")
        || raw_bytes.starts_with(b"%PDF-")
}

/// Look up cached full-text blob in F10/F3 fetch cache (checking canonical key, aliases, and version).
pub(crate) fn lookup_library_cache(
    data_dir: &Path,
    paper: &PaperRef,
) -> Result<Option<CachedLibraryPaper>, String> {
    let wanted_ver = paper.version.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let mut keys_to_try: Vec<PaperKey> = vec![paper.key.clone()];
    for alias in crate::library::LibraryIds::from_paper_ids(&paper.ids).canonical_aliases() {
        if let Ok(k) = PaperKey::parse(&alias) {
            if !keys_to_try.contains(&k) {
                keys_to_try.push(k);
            }
        }
    }

    if let Ok(lib) = Library::open(data_dir) {
        if let Ok(db) = LibraryDb::open(data_dir) {
            for k in &keys_to_try {
                if let Ok(Some(detail)) = lib.get_paper(k) {
                    let title =
                        (!detail.paper.title.trim().is_empty()).then(|| detail.paper.title.clone());
                    for ft in detail.fulltext.iter().filter(|f| f.origin != ORIGIN_USER_LIBRARY) {
                        if let Some(v) = wanted_ver {
                            if ft.version.as_deref() != Some(v) {
                                continue;
                            }
                        }
                        if let Some(hash) = ft.raw_blob.as_deref().filter(|h| !h.is_empty()) {
                            if let Ok(Some(bytes)) = db.read_verified_blob_any(hash) {
                                return Ok(Some(CachedLibraryPaper {
                                    version: ft.version.clone(),
                                    format: ft.format.clone(),
                                    raw_blob: hash.to_string(),
                                    bytes,
                                    title,
                                }));
                            }
                        }
                    }
                }
            }
        }
    }

    if let Ok(cache) = FulltextCache::open(data_dir) {
        for k in &keys_to_try {
            if let Ok(Some((entry, bytes))) = cache.get_bytes(k.as_str(), wanted_ver) {
                return Ok(Some(CachedLibraryPaper {
                    version: entry.version,
                    format: entry.format,
                    raw_blob: entry.raw_blob,
                    bytes,
                    title: None,
                }));
            }
        }
    }
    Ok(None)
}

/// Look up an F13a user-owned file for `key`, strictly enforcing registered source-root
/// containment, symlink rejection, skipping `Missing` items and `FILE_ROLE_NOTE`, and
/// preferring `FILE_ROLE_TEX` over `FILE_ROLE_PDF`.
pub(crate) fn lookup_user_library_file(
    data_dir: &Path,
    key: &PaperKey,
    version: Option<&str>,
) -> Result<Option<UserLibraryFile>, String> {
    let Ok(lib) = Library::open(data_dir) else { return Ok(None) };
    let Ok(Some(detail)) = lib.get_paper(key) else { return Ok(None) };
    let sources = lib.list_sources().unwrap_or_default();
    let allowed_roots = collect_allowed_roots(&sources);
    if allowed_roots.is_empty() {
        return Ok(None);
    }

    let wanted_ver = version.map(str::trim).filter(|s| !s.is_empty());
    let mut candidates: Vec<(String, String, Option<String>)> = Vec::new();

    // 1. Prefer active non-Missing source_items: TeX before PDF, excluding FILE_ROLE_NOTE.
    for want_role in [FILE_ROLE_TEX, FILE_ROLE_PDF] {
        for item in &detail.source_items {
            if item.state == SourceItemState::Missing {
                continue;
            }
            if let Some(v) = wanted_ver {
                if item.version.as_deref() != Some(v) {
                    continue;
                }
            }
            for f in &item.files {
                if f.role == FILE_ROLE_NOTE || f.role != want_role || f.path.trim().is_empty() {
                    continue;
                }
                candidates.push((f.path.clone(), f.role.clone(), item.version.clone()));
            }
        }
    }

    // 2. Fall back to `user_library` fulltext rows (TeX before PDF).
    for want_tex in [true, false] {
        for ft in detail.fulltext.iter().filter(|f| f.origin == ORIGIN_USER_LIBRARY) {
            if let Some(v) = wanted_ver {
                if ft.version.as_deref() != Some(v) {
                    continue;
                }
            }
            let Some(p) = ft.file_path.as_deref().filter(|s| !s.trim().is_empty()) else {
                continue;
            };
            let is_tex = !ft.format.eq_ignore_ascii_case("pdf")
                && !p.to_ascii_lowercase().ends_with(".pdf");
            if is_tex != want_tex {
                continue;
            }
            let role = if is_tex { FILE_ROLE_TEX } else { FILE_ROLE_PDF };
            candidates.push((p.to_string(), role.to_string(), ft.version.clone()));
        }
    }

    let title = (!detail.paper.title.trim().is_empty()).then(|| detail.paper.title.clone());
    for (raw_path, role, ver) in candidates {
        let cand_path = PathBuf::from(&raw_path);
        let Some(safe_root) = allowed_roots
            .iter()
            .find(|r| resolve_contained_abs(r, &cand_path).is_ok())
        else {
            continue;
        };
        let max_bytes = if role == FILE_ROLE_PDF {
            MAX_USER_PDF_BYTES
        } else {
            MAX_SOURCE_TEXT_BYTES
        };
        let Ok(bytes) = read_contained_bytes(safe_root, &cand_path, max_bytes) else {
            continue;
        };
        let format = format_from_path_and_role(&raw_path, &role);
        let file_name = cand_path
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("paper.{format}"));
        return Ok(Some(UserLibraryFile {
            version: ver,
            format,
            file_name,
            bytes,
            title,
        }));
    }
    Ok(None)
}

fn collect_allowed_roots(sources: &[SourceRecord]) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for src in sources {
        let p = PathBuf::from(&src.root);
        let base = if p.is_file() {
            p.parent().map(Path::to_path_buf)
        } else {
            Some(p)
        };
        if let Some(dir) = base {
            if let Ok(canon) = std::fs::canonicalize(&dir) {
                if !roots.contains(&canon) {
                    roots.push(canon);
                }
            }
        }
        if let Some(arr) = src.settings.get("shared_dirs").and_then(serde_json::Value::as_array) {
            for extra in arr.iter().filter_map(serde_json::Value::as_str) {
                if let Ok(canon) = std::fs::canonicalize(extra) {
                    if !roots.contains(&canon) {
                        roots.push(canon);
                    }
                }
            }
        }
    }
    roots
}

/// Ingest bytes into `session_id` via `ingest_pdf_pages` (for PDFs, reusing/caching F10
/// `paper_text` and running `PdfExtractJob` on cache miss) or `ingest_bytes` (for TeX/archives).
#[allow(clippy::too_many_arguments)]
pub(crate) fn ingest_with_pdf_cache(
    data_dir: &Path,
    sandbox: &WorkspaceSandbox,
    ledger: &Mutex<Ledger>,
    key: &PaperKey,
    version: Option<&str>,
    req: &IngestRequest<'_>,
    format: &str,
    user_supplied: bool,
    cancel: Option<&AtomicBool>,
    on_pdf_page: &mut dyn FnMut(usize, usize),
) -> Result<(IngestedSource, bool), String> {
    if !is_pdf_bytes(req.file_name, format, req.raw_bytes) {
        return Ok((ingest_bytes(sandbox, ledger, req)?, false));
    }
    let file_sha256 = sha256_hex(req.raw_bytes);
    if let Some(cached_doc) = lookup_cached_pdf_pages(data_dir, key, req.raw_bytes, &file_sha256)? {
        let ingested = ingest_pdf_pages(sandbox, ledger, req, cached_doc)?;
        return Ok((ingested, true));
    }

    let opts = if user_supplied {
        PdfExtractOptions::user_supplied()
    } else {
        PdfExtractOptions::default()
    };
    let job = PdfExtractJob::start(
        format!("idea-pdf-{}", &file_sha256[..12]),
        req.raw_bytes.to_vec(),
        opts,
    );
    let mut last_reported = (0usize, 0usize);
    while !job.is_finished() {
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            job.cancel();
        }
        let snap = job.snapshot();
        if snap.total_pages > 0 && (snap.pages_done, snap.total_pages) != last_reported {
            last_reported = (snap.pages_done, snap.total_pages);
            on_pdf_page(snap.pages_done, snap.total_pages);
        }
        thread::sleep(Duration::from_millis(PDF_JOB_POLL_MS));
    }
    let snap = job.snapshot();
    if snap.total_pages > 0 && (snap.pages_done, snap.total_pages) != last_reported {
        on_pdf_page(snap.pages_done, snap.total_pages);
    }
    let extracted = job.join().map_err(|e| format!("PDF extraction failed: {e}"))?;
    let _ = store_pdf_pages_in_library(data_dir, key, version, &file_sha256, &extracted);
    let ingested = ingest_pdf_pages(sandbox, ledger, req, extracted)?;
    Ok((ingested, false))
}

/// Look up cached PDF text in F10 `paper_text` matching `(file_sha256, extractor_id)` under
/// either the paper's canonical key or its `file:<sha16>` fallback key (F14b OCR caches).
pub fn lookup_cached_pdf_pages(
    data_dir: &Path,
    key: &PaperKey,
    raw_bytes: &[u8],
    file_sha256: &str,
) -> Result<Option<PdfDocumentPages>, String> {
    let Ok(lib) = Library::open(data_dir) else { return Ok(None) };
    let Ok(db) = LibraryDb::open(data_dir) else { return Ok(None) };
    let file_key = PaperKey::from_file_bytes(raw_bytes);
    let ocr_prefix = format!("{DEFAULT_PDF_EXTRACTOR_ID}+");

    let mut keys = vec![key.clone()];
    if file_key != *key {
        keys.push(file_key);
    }

    for candidate_key in &keys {
        let Ok(Some(detail)) = lib.get_paper(candidate_key) else { continue };
        let chosen = detail
            .paper_text
            .iter()
            .find(|pt| pt.file_sha256 == file_sha256 && pt.extractor.starts_with(&ocr_prefix))
            .or_else(|| {
                detail.paper_text.iter().find(|pt| {
                    pt.file_sha256 == file_sha256 && pt.extractor == DEFAULT_PDF_EXTRACTOR_ID
                })
            });
        let Some(rec) = chosen else { continue };
        let Ok(Some(bytes)) = db.read_verified_blob_any(&rec.text_blob) else { continue };
        let Ok(text) = String::from_utf8(bytes) else { continue };
        return Ok(Some(reconstruct_pdf_document_pages(
            &rec.extractor,
            text,
            &rec.page_basis,
        )));
    }
    Ok(None)
}

/// Reconstruct [`PdfDocumentPages`] from cached F10 `paper_text` text (`\n\x0c\n` separated)
/// and per-page [`LibPageBasis`], preserving scan/OCR warnings and exact page counts.
pub fn reconstruct_pdf_document_pages(
    extractor_id: &str,
    text: String,
    cached_bases: &[LibPageBasis],
) -> PdfDocumentPages {
    let sep = format!("\n{PAGE_BREAK_CHAR}\n");
    if text.is_empty() {
        let count = cached_bases.len().max(1);
        let pages = (0..count)
            .map(|i| PageRecord {
                page: i + 1,
                basis: map_lib_basis(cached_bases.get(i).copied().unwrap_or(LibPageBasis::None)),
                first_line: None,
                last_line: None,
                metrics: zero_metrics(),
                reason: None,
            })
            .collect();
        return PdfDocumentPages {
            extractor_id: extractor_id.to_string(),
            status: PdfExtractStatus::EmptyTextLayer,
            page_count: count,
            page_map: PageMap { page_count: count, pages },
            text,
            warning: Some(
                "PDF contains no extractable text layer (likely scanned or image-only).".into(),
            ),
        };
    }

    let chunks: Vec<&str> = text.split(&sep).collect();
    let page_count = chunks.len().max(cached_bases.len());
    let mut records = Vec::with_capacity(page_count);
    let mut cur_line = 1usize;
    for idx in 0..page_count {
        let chunk = chunks.get(idx).copied().unwrap_or("");
        if idx > 0 && idx < chunks.len() {
            cur_line += 1; // form-feed separator line
        }
        let basis = map_lib_basis(cached_bases.get(idx).copied().unwrap_or(LibPageBasis::TextLayer));
        let (first_line, last_line) = if chunk.is_empty() {
            if idx < chunks.len() {
                cur_line += 1;
            }
            (None, None)
        } else {
            let line_count = chunk.split('\n').count();
            let first = cur_line;
            let last = first + line_count - 1;
            cur_line = last + 1;
            (Some(first), Some(last))
        };
        records.push(PageRecord {
            page: idx + 1,
            basis,
            first_line,
            last_line,
            metrics: zero_metrics(),
            reason: None,
        });
    }

    let none_count = records.iter().filter(|r| r.basis.needs_ocr()).count();
    let ocr_count = records.iter().filter(|r| matches!(r.basis, PageBasis::Ocr)).count();
    let scan_count = records
        .iter()
        .filter(|r| matches!(r.basis, PageBasis::ScanTextLayer))
        .count();
    let warning = if none_count > 0 {
        Some(format!(
            "{none_count} of {page_count} extracted page(s) have no usable text layer (OCR needed)."
        ))
    } else if ocr_count > 0 {
        Some(format!(
            "{ocr_count} of {page_count} extracted page(s) use local OCR text."
        ))
    } else if scan_count > 0 {
        Some(format!(
            "{scan_count} of {page_count} extracted page(s) appear to be scanned with an embedded text layer."
        ))
    } else {
        None
    };

    PdfDocumentPages {
        extractor_id: extractor_id.to_string(),
        status: PdfExtractStatus::Ok,
        page_count,
        page_map: PageMap { page_count, pages: records },
        text,
        warning,
    }
}

fn store_pdf_pages_in_library(
    data_dir: &Path,
    key: &PaperKey,
    version: Option<&str>,
    file_sha256: &str,
    doc: &PdfDocumentPages,
) -> Result<(), String> {
    let lib = Library::open(data_dir)?;
    let now = now_ms();
    if lib.get_paper(key)?.is_none() {
        lib.upsert_paper(&PaperRecord {
            key: key.clone(),
            ids: LibraryIds::default(),
            title: String::new(),
            authors: Vec::new(),
            year: None,
            abstract_text: String::new(),
            bibtex: String::new(),
            flags: PaperFlags::default(),
            first_seen_ms: now,
            last_seen_ms: now,
        })?;
    }
    let canonical = lib.resolve_key(key)?.unwrap_or_else(|| key.clone());
    let page_basis = doc
        .page_map
        .pages
        .iter()
        .map(|r| match r.basis {
            PageBasis::TextLayer => LibPageBasis::TextLayer,
            PageBasis::ScanTextLayer => LibPageBasis::ScanTextLayer,
            PageBasis::Ocr => LibPageBasis::Ocr,
            PageBasis::None => LibPageBasis::None,
        })
        .collect();
    let rec = PaperTextRecord {
        key: canonical,
        version: version.map(str::to_string),
        file_sha256: file_sha256.to_string(),
        extractor: doc.extractor_id.clone(),
        text_blob: sha256_hex(doc.text.as_bytes()),
        page_basis,
        extracted_ms: now,
    };
    lib.upsert_paper_text(&rec, Some(doc.text.as_bytes()))
}

fn map_lib_basis(b: LibPageBasis) -> PageBasis {
    match b {
        LibPageBasis::TextLayer => PageBasis::TextLayer,
        LibPageBasis::ScanTextLayer => PageBasis::ScanTextLayer,
        LibPageBasis::Ocr => PageBasis::Ocr,
        LibPageBasis::None => PageBasis::None,
    }
}

fn zero_metrics() -> PageQualityMetrics {
    PageQualityMetrics {
        char_count: 0, alpha_count: 0, word_count: 0, image_coverage_pct: 0,
        has_invisible_text: false, width_pt: 0, height_pt: 0,
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
