//! Needs a Python with SymPy. Set RE_SEARCH_PYTHON (or RESEARCH_CORE_PYTHON)
//! to its path; without it the test is skipped (and says so).
#![cfg(windows)]

use std::path::PathBuf;
use std::time::{Duration, Instant};
use worker::sandbox::JobObjectSandbox;
use worker::{Worker, WorkerConfig, WorkerError};

fn python_exe() -> Option<String> {
    std::env::var("RE_SEARCH_PYTHON")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var("RESEARCH_CORE_PYTHON").ok().filter(|v| !v.is_empty()))
}

#[test]
fn evaluate_timeout_kill_respawn() {
    let Some(python) = python_exe() else {
        eprintln!("SKIPPED: set RE_SEARCH_PYTHON to run the worker test");
        return;
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let scratch = std::env::temp_dir().join(format!("worker-test-{}", std::process::id()));
    let config = WorkerConfig {
        python: PathBuf::from(python),
        script: root.join("python/worker.py"),
        scratch: scratch.clone(),
        pycache: scratch.join("pycache"),
        startup_timeout: Duration::from_secs(60),
        job_memory_limit_bytes: None,
    };
    let mut w = Worker::new(config, Box::new(JobObjectSandbox));
    assert_eq!(w.live_epoch(), None);
    let t = Instant::now();

    // 1. A real evaluation; printing does not disturb the protocol.
    let (r, ep) = w
        .evaluate_in_epoch("x = symbols('x')\nprint('printed')\nintegrate(x*sin(x), x)", Duration::from_secs(60), None)
        .unwrap();
    assert_eq!(ep, 1);
    assert_eq!(w.live_epoch(), Some(1));
    assert_eq!(r["value"], "-x*cos(x) + sin(x)");
    assert_eq!(r["stdout"], "printed\n");
    eprintln!("# first evaluate (with start-up): {:?}", t.elapsed());

    // 2. State persists between calls within the epoch; an exception is reported, not fatal.
    let (r, ep) = w.evaluate_in_epoch("diff(-x*cos(x) + sin(x), x)", Duration::from_secs(20), Some(1)).unwrap();
    assert_eq!(ep, 1);
    assert_eq!(r["value"], "x*sin(x)");
    let r = w.evaluate("1/0", Duration::from_secs(20)).unwrap();
    assert_eq!(r["error"]["type"], "ZeroDivisionError");

    // 3. TEMP is redirected to the scratch directory.
    let r = w.evaluate("import tempfile; tempfile.gettempdir()", Duration::from_secs(20)).unwrap();
    let temp = r["value"].as_str().unwrap().to_lowercase().replace("\\\\", "\\");
    assert!(temp.contains(&scratch.file_name().unwrap().to_string_lossy().to_lowercase()), "{temp}");

    // 4. A call that hangs is killed after the timeout, clearing the live epoch.
    let pid = w.hello().unwrap()["pid"].as_u64().unwrap() as u32;
    let t = Instant::now();
    let err = w.evaluate("while True: pass", Duration::from_secs(2)).unwrap_err();
    assert!(matches!(err, WorkerError::Timeout(_)), "{err}");
    assert_eq!(w.live_epoch(), None);
    assert!(t.elapsed() < Duration::from_secs(10));
    assert!(worker::win::process_is_gone(pid, 5000), "interpreter {pid} survived the kill");

    // 4b. Requiring the dead epoch refuses to respawn.
    let err = w.evaluate_in_epoch("1+1", Duration::from_secs(5), Some(1)).unwrap_err();
    assert!(matches!(err, WorkerError::EpochMismatch { expected: 1, actual: None }), "{err}");
    assert_eq!(w.spawn_count(), 1);

    // 5. Starting a new session starts epoch 2 with fresh state.
    let (r, ep) = w.evaluate_in_epoch("'x' in dir()", Duration::from_secs(60), None).unwrap();
    assert_eq!(ep, 2);
    assert_eq!(w.live_epoch(), Some(2));
    assert_eq!(r["value"], "False");
    assert_eq!(w.spawn_count(), 2);

    // 6. Dropping the worker kills the tree (job handle closed).
    let pid = w.hello().unwrap()["pid"].as_u64().unwrap() as u32;
    drop(w);
    assert!(worker::win::process_is_gone(pid, 5000), "interpreter {pid} survived the drop");
    let _ = std::fs::remove_dir_all(&scratch);
}
