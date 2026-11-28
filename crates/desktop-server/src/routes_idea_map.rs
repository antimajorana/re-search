//! HTTP handlers for Explore map notes & Rule V6 verdict confirmation (`POST /api/sessions/{id}/idea/note`,
//! package X12).
//!
//! Kept in a sibling module so `routes_idea.rs` stays comfortably <= 600 lines.

use crate::idea::{
    latest_statement_seq, EVENT_IDEA_MAP, EVENT_IDEA_NOTE, MAX_NOTE_COMMENT_CHARS, NOTE_ACTIONS,
    NOTE_TARGET_KINDS,
};
use crate::idea_view;
use crate::routes_session::{bad, conflict, internal, parse_json_body, ApiResult};
use crate::session_lifecycle::is_turn_running;
use crate::state::DesktopState;
use ledger::Ledger;
use research_tools::prior_work::{NOTE_ACTION_CONFIRM_SAME, NOTE_ACTION_REJECT_SAME};
use serde_json::{json, Value};
use tools::EVENT_SCHEMA_VERSION;

/// `POST /api/sessions/{id}/idea/note`: record a user note/flag on a map node, direction,
/// candidate, paper, or verdict (`confirm_same` / `reject_same` for Rule V6 across all three
/// `judged_*` statuses).
///
/// For `confirm_same` / `reject_same` on verdicts, attaches `subclaim_id` and binds `map_seq` to
/// the active revision's latest `idea.map` (rejecting stale/mismatched `map_seq` with HTTP 409)
/// so F12 `compute_prior_work_from_events` and `idea_verdict::compute` recompute
/// `confirmed_by_user` and the F12 chip deterministically (Rules V6, V7, V10).
pub fn handle_note(
    state: &DesktopState,
    ledger: &Ledger,
    session_id: &str,
    body: &[u8],
) -> ApiResult {
    let val = parse_json_body(body)?;
    let target_kind = val
        .get("target_kind")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| NOTE_TARGET_KINDS.contains(s))
        .ok_or_else(|| bad(format!("`target_kind` must be one of: {}", NOTE_TARGET_KINDS.join(", "))))?;

    let target_id = val
        .get("target_id")
        .or_else(|| (target_kind == "verdict").then(|| val.get("subclaim_id")).flatten())
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.chars().count() <= 256)
        .ok_or_else(|| bad("`target_id` must be a non-empty string (<= 256 chars)"))?;

    let action = val
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| NOTE_ACTIONS.contains(s))
        .ok_or_else(|| bad(format!("`action` must be one of: {}", NOTE_ACTIONS.join(", "))))?;

    let is_verdict_confirm =
        action == NOTE_ACTION_CONFIRM_SAME || action == NOTE_ACTION_REJECT_SAME;
    if is_verdict_confirm && target_kind != "verdict" {
        return Err(bad(format!(
            "`action` `{action}` is only valid with `target_kind: \"verdict\"`"
        )));
    }

    let comment = match val.get("comment").filter(|v| !v.is_null()) {
        None => String::new(),
        Some(Value::String(s)) => {
            if s.chars().count() > MAX_NOTE_COMMENT_CHARS {
                return Err(bad(format!(
                    "`comment` exceeds {MAX_NOTE_COMMENT_CHARS} characters"
                )));
            }
            s.trim().to_string()
        }
        Some(_) => return Err(bad("`comment` must be a string")),
    };
    if action == "comment" && comment.is_empty() {
        return Err(bad("`comment` is required when `action` is `comment`"));
    }

    let explicit_map_seq = match val.get("map_seq").filter(|v| !v.is_null()) {
        None => None,
        Some(v) => Some(
            v.as_u64()
                .filter(|&n| n > 0)
                .ok_or_else(|| bad("`map_seq` must be a positive integer"))?,
        ),
    };

    let payload = {
        let _lock = state
            .ledger_write_lock
            .lock()
            .map_err(|_| internal("ledger lock poisoned"))?;
        let mut obj = json!({
            "target_kind": target_kind,
            "target_id": target_id,
            "action": action,
            "comment": comment,
        });
        if is_verdict_confirm {
            let events = ledger
                .read_session(session_id)
                .map_err(|e| internal(e.to_string()))?;
            let stmt_seq = latest_statement_seq(&events);
            let active_map_seq = events
                .iter()
                .rev()
                .find(|e| e.kind == EVENT_IDEA_MAP && e.seq > stmt_seq)
                .map(|e| e.seq)
                .ok_or_else(|| {
                    conflict("cannot confirm or reject verdict without an active map in the current revision")
                })?;
            if let Some(mseq) = explicit_map_seq {
                if mseq != active_map_seq {
                    return Err(conflict(format!(
                        "`map_seq` ({mseq}) does not match active map ({active_map_seq})"
                    )));
                }
            }
            if let Some(map_obj) = obj.as_object_mut() {
                map_obj.insert("subclaim_id".into(), Value::String(target_id.to_string()));
                map_obj.insert("map_seq".into(), json!(active_map_seq));
            }
        }
        ledger
            .append(session_id, EVENT_IDEA_NOTE, EVENT_SCHEMA_VERSION, &obj)
            .map_err(|e| internal(e.to_string()))?;
        obj
    };

    state.sse.emit(
        session_id,
        "session_event",
        json!({ "session": session_id, "kind": EVENT_IDEA_NOTE, "payload": payload }),
    );
    let updated = ledger
        .read_session(session_id)
        .map_err(|e| internal(e.to_string()))?;
    let running = is_turn_running(state, session_id);
    Ok((
        200,
        json!({ "ok": true, "idea": idea_view::view(&updated, running) }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthGuard;
    use crate::chatgpt::ChatGptHub;
    use crate::idea::{
        append_explore_session_start, build_idea_statement, IdeaPreset, EVENT_IDEA_BRIEF,
        EVENT_IDEA_PAPER, EVENT_IDEA_VERDICT,
    };
    use crate::sse::SseHub;
    use crate::state::SessionMeta;
    use crate::DesktopConfig;
    use research_tools::idea_verdict;
    use research_tools::quote_core::{quote_sha256, EVIDENCE_QUOTE_EVENT};
    use research_tools::LiteratureEndpoints;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn test_state(dir: &TempDir) -> DesktopState {
        let data_dir = dir.path().join("data");
        let cfg = DesktopConfig {
            port: 0,
            data_dir,
            workspace_root: None,
            python: None,
            worker_script: None,
            api_base_override: None,
            literature_endpoints: LiteratureEndpoints::default(),
            open_window: false,
            print_url_json: false,
            eval_timeout_s: 30,
            core_exe: None,
            claude_exe: None,
            agy_exe: None,
            connection_env: Some(Default::default()),
            chatgpt_test: None,
        };
        let chatgpt = ChatGptHub::new(&cfg).unwrap();
        let state = DesktopState::new(cfg, AuthGuard::new(0).unwrap(), Arc::new(SseHub::new()), chatgpt);
        state
            .save_meta(&SessionMeta {
                id: "s-v6-test".into(),
                title: "V6 test".into(),
                ..Default::default()
            })
            .unwrap();
        state
    }

    fn seed_three_judged_subclaims(ledger: &Ledger, sid: &str) {
        let stmt = build_idea_statement(1, "Lanczos growth across monitored transition", &[], &[]).unwrap();
        append_explore_session_start(ledger, sid, IdeaPreset::Standard, false, &stmt).unwrap();
        ledger
            .append(
                sid,
                EVENT_IDEA_BRIEF,
                1,
                &json!({
                    "idea_rev": 1,
                    "restatement": "Test restatement",
                    "subclaims": [
                        { "id": "sc-1", "text": "Part 1", "idea_quote": "Lanczos growth" },
                        { "id": "sc-2", "text": "Part 2", "idea_quote": "monitored" },
                        { "id": "sc-3", "text": "Part 3", "idea_quote": "transition" }
                    ]
                }),
            )
            .unwrap();
        let file_sha = "a".repeat(64);
        ledger
            .append(
                sid,
                "source.ingested",
                1,
                &json!({
                    "source_id": "src-1",
                    "files_sha256": { "src-1/paper.tex": file_sha }
                }),
            )
            .unwrap();
        ledger
            .append(
                sid,
                EVENT_IDEA_PAPER,
                1,
                &json!({
                    "key": "arxiv:2405.01111",
                    "source_id": "src-1",
                    "status": "read_passages",
                    "text_basis": "tex",
                    "lines_sent": 40,
                    "total_lines": 100
                }),
            )
            .unwrap();
        let q1 = "Lanczos coefficients grow linearly in the synthetic volume-law chain.";
        let q2 = "Spread complexity saturation appears in the synthetic monitored phase.";
        let ev1 = ledger
            .append(
                sid,
                EVIDENCE_QUOTE_EVENT,
                1,
                &json!({
                    "paper_key": "arxiv:2405.01111",
                    "source_id": "src-1",
                    "path": "src-1/paper.tex",
                    "start_line": 10,
                    "end_line": 12,
                    "file_sha256": file_sha,
                    "quote_sha256": quote_sha256(q1),
                    "quote": q1,
                    "excerpt": q1,
                    "role": "states_result",
                    "gist": "Linear growth",
                    "subclaim_ids": ["sc-1", "sc-2"]
                }),
            )
            .unwrap();
        let ev2 = ledger
            .append(
                sid,
                EVIDENCE_QUOTE_EVENT,
                1,
                &json!({
                    "paper_key": "arxiv:2405.01111",
                    "source_id": "src-1",
                    "path": "src-1/paper.tex",
                    "start_line": 20,
                    "end_line": 22,
                    "file_sha256": file_sha,
                    "quote_sha256": quote_sha256(q2),
                    "quote": q2,
                    "excerpt": q2,
                    "role": "states_result",
                    "gist": "Spread complexity saturation",
                    "subclaim_ids": ["sc-3"]
                }),
            )
            .unwrap();
        let xe1 = format!("xe-{ev1}");
        let xe2 = format!("xe-{ev2}");
        let map_seq = ledger
            .append(
                sid,
                EVENT_IDEA_MAP,
                1,
                &json!({
                    "idea_rev": 1,
                    "nodes": [{
                        "id": "k-1",
                        "kind": "known_result",
                        "rev": 1,
                        "label": "Synthetic Lanczos growth",
                        "statement": "Shows linear growth in the toy model.",
                        "evidence": [xe1],
                        "relation_to_idea": "same",
                        "subclaim_ids": ["sc-1"],
                        "papers": ["arxiv:2405.01111"],
                        "status": "active",
                        "identity_key": xe1
                    }],
                    "edges": [],
                    "overlaps": [
                        {
                            "subclaim_id": "sc-1",
                            "paper_key": "arxiv:2405.01111",
                            "relation": "same",
                            "evidence": [xe1],
                            "difference": "Same toy geometry"
                        },
                        {
                            "subclaim_id": "sc-2",
                            "paper_key": "arxiv:2405.01111",
                            "relation": "partial",
                            "evidence": [xe1],
                            "difference": "No projective monitoring"
                        },
                        {
                            "subclaim_id": "sc-3",
                            "paper_key": "arxiv:2405.01111",
                            "relation": "other_name",
                            "evidence": [xe2],
                            "difference": "Uses spread complexity naming",
                            "other_name": { "term": "spread complexity saturation" }
                        }
                    ],
                    "dropped": []
                }),
            )
            .unwrap();
        let evs = ledger.read_session(sid).unwrap();
        let mut verdict = idea_verdict::compute(&evs, None);
        verdict.map_seq = Some(map_seq);
        ledger
            .append(sid, EVENT_IDEA_VERDICT, 1, &serde_json::to_value(&verdict).unwrap())
            .unwrap();
    }

    #[test]
    fn idea_same_needs_user_confirmation_across_all_three_judged_statuses() {
        let dir = TempDir::new().unwrap();
        let state = test_state(&dir);
        let ledger = state.open_ledger().unwrap();
        let sid = "s-v6-test";
        seed_three_judged_subclaims(&ledger, sid);

        let before = idea_view::view(&ledger.read_session(sid).unwrap(), false);
        let active_map_seq = before["verdicts"]["map_seq"].as_u64().unwrap();
        let chip_of = |view: &Value, sc: &str| {
            view["verdicts"]["verdicts"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["subclaim_id"] == sc)
                .unwrap()["chip"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert_eq!(before["verdicts"]["verdicts"].as_array().unwrap().len(), 3);
        assert_eq!(chip_of(&before, "sc-1"), "Model judges: same");
        assert_eq!(chip_of(&before, "sc-2"), "Model judges: partly the same");
        assert_eq!(chip_of(&before, "sc-3"), "Model judges: same under another name");

        // Confirm sc-1 and sc-2 via target_id, and sc-3 via subclaim_id alias.
        for (body_val, sc, expected_chip) in [
            (json!({ "target_kind": "verdict", "target_id": "sc-1", "action": "confirm_same" }), "sc-1", "found (same)"),
            (json!({ "target_kind": "verdict", "target_id": "sc-2", "action": "confirm_same", "map_seq": active_map_seq }), "sc-2", "found (partly)"),
            (json!({ "target_kind": "verdict", "subclaim_id": "sc-3", "action": "confirm_same" }), "sc-3", "found under another name"),
        ] {
            let body = serde_json::to_vec(&body_val).unwrap();
            let (code, res) = handle_note(&state, &ledger, sid, &body).unwrap();
            assert_eq!(code, 200);
            assert_eq!(chip_of(&res["idea"], sc), expected_chip);
            let last_note = res["idea"]["notes"].as_array().unwrap().last().unwrap();
            assert_eq!(last_note["subclaim_id"], sc);
            assert_eq!(last_note["map_seq"], active_map_seq);
        }

        // Mismatched explicit map_seq -> HTTP 409 conflict.
        let bad_seq = serde_json::to_vec(&json!({
            "target_kind": "verdict",
            "target_id": "sc-1",
            "action": "confirm_same",
            "map_seq": active_map_seq + 99
        }))
        .unwrap();
        assert_eq!(handle_note(&state, &ledger, sid, &bad_seq).unwrap_err().0, 409);

        // Rejecting reverts confirmed status back to unconfirmed "Model judges: ..." (Rule V6).
        let rej = serde_json::to_vec(&json!({
            "target_kind": "verdict",
            "target_id": "sc-1",
            "action": "reject_same"
        }))
        .unwrap();
        let (code, res) = handle_note(&state, &ledger, sid, &rej).unwrap();
        assert_eq!(code, 200);
        assert_eq!(chip_of(&res["idea"], "sc-1"), "Model judges: same");
        assert_eq!(chip_of(&res["idea"], "sc-2"), "found (partly)");
        assert_eq!(chip_of(&res["idea"], "sc-3"), "found under another name");

        // Node flags (wrong / not_relevant / pin / comment) are accepted; empty comment rejected.
        for action in ["wrong", "not_relevant", "pin"] {
            let b = serde_json::to_vec(&json!({
                "target_kind": "node",
                "target_id": "k-1",
                "action": action
            }))
            .unwrap();
            assert_eq!(handle_note(&state, &ledger, sid, &b).unwrap().0, 200);
        }
        let empty_comment = serde_json::to_vec(&json!({
            "target_kind": "node",
            "target_id": "k-1",
            "action": "comment",
            "comment": "   "
        }))
        .unwrap();
        assert_eq!(handle_note(&state, &ledger, sid, &empty_comment).unwrap_err().0, 400);

        // `confirm_same` with non-verdict target_kind is rejected with 400.
        let wrong_kind = serde_json::to_vec(&json!({
            "target_kind": "node",
            "target_id": "k-1",
            "action": "confirm_same"
        }))
        .unwrap();
        assert_eq!(handle_note(&state, &ledger, sid, &wrong_kind).unwrap_err().0, 400);

        // After advancing idea revision without a new map, verdict confirmation returns HTTP 409.
        let rev2 = build_idea_statement(2, "Revised idea without map", &[], &[]).unwrap();
        ledger.append(sid, crate::idea::EVENT_IDEA_STATED, 1, &serde_json::to_value(&rev2).unwrap()).unwrap();
        let stale_confirm = serde_json::to_vec(&json!({
            "target_kind": "verdict",
            "target_id": "sc-1",
            "action": "confirm_same"
        }))
        .unwrap();
        assert_eq!(handle_note(&state, &ledger, sid, &stale_confirm).unwrap_err().0, 409);
        // Stale map evidence remains available in idea_view for quote chips.
        let stale_view = idea_view::view(&ledger.read_session(sid).unwrap(), false);
        assert_eq!(stale_view["map_stale"], true);
        assert!(!stale_view["evidence"].as_array().unwrap().is_empty());
    }
}
