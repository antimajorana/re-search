//! Solve mode HTTP routes: deterministic prefill, setup recording, and view projection (package S4).
//!
//! Sub-dispatcher registered at `/api/sessions/{id}/solve/..` behind `AuthGuard::authorize_api`.
//! Seams reserved for downstream Solve briefs (keep this module `< 350` lines):
//! - S8: `POST /api/sessions/{id}/solve/attach-source`, `POST /api/sessions/{id}/solve/ack`
//! - S9: `POST /api/sessions/{id}/solve/check`, `POST /api/sessions/{id}/solve/recheck`
//! - S17: `POST /api/sessions/{id}/solve/transcribe`
//!
//! S12 puts `POST .../solve/continue` in `solve_turn_routes.rs`; S18 puts artifact GET in
//! `solve_artifacts_route.rs`.

use crate::connections::{credential, API_PROVIDERS};
use crate::routes_session::{bad, internal, not_found, parse_json_body, ApiResult};
use crate::session_lifecycle::is_turn_running;
use crate::session_mode::session_mode;
pub use crate::solve_view::{
    build_solve_approval_grants, probe_toolbox, LINUX_TOOLBOX_LINE, WORKER_RESTART_NOTE,
};
use crate::solve_view::build_solve_view;
use crate::state::DesktopState;
use agent::ProviderKeyStore;
use ledger::Event;
use research_tools::lessons::LessonsStore;
use research_tools::lessons_select::suggest_solve_defaults;
use research_tools::paper_key::Mode;
use research_tools::solve_model::{
    compute_entry_sha256, event_kinds, RoleModel, SolveSetup, StepEntryType, StepOrigin,
};
use research_tools::solve_setup::{
    prefill_solve_setup, scan_statement_symbols, validate_and_build_setup, PrefillRequest,
    SetupDefaults, SolveSetupInput,
};
use research_tools::solve_state::SolveState;
use research_tools::validate_session_id;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Single sub-dispatcher for `/api/sessions/{id}/solve/..`.
pub fn handle_solve(
    state: &Arc<DesktopState>,
    method: &str,
    session_id: &str,
    rest: &[&str],
    body: &[u8],
) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let ledger = state.open_ledger().map_err(internal)?;
    let events = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
    if session_mode(&events) != Mode::Solve {
        return Err(bad("This endpoint is only available for Solve sessions."));
    }

    match (method, rest) {
        ("POST", ["prefill"]) => handle_prefill(state, session_id, body),
        ("POST", ["setup"]) => handle_setup(state, session_id, body),
        ("GET", ["view"]) => handle_view(state, session_id, &events),
        // Downstream seams: S8 (attach-source, ack), S9 (check, recheck), S17 (transcribe).
        _ => Err(not_found(format!("unknown solve endpoint: {method} /{}", rest.join("/")))),
    }
}

/// Build the Solve view payload for `GET /api/sessions/{id}` and `GET /api/sessions/{id}/solve/view`.
pub fn session_solve_view(
    state: &DesktopState,
    session_id: &str,
    events: &[Event],
    running: bool,
) -> Value {
    let toolbox = probe_toolbox(state);
    let approval_grants = build_solve_approval_grants(state, session_id);
    let model_connected = has_connected_model(state);
    build_solve_view(events, running, toolbox, approval_grants, model_connected)
}

fn handle_view(state: &DesktopState, session_id: &str, events: &[Event]) -> ApiResult {
    let running = is_turn_running(state, session_id);
    Ok((200, session_solve_view(state, session_id, events, running)))
}

fn handle_prefill(state: &DesktopState, session_id: &str, body: &[u8]) -> ApiResult {
    use research_tools::solve_model::{MAX_CONVENTIONS_CHARS, MAX_STATEMENT_CHARS};
    let val = parse_json_body(body)?;
    let statement = val.get("statement").and_then(Value::as_str).unwrap_or("").to_string();
    if statement.chars().count() > MAX_STATEMENT_CHARS {
        return Err(bad(format!("statement exceeds {MAX_STATEMENT_CHARS} characters")));
    }
    let meta = state.load_meta(session_id);
    let ui = state.load_ui_settings();

    let pinned_conventions = val
        .get("conventions")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| meta.as_ref().and_then(|m| m.pinned_conventions.clone()))
        .or(ui.pinned_conventions.clone())
        .unwrap_or_default();
    if pinned_conventions.chars().count() > MAX_CONVENTIONS_CHARS {
        return Err(bad(format!("conventions exceeds {MAX_CONVENTIONS_CHARS} characters")));
    }

    let solver = parse_optional_role(val.get("solver")).or_else(|| {
        let p = meta.as_ref().and_then(|m| m.default_provider.clone()).or(ui.default_provider.clone());
        let m = meta.as_ref().and_then(|m| m.default_model.clone()).or(ui.default_model.clone());
        p.zip(m).map(|(provider, model)| RoleModel { provider, model })
    });
    let verifier = parse_optional_role(val.get("verifier")).or_else(|| {
        ui.agent_models.get("verifier").map(|s| RoleModel {
            provider: s.provider.clone(),
            model: s.model.clone(),
        })
    });
    let task_kind_override = val
        .get("task_kind")
        .filter(|v| !v.is_null())
        .map(|v| serde_json::from_value(v.clone()).map_err(|e| bad(format!("invalid task_kind: {e}"))))
        .transpose()?;
    let allow_numeric_target =
        val.get("allow_numeric_target").and_then(Value::as_bool).unwrap_or(false);

    // Optional F16a user-corrected defaults lookup (strictly local, zero model/network calls).
    let defaults = if val.get("use_lessons_defaults").and_then(Value::as_bool).unwrap_or(true) {
        load_f16a_defaults(state, &statement)
    } else {
        None
    };

    let prefill = prefill_solve_setup(&PrefillRequest {
        statement,
        pinned_conventions,
        defaults,
        solver: solver.clone(),
        verifier: verifier.clone(),
        task_kind_override,
        allow_numeric_target,
    });

    Ok((
        200,
        json!({
            "prefill": prefill,
            "roles": { "solver": solver, "verifier": verifier },
            "toolbox": probe_toolbox(state),
            "approval_grants": build_solve_approval_grants(state, session_id),
            "model_connected": has_connected_model(state),
        }),
    ))
}

fn handle_setup(state: &DesktopState, session_id: &str, body: &[u8]) -> ApiResult {
    let input: SolveSetupInput =
        serde_json::from_slice(body).map_err(|e| bad(format!("invalid solve setup JSON: {e}")))?;
    if !input.conventions.trim().is_empty() {
        agent::validate_pinned_conventions(input.conventions.trim()).map_err(bad)?;
    }

    let _guard = state.ledger_write_lock.lock().map_err(|_| internal("ledger write lock poisoned"))?;
    let ledger = state.open_ledger().map_err(internal)?;
    let existing = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
    if session_mode(&existing) != Mode::Solve {
        return Err(bad("This endpoint is only available for Solve sessions."));
    }

    let folded = SolveState::from_events(&existing);
    let prev_rev = folded.setup.as_ref().map(|s| s.revision).unwrap_or(0);
    let setup = validate_and_build_setup(&input, prev_rev).map_err(bad)?;

    let mut setup_payload = serde_json::to_value(&setup).map_err(|e| internal(e.to_string()))?;
    if let Some(obj) = setup_payload.as_object_mut() {
        obj.insert("ask_before_running".into(), json!(input.ask_before_running));
    }
    ledger
        .append_with_hash(session_id, event_kinds::SOLVE_SETUP, 1, &setup_payload)
        .map_err(|e| internal(format!("failed to write solve.setup: {e}")))?;

    let given_entry_ids = sync_setup_given_steps(&ledger, session_id, &folded, &setup)?;
    let updated_events = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
    drop(_guard);

    state.sse.emit(session_id, "solve_updated", json!({ "session": session_id, "revision": setup.revision }));

    let view = session_solve_view(state, session_id, &updated_events, is_turn_running(state, session_id));
    Ok((
        200,
        json!({
            "ok": true,
            "setup": setup,
            "ask_before_running": input.ask_before_running,
            "given_entry_ids": given_entry_ids,
            "view": view,
        }),
    ))
}

fn sync_setup_given_steps(
    ledger: &ledger::Ledger,
    session_id: &str,
    folded: &SolveState,
    setup: &SolveSetup,
) -> Result<Vec<String>, (u16, String)> {
    let desired = build_desired_given_steps(setup);
    let existing_setup: BTreeMap<(String, String), String> = folded
        .active_entries()
        .into_iter()
        .filter(|e| e.origin == Some(StepOrigin::Setup))
        .map(|e| ((e.summary.clone(), e.details.clone()), e.entry_id.clone()))
        .collect();

    let mut next_num = folded
        .entries
        .iter()
        .filter_map(|e| e.entry_id.strip_prefix("e-").and_then(|n| n.parse::<u64>().ok()))
        .max()
        .unwrap_or(0);

    let mut kept_ids = Vec::new();
    let mut new_entries = Vec::new();
    let mut replacement_by_summary: BTreeMap<String, String> = BTreeMap::new();
    for (summary, details) in &desired {
        if let Some(eid) = existing_setup.get(&(summary.clone(), details.clone())) {
            kept_ids.push(eid.clone());
            replacement_by_summary.insert(summary.clone(), eid.clone());
        } else {
            next_num += 1;
            let eid = format!("e-{next_num}");
            kept_ids.push(eid.clone());
            replacement_by_summary.insert(summary.clone(), eid.clone());
            new_entries.push((eid, summary.clone(), details.clone()));
        }
    }

    // Write new `Given` entries before superseding replaced ones so `replaced_by` always points at an existing `e-N`.
    for (eid, summary, details) in new_entries {
        let entry_sha256 = compute_entry_sha256(None, &summary, &details);
        let payload = json!({
            "entry_id": eid,
            "entry_type": StepEntryType::Assumption.as_str(),
            "summary": summary,
            "details": details,
            "depends_on": [],
            "origin": StepOrigin::Setup,
            "entry_sha256": entry_sha256,
        });
        ledger
            .append_with_hash(session_id, event_kinds::PROOF_ENTRY, 1, &payload)
            .map_err(|e| internal(format!("failed to write Given proof.entry: {e}")))?;
    }

    let fallback_anchor = kept_ids.first().cloned().unwrap_or_default();
    for ((summary, details), old_id) in &existing_setup {
        if !desired.iter().any(|(s, d)| s == summary && d == details) {
            let replaced_by = replacement_by_summary
                .get(summary)
                .cloned()
                .unwrap_or_else(|| fallback_anchor.clone());
            let payload = json!({
                "entry_id": old_id,
                "replaced_by": replaced_by,
                "reason": format!("Setup updated to revision {}", setup.revision),
            });
            ledger
                .append_with_hash(session_id, event_kinds::PROOF_SUPERSEDE, 1, &payload)
                .map_err(|e| internal(format!("failed to supersede old setup step: {e}")))?;
        }
    }

    Ok(kept_ids)
}

fn build_desired_given_steps(setup: &SolveSetup) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if !setup.conventions.trim().is_empty() {
        out.push(("Setup conventions".to_string(), setup.conventions.clone()));
    }
    if !setup.symbols.is_empty() {
        let lines: Vec<String> = setup
            .symbols
            .iter()
            .map(|s| {
                if s.assumptions.is_empty() {
                    s.name.clone()
                } else {
                    format!("{}: {}", s.name, s.assumptions.join(", "))
                }
            })
            .collect();
        out.push(("Symbol assumptions".to_string(), lines.join("; ")));
    }
    for assumption in &setup.assumptions {
        out.push((format!("Assumption ({})", assumption.id), assumption.text.clone()));
    }
    out
}

fn load_f16a_defaults(state: &DesktopState, statement: &str) -> Option<SetupDefaults> {
    let scanned = scan_statement_symbols(statement, None);
    let sym_refs: Vec<&str> = scanned.iter().map(|s| s.name.as_str()).collect();
    let store = LessonsStore::open(&state.cfg.data_dir).ok()?;
    let suggested = suggest_solve_defaults(&store, &sym_refs, None).ok()?;
    if suggested.symbols.is_empty() && suggested.conventions.is_empty() {
        return None;
    }
    Some(SetupDefaults::from_suggested_lessons(&suggested))
}

fn parse_optional_role(val: Option<&Value>) -> Option<RoleModel> {
    let obj = val?.as_object()?;
    let provider = obj.get("provider")?.as_str()?.trim().to_string();
    let model = obj.get("model")?.as_str()?.trim().to_string();
    if provider.is_empty() && model.is_empty() { None } else { Some(RoleModel { provider, model }) }
}

fn has_connected_model(state: &DesktopState) -> bool {
    if state.chatgpt.ready() {
        return true;
    }
    let store = ProviderKeyStore::load_from_data_dir(&state.cfg.data_dir).unwrap_or_default();
    if API_PROVIDERS.iter().any(|p| credential(state, *p, &store).ok().is_some_and(|c| c.key.is_some())) {
        return true;
    }
    let ui = state.load_ui_settings();
    ui.antigravity_ack.is_some()
        || (ui.default_provider.as_deref().is_some_and(|p| !p.trim().is_empty())
            && ui.default_model.as_deref().is_some_and(|m| !m.trim().is_empty()))
}
