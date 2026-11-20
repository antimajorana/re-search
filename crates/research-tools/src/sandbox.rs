//! Canonical path containment sandbox for attached `workspace_root` and
//! per-session `sessions/<session_id>/sources` directories.

use std::fmt;
use std::fs::{self, Metadata, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use tools::canon::random_hex;

pub const MAX_SESSION_ID_LEN: usize = 43;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxRoot {
    Workspace,
    Sources,
}

pub type RootKind = SandboxRoot;

impl SandboxRoot {
    pub fn parse_opt(raw: Option<&str>) -> Result<Self, SandboxError> {
        match raw.unwrap_or("workspace") {
            "workspace" => Ok(Self::Workspace),
            "sources" => Ok(Self::Sources),
            other => Err(SandboxError::InvalidRoot(format!(
                "invalid root `{other}` (expected `workspace` or `sources`)"
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Sources => "sources",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxError {
    InvalidSessionId(String),
    InvalidRoot(String),
    WorkspaceNotAttached,
    InvalidPath(String),
    EscapesRoot(String),
    NotFound(String),
    NotADirectory(String),
    Io(String),
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSessionId(m) => write!(f, "{m}"),
            Self::InvalidRoot(m) => write!(f, "{m}"),
            Self::WorkspaceNotAttached => write!(f, "no workspace directory is attached to this session"),
            Self::InvalidPath(m) => write!(f, "{m}"),
            Self::EscapesRoot(m) => write!(f, "path escapes sandbox root: {m}"),
            Self::NotFound(m) => write!(f, "path not found: {m}"),
            Self::NotADirectory(m) => write!(f, "not a directory: {m}"),
            Self::Io(m) => write!(f, "io error: {m}"),
        }
    }
}

impl std::error::Error for SandboxError {}

/// Validates a canonical session identifier (`1..=43` chars of `[a-z0-9._-]`,
/// rejecting `"."`, `".."`, trailing `'.'`, `':'`, `'/'`, `'\\'`, NUL, and control chars).
pub fn validate_session_id(session: &str) -> Result<(), SandboxError> {
    if session.is_empty()
        || session.len() > MAX_SESSION_ID_LEN
        || matches!(session, "." | "..")
        || session.ends_with('.')
        || !session
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
    {
        return Err(SandboxError::InvalidSessionId(format!("invalid session id `{session}`")));
    }
    Ok(())
}

/// Resolves the underlying session ID for read-only `Sources` access, mapping
/// `"sidetalk:<base_session>"` to `<base_session>` while validating `<base_session>`.
pub fn resolve_sources_session_id(ctx_session: &str) -> Result<&str, SandboxError> {
    let base = ctx_session.strip_prefix("sidetalk:").unwrap_or(ctx_session);
    validate_session_id(base)?;
    Ok(base)
}

/// Strips Windows `\\?\UNC\server\share` -> `\\server\share` and `\\?\C:\...` -> `C:\...`
/// verbatim prefixes across platforms for clean user-facing paths and subprocess CWDs.
pub fn strip_verbatim_str(s: &str) -> String {
    let bytes = s.as_bytes();
    if bytes.len() >= 8 && bytes[..4] == *b"\\\\?\\" && bytes[4..8].eq_ignore_ascii_case(b"UNC\\") {
        return format!("\\\\{}", &s[8..]);
    }
    s.strip_prefix(r"\\?\").map(str::to_string).unwrap_or_else(|| s.to_string())
}

pub fn strip_verbatim_prefix(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        use std::ffi::OsString;
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        const PFX: [u16; 4] = [b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
        if wide.starts_with(&PFX) {
            if wide.len() >= 8
                && matches!(wide[4], 0x55 | 0x75)
                && matches!(wide[5], 0x4E | 0x6E)
                && matches!(wide[6], 0x43 | 0x63)
                && wide[7] == b'\\' as u16
            {
                let mut unc = vec![b'\\' as u16, b'\\' as u16];
                unc.extend_from_slice(&wide[8..]);
                return PathBuf::from(OsString::from_wide(&unc));
            }
            return PathBuf::from(OsString::from_wide(&wide[4..]));
        }
    }
    path.to_str()
        .map(|s| PathBuf::from(strip_verbatim_str(s)))
        .unwrap_or_else(|| path.to_path_buf())
}

fn is_windows_reserved_stem(seg: &str) -> bool {
    let stem = seg.split('.').next().unwrap_or("");
    matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON" | "PRN" | "AUX" | "NUL"
            | "COM1" | "COM2" | "COM3" | "COM4" | "COM5" | "COM6" | "COM7" | "COM8" | "COM9"
            | "LPT1" | "LPT2" | "LPT3" | "LPT4" | "LPT5" | "LPT6" | "LPT7" | "LPT8" | "LPT9"
    )
}

/// Validates and normalizes a relative path into clean UTF-8 components and a `/`-joined string.
/// Rejects NUL/control bytes, `\`, `:`, leading `/`, `..`, root/prefix components, and reserved stems.
pub fn sanitize_rel_components(rel: &str, allow_empty: bool) -> Result<(Vec<String>, String), SandboxError> {
    if rel.bytes().any(|b| b < 0x20 || b == 0x7f)
        || rel.contains('\\')
        || rel.contains(':')
        || rel.starts_with('/')
    {
        return Err(SandboxError::InvalidPath(format!("unsafe path `{rel}`")));
    }
    let mut parts = Vec::new();
    for comp in Path::new(rel).components() {
        match comp {
            Component::CurDir => {}
            Component::Normal(os) => {
                let s = os
                    .to_str()
                    .ok_or_else(|| SandboxError::InvalidPath("non-UTF-8 path".into()))?;
                if s == ".." || s.is_empty() {
                    return Err(SandboxError::EscapesRoot(rel.to_string()));
                }
                if s.ends_with('.') || s.ends_with(' ') || is_windows_reserved_stem(s) {
                    return Err(SandboxError::InvalidPath(format!("disallowed filename segment `{s}`")));
                }
                parts.push(s.to_string());
            }
            _ => return Err(SandboxError::EscapesRoot(format!("disallowed component in `{rel}`"))),
        }
    }
    if parts.is_empty() && !allow_empty {
        return Err(SandboxError::InvalidPath("path must not be empty".into()));
    }
    let norm = parts.join("/");
    Ok((parts, norm))
}

pub fn validate_rel_workspace_path(rel: &str) -> Result<Vec<String>, String> {
    sanitize_rel_components(rel, false)
        .map(|(parts, _)| parts)
        .map_err(|e| e.to_string())
}

#[derive(Debug, Clone)]
pub struct ResolvedPath {
    pub canonical_root: PathBuf,
    pub canonical_target: PathBuf,
    pub rel_path: String,
    pub metadata: Metadata,
}

#[derive(Debug, Clone)]
pub struct ResolvedWritePath {
    pub canonical_root: PathBuf,
    pub target_path: PathBuf,
    pub rel_parts: Vec<String>,
    pub rel_path: String,
    pub exists: bool,
}

#[derive(Debug, Clone)]
pub struct WorkspaceSandbox {
    data_dir: PathBuf,
    workspace_root: Option<PathBuf>,
}

impl WorkspaceSandbox {
    pub fn new(data_dir: impl Into<PathBuf>, workspace_root: Option<PathBuf>) -> Self {
        Self { data_dir: data_dir.into(), workspace_root }
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn workspace_root(&self) -> Option<&Path> {
        self.workspace_root.as_deref()
    }

    pub fn canonicalize_existing_dir(dir: &Path) -> Result<PathBuf, SandboxError> {
        let canon = fs::canonicalize(dir).map_err(|e| SandboxError::NotFound(format!("{}: {e}", dir.display())))?;
        if !canon.is_dir() {
            return Err(SandboxError::NotADirectory(dir.display().to_string()));
        }
        Ok(canon)
    }

    pub fn is_same_or_inside(canon_root: &Path, canon_target: &Path) -> bool {
        let r = strip_verbatim_prefix(canon_root);
        let t = strip_verbatim_prefix(canon_target);
        let rc: Vec<_> = r.components().collect();
        let tc: Vec<_> = t.components().collect();
        if tc.len() < rc.len() {
            return false;
        }
        rc.iter().zip(tc.iter()).all(|(a, b)| {
            #[cfg(windows)]
            {
                a.as_os_str().to_string_lossy().eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
            }
            #[cfg(not(windows))]
            {
                a == b
            }
        })
    }

    pub fn canonical_root(
        &self,
        root: SandboxRoot,
        session_id: &str,
        create_sources: bool,
    ) -> Result<Option<PathBuf>, SandboxError> {
        match root {
            SandboxRoot::Workspace => {
                let ws = self.workspace_root.as_deref().ok_or(SandboxError::WorkspaceNotAttached)?;
                Ok(Some(Self::canonicalize_existing_dir(ws)?))
            }
            SandboxRoot::Sources => {
                let base = if create_sources {
                    validate_session_id(session_id)?;
                    session_id
                } else {
                    resolve_sources_session_id(session_id)?
                };
                if create_sources {
                    fs::create_dir_all(&self.data_dir).map_err(|e| SandboxError::Io(e.to_string()))?;
                } else if !self.data_dir.exists() {
                    return Ok(None);
                }
                let canon_data = Self::canonicalize_existing_dir(&self.data_dir)?;
                let mut cur = canon_data.clone();
                for seg in ["sessions", base, "sources"] {
                    let next = cur.join(seg);
                    match fs::symlink_metadata(&next) {
                        Ok(lmeta) => {
                            if lmeta.file_type().is_symlink() {
                                return Err(SandboxError::EscapesRoot(format!("symlink at `{seg}`")));
                            }
                            if !lmeta.is_dir() {
                                return Err(SandboxError::NotADirectory(seg.to_string()));
                            }
                        }
                        Err(e) if e.kind() == io::ErrorKind::NotFound => {
                            if !create_sources {
                                return Ok(None);
                            }
                            fs::create_dir(&next).map_err(|e| SandboxError::Io(e.to_string()))?;
                        }
                        Err(e) => return Err(SandboxError::Io(e.to_string())),
                    }
                    let canon_next = fs::canonicalize(&next).map_err(|e| SandboxError::EscapesRoot(e.to_string()))?;
                    if !Self::is_same_or_inside(&canon_data, &canon_next) {
                        return Err(SandboxError::EscapesRoot(format!("`{seg}` escapes data_dir")));
                    }
                    cur = canon_next;
                }
                Ok(Some(cur))
            }
        }
    }

    pub fn resolve_existing(
        &self,
        root: SandboxRoot,
        session_id: &str,
        rel: &str,
        allow_root_dir: bool,
    ) -> Result<ResolvedPath, SandboxError> {
        let (parts, norm_rel) = sanitize_rel_components(rel, allow_root_dir)?;
        let canon_root = self
            .canonical_root(root, session_id, false)?
            .ok_or_else(|| SandboxError::NotFound(norm_rel.clone()))?;
        let mut cur = canon_root.clone();
        for (idx, seg) in parts.iter().enumerate() {
            let is_last = idx + 1 == parts.len();
            let next = cur.join(seg);
            let lmeta = fs::symlink_metadata(&next).map_err(|e| {
                if e.kind() == io::ErrorKind::NotFound {
                    SandboxError::NotFound(norm_rel.clone())
                } else {
                    SandboxError::Io(e.to_string())
                }
            })?;
            let canon_next = fs::canonicalize(&next).map_err(|e| {
                if lmeta.file_type().is_symlink() {
                    SandboxError::EscapesRoot(format!("broken or escaping symlink `{norm_rel}`: {e}"))
                } else {
                    SandboxError::NotFound(norm_rel.clone())
                }
            })?;
            if !Self::is_same_or_inside(&canon_root, &canon_next) {
                return Err(SandboxError::EscapesRoot(format!("`{norm_rel}` points outside sandbox root")));
            }
            if !is_last && !canon_next.is_dir() {
                return Err(SandboxError::NotADirectory(norm_rel));
            }
            cur = canon_next;
        }
        let metadata = fs::metadata(&cur).map_err(|e| SandboxError::Io(e.to_string()))?;
        Ok(ResolvedPath { canonical_root: canon_root, canonical_target: cur, rel_path: norm_rel, metadata })
    }

    /// Walks every existing ancestor from `canon_root` via `symlink_metadata` + `canonicalize`,
    /// rejecting symlinks and junctions BEFORE any directory creation or file write occurs.
    pub fn resolve_for_write(
        &self,
        root: SandboxRoot,
        session_id: &str,
        rel: &str,
    ) -> Result<ResolvedWritePath, SandboxError> {
        let (parts, norm_rel) = sanitize_rel_components(rel, false)?;
        let canon_root = self
            .canonical_root(root, session_id, root == SandboxRoot::Sources)?
            .ok_or(SandboxError::WorkspaceNotAttached)?;
        let (mut cur, mut exists) = (canon_root.clone(), true);
        for (idx, seg) in parts.iter().enumerate() {
            let is_last = idx + 1 == parts.len();
            let next = cur.join(seg);
            if !exists {
                cur = next;
                continue;
            }
            match fs::symlink_metadata(&next) {
                Ok(lmeta) => {
                    if lmeta.file_type().is_symlink() {
                        return Err(SandboxError::EscapesRoot(format!("symlink forbidden in `{norm_rel}`")));
                    }
                    let canon_next = fs::canonicalize(&next).map_err(|e| SandboxError::EscapesRoot(e.to_string()))?;
                    if !Self::is_same_or_inside(&canon_root, &canon_next) {
                        return Err(SandboxError::EscapesRoot(format!("`{norm_rel}` escapes sandbox root")));
                    }
                    if !is_last && !canon_next.is_dir() {
                        return Err(SandboxError::NotADirectory(norm_rel));
                    }
                    if is_last && canon_next.is_dir() {
                        return Err(SandboxError::InvalidPath(format!("target `{norm_rel}` is a directory")));
                    }
                    cur = canon_next;
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    exists = false;
                    cur = next;
                }
                Err(e) => return Err(SandboxError::Io(e.to_string())),
            }
        }
        Ok(ResolvedWritePath {
            canonical_root: canon_root,
            target_path: cur,
            rel_parts: parts,
            rel_path: norm_rel,
            exists,
        })
    }

    /// Atomically writes `bytes` to `rel_parts` inside `canon_root` using a sibling
    /// `.re-search-tmp-*` `create_new` file + `sync_all` + TOCTOU recheck + `rename`.
    pub fn atomic_write_in_root(
        canon_root: &Path,
        rel_parts: &[String],
        bytes: &[u8],
    ) -> Result<PathBuf, SandboxError> {
        let (file_name, dirs) = rel_parts
            .split_last()
            .ok_or_else(|| SandboxError::InvalidPath("empty path".into()))?;
        let mut cur = canon_root.to_path_buf();
        for dir in dirs {
            let next = cur.join(dir);
            match fs::symlink_metadata(&next) {
                Ok(meta) => {
                    if meta.file_type().is_symlink() || !meta.is_dir() {
                        return Err(SandboxError::EscapesRoot(format!("unsafe parent `{dir}`")));
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    fs::create_dir(&next).map_err(|e| SandboxError::Io(format!("create `{dir}`: {e}")))?;
                }
                Err(e) => return Err(SandboxError::Io(e.to_string())),
            }
            let next_canon = fs::canonicalize(&next).map_err(|e| SandboxError::EscapesRoot(e.to_string()))?;
            if !Self::is_same_or_inside(canon_root, &next_canon) {
                return Err(SandboxError::EscapesRoot(format!("directory `{dir}` escaped root")));
            }
            cur = next_canon;
        }

        let target = cur.join(file_name);
        let tmp_path = cur.join(format!(".re-search-tmp-{}-{}", std::process::id(), random_hex(8)));
        let res = (|| -> Result<PathBuf, SandboxError> {
            let mut f = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp_path)
                .map_err(|e| SandboxError::Io(format!("create temp file: {e}")))?;
            f.write_all(bytes).map_err(|e| SandboxError::Io(format!("write temp file: {e}")))?;
            f.sync_all().map_err(|e| SandboxError::Io(format!("sync temp file: {e}")))?;
            drop(f);

            let recheck = fs::canonicalize(&cur).map_err(|e| SandboxError::EscapesRoot(e.to_string()))?;
            if !Self::is_same_or_inside(canon_root, &recheck) {
                return Err(SandboxError::EscapesRoot("parent directory escaped root before rename".into()));
            }
            if let Ok(meta) = fs::symlink_metadata(&target) {
                if meta.file_type().is_symlink() || meta.is_dir() {
                    return Err(SandboxError::EscapesRoot(format!(
                        "refusing to overwrite symlink or directory `{file_name}`"
                    )));
                }
            }
            fs::rename(&tmp_path, &target)
                .map_err(|e| SandboxError::Io(format!("atomic rename to `{file_name}`: {e}")))?;
            Ok(target)
        })();

        if res.is_err() {
            let _ = fs::remove_file(&tmp_path);
        }
        res
    }
}
