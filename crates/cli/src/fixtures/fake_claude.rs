use std::io::{self, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--sleep-child") {
        std::thread::sleep(Duration::from_secs(60));
        return;
    }
    if args.iter().any(|a| a == "--version") {
        let ver = std::env::var("FAKE_CLAUDE_VERSION").unwrap_or_else(|_| "2.1.318 (Claude Code)".into());
        println!("{ver}");
        return;
    }

    if let Ok(pid_file) = std::env::var("FAKE_CLAUDE_PID_FILE") {
        if !pid_file.is_empty() {
            let exe = std::env::current_exe().unwrap();
            let child = Command::new(exe)
                .arg("--sleep-child")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            std::fs::write(pid_file, child.id().to_string()).unwrap();
        }
    }

    let mut out = io::stdout().lock();
    if let Ok(init_line) = std::env::var("FAKE_CLAUDE_INIT_LINE") {
        writeln!(out, "{init_line}").unwrap();
        out.flush().unwrap();
    }
    if let Ok(extra_line) = std::env::var("FAKE_CLAUDE_EXTRA_LINE") {
        if !extra_line.is_empty() {
            writeln!(out, "{extra_line}").unwrap();
            out.flush().unwrap();
        }
    }
    if let Ok(sleep_ms) = std::env::var("FAKE_CLAUDE_SLEEP_MS") {
        if let Ok(ms) = sleep_ms.parse::<u64>() {
            std::thread::sleep(Duration::from_millis(ms));
        }
    }
}
