use super::anchor::{faults_error, AnchorFault, AnchorReport};
use super::{anchor_schema, bounded, string, valid_id, Anchor, SourceIndex};
use ledger::Event;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use tools::{ApprovalScope, ToolDef};

pub const RECORD_PAPER_CONVENTIONS: &str = "record_paper_conventions";
const EVENT: &str = "investigation.conventions";
const CATEGORIES: &[&str] = &[
    "notation",
    "terminology",
    "units",
    "normalization",
    "sign",
    "coordinates",
    "indexing",
    "ordering",
    "orientation",
    "algebra",
    "geometry",
    "reporting",
    "other",
];
const MAX_BATCH: usize = 32;

pub fn conventions_def() -> ToolDef {
    let item = json!({"type":"object","properties":{
        "id":string(48),"category":{"type":"string","enum":CATEGORIES},"label":string(160),"meaning":string(1000),
        "scope":string(450),"basis":{"type":"string","enum":["explicit","inferred"]},"rationale":string(600),
        "anchors":{"type":"array","items":anchor_schema(),"minItems":1,"maxItems":4}
    },"required":["id","category","label","meaning","scope","basis","anchors"],"additionalProperties":false});
    ToolDef {
        id:RECORD_PAPER_CONVENTIONS.into(),
        description:"Record the selected paper's notation and conventions from its actual text. Name the meaning and scope of each choice and cite each anchor with start_line, end_line, and a short verbatim quote (the app fills in the exact cited lines), including local exceptions. Distinguish explicit author choices from inferences about usage, explaining each inference in rationale. Use reporting for statistical, uncertainty and experimental reporting conventions. Send sequential 1-based batches and mark the final batch done. Coverage is the model's assessment of the selected material. Explain any final partial coverage or empty extraction in limitations.".into(),
        input_schema:json!({"type":"object","properties":{
            "request_seq":{"type":"integer","minimum":1},"source_id":string(64),"batch_index":{"type":"integer","minimum":1},
            "done":{"type":"boolean"},"coverage":{"type":"string","enum":["paper","partial"]},
            "limitations":{"type":"array","maxItems":12,"items":string(500)},
            "items":{"type":"array","maxItems":MAX_BATCH,"items":item}
        },"required":["request_seq","source_id","batch_index","done","coverage","limitations","items"],"additionalProperties":false}),
        effect:policy::EffectClass::ReadWorkspace,approval_scope:ApprovalScope::Once,
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Batch {
    request_seq: u64,
    source_id: String,
    batch_index: u64,
    done: bool,
    coverage: String,
    limitations: Vec<String>,
    items: Vec<Item>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Item {
    id: String,
    category: String,
    label: String,
    meaning: String,
    scope: String,
    basis: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rationale: Option<String>,
    anchors: Vec<Anchor>,
}

fn batches(events: &[Event], request_seq: u64) -> impl Iterator<Item = &Event> {
    events.iter().filter(move |e| {
        e.kind == EVENT
            && e.seq > request_seq
            && e.payload["request_seq"].as_u64() == Some(request_seq)
    })
}

fn pending_items_note(pending_items: &[String]) -> String {
    let noun = if pending_items.len() == 1 {
        "item"
    } else {
        "items"
    };
    format!(
        "{} {noun} awaiting valid citations: {}",
        pending_items.len(),
        pending_items.join(", ")
    )
    .chars()
    .take(500)
    .collect()
}

fn is_pending_items_note(s: &str) -> bool {
    let Some((count, rest)) = s.split_once(' ') else {
        return false;
    };
    !count.is_empty()
        && count.bytes().all(|b| b.is_ascii_digit())
        && (rest.starts_with("item awaiting valid citations: ")
            || rest.starts_with("items awaiting valid citations: "))
}

fn is_id_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | '_')
}

fn mentions_item_id(text: &str, id: &str) -> bool {
    if id.is_empty() {
        return false;
    }
    text.match_indices(id).any(|(start, _)| {
        let end = start + id.len();
        let left_ok = text[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !is_id_char(c));
        let right_ok = text[end..]
            .chars()
            .next()
            .is_none_or(|c| !is_id_char(c));
        left_ok && right_ok
    })
}

/// Present accepted batches for one run. Later requests do not inherit the coverage claim.
pub fn paper_conventions(events: &[Event], request_seq: u64) -> Option<Value> {
    let mut items = Vec::new();
    let mut accepted_ids = BTreeSet::new();
    let mut rejected_order = Vec::new();
    let mut rejected_seen = BTreeSet::new();
    let mut abandoned_order = Vec::new();
    let mut abandoned_ids = BTreeSet::new();
    let mut limitations = Vec::new();
    let mut limitations_seen = BTreeSet::new();
    let mut latest = None;
    for event in batches(events, request_seq) {
        latest = Some(event);
        for item in event.payload["items"].as_array()? {
            if let Some(id) = item["id"].as_str() {
                accepted_ids.insert(id.to_owned());
                abandoned_ids.remove(id);
            }
            items.push(item.clone());
        }
        for rejected in event.payload["rejected_items"].as_array().into_iter().flatten() {
            if let Some(id) = rejected["id"].as_str() {
                abandoned_ids.remove(id);
                if rejected_seen.insert(id.to_owned()) {
                    rejected_order.push(id.to_owned());
                }
            }
        }
        for abandoned in event.payload["abandoned_items"].as_array().into_iter().flatten() {
            if let Some(id) = abandoned.as_str() {
                if abandoned_ids.insert(id.to_owned()) && !abandoned_order.iter().any(|x| x == id) {
                    abandoned_order.push(id.to_owned());
                }
            }
        }
        for lim in event.payload["limitations"].as_array().into_iter().flatten() {
            if let Some(s) = lim.as_str() {
                if !is_pending_items_note(s) && limitations_seen.insert(s.to_owned()) {
                    limitations.push(Value::String(s.to_owned()));
                }
            }
        }
    }
    let latest = latest?;
    let abandoned_items: Vec<String> = abandoned_order
        .into_iter()
        .filter(|id| abandoned_ids.contains(id) && !accepted_ids.contains(id))
        .collect();
    let pending_items: Vec<String> = rejected_order
        .into_iter()
        .filter(|id| !accepted_ids.contains(id) && !abandoned_ids.contains(id))
        .collect();
    if !pending_items.is_empty() {
        let note = pending_items_note(&pending_items);
        if limitations_seen.insert(note.clone()) {
            limitations.push(Value::String(note));
        }
    }
    let mut result = latest.payload.clone();
    result["items"] = Value::Array(items);
    result["limitations"] = Value::Array(limitations);
    result["conventions_seq"] = json!(latest.seq);
    result["pending_items"] = json!(pending_items);
    if !abandoned_items.is_empty() {
        result["abandoned_items"] = json!(abandoned_items);
    }
    if !pending_items.is_empty() {
        result["done"] = json!(false);
        result["coverage"] = json!("partial");
    }
    Some(result)
}

pub(super) fn payload<'a>(
    args: &Value,
    index: &mut SourceIndex<'_>,
    history: &'a [Event],
    request: &Event,
    report: &mut AnchorReport,
) -> Result<(Value, Option<&'a Event>), String> {
    let mut batch: Batch = serde_json::from_value(args.clone())
        .map_err(|e| format!("Invalid conventions batch: {e}"))?;
    if batch.request_seq != request.seq {
        return Err("Use the current paper mapping request number when saving conventions.".into());
    }
    if batch.batch_index == 0
        || batch.items.len() > MAX_BATCH
        || !matches!(batch.coverage.as_str(), "paper" | "partial")
        || batch.limitations.len() > 12
        || batch.limitations.iter().any(|s| !bounded(s, 500))
    {
        return Err(
            "Use a positive batch index, valid coverage and bounded conventions and limitations."
                .into(),
        );
    }
    let active_marker = history
        .iter()
        .rev()
        .find(|e| {
            e.seq >= request.seq
                && matches!(
                    e.kind.as_str(),
                    "investigation.mapping_requested" | "investigation.conventions_resumed"
                )
        })
        .unwrap_or(request);
    let paper_resume = active_marker.kind == "investigation.conventions_resumed"
        && active_marker.payload["scope"].as_str() == Some("paper");
    let derived_scope = if active_marker.kind == "investigation.conventions_resumed" {
        active_marker.payload["scope"].as_str()
    } else {
        request.payload["conventions_scope"].as_str()
    };
    let mut prior_ids = BTreeSet::new();
    let mut prior_rejected_order = Vec::new();
    let mut prior_rejected_seen = BTreeSet::new();
    let mut prior_abandoned_ids = BTreeSet::new();
    let mut labels = BTreeSet::new();
    let mut next = 1;
    let mut previous = None;
    for event in batches(history, request.seq) {
        if next == batch.batch_index {
            previous = Some(event);
            break;
        }
        if event.payload["done"] == true {
            let section_upgrade =
                event.payload["coverage_scope"].as_str() == Some("section") && paper_resume;
            if !section_upgrade {
                return Err("The conventions list for this request is already finalized.".into());
            }
        }
        for item in event.payload["items"].as_array().into_iter().flatten() {
            if let (Some(id), Some(label), Some(scope)) = (
                item["id"].as_str(),
                item["label"].as_str(),
                item["scope"].as_str(),
            ) {
                prior_ids.insert(id.to_owned());
                prior_abandoned_ids.remove(id);
                labels.insert((label.trim().to_owned(), scope.trim().to_owned()));
            }
        }
        for rejected in event.payload["rejected_items"].as_array().into_iter().flatten() {
            if let Some(id) = rejected["id"].as_str() {
                prior_abandoned_ids.remove(id);
                if prior_rejected_seen.insert(id.to_owned()) {
                    prior_rejected_order.push(id.to_owned());
                }
            }
        }
        for abandoned in event.payload["abandoned_items"].as_array().into_iter().flatten() {
            if let Some(id) = abandoned.as_str() {
                prior_abandoned_ids.insert(id.to_owned());
            }
        }
        next += 1;
    }
    if batch.batch_index != next {
        return Err(format!(
            "Use batch_index={next} for the next conventions batch."
        ));
    }
    if !batch.done && (batch.items.is_empty() || batch.coverage != "partial") {
        return Err(
            "An unfinished batch needs at least one convention and partial coverage.".into(),
        );
    }
    if batch.done
        && batch.limitations.is_empty()
        && (batch.coverage == "partial" || (prior_ids.is_empty() && batch.items.is_empty()))
    {
        return Err("Explain any unexamined material, or why no paper-specific conventions were identified.".into());
    }
    // Pass 1: structural validation against tentative sets (accepted prior items + this batch).
    let mut tentative_ids = prior_ids.clone();
    let mut tentative_labels = labels.clone();
    for item in &batch.items {
        if !valid_id(&item.id, 48)
            || !tentative_ids.insert(item.id.clone())
            || !CATEGORIES.contains(&item.category.as_str())
            || !bounded(&item.label, 160)
            || !bounded(&item.meaning, 1000)
            || !bounded(&item.scope, 450)
            || !tentative_labels.insert((item.label.trim().to_owned(), item.scope.trim().to_owned()))
        {
            return Err(format!("Convention '{}' needs a unique ID and label in its scope, a supported category, meaning and scope.",item.id));
        }
        if !matches!(item.basis.as_str(), "explicit" | "inferred")
            || item.rationale.as_deref().is_some_and(|s| !bounded(s, 600))
            || (item.basis == "inferred" && item.rationale.is_none())
        {
            return Err(format!(
                "State whether '{}' is explicit or inferred, and explain any inferred usage.",
                item.id
            ));
        }
        if !(1..=4).contains(&item.anchors.len()) {
            return Err(format!(
                "Cite one to four passages for convention '{}'.",
                item.id
            ));
        }
        for anchor in &item.anchors {
            if anchor.source_id != batch.source_id {
                return Err("Conventions must cite the selected paper.".into());
            }
        }
    }
    // Pass 2: anchor verification; exclude items whose pointer anchors all fail, fail fast on legacy faults.
    let had_items = !batch.items.is_empty();
    let mut accepted_items = Vec::new();
    let mut rejected_items = Vec::new();
    let mut all_faults: Vec<AnchorFault> = Vec::new();
    for mut item in std::mem::take(&mut batch.items) {
        let mut kept = Vec::new();
        let mut item_notes = Vec::new();
        let mut item_faults = Vec::new();
        for (i, mut anchor) in std::mem::take(&mut item.anchors).into_iter().enumerate() {
            let owner = format!("items[{}].anchors[{i}]", item.id);
            match index.verify_any(&owner, &mut anchor) {
                Ok(note) => {
                    kept.push(anchor);
                    item_notes.push(note);
                }
                Err(fault) => {
                    if fault.legacy {
                        return Err(fault.message);
                    }
                    item_faults.push(fault);
                }
            }
        }
        if kept.is_empty() {
            all_faults.extend(item_faults.iter().cloned());
            rejected_items.push(json!({"id": item.id, "faults": item_faults}));
        } else {
            for note in item_notes {
                report.record_note(note);
            }
            for fault in &item_faults {
                report.record_dropped(fault);
            }
            item.anchors = kept;
            accepted_items.push(item);
        }
    }
    if had_items && accepted_items.is_empty() {
        return Err(faults_error(&all_faults));
    }
    // Rebuild accepted sets from accepted items only (prior accepted items + accepted_items).
    for item in &accepted_items {
        prior_ids.insert(item.id.clone());
        prior_abandoned_ids.remove(&item.id);
        labels.insert((item.label.trim().to_owned(), item.scope.trim().to_owned()));
    }
    let mut newly_rejected_ids = BTreeSet::new();
    for rejected in &rejected_items {
        if let Some(id) = rejected["id"].as_str() {
            newly_rejected_ids.insert(id.to_owned());
            prior_abandoned_ids.remove(id);
            if prior_rejected_seen.insert(id.to_owned()) {
                prior_rejected_order.push(id.to_owned());
            }
        }
    }
    let mut abandoned_items = Vec::new();
    if batch.done && batch.coverage == "partial" {
        for id in &prior_rejected_order {
            if prior_ids.contains(id)
                || prior_abandoned_ids.contains(id)
                || newly_rejected_ids.contains(id)
            {
                continue;
            }
            if batch
                .limitations
                .iter()
                .any(|lim| !is_pending_items_note(lim) && mentions_item_id(lim, id))
            {
                abandoned_items.push(id.clone());
                prior_abandoned_ids.insert(id.clone());
            }
        }
    }
    let pending_items: Vec<String> = prior_rejected_order
        .into_iter()
        .filter(|id| !prior_ids.contains(id) && !prior_abandoned_ids.contains(id))
        .collect();
    batch.items = accepted_items;
    let mut done_requested = None;
    let mut coverage_reported = None;
    if !pending_items.is_empty() {
        done_requested = Some(batch.done);
        coverage_reported = Some(batch.coverage.clone());
        batch.done = false;
        batch.coverage = "partial".into();
        let note = pending_items_note(&pending_items);
        if !batch.limitations.iter().any(|s| s == &note) {
            if batch.limitations.len() >= 12 {
                if let Some(pos) = batch.limitations.iter().rposition(|lim| {
                    !abandoned_items.iter().any(|id| mentions_item_id(lim, id))
                }) {
                    batch.limitations.remove(pos);
                } else {
                    batch.limitations.truncate(11);
                }
            }
            batch.limitations.push(note);
        }
    }
    let mut payload = serde_json::to_value(batch).map_err(|e| e.to_string())?;
    payload["coverage_basis"] = json!("model_reported");
    if !rejected_items.is_empty() {
        payload["rejected_items"] = Value::Array(rejected_items);
    }
    if !abandoned_items.is_empty() {
        payload["abandoned_items"] = Value::Array(
            abandoned_items.into_iter().map(Value::String).collect(),
        );
    }
    if let (Some(done_req), Some(cov_rep)) = (done_requested, coverage_reported) {
        payload["done_requested"] = json!(done_req);
        payload["coverage_reported"] = json!(cov_rep);
    }
    if let Some(prev) = previous {
        if let Some(prev_scope) = prev.payload.get("coverage_scope") {
            payload["coverage_scope"] = prev_scope.clone();
        }
    } else if let Some(scope @ ("section" | "paper")) = derived_scope {
        payload["coverage_scope"] = json!(scope);
    }
    Ok((payload, previous))
}
