//! Windows pieces: inheritable pipes, and a Job Object that kills the worker
//! tree when its handle is closed.

use crate::sandbox::Contained;
use std::fs::File;
use std::io;
use std::mem::{size_of, zeroed};
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
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    OpenProcess, OpenThread, ResumeThread, WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED,
    PROCESS_SYNCHRONIZE, THREAD_SUSPEND_RESUME,
};

fn last_error(what: &str) -> io::Error {
    let e = io::Error::last_os_error();
    io::Error::new(e.kind(), format!("{what}: {e}"))
}

/// One anonymous pipe. `ours` stays in this process; `theirs` is inheritable
/// and is handed to the child by handle value.
pub struct PipePair {
    ours: Option<File>,
    theirs: HANDLE,
}

// SAFETY: the raw handle is only closed once, by `close_theirs` or Drop.
unsafe impl Send for PipePair {}

pub enum Direction {
    /// We write, the child reads.
    ToChild,
    /// The child writes, we read.
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
        // SAFETY: `ours` is a valid handle we own; File takes ownership of it.
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

    /// Close our copy of the child's end (after the child has been started),
    /// and return our end.
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

struct Job(HANDLE);
// SAFETY: a job handle may be used from any thread.
unsafe impl Send for Job {}

impl Drop for Job {
    fn drop(&mut self) {
        // Closing the last handle kills every process in the job
        // (JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE).
        unsafe { CloseHandle(self.0) };
    }
}

pub struct JobChild {
    child: Child,
    job: Job,
}

impl Contained for JobChild {
    fn child(&mut self) -> &mut Child {
        &mut self.child
    }

    fn kill_tree(&mut self) {
        // SAFETY: valid job handle.
        unsafe { TerminateJobObject(self.job.0, 1) };
        let _ = self.child.wait();
    }
}

/// Start the command suspended, put it in a fresh kill-on-close job, then let
/// it run. Because it is in the job before it executes anything, every
/// process it starts (for example the real interpreter behind a venv
/// launcher) is in the job too.
pub fn spawn_in_job(mut command: Command) -> io::Result<JobChild> {
    // SAFETY: FFI calls with valid arguments; handles are closed by `Job`.
    let job = unsafe {
        let handle = CreateJobObjectW(null_mut(), null_mut());
        if handle.is_null() {
            return Err(last_error("CreateJobObjectW"));
        }
        let job = Job(handle);
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if ok == 0 {
            return Err(last_error("SetInformationJobObject"));
        }
        job
    };
    command.creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
    let mut child = command.spawn()?;
    // SAFETY: both handles are valid for the duration of the call.
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

/// Resume the (single, initial) suspended thread of process `pid`.
fn resume_threads(pid: u32) -> io::Result<()> {
    let mut resumed = 0;
    // SAFETY: snapshot handle is checked and closed; THREADENTRY32 is plain data.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snap == INVALID_HANDLE_VALUE {
            return Err(last_error("CreateToolhelp32Snapshot"));
        }
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
        CloseHandle(snap);
    }
    if resumed == 0 {
        return Err(io::Error::new(io::ErrorKind::Other, "could not resume the worker's main thread"));
    }
    Ok(())
}

/// True if process `pid` no longer exists, or exits within `wait_ms`.
pub fn process_is_gone(pid: u32, wait_ms: u32) -> bool {
    // SAFETY: handle is checked and closed.
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            return true;
        }
        let gone = WaitForSingleObject(handle, wait_ms) == WAIT_OBJECT_0;
        CloseHandle(handle);
        gone
    }
}
