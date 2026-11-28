//! Tool-call dispatch, session-title capture, and text helpers shared by agent loops.

use crate::runner::{Audit, LoopParams, SESSION_TITLE_EVENT};
use crate::transport::{CancelToken, ContentBlock, ToolSpec};
use serde_json::{json, Value};
use std::collections::HashSet;
use tools::builtin::SET_SESSION_TITLE;
use tools::{CallOutcome, Router};

pub(crate) const RESULT_TEXT_LIMIT: usize = 4000;
pub(crate) const NOT_RUN_CAP_TEXT: &str = "NOT RUN: model-call cap reached";

pub(crate) fn has_allowlisted_tool(content: &[ContentBlock], allowlist: &[&str]) -> bool {
    !allowlist.is_empty()
        && content.iter().any(|block| {
            matches!(block, ContentBlock::ToolUse { name, .. } if allowlist.contains(&name.as_str()))
        })
}

pub(crate) fn filter_turn_tools(
    tool_specs: &[ToolSpec],
    is_final_turn: bool,
    allowlist: &[&str],
) -> Vec<ToolSpec> {
    if is_final_turn && !allowlist.is_empty() {
        tool_specs
            .iter()
            .filter(|spec| allowlist.contains(&spec.name.as_str()))
            .cloned()
            .collect()
    } else {
        tool_specs.to_vec()
    }
}

pub(crate) fn request_tool_surface<'a>(
    tools: &'a [ToolSpec],
    host_tool_set: &HashSet<String>,
) -> (Vec<&'a str>, Vec<&'a str>) {
    let tool_names = tools.iter().map(|spec| spec.name.as_str()).collect();
    let host_tools = tools
        .iter()
        .filter(|spec| host_tool_set.contains(&spec.name))
        .map(|spec| spec.name.as_str())
        .collect();
    (tool_names, host_tools)
}

pub(crate) fn format_final_call_notice(allowlist: &[&str]) -> String {
    let names = allowlist
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join("/");
    format!(
        "[harness notice, not from the user] Final model call next: only {names} will run. \
         Submit all remaining items now with done: true."
    )
}

pub(crate) fn emit_final_call_notice(
    audit: &mut Audit<'_>,
    next_turn: u32,
    allowlist: &[&str],
    results: &mut Vec<ContentBlock>,
) -> Result<(), String> {
    audit.emit("session.notice", json!({ "kind": "final_call_next", "turn": next_turn }))?;
    results.push(ContentBlock::Text { text: format_final_call_notice(allowlist) });
    Ok(())
}

pub(crate) fn dispatch_tool_calls(
    audit: &mut Audit<'_>,
    router: &Router,
    caller: &str,
    turn: u32,
    content: &[ContentBlock],
    params: &LoopParams<'_>,
    use_title_tool: bool,
) -> Result<Vec<ContentBlock>, String> {
    dispatch_tool_calls_with_allowlist(audit, router, caller, turn, content, params, use_title_tool, None)
}

pub(crate) fn dispatch_final_turn_tools(
    audit: &mut Audit<'_>,
    router: &Router,
    caller: &str,
    turn: u32,
    content: &[ContentBlock],
    params: &LoopParams<'_>,
    use_title_tool: bool,
) -> Result<Vec<ContentBlock>, String> {
    dispatch_tool_calls_with_allowlist(
        audit,
        router,
        caller,
        turn,
        content,
        params,
        use_title_tool,
        Some(params.final_turn_tools),
    )
}

#[allow(clippy::too_many_arguments)]
fn dispatch_tool_calls_with_allowlist(
    audit: &mut Audit<'_>,
    router: &Router,
    caller: &str,
    turn: u32,
    content: &[ContentBlock],
    params: &LoopParams<'_>,
    use_title_tool: bool,
    allowlist: Option<&[&str]>,
) -> Result<Vec<ContentBlock>, String> {
    let mut results = Vec::new();
    for block in content {
        let ContentBlock::ToolUse { id, name, input } = block else {
            continue;
        };
        let (is_error, content_text) = if let Some(allowed) = allowlist {
            if !allowed.contains(&name.as_str()) {
                (true, NOT_RUN_CAP_TEXT.to_string())
            } else {
                dispatch_single_tool(audit, router, caller, id, name, input, params, use_title_tool)?
            }
        } else {
            dispatch_single_tool(audit, router, caller, id, name, input, params, use_title_tool)?
        };
        let (clipped, truncated) = clip_text(&content_text);
        audit.emit(
            "model.tool_result",
            json!({
                "turn": turn,
                "tool_use_id": id,
                "is_error": is_error,
                "text": clipped,
                "truncated": truncated,
            }),
        )?;
        results.push(ContentBlock::ToolResult {
            tool_use_id: id.clone(),
            is_error,
            content: content_text,
        });
    }
    Ok(results)
}

#[allow(clippy::too_many_arguments)]
fn dispatch_single_tool(
    audit: &mut Audit<'_>,
    router: &Router,
    caller: &str,
    id: &str,
    name: &str,
    input: &Value,
    params: &LoopParams<'_>,
    use_title_tool: bool,
) -> Result<(bool, String), String> {
    if params.cancel.is_some_and(CancelToken::is_cancelled) {
        return Ok((true, "DENIED: stopped by user".to_string()));
    }
    if name == SET_SESSION_TITLE && use_title_tool {
        return record_session_title(audit, input);
    }
    if crate::interaction::is_human_tool(name) {
        return crate::interaction::run_ask_user_tool(audit, params.human, id, input, params.cancel);
    }
    if crate::delegate::is_delegation_tool(name) {
        let res = match params.delegation {
            Some(dcfg) => crate::delegate::run_delegation_call(audit, router, dcfg, id, name, input, params.cancel)?,
            None => (true, "DENIED: delegation is not enabled for this session".to_string()),
        };
        return if name == crate::explore::DELEGATE_EXPLORE {
            crate::interaction::maybe_pause_explore_branch_picker(
                audit,
                params.human,
                params.pause_after_explore,
                id,
                res,
                params.cancel,
            )
        } else {
            Ok(res)
        };
    }
    let outcome = router.try_call(caller, name, input)?;
    audit.flush_interleaved()?;
    Ok(match outcome {
        CallOutcome::Ok(val) => (false, tools::canon::canonical_json(&val)),
        CallOutcome::Denied(why) => (true, format!("DENIED: {why}")),
        CallOutcome::Error(why) => (true, format!("ERROR: {why}")),
    })
}

fn record_session_title(audit: &mut Audit<'_>, input: &Value) -> Result<(bool, String), String> {
    if !audit.sink.wants_session_title()
        || audit
            .ledger
            .read_session(audit.session)
            .map_err(|error| error.to_string())?
            .iter()
            .any(|event| event.kind == SESSION_TITLE_EVENT)
    {
        return Ok((false, "The session title is already fixed.".into()));
    }
    let title = match tools::builtin::normalize_session_title(input) {
        Ok(title) => title,
        Err(reason) => return Ok((true, reason)),
    };
    audit.emit(SESSION_TITLE_EVENT, json!({ "title": title }))?;
    Ok((false, "Session title set.".into()))
}

pub(crate) fn collect_text(content: &[ContentBlock]) -> String {
    content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn clip_text(text: &str) -> (String, bool) {
    if text.chars().count() <= RESULT_TEXT_LIMIT {
        (text.to_string(), false)
    } else {
        (text.chars().take(RESULT_TEXT_LIMIT).collect(), true)
    }
}
