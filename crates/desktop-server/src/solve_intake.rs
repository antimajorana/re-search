//! Solve S17a TeX-only check-a-derivation intake route (`POST /api/sessions/{id}/solve/transcribe`).
//!
//! Deterministically splits a user-provided `.tex` derivation (`< 1` s, zero model calls),
//! pins the raw TeX bytes via `research_tools::ingest::ingest_bytes` (`source.ingested` + blob),
//! appends numbered `proof.entry` v1 steps (`origin: "given"`, `entry_type: "derivation"`,
//! consecutive `depends_on` pairs, `transcription: { source_tex_sha256 }`), and returns the
//! updated Solve view plus `given_source: { source_id, file_sha256 }`.
//! PDF derivations are rejected fail-closed (`00-overview.md` 2.12).

use crate::routes_session::{bad, conflict, internal, not_found, parse_json_body, ApiResult};
use crate::session_lifecycle::is_turn_running;
use crate::session_mode::session_mode;
use crate::solve::session_solve_view;
use crate::state::DesktopState;
use ledger::Event;
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::paper_key::Mode;
use research_tools::solve_derivation::{
    reject_if_pdf, split_tex_derivation_bytes, DerivationEquation, DerivationSplit,
    MAX_DERIVATION_TEX_BYTES,
};
use research_tools::solve_model::{
    compute_entry_sha256, event_kinds, SolveEntry, StepEntryType, StepOrigin,
};
use research_tools::solve_state::SolveState;
use research_tools::{validate_session_id, ResearchEnv};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

const MAX_TRANSCRIBE_BODY_BYTES: usize = MAX_DERIVATION_TEX_BYTES * 2;
const MAX_FILENAME_STEM_CHARS: usize = 120;

/// `POST /api/sessions/{id}/solve/transcribe`.
pub fn handle_transcribe(state: &DesktopState, session_id: &str, body: &[u8]) -> ApiResult {
    let started = Instant::now();
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    if !state.session_exists(session_id).map_err(internal)? {
        return Err(not_found(format!("session `{session_id}` not found")));
    }

    // Fail closed on raw binary `%PDF-` bodies or oversized JSON before allocating/parsing JSON.
    reject_if_pdf(None, body).map_err(bad)?;
    if body.len() > MAX_TRANSCRIBE_BODY_BYTES {
        return Err((
            413,
            format!("transcribe request exceeds {MAX_TRANSCRIBE_BODY_BYTES} byte limit"),
        ));
    }

    let val = parse_json_body(body)?;
    let raw_file_name = val
        .get("file_name")
        .or_else(|| val.get("filename"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let replace = val.get("replace").and_then(Value::as_bool).unwrap_or(false);
    let tex_str = val
        .get("tex")
        .or_else(|| val.get("content"))
        .or_else(|| val.get("source_tex"))
        .and_then(Value::as_str);

    // Check PDF rejection first (even if tex body is empty) so users always get the TeX-only error.
    reject_if_pdf(raw_file_name, tex_str.unwrap_or("").as_bytes()).map_err(bad)?;

    let tex = tex_str
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| bad("`tex` must be a non-empty LaTeX string"))?;
    if tex.len() > MAX_DERIVATION_TEX_BYTES {
        return Err((
            413,
            format!("TeX derivation exceeds maximum size of {MAX_DERIVATION_TEX_BYTES} bytes"),
        ));
    }

    let split = split_tex_derivation_bytes(raw_file_name, tex.as_bytes()).map_err(bad)?;
    let safe_file_name = sanitize_tex_file_name(raw_file_name);
    let origin = format!("solve_transcribe:{safe_file_name}");

    let _guard = state
        .ledger_write_lock
        .lock()
        .map_err(|_| internal("ledger write lock poisoned"))?;
    if is_turn_running(state, session_id) {
        return Err(conflict(
            "Cannot transcribe a derivation while a Solve turn is running.",
        ));
    }

    let ledger_mutex = Mutex::new(state.open_ledger().map_err(internal)?);
    let existing_events = {
        let ledger = ledger_mutex.lock().map_err(|_| internal("ledger lock poisoned"))?;
        let evs = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
        if session_mode(&evs) != Mode::Solve {
            return Err(bad("This endpoint is only available for Solve sessions."));
        }
        evs
    };

    let folded_before = SolveState::from_events(&existing_events);
    let active_transcribed: Vec<&SolveEntry> = folded_before
        .active_entries()
        .into_iter()
        .filter(|e| e.transcription.is_some())
        .collect();

    if !active_transcribed.is_empty() {
        let same_sequence = active_transcribed.len() == split.equations.len()
            && active_transcribed.iter().zip(split.equations.iter()).all(|(e, eq)| {
                e.transcription.as_ref().map(|t| t.source_tex_sha256.as_str())
                    == Some(eq.source_tex_sha256.as_str())
            });
        if same_sequence {
            let (source_id, raw_blob) = latest_transcribe_source(&existing_events);
            let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            drop(_guard);
            let view = session_solve_view(
                state,
                session_id,
                &existing_events,
                is_turn_running(state, session_id),
            );
            return Ok(build_idempotent_response(
                &active_transcribed,
                &split,
                source_id,
                raw_blob,
                elapsed_ms,
                view,
            ));
        }
        if !replace {
            return Err(conflict(
                "Session already has transcribed derivation steps; pass `replace: true` to replace them.",
            ));
        }
    }

    let old_transcribed_ids: Vec<String> =
        active_transcribed.iter().map(|e| e.entry_id.clone()).collect();

    // `ingest_bytes` writes solely into `SandboxRoot::Sources` under `data_dir`.
    let env = ResearchEnv::new(state.cfg.data_dir.clone());
    let ingested = ingest_bytes(
        &env.sandbox(),
        &ledger_mutex,
        &IngestRequest {
            session_id,
            file_name: &safe_file_name,
            origin: &origin,
            raw_bytes: tex.as_bytes(),
            source_id_hint: None,
            section_filter: None,
            title: None,
        },
    )
    .map_err(map_ingest_err)?;

    let ledger = ledger_mutex.lock().map_err(|_| internal("ledger lock poisoned"))?;
    let events_after_ingest = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
    let folded = SolveState::from_events(&events_after_ingest);
    let mut next_num = folded
        .entries
        .iter()
        .filter_map(|e| e.entry_id.strip_prefix("e-").and_then(|n| n.parse::<u64>().ok()))
        .max()
        .unwrap_or(0);

    let mut created_steps: Vec<Value> = Vec::with_capacity(split.equations.len());
    let mut created_ids: Vec<String> = Vec::with_capacity(split.equations.len());
    let mut prev_entry_id: Option<String> = None;

    for eq in &split.equations {
        next_num += 1;
        let entry_id = format!("e-{next_num}");
        let summary = format_step_summary(eq);
        let details = eq.raw_tex.clone();
        let depends_on: Vec<String> = prev_entry_id.iter().cloned().collect();
        let entry_sha256 = compute_entry_sha256(None, &summary, &details);

        let payload = json!({
            "entry_id": entry_id,
            "entry_type": StepEntryType::Derivation.as_str(),
            "summary": summary,
            "details": details,
            "depends_on": depends_on,
            "origin": StepOrigin::Given,
            "entry_sha256": entry_sha256,
            "transcription": { "source_tex_sha256": eq.source_tex_sha256 },
        });
        let seq = ledger
            .append_with_hash(session_id, event_kinds::PROOF_ENTRY, 1, &payload)
            .map_err(|e| internal(format!("failed to write transcribed proof.entry: {e}")))?
            .seq;

        created_steps.push(json!({
            "entry_id": entry_id,
            "seq": seq,
            "index": eq.index,
            "label": eq.label,
            "tex_label": eq.tex_label,
            "env": eq.env,
            "section": eq.section,
            "start_line": eq.start_line,
            "end_line": eq.end_line,
            "raw_tex": eq.raw_tex,
            "depends_on": depends_on,
            "entry_sha256": entry_sha256,
            "source_tex_sha256": eq.source_tex_sha256,
        }));
        created_ids.push(entry_id.clone());
        prev_entry_id = Some(entry_id);
    }

    if replace && !old_transcribed_ids.is_empty() {
        let anchor = created_ids.first().cloned().unwrap_or_default();
        for old_id in &old_transcribed_ids {
            let sup = json!({
                "entry_id": old_id,
                "replaced_by": anchor,
                "reason": format!("Replaced by new TeX derivation ({})", ingested.source_id),
            });
            ledger
                .append_with_hash(session_id, event_kinds::PROOF_SUPERSEDE, 1, &sup)
                .map_err(|e| internal(format!("failed to supersede prior transcribed step: {e}")))?;
        }
    }

    let updated_events = ledger.read_session(session_id).map_err(|e| internal(e.to_string()))?;
    drop(ledger);
    drop(_guard);

    let pairs = build_pair_values(&split, &created_ids);
    state.sse.emit(
        session_id,
        "solve_updated",
        json!({
            "session": session_id,
            "source_id": ingested.source_id,
            "equation_count": created_ids.len(),
            "step_ids": created_ids,
        }),
    );

    let view = session_solve_view(
        state,
        session_id,
        &updated_events,
        is_turn_running(state, session_id),
    );
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    Ok((
        200,
        json!({
            "ok": true,
            "idempotent": false,
            "source_id": ingested.source_id,
            "raw_blob": ingested.raw_blob,
            "raw_sha256": ingested.raw_blob,
            "normalized_sha256": split.source_sha256,
            "source_sha256": split.source_sha256,
            "given_source": {
                "source_id": ingested.source_id,
                "file_sha256": ingested.raw_blob,
            },
            "outline_style": split.outline_style,
            "equation_count": created_steps.len(),
            "elapsed_ms": elapsed_ms,
            "steps": created_steps,
            "pairs": pairs,
            "view": view,
        }),
    ))
}

fn build_pair_values(split: &DerivationSplit, step_ids: &[String]) -> Vec<Value> {
    split
        .pairs
        .iter()
        .filter_map(|p| {
            let from_id = step_ids.get(p.from_index.checked_sub(1)?)?;
            let to_id = step_ids.get(p.to_index.checked_sub(1)?)?;
            Some(json!({
                "pair_index": p.pair_index,
                "from_entry_id": from_id,
                "to_entry_id": to_id,
                "from_label": p.from_label,
                "to_label": p.to_label,
            }))
        })
        .collect()
}

fn latest_transcribe_source(events: &[Event]) -> (String, String) {
    for ev in events.iter().rev() {
        if ev.kind == "source.ingested"
            && ev.payload.get("origin").and_then(Value::as_str).is_some_and(|o| o.starts_with("solve_transcribe:"))
        {
            let sid = ev.payload.get("source_id").and_then(Value::as_str).unwrap_or("").to_string();
            let blob = ev.payload.get("raw_blob").and_then(Value::as_str).unwrap_or("").to_string();
            return (sid, blob);
        }
    }
    (String::new(), String::new())
}

fn build_idempotent_response(
    existing: &[&SolveEntry],
    split: &DerivationSplit,
    source_id: String,
    raw_blob: String,
    elapsed_ms: u64,
    view: Value,
) -> (u16, Value) {
    let ids: Vec<String> = existing.iter().map(|e| e.entry_id.clone()).collect();
    let steps: Vec<Value> = existing
        .iter()
        .zip(split.equations.iter())
        .map(|(e, eq)| {
            json!({
                "entry_id": e.entry_id,
                "seq": e.seq,
                "index": eq.index,
                "label": eq.label,
                "tex_label": eq.tex_label,
                "env": eq.env,
                "section": eq.section,
                "start_line": eq.start_line,
                "end_line": eq.end_line,
                "raw_tex": e.details,
                "depends_on": e.depends_on,
                "entry_sha256": e.entry_sha256,
                "source_tex_sha256": eq.source_tex_sha256,
            })
        })
        .collect();
    let pairs = build_pair_values(split, &ids);
    (
        200,
        json!({
            "ok": true,
            "idempotent": true,
            "source_id": source_id,
            "raw_blob": raw_blob,
            "raw_sha256": raw_blob,
            "normalized_sha256": split.source_sha256,
            "source_sha256": split.source_sha256,
            "given_source": { "source_id": source_id, "file_sha256": raw_blob },
            "outline_style": split.outline_style,
            "equation_count": steps.len(),
            "elapsed_ms": elapsed_ms,
            "steps": steps,
            "pairs": pairs,
            "view": view,
        }),
    )
}

fn map_ingest_err(err: String) -> (u16, String) {
    let lower = err.to_ascii_lowercase();
    if lower.contains("ledger")
        || lower.contains("sqlite")
        || lower.contains("blob")
        || lower.contains("permission denied")
        || lower.contains("no space")
        || lower.contains("i/o")
    {
        internal(err)
    } else {
        bad(err)
    }
}

fn format_step_summary(eq: &DerivationEquation) -> String {
    let lines = if eq.start_line == eq.end_line {
        format!("line {}", eq.start_line)
    } else {
        format!("lines {}-{}", eq.start_line, eq.end_line)
    };
    match eq.section.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(sec) => format!("Equation {} [{}] ({lines})", eq.label, sec),
        None => format!("Equation {} ({lines})", eq.label),
    }
}

fn is_windows_reserved_stem(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL"
            | "COM1" | "COM2" | "COM3" | "COM4" | "COM5" | "COM6" | "COM7" | "COM8" | "COM9"
            | "LPT1" | "LPT2" | "LPT3" | "LPT4" | "LPT5" | "LPT6" | "LPT7" | "LPT8" | "LPT9"
    )
}

fn sanitize_tex_file_name(raw: Option<&str>) -> String {
    let base = raw
        .and_then(|s| s.rsplit(['/', '\\']).next())
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "." && *s != "..")
        .unwrap_or("derivation.tex");
    let cleaned: String = base
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .collect();
    let trimmed = cleaned.trim_matches('.');
    if trimmed.is_empty() {
        return "derivation.tex".to_string();
    }
    let ext = Path::new(trimmed)
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| e.eq_ignore_ascii_case("tex") || e.eq_ignore_ascii_case("ltx"))
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_else(|| "tex".to_string());
    let raw_stem = Path::new(trimmed)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("derivation");
    let mut stem: String = raw_stem.chars().take(MAX_FILENAME_STEM_CHARS).collect();
    if is_windows_reserved_stem(&stem) {
        stem = format!("deriv_{stem}");
    }
    format!("{stem}.{ext}")
}
