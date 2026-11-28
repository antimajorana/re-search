//! FTS5 indexing, full-text + structured search, and role recency queries for [`LibraryDb`].

use crate::library_db::LibraryDb;
use crate::library_rows::{
    row_to_paper, row_to_user_mark, LibraryPaperRow, LibrarySearchParams, LibrarySearchRow,
};
use crate::library_schema::ORIGIN_USER_LIBRARY;
use crate::Result;
use rusqlite::{params, types::Value as SqlValue, Connection, OptionalExtension, TransactionBehavior};

const MAX_FTS_FULLTEXT_BYTES: usize = 2 * 1024 * 1024;

impl LibraryDb {
    /// Re-index one paper in `paper_fts` (title, authors, abstract, finding gists, and
    /// extracted full text where `paper_text` has a readable verified blob).
    pub fn refresh_fts_for_paper(&self, paper_key: &str) -> Result<()> {
        refresh_fts_with_conn(&self.conn, self, paper_key)
    }

    /// Rebuild `paper_fts` for every row currently in `paper`.
    pub fn rebuild_all_fts(&mut self) -> Result<()> {
        let keys: Vec<String> = {
            let mut stmt = self
                .conn
                .prepare("SELECT paper_key FROM paper ORDER BY paper_key ASC")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        let mut entries = Vec::with_capacity(keys.len());
        for key in &keys {
            if let Some(doc) = collect_fts_doc(&self.conn, self, key)? {
                entries.push(doc);
            }
        }
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM paper_fts", [])?;
        for (key, title, authors, abstract_text, gists, full_text) in entries {
            tx.execute(
                "INSERT INTO paper_fts (paper_key, title, authors, abstract, gists, full_text)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![key, title, authors, abstract_text, gists, full_text],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Search and filter papers by free-text FTS5 `q`, `title`, `author`, `year`, `paper_key`
    /// (with alias resolution), `status`, and `role`.
    pub fn search_papers(&self, p: &LibrarySearchParams) -> Result<Vec<LibrarySearchRow>> {
        let mut sql = String::from(
            "SELECT p.paper_key, p.ids_json, p.title, p.authors_json, p.year,
                    p.abstract, p.bibtex, p.flags_json, p.first_seen_ms, p.last_seen_ms
             FROM paper p
             LEFT JOIN user_mark m ON m.paper_key = p.paper_key",
        );
        let mut binds: Vec<SqlValue> = Vec::new();
        let mut clauses: Vec<String> = Vec::new();

        let fts_match = p.q.as_deref().and_then(build_safe_fts_query);
        if let Some(ref fts) = fts_match {
            sql.push_str(" JOIN paper_fts f ON f.paper_key = p.paper_key");
            binds.push(SqlValue::Text(fts.clone()));
            clauses.push(format!("paper_fts MATCH ?{}", binds.len()));
        } else if let Some(raw_q) = p.q.as_deref() {
            if !raw_q.trim().is_empty() {
                return Ok(Vec::new());
            }
        }
        if let Some(title) = p.title.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            binds.push(SqlValue::Text(title.to_lowercase()));
            clauses.push(format!("instr(lower(p.title), ?{}) > 0", binds.len()));
        }
        if let Some(author) = p.author.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            binds.push(SqlValue::Text(author.to_lowercase()));
            clauses.push(format!(
                "EXISTS (SELECT 1 FROM json_each(p.authors_json) WHERE instr(lower(value), ?{}) > 0)",
                binds.len()
            ));
        }
        if let Some(year) = p.year {
            binds.push(SqlValue::Integer(i64::from(year)));
            clauses.push(format!("p.year = ?{}", binds.len()));
        }
        if let Some(key) = p.paper_key.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            binds.push(SqlValue::Text(key.to_string()));
            let idx = binds.len();
            clauses.push(format!(
                "(p.paper_key = ?{idx} OR p.paper_key = (SELECT paper_key FROM alias WHERE identifier = ?{idx}))"
            ));
        }
        if let Some(status) = p.status.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            binds.push(SqlValue::Text(status.to_string()));
            clauses.push(format!("COALESCE(m.status, 'unread') = ?{}", binds.len()));
        }
        if let Some(role) = p.role.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            binds.push(SqlValue::Text(role.to_string()));
            clauses.push(format!(
                "EXISTS (SELECT 1 FROM seen s WHERE s.paper_key = p.paper_key AND s.role = ?{})",
                binds.len()
            ));
        }

        if !clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&clauses.join(" AND "));
        }
        if fts_match.is_some() {
            sql.push_str(" ORDER BY bm25(paper_fts) ASC, p.last_seen_ms DESC, p.paper_key ASC");
        } else {
            sql.push_str(" ORDER BY p.last_seen_ms DESC, p.paper_key ASC");
        }
        let limit = p.limit.clamp(1, 200) as i64;
        let offset = i64::try_from(p.offset).unwrap_or(i64::MAX);
        binds.push(SqlValue::Integer(limit));
        let lim_idx = binds.len();
        binds.push(SqlValue::Integer(offset));
        let off_idx = binds.len();
        sql.push_str(&format!(" LIMIT ?{lim_idx} OFFSET ?{off_idx}"));

        let papers = {
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(binds), row_to_paper)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        self.hydrate_search_rows(papers)
    }

    /// Most recently seen papers carrying `role` (used e.g. for Investigate's "recent papers").
    pub fn recent_papers_by_role(&self, role: &str, limit: usize) -> Result<Vec<LibrarySearchRow>> {
        let lim = limit.clamp(1, 200) as i64;
        let papers = {
            let mut stmt = self.conn.prepare(
                "SELECT p.paper_key, p.ids_json, p.title, p.authors_json, p.year,
                        p.abstract, p.bibtex, p.flags_json, p.first_seen_ms, p.last_seen_ms
                 FROM paper p
                 JOIN (
                     SELECT paper_key, MAX(seen_ms) AS max_seen
                     FROM seen WHERE role = ?1 GROUP BY paper_key
                 ) s ON s.paper_key = p.paper_key
                 ORDER BY s.max_seen DESC, p.paper_key ASC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![role, lim], row_to_paper)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        self.hydrate_search_rows(papers)
    }

    fn hydrate_search_rows(&self, papers: Vec<LibraryPaperRow>) -> Result<Vec<LibrarySearchRow>> {
        let mut out = Vec::with_capacity(papers.len());
        for paper in papers {
            let key = &paper.paper_key;
            let user_mark = self
                .conn
                .query_row(
                    "SELECT paper_key, status, tags_json, note, updated_ms FROM user_mark WHERE paper_key = ?1",
                    params![key],
                    row_to_user_mark,
                )
                .optional()?;
            let has_fulltext: bool = self.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM fulltext WHERE paper_key = ?1)",
                params![key],
                |r| r.get(0),
            )?;
            let owned: bool = self.conn.query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM fulltext WHERE paper_key = ?1 AND origin = ?2
                 ) OR EXISTS(
                     SELECT 1 FROM source_item WHERE paper_key = ?1 AND state <> 'missing'
                 )",
                params![key, ORIGIN_USER_LIBRARY],
                |r| r.get(0),
            )?;
            let seen_rows = self.list_seen_for_paper(key)?;
            let mut seen_roles: Vec<String> = Vec::new();
            let mut seen_sessions: Vec<String> = Vec::new();
            for s in seen_rows {
                if !seen_roles.contains(&s.role) {
                    seen_roles.push(s.role);
                }
                if !seen_sessions.contains(&s.session_id) {
                    seen_sessions.push(s.session_id);
                }
            }
            seen_roles.sort();
            seen_sessions.sort();
            let finding_count: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM finding WHERE paper_key = ?1",
                params![key],
                |r| r.get(0),
            )?;
            let skeleton_count: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM skeleton WHERE paper_key = ?1",
                params![key],
                |r| r.get(0),
            )?;
            out.push(LibrarySearchRow {
                paper,
                user_mark,
                has_fulltext,
                owned,
                seen_roles,
                seen_sessions,
                finding_count: finding_count.max(0) as usize,
                skeleton_count: skeleton_count.max(0) as usize,
            });
        }
        Ok(out)
    }
}

type FtsDoc = (String, String, String, String, String, String);

fn collect_fts_doc(conn: &Connection, db: &LibraryDb, paper_key: &str) -> Result<Option<FtsDoc>> {
    let paper = conn
        .query_row(
            "SELECT title, authors_json, abstract FROM paper WHERE paper_key = ?1",
            params![paper_key],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)),
        )
        .optional()?;
    let Some((title, authors_json, abstract_text)) = paper else {
        return Ok(None);
    };
    let authors = serde_json::from_str::<Vec<String>>(&authors_json)
        .unwrap_or_default()
        .join(" ");
    let gists = {
        let mut stmt = conn.prepare(
            "SELECT gist FROM finding WHERE paper_key = ?1 AND gist <> '' ORDER BY recorded_ms ASC, evidence_seq ASC",
        )?;
        let rows = stmt.query_map(params![paper_key], |r| r.get::<_, String>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?.join("\n")
    };
    let text_blobs = {
        let mut stmt = conn.prepare(
            "SELECT text_blob FROM paper_text WHERE paper_key = ?1 ORDER BY extracted_ms DESC",
        )?;
        let rows = stmt.query_map(params![paper_key], |r| r.get::<_, String>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    };
    let mut full_text = String::new();
    for hash in text_blobs {
        if full_text.len() >= MAX_FTS_FULLTEXT_BYTES {
            break;
        }
        if let Ok(Some(bytes)) = db.read_verified_blob_any(&hash) {
            if let Ok(text) = std::str::from_utf8(&bytes) {
                let remaining = MAX_FTS_FULLTEXT_BYTES - full_text.len();
                let chunk = truncate_utf8(text, remaining);
                if !full_text.is_empty() && !chunk.is_empty() {
                    full_text.push('\n');
                }
                full_text.push_str(chunk);
            }
        }
    }
    Ok(Some((paper_key.to_string(), title, authors, abstract_text, gists, full_text)))
}

pub(crate) fn refresh_fts_with_conn(conn: &Connection, db: &LibraryDb, paper_key: &str) -> Result<()> {
    conn.execute("DELETE FROM paper_fts WHERE paper_key = ?1", params![paper_key])?;
    if let Some((key, title, authors, abstract_text, gists, full_text)) =
        collect_fts_doc(conn, db, paper_key)?
    {
        conn.execute(
            "INSERT INTO paper_fts (paper_key, title, authors, abstract, gists, full_text)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![key, title, authors, abstract_text, gists, full_text],
        )?;
    }
    Ok(())
}

/// Update `paper_fts` metadata columns (`title`, `authors`, `abstract`) purely inside SQLite
/// without reading any `paper_text` blobs from disk while holding a write transaction.
pub(crate) fn refresh_fts_metadata_in_tx(conn: &Connection, paper_key: &str) -> Result<()> {
    let paper = conn
        .query_row(
            "SELECT title, authors_json, abstract FROM paper WHERE paper_key = ?1",
            params![paper_key],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)),
        )
        .optional()?;
    let existing = conn
        .query_row(
            "SELECT gists, full_text FROM paper_fts WHERE paper_key = ?1 LIMIT 1",
            params![paper_key],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?;
    conn.execute("DELETE FROM paper_fts WHERE paper_key = ?1", params![paper_key])?;
    let Some((title, authors_json, abstract_text)) = paper else {
        return Ok(());
    };
    let authors = serde_json::from_str::<Vec<String>>(&authors_json)
        .unwrap_or_default()
        .join(" ");
    let (gists, full_text) = match existing {
        Some((g, ft)) => (g, ft),
        None => {
            let mut stmt = conn.prepare(
                "SELECT gist FROM finding WHERE paper_key = ?1 AND gist <> '' ORDER BY recorded_ms ASC, evidence_seq ASC",
            )?;
            let rows = stmt.query_map(params![paper_key], |r| r.get::<_, String>(0))?;
            let gists = rows.collect::<std::result::Result<Vec<_>, _>>()?.join("\n");
            (gists, String::new())
        }
    };
    conn.execute(
        "INSERT INTO paper_fts (paper_key, title, authors, abstract, gists, full_text)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![paper_key, title, authors, abstract_text, gists, full_text],
    )?;
    Ok(())
}

fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Turn arbitrary user input into a safe FTS5 `MATCH` query by quoting every token so
/// operators (`OR`, `NOT`, `NEAR`, `:`, `*`, `()`, `-`) cannot trigger FTS5 syntax errors.
pub(crate) fn build_safe_fts_query(raw: &str) -> Option<String> {
    let mut terms = Vec::new();
    for token in raw.split_whitespace() {
        let cleaned: String = token.chars().filter(|c| !c.is_control()).collect();
        if cleaned.is_empty() || !cleaned.chars().any(char::is_alphanumeric) {
            continue;
        }
        let escaped = cleaned.replace('"', "\"\"");
        terms.push(format!("\"{escaped}\""));
    }
    (!terms.is_empty()).then(|| terms.join(" "))
}
