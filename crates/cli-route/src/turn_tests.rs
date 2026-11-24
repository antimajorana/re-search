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
        reasoning_effort: None,
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

    // Even when spawn fails, version() ran before write_tool_config and ToolConfigGuard
    // cleans up the config file on exit. The exe is an absolute path that does not exist
    // (never a bare name looked up on PATH), so the spawn fails on every machine; the
    // failure after the start event is recorded as `session.abort`.
    let missing_exe = dir.join("definitely-missing-cli.exe");
    let spawn_spec = make_spec("s-1", &missing_exe, &dir, &cfg, &core);
    let spawn_fail_vendor = StubVendor::ok();
    let report = run_turn(&spawn_fail_vendor, &rec, &mut NoopSink, &spawn_spec, &StopHandle::new()).unwrap();
    assert!(matches!(&report.end, TurnEnd::Aborted(r) if r.contains("could not start the CLI")), "{:?}", report.end);
    assert!(!spawn_fail_vendor.tool_config_seen_in_version.load(Ordering::Acquire));
    assert_eq!(spawn_fail_vendor.writes.load(Ordering::Acquire), 1);
    assert!(!cfg.exists());
    assert_eq!(last_kinds(&rec, 2), ["session.start", "session.abort"]);
    let _ = std::fs::remove_dir_all(&dir);
}

fn last_kinds(rec: &Recorder, n: usize) -> Vec<String> {
    let events = rec.ledger.read_session(&rec.session).unwrap();
    events.iter().rev().take(n).rev().map(|e| e.kind.clone()).collect()
}

struct FailingConfigVendor(StubVendor);

impl VendorCli for FailingConfigVendor {
    fn route(&self) -> &'static str {
        self.0.route()
    }
    fn caller(&self, session: &str) -> String {
        self.0.caller(session)
    }
    fn locate(&self, explicit: Option<&Path>, env: &dyn Fn(&str) -> Option<OsString>) -> Result<PathBuf, String> {
        self.0.locate(explicit, env)
    }
    fn version(&self, exe: &Path) -> Result<VersionInfo, String> {
        self.0.version(exe)
    }
    fn env_auth_vars(&self) -> &'static [&'static str] {
        &[]
    }
    fn tool_name(&self, tool_id: &str) -> String {
        tool_id.to_string()
    }
    fn write_tool_config(&self, _path: &Path, _core: &CoreLaunch) -> Result<(), String> {
        Err("disk full".into())
    }
    fn args(&self, model: &str, tool_config: &Path) -> Vec<String> {
        self.0.args(model, tool_config)
    }
    fn prepare(&self, _cmd: &mut Command) {}
    fn is_handshake(&self, line: &Value) -> bool {
        self.0.is_handshake(line)
    }
    fn check_handshake(&self, line: &Value, expected_tools: &[String]) -> Result<(), String> {
        self.0.check_handshake(line, expected_tools)
    }
    fn check_auth(&self, line: &Value, allow_env_auth: bool) -> Result<(), String> {
        self.0.check_auth(line, allow_env_auth)
    }
    fn handshake_version(&self, line: &Value) -> Option<String> {
        self.0.handshake_version(line)
    }
    fn is_model_activity(&self, line: &Value) -> bool {
        self.0.is_model_activity(line)
    }
    fn map_line(&self, line: &Value) -> Vec<(String, Value)> {
        self.0.map_line(line)
    }
}

#[test]
fn a_tool_config_write_failure_after_the_start_event_records_session_abort() {
    let dir = std::env::temp_dir().join(format!("cli-route-turn-cfgfail-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("mcp.json");
    let core = CoreLaunch { exe: PathBuf::from("core"), env: BTreeMap::new() };
    let rec = Recorder { ledger: Ledger::open(&dir).unwrap(), session: "s-cfgfail".into() };
    let missing_exe = dir.join("definitely-missing-cli.exe");
    let spec = make_spec("s-cfgfail", &missing_exe, &dir, &cfg, &core);
    let report = run_turn(&FailingConfigVendor(StubVendor::ok()), &rec, &mut NoopSink, &spec, &StopHandle::new()).unwrap();
    assert!(matches!(&report.end, TurnEnd::Aborted(r) if r.contains("tool configuration") && r.contains("disk full")), "{:?}", report.end);
    assert_eq!(last_kinds(&rec, 2), ["session.start", "session.abort"]);
    assert!(!cfg.exists());
    let prior = crate::history::prior_turns(&rec.ledger.read_session("s-cfgfail").unwrap());
    assert!(prior.last().is_some_and(|t| t.aborted));
    let _ = std::fs::remove_dir_all(&dir);
}

struct StubProc {
    code: Option<i32>,
    killed: bool,
    stop_on_wait: Option<StopHandle>,
}

impl TurnProcess for StubProc {
    fn kill(&mut self) {
        self.killed = true;
    }
    fn wait(&mut self) -> Result<Option<i32>, String> {
        if let Some(stop) = &self.stop_on_wait {
            stop.stop();
        }
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
    let mut proc = StubProc { code: Some(0), killed: false, stop_on_wait: None };
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

    rec.emit_into(&mut sink, "session.resume", json!({ "prompt": "try again" }), None).unwrap();
    let mut reader = Cursor::new(b"{\"event\":\"init\"}\n");
    let mut proc = StubProc { code: Some(0), killed: false, stop_on_wait: Some(stop.clone()) };
    let report =
        drive_turn_stream(&vendor, &rec, &mut sink, &version, &[], false, &stop, &mut reader, &mut proc).unwrap();
    assert_eq!(report.end, TurnEnd::Stopped);
    let stopped = rec.ledger.read_session("s-noinit").unwrap().pop().unwrap();
    assert_eq!(stopped.kind, "session.abort");
    assert_eq!(stopped.payload["stopped"], true);
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
    let mut proc = StubProc { code: Some(0), killed: false, stop_on_wait: None };
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

#[test]
fn turn_spec_forwards_reasoning_effort_into_claude_and_antigravity_args() {
    use crate::antigravity::Antigravity;
    use crate::claude::ClaudeCode;

    let mcp = PathBuf::from("C:/d/mcp.json");
    let claude_none = ClaudeCode.args_with_effort("sonnet", &mcp, None);
    let claude_eff = ClaudeCode.args_with_effort("sonnet", &mcp, Some("xhigh"));
    assert!(!claude_none.iter().any(|a| a == "--effort"));
    assert!(claude_eff.windows(2).any(|w| w == ["--effort", "xhigh"]), "{claude_eff:?}");

    let agy = Antigravity::new();
    let agy_none = agy.args_with_effort("m", &mcp, None);
    let agy_eff = agy.args_with_effort("m", &mcp, Some("medium"));
    assert!(!agy_none.iter().any(|a| a == "--effort"));
    assert!(agy_eff.windows(2).any(|w| w == ["--effort", "medium"]), "{agy_eff:?}");
}
