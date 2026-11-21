// A scripted stand-in for a vendor CLI (stream-json output), for tests only.
// Compiled with plain `rustc` (no dependencies). It reads its instructions from files
// next to its own executable, so each test uses its own copy:
//   version.txt            printed for `--version` (default "2.1.318 (Claude Code)")
//   script.ndjson          lines printed to stdout in order; special lines:
//                            @approval   send approval_message.json (with @TOKEN@ replaced by
//                                        CORE_SESSION_TOKEN) to CORE_APPROVAL_ADDR from the
//                                        --mcp-config file, save the reply line to
//                                        approval_reply.txt, then ACK with ack.json
//                            @sleep MS   pause
//                            @bytes HEX  print these raw bytes (may be invalid UTF-8) as a line
//   exit_code.txt          exit code (default 0)
//   version_sleep_ms.txt   pause before answering `--version`
// Unknown options are rejected (exit 1), as the real CLI's parser does.
// It records what it was given:
//   argv.txt               every argument, one per line
//   stdin.txt              everything read from stdin (until EOF)
//   mcp_path.txt           the --mcp-config path
//   mcp_copy.json          the --mcp-config file's contents as read at start (test token only)
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn here() -> PathBuf {
    std::env::current_exe().unwrap().parent().unwrap().to_path_buf()
}

fn read(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(name)).ok()
}

/// Value of `"key": "value"` in a JSON text (no escapes in the values we look for).
fn json_str(text: &str, key: &str) -> Option<String> {
    let at = text.find(&format!("\"{key}\""))? + key.len() + 2;
    let rest = &text[at..];
    let colon = rest.find(':')?;
    let rest = &rest[colon + 1..];
    let open = rest.find('"')?;
    let rest = &rest[open + 1..];
    let close = rest.find('"')?;
    Some(rest[..close].to_string())
}

fn approval(dir: &Path, mcp: &str) {
    let addr = json_str(mcp, "CORE_APPROVAL_ADDR").expect("CORE_APPROVAL_ADDR in mcp config");
    let token = json_str(mcp, "CORE_SESSION_TOKEN").expect("CORE_SESSION_TOKEN in mcp config");
    let msg = read(dir, "approval_message.json").expect("approval_message.json").replace("@TOKEN@", &token);
    let mut stream = TcpStream::connect(addr).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(120))).unwrap();
    writeln!(stream, "{}", msg.trim()).unwrap();
    stream.flush().unwrap();
    let mut reply = String::new();
    BufReader::new(&mut stream).read_line(&mut reply).unwrap();
    std::fs::write(dir.join("approval_reply.txt"), &reply).unwrap();
    if let Some(ack) = read(dir, "ack.json") {
        writeln!(stream, "{}", ack.trim()).unwrap();
        stream.flush().unwrap();
    }
    std::thread::sleep(Duration::from_millis(100));
}

/// Options of `claude --help` 2.1.318 that re:Search may pass (see
/// fixtures/claude_help_2_1_318.txt): `true` when the option takes a value.
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
            Some((_, true)) => {
                eprintln!("error: option '{a}' argument missing");
                std::process::exit(1);
            }
            Some((_, false)) => i += 1,
            None => {
                eprintln!("error: unknown option '{a}'");
                std::process::exit(1);
            }
        }
    }
}

fn main() {
    let dir = here();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        if let Some(ms) = read(&dir, "version_sleep_ms.txt").and_then(|s| s.trim().parse().ok()) {
            std::thread::sleep(Duration::from_millis(ms));
        }
        let v = read(&dir, "version.txt").unwrap_or_else(|| "2.1.318 (Claude Code)".to_string());
        println!("{}", v.trim());
        return;
    }
    check_flags(&args);
    std::fs::write(dir.join("argv.txt"), args.join("\n")).unwrap();
    let mut input = String::new();
    let _ = io::stdin().read_to_string(&mut input);
    std::fs::write(dir.join("stdin.txt"), &input).unwrap();
    let mcp_path = args.iter().position(|a| a == "--mcp-config").and_then(|i| args.get(i + 1)).cloned();
    let mcp = match &mcp_path {
        Some(p) => {
            std::fs::write(dir.join("mcp_path.txt"), p).unwrap();
            let text = std::fs::read_to_string(p).unwrap_or_default();
            std::fs::write(dir.join("mcp_copy.json"), &text).unwrap();
            text
        }
        None => String::new(),
    };

    let script = read(&dir, "script.ndjson").unwrap_or_default();
    let mut out = io::stdout().lock();
    for line in script.lines() {
        if line.trim() == "@approval" {
            approval(&dir, &mcp);
        } else if let Some(ms) = line.trim().strip_prefix("@sleep ") {
            std::thread::sleep(Duration::from_millis(ms.trim().parse().unwrap_or(0)));
        } else if let Some(hex) = line.trim().strip_prefix("@bytes ") {
            let bytes: Vec<u8> = (0..hex.len() / 2).map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap()).collect();
            out.write_all(&bytes).unwrap();
            out.write_all(b"\n").unwrap();
            out.flush().unwrap();
        } else if !line.trim().is_empty() {
            writeln!(out, "{line}").unwrap();
            out.flush().unwrap();
        }
    }
    let code = read(&dir, "exit_code.txt").and_then(|c| c.trim().parse().ok()).unwrap_or(0);
    std::process::exit(code);
}
