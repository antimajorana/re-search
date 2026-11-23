use super::*;
use crate::approval::{ApprovalReply, ApprovalScope, WorkerEpoch};
use crate::registry::ToolDef;
use policy::{EffectClass, Match, Rule};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

#[derive(Clone, Copy)]
enum Mode {
    Approve,
    Decline,
    /// Says yes, but for a digest other than the one it was asked about.
    WrongDigest,
}

struct MockApprover {
    mode: Mode,
    asked: AtomicUsize,
}

impl Approver for MockApprover {
    fn decide(&self, request: &ApprovalRequest) -> ApprovalReply {
        self.asked.fetch_add(1, Ordering::SeqCst);
        let (approved, digest) = match self.mode {
            Mode::Approve => (true, request.digest.clone()),
            Mode::Decline => (false, request.digest.clone()),
            Mode::WrongDigest => (true, "0".repeat(64)),
        };
        ApprovalReply { approved, digest, by: "mock".into() }
    }
}

struct MockEpoch {
    epoch: AtomicU64,
    contained: AtomicBool,
    fail_next: Mutex<bool>,
}

impl MockEpoch {
    fn new() -> Self {
        MockEpoch { epoch: AtomicU64::new(0), contained: AtomicBool::new(true), fail_next: Mutex::new(false) }
    }

    fn set_dead(&self) {
        self.epoch.store(0, Ordering::Release);
    }

    fn set_epoch(&self, ep: u64) {
        self.epoch.store(ep, Ordering::Release);
    }

    fn set_contained(&self, contained: bool) {
        self.contained.store(contained, Ordering::Release);
    }

    fn set_fail_next(&self, fail: bool) {
        *self.fail_next.lock().unwrap() = fail;
    }
}

impl WorkerEpoch for MockEpoch {
    fn live_epoch(&self) -> Option<u64> {
        match self.epoch.load(Ordering::Acquire) {
            0 => None,
            n => Some(n),
        }
    }

    fn is_contained(&self) -> bool {
        self.contained.load(Ordering::Acquire)
    }
}

struct Fixture {
    router: Router,
    approver: Arc<MockApprover>,
    runs: Arc<AtomicUsize>,
    epoch: Arc<MockEpoch>,
    dir: PathBuf,
}

fn tool(id: &str, effect: EffectClass, approval_scope: ApprovalScope) -> ToolDef {
    ToolDef {
        id: id.into(),
        description: "test tool".into(),
        input_schema: json!({
            "type": "object",
            "properties": {"x": {"type": "integer"}},
            "required": ["x"],
            "additionalProperties": false
        }),
        effect,
        approval_scope,
    }
}

fn fixture_with_policy(name: &str, mode: Mode, policy_version: &str, sess_tool_def: ToolDef) -> Fixture {
    let dir = std::env::temp_dir().join(format!("tools-test-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let runs = Arc::new(AtomicUsize::new(0));
    let epoch = Arc::new(MockEpoch::new());
    let mut registry = Registry::new();
    for (id, effect) in [
        ("read", EffectClass::ObserveLocal),
        ("run", EffectClass::ExecuteCode),
        ("forbidden", EffectClass::MutateIrreversible),
        ("fails", EffectClass::ObserveLocal),
    ] {
        let runs = runs.clone();
        let fails = id == "fails";
        registry.register(
            tool(id, effect, ApprovalScope::Once),
            Arc::new(move |_: &CallCtx<'_>, args: &Value| {
                runs.fetch_add(1, Ordering::SeqCst);
                if fails {
                    Err("handler broke".to_string())
                } else {
                    Ok(json!({ "echo": args["x"] }))
                }
            }),
        );
    }
    let runs_s = runs.clone();
    let ep_s = epoch.clone();
    let sess_id = sess_tool_def.id.clone();
    registry.register_worker_session(
        sess_tool_def,
        epoch.clone(),
        Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
            let request_id = runs_s.fetch_add(1, Ordering::SeqCst) as u64 + 1;
            let mut fail = ep_s.fail_next.lock().unwrap();
            if *fail {
                *fail = false;
                ep_s.set_dead();
                return Err("worker failed before/during call".to_string());
            }
            let current = ep_s.live_epoch();
            if let Some(req) = ctx.required_epoch {
                if current != Some(req) {
                    return Err(format!("epoch mismatch: expected {req}, got {current:?}"));
                }
                ctx.record_worker_execution(req, request_id);
            } else {
                let next = current.unwrap_or(1);
                ep_s.set_epoch(next);
                ctx.record_worker_execution(next, request_id);
            }
            Ok(json!({ "echo": args["x"] }))
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
        version: policy_version.into(),
        // "run" and `sess_id` are listed as allow: the engine must still clamp them to ask.
        rules: vec![
            rule("read", Decision::Allow),
            rule("fails", Decision::Allow),
            rule("run", Decision::Allow),
            rule(&sess_id, Decision::Allow),
        ],
        default: Decision::Deny,
    };
    let approver = Arc::new(MockApprover { mode, asked: AtomicUsize::new(0) });
    let router = Router::new(registry, policy, Ledger::open(&dir).unwrap(), "s", approver.clone());
    Fixture { router, approver, runs, epoch, dir }
}

fn fixture(name: &str, mode: Mode) -> Fixture {
    fixture_with_policy(name, mode, "test-1", tool("sess", EffectClass::ExecuteCode, ApprovalScope::WorkerSession))
}

fn kinds(f: &Fixture) -> Vec<String> {
    let ledger = Ledger::open(&f.dir).unwrap();
    ledger.read_session("s").unwrap().into_iter().map(|e| e.kind).collect()
}

#[test]
fn allowed_call_runs_and_is_recorded() {
    let f = fixture("allow", Mode::Decline);
    assert_eq!(f.router.call("c", "read", &json!({"x": 1})), CallOutcome::Ok(json!({"echo": 1})));
    assert_eq!(f.approver.asked.load(Ordering::SeqCst), 0);
    assert_eq!(kinds(&f), ["tool.request", "policy.decision", "tool.result"]);
}

#[test]
fn denied_and_unknown_tools_never_run() {
    let f = fixture("deny", Mode::Approve);
    assert!(matches!(f.router.call("c", "forbidden", &json!({"x": 1})), CallOutcome::Denied(_)));
    assert!(matches!(f.router.call("c", "nonexistent", &json!({"x": 1})), CallOutcome::Denied(_)));
    assert_eq!(f.runs.load(Ordering::SeqCst), 0);
    assert_eq!(
        kinds(&f),
        ["tool.request", "policy.decision", "tool.denied", "tool.request", "tool.denied"]
    );
}

#[test]
fn bad_arguments_stop_before_policy() {
    let f = fixture("badargs", Mode::Approve);
    assert!(matches!(f.router.call("c", "run", &json!({"x": "one"})), CallOutcome::Error(_)));
    assert!(matches!(f.router.call("c", "run", &json!({"x": 1, "y": 2})), CallOutcome::Error(_)));
    assert_eq!(f.runs.load(Ordering::SeqCst), 0);
    assert_eq!(f.approver.asked.load(Ordering::SeqCst), 0);
}

#[test]
fn execute_code_once_asks_every_call() {
    let f = fixture("ask-yes", Mode::Approve);
    assert_eq!(f.router.call("c", "run", &json!({"x": 7})), CallOutcome::Ok(json!({"echo": 7})));
    assert_eq!(f.approver.asked.load(Ordering::SeqCst), 1);
    assert_eq!(f.runs.load(Ordering::SeqCst), 1);
    assert_eq!(
        kinds(&f),
        [
            "tool.request",
            "policy.decision",
            "approval.requested",
            "approval.granted",
            "approval.consumed",
            "tool.result"
        ]
    );
    // A second identical call for a Once-scoped tool needs a second approval.
    assert!(matches!(f.router.call("c", "run", &json!({"x": 7})), CallOutcome::Ok(_)));
    assert_eq!(f.approver.asked.load(Ordering::SeqCst), 2);
}

#[test]
fn worker_session_asks_once_and_reuses_until_worker_restarts() {
    let f = fixture("sess-reuse", Mode::Approve);
    assert_eq!(f.router.call("c", "sess", &json!({"x": 1})), CallOutcome::Ok(json!({"echo": 1})));
    assert_eq!(f.approver.asked.load(Ordering::SeqCst), 1);

    // Second call with different arguments on the same live epoch reuses the approval.
    assert_eq!(f.router.call("c", "sess", &json!({"x": 2})), CallOutcome::Ok(json!({"echo": 2})));
    assert_eq!(f.approver.asked.load(Ordering::SeqCst), 1);
    assert_eq!(f.runs.load(Ordering::SeqCst), 2);
    assert_eq!(
        kinds(&f),
        [
            "tool.request",
            "policy.decision",
            "approval.requested",
            "approval.granted",
            "approval.consumed",
            "tool.result",
            "approval.session_bound",
            "tool.request",
            "policy.decision",
            "approval.session_reused",
            "tool.result",
        ]
    );

    // Different caller cannot reuse caller "c"'s session grant even while epoch 1 is live.
    assert_eq!(f.router.call("other", "sess", &json!({"x": 3})), CallOutcome::Ok(json!({"echo": 3})));
    assert_eq!(f.approver.asked.load(Ordering::SeqCst), 2);

    // Worker dies and a new epoch (2) starts: caller "c" must be asked again.
    f.epoch.set_dead();
    f.epoch.set_epoch(2);
    assert_eq!(f.router.call("c", "sess", &json!({"x": 4})), CallOutcome::Ok(json!({"echo": 4})));
    assert_eq!(f.approver.asked.load(Ordering::SeqCst), 3);
}

#[test]
fn uncontained_worker_forces_per_call_once_approval() {
    let f = fixture("uncontained-once", Mode::Approve);
    f.epoch.set_contained(false);

    assert_eq!(f.router.call("c", "sess", &json!({"x": 1})), CallOutcome::Ok(json!({"echo": 1})));
    assert_eq!(f.approver.asked.load(Ordering::SeqCst), 1);
    assert_eq!(f.epoch.live_epoch(), Some(1));

    // Even while the same worker epoch stays live, uncontained execution must ask again.
    assert_eq!(f.router.call("c", "sess", &json!({"x": 2})), CallOutcome::Ok(json!({"echo": 2})));
    assert_eq!(f.approver.asked.load(Ordering::SeqCst), 2);
    assert_eq!(f.runs.load(Ordering::SeqCst), 2);
    assert_eq!(
        kinds(&f),
        [
            "tool.request",
            "policy.decision",
            "approval.requested",
            "approval.granted",
            "approval.consumed",
            "tool.result",
            "tool.request",
            "policy.decision",
            "approval.requested",
            "approval.granted",
            "approval.consumed",
            "tool.result",
        ]
    );

    let ledger = Ledger::open(&f.dir).unwrap();
    let events = ledger.read_session("s").unwrap();
    assert!(!events.iter().any(|e| e.kind == "approval.session_bound"));
    assert!(!events.iter().any(|e| e.kind == "approval.session_reused"));
    let executions: Vec<_> = events
        .iter()
        .filter(|e| e.kind == "tool.result")
        .map(|e| &e.payload["worker_execution"])
        .collect();
    let instance = executions[0]["instance"].as_str().unwrap();
    assert!(!instance.is_empty());
    assert_eq!(executions[0], &json!({ "instance": instance, "epoch": 1, "request_id": 1 }));
    assert_eq!(executions[1], &json!({ "instance": instance, "epoch": 1, "request_id": 2 }));
    for e in &events {
        match e.kind.as_str() {
            "tool.request" => assert_eq!(e.payload["approval_scope"], "once"),
            "approval.requested" | "approval.granted" => assert_eq!(e.payload["scope"], "once"),
            _ => {}
        }
    }
}

#[test]
fn worker_session_does_not_bind_when_initial_call_fails() {
    let f = fixture("sess-fail", Mode::Approve);
    f.epoch.set_fail_next(true);
    assert!(matches!(f.router.call("c", "sess", &json!({"x": 1})), CallOutcome::Error(_)));
    assert_eq!(f.approver.asked.load(Ordering::SeqCst), 1);
    assert_eq!(f.epoch.live_epoch(), None);

    // Next call must prompt again because no worker session became live.
    assert_eq!(f.router.call("c", "sess", &json!({"x": 2})), CallOutcome::Ok(json!({"echo": 2})));
    assert_eq!(f.approver.asked.load(Ordering::SeqCst), 2);
}

#[test]
fn worker_session_dispatch_rejects_stale_grant() {
    let f = fixture("sess-stale-dispatch", Mode::Approve);
    let args = json!({"x": 5});
    f.epoch.set_epoch(1);
    let def_hash = f.router.registry().get("sess").unwrap().0.def_hash();
    f.router.store.bind_session_grant(
        "c",
        "sess",
        SessionGrant {
            tool_def_hash: def_hash,
            policy_version: "test-1".into(),
            worker_epoch: 1,
            grant_digest: "d".into(),
            granted_call_id: "c0".into(),
        },
    );
    // Worker died before dispatch ran: must refuse rather than respawn.
    f.epoch.set_dead();
    let out = f.router.dispatch_auth("c", "sess", &args, DispatchAuth::Session(1), "id").unwrap();
    assert!(matches!(out, CallOutcome::Denied(_)), "{out:?}");
    assert_eq!(f.runs.load(Ordering::SeqCst), 0);
}

#[test]
fn declined_approval_does_not_run() {
    let f = fixture("ask-no", Mode::Decline);
    assert!(matches!(f.router.call("c", "run", &json!({"x": 7})), CallOutcome::Denied(_)));
    assert_eq!(f.runs.load(Ordering::SeqCst), 0);
    assert_eq!(
        kinds(&f),
        ["tool.request", "policy.decision", "approval.requested", "approval.denied", "tool.denied"]
    );
}

#[test]
fn approval_for_a_different_digest_does_not_run() {
    let f = fixture("ask-wrong", Mode::WrongDigest);
    assert!(matches!(f.router.call("c", "run", &json!({"x": 7})), CallOutcome::Denied(_)));
    assert_eq!(f.runs.load(Ordering::SeqCst), 0);
}

#[test]
fn a_grant_cannot_be_replayed_or_moved_to_another_call() {
    let f = fixture("replay", Mode::Approve);
    let args = json!({"x": 7});
    let def_hash = f.router.registry().get("run").unwrap().0.def_hash();
    let digest = approval_digest(
        ApprovalScope::Once,
        "c",
        "run",
        &def_hash,
        &canonical_json(&args),
        "test-1",
        "nonce-1",
    );
    f.router.store.grant(&digest);

    // Same grant, different arguments / caller / nonce: refused, grant untouched.
    for (caller, a, nonce) in
        [("c", json!({"x": 8}), "nonce-1"), ("other", args.clone(), "nonce-1"), ("c", args.clone(), "nonce-2")]
    {
        let out = f.router.dispatch(caller, "run", &a, Some(nonce), "id").unwrap();
        assert!(matches!(out, CallOutcome::Denied(_)), "{out:?}");
    }
    assert_eq!(f.runs.load(Ordering::SeqCst), 0);

    // The exact call: runs once.
    assert!(matches!(f.router.dispatch("c", "run", &args, Some("nonce-1"), "id").unwrap(), CallOutcome::Ok(_)));
    // Replay of the exact call: refused.
    assert!(matches!(f.router.dispatch("c", "run", &args, Some("nonce-1"), "id").unwrap(), CallOutcome::Denied(_)));
    assert_eq!(f.runs.load(Ordering::SeqCst), 1);
}

#[test]
fn handler_errors_are_recorded() {
    let f = fixture("fails", Mode::Approve);
    assert_eq!(f.router.call("c", "fails", &json!({"x": 1})), CallOutcome::Error("handler broke".into()));
    let ledger = Ledger::open(&f.dir).unwrap();
    let last = ledger.read_session("s").unwrap().pop().unwrap();
    assert_eq!((last.kind.as_str(), &last.payload["ok"]), ("tool.result", &json!(false)));
}

#[test]
fn large_arguments_go_to_a_blob() {
    let f = fixture("blob", Mode::Approve);
    // An unknown tool still gets its request recorded; use it to carry big args.
    let big = "y".repeat(10_000);
    let _ = f.router.call("c", "nonexistent", &json!({ "big": big }));
    let ledger = Ledger::open(&f.dir).unwrap();
    let first = &ledger.read_session("s").unwrap()[0];
    let hash = first.payload["args"]["blob"].as_str().expect("args stored as blob");
    let stored = String::from_utf8(ledger.get_blob(hash).unwrap()).unwrap();
    assert_eq!(stored, canonical_json(&json!({ "big": big })));
}

#[test]
fn try_call_distinguishes_ledger_failure_from_handler_and_arg_errors() {
    let f = fixture("try-call", Mode::Approve);

    // Recorded argument validation failure -> Ok(CallOutcome::Error).
    assert!(matches!(f.router.try_call("c", "read", &json!({"x": "bad"})), Ok(CallOutcome::Error(_))));
    // Recorded handler runtime failure -> Ok(CallOutcome::Error).
    assert_eq!(f.router.try_call("c", "fails", &json!({"x": 1})), Ok(CallOutcome::Error("handler broke".into())));
    assert_eq!(f.runs.load(Ordering::SeqCst), 1);

    // Make blob storage unwritable mid-session so recording a >4 KiB request fails closed.
    let blobs = f.dir.join("blobs");
    let _ = std::fs::remove_dir_all(&blobs);
    std::fs::write(&blobs, b"not-a-directory").unwrap();

    let big_args = json!({ "big": "z".repeat(6000) });
    let res = f.router.try_call("c", "read", &big_args);
    assert!(res.is_err(), "expected ledger/blob failure Err, got {res:?}");
    assert_eq!(f.runs.load(Ordering::SeqCst), 1, "handler must not execute when request recording fails");
    assert!(matches!(f.router.call("c", "read", &big_args), CallOutcome::Error(_)));
    let _ = std::fs::remove_dir_all(&f.dir);
}

#[test]
fn policy_accessor_returns_configured_policy() {
    let f = fixture("policy-getter", Mode::Decline);
    assert_eq!((f.router.policy().version.as_str(), f.router.policy().default), ("test-1", Decision::Deny));
    let _ = std::fs::remove_dir_all(&f.dir);
}
