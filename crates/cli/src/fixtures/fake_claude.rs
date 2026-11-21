use std::io::{self, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Options of `claude --help` 2.1.318 that re:Search may pass (see
/// crates/cli-route/fixtures/claude_help_2_1_318.txt): `true` when the option takes a value.
const KNOWN_FLAGS: &[(&str, bool)] = &[
    ("-p", false),
    ("--print", false),
    ("--model", true),
    ("--output-format", true),
    ("--verbose", false),
    ("--tools", true),
    ("--strict-mcp-config", false),
    ("--mcp-config", true),
    ("--setting-sources", true),
    ("--disable-slash-commands", false),
    ("--no-session-persistence", false),
    ("--permission-prompts", true),
    ("--allowedTools", true),
    ("--allowed-tools", true),
];

/// Like the real CLI's option parser: an unknown option is an error (exit 1), so a
/// mistyped flag fails the tests instead of being ignored.
fn check_flags(args: &[String]) {
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        match KNOWN_FLAGS.iter().find(|(name, _)| name == a) {
            Some((_, true)) if i + 1 < args.len() => i += 2,
            Some((_, false)) => i += 1,
            _ => {
                eprintln!("error: unknown option or missing argument '{a}'");
                std::process::exit(1);
            }
        }
    }
}

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
    check_flags(&args);

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
