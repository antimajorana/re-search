//! Native Windows folder picker (`POST /api/workspace/pick`) and in-place
//! session workspace attachment (`PUT /api/sessions/{id}/workspace`).

use crate::routes_session::{bad, conflict, handle_get_session, internal, not_found, parse_json_body, ApiResult};
use crate::state::DesktopState;
use research_tools::{strip_verbatim_prefix, validate_session_id, ResearchEnv};
#[cfg(not(test))]
use serde_json::json;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

/// Opens the OS folder picker dialog on Windows (disabled in test and headless
/// `--print-url-json` modes so automated tests can never block on a modal window).
pub fn handle_pick_workspace(state: &DesktopState) -> ApiResult {
    if cfg!(test) || state.cfg.chatgpt_test.is_some() || state.cfg.print_url_json {
        return Err(bad("native folder picker is disabled in test mode"));
    }
    pick_folder_impl()
}

#[cfg(test)]
fn pick_folder_impl() -> ApiResult {
    Err(bad("native folder picker is disabled in test mode"))
}

#[cfg(all(not(windows), not(test)))]
fn pick_folder_impl() -> ApiResult {
    Ok((200, json!({ "supported": false, "cancelled": false })))
}

#[cfg(all(windows, not(test)))]
fn pick_folder_impl() -> ApiResult {
    // Run COM `IFileOpenDialog` on a dedicated single-threaded apartment (STA) thread.
    let joined = std::thread::Builder::new()
        .name("workspace-folder-picker".into())
        .spawn(win_pick_folder_sta)
        .map_err(|e| internal(format!("failed to spawn folder picker thread: {e}")))?
        .join()
        .map_err(|_| internal("folder picker thread panicked"))?;

    match joined {
        Ok(Some(path)) => Ok((200, json!({ "supported": true, "cancelled": false, "path": path }))),
        Ok(None) => Ok((200, json!({ "supported": true, "cancelled": true }))),
        Err(err) => Err(internal(err)),
    }
}

#[cfg(all(windows, not(test)))]
fn win_pick_folder_sta() -> Result<Option<String>, String> {
    use windows::core::{HSTRING, PWSTR};
    use windows::Win32::Foundation::ERROR_CANCELLED;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
    };
    use windows::Win32::UI::Shell::{
        FileOpenDialog, IFileOpenDialog, FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR, FOS_PATHMUSTEXIST,
        FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
    };

    struct ComGuard(bool);
    impl Drop for ComGuard {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: Paired with a successful `CoInitializeEx` on this exact STA thread.
                unsafe { CoUninitialize() };
            }
        }
    }

    // SAFETY: Fresh thread owns its COM apartment; all COM pointers stay on this thread and are
    // dropped before `ComGuard` calls `CoUninitialize`, and `PWSTR` from `GetDisplayName` is freed
    // via `CoTaskMemFree`.
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        let _com = ComGuard(hr.is_ok());

        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).map_err(|e| format!("CoCreateInstance failed: {e}"))?;
        let opts = dialog.GetOptions().map_err(|e| format!("GetOptions failed: {e}"))?;
        dialog
            .SetOptions(opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_NOCHANGEDIR | FOS_PATHMUSTEXIST)
            .map_err(|e| format!("SetOptions failed: {e}"))?;
        let _ = dialog.SetTitle(&HSTRING::from("Attach Workspace Folder - reSearch"));

        if let Err(err) = dialog.Show(None) {
            if err.code() == ERROR_CANCELLED.to_hresult() {
                return Ok(None);
            }
            return Err(format!("folder picker failed: {err}"));
        }

        let item = dialog.GetResult().map_err(|e| format!("GetResult failed: {e}"))?;
        let raw_pwstr: PWSTR = item
            .GetDisplayName(SIGDN_FILESYSPATH)
            .map_err(|e| format!("GetDisplayName failed: {e}"))?;
        if raw_pwstr.is_null() {
            return Ok(None);
        }
        let chosen = raw_pwstr
            .to_string()
            .map_err(|e| format!("folder path is not valid UTF-16: {e}"));
        CoTaskMemFree(Some(raw_pwstr.0 as *const core::ffi::c_void));
        chosen.map(Some)
    }
}

/// Attaches or updates `workspace_root` in-place on an existing session (`PUT /api/sessions/{id}/workspace`)
/// without discarding its conversation or Proof DAG.
pub fn handle_put_session_workspace(
    state: &Arc<DesktopState>,
    session_id: &str,
    body: &[u8],
) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }
    if state
        .active_turns
        .lock()
        .map(|a| a.contains(session_id))
        .unwrap_or(false)
    {
        return Err(conflict("cannot change workspace while a turn is running in this session"));
    }

    let val = parse_json_body(body)?;
    let next_ws = match val.get("workspace_root") {
        Some(Value::Null) => None,
        Some(Value::String(raw)) if raw.trim().is_empty() => None,
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            let p = PathBuf::from(trimmed);
            if !p.exists() || !p.is_dir() {
                return Err(bad(format!(
                    "workspace_root `{trimmed}` does not exist or is not a directory"
                )));
            }
            let env = ResearchEnv::with_workspace(state.cfg.data_dir.clone(), &p)
                .map_err(|e| bad(e.to_string()))?;
            let canon = strip_verbatim_prefix(&env.workspace_root.unwrap_or(p))
                .display()
                .to_string();
            Some(canon)
        }
        _ => return Err(bad("`workspace_root` string (or null) is required")),
    };

    let mut meta = state.load_meta(session_id).unwrap_or_else(|| crate::state::SessionMeta {
        id: session_id.to_string(),
        title: session_id.to_string(),
        created_ms: crate::state::now_ms(),
        ..Default::default()
    });
    meta.workspace_detached = next_ws.is_none();
    meta.workspace_root = next_ws;
    state.save_meta(&meta).map_err(internal)?;
    handle_get_session(state, session_id)
}
