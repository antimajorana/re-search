//! Thread-capable job and progress-state wrapper for [`crate::paper_fetch`] (used by V4).

use crate::net_rate::{global_gate, NetRateGate};
use crate::paper_fetch::{fetch_with_progress, FetchOutcome, FetchWant};
use crate::paper_key::PaperRef;
use crate::ResearchEnv;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

/// Progress state of a [`FetchJob`] so later routes (V4) can report step-by-step status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum FetchStage {
    Queued,
    CheckingCache,
    ResolvingId,
    FetchingMetadata,
    FetchingVersions,
    FetchingEprint,
    FetchingPdf,
    Validating,
    Done { status: String },
    NeedsFile { reason: String },
    Failed { error: String },
}

/// Serializable point-in-time view of a [`FetchJob`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchJobSnapshot {
    pub job_id: String,
    pub paper: PaperRef,
    pub want: FetchWant,
    pub stage: FetchStage,
    pub history: Vec<FetchStage>,
}

struct FetchJobState {
    job_id: String,
    paper: PaperRef,
    want: FetchWant,
    stage: FetchStage,
    history: Vec<FetchStage>,
    finished: bool,
}

impl FetchJobState {
    fn new(job_id: String, paper: PaperRef, want: FetchWant) -> Self {
        Self {
            job_id,
            paper,
            want,
            stage: FetchStage::Queued,
            history: vec![FetchStage::Queued],
            finished: false,
        }
    }

    fn set_stage(&mut self, stage: FetchStage) {
        self.finished = matches!(
            stage,
            FetchStage::Done { .. } | FetchStage::NeedsFile { .. } | FetchStage::Failed { .. }
        );
        self.stage = stage.clone();
        self.history.push(stage);
    }

    fn snapshot(&self) -> FetchJobSnapshot {
        FetchJobSnapshot {
            job_id: self.job_id.clone(),
            paper: self.paper.clone(),
            want: self.want,
            stage: self.stage.clone(),
            history: self.history.clone(),
        }
    }
}

/// Small thread-capable job wrapper around [`fetch_with_progress`] for V4 routes.
pub struct FetchJob {
    state: Arc<Mutex<FetchJobState>>,
    handle: Option<JoinHandle<Result<FetchOutcome, String>>>,
    sync_result: Option<Result<FetchOutcome, String>>,
}

impl FetchJob {
    /// Start fetching on a background thread using [`global_gate`].
    pub fn start(
        job_id: impl Into<String>,
        env: ResearchEnv,
        paper: PaperRef,
        want: FetchWant,
    ) -> Self {
        Self::start_with_gate(job_id, env, paper, want, None)
    }

    /// Start fetching on a background thread using an optional injected [`NetRateGate`].
    pub fn start_with_gate(
        job_id: impl Into<String>,
        env: ResearchEnv,
        paper: PaperRef,
        want: FetchWant,
        gate: Option<Arc<NetRateGate>>,
    ) -> Self {
        let state = Arc::new(Mutex::new(FetchJobState::new(
            job_id.into(),
            paper.clone(),
            want,
        )));
        let st = state.clone();
        let handle = thread::spawn(move || {
            let mut report = |stage: FetchStage| {
                let mut g = st.lock().unwrap_or_else(|e| e.into_inner());
                g.set_stage(stage);
            };
            let gate_ref: &NetRateGate = match gate.as_deref() {
                Some(g) => g,
                None => global_gate(),
            };
            let res = fetch_with_progress(&env, &paper, want, gate_ref, &mut report);
            record_terminal_stage(&st, &res);
            res
        });
        Self {
            state,
            handle: Some(handle),
            sync_result: None,
        }
    }

    /// Run synchronously on the calling thread while still recording [`FetchJobSnapshot`]
    /// history (convenient for deterministic fake-clock tests).
    pub fn run_sync(
        job_id: impl Into<String>,
        env: &ResearchEnv,
        paper: &PaperRef,
        want: FetchWant,
        gate: &NetRateGate,
    ) -> Self {
        let state = Arc::new(Mutex::new(FetchJobState::new(
            job_id.into(),
            paper.clone(),
            want,
        )));
        let st = state.clone();
        let mut report = |stage: FetchStage| {
            let mut g = st.lock().unwrap_or_else(|e| e.into_inner());
            g.set_stage(stage);
        };
        let res = fetch_with_progress(env, paper, want, gate, &mut report);
        record_terminal_stage(&st, &res);
        Self {
            state,
            handle: None,
            sync_result: Some(res),
        }
    }

    pub fn snapshot(&self) -> FetchJobSnapshot {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot()
    }

    pub fn is_finished(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .finished
    }

    /// Wait for the job (if background) and return its [`FetchOutcome`].
    pub fn join(mut self) -> Result<FetchOutcome, String> {
        if let Some(res) = self.sync_result.take() {
            return res;
        }
        match self.handle.take() {
            Some(h) => h
                .join()
                .map_err(|_| "paper fetch thread panicked".to_string())?,
            None => Err("paper fetch job already joined".into()),
        }
    }
}

fn record_terminal_stage(state: &Arc<Mutex<FetchJobState>>, res: &Result<FetchOutcome, String>) {
    let terminal = match res {
        Ok(FetchOutcome::NeedsFile { reason, .. }) => FetchStage::NeedsFile {
            reason: reason.clone(),
        },
        Ok(other) => FetchStage::Done {
            status: other.status_str().to_string(),
        },
        Err(err) => FetchStage::Failed { error: err.clone() },
    };
    let mut g = state.lock().unwrap_or_else(|e| e.into_inner());
    g.set_stage(terminal);
}
