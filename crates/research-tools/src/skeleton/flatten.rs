//! In-memory `\input` / `\include` flattening with a physical line-origin map.
//!
//! Reads exclusively from the already ingested files of one source (either an in-memory map
//! or a verified `source.ingested` ledger receipt under `SandboxRoot::Sources`), never
//! accessing external paths. Reuses `tex_bundle::{score_tex_entrypoint,
//! find_tex_includes_in_line_block, normalize_rel_segments}` and
//! `tex_scan::strip_latex_comments_line`.

use super::model::{compute_files_root_sha256, Span};
use crate::sandbox::{SandboxRoot, WorkspaceSandbox};
use crate::tex_bundle::{
    find_tex_includes_in_line_block, normalize_rel_segments, score_tex_entrypoint,
};
use crate::tex_scan::strip_latex_comments_line;
use ledger::Event;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;

pub const MAX_INCLUDE_DEPTH: usize = 8;
pub const MAX_FLATTENED_LINES: usize = 250_000;
pub const MAX_FLATTENED_BYTES: usize = 16 * 1024 * 1024;
const MAX_INGESTED_FILE_BYTES: usize = 16 * 1024 * 1024;
const MAX_INGESTED_TOTAL_BYTES: usize = 32 * 1024 * 1024;

pub(crate) const VERBATIM_ENVS: &[&str] = &[
    "verbatim",
    "verbatim*",
    "Verbatim",
    "Verbatim*",
    "BVerbatim",
    "LVerbatim",
    "lstlisting",
    "minted",
    "comment",
    "filecontents",
    "filecontents*",
];

/// Physical source file and 1-based line number for one flattened line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginEntry {
    pub path: String,
    pub line: usize,
}

/// One line in the in-memory flattened TeX view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlattenedLine {
    pub text: String,
    pub origin: OriginEntry,
}

/// Result of in-memory `\input` / `\include` flattening.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlattenedTex {
    pub primary_path: String,
    pub lines: Vec<FlattenedLine>,
    pub inlined_files: Vec<String>,
    pub warnings: Vec<String>,
}

impl FlattenedTex {
    /// Physical origin of a 1-based flattened line index.
    pub fn origin_of(&self, flat_line: usize) -> Option<&OriginEntry> {
        flat_line.checked_sub(1).and_then(|idx| self.lines.get(idx)).map(|fl| &fl.origin)
    }

    /// Maps an inclusive 1-based flattened line range `[flat_start, flat_end]` into ordered
    /// contiguous physical [`Span`]s.
    ///
    /// Because contiguous runs in a sub-interval can only grow when the interval grows,
    /// every span produced for `[c_start, c_end]` lies inside some span produced for any
    /// enclosing interval `[p_start, p_end]` (`p_start <= c_start <= c_end <= p_end`).
    pub fn spans_for_range(&self, flat_start: usize, flat_end: usize) -> Vec<Span> {
        if self.lines.is_empty() || flat_start == 0 || flat_end < flat_start {
            return Vec::new();
        }
        let start_idx = flat_start.saturating_sub(1).min(self.lines.len().saturating_sub(1));
        let end_idx = flat_end.saturating_sub(1).min(self.lines.len().saturating_sub(1));
        if start_idx > end_idx {
            return Vec::new();
        }
        let mut spans: Vec<Span> = Vec::new();
        for fl in &self.lines[start_idx..=end_idx] {
            let entry = &fl.origin;
            if let Some(last) = spans.last_mut() {
                if last.path == entry.path
                    && (entry.line == last.end_line || entry.line == last.end_line + 1)
                {
                    last.end_line = entry.line;
                    continue;
                }
            }
            spans.push(Span::new(entry.path.clone(), entry.line, entry.line));
        }
        spans
    }

    /// Concatenates the raw lines in an inclusive 1-based flattened line range.
    pub fn raw_range_text(&self, flat_start: usize, flat_end: usize) -> String {
        if self.lines.is_empty() || flat_start == 0 || flat_end < flat_start {
            return String::new();
        }
        let start_idx = flat_start.saturating_sub(1).min(self.lines.len().saturating_sub(1));
        let end_idx = flat_end.saturating_sub(1).min(self.lines.len().saturating_sub(1));
        if start_idx > end_idx {
            return String::new();
        }
        let mut out = String::new();
        for (i, fl) in self.lines[start_idx..=end_idx].iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&fl.text);
        }
        out
    }
}

/// Verified physical TeX files loaded from one `source.ingested` ledger receipt.
#[derive(Debug, Clone)]
pub struct IngestedTexSource {
    pub source_id: String,
    pub source_seq: u64,
    pub primary_path: Option<String>,
    pub files: BTreeMap<String, String>,
    pub files_sha256: BTreeMap<String, String>,
    pub files_root_sha256: String,
}

/// Loads all physical text/TeX files listed in a `source.ingested` receipt, verifying each
/// file's SHA-256 digest against `files_sha256` and skipping the synthetic `assembled_path`.
pub fn load_ingested_tex_source(
    sandbox: &WorkspaceSandbox,
    session_id: &str,
    receipt: &Event,
) -> Result<IngestedTexSource, String> {
    if receipt.kind != "source.ingested" {
        return Err(format!("expected `source.ingested` event, got `{}`", receipt.kind));
    }
    let data = &receipt.payload;
    let source_id = data["source_id"]
        .as_str()
        .ok_or_else(|| "source.ingested receipt missing `source_id`".to_string())?
        .to_string();
    let primary_path = data["primary_path"].as_str().map(str::to_string);
    let assembled_path = data["assembled_path"].as_str();
    let files_arr = data["files"]
        .as_array()
        .ok_or_else(|| "source.ingested receipt missing `files` array".to_string())?;
    let hashes_obj = data["files_sha256"]
        .as_object()
        .ok_or_else(|| "source.ingested receipt missing `files_sha256` map".to_string())?;

    let mut files_sha256 = BTreeMap::new();
    for (k, v) in hashes_obj {
        let digest = v
            .as_str()
            .ok_or_else(|| format!("non-string digest for `{k}` in files_sha256"))?;
        files_sha256.insert(k.clone(), digest.to_string());
    }
    let files_root_sha256 = compute_files_root_sha256(&files_sha256);

    let mut files = BTreeMap::new();
    let mut bytes_left = MAX_INGESTED_TOTAL_BYTES;

    for item in files_arr {
        let Some(rel_path) = item.as_str() else { continue };
        // Never scan the synthetic assembled file (investigate.md 0.1).
        if assembled_path == Some(rel_path) && primary_path.as_deref() != Some(rel_path) {
            continue;
        }
        let expected_digest = files_sha256
            .get(rel_path)
            .ok_or_else(|| format!("missing files_sha256 entry for `{rel_path}`"))?;
        let resolved = sandbox
            .resolve_existing(SandboxRoot::Sources, session_id, rel_path, false)
            .map_err(|e| format!("cannot resolve ingested file `{rel_path}`: {e}"))?;
        if !resolved.metadata.is_file() || resolved.rel_path != rel_path {
            return Err(format!("ingested path `{rel_path}` is not a regular source file"));
        }
        let limit = bytes_left.min(MAX_INGESTED_FILE_BYTES);
        if resolved.metadata.len() > limit as u64 {
            return Err(format!("ingested file `{rel_path}` exceeds size budget"));
        }
        let mut bytes = Vec::new();
        File::open(&resolved.canonical_target)
            .and_then(|f| f.take(limit as u64 + 1).read_to_end(&mut bytes))
            .map_err(|e| format!("failed reading `{rel_path}`: {e}"))?;
        if bytes.len() > limit {
            return Err(format!("ingested file `{rel_path}` exceeds size budget"));
        }
        if ledger::sha256_hex(&bytes) != *expected_digest {
            return Err(format!("ingested file `{rel_path}` changed since ingestion"));
        }
        bytes_left = bytes_left.saturating_sub(bytes.len());
        if let Ok(text) = String::from_utf8(bytes) {
            files.insert(rel_path.to_string(), text);
        }
    }

    Ok(IngestedTexSource {
        source_id,
        source_seq: receipt.seq,
        primary_path,
        files,
        files_sha256,
        files_root_sha256,
    })
}

/// Picks the primary `.tex` entry point using [`score_tex_entrypoint`] when `preferred` is
/// absent or not found in `files`.
pub fn select_primary_tex(
    files: &BTreeMap<String, String>,
    preferred: Option<&str>,
) -> Option<String> {
    if let Some(pref) = preferred {
        if files.contains_key(pref) {
            return Some(pref.to_string());
        }
    }
    let tex_candidates: BTreeMap<String, String> = files
        .iter()
        .filter(|(p, _)| {
            let low = p.to_ascii_lowercase();
            low.ends_with(".tex") || low.ends_with(".ltx")
        })
        .map(|(p, raw)| {
            let stripped = raw
                .split('\n')
                .map(|l| strip_latex_comments_line(l.strip_suffix('\r').unwrap_or(l)))
                .collect::<Vec<_>>()
                .join("\n");
            (p.clone(), stripped)
        })
        .collect();
    if tex_candidates.is_empty() {
        return files.keys().next().cloned();
    }

    let mut referenced = BTreeSet::new();
    for (path, stripped) in &tex_candidates {
        let dir = parent_dir(path);
        for (_, target) in find_tex_includes_in_line_block(stripped) {
            if let Some(resolved) = resolve_include(path, dir, &target, files) {
                referenced.insert(resolved);
            }
        }
    }

    let mut best: Option<(i32, String)> = None;
    for (path, stripped) in &tex_candidates {
        let score = score_tex_entrypoint(path, stripped, !referenced.contains(path));
        let replace = match &best {
            None => true,
            Some((best_score, best_path)) => {
                score > *best_score || (score == *best_score && path < best_path)
            }
        };
        if replace {
            best = Some((score, path.clone()));
        }
    }
    best.map(|(_, path)| path)
}

/// Flattens `files` starting from `primary_path` (or auto-selected entry point), tracking
/// exact physical `(path, 1-based line)` origins for every flattened line.
pub fn flatten_tex_files(
    files: &BTreeMap<String, String>,
    primary_override: Option<&str>,
) -> Option<FlattenedTex> {
    let primary_path = select_primary_tex(files, primary_override)?;
    let primary_dir = parent_dir(&primary_path).to_string();
    let mut ctx = FlattenCtx {
        files,
        primary_dir,
        stack: vec![primary_path.clone()],
        inlined_files: Vec::new(),
        warnings: Vec::new(),
        lines: Vec::new(),
        total_bytes: 0,
        capped: false,
    };
    ctx.flatten_file(&primary_path, 0);
    Some(FlattenedTex {
        primary_path,
        lines: ctx.lines,
        inlined_files: ctx.inlined_files,
        warnings: ctx.warnings,
    })
}

struct FlattenCtx<'a> {
    files: &'a BTreeMap<String, String>,
    primary_dir: String,
    stack: Vec<String>,
    inlined_files: Vec<String>,
    warnings: Vec<String>,
    lines: Vec<FlattenedLine>,
    total_bytes: usize,
    capped: bool,
}

impl FlattenCtx<'_> {
    fn push_line(&mut self, text: String, path: &str, line: usize) -> bool {
        if self.capped {
            return false;
        }
        if self.lines.len() >= MAX_FLATTENED_LINES
            || self.total_bytes.saturating_add(text.len() + 1) > MAX_FLATTENED_BYTES
        {
            self.capped = true;
            self.warnings.push(format!(
                "flattened output cap reached ({MAX_FLATTENED_LINES} lines / {MAX_FLATTENED_BYTES} bytes)"
            ));
            return false;
        }
        self.total_bytes += text.len() + 1;
        self.lines.push(FlattenedLine {
            text,
            origin: OriginEntry { path: path.to_string(), line },
        });
        true
    }

    fn flatten_file(&mut self, cur_path: &str, depth: usize) {
        let Some(raw) = self.files.get(cur_path) else { return };
        let mut active_verb: Option<String> = None;
        let mut iffalse_depth: usize = 0;

        for (idx, raw_line) in raw.lines().enumerate() {
            if self.capped {
                return;
            }
            let phys_line = idx + 1;
            let clean_line = raw_line.strip_suffix('\r').unwrap_or(raw_line);

            if let Some(verb_env) = active_verb.as_deref() {
                let end_marker = format!("\\end{{{verb_env}}}");
                if clean_line.contains(&end_marker) {
                    active_verb = None;
                }
                self.push_line(clean_line.to_string(), cur_path, phys_line);
                continue;
            }

            let code_part = strip_latex_comments_line(clean_line);
            // Mask `\verb`, inline verbatim, and `\iffalse ... \fi` so includes inside them stay dormant.
            let scan_view = mask_dormant_spans(&code_part, &mut iffalse_depth);
            if let Some(v) = find_unclosed_verbatim_begin(&scan_view) {
                active_verb = Some(v);
                self.push_line(clean_line.to_string(), cur_path, phys_line);
                continue;
            }

            let includes = find_tex_includes_in_line_block(&scan_view);
            if includes.is_empty() {
                self.push_line(clean_line.to_string(), cur_path, phys_line);
                continue;
            }

            let mut cursor = 0usize;
            for ((span_start, span_end), target) in includes {
                let before = &code_part[cursor..span_start];
                if !before.trim().is_empty()
                    && !self.push_line(before.to_string(), cur_path, phys_line)
                {
                    return;
                }
                cursor = span_end;

                match resolve_include(cur_path, &self.primary_dir, &target, self.files) {
                    Some(resolved) if self.stack.contains(&resolved) => {
                        self.warnings.push(format!(
                            "include cycle detected: {cur_path}:{phys_line} -> {resolved}"
                        ));
                    }
                    Some(resolved) if depth >= MAX_INCLUDE_DEPTH => {
                        self.warnings.push(format!(
                            "include depth limit ({MAX_INCLUDE_DEPTH}) exceeded: {cur_path}:{phys_line} -> {resolved}"
                        ));
                    }
                    Some(resolved) => {
                        if !self.inlined_files.contains(&resolved) {
                            self.inlined_files.push(resolved.clone());
                        }
                        self.stack.push(resolved.clone());
                        self.flatten_file(&resolved, depth + 1);
                        self.stack.pop();
                    }
                    None => {
                        if is_tex_include_target(&target) {
                            self.warnings.push(format!(
                                "unresolved include `{target}` at {cur_path}:{phys_line}"
                            ));
                        }
                        if !self.push_line(
                            code_part[span_start..span_end].to_string(),
                            cur_path,
                            phys_line,
                        ) {
                            return;
                        }
                    }
                }
            }
            let after = &code_part[cursor..];
            if !after.trim().is_empty() && !self.push_line(after.to_string(), cur_path, phys_line) {
                return;
            }
        }
    }
}

pub(crate) fn resolve_include(
    cur_path: &str,
    primary_dir: &str,
    raw_target: &str,
    files: &BTreeMap<String, String>,
) -> Option<String> {
    let cleaned = raw_target.trim().trim_matches(['"', '\'']).replace('\\', "/");
    if cleaned.is_empty() || cleaned.starts_with('/') || cleaned.contains('\0') {
        return None;
    }
    let cur_dir = parent_dir(cur_path);
    let mut bases = Vec::with_capacity(3);
    bases.push(primary_dir);
    if cur_dir != primary_dir {
        bases.push(cur_dir);
    }
    if !bases.contains(&"") {
        bases.push("");
    }
    for base in bases {
        let joined = if base.is_empty() { cleaned.clone() } else { format!("{base}/{cleaned}") };
        let mut cands = vec![joined.clone()];
        let low = joined.to_ascii_lowercase();
        if !low.ends_with(".tex") && !low.ends_with(".ltx") {
            cands.push(format!("{joined}.tex"));
        }
        for cand in cands {
            if let Some(norm) = normalize_rel_segments(&cand) {
                if files.contains_key(&norm) {
                    return Some(norm);
                }
            }
        }
    }
    None
}

fn parent_dir(path: &str) -> &str {
    path.rfind('/').map(|idx| &path[..idx]).unwrap_or("")
}

fn is_tex_include_target(target: &str) -> bool {
    let trimmed = target.trim().trim_matches(['"', '\'']);
    let leaf = trimmed.rsplit('/').next().unwrap_or(trimmed);
    match leaf.rsplit_once('.') {
        None => true,
        Some((_, ext)) => matches!(ext.to_ascii_lowercase().as_str(), "tex" | "ltx"),
    }
}

fn mask_dormant_spans(code: &str, iffalse_depth: &mut usize) -> String {
    let mut masked = code.as_bytes().to_vec();
    let len = masked.len();
    let mut i = 0usize;
    while i < len {
        if *iffalse_depth == 0 && masked[i] == b'\\' && code[i + 1..].starts_with("verb") {
            let mut j = i + 5;
            if j < len && masked[j] == b'*' {
                j += 1;
            }
            if j < len && !masked[j].is_ascii_alphabetic() && !masked[j].is_ascii_whitespace() {
                let delim = masked[j];
                j += 1;
                while j < len && masked[j] != delim {
                    j += 1;
                }
                let end = (j + 1).min(len);
                masked[i..end].fill(b' ');
                i = end;
                continue;
            }
        }
        if *iffalse_depth == 0 && masked[i] == b'\\' && code[i + 1..].starts_with("begin{") {
            if let Some(rel_close) = code[i + 7..].find('}') {
                let env = &code[i + 7..i + 7 + rel_close];
                if VERBATIM_ENVS.contains(&env) {
                    let end_pat = format!("\\end{{{env}}}");
                    let after_begin = i + 7 + rel_close + 1;
                    if let Some(rel_end) = code[after_begin..].find(&end_pat) {
                        let end = after_begin + rel_end + end_pat.len();
                        masked[i..end].fill(b' ');
                        i = end;
                        continue;
                    }
                }
            }
        }
        if masked[i] == b'\\' {
            let mut k = i + 1;
            while k < len && masked[k].is_ascii_alphabetic() {
                k += 1;
            }
            if k == i + 1 {
                let end = (i + 2).min(len);
                if *iffalse_depth > 0 {
                    masked[i..end].fill(b' ');
                }
                i = end;
                continue;
            }
            let cmd = &code[i + 1..k];
            let dormant = *iffalse_depth > 0;
            if !dormant {
                if cmd == "iffalse" {
                    *iffalse_depth = 1;
                    masked[i..k].fill(b' ');
                }
            } else {
                masked[i..k].fill(b' ');
                if cmd == "fi" {
                    *iffalse_depth -= 1;
                } else if cmd == "else" && *iffalse_depth == 1 {
                    *iffalse_depth = 0;
                } else if is_nested_tex_if(cmd) {
                    *iffalse_depth += 1;
                }
            }
            i = k;
            continue;
        }
        if *iffalse_depth > 0 {
            let ch_len = code[i..].chars().next().map_or(1, char::len_utf8);
            masked[i..i + ch_len].fill(b' ');
            i += ch_len;
        } else {
            i += code[i..].chars().next().map_or(1, char::len_utf8);
        }
    }
    String::from_utf8(masked).unwrap_or_else(|_| code.to_string())
}

fn find_unclosed_verbatim_begin(scan_view: &str) -> Option<String> {
    for &env in VERBATIM_ENVS {
        let begin_pat = format!("\\begin{{{env}}}");
        if let Some(bpos) = scan_view.find(&begin_pat) {
            let end_pat = format!("\\end{{{env}}}");
            if !scan_view[bpos + begin_pat.len()..].contains(&end_pat) {
                return Some(env.to_string());
            }
        }
    }
    None
}

pub(crate) fn is_nested_tex_if(cmd: &str) -> bool {
    cmd.starts_with("if") && !matches!(cmd, "iff" | "ifthenelse")
}
