//! Deterministic cited-result lookup, candidate picker, fetch-budget raise, and cross-paper
//! node inspection for Investigate (package V11).
//!
//! # Approval & model-call rules (`00-overview.md` §2.4; `investigate.md` §5.2–5.3, §6.9, §6.16)
//!
//! - **Zero model calls.** Pure deterministic resolution, F10 library reuse, F3 gated fetch,
//!   F10 skeleton caching, and strict Rule 6.9 locator matching.
//! - **User-initiated only:** `POST /api/sessions/{id}/investigation/cited` runs directly from
//!   a reader's click on a citation chip, candidate button, or budget-raise button.
//! - **Budget enforcement:** cache hits (session or F10 library) make zero network requests;
//!   cache misses enforce `paper_fetch::check_fetch_budget` before any network call.

use crate::investigation::is_investigation;
use crate::investigation_cited_support::{
    bib_json, build_cited_view_json, build_statement_json, enrich_candidates,
    find_session_source_for_key, find_source_receipt, format_short_ref,
    load_or_build_cached_skeleton, load_pinned_context, non_empty, paper_ref_from_key,
    read_session_events, record_cited_in_library, resolve_bib_to_paper_ref,
    resolve_external_source_identity, resolve_library_canonical, storage, InFlightFetchGuard,
    PinnedContext,
};
use crate::investigation_mapview::format_quality_line;
use crate::investigation_open::effective_fetch_limit;
use crate::investigation_skeleton::{derive_paper_identity, is_skeleton_receipt};
use crate::routes_session::{bad, internal, not_found, parse_json_body, ApiResult};
use crate::state::DesktopState;
use research_tools::library::Library;
use research_tools::net_rate::{global_gate, NetRateGate};
use research_tools::paper_fetch::{
    check_fetch_budget, fetch_with_gate, fetched_sources, FetchOutcome, FetchWant,
    DEFAULT_INVESTIGATE_FETCH_LIMIT,
};
use research_tools::paper_key::PaperRef;
use research_tools::skeleton::{
    detect_tex_counter_drift, flatten_tex_files, format_version_caveat, load_ingested_pdf_source,
    load_ingested_tex_source, locate_in_skeleton_with_context, scan_tex_stream, LocateContext,
    LocateStatus, SkeletonBuilder, CITED_STATUS_FOUND, CITED_STATUS_NO_TEX,
    CITED_STATUS_UNRESOLVED, NO_TEX_REASON_EMPTY_TEXT_LAYER, NO_TEX_REASON_NEEDS_FILE,
    PROVENANCE_OUTSIDE_PAPER,
};
use research_tools::validate_session_id;
use serde_json::{json, Value};
use std::sync::Mutex;

// Named constants for accepted owner decisions (`00-overview.md` §8, `investigate.md` §4.4, §6.16).
pub const EVENT_INVESTIGATION_CITED_LOOKUP: &str = "investigation.cited_lookup";
const EVENT_INVESTIGATION_NOTE: &str = "investigation.note";
const MAX_FETCH_LIMIT: usize = 256;
pub const ATTACH_FILE_PROMPT: &str =
    "Source is unavailable automatically; attach a PDF or TeX file for this cited paper.";

/// HTTP entry point for `POST /api/sessions/{id}/investigation/cited` using the global rate gate.
pub fn handle(state: &DesktopState, session_id: &str, body: &[u8]) -> ApiResult {
    handle_with_gate(state, session_id, body, global_gate())
}

/// Testable entry point using an injectable [`NetRateGate`] (fake clock, zero real sleeps).
pub fn handle_with_gate(
    state: &DesktopState,
    session_id: &str,
    body: &[u8],
    gate: &NetRateGate,
) -> ApiResult {
    validate_session_id(session_id).map_err(|e| bad(e.to_string()))?;
    let val = parse_json_body(body)?;
    let action = non_empty(&val["action"]).unwrap_or("lookup");
    match action {
        "lookup" => handle_lookup(state, session_id, &val, gate),
        "pick" => handle_pick(state, session_id, &val),
        "raise_budget" => handle_raise_budget(state, session_id, &val),
        "inspect_node" => handle_inspect_node(state, session_id, &val),
        other => Err(bad(format!(
            "unknown cited action `{other}`; expected `lookup`, `pick`, `raise_budget`, or `inspect_node`"
        ))),
    }
}

fn handle_lookup(
    state: &DesktopState,
    session_id: &str,
    val: &Value,
    gate: &NetRateGate,
) -> ApiResult {
    let (events, pinned) = load_pinned_context(state, session_id, val)?;
    let locator = val["locator"].as_str().map(str::trim).unwrap_or("");
    let limit = effective_fetch_limit(&events);
    let used = fetched_sources(&events);
    let lib = Library::open(&state.cfg.data_dir).map_err(internal)?;

    let Some(mut paper_ref) = resolve_bib_to_paper_ref(&pinned.bib, &lib)? else {
        let reason = "bibliography entry has no arXiv id, DOI, or matching local library paper";
        let seq = append_cited_event(
            state,
            session_id,
            &json!({
                "tag": pinned.tag, "bib_key": pinned.bib.key, "locator": locator,
                "resolved": Value::Null, "external_source_id": Value::Null,
                "external_tag": Value::Null, "status": CITED_STATUS_UNRESOLVED,
                "number_basis": Value::Null, "reason": reason, "picked_by_user": false,
            }),
        )?;
        return Ok((
            200,
            json!({
                "event_seq": seq, "status": CITED_STATUS_UNRESOLVED, "reason": reason,
                "tag": pinned.tag, "bib_key": pinned.bib.key, "locator": locator,
                "bib": bib_json(&pinned.bib), "resolved": Value::Null,
                "short_ref": pinned.bib.key, "provenance": PROVENANCE_OUTSIDE_PAPER,
                "attach_prompt": ATTACH_FILE_PROMPT, "return_ref": pinned.return_ref,
                "fetch_budget": { "used": used, "limit": limit },
            }),
        ));
    };

    let original_key = paper_ref.key.clone();
    let canonical_key = resolve_library_canonical(&lib, &paper_ref)?;
    if canonical_key != paper_ref.key {
        paper_ref.key = canonical_key.clone();
    }
    let env = state.research_env_for_session(session_id).map_err(internal)?;

    // 1) Reuse already-ingested session source, 2) attach from F10 library (zero network),
    // 3) atomically reserve fetch budget before network fetch via F3.
    let (ext_source_id, from_cache) = if let Some(existing_sid) =
        find_session_source_for_key(&events, EVENT_INVESTIGATION_CITED_LOOKUP, &canonical_key)
    {
        (existing_sid, true)
    } else if let Some((ft, _bytes)) = lib
        .read_fulltext_bytes(&canonical_key, paper_ref.version.as_deref())
        .map_err(internal)?
    {
        if paper_ref.version.is_none() {
            paper_ref.version = ft.version.clone();
        }
        let ledger_mutex = Mutex::new(state.open_ledger().map_err(internal)?);
        let _lock = state.ledger_write_lock.lock().map_err(|_| internal("ledger lock poisoned"))?;
        let ing = lib
            .attach_to_session(
                &env.sandbox(), &ledger_mutex, session_id, &canonical_key,
                paper_ref.version.as_deref(), None,
            )
            .map_err(internal)?;
        (ing.source_id, true)
    } else {
        if let Err(err) = check_fetch_budget(&events, limit) {
            let suggested = (limit + DEFAULT_INVESTIGATE_FETCH_LIMIT).min(MAX_FETCH_LIMIT);
            return Ok((429, json!({
                "error": err, "code": "fetch_budget_exceeded",
                "fetch_budget": { "used": used, "limit": limit, "suggested_limit": suggested },
            })));
        }
        let _in_flight = match InFlightFetchGuard::try_reserve(session_id, used, limit) {
            Ok(guard) => guard,
            Err(effective_used) => {
                let suggested = (limit + DEFAULT_INVESTIGATE_FETCH_LIMIT).min(MAX_FETCH_LIMIT);
                return Ok((429, json!({
                    "error": format!("session fetch limit reached ({effective_used} of {limit} fetched sources); raise the limit explicitly to fetch more"),
                    "code": "fetch_budget_exceeded",
                    "fetch_budget": { "used": effective_used, "limit": limit, "suggested_limit": suggested },
                })));
            }
        };
        match fetch_with_gate(&env, &paper_ref, FetchWant::Source, gate).map_err(internal)? {
            FetchOutcome::Source(fetched) => {
                paper_ref = fetched.paper.clone();
                if original_key != paper_ref.key {
                    let _ = lib.put_alias(&original_key, &paper_ref.key);
                }
                let ledger_mutex = Mutex::new(state.open_ledger().map_err(internal)?);
                let _lock = state.ledger_write_lock.lock().map_err(|_| internal("ledger lock poisoned"))?;
                let ing = fetched
                    .ingest_into_session(&env.sandbox(), &ledger_mutex, session_id, None)
                    .map_err(internal)?;
                (ing.source_id, false)
            }
            FetchOutcome::NeedsFile { paper, reason } => {
                return record_and_return_no_tex(
                    state, session_id, &pinned, locator, &paper, None,
                    NO_TEX_REASON_NEEDS_FILE, &reason, (used, limit), false,
                );
            }
            other => return Err(internal(format!("unexpected fetch outcome: {}", other.status_str()))),
        }
    };

    let updated_events = read_session_events(state, session_id)?;
    let budget_after = (fetched_sources(&updated_events), effective_fetch_limit(&updated_events));
    let receipt = find_source_receipt(&updated_events, &ext_source_id)
        .ok_or_else(|| internal(format!("ingested external source `{ext_source_id}` missing receipt")))?;

    if receipt.payload["status"].as_str() == Some(NO_TEX_REASON_EMPTY_TEXT_LAYER) {
        return record_and_return_no_tex(
            state, session_id, &pinned, locator, &paper_ref, Some(&ext_source_id),
            NO_TEX_REASON_EMPTY_TEXT_LAYER, "PDF has no extractable text layer (image-only scan).",
            budget_after, from_cache,
        );
    }
    if !is_skeleton_receipt(receipt) {
        return record_and_return_no_tex(
            state, session_id, &pinned, locator, &paper_ref, Some(&ext_source_id),
            NO_TEX_REASON_NEEDS_FILE, "Ingested source is neither TeX nor a page-mapped PDF.",
            budget_after, from_cache,
        );
    }

    let (derived_key, derived_ver) =
        derive_paper_identity(receipt, Some(paper_ref.key.clone()), paper_ref.version.clone())
            .map_err(internal)?;
    if paper_ref.version.is_none() { paper_ref.version = derived_ver; }
    record_cited_in_library(&lib, session_id, &paper_ref, pinned.bib.title.as_deref());

    let (cited_skel, skel_blob, skeleton_cache_hit) = load_or_build_cached_skeleton(
        state, &lib, session_id, receipt, &derived_key, paper_ref.version.as_deref(),
    )?;

    let pdf_loaded = if cited_skel.builder == SkeletonBuilder::Pdf1 {
        Some(load_ingested_pdf_source(&env.sandbox(), session_id, receipt).map_err(internal)?)
    } else {
        None
    };
    let (drift_reason, single_counter_confirmed) = if cited_skel.builder == SkeletonBuilder::Tex1 {
        let tex = load_ingested_tex_source(&env.sandbox(), session_id, receipt).map_err(internal)?;
        match flatten_tex_files(&tex.files, tex.primary_path.as_deref()) {
            Some(flat) => {
                let table = scan_tex_stream(&flat).theorem_table;
                let drift = detect_tex_counter_drift(&tex.files, Some(&table), &cited_skel.nodes);
                let confirmed = drift.is_none();
                (drift, confirmed)
            }
            None => (Some("unable to flatten TeX source to confirm theorem counters".into()), false),
        }
    } else {
        (None, false)
    };

    let loc_res = locate_in_skeleton_with_context(
        &cited_skel,
        locator,
        &LocateContext {
            page_map: pdf_loaded.as_ref().map(|p| &p.page_map),
            page_lines: None,
            counter_drift_reason: drift_reason.as_deref(),
            single_counter_confirmed,
        },
    );

    if let LocateStatus::NoTex { reason } = &loc_res.outcome {
        return record_and_return_no_tex(
            state, session_id, &pinned, locator, &paper_ref, Some(&ext_source_id),
            reason, "PDF has no extractable text layer (image-only scan).",
            budget_after, from_cache,
        );
    }

    let (ext_tag, num_basis, reason, statement, candidates) = match &loc_res.outcome {
        LocateStatus::Found { tag, number_basis } => {
            let stmt = build_statement_json(
                &env.sandbox(), session_id, receipt, &cited_skel, tag, pdf_loaded.as_ref(),
            )?;
            (Some(tag.clone()), Some(*number_basis), None, Some(stmt), Vec::new())
        }
        LocateStatus::Candidates { candidates, reason }
        | LocateStatus::PaperFoundLocatorNotFound { candidates, reason } => {
            let enriched =
                enrich_candidates(&env.sandbox(), session_id, receipt, &cited_skel, candidates);
            (None, None, Some(reason.clone()), None, enriched)
        }
        LocateStatus::NoTex { .. } | LocateStatus::Unresolved { .. } => {
            return Err(internal("unexpected locate status for built skeleton"));
        }
    };

    let status_str = loc_res.outcome.status_str();
    let seq = append_cited_event(
        state,
        session_id,
        &json!({
            "tag": pinned.tag, "bib_key": pinned.bib.key, "locator": locator,
            "resolved": paper_ref, "external_source_id": ext_source_id,
            "external_tag": ext_tag, "status": status_str, "number_basis": num_basis,
            "reason": reason, "builder": cited_skel.builder, "from_cache": from_cache,
            "skeleton_cache_hit": skeleton_cache_hit, "picked_by_user": false,
        }),
    )?;

    let cited_view = build_cited_view_json(
        &cited_skel,
        &skel_blob,
        &ext_source_id,
        ext_tag.as_deref(),
        pdf_loaded.as_ref(),
    );
    Ok((
        200,
        json!({
            "event_seq": seq, "status": status_str, "reason": reason,
            "tag": pinned.tag, "bib_key": pinned.bib.key, "locator": locator,
            "bib": bib_json(&pinned.bib), "resolved": paper_ref,
            "short_ref": format_short_ref(&pinned.bib, &paper_ref),
            "provenance": PROVENANCE_OUTSIDE_PAPER,
            "external_source_id": ext_source_id, "external_tag": ext_tag,
            "number_basis": num_basis, "builder": cited_skel.builder,
            "quality": cited_skel.quality, "quality_line": format_quality_line(&cited_skel),
            "version_caveat": loc_res.version_caveat, "picked_by_user": false,
            "from_cache": from_cache, "skeleton_cache_hit": skeleton_cache_hit,
            "statement": statement, "candidates": candidates, "cited_view": cited_view,
            "return_ref": pinned.return_ref,
            "fetch_budget": { "used": budget_after.0, "limit": budget_after.1 },
        }),
    ))
}

fn handle_pick(state: &DesktopState, session_id: &str, val: &Value) -> ApiResult {
    let (events, pinned) = load_pinned_context(state, session_id, val)?;
    let locator = val["locator"].as_str().map(str::trim).unwrap_or("");
    let ext_source_id = non_empty(&val["external_source_id"])
        .ok_or_else(|| bad("Provide `external_source_id` for `pick`."))?;
    let ext_tag =
        non_empty(&val["external_tag"]).ok_or_else(|| bad("Provide `external_tag` for `pick`."))?;

    if ext_source_id == pinned.primary_source_id {
        return Err(bad("`external_source_id` must name an external cited source, not the primary paper."));
    }
    let receipt = find_source_receipt(&events, ext_source_id)
        .ok_or_else(|| not_found(format!("external source `{ext_source_id}` not found in session")))?;
    if receipt.payload["status"].as_str() == Some(NO_TEX_REASON_EMPTY_TEXT_LAYER)
        || !is_skeleton_receipt(receipt)
    {
        return Err(bad("selected external source has no deterministic skeleton"));
    }
    let (key, ver) =
        resolve_external_source_identity(&events, EVENT_INVESTIGATION_CITED_LOOKUP, receipt)?;
    let lib = Library::open(&state.cfg.data_dir).map_err(internal)?;
    let (cited_skel, skel_blob, skeleton_cache_hit) =
        load_or_build_cached_skeleton(state, &lib, session_id, receipt, &key, ver.as_deref())?;
    let picked_node = cited_skel
        .nodes
        .iter()
        .find(|n| n.tag.as_deref() == Some(ext_tag))
        .ok_or_else(|| not_found(format!("external node `{ext_tag}` not found in cited skeleton")))?;

    let env = state.research_env_for_session(session_id).map_err(internal)?;
    let pdf_loaded = if cited_skel.builder == SkeletonBuilder::Pdf1 {
        Some(load_ingested_pdf_source(&env.sandbox(), session_id, receipt).map_err(internal)?)
    } else {
        None
    };
    let statement = build_statement_json(
        &env.sandbox(), session_id, receipt, &cited_skel, ext_tag, pdf_loaded.as_ref(),
    )?;
    let paper_ref = paper_ref_from_key(cited_skel.paper_key.clone(), cited_skel.version.clone());
    let seq = append_cited_event(
        state,
        session_id,
        &json!({
            "tag": pinned.tag, "bib_key": pinned.bib.key, "locator": locator,
            "resolved": paper_ref, "external_source_id": ext_source_id,
            "external_tag": ext_tag, "status": CITED_STATUS_FOUND,
            "number_basis": picked_node.number_basis, "builder": cited_skel.builder,
            "picked_by_user": true,
        }),
    )?;
    let budget = (fetched_sources(&events), effective_fetch_limit(&events));
    let cited_view = build_cited_view_json(
        &cited_skel, &skel_blob, ext_source_id, Some(ext_tag), pdf_loaded.as_ref(),
    );
    Ok((
        200,
        json!({
            "event_seq": seq, "status": CITED_STATUS_FOUND, "tag": pinned.tag,
            "bib_key": pinned.bib.key, "locator": locator, "bib": bib_json(&pinned.bib),
            "resolved": paper_ref, "short_ref": format_short_ref(&pinned.bib, &paper_ref),
            "provenance": PROVENANCE_OUTSIDE_PAPER,
            "external_source_id": ext_source_id, "external_tag": ext_tag,
            "number_basis": picked_node.number_basis, "builder": cited_skel.builder,
            "quality": cited_skel.quality, "quality_line": format_quality_line(&cited_skel),
            "version_caveat": format_version_caveat(&cited_skel),
            "picked_by_user": true, "from_cache": true,
            "skeleton_cache_hit": skeleton_cache_hit,
            "statement": statement, "candidates": [], "cited_view": cited_view,
            "return_ref": pinned.return_ref,
            "fetch_budget": { "used": budget.0, "limit": budget.1 },
        }),
    ))
}

fn handle_raise_budget(state: &DesktopState, session_id: &str, val: &Value) -> ApiResult {
    let explicit_limit = match val.get("new_limit") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let n = v
                .as_u64()
                .ok_or_else(|| bad("`new_limit` must be a positive integer."))?;
            Some(usize::try_from(n).map_err(|_| bad("`new_limit` is out of range"))?)
        }
    };
    let _lock = state
        .ledger_write_lock
        .lock()
        .map_err(|_| internal("ledger lock poisoned"))?;
    let ledger = state.open_ledger().map_err(internal)?;
    let events = ledger.read_session(session_id).map_err(storage)?;
    if !is_investigation(&events) {
        return Err(bad("Open an Investigation session before raising its fetch budget."));
    }
    let current_limit = effective_fetch_limit(&events);
    let used = fetched_sources(&events);
    let new_limit = explicit_limit.unwrap_or_else(|| {
        current_limit
            .saturating_add(DEFAULT_INVESTIGATE_FETCH_LIMIT)
            .min(MAX_FETCH_LIMIT)
    });
    if new_limit <= current_limit || new_limit > MAX_FETCH_LIMIT {
        return Err(bad(format!(
            "`new_limit` must be greater than current limit ({current_limit}) and at most {MAX_FETCH_LIMIT}."
        )));
    }
    let payload = json!({
        "tag": non_empty(&val["tag"]),
        "fetch_limit": new_limit,
        "previous_limit": current_limit,
        "reason": "user_raised_fetch_budget",
        "text": format!("Raised external paper fetch limit from {current_limit} to {new_limit}."),
    });
    let seq = ledger
        .append(session_id, EVENT_INVESTIGATION_NOTE, 1, &payload)
        .map_err(storage)?;
    Ok((
        200,
        json!({
            "event_seq": seq,
            "fetch_budget": { "used": used, "limit": new_limit, "previous_limit": current_limit },
        }),
    ))
}

fn handle_inspect_node(state: &DesktopState, session_id: &str, val: &Value) -> ApiResult {
    let events = read_session_events(state, session_id)?;
    if !is_investigation(&events) {
        return Err(bad("Open an Investigation session first."));
    }
    let ext_source_id = non_empty(&val["external_source_id"])
        .ok_or_else(|| bad("Provide `external_source_id`."))?;
    let ext_tag =
        non_empty(&val["external_tag"]).ok_or_else(|| bad("Provide `external_tag`."))?;
    let receipt = find_source_receipt(&events, ext_source_id)
        .ok_or_else(|| not_found(format!("external source `{ext_source_id}` not found in session")))?;
    if receipt.payload["status"].as_str() == Some(NO_TEX_REASON_EMPTY_TEXT_LAYER)
        || !is_skeleton_receipt(receipt)
    {
        return Err(bad("external source has no deterministic skeleton"));
    }
    let (key, ver) =
        resolve_external_source_identity(&events, EVENT_INVESTIGATION_CITED_LOOKUP, receipt)?;
    let lib = Library::open(&state.cfg.data_dir).map_err(internal)?;
    let (cited_skel, _, _) =
        load_or_build_cached_skeleton(state, &lib, session_id, receipt, &key, ver.as_deref())?;
    let env = state.research_env_for_session(session_id).map_err(internal)?;
    let pdf_loaded = if cited_skel.builder == SkeletonBuilder::Pdf1 {
        Some(load_ingested_pdf_source(&env.sandbox(), session_id, receipt).map_err(internal)?)
    } else {
        None
    };
    let node = build_statement_json(
        &env.sandbox(), session_id, receipt, &cited_skel, ext_tag, pdf_loaded.as_ref(),
    )?;
    Ok((
        200,
        json!({
            "external_source_id": ext_source_id,
            "selected_tag": ext_tag,
            "statement": node,
            "node": node,
        }),
    ))
}

#[allow(clippy::too_many_arguments)]
fn record_and_return_no_tex(
    state: &DesktopState,
    session_id: &str,
    pinned: &PinnedContext,
    locator: &str,
    paper_ref: &PaperRef,
    ext_source_id: Option<&str>,
    reason: &str,
    message: &str,
    budget: (usize, usize),
    from_cache: bool,
) -> ApiResult {
    let seq = append_cited_event(
        state,
        session_id,
        &json!({
            "tag": pinned.tag, "bib_key": pinned.bib.key, "locator": locator,
            "resolved": paper_ref, "external_source_id": ext_source_id,
            "external_tag": Value::Null, "status": CITED_STATUS_NO_TEX,
            "number_basis": Value::Null, "reason": reason, "message": message,
            "from_cache": from_cache, "picked_by_user": false,
        }),
    )?;
    Ok((
        200,
        json!({
            "event_seq": seq, "status": CITED_STATUS_NO_TEX, "reason": reason, "message": message,
            "tag": pinned.tag, "bib_key": pinned.bib.key, "locator": locator,
            "bib": bib_json(&pinned.bib), "resolved": paper_ref,
            "short_ref": format_short_ref(&pinned.bib, paper_ref),
            "provenance": PROVENANCE_OUTSIDE_PAPER, "external_source_id": ext_source_id,
            "attach_prompt": ATTACH_FILE_PROMPT, "return_ref": pinned.return_ref,
            "from_cache": from_cache, "fetch_budget": { "used": budget.0, "limit": budget.1 },
        }),
    ))
}

fn append_cited_event(
    state: &DesktopState,
    session_id: &str,
    payload: &Value,
) -> Result<u64, (u16, String)> {
    let ledger = state.open_ledger().map_err(internal)?;
    let _lock = state
        .ledger_write_lock
        .lock()
        .map_err(|_| internal("ledger lock poisoned"))?;
    ledger
        .append(session_id, EVENT_INVESTIGATION_CITED_LOOKUP, 1, payload)
        .map_err(storage)
}

#[cfg(test)]
#[path = "investigation_cited_tests.rs"]
mod tests;
