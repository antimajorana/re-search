//! Cross-platform compiled multi-attempt fake Claude & Antigravity CLIs for X18 tests.
//!
//! Compiles a tiny dependency-free Rust executable via `rustc` once per test process so
//! multi-attempt repair tests (attempt 1 -> attempt 2 -> attempt 3) run identically on Linux
//! and Windows without shell scripts or live vendor CLIs.

#![allow(dead_code)]

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

static MULTI_CLI_BIN: OnceLock<Result<PathBuf, String>> = OnceLock::new();

const MULTI_CLI_SRC: &str = r#"
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

fn here() -> PathBuf {
    std::env::current_exe().unwrap().parent().unwrap().to_path_buf()
}

fn read(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(name)).ok()
}

fn main() {
    let dir = here();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        let v = read(&dir, "version.txt").unwrap_or_else(|| "2.1.318 (Claude Code)".into());
        println!("{}", v.trim());
        return;
    }
    let idx: usize = read(&dir, "counter.txt")
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
        + 1;
    std::fs::write(dir.join("counter.txt"), idx.to_string()).unwrap();
    std::fs::write(dir.join(format!("argv_{idx}.txt")), args.join("\n")).unwrap();
    let mut stdin_buf = String::new();
    let _ = io::stdin().read_to_string(&mut stdin_buf);
    std::fs::write(dir.join(format!("stdin_{idx}.txt")), &stdin_buf).unwrap();

    if let Some(pos) = args.iter().position(|a| a == "--mcp-config") {
        if let Some(path) = args.get(pos + 1) {
            let txt = std::fs::read_to_string(path).unwrap_or_default();
            std::fs::write(dir.join(format!("mcp_{idx}.json")), txt).unwrap();
        }
    }

    let script = read(&dir, &format!("attempt_{idx}.ndjson"))
        .or_else(|| read(&dir, "script.ndjson"))
        .unwrap_or_default();
    let mut out = io::stdout().lock();
    for line in script.lines() {
        if !line.trim().is_empty() {
            writeln!(out, "{line}").unwrap();
            out.flush().unwrap();
        }
    }
}
"#;

fn compile_multi_cli() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("research-x18-multicli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let src = dir.join("multi_cli.rs");
    let exe = dir.join(format!("multi_cli{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&src, MULTI_CLI_SRC).map_err(|e| e.to_string())?;
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let out = Command::new(rustc)
        .args(["--edition", "2021"])
        .arg(&src)
        .arg("-o")
        .arg(&exe)
        .output()
        .map_err(|e| format!("failed to run rustc: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "rustc multi_cli failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(exe)
}

pub struct MultiAttemptCli {
    pub dir: PathBuf,
    pub exe: PathBuf,
}

impl MultiAttemptCli {
    pub fn new(parent: &Path, name: &str, version: &str) -> Self {
        let compiled = MULTI_CLI_BIN
            .get_or_init(compile_multi_cli)
            .clone()
            .expect("compile multi_cli");
        let dir = parent.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(compiled.file_name().unwrap());
        std::fs::copy(&compiled, &exe).unwrap();
        std::fs::write(dir.join("version.txt"), version).unwrap();
        Self { dir, exe }
    }

    pub fn set_attempt(&self, n: usize, lines: &[String]) {
        std::fs::write(self.dir.join(format!("attempt_{n}.ndjson")), lines.join("\n")).unwrap();
    }

    pub fn attempts_run(&self) -> usize {
        std::fs::read_to_string(self.dir.join("counter.txt"))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    }

    pub fn stdin_of(&self, n: usize) -> Option<String> {
        std::fs::read_to_string(self.dir.join(format!("stdin_{n}.txt"))).ok()
    }

    pub fn argv_of(&self, n: usize) -> Option<Vec<String>> {
        std::fs::read_to_string(self.dir.join(format!("argv_{n}.txt")))
            .ok()
            .map(|s| s.lines().map(str::to_string).collect())
    }

    pub fn mcp_of(&self, n: usize) -> Option<String> {
        std::fs::read_to_string(self.dir.join(format!("mcp_{n}.json"))).ok()
    }
}

pub fn claude_exchange_lines(text: &str) -> Vec<String> {
    vec![
        json!({
            "type": "system",
            "subtype": "init",
            "tools": [],
            "mcp_servers": [],
            "model": "claude-sonnet-4-6",
            "claude_code_version": "2.1.318",
            "apiKeySource": "none",
            "session_id": "cli-ex-1"
        })
        .to_string(),
        json!({
            "type": "assistant",
            "message": { "content": [{ "type": "text", "text": text }] }
        })
        .to_string(),
        json!({
            "type": "result",
            "subtype": "success",
            "is_error": false,
            "result": text,
            "num_turns": 1,
            "usage": { "input_tokens": 111, "output_tokens": 52 }
        })
        .to_string(),
    ]
}

pub fn agy_exchange_lines(text: &str) -> Vec<String> {
    vec![
        json!({
            "event": "init",
            "conversation_id": "agy-ex-1",
            "init": {
                "permission_mode": "request-review",
                "tools": ["read_file", "run_command"],
                "model": "gemini-3.8-flash-medium",
                "cwd": "."
            }
        })
        .to_string(),
        json!({
            "event": "step_update",
            "step_update": {
                "step_index": 0,
                "step_type": "agent_response",
                "state": "DONE",
                "text_delta": text
            }
        })
        .to_string(),
        json!({
            "event": "result",
            "result": {
                "status": "SUCCESS",
                "response": text,
                "num_turns": 1,
                "usage": { "input_tokens": 95, "output_tokens": 44 }
            }
        })
        .to_string(),
    ]
}

pub fn fenced_json(val: &Value) -> String {
    format!("```json\n{}\n```", serde_json::to_string_pretty(val).unwrap())
}
