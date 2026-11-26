//! Overlap validation and per-subclaim verdict evaluation for `crate::prior_work`.

use super::{
    is_origin_paper, quote_contains_term, AbstractEvidence, AbstractMatch, AcceptedOverlap,
    EvidenceId, FullTextEvidence, OverlapCandidate, OverlapRejectionReason, OverlapRelation,
    PriorWorkInput, PriorWorkStatus, PriorWorkVerdict, RejectedOverlap, TextBasis,
    MAX_DIFFERENCE_CHARS,
};
use crate::prior_work_wording::{check_model_prose_phrases, format_verdict_sentence, status_chip};
use crate::quote_core::clip_quote;
use crate::search_coverage::PaperReadStatus;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn evaluate_one_subclaim(
    subclaim_id: &str,
    input: &PriorWorkInput,
    ft_by_id: &BTreeMap<EvidenceId, &FullTextEvidence>,
    abs_by_id: &BTreeMap<EvidenceId, &AbstractEvidence>,
) -> PriorWorkVerdict {
    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    let mut seen_abs: BTreeSet<EvidenceId> = BTreeSet::new();
    let mut abstract_matches = Vec::new();

    for ev in abs_by_id.values() {
        if is_origin_paper(input.origin_paper.as_ref(), &ev.paper_key) {
            continue;
        }
        if ev.subclaim_ids.iter().any(|s| s == subclaim_id) && seen_abs.insert(ev.id) {
            abstract_matches.push(AbstractMatch {
                paper_key: ev.paper_key.clone(),
                evidence_id: ev.id,
                quote: clip_quote(ev.quote.trim()).to_string(),
                provider: ev.provider,
            });
        }
    }

    for cand in input.overlaps.iter().filter(|o| o.subclaim_id == subclaim_id) {
        if is_origin_paper(input.origin_paper.as_ref(), &cand.paper_key) {
            rejected.push(RejectedOverlap {
                subclaim_id: cand.subclaim_id.clone(),
                paper_key: cand.paper_key.clone(),
                relation: cand.relation,
                reason: OverlapRejectionReason::OriginPaperExcluded,
            });
            continue;
        }

        // Any valid verbatim `xa-*` on the same paper still caps at AbstractOnly if full-text fails.
        for eid in &cand.evidence_ids {
            if let Some(aev) = abs_by_id.get(eid) {
                if aev.paper_key == cand.paper_key
                    && !is_origin_paper(input.origin_paper.as_ref(), &aev.paper_key)
                    && seen_abs.insert(aev.id)
                {
                    abstract_matches.push(AbstractMatch {
                        paper_key: aev.paper_key.clone(),
                        evidence_id: aev.id,
                        quote: clip_quote(aev.quote.trim()).to_string(),
                        provider: aev.provider,
                    });
                }
            }
        }

        match validate_overlap(cand, input, ft_by_id, abs_by_id) {
            Ok(ok) => accepted.push(ok),
            Err(reason) => rejected.push(RejectedOverlap {
                subclaim_id: cand.subclaim_id.clone(),
                paper_key: cand.paper_key.clone(),
                relation: cand.relation,
                reason,
            }),
        }
    }

    let status = if let Some(best) = accepted.iter().max_by_key(|o| o.relation.priority()) {
        best.relation.to_status()
    } else if !abstract_matches.is_empty() {
        PriorWorkStatus::AbstractOnly
    } else if input.coverage.gate.passed {
        PriorWorkStatus::NoneInScope
    } else {
        PriorWorkStatus::Incomplete
    };

    let confirmed_by_user = status.is_judged() && input.confirmed_subclaims.contains(subclaim_id);
    // Scope top-level OCR flag to the primary overlaps supporting `status` (M2).
    let has_ocr_evidence = accepted
        .iter()
        .filter(|o| o.relation.to_status() == status)
        .any(|o| o.ocr_sourced);
    let display_grade = status.display_grade(confirmed_by_user, has_ocr_evidence);
    let border_style = status.border_style(confirmed_by_user);
    let chip = status_chip(status, confirmed_by_user).to_string();
    let text = format_verdict_sentence(
        status,
        confirmed_by_user,
        &accepted,
        &abstract_matches,
        &input.coverage,
    );

    PriorWorkVerdict {
        subclaim_id: subclaim_id.to_string(),
        status,
        overlaps: accepted,
        abstract_matches,
        rejected_overlaps: rejected,
        confirmed_by_user,
        has_ocr_evidence,
        display_grade,
        border_style,
        chip,
        text,
    }
}

fn validate_overlap(
    cand: &OverlapCandidate,
    input: &PriorWorkInput,
    ft_by_id: &BTreeMap<EvidenceId, &FullTextEvidence>,
    abs_by_id: &BTreeMap<EvidenceId, &AbstractEvidence>,
) -> Result<AcceptedOverlap, OverlapRejectionReason> {
    let read_status = input.paper_reads.get(&cand.paper_key).copied();
    if !read_status.is_some_and(PaperReadStatus::can_support_judged_overlap) {
        return Err(OverlapRejectionReason::PaperNotRead { status: read_status });
    }

    let trimmed_diff = cand.difference.trim();
    let diff_chars = trimmed_diff.chars().count();
    if diff_chars > MAX_DIFFERENCE_CHARS {
        return Err(OverlapRejectionReason::DifferenceTooLong {
            chars: diff_chars,
            max: MAX_DIFFERENCE_CHARS,
        });
    }
    if let Err(hits) = check_model_prose_phrases(trimmed_diff) {
        return Err(OverlapRejectionReason::BannedProsePhrase {
            phrases: hits.into_iter().map(str::to_string).collect(),
        });
    }

    let mut valid_ids = Vec::new();
    let mut valid_evs = Vec::new();
    let mut seen_ids = BTreeSet::new();
    for &eid in &cand.evidence_ids {
        if !eid.is_valid() {
            return Err(OverlapRejectionReason::UnknownEvidenceId { evidence_id: eid });
        }
        if eid.is_abstract() {
            // Reject dangling/non-verbatim or cross-paper abstract IDs instead of skipping (R1).
            let Some(aev) = abs_by_id.get(&eid) else {
                return Err(OverlapRejectionReason::UnknownEvidenceId { evidence_id: eid });
            };
            if aev.paper_key != cand.paper_key {
                return Err(OverlapRejectionReason::EvidencePaperMismatch { evidence_id: eid });
            }
            continue;
        }
        // Reject dangling or empty full-text evidence IDs instead of silently skipping them (M6).
        let Some(ev) = ft_by_id.get(&eid) else {
            return Err(OverlapRejectionReason::UnknownEvidenceId { evidence_id: eid });
        };
        if ev.paper_key != cand.paper_key {
            return Err(OverlapRejectionReason::EvidencePaperMismatch { evidence_id: eid });
        }
        if seen_ids.insert(eid) {
            valid_ids.push(eid);
            valid_evs.push(*ev);
        }
    }
    if valid_evs.is_empty() {
        return Err(OverlapRejectionReason::NoValidFullTextEvidence);
    }

    let other_name = if cand.relation == OverlapRelation::OtherName {
        let term = cand
            .other_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or(OverlapRejectionReason::OtherNameMissing)?;
        let term_in_full_text = valid_evs
            .iter()
            .any(|ev| quote_contains_term(&ev.quote, term) || quote_contains_term(&ev.excerpt, term));
        if !term_in_full_text {
            return Err(OverlapRejectionReason::OtherNameNotInFullTextQuote {
                term: term.to_string(),
            });
        }
        Some(term.to_string())
    } else {
        None
    };

    let paper_basis = input.paper_text_basis.get(&cand.paper_key).copied();
    let ocr_sourced = paper_basis.is_some_and(TextBasis::is_ocr_source)
        || valid_evs.iter().any(|ev| ev.text_basis.is_ocr_source());

    Ok(AcceptedOverlap {
        paper_key: cand.paper_key.clone(),
        relation: cand.relation,
        evidence_ids: valid_ids,
        difference: trimmed_diff.to_string(),
        other_name,
        ocr_sourced,
    })
}
