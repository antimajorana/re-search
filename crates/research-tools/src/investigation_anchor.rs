//! Source-grounded anchor verification for Investigation records (legacy verbatim excerpts
//! and app-filled pointer quotes).
use crate::{SandboxRoot, WorkspaceSandbox};
use ledger::{Event, Ledger};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;

pub(crate) const ANCHOR_KINDS: &[&str] = &["passage", "equation", "figure", "table", "reference"];
pub(crate) const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_TOTAL_BYTES: usize = 32 * 1024 * 1024;
const MAX_ANCHOR_LINES: usize = 64;
const MAX_EXCERPT_BYTES: usize = 3_200;
const MAX_QUOTE_CHARS: usize = 300;
const MIN_QUOTE_NON_SPACE: usize = 16;
const MIN_WHOLE_LINE_NON_SPACE: usize = 3;
const RELOCATE_RADIUS: usize = 12;
const NARROW_PAD: usize = 2;
const LABEL_ABOVE_RADIUS: usize = 3;
const MAX_TRUE_TEXT_CHARS: usize = 600;
const MAX_FAULTS_ERROR_CHARS: usize = 3_500;
const APP_ONLY_FIELDS: &[&str] = &["excerpt_basis", "cited_range", "range_basis", "excerpt_window"];
const SECTION_NAMES: &[&str] = &[
    "abstract", "introduction", "conclusion", "conclusions", "discussion", "results",
    "methods", "summary", "appendix", "references", "acknowledgments", "acknowledgements",
];

pub(crate) fn bounded(text: &str, max: usize) -> bool { !text.trim().is_empty() && text.chars().count() <= max }
pub(crate) fn valid_id(id: &str, max: usize) -> bool {
    !id.is_empty() && id.len() <= max && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}
pub(crate) fn string_schema(max: usize) -> Value { json!({"type":"string","minLength":1,"maxLength":max}) }

pub(crate) fn anchor_schema() -> Value {
    json!({"type":"object","properties":{
        "source_id":string_schema(64),"path":string_schema(1024),
        "start_line":{"type":"integer","minimum":1},"end_line":{"type":"integer","minimum":1},
        "excerpt":string_schema(MAX_EXCERPT_BYTES),
        "quote":{"type":"string","minLength":MIN_WHOLE_LINE_NON_SPACE,"maxLength":MAX_QUOTE_CHARS},
        "evidence":{"type":"string","pattern":"^ev-[0-9]{1,12}$"},
        "kind":{"type":"string","enum":ANCHOR_KINDS},"label":string_schema(160)
    },"required":["source_id","kind"],"additionalProperties":false})
}

/// Refuse any app-only anchor field supplied on tool input before deserialization.
pub(crate) fn reject_app_only_fields(args: &Value) -> Result<(), String> {
    match args {
        Value::Object(map) => {
            for &field in APP_ONLY_FIELDS {
                if map.contains_key(field) {
                    return Err(format!("Anchor field '{field}' is set by the app and must not be supplied on input."));
                }
            }
            for value in map.values() { reject_app_only_fields(value)?; }
        }
        Value::Array(items) => { for item in items { reject_app_only_fields(item)?; } }
        _ => {}
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Anchor {
    pub(crate) source_id: String,
    #[serde(default)] pub(crate) path: String,
    #[serde(default)] pub(crate) start_line: usize,
    #[serde(default)] pub(crate) end_line: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub(crate) excerpt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub(crate) quote: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub(crate) evidence: Option<String>,
    pub(crate) kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub(crate) label: Option<String>,
    #[serde(default)] pub(crate) file_sha256: String,
    #[serde(default)] pub(crate) source_seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub(crate) excerpt_basis: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub(crate) cited_range: Option<[usize; 2]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub(crate) range_basis: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub(crate) excerpt_window: Option<[usize; 2]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnchorBasis { Model, CitedLines, Evidence }

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct AnchorCorrection {
    pub(crate) owner: String, pub(crate) lines: [usize; 2], pub(crate) from: [usize; 2], pub(crate) basis: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AnchorNote { pub(crate) basis: AnchorBasis, pub(crate) corrected: Option<AnchorCorrection> }

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct AnchorFault {
    pub(crate) owner: String, pub(crate) code: &'static str, pub(crate) message: String,
    #[serde(skip_serializing_if = "Option::is_none")] pub(crate) lines: Option<[usize; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")] pub(crate) true_text: Option<String>,
    #[serde(skip_serializing)] pub(crate) legacy: bool,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct AnchorReport {
    pub(crate) dropped: Vec<Value>, pub(crate) corrected: Vec<AnchorCorrection>,
    pub(crate) model_count: usize, pub(crate) cited_lines_count: usize, pub(crate) evidence_count: usize,
}

impl AnchorReport {
    pub(crate) fn record_note(&mut self, note: AnchorNote) {
        match note.basis {
            AnchorBasis::Model => self.model_count += 1,
            AnchorBasis::CitedLines => self.cited_lines_count += 1,
            AnchorBasis::Evidence => self.evidence_count += 1,
        }
        if let Some(correction) = note.corrected { self.corrected.push(correction); }
    }
    pub(crate) fn record_dropped(&mut self, fault: &AnchorFault) {
        self.dropped.push(json!({"owner": fault.owner, "code": fault.code}));
    }
    pub(crate) fn to_value(&self) -> Value { json!({"dropped": self.dropped, "corrected": self.corrected}) }
    pub(crate) fn basis_value(&self) -> Value {
        json!({"model": self.model_count, "cited_lines": self.cited_lines_count, "evidence": self.evidence_count})
    }
}

pub(crate) struct VerifiedFile {
    pub(crate) text: String, pub(crate) line_starts: Vec<usize>, pub(crate) sha256: String, pub(crate) source_seq: u64,
}

impl VerifiedFile {
    pub(crate) fn line_count(&self) -> usize { self.line_starts.len() }
    pub(crate) fn line_range_bytes(&self, start_line: usize, end_line: usize) -> (usize, usize) {
        (self.line_starts[start_line - 1], *self.line_starts.get(end_line).unwrap_or(&self.text.len()))
    }
    pub(crate) fn lines_text(&self, start_line: usize, end_line: usize) -> &str {
        let (start, end) = self.line_range_bytes(start_line, end_line);
        self.text[start..end].strip_suffix('\n').unwrap_or(&self.text[start..end])
    }
    pub(crate) fn line_str(&self, line: usize) -> &str { self.lines_text(line, line) }
    pub(crate) fn line_of_byte(&self, byte_pos: usize) -> usize {
        self.line_starts.partition_point(|&start| start <= byte_pos)
    }
}

pub(crate) struct SelectedPages<'a> {
    pub(crate) source: &'a str, pub(crate) path: &'a str,
    pub(crate) first: usize, pub(crate) last: usize, pub(crate) lines: Vec<(usize, usize, usize)>,
}

impl<'a> SelectedPages<'a> {
    pub(crate) fn for_paper(ledger: &Ledger, paper: &Value, receipt: &'a Value) -> Result<Option<Self>, String> {
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
    pub(crate) fn contains_line(&self, line: usize) -> bool {
        self.lines.iter().any(|&(p, start, end)| (self.first..=self.last).contains(&p) && (start..=end).contains(&line))
    }
    pub(crate) fn check_range(&self, source_id: &str, path: &str, start_line: usize, end_line: usize) -> Result<(), String> {
        if source_id != self.source { return Ok(()); }
        if path != self.path || !self.contains_line(start_line) || !self.contains_line(end_line) {
            return Err(format!("The entire primary-paper anchor must fall within selected PDF pages {}–{}.", self.first, self.last));
        }
        Ok(())
    }
    pub(crate) fn check(&self, a: &Anchor) -> Result<(), String> { self.check_range(&a.source_id, &a.path, a.start_line, a.end_line) }
}

pub(crate) struct SourceIndex<'a> {
    pub(crate) sandbox: WorkspaceSandbox, pub(crate) session: &'a str, pub(crate) primary_source_id: &'a str,
    pub(crate) receipts: BTreeMap<&'a str, &'a Event>, pub(crate) files: BTreeMap<String, VerifiedFile>,
    pub(crate) bytes_left: usize, pub(crate) pages: Option<SelectedPages<'a>>,
    pub(crate) history: &'a [Event], pub(crate) active_marker_seq: Option<u64>,
}

impl SourceIndex<'_> {
    pub(crate) fn load(&mut self, source_id: &str, path: &str) -> Result<&VerifiedFile, String> {
        if !self.files.contains_key(path) {
            let receipt = self.receipts.get(source_id)
                .ok_or("This anchor does not belong to an ingested source in this session.")?;
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
            let digest = data["files_sha256"].get(path).and_then(Value::as_str)
                .ok_or("No verified digest for this source file.")?;
            let resolved = self.sandbox.resolve_existing(SandboxRoot::Sources, self.session, path, false)
                .map_err(|_| format!("Recorded source file is unavailable: {path}"))?;
            if !resolved.metadata.is_file() || resolved.rel_path != path {
                return Err("Invalid source file.".into());
            }
            let limit = self.bytes_left.min(MAX_FILE_BYTES);
            if resolved.metadata.len() > limit as u64 {
                return Err("Source text exceeds the map reader's size limit. Use a shorter extracted source.".into());
            }
            let mut bytes = Vec::new();
            File::open(resolved.canonical_target)
                .and_then(|f| f.take(limit as u64 + 1).read_to_end(&mut bytes))
                .map_err(|e| e.to_string())?;
            if bytes.len() > limit || ledger::sha256_hex(&bytes) != digest {
                return Err(format!("Source file changed since ingestion: {path}"));
            }
            self.bytes_left -= bytes.len();
            let text = String::from_utf8(bytes)
                .map_err(|_| "Use a searchable UTF-8 source for this anchor.")?
                .replace("\r\n", "\n");
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

    /// Legacy verbatim-excerpt verification, byte-identical to pre-pointer behaviour.
    pub(crate) fn verify(&mut self, anchor: &mut Anchor) -> Result<(), String> {
        let excerpt_raw = anchor.excerpt.as_deref().unwrap_or("");
        if let Some(pages) = &self.pages { pages.check(anchor)?; }
        if !valid_id(&anchor.source_id, 64)
            || anchor.path.len() > 1024
            || anchor.path.split_once('/').map(|(id, _)| id) != Some(anchor.source_id.as_str())
            || !ANCHOR_KINDS.contains(&anchor.kind.as_str())
            || anchor.label.as_deref().is_some_and(|s| !bounded(s, 160))
            || anchor.start_line == 0
            || anchor.end_line < anchor.start_line
            || anchor.end_line - anchor.start_line >= MAX_ANCHOR_LINES
            || !bounded(excerpt_raw, MAX_EXCERPT_BYTES)
        {
            return Err("Each anchor needs a recorded path, 1–64 consecutive lines, and a short verbatim excerpt.".into());
        }
        let file = self.load(&anchor.source_id, &anchor.path)?;
        let (start, end) = (anchor.start_line - 1, anchor.end_line);
        if end > file.line_starts.len() {
            return Err(format!("Line {end} is outside {}.", anchor.path));
        }
        let passage = &file.text[file.line_starts[start]..*file.line_starts.get(end).unwrap_or(&file.text.len())];
        let excerpt = excerpt_raw.replace("\r\n", "\n");
        let needle = excerpt.strip_suffix('\n').unwrap_or(&excerpt);
        if anchor.label.as_deref().is_some_and(|label| !passage.contains(label)) {
            return Err(format!("Copy the printed label exactly from {} lines {}–{} or omit it.", anchor.path, anchor.start_line, end));
        }
        if needle.split('\n').count() != end - start || !passage.strip_suffix('\n').unwrap_or(passage).contains(needle) {
            return Err(format!("Excerpt does not match {} lines {}–{} exactly. Read the lines and retry.", anchor.path, anchor.start_line, end));
        }
        if (!anchor.file_sha256.is_empty() && anchor.file_sha256 != file.sha256)
            || (anchor.source_seq != 0 && anchor.source_seq != file.source_seq)
        {
            return Err("The cited source revision differs from the verified source file.".into());
        }
        anchor.file_sha256.clone_from(&file.sha256);
        anchor.source_seq = file.source_seq;
        Ok(())
    }

    pub(crate) fn verify_any(&mut self, owner: &str, anchor: &mut Anchor) -> Result<AnchorNote, AnchorFault> {
        if anchor.excerpt.is_some() {
            if anchor.quote.is_some() || anchor.evidence.is_some() {
                return Err(fault(owner, "invalid_anchor_form", "Provide exactly one of excerpt, quote, or evidence for each anchor.", range_opt(anchor), None, false));
            }
            return self.verify(anchor)
                .map(|()| AnchorNote { basis: AnchorBasis::Model, corrected: None })
                .map_err(|msg| fault(owner, "legacy_excerpt_failed", msg, range_opt(anchor), None, true));
        }
        if anchor.evidence.is_some() {
            return crate::investigation_evidence::resolve_evidence(self, owner, anchor);
        }
        if anchor.quote.is_some() {
            return self.verify_pointer(owner, anchor);
        }
        Err(fault(owner, "invalid_anchor_form", "Provide exactly one of excerpt, quote, or evidence for each anchor.", range_opt(anchor), None, false))
    }

    pub(crate) fn verify_pointer(&mut self, owner: &str, anchor: &mut Anchor) -> Result<AnchorNote, AnchorFault> {
        let cited = range_opt(anchor);
        let fail = |code: &'static str, msg: String, lines: Option<[usize; 2]>, tt: Option<String>| {
            Err(fault(owner, code, msg, lines, tt, false))
        };
        if !valid_id(&anchor.source_id, 64)
            || !ANCHOR_KINDS.contains(&anchor.kind.as_str())
            || anchor.label.as_deref().is_some_and(|s| !bounded(s, 160))
        {
            return fail("invalid_anchor", "Anchor requires a valid source_id, kind, and optional bounded label.".into(), cited, None);
        }
        let (s, e) = (anchor.start_line, anchor.end_line);
        if s == 0 || e < s || e - s >= MAX_ANCHOR_LINES {
            return fail("invalid_line_range", "Provide 1-based start_line and end_line spanning 1–64 lines.".into(), cited, None);
        }
        let quote_raw = anchor.quote.as_deref().unwrap_or("");
        if quote_raw.trim().is_empty() || quote_raw.chars().count() > MAX_QUOTE_CHARS {
            return fail("invalid_quote", "Provide a non-empty quote of at most 300 characters.".into(), cited, None);
        }
        if anchor.path.is_empty() {
            if anchor.source_id == self.primary_source_id {
                let Some(primary) = self.receipts.get(anchor.source_id.as_str()).and_then(|ev| ev.payload["primary_path"].as_str()) else {
                    return fail("path_required", "Primary paper path is unavailable.".into(), cited, None);
                };
                anchor.path = primary.to_string();
            } else {
                return fail("path_required", "Non-primary source anchors must specify path.".into(), cited, None);
            }
        }
        if anchor.path.len() > 1024 || anchor.path.split_once('/').map(|(id, _)| id) != Some(anchor.source_id.as_str()) {
            return fail("invalid_path", "Anchor path must belong to its source_id.".into(), cited, None);
        }
        let (source_id, path) = (anchor.source_id.clone(), anchor.path.clone());
        if let Err(msg) = self.load(&source_id, &path) {
            return fail("source_unavailable", msg, cited, None);
        }
        let file = &self.files[&path];
        let total = file.line_count();
        let tt_at = |lo, hi| build_true_text(file, self.pages.as_ref(), &source_id, &path, lo, hi);
        if s > total || e > total {
            return fail("line_out_of_range", format!("Cited lines {s}–{e} exceed {path} ({total} lines)."), Some([s, e]), tt_at(s, e));
        }
        let non_space = quote_raw.chars().filter(|c| !c.is_whitespace()).count();
        let (folded_quote, _) = fold_with_map(quote_raw);
        if non_space < MIN_WHOLE_LINE_NON_SPACE || folded_quote.is_empty() {
            return fail("quote_too_short", "Quote needs at least 16 non-space characters (or 3+ for an exact whole-line match).".into(), Some([s, e]), tt_at(s, e));
        }
        let in_range = find_folded_matches(file, s, e, &folded_quote);
        let mut range_basis = Vec::new();
        let mut cited_range = None;
        let (match_a, match_b, relocated) = match in_range.len() {
            1 => {
                let m = in_range[0];
                if non_space < MIN_QUOTE_NON_SPACE {
                    let (folded_cited, _) = fold_with_map(file.lines_text(s, e));
                    if !(m.start_line == s && m.end_line == e && folded_quote == folded_cited) {
                        return fail("quote_too_short", "A quote under 16 non-space characters must match the entire cited range verbatim.".into(), Some([s, e]), tt_at(s, e));
                    }
                }
                (m.start_line, m.end_line, false)
            }
            n if n >= 2 => return fail("quote_ambiguous", format!("Quote matches multiple places in {path} (lines {}).", format_candidates(&in_range)), Some([s, e]), tt_at(s, e)),
            _ => {
                if non_space < MIN_QUOTE_NON_SPACE {
                    return fail("quote_too_short", "Quote needs at least 16 non-space characters.".into(), Some([s, e]), tt_at(s, e));
                }
                let nearby = find_folded_matches(file, s.saturating_sub(RELOCATE_RADIUS).max(1), e.saturating_add(RELOCATE_RADIUS).min(total), &folded_quote);
                match nearby.len() {
                    1 => {
                        range_basis.push("relocated".into());
                        cited_range = Some([s, e]);
                        (nearby[0].start_line, nearby[0].end_line, true)
                    }
                    0 => return fail("quote_not_found", format!("Quote was not found in {path} around lines {s}–{e}."), Some([s, e]), tt_at(s, e)),
                    _ => return fail("quote_ambiguous", format!("Quote matches multiple places near lines {s}–{e} in {path} (lines {}).", format_candidates(&nearby)), Some([s, e]), tt_at(s, e)),
                }
            }
        };
        let (mut final_start, mut final_end) = if relocated {
            (match_a, match_b)
        } else {
            let (ns, ne) = (s.max(match_a.saturating_sub(NARROW_PAD)), e.min(match_b + NARROW_PAD));
            if [ns, ne] != [s, e] { range_basis.push("narrowed".into()); cited_range = Some([s, e]); }
            (ns, ne)
        };
        let before_trim = [final_start, final_end];
        while final_start < final_end && file.line_str(final_start).trim().is_empty() { final_start += 1; }
        while final_end > final_start && file.line_str(final_end).trim().is_empty() { final_end -= 1; }
        if [final_start, final_end] != before_trim {
            range_basis.push("trimmed".into());
            cited_range.get_or_insert([s, e]);
        }
        if let Some(label) = anchor.label.as_deref() {
            if is_bare_section_name(label) {
                return fail("label_is_section_name", format!("Omit bare section heading '{label}' as anchor label; use printed equation, figure, or table labels only."), Some([s, e]), None);
            }
            if !file.lines_text(final_start, final_end).contains(label) {
                let mut extended = None;
                if final_start > 1 {
                    let (above_lo, above_hi) = (final_start.saturating_sub(LABEL_ABOVE_RADIUS).max(1), final_start - 1);
                    if above_lo <= above_hi {
                        let (base_off, _) = file.line_range_bytes(above_lo, above_hi);
                        let mut hits = file.lines_text(above_lo, above_hi).match_indices(label);
                        if let (Some((pos, _)), None) = (hits.next(), hits.next()) {
                            extended = Some(file.line_of_byte(base_off + pos));
                        }
                    }
                }
                if let Some(label_line) = extended {
                    final_start = label_line;
                    range_basis.push("label_extended".into());
                    cited_range.get_or_insert([s, e]);
                } else {
                    return fail("label_not_found", format!("Label '{label}' was not found in {path} lines {final_start}–{final_end} or immediately above."), Some([s, e]), tt_at(final_start, final_end));
                }
            }
        }
        if [final_start, final_end] == [s, e] {
            cited_range = None;
            range_basis.clear();
        }
        if final_end < final_start || final_end - final_start >= MAX_ANCHOR_LINES {
            return fail("range_too_long", "Final anchor range exceeds the 64-line limit.".into(), Some([s, e]), None);
        }
        if let Some(pages) = &self.pages {
            if let Err(msg) = pages.check_range(&source_id, &path, final_start, final_end) {
                return fail("outside_selected_pages", msg, Some([s, e]), None);
            }
        }
        if (!anchor.file_sha256.is_empty() && anchor.file_sha256 != file.sha256)
            || (anchor.source_seq != 0 && anchor.source_seq != file.source_seq)
        {
            return fail("source_revision_mismatch", "The cited source revision differs from the verified source file.".into(), Some([s, e]), None);
        }
        let full_lines = file.lines_text(final_start, final_end);
        let (excerpt, excerpt_window) = if full_lines.len() <= MAX_EXCERPT_BYTES {
            (full_lines.to_string(), None)
        } else {
            let (folded_full, orig_map) = fold_with_map(full_lines);
            let q_pos = folded_full.find(&folded_quote).unwrap_or(0);
            let q_start = orig_map.get(q_pos).copied().unwrap_or(0);
            let q_end = orig_map.get(q_pos + folded_quote.len().saturating_sub(1)).map(|&b| b + 1).unwrap_or(q_start);
            if q_end.saturating_sub(q_start) > MAX_EXCERPT_BYTES {
                return fail(
                    "quote_span_too_wide",
                    format!("Matched quote spans {} bytes in {path}, exceeding the {MAX_EXCERPT_BYTES}-byte excerpt limit.", q_end - q_start),
                    Some([s, e]),
                    None,
                );
            }
            let (w0, w1) = window_around(full_lines, q_start, q_end, MAX_EXCERPT_BYTES);
            (full_lines[w0..w1].to_string(), Some([w0, w1]))
        };
        anchor.start_line = final_start;
        anchor.end_line = final_end;
        anchor.excerpt = Some(excerpt);
        anchor.excerpt_window = excerpt_window;
        anchor.excerpt_basis = Some("cited_lines".into());
        anchor.cited_range = cited_range;
        anchor.range_basis = range_basis.clone();
        anchor.file_sha256.clone_from(&file.sha256);
        anchor.source_seq = file.source_seq;
        let corrected = cited_range.map(|from| AnchorCorrection {
            owner: owner.to_string(), lines: [final_start, final_end], from, basis: range_basis,
        });
        Ok(AnchorNote { basis: AnchorBasis::CitedLines, corrected })
    }
}

#[derive(Debug, Clone, Copy)]
struct FoldMatch { start_line: usize, end_line: usize }

/// Collapse every whitespace run (newlines included) to one space and trim ends.
/// Returns `(folded, orig_byte)` mapping each byte of `folded` back to its byte index in `raw`.
pub(crate) fn fold_with_map(raw: &str) -> (String, Vec<usize>) {
    let (mut folded, mut orig_byte) = (String::with_capacity(raw.len()), Vec::with_capacity(raw.len()));
    let (mut pending_ws, mut seen_non_ws) = (None, false);
    for (idx, ch) in raw.char_indices() {
        if ch.is_whitespace() {
            if seen_non_ws && pending_ws.is_none() { pending_ws = Some(idx); }
            continue;
        }
        if let Some(ws_idx) = pending_ws.take() { folded.push(' '); orig_byte.push(ws_idx); }
        seen_non_ws = true;
        let mut buf = [0u8; 4];
        let encoded = ch.encode_utf8(&mut buf);
        folded.push_str(encoded);
        for offset in 0..encoded.len() { orig_byte.push(idx + offset); }
    }
    (folded, orig_byte)
}

fn find_folded_matches(file: &VerifiedFile, start_line: usize, end_line: usize, folded_quote: &str) -> Vec<FoldMatch> {
    let (base_off, _) = file.line_range_bytes(start_line, end_line);
    let (folded, orig_byte) = fold_with_map(file.lines_text(start_line, end_line));
    let (mut hits, mut search_from) = (Vec::new(), 0);
    while search_from <= folded.len() {
        let Some(rel) = folded[search_from..].find(folded_quote) else { break };
        let (pos, end_pos) = (search_from + rel, search_from + rel + folded_quote.len());
        hits.push(FoldMatch {
            start_line: file.line_of_byte(base_off + orig_byte[pos]),
            end_line: file.line_of_byte(base_off + orig_byte[end_pos - 1]),
        });
        search_from = pos + folded[pos..].chars().next().map_or(1, char::len_utf8);
    }
    hits
}

fn format_candidates(matches: &[FoldMatch]) -> String {
    matches.iter().map(|m| if m.start_line == m.end_line { m.start_line.to_string() } else { format!("{}-{}", m.start_line, m.end_line) })
        .collect::<Vec<_>>().join(", ")
}

pub(crate) fn is_bare_section_name(label: &str) -> bool {
    let mut s = label.trim().trim_end_matches(['.', ':']).trim();
    if let Some(rest) = strip_section_number_prefix(s) { s = rest.trim(); }
    SECTION_NAMES.contains(&s.to_ascii_lowercase().as_str())
}

fn strip_section_number_prefix(s: &str) -> Option<&str> {
    let mut parts = s.splitn(2, char::is_whitespace);
    let (first, rest) = (parts.next()?, parts.next()?);
    let token = first.trim_end_matches('.');
    if token.is_empty() { return None; }
    let arabic = token.split('.').all(|seg| !seg.is_empty() && seg.bytes().all(|b| b.is_ascii_digit()));
    let roman = token.bytes().all(|b| matches!(b.to_ascii_uppercase(), b'I' | b'V' | b'X' | b'L' | b'C'));
    let single_letter = token.len() == 1 && token.bytes().all(|b| b.is_ascii_alphabetic());
    (arabic || roman || single_letter).then_some(rest)
}

fn window_around(text: &str, q_start: usize, q_end: usize, max_bytes: usize) -> (usize, usize) {
    let len = text.len();
    if len <= max_bytes { return (0, len); }
    let (mut q_start, mut q_end) = (q_start.min(len), q_end.clamp(q_start.min(len), len));
    while q_start > 0 && !text.is_char_boundary(q_start) { q_start -= 1; }
    while q_end < len && !text.is_char_boundary(q_end) { q_end += 1; }
    let q_len = q_end - q_start;
    let mut w0 = q_start.saturating_sub(max_bytes.saturating_sub(q_len) / 2).min(len.saturating_sub(max_bytes));
    while w0 < q_start && !text.is_char_boundary(w0) { w0 += 1; }
    let mut w1 = (w0 + max_bytes).min(len);
    while w1 > w0 && !text.is_char_boundary(w1) { w1 -= 1; }
    if w1 < q_end && q_len <= max_bytes {
        w1 = q_end;
        w0 = w1.saturating_sub(max_bytes);
        while w0 < q_start && !text.is_char_boundary(w0) { w0 += 1; }
    }
    (w0, w1)
}

fn build_true_text(file: &VerifiedFile, pages: Option<&SelectedPages<'_>>, source_id: &str, path: &str, s: usize, e: usize) -> Option<String> {
    let total = file.line_count();
    if total == 0 || pages.is_some_and(|p| source_id == p.source && path != p.path) { return None; }
    let (lo, hi) = (s.saturating_sub(2).max(1).min(total), e.saturating_add(2).min(total));
    let mut lines = Vec::new();
    for line in lo..=hi {
        if pages.is_some_and(|p| source_id == p.source && !p.contains_line(line)) { continue; }
        lines.push(format!("{line}| {}", file.line_str(line)));
    }
    if lines.is_empty() { return None; }
    Some(lines.join("\n").chars().take(MAX_TRUE_TEXT_CHARS).collect())
}

fn range_opt(a: &Anchor) -> Option<[usize; 2]> { (a.start_line > 0 && a.end_line > 0).then_some([a.start_line, a.end_line]) }

fn fault(owner: &str, code: &'static str, message: impl Into<String>, lines: Option<[usize; 2]>, true_text: Option<String>, legacy: bool) -> AnchorFault {
    AnchorFault { owner: owner.to_string(), code, message: message.into(), lines, true_text, legacy }
}

/// Format collected pointer faults as JSON capped at 3,500 chars:
/// drops `true_text` from the last faults first, then drops whole faults.
pub(crate) fn faults_error(faults: &[AnchorFault]) -> String {
    let mut items: Vec<Value> = faults.iter()
        .map(|f| serde_json::to_value(f).unwrap_or_else(|_| json!({"owner": f.owner, "code": f.code})))
        .collect();
    let mut more_faults: usize = 0;
    let render = |list: &[Value], more: usize| json!({"error": "anchors_failed", "faults": list, "more_faults": more}).to_string();
    while render(&items, more_faults).chars().count() > MAX_FAULTS_ERROR_CHARS {
        if let Some(obj) = items.iter_mut().rev().filter_map(Value::as_object_mut).find(|o| o.contains_key("true_text")) {
            obj.remove("true_text");
            continue;
        }
        if items.len() > 1 {
            items.pop();
            more_faults += 1;
            continue;
        }
        if let Some(obj) = items.first_mut().and_then(Value::as_object_mut) {
            if let Some(msg) = obj.get("message").and_then(Value::as_str) {
                let shorter: String = msg.chars().take(msg.chars().count() / 2).collect();
                obj.insert("message".into(), Value::String(shorter));
                continue;
            }
        }
        break;
    }
    render(&items, more_faults)
}

#[cfg(test)]
#[path = "investigation_anchor_tests.rs"]
mod tests;
