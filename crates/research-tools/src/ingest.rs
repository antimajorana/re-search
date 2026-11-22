//! Multi-format document ingestion (`doc_ingest` & `ingest_bytes`) and `"source.ingested"`
//! `"raw_blob"` ledger provenance.

use crate::archive::{extract_gzip_or_tar_gz, extract_zip_bytes, ArchiveEntry, ArchivePayload};
use crate::pdf::{extract_pdf_bytes, PdfExtractStatus};
use crate::sandbox::{sanitize_rel_components, validate_session_id, SandboxRoot, WorkspaceSandbox};
pub use crate::tex_bundle::{assemble_tex_bundle, filter_tex_sections, score_tex_entrypoint, TexBundle};
use crate::ResearchEnv;
use ledger::Ledger;
use policy::EffectClass;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tools::{ApprovalScope, CallCtx, Registry, ToolDef, EVENT_SCHEMA_VERSION};

pub const DOC_INGEST: &str = "doc_ingest";
const MAX_SINGLE_TEXT_BYTES: usize = 16 * 1024 * 1024;
const MAX_INGEST_INPUT_BYTES: u64 = 32 * 1024 * 1024;
const MAX_EXCERPT_CHARS: usize = 12_000;

const TEXT_EXTENSIONS: &[&str] = &[
    "tex", "ltx", "bib", "bbl", "sty", "cls", "md", "markdown", "txt", "csv", "tsv", "json", "py",
    "log", "yml", "yaml",
];

#[derive(Debug, Clone)]
pub struct IngestRequest<'a> {
    pub session_id: &'a str,
    pub file_name: &'a str,
    pub origin: &'a str,
    pub raw_bytes: &'a [u8],
    pub source_id_hint: Option<&'a str>,
    pub section_filter: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestedSource {
    pub source_id: String,
    pub origin: String,
    pub format: String,
    pub status: String,
    pub raw_blob: String,
    pub event_seq: u64,
    pub bytes: usize,
    pub primary_path: Option<String>,
    pub assembled_path: Option<String>,
    pub files: Vec<String>,
    pub skipped_binary_entries: Vec<String>,
    pub page_count: Option<usize>,
    pub warning: Option<String>,
    pub excerpt: String,
}

pub type IngestReport = IngestedSource;

pub fn doc_ingest_def() -> ToolDef {
    ToolDef {
        id: DOC_INGEST.into(),
        description: "Ingest a `.tex`, `.bib`, `.md`, `.txt`, `.csv`, `.tsv`, `.json`, `.py`, \
                      `.pdf`, `.zip`, or `.tar.gz`/`.tgz`/`.gz` document from `workspace` or \
                      `sources` into `sessions/<id>/sources/` with TeX bundle assembly and \
                      content-addressed `source.ingested` ledger provenance."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "maxLength": 512, "description": "Relative file path in `workspace` or `sources`." },
                "root": { "type": "string", "maxLength": 16, "description": "'workspace' (default) or 'sources'." },
                "section_filter": { "type": "string", "maxLength": 128, "description": "Optional case-insensitive TeX section heading filter." }
            },
            "required": ["path"],
            "additionalProperties": false
        }),
        effect: EffectClass::ReadWorkspace,
        approval_scope: ApprovalScope::Once,
    }
}

pub fn register(registry: &mut Registry, env: Arc<ResearchEnv>) {
    registry.register(
        doc_ingest_def(),
        Arc::new(move |ctx: &CallCtx<'_>, args: &Value| handle_doc_ingest(&env, ctx, args)),
    );
}

fn handle_doc_ingest(env: &ResearchEnv, ctx: &CallCtx<'_>, args: &Value) -> Result<Value, String> {
    let raw_path = args.get("path").and_then(Value::as_str).ok_or("`path` must be a string")?;
    let root = SandboxRoot::parse_opt(args.get("root").and_then(Value::as_str)).map_err(|e| e.to_string())?;
    let section_filter = args.get("section_filter").and_then(Value::as_str).filter(|s| !s.trim().is_empty());

    let sandbox = env.sandbox();
    let resolved = sandbox
        .resolve_existing(root, ctx.session, raw_path, false)
        .map_err(|e| e.to_string())?;
    if !resolved.metadata.is_file() {
        return Err(format!("`{}` is not a regular file", resolved.rel_path));
    }

    let file = File::open(&resolved.canonical_target)
        .map_err(|e| format!("cannot open `{}`: {e}", resolved.rel_path))?;
    let mut raw_bytes = Vec::new();
    file.take(MAX_INGEST_INPUT_BYTES + 1)
        .read_to_end(&mut raw_bytes)
        .map_err(|e| format!("cannot read `{}`: {e}", resolved.rel_path))?;
    if raw_bytes.len() as u64 > MAX_INGEST_INPUT_BYTES {
        return Err(format!("`{}` exceeds 32 MiB ingestion limit", resolved.rel_path));
    }

    let origin = format!("{}:{}", root.as_str(), resolved.rel_path);
    let report = ingest_bytes(
        &sandbox,
        ctx.ledger,
        &IngestRequest {
            session_id: ctx.session,
            file_name: &resolved.rel_path,
            origin: &origin,
            raw_bytes: &raw_bytes,
            source_id_hint: None,
            section_filter,
        },
    )?;
    serde_json::to_value(&report).map_err(|e| e.to_string())
}

pub fn ingest_path(
    sandbox: &WorkspaceSandbox,
    ledger: &Mutex<Ledger>,
    session_id: &str,
    path: &Path,
    section_filter: Option<&str>,
) -> Result<IngestedSource, String> {
    let file_name = path.file_name().and_then(|s| s.to_str()).ok_or("invalid UTF-8 filename")?;
    let f = File::open(path).map_err(|e| format!("cannot open `{}`: {e}", path.display()))?;
    let mut buf = Vec::new();
    f.take(MAX_INGEST_INPUT_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("cannot read `{}`: {e}", path.display()))?;
    if buf.len() as u64 > MAX_INGEST_INPUT_BYTES {
        return Err("input file exceeds 32 MiB ingestion limit".into());
    }
    ingest_bytes(
        sandbox,
        ledger,
        &IngestRequest {
            session_id,
            file_name,
            origin: file_name,
            raw_bytes: &buf,
            source_id_hint: None,
            section_filter,
        },
    )
}

struct PreparedIngestion {
    format: String,
    status: String,
    materialized: BTreeMap<String, String>,
    primary_rel: Option<String>,
    assembled_rel: Option<String>,
    skipped_binary: Vec<String>,
    page_count: Option<usize>,
    warning: Option<String>,
    excerpt: String,
}

/// Validates and extracts `req.raw_bytes`, materializes clean UTF-8 files under
/// `sessions/<session_id>/sources/<source_id>/`, and records `put_blob` + `"source.ingested"`
/// atomically under `ledger` lock so failed inputs leave zero unreferenced blobs.
pub fn ingest_bytes(
    sandbox: &WorkspaceSandbox,
    ledger: &Mutex<Ledger>,
    req: &IngestRequest<'_>,
) -> Result<IngestedSource, String> {
    validate_session_id(req.session_id).map_err(|e| e.to_string())?;
    let source_id = build_source_id(req.source_id_hint.unwrap_or(req.file_name), req.raw_bytes);

    let prepared = prepare_ingestion(req.file_name, req.raw_bytes, req.section_filter)?;

    let canon_sources = sandbox
        .canonical_root(SandboxRoot::Sources, req.session_id, true)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "cannot initialize session sources root".to_string())?;

    let mut written_files = Vec::with_capacity(prepared.materialized.len());
    for (rel_inside, content) in &prepared.materialized {
        let full_rel = format!("{source_id}/{rel_inside}");
        let (parts, norm) = sanitize_rel_components(&full_rel, false).map_err(|e| e.to_string())?;
        WorkspaceSandbox::atomic_write_in_root(&canon_sources, &parts, content.as_bytes())
            .map_err(|e| e.to_string())?;
        written_files.push(norm);
    }

    let primary_path = prepared.primary_rel.map(|p| format!("{source_id}/{p}"));
    let assembled_path = prepared.assembled_rel.map(|p| format!("{source_id}/{p}"));

    let (raw_blob, event_seq) = {
        let guard = ledger.lock().map_err(|_| "ledger lock poisoned".to_string())?;
        let blob_hash = guard
            .put_blob(req.raw_bytes)
            .map_err(|e| format!("source.ingested put_blob failed: {e}"))?;
        let seq = guard
            .append(
                req.session_id,
                "source.ingested",
                EVENT_SCHEMA_VERSION,
                &json!({
                    "source_id": source_id,
                    "origin": req.origin,
                    "format": prepared.format,
                    "status": prepared.status,
                    "bytes": req.raw_bytes.len(),
                    "raw_blob": blob_hash,
                    "primary_path": primary_path,
                    "assembled_path": assembled_path,
                    "files": written_files,
                    "page_count": prepared.page_count,
                }),
            )
            .map_err(|e| format!("source.ingested append failed: {e}"))?;
        (blob_hash, seq)
    };

    Ok(IngestedSource {
        source_id,
        origin: req.origin.to_string(),
        format: prepared.format,
        status: prepared.status,
        raw_blob,
        event_seq,
        bytes: req.raw_bytes.len(),
        primary_path,
        assembled_path,
        files: written_files,
        skipped_binary_entries: prepared.skipped_binary,
        page_count: prepared.page_count,
        warning: prepared.warning,
        excerpt: prepared.excerpt,
    })
}

fn build_source_id(hint: &str, raw_bytes: &[u8]) -> String {
    let base = hint.rsplit('/').next().unwrap_or(hint);
    let stem = base
        .strip_suffix(".tar.gz")
        .or_else(|| base.strip_suffix(".TAR.GZ"))
        .unwrap_or_else(|| base.split('.').next().unwrap_or(base));
    let mut slug = String::new();
    for ch in stem.chars() {
        if slug.len() >= 24 {
            break;
        }
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if matches!(ch, '-' | '_') && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let clean = slug.trim_matches('-');
    let prefix = if clean.is_empty() { "doc" } else { clean };
    let short_hash = &ledger::sha256_hex(raw_bytes)[..8];
    format!("{prefix}-{short_hash}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DetectedFormat<'a> {
    Text(&'a str),
    Pdf,
    Zip,
    TarGz,
    Gzip,
}

fn detect_format(file_name: &str) -> Result<DetectedFormat<'static>, String> {
    let lower = file_name.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        return Ok(DetectedFormat::TarGz);
    }
    if lower.ends_with(".gz") {
        return Ok(DetectedFormat::Gzip);
    }
    if lower.ends_with(".zip") {
        return Ok(DetectedFormat::Zip);
    }
    if lower.ends_with(".pdf") {
        return Ok(DetectedFormat::Pdf);
    }
    let ext = lower.rsplit('.').next().unwrap_or("");
    for &allowed in TEXT_EXTENSIONS {
        if ext == allowed {
            return Ok(DetectedFormat::Text(allowed));
        }
    }
    Err(format!("unsupported document format for `{file_name}`"))
}

fn prepare_ingestion(
    file_name: &str,
    raw_bytes: &[u8],
    section_filter: Option<&str>,
) -> Result<PreparedIngestion, String> {
    match detect_format(file_name)? {
        DetectedFormat::Text(ext) => {
            if raw_bytes.len() > MAX_SINGLE_TEXT_BYTES {
                return Err(format!("text file `{file_name}` exceeds 16 MiB limit"));
            }
            if raw_bytes.contains(&0u8) {
                return Err(format!("text file `{file_name}` contains binary NUL bytes"));
            }
            let text = std::str::from_utf8(raw_bytes)
                .map_err(|_| format!("text file `{file_name}` is not valid UTF-8"))?;
            let leaf = sanitize_leaf_name(file_name, ext);
            let mut materialized = BTreeMap::new();
            let display_text = if ext == "tex" {
                if let Some(f) = section_filter {
                    let filtered = filter_tex_sections(text, f);
                    materialized.insert("__filtered__.tex".into(), filtered.clone());
                    filtered
                } else {
                    text.to_string()
                }
            } else {
                text.to_string()
            };
            materialized.insert(leaf.clone(), text.to_string());
            Ok(PreparedIngestion {
                format: ext.to_string(),
                status: "ok".into(),
                materialized,
                primary_rel: Some(leaf),
                assembled_rel: section_filter.filter(|_| ext == "tex").map(|_| "__filtered__.tex".into()),
                skipped_binary: Vec::new(),
                page_count: None,
                warning: None,
                excerpt: take_chars(&display_text, MAX_EXCERPT_CHARS),
            })
        }
        DetectedFormat::Pdf => prepare_pdf_ingestion(file_name, raw_bytes),
        DetectedFormat::Zip => {
            let entries = extract_zip_bytes(raw_bytes).map_err(|e| e.to_string())?;
            prepare_archive_files("zip", entries, section_filter)
        }
        DetectedFormat::TarGz => {
            match extract_gzip_or_tar_gz(raw_bytes, file_name, true).map_err(|e| e.to_string())? {
                ArchivePayload::Files(entries) => prepare_archive_files("tar.gz", entries, section_filter),
                _ => Err("unexpected non-tar payload in .tar.gz".into()),
            }
        }
        DetectedFormat::Gzip => {
            match extract_gzip_or_tar_gz(raw_bytes, file_name, false).map_err(|e| e.to_string())? {
                ArchivePayload::Files(entries) => prepare_archive_files("tar.gz", entries, section_filter),
                ArchivePayload::RawPdf(pdf_bytes) => {
                    let mut p = prepare_pdf_ingestion("eprint.pdf", &pdf_bytes)?;
                    if p.status == "ok" {
                        p.status = "pdf_only".into();
                    }
                    Ok(p)
                }
                ArchivePayload::SingleGzipText { suggested_name, text } => {
                    let lower = suggested_name.to_ascii_lowercase();
                    let is_tex = lower.ends_with(".tex") || lower.ends_with(".ltx");
                    let mut materialized = BTreeMap::new();
                    materialized.insert(suggested_name.clone(), text.clone());
                    let mut assembled_rel = None;
                    let display = if is_tex {
                        let (asm_name, out_tex) = match section_filter {
                            Some(f) => ("__filtered__.tex", filter_tex_sections(&text, f)),
                            None => ("__assembled__.tex", text),
                        };
                        materialized.insert(asm_name.into(), out_tex.clone());
                        assembled_rel = Some(asm_name.into());
                        out_tex
                    } else {
                        text
                    };
                    Ok(PreparedIngestion {
                        format: "gz".into(),
                        status: "ok".into(),
                        materialized,
                        primary_rel: Some(suggested_name),
                        assembled_rel,
                        skipped_binary: Vec::new(),
                        page_count: None,
                        warning: None,
                        excerpt: take_chars(&display, MAX_EXCERPT_CHARS),
                    })
                }
            }
        }
    }
}

fn prepare_pdf_ingestion(file_name: &str, pdf_bytes: &[u8]) -> Result<PreparedIngestion, String> {
    let extracted = extract_pdf_bytes(pdf_bytes).map_err(|e| e.to_string())?;
    let leaf = format!("{}.extracted.txt", sanitize_leaf_name(file_name, "pdf"));
    let mut materialized = BTreeMap::new();
    let content = if extracted.status == PdfExtractStatus::EmptyTextLayer {
        String::new()
    } else {
        extracted.text.clone()
    };
    materialized.insert(leaf.clone(), content);
    Ok(PreparedIngestion {
        format: "pdf".into(),
        status: extracted.status.as_str().into(),
        materialized,
        primary_rel: Some(leaf),
        assembled_rel: None,
        skipped_binary: Vec::new(),
        page_count: Some(extracted.page_count),
        warning: extracted.warning,
        excerpt: take_chars(&extracted.text, MAX_EXCERPT_CHARS),
    })
}

fn prepare_archive_files(
    format_label: &str,
    entries: Vec<ArchiveEntry>,
    section_filter: Option<&str>,
) -> Result<PreparedIngestion, String> {
    if entries.is_empty() {
        return Err("archive contains 0 regular files".into());
    }
    let mut materialized = BTreeMap::new();
    let mut tex_files = BTreeMap::new();
    let mut pdf_candidates = Vec::new();
    let mut skipped_binary = Vec::new();

    for entry in entries {
        let ext = entry.rel_path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        if TEXT_EXTENSIONS.contains(&ext.as_str()) && !entry.bytes.contains(&0u8) {
            let decoded = match std::str::from_utf8(&entry.bytes) {
                Ok(s) => s.to_string(),
                Err(_) if matches!(ext.as_str(), "tex" | "bib" | "bbl" | "sty" | "cls") => {
                    entry.bytes.iter().map(|&b| char::from(b)).collect()
                }
                Err(_) => {
                    skipped_binary.push(entry.rel_path);
                    continue;
                }
            };
            if ext == "tex" {
                tex_files.insert(entry.rel_path.clone(), decoded.clone());
            }
            materialized.insert(entry.rel_path, decoded);
        } else if ext == "pdf" {
            pdf_candidates.push((entry.rel_path, entry.bytes));
        } else {
            skipped_binary.push(entry.rel_path);
        }
    }

    if materialized.is_empty() {
        if let Some((pdf_name, pdf_bytes)) = pdf_candidates.into_iter().next() {
            let mut p = prepare_pdf_ingestion(&pdf_name, &pdf_bytes)?;
            p.format = format_label.into();
            if p.status == "ok" {
                p.status = "pdf_only".into();
            }
            p.skipped_binary = skipped_binary;
            return Ok(p);
        }
        return Err("archive contained no readable text, LaTeX, or PDF files".into());
    }

    let mut primary_rel = materialized.keys().next().cloned();
    let mut assembled_rel = None;
    let mut excerpt_src = String::new();

    if let Some(bundle) = assemble_tex_bundle(&tex_files) {
        primary_rel = Some(bundle.primary_path);
        let final_tex = match section_filter {
            Some(f) => filter_tex_sections(&bundle.assembled_tex, f),
            None => bundle.assembled_tex,
        };
        excerpt_src = final_tex.clone();
        materialized.insert("__assembled__.tex".into(), final_tex);
        assembled_rel = Some("__assembled__.tex".into());
    } else if let Some(first_key) = &primary_rel {
        excerpt_src = materialized.get(first_key).cloned().unwrap_or_default();
    }

    Ok(PreparedIngestion {
        format: format_label.into(),
        status: "ok".into(),
        materialized,
        primary_rel,
        assembled_rel,
        skipped_binary,
        page_count: None,
        warning: None,
        excerpt: take_chars(&excerpt_src, MAX_EXCERPT_CHARS),
    })
}

fn sanitize_leaf_name(file_name: &str, fallback_ext: &str) -> String {
    let leaf = file_name.rsplit('/').next().unwrap_or(file_name);
    sanitize_rel_components(leaf, false)
        .map(|(_, norm)| norm)
        .unwrap_or_else(|_| format!("document.{fallback_ext}"))
}

fn take_chars(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}
