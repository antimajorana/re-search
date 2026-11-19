use super::*;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cli-approval-test-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn sample_request(session: &str, scope: &str) -> Value {
    json!({
        "schema_version": tools::APPROVAL_SCHEMA_VERSION,
        "session": session,
        "call_id": "call-1",
        "digest": "d".repeat(64),
        "caller": format!("claude-code-cli:{session}"),
        "tool_id": "sympy_eval",
        "scope": scope,
        "effect": "execute-code",
        "canonical_args": r#"{"code":"1+1"}"#,
        "policy_reason": "matched rule",
    })
}

fn tcp_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let client = TcpStream::connect(addr).unwrap();
    let (server, _) = listener.accept().unwrap();
    (client, server)
}

fn kinds(ledger: &Ledger, session: &str) -> Vec<String> {
    ledger.read_session(session).unwrap().into_iter().map(|e| e.kind).collect()
}

#[test]
fn peer_disconnect_mid_prompt_records_expired_only() {
    let dir = temp_dir("expired");
    let ledger = Ledger::open(&dir).unwrap();
    let (mut client, server) = tcp_pair();
    let msg = json!({
        "v": APPROVAL_CHANNEL_VERSION,
        "token": "tok",
        "request": sample_request("s1", "worker-session"),
        "containment": "none",
    });
    writeln!(client, "{msg}").unwrap();
    client.flush().unwrap();

    // Never send a terminal answer; drop the client socket after 100 ms.
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        drop(client);
    });
    let (_tx, rx) = mpsc::channel();
    handle_stream(
        server,
        "tok",
        "s1",
        &ledger,
        TerminalInput::Test { rx, anti_buffer: Duration::ZERO },
    )
    .unwrap();

    assert_eq!(kinds(&ledger, "s1"), ["approval.expired"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn acked_approval_records_answered_and_delivered() {
    let dir = temp_dir("acked");
    let ledger = Ledger::open(&dir).unwrap();
    let (mut client, server) = tcp_pair();
    let msg = json!({
        "v": APPROVAL_CHANNEL_VERSION,
        "token": "tok",
        "request": sample_request("s1", "worker-session"),
        "containment": "none",
    });
    writeln!(client, "{msg}").unwrap();
    client.flush().unwrap();

    let (tx, rx) = mpsc::channel();
    tx.send(StdinEvent::Line("y\n".into())).unwrap();

    let handle_thread = std::thread::spawn(move || {
        handle_stream(
            server,
            "tok",
            "s1",
            &ledger,
            TerminalInput::Test { rx, anti_buffer: Duration::ZERO },
        )
        .unwrap();
    });

    let mut reply_line = String::new();
    BufReader::new(&mut client).read_line(&mut reply_line).unwrap();
    let reply: Value = serde_json::from_str(&reply_line).unwrap();
    assert_eq!(reply["v"], APPROVAL_CHANNEL_VERSION);
    assert_eq!(reply["approved"], true);
    writeln!(client, "{}", json!({ "v": APPROVAL_CHANNEL_VERSION, "ack": true })).unwrap();
    client.flush().unwrap();
    handle_thread.join().unwrap();

    let ledger = Ledger::open(&dir).unwrap();
    let events = ledger.read_session("s1").unwrap();
    assert_eq!(events.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(), ["approval.answered", "approval.delivered"]);
    assert_eq!(events[0].payload["approved"], true);
    assert_eq!(events[0].payload["scope"], "worker-session");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn disconnect_before_ack_records_answered_and_undelivered() {
    let dir = temp_dir("noack");
    let ledger = Ledger::open(&dir).unwrap();
    let (mut client, server) = tcp_pair();
    let msg = json!({
        "v": APPROVAL_CHANNEL_VERSION,
        "token": "tok",
        "request": sample_request("s1", "once"),
        "containment": "none",
    });
    writeln!(client, "{msg}").unwrap();
    client.flush().unwrap();

    let (tx, rx) = mpsc::channel();
    tx.send(StdinEvent::Line("y\n".into())).unwrap();

    let handle_thread = std::thread::spawn(move || {
        handle_stream(
            server,
            "tok",
            "s1",
            &ledger,
            TerminalInput::Test { rx, anti_buffer: Duration::ZERO },
        )
        .unwrap();
    });

    let mut reply_line = String::new();
    BufReader::new(&mut client).read_line(&mut reply_line).unwrap();
    // Drop without sending ACK.
    drop(client);
    handle_thread.join().unwrap();

    let ledger = Ledger::open(&dir).unwrap();
    assert_eq!(kinds(&ledger, "s1"), ["approval.answered", "approval.undelivered"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_token_session_version_or_scope_is_denied() {
    let cases = [
        json!({ "v": 99, "token": "tok", "request": sample_request("s1", "once") }),
        json!({ "v": APPROVAL_CHANNEL_VERSION, "token": "wrong", "request": sample_request("s1", "once") }),
        json!({ "v": APPROVAL_CHANNEL_VERSION, "token": "tok", "request": sample_request("other", "once") }),
        json!({ "v": APPROVAL_CHANNEL_VERSION, "token": "tok", "request": sample_request("s1", "bogus") }),
    ];
    for (idx, msg) in cases.into_iter().enumerate() {
        let dir = temp_dir(&format!("deny-{idx}"));
        let ledger = Ledger::open(&dir).unwrap();
        let (mut client, server) = tcp_pair();
        writeln!(client, "{msg}").unwrap();
        client.flush().unwrap();

        let (_tx, rx) = mpsc::channel();
        let h = std::thread::spawn(move || {
            handle_stream(
                server,
                "tok",
                "s1",
                &ledger,
                TerminalInput::Test { rx, anti_buffer: Duration::ZERO },
            )
            .unwrap();
        });

        let mut reply_line = String::new();
        BufReader::new(&mut client).read_line(&mut reply_line).unwrap();
        let reply: Value = serde_json::from_str(&reply_line).unwrap();
        assert_eq!(reply["approved"], false, "case {idx}");
        writeln!(client, "{}", json!({ "v": APPROVAL_CHANNEL_VERSION, "ack": true })).unwrap();
        client.flush().unwrap();
        h.join().unwrap();

        let ledger = Ledger::open(&dir).unwrap();
        assert_eq!(kinds(&ledger, "s1"), ["approval.answered", "approval.delivered"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn after_stdin_eof_subsequent_prompt_denies_immediately() {
    let dir = temp_dir("eof");
    let ledger = Ledger::open(&dir).unwrap();
    let (mut client, server) = tcp_pair();
    let msg = json!({
        "v": APPROVAL_CHANNEL_VERSION,
        "token": "tok",
        "request": sample_request("s1", "once"),
        "containment": "none",
    });
    writeln!(client, "{msg}").unwrap();
    client.flush().unwrap();

    let hub = std::sync::Arc::new(StdinHub::new());
    hub.mark_eof();
    let hub_thread = hub.clone();
    let start = Instant::now();
    let h = std::thread::spawn(move || {
        handle_stream(server, "tok", "s1", &ledger, TerminalInput::Hub(hub_thread)).unwrap();
    });

    let mut reply_line = String::new();
    BufReader::new(&mut client).read_line(&mut reply_line).unwrap();
    let reply: Value = serde_json::from_str(&reply_line).unwrap();
    assert_eq!(reply["approved"], false);
    assert_eq!(reply["by"], "supervisor: terminal EOF");
    assert!(start.elapsed() < Duration::from_secs(2), "prompt waited instead of denying at EOF");
    writeln!(client, "{}", json!({ "v": APPROVAL_CHANNEL_VERSION, "ack": true })).unwrap();
    client.flush().unwrap();
    h.join().unwrap();

    let ledger = Ledger::open(&dir).unwrap();
    assert_eq!(kinds(&ledger, "s1"), ["approval.answered", "approval.delivered"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn banner_escapes_ansi_and_control_sequences_in_code() {
    let mut req = sample_request("s1", "once");
    req["canonical_args"] = json!({
        "code": "a\t=\t1\n\x1b[31mred\x1b[0m\rFAKE PROMPT\x08\u{009b}2J"
    })
    .to_string()
    .into();
    let banner = format_banner(&req, "backend\x1b[0m", "once");
    assert!(!banner.contains('\x1b'), "raw ESC leaked into banner: {banner:?}");
    assert!(!banner.contains('\r'), "raw CR leaked into banner: {banner:?}");
    assert!(!banner.contains('\x08'), "raw BS leaked into banner: {banner:?}");
    assert!(!banner.contains('\u{009b}'), "raw CSI leaked into banner: {banner:?}");
    assert!(banner.contains("    | a\t=\t1\n"), "tabs should be preserved: {banner:?}");
    assert!(banner.contains("\\u{1b}[31mred\\u{1b}[0m\\rFAKE PROMPT\\u{8}\\u{9b}2J"));
    assert!(banner.contains("(backend\\u{1b}[0m)"));
}

#[test]
fn uncontained_worker_session_request_is_denied_by_supervisor() {
    let dir = temp_dir("uncontained-ws");
    let ledger = Ledger::open(&dir).unwrap();
    let (mut client, server) = tcp_pair();
    let msg = json!({
        "v": APPROVAL_CHANNEL_VERSION,
        "token": "tok",
        "request": sample_request("s1", "worker-session"),
        "containment": "job-object",
        "contained": false,
    });
    writeln!(client, "{msg}").unwrap();
    client.flush().unwrap();

    let (tx, rx) = mpsc::channel();
    tx.send(StdinEvent::Line("y\n".into())).unwrap();
    let h = std::thread::spawn(move || {
        handle_stream(
            server,
            "tok",
            "s1",
            &ledger,
            TerminalInput::Test { rx, anti_buffer: Duration::ZERO },
        )
        .unwrap();
    });

    let mut reply_line = String::new();
    BufReader::new(&mut client).read_line(&mut reply_line).unwrap();
    let reply: Value = serde_json::from_str(&reply_line).unwrap();
    assert_eq!(reply["approved"], false);
    assert_eq!(reply["by"], "supervisor: worker-session requires a contained sandbox");
    writeln!(client, "{}", json!({ "v": APPROVAL_CHANNEL_VERSION, "ack": true })).unwrap();
    client.flush().unwrap();
    h.join().unwrap();

    let ledger = Ledger::open(&dir).unwrap();
    let events = ledger.read_session("s1").unwrap();
    assert_eq!(events.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(), ["approval.answered", "approval.delivered"]);
    assert_eq!(events[0].payload["approved"], false);
    assert_eq!(events[0].payload["scope"], "worker-session");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn contained_banner_states_contained_limits_and_escapes_controls() {
    let mut req = sample_request("s1", "worker-session");
    req["canonical_args"] = json!({
        "code": "1+1\x1b[31m\r\x08\u{009b}"
    })
    .to_string()
    .into();
    let banner = format_banner_with_containment(&req, "appcontainer\x1b[0m", true, "worker-session");
    assert!(banner.contains("CONTAINED (appcontainer):"), "{banner}");
    assert!(!banner.contains("NOT CONTAINED:"), "{banner}");
    assert!(!banner.contains('\x1b'), "raw ESC leaked: {banner:?}");
    assert!(!banner.contains('\r'), "raw CR leaked: {banner:?}");
    assert!(!banner.contains('\x08'), "raw BS leaked: {banner:?}");
    assert!(!banner.contains('\u{009b}'), "raw CSI leaked: {banner:?}");
    assert!(banner.contains("(appcontainer\\u{1b}[0m)"), "{banner}");
}
