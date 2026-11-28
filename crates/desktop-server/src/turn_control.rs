//! Unified turn cancellation (`POST /api/sessions/{id}/stop`) across both in-core
//! API/ChatGPT turns (`CancelToken`) and subscription-CLI turns (`StopHandle`).

use crate::routes_session::{bad, ApiResult};
use crate::state::DesktopState;
use agent::CancelToken;
use research_tools::validate_session_id;
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// RAII registration of an in-core API/ChatGPT turn's [`CancelToken`] on [`DesktopState`],
/// registered before spawning the worker thread so a stop immediately after `202 Accepted`
/// reaches the turn.
pub(crate) struct ApiTurnGuard {
    state: Arc<DesktopState>,
    session_id: String,
    pub(crate) cancel: CancelToken,
}

impl ApiTurnGuard {
    pub(crate) fn new(state: &Arc<DesktopState>, session_id: &str) -> Result<Self, String> {
        let cancel = CancelToken::new();
        let mut map = state
            .api_turns
            .lock()
            .map_err(|_| "api_turns lock poisoned".to_string())?;
        map.insert(session_id.to_string(), cancel.clone());
        Ok(Self {
            state: Arc::clone(state),
            session_id: session_id.to_string(),
            cancel,
        })
    }
}

impl Drop for ApiTurnGuard {
    fn drop(&mut self) {
        if let Ok(mut map) = self.state.api_turns.lock() {
            map.remove(&self.session_id);
        }
    }
}

pub fn handle_stop_turn(state: &DesktopState, session_id: &str) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;

    let cli_handle = state
        .cli_turns
        .lock()
        .ok()
        .and_then(|m| m.get(session_id).map(|(_, h)| h.clone()));
    let api_cancel = state
        .api_turns
        .lock()
        .ok()
        .and_then(|m| m.get(session_id).cloned());

    if cli_handle.is_none() && api_cancel.is_none() {
        return Err((409, format!("no turn is running for `{session_id}`")));
    }

    if let Some(h) = cli_handle {
        h.stop();
    }
    if let Some(c) = api_cancel {
        c.cancel();
    }
    let denied = state.deny_pending(session_id, None, "supervisor: turn stopped by the user");
    state.hitl.cancel_session(session_id);
    Ok((
        200,
        json!({
            "ok": true,
            "session": session_id,
            "stopped": true,
            "approvals_denied": denied,
        }),
    ))
}

/// Stops all running API/ChatGPT and subscription-CLI turns on server shutdown.
pub fn stop_all_turns(state: &DesktopState) {
    let api_tokens: Vec<(String, CancelToken)> = state
        .api_turns
        .lock()
        .map(|m| m.iter().map(|(s, c)| (s.clone(), c.clone())).collect())
        .unwrap_or_default();
    for (sid, tok) in &api_tokens {
        tok.cancel();
        state.deny_pending(sid, None, "desktop server shutting down; denied");
        state.hitl.cancel_session(sid);
    }
    state.stop_all_cli_turns();
    if !api_tokens.is_empty() {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if state.api_turns.lock().map(|m| m.is_empty()).unwrap_or(true) {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthGuard;
    use crate::cli_turn::CliRouteKind;
    use crate::sse::SseHub;
    use crate::DesktopConfig;
    use std::path::Path;

    fn test_state(dir: &Path) -> Arc<DesktopState> {
        let cfg = DesktopConfig {
            port: 0,
            data_dir: dir.to_path_buf(),
            workspace_root: None,
            python: None,
            worker_script: None,
            api_base_override: None,
            literature_endpoints: research_tools::LiteratureEndpoints::default(),
            open_window: false,
            print_url_json: false,
            eval_timeout_s: 30,
            core_exe: None,
            claude_exe: None,
            agy_exe: None,
            connection_env: Some(std::collections::BTreeMap::new()),
            chatgpt_test: Some(crate::chatgpt::offline_test_hooks()),
        };
        let chatgpt = crate::chatgpt::ChatGptHub::new(&cfg).unwrap();
        Arc::new(DesktopState::new(cfg, AuthGuard::new(1).unwrap(), Arc::new(SseHub::new()), chatgpt))
    }

    #[test]
    fn stop_turn_cancels_api_turn_and_returns_409_once_dropped() {
        let tmp = tempfile::TempDir::new().unwrap();
        let st = test_state(tmp.path());
        let guard = ApiTurnGuard::new(&st, "s-apiturn1").unwrap();
        assert!(!guard.cancel.is_cancelled());

        let (code, body) = handle_stop_turn(&st, "s-apiturn1").unwrap();
        assert_eq!(code, 200);
        assert_eq!(body["stopped"], true);
        assert!(guard.cancel.is_cancelled());

        drop(guard);
        let err = handle_stop_turn(&st, "s-apiturn1").unwrap_err();
        assert_eq!(err.0, 409);
    }

    #[test]
    fn stop_cli_turns_of_only_stops_matching_route() {
        let tmp = tempfile::TempDir::new().unwrap();
        let st = test_state(tmp.path());
        let claude_stop = cli_route::StopHandle::new();
        let agy_stop = cli_route::StopHandle::new();
        {
            let mut m = st.cli_turns.lock().unwrap();
            m.insert("s-claude".into(), (CliRouteKind::Claude, claude_stop.clone()));
            m.insert("s-agy".into(), (CliRouteKind::Antigravity, agy_stop.clone()));
        }
        assert_eq!(st.stop_cli_turns_of(CliRouteKind::Antigravity), 1);
        assert!(agy_stop.is_stopped(), "antigravity turn must be stopped");
        assert!(!claude_stop.is_stopped(), "claude turn must stay running");
    }

    #[test]
    fn update_ui_settings_serializes_concurrent_updates() {
        let tmp = tempfile::TempDir::new().unwrap();
        let st = test_state(tmp.path());
        st.update_ui_settings(|ui| {
            ui.default_provider = Some("openai".into());
        })
        .unwrap();
        let s1 = Arc::clone(&st);
        let s2 = Arc::clone(&st);
        let t1 = std::thread::spawn(move || {
            for i in 0..20 {
                s1.update_ui_settings(|ui| ui.default_model = Some(format!("m-{i}"))).unwrap();
            }
        });
        let t2 = std::thread::spawn(move || {
            for _ in 0..20 {
                s2.update_ui_settings(|ui| ui.pinned_conventions = Some("conv".into())).unwrap();
            }
        });
        t1.join().unwrap();
        t2.join().unwrap();
        let ui = st.load_ui_settings();
        assert_eq!(ui.default_provider.as_deref(), Some("openai"), "default_provider must not be lost");
        assert_eq!(ui.pinned_conventions.as_deref(), Some("conv"));
        assert!(ui.default_model.is_some());
    }
}
