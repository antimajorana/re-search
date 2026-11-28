//! Metadata, fail-closed licence checks, PDF/OCR page lookup, F16a override folding,
//! verified source loading, and bibliography resolution for the F15 Investigate write-up
//! adapter (`writeup_paper.rs`, `00-overview.md` 2.13, `investigate.md` 3 L', 6.21, 6.22, 12, D9).
//!
//! Strictly pure/read-only over verified ledger events, pinned source files, and optional
//! existing `library.db` / `lessons.db` stores. Zero model calls and zero explicit DB writes.

use crate::writeup::{BibEntry, QuoteLocator};
use crate::writeup_bib::resolve_bib_entry;
use ledger::{sha256_hex, Event};
use research_tools::investigation_markers::EVENT_INVESTIGATION_OVERRIDE;
use research_tools::lessons::LessonsStore;
use research_tools::lessons_schema::{
    EVENT_INVESTIGATION_NODE_OVERRIDE, LESSONS_DB_NAME, OCR_OVERLAY_LABEL,
};
use research_tools::library::{Library, PaperDetail};
use research_tools::paper_key::{PaperIds, PaperKey, PaperKeyKind, PaperRef};
use research_tools::sandbox::{SandboxRoot, WorkspaceSandbox};
use research_tools::skeleton::{compute_files_root_sha256, BibRef, PaperSkeleton, SkNode};
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

/// Fail-closed check: returns `true` ONLY for exact permissive open licences (`CC-BY`,
/// `CC-BY-<ver>`, `creativecommons.org/licenses/by/<ver>`, `CC0*`, Public Domain).
/// Rejects `nonexclusive-distrib`, `NC`, `ND`, `SA` (ShareAlike copyleft), negated strings,
/// unknown strings, and absent licences (`<= 300`-char quote cap enforced).
pub fn is_permissive_licence(licence: Option<&str>) -> bool {
    let Some(raw) = licence.map(str::trim).filter(|s| !s.is_empty()) else { return false };
    let lower = raw.to_ascii_lowercase();
    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '.')
        .filter(|t| !t.is_empty())
        .collect();
    // Reject any restriction, copyleft, or negation token/phrase fail-closed.
    let banned_tokens = [
        "nc", "nd", "sa", "nonexclusive", "noncommercial", "noderivs", "noderivatives",
        "sharealike", "restricted", "proprietary", "not", "no", "unlike", "except", "reserved",
    ];
    if tokens.iter().any(|t| banned_tokens.contains(t)) {
        return false;
    }
    if lower.contains("non-exclusive")
        || lower.contains("non-commercial")
        || lower.contains("no-deriv")
        || lower.contains("share-alike")
        || lower.contains("all rights reserved")
    {
        return false;
    }

    // 1. Canonical Creative Commons URLs (exact path check; no extra suffixes allowed).
    if let Some(rest) = lower
        .strip_prefix("https://creativecommons.org/")
        .or_else(|| lower.strip_prefix("http://creativecommons.org/"))
    {
        let rest = rest.trim_end_matches('/');
        if let Some(ver) = rest.strip_prefix("licenses/by/") {
            let ver = ver.strip_suffix("/legalcode").unwrap_or(ver);
            return is_cc_version(ver);
        }
        if let Some(ver) = rest.strip_prefix("publicdomain/zero/") {
            let ver = ver.strip_suffix("/legalcode").unwrap_or(ver);
            return is_cc_version(ver);
        }
        return false;
    }

    // 2. Exact SPDX / short identifiers (tokenized).
    match tokens.as_slice() {
        ["pd"] | ["public", "domain"] | ["cc0"] => true,
        ["cc0", ver] | ["cc0", ver, "universal"] => is_cc_version(ver),
        ["cc", "0"] => true,
        ["cc", "0", ver] => is_cc_version(ver),
        ["cc", "by"] => true,
        ["cc", "by", ver] | ["cc", "by", ver, "international"] | ["cc", "by", ver, "unported"] => {
            is_cc_version(ver)
        }
        _ => false,
    }
}

fn is_cc_version(ver: &str) -> bool {
    !ver.is_empty()
        && ver.len() <= 8
        && ver.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        && ver.bytes().next().is_some_and(|b| b.is_ascii_digit())
        && ver.bytes().last().is_some_and(|b| b.is_ascii_digit())
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
            let empty_layer = data["status"].as_str() == Some("empty_text_layer");
            for t in arr.iter().filter_map(Value::as_array).filter(|t| t.len() >= 3) {
                let page = t[0].as_u64().unwrap_or(0) as usize;
                let first_line = t[1].as_u64().unwrap_or(0) as usize;
                let last_line = t[2].as_u64().unwrap_or(0) as usize;
                if page > 0 && first_line > 0 && last_line >= first_line {
                    pages.push(PdfPageEntry { page, first_line, last_line, is_ocr: false, is_none: empty_layer });
                }
            }
        }
        Self { is_pdf, primary_path, pages }
    }

    pub fn has_scanned_pages(&self) -> bool {
        self.pages.iter().any(|p| p.is_ocr || p.is_none)
    }

    /// Resolve physical page locator and OCR flag for `(path, start_line..=end_line)`.
    /// Keeps physical page numbers even on `PageBasis::None` while marking `ocr = true`.
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
                min_p = Some(min_p.map_or(e.page, |m| m.min(e.page)));
                max_p = Some(max_p.map_or(e.page, |m| m.max(e.page)));
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
/// Applies only to tags present in `skeleton` with matching `node_fp` when provided (rule 6.22).
pub(crate) fn collect_reader_overrides(
    events: &[Event],
    skeleton: &PaperSkeleton,
    data_dir: &Path,
) -> BTreeMap<String, ReaderNodeOverride> {
    let fps_by_tag: BTreeMap<&str, &str> = skeleton
        .nodes
        .iter()
        .filter_map(|n| Some((n.tag.as_deref()?, n.fp.as_str())))
        .collect();
    let mut out: BTreeMap<String, ReaderNodeOverride> = BTreeMap::new();

    if data_dir.join("library").join(LESSONS_DB_NAME).is_file() {
        if let Ok(store) = LessonsStore::open(data_dir) {
            for ov in store.list_skeleton_overrides().unwrap_or_default() {
                if !fps_by_tag.contains_key(ov.node_tag.as_str())
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
            .filter(|s| fps_by_tag.contains_key(*s))
        else {
            continue;
        };
        if let Some(fp) = non_empty_str(p.get("node_fp").and_then(Value::as_str)) {
            if fps_by_tag.get(tag).copied() != Some(fp.as_str()) {
                continue;
            }
        }
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
/// Fails closed on empty/mismatched `files_root_sha256`, missing files, or SHA-256 mismatch.
/// Binary non-UTF-8 assets are hash-verified and skipped from the UTF-8 text map.
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
    if skeleton.files_root_sha256.trim().is_empty() {
        return Err("pinned skeleton is missing required `files_root_sha256` digest".to_string());
    }
    let actual_root = compute_files_root_sha256(&expected_map);
    if actual_root != skeleton.files_root_sha256 {
        return Err(format!(
            "source files_root_sha256 mismatch: skeleton pinned `{}`, receipt has `{actual_root}`",
            skeleton.files_root_sha256
        ));
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
        if let Ok(text) = String::from_utf8(bytes) {
            let lines = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l).to_string()).collect();
            files.insert(rel.clone(), lines);
        }
    }
    Ok(files)
}

/// Check if a line contains ONLY a `\begin{...}` opener (+ optional `[title]` / `\label{...}` / comment).
fn is_pure_begin_wrapper_line(line: &str) -> bool {
    let mut s = line.trim();
    if let Some(idx) = s.find('%') {
        if idx == 0 || !s[..idx].ends_with('\\') {
            s = s[..idx].trim();
        }
    }
    let Some(after_begin) = s.strip_prefix("\\begin{") else { return false };
    let Some(close) = after_begin.find('}') else { return false };
    let mut rest = after_begin[close + 1..].trim();
    if let Some(after_opt) = rest.strip_prefix('[') {
        let Some(opt_end) = after_opt.find(']') else { return false };
        rest = after_opt[opt_end + 1..].trim();
    }
    while let Some(after_lbl) = rest.strip_prefix("\\label{") {
        let Some(lbl_end) = after_lbl.find('}') else { return false };
        rest = after_lbl[lbl_end + 1..].trim();
    }
    rest.is_empty()
}

/// Check if a line contains ONLY an `\end{...}` closing tag (+ optional comment).
fn is_pure_end_wrapper_line(line: &str) -> bool {
    let mut s = line.trim();
    if let Some(idx) = s.find('%') {
        if idx == 0 || !s[..idx].ends_with('\\') {
            s = s[..idx].trim();
        }
    }
    let Some(after_end) = s.strip_prefix("\\end{") else { return false };
    let Some(close) = after_end.find('}') else { return false };
    after_end[close + 1..].trim().is_empty()
}

/// Extracted verbatim statement across all `node.spans` (supporting multi-file `\input` spans)
/// plus the primary file's verified `(path, start_line, end_line)` and whether `end_line` override applied.
pub(crate) struct ExtractedStatement {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub primary_text: String,
    pub full_text: String,
    pub end_line_override_applied: bool,
}

fn is_pure_input_directive(text: &str) -> bool {
    let t = text.trim();
    (t.starts_with("\\input{") || t.starts_with("\\include{")) && t.ends_with('}') && !t.contains('\n')
}

/// Extract clean verbatim statement text in `node.spans` order and its exact 1-based physical `(path, start, end)`.
pub(crate) fn extract_node_statement_span(
    node: &SkNode,
    override_end_line: Option<usize>,
    files: &BTreeMap<String, Vec<String>>,
) -> Option<ExtractedStatement> {
    let mut chunks: Vec<(String, usize, usize, String)> = Vec::new();
    let mut end_line_override_applied = false;
    for (idx, span) in node.spans.iter().enumerate() {
        let Some(lines) = files.get(&span.path) else { continue };
        let mut start = span.start_line;
        let mut end = span.end_line;
        if idx == 0 {
            if let Some(ov_end) = override_end_line.filter(|&e| e >= start && e <= lines.len()) {
                end = ov_end;
                end_line_override_applied = true;
            }
        }
        if start == 0 || end < start || end > lines.len() {
            continue;
        }
        if idx == 0 && is_pure_begin_wrapper_line(&lines[start - 1]) {
            start += 1;
        }
        if idx == 0 && end >= start && is_pure_end_wrapper_line(&lines[end - 1]) {
            end -= 1;
        }
        while start <= end && lines[start - 1].trim().is_empty() {
            start += 1;
        }
        while end >= start && lines[end - 1].trim().is_empty() {
            end -= 1;
        }
        if start > end {
            continue;
        }
        let text = lines[start - 1..end].join("\n").trim().to_string();
        if !text.is_empty() {
            chunks.push((span.path.clone(), start, end, text));
        }
    }
    if chunks.is_empty() {
        return None;
    }
    let primary_idx = if chunks.len() > 1 && is_pure_input_directive(&chunks[0].3) { 1 } else { 0 };
    let (path, start_line, end_line, primary_text) = chunks[primary_idx].clone();
    let full_text = chunks
        .iter()
        .enumerate()
        .filter(|(i, (_, _, _, t))| !(*i == 0 && primary_idx > 0 && is_pure_input_directive(t)))
        .map(|(_, (_, _, _, t))| t.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    Some(ExtractedStatement { path, start_line, end_line, primary_text, full_text, end_line_override_applied })
}

/// Neutralize TeX/Markdown math delimiters on OCR statements (`investigate.md` rule 6.21).
pub(crate) fn neutralize_ocr_math_delimiters(text: &str) -> String {
    text.replace("$$", " ")
        .replace('$', "")
        .replace("\\[", "(")
        .replace("\\]", ")")
        .replace("\\(", "(")
        .replace("\\)", ")")
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_bibliography(
    lib: Option<&Library>,
    primary_ref: &PaperRef,
    primary_title: &str,
    primary_authors: &[String],
    primary_year: Option<i32>,
    skeleton: &PaperSkeleton,
    referenced_bib_keys: &BTreeSet<String>,
    cited_by_bib: &BTreeMap<String, PaperRef>,
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
        let bref = bib_by_key.get(raw_key.as_str()).copied();
        let pref = cited_by_bib
            .get(raw_key)
            .cloned()
            .or_else(|| bref.map(bib_ref_to_paper_ref));
        let Some(pref) = pref else { continue };
        let title = bref
            .and_then(|b| b.title.as_deref().map(str::trim).filter(|s| !s.is_empty()))
            .or_else(|| bref.map(|b| b.raw.trim()).filter(|s| !s.is_empty()))
            .unwrap_or(raw_key.as_str());
        let entry = resolve_bib_entry(lib, &pref, title, &[], None, &mut used_keys)?;
        cite_key_map.insert(raw_key.clone(), entry.cite_key.clone());
        paper_ref_map.insert(raw_key.clone(), pref);
        entries.push(entry);
    }
    Ok(ResolvedBibliography { primary_cite_key, cite_key_map, paper_ref_map, entries })
}
