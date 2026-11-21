// A scripted stand-in for the Antigravity CLI (`agy --input-format stream-json
// --output-format stream-json`), for
// tests only. Compiled with plain `rustc` (no dependencies). It reads its instructions from
// files next to its own executable, so each test uses its own copy:
//   version.txt            printed for `--version` (default "1.5.14")
//   models.txt             printed for `models` (default two models)
//   script.ndjson          lines printed to stdout in order; special lines:
//                            @approval   send approval_message.json (with @TOKEN@ replaced by
//                                        CORE_SESSION_TOKEN) to CORE_APPROVAL_ADDR taken from
//                                        <cwd>/.agents/mcp_config.json, save the reply line to
//                                        approval_reply.txt, then ACK with ack.json
//                            @sleep MS   pause
//   exit_code.txt          exit code (default 0)
// Unknown flags are rejected (exit 2, as Go's flag package does), and so are a stray
// positional argument and `-p`/`--print`/`--prompt` (the prompt must come on stdin; the real
// CLI drops a flag prompt in stream input mode). `--input-format stream-json` without
// `--output-format stream-json`, or stdin that is not exactly one `user` event line, exits 1
// (as the Headless page documents for invalid stream input).
// It records what it was given:
//   argv.txt               every argument, one per line
//   stdin.txt              everything read from stdin (until EOF)
//   cwd.txt                its working directory
//   mcp_path.txt           <cwd>/.agents/mcp_config.json
//   mcp_copy.json          that file's contents as read at start (test token only)
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
    let addr = json_str(mcp, "CORE_APPROVAL_ADDR").expect("CORE_APPROVAL_ADDR in mcp_config.json");
    let token = json_str(mcp, "CORE_SESSION_TOKEN").expect("CORE_SESSION_TOKEN in mcp_config.json");
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

/// Flags of `agy --help` 1.5.14 that re:Search may pass: `true` when the flag takes a value.
const KNOWN_FLAGS: &[(&str, bool)] = &[
    ("--input-format", true),
    ("--output-format", true),
    ("--model", true),
    ("--sandbox", false),
    ("--disable-slash-commands", false),
    ("--settings", true),
    ("--mode", true),
    ("--print-timeout", true),
];

fn check_flags(args: &[String]) {
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if matches!(a.as_str(), "-p" | "--print" | "--prompt") {
            eprintln!("re:Search must not pass a prompt flag: {a}");
            std::process::exit(2);
        }
        match KNOWN_FLAGS.iter().find(|(name, _)| name == a) {
            Some((_, true)) if i + 1 < args.len() => i += 2,
            Some((_, true)) => {
                eprintln!("flag needs an argument: {a}");
                std::process::exit(2);
            }
            Some((_, false)) => i += 1,
            None if a.starts_with('-') => {
                eprintln!("flag provided but not defined: {a}");
                std::process::exit(2);
            }
            None => {
                eprintln!("unexpected argument: {a}");
                std::process::exit(2);
            }
        }
    }
}

fn main() {
    let dir = here();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        println!("{}", read(&dir, "version.txt").unwrap_or_else(|| "1.5.14".to_string()).trim());
        return;
    }
    if args.first().map(String::as_str) == Some("models") {
        eprintln!("Fetching available models...");
        let text = read(&dir, "models.txt").unwrap_or_else(|| "fake-model-a\tFake Model A\nfake-model-b\tFake Model B\n".into());
        print!("{text}");
        return;
    }
    check_flags(&args);
    std::fs::write(dir.join("argv.txt"), args.join("\n")).unwrap();
    let mut input = String::new();
    let _ = io::stdin().read_to_string(&mut input);
    std::fs::write(dir.join("stdin.txt"), &input).unwrap();
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str);
    if flag("--input-format") != Some("stream-json") || flag("--output-format") != Some("stream-json") {
        eprintln!("expected --input-format stream-json with --output-format stream-json");
        std::process::exit(1);
    }
    let lines: Vec<&str> = input.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() != 1 || !lines[0].starts_with("{\"event\":\"user\",\"message\":{\"content\":") || !input.ends_with('\n') {
        eprintln!("stdin is not exactly one stream-json user event line");
        std::process::exit(1);
    }
    let cwd = std::env::current_dir().unwrap();
    std::fs::write(dir.join("cwd.txt"), cwd.to_string_lossy().as_bytes()).unwrap();
    let mcp_path = cwd.join(".agents").join("mcp_config.json");
    std::fs::write(dir.join("mcp_path.txt"), mcp_path.to_string_lossy().as_bytes()).unwrap();
    let mcp = std::fs::read_to_string(&mcp_path).unwrap_or_default();
    std::fs::write(dir.join("mcp_copy.json"), &mcp).unwrap();

    let script = read(&dir, "script.ndjson").unwrap_or_default();
    let mut out = io::stdout().lock();
    for line in script.lines() {
        if line.trim() == "@approval" {
            approval(&dir, &mcp);
        } else if let Some(ms) = line.trim().strip_prefix("@sleep ") {
            std::thread::sleep(Duration::from_millis(ms.trim().parse().unwrap_or(0)));
        } else if !line.trim().is_empty() {
            writeln!(out, "{line}").unwrap();
            out.flush().unwrap();
        }
    }
    let code = read(&dir, "exit_code.txt").and_then(|c| c.trim().parse().ok()).unwrap_or(0);
    std::process::exit(code);
}
