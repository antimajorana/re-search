use super::*;
use crate::approval::{ApprovalReply, ApprovalScope};
use crate::gate::{
    Gate, GateChain, GateDecision, GateRefusal, GateRequest, PostCallCtx, GATE_ERROR_PREFIX,
    GUARDRAIL_BLOCKED_EVENT,
};
use crate::registry::ToolDef;
use policy::{EffectClass, Match, Rule};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

struct TestApprover {
    approve: bool,
    asked: AtomicUsize,
    seen: Mutex<Vec<(String, String)>>,
}

impl TestApprover {
    fn new(approve: bool) -> Arc<Self> {
        Arc::new(Self { approve, asked: AtomicUsize::new(0), seen: Mutex::new(Vec::new()) })
    }
}

impl Approver for TestApprover {
    fn decide(&self, request: &ApprovalRequest) -> ApprovalReply {
        self.asked.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push((request.caller.clone(), request.tool_id.clone()));
        ApprovalReply { approved: self.approve, digest: request.digest.clone(), by: "test".into() }
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("tools-gate-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn def(id: &str, effect: EffectClass) -> ToolDef {
    ToolDef {
        id: id.into(),
        description: format!("test tool {id}"),
        input_schema: json!({
            "type": "object",
            "properties": {"x": {"type": "integer"}},
            "required": ["x"],
            "additionalProperties": false
        }),
        effect,
        approval_scope: ApprovalScope::Once,
    }
}

struct Harness {
    router: Router,
    approver: Arc<TestApprover>,
    runs: Arc<AtomicUsize>,
    check_runs: Arc<AtomicUsize>,
    dir: TempDir,
}

fn build_harness(name: &str, approve: bool, gate: Option<Arc<dyn Gate>>) -> Harness {
    let dir = TempDir::new(name);
    let runs = Arc::new(AtomicUsize::new(0));
    let check_runs = Arc::new(AtomicUsize::new(0));
    let mut registry = Registry::new();

    for (id, effect) in [
        ("read", EffectClass::ObserveLocal),
        ("step_record", EffectClass::ObserveLocal),
        ("run", EffectClass::ExecuteCode),
        ("forbidden", EffectClass::MutateIrreversible),
        ("fails", EffectClass::ObserveLocal),
    ] {
        let runs = runs.clone();
        let fails = id == "fails";
        registry.register(
            def(id, effect),
            Arc::new(move |_: &CallCtx<'_>, args: &Value| {
                runs.fetch_add(1, Ordering::SeqCst);
                if fails {
                    Err("handler broke".to_string())
                } else {
                    Ok(json!({ "tool": id, "echo": args["x"] }))
                }
            }),
        );
    }

    let cr = check_runs.clone();
    registry.register(
        def("check_run", EffectClass::ExecuteCode),
        Arc::new(move |_: &CallCtx<'_>, args: &Value| {
            cr.fetch_add(1, Ordering::SeqCst);
            Ok(json!({ "check": "identity_symbolic", "outcome": "pass", "echo": args["x"] }))
        }),
    );

    let rule = |tool: &str, decision| Rule {
        id: format!("r-{tool}"),
        caller: Match::Any,
        tool: Match::Exact(tool.into()),
        effect: None,
        tool_def_hash: None,
        decision,
    };
    let policy = Policy {
        version: "gate-test-1".into(),
        rules: vec![
            rule("read", Decision::Allow),
            rule("step_record", Decision::Allow),
            rule("fails", Decision::Allow),
            // ExecuteCode is always clamped to Ask by the policy engine.
            rule("run", Decision::Allow),
            rule("check_run", Decision::Allow),
        ],
        default: Decision::Deny,
    };

    let approver = TestApprover::new(approve);
    let mut router = Router::new(registry, policy, Ledger::open(&dir.0).unwrap(), "s", approver.clone());
    if let Some(g) = gate {
        router = router.with_gate(g);
    }
    Harness { router, approver, runs, check_runs, dir }
}

fn read_events(dir: &TempDir) -> Vec<ledger::Event> {
    let ledger = Ledger::open(&dir.0).unwrap();
    assert!(ledger.verify_session("s").is_ok(), "session hash chain must verify");
    ledger.read_session("s").unwrap()
}

fn event_kinds(dir: &TempDir) -> Vec<String> {
    read_events(dir).into_iter().map(|e| e.kind).collect()
}

#[derive(Default)]
struct RecordingGate {
    block_tools: Vec<String>,
    gate_id: String,
    message: String,
    before_calls: AtomicUsize,
    after_calls: AtomicUsize,
    seen_requests: Mutex<Vec<(String, String, EffectClass, ApprovalScope)>>,
    outcomes: Mutex<Vec<(String, String, CallOutcome)>>,
}

impl RecordingGate {
    fn allow_all() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn blocking(tools: &[&str], gate_id: &str, message: &str) -> Arc<Self> {
        Arc::new(Self {
            block_tools: tools.iter().map(|s| (*s).to_string()).collect(),
            gate_id: gate_id.into(),
            message: message.into(),
            ..Self::default()
        })
    }
}

impl Gate for RecordingGate {
    fn before(&self, req: &GateRequest<'_>) -> GateDecision {
        self.before_calls.fetch_add(1, Ordering::SeqCst);
        self.seen_requests.lock().unwrap().push((
            req.caller.to_string(),
            req.tool_id.to_string(),
            req.effect,
            req.approval_scope,
        ));
        if self.block_tools.iter().any(|t| t == req.tool_id) {
            GateDecision::block(&self.gate_id, &self.message)
        } else {
            GateDecision::Allow
        }
    }

    fn after_success(&self, _ctx: &mut PostCallCtx<'_>) -> Result<(), String> {
        self.after_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn on_outcome(&self, req: &GateRequest<'_>, outcome: &CallOutcome) {
        self.outcomes.lock().unwrap().push((req.caller.to_string(), req.tool_id.to_string(), outcome.clone()));
    }
}

#[test]
fn router_without_gate_preserves_existing_behavior() {
    let h = build_harness("no-gate", true, None);
    assert!(h.router.gate().is_none());
    assert_eq!(
        h.router.try_call("c", "read", &json!({"x": 3})).unwrap(),
        CallOutcome::Ok(json!({"tool": "read", "echo": 3}))
    );
    assert_eq!(event_kinds(&h.dir), ["tool.request", "policy.decision", "tool.result"]);
}

#[test]
fn gate_refusal_records_guardrail_blocked_formats_error_and_short_circuits() {
    let msg = "interpret pending runs before computing again";
    let gate = RecordingGate::blocking(&["run"], "G1", msg);
    let h = build_harness("refusal", true, Some(gate.clone()));
    assert!(h.router.gate().is_some());

    let out = h.router.try_call("agent:anthropic:s1", "run", &json!({"x": 7})).unwrap();
    let expected_err = format!("{GATE_ERROR_PREFIX} G1: {msg}");
    assert_eq!(out, CallOutcome::Error(expected_err.clone()));
    assert_eq!(h.approver.asked.load(Ordering::SeqCst), 0);
    assert_eq!(h.runs.load(Ordering::SeqCst), 0);
    assert_eq!(gate.after_calls.load(Ordering::SeqCst), 0);

    let events = read_events(&h.dir);
    let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(kinds, ["tool.request", GUARDRAIL_BLOCKED_EVENT, "tool.result"]);

    let call_id = events[0].payload["call_id"].as_str().unwrap();
    assert_eq!(
        events[1].payload,
        json!({ "gate": "G1", "tool": "run", "call_id": call_id, "message": msg })
    );
    assert_eq!(events[2].payload["call_id"], json!(call_id));
    assert_eq!(events[2].payload["ok"], json!(false));
    assert_eq!(events[2].payload["error"]["inline"], json!(expected_err));

    let outcomes = gate.outcomes.lock().unwrap();
    assert_eq!(outcomes.as_slice(), &[("agent:anthropic:s1".into(), "run".into(), CallOutcome::Error(expected_err))]);
}

#[test]
fn r11_gate_allow_cannot_widen_policy_deny() {
    let gate = RecordingGate::allow_all();
    let h = build_harness("r11-deny", true, Some(gate.clone()));

    let out = h.router.try_call("agent:anthropic:s1", "forbidden", &json!({"x": 1})).unwrap();
    assert!(matches!(out, CallOutcome::Denied(_)), "expected Denied, got {out:?}");
    assert_eq!(h.runs.load(Ordering::SeqCst), 0);
    assert_eq!(h.approver.asked.load(Ordering::SeqCst), 0);
    assert_eq!(gate.before_calls.load(Ordering::SeqCst), 1);
    assert_eq!(gate.after_calls.load(Ordering::SeqCst), 0);
    assert_eq!(gate.outcomes.lock().unwrap().len(), 1);
    assert!(matches!(gate.outcomes.lock().unwrap()[0].2, CallOutcome::Denied(_)));
    assert_eq!(event_kinds(&h.dir), ["tool.request", "policy.decision", "tool.denied"]);
}

#[test]
fn r11_gate_allow_cannot_bypass_human_approval() {
    let gate = RecordingGate::allow_all();
    let h = build_harness("r11-approval", false, Some(gate.clone()));

    let out = h.router.try_call("agent:anthropic:s1", "run", &json!({"x": 9})).unwrap();
    assert_eq!(out, CallOutcome::Denied("approval: declined".into()));
    assert_eq!(h.approver.asked.load(Ordering::SeqCst), 1);
    assert_eq!(h.runs.load(Ordering::SeqCst), 0);
    assert_eq!(gate.after_calls.load(Ordering::SeqCst), 0);
    assert_eq!(gate.outcomes.lock().unwrap()[0].2, CallOutcome::Denied("approval: declined".into()));
    assert_eq!(
        event_kinds(&h.dir),
        ["tool.request", "policy.decision", "approval.requested", "approval.denied", "tool.denied"]
    );
}

#[test]
fn r11_gate_binds_in_core_mcp_and_delegated_worker_callers() {
    let gate = RecordingGate::blocking(&["read"], "G1", "record pending runs first");
    let h = build_harness("r11-routes", true, Some(gate.clone()));
    let callers = ["agent:anthropic:s1", "mcp:claude-cli:s1", "worker:delegate:s1"];

    for (idx, caller) in callers.iter().enumerate() {
        let out = if idx == 1 {
            h.router.call(caller, "read", &json!({"x": idx as i64}))
        } else {
            h.router.try_call(caller, "read", &json!({"x": idx as i64})).unwrap()
        };
        assert_eq!(out, CallOutcome::Error("GATE G1: record pending runs first".into()));
    }
    assert_eq!(h.runs.load(Ordering::SeqCst), 0);
    assert_eq!(gate.before_calls.load(Ordering::SeqCst), 3);

    let events = read_events(&h.dir);
    let blocked: Vec<_> = events.iter().filter(|e| e.kind == GUARDRAIL_BLOCKED_EVENT).collect();
    assert_eq!(blocked.len(), 3);
    let seen_callers: Vec<String> =
        gate.seen_requests.lock().unwrap().iter().map(|r| r.0.clone()).collect();
    assert_eq!(seen_callers, callers);
}

struct AutoCheckGate {
    nested_tool: String,
    block_nested: bool,
    seen_before: Mutex<Vec<(String, String)>>,
    seen_outcomes: Mutex<Vec<(String, String, CallOutcome)>>,
}

impl AutoCheckGate {
    fn new(nested_tool: &str, block_nested: bool) -> Arc<Self> {
        Arc::new(Self {
            nested_tool: nested_tool.into(),
            block_nested,
            seen_before: Mutex::new(Vec::new()),
            seen_outcomes: Mutex::new(Vec::new()),
        })
    }
}

impl Gate for AutoCheckGate {
    fn before(&self, req: &GateRequest<'_>) -> GateDecision {
        self.seen_before.lock().unwrap().push((req.caller.to_string(), req.tool_id.to_string()));
        if self.block_nested && req.tool_id == self.nested_tool {
            return GateDecision::block("G9", "nested check blocked");
        }
        GateDecision::Allow
    }

    fn after_success(&self, ctx: &mut PostCallCtx<'_>) -> Result<(), String> {
        if ctx.tool_id != "step_record" {
            return Ok(());
        }
        let nested = ctx.router.try_call("host:check", &self.nested_tool, ctx.args)?;
        let entry = match nested {
            CallOutcome::Ok(val) => val,
            CallOutcome::Denied(why) => json!({ "check": self.nested_tool, "outcome": "not_run", "why": why }),
            CallOutcome::Error(err) => json!({ "check": self.nested_tool, "outcome": "error", "error": err }),
        };
        if let Some(obj) = ctx.result.as_object_mut() {
            obj.insert("auto_checks".into(), json!([entry]));
        }
        Ok(())
    }

    fn on_outcome(&self, req: &GateRequest<'_>, outcome: &CallOutcome) {
        self.seen_outcomes
            .lock()
            .unwrap()
            .push((req.caller.to_string(), req.tool_id.to_string(), outcome.clone()));
    }
}

#[test]
fn post_call_hook_mutates_result_and_runs_reentrant_call_through_full_pipeline() {
    let gate = AutoCheckGate::new("check_run", false);
    let h = build_harness("post-call-ok", true, Some(gate.clone()));

    let out = h.router.try_call("agent:anthropic:s1", "step_record", &json!({"x": 42})).unwrap();
    let expected = json!({
        "tool": "step_record",
        "echo": 42,
        "auto_checks": [{ "check": "identity_symbolic", "outcome": "pass", "echo": 42 }]
    });
    assert_eq!(out, CallOutcome::Ok(expected.clone()));
    assert_eq!(h.runs.load(Ordering::SeqCst), 1);
    assert_eq!(h.check_runs.load(Ordering::SeqCst), 1);
    assert_eq!(h.approver.asked.load(Ordering::SeqCst), 1);
    assert_eq!(*h.approver.seen.lock().unwrap(), vec![("host:check".into(), "check_run".into())]);
    assert_eq!(
        *gate.seen_before.lock().unwrap(),
        vec![("agent:anthropic:s1".into(), "step_record".into()), ("host:check".into(), "check_run".into())]
    );

    // Outer tool.result in ledger contains the mutated auto_checks payload.
    let events = read_events(&h.dir);
    let outer_req_id = events[0].payload["call_id"].as_str().unwrap();
    let outer_result = events
        .iter()
        .find(|e| e.kind == "tool.result" && e.payload["call_id"] == json!(outer_req_id))
        .expect("outer tool.result recorded");
    let decoded: Value = serde_json::from_str(outer_result.payload["result"]["inline"].as_str().unwrap()).unwrap();
    assert_eq!(decoded, expected);

    // Outcomes fire inner-first, then outer with the mutated result.
    let outcomes = gate.seen_outcomes.lock().unwrap();
    assert_eq!(outcomes.len(), 2);
    assert_eq!((outcomes[0].0.as_str(), outcomes[0].1.as_str()), ("host:check", "check_run"));
    assert_eq!((outcomes[1].0.as_str(), outcomes[1].1.as_str()), ("agent:anthropic:s1", "step_record"));
    assert_eq!(outcomes[1].2, CallOutcome::Ok(expected));
}

#[test]
fn post_call_reentrant_call_fails_closed_on_declined_approval_policy_deny_and_gate_block() {
    // 1. Declined approval on nested ExecuteCode check_run -> not_run, check handler never executes.
    let gate_declined = AutoCheckGate::new("check_run", false);
    let h1 = build_harness("post-call-declined", false, Some(gate_declined));
    let out1 = h1.router.try_call("agent:anthropic:s1", "step_record", &json!({"x": 1})).unwrap();
    assert_eq!(h1.check_runs.load(Ordering::SeqCst), 0);
    assert_eq!(h1.approver.asked.load(Ordering::SeqCst), 1);
    let CallOutcome::Ok(v1) = out1 else { panic!("expected Ok, got {out1:?}") };
    assert_eq!(v1["auto_checks"][0]["outcome"], json!("not_run"));
    let _ = read_events(&h1.dir);

    // 2. Policy denies nested tool -> check handler never executes and approver is never asked.
    let gate_denied = AutoCheckGate::new("forbidden", false);
    let h2 = build_harness("post-call-policy-deny", true, Some(gate_denied));
    let out2 = h2.router.try_call("agent:anthropic:s1", "step_record", &json!({"x": 2})).unwrap();
    assert_eq!(h2.runs.load(Ordering::SeqCst), 1); // only step_record ran
    assert_eq!(h2.approver.asked.load(Ordering::SeqCst), 0);
    let CallOutcome::Ok(v2) = out2 else { panic!("expected Ok, got {out2:?}") };
    assert_eq!(v2["auto_checks"][0]["outcome"], json!("not_run"));
    let _ = read_events(&h2.dir);

    // 3. Gate blocks nested tool -> guardrail.blocked recorded, approver and handler never run.
    let gate_blocked = AutoCheckGate::new("check_run", true);
    let h3 = build_harness("post-call-gate-block", true, Some(gate_blocked));
    let out3 = h3.router.try_call("agent:anthropic:s1", "step_record", &json!({"x": 3})).unwrap();
    assert_eq!(h3.check_runs.load(Ordering::SeqCst), 0);
    assert_eq!(h3.approver.asked.load(Ordering::SeqCst), 0);
    let CallOutcome::Ok(v3) = out3 else { panic!("expected Ok, got {out3:?}") };
    assert_eq!(v3["auto_checks"][0]["error"], json!("GATE G9: nested check blocked"));
    let events3 = read_events(&h3.dir);
    assert!(events3.iter().any(|e| e.kind == GUARDRAIL_BLOCKED_EVENT && e.payload["tool"] == json!("check_run")));
}

#[test]
fn post_call_hook_does_not_run_on_denied_errored_unknown_or_invalid_calls() {
    let gate = RecordingGate::blocking(&["read"], "G1", "blocked");
    let h = build_harness("no-post-call", false, Some(gate.clone()));

    // Unknown tool and invalid args stop before gate.before.
    assert!(matches!(h.router.try_call("c", "nonexistent", &json!({"x": 1})).unwrap(), CallOutcome::Denied(_)));
    assert!(matches!(h.router.try_call("c", "read", &json!({"x": "bad"})).unwrap(), CallOutcome::Error(_)));
    assert_eq!(gate.before_calls.load(Ordering::SeqCst), 0);

    // Gate-blocked, policy-denied, approval-declined, and handler-errored calls never run after_success.
    assert!(matches!(h.router.try_call("c", "read", &json!({"x": 1})).unwrap(), CallOutcome::Error(_)));
    assert!(matches!(h.router.try_call("c", "forbidden", &json!({"x": 1})).unwrap(), CallOutcome::Denied(_)));
    assert!(matches!(h.router.try_call("c", "run", &json!({"x": 1})).unwrap(), CallOutcome::Denied(_)));
    assert_eq!(
        h.router.try_call("c", "fails", &json!({"x": 1})).unwrap(),
        CallOutcome::Error("handler broke".into())
    );
    assert_eq!(gate.before_calls.load(Ordering::SeqCst), 4);
    assert_eq!(gate.after_calls.load(Ordering::SeqCst), 0);
    let _ = read_events(&h.dir);
}

#[test]
fn post_call_hook_error_propagates_fail_closed_from_try_call() {
    struct FailingPostHook;
    impl Gate for FailingPostHook {
        fn before(&self, _req: &GateRequest<'_>) -> GateDecision {
            GateDecision::Allow
        }
        fn after_success(&self, _ctx: &mut PostCallCtx<'_>) -> Result<(), String> {
            Err("nested ledger write failed".into())
        }
    }

    let h = build_harness("post-call-err", true, Some(Arc::new(FailingPostHook)));
    let err = h.router.try_call("c", "read", &json!({"x": 5})).unwrap_err();
    assert_eq!(err, "nested ledger write failed");
    // No tool.result was written because the post-call infrastructure error aborted the call.
    let events = read_events(&h.dir);
    let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(kinds, ["tool.request", "policy.decision"]);
    // Router::call collapses Err into CallOutcome::Error.
    assert_eq!(h.router.call("c", "read", &json!({"x": 5})), CallOutcome::Error("nested ledger write failed".into()));
}

#[test]
fn gate_chain_stops_at_first_block_runs_after_in_order_and_notifies_all_outcomes() {
    struct TraceGate {
        tag: &'static str,
        block_tool: Option<&'static str>,
        log: Arc<Mutex<Vec<String>>>,
    }
    impl Gate for TraceGate {
        fn before(&self, req: &GateRequest<'_>) -> GateDecision {
            self.log.lock().unwrap().push(format!("before:{}", self.tag));
            if self.block_tool == Some(req.tool_id) {
                GateDecision::Block(GateRefusal::new(self.tag, format!("stopped by {}", self.tag)))
            } else {
                GateDecision::Allow
            }
        }
        fn after_success(&self, ctx: &mut PostCallCtx<'_>) -> Result<(), String> {
            self.log.lock().unwrap().push(format!("after:{}", self.tag));
            let arr = ctx
                .result
                .as_object_mut()
                .unwrap()
                .entry("trace")
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .unwrap();
            arr.push(json!(self.tag));
            Ok(())
        }
        fn on_outcome(&self, _req: &GateRequest<'_>, _outcome: &CallOutcome) {
            self.log.lock().unwrap().push(format!("outcome:{}", self.tag));
        }
    }

    let log = Arc::new(Mutex::new(Vec::new()));
    let mut chain = GateChain::new(Vec::new());
    assert!(chain.is_empty());
    chain.push(Arc::new(TraceGate { tag: "G1", block_tool: None, log: log.clone() }));
    chain.push(Arc::new(TraceGate { tag: "G2", block_tool: Some("run"), log: log.clone() }));
    chain.push(Arc::new(TraceGate { tag: "G3", block_tool: Some("run"), log: log.clone() }));
    assert_eq!(chain.len(), 3);

    let h = build_harness("chain", true, Some(Arc::new(chain)));

    // Blocked at G2: G3.before does not run, but all three receive on_outcome.
    let blocked = h.router.try_call("c", "run", &json!({"x": 1})).unwrap();
    assert_eq!(blocked, CallOutcome::Error("GATE G2: stopped by G2".into()));
    assert_eq!(
        *log.lock().unwrap(),
        vec!["before:G1", "before:G2", "outcome:G1", "outcome:G2", "outcome:G3"]
    );

    // Allowed: all three run before, after_success, and on_outcome in order.
    log.lock().unwrap().clear();
    let ok = h.router.try_call("c", "read", &json!({"x": 2})).unwrap();
    assert_eq!(ok, CallOutcome::Ok(json!({ "tool": "read", "echo": 2, "trace": ["G1", "G2", "G3"] })));
    assert_eq!(
        *log.lock().unwrap(),
        vec![
            "before:G1", "before:G2", "before:G3",
            "after:G1", "after:G2", "after:G3",
            "outcome:G1", "outcome:G2", "outcome:G3"
        ]
    );
    let _ = read_events(&h.dir);
}

#[test]
fn ledger_failure_recording_guardrail_blocked_fails_closed() {
    struct PoisonOnBefore {
        router_slot: OnceLock<Arc<Router>>,
        outcomes: AtomicUsize,
    }
    impl Gate for PoisonOnBefore {
        fn before(&self, _req: &GateRequest<'_>) -> GateDecision {
            // `tool.request` already succeeded; poison the ledger mutex right before
            // `Router` records `guardrail.blocked`.
            let router = self.router_slot.get().unwrap().clone();
            let _ = std::thread::spawn(move || {
                let _guard = router.ledger.lock().unwrap();
                panic!("simulated ledger failure");
            })
            .join();
            GateDecision::block("G1", "blocked after ledger failure")
        }
        fn on_outcome(&self, _req: &GateRequest<'_>, _outcome: &CallOutcome) {
            self.outcomes.fetch_add(1, Ordering::SeqCst);
        }
    }

    let gate = Arc::new(PoisonOnBefore { router_slot: OnceLock::new(), outcomes: AtomicUsize::new(0) });
    let h = build_harness("ledger-fail-guardrail", true, Some(gate.clone()));
    let router = Arc::new(h.router);
    let _ = gate.router_slot.set(router.clone());

    let err = router.try_call("c", "run", &json!({"x": 1})).unwrap_err();
    assert!(err.contains("ledger lock poisoned"), "unexpected error: {err}");
    assert_eq!(h.approver.asked.load(Ordering::SeqCst), 0);
    assert_eq!(h.runs.load(Ordering::SeqCst), 0);
    assert_eq!(gate.outcomes.load(Ordering::SeqCst), 0);
}

#[test]
fn blob_failure_recording_gate_error_result_fails_closed() {
    let huge_msg = "x".repeat(5000);
    let gate = RecordingGate::blocking(&["read"], "G1", &huge_msg);
    let h = build_harness("blob-fail-guardrail", true, Some(gate));

    let blobs = h.dir.0.join("blobs");
    let _ = std::fs::remove_dir_all(&blobs);
    std::fs::write(&blobs, b"not-a-directory").unwrap();

    let res = h.router.try_call("c", "read", &json!({"x": 1}));
    assert!(res.is_err(), "expected Err when finish_error cannot store blob, got {res:?}");
    assert_eq!(h.runs.load(Ordering::SeqCst), 0);
}
