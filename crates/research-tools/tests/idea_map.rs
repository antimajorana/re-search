//! Integration tests for Explore X11 `record_idea_map` and F6 `GraphView` projection.
//!
//! Covers Rules C4, C5, C6, S4, V8 and F6 edge direction / solid-dashed rules:
//! - `idea_map_refuses_inline_quotes`
//! - `idea_map_node_evidence_rules`
//! - `idea_map_rejects_evidence_from_other_session`
//! - `idea_open_question_needs_author_statement`
//! - `idea_node_ids_stable_across_remap`
//! - `idea_map_graph_view_edge_directions_and_styles`
//! - `idea_prose_rejects_absolute_phrases`
//! - `idea_prose_allows_first_calculation`

use ledger::Event;
use research_tools::idea_map::{
    check_explore_prose, project_graph_view, record_idea_map_schema, validate_record_idea_map,
    MapNodeKind, MapNodeStatus, EVENT_IDEA_CITATION_NET, EVENT_IDEA_MAP,
};
use research_tools::quote_core::{EVIDENCE_ABSTRACT_EVENT, EVIDENCE_QUOTE_EVENT};
use serde_json::{json, Value};
use std::collections::BTreeSet;

const IDEA_TEXT: &str = "Does Krylov complexity grow linearly in a monitored Majorana chain?";

fn ev(seq: u64, kind: &str, payload: Value) -> Event {
    Event {
        session: "s-map".into(),
        seq,
        ts_ms: seq as i64 * 1000,
        kind: kind.into(),
        schema_version: 1,
        payload,
        prev_hash: String::new(),
        event_hash: String::new(),
    }
}

fn base_session_events() -> Vec<Event> {
    vec![
        ev(1, "idea.stated", json!({ "rev": 1, "text": IDEA_TEXT })),
        ev(2, "idea.candidates", json!({
            "round": 1,
            "candidates": [
                { "key": "arxiv:2101.00001", "year": 2021, "title": "Paper A" },
                { "key": "arxiv:2401.00002", "year": 2024, "title": "Paper B" }
            ]
        })),
        ev(3, EVIDENCE_QUOTE_EVENT, json!({
            "paper_key": "arxiv:2101.00001", "role": "defines",
            "quote": "Krylov complexity is the average position on the Krylov chain."
        })),
        ev(4, EVIDENCE_QUOTE_EVENT, json!({
            "paper_key": "arxiv:2101.00001", "role": "states_result",
            "quote": "Lanczos coefficients grow linearly with slope alpha."
        })),
        ev(5, EVIDENCE_QUOTE_EVENT, json!({
            "paper_key": "arxiv:2101.00001", "role": "poses_open_question",
            "quote": "Whether monitored free fermions retain linear growth remains open."
        })),
        ev(6, EVIDENCE_ABSTRACT_EVENT, json!({
            "paper_key": "arxiv:2401.00002",
            "quote": "Abstract discussing operator spreading in open fermion chains."
        })),
        ev(7, EVENT_IDEA_CITATION_NET, json!({
            "edges": [{ "from": "arxiv:2401.00002", "to": "arxiv:2101.00001" }]
        })),
    ]
}

#[test]
fn idea_map_refuses_inline_quotes() {
    // Rule C4: JSON schema must not contain quote/excerpt/line/start_line/end_line/path.
    let schema_str = record_idea_map_schema().to_string();
    for forbidden in ["\"quote\"", "\"excerpt\"", "\"line\"", "\"start_line\"", "\"end_line\"", "\"path\""] {
        assert!(!schema_str.contains(forbidden), "schema must not expose {forbidden}");
    }

    let events = base_session_events();
    let sc = vec!["sc-1".into()];
    for bad_field in ["quote", "excerpt", "line", "start_line", "end_line", "path"] {
        let mut node = json!({
            "temp_id": "n1", "kind": "known_result", "label": "Linear growth",
            "statement": "Lanczos coefficients grow linearly.",
            "evidence": ["xe-4"], "relation_to_idea": "same"
        });
        node.as_object_mut().unwrap().insert(bad_field.into(), json!("inline data"));
        let res = validate_record_idea_map(&events, 1, IDEA_TEXT, 1, &sc, None, &json!({
            "nodes": [node], "edges": [], "overlaps": []
        }));
        assert!(res.is_err(), "Rule C4 must reject inline field `{bad_field}`");
    }
}

#[test]
fn idea_map_node_evidence_rules() {
    let events = base_session_events();
    let sc = vec!["sc-1".into()];

    // Mix valid nodes of all 5 kinds + invalid abstract-only def/known_result/open_question.
    let raw = json!({
        "nodes": [
            {
                "temp_id": "c1", "kind": "concept", "label": "Krylov complexity",
                "statement": "Spread complexity in the Krylov basis.",
                "evidence": [], "idea_quote": "Krylov complexity",
                "relation_to_idea": "same", "aliases": ["spread complexity"]
            },
            {
                "temp_id": "d1", "kind": "definition", "label": "Krylov chain",
                "statement": "Definition of Krylov complexity.",
                "evidence": ["xe-3"], "relation_to_idea": "ingredient", "symbol": "K(t)"
            },
            {
                "temp_id": "k1", "kind": "known_result", "label": "Linear Lanczos bound",
                "statement": "Lanczos coefficients grow linearly.",
                "evidence": ["xe-4"], "relation_to_idea": "same", "subclaim_ids": ["sc-1"]
            },
            {
                "temp_id": "w1", "kind": "related_work", "label": "Paper B (abstract)",
                "statement": "Open fermion chain study.",
                "evidence": ["xa-6"], "relation_to_idea": "background"
            },
            {
                "temp_id": "bad_def", "kind": "definition", "label": "Abstract def",
                "statement": "Abstract-only definition should be dropped.",
                "evidence": ["xa-6"], "relation_to_idea": "background"
            },
            {
                "temp_id": "bad_res", "kind": "known_result", "label": "Abstract result",
                "statement": "Abstract-only result should be dropped.",
                "evidence": ["xa-6"], "relation_to_idea": "background"
            }
        ],
        "edges": [],
        "overlaps": []
    });
    let map = validate_record_idea_map(&events, 1, IDEA_TEXT, 1, &sc, None, &raw).unwrap();
    assert_eq!(map.nodes.len(), 4);
    assert_eq!(map.dropped.len(), 2, "failing abstract-only nodes must be dropped with reasons");
    assert!(map.nodes.iter().any(|n| n.kind == MapNodeKind::Definition && n.id == "d-1" && n.papers == vec!["arxiv:2101.00001"]));
    let rw = map.nodes.iter().find(|n| n.kind == MapNodeKind::RelatedWork).unwrap();
    assert_eq!(rw.basis.as_deref(), Some("abstract"));
    assert_eq!(rw.year, Some(2024));

    // If ZERO nodes survive validation, the entire record is refused (Rule C5).
    let all_bad = json!({
        "nodes": [{
            "temp_id": "bad_only", "kind": "known_result", "label": "Abstract only",
            "statement": "Cannot rest on abstract.",
            "evidence": ["xa-6"], "relation_to_idea": "background"
        }],
        "edges": [], "overlaps": []
    });
    assert!(validate_record_idea_map(&events, 1, IDEA_TEXT, 1, &sc, None, &all_bad).is_err());
}

#[test]
fn idea_map_rejects_evidence_from_other_session() {
    let events = base_session_events();
    let sc = vec!["sc-1".into()];
    let raw = json!({
        "nodes": [{
            "temp_id": "k_other", "kind": "known_result", "label": "Foreign quote",
            "statement": "Cites xe-999 from another session.",
            "evidence": ["xe-999"], "relation_to_idea": "same"
        }],
        "edges": [], "overlaps": []
    });
    let err = validate_record_idea_map(&events, 1, IDEA_TEXT, 1, &sc, None, &raw).unwrap_err();
    assert!(err.iter().any(|e| e.contains("xe-999") && e.contains("this session")));
}

#[test]
fn idea_open_question_needs_author_statement() {
    let events = base_session_events();
    let sc = vec!["sc-1".into()];

    // Wrong role (`states_result` xe-4 instead of `poses_open_question` xe-5) is refused.
    let bad_q = json!({
        "nodes": [{
            "temp_id": "q_bad", "kind": "open_question", "label": "Inferred gap",
            "statement": "Model-inferred open question.",
            "evidence": ["xe-4"], "relation_to_idea": "same"
        }],
        "edges": [], "overlaps": []
    });
    assert!(validate_record_idea_map(&events, 1, IDEA_TEXT, 1, &sc, None, &bad_q).is_err());

    // Author statement (`poses_open_question` xe-5) succeeds and app-fills stated_year & later_work_checked.
    let good_q = json!({
        "nodes": [{
            "temp_id": "q_ok", "kind": "open_question", "label": "Monitored growth",
            "statement": "Whether monitored free fermions retain linear growth.",
            "evidence": ["xe-5"], "relation_to_idea": "same"
        }],
        "edges": [], "overlaps": []
    });
    let map = validate_record_idea_map(&events, 1, IDEA_TEXT, 1, &sc, None, &good_q).unwrap();
    assert_eq!(map.nodes.len(), 1);
    assert_eq!(map.nodes[0].id, "q-1");
    assert_eq!(map.nodes[0].stated_year, Some(2021));
    assert_eq!(map.nodes[0].later_work_checked, Some(true));
}

#[test]
fn idea_node_ids_stable_across_remap() {
    let mut events = base_session_events();
    let sc = vec!["sc-1".into()];

    // Duplicate (kind, first_evidence_id) in one call is rejected (Rule S4).
    let dup = json!({
        "nodes": [
            { "temp_id": "a", "kind": "known_result", "label": "A", "statement": "First.", "evidence": ["xe-4"], "relation_to_idea": "same" },
            { "temp_id": "b", "kind": "known_result", "label": "B", "statement": "Second.", "evidence": ["xe-4"], "relation_to_idea": "same" }
        ],
        "edges": [], "overlaps": []
    });
    assert!(validate_record_idea_map(&events, 1, IDEA_TEXT, 1, &sc, None, &dup).is_err());

    // Map 1 creates d-1 (xe-3) and k-1 (xe-4).
    let map1 = validate_record_idea_map(&events, 1, IDEA_TEXT, 1, &sc, None, &json!({
        "nodes": [
            { "temp_id": "d", "kind": "definition", "label": "Def", "statement": "Def.", "evidence": ["xe-3"], "relation_to_idea": "ingredient" },
            { "temp_id": "k", "kind": "known_result", "label": "Res", "statement": "Res.", "evidence": ["xe-4"], "relation_to_idea": "same" }
        ],
        "edges": [], "overlaps": []
    })).unwrap();
    assert_eq!(map1.nodes[0].id, "d-1");
    assert_eq!(map1.nodes[1].id, "k-1");

    events.push(ev(8, EVENT_IDEA_MAP, serde_json::to_value(&map1).unwrap()));
    // User adds a note on d-1.
    events.push(ev(9, "idea.note", json!({ "target_kind": "node", "target_id": "d-1", "action": "pin", "comment": "Keep this definition" })));

    // Map 2 omits d-1 and adds q-1 alongside k-1: k-1 keeps id "k-1", and user-noted d-1 is preserved as `not_in_current_map`!
    let map2 = validate_record_idea_map(&events, 1, IDEA_TEXT, 1, &sc, None, &json!({
        "nodes": [
            { "temp_id": "k", "kind": "known_result", "label": "Res updated", "statement": "Res updated.", "evidence": ["xe-4"], "relation_to_idea": "same" },
            { "temp_id": "q", "kind": "open_question", "label": "Open Q", "statement": "Open Q.", "evidence": ["xe-5"], "relation_to_idea": "same" }
        ],
        "edges": [], "overlaps": []
    })).unwrap();
    let k_node = map2.nodes.iter().find(|n| n.id == "k-1").expect("k-1 id must stay stable");
    assert_eq!(k_node.status, MapNodeStatus::Active);
    let kept_d = map2.nodes.iter().find(|n| n.id == "d-1").expect("user-noted d-1 must never be deleted");
    assert_eq!(kept_d.status, MapNodeStatus::NotInCurrentMap);
    assert!(map2.nodes.iter().any(|n| n.id == "q-1"));
}

#[test]
fn idea_map_graph_view_edge_directions_and_styles() {
    let events = base_session_events();
    let sc = vec!["sc-1".into()];
    let raw = json!({
        "nodes": [
            { "temp_id": "c1", "kind": "concept", "label": "C1", "statement": "Concept.", "evidence": ["xe-3"], "relation_to_idea": "same" },
            { "temp_id": "d1", "kind": "definition", "label": "D1", "statement": "Def.", "evidence": ["xe-3"], "relation_to_idea": "ingredient" },
            { "temp_id": "k1", "kind": "known_result", "label": "K1", "statement": "Result 1.", "evidence": ["xe-4"], "relation_to_idea": "same" },
            { "temp_id": "q1", "kind": "open_question", "label": "Q1", "statement": "Question.", "evidence": ["xe-5"], "relation_to_idea": "same" },
            { "temp_id": "w1", "kind": "related_work", "label": "Paper A", "statement": "Paper A.", "evidence": ["xe-4"], "relation_to_idea": "same", "paper_key": "arxiv:2101.00001" },
            { "temp_id": "w2", "kind": "related_work", "label": "Paper B", "statement": "Paper B.", "evidence": ["xa-6"], "relation_to_idea": "background", "paper_key": "arxiv:2401.00002" }
        ],
        "edges": [
            { "from": "d1", "to": "c1", "kind": "defines", "basis": "evidence", "evidence": "xe-3" },
            { "from": "k1", "to": "d1", "kind": "uses", "basis": "evidence", "evidence": "xe-4" },
            { "from": "k1", "to": "c1", "kind": "special_case_of", "basis": "model_inferred", "reason": "Restricts to monitored regime." },
            { "from": "k1", "to": "w1", "kind": "generalises", "basis": "evidence", "evidence": "xe-4" },
            { "from": "k1", "to": "q1", "kind": "answers", "basis": "evidence", "evidence": "xe-4" },
            { "from": "w1", "to": "q1", "kind": "leaves_open", "basis": "evidence", "evidence": "xe-5" },
            { "from": "k1", "to": "w2", "kind": "contradicts", "basis": "model_inferred", "reason": "Opposite scaling regime." },
            { "from": "c1", "to": "d1", "kind": "same_as", "basis": "evidence", "evidence": "xe-3" }
        ],
        "overlaps": []
    });
    let map = validate_record_idea_map(&events, 1, IDEA_TEXT, 1, &sc, None, &raw).unwrap();
    // App synthesized `cites` edge from idea.citation_net: w2 (Paper B) cites w1 (Paper A).
    assert!(map.edges.iter().any(|e| e.kind.as_str() == "cites" && e.from == "w-2" && e.to == "w-1"));

    let gv = project_graph_view(&map, &BTreeSet::new());
    let edges = gv["edges"].as_array().unwrap();
    let find_edge = |k: &str| edges.iter().find(|e| e["kind"] == k).unwrap();

    // Fixed direction table (explore.md 4.1): src = relied-on, dst = relies-on-it.
    assert_eq!((find_edge("defines")["src"].as_str(), find_edge("defines")["dst"].as_str(), find_edge("defines")["style"].as_str()), (Some("d-1"), Some("c-1"), Some("solid")));
    assert_eq!((find_edge("uses")["src"].as_str(), find_edge("uses")["dst"].as_str(), find_edge("uses")["style"].as_str()), (Some("d-1"), Some("k-1"), Some("solid")));
    assert_eq!((find_edge("special_case_of")["src"].as_str(), find_edge("special_case_of")["dst"].as_str(), find_edge("special_case_of")["style"].as_str()), (Some("c-1"), Some("k-1"), Some("dashed")));
    assert_eq!((find_edge("generalises")["src"].as_str(), find_edge("generalises")["dst"].as_str()), (Some("k-1"), Some("w-1")));
    assert_eq!((find_edge("answers")["src"].as_str(), find_edge("answers")["dst"].as_str()), (Some("q-1"), Some("k-1")));
    assert_eq!((find_edge("leaves_open")["src"].as_str(), find_edge("leaves_open")["dst"].as_str()), (Some("w-1"), Some("q-1")));
    // cites: w-2 cites w-1 -> src = w-1 (cited), dst = w-2 (citing); dashed because w-2 is abstract-only!
    assert_eq!((find_edge("cites")["src"].as_str(), find_edge("cites")["dst"].as_str(), find_edge("cites")["basis"].as_str(), find_edge("cites")["style"].as_str()), (Some("w-1"), Some("w-2"), Some("code"), Some("dashed")));
    assert_eq!(find_edge("contradicts")["symmetric"], true);
    assert_eq!(find_edge("contradicts")["directed"], false);
    assert_eq!(find_edge("same_as")["symmetric"], true);
    assert_eq!(find_edge("same_as")["directed"], false);
}

#[test]
fn idea_prose_rejects_absolute_phrases() {
    let banned_samples = [
        "This calculation has not been done anywhere.",
        "This limit has never been studied.",
        "There is no prior work on this model.",
        "Nobody has computed the Krylov slope.",
        "We are the first to derive this bound.",
        "Computed for the first time in 1D.",
        "A completely novel mechanism.",
        "An unexplored parameter regime.",
    ];
    for s in banned_samples {
        assert!(check_explore_prose(s).is_err(), "Rule V8 must reject: {s}");
    }

    // Also rejected inside record_idea_map (`statement`, `reason`, `difference`).
    let events = base_session_events();
    let sc = vec!["sc-1".into()];
    let bad_map = json!({
        "nodes": [{
            "temp_id": "k1", "kind": "known_result", "label": "Bound",
            "statement": "This is a novel derivation with no prior work.",
            "evidence": ["xe-4"], "relation_to_idea": "same"
        }],
        "edges": [], "overlaps": []
    });
    assert!(validate_record_idea_map(&events, 1, IDEA_TEXT, 1, &sc, None, &bad_map).is_err());
}

#[test]
fn idea_prose_allows_first_calculation() {
    let allowed = [
        "A concrete first calculation is to evaluate b_1 and b_2 for N=4.",
        "First-order perturbation theory gives the leading term.",
        "The new notation simplifies the boundary recurrence.",
    ];
    for s in allowed {
        assert!(check_explore_prose(s).is_ok(), "Rule V8 must allow: {s}");
    }

    // Audit regressions (HIGH #2 & #3): model-supplied `metadata` edges and empty-evidence overlaps are dropped.
    let events = base_session_events();
    let sc = vec!["sc-1".into()];
    let map = validate_record_idea_map(&events, 1, IDEA_TEXT, 1, &sc, None, &json!({
        "nodes": [
            { "temp_id": "d1", "kind": "definition", "label": "D1", "statement": "Def.", "evidence": ["xe-3"], "relation_to_idea": "ingredient" },
            { "temp_id": "k1", "kind": "known_result", "label": "K1", "statement": "Res.", "evidence": ["xe-4"], "relation_to_idea": "same" }
        ],
        "edges": [
            { "from": "k1", "to": "d1", "kind": "uses", "basis": "metadata" }
        ],
        "overlaps": [
            { "subclaim_id": "sc-1", "paper_key": "arxiv:2101.00001", "relation": "same", "evidence": [], "difference": "Empty evidence." }
        ]
    })).unwrap();
    assert!(map.edges.is_empty(), "model-supplied metadata edge must be dropped");
    assert!(map.overlaps.is_empty(), "empty-evidence overlap must be dropped");
    assert_eq!(map.dropped.len(), 2);
}
