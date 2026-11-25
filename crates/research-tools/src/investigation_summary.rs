//! The early, source-grounded reading overview appears while the detailed graph is being prepared.
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
        description: "Record the reader's early paper overview before building the detailed map. Give one or two concise sentences each for what the paper does, its stated motivation, and its method. Cite exact excerpts in the primary source. If the available text does not state why or how, send only {unstated:true} for that part.".into(),
        input_schema: json!({"type":"object","properties":{"source_id":string(64),"title":string(300),"what":what,"why":stated,"how":stated},
            "required":["source_id","what","why","how"],"additionalProperties":false}),
        effect:policy::EffectClass::ReadWorkspace, approval_scope:ApprovalScope::Once,
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PaperSummary {
    source_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    what: Part,
    why: Part,
    how: Part,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum Part {
    Stated { text: String, anchors: Vec<Anchor> },
    Unstated { unstated: bool },
}

pub(super) fn payload(args: &Value, index: &mut SourceIndex<'_>) -> Result<Value, String> {
    let mut summary: PaperSummary = serde_json::from_value(args.clone()).map_err(|_| {
        "Each summary part needs text and anchors, or only unstated=true for why or how."
    })?;
    if summary.title.as_deref().is_some_and(|s| !bounded(s, 300)) {
        return Err("Choose a short paper title.".into());
    }
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
                for (idx, anchor) in anchors.iter_mut().enumerate() {
                    if anchor.source_id != summary.source_id {
                        return Err(format!(
                            "The '{name}' summary must cite the selected paper."
                        ));
                    }
                    let owner = format!("{name}.anchors[{idx}]");
                    index.verify_any(&owner, anchor).map_err(|f| f.to_error_string())?;
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
    serde_json::to_value(summary).map_err(|e| e.to_string())
}
