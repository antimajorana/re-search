//! Python compute worker.
//!
//! The worker is a separate Python process. The protocol (one JSON object per
//! line) runs on two dedicated inherited pipes, not on stdin/stdout, so code
//! that prints cannot corrupt it. v0 methods: `evaluate` and `shutdown`.
//! A call that exceeds its timeout gets the whole process tree killed; the
//! next call starts a fresh worker (with fresh state).
//!
//! The worker is not sandboxed in v0: see [`sandbox::Containment`].

pub mod sandbox;
#[cfg(windows)]
pub mod win;

use sandbox::{Contained, Containment, Sandbox};
use serde_json::{json, Value};
use std::fmt;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

pub const PROTOCOL_VERSION: u64 = 1;

#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub python: PathBuf,
    pub script: PathBuf,
    /// Working directory of the worker; TEMP, the bytecode cache and the
    /// matplotlib config are redirected here.
    pub scratch: PathBuf,
    /// Where Python keeps compiled bytecode. Sharing it between sessions
    /// saves ~15 s of start-up; it is never inside the user's source trees.
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
}

impl fmt::Display for WorkerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkerError::Spawn(e) => write!(f, "could not start the worker: {e}"),
            WorkerError::Timeout(d) => {
                write!(f, "no answer within {} s; the worker was killed and its state is lost", d.as_secs())
            }
            WorkerError::Protocol(e) => write!(f, "worker protocol error (worker killed): {e}"),
        }
    }
}
impl std::error::Error for WorkerError {}

struct Running {
    proc: Box<dyn Contained>,
    requests: File,
    responses: Receiver<String>,
    hello: Value,
}

pub struct Worker {
    config: WorkerConfig,
    sandbox: Box<dyn Sandbox>,
    running: Option<Running>,
    next_id: u64,
    spawn_count: u64,
}

impl Worker {
    /// Nothing is started until the first call.
    pub fn new(config: WorkerConfig, sandbox: Box<dyn Sandbox>) -> Worker {
        Worker { config, sandbox, running: None, next_id: 1, spawn_count: 0 }
    }

    pub fn containment(&self) -> Containment {
        self.sandbox.containment()
    }

    /// How many worker processes have been started so far.
    pub fn spawn_count(&self) -> u64 {
        self.spawn_count
    }

    /// The worker's start-up message (pid, Python and SymPy versions), if it
    /// is running.
    pub fn hello(&self) -> Option<&Value> {
        self.running.as_ref().map(|r| &r.hello)
    }

    #[cfg(windows)]
    fn spawn(&mut self) -> Result<(), WorkerError> {
        use win::{Direction, PipePair};
        let err = |e: std::io::Error| WorkerError::Spawn(e.to_string());
        std::fs::create_dir_all(&self.config.scratch).map_err(err)?;
        let to_child = PipePair::new(Direction::ToChild).map_err(err)?;
        let from_child = PipePair::new(Direction::FromChild).map_err(err)?;

        let scratch = &self.config.scratch;
        let mut command = Command::new(&self.config.python);
        command
            .arg("-u")
            .arg(&self.config.script)
            .arg(to_child.their_handle_value().to_string())
            .arg(from_child.their_handle_value().to_string())
            .current_dir(scratch)
            .env("TEMP", scratch)
            .env("TMP", scratch)
            .env("PYTHONPYCACHEPREFIX", &self.config.pycache)
            .env("MPLCONFIGDIR", scratch.join("mpl"))
            .env("PYTHONNOUSERSITE", "1")
            .env("PYTHONIOENCODING", "utf-8")
            // Not the protocol: whatever the interpreter itself writes here is dropped.
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let proc = self.sandbox.spawn(command).map_err(err)?;
        self.spawn_count += 1;
        let requests = to_child.into_ours();
        let responses_file = from_child.into_ours();

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(responses_file).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
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
                    Err(WorkerError::Spawn(format!("unexpected start-up message: {line}")))
                }
            },
            Err(_) => {
                running.proc.kill_tree();
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

    /// Run `code` in the worker. Returns the worker's result object:
    /// `value` (repr of the last expression, or null), `stdout`, `stderr`,
    /// `truncated`, optionally `latex`, and `error` if the code raised.
    pub fn evaluate(&mut self, code: &str, timeout: Duration) -> Result<Value, WorkerError> {
        let answer = self.request("evaluate", json!({ "code": code }), timeout)?;
        if answer["ok"] == json!(true) {
            Ok(answer["result"].clone())
        } else {
            Err(WorkerError::Protocol(answer["error"].to_string()))
        }
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
