//! Per-session zero-capability Windows AppContainer sandbox backend.

use crate::sandbox::{Contained, Containment, Sandbox, WorkerLaunch};
use crate::stage::{
    ensure_staged_runtime, grant_icacls_inside, is_same_or_inside, is_strictly_inside, lexical_same_or_inside,
    lexical_strictly_inside, strip_verbatim, StagedRuntime,
};
use crate::win::{encode_wide_z, sid_to_string, spawn_extended};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::ptr::{null, null_mut};
use std::time::Duration;
use windows_sys::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeleteAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
};
use windows_sys::Win32::Security::{FreeSid, PSID};

const PROFILE_PREFIX: &str = "re-search.worker.";
const MAX_PROFILE_LEN: usize = 60;
const MAX_SESSION_ID_LEN: usize = MAX_PROFILE_LEN - PROFILE_PREFIX.len();
const STALE_PROFILE_AGE: Duration = Duration::from_secs(2 * 60 * 60);
const ORPHAN_PROFILE_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const HRESULT_ALREADY_EXISTS: i32 = 0x800700B7_u32 as i32;

fn try_profile_name_for_session(session: &str) -> io::Result<String> {
    if session.is_empty()
        || session.len() > MAX_SESSION_ID_LEN
        || matches!(session, "." | "..")
        || session.ends_with('.')
        || !session
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "session id must be 1..=43 lowercase ASCII [a-z0-9._-] characters without trailing dot",
        ));
    }
    Ok(format!("{PROFILE_PREFIX}{session}"))
}

/// Map a validated `session` id injectively into an AppContainer profile name (`<= 60` chars).
pub fn profile_name_for_session(session: &str) -> String {
    try_profile_name_for_session(session).expect("session id must be 1..=43 lowercase ASCII [a-z0-9._-]")
}

/// RAII handle for a per-session Windows AppContainer profile and its SID.
pub struct AppContainerProfile {
    name: String,
    sid_str: String,
    psid: PSID,
}

// SAFETY: The `PSID` buffer is owned exclusively by `AppContainerProfile` (freed once in `Drop`)
// and Windows SIDs are immutable across threads.
unsafe impl Send for AppContainerProfile {}
unsafe impl Sync for AppContainerProfile {}

impl AppContainerProfile {
    pub fn create(session: &str) -> io::Result<Self> {
        let name = try_profile_name_for_session(session)?;
        let name_w = encode_wide_z(OsStr::new(&name))?;
        let mut psid: PSID = null_mut();
        // SAFETY: `name_w` is NUL-terminated UTF-16 and `psid` is a valid out-pointer.
        let hr = unsafe {
            CreateAppContainerProfile(name_w.as_ptr(), name_w.as_ptr(), name_w.as_ptr(), null(), 0, &mut psid)
        };
        if hr == HRESULT_ALREADY_EXISTS {
            // SAFETY: profile exists; derive its SID by exact name.
            let hr2 = unsafe { DeriveAppContainerSidFromAppContainerName(name_w.as_ptr(), &mut psid) };
            if hr2 != 0 {
                return Err(io::Error::other(format!(
                    "DeriveAppContainerSidFromAppContainerName({name}) failed: 0x{:08x}",
                    hr2 as u32
                )));
            }
        } else if hr != 0 {
            return Err(io::Error::other(format!("CreateAppContainerProfile({name}) failed: 0x{:08x}", hr as u32)));
        }
        if psid.is_null() {
            return Err(io::Error::other(format!("AppContainer profile `{name}` yielded a null PSID")));
        }
        // SAFETY: `psid` is a valid SID allocated by UserEnv; freed below on error or in `Drop`.
        let sid_str = match unsafe { sid_to_string(psid) } {
            Ok(s) => s,
            Err(e) => {
                unsafe { FreeSid(psid) };
                return Err(e);
            }
        };
        Ok(Self { name, sid_str, psid })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn sid_str(&self) -> &str {
        &self.sid_str
    }

    pub(crate) fn psid(&self) -> *mut core::ffi::c_void {
        self.psid
    }
}

impl Drop for AppContainerProfile {
    fn drop(&mut self) {
        if !self.psid.is_null() {
            // SAFETY: `psid` was allocated by `CreateAppContainerProfile` / `DeriveAppContainerSidFromAppContainerName`.
            unsafe { FreeSid(self.psid) };
            self.psid = null_mut();
        }
        if let Ok(name_w) = encode_wide_z(OsStr::new(&self.name)) {
            // SAFETY: best-effort profile cleanup; supervisor also calls `delete_session_profile`.
            unsafe { DeleteAppContainerProfile(name_w.as_ptr()) };
        }
    }
}

/// Best-effort deletion of the AppContainer profile belonging to `session`.
pub fn delete_session_profile(session: &str) {
    let Ok(name) = try_profile_name_for_session(session) else {
        return;
    };
    if let Ok(name_w) = encode_wide_z(OsStr::new(&name)) {
        // SAFETY: `name_w` is NUL-terminated UTF-16.
        unsafe { DeleteAppContainerProfile(name_w.as_ptr()) };
    }
}

fn has_live_pid_lock(lock_file: &Path) -> bool {
    fs::read_to_string(lock_file)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .is_some_and(|pid| !crate::win::process_is_gone(pid, 0))
}

fn file_age(path: &Path) -> Option<Duration> {
    fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok())
}

fn is_session_profile_stale(sess_dir: &Path, pkg_age: Option<Duration>) -> bool {
    if !sess_dir.is_dir() {
        return pkg_age.is_some_and(|age| age > ORPHAN_PROFILE_AGE);
    }
    let active_lock = sess_dir.join(".active.lock");
    let worker_lock = sess_dir.join(".worker.lock");
    if has_live_pid_lock(&active_lock) || has_live_pid_lock(&worker_lock) {
        return false;
    }
    file_age(&active_lock).is_some_and(|age| age > STALE_PROFILE_AGE)
        || file_age(&worker_lock).is_some_and(|age| age > STALE_PROFILE_AGE)
        || pkg_age.is_some_and(|age| age > STALE_PROFILE_AGE)
}

/// Sweep orphaned `%LOCALAPPDATA%\Packages\re-search.worker.*` profiles.
pub fn sweep_stale_profiles(data_dir: &Path) {
    let Some(local) = std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty()) else {
        return;
    };
    let packages_dir = PathBuf::from(local).join("Packages");
    let Ok(entries) = fs::read_dir(packages_dir) else {
        return;
    };
    let sessions_root = data_dir.join("sessions");
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        let Some(sess_part) = name_str.strip_prefix(PROFILE_PREFIX) else {
            continue;
        };
        if sess_part.is_empty() || sess_part.contains(['/', '\\']) || matches!(sess_part, "." | "..") {
            continue;
        }
        let pkg_age = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
        let sess_dir = sessions_root.join(sess_part);
        if is_session_profile_stale(&sess_dir, pkg_age) {
            if let Ok(name_w) = encode_wide_z(OsStr::new(name_str)) {
                // SAFETY: `name_str` starts with `"re-search.worker."` and is NUL-terminated.
                unsafe { DeleteAppContainerProfile(name_w.as_ptr()) };
            }
        }
    }
}

fn validate_session_subdir(path: &Path, session_dir: &Path, work_dir: &Path, label: &str) -> io::Result<()> {
    if !lexical_strictly_inside(path, session_dir)? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{label} must be strictly inside session dir {}: {}", session_dir.display(), path.display()),
        ));
    }
    if lexical_same_or_inside(path, work_dir)? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{label} must not overlap session work dir {}", work_dir.display()),
        ));
    }
    Ok(())
}

/// Zero-capability Windows AppContainer sandbox with fingerprinted runtime staging
/// and per-session `scratch`/`pycache` ACLs.
pub struct AppContainerSandbox {
    staged: StagedRuntime,
    profile: AppContainerProfile,
    scratch: PathBuf,
    pycache: PathBuf,
    worker_lock: PathBuf,
}

impl Drop for AppContainerSandbox {
    fn drop(&mut self) {
        let my_pid = std::process::id().to_string();
        if fs::read_to_string(&self.worker_lock).ok().is_some_and(|s| s.trim() == my_pid) {
            let _ = fs::remove_file(&self.worker_lock);
        }
    }
}

impl AppContainerSandbox {
    pub fn new(
        data_dir: PathBuf,
        session: &str,
        source_python: PathBuf,
        worker_script: PathBuf,
        scratch: PathBuf,
        pycache: PathBuf,
    ) -> io::Result<Self> {
        if session.is_empty()
            || session.contains(['/', '\\', '\0'])
            || matches!(session, "." | "..")
            || session.chars().any(|c| c == ':')
        {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid session identifier"));
        }
        let _ = try_profile_name_for_session(session)?;
        if data_dir.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "data_dir must not contain `..`"));
        }

        let session_dir = data_dir.join("sessions").join(session);
        let work_dir = session_dir.join("work");
        validate_session_subdir(&scratch, &session_dir, &work_dir, "scratch")?;
        validate_session_subdir(&pycache, &session_dir, &work_dir, "pycache")?;

        fs::create_dir_all(&session_dir)?;
        fs::create_dir_all(&scratch)?;
        for sub in ["tmp", "home", "appdata", "localappdata", "mpl"] {
            fs::create_dir_all(scratch.join(sub))?;
        }
        fs::create_dir_all(&pycache)?;

        if !is_strictly_inside(&scratch, &session_dir)?
            || !is_strictly_inside(&pycache, &session_dir)?
            || (work_dir.exists()
                && (is_same_or_inside(&scratch, &work_dir)? || is_same_or_inside(&pycache, &work_dir)?))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "canonicalized scratch or pycache escapes session isolation boundary",
            ));
        }

        let staged = ensure_staged_runtime(&data_dir, &source_python, &worker_script)?;
        let worker_lock = session_dir.join(".worker.lock");
        fs::write(&worker_lock, std::process::id().to_string())?;
        let profile = AppContainerProfile::create(session)?;
        let sid_grant = format!("*{}:(OI)(CI)M", profile.sid_str());
        grant_icacls_inside(&data_dir, &scratch, &sid_grant)?;
        grant_icacls_inside(&data_dir, &pycache, &sid_grant)?;

        Ok(Self {
            staged,
            profile,
            scratch: strip_verbatim(&fs::canonicalize(&scratch)?),
            pycache: strip_verbatim(&fs::canonicalize(&pycache)?),
            worker_lock,
        })
    }

    pub fn profile_sid(&self) -> &str {
        self.profile.sid_str()
    }

    pub fn staged(&self) -> &StagedRuntime {
        &self.staged
    }
}

impl Sandbox for AppContainerSandbox {
    fn containment(&self) -> Containment {
        Containment {
            backend: "appcontainer",
            filesystem_restricted: true,
            network_restricted: true,
            process_tree_killed_with_core: true,
        }
    }

    fn spawn_worker(&self, mut launch: WorkerLaunch) -> io::Result<Box<dyn Contained>> {
        if launch.args.len() < 2 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "worker launch args must contain `-u` and script path",
            ));
        }
        launch.exe = self.staged.python_exe.clone();
        launch.args[1] = strip_verbatim(&self.staged.worker_script).to_string_lossy().into_owned();
        launch.cwd = strip_verbatim(&self.scratch);

        let system_root = std::env::var("SystemRoot")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| r"C:\Windows".to_string());
        let p = |path: PathBuf| strip_verbatim(&path).to_string_lossy().into_owned();
        let mut env = BTreeMap::new();
        env.insert("SystemRoot".into(), system_root);
        env.insert("LOCALAPPDATA".into(), p(self.scratch.join("localappdata")));
        env.insert("APPDATA".into(), p(self.scratch.join("appdata")));
        env.insert("USERPROFILE".into(), p(self.scratch.join("home")));
        env.insert("HOME".into(), p(self.scratch.join("home")));
        env.insert("TEMP".into(), p(self.scratch.join("tmp")));
        env.insert("TMP".into(), p(self.scratch.join("tmp")));
        env.insert("MPLCONFIGDIR".into(), p(self.scratch.join("mpl")));
        env.insert("PYTHONPYCACHEPREFIX".into(), p(self.pycache.clone()));
        env.insert("PYTHONPATH".into(), p(self.staged.site_dir.clone()));
        env.insert("PYTHONNOUSERSITE".into(), "1".into());
        env.insert("PYTHONIOENCODING".into(), "utf-8".into());
        launch.env = env;

        Ok(Box::new(spawn_extended(&launch, Some(&self.profile))?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_name_is_injective_and_rejects_bad_or_overlong_session_ids() {
        let name = profile_name_for_session("s-8fda_1e0a.5933-0e46");
        assert_eq!(name, "re-search.worker.s-8fda_1e0a.5933-0e46");
        let max_valid = profile_name_for_session(&"a".repeat(MAX_SESSION_ID_LEN));
        assert_eq!(max_valid.len(), MAX_PROFILE_LEN);
        assert!(try_profile_name_for_session(&"a".repeat(MAX_SESSION_ID_LEN + 1)).is_err());
        assert!(try_profile_name_for_session("S-upper").is_err());
        assert!(try_profile_name_for_session("s/slash").is_err());
        assert!(try_profile_name_for_session("trailing.").is_err());
        assert!(try_profile_name_for_session("..").is_err());
    }

    #[test]
    fn stale_sweep_preserves_live_worker_or_fresh_session_without_active_lock() {
        let dir = std::env::temp_dir().join(format!("ac-sweep-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let sess_dir = dir.join("sessions").join("s-standalone");
        fs::create_dir_all(&sess_dir).unwrap();

        // Brand-new session directory without `.active.lock` (e.g. standalone `core` starting) is NOT stale.
        assert!(!is_session_profile_stale(&sess_dir, Some(Duration::from_secs(5))));
        // Old package age WITH a live `.worker.lock` PID is STILL not stale.
        fs::write(sess_dir.join(".worker.lock"), std::process::id().to_string()).unwrap();
        assert!(!is_session_profile_stale(&sess_dir, Some(STALE_PROFILE_AGE + Duration::from_secs(60))));
        // Dead PID + aged package IS stale.
        fs::write(sess_dir.join(".worker.lock"), u32::MAX.to_string()).unwrap();
        assert!(is_session_profile_stale(&sess_dir, Some(STALE_PROFILE_AGE + Duration::from_secs(60))));

        let _ = fs::remove_dir_all(&dir);
    }
}
