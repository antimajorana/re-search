//! 64 MiB scanned-PDF ingestion with selective local OCR, immutable `source.ingested`
//! OCR provenance pinning, and F10 `Library` `paper_text` caching (package F14b).

use crate::ingest::IngestedSource;
use crate::library::Library;
use crate::library_marks::now_ms;
use crate::paper_key::PaperKey;
use crate::pdf::PdfExtractStatus;
use crate::pdf_ocr::{
    cache_pdf_ocr_in_library, extract_and_ocr_pdf_pages, OcrPageRunner, PdfOcrOptions,
    PdfOcrOutcome,
};
use crate::pdf_ocr_models::OcrModelProvenance;
use crate::pdf_pages::{PageMap, MAX_USER_PDF_BYTES};
use crate::pdf_render::PdfPageRenderer;
use crate::sandbox::{sanitize_rel_components, validate_session_id, SandboxRoot, WorkspaceSandbox};
use crate::tex_bundle::clean_document_title;
use ledger::Ledger;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use tools::EVENT_SCHEMA_VERSION;

const MAX_EXCERPT_CHARS: usize = 12_000;

/// Request for synchronous PDF extraction + selective local OCR + session pinning.
#[derive(Debug, Clone)]
pub struct OcrIngestRequest<'a> {
    pub session_id: &'a str,
    pub file_name: &'a str,
    pub origin: &'a str,
    pub raw_bytes: &'a [u8],
    pub source_id_hint: Option<&'a str>,
    pub title: Option<&'a str>,
    pub ocr_opts: PdfOcrOptions,
}

impl<'a> OcrIngestRequest<'a> {
    pub fn meta(&self) -> OcrIngestMeta<'a> {
        OcrIngestMeta {
            session_id: self.session_id,
            file_name: self.file_name,
            origin: self.origin,
            source_id_hint: self.source_id_hint,
            title: self.title,
        }
    }
}

/// Session and source metadata used when pinning a precomputed [`PdfOcrOutcome`].
#[derive(Debug, Clone, Copy)]
pub struct OcrIngestMeta<'a> {
    pub session_id: &'a str,
    pub file_name: &'a str,
    pub origin: &'a str,
    pub source_id_hint: Option<&'a str>,
    pub title: Option<&'a str>,
}

/// Result of ingesting a PDF with selective OCR into a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcrIngestedSource {
    pub source: IngestedSource,
    pub ocr_available: bool,
    pub ocr_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr_engine: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ocr_models: Vec<OcrModelProvenance>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pages_ocr_applied: Vec<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pages_remaining_none: Vec<usize>,
    pub page_map: PageMap,
}

/// Reads a local PDF file up to [`MAX_USER_PDF_BYTES`] (64 MiB) and ingests it with selective OCR.
#[allow(clippy::too_many_arguments)]
pub fn ingest_pdf_path_with_ocr(
    sandbox: &WorkspaceSandbox,
    ledger: &Mutex<Ledger>,
    library: Option<&Library>,
    session_id: &str,
    path: &Path,
    ocr_opts: PdfOcrOptions,
    renderer: &dyn PdfPageRenderer,
    runner: Option<&dyn OcrPageRunner>,
) -> Result<OcrIngestedSource, String> {
    validate_session_id(session_id).map_err(|e| e.to_string())?;
    let file_name = path.file_name().and_then(|s| s.to_str()).ok_or_else(|| "invalid UTF-8 filename".to_string())?;
    let file = File::open(path).map_err(|e| format!("cannot open `{}`: {e}", path.display()))?;
    let cap = u64::try_from(MAX_USER_PDF_BYTES)
        .ok()
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| "PDF byte limit overflow".to_string())?;
    let mut raw_bytes = Vec::new();
    file.take(cap).read_to_end(&mut raw_bytes).map_err(|e| format!("cannot read `{}`: {e}", path.display()))?;
    enforce_user_pdf_cap(file_name, raw_bytes.len())?;
    let req = OcrIngestRequest {
        session_id,
        file_name,
        origin: file_name,
        raw_bytes: &raw_bytes,
        source_id_hint: None,
        title: None,
        ocr_opts,
    };
    ingest_pdf_with_ocr(sandbox, ledger, library, &req, renderer, runner)
}

/// Runs PDF text extraction + selective local OCR and pins the resulting reading file and
/// `source.ingested` ledger receipt into `req.session_id`.
pub fn ingest_pdf_with_ocr(
    sandbox: &WorkspaceSandbox,
    ledger: &Mutex<Ledger>,
    library: Option<&Library>,
    req: &OcrIngestRequest<'_>,
    renderer: &dyn PdfPageRenderer,
    runner: Option<&dyn OcrPageRunner>,
) -> Result<OcrIngestedSource, String> {
    validate_session_id(req.session_id).map_err(|e| e.to_string())?;
    enforce_user_pdf_cap(req.file_name, req.raw_bytes.len())?;
    let outcome = extract_and_ocr_pdf_pages(req.raw_bytes, &req.ocr_opts, renderer, runner)
        .map_err(|e| e.to_string())?;
    pin_ocr_outcome_to_session(sandbox, ledger, library, &req.meta(), req.raw_bytes, outcome)
}

/// Pins an already-computed [`PdfOcrOutcome`] (e.g. from [`crate::pdf_ocr_job::PdfOcrJob`])
/// into `sessions/<id>/sources/`, appends a fresh `source.ingested` receipt with OCR
/// provenance, and optionally caches text in [`Library`].
pub fn pin_ocr_outcome_to_session(
    sandbox: &WorkspaceSandbox,
    ledger: &Mutex<Ledger>,
    library: Option<&Library>,
    req_meta: &OcrIngestMeta<'_>,
    raw_bytes: &[u8],
    outcome: PdfOcrOutcome,
) -> Result<OcrIngestedSource, String> {
    validate_session_id(req_meta.session_id).map_err(|e| e.to_string())?;
    enforce_user_pdf_cap(req_meta.file_name, raw_bytes.len())?;

    let leaf = format!("{}.extracted.txt", sanitize_leaf_name(req_meta.file_name, "pdf"));
    let content = if outcome.document.status == PdfExtractStatus::EmptyTextLayer {
        String::new()
    } else {
        outcome.document.text.clone()
    };
    let content_sha = ledger::sha256_hex(content.as_bytes());
    let mut materialized_hashes = BTreeMap::new();
    materialized_hashes.insert(leaf.clone(), content_sha.clone());

    let identity = json!({
        "raw": ledger::sha256_hex(raw_bytes),
        "format": "pdf",
        "primary": Some(&leaf),
        "assembled": Value::Null,
        "files": materialized_hashes,
    });
    let source_id = build_source_id(req_meta.source_id_hint.unwrap_or(req_meta.file_name), &identity);

    let canon_sources = sandbox
        .canonical_root(SandboxRoot::Sources, req_meta.session_id, true)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "cannot initialize session sources root".to_string())?;
    let full_rel = format!("{source_id}/{leaf}");
    let (parts, norm_path) = sanitize_rel_components(&full_rel, false).map_err(|e| e.to_string())?;
    WorkspaceSandbox::atomic_write_in_root(&canon_sources, &parts, content.as_bytes())
        .map_err(|e| e.to_string())?;

    let written_files = vec![norm_path.clone()];
    let mut files_sha256 = BTreeMap::new();
    files_sha256.insert(norm_path.clone(), content_sha);
    let primary_path = Some(norm_path);
    let page_lines = outcome.document.page_map.page_lines();
    let arxiv_id = arxiv_cover_stamp(&page_lines, &content);
    let title = req_meta.title.and_then(clean_document_title);
    let status_str = outcome.document.status.as_str().to_string();

    let (raw_blob, event_seq) = {
        let guard = ledger.lock().map_err(|_| "ledger lock poisoned".to_string())?;
        let blob_hash = guard
            .put_blob(raw_bytes)
            .map_err(|e| format!("source.ingested put_blob failed: {e}"))?;
        let mut payload = json!({
            "source_id": source_id,
            "origin": req_meta.origin,
            "title": title,
            "format": "pdf",
            "status": status_str,
            "bytes": raw_bytes.len(),
            "raw_blob": blob_hash,
            "primary_path": primary_path,
            "assembled_path": Value::Null,
            "files": written_files,
            "files_sha256": files_sha256,
            "page_count": Some(outcome.document.page_count),
            "pdf_page_lines": page_lines,
            "arxiv_cover_id": arxiv_id,
            "pdf_extractor": outcome.document.extractor_id,
            "pdf_pages": outcome.document.page_map.pages,
            "ocr_status": outcome.ocr_status,
        });
        if let Some(msg) = &outcome.ocr_message {
            payload["ocr_message"] = json!(msg);
        }
        if let Some(prov) = &outcome.provenance {
            payload["ocr_engine"] = json!(prov.ocr_engine);
            payload["ocr_models"] = json!(prov.ocr_models);
        }
        let seq = guard
            .append(req_meta.session_id, "source.ingested", EVENT_SCHEMA_VERSION, &payload)
            .map_err(|e| format!("source.ingested append failed: {e}"))?;
        (blob_hash, seq)
    };

    if let Some(lib) = library {
        if outcome.document.status == PdfExtractStatus::Ok {
            let paper_key = PaperKey::from_file_bytes(raw_bytes);
            cache_pdf_ocr_in_library(lib, &paper_key, None, raw_bytes, &outcome, now_ms())?;
        }
    }

    let pages_remaining_none = outcome.document.page_map.pages_needing_ocr();
    let (ocr_engine, ocr_models, pages_ocr_applied) = match outcome.provenance {
        Some(prov) => (Some(prov.ocr_engine), prov.ocr_models, prov.pages_ocr_applied),
        None => (None, Vec::new(), Vec::new()),
    };
    let source = IngestedSource {
        source_id,
        origin: req_meta.origin.to_string(),
        title,
        format: "pdf".into(),
        status: status_str,
        raw_blob,
        event_seq,
        bytes: raw_bytes.len(),
        primary_path,
        assembled_path: None,
        files: written_files,
        skipped_binary_entries: Vec::new(),
        page_count: Some(outcome.document.page_count),
        warning: outcome.document.warning,
        excerpt: take_chars(&outcome.document.text, MAX_EXCERPT_CHARS),
    };

    Ok(OcrIngestedSource {
        source,
        ocr_available: outcome.ocr_available,
        ocr_status: outcome.ocr_status,
        ocr_message: outcome.ocr_message,
        ocr_engine,
        ocr_models,
        pages_ocr_applied,
        pages_remaining_none,
        page_map: outcome.document.page_map,
    })
}

fn enforce_user_pdf_cap(file_name: &str, len: usize) -> Result<(), String> {
    if len > MAX_USER_PDF_BYTES {
        return Err(format!(
            "`{file_name}` ({len} bytes) exceeds {} MiB user PDF ingestion limit",
            MAX_USER_PDF_BYTES / (1024 * 1024)
        ));
    }
    Ok(())
}

fn build_source_id(hint: &str, identity: &Value) -> String {
    let base = hint.rsplit(['/', '\\']).next().unwrap_or(hint);
    let stem = base
        .strip_suffix(".tar.gz")
        .or_else(|| base.strip_suffix(".TAR.GZ"))
        .unwrap_or_else(|| base.split('.').next().unwrap_or(base));
    let mut slug = String::new();
    for ch in stem.chars() {
        if slug.len() >= 24 { break; }
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if matches!(ch, '-' | '_') && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let clean = slug.trim_matches('-');
    let prefix = if clean.is_empty() { "doc" } else { clean };
    let digest = ledger::sha256_hex(identity.to_string().as_bytes());
    let short_hash = digest.get(..16).unwrap_or(&digest);
    format!("{prefix}-{short_hash}")
}

fn sanitize_leaf_name(file_name: &str, fallback_ext: &str) -> String {
    let leaf = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    sanitize_rel_components(leaf, false)
        .map(|(_, norm)| norm)
        .unwrap_or_else(|_| format!("document.{fallback_ext}"))
}

fn arxiv_cover_stamp(page_lines: &[(usize, usize, usize)], content: &str) -> Option<String> {
    let (_, first, last) = page_lines.iter().find(|(page, _, _)| *page == 1)?;
    let cover = content
        .lines()
        .skip(first.saturating_sub(1))
        .take(last.saturating_sub(*first) + 1)
        .collect::<Vec<_>>()
        .join(" ");
    static STAMP: OnceLock<Option<Regex>> = OnceLock::new();
    let pattern = STAMP
        .get_or_init(|| Regex::new(r"(?i)arxiv\s*:\s*([0-9]{4}\.[0-9]{4,5}v[0-9]+)\s*\[[a-z][a-z0-9.\-]+\]").ok())
        .as_ref()?;
    pattern.captures(&cover).and_then(|m| m.get(1)).map(|m| m.as_str().to_ascii_lowercase())
}

fn take_chars(s: &str, max_chars: usize) -> String { s.chars().take(max_chars).collect() }
