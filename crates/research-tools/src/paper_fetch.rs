//! Shared paper fetcher, job progress wrapper, and session fetch counter (package F3).
//!
//! # Approval rule for callers (`00-overview.md` section 2.4)
//!
//! A network request runs without an approval card **only** when the user's own click or
//! keystroke named exactly that request (an arXiv id typed in, a citation chip clicked, a
//! query string shown in the form). Anything a model or the app composes MUST go through
//! `tools::Router::try_call` and the real approver.
//!
//! This module does not build an approval path and writes **no** session ledger events.
//! Callers must enforce the approval rule above and the session fetch budget
//! ([`fetched_sources`], [`check_fetch_budget`]) before calling [`fetch`], and bring any
//! fetched bytes into a session strictly through [`crate::ingest::ingest_bytes`] (or
//! [`FetchedSource::ingest_into_session`]), which writes that session's own
//! `source.ingested` event.

use crate::http_client::{fetch_response_with_gate, MAX_EPRINT_BYTES};
use crate::ingest::{ingest_bytes, IngestRequest, IngestedSource};
use crate::net_rate::{global_gate, NetRateGate, Service};
use crate::paper_fetch_meta::{
    arxiv_target_id, fetch_arxiv_metadata, fetch_arxiv_versions, fetch_doi_metadata,
    local_arxiv_base, resolve_to_arxiv,
};
use crate::paper_fetch_validate::{
    safe_id_slug, validate_eprint_download, validate_pdf_download, ValidatedDownload,
};
use crate::paper_key::{PaperKeyKind, PaperRef};
use crate::pdf::MAX_PDF_BYTES;
use crate::sandbox::WorkspaceSandbox;
use crate::ResearchEnv;
use ledger::{sha256_hex, Event, FulltextCache, FulltextEntry, Ledger};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::Mutex;

pub use crate::paper_fetch_job::{FetchJob, FetchJobSnapshot, FetchStage};
pub use crate::paper_fetch_meta::{parse_paper_input, PaperMetadata, PaperVersion};

/// Default per-session fetched-source limit for Investigate (`00-overview.md` 2.4).
pub const DEFAULT_INVESTIGATE_FETCH_LIMIT: usize = 8;

/// Wire status string for a paper with no fetchable arXiv source (`00-overview.md` 2.4).
pub const STATUS_NEEDS_FILE: &str = "needs_file";

/// What [`fetch`] is asked to retrieve for a [`PaperRef`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchWant {
    Metadata,
    Versions,
    Source,
}

/// Where a fetched source came from (matching `explore.md` section 4.1 `PaperRead.origin`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceOrigin {
    Library,
    ArxivEprint,
    ArxivPdf,
}

impl SourceOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Library => "library",
            Self::ArxivEprint => "arxiv_eprint",
            Self::ArxivPdf => "arxiv_pdf",
        }
    }
}

/// Validated raw bytes retrieved by [`fetch`] for `FetchWant::Source`.
///
/// Enters a session only through [`crate::ingest::ingest_bytes`] (or
/// [`Self::ingest_into_session`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedSource {
    pub paper: PaperRef,
    pub origin: SourceOrigin,
    pub file_name: String,
    pub origin_url: String,
    pub format: String,
    pub raw_blob: String,
    pub raw_bytes: Vec<u8>,
    pub title: Option<String>,
}

impl FetchedSource {
    /// Materialize and pin these fetched bytes into `session_id` via `ingest_bytes`,
    /// writing that session's own `source.ingested` event.
    pub fn ingest_into_session(
        &self,
        sandbox: &WorkspaceSandbox,
        ledger: &Mutex<Ledger>,
        session_id: &str,
        section_filter: Option<&str>,
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
                section_filter,
                title: self.title.as_deref(),
            },
        )
    }
}

/// Result of [`fetch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchOutcome {
    Metadata(PaperMetadata),
    Versions {
        paper: PaperRef,
        versions: Vec<PaperVersion>,
    },
    Source(FetchedSource),
    NeedsFile {
        paper: PaperRef,
        reason: String,
    },
}

impl FetchOutcome {
    pub fn status_str(&self) -> &'static str {
        match self {
            Self::Metadata(_) => "metadata",
            Self::Versions { .. } => "versions",
            Self::Source(_) => "source",
            Self::NeedsFile { .. } => STATUS_NEEDS_FILE,
        }
    }
}

/// Count distinct fetched sources in a session's ledger events (`00-overview.md` 2.4).
///
/// Counts `source.ingested` events whose `origin` is a network URL (`http://`, `https://`)
/// or library cache entry (`library:`), deduplicated by `source_id` (falling back to
/// `raw_blob`). User-uploaded or workspace-local files (`upload`, `workspace:...`, local
/// filenames) are not counted as fetched external sources.
pub fn fetched_sources(events: &[Event]) -> usize {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for ev in events {
        if ev.kind != "source.ingested" {
            continue;
        }
        let origin = ev.payload["origin"].as_str().unwrap_or("");
        if !is_fetched_origin(origin) {
            continue;
        }
        let id = ev.payload["source_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| ev.payload["raw_blob"].as_str().filter(|s| !s.is_empty()));
        if let Some(key) = id {
            seen.insert(key.to_string());
        } else {
            seen.insert(format!("seq:{}", ev.seq));
        }
    }
    seen.len()
}

fn is_fetched_origin(origin: &str) -> bool {
    let lower = origin.trim().to_ascii_lowercase();
    lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("library:")
}

/// Current fetched-source budget usage for a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchBudgetStatus {
    pub used: usize,
    pub limit: usize,
}

impl FetchBudgetStatus {
    pub fn remaining(self) -> usize {
        self.limit.saturating_sub(self.used)
    }

    pub fn can_fetch(self) -> bool {
        self.used < self.limit
    }
}

/// Check whether `events` still has room under `limit` fetched sources (`limit` is a parameter).
pub fn check_fetch_budget(events: &[Event], limit: usize) -> Result<FetchBudgetStatus, String> {
    let used = fetched_sources(events);
    let status = FetchBudgetStatus { used, limit };
    if !status.can_fetch() {
        return Err(format!(
            "session fetch limit reached ({used} of {limit} fetched sources); raise the limit explicitly to fetch more"
        ));
    }
    Ok(status)
}

/// Synchronous paper fetcher using the process-wide [`global_gate`].
pub fn fetch(env: &ResearchEnv, paper: &PaperRef, want: FetchWant) -> Result<FetchOutcome, String> {
    fetch_with_gate(env, paper, want, global_gate())
}

/// Synchronous paper fetcher using an explicit [`NetRateGate`] (for fake-clock tests).
pub fn fetch_with_gate(
    env: &ResearchEnv,
    paper: &PaperRef,
    want: FetchWant,
    gate: &NetRateGate,
) -> Result<FetchOutcome, String> {
    let mut noop = |_stage: FetchStage| {};
    fetch_with_progress(env, paper, want, gate, &mut noop)
}

/// Synchronous core with progress callback (`on_stage`) shared by [`fetch`] and [`FetchJob`].
pub fn fetch_with_progress(
    env: &ResearchEnv,
    paper: &PaperRef,
    want: FetchWant,
    gate: &NetRateGate,
    on_stage: &mut dyn FnMut(FetchStage),
) -> Result<FetchOutcome, String> {
    match want {
        FetchWant::Metadata => fetch_metadata_flow(env, paper, gate, on_stage),
        FetchWant::Versions => fetch_versions_flow(env, paper, gate, on_stage),
        FetchWant::Source => fetch_source_flow(env, paper, gate, on_stage),
    }
}

fn fetch_metadata_flow(
    env: &ResearchEnv,
    paper: &PaperRef,
    gate: &NetRateGate,
    on_stage: &mut dyn FnMut(FetchStage),
) -> Result<FetchOutcome, String> {
    on_stage(FetchStage::ResolvingId);
    if let Some((resolved, _)) = resolve_to_arxiv(paper, &env.network_endpoints, gate)? {
        if let Some(base) = local_arxiv_base(&resolved) {
            on_stage(FetchStage::FetchingMetadata);
            if let Some(meta) =
                fetch_arxiv_metadata(&resolved, &base, &env.network_endpoints, gate)?
            {
                return Ok(FetchOutcome::Metadata(meta));
            }
        }
    }
    if let Some(doi) = paper
        .ids
        .doi
        .as_deref()
        .or_else(|| (paper.key.kind() == PaperKeyKind::Doi).then_some(paper.key.value()))
    {
        on_stage(FetchStage::FetchingMetadata);
        if let Some(meta) = fetch_doi_metadata(paper, doi, &env.network_endpoints, gate)? {
            return Ok(FetchOutcome::Metadata(meta));
        }
    }
    Ok(FetchOutcome::NeedsFile {
        paper: paper.clone(),
        reason: "no metadata available on allowlisted services; drop or attach the file".into(),
    })
}

fn fetch_versions_flow(
    env: &ResearchEnv,
    paper: &PaperRef,
    gate: &NetRateGate,
    on_stage: &mut dyn FnMut(FetchStage),
) -> Result<FetchOutcome, String> {
    on_stage(FetchStage::ResolvingId);
    let Some((resolved, _)) = resolve_to_arxiv(paper, &env.network_endpoints, gate)? else {
        return Ok(FetchOutcome::NeedsFile {
            paper: paper.clone(),
            reason: "paper has no arXiv version; drop or attach a PDF or TeX file".into(),
        });
    };
    let Some(base) = local_arxiv_base(&resolved) else {
        return Ok(FetchOutcome::NeedsFile {
            paper: paper.clone(),
            reason: "paper has no arXiv id".into(),
        });
    };
    on_stage(FetchStage::FetchingVersions);
    match fetch_arxiv_versions(&resolved, &base, &env.network_endpoints, gate)? {
        Some((enriched, versions)) => Ok(FetchOutcome::Versions {
            paper: enriched,
            versions,
        }),
        None => Ok(FetchOutcome::NeedsFile {
            paper: resolved,
            reason: "arXiv returned no version entries for this paper".into(),
        }),
    }
}

fn fetch_source_flow(
    env: &ResearchEnv,
    paper: &PaperRef,
    gate: &NetRateGate,
    on_stage: &mut dyn FnMut(FetchStage),
) -> Result<FetchOutcome, String> {
    // 1. Library blob cache first: zero network requests on hit.
    on_stage(FetchStage::CheckingCache);
    let cache = FulltextCache::open(&env.data_dir)
        .map_err(|e| format!("cannot open fulltext cache: {e}"))?;
    if let Some(hit) = lookup_cache(&cache, paper, None)? {
        return Ok(FetchOutcome::Source(hit));
    }

    // 2. Resolve to an arXiv id (DOI / INSPIRE / S2 -> arXiv). If none exists, return needs_file.
    on_stage(FetchStage::ResolvingId);
    let Some((resolved, discovered_title)) =
        resolve_to_arxiv(paper, &env.network_endpoints, gate)?
    else {
        return Ok(FetchOutcome::NeedsFile {
            paper: paper.clone(),
            reason: "DOI or identifier has no arXiv version; drop or attach a PDF or TeX file"
                .into(),
        });
    };
    if resolved.key != paper.key {
        if let Some(hit) = lookup_cache(&cache, &resolved, discovered_title.clone())? {
            return Ok(FetchOutcome::Source(hit));
        }
    }

    let Some(base_id) = local_arxiv_base(&resolved) else {
        return Ok(FetchOutcome::NeedsFile {
            paper: resolved,
            reason: "resolved paper reference lacks an arXiv id".into(),
        });
    };
    let target_id = arxiv_target_id(&base_id, resolved.version.as_deref());
    let eprint_base = env.network_endpoints.arxiv_eprint_base.trim_end_matches('/');

    // 3. Try arXiv e-print (TeX) first.
    on_stage(FetchStage::FetchingEprint);
    let eprint_url = format!("{eprint_base}/e-print/{target_id}");
    let eprint_resp = fetch_response_with_gate(
        &eprint_url,
        &env.network_endpoints,
        MAX_EPRINT_BYTES,
        Some(Service::Arxiv),
        gate,
    )?;

    if eprint_resp.status == 200 {
        on_stage(FetchStage::Validating);
        let validated = validate_eprint_download(
            &target_id,
            &eprint_resp.final_url,
            eprint_resp.content_type.as_deref(),
            eprint_resp.body,
        )?;
        let fetched =
            cache_and_build_source(&cache, paper, &resolved, validated, discovered_title)?;
        return Ok(FetchOutcome::Source(fetched));
    }
    if !matches!(eprint_resp.status, 403 | 404 | 410) {
        return Err(format!(
            "HTTP {} fetching arXiv e-print `{eprint_url}`",
            eprint_resp.status
        ));
    }

    // 4. Fall back to arXiv PDF.
    on_stage(FetchStage::FetchingPdf);
    let pdf_url = format!("{eprint_base}/pdf/{target_id}.pdf");
    let pdf_resp = fetch_response_with_gate(
        &pdf_url,
        &env.network_endpoints,
        MAX_PDF_BYTES,
        Some(Service::Arxiv),
        gate,
    )?;

    if pdf_resp.status == 200 {
        on_stage(FetchStage::Validating);
        validate_pdf_download(pdf_resp.content_type.as_deref(), &pdf_resp.body)?;
        let validated = ValidatedDownload {
            origin: SourceOrigin::ArxivPdf,
            format: "pdf".into(),
            file_name: format!("arxiv_{}.pdf", safe_id_slug(&target_id)),
            origin_url: pdf_resp.final_url,
            raw_bytes: pdf_resp.body,
        };
        let fetched =
            cache_and_build_source(&cache, paper, &resolved, validated, discovered_title)?;
        return Ok(FetchOutcome::Source(fetched));
    }
    if pdf_resp.status == 404 {
        return Ok(FetchOutcome::NeedsFile {
            paper: resolved,
            reason: "neither arXiv e-print nor PDF was found; drop or attach the file".into(),
        });
    }
    Err(format!(
        "HTTP {} fetching arXiv PDF `{pdf_url}`",
        pdf_resp.status
    ))
}

fn lookup_cache(
    cache: &FulltextCache,
    paper: &PaperRef,
    title: Option<String>,
) -> Result<Option<FetchedSource>, String> {
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
    let file_name = cached_file_name(&resolved, &entry);
    let origin_url = format!("library:{}", entry.raw_blob);
    Ok(Some(FetchedSource {
        paper: resolved,
        origin: SourceOrigin::Library,
        file_name,
        origin_url,
        format: entry.format,
        raw_blob: entry.raw_blob,
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

fn cache_and_build_source(
    cache: &FulltextCache,
    original_paper: &PaperRef,
    resolved_paper: &PaperRef,
    download: ValidatedDownload,
    title: Option<String>,
) -> Result<FetchedSource, String> {
    let entry = cache
        .put_bytes(
            resolved_paper.key.as_str(),
            resolved_paper.version.as_deref(),
            &download.format,
            download.origin.as_str(),
            &download.raw_bytes,
        )
        .map_err(|e| format!("fulltext cache write failed: {e}"))?;
    if original_paper.key != resolved_paper.key {
        let _ = cache.index_existing_blob(
            original_paper.key.as_str(),
            original_paper.version.as_deref(),
            &entry.raw_blob,
            &download.format,
            entry.bytes,
            download.origin.as_str(),
        );
    }
    Ok(FetchedSource {
        paper: resolved_paper.clone(),
        origin: download.origin,
        file_name: download.file_name,
        origin_url: download.origin_url,
        format: download.format,
        raw_blob: sha256_hex(&download.raw_bytes),
        raw_bytes: download.raw_bytes,
        title,
    })
}

#[cfg(test)]
#[path = "paper_fetch_tests.rs"]
mod tests;
