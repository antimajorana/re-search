//! Metadata, fail-closed licence checks, PDF/OCR page lookup, F16a override folding,
//! verified source loading, and bibliography resolution for the F15 Investigate write-up
//! adapter (`writeup_paper.rs`, `00-overview.md` 2.13, `investigate.md` 3 L', 6.21, 6.22, 12, D9).
//!
//! Strictly pure/read-only over verified ledger events, pinned source files, and optional
//! existing `library.db` / `lessons.db` stores. Zero model calls and zero disk writes.

use crate::writeup::{BibEntry, QuoteLocator};
use crate::writeup_bib::resolve_bib_entry;
use ledger::{sha256_hex, Event};
use research_tools::investigation_markers::EVENT_INVESTIGATION_OVERRIDE;
use research_tools::lessons::LessonsStore;
use research_tools::lessons_schema::{
    EVENT_INVESTIGATION_NODE_OVERRIDE, LESSONS_DB_NAME, OCR_OVERLAY_LABEL,
};
use research_tools::library::{Library, PaperDetail};
use research_tools::map_fold::FoldedTask;
use research_tools::paper_key::{PaperIds, PaperKey, PaperKeyKind, PaperRef};
use research_tools::sandbox::{SandboxRoot, WorkspaceSandbox};
use research_tools::skeleton::{
    compute_files_root_sha256, BibRef, PaperSkeleton, SkNode, SkNodeKind, SkeletonBuilder,
    SkeletonQuality,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

pub const EVENT_INVESTIGATION_PAPER: &str = "investigation.paper";
pub const EVENT_SOURCE_INGESTED: &str = "source.ingested";
pub const LIBRARY_DB_NAME: &str = "library.db";

/// Display tag for F16a reader overrides (`00-overview.md` 2.14, `investigate.md` rule 6.22,
/// owner scope decision 2026-11-25 in `00-overview.md` 8.1).
pub const CORRECTED_BY_YOU_LABEL: &str = OCR_OVERLAY_LABEL;
/// Note emitted for user-pinned main results (`investigate.md` §4.3).
pub const PINNED_MAIN_RESULT_NOTE: &str = "pinned as main result by you";
/// Status phrase when no model tasks ran (`00-overview.md` 2.13, 8.3 Q10).
pub const SKELETON_ONLY_STATUS: &str = "Skeleton only — no model tasks run";
/// Explanation when falling back to theorem-like skeleton nodes before Orient runs.
pub const FALLBACK_RESULTS_NOTE: &str =
    "Orient has not run and no main results are pinned; listing deterministic theorem-like nodes from the skeleton.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LicenceResolution {
    pub raw: Option<String>,
    pub open: bool,
    pub source: &'static str,
}

/// Fail-closed check: returns `true` ONLY for explicit permissive open licences (`CC-BY*` without
/// `-NC`/`-ND`, `CC0*`, Public Domain). arXiv's default `nonexclusive-distrib`, absent, and
/// unknown licences return `false` (`<= 300`-char quote cap enforced).
pub fn is_permissive_licence(licence: Option<&str>) -> bool {
    let Some(raw) = licence.map(str::trim).filter(|s| !s.is_empty()) else { return false };
    let lower = raw.to_ascii_lowercase();
    if lower.contains("nonexclusive")
        || lower.contains("non-exclusive")
        || lower.contains("all-rights-reserved")
        || lower.contains("all rights reserved")
        || lower.contains("by-nd")
        || lower.contains("by-nc")
        || lower.contains("by_nd")
        || lower.contains("by_nc")
    {
        return false;
    }
    if lower.contains("cc0")
        || lower.contains("publicdomain/zero")
        || lower.contains("public-domain")
        || lower.contains("public domain")
        || lower == "pd"
    {
        return true;
    }
    lower.contains("creativecommons.org/licenses/by/")
        || lower.contains("creativecommons.org/licenses/by-sa")
        || lower == "cc-by"
        || lower == "cc by"
        || lower.starts_with("cc-by-")
        || lower.starts_with("cc-by ")
        || lower.starts_with("cc by ")
        || lower.starts_with("cc by-")
        || lower.contains("cc-by-")
        || lower.contains("cc by ")
}

fn non_empty_str(v: Option<&str>) -> Option<String> {
    v.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn extract_licence_from_payload(payload: &Value) -> Option<String> {
    non_empty_str(payload.get("licence").and_then(Value::as_str))
        .or_else(|| non_empty_str(payload.get("license").and_then(Value::as_str)))
        .or_else(|| {
            payload.get("metadata").and_then(|m| {
                non_empty_str(m.get("licence").and_then(Value::as_str))
                    .or_else(|| non_empty_str(m.get("license").and_then(Value::as_str)))
            })
        })
}

/// Resolve paper licence in deterministic order:
/// 1. `investigation.paper` (`licence`/`license`), 2. `source.ingested` (`licence`/`license`),
/// 3. Read-only `Library::get_paper` (`PaperFlags.license`).
pub(crate) fn resolve_licence(
    paper_ev: Option<&Event>,
    receipt: &Event,
    lib_detail: Option<&PaperDetail>,
) -> LicenceResolution {
    if let Some(raw) = paper_ev.and_then(|e| extract_licence_from_payload(&e.payload)) {
        let open = is_permissive_licence(Some(&raw));
        return LicenceResolution { raw: Some(raw), open, source: "session metadata" };
    }
    if let Some(raw) = extract_licence_from_payload(&receipt.payload) {
        let open = is_permissive_licence(Some(&raw));
        return LicenceResolution { raw: Some(raw), open, source: "source receipt" };
    }
    if let Some(raw) = lib_detail.and_then(|d| non_empty_str(d.paper.flags.license.as_deref())) {
        let open = is_permissive_licence(Some(&raw));
        return LicenceResolution { raw: Some(raw), open, source: "library" };
    }
    LicenceResolution { raw: None, open: false, source: "not recorded" }
}

/// One physical PDF page entry parsed permissively from `source.ingested`.
/// Handles both F14a `PageBasis` (`text_layer`, `scan_text_layer`, `none`) and future F14b `"ocr"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PdfPageEntry {
    pub page: usize,
    pub first_line: usize,
    pub last_line: usize,
    pub is_ocr: bool,
    pub is_none: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PdfPageIndex {
    pub is_pdf: bool,
    pub primary_path: Option<String>,
    pub pages: Vec<PdfPageEntry>,
}

impl PdfPageIndex {
    pub fn from_receipt(receipt: &Event) -> Self {
        let data = &receipt.payload;
        let is_pdf = data["format"].as_str() == Some("pdf")
            || data["status"].as_str() == Some("pdf_only")
            || data.get("pdf_extractor").is_some_and(Value::is_string)
            || data.get("pdf_pages").is_some_and(Value::is_array)
            || data.get("pdf_page_lines").is_some_and(Value::is_array);
        let primary_path = non_empty_str(data["primary_path"].as_str());
        let mut pages = Vec::new();

        if let Some(arr) = data.get("pdf_pages").and_then(Value::as_array) {
            for item in arr {
                let Some(page) = item.get("page").and_then(Value::as_u64).map(|n| n as usize) else {
                    continue;
                };
                let basis = item.get("basis").and_then(Value::as_str).unwrap_or("").trim().to_ascii_lowercase();
                let is_none = basis == "none";
                let is_ocr = matches!(basis.as_str(), "scan_text_layer" | "ocr" | "scan")
                    || item.get("ocr").and_then(Value::as_bool) == Some(true);
                let first_line = item.get("first_line").and_then(Value::as_u64).unwrap_or(0) as usize;
                let last_line = item.get("last_line").and_then(Value::as_u64).unwrap_or(0) as usize;
                if page > 0 {
                    pages.push(PdfPageEntry { page, first_line, last_line, is_ocr, is_none });
                }
            }
        } else if let Some(arr) = data.get("pdf_page_lines").and_then(Value::as_array) {
            for t in arr.iter().filter_map(Value::as_array).filter(|t| t.len() >= 3) {
                let page = t[0].as_u64().unwrap_or(0) as usize;
                let first_line = t[1].as_u64().unwrap_or(0) as usize;
                let last_line = t[2].as_u64().unwrap_or(0) as usize;
                if page > 0 && first_line > 0 && last_line >= first_line {
                    pages.push(PdfPageEntry { page, first_line, last_line, is_ocr: false, is_none: false });
                }
            }
        }
        Self { is_pdf, primary_path, pages }
    }

    pub fn has_scanned_pages(&self) -> bool {
        self.pages.iter().any(|p| p.is_ocr || p.is_none)
    }

    pub fn locator_and_ocr(
        &self,
        path: &str,
        start_line: usize,
        end_line: usize,
    ) -> (QuoteLocator, bool, Option<(usize, usize)>) {
        if !self.is_pdf
            || self.pages.is_empty()
            || self.primary_path.as_deref().is_some_and(|p| p != path)
        {
            return (QuoteLocator::Lines { start_line, end_line }, false, None);
        }
        let (mut min_p, mut max_p, mut ocr) = (None::<usize>, None::<usize>, false);
        for e in &self.pages {
            if e.first_line == 0 || e.last_line < e.first_line {
                continue;
            }
            if start_line <= e.last_line && end_line >= e.first_line {
                if e.is_ocr || e.is_none {
                    ocr = true;
                }
                if !e.is_none {
                    min_p = Some(min_p.map_or(e.page, |m| m.min(e.page)));
                    max_p = Some(max_p.map_or(e.page, |m| m.max(e.page)));
                }
            }
        }
        if let (Some(p1), Some(p2)) = (min_p, max_p) {
            (QuoteLocator::Page { page: p1, start_line, end_line }, ocr, Some((p1, p2)))
        } else {
            (QuoteLocator::Lines { start_line, end_line }, ocr, None)
        }
    }
}

/// Non-mutating F16a reader override for a skeleton node (`investigate.md` §4.3, rule 6.22).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ReaderNodeOverride {
    pub kind: Option<String>,
    pub number: Option<String>,
    pub end_line: Option<usize>,
    pub not_a_node: bool,
    pub note: Option<String>,
}

/// Fold F16a reader overrides from read-only `lessons.db` (if present) and session ledger events.
pub(crate) fn collect_reader_overrides(
    events: &[Event],
    skeleton: &PaperSkeleton,
    data_dir: &Path,
) -> BTreeMap<String, ReaderNodeOverride> {
    let valid_tags: BTreeSet<&str> = skeleton.nodes.iter().filter_map(|n| n.tag.as_deref()).collect();
    let mut out: BTreeMap<String, ReaderNodeOverride> = BTreeMap::new();

    if data_dir.join("library").join(LESSONS_DB_NAME).is_file() {
        if let Ok(store) = LessonsStore::open(data_dir) {
            for ov in store.list_skeleton_overrides().unwrap_or_default() {
                if !valid_tags.contains(ov.node_tag.as_str())
                    || ov.paper_key.as_ref().is_some_and(|pk| pk != &skeleton.paper_key)
                {
                    continue;
                }
                out.insert(
                    ov.node_tag,
                    ReaderNodeOverride {
                        kind: ov.kind_override,
                        number: None,
                        end_line: ov.end_line_override,
                        not_a_node: false,
                        note: (!ov.note.trim().is_empty()).then_some(ov.note),
                    },
                );
            }
        }
    }

    for ev in events {
        if ev.kind != EVENT_INVESTIGATION_OVERRIDE && ev.kind != EVENT_INVESTIGATION_NODE_OVERRIDE {
            continue;
        }
        let p = &ev.payload;
        if p.get("source_id").and_then(Value::as_str).is_some_and(|s| s != skeleton.source_id) {
            continue;
        }
        let Some(tag) = p
            .get("tag")
            .or_else(|| p.get("node_tag"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| valid_tags.contains(*s))
        else {
            continue;
        };
        let fix = p.get("fix").filter(|v| v.is_object()).unwrap_or(p);
        let entry = out.entry(tag.to_string()).or_default();
        if let Some(k) = non_empty_str(fix.get("kind").or_else(|| fix.get("kind_override")).and_then(Value::as_str)) {
            entry.kind = Some(k);
        }
        if let Some(num) = non_empty_str(fix.get("number").and_then(Value::as_str)) {
            entry.number = Some(num);
        }
        if let Some(el) = fix
            .get("end_line")
            .or_else(|| fix.get("end_line_override"))
            .or_else(|| fix.get("span").and_then(|s| s.get("end_line")))
            .and_then(Value::as_u64)
            .filter(|&n| n > 0)
        {
            entry.end_line = Some(el as usize);
        }
        if let Some(flag) = fix.get("not_a_node").and_then(Value::as_bool) {
            entry.not_a_node = flag;
        }
        if let Some(note) = non_empty_str(p.get("note").or_else(|| fix.get("note")).and_then(Value::as_str)) {
            entry.note = Some(note);
        }
    }
    out
}

/// Load and cryptographically verify every physical source file listed in `receipt["files_sha256"]`.
pub(crate) fn load_and_verify_source_files(
    sandbox: &WorkspaceSandbox,
    session_id: &str,
    receipt: &Event,
    skeleton: &PaperSkeleton,
) -> Result<BTreeMap<String, Vec<String>>, String> {
    let hashes_obj = receipt.payload["files_sha256"]
        .as_object()
        .ok_or_else(|| "source.ingested receipt missing `files_sha256` map".to_string())?;
    let mut expected_map: BTreeMap<String, String> = BTreeMap::new();
    for (rel, val) in hashes_obj {
        let digest = val
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("source.ingested `files_sha256` entry for `{rel}` is invalid"))?;
        expected_map.insert(rel.clone(), digest.to_string());
    }
    if !skeleton.files_root_sha256.is_empty() {
        let actual_root = compute_files_root_sha256(&expected_map);
        if actual_root != skeleton.files_root_sha256 {
            return Err(format!(
                "source files_root_sha256 mismatch: skeleton pinned `{}`, receipt has `{actual_root}`",
                skeleton.files_root_sha256
            ));
        }
    }

    let mut files = BTreeMap::new();
    for (rel, expected_sha) in &expected_map {
        let resolved = sandbox
            .resolve_existing(SandboxRoot::Sources, session_id, rel, false)
            .map_err(|e| format!("pinned source file `{rel}` unavailable: {e}"))?;
        let bytes = fs::read(&resolved.canonical_target)
            .map_err(|e| format!("failed to read pinned source `{rel}`: {e}"))?;
        let actual_sha = sha256_hex(&bytes);
        if actual_sha != *expected_sha {
            return Err(format!(
                "pinned source file `{rel}` hash mismatch: expected `{expected_sha}`, got `{actual_sha}`"
            ));
        }
        let text = String::from_utf8(bytes)
            .map_err(|_| format!("pinned source file `{rel}` is not valid UTF-8"))?;
        let lines = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l).to_string()).collect();
        files.insert(rel.clone(), lines);
    }
    Ok(files)
}

/// Extract clean verbatim statement text and its exact 1-based physical `(path, start, end)`.
pub(crate) fn extract_node_statement_span(
    node: &SkNode,
    override_end_line: Option<usize>,
    files: &BTreeMap<String, Vec<String>>,
) -> Option<(String, usize, usize, String)> {
    let first_span = node.spans.first()?;
    let lines = files.get(&first_span.path)?;
    let mut start = first_span.start_line;
    let mut end = node
        .spans
        .iter()
        .filter(|s| s.path == first_span.path)
        .map(|s| s.end_line)
        .max()
        .unwrap_or(first_span.end_line);
    if let Some(ov_end) = override_end_line {
        if ov_end >= start && ov_end <= lines.len() {
            end = ov_end;
        }
    }
    if start == 0 || end < start || end > lines.len() {
        return None;
    }
    if end > start + 1
        && lines[start - 1].trim().starts_with("\\begin{")
        && lines[end - 1].trim().starts_with("\\end{")
        && !lines[start..end - 1].join("\n").trim().is_empty()
    {
        start += 1;
        end -= 1;
    }
    while start < end && lines[start - 1].trim().is_empty() {
        start += 1;
    }
    while end > start && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    let body = lines[start - 1..end].join("\n").trim().to_string();
    (!body.is_empty()).then_some((first_span.path.clone(), start, end, body))
}

pub(crate) fn open_existing_library(data_dir: &Path) -> Option<Library> {
    data_dir.join("library").join(LIBRARY_DB_NAME).is_file().then(|| Library::open(data_dir).ok()).flatten()
}

pub(crate) fn resolve_primary_paper_ref(
    skeleton: &PaperSkeleton,
    lib_detail: Option<&PaperDetail>,
) -> PaperRef {
    let mut ids = PaperIds::default();
    if let Some(d) = lib_detail {
        ids.arxiv = d.paper.ids.arxiv.clone();
        ids.doi = d.paper.ids.doi.clone();
        ids.inspire = d.paper.ids.inspire.clone();
        ids.s2 = d.paper.ids.s2.clone();
    }
    match skeleton.paper_key.kind() {
        PaperKeyKind::Arxiv if ids.arxiv.is_none() => ids.arxiv = Some(skeleton.paper_key.value().to_string()),
        PaperKeyKind::Doi if ids.doi.is_none() => ids.doi = Some(skeleton.paper_key.value().to_string()),
        PaperKeyKind::Inspire if ids.inspire.is_none() => ids.inspire = Some(skeleton.paper_key.value().to_string()),
        PaperKeyKind::S2 if ids.s2.is_none() => ids.s2 = Some(skeleton.paper_key.value().to_string()),
        _ => {}
    }
    PaperRef { key: skeleton.paper_key.clone(), version: skeleton.version.clone(), ids }
}

pub(crate) fn bib_ref_to_paper_ref(bib: &BibRef) -> PaperRef {
    let ids = PaperIds { arxiv: bib.arxiv_id.clone(), doi: bib.doi.clone(), ..Default::default() };
    if let Ok(pr) = PaperRef::from_ids(ids, None) {
        return pr;
    }
    let seed = if !bib.raw.trim().is_empty() { bib.raw.as_bytes() } else { bib.key.as_bytes() };
    PaperRef { key: PaperKey::from_file_bytes(seed), version: None, ids: PaperIds::default() }
}

pub(crate) struct ResolvedBibliography {
    pub primary_cite_key: String,
    pub cite_key_map: BTreeMap<String, String>,
    pub paper_ref_map: BTreeMap<String, PaperRef>,
    pub entries: Vec<BibEntry>,
}

pub(crate) fn resolve_bibliography(
    lib: Option<&Library>,
    primary_ref: &PaperRef,
    primary_title: &str,
    primary_authors: &[String],
    primary_year: Option<i32>,
    skeleton: &PaperSkeleton,
    referenced_bib_keys: &BTreeSet<String>,
) -> Result<ResolvedBibliography, String> {
    let mut used_keys = BTreeSet::new();
    let mut entries = Vec::new();
    let primary_entry = resolve_bib_entry(
        lib, primary_ref, primary_title, primary_authors, primary_year, &mut used_keys,
    )?;
    let primary_cite_key = primary_entry.cite_key.clone();
    entries.push(primary_entry);

    let bib_by_key: BTreeMap<&str, &BibRef> = skeleton.bib.iter().map(|b| (b.key.as_str(), b)).collect();
    let mut cite_key_map = BTreeMap::new();
    let mut paper_ref_map = BTreeMap::new();
    for raw_key in referenced_bib_keys {
        if let Some(bref) = bib_by_key.get(raw_key.as_str()) {
            let pref = bib_ref_to_paper_ref(bref);
            let title = bref.title.as_deref().map(str::trim).filter(|s| !s.is_empty()).unwrap_or_else(|| {
                let r = bref.raw.trim();
                if r.is_empty() { bref.key.as_str() } else { r }
            });
            let entry = resolve_bib_entry(lib, &pref, title, &[], None, &mut used_keys)?;
            cite_key_map.insert(raw_key.clone(), entry.cite_key.clone());
            paper_ref_map.insert(raw_key.clone(), pref);
            entries.push(entry);
        }
    }
    Ok(ResolvedBibliography { primary_cite_key, cite_key_map, paper_ref_map, entries })
}

pub(crate) fn quality_word(q: SkeletonQuality) -> &'static str {
    match q {
        SkeletonQuality::Full => "full",
        SkeletonQuality::Partial => "partial",
        SkeletonQuality::Weak => "weak",
    }
}

/// Mandatory opening status line (`00-overview.md` 2.13 rule 1, 8.3 Q10, `investigate.md` 6.21).
/// Never contains `"verified"`, `"proved"`, `"new"`, or `"novel"`.
pub(crate) fn build_status_line(
    skeleton: &PaperSkeleton,
    pdf_index: &PdfPageIndex,
    tasks: &[FoldedTask],
) -> String {
    let paper_id = match skeleton.version.as_deref().filter(|s| !s.is_empty()) {
        Some(v) => format!("{} ({v})", skeleton.paper_key),
        None => skeleton.paper_key.to_string(),
    };
    let origin = match skeleton.builder {
        SkeletonBuilder::Tex1 => "Structure from TeX",
        SkeletonBuilder::Html1 => "Structure from arXiv HTML",
        SkeletonBuilder::Pdf1 => "Structure estimated from PDF text",
    };
    let reason = skeleton
        .quality_reason
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| format!(" — {s}"))
        .unwrap_or_else(|| {
            if skeleton.builder == SkeletonBuilder::Pdf1 && pdf_index.has_scanned_pages() {
                " — scanned pages (OCR)".to_string()
            } else {
                String::new()
            }
        });
    let structure = format!(
        "{origin} ({}, {}{reason})",
        skeleton.builder.as_str(),
        quality_word(skeleton.quality)
    );
    let task_summary = if tasks.is_empty() {
        SKELETON_ONLY_STATUS.to_string()
    } else {
        let mut parts = Vec::new();
        if let Some(orient) = tasks.iter().rev().find(|t| t.task == "orient") {
            parts.push(format!("Orient: {}", orient.run_state_label));
        }
        let open_count = tasks.iter().filter(|t| t.task == "open").count();
        if open_count > 0 {
            parts.push(format!("Open tasks: {open_count}"));
        }
        let ctx_count = tasks.iter().filter(|t| t.task == "context").count();
        if ctx_count > 0 {
            parts.push(format!("Context tasks: {ctx_count}"));
        }
        if parts.is_empty() { format!("Tasks recorded: {}", tasks.len()) } else { parts.join("; ") }
    };
    format!("{paper_id}. {structure}. {task_summary}.")
}

pub(crate) fn kind_display(kind: SkNodeKind) -> &'static str {
    match kind {
        SkNodeKind::FrontMatter => "Front matter",
        SkNodeKind::Section => "Section",
        SkNodeKind::Definition => "Definition",
        SkNodeKind::Assumption => "Assumption",
        SkNodeKind::Theorem => "Theorem",
        SkNodeKind::Lemma => "Lemma",
        SkNodeKind::Proposition => "Proposition",
        SkNodeKind::Corollary => "Corollary",
        SkNodeKind::Conjecture => "Conjecture",
        SkNodeKind::Claim => "Claim",
        SkNodeKind::Proof => "Proof",
        SkNodeKind::Example => "Example",
        SkNodeKind::Remark => "Remark",
        SkNodeKind::Equation => "Equation",
        SkNodeKind::Figure => "Figure",
        SkNodeKind::Table => "Table",
        SkNodeKind::Algorithm => "Algorithm",
        SkNodeKind::Bibliography => "Bibliography",
    }
}
