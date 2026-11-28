//! Outgoing Investigate hand-off packet builders and read-only child status projection
//! (package V16, `00-overview.md` §3.1 row 9 & §4, `investigate.md` §9 V16).
//!
//! Zero model calls and zero network calls. Preserves F7's five rules:
//! badges/tiers never leave Investigate; model gists travel only in `model_text` tagged `"(model)"`;
//! parent ledgers are never written when reading child session statuses.

#[path = "investigation_handoff_extract.rs"]
mod extract;
#[path = "investigation_handoff_status.rs"]
mod status;

pub use extract::parse_skeleton_from_event;
pub use status::{
    attach_child_statuses_to_mapview, child_handoff_statuses, sanitize_f12_status_label,
    ChildHandoffStatusView,
};

use self::extract::{
    build_verified_quotes_for_spans, clamp_chars, collect_conventions_summary, collect_model_text,
    collect_relevant_symbols, collect_seed_papers, collect_statement_closure, collect_user_notes,
    context_role_for_kind, extract_spans_text, load_readonly_skeleton, load_source_file_map,
    non_empty_description, non_empty_preview, solve_kind_and_form,
};
use crate::investigation::is_investigation;
use crate::investigation_cited_support::paper_ref_from_key;
use crate::routes_session::{bad, internal, not_found, parse_json_body, ApiResult};
use crate::state::DesktopState;
use export::handoff::{
    ExploreHandoffAsk, ExploreQuestionKind, HandoffAsk, HandoffContextItem, HandoffPacket,
    HandoffRequest, HandoffStatement, SolveHandoffAsk, MAX_CONTEXT_ITEMS,
    MAX_CONTEXT_TEXT_CHARS, MAX_PAPERS, MAX_STATEMENT_CHARS, MAX_TITLE_CHARS,
};
use research_tools::map_fold::fold_map;
use research_tools::paper_key::{Mode, NodeRef};
use research_tools::skeleton::format_candidate_label;
use research_tools::solve_model::SolveTarget;
use research_tools::validate_session_id;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Supported Investigate outgoing hand-off button / action kinds (`investigate.md` §9 V16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffActionKind {
    DeriveInSolve,
    HasBeenExtended,
    SearchWiderInExplore,
}

impl HandoffActionKind {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "derive_solve" | "derive_in_solve" => Some(Self::DeriveInSolve),
            "extend_explore" | "has_been_extended" => Some(Self::HasBeenExtended),
            "prior_work_explore" | "search_wider_in_explore" | "search_wider" => {
                Some(Self::SearchWiderInExplore)
            }
            _ => None,
        }
    }

    pub fn action_key(self) -> &'static str {
        match self {
            Self::DeriveInSolve => "derive_solve",
            Self::HasBeenExtended => "extend_explore",
            Self::SearchWiderInExplore => "prior_work_explore",
        }
    }
}

/// `POST /api/sessions/{id}/investigation/handoff-packet`: build a verified [`HandoffRequest`]
/// from an Investigate node with zero model calls and zero network calls.
pub fn handle_build_handoff_packet(state: &DesktopState, session_id: &str, body: &[u8]) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let val = parse_json_body(body)?;
    let Some(obj) = val.as_object() else {
        return Err(bad("Handoff packet request body must be a JSON object."));
    };
    let action_raw = obj
        .get("action")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("Provide `action` (`derive_solve`, `extend_explore`, or `prior_work_explore`)."))?;
    let action = HandoffActionKind::parse(action_raw)
        .ok_or_else(|| bad(format!("Unknown handoff action `{action_raw}`.")))?;
    let tag = obj
        .get("tag")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad("Provide a non-empty `tag` for the target node."))?;

    let req = build_investigation_handoff_request(state, session_id, action, tag)?;
    let payload = serde_json::to_value(&req).map_err(|e| internal(e.to_string()))?;
    Ok((200, payload))
}

pub fn build_derive_in_solve_request(
    state: &DesktopState,
    session_id: &str,
    tag: &str,
) -> Result<HandoffRequest, (u16, String)> {
    build_investigation_handoff_request(state, session_id, HandoffActionKind::DeriveInSolve, tag)
}

pub fn build_extend_explore_request(
    state: &DesktopState,
    session_id: &str,
    tag: &str,
) -> Result<HandoffRequest, (u16, String)> {
    build_investigation_handoff_request(state, session_id, HandoffActionKind::HasBeenExtended, tag)
}

pub fn build_search_wider_explore_request(
    state: &DesktopState,
    session_id: &str,
    tag: &str,
) -> Result<HandoffRequest, (u16, String)> {
    build_investigation_handoff_request(state, session_id, HandoffActionKind::SearchWiderInExplore, tag)
}

/// Deterministically construct a verified `HandoffRequest` for a skeleton node `tag`.
pub fn build_investigation_handoff_request(
    state: &DesktopState,
    session_id: &str,
    action: HandoffActionKind,
    tag: &str,
) -> Result<HandoffRequest, (u16, String)> {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    let tag = tag.trim();
    if tag.is_empty() {
        return Err(bad("Target node `tag` must not be empty."));
    }
    let ledger = state.open_ledger().map_err(|e| internal(e.to_string()))?;
    let events = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
    if !is_investigation(&events) {
        return Err(bad("Open an Investigation session before creating a hand-off packet."));
    }
    let rec = load_readonly_skeleton(state, &ledger, session_id, &events)?
        .ok_or_else(|| bad("Pin a paper and build its skeleton before creating a hand-off packet."))?;
    let node = rec
        .skeleton
        .nodes
        .iter()
        .find(|n| n.tag.as_deref() == Some(tag))
        .ok_or_else(|| not_found(format!("skeleton node `{tag}` not found")))?;

    let receipt = events
        .iter()
        .rfind(|e| {
            e.kind == "source.ingested"
                && e.payload["source_id"].as_str() == Some(&rec.skeleton.source_id)
                && e.seq == rec.skeleton.source_seq
        })
        .or_else(|| {
            events.iter().rfind(|e| {
                e.kind == "source.ingested" && e.payload["source_id"].as_str() == Some(&rec.skeleton.source_id)
            })
        })
        .ok_or_else(|| not_found(format!("source `{}` not ingested in `{session_id}`", rec.skeleton.source_id)))?;

    let env = state.research_env_for_session(session_id).map_err(internal)?;
    let sandbox = env.sandbox();
    let files = load_source_file_map(&sandbox, session_id, receipt, &rec.skeleton).map_err(internal)?;
    let mut quote_texts = BTreeMap::new();

    let raw_stmt = extract_spans_text(&files, &node.spans).map_err(internal)?;
    let stmt_text = clamp_chars(raw_stmt.trim(), MAX_STATEMENT_CHARS);
    let stmt_quotes = build_verified_quotes_for_spans(
        &sandbox, session_id, &events, &rec.skeleton.source_id, &node.spans, &files, &mut quote_texts,
    );

    let origin_paper = paper_ref_from_key(rec.skeleton.paper_key.clone(), rec.skeleton.version.clone());
    let closure_tags = collect_statement_closure(&rec.skeleton, node);
    let mut context = Vec::new();
    for ctx_tag in &closure_tags {
        if context.len() >= MAX_CONTEXT_ITEMS {
            break;
        }
        let Some(ctx_node) = rec.skeleton.nodes.iter().find(|n| n.tag.as_deref() == Some(ctx_tag.as_str())) else {
            continue;
        };
        let Ok(raw_ctx) = extract_spans_text(&files, &ctx_node.spans) else { continue; };
        let ctx_text = clamp_chars(raw_ctx.trim(), MAX_CONTEXT_TEXT_CHARS);
        let ctx_quotes = build_verified_quotes_for_spans(
            &sandbox, session_id, &events, &rec.skeleton.source_id, &ctx_node.spans, &files, &mut quote_texts,
        );
        context.push(HandoffContextItem {
            role: context_role_for_kind(ctx_node.kind),
            from: NodeRef {
                session: session_id.to_string(),
                mode: Mode::Investigate,
                id: ctx_tag.clone(),
                paper: Some(origin_paper.clone()),
                node_fp: Some(ctx_node.fp.clone()),
            },
            text: ctx_text,
            quotes: ctx_quotes,
        });
    }

    let seed_papers = collect_seed_papers(state, &rec.skeleton, node, &origin_paper)?;
    let mut papers = Vec::with_capacity(1 + seed_papers.len());
    papers.push(origin_paper.clone());
    for seed in &seed_papers {
        if papers.len() >= MAX_PAPERS {
            break;
        }
        papers.push(seed.clone());
    }

    let folded = fold_map(&rec.skeleton, &events, false);
    let model_text = collect_model_text(&folded, tag, &closure_tags);
    let user_notes = collect_user_notes(&folded, tag, &closure_tags);
    let conventions = collect_conventions_summary(state, session_id, &events);
    let node_label = format_candidate_label(node);

    let (target_mode, title, ask) = match action {
        HandoffActionKind::DeriveInSolve => {
            let (task_kind, form) = solve_kind_and_form(node.kind);
            let symbols = collect_relevant_symbols(&rec.skeleton, node, &closure_tags, &stmt_text, &context);
            let desc = non_empty_description(&node_label, &stmt_text);
            let title = clamp_chars(&format!("Derive in Solve — {node_label} ({tag})"), MAX_TITLE_CHARS);
            let ask = HandoffAsk::Solve(SolveHandoffAsk {
                task_kind,
                target: SolveTarget { description: desc, form, order: None, digits: None },
                symbols,
                suggested_done: Vec::new(),
                check_first: Vec::new(),
                risks: Vec::new(),
                stop_if: Vec::new(),
            });
            (Mode::Solve, title, ask)
        }
        HandoffActionKind::HasBeenExtended => {
            let title = clamp_chars(&format!("Has this been extended? — {node_label} ({tag})"), MAX_TITLE_CHARS);
            let idea_text = clamp_chars(
                &format!("Has this result been extended or generalized? {}: {}", node_label, non_empty_preview(&stmt_text, &node_label)),
                MAX_STATEMENT_CHARS,
            );
            let ask = HandoffAsk::Explore(ExploreHandoffAsk {
                idea_text,
                question_kind: ExploreQuestionKind::Extended,
                origin_paper: Some(origin_paper.clone()),
                seed_papers,
                focus_claims: vec![node_label],
            });
            (Mode::Explore, title, ask)
        }
        HandoffActionKind::SearchWiderInExplore => {
            let title = clamp_chars(&format!("Search wider in Explore — {node_label} ({tag})"), MAX_TITLE_CHARS);
            let idea_text = clamp_chars(
                &format!("Is this result earlier? Search wider in literature for {}: {}", node_label, non_empty_preview(&stmt_text, &node_label)),
                MAX_STATEMENT_CHARS,
            );
            let ask = HandoffAsk::Explore(ExploreHandoffAsk {
                idea_text,
                question_kind: ExploreQuestionKind::PriorWork,
                origin_paper: Some(origin_paper.clone()),
                seed_papers,
                focus_claims: vec![node_label],
            });
            (Mode::Explore, title, ask)
        }
    };

    let packet = HandoffPacket {
        from: NodeRef {
            session: session_id.to_string(),
            mode: Mode::Investigate,
            id: tag.to_string(),
            paper: Some(origin_paper),
            node_fp: Some(node.fp.clone()),
        },
        target_mode,
        title,
        statement: HandoffStatement { text: stmt_text, quotes: stmt_quotes },
        context,
        papers,
        conventions,
        model_text,
        user_notes,
        ask,
    };
    packet.validate().map_err(internal)?;
    Ok(HandoffRequest { packet, quote_texts, extra_source_ids: vec![rec.skeleton.source_id] })
}
