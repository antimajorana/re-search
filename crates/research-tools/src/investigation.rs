//! Source-grounded claim maps for Investigation sessions.
use crate::ResearchEnv;
use ledger::Ledger;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use tools::{ApprovalScope, Registry, ToolDef};

pub const RECORD_PAPER_MAP: &str = "record_paper_map";
#[path = "investigation_anchor.rs"]
pub(crate) mod anchor;
use anchor::{
    anchor_schema, bounded, faults_error, reject_app_only_fields, string_schema as string,
    valid_id, Anchor, AnchorFault, AnchorReport, SelectedPages, SourceIndex, MAX_TOTAL_BYTES,
};
#[path = "investigation_legacy_defs.rs"]
mod legacy_defs;
pub use legacy_defs::{
    earlier_defs, is_current_recorder_def, GOLDEN_BRANCH_7C36A0E, GOLDEN_CONVENTIONS_7C36A0E,
    GOLDEN_MAP_7C36A0E, GOLDEN_MAP_NO_ANSWER_INDEX_7C36A0E, GOLDEN_SUMMARY_7C36A0E,
};
#[path = "investigation_conventions.rs"]
mod conventions;
pub use conventions::{conventions_def, paper_conventions, RECORD_PAPER_CONVENTIONS};
#[path = "investigation_branch.rs"]
mod branch;
#[path = "investigation_prompts.rs"]
mod prompts;
#[path = "investigation_summary.rs"]
mod summary;
pub use branch::{branch_def, expanded_map, paper_branch_instructions, RECORD_PAPER_BRANCH};
pub use prompts::paper_map_instructions;
pub use summary::{summary_def, RECORD_PAPER_SUMMARY};
const NODE_KINDS: &[&str] = &[
    "assumption",
    "definition",
    "method",
    "equation",
    "prediction",
    "measurement",
    "evidence",
    "result",
    "conclusion",
    "reference",
];
const EDGE_KINDS: &[&str] = &["depends_on", "supports", "cites"];
const MAX_NODES: usize = 96;
const MAX_EDGES: usize = 256;

pub fn def() -> ToolDef {
    let strings = |n, max| json!({"type":"array","maxItems":n,"items":string(max)});
    let anchor = anchor_schema();
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
        description: "Record a paper's argument map in the current Investigation. Every node and edge needs 1-based lines and a short verbatim quote from the selected paper (the app fills in the exact cited lines). Each arrow points from supporting material to the claim that uses it. External reference text may supplement its citation in the primary paper. Coverage is the model's assessment; the recorder verifies exact anchors and DAG structure.".into(),
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

pub fn register(registry: &mut Registry, env: Arc<ResearchEnv>) {
    let conventions_env = env.clone();
    registry.register(
        conventions_def(),
        Arc::new(move |ctx: &tools::CallCtx<'_>, args: &Value| {
            let ledger = ctx.ledger.lock().map_err(|_| "ledger lock poisoned")?;
            record_conventions(&conventions_env, &ledger, ctx.session, args)
        }),
    );
    let branch_env = env.clone();
    registry.register(
        branch_def(),
        Arc::new(move |ctx: &tools::CallCtx<'_>, args: &Value| {
            let ledger = ctx.ledger.lock().map_err(|_| "ledger lock poisoned")?;
            record_branch(&branch_env, &ledger, ctx.session, args)
        }),
    );
    let summary_env = env.clone();
    registry.register(
        summary_def(),
        Arc::new(move |ctx: &tools::CallCtx<'_>, args: &Value| {
            let ledger = ctx.ledger.lock().map_err(|_| "ledger lock poisoned")?;
            record_summary(&summary_env, &ledger, ctx.session, args)
        }),
    );
    registry.register(
        def(),
        Arc::new(move |ctx: &tools::CallCtx<'_>, args: &Value| {
            let ledger = ctx.ledger.lock().map_err(|_| "ledger lock poisoned")?;
            record(&env, &ledger, ctx.session, args)
        }),
    );
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PaperMap {
    source_id: String,
    title: String,
    coverage: String,
    limitations: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    answer_node_ids: Option<Vec<String>>,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Node {
    id: String,
    sentence: String,
    kind: String,
    main: bool,
    anchors: Vec<Anchor>,
    #[serde(default)]
    questions: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Edge {
    from: String,
    to: String,
    kind: String,
    basis: String,
    reason: String,
    anchor: Anchor,
}

fn validate_map(map: &PaperMap, node_limit: usize, edge_limit: usize) -> Result<(), String> {
    if !valid_id(&map.source_id, 64)
        || !bounded(&map.title, 300)
        || !matches!(map.coverage.as_str(), "paper" | "partial")
        || map.limitations.len() > 12
        || map.limitations.iter().any(|s| !bounded(s, 500))
    {
        return Err("Provide a source ID, title, coverage and short coverage limitations.".into());
    }
    if map.coverage == "partial" && map.limitations.is_empty() {
        return Err("State which material could not be covered in this partial map.".into());
    }
    if map.nodes.is_empty()
        || map.nodes.len() > node_limit
        || map.edges.len() > edge_limit
        || !map.nodes.iter().any(|n| n.main && n.kind != "reference")
    {
        return Err(format!(
            "Record 1–{node_limit} nodes, a main scientific claim and at most {edge_limit} edges."
        ));
    }
    let mut kinds = BTreeMap::new();
    let mut indegree = BTreeMap::new();
    for n in &map.nodes {
        if !valid_id(&n.id, 48)
            || kinds.insert(n.id.as_str(), n.kind.as_str()).is_some()
            || !bounded(&n.sentence, 1400)
            || !NODE_KINDS.contains(&n.kind.as_str())
            || n.questions.len() > 4
            || n.questions.iter().any(|q| !bounded(q, 360))
        {
            return Err(format!("Invalid or duplicate claim node '{}'.", n.id));
        }
        if !(1..=4).contains(&n.anchors.len())
            || !n.anchors.iter().any(|a| a.source_id == map.source_id)
            || (n.kind != "reference" && n.anchors.iter().any(|a| a.source_id != map.source_id))
        {
            return Err(format!("Node '{}' needs an anchor in the selected paper. Only reference nodes may add another paper.",n.id));
        }
        indegree.insert(n.id.as_str(), 0_usize);
    }
    if let Some(ids) = &map.answer_node_ids {
        let mut answers = BTreeSet::new();
        if ids.len() > map.nodes.len()
            || ids
                .iter()
                .any(|id| !kinds.contains_key(id.as_str()) || !answers.insert(id))
        {
            return Err(
                "Each answer_node_ids entry must be a unique ID from the paper map.".into(),
            );
        }
    }
    let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut links = BTreeSet::new();
    let mut used_references = BTreeSet::new();
    for e in &map.edges {
        let from = e.from.as_str();
        let to = e.to.as_str();
        if !kinds.contains_key(from)
            || !kinds.contains_key(to)
            || from == to
            || !links.insert((from, to))
            || !EDGE_KINDS.contains(&e.kind.as_str())
            || !matches!(e.basis.as_str(), "explicit" | "inferred")
            || !bounded(&e.reason, 1000)
            || e.anchor.source_id != map.source_id
        {
            return Err(format!(
                "Invalid, duplicate or ungrounded edge '{from}' to '{to}'."
            ));
        }
        if e.kind == "cites" {
            if kinds[from] != "reference" || kinds[to] == "reference" {
                return Err(
                    "A citation must point from a reference node to the claim that uses it.".into(),
                );
            }
            used_references.insert(from);
        }
        children.entry(from).or_default().push(to);
        *indegree.get_mut(to).unwrap() += 1;
    }
    if kinds
        .iter()
        .any(|(id, kind)| *kind == "reference" && !used_references.contains(*id))
    {
        return Err("Connect each reference to the claim that uses it with a cites edge.".into());
    }
    let mut ready: VecDeque<_> = indegree
        .iter()
        .filter_map(|(id, count)| (*count == 0).then_some(*id))
        .collect();
    let mut visited = 0;
    while let Some(id) = ready.pop_front() {
        visited += 1;
        for &child in children.get(id).into_iter().flatten() {
            let count = indegree.get_mut(child).unwrap();
            *count -= 1;
            if *count == 0 {
                ready.push_back(child);
            }
        }
    }
    if visited != map.nodes.len() {
        return Err("Dependencies must be acyclic. Remove the circular link.".into());
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Document {
    Map,
    Summary,
    Branch,
    Conventions,
}

pub fn record(
    env: &ResearchEnv,
    ledger: &Ledger,
    session: &str,
    args: &Value,
) -> Result<Value, String> {
    record_document(env, ledger, session, args, Document::Map)
}

pub fn record_summary(
    env: &ResearchEnv,
    ledger: &Ledger,
    session: &str,
    args: &Value,
) -> Result<Value, String> {
    record_document(env, ledger, session, args, Document::Summary)
}

pub fn record_conventions(
    env: &ResearchEnv,
    ledger: &Ledger,
    session: &str,
    args: &Value,
) -> Result<Value, String> {
    record_document(env, ledger, session, args, Document::Conventions)
}

pub fn record_branch(
    env: &ResearchEnv,
    ledger: &Ledger,
    session: &str,
    args: &Value,
) -> Result<Value, String> {
    record_document(env, ledger, session, args, Document::Branch)
}

fn verify_graph_anchors(
    source_id: &str,
    nodes: &mut [Node],
    edges: &mut [Edge],
    index: &mut SourceIndex<'_>,
    report: &mut AnchorReport,
) -> Result<(), String> {
    let mut fatal_faults: Vec<AnchorFault> = Vec::new();
    for node in nodes {
        let mut kept = Vec::new();
        let mut node_notes = Vec::new();
        let mut node_faults = Vec::new();
        for (i, mut anchor) in std::mem::take(&mut node.anchors).into_iter().enumerate() {
            let owner = format!("nodes[{}].anchors[{i}]", node.id);
            match index.verify_any(&owner, &mut anchor) {
                Ok(note) => {
                    kept.push(anchor);
                    node_notes.push(note);
                }
                Err(fault) => {
                    if fault.legacy {
                        return Err(fault.message);
                    }
                    node_faults.push(fault);
                }
            }
        }
        if kept.is_empty() || !kept.iter().any(|a| a.source_id == source_id) {
            fatal_faults.extend(node_faults);
        } else {
            for note in node_notes {
                report.record_note(note);
            }
            for fault in &node_faults {
                report.record_dropped(fault);
            }
            node.anchors = kept;
        }
    }
    for edge in edges {
        let owner = format!("edges[{}->{}].anchor", edge.from, edge.to);
        match index.verify_any(&owner, &mut edge.anchor) {
            Ok(note) => report.record_note(note),
            Err(fault) => {
                if fault.legacy {
                    return Err(fault.message);
                }
                fatal_faults.push(fault);
            }
        }
    }
    if !fatal_faults.is_empty() {
        return Err(faults_error(&fatal_faults));
    }
    Ok(())
}

fn record_document(
    env: &ResearchEnv,
    ledger: &Ledger,
    session: &str,
    args: &Value,
    document: Document,
) -> Result<Value, String> {
    reject_app_only_fields(args)?;
    crate::validate_session_id(session).map_err(|e| e.to_string())?;
    let (verified, events) = ledger
        .read_verified_session(session)
        .map_err(|_| "The Investigation ledger or its saved artifacts could not be verified.")?;
    let mode = events.iter().rposition(|e| {
        matches!(
            e.kind.as_str(),
            "investigation.started" | "investigation.handoff"
        )
    });
    let active = mode
        .filter(|&i| events[i].kind == "investigation.started")
        .ok_or("Paper maps can only be recorded in an Investigation.")?;
    let history = &events[active..];
    let paper = history
        .iter()
        .find(|e| e.kind == "investigation.paper")
        .ok_or("Select and start an ingested paper before recording a map.")?;
    let expected = if document == Document::Branch {
        "investigation.branch_requested"
    } else {
        "investigation.mapping_requested"
    };
    let marker = history
        .iter()
        .rev()
        .find(|e| {
            matches!(
                e.kind.as_str(),
                "investigation.mapping_requested"
                    | "investigation.mapping_finished"
                    | "investigation.branch_requested"
                    | "investigation.branch_finished"
                    | "investigation.conventions_resumed"
                    | "investigation.conventions_finished"
                    | "investigation.question_requested"
                    | "investigation.answer"
                    | "investigation.requested"
                    | "investigation.result"
                    | "session.end"
                    | "session.abort"
            )
        })
        .filter(|e| e.kind == expected || (document==Document::Conventions && e.kind=="investigation.conventions_resumed"))
        .ok_or(if document == Document::Branch { "Select a result and start a focused branch before saving its dependencies." } else { "Start mapping the paper before recording its overview. Claim checks use the existing map." })?;
    let request = if marker.kind == "investigation.conventions_resumed" {
        let original = marker.payload["for_request_seq"]
            .as_u64()
            .ok_or("Missing original conventions request.")?;
        let current_map = history
            .iter()
            .rev()
            .find(|e| e.kind == "investigation.map")
            .ok_or("No saved overview to resume.")?;
        if current_map.payload["request_seq"].as_u64() != Some(original)
            || current_map.payload["source_id"] != marker.payload["source_id"]
        {
            return Err("This conventions resume belongs to a different paper overview.".into());
        }
        history
            .iter()
            .find(|e| e.seq == original && e.kind == "investigation.mapping_requested")
            .ok_or("Original paper mapping request is missing.")?
    } else {
        marker
    };
    let source_id = args["source_id"]
        .as_str()
        .filter(|s| valid_id(s, 64))
        .ok_or("Supply the selected source ID.")?;
    if paper.payload["source_id"] != source_id || request.payload["source_id"] != source_id {
        return Err("Use the paper selected for this Investigation.".into());
    }
    let prior_record = |kind| {
        history
            .iter()
            .rev()
            .take_while(|e| e.seq > request.seq)
            .find(|e| e.kind == kind && e.payload["request_seq"].as_u64() == Some(request.seq))
    };
    let prior_summary = prior_record("investigation.summary");
    let prior_map = prior_record("investigation.map");
    if matches!(document, Document::Map | Document::Conventions)
        && request.payload["summary_requested"] == true
        && prior_summary.is_none()
    {
        return Err("Record the early paper overview with record_paper_summary before saving the other reading results.".into());
    }
    if document == Document::Summary && prior_map.is_some() && prior_summary.is_none() {
        return Err("The detailed map is already saved. Start a new mapping request to record an early overview.".into());
    }
    let receipts: BTreeMap<_, _> = events
        .iter()
        .filter(|e| e.kind == "source.ingested")
        .filter_map(|e| Some((e.payload["source_id"].as_str()?, e)))
        .collect();
    let primary = receipts
        .get(source_id)
        .copied()
        .ok_or("Primary paper receipt missing.")?;
    let pages = SelectedPages::for_paper(ledger, &paper.payload, &primary.payload)?;
    let mut index = SourceIndex {
        pages,
        sandbox: env.sandbox(),
        session,
        primary_source_id: source_id,
        receipts,
        files: BTreeMap::new(),
        bytes_left: MAX_TOTAL_BYTES,
    };
    let mut report = AnchorReport::default();
    let (mut payload, event_kind, previous, seq_field) = match document {
        Document::Map => {
            let mut map: PaperMap = serde_json::from_value(args.clone())
                .map_err(|e| format!("Invalid paper map: {e}"))?;
            validate_map(&map, MAX_NODES, MAX_EDGES)?;
            if request.payload["overview_only"] == true
                && (map
                    .nodes
                    .iter()
                    .any(|node| !node.main || node.kind == "reference")
                    || map.edges.iter().any(|edge| edge.basis != "explicit"))
            {
                return Err("Save only main results and any dependencies explicitly stated between those results. The reader can explore their supporting arguments afterwards.".into());
            }
            verify_graph_anchors(
                &map.source_id,
                &mut map.nodes,
                &mut map.edges,
                &mut index,
                &mut report,
            )?;
            let mut payload = serde_json::to_value(&map).map_err(|e| e.to_string())?;
            payload["coverage_basis"] = json!("model_reported");
            (payload, "investigation.map", prior_map, "map_seq")
        }
        Document::Summary => (
            summary::payload(args, &mut index, &mut report)?,
            "investigation.summary",
            prior_summary,
            "summary_seq",
        ),
        Document::Branch => (
            branch::payload(args, &mut index, history, request, &mut report)?,
            "investigation.branch",
            prior_record("investigation.branch"),
            "branch_seq",
        ),
        Document::Conventions => {
            let (payload, previous) =
                conventions::payload(args, &mut index, history, request, &mut report)?;
            (
                payload,
                "investigation.conventions",
                previous,
                "conventions_seq",
            )
        }
    };
    payload["schema_version"] = json!(1);
    payload["request_seq"] = json!(request.seq);
    payload["anchors_verified"] = json!(true);
    payload["anchor_report"] = report.to_value();
    if let Some(pages) = index.pages {
        payload["page_start"] = json!(pages.first);
        payload["page_end"] = json!(pages.last);
    }
    let seq = match previous {
        Some(prev) if prev.payload == payload => prev.seq,
        Some(_) => return Err("This record was already saved for this request. Start another mapping request to revise it.".into()),
        _ => {
            ledger
                .append_at_head(
                    session,
                    event_kind,
                    1,
                    &payload,
                    &verified.head_hash,
                )
                .map_err(|e| e.to_string())?
                .seq
        }
    };
    let mut result = json!({
        "status": "recorded",
        "source_id": source_id,
        "request_seq": request.seq,
        "anchors_verified": true,
        "anchor_report": payload["anchor_report"],
        "anchor_basis": report.basis_value(),
    });
    result[seq_field] = json!(seq);
    if document == Document::Map {
        result["coverage"] = payload["coverage"].clone();
    }
    if matches!(document, Document::Map | Document::Branch) {
        result["nodes"] = json!(payload["nodes"].as_array().unwrap().len());
        result["edges"] = json!(payload["edges"].as_array().unwrap().len());
    }
    if document == Document::Conventions {
        for field in ["batch_index", "done", "coverage"] {
            result[field] = payload[field].clone();
        }
        result["items"] = json!(payload["items"].as_array().unwrap().len());
        result["next_batch_index"] = json!(payload["batch_index"].as_u64().unwrap_or(0) + 1);
        if let Some(rejected) = payload.get("rejected_items") {
            result["rejected_items"] = rejected.clone();
        }
    }
    Ok(result)
}
