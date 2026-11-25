//! Bounded, 1-based line reader over hash-pinned session paper sources (`read_paper_lines`).
use crate::investigation::anchor::{valid_id, SourceIndex, MAX_TOTAL_BYTES};
use crate::{resolve_sources_session_id, ResearchEnv};
use ledger::Ledger;
use policy::EffectClass;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;
use tools::{ApprovalScope, CallCtx, Registry, ToolDef};

pub const READ_PAPER_LINES: &str = "read_paper_lines";
pub const MAIN_BUDGET_CHARS: usize = 14_000;
pub const READER_BUDGET_CHARS: usize = 7_000;
const MAX_LINES_PER_CALL: usize = 400;

pub fn def() -> ToolDef {
    ToolDef {
        id: READ_PAPER_LINES.into(),
        description: "Read 1-based numbered lines from an ingested paper in this session (`start_line`..=`end_line`, at most 400 lines). Output is cut at a line boundary to fit the tool's character budget; check `a_meta.end_line` and `a_meta.next_line` for exact delivery.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "source_id": { "type": "string", "minLength": 1, "maxLength": 64,
                    "description": "Ingested source ID in this session." },
                "path": { "type": "string", "minLength": 1, "maxLength": 1024,
                    "description": "Optional file path inside the source; defaults to the primary source file." },
                "start_line": { "type": "integer", "minimum": 1, "description": "First 1-based line to read." },
                "end_line": { "type": "integer", "minimum": 1, "description": "Last 1-based line to read (at most 400 lines per call)." }
            },
            "required": ["source_id", "start_line", "end_line"],
            "additionalProperties": false
        }),
        effect: EffectClass::ReadWorkspace,
        approval_scope: ApprovalScope::Once,
    }
}

pub fn register(registry: &mut Registry, env: Arc<ResearchEnv>) {
    register_with_budget(registry, env, MAIN_BUDGET_CHARS);
}

pub fn register_with_budget(registry: &mut Registry, env: Arc<ResearchEnv>, max_chars: usize) {
    registry.register(
        def(),
        Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
            let ledger = ctx.ledger.lock().map_err(|_| "ledger lock poisoned")?;
            read_lines(&env, &ledger, ctx.session, args, max_chars)
        }),
    );
}

pub fn read_lines(
    env: &ResearchEnv,
    ledger: &Ledger,
    session: &str,
    args: &Value,
    max_chars: usize,
) -> Result<Value, String> {
    let session = resolve_sources_session_id(session).map_err(|e| e.to_string())?;
    tools::registry::check_args(&def().input_schema, args)?;
    let source_id = args["source_id"]
        .as_str()
        .filter(|s| valid_id(s, 64))
        .ok_or("Invalid source_id.")?;
    let start_line = args["start_line"]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .filter(|&n| n >= 1)
        .ok_or("start_line must be a positive integer.")?;
    let end_line = args["end_line"]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .filter(|&n| n >= start_line)
        .ok_or("end_line must be >= start_line.")?;
    if end_line - start_line >= MAX_LINES_PER_CALL {
        return Err(format!(
            "read_paper_lines reads at most {MAX_LINES_PER_CALL} lines per call."
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
        .ok_or("This source_id does not belong to an ingested source in this session.")?;
    let path = match args.get("path").and_then(Value::as_str) {
        Some(p) => p,
        None => receipt.payload["primary_path"]
            .as_str()
            .ok_or("The source has no readable primary text file.")?,
    };
    if receipt.payload["assembled_path"].as_str() == Some(path) {
        return Err(
            "Use the paper's original source file. An assembled TeX view has generated text and different line numbers."
                .into(),
        );
    }
    let page_lines: Vec<(usize, usize, usize)> = if receipt.payload["format"].as_str() == Some("pdf") {
        crate::pdf::verified_page_lines(&receipt.payload, || {
            let blob = receipt.payload["raw_blob"]
                .as_str()
                .ok_or("Original PDF unavailable.")?;
            ledger.get_blob(blob).map_err(|e| e.to_string())
        })
        .unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut index = SourceIndex {
        sandbox: env.sandbox(),
        session,
        primary_source_id: source_id,
        receipts,
        files: BTreeMap::new(),
        bytes_left: MAX_TOTAL_BYTES,
        pages: None,
    };
    let file = index.load(source_id, path)?;
    let total_lines = file.line_count();
    if start_line > total_lines {
        return Err(format!(
            "Line {start_line} is outside {path} ({total_lines} lines)."
        ));
    }
    let req_end = end_line.min(total_lines);
    let build = |delivered_end: usize, text: String, line_truncated: bool| -> Value {
        let pages: Vec<Value> = page_lines
            .iter()
            .filter(|&&(_, first, last)| last >= start_line && first <= delivered_end)
            .map(|&(p, first, last)| json!([p, first, last]))
            .collect();
        let mut meta = json!({
            "path": path,
            "file_sha256": file.sha256,
            "source_seq": file.source_seq,
            "start_line": start_line,
            "end_line": delivered_end,
            "total_lines": total_lines,
            "pages": pages,
        });
        if delivered_end < total_lines {
            meta["next_line"] = json!(delivered_end + 1);
        }
        if line_truncated {
            meta["line_truncated"] = json!(true);
        }
        json!({ "a_meta": meta, "text": text })
    };
    let format_range = |count: usize| -> String {
        let mut out = String::new();
        for n in start_line..(start_line + count) {
            out.push_str(&format!("{n}\t{}\n", file.line_str(n)));
        }
        out
    };
    let fits = |val: &Value| tools::canon::canonical_json(val).chars().count() <= max_chars;

    let max_count = req_end - start_line + 1;
    let first_val = build(start_line, format_range(1), false);
    if !fits(&first_val) {
        // Single line exceeds budget: binary search character prefix and set line_truncated=true.
        let line = file.line_str(start_line);
        let char_offsets: Vec<usize> = line.char_indices().map(|(i, _)| i).collect();
        let (mut lo, mut hi) = (0usize, char_offsets.len());
        while lo < hi {
            let mid = lo + (hi - lo).div_ceil(2);
            let byte_end = char_offsets.get(mid).copied().unwrap_or(line.len());
            let cand = build(start_line, format!("{start_line}\t{}\n", &line[..byte_end]), true);
            if fits(&cand) {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        let byte_end = char_offsets.get(lo).copied().unwrap_or(line.len());
        return Ok(build(
            start_line,
            format!("{start_line}\t{}\n", &line[..byte_end]),
            true,
        ));
    }

    let (mut lo, mut hi) = (1usize, max_count);
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        let cand = build(start_line + mid - 1, format_range(mid), false);
        if fits(&cand) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    Ok(build(start_line + lo - 1, format_range(lo), false))
}
