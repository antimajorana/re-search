//! Open an earlier Research session in Solve (`solve.md` 8.4 & D19, package S21).
//!
//! `POST /api/sessions/{id}/open-in-solve` forks an earlier `Mode::Research` session up to
//! `at_seq` (leaving the original session untouched so it can still resume under `research/1`),
//! copies its ledger prefix and ingested `sources/` directory, appends `solve.started` and
//! `solve.setup` carrying `origin: NodeRef { session, mode: Research, id: "seq:<at_seq>", ... }`,
//! and returns the initial Solve view, D19 tiers, and legacy `lean_check` / `sympy_eval` receipts.

use crate::projects::{attach_session, project_for_session};
use crate::routes_session::{bad, conflict, internal, not_found, parse_json_body, session_title, ApiResult};
use crate::session_lifecycle::is_turn_running;
use crate::session_mode::{api_mode_name, mode_label, session_mode};
use crate::solve::session_solve_view;
use crate::state::{now_ms, DesktopState, ForkInfo, SessionMeta, UiSettings};
use agent::ProviderKind;
use export::{fork_session, ForkOptions};
use ledger::{Event, Ledger};
use research_tools::paper_key::{Mode, NodeRef};
use research_tools::solve_model::{
    event_kinds, RoleModel, SetupAssumption, SolveRoles, SolveSetup, StepEntryType, TaskKind,
    MAX_CONVENTIONS_CHARS, MAX_STATEMENT_CHARS,
};
use research_tools::solve_receipts::build_receipts;
use research_tools::solve_setup::{
    prefill_solve_setup, validate_and_build_setup, PrefillRequest, SolveSetupInput,
    MAX_ASSUMPTIONS, MAX_SOURCES,
};
use research_tools::solve_state::SolveState;
use research_tools::solve_status::compute_all_tiers;
use research_tools::tool_profile::MARKER_SOLVE_STARTED;
use research_tools::validate_session_id;
use serde_json::{json, Value};
use std::collections::BTreeSet;

/// Receipt display label for an earlier Research session's `lean_check` call (`solve.md` D19).
pub const LEGACY_LEAN_CHECK_RECEIPT_LABEL: &str = "Lean check recorded in the earlier session";
/// Receipt display note for an earlier Research session's `sympy_eval` call (`solve.md` D19).
pub const LEGACY_SYMPY_RECEIPT_LABEL: &str =
    "SymPy validation recorded in the earlier session (receipt only; no computed tier without handles)";
/// Default fallback statement prefix when an earlier Research session has no recorded prompt.
const FALLBACK_STATEMENT_PREFIX: &str = "Continued from earlier Research session ";
const MAX_ASSUMPTION_TEXT_CHARS: usize = 2_000;
const MAX_ASSUMPTION_ID_CHARS: usize = 64;
const MAX_ROLE_CHARS: usize = 200;
const ALLOWED_OPTION_KEYS: &[&str] = &["at_seq", "statement", "conventions", "task_kind", "solver", "verifier"];

struct OpenInSolveOptions {
    at_seq: Option<u64>,
    statement: Option<String>,
    conventions: Option<String>,
    task_kind: Option<TaskKind>,
    solver: Option<RoleModel>,
    verifier: Option<RoleModel>,
}

/// `POST /api/sessions/{id}/open-in-solve`: fork an earlier Research session into Solve mode.
pub fn handle_open_in_solve(state: &DesktopState, session_id: &str, body: &[u8]) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let val = parse_json_body(body)?;
    if !val.is_object() {
        return Err(bad("request body must be a JSON object"));
    }
    let opts = parse_options(&val)?;

    let parent_meta = state.load_meta(session_id);
    let parent_project = project_for_session(state, session_id)?;
    let ui = state.load_ui_settings();
    let ledger = state.open_ledger().map_err(internal)?;
    let mut warnings: Vec<String> = Vec::new();

    let (receipt, origin, setup, child_events, prefix_events) = {
        let _guard = state.ledger_write_lock.lock().map_err(|_| internal("ledger write lock poisoned"))?;
        if is_turn_running(state, session_id) {
            return Err(conflict("Cannot open a Research session in Solve while a turn is running."));
        }
        let parent_events = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
        if parent_events.is_empty() {
            return Err(bad("Research session has no ledger events to open in Solve."));
        }
        let full_mode = session_mode(&parent_events);
        if full_mode != Mode::Research {
            return Err(bad(format!(
                "Only earlier Research sessions can be opened in Solve (session `{session_id}` is {}).",
                mode_label(full_mode)
            )));
        }
        let default_seq = parent_events.last().map(|e| e.seq).unwrap_or(1);
        let at_seq = opts.at_seq.unwrap_or(default_seq);
        let prefix_len = usize::try_from(at_seq)
            .ok()
            .filter(|&len| len > 0 && len <= parent_events.len())
            .ok_or_else(|| bad(format!(
                "fork sequence `at_seq` ({at_seq}) out of bounds for session `{session_id}` (1..={default_seq})"
            )))?;
        let prefix = &parent_events[..prefix_len];
        let anchor = prefix
            .last()
            .filter(|e| e.seq == at_seq)
            .ok_or_else(|| bad(format!("parent session `{session_id}` seq gap at {at_seq}")))?;
        let prefix_mode = session_mode(prefix);
        if prefix_mode != Mode::Research {
            return Err(bad(format!(
                "Selected prefix up to seq {at_seq} is {} rather than Research.",
                mode_label(prefix_mode)
            )));
        }

        // Build, validate, and serialize `solve.setup` BEFORE calling `fork_session` so no
        // orphan fork session is ever committed on validation failure.
        let origin = NodeRef {
            session: session_id.to_string(),
            mode: Mode::Research,
            id: format!("seq:{at_seq}"),
            paper: None,
            node_fp: Some(anchor.event_hash.clone()),
        };
        let setup = build_migrated_setup(
            session_id, prefix, parent_meta.as_ref(), &ui, &opts, &origin, &mut warnings,
        )?;
        let mut setup_payload = serde_json::to_value(&setup).map_err(|e| internal(e.to_string()))?;
        if let Some(obj) = setup_payload.as_object_mut() {
            obj.insert("ask_before_running".into(), json!(false));
        }

        let receipt = fork_session(
            &state.cfg.data_dir,
            &ForkOptions { parent_session: session_id.to_string(), at_seq, modified_assumption: None },
        ).map_err(bad)?;
        ledger
            .append_with_hash(&receipt.new_session, MARKER_SOLVE_STARTED, 1, &json!({}))
            .map_err(|e| internal(format!("failed to write solve.started: {e}")))?;
        ledger
            .append_with_hash(&receipt.new_session, event_kinds::SOLVE_SETUP, 1, &setup_payload)
            .map_err(|e| internal(format!("failed to write solve.setup: {e}")))?;
        let child_events = ledger.read_session(&receipt.new_session).map_err(|e| internal(e.to_string()))?;
        (receipt, origin, setup, child_events, prefix.to_vec())
    };

    let parent_title = session_title(parent_meta.as_ref(), &prefix_events, session_id);
    let child_meta = build_child_meta(
        &receipt.new_session, session_id, receipt.parent_seq, &parent_title,
        &setup.conventions, opts.conventions.is_some(), parent_meta.as_ref(),
    );
    state.save_meta(&child_meta).map_err(internal)?;

    let mut attached_project_id: Option<String> = None;
    if let Some(project) = parent_project {
        match attach_session(state, &project.id, &receipt.new_session) {
            Ok(_) => attached_project_id = Some(project.id),
            Err((_, err)) => warnings.push(format!("Could not link Solve session to project: {err}")),
        }
    }

    let solve_state = SolveState::from_events(&child_events);
    let tiers = compute_all_tiers(&solve_state);
    let legacy_receipts = collect_legacy_receipts(&child_events, &ledger);
    let view = session_solve_view(state, &receipt.new_session, &child_events, false);
    let warning = (!warnings.is_empty()).then(|| warnings.join("; "));

    Ok((201, json!({
        "ok": true,
        "session_id": receipt.new_session,
        "parent_session": session_id,
        "mode": api_mode_name(Mode::Solve),
        "mode_label": mode_label(Mode::Solve),
        "fork": receipt,
        "origin": origin,
        "setup": setup,
        "ask_before_running": false,
        "tiers": tiers,
        "legacy_receipts": legacy_receipts,
        "project_id": attached_project_id,
        "warning": warning,
        "warnings": warnings,
        "view": view,
    })))
}

fn parse_options(val: &Value) -> Result<OpenInSolveOptions, (u16, String)> {
    if let Some(map) = val.as_object() {
        for key in map.keys() {
            if !ALLOWED_OPTION_KEYS.contains(&key.as_str()) {
                return Err(bad(format!("unknown field `{key}` in open-in-solve request")));
            }
        }
    }
    let at_seq = match val.get("at_seq") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().filter(|&n| n >= 1).ok_or_else(|| bad("`at_seq` must be >= 1"))?),
    };
    let statement = match val.get("statement") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let trimmed = v.as_str().ok_or_else(|| bad("`statement` must be a string"))?.trim();
            if trimmed.is_empty() { return Err(bad("`statement` must not be empty")); }
            if trimmed.chars().count() > MAX_STATEMENT_CHARS {
                return Err(bad(format!("statement exceeds {MAX_STATEMENT_CHARS} characters")));
            }
            if has_disallowed_control(trimmed) {
                return Err(bad("statement contains disallowed control characters"));
            }
            Some(trimmed.to_string())
        }
    };
    let conventions = match val.get("conventions") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let trimmed = v.as_str().ok_or_else(|| bad("`conventions` must be a string"))?.trim();
            if trimmed.chars().count() > MAX_CONVENTIONS_CHARS {
                return Err(bad(format!("conventions exceeds {MAX_CONVENTIONS_CHARS} characters")));
            }
            if has_disallowed_control(trimmed) {
                return Err(bad("conventions contains disallowed control characters"));
            }
            if !trimmed.is_empty() { agent::validate_pinned_conventions(trimmed).map_err(bad)?; }
            Some(trimmed.to_string())
        }
    };
    let task_kind = val
        .get("task_kind")
        .filter(|v| !v.is_null())
        .map(|v| serde_json::from_value::<TaskKind>(v.clone()).map_err(|e| bad(format!("invalid `task_kind`: {e}"))))
        .transpose()?;
    Ok(OpenInSolveOptions {
        at_seq, statement, conventions, task_kind,
        solver: parse_role_override(val.get("solver"), "solver")?,
        verifier: parse_role_override(val.get("verifier"), "verifier")?,
    })
}

fn parse_role_override(val: Option<&Value>, label: &str) -> Result<Option<RoleModel>, (u16, String)> {
    let Some(v) = val.filter(|v| !v.is_null()) else { return Ok(None); };
    let obj = v.as_object().ok_or_else(|| bad(format!("`{label}` must be an object {{ provider, model }}")))?;
    let provider = obj.get("provider").and_then(Value::as_str)
        .ok_or_else(|| bad(format!("`{label}.provider` must be a string")))?.trim();
    let model = obj.get("model").and_then(Value::as_str)
        .ok_or_else(|| bad(format!("`{label}.model` must be a string")))?.trim();
    if provider.chars().count() > MAX_ROLE_CHARS || model.chars().count() > MAX_ROLE_CHARS {
        return Err(bad(format!("`{label}` provider/model exceeds {MAX_ROLE_CHARS} characters")));
    }
    if has_disallowed_control(provider) || has_disallowed_control(model) {
        return Err(bad(format!("`{label}` contains disallowed control characters")));
    }
    Ok(Some(RoleModel { provider: canonical_provider(provider), model: model.to_string() }))
}

fn build_migrated_setup(
    session_id: &str,
    prefix: &[Event],
    parent_meta: Option<&SessionMeta>,
    ui: &UiSettings,
    opts: &OpenInSolveOptions,
    origin: &NodeRef,
    warnings: &mut Vec<String>,
) -> Result<SolveSetup, (u16, String)> {
    let statement = extract_statement(session_id, prefix, parent_meta, opts.statement.as_deref(), warnings);
    let pinned_conventions = extract_conventions(prefix, parent_meta, ui, opts.conventions.as_deref(), warnings);
    let solver = opts.solver.clone().unwrap_or_else(|| extract_solver_role(prefix, parent_meta, ui));
    let verifier = opts.verifier.clone().unwrap_or_else(|| extract_verifier_role(prefix, ui, &solver));
    let prefill = prefill_solve_setup(&PrefillRequest {
        statement: statement.clone(),
        pinned_conventions,
        defaults: None,
        solver: non_empty_role(&solver),
        verifier: non_empty_role(&verifier),
        task_kind_override: opts.task_kind,
        allow_numeric_target: false,
    });
    let input = SolveSetupInput {
        statement,
        task_kind: prefill.task_kind,
        target: prefill.target,
        done: prefill.done,
        conventions: prefill.conventions,
        symbols: prefill.symbols,
        dimensions: prefill.dimensions,
        assumptions: extract_active_assumptions(prefix, warnings),
        given_source: None,
        sources: extract_ingested_sources(prefix, warnings),
        roles: SolveRoles { solver, verifier, compute: None },
        budgets: prefill.budgets,
        review: prefill.review,
        tool_profile: prefill.tool_profile,
        origin: Some(origin.clone()),
        ask_before_running: false,
    };
    validate_and_build_setup(&input, 0).map_err(bad)
}

fn extract_statement(
    session_id: &str,
    prefix: &[Event],
    parent_meta: Option<&SessionMeta>,
    override_stmt: Option<&str>,
    warnings: &mut Vec<String>,
) -> String {
    if let Some(s) = override_stmt { return s.to_string(); }
    let raw = prefix.iter()
        .filter(|e| matches!(e.kind.as_str(), "session.start" | "session.resume"))
        .find_map(|e| non_empty_str(e.payload.get("prompt")))
        .or_else(|| prefix.iter().filter(|e| e.kind == "investigation.handoff").find_map(|e| non_empty_str(e.payload.get("question"))))
        .or_else(|| parent_meta.map(|m| m.title.as_str()).map(str::trim).filter(|s| !s.is_empty()))
        .map(str::to_string)
        .unwrap_or_else(|| format!("{FALLBACK_STATEMENT_PREFIX}{session_id}"));
    let clean = strip_disallowed_control(&raw);
    let trimmed = clean.trim();
    if trimmed.is_empty() { return format!("{FALLBACK_STATEMENT_PREFIX}{session_id}"); }
    if trimmed.chars().count() > MAX_STATEMENT_CHARS {
        warnings.push(format!("Statement truncated to {MAX_STATEMENT_CHARS} characters."));
    }
    trimmed.chars().take(MAX_STATEMENT_CHARS).collect()
}

fn extract_conventions(
    prefix: &[Event],
    parent_meta: Option<&SessionMeta>,
    ui: &UiSettings,
    override_conv: Option<&str>,
    warnings: &mut Vec<String>,
) -> String {
    if let Some(c) = override_conv { return c.to_string(); }
    // Respect an explicitly cleared `meta.pinned_conventions == Some("")` without resurrecting old turns.
    let raw = if let Some(meta_conv) = parent_meta.and_then(|m| m.pinned_conventions.as_deref()) {
        meta_conv.trim().to_string()
    } else if let Some(turn_ev) = prefix.iter().rev().find(|e| matches!(e.kind.as_str(), "session.start" | "session.resume")) {
        extract_conventions_from_turn_event(turn_ev).unwrap_or_default()
    } else {
        ui.pinned_conventions.as_deref().map(str::trim).unwrap_or_default().to_string()
    };
    let clean = strip_disallowed_control(raw.trim());
    if clean.chars().count() > MAX_CONVENTIONS_CHARS {
        warnings.push(format!("Pinned conventions truncated to {MAX_CONVENTIONS_CHARS} characters."));
    }
    clean.chars().take(MAX_CONVENTIONS_CHARS).collect()
}

fn extract_conventions_from_turn_event(ev: &Event) -> Option<String> {
    const OPEN: &str = "<pinned_conventions>";
    const CLOSE: &str = "</pinned_conventions>";
    if let Some(v) = ev.payload.get("pinned_conventions") {
        return v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    }
    let sys = ev.payload.get("system_prompt").and_then(Value::as_str)?;
    let start = sys.find(OPEN)?;
    let rest = &sys[start + OPEN.len()..];
    let block = rest[..rest.find(CLOSE)?].trim();
    (!block.is_empty()).then(|| block.to_string())
}

fn extract_active_assumptions(prefix: &[Event], warnings: &mut Vec<String>) -> Vec<SetupAssumption> {
    let folded = SolveState::from_events(prefix);
    let mut seen_ids = BTreeSet::new();
    let mut out = Vec::new();
    let mut dropped = 0usize;
    for entry in folded.entries {
        if entry.superseded_by.is_some() || entry.entry_type != StepEntryType::Assumption {
            continue;
        }
        let summary = strip_disallowed_control(entry.summary.trim());
        let details = strip_disallowed_control(entry.details.trim());
        let combined = match (summary.is_empty(), details.is_empty()) {
            (true, true) => continue,
            (false, true) => summary,
            (true, false) => details,
            (false, false) if details == summary || details.starts_with(&summary) => details,
            (false, false) => format!("{summary}: {details}"),
        };
        if out.len() >= MAX_ASSUMPTIONS {
            dropped += 1;
            continue;
        }
        let text: String = combined.chars().take(MAX_ASSUMPTION_TEXT_CHARS).collect();
        let base_clean = strip_disallowed_control(entry.entry_id.trim());
        let base_id = if base_clean.trim().is_empty() {
            format!("legacy-{}", out.len() + 1)
        } else {
            base_clean.trim().chars().take(MAX_ASSUMPTION_ID_CHARS).collect()
        };
        out.push(SetupAssumption { id: deduplicate_id(&base_id, &mut seen_ids), text });
    }
    if dropped > 0 {
        warnings.push(format!(
            "Copied first {MAX_ASSUMPTIONS} active assumptions into setup ({dropped} omitted from setup list; all remain in ledger)."
        ));
    }
    out
}

fn deduplicate_id(base: &str, seen: &mut BTreeSet<String>) -> String {
    if seen.insert(base.to_string()) { return base.to_string(); }
    for suffix in 2usize.. {
        let tag = format!("-{suffix}");
        let prefix: String = base.chars().take(MAX_ASSUMPTION_ID_CHARS.saturating_sub(tag.len())).collect();
        let candidate = format!("{prefix}{tag}");
        if seen.insert(candidate.clone()) { return candidate; }
    }
    base.to_string()
}

fn extract_ingested_sources(prefix: &[Event], warnings: &mut Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    let mut dropped = 0usize;
    for ev in prefix.iter().filter(|e| e.kind == "source.ingested") {
        let Some(trimmed) = ev.payload.get("source_id").and_then(Value::as_str).map(str::trim) else {
            continue;
        };
        if trimmed.is_empty() || trimmed.len() > 128 || has_disallowed_control(trimmed) {
            dropped += 1;
            continue;
        }
        if seen.insert(trimmed.to_string()) {
            if out.len() >= MAX_SOURCES { dropped += 1; } else { out.push(trimmed.to_string()); }
        }
    }
    if dropped > 0 {
        warnings.push(format!(
            "Setup sources list omitted {dropped} source(s) due to setup limits (all copied source files remain in session)."
        ));
    }
    out
}

fn extract_solver_role(prefix: &[Event], parent_meta: Option<&SessionMeta>, ui: &UiSettings) -> RoleModel {
    let start_ev = prefix.iter().rev().find(|e| matches!(e.kind.as_str(), "session.start" | "session.resume"));
    let provider = start_ev
        .and_then(|e| non_empty_str(e.payload.get("provider")).or_else(|| non_empty_str(e.payload.get("route"))))
        .or_else(|| parent_meta.and_then(|m| non_empty_str_opt(m.default_provider.as_deref())))
        .or_else(|| non_empty_str_opt(ui.default_provider.as_deref()))
        .unwrap_or("");
    let model = start_ev
        .and_then(|e| non_empty_str(e.payload.get("model")).or_else(|| non_empty_str(e.payload.get("model_requested"))))
        .or_else(|| {
            prefix.iter().rev()
                .filter(|e| matches!(e.kind.as_str(), "cli.init" | "model.response" | "model.request"))
                .find_map(|e| non_empty_str(e.payload.get("model")))
        })
        .or_else(|| parent_meta.and_then(|m| non_empty_str_opt(m.default_model.as_deref())))
        .or_else(|| non_empty_str_opt(ui.default_model.as_deref()))
        .unwrap_or("");
    RoleModel { provider: canonical_provider(provider), model: clean_role_str(model) }
}

fn extract_verifier_role(prefix: &[Event], ui: &UiSettings, solver: &RoleModel) -> RoleModel {
    for key in ["full_verifier", "step_verifier", "verifier"] {
        if let Some(spec) = ui.agent_models.get(key) {
            return RoleModel { provider: canonical_provider(&spec.provider), model: clean_role_str(&spec.model) };
        }
    }
    if let Some(presets) = ui.role_presets.as_ref().and_then(Value::as_object) {
        for key in ["full_verifier", "verifier", "step_verifier"] {
            if let Some(obj) = presets.get(key).and_then(Value::as_object) {
                let p = obj.get("provider").and_then(Value::as_str).unwrap_or("");
                let m = obj.get("model").and_then(Value::as_str).unwrap_or("");
                if !p.trim().is_empty() || !m.trim().is_empty() {
                    return RoleModel { provider: canonical_provider(p), model: clean_role_str(m) };
                }
            }
        }
    }
    let ver_req = prefix.iter().rev().find(|e| e.kind == "verifier.request");
    let model = prefix.iter().rev()
        .filter(|e| e.kind == event_kinds::PROOF_VERIFICATION)
        .find_map(|e| non_empty_str(e.payload.get("verifier_model")).or_else(|| non_empty_str(e.payload.get("model"))))
        .or_else(|| ver_req.and_then(|e| non_empty_str(e.payload.get("verifier_model")).or_else(|| non_empty_str(e.payload.get("model")))))
        .unwrap_or("");
    let clean_model = clean_role_str(model);
    let provider = ver_req
        .and_then(|e| non_empty_str(e.payload.get("provider")))
        .map(canonical_provider)
        .unwrap_or_else(|| if !clean_model.is_empty() && clean_model == solver.model { solver.provider.clone() } else { String::new() });
    RoleModel { provider, model: clean_model }
}

fn build_child_meta(
    child_id: &str,
    parent_id: &str,
    parent_seq: u64,
    parent_title: &str,
    setup_conventions: &str,
    explicit_conventions_override: bool,
    parent_meta: Option<&SessionMeta>,
) -> SessionMeta {
    let title: String = format!("Solve: {parent_title}").chars().take(80).collect();
    let pinned_conventions = if !setup_conventions.trim().is_empty() {
        Some(setup_conventions.to_string())
    } else if explicit_conventions_override || parent_meta.and_then(|m| m.pinned_conventions.as_ref()).is_some() {
        Some(String::new())
    } else {
        None
    };
    SessionMeta {
        id: child_id.to_string(),
        title,
        allow_model_title: Some(false),
        workspace_root: parent_meta.and_then(|m| m.workspace_root.clone()),
        workspace_detached: parent_meta.is_some_and(|m| m.workspace_detached),
        pinned_conventions,
        verification_mode: false,
        calculation_mode: false,
        model_preset: parent_meta.and_then(|m| m.model_preset.clone()),
        default_provider: parent_meta.and_then(|m| m.default_provider.clone()),
        default_model: parent_meta.and_then(|m| m.default_model.clone()),
        reasoning_effort: parent_meta.and_then(|m| m.reasoning_effort.clone()),
        created_ms: now_ms(),
        fork_info: Some(ForkInfo {
            parent_session: parent_id.to_string(),
            forked_at_seq: parent_seq,
            modified_assumption: None,
        }),
        route: parent_meta.and_then(|m| m.route.clone()),
        archived: false,
        deleted: false,
    }
}

fn collect_legacy_receipts(child_events: &[Event], ledger: &Ledger) -> Vec<Value> {
    build_receipts(child_events, Some(ledger))
        .into_values()
        .filter_map(|r| {
            let (label, effective_ok, status) = match r.tool.as_str() {
                "lean_check" => {
                    let st = r.decoded_result.as_ref().and_then(|v| v.get("status")).and_then(Value::as_str).map(str::to_string);
                    let inner_ok = r.decoded_result.as_ref().and_then(|v| v.get("ok")).and_then(Value::as_bool);
                    let ok = r.ok && !r.result_missing && inner_ok.unwrap_or_else(|| st.as_deref().is_none_or(|s| s == "verified"));
                    (LEGACY_LEAN_CHECK_RECEIPT_LABEL, ok, st)
                }
                "sympy_eval" => (LEGACY_SYMPY_RECEIPT_LABEL, r.sympy_succeeded, None),
                _ => return None,
            };
            Some(json!({
                "call_id": r.call_id, "tool": r.tool, "request_seq": r.request_seq, "result_seq": r.result_seq,
                "ok": effective_ok, "status": status, "error": r.error, "result_missing": r.result_missing,
                "label": label, "grants_tier": false,
            }))
        })
        .collect()
}

fn canonical_provider(raw: &str) -> String {
    let clean = clean_role_str(raw);
    if clean.is_empty() { return clean; }
    ProviderKind::parse(&clean).map(|k| k.cli_name().to_string()).unwrap_or(clean)
}

fn clean_role_str(raw: &str) -> String {
    strip_disallowed_control(raw.trim()).chars().take(MAX_ROLE_CHARS).collect()
}

fn non_empty_role(role: &RoleModel) -> Option<RoleModel> {
    (!(role.provider.trim().is_empty() && role.model.trim().is_empty())).then(|| role.clone())
}

fn non_empty_str(val: Option<&Value>) -> Option<&str> {
    non_empty_str_opt(val.and_then(Value::as_str))
}

fn non_empty_str_opt(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim).filter(|s| !s.is_empty())
}

fn has_disallowed_control(s: &str) -> bool {
    s.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
}

fn strip_disallowed_control(s: &str) -> String {
    s.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t')).collect()
}
