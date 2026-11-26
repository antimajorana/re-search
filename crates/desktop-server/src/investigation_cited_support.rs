//! Supporting F10 library/skeleton caching, `BibRef -> PaperRef` resolution, and response
//! builders for `desktop_server::investigation_cited` (Investigate V11).

use crate::investigation::{is_investigation, latest};
use crate::investigation_mapview::{
    attach_pdf_page_metadata, build_mapview_json, format_quality_line, read_verified_node_text,
};
use crate::investigation_skeleton::{
    build_skeleton_from_receipt, derive_paper_identity, ensure_skeleton_on_open, is_pdf_receipt,
    is_tex_receipt, EVENT_INVESTIGATION_SKELETON,
};
use crate::routes_session::{bad, internal, not_found};
use crate::state::{now_ms, DesktopState};
use ledger::{Event, Ledger};
use research_tools::library::{
    Library, LibraryIds, LibraryQuery, PaperFlags, PaperRecord, SeenRecord, SeenRole,
    SkeletonRecord as LibSkeletonRecord,
};
use research_tools::paper_key::{
    canonical_doi, split_arxiv_id, Mode, NodeRef, PaperIds, PaperKey, PaperKeyKind, PaperRef,
};
use research_tools::pdf_pages::PageBasis;
use research_tools::skeleton::{
    compute_files_root_sha256, format_candidate_label, node_page_basis, page_range_for_node,
    render_formulas_as_math, BibRef, IngestedPdfSource, LocateCandidate, PaperSkeleton,
    SkeletonBuilder,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};

pub(crate) const EVENT_SOURCE_INGESTED: &str = "source.ingested";
const CANDIDATE_EXCERPT_CHARS: usize = 200;

static IN_FLIGHT_FETCHES: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();

fn in_flight_map() -> &'static Mutex<HashMap<String, usize>> {
    IN_FLIGHT_FETCHES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// RAII reservation preventing concurrent cited lookups from racing past a session's fetch budget.
pub(crate) struct InFlightFetchGuard {
    session_id: String,
}

impl InFlightFetchGuard {
    pub(crate) fn try_reserve(session_id: &str, used: usize, limit: usize) -> Result<Self, usize> {
        let mut map = in_flight_map().lock().unwrap_or_else(|e| e.into_inner());
        let effective = used.saturating_add(map.get(session_id).copied().unwrap_or(0));
        if effective >= limit { return Err(effective); }
        *map.entry(session_id.to_string()).or_insert(0) += 1;
        Ok(Self { session_id: session_id.to_string() })
    }
}

impl Drop for InFlightFetchGuard {
    fn drop(&mut self) {
        let mut map = in_flight_map().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(count) = map.get_mut(&self.session_id) {
            if *count <= 1 { map.remove(&self.session_id); } else { *count -= 1; }
        }
    }
}

pub(crate) struct PinnedContext {
    pub primary_source_id: String,
    pub tag: String,
    pub bib: BibRef,
    pub return_ref: NodeRef,
}

pub(crate) fn load_pinned_context(
    state: &DesktopState,
    session_id: &str,
    val: &Value,
) -> Result<(Vec<Event>, PinnedContext), (u16, String)> {
    let tag = non_empty(&val["tag"]).ok_or_else(|| bad("Provide `tag` in request body."))?;
    let bib_key =
        non_empty(&val["bib_key"]).ok_or_else(|| bad("Provide `bib_key` in request body."))?;
    let events = read_session_events(state, session_id)?;
    if !is_investigation(&events) {
        return Err(bad("Open an Investigation session before looking up citations."));
    }
    let rec = ensure_skeleton_on_open(state, session_id)?
        .or_else(|| load_latest_skeleton_fallback(state, session_id, &events))
        .ok_or_else(|| bad("Pin a paper and build its skeleton before looking up citations."))?;
    let node = rec.skeleton.nodes.iter().find(|n| n.tag.as_deref() == Some(tag))
        .ok_or_else(|| not_found(format!("node `{tag}` not found in pinned skeleton")))?;
    let bib = rec.skeleton.bib.iter().find(|b| b.key == bib_key)
        .ok_or_else(|| not_found(format!("bibliography key `{bib_key}` not found in pinned skeleton")))?
        .clone();
    let return_ref = NodeRef {
        session: session_id.to_string(),
        mode: Mode::Investigate,
        id: tag.to_string(),
        paper: Some(paper_ref_from_key(rec.skeleton.paper_key.clone(), rec.skeleton.version.clone())),
        node_fp: Some(node.fp.clone()),
    };
    let events = read_session_events(state, session_id)?;
    Ok((events, PinnedContext {
        primary_source_id: rec.skeleton.source_id,
        tag: tag.to_string(),
        bib,
        return_ref,
    }))
}

pub(crate) fn load_or_build_cached_skeleton(
    state: &DesktopState,
    lib: &Library,
    session_id: &str,
    receipt: &Event,
    paper_key: &PaperKey,
    version: Option<&str>,
) -> Result<(PaperSkeleton, String, bool), (u16, String)> {
    let builder = if is_tex_receipt(receipt) {
        SkeletonBuilder::Tex1
    } else if is_pdf_receipt(receipt) {
        SkeletonBuilder::Pdf1
    } else {
        return Err(bad("source has no supported TeX or PDF skeleton builder"));
    };
    let receipt_hashes = receipt_files_sha256(receipt);
    let expected_root = (!receipt_hashes.is_empty()).then(|| compute_files_root_sha256(&receipt_hashes));
    let ledger = state.open_ledger().map_err(internal)?;

    if let Some((skel, blob_hash)) = try_load_library_skeleton(
        lib,
        &ledger,
        paper_key,
        version,
        builder,
        expected_root.as_deref(),
        &receipt_hashes,
    ) {
        return Ok((skel, blob_hash, true));
    }

    let env = state.research_env_for_session(session_id).map_err(internal)?;
    let (skel, _timing) = build_skeleton_from_receipt(
        &env.sandbox(),
        session_id,
        receipt,
        paper_key.clone(),
        version.map(str::to_string),
    )
    .map_err(internal)?;
    let bytes = serde_json::to_vec(&skel).map_err(storage)?;
    let blob_hash = ledger.put_blob(&bytes).map_err(storage)?;
    let _ = lib.upsert_skeleton(&LibSkeletonRecord {
        key: skel.paper_key.clone(),
        version: skel.version.clone(),
        builder: skel.builder.as_str().to_string(),
        blob_sha256: blob_hash.clone(),
        built_ms: now_ms(),
    });
    Ok((skel, blob_hash, false))
}

fn try_load_library_skeleton(
    lib: &Library,
    ledger: &Ledger,
    paper_key: &PaperKey,
    version: Option<&str>,
    builder: SkeletonBuilder,
    expected_root: Option<&str>,
    receipt_hashes: &BTreeMap<String, String>,
) -> Option<(PaperSkeleton, String)> {
    let mut candidates = Vec::new();
    if let Ok(Some(exact)) = lib.get_skeleton(paper_key, version, builder.as_str()) {
        candidates.push(exact);
    }
    if version.is_none() {
        if let Ok(Some(detail)) = lib.get_paper(paper_key) {
            for rec in detail.skeletons {
                if rec.builder == builder.as_str() {
                    candidates.push(rec);
                }
            }
        }
    }
    for rec in candidates {
        let Ok(bytes) = ledger.get_blob(&rec.blob_sha256) else { continue; };
        if ledger::sha256_hex(&bytes) != rec.blob_sha256 { continue; }
        let Ok(mut skel) = serde_json::from_slice::<PaperSkeleton>(&bytes) else { continue; };
        if expected_root.is_some_and(|root| skel.files_root_sha256 == root) {
            return Some((skel, rec.blob_sha256));
        }
        if try_remap_single_file_skeleton(&mut skel, receipt_hashes) {
            let remapped_hash = serde_json::to_vec(&skel)
                .map(|b| ledger::sha256_hex(&b))
                .unwrap_or(rec.blob_sha256);
            return Some((skel, remapped_hash));
        }
    }
    None
}

fn try_remap_single_file_skeleton(
    skel: &mut PaperSkeleton,
    receipt_hashes: &BTreeMap<String, String>,
) -> bool {
    if receipt_hashes.len() != 1 { return false; }
    let Some((new_path, file_sha)) = receipt_hashes.iter().next() else { return false; };
    let old_path = skel.nodes.iter().find_map(|n| n.spans.first().map(|s| s.path.clone()))
        .or_else(|| skel.edges.first().map(|e| e.site.path.clone()));
    let Some(old_path) = old_path else { return false; };
    let check_map = BTreeMap::from([(old_path.clone(), file_sha.clone())]);
    if compute_files_root_sha256(&check_map) != skel.files_root_sha256 { return false; }
    if old_path != *new_path {
        for n in &mut skel.nodes {
            for s in &mut n.spans {
                if s.path == old_path { s.path = new_path.clone(); }
            }
        }
        for e in &mut skel.edges {
            if e.site.path == old_path { e.site.path = new_path.clone(); }
        }
        for b in &mut skel.bib {
            for s in &mut b.cited_at {
                if s.path == old_path { s.path = new_path.clone(); }
            }
            for loc in &mut b.locators {
                if loc.site.path == old_path { loc.site.path = new_path.clone(); }
            }
        }
        for sym in &mut skel.symbols {
            if sym.span.path == old_path { sym.span.path = new_path.clone(); }
        }
        skel.files_root_sha256 = compute_files_root_sha256(receipt_hashes);
    }
    true
}

pub(crate) fn build_statement_json(
    sandbox: &research_tools::sandbox::WorkspaceSandbox,
    session_id: &str,
    receipt: &Event,
    skeleton: &PaperSkeleton,
    tag: &str,
    pdf: Option<&IngestedPdfSource>,
) -> Result<Value, (u16, String)> {
    let node = skeleton
        .nodes
        .iter()
        .find(|n| n.tag.as_deref() == Some(tag))
        .ok_or_else(|| not_found(format!("cited node `{tag}` not found")))?;
    let raw = read_verified_node_text(sandbox, session_id, receipt, skeleton, tag).map_err(internal)?;
    let (start_page, end_page, page_basis, ocr, render_math) = if let Some(p) = pdf {
        let range = page_range_for_node(&p.page_map, node);
        let basis = node_page_basis(&p.page_map, node);
        (
            range.map(|r| r.0),
            range.map(|r| r.1),
            basis,
            basis.is_some_and(PageBasis::is_scan),
            basis.is_some_and(render_formulas_as_math),
        )
    } else {
        (None, None, None, false, true)
    };
    Ok(json!({
        "tag": tag, "kind": node.kind, "label": format_candidate_label(node),
        "number": node.number, "number_basis": node.number_basis, "title": node.title,
        "text": raw["text"], "spans": node.spans,
        "start_page": start_page, "end_page": end_page, "page_basis": page_basis,
        "ocr": ocr, "render_math": render_math,
        "proof_tag": raw["proof_tag"], "proof_text": raw["proof_text"],
    }))
}

pub(crate) fn enrich_candidates(
    sandbox: &research_tools::sandbox::WorkspaceSandbox,
    session_id: &str,
    receipt: &Event,
    skeleton: &PaperSkeleton,
    candidates: &[LocateCandidate],
) -> Vec<Value> {
    candidates
        .iter()
        .map(|c| {
            let excerpt = read_verified_node_text(sandbox, session_id, receipt, skeleton, &c.tag)
                .ok()
                .and_then(|v| v["text"].as_str().map(str::to_string))
                .map(|s| clamp_excerpt(&s, CANDIDATE_EXCERPT_CHARS));
            let mut v = serde_json::to_value(c).unwrap_or_else(|_| json!({}));
            if let (Some(obj), Some(ex)) = (v.as_object_mut(), excerpt) {
                obj.insert("excerpt".to_string(), json!(ex));
            }
            v
        })
        .collect()
}

pub(crate) fn build_cited_view_json(
    skeleton: &PaperSkeleton,
    blob_hash: &str,
    source_id: &str,
    selected_tag: Option<&str>,
    pdf: Option<&IngestedPdfSource>,
) -> Value {
    let mut mv = build_mapview_json(skeleton, 0, blob_hash, blob_hash, None);
    mv["source_id"] = json!(source_id);
    if let Some(p) = pdf {
        attach_pdf_page_metadata(&mut mv, skeleton, p);
    }
    json!({
        "source_id": source_id, "paper_key": skeleton.paper_key, "version": skeleton.version,
        "builder": skeleton.builder, "quality": skeleton.quality,
        "quality_line": format_quality_line(skeleton), "selected_tag": selected_tag,
        "mapview": mv,
    })
}

pub(crate) fn resolve_bib_to_paper_ref(
    bib: &BibRef,
    lib: &Library,
) -> Result<Option<PaperRef>, (u16, String)> {
    let doi = bib.doi.as_deref().and_then(|d| canonical_doi(d).ok());
    if let Some(raw_ax) = bib.arxiv_id.as_deref() {
        if let Ok((base, ver)) = split_arxiv_id(raw_ax) {
            let version = ver.or_else(|| extract_arxiv_version_from_raw(&bib.raw, &base));
            let key = PaperKey::from_arxiv(&base).map_err(internal)?;
            return Ok(Some(PaperRef {
                key,
                version,
                ids: PaperIds { arxiv: Some(base), doi, ..Default::default() },
            }));
        }
    }
    if let Some(d) = doi {
        let key = PaperKey::from_doi(&d).map_err(internal)?;
        return Ok(Some(PaperRef {
            key,
            version: None,
            ids: PaperIds { doi: Some(d), ..Default::default() },
        }));
    }
    if let Some(title) = bib.title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        let norm_target = normalize_title(title);
        let hits = lib
            .search(&LibraryQuery {
                title: Some(title.to_string()),
                limit: Some(5),
                ..Default::default()
            })
            .map_err(internal)?;
        for h in hits {
            if normalize_title(&h.paper.title) == norm_target {
                if let Some(detail) = lib.get_paper(&h.paper.key).map_err(internal)? {
                    return Ok(Some(detail.paper.paper_ref(None)));
                }
            }
        }
    }
    Ok(None)
}

pub(crate) fn resolve_library_canonical(
    lib: &Library,
    paper_ref: &PaperRef,
) -> Result<PaperKey, (u16, String)> {
    if let Some(k) = lib.resolve_key(&paper_ref.key).map_err(internal)? {
        return Ok(k);
    }
    if let Some(doi) = paper_ref.ids.doi.as_deref().and_then(|d| PaperKey::from_doi(d).ok()) {
        if let Some(k) = lib.resolve_key(&doi).map_err(internal)? {
            return Ok(k);
        }
    }
    Ok(paper_ref.key.clone())
}

pub(crate) fn find_session_source_for_key(
    events: &[Event],
    cited_event_kind: &str,
    key: &PaperKey,
) -> Option<String> {
    for ev in events.iter().rev() {
        if ev.kind == cited_event_kind
            && ev.payload["resolved"]["key"].as_str() == Some(key.as_str())
        {
            if let Some(sid) = ev.payload["external_source_id"].as_str() {
                if find_source_receipt(events, sid).is_some() {
                    return Some(sid.to_string());
                }
            }
        }
    }
    for ev in events.iter().rev() {
        if ev.kind != EVENT_SOURCE_INGESTED {
            continue;
        }
        if let Ok((k, _)) = derive_paper_identity(ev, None, None) {
            if &k == key {
                if let Some(sid) = ev.payload["source_id"].as_str() {
                    return Some(sid.to_string());
                }
            }
        }
    }
    None
}

/// Resolves the canonical `(PaperKey, version)` for an already-ingested external source receipt,
/// checking prior `investigation.cited_lookup` events, `library:<PaperKey>` origins, and `derive_paper_identity`.
pub(crate) fn resolve_external_source_identity(
    events: &[Event],
    cited_event_kind: &str,
    receipt: &Event,
) -> Result<(PaperKey, Option<String>), (u16, String)> {
    if let Some(sid) = receipt.payload["source_id"].as_str() {
        for ev in events.iter().rev() {
            if ev.kind == cited_event_kind
                && ev.payload["external_source_id"].as_str() == Some(sid)
            {
                if let Some(raw_key) = ev.payload["resolved"]["key"].as_str() {
                    if let Ok(key) = PaperKey::parse(raw_key) {
                        let ver = non_empty(&ev.payload["resolved"]["version"]).map(str::to_string);
                        return Ok((key, ver));
                    }
                }
            }
        }
    }
    if let Some(origin) = non_empty(&receipt.payload["origin"]) {
        if let Some(rest) = origin.strip_prefix("library:") {
            if let Ok(key) = PaperKey::parse(rest) {
                return Ok((key, None));
            }
        }
    }
    derive_paper_identity(receipt, None, None).map_err(internal)
}

pub(crate) fn record_cited_in_library(
    lib: &Library,
    session_id: &str,
    paper: &PaperRef,
    title: Option<&str>,
) {
    let ts = now_ms();
    if lib.get_paper(&paper.key).ok().flatten().is_none() {
        let _ = lib.upsert_paper(&PaperRecord {
            key: paper.key.clone(),
            ids: LibraryIds::from_paper_ids(&paper.ids),
            title: title.unwrap_or_default().to_string(),
            authors: Vec::new(),
            year: None,
            abstract_text: String::new(),
            bibtex: String::new(),
            flags: PaperFlags::default(),
            first_seen_ms: ts,
            last_seen_ms: ts,
        });
    }
    let _ = lib.record_seen(&SeenRecord {
        key: paper.key.clone(),
        session_id: session_id.to_string(),
        role: SeenRole::CitedLookup,
        seen_ms: ts,
    });
}

fn load_latest_skeleton_fallback(
    state: &DesktopState,
    session_id: &str,
    events: &[Event],
) -> Option<crate::investigation_skeleton::SkeletonRecord> {
    let ev = latest(events, EVENT_INVESTIGATION_SKELETON)?;
    let sid = ev.payload["source_id"].as_str()?;
    crate::investigation_skeleton::build_and_record_skeleton(state, session_id, sid, None, None).ok()
}

pub(crate) fn read_session_events(
    state: &DesktopState,
    session_id: &str,
) -> Result<Vec<Event>, (u16, String)> {
    state
        .open_ledger()
        .map_err(internal)?
        .read_session(session_id)
        .map_err(storage)
}

pub(crate) fn find_source_receipt<'a>(events: &'a [Event], source_id: &str) -> Option<&'a Event> {
    events.iter().rev().find(|e| {
        e.kind == EVENT_SOURCE_INGESTED && e.payload["source_id"].as_str() == Some(source_id)
    })
}

fn receipt_files_sha256(receipt: &Event) -> BTreeMap<String, String> {
    receipt.payload["files_sha256"]
        .as_object()
        .map(|map| {
            map.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn bib_json(b: &BibRef) -> Value {
    json!({ "key": b.key, "title": b.title, "raw": b.raw, "arxiv_id": b.arxiv_id, "doi": b.doi })
}

pub(crate) fn format_short_ref(bib: &BibRef, paper: &PaperRef) -> String {
    match (paper.key.kind(), paper.version.as_deref()) {
        (PaperKeyKind::Arxiv, Some(v)) => format!("{} (arXiv:{}{v})", bib.key, paper.key.value()),
        (PaperKeyKind::Arxiv, None) => format!("{} (arXiv:{})", bib.key, paper.key.value()),
        (PaperKeyKind::Doi, _) => format!("{} (DOI:{})", bib.key, paper.key.value()),
        _ => bib
            .title
            .as_deref()
            .map(|t| format!("{} — {t}", bib.key))
            .unwrap_or_else(|| bib.key.clone()),
    }
}

pub(crate) fn paper_ref_from_key(key: PaperKey, version: Option<String>) -> PaperRef {
    let v = || Some(key.value().to_string());
    let ids = match key.kind() {
        PaperKeyKind::Arxiv => PaperIds { arxiv: v(), ..Default::default() },
        PaperKeyKind::Doi => PaperIds { doi: v(), ..Default::default() },
        PaperKeyKind::Inspire => PaperIds { inspire: v(), ..Default::default() },
        PaperKeyKind::S2 => PaperIds { s2: v(), ..Default::default() },
        PaperKeyKind::Openalex | PaperKeyKind::File => PaperIds::default(),
    };
    PaperRef { key, version, ids }
}

fn extract_arxiv_version_from_raw(raw: &str, base: &str) -> Option<String> {
    let lower_raw = raw.to_ascii_lowercase();
    let lower_base = base.to_ascii_lowercase();
    let bytes = lower_raw.as_bytes();
    let mut start = 0;
    while let Some(rel) = lower_raw[start..].find(&lower_base) {
        let pos = start + rel;
        let left_ok = pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric();
        let after = pos + lower_base.len();
        if left_ok {
            if let Some(digits_rest) = lower_raw[after..].strip_prefix('v') {
                let dlen = digits_rest.bytes().take_while(u8::is_ascii_digit).count();
                let right_pos = after + 1 + dlen;
                let right_ok = right_pos >= bytes.len() || !bytes[right_pos].is_ascii_alphanumeric();
                if dlen > 0 && dlen <= 4 && right_ok {
                    return Some(format!("v{}", &digits_rest[..dlen]));
                }
            }
        }
        start = after;
    }
    None
}

fn normalize_title(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn clamp_excerpt(s: &str, max_chars: usize) -> String {
    let clean = s.split_whitespace().collect::<Vec<_>>().join(" ");
    match clean.char_indices().nth(max_chars) {
        Some((idx, _)) => format!("{}…", clean[..idx].trim_end()),
        None => clean,
    }
}

pub(crate) fn non_empty(v: &Value) -> Option<&str> {
    v.as_str().map(str::trim).filter(|s| !s.is_empty())
}

pub(crate) fn storage(err: impl ToString) -> (u16, String) {
    internal(err.to_string())
}
