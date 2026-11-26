//! Pure aggregation and completeness-gate evaluation for `crate::search_coverage`.

use super::{
    paper_ref_matches_key, CandidateCoverageRecord, CorePaperGap, CoverageGate, CoverageInput,
    ExplorePreset, GateFailure, KnownPaperRecall, PaperReadCounts, PaperReadStatus,
    PerspectiveCoverage, PerspectiveKind, ProviderCoverage, ProviderCoverageState,
    QueryOutcomeStatus, TriageRelevance, UnresolvedCorePaper, MIN_ANSWERED_EXTERNAL_SERVICES,
};
use crate::lit_providers::LitProvider;
use crate::paper_key::{PaperKey, PaperRef};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
struct ProviderAcc {
    queries_ok: usize,
    queries_zero_results: usize,
    queries_failed: usize,
    queries_rate_limited: usize,
    queries_skipped_budget: usize,
    queries_unsupported: usize,
    seed_lookup_queries_ok: usize,
    cached: usize,
    hits: usize,
    saw_not_configured: bool,
    saw_disabled: bool,
    explicit_state: Option<ProviderCoverageState>,
}

pub(super) fn aggregate_providers(input: &CoverageInput) -> Vec<ProviderCoverage> {
    let mut map: BTreeMap<LitProvider, ProviderAcc> = BTreeMap::new();
    for &(prov, st) in &input.provider_states {
        map.entry(prov).or_default().explicit_state = Some(st);
    }
    for q in &input.queries {
        let acc = map.entry(q.provider).or_default();
        match q.status {
            QueryOutcomeStatus::Ok => {
                if q.is_seed_lookup {
                    // Direct seed lookups never satisfy the external-service search minimum (M1).
                    acc.seed_lookup_queries_ok += 1;
                } else {
                    acc.queries_ok += 1;
                    acc.hits += q.hits;
                    if q.hits == 0 {
                        acc.queries_zero_results += 1;
                    }
                }
                if q.cached {
                    acc.cached += 1;
                }
            }
            QueryOutcomeStatus::Failed => acc.queries_failed += 1,
            QueryOutcomeStatus::RateLimited => acc.queries_rate_limited += 1,
            QueryOutcomeStatus::NotConfigured => acc.saw_not_configured = true,
            QueryOutcomeStatus::Disabled => acc.saw_disabled = true,
            QueryOutcomeStatus::Unsupported => acc.queries_unsupported += 1,
            QueryOutcomeStatus::SkippedBudget => acc.queries_skipped_budget += 1,
        }
    }
    map.into_iter()
        .map(|(provider, acc)| ProviderCoverage {
            provider,
            state: derive_provider_state(&acc),
            queries_ok: acc.queries_ok,
            queries_zero_results: acc.queries_zero_results,
            queries_failed: acc.queries_failed,
            queries_rate_limited: acc.queries_rate_limited,
            queries_skipped_budget: acc.queries_skipped_budget,
            queries_unsupported: acc.queries_unsupported,
            seed_lookup_queries_ok: acc.seed_lookup_queries_ok,
            cached: acc.cached,
            hits: acc.hits,
        })
        .collect()
}

fn derive_provider_state(acc: &ProviderAcc) -> ProviderCoverageState {
    if acc.queries_ok > 0 {
        if acc.queries_failed > 0
            || acc.queries_rate_limited > 0
            || acc.explicit_state == Some(ProviderCoverageState::Partial)
        {
            ProviderCoverageState::Partial
        } else {
            ProviderCoverageState::Ok
        }
    } else if acc.queries_rate_limited > 0 {
        ProviderCoverageState::RateLimited
    } else if acc.queries_failed > 0 {
        ProviderCoverageState::Failed
    } else if acc.saw_not_configured {
        ProviderCoverageState::NotConfigured
    } else if acc.saw_disabled {
        ProviderCoverageState::Disabled
    } else if let Some(explicit) = acc.explicit_state {
        // With 0 answered queries, an explicit Partial state cannot be treated as answered.
        if explicit == ProviderCoverageState::Partial {
            ProviderCoverageState::Failed
        } else {
            explicit
        }
    } else {
        ProviderCoverageState::NotAttempted
    }
}

pub(super) fn aggregate_perspectives(input: &CoverageInput) -> Vec<PerspectiveCoverage> {
    let mut map: BTreeMap<String, PerspectiveCoverage> = BTreeMap::new();
    for (id, kind) in &input.perspectives {
        map.entry(id.clone()).or_insert_with(|| PerspectiveCoverage {
            id: id.clone(),
            kind: *kind,
            queries_planned: 0,
            queries_ok: 0,
            queries_zero_results: 0,
            queries_failed: 0,
            hits: 0,
        });
    }
    for q in &input.queries {
        // Direct seed lookups never count toward perspective coverage (M1).
        if q.is_seed_lookup {
            continue;
        }
        let key_and_kind = match (&q.perspective_id, q.perspective_kind) {
            (Some(id), Some(k)) => Some((id.clone(), k)),
            (Some(id), None) => map.get(id).map(|p| (id.clone(), p.kind)),
            (None, Some(k)) => Some((format!("kind:{}", k.as_str()), k)),
            (None, None) => None,
        };
        if let Some((pid, kind)) = key_and_kind {
            let entry = map.entry(pid.clone()).or_insert_with(|| PerspectiveCoverage {
                id: pid,
                kind,
                queries_planned: 0,
                queries_ok: 0,
                queries_zero_results: 0,
                queries_failed: 0,
                hits: 0,
            });
            entry.queries_planned += 1;
            if q.status.is_answered() {
                entry.queries_ok += 1;
                entry.hits += q.hits;
                if q.hits == 0 {
                    entry.queries_zero_results += 1;
                }
            } else if q.status.is_attempt_failure() {
                entry.queries_failed += 1;
            }
        }
    }
    map.into_values().collect()
}

pub(super) fn summarize_candidates(
    candidates: &[CandidateCoverageRecord],
) -> (usize, Option<[i32; 2]>) {
    let mut seen: BTreeSet<&PaperKey> = BTreeSet::new();
    let mut min_y: Option<i32> = None;
    let mut max_y: Option<i32> = None;
    for c in candidates {
        seen.insert(&c.paper.key);
        if let Some(y) = c.year {
            min_y = Some(min_y.map_or(y, |m| m.min(y)));
            max_y = Some(max_y.map_or(y, |m| m.max(y)));
        }
    }
    (seen.len(), min_y.zip(max_y).map(|(a, b)| [a, b]))
}

pub(super) struct ReadAndTriageSummary {
    pub papers: PaperReadCounts,
    pub unresolved_core: Vec<UnresolvedCorePaper>,
    pub unavailable_core: Vec<PaperKey>,
    pub dismissed_core: Vec<PaperKey>,
    pub triaged_count: usize,
}

pub(super) fn summarize_reads_and_triage(input: &CoverageInput) -> ReadAndTriageSummary {
    let mut latest_read: BTreeMap<PaperKey, PaperReadStatus> = BTreeMap::new();
    for r in &input.paper_reads {
        latest_read.insert(r.key.clone(), r.status);
    }
    let mut papers = PaperReadCounts::default();
    for &st in latest_read.values() {
        match st {
            PaperReadStatus::ReadFull => papers.read_full += 1,
            PaperReadStatus::ReadPassages => papers.read_passages += 1,
            PaperReadStatus::NothingRelevant => papers.nothing_relevant += 1,
            PaperReadStatus::Unavailable => papers.unavailable += 1,
            PaperReadStatus::Failed => papers.failed += 1,
        }
    }

    let mut triage_map: BTreeMap<PaperKey, TriageRelevance> = BTreeMap::new();
    for t in &input.triage {
        triage_map.insert(t.key.clone(), t.relevance);
    }
    let core_keys: BTreeSet<PaperKey> = triage_map
        .iter()
        .filter_map(|(k, r)| (*r == TriageRelevance::Core).then_some(k.clone()))
        .collect();

    let mut unresolved_core = Vec::new();
    let mut unavailable_core = Vec::new();
    let mut dismissed_core = Vec::new();
    for key in &core_keys {
        if latest_read.get(key).is_some_and(|st| st.has_full_text_read()) {
            continue;
        }
        if input.dismissed_core.contains(key) {
            dismissed_core.push(key.clone());
            continue;
        }
        let reason = match latest_read.get(key).copied() {
            Some(PaperReadStatus::Unavailable) => {
                unavailable_core.push(key.clone());
                CorePaperGap::Unavailable
            }
            Some(PaperReadStatus::Failed) => CorePaperGap::Failed,
            _ => CorePaperGap::Unread,
        };
        unresolved_core.push(UnresolvedCorePaper { key: key.clone(), reason });
    }
    ReadAndTriageSummary {
        papers,
        unresolved_core,
        unavailable_core,
        dismissed_core,
        triaged_count: triage_map.len(),
    }
}

pub(super) fn compute_seed_recall(input: &CoverageInput) -> KnownPaperRecall {
    let mut given_set: BTreeSet<PaperKey> = BTreeSet::new();
    let mut given = Vec::new();
    for s in &input.user_seeds {
        if given_set.insert(s.clone()) {
            given.push(s.clone());
        }
    }
    if given.is_empty() {
        return KnownPaperRecall::default();
    }

    let answered_search_qids: BTreeSet<&str> = input
        .queries
        .iter()
        .filter(|q| q.status.is_answered() && !q.is_seed_lookup)
        .map(|q| q.qid.as_str())
        .collect();

    let mut search_found_refs: Vec<&PaperRef> = Vec::new();
    for q in &input.queries {
        if q.status.is_answered() && !q.is_seed_lookup {
            for p in &q.found_papers {
                search_found_refs.push(p);
            }
        }
    }
    for c in &input.candidates {
        if c.found_by.iter().any(|qid| answered_search_qids.contains(qid.as_str())) {
            search_found_refs.push(&c.paper);
        }
    }

    let mut found_by_search = Vec::new();
    let mut missed = Vec::new();
    for seed in &given {
        if search_found_refs.iter().any(|p| paper_ref_matches_key(p, seed)) {
            found_by_search.push(seed.clone());
        } else {
            missed.push(seed.clone());
        }
    }
    KnownPaperRecall { given, found_by_search, missed }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn evaluate_gate(
    preset: ExplorePreset,
    providers: &[ProviderCoverage],
    perspectives: &[PerspectiveCoverage],
    candidates_total: usize,
    screened: usize,
    screened_required: usize,
    unresolved_core: &[UnresolvedCorePaper],
    recall: &KnownPaperRecall,
) -> CoverageGate {
    let mut unsatisfied = Vec::new();

    let preset_allows_none_in_scope = preset.allows_none_in_scope();
    if !preset_allows_none_in_scope {
        unsatisfied.push(GateFailure::QuickPresetNoFullText);
    }

    let answered_providers: Vec<LitProvider> = providers
        .iter()
        .filter(|p| p.answered())
        .map(|p| p.provider)
        .collect();
    let answered_external_services = answered_providers.len();
    let services_minimum_met = answered_external_services >= MIN_ANSWERED_EXTERNAL_SERVICES;
    if !services_minimum_met {
        unsatisfied.push(GateFailure::TooFewAnsweredServices {
            answered: answered_external_services,
            required: MIN_ANSWERED_EXTERNAL_SERVICES,
            answered_providers,
        });
    }

    let mut no_attempted_service_failed = true;
    for p in providers {
        let explicit_partial_only = p.state == ProviderCoverageState::Partial
            && p.queries_failed == 0
            && p.queries_rate_limited == 0;
        if p.queries_failed > 0
            || p.state == ProviderCoverageState::Failed
            || explicit_partial_only
        {
            no_attempted_service_failed = false;
            unsatisfied.push(GateFailure::AttemptedServiceFailed {
                provider: p.provider,
                queries_failed: p.queries_failed.max(1),
            });
        }
        if p.queries_rate_limited > 0 || p.state == ProviderCoverageState::RateLimited {
            no_attempted_service_failed = false;
            unsatisfied.push(GateFailure::AttemptedServiceRateLimited {
                provider: p.provider,
                queries_rate_limited: p.queries_rate_limited.max(1),
            });
        }
    }

    let exact_perspective_answered = perspectives
        .iter()
        .any(|p| p.kind == PerspectiveKind::Exact && p.answered());
    if !exact_perspective_answered {
        unsatisfied.push(GateFailure::MissingExactPerspective);
    }

    let other_names_perspective_answered = perspectives
        .iter()
        .any(|p| p.kind == PerspectiveKind::OtherNames && p.answered());
    if !other_names_perspective_answered {
        unsatisfied.push(GateFailure::MissingOtherNamesPerspective);
    }

    let enough_candidates_screened = screened >= screened_required;
    if !enough_candidates_screened {
        unsatisfied.push(GateFailure::InsufficientScreenedCandidates {
            screened,
            required: screened_required,
            candidates_total,
        });
    }

    let all_core_papers_resolved = unresolved_core.is_empty();
    if !all_core_papers_resolved {
        unsatisfied.push(GateFailure::UnresolvedCorePapers {
            papers: unresolved_core.to_vec(),
        });
    }

    let all_seeds_found = recall.missed.is_empty();
    if !all_seeds_found {
        unsatisfied.push(GateFailure::MissedUserSeeds {
            missed: recall.missed.clone(),
        });
    }

    CoverageGate {
        passed: unsatisfied.is_empty(),
        preset_allows_none_in_scope,
        answered_external_services,
        min_external_services: MIN_ANSWERED_EXTERNAL_SERVICES,
        services_minimum_met,
        no_attempted_service_failed,
        exact_perspective_answered,
        other_names_perspective_answered,
        enough_candidates_screened,
        all_core_papers_resolved,
        all_seeds_found,
        unsatisfied,
    }
}
