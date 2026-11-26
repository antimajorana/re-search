//! Session-level Investigation state. Map revisions, reader corrections and optional checks are
//! separate ledger records, so reopening a paper preserves what the authors and the reader said.
pub(crate) use crate::investigation_finish::finish;
pub(crate) use crate::investigation_view::view;
use crate::investigation_view::branch_history;
use crate::routes_session::{bad, conflict, internal, not_found, ApiResult};
use crate::state::DesktopState;
use ledger::{Event, Ledger};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;

#[path = "investigation_prompts.rs"]
pub(crate) mod prompts;
pub(crate) use prompts::PaperRoute;

type Result<T> = std::result::Result<T, (u16, String)>;

fn storage(error: impl ToString) -> (u16, String) {
    internal(error.to_string())
}

pub(crate) fn marker(events: &[Event]) -> Option<&Event> {
    events.iter().rev().find(|e| {
        matches!(
            e.kind.as_str(),
            "investigation.started" | "investigation.handoff"
        )
    })
}

pub(crate) fn is_investigation(events: &[Event]) -> bool {
    crate::session_mode::session_mode(events) == research_tools::paper_key::Mode::Investigate
}

pub(crate) fn latest<'a>(events: &'a [Event], kind: &str) -> Option<&'a Event> {
    events.iter().rev().find(|e| e.kind == kind)
}

pub(crate) fn list(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or_default()
}

fn text<'a>(value: &'a Value, key: &str, limit: usize) -> Result<&'a str> {
    value[key]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.chars().count() <= limit)
        .ok_or_else(|| bad(format!("Provide {key} using up to {limit} characters.")))
}

pub(crate) fn current_map(events: &[Event], seq: Option<u64>) -> Result<&Event> {
    if !is_investigation(events) {
        return Err(bad("Open an Investigation session to use this action."));
    }
    let map = latest(events, "investigation.map").ok_or_else(|| bad("Map the paper first."))?;
    if Some(map.seq) != seq {
        return Err(conflict(
            "The paper map changed. Reopen the claim in the current map.",
        ));
    }
    Ok(map)
}

fn selection(events: &[Event], body: &Value) -> Result<(Event, Value)> {
    let mut map = current_map(events, body["map_seq"].as_u64())?.clone();
    map.payload = research_tools::investigation::expanded_map(events, &map);
    let id = text(body, "node_id", 128)?;
    let node = list(&map.payload["nodes"])
        .iter()
        .find(|n| n["id"] == id)
        .ok_or_else(|| bad("Claim missing from this map."))?;
    if let Some(from) = body["edge_from"].as_str() {
        if !list(&map.payload["edges"])
            .iter()
            .any(|e| e["from"] == from && e["to"] == id)
        {
            return Err(bad("Dependency missing from this map."));
        }
    }
    let node = node.clone();
    Ok((map, node))
}

fn selected_pages(
    req: &Value,
    pinned: Option<&Event>,
    source: &Event,
) -> Result<(Option<u64>, Option<u64>)> {
    let current = pinned.map(|event| {
        (
            event.payload["page_start"].as_u64(),
            event.payload["page_end"].as_u64(),
        )
    });
    if req["investigation_page_start"].is_null() && req["investigation_page_end"].is_null() {
        return Ok(current.unwrap_or_default());
    }
    let pages = source.payload["page_count"]
        .as_u64()
        .ok_or_else(|| bad("Page ranges apply to PDF papers."))?;
    let number = |name: &str, default| {
        if req[name].is_null() {
            Ok(default)
        } else {
            req[name]
                .as_u64()
                .ok_or_else(|| bad("Use whole physical PDF page numbers."))
        }
    };
    let start = number("investigation_page_start", 1)?;
    let end = number("investigation_page_end", pages)?;
    if start == 0 || end < start || end > pages {
        return Err(bad(format!("Choose a page range between 1 and {pages}.")));
    }
    let range = (Some(start), Some(end));
    if current.is_some_and(|old| old != range) {
        return Err(conflict(
            "Start another Investigation to map a different page range.",
        ));
    }
    Ok(range)
}

pub(crate) fn include_paper_conventions(context: &mut Value, events: &[Event], request: Option<u64>) {
    if let Some(catalog) =
        request.and_then(|seq| research_tools::investigation::paper_conventions(events, seq))
    {
        context["paper_conventions"] = catalog["items"].clone();
        context["conventions_status"] = json!({"done":catalog["done"], "coverage":catalog["coverage"],
            "limitations":catalog["limitations"], "request_seq":catalog["request_seq"]});
    }
}

fn claim_context(map: &Event, node: &Value, events: &[Event]) -> Value {
    let mut pending = vec![node["id"].as_str().unwrap_or_default()];
    let mut ids = HashSet::new();
    let edges = list(&map.payload["edges"]);
    while let Some(id) = pending.pop() {
        if ids.insert(id) {
            pending.extend(
                edges
                    .iter()
                    .filter(|e| e["to"] == id)
                    .filter_map(|e| e["from"].as_str()),
            );
        }
    }
    let nodes: Vec<_> = list(&map.payload["nodes"])
        .iter()
        .filter(|n| n["id"].as_str().is_some_and(|id| ids.contains(id)))
        .collect();
    let assumptions: Vec<_> = nodes
        .iter()
        .filter(|n| n["kind"] == "assumption")
        .map(|n| &n["sentence"])
        .collect();
    let mut sources = Vec::new();
    let branch_edges: Vec<_> = edges
        .iter()
        .filter(|e| e["to"].as_str().is_some_and(|id| ids.contains(id)))
        .collect();
    for anchor in nodes
        .iter()
        .flat_map(|n| list(&n["anchors"]))
        .chain(branch_edges.iter().filter_map(|e| e.get("anchor")))
    {
        if !anchor.is_null() && !sources.contains(&anchor) {
            sources.push(anchor);
        }
    }
    let corrections: Vec<_> = events
        .iter()
        .filter(|e| {
            e.kind == "investigation.correction"
                && e.payload["map_seq"].as_u64() == Some(map.seq)
                && e.payload["node_id"]
                    .as_str()
                    .is_some_and(|id| ids.contains(id))
        })
        .map(|e| &e.payload)
        .collect();
    let mut context = json!({"claim":node["sentence"], "node_id":node["id"], "map_seq":map.seq, "map_hash":map.event_hash,
        "assumptions":assumptions, "sources":sources, "dependencies":nodes, "edges":branch_edges, "corrections":corrections});
    include_paper_conventions(&mut context, events, map.payload["request_seq"].as_u64());
    context
}

pub(crate) fn turn_context(project: Option<String>, req: &Value) -> Option<String> {
    match (project, req["_paper_context"].as_str()) {
        (Some(p), Some(i)) => Some(format!("{p}\n\n{i}")),
        (None, Some(i)) => Some(i.to_owned()),
        (p, None) => p,
    }
}

// Full evidence stays in the ledger. A long-lived map should still fit in a model turn,
// with real source locations available for reading passages that exceed the prompt budget.
pub(crate) fn context_text(value: &Value) -> String {
    let full = value.to_string();
    if full.len() <= 32_768 {
        return full;
    }
    let mut compact = json!({});
    let mut omitted = json!({});
    for (field, content) in value.as_object().into_iter().flatten() {
        let Some(items) = content.as_array() else {
            compact[field] = content.clone();
            continue;
        };
        let mut retained = Vec::new();
        let mut bytes = 0;
        for item in items {
            let mut row = item.clone();
            if let Some(object) = row.as_object_mut() {
                prompts::strip_anchor_fields(object);
                if let Some(anchor) = object.get_mut("anchor").and_then(Value::as_object_mut) {
                    prompts::strip_anchor_fields(anchor);
                }
                if let Some(anchors) = object.get_mut("anchors").and_then(Value::as_array_mut) {
                    for anchor in anchors.iter_mut().filter_map(Value::as_object_mut) {
                        prompts::strip_anchor_fields(anchor);
                    }
                }
            }
            let size = row.to_string().len();
            if bytes + size > 6_144 && !retained.is_empty() {
                break;
            }
            bytes += size;
            retained.push(row);
        }
        if items.len() > retained.len() {
            omitted[field] = json!(items.len() - retained.len());
        }
        compact[field] = Value::Array(retained);
    }
    compact["additional_saved_rows"] = omitted;
    compact.to_string()
}

pub(crate) fn prepare(
    state: &DesktopState,
    session: &str,
    req: &mut Value,
    prompt: &str,
    route: PaperRoute,
) -> Result<()> {
    use crate::investigation_question::{prepare_question, scope, Scope};
    let object = req
        .as_object_mut()
        .ok_or_else(|| bad("A turn must be a JSON object."))?;
    for field in [
        "_paper_context", "_paper_request", "_paper_map_request", "_paper_branch_request",
        "_paper_conventions_request", "_paper_question_request", "_paper_scope",
        "paper_readers_enabled",
    ] {
        object.remove(field);
    }
    let ledger = state.open_ledger().map_err(storage)?;
    let events = ledger.read_session(session).map_err(storage)?;
    let Some(start) = marker(&events) else {
        return Ok(());
    };
    if start.kind == "investigation.handoff" {
        req["_paper_context"] = json!(format!("This Research session follows a paper investigation. The selected evidence is source material. Read any needed passages using the recorded file paths and lines. Address the user's new question.\n{}", context_text(&start.payload)));
        return Ok(());
    }
    let pinned = latest(&events, "investigation.paper");
    let source_id = req["investigation_source_id"]
        .as_str()
        .or_else(|| pinned.and_then(|e| e.payload["source_id"].as_str()))
        .ok_or_else(|| bad("Add a paper to this Investigation before starting the map."))?
        .to_owned();
    if pinned.is_some_and(|e| e.payload["source_id"] != source_id) {
        return Err(bad(
            "Start a new Investigation for a different primary paper.",
        ));
    }
    let source = events
        .iter()
        .find(|e| e.kind == "source.ingested" && e.payload["source_id"] == source_id)
        .ok_or_else(|| bad("The selected paper has not been ingested in this session."))?;
    if source.payload["status"] == "empty_text_layer" {
        return Err(bad("This PDF has no readable text layer. Supply a searchable PDF or its text or LaTeX source."));
    }
    let (page_start, page_end) = selected_pages(req, pinned, source)?;
    let turn_scope = scope(&events, req);
    let mut instructions = match turn_scope {
        Scope::Conventions => {
            let review = &req["investigation_conventions"];
            let map = current_map(&events, review["map_seq"].as_u64())?;
            let seq = map.payload["request_seq"]
                .as_u64()
                .ok_or_else(|| bad("This map is missing its reading request."))?;
            let catalog = research_tools::investigation::paper_conventions(&events, seq)
                .unwrap_or(Value::Null);
            if !crate::investigation_scope::conventions_resume_allows(&catalog) {
                return Err(conflict("The conventions review is already saved."));
            }
            let resume_scope =
                crate::investigation_scope::conventions_resume_scope(&events, seq, &catalog);
            let expanding = crate::investigation_scope::conventions_expanding_to_paper(&catalog);
            let section_scope = (resume_scope == Some("section"))
                .then(|| crate::investigation_scope::section_conventions_scope_suffix(&events, seq))
                .flatten();
            let mode = match (expanding, section_scope.as_deref()) {
                (true, _) => prompts::ConventionsResumeMode::ExpandToPaper,
                (false, Some(s)) => prompts::ConventionsResumeMode::Section(s),
                (false, None) => prompts::ConventionsResumeMode::Paper,
            };
            let prior = context_text(&catalog);
            let next_batch = catalog["batch_index"].as_u64().unwrap_or(0) + 1;
            let text = prompts::conventions_resume_instructions(
                route, &source_id, &source.payload["primary_path"], seq, next_batch, &catalog,
                &prior, mode,
            );
            let mut resumed_payload =
                json!({"for_request_seq":seq,"source_id":source_id,"map_seq":map.seq});
            if let Some(scope_name) = resume_scope {
                resumed_payload["scope"] = json!(scope_name);
            }
            ledger
                .append(session, "investigation.conventions_resumed", 1, &resumed_payload)
                .map_err(storage)?;
            req["_paper_conventions_request"] = json!(seq);
            text
        }
        Scope::Branch => {
            let branch = req["investigation_branch"].clone();
            let (map, node) = selection(&events, &branch)?;
            let focus = text(&branch, "question", 2000)?;
            let existing: Vec<_> = list(&map.payload["nodes"])
                .iter()
                .map(|n| json!({"id":n["id"],"kind":n["kind"],"sentence":n["sentence"]}))
                .collect();
            let edges: Vec<_> = list(&map.payload["edges"])
                .iter()
                .map(|e| json!({"from":e["from"], "to":e["to"], "kind":e["kind"]}))
                .collect();
            let selected_context = claim_context(&map, &node, &events);
            let context = json!({"source_id":source_id,"source_path":source.payload["primary_path"],"selected":node,
                "focus":focus,"page_start":page_start,"page_end":page_end,"existing_statements":existing,
                "paper_conventions":selected_context["paper_conventions"],"conventions_status":selected_context["conventions_status"],
                "existing_edges":edges,"selected_edges":selected_context["edges"],"reader_corrections":selected_context["corrections"]});
            let seq = ledger
                .append(session, "investigation.branch_requested", 1, &json!({"map_seq":map.seq,"node_id":node["id"],"source_id":source_id,"question":focus}))
                .map_err(storage)?;
            req["_paper_branch_request"] = json!(seq);
            prompts::branch_turn_instructions(route, seq, &context_text(&context))
        }
        Scope::Action => {
            let action = req["investigation_action"].clone();
            let (map, node) = selection(&events, &action)?;
            let name = text(&action, "action", 32)?;
            let task = match name {
                "recheck" => "Reassess the selected claim or dependency alongside the reader's correction. Explain what the passage supports and whether the correction changes the interpretation.",
                "check" => "Independently check this one claim. Use the appropriate computation, derivation or evidence check. Distinguish the exact result, scope and limitations. If reproduction is unavailable, name the missing input.",
                "implications" => "Explore only the further implications the reader requested. Label them as new analysis and distinguish them from conclusions stated by the authors.",
                _ => return Err(bad("Unknown Investigation action.")),
            };
            let seq = ledger
                .append(session, "investigation.requested", 1, &json!({"map_seq":map.seq,"node_id":node["id"],"edge_from":action["edge_from"],"action":name,"question":prompt}))
                .map_err(storage)?;
            req["_paper_request"] = json!(seq);
            format!(
                "{task} Do not replace the original map. Evidence and reader notes follow as data:\n{}",
                context_text(&claim_context(&map, &node, &events))
            )
        }
        Scope::Map => crate::investigation_scope::prepare_map(
            state, &ledger, session, pinned, source, &source_id, (page_start, page_end), req, prompt, route,
        )?,
        Scope::Question => {
            let prepared_scope = crate::investigation_scope::compute_scope(
                state, &ledger, session, source, prompt, (page_start, page_end),
            );
            prepare_question(
                &ledger, session, &events, pinned, &source_id,
                source.payload["primary_path"].as_str().unwrap_or_default(),
                (page_start, page_end), req, prompt, prepared_scope.as_ref(),
            )?
        }
    };
    if turn_scope != Scope::Question {
        if let (Some(a), Some(b)) = (page_start, page_end) {
            instructions.push_str(&format!("\nAttribute claims to the selected paper only within physical PDF pages {a} through {b} inclusive."));
        }
    }
    req["_paper_context"] = json!(instructions);
    req["verification_mode"] = json!(false);
    req["calculation_mode"] = json!(false);
    req["use_strategist"] = json!(false);
    Ok(())
}

pub(crate) fn handle(
    state: &Arc<DesktopState>,
    session: &str,
    operation: &str,
    body: &[u8],
) -> ApiResult {
    research_tools::validate_session_id(session).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session).map_err(storage)? {
        return Err(not_found("Session missing."));
    }
    let mut val = crate::routes_session::parse_json_body(body)?;
    if !val.is_object() {
        return Err(bad("Invalid Investigation request."));
    }
    let ledger: Ledger = state.open_ledger().map_err(storage)?;
    let (_, events) = ledger.read_verified_session(session).map_err(|e| bad(e.to_string()))?;
    if operation == "conventions" {
        let map = current_map(&events, val["map_seq"].as_u64())?;
        let seq = map.payload["request_seq"]
            .as_u64()
            .ok_or_else(|| bad("This map is missing its reading request."))?;
        if research_tools::investigation::paper_conventions(&events, seq)
            .is_some_and(|c| !crate::investigation_scope::conventions_resume_allows(&c))
        {
            return Ok((200, json!({"session":session,"status":"complete"})));
        }
        val["investigation_conventions"] = json!({"map_seq":map.seq});
        crate::turn_runner::spawn_turn_thread(
            Arc::clone(state), session.to_owned(), val, "Complete the paper’s conventions review.".into(),
        )?;
        return Ok((202, json!({"session":session,"status":"running"})));
    }
    if operation == "research" {
        return open_research(state, &ledger, session, &events, &val);
    }
    let (map, node) = selection(&events, &val)?;
    match operation {
        "branches" => {
            let question = text(&val, "question", 2000)?.to_owned();
            let branch = json!({"map_seq":map.seq,"node_id":node["id"],"question":question});
            if branch_history(&events, false).iter().any(|r| {
                r["map_seq"] == branch["map_seq"]
                    && r["node_id"] == branch["node_id"]
                    && r["question"] == branch["question"]
                    && r["status"] == "built"
            }) {
                return Ok((200, json!({"session":session,"status":"built"})));
            }
            val["investigation_branch"] = branch;
            let prompt = format!(
                "Develop this result: {}\nReading question: {question}",
                node["sentence"].as_str().unwrap_or_default()
            );
            crate::turn_runner::spawn_turn_thread(Arc::clone(state), session.to_owned(), val, prompt)?;
            Ok((202, json!({"session":session,"status":"running"})))
        }
        "corrections" => {
            let correction = json!({"map_seq":map.seq,"node_id":node["id"],"text":text(&val,"text",4000)?,"edge_from":val["edge_from"]});
            let seq = ledger
                .append_at_head(
                    session,
                    "investigation.correction",
                    1,
                    &correction,
                    &events.last().ok_or_else(|| bad("Map missing from session."))?.event_hash,
                )
                .map_err(|err| match err {
                    ledger::Error::SessionChanged(_) => {
                        conflict("The session changed. Reopen the claim and retry your correction.")
                    }
                    other => storage(other),
                })?
                .seq;
            Ok((201, json!({"seq":seq,"correction":correction})))
        }
        "rechecks" => {
            let action = text(&val, "action", 32)?;
            let prompt = val["prompt"]
                .as_str()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    format!(
                        "{}: {}",
                        match action {
                            "recheck" => "Reassess the interpretation",
                            "check" => "Check this claim",
                            _ => "Explore further implications",
                        },
                        node["sentence"].as_str().unwrap_or_default()
                    )
                });
            if prompt.chars().count() > 8192 {
                return Err(bad("Keep the question under 8192 characters."));
            }
            val["investigation_action"] = json!({"action":action,"node_id":node["id"],"map_seq":map.seq,"edge_from":val["edge_from"]});
            crate::turn_runner::spawn_turn_thread(Arc::clone(state), session.to_owned(), val, prompt)?;
            Ok((202, json!({"status":"running","session":session})))
        }
        _ => Err(not_found("Unknown Investigation action.")),
    }
}

fn open_research(
    state: &DesktopState,
    ledger: &Ledger,
    session: &str,
    events: &[Event],
    val: &Value,
) -> ApiResult {
    let paper = latest(events, "investigation.paper");
    let has_map_target = !val["map_seq"].is_null() || !val["node_id"].is_null();
    let (mut context, question, default_through, mut linked) = if has_map_target {
        let (map, node) = selection(events, val)?;
        let q = text(val, "question", 8192)?.to_owned();
        let mut ctx = claim_context(&map, &node, events);
        ctx["question"] = json!(q);
        (ctx, q, map.seq, json!({"map_seq": map.seq, "node_id": node["id"]}))
    } else {
        if !is_investigation(events) {
            return Err(bad("Open an Investigation session to use this action."));
        }
        let answer = if let Some(aseq) = val["answer_seq"].as_u64() {
            events.iter().rev().find(|e| {
                e.kind == "investigation.answer"
                    && e.payload["status"] == "answered"
                    && (e.seq == aseq || e.payload["request_seq"].as_u64() == Some(aseq))
            })
        } else {
            events
                .iter()
                .rev()
                .find(|e| e.kind == "investigation.answer" && e.payload["status"] == "answered")
        }
        .ok_or_else(|| bad("Answer a question first."))?;
        let q_req = answer.payload["request_seq"].as_u64().and_then(|seq| {
            events.iter().find(|e| e.seq == seq && e.kind == "investigation.question_requested")
        });
        let q = if val["question"].as_str().is_some_and(|s| !s.trim().is_empty()) {
            text(val, "question", 8192)?.to_owned()
        } else {
            q_req.and_then(|e| e.payload["question"].as_str())
                .or_else(|| paper.and_then(|p| p.payload["question"].as_str()))
                .map(str::trim)
                .filter(|s| !s.is_empty() && s.chars().count() <= 8192)
                .ok_or_else(|| bad("Provide question using up to 8192 characters."))?
                .to_owned()
        };
        let source_id = paper.and_then(|p| p.payload["source_id"].as_str())
            .or_else(|| q_req.and_then(|e| e.payload["source_id"].as_str()))
            .or_else(|| latest(events, "source.ingested").and_then(|e| e.payload["source_id"].as_str()))
            .ok_or_else(|| bad("Add a paper to this Investigation first."))?;
        let primary_path = events
            .iter()
            .rev()
            .find(|e| e.kind == "source.ingested" && e.payload["source_id"].as_str() == Some(source_id))
            .and_then(|e| e.payload["primary_path"].as_str());
        let sources: Vec<Value> = list(&answer.payload["cites"]).iter()
            .filter(|c| c["read"] == true)
            .filter_map(|c| {
                let (s, e) = (c["start"].as_u64()?, c["end"].as_u64()?);
                (s >= 1 && s <= e).then(|| json!({
                    "source_id": source_id, "path": primary_path,
                    "start_line": s, "end_line": e, "kind": "passage"
                }))
            })
            .collect();
        let ctx = json!({"source_id":source_id,"answer_seq":answer.seq,"answer_hash":answer.event_hash,
            "question":q,"sources":sources,"assumptions":[],"dependencies":[],
            "edges":[],"corrections":[],"paper_conventions":[]});
        (ctx, q, answer.seq, json!({"answer_seq": answer.seq}))
    };
    if let Some(p) = paper {
        context["primary_source_id"] = p.payload["source_id"].clone();
        context["page_start"] = p.payload["page_start"].clone();
        context["page_end"] = p.payload["page_end"].clone();
    }
    context["parent_session"] = json!(session);
    context["evidence_through_seq"] = json!(events.last().map_or(default_through, |e| e.seq));
    let child = export::fork::research_from_paper(&state.cfg.data_dir, session, &context).map_err(bad)?;
    let mut meta = state.load_meta(session).unwrap_or_default();
    meta.id = child.clone();
    meta.title = question.chars().take(80).collect();
    meta.allow_model_title = Some(false);
    meta.archived = false;
    meta.fork_info = None;
    meta.created_ms = crate::state::now_ms();
    state.save_meta(&meta).map_err(storage)?;
    if let Some(project) = crate::projects::project_for_session(state, session)? {
        crate::projects::attach_session(state, &project.id, &child)?;
    }
    linked["session_id"] = json!(child);
    linked["question"] = json!(question);
    ledger.append(session, "investigation.linked", 1, &linked).map_err(storage)?;
    Ok((201, json!({"session_id": child})))
}

#[cfg(test)]
#[path = "investigation_tests.rs"]
mod tests;
