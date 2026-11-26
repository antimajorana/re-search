//! Overlap-safe `user_library` fulltext helpers used inside [`crate::library_db_sync`]
//! transactions (`BEGIN IMMEDIATE`).

use crate::library_rows::LibraryFulltextRow;
use crate::library_schema::ORIGIN_USER_LIBRARY;
use crate::{is_sha256_hex, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::io;

#[derive(Debug, Clone)]
pub(crate) struct ActiveSourceFile {
    pub(crate) path: String,
    pub(crate) role: String,
    pub(crate) size: u64,
    pub(crate) mtime_ms: i64,
    pub(crate) sha256: Option<String>,
    pub(crate) rank: u8,
}

pub(crate) type SourcePathsAndKeys = (Vec<String>, Vec<(String, String)>);
pub(crate) type ActiveFilesIndex = (BTreeSet<String>, BTreeMap<(String, String), ActiveSourceFile>);

pub(crate) fn collect_source_paths_and_keys(
    conn: &Connection,
    source_id: &str,
) -> Result<SourcePathsAndKeys> {
    let mut stmt = conn.prepare(
        "SELECT paper_key, version, files_json FROM source_item WHERE source_id = ?1",
    )?;
    let rows = stmt.query_map(params![source_id], |r| {
        Ok((
            r.get::<_, Option<String>>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let mut paths = Vec::new();
    let mut keys = Vec::new();
    for row in rows {
        let (pk, ver, files_json) = row?;
        if let Some(k) = pk.filter(|s| !s.is_empty()) {
            let v = ver.as_deref().map(str::trim).unwrap_or("").to_string();
            keys.push((k, v));
        }
        for file in parse_non_note_files(&files_json) {
            paths.push(file.path);
        }
    }
    Ok((paths, keys))
}

pub(crate) fn collect_active_source_files(conn: &Connection) -> Result<ActiveFilesIndex> {
    let mut stmt = conn.prepare(
        "SELECT paper_key, version, files_json
         FROM source_item
         WHERE state <> 'missing'
         ORDER BY source_id ASC, external_id ASC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, Option<String>>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let mut active_paths = BTreeSet::new();
    let mut by_key: BTreeMap<(String, String), ActiveSourceFile> = BTreeMap::new();
    for row in rows {
        let (pk, ver, files_json) = row?;
        let ver_norm = ver.as_deref().map(str::trim).unwrap_or("").to_string();
        for f in parse_non_note_files(&files_json) {
            active_paths.insert(normalize_index_path(&f.path));
            if let Some(ref key) = pk {
                if !key.is_empty() {
                    let kv = (key.clone(), ver_norm.clone());
                    let replace = match by_key.get(&kv) {
                        None => true,
                        Some(cur) => {
                            f.rank < cur.rank
                                || (f.rank == cur.rank
                                    && f.sha256.is_some()
                                    && cur.sha256.is_none())
                        }
                    };
                    if replace {
                        by_key.insert(kv, f);
                    }
                }
            }
        }
    }
    Ok((active_paths, by_key))
}

fn parse_non_note_files(files_json: &str) -> Vec<ActiveSourceFile> {
    let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(files_json) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in arr {
        let Some(obj) = item.as_object() else { continue };
        let role = obj.get("role").and_then(Value::as_str).unwrap_or("");
        if role == "note" {
            continue;
        }
        let path = obj.get("path").and_then(Value::as_str).unwrap_or("").trim();
        if path.is_empty() {
            continue;
        }
        let size = obj.get("size").and_then(Value::as_u64).unwrap_or(0);
        let mtime_ms = obj.get("mtime_ms").and_then(Value::as_i64).unwrap_or(0);
        let sha256 = obj
            .get("sha256")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| is_sha256_hex(s))
            .map(str::to_string);
        let rank = match role {
            "tex" => 0,
            "pdf" => 1,
            _ => 2,
        };
        out.push(ActiveSourceFile {
            path: path.to_string(),
            role: role.to_string(),
            size,
            mtime_ms,
            sha256,
            rank,
        });
    }
    out
}

pub(crate) fn upsert_user_fulltext_in_tx(
    conn: &Connection,
    row: &LibraryFulltextRow,
    active_paths: &BTreeSet<String>,
) -> Result<()> {
    let ver = row.version.as_deref().map(str::trim).unwrap_or("");
    let existing = conn
        .query_row(
            "SELECT origin, format, file_path, file_size, file_mtime_ms, file_sha256
             FROM fulltext WHERE paper_key = ?1 AND version = ?2",
            params![row.paper_key, ver],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .optional()?;

    let mut effective_sha = row.file_sha256.clone();
    if let Some((ex_origin, ex_format, ex_path, ex_size, ex_mtime, ex_sha)) = existing {
        // Never overwrite a fulltext row owned by an unrelated origin (e.g. arxiv_eprint, ledger).
        if ex_origin != ORIGIN_USER_LIBRARY {
            return Ok(());
        }
        // Preserve an active TeX user_library source over an incoming PDF from an overlapping source.
        if ex_format != "pdf"
            && row.format == "pdf"
            && ex_path
                .as_deref()
                .is_some_and(|p| active_paths.contains(&normalize_index_path(p)))
        {
            return Ok(());
        }
        if effective_sha.is_none()
            && ex_path == row.file_path
            && ex_size == row.file_size.and_then(|v| i64::try_from(v).ok())
            && ex_mtime == row.file_mtime_ms
        {
            effective_sha = ex_sha;
        }
    }

    let bytes_i64 = i64::try_from(row.bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "bytes overflow"))?;
    let file_size_i64 = row
        .file_size
        .map(i64::try_from)
        .transpose()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "file_size overflow"))?;
    conn.execute(
        "INSERT INTO fulltext (
             paper_key, version, raw_blob, format, bytes, origin, fetched_ms,
             file_path, file_size, file_mtime_ms, file_sha256
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(paper_key, version) DO UPDATE SET
             raw_blob = excluded.raw_blob,
             format = excluded.format,
             bytes = excluded.bytes,
             origin = excluded.origin,
             fetched_ms = excluded.fetched_ms,
             file_path = excluded.file_path,
             file_size = excluded.file_size,
             file_mtime_ms = excluded.file_mtime_ms,
             file_sha256 = excluded.file_sha256
         WHERE fulltext.origin = ?12",
        params![
            row.paper_key,
            ver,
            row.raw_blob,
            row.format,
            bytes_i64,
            ORIGIN_USER_LIBRARY,
            row.fetched_ms,
            row.file_path,
            file_size_i64,
            row.file_mtime_ms,
            effective_sha,
            ORIGIN_USER_LIBRARY,
        ],
    )?;
    Ok(())
}

pub(crate) fn prune_unreferenced_user_fulltext_in_tx(
    conn: &Connection,
    candidate_paths: &[String],
    active_paths: &BTreeSet<String>,
) -> Result<usize> {
    let stale_norm: BTreeSet<String> = candidate_paths
        .iter()
        .map(|p| normalize_index_path(p))
        .filter(|norm| !norm.is_empty() && !active_paths.contains(norm))
        .collect();
    if stale_norm.is_empty() {
        return Ok(0);
    }

    let targets: Vec<(String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT paper_key, version, file_path
             FROM fulltext
             WHERE origin = ?1 AND file_path IS NOT NULL",
        )?;
        let rows = stmt.query_map(params![ORIGIN_USER_LIBRARY], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (pk, ver, path) = row?;
            if stale_norm.contains(&normalize_index_path(&path)) {
                out.push((pk, ver));
            }
        }
        out
    };

    let mut removed = 0usize;
    for (pk, ver) in targets {
        removed += conn.execute(
            "DELETE FROM fulltext WHERE paper_key = ?1 AND version = ?2 AND origin = ?3",
            params![pk, ver, ORIGIN_USER_LIBRARY],
        )?;
    }
    Ok(removed)
}

pub(crate) fn restore_surviving_user_fulltext_in_tx(
    conn: &Connection,
    affected_keys: &[(String, String)],
    active_by_key: &BTreeMap<(String, String), ActiveSourceFile>,
    ts_ms: i64,
) -> Result<()> {
    let unique: BTreeSet<&(String, String)> = affected_keys.iter().collect();
    for kv in unique {
        let Some(file) = active_by_key.get(kv) else {
            continue;
        };
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM fulltext WHERE paper_key = ?1 AND version = ?2)",
            params![kv.0, kv.1],
            |r| r.get(0),
        )?;
        if exists {
            continue;
        }
        let format = format_from_path_and_role(&file.path, &file.role);
        let size_i64 = i64::try_from(file.size)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "file_size overflow"))?;
        conn.execute(
            "INSERT OR IGNORE INTO fulltext (
                 paper_key, version, raw_blob, format, bytes, origin, fetched_ms,
                 file_path, file_size, file_mtime_ms, file_sha256
             ) VALUES (?1, ?2, '', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                kv.0,
                kv.1,
                format,
                size_i64,
                ORIGIN_USER_LIBRARY,
                ts_ms,
                file.path,
                size_i64,
                file.mtime_ms,
                file.sha256,
            ],
        )?;
    }
    Ok(())
}

fn format_from_path_and_role(path: &str, role: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        "tar.gz"
    } else if lower.ends_with(".zip") {
        "zip"
    } else if lower.ends_with(".tex") || role == "tex" {
        "tex"
    } else {
        "pdf"
    }
}

pub(crate) fn normalize_index_path(raw: &str) -> String {
    let mut s = raw.trim().replace('\\', "/");
    if let Some(rest) = s.strip_prefix("//?/") {
        s = rest.to_string();
    }
    while s.len() > 1 && s.ends_with('/') {
        s.pop();
    }
    #[cfg(windows)]
    {
        s.make_ascii_lowercase();
    }
    s
}
