//! Atomic F13a source sync commits and overlap-safe source unregistration for [`LibraryDb`].
//!
//! Keeps every multi-table index mutation (`source_item`, `paper`, `alias`, `fulltext`,
//! `paper_fts`, and the `source` last-sync marker) inside a single `BEGIN IMMEDIATE`
//! transaction while leaving all expensive filesystem scans outside the write lock.

use crate::library_db::{validate_non_empty, LibraryDb};
use crate::library_db_query::refresh_fts_metadata_in_tx;
use crate::library_db_sync_ft::{
    collect_active_source_files, collect_source_paths_and_keys, prune_rekeyed_user_fulltext_in_tx,
    prune_unreferenced_user_fulltext_in_tx, restore_surviving_user_fulltext_in_tx,
    upsert_user_fulltext_in_tx,
};
use crate::library_rows::{
    row_to_paper, LibraryFulltextRow, LibrarySourceRow, PaperUpdateMode, SourceSyncBatch,
    SourceSyncPaperUpdate, SourceSyncSnapshot,
};
use crate::library_schema::ORIGIN_USER_LIBRARY;
use crate::{is_sha256_hex, now_ms, Error, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::io;

/// RAII guard for a `BEGIN IMMEDIATE` transaction on `&Connection` (zero `unsafe`).
pub(crate) struct ImmediateTx<'a> {
    pub(crate) conn: &'a Connection,
    committed: bool,
}

impl<'a> ImmediateTx<'a> {
    pub(crate) fn begin(conn: &'a Connection) -> Result<Self> {
        if !conn.is_autocommit() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot begin nested immediate transaction on library database",
            )
            .into());
        }
        conn.execute_batch("BEGIN IMMEDIATE")?;
        Ok(Self {
            conn,
            committed: false,
        })
    }

    pub(crate) fn commit(mut self) -> Result<()> {
        self.conn.execute_batch("COMMIT")?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for ImmediateTx<'_> {
    fn drop(&mut self) {
        if !self.committed {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
    }
}

impl LibraryDb {
    /// Atomically register a source if absent without clobbering an existing source's
    /// `last_sync_ms` or `state`.
    ///
    /// Returns `(row, true)` when newly inserted, `(existing, false)` when an identical source
    /// (`kind`, `root`, `settings_json`) is already registered, and an error if the same `id`
    /// already exists with a different configuration.
    pub fn register_source_atomic(&self, row: &LibrarySourceRow) -> Result<(LibrarySourceRow, bool)> {
        validate_non_empty("id", &row.id)?;
        validate_non_empty("root", &row.root)?;
        serde_json::from_str::<Value>(&row.settings_json)?;
        let tx = ImmediateTx::begin(&self.conn)?;
        if let Some(live) = query_source_row(tx.conn, &row.id)? {
            if live.kind == row.kind
                && live.root == row.root
                && live.settings_json == row.settings_json
            {
                tx.commit()?;
                return Ok((live, false));
            }
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "paper source `{}` is already registered with a different kind, root, or settings",
                    row.id
                ),
            )
            .into());
        }
        tx.conn.execute(
            "INSERT INTO source (id, kind, root, settings_json, added_ms, last_sync_ms, state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                row.id,
                row.kind,
                row.root,
                row.settings_json,
                row.added_ms,
                row.last_sync_ms,
                row.state,
            ],
        )?;
        tx.commit()?;
        Ok((row.clone(), true))
    }

    /// Atomically apply one source sync batch inside a single `BEGIN IMMEDIATE` transaction.
    ///
    /// Uses optimistic concurrency control (CAS) on `batch.expected_source`: if the `source`
    /// row was removed or modified while scanning, returns an error and rolls back every write
    /// without advancing `last_sync_ms` or leaving a partial index. Performs zero disk I/O.
    pub fn apply_source_sync_batch(&self, batch: &SourceSyncBatch) -> Result<i64> {
        validate_sync_batch(batch)?;
        let tx = ImmediateTx::begin(&self.conn)?;

        let live = query_source_row(tx.conn, &batch.expected_source.id)?.ok_or_else(|| {
            Error::Io(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "paper source `{}` was removed before sync commit",
                    batch.expected_source.id
                ),
            ))
        })?;
        let live_snap = SourceSyncSnapshot::from_source_row(&live);
        if live_snap != batch.expected_source {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "concurrent sync conflict for source `{}`: source changed while scanning (expected last_sync_ms={:?}, state={}; live last_sync_ms={:?}, state={})",
                    batch.expected_source.id,
                    batch.expected_source.last_sync_ms,
                    batch.expected_source.state,
                    live.last_sync_ms,
                    live.state,
                ),
            )
            .into());
        }

        let committed_sync_ms = match live.last_sync_ms {
            Some(prev) => batch.sync_ts_ms.max(prev.saturating_add(1)),
            None => batch.sync_ts_ms.max(1),
        };

        for row in &batch.upsert_source_items {
            let mut resolved = row.clone();
            if let Some(ref pk) = row.paper_key {
                resolved.paper_key = Some(resolve_live_canonical_key(tx.conn, pk)?);
            }
            upsert_source_item_in_tx(tx.conn, &resolved)?;
        }

        let mut touched_papers = BTreeSet::new();
        for upd in &batch.paper_updates {
            let mut resolved = upd.clone();
            resolved.paper_key = resolve_live_canonical_key(tx.conn, &upd.paper_key)?;
            if apply_paper_update_in_tx(tx.conn, &resolved)? {
                touched_papers.insert(resolved.paper_key);
            }
        }

        for (ident, canon_key) in &batch.aliases {
            let live_canon = resolve_live_canonical_key(tx.conn, canon_key)?;
            if ident == &live_canon {
                continue;
            }
            tx.conn.execute(
                "INSERT INTO alias (identifier, paper_key) VALUES (?1, ?2)
                 ON CONFLICT(identifier) DO UPDATE SET paper_key = excluded.paper_key",
                params![ident, live_canon],
            )?;
        }

        let active = collect_active_source_files(tx.conn)?;
        for ft in &batch.upsert_user_fulltext {
            let mut resolved = ft.clone();
            resolved.paper_key = resolve_live_canonical_key(tx.conn, &ft.paper_key)?;
            upsert_user_fulltext_in_tx(tx.conn, &resolved, &active.paths)?;
        }
        prune_unreferenced_user_fulltext_in_tx(tx.conn, &batch.stale_user_paths, &active.paths)?;
        prune_rekeyed_user_fulltext_in_tx(
            tx.conn,
            &batch.stale_user_fulltext_keys,
            &active.key_ver_paths,
        )?;
        restore_surviving_user_fulltext_in_tx(
            tx.conn,
            &batch.affected_fulltext_keys,
            &active.by_key,
            committed_sync_ms,
        )?;

        for key in &touched_papers {
            refresh_fts_metadata_in_tx(tx.conn, key)?;
        }

        let updated = tx.conn.execute(
            "UPDATE source SET last_sync_ms = ?1, state = ?2 WHERE id = ?3",
            params![committed_sync_ms, batch.next_state, batch.expected_source.id],
        )?;
        if updated != 1 {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "paper source `{}` disappeared before sync marker update",
                    batch.expected_source.id
                ),
            )
            .into());
        }

        tx.commit()?;
        Ok(committed_sync_ms)
    }

    /// Atomically remove a registered source and its `source_item` rows from `library.db` while
    /// preserving any `fulltext` path still indexed by another registered source and preserving
    /// every `fulltext` row from unrelated origins (`origin <> 'user_library'`).
    ///
    /// Safe to retry: unregistering a missing source is an idempotent no-op returning `Ok(false)`.
    pub fn unregister_source_atomic(&self, source_id: &str) -> Result<bool> {
        if source_id.trim().is_empty() {
            return Ok(false);
        }
        let tx = ImmediateTx::begin(&self.conn)?;
        if query_source_row(tx.conn, source_id)?.is_none() {
            tx.commit()?;
            return Ok(false);
        }

        let (candidate_paths, affected_keys) = collect_source_paths_and_keys(tx.conn, source_id)?;
        let removed = tx
            .conn
            .execute("DELETE FROM source WHERE id = ?1", params![source_id])?;
        if removed == 0 {
            tx.commit()?;
            return Ok(false);
        }

        let surviving = collect_active_source_files(tx.conn)?;
        prune_unreferenced_user_fulltext_in_tx(tx.conn, &candidate_paths, &surviving.paths)?;
        restore_surviving_user_fulltext_in_tx(
            tx.conn,
            &affected_keys,
            &surviving.by_key,
            now_ms(),
        )?;

        tx.commit()?;
        Ok(true)
    }

    /// Upsert a `user_library` fulltext row without overwriting any unrelated `fulltext.origin`.
    pub fn upsert_user_library_fulltext_preserve_other_origins(
        &self,
        row: &LibraryFulltextRow,
    ) -> Result<()> {
        validate_user_fulltext_row(row)?;
        let tx = ImmediateTx::begin(&self.conn)?;
        let active = collect_active_source_files(tx.conn)?;
        upsert_user_fulltext_in_tx(tx.conn, row, &active.paths)?;
        tx.commit()?;
        Ok(())
    }

    /// Overlap-safe atomic helper for [`LibraryDb::delete_user_library_fulltext_for_paths`].
    pub(crate) fn prune_unreferenced_user_fulltext_atomic(&self, paths: &[String]) -> Result<usize> {
        if paths.is_empty() {
            return Ok(0);
        }
        let tx = ImmediateTx::begin(&self.conn)?;
        let active = collect_active_source_files(tx.conn)?;
        let removed = prune_unreferenced_user_fulltext_in_tx(tx.conn, paths, &active.paths)?;
        tx.commit()?;
        Ok(removed)
    }
}

fn resolve_live_canonical_key(conn: &Connection, key: &str) -> Result<String> {
    let aliased: Option<String> = conn
        .query_row(
            "SELECT paper_key FROM alias WHERE identifier = ?1",
            params![key],
            |r| r.get(0),
        )
        .optional()?;
    Ok(aliased.unwrap_or_else(|| key.to_string()))
}

fn validate_sync_batch(batch: &SourceSyncBatch) -> Result<()> {
    validate_non_empty("source_id", &batch.expected_source.id)?;
    validate_non_empty("state", &batch.next_state)?;
    for row in &batch.upsert_source_items {
        validate_non_empty("source_id", &row.source_id)?;
        validate_non_empty("external_id", &row.external_id)?;
        if row.source_id != batch.expected_source.id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "source_item `{}` belongs to `{}` instead of `{}`",
                    row.external_id, row.source_id, batch.expected_source.id
                ),
            )
            .into());
        }
    }
    for upd in &batch.paper_updates {
        validate_non_empty("paper_key", &upd.paper_key)?;
        serde_json::from_str::<Value>(&upd.ids_json)?;
        serde_json::from_str::<Value>(&upd.authors_json)?;
    }
    for (ident, key) in &batch.aliases {
        validate_non_empty("identifier", ident)?;
        validate_non_empty("paper_key", key)?;
    }
    for ft in &batch.upsert_user_fulltext {
        validate_user_fulltext_row(ft)?;
    }
    Ok(())
}

fn validate_user_fulltext_row(row: &LibraryFulltextRow) -> Result<()> {
    validate_non_empty("paper_key", &row.paper_key)?;
    validate_non_empty("format", &row.format)?;
    if row.origin != ORIGIN_USER_LIBRARY {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "source sync fulltext row must have origin `{ORIGIN_USER_LIBRARY}`, got `{}`",
                row.origin
            ),
        )
        .into());
    }
    if !row.raw_blob.is_empty() && !is_sha256_hex(&row.raw_blob) {
        return Err(Error::BadHash(row.raw_blob.clone()));
    }
    if let Some(sha) = &row.file_sha256 {
        if !is_sha256_hex(sha) {
            return Err(Error::BadHash(sha.clone()));
        }
    }
    i64::try_from(row.bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "bytes overflow"))?;
    row.file_size
        .map(i64::try_from)
        .transpose()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "file_size overflow"))?;
    Ok(())
}

fn query_source_row(conn: &Connection, source_id: &str) -> Result<Option<LibrarySourceRow>> {
    let row = conn
        .query_row(
            "SELECT id, kind, root, settings_json, added_ms, last_sync_ms, state
             FROM source WHERE id = ?1",
            params![source_id],
            |r| {
                Ok(LibrarySourceRow {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    root: r.get(2)?,
                    settings_json: r.get(3)?,
                    added_ms: r.get(4)?,
                    last_sync_ms: r.get(5)?,
                    state: r.get(6)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

fn upsert_source_item_in_tx(
    conn: &Connection,
    row: &crate::library_rows::LibrarySourceItemRow,
) -> Result<()> {
    conn.execute(
        "INSERT INTO source_item (
             source_id, external_id, paper_key, citekey, ids_json, version,
             title, authors_json, year, item_kind, collections_json, tags_json,
             files_json, state, updated_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
         ON CONFLICT(source_id, external_id) DO UPDATE SET
             paper_key = excluded.paper_key,
             citekey = excluded.citekey,
             ids_json = excluded.ids_json,
             version = excluded.version,
             title = excluded.title,
             authors_json = excluded.authors_json,
             year = excluded.year,
             item_kind = excluded.item_kind,
             collections_json = excluded.collections_json,
             tags_json = excluded.tags_json,
             files_json = excluded.files_json,
             state = excluded.state,
             updated_ms = excluded.updated_ms",
        params![
            row.source_id,
            row.external_id,
            row.paper_key,
            row.citekey,
            row.ids_json,
            row.version,
            row.title,
            row.authors_json,
            row.year,
            row.item_kind,
            row.collections_json,
            row.tags_json,
            row.files_json,
            row.state,
            row.updated_ms,
        ],
    )?;
    Ok(())
}

fn apply_paper_update_in_tx(conn: &Connection, upd: &SourceSyncPaperUpdate) -> Result<bool> {
    let existing = conn
        .query_row(
            "SELECT paper_key, ids_json, title, authors_json, year,
                    abstract, bibtex, flags_json, first_seen_ms, last_seen_ms
             FROM paper WHERE paper_key = ?1",
            params![upd.paper_key],
            row_to_paper,
        )
        .optional()?;

    let merged = match (upd.mode, existing) {
        (PaperUpdateMode::InsertIfMissing, Some(_)) => return Ok(false),
        (PaperUpdateMode::MergeIdsIfExists, None) => return Ok(false),
        (PaperUpdateMode::MergeIdsIfExists, Some(mut ex)) => {
            ex.ids_json = merge_ids_json(&ex.ids_json, &upd.ids_json)?;
            ex.flags_json = merge_flags_json(&ex.flags_json, upd.venue.as_deref())?;
            ex.last_seen_ms = ex.last_seen_ms.max(upd.seen_ms);
            ex
        }
        (PaperUpdateMode::UpsertMerge | PaperUpdateMode::InsertIfMissing, existing_opt) => {
            let ids_json = match &existing_opt {
                Some(ex) => merge_ids_json(&ex.ids_json, &upd.ids_json)?,
                None => upd.ids_json.clone(),
            };
            let flags_json = merge_flags_json(
                existing_opt.as_ref().map_or("{}", |p| p.flags_json.as_str()),
                upd.venue.as_deref(),
            )?;
            let title = if !upd.title.trim().is_empty() {
                upd.title.clone()
            } else {
                existing_opt.as_ref().map(|p| p.title.clone()).unwrap_or_default()
            };
            let incoming_authors_non_empty = serde_json::from_str::<Vec<String>>(&upd.authors_json)
                .map(|v| !v.is_empty())
                .unwrap_or(false);
            let authors_json = if incoming_authors_non_empty {
                upd.authors_json.clone()
            } else {
                existing_opt
                    .as_ref()
                    .map(|p| p.authors_json.clone())
                    .unwrap_or_else(|| "[]".to_string())
            };
            let year = upd.year.or_else(|| existing_opt.as_ref().and_then(|p| p.year));
            let abstract_text = if !upd.abstract_text.trim().is_empty() {
                upd.abstract_text.clone()
            } else {
                existing_opt
                    .as_ref()
                    .map(|p| p.abstract_text.clone())
                    .unwrap_or_default()
            };
            let bibtex = existing_opt
                .as_ref()
                .map(|p| p.bibtex.clone())
                .unwrap_or_default();
            let first_seen_ms = existing_opt
                .as_ref()
                .map(|p| p.first_seen_ms.min(upd.seen_ms))
                .unwrap_or(upd.seen_ms);
            let last_seen_ms = existing_opt
                .as_ref()
                .map(|p| p.last_seen_ms.max(upd.seen_ms))
                .unwrap_or(upd.seen_ms);
            crate::library_rows::LibraryPaperRow {
                paper_key: upd.paper_key.clone(),
                ids_json,
                title,
                authors_json,
                year,
                abstract_text,
                bibtex,
                flags_json,
                first_seen_ms,
                last_seen_ms,
            }
        }
    };

    conn.execute(
        "INSERT INTO paper (
             paper_key, ids_json, title, authors_json, year,
             abstract, bibtex, flags_json, first_seen_ms, last_seen_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(paper_key) DO UPDATE SET
             ids_json = excluded.ids_json,
             title = excluded.title,
             authors_json = excluded.authors_json,
             year = excluded.year,
             abstract = excluded.abstract,
             bibtex = excluded.bibtex,
             flags_json = excluded.flags_json,
             first_seen_ms = MIN(paper.first_seen_ms, excluded.first_seen_ms),
             last_seen_ms = MAX(paper.last_seen_ms, excluded.last_seen_ms)",
        params![
            merged.paper_key,
            merged.ids_json,
            merged.title,
            merged.authors_json,
            merged.year,
            merged.abstract_text,
            merged.bibtex,
            merged.flags_json,
            merged.first_seen_ms,
            merged.last_seen_ms,
        ],
    )?;
    Ok(true)
}

fn merge_ids_json(existing_json: &str, incoming_json: &str) -> Result<String> {
    let mut base: Map<String, Value> = serde_json::from_str(existing_json).unwrap_or_default();
    let incoming: Map<String, Value> = serde_json::from_str(incoming_json)?;
    for (k, v) in incoming {
        let Some(new_str) = v.as_str().map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        let cur_str = base
            .get(&k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let replace = match cur_str {
            None => true,
            Some(cur) if k == "doi" => {
                cur.to_ascii_lowercase().starts_with("10.48550/arxiv.")
                    && !new_str.to_ascii_lowercase().starts_with("10.48550/arxiv.")
            }
            Some(_) => false,
        };
        if replace {
            base.insert(k, Value::String(new_str.to_string()));
        }
    }
    Ok(serde_json::to_string(&base)?)
}

fn merge_flags_json(existing_json: &str, venue: Option<&str>) -> Result<String> {
    let mut map: Map<String, Value> = serde_json::from_str(existing_json).unwrap_or_default();
    map.insert("owned".to_string(), Value::Bool(true));
    if let Some(v) = venue.map(str::trim).filter(|s| !s.is_empty()) {
        let has_venue = map
            .get("venue")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.trim().is_empty());
        if !has_venue {
            map.insert("venue".to_string(), Value::String(v.to_string()));
        }
    }
    Ok(serde_json::to_string(&map)?)
}

#[cfg(test)]
#[path = "library_db_sync_tests.rs"]
mod tests;
