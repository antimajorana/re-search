//! Synthetic test helpers for F15 Explore write-up Set W tests (`writeup_idea.rs`).
//!
//! Zero third-party paper text, zero live network calls, zero model calls.
#![allow(dead_code)]

use ledger::Ledger;
use research_tools::idea_brief::EVENT_IDEA_BRIEF;
use research_tools::idea_map::EVENT_IDEA_MAP;
use research_tools::idea_verdict;
use research_tools::ingest::{ingest_bytes, IngestRequest, IngestedSource};
use research_tools::library::{
    Library, LibraryIds, PaperFlags, PaperRecord, SourceItemRecord, SourceItemState, SourceKind,
    SourceRecord,
};
use research_tools::lit_abstracts::locate_abstract_quote_in_blob;
use research_tools::lit_providers::LitProvider;
use research_tools::paper_key::{PaperKey, PaperRef};
use research_tools::pdf_pages::{PageBasis, PageQualityMetrics, PageRecord};
use research_tools::prior_work::IDEA_VERDICT_EVENT;
use research_tools::quote_core::{
    verify_quote, EvidenceAbstractPayload, EVIDENCE_ABSTRACT_EVENT, EVIDENCE_QUOTE_EVENT,
};
use research_tools::sandbox::WorkspaceSandbox;
use research_tools::search_coverage::{
    IDEA_CANDIDATES_EVENT, IDEA_NOTE_EVENT, IDEA_PAPER_EVENT, IDEA_SEARCH_FINISHED_EVENT,
    IDEA_SEARCH_PLANNED_EVENT, IDEA_STARTED_EVENT, IDEA_STATED_EVENT, IDEA_TRIAGE_EVENT,
};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Mutex;

pub const SYNTH_PAPER_A_TEX: &[u8] = br"\documentclass{article}
\begin{document}
\section{Boundary Scaling}
In a finite monitored chain the Krylov slope scales linearly with lattice size across all even lengths.
We refer to this saturation regime as spread complexity damping throughout the analysis.
\end{document}
";

pub const SYNTH_PAPER_OCR_TXT: &[u8] = b"Page one preface text for synthetic scanned paper.\n\
\x0cPage two setup notes for synthetic scanned paper.\n\
\x0cTheorem 3: The operator growth rate saturates at $b_n = \\alpha n$ for monitored dimer chains.\n";

pub fn synth_arxiv_atom_blob(arxiv_id: &str, title: &str, summary: &str) -> Vec<u8> {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <id>http://arxiv.org/abs/{arxiv_id}v1</id>
    <title>{title}</title>
    <summary>{summary}</summary>
  </entry>
</feed>"#
    )
    .into_bytes()
}

pub fn synth_openalex_rebuilt_blob(work_id: &str, title: &str) -> Vec<u8> {
    json!({
        "results": [{
            "id": format!("https://openalex.org/{work_id}"),
            "title": title,
            "publication_year": 2024,
            "abstract_inverted_index": {
                "Rebuilt": [0],
                "abstract": [1],
                "about": [2],
                "dimer": [3],
                "chains": [4]
            }
        }]
    })
    .to_string()
    .into_bytes()
}

pub fn setup_library_with_citekey(
    data_dir: &Path,
    paper: &PaperRef,
    citekey: &str,
    title: &str,
    author: &str,
    year: i32,
) {
    let lib = Library::open(data_dir).unwrap();
    lib.upsert_paper(&PaperRecord {
        key: paper.key.clone(),
        ids: LibraryIds::from_paper_ids(&paper.ids),
        title: title.into(),
        authors: vec![author.into()],
        year: Some(year),
        abstract_text: "Synthetic library abstract.".into(),
        bibtex: String::new(),
        flags: PaperFlags {
            owned: true,
            venue: Some("Journal of Synthetic Chains".into()),
            ..Default::default()
        },
        first_seen_ms: 1,
        last_seen_ms: 1,
    })
    .unwrap();
    lib.upsert_source(&SourceRecord {
        id: "vault-1".into(),
        kind: SourceKind::KandinskyVault,
        root: "/tmp/synth-vault".into(),
        settings: json!({}),
        added_ms: 1,
        last_sync_ms: Some(1),
        state: "synced".into(),
    })
    .unwrap();
    lib.upsert_source_item(&SourceItemRecord {
        source_id: "vault-1".into(),
        external_id: citekey.into(),
        key: Some(paper.key.clone()),
        citekey: Some(citekey.into()),
        ids: LibraryIds::from_paper_ids(&paper.ids),
        version: paper.version.clone(),
        title: title.into(),
        authors: vec![author.into()],
        year: Some(year),
        item_kind: "journalArticle".into(),
        collections: vec![],
        tags: vec![],
        files: vec![],
        state: SourceItemState::Present,
        updated_ms: 1,
    })
    .unwrap();
}

pub fn append_explore_start_and_brief(
    ledger: &Ledger,
    sid: &str,
    preset: &str,
    idea_text: &str,
    restatement: &str,
    subclaims: &[(&str, &str, &str)],
    library_cov: Option<Value>,
) {
    let mut started = json!({
        "preset": preset,
        "pipeline_version": 1,
        "offline": false,
    });
    if let Some(lib_cov) = library_cov {
        started["library"] = lib_cov;
    }
    ledger.append_with_hash(sid, IDEA_STARTED_EVENT, 1, &started).unwrap();
    ledger
        .append_with_hash(
            sid,
            IDEA_STATED_EVENT,
            1,
            &json!({
                "rev": 1,
                "text": idea_text,
                "text_sha256": ledger::sha256_hex(idea_text.as_bytes()),
                "fields": ["quant-ph"],
                "seeds": [],
                "author": "user",
            }),
        )
        .unwrap();
    let sc_vals: Vec<Value> = subclaims
        .iter()
        .map(|(id, text, quote)| json!({ "id": id, "text": text, "idea_quote": quote }))
        .collect();
    ledger
        .append_with_hash(
            sid,
            EVENT_IDEA_BRIEF,
            1,
            &json!({
                "idea_rev": 1,
                "restatement": restatement,
                "assumptions": ["Finite chain with periodic boundaries"],
                "subclaims": sc_vals,
                "perspectives": [
                    { "id": "p-1", "kind": "exact", "question": "Exact?", "queries": [{ "provider": "arxiv", "q": "Krylov slope" }] },
                    { "id": "p-2", "kind": "other_names", "question": "Aliases?", "queries": [{ "provider": "inspire", "q": "spread complexity damping" }] }
                ],
                "terms": [],
                "questions": [],
                "exclusions": [],
            }),
        )
        .unwrap();
}

#[allow(clippy::too_many_arguments)]
pub fn append_search_and_triage(
    ledger: &Ledger,
    sid: &str,
    queries_finished: Vec<Value>,
    candidates: Vec<Value>,
    triage_items: Vec<Value>,
    library_cov: Option<Value>,
) {
    ledger
        .append_with_hash(
            sid,
            IDEA_SEARCH_PLANNED_EVENT,
            1,
            &json!({
                "round": 1,
                "queries": [
                    { "qid": "q-1", "perspective_id": "p-1", "provider": "arxiv", "q": "Krylov slope", "origin": "model" },
                    { "qid": "q-2", "perspective_id": "p-2", "provider": "inspire", "q": "spread complexity damping", "origin": "model" }
                ]
            }),
        )
        .unwrap();
    let mut fin = json!({ "round": 1, "queries": queries_finished });
    if let Some(lib_cov) = library_cov {
        fin["library"] = lib_cov;
    }
    ledger.append_with_hash(sid, IDEA_SEARCH_FINISHED_EVENT, 1, &fin).unwrap();
    ledger
        .append_with_hash(sid, IDEA_CANDIDATES_EVENT, 1, &json!({ "round": 1, "candidates": candidates }))
        .unwrap();
    ledger
        .append_with_hash(sid, IDEA_TRIAGE_EVENT, 1, &json!({ "items": triage_items, "reading_list": [] }))
        .unwrap();
}

#[allow(clippy::too_many_arguments)]
pub fn ingest_and_record_xe(
    data_dir: &Path,
    ledger: &Ledger,
    sid: &str,
    paper_key: &PaperKey,
    file_name: &str,
    raw_bytes: &[u8],
    line: usize,
    quote: &str,
    subclaim_ids: &[&str],
    text_basis: &str,
) -> (IngestedSource, u64) {
    let sandbox = WorkspaceSandbox::new(data_dir, None);
    let mtx = Mutex::new(Ledger::open(data_dir).unwrap());
    let ing = ingest_bytes(
        &sandbox,
        &mtx,
        &IngestRequest {
            session_id: sid,
            file_name,
            origin: &format!("library:{paper_key}"),
            raw_bytes,
            source_id_hint: Some(file_name),
            section_filter: None,
            title: Some("Synthetic Source Paper"),
        },
    )
    .unwrap();
    drop(mtx);
    ledger
        .append_with_hash(
            sid,
            IDEA_PAPER_EVENT,
            1,
            &json!({
                "key": paper_key,
                "source_id": ing.source_id,
                "status": "read_full",
                "text_basis": text_basis,
                "lines_sent": 10,
                "total_lines": 10,
                "origin": "library",
            }),
        )
        .unwrap();
    let evs = ledger.read_session(sid).unwrap();
    let vq = verify_quote(&sandbox, sid, &evs, &ing.source_id, &ing.files[0], line, line, quote)
        .unwrap();
    let seq = ledger
        .append_with_hash(
            sid,
            EVIDENCE_QUOTE_EVENT,
            1,
            &json!({
                "source_id": vq.quote_ref.source_id,
                "path": vq.quote_ref.path,
                "start_line": vq.quote_ref.start_line,
                "end_line": vq.quote_ref.end_line,
                "file_sha256": vq.quote_ref.file_sha256,
                "quote_sha256": vq.quote_ref.quote_sha256,
                "quote": quote,
                "excerpt": vq.excerpt,
                "source_seq": vq.source_seq,
                "caller": format!("reader:{sid}:{paper_key}"),
                "paper_key": paper_key,
                "role": "states_result",
                "subclaim_ids": subclaim_ids,
                "gist": "Synthetic result quote.",
            }),
        )
        .unwrap()
        .seq;
    (ing, seq)
}

#[allow(clippy::too_many_arguments)]
pub fn append_ocr_pdf_pages_receipt(
    ledger: &Ledger,
    sid: &str,
    source_id: &str,
    path: &str,
    file_sha: &str,
    page_num: usize,
    line_num: usize,
    basis: PageBasis,
) {
    let page_rec = PageRecord {
        page: page_num,
        basis,
        first_line: Some(line_num),
        last_line: Some(line_num),
        metrics: PageQualityMetrics {
            char_count: 80,
            alpha_count: 60,
            word_count: 14,
            image_coverage_pct: 95,
            has_invisible_text: true,
            width_pt: 612,
            height_pt: 792,
        },
        reason: Some("full-page scan image".into()),
    };
    ledger
        .append_with_hash(
            sid,
            "source.ingested",
            1,
            &json!({
                "source_id": source_id,
                "format": "pdf",
                "files": [path],
                "files_sha256": { path: file_sha },
                "pdf_pages": [page_rec],
            }),
        )
        .unwrap();
}

pub fn append_arxiv_abstract_evidence(
    ledger: &Ledger,
    sid: &str,
    arxiv_id: &str,
    title: &str,
    summary: &str,
    quote: &str,
    subclaim_ids: &[&str],
) -> (PaperKey, u64) {
    let key = PaperKey::from_arxiv(arxiv_id).unwrap();
    let blob_bytes = synth_arxiv_atom_blob(arxiv_id, title, summary);
    let blob_hash = ledger.put_blob(&blob_bytes).unwrap();
    let matched = locate_abstract_quote_in_blob(LitProvider::Arxiv, &blob_bytes, &key, quote)
        .unwrap();
    let payload = EvidenceAbstractPayload::new(
        key.clone(),
        matched.matched_text,
        matched.start,
        matched.end,
        blob_hash,
        "arxiv",
        None,
        subclaim_ids.iter().map(|s| (*s).to_string()).collect(),
        Some("Abstract relevance note.".into()),
    )
    .unwrap();
    let seq = ledger
        .append_with_hash(sid, EVIDENCE_ABSTRACT_EVENT, 1, &serde_json::to_value(&payload).unwrap())
        .unwrap()
        .seq;
    (key, seq)
}

pub fn append_map_and_honest_verdict(ledger: &Ledger, sid: &str, overlaps: Vec<Value>) -> (u64, u64) {
    let map_seq = ledger
        .append_with_hash(
            sid,
            EVENT_IDEA_MAP,
            1,
            &json!({ "idea_rev": 1, "nodes": [], "edges": [], "overlaps": overlaps, "dropped": [] }),
        )
        .unwrap()
        .seq;
    let evs = ledger.read_session(sid).unwrap();
    let mut verdict = idea_verdict::compute(&evs, None);
    verdict.map_seq = Some(map_seq);
    let verdict_seq = ledger
        .append_with_hash(sid, IDEA_VERDICT_EVENT, 1, &serde_json::to_value(&verdict).unwrap())
        .unwrap()
        .seq;
    (map_seq, verdict_seq)
}

pub fn append_confirm_same(ledger: &Ledger, sid: &str, subclaim_id: &str) {
    ledger
        .append_with_hash(
            sid,
            IDEA_NOTE_EVENT,
            1,
            &json!({ "target_kind": "verdict", "target_id": subclaim_id, "action": "confirm_same", "comment": "" }),
        )
        .unwrap();
}
