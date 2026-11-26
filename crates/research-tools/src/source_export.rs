//! Opt-in sending papers back (package F13c, `00-overview.md` 2.11 & 8.3 decision 5):
//!
//! 1. **Export BibTeX**: formats selected library papers (preferring the user's stored
//!    `source_item.citekey` over synthesized keys) and writes atomically ONLY to a user-chosen
//!    destination path, refusing `..`, symlinks/junctions, and any registered read-only
//!    `PaperSource` root.
//! 2. **Send to Kandinsky**: validates an external identifier/URL target (`arXiv:`, `doi:`,
//!    or `https://`) and builds either a strict argv-only `kandinsky capture-paper <target>`
//!    command (NEVER invoked through a shell) or a percent-encoded
//!    `kandinsky://capture-paper?...` deep link.
//!
//! Invariant: re:Search NEVER writes files directly into a Kandinsky vault or Zotero database,
//! and sync never invokes export or Kandinsky capture automatically.

use crate::library::{Library, SourceKind, SourceRecord};
use crate::paper_key::{canonical_doi, split_arxiv_id, PaperKey};
use crate::paper_sources::{canonicalize_source_root, portable_path_string};
use crate::sandbox::{is_symlink_or_junction, strip_verbatim_prefix, WorkspaceSandbox};
pub use crate::source_export_bib::format_library_papers_bibtex;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

/// User-facing label for opt-in BibTeX export (`00-overview.md` 2.11 & 8.3 decision 5).
pub const LABEL_EXPORT_BIBTEX: &str = "Export BibTeX";
/// User-facing label for opt-in Kandinsky handoff (`00-overview.md` 2.11 & 8.3 decision 5).
pub const LABEL_SEND_TO_KANDINSKY: &str = "Send to Kandinsky";
/// Kandinsky CLI subcommand (`00-overview.md` 2.11, "Sending papers back").
pub const KANDINSKY_CAPTURE_SUBCOMMAND: &str = "capture-paper";
/// Kandinsky custom URL scheme (`00-overview.md` 2.11, "Sending papers back").
pub const KANDINSKY_DEEP_LINK_SCHEME: &str = "kandinsky";
/// Kandinsky custom URL host/action (`00-overview.md` 2.11, "Sending papers back").
pub const KANDINSKY_DEEP_LINK_ACTION: &str = "capture-paper";
/// Default executable name for Kandinsky CLI invocation (never resolved through a shell).
pub const KANDINSKY_DEFAULT_BINARY: &str = "kandinsky";

const MAX_CAPTURE_TARGET_BYTES: usize = 512;
const MAX_CLI_OUTPUT_BYTES: usize = 4096;
const MAX_DEEP_LINK_TITLE_CHARS: usize = 200;

/// RFC 3986 unreserved characters (`A-Z`, `a-z`, `0-9`, `-`, `_`, `.`, `~`) left unencoded in query values.
const QUERY_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

static EXPORT_TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Summary returned after an opt-in BibTeX file export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BibtexExportResult {
    pub output_path: String,
    pub bytes_written: usize,
    pub count: usize,
    pub citekeys: Vec<String>,
}

/// Validated target for `kandinsky capture-paper <target>` or `kandinsky://capture-paper?...`.
///
/// Guaranteed non-empty, ASCII-safe, free of whitespace, control characters, leading `-`,
/// shell metacharacters, and restricted to `arXiv:...`, `doi:...`, or `https://...`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KandinskyCaptureTarget(String);

impl KandinskyCaptureTarget {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Result of executing `kandinsky capture-paper <target>` directly via argv (no shell).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KandinskyCliOutcome {
    pub program: String,
    pub argv: Vec<String>,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Validate that `dest_path` is a safe user-chosen export file path:
/// - non-empty, absolute, free of NUL, control characters, and `..` traversal segments;
/// - existing parent directory; target itself is not a directory, symlink, or NTFS junction;
/// - NEVER inside any registered read-only `PaperSource` root (or Kandinsky/Zotero marker dir).
pub fn validate_export_destination(lib: &Library, dest_path: &Path) -> Result<PathBuf, String> {
    let raw = dest_path.to_string_lossy();
    if raw.trim().is_empty() || raw.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return Err("export destination path must not be empty or contain control characters".into());
    }
    if !dest_path.is_absolute() {
        return Err(format!(
            "export destination `{}` must be an absolute user-chosen path",
            dest_path.display()
        ));
    }
    for comp in dest_path.components() {
        if matches!(comp, Component::ParentDir) {
            return Err(format!(
                "export destination `{}` must not contain `..` traversal",
                dest_path.display()
            ));
        }
    }
    let parent = dest_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| format!("export destination `{}` has no parent directory", dest_path.display()))?;
    let file_name = dest_path
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty() && *n != "." && *n != "..")
        .ok_or_else(|| format!("invalid export file name in `{}`", dest_path.display()))?;
    if file_name.contains(['/', '\\', ':', '\0']) {
        return Err(format!("unsafe export file name `{file_name}`"));
    }

    if let Ok(raw_meta) = fs::symlink_metadata(dest_path) {
        if is_symlink_or_junction(dest_path, &raw_meta) {
            return Err(format!(
                "refused export to symlink or junction `{}`",
                dest_path.display()
            ));
        }
        if raw_meta.is_dir() {
            return Err(format!(
                "export destination `{}` is a directory",
                dest_path.display()
            ));
        }
    }

    let canon_parent = strip_verbatim_prefix(
        &WorkspaceSandbox::canonicalize_existing_dir(parent)
            .map_err(|e| format!("invalid export parent directory `{}`: {e}", parent.display()))?,
    );
    let canon_dest = canon_parent.join(file_name);
    if let Ok(meta) = fs::symlink_metadata(&canon_dest) {
        if is_symlink_or_junction(&canon_dest, &meta) {
            return Err(format!(
                "refused export to symlink or junction `{}`",
                canon_dest.display()
            ));
        }
        if meta.is_dir() {
            return Err(format!(
                "export destination `{}` is a directory",
                canon_dest.display()
            ));
        }
    }

    // Refuse writing inside any registered read-only paper source root (or its extra roots).
    let protected_roots = collect_protected_source_roots(&lib.list_sources()?);
    for root in &protected_roots {
        if WorkspaceSandbox::is_same_or_inside(root, &canon_dest) {
            return Err(format!(
                "refused BibTeX export inside registered read-only paper source `{}`",
                root.display()
            ));
        }
    }

    // Defense-in-depth: never write anywhere inside a Kandinsky vault or Zotero directory
    // (including subdirectories such as `refs/papers/` or `storage/`) even when unregistered.
    for anc in canon_parent.ancestors() {
        if anc.join(".kandinsky").is_dir() || anc.join("zotero.sqlite").is_file() {
            return Err(format!(
                "refused BibTeX export inside a Kandinsky vault or Zotero directory `{}`",
                anc.display()
            ));
        }
    }

    Ok(canon_dest)
}

fn collect_protected_source_roots(sources: &[SourceRecord]) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for src in sources {
        match src.kind {
            SourceKind::Bibtex => {
                for r in crate::source_bibtex::allowed_roots_for_bibtex(
                    Path::new(&src.root),
                    &src.settings,
                ) {
                    if !roots.contains(&r) {
                        roots.push(r);
                    }
                }
            }
            SourceKind::Zotero => {
                for r in crate::source_zotero::allowed_roots_for_zotero(
                    Path::new(&src.root),
                    &src.settings,
                ) {
                    if !roots.contains(&r) {
                        roots.push(r);
                    }
                }
            }
            SourceKind::KandinskyVault => {
                if let Ok(canon) = canonicalize_source_root(Path::new(&src.root)) {
                    if !roots.contains(&canon) {
                        roots.push(canon.clone());
                    }
                    for r in crate::source_kandinsky::allowed_roots_for_vault(&canon, &src.settings) {
                        if !roots.contains(&r) {
                            roots.push(r);
                        }
                    }
                }
            }
            SourceKind::Folder => {
                if let Ok(canon) = canonicalize_source_root(Path::new(&src.root)) {
                    if !roots.contains(&canon) {
                        roots.push(canon);
                    }
                }
            }
        }
    }
    roots
}

/// Atomically write BibTeX content to a validated user-chosen destination path.
pub fn write_bibtex_export_atomic(
    lib: &Library,
    dest_path: &Path,
    bibtex_content: &str,
) -> Result<(PathBuf, usize), String> {
    let canon_dest = validate_export_destination(lib, dest_path)?;
    let bytes = bibtex_content.as_bytes();
    let parent = canon_dest
        .parent()
        .ok_or_else(|| "export destination missing parent".to_string())?;
    let base_name = canon_dest
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("export.bib");

    let (tmp_path, mut file) = loop {
        let seq = EXPORT_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let cand = parent.join(format!(".{base_name}.tmp.{}.{seq}", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&cand) {
            Ok(f) => break (cand, f),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("create temp export file `{}`: {e}", cand.display())),
        }
    };

    let write_res = file
        .write_all(bytes)
        .and_then(|()| file.sync_all());
    drop(file);
    if let Err(e) = write_res {
        let _ = fs::remove_file(&tmp_path);
        return Err(format!("write export file `{}`: {e}", canon_dest.display()));
    }
    // Re-verify parent identity and destination non-symlink status right before rename (TOCTOU guard).
    let rechecked_parent = fs::canonicalize(parent).map(|p| strip_verbatim_prefix(&p));
    let dest_swapped = fs::symlink_metadata(&canon_dest)
        .map(|m| is_symlink_or_junction(&canon_dest, &m) || m.is_dir())
        .unwrap_or(false);
    if rechecked_parent.as_deref().ok() != Some(parent) || dest_swapped {
        let _ = fs::remove_file(&tmp_path);
        return Err(format!(
            "refused export rename: destination `{}` changed concurrently",
            canon_dest.display()
        ));
    }
    if let Err(e) = fs::rename(&tmp_path, &canon_dest) {
        let _ = fs::remove_file(&tmp_path);
        return Err(format!(
            "atomic rename export file `{}`: {e}",
            canon_dest.display()
        ));
    }
    Ok((canon_dest, bytes.len()))
}

/// Format selected library papers and atomically write them to `dest_path`.
pub fn export_library_bibtex_to_path(
    lib: &Library,
    keys: &[PaperKey],
    dest_path: &Path,
) -> Result<BibtexExportResult, String> {
    let (content, citekeys) = format_library_papers_bibtex(lib, keys)?;
    let (canon_dest, bytes_written) = write_bibtex_export_atomic(lib, dest_path, &content)?;
    Ok(BibtexExportResult {
        output_path: portable_path_string(&canon_dest),
        bytes_written,
        count: citekeys.len(),
        citekeys,
    })
}

/// Strictly validate a Kandinsky `capture-paper` target (`arXiv:...`, `doi:...`, or `https://...`).
///
/// Rejects option injection (leading `-`), whitespace, control characters, shell metacharacters,
/// and non-HTTPS schemes (`http://`, `file://`, `javascript:`, etc.).
pub fn validate_kandinsky_capture_target(raw: &str) -> Result<KandinskyCaptureTarget, String> {
    if raw.is_empty() || raw.len() > MAX_CAPTURE_TARGET_BYTES {
        return Err("capture target must be 1..=512 bytes".into());
    }
    if raw.starts_with('-') {
        return Err("capture target must not start with `-` (option injection rejected)".into());
    }
    if !raw.is_ascii()
        || raw
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || is_forbidden_shell_char(c))
    {
        return Err(
            "capture target must be ASCII and free of whitespace, control, or shell metacharacters"
                .into(),
        );
    }

    let low = raw.to_ascii_lowercase();
    if let Some(rest) = low.strip_prefix("arxiv:") {
        let orig_rest = &raw[raw.len() - rest.len()..];
        let (base, ver) = split_arxiv_id(orig_rest)
            .map_err(|e| format!("invalid arXiv capture target `{raw}`: {e}"))?;
        let normalized = match ver {
            Some(v) => format!("arXiv:{base}{v}"),
            None => format!("arXiv:{base}"),
        };
        return Ok(KandinskyCaptureTarget(normalized));
    }
    if let Some(rest) = low.strip_prefix("doi:") {
        let orig_rest = &raw[raw.len() - rest.len()..];
        let norm = canonical_doi(orig_rest)
            .map_err(|e| format!("invalid DOI capture target `{raw}`: {e}"))?;
        if norm
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || is_forbidden_shell_char(c))
        {
            return Err(format!("DOI capture target `{raw}` contains disallowed characters"));
        }
        return Ok(KandinskyCaptureTarget(format!("doi:{norm}")));
    }
    if let Some(after_scheme) = raw.strip_prefix("https://") {
        let (authority, _path) = match after_scheme.find(['/', '?', '#']) {
            Some(idx) => (&after_scheme[..idx], &after_scheme[idx..]),
            None => (after_scheme, ""),
        };
        if authority.is_empty() || authority.contains('@') || authority.contains("..") {
            return Err(format!("invalid HTTPS host in capture target `{raw}`"));
        }
        let host = authority.split(':').next().unwrap_or("");
        if host.is_empty()
            || !host.contains('.')
            || host.starts_with('.')
            || host.ends_with('.')
            || !host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
        {
            return Err(format!("invalid HTTPS host `{host}` in capture target `{raw}`"));
        }
        if let Some((_, port_str)) = authority.split_once(':') {
            if port_str.is_empty() || port_str.parse::<u16>().is_err() {
                return Err(format!("invalid HTTPS port in capture target `{raw}`"));
            }
        }
        return Ok(KandinskyCaptureTarget(raw.to_string()));
    }

    Err(format!(
        "capture target `{raw}` must start with `arXiv:`, `doi:`, or `https://`"
    ))
}

fn is_forbidden_shell_char(c: char) -> bool {
    matches!(
        c,
        ';' | '|' | '&' | '$' | '`' | '<' | '>' | '(' | ')' | '\'' | '"' | '\\' | '!' | '*'
    )
}

/// Resolve a validated Kandinsky capture target from an indexed library paper (arXiv preferred,
/// then DOI). Local-only `file:` papers without an external identifier return an error.
pub fn resolve_kandinsky_target_for_paper(
    lib: &Library,
    key: &PaperKey,
) -> Result<(KandinskyCaptureTarget, String), String> {
    let canonical = lib.resolve_key(key)?.unwrap_or_else(|| key.clone());
    let detail = lib
        .get_paper(&canonical)?
        .ok_or_else(|| format!("paper `{canonical}` not found in library"))?;

    if let Some(ax) = detail.paper.ids.arxiv.as_deref() {
        let ver = detail
            .source_items
            .iter()
            .find_map(|it| it.version.as_deref())
            .map(str::trim)
            .filter(|v| !v.is_empty());
        let raw = match ver {
            Some(v) => format!("arXiv:{ax}{v}"),
            None => format!("arXiv:{ax}"),
        };
        return Ok((validate_kandinsky_capture_target(&raw)?, detail.paper.title));
    }
    if let Some(doi) = detail.paper.ids.doi.as_deref() {
        let raw = format!("doi:{doi}");
        return Ok((validate_kandinsky_capture_target(&raw)?, detail.paper.title));
    }
    Err(format!(
        "paper `{canonical}` has no arXiv id or DOI to send to Kandinsky"
    ))
}

/// Build the argv vector for `kandinsky capture-paper <validated_target>` (never via shell).
pub fn build_kandinsky_capture_argv(target: &KandinskyCaptureTarget) -> Vec<String> {
    vec![
        KANDINSKY_CAPTURE_SUBCOMMAND.to_string(),
        target.as_str().to_string(),
    ]
}

/// Build a validated, percent-encoded `kandinsky://capture-paper?...` deep link.
pub fn build_kandinsky_deep_link(target: &KandinskyCaptureTarget, title: Option<&str>) -> String {
    let enc_target = utf8_percent_encode(target.as_str(), QUERY_ENCODE_SET);
    let mut url = format!(
        "{KANDINSKY_DEEP_LINK_SCHEME}://{KANDINSKY_DEEP_LINK_ACTION}?target={enc_target}"
    );
    if let Some(raw_title) = title {
        let clean: String = raw_title
            .chars()
            .filter(|c| !c.is_control())
            .take(MAX_DEEP_LINK_TITLE_CHARS)
            .collect();
        let trimmed = clean.trim();
        if !trimmed.is_empty() {
            let enc_title = utf8_percent_encode(trimmed, QUERY_ENCODE_SET);
            let _ = write!(url, "&title={enc_title}");
        }
    }
    url
}

/// Execute `kandinsky capture-paper <validated_target>` directly via `std::process::Command`
/// without invoking any shell (`cmd /c` or `sh -c`).
pub fn invoke_kandinsky_cli(
    bin: &Path,
    target: &KandinskyCaptureTarget,
) -> Result<KandinskyCliOutcome, String> {
    let bin_str = bin.to_string_lossy();
    if bin_str.trim().is_empty() || bin_str.contains('\0') {
        return Err("Kandinsky executable path must not be empty or contain NUL".into());
    }
    // Re-verify target invariants at the execution boundary.
    let verified = validate_kandinsky_capture_target(target.as_str())?;
    let argv = build_kandinsky_capture_argv(&verified);
    let output = Command::new(bin)
        .args(&argv)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("failed to launch `{}`: {e}", bin.display()))?;
    let stdout = truncate_utf8(&output.stdout, MAX_CLI_OUTPUT_BYTES);
    let stderr = truncate_utf8(&output.stderr, MAX_CLI_OUTPUT_BYTES);
    Ok(KandinskyCliOutcome {
        program: portable_path_string(bin),
        argv,
        exit_code: output.status.code(),
        success: output.status.success(),
        stdout,
        stderr,
    })
}

fn truncate_utf8(bytes: &[u8], max_bytes: usize) -> String {
    let slice = if bytes.len() > max_bytes {
        &bytes[..max_bytes]
    } else {
        bytes
    };
    String::from_utf8_lossy(slice).into_owned()
}

#[cfg(test)]
#[path = "source_export_tests.rs"]
mod tests;
