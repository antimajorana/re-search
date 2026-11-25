//! Historical `ToolDef` schemas for Investigation recorders (commit `7c36a0e`),
//! used by session resume drift checks so pre-pointer sessions can still resume safely.
use super::anchor::{string_schema as string, ANCHOR_KINDS};
use super::{
    branch_def, conventions_def, def, summary_def, EDGE_KINDS, MAX_EDGES, MAX_NODES, NODE_KINDS,
    RECORD_PAPER_BRANCH, RECORD_PAPER_CONVENTIONS, RECORD_PAPER_MAP, RECORD_PAPER_SUMMARY,
};
use serde_json::{json, Value};
use tools::{ApprovalScope, ToolDef};

pub const GOLDEN_MAP_7C36A0E: &str =
    "b58425d3f6b5b16425a9873830c0c5b39454dca3948418ef9ef77236d0475b17";
pub const GOLDEN_MAP_NO_ANSWER_INDEX_7C36A0E: &str =
    "2eb56f1c1bd5ea54bed3995e95eeb98f5e4c8a30697ef3da0bc6f0ed0f6d5fcf";
pub const GOLDEN_SUMMARY_7C36A0E: &str =
    "c1757b2c8712e12435273a4e655486f7566432380ff13b79c04a3dfcaae91b27";
pub const GOLDEN_BRANCH_7C36A0E: &str =
    "c1a960a090f0481b8e2277b08d92a164feb6894af79cdb70cde24b6d84cabf88";
pub const GOLDEN_CONVENTIONS_7C36A0E: &str =
    "ff8682073cb81ffa3f20270b2d9da2a589175e5f739f30d4347f6e7579a2031d";

const LEGACY_CONVENTION_CATEGORIES: &[&str] = &[
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

fn legacy_anchor_schema() -> Value {
    json!({"type":"object","properties":{
        "source_id":string(64),"path":string(1024),
        "start_line":{"type":"integer","minimum":1},"end_line":{"type":"integer","minimum":1},
        "excerpt":string(3200),"kind":{"type":"string","enum":ANCHOR_KINDS},"label":string(160)
    },"required":["source_id","path","start_line","end_line","excerpt","kind"],"additionalProperties":false})
}

fn legacy_map_def_7c36a0e() -> ToolDef {
    let strings = |n, max| json!({"type":"array","maxItems":n,"items":string(max)});
    let anchor = legacy_anchor_schema();
    let node = json!({"type":"object","properties":{
        "id":string(48),"sentence":string(1400),"kind":{"type":"string","enum":NODE_KINDS},"main":{"type":"boolean"},
        "questions":strings(4,360),"anchors":{"type":"array","minItems":1,"maxItems":4,"items":anchor}
    },"required":["id","sentence","kind","main","anchors"],"additionalProperties":false});
    let edge = json!({"type":"object","properties":{
        "from":string(48),"to":string(48),"kind":{"type":"string","enum":EDGE_KINDS},"basis":{"type":"string","enum":["explicit","inferred"]},
        "reason":string(1000),"anchor":anchor
    },"required":["from","to","kind","basis","reason","anchor"],"additionalProperties":false});
    ToolDef {
        id: RECORD_PAPER_MAP.into(),
        description: "Record a paper's argument map in the current Investigation. Every node and edge needs a verbatim excerpt with 1-based lines from the selected paper. Each arrow points from supporting material to the claim that uses it. External reference text may supplement its citation in the primary paper. Coverage is the model's assessment; the recorder verifies exact anchors and DAG structure.".into(),
        input_schema: json!({"type":"object","properties":{
            "source_id":string(64),"title":string(300),"coverage":{"type":"string","enum":["paper","partial"]},
            "answer_node_ids":strings(MAX_NODES,48),
            "limitations":strings(12,500),"nodes":{"type":"array","minItems":1,"maxItems":MAX_NODES,"items":node},
            "edges":{"type":"array","maxItems":MAX_EDGES,"items":edge}
        },"required":["source_id","title","coverage","limitations","nodes","edges"],"additionalProperties":false}),
        effect: policy::EffectClass::ReadWorkspace,
        approval_scope: ApprovalScope::Once,
    }
}

fn legacy_summary_def_7c36a0e() -> ToolDef {
    let mut stated = json!({"type":"object","properties":{
        "text":string(700),"anchors":{"type":"array","minItems":1,"maxItems":3,"items":legacy_anchor_schema()}
    },"required":["text","anchors"],"additionalProperties":false});
    let what = stated.clone();
    stated.as_object_mut().unwrap().remove("required");
    stated["properties"]["unstated"] = json!({"type":"boolean","description":"Use true alone if the available source does not state this."});
    ToolDef {
        id: RECORD_PAPER_SUMMARY.into(),
        description: "Record the reader's early paper overview before building the detailed map. Give one or two concise sentences each for what the paper does, its stated motivation, and its method. Cite exact excerpts in the primary source. If the available text does not state why or how, send only {unstated:true} for that part.".into(),
        input_schema: json!({"type":"object","properties":{"source_id":string(64),"title":string(300),"what":what,"why":stated,"how":stated},
            "required":["source_id","what","why","how"],"additionalProperties":false}),
        effect: policy::EffectClass::ReadWorkspace,
        approval_scope: ApprovalScope::Once,
    }
}

fn legacy_branch_def_7c36a0e() -> ToolDef {
    let mut def = legacy_map_def_7c36a0e();
    let base = &def.input_schema["properties"];
    let mut nodes = base["nodes"].clone();
    nodes["minItems"] = json!(0);
    nodes["maxItems"] = json!(96);
    let mut edges = base["edges"].clone();
    edges["maxItems"] = json!(256);
    def.id = RECORD_PAPER_BRANCH.into();
    def.description = "Save the source-grounded dependencies for the currently selected paper result and reading question. Supply new nodes and new edges pointing from evidence or assumptions towards that result. Use limitations for unsupported parts of the argument.".into();
    def.input_schema = json!({"type":"object","properties":{
        "request_seq":{"type":"integer","minimum":1,"description":"The request number in the current branch instructions."},
        "source_id":base["source_id"],"nodes":nodes,"edges":edges,"limitations":base["limitations"]
    },"required":["request_seq","source_id","nodes","edges","limitations"],"additionalProperties":false});
    def
}

fn legacy_conventions_def_7c36a0e() -> ToolDef {
    let item = json!({"type":"object","properties":{
        "id":string(48),"category":{"type":"string","enum":LEGACY_CONVENTION_CATEGORIES},"label":string(160),"meaning":string(1000),
        "scope":string(450),"basis":{"type":"string","enum":["explicit","inferred"]},"rationale":string(600),
        "anchors":{"type":"array","items":legacy_anchor_schema(),"minItems":1,"maxItems":4}
    },"required":["id","category","label","meaning","scope","basis","anchors"],"additionalProperties":false});
    ToolDef {
        id: RECORD_PAPER_CONVENTIONS.into(),
        description: "Record the selected paper's notation and conventions from its actual text. Name the meaning and scope of each choice and cite exact excerpts, including local exceptions. Distinguish explicit author choices from inferences about usage, explaining each inference in rationale. Use reporting for statistical, uncertainty and experimental reporting conventions. Send sequential 1-based batches and mark the final batch done. Coverage is the model's assessment of the selected material. Explain any final partial coverage or empty extraction in limitations.".into(),
        input_schema: json!({"type":"object","properties":{
            "request_seq":{"type":"integer","minimum":1},"source_id":string(64),"batch_index":{"type":"integer","minimum":1},
            "done":{"type":"boolean"},"coverage":{"type":"string","enum":["paper","partial"]},
            "limitations":{"type":"array","maxItems":12,"items":string(500)},
            "items":{"type":"array","maxItems":32,"items":item}
        },"required":["request_seq","source_id","batch_index","done","coverage","limitations","items"],"additionalProperties":false}),
        effect: policy::EffectClass::ReadWorkspace,
        approval_scope: ApprovalScope::Once,
    }
}

/// Returns prior released `ToolDef` schemas accepted for `id` during session resume.
pub fn earlier_defs(id: &str) -> Vec<ToolDef> {
    match id {
        RECORD_PAPER_MAP => {
            let map = legacy_map_def_7c36a0e();
            let mut no_answer_index = map.clone();
            if let Some(props) = no_answer_index.input_schema["properties"].as_object_mut() {
                props.remove("answer_node_ids");
            }
            vec![map, no_answer_index]
        }
        RECORD_PAPER_SUMMARY => vec![legacy_summary_def_7c36a0e()],
        RECORD_PAPER_BRANCH => vec![legacy_branch_def_7c36a0e()],
        RECORD_PAPER_CONVENTIONS => vec![legacy_conventions_def_7c36a0e()],
        _ => Vec::new(),
    }
}

/// True iff `candidate` matches the current official `ToolDef` of one of the four recorders.
pub fn is_current_recorder_def(candidate: &ToolDef) -> bool {
    let current = match candidate.id.as_str() {
        RECORD_PAPER_MAP => def(),
        RECORD_PAPER_SUMMARY => summary_def(),
        RECORD_PAPER_BRANCH => branch_def(),
        RECORD_PAPER_CONVENTIONS => conventions_def(),
        _ => return false,
    };
    candidate.def_hash() == current.def_hash()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn earlier_defs_reproduce_golden_7c36a0e_hashes() {
        let map_hashes: Vec<String> = earlier_defs(RECORD_PAPER_MAP).iter().map(ToolDef::def_hash).collect();
        assert_eq!(map_hashes, vec![GOLDEN_MAP_7C36A0E, GOLDEN_MAP_NO_ANSWER_INDEX_7C36A0E]);
        assert_eq!(earlier_defs(RECORD_PAPER_SUMMARY)[0].def_hash(), GOLDEN_SUMMARY_7C36A0E);
        assert_eq!(earlier_defs(RECORD_PAPER_BRANCH)[0].def_hash(), GOLDEN_BRANCH_7C36A0E);
        assert_eq!(earlier_defs(RECORD_PAPER_CONVENTIONS)[0].def_hash(), GOLDEN_CONVENTIONS_7C36A0E);
        assert!(earlier_defs("fs_read").is_empty());
    }
}
