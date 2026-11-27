use super::*;
use research_tools::paper_key::PaperKey;
use research_tools::skeleton::{
    NumberBasis, SkConfidence, SkNode, SkNodeKind, SkeletonBuilder, SkeletonQuality, Span,
};

fn make_event(seq: u64, kind: &str, payload: Value) -> Event {
    Event {
        session: "s-legacy-test".to_string(),
        seq,
        ts_ms: seq as i64 * 100,
        kind: kind.to_string(),
        schema_version: 1,
        payload,
        prev_hash: String::new(),
        event_hash: format!("h-{seq}"),
    }
}

fn make_sk_node(
    tag: &str,
    kind: SkNodeKind,
    order: usize,
    start_line: usize,
    end_line: usize,
) -> SkNode {
    SkNode {
        tag: Some(tag.to_string()),
        kind,
        number: None,
        number_basis: Some(NumberBasis::Computed),
        env_name: None,
        title: None,
        label: None,
        parent: None,
        order,
        spans: vec![Span::new("paper.tex", start_line, end_line)],
        proof_of: None,
        head_cites: Vec::new(),
        intro_refs: false,
        fp: format!("fp-{order:012}"),
        words: 10,
        eq_count: 0,
        read_minutes: 0.2,
        confidence: SkConfidence::TexEnv,
    }
}

fn make_skeleton(nodes: Vec<SkNode>) -> PaperSkeleton {
    PaperSkeleton {
        schema: 1,
        source_id: "src-1".to_string(),
        source_seq: 2,
        builder: SkeletonBuilder::Tex1,
        files_root_sha256: "root".to_string(),
        paper_key: PaperKey::from_file_bytes(b"src-1"),
        version: None,
        quality: SkeletonQuality::Full,
        quality_reason: None,
        nodes,
        edges: Vec::new(),
        bib: Vec::new(),
        symbols: Vec::new(),
        macros: Vec::new(),
        stats: Vec::new(),
        carry: None,
    }
}

#[test]
fn pure_v2_session_has_no_legacy_and_null_embed() {
    let events = vec![
        make_event(1, "investigation.started", json!({})),
        make_event(2, "investigation.paper", json!({ "source_id": "src-1" })),
        make_event(3, "investigation.skeleton", json!({ "source_id": "src-1" })),
        make_event(4, "investigation.notes", json!({ "tag": "n-1", "note": "reader note" })),
    ];
    let skel = make_skeleton(vec![make_sk_node("n-1", SkNodeKind::Theorem, 0, 1, 10)]);
    let proj = adapt_legacy_investigation(&events, Some(&skel));
    assert!(!proj.has_legacy);
    assert!(!proj.fallback_old_view);
    assert!(proj.nodes.is_empty());

    let mut mv = json!({ "skeleton_seq": 3, "nodes": [{ "id": "n-1" }], "edges": [] });
    embed_legacy_into_mapview(&mut mv, &events, Some(&skel));
    assert_eq!(mv["legacy"], Value::Null);
    assert!(mv["nodes"][0].get("earlier_map").is_none());
}

#[test]
fn attaches_to_innermost_span_with_order_tie_break_and_unplaced() {
    let mut sec = make_sk_node("n-sec", SkNodeKind::Section, 0, 1, 30);
    sec.spans = vec![Span::new("paper.tex", 1, 30)];
    let thm = make_sk_node("n-thm", SkNodeKind::Theorem, 1, 5, 12);
    let tied_a = make_sk_node("n-eq-a", SkNodeKind::Equation, 2, 15, 15);
    let tied_b = make_sk_node("n-eq-b", SkNodeKind::Equation, 3, 15, 15);
    let skel = make_skeleton(vec![sec, thm, tied_a, tied_b]);

    let events = vec![
        make_event(1, "investigation.started", json!({})),
        make_event(
            10,
            "investigation.map",
            json!({
                "source_id": "src-1",
                "title": "Synthetic Map",
                "coverage": "paper",
                "limitations": [],
                "nodes": [
                    {
                        "id": "inside_thm",
                        "sentence": "Inside theorem nested inside section.",
                        "kind": "result",
                        "main": true,
                        "anchors": [{ "source_id": "src-1", "path": "paper.tex", "start_line": 7, "end_line": 8, "kind": "passage" }]
                    },
                    {
                        "id": "tied_span",
                        "sentence": "Tied single-line span breaks deterministically by SkNode.order.",
                        "kind": "equation",
                        "main": false,
                        "anchors": [{ "source_id": "src-1", "path": "paper.tex", "start_line": 15, "end_line": 15, "kind": "equation" }]
                    },
                    {
                        "id": "pre_pointer",
                        "sentence": "Pre-pointer anchor lacking path/lines goes to Unplaced.",
                        "kind": "claim",
                        "main": false,
                        "anchors": [{ "source_id": "src-1", "excerpt": "Old quote only", "kind": "passage" }]
                    },
                    {
                        "id": "outside_span",
                        "sentence": "Line outside skeleton spans goes to Unplaced.",
                        "kind": "claim",
                        "main": false,
                        "anchors": [{ "source_id": "src-1", "path": "paper.tex", "start_line": 99, "end_line": 99, "kind": "passage" }]
                    }
                ],
                "edges": [
                    {
                        "from": "tied_span",
                        "to": "inside_thm",
                        "kind": "proof",
                        "basis": "explicit",
                        "reason": "Used in proof.",
                        "anchor": { "source_id": "src-1", "path": "paper.tex", "start_line": 15, "end_line": 15, "kind": "passage" }
                    }
                ]
            }),
        ),
        make_event(
            14,
            "investigation.branch",
            json!({
                "map_seq": 10,
                "source_id": "src-1",
                "request_seq": 13,
                "node_id": "inside_thm",
                "question": "What triggers this bound?",
                "limitations": [],
                "nodes": [{
                    "id": "branch_lem",
                    "sentence": "Branched auxiliary assumption in section.",
                    "kind": "assumption",
                    "main": false,
                    "anchors": [{ "source_id": "src-1", "path": "paper.tex", "start_line": 22, "end_line": 23, "kind": "passage" }]
                }],
                "edges": [{
                    "from": "branch_lem",
                    "to": "inside_thm",
                    "kind": "depends_on",
                    "basis": "inferred",
                    "reason": "Branch prerequisite.",
                    "anchor": { "source_id": "src-1", "path": "paper.tex", "start_line": 22, "end_line": 23, "kind": "passage" }
                }]
            }),
        ),
        make_event(
            15,
            "investigation.correction",
            json!({
                "map_seq": 10,
                "node_id": "inside_thm",
                "edge_from": "tied_span",
                "text": "Reader note: edge was conditional on k > 0."
            }),
        ),
    ];

    let proj_a = adapt_legacy_investigation(&events, Some(&skel));
    let proj_b = adapt_legacy_investigation(&events, Some(&skel));
    assert_eq!(proj_a, proj_b, "adapter output must be deterministic");
    assert!(proj_a.has_legacy);
    assert!(!proj_a.fallback_old_view);

    let thm_attached = proj_a.placed.get("n-thm").expect("inside_thm on n-thm");
    assert_eq!(thm_attached.len(), 1);
    assert_eq!(thm_attached[0].id, "legacy:10:inside_thm");
    assert_eq!(thm_attached[0].corrections.len(), 1);
    assert_eq!(thm_attached[0].edges.len(), 2);
    assert_eq!(proj_a.placed.get("n-eq-a").unwrap()[0].id, "legacy:10:tied_span");
    assert_eq!(proj_a.placed.get("n-sec").unwrap()[0].id, "legacy:14:branch_lem");

    assert_eq!(proj_a.unplaced.len(), 2);
    assert_eq!(proj_a.unplaced[0].id, "legacy:10:pre_pointer");
    assert_eq!(proj_a.unplaced[1].id, "legacy:10:outside_span");

    assert_eq!(proj_a.edges.len(), 2);
    for edge in &proj_a.edges {
        assert_eq!(edge.role, LEGACY_EDGE_ROLE);
        assert_eq!(edge.basis, LEGACY_EDGE_BASIS);
        assert!(edge.dashed);
        assert_ne!(edge.kind.to_ascii_lowercase(), "proof");
    }
    assert_eq!(proj_a.edges[0].src, "legacy:10:tied_span");
    assert_eq!(proj_a.edges[0].dst, "legacy:10:inside_thm");
    assert_eq!(proj_a.edges[1].src, "legacy:14:branch_lem");
    assert_eq!(proj_a.edges[1].dst, "legacy:10:inside_thm");

    let mut mv = json!({
        "skeleton_seq": 3,
        "nodes": [{ "id": "n-thm" }, { "id": "n-sec" }, { "id": "n-eq-a" }],
        "edges": []
    });
    embed_legacy_into_mapview(&mut mv, &events, Some(&skel));
    assert_eq!(mv["nodes"][0]["earlier_map"][0]["id"], "legacy:10:inside_thm");
    assert!(!mv.to_string().to_ascii_lowercase().contains("verified"));
}

#[test]
fn preserves_standalone_branch_summary_conventions_activities_answers_and_fallback() {
    let events = vec![
        make_event(1, "investigation.started", json!({})),
        make_event(2, "investigation.paper", json!({ "source_id": "src-1" })),
        make_event(3, "investigation.mapping_requested", json!({ "source_id": "src-1" })),
        make_event(
            4,
            "investigation.summary",
            json!({
                "source_id": "src-1",
                "request_seq": 3,
                "what": { "text": "Defines a synthetic bound.", "anchors": [] },
                "why": { "unstated": true },
                "how": { "text": "By induction on step count.", "anchors": [] }
            }),
        ),
        make_event(
            5,
            "investigation.conventions",
            json!({
                "request_seq": 3,
                "source_id": "src-1",
                "batch_index": 1,
                "done": true,
                "coverage": "full",
                "coverage_scope": "paper",
                "limitations": [],
                "items": [{ "id": "c1", "symbol": "k", "meaning": "stiffness constant", "anchors": [] }]
            }),
        ),
        make_event(
            6,
            "investigation.branch",
            json!({
                "map_seq": 99,
                "source_id": "src-1",
                "request_seq": 5,
                "node_id": "old_n",
                "question": "Standalone branch?",
                "limitations": [],
                "nodes": [{
                    "id": "b_only",
                    "sentence": "Standalone branch claim.",
                    "kind": "claim",
                    "main": false,
                    "anchors": [{ "source_id": "src-1", "path": "paper.tex", "start_line": 2, "end_line": 2, "kind": "passage" }]
                }],
                "edges": []
            }),
        ),
        make_event(
            7,
            "investigation.question_requested",
            json!({ "question": "Does it scale linearly?" }),
        ),
        make_event(
            8,
            "investigation.answer",
            json!({ "request_seq": 7, "status": "answered", "text": "Yes, for all k > 0.", "cites": [] }),
        ),
        make_event(
            9,
            "investigation.requested",
            json!({ "question": "Follow up on k." }),
        ),
        make_event(
            10,
            "investigation.result",
            json!({ "request_seq": 9, "status": "complete", "text": "Checked follow up." }),
        ),
    ];

    let proj = adapt_legacy_investigation(&events, None);
    assert!(proj.has_legacy);
    assert!(proj.fallback_old_view);
    assert!(proj.diagnostic_note.is_some());
    assert_eq!(proj.unplaced.len(), 1);
    assert_eq!(proj.unplaced[0].id, "legacy:6:b_only");
    assert_eq!(proj.summary.as_ref().unwrap()["what"]["text"], "Defines a synthetic bound.");
    assert_eq!(proj.conventions.as_ref().unwrap()["items"][0]["symbol"], "k");
    assert_eq!(proj.answers.len(), 1);
    assert_eq!(proj.answers[0]["text"], "Yes, for all k > 0.");
    assert_eq!(proj.activities.len(), 1);
    assert_eq!(proj.activities[0]["result"], "Checked follow up.");
    assert!(!serde_json::to_string(&proj).unwrap().to_ascii_lowercase().contains("verified"));
}
