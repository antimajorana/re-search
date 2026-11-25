//! Post-turn finalization for Investigation sessions.
use crate::state::DesktopState;
use serde_json::{json, Value};

pub(crate) fn finish(
    state: &DesktopState,
    session: &str,
    req: &Value,
    outcome: std::result::Result<String, String>,
) -> std::result::Result<String, String> {
    if let Some(seq) = req["_paper_question_request"].as_u64() {
        return match state.open_ledger() {
            Ok(ledger) => {
                crate::investigation_question::finish_question(state, &ledger, session, seq, outcome)
            }
            Err(_) => outcome,
        };
    }
    if let Some(seq) = req["_paper_conventions_request"].as_u64() {
        let ledger = state.open_ledger()?;
        let events = ledger.read_session(session).map_err(|e| e.to_string())?;
        let catalog = research_tools::investigation::paper_conventions(&events, seq);
        let done =
            crate::investigation_scope::conventions_finish_done(&events, seq, catalog.as_ref());
        let outcome = outcome.and_then(|reply| {
            if done {
                Ok(reply)
            } else {
                Err(
                    "The conventions review is still incomplete. Its saved entries are available."
                        .into(),
                )
            }
        });
        ledger.append(session,"investigation.conventions_finished",1,&json!({"for_request_seq":seq,"status":if done {"complete"} else {"incomplete"},"error":outcome.as_ref().err()})).map_err(|e|e.to_string())?;
        return outcome;
    } else if let Some(seq) = req["_paper_branch_request"].as_u64() {
        let ledger = state.open_ledger()?;
        let events = ledger.read_session(session).map_err(|e| e.to_string())?;
        let recorded = events.iter().any(|e| {
            e.kind == "investigation.branch" && e.payload["request_seq"].as_u64() == Some(seq)
        });
        let outcome = outcome.and_then(|reply| {
            if recorded {
                Ok(reply)
            } else {
                Err(
                    "The model did not save a source-anchored branch. You can retry this result."
                        .into(),
                )
            }
        });
        ledger
            .append(
                session,
                "investigation.branch_finished",
                1,
                &json!({"request_seq":seq,"status":if outcome.is_ok() {"built"}else{"failed"},
            "error":outcome.as_ref().err()}),
            )
            .map_err(|e| e.to_string())?;
        return outcome;
    } else if let Some(seq) = req["_paper_request"].as_u64() {
        let ledger = state.open_ledger()?;
        let (status, text) = match &outcome {
            Ok(t) => ("answered", t),
            Err(t) => {
                let events = ledger.read_session(session).unwrap_or_default();
                let stopped = events.iter().any(|e| {
                    e.seq > seq && e.kind == "session.abort" && e.payload["stopped"] == true
                });
                (if stopped { "stopped" } else { "failed" }, t)
            }
        };
        ledger.append(session,"investigation.result",1,&json!({"request_seq":seq,"status":status,"text":text.chars().take(24000).collect::<String>()})).map_err(|e| e.to_string())?;
    } else if let Some(seq) = req["_paper_map_request"].as_u64() {
        let ledger = state.open_ledger()?;
        let events = ledger.read_session(session).map_err(|e| e.to_string())?;
        let recorded = events.iter().any(|e| {
            e.kind == "investigation.map" && e.payload["request_seq"].as_u64() == Some(seq)
        });
        let needs_conventions = events.iter().any(|e| {
            e.seq == seq
                && e.kind == "investigation.mapping_requested"
                && e.payload["conventions_requested"] == true
        });
        let completed_catalog = events.iter().any(|e| {
            e.kind == "investigation.conventions"
                && e.payload["request_seq"].as_u64() == Some(seq)
                && e.payload["done"] == true
        });
        let outcome = outcome.and_then(|reply| {
            if !recorded {
                Err("The model finished without saving the main results with their sources. Try again, or choose another model.".into())
            } else if needs_conventions && !completed_catalog {
                Err("The main results are saved. The model finished before its conventions review was complete.".into())
            } else { Ok(reply) }
        });
        ledger.append(session,"investigation.mapping_finished",1,&json!({"request_seq":seq,
            "status":if recorded {"mapped"} else {"failed"}, "error":if recorded {None}else{outcome.as_ref().err()},
            "conventions_error":if recorded && needs_conventions && !completed_catalog {outcome.as_ref().err()}else{None}})).map_err(|e|e.to_string())?;
        return outcome;
    }
    outcome
}
