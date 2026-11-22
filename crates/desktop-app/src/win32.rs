//! The few Win32 calls the app makes itself, through the `windows` crate version that
//! `wry`/`tao` already depend on.

use windows::core::{w, HSTRING};
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
#[cfg(not(test))]
use windows::Win32::UI::Shell::ShellExecuteW;
#[cfg(not(test))]
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND};

/// Groups the window and taskbar button under one identity. Must run before any window exists.
pub fn set_app_user_model_id() {
    // SAFETY: a static, NUL-terminated wide string; the call has no other preconditions.
    let _ = unsafe { SetCurrentProcessExplicitAppUserModelID(w!("ReSearch.Desktop")) };
}

/// Holds the single-instance mutex for the life of the process.
pub struct SingleInstance(#[allow(dead_code)] HANDLE);

/// `Some` when this is the first instance in this logon session, `None` when another
/// `re-search-app` already holds `Local\ReSearch.Desktop.SingleInstance`.
pub fn acquire_single_instance() -> Option<SingleInstance> {
    // SAFETY: default security, not initially owned, static name. The handle is kept (never
    // closed) so the mutex lives exactly as long as this process.
    let handle = unsafe { CreateMutexW(None, false, w!("Local\\ReSearch.Desktop.SingleInstance")) };
    let already = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    match handle {
        // Another instance owns it: do not start twice.
        Ok(_) if already => None,
        Ok(h) => Some(SingleInstance(h)),
        // The mutex could not be created at all (for example the name is taken by another
        // kind of object): start anyway rather than never starting, without the guard.
        Err(_) => Some(SingleInstance(HANDLE::default())),
    }
}

/// A modal error box, for failures a console-less app cannot print.
pub fn error_box(text: &str) {
    let text = HSTRING::from(text);
    // SAFETY: valid wide strings that outlive the call; no owner window.
    unsafe {
        MessageBoxW(None, &text, w!("reSearch"), MB_OK | MB_ICONERROR | MB_SETFOREGROUND);
    }
}

/// Opens a vetted external academic source URL in the user's OS handler when clicked
/// from inside the WebView2 UI (no-op under `cfg(test)`).
pub fn open_external_source_url(url: &str) {
    if crate::rules::is_allowed_external_source_url(url) {
        #[cfg(not(test))]
        {
            let target = HSTRING::from(url);
            // SAFETY: valid NUL-terminated wide strings; URL is verified HTTPS allowlisted.
            unsafe {
                let _ = ShellExecuteW(None, w!("open"), &target, None, None, SW_SHOWNORMAL);
            }
        }
    }
}
