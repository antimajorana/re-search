//! Shim for `split-1`'s `investigation_anchor.rs` (Section 2.1 of 01-reader-recorder-split.md).
//!
//! At merge, the coordinator replaces this shim with `split-1`'s `investigation_anchor.rs`
//! and wires `verify_any` to call `crate::investigation_evidence::resolve_evidence`.

use crate::{SandboxRoot, WorkspaceSandbox};
use ledger::{Event, Ledger};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;

pub const ANCHOR_KINDS: &[&str] = &["passage", "equation", "figure", "table", "reference"];
pub const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: usize = 32 * 1024 * 1024;
const MAX_EXCERPT_BYTES: usize = 3200;
const RELOCATE_WINDOW: usize = 12;
const TRUE_TEXT_CAP: usize = 600;
const APP_ONLY_FIELDS: &[&str] = &["excerpt_basis", "cited_range", "range_basis", "excerpt_window"];
const SECTION_NAMES: &[&str] = &[
    "abstract", "introduction", "conclusion", "conclusions", "discussion", "results",
    "methods", "summary", "appendix", "references", "acknowledgment", "acknowledgments",
    "acknowledgement", "acknowledgements",
];

fn str_schema(max: usize) -> Value { json!({"type": "string", "minLength": 1, "maxLength": max}) }

pub fn anchor_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "source_id": str_schema(64), "path": str_schema(1024),
            "start_line": {"type": "integer", "minimum": 1}, "end_line": {"type": "integer", "minimum": 1},
            "excerpt": str_schema(MAX_EXCERPT_BYTES),
            "quote": {"type": "string", "minLength": 3, "maxLength": 300},
            "evidence": {"type": "string", "pattern": "^ev-[0-9]{1,12}$", "maxLength": 16},
            "kind": {"type": "string", "enum": ANCHOR_KINDS}, "label": str_schema(160)
        },
        "required": ["source_id", "kind"],
        "additionalProperties": false
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Anchor {
    pub source_id: String,
    #[serde(default)] pub path: String,
    #[serde(default)] pub start_line: usize,
    #[serde(default)] pub end_line: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub excerpt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub quote: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub evidence: Option<String>,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub label: Option<String>,
    #[serde(default)] pub file_sha256: String,
    #[serde(default)] pub source_seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub excerpt_basis: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub cited_range: Option<[usize; 2]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub range_basis: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub excerpt_window: Option<[usize; 2]>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnchorFault {
    pub owner: String,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub lines: Option<[usize; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub true_text: Option<String>,
    pub legacy: bool,
}

impl AnchorFault {
    pub fn to_error_string(&self) -> String {
        if self.legacy { self.message.clone() } else { format!("[{}] {}", self.code, self.message) }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnchorNote {
    pub owner: String,
    pub basis: String,
    pub corrected: bool,
    pub lines: [usize; 2],
    pub from: Option<[usize; 2]>,
    pub range_basis: Vec<String>,
}

pub fn reject_app_only_fields(val: &Value) -> Result<(), String> {
    match val {
        Value::Object(map) => {
            for field in APP_ONLY_FIELDS {
                if map.contains_key(*field) {
                    return Err(format!("[app_only_field] Anchor field `{field}` is set by the app and may not be supplied."));
                }
            }
            for v in map.values() { reject_app_only_fields(v)?; }
            Ok(())
        }
        Value::Array(items) => { items.iter().try_for_each(reject_app_only_fields) }
        _ => Ok(()),
    }
}

pub(crate) fn bounded(text: &str, max: usize) -> bool { !text.trim().is_empty() && text.chars().count() <= max }
pub(crate) fn valid_id(id: &str, max: usize) -> bool {
    !id.is_empty() && id.len() <= max && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}
pub(crate) fn non_space_count(s: &str) -> usize { s.chars().filter(|c| !c.is_whitespace()).count() }

pub(crate) fn fold_with_map(s: &str) -> (String, Vec<usize>) {
    let (mut folded, mut map, mut in_ws, mut ws_start) = (String::new(), Vec::new(), false, 0usize);
    for (idx, ch) in s.char_indices() {
        if ch.is_whitespace() {
            if !folded.is_empty() && !in_ws { in_ws = true; ws_start = idx; }
        } else {
            if in_ws { folded.push(' '); map.push(ws_start); in_ws = false; }
            let start_len = folded.len();
            folded.push(ch);
            for _ in start_len..folded.len() { map.push(idx); }
        }
    }
    (folded, map)
}

pub(crate) fn fold_text(s: &str) -> String { fold_with_map(s).0 }

pub(crate) fn find_folded_matches(haystack: &str, needle_raw: &str) -> Vec<(usize, usize)> {
    let needle = fold_text(needle_raw);
    if needle.is_empty() { return Vec::new(); }
    let (folded, map) = fold_with_map(haystack);
    let (mut matches, mut pos) = (Vec::new(), 0usize);
    while pos + needle.len() <= folded.len() {
        let Some(rel) = folded[pos..].find(&needle) else { break };
        let (f_start, f_end) = (pos + rel, pos + rel + needle.len());
        let orig_start = map[f_start];
        let last_orig = map[f_end - 1];
        let orig_end = haystack[last_orig..].chars().next().map_or(haystack.len(), |c| last_orig + c.len_utf8());
        matches.push((orig_start, orig_end));
        pos = f_start + folded[f_start..].chars().next().map_or(1, char::len_utf8);
    }
    matches
}

pub(crate) fn is_bare_section_name(label: &str) -> bool {
    let lower = label.trim().trim_end_matches(['.', ':', ')']).trim().to_ascii_lowercase();
    if lower.is_empty() { return false; }
    if SECTION_NAMES.contains(&lower.as_str()) { return true; }
    let stripped = lower
        .strip_prefix("section ").or_else(|| lower.strip_prefix("sec. "))
        .or_else(|| lower.strip_prefix("sec ")).or_else(|| lower.strip_prefix("§"))
        .unwrap_or(&lower).trim();
    if let Some(after) = stripped.strip_prefix("appendix").map(str::trim) {
        if after.is_empty() || (after.len() <= 3 && after.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')) {
            return true;
        }
    }
    let rest = stripped.trim_start_matches(|c: char| c.is_ascii_digit() || matches!(c, '.' | ')' | '-' | 'i' | 'v' | 'x' | ' '));
    !rest.is_empty() && SECTION_NAMES.contains(&rest)
}

pub struct VerifiedFile {
    pub text: String,
    pub line_starts: Vec<usize>,
    pub sha256: String,
    pub source_seq: u64,
}

impl VerifiedFile {
    pub fn line_count(&self) -> usize { self.line_starts.len() }
    pub fn line_byte_range(&self, start_line: usize, end_line: usize) -> Option<(usize, usize)> {
        if start_line == 0 || end_line < start_line || end_line > self.line_starts.len() { return None; }
        Some((self.line_starts[start_line - 1], *self.line_starts.get(end_line).unwrap_or(&self.text.len())))
    }
    pub fn slice_lines(&self, start_line: usize, end_line: usize) -> Option<&str> {
        let (s, e) = self.line_byte_range(start_line, end_line)?;
        let raw = &self.text[s..e];
        Some(raw.strip_suffix('\n').unwrap_or(raw))
    }
    pub fn line_for_byte(&self, byte_idx: usize) -> usize {
        match self.line_starts.binary_search(&byte_idx) { Ok(i) => i + 1, Err(i) => i.max(1) }
    }
}

pub struct SelectedPages<'a> {
    pub source: &'a str,
    pub path: &'a str,
    pub first: usize,
    pub last: usize,
    pub lines: Vec<(usize, usize, usize)>,
}

impl<'a> SelectedPages<'a> {
    pub fn for_paper(ledger: &Ledger, paper: &Value, receipt: &'a Value) -> Result<Option<Self>, String> {
        if paper["page_start"].is_null() && paper["page_end"].is_null() { return Ok(None); }
        let read_page = |field: &str, default: usize| -> Result<usize, String> {
            if paper[field].is_null() { return Ok(default); }
            paper[field].as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| "Invalid PDF page range.".into())
        };
        let count = receipt["page_count"].as_u64().and_then(|n| usize::try_from(n).ok()).ok_or("Page ranges require an ingested PDF.")?;
        let (first, last) = (read_page("page_start", 1)?, read_page("page_end", count)?);
        if receipt["format"] != "pdf" || first == 0 || last < first || last > count {
            return Err("Page range is outside the physical pages of the selected PDF.".into());
        }
        let source = receipt["source_id"].as_str().ok_or("Missing primary source ID.")?;
        let path = receipt["primary_path"].as_str().ok_or("Missing primary PDF extraction.")?;
        let lines = crate::pdf::verified_page_lines(receipt, || {
            ledger.get_blob(receipt["raw_blob"].as_str().ok_or("Original PDF unavailable.")?).map_err(|e| e.to_string())
        })?;
        Ok(Some(Self { source, path, first, last, lines }))
    }

    pub fn check_range(&self, source_id: &str, path: &str, start_line: usize, end_line: usize) -> Result<(), String> {
        if source_id != self.source { return Ok(()); }
        if path != self.path || !self.line_in_selection(start_line) || !self.line_in_selection(end_line) {
            return Err(format!("The entire primary-paper anchor must fall within selected PDF pages {}–{}.", self.first, self.last));
        }
        Ok(())
    }

    pub fn check(&self, a: &Anchor) -> Result<(), String> { self.check_range(&a.source_id, &a.path, a.start_line, a.end_line) }

    pub fn line_in_selection(&self, line: usize) -> bool {
        self.lines.iter().any(|&(p, start, end)| (self.first..=self.last).contains(&p) && (start..=end).contains(&line))
    }
}

pub struct SourceIndex<'a> {
    pub sandbox: WorkspaceSandbox,
    pub session: &'a str,
    pub primary_source_id: &'a str,
    pub receipts: BTreeMap<&'a str, &'a Event>,
    pub files: BTreeMap<String, VerifiedFile>,
    pub bytes_left: usize,
    pub pages: Option<SelectedPages<'a>>,
    pub history: &'a [Event],
    pub active_marker_seq: Option<u64>,
}

impl SourceIndex<'_> {
    pub(crate) fn load_path(&mut self, source_id: &str, path: &str) -> Result<&VerifiedFile, String> {
        if !self.files.contains_key(path) {
            let receipt = self.receipts.get(source_id).ok_or("This anchor does not belong to an ingested source in this session.")?;
            let data = &receipt.payload;
            if data["status"].as_str().is_some_and(|s| !matches!(s, "ok" | "pdf_only")) {
                return Err("The source lacks readable extracted text. Upload searchable PDF, TeX or text.".into());
            }
            if data["assembled_path"].as_str() == Some(path) && data["primary_path"].as_str() != Some(path) {
                return Err("Use the paper's original source file for this anchor. An assembled view has generated text and different line numbers.".into());
            }
            if !data["files"].as_array().is_some_and(|items| items.iter().any(|f| f.as_str() == Some(path))) {
                return Err(format!("The source receipt does not contain {path}."));
            }
            let digest = data["files_sha256"].get(path).and_then(Value::as_str).ok_or("No verified digest for this source file.")?;
            let resolved = self.sandbox.resolve_existing(SandboxRoot::Sources, self.session, path, false)
                .map_err(|_| format!("Recorded source file is unavailable: {path}"))?;
            if !resolved.metadata.is_file() || resolved.rel_path != path { return Err("Invalid source file.".into()); }
            let limit = self.bytes_left.min(MAX_FILE_BYTES);
            if resolved.metadata.len() > limit as u64 {
                return Err("Source text exceeds the map reader's size limit. Use a shorter extracted source.".into());
            }
            let mut bytes = Vec::new();
            File::open(resolved.canonical_target).and_then(|f| f.take(limit as u64 + 1).read_to_end(&mut bytes)).map_err(|e| e.to_string())?;
            if bytes.len() > limit || ledger::sha256_hex(&bytes) != digest {
                return Err(format!("Source file changed since ingestion: {path}"));
            }
            self.bytes_left -= bytes.len();
            let text = String::from_utf8(bytes).map_err(|_| "Use a searchable UTF-8 source for this anchor.")?.replace("\r\n", "\n");
            let mut line_starts = vec![0];
            for (i, _) in text.match_indices('\n') {
                if i + 1 < text.len() {
                    if line_starts.len() >= 1_000_000 { return Err("Source contains too many lines.".into()); }
                    line_starts.push(i + 1);
                }
            }
            self.files.insert(path.to_string(), VerifiedFile { text, line_starts, sha256: digest.into(), source_seq: receipt.seq });
        }
        Ok(&self.files[path])
    }

    pub(crate) fn load(&mut self, anchor: &Anchor) -> Result<&VerifiedFile, String> { self.load_path(&anchor.source_id, &anchor.path) }

    fn format_true_text(&self, source_id: &str, path: &str, s: usize, e: usize) -> Option<String> {
        let file = self.files.get(path)?;
        let (lo, hi) = (s.saturating_sub(2).max(1), (e + 2).min(file.line_count()));
        let mut out = String::new();
        for ln in lo..=hi {
            if self.pages.as_ref().is_some_and(|p| source_id == p.source && !p.line_in_selection(ln)) { continue; }
            let row = format!("{ln}| {}\n", file.slice_lines(ln, ln).unwrap_or(""));
            if out.chars().count() + row.chars().count() > TRUE_TEXT_CAP {
                out.extend(row.chars().take(TRUE_TEXT_CAP.saturating_sub(out.chars().count())));
                break;
            }
            out.push_str(&row);
        }
        (!out.is_empty()).then_some(out)
    }

    pub fn verify(&mut self, anchor: &mut Anchor) -> Result<(), String> {
        let Some(excerpt_raw) = anchor.excerpt.as_deref() else {
            return Err("Each anchor needs a recorded path, 1–64 consecutive lines, and a short verbatim excerpt.".into());
        };
        if let Some(pages) = &self.pages { pages.check(anchor)?; }
        if !valid_id(&anchor.source_id, 64)
            || anchor.path.len() > 1024
            || anchor.path.split_once('/').map(|(id, _)| id) != Some(anchor.source_id.as_str())
            || !ANCHOR_KINDS.contains(&anchor.kind.as_str())
            || anchor.label.as_deref().is_some_and(|s| !bounded(s, 160))
            || anchor.start_line == 0 || anchor.end_line < anchor.start_line || anchor.end_line - anchor.start_line >= 64
            || !bounded(excerpt_raw, MAX_EXCERPT_BYTES)
        {
            return Err("Each anchor needs a recorded path, 1–64 consecutive lines, and a short verbatim excerpt.".into());
        }
        let file = self.load(anchor)?;
        let (start, end) = (anchor.start_line - 1, anchor.end_line);
        if end > file.line_starts.len() { return Err(format!("Line {end} is outside {}.", anchor.path)); }
        let passage = &file.text[file.line_starts[start]..*file.line_starts.get(end).unwrap_or(&file.text.len())];
        let excerpt = excerpt_raw.replace("\r\n", "\n");
        let needle = excerpt.strip_suffix('\n').unwrap_or(&excerpt);
        if anchor.label.as_deref().is_some_and(|label| !passage.contains(label)) {
            return Err(format!("Copy the printed label exactly from {} lines {}–{} or omit it.", anchor.path, anchor.start_line, end));
        }
        if needle.split('\n').count() != end - start || !passage.strip_suffix('\n').unwrap_or(passage).contains(needle) {
            return Err(format!("Excerpt does not match {} lines {}–{} exactly. Read the lines and retry.", anchor.path, anchor.start_line, end));
        }
        if (!anchor.file_sha256.is_empty() && anchor.file_sha256 != file.sha256) || (anchor.source_seq != 0 && anchor.source_seq != file.source_seq) {
            return Err("The cited source revision differs from the verified source file.".into());
        }
        anchor.file_sha256.clone_from(&file.sha256);
        anchor.source_seq = file.source_seq;
        Ok(())
    }

    #[allow(clippy::result_large_err)]
    pub fn verify_pointer(&mut self, owner: &str, anchor: &mut Anchor) -> Result<AnchorNote, AnchorFault> {
        let mk_fault = |code: &str, msg: String, lines: Option<[usize; 2]>, true_text: Option<String>| AnchorFault {
            owner: owner.to_string(), code: code.to_string(), message: msg, lines, true_text, legacy: false,
        };
        if !valid_id(&anchor.source_id, 64) || !ANCHOR_KINDS.contains(&anchor.kind.as_str()) || anchor.label.as_deref().is_some_and(|s| !bounded(s, 160)) {
            return Err(mk_fault("invalid_anchor", "Invalid source_id, kind, or label.".into(), None, None));
        }
        if let Some(label) = anchor.label.as_deref().filter(|l| is_bare_section_name(l)) {
            return Err(mk_fault("label_is_section_name", format!("Label `{label}` is a bare section name."), None, None));
        }
        let Some(quote_raw) = anchor.quote.as_deref().filter(|q| bounded(q, 300)).map(str::to_string) else {
            return Err(mk_fault("quote_missing", "Pointer anchor requires a bounded quote.".into(), None, None));
        };
        if anchor.start_line == 0 || anchor.end_line < anchor.start_line {
            return Err(mk_fault("invalid_lines", "Pointer anchor requires 1-based start_line <= end_line.".into(), None, None));
        }
        if anchor.path.is_empty() {
            if anchor.source_id == self.primary_source_id {
                anchor.path = self.receipts.get(anchor.source_id.as_str()).and_then(|r| r.payload["primary_path"].as_str())
                    .map(str::to_string).ok_or_else(|| mk_fault("source_missing", "Primary paper receipt missing primary_path.".into(), None, None))?;
            } else {
                return Err(mk_fault("path_required", "Non-primary sources require an explicit path.".into(), None, None));
            }
        }
        if anchor.path.len() > 1024 || anchor.path.split_once('/').map(|(id, _)| id) != Some(anchor.source_id.as_str()) {
            return Err(mk_fault("invalid_path", "Anchor path must belong to source_id.".into(), None, None));
        }
        let (s, e, source_id, path) = (anchor.start_line, anchor.end_line, anchor.source_id.clone(), anchor.path.clone());
        self.load_path(&source_id, &path).map_err(|msg| mk_fault("source_load_failed", msg, Some([s, e]), None))?;
        let total_lines = self.files[&path].line_count();
        if s > total_lines {
            return Err(mk_fault("line_out_of_range", format!("Line {s} is outside {path} (total {total_lines} lines)."), Some([s, e]), self.format_true_text(&source_id, &path, total_lines, total_lines)));
        }
        let clamped_e = e.min(total_lines);
        let (ns_len, folded_quote) = (non_space_count(&quote_raw), fold_text(&quote_raw));
        if ns_len < 16 {
            let whole_ok = ns_len >= 3 && self.files[&path].slice_lines(s, clamped_e).is_some_and(|sl| fold_text(sl) == folded_quote && e == clamped_e);
            if !whole_ok {
                return Err(mk_fault("quote_too_short", "Quote needs >= 16 non-space chars (or >= 3 matching the exact cited range).".into(), Some([s, e]), self.format_true_text(&source_id, &path, s, clamped_e)));
            }
        }
        let (file_sha, source_seq, final_start, final_end, mut range_basis, match_bytes) = {
            let file = &self.files[&path];
            let (in_b0, in_b1) = file.line_byte_range(s, clamped_e).unwrap();
            let in_m = find_folded_matches(&file.text[in_b0..in_b1], &quote_raw);
            let mut rb = Vec::new();
            let (mut f_start, mut f_end, m_b0, m_b1, relocated) = if in_m.len() == 1 {
                let (a0, a1) = (in_b0 + in_m[0].0, in_b0 + in_m[0].1);
                (file.line_for_byte(a0), file.line_for_byte(a1.saturating_sub(1).max(a0)), a0, a1, false)
            } else if in_m.len() > 1 {
                let cands: Vec<usize> = in_m.iter().map(|(m0, _)| file.line_for_byte(in_b0 + m0)).collect();
                return Err(mk_fault("quote_ambiguous", format!("Quote matched multiple times at lines {cands:?}."), Some([s, e]), self.format_true_text(&source_id, &path, s, clamped_e)));
            } else {
                let (wb0, wb1) = file.line_byte_range(s.saturating_sub(RELOCATE_WINDOW).max(1), (clamped_e + RELOCATE_WINDOW).min(total_lines)).unwrap();
                let wm = find_folded_matches(&file.text[wb0..wb1], &quote_raw);
                if wm.len() == 1 {
                    let (a0, a1) = (wb0 + wm[0].0, wb0 + wm[0].1);
                    rb.push("relocated".to_string());
                    (file.line_for_byte(a0), file.line_for_byte(a1.saturating_sub(1).max(a0)), a0, a1, true)
                } else if wm.len() > 1 {
                    let cands: Vec<usize> = wm.iter().map(|(m0, _)| file.line_for_byte(wb0 + m0)).collect();
                    return Err(mk_fault("quote_ambiguous", format!("Quote matched multiple times nearby at lines {cands:?}."), Some([s, e]), self.format_true_text(&source_id, &path, s, clamped_e)));
                } else {
                    return Err(mk_fault("quote_not_found", format!("Quote not found in {path} near lines {s}–{e}."), Some([s, e]), self.format_true_text(&source_id, &path, s, clamped_e)));
                }
            };
            if !relocated {
                let (ns, ne) = (s.max(f_start.saturating_sub(2).max(1)), clamped_e.min(f_end + 2));
                if ns != s || ne != e { rb.push("narrowed".to_string()); }
                f_start = ns; f_end = ne;
            }
            let pre_trim = (f_start, f_end);
            while f_start < f_end && file.slice_lines(f_start, f_start).is_some_and(|l| l.trim().is_empty()) { f_start += 1; }
            while f_end > f_start && file.slice_lines(f_end, f_end).is_some_and(|l| l.trim().is_empty()) { f_end -= 1; }
            if (f_start, f_end) != pre_trim { rb.push("trimmed".to_string()); }
            if let Some(label) = anchor.label.as_deref() {
                if !file.slice_lines(f_start, f_end).unwrap_or("").contains(label) {
                    let (lo, hi) = (s.min(f_start.saturating_sub(3).max(1)), f_start.saturating_sub(1));
                    let found: Vec<usize> = if hi >= lo && lo >= 1 {
                        (lo..=hi).filter(|&ln| file.slice_lines(ln, ln).is_some_and(|l| l.contains(label))).collect()
                    } else { Vec::new() };
                    if found.len() == 1 { f_start = found[0]; rb.push("label_extended".to_string()); } else {
                        return Err(mk_fault("label_not_found", format!("Label `{label}` not found in {path} lines {f_start}–{f_end}."), Some([f_start, f_end]), self.format_true_text(&source_id, &path, f_start, f_end)));
                    }
                }
            }
            (file.sha256.clone(), file.source_seq, f_start, f_end, rb, (m_b0, m_b1))
        };
        if final_end - final_start >= 64 {
            return Err(mk_fault("range_too_long", "Final anchor range exceeds 64 lines.".into(), Some([final_start, final_end]), None));
        }
        if let Some(pages) = &self.pages {
            pages.check_range(&source_id, &path, final_start, final_end).map_err(|msg| mk_fault("outside_selected_pages", msg, Some([final_start, final_end]), None))?;
        }
        let file = &self.files[&path];
        let (p_b0, _) = file.line_byte_range(final_start, final_end).unwrap();
        let full_slice = file.slice_lines(final_start, final_end).unwrap_or("");
        let (excerpt, excerpt_window) = if full_slice.len() <= MAX_EXCERPT_BYTES {
            (full_slice.to_string(), None)
        } else {
            let (w0, w1) = compute_excerpt_window(full_slice, match_bytes.0.saturating_sub(p_b0).min(full_slice.len()), match_bytes.1.saturating_sub(p_b0).min(full_slice.len()), MAX_EXCERPT_BYTES);
            (full_slice[w0..w1].to_string(), Some([w0, w1]))
        };
        let corrected = final_start != s || final_end != e || !range_basis.is_empty();
        anchor.start_line = final_start;
        anchor.end_line = final_end;
        anchor.excerpt = Some(excerpt);
        anchor.excerpt_basis = Some("cited_lines".to_string());
        anchor.cited_range = (final_start != s || final_end != e).then_some([s, e]);
        anchor.range_basis = range_basis.clone();
        anchor.excerpt_window = excerpt_window;
        anchor.file_sha256 = file_sha;
        anchor.source_seq = source_seq;
        range_basis.dedup();
        Ok(AnchorNote { owner: owner.to_string(), basis: "cited_lines".to_string(), corrected, lines: [final_start, final_end], from: anchor.cited_range, range_basis })
    }

    #[allow(clippy::result_large_err)]
    pub fn verify_any(&mut self, owner: &str, anchor: &mut Anchor) -> Result<AnchorNote, AnchorFault> {
        match (anchor.excerpt.is_some(), anchor.quote.is_some(), anchor.evidence.is_some()) {
            (true, false, false) => {
                self.verify(anchor).map_err(|msg| AnchorFault {
                    owner: owner.to_string(), code: "legacy_anchor_failed".into(), message: msg,
                    lines: Some([anchor.start_line, anchor.end_line]), true_text: None, legacy: true,
                })?;
                Ok(AnchorNote { owner: owner.to_string(), basis: "model".into(), corrected: false, lines: [anchor.start_line, anchor.end_line], from: None, range_basis: Vec::new() })
            }
            (false, true, false) => self.verify_pointer(owner, anchor),
            (false, _, true) => crate::investigation_evidence::resolve_evidence(self, owner, anchor),
            _ => Err(AnchorFault {
                owner: owner.to_string(), code: "invalid_anchor_form".into(),
                message: "Supply either `excerpt`, `quote`, or `evidence` + `quote`.".into(),
                lines: None, true_text: None, legacy: false,
            }),
        }
    }
}

fn compute_excerpt_window(s: &str, m0: usize, m1: usize, max_bytes: usize) -> (usize, usize) {
    let mid = m0 + m1.saturating_sub(m0) / 2;
    let mut start = mid.saturating_sub(max_bytes / 2);
    let mut end = (start + max_bytes).min(s.len());
    if end == s.len() { start = s.len().saturating_sub(max_bytes); }
    if m0 < start { start = m0; end = (start + max_bytes).min(s.len()); }
    else if m1 > end { end = m1.min(s.len()); start = end.saturating_sub(max_bytes); }
    while start < s.len() && !s.is_char_boundary(start) { start += 1; }
    while end > start && !s.is_char_boundary(end) { end -= 1; }
    (start, end)
}
