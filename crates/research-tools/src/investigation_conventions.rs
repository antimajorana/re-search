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
        description:"Record the selected paper's notation and conventions from its actual text. Name the meaning and scope of each choice and cite exact excerpts, including local exceptions. Distinguish explicit author choices from inferences about usage, explaining each inference in rationale. Use reporting for statistical, uncertainty and experimental reporting conventions. Send sequential 1-based batches and mark the final batch done. Coverage is the model's assessment of the selected material. Explain any final partial coverage or empty extraction in limitations.".into(),
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

/// Present accepted batches for one run. Later requests do not inherit the coverage claim.
pub fn paper_conventions(events: &[Event], request_seq: u64) -> Option<Value> {
    let mut items = Vec::new();
    let mut latest = None;
    for event in batches(events, request_seq) {
        latest = Some(event);
        items.extend(event.payload["items"].as_array()?.iter().cloned());
    }
    let latest = latest?;
    let mut result = latest.payload.clone();
    result["items"] = Value::Array(items);
    result["conventions_seq"] = json!(latest.seq);
    Some(result)
}

pub(super) fn payload<'a>(
    args: &Value,
    index: &mut SourceIndex<'_>,
    history: &'a [Event],
    request: &Event,
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
    let mut prior_ids = BTreeSet::new();
    let mut labels = BTreeSet::new();
    let mut next = 1;
    let mut previous = None;
    for event in batches(history, request.seq) {
        if next == batch.batch_index {
            previous = Some(event);
            break;
        }
        if event.payload["done"] == true {
            return Err("The conventions list for this request is already finalized.".into());
        }
        for item in event.payload["items"].as_array().into_iter().flatten() {
            if let (Some(id), Some(label), Some(scope)) = (
                item["id"].as_str(),
                item["label"].as_str(),
                item["scope"].as_str(),
            ) {
                prior_ids.insert(id.to_owned());
                labels.insert((label.trim().to_owned(), scope.trim().to_owned()));
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
    for item in &mut batch.items {
        if !valid_id(&item.id, 48)
            || !prior_ids.insert(item.id.clone())
            || !CATEGORIES.contains(&item.category.as_str())
            || !bounded(&item.label, 160)
            || !bounded(&item.meaning, 1000)
            || !bounded(&item.scope, 450)
            || !labels.insert((item.label.trim().to_owned(), item.scope.trim().to_owned()))
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
        for (idx, anchor) in item.anchors.iter_mut().enumerate() {
            if anchor.source_id != batch.source_id {
                return Err("Conventions must cite the selected paper.".into());
            }
            let owner = format!("items[{}].anchors[{idx}]", item.id);
            index.verify_any(&owner, anchor).map_err(|f| f.to_error_string())?;
        }
    }
    let mut payload = serde_json::to_value(batch).map_err(|e| e.to_string())?;
    payload["coverage_basis"] = json!("model_reported");
    Ok((payload, previous))
}
