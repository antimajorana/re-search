//! Windows Job Object memory-cap integration tests (`JOB_OBJECT_LIMIT_JOB_MEMORY`).
//! Set `RE_SEARCH_PYTHON` (or `RESEARCH_CORE_PYTHON`) to a Python with SymPy.
#![cfg(windows)]

#[allow(dead_code)]
mod support;

use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;
use support::{py_json, python_exe, repo_root, TempSandboxFixture};
use worker::appcontainer::AppContainerSandbox;
use worker::sandbox::JobObjectSandbox;
use worker::win::process_is_gone;
use worker::{Worker, WorkerConfig, DEFAULT_JOB_MEMORY_LIMIT_BYTES};

const QUERY_JOB_MEMORY_PY: &str = r#"import ctypes, ctypes.wintypes as W, json
class IO_COUNTERS(ctypes.Structure):
    _fields_ = [
        ('ReadOperationCount', ctypes.c_uint64),
        ('WriteOperationCount', ctypes.c_uint64),
        ('OtherOperationCount', ctypes.c_uint64),
        ('ReadTransferCount', ctypes.c_uint64),
        ('WriteTransferCount', ctypes.c_uint64),
        ('OtherTransferCount', ctypes.c_uint64),
    ]
class JOBOBJECT_BASIC_LIMIT_INFORMATION(ctypes.Structure):
    _fields_ = [
        ('PerProcessUserTimeLimit', ctypes.c_int64),
        ('PerJobUserTimeLimit', ctypes.c_int64),
        ('LimitFlags', W.DWORD),
        ('MinimumWorkingSetSize', ctypes.c_size_t),
        ('MaximumWorkingSetSize', ctypes.c_size_t),
        ('ActiveProcessLimit', W.DWORD),
        ('Affinity', ctypes.c_size_t),
        ('PriorityClass', W.DWORD),
        ('SchedulingClass', W.DWORD),
    ]
class JOBOBJECT_EXTENDED_LIMIT_INFORMATION(ctypes.Structure):
    _fields_ = [
        ('BasicLimitInformation', JOBOBJECT_BASIC_LIMIT_INFORMATION),
        ('IoInfo', IO_COUNTERS),
        ('ProcessMemoryLimit', ctypes.c_size_t),
        ('JobMemoryLimit', ctypes.c_size_t),
        ('PeakProcessMemoryUsed', ctypes.c_size_t),
        ('PeakJobMemoryUsed', ctypes.c_size_t),
    ]
class PROCESS_MEMORY_COUNTERS_EX(ctypes.Structure):
    _fields_ = [
        ('cb', W.DWORD), ('PageFaultCount', W.DWORD),
        ('PeakWorkingSetSize', ctypes.c_size_t), ('WorkingSetSize', ctypes.c_size_t),
        ('QuotaPeakPagedPoolUsage', ctypes.c_size_t), ('QuotaPagedPoolUsage', ctypes.c_size_t),
        ('QuotaPeakNonPagedPoolUsage', ctypes.c_size_t), ('QuotaNonPagedPoolUsage', ctypes.c_size_t),
        ('PagefileUsage', ctypes.c_size_t), ('PeakPagefileUsage', ctypes.c_size_t),
        ('PrivateUsage', ctypes.c_size_t),
    ]
k32 = ctypes.WinDLL('kernel32', use_last_error=True)
k32.GetCurrentProcess.restype = W.HANDLE
k32.K32GetProcessMemoryInfo.argtypes = [W.HANDLE, ctypes.POINTER(PROCESS_MEMORY_COUNTERS_EX), W.DWORD]
k32.K32GetProcessMemoryInfo.restype = W.BOOL
k32.QueryInformationJobObject.argtypes = [W.HANDLE, W.DWORD, ctypes.c_void_p, W.DWORD, ctypes.POINTER(W.DWORD)]
k32.QueryInformationJobObject.restype = W.BOOL
info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION()
ret = W.DWORD(0)
ok = bool(k32.QueryInformationJobObject(None, 9, ctypes.byref(info), ctypes.sizeof(info), ctypes.byref(ret)))
assert ok, f'QueryInformationJobObject failed: {ctypes.get_last_error()}'
pmc = PROCESS_MEMORY_COUNTERS_EX()
pmc.cb = ctypes.sizeof(pmc)
assert k32.K32GetProcessMemoryInfo(k32.GetCurrentProcess(), ctypes.byref(pmc), pmc.cb)
print(json.dumps({
    'flags': info.BasicLimitInformation.LimitFlags,
    'job_memory_limit': info.JobMemoryLimit,
    'peak_job_memory_used': info.PeakJobMemoryUsed,
    'private_usage': pmc.PrivateUsage,
}))"#;

fn verify_job_memory_enforced(w: &mut Worker, expected_limit: usize, label: &str) {
    const KILL_ON_CLOSE: u64 = 0x2000;
    const JOB_MEMORY: u64 = 0x0200;

    let before = py_json(w, QUERY_JOB_MEMORY_PY, Duration::from_secs(20));
    let flags = before["flags"].as_u64().unwrap();
    assert_eq!(
        flags & (KILL_ON_CLOSE | JOB_MEMORY),
        KILL_ON_CLOSE | JOB_MEMORY,
        "{label} missing Job Object flags: {before}"
    );
    assert_eq!(before["job_memory_limit"].as_u64().unwrap(), expected_limit as u64, "{label}: {before}");

    // Spawn a sleeping grandchild inside the Job so we also prove zero orphan children survive.
    let sleeper = py_json(
        w,
        "import json, subprocess, sys\n\
         p = subprocess.Popen([sys.executable, '-I', '-c', 'import time; time.sleep(30)'])\n\
         print(json.dumps({'pid': p.pid}))",
        Duration::from_secs(20),
    );
    let sleeper_pid = sleeper["pid"].as_u64().unwrap() as u32;

    // A child process inside the Job trying to allocate past the Job cap fails cleanly.
    let over_child = py_json(
        w,
        "import json, subprocess, sys\n\
         p = subprocess.run([sys.executable, '-I', '-c', 'b = bytearray(256 * 1024 * 1024)'], capture_output=True, text=True)\n\
         print(json.dumps({'rc': p.returncode, 'stderr': p.stderr[-400:]}))",
        Duration::from_secs(30),
    );
    assert_ne!(over_child["rc"], json!(0), "{label} child unexpectedly allocated 256 MiB: {over_child}");

    // An over-limit allocation in the worker itself is also blocked by the kernel Job cap.
    match w.evaluate("buf = bytearray(256 * 1024 * 1024)", Duration::from_secs(30)) {
        Ok(res) => {
            assert_eq!(
                res["error"]["type"],
                json!("MemoryError"),
                "{label} expected MemoryError on over-limit allocation: {res}"
            );
            let after = py_json(w, QUERY_JOB_MEMORY_PY, Duration::from_secs(20));
            let private_usage = after["private_usage"].as_u64().unwrap();
            let peak = after["peak_job_memory_used"].as_u64().unwrap();
            assert!(
                private_usage <= expected_limit as u64,
                "{label} live commit {private_usage} exceeded limit {expected_limit}"
            );
            assert!(
                peak >= expected_limit as u64,
                "{label} expected kernel peak charge >= {expected_limit} during rejected commit, got {peak}"
            );
            assert_eq!(w.evaluate("1 + 1", Duration::from_secs(10)).unwrap()["value"], "2");
        }
        Err(_) => {
            // If the runtime terminated the worker process on commit exhaustion, that is also clean.
        }
    }

    let worker_pid = w.worker_pid();
    w.shutdown();
    if let Some(pid) = worker_pid {
        assert!(process_is_gone(pid, 5000), "{label} worker {pid} survived shutdown");
    }
    assert!(process_is_gone(sleeper_pid, 5000), "{label} grandchild {sleeper_pid} leaked!");
}

#[test]
fn default_and_tight_job_memory_limits_enforce_cap_without_orphan_children() {
    let Some(python) = python_exe() else {
        eprintln!("SKIPPED: set RE_SEARCH_PYTHON to run job_memory_limit tests");
        return;
    };
    let python_path = PathBuf::from(python);
    let script_path = repo_root().join("python/worker.py");
    let mut fx = TempSandboxFixture::new("job-mem");

    // 1. Default config (`job_memory_limit_bytes: None`) sets 2 GiB in the kernel Job Object.
    let (def_scratch, def_pycache) = fx.prepare_session("s-mem-default");
    let mut def_worker = Worker::new(
        WorkerConfig {
            python: python_path.clone(),
            script: script_path.clone(),
            scratch: def_scratch,
            pycache: def_pycache,
            startup_timeout: Duration::from_secs(60),
            job_memory_limit_bytes: None,
        },
        Box::new(JobObjectSandbox),
    );
    let def_info: Value = py_json(&mut def_worker, QUERY_JOB_MEMORY_PY, Duration::from_secs(30));
    assert_eq!(
        def_info["job_memory_limit"].as_u64().unwrap(),
        DEFAULT_JOB_MEMORY_LIMIT_BYTES as u64,
        "default worker must use 2 GiB job memory limit: {def_info}"
    );
    let def_pid = def_worker.worker_pid().unwrap();
    drop(def_worker);
    assert!(process_is_gone(def_pid, 5000));

    // 2. Tight 128 MiB limit enforced under JobObjectSandbox.
    let tight_limit = 128 * 1024 * 1024;
    let (job_scratch, job_pycache) = fx.prepare_session("s-mem-job");
    let mut job_worker = Worker::new(
        WorkerConfig {
            python: python_path.clone(),
            script: script_path.clone(),
            scratch: job_scratch,
            pycache: job_pycache,
            startup_timeout: Duration::from_secs(60),
            job_memory_limit_bytes: Some(tight_limit),
        },
        Box::new(JobObjectSandbox),
    );
    verify_job_memory_enforced(&mut job_worker, tight_limit, "job-object");

    // 3. Tight 128 MiB limit enforced under AppContainerSandbox.
    let (ac_scratch, ac_pycache) = fx.prepare_session("s-mem-ac");
    let ac_sb = AppContainerSandbox::new(
        fx.data_dir.clone(),
        "s-mem-ac",
        python_path.clone(),
        script_path.clone(),
        ac_scratch.clone(),
        ac_pycache.clone(),
    )
    .unwrap();
    let mut ac_worker = Worker::new(
        WorkerConfig {
            python: python_path,
            script: script_path,
            scratch: ac_scratch,
            pycache: ac_pycache,
            startup_timeout: Duration::from_secs(60),
            job_memory_limit_bytes: Some(tight_limit),
        },
        Box::new(ac_sb),
    );
    verify_job_memory_enforced(&mut ac_worker, tight_limit, "appcontainer");
}
