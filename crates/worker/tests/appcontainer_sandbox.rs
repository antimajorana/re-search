//! Offline Control (`JobObjectSandbox`) vs Contained (`AppContainerSandbox`) probes.
//! Set `RE_SEARCH_PYTHON` (or `RESEARCH_CORE_PYTHON`) to a Python with SymPy + NumPy.
#![cfg(windows)]

mod support;

use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::net::{TcpListener, UdpSocket};
use std::path::{Path, PathBuf};
use std::time::Duration;
use support::{
    discover_local_ipv4, py_json, py_repr, python_exe, repo_root, send_udp_probe, NamedPipeProbe, TempSandboxFixture,
};
use worker::appcontainer::{delete_session_profile, profile_name_for_session, AppContainerSandbox};
use worker::sandbox::JobObjectSandbox;
use worker::stage::grant_icacls_inside;
use worker::win::{inspect_process_token, process_is_gone, Direction, PipePair};
use worker::{Worker, WorkerConfig, WorkerError, WorkerRole};

fn probe_token_and_job(ctrl: &Worker, cont: &Worker, expected_sid: &str) {
    let ctrl_tok = inspect_process_token(ctrl.worker_pid().unwrap()).unwrap();
    assert!(!ctrl_tok.is_appcontainer, "control should not be AppContainer: {ctrl_tok:?}");
    assert!(ctrl_tok.in_job, "control should be in Job: {ctrl_tok:?}");

    let cont_tok = inspect_process_token(cont.worker_pid().unwrap()).unwrap();
    assert!(cont_tok.is_appcontainer, "contained must be AppContainer: {cont_tok:?}");
    assert_eq!(cont_tok.integrity_sid, "S-1-16-4096", "{cont_tok:?}");
    assert_eq!(cont_tok.appcontainer_sid.as_deref(), Some(expected_sid), "{cont_tok:?}");
    assert!(cont_tok.in_job, "contained must be in Job: {cont_tok:?}");
}

fn probe_job_ui_restrictions(cont: &mut Worker) {
    let res = py_json(
        cont,
        r#"import ctypes, json
u32 = ctypes.WinDLL('user32', use_last_error=True)
u32.OpenClipboard.argtypes = [ctypes.c_void_p]
u32.OpenClipboard.restype = ctypes.c_int
ctypes.set_last_error(0)
opened = bool(u32.OpenClipboard(None))
err = ctypes.get_last_error()
if opened:
    u32.CloseClipboard()
print(json.dumps({'clipboard_opened': opened, 'winerror': err}))"#,
        Duration::from_secs(15),
    );
    assert_eq!(res["clipboard_opened"], json!(false), "worker Job UI limits must deny OpenClipboard: {res}");
}

fn probe_sympy_numpy_and_noise(w: &mut Worker, label: &str) {
    let r = w.evaluate("x = symbols('x')\nintegrate(x*sin(x), x)", Duration::from_secs(30)).unwrap();
    assert_eq!(r["value"], "-x*cos(x) + sin(x)", "{label}: {r}");

    let res = py_json(
        w,
        "import json, numpy as np\n\
         A = np.diag(np.arange(1.0, 65.0)) + 0.01\n\
         A = 0.5 * (A + A.T)\n\
         evals, vecs = np.linalg.eigh(A)\n\
         resid = float(np.max(np.abs(A @ vecs - vecs * evals)))\n\
         print(json.dumps({'resid': resid}))",
        Duration::from_secs(30),
    );
    let resid = res["resid"].as_f64().unwrap();
    assert!(resid < 1e-11, "{label} eigh residual too large: {resid}");

    let r = w
        .evaluate(
            r#"import sys
for i in range(2500):
    print(f'out-{i}')
    print(f'err-{i}', file=sys.stderr)
'noise-ok'"#,
            Duration::from_secs(30),
        )
        .unwrap();
    assert_eq!(r["value"], "'noise-ok'", "{label}: {r}");
    assert!(r.get("error").is_none(), "{label}: {r}");
    let r2 = w.evaluate("1 + 1", Duration::from_secs(10)).unwrap();
    assert_eq!(r2["value"], "2", "{label}: protocol corrupted after noise: {r2}");
}

fn probe_handle_non_inheritance(
    ctrl: &mut Worker,
    cont: &mut Worker,
    extra_pipe: PipePair,
    high_handle: usize,
) {
    // Part A: Child-side `GetHandleInformation` on `high_handle` (above fresh CPython's low handle slots,
    // queried before `numpy`/OpenBLAS allocates hundreds of internal handles). UCRT's `open_osfhandle` is
    // avoided because its invalid-parameter handler aborts (`0xC0000417`), and AppContainer strict-handle
    // checks raise SEH `0xC0000008` (`STATUS_INVALID_HANDLE`), surfaced by `ctypes` as `OSError`.
    let code = format!(
        r#"import ctypes, ctypes.wintypes as W, json, msvcrt
k32 = ctypes.WinDLL('kernel32', use_last_error=True)
k32.GetHandleInformation.argtypes = [W.HANDLE, ctypes.POINTER(W.DWORD)]
k32.GetHandleInformation.restype = W.BOOL
flags = W.DWORD(0)
self_ok = bool(k32.GetHandleInformation(msvcrt.get_osfhandle(0), ctypes.byref(flags)))
ctypes.set_last_error(0)
try:
    extra_ok = bool(k32.GetHandleInformation(W.HANDLE({high_handle}), ctypes.byref(flags)))
    err_info = ctypes.get_last_error()
except OSError as e:
    extra_ok = False
    err_info = e.winerror
print(json.dumps({{'self_ok': self_ok, 'ok': extra_ok, 'winerror': err_info}}))"#
    );
    for (label, w) in [("control", &mut *ctrl), ("contained", &mut *cont)] {
        let res = py_json(w, &code, Duration::from_secs(15));
        assert_eq!(res["self_ok"], json!(true), "{label} sanity handle check failed: {res}");
        assert_eq!(res["ok"], json!(false), "{label} inherited extra handle {high_handle}: {res}");
        let code_num = res["winerror"].as_i64().unwrap_or(0);
        assert!(
            code_num == 6 || (code_num as u32) == 0xC000_0008,
            "{label} unexpected winerror for uninherited handle: {res}"
        );
    }

    // Part B: Kernel reference-count proof. Close the parent's write end while BOTH workers remain alive;
    // `read` on the parent's read end returns `Ok(0)` (EOF / `ERROR_BROKEN_PIPE`) immediately iff zero
    // child processes inherited the write handle.
    let mut reader = extra_pipe.into_ours();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = [0u8; 16];
        let _ = tx.send(reader.read(&mut buf));
    });
    let eof = rx
        .recv_timeout(Duration::from_secs(2))
        .expect("extra_pipe read blocked: inheritable write handle leaked into a live worker!");
    assert_eq!(eof.unwrap(), 0, "unexpected bytes on uninherited extra_pipe");
}

fn probe_udp_egress(ctrl: &mut Worker, cont: &mut Worker) {
    // 4a. Loopback UDP sink
    let lb = UdpSocket::bind("127.0.0.1:0").unwrap();
    lb.set_read_timeout(Some(Duration::from_millis(400))).unwrap();
    let lb_port = lb.local_addr().unwrap().port();

    let c_res = send_udp_probe(ctrl, "127.0.0.1", lb_port, b"udp-loopback");
    assert_eq!(c_res["sent"], 12, "control loopback sendto failed: {c_res}");
    let mut buf = [0u8; 64];
    let (n, _) = lb.recv_from(&mut buf).expect("control loopback packet not received");
    assert_eq!(&buf[..n], b"udp-loopback");

    let _ = send_udp_probe(cont, "127.0.0.1", lb_port, b"udp-loopback");
    let err = lb.recv_from(&mut buf).expect_err("contained loopback UDP leaked packet!");
    assert!(matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut), "{err}");

    // 4b. Non-loopback local NIC UDP sink
    if let Some(ip) = discover_local_ipv4() {
        let lan = UdpSocket::bind((ip, 0)).unwrap();
        lan.set_read_timeout(Some(Duration::from_millis(400))).unwrap();
        let lan_port = lan.local_addr().unwrap().port();
        let ip_str = ip.to_string();

        let c_res = send_udp_probe(ctrl, &ip_str, lan_port, b"udp-lan");
        assert_eq!(c_res["sent"], 7, "control LAN sendto failed: {c_res}");
        let (n, _) = lan.recv_from(&mut buf).expect("control LAN packet not received");
        assert_eq!(&buf[..n], b"udp-lan");

        let _ = send_udp_probe(cont, &ip_str, lan_port, b"udp-lan");
        let err = lan.recv_from(&mut buf).expect_err("contained LAN UDP leaked packet!");
        assert!(matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut), "{err}");
    } else {
        eprintln!("SKIPPED Probe 4b: no non-loopback local IPv4 interface");
    }

    // 4c. DNS & TEST-NET-1 UDP
    let dns = py_json(
        cont,
        r#"import json, socket
try:
    socket.getaddrinfo('example.invalid', 443)
    print(json.dumps({'dns_failed': False}))
except OSError:
    print(json.dumps({'dns_failed': True}))"#,
        Duration::from_secs(15),
    );
    assert_eq!(dns["dns_failed"], json!(true));
    let _ = send_udp_probe(cont, "192.0.2.1", 53, b"test-net");
    assert_eq!(cont.evaluate("1 + 1", Duration::from_secs(10)).unwrap()["value"], "2");
}

fn probe_tcp_and_named_pipe(ctrl: &mut Worker, cont: &mut Worker) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();

    let tcp_code = format!(
        r#"import json, socket
try:
    with socket.create_connection(('127.0.0.1', {port}), timeout=0.6) as s:
        print(json.dumps({{'connected': True, 'err': None}}))
except OSError as e:
    print(json.dumps({{'connected': False, 'err': e.winerror or e.errno or type(e).__name__}}))"#
    );
    assert_eq!(py_json(ctrl, &tcp_code, Duration::from_secs(15))["connected"], json!(true));
    let _ = listener.accept().expect("control TCP connection should arrive");

    let cont_tcp = py_json(cont, &tcp_code, Duration::from_secs(15));
    assert_eq!(cont_tcp["connected"], json!(false), "contained TCP unexpectedly connected: {cont_tcp}");
    assert!(
        matches!(listener.accept(), Err(ref e) if e.kind() == ErrorKind::WouldBlock),
        "contained TCP produced an accept!"
    );

    let pipe_ctrl = NamedPipeProbe::create("ctrl");
    let ctrl_lit = serde_json::to_string(&pipe_ctrl.pipe_name).unwrap();
    let ctrl_pipe_code = format!(
        r#"import json
try:
    with open({ctrl_lit}, 'rb'):
        print(json.dumps({{'opened': True}}))
except OSError as e:
    print(json.dumps({{'opened': False, 'type': type(e).__name__, 'errno': e.errno, 'winerror': e.winerror}}))"#
    );
    assert_eq!(py_json(ctrl, &ctrl_pipe_code, Duration::from_secs(15))["opened"], json!(true));
    drop(pipe_ctrl);

    let pipe_cont = NamedPipeProbe::create("cont");
    let cont_lit = serde_json::to_string(&pipe_cont.pipe_name).unwrap();
    let cont_pipe_code = format!(
        r#"import json
try:
    with open({cont_lit}, 'rb'):
        print(json.dumps({{'opened': True}}))
except OSError as e:
    print(json.dumps({{'opened': False, 'type': type(e).__name__, 'errno': e.errno, 'winerror': e.winerror}}))"#
    );
    let cont_pipe = py_json(cont, &cont_pipe_code, Duration::from_secs(15));
    assert_eq!(cont_pipe["opened"], json!(false), "{cont_pipe}");
    assert!(
        matches!(cont_pipe["type"].as_str(), Some("PermissionError" | "FileNotFoundError")),
        "unexpected named-pipe error in AppContainer: {cont_pipe}"
    );
}

fn probe_filesystem_isolation(
    fx: &mut TempSandboxFixture,
    cont: &mut Worker,
    python: &Path,
    script: &Path,
    staged_py: &Path,
    staged_site: &Path,
    alpha_scratch: &Path,
) {
    let mcp_json = fx.data_dir.join("sessions/s-alpha/mcp.json");
    fs::write(&mcp_json, br#"{"secret_token":"top-secret"}"#).unwrap();

    let (beta_scratch, beta_pycache) = fx.prepare_session("s-beta");
    let _beta_sb = AppContainerSandbox::new(
        fx.data_dir.clone(),
        "s-beta",
        python.to_path_buf(),
        script.to_path_buf(),
        beta_scratch.clone(),
        beta_pycache,
    )
    .unwrap();
    let beta_secret = beta_scratch.join("beta-secret.txt");
    fs::write(&beta_secret, b"beta-secret").unwrap();

    // Host guardrail assertions
    assert!(AppContainerSandbox::new(
        fx.data_dir.clone(),
        "s-alpha",
        python.to_path_buf(),
        script.to_path_buf(),
        fx.data_dir.join("scratch"),
        fx.data_dir.join("sessions/s-alpha/pycache"),
    )
    .is_err());
    assert!(AppContainerSandbox::new(
        fx.data_dir.clone(),
        "s-alpha",
        python.to_path_buf(),
        script.to_path_buf(),
        fx.data_dir.join("sessions/s-alpha/work"),
        fx.data_dir.join("sessions/s-alpha/pycache"),
    )
    .is_err());
    assert!(grant_icacls_inside(&fx.data_dir, &fx.external_canary, "*S-1-15-2-1:(OI)(CI)RX").is_err());

    let code = format!(
        r#"import json
def try_read(p):
    try:
        with open(p, 'rb') as f: return {{'ok': True, 'data': f.read().decode()}}
    except OSError as e:
        return {{'ok': False, 'type': type(e).__name__, 'errno': e.errno, 'winerror': e.winerror}}
def try_write(p):
    try:
        with open(p, 'wb') as f: f.write(b'x')
        return {{'ok': True}}
    except OSError as e:
        return {{'ok': False, 'type': type(e).__name__, 'errno': e.errno, 'winerror': e.winerror}}
ok_file = {ok_file}
with open(ok_file, 'w', encoding='utf-8') as f: f.write('alpha-ok')
with open(ok_file, 'r', encoding='utf-8') as f: ok_back = f.read()
print(json.dumps({{
    'ext': try_read({ext}),
    'ext_verbatim': try_read({ext_verbatim}),
    'mcp': try_read({mcp}),
    'ledger': try_read({ledger}),
    'beta': try_read({beta}),
    'pwn_py': try_write({pwn_py}),
    'pwn_site': try_write({pwn_site}),
    'ok_back': ok_back,
}}))"#,
        ok_file = py_repr(&alpha_scratch.join("ok.txt")),
        ext = py_repr(&fx.external_canary),
        ext_verbatim = py_repr(&fs::canonicalize(&fx.external_canary).unwrap()),
        mcp = py_repr(&mcp_json),
        ledger = py_repr(&fx.data_dir.join("ledger.db")),
        beta = py_repr(&beta_secret),
        pwn_py = py_repr(&staged_py.join("pwn.txt")),
        pwn_site = py_repr(&staged_site.join("pwn.txt")),
    );
    let res = py_json(cont, &code, Duration::from_secs(15));
    for key in ["ext", "ext_verbatim", "mcp", "ledger", "beta", "pwn_py", "pwn_site"] {
        assert_eq!(res[key]["ok"], json!(false), "{key} unexpectedly succeeded: {res}");
        assert_eq!(res[key]["type"], "PermissionError", "{key}: {res}");
        assert_eq!(res[key]["errno"], 13, "{key}: {res}");
    }
    assert_eq!(res["ok_back"], "alpha-ok");
}

#[test]
fn control_vs_contained_probes() {
    let Some(python) = python_exe() else {
        eprintln!("SKIPPED: set RE_SEARCH_PYTHON to run AppContainer probes");
        return;
    };
    let python_path = PathBuf::from(python);
    let script_path = repo_root().join("python/worker.py");
    let mut fx = TempSandboxFixture::new("matrix");

    let (ctrl_scratch, ctrl_pycache) = fx.prepare_session("s-ctrl");
    let (alpha_scratch, alpha_pycache) = fx.prepare_session("s-alpha");
    let alpha_sb = AppContainerSandbox::new(
        fx.data_dir.clone(),
        "s-alpha",
        python_path.clone(),
        script_path.clone(),
        alpha_scratch.clone(),
        alpha_pycache.clone(),
    )
    .unwrap();
    let alpha_sid = alpha_sb.profile_sid().to_string();
    let staged_py = alpha_sb.staged().root.join("py");
    let staged_site = alpha_sb.staged().site_dir.clone();

    // Pad low handle slots in the parent so `high_pipe`'s inheritable handle value sits well above
    // fresh CPython's ~60 startup handles, and keep `extra_pipe` open across both spawns for the
    // kernel pipe-EOF refcount proof.
    let pad_pipes: Vec<PipePair> = (0..1024).map(|_| PipePair::new(Direction::FromChild).unwrap()).collect();
    let extra_pipe = PipePair::new(Direction::FromChild).unwrap();
    let high_pipe = PipePair::new(Direction::FromChild).unwrap();
    let high_handle = high_pipe.their_handle_value();

    let mut ctrl = Worker::new(
        WorkerConfig {
            python: python_path.clone(),
            script: script_path.clone(),
            scratch: ctrl_scratch,
            pycache: ctrl_pycache,
            startup_timeout: Duration::from_secs(60),
        },
        Box::new(JobObjectSandbox),
    );
    let mut cont = Worker::new(
        WorkerConfig {
            python: python_path.clone(),
            script: script_path.clone(),
            scratch: alpha_scratch.clone(),
            pycache: alpha_pycache,
            startup_timeout: Duration::from_secs(60),
        },
        Box::new(alpha_sb),
    );

    // Spawn both workers via a minimal `1 + 1` call and run Probe 3 BEFORE `numpy`/OpenBLAS allocates
    // hundreds of child handles, while keeping `extra_pipe`/`high_pipe` alive across spawn.
    assert_eq!(ctrl.evaluate("1 + 1", Duration::from_secs(30)).unwrap()["value"], "2");
    assert_eq!(cont.evaluate("1 + 1", Duration::from_secs(30)).unwrap()["value"], "2");
    probe_handle_non_inheritance(&mut ctrl, &mut cont, extra_pipe, high_handle);
    drop(high_pipe);
    drop(pad_pipes);

    // Probe 2: SymPy, NumPy `eigh`, and 2500-line stdout/stderr noise.
    probe_sympy_numpy_and_noise(&mut ctrl, "control");
    probe_sympy_numpy_and_noise(&mut cont, "contained");
    // Probe 1
    probe_token_and_job(&ctrl, &cont, &alpha_sid);
    probe_job_ui_restrictions(&mut cont);
    // Probe 4
    probe_udp_egress(&mut ctrl, &mut cont);
    // Probe 5
    probe_tcp_and_named_pipe(&mut ctrl, &mut cont);
    // Probe 6
    probe_filesystem_isolation(&mut fx, &mut cont, &python_path, &script_path, &staged_py, &staged_site, &alpha_scratch);

    // Probe 7: Process-tree kill & profile cleanup
    let ctrl_pid = ctrl.worker_pid().unwrap();
    drop(ctrl);
    assert!(process_is_gone(ctrl_pid, 5000));

    let gc = py_json(
        &mut cont,
        "import json, subprocess, sys\n\
         p = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'])\n\
         print(json.dumps({'pid': p.pid}))",
        Duration::from_secs(15),
    );
    let grandchild_pid = gc["pid"].as_u64().unwrap() as u32;
    let worker1_pid = cont.worker_pid().unwrap();
    let err = cont.evaluate("while True: pass", Duration::from_secs(1)).unwrap_err();
    assert!(matches!(err, WorkerError::Timeout(_)), "{err}");
    assert!(process_is_gone(worker1_pid, 5000), "worker1 survived timeout");
    assert!(process_is_gone(grandchild_pid, 5000), "grandchild survived timeout");

    assert_eq!(cont.evaluate("1 + 1", Duration::from_secs(60)).unwrap()["value"], "2");
    let worker2_pid = cont.worker_pid().unwrap();
    drop(cont);
    assert!(process_is_gone(worker2_pid, 5000), "worker2 survived drop");
    delete_session_profile("s-alpha");
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let pkg = PathBuf::from(local).join("Packages").join(profile_name_for_session("s-alpha"));
        assert!(!pkg.exists(), "AppContainer package dir survived cleanup: {}", pkg.display());
    }
}

#[test]
fn contained_check_worker_loads_staged_modules_and_reports_its_role() {
    let Some(python) = python_exe() else {
        eprintln!("SKIPPED: set RE_SEARCH_PYTHON to run AppContainer probes");
        return;
    };
    let mut fixture = TempSandboxFixture::new("check-worker");
    let (scratch, pycache) = fixture.prepare_session("s-check-worker");
    let python = PathBuf::from(python);
    let script = repo_root().join("python/worker.py");
    let sandbox = AppContainerSandbox::new(
        fixture.data_dir.clone(), "s-check-worker", python.clone(), script.clone(),
        scratch.clone(), pycache.clone(),
    ).unwrap();
    assert!(sandbox.staged().root.join("checks/srepr_safe.py").is_file());
    let config = WorkerConfig { python, script, scratch, pycache, startup_timeout: Duration::from_secs(60) };
    let mut worker = Worker::new_with_role(config, Box::new(sandbox), WorkerRole::Check);
    let handles = BTreeMap::from([
        ("lhs".into(), "Integer(2)".into()),
        ("rhs".into(), "Integer(2)".into()),
    ]);
    let (receipt, _) = worker.run_check("symbolic_zero_smoke", &handles, json!({}), Duration::from_secs(30), None).unwrap();
    assert_eq!(worker.negotiated_protocol(), Some(2));
    assert_eq!(worker.hello().unwrap()["role"], "check");
    assert_eq!(receipt["process"], "check");
    assert_eq!(receipt["outcome"], "pass");
    assert_eq!(receipt["evidence"]["residual_srepr"], "Integer(0)");
    assert!(inspect_process_token(worker.worker_pid().unwrap()).unwrap().is_appcontainer);
    assert!(matches!(worker.evaluate("1 + 1", Duration::from_secs(5)), Err(WorkerError::Protocol(_))));
}
