//! Tool-call dispatch, session-title capture, and text helpers shared by agent loops.

use crate::runner::{Audit, LoopParams, SESSION_TITLE_EVENT};
use crate::transport::{CancelToken, ContentBlock};
use serde_json::{json, Value};
use tools::builtin::SET_SESSION_TITLE;
use tools::{CallOutcome, Router};

pub(crate) const RESULT_TEXT_LIMIT: usize = 4000;

pub(crate) fn dispatch_tool_calls(
    audit: &mut Audit<'_>,
    router: &Router,
    caller: &str,
    turn: u32,
    content: &[ContentBlock],
    params: &LoopParams<'_>,
    use_title_tool: bool,
) -> Result<Vec<ContentBlock>, String> {
    let mut results = Vec::new();
    for block in content {
        let ContentBlock::ToolUse { id, name, input } = block else {
            continue;
        };
        let (is_error, content_text) = if params.cancel.is_some_and(CancelToken::is_cancelled) {
            (true, "DENIED: stopped by user".to_string())
        } else if name == SET_SESSION_TITLE && use_title_tool {
            record_session_title(audit, input)?
        } else if crate::interaction::is_human_tool(name) {
            crate::interaction::run_ask_user_tool(audit, params.human, id, input, params.cancel)?
        } else if crate::delegate::is_delegation_tool(name) {
            let res = match params.delegation {
                Some(dcfg) => crate::delegate::run_delegation_call(audit, router, dcfg, id, name, input, params.cancel)?,
                None => (true, "DENIED: delegation is not enabled for this session".to_string()),
            };
            if name == crate::explore::DELEGATE_EXPLORE {
                crate::interaction::maybe_pause_explore_branch_picker(
                    audit,
                    params.human,
                    params.pause_after_explore,
                    id,
                    res,
                    params.cancel,
                )?
            } else {
                res
            }
        } else {
            let outcome = router.try_call(caller, name, input)?;
            audit.flush_interleaved()?;
            match outcome {
                CallOutcome::Ok(val) => (false, tools::canon::canonical_json(&val)),
                CallOutcome::Denied(why) => (true, format!("DENIED: {why}")),
                CallOutcome::Error(why) => (true, format!("ERROR: {why}")),
            }
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
