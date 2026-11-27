//! Authenticated F14b scanned-PDF OCR ingestion, background job registry,
//! and verified PDF page-image / crop serving routes.

use crate::http_srv::{write_api_res, write_bytes};
use crate::ocr_settings::{effective_ocr_status, load_ocr_settings};
use crate::routes_session::{bad, conflict, internal, not_found, parse_json_body, ApiResult};
use crate::state::{decode_base64, parse_multipart_files, DesktopState};
use research_tools::ingest_ocr::{
    ingest_pdf_with_ocr, pin_ocr_outcome_to_session, OcrIngestMeta, OcrIngestRequest,
    OcrIngestedSource,
};
use research_tools::library::Library;
use research_tools::pdf::PdfError;
use research_tools::pdf_ocr::{OcrPageRunner, PdfOcrOptions};
use research_tools::pdf_ocr_job::{PdfOcrJob, PdfOcrJobSnapshot};
use research_tools::pdf_ocr_models::{
    effective_langs, format_ocr_engine_id, hash_local_traineddata, OcrLocalConfig,
    OcrModelProvenance,
};
use research_tools::pdf_pages::{PdfExtractOptions, MAX_USER_PDF_BYTES};
use research_tools::pdf_render::{
    CropRect, PdfPageRenderer, RasterXObjectRenderer, RenderedPage, DEFAULT_RENDER_DPI,
    MAX_RENDER_DPI, MIN_RENDER_DPI,
};
use research_tools::{validate_session_id, WorkspaceSandbox};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tools::canon::random_hex;
use worker::ocr_helper::{
    OcrEngine, OcrPageInput, TesseractCliConfig, TesseractCliEngine, DEFAULT_OCR_PAGE_TIMEOUT,
    MAX_OCR_OUTPUT_TEXT_BYTES,
};

const MAX_OCR_JOBS: usize = 64;
const DEFAULT_PDF_FILENAME: &str = "scan.pdf";

static OCR_JOBS: OnceLock<Mutex<HashMap<String, Arc<StoredOcrJob>>>> = OnceLock::new();
static OCR_JOB_SEQ: AtomicU64 = AtomicU64::new(1);

fn ocr_jobs() -> &'static Mutex<HashMap<String, Arc<StoredOcrJob>>> {
    OCR_JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Bridges [`OcrPageRunner`] over the contained [`TesseractCliEngine`]
/// (Windows AppContainerSandbox with `network_restricted`, `filesystem_restricted`, `KILL_ON_JOB_CLOSE`).
#[derive(Debug, Clone)]
pub struct WorkerOcrRunner {
    engine: TesseractCliEngine,
    provenance: Vec<OcrModelProvenance>,
}

impl WorkerOcrRunner {
    /// Builds a contained local Tesseract runner when local Tesseract and required
    /// `.traineddata` files are available; returns `None` otherwise.
    pub fn from_config(data_dir: &Path, config: &OcrLocalConfig) -> Option<Self> {
        let status = effective_ocr_status(config);
        if !status.available { return None; }
        let tesseract_exe = PathBuf::from(status.tesseract_path?);
        let tessdata_dir = PathBuf::from(status.tessdata_dir?);
        let langs = effective_langs(&config.langs).ok()?;
        let provenance = hash_local_traineddata(&tessdata_dir, &langs).ok()?;
        let engine = TesseractCliEngine::new(TesseractCliConfig {
            tesseract_exe, tessdata_dir, data_dir: data_dir.to_path_buf(),
            engine_id: format_ocr_engine_id(status.version.as_deref()),
            page_timeout: DEFAULT_OCR_PAGE_TIMEOUT, max_output_bytes: MAX_OCR_OUTPUT_TEXT_BYTES,
        });
        Some(Self { engine, provenance })
    }
}

impl OcrPageRunner for WorkerOcrRunner {
    fn engine_id(&self) -> &str { self.engine.engine_id() }
    fn model_provenance(&self) -> Vec<OcrModelProvenance> { self.provenance.clone() }
    fn ocr_png(&self, page: usize, png_bytes: &[u8], dpi: u16, langs: &[String]) -> Result<String, String> {
        let input = OcrPageInput { page, png_bytes: png_bytes.to_vec(), dpi, langs: langs.to_vec() };
        self.engine.ocr_page(&input).map(|res| res.text).map_err(|e| e.to_string())
    }
}

struct StoredOcrJob {
    session_id: String,
    created_seq: u64,
    cancel_flag: Arc<AtomicBool>,
    inner: Mutex<OcrJobInner>,
}

struct OcrJobInner {
    file_name: String,
    title: Option<String>,
    raw_bytes: Vec<u8>,
    data_dir: PathBuf,
    workspace_root: Option<PathBuf>,
    job: Option<PdfOcrJob>,
    final_snapshot: Option<PdfOcrJobSnapshot>,
    source: Option<OcrIngestedSource>,
    error: Option<String>,
}

impl StoredOcrJob {
    fn is_finished_nonblocking(&self) -> bool {
        self.inner.try_lock().is_ok_and(|g| g.job.as_ref().is_none_or(PdfOcrJob::is_finished))
    }
}

struct ParsedOcrIngest {
    file_name: String,
    raw_bytes: Vec<u8>,
    title: Option<String>,
    async_mode: bool,
    page_range: Option<(usize, usize)>,
    dpi: u16,
}

fn require_session(state: &DesktopState, session: &str) -> Result<(), (u16, String)> {
    validate_session_id(session).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session).map_err(internal)? {
        return Err(not_found(format!("session `{session}` not found")));
    }
    Ok(())
}

fn validate_job_id(job_id: &str) -> Result<(), (u16, String)> {
    let ok = !job_id.is_empty()
        && job_id.len() <= 128
        && job_id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'));
    if !ok { return Err(bad(format!("invalid OCR job_id `{job_id}`"))); }
    Ok(())
}

fn non_empty_query(query: &HashMap<String, String>, key: &str) -> Option<String> {
    query.get(key).map(|s| s.trim()).filter(|s| !s.is_empty()).map(str::to_string)
}

fn parse_query_bool(raw: Option<&String>) -> Result<bool, (u16, String)> {
    let Some(val) = raw.map(|s| s.trim()).filter(|s| !s.is_empty()) else { return Ok(false); };
    match val.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" => Ok(true),
        "0" | "false" | "no" => Ok(false),
        _ => Err(bad(format!("invalid boolean query value `{val}`"))),
    }
}

fn parse_dpi_value(dpi: u16) -> Result<u16, (u16, String)> {
    if !(MIN_RENDER_DPI..=MAX_RENDER_DPI).contains(&dpi) {
        return Err(bad(format!("`dpi` ({dpi}) must be between {MIN_RENDER_DPI} and {MAX_RENDER_DPI}.")));
    }
    Ok(dpi)
}

fn parse_query_dpi(query: &HashMap<String, String>) -> Result<u16, (u16, String)> {
    let Some(raw) = non_empty_query(query, "dpi") else { return Ok(DEFAULT_RENDER_DPI); };
    let dpi = raw.parse::<u16>().map_err(|_| bad(format!("invalid `dpi` parameter `{raw}`")))?;
    parse_dpi_value(dpi)
}

fn validate_page_range(start: Option<usize>, end: Option<usize>) -> Result<Option<(usize, usize)>, (u16, String)> {
    match (start, end) {
        (None, None) => Ok(None),
        (Some(s), Some(e)) if s >= 1 && e >= s => Ok(Some((s, e))),
        (Some(_), Some(_)) => Err(bad("`page_start` and `page_end` must satisfy 1 <= page_start <= page_end.")),
        _ => Err(bad("Provide both `page_start` and `page_end` together.")),
    }
}

fn parse_query_page_range(query: &HashMap<String, String>) -> Result<Option<(usize, usize)>, (u16, String)> {
    let parse_opt = |key: &str| match non_empty_query(query, key) {
        Some(raw) => raw.parse::<usize>().map(Some).map_err(|_| bad(format!("invalid `{key}` parameter `{raw}`"))),
        None => Ok(None),
    };
    validate_page_range(parse_opt("page_start")?, parse_opt("page_end")?)
}

fn has_pdf_header(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(1024)].windows(5).any(|w| w == b"%PDF-")
}

fn sanitize_filename(raw: Option<String>) -> Result<String, (u16, String)> {
    let name = raw.unwrap_or_else(|| DEFAULT_PDF_FILENAME.to_string());
    if name.contains('\0') { return Err(bad("`file_name` must not contain NUL bytes.")); }
    let trimmed = name.trim();
    Ok(if trimmed.is_empty() { DEFAULT_PDF_FILENAME.to_string() } else { trimmed.to_string() })
}

fn parse_json_usize(obj: &Map<String, Value>, key: &str) -> Result<Option<usize>, (u16, String)> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().and_then(|n| usize::try_from(n).ok()).map(Some)
            .ok_or_else(|| bad(format!("`{key}` must be a positive integer."))),
    }
}

fn parse_json_ingest(query: &HashMap<String, String>, body: &[u8]) -> Result<ParsedOcrIngest, (u16, String)> {
    let val = parse_json_body(body)?;
    let Some(obj) = val.as_object() else { return Err(bad("OCR ingest body must be a JSON object.")); };
    let file_name = match obj.get("file_name") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return Err(bad("`file_name` must be a string.")),
    };
    let b64 = obj.get("bytes_base64").or_else(|| obj.get("content_base64")).and_then(Value::as_str)
        .ok_or_else(|| bad("`bytes_base64` (or `content_base64`) is required."))?;
    let raw_bytes = decode_base64(b64).map_err(|e| bad(format!("invalid base64 PDF payload: {e}")))?;
    let title = match obj.get("title") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => (!s.trim().is_empty()).then(|| s.trim().to_string()),
        Some(_) => return Err(bad("`title` must be a string or null.")),
    };
    let async_mode = match obj.get("async") {
        None | Some(Value::Null) => parse_query_bool(query.get("async"))?,
        Some(v) => v.as_bool().ok_or_else(|| bad("`async` must be a boolean."))?,
    };
    let page_range = validate_page_range(parse_json_usize(obj, "page_start")?, parse_json_usize(obj, "page_end")?)?;
    let dpi = match obj.get("dpi") {
        None | Some(Value::Null) => DEFAULT_RENDER_DPI,
        Some(v) => parse_dpi_value(
            v.as_u64().and_then(|n| u16::try_from(n).ok()).ok_or_else(|| bad("`dpi` must be an integer."))?,
        )?,
    };
    Ok(ParsedOcrIngest { file_name: sanitize_filename(file_name)?, raw_bytes, title, async_mode, page_range, dpi })
}

fn parse_ingest_request(
    content_type: Option<&str>,
    query: &HashMap<String, String>,
    body: &[u8],
) -> Result<ParsedOcrIngest, (u16, String)> {
    let ct_lower = content_type.unwrap_or("").to_ascii_lowercase();
    let parsed = if ct_lower.contains("application/pdf") || ct_lower.contains("multipart/form-data") {
        let (file_name, raw_bytes) = if ct_lower.contains("application/pdf") {
            (sanitize_filename(non_empty_query(query, "file_name"))?, body.to_vec())
        } else {
            let ct = content_type.ok_or_else(|| bad("missing Content-Type header"))?;
            let (name, bytes) = parse_multipart_files(ct, body)?
                .into_iter()
                .find(|(n, b)| n.to_ascii_lowercase().ends_with(".pdf") || has_pdf_header(b))
                .ok_or_else(|| bad("multipart/form-data upload must include a PDF file."))?;
            (sanitize_filename(Some(name))?, bytes)
        };
        ParsedOcrIngest {
            file_name,
            raw_bytes,
            title: non_empty_query(query, "title"),
            async_mode: parse_query_bool(query.get("async"))?,
            page_range: parse_query_page_range(query)?,
            dpi: parse_query_dpi(query)?,
        }
    } else {
        parse_json_ingest(query, body)?
    };
    if parsed.raw_bytes.is_empty() { return Err(bad("PDF payload is empty.")); }
    if parsed.raw_bytes.len() > MAX_USER_PDF_BYTES {
        return Err((413, format!("PDF payload exceeds {} MiB limit.", MAX_USER_PDF_BYTES / (1024 * 1024))));
    }
    if !has_pdf_header(&parsed.raw_bytes) {
        return Err(bad("Payload is not a valid PDF (missing %PDF- header)."));
    }
    Ok(parsed)
}

fn register_ocr_job(job_id: String, slot: Arc<StoredOcrJob>) -> Result<(), (u16, String)> {
    let mut map = ocr_jobs().lock().map_err(|_| internal("OCR job registry lock poisoned"))?;
    if map.len() >= MAX_OCR_JOBS {
        let oldest = map.iter().filter(|(_, s)| s.is_finished_nonblocking())
            .min_by_key(|(_, s)| s.created_seq).map(|(k, _)| k.clone());
        match oldest {
            Some(key) => { map.remove(&key); }
            None => {
                slot.cancel_flag.store(true, Ordering::Relaxed);
                return Err(conflict("Too many concurrent OCR jobs are running; wait for one to finish."));
            }
        }
    }
    map.insert(job_id, slot);
    Ok(())
}

fn lookup_session_job(session: &str, job_id: &str) -> Result<Arc<StoredOcrJob>, (u16, String)> {
    let map = ocr_jobs().lock().map_err(|_| internal("OCR job registry lock poisoned"))?;
    match map.get(job_id) {
        Some(slot) if slot.session_id == session => Ok(Arc::clone(slot)),
        _ => Err(not_found(format!("OCR job `{job_id}` not found"))),
    }
}

/// HTTP handler for `POST /api/sessions/{id}/ocr/ingest`.
pub fn handle_ocr_ingest(
    state: &DesktopState,
    session: &str,
    content_type: Option<&str>,
    query: &HashMap<String, String>,
    body: &[u8],
) -> ApiResult {
    require_session(state, session)?;
    let parsed = parse_ingest_request(content_type, query, body)?;
    let local_config = load_ocr_settings(&state.cfg.data_dir);
    let mut extract_opts = PdfExtractOptions::user_supplied();
    if let Some((start, end)) = parsed.page_range {
        extract_opts = extract_opts.with_page_range(start, end);
    }
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let ocr_opts = PdfOcrOptions {
        extract_opts,
        local_config: local_config.clone(),
        dpi: parsed.dpi,
        cancel: Some(Arc::clone(&cancel_flag)),
        ..PdfOcrOptions::default()
    };

    let env = state.research_env_for_session(session).map_err(bad)?;
    let runner = WorkerOcrRunner::from_config(&state.cfg.data_dir, &local_config);

    if parsed.async_mode {
        let job_id = format!("ocr-{}", random_hex(8));
        let renderer: Arc<dyn PdfPageRenderer> = Arc::new(RasterXObjectRenderer::default());
        let runner_arc = runner.map(|r| Arc::new(r) as Arc<dyn OcrPageRunner>);
        let job = PdfOcrJob::start(job_id.clone(), parsed.raw_bytes.clone(), ocr_opts, renderer, runner_arc);
        let snapshot = job.snapshot();
        let slot = Arc::new(StoredOcrJob {
            session_id: session.to_string(),
            created_seq: OCR_JOB_SEQ.fetch_add(1, Ordering::Relaxed),
            cancel_flag,
            inner: Mutex::new(OcrJobInner {
                file_name: parsed.file_name,
                title: parsed.title,
                raw_bytes: parsed.raw_bytes,
                data_dir: env.data_dir,
                workspace_root: env.workspace_root,
                job: Some(job),
                final_snapshot: None,
                source: None,
                error: None,
            }),
        });
        register_ocr_job(job_id.clone(), slot)?;
        return Ok((202, json!({ "job_id": job_id, "snapshot": snapshot })));
    }

    let renderer = RasterXObjectRenderer::default();
    let sandbox = WorkspaceSandbox::new(env.data_dir.clone(), env.workspace_root.clone());
    let ledger = Mutex::new(state.open_ledger().map_err(internal)?);
    let library = Library::open(&state.cfg.data_dir).ok();
    let req = OcrIngestRequest {
        session_id: session,
        file_name: &parsed.file_name,
        origin: &parsed.file_name,
        raw_bytes: &parsed.raw_bytes,
        source_id_hint: None,
        title: parsed.title.as_deref(),
        ocr_opts,
    };
    let runner_ref = runner.as_ref().map(|r| r as &dyn OcrPageRunner);
    let ingested = ingest_pdf_with_ocr(&sandbox, &ledger, library.as_ref(), &req, &renderer, runner_ref)
        .map_err(bad)?;
    Ok((200, serde_json::to_value(ingested).map_err(|e| internal(e.to_string()))?))
}

/// HTTP handler for `GET /api/sessions/{id}/ocr/jobs/{job_id}`.
pub fn handle_ocr_job_status(state: &DesktopState, session: &str, job_id: &str) -> ApiResult {
    require_session(state, session)?;
    validate_job_id(job_id)?;
    let slot = lookup_session_job(session, job_id)?;
    let mut guard = slot.inner.lock().map_err(|_| internal("OCR job state lock poisoned"))?;
    let inner = &mut *guard;

    if inner.job.as_ref().is_some_and(PdfOcrJob::is_finished) {
        if let Some(job) = inner.job.take() {
            let snap = job.snapshot();
            let raw_bytes = std::mem::take(&mut inner.raw_bytes);
            match job.join() {
                Ok(outcome) => {
                    let sandbox = WorkspaceSandbox::new(inner.data_dir.clone(), inner.workspace_root.clone());
                    let ledger = Mutex::new(state.open_ledger().map_err(internal)?);
                    let library = Library::open(&inner.data_dir).ok();
                    let pin_res = {
                        let meta = OcrIngestMeta {
                            session_id: &slot.session_id,
                            file_name: &inner.file_name,
                            origin: &inner.file_name,
                            source_id_hint: None,
                            title: inner.title.as_deref(),
                        };
                        pin_ocr_outcome_to_session(&sandbox, &ledger, library.as_ref(), &meta, &raw_bytes, outcome)
                    };
                    match pin_res {
                        Ok(ingested) => inner.source = Some(ingested),
                        Err(err) => inner.error = Some(err),
                    }
                }
                Err(PdfError::Cancelled) => {}
                Err(err) => inner.error = Some(err.to_string()),
            }
            inner.final_snapshot = Some(snap);
        }
    }

    let (finished, snapshot) = if let Some(ref job) = inner.job {
        (false, job.snapshot())
    } else if let Some(ref snap) = inner.final_snapshot {
        (true, snap.clone())
    } else {
        return Err(internal("OCR job snapshot unavailable"));
    };
    Ok((200, json!({
        "job_id": job_id, "snapshot": snapshot, "finished": finished,
        "source": inner.source, "error": inner.error,
    })))
}

/// HTTP handler for `POST /api/sessions/{id}/ocr/jobs/{job_id}/cancel`.
pub fn handle_ocr_job_cancel(state: &DesktopState, session: &str, job_id: &str) -> ApiResult {
    require_session(state, session)?;
    validate_job_id(job_id)?;
    let slot = lookup_session_job(session, job_id)?;
    slot.cancel_flag.store(true, Ordering::Relaxed);
    let inner = slot.inner.lock().map_err(|_| internal("OCR job state lock poisoned"))?;
    let snapshot = if let Some(ref job) = inner.job {
        job.cancel();
        job.snapshot()
    } else if let Some(ref snap) = inner.final_snapshot {
        snap.clone()
    } else {
        return Err(internal("OCR job snapshot unavailable"));
    };
    Ok((200, json!({ "job_id": job_id, "cancelled": true, "snapshot": snapshot })))
}

/// Renders a verified original PDF page (or normalized crop) to PNG bytes.
pub fn render_page_image(
    state: &DesktopState,
    session: &str,
    source: &str,
    query: &HashMap<String, String>,
) -> Result<RenderedPage, (u16, String)> {
    let page_raw = non_empty_query(query, "page")
        .ok_or_else(|| bad("missing required `page` query parameter."))?;
    let page = page_raw.parse::<usize>().ok().filter(|&p| p >= 1)
        .ok_or_else(|| bad(format!("invalid `page` parameter `{page_raw}` (must be >= 1)")))?;
    let dpi = parse_query_dpi(query)?;
    let crop = non_empty_query(query, "crop").map(|raw| CropRect::parse(&raw).map_err(bad)).transpose()?;
    let pdf = crate::original_pdf::read(state, session, source, None, None)?;
    let renderer = RasterXObjectRenderer::default();
    let res = match crop {
        Some(rect) => renderer.render_crop(&pdf.bytes, page, dpi, rect),
        None => renderer.render_page(&pdf.bytes, page, dpi),
    };
    res.map_err(|e| if e.contains("out of range") { not_found(e) } else { (422, e) })
}

/// HTTP handler for `GET /api/sessions/{id}/sources/{source}/page-image`.
pub fn serve_page_image(
    stream: &mut TcpStream,
    state: &DesktopState,
    session: &str,
    source: &str,
    query: &HashMap<String, String>,
) {
    match render_page_image(state, session, source, query) {
        Ok(rendered) => {
            let (page_s, w_s, h_s) = (rendered.page.to_string(), rendered.width.to_string(), rendered.height.to_string());
            write_bytes(stream, 200, "image/png", &rendered.png_bytes, &[
                ("Cache-Control", "no-store"),
                ("X-Content-Type-Options", "nosniff"),
                ("Content-Security-Policy", "sandbox"),
                ("X-ReSearch-Pdf-Page", &page_s),
                ("X-ReSearch-Image-Width", &w_s),
                ("X-ReSearch-Image-Height", &h_s),
            ]);
        }
        Err(err) => write_api_res(stream, Err(err)),
    }
}
