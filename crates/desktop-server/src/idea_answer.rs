//! User answer and sharpened-idea edit handler (`POST /api/sessions/{id}/idea/answer`) — package X6.
//!
//! Extracted from `idea_exchanges.rs` to keep every source module well under 600 lines.
//! Enforces strict perspective invariants (2..=6 perspectives with required `exact` + `other_names`,
//! unique IDs, bounded question/query lengths) so user edits cannot break F12's V3 coverage gate.

use crate::idea::{latest_statement_seq, EVENT_IDEA_ANSWER};
use crate::idea_view;
use crate::routes_session::{bad, internal, parse_json_body, ApiResult};
use crate::session_lifecycle::is_turn_running;
use crate::state::DesktopState;
use ledger::Ledger;
use research_tools::idea_brief::{
    MAX_ASSUMPTIONS, MAX_ASSUMPTION_CHARS, MAX_EXCLUSIONS, MAX_PERSPECTIVES,
    MAX_PERSPECTIVE_QUERIES, MAX_QUERY_CHARS, MAX_RESTATEMENT_CHARS, MIN_PERSPECTIVES,
};
use research_tools::lit_providers::LitProvider;
use research_tools::paper_key::PaperKey;
use research_tools::search_coverage::PerspectiveKind;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use tools::EVENT_SCHEMA_VERSION;

const MAX_ANSWER_ENTRIES: usize = 16;
const MAX_ANSWER_ID_CHARS: usize = 32;
const MAX_ANSWER_TEXT_CHARS: usize = 400;
const MAX_PERSPECTIVE_QUESTION_CHARS: usize = 400;
const MAX_EXCLUSION_CHARS: usize = 200;

/// HTTP handler for `POST /api/sessions/{id}/idea/answer`: records user answers to Sharpen
/// questions and/or user edits to the sharpened idea, queries, reading list, or pause toggle.
pub fn handle_answer_op(
    state: &DesktopState,
    ledger: &Ledger,
    session_id: &str,
    body: &[u8],
) -> ApiResult {
    let val = parse_json_body(body)?;
    let obj = val
        .as_object()
        .ok_or_else(|| bad("`idea/answer` body must be a JSON object"))?;
    let mut payload = serde_json::Map::new();
    let mut has_change = false;

    if let Some(ans) = obj.get("answers").filter(|v| !v.is_null()) {
        validate_answers_field(ans)?;
        payload.insert("answers".into(), ans.clone());
        has_change = true;
    } else if let (Some(qid), Some(answer)) = (obj.get("question_id"), obj.get("answer")) {
        let q = qid
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.chars().count() <= MAX_ANSWER_ID_CHARS)
            .ok_or_else(|| bad("`question_id` must be a non-empty string"))?;
        let a = answer
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.chars().count() <= MAX_ANSWER_TEXT_CHARS)
            .ok_or_else(|| {
                bad(format!(
                    "`answer` must be a non-empty string (<= {MAX_ANSWER_TEXT_CHARS} chars)"
                ))
            })?;
        payload.insert("question_id".into(), json!(q));
        payload.insert("answer".into(), json!(a));
        has_change = true;
    }

    if let Some(r) = obj.get("restatement").filter(|v| !v.is_null()) {
        let s = r
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.chars().count() <= MAX_RESTATEMENT_CHARS)
            .ok_or_else(|| {
                bad(format!(
                    "`restatement` must be a non-empty string (<= {MAX_RESTATEMENT_CHARS} chars)"
                ))
            })?;
        payload.insert("restatement".into(), json!(s));
        has_change = true;
    }

    if let Some(a) = obj.get("assumptions").filter(|v| !v.is_null()) {
        let arr = a
            .as_array()
            .ok_or_else(|| bad("`assumptions` must be an array of strings"))?;
        if arr.len() > MAX_ASSUMPTIONS {
            return Err(bad(format!("at most {MAX_ASSUMPTIONS} assumptions allowed")));
        }
        let mut clean = Vec::with_capacity(arr.len());
        for item in arr {
            let s = item
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty() && s.chars().count() <= MAX_ASSUMPTION_CHARS)
                .ok_or_else(|| {
                    bad(format!(
                        "each assumption must be a non-empty string (<= {MAX_ASSUMPTION_CHARS} chars)"
                    ))
                })?;
            clean.push(json!(s));
        }
        payload.insert("assumptions".into(), Value::Array(clean));
        has_change = true;
    }

    if let Some(ps) = obj.get("perspectives").filter(|v| !v.is_null()) {
        let clean = validate_edited_perspectives(ps)?;
        payload.insert("perspectives".into(), Value::Array(clean));
        has_change = true;
    }

    if let Some(ex) = obj.get("exclusions").filter(|v| !v.is_null()) {
        let arr = ex
            .as_array()
            .ok_or_else(|| bad("`exclusions` must be an array of strings"))?;
        if arr.len() > MAX_EXCLUSIONS {
            return Err(bad(format!("at most {MAX_EXCLUSIONS} exclusions allowed")));
        }
        let mut clean = Vec::with_capacity(arr.len());
        for item in arr {
            let s = item
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty() && s.chars().count() <= MAX_EXCLUSION_CHARS)
                .ok_or_else(|| {
                    bad(format!(
                        "each exclusion must be a non-empty string (<= {MAX_EXCLUSION_CHARS} chars)"
                    ))
                })?;
            clean.push(json!(s));
        }
        payload.insert("exclusions".into(), Value::Array(clean));
        has_change = true;
    }

    if let Some(rl) = obj.get("reading_list").filter(|v| !v.is_null()) {
        let arr = rl
            .as_array()
            .ok_or_else(|| bad("`reading_list` must be an array of paper keys"))?;
        let mut clean = Vec::with_capacity(arr.len());
        for item in arr {
            let raw = item
                .as_str()
                .map(str::trim)
                .ok_or_else(|| bad("each `reading_list` entry must be a PaperKey string"))?;
            let key = PaperKey::parse(raw).map_err(|_| bad(format!("invalid PaperKey `{raw}`")))?;
            let ks = key.as_str().to_string();
            if !clean.iter().any(|v: &Value| v.as_str() == Some(ks.as_str())) {
                clean.push(json!(ks));
            }
        }
        payload.insert("reading_list".into(), Value::Array(clean));
        has_change = true;
    }

    if let Some(pause) = obj.get("pause_before_reading").filter(|v| !v.is_null()) {
        let flag = pause
            .as_bool()
            .ok_or_else(|| bad("`pause_before_reading` must be a boolean"))?;
        payload.insert("pause_before_reading".into(), json!(flag));
        has_change = true;
    }

    if !has_change {
        return Err(bad(
            "`idea/answer` requires at least one of `answers`, `question_id`+`answer`, \
             `restatement`, `assumptions`, `perspectives`, `exclusions`, `reading_list`, or `pause_before_reading`",
        ));
    }

    let _lock = state
        .ledger_write_lock
        .lock()
        .map_err(|_| internal("ledger lock poisoned"))?;
    let events = ledger
        .read_session(session_id)
        .map_err(|e| internal(e.to_string()))?;
    let stmt_seq = latest_statement_seq(&events);
    if stmt_seq == 0 {
        return Err(bad("session has no idea statement"));
    }
    if let Some(rl_arr) = payload.get("reading_list").and_then(Value::as_array) {
        let budget = crate::idea::effective_budget(&events);
        if budget.max_papers == 0 && !rl_arr.is_empty() {
            return Err(bad(
                "Quick preset reads 0 full papers (`reading_list` must be empty; Rule V2)",
            ));
        }
        if rl_arr.len() > budget.max_papers {
            return Err(bad(format!(
                "`reading_list` contains {} papers, exceeding preset cap of {}",
                rl_arr.len(),
                budget.max_papers
            )));
        }
        let known_cands: std::collections::BTreeSet<String> = events
            .iter()
            .rev()
            .find(|e| e.kind == crate::idea::EVENT_IDEA_CANDIDATES && e.seq > stmt_seq)
            .and_then(|e| e.payload.get("candidates"))
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|c| c.get("key").and_then(Value::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        if !known_cands.is_empty() {
            for entry in rl_arr {
                if let Some(k) = entry.as_str() {
                    if !known_cands.contains(k) {
                        return Err(bad(format!(
                            "`reading_list` entry `{k}` is not among current session candidates"
                        )));
                    }
                }
            }
        }
    }
    payload.insert("author".into(), json!("user"));
    let val_payload = Value::Object(payload);
    ledger
        .append(session_id, EVENT_IDEA_ANSWER, EVENT_SCHEMA_VERSION, &val_payload)
        .map_err(|e| internal(e.to_string()))?;
    state.sse.emit(
        session_id,
        "session_event",
        json!({ "session": session_id, "kind": EVENT_IDEA_ANSWER, "payload": val_payload }),
    );
    let updated = ledger
        .read_session(session_id)
        .map_err(|e| internal(e.to_string()))?;
    Ok((
        200,
        json!({ "ok": true, "idea": idea_view::view(&updated, is_turn_running(state, session_id)) }),
    ))
}

fn validate_answers_field(ans: &Value) -> Result<(), (u16, String)> {
    if let Some(map) = ans.as_object() {
        if map.len() > MAX_ANSWER_ENTRIES {
            return Err(bad(format!("at most {MAX_ANSWER_ENTRIES} answers allowed")));
        }
        for (k, v) in map {
            if k.trim().is_empty() || k.chars().count() > MAX_ANSWER_ID_CHARS {
                return Err(bad(format!(
                    "question id in `answers` must be non-empty (<= {MAX_ANSWER_ID_CHARS} chars)"
                )));
            }
            let _s = v
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty() && s.chars().count() <= MAX_ANSWER_TEXT_CHARS)
                .ok_or_else(|| {
                    bad(format!(
                        "answer for `{k}` must be a non-empty string (<= {MAX_ANSWER_TEXT_CHARS} chars)"
                    ))
                })?;
        }
        return Ok(());
    }
    if let Some(arr) = ans.as_array() {
        if arr.len() > MAX_ANSWER_ENTRIES {
            return Err(bad(format!("at most {MAX_ANSWER_ENTRIES} answers allowed")));
        }
        for item in arr {
            let _q = item
                .get("question_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty() && s.chars().count() <= MAX_ANSWER_ID_CHARS)
                .ok_or_else(|| bad("each answer requires a non-empty `question_id`"))?;
            let _a = item
                .get("answer")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty() && s.chars().count() <= MAX_ANSWER_TEXT_CHARS)
                .ok_or_else(|| {
                    bad(format!(
                        "each answer requires a non-empty `answer` (<= {MAX_ANSWER_TEXT_CHARS} chars)"
                    ))
                })?;
        }
        return Ok(());
    }
    Err(bad(
        "`answers` must be an object map or an array of `{question_id, answer}`",
    ))
}

fn validate_edited_perspectives(raw: &Value) -> Result<Vec<Value>, (u16, String)> {
    let arr = raw
        .as_array()
        .ok_or_else(|| bad("`perspectives` must be an array"))?;
    if arr.len() < MIN_PERSPECTIVES || arr.len() > MAX_PERSPECTIVES {
        return Err(bad(format!(
            "`perspectives` must contain {MIN_PERSPECTIVES}..={MAX_PERSPECTIVES} items"
        )));
    }
    let mut out = Vec::with_capacity(arr.len());
    let mut seen_ids = BTreeSet::new();
    let mut has_exact = false;
    let mut has_other_names = false;
    for (idx, item) in arr.iter().enumerate() {
        let obj = item
            .as_object()
            .ok_or_else(|| bad("each perspective must be an object"))?;
        let id = obj
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.chars().count() <= 32)
            .map(str::to_string)
            .unwrap_or_else(|| format!("p-{}", idx + 1));
        if !seen_ids.insert(id.clone()) {
            return Err(bad(format!("duplicate perspective id `{id}`")));
        }
        let kind_str = obj
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| bad("each perspective requires `kind`"))?;
        let kind = PerspectiveKind::parse(kind_str)
            .ok_or_else(|| bad(format!("invalid perspective kind `{kind_str}`")))?;
        if kind == PerspectiveKind::Exact {
            has_exact = true;
        }
        if kind == PerspectiveKind::OtherNames {
            has_other_names = true;
        }
        let question = obj
            .get("question")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.chars().count() <= MAX_PERSPECTIVE_QUESTION_CHARS)
            .ok_or_else(|| {
                bad(format!(
                    "each perspective requires a non-empty `question` (<= {MAX_PERSPECTIVE_QUESTION_CHARS} chars)"
                ))
            })?;
        let qarr = obj
            .get("queries")
            .and_then(Value::as_array)
            .ok_or_else(|| bad("each perspective requires a `queries` array"))?;
        if qarr.is_empty() || qarr.len() > MAX_PERSPECTIVE_QUERIES {
            return Err(bad(format!(
                "perspective `{id}` must have 1..={MAX_PERSPECTIVE_QUERIES} queries"
            )));
        }
        let mut clean_qs = Vec::with_capacity(qarr.len());
        for qitem in qarr {
            let prov_raw = qitem
                .get("provider")
                .and_then(Value::as_str)
                .ok_or_else(|| bad("each query requires `provider`"))?;
            let prov = LitProvider::parse(prov_raw)
                .ok_or_else(|| bad(format!("invalid provider `{prov_raw}`")))?;
            let qtext = qitem
                .get("q")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty() && s.chars().count() <= MAX_QUERY_CHARS)
                .ok_or_else(|| {
                    bad(format!(
                        "each query `q` must be non-empty and <= {MAX_QUERY_CHARS} chars"
                    ))
                })?;
            clean_qs.push(json!({ "provider": prov.as_str(), "q": qtext }));
        }
        out.push(json!({
            "id": id,
            "kind": kind.as_str(),
            "question": question,
            "queries": clean_qs,
        }));
    }
    if !has_exact || !has_other_names {
        return Err(bad(
            "`perspectives` must retain at least one `exact` and one `other_names` perspective",
        ));
    }
    Ok(out)
}
