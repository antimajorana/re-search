//! End-to-end Explore (`explore/1`) MCP parity & approval-relay tests (package X18 CP2).
//!
//! Spawns the real compiled `core` binary over stdio JSON-RPC MCP with
//! `CORE_TOOL_PROFILE=explore/1` (even when `CORE_PYTHON`/`CORE_WORKER_SCRIPT` are set) and
//! verifies:
//! 1. MCP `initialize` instructions name only `explore/1` tools (zero `sympy_eval`, zero
//!    `ledger_note`, zero `search_formal_library`, zero `"verified"`).
//! 2. MCP `tools/list` serves strictly the 5 `explore/1.mcp_defs()` tools.
//! 3. `read_paper_lines` and `search_paper_passages` run over MCP and credit lines to
//!    `claude-code-cli:<session>` in F4 `lines_sent_with_ledger`.
//! 4. `idea_search` fails closed (`DENIED`) without approval and succeeds through a real
//!    loopback `TcpApprover` <-> `cli_route::relay::start_for_caller` + loopback mock arXiv.

use cli_route::relay::{start_for_caller, RelayAsk, RelayDecider, RelayDecision};
use ledger::Ledger;
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::lines_sent::lines_sent_with_ledger;
use research_tools::tool_profile::{explore_v1, PROFILE_EXPLORE_V1};
use research_tools::ResearchEnv;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!(
            "research-x18-mcp-{tag}-{}-{}",
            std::process::id(),
            tools::canon::random_hex(4)
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
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

struct McpClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl McpClient {
    fn spawn(envs: &[(&str, &str)]) -> Self {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_core"));
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().expect("spawn core MCP server");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self { child, stdin, stdout, next_id: 1 }
    }

    fn send(&mut self, msg: Value) {
        writeln!(self.stdin, "{msg}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn recv_for_id(&mut self, want_id: u64) -> Value {
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).unwrap();
            assert!(n > 0, "MCP server closed stdout unexpectedly");
            let v: Value = serde_json::from_str(line.trim()).unwrap();
            if v.get("id").and_then(Value::as_u64) == Some(want_id) {
                return v;
            }
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        self.recv_for_id(id)
    }

    fn initialize(&mut self) -> Value {
        let resp = self.call(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "x18-test", "version": "0.1" }
            }),
        );
        self.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        resp
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct ApproveAllRelay;
impl RelayDecider for ApproveAllRelay {
    fn decide(&self, _ask: &RelayAsk<'_>) -> RelayDecision {
        RelayDecision::Answered { approved: true, by: "test-human".into() }
    }
}

fn spawn_mock_arxiv() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut buf = [0u8; 2048];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let first = req.lines().next().unwrap_or("");
            let (ctype, body) = if first.contains("/e-print/") {
                (
                    "application/x-tex",
                    "\\section{Fetched}\nSynthetic fetched line two.\nSynthetic fetched line three.\n",
                )
            } else {
                (
                    "application/atom+xml",
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <id>http://arxiv.org/abs/2501.07777v1</id>
    <title>Synthetic Krylov Paper from MCP Test</title>
    <summary>Lanczos growth scales linearly in the volume-law phase.</summary>
    <published>2025-01-12T00:00:00Z</published>
    <author><name>T. Author</name></author>
  </entry>
</feed>"#,
                )
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    });
    format!("http://127.0.0.1:{port}")
}

#[test]
fn explore_v1_mcp_serves_exact_tools_clean_instructions_reads_papers_and_gates_network() {
    let tmp = TempDir::new("explore-v1");
    let data = tmp.path().join("data");
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&ws).unwrap();
    let sid = "s-mcp-explore";

    let env = ResearchEnv::new(&data);
    let ledger = Ledger::open(&data).unwrap();
    ledger.append_with_hash(sid, "idea.started", 1, &json!({})).unwrap();
    let ledger_mux = Mutex::new(Ledger::open(&data).unwrap());
    let tex = "\\section{Main Result}\n\
               Theorem 1: Lanczos coefficients grow linearly in the volume-law phase.\n\
               Line three filler.\n\
               Line four filler.\n\
               Line five filler.\n\
               Line six filler.\n\
               Line seven antigravity-only target.\n\
               Line eight filler.\n\
               Line nine filler.\n\
               Line ten filler.\n\
               Line eleven filler.\n\
               Line twelve filler.\n\
               Line thirteen filler.\n\
               Remark 2: Distinct unique passage marker alpha-beta-gamma.\n";
    let receipt = ingest_bytes(
        &env.sandbox(),
        &ledger_mux,
        &IngestRequest {
            session_id: sid,
            file_name: "paper.tex",
            origin: "upload",
            raw_bytes: tex.as_bytes(),
            source_id_hint: Some("src-exp1"),
            section_filter: None,
            title: Some("Synthetic Explore Paper"),
        },
    )
    .unwrap();
    drop(ledger_mux);

    // 1. Spawn real `core` binary with CORE_TOOL_PROFILE=explore/1 EVEN WITH
    //    CORE_ENABLE_RESEARCH_TOOLS=1 + CORE_WORKSPACE + bogus CORE_PYTHON/CORE_WORKER_SCRIPT
    //    (proves explore/1 isolates tools strictly and never starts a Python worker).
    let mut client = McpClient::spawn(&[
        ("CORE_SESSION", sid),
        ("CORE_DATA_DIR", &data.to_string_lossy()),
        ("CORE_TOOL_PROFILE", PROFILE_EXPLORE_V1),
        ("CORE_ENABLE_RESEARCH_TOOLS", "1"),
        ("CORE_WORKSPACE", &ws.to_string_lossy()),
        ("CORE_PYTHON", "/nonexistent/python"),
        ("CORE_WORKER_SCRIPT", "/nonexistent/worker.py"),
    ]);

    let init = client.initialize();
    let instructions = init["result"]["instructions"].as_str().unwrap_or("");
    assert!(instructions.contains("explore/1"), "missing explore/1 in: {instructions}");
    assert!(instructions.contains("idea_search"), "missing idea_search in: {instructions}");
    for forbidden in ["sympy_eval", "ledger_note", "search_formal_library", "verified"] {
        assert!(
            !instructions.to_ascii_lowercase().contains(forbidden),
            "Explore MCP instructions leaked `{forbidden}`: {instructions}"
        );
    }

    let list = client.call("tools/list", json!({}));
    let mut names: Vec<String> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    names.sort();
    let mut expected: Vec<String> = explore_v1().mcp_defs().into_iter().map(|d| d.id).collect();
    expected.sort();
    assert_eq!(names, expected, "explore/1 MCP tool list mismatch");
    for forbidden in ["sympy_eval", "fs_write", "lean_check", "search_formal_library"] {
        assert!(!names.iter().any(|n| n == forbidden));
    }

    // 2. Call both `read_paper_lines` (lines 1-2) and `search_paper_passages` (line 4) over MCP.
    let primary_path = receipt.primary_path.as_deref().unwrap();
    let read_resp = client.call(
        "tools/call",
        json!({
            "name": "read_paper_lines",
            "arguments": {
                "source_id": receipt.source_id,
                "path": primary_path,
                "start_line": 1,
                "end_line": 2
            }
        }),
    );
    assert_ne!(read_resp["result"]["isError"], json!(true), "{read_resp}");

    let pass_resp = client.call(
        "tools/call",
        json!({
            "name": "search_paper_passages",
            "arguments": {
                "query": "alpha-beta-gamma",
                "source_id": receipt.source_id
            }
        }),
    );
    assert_ne!(pass_resp["result"]["isError"], json!(true), "{pass_resp}");

    // 3. Without an approval relay, BOTH `idea_search` and `idea_fetch_papers` MUST fail closed (DENIED).
    let denied_search = client.call(
        "tools/call",
        json!({
            "name": "idea_search",
            "arguments": {
                "queries": [{ "qid": "q-1", "provider": "arxiv", "q": "Krylov complexity" }]
            }
        }),
    );
    assert_eq!(denied_search["result"]["isError"], json!(true), "{denied_search}");
    assert!(denied_search["result"]["content"][0]["text"].as_str().unwrap_or("").contains("DENIED"));

    let denied_fetch = client.call(
        "tools/call",
        json!({
            "name": "idea_fetch_papers",
            "arguments": { "keys": ["arxiv:2501.07777"] }
        }),
    );
    assert_eq!(denied_fetch["result"]["isError"], json!(true), "{denied_fetch}");
    assert!(denied_fetch["result"]["content"][0]["text"].as_str().unwrap_or("").contains("DENIED"));
    drop(client);

    let events = ledger.read_session(sid).unwrap();
    let sha = events
        .iter()
        .find(|e| e.kind == "source.ingested")
        .unwrap()
        .payload["files_sha256"][primary_path]
        .as_str()
        .unwrap()
        .to_string();
    let sent = lines_sent_with_ledger(&events, Some(&ledger), None);
    let claude_caller = format!("claude-code-cli:{sid}");
    let agy_caller = format!("antigravity:{sid}");
    assert!(sent.covers(&claude_caller, &receipt.source_id, primary_path, &sha, 1, 2));
    assert!(sent.covers(&claude_caller, &receipt.source_id, primary_path, &sha, 14, 14));
    assert!(
        !sent.covers(&agy_caller, &receipt.source_id, primary_path, &sha, 1, 2),
        "claude-code-cli reads must not credit antigravity caller"
    );

    // 4. With real loopback `cli_route::relay` approval + mock arXiv + `CORE_CALLER_ROUTE=antigravity`,
    //    both `idea_search` and `idea_fetch_papers` succeed and credit `antigravity:<session>`.
    let mock_arxiv = spawn_mock_arxiv();
    let token = tools::canon::random_hex(16);
    let relay = start_for_caller(
        Ledger::open(&data).unwrap(),
        sid.to_string(),
        token.clone(),
        agy_caller.clone(),
        Arc::new(ApproveAllRelay),
    )
    .unwrap();
    let mut agy_client = McpClient::spawn(&[
        ("CORE_SESSION", sid),
        ("CORE_DATA_DIR", &data.to_string_lossy()),
        ("CORE_TOOL_PROFILE", PROFILE_EXPLORE_V1),
        ("CORE_CALLER_ROUTE", "antigravity"),
        ("CORE_APPROVAL_ADDR", &relay.addr),
        ("CORE_SESSION_TOKEN", &token),
        ("RE_SEARCH_ARXIV_BASE", &mock_arxiv),
    ]);
    let _ = agy_client.initialize();
    let ok_search = agy_client.call(
        "tools/call",
        json!({
            "name": "idea_search",
            "arguments": {
                "queries": [{ "qid": "q-ok", "provider": "arxiv", "q": "Krylov complexity" }]
            }
        }),
    );
    assert_ne!(ok_search["result"]["isError"], json!(true), "{ok_search}");
    assert!(ok_search["result"]["content"][0]["text"].as_str().unwrap_or("").contains("q-ok"));

    let ok_fetch = agy_client.call(
        "tools/call",
        json!({
            "name": "idea_fetch_papers",
            "arguments": { "keys": ["arxiv:2501.07777"] }
        }),
    );
    assert_ne!(ok_fetch["result"]["isError"], json!(true), "{ok_fetch}");
    assert!(ok_fetch["result"]["content"][0]["text"].as_str().unwrap_or("").contains("2501.07777"));

    let agy_read = agy_client.call(
        "tools/call",
        json!({
            "name": "read_paper_lines",
            "arguments": {
                "source_id": receipt.source_id,
                "path": primary_path,
                "start_line": 7,
                "end_line": 7
            }
        }),
    );
    assert_ne!(agy_read["result"]["isError"], json!(true), "{agy_read}");
    drop(agy_client);
    drop(relay);

    let post_events = ledger.read_session(sid).unwrap();
    let post_sent = lines_sent_with_ledger(&post_events, Some(&ledger), None);
    assert!(post_sent.covers(&agy_caller, &receipt.source_id, primary_path, &sha, 7, 7));
    assert!(!post_sent.covers(&claude_caller, &receipt.source_id, primary_path, &sha, 7, 7));
}

#[test]
fn explore_session_without_sources_or_profile_env_still_resolves_explore_v1() {
    let tmp = TempDir::new("explore-nosources");
    let data = tmp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let sid = "s-mcp-explore-empty";
    let ledger = Ledger::open(&data).unwrap();
    ledger.append_with_hash(sid, "idea.started", 1, &json!({})).unwrap();

    // No CORE_TOOL_PROFILE, no sources/, bogus CORE_PYTHON: must resolve explore/1 (never sympy_eval).
    let mut client = McpClient::spawn(&[
        ("CORE_SESSION", sid),
        ("CORE_DATA_DIR", &data.to_string_lossy()),
        ("CORE_PYTHON", "/nonexistent/python"),
        ("CORE_WORKER_SCRIPT", "/nonexistent/worker.py"),
    ]);
    let init = client.initialize();
    let instructions = init["result"]["instructions"].as_str().unwrap_or("");
    assert!(instructions.contains("explore/1"), "{instructions}");
    let list = client.call("tools/list", json!({}));
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(!names.contains(&"sympy_eval"), "leaked sympy_eval: {names:?}");
    assert!(names.contains(&"idea_search"), "missing idea_search: {names:?}");
}
