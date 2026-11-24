//! Deterministic passage retrieval over the current session's recorded source files.

use crate::{resolve_sources_session_id, ResearchEnv, SandboxRoot, WorkspaceSandbox};
use ledger::{Event, Ledger};
use policy::EffectClass;
use serde::Serialize;
use serde_json::{json, Value};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tools::{ApprovalScope, CallCtx, Registry, ToolDef};

pub const SEARCH_PAPER_PASSAGES: &str = "search_paper_passages";
const MAX_QUERY_CHARS: usize = 512;
const MAX_QUERY_TERMS: usize = 32;
const MAX_SOURCE_ID_CHARS: usize = 64;
const MAX_SOURCE_PATH_BYTES: usize = 1024;
const DEFAULT_RESULTS: usize = 5;
const MAX_RESULTS: usize = 10;
const MAX_FILES: usize = 64;
const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 8 * 1024 * 1024;
const MAX_SCAN_TIME: Duration = Duration::from_secs(2);
const MAX_EXCERPT_BYTES: usize = 1800;
const MAX_EXCERPT_LINES: usize = 8;

pub fn def() -> ToolDef {
    ToolDef {
        id: SEARCH_PAPER_PASSAGES.into(),
        description: "Find exact passages relevant to a claim in this session's ingested papers. Returns source IDs, file paths, SHA-256 digests and 1-based line ranges. Treat results as candidate evidence and review surrounding context before citing. No match only describes the searched material.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "minLength": 1, "maxLength": MAX_QUERY_CHARS,
                    "description": "A claim, question or distinctive terms to look for." },
                "source_id": { "type": "string", "minLength": 1, "maxLength": MAX_SOURCE_ID_CHARS,
                    "description": "Restrict the search to one ingested source." },
                "limit": { "type": "integer", "minimum": 1, "maximum": MAX_RESULTS }
            },
            "required": ["query"],
            "additionalProperties": false
        }),
        effect: EffectClass::ReadWorkspace,
        approval_scope: ApprovalScope::Once,
    }
}

pub fn register(registry: &mut Registry, env: Arc<ResearchEnv>) {
    registry.register(
        def(),
        Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
            let ledger = ctx.ledger.lock().map_err(|_| "ledger lock poisoned")?;
            run_search(&env, &ledger, ctx.session, args)
        }),
    );
}

#[derive(Clone, Copy, Serialize)]
struct SourceFile<'a> {
    source_id: &'a str,
    title: Option<&'a str>,
    path: &'a str,
    source_sha256: &'a str,
    file_sha256: &'a str,
    source_seq: u64,
}

#[derive(Serialize)]
struct Passage<'a> {
    #[serde(flatten)]
    file: SourceFile<'a>,
    start_line: usize,
    end_line: usize,
    start_byte: usize,
    end_byte: usize,
    excerpt: String,
    matched_terms: Vec<&'a str>,
    score: usize,
}

#[derive(Default, Serialize)]
struct Coverage {
    files_searched: usize,
    bytes_searched: usize,
    matching_passages: usize,
    omissions: BTreeMap<&'static str, usize>,
    truncated: bool,
}

impl Coverage {
    fn omit(&mut self, reason: &'static str, count: usize) {
        *self.omissions.entry(reason).or_default() += count;
    }
}

pub fn run_search(
    env: &ResearchEnv,
    ledger: &Ledger,
    session: &str,
    args: &Value,
) -> Result<Value, String> {
    let session = resolve_sources_session_id(session).map_err(|e| e.to_string())?;
    tools::registry::check_args(&def().input_schema, args)?;
    let query = args["query"].as_str().unwrap_or_default().trim();
    if query.is_empty() || query.chars().count() > MAX_QUERY_CHARS {
        return Err(format!(
            "query must contain 1 to {MAX_QUERY_CHARS} characters"
        ));
    }
    let source_id = args.get("source_id").and_then(Value::as_str);
    if source_id.is_some_and(|id| !valid_source_id(id)) {
        return Err("invalid source ID".into());
    }
    let limit = match args.get("limit") {
        Some(limit) => limit
            .as_u64()
            .filter(|n| (1..=MAX_RESULTS as u64).contains(n))
            .ok_or_else(|| format!("limit must be between 1 and {MAX_RESULTS}"))?
            as usize,
        None => DEFAULT_RESULTS,
    };
    let query_lower = query.to_lowercase();
    let terms: BTreeSet<_> = words(&query_lower)
        .filter(|word| !is_stopword(word))
        .collect();
    if terms.is_empty() || terms.len() > MAX_QUERY_TERMS {
        return Err(format!(
            "query must contain between 1 and {MAX_QUERY_TERMS} distinct search terms"
        ));
    }
    let terms: Vec<_> = terms.into_iter().collect();
    let events = match ledger.read_verified_session(session) {
        Ok((_, events)) => events,
        Err(ledger::Error::UnknownSession(_)) => Vec::new(),
        Err(_) => {
            return Err("The session ledger or a recorded artifact could not be verified.".into())
        }
    };
    let mut coverage = Coverage::default();
    let files = source_files(&events, source_id, &mut coverage);
    let passages = search_files(
        &env.sandbox(),
        session,
        &files,
        &terms,
        limit,
        &mut coverage,
    );
    Ok(json!({
        "query": query,
        "status": if passages.is_empty() { "no_match" } else { "matches" },
        "passages": passages,
        "coverage": coverage,
    }))
}

fn source_files<'a>(
    events: &'a [Event],
    requested_source: Option<&str>,
    coverage: &mut Coverage,
) -> Vec<SourceFile<'a>> {
    let mut files = BTreeMap::new();
    let mut seen_sources = BTreeSet::new();
    for event in events
        .iter()
        .rev()
        .filter(|event| event.kind == "source.ingested")
    {
        let data = &event.payload;
        let source_id = data["source_id"].as_str().unwrap_or_default();
        if requested_source.is_some_and(|id| id != source_id) || !seen_sources.insert(source_id) {
            continue;
        }
        if !valid_source_id(source_id) {
            coverage.omit("unverified_receipt", 1);
            continue;
        }
        let source_prefix = format!("{source_id}/");
        let Some(paths) = data["files"].as_array() else {
            coverage.omit("unverified_receipt", 1);
            continue;
        };
        for path in paths {
            let Some(path) = path.as_str() else {
                coverage.omit("unverified_file", 1);
                continue;
            };
            if data["assembled_path"].as_str() == Some(path) {
                coverage.omit("generated_view", 1);
                continue;
            }
            let extension = path
                .rsplit_once('.')
                .map(|(_, ext)| ext.to_ascii_lowercase());
            if !matches!(
                extension.as_deref(),
                Some("txt" | "tex" | "ltx" | "md" | "markdown")
            ) {
                coverage.omit("unsupported_file", 1);
                continue;
            }
            if path.len() > MAX_SOURCE_PATH_BYTES {
                coverage.omit("path_size_limit", 1);
                coverage.truncated = true;
                continue;
            }
            let (Some(source_sha256), Some(file_sha256)) = (
                digest(&data["raw_blob"]),
                digest(&data["files_sha256"][path]),
            ) else {
                coverage.omit("unverified_file", 1);
                continue;
            };
            if !path.starts_with(&source_prefix) {
                coverage.omit("unverified_file", 1);
                continue;
            }
            let primary = data["primary_path"].as_str() == Some(path);
            files
                .entry((!primary, event.seq, path))
                .or_insert(SourceFile {
                    source_id,
                    title: data["title"].as_str().filter(|title| title.len() <= 960),
                    path,
                    source_sha256,
                    file_sha256,
                    source_seq: event.seq,
                });
            if files.len() > MAX_FILES {
                files.pop_last();
                coverage.omit("file_limit", 1);
                coverage.truncated = true;
            }
        }
    }
    if requested_source.is_some() && seen_sources.is_empty() {
        coverage.omit("source_not_ingested", 1);
    }
    files.into_values().collect()
}

fn valid_source_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_SOURCE_ID_CHARS
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn digest(value: &Value) -> Option<&str> {
    value.as_str().filter(|value| {
        value.len() == 64
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

fn load_file(
    sandbox: &WorkspaceSandbox,
    session: &str,
    file: SourceFile<'_>,
    remaining_bytes: &mut usize,
) -> Result<String, &'static str> {
    let resolved = sandbox
        .resolve_existing(SandboxRoot::Sources, session, file.path, false)
        .map_err(|_| "unavailable_file")?;
    if !resolved.metadata.is_file() || resolved.rel_path != file.path {
        return Err("unavailable_file");
    }
    if resolved.metadata.len() > MAX_FILE_BYTES as u64 {
        return Err("file_size_limit");
    }
    if resolved.metadata.len() >= *remaining_bytes as u64 {
        return Err("byte_limit");
    }
    let byte_limit = (MAX_FILE_BYTES + 1).min(*remaining_bytes);
    let input = File::open(&resolved.canonical_target).map_err(|_| "unavailable_file")?;
    let mut bytes = Vec::new();
    let read_result = input.take(byte_limit as u64).read_to_end(&mut bytes);
    *remaining_bytes -= bytes.len();
    read_result.map_err(|_| "unavailable_file")?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err("file_size_limit");
    }
    if ledger::sha256_hex(&bytes) != file.file_sha256
        || bytes.len() as u64 != resolved.metadata.len()
    {
        return Err("hash_mismatch");
    }
    String::from_utf8(bytes).map_err(|_| "invalid_text")
}

fn search_files<'a>(
    sandbox: &WorkspaceSandbox,
    session: &str,
    files: &[SourceFile<'a>],
    terms: &[&'a str],
    limit: usize,
    coverage: &mut Coverage,
) -> Vec<Passage<'a>> {
    let mut ranked = Vec::new();
    let deadline = Instant::now() + MAX_SCAN_TIME;
    let mut remaining_bytes = MAX_TOTAL_BYTES;
    for (index, &file) in files.iter().enumerate() {
        if Instant::now() >= deadline {
            coverage.omit("time_limit", files.len() - index);
            coverage.truncated = true;
            break;
        }
        let text = match load_file(sandbox, session, file, &mut remaining_bytes) {
            Ok(text) if !text.trim().is_empty() => text,
            Ok(_) => {
                coverage.omit("empty_text", 1);
                continue;
            }
            Err(reason) => {
                coverage.omit(reason, 1);
                coverage.truncated |= matches!(reason, "byte_limit" | "file_size_limit");
                continue;
            }
        };
        coverage.files_searched += 1;
        let mut scan = 0;
        let mut line = 1;
        while scan < text.len() && Instant::now() < deadline {
            let excerpt = next_excerpt(&text[scan..]);
            let breaks = excerpt.bytes().filter(|&byte| byte == b'\n').count();
            let mut counts = [0usize; MAX_QUERY_TERMS];
            for word in words(&excerpt.to_lowercase()) {
                if let Ok(term) = terms.binary_search(&word) {
                    counts[term] = (counts[term] + 1).min(3);
                }
            }
            let distinct = counts.iter().filter(|&&count| count > 0).count();
            let score = distinct * 100 + counts.iter().sum::<usize>();
            if distinct > 0 {
                coverage.matching_passages += 1;
                if ranked.len() < limit
                    || score
                        > ranked
                            .last()
                            .map_or(0, |passage: &Passage<'_>| passage.score)
                {
                    ranked.push(Passage {
                        file,
                        start_line: line,
                        end_line: line + breaks - usize::from(excerpt.ends_with('\n')),
                        start_byte: scan,
                        end_byte: scan + excerpt.len(),
                        excerpt: excerpt.to_string(),
                        matched_terms: terms
                            .iter()
                            .zip(counts)
                            .filter_map(|(&term, count)| (count > 0).then_some(term))
                            .collect(),
                        score,
                    });
                    ranked.sort_by_key(|passage| Reverse(passage.score));
                    ranked.truncate(limit);
                }
            }
            line += breaks;
            scan += excerpt.len();
        }
        coverage.bytes_searched += scan;
        if scan < text.len() {
            coverage.omit("time_limit", files.len() - index);
            coverage.truncated = true;
            break;
        }
    }
    coverage.truncated |= coverage.matching_passages > ranked.len();
    ranked
}

/// Return a source slice with its original line endings.
fn next_excerpt(text: &str) -> &str {
    let cap = text.floor_char_boundary(text.len().min(MAX_EXCERPT_BYTES));
    let block = &text[..cap];
    let mut end = 0;
    let mut has_content = false;
    for line in block.split_inclusive('\n').take(MAX_EXCERPT_LINES) {
        end += line.len();
        if line.trim().is_empty() && line.ends_with('\n') && has_content {
            break;
        }
        has_content |= !line.trim().is_empty();
    }
    if end == cap && end < text.len() && !block.ends_with('\n') {
        // Keep oversized physical lines searchable without dropping their suffix.
        if let Some((offset, whitespace)) = block
            .char_indices()
            .rev()
            .find(|(_, ch)| ch.is_whitespace())
        {
            if offset >= cap / 2 {
                end = offset + whitespace.len_utf8();
            }
        }
    }
    &text[..end]
}

fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| !word.is_empty())
}

fn is_stopword(word: &str) -> bool {
    matches!(
        word,
        "a" | "an"
            | "and"
            | "are"
            | "as"
            | "at"
            | "be"
            | "by"
            | "can"
            | "did"
            | "do"
            | "does"
            | "for"
            | "from"
            | "has"
            | "have"
            | "how"
            | "i"
            | "in"
            | "is"
            | "it"
            | "of"
            | "on"
            | "or"
            | "that"
            | "the"
            | "their"
            | "this"
            | "to"
            | "was"
            | "we"
            | "were"
            | "what"
            | "when"
            | "where"
            | "which"
            | "who"
            | "why"
            | "will"
            | "with"
            | "would"
            | "you"
            | "your"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ingest_bytes, IngestRequest, IngestedSource};
    use std::sync::Mutex;

    const SESSION: &str = "s-passages";

    fn ingest(env: &ResearchEnv, ledger: &Mutex<Ledger>, name: &str, text: &str) -> IngestedSource {
        ingest_bytes(
            &env.sandbox(),
            ledger,
            &IngestRequest {
                session_id: SESSION,
                file_name: name,
                origin: "test attachment",
                raw_bytes: text.as_bytes(),
                source_id_hint: None,
                section_filter: None,
                title: Some(name),
            },
        )
        .unwrap()
    }

    #[test]
    fn ranks_across_ingested_sources_and_returns_exact_utf8_locations() {
        let dir = tempfile::tempdir().unwrap();
        let env = ResearchEnv::new(dir.path());
        let ledger = Mutex::new(Ledger::open(dir.path()).unwrap());
        let weak = ingest(&env, &ledger, "weak.md", "Entropy appears here.\n");
        let raw = "Introductory section.\r\n\r\nSpectral entropy with α=2 quantifies the spread.\r\nThe result applies in the continuum limit.\r\n";
        let strong = ingest(&env, &ledger, "strong.txt", raw);
        let ledger = ledger.into_inner().unwrap();

        let query = "How does spectral entropy change with α?";
        let result = run_search(&env, &ledger, SESSION, &json!({ "query": query })).unwrap();
        let hits = result["passages"].as_array().unwrap();
        assert_eq!(hits.len(), 2);
        let best = &hits[0];
        assert_eq!(best["source_id"], strong.source_id);
        assert_eq!(best["path"], strong.primary_path.unwrap());
        assert_eq!(best["title"], "strong.txt");
        assert_eq!(best["source_seq"], strong.event_seq);
        assert_eq!(best["file_sha256"], ledger::sha256_hex(raw.as_bytes()));
        assert_eq!(best["source_sha256"], strong.raw_blob);
        assert_eq!(best["start_line"], 3);
        assert_eq!(best["end_line"], 4);
        let start = best["start_byte"].as_u64().unwrap() as usize;
        let end = best["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(best["excerpt"], &raw[start..end]);
        assert_eq!(best["matched_terms"], json!(["entropy", "spectral", "α"]));

        let filtered = run_search(
            &env,
            &ledger,
            "sidetalk:s-passages",
            &json!({
                "query": query, "source_id": weak.source_id, "limit": 1
            }),
        )
        .unwrap();
        assert_eq!(filtered["passages"][0]["source_id"], weak.source_id);
        assert_eq!(filtered["coverage"]["files_searched"], 1);
        let fresh = run_search(&env, &ledger, "s-fresh", &json!({ "query": query })).unwrap();
        assert_eq!(fresh["status"], "no_match");
        assert_eq!(fresh["coverage"]["files_searched"], 0);
    }

    #[test]
    fn omits_unrecorded_legacy_changed_and_oversized_files() {
        let dir = tempfile::tempdir().unwrap();
        let env = ResearchEnv::new(dir.path());
        let ledger = Mutex::new(Ledger::open(dir.path()).unwrap());
        let changed = ingest(&env, &ledger, "changed.txt", "Original calculation.\n");
        ingest(&env, &ledger, "large.txt", &"x".repeat(MAX_FILE_BYTES + 1));
        let sources = dir.path().join("sessions").join(SESSION).join("sources");
        std::fs::write(
            sources.join(changed.primary_path.unwrap()),
            "Injected matching term.\n",
        )
        .unwrap();
        std::fs::create_dir(sources.join("legacy")).unwrap();
        std::fs::write(
            sources.join("legacy/doc.txt"),
            "Matching term in an unverified old receipt.",
        )
        .unwrap();
        std::fs::write(
            sources.join("unrecorded.txt"),
            "Matching term without a receipt.",
        )
        .unwrap();
        let ledger = ledger.into_inner().unwrap();
        let legacy_raw = ledger.put_blob(b"Legacy").unwrap();
        ledger
            .append_with_hash(
                SESSION,
                "source.ingested",
                1,
                &json!({
                    "source_id": "legacy", "raw_blob": legacy_raw, "files": ["legacy/doc.txt"]
                }),
            )
            .unwrap();

        let result =
            run_search(&env, &ledger, SESSION, &json!({ "query": "matching term" })).unwrap();
        assert_eq!(result["status"], "no_match");
        assert_eq!(result["passages"], json!([]));
        assert_eq!(
            result["coverage"]["omissions"],
            json!({
                "file_size_limit": 1, "hash_mismatch": 1, "unverified_file": 1
            })
        );
        assert_eq!(result["coverage"]["files_searched"], 0);
        assert_eq!(result["coverage"]["truncated"], true);
    }

    #[test]
    fn oversized_multibyte_line_is_searchable_at_its_end() {
        let dir = tempfile::tempdir().unwrap();
        let env = ResearchEnv::new(dir.path());
        let ledger = Mutex::new(Ledger::open(dir.path()).unwrap());
        let raw = format!("{} rareterm\n", "α".repeat(MAX_EXCERPT_BYTES));
        ingest(&env, &ledger, "long.tex", &raw);
        let result = run_search(
            &env,
            &ledger.into_inner().unwrap(),
            SESSION,
            &json!({
                "query": "rareterm"
            }),
        )
        .unwrap();
        let hit = &result["passages"][0];
        assert_eq!(hit["start_line"], 1);
        assert_eq!(hit["end_line"], 1);
        let start = hit["start_byte"].as_u64().unwrap() as usize;
        let end = hit["end_byte"].as_u64().unwrap() as usize;
        assert_eq!(&raw[start..end], hit["excerpt"].as_str().unwrap());
        assert_eq!(end, raw.len());
    }
}
