//! Win32 process token and Job inspection helpers.

use super::{last_error, OwnedHandle};
use std::io;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{LocalFree, HANDLE, HLOCAL};
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenAppContainerSid, TokenIntegrityLevel, TokenIsAppContainer, PSID,
    TOKEN_APPCONTAINER_INFORMATION, TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
};
use windows_sys::Win32::System::JobObjects::IsProcessInJob;
use windows_sys::Win32::System::Threading::{OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenReport {
    pub is_appcontainer: bool,
    pub appcontainer_sid: Option<String>,
    pub integrity_sid: String,
    pub in_job: bool,
}

pub(crate) unsafe fn sid_to_string(psid: PSID) -> io::Result<String> {
    if psid.is_null() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "null PSID"));
    }
    let mut raw: *mut u16 = null_mut();
    if unsafe { ConvertSidToStringSidW(psid, &mut raw) } == 0 {
        return Err(last_error("ConvertSidToStringSidW"));
    }
    struct LocalGuard(*mut u16);
    impl Drop for LocalGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { LocalFree(self.0 as HLOCAL) };
            }
        }
    }
    let _guard = LocalGuard(raw);
    let mut len = 0usize;
    while unsafe { *raw.add(len) } != 0 {
        len += 1;
    }
    let slice = unsafe { std::slice::from_raw_parts(raw, len) };
    Ok(String::from_utf16_lossy(slice))
}

fn query_token_buf(token: HANDLE, class: i32) -> io::Result<Vec<u64>> {
    let mut needed: u32 = 0;
    unsafe { GetTokenInformation(token, class, null_mut(), 0, &mut needed) };
    if needed == 0 {
        return Err(last_error("GetTokenInformation(size)"));
    }
    let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
    if unsafe { GetTokenInformation(token, class, buf.as_mut_ptr().cast(), needed, &mut needed) } == 0 {
        return Err(last_error("GetTokenInformation"));
    }
    Ok(buf)
}

/// Inspect a live process's AppContainer status, SID, integrity level, and Job membership.
pub fn inspect_process_token(pid: u32) -> io::Result<TokenReport> {
    // SAFETY: `PROCESS_QUERY_LIMITED_INFORMATION` suffices for `IsProcessInJob` and `OpenProcessToken(..., TOKEN_QUERY)`.
    let hproc = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if hproc.is_null() {
        return Err(last_error("OpenProcess"));
    }
    let _proc_guard = OwnedHandle(hproc);

    let mut in_job: i32 = 0;
    if unsafe { IsProcessInJob(hproc, null_mut(), &mut in_job) } == 0 {
        return Err(last_error("IsProcessInJob"));
    }
    let mut token: HANDLE = null_mut();
    if unsafe { OpenProcessToken(hproc, TOKEN_QUERY, &mut token) } == 0 {
        return Err(last_error("OpenProcessToken"));
    }
    let _tok_guard = OwnedHandle(token);

    let mut is_ac: u32 = 0;
    let mut ret: u32 = 0;
    if unsafe { GetTokenInformation(token, TokenIsAppContainer, (&mut is_ac as *mut u32).cast(), 4, &mut ret) } == 0 {
        return Err(last_error("GetTokenInformation(TokenIsAppContainer)"));
    }
    let is_appcontainer = is_ac != 0;

    let appcontainer_sid = if is_appcontainer {
        let buf = query_token_buf(token, TokenAppContainerSid)?;
        // SAFETY: `buf` is 8-byte aligned and populated by `GetTokenInformation`.
        let info = unsafe { *(buf.as_ptr().cast::<TOKEN_APPCONTAINER_INFORMATION>()) };
        if info.TokenAppContainer.is_null() { None } else { Some(unsafe { sid_to_string(info.TokenAppContainer)? }) }
    } else {
        None
    };

    let il_buf = query_token_buf(token, TokenIntegrityLevel)?;
    // SAFETY: `il_buf` is 8-byte aligned and populated by `GetTokenInformation`.
    let label = unsafe { *(il_buf.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()) };
    let integrity_sid = unsafe { sid_to_string(label.Label.Sid)? };

    Ok(TokenReport { is_appcontainer, appcontainer_sid, integrity_sid, in_job: in_job != 0 })
}
