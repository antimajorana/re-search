//! The supervisor's side of the approval channel: a loopback listener that
//! the `core` MCP server connects to, and the console prompt.
//!
//! There is no way to approve without a person typing at a terminal: if
//! stdin is not a terminal, or the token is wrong, the answer is "no".

use ledger::Ledger;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, IsTerminal, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;

const CHANNEL_VERSION: u32 = 1;

pub struct ApprovalServer {
    pub addr: String,
}

/// Bind 127.0.0.1 on a free port and answer approval requests on a
/// background thread, one at a time.
pub fn start(data_dir: PathBuf, session: String, token: String) -> std::io::Result<ApprovalServer> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?.to_string();
    std::thread::spawn(move || {
        let ledger = Ledger::open(&data_dir).ok();
        for stream in listener.incoming().flatten() {
            if let Err(e) = handle(stream, &token, &session, ledger.as_ref()) {
                eprintln!("[supervisor] approval request failed: {e}");
            }
        }
    });
    Ok(ApprovalServer { addr })
}

fn handle(stream: TcpStream, token: &str, session: &str, ledger: Option<&Ledger>) -> Result<(), String> {
    let mut line = String::new();
    BufReader::new(stream.try_clone().map_err(|e| e.to_string())?)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    let message: Value = serde_json::from_str(&line).map_err(|e| format!("bad request: {e}"))?;
    let request = &message["request"];
    let digest = request["digest"].as_str().unwrap_or_default().to_string();

    let (approved, by) = if message["v"] != json!(CHANNEL_VERSION) {
        (false, "supervisor: unknown channel version")
    } else if message["token"].as_str() != Some(token) {
        (false, "supervisor: wrong session token")
    } else if request["session"].as_str() != Some(session) {
        (false, "supervisor: request is for another session")
    } else if !std::io::stdin().is_terminal() {
        eprintln!("[supervisor] approval requested but stdin is not a terminal: answering NO");
        (false, "supervisor: no terminal to ask on")
    } else {
        (prompt(request, message["containment"].as_str().unwrap_or("unknown")), "console")
    };

    if let Some(ledger) = ledger {
        let payload = json!({
            "call_id": request["call_id"], "digest": digest, "approved": approved, "by": by,
        });
        let _ = ledger.append(session, "approval.answered", 1, &payload);
    }
    let reply = json!({ "v": CHANNEL_VERSION, "approved": approved, "digest": digest, "by": by });
    (&stream).write_all(format!("{reply}\n").as_bytes()).map_err(|e| e.to_string())
}

/// Show the request on stderr and read y/N from the terminal.
fn prompt(request: &Value, containment: &str) -> bool {
    let args = request["canonical_args"].as_str().unwrap_or("");
    let parsed: Value = serde_json::from_str(args).unwrap_or(Value::Null);
    let mut err = std::io::stderr().lock();
    let _ = writeln!(err, "\n================ APPROVAL NEEDED ================");
    let _ = writeln!(err, "tool:    {}   (effect class: {})", request["tool_id"], request["effect"]);
    let _ = writeln!(err, "caller:  {}", request["caller"]);
    match parsed["code"].as_str() {
        Some(code) => {
            let _ = writeln!(err, "code to run:");
            for l in code.lines() {
                let _ = writeln!(err, "    | {l}");
            }
        }
        None => {
            let _ = writeln!(err, "arguments: {args}");
        }
    }
    let _ = writeln!(err, "NOT CONTAINED: this runs as you. It can read and write your files and use the network.");
    let _ = writeln!(err, "({containment})");
    let _ = writeln!(err, "digest:  {}", request["digest"].as_str().unwrap_or(""));
    let _ = write!(err, "Run exactly this, once? [y/N]: ");
    let _ = err.flush();
    drop(err);
    let mut answer = String::new();
    if std::io::stdin().lock().read_line(&mut answer).is_err() {
        return false;
    }
    let yes = matches!(answer.trim(), "y" | "Y" | "yes" | "Yes" | "YES");
    eprintln!("[supervisor] {}", if yes { "approved" } else { "declined" });
    yes
}
