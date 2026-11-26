//! Paper source model, read-only adapter trait, path containment helpers, and
//! session attachment for user-owned paper libraries (package F13a, `00-overview.md` 2.11).
//!
//! Invariant: F13a **only reads** user paper sources (`00-overview.md` section 8.3, decision 5).
//! No file inside a registered source root is ever created, modified, moved, or deleted.
//! Opening a library paper in a session pins its bytes through [`ingest_bytes`], so the
//! library index is never evidence and removing a source never breaks any session quote check.

use crate::ingest::{ingest_bytes, IngestRequest, IngestedSource};
use crate::library::{
    FulltextRecord, Library, LibraryIds, SourceItemFile, SourceItemRecord, SourceItemState,
    SourceKind, SourceRecord, ORIGIN_USER_LIBRARY,
};
use crate::paper_key::PaperKey;
use crate::pdf_pages::MAX_USER_PDF_BYTES;
use crate::sandbox::{is_symlink_or_junction, strip_verbatim_prefix, WorkspaceSandbox};
use ledger::Ledger;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Source lifecycle state strings (`00-overview.md` section 2.11 & 8.3 decision 5).
pub const SOURCE_STATE_PENDING: &str = "pending_sync";
pub const SOURCE_STATE_SYNCED: &str = "synced";
pub const SOURCE_STATE_ERROR: &str = "error";

/// Attached file roles inside [`SourceItemFile`] (`00-overview.md` section 2.11).
pub const FILE_ROLE_PDF: &str = "pdf";
pub const FILE_ROLE_TEX: &str = "tex";
pub const FILE_ROLE_NOTE: &str = "note";

/// Upper bound when reading non-PDF metadata/TeX files from a user source (8 MiB).
pub const MAX_SOURCE_TEXT_BYTES: usize = 8 * 1024 * 1024;

/// Context passed into a read-only [`PaperSourceAdapter::scan`] call.
#[derive(Debug, Clone)]
pub struct ScanContext {
    /// Previously indexed items for this source keyed by `external_id`.
    pub previous_by_id: BTreeMap<String, SourceItemRecord>,
    /// Previously indexed items keyed by normalized file path (for unchanged / moved checks).
    pub previous_by_path: BTreeMap<String, SourceItemRecord>,
    /// Whether the adapter may hash identifier-less files during this scan.
    pub hash_files: bool,
    /// Current timestamp in milliseconds since Unix epoch.
    pub now_ms: i64,
}

impl ScanContext {
    pub fn from_previous(previous: &[SourceItemRecord], hash_files: bool, now_ms: i64) -> Self {
        let mut previous_by_id = BTreeMap::new();
        let mut previous_by_path = BTreeMap::new();
        for item in previous {
            previous_by_id.insert(item.external_id.clone(), item.clone());
            for f in &item.files {
                if f.role != FILE_ROLE_NOTE && !f.path.is_empty() {
                    previous_by_path.insert(f.path.clone(), item.clone());
                }
            }
        }
        Self {
            previous_by_id,
            previous_by_path,
            hash_files,
            now_ms,
        }
    }
}

/// One scanned item produced by a [`PaperSourceAdapter`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScannedItem {
    pub item: SourceItemRecord,
    /// Optional abstract text extracted from metadata (not stored on `source_item`, upserted to `paper`).
    #[serde(default)]
    pub abstract_text: String,
    /// Optional publication venue / journal name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub venue: Option<String>,
    /// True when the adapter reused previous metadata via `(path, size, mtime_ms)` without reading content.
    #[serde(default)]
    pub reused_unchanged: bool,
}

/// Non-fatal problem noticed while scanning a source (escaped path, rejected symlink, unreadable PDF).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanWarning {
    pub path: String,
    pub reason: String,
}

/// Read-only output of [`PaperSourceAdapter::scan`].
#[derive(Debug, Clone, Default)]
pub struct ScanOutput {
    pub items: Vec<ScannedItem>,
    pub warnings: Vec<ScanWarning>,
    pub roots_checked: Vec<PathBuf>,
}

/// Read-only paper source adapter (`00-overview.md` 2.11).
pub trait PaperSourceAdapter {
    fn kind(&self) -> SourceKind;
    fn scan(&self, source: &SourceRecord, ctx: &ScanContext) -> Result<ScanOutput, String>;
}

/// Query submitted to an approved network title-lookup hook (`00-overview.md` 2.4 & 2.11).
///
/// Metadata lookup by title over the network is **not built in F13a** because it composes an
/// external request and therefore requires the approval card. Callers leave the hook `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TitleLookupRequest {
    pub title: String,
    pub authors: Vec<String>,
    pub year: Option<i32>,
}

/// Result returned by an approved [`TitleLookupHook`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TitleLookupResult {
    pub key: PaperKey,
    pub ids: LibraryIds,
    pub title: String,
    pub authors: Vec<String>,
    pub year: Option<i32>,
}

/// Optional hook for future approval-gated network metadata lookup by title (not wired in F13a).
pub trait TitleLookupHook {
    fn lookup_by_title(&self, req: &TitleLookupRequest) -> Result<Option<TitleLookupResult>, String>;
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let digest = h.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

/// Canonicalize a registered source root directory and strip any Windows verbatim `\\?\` prefix.
pub fn canonicalize_source_root(root: &Path) -> Result<PathBuf, String> {
    let raw = root.to_string_lossy();
    if raw.trim().is_empty() || raw.contains('\0') {
        return Err("source root path must not be empty or contain NUL".into());
    }
    let canon = WorkspaceSandbox::canonicalize_existing_dir(root)
        .map_err(|e| format!("invalid source root `{}`: {e}", root.display()))?;
    Ok(strip_verbatim_prefix(&canon))
}

/// Normalize a filesystem path to forward slashes without Windows verbatim prefixes.
pub fn portable_path_string(path: &Path) -> String {
    strip_verbatim_prefix(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Resolve a relative path strictly inside `canon_root`, refusing NUL, control characters,
/// `..`, absolute prefixes, symlinks, NTFS directory junctions, and any target outside `canon_root`.
pub fn resolve_contained_rel(canon_root: &Path, rel: &str) -> Result<PathBuf, String> {
    let normalized = rel.replace('\\', "/");
    if normalized.is_empty()
        || normalized.bytes().any(|b| b < 0x20 || b == 0x7f)
        || normalized.starts_with('/')
    {
        return Err(format!("unsafe relative path `{rel}`"));
    }
    let mut parts: Vec<String> = Vec::new();
    for comp in Path::new(&normalized).components() {
        match comp {
            Component::CurDir => {}
            Component::Normal(os) => {
                let s = os
                    .to_str()
                    .ok_or_else(|| format!("non-UTF-8 path segment in `{rel}`"))?;
                if s.is_empty() || s == ".." {
                    return Err(format!("path `{rel}` escapes registered root"));
                }
                if s.contains(':') {
                    return Err(format!("disallowed drive/stream syntax in `{rel}`"));
                }
                parts.push(s.to_string());
            }
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!("path `{rel}` escapes registered root"));
            }
        }
    }
    if parts.is_empty() {
        return Err(format!("empty relative path `{rel}`"));
    }
    walk_contained_parts(canon_root, &parts, rel)
}

/// Resolve an absolute path strictly inside `canon_root` by verifying every component from
/// `canon_root` downward against symlinks, junctions, and root containment.
pub fn resolve_contained_abs(canon_root: &Path, candidate: &Path) -> Result<PathBuf, String> {
    let raw = candidate.to_string_lossy();
    if raw.is_empty() || raw.contains('\0') {
        return Err("empty or NUL path".into());
    }
    let clean_root = strip_verbatim_prefix(canon_root);
    let clean_cand = strip_verbatim_prefix(candidate);
    let rel_comps = strip_root_components(&clean_root, &clean_cand).ok_or_else(|| {
        format!(
            "path `{}` escapes registered root `{}`",
            candidate.display(),
            clean_root.display()
        )
    })?;
    let mut parts = Vec::new();
    for comp in rel_comps {
        match comp {
            Component::CurDir => {}
            Component::Normal(os) => {
                let s = os
                    .to_str()
                    .ok_or_else(|| "non-UTF-8 path segment".to_string())?;
                if s.is_empty() || s == ".." {
                    return Err(format!("path `{}` escapes registered root", candidate.display()));
                }
                parts.push(s.to_string());
            }
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!("path `{}` escapes registered root", candidate.display()));
            }
        }
    }
    if parts.is_empty() {
        return Ok(clean_root);
    }
    walk_contained_parts(&clean_root, &parts, &raw)
}

fn strip_root_components<'a>(root: &Path, cand: &'a Path) -> Option<Vec<Component<'a>>> {
    let r_comps: Vec<_> = root.components().collect();
    let c_comps: Vec<_> = cand.components().collect();
    if c_comps.len() < r_comps.len() {
        return None;
    }
    for (rc, cc) in r_comps.iter().zip(c_comps.iter()) {
        #[cfg(windows)]
        let eq = rc
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&cc.as_os_str().to_string_lossy());
        #[cfg(not(windows))]
        let eq = rc == cc;
        if !eq {
            return None;
        }
    }
    Some(c_comps[r_comps.len()..].to_vec())
}

fn walk_contained_parts(
    canon_root: &Path,
    parts: &[String],
    display_orig: &str,
) -> Result<PathBuf, String> {
    let mut cur = canon_root.to_path_buf();
    for (idx, seg) in parts.iter().enumerate() {
        let is_last = idx + 1 == parts.len();
        let next = cur.join(seg);
        let lmeta = fs::symlink_metadata(&next)
            .map_err(|e| format!("cannot inspect `{display_orig}`: {e}"))?;
        if is_symlink_or_junction(&next, &lmeta) {
            return Err(format!(
                "refused symlink or junction `{display_orig}` inside source root"
            ));
        }
        let canon_next = fs::canonicalize(&next)
            .map_err(|e| format!("cannot resolve `{display_orig}`: {e}"))?;
        if !WorkspaceSandbox::is_same_or_inside(canon_root, &canon_next) {
            return Err(format!(
                "path `{display_orig}` resolves outside registered source root"
            ));
        }
        if !is_last && !canon_next.is_dir() {
            return Err(format!("parent segment `{seg}` in `{display_orig}` is not a directory"));
        }
        cur = strip_verbatim_prefix(&canon_next);
    }
    Ok(cur)
}

/// Read `(size, mtime_ms)` from filesystem metadata without opening or reading file content.
pub fn file_meta_without_reading(path: &Path) -> Result<(u64, i64), String> {
    let md = fs::metadata(path).map_err(|e| format!("stat `{}`: {e}", path.display()))?;
    let mtime_ms = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0);
    Ok((md.len(), mtime_ms))
}

/// Read a bounded file strictly inside `canon_root`, refusing symlinks, junctions, escapes, and oversize files.
pub fn read_contained_bytes(
    canon_root: &Path,
    candidate: &Path,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let safe = resolve_contained_abs(canon_root, candidate)?;
    let md = fs::metadata(&safe).map_err(|e| format!("stat `{}`: {e}", safe.display()))?;
    if !md.is_file() {
        return Err(format!("`{}` is not a regular file", safe.display()));
    }
    if md.len() > max_bytes as u64 {
        return Err(format!(
            "`{}` exceeds {} byte limit ({} bytes)",
            safe.display(),
            max_bytes,
            md.len()
        ));
    }
    let file = fs::File::open(&safe).map_err(|e| format!("open `{}`: {e}", safe.display()))?;
    let mut buf = Vec::with_capacity((md.len() as usize).min(max_bytes));
    file.take((max_bytes as u64) + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("read `{}`: {e}", safe.display()))?;
    if buf.len() > max_bytes {
        return Err(format!("`{}` exceeded {max_bytes} byte limit while reading", safe.display()));
    }
    Ok(buf)
}

/// Compute SHA-256 of a contained regular file without modifying it.
pub fn hash_contained_file(
    canon_root: &Path,
    candidate: &Path,
    max_bytes: usize,
) -> Result<String, String> {
    let bytes = read_contained_bytes(canon_root, candidate, max_bytes)?;
    Ok(sha256_hex(&bytes))
}

/// Derive a canonical [`PaperKey`] from known identifiers using F2 precedence
/// (`arxiv:` -> `doi:` -> `inspire:` -> `s2:` -> `openalex:` -> `file:`).
pub fn derive_paper_key(ids: &LibraryIds, file_sha256: Option<&str>) -> Option<PaperKey> {
    if let Some(ax) = ids.arxiv.as_deref() {
        if let Ok(k) = PaperKey::from_arxiv(ax) {
            return Some(k);
        }
    }
    if let Some(doi) = ids.doi.as_deref() {
        if let Ok(k) = PaperKey::from_doi(doi) {
            return Some(k);
        }
    }
    if let Some(ins) = ids.inspire.as_deref() {
        if let Ok(k) = PaperKey::from_inspire(ins) {
            return Some(k);
        }
    }
    if let Some(s2) = ids.s2.as_deref() {
        if let Ok(k) = PaperKey::from_s2(s2) {
            return Some(k);
        }
    }
    if let Some(oa) = ids.openalex.as_deref() {
        if let Ok(k) = PaperKey::from_openalex(oa) {
            return Some(k);
        }
    }
    if let Some(sha) = file_sha256 {
        let lower = sha.trim().to_ascii_lowercase();
        if lower.len() == 64 && lower.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            if let Ok(k) = PaperKey::parse(&format!("file:{}", &lower[..16])) {
                return Some(k);
            }
        }
    }
    None
}

/// Attach a paper from either the cached library blob store OR a registered user paper source
/// into `session_id` through [`ingest_bytes`].
///
/// The session gets its own pinned `source.ingested` copy; editing, moving, or deleting the
/// user's file (or removing the source / deleting `library.db`) never breaks the session.
pub fn attach_source_paper_to_session(
    lib: &Library,
    sandbox: &WorkspaceSandbox,
    ledger: &Mutex<Ledger>,
    session_id: &str,
    key: &PaperKey,
    version: Option<&str>,
    section_filter: Option<&str>,
) -> Result<IngestedSource, String> {
    let canonical = lib.resolve_key(key)?.unwrap_or_else(|| key.clone());
    if lib.read_fulltext_bytes(&canonical, version)?.is_some() {
        return lib.attach_to_session(sandbox, ledger, session_id, &canonical, version, section_filter);
    }
    let detail = lib
        .get_paper(&canonical)?
        .ok_or_else(|| format!("paper `{canonical}` not found in library"))?;

    let allowed_roots = collect_allowed_roots_for_sources(&lib.list_sources()?);
    let (chosen, chosen_version) =
        pick_best_user_file(&detail.source_items, &detail.fulltext, version)
            .ok_or_else(|| format!("no readable full-text file found in library for `{canonical}`"))?;

    let cand_path = PathBuf::from(&chosen.path);
    let safe_root = allowed_roots
        .iter()
        .find(|r| resolve_contained_abs(r, &cand_path).is_ok())
        .ok_or_else(|| {
            format!(
                "library file `{}` is outside all registered source roots or uses a symlink",
                chosen.path
            )
        })?;

    let max_bytes = if chosen.role == FILE_ROLE_PDF {
        MAX_USER_PDF_BYTES
    } else {
        MAX_SOURCE_TEXT_BYTES
    };
    let raw_bytes = read_contained_bytes(safe_root, &cand_path, max_bytes)?;
    let sha = sha256_hex(&raw_bytes);
    let (size, mtime_ms) = file_meta_without_reading(&cand_path)?;

    let file_name = cand_path
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .unwrap_or("paper.pdf")
        .to_string();
    let format = format_from_path_and_role(&chosen.path, &chosen.role);
    let effective_version = version
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .or(chosen_version);

    lib.upsert_user_library_fulltext(&FulltextRecord {
        key: canonical.clone(),
        version: effective_version,
        raw_blob: None,
        format,
        bytes: raw_bytes.len() as u64,
        origin: ORIGIN_USER_LIBRARY.to_string(),
        fetched_ms: now_ms(),
        file_path: Some(chosen.path.clone()),
        file_size: Some(size),
        file_mtime_ms: Some(mtime_ms),
        file_sha256: Some(sha),
    })?;

    let title = (!detail.paper.title.trim().is_empty()).then_some(detail.paper.title.as_str());
    let origin = format!("user_library:{canonical}");
    ingest_bytes(
        sandbox,
        ledger,
        &IngestRequest {
            session_id,
            file_name: &file_name,
            origin: &origin,
            raw_bytes: &raw_bytes,
            source_id_hint: Some(&file_name),
            section_filter,
            title,
        },
    )
}

fn collect_allowed_roots_for_sources(sources: &[SourceRecord]) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for src in sources {
        if src.kind == SourceKind::Bibtex {
            for extra in crate::source_bibtex::allowed_roots_for_bibtex(Path::new(&src.root), &src.settings) {
                if !roots.contains(&extra) {
                    roots.push(extra);
                }
            }
            continue;
        }
        if let Ok(canon) = canonicalize_source_root(Path::new(&src.root)) {
            if !roots.contains(&canon) {
                roots.push(canon.clone());
            }
            if src.kind == SourceKind::KandinskyVault {
                for extra in crate::source_kandinsky::allowed_roots_for_vault(&canon, &src.settings) {
                    if !roots.contains(&extra) {
                        roots.push(extra);
                    }
                }
            }
        }
    }
    roots
}

fn pick_best_user_file(
    items: &[SourceItemRecord],
    fulltext: &[FulltextRecord],
    requested_version: Option<&str>,
) -> Option<(SourceItemFile, Option<String>)> {
    let wanted = requested_version.map(str::trim).filter(|v| !v.is_empty());
    // Prefer TeX over PDF when both exist (`00-overview.md` 2.11), matching requested version if given.
    for want_role in [FILE_ROLE_TEX, FILE_ROLE_PDF] {
        for item in items {
            if item.state == SourceItemState::Missing {
                continue;
            }
            if let Some(v) = wanted {
                if item.version.as_deref().map(str::trim) != Some(v) {
                    continue;
                }
            }
            if let Some(f) = item.files.iter().find(|f| f.role == want_role) {
                return Some((f.clone(), item.version.clone()));
            }
        }
    }
    for ft in fulltext {
        if ft.origin == ORIGIN_USER_LIBRARY {
            if let Some(v) = wanted {
                if ft.version.as_deref().map(str::trim) != Some(v) {
                    continue;
                }
            }
            if let Some(path) = &ft.file_path {
                let role = if ft.format == "tex" {
                    FILE_ROLE_TEX
                } else {
                    FILE_ROLE_PDF
                };
                return Some((
                    SourceItemFile {
                        path: path.clone(),
                        role: role.to_string(),
                        size: ft.file_size.unwrap_or(ft.bytes),
                        mtime_ms: ft.file_mtime_ms.unwrap_or(0),
                        sha256: ft.file_sha256.clone(),
                    },
                    ft.version.clone(),
                ));
            }
        }
    }
    None
}

pub(crate) fn format_from_path_and_role(path: &str, role: &str) -> String {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        "tar.gz".to_string()
    } else if lower.ends_with(".zip") {
        "zip".to_string()
    } else if lower.ends_with(".tex") || role == FILE_ROLE_TEX {
        "tex".to_string()
    } else {
        "pdf".to_string()
    }
}
