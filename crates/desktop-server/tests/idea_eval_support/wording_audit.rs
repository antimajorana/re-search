//! Comprehensive Rule S8 forbidden-word property checks across Explore `idea_view`, F12 static
//! wording constants, dynamic F12 verdict/scope/abstract formatters, and empty-state views.

use desktop_server::idea_view;
use ledger::Event;
use research_tools::prior_work::{AbstractMatch, AcceptedOverlap, OverlapRelation, PriorWorkStatus};
use research_tools::prior_work_wording::{
    contains_forbidden_prior_work_word, format_abstract_evidence_note, format_coverage_scope,
    format_gate_failure, format_verdict_sentence, wording::ALL_STATIC_CONSTANTS,
};
use research_tools::search_coverage::{compute_coverage_from_events, GateFailure};
use serde_json::Value;

/// Result of Rule S8 forbidden-word checking: `(failures, total_strings_checked)`.
#[derive(Debug, Default, Clone, Copy)]
pub struct S8AuditCount {
    pub failures: u64,
    pub checked: u64,
}

impl S8AuditCount {
    pub fn add(&mut self, other: Self) {
        self.failures += other.failures;
        self.checked += other.checked;
    }

    fn check(&mut self, s: &str) {
        self.checked += 1;
        if contains_forbidden_prior_work_word(s).is_some() {
            self.failures += 1;
        }
    }
}

/// Audit all F12 static constants and representative dynamic F12 formatters under Rule S8.
pub fn audit_f12_wording_rule_s8(sample_events: &[Event]) -> S8AuditCount {
    let mut acc = S8AuditCount::default();
    for s in ALL_STATIC_CONSTANTS {
        acc.check(s);
    }
    let cov = compute_coverage_from_events(sample_events);
    acc.check(&format_coverage_scope(&cov));
    for fail in [
        GateFailure::MissingExactPerspective,
        GateFailure::MissingOtherNamesPerspective,
        GateFailure::InsufficientScreenedCandidates {
            screened: 2,
            required: 10,
            candidates_total: 12,
        },
    ] {
        acc.check(&format_gate_failure(&fail));
    }
    let sample_abs = AbstractMatch {
        paper_key: research_tools::paper_key::PaperKey::parse("arxiv:2401.90101").unwrap(),
        evidence_id: research_tools::prior_work::EvidenceId::parse("xa-1").unwrap(),
        quote: "power-law exponent directly proportional to the Luttinger parameter K".into(),
        provider: Some(research_tools::lit_providers::LitProvider::Arxiv),
    };
    acc.check(&format_abstract_evidence_note(&sample_abs));
    let sample_overlap = AcceptedOverlap {
        paper_key: research_tools::paper_key::PaperKey::parse("arxiv:2401.90101").unwrap(),
        relation: OverlapRelation::Same,
        evidence_ids: vec![research_tools::prior_work::EvidenceId::parse("xe-1").unwrap()],
        difference: "Same zero-temperature scaling regime.".into(),
        other_name: None,
        ocr_sourced: false,
    };
    for (status, confirmed) in [
        (PriorWorkStatus::JudgedSame, false),
        (PriorWorkStatus::JudgedSame, true),
        (PriorWorkStatus::JudgedPartial, false),
        (PriorWorkStatus::JudgedOtherName, false),
        (PriorWorkStatus::AbstractOnly, false),
        (PriorWorkStatus::NoneInScope, false),
        (PriorWorkStatus::Incomplete, false),
    ] {
        let ovs = if matches!(
            status,
            PriorWorkStatus::JudgedSame
                | PriorWorkStatus::JudgedPartial
                | PriorWorkStatus::JudgedOtherName
        ) {
            std::slice::from_ref(&sample_overlap)
        } else {
            &[]
        };
        let abs = if status == PriorWorkStatus::AbstractOnly {
            std::slice::from_ref(&sample_abs)
        } else {
            &[]
        };
        acc.check(&format_verdict_sentence(status, confirmed, ovs, abs, &cov));
    }
    acc
}

/// Audit every app-authored user-facing string in an Explore `idea_view::view` projection
/// (excluding verbatim user idea text and paper titles/abstracts).
pub fn audit_explore_view_rule_s8(view: &Value, events: &[Event]) -> S8AuditCount {
    let mut acc = S8AuditCount::default();
    for key in ["preset_label", "no_model_notice"] {
        if let Some(s) = view[key].as_str() {
            acc.check(s);
        }
    }
    if let Some(h) = view["sharpened_idea"]["heading"].as_str() {
        acc.check(h);
        assert!(
            !h.to_ascii_lowercase().contains("brief"),
            "Sharpened idea heading must never show 'brief': {h}"
        );
    }
    if let Some(stages) = view["stages"].as_array() {
        for st in stages {
            for field in ["label", "run_state_label", "action_label"] {
                if let Some(s) = st[field].as_str() {
                    acc.check(s);
                }
            }
        }
    }
    // Check real `idea_view::build_run_bar` fields (`label`, `reason`, `preset_label`, `cost_line`, `extras[]`).
    let rb = &view["run_bar"];
    assert!(rb["cost_line"].is_string(), "idea_view.run_bar.cost_line must be present");
    for field in ["label", "reason", "preset_label", "cost_line"] {
        if let Some(s) = rb[field].as_str() {
            acc.check(s);
        }
    }
    if let Some(extras) = rb["extras"].as_array() {
        for item in extras {
            if let Some(s) = item.as_str() {
                acc.check(s);
            }
        }
    }
    // Check `"Where I looked"` disclosures: `not_searched`, `scope_summary`, `library.summary`, providers.
    let wil = &view["where_i_looked"];
    if let Some(ns) = wil["not_searched"].as_array() {
        assert!(!ns.is_empty(), "where_i_looked.not_searched must be populated");
        for item in ns {
            if let Some(s) = item.as_str() {
                acc.check(s);
            }
        }
    }
    if let Some(s) = wil["scope_summary"].as_str() {
        acc.check(s);
    }
    if let Some(s) = wil["library"]["summary"].as_str() {
        acc.check(s);
    }
    if let Some(provs) = wil["providers"].as_array() {
        for p in provs {
            for field in ["label", "state", "summary"] {
                if let Some(s) = p[field].as_str() {
                    acc.check(s);
                }
            }
        }
    }
    // Candidate badges, triage basis labels, plan approval card, and empty-state message.
    if let Some(cands) = view["candidates"].as_array() {
        for c in cands {
            for field in ["read_badge", "library_badge"] {
                if let Some(s) = c[field].as_str() {
                    acc.check(s);
                }
            }
        }
    }
    if let Some(items) = view["triage"]["items"].as_array() {
        for it in items {
            if let Some(s) = it["label_basis"].as_str() {
                acc.check(s);
            }
        }
    }
    if let Some(pa) = view["plan_approval"].as_object() {
        for field in ["heading", "summary"] {
            if let Some(s) = pa.get(field).and_then(Value::as_str) {
                acc.check(s);
            }
        }
        if let Some(grants) = pa.get("grants").and_then(Value::as_array) {
            for g in grants {
                for field in ["label", "note"] {
                    if let Some(s) = g[field].as_str() {
                        acc.check(s);
                    }
                }
            }
        }
    }
    if let Some(msg) = view["empty_state"]["message"].as_str() {
        acc.check(msg);
    }
    let cov = compute_coverage_from_events(events);
    acc.check(&format_coverage_scope(&cov));
    acc
}

/// Audit Explore empty-state and static view message constants (`EMPTY_OFFLINE_MESSAGE`,
/// `EMPTY_ALL_FAILED_MESSAGE`, `EMPTY_ZERO_CANDIDATES_MESSAGE`, badges).
pub fn audit_empty_state_messages_rule_s8() -> S8AuditCount {
    let mut acc = S8AuditCount::default();
    for msg in [
        idea_view::NO_MODEL_NOTICE,
        idea_view::EMPTY_OFFLINE_MESSAGE,
        idea_view::EMPTY_ALL_FAILED_MESSAGE,
        idea_view::EMPTY_ZERO_CANDIDATES_MESSAGE,
        idea_view::BADGE_NOT_READ,
        idea_view::BADGE_IN_LIBRARY,
        idea_view::STAGE_IDLE_LABEL,
    ] {
        acc.check(msg);
    }
    acc
}
