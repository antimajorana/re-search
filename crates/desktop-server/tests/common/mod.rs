//! Shared helpers for the desktop-server integration tests.
#![allow(dead_code)]

use desktop_server::DesktopConfig;
use research_tools::LiteratureEndpoints;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

pub struct MockReply {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

pub struct MockHttp {
    pub base_url: String,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockHttp {
    pub fn start(replies: Vec<MockReply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            for r in replies {
                let (mut stream, _) = listener.accept().unwrap();
                drain_http_request(&mut stream);
                let hdr = format!(
                    "HTTP/1.1 {} OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    r.status,
                    r.content_type,
                    r.body.len()
                );
                let _ = stream.write_all(hdr.as_bytes());
                let _ = stream.write_all(&r.body);
                let _ = stream.flush();
            }
        });
        Self { base_url: format!("http://{addr}"), handle: Some(handle) }
    }

    pub fn finish(mut self) {
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

pub fn drain_http_request(stream: &mut TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut reader = BufReader::new(stream);
    let mut content_len = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                content_len = v.trim().parse().unwrap_or(0);
            }
        }
    }
    if content_len > 0 {
        let mut buf = vec![0u8; content_len];
        let _ = reader.read_exact(&mut buf);
    }
}

pub fn raw_http(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nConnection: close\r\n");
    let mut has_host = false;
    for &(k, v) in headers {
        if k.eq_ignore_ascii_case("host") {
            has_host = true;
        }
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if !has_host {
        req.push_str(&format!("Host: 127.0.0.1:{port}\r\n"));
    }
    if !body.is_empty() && !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-length")) {
        req.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    req.push_str("\r\n");
    stream.write_all(req.as_bytes()).unwrap();
    if !body.is_empty() {
        stream.write_all(body).unwrap();
    }
    stream.flush().unwrap();

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&raw[..split]);
    let mut lines = head.lines();
    let status_line = lines.next().unwrap();
    let status: u16 = status_line.split_whitespace().nth(1).unwrap().parse().unwrap();
    let mut out_hdrs = Vec::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            out_hdrs.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    (status, out_hdrs, raw[split + 4..].to_vec())
}

pub fn test_cfg(data_dir: &Path, workspace: Option<PathBuf>, api_base: Option<String>, lit_base: Option<&str>) -> DesktopConfig {
    let endpoints = match lit_base {
        Some(u) => LiteratureEndpoints::loopback_mock(u).unwrap(),
        None => LiteratureEndpoints::default(),
    };
    DesktopConfig {
        port: 0,
        data_dir: data_dir.to_path_buf(),
        workspace_root: workspace,
        python: None,
        worker_script: None,
        api_base_override: api_base,
        literature_endpoints: endpoints,
        open_window: false,
        print_url_json: false,
        eval_timeout_s: 30,
        core_exe: None,
        claude_exe: None,
        agy_exe: None,
        // Hermetic: connection status and model lists never see this machine's keys or CLI.
        connection_env: Some(std::collections::BTreeMap::new()),
        chatgpt_test: None,
    }
}

/// Loads the launch URL like the browser does and returns the `name=value` stream cookie.
pub fn launch_cookie(port: u16, token: &str) -> String {
    let (st, hdrs, _) = raw_http(port, "GET", &format!("/?token={token}"), &[], b"");
    assert_eq!(st, 200);
    let set_cookie = hdrs.iter().find(|(k, _)| k == "set-cookie").map(|(_, v)| v.clone()).expect("Set-Cookie");
    set_cookie.split(';').next().unwrap().trim().to_string()
}
