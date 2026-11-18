#![cfg(windows)]

use ledger::Ledger;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct FakeClaude {
    dir: PathBuf,
    exe: PathBuf,
}

impl FakeClaude {
    fn compile(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("cli-fake-claude-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/fake_claude.rs");
        let exe = dir.join("fake_claude.exe");
        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let out = Command::new(rustc)
            .arg("--edition")
            .arg("2021")
            .arg(&src)
            .arg("-o")
            .arg(&exe)
            .output()
            .expect("failed to invoke rustc for fake_claude.rs");
        assert!(
            out.status.success(),
            "rustc fake_claude.rs failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        FakeClaude { dir, exe }
    }
}

impl Drop for FakeClaude {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn temp_data_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cli-init-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn supervisor_cmd(fake_exe: &Path, data_dir: &Path, prompt: &str) -> Command {
    let worker_script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../python/worker.py");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_re-search"));
    for (k, _) in std::env::vars_os() {
        let name = k.to_string_lossy();
        if name.starts_with("RE_SEARCH_") || name.starts_with("RESEARCH_CORE_") {
            cmd.env_remove(k);
        }
    }
    cmd.stdin(Stdio::null())
        .arg("run")
        .arg("--prompt")
        .arg(prompt)
        .arg("--claude")
        .arg(fake_exe)
        .arg("--data-dir")
        .arg(data_dir)
        .arg("--python")
        .arg(fake_exe)
        .arg("--worker-script")
        .arg(worker_script)
        .arg("--core-exe")
        .arg(fake_exe);
    cmd
}

#[test]
fn mismatched_init_aborts_live_and_kills_tree() {
    let fake = FakeClaude::compile("abort");
    let data_dir = temp_data_dir("abort");
    let pid_file = data_dir.join("grandchild.pid");
    let init_line = serde_json::json!({
        "type": "system",
        "subtype": "init",
        "tools": ["mcp__core__ledger_note", "mcp__core__sympy_eval", "Bash"],
        "mcp_servers": [{ "name": "core", "status": "connected" }],
        "model": "m",
        "claude_code_version": "9.9.9"
    })
    .to_string();

    let mut cmd = supervisor_cmd(&fake.exe, &data_dir, "test abort");
    cmd.env("FAKE_CLAUDE_VERSION", "9.9.9 (Claude Code)")
        .env("FAKE_CLAUDE_PID_FILE", &pid_file)
        .env("FAKE_CLAUDE_INIT_LINE", init_line)
        .env("FAKE_CLAUDE_SLEEP_MS", "60000");

    let start = Instant::now();
    let out = cmd.output().unwrap();
    let elapsed = start.elapsed();

    assert_eq!(
        out.status.code(),
        Some(3),
        "expected EXIT_ABORTED (3), stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(elapsed < Duration::from_secs(15), "abort took {elapsed:?}");

    let pid_text = std::fs::read_to_string(&pid_file).expect("grandchild pid should be written");
    let pid: u32 = pid_text.trim().parse().expect("valid pid");
    assert!(worker::win::process_is_gone(pid, 5000), "grandchild pid {pid} survived abort");

    let ledger = Ledger::open(&data_dir).unwrap();
    let sessions = ledger.sessions().unwrap();
    assert_eq!(sessions.len(), 1);
    let events = ledger.read_session(&sessions[0].0).unwrap();
    let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
    assert!(kinds.contains(&"session.start"), "{kinds:?}");
    let warn = events.iter().find(|e| e.kind == "cli.version_warning").expect("version warning");
    assert_eq!(warn.payload["detected"], "9.9.9");
    assert!(kinds.contains(&"cli.init"), "{kinds:?}");
    let abort = events.iter().find(|e| e.kind == "session.abort").expect("session.abort");
    assert!(abort.payload["reason"].as_str().unwrap_or("").contains("tool list mismatch"), "{abort:?}");
    assert!(!kinds.contains(&"session.end"), "{kinds:?}");

    let _ = std::fs::remove_dir_all(&data_dir);
}

#[test]
fn matching_cli_version_emits_no_version_warning() {
    let fake = FakeClaude::compile("match");
    let data_dir = temp_data_dir("match");
    let init_line = serde_json::json!({
        "type": "system",
        "subtype": "init",
        "tools": ["mcp__core__ledger_note", "mcp__core__sympy_eval"],
        "mcp_servers": [{ "name": "core", "status": "connected" }],
        "model": "m",
        "claude_code_version": "2.1.318"
    })
    .to_string();
    let result_line = serde_json::json!({
        "type": "result",
        "subtype": "success",
        "is_error": false,
        "result": "ok",
        "num_turns": 1,
        "duration_ms": 1
    })
    .to_string();

    let mut cmd = supervisor_cmd(&fake.exe, &data_dir, "test match");
    cmd.env("FAKE_CLAUDE_VERSION", "2.1.318 (Claude Code)")
        .env("FAKE_CLAUDE_INIT_LINE", init_line)
        .env("FAKE_CLAUDE_EXTRA_LINE", result_line);

    let out = cmd.output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "expected exit 0, stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let ledger = Ledger::open(&data_dir).unwrap();
    let sessions = ledger.sessions().unwrap();
    assert_eq!(sessions.len(), 1);
    let events = ledger.read_session(&sessions[0].0).unwrap();
    let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
    assert!(!kinds.contains(&"cli.version_warning"), "{kinds:?}");
    let end = events.iter().find(|e| e.kind == "session.end").expect("session.end");
    assert_eq!(end.payload["init_verified"], true);

    let _ = std::fs::remove_dir_all(&data_dir);
}
