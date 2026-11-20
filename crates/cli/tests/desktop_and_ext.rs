//! Integration tests for Stage 6-8 CLI subcommands (`fork`, `diff`, `export`,
//! `ui --print-url-json true --no-open true`, `desktop`) and `--route api --workspace`
//! research execution + ledger verification.

use ledger::Ledger;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "cli-desktop-ext-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_re-search")
}

struct ScriptedMock {
    base_url: String,
    handle: Option<thread::JoinHandle<()>>,
}

impl ScriptedMock {
    fn start(replies: Vec<Value>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            for val in replies {
                let (mut stream, _) = listener.accept().unwrap();
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut reader = BufReader::new(&stream);
                let mut cl = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" || line == "\n" {
                        break;
                    }
                    if let Some((k, v)) = line.split_once(':') {
                        if k.eq_ignore_ascii_case("content-length") {
                            cl = v.trim().parse().unwrap_or(0);
                        }
                    }
                }
                if cl > 0 {
                    let mut body = vec![0u8; cl];
                    let _ = reader.read_exact(&mut body);
                }
                let raw = serde_json::to_vec(&val).unwrap();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    raw.len()
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.write_all(&raw);
                let _ = stream.flush();
            }
        });
        Self { base_url: format!("http://{addr}"), handle: Some(handle) }
    }

    fn finish(mut self) {
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[test]
fn cli_research_workspace_read_fork_diff_export_and_verify() {
    let tmp = TempDir::new("ext");
    let data = tmp.path().join("data");
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("theorem.tex"), "\\begin{lemma}\nx = 1\n\\end{lemma}\n").unwrap();

    // Turn 1: fs_read (once approval? Wait - ReadWorkspace is Allowed silently under research_policy!)
    // Turn 2: ledger_append + final text!
    let turn1 = json!({
        "id": "m1",
        "type": "message",
        "role": "assistant",
        "model": "claude-haiku-4-5",
        "stop_reason": "tool_use",
        "content": [
            { "type": "tool_use", "id": "tu_r", "name": "fs_read", "input": { "path": "theorem.tex" } }
        ],
        "usage": { "input_tokens": 10, "output_tokens": 10 }
    });
    let turn2 = json!({
        "id": "m2",
        "type": "message",
        "role": "assistant",
        "model": "claude-haiku-4-5",
        "stop_reason": "tool_use",
        "content": [
            {
                "type": "tool_use",
                "id": "tu_l",
                "name": "ledger_append",
                "input": {
                    "entry_type": "lemma",
                    "summary": "Read theorem.tex lemma",
                    "latex": "x = 1",
                    "depends_on": []
                }
            }
        ],
        "usage": { "input_tokens": 12, "output_tokens": 10 }
    });
    let turn3 = json!({
        "id": "m3",
        "type": "message",
        "role": "assistant",
        "model": "claude-haiku-4-5",
        "stop_reason": "end_turn",
        "content": [{ "type": "text", "text": "Recorded lemma from theorem.tex: $x = 1$." }],
        "usage": { "input_tokens": 14, "output_tokens": 12 }
    });
    let mock = ScriptedMock::start(vec![turn1, turn2, turn3]);

    let out = Command::new(bin())
        .args([
            "run",
            "--prompt",
            "Read theorem.tex and record lemma",
            "--route",
            "api",
            "--provider",
            "anthropic",
            "--api-base",
            &mock.base_url,
            "--workspace",
            ws.to_str().unwrap(),
            "--data-dir",
            data.to_str().unwrap(),
        ])
        .env("RE_SEARCH_API_KEY", "sk-ant-cli-ext-test")
        .output()
        .unwrap();
    mock.finish();
    assert_eq!(out.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&out.stderr));

    let stdout_str = String::from_utf8(out.stdout).unwrap();
    let first_ev: Value = serde_json::from_str(stdout_str.lines().next().unwrap()).unwrap();
    let sid = first_ev["session"].as_str().unwrap().to_string();
    assert_eq!(first_ev["data"]["policy_version"], "v0.3-research");

    // Verify original session passes.
    let v_out = Command::new(bin())
        .args(["verify", &sid, "--data-dir", data.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(v_out.status.code(), Some(0));

    // Fork at seq 2 with modified assumption.
    let f_out = Command::new(bin())
        .args([
            "fork",
            &sid,
            "--at-seq",
            "2",
            "--assumption",
            "Assume x = 2",
            "--data-dir",
            data.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(f_out.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&f_out.stderr));
    let receipt: Value = serde_json::from_slice(&f_out.stdout).unwrap();
    let forked_id = receipt["new_session"].as_str().unwrap().to_string();

    // Diff parent and fork.
    let d_out = Command::new(bin())
        .args(["diff", &sid, &forked_id, "--data-dir", data.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(d_out.status.code(), Some(0));
    let diff_v: Value = serde_json::from_slice(&d_out.stdout).unwrap();
    assert_eq!(diff_v["base_session"], sid);
    assert_eq!(diff_v["target_session"], forked_id);

    // Export tex, html, zip.
    for (fmt, name) in [("tex", "out.tex"), ("html", "out.html"), ("zip", "out.zip")] {
        let out_path = tmp.path().join(name);
        let e_out = Command::new(bin())
            .args([
                "export",
                &sid,
                "--format",
                fmt,
                "--out",
                out_path.to_str().unwrap(),
                "--data-dir",
                data.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert_eq!(e_out.status.code(), Some(0), "export {fmt} failed: {}", String::from_utf8_lossy(&e_out.stderr));
        assert!(std::fs::metadata(&out_path).unwrap().len() > 20);
    }

    let ledger = Ledger::open_existing(&data).unwrap();
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
}

#[test]
fn cli_ui_print_url_json_serves_health_and_exits_cleanly() {
    let tmp = TempDir::new("ui");
    let data = tmp.path().join("data");

    let mut child = Command::new(bin())
        .args([
            "ui",
            "--port",
            "0",
            "--data-dir",
            data.to_str().unwrap(),
            "--no-open",
            "true",
            "--print-url-json",
            "true",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut first_line = String::new();
    reader.read_line(&mut first_line).unwrap();
    let launch: Value = serde_json::from_str(&first_line).expect("expected launch JSON on stdout");
    let port = launch["port"].as_u64().unwrap() as u16;
    let tok = launch["token"].as_str().unwrap();
    assert!(launch["url"].as_str().unwrap().contains(&format!("token={tok}")));

    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let req = format!("GET /api/health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).unwrap();
    let mut resp = String::new();
    stream.read_to_string(&mut resp).unwrap();
    assert!(resp.contains("200 OK") && resp.contains("\"ok\":true"));

    let _ = child.kill();
    let _ = child.wait();
}
