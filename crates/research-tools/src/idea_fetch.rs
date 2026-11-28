//! Thin Explore paper fetcher, F10 `paper_text` PDF cache reuse, and Rule C9 ingestion
//! (package X9, `explore.md` 4.1, 4.3, 5.2, 6 Rules C8, C9, P3, P5, P6).
//!
//! Resolution order per paper:
//! 1. F10/F3 `library` fetch cache (`FulltextCache` / `Library` non-`user_library` blob).
//! 2. F13a `user_library` owned file inside a registered source root (always copied into the
//!    session via `ingest_bytes` / `ingest_pdf_pages`, never quote-checked in place — Rule C9).
//! 3. F3 `paper_fetch::fetch_with_gate` (`arxiv_eprint`, then `arxiv_pdf`, one at a time through
//!    F3's process-wide [`NetRateGate`]).

#[path = "idea_fetch_cache.rs"]
mod cache;

pub use cache::{lookup_cached_pdf_pages, reconstruct_pdf_document_pages};

use crate::ingest::IngestRequest;
use crate::library::{Library, ORIGIN_USER_LIBRARY};
use crate::net_rate::{global_gate, NetRateGate};
use crate::paper_fetch::{
    check_fetch_budget, fetch_with_gate, parse_paper_input, FetchOutcome, FetchWant, SourceOrigin,
};
use crate::paper_key::{PaperIds, PaperKey, PaperRef};
use crate::prior_work::TextBasis;
use crate::sandbox::{SandboxRoot, WorkspaceSandbox};
use crate::ResearchEnv;
use ledger::{Event, Ledger};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tools::{CallCtx, ToolHandler};

/// Default per-session paper fetch ceiling when `idea.started` is absent (Standard preset).
pub const DEFAULT_IDEA_MAX_PAPERS: usize = 6;
/// Maximum keys accepted in one `idea_fetch_papers` call (`explore.md` 5.2).
pub const MAX_FETCH_KEYS: usize = 8;
/// Exact user-facing reason when a paper is neither on arXiv nor in the user's library (`explore.md` 3).
pub const UNAVAILABLE_PAPER_REASON: &str =
    "unavailable: not on arXiv and not in your library; attach PDF";
/// Exact user-facing label for scanned/OCR papers (`explore.md` 3, Rule C8).
pub const SCAN_OCR_ROW_LABEL: &str = "scan: quotes are from OCR text";

/// Callback invoked during `idea_fetch_papers` for live SSE row/page progress:
/// `(paper_key, row_state, label, optional (page, total_pages))`.
pub type FetchProgressSink = Arc<dyn Fn(&str, &str, &str, Option<(usize, usize)>) + Send + Sync>;

/// Result of fetching and ingesting one paper into an Explore session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchedPaperRecord {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reading_path: Option<String>,
    pub origin: String,
    pub text_basis: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub total_lines: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_blob: Option<String>,
    pub pdf_text_cached: bool,
}

/// Optional progress callbacks for reading-queue streaming (SSE).
pub struct FetchProgressCallbacks<'a> {
    pub cancel: Option<&'a AtomicBool>,
    pub on_row_state: &'a mut dyn FnMut(&str, &str),
    pub on_pdf_page: &'a mut dyn FnMut(usize, usize),
}

/// Derive the effective `max_papers` fetch budget from `idea.started` + `idea.budget_raised`.
pub fn effective_max_papers(events: &[Event]) -> usize {
    let mut max_papers = DEFAULT_IDEA_MAX_PAPERS;
    for ev in events {
        if ev.kind == "idea.started" {
            if let Some(p) = ev.payload.get("preset").and_then(Value::as_str) {
                max_papers = match p.trim().to_ascii_lowercase().as_str() {
                    "quick" => 0,
                    "thorough" => 12,
                    _ => DEFAULT_IDEA_MAX_PAPERS,
                };
            }
            if let Some(v) = ev
                .payload
                .get("budget")
                .and_then(|b| b.get("max_papers"))
                .and_then(Value::as_u64)
            {
                max_papers = v as usize;
            }
        } else if ev.kind == "idea.budget_raised" {
            let extra = ev.payload.get("extra_papers").and_then(Value::as_u64).unwrap_or(0) as usize;
            max_papers = max_papers.saturating_add(extra);
        }
    }
    max_papers
}

/// Build the real X9 `idea_fetch_papers` [`ToolHandler`] using F3's process-wide [`global_gate`].
pub fn idea_fetch_papers_handler(env: Arc<ResearchEnv>) -> Arc<dyn ToolHandler> {
    idea_fetch_papers_handler_with_gate_and_progress(env, None, None, None)
}

/// Build an `idea_fetch_papers` [`ToolHandler`] with an explicit [`NetRateGate`] for tests.
pub fn idea_fetch_papers_handler_with_gate(
    env: Arc<ResearchEnv>,
    gate: Option<Arc<NetRateGate>>,
) -> Arc<dyn ToolHandler> {
    idea_fetch_papers_handler_with_gate_and_progress(env, gate, None, None)
}

/// Build an `idea_fetch_papers` [`ToolHandler`] with optional [`NetRateGate`], progress sink,
/// and cooperative cancellation flag.
pub fn idea_fetch_papers_handler_with_gate_and_progress(
    env: Arc<ResearchEnv>,
    gate: Option<Arc<NetRateGate>>,
    progress_sink: Option<FetchProgressSink>,
    cancel_flag: Option<Arc<AtomicBool>>,
) -> Arc<dyn ToolHandler> {
    Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
        let keys = parse_fetch_keys(args)?;
        let active_gate: &NetRateGate = match gate.as_deref() {
            Some(g) => g,
            None => global_gate(),
        };
        let events = {
            let guard = ctx.ledger.lock().map_err(|_| "ledger lock poisoned".to_string())?;
            guard.read_session(ctx.session).map_err(|e| e.to_string())?
        };
        let max_papers = effective_max_papers(&events);
        let mut results = Vec::with_capacity(keys.len());
        let mut fetched = 0usize;
        let mut unavailable = 0usize;
        let mut stopped_at_cap = false;

        for key in keys {
            let cur_events = {
                let guard = ctx.ledger.lock().map_err(|_| "ledger lock poisoned".to_string())?;
                guard.read_session(ctx.session).map_err(|e| e.to_string())?
            };
            let key_for_state = key.to_string();
            let sink_state = progress_sink.clone();
            let mut on_row_state = move |st: &str, lbl: &str| {
                if let Some(sink) = &sink_state {
                    sink(&key_for_state, st, lbl, None);
                }
            };
            let key_for_page = key.to_string();
            let sink_page = progress_sink.clone();
            let mut on_pdf_page = move |page: usize, total: usize| {
                if let Some(sink) = &sink_page {
                    let lbl = format!("extracting text (page {page} of {total})");
                    sink(&key_for_page, "extracting", &lbl, Some((page, total)));
                }
            };
            let mut progress = FetchProgressCallbacks {
                cancel: cancel_flag.as_deref(),
                on_row_state: &mut on_row_state,
                on_pdf_page: &mut on_pdf_page,
            };
            let rec = match fetch_and_ingest_one_paper(
                &env,
                ctx.ledger,
                ctx.session,
                &cur_events,
                &key,
                max_papers,
                active_gate,
                &mut progress,
            ) {
                Ok(r) => r,
                Err(err) => failed_record(&key, "failed", &err),
            };
            match rec.status.as_str() {
                "ready" => fetched += 1,
                "unavailable" => unavailable += 1,
                "stopped_at_cap" => stopped_at_cap = true,
                _ => {}
            }
            results.push(rec);
            if stopped_at_cap {
                break;
            }
        }

        Ok(json!({
            "papers": results,
            "fetched": fetched,
            "unavailable": unavailable,
            "stopped_at_cap": stopped_at_cap,
        }))
    })
}

fn parse_fetch_keys(args: &Value) -> Result<Vec<PaperKey>, String> {
    let obj = args.as_object().ok_or("`idea_fetch_papers` arguments must be an object")?;
    for k in obj.keys() {
        if k != "keys" {
            return Err(format!("`idea_fetch_papers` has unknown field `{k}`"));
        }
    }
    let arr = obj
        .get("keys")
        .and_then(Value::as_array)
        .ok_or("`keys` must be a non-empty array")?;
    if arr.is_empty() || arr.len() > MAX_FETCH_KEYS {
        return Err(format!("`keys` must contain 1..={MAX_FETCH_KEYS} items (got {})", arr.len()));
    }
    let mut out = Vec::with_capacity(arr.len());
    let mut seen = BTreeSet::new();
    for (idx, item) in arr.iter().enumerate() {
        let raw = item
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.chars().count() <= 128)
            .ok_or_else(|| format!("keys[{idx}] must be a non-empty string (<= 128 chars)"))?;
        let key = PaperKey::parse(raw)
            .or_else(|_| parse_paper_input(raw).map(|r| r.key))
            .map_err(|e| format!("keys[{idx}] `{raw}` is not a valid PaperKey: {e}"))?;
        if seen.insert(key.clone()) {
            out.push(key);
        }
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn fetch_and_ingest_one_paper(
    env: &ResearchEnv,
    ledger: &Mutex<Ledger>,
    session_id: &str,
    session_events: &[Event],
    key: &PaperKey,
    max_papers: usize,
    gate: &NetRateGate,
    progress: &mut FetchProgressCallbacks<'_>,
) -> Result<FetchedPaperRecord, String> {
    if progress.cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
        return Ok(failed_record(key, "cancelled", "Cancelled before fetching"));
    }
    if max_papers == 0 {
        return Ok(stopped_at_cap_record(
            key,
            "Quick preset allows 0 papers read (max_papers = 0); raise the limit to read papers",
        ));
    }

    if let Some(existing) = find_existing_session_paper(env, session_id, session_events, key) {
        return Ok(existing);
    }

    let sandbox = env.sandbox();
    let paper_ref = resolve_paper_ref_from_context(&env.data_dir, session_events, key);

    // 1. Check F10/F3 fetch cache (`library`) first — zero network requests on hit.
    if let Some(hit) = cache::lookup_library_cache(&env.data_dir, &paper_ref)? {
        if let Err(reason) = check_fetch_budget(session_events, max_papers) {
            return Ok(stopped_at_cap_record(key, &reason));
        }
        (progress.on_row_state)("from_library", "from your library");
        let file_name = build_ingest_filename(key, hit.version.as_deref(), &hit.format);
        let origin_url = format!("library:{}", hit.raw_blob);
        return ingest_and_summarize(
            env, &sandbox, ledger, session_id, key, hit.version, "library", &file_name,
            &origin_url, &hit.format, &hit.bytes, hit.title.as_deref(), false, progress,
        );
    }

    // 2. Check F13a `user_library` owned file inside registered source roots (Rule C9 ingest).
    if let Some(owned) = cache::lookup_user_library_file(
        &env.data_dir,
        key,
        paper_ref.version.as_deref(),
    )? {
        (progress.on_row_state)("from_library", "from your library");
        let origin_url = format!("user_library:{key}");
        return ingest_and_summarize(
            env, &sandbox, ledger, session_id, key, owned.version, ORIGIN_USER_LIBRARY,
            &owned.file_name, &origin_url, &owned.format, &owned.bytes, owned.title.as_deref(),
            true, progress,
        );
    }

    // 3. Enforce F3 fetch budget before any external arXiv request (Rule P3).
    if let Err(reason) = check_fetch_budget(session_events, max_papers) {
        return Ok(stopped_at_cap_record(key, &reason));
    }

    (progress.on_row_state)("fetching", "fetching");
    match fetch_with_gate(env, &paper_ref, FetchWant::Source, gate) {
        Ok(FetchOutcome::Source(fetched)) => {
            let origin_str = match fetched.origin {
                SourceOrigin::Library => "library",
                SourceOrigin::ArxivEprint => "arxiv_eprint",
                SourceOrigin::ArxivPdf => "arxiv_pdf",
            };
            ingest_and_summarize(
                env, &sandbox, ledger, session_id, key, fetched.paper.version.clone(),
                origin_str, &fetched.file_name, &fetched.origin_url, &fetched.format,
                &fetched.raw_bytes, fetched.title.as_deref(), false, progress,
            )
        }
        Ok(FetchOutcome::NeedsFile { .. }) => Ok(FetchedPaperRecord {
            key: key.to_string(),
            version: paper_ref.version,
            source_id: None, source_seq: None, reading_path: None,
            origin: "none".into(), text_basis: "none".into(),
            status: "unavailable".into(), reason: Some(UNAVAILABLE_PAPER_REASON.to_string()),
            total_lines: 0, raw_blob: None, pdf_text_cached: false,
        }),
        Ok(_) => Ok(failed_record(key, "failed", "unexpected non-source fetch outcome")),
        Err(err) => Ok(failed_record(key, "failed", &err)),
    }
}

/// Ingest user-uploaded PDF/TeX bytes for `key` into `session_id` (Attach PDF flow; Rule C9).
#[allow(clippy::too_many_arguments)]
pub fn ingest_uploaded_paper_bytes(
    env: &ResearchEnv,
    ledger: &Mutex<Ledger>,
    session_id: &str,
    key: &PaperKey,
    file_name: &str,
    raw_bytes: &[u8],
    title: Option<&str>,
    progress: &mut FetchProgressCallbacks<'_>,
) -> Result<FetchedPaperRecord, String> {
    let sandbox = env.sandbox();
    let lower = file_name.to_ascii_lowercase();
    let format = if cache::is_pdf_bytes(file_name, "", raw_bytes) {
        "pdf"
    } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        "tar.gz"
    } else if lower.ends_with(".zip") {
        "zip"
    } else {
        "tex"
    };
    let origin_url = format!("upload:{file_name}");
    ingest_and_summarize(
        env, &sandbox, ledger, session_id, key, None, "upload", file_name,
        &origin_url, format, raw_bytes, title, true, progress,
    )
}

#[allow(clippy::too_many_arguments)]
fn ingest_and_summarize(
    env: &ResearchEnv,
    sandbox: &WorkspaceSandbox,
    ledger: &Mutex<Ledger>,
    session_id: &str,
    key: &PaperKey,
    version: Option<String>,
    origin: &str,
    file_name: &str,
    origin_url: &str,
    format: &str,
    raw_bytes: &[u8],
    title: Option<&str>,
    user_supplied_pdf: bool,
    progress: &mut FetchProgressCallbacks<'_>,
) -> Result<FetchedPaperRecord, String> {
    let req = IngestRequest {
        session_id, file_name, origin: origin_url, raw_bytes,
        source_id_hint: Some(file_name), section_filter: None, title,
    };
    let (ingested, pdf_text_cached) = cache::ingest_with_pdf_cache(
        &env.data_dir, sandbox, ledger, key, version.as_deref(), &req, format,
        user_supplied_pdf, progress.cancel, progress.on_pdf_page,
    )?;

    let receipt = {
        let guard = ledger.lock().map_err(|_| "ledger lock poisoned".to_string())?;
        guard
            .read_session(session_id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|e| e.seq == ingested.event_seq)
    };
    let text_basis = derive_text_basis_from_receipt(receipt.as_ref(), &ingested.format);
    let reading_path = ingested.assembled_path.clone().or_else(|| ingested.primary_path.clone());
    let total_lines = reading_path
        .as_deref()
        .and_then(|p| count_source_lines(sandbox, session_id, p).ok())
        .unwrap_or(0);

    Ok(FetchedPaperRecord {
        key: key.to_string(),
        version,
        source_id: Some(ingested.source_id),
        source_seq: Some(ingested.event_seq),
        reading_path,
        origin: origin.to_string(),
        text_basis: text_basis_str(text_basis).to_string(),
        status: "ready".into(),
        reason: ingested.warning,
        total_lines,
        raw_blob: Some(ingested.raw_blob),
        pdf_text_cached,
    })
}

fn resolve_paper_ref_from_context(data_dir: &Path, events: &[Event], key: &PaperKey) -> PaperRef {
    for ev in events.iter().rev() {
        if ev.kind != "idea.candidates" {
            continue;
        }
        if let Some(arr) = ev.payload.get("candidates").and_then(Value::as_array) {
            for c in arr {
                if c.get("key").and_then(Value::as_str) == Some(key.as_str()) {
                    let ids: PaperIds = c
                        .get("ids")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                        .unwrap_or_default();
                    return PaperRef { key: key.clone(), version: None, ids };
                }
            }
        }
    }
    if let Ok(lib) = Library::open(data_dir) {
        if let Ok(Some(detail)) = lib.get_paper(key) {
            return detail.paper.paper_ref(None);
        }
    }
    PaperRef { key: key.clone(), version: None, ids: PaperIds::default() }
}

fn find_existing_session_paper(
    env: &ResearchEnv,
    session_id: &str,
    events: &[Event],
    key: &PaperKey,
) -> Option<FetchedPaperRecord> {
    let rev_seq = events.iter().rev().find(|e| e.kind == "idea.stated").map_or(0, |e| e.seq);
    let ev = events.iter().rev().find(|e| {
        e.seq >= rev_seq
            && e.kind == "idea.paper"
            && e.payload.get("key").and_then(Value::as_str) == Some(key.as_str())
            && e.payload.get("source_id").and_then(Value::as_str).is_some_and(|s| !s.is_empty())
    })?;
    let source_id = ev.payload.get("source_id")?.as_str()?.to_string();
    let src_ev = events.iter().rev().find(|e| {
        e.kind == "source.ingested"
            && e.payload.get("source_id").and_then(Value::as_str) == Some(source_id.as_str())
    })?;
    let reading_path = src_ev
        .payload
        .get("assembled_path")
        .or_else(|| src_ev.payload.get("primary_path"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let total_lines = ev
        .payload
        .get("total_lines")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .or_else(|| {
            reading_path
                .as_deref()
                .and_then(|p| count_source_lines(&env.sandbox(), session_id, p).ok())
        })
        .unwrap_or(0);
    Some(FetchedPaperRecord {
        key: key.to_string(),
        version: ev.payload.get("version").and_then(Value::as_str).map(str::to_string),
        source_id: Some(source_id),
        source_seq: Some(src_ev.seq),
        reading_path,
        origin: ev.payload.get("origin").and_then(Value::as_str).unwrap_or("library").to_string(),
        text_basis: ev
            .payload
            .get("text_basis")
            .and_then(Value::as_str)
            .unwrap_or("tex")
            .to_string(),
        status: "ready".into(),
        reason: None,
        total_lines,
        raw_blob: src_ev.payload.get("raw_blob").and_then(Value::as_str).map(str::to_string),
        pdf_text_cached: false,
    })
}

/// Derive `PaperRead.text_basis` from a `source.ingested` ledger event.
pub fn derive_text_basis_from_receipt(receipt: Option<&Event>, format: &str) -> TextBasis {
    if let Some(ev) = receipt {
        if let Some(pages) = ev.payload.get("pdf_pages").and_then(Value::as_array) {
            if !pages.is_empty() {
                let mut has_ocr = false;
                let mut has_scan = false;
                let mut has_text = false;
                for p in pages {
                    match p.get("basis").and_then(Value::as_str) {
                        Some("ocr") => has_ocr = true,
                        Some("scan_text_layer") => has_scan = true,
                        Some("text_layer") => has_text = true,
                        _ => {}
                    }
                }
                if has_ocr { return TextBasis::Ocr; }
                if has_scan { return TextBasis::ScanTextLayer; }
                if has_text { return TextBasis::PdfText; }
                return TextBasis::ScanTextLayer;
            }
        }
        if ev.payload.get("format").and_then(Value::as_str) == Some("pdf")
            || ev.payload.get("status").and_then(Value::as_str) == Some("pdf_only")
        {
            return TextBasis::PdfText;
        }
    }
    if format.eq_ignore_ascii_case("pdf") { TextBasis::PdfText } else { TextBasis::Tex }
}

pub fn text_basis_str(basis: TextBasis) -> &'static str {
    match basis {
        TextBasis::Tex => "tex",
        TextBasis::PdfText => "pdf_text",
        TextBasis::ScanTextLayer => "scan_text_layer",
        TextBasis::Ocr => "ocr",
    }
}

pub fn count_source_lines(sandbox: &WorkspaceSandbox, session_id: &str, rel_path: &str) -> Result<usize, String> {
    let resolved = sandbox
        .resolve_existing(SandboxRoot::Sources, session_id, rel_path, false)
        .map_err(|e| e.to_string())?;
    let text = std::fs::read_to_string(&resolved.canonical_target).map_err(|e| e.to_string())?;
    Ok(text.lines().count())
}

fn build_ingest_filename(key: &PaperKey, version: Option<&str>, format: &str) -> String {
    let slug: String = key
        .value()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let ver = version.unwrap_or("");
    let ext = match format {
        "pdf" => "pdf",
        "tex" | "latex" => "tex",
        "tar.gz" | "tar_gz" | "eprint" => "tar.gz",
        "zip" => "zip",
        "gz" => "gz",
        _ => "txt",
    };
    format!("paper_{slug}{ver}.{ext}")
}

fn stopped_at_cap_record(key: &PaperKey, reason: &str) -> FetchedPaperRecord {
    FetchedPaperRecord {
        key: key.to_string(),
        version: None, source_id: None, source_seq: None, reading_path: None,
        origin: "none".into(), text_basis: "none".into(),
        status: "stopped_at_cap".into(), reason: Some(reason.to_string()),
        total_lines: 0, raw_blob: None, pdf_text_cached: false,
    }
}

fn failed_record(key: &PaperKey, status: &str, reason: &str) -> FetchedPaperRecord {
    FetchedPaperRecord {
        key: key.to_string(),
        version: None, source_id: None, source_seq: None, reading_path: None,
        origin: "none".into(), text_basis: "none".into(),
        status: status.into(), reason: Some(reason.to_string()),
        total_lines: 0, raw_blob: None, pdf_text_cached: false,
    }
}

#[cfg(test)]
#[path = "idea_fetch_tests.rs"]
mod tests;
