//! Safe bounded extraction for `.zip`, `.tar.gz`/`.tgz`, and single-file `.gz` archives.
//! Enforces strict raw path validation, anti-bomb `Read::take` caps, and zero symlinks/devices.

use crate::sandbox::sanitize_rel_components;
use flate2::read::GzDecoder;
use std::collections::BTreeSet;
use std::fmt;
use std::io::{Cursor, Read};

pub const MAX_ARCHIVE_COMPRESSED_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_ENTRY_UNPACKED_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_CUMULATIVE_UNPACKED_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_ARCHIVE_ENTRIES: usize = 256;
const MAX_RAW_HEADER_RECORDS: usize = 1_024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveEntry {
    pub rel_path: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchivePayload {
    Files(Vec<ArchiveEntry>),
    SingleGzipText { suggested_name: String, text: String },
    RawPdf(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveError {
    UnsafeEntry(String),
    EncryptedArchive,
    Zip64Unsupported,
    LimitExceeded(String),
    BinarySingleGzip,
    CorruptArchive(String),
}

impl fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsafeEntry(m) => write!(f, "unsafe archive entry: {m}"),
            Self::EncryptedArchive => write!(f, "encrypted archives are forbidden"),
            Self::Zip64Unsupported => write!(f, "ZIP64 archives are forbidden"),
            Self::LimitExceeded(m) => write!(f, "archive limit exceeded: {m}"),
            Self::BinarySingleGzip => write!(f, "single-file .gz payload contains binary NUL bytes"),
            Self::CorruptArchive(m) => write!(f, "corrupt archive: {m}"),
        }
    }
}

impl std::error::Error for ArchiveError {}

/// Validates raw archive entry path bytes without relying on `ZipFile::enclosed_name()`
/// (which permits internal `foo/../bar`). Returns `Ok(None)` for valid directory entries.
pub fn validate_archive_entry_path(raw: &[u8], is_dir_flag: bool) -> Result<Option<String>, ArchiveError> {
    let s = std::str::from_utf8(raw).map_err(|_| ArchiveError::UnsafeEntry("non-UTF-8 entry name".into()))?;
    if s.is_empty() {
        return Err(ArchiveError::UnsafeEntry("empty entry name".into()));
    }
    if s.bytes().any(|b| b < 0x20 || b == 0x7f) || s.contains('\\') || s.contains(':') || s.starts_with('/') {
        return Err(ArchiveError::UnsafeEntry(format!("disallowed path `{s}`")));
    }
    let has_trailing_slash = s.ends_with('/');
    let trimmed = s.trim_end_matches('/');
    let stripped = trimmed.strip_prefix("./").unwrap_or(trimmed);
    if stripped.is_empty() || stripped == "." {
        if is_dir_flag || has_trailing_slash {
            return Ok(None);
        }
        return Err(ArchiveError::UnsafeEntry(format!("invalid file entry `{s}`")));
    }
    for seg in stripped.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            return Err(ArchiveError::UnsafeEntry(format!("parent traversal `..` in `{s}`")));
        }
    }
    let (_, norm) = sanitize_rel_components(stripped, false)
        .map_err(|e| ArchiveError::UnsafeEntry(format!("`{s}`: {e}")))?;
    if is_dir_flag || has_trailing_slash {
        Ok(None)
    } else {
        Ok(Some(norm))
    }
}

fn read_bounded_entry<R: Read>(mut reader: R, cumulative: &mut usize, label: &str) -> Result<Vec<u8>, ArchiveError> {
    let remaining_total = MAX_CUMULATIVE_UNPACKED_BYTES.saturating_sub(*cumulative);
    let cap = MAX_ENTRY_UNPACKED_BYTES.min(remaining_total);
    let mut buf = Vec::new();
    (&mut reader)
        .take((cap as u64) + 1)
        .read_to_end(&mut buf)
        .map_err(|e| ArchiveError::CorruptArchive(format!("reading `{label}`: {e}")))?;
    if buf.len() > MAX_ENTRY_UNPACKED_BYTES {
        return Err(ArchiveError::LimitExceeded(format!(
            "entry `{label}` exceeds 16 MiB unpacked limit"
        )));
    }
    if *cumulative + buf.len() > MAX_CUMULATIVE_UNPACKED_BYTES {
        return Err(ArchiveError::LimitExceeded(
            "cumulative unpacked archive size exceeds 32 MiB limit".into(),
        ));
    }
    *cumulative += buf.len();
    Ok(buf)
}

fn has_zip64_markers(bytes: &[u8]) -> bool {
    let tail_start = bytes.len().saturating_sub(65_600);
    let tail = &bytes[tail_start..];
    if tail.windows(4).any(|w| w == b"PK\x06\x06" || w == b"PK\x06\x07") {
        return true;
    }
    if let Some(pos) = tail.windows(4).rposition(|w| w == b"PK\x05\x06") {
        let eocd = &tail[pos..];
        if eocd.len() >= 22 {
            let total_entries = u16::from_le_bytes([eocd[10], eocd[11]]);
            let cd_size = u32::from_le_bytes([eocd[12], eocd[13], eocd[14], eocd[15]]);
            let cd_offset = u32::from_le_bytes([eocd[16], eocd[17], eocd[18], eocd[19]]);
            if total_entries == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_offset == 0xFFFF_FFFF {
                return true;
            }
        }
    }
    false
}

fn check_unix_mode_regular_or_dir(mode: Option<u32>, label: &str) -> Result<(), ArchiveError> {
    if let Some(m) = mode {
        let kind = m & 0o170_000;
        if !matches!(kind, 0 | 0o100_000 | 0o040_000) {
            return Err(ArchiveError::UnsafeEntry(format!(
                "special unix file mode {m:#o} on `{label}`"
            )));
        }
    }
    Ok(())
}

pub fn extract_zip_bytes(bytes: &[u8]) -> Result<Vec<ArchiveEntry>, ArchiveError> {
    if bytes.len() > MAX_ARCHIVE_COMPRESSED_BYTES {
        return Err(ArchiveError::LimitExceeded("compressed ZIP exceeds 32 MiB".into()));
    }
    if has_zip64_markers(bytes) {
        return Err(ArchiveError::Zip64Unsupported);
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| {
            let msg = e.to_string();
            if msg.to_ascii_lowercase().contains("password") || msg.to_ascii_lowercase().contains("encrypt") {
                ArchiveError::EncryptedArchive
            } else {
                ArchiveError::CorruptArchive(msg)
            }
        })?;
    if archive.raw_zip64_extensible_data_sector().is_some() {
        return Err(ArchiveError::Zip64Unsupported);
    }
    archive
        .validate_header_consistency()
        .map_err(|e| ArchiveError::CorruptArchive(format!("header consistency: {e}")))?;

    if archive.len() > MAX_RAW_HEADER_RECORDS {
        return Err(ArchiveError::LimitExceeded(format!(
            "ZIP record count ({}) exceeds limit",
            archive.len()
        )));
    }

    let mut entries = Vec::new();
    let mut seen_lower = BTreeSet::new();
    let mut cumulative = 0usize;

    for idx in 0..archive.len() {
        {
            let meta = archive
                .by_index_data(idx)
                .map_err(|e| ArchiveError::CorruptArchive(e.to_string()))?;
            if meta.encrypted() {
                return Err(ArchiveError::EncryptedArchive);
            }
            if meta.version_needed() >= 45 {
                return Err(ArchiveError::Zip64Unsupported);
            }
            if meta.is_symlink() {
                return Err(ArchiveError::UnsafeEntry("symlink entry in ZIP".into()));
            }
            check_unix_mode_regular_or_dir(meta.unix_mode(), &String::from_utf8_lossy(meta.name_raw()))?;
        }

        let file = archive.by_index(idx).map_err(|e| {
            let msg = e.to_string();
            if msg.to_ascii_lowercase().contains("password") || msg.to_ascii_lowercase().contains("encrypt") {
                ArchiveError::EncryptedArchive
            } else {
                ArchiveError::CorruptArchive(msg)
            }
        })?;
        if file.encrypted() {
            return Err(ArchiveError::EncryptedArchive);
        }
        if file.is_symlink() {
            return Err(ArchiveError::UnsafeEntry("symlink entry in ZIP".into()));
        }
        for extra in file.extra_data_fields() {
            if matches!(extra, zip::ExtraField::Zip64ExtendedInformation(_)) {
                return Err(ArchiveError::Zip64Unsupported);
            }
        }
        let is_dir = file.is_dir();
        let Some(norm_path) = validate_archive_entry_path(file.name_raw(), is_dir)? else {
            continue;
        };
        if entries.len() >= MAX_ARCHIVE_ENTRIES {
            return Err(ArchiveError::LimitExceeded(format!(
                "archive exceeds {MAX_ARCHIVE_ENTRIES} file limit"
            )));
        }
        if !seen_lower.insert(norm_path.to_ascii_lowercase()) {
            return Err(ArchiveError::UnsafeEntry(format!("duplicate/colliding path `{norm_path}`")));
        }
        let data = read_bounded_entry(file, &mut cumulative, &norm_path)?;
        entries.push(ArchiveEntry { rel_path: norm_path, bytes: data });
    }

    Ok(entries)
}

pub fn extract_tar_bytes(bytes: &[u8]) -> Result<Vec<ArchiveEntry>, ArchiveError> {
    let mut archive = tar::Archive::new(Cursor::new(bytes));
    let iter = archive.entries().map_err(|e| ArchiveError::CorruptArchive(e.to_string()))?;

    let mut entries = Vec::new();
    let mut seen_lower = BTreeSet::new();
    let mut cumulative = 0usize;
    let mut raw_records = 0usize;

    for item in iter {
        raw_records += 1;
        if raw_records > MAX_RAW_HEADER_RECORDS {
            return Err(ArchiveError::LimitExceeded("TAR header record count exceeded".into()));
        }
        let entry = item.map_err(|e| ArchiveError::CorruptArchive(e.to_string()))?;
        let entry_type = entry.header().entry_type();
        let raw_name = entry.path_bytes().into_owned();
        let lossy_name = String::from_utf8_lossy(&raw_name).into_owned();

        match entry_type {
            tar::EntryType::XHeader | tar::EntryType::XGlobalHeader | tar::EntryType::GNULongName => continue,
            tar::EntryType::Directory => {
                let _ = validate_archive_entry_path(&raw_name, true)?;
                continue;
            }
            tar::EntryType::Regular => {
                if let Ok(mode) = entry.header().mode() {
                    check_unix_mode_regular_or_dir(Some(mode), &lossy_name)?;
                }
            }
            tar::EntryType::Symlink | tar::EntryType::Link | tar::EntryType::GNULongLink => {
                return Err(ArchiveError::UnsafeEntry(format!("link entry `{lossy_name}` forbidden in TAR")));
            }
            _ => {
                return Err(ArchiveError::UnsafeEntry(format!(
                    "special TAR entry type `{entry_type:?}` on `{lossy_name}` forbidden"
                )));
            }
        }

        let Some(norm_path) = validate_archive_entry_path(&raw_name, false)? else {
            continue;
        };
        if entries.len() >= MAX_ARCHIVE_ENTRIES {
            return Err(ArchiveError::LimitExceeded(format!(
                "archive exceeds {MAX_ARCHIVE_ENTRIES} file limit"
            )));
        }
        if !seen_lower.insert(norm_path.to_ascii_lowercase()) {
            return Err(ArchiveError::UnsafeEntry(format!("duplicate/colliding path `{norm_path}`")));
        }
        let data = read_bounded_entry(entry, &mut cumulative, &norm_path)?;
        entries.push(ArchiveEntry { rel_path: norm_path, bytes: data });
    }

    Ok(entries)
}

fn is_probable_tar(bytes: &[u8]) -> bool {
    if bytes.len() < 512 {
        return false;
    }
    let block = &bytes[..512];
    if block.iter().all(|&b| b == 0) {
        return false;
    }
    if &block[257..262] == b"ustar" {
        return true;
    }
    let chk_field = &block[148..156];
    let chk_str = std::str::from_utf8(chk_field)
        .unwrap_or("")
        .trim_matches(|c: char| c == '\0' || c.is_ascii_whitespace());
    let Ok(stored_sum) = u32::from_str_radix(chk_str, 8) else {
        return false;
    };
    let mut unsigned_sum = 0u32;
    let mut signed_sum = 0i32;
    for (i, &b) in block.iter().enumerate() {
        let v = if (148..156).contains(&i) { b' ' } else { b };
        unsigned_sum += v as u32;
        signed_sum += (v as i8) as i32;
    }
    stored_sum == unsigned_sum || (signed_sum >= 0 && stored_sum == signed_sum as u32)
}

const KNOWN_GZIP_INNER_EXTS: &[&str] = &[
    "tex", "ltx", "sty", "cls", "bib", "bbl", "md", "markdown", "txt", "csv", "tsv", "json", "py",
    "log", "yml", "yaml",
];

fn suggested_single_gzip_name(file_hint: &str) -> String {
    let base = file_hint.rsplit(['/', '\\']).next().unwrap_or(file_hint);
    let stripped = if base.len() > 3 && base.as_bytes()[base.len() - 3..].eq_ignore_ascii_case(b".gz") {
        &base[..base.len() - 3]
    } else {
        base
    };
    let clean: String = stripped
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' })
        .collect();
    let trimmed = clean.trim_end_matches(['.', ' ']);
    let stem = if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        "source".to_string()
    } else {
        trimmed.to_string()
    };
    let has_known_ext = stem
        .rsplit_once('.')
        .is_some_and(|(_, ext)| KNOWN_GZIP_INNER_EXTS.contains(&ext.to_ascii_lowercase().as_str()));
    let candidate = if has_known_ext {
        stem
    } else {
        format!("{stem}.tex")
    };
    sanitize_rel_components(&candidate, false)
        .map(|(_, norm)| norm)
        .unwrap_or_else(|_| "source.tex".to_string())
}

pub fn extract_gzip_or_tar_gz(
    bytes: &[u8],
    file_hint: &str,
    expect_tar_only: bool,
) -> Result<ArchivePayload, ArchiveError> {
    if bytes.len() > MAX_ARCHIVE_COMPRESSED_BYTES {
        return Err(ArchiveError::LimitExceeded("compressed GZIP exceeds 32 MiB".into()));
    }
    let mut gunzipped = Vec::new();
    GzDecoder::new(Cursor::new(bytes))
        .take((MAX_CUMULATIVE_UNPACKED_BYTES as u64) + 1)
        .read_to_end(&mut gunzipped)
        .map_err(|e| ArchiveError::CorruptArchive(format!("gzip decode error: {e}")))?;
    if gunzipped.len() > MAX_CUMULATIVE_UNPACKED_BYTES {
        return Err(ArchiveError::LimitExceeded(
            "unpacked GZIP stream exceeds 32 MiB anti-bomb limit".into(),
        ));
    }

    if !expect_tar_only && gunzipped[..gunzipped.len().min(1024)].windows(5).any(|w| w == b"%PDF-") {
        return Ok(ArchivePayload::RawPdf(gunzipped));
    }

    if expect_tar_only || is_probable_tar(&gunzipped) {
        return extract_tar_bytes(&gunzipped).map(ArchivePayload::Files);
    }

    if gunzipped.len() > MAX_ENTRY_UNPACKED_BYTES {
        return Err(ArchiveError::LimitExceeded(
            "single-file .gz payload exceeds 16 MiB limit".into(),
        ));
    }
    if gunzipped.contains(&0u8) {
        return Err(ArchiveError::BinarySingleGzip);
    }
    let text = match std::str::from_utf8(&gunzipped) {
        Ok(s) => s.to_string(),
        Err(_) => gunzipped.iter().map(|&b| char::from(b)).collect(),
    };
    Ok(ArchivePayload::SingleGzipText {
        suggested_name: suggested_single_gzip_name(file_hint),
        text,
    })
}
