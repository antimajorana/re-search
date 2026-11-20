//! Session ID validation and atomic `.active.lock` acquisition.

use crate::session::SessionGuard;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

const MAX_SESSION_ID_LEN: usize = 43;

/// Enforce the exact session ID rules required across all platforms and by
/// `worker::appcontainer` (`1..=43` chars of `[a-z0-9._-]`, not `.` or `..`,
/// and no trailing `.`).
pub fn validate_session_id(session: &str) -> Result<(), String> {
    if session.is_empty()
        || session.len() > MAX_SESSION_ID_LEN
        || matches!(session, "." | "..")
        || session.ends_with('.')
        || !session
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
    {
        return Err(format!(
            "invalid session id `{session}` (expected 1..={MAX_SESSION_ID_LEN} lowercase ASCII [a-z0-9._-] characters without trailing dot)"
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn is_pid_definitely_dead(pid: u32) -> bool {
    pid != std::process::id() && worker::win::process_is_gone(pid, 0)
}

#[cfg(target_os = "linux")]
fn is_pid_definitely_dead(pid: u32) -> bool {
    pid != std::process::id() && !Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(all(not(windows), not(target_os = "linux")))]
fn is_pid_definitely_dead(_pid: u32) -> bool {
    false
}

fn read_lock_pid(path: &Path) -> Option<u32> {
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| raw.trim().parse::<u32>().ok())
        .filter(|&p| p > 0)
}

fn create_lock_file(lock_path: &Path) -> Result<std::fs::File, std::io::Error> {
    OpenOptions::new().write(true).create_new(true).open(lock_path)
}

fn write_pid_and_guard(
    mut file: std::fs::File,
    lock_path: PathBuf,
    #[cfg_attr(not(windows), allow(unused_variables))] session: &str,
) -> Result<SessionGuard, String> {
    let pid_text = std::process::id().to_string();
    file.write_all(pid_text.as_bytes())
        .and_then(|_| file.flush())
        .map_err(|e| format!("failed to write session lock {}: {e}", lock_path.display()))?;
    Ok(SessionGuard {
        #[cfg(windows)]
        session: session.to_string(),
        lock_path,
    })
}

struct ReclaimGuard(PathBuf);

impl Drop for ReclaimGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn acquire_reclaim_guard(session_dir: &Path) -> Option<ReclaimGuard> {
    let reclaim_path = session_dir.join(".reclaim.lock");
    match create_lock_file(&reclaim_path) {
        Ok(mut f) => {
            let _ = f.write_all(std::process::id().to_string().as_bytes());
            let _ = f.flush();
            Some(ReclaimGuard(reclaim_path))
        }
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {
            if read_lock_pid(&reclaim_path).is_some_and(is_pid_definitely_dead) {
                let _ = fs::remove_file(&reclaim_path);
                if let Ok(mut f) = create_lock_file(&reclaim_path) {
                    let _ = f.write_all(std::process::id().to_string().as_bytes());
                    let _ = f.flush();
                    return Some(ReclaimGuard(reclaim_path));
                }
            }
            None
        }
        Err(_) => None,
    }
}

/// Atomically create `<session_dir>/.active.lock` with `create_new(true)` and
/// our PID. Serializes stale-PID reclamation via `.reclaim.lock` so two racing
/// supervisors can never unlink each other's newly created lock.
pub fn acquire_session_lock(session_dir: &Path, session: &str) -> Result<SessionGuard, String> {
    validate_session_id(session)?;
    let lock_path = session_dir.join(".active.lock");
    let active_err = || {
        format!(
            "session `{session}` is currently active (lock held at {})",
            lock_path.display()
        )
    };
    match create_lock_file(&lock_path) {
        Ok(file) => write_pid_and_guard(file, lock_path, session),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {
            if !read_lock_pid(&lock_path).is_some_and(is_pid_definitely_dead) {
                return Err(active_err());
            }
            let Some(_reclaim) = acquire_reclaim_guard(session_dir) else {
                return Err(active_err());
            };
            // Re-verify under `.reclaim.lock` that `.active.lock` still holds a dead PID.
            match read_lock_pid(&lock_path) {
                Some(pid) if is_pid_definitely_dead(pid) => {
                    if let Err(rm_err) = fs::remove_file(&lock_path) {
                        if rm_err.kind() != ErrorKind::NotFound {
                            return Err(format!(
                                "session `{session}` lock at {} could not be reclaimed: {rm_err}",
                                lock_path.display()
                            ));
                        }
                    }
                }
                None if !lock_path.exists() => {}
                _ => return Err(active_err()),
            }
            match create_lock_file(&lock_path) {
                Ok(file) => write_pid_and_guard(file, lock_path, session),
                Err(_) => Err(active_err()),
            }
        }
        Err(e) => Err(format!("failed to create session lock {}: {e}", lock_path.display())),
    }
}
