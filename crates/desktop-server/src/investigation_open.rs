//! Asynchronous paper-open orchestration, progress tracking, and HTTP handlers for
//! Investigate (package V4).
//!
//! # Approval rule (`00-overview.md` §2.4; `research_tools::paper_fetch`)
//!
//! Outgoing network requests run without an approval card **only** when initiated by the
//! user's own click or keystroke naming that exact paper (`arxiv_id`, `doi`, `url`,
//! `source_id`, or `library_key`). This endpoint is host-only and MUST NEVER be invoked by
//! a model or autonomous agent; model-initiated fetches must go through
//! `tools::Router::try_call` and the real approver.
//!
//! # Registration note (`http_srv.rs` is not edited by V4)
//!
//! Add one delegating arm before the catch-all `investigation` arm in `dispatch_api`:
//! ```text
//! (method, ["api", "sessions", id, "investigation", op @ ("open" | "open-status")]) => {
//!     write_api_res(&mut stream, crate::investigation_open::handle(state, method, id, op, &req.query, &req.body));
//! }
//! ```

use crate::investigation::{is_investigation, latest};
use crate::investigation_skeleton::{
    build_and_record_skeleton, build_skeleton_from_receipt, derive_paper_identity,
    is_skeleton_receipt, pin_source_and_build_skeleton, EVENT_INVESTIGATION_PAPER,
    EVENT_INVESTIGATION_SKELETON,
};
use crate::routes_session::{bad, conflict, internal, not_found, parse_json_body, ApiResult};
use crate::state::{now_ms, DesktopState};
use ledger::{Event, Ledger};
use research_tools::library::{Library, LibraryIds, PaperFlags, PaperRecord, SeenRecord, SeenRole};
use research_tools::net_rate::{global_gate, NetRateGate};
use research_tools::paper_fetch::{
    check_fetch_budget, fetch_with_progress, fetched_sources, parse_paper_input, FetchOutcome,
    FetchStage, FetchWant, PaperMetadata, DEFAULT_INVESTIGATE_FETCH_LIMIT,
};
use research_tools::paper_key::{PaperIds, PaperKey, PaperKeyKind, PaperRef};
use research_tools::skeleton::{PaperSkeleton, SkNodeKind};
use research_tools::{validate_session_id, ResearchEnv};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{atomic::{AtomicU64, Ordering}, Arc, Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use tools::canon::random_hex;

/// Default per-session fetched-source budget for Investigate (`investigate.md` §6.16).
pub const DEFAULT_FETCH_LIMIT: usize = DEFAULT_INVESTIGATE_FETCH_LIMIT;

/// User-facing guidance when a DOI has no arXiv source and is absent from the local library
/// (`investigate.md` §3 Stage A).
pub const NEEDS_FILE_HINT: &str =
    "This DOI has no arXiv version and is not in your library; drop the PDF or TeX";

/// Upper bound on reader-raised session fetch limits (`investigate.md` §6.16).
const MAX_FETCH_LIMIT: usize = 256;
const MAX_STORED_JOBS: usize = 256;
const EVENT_INVESTIGATION_NOTE: &str = "investigation.note";
const EVENT_SOURCE_INGESTED: &str = "source.ingested";

/// Coarse progress stage of an Investigate paper-open job (`investigate.md` §3 Stage A).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenStep {
    Queued, Metadata, LibraryLookup, EprintTex, PdfFallback,
    Ingest, Skeleton, Done, NeedsFile, Failed,
}

impl OpenStep {
    /// True once the job will make no further transitions.
    pub fn is_terminal(self) -> bool { matches!(self, Self::Done | Self::NeedsFile | Self::Failed) }
}

/// Serializable point-in-time snapshot of an Investigate paper-open job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenJobSnapshot {
    pub job_id: String,
    pub session_id: String,
    pub paper: Option<PaperRef>,
    pub step: OpenStep,
    pub history: Vec<OpenStep>,
    pub status: String,
    pub message: String,
    pub metadata: Option<PaperMetadata>,
    pub source_id: Option<String>,
    pub source_origin: Option<String>,
    pub skeleton_seq: Option<u64>,
    pub skeleton_blob: Option<String>,
    pub skeleton_timing_us: Option<u64>,
    pub fetch_budget_used: usize,
    pub fetch_budget_limit: usize,
    pub needs_file_reason: Option<String>,
    pub error: Option<String>,
}

impl OpenJobSnapshot {
    fn new(session_id: &str, paper: Option<PaperRef>, used: usize, limit: usize) -> Self {
        Self {
            job_id: next_job_id(), session_id: session_id.to_string(), paper,
            step: OpenStep::Queued, history: vec![OpenStep::Queued],
            status: "running".to_string(), message: "Queued paper open job.".to_string(),
            metadata: None, source_id: None, source_origin: None,
            skeleton_seq: None, skeleton_blob: None, skeleton_timing_us: None,
            fetch_budget_used: used, fetch_budget_limit: limit, needs_file_reason: None, error: None,
        }
    }

    fn advance(&mut self, step: OpenStep, status: &str, msg: impl Into<String>) {
        if self.step != step { self.step = step; self.history.push(step); }
        self.status = status.to_string();
        self.message = msg.into();
    }
}

struct OpenJobSlot { snapshot: Mutex<OpenJobSnapshot>, cv: Condvar }
struct JobMap { by_id: HashMap<String, Arc<OpenJobSlot>>, order: VecDeque<String> }
struct BuiltSkeletonSummary { event_seq: u64, blob_hash: String, timing_us: Option<u64> }
struct LocalOpenOpts {
    key_override: Option<PaperKey>, version: Option<String>, question: Option<String>,
    title: Option<String>, should_pin: bool, used: usize, limit: usize,
}
struct OpenWorkerConfig {
    data_dir: PathBuf, env: ResearchEnv, session_id: String, paper: PaperRef,
    question: Option<String>, should_pin: bool, gate: Option<Arc<NetRateGate>>,
}

static JOB_SEQ: AtomicU64 = AtomicU64::new(1);
static JOB_STORE: OnceLock<Mutex<JobMap>> = OnceLock::new();

fn job_store() -> &'static Mutex<JobMap> {
    JOB_STORE.get_or_init(|| Mutex::new(JobMap { by_id: HashMap::new(), order: VecDeque::new() }))
}
fn next_job_id() -> String { format!("open_{}_{}", JOB_SEQ.fetch_add(1, Ordering::Relaxed), random_hex(6)) }
fn register_job(initial: OpenJobSnapshot) -> Arc<OpenJobSlot> {
    let job_id = initial.job_id.clone();
    let slot = Arc::new(OpenJobSlot { snapshot: Mutex::new(initial), cv: Condvar::new() });
    let mut map = job_store().lock().unwrap_or_else(|e| e.into_inner());
    while map.order.len() >= MAX_STORED_JOBS {
        if let Some(evicted) = map.order.pop_front() { map.by_id.remove(&evicted); }
    }
    map.order.push_back(job_id.clone());
    map.by_id.insert(job_id, Arc::clone(&slot));
    slot
}
fn get_job_slot(session_id: &str, job_id: &str) -> Option<Arc<OpenJobSlot>> {
    let slot = job_store().lock().unwrap_or_else(|e| e.into_inner()).by_id.get(job_id).cloned()?;
    let matches = slot.snapshot.lock().unwrap_or_else(|e| e.into_inner()).session_id == session_id;
    matches.then_some(slot)
}
fn update_slot(slot: &OpenJobSlot, f: impl FnOnce(&mut OpenJobSnapshot)) {
    f(&mut slot.snapshot.lock().unwrap_or_else(|e| e.into_inner()));
    slot.cv.notify_all();
}

/// Computes the effective per-session fetched-source limit (`investigate.md` §6.16).
/// Starts at [`DEFAULT_FETCH_LIMIT`] (8) and honors the highest reader-recorded
/// `investigation.note` event carrying a numeric `fetch_limit` (clamped to `<= 256`).
pub fn effective_fetch_limit(events: &[Event]) -> usize {
    events.iter().filter(|e| e.kind == EVENT_INVESTIGATION_NOTE)
        .filter_map(|e| e.payload["fetch_limit"].as_u64())
        .map(|n| n.min(MAX_FETCH_LIMIT as u64) as usize)
        .fold(DEFAULT_FETCH_LIMIT, usize::max)
}

/// Starts an Investigate paper-open job using the process-wide [`global_gate`].
pub fn start_open_job(state: &DesktopState, session_id: &str, req_body: &Value) -> Result<OpenJobSnapshot, (u16, String)> {
    start_open_job_with_gate(state, session_id, req_body, None)
}

/// Starts an Investigate paper-open job using an optional injected [`NetRateGate`].
pub fn start_open_job_with_gate(
    state: &DesktopState, session_id: &str, req_body: &Value, gate: Option<Arc<NetRateGate>>,
) -> Result<OpenJobSnapshot, (u16, String)> {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    let events = state.open_ledger().map_err(internal)?.read_session(session_id).map_err(storage)?;
    if !is_investigation(&events) { return Err(bad("Open an Investigation session before opening a paper.")); }

    let (limit, used) = (effective_fetch_limit(&events), fetched_sources(&events));
    let opts = LocalOpenOpts {
        key_override: None,
        version: non_empty(&req_body["version"]).map(str::to_string),
        question: non_empty(&req_body["question"]).map(str::to_string),
        title: None,
        should_pin: req_body.get("pin").and_then(Value::as_bool).unwrap_or(true),
        used, limit,
    };
    if let Some(sid) = non_empty(&req_body["source_id"]) {
        return open_existing_source(state, session_id, &events, sid, req_body, opts);
    }
    if let Some(raw_key) = non_empty(&req_body["library_key"]) {
        return open_from_library(state, session_id, raw_key, opts);
    }

    let raw_input = ["arxiv_id", "doi", "url", "input"].iter().find_map(|k| non_empty(&req_body[k]))
        .ok_or_else(|| bad("Provide one of `source_id`, `library_key`, `arxiv_id`, `doi`, `url`, or `input`."))?;
    let mut paper = parse_paper_input(raw_input).map_err(bad)?;
    if paper.version.is_none() { paper.version = opts.version; }
    // Rule 6.16: enforce the session fetch budget before spawning any network work.
    check_fetch_budget(&events, limit).map_err(|err| (429, err))?;

    let env = state.research_env_for_session(session_id).map_err(internal)?;
    let initial = OpenJobSnapshot::new(session_id, Some(paper.clone()), used, limit);
    let slot = register_job(initial.clone());
    let cfg = OpenWorkerConfig {
        data_dir: state.cfg.data_dir.clone(), env, session_id: session_id.to_string(),
        paper, question: opts.question, should_pin: opts.should_pin, gate,
    };
    thread::Builder::new().name(format!("investigate-open-{}", initial.job_id))
        .spawn(move || run_network_open_worker(cfg, slot))
        .map_err(|e| internal(format!("failed to spawn paper open worker: {e}")))?;
    Ok(initial)
}

/// Returns the latest snapshot of `job_id` only if it belongs to `session_id`.
pub fn get_open_job(session_id: &str, job_id: &str) -> Option<OpenJobSnapshot> {
    let slot = get_job_slot(session_id, job_id)?;
    let snap = slot.snapshot.lock().unwrap_or_else(|e| e.into_inner()).clone();
    Some(snap)
}

/// Waits up to `timeout` for `job_id` (scoped to `session_id`) to reach a terminal [`OpenStep`].
pub fn wait_for_open_job(session_id: &str, job_id: &str, timeout: Duration) -> Option<OpenJobSnapshot> {
    let slot = get_job_slot(session_id, job_id)?;
    let deadline = Instant::now() + timeout;
    let mut guard = slot.snapshot.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        if guard.step.is_terminal() || Instant::now() >= deadline { return Some(guard.clone()); }
        let rem = deadline.saturating_duration_since(Instant::now());
        let (next, res) = slot.cv.wait_timeout(guard, rem).unwrap_or_else(|e| e.into_inner());
        guard = next;
        if res.timed_out() { return Some(guard.clone()); }
    }
}

/// HTTP route dispatcher for `/api/sessions/{id}/investigation/{open|open-status}`.
pub fn handle(
    state: &DesktopState, method: &str, session_id: &str, op: &str,
    query: &HashMap<String, String>, body: &[u8],
) -> ApiResult {
    match (method, op) {
        ("POST", "open") => {
            let snap = start_open_job(state, session_id, &parse_json_body(body)?)?;
            Ok((if snap.step.is_terminal() { 200 } else { 202 }, json!({ "job_id": snap.job_id, "job": snap })))
        }
        ("GET", "open" | "open-status") => {
            validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
            let job_id = query.get("job_id").map(String::as_str).map(str::trim).filter(|s| !s.is_empty())
                .ok_or_else(|| bad("Provide `job_id` query parameter."))?;
            let snap = get_open_job(session_id, job_id)
                .ok_or_else(|| not_found(format!("open job `{job_id}` not found for session `{session_id}`")))?;
            Ok((200, json!({ "job": snap })))
        }
        _ => Err(not_found(format!("unknown investigation open endpoint: {method} /api/sessions/{session_id}/investigation/{op}"))),
    }
}

fn open_existing_source(
    state: &DesktopState, session_id: &str, events: &[Event], source_id: &str,
    req_body: &Value, mut opts: LocalOpenOpts,
) -> Result<OpenJobSnapshot, (u16, String)> {
    let receipt = find_source_receipt(events, source_id, None)
        .ok_or_else(|| not_found(format!("source `{source_id}` has not been ingested in session `{session_id}`")))?;
    opts.key_override = non_empty(&req_body["paper_key"]).map(PaperKey::parse).transpose().map_err(bad)?;
    opts.title = non_empty(&receipt.payload["title"]).map(str::to_string);
    let mut snap = OpenJobSnapshot::new(session_id, None, opts.used, opts.limit);
    snap.source_id = Some(source_id.to_string());
    snap.source_origin = non_empty(&receipt.payload["origin"]).map(str::to_string);
    finish_local_source(state, session_id, receipt, &opts, &mut snap, ("Paper skeleton ready.", "PDF ingested; PDF skeleton awaits V17"))?;
    let _ = register_job(snap.clone());
    Ok(snap)
}

fn open_from_library(
    state: &DesktopState, session_id: &str, raw_key: &str, mut opts: LocalOpenOpts,
) -> Result<OpenJobSnapshot, (u16, String)> {
    let parsed = PaperKey::parse(raw_key).map_err(bad)?;
    let lib = Library::open(&state.cfg.data_dir).map_err(internal)?;
    let canonical = lib.resolve_key(&parsed).map_err(internal)?.unwrap_or(parsed);
    let detail = lib.get_paper(&canonical).map_err(internal)?;
    let paper_ref = detail.as_ref().map(|d| d.paper.paper_ref(opts.version.clone()))
        .unwrap_or_else(|| paper_ref_from_key(canonical.clone(), opts.version.clone()));
    opts.title = detail.map(|d| d.paper.title).filter(|t| !t.trim().is_empty());

    let mut snap = OpenJobSnapshot::new(session_id, Some(paper_ref), opts.used, opts.limit);
    snap.source_origin = Some("library".to_string());
    snap.advance(OpenStep::LibraryLookup, "running", "Looking up paper in library.");
    let env = state.research_env_for_session(session_id).map_err(internal)?;
    let attach_res = {
        let ledger_mutex = Mutex::new(state.open_ledger().map_err(internal)?);
        let _lock = state.ledger_write_lock.lock().map_err(|_| internal("ledger lock poisoned"))?;
        lib.attach_to_session(&env.sandbox(), &ledger_mutex, session_id, &canonical, opts.version.as_deref(), None)
    };
    let ingested = match attach_res {
        Ok(ing) => ing,
        Err(err) if err.starts_with("no cached full-text blob") => {
            snap.needs_file_reason = Some(err);
            snap.advance(OpenStep::NeedsFile, "needs_file", "No cached full-text file in library; drop the PDF or TeX");
            let _ = register_job(snap.clone());
            return Ok(snap);
        }
        Err(err) => return Err(internal(err)),
    };

    let updated = state.open_ledger().map_err(internal)?.read_session(session_id).map_err(storage)?;
    snap.source_id = Some(ingested.source_id.clone());
    snap.fetch_budget_used = fetched_sources(&updated);
    snap.advance(OpenStep::Ingest, "running", "Ingested paper from library.");
    let receipt = find_source_receipt(&updated, &ingested.source_id, Some(ingested.event_seq))
        .or_else(|| find_source_receipt(&updated, &ingested.source_id, None))
        .ok_or_else(|| internal("library attach succeeded without a source.ingested receipt"))?;
    opts.key_override = Some(canonical);
    finish_local_source(
        state, session_id, receipt, &opts, &mut snap,
        ("Paper opened from library and skeleton ready.", "PDF ingested from library; PDF skeleton awaits V17"),
    )?;
    let _ = register_job(snap.clone());
    Ok(snap)
}

fn finish_local_source(
    state: &DesktopState, session_id: &str, receipt: &Event, opts: &LocalOpenOpts,
    snap: &mut OpenJobSnapshot, done_msgs: (&str, &str),
) -> Result<(), (u16, String)> {
    let source_id = snap.source_id.clone().ok_or_else(|| internal("missing source_id while finishing paper open"))?;
    if is_skeleton_receipt(receipt) {
        snap.advance(OpenStep::Skeleton, "running", "Building deterministic paper skeleton.");
        let rec = if opts.should_pin {
            pin_source_and_build_skeleton(state, session_id, &source_id, opts.key_override.clone(), opts.version.clone(), opts.question.as_deref())?
        } else {
            build_and_record_skeleton(state, session_id, &source_id, opts.key_override.clone(), opts.version.clone())?
        };
        let pref = snap.paper.clone()
            .unwrap_or_else(|| paper_ref_from_key(rec.skeleton.paper_key.clone(), rec.skeleton.version));
        snap.paper = Some(pref.clone());
        snap.skeleton_seq = Some(rec.event_seq);
        snap.skeleton_blob = Some(rec.blob_hash);
        snap.skeleton_timing_us = rec.timing.map(|t| t.total_us);
        record_investigated_in_library(&state.cfg.data_dir, session_id, &pref, None, opts.title.as_deref());
        snap.advance(OpenStep::Done, "done", done_msgs.0);
    } else {
        if opts.should_pin {
            let ledger = state.open_ledger().map_err(internal)?;
            let events = ledger.read_session(session_id).map_err(storage)?;
            let _lock = state.ledger_write_lock.lock().map_err(|_| internal("ledger lock poisoned"))?;
            pin_paper_event(&ledger, &events, session_id, &source_id, opts.question.as_deref(), true)?;
        }
        if snap.paper.is_none() {
            snap.paper = derive_paper_identity(receipt, opts.key_override.clone(), opts.version.clone())
                .ok().map(|(k, v)| paper_ref_from_key(k, v));
        }
        if let Some(ref pref) = snap.paper {
            record_investigated_in_library(&state.cfg.data_dir, session_id, pref, None, opts.title.as_deref());
        }
        snap.advance(OpenStep::Done, "done", done_msgs.1);
    }
    Ok(())
}

fn run_network_open_worker(cfg: OpenWorkerConfig, slot: Arc<OpenJobSlot>) {
    if let Err(err) = run_network_open_inner(&cfg, &slot) {
        update_slot(&slot, |s| { s.error = Some(err.clone()); s.advance(OpenStep::Failed, "failed", err); });
    }
}

fn run_network_open_inner(cfg: &OpenWorkerConfig, slot: &Arc<OpenJobSlot>) -> Result<(), String> {
    let gate_ref: &NetRateGate = match cfg.gate.as_deref() { Some(g) => g, None => global_gate() };

    // Step 1: non-fatal metadata lookup.
    update_slot(slot, |s| s.advance(OpenStep::Metadata, "running", "Fetching paper metadata."));
    let mut metadata: Option<PaperMetadata> = None;
    if let Ok(FetchOutcome::Metadata(meta)) =
        fetch_with_progress(&cfg.env, &cfg.paper, FetchWant::Metadata, gate_ref, &mut |_| {})
    {
        update_slot(slot, |s| { s.paper = Some(meta.paper.clone()); s.metadata = Some(meta.clone()); });
        metadata = Some(meta);
    }

    // Step 2: source lookup (cache -> arXiv TeX e-print -> arXiv PDF fallback).
    let slot_cb = Arc::clone(slot);
    let outcome = fetch_with_progress(&cfg.env, &cfg.paper, FetchWant::Source, gate_ref, &mut |fs| {
        let next = match fs {
            FetchStage::CheckingCache => Some((OpenStep::LibraryLookup, "Checking local paper library cache.")),
            FetchStage::FetchingEprint => Some((OpenStep::EprintTex, "Downloading arXiv TeX source.")),
            FetchStage::FetchingPdf => Some((OpenStep::PdfFallback, "Falling back to arXiv PDF.")),
            _ => None,
        };
        if let Some((step, msg)) = next { update_slot(&slot_cb, |s| s.advance(step, "running", msg)); }
    })?;

    let mut fetched = match outcome {
        FetchOutcome::Source(src) => src,
        FetchOutcome::NeedsFile { paper, reason } => {
            let why = if reason.trim().is_empty() { NEEDS_FILE_HINT.to_string() } else { reason };
            update_slot(slot, |s| {
                if s.metadata.is_none() { s.paper = Some(paper); }
                s.needs_file_reason = Some(why.clone());
                s.advance(OpenStep::NeedsFile, "needs_file", why);
            });
            return Ok(());
        }
        other => return Err(format!("unexpected fetch outcome: {}", other.status_str())),
    };
    if fetched.title.is_none() {
        fetched.title = metadata.as_ref().map(|m| m.title.clone()).filter(|t| !t.trim().is_empty());
    }

    // Step 3: ingest validated bytes into this session's own ledger.
    update_slot(slot, |s| s.advance(OpenStep::Ingest, "running", "Ingesting paper into session."));
    let ledger_mutex = Mutex::new(Ledger::open(&cfg.data_dir).map_err(|e| format!("failed to open ledger: {e}"))?);
    let ingested = fetched.ingest_into_session(&cfg.env.sandbox(), &ledger_mutex, &cfg.session_id, None)?;
    let ledger = ledger_mutex.into_inner().unwrap_or_else(|e| e.into_inner());
    record_investigated_in_library(&cfg.data_dir, &cfg.session_id, &fetched.paper, metadata.as_ref(), fetched.title.as_deref());

    let events = ledger.read_session(&cfg.session_id).map_err(|e| format!("failed to read session events: {e}"))?;
    let (budget_used, budget_limit, origin_str) =
        (fetched_sources(&events), effective_fetch_limit(&events), fetched.origin.as_str().to_string());
    update_slot(slot, |s| {
        s.paper = Some(fetched.paper.clone()); s.source_id = Some(ingested.source_id.clone());
        s.source_origin = Some(origin_str); s.fetch_budget_used = budget_used; s.fetch_budget_limit = budget_limit;
    });
    let receipt = find_source_receipt(&events, &ingested.source_id, Some(ingested.event_seq))
        .or_else(|| find_source_receipt(&events, &ingested.source_id, None))
        .ok_or_else(|| "ingested paper is missing its source.ingested receipt".to_string())?;

    // Step 4: pin primary paper and build deterministic skeleton for TeX and PDF sources.
    if is_skeleton_receipt(receipt) {
        update_slot(slot, |s| s.advance(OpenStep::Skeleton, "running", "Building deterministic paper skeleton."));
        let built = pin_and_build_for_env(cfg, &ledger, &events, &ingested.source_id, receipt, &fetched.paper)?;
        update_slot(slot, |s| {
            s.skeleton_seq = Some(built.event_seq); s.skeleton_blob = Some(built.blob_hash);
            s.skeleton_timing_us = built.timing_us;
            s.advance(OpenStep::Done, "done", "Paper opened and skeleton ready.");
        });
    } else {
        if cfg.should_pin {
            pin_paper_event(&ledger, &events, &cfg.session_id, &ingested.source_id, cfg.question.as_deref(), false)
                .map_err(|(_, msg)| msg)?;
        }
        update_slot(slot, |s| s.advance(OpenStep::Done, "done", "Source ingested."));
    }
    Ok(())
}

fn pin_and_build_for_env(
    cfg: &OpenWorkerConfig, ledger: &Ledger, events: &[Event],
    source_id: &str, receipt: &Event, paper: &PaperRef,
) -> Result<BuiltSkeletonSummary, String> {
    if !is_investigation(events) { return Err("Open an Investigation session before pinning a paper.".to_string()); }
    if cfg.should_pin {
        pin_paper_event(ledger, events, &cfg.session_id, source_id, cfg.question.as_deref(), true).map_err(|(_, m)| m)?;
    }
    if let Some((event_seq, blob_hash, bytes)) = latest_skeleton_for_source(events, source_id, receipt.seq)
        .and_then(|ev| ev.payload["blob"].as_str().map(|b| (ev.seq, b)))
        .and_then(|(s, b)| ledger.get_blob(b).ok().filter(|r| ledger::sha256_hex(r) == b).map(|r| (s, b, r)))
    {
        let _skel: PaperSkeleton = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        return Ok(BuiltSkeletonSummary { event_seq, blob_hash: blob_hash.to_string(), timing_us: None });
    }
    let (paper_key, version) = derive_paper_identity(receipt, Some(paper.key.clone()), paper.version.clone())?;
    let (skel, timing) =
        build_skeleton_from_receipt(&cfg.env.sandbox(), &cfg.session_id, receipt, paper_key, version)?;
    let blob_hash = ledger.put_blob(&serde_json::to_vec(&skel).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let payload = json!({
        "source_id": skel.source_id, "source_seq": skel.source_seq, "builder": skel.builder,
        "files_root_sha256": skel.files_root_sha256, "paper_key": skel.paper_key,
        "version": skel.version, "quality": skel.quality, "quality_reason": skel.quality_reason,
        "counts": {
            "nodes": skel.nodes.len(), "edges": skel.edges.len(), "bib": skel.bib.len(),
            "sections": skel.nodes.iter().filter(|n| n.kind == SkNodeKind::Section).count(),
            "results": skel.nodes.iter().filter(|n| n.kind.is_theorem_like()).count(),
            "equations": skel.nodes.iter().filter(|n| n.kind == SkNodeKind::Equation).count(),
        },
        "blob": blob_hash, "prev_skeleton_seq": Value::Null, "carry_counts": Value::Null,
    });
    let event_seq = ledger.append(&cfg.session_id, EVENT_INVESTIGATION_SKELETON, 1, &payload).map_err(|e| e.to_string())?;
    Ok(BuiltSkeletonSummary { event_seq, blob_hash, timing_us: Some(timing.total_us) })
}

fn pin_paper_event(
    ledger: &Ledger, events: &[Event], session_id: &str, source_id: &str,
    question: Option<&str>, error_on_conflict: bool,
) -> Result<(), (u16, String)> {
    if let Some(pinned) = latest(events, EVENT_INVESTIGATION_PAPER) {
        if error_on_conflict && pinned.payload["source_id"].as_str() != Some(source_id) {
            return Err(conflict("Start a new Investigation for a different primary paper."));
        }
        return Ok(());
    }
    let payload = json!({ "source_id": source_id, "question": question.map(str::trim).filter(|s| !s.is_empty()) });
    ledger.append(session_id, EVENT_INVESTIGATION_PAPER, 1, &payload).map_err(storage)?;
    Ok(())
}

fn record_investigated_in_library(
    data_dir: &Path, session_id: &str, paper: &PaperRef,
    metadata: Option<&PaperMetadata>, fallback_title: Option<&str>,
) {
    let Ok(lib) = Library::open(data_dir) else { return; };
    let ts = now_ms();
    if lib.get_paper(&paper.key).ok().flatten().is_none() {
        let title = metadata.map(|m| m.title.clone()).filter(|t| !t.trim().is_empty())
            .or_else(|| fallback_title.map(str::to_string)).unwrap_or_default();
        let year = metadata.and_then(|m| m.published.as_deref()).and_then(|s| s.get(..4)).and_then(|y| y.parse().ok());
        let flags = metadata.map(|m| PaperFlags {
            withdrawn: m.withdrawn, retracted: m.retracted, owned: false,
            license: m.licence.clone(), venue: m.journal_ref.clone(),
        }).unwrap_or_default();
        let _ = lib.upsert_paper(&PaperRecord {
            key: paper.key.clone(), ids: LibraryIds::from_paper_ids(&paper.ids), title,
            authors: metadata.map(|m| m.authors.clone()).unwrap_or_default(), year,
            abstract_text: metadata.map(|m| m.abstract_text.clone()).unwrap_or_default(),
            bibtex: String::new(), flags, first_seen_ms: ts, last_seen_ms: ts,
        });
    }
    let _ = lib.record_seen(&SeenRecord {
        key: paper.key.clone(), session_id: session_id.to_string(), role: SeenRole::Investigated, seen_ms: ts,
    });
}

fn paper_ref_from_key(key: PaperKey, version: Option<String>) -> PaperRef {
    let val = || Some(key.value().to_string());
    let ids = match key.kind() {
        PaperKeyKind::Arxiv => PaperIds { arxiv: val(), ..Default::default() },
        PaperKeyKind::Doi => PaperIds { doi: val(), ..Default::default() },
        PaperKeyKind::Inspire => PaperIds { inspire: val(), ..Default::default() },
        PaperKeyKind::S2 => PaperIds { s2: val(), ..Default::default() },
        PaperKeyKind::Openalex | PaperKeyKind::File => PaperIds::default(),
    };
    PaperRef { key, version, ids }
}

fn find_source_receipt<'a>(events: &'a [Event], sid: &str, seq: Option<u64>) -> Option<&'a Event> {
    events.iter().rev().find(|e| e.kind == EVENT_SOURCE_INGESTED && e.payload["source_id"].as_str() == Some(sid) && seq.is_none_or(|s| e.seq == s))
}
fn latest_skeleton_for_source<'a>(events: &'a [Event], sid: &str, seq: u64) -> Option<&'a Event> {
    events.iter().rev().find(|e| e.kind == EVENT_INVESTIGATION_SKELETON && e.payload["source_id"].as_str() == Some(sid) && e.payload["source_seq"].as_u64() == Some(seq))
}
fn non_empty(v: &Value) -> Option<&str> { v.as_str().map(str::trim).filter(|s| !s.is_empty()) }
fn storage(err: impl ToString) -> (u16, String) { internal(err.to_string()) }
