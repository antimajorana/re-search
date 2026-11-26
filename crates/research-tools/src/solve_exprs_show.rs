//! Read-only `expr_show` tool handler and handle query resolution (package S7).
//! Re-exported from `solve_exprs` so callers import everything from `research_tools::solve_exprs`.

use crate::solve_exprs::{
    clip_utf8_chars, load_handle_srepr, validate_expr_name, MAX_EXPR_SHOW_CHARS,
};
use crate::solve_model::{HandleRef, SolveExpr, MAX_STR_PREVIEW_CHARS};
use crate::solve_state::SolveState;
use ledger::Ledger;
use serde_json::{json, Value};
use std::sync::Arc;
use tools::{CallCtx, ToolHandler};

/// Read-only `expr_show` tool handler (`solve.md` 5.1, package S7).
pub fn expr_show_handler() -> Arc<dyn ToolHandler> {
    Arc::new(|ctx: &CallCtx<'_>, args: &Value| {
        let ledger = ctx.ledger.lock().map_err(|_| "ledger lock poisoned".to_string())?;
        run_expr_show(&ledger, ctx.session, args)
    })
}

/// Execute `expr_show` against `ledger` and `session` without writing any ledger events.
pub fn run_expr_show(ledger: &Ledger, session: &str, args: &Value) -> Result<Value, String> {
    let max_chars = match args.get("max_chars") {
        None | Some(Value::Null) => MAX_EXPR_SHOW_CHARS,
        Some(v) => {
            let n = v.as_i64().ok_or("`max_chars` must be an integer")?;
            if !(1..=MAX_EXPR_SHOW_CHARS as i64).contains(&n) {
                return Err(format!("`max_chars` must be between 1 and {MAX_EXPR_SHOW_CHARS}"));
            }
            n as usize
        }
    };
    let form = match args.get("form") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let s = v.as_str().ok_or("`form` must be a string")?;
            if !matches!(s, "latex" | "str" | "srepr") {
                return Err(format!("`form` must be one of `latex`, `str`, or `srepr` (got `{s}`)"));
            }
            Some(s)
        }
    };
    let include_srepr = match args.get("include_srepr") {
        None | Some(Value::Null) => false,
        Some(v) => v.as_bool().ok_or("`include_srepr` must be a boolean")?,
    };
    let explicit_version = match args.get("version") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let n = v.as_i64().ok_or("`version` must be an integer")?;
            if n < 1 || n > u32::MAX as i64 {
                return Err("`version` must be a positive integer".to_string());
            }
            Some(n as u32)
        }
    };
    let raw_name = match args.get("name") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str().ok_or("`name` must be a string")?),
    };

    let events = ledger.read_session(session).map_err(|e| e.to_string())?;
    let state = SolveState::from_events(&events);

    let Some(raw_name) = raw_name else {
        if explicit_version.is_some() {
            return Err("`version` requires `name`".to_string());
        }
        let handles: Vec<Value> = state
            .handles
            .iter()
            .map(|(href, expr)| {
                let (latex, _) = clip_utf8_chars(&expr.latex, max_chars);
                let (preview, _) =
                    clip_utf8_chars(&expr.str_preview, max_chars.min(MAX_STR_PREVIEW_CHARS));
                json!({
                    "handle": href.to_string(),
                    "name": expr.name,
                    "version": expr.version,
                    "latex": latex,
                    "str_preview": preview,
                    "free_symbols": expr.free_symbols,
                    "portable": expr.portable,
                    "srepr_sha256": expr.srepr_sha256,
                    "produced_by": expr.produced_by,
                    "owner_entry": expr.owner_entry,
                })
            })
            .collect();
        return Ok(json!({ "count": handles.len(), "handles": handles }));
    };

    let (target_ref, expr) = resolve_handle_query(&state, raw_name, explicit_version)?;
    let (latex, latex_cut) = clip_utf8_chars(&expr.latex, max_chars);
    let (preview, preview_cut) =
        clip_utf8_chars(&expr.str_preview, max_chars.min(MAX_STR_PREVIEW_CHARS));
    let mut truncated = latex_cut || preview_cut;

    let srepr_opt = if include_srepr || form == Some("srepr") {
        match load_handle_srepr(ledger, expr)? {
            Some(raw) => {
                let (clipped, cut) = clip_utf8_chars(&raw, max_chars);
                truncated |= cut;
                Some(clipped)
            }
            None => Some(String::new()),
        }
    } else {
        None
    };

    let mut out = json!({
        "handle": target_ref.to_string(),
        "name": expr.name,
        "version": expr.version,
        "latex": latex,
        "str_preview": preview,
        "free_symbols": expr.free_symbols,
        "portable": expr.portable,
        "srepr_sha256": expr.srepr_sha256,
        "produced_by": expr.produced_by,
        "owner_entry": expr.owner_entry,
        "truncated": truncated,
    });
    if !expr.portable {
        out["note"] = json!("handle is non-portable; canonical srepr and latex are unavailable");
    }
    if let Some(srepr) = srepr_opt {
        out["srepr"] = json!(srepr);
    }
    if let Some(f) = form {
        let text = match f {
            "latex" => out["latex"].clone(),
            "str" => out["str_preview"].clone(),
            "srepr" => out["srepr"].clone(),
            _ => Value::String(String::new()),
        };
        out["form"] = json!(f);
        out["text"] = text;
    }
    Ok(out)
}

fn resolve_handle_query<'a>(
    state: &'a SolveState,
    raw_name: &str,
    explicit_version: Option<u32>,
) -> Result<(HandleRef, &'a SolveExpr), String> {
    if raw_name.contains('@') {
        let parsed = HandleRef::parse(raw_name)?;
        validate_expr_name(&parsed.name)?;
        if let Some(ver) = explicit_version {
            if ver != parsed.version {
                return Err(format!("conflicting handle version: `{raw_name}` vs `version: {ver}`"));
            }
        }
        let expr = state
            .handles
            .get(&parsed)
            .ok_or_else(|| format!("unknown expression handle `{parsed}`"))?;
        return Ok((parsed, expr));
    }
    validate_expr_name(raw_name)?;
    if let Some(ver) = explicit_version {
        let parsed = HandleRef::new(raw_name.to_string(), ver)?;
        let expr = state
            .handles
            .get(&parsed)
            .ok_or_else(|| format!("unknown expression handle `{parsed}`"))?;
        return Ok((parsed, expr));
    }
    state
        .handles
        .iter()
        .filter(|(h, _)| h.name == raw_name)
        .max_by_key(|(h, _)| h.version)
        .map(|(h, e)| (h.clone(), e))
        .ok_or_else(|| format!("unknown expression handle `{raw_name}`"))
}
