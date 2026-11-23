//! Python compute worker.
//!
//! The worker is a separate Python process. The protocol (one JSON object per
//! line) runs on two dedicated inherited pipes, not on stdin/stdout, so code
//! that prints cannot corrupt it. Methods: `evaluate` and `shutdown`.
//! A call that exceeds its timeout gets the whole process tree killed; the
//! next call starts a fresh worker (with fresh state).
//!
//! Containment properties are described by [`sandbox::Containment`] and
//! implemented by [`appcontainer::AppContainerSandbox`] and [`sandbox::JobObjectSandbox`].

#[cfg(windows)]
pub mod appcontainer;
pub mod sandbox;
#[cfg(windows)]
pub mod stage;
#[cfg(windows)]
pub mod win;

use sandbox::{Contained, Containment, Sandbox};
use serde_json::{json, Value};
use std::fmt;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::Duration;

pub const PROTOCOL_VERSION: u64 = 1;

#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub python: PathBuf,
    pub script: PathBuf,
    /// Working directory of the worker; TEMP and the matplotlib config are
    /// redirected here.
    pub scratch: PathBuf,
    /// Per-session directory where Python keeps compiled bytecode
    /// (`sessions/<id>/pycache`), isolating sessions from each other.
    pub pycache: PathBuf,
    pub startup_timeout: Duration,
}

#[derive(Debug)]
pub enum WorkerError {
    Spawn(String),
    /// The call did not answer in time; the worker was killed.
    Timeout(Duration),
    /// The worker died or sent something unexpected; it was killed.
    Protocol(String),
    /// A session-scoped call required a specific live worker epoch, which has ended.
    EpochMismatch { expected: u64, actual: Option<u64> },
}

impl fmt::Display for WorkerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkerError::Spawn(e) => write!(f, "could not start the worker: {e}"),
            WorkerError::Timeout(d) => {
                write!(f, "no answer within {} s; the worker was killed and its state is lost", d.as_secs())
            }
            WorkerError::Protocol(e) => write!(f, "worker protocol error (worker killed): {e}"),
            WorkerError::EpochMismatch { expected, actual } => write!(
                f,
                "worker session ended (expected epoch {expected}, current {actual:?}); a new approval is required"
            ),
        }
    }
}
impl std::error::Error for WorkerError {}

/// Lock-free handle reporting which worker epoch (`1, 2, ...`) is currently alive.
#[derive(Clone, Debug)]
pub struct EpochHandle(Arc<AtomicU64>);

impl EpochHandle {
    pub fn live(&self) -> Option<u64> {
        match self.0.load(Ordering::Acquire) {
            0 => None,
            n => Some(n),
        }
    }
}

struct Running {
    proc: Box<dyn Contained>,
    requests: File,
    responses: Receiver<String>,
    hello: Value,
}

pub struct Worker {
    #[cfg_attr(not(windows), allow(dead_code))]
    config: WorkerConfig,
    sandbox: Box<dyn Sandbox>,
    running: Option<Running>,
    next_id: u64,
    spawn_count: u64,
    epoch: Arc<AtomicU64>,
}

impl Worker {
    /// Nothing is started until the first call.
    pub fn new(config: WorkerConfig, sandbox: Box<dyn Sandbox>) -> Worker {
        Worker {
            config,
            sandbox,
            running: None,
            next_id: 1,
            spawn_count: 0,
            epoch: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn containment(&self) -> Containment {
        self.sandbox.containment()
    }

    /// How many worker processes have been started so far.
    pub fn spawn_count(&self) -> u64 {
        self.spawn_count
    }

    /// Serial number of the most recent request. Read this under the worker lock
    /// immediately after a successful evaluation to record its execution order.
    pub fn last_request_id(&self) -> u64 {
        self.next_id - 1
    }

    /// Handle for querying the live worker epoch without taking the worker lock.
    pub fn epoch_handle(&self) -> EpochHandle {
        EpochHandle(self.epoch.clone())
    }

    /// Epoch of the currently running worker (`None` if not started or exited).
    pub fn live_epoch(&self) -> Option<u64> {
        self.epoch_handle().live()
    }

    /// The worker's start-up message (pid, Python and SymPy versions), if it
    /// is running.
    pub fn hello(&self) -> Option<&Value> {
        self.running.as_ref().map(|r| &r.hello)
    }

    /// OS process ID of the currently running worker process, if any.
    pub fn worker_pid(&self) -> Option<u32> {
        self.running.as_ref().map(|r| r.proc.pid())
    }

    #[cfg(windows)]
    fn spawn(&mut self) -> Result<(), WorkerError> {
        use sandbox::WorkerLaunch;
        use stage::strip_verbatim;
        use std::collections::BTreeMap;
        use std::io::{BufRead, BufReader};
        use std::sync::mpsc;
        use win::{Direction, PipePair};

        let err = |e: std::io::Error| WorkerError::Spawn(e.to_string());
        let scratch = strip_verbatim(&self.config.scratch);
        let pycache = strip_verbatim(&self.config.pycache);
        std::fs::create_dir_all(&scratch).map_err(err)?;
        let to_child = PipePair::new(Direction::ToChild).map_err(err)?;
        let from_child = PipePair::new(Direction::FromChild).map_err(err)?;

        let system_root = std::env::var("SystemRoot")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| r"C:\Windows".to_string());
        let local_appdata = std::env::var("LOCALAPPDATA")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| scratch.to_string_lossy().into_owned());
        let s = |p: &std::path::Path| p.to_string_lossy().into_owned();
        let mut env = BTreeMap::new();
        env.insert("SystemRoot".into(), system_root);
        env.insert("LOCALAPPDATA".into(), local_appdata);
        env.insert("TEMP".into(), s(&scratch));
        env.insert("TMP".into(), s(&scratch));
        env.insert("PYTHONPYCACHEPREFIX".into(), s(&pycache));
        env.insert("MPLCONFIGDIR".into(), s(&scratch.join("mpl")));
        env.insert("PYTHONNOUSERSITE".into(), "1".into());
        env.insert("PYTHONIOENCODING".into(), "utf-8".into());

        let launch = WorkerLaunch {
            exe: self.config.python.clone(),
            args: vec![
                "-u".to_string(),
                strip_verbatim(&self.config.script).to_string_lossy().into_owned(),
                to_child.their_handle_value().to_string(),
                from_child.their_handle_value().to_string(),
            ],
            cwd: scratch,
            env,
            inherit_handles: vec![to_child.their_handle_value(), from_child.their_handle_value()],
        };
        let proc = self.sandbox.spawn_worker(launch).map_err(err)?;
        self.spawn_count += 1;
        let my_epoch = self.spawn_count;
        // Publish before spawning the reader thread so an early exit cannot lose its clear.
        self.epoch.store(my_epoch, Ordering::Release);
        let requests = to_child.into_ours();
        let responses_file = from_child.into_ours();

        let (tx, rx) = mpsc::channel();
        let epoch_on_exit = self.epoch.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(responses_file).lines() {
                match line {
                    Ok(line) => {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = epoch_on_exit.compare_exchange(my_epoch, 0, Ordering::Release, Ordering::Relaxed);
        });
        let mut running = Running { proc, requests, responses: rx, hello: Value::Null };
        match running.responses.recv_timeout(self.config.startup_timeout) {
            Ok(line) => match serde_json::from_str::<Value>(&line) {
                Ok(v) if v["ready"] == json!(true) && v["protocol"] == json!(PROTOCOL_VERSION) => {
                    running.hello = v;
                    self.running = Some(running);
                    Ok(())
                }
                _ => {
                    running.proc.kill_tree();
                    let _ = self.epoch.compare_exchange(my_epoch, 0, Ordering::Release, Ordering::Relaxed);
                    Err(WorkerError::Spawn(format!("unexpected start-up message: {line}")))
                }
            },
            Err(_) => {
                running.proc.kill_tree();
                let _ = self.epoch.compare_exchange(my_epoch, 0, Ordering::Release, Ordering::Relaxed);
                Err(WorkerError::Spawn("the worker did not report ready (wrong interpreter, or SymPy missing?)".into()))
            }
        }
    }

    #[cfg(not(windows))]
    fn spawn(&mut self) -> Result<(), WorkerError> {
        Err(WorkerError::Spawn("the v0 worker runs on Windows only".into()))
    }

    fn kill(&mut self) {
        if let Some(mut running) = self.running.take() {
            running.proc.kill_tree();
        }
        self.epoch.store(0, Ordering::Release);
    }

    fn request(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value, WorkerError> {
        if self.running.is_none() {
            self.spawn()?;
        }
        let id = self.next_id;
        self.next_id += 1;
        let line = format!("{}\n", json!({ "id": id, "method": method, "params": params }));
        let running = self.running.as_mut().expect("spawned above");
        if let Err(e) = running.requests.write_all(line.as_bytes()) {
            self.kill();
            return Err(WorkerError::Protocol(format!("could not send the request: {e}")));
        }
        let answer = running.responses.recv_timeout(timeout);
        match answer {
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(v) if v["id"] == json!(id) => Ok(v),
                _ => {
                    self.kill();
                    Err(WorkerError::Protocol("answer does not match the request".into()))
                }
            },
            Err(RecvTimeoutError::Timeout) => {
                self.kill();
                Err(WorkerError::Timeout(timeout))
            }
            Err(RecvTimeoutError::Disconnected) => {
                self.kill();
                Err(WorkerError::Protocol("the worker exited".into()))
            }
        }
    }

    /// Run `code` in the worker, optionally enforcing under `&mut self` that
    /// the call runs inside `required_epoch` without respawning a dead worker.
    /// Returns `(result, ran_epoch)`.
    pub fn evaluate_in_epoch(
        &mut self,
        code: &str,
        timeout: Duration,
        required_epoch: Option<u64>,
    ) -> Result<(Value, u64), WorkerError> {
        if self.running.is_some() && self.epoch.load(Ordering::Acquire) == 0 {
            self.kill();
        }
        if let Some(req) = required_epoch {
            let current = self.live_epoch();
            if self.running.is_none() || self.spawn_count != req || current != Some(req) {
                return Err(WorkerError::EpochMismatch { expected: req, actual: current });
            }
        }
        let answer = self.request("evaluate", json!({ "code": code }), timeout)?;
        let ran_epoch = self.spawn_count;
        if answer["ok"] == json!(true) {
            Ok((answer["result"].clone(), ran_epoch))
        } else {
            Err(WorkerError::Protocol(answer["error"].to_string()))
        }
    }

    /// Run `code` in the worker. Returns the worker's result object:
    /// `value` (repr of the last expression, or null), `stdout`, `stderr`,
    /// `truncated`, optionally `latex`, and `error` if the code raised.
    pub fn evaluate(&mut self, code: &str, timeout: Duration) -> Result<Value, WorkerError> {
        self.evaluate_in_epoch(code, timeout, None).map(|(v, _)| v)
    }

    /// Ask the worker to exit; kill it if it does not.
    pub fn shutdown(&mut self) {
        if self.running.is_some() {
            let _ = self.request("shutdown", json!({}), Duration::from_secs(5));
            self.kill();
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.kill();
    }
}
