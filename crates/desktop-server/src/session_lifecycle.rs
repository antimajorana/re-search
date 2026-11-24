//! Session lifecycle handlers (archive, unarchive, delete) and per-session
//! human-approved capability grants (`execute-code` when contained, `network-read`,
//! `write-workspace`) shared across both in-core API/ChatGPT turns and CLI-relay turns.

use crate::routes_session::{bad, internal, not_found, parse_json_body, ApiResult};
use crate::state::{now_ms, DesktopState, SessionMeta};
use policy::EffectClass;
use research_tools::validate_session_id;
use serde_json::{json, Value};
use std::fs;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct SessionCapabilityGrant {
    pub effect: EffectClass,
    pub grant_digest: String,
    pub contained: bool,
}

/// Only `network-read`, `write-workspace`, and actively contained `execute-code`
/// (`AppContainer`) may be opted into a session grant by a human click.
pub fn can_grant_for_session(effect: EffectClass, contained: bool) -> bool {
    match effect {
        EffectClass::NetworkRead | EffectClass::WriteWorkspace => true,
        EffectClass::ExecuteCode => contained,
        _ => false,
    }
}

pub fn matching_session_grant(
    state: &DesktopState,
    session_id: &str,
    effect: EffectClass,
    contained: bool,
) -> Option<SessionCapabilityGrant> {
    if !can_grant_for_session(effect, contained) {
        return None;
    }
    let map = state.session_capability_grants.lock().ok()?;
    let grant = map.get(&(session_id.to_string(), effect))?.clone();
    if effect == EffectClass::ExecuteCode && (!grant.contained || !contained) {
        return None;
    }
    Some(grant)
}

pub fn bind_session_grant(
    state: &DesktopState,
    session_id: &str,
    effect: EffectClass,
    contained: bool,
    grant_digest: &str,
) {
    if !can_grant_for_session(effect, contained) {
        return;
    }
    if let Ok(mut map) = state.session_capability_grants.lock() {
        map.insert(
            (session_id.to_string(), effect),
            SessionCapabilityGrant {
                effect,
                grant_digest: grant_digest.to_string(),
                contained,
            },
        );
    }
}

pub fn list_session_grants(state: &DesktopState, session_id: &str) -> Vec<&'static str> {
    let Ok(map) = state.session_capability_grants.lock() else {
        return Vec::new();
    };
    let mut out: Vec<&'static str> = map
        .keys()
        .filter(|(sid, _)| sid == session_id)
        .map(|(_, eff)| eff.as_str())
        .collect();
    out.sort_unstable();
    out
}

pub fn clear_session_grants(state: &DesktopState, session_id: &str) {
    if let Ok(mut map) = state.session_capability_grants.lock() {
        map.retain(|(sid, _), _| sid != session_id);
    }
}

pub fn is_turn_running(state: &DesktopState, session_id: &str) -> bool {
    state
        .active_turns
        .lock()
        .map(|s| s.contains(session_id))
        .unwrap_or(false)
        || state
            .api_turns
            .lock()
            .map(|m| m.contains_key(session_id))
            .unwrap_or(false)
        || state
            .cli_turns
            .lock()
            .map(|m| m.contains_key(session_id))
            .unwrap_or(false)
}

pub(crate) fn reserve_turn(state: &DesktopState, session: &str) -> Result<fs::File, (u16, String)> {
    research_tools::session_activity::start_turn(&state.cfg.data_dir, session)
        .map_err(|e| if e.kind() == std::io::ErrorKind::WouldBlock { (409, e.to_string()) } else { internal(e.to_string()) })
}

fn stop_and_wait_briefly(state: &DesktopState, session_id: &str) {
    let _ = crate::turn_control::handle_stop_turn(state, session_id);
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        if !is_turn_running(state, session_id) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn handle_archive_session(state: &DesktopState, session_id: &str, body: &[u8]) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    let val = parse_json_body(body)?;
    let archived = val
        .get("archived")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    if archived {
        stop_and_wait_briefly(state, session_id);
        clear_session_grants(state, session_id);
    }

    let mut meta = state.load_meta(session_id).unwrap_or_else(|| SessionMeta {
        id: session_id.to_string(),
        title: session_id.to_string(),
        created_ms: now_ms(),
        ..SessionMeta::default()
    });
    meta.archived = archived;
    state.save_meta(&meta).map_err(internal)?;

    Ok((
        200,
        json!({
            "ok": true,
            "id": session_id,
            "archived": archived,
        }),
    ))
}

pub fn handle_delete_session(state: &DesktopState, session_id: &str) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }

    stop_and_wait_briefly(state, session_id);
    clear_session_grants(state, session_id);
    crate::cli_turn::sweep_session_tool_config(state, session_id);

    let ledger = state.open_ledger().map_err(internal)?;
    let has_ledger_events = !ledger
        .read_session(session_id)
        .map_err(|e| internal(e.to_string()))?
        .is_empty();

    let _draft_guard = crate::manuscript_drafts::lock_writes(state)?;
    let sdir = state.session_dir(session_id).map_err(internal)?;
    if sdir.exists() {
        for sub in ["scratch", "pycache", "sources"] {
            let p = sdir.join(sub);
            if p.exists() {
                let _ = fs::remove_dir_all(&p);
            }
        }
        for name in ["sidetalk.jsonl", "manuscript-drafts.json"] {
            let _ = fs::remove_file(sdir.join(name));
        }
    }

    // Preserve append-only SQLite hash-chain integrity: never mutate `ledger.db` or `blobs/`.
    // Empty sessions with zero ledger rows remove their session folder completely; sessions
    // with recorded ledger rows store a minimal tombstone `meta.json` so they vanish from UI/API
    // while `ledger.verify_all()` remains 100% valid.
    if !has_ledger_events {
        if sdir.exists() {
            let _ = fs::remove_dir_all(&sdir);
        }
    } else {
        let tombstone = SessionMeta {
            id: session_id.to_string(),
            title: String::new(),
            created_ms: now_ms(),
            deleted: true,
            ..SessionMeta::default()
        };
        state.save_meta(&tombstone).map_err(internal)?;
    }
    crate::session_folders::forget_session(state, session_id)?;

    Ok((
        200,
        json!({
            "ok": true,
            "id": session_id,
            "deleted": true,
        }),
    ))
}

pub fn handle_revoke_session_grants(state: &DesktopState, session_id: &str) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    clear_session_grants(state, session_id);
    Ok((
        200,
        json!({
            "ok": true,
            "id": session_id,
            "session_grants": [],
        }),
    ))
}
