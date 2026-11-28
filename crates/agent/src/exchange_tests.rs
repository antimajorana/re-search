use super::*;
use crate::transport::{
    ModelResponse, PreparedRequest, StopReason, TransportError, TransportInfo,
};
use research_tools::run_budget::{count_model_calls, ModelCallBudget, RunScope};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

struct ScriptedTransport {
    responses: Mutex<VecDeque<Result<ModelResponse, TransportError>>>,
    seen_requests: Arc<Mutex<Vec<ModelRequest>>>,
}

impl ScriptedTransport {
    fn new(responses: Vec<Result<ModelResponse, TransportError>>) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
            seen_requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn seen_requests(&self) -> Vec<ModelRequest> {
        self.seen_requests.lock().unwrap().clone()
    }
}

impl Transport for ScriptedTransport {
    fn info(&self) -> TransportInfo {
        TransportInfo {
            provider: "scripted-test",
            endpoint: "http://127.0.0.1:0/v1/messages".into(),
            api_key_env: "none".into(),
        }
    }

    fn prepare_request(&self, req: &ModelRequest) -> Result<PreparedRequest, TransportError> {
        self.seen_requests.lock().unwrap().push(req.clone());
        let body = serde_json::to_vec(req).unwrap();
        Ok(PreparedRequest { body })
    }

    fn send_prepared(&self, _prepared: &PreparedRequest) -> Result<ModelResponse, TransportError> {
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted transport ran out of responses")
    }
}

fn tool_use_response(id: &str, name: &str, input: Value, inp: u64, out: u64) -> ModelResponse {
    let raw_body = serde_json::to_vec(&json!({
        "id": id, "name": name, "input": input,
        "usage": { "input_tokens": inp, "output_tokens": out }
    }))
    .unwrap();
    ModelResponse {
        raw_body,
        id: Some(format!("msg-{id}")),
        model: "scripted-model-v1".into(),
        stop_reason: StopReason::ToolUse,
        content: vec![ContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }],
        usage: json!({ "input_tokens": inp, "output_tokens": out }),
    }
}

fn sample_spec() -> ExchangeSpec {
    ExchangeSpec::new(
        "sharpen",
        "scripted-model-v1",
        "Sharpen this synthetic idea.",
        ToolSpec {
            name: "record_idea_brief".into(),
            description: "Record the sharpened idea.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "ok": { "type": "boolean" } },
                "required": ["ok"],
                "additionalProperties": false
            }),
        },
    )
    .with_system("Structured exchange only.")
    .with_run_id("run-x6-1")
}

#[test]
fn exchange_offers_one_tool_repairs_up_to_two_times_and_audits_every_attempt() {
    let tmp = TempDir::new().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-ex-repair";
    ledger.append_new_session(sid, &[("idea.started", 1, &json!({}))]).unwrap();

    let transport = ScriptedTransport::new(vec![
        Ok(tool_use_response("tu-1", "record_idea_brief", json!({ "ok": false }), 100, 20)),
        Ok(tool_use_response("tu-2", "record_idea_brief", json!({ "ok": false }), 140, 25)),
        Ok(tool_use_response("tu-3", "record_idea_brief", json!({ "ok": true }), 180, 30)),
    ]);

    let mut checks = 0usize;
    let outcome = run_exchange(
        &transport,
        &ledger,
        sid,
        &sample_spec(),
        None,
        || {
            checks += 1;
            let events = ledger.read_session(sid).unwrap();
            let b = ModelCallBudget::from_events(&events, RunScope::All, 5);
            if b.can_spend(1) { Ok(()) } else { Err("cap".into()) }
        },
        |v| {
            if v.get("ok").and_then(Value::as_bool) == Some(true) {
                Ok("accepted".to_string())
            } else {
                Err(vec!["field `ok` must be true".into()])
            }
        },
    )
    .expect("third attempt should succeed");

    assert_eq!(outcome.value, "accepted");
    assert_eq!(outcome.attempts, 3);
    assert_eq!(checks, 3);
    assert_eq!(outcome.audit.len(), 3);
    assert!(!outcome.audit[0].accepted);
    assert!(!outcome.audit[1].accepted);
    assert!(outcome.audit[2].accepted);
    assert_eq!(outcome.audit[2].tokens_in, Some(180));
    assert_eq!(outcome.audit[2].tokens_out, Some(30));

    let reqs = transport.seen_requests();
    assert_eq!(reqs.len(), 3);
    for req in &reqs {
        assert_eq!(req.tools.len(), 1);
        assert_eq!(req.tools[0].name, "record_idea_brief");
    }

    let events = ledger.read_session(sid).unwrap();
    assert_eq!(count_model_calls(&events, RunScope::All), 3);
    assert_eq!(count_model_calls(&events, RunScope::RunId("run-x6-1")), 3);
    let ex_events: Vec<_> = events.iter().filter(|e| e.kind == IDEA_EXCHANGE_EVENT).collect();
    assert_eq!(ex_events.len(), 3);
    assert_eq!(ex_events[0].payload["attempt"], 1);
    assert_eq!(ex_events[0].payload["accepted"], false);
    assert_eq!(ex_events[2].payload["attempt"], 3);
    assert_eq!(ex_events[2].payload["accepted"], true);
    assert_eq!(ex_events[2].payload["run_id"], "run-x6-1");
    ledger.verify_session(sid).unwrap();
    for rec in &outcome.audit {
        assert!(!ledger.get_blob(&rec.request_blob).unwrap().is_empty());
        assert!(!ledger.get_blob(rec.response_blob.as_deref().unwrap()).unwrap().is_empty());
    }
}

#[test]
fn idea_exchange_rejects_other_tool_calls() {
    // Rule P2: an exchange accepts only its single recorder tool; any other tool call fails immediately.
    let tmp = TempDir::new().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-ex-p2";
    ledger.append_new_session(sid, &[("idea.started", 1, &json!({}))]).unwrap();

    let transport = ScriptedTransport::new(vec![
        Ok(tool_use_response("tu-bad", "sympy_eval", json!({ "code": "1+1" }), 50, 10)),
        Ok(tool_use_response("tu-never", "record_idea_brief", json!({ "ok": true }), 50, 10)),
    ]);

    let mut validator_called = false;
    let err = run_exchange(
        &transport,
        &ledger,
        sid,
        &sample_spec(),
        None,
        || Ok(()),
        |_| {
            validator_called = true;
            Ok(())
        },
    )
    .expect_err("unexpected tool call must fail immediately");

    assert!(!validator_called);
    assert_eq!(
        err,
        ExchangeError::UnexpectedToolCall {
            attempts: 1,
            tool: "sympy_eval".into(),
            expected: "record_idea_brief".into(),
        }
    );
    assert_eq!(transport.seen_requests().len(), 1);
    let events = ledger.read_session(sid).unwrap();
    let ex_events: Vec<_> = events.iter().filter(|e| e.kind == IDEA_EXCHANGE_EVENT).collect();
    assert_eq!(ex_events.len(), 1);
    assert_eq!(ex_events[0].payload["accepted"], false);
    assert!(ex_events[0].payload["faults"][0].as_str().unwrap().contains("Rule P2"));
}

#[test]
fn idea_budget_stops_exchanges_at_cap() {
    // Rule P3: F9 budget check runs before every provider call, including repair retries.
    let tmp = TempDir::new().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-ex-p3";
    ledger.append_new_session(sid, &[("idea.started", 1, &json!({}))]).unwrap();

    // Budget cap = 1 model call: initial call fails validation, repair call MUST be blocked before sending.
    let transport = ScriptedTransport::new(vec![
        Ok(tool_use_response("tu-1", "record_idea_brief", json!({ "ok": false }), 60, 15)),
        Ok(tool_use_response("tu-2", "record_idea_brief", json!({ "ok": true }), 60, 15)),
    ]);

    let err = run_exchange(
        &transport,
        &ledger,
        sid,
        &sample_spec(),
        None,
        || {
            let events = ledger.read_session(sid).unwrap();
            let budget = ModelCallBudget::from_events(&events, RunScope::All, 1);
            if budget.can_spend(1) {
                Ok(())
            } else {
                Err("model call cap (1) reached".into())
            }
        },
        |_| Err::<(), _>(vec!["invalid field".into()]),
    )
    .expect_err("repair attempt must be blocked by F9 budget cap");

    assert_eq!(
        err,
        ExchangeError::BudgetExceeded {
            attempts: 1,
            reason: "model call cap (1) reached".into(),
        }
    );
    // Only 1 request ever reached the transport!
    assert_eq!(transport.seen_requests().len(), 1);
    let events = ledger.read_session(sid).unwrap();
    assert_eq!(count_model_calls(&events, RunScope::All), 1);
}

#[test]
fn exchange_exhausts_after_initial_plus_two_repairs_and_honors_cancel() {
    let tmp = TempDir::new().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-ex-exhaust";
    ledger.append_new_session(sid, &[("idea.started", 1, &json!({}))]).unwrap();

    let transport = ScriptedTransport::new(vec![
        Ok(tool_use_response("tu-1", "record_idea_brief", json!({ "ok": false }), 10, 5)),
        Ok(tool_use_response("tu-2", "record_idea_brief", json!({ "ok": false }), 10, 5)),
        Ok(tool_use_response("tu-3", "record_idea_brief", json!({ "ok": false }), 10, 5)),
    ]);
    // Requesting max_repairs = 99 still clamps to MAX_REPAIR_ATTEMPTS (2) -> max 3 total calls.
    let spec = sample_spec().with_max_repairs(99);
    let err = run_exchange(
        &transport,
        &ledger,
        sid,
        &spec,
        None,
        || Ok(()),
        |_| Err::<(), _>(vec!["still invalid".into()]),
    )
    .unwrap_err();
    assert_eq!(
        err,
        ExchangeError::ValidationExhausted {
            attempts: 3,
            last_faults: vec!["still invalid".into()],
        }
    );
    assert_eq!(transport.seen_requests().len(), 3);

    // Pre-cancelled token sends 0 calls.
    let cancel = CancelToken::new();
    cancel.cancel();
    let transport2 = ScriptedTransport::new(vec![]);
    let err2 = run_exchange(
        &transport2,
        &ledger,
        sid,
        &sample_spec(),
        Some(&cancel),
        || Ok(()),
        |_| Ok(()),
    )
    .unwrap_err();
    assert_eq!(err2, ExchangeError::Cancelled { attempts: 0 });
}
