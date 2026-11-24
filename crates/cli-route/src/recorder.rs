//! Ledger recorder, event sinks, and the guards that clean up a session's lock, sandbox
//! profile and tool config on every exit path.

use crate::EVENT_VERSION;
use ledger::{AppendedEvent, Ledger};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Receives every event right after it is durably appended to the ledger.
pub trait EventSink {
    fn on_event(&mut self, receipt: &AppendedEvent, kind: &str, data: &Value) -> Result<(), String>;
}

/// One versioned NDJSON line (with the chain receipt) for an appended event.
pub fn ndjson_line(session: &str, ev: &AppendedEvent, kind: &str, payload: &Value) -> String {
    json!({
        "v": EVENT_VERSION,
        "session": session,
        "seq": ev.seq,
        "ts_ms": ev.ts_ms,
        "prev_hash": ev.prev_hash,
        "event_hash": ev.event_hash,
        "kind": kind,
        "data": payload,
    })
    .to_string()
}

/// Appends a session's events to the hash-chained ledger.
pub struct Recorder {
    pub ledger: Ledger,
    pub session: String,
}

impl Recorder {
    pub fn write_ndjson_line(
        &self,
        out: &mut impl Write,
        ev: &AppendedEvent,
        kind: &str,
        payload: &Value,
    ) -> Result<(), String> {
        let line = ndjson_line(&self.session, ev, kind, payload);
        writeln!(out, "{line}").and_then(|_| out.flush()).map_err(|e| e.to_string())
    }

    /// Append `data` (plus `raw_blob` when given) and return the receipt and stored payload.
    pub fn record(&self, kind: &str, data: Value, raw: Option<&str>) -> Result<(AppendedEvent, Value), String> {
        let mut payload = data;
        if let (Some(hash), Some(map)) = (raw, payload.as_object_mut()) {
            map.insert("raw_blob".into(), json!(hash));
        }
        let ev = self.ledger.append_with_hash(&self.session, kind, EVENT_VERSION, &payload).map_err(|e| e.to_string())?;
        Ok((ev, payload))
    }

    /// Append to the ledger and print the same event as one NDJSON line on stdout.
    pub fn emit(&self, kind: &str, data: Value, raw: Option<&str>) -> Result<(), String> {
        self.emit_to(&mut std::io::stdout().lock(), kind, data, raw)
    }

    pub fn emit_to(&self, out: &mut impl Write, kind: &str, data: Value, raw: Option<&str>) -> Result<(), String> {
        let (ev, payload) = self.record(kind, data, raw)?;
        self.write_ndjson_line(out, &ev, kind, &payload)
    }

    /// Append to the ledger, then hand the event to `sink`.
    pub fn emit_into(&self, sink: &mut dyn EventSink, kind: &str, data: Value, raw: Option<&str>) -> Result<(), String> {
        let (ev, payload) = self.record(kind, data, raw)?;
        sink.on_event(&ev, kind, &payload)
    }
}

/// Removes the session's `.active.lock` (when this process owns it) and, on Windows, the
/// session's AppContainer profile.
pub struct SessionGuard {
    #[cfg(windows)]
    pub session: String,
    pub lock_path: PathBuf,
}

impl SessionGuard {
    /// Write `<session_dir>/.active.lock` with this process id and sweep stale sandbox
    /// profiles (in that order, so this session is never swept as stale).
    pub fn create(data_dir: &Path, session_dir: &Path, session: &str) -> Result<SessionGuard, String> {
        let lock_path = session_dir.join(".active.lock");
        std::fs::write(&lock_path, std::process::id().to_string())
            .map_err(|e| format!("could not write {}: {e}", lock_path.display()))?;
        #[cfg(windows)]
        worker::appcontainer::sweep_stale_profiles(data_dir);
        let _ = (data_dir, session);
        Ok(SessionGuard {
            #[cfg(windows)]
            session: session.to_string(),
            lock_path,
        })
    }
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        let owned = match std::fs::read_to_string(&self.lock_path) {
            Ok(raw) => raw.trim().parse::<u32>().ok() == Some(std::process::id()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
            Err(_) => false,
        };
        if owned {
            let _ = std::fs::remove_file(&self.lock_path);
            #[cfg(windows)]
            worker::appcontainer::delete_session_profile(&self.session);
        }
    }
}

/// Deletes the per-turn tool config (it holds `CORE_SESSION_TOKEN`) when dropped, so it
/// is gone on every exit path of a turn.
pub struct ToolConfigGuard {
    path: PathBuf,
}

impl ToolConfigGuard {
    pub fn new(path: &Path) -> Self {
        ToolConfigGuard { path: path.to_path_buf() }
    }
}

impl Drop for ToolConfigGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// File name of the per-turn tool config in a session directory.
pub const TOOL_CONFIG_FILE: &str = "mcp.json";

/// Every per-turn tool config a session directory can hold, relative to it: the Claude
/// route's `mcp.json` and the Antigravity route's `agy/.agents/mcp_config.json`.
pub const TOOL_CONFIG_FILES: [&str; 2] = [TOOL_CONFIG_FILE, "agy/.agents/mcp_config.json"];

/// Write `bytes` to `path` with owner-only (`0o600`) permissions on Unix so per-turn tool
/// configs carrying `CORE_SESSION_TOKEN` are never world-readable.
pub(crate) fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| format!("could not write {}: {e}", path.display()))?;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("could not set permissions on {}: {e}", path.display()))?;
        f.write_all(bytes)
            .map_err(|e| format!("could not write {}: {e}", path.display()))
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes).map_err(|e| format!("could not write {}: {e}", path.display()))
    }
}

/// Whether the session's `.active.lock` names a running process other than this one.
/// Unknown liveness counts as running (nothing is deleted under a turn that may be live).
pub fn locked_by_other_live_process(session_dir: &Path) -> bool {
    let Some(pid) = std::fs::read_to_string(session_dir.join(".active.lock")).ok().and_then(|s| s.trim().parse::<u32>().ok())
    else {
        return false;
    };
    #[cfg(windows)]
    {
        pid != std::process::id() && !worker::win::process_is_gone(pid, 0)
    }
    #[cfg(target_os = "linux")]
    {
        pid != std::process::id() && Path::new(&format!("/proc/{pid}")).exists()
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        pid != std::process::id()
    }
}

/// Delete the session's tool configs ([`TOOL_CONFIG_FILES`]; they hold a turn's
/// `CORE_SESSION_TOKEN`) when no live turn owns them: `live_here` says whether this process
/// runs a turn for the session; a lock held by another running process also keeps them.
/// Covers configs left behind when the process was killed before its guards ran. Returns
/// whether a file was removed.
pub fn remove_stale_tool_config(session_dir: &Path, live_here: bool) -> bool {
    let present: Vec<PathBuf> =
        TOOL_CONFIG_FILES.iter().map(|rel| session_dir.join(rel)).filter(|p| p.is_file()).collect();
    if live_here || present.is_empty() || locked_by_other_live_process(session_dir) {
        return false;
    }
    // Every file is attempted (no short circuit), then report whether any went.
    let removed: Vec<bool> = present.iter().map(|p| std::fs::remove_file(p).is_ok()).collect();
    removed.contains(&true)
}

/// [`remove_stale_tool_config`] for every `sessions/*` directory under `data_dir`;
/// `live_here(session_id)` says whether this process runs a turn for that session.
/// Returns how many files were removed.
pub fn sweep_stale_tool_configs(data_dir: &Path, live_here: &dyn Fn(&str) -> bool) -> usize {
    let Ok(entries) = std::fs::read_dir(data_dir.join("sessions")) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter(|e| {
            let id = e.file_name().to_string_lossy().into_owned();
            remove_stale_tool_config(&e.path(), live_here(&id))
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_tool_configs_are_swept_unless_a_turn_owns_them() {
        let data = std::env::temp_dir().join(format!("cli-route-sweep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let mk = |id: &str, lock: Option<String>| {
            let d = data.join("sessions").join(id);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(TOOL_CONFIG_FILE), "{\"token\":\"t\"}").unwrap();
            if let Some(l) = lock {
                std::fs::write(d.join(".active.lock"), l).unwrap();
            }
            d.join(TOOL_CONFIG_FILE)
        };
        let no_lock = mk("s-nolock", None);
        let dead = mk("s-dead", Some(u32::MAX.to_string()));
        let ours_idle = mk("s-ours-idle", Some(std::process::id().to_string()));
        let ours_live = mk("s-ours-live", Some(std::process::id().to_string()));
        let removed = sweep_stale_tool_configs(&data, &|id| id == "s-ours-live");
        assert_eq!(removed, 3);
        assert!(!no_lock.exists() && !dead.exists() && !ours_idle.exists());
        assert!(ours_live.exists(), "a live turn's config is kept");
        assert!(!remove_stale_tool_config(&data.join("sessions").join("s-ours-live"), true));
        assert!(ours_live.exists());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn stale_antigravity_tool_config_is_swept_too() {
        let data = std::env::temp_dir().join(format!("cli-route-sweep-agy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let cfg = data.join("sessions").join("s-agy").join(TOOL_CONFIG_FILES[1]);
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        std::fs::write(&cfg, "{\"token\":\"t\"}").unwrap();
        assert_eq!(sweep_stale_tool_configs(&data, &|id| id == "s-agy"), 0, "kept while a turn owns it");
        assert!(cfg.exists());
        assert_eq!(sweep_stale_tool_configs(&data, &|_| false), 1);
        assert!(!cfg.exists());
        let _ = std::fs::remove_dir_all(&data);
    }

    struct Collect(Vec<(u64, String)>);
    impl EventSink for Collect {
        fn on_event(&mut self, receipt: &AppendedEvent, kind: &str, _data: &Value) -> Result<(), String> {
            self.0.push((receipt.seq, kind.to_string()));
            Ok(())
        }
    }

    #[test]
    fn emit_into_records_then_hands_the_event_to_the_sink() {
        let dir = std::env::temp_dir().join(format!("cli-route-recorder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rec = Recorder { ledger: Ledger::open(&dir).unwrap(), session: "s-rec".into() };
        let raw = rec.ledger.put_blob(b"line").unwrap();
        let mut sink = Collect(Vec::new());
        rec.emit_into(&mut sink, "session.start", json!({}), None).unwrap();
        rec.emit_into(&mut sink, "model.text", json!({ "text": "x" }), Some(&raw)).unwrap();
        assert_eq!(sink.0, vec![(1, "session.start".to_string()), (2, "model.text".to_string())]);
        let events = rec.ledger.read_session("s-rec").unwrap();
        assert_eq!(events[1].payload["raw_blob"], raw);
        assert_eq!(rec.ledger.verify_session("s-rec").unwrap().event_count, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tool_config_guard_removes_the_file() {
        let dir = std::env::temp_dir().join(format!("cli-route-cfg-guard-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mcp.json");
        std::fs::write(&path, "{}").unwrap();
        drop(ToolConfigGuard::new(&path));
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
