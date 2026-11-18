use super::*;
use crate::approval::ApprovalReply;
use crate::registry::ToolDef;
use policy::{EffectClass, Match, Rule};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

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

struct Fixture {
    router: Router,
    approver: Arc<MockApprover>,
    runs: Arc<AtomicUsize>,
    dir: PathBuf,
}

fn tool(id: &str, effect: EffectClass) -> ToolDef {
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
    }
}

fn fixture(name: &str, mode: Mode) -> Fixture {
    let dir = std::env::temp_dir().join(format!("tools-test-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let runs = Arc::new(AtomicUsize::new(0));
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
            tool(id, effect),
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
    let rule = |tool: &str, decision| Rule {
        id: format!("r-{tool}"),
        caller: Match::Any,
        tool: Match::Exact(tool.into()),
        effect: None,
        tool_def_hash: None,
        decision,
    };
    let policy = Policy {
        version: "test-1".into(),
        // "run" is listed as allow: the engine must still clamp it to ask.
        rules: vec![rule("read", Decision::Allow), rule("fails", Decision::Allow), rule("run", Decision::Allow)],
        default: Decision::Deny,
    };
    let approver = Arc::new(MockApprover { mode, asked: AtomicUsize::new(0) });
    let router = Router::new(registry, policy, Ledger::open(&dir).unwrap(), "s", approver.clone());
    Fixture { router, approver, runs, dir }
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
fn execute_code_asks_and_runs_once_when_approved() {
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
    // A second identical call needs a second approval: nothing is remembered.
    assert!(matches!(f.router.call("c", "run", &json!({"x": 7})), CallOutcome::Ok(_)));
    assert_eq!(f.approver.asked.load(Ordering::SeqCst), 2);
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
    let digest = approval_digest("c", "run", &def_hash, &canonical_json(&args), "test-1", "nonce-1");
    f.router.store.grant(&digest);

    // Same grant, different arguments / caller / nonce: refused, grant untouched.
    for (caller, a, nonce) in [("c", json!({"x": 8}), "nonce-1"), ("other", args.clone(), "nonce-1"), ("c", args.clone(), "nonce-2")] {
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
