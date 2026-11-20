//! Windows helpers: inheritable pipes, kill-on-close Job Objects, scoped
//! `STARTUPINFOEXW` process launch (`HANDLE_LIST` + `SECURITY_CAPABILITIES`),
//! and process token inspection.

mod launch;
mod token;

pub use launch::{spawn_extended, WinChild};
pub use token::{inspect_process_token, TokenReport};
pub(crate) use token::sid_to_string;

use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::os::windows::process::CommandExt;
use std::process::{Child, Command};
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{
    CloseHandle, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_UILIMIT_DESKTOP, JOB_OBJECT_UILIMIT_DISPLAYSETTINGS,
    JOB_OBJECT_UILIMIT_EXITWINDOWS, JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES,
    JOB_OBJECT_UILIMIT_READCLIPBOARD, JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    OpenProcess, OpenThread, ResumeThread, WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED,
    PROCESS_SYNCHRONIZE, THREAD_SUSPEND_RESUME,
};

pub(crate) fn last_error(what: &str) -> io::Error {
    let e = io::Error::last_os_error();
    io::Error::new(e.kind(), format!("{what}: {e}"))
}

pub(crate) fn encode_wide_z(s: &OsStr) -> io::Result<Vec<u16>> {
    let mut out = Vec::with_capacity(s.len() + 1);
    for unit in s.encode_wide() {
        if unit == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "unexpected NUL byte in Win32 string"));
        }
        out.push(unit);
    }
    out.push(0);
    Ok(out)
}

pub(crate) struct OwnedHandle(pub(crate) HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            // SAFETY: handle is owned by `OwnedHandle` and closed once.
            unsafe { CloseHandle(self.0) };
            self.0 = null_mut();
        }
    }
}

/// One anonymous pipe. `ours` stays in this process; `theirs` is inheritable
/// and is handed to the child by handle value.
pub struct PipePair {
    ours: Option<File>,
    theirs: HANDLE,
}
// SAFETY: the raw handle is only closed once, by `close_theirs` or `Drop`.
unsafe impl Send for PipePair {}

pub enum Direction {
    ToChild,
    FromChild,
}

impl PipePair {
    pub fn new(direction: Direction) -> io::Result<PipePair> {
        let (mut read, mut write): (HANDLE, HANDLE) = (null_mut(), null_mut());
        // SAFETY: plain FFI call with valid out-pointers; both ends start non-inheritable.
        if unsafe { CreatePipe(&mut read, &mut write, null_mut(), 0) } == 0 {
            return Err(last_error("CreatePipe"));
        }
        let (ours, theirs) = match direction {
            Direction::ToChild => (write, read),
            Direction::FromChild => (read, write),
        };
        // SAFETY: `ours` is a valid handle we own; `File` takes ownership of it.
        let ours = unsafe { File::from_raw_handle(ours as _) };
        // SAFETY: `theirs` is a valid handle we own.
        if unsafe { SetHandleInformation(theirs, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) } == 0 {
            let e = last_error("SetHandleInformation");
            unsafe { CloseHandle(theirs) };
            return Err(e);
        }
        Ok(PipePair { ours: Some(ours), theirs })
    }

    /// The child's handle value, to pass on its command line.
    pub fn their_handle_value(&self) -> usize {
        self.theirs as usize
    }

    /// Close our copy of the child's end (after the child has been started) and return our end.
    pub fn into_ours(mut self) -> File {
        self.close_theirs();
        self.ours.take().expect("our pipe end is present until taken")
    }

    fn close_theirs(&mut self) {
        if !self.theirs.is_null() {
            // SAFETY: valid handle, closed exactly once.
            unsafe { CloseHandle(self.theirs) };
            self.theirs = null_mut();
        }
    }
}

impl Drop for PipePair {
    fn drop(&mut self) {
        self.close_theirs();
    }
}

pub(crate) struct Job(pub(crate) HANDLE);
// SAFETY: a job handle may be used and closed from any thread.
unsafe impl Send for Job {}

impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: valid job handle; closing it kills every process in the job.
        unsafe { CloseHandle(self.0) };
    }
}

pub(crate) fn create_kill_on_close_job() -> io::Result<Job> {
    // SAFETY: valid FFI calls; `Job` owns the created handle on success.
    unsafe {
        let handle = CreateJobObjectW(null_mut(), null_mut());
        if handle.is_null() {
            return Err(last_error("CreateJobObjectW"));
        }
        let job = Job(handle);
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) == 0
        {
            return Err(last_error("SetInformationJobObject"));
        }
        Ok(job)
    }
}

pub(crate) fn restrict_job_ui(job: &Job) -> io::Result<()> {
    let ui_limits = JOBOBJECT_BASIC_UI_RESTRICTIONS {
        UIRestrictionsClass: JOB_OBJECT_UILIMIT_HANDLES
            | JOB_OBJECT_UILIMIT_READCLIPBOARD
            | JOB_OBJECT_UILIMIT_WRITECLIPBOARD
            | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
            | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
            | JOB_OBJECT_UILIMIT_GLOBALATOMS
            | JOB_OBJECT_UILIMIT_DESKTOP
            | JOB_OBJECT_UILIMIT_EXITWINDOWS,
    };
    // SAFETY: `job.0` is a valid job handle and `ui_limits` matches `JobObjectBasicUIRestrictions`.
    if unsafe {
        SetInformationJobObject(
            job.0,
            JobObjectBasicUIRestrictions,
            (&ui_limits as *const JOBOBJECT_BASIC_UI_RESTRICTIONS).cast(),
            size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32,
        )
    } == 0
    {
        return Err(last_error("SetInformationJobObject(JobObjectBasicUIRestrictions)"));
    }
    Ok(())
}

pub struct JobChild {
    child: Child,
    job: Job,
}

impl JobChild {
    pub fn child(&mut self) -> &mut Child {
        &mut self.child
    }

    pub fn kill_tree(&mut self) {
        // SAFETY: valid job handle owned by `self`.
        unsafe { TerminateJobObject(self.job.0, 1) };
        let _ = self.child.wait();
    }
}

/// Start `command` suspended inside a fresh kill-on-close Job Object, then resume it.
pub fn spawn_in_job(mut command: Command) -> io::Result<JobChild> {
    let job = create_kill_on_close_job()?;
    command.creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
    let mut child = command.spawn()?;
    // SAFETY: `job.0` and `child` process handle are valid for this call.
    if unsafe { AssignProcessToJobObject(job.0, child.as_raw_handle() as HANDLE) } == 0 {
        let e = last_error("AssignProcessToJobObject");
        let _ = child.kill();
        return Err(e);
    }
    if let Err(e) = resume_threads(child.id()) {
        let _ = child.kill();
        return Err(e);
    }
    Ok(JobChild { child, job })
}

fn resume_threads(pid: u32) -> io::Result<()> {
    let mut resumed = 0;
    // SAFETY: snapshot handle is checked and closed; `THREADENTRY32` is plain data.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snap == INVALID_HANDLE_VALUE {
            return Err(last_error("CreateToolhelp32Snapshot"));
        }
        let _snap = OwnedHandle(snap);
        let mut entry: THREADENTRY32 = zeroed();
        entry.dwSize = size_of::<THREADENTRY32>() as u32;
        let mut more = Thread32First(snap, &mut entry);
        while more != 0 {
            if entry.th32OwnerProcessID == pid {
                let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                if !thread.is_null() {
                    if ResumeThread(thread) != u32::MAX {
                        resumed += 1;
                    }
                    CloseHandle(thread);
                }
            }
            more = Thread32Next(snap, &mut entry);
        }
    }
    if resumed == 0 {
        return Err(io::Error::other("could not resume the child's main thread"));
    }
    Ok(())
}

/// True if process `pid` no longer exists, or exits within `wait_ms`.
/// Fails closed (`false`) if `pid == 0` or if `OpenProcess` fails for any
/// reason other than `ERROR_INVALID_PARAMETER` (87, nonexistent PID) — e.g.,
/// `ERROR_ACCESS_DENIED` (5) on an elevated or other-user live process.
pub fn process_is_gone(pid: u32, wait_ms: u32) -> bool {
    const WIN32_ERROR_INVALID_PARAMETER: i32 = 87;
    if pid == 0 {
        return false;
    }
    // SAFETY: handle is checked and closed.
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            return io::Error::last_os_error().raw_os_error() == Some(WIN32_ERROR_INVALID_PARAMETER);
        }
        let gone = WaitForSingleObject(handle, wait_ms) == WAIT_OBJECT_0;
        CloseHandle(handle);
        gone
    }
}
