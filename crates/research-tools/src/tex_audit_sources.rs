//! Source file walker and session ledger/JSON withdrawal & retraction scanners for `tex_audit`.

use crate::sandbox::{is_symlink_or_junction, strip_verbatim_prefix, WorkspaceSandbox};
use crate::tex_bundle::{find_tex_includes_in_line_block, normalize_rel_segments};
use crate::tex_scan::{base_arxiv_id, normalize_doi, normalize_ws, strip_latex_comments};
use ledger::Ledger;
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

const MAX_SCANNED_FILES: usize = 512;
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

pub(crate) struct LoadedFile {
    pub display_id: String,
    pub ext: String,
    pub from_sources_dir: bool,
    pub raw: String,
    pub stripped_tex: String,
    pub norm_raw: String,
    pub norm_stripped: String,
}

/// Resolve the files used by a selected manuscript. The lookup is confined to
/// the files already loaded from its workspace or ingested source bundle.
pub(crate) fn manuscript_files(files: &[LoadedFile], root: &LoadedFile) -> BTreeSet<String> {
    static BIB: OnceLock<Regex> = OnceLock::new();
    let bib = BIB.get_or_init(|| {
        Regex::new(r"\\(?:bibliography|addbibresource)(?:\s*\[[^\]]*\])?\s*\{([^{}]+)\}").unwrap()
    });
    let bundle = if root.from_sources_dir {
        root.display_id.splitn(3, '/').take(2).collect::<Vec<_>>().join("/")
    } else {
        String::new()
    };
    let mut selected = BTreeSet::new();
    let mut pending = vec![root];
    while let Some(file) = pending.pop() {
        if !selected.insert(file.display_id.clone()) || file.ext != "tex" {
            continue;
        }
        let parent = file.display_id.rsplit_once('/').map_or("", |(parent, _)| parent);
        let tex = find_tex_includes_in_line_block(&file.stripped_tex);
        let bibliographies = bib.captures_iter(&file.stripped_tex)
            .flat_map(|capture| capture.get(1).unwrap().as_str().split(','))
            .map(|path| (path.trim(), "bib"));
        let targets = tex.iter().map(|(_, path)| (path.as_str(), "tex")).chain(bibliographies);
        for (path, ext) in targets {
            let path = path.trim().trim_matches(['"', '\'']);
            if path.is_empty() || path.starts_with('/') {
                continue;
            }
            for base in [parent, bundle.as_str()] {
                let joined = if base.is_empty() { path.to_string() } else { format!("{base}/{path}") };
                let Some(joined) = normalize_rel_segments(&joined) else { continue };
                let Some(candidate) = files.iter().find(|candidate| {
                    candidate.ext == ext && candidate.from_sources_dir == root.from_sources_dir
                        && (!root.from_sources_dir || candidate.display_id.starts_with(&format!("{bundle}/")))
                        && (candidate.display_id == joined
                            || candidate.display_id.strip_suffix(&format!(".{ext}")) == Some(joined.as_str()))
                }) else { continue };
                pending.push(candidate);
                break;
            }
        }
        if let Some(stem) = file.display_id.rsplit_once('.').map(|(stem, _)| stem) {
            let generated = format!("{stem}.bbl");
            if files.iter().any(|f| f.display_id == generated && f.from_sources_dir == root.from_sources_dir) {
                selected.insert(generated);
            }
        }
    }
    selected
}

pub(crate) fn collect_text_files(
    canon_root: &Path,
    dir: &Path,
    prefix: &str,
    from_sources_dir: bool,
    out: &mut Vec<LoadedFile>,
) {
    if out.len() >= MAX_SCANNED_FILES {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    let clean_root = strip_verbatim_prefix(canon_root);

    for entry in entries {
        if out.len() >= MAX_SCANNED_FILES {
            break;
        }
        let name = entry.file_name();
        let Some(name_s) = name.to_str() else { continue };
        if name_s.starts_with('.') || matches!(name_s, "target" | "node_modules" | "__pycache__") {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else { continue };
        if is_symlink_or_junction(&path, &meta) {
            continue;
        }
        let Ok(canon) = fs::canonicalize(&path) else { continue };
        if !WorkspaceSandbox::is_same_or_inside(canon_root, &canon) {
            continue;
        }
        if meta.is_dir() {
            collect_text_files(canon_root, &canon, prefix, from_sources_dir, out);
        } else if meta.is_file() && meta.len() <= MAX_FILE_BYTES {
            let ext = canon.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            if !matches!(ext.as_str(), "tex" | "bib" | "bbl" | "md" | "markdown" | "txt" | "json") {
                continue;
            }
            let Ok(bytes) = fs::read(&canon) else { continue };
            if bytes.contains(&0) {
                continue;
            }
            let raw = String::from_utf8_lossy(&bytes).into_owned();
            let clean_canon = strip_verbatim_prefix(&canon);
            let rel = clean_canon
                .strip_prefix(&clean_root)
                .ok()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|| name_s.to_string());
            let display_id = if from_sources_dir { format!("{prefix}/{rel}") } else { rel };
            let stripped_tex = if ext == "tex" { strip_latex_comments(&raw) } else { raw.clone() };
            let norm_raw = normalize_ws(&raw);
            let norm_stripped = normalize_ws(&stripped_tex);
            out.push(LoadedFile {
                display_id,
                ext,
                from_sources_dir,
                raw,
                stripped_tex,
                norm_raw,
                norm_stripped,
            });
        }
    }
}

pub(crate) fn collect_ledger_retractions(
    ledger: &Ledger,
    session: &str,
    withdrawn_arxiv: &mut BTreeMap<String, String>,
    retracted_dois: &mut BTreeSet<String>,
) {
    let base_session = session.strip_prefix("sidetalk:").unwrap_or(session);
    let Ok(events) = ledger.read_session(base_session) else { return };
    let mut scanned_blobs = BTreeSet::new();
    for ev in events {
        scan_json_for_retractions(&ev.payload, withdrawn_arxiv, retracted_dois);
        if let Some(raw_hash) = ev.payload.get("raw_blob").and_then(Value::as_str) {
            scan_blob_if_json(ledger, raw_hash, &mut scanned_blobs, withdrawn_arxiv, retracted_dois);
        }
        if ev.kind == "tool.result" {
            if let Some(res_obj) = ev.payload.get("result") {
                if let Some(inline_str) = res_obj.get("inline").and_then(Value::as_str) {
                    if let Ok(v) = serde_json::from_str::<Value>(inline_str) {
                        scan_json_for_retractions(&v, withdrawn_arxiv, retracted_dois);
                    }
                }
                if let Some(blob_hash) = res_obj.get("blob").and_then(Value::as_str) {
                    scan_blob_if_json(ledger, blob_hash, &mut scanned_blobs, withdrawn_arxiv, retracted_dois);
                }
            }
        }
    }
}

fn scan_blob_if_json(
    ledger: &Ledger,
    hash: &str,
    scanned: &mut BTreeSet<String>,
    withdrawn_arxiv: &mut BTreeMap<String, String>,
    retracted_dois: &mut BTreeSet<String>,
) {
    if scanned.insert(hash.to_string()) {
        if let Ok(bytes) = ledger.get_blob(hash) {
            if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
                scan_json_for_retractions(&v, withdrawn_arxiv, retracted_dois);
            }
        }
    }
}

pub(crate) fn scan_json_for_retractions(
    v: &Value,
    withdrawn_arxiv: &mut BTreeMap<String, String>,
    retracted_dois: &mut BTreeSet<String>,
) {
    match v {
        Value::Object(map) => {
            if map.get("withdrawn") == Some(&Value::Bool(true)) {
                for k in ["arxiv_id", "id", "eprint"] {
                    if let Some(s) = map.get(k).and_then(Value::as_str) {
                        if let Some(base) = base_arxiv_id(s) {
                            withdrawn_arxiv.insert(base, s.to_string());
                        }
                    }
                }
            }
            if map.get("retracted") == Some(&Value::Bool(true)) {
                for k in ["doi", "DOI"] {
                    if let Some(s) = map.get(k).and_then(Value::as_str) {
                        if let Some(norm) = normalize_doi(s) {
                            retracted_dois.insert(norm);
                        }
                    }
                }
            }
            if let Some(arr) = map.get("withdrawn_arxiv_ids").and_then(Value::as_array) {
                for item in arr.iter().filter_map(Value::as_str) {
                    if let Some(base) = base_arxiv_id(item) {
                        withdrawn_arxiv.insert(base, item.to_string());
                    }
                }
            }
            if let Some(arr) = map.get("retracted_dois").and_then(Value::as_array) {
                for item in arr.iter().filter_map(Value::as_str) {
                    if let Some(norm) = normalize_doi(item) {
                        retracted_dois.insert(norm);
                    }
                }
            }
            for child in map.values() {
                scan_json_for_retractions(child, withdrawn_arxiv, retracted_dois);
            }
        }
        Value::Array(items) => {
            for item in items {
                scan_json_for_retractions(item, withdrawn_arxiv, retracted_dois);
            }
        }
        _ => {}
    }
}
