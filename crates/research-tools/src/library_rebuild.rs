//! Rebuild `<data_dir>/library/library.db` from `<data_dir>/ledger.db` and `<data_dir>/blobs/`.
//!
//! The library is strictly an index and cache, never evidence (`00-overview.md` 2.7;
//! `explore.md` 4.3). Deleting `library.db` and calling [`rebuild_from_ledger`] reconstructs
//! every ledger-derived table identically and restores non-rebuildable user marks from
//! `<data_dir>/library/user_marks.json` (falling back to `user_marks.json.bak`).

use crate::library_marks::load_sidecar;
use crate::library_rebuild_lit::extract_literature_papers;
use crate::library_schema::{
    FindingRecord, FulltextRecord, LibraryIds, PageBasis, PaperFlags, PaperRecord, PaperTextRecord,
    RebuildReport, SeenRecord, SeenRole, SkeletonRecord,
};
use crate::paper_fetch::parse_paper_input;
use crate::paper_key::{split_arxiv_id, Mode, PaperKey};
use ledger::library_db::{LibraryDb, RebuildBatch};
use ledger::{Event, Ledger};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::io;

#[derive(Default)]
struct RebuildState {
    papers: BTreeMap<String, PaperRecord>,
    aliases: BTreeMap<String, String>,
    fulltext: BTreeMap<(String, String), FulltextRecord>,
    seen: BTreeMap<(String, String, SeenRole), SeenRecord>,
    skeletons: BTreeMap<(String, String, String), SkeletonRecord>,
    findings: BTreeMap<(String, String, u64), FindingRecord>,
    paper_text: BTreeMap<(String, String, String, String), PaperTextRecord>,
    warnings: Vec<String>,
}

/// Rebuild all ledger-derived tables in `db` from `<data_dir>/ledger.db` and `<data_dir>/blobs/`,
/// then restore user marks from `<data_dir>/library/user_marks.json` (or `.bak`).
pub(crate) fn rebuild_from_ledger(db: &mut LibraryDb) -> Result<RebuildReport, String> {
    let data_dir = db.data_dir().to_path_buf();
    let mut state = RebuildState::default();
    let mut sessions_scanned = 0usize;
    let mut events_scanned = 0usize;

    match Ledger::open_existing(&data_dir) {
        Ok(ledger) => {
            let mut sessions = ledger
                .sessions()
                .map_err(|e| format!("failed to list ledger sessions: {e}"))?;
            sessions.sort_by(|a, b| a.0.cmp(&b.0));
            for (session_id, _) in sessions {
                let events = match ledger.read_session(&session_id) {
                    Ok(evs) => evs,
                    Err(e) => {
                        state
                            .warnings
                            .push(format!("skipped unreadable session `{session_id}`: {e}"));
                        continue;
                    }
                };
                sessions_scanned += 1;
                events_scanned += events.len();
                let mode = detect_session_mode(&events);
                ingest_session_events(&ledger, &session_id, mode, &events, &mut state);
            }
        }
        Err(ledger::Error::Io(e)) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("failed to open ledger for library rebuild: {e}")),
    }

    let batch = finalize_batch(&mut state)?;
    let mut report = RebuildReport {
        sessions_scanned,
        events_scanned,
        papers: batch.papers.len(),
        aliases: batch.aliases.len(),
        fulltext_entries: batch.fulltext.len(),
        seen_entries: batch.seen.len(),
        skeleton_entries: batch.skeletons.len(),
        finding_entries: batch.findings.len(),
        paper_text_entries: batch.paper_text.len(),
        user_marks_restored: 0,
        warnings: state.warnings,
    };

    db.apply_rebuild_batch(&batch)
        .map_err(|e| format!("failed to write rebuilt library index: {e}"))?;

    if let Some(marks) = load_sidecar(db.library_dir())? {
        let rows: Vec<_> = marks
            .iter()
            .map(|m| m.to_row())
            .collect::<Result<Vec<_>, _>>()?;
        db.replace_all_user_marks(&rows)
            .map_err(|e| format!("failed to restore user marks into library.db: {e}"))?;
        report.user_marks_restored = rows.len();
    }
    Ok(report)
}

fn detect_session_mode(events: &[Event]) -> Mode {
    let mut mode = Mode::Research;
    for ev in events {
        match ev.kind.as_str() {
            "investigation.started" => mode = Mode::Investigate,
            "investigation.handoff" => mode = Mode::Research,
            "idea.started" => mode = Mode::Explore,
            "solve.started" => mode = Mode::Solve,
            _ => {}
        }
    }
    mode
}

fn ingest_session_events(
    ledger: &Ledger,
    session_id: &str,
    mode: Mode,
    events: &[Event],
    state: &mut RebuildState,
) {
    let mut source_map: HashMap<String, (PaperKey, Option<String>)> = HashMap::new();
    for ev in events {
        match ev.kind.as_str() {
            "literature.fetched" => {
                for (rec, role) in extract_literature_papers(ledger, ev) {
                    let key = rec.key.clone();
                    merge_paper(state, rec);
                    record_seen(state, key, session_id, role, ev.ts_ms);
                }
            }
            "source.ingested" => {
                handle_source_ingested(session_id, mode, ev, &mut source_map, state)
            }
            "investigation.skeleton" => handle_skeleton(ev, &source_map, state),
            "investigation.evidence" | "evidence.quote" | "evidence.abstract" => {
                handle_finding(session_id, ev, &source_map, state)
            }
            "idea.paper" | "idea.triage" => handle_idea_seen(session_id, ev, state),
            "paper.text_extracted" => handle_paper_text(ev, &source_map, state),
            _ => {}
        }
    }
}

fn handle_source_ingested(
    session_id: &str,
    mode: Mode,
    ev: &Event,
    source_map: &mut HashMap<String, (PaperKey, Option<String>)>,
    state: &mut RebuildState,
) {
    let p = &ev.payload;
    let raw_blob = p["raw_blob"].as_str().unwrap_or("").trim();
    if raw_blob.len() != 64 || !raw_blob.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return;
    }
    let origin = p["origin"].as_str().unwrap_or("upload");
    let (key, version, mut ids) = derive_ingested_key(p, origin, raw_blob);
    let file_alias = format!("file:{}", &raw_blob[..16]);

    if let Some(sid) = p["source_id"].as_str().filter(|s| !s.is_empty()) {
        source_map.insert(sid.to_string(), (key.clone(), version.clone()));
    }
    if key.kind() == crate::paper_key::PaperKeyKind::Arxiv && ids.arxiv.is_none() {
        ids.arxiv = Some(key.value().to_string());
    } else if key.kind() == crate::paper_key::PaperKeyKind::Doi && ids.doi.is_none() {
        ids.doi = Some(key.value().to_string());
    }

    let rec = PaperRecord {
        key: key.clone(),
        ids,
        title: p["title"].as_str().unwrap_or("").to_string(),
        authors: Vec::new(),
        year: None,
        abstract_text: String::new(),
        bibtex: String::new(),
        flags: PaperFlags::default(),
        first_seen_ms: ev.ts_ms,
        last_seen_ms: ev.ts_ms,
    };
    merge_paper(state, rec);
    if file_alias != key.as_str() {
        state.aliases.insert(file_alias, key.to_string());
    }

    let format = p["format"].as_str().unwrap_or("unknown").to_string();
    let bytes = p["bytes"].as_u64().unwrap_or(0);
    let ft_origin = normalize_fulltext_origin(origin, &format);
    let ft_key = (key.to_string(), version.clone().unwrap_or_default());
    let ft_rec = FulltextRecord {
        key: key.clone(),
        version,
        raw_blob: Some(raw_blob.to_string()),
        format,
        bytes,
        origin: ft_origin,
        fetched_ms: ev.ts_ms,
        file_path: None,
        file_size: None,
        file_mtime_ms: None,
        file_sha256: None,
    };
    state
        .fulltext
        .entry(ft_key)
        .and_modify(|existing| {
            if ft_rec.fetched_ms >= existing.fetched_ms {
                *existing = ft_rec.clone();
            }
        })
        .or_insert(ft_rec);

    let role = p["role"]
        .as_str()
        .and_then(|r| SeenRole::parse(r).ok())
        .unwrap_or(match mode {
            Mode::Investigate => SeenRole::Investigated,
            Mode::Solve => SeenRole::SolveSource,
            Mode::Explore | Mode::Research => SeenRole::Read,
        });
    record_seen(state, key, session_id, role, ev.ts_ms);
}

fn derive_ingested_key(p: &Value, origin: &str, raw_blob: &str) -> (PaperKey, Option<String>, LibraryIds) {
    let explicit_ver = p["version"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    if let Some(raw_key) = p["paper_key"].as_str().and_then(|s| PaperKey::parse(s).ok()) {
        return (raw_key, explicit_ver, LibraryIds::default());
    }
    if let Some(lib_key) = origin
        .strip_prefix("library:")
        .and_then(|s| PaperKey::parse(s.trim()).ok())
    {
        return (lib_key, explicit_ver, LibraryIds::default());
    }
    if let Some(cover) = p["arxiv_cover_id"].as_str() {
        if let Ok((base, ver)) = split_arxiv_id(cover) {
            if let Ok(k) = PaperKey::from_arxiv(&base) {
                return (
                    k,
                    explicit_ver.or(ver),
                    LibraryIds { arxiv: Some(base), ..Default::default() },
                );
            }
        }
    }
    if let Ok(pref) = parse_paper_input(origin) {
        return (
            pref.key,
            explicit_ver.or(pref.version),
            LibraryIds::from_paper_ids(&pref.ids),
        );
    }
    let file_key = PaperKey::parse(&format!("file:{}", &raw_blob[..16]))
        .expect("64-hex raw_blob prefix is always a valid file paper key");
    (file_key, explicit_ver, LibraryIds::default())
}

fn normalize_fulltext_origin(origin: &str, format: &str) -> String {
    let lower = origin.to_ascii_lowercase();
    if lower.contains("arxiv.org/e-print/") || lower.contains("/e-print/") {
        "arxiv_eprint".into()
    } else if lower.contains("arxiv.org/pdf/") || lower.contains("/pdf/") {
        "arxiv_pdf".into()
    } else if lower.starts_with("library:") {
        "ledger".into()
    } else if format == "tex" && lower.starts_with("http") {
        "arxiv_eprint".into()
    } else {
        "upload".into()
    }
}

fn handle_skeleton(
    ev: &Event,
    source_map: &HashMap<String, (PaperKey, Option<String>)>,
    state: &mut RebuildState,
) {
    let p = &ev.payload;
    let resolved = p["paper_key"]
        .as_str()
        .or_else(|| p["key"].as_str())
        .and_then(|s| PaperKey::parse(s).ok())
        .map(|k| (k, p["version"].as_str().map(str::to_string)))
        .or_else(|| {
            p["source_id"]
                .as_str()
                .and_then(|sid| source_map.get(sid).cloned())
        });
    let Some((key, version)) = resolved else {
        return;
    };
    let builder = p["builder"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("tex");
    let blob = p["blob_sha256"]
        .as_str()
        .or_else(|| p["raw_blob"].as_str())
        .map(str::trim)
        .unwrap_or("");
    if blob.len() != 64 || !blob.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return;
    }
    let ver = p["version"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or(version);
    ensure_paper_stub(state, &key, ev.ts_ms);
    let rec = SkeletonRecord {
        key: key.clone(),
        version: ver.clone(),
        builder: builder.to_string(),
        blob_sha256: blob.to_string(),
        built_ms: ev.ts_ms,
    };
    state.skeletons.insert(
        (key.to_string(), ver.unwrap_or_default(), builder.to_string()),
        rec,
    );
}

fn handle_finding(
    session_id: &str,
    ev: &Event,
    source_map: &HashMap<String, (PaperKey, Option<String>)>,
    state: &mut RebuildState,
) {
    let p = &ev.payload;
    let key = p["paper_key"]
        .as_str()
        .or_else(|| p["key"].as_str())
        .and_then(|s| PaperKey::parse(s).ok())
        .or_else(|| {
            p["source_id"]
                .as_str()
                .and_then(|sid| source_map.get(sid).map(|(k, _)| k.clone()))
        });
    let Some(key) = key else {
        return;
    };
    let cited_range = p["cited_range"]
        .as_array()
        .filter(|a| a.len() == 2)
        .and_then(|a| Some([a[0].as_u64()? as usize, a[1].as_u64()? as usize]))
        .or_else(|| {
            Some([
                p["start_line"].as_u64()? as usize,
                p["end_line"].as_u64()? as usize,
            ])
        });
    ensure_paper_stub(state, &key, ev.ts_ms);
    let rec = FindingRecord {
        key: key.clone(),
        session_id: session_id.to_string(),
        evidence_seq: ev.seq,
        role: p["role"].as_str().unwrap_or("").to_string(),
        gist: p["gist"].as_str().unwrap_or("").to_string(),
        quote: p["quote"].as_str().unwrap_or("").to_string(),
        cited_range,
        recorded_ms: ev.ts_ms,
    };
    state
        .findings
        .insert((key.to_string(), session_id.to_string(), ev.seq), rec);
}

fn handle_idea_seen(session_id: &str, ev: &Event, state: &mut RebuildState) {
    let p = &ev.payload;
    let Some(key) = p["paper_key"]
        .as_str()
        .or_else(|| p["key"].as_str())
        .and_then(|s| PaperKey::parse(s).ok())
    else {
        return;
    };
    let role = match p["status"].as_str().or_else(|| p["role"].as_str()).unwrap_or("") {
        "triaged" => SeenRole::Triaged,
        "read" | "read_full" | "read_passages" => SeenRole::Read,
        "candidate" => SeenRole::Candidate,
        other => match SeenRole::parse(other) {
            Ok(r) => r,
            Err(_) => return,
        },
    };
    ensure_paper_stub(state, &key, ev.ts_ms);
    record_seen(state, key, session_id, role, ev.ts_ms);
}

fn handle_paper_text(
    ev: &Event,
    source_map: &HashMap<String, (PaperKey, Option<String>)>,
    state: &mut RebuildState,
) {
    let p = &ev.payload;
    let key = p["paper_key"]
        .as_str()
        .or_else(|| p["key"].as_str())
        .and_then(|s| PaperKey::parse(s).ok())
        .or_else(|| {
            p["source_id"]
                .as_str()
                .and_then(|sid| source_map.get(sid).map(|(k, _)| k.clone()))
        });
    let Some(key) = key else {
        return;
    };
    let file_sha256 = p["file_sha256"].as_str().unwrap_or("").trim();
    let text_blob = p["text_blob"].as_str().unwrap_or("").trim();
    let extractor = p["extractor"].as_str().unwrap_or("").trim();
    if file_sha256.len() != 64 || text_blob.len() != 64 || extractor.is_empty() {
        return;
    }
    let version = p["version"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let page_basis: Vec<PageBasis> = serde_json::from_value(p["page_basis"].clone()).unwrap_or_default();
    ensure_paper_stub(state, &key, ev.ts_ms);
    let rec = PaperTextRecord {
        key: key.clone(),
        version: version.clone(),
        file_sha256: file_sha256.to_string(),
        extractor: extractor.to_string(),
        text_blob: text_blob.to_string(),
        page_basis,
        extracted_ms: ev.ts_ms,
    };
    state.paper_text.insert(
        (
            key.to_string(),
            version.unwrap_or_default(),
            file_sha256.to_string(),
            extractor.to_string(),
        ),
        rec,
    );
}

fn ensure_paper_stub(state: &mut RebuildState, key: &PaperKey, ts_ms: i64) {
    state
        .papers
        .entry(key.to_string())
        .and_modify(|p| {
            p.first_seen_ms = p.first_seen_ms.min(ts_ms);
            p.last_seen_ms = p.last_seen_ms.max(ts_ms);
        })
        .or_insert_with(|| PaperRecord {
            key: key.clone(),
            ids: LibraryIds::default(),
            title: String::new(),
            authors: Vec::new(),
            year: None,
            abstract_text: String::new(),
            bibtex: String::new(),
            flags: PaperFlags::default(),
            first_seen_ms: ts_ms,
            last_seen_ms: ts_ms,
        });
}

fn merge_paper(state: &mut RebuildState, incoming: PaperRecord) {
    let key_str = incoming.key.to_string();
    for alias in incoming.ids.canonical_aliases() {
        if alias != key_str {
            state.aliases.entry(alias).or_insert_with(|| key_str.clone());
        }
    }
    state
        .papers
        .entry(key_str)
        .and_modify(|cur| {
            cur.ids.merge_from(&incoming.ids);
            if cur.title.is_empty() && !incoming.title.is_empty() {
                cur.title = incoming.title.clone();
            }
            if cur.authors.is_empty() && !incoming.authors.is_empty() {
                cur.authors = incoming.authors.clone();
            }
            if cur.year.is_none() {
                cur.year = incoming.year;
            }
            if cur.abstract_text.is_empty() && !incoming.abstract_text.is_empty() {
                cur.abstract_text = incoming.abstract_text.clone();
            }
            if cur.bibtex.is_empty() && !incoming.bibtex.is_empty() {
                cur.bibtex = incoming.bibtex.clone();
            }
            cur.flags.merge_from(&incoming.flags);
            cur.first_seen_ms = cur.first_seen_ms.min(incoming.first_seen_ms);
            cur.last_seen_ms = cur.last_seen_ms.max(incoming.last_seen_ms);
        })
        .or_insert(incoming);
}

fn record_seen(
    state: &mut RebuildState,
    key: PaperKey,
    session_id: &str,
    role: SeenRole,
    seen_ms: i64,
) {
    let map_key = (key.to_string(), session_id.to_string(), role);
    state
        .seen
        .entry(map_key)
        .and_modify(|cur| cur.seen_ms = cur.seen_ms.max(seen_ms))
        .or_insert(SeenRecord {
            key,
            session_id: session_id.to_string(),
            role,
            seen_ms,
        });
}

fn finalize_batch(state: &mut RebuildState) -> Result<RebuildBatch, String> {
    // Ensure every paper's aliases point to an existing canonical key in `papers`.
    let mut batch = RebuildBatch::default();
    for rec in state.papers.values() {
        batch.papers.push(rec.to_row()?);
    }
    for (ident, target) in &state.aliases {
        if ident != target && state.papers.contains_key(target) {
            batch.aliases.push((ident.clone(), target.clone()));
        }
    }
    for rec in state.fulltext.values() {
        batch.fulltext.push(rec.to_row());
    }
    for rec in state.seen.values() {
        batch.seen.push(rec.to_row());
    }
    for rec in state.skeletons.values() {
        batch.skeletons.push(rec.to_row());
    }
    for rec in state.findings.values() {
        batch.findings.push(rec.to_row());
    }
    for rec in state.paper_text.values() {
        batch.paper_text.push(rec.to_row()?);
    }
    Ok(batch)
}
