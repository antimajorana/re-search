//! Isolated paper-reader evidence store (`note_evidence`) and recorder verification (`resolve_evidence`).
//!
//! `note_evidence` is registered ONLY on the isolated paper-reader router via
//! [`register_reader_tools`] and is never exposed in the main session registry.

use crate::investigation::anchor::{
    bounded, fold_with_map, is_bare_section_name, valid_id, Anchor, AnchorBasis, AnchorCorrection,
    AnchorFault, AnchorNote, SelectedPages, SourceIndex, ANCHOR_KINDS, MAX_TOTAL_BYTES,
};
use crate::ResearchEnv;
use ledger::{Event, Ledger};
use policy::EffectClass;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tools::{ApprovalScope, CallCtx, Registry, ToolDef};

pub const NOTE_EVIDENCE: &str = "note_evidence";
pub const EVIDENCE_EVENT: &str = "investigation.evidence";
pub const READER_CALLER_PREFIX: &str = "worker:paper_reader:";
pub const READER_LINE_BUDGET: usize = 7_000;
const EVIDENCE_ROLES: &[&str] = &[
    "result", "definition", "convention", "notation", "method", "motivation", "scope",
];
const MAX_ITEMS: usize = 12;
const MAX_GIST_CHARS: usize = 240;
const MAX_EXCERPT_BYTES: usize = 3200;
const MAX_SESSION_CHANGED_RETRIES: usize = 3;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRecord {
    pub marker_seq: u64,
    pub source_id: String,
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cited_range: Option<[usize; 2]>,
    #[serde(default)]
    pub range_basis: Vec<String>,
    pub kind: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub gist: String,
    pub quote: String,
    pub excerpt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt_window: Option<[usize; 2]>,
    pub file_sha256: String,
    pub source_seq: u64,
    pub caller: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoteEvidenceArgs {
    items: Vec<NoteEvidenceItem>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoteEvidenceItem {
    source_id: String,
    #[serde(default)]
    path: String,
    start_line: usize,
    end_line: usize,
    quote: String,
    kind: String,
    role: String,
    #[serde(default)]
    label: Option<String>,
    gist: String,
}

pub fn note_evidence_def() -> ToolDef {
    let item = json!({
        "type": "object",
        "properties": {
            "source_id": { "type": "string", "minLength": 1, "maxLength": 64 },
            "path": { "type": "string", "minLength": 1, "maxLength": 1024 },
            "start_line": { "type": "integer", "minimum": 1 },
            "end_line": { "type": "integer", "minimum": 1 },
            "quote": { "type": "string", "minLength": 3, "maxLength": 300 },
            "kind": { "type": "string", "enum": ANCHOR_KINDS },
            "role": { "type": "string", "enum": EVIDENCE_ROLES },
            "label": { "type": "string", "minLength": 1, "maxLength": 160 },
            "gist": { "type": "string", "minLength": 1, "maxLength": MAX_GIST_CHARS }
        },
        "required": ["source_id", "start_line", "end_line", "quote", "kind", "role", "gist"],
        "additionalProperties": false
    });
    ToolDef {
        id: NOTE_EVIDENCE.into(),
        description: "Save 1–12 verified paper evidence pointers with exact line spans, verbatim quotes, and concise gists for the orchestrator to cite by evidence ID.".into(),
        input_schema: json!({
            "type": "object",
            "properties": { "items": { "type": "array", "minItems": 1, "maxItems": MAX_ITEMS, "items": item } },
            "required": ["items"],
            "additionalProperties": false
        }),
        effect: EffectClass::ReadWorkspace,
        approval_scope: ApprovalScope::Once,
    }
}

/// Register the exact three tools permitted on an isolated paper-reader router:
/// `search_paper_passages`, `read_paper_lines` (7,000-char budget), and `note_evidence`.
pub fn register_reader_tools(registry: &mut Registry, env: Arc<ResearchEnv>) {
    crate::paper_passages::register(registry, env.clone());
    crate::paper_lines::register_with_budget(registry, env.clone(), READER_LINE_BUDGET);
    registry.register(
        note_evidence_def(),
        Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
            note_evidence(&env, ctx.ledger, ctx.session, ctx.caller, args)
        }),
    );
}

pub(crate) fn find_active_marker(events: &[Event]) -> Result<(&[Event], &Event, &Event), String> {
    let active = events
        .iter()
        .rposition(|e| matches!(e.kind.as_str(), "investigation.started" | "investigation.handoff"))
        .filter(|&i| events[i].kind == "investigation.started")
        .ok_or("Evidence can only be noted inside an active Investigation.")?;
    let history = &events[active..];
    let paper = history
        .iter()
        .find(|e| e.kind == "investigation.paper")
        .ok_or("Select and start an ingested paper before noting evidence.")?;
    let marker = history
        .iter()
        .rev()
        .find(|e| {
            matches!(
                e.kind.as_str(),
                "investigation.mapping_requested" | "investigation.mapping_finished"
                    | "investigation.branch_requested" | "investigation.branch_finished"
                    | "investigation.conventions_resumed" | "investigation.conventions_finished"
                    | "investigation.question_requested" | "investigation.answer"
                    | "investigation.requested" | "investigation.result"
                    | "session.end" | "session.abort"
            )
        })
        .filter(|e| {
            matches!(
                e.kind.as_str(),
                "investigation.mapping_requested" | "investigation.branch_requested" | "investigation.conventions_resumed"
            )
        })
        .ok_or("No active paper reading request (mapping, branch, or conventions) is open.")?;
    Ok((history, paper, marker))
}

pub fn note_evidence(
    env: &ResearchEnv,
    ledger_mutex: &Mutex<Ledger>,
    session: &str,
    caller: &str,
    args: &Value,
) -> Result<Value, String> {
    note_evidence_with_hook(env, ledger_mutex, session, caller, args, None)
}

#[doc(hidden)]
pub fn note_evidence_with_hook(
    env: &ResearchEnv,
    ledger_mutex: &Mutex<Ledger>,
    session: &str,
    caller: &str,
    args: &Value,
    before_append: Option<&dyn Fn(usize, usize)>,
) -> Result<Value, String> {
    if !caller.starts_with(READER_CALLER_PREFIX) {
        return Err("note_evidence may only be called by an isolated paper_reader worker.".into());
    }
    crate::validate_session_id(session).map_err(|e| e.to_string())?;
    let parsed: NoteEvidenceArgs =
        serde_json::from_value(args.clone()).map_err(|e| format!("Invalid note_evidence args: {e}"))?;
    if parsed.items.is_empty() || parsed.items.len() > MAX_ITEMS {
        return Err(format!("Supply between 1 and {MAX_ITEMS} evidence items."));
    }
    {
        let ledger = ledger_mutex.lock().map_err(|_| "ledger lock poisoned")?;
        let (_, events) = ledger
            .read_verified_session(session)
            .map_err(|_| "The Investigation ledger or its saved artifacts could not be verified.")?;
        find_active_marker(&events)?;
    }

    let mut results = Vec::with_capacity(parsed.items.len());
    let (mut noted, mut failed) = (0usize, 0usize);

    for (idx, item) in parsed.items.into_iter().enumerate() {
        let owner = format!("items[{idx}]");
        if !valid_id(&item.source_id, 64)
            || !ANCHOR_KINDS.contains(&item.kind.as_str())
            || !EVIDENCE_ROLES.contains(&item.role.as_str())
            || !bounded(&item.gist, MAX_GIST_CHARS)
            || item.label.as_deref().is_some_and(|l| !bounded(l, 160))
        {
            failed += 1;
            results.push(json!({ "index": idx, "ok": false, "error": "invalid_item", "message": "Invalid item fields." }));
            continue;
        }

        let mut outcome: Option<Value> = None;
        for attempt in 0..=MAX_SESSION_CHANGED_RETRIES {
            let ledger = ledger_mutex.lock().map_err(|_| "ledger lock poisoned")?;
            let (verified, events) = ledger
                .read_verified_session(session)
                .map_err(|_| "The Investigation ledger or its saved artifacts could not be verified.")?;
            let (history, paper, marker) = find_active_marker(&events)?;
            let primary_source_id = paper.payload["source_id"].as_str().ok_or("Investigation paper is missing source_id.")?;
            let receipts: BTreeMap<_, _> = events
                .iter()
                .filter(|e| e.kind == "source.ingested")
                .filter_map(|e| Some((e.payload["source_id"].as_str()?, e)))
                .collect();
            let primary = receipts.get(primary_source_id).copied().ok_or("Primary paper receipt missing.")?;
            let pages = SelectedPages::for_paper(&ledger, &paper.payload, &primary.payload)?;
            let mut index = SourceIndex {
                sandbox: env.sandbox(), session, primary_source_id, receipts,
                files: BTreeMap::new(), bytes_left: MAX_TOTAL_BYTES, pages, history,
                active_marker_seq: Some(marker.seq),
            };
            let mut anchor = Anchor {
                source_id: item.source_id.clone(), path: item.path.clone(),
                start_line: item.start_line, end_line: item.end_line,
                excerpt: None, quote: Some(item.quote.clone()), evidence: None,
                kind: item.kind.clone(), label: item.label.clone(),
                file_sha256: String::new(), source_seq: 0, excerpt_basis: None,
                cited_range: None, range_basis: Vec::new(), excerpt_window: None,
            };
            let note = match index.verify_pointer(&owner, &mut anchor) {
                Ok(n) => n,
                Err(fault) => {
                    failed += 1;
                    outcome = Some(json!({ "index": idx, "ok": false, "error": fault.code, "fault": fault }));
                    break;
                }
            };
            let rec = EvidenceRecord {
                marker_seq: marker.seq, source_id: anchor.source_id.clone(), path: anchor.path.clone(),
                start_line: anchor.start_line, end_line: anchor.end_line, cited_range: anchor.cited_range,
                range_basis: anchor.range_basis.clone(), kind: anchor.kind.clone(), role: item.role.clone(),
                label: anchor.label.clone(), gist: item.gist.trim().to_string(), quote: item.quote.clone(),
                excerpt: anchor.excerpt.clone().unwrap_or_default(), excerpt_window: anchor.excerpt_window,
                file_sha256: anchor.file_sha256.clone(), source_seq: anchor.source_seq, caller: caller.to_string(),
            };
            let rec_val = serde_json::to_value(&rec).map_err(|e| e.to_string())?;
            if let Some(hook) = before_append { hook(idx, attempt); }
            match ledger.append_at_head(session, EVIDENCE_EVENT, 1, &rec_val, &verified.head_hash) {
                Ok(appended) => {
                    noted += 1;
                    let mut entry = json!({
                        "index": idx, "ok": true, "evidence": format!("ev-{}", appended.seq),
                        "start_line": anchor.start_line, "end_line": anchor.end_line,
                    });
                    if note.corrected.is_some() { entry["corrected"] = json!(true); }
                    outcome = Some(entry);
                    break;
                }
                Err(ledger::Error::SessionChanged(_)) if attempt < MAX_SESSION_CHANGED_RETRIES => continue,
                Err(ledger::Error::SessionChanged(_)) => {
                    failed += 1;
                    outcome = Some(json!({ "index": idx, "ok": false, "error": "ledger_busy", "message": "Session head changed concurrently after 3 retries." }));
                    break;
                }
                Err(other) => return Err(other.to_string()),
            }
        }
        if let Some(res) = outcome { results.push(res); }
    }
    Ok(json!({ "results": results, "noted": noted, "failed": failed }))
}

fn parse_evidence_seq(ev_id: &str) -> Option<u64> {
    let digits = ev_id.strip_prefix("ev-")?;
    if digits.is_empty() || digits.len() > 12 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u64>().ok().filter(|&n| n >= 1)
}

/// Resolve and re-verify an evidence-backed recorder anchor `{source_id, evidence, quote, kind, label?}`.
#[allow(clippy::result_large_err)]
pub(crate) fn resolve_evidence(
    index: &mut SourceIndex<'_>,
    owner: &str,
    anchor: &mut Anchor,
) -> Result<AnchorNote, AnchorFault> {
    let fault = |code: &'static str, msg: String| AnchorFault {
        owner: owner.to_string(), code, message: msg, lines: None, true_text: None, legacy: false,
    };
    if !valid_id(&anchor.source_id, 64)
        || !ANCHOR_KINDS.contains(&anchor.kind.as_str())
        || anchor.label.as_deref().is_some_and(|l| !bounded(l, 160))
    {
        return Err(fault("invalid_anchor", "Invalid source_id, kind, or label.".into()));
    }
    if let Some(label) = anchor.label.as_deref().filter(|l| is_bare_section_name(l)) {
        return Err(fault("label_is_section_name", format!("Label `{label}` is a bare section name.")));
    }
    let Some(ev_id) = anchor.evidence.as_deref() else {
        return Err(fault("evidence_missing", "Missing evidence ID.".into()));
    };
    let Some(seq) = parse_evidence_seq(ev_id) else {
        return Err(fault("invalid_evidence_id", format!("Evidence ID `{ev_id}` must match `ev-<seq>`.")));
    };
    let Some(quote_raw) = anchor.quote.as_deref().filter(|q| bounded(q, 300)) else {
        return Err(fault("evidence_quote_missing", "Evidence anchors require the recorder's own verbatim `quote`.".into()));
    };
    let Some(active_marker_seq) = index.active_marker_seq else {
        return Err(fault("no_active_marker", "No active reading marker for evidence resolution.".into()));
    };
    let Some(ev) = index.history.iter().find(|e| e.seq == seq) else {
        return Err(fault("evidence_not_found", format!("Evidence `{ev_id}` not found in active Investigation.")));
    };
    if ev.kind != EVIDENCE_EVENT {
        return Err(fault("evidence_wrong_kind", format!("Event `{ev_id}` is `{}` rather than `{EVIDENCE_EVENT}`.", ev.kind)));
    }
    let rec: EvidenceRecord = serde_json::from_value(ev.payload.clone())
        .map_err(|e| fault("invalid_evidence_record", format!("Evidence `{ev_id}` failed schema validation: {e}")))?;
    if !rec.caller.starts_with(READER_CALLER_PREFIX) {
        return Err(fault("invalid_evidence_caller", format!("Evidence `{ev_id}` was not recorded by a paper_reader worker.")));
    }
    if rec.marker_seq != active_marker_seq {
        return Err(fault("evidence_marker_mismatch", format!("Evidence `{ev_id}` belongs to marker {} rather than active marker {active_marker_seq}.", rec.marker_seq)));
    }
    if rec.source_id != anchor.source_id {
        return Err(fault("evidence_source_mismatch", format!("Evidence `{ev_id}` belongs to source `{}` rather than `{}`.", rec.source_id, anchor.source_id)));
    }
    if (!anchor.path.is_empty() && anchor.path != rec.path)
        || (anchor.start_line != 0 && anchor.start_line != rec.start_line)
        || (anchor.end_line != 0 && anchor.end_line != rec.end_line)
    {
        return Err(fault("evidence_location_mismatch", format!("Supplied path/lines conflict with evidence `{ev_id}`.")));
    }
    if rec.start_line == 0 || rec.end_line < rec.start_line || rec.end_line - rec.start_line >= 64 {
        return Err(fault("range_too_long", format!("Evidence `{ev_id}` has an invalid line span.")));
    }
    if let Some(pages) = &index.pages {
        pages.check_range(&rec.source_id, &rec.path, rec.start_line, rec.end_line).map_err(|msg| fault("outside_selected_pages", msg))?;
    }
    let file = index.load(&rec.source_id, &rec.path).map_err(|msg| fault("source_load_failed", msg))?;
    if file.sha256 != rec.file_sha256 || file.source_seq != rec.source_seq {
        return Err(fault("source_changed", format!("Source file `{}` revision no longer matches `{ev_id}`.", rec.path)));
    }
    let Some(full_lines) = (rec.start_line >= 1 && rec.start_line <= rec.end_line && rec.end_line <= file.line_count())
        .then(|| file.lines_text(rec.start_line, rec.end_line)) else {
        return Err(fault("line_out_of_range", format!("Evidence `{ev_id}` lines {}–{} are outside `{}`.", rec.start_line, rec.end_line, rec.path)));
    };
    let derived: &str = if let Some([w0, w1]) = rec.excerpt_window {
        if w0 > w1 || w1 > full_lines.len() || !full_lines.is_char_boundary(w0) || !full_lines.is_char_boundary(w1) || w1 - w0 > MAX_EXCERPT_BYTES {
            return Err(fault("invalid_excerpt_window", format!("Evidence `{ev_id}` has an invalid excerpt_window.")));
        }
        &full_lines[w0..w1]
    } else if full_lines.len() > MAX_EXCERPT_BYTES {
        return Err(fault("excerpt_too_large", format!("Evidence `{ev_id}` exceeds {MAX_EXCERPT_BYTES} bytes without excerpt_window.")));
    } else {
        full_lines
    };
    if derived != rec.excerpt {
        return Err(fault("evidence_excerpt_mismatch", format!("Re-derived lines do not match stored excerpt of `{ev_id}`.")));
    }
    if let Some(label) = anchor.label.as_deref().filter(|l| !full_lines.contains(*l)) {
        return Err(fault("label_not_found", format!("Label `{label}` not found in evidence `{ev_id}`.")));
    }
    let ns_len = quote_raw.chars().filter(|c| !c.is_whitespace()).count();
    let (folded_derived, _) = fold_with_map(derived);
    let (folded_quote, _) = fold_with_map(quote_raw);
    let whole_ok = ns_len >= 3 && folded_derived == folded_quote;
    if (ns_len < 16 && !whole_ok) || folded_quote.is_empty() || !folded_derived.contains(&folded_quote) {
        return Err(fault("evidence_quote_mismatch", format!("Recorder quote was not found in evidence `{ev_id}`.")));
    }
    let (derived_owned, file_sha, source_seq) = (derived.to_string(), file.sha256.clone(), file.source_seq);
    anchor.path = rec.path;
    anchor.start_line = rec.start_line;
    anchor.end_line = rec.end_line;
    anchor.excerpt = Some(derived_owned);
    anchor.excerpt_basis = Some("evidence".to_string());
    anchor.cited_range = rec.cited_range;
    anchor.range_basis = rec.range_basis.clone();
    anchor.excerpt_window = rec.excerpt_window;
    anchor.file_sha256 = file_sha;
    anchor.source_seq = source_seq;
    let corrected = rec.cited_range.map(|from| AnchorCorrection {
        owner: owner.to_string(),
        lines: [anchor.start_line, anchor.end_line],
        from,
        basis: rec.range_basis,
    });
    Ok(AnchorNote { basis: AnchorBasis::Evidence, corrected })
}
