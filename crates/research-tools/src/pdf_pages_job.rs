//! Thread-capable background extraction job with per-page progress (package F14a).
//!
//! Mirrors the shape of [`crate::paper_fetch::FetchJob`] so later Investigate / library routes
//! can run large PDF extractions off the request thread and poll progress per page.

use crate::pdf::{PdfError, PdfExtractStatus};
use crate::pdf_pages::{
    default_extractor, PageProgress, PdfDocumentPages, PdfExtractOptions, PdfExtractor,
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

/// Progress state of a [`PdfExtractJob`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PdfExtractStage {
    Queued,
    Loading,
    ExtractingPage { page: usize, total_pages: usize },
    Done {
        status: PdfExtractStatus,
        page_count: usize,
        extractor_id: String,
    },
    Failed { error: String },
}

/// Serializable point-in-time snapshot of a [`PdfExtractJob`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PdfExtractJobSnapshot {
    pub job_id: String,
    pub extractor_id: String,
    pub pages_done: usize,
    pub total_pages: usize,
    pub stage: PdfExtractStage,
    pub history: Vec<PdfExtractStage>,
}

struct PdfExtractJobState {
    job_id: String,
    extractor_id: String,
    pages_done: usize,
    total_pages: usize,
    stage: PdfExtractStage,
    history: Vec<PdfExtractStage>,
    finished: bool,
}

impl PdfExtractJobState {
    fn new(job_id: String, extractor_id: String) -> Self {
        Self {
            job_id,
            extractor_id,
            pages_done: 0,
            total_pages: 0,
            stage: PdfExtractStage::Queued,
            history: vec![PdfExtractStage::Queued],
            finished: false,
        }
    }

    fn set_stage(&mut self, stage: PdfExtractStage) {
        if let PdfExtractStage::ExtractingPage { page, total_pages } = stage {
            self.pages_done = page;
            self.total_pages = total_pages;
        }
        self.finished = matches!(
            stage,
            PdfExtractStage::Done { .. } | PdfExtractStage::Failed { .. }
        );
        self.stage = stage.clone();
        self.history.push(stage);
    }

    fn snapshot(&self) -> PdfExtractJobSnapshot {
        PdfExtractJobSnapshot {
            job_id: self.job_id.clone(),
            extractor_id: self.extractor_id.clone(),
            pages_done: self.pages_done,
            total_pages: self.total_pages,
            stage: self.stage.clone(),
            history: self.history.clone(),
        }
    }
}

/// Thread-capable background PDF extraction job reporting progress page by page.
pub struct PdfExtractJob {
    state: Arc<Mutex<PdfExtractJobState>>,
    cancel_flag: Arc<AtomicBool>,
    handle: Option<JoinHandle<Result<PdfDocumentPages, PdfError>>>,
    sync_result: Option<Result<PdfDocumentPages, PdfError>>,
}

impl PdfExtractJob {
    /// Start extracting `bytes` on a background thread using the default extractor.
    pub fn start(
        job_id: impl Into<String>,
        bytes: Vec<u8>,
        opts: PdfExtractOptions,
    ) -> Self {
        Self::start_with_extractor(job_id, bytes, opts, default_extractor())
    }

    /// Start extracting `bytes` on a background thread with an explicit [`PdfExtractor`].
    pub fn start_with_extractor<E>(
        job_id: impl Into<String>,
        bytes: Vec<u8>,
        mut opts: PdfExtractOptions,
        extractor: E,
    ) -> Self
    where
        E: PdfExtractor + Send + 'static,
    {
        let state = Arc::new(Mutex::new(PdfExtractJobState::new(
            job_id.into(),
            extractor.id().to_string(),
        )));
        let cancel_flag = Arc::new(AtomicBool::new(false));
        opts.cancel = Some(cancel_flag.clone());

        let st = state.clone();
        let handle = thread::spawn(move || {
            {
                let mut g = st.lock().unwrap_or_else(|e| e.into_inner());
                g.set_stage(PdfExtractStage::Loading);
            }
            let mut on_page = |prog: PageProgress| {
                let mut g = st.lock().unwrap_or_else(|e| e.into_inner());
                g.set_stage(PdfExtractStage::ExtractingPage {
                    page: prog.page,
                    total_pages: prog.total_pages,
                });
            };
            let res = extractor.extract_with_progress(&bytes, &opts, &mut on_page);
            record_terminal(&st, &res);
            res
        });

        Self {
            state,
            cancel_flag,
            handle: Some(handle),
            sync_result: None,
        }
    }

    /// Run synchronously on the calling thread while recording the full [`PdfExtractJobSnapshot`]
    /// stage history (convenient for deterministic tests).
    pub fn run_sync<E: PdfExtractor>(
        job_id: impl Into<String>,
        bytes: &[u8],
        opts: &PdfExtractOptions,
        extractor: &E,
    ) -> Self {
        let state = Arc::new(Mutex::new(PdfExtractJobState::new(
            job_id.into(),
            extractor.id().to_string(),
        )));
        let cancel_flag = Arc::new(AtomicBool::new(false));
        {
            let mut g = state.lock().unwrap_or_else(|e| e.into_inner());
            g.set_stage(PdfExtractStage::Loading);
        }
        let st = state.clone();
        let mut on_page = |prog: PageProgress| {
            let mut g = st.lock().unwrap_or_else(|e| e.into_inner());
            g.set_stage(PdfExtractStage::ExtractingPage {
                page: prog.page,
                total_pages: prog.total_pages,
            });
        };
        let res = extractor.extract_with_progress(bytes, opts, &mut on_page);
        record_terminal(&st, &res);
        Self {
            state,
            cancel_flag,
            handle: None,
            sync_result: Some(res),
        }
    }

    /// Request cooperative cancellation between pages.
    pub fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> PdfExtractJobSnapshot {
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

    /// Wait for the job (if background) and return its extraction result.
    pub fn join(mut self) -> Result<PdfDocumentPages, PdfError> {
        if let Some(res) = self.sync_result.take() {
            return res;
        }
        match self.handle.take() {
            Some(h) => h.join().map_err(|_| PdfError::ParserPanic)?,
            None => Err(PdfError::Malformed("PDF extraction job already joined".into())),
        }
    }
}

fn record_terminal(
    state: &Arc<Mutex<PdfExtractJobState>>,
    res: &Result<PdfDocumentPages, PdfError>,
) {
    let terminal = match res {
        Ok(doc) => PdfExtractStage::Done {
            status: doc.status,
            page_count: doc.page_count,
            extractor_id: doc.extractor_id.clone(),
        },
        Err(err) => PdfExtractStage::Failed {
            error: err.to_string(),
        },
    };
    let mut g = state.lock().unwrap_or_else(|e| e.into_inner());
    g.set_stage(terminal);
}
