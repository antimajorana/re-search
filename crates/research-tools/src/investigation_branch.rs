use super::anchor::AnchorReport;
use super::{bounded, validate_map, verify_graph_anchors, Edge, Node, PaperMap, SourceIndex};
use ledger::Event;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use tools::ToolDef;

pub const RECORD_PAPER_BRANCH: &str = "record_paper_branch";
const MAX_NEW_NODES: usize = 96;
const MAX_NEW_EDGES: usize = 256;
const MAX_TOTAL_NODES: usize = 1024;
const MAX_TOTAL_EDGES: usize = 4096;

pub fn paper_branch_instructions() -> &'static str {
    concat!(
        "Explore the selected statement in the saved paper map. Read the requested focus and existing graph first. Trace its complete supporting argument within that focus back to the paper's assumptions, evidence and methods. ",
        "Use read_paper_lines and search_paper_passages to inspect the actual definitions, derivations, equations, figures, tables, or experimental controls and uncertainties on which this result depends. Respect the selected physical pages, and distinguish the authors' results from later commentary. ",
        "Cite every anchor with {source_id, start_line, end_line, quote, kind} (plus label for printed equation, figure, or table labels only; never use bare section headings like Abstract or Introduction). Give a short verbatim phrase (16–300 chars) copied from the cited lines as quote; the app fills in the exact lines. Omit path for the primary paper; include path for external reference sources. ",
        "An arrow points from a contribution to the downstream claim that uses it. Give a source-anchored reason for each arrow; label author-stated links explicit and your interpretation inferred. Abstract theory and empirical observations need different supporting details. ",
        "Reuse the IDs of existing statements where they already express a prerequisite. Supply only NEW nodes with main=false, and NEW edges along paths leading to the selected statement. Preserve previously recorded statements, edges and corrections. Where the argument relies on an external theorem or result, identify it and cite the passage in the primary paper. Let the reader choose which external work to open or investigate. ",
        "Use record_paper_branch once you have traced the relevant chain. State any specific gaps, unreadable equations, unproved assumptions, or remaining dependencies in limitations. Return empty nodes and edges with a concrete limitation when the paper provides no further support. Do not start independent computations or add unrelated paper branches."
    )
}

pub fn branch_def() -> ToolDef {
    let mut def = super::def();
    let base = &def.input_schema["properties"];
    let mut nodes = base["nodes"].clone();
    nodes["minItems"] = json!(0);
    nodes["maxItems"] = json!(MAX_NEW_NODES);
    let mut edges = base["edges"].clone();
    edges["maxItems"] = json!(MAX_NEW_EDGES);
    def.id = RECORD_PAPER_BRANCH.into();
    def.description = "Save the source-grounded dependencies for the currently selected paper result and reading question. Supply new nodes and new edges pointing from evidence or assumptions towards that result, citing each anchor with start_line, end_line, and a short verbatim quote. Use limitations for unsupported parts of the argument.".into();
    def.input_schema = json!({"type":"object","properties":{
        "request_seq":{"type":"integer","minimum":1,"description":"The request number in the current branch instructions."},
        "source_id":base["source_id"],"nodes":nodes,"edges":edges,"limitations":base["limitations"]
    },"required":["request_seq","source_id","nodes","edges","limitations"],"additionalProperties":false});
    def
}

/// Materialize the original map and its immutable branches without changing its ledger identity.
pub fn expanded_map(events: &[Event], base: &Event) -> Value {
    let mut map = base.payload.clone();
    let mut branches = Vec::new();
    for event in events.iter().filter(|e| {
        e.seq > base.seq
            && e.kind == "investigation.branch"
            && e.payload["map_seq"].as_u64() == Some(base.seq)
            && e.payload["source_id"] == base.payload["source_id"]
    }) {
        let branch = &event.payload;
        for field in ["nodes", "edges"] {
            if let (Some(existing), Some(added)) =
                (map[field].as_array_mut(), branch[field].as_array())
            {
                existing.extend(added.iter().cloned());
            }
        }
        let node_ids: Vec<_> = branch["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|node| &node["id"])
            .collect();
        branches.push(json!({"seq":event.seq,"request_seq":branch["request_seq"],"node_id":branch["node_id"],"question":branch["question"],"limitations":branch["limitations"],"node_ids":node_ids}));
    }
    map["map_seq"] = json!(base.seq);
    map["branches"] = json!(branches);
    map
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Branch {
    request_seq: u64,
    source_id: String,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    limitations: Vec<String>,
}

fn check_paths(
    target: &str,
    prior_edges: &[Edge],
    nodes: &[Node],
    edges: &[Edge],
) -> Result<(), String> {
    let mut parents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for edge in prior_edges.iter().chain(edges) {
        parents.entry(&edge.to).or_default().push(&edge.from);
    }
    let mut seen = BTreeSet::new();
    let mut pending = vec![target];
    while let Some(node) = pending.pop() {
        if seen.insert(node) {
            pending.extend(parents.get(node).into_iter().flatten().copied());
        }
    }
    if nodes.iter().any(|node| !seen.contains(node.id.as_str()))
        || edges.iter().any(|edge| !seen.contains(edge.to.as_str()))
    {
        return Err(
            "Every new statement and supporting link needs a directed path to the selected result."
                .into(),
        );
    }
    Ok(())
}

pub(super) fn payload(
    args: &Value,
    index: &mut SourceIndex<'_>,
    history: &[Event],
    request: &Event,
    report: &mut AnchorReport,
) -> Result<Value, String> {
    let mut branch: Branch =
        serde_json::from_value(args.clone()).map_err(|e| format!("Invalid paper branch: {e}"))?;
    if branch.request_seq != request.seq {
        return Err("This branch request is no longer active. Use the request number for the current reading focus.".into());
    }
    if branch.nodes.len() > MAX_NEW_NODES
        || branch.edges.len() > MAX_NEW_EDGES
        || branch.limitations.len() > 12
        || branch.limitations.iter().any(|x| !bounded(x, 500))
        || (branch.edges.is_empty() && branch.nodes.is_empty() && branch.limitations.is_empty())
    {
        return Err(
            "Supply a bounded dependency graph or describe the specific source limitation.".into(),
        );
    }
    if branch.nodes.iter().any(|node| node.main) {
        return Err(
            "Use main=false for new branch nodes. Main results belong to the original overview."
                .into(),
        );
    }
    let base = history
        .iter()
        .rev()
        .find(|e| e.kind == "investigation.map")
        .filter(|e| {
            request.payload["map_seq"].as_u64() == Some(e.seq)
                && e.payload["source_id"] == branch.source_id
        })
        .ok_or("This branch request no longer names the current paper map.")?;
    let target = request.payload["node_id"]
        .as_str()
        .ok_or("The branch request is missing its selected result.")?;
    let question = request.payload["question"]
        .as_str()
        .filter(|q| bounded(q, 2000))
        .ok_or("The branch request is missing its reading question.")?;
    // Use the graph at request creation, so an identical retry can recognize the saved receipt.
    let before = &history[..history.partition_point(|e| e.seq < request.seq)];
    let prior = expanded_map(before, base);
    let nodes: Vec<Node> = serde_json::from_value(prior["nodes"].clone())
        .map_err(|e| format!("Invalid saved map nodes: {e}"))?;
    let mut edges: Vec<Edge> = serde_json::from_value(prior["edges"].clone())
        .map_err(|e| format!("Invalid saved map edges: {e}"))?;
    if !nodes.iter().any(|node| node.id == target) {
        return Err("The selected result does not exist in this map.".into());
    }
    check_paths(target, &edges, &branch.nodes, &branch.edges)?;
    // Historical maps allowed two edge kinds between the same pair. Check their common
    // dependency once, retaining a citation where that authorizes a reference node.
    edges.sort_by_key(|edge| edge.kind != "cites");
    let mut previous_pairs = BTreeSet::new();
    edges.retain(|edge| previous_pairs.insert((edge.from.clone(), edge.to.clone())));
    let start_node = nodes.len();
    let start_edge = edges.len();
    let mut union = PaperMap {
        source_id: branch.source_id,
        title: "Branch dependencies".into(),
        coverage: "paper".into(),
        limitations: Vec::new(),
        answer_node_ids: None,
        nodes,
        edges,
    };
    union.nodes.append(&mut branch.nodes);
    union.edges.append(&mut branch.edges);
    validate_map(&union, MAX_TOTAL_NODES, MAX_TOTAL_EDGES)?;
    verify_graph_anchors(
        &union.source_id,
        &mut union.nodes[start_node..],
        &mut union.edges[start_edge..],
        index,
        report,
    )?;
    Ok(
        json!({"source_id":union.source_id,"map_seq":base.seq,"node_id":target,"question":question,"nodes":&union.nodes[start_node..],"edges":&union.edges[start_edge..],"limitations":branch.limitations}),
    )
}
