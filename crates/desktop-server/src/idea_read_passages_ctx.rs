//! Prompt builders, SSE emission, and F10 library `seen`/`finding` helpers for X10 passage readers.

use crate::idea::{latest_statement_seq, EVENT_IDEA_BRIEF};
use crate::idea_exchanges::{latest_idea_statement, wrap_untrusted_data};
use crate::idea_view_coverage::fold_answers;
use crate::state::now_ms;
use ledger::Event;
use research_tools::idea_evidence::{SavedEvidenceItem, RECORDER_PAPER_EVIDENCE};
use research_tools::idea_terms::extract as extract_idea_terms;
use research_tools::library::{
    FindingRecord, Library, LibraryIds, PaperFlags, PaperRecord, SeenRecord, SeenRole,
};
use research_tools::paper_key::PaperKey;
use research_tools::quote_core::EVIDENCE_QUOTE_EVENT;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Clone)]
pub(crate) struct BriefReaderContext {
    pub stmt_seq: u64,
    pub restatement: String,
    pub subclaims: Vec<(String, String)>,
    pub terms: Vec<String>,
}

pub(crate) fn extract_brief_reader_context(events: &[Event]) -> Result<BriefReaderContext, String> {
    let stmt = latest_idea_statement(events)?;
    let stmt_seq = latest_statement_seq(events);
    let brief_ev = events
        .iter()
        .rev()
        .find(|e| e.kind == EVENT_IDEA_BRIEF && e.seq > stmt_seq);
    let mut terms: Vec<String> = Vec::new();
    let mut seen = BTreeSet::new();
    let mut push_term = |s: &str| {
        let trimmed = s.trim();
        if trimmed.len() >= 2 && seen.insert(trimmed.to_ascii_lowercase()) {
            terms.push(trimmed.to_string());
        }
    };

    let (restatement, subclaims) = if let Some(ev) = brief_ev {
        let applied = fold_answers(events, stmt_seq, Some(&ev.payload), ev.seq, 0);
        let body = applied.sharpened_idea.as_ref().unwrap_or(&ev.payload);
        let restate = body["restatement"].as_str().unwrap_or("").to_string();
        let scs: Vec<(String, String)> = body["subclaims"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| Some((s["id"].as_str()?.to_string(), s["text"].as_str()?.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        if let Some(arr) = body["terms"].as_array() {
            for t in arr {
                if let Some(main) = t["term"].as_str() { push_term(main); }
                if let Some(aliases) = t["aliases"].as_array() {
                    for a in aliases.iter().filter_map(Value::as_str) { push_term(a); }
                }
            }
        }
        (restate, scs)
    } else {
        (stmt.text.clone(), Vec::new())
    };

    for t in extract_idea_terms(&stmt.text).terms {
        push_term(&t.text);
    }

    Ok(BriefReaderContext { stmt_seq, restatement, subclaims, terms })
}

pub(crate) fn build_initial_reader_prompt(
    brief: &BriefReaderContext,
    paper_key: &str,
    outline_text: &str,
    passages: &str,
    is_full_file: bool,
    max_followups: usize,
) -> String {
    let sc_block = brief.subclaims.iter().map(|(id, txt)| format!("- {id}: {txt}")).collect::<Vec<_>>().join("\n");
    let wrapped_brief = wrap_untrusted_data("IDEA_BRIEF", &format!("Restatement: {}\nSubclaims:\n{sc_block}", brief.restatement));
    let wrapped_paper = wrap_untrusted_data("PAPER_TEXT", &format!("Paper: {paper_key}\n\nOutline:\n{outline_text}\n\nSupplied lines:\n{passages}"));
    let followup_note = if is_full_file || max_followups == 0 {
        "The full file is supplied above; set `want_lines` to `[]`.".to_string()
    } else {
        format!("You may request up to {max_followups} follow-up batch(es) via `want_lines` (<= 3 windows, <= 400 lines).")
    };
    format!(
        "Read paper `{paper_key}` and call `{RECORDER_PAPER_EVIDENCE}` with verbatim quotes from the supplied lines.\n\
         {followup_note}\n\n{wrapped_brief}\n\n{wrapped_paper}"
    )
}

pub(crate) fn build_followup_reader_prompt(
    brief: &BriefReaderContext,
    paper_key: &str,
    existing_ids: &[String],
    passages: &str,
    followups_remaining: usize,
) -> String {
    let sc_block = brief.subclaims.iter().map(|(id, txt)| format!("- {id}: {txt}")).collect::<Vec<_>>().join("\n");
    let wrapped_brief = wrap_untrusted_data("IDEA_BRIEF", &format!("Restatement: {}\nSubclaims:\n{sc_block}", brief.restatement));
    let wrapped_paper = wrap_untrusted_data("PAPER_TEXT", &format!("Paper: {paper_key}\n\nFollow-up supplied lines:\n{passages}"));
    let already = if existing_ids.is_empty() { "none yet".to_string() } else { existing_ids.join(", ") };
    let followup_note = if followups_remaining == 0 {
        "This is your final follow-up batch; set `want_lines` to `[]` and finish.".to_string()
    } else {
        format!("At most {followups_remaining} more follow-up batch(es) remain.")
    };
    format!(
        "Follow-up lines for `{paper_key}` (already saved quotes: {already}). Call `{RECORDER_PAPER_EVIDENCE}` with any additional verbatim quotes.\n\
         {followup_note}\n\n{wrapped_brief}\n\n{wrapped_paper}"
    )
}

pub(crate) fn emit_quote_sse<F>(on_progress: &F, item: &SavedEvidenceItem, fallback_basis: &str)
where
    F: Fn(&str, Value) + Sync,
{
    let eff_basis = item.page_basis.map(|b| b.as_str()).unwrap_or(fallback_basis);
    on_progress(
        EVIDENCE_QUOTE_EVENT,
        json!({
            "id": item.id,
            "seq": item.seq,
            "paper_key": item.payload.paper_key,
            "source_id": item.payload.source_id,
            "path": item.payload.path,
            "start_line": item.payload.start_line,
            "end_line": item.payload.end_line,
            "quote": item.payload.quote,
            "excerpt": item.payload.excerpt,
            "role": item.payload.role,
            "gist": item.payload.gist,
            "subclaim_ids": item.payload.subclaim_ids,
            "page": item.page,
            "end_page": item.end_page,
            "text_basis": eff_basis,
            "ocr_tag": item.ocr_tag,
        }),
    );
}

pub(crate) fn record_library_read_and_findings(
    data_dir: &Path,
    session_id: &str,
    paper_key: &PaperKey,
    saved: &[SavedEvidenceItem],
) {
    let Ok(lib) = Library::open(data_dir) else { return };
    let now = now_ms();
    let canonical = lib.resolve_key(paper_key).ok().flatten().unwrap_or_else(|| paper_key.clone());
    if lib.get_paper(&canonical).ok().flatten().is_none() {
        let _ = lib.upsert_paper(&PaperRecord {
            key: canonical.clone(),
            ids: LibraryIds::default(),
            title: String::new(),
            authors: Vec::new(),
            year: None,
            abstract_text: String::new(),
            bibtex: String::new(),
            flags: PaperFlags::default(),
            first_seen_ms: now,
            last_seen_ms: now,
        });
    }
    let _ = lib.record_seen(&SeenRecord {
        key: canonical.clone(),
        session_id: session_id.to_string(),
        role: SeenRole::Read,
        seen_ms: now,
    });
    for item in saved {
        let role = item
            .payload
            .role
            .and_then(|r| serde_json::to_value(r).ok())
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "states_result".to_string());
        let _ = lib.upsert_finding(&FindingRecord {
            key: canonical.clone(),
            session_id: session_id.to_string(),
            evidence_seq: item.seq,
            role,
            gist: item.payload.gist.clone().unwrap_or_default(),
            quote: item.payload.quote.clone(),
            cited_range: item.payload.cited_range.or(Some([item.payload.start_line, item.payload.end_line])),
            recorded_ms: now,
        });
    }
}
