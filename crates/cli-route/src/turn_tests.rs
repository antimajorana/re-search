use super::*;
use crate::vendor::VersionInfo;
use ledger::{AppendedEvent, Ledger};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;

struct NoopSink;
impl EventSink for NoopSink {
    fn on_event(&mut self, _receipt: &AppendedEvent, _kind: &str, _data: &Value) -> Result<(), String> {
        Ok(())
    }
}

struct StubVendor {
    tool_config_seen_in_version: AtomicBool,
    writes: AtomicUsize,
    version_result: Result<VersionInfo, String>,
}

impl StubVendor {
    fn ok() -> Self {
        Self {
            tool_config_seen_in_version: AtomicBool::new(false),
            writes: AtomicUsize::new(0),
            version_result: Ok(VersionInfo { raw: "1.0".into(), detected: "1.0".into(), tested: "1.0".into() }),
        }
    }
}

impl VendorCli for StubVendor {
    fn route(&self) -> &'static str {
        "stub"
    }
    fn caller(&self, session: &str) -> String {
        format!("stub:{session}")
    }
    fn locate(&self, explicit: Option<&Path>, _env: &dyn Fn(&str) -> Option<OsString>) -> Result<PathBuf, String> {
        Ok(explicit.unwrap_or_else(|| Path::new("stub")).to_path_buf())
    }
    fn version(&self, _exe: &Path) -> Result<VersionInfo, String> {
        if self.writes.load(Ordering::Acquire) > 0 {
            self.tool_config_seen_in_version.store(true, Ordering::Release);
        }
        self.version_result.clone()
    }
    fn env_auth_vars(&self) -> &'static [&'static str] {
        &[]
    }
    fn tool_name(&self, tool_id: &str) -> String {
        tool_id.to_string()
    }
    fn write_tool_config(&self, path: &Path, _core: &CoreLaunch) -> Result<(), String> {
        self.writes.fetch_add(1, Ordering::AcqRel);
        std::fs::write(path, b"{}").map_err(|e| e.to_string())
    }
    fn args(&self, _model: &str, _tool_config: &Path) -> Vec<String> {
        Vec::new()
    }
    fn prepare(&self, _cmd: &mut Command) {}
    fn is_handshake(&self, line: &Value) -> bool {
        line["event"] == "init"
    }
    fn check_handshake(&self, _line: &Value, _expected_tools: &[String]) -> Result<(), String> {
        Ok(())
    }
    fn check_auth(&self, _line: &Value, _allow_env_auth: bool) -> Result<(), String> {
        Ok(())
    }
    fn handshake_version(&self, _line: &Value) -> Option<String> {
        None
    }
    fn is_model_activity(&self, line: &Value) -> bool {
        line["event"] == "result"
    }
    fn map_line(&self, line: &Value) -> Vec<(String, Value)> {
        vec![("cli.other".into(), line.clone())]
    }
}

fn make_spec<'a>(
    session: &'a str,
    exe: &'a Path,
    work_dir: &'a Path,
    tool_config: &'a Path,
    core: &'a CoreLaunch,
) -> TurnSpec<'a> {
    TurnSpec {
        session,
        exe,
        work_dir,
        tool_config,
        core,
        model: "m",
        prompt: "hi",
        tool_defs: &[],
        policy_version: "v0",
        start_kind: "session.start",
        start_fields: json!({ "prompt": "hi" }),
        inherit_stderr: false,
        allow_env_auth: false,
    }
}

#[test]
fn tool_config_is_not_written_when_version_fails_or_turn_is_stopped() {
    let dir = std::env::temp_dir().join(format!("cli-route-turn-cfg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("mcp.json");
    let core = CoreLaunch { exe: PathBuf::from("core"), env: BTreeMap::new() };
    let rec = Recorder { ledger: Ledger::open(&dir).unwrap(), session: "s-1".into() };
    let spec = make_spec("s-1", Path::new("stub"), &dir, &cfg, &core);

    let mut fail_vendor = StubVendor::ok();
    fail_vendor.version_result = Err("version failed".into());
    assert!(run_turn(&fail_vendor, &rec, &mut NoopSink, &spec, &StopHandle::new()).is_err());
    assert_eq!(fail_vendor.writes.load(Ordering::Acquire), 0);
    assert!(!cfg.exists());

    let stop = StopHandle::new();
    stop.stop();
    let ok_vendor = StubVendor::ok();
    let report = run_turn(&ok_vendor, &rec, &mut NoopSink, &spec, &stop).unwrap();
    assert_eq!(report.end, TurnEnd::Stopped);
    assert_eq!(ok_vendor.writes.load(Ordering::Acquire), 0);
    assert!(!cfg.exists());

    // Even when spawn fails (e.g. non-Windows stub sandbox or missing exe), version() ran
    // before write_tool_config and ToolConfigGuard cleans up the config file on exit.
    let spawn_fail_vendor = StubVendor::ok();
    let _ = run_turn(&spawn_fail_vendor, &rec, &mut NoopSink, &spec, &StopHandle::new());
    assert!(!spawn_fail_vendor.tool_config_seen_in_version.load(Ordering::Acquire));
    assert_eq!(spawn_fail_vendor.writes.load(Ordering::Acquire), 1);
    assert!(!cfg.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

struct StubProc {
    code: Option<i32>,
    killed: bool,
}

impl TurnProcess for StubProc {
    fn kill(&mut self) {
        self.killed = true;
    }
    fn wait(&mut self) -> Result<Option<i32>, String> {
        self.killed = true;
        Ok(self.code)
    }
}

#[test]
fn cli_exit_zero_without_init_records_session_abort() {
    let dir = std::env::temp_dir().join(format!("cli-route-turn-noinit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let rec = Recorder { ledger: Ledger::open(&dir).unwrap(), session: "s-noinit".into() };
    let mut sink = NoopSink;
    rec.emit_into(&mut sink, "session.start", json!({ "prompt": "hi" }), None).unwrap();

    let vendor = StubVendor::ok();
    let version = VersionInfo { raw: "1.0".into(), detected: "1.0".into(), tested: "1.0".into() };
    let mut reader = Cursor::new(Vec::<u8>::new());
    let stop = StopHandle::new();
    let mut proc = StubProc { code: Some(0), killed: false };
    let report =
        drive_turn_stream(&vendor, &rec, &mut sink, &version, &[], false, &stop, &mut reader, &mut proc).unwrap();

    assert!(proc.killed);
    assert_eq!(report.end, TurnEnd::Aborted("CLI exited without a verified init handshake".into()));
    let events = rec.ledger.read_session("s-noinit").unwrap();
    let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
    assert!(kinds.contains(&"session.abort"), "{kinds:?}");
    assert!(!kinds.contains(&"session.end"), "{kinds:?}");
    let prior = crate::history::prior_turns(&events);
    assert_eq!(prior.len(), 1);
    assert!(prior[0].aborted);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn model_activity_before_init_mentions_init_handshake() {
    let dir = std::env::temp_dir().join(format!("cli-route-turn-preinit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let rec = Recorder { ledger: Ledger::open(&dir).unwrap(), session: "s-preinit".into() };
    let mut sink = NoopSink;
    rec.emit_into(&mut sink, "session.start", json!({ "prompt": "hi" }), None).unwrap();

    let vendor = StubVendor::ok();
    let version = VersionInfo { raw: "1.0".into(), detected: "1.0".into(), tested: "1.0".into() };
    let mut reader = Cursor::new(b"{\"event\":\"result\"}\n".to_vec());
    let stop = StopHandle::new();
    let mut proc = StubProc { code: Some(0), killed: false };
    let report =
        drive_turn_stream(&vendor, &rec, &mut sink, &version, &[], false, &stop, &mut reader, &mut proc).unwrap();
    assert!(proc.killed);
    match report.end {
        TurnEnd::Aborted(reason) => {
            assert!(reason.contains("init handshake"), "{reason}");
            assert!(!reason.contains("system/init"), "{reason}");
        }
        other => panic!("expected Aborted, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}
