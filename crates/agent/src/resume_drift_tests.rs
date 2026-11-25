use super::check_tool_defs_drift;
use research_tools::investigation::{
    branch_def, conventions_def, def as map_def, summary_def, GOLDEN_BRANCH_7C36A0E,
    GOLDEN_CONVENTIONS_7C36A0E, GOLDEN_MAP_7C36A0E, GOLDEN_MAP_NO_ANSWER_INDEX_7C36A0E,
    GOLDEN_SUMMARY_7C36A0E, RECORD_PAPER_BRANCH, RECORD_PAPER_CONVENTIONS, RECORD_PAPER_MAP,
    RECORD_PAPER_SUMMARY,
};
use serde_json::json;

#[test]
fn earlier_map_tool_can_gain_only_its_optional_answer_index() {
    let current = research_tools::investigation::def();
    let mut earlier = current.clone();
    earlier.input_schema["properties"]
        .as_object_mut()
        .unwrap()
        .remove("answer_node_ids");
    let recorded = json!({"tool_defs":[{"id":earlier.id,"def_hash":earlier.def_hash()}]});
    assert!(check_tool_defs_drift("s-map", &recorded, std::slice::from_ref(&current)).is_ok());
    let mut changed = current;
    changed.input_schema["properties"]["nodes"]["type"] = json!("string");
    assert!(check_tool_defs_drift("s-map", &recorded, &[changed]).is_err());
}

#[test]
fn golden_7c36a0e_recorder_hashes_accepted() {
    let expected = vec![branch_def(), conventions_def(), map_def(), summary_def()];
    let recorded = json!({
        "tool_defs": [
            {"id": RECORD_PAPER_BRANCH, "def_hash": GOLDEN_BRANCH_7C36A0E},
            {"id": RECORD_PAPER_CONVENTIONS, "def_hash": GOLDEN_CONVENTIONS_7C36A0E},
            {"id": RECORD_PAPER_MAP, "def_hash": GOLDEN_MAP_7C36A0E},
            {"id": RECORD_PAPER_SUMMARY, "def_hash": GOLDEN_SUMMARY_7C36A0E},
        ]
    });
    assert!(check_tool_defs_drift("s-golden", &recorded, &expected).is_ok());
}

#[test]
fn answer_index_shim_kept() {
    let current = map_def();
    let recorded_7c36a0e_no_idx = json!({
        "tool_defs": [{"id": RECORD_PAPER_MAP, "def_hash": GOLDEN_MAP_NO_ANSWER_INDEX_7C36A0E}]
    });
    assert!(check_tool_defs_drift("s-shim", &recorded_7c36a0e_no_idx, std::slice::from_ref(&current)).is_ok());
    let mut current_no_idx = current.clone();
    current_no_idx.input_schema["properties"]
        .as_object_mut()
        .unwrap()
        .remove("answer_node_ids");
    let recorded_current_no_idx = json!({
        "tool_defs": [{"id": RECORD_PAPER_MAP, "def_hash": current_no_idx.def_hash()}]
    });
    assert!(check_tool_defs_drift("s-shim2", &recorded_current_no_idx, &[current]).is_ok());
}

#[test]
fn modified_schema_rejected() {
    let recorded = json!({
        "tool_defs": [
            {"id": RECORD_PAPER_BRANCH, "def_hash": GOLDEN_BRANCH_7C36A0E},
            {"id": RECORD_PAPER_CONVENTIONS, "def_hash": GOLDEN_CONVENTIONS_7C36A0E},
            {"id": RECORD_PAPER_MAP, "def_hash": GOLDEN_MAP_7C36A0E},
            {"id": RECORD_PAPER_SUMMARY, "def_hash": GOLDEN_SUMMARY_7C36A0E},
        ]
    });
    // Mutating any expected recorder schema must be rejected even against golden 7c36a0e hashes.
    let mut bad_summary = summary_def();
    bad_summary.input_schema["properties"]["what"]["type"] = json!("string");
    let expected = vec![branch_def(), conventions_def(), map_def(), bad_summary];
    assert!(check_tool_defs_drift("s-mod", &recorded, &expected).is_err());

    // An unknown recorded hash for a recorder must also be rejected.
    let bogus = json!({
        "tool_defs": [{"id": RECORD_PAPER_MAP, "def_hash": "0".repeat(64)}]
    });
    assert!(check_tool_defs_drift("s-bogus", &bogus, &[map_def()]).is_err());
}

#[test]
fn read_paper_lines_may_be_added() {
    let mut expected = vec![
        research_tools::paper_lines::def(),
        map_def(),
    ];
    expected.sort_by(|a, b| a.id.cmp(&b.id));
    let recorded = json!({
        "tool_defs": [{"id": RECORD_PAPER_MAP, "def_hash": GOLDEN_MAP_7C36A0E}]
    });
    assert!(check_tool_defs_drift("s-add", &recorded, &expected).is_ok());
}

#[test]
fn reordered_defs_rejected() {
    let expected = vec![branch_def(), map_def()];
    let recorded = json!({
        "tool_defs": [
            {"id": RECORD_PAPER_MAP, "def_hash": GOLDEN_MAP_7C36A0E},
            {"id": RECORD_PAPER_BRANCH, "def_hash": GOLDEN_BRANCH_7C36A0E},
        ]
    });
    assert!(check_tool_defs_drift("s-order", &recorded, &expected).is_err());
}
