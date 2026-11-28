//! Thread-capable background PDF OCR job with per-page progress and cooperative
//! cancellation (package F14b).

use crate::pdf::{PdfError, PdfExtractStatus};
use crate::pdf_ocr::{
    extract_and_ocr_pdf_pages_with_progress, OcrPageRunner, PdfOcrOptions, PdfOcrOutcome,
    PdfOcrProgress,
};
use crate::pdf_render::PdfPageRenderer;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

/// Progress stage of a [`PdfOcrJob`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PdfOcrStage {
    Queued,
    ExtractingText,
    OcrPage { page: usize, done: usize, total_ocr_pages: usize },
    Done {
        status: PdfExtractStatus,
        page_count: usize,
        pages_ocr_applied: usize,
        ocr_available: bool,
        ocr_status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ocr_message: Option<String>,
    },
    Cancelled,
    Failed { error: String },
}

/// Serializable point-in-time snapshot of a [`PdfOcrJob`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PdfOcrJobSnapshot {
    pub job_id: String,
    pub pages_done: usize,
    pub total_ocr_pages: usize,
    pub stage: PdfOcrStage,
    pub history: Vec<PdfOcrStage>,
}

struct PdfOcrJobState {
    job_id: String,
    pages_done: usize,
    total_ocr_pages: usize,
    stage: PdfOcrStage,
    history: Vec<PdfOcrStage>,
    finished: bool,
}

impl PdfOcrJobState {
    fn new(job_id: String) -> Self {
        Self {
            job_id,
            pages_done: 0,
            total_ocr_pages: 0,
            stage: PdfOcrStage::Queued,
            history: vec![PdfOcrStage::Queued],
            finished: false,
        }
    }

    fn set_stage(&mut self, stage: PdfOcrStage) {
        if let PdfOcrStage::OcrPage { done, total_ocr_pages, .. } = stage {
            self.pages_done = done;
            self.total_ocr_pages = total_ocr_pages;
        }
        self.finished = matches!(
            stage,
            PdfOcrStage::Done { .. } | PdfOcrStage::Cancelled | PdfOcrStage::Failed { .. }
        );
        self.stage = stage.clone();
        self.history.push(stage);
    }

    fn snapshot(&self) -> PdfOcrJobSnapshot {
        PdfOcrJobSnapshot {
            job_id: self.job_id.clone(),
            pages_done: self.pages_done,
            total_ocr_pages: self.total_ocr_pages,
            stage: self.stage.clone(),
            history: self.history.clone(),
        }
    }
}

/// Thread-capable background PDF text-extraction and selective OCR job.
pub struct PdfOcrJob {
    state: Arc<Mutex<PdfOcrJobState>>,
    cancel_flag: Arc<AtomicBool>,
    handle: Option<JoinHandle<Result<PdfOcrOutcome, PdfError>>>,
    sync_result: Option<Result<PdfOcrOutcome, PdfError>>,
}

impl PdfOcrJob {
    /// Start extracting and selectively OCRing `bytes` on a background thread.
    pub fn start(
        job_id: impl Into<String>,
        bytes: Vec<u8>,
        mut opts: PdfOcrOptions,
        renderer: Arc<dyn PdfPageRenderer>,
        runner: Option<Arc<dyn OcrPageRunner>>,
    ) -> Self {
        let state = Arc::new(Mutex::new(PdfOcrJobState::new(job_id.into())));
        let cancel_flag = opts.cancel.clone().unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
        opts.cancel = Some(cancel_flag.clone());

        let st = state.clone();
        let handle = thread::spawn(move || {
            let runner_ref = runner.as_ref().map(|r| r.as_ref());
            run_with_state(&st, &bytes, &opts, renderer.as_ref(), runner_ref)
        });
        Self { state, cancel_flag, handle: Some(handle), sync_result: None }
    }

    /// Run synchronously on the calling thread while recording the full
    /// [`PdfOcrJobSnapshot`] stage history (convenient for deterministic tests).
    pub fn run_sync(
        job_id: impl Into<String>,
        bytes: &[u8],
        opts: &PdfOcrOptions,
        renderer: &dyn PdfPageRenderer,
        runner: Option<&dyn OcrPageRunner>,
    ) -> Self {
        let state = Arc::new(Mutex::new(PdfOcrJobState::new(job_id.into())));
        let cancel_flag = opts.cancel.clone().unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
        let mut run_opts = opts.clone();
        run_opts.cancel = Some(cancel_flag.clone());
        let res = run_with_state(&state, bytes, &run_opts, renderer, runner);
        Self { state, cancel_flag, handle: None, sync_result: Some(res) }
    }

    /// Request cooperative cancellation between pages.
    pub fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> PdfOcrJobSnapshot {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).snapshot()
    }

    pub fn is_finished(&self) -> bool {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).finished
    }

    /// Wait for the job (if background) and return its OCR outcome.
    pub fn join(mut self) -> Result<PdfOcrOutcome, PdfError> {
        if let Some(res) = self.sync_result.take() {
            return res;
        }
        match self.handle.take() {
            Some(h) => h.join().map_err(|_| PdfError::ParserPanic)?,
            None => Err(PdfError::Malformed("PDF OCR job already joined".into())),
        }
    }
}

fn run_with_state(
    state: &Arc<Mutex<PdfOcrJobState>>,
    bytes: &[u8],
    opts: &PdfOcrOptions,
    renderer: &dyn PdfPageRenderer,
    runner: Option<&dyn OcrPageRunner>,
) -> Result<PdfOcrOutcome, PdfError> {
    {
        let mut g = state.lock().unwrap_or_else(|e| e.into_inner());
        g.set_stage(PdfOcrStage::ExtractingText);
    }
    let mut on_progress = |prog: PdfOcrProgress| {
        let mut g = state.lock().unwrap_or_else(|e| e.into_inner());
        g.set_stage(PdfOcrStage::OcrPage {
            page: prog.page,
            done: prog.done,
            total_ocr_pages: prog.total_ocr_pages,
        });
    };
    let res = extract_and_ocr_pdf_pages_with_progress(bytes, opts, renderer, runner, &mut on_progress);
    record_terminal(state, &res);
    res
}

fn record_terminal(state: &Arc<Mutex<PdfOcrJobState>>, res: &Result<PdfOcrOutcome, PdfError>) {
    let terminal = match res {
        Ok(outcome) => PdfOcrStage::Done {
            status: outcome.document.status,
            page_count: outcome.document.page_count,
            pages_ocr_applied: outcome.provenance.as_ref().map_or(0, |p| p.pages_ocr_applied.len()),
            ocr_available: outcome.ocr_available,
            ocr_status: outcome.ocr_status.clone(),
            ocr_message: outcome.ocr_message.clone(),
        },
        Err(PdfError::Cancelled) => PdfOcrStage::Cancelled,
        Err(err) => PdfOcrStage::Failed { error: err.to_string() },
    };
    let mut g = state.lock().unwrap_or_else(|e| e.into_inner());
    g.set_stage(terminal);
}
