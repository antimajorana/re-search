//! Local Tesseract OCR pipeline for image-only (`PageBasis::None`) PDF pages,
//! runtime provenance tracking, and F10 `Library` `paper_text` caching (package F14b).

use crate::library::Library;
use crate::library_schema::{
    LibraryIds, PageBasis as LibPageBasis, PaperFlags, PaperRecord, PaperTextRecord,
};
use crate::paper_key::PaperKey;
use crate::pdf::{PdfError, PdfExtractStatus, MAX_PDF_PAGES};
use crate::pdf_ocr_models::{
    effective_langs, format_ocr_engine_id, hash_local_traineddata, ocr_status,
    ocr_status_with_probe, OcrEnvProbe, OcrLocalConfig, OcrModelProvenance, OcrStatus,
    OCR_INSTALL_PROMPT,
};
use crate::pdf_pages::{
    extract_pdf_pages, PageBasis, PageMap, PageQualityMetrics, PageRecord, PdfDocumentPages,
    PdfExtractOptions, PAGE_BREAK_CHAR,
};
use crate::pdf_render::{PdfPageRenderer, DEFAULT_RENDER_DPI, MAX_RENDER_DPI, MIN_RENDER_DPI};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const DEFAULT_OCR_PAGE_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_OCR_MAX_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
const OCR_POLL_INTERVAL: Duration = Duration::from_millis(15);
const MAX_STDERR_SNIPPET_BYTES: u64 = 2048;
static OCR_TEMP_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

/// Pluggable per-page OCR engine over rendered PNG bytes.
pub trait OcrPageRunner: Send + Sync {
    fn engine_id(&self) -> &str;
    fn model_provenance(&self) -> Vec<OcrModelProvenance>;
    fn ocr_png(&self, page_1based: usize, png_bytes: &[u8], dpi: u16, langs: &[String]) -> Result<String, String>;
}

/// Deterministic in-memory OCR runner for unit and integration tests.
#[derive(Debug, Clone)]
pub struct FakeOcrPageRunner {
    pub pages: BTreeMap<usize, Result<String, String>>,
    pub engine_id: String,
    pub provenance: Vec<OcrModelProvenance>,
    pub calls: Arc<Mutex<Vec<usize>>>,
}

impl FakeOcrPageRunner {
    pub fn new(pages: BTreeMap<usize, Result<String, String>>) -> Self {
        Self { pages, engine_id: "tesseract/fake-5.3.4".into(), provenance: Vec::new(), calls: Arc::new(Mutex::new(Vec::new())) }
    }
    pub fn with_engine_id(mut self, engine_id: impl Into<String>) -> Self { self.engine_id = engine_id.into(); self }
    pub fn with_provenance(mut self, provenance: Vec<OcrModelProvenance>) -> Self { self.provenance = provenance; self }
    pub fn calls(&self) -> Vec<usize> { self.calls.lock().unwrap_or_else(|e| e.into_inner()).clone() }
    pub fn calls_handle(&self) -> Arc<Mutex<Vec<usize>>> { self.calls.clone() }
}

impl OcrPageRunner for FakeOcrPageRunner {
    fn engine_id(&self) -> &str { &self.engine_id }
    fn model_provenance(&self) -> Vec<OcrModelProvenance> { self.provenance.clone() }
    fn ocr_png(&self, page_1based: usize, _png_bytes: &[u8], _dpi: u16, _langs: &[String]) -> Result<String, String> {
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).push(page_1based);
        self.pages.get(&page_1based).cloned().unwrap_or_else(|| Err(format!("no fake OCR result configured for page {page_1based}")))
    }
}

struct OcrTempDirGuard(PathBuf);
impl OcrTempDirGuard {
    fn create() -> Result<Self, String> {
        let (base, pid) = (std::env::temp_dir(), std::process::id());
        for _ in 0..32 {
            let dir = base.join(format!("research-pdf-ocr-{pid}-{}", OCR_TEMP_DIR_SEQ.fetch_add(1, Ordering::Relaxed)));
            if fs::create_dir(&dir).is_ok() { return Ok(Self(dir)); }
        }
        Err("failed to create temporary directory for local OCR".into())
    }
}
impl Drop for OcrTempDirGuard {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

/// Local Tesseract subprocess runner with minimal environment, RAII temp dir,
/// per-page timeout, and bounded output reads.
#[derive(Debug, Clone)]
pub struct LocalTesseractPageRunner {
    tesseract_exe: PathBuf,
    tessdata_dir: PathBuf,
    engine_id: String,
    provenance: Vec<OcrModelProvenance>,
    page_timeout: Duration,
    max_output_bytes: usize,
}

impl LocalTesseractPageRunner {
    pub fn from_config(config: &OcrLocalConfig) -> Result<Self, String> {
        Self::from_status(ocr_status(config), &config.langs)
    }

    pub fn from_config_with_probe(config: &OcrLocalConfig, probe: &OcrEnvProbe) -> Result<Self, String> {
        Self::from_status(ocr_status_with_probe(config, probe), &config.langs)
    }

    fn from_status(status: OcrStatus, config_langs: &[String]) -> Result<Self, String> {
        if !status.available { return Err(OCR_INSTALL_PROMPT.to_string()); }
        let tesseract_exe = status.tesseract_path.map(PathBuf::from).ok_or_else(|| OCR_INSTALL_PROMPT.to_string())?;
        let tessdata_dir = status.tessdata_dir.map(PathBuf::from).ok_or_else(|| OCR_INSTALL_PROMPT.to_string())?;
        let langs = effective_langs(config_langs)?;
        let provenance = hash_local_traineddata(&tessdata_dir, &langs)?;
        let engine_id = format_ocr_engine_id(status.version.as_deref());
        Ok(Self {
            tesseract_exe, tessdata_dir, engine_id, provenance,
            page_timeout: DEFAULT_OCR_PAGE_TIMEOUT, max_output_bytes: DEFAULT_OCR_MAX_OUTPUT_BYTES,
        })
    }

    pub fn with_page_timeout(mut self, timeout: Duration) -> Self { self.page_timeout = timeout; self }
    pub fn with_max_output_bytes(mut self, max_bytes: usize) -> Self { self.max_output_bytes = max_bytes; self }
    pub fn tesseract_exe(&self) -> &Path { &self.tesseract_exe }
    pub fn tessdata_dir(&self) -> &Path { &self.tessdata_dir }
}

impl OcrPageRunner for LocalTesseractPageRunner {
    fn engine_id(&self) -> &str { &self.engine_id }
    fn model_provenance(&self) -> Vec<OcrModelProvenance> { self.provenance.clone() }

    fn ocr_png(&self, page_1based: usize, png_bytes: &[u8], dpi: u16, langs: &[String]) -> Result<String, String> {
        if page_1based == 0 { return Err("page number must be >= 1".into()); }
        if png_bytes.is_empty() { return Err(format!("empty PNG input for page {page_1based}")); }
        if !(MIN_RENDER_DPI..=MAX_RENDER_DPI).contains(&dpi) {
            return Err(format!("OCR DPI {dpi} out of bounds ({MIN_RENDER_DPI}..={MAX_RENDER_DPI})"));
        }
        let lang_spec = effective_langs(langs)?.join("+");
        let temp = OcrTempDirGuard::create()?;
        fs::write(temp.0.join("input.png"), png_bytes)
            .map_err(|e| format!("failed writing temp OCR PNG for page {page_1based}: {e}"))?;
        let stderr_path = temp.0.join("stderr.log");
        let stderr_file = File::create(&stderr_path).map_err(|e| format!("failed creating temp OCR stderr log: {e}"))?;

        let mut cmd = Command::new(&self.tesseract_exe);
        cmd.env_clear().current_dir(&temp.0).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::from(stderr_file))
            .env("TESSDATA_PREFIX", &self.tessdata_dir).env("OMP_THREAD_LIMIT", "1");
        if let Some(path_env) = std::env::var_os("PATH") { cmd.env("PATH", path_env); }
        #[cfg(windows)]
        {
            if let Some(sysroot) = std::env::var_os("SYSTEMROOT") { cmd.env("SYSTEMROOT", sysroot); }
            cmd.env("TEMP", &temp.0).env("TMP", &temp.0);
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        cmd.args(["input.png", "out", "--tessdata-dir"]).arg(&self.tessdata_dir)
            .args(["-l", &lang_spec, "--dpi", &dpi.to_string()]);

        let mut child = cmd.spawn().map_err(|e| format!("failed spawning local tesseract for page {page_1based}: {e}"))?;
        let deadline = Instant::now() + self.page_timeout.max(Duration::from_millis(1));
        let status = loop {
            match child.try_wait().map_err(|e| format!("failed polling local tesseract for page {page_1based}: {e}"))? {
                Some(st) => break st,
                None if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("local tesseract timed out on page {page_1based} after {} ms", self.page_timeout.as_millis()));
                }
                None => std::thread::sleep(OCR_POLL_INTERVAL),
            }
        };
        if !status.success() {
            let detail = read_stderr_snippet(&stderr_path);
            return Err(if detail.is_empty() {
                format!("local tesseract exited with status {status} on page {page_1based}")
            } else {
                format!("local tesseract exited with status {status} on page {page_1based}: {detail}")
            });
        }

        let cap = u64::try_from(self.max_output_bytes).ok().and_then(|n| n.checked_add(1)).ok_or("OCR output cap overflow")?;
        let mut buf = Vec::new();
        File::open(temp.0.join("out.txt"))
            .map_err(|e| format!("local tesseract produced no output file on page {page_1based}: {e}"))?
            .take(cap).read_to_end(&mut buf)
            .map_err(|e| format!("failed reading local tesseract output on page {page_1based}: {e}"))?;
        if buf.len() > self.max_output_bytes {
            return Err(format!("local tesseract output on page {page_1based} exceeds {} byte limit", self.max_output_bytes));
        }
        Ok(sanitize_ocr_text(&String::from_utf8_lossy(&buf)))
    }
}

fn read_stderr_snippet(path: &Path) -> String {
    let Ok(file) = File::open(path) else { return String::new(); };
    let mut buf = Vec::new();
    let _ = file.take(MAX_STDERR_SNIPPET_BYTES).read_to_end(&mut buf);
    sanitize_ocr_text(&String::from_utf8_lossy(&buf)).lines().take(3).collect::<Vec<_>>().join(" | ")
}

/// Options controlling PDF text extraction and selective local OCR on `PageBasis::None` pages.
#[derive(Debug, Clone)]
pub struct PdfOcrOptions {
    pub extract_opts: PdfExtractOptions,
    pub local_config: OcrLocalConfig,
    pub dpi: u16,
    pub max_ocr_pages: usize,
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Default for PdfOcrOptions {
    fn default() -> Self {
        Self {
            extract_opts: PdfExtractOptions::user_supplied(),
            local_config: OcrLocalConfig::default(),
            dpi: DEFAULT_RENDER_DPI,
            max_ocr_pages: MAX_PDF_PAGES,
            cancel: None,
        }
    }
}

/// Per-page OCR progress event reported during [`extract_and_ocr_pdf_pages_with_progress`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PdfOcrProgress {
    pub page: usize,
    pub done: usize,
    pub total_ocr_pages: usize,
}

/// Runtime provenance of an OCR pass pinned into `source.ingested`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PdfOcrProvenance {
    pub ocr_engine: String,
    pub ocr_models: Vec<OcrModelProvenance>,
    pub ocr_dpi: u16,
    pub pages_ocr_applied: Vec<usize>,
    pub pages_ocr_skipped: Vec<usize>,
    pub pages_ocr_failed: Vec<(usize, String)>,
}

/// Combined PDF text extraction and selective local OCR outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PdfOcrOutcome {
    pub document: PdfDocumentPages,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<PdfOcrProvenance>,
    pub ocr_available: bool,
    pub ocr_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr_message: Option<String>,
}

fn is_cancelled(cancel: &Option<Arc<AtomicBool>>) -> bool {
    cancel.as_ref().is_some_and(|f| f.load(Ordering::Relaxed))
}

pub(crate) fn sanitize_ocr_text(raw: &str) -> String {
    let mut cleaned = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\0' => {}
            '\r' => { if chars.peek() == Some(&'\n') { chars.next(); } cleaned.push('\n'); }
            PAGE_BREAK_CHAR => cleaned.push('\n'),
            _ => cleaned.push(c),
        }
    }
    cleaned.trim().to_string()
}

struct PageEntry {
    page: usize,
    text: String,
    basis: PageBasis,
    metrics: PageQualityMetrics,
    reason: Option<String>,
}

fn split_document_pages(doc: &PdfDocumentPages) -> Vec<PageEntry> {
    let lines: Vec<&str> = if doc.text.is_empty() { Vec::new() } else { doc.text.split('\n').collect() };
    doc.page_map
        .pages
        .iter()
        .map(|rec| {
            let text = match (rec.first_line, rec.last_line) {
                (Some(f), Some(l)) if f >= 1 && l >= f && l <= lines.len() => lines[(f - 1)..l].join("\n"),
                _ => String::new(),
            };
            PageEntry { page: rec.page, text, basis: rec.basis, metrics: rec.metrics.clone(), reason: rec.reason.clone() }
        })
        .collect()
}

fn assemble_ocr_reading_file(extractor_id: &str, total_pages: usize, entries: Vec<PageEntry>) -> PdfDocumentPages {
    if !entries.iter().any(|e| !e.text.is_empty()) {
        let pages = entries
            .into_iter()
            .map(|e| PageRecord { page: e.page, basis: e.basis, first_line: None, last_line: None, metrics: e.metrics, reason: e.reason })
            .collect();
        return PdfDocumentPages {
            extractor_id: extractor_id.to_string(),
            status: PdfExtractStatus::EmptyTextLayer,
            page_count: total_pages,
            page_map: PageMap { page_count: total_pages, pages },
            text: String::new(),
            warning: Some("PDF contains no extractable text layer (likely scanned or image-only).".into()),
        };
    }

    let mut combined = String::new();
    let mut records = Vec::with_capacity(entries.len());
    let mut cur_line = 1usize;
    for (idx, entry) in entries.into_iter().enumerate() {
        if idx > 0 {
            combined.push('\n');
            combined.push(PAGE_BREAK_CHAR);
            combined.push('\n');
            cur_line += 1;
        }
        let (first_line, last_line) = if entry.text.is_empty() {
            cur_line += 1;
            (None, None)
        } else {
            let line_count = entry.text.split('\n').count();
            let (first, last) = (cur_line, cur_line + line_count - 1);
            combined.push_str(&entry.text);
            cur_line = last + 1;
            (Some(first), Some(last))
        };
        records.push(PageRecord {
            page: entry.page, basis: entry.basis, first_line, last_line, metrics: entry.metrics, reason: entry.reason,
        });
    }

    let none_n = records.iter().filter(|r| r.basis.needs_ocr()).count();
    let ocr_n = records.iter().filter(|r| r.basis == PageBasis::Ocr).count();
    let scan_n = records.iter().filter(|r| r.basis == PageBasis::ScanTextLayer).count();
    let total = records.len();
    let warning = if none_n > 0 && ocr_n > 0 {
        Some(format!("{ocr_n} of {total} extracted page(s) use OCR text; {none_n} page(s) still have no usable text layer."))
    } else if none_n > 0 {
        Some(format!("{none_n} of {total} extracted page(s) have no usable text layer (OCR needed)."))
    } else if ocr_n > 0 {
        Some(format!("{ocr_n} of {total} extracted page(s) use local OCR text (verify formulas against page images)."))
    } else if scan_n > 0 {
        Some(format!("{scan_n} of {total} extracted page(s) appear to be scanned with an embedded text layer."))
    } else {
        None
    };

    PdfDocumentPages {
        extractor_id: extractor_id.to_string(),
        status: PdfExtractStatus::Ok,
        page_count: total_pages,
        page_map: PageMap { page_count: total_pages, pages: records },
        text: combined,
        warning,
    }
}

fn ocr_single_page(
    pdf_bytes: &[u8],
    page: usize,
    dpi: u16,
    langs: &[String],
    renderer: &dyn PdfPageRenderer,
    runner: &dyn OcrPageRunner,
) -> Result<(String, usize), String> {
    let rendered = renderer.render_page(pdf_bytes, page, dpi).map_err(|e| format!("render failed: {e}"))?;
    let raw = runner.ocr_png(page, &rendered.png_bytes, dpi, langs).map_err(|e| format!("OCR failed: {e}"))?;
    let clean = sanitize_ocr_text(&raw);
    let char_count = clean.chars().filter(|c| !c.is_whitespace()).count();
    if clean.is_empty() || char_count == 0 {
        return Err("OCR produced no recognizable text".into());
    }
    Ok((clean, char_count))
}

/// Extracts digital text from `pdf_bytes` and selectively runs local OCR strictly on
/// `PageBasis::None` pages, reporting per-page progress via `on_progress`.
pub fn extract_and_ocr_pdf_pages_with_progress(
    pdf_bytes: &[u8],
    opts: &PdfOcrOptions,
    renderer: &dyn PdfPageRenderer,
    runner: Option<&dyn OcrPageRunner>,
    on_progress: &mut dyn FnMut(PdfOcrProgress),
) -> Result<PdfOcrOutcome, PdfError> {
    if is_cancelled(&opts.cancel) { return Err(PdfError::Cancelled); }
    let mut extract_opts = opts.extract_opts.clone();
    if opts.cancel.is_some() { extract_opts.cancel = opts.cancel.clone(); }
    let doc = extract_pdf_pages(pdf_bytes, &extract_opts)?;
    let pages_needing_ocr = doc.page_map.pages_needing_ocr();
    if pages_needing_ocr.is_empty() {
        return Ok(PdfOcrOutcome {
            document: doc, provenance: None, ocr_available: runner.is_some(), ocr_status: "not_needed".into(), ocr_message: None,
        });
    }
    if !opts.local_config.enabled {
        return Ok(PdfOcrOutcome {
            document: doc, provenance: None, ocr_available: runner.is_some(), ocr_status: "disabled".into(),
            ocr_message: Some("OCR disabled in settings.".into()),
        });
    }
    let Some(ocr_runner) = runner else {
        return Ok(PdfOcrOutcome {
            document: doc, provenance: None, ocr_available: false, ocr_status: "unavailable".into(),
            ocr_message: Some(OCR_INSTALL_PROMPT.to_string()),
        });
    };

    let langs = effective_langs(&opts.local_config.langs)
        .map_err(|e| PdfError::Malformed(format!("invalid OCR language config: {e}")))?;
    let mut entries = split_document_pages(&doc);
    let mut total_bytes: usize = entries.iter().map(|e| e.text.len()).sum();
    let (mut pages_ocr_applied, mut pages_ocr_skipped, mut pages_ocr_failed) = (Vec::new(), Vec::new(), Vec::new());
    pages_ocr_skipped.extend(entries.iter().filter(|e| !e.basis.needs_ocr()).map(|e| e.page));
    let total_ocr_pages = pages_needing_ocr.len().min(opts.max_ocr_pages);
    pages_ocr_skipped.extend(pages_needing_ocr.iter().skip(total_ocr_pages).copied());

    for (idx, &page) in pages_needing_ocr.iter().take(total_ocr_pages).enumerate() {
        if is_cancelled(&opts.cancel) { return Err(PdfError::Cancelled); }
        match ocr_single_page(pdf_bytes, page, opts.dpi, &langs, renderer, ocr_runner) {
            Ok((clean, char_count)) => {
                if let Some(entry) = entries.iter_mut().find(|e| e.page == page) {
                    let next_total = total_bytes.saturating_sub(entry.text.len()).saturating_add(clean.len());
                    if next_total > opts.extract_opts.max_extracted_bytes { return Err(PdfError::OutputTooLarge); }
                    total_bytes = next_total;
                    entry.metrics.char_count = char_count;
                    entry.metrics.alpha_count = clean.chars().filter(|c| c.is_alphabetic()).count();
                    entry.metrics.word_count = clean.split_whitespace().filter(|w| w.chars().any(|c| c.is_alphanumeric())).count();
                    entry.text = clean;
                    entry.basis = PageBasis::Ocr;
                    entry.reason = Some("OCR text extracted via local Tesseract".into());
                    pages_ocr_applied.push(page);
                }
            }
            Err(err) => pages_ocr_failed.push((page, err)),
        }
        on_progress(PdfOcrProgress { page, done: idx + 1, total_ocr_pages });
    }

    let assembled = assemble_ocr_reading_file(&doc.extractor_id, doc.page_count, entries);
    let remaining_none = assembled.page_map.pages_needing_ocr().len();
    let (ocr_status, ocr_message) = if !pages_ocr_applied.is_empty() && remaining_none == 0 && pages_ocr_failed.is_empty() {
        ("applied".to_string(), None)
    } else if !pages_ocr_applied.is_empty() {
        ("partial".to_string(), Some(format!("OCR applied to {} page(s); {remaining_none} page(s) remain without text.", pages_ocr_applied.len())))
    } else {
        ("partial".to_string(), Some(format!("OCR produced no text; {remaining_none} page(s) remain without text.")))
    };

    Ok(PdfOcrOutcome {
        document: assembled,
        provenance: Some(PdfOcrProvenance {
            ocr_engine: ocr_runner.engine_id().to_string(),
            ocr_models: ocr_runner.model_provenance(),
            ocr_dpi: opts.dpi,
            pages_ocr_applied,
            pages_ocr_skipped,
            pages_ocr_failed,
        }),
        ocr_available: true,
        ocr_status,
        ocr_message,
    })
}

/// Synchronous convenience wrapper around [`extract_and_ocr_pdf_pages_with_progress`].
pub fn extract_and_ocr_pdf_pages(
    pdf_bytes: &[u8],
    opts: &PdfOcrOptions,
    renderer: &dyn PdfPageRenderer,
    runner: Option<&dyn OcrPageRunner>,
) -> Result<PdfOcrOutcome, PdfError> {
    let mut noop = |_| {};
    extract_and_ocr_pdf_pages_with_progress(pdf_bytes, opts, renderer, runner, &mut noop)
}

/// Caches extracted/OCR reading text and per-page [`PageBasis`] in the F10 [`Library`] `paper_text` table.
pub fn cache_pdf_ocr_in_library(
    lib: &Library, paper_key: &PaperKey, version: Option<&str>, pdf_bytes: &[u8], outcome: &PdfOcrOutcome, now_ms: i64,
) -> Result<(), String> {
    if lib.get_paper(paper_key)?.is_none() {
        lib.upsert_paper(&PaperRecord {
            key: paper_key.clone(), ids: LibraryIds::default(), title: String::new(), authors: Vec::new(),
            year: None, abstract_text: String::new(), bibtex: String::new(), flags: PaperFlags::default(),
            first_seen_ms: now_ms, last_seen_ms: now_ms,
        })?;
    }
    let canonical = lib.resolve_key(paper_key)?.unwrap_or_else(|| paper_key.clone());
    let extractor = match &outcome.provenance {
        Some(prov) if !prov.pages_ocr_applied.is_empty() => format!("{}+{}", outcome.document.extractor_id, prov.ocr_engine),
        _ => outcome.document.extractor_id.clone(),
    };
    let page_basis = outcome.document.page_map.pages.iter().map(|rec| match rec.basis {
        PageBasis::TextLayer => LibPageBasis::TextLayer,
        PageBasis::ScanTextLayer => LibPageBasis::ScanTextLayer,
        PageBasis::Ocr => LibPageBasis::Ocr,
        PageBasis::None => LibPageBasis::None,
    }).collect();
    let record = PaperTextRecord {
        key: canonical, version: version.map(str::to_string), file_sha256: ledger::sha256_hex(pdf_bytes),
        extractor, text_blob: ledger::sha256_hex(outcome.document.text.as_bytes()), page_basis, extracted_ms: now_ms,
    };
    lib.upsert_paper_text(&record, Some(outcome.document.text.as_bytes()))
}

#[cfg(test)]
#[path = "pdf_ocr_tests.rs"]
mod tests;
