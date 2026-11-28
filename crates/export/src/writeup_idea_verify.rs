//! Verification and honesty-gate helpers for the F15 Explore write-up adapter (`writeup_idea.rs`).
//!
//! - Rule V7/V10 fail-closed stored `idea.verdict` parity check (`verify_stored_verdict_parity`).
//! - Audit C2 iterative pre-verification of all referenced `xe-*`/`xa-*` quotes before verdict
//!   finalization and bibliography registration (`verify_and_recompute_verdicts`).
//! - Rule S8 full-document honesty check (`enforce_explore_honesty`).

use super::cite::{IdeaCiteResolver, VerifiedAbstractData, VerifiedFullTextData};
use super::sections::{clip_title, contains_ascii_word_ci};
use super::ActiveBriefView;
use crate::writeup::{ProseBasis, Writeup, WriteupBlock};
use ledger::Event;
use research_tools::idea_verdict::{self, IdeaVerdictPayload};
use research_tools::paper_key::PaperKey;
use research_tools::prior_work::{EvidenceId, IDEA_VERDICT_EVENT};
use research_tools::quote_core::{EVIDENCE_ABSTRACT_EVENT, EVIDENCE_QUOTE_EVENT};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct VerifiedVerdictState {
    pub current: IdeaVerdictPayload,
    pub verified_xe: BTreeMap<(PaperKey, u64), Option<VerifiedFullTextData>>,
    pub verified_xa: BTreeMap<(PaperKey, u64), Option<VerifiedAbstractData>>,
    pub dropped_by_subclaim: BTreeMap<String, usize>,
}

/// Rule V7 & V10 fail-closed verification: if an `idea.verdict` exists in the active revision,
/// its stored payload must exactly match pure ledger recomputation at that event's ledger head.
pub(crate) fn verify_stored_verdict_parity(
    events: &[Event],
    latest_stmt_seq: u64,
) -> Result<(), String> {
    let Some((verdict_idx, verdict_ev)) = events
        .iter()
        .enumerate()
        .rev()
        .find(|(_, e)| e.kind == IDEA_VERDICT_EVENT && e.seq > latest_stmt_seq)
    else {
        return Ok(());
    };
    let stored: IdeaVerdictPayload =
        serde_json::from_value(verdict_ev.payload.clone()).map_err(|e| {
            format!("Explore write-up refused (Rule V7): stored `idea.verdict` is malformed ({e})")
        })?;
    let recomputed = idea_verdict::compute(&events[..verdict_idx], None);
    let map_seq_ok = stored.map_seq.is_none_or(|ms| Some(ms) == recomputed.map_seq);
    if stored.verdicts != recomputed.verdicts
        || stored.coverage != recomputed.coverage
        || stored.origin_paper != recomputed.origin_paper
        || !map_seq_ok
    {
        return Err(
            "Explore write-up refused (Rule V7): stored `idea.verdict` disagrees with pure ledger recomputation"
                .to_string(),
        );
    }
    Ok(())
}

/// Pre-verify every referenced `xe-*`/`xa-*` quote on disk / in pinned `raw_blob`s before emitting
/// prose or registering `.bib` entries, filtering failed evidence events and recomputing verdicts
/// until fixed point so statuses honestly downgrade when evidence is dropped (Audit C2).
pub(crate) fn verify_and_recompute_verdicts(
    events: &[Event],
    brief_view: &ActiveBriefView,
    cite_resolver: &mut IdeaCiteResolver<'_>,
) -> VerifiedVerdictState {
    let mut active_events: Option<Vec<Event>> = None;
    let mut current = idea_verdict::compute(events, None);
    let mut verified_xe: BTreeMap<(PaperKey, u64), Option<VerifiedFullTextData>> = BTreeMap::new();
    let mut verified_xa: BTreeMap<(PaperKey, u64), Option<VerifiedAbstractData>> = BTreeMap::new();
    let mut dropped_by_subclaim: BTreeMap<String, usize> = BTreeMap::new();
    let mut counted_drops: BTreeSet<(String, bool, u64)> = BTreeSet::new();

    for _ in 0..=events.len() {
        let mut newly_failed_xe: BTreeSet<u64> = BTreeSet::new();
        let mut newly_failed_xa: BTreeSet<u64> = BTreeSet::new();

        for verdict in &current.verdicts {
            let subclaim_text = brief_view
                .subclaim_texts
                .get(&verdict.subclaim_id)
                .map(String::as_str)
                .unwrap_or(verdict.subclaim_id.as_str());
            let sec_title =
                format!("Part {}: {}", verdict.subclaim_id, clip_title(subclaim_text, 64));

            for overlap in &verdict.overlaps {
                for eid in &overlap.evidence_ids {
                    if let EvidenceId::FullText(seq) = *eid {
                        let key = (overlap.paper_key.clone(), seq);
                        let is_ok = if let Some(cached) = verified_xe.get(&key) {
                            cached.is_some()
                        } else {
                            let res = cite_resolver.verify_full_text_quote(
                                &sec_title,
                                &overlap.paper_key,
                                seq,
                            );
                            let ok = res.is_some();
                            if !ok {
                                newly_failed_xe.insert(seq);
                            }
                            verified_xe.insert(key, res);
                            ok
                        };
                        if !is_ok && counted_drops.insert((verdict.subclaim_id.clone(), false, seq)) {
                            *dropped_by_subclaim.entry(verdict.subclaim_id.clone()).or_insert(0) += 1;
                        }
                    }
                }
            }

            for abs in &verdict.abstract_matches {
                if let EvidenceId::Abstract(seq) = abs.evidence_id {
                    let key = (abs.paper_key.clone(), seq);
                    let is_ok = if let Some(cached) = verified_xa.get(&key) {
                        cached.is_some()
                    } else {
                        let res =
                            cite_resolver.verify_abstract_quote(&sec_title, &abs.paper_key, seq);
                        let ok = res.is_some();
                        if !ok {
                            newly_failed_xa.insert(seq);
                        }
                        verified_xa.insert(key, res);
                        ok
                    };
                    if !is_ok && counted_drops.insert((verdict.subclaim_id.clone(), true, seq)) {
                        *dropped_by_subclaim.entry(verdict.subclaim_id.clone()).or_insert(0) += 1;
                    }
                }
            }
        }

        if newly_failed_xe.is_empty() && newly_failed_xa.is_empty() {
            break;
        }
        let filtered = active_events.get_or_insert_with(|| events.to_vec());
        filtered.retain(|ev| {
            !(ev.kind == EVIDENCE_QUOTE_EVENT && newly_failed_xe.contains(&ev.seq))
                && !(ev.kind == EVIDENCE_ABSTRACT_EVENT && newly_failed_xa.contains(&ev.seq))
        });
        current = idea_verdict::compute(filtered, None);
    }

    VerifiedVerdictState {
        current,
        verified_xe,
        verified_xa,
        dropped_by_subclaim,
    }
}

/// Rule S8 whole-document honesty check: forbids `"verified"` across status line, title, coverage,
/// section headers, app/model prose, statements, and check-table rows (while permitting user-owned
/// text and verbatim paper quotes).
pub(crate) fn enforce_explore_honesty(doc: &Writeup) -> Result<(), String> {
    if doc.status_line.to_ascii_lowercase().contains("verified") {
        return Err("Explore write-up status_line must not contain \"verified\" (Rule S8)".into());
    }
    if contains_ascii_word_ci(&doc.title, "verified") {
        return Err("Explore write-up title must not contain \"verified\" (Rule S8)".into());
    }
    if let Some(cov) = doc.coverage_paragraph.as_deref() {
        if cov.to_ascii_lowercase().contains("verified") {
            return Err("Explore coverage paragraph must not contain \"verified\" (Rule S8)".into());
        }
    }
    for sec in &doc.sections {
        if contains_ascii_word_ci(&sec.title, "verified") {
            return Err(format!(
                "Explore section `{}` must not contain \"verified\" (Rule S8)",
                sec.title
            ));
        }
        for blk in &sec.blocks {
            match blk {
                WriteupBlock::Prose { text, basis: ProseBasis::App | ProseBasis::Model }
                    if contains_ascii_word_ci(text, "verified") =>
                {
                    return Err(
                        "Explore app/model prose must not contain \"verified\" (Rule S8)".into(),
                    );
                }
                WriteupBlock::Statement { id, text, note, .. }
                    if contains_ascii_word_ci(id, "verified")
                        || contains_ascii_word_ci(text, "verified")
                        || note.as_deref().is_some_and(|n| contains_ascii_word_ci(n, "verified")) =>
                {
                    return Err(format!(
                        "Explore statement `{id}` must not contain \"verified\" (Rule S8)"
                    ));
                }
                _ => {}
            }
        }
    }
    for row in doc.derived_check_table() {
        if contains_ascii_word_ci(&row.id, "verified")
            || contains_ascii_word_ci(&row.badge_text(), "verified")
            || contains_ascii_word_ci(&row.statement, "verified")
            || row.note.as_deref().is_some_and(|n| contains_ascii_word_ci(n, "verified"))
        {
            return Err(format!(
                "Explore check table row `{}` must not contain \"verified\" (Rule S8)",
                row.id
            ));
        }
    }
    Ok(())
}
