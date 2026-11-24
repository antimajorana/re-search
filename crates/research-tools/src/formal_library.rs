//! Bounded lexical search of Physlib Lean sources in an attached workspace.

use crate::sandbox::{
    is_symlink_or_junction, sanitize_rel_components, SandboxRoot, WorkspaceSandbox,
};
use crate::ResearchEnv;
use policy::EffectClass;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tools::{ApprovalScope, CallCtx, Registry, ToolDef};

pub const SEARCH_FORMAL_LIBRARY: &str = "search_formal_library";

const MAX_QUERY_CHARS: usize = 160;
const MAX_RESULTS: usize = 20;
const MAX_ENTRIES: usize = 25_000;
const MAX_FILES: usize = 4_000;
const MAX_FILE_BYTES: usize = 512 * 1024;
const MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;
const MAX_DEPTH: usize = 30;
const MAX_PATH_BYTES: usize = 2_000;
const MAX_NAME_BYTES: usize = 384;
const MAX_SIGNATURE_BYTES: usize = 2_000;
const ROOTS: &[&str] = &[
    "Physlib.lean",
    "Physlib",
    ".lake/packages/physlib/Physlib.lean",
    ".lake/packages/physlib/Physlib",
    ".lake/packages/Physlib/Physlib.lean",
    ".lake/packages/Physlib/Physlib",
];

pub fn search_formal_library_def() -> ToolDef {
    ToolDef {
        id: SEARCH_FORMAL_LIBRARY.into(),
        description: concat!(
            "Search named Lean declarations in the attached workspace’s Physlib source. ",
            "Results contain lexical signatures and source locations. Use Lean to check an intended application."
        )
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "minLength": 1, "maxLength": MAX_QUERY_CHARS },
                "max_results": { "type": "integer", "minimum": 1, "maximum": MAX_RESULTS }
            },
            "required": ["query"],
            "additionalProperties": false
        }),
        effect: EffectClass::ReadWorkspace,
        approval_scope: ApprovalScope::Once,
    }
}

pub fn register_formal_library(registry: &mut Registry, env: Arc<ResearchEnv>) {
    registry.register(
        search_formal_library_def(),
        Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
            handle_search_formal_library(&env, ctx.session, args)
        }),
    );
}

pub fn handle_search_formal_library(
    env: &ResearchEnv,
    session: &str,
    args: &Value,
) -> Result<Value, String> {
    tools::registry::check_args(&search_formal_library_def().input_schema, args)?;
    let query = args["query"].as_str().unwrap_or_default().trim();
    if query.is_empty()
        || query.chars().count() > MAX_QUERY_CHARS
        || query.chars().any(char::is_control)
    {
        return Err("Enter a Lean name or keyword to search.".into());
    }
    let limit = args["max_results"]
        .as_u64()
        .unwrap_or(10)
        .clamp(1, MAX_RESULTS as u64) as usize;
    let sandbox = env.sandbox();
    let workspace = sandbox
        .canonical_root(SandboxRoot::Workspace, session, false)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Attach a Lean workspace to search Physlib.".to_string())?;

    let mut scan = Scan::new(&workspace, query, limit);
    let mut roots = Vec::new();
    for rel in ROOTS {
        if let Some(root) = plain_root(&workspace, rel) {
            if !roots.iter().any(|(_, path)| path == &root) {
                roots.push((*rel, root));
            }
        }
    }
    for (_, root) in &roots {
        scan.walk(root, root, 0);
        if scan.exhausted() {
            break;
        }
    }

    let status = if roots.is_empty() {
        "library_not_found"
    } else {
        "lexical_unverified"
    };
    let count = scan.hits.len();
    Ok(json!({
        "query": query,
        "status": status,
        "roots": roots.iter().map(|(rel, _)| *rel).collect::<Vec<_>>(),
        "count": count,
        "results": scan.hits,
        "files_scanned": scan.files - scan.skipped_files,
        "skipped_files": scan.skipped_files,
        "scan_truncated": scan.scan_truncated,
        "results_truncated": scan.matches > count,
    }))
}

/// Resolve each fixed path segment without following links, including .lake and its package.
fn plain_root(workspace: &Path, rel: &str) -> Option<PathBuf> {
    let mut current = workspace.to_path_buf();
    for segment in rel.split('/') {
        current.push(segment);
        let metadata = fs::symlink_metadata(&current).ok()?;
        if is_symlink_or_junction(&current, &metadata) {
            return None;
        }
    }
    let metadata = fs::symlink_metadata(&current).ok()?;
    if (rel.ends_with(".lean") && !metadata.is_file())
        || (!rel.ends_with(".lean") && !metadata.is_dir())
    {
        return None;
    }
    let canonical = fs::canonicalize(current).ok()?;
    WorkspaceSandbox::is_same_or_inside(workspace, &canonical).then_some(canonical)
}

#[derive(Serialize)]
struct Hit {
    name: String,
    qualified_name: Option<String>,
    namespace: Option<String>,
    kind: String,
    visibility: &'static str,
    module: String,
    path: String,
    line: usize,
    signature: String,
    signature_truncated: bool,
    source_sha256: String,
    status: &'static str,
    #[serde(skip)]
    rank: u8,
}

struct Scan<'a> {
    workspace: &'a Path,
    query: &'a str,
    folded: String,
    terms: Vec<String>,
    limit: usize,
    entries: usize,
    files: usize,
    bytes: usize,
    matches: usize,
    skipped_files: usize,
    scan_truncated: bool,
    hits: Vec<Hit>,
    seen_files: HashSet<PathBuf>,
}

impl<'a> Scan<'a> {
    fn new(workspace: &'a Path, query: &'a str, limit: usize) -> Self {
        let folded = query.to_lowercase();
        let terms = folded
            .split(|ch: char| !ch.is_alphanumeric() && ch != '\'')
            .filter(|word| !word.is_empty())
            .map(str::to_string)
            .collect();
        Self {
            workspace,
            query,
            folded,
            terms,
            limit,
            entries: 0,
            files: 0,
            bytes: 0,
            matches: 0,
            skipped_files: 0,
            scan_truncated: false,
            hits: Vec::new(),
            seen_files: HashSet::new(),
        }
    }

    fn exhausted(&mut self) -> bool {
        let reached = self.entries >= MAX_ENTRIES || self.file_budget_exhausted();
        self.scan_truncated |= reached;
        reached
    }

    fn file_budget_exhausted(&self) -> bool {
        self.files >= MAX_FILES || self.bytes >= MAX_TOTAL_BYTES
    }

    fn walk(&mut self, library: &Path, path: &Path, depth: usize) {
        if self.file_budget_exhausted() {
            self.scan_truncated = true;
            return;
        }
        let Ok(metadata) = fs::symlink_metadata(path) else {
            self.scan_truncated = true;
            return;
        };
        if is_symlink_or_junction(path, &metadata) {
            return;
        }
        let Ok(canonical) = fs::canonicalize(path) else {
            self.scan_truncated = true;
            return;
        };
        if !WorkspaceSandbox::is_same_or_inside(library, &canonical) {
            return;
        }
        if metadata.is_dir() {
            if depth >= MAX_DEPTH || self.entries >= MAX_ENTRIES {
                self.scan_truncated = true;
                return;
            }
            let Ok(entries) = fs::read_dir(&canonical) else {
                self.scan_truncated = true;
                return;
            };
            let mut children = Vec::new();
            for entry in entries {
                if self.entries == MAX_ENTRIES {
                    self.scan_truncated = true;
                    break;
                }
                self.entries += 1;
                match entry {
                    Ok(e)
                        if e.file_name()
                            .to_str()
                            .is_some_and(|name| !name.starts_with('.')) =>
                    {
                        children.push(e.path());
                    }
                    Ok(_) => {}
                    Err(_) => self.scan_truncated = true,
                }
            }
            children.sort();
            for child in children {
                self.walk(library, &child, depth + 1);
                if self.file_budget_exhausted() {
                    self.scan_truncated = true;
                    break;
                }
            }
        } else if metadata.is_file()
            && canonical
                .extension()
                .is_some_and(|extension| extension == "lean")
            && self.seen_files.insert(canonical.clone())
        {
            let available = (MAX_TOTAL_BYTES - self.bytes).min(MAX_FILE_BYTES);
            self.scan_file(library, &canonical, available, metadata.len());
        }
    }

    fn scan_file(&mut self, library: &Path, file: &Path, available: usize, length: u64) {
        self.files += 1;
        if length > available as u64 {
            self.skipped_files += 1;
            self.scan_truncated = true;
            return;
        }
        let source = match read_source(file, available) {
            Ok(source) => source,
            Err(_) => {
                self.skipped_files += 1;
                self.scan_truncated = true;
                return;
            }
        };
        self.bytes += source.len();
        let Some(path) = slash_path(file.strip_prefix(self.workspace).ok()) else {
            self.skipped_files += 1;
            self.scan_truncated = true;
            return;
        };
        let module = if library == file {
            "Physlib".to_string()
        } else {
            let Some(relative) = slash_path(file.strip_prefix(library).ok()) else {
                self.skipped_files += 1;
                self.scan_truncated = true;
                return;
            };
            let suffix = relative.strip_suffix(".lean").unwrap_or(&relative);
            format!("Physlib.{}", suffix.replace('/', "."))
        };
        let initial_haystack = source.to_lowercase();
        let module_search = module.to_lowercase();
        if !self
            .terms
            .iter()
            .all(|term| initial_haystack.contains(term) || module_search.contains(term))
        {
            return;
        }
        let mut source_hash = None;
        search_declarations(&source, |declaration| {
            let Some(rank) = self.rank(&declaration, &module_search) else {
                return;
            };
            self.matches += 1;
            let hit = Hit {
                name: declaration.name.into(),
                qualified_name: declaration.qualified_name,
                namespace: declaration.namespace,
                kind: declaration.kind.into(),
                visibility: declaration.visibility,
                module: module.clone(),
                path: path.clone(),
                line: declaration.line,
                signature: declaration.signature.into(),
                signature_truncated: declaration.signature_truncated,
                source_sha256: source_hash
                    .get_or_insert_with(|| ledger::sha256_hex(source.as_bytes()))
                    .clone(),
                status: "lexical_unverified",
                rank,
            };
            let index = self.hits.partition_point(|stored| {
                (stored.rank, stored.path.as_str(), stored.line)
                    <= (hit.rank, hit.path.as_str(), hit.line)
            });
            if index < self.limit {
                self.hits.insert(index, hit);
                self.hits.truncate(self.limit);
            }
        });
    }

    fn rank(&self, decl: &Declaration<'_>, module: &str) -> Option<u8> {
        let full_name = decl.qualified_name.as_deref().unwrap_or(decl.name);
        if full_name == self.query || decl.name == self.query {
            return Some(0);
        }
        let name = decl.name.to_lowercase();
        let full_name = full_name.to_lowercase();
        if full_name == self.folded || name == self.folded {
            return Some(1);
        }
        if name.contains(&self.folded) || full_name.contains(&self.folded) {
            return Some(2);
        }
        let signature = decl.signature.to_lowercase();
        (!self.terms.is_empty()
            && self.terms.iter().all(|term| {
                full_name.contains(term) || module.contains(term) || signature.contains(term)
            }))
        .then_some(3)
    }
}

fn slash_path(path: Option<&Path>) -> Option<String> {
    let names: Vec<_> = path?
        .components()
        .map(|part| part.as_os_str().to_str())
        .collect::<Option<_>>()?;
    let joined = names.join("/");
    if joined.len() > MAX_PATH_BYTES {
        return None;
    }
    sanitize_rel_components(&joined, false)
        .ok()
        .map(|(_, path)| path)
}

fn read_source(path: &Path, limit: usize) -> io::Result<String> {
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit || bytes.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid Lean source size or bytes",
        ));
    }
    String::from_utf8(bytes).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

#[derive(Clone, Copy)]
struct Token<'a> {
    text: &'a str,
    start: usize,
    end: usize,
    line: usize,
    column: usize,
}

fn name_end(source: &str, pos: usize) -> Option<usize> {
    fn component_end(source: &str, pos: usize) -> Option<usize> {
        let text = &source[pos..];
        if let Some(rest) = text.strip_prefix('«') {
            let end = rest.find(['»', '\n'])?;
            return rest[end..]
                .starts_with('»')
                .then(|| pos + '«'.len_utf8() + end + '»'.len_utf8());
        }
        let first = text.chars().next()?;
        if !(first.is_alphabetic() || first == '_') {
            return None;
        }
        let mut end = pos + first.len_utf8();
        for ch in source[end..].chars() {
            if !(ch.is_alphanumeric() || matches!(ch, '_' | '\'' | '\u{0300}'..='\u{036f}')) {
                break;
            }
            end += ch.len_utf8();
        }
        Some(end)
    }
    let mut end = component_end(source, pos)?;
    while source.as_bytes().get(end) == Some(&b'.') {
        let Some(next) = component_end(source, end + 1) else {
            break;
        };
        end = next;
    }
    Some(end)
}

fn lex(source: &str) -> Vec<Token<'_>> {
    let mut out = Vec::new();
    let (mut start, mut line, mut line_start) = (0, 1, 0);
    while start < source.len() {
        let text = &source[start..];
        let first = text.chars().next().unwrap();
        let (end, keep) = if first.is_whitespace() {
            (start + first.len_utf8(), false)
        } else if text.starts_with("--") {
            (start + text.find('\n').unwrap_or(text.len()), false)
        } else if text.starts_with("/-") {
            (start + block_comment_len(text), false)
        } else if let Some(len) = literal_len(text) {
            (start + len, true)
        } else if text.starts_with(":=") || text.starts_with("@[") {
            (start + 2, true)
        } else {
            (
                name_end(source, start).unwrap_or(start + first.len_utf8()),
                true,
            )
        };
        if keep {
            out.push(Token {
                text: &source[start..end],
                start,
                end,
                line,
                column: start - line_start,
            });
        }
        let consumed = &source.as_bytes()[start..end];
        for (i, byte) in consumed.iter().enumerate() {
            if *byte == b'\n' {
                line += 1;
                line_start = start + i + 1;
            }
        }
        start = end;
    }
    out
}

fn block_comment_len(text: &str) -> usize {
    let bytes = text.as_bytes();
    let (mut depth, mut cursor) = (1, 2);
    while cursor < bytes.len() && depth != 0 {
        match bytes.get(cursor..cursor + 2) {
            Some(b"/-") => {
                depth += 1;
                cursor += 2;
            }
            Some(b"-/") => {
                depth -= 1;
                cursor += 2;
            }
            _ => cursor += 1,
        }
    }
    cursor
}

fn literal_len(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    if let Some(after_r) = text.strip_prefix('r') {
        let hashes = after_r.bytes().take_while(|&b| b == b'#').count();
        let opening = 1 + hashes;
        if bytes.get(opening) == Some(&b'"') {
            let suffix = format!("\"{}", "#".repeat(hashes));
            return Some(
                text[opening + 1..]
                    .find(&suffix)
                    .map_or(text.len(), |i| opening + 1 + i + suffix.len()),
            );
        }
    }
    if bytes.first() == Some(&b'\'') {
        let char_start = if bytes.get(1) == Some(&b'\\') { 2 } else { 1 };
        let length = text.get(char_start..)?.chars().next()?.len_utf8();
        return (bytes.get(char_start + length) == Some(&b'\'')).then_some(char_start + length + 1);
    }
    if bytes.first() != Some(&b'"') {
        return None;
    }
    let mut escaped = false;
    for (i, byte) in bytes.iter().enumerate().skip(1) {
        if *byte == b'"' && !escaped {
            return Some(i + 1);
        }
        escaped = *byte == b'\\' && !escaped;
    }
    Some(text.len())
}

fn depth_change(token: &str) -> isize {
    match token {
        "(" | "[" | "@[" | "{" | "⟨" => 1,
        ")" | "]" | "}" | "⟩" => -1,
        _ => 0,
    }
}

fn command_intro(tokens: &[Token<'_>], mut i: usize) -> (usize, &'static str) {
    let mut visibility = "public";
    while let Some(token) = tokens.get(i) {
        match token.text {
            "@[" => {
                let mut depth = 1;
                i += 1;
                while depth > 0 {
                    let Some(attr) = tokens.get(i) else { break };
                    depth += depth_change(attr.text);
                    i += 1;
                }
                continue;
            }
            "private" => visibility = "private",
            "public" => visibility = "public",
            "local" => visibility = "local",
            "protected" => visibility = "protected",
            "scoped" | "unsafe" | "partial" | "noncomputable" => {}
            _ => break,
        }
        i += 1;
    }
    (i, visibility)
}

const DECLARATIONS: &[&str] = &[
    "theorem",
    "lemma",
    "def",
    "abbrev",
    "opaque",
    "axiom",
    "constant",
    "structure",
    "class",
    "inductive",
    "instance",
];

struct Scope<'a> {
    name: Option<&'a str>,
    namespace: bool,
}

fn lexical_namespace(scopes: &[Scope<'_>]) -> Option<String> {
    if scopes.len() > MAX_DEPTH {
        return None;
    }
    let mut namespace = String::new();
    for scope in scopes.iter().filter(|scope| scope.namespace) {
        let name = scope.name?;
        if namespace.len() + name.len() >= MAX_NAME_BYTES {
            return None;
        }
        if !namespace.is_empty() {
            namespace.push('.');
        }
        namespace.push_str(name);
    }
    Some(namespace)
}

struct Declaration<'a> {
    name: &'a str,
    qualified_name: Option<String>,
    namespace: Option<String>,
    kind: &'a str,
    visibility: &'static str,
    line: usize,
    signature: &'a str,
    signature_truncated: bool,
}

fn search_declarations(source: &str, mut found: impl FnMut(Declaration<'_>)) {
    let tokens = lex(source);
    let mut scopes: Vec<Scope<'_>> = Vec::new();
    let (mut previous_line, mut depth, mut last_intro) = (0, 0, None);
    let mut scope_known = true;
    for (i, token) in tokens.iter().enumerate() {
        let starts_line = token.line != previous_line;
        previous_line = token.line;
        let at_command = starts_line && depth == 0;
        depth = (depth + depth_change(token.text)).max(0);
        if !at_command {
            continue;
        }
        let (intro, visibility) = command_intro(&tokens, i);
        let Some(kind) = tokens.get(intro) else {
            continue;
        };
        if last_intro == Some(intro) {
            continue;
        }
        last_intro = Some(intro);
        let named = tokens
            .get(intro + 1)
            .filter(|next| name_end(next.text, 0) == Some(next.text.len()));
        match kind.text {
            "namespace" | "section" | "mutual" => {
                let name = named
                    .filter(|next| next.line == kind.line)
                    .map(|next| next.text);
                scope_known &= kind.text != "namespace" || name.is_some();
                scopes.push(Scope {
                    name,
                    namespace: kind.text == "namespace",
                });
            }
            "end" => {
                let name = named
                    .filter(|next| next.line == kind.line)
                    .map(|next| next.text);
                scope_known &= scopes.pop().is_some_and(|scope| {
                    name.is_none_or(|name| {
                        scope.name.is_some_and(|scope| {
                            scope == name || scope.ends_with(&format!(".{name}"))
                        })
                    })
                });
            }
            declaration if DECLARATIONS.contains(&declaration) => {
                let Some(name) = named else { continue };
                if name.line != kind.line
                    || name.text.len() > MAX_NAME_BYTES
                    || name.end - kind.start > MAX_SIGNATURE_BYTES
                    || (declaration == "instance" && name.text == "where")
                {
                    continue;
                }
                let namespace = scope_known.then(|| lexical_namespace(&scopes)).flatten();
                let qualified_name = if matches!(visibility, "private" | "local") {
                    None
                } else if let Some(rooted) = name.text.strip_prefix("_root_.") {
                    Some(rooted.to_string())
                } else {
                    namespace.as_ref().and_then(|namespace| {
                        if namespace.is_empty() {
                            Some(name.text.to_string())
                        } else if namespace.len() + name.text.len() < MAX_NAME_BYTES {
                            Some(format!("{namespace}.{}", name.text))
                        } else {
                            None
                        }
                    })
                };
                let full_prefix =
                    token.line == kind.line && name.end - token.start <= MAX_SIGNATURE_BYTES;
                let start = if full_prefix { token.start } else { kind.start };
                let (end, signature_truncated) =
                    signature_end(&tokens, intro + 1, start, token.column);
                found(Declaration {
                    name: name.text,
                    qualified_name,
                    namespace: namespace.filter(|name| !name.is_empty()),
                    kind: declaration,
                    visibility,
                    line: kind.line,
                    signature: source[start..end].trim_end(),
                    signature_truncated: signature_truncated
                        || (token.line == kind.line && !full_prefix),
                });
            }
            _ => {}
        }
    }
}

fn signature_end(tokens: &[Token<'_>], name: usize, start: usize, column: usize) -> (usize, bool) {
    let mut previous = tokens[name];
    let mut depth = 0;
    for (i, token) in tokens.iter().enumerate().skip(name + 1) {
        if depth == 0 && matches!(token.text, ":=" | "where" | "|") {
            return (previous.end, false);
        }
        if depth == 0 && token.line != previous.line && token.column <= column {
            let (intro, _) = command_intro(tokens, i);
            let next_command = tokens.get(intro).is_some_and(|head| {
                DECLARATIONS.contains(&head.text)
                    || matches!(
                        head.text,
                        "namespace"
                            | "section"
                            | "end"
                            | "open"
                            | "variable"
                            | "attribute"
                            | "set_option"
                            | "mutual"
                    )
                    || head.text.starts_with('#')
            });
            return (previous.end, !next_command);
        }
        if token.end - start > MAX_SIGNATURE_BYTES {
            return (previous.end, true);
        }
        depth = (depth + depth_change(token.text)).max(0);
        previous = *token;
    }
    (previous.end, depth != 0)
}
