//! ArXiv LaTeXML HTML fallback (`/html/<id>`) during Investigate paper-open (`investigate.md` V18).
//!
//! Runs after `LibraryLookup` and `EprintTex` (TeX e-print 403/404/410) and before
//! `PdfFallback`, routing every arXiv HTTP request through `fetch_response_with_gate` with
//! `Service::Arxiv` (1 req / 3s). Rejects 404 or non-LaTeXML HTML cleanly without failing
//! the job so PDF fallback still runs.

use crate::investigation_open::{update_slot, OpenJobSlot, OpenStep, OpenWorkerConfig, NEEDS_FILE_HINT};
use ledger::{sha256_hex, FulltextCache, FulltextEntry, Ledger};
use research_tools::archive::{extract_gzip_or_tar_gz, ArchivePayload};
use research_tools::http_client::{fetch_response_with_gate, MAX_EPRINT_BYTES};
use research_tools::ingest::{ingest_bytes, IngestRequest, IngestedSource};
use research_tools::net_rate::{NetRateGate, Service};
use research_tools::paper_fetch::{
    fetch_with_progress, FetchOutcome, FetchStage, FetchWant, PaperMetadata, SourceOrigin,
};
use research_tools::paper_key::{split_arxiv_id, PaperKeyKind, PaperRef};
use research_tools::pdf::MAX_PDF_BYTES;
use research_tools::sandbox::WorkspaceSandbox;
use research_tools::skeleton::{is_latexml_html_document, MAX_HTML_BYTES};
use std::sync::{Arc, Mutex};

const MAX_PLAIN_TEX_BYTES: usize = 16 * 1024 * 1024;

pub(crate) struct ResolvedOpenSource {
    pub(crate) paper: PaperRef,
    pub(crate) source_origin: String,
    pub(crate) file_name: String,
    pub(crate) origin_url: String,
    pub(crate) raw_bytes: Vec<u8>,
    pub(crate) title: Option<String>,
}

impl ResolvedOpenSource {
    pub(crate) fn ingest_into_session(
        &self,
        sandbox: &WorkspaceSandbox,
        ledger: &Mutex<Ledger>,
        session_id: &str,
    ) -> Result<IngestedSource, String> {
        ingest_bytes(
            sandbox,
            ledger,
            &IngestRequest {
                session_id,
                file_name: &self.file_name,
                origin: &self.origin_url,
                raw_bytes: &self.raw_bytes,
                source_id_hint: Some(&self.file_name),
                section_filter: None,
                title: self.title.as_deref(),
            },
        )
    }
}

/// Resolves the paper source with an arXiv `/html/<target_id>` step between TeX e-print
/// and PDF fallback, preserving cache-first zero-network behavior and non-arXiv delegation.
pub(crate) fn fetch_source_with_html_fallback(
    cfg: &OpenWorkerConfig,
    gate: &NetRateGate,
    metadata: Option<&PaperMetadata>,
    slot: &Arc<OpenJobSlot>,
) -> Result<Option<ResolvedOpenSource>, String> {
    let discovered_title = metadata
        .map(|m| m.title.clone())
        .filter(|t| !t.trim().is_empty());

    // 1. Local fulltext blob cache first: zero network requests on hit.
    update_slot(slot, |s| {
        s.advance(OpenStep::LibraryLookup, "running", "Checking local paper library cache.")
    });
    let cache = FulltextCache::open(&cfg.env.data_dir)
        .map_err(|e| format!("cannot open fulltext cache: {e}"))?;
    if let Some(hit) = lookup_cache(&cache, &cfg.paper, discovered_title.clone())? {
        return Ok(Some(hit));
    }
    if let Some(meta) = metadata {
        if meta.paper.key != cfg.paper.key {
            if let Some(hit) = lookup_cache(&cache, &meta.paper, discovered_title.clone())? {
                return Ok(Some(hit));
            }
        }
    }

    // Determine arXiv ID + resolved PaperRef (direct input or Step 1 Metadata resolution).
    let arxiv_identity = resolve_arxiv_identity(&cfg.paper, metadata);
    let Some((resolved_paper, base_id)) = arxiv_identity else {
        return delegate_non_arxiv_fetch(cfg, gate, discovered_title, slot);
    };

    let target_id = match resolved_paper.version.as_deref() {
        Some(v) if v.starts_with('v') => format!("{base_id}{v}"),
        _ => base_id.clone(),
    };
    let slug = safe_id_slug(&target_id);
    let eprint_base = cfg.env.network_endpoints.arxiv_eprint_base.trim_end_matches('/');

    // 2. Try arXiv TeX e-print first.
    update_slot(slot, |s| s.advance(OpenStep::EprintTex, "running", "Downloading arXiv TeX source."));
    let eprint_url = format!("{eprint_base}/e-print/{target_id}");
    let eprint_resp = fetch_response_with_gate(
        &eprint_url,
        &cfg.env.network_endpoints,
        MAX_EPRINT_BYTES,
        Some(Service::Arxiv),
        gate,
    )?;
    if eprint_resp.status == 200 {
        let src = validate_and_cache_eprint(
            &cache,
            &cfg.paper,
            &resolved_paper,
            &slug,
            &eprint_resp.final_url,
            eprint_resp.content_type.as_deref(),
            eprint_resp.body,
            discovered_title,
        )?;
        return Ok(Some(src));
    }
    if !matches!(eprint_resp.status, 403 | 404 | 410) {
        return Err(format!("HTTP {} fetching arXiv e-print `{eprint_url}`", eprint_resp.status));
    }

    // 3. Try arXiv LaTeXML HTML fallback before PDF fallback (V18, Service::Arxiv gated).
    update_slot(slot, |s| {
        s.advance(OpenStep::HtmlFallback, "running", "Trying arXiv LaTeXML HTML fallback.")
    });
    let html_url = format!("{eprint_base}/html/{target_id}");
    if let Ok(resp) = fetch_response_with_gate(
        &html_url,
        &cfg.env.network_endpoints,
        MAX_HTML_BYTES,
        Some(Service::Arxiv),
        gate,
    ) {
        if resp.status == 200 && resp.body.len() <= MAX_HTML_BYTES && !resp.body.contains(&0u8) {
            if let Ok(html_text) = std::str::from_utf8(&resp.body) {
                if is_latexml_html_document(html_text) {
                    return Ok(Some(ResolvedOpenSource {
                        paper: resolved_paper,
                        source_origin: "arxiv_html".to_string(),
                        file_name: format!("arxiv_{slug}.html"),
                        origin_url: resp.final_url,
                        raw_bytes: resp.body,
                        title: discovered_title,
                    }));
                }
            }
        }
    }

    // 4. Fall back to arXiv PDF.
    update_slot(slot, |s| s.advance(OpenStep::PdfFallback, "running", "Falling back to arXiv PDF."));
    let pdf_url = format!("{eprint_base}/pdf/{target_id}.pdf");
    let pdf_resp = fetch_response_with_gate(
        &pdf_url,
        &cfg.env.network_endpoints,
        MAX_PDF_BYTES,
        Some(Service::Arxiv),
        gate,
    )?;
    if pdf_resp.status == 200 {
        if pdf_resp.body.is_empty()
            || pdf_resp.body.len() > MAX_PDF_BYTES
            || looks_like_html(pdf_resp.content_type.as_deref(), &pdf_resp.body)
            || !has_pdf_magic(&pdf_resp.body)
        {
            return Err("malformed arXiv PDF download".to_string());
        }
        cache_put(&cache, &cfg.paper, &resolved_paper, "pdf", SourceOrigin::ArxivPdf.as_str(), &pdf_resp.body)?;
        return Ok(Some(ResolvedOpenSource {
            paper: resolved_paper,
            source_origin: SourceOrigin::ArxivPdf.as_str().to_string(),
            file_name: format!("arxiv_{slug}.pdf"),
            origin_url: pdf_resp.final_url,
            raw_bytes: pdf_resp.body,
            title: discovered_title,
        }));
    }
    if pdf_resp.status == 404 {
        let why = "neither arXiv e-print, HTML, nor PDF was found; drop or attach the file".to_string();
        update_slot(slot, |s| {
            if s.metadata.is_none() {
                s.paper = Some(resolved_paper);
            }
            s.needs_file_reason = Some(why.clone());
            s.advance(OpenStep::NeedsFile, "needs_file", why);
        });
        return Ok(None);
    }
    Err(format!("HTTP {} fetching arXiv PDF `{pdf_url}`", pdf_resp.status))
}

fn resolve_arxiv_identity(
    input_paper: &PaperRef,
    metadata: Option<&PaperMetadata>,
) -> Option<(PaperRef, String)> {
    if input_paper.key.kind() == PaperKeyKind::Arxiv {
        let ver = input_paper
            .version
            .clone()
            .or_else(|| metadata.and_then(|m| m.paper.version.clone()));
        let mut resolved = input_paper.clone();
        resolved.version = ver;
        let base = input_paper.key.value().to_string();
        return Some((resolved, base));
    }
    let meta = metadata?;
    if meta.paper.key.kind() == PaperKeyKind::Arxiv {
        let base = meta.paper.key.value().to_string();
        return Some((meta.paper.clone(), base));
    }
    if let Some(raw_arxiv) = meta.paper.ids.arxiv.as_deref() {
        if let Ok((base, ver)) = split_arxiv_id(raw_arxiv) {
            let mut resolved = meta.paper.clone();
            if resolved.version.is_none() {
                resolved.version = ver;
            }
            return Some((resolved, base));
        }
    }
    None
}

fn delegate_non_arxiv_fetch(
    cfg: &OpenWorkerConfig,
    gate: &NetRateGate,
    discovered_title: Option<String>,
    slot: &Arc<OpenJobSlot>,
) -> Result<Option<ResolvedOpenSource>, String> {
    let slot_cb = Arc::clone(slot);
    let outcome = fetch_with_progress(&cfg.env, &cfg.paper, FetchWant::Source, gate, &mut |fs| {
        let next = match fs {
            FetchStage::CheckingCache => Some((OpenStep::LibraryLookup, "Checking local paper library cache.")),
            FetchStage::FetchingEprint => Some((OpenStep::EprintTex, "Downloading arXiv TeX source.")),
            FetchStage::FetchingPdf => Some((OpenStep::PdfFallback, "Falling back to arXiv PDF.")),
            _ => None,
        };
        if let Some((step, msg)) = next {
            update_slot(&slot_cb, |s| s.advance(step, "running", msg));
        }
    })?;
    match outcome {
        FetchOutcome::Source(src) => Ok(Some(ResolvedOpenSource {
            paper: src.paper,
            source_origin: src.origin.as_str().to_string(),
            file_name: src.file_name,
            origin_url: src.origin_url,
            raw_bytes: src.raw_bytes,
            title: src.title.or(discovered_title),
        })),
        FetchOutcome::NeedsFile { paper, reason } => {
            let why = if reason.trim().is_empty() { NEEDS_FILE_HINT.to_string() } else { reason };
            update_slot(slot, |s| {
                if s.metadata.is_none() {
                    s.paper = Some(paper);
                }
                s.needs_file_reason = Some(why.clone());
                s.advance(OpenStep::NeedsFile, "needs_file", why);
            });
            Ok(None)
        }
        other => Err(format!("unexpected fetch outcome: {}", other.status_str())),
    }
}

fn lookup_cache(
    cache: &FulltextCache,
    paper: &PaperRef,
    title: Option<String>,
) -> Result<Option<ResolvedOpenSource>, String> {
    let Some((entry, bytes)) = cache
        .get_bytes(paper.key.as_str(), paper.version.as_deref())
        .map_err(|e| format!("fulltext cache read failed: {e}"))?
    else {
        return Ok(None);
    };
    let mut resolved = paper.clone();
    if resolved.version.is_none() {
        resolved.version = entry.version.clone();
    }
    Ok(Some(ResolvedOpenSource {
        paper: resolved.clone(),
        source_origin: SourceOrigin::Library.as_str().to_string(),
        file_name: cached_file_name(&resolved, &entry),
        origin_url: format!("library:{}", entry.raw_blob),
        raw_bytes: bytes,
        title,
    }))
}

fn cached_file_name(paper: &PaperRef, entry: &FulltextEntry) -> String {
    let slug = safe_id_slug(paper.key.value());
    let ver = entry.version.as_deref().unwrap_or("");
    let ext = match entry.format.as_str() {
        "pdf" => "pdf",
        "tex" => "tex",
        "tar.gz" => "tar.gz",
        "zip" => "zip",
        _ => "gz",
    };
    format!("paper_{slug}{ver}.{ext}")
}

#[allow(clippy::too_many_arguments)]
fn validate_and_cache_eprint(
    cache: &FulltextCache,
    orig_paper: &PaperRef,
    resolved: &PaperRef,
    slug: &str,
    final_url: &str,
    content_type: Option<&str>,
    body: Vec<u8>,
    title: Option<String>,
) -> Result<ResolvedOpenSource, String> {
    if body.is_empty() || body.len() > MAX_EPRINT_BYTES {
        return Err("malformed arXiv e-print download".to_string());
    }
    if looks_like_html(content_type, &body) {
        return Err("malformed arXiv e-print download: received HTML instead of paper source".into());
    }
    if has_pdf_magic(&body) {
        cache_put(cache, orig_paper, resolved, "pdf", SourceOrigin::ArxivPdf.as_str(), &body)?;
        return Ok(ResolvedOpenSource {
            paper: resolved.clone(),
            source_origin: SourceOrigin::ArxivPdf.as_str().to_string(),
            file_name: format!("arxiv_{slug}.pdf"),
            origin_url: final_url.to_string(),
            raw_bytes: body,
            title,
        });
    }
    if body.starts_with(&[0x1f, 0x8b]) {
        let hint = format!("arxiv_{slug}.gz");
        let payload = extract_gzip_or_tar_gz(&body, &hint, false)
            .map_err(|e| format!("malformed arXiv e-print archive: {e}"))?;
        let (fmt, origin, file_name, store_bytes) = match payload {
            ArchivePayload::Files(entries) => {
                if entries.is_empty() {
                    return Err("malformed arXiv e-print archive: contains 0 files".into());
                }
                ("tar.gz", SourceOrigin::ArxivEprint, hint, body)
            }
            ArchivePayload::SingleGzipText { .. } => ("gz", SourceOrigin::ArxivEprint, hint, body),
            ArchivePayload::RawPdf(pdf_bytes) => {
                if !has_pdf_magic(&pdf_bytes) {
                    return Err("malformed arXiv raw PDF inside gzip".into());
                }
                ("pdf", SourceOrigin::ArxivPdf, format!("arxiv_{slug}.pdf"), pdf_bytes)
            }
        };
        cache_put(cache, orig_paper, resolved, fmt, origin.as_str(), &store_bytes)?;
        return Ok(ResolvedOpenSource {
            paper: resolved.clone(),
            source_origin: origin.as_str().to_string(),
            file_name,
            origin_url: final_url.to_string(),
            raw_bytes: store_bytes,
            title,
        });
    }
    if body.len() <= MAX_PLAIN_TEX_BYTES && !body.contains(&0u8) {
        if let Ok(text) = std::str::from_utf8(&body) {
            if text.contains("\\documentclass")
                || text.contains("\\begin{document}")
                || text.contains("\\section")
                || text.contains("\\documentstyle")
            {
                cache_put(cache, orig_paper, resolved, "tex", SourceOrigin::ArxivEprint.as_str(), &body)?;
                return Ok(ResolvedOpenSource {
                    paper: resolved.clone(),
                    source_origin: SourceOrigin::ArxivEprint.as_str().to_string(),
                    file_name: format!("arxiv_{slug}.tex"),
                    origin_url: final_url.to_string(),
                    raw_bytes: body,
                    title,
                });
            }
        }
    }
    Err("malformed arXiv e-print download: unrecognized or corrupt payload".into())
}

fn cache_put(
    cache: &FulltextCache,
    orig: &PaperRef,
    resolved: &PaperRef,
    fmt: &str,
    origin: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let entry = cache
        .put_bytes(resolved.key.as_str(), resolved.version.as_deref(), fmt, origin, bytes)
        .map_err(|e| format!("fulltext cache write failed: {e}"))?;
    if orig.key != resolved.key {
        let _ = cache.index_existing_blob(
            orig.key.as_str(),
            orig.version.as_deref(),
            &entry.raw_blob,
            fmt,
            entry.bytes,
            origin,
        );
    }
    let _ = sha256_hex(bytes);
    Ok(())
}

fn safe_id_slug(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .collect()
}

fn has_pdf_magic(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(1024)].windows(5).any(|w| w == b"%PDF-")
}

fn looks_like_html(content_type: Option<&str>, bytes: &[u8]) -> bool {
    if content_type.is_some_and(|ct| ct.to_ascii_lowercase().contains("text/html")) {
        return true;
    }
    let head_bytes: Vec<u8> = bytes.iter().skip_while(|b| b.is_ascii_whitespace()).take(32).copied().collect();
    let head = String::from_utf8_lossy(&head_bytes).to_ascii_lowercase();
    head.starts_with("<!doctype html") || head.starts_with("<html")
}
