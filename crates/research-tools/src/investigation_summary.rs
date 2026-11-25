//! The early, source-grounded reading overview appears while the detailed graph is being prepared.
use super::anchor::{faults_error, AnchorFault, AnchorReport};
use super::{anchor_schema, bounded, string, Anchor, SourceIndex};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tools::{ApprovalScope, ToolDef};

pub const RECORD_PAPER_SUMMARY: &str = "record_paper_summary";

pub fn summary_def() -> ToolDef {
    let mut stated = json!({"type":"object","properties":{
        "text":string(700),"anchors":{"type":"array","minItems":1,"maxItems":3,"items":anchor_schema()}
    },"required":["text","anchors"],"additionalProperties":false});
    let what = stated.clone();
    // Stored and validated as a strict union. A flat schema works with providers
    // whose tool-call dialects do not support nested oneOf declarations.
    stated.as_object_mut().unwrap().remove("required");
    stated["properties"]["unstated"] = json!({"type":"boolean","description":"Use true alone if the available source does not state this."});
    ToolDef {
        id: RECORD_PAPER_SUMMARY.into(),
        description: "Record the reader's early paper overview before building the detailed map. Give one or two concise sentences each for what the paper does, its stated motivation, and its method. Cite each anchor with start_line, end_line, and a short verbatim quote from the primary source (the app fills in the exact cited lines). If the available text does not state why or how, send only {unstated:true} for that part.".into(),
        input_schema: json!({"type":"object","properties":{"source_id":string(64),"title":string(300),"what":what,"why":stated,"how":stated},
            "required":["source_id","what","why","how"],"additionalProperties":false}),
        effect:policy::EffectClass::ReadWorkspace, approval_scope:ApprovalScope::Once,
    }
}

#[derive(Serialize)]
struct PaperSummary {
    source_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    what: Part,
    why: Part,
    how: Part,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Part {
    Stated { text: String, anchors: Vec<Anchor> },
    Unstated { unstated: bool },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StatedInput {
    text: String,
    anchors: Vec<Anchor>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnstatedInput {
    unstated: bool,
}

fn parse_part(name: &str, value: &Value) -> Result<Part, String> {
    let obj = value
        .as_object()
        .ok_or_else(|| format!("Invalid '{name}' summary part: expected a JSON object."))?;
    if obj.contains_key("unstated") && !obj.contains_key("text") && !obj.contains_key("anchors") {
        let parsed: UnstatedInput = serde_json::from_value(value.clone())
            .map_err(|e| format!("Invalid '{name}' summary part: {e}"))?;
        if !parsed.unstated || name == "what" {
            return Err(format!(
                "The '{name}' summary requires a source-grounded statement."
            ));
        }
        return Ok(Part::Unstated { unstated: true });
    }
    let parsed: StatedInput = serde_json::from_value(value.clone())
        .map_err(|e| format!("Invalid '{name}' summary part: {e}"))?;
    Ok(Part::Stated {
        text: parsed.text,
        anchors: parsed.anchors,
    })
}

fn parse_summary(args: &Value) -> Result<PaperSummary, String> {
    let obj = args
        .as_object()
        .ok_or("Invalid paper summary: expected a JSON object.")?;
    for key in obj.keys() {
        if !matches!(key.as_str(), "source_id" | "title" | "what" | "why" | "how") {
            return Err(format!("Invalid paper summary: unknown field `{key}`."));
        }
    }
    let source_id = obj
        .get("source_id")
        .and_then(Value::as_str)
        .ok_or("Invalid paper summary: missing string `source_id`.")?
        .to_string();
    let title = match obj.get("title") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_str()
                .ok_or("Invalid paper summary: `title` must be a string.")?
                .to_string(),
        ),
    };
    let req_part = |name: &str| {
        let v = obj
            .get(name)
            .ok_or_else(|| format!("Invalid paper summary: missing `{name}`."))?;
        parse_part(name, v)
    };
    Ok(PaperSummary {
        source_id,
        title,
        what: req_part("what")?,
        why: req_part("why")?,
        how: req_part("how")?,
    })
}

pub(super) fn payload(
    args: &Value,
    index: &mut SourceIndex<'_>,
    report: &mut AnchorReport,
) -> Result<Value, String> {
    let mut summary = parse_summary(args)?;
    if summary.title.as_deref().is_some_and(|s| !bounded(s, 300)) {
        return Err("Choose a short paper title.".into());
    }
    let mut fatal_faults: Vec<AnchorFault> = Vec::new();
    for (name, field) in [
        ("what", &mut summary.what),
        ("why", &mut summary.why),
        ("how", &mut summary.how),
    ] {
        match field {
            Part::Stated { text, anchors } => {
                if !bounded(text, 700) || !(1..=3).contains(&anchors.len()) {
                    return Err(format!(
                        "Give a short '{name}' statement and one to three verbatim anchors."
                    ));
                }
                for anchor in anchors.iter() {
                    if anchor.source_id != summary.source_id {
                        return Err(format!(
                            "The '{name}' summary must cite the selected paper."
                        ));
                    }
                }
                let mut kept = Vec::new();
                let mut part_faults = Vec::new();
                let mut part_notes = Vec::new();
                for (i, mut anchor) in std::mem::take(anchors).into_iter().enumerate() {
                    let owner = format!("{name}.anchors[{i}]");
                    match index.verify_any(&owner, &mut anchor) {
                        Ok(note) => {
                            kept.push(anchor);
                            part_notes.push(note);
                        }
                        Err(fault) => {
                            if fault.legacy {
                                return Err(fault.message);
                            }
                            part_faults.push(fault);
                        }
                    }
                }
                if kept.is_empty() {
                    fatal_faults.extend(part_faults);
                } else {
                    for note in part_notes {
                        report.record_note(note);
                    }
                    for fault in &part_faults {
                        report.record_dropped(fault);
                    }
                    *anchors = kept;
                }
            }
            Part::Unstated { unstated: true } if name != "what" => {}
            _ => {
                return Err(format!(
                    "The '{name}' summary requires a source-grounded statement."
                ))
            }
        }
    }
    if !fatal_faults.is_empty() {
        return Err(faults_error(&fatal_faults));
    }
    serde_json::to_value(summary).map_err(|e| e.to_string())
}
