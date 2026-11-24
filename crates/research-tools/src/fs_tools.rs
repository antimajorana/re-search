//! Sandboxed directory listing (`fs_list`) and bounded UTF-8 file reading (`fs_read`).

use crate::sandbox::{
    is_symlink_or_junction, sanitize_rel_components, SandboxError, SandboxRoot, WorkspaceSandbox,
};
use crate::ResearchEnv;
use policy::EffectClass;
use regex::RegexBuilder;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tools::{ApprovalScope, CallCtx, Registry, ToolDef};

pub const FS_LIST: &str = "fs_list";
pub const FS_READ: &str = "fs_read";

pub const DEFAULT_FS_READ_CHARS: usize = 8_000;
pub const MAX_FS_READ_CHARS: usize = 12_000;
pub const MAX_FS_READ_FILE_BYTES: u64 = 16 * 1024 * 1024;

const DEFAULT_FS_LIST_MAX: usize = 500;
const MAX_FS_LIST_ENTRIES: usize = 2_000;
const MAX_LIST_DEPTH: usize = 16;
const MAX_QUERY_MATCHES: usize = 50;
const MAX_LINE_EXCERPT_CHARS: usize = 500;
const MAX_REGEX_DFA_BYTES: usize = 256 * 1024;

pub fn fs_list_def() -> ToolDef {
    ToolDef {
        id: FS_LIST.into(),
        description: "List files and directories deterministically inside the attached `workspace` \
                      or per-session `sources` directory. Returns `/`-separated relative paths, \
                      entry types (`file` or `dir`), and byte sizes."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "root": { "type": "string", "maxLength": 16, "description": "'workspace' (default) or 'sources'." },
                "path": { "type": "string", "maxLength": 512, "description": "Optional relative directory path." },
                "recursive": { "type": "boolean", "description": "Recursively walk subdirectories (default false)." },
                "max_entries": { "type": "integer", "description": "Maximum entries to return (1..=2000, default 500)." }
            },
            "additionalProperties": false
        }),
        effect: EffectClass::ReadWorkspace,
        approval_scope: ApprovalScope::Once,
    }
}

pub fn fs_read_def() -> ToolDef {
    ToolDef {
        id: FS_READ.into(),
        description: "Read a UTF-8 text/LaTeX/BibTeX/data file inside the attached `workspace` or \
                      per-session `sources` directory with astral-safe character slicing (`offset`, \
                      `max_chars <= 12000`) or bounded substring/regex `query` search."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "maxLength": 512, "description": "Relative file path." },
                "root": { "type": "string", "maxLength": 16, "description": "'workspace' (default) or 'sources'." },
                "offset": { "type": "integer", "description": "0-based Unicode scalar character offset (default 0)." },
                "max_chars": { "type": "integer", "description": "Maximum characters to return (1..=12000, default 8000)." },
                "query": { "type": "string", "maxLength": 512, "description": "Optional substring or regex search pattern." },
                "regex": { "type": "boolean", "description": "Interpret `query` as a bounded regular expression." }
            },
            "required": ["path"],
            "additionalProperties": false
        }),
        effect: EffectClass::ReadWorkspace,
        approval_scope: ApprovalScope::Once,
    }
}

pub fn register(registry: &mut Registry, env: Arc<ResearchEnv>) {
    let list_env = env.clone();
    registry.register(
        fs_list_def(),
        Arc::new(move |ctx: &CallCtx<'_>, args: &Value| handle_fs_list(&list_env, ctx.session, args)),
    );
    registry.register(
        fs_read_def(),
        Arc::new(move |ctx: &CallCtx<'_>, args: &Value| handle_fs_read(&env, ctx.session, args)),
    );
}

#[derive(Debug)]
struct ListedEntry {
    path: String,
    entry_type: &'static str,
    bytes: u64,
}

pub fn handle_fs_list(env: &ResearchEnv, session: &str, args: &Value) -> Result<Value, String> {
    let root = SandboxRoot::parse_opt(args.get("root").and_then(Value::as_str)).map_err(|e| e.to_string())?;
    let rel_raw = args.get("path").and_then(Value::as_str).unwrap_or("");
    let recursive = args.get("recursive").and_then(Value::as_bool).unwrap_or(false);
    let max_entries = match args.get("max_entries") {
        Some(v) => {
            let n = v.as_i64().ok_or("`max_entries` must be a positive integer")?;
            if n < 1 {
                return Err("`max_entries` must be >= 1".into());
            }
            (n as usize).min(MAX_FS_LIST_ENTRIES)
        }
        None => DEFAULT_FS_LIST_MAX,
    };

    let sandbox = env.sandbox();
    let (_, norm_req) = sanitize_rel_components(rel_raw, true).map_err(|e| e.to_string())?;

    // Uncreated session `sources` root returns clean empty listing when querying root.
    if root == SandboxRoot::Sources
        && norm_req.is_empty()
        && sandbox
            .canonical_root(SandboxRoot::Sources, session, false)
            .map_err(|e| e.to_string())?
            .is_none()
    {
        return Ok(json!({
            "root": root.as_str(),
            "path": "",
            "entries": [],
            "total_entries": 0,
            "truncated": false
        }));
    }

    let resolved = sandbox
        .resolve_existing(root, session, rel_raw, true)
        .map_err(|e: SandboxError| e.to_string())?;
    if !resolved.metadata.is_dir() {
        return Err(format!("`{}` is not a directory", resolved.rel_path));
    }

    let mut visited = BTreeSet::new();
    visited.insert(resolved.canonical_target.clone());
    let mut collected = Vec::new();
    walk_dir_entries(
        &resolved.canonical_root,
        &resolved.canonical_target,
        &resolved.rel_path,
        recursive,
        0,
        &mut visited,
        &mut collected,
    )?;

    collected.sort_by(|a, b| a.path.cmp(&b.path));
    let total_entries = collected.len();
    let truncated = total_entries > max_entries;
    let items: Vec<Value> = collected
        .into_iter()
        .take(max_entries)
        .map(|e| json!({ "path": e.path, "type": e.entry_type, "bytes": e.bytes }))
        .collect();

    Ok(json!({
        "root": root.as_str(),
        "path": resolved.rel_path,
        "entries": items,
        "total_entries": total_entries,
        "truncated": truncated
    }))
}

fn walk_dir_entries(
    canon_root: &Path,
    dir: &Path,
    rel_prefix: &str,
    recursive: bool,
    depth: usize,
    visited: &mut BTreeSet<PathBuf>,
    out: &mut Vec<ListedEntry>,
) -> Result<(), String> {
    let rd = fs::read_dir(dir).map_err(|e| format!("cannot read directory: {e}"))?;
    let mut dir_items: Vec<_> = rd.filter_map(Result::ok).collect();
    dir_items.sort_by_key(|e| e.file_name());

    for entry in dir_items {
        let os_name = entry.file_name();
        let Some(name) = os_name.to_str() else { continue };
        if matches!(name, ".active.lock" | ".reclaim.lock" | "sidetalk.lock")
            || name.starts_with(".re-search-tmp-")
        {
            continue;
        }
        let raw_path = entry.path();
        let Ok(raw_meta) = fs::symlink_metadata(&raw_path) else { continue };
        let is_symlink = is_symlink_or_junction(&raw_path, &raw_meta);
        let Ok(canon_entry) = fs::canonicalize(&raw_path) else {
            continue; // skip broken symlinks
        };
        if !WorkspaceSandbox::is_same_or_inside(canon_root, &canon_entry) {
            continue; // skip symlinks/junctions escaping sandbox root
        }
        let Ok(canon_meta) = fs::metadata(&canon_entry) else { continue };
        let rel_path = if rel_prefix.is_empty() {
            name.to_string()
        } else {
            format!("{rel_prefix}/{name}")
        };
        if canon_meta.is_dir() {
            out.push(ListedEntry { path: rel_path.clone(), entry_type: "dir", bytes: 0 });
            if recursive && !is_symlink && depth + 1 < MAX_LIST_DEPTH && visited.insert(canon_entry.clone()) {
                walk_dir_entries(canon_root, &canon_entry, &rel_path, true, depth + 1, visited, out)?;
            }
        } else if canon_meta.is_file() {
            out.push(ListedEntry { path: rel_path, entry_type: "file", bytes: canon_meta.len() });
        }
    }
    Ok(())
}

pub fn handle_fs_read(env: &ResearchEnv, session: &str, args: &Value) -> Result<Value, String> {
    let raw_path = args.get("path").and_then(Value::as_str).ok_or("`path` must be a string")?;
    let root = SandboxRoot::parse_opt(args.get("root").and_then(Value::as_str)).map_err(|e| e.to_string())?;
    let offset = match args.get("offset") {
        Some(v) => {
            let n = v.as_i64().ok_or("`offset` must be a non-negative integer")?;
            if n < 0 {
                return Err("`offset` must be >= 0".into());
            }
            n as usize
        }
        None => 0,
    };
    let effective_max = match args.get("max_chars") {
        Some(v) => {
            let n = v.as_i64().ok_or("`max_chars` must be a positive integer")?;
            if n < 1 {
                return Err("`max_chars` must be >= 1".into());
            }
            (n as usize).min(MAX_FS_READ_CHARS)
        }
        None => DEFAULT_FS_READ_CHARS,
    };

    let sandbox = env.sandbox();
    let resolved = sandbox
        .resolve_existing(root, session, raw_path, false)
        .map_err(|e| e.to_string())?;
    if !resolved.metadata.is_file() {
        return Err(format!("`{}` is not a regular file", resolved.rel_path));
    }

    let file = File::open(&resolved.canonical_target)
        .map_err(|e| format!("cannot open `{}`: {e}", resolved.rel_path))?;
    let mut buf = Vec::new();
    file.take(MAX_FS_READ_FILE_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("cannot read `{}`: {e}", resolved.rel_path))?;
    if buf.len() as u64 > MAX_FS_READ_FILE_BYTES {
        return Err(format!("file `{}` exceeds 16 MiB read limit", resolved.rel_path));
    }
    if buf.contains(&0u8) {
        return Err(format!("file `{}` appears to be binary (contains NUL bytes)", resolved.rel_path));
    }
    let text = std::str::from_utf8(&buf)
        .map_err(|_| format!("file `{}` is not valid UTF-8 text", resolved.rel_path))?;

    let total_chars = text.chars().count();
    let total_lines = if text.is_empty() { 0 } else { text.lines().count() };
    let query = args.get("query").and_then(Value::as_str).unwrap_or("");
    let use_regex = args.get("regex").and_then(Value::as_bool).unwrap_or(false);

    if !query.is_empty() {
        let mut result = run_query_mode(&QueryParams {
            rel_path: &resolved.rel_path,
            root,
            text,
            total_chars,
            total_lines,
            query,
            use_regex,
            max_chars: effective_max,
        })?;
        result["file_sha256"] = json!(ledger::sha256_hex(&buf));
        return Ok(result);
    }

    let (slice, actual_start, actual_end, start_line, end_line) = slice_utf8_chars(text, offset, effective_max);
    let truncated = actual_end < total_chars;

    Ok(json!({
        "root": root.as_str(),
        "path": resolved.rel_path,
        "file_sha256": ledger::sha256_hex(&buf),
        "content": slice,
        "offset": actual_start,
        "end_offset": actual_end,
        "returned_chars": actual_end - actual_start,
        "total_chars": total_chars,
        "total_lines": total_lines,
        "start_line": start_line,
        "end_line": end_line,
        "truncated": truncated
    }))
}

fn slice_utf8_chars(text: &str, offset: usize, max_chars: usize) -> (&str, usize, usize, usize, usize) {
    if text.is_empty() {
        return ("", 0, 0, 0, 0);
    }
    let mut start_byte = text.len();
    let mut end_byte = text.len();
    let mut start_char = 0usize;
    let mut end_char = 0usize;
    let mut line_no = 1usize;
    let mut start_line = 1usize;
    let mut end_line = 1usize;
    let mut found_start = false;

    for (char_idx, (byte_idx, ch)) in text.char_indices().enumerate() {
        if char_idx == offset {
            start_byte = byte_idx;
            start_char = char_idx;
            start_line = line_no;
            found_start = true;
        }
        if found_start && char_idx - start_char == max_chars {
            end_byte = byte_idx;
            end_char = char_idx;
            break;
        }
        if found_start {
            end_line = line_no;
        }
        if ch == '\n' {
            line_no += 1;
        }
        end_char = char_idx + 1;
    }

    if !found_start {
        let total = text.chars().count();
        return ("", total, total, line_no, line_no);
    }
    (&text[start_byte..end_byte], start_char, end_char, start_line, end_line)
}

struct QueryParams<'a> {
    rel_path: &'a str,
    root: SandboxRoot,
    text: &'a str,
    total_chars: usize,
    total_lines: usize,
    query: &'a str,
    use_regex: bool,
    max_chars: usize,
}

fn run_query_mode(p: &QueryParams<'_>) -> Result<Value, String> {
    let compiled_re = if p.use_regex {
        Some(
            RegexBuilder::new(p.query)
                .size_limit(MAX_REGEX_DFA_BYTES)
                .dfa_size_limit(MAX_REGEX_DFA_BYTES)
                .build()
                .map_err(|e| format!("invalid or overly complex regex `{}`: {e}", p.query))?,
        )
    } else {
        None
    };

    let mut matches = Vec::new();
    let mut formatted = String::new();
    let mut used_chars = 0usize;
    let mut truncated = false;

    for (idx, line) in p.text.lines().enumerate() {
        let hit_byte = match &compiled_re {
            Some(re) => re.find(line).map(|m| m.start()),
            None => line.find(p.query),
        };
        let Some(match_byte) = hit_byte else { continue };
        if matches.len() >= MAX_QUERY_MATCHES {
            truncated = true;
            break;
        }
        let excerpt = excerpt_around_match(line, match_byte, MAX_LINE_EXCERPT_CHARS);
        let line_num = idx + 1;
        let rendered = format!("{line_num}: {excerpt}");
        let r_chars = rendered.chars().count();
        if used_chars + r_chars > p.max_chars && !matches.is_empty() {
            truncated = true;
            break;
        }
        if !formatted.is_empty() {
            formatted.push('\n');
            used_chars += 1;
        }
        let clipped: String = rendered.chars().take(p.max_chars.saturating_sub(used_chars)).collect();
        if clipped.chars().count() < r_chars {
            truncated = true;
        }
        used_chars += clipped.chars().count();
        formatted.push_str(&clipped);
        matches.push(json!({ "line": line_num, "text": excerpt }));
    }

    Ok(json!({
        "root": p.root.as_str(),
        "path": p.rel_path,
        "query": p.query,
        "regex": p.use_regex,
        "match_count": matches.len(),
        "matches": matches,
        "content": formatted,
        "total_chars": p.total_chars,
        "total_lines": p.total_lines,
        "truncated": truncated
    }))
}

fn excerpt_around_match(line: &str, match_byte: usize, cap_chars: usize) -> String {
    let total = line.chars().count();
    if total <= cap_chars {
        return line.to_string();
    }
    let match_char = line[..match_byte.min(line.len())].chars().count();
    let half = cap_chars / 2;
    let start = match_char.saturating_sub(half).min(total.saturating_sub(cap_chars));
    let mut out = String::new();
    if start > 0 {
        out.push_str("...");
    }
    let body: String = line.chars().skip(start).take(cap_chars).collect();
    out.push_str(&body);
    if start + cap_chars < total {
        out.push_str("...");
    }
    out
}
