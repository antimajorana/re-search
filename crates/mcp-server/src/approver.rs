//! Approval over a loopback TCP connection to the supervisor.
//!
//! This process is a child of the agent CLI: its stdin/stdout carry MCP and
//! it has no console of its own. So it asks the supervisor (the `cli`
//! process the user started), which prompts on the user's terminal. Any
//! failure of the channel counts as "not approved".

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;
use tools::{ApprovalReply, ApprovalRequest, Approver, APPROVAL_CHANNEL_VERSION};

pub struct TcpApprover {
    pub addr: String,
    pub token: String,
    /// What the worker sandbox does and does not contain, shown to the user.
    pub containment: String,
    /// Whether the worker backend restricts both filesystem and network access.
    pub contained: bool,
    pub wait: Duration,
}

impl TcpApprover {
    fn ask(&self, request: &ApprovalRequest) -> Result<ApprovalReply, String> {
        let mut stream = TcpStream::connect(&self.addr).map_err(|e| format!("connect: {e}"))?;
        stream.set_read_timeout(Some(self.wait)).map_err(|e| e.to_string())?;
        stream.set_write_timeout(Some(Duration::from_secs(5))).map_err(|e| e.to_string())?;
        let message = json!({
            "v": APPROVAL_CHANNEL_VERSION,
            "token": self.token,
            "request": request,
            "containment": self.containment,
            "contained": self.contained,
        });
        stream.write_all(format!("{message}\n").as_bytes()).map_err(|e| format!("send: {e}"))?;
        stream.flush().map_err(|e| format!("flush request: {e}"))?;

        let mut line = String::new();
        let n = {
            let mut reader = BufReader::new((&mut stream).take(65_536));
            reader.read_line(&mut line).map_err(|e| format!("no answer: {e}"))?
        };
        if n == 0 {
            return Err("approval channel closed before answer".into());
        }
        let reply: Value = serde_json::from_str(&line).map_err(|e| format!("bad answer: {e}"))?;
        if reply["v"] != json!(APPROVAL_CHANNEL_VERSION) {
            return Err("answer has an unknown version".into());
        }

        let ack = json!({ "v": APPROVAL_CHANNEL_VERSION, "ack": true });
        stream.write_all(format!("{ack}\n").as_bytes()).map_err(|e| format!("ack write: {e}"))?;
        stream.flush().map_err(|e| format!("ack flush: {e}"))?;

        Ok(ApprovalReply {
            approved: reply["approved"] == json!(true),
            digest: reply["digest"].as_str().unwrap_or_default().to_string(),
            by: reply["by"].as_str().unwrap_or("supervisor").to_string(),
        })
    }
}

impl Approver for TcpApprover {
    fn decide(&self, request: &ApprovalRequest) -> ApprovalReply {
        self.ask(request).unwrap_or_else(|e| {
            eprintln!("core: approval channel failed: {e}");
            ApprovalReply { approved: false, digest: String::new(), by: format!("approval channel error: {e}") }
        })
    }
}

/// Used when no supervisor address was given: nothing can be approved.
pub struct DenyAll;

impl Approver for DenyAll {
    fn decide(&self, _request: &ApprovalRequest) -> ApprovalReply {
        ApprovalReply { approved: false, digest: String::new(), by: "no approval channel configured".into() }
    }
}
