use serde_json::Value;
use std::fs;
use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use std::time::Duration;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
use windows_sys::Win32::System::Pipes::{CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_WAIT};
use worker::appcontainer::delete_session_profile;
use worker::Worker;

pub fn python_exe() -> Option<String> {
    std::env::var("RE_SEARCH_PYTHON")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var("RESEARCH_CORE_PYTHON").ok().filter(|v| !v.is_empty()))
}

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub struct TempSandboxFixture {
    pub root: PathBuf,
    pub data_dir: PathBuf,
    pub external_canary: PathBuf,
    sessions: Vec<String>,
}

impl TempSandboxFixture {
    pub fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("worker-ac-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let data_dir = root.join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let external_canary = root.join("external-secret.txt");
        fs::write(&external_canary, b"external-canary").unwrap();
        fs::write(data_dir.join("ledger.db"), b"ledger-canary").unwrap();
        Self { root, data_dir, external_canary, sessions: Vec::new() }
    }

    pub fn prepare_session(&mut self, session: &str) -> (PathBuf, PathBuf) {
        let sdir = self.data_dir.join("sessions").join(session);
        let work = sdir.join("work");
        let scratch = sdir.join("scratch");
        let pycache = sdir.join("pycache");
        fs::create_dir_all(&work).unwrap();
        fs::create_dir_all(&scratch).unwrap();
        fs::create_dir_all(&pycache).unwrap();
        self.sessions.push(session.to_string());
        (scratch, pycache)
    }
}

impl Drop for TempSandboxFixture {
    fn drop(&mut self) {
        for s in &self.sessions {
            delete_session_profile(s);
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub struct NamedPipeProbe {
    pub pipe_name: String,
    handle: HANDLE,
}

impl NamedPipeProbe {
    pub fn create(tag: &str) -> Self {
        let pipe_name = format!(r"\\.\pipe\re-search-probe-{}-{tag}", std::process::id());
        let wide: Vec<u16> = pipe_name.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: `wide` is NUL-terminated UTF-16; handle ownership stays in `NamedPipeProbe`.
        let handle = unsafe {
            CreateNamedPipeW(
                wide.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_WAIT,
                1,
                1024,
                1024,
                0,
                null_mut(),
            )
        };
        assert!(!handle.is_null() && handle != INVALID_HANDLE_VALUE, "CreateNamedPipeW failed");
        Self { pipe_name, handle }
    }
}

impl Drop for NamedPipeProbe {
    fn drop(&mut self) {
        if !self.handle.is_null() && self.handle != INVALID_HANDLE_VALUE {
            // SAFETY: owned handle closed once.
            unsafe { CloseHandle(self.handle) };
            self.handle = null_mut();
        }
    }
}

pub fn discover_local_ipv4() -> Option<Ipv4Addr> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("192.0.2.1:53").ok()?;
    match sock.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

pub fn py_json(w: &mut Worker, code: &str, timeout: Duration) -> Value {
    let r = w.evaluate(code, timeout).unwrap();
    assert!(r.get("error").is_none(), "unexpected python error: {r}\ncode:\n{code}");
    let stdout = r["stdout"].as_str().unwrap_or("");
    serde_json::from_str(stdout.trim()).unwrap_or_else(|e| panic!("bad probe json ({e}): {stdout:?}"))
}

pub fn py_repr(p: &Path) -> String {
    serde_json::to_string(&p.to_string_lossy().as_ref()).unwrap()
}

pub fn send_udp_probe(w: &mut Worker, host: &str, port: u16, msg: &[u8]) -> Value {
    let payload = serde_json::to_string(std::str::from_utf8(msg).unwrap()).unwrap();
    let code = format!(
        r#"import json, socket
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
try:
    n = s.sendto({payload}.encode(), ({host:?}, {port}))
    print(json.dumps({{'sent': n, 'err': None}}))
except OSError as e:
    print(json.dumps({{'sent': 0, 'err': e.winerror or e.errno}}))
finally:
    s.close()"#
    );
    py_json(w, &code, Duration::from_secs(15))
}
