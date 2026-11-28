//! Typed global paper library API across all modes (package F10).
//!
//! Backed by `<data_dir>/library/library.db` via [`ledger::LibraryDb`] (keeping `rusqlite`
//! out of `research-tools`), extending F3's `fulltext` table in place. The library is a
//! rebuildable index over ledger events and content-addressed blobs — **never evidence**.
//!
//! Non-rebuildable user marks (`user_mark`) live in `<data_dir>/library/user_marks.json`
//! (with a rolling previous-generation backup `<data_dir>/library/user_marks.json.bak` and
//! portable checksummed JSON backup/restore) and are mirrored into `library.db` for fast
//! queries (`00-overview.md` section 8.2, decision 5).

use crate::ingest::{ingest_bytes, IngestRequest, IngestedSource};
use crate::library_marks::{
    build_backup, export_backup_bytes, load_sidecar, now_ms, parse_and_verify_backup, save_sidecar,
    validate_and_normalize_mark,
};
use crate::library_rebuild::rebuild_from_ledger;
pub use crate::library_schema::*;
use crate::paper_key::PaperKey;
use crate::sandbox::WorkspaceSandbox;
use ledger::library_db::{LibraryDb, LibrarySearchParams, SourceSyncBatch};
use ledger::Ledger;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Handle to the global per-data-dir paper library.
pub struct Library {
    db: LibraryDb,
}

impl Library {
    /// Open or create `<data_dir>/library/library.db`, applying any forward-only migration
    /// (including upgrading an F3 `FulltextCache` database in place).
    ///
    /// If `library.db` has no `user_mark` rows yet and a valid `user_marks.json` (or `.bak`)
    /// sidecar exists, marks are restored into `library.db` automatically.
    pub fn open(data_dir: &Path) -> Result<Self, String> {
        let mut db = LibraryDb::open(data_dir)
            .map_err(|e| format!("failed to open library database: {e}"))?;
        let existing_marks = db
            .list_user_marks()
            .map_err(|e| format!("failed to inspect user marks: {e}"))?;
        if existing_marks.is_empty() {
            if let Ok(Some(sidecar_marks)) = load_sidecar(db.library_dir()) {
                if !sidecar_marks.is_empty() {
                    let rows = sidecar_marks
                        .iter()
                        .map(|m| m.to_row())
                        .collect::<Result<Vec<_>, _>>()?;
                    db.replace_all_user_marks(&rows)
                        .map_err(|e| format!("failed to hydrate user marks from sidecar: {e}"))?;
                }
            }
        }
        Ok(Self { db })
    }

    pub fn schema_version(&self) -> Result<u32, String> {
        self.db.schema_version().map_err(|e| e.to_string())
    }

    pub fn db_path(&self) -> PathBuf {
        self.db.db_path()
    }

    pub fn user_marks_path(&self) -> PathBuf {
        self.db.library_dir().join(USER_MARKS_FILE_NAME)
    }

    pub fn user_marks_backup_path(&self) -> PathBuf {
        self.db.library_dir().join(USER_MARKS_BACKUP_NAME)
    }

    /// Resolve `key` to its canonical `paper.paper_key` (directly or through the `alias` table).
    pub fn resolve_key(&self, key: &PaperKey) -> Result<Option<PaperKey>, String> {
        let raw = key.as_str();
        if self
            .db
            .get_paper(raw)
            .map_err(|e| e.to_string())?
            .is_some()
        {
            return Ok(Some(key.clone()));
        }
        if let Some(target) = self.db.resolve_alias(raw).map_err(|e| e.to_string())? {
            return Ok(Some(PaperKey::parse(&target)?));
        }
        Ok(None)
    }

    /// Upsert a paper record, register all canonical identifier aliases, and refresh `paper_fts`.
    pub fn upsert_paper(&self, paper: &PaperRecord) -> Result<(), String> {
        let row = paper.to_row()?;
        self.db.upsert_paper(&row).map_err(|e| e.to_string())?;
        for alias in paper.ids.canonical_aliases() {
            if alias != paper.key.as_str() {
                self.db
                    .put_alias(&alias, paper.key.as_str())
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    /// Register an explicit alias mapping `identifier -> canonical_key`.
    pub fn put_alias(&self, identifier: &PaperKey, canonical_key: &PaperKey) -> Result<(), String> {
        self.db
            .put_alias(identifier.as_str(), canonical_key.as_str())
            .map_err(|e| e.to_string())
    }

    /// Fetch the complete library record for `key` (resolving aliases automatically).
    pub fn get_paper(&self, key: &PaperKey) -> Result<Option<PaperDetail>, String> {
        let Some(canonical) = self.resolve_key(key)? else {
            return Ok(None);
        };
        let raw = canonical.as_str();
        let Some(paper_row) = self.db.get_paper(raw).map_err(|e| e.to_string())? else {
            return Ok(None);
        };
        let paper = PaperRecord::from_row(paper_row)?;
        let aliases = self
            .db
            .list_aliases_for_paper(raw)
            .map_err(|e| e.to_string())?;
        let fulltext = self
            .db
            .list_fulltext_for_paper(raw)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(FulltextRecord::from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let seen = self
            .db
            .list_seen_for_paper(raw)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(SeenRecord::from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let skeletons = self
            .db
            .list_skeletons_for_paper(raw)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(SkeletonRecord::from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let findings = self
            .db
            .list_findings_for_paper(raw)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(FindingRecord::from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let paper_text = self
            .db
            .list_paper_text_for_paper(raw)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(PaperTextRecord::from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let source_items = self
            .db
            .list_source_items_for_paper(raw)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(SourceItemRecord::from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let user_mark = self
            .db
            .get_user_mark(raw)
            .map_err(|e| e.to_string())?
            .map(UserMark::from_row)
            .transpose()?;
        Ok(Some(PaperDetail {
            paper,
            aliases,
            fulltext,
            seen,
            skeletons,
            findings,
            paper_text,
            source_items,
            user_mark,
        }))
    }

    /// Read-only paginated list of papers ordered by most recently seen.
    pub fn list(&self, limit: usize, offset: usize) -> Result<Vec<PaperSummary>, String> {
        self.search(&LibraryQuery {
            limit: Some(limit),
            offset,
            ..Default::default()
        })
    }

    /// Read-only search by free-text FTS5 `q`, `title`, `author`, `year`, `key` (including
    /// aliases), `status`, and `role`.
    pub fn search(&self, query: &LibraryQuery) -> Result<Vec<PaperSummary>, String> {
        let params = LibrarySearchParams {
            q: query.q.clone(),
            title: query.title.clone(),
            author: query.author.clone(),
            year: query.year,
            paper_key: query.key.as_ref().map(ToString::to_string),
            status: query.status.map(|s| s.as_str().to_string()),
            role: query.role.map(|r| r.as_str().to_string()),
            limit: query
                .limit
                .unwrap_or(DEFAULT_LIBRARY_LIMIT)
                .clamp(1, MAX_LIBRARY_LIMIT),
            offset: query.offset,
        };
        self.db
            .search_papers(&params)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(PaperSummary::from_search_row)
            .collect()
    }

    /// Most recently seen papers for a given [`SeenRole`] (e.g. Investigate's "recent papers").
    pub fn recent_by_role(&self, role: SeenRole, limit: usize) -> Result<Vec<PaperSummary>, String> {
        self.db
            .recent_papers_by_role(role.as_str(), limit.clamp(1, MAX_LIBRARY_LIMIT))
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(PaperSummary::from_search_row)
            .collect()
    }

    pub fn record_seen(&self, record: &SeenRecord) -> Result<(), String> {
        self.db
            .record_seen(&record.to_row())
            .map_err(|e| e.to_string())
    }

    pub fn upsert_fulltext(&self, record: &FulltextRecord) -> Result<(), String> {
        self.db
            .upsert_fulltext(&record.to_row())
            .map_err(|e| e.to_string())
    }

    /// Read SHA-256-verified full-text bytes for `key` (resolving aliases) from the ledger or
    /// library blob store without making any network call.
    pub fn read_fulltext_bytes(
        &self,
        key: &PaperKey,
        version: Option<&str>,
    ) -> Result<Option<(FulltextRecord, Vec<u8>)>, String> {
        let lookup_key = self
            .resolve_key(key)?
            .unwrap_or_else(|| key.clone());
        let rows = self
            .db
            .list_fulltext_for_paper(lookup_key.as_str())
            .map_err(|e| e.to_string())?;
        let wanted_ver = version.map(str::trim).filter(|v| !v.is_empty());
        for row in rows {
            if let Some(v) = wanted_ver {
                if row.version.as_deref() != Some(v) {
                    continue;
                }
            }
            let hash = if !row.raw_blob.is_empty() {
                row.raw_blob.as_str()
            } else if let Some(sha) = row.file_sha256.as_deref() {
                sha
            } else {
                continue;
            };
            if let Some(bytes) = self
                .db
                .read_verified_blob_any(hash)
                .map_err(|e| e.to_string())?
            {
                let rec = FulltextRecord::from_row(row)?;
                return Ok(Some((rec, bytes)));
            }
        }
        Ok(None)
    }

    pub fn upsert_skeleton(&self, record: &SkeletonRecord) -> Result<(), String> {
        self.db
            .upsert_skeleton(&record.to_row())
            .map_err(|e| e.to_string())
    }

    pub fn get_skeleton(
        &self,
        key: &PaperKey,
        version: Option<&str>,
        builder: &str,
    ) -> Result<Option<SkeletonRecord>, String> {
        let lookup_key = self.resolve_key(key)?.unwrap_or_else(|| key.clone());
        self.db
            .get_skeleton(lookup_key.as_str(), version, builder)
            .map_err(|e| e.to_string())?
            .map(SkeletonRecord::from_row)
            .transpose()
    }

    pub fn upsert_finding(&self, record: &FindingRecord) -> Result<(), String> {
        self.db
            .upsert_finding(&record.to_row())
            .map_err(|e| e.to_string())
    }

    /// Store extracted text (optional raw UTF-8 `text_bytes` stored in the cache blob dir)
    /// and index it in `paper_text` and `paper_fts` (F14 readiness).
    pub fn upsert_paper_text(
        &self,
        record: &PaperTextRecord,
        text_bytes: Option<&[u8]>,
    ) -> Result<(), String> {
        if let Some(bytes) = text_bytes {
            let hash = self.db.put_cache_blob(bytes).map_err(|e| e.to_string())?;
            if hash != record.text_blob {
                return Err(format!(
                    "paper_text text_blob `{}` does not match provided bytes sha256 `{hash}`",
                    record.text_blob
                ));
            }
        }
        let row = record.to_row()?;
        self.db.upsert_paper_text(&row).map_err(|e| e.to_string())
    }

    pub fn upsert_source(&self, record: &SourceRecord) -> Result<(), String> {
        let row = record.to_row()?;
        self.db.upsert_source(&row).map_err(|e| e.to_string())
    }

    pub fn list_sources(&self) -> Result<Vec<SourceRecord>, String> {
        self.db
            .list_sources()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(SourceRecord::from_row)
            .collect()
    }

    pub fn delete_source(&self, source_id: &str) -> Result<bool, String> {
        self.db.delete_source(source_id).map_err(|e| e.to_string())
    }

    pub fn upsert_source_item(&self, record: &SourceItemRecord) -> Result<(), String> {
        let row = record.to_row()?;
        self.db.upsert_source_item(&row).map_err(|e| e.to_string())
    }

    pub fn list_source_items_for_source(&self, source_id: &str) -> Result<Vec<SourceItemRecord>, String> {
        self.db
            .list_source_items_for_source(source_id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(SourceItemRecord::from_row)
            .collect()
    }

    pub fn list_all_source_items(&self) -> Result<Vec<SourceItemRecord>, String> {
        self.db
            .list_all_source_items()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(SourceItemRecord::from_row)
            .collect()
    }

    pub fn delete_source_item(&self, source_id: &str, external_id: &str) -> Result<bool, String> {
        self.db
            .delete_source_item(source_id, external_id)
            .map_err(|e| e.to_string())
    }

    pub fn delete_user_library_fulltext_for_paths(&self, paths: &[String]) -> Result<usize, String> {
        self.db
            .delete_user_library_fulltext_for_paths(paths)
            .map_err(|e| e.to_string())
    }

    pub fn register_source_atomic(
        &self,
        record: &SourceRecord,
    ) -> Result<(SourceRecord, bool), String> {
        let row = record.to_row()?;
        let (stored, created) = self
            .db
            .register_source_atomic(&row)
            .map_err(|e| e.to_string())?;
        Ok((SourceRecord::from_row(stored)?, created))
    }

    pub fn apply_source_sync_batch(&self, batch: &SourceSyncBatch) -> Result<i64, String> {
        self.db
            .apply_source_sync_batch(batch)
            .map_err(|e| e.to_string())
    }

    pub fn unregister_source(&self, source_id: &str) -> Result<bool, String> {
        self.db
            .unregister_source_atomic(source_id)
            .map_err(|e| e.to_string())
    }

    pub fn upsert_user_library_fulltext(&self, record: &FulltextRecord) -> Result<(), String> {
        self.db
            .upsert_user_library_fulltext_preserve_other_origins(&record.to_row())
            .map_err(|e| e.to_string())
    }

    pub fn has_seen(&self, key: &PaperKey) -> Result<bool, String> {
        let canonical = self.resolve_key(key)?.unwrap_or_else(|| key.clone());
        let rows = self
            .db
            .list_seen_for_paper(canonical.as_str())
            .map_err(|e| e.to_string())?;
        Ok(!rows.is_empty())
    }

    pub fn upsert_search_cache(&self, record: &SearchCacheRecord) -> Result<(), String> {
        self.db
            .upsert_search_cache(&ledger::library_db::LibrarySearchCacheRow {
                service: record.service.clone(),
                url_sha256: record.url_sha256.clone(),
                raw_blob: record.raw_blob.clone(),
                fetched_ms: record.fetched_ms,
            })
            .map_err(|e| e.to_string())
    }

    pub fn get_search_cache(
        &self,
        service: &str,
        url_sha256: &str,
    ) -> Result<Option<SearchCacheRecord>, String> {
        self.db
            .get_search_cache(service, url_sha256)
            .map_err(|e| e.to_string())
            .map(|opt| opt.map(SearchCacheRecord::from))
    }

    pub fn prune_search_cache(&self, older_than_ms: i64) -> Result<usize, String> {
        self.db
            .prune_search_cache(older_than_ms)
            .map_err(|e| e.to_string())
    }

    /// Write a user mark to `library.db` and atomically mirror all marks to
    /// `<data_dir>/library/user_marks.json` (rotating the prior valid copy to `.bak`).
    pub fn set_user_mark(
        &self,
        key: &PaperKey,
        status: MarkStatus,
        tags: Vec<String>,
        note: String,
    ) -> Result<UserMark, String> {
        let canonical = self.resolve_key(key)?.unwrap_or_else(|| key.clone());
        let mark = validate_and_normalize_mark(canonical, status, tags, note, now_ms())?;
        let row = mark.to_row()?;
        self.db.upsert_user_mark(&row).map_err(|e| e.to_string())?;
        let all = self.list_user_marks()?;
        save_sidecar(self.db.library_dir(), &all)?;
        Ok(mark)
    }

    pub fn get_user_mark(&self, key: &PaperKey) -> Result<Option<UserMark>, String> {
        let canonical = self.resolve_key(key)?.unwrap_or_else(|| key.clone());
        self.db
            .get_user_mark(canonical.as_str())
            .map_err(|e| e.to_string())?
            .map(UserMark::from_row)
            .transpose()
    }

    pub fn delete_user_mark(&self, key: &PaperKey) -> Result<bool, String> {
        let canonical = self.resolve_key(key)?.unwrap_or_else(|| key.clone());
        let removed = self
            .db
            .delete_user_mark(canonical.as_str())
            .map_err(|e| e.to_string())?;
        if removed {
            let all = self.list_user_marks()?;
            save_sidecar(self.db.library_dir(), &all)?;
        }
        Ok(removed)
    }

    pub fn list_user_marks(&self) -> Result<Vec<UserMark>, String> {
        self.db
            .list_user_marks()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(UserMark::from_row)
            .collect()
    }

    /// Export all non-rebuildable user marks as a portable, SHA-256-checksummed JSON backup.
    pub fn export_user_marks_backup(&self) -> Result<UserMarksBackup, String> {
        let marks = self.list_user_marks()?;
        build_backup(&marks)
    }

    /// Export all non-rebuildable user marks as UTF-8 JSON bytes ready to include in a backup.
    pub fn export_user_marks_backup_bytes(&self) -> Result<Vec<u8>, String> {
        let marks = self.list_user_marks()?;
        export_backup_bytes(&marks)
    }

    /// Verify and restore user marks from a portable backup created by
    /// [`Self::export_user_marks_backup_bytes`], updating both `user_marks.json` and `library.db`.
    pub fn restore_user_marks_backup(&mut self, bytes: &[u8]) -> Result<usize, String> {
        let env = parse_and_verify_backup(bytes)?;
        let rows = env
            .marks
            .iter()
            .map(|m| m.to_row())
            .collect::<Result<Vec<_>, _>>()?;
        save_sidecar(self.db.library_dir(), &env.marks)?;
        self.db
            .replace_all_user_marks(&rows)
            .map_err(|e| e.to_string())?;
        Ok(rows.len())
    }

    /// Remove a paper from rebuildable index tables. Never touches `ledger.db`, `<data_dir>/blobs/`,
    /// or `user_mark`.
    pub fn delete_paper(&mut self, key: &PaperKey) -> Result<bool, String> {
        let canonical = self.resolve_key(key)?.unwrap_or_else(|| key.clone());
        self.db
            .delete_paper_index_rows(canonical.as_str())
            .map_err(|e| e.to_string())
    }

    /// Re-ingest a cached library paper into `session_id` through [`ingest_bytes`], writing a
    /// fresh `source.ingested` event so the session pins its own copy (`explore.md` 4.3, rule S5).
    pub fn attach_to_session(
        &self,
        sandbox: &WorkspaceSandbox,
        ledger: &Mutex<Ledger>,
        session_id: &str,
        key: &PaperKey,
        version: Option<&str>,
        section_filter: Option<&str>,
    ) -> Result<IngestedSource, String> {
        let canonical = self.resolve_key(key)?.unwrap_or_else(|| key.clone());
        let Some((entry, raw_bytes)) = self.read_fulltext_bytes(&canonical, version)? else {
            return Err(format!(
                "no cached full-text blob found in library for `{canonical}`"
            ));
        };
        let title = self
            .db
            .get_paper(canonical.as_str())
            .map_err(|e| e.to_string())?
            .map(|p| p.title)
            .filter(|t| !t.trim().is_empty());
        let ext = match entry.format.as_str() {
            "pdf" => "pdf",
            "tex" | "latex" => "tex",
            "tar_gz" | "tar.gz" | "eprint" => "tar.gz",
            "zip" => "zip",
            _ => "txt",
        };
        let slug: String = canonical
            .as_str()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let file_name = format!("{slug}.{ext}");
        let origin = format!("library:{canonical}");
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
                title: title.as_deref(),
            },
        )
    }

    /// Rebuild all rebuildable tables in `library.db` from `<data_dir>/ledger.db` and
    /// `<data_dir>/blobs/`, then restore `user_mark` from `user_marks.json` (or `.bak`).
    pub fn rebuild(&mut self) -> Result<RebuildReport, String> {
        rebuild_from_ledger(&mut self.db)
    }
}
