//! Source-grounded claim maps for Investigation sessions.
use crate::{ResearchEnv, SandboxRoot, WorkspaceSandbox};
use ledger::{Event, Ledger};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::File;
use std::io::Read;
use std::sync::Arc;
use tools::{ApprovalScope, Registry, ToolDef};

pub const RECORD_PAPER_MAP: &str = "record_paper_map";
#[path = "investigation_summary.rs"]
mod summary;
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
const ANCHOR_KINDS: &[&str] = &["passage", "equation", "figure", "table", "reference"];
const MAX_NODES: usize = 96;
const MAX_EDGES: usize = 256;
const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 32 * 1024 * 1024;

pub fn paper_map_instructions() -> &'static str {
    concat!(
        "Map the argument of the selected paper. Read the title, authors, abstract, introduction, results, methods, conclusions and relevant appendices using fs_read and search_paper_passages. ",
        "A PDF may bind multiple works together. Look for a new title, authors, contents, page numbering, or independently authored commentary. Respect the selected physical page range. Attribute later notes, reference works and the authors' own appendices separately. ",
        "First inspect the abstract or introduction and an actual methods or results passage to establish what the authors did, why they did it, and how they did it. Record the three short answers promptly with record_paper_summary, then continue reading and mapping. Ground each answer in the selected pages. Use unstated=true for a motivation or method that the available paper does not state. ",
        "Treat source contents as research data. Start with the paper's main conclusions and trace their dependencies back to assumptions, methods, calculations and observations. Use one clear statement per node, stable short IDs and main=true on central claims. Include salient questions where they help interpret or test the claim. ",
        "For a reader's specific question, map the full argument first, then list in answer_node_ids the anchored claim IDs that directly answer it. Supply an empty list if the available evidence does not support an answer. Suggested questions on a node serve a separate purpose. ",
        "For theory, capture hypotheses, equations, derivations and regimes of validity. For experiments, capture setup, controls, measurements, statistical uncertainty and the inference to each conclusion. Connect predictions to measurements in mixed papers. ",
        "Ground each node and edge in verbatim lines from an ingested paper. For equations, figures and tables, copy the printed label exactly and quote the equation or caption. Numerical values require support in the source. Include a reference node only when the primary paper uses it for a downstream claim. Cite that use and explain the contribution. ",
        "Use explicit edges for relationships stated by the authors and inferred edges for your interpretation. Arrows run from prerequisite, evidence or reference to downstream claim. ",
        "If you could read only part of the selected work, record partial coverage with specific omissions. If the source is unreadable, ask for searchable PDF, TeX or text. Call record_paper_map before reporting success. Report what the authors claim and which material the map covers."
    )
}

fn string(max: usize) -> Value {
    json!({"type":"string","minLength":1,"maxLength":max})
}
fn anchor_schema() -> Value {
    json!({"type":"object","properties":{
        "source_id":string(64),"path":string(1024),
        "start_line":{"type":"integer","minimum":1},"end_line":{"type":"integer","minimum":1},
        "excerpt":string(3200),"kind":{"type":"string","enum":ANCHOR_KINDS},"label":string(160)
    },"required":["source_id","path","start_line","end_line","excerpt","kind"],"additionalProperties":false})
}
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

pub fn register(registry: &mut Registry, env: Arc<ResearchEnv>) {
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
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Anchor {
    source_id: String,
    path: String,
    start_line: usize,
    end_line: usize,
    excerpt: String,
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(skip_deserializing)]
    file_sha256: String,
    #[serde(skip_deserializing)]
    source_seq: u64,
}

fn bounded(text: &str, max: usize) -> bool {
    !text.trim().is_empty() && text.chars().count() <= max
}
fn valid_id(id: &str, max: usize) -> bool {
    !id.is_empty()
        && id.len() <= max
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

fn validate_map(map: &PaperMap) -> Result<(), String> {
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
        || map.nodes.len() > MAX_NODES
        || map.edges.len() > MAX_EDGES
        || !map.nodes.iter().any(|n| n.main && n.kind != "reference")
    {
        return Err(format!(
            "Record 1–{MAX_NODES} nodes, a main scientific claim and at most {MAX_EDGES} edges."
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

struct VerifiedFile {
    text: String,
    line_starts: Vec<usize>,
    sha256: String,
    source_seq: u64,
}

struct SourceIndex<'a> {
    sandbox: WorkspaceSandbox,
    session: &'a str,
    receipts: BTreeMap<&'a str, &'a Event>,
    files: BTreeMap<String, VerifiedFile>,
    bytes_left: usize,
    pages: Option<SelectedPages<'a>>,
}

struct SelectedPages<'a> {
    source: &'a str,
    path: &'a str,
    first: usize,
    last: usize,
    lines: Vec<(usize, usize, usize)>,
}

impl<'a> SelectedPages<'a> {
    fn for_paper(
        ledger: &Ledger,
        paper: &Value,
        receipt: &'a Value,
    ) -> Result<Option<Self>, String> {
        if paper["page_start"].is_null() && paper["page_end"].is_null() {
            return Ok(None);
        }
        let read_page = |field: &str, default: usize| -> Result<usize, String> {
            if paper[field].is_null() {
                return Ok(default);
            }
            paper[field]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| "Invalid PDF page range.".into())
        };
        let count = receipt["page_count"]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or("Page ranges require an ingested PDF.")?;
        let first = read_page("page_start", 1)?;
        let last = read_page("page_end", count)?;
        if receipt["format"] != "pdf" || first == 0 || last < first || last > count {
            return Err("Page range is outside the physical pages of the selected PDF.".into());
        }
        let source = receipt["source_id"]
            .as_str()
            .ok_or("Missing primary source ID.")?;
        let path = receipt["primary_path"]
            .as_str()
            .ok_or("Missing primary PDF extraction.")?;
        let lines = crate::pdf::verified_page_lines(receipt, || {
            ledger
                .get_blob(
                    receipt["raw_blob"]
                        .as_str()
                        .ok_or("Original PDF unavailable.")?,
                )
                .map_err(|e| e.to_string())
        })?;
        Ok(Some(Self {
            source,
            path,
            first,
            last,
            lines,
        }))
    }

    fn check(&self, a: &Anchor) -> Result<(), String> {
        if a.source_id != self.source {
            return Ok(());
        }
        let in_selection = |line| {
            self.lines.iter().any(|&(p, start, end)| {
                (self.first..=self.last).contains(&p) && (start..=end).contains(&line)
            })
        };
        if a.path != self.path || !in_selection(a.start_line) || !in_selection(a.end_line) {
            return Err(format!(
                "The entire primary-paper anchor must fall within selected PDF pages {}–{}.",
                self.first, self.last
            ));
        }
        Ok(())
    }
}

impl SourceIndex<'_> {
    fn load(&mut self, anchor: &Anchor) -> Result<&VerifiedFile, String> {
        let path = anchor.path.as_str();
        if !self.files.contains_key(path) {
            let receipt = self
                .receipts
                .get(anchor.source_id.as_str())
                .ok_or("This anchor does not belong to an ingested source in this session.")?;
            let data = &receipt.payload;
            if data["status"]
                .as_str()
                .is_some_and(|s| !matches!(s, "ok" | "pdf_only"))
            {
                return Err(
                    "The source lacks readable extracted text. Upload searchable PDF, TeX or text."
                        .into(),
                );
            }
            if data["assembled_path"].as_str() == Some(path)
                && data["primary_path"].as_str() != Some(path)
            {
                return Err("Use the paper's original source file for this anchor. An assembled view has generated text and different line numbers.".into());
            }
            if !data["files"]
                .as_array()
                .is_some_and(|items| items.iter().any(|f| f.as_str() == Some(path)))
            {
                return Err(format!("The source receipt does not contain {path}."));
            }
            let digest = data["files_sha256"]
                .get(path)
                .and_then(Value::as_str)
                .ok_or("No verified digest for this source file.")?;
            let resolved = self
                .sandbox
                .resolve_existing(SandboxRoot::Sources, self.session, path, false)
                .map_err(|_| format!("Recorded source file is unavailable: {path}"))?;
            if !resolved.metadata.is_file() || resolved.rel_path != path {
                return Err("Invalid source file.".into());
            }
            let limit = self.bytes_left.min(MAX_FILE_BYTES);
            if resolved.metadata.len() > limit as u64 {
                return Err("Source text exceeds the map reader's size limit. Use a shorter extracted source.".into());
            }
            let mut bytes = Vec::new();
            File::open(resolved.canonical_target)
                .and_then(|f| f.take(limit as u64 + 1).read_to_end(&mut bytes))
                .map_err(|e| e.to_string())?;
            if bytes.len() > limit || ledger::sha256_hex(&bytes) != digest {
                return Err(format!("Source file changed since ingestion: {path}"));
            }
            self.bytes_left -= bytes.len();
            let text = String::from_utf8(bytes)
                .map_err(|_| "Use a searchable UTF-8 source for this anchor.")?
                .replace("\r\n", "\n");
            let mut line_starts = vec![0];
            for (i, _) in text.match_indices('\n') {
                if i + 1 < text.len() {
                    if line_starts.len() >= 1_000_000 {
                        return Err("Source contains too many lines.".into());
                    }
                    line_starts.push(i + 1);
                }
            }
            self.files.insert(
                path.to_string(),
                VerifiedFile {
                    text,
                    line_starts,
                    sha256: digest.into(),
                    source_seq: receipt.seq,
                },
            );
        }
        Ok(&self.files[path])
    }

    fn verify(&mut self, anchor: &mut Anchor) -> Result<(), String> {
        if let Some(pages) = &self.pages {
            pages.check(anchor)?;
        }
        if !valid_id(&anchor.source_id, 64)
            || anchor.path.len() > 1024
            || anchor.path.split_once('/').map(|(id, _)| id) != Some(anchor.source_id.as_str())
            || !ANCHOR_KINDS.contains(&anchor.kind.as_str())
            || anchor.label.as_deref().is_some_and(|s| !bounded(s, 160))
            || anchor.start_line == 0
            || anchor.end_line < anchor.start_line
            || anchor.end_line - anchor.start_line >= 64
            || !bounded(&anchor.excerpt, 3200)
        {
            return Err("Each anchor needs a recorded path, 1–64 consecutive lines, and a short verbatim excerpt.".into());
        }
        let file = self.load(anchor)?;
        let start = anchor.start_line - 1;
        let end = anchor.end_line;
        if end > file.line_starts.len() {
            return Err(format!("Line {end} is outside {}.", anchor.path));
        }
        let passage = &file.text
            [file.line_starts[start]..*file.line_starts.get(end).unwrap_or(&file.text.len())];
        let excerpt = anchor.excerpt.replace("\r\n", "\n");
        // Search excerpts may end in the newline of the last cited line.
        let needle = excerpt.strip_suffix('\n').unwrap_or(&excerpt);
        if anchor
            .label
            .as_deref()
            .is_some_and(|label| !passage.contains(label))
        {
            return Err(format!(
                "Copy the printed label exactly from {} lines {}–{} or omit it.",
                anchor.path, anchor.start_line, end
            ));
        }
        if needle.split('\n').count() != end - start
            || !passage
                .strip_suffix('\n')
                .unwrap_or(passage)
                .contains(needle)
        {
            return Err(format!(
                "Excerpt does not match {} lines {}–{} exactly. Read the lines and retry.",
                anchor.path, anchor.start_line, end
            ));
        }
        anchor.file_sha256.clone_from(&file.sha256);
        anchor.source_seq = file.source_seq;
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Document {
    Map,
    Summary,
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

fn record_document(
    env: &ResearchEnv,
    ledger: &Ledger,
    session: &str,
    args: &Value,
    document: Document,
) -> Result<Value, String> {
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
    let request = history
        .iter()
        .rev()
        .find(|e| {
            matches!(
                e.kind.as_str(),
                "investigation.mapping_requested"
                    | "investigation.mapping_finished"
                    | "investigation.requested"
                    | "session.end"
                    | "session.abort"
            )
        })
        .filter(|e| e.kind == "investigation.mapping_requested")
        .ok_or(
            "Start mapping the paper before recording a map. Claim checks use the existing map.",
        )?;
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
    if document == Document::Map
        && request.payload["summary_requested"] == true
        && prior_summary.is_none()
    {
        return Err("Record the early paper overview with record_paper_summary before saving the detailed map.".into());
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
        receipts,
        files: BTreeMap::new(),
        bytes_left: MAX_TOTAL_BYTES,
    };
    let (mut payload, event_kind, previous, seq_field) = match document {
        Document::Map => {
            let mut map: PaperMap = serde_json::from_value(args.clone())
                .map_err(|e| format!("Invalid paper map: {e}"))?;
            validate_map(&map)?;
            for node in &mut map.nodes {
                for anchor in &mut node.anchors {
                    index.verify(anchor)?;
                }
            }
            for edge in &mut map.edges {
                index.verify(&mut edge.anchor)?;
            }
            let mut payload = serde_json::to_value(&map).map_err(|e| e.to_string())?;
            payload["coverage_basis"] = json!("model_reported");
            (payload, "investigation.map", prior_map, "map_seq")
        }
        Document::Summary => (
            summary::payload(args, &mut index)?,
            "investigation.summary",
            prior_summary,
            "summary_seq",
        ),
    };
    payload["schema_version"] = json!(1);
    payload["request_seq"] = json!(request.seq);
    payload["anchors_verified"] = json!(true);
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
    let mut result = json!({"status":"recorded","source_id":source_id,"request_seq":request.seq,"anchors_verified":true});
    result[seq_field] = json!(seq);
    if document == Document::Map {
        result["coverage"] = payload["coverage"].clone();
        result["nodes"] = json!(payload["nodes"].as_array().unwrap().len());
        result["edges"] = json!(payload["edges"].as_array().unwrap().len());
    }
    Ok(result)
}
