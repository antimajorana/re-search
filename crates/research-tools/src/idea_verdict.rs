//! Pure ledger-to-verdict computation for Explore mode (package X11).
//!
//! Delegates to F12 [`crate::prior_work`] and [`crate::search_coverage`] while adding:
//! - Active `idea.stated` revision scoping
//! - Dual-path Rule V9 origin paper exclusion (`handoff.packet.from.paper` OR `ask.fields.origin_paper`)
//! - Rule V9 origin bibliography extraction (`origin_cited_keys` for `cited_by_origin = true`)
//! - Normalization of Explore `idea.note` (`target_id`) confirmation notes for Rule V6
//! - Explicit Rule V10 exclusion of `lessons.supplied` and non-confirmation library/ranking marks
//! - Optional `IdeaPresetLimits` override for deterministic testing and Quick-preset enforcement (Rule V2).

use crate::paper_key::{PaperIds, PaperKey, PaperRef};
use crate::prior_work::{
    compute_prior_work_from_events, PriorWorkReport, PriorWorkVerdict, HANDOFF_PACKET_EVENT,
    IDEA_VERDICT_EVENT, NOTE_ACTION_CONFIRM_SAME, NOTE_ACTION_REJECT_SAME,
};
use crate::search_coverage::{ExplorePreset, SearchCoverage};
use ledger::Event;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

/// Optional preset & budget ceilings passed into [`compute`] (overrides `idea.started` when set).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeaPresetLimits {
    pub preset: ExplorePreset,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_screened: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_papers: Option<usize>,
}

impl IdeaPresetLimits {
    pub const fn for_preset(preset: ExplorePreset) -> Self {
        Self {
            preset,
            max_screened: None,
            max_papers: None,
        }
    }
}

/// Persisted `idea.verdict` payload written atomically at the same head as `idea.map` (`explore.md` 4.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeaVerdictPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map_seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_paper: Option<String>,
    pub verdicts: Vec<PriorWorkVerdict>,
    pub coverage: SearchCoverage,
}

/// Extract the origin [`PaperRef`] (if any) from `handoff.packet` events (Rule V9).
///
/// Inspects both `handoff.packet.from.paper` (F7 standard) and `handoff.packet.ask.fields.origin_paper`
/// (`ExploreHandoffAsk`).
pub fn origin_paper_from_events(events: &[Event]) -> Option<PaperRef> {
    for ev in events.iter().rev() {
        if ev.kind != HANDOFF_PACKET_EVENT {
            continue;
        }
        if let Some(pref) = parse_paper_ref_value(&ev.payload["from"]["paper"]) {
            return Some(pref);
        }
        if let Some(pref) = parse_paper_ref_value(&ev.payload["ask"]["fields"]["origin_paper"]) {
            return Some(pref);
        }
    }
    None
}

/// Extract canonical [`PaperKey`]s of papers cited by the origin paper from `handoff.packet`
/// (`papers[]` and `ask.fields.seed_papers[]`), excluding the origin paper itself (Rule V9).
///
/// Candidates matching any key (or alias key) in this set are marked `cited_by_origin = true`.
pub fn origin_cited_keys(events: &[Event]) -> BTreeSet<PaperKey> {
    let origin = origin_paper_from_events(events);
    let mut keys = BTreeSet::new();

    for ev in events {
        if ev.kind != HANDOFF_PACKET_EVENT {
            continue;
        }
        let lists = [
            ev.payload.get("papers").and_then(Value::as_array),
            ev.payload
                .get("ask")
                .and_then(|a| a.get("fields"))
                .and_then(|f| f.get("seed_papers"))
                .and_then(Value::as_array),
        ];
        for list in lists.into_iter().flatten() {
            for entry in list {
                if let Some(pref) = parse_paper_ref_value(entry) {
                    if is_same_paper_ref(origin.as_ref(), &pref) {
                        continue;
                    }
                    keys.insert(pref.key.clone());
                    for alias in all_ref_keys(&pref) {
                        keys.insert(alias);
                    }
                }
            }
        }
    }
    keys
}

/// Pure function computing per-subclaim verdicts + F12 [`SearchCoverage`] strictly from ledger
/// events (Rules V1–V10).
///
/// - Ignores `lessons.supplied` and all non-confirmation F16/library marks (Rule V10).
/// - Excludes origin paper (`handoff.packet.from.paper` / `ask.fields.origin_paper`) from
///   supporting `judged_*` or `abstract_only` verdicts (Rule V9).
/// - Never counts the user's private library as a literature service for the gate (Rule V10).
/// - Guarantees Quick preset never produces `none_in_scope` (Rule V2).
pub fn compute(events: &[Event], preset_limits: Option<IdeaPresetLimits>) -> IdeaVerdictPayload {
    let latest_stmt_seq = events
        .iter()
        .rev()
        .find(|e| e.kind == "idea.stated")
        .map_or(0, |e| e.seq);
    let origin_paper = origin_paper_from_events(events);
    let latest_map_seq = events
        .iter()
        .rev()
        .find(|e| e.kind == "idea.map" && e.seq > latest_stmt_seq)
        .map(|e| e.seq);

    let normalized = prepare_events_for_f12(events, latest_stmt_seq, origin_paper.as_ref(), preset_limits);
    let PriorWorkReport { coverage, verdicts } = compute_prior_work_from_events(&normalized);

    IdeaVerdictPayload {
        map_seq: latest_map_seq,
        origin_paper: origin_paper.map(|p| p.key.to_string()),
        verdicts,
        coverage,
    }
}

/// Rule V7 parity check: verifies that a stored [`IdeaVerdictPayload`] equals the pure
/// recomputation from `events` (ignoring `map_seq` metadata only if absent).
pub fn matches_recomputed(
    stored: &IdeaVerdictPayload,
    events: &[Event],
    preset_limits: Option<IdeaPresetLimits>,
) -> bool {
    let recomputed = compute(events, preset_limits);
    stored.verdicts == recomputed.verdicts
        && stored.coverage == recomputed.coverage
        && stored.origin_paper == recomputed.origin_paper
}

/// Extract the latest stored `idea.verdict` event from the active revision, if present.
pub fn latest_stored_verdict(events: &[Event]) -> Option<IdeaVerdictPayload> {
    let stmt_seq = events
        .iter()
        .rev()
        .find(|e| e.kind == "idea.stated")
        .map_or(0, |e| e.seq);
    events
        .iter()
        .rev()
        .find(|e| e.kind == IDEA_VERDICT_EVENT && e.seq > stmt_seq)
        .and_then(|e| serde_json::from_value::<IdeaVerdictPayload>(e.payload.clone()).ok())
}

fn prepare_events_for_f12(
    events: &[Event],
    latest_stmt_seq: u64,
    origin_paper: Option<&PaperRef>,
    preset_limits: Option<IdeaPresetLimits>,
) -> Vec<Event> {
    let mut out = Vec::with_capacity(events.len() + 1);
    let mut saw_started = false;

    for ev in events {
        // Rule V10: explicitly strip `lessons.supplied` and any library/mark events.
        if ev.kind == "lessons.supplied" || ev.kind.starts_with("library.") || ev.kind.starts_with("lesson.") {
            continue;
        }
        // When caller passes explicit `preset_limits`, strip `idea.budget_raised` so limits govern.
        if preset_limits.is_some() && ev.kind == "idea.budget_raised" {
            continue;
        }
        let keep_pre_stmt = matches!(
            ev.kind.as_str(),
            "idea.started" | "idea.budget_raised" | HANDOFF_PACKET_EVENT | "source.ingested"
        );
        if ev.seq < latest_stmt_seq && !keep_pre_stmt {
            continue;
        }

        let mut cloned = ev.clone();
        match cloned.kind.as_str() {
            "idea.started" => {
                saw_started = true;
                if let Some(limits) = preset_limits {
                    apply_preset_limits(&mut cloned.payload, limits);
                }
            }
            HANDOFF_PACKET_EVENT => {
                // Rule V9 (Audit CRITICAL #1): populate `from.paper` whenever missing OR null.
                let has_valid_from_paper = cloned
                    .payload
                    .get("from")
                    .and_then(|f| f.get("paper"))
                    .and_then(parse_paper_ref_value)
                    .is_some();
                if !has_valid_from_paper {
                    if let Some(orig) = origin_paper {
                        if let Ok(pref_val) = serde_json::to_value(orig) {
                            if !cloned.payload.is_object() {
                                cloned.payload = json!({});
                            }
                            if let Some(obj) = cloned.payload.as_object_mut() {
                                if !obj.get("from").is_some_and(Value::is_object) {
                                    obj.insert("from".into(), json!({}));
                                }
                                if let Some(from_obj) = obj.get_mut("from").and_then(Value::as_object_mut) {
                                    from_obj.insert("paper".into(), pref_val);
                                }
                            }
                        }
                    }
                }
            }
            "idea.note" => {
                // Rule V10: keep only confirmation/rejection notes (`confirm_same`/`reject_same`) and
                // bridge Explore's `target_id` field to F12's `subclaim_id` lookup (even if null).
                let action = cloned.payload["action"].as_str().unwrap_or("");
                if action != NOTE_ACTION_CONFIRM_SAME && action != NOTE_ACTION_REJECT_SAME {
                    continue;
                }
                let has_target = cloned.payload.get("subclaim_id").and_then(Value::as_str).is_some()
                    || cloned.payload.get("target").and_then(Value::as_str).is_some();
                if !has_target {
                    if let Some(tid) = cloned.payload.get("target_id").and_then(Value::as_str).map(str::to_string) {
                        if let Some(obj) = cloned.payload.as_object_mut() {
                            obj.insert("subclaim_id".into(), Value::String(tid));
                        }
                    }
                }
            }
            _ => {}
        }
        out.push(cloned);
    }

    if !saw_started {
        if let Some(limits) = preset_limits {
            let mut payload = json!({ "preset": preset_str(limits.preset), "budget": {} });
            apply_preset_limits(&mut payload, limits);
            out.insert(
                0,
                Event {
                    session: events.first().map(|e| e.session.clone()).unwrap_or_default(),
                    seq: 0,
                    ts_ms: 0,
                    kind: "idea.started".into(),
                    schema_version: 1,
                    payload,
                    prev_hash: String::new(),
                    event_hash: String::new(),
                },
            );
        }
    }

    out
}

fn preset_str(preset: ExplorePreset) -> &'static str {
    match preset {
        ExplorePreset::Quick => "quick",
        ExplorePreset::Standard => "standard",
        ExplorePreset::Thorough => "thorough",
    }
}

fn apply_preset_limits(payload: &mut Value, limits: IdeaPresetLimits) {
    if !payload.is_object() {
        *payload = json!({});
    }
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("preset".into(), Value::String(preset_str(limits.preset).into()));
        let budget_val = obj.entry("budget").or_insert_with(|| json!({}));
        if let Some(bobj) = budget_val.as_object_mut() {
            if let Some(ms) = limits.max_screened {
                bobj.insert("max_screened".into(), json!(ms));
            }
            if let Some(mp) = limits.max_papers {
                bobj.insert("max_papers".into(), json!(mp));
            } else if limits.preset == ExplorePreset::Quick {
                bobj.insert("max_papers".into(), json!(0));
            }
        }
    }
}

fn parse_paper_ref_value(val: &Value) -> Option<PaperRef> {
    if let Some(raw_key) = val.as_str() {
        let key = PaperKey::parse(raw_key).ok()?;
        return Some(PaperRef { key, version: None, ids: PaperIds::default() });
    }
    let obj = val.as_object()?;
    let key = obj.get("key")?.as_str().and_then(|s| PaperKey::parse(s).ok())?;
    let ids: PaperIds = obj
        .get("ids")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let version = obj.get("version").and_then(Value::as_str).map(str::to_string);
    Some(PaperRef { key, version, ids })
}

fn all_ref_keys(pref: &PaperRef) -> Vec<PaperKey> {
    let mut out = vec![pref.key.clone()];
    if let Some(ref a) = pref.ids.arxiv {
        if let Ok(k) = PaperKey::from_arxiv(a) { out.push(k); }
    }
    if let Some(ref d) = pref.ids.doi {
        if let Ok(k) = PaperKey::from_doi(d) { out.push(k); }
    }
    if let Some(ref i) = pref.ids.inspire {
        if let Ok(k) = PaperKey::from_inspire(i) { out.push(k); }
    }
    if let Some(ref s) = pref.ids.s2 {
        if let Ok(k) = PaperKey::from_s2(s) { out.push(k); }
    }
    out
}

fn is_same_paper_ref(origin: Option<&PaperRef>, cand: &PaperRef) -> bool {
    let Some(orig) = origin else { return false };
    let orig_keys: BTreeSet<PaperKey> = all_ref_keys(orig).into_iter().collect();
    all_ref_keys(cand).iter().any(|k| orig_keys.contains(k))
}
