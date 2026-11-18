//! Platform containment for the worker process.
//!
//! v0 has one backend, [`JobObjectSandbox`], which contains nothing except
//! the lifetime of the process tree: when the job handle is closed (the core
//! exits or drops the worker) every process in the tree is killed. It does
//! not restrict files or the network. Stronger Windows backends
//! (AppContainer, MXC) are meant to implement the same trait later.

use std::io;
use std::process::{Child, Command};

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

/// A spawned process together with whatever keeps it contained.
pub trait Contained: Send {
    fn child(&mut self) -> &mut Child;
    /// Kill the whole process tree now.
    fn kill_tree(&mut self);
}

pub trait Sandbox: Send + Sync {
    fn containment(&self) -> Containment;
    /// Start `command` contained. The command's inheritable handles are
    /// passed to the child.
    fn spawn(&self, command: Command) -> io::Result<Box<dyn Contained>>;
}

pub struct JobObjectSandbox;

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
    fn spawn(&self, command: Command) -> io::Result<Box<dyn Contained>> {
        Ok(Box::new(crate::win::spawn_in_job(command)?))
    }

    #[cfg(not(windows))]
    fn spawn(&self, _command: Command) -> io::Result<Box<dyn Contained>> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "the v0 worker runs on Windows only"))
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
