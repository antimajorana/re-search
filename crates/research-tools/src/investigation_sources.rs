//! Source file loading, page-range checking, and verbatim anchor verification for Investigation records.
use super::{bounded, valid_id, Anchor, ANCHOR_KINDS};
use crate::{SandboxRoot, WorkspaceSandbox};
use ledger::{Event, Ledger};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;

const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 32 * 1024 * 1024;

struct VerifiedFile {
    text: String,
    line_starts: Vec<usize>,
    sha256: String,
    source_seq: u64,
}

pub(super) struct SourceIndex<'a> {
    sandbox: WorkspaceSandbox,
    session: &'a str,
    receipts: BTreeMap<&'a str, &'a Event>,
    files: BTreeMap<String, VerifiedFile>,
    bytes_left: usize,
    pub(super) pages: Option<SelectedPages<'a>>,
}

pub(super) struct SelectedPages<'a> {
    source: &'a str,
    path: &'a str,
    pub(super) first: usize,
    pub(super) last: usize,
    lines: Vec<(usize, usize, usize)>,
}

impl<'a> SelectedPages<'a> {
    pub(super) fn for_paper(
        ledger: &Ledger,
        paper: &Value,
        receipt: &'a Value,
    ) -> Result<Option<Self>, String> {
        if paper["page_start"].is_null() && paper["page_end"].is_null() {
            return Ok(None);
        }
        let read_page = |field: &str, default: usize| -> Result<usize, String> {
            if paper[field].is_null() {
                return Ok(default);
            }
            paper[field]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| "Invalid PDF page range.".into())
        };
        let count = receipt["page_count"]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or("Page ranges require an ingested PDF.")?;
        let first = read_page("page_start", 1)?;
        let last = read_page("page_end", count)?;
        if receipt["format"] != "pdf" || first == 0 || last < first || last > count {
            return Err("Page range is outside the physical pages of the selected PDF.".into());
        }
        let source = receipt["source_id"]
            .as_str()
            .ok_or("Missing primary source ID.")?;
        let path = receipt["primary_path"]
            .as_str()
            .ok_or("Missing primary PDF extraction.")?;
        let lines = crate::pdf::verified_page_lines(receipt, || {
            ledger
                .get_blob(
                    receipt["raw_blob"]
                        .as_str()
                        .ok_or("Original PDF unavailable.")?,
                )
                .map_err(|e| e.to_string())
        })?;
        Ok(Some(Self {
            source,
            path,
            first,
            last,
            lines,
        }))
    }

    fn check(&self, a: &Anchor) -> Result<(), String> {
        if a.source_id != self.source {
            return Ok(());
        }
        let in_selection = |line| {
            self.lines.iter().any(|&(p, start, end)| {
                (self.first..=self.last).contains(&p) && (start..=end).contains(&line)
            })
        };
        if a.path != self.path || !in_selection(a.start_line) || !in_selection(a.end_line) {
            return Err(format!(
                "The entire primary-paper anchor must fall within selected PDF pages {}–{}.",
                self.first, self.last
            ));
        }
        Ok(())
    }
}

impl<'a> SourceIndex<'a> {
    pub(super) fn new(
        sandbox: WorkspaceSandbox,
        session: &'a str,
        receipts: BTreeMap<&'a str, &'a Event>,
        pages: Option<SelectedPages<'a>>,
    ) -> Self {
        Self {
            pages,
            sandbox,
            session,
            receipts,
            files: BTreeMap::new(),
            bytes_left: MAX_TOTAL_BYTES,
        }
    }

    fn load(&mut self, anchor: &Anchor) -> Result<&VerifiedFile, String> {
        let path = anchor.path.as_str();
        if !self.files.contains_key(path) {
            let receipt = self
                .receipts
                .get(anchor.source_id.as_str())
                .ok_or("This anchor does not belong to an ingested source in this session.")?;
            let data = &receipt.payload;
            if data["status"]
                .as_str()
                .is_some_and(|s| !matches!(s, "ok" | "pdf_only"))
            {
                return Err(
                    "The source lacks readable extracted text. Upload searchable PDF, TeX or text."
                        .into(),
                );
            }
            if data["assembled_path"].as_str() == Some(path)
                && data["primary_path"].as_str() != Some(path)
            {
                return Err("Use the paper's original source file for this anchor. An assembled view has generated text and different line numbers.".into());
            }
            if !data["files"]
                .as_array()
                .is_some_and(|items| items.iter().any(|f| f.as_str() == Some(path)))
            {
                return Err(format!("The source receipt does not contain {path}."));
            }
            let digest = data["files_sha256"]
                .get(path)
                .and_then(Value::as_str)
                .ok_or("No verified digest for this source file.")?;
            let resolved = self
                .sandbox
                .resolve_existing(SandboxRoot::Sources, self.session, path, false)
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
                    if line_starts.len() >= 1_000_000 {
                        return Err("Source contains too many lines.".into());
                    }
                    line_starts.push(i + 1);
                }
            }
            self.files.insert(
                path.to_string(),
                VerifiedFile {
                    text,
                    line_starts,
                    sha256: digest.into(),
                    source_seq: receipt.seq,
                },
            );
        }
        Ok(&self.files[path])
    }

    pub(super) fn verify(&mut self, anchor: &mut Anchor) -> Result<(), String> {
        if let Some(pages) = &self.pages {
            pages.check(anchor)?;
        }
        if !valid_id(&anchor.source_id, 64)
            || anchor.path.len() > 1024
            || anchor.path.split_once('/').map(|(id, _)| id) != Some(anchor.source_id.as_str())
            || !ANCHOR_KINDS.contains(&anchor.kind.as_str())
            || anchor.label.as_deref().is_some_and(|s| !bounded(s, 160))
            || anchor.start_line == 0
            || anchor.end_line < anchor.start_line
            || anchor.end_line - anchor.start_line >= 64
            || !bounded(&anchor.excerpt, 3200)
        {
            return Err("Each anchor needs a recorded path, 1–64 consecutive lines, and a short verbatim excerpt.".into());
        }
        let file = self.load(anchor)?;
        let start = anchor.start_line - 1;
        let end = anchor.end_line;
        if end > file.line_starts.len() {
            return Err(format!("Line {end} is outside {}.", anchor.path));
        }
        let passage = &file.text
            [file.line_starts[start]..*file.line_starts.get(end).unwrap_or(&file.text.len())];
        let excerpt = anchor.excerpt.replace("\r\n", "\n");
        // Search excerpts may end in the newline of the last cited line.
        let needle = excerpt.strip_suffix('\n').unwrap_or(&excerpt);
        if anchor
            .label
            .as_deref()
            .is_some_and(|label| !passage.contains(label))
        {
            return Err(format!(
                "Copy the printed label exactly from {} lines {}–{} or omit it.",
                anchor.path, anchor.start_line, end
            ));
        }
        if needle.split('\n').count() != end - start
            || !passage
                .strip_suffix('\n')
                .unwrap_or(passage)
                .contains(needle)
        {
            return Err(format!(
                "Excerpt does not match {} lines {}–{} exactly. Read the lines and retry.",
                anchor.path, anchor.start_line, end
            ));
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
}
