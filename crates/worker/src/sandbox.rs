//! Platform containment for the worker process.
//!
//! [`JobObjectSandbox`] enforces process-tree lifetime via a Windows Job Object
//! (`KILL_ON_JOB_CLOSE`) and scopes handle inheritance via
//! `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`, without restricting files or network.
//! [`crate::appcontainer::AppContainerSandbox`] additionally confines the
//! worker inside a zero-capability Windows AppContainer profile.

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::process::Command;

/// What a backend does and does not contain. Shown to the user when they are
/// asked to approve code execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Containment {
    pub backend: &'static str,
    pub filesystem_restricted: bool,
    pub network_restricted: bool,
    pub process_tree_killed_with_core: bool,
}

impl Containment {
    pub fn is_contained(&self) -> bool {
        self.filesystem_restricted && self.network_restricted
    }

    pub fn summary(&self) -> String {
        let yn = |b: bool| if b { "yes" } else { "NO" };
        format!(
            "sandbox backend `{}`: files restricted: {}; network restricted: {}; killed with the core: {}",
            self.backend,
            yn(self.filesystem_restricted),
            yn(self.network_restricted),
            yn(self.process_tree_killed_with_core)
        )
    }
}

/// Parameters for launching a worker process with explicit handle scoping and
/// environment isolation.
#[derive(Debug, Clone)]
pub struct WorkerLaunch {
    pub exe: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    /// Raw inheritable HANDLE values (cast to `usize`) explicitly allowed via
    /// `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`.
    pub inherit_handles: Vec<usize>,
}

/// A spawned worker process together with whatever keeps it contained.
pub trait Contained: Send {
    /// OS process ID of the root worker process.
    fn pid(&self) -> u32;
    /// Non-blocking exit check (`Ok(Some(code))` if exited, `Ok(None)` if running).
    fn try_wait(&mut self) -> io::Result<Option<u32>>;
    /// Kill the whole process tree now.
    fn kill_tree(&mut self);
}

pub trait Sandbox: Send + Sync {
    fn containment(&self) -> Containment;
    /// Optionally rewrite `WorkerLaunch` (for example staged python/worker paths
    /// and AppContainer environment) and spawn the worker.
    fn spawn_worker(&self, launch: WorkerLaunch) -> io::Result<Box<dyn Contained>>;
}

pub struct JobObjectSandbox;

#[cfg(not(windows))]
pub struct StubJobChild;

#[cfg(not(windows))]
impl StubJobChild {
    pub fn child(&mut self) -> &mut std::process::Child {
        unreachable!("StubJobChild cannot be constructed on non-Windows")
    }

    pub fn kill_tree(&mut self) {}
}

impl JobObjectSandbox {
    /// Start an arbitrary `Command` inside a fresh kill-on-close Job Object
    /// (used by the supervisor for `claude.exe`).
    #[cfg(windows)]
    pub fn spawn_command(&self, command: Command) -> io::Result<crate::win::JobChild> {
        crate::win::spawn_in_job(command)
    }

    #[cfg(not(windows))]
    pub fn spawn_command(&self, _command: Command) -> io::Result<StubJobChild> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "the worker runs on Windows only"))
    }
}

impl Sandbox for JobObjectSandbox {
    fn containment(&self) -> Containment {
        Containment {
            backend: "job-object",
            filesystem_restricted: false,
            network_restricted: false,
            process_tree_killed_with_core: true,
        }
    }

    #[cfg(windows)]
    fn spawn_worker(&self, launch: WorkerLaunch) -> io::Result<Box<dyn Contained>> {
        Ok(Box::new(crate::win::spawn_extended(&launch, None)?))
    }

    #[cfg(not(windows))]
    fn spawn_worker(&self, _launch: WorkerLaunch) -> io::Result<Box<dyn Contained>> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "the worker runs on Windows only"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_object_sandbox_is_not_contained() {
        assert!(!JobObjectSandbox.containment().is_contained());
        assert!(Containment {
            backend: "test",
            filesystem_restricted: true,
            network_restricted: true,
            process_tree_killed_with_core: true,
        }
        .is_contained());
        assert!(!Containment {
            backend: "fs-only",
            filesystem_restricted: true,
            network_restricted: false,
            process_tree_killed_with_core: true,
        }
        .is_contained());
    }
}
