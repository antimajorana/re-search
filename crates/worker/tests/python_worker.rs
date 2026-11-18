//! Needs a Python with SymPy. Set RESEARCH_CORE_PYTHON to its path; without
//! it the test is skipped (and says so).
#![cfg(windows)]

use std::path::PathBuf;
use std::time::{Duration, Instant};
use worker::sandbox::JobObjectSandbox;
use worker::{Worker, WorkerConfig, WorkerError};

#[test]
fn evaluate_timeout_kill_respawn() {
    let Ok(python) = std::env::var("RESEARCH_CORE_PYTHON") else {
        eprintln!("SKIPPED: set RESEARCH_CORE_PYTHON to run the worker test");
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
    };
    let mut w = Worker::new(config, Box::new(JobObjectSandbox));
    let t = Instant::now();

    // 1. A real evaluation; printing does not disturb the protocol.
    let r = w
        .evaluate("x = symbols('x')\nprint('printed')\nintegrate(x*sin(x), x)", Duration::from_secs(60))
        .unwrap();
    assert_eq!(r["value"], "-x*cos(x) + sin(x)");
    assert_eq!(r["stdout"], "printed\n");
    eprintln!("# first evaluate (with start-up): {:?}", t.elapsed());

    // 2. State persists between calls; an exception is reported, not fatal.
    let r = w.evaluate("diff(-x*cos(x) + sin(x), x)", Duration::from_secs(20)).unwrap();
    assert_eq!(r["value"], "x*sin(x)");
    let r = w.evaluate("1/0", Duration::from_secs(20)).unwrap();
    assert_eq!(r["error"]["type"], "ZeroDivisionError");

    // 3. TEMP is redirected to the scratch directory.
    let r = w.evaluate("import tempfile; tempfile.gettempdir()", Duration::from_secs(20)).unwrap();
    let temp = r["value"].as_str().unwrap().to_lowercase().replace("\\\\", "\\");
    assert!(temp.contains(&scratch.file_name().unwrap().to_string_lossy().to_lowercase()), "{temp}");

    // 4. A call that hangs is killed after the timeout, with the whole tree.
    let pid = w.hello().unwrap()["pid"].as_u64().unwrap() as u32;
    let t = Instant::now();
    let err = w.evaluate("while True: pass", Duration::from_secs(2)).unwrap_err();
    assert!(matches!(err, WorkerError::Timeout(_)), "{err}");
    assert!(t.elapsed() < Duration::from_secs(10));
    assert!(worker::win::process_is_gone(pid, 5000), "interpreter {pid} survived the kill");

    // 5. The next call starts a fresh worker with fresh state.
    let r = w.evaluate("'x' in dir()", Duration::from_secs(60)).unwrap();
    assert_eq!(r["value"], "False");
    assert_eq!(w.spawn_count(), 2);

    // 6. Dropping the worker kills the tree (job handle closed).
    let pid = w.hello().unwrap()["pid"].as_u64().unwrap() as u32;
    drop(w);
    assert!(worker::win::process_is_gone(pid, 5000), "interpreter {pid} survived the drop");
    let _ = std::fs::remove_dir_all(&scratch);
}
