//! Ledger-event extractor for `crate::search_coverage` (`explore.md` 4.2, rules V7 & V10).
//!
//! Reads events tolerantly through `serde_json::Value` and explicitly ignores `lessons.supplied`
//! and F16 ranking marks (`not_relevant`, `pin`, `key_paper`, `wrong`, `comment`).

use super::{
    CandidateCoverageRecord, CoverageInput, ExplorePreset, LibraryCoverage,
    PaperReadCoverageRecord, PaperReadStatus, PerspectiveKind, ProviderCoverageState,
    QueryCoverageRecord, QueryOutcomeStatus, TriageCoverageRecord, TriageRelevance,
    IDEA_BRIEF_EVENT, IDEA_CANDIDATES_EVENT, IDEA_CITATION_NET_EVENT, IDEA_CORE_DISMISSED_EVENT,
    IDEA_NOTE_EVENT, IDEA_PAPER_EVENT, IDEA_SEARCH_FINISHED_EVENT, IDEA_SEARCH_PLANNED_EVENT,
    IDEA_STARTED_EVENT, IDEA_STATED_EVENT, IDEA_TRIAGE_EVENT, NOTE_ACTION_DISMISS_CORE,
    NOTE_ACTION_UNDISMISS_CORE,
};
use crate::lit_providers::{LitProvider, LITERATURE_FETCHED_EVENT};
use crate::paper_key::{PaperIds, PaperKey, PaperRef};
use ledger::Event;
use serde_json::Value;
use std::collections::BTreeMap;

type PlannedQueryMeta = (LitProvider, Option<String>, Option<PerspectiveKind>, bool);

pub(super) fn extract_coverage_input(events: &[Event]) -> CoverageInput {
    let mut input = CoverageInput::default();
    let mut planned: BTreeMap<String, PlannedQueryMeta> = BTreeMap::new();
    let mut finished: BTreeMap<String, QueryCoverageRecord> = BTreeMap::new();
    let mut candidates_map: BTreeMap<PaperKey, CandidateCoverageRecord> = BTreeMap::new();
    let mut triage_map: BTreeMap<PaperKey, TriageRelevance> = BTreeMap::new();
    let mut perspective_kinds: BTreeMap<String, PerspectiveKind> = BTreeMap::new();
    let mut seen_brief = false;

    for ev in events {
        match ev.kind.as_str() {
            IDEA_STARTED_EVENT => parse_started(&ev.payload, &mut input),
            IDEA_STATED_EVENT => parse_stated(&ev.payload, &mut input),
            IDEA_BRIEF_EVENT => {
                parse_brief(
                    &ev.payload,
                    &mut perspective_kinds,
                    &mut seen_brief,
                    &mut planned,
                    &mut finished,
                );
            }
            IDEA_SEARCH_PLANNED_EVENT => parse_planned(&ev.payload, &mut planned),
            IDEA_SEARCH_FINISHED_EVENT => {
                parse_finished(&ev.payload, &planned, &perspective_kinds, &mut finished, &mut input);
            }
            LITERATURE_FETCHED_EVENT => {
                parse_lit_fetched(&ev.payload, &planned, &perspective_kinds, &mut finished);
            }
            IDEA_CANDIDATES_EVENT => parse_candidates(&ev.payload, &mut candidates_map),
            IDEA_CITATION_NET_EVENT => parse_citation_net(&ev.payload, &mut input),
            IDEA_TRIAGE_EVENT => parse_triage(&ev.payload, &mut triage_map),
            IDEA_PAPER_EVENT => parse_paper(&ev.payload, &mut input),
            IDEA_CORE_DISMISSED_EVENT => parse_core_dismissed(&ev.payload, &mut input),
            IDEA_NOTE_EVENT => parse_note_dismissal(&ev.payload, &mut input),
            _ => {}
        }
    }

    input.perspectives = perspective_kinds.into_iter().collect();
    input.queries = finished.into_values().collect();
    input.candidates = candidates_map.into_values().collect();
    input.triage = triage_map
        .into_iter()
        .map(|(key, relevance)| TriageCoverageRecord { key, relevance })
        .collect();
    input
}

fn parse_started(p: &Value, input: &mut CoverageInput) {
    if let Some(preset) = p.get("preset").and_then(Value::as_str).and_then(ExplorePreset::parse) {
        input.preset = preset;
    }
    // Reject max_screened == 0 so a malformed budget cannot bypass candidate screening (M4).
    if let Some(ms) = p
        .get("budget")
        .and_then(|b| b.get("max_screened"))
        .and_then(Value::as_u64)
        .filter(|&v| v >= 1)
    {
        input.screened_limit_override = Some(ms as usize);
    }
    parse_provider_states_field(p, input);
    parse_library_field(p, input);
}

fn parse_stated(p: &Value, input: &mut CoverageInput) {
    if let Some(arr) = p.get("seeds").and_then(Value::as_array) {
        input.user_seeds = arr
            .iter()
            .filter_map(Value::as_str)
            .filter_map(parse_seed_key)
            .collect();
    }
}

fn parse_seed_key(raw: &str) -> Option<PaperKey> {
    PaperKey::parse(raw)
        .or_else(|_| PaperKey::from_arxiv(raw))
        .or_else(|_| PaperKey::from_doi(raw))
        .or_else(|_| PaperKey::from_inspire(raw))
        .or_else(|_| PaperKey::from_s2(raw))
        .or_else(|_| PaperKey::from_openalex(raw))
        .ok()
}

fn parse_brief(
    p: &Value,
    kinds: &mut BTreeMap<String, PerspectiveKind>,
    seen_brief: &mut bool,
    planned: &mut BTreeMap<String, PlannedQueryMeta>,
    finished: &mut BTreeMap<String, QueryCoverageRecord>,
) {
    let Some(arr) = p.get("perspectives").and_then(Value::as_array) else { return };
    if *seen_brief {
        // Re-sharpening replaced perspectives: prior-brief planned & finished perspective bindings
        // must not satisfy the revised brief's exact/other_names perspective gate (M4, R3c).
        for meta in planned.values_mut() {
            meta.1 = None;
            meta.2 = None;
        }
        for rec in finished.values_mut() {
            rec.perspective_id = None;
            rec.perspective_kind = None;
        }
    }
    *seen_brief = true;
    kinds.clear();
    for item in arr {
        if let (Some(id), Some(kind)) = (
            item.get("id").and_then(Value::as_str),
            item.get("kind").and_then(Value::as_str).and_then(PerspectiveKind::parse),
        ) {
            kinds.insert(id.to_string(), kind);
        }
    }
}

fn parse_planned(p: &Value, planned: &mut BTreeMap<String, PlannedQueryMeta>) {
    let Some(arr) = p.get("queries").and_then(Value::as_array) else { return };
    for q in arr {
        let Some(qid) = q.get("qid").and_then(Value::as_str) else { continue };
        let Some(prov) = q.get("provider").and_then(Value::as_str).and_then(LitProvider::parse)
        else {
            continue;
        };
        let pid = q.get("perspective_id").and_then(Value::as_str).map(str::to_string);
        let pkind = q
            .get("perspective_kind")
            .and_then(Value::as_str)
            .and_then(PerspectiveKind::parse);
        let explicit_seed = q.get("seed_lookup").and_then(Value::as_bool).unwrap_or(false);
        let origin_app = q.get("origin").and_then(Value::as_str) == Some("app");
        let relation_none = match q.get("relation") {
            None | Some(Value::Null) => true,
            Some(Value::String(s)) => s.eq_ignore_ascii_case("none"),
            Some(Value::Object(m)) => m.get("kind").and_then(Value::as_str) == Some("none"),
            _ => false,
        };
        planned.insert(
            qid.to_string(),
            (prov, pid, pkind, explicit_seed || (origin_app && relation_none)),
        );
    }
}

fn parse_finished(
    p: &Value,
    planned: &BTreeMap<String, PlannedQueryMeta>,
    kinds: &BTreeMap<String, PerspectiveKind>,
    finished: &mut BTreeMap<String, QueryCoverageRecord>,
    input: &mut CoverageInput,
) {
    parse_provider_states_field(p, input);
    parse_library_field(p, input);
    let Some(arr) = p.get("queries").and_then(Value::as_array) else { return };
    for q in arr {
        let Some(qid) = q.get("qid").and_then(Value::as_str) else { continue };
        let Some(status) = q.get("status").and_then(Value::as_str).and_then(QueryOutcomeStatus::parse)
        else {
            continue;
        };
        let plan = planned.get(qid);
        let prev = finished.get(qid);
        let provider = q
            .get("provider")
            .and_then(Value::as_str)
            .and_then(LitProvider::parse)
            .or_else(|| plan.map(|t| t.0))
            .or_else(|| prev.map(|r| r.provider));
        let Some(provider) = provider else { continue };
        let pid = q
            .get("perspective_id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| plan.and_then(|t| t.1.clone()))
            .or_else(|| prev.and_then(|r| r.perspective_id.clone()));
        let pkind = q
            .get("perspective_kind")
            .and_then(Value::as_str)
            .and_then(PerspectiveKind::parse)
            .or_else(|| plan.and_then(|t| t.2))
            .or_else(|| pid.as_deref().and_then(|id| kinds.get(id).copied()))
            .or_else(|| prev.and_then(|r| r.perspective_kind));
        // Preserve hits/cached recorded by an earlier `literature.fetched` event (H1, R3c).
        let explicit_hits = q
            .get("hits")
            .or_else(|| q.get("count"))
            .and_then(Value::as_u64)
            .map(|n| n as usize);
        let hits = match (explicit_hits, prev) {
            (Some(0), Some(r)) if status.is_answered() && r.hits > 0 => r.hits,
            (Some(h), _) => h,
            (None, Some(r)) => r.hits,
            (None, None) => 0,
        };
        let cached = q.get("cached").and_then(Value::as_bool).unwrap_or(false)
            || prev.is_some_and(|r| r.cached);
        let is_seed_lookup = q
            .get("seed_lookup")
            .and_then(Value::as_bool)
            .unwrap_or_else(|| plan.is_some_and(|t| t.3) || prev.is_some_and(|r| r.is_seed_lookup));
        let existing_papers = prev.map(|r| r.found_papers.clone()).unwrap_or_default();
        finished.insert(
            qid.to_string(),
            QueryCoverageRecord {
                qid: qid.to_string(),
                provider,
                perspective_id: pid,
                perspective_kind: pkind,
                status,
                hits,
                cached,
                is_seed_lookup,
                found_papers: existing_papers,
            },
        );
    }
}

fn parse_lit_fetched(
    p: &Value,
    planned: &BTreeMap<String, PlannedQueryMeta>,
    kinds: &BTreeMap<String, PerspectiveKind>,
    finished: &mut BTreeMap<String, QueryCoverageRecord>,
) {
    let Some(provider) = p.get("provider").and_then(Value::as_str).and_then(LitProvider::parse)
    else {
        return;
    };
    let Some(qid) = p.get("qid").and_then(Value::as_str) else { return };
    let count = p.get("count").and_then(Value::as_u64).unwrap_or(0) as usize;
    let cached = p.get("cached_from_ms").and_then(Value::as_u64).is_some();
    let found_papers = extract_lit_fetched_refs(provider, p);

    if let Some(rec) = finished.get_mut(qid) {
        if rec.status.is_answered() && rec.hits == 0 && count > 0 {
            rec.hits = count;
        }
        rec.cached |= cached;
        rec.found_papers.extend(found_papers);
    } else {
        let plan = planned.get(qid);
        let pid = plan.and_then(|t| t.1.clone());
        let pkind = plan
            .and_then(|t| t.2)
            .or_else(|| pid.as_deref().and_then(|id| kinds.get(id).copied()));
        let is_seed_lookup = plan.is_some_and(|t| t.3);
        finished.insert(
            qid.to_string(),
            QueryCoverageRecord {
                qid: qid.to_string(),
                provider,
                perspective_id: pid,
                perspective_kind: pkind,
                status: QueryOutcomeStatus::Ok,
                hits: count,
                cached,
                is_seed_lookup,
                found_papers,
            },
        );
    }
}

pub(crate) fn extract_lit_fetched_refs(provider: LitProvider, p: &Value) -> Vec<PaperRef> {
    let items = p.get("papers").or_else(|| p.get("hits")).and_then(Value::as_array);
    let Some(arr) = items else { return Vec::new() };
    arr.iter()
        .filter_map(|item| {
            let fallback_key = item
                .get("key")
                .and_then(Value::as_str)
                .and_then(|s| PaperKey::parse(s).ok());
            let prov_id = item.get("id").and_then(Value::as_str).map(str::to_string);
            let raw_ids = PaperIds {
                arxiv: item
                    .get("base_id")
                    .or_else(|| item.get("arxiv_id"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                doi: item.get("doi").and_then(Value::as_str).map(str::to_string),
                inspire: item
                    .get("inspire")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| (provider == LitProvider::Inspire).then(|| prov_id.clone()).flatten()),
                s2: item
                    .get("s2")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| {
                        (provider == LitProvider::SemanticScholar).then(|| prov_id.clone()).flatten()
                    }),
            };
            let normalized = normalize_paper_ids(&raw_ids);
            if let Ok(pref) = PaperRef::from_ids_or(normalized.clone(), None, fallback_key.clone()) {
                return Some(pref);
            }
            fallback_key.map(|key| PaperRef { key, version: None, ids: normalized })
        })
        .collect()
}

pub(crate) fn normalize_paper_ids(ids: &PaperIds) -> PaperIds {
    PaperIds {
        arxiv: ids
            .arxiv
            .as_deref()
            .and_then(|s| PaperKey::from_arxiv(s).ok())
            .map(|k| k.value().to_string()),
        doi: ids
            .doi
            .as_deref()
            .and_then(|s| PaperKey::from_doi(s).ok())
            .map(|k| k.value().to_string()),
        inspire: ids
            .inspire
            .as_deref()
            .and_then(|s| PaperKey::from_inspire(s).ok())
            .map(|k| k.value().to_string()),
        s2: ids
            .s2
            .as_deref()
            .and_then(|s| PaperKey::from_s2(s).ok())
            .map(|k| k.value().to_string()),
    }
}

pub(crate) fn merge_paper_ids(dst: &mut PaperIds, src: &PaperIds) {
    if dst.arxiv.is_none() {
        dst.arxiv = src.arxiv.clone();
    }
    if dst.doi.is_none() {
        dst.doi = src.doi.clone();
    }
    if dst.inspire.is_none() {
        dst.inspire = src.inspire.clone();
    }
    if dst.s2.is_none() {
        dst.s2 = src.s2.clone();
    }
}

fn parse_candidates(p: &Value, map: &mut BTreeMap<PaperKey, CandidateCoverageRecord>) {
    let Some(arr) = p.get("candidates").and_then(Value::as_array) else { return };
    for c in arr {
        let Some(key) = c.get("key").and_then(Value::as_str).and_then(|s| PaperKey::parse(s).ok())
        else {
            continue;
        };
        let raw_ids: PaperIds = c
            .get("ids")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let ids = normalize_paper_ids(&raw_ids);
        let year = c.get("year").and_then(Value::as_i64).map(|y| y as i32);
        let found_by: Vec<String> = c
            .get("found_by")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        let user_seed = c.get("user_seed").and_then(Value::as_bool).unwrap_or(false);
        let entry = map.entry(key.clone()).or_insert_with(|| CandidateCoverageRecord {
            paper: PaperRef { key, version: None, ids: ids.clone() },
            year,
            found_by: Vec::new(),
            user_seed,
        });
        merge_paper_ids(&mut entry.paper.ids, &ids);
        entry.user_seed |= user_seed;
        if entry.year.is_none() {
            entry.year = year;
        }
        for qid in found_by {
            if !entry.found_by.contains(&qid) {
                entry.found_by.push(qid);
            }
        }
    }
}

fn parse_citation_net(p: &Value, input: &mut CoverageInput) {
    let seeds = p
        .get("seeds")
        .and_then(Value::as_array)
        .map(Vec::len)
        .or_else(|| p.get("seeds").and_then(Value::as_u64).map(|n| n as usize))
        .unwrap_or(0);
    input.citation_expansion.seeds = input.citation_expansion.seeds.max(seeds);
    let default_hops = usize::from(seeds > 0);
    let hops = p
        .get("hops")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(default_hops);
    input.citation_expansion.hops = input.citation_expansion.hops.max(hops);
}

fn parse_triage(p: &Value, triage_map: &mut BTreeMap<PaperKey, TriageRelevance>) {
    // Merge incremental rounds instead of clearing Round 1 core papers (M4).
    if let Some(arr) = p.get("items").and_then(Value::as_array) {
        for item in arr {
            if let (Some(key), Some(rel)) = (
                item.get("key").and_then(Value::as_str).and_then(|s| PaperKey::parse(s).ok()),
                item.get("relevance").and_then(Value::as_str).and_then(TriageRelevance::parse),
            ) {
                triage_map.insert(key, rel);
            }
        }
    }
}

fn parse_paper(p: &Value, input: &mut CoverageInput) {
    if let (Some(key), Some(status)) = (
        p.get("key").and_then(Value::as_str).and_then(|s| PaperKey::parse(s).ok()),
        p.get("status").and_then(Value::as_str).and_then(PaperReadStatus::parse),
    ) {
        input.paper_reads.push(PaperReadCoverageRecord { key, status });
    }
}

fn parse_core_dismissed(p: &Value, input: &mut CoverageInput) {
    if let Some(key) = p
        .get("paper_key")
        .or_else(|| p.get("key"))
        .and_then(Value::as_str)
        .and_then(|s| PaperKey::parse(s).ok())
    {
        if p.get("dismissed").and_then(Value::as_bool) == Some(false) {
            input.dismissed_core.remove(&key);
        } else {
            input.dismissed_core.insert(key);
        }
    }
}

fn parse_note_dismissal(p: &Value, input: &mut CoverageInput) {
    let Some(action) = p.get("action").and_then(Value::as_str) else { return };
    if action != NOTE_ACTION_DISMISS_CORE && action != NOTE_ACTION_UNDISMISS_CORE {
        return;
    }
    if let Some(key) = p
        .get("paper_key")
        .or_else(|| p.get("target"))
        .and_then(Value::as_str)
        .and_then(|s| PaperKey::parse(s).ok())
    {
        if action == NOTE_ACTION_DISMISS_CORE {
            input.dismissed_core.insert(key);
        } else {
            input.dismissed_core.remove(&key);
        }
    }
}

fn parse_provider_states_field(p: &Value, input: &mut CoverageInput) {
    let Some(arr) = p.get("providers").and_then(Value::as_array) else { return };
    for item in arr {
        if let (Some(prov), Some(st)) = (
            item.get("provider")
                .or_else(|| item.get("name"))
                .and_then(Value::as_str)
                .and_then(LitProvider::parse),
            item.get("state")
                .and_then(Value::as_str)
                .and_then(ProviderCoverageState::parse),
        ) {
            input.provider_states.push((prov, st));
        }
    }
}

fn parse_library_field(p: &Value, input: &mut CoverageInput) {
    let Some(lib) = p.get("library").and_then(Value::as_object) else { return };
    input.library = LibraryCoverage {
        searched: lib.get("searched").and_then(Value::as_bool).unwrap_or(true),
        papers: lib.get("papers").and_then(Value::as_u64).unwrap_or(0) as usize,
        with_full_text: lib.get("with_full_text").and_then(Value::as_u64).unwrap_or(0) as usize,
        hits: lib.get("hits").and_then(Value::as_u64).unwrap_or(0) as usize,
    };
}
