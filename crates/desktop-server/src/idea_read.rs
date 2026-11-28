//! Explore reading queue orchestration and `"Attach PDF"` ingestion (package X9).
//!
//! Governance: `explore.md` 3, 4.1 (`PaperRead`), 4.2 (`idea.paper`, `source.ingested`),
//! 4.4, 5.2 (`idea_fetch_papers`), 6 (Rules C8, C9, P1, P3, P5, P6, S3).
//!
//! Every external fetch executes strictly through `Router::try_call("host:idea:<session>",
//! "idea_fetch_papers", ...)` on `build_idea_router` (Rule P5). User-attached PDFs enter the
//! session strictly via `ingest_bytes` / `ingest_pdf_pages` (Rule C9).

use crate::idea::{
    budget_snapshot, effective_budget, latest_statement_seq, next_stage, resolve_stages,
    IdeaStage, EVENT_IDEA_PAPER, EVENT_IDEA_TRIAGE,
};
use crate::idea_exchanges::{emit_stage, latest_idea_statement};
use crate::idea_view_coverage::fold_answers;
use agent::CancelToken;
use ledger::{Event, Ledger};
use research_tools::idea_fetch::{
    count_source_lines, derive_text_basis_from_receipt, ingest_uploaded_paper_bytes,
    text_basis_str, FetchProgressCallbacks, FetchedPaperRecord, UNAVAILABLE_PAPER_REASON,
};
use research_tools::idea_tools::IDEA_FETCH_PAPERS;
use research_tools::paper_key::PaperKey;
use research_tools::run_budget::ExploreStageState;
use research_tools::ResearchEnv;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use tools::canon::random_hex;
use tools::{CallOutcome, Router, EVENT_SCHEMA_VERSION};

/// SSE-only transient progress event kind for live per-paper reading-queue updates.
pub const SSE_IDEA_READ_PROGRESS: &str = "idea.read_progress";

/// `PaperRead` payload written into `idea.paper` ledger events (`explore.md` 4.1–4.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeaPaperEventPayload {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_seq: Option<u64>,
    pub origin: String,
    pub text_basis: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub lines_sent: usize,
    pub total_lines: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_blob: Option<String>,
}

/// Outcome of executing the Explore reading queue (fetch + ingest phase in X9).
#[derive(Debug, Clone)]
pub enum ReadQueueResult {
    Finished {
        papers: Vec<IdeaPaperEventPayload>,
        ready_count: usize,
        unavailable_count: usize,
    },
    Denied {
        reason: String,
    },
    StoppedAtCap {
        reason: String,
        papers: Vec<IdeaPaperEventPayload>,
    },
    Cancelled {
        papers: Vec<IdeaPaperEventPayload>,
    },
}

/// Resolve the active reading list for the current revision: `idea.answer.reading_list`
/// override takes precedence over `idea.triage.reading_list`, capped at `budget.max_papers`.
pub fn resolve_reading_list(events: &[Event]) -> Vec<PaperKey> {
    let stmt_seq = latest_statement_seq(events);
    let budget = effective_budget(events);
    if budget.max_papers == 0 {
        return Vec::new();
    }
    let triage_ev = events
        .iter()
        .rev()
        .find(|e| e.kind == EVENT_IDEA_TRIAGE && e.seq > stmt_seq);
    let triage_seq = triage_ev.map_or(0, |e| e.seq);
    let applied = fold_answers(events, stmt_seq, None, 0, triage_seq);
    let raw_keys: Vec<String> = if let Some(ov) = applied.reading_list_override {
        ov
    } else if let Some(ev) = triage_ev {
        ev.payload
            .get("reading_list")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for raw in raw_keys {
        if out.len() >= budget.max_papers {
            break;
        }
        if let Ok(k) = PaperKey::parse(&raw) {
            if seen.insert(k.clone()) {
                out.push(k);
            }
        }
    }
    out
}

/// Internal handoff between the X9 fetch phase and the X10 passage-reader phase.
pub(crate) enum FetchPhaseOutcome {
    Done {
        request_id: String,
        papers: Vec<IdeaPaperEventPayload>,
        ready_count: usize,
        unavailable_count: usize,
    },
    EarlyReturn(ReadQueueResult),
}

/// Execute the reading queue fetch phase strictly via `Router::try_call("host:idea:<session>",
/// "idea_fetch_papers", ...)` (Rule P5), recording `idea.paper` per paper and finishing the stage.
#[allow(clippy::too_many_arguments)]
pub fn run_reading_queue_with_router<F>(
    ledger: &Ledger,
    router: &Router,
    session_id: &str,
    explicit_keys: Option<Vec<PaperKey>>,
    cancel: Option<&CancelToken>,
    mut on_progress: F,
) -> Result<ReadQueueResult, String>
where
    F: FnMut(&str, Value),
{
    let phase = fetch_papers_phase(
        ledger,
        router,
        session_id,
        explicit_keys,
        cancel,
        &mut on_progress,
    )?;
    let (request_id, papers, ready_count, unavailable_count) = match phase {
        FetchPhaseOutcome::EarlyReturn(res) => return Ok(res),
        FetchPhaseOutcome::Done {
            request_id,
            papers,
            ready_count,
            unavailable_count,
        } => (request_id, papers, ready_count, unavailable_count),
    };
    let post_events = ledger.read_session(session_id).map_err(|e| e.to_string())?;
    let _ = budget_snapshot(&post_events, true);
    emit_stage(
        ledger,
        session_id,
        IdeaStage::Read,
        ExploreStageState::Finished,
        &request_id,
        None,
        &mut on_progress,
    )?;
    Ok(ReadQueueResult::Finished {
        papers,
        ready_count,
        unavailable_count,
    })
}

/// Shared fetch/ingest phase (Rule P1, Rule P5, Rule S3) used by both X9 fetch-only queue and
/// X10 full fetch + passage-reader stage.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fetch_papers_phase<F>(
    ledger: &Ledger,
    router: &Router,
    session_id: &str,
    explicit_keys: Option<Vec<PaperKey>>,
    cancel: Option<&CancelToken>,
    on_progress: &mut F,
) -> Result<FetchPhaseOutcome, String>
where
    F: FnMut(&str, Value),
{
    let initial_events = ledger.read_session(session_id).map_err(|e| e.to_string())?;
    let stmt = latest_idea_statement(&initial_events)?;
    let budget = effective_budget(&initial_events);

    if budget.max_papers == 0 {
        return Err(
            "Quick preset reads abstracts only (max_papers = 0); raise paper budget to read full papers"
                .to_string(),
        );
    }

    // Rule P1 & Rule S3: pass `running = false` so a crashed `Requested` Read stage converts to
    // `Interrupted` and can be retried (concurrent runs are blocked by ActiveExploreGuard).
    let stages = resolve_stages(&initial_events, false);
    let state_of = |t: IdeaStage| stages.iter().find(|r| r.stage == t).and_then(|r| r.stored_state);
    let next = next_stage(&initial_events);
    let allowed = next == Some(IdeaStage::Read)
        || matches!(
            state_of(IdeaStage::Read),
            Some(
                ExploreStageState::Finished
                    | ExploreStageState::Failed
                    | ExploreStageState::Interrupted
                    | ExploreStageState::StoppedAtCap
                    | ExploreStageState::Cancelled
            )
        );
    let has_triage = initial_events
        .iter()
        .any(|e| e.seq > stmt.seq && e.kind == EVENT_IDEA_TRIAGE);
    if !allowed
        || state_of(IdeaStage::Triage) != Some(ExploreStageState::Finished)
        || !has_triage
    {
        return Err(format!(
            "cannot run Read outside stage `read` (next stage is `{}`)",
            next.map_or("done", IdeaStage::as_str)
        ));
    }

    let keys = match explicit_keys {
        Some(list) if !list.is_empty() => {
            let mut dedup = Vec::new();
            let mut seen = BTreeSet::new();
            for k in list {
                if seen.insert(k.clone()) {
                    dedup.push(k);
                }
            }
            dedup
        }
        _ => resolve_reading_list(&initial_events),
    };
    if keys.is_empty() {
        return Err("reading list is empty; select at least one paper to read".to_string());
    }

    // Fold latest `idea.paper` per key in ledger order so a newly attached PDF overrides any
    // older terminal record and already-fetched pinned papers are reused without re-fetching.
    let mut existing_by_key: std::collections::BTreeMap<String, IdeaPaperEventPayload> =
        std::collections::BTreeMap::new();
    for ev in initial_events.iter().filter(|e| e.seq > stmt.seq && e.kind == EVENT_IDEA_PAPER) {
        if let Ok(rec) = serde_json::from_value::<IdeaPaperEventPayload>(ev.payload.clone()) {
            existing_by_key.insert(rec.key.clone(), rec);
        }
    }

    let request_id = format!("read-r{}-{}", stmt.rev, random_hex(4));
    emit_stage(ledger, session_id, IdeaStage::Read, ExploreStageState::Requested, &request_id, None, on_progress)?;

    for k in &keys {
        let already_terminal = existing_by_key.get(k.as_str()).is_some_and(|r| {
            r.source_id.is_some()
                && matches!(r.status.as_str(), "read_full" | "read_passages" | "nothing_relevant")
        });
        if !already_terminal {
            on_progress(
                SSE_IDEA_READ_PROGRESS,
                json!({ "key": k.as_str(), "row_state": "queued", "label": "queued" }),
            );
        }
    }

    let caller = format!("host:idea:{session_id}");
    let mut recorded: Vec<IdeaPaperEventPayload> = Vec::with_capacity(keys.len());
    let mut ready_count = 0usize;
    let mut unavailable_count = 0usize;

    for key in &keys {
        if cancel.is_some_and(CancelToken::is_cancelled) {
            emit_stage(ledger, session_id, IdeaStage::Read, ExploreStageState::Cancelled, &request_id, Some("Stopped by user"), on_progress)?;
            return Ok(FetchPhaseOutcome::EarlyReturn(ReadQueueResult::Cancelled { papers: recorded }));
        }
        if let Some(prev) = existing_by_key.get(key.as_str()) {
            if prev.source_id.is_some()
                && matches!(
                    prev.status.as_str(),
                    "read_full" | "read_passages" | "nothing_relevant" | "fetched"
                )
            {
                ready_count += 1;
                recorded.push(prev.clone());
                continue;
            }
        }

        let args = json!({ "keys": [key.as_str()] });
        let outcome = router
            .try_call(&caller, IDEA_FETCH_PAPERS, &args)
            .map_err(|e| format!("router ledger error during idea_fetch_papers: {e}"))?;

        if cancel.is_some_and(CancelToken::is_cancelled) {
            emit_stage(ledger, session_id, IdeaStage::Read, ExploreStageState::Cancelled, &request_id, Some("Stopped by user"), on_progress)?;
            return Ok(FetchPhaseOutcome::EarlyReturn(ReadQueueResult::Cancelled { papers: recorded }));
        }

        let tool_val = match outcome {
            CallOutcome::Ok(v) => v,
            CallOutcome::Denied(reason) => {
                emit_stage(ledger, session_id, IdeaStage::Read, ExploreStageState::Cancelled, &request_id, Some(&reason), on_progress)?;
                return Ok(FetchPhaseOutcome::EarlyReturn(ReadQueueResult::Denied { reason }));
            }
            CallOutcome::Error(err) => {
                let ev_payload = failed_paper_payload(key.as_str(), err);
                append_idea_paper(ledger, session_id, &ev_payload, on_progress)?;
                recorded.push(ev_payload);
                continue;
            }
        };

        let fetched_opt = tool_val
            .get("papers")
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
            .and_then(|v| serde_json::from_value::<FetchedPaperRecord>(v.clone()).ok());
        let Some(rec) = fetched_opt else {
            let ev_payload = failed_paper_payload(key.as_str(), "idea_fetch_papers returned malformed result".into());
            append_idea_paper(ledger, session_id, &ev_payload, on_progress)?;
            recorded.push(ev_payload);
            continue;
        };

        if rec.status == "stopped_at_cap" {
            let reason = rec.reason.unwrap_or_else(|| "Paper fetch budget cap reached".to_string());
            emit_stage(ledger, session_id, IdeaStage::Read, ExploreStageState::StoppedAtCap, &request_id, Some(&reason), on_progress)?;
            return Ok(FetchPhaseOutcome::EarlyReturn(ReadQueueResult::StoppedAtCap { reason, papers: recorded }));
        }

        let (paper_status, row_state) = match rec.status.as_str() {
            "ready" => { ready_count += 1; ("fetched", "fetched") }
            "unavailable" => { unavailable_count += 1; ("unavailable", "unavailable") }
            _ => ("failed", "failed"),
        };

        let ev_payload = IdeaPaperEventPayload {
            key: rec.key.clone(),
            version: rec.version,
            source_id: rec.source_id,
            source_seq: rec.source_seq,
            origin: rec.origin,
            text_basis: rec.text_basis,
            status: paper_status.to_string(),
            reason: rec.reason.or_else(|| {
                (paper_status == "unavailable").then(|| UNAVAILABLE_PAPER_REASON.to_string())
            }),
            lines_sent: 0,
            total_lines: rec.total_lines,
            raw_blob: rec.raw_blob,
        };
        append_idea_paper(ledger, session_id, &ev_payload, on_progress)?;
        on_progress(
            SSE_IDEA_READ_PROGRESS,
            json!({
                "key": ev_payload.key,
                "row_state": row_state,
                "origin": ev_payload.origin,
                "text_basis": ev_payload.text_basis,
                "total_lines": ev_payload.total_lines,
            }),
        );
        recorded.push(ev_payload);
    }

    if cancel.is_some_and(CancelToken::is_cancelled) {
        emit_stage(ledger, session_id, IdeaStage::Read, ExploreStageState::Cancelled, &request_id, Some("Stopped by user"), on_progress)?;
        return Ok(FetchPhaseOutcome::EarlyReturn(ReadQueueResult::Cancelled { papers: recorded }));
    }

    Ok(FetchPhaseOutcome::Done {
        request_id,
        papers: recorded,
        ready_count,
        unavailable_count,
    })
}

fn failed_paper_payload(key: &str, reason: String) -> IdeaPaperEventPayload {
    IdeaPaperEventPayload {
        key: key.to_string(),
        version: None,
        source_id: None,
        source_seq: None,
        origin: "none".into(),
        text_basis: "none".into(),
        status: "failed".into(),
        reason: Some(reason),
        lines_sent: 0,
        total_lines: 0,
        raw_blob: None,
    }
}

/// Prepare and ingest a user-supplied PDF/TeX file for `paper_key` without holding any
/// global ledger write lock across PDF extraction (Rule C9, HIGH #6).
#[allow(clippy::too_many_arguments)]
pub fn prepare_attached_pdf<F>(
    env: &ResearchEnv,
    ledger: &Ledger,
    session_id: &str,
    paper_key: &PaperKey,
    existing_source_id: Option<&str>,
    upload_file: Option<(&str, &[u8])>,
    mut on_progress: F,
) -> Result<IdeaPaperEventPayload, String>
where
    F: FnMut(&str, Value),
{
    if let Some((file_name, raw_bytes)) = upload_file {
        let ledger_mutex = Mutex::new(
            Ledger::open(&env.data_dir).map_err(|e| format!("cannot open ledger: {e}"))?,
        );
        let cancel_flag = AtomicBool::new(false);
        let progress_cell = std::cell::RefCell::new(&mut on_progress);
        let key_str = paper_key.to_string();
        let mut on_row = |state: &str, label: &str| {
            (progress_cell.borrow_mut())(
                SSE_IDEA_READ_PROGRESS,
                json!({ "key": key_str, "row_state": state, "label": label }),
            );
        };
        let key_str2 = paper_key.to_string();
        let mut on_page = |page: usize, total: usize| {
            (progress_cell.borrow_mut())(
                SSE_IDEA_READ_PROGRESS,
                json!({
                    "key": key_str2,
                    "row_state": "extracting",
                    "page": page,
                    "total_pages": total,
                    "label": format!("extracting text (page {page} of {total})"),
                }),
            );
        };
        let rec = {
            let mut cb = FetchProgressCallbacks {
                cancel: Some(&cancel_flag),
                on_row_state: &mut on_row,
                on_pdf_page: &mut on_page,
            };
            ingest_uploaded_paper_bytes(
                env,
                &ledger_mutex,
                session_id,
                paper_key,
                file_name,
                raw_bytes,
                None,
                &mut cb,
            )?
        };
        Ok(IdeaPaperEventPayload {
            key: rec.key,
            version: rec.version,
            source_id: rec.source_id,
            source_seq: rec.source_seq,
            origin: "upload".into(),
            text_basis: rec.text_basis,
            status: "fetched".into(),
            reason: rec.reason,
            lines_sent: 0,
            total_lines: rec.total_lines,
            raw_blob: rec.raw_blob,
        })
    } else if let Some(src_id) = existing_source_id.filter(|s| !s.trim().is_empty()) {
        let events = ledger.read_session(session_id).map_err(|e| e.to_string())?;
        let src_ev = events
            .iter()
            .rev()
            .find(|e| {
                e.kind == "source.ingested"
                    && e.payload.get("source_id").and_then(Value::as_str) == Some(src_id)
            })
            .ok_or_else(|| format!("uploaded source_id `{src_id}` not found in session `{session_id}`"))?;
        let fmt = src_ev.payload.get("format").and_then(Value::as_str).unwrap_or("pdf");
        let basis = derive_text_basis_from_receipt(Some(src_ev), fmt);
        let reading_path = src_ev
            .payload
            .get("assembled_path")
            .or_else(|| src_ev.payload.get("primary_path"))
            .and_then(Value::as_str);
        let total_lines = reading_path
            .and_then(|p| count_source_lines(&env.sandbox(), session_id, p).ok())
            .unwrap_or(0);
        Ok(IdeaPaperEventPayload {
            key: paper_key.to_string(),
            version: None,
            source_id: Some(src_id.to_string()),
            source_seq: Some(src_ev.seq),
            origin: "upload".into(),
            text_basis: text_basis_str(basis).to_string(),
            status: "fetched".into(),
            reason: None,
            lines_sent: 0,
            total_lines,
            raw_blob: src_ev.payload.get("raw_blob").and_then(Value::as_str).map(str::to_string),
        })
    } else {
        Err("attach_pdf requires either `source_id` or (`file_name` + `bytes_base64`)".into())
    }
}

/// Convenience wrapper for tests: prepares an attached PDF and appends its `idea.paper` event.
#[allow(clippy::too_many_arguments)]
pub fn attach_pdf_to_paper<F>(
    env: &ResearchEnv,
    ledger: &Ledger,
    session_id: &str,
    paper_key: &PaperKey,
    existing_source_id: Option<&str>,
    upload_file: Option<(&str, &[u8])>,
    mut on_progress: F,
) -> Result<IdeaPaperEventPayload, String>
where
    F: FnMut(&str, Value),
{
    let payload = prepare_attached_pdf(
        env, ledger, session_id, paper_key, existing_source_id, upload_file, &mut on_progress,
    )?;
    append_idea_paper(ledger, session_id, &payload, &mut on_progress)?;
    Ok(payload)
}

/// Append an `idea.paper` ledger event and forward it over SSE.
pub fn append_idea_paper<F>(
    ledger: &Ledger,
    session_id: &str,
    payload: &IdeaPaperEventPayload,
    on_progress: &mut F,
) -> Result<u64, String>
where
    F: FnMut(&str, Value),
{
    let val = serde_json::to_value(payload).map_err(|e| e.to_string())?;
    let seq = ledger
        .append(session_id, EVENT_IDEA_PAPER, EVENT_SCHEMA_VERSION, &val)
        .map_err(|e| format!("failed to append `{EVENT_IDEA_PAPER}`: {e}"))?;
    on_progress(EVENT_IDEA_PAPER, val);
    Ok(seq)
}
