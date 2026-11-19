//! Scoped `STARTUPINFOEXW` process launcher (`HANDLE_LIST`, `NUL` stdio,
//! optional `SECURITY_CAPABILITIES` AppContainer token, clean env, Job attach).

use super::{create_kill_on_close_job, encode_wide_z, last_error, restrict_job_ui, Job};
use crate::appcontainer::AppContainerProfile;
use crate::sandbox::{Contained, WorkerLaunch};
use crate::stage::strip_verbatim;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_INSUFFICIENT_BUFFER, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, WAIT_FAILED,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::{SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, TerminateJobObject};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess, InitializeProcThreadAttributeList,
    ResumeThread, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject, CREATE_NO_WINDOW,
    CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
    STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW,
};

struct NulStdHandles {
    stdin: HANDLE,
    stdout: HANDLE,
}

impl NulStdHandles {
    fn open() -> io::Result<Self> {
        let path = encode_wide_z(OsStr::new(r"\\.\NUL"))?;
        let sa = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: null_mut(),
            bInheritHandle: 1,
        };
        let mut out = NulStdHandles { stdin: INVALID_HANDLE_VALUE, stdout: INVALID_HANDLE_VALUE };
        // SAFETY: `path` is NUL-terminated UTF-16 and `sa` lives for both `CreateFileW` calls.
        out.stdin = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                &sa,
                OPEN_EXISTING,
                0,
                null_mut(),
            )
        };
        if out.stdin == INVALID_HANDLE_VALUE {
            return Err(last_error("CreateFileW(\\\\.\\NUL, read)"));
        }
        // SAFETY: same as above, opened for write.
        out.stdout = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                &sa,
                OPEN_EXISTING,
                0,
                null_mut(),
            )
        };
        if out.stdout == INVALID_HANDLE_VALUE {
            return Err(last_error("CreateFileW(\\\\.\\NUL, write)"));
        }
        Ok(out)
    }
}

impl Drop for NulStdHandles {
    fn drop(&mut self) {
        for h in [self.stdin, self.stdout] {
            if !h.is_null() && h != INVALID_HANDLE_VALUE {
                // SAFETY: valid handle opened by `NulStdHandles::open`, closed once.
                unsafe { CloseHandle(h) };
            }
        }
    }
}

struct AttrList {
    storage: Vec<u64>,
    initialized: bool,
}

impl AttrList {
    fn new(count: u32) -> io::Result<Self> {
        let mut size: usize = 0;
        // SAFETY: query required byte count with null buffer.
        let ok = unsafe { InitializeProcThreadAttributeList(null_mut(), count, 0, &mut size) };
        if ok == 0 {
            let err = io::Error::last_os_error();
            if err.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32) {
                return Err(io::Error::new(err.kind(), format!("InitializeProcThreadAttributeList(size): {err}")));
            }
        }
        if size == 0 {
            return Err(io::Error::other("InitializeProcThreadAttributeList returned zero size"));
        }
        let mut storage = vec![0u64; size.div_ceil(8)];
        // SAFETY: `storage` is 8-byte aligned and has at least `size` bytes.
        if unsafe { InitializeProcThreadAttributeList(storage.as_mut_ptr().cast(), count, 0, &mut size) } == 0 {
            return Err(last_error("InitializeProcThreadAttributeList"));
        }
        Ok(AttrList { storage, initialized: true })
    }

    fn as_mut_ptr(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.storage.as_mut_ptr().cast()
    }

    fn update(&mut self, attribute: u32, value: *const core::ffi::c_void, size: usize) -> io::Result<()> {
        // SAFETY: caller guarantees `value` points to `size` bytes that outlive `CreateProcessW`.
        if unsafe {
            UpdateProcThreadAttribute(self.as_mut_ptr(), 0, attribute as usize, value, size, null_mut(), null())
        } == 0
        {
            return Err(last_error("UpdateProcThreadAttribute"));
        }
        Ok(())
    }
}

impl Drop for AttrList {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: initialized by `InitializeProcThreadAttributeList`.
            unsafe { DeleteProcThreadAttributeList(self.as_mut_ptr()) };
            self.initialized = false;
        }
    }
}

/// Child process launched directly with `CreateProcessW` + `STARTUPINFOEXW` and
/// attached to a kill-on-close Job Object.
pub struct WinChild {
    proc_handle: HANDLE,
    pid: u32,
    job: Job,
}
// SAFETY: Win32 process and job handles are thread-safe to wait on, terminate, and close.
unsafe impl Send for WinChild {}

impl Contained for WinChild {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn try_wait(&mut self) -> io::Result<Option<u32>> {
        // SAFETY: `self.proc_handle` is a valid process handle owned by `self`.
        match unsafe { WaitForSingleObject(self.proc_handle, 0) } {
            WAIT_OBJECT_0 => {
                let mut code: u32 = 0;
                if unsafe { GetExitCodeProcess(self.proc_handle, &mut code) } == 0 {
                    return Err(last_error("GetExitCodeProcess"));
                }
                Ok(Some(code))
            }
            WAIT_TIMEOUT => Ok(None),
            WAIT_FAILED => Err(last_error("WaitForSingleObject")),
            other => Err(io::Error::other(format!("unexpected WaitForSingleObject result: {other}"))),
        }
    }

    fn kill_tree(&mut self) {
        // SAFETY: `self.job.0` and `self.proc_handle` are valid Win32 handles owned by `self`.
        unsafe {
            TerminateJobObject(self.job.0, 1);
            WaitForSingleObject(self.proc_handle, 5000);
        }
    }
}

impl Drop for WinChild {
    fn drop(&mut self) {
        if !self.proc_handle.is_null() {
            // SAFETY: `proc_handle` is owned by `WinChild` and closed once.
            unsafe { CloseHandle(self.proc_handle) };
            self.proc_handle = null_mut();
        }
    }
}

fn push_quoted_arg(cmd: &mut String, arg: &str) {
    if !arg.is_empty() && !arg.bytes().any(|b| matches!(b, b' ' | b'\t' | b'"')) {
        cmd.push_str(arg);
        return;
    }
    cmd.push('"');
    let mut backslashes = 0usize;
    for ch in arg.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                cmd.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                cmd.push('"');
                backslashes = 0;
            }
            _ => {
                cmd.extend(std::iter::repeat_n('\\', backslashes));
                backslashes = 0;
                cmd.push(ch);
            }
        }
    }
    cmd.extend(std::iter::repeat_n('\\', backslashes * 2));
    cmd.push('"');
}

fn build_command_line(exe: &Path, args: &[String]) -> io::Result<Vec<u16>> {
    let exe_str = exe.to_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "non-UTF-8 exe path"))?;
    let mut cmd = String::new();
    push_quoted_arg(&mut cmd, exe_str);
    for arg in args {
        cmd.push(' ');
        push_quoted_arg(&mut cmd, arg);
    }
    encode_wide_z(OsStr::new(&cmd))
}

fn build_env_block(env: &BTreeMap<String, String>) -> io::Result<Vec<u16>> {
    let mut pairs: Vec<(String, String)> = Vec::with_capacity(env.len() + 1);
    let mut has_system_root = false;
    for (k, v) in env {
        if k.is_empty() || k.contains('=') || k.contains('\0') || v.contains('\0') {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("invalid env entry `{k}`")));
        }
        if k.eq_ignore_ascii_case("SystemRoot") {
            has_system_root = true;
        }
        pairs.push((k.clone(), v.clone()));
    }
    if !has_system_root {
        let sr = std::env::var("SystemRoot").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| r"C:\Windows".to_string());
        if sr.contains('\0') {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "SystemRoot contains NUL"));
        }
        pairs.push(("SystemRoot".to_string(), sr));
    }
    pairs.sort_by(|(a, _), (b, _)| a.to_ascii_uppercase().cmp(&b.to_ascii_uppercase()).then_with(|| a.cmp(b)));
    for w in pairs.windows(2) {
        if w[0].0.eq_ignore_ascii_case(&w[1].0) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("duplicate env key `{}`", w[1].0)));
        }
    }
    let mut out = Vec::new();
    for (k, v) in pairs {
        out.extend(OsStr::new(&k).encode_wide());
        out.push(b'=' as u16);
        out.extend(OsStr::new(&v).encode_wide());
        out.push(0);
    }
    if out.is_empty() {
        out.push(0);
    }
    out.push(0);
    Ok(out)
}

/// Spawn a worker via `CreateProcessW` + `STARTUPINFOEXW` using `HANDLE_LIST`
/// and optional `SECURITY_CAPABILITIES` (without LPAC), inside a UI-restricted kill-on-close Job.
pub fn spawn_extended(launch: &WorkerLaunch, profile: Option<&AppContainerProfile>) -> io::Result<WinChild> {
    let appcontainer_sid = profile.map(AppContainerProfile::psid);
    let nul = NulStdHandles::open()?;
    let mut handles: Vec<HANDLE> = Vec::with_capacity(launch.inherit_handles.len() + 2);
    for &raw in &launch.inherit_handles {
        let h = raw as HANDLE;
        if h.is_null() || h == INVALID_HANDLE_VALUE {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid handle in inherit_handles"));
        }
        if !handles.iter().any(|existing| std::ptr::eq(*existing, h)) {
            handles.push(h);
        }
    }
    for h in [nul.stdin, nul.stdout] {
        if !handles.iter().any(|existing| std::ptr::eq(*existing, h)) {
            handles.push(h);
        }
    }

    let caps = appcontainer_sid.map(|psid| SECURITY_CAPABILITIES {
        AppContainerSid: psid,
        Capabilities: null_mut(),
        CapabilityCount: 0,
        Reserved: 0,
    });
    let mut attr_list = AttrList::new(1 + u32::from(caps.is_some()))?;
    attr_list.update(
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
        handles.as_ptr().cast(),
        std::mem::size_of_val(handles.as_slice()),
    )?;
    if let Some(ref c) = caps {
        attr_list.update(
            PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
            (c as *const SECURITY_CAPABILITIES).cast(),
            size_of::<SECURITY_CAPABILITIES>(),
        )?;
    }

    let exe_clean = strip_verbatim(&launch.exe);
    let exe_w = encode_wide_z(exe_clean.as_os_str())?;
    let mut cmd_w = build_command_line(&exe_clean, &launch.args)?;
    let env_w = build_env_block(&launch.env)?;
    let cwd_w = encode_wide_z(strip_verbatim(&launch.cwd).as_os_str())?;

    // SAFETY: `StartupInfo.cb` is `STARTUPINFOEXW`'s size and `lpAttributeList` stays alive past `CreateProcessW`.
    // Pass `L""` (not NULL) for `lpDesktop` when launching in AppContainer so Win32 connects via the
    // logon session's default window station (`WinSta0\Default`), whose OS DACL grants `S-1-15-2-1`;
    // inheriting a custom parent desktop (`NULL`) fails `user32.dll` / `_ctypes.pyd` init (`ERROR_DLL_INIT_FAILED`)
    // when the parent runs inside a restricted desktop without an AppContainer ACE.
    let mut empty_desktop: [u16; 1] = [0];
    let mut si: STARTUPINFOEXW = unsafe { zeroed() };
    si.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    if appcontainer_sid.is_some() {
        si.StartupInfo.lpDesktop = empty_desktop.as_mut_ptr();
    }
    si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    si.StartupInfo.hStdInput = nul.stdin;
    si.StartupInfo.hStdOutput = nul.stdout;
    si.StartupInfo.hStdError = nul.stdout;
    si.lpAttributeList = attr_list.as_mut_ptr();

    let job = create_kill_on_close_job()?;
    restrict_job_ui(&job)?;
    let flags = EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW;
    // SAFETY: `PROCESS_INFORMATION` is zeroed out-parameter; all buffers remain valid until `CreateProcessW` returns.
    let mut pi: PROCESS_INFORMATION = unsafe { zeroed() };
    let ok = unsafe {
        CreateProcessW(
            exe_w.as_ptr(),
            cmd_w.as_mut_ptr(),
            null_mut(),
            null_mut(),
            1,
            flags,
            env_w.as_ptr().cast(),
            cwd_w.as_ptr(),
            (&si.StartupInfo) as *const STARTUPINFOW,
            &mut pi,
        )
    };
    let spawn_err = (ok == 0).then(|| last_error("CreateProcessW"));
    let _ = (&caps, &exe_w, &cmd_w, &env_w, &cwd_w, &empty_desktop);
    drop(attr_list);
    drop(handles);
    drop(nul);
    if let Some(err) = spawn_err {
        return Err(err);
    }

    // SAFETY: `pi.hProcess` and `pi.hThread` are valid newly created handles.
    if unsafe { AssignProcessToJobObject(job.0, pi.hProcess) } == 0 {
        let e = last_error("AssignProcessToJobObject");
        unsafe {
            TerminateProcess(pi.hProcess, 1);
            CloseHandle(pi.hThread);
            CloseHandle(pi.hProcess);
        }
        return Err(e);
    }
    if unsafe { ResumeThread(pi.hThread) } == u32::MAX {
        let e = last_error("ResumeThread");
        unsafe {
            TerminateProcess(pi.hProcess, 1);
            CloseHandle(pi.hThread);
            CloseHandle(pi.hProcess);
        }
        return Err(e);
    }
    unsafe { CloseHandle(pi.hThread) };
    Ok(WinChild { proc_handle: pi.hProcess, pid: pi.dwProcessId, job })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_quotes_spaces_quotes_and_backslashes() {
        let args = vec![
            "".into(),
            "plain".into(),
            "two words".into(),
            r#"say "hi""#.into(),
            r#"a\"b"#.into(),
            r#"C:\dir with space\"#.into(),
        ];
        let wide = build_command_line(Path::new(r"C:\Program Files\py.exe"), &args).unwrap();
        let text = String::from_utf16_lossy(&wide[..wide.len() - 1]);
        assert_eq!(
            text,
            r#""C:\Program Files\py.exe" "" plain "two words" "say \"hi\"" "a\\\"b" "C:\dir with space\\""#
        );
    }

    #[test]
    fn env_block_sorts_case_insensitively_adds_system_root_and_rejects_bad_entries() {
        let mut env = BTreeMap::new();
        env.insert("zeta".into(), "1".into());
        env.insert("Alpha".into(), "2".into());
        let block = build_env_block(&env).unwrap();
        assert!(block.ends_with(&[0, 0]));
        let decoded = String::from_utf16_lossy(&block[..block.len() - 2]);
        let entries: Vec<&str> = decoded.split('\0').collect();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0], "Alpha=2");
        assert!(entries[1].to_ascii_uppercase().starts_with("SYSTEMROOT="));
        assert_eq!(entries[2], "zeta=1");

        let mut bad = BTreeMap::new();
        bad.insert("A=B".into(), "1".into());
        assert!(build_env_block(&bad).is_err());
    }
}
