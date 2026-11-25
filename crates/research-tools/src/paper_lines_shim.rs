//! Shim for `split-1`'s `paper_lines.rs` (Section 2.3 of 01-reader-recorder-split.md).
//!
//! Used by the isolated paper-reader router (`register_reader_tools`) with a 7,000-char
//! canonical JSON budget. At merge, the coordinator replaces this shim with `split-1`'s
//! `paper_lines.rs`.

use crate::investigation_anchor_shim::{SourceIndex, MAX_TOTAL_BYTES};
use crate::ResearchEnv;
use ledger::Ledger;
use policy::EffectClass;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;
use tools::{ApprovalScope, CallCtx, Registry, ToolDef};

pub const READ_PAPER_LINES: &str = "read_paper_lines";
const MAX_LINES_PER_CALL: usize = 400;

pub fn def() -> ToolDef {
    ToolDef {
        id: READ_PAPER_LINES.into(),
        description: "Read consecutive 1-based lines from an ingested paper in this session. Returns line-numbered text (`N\\t<line>`) with verified file metadata and page ranges, cut cleanly at a line boundary to fit the output budget.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "source_id": { "type": "string", "minLength": 1, "maxLength": 64 },
                "start_line": { "type": "integer", "minimum": 1 },
                "end_line": { "type": "integer", "minimum": 1 }
            },
            "required": ["source_id", "start_line", "end_line"],
            "additionalProperties": false
        }),
        effect: EffectClass::ReadWorkspace,
        approval_scope: ApprovalScope::Once,
    }
}

pub fn register_with_budget(
    registry: &mut Registry,
    env: Arc<ResearchEnv>,
    max_json_chars: usize,
) {
    registry.register(
        def(),
        Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
            let ledger = ctx.ledger.lock().map_err(|_| "ledger lock poisoned")?;
            run_read_lines(&env, &ledger, ctx.session, args, max_json_chars)
        }),
    );
}

pub fn run_read_lines(
    env: &ResearchEnv,
    ledger: &Ledger,
    session: &str,
    args: &Value,
    max_json_chars: usize,
) -> Result<Value, String> {
    crate::validate_session_id(session).map_err(|e| e.to_string())?;
    tools::registry::check_args(&def().input_schema, args)?;
    let source_id = args["source_id"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 64)
        .ok_or("Invalid source_id.")?;
    let start_line = args["start_line"]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .filter(|&n| n >= 1)
        .ok_or("start_line must be >= 1.")?;
    let end_line = args["end_line"]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .filter(|&n| n >= start_line)
        .ok_or("end_line must be >= start_line.")?;
    if end_line - start_line + 1 > MAX_LINES_PER_CALL {
        return Err(format!(
            "Requested range exceeds {MAX_LINES_PER_CALL} lines per call."
        ));
    }

    let (_, events) = ledger
        .read_verified_session(session)
        .map_err(|_| "The session ledger or a recorded artifact could not be verified.")?;
    let receipts: BTreeMap<_, _> = events
        .iter()
        .filter(|e| e.kind == "source.ingested")
        .filter_map(|e| Some((e.payload["source_id"].as_str()?, e)))
        .collect();
    let receipt = receipts
        .get(source_id)
        .copied()
        .ok_or("Source has not been ingested in this session.")?;
    let path = receipt.payload["primary_path"]
        .as_str()
        .ok_or("Source receipt is missing primary_path.")?
        .to_string();
    if receipt.payload["assembled_path"].as_str() == Some(path.as_str())
        && receipt.payload["format"].as_str() == Some("tex")
    {
        return Err("Refusing assembled TeX view; use original source files.".into());
    }

    let mut index = SourceIndex {
        sandbox: env.sandbox(),
        session,
        primary_source_id: source_id,
        receipts,
        files: BTreeMap::new(),
        bytes_left: MAX_TOTAL_BYTES,
        pages: None,
        history: &events,
        active_marker_seq: None,
    };
    let file = index.load_path(source_id, &path)?;
    let total_lines = file.line_count();
    if start_line > total_lines {
        return Err(format!(
            "start_line {start_line} is beyond end of file ({total_lines} lines)."
        ));
    }
    let req_end = end_line.min(total_lines);
    let page_lines: Vec<(usize, usize, usize)> = receipt
        .payload
        .get("pdf_page_lines")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let pages_for = |a: usize, b: usize| -> Vec<Value> {
        page_lines
            .iter()
            .filter(|&&(_, pf, pl)| pl >= a && pf <= b)
            .map(|&(p, pf, pl)| json!([p, pf, pl]))
            .collect()
    };

    let build_payload = |delivered_end: usize, text: &str, truncated: bool| -> Value {
        let mut meta = json!({
            "path": path,
            "file_sha256": file.sha256,
            "source_seq": file.source_seq,
            "start_line": start_line,
            "end_line": delivered_end,
            "total_lines": total_lines,
            "pages": pages_for(start_line, delivered_end),
        });
        if delivered_end < req_end {
            meta["next_line"] = json!(delivered_end + 1);
        }
        if truncated {
            meta["line_truncated"] = json!(true);
        }
        json!({ "a_meta": meta, "text": text })
    };

    let mut text = String::new();
    let mut delivered_end = start_line - 1;
    let mut last_fit: Option<Value> = None;
    for ln in start_line..=req_end {
        let line = file.slice_lines(ln, ln).unwrap_or("");
        let row = format!("{ln}\t{line}\n");
        let prev_len = text.len();
        text.push_str(&row);
        let candidate = build_payload(ln, &text, false);
        if tools::canon::canonical_json(&candidate).chars().count() <= max_json_chars {
            delivered_end = ln;
            last_fit = Some(candidate);
        } else {
            text.truncate(prev_len);
            break;
        }
    }

    if let Some(fit) = last_fit {
        return Ok(fit);
    }

    // Even the single first line exceeded budget: truncate it cleanly to fit.
    let first_line = file.slice_lines(start_line, start_line).unwrap_or("");
    let prefix = format!("{start_line}\t");
    let mut hi = first_line.chars().count();
    let mut lo = 0usize;
    let mut best = build_payload(start_line, &format!("{prefix}\n"), true);
    while lo <= hi {
        let mid = lo + (hi - lo) / 2;
        let truncated_body: String = first_line.chars().take(mid).collect();
        let candidate = build_payload(start_line, &format!("{prefix}{truncated_body}\n"), true);
        if tools::canon::canonical_json(&candidate).chars().count() <= max_json_chars {
            best = candidate;
            lo = mid + 1;
        } else if mid == 0 {
            break;
        } else {
            hi = mid - 1;
        }
    }
    let _ = delivered_end;
    Ok(best)
}
