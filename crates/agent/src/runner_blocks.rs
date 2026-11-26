//! Content-block recording, turn validation, session title, and text helpers
//! extracted from `runner.rs` to keep `runner.rs` within the 600-line limit.

use super::{Audit, RESULT_TEXT_LIMIT, SESSION_TITLE_EVENT};
use crate::transport::{ContentBlock, ModelResponse, StopReason};
use serde_json::{json, Value};
use std::collections::HashSet;

pub(super) fn record_content_blocks(audit: &mut Audit<'_>, content: &[ContentBlock]) -> Result<(), String> {
    for block in content {
        let (kind, data) = match block {
            ContentBlock::Text { text } => ("model.text", json!({ "text": text })),
            ContentBlock::Thinking { thinking, .. } => ("model.thinking", json!({ "chars": thinking.chars().count() })),
            ContentBlock::RedactedThinking { data } => {
                ("model.thinking", json!({ "chars": data.chars().count(), "redacted": true }))
            }
            ContentBlock::ToolUse { id, name, input } => {
                ("model.tool_use", json!({ "id": id, "name": name, "input": input }))
            }
            ContentBlock::ToolResult { .. } => ("model.other", json!({ "block_type": "tool_result" })),
            ContentBlock::Image { media_type, .. } => {
                ("model.other", json!({ "block_type": "image", "media_type": media_type }))
            }
            ContentBlock::Other { block_type, .. } => ("model.other", json!({ "block_type": block_type })),
        };
        audit.emit(kind, data)?;
    }
    Ok(())
}

pub(crate) fn validate_turn(resp: &ModelResponse) -> Option<String> {
    match &resp.stop_reason {
        StopReason::MaxTokens => return Some("model response truncated at max_tokens".into()),
        StopReason::PauseTurn => return Some("model returned unsupported stop_reason `pause_turn`".into()),
        StopReason::Other(s) => return Some(format!("model returned unknown stop_reason `{s}`")),
        StopReason::EndTurn | StopReason::StopSequence | StopReason::ToolUse | StopReason::Refusal => {}
    }

    let mut seen_ids = HashSet::new();
    let mut tool_count = 0usize;
    for block in &resp.content {
        match block {
            ContentBlock::ToolUse { id, name, .. } => {
                tool_count += 1;
                if id.trim().is_empty() {
                    return Some("model emitted tool_use block with empty id".into());
                }
                if name.trim().is_empty() {
                    return Some("model emitted tool_use block with empty name".into());
                }
                if !seen_ids.insert(id.as_str()) {
                    return Some(format!("model emitted duplicate tool_use id `{id}`"));
                }
            }
            ContentBlock::ToolResult { .. } => {
                return Some("model emitted unexpected tool_result block in assistant message".into());
            }
            _ => {}
        }
    }

    match resp.stop_reason {
        StopReason::ToolUse if tool_count == 0 => {
            Some("model returned stop_reason `tool_use` without any tool_use blocks".into())
        }
        StopReason::EndTurn | StopReason::StopSequence if tool_count > 0 => Some(format!(
            "model returned `{}` with {} tool_use block(s)",
            resp.stop_reason.as_str(),
            tool_count
        )),
        _ => None,
    }
}

pub(super) fn record_session_title(audit: &mut Audit<'_>, input: &Value) -> Result<(bool, String), String> {
    if !audit.sink.wants_session_title()
        || audit.ledger.read_session(audit.session).map_err(|error| error.to_string())?
            .iter().any(|event| event.kind == SESSION_TITLE_EVENT)
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

pub(super) fn collect_text(content: &[ContentBlock]) -> String {
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
