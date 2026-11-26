//! Parsing `literature.fetched` ledger events and raw response blobs during library rebuild.

use crate::arxiv::parse_atom_feed;
use crate::library_schema::{LibraryIds, PaperFlags, PaperRecord, SeenRole};
use crate::literature::parse_hits;
use crate::paper_key::{PaperIds, PaperKey, PaperKeyKind, PaperRef};
use ledger::{Event, Ledger};

pub(crate) fn extract_literature_papers(
    ledger: &Ledger,
    ev: &Event,
) -> Vec<(PaperRecord, SeenRole)> {
    let provider = ev.payload["provider"].as_str().unwrap_or("").trim();
    let blob_hash = ev.payload["raw_blob"].as_str().unwrap_or("").trim();
    let mut out = Vec::new();

    if !blob_hash.is_empty() {
        if let Ok(raw) = ledger.get_blob(blob_hash) {
            if provider == "arxiv" {
                if let Ok(papers) = parse_atom_feed(&raw) {
                    for p in papers {
                        let ids = PaperIds {
                            arxiv: Some(p.base_id.clone()),
                            doi: p.doi.clone(),
                            ..Default::default()
                        };
                        let Ok(pref) = PaperRef::from_ids(ids, None) else {
                            continue;
                        };
                        let year = p.published.get(..4).and_then(|s| s.parse::<i32>().ok());
                        out.push((
                            PaperRecord {
                                key: pref.key,
                                ids: LibraryIds::from_paper_ids(&pref.ids),
                                title: p.title,
                                authors: p.authors,
                                year,
                                abstract_text: p.abstract_text,
                                bibtex: p.bibtex,
                                flags: PaperFlags {
                                    withdrawn: p.withdrawn,
                                    ..Default::default()
                                },
                                first_seen_ms: ev.ts_ms,
                                last_seen_ms: ev.ts_ms,
                            },
                            SeenRole::Candidate,
                        ));
                    }
                    return out;
                }
            } else if matches!(provider, "inspire" | "crossref" | "semantic_scholar") {
                if let Ok(hits) = parse_hits(provider, &raw) {
                    for h in hits {
                        let ids = PaperIds {
                            arxiv: h.arxiv_id.clone(),
                            doi: h.doi.clone(),
                            inspire: (provider == "inspire").then(|| h.id.clone()),
                            s2: (provider == "semantic_scholar").then(|| h.id.clone()),
                        };
                        let Ok(pref) = PaperRef::from_ids(ids, None) else {
                            continue;
                        };
                        out.push((
                            PaperRecord {
                                key: pref.key,
                                ids: LibraryIds::from_paper_ids(&pref.ids),
                                title: h.title,
                                authors: h.authors,
                                year: h.year,
                                abstract_text: String::new(),
                                bibtex: h.bibtex,
                                flags: PaperFlags {
                                    retracted: h.retracted,
                                    ..Default::default()
                                },
                                first_seen_ms: ev.ts_ms,
                                last_seen_ms: ev.ts_ms,
                            },
                            SeenRole::Candidate,
                        ));
                    }
                    return out;
                }
            }
        }
    }

    // Fallback when blob is absent or from another provider (e.g. openalex): inspect payload arrays.
    let Some(items) = ev.payload["papers"]
        .as_array()
        .or_else(|| ev.payload["hits"].as_array())
    else {
        return out;
    };
    for item in items {
        let arxiv = item["base_id"]
            .as_str()
            .or_else(|| item["arxiv_id"].as_str())
            .or_else(|| (provider == "arxiv").then(|| item["id"].as_str()).flatten())
            .map(str::to_string);
        let doi = item["doi"].as_str().map(str::to_string);
        let inspire = (provider == "inspire")
            .then(|| item["id"].as_str().map(str::to_string))
            .flatten();
        let s2 = (provider == "semantic_scholar")
            .then(|| item["id"].as_str().map(str::to_string))
            .flatten();
        let fallback = (provider == "openalex")
            .then(|| item["id"].as_str().and_then(|s| PaperKey::from_openalex(s).ok()))
            .flatten();
        let Ok(pref) = PaperRef::from_ids_or(PaperIds { arxiv, doi, inspire, s2 }, None, fallback)
        else {
            continue;
        };
        let mut ids = LibraryIds::from_paper_ids(&pref.ids);
        if pref.key.kind() == PaperKeyKind::Openalex {
            ids.openalex = Some(pref.key.value().to_string());
        }
        out.push((
            PaperRecord {
                key: pref.key,
                ids,
                title: item["title"].as_str().unwrap_or("").to_string(),
                authors: Vec::new(),
                year: item["year"].as_i64().and_then(|y| i32::try_from(y).ok()),
                abstract_text: item["abstract"].as_str().unwrap_or("").to_string(),
                bibtex: String::new(),
                flags: PaperFlags {
                    withdrawn: item["withdrawn"].as_bool().unwrap_or(false),
                    retracted: item["retracted"].as_bool().unwrap_or(false),
                    ..Default::default()
                },
                first_seen_ms: ev.ts_ms,
                last_seen_ms: ev.ts_ms,
            },
            SeenRole::Candidate,
        ));
    }
    out
}
