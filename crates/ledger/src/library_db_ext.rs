//! F13 (`source`, `source_item`), F14 (`paper_text`), F11 (`search_cache`), and
//! transactional rebuild batch helpers for [`LibraryDb`].

use crate::library_db::{validate_non_empty, LibraryDb};
use crate::library_rows::{
    row_to_source_item, LibraryPaperTextRow, LibrarySearchCacheRow, LibrarySourceItemRow,
    LibrarySourceRow, RebuildBatch,
};
use crate::library_schema::ORIGIN_USER_LIBRARY;
use crate::{is_sha256_hex, Error, Result};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use std::io;

impl LibraryDb {
    pub fn upsert_source(&self, row: &LibrarySourceRow) -> Result<()> {
        validate_non_empty("id", &row.id)?;
        validate_non_empty("root", &row.root)?;
        self.conn.execute(
            "INSERT INTO source (id, kind, root, settings_json, added_ms, last_sync_ms, state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
                 kind = excluded.kind,
                 root = excluded.root,
                 settings_json = excluded.settings_json,
                 last_sync_ms = excluded.last_sync_ms,
                 state = excluded.state",
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
        Ok(())
    }

    pub fn list_sources(&self) -> Result<Vec<LibrarySourceRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, root, settings_json, added_ms, last_sync_ms, state
             FROM source ORDER BY added_ms ASC, id ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(LibrarySourceRow {
                id: r.get(0)?,
                kind: r.get(1)?,
                root: r.get(2)?,
                settings_json: r.get(3)?,
                added_ms: r.get(4)?,
                last_sync_ms: r.get(5)?,
                state: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn delete_source(&self, source_id: &str) -> Result<bool> {
        let n = self
            .conn
            .execute("DELETE FROM source WHERE id = ?1", params![source_id])?;
        Ok(n > 0)
    }

    pub fn upsert_source_item(&self, row: &LibrarySourceItemRow) -> Result<()> {
        validate_non_empty("source_id", &row.source_id)?;
        validate_non_empty("external_id", &row.external_id)?;
        self.conn.execute(
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

    pub fn list_source_items_for_paper(&self, paper_key: &str) -> Result<Vec<LibrarySourceItemRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT source_id, external_id, paper_key, citekey, ids_json, version,
                    title, authors_json, year, item_kind, collections_json, tags_json,
                    files_json, state, updated_ms
             FROM source_item WHERE paper_key = ?1
             ORDER BY source_id ASC, external_id ASC",
        )?;
        let rows = stmt.query_map(params![paper_key], row_to_source_item)?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn list_source_items_for_source(&self, source_id: &str) -> Result<Vec<LibrarySourceItemRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT source_id, external_id, paper_key, citekey, ids_json, version,
                    title, authors_json, year, item_kind, collections_json, tags_json,
                    files_json, state, updated_ms
             FROM source_item WHERE source_id = ?1
             ORDER BY external_id ASC",
        )?;
        let rows = stmt.query_map(params![source_id], row_to_source_item)?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn list_all_source_items(&self) -> Result<Vec<LibrarySourceItemRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT source_id, external_id, paper_key, citekey, ids_json, version,
                    title, authors_json, year, item_kind, collections_json, tags_json,
                    files_json, state, updated_ms
             FROM source_item
             ORDER BY source_id ASC, external_id ASC",
        )?;
        let rows = stmt.query_map([], row_to_source_item)?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn delete_source_item(&self, source_id: &str, external_id: &str) -> Result<bool> {
        let n = self.conn.execute(
            "DELETE FROM source_item WHERE source_id = ?1 AND external_id = ?2",
            params![source_id, external_id],
        )?;
        Ok(n > 0)
    }

    pub fn delete_user_library_fulltext_for_paths(&self, paths: &[String]) -> Result<usize> {
        self.prune_unreferenced_user_fulltext_atomic(paths)
    }

    pub fn upsert_paper_text(&self, row: &LibraryPaperTextRow) -> Result<()> {
        validate_non_empty("paper_key", &row.paper_key)?;
        validate_non_empty("extractor", &row.extractor)?;
        if !is_sha256_hex(&row.file_sha256) {
            return Err(Error::BadHash(row.file_sha256.clone()));
        }
        if !is_sha256_hex(&row.text_blob) {
            return Err(Error::BadHash(row.text_blob.clone()));
        }
        let ver = row.version.as_deref().map(str::trim).unwrap_or("");
        self.conn.execute(
            "INSERT INTO paper_text (
                 paper_key, version, file_sha256, extractor, text_blob, page_basis_json, extracted_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(paper_key, version, file_sha256, extractor) DO UPDATE SET
                 text_blob = excluded.text_blob,
                 page_basis_json = excluded.page_basis_json,
                 extracted_ms = excluded.extracted_ms",
            params![
                row.paper_key,
                ver,
                row.file_sha256,
                row.extractor,
                row.text_blob,
                row.page_basis_json,
                row.extracted_ms,
            ],
        )?;
        self.refresh_fts_for_paper(&row.paper_key)
    }

    pub fn list_paper_text_for_paper(&self, paper_key: &str) -> Result<Vec<LibraryPaperTextRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT paper_key, version, file_sha256, extractor, text_blob, page_basis_json, extracted_ms
             FROM paper_text WHERE paper_key = ?1
             ORDER BY extracted_ms DESC, version DESC, extractor ASC",
        )?;
        let rows = stmt.query_map(params![paper_key], |r| {
            let ver: String = r.get(1)?;
            Ok(LibraryPaperTextRow {
                paper_key: r.get(0)?,
                version: (!ver.is_empty()).then_some(ver),
                file_sha256: r.get(2)?,
                extractor: r.get(3)?,
                text_blob: r.get(4)?,
                page_basis_json: r.get(5)?,
                extracted_ms: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn upsert_search_cache(&self, row: &LibrarySearchCacheRow) -> Result<()> {
        validate_non_empty("service", &row.service)?;
        if !is_sha256_hex(&row.url_sha256) {
            return Err(Error::BadHash(row.url_sha256.clone()));
        }
        if !is_sha256_hex(&row.raw_blob) {
            return Err(Error::BadHash(row.raw_blob.clone()));
        }
        self.conn.execute(
            "INSERT INTO search_cache (service, url_sha256, raw_blob, fetched_ms)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(service, url_sha256) DO UPDATE SET
                 raw_blob = excluded.raw_blob,
                 fetched_ms = excluded.fetched_ms",
            params![row.service, row.url_sha256, row.raw_blob, row.fetched_ms],
        )?;
        Ok(())
    }

    pub fn get_search_cache(
        &self,
        service: &str,
        url_sha256: &str,
    ) -> Result<Option<LibrarySearchCacheRow>> {
        let row = self
            .conn
            .query_row(
                "SELECT service, url_sha256, raw_blob, fetched_ms
                 FROM search_cache WHERE service = ?1 AND url_sha256 = ?2",
                params![service, url_sha256],
                |r| {
                    Ok(LibrarySearchCacheRow {
                        service: r.get(0)?,
                        url_sha256: r.get(1)?,
                        raw_blob: r.get(2)?,
                        fetched_ms: r.get(3)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    pub fn prune_search_cache(&self, older_than_ms: i64) -> Result<usize> {
        let n = self.conn.execute(
            "DELETE FROM search_cache WHERE fetched_ms < ?1",
            params![older_than_ms],
        )?;
        Ok(n)
    }

    /// Replace all rebuildable ledger-derived tables inside one transaction, leaving
    /// `user_mark`, `source`, `source_item`, `search_cache`, and `user_library` fulltext rows intact.
    pub fn apply_rebuild_batch(&mut self, batch: &RebuildBatch) -> Result<()> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM paper_fts", [])?;
        tx.execute("DELETE FROM finding", [])?;
        tx.execute("DELETE FROM skeleton", [])?;
        tx.execute("DELETE FROM seen", [])?;
        tx.execute("DELETE FROM paper_text", [])?;
        tx.execute("DELETE FROM alias", [])?;
        tx.execute(
            "DELETE FROM fulltext WHERE origin <> ?1",
            params![ORIGIN_USER_LIBRARY],
        )?;
        tx.execute("DELETE FROM paper", [])?;

        for row in &batch.papers {
            tx.execute(
                "INSERT INTO paper (
                     paper_key, ids_json, title, authors_json, year,
                     abstract, bibtex, flags_json, first_seen_ms, last_seen_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    row.paper_key,
                    row.ids_json,
                    row.title,
                    row.authors_json,
                    row.year,
                    row.abstract_text,
                    row.bibtex,
                    row.flags_json,
                    row.first_seen_ms,
                    row.last_seen_ms,
                ],
            )?;
        }
        for (ident, key) in &batch.aliases {
            tx.execute(
                "INSERT OR REPLACE INTO alias (identifier, paper_key) VALUES (?1, ?2)",
                params![ident, key],
            )?;
        }
        for row in &batch.fulltext {
            let ver = row.version.as_deref().map(str::trim).unwrap_or("");
            let bytes_i64 = i64::try_from(row.bytes)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "bytes overflow"))?;
            let file_size_i64 = row
                .file_size
                .map(i64::try_from)
                .transpose()
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "file_size overflow"))?;
            tx.execute(
                "INSERT OR REPLACE INTO fulltext (
                     paper_key, version, raw_blob, format, bytes, origin, fetched_ms,
                     file_path, file_size, file_mtime_ms, file_sha256
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    row.paper_key,
                    ver,
                    row.raw_blob,
                    row.format,
                    bytes_i64,
                    row.origin,
                    row.fetched_ms,
                    row.file_path,
                    file_size_i64,
                    row.file_mtime_ms,
                    row.file_sha256,
                ],
            )?;
        }
        for row in &batch.seen {
            tx.execute(
                "INSERT OR REPLACE INTO seen (paper_key, session_id, role, seen_ms)
                 VALUES (?1, ?2, ?3, ?4)",
                params![row.paper_key, row.session_id, row.role, row.seen_ms],
            )?;
        }
        for row in &batch.skeletons {
            let ver = row.version.as_deref().map(str::trim).unwrap_or("");
            tx.execute(
                "INSERT OR REPLACE INTO skeleton (paper_key, version, builder, blob_sha256, built_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![row.paper_key, ver, row.builder, row.blob_sha256, row.built_ms],
            )?;
        }
        for row in &batch.findings {
            let seq = i64::try_from(row.evidence_seq)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "evidence_seq overflow"))?;
            tx.execute(
                "INSERT OR REPLACE INTO finding (
                     paper_key, session_id, evidence_seq, role, gist, quote,
                     cited_start, cited_end, recorded_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    row.paper_key,
                    row.session_id,
                    seq,
                    row.role,
                    row.gist,
                    row.quote,
                    row.cited_start.map(|v| v as i64),
                    row.cited_end.map(|v| v as i64),
                    row.recorded_ms,
                ],
            )?;
        }
        for row in &batch.paper_text {
            let ver = row.version.as_deref().map(str::trim).unwrap_or("");
            tx.execute(
                "INSERT OR REPLACE INTO paper_text (
                     paper_key, version, file_sha256, extractor, text_blob, page_basis_json, extracted_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    row.paper_key,
                    ver,
                    row.file_sha256,
                    row.extractor,
                    row.text_blob,
                    row.page_basis_json,
                    row.extracted_ms,
                ],
            )?;
        }
        tx.commit()?;
        self.rebuild_all_fts()
    }
}
