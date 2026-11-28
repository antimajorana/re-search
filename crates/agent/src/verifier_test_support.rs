//! Shared test harness for Solve verifier loop (S14) and blind compute (S20) tests.

use crate::transport::{
    ContentBlock, ModelRequest, ModelResponse, PreparedRequest, StopReason, StreamObserver,
    Transport, TransportError, TransportErrorKind, TransportInfo,
};
use crate::verifier_tools::{
    CheckRunnerBinding, IsolatedEvalOutput, IsolatedPythonWorker, IsolatedWorkerFactory,
};
use ledger::Ledger;
use research_tools::solve_checks::{CheckExecError, CheckExecResult, CheckProcessRunner};
use research_tools::solve_exprs::{CanonicalizedExpr, CheckCanonicalizer};
use research_tools::solve_model::{
    compute_claim_sha256, compute_entry_sha256, compute_math_sha256, event_kinds, Claim, ClaimForm,
    ClaimRelation, DoneCriterion, DoneCriterionKind, HandleRef, ReceiptRef, ReviewPolicy,
    RoleModel, SolveBudgets, SolveExpr, SolveRoles, SolveSetup, SolveTarget, StepEntryType,
    StepOrigin, SymbolSpec, TargetForm, TaskKind,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tools::{ApprovalReply, ApprovalRequest, Approver, WorkerEpoch};

pub struct AllowAllApprover;
impl Approver for AllowAllApprover {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        ApprovalReply {
            approved: true,
            digest: req.digest.clone(),
            by: "test".into(),
        }
    }
}

pub struct ScriptedTransport {
    responses: Mutex<Vec<ModelResponse>>,
    requests: Mutex<Vec<ModelRequest>>,
}

impl ScriptedTransport {
    pub fn new(responses: Vec<ModelResponse>) -> Self {
        Self {
            responses: Mutex::new(responses),
            requests: Mutex::new(Vec::new()),
        }
    }
    pub fn seen_requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Transport for ScriptedTransport {
    fn info(&self) -> TransportInfo {
        TransportInfo {
            provider: "scripted",
            endpoint: "http://127.0.0.1".into(),
            api_key_env: String::new(),
        }
    }
    fn prepare_request(&self, req: &ModelRequest) -> Result<PreparedRequest, TransportError> {
        self.requests.lock().unwrap().push(req.clone());
        Ok(PreparedRequest {
            body: serde_json::to_vec(req).unwrap(),
        })
    }
    fn send_prepared(&self, _prepared: &PreparedRequest) -> Result<ModelResponse, TransportError> {
        let mut guard = self.responses.lock().unwrap();
        if guard.is_empty() {
            return Err(TransportError {
                kind: TransportErrorKind::Network,
                status: None,
                message: "scripted transport exhausted".into(),
                raw_body: None,
            });
        }
        Ok(guard.remove(0))
    }
    fn send_prepared_streaming(
        &self,
        prepared: &PreparedRequest,
        _observer: &mut dyn StreamObserver,
    ) -> Result<ModelResponse, TransportError> {
        self.send_prepared(prepared)
    }
}

pub fn tool_use_resp(id: &str, name: &str, input: Value) -> ModelResponse {
    let raw = json!({ "tool": name, "input": input }).to_string().into_bytes();
    ModelResponse {
        id: Some(format!("resp-{id}")),
        model: "verifier-model".into(),
        content: vec![ContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }],
        stop_reason: StopReason::ToolUse,
        usage: json!({}),
        raw_body: raw,
    }
}

pub struct StubCheckEpoch;
impl WorkerEpoch for StubCheckEpoch {
    fn live_epoch(&self) -> Option<u64> {
        Some(1)
    }
    fn is_contained(&self) -> bool {
        true
    }
}

pub struct StubCheckRunner;
impl CheckProcessRunner for StubCheckRunner {
    fn run_check(
        &self,
        _name: &str,
        _handles: &BTreeMap<String, String>,
        _params: &Value,
        _timeout: Duration,
        _required_epoch: Option<u64>,
    ) -> Result<CheckExecResult, CheckExecError> {
        Ok(CheckExecResult {
            raw: json!({
                "outcome": "pass",
                "evidence": { "comparison": "exact", "exact": true },
                "registry_sha256": "reg-sha"
            }),
            epoch: 1,
            request_id: 1,
            environment: None,
        })
    }
}

pub struct StubCanonicalizer;
impl CheckCanonicalizer for StubCanonicalizer {
    fn canonicalize(&self, _raw: &str) -> Result<CanonicalizedExpr, String> {
        Err("unused".into())
    }
}

pub fn find_sympy_python() -> Result<String, String> {
    let mut candidates: Vec<String> = Vec::new();
    if let Ok(env_py) = std::env::var("RE_SEARCH_PYTHON") {
        let trimmed = env_py.trim().to_string();
        if !trimmed.is_empty() {
            candidates.push(trimmed);
        }
    }
    candidates.push("python3".to_string());
    candidates.push("python".to_string());
    for cand in &candidates {
        if let Ok(out) = Command::new(cand).args(["-c", "import sympy"]).output() {
            if out.status.success() {
                return Ok(cand.clone());
            }
        }
    }
    Err(format!(
        "no Python interpreter with sympy found (tried: {})",
        candidates.join(", ")
    ))
}

pub struct RealStdioPythonWorker {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    stdout: Mutex<BufReader<ChildStdout>>,
    next_id: AtomicUsize,
    role: String,
}

impl RealStdioPythonWorker {
    pub fn spawn() -> Result<Self, String> {
        Self::spawn_role("solver")
    }

    pub fn spawn_role(role: &str) -> Result<Self, String> {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let worker_py = repo_root.join("python").join("worker.py");
        let python = find_sympy_python()?;
        let mut child = Command::new(&python)
            .arg("-u")
            .arg(&worker_py)
            .arg("--stdio")
            .arg("--role")
            .arg(role)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("failed to spawn python worker ({python}): {e}"))?;
        let stdin = child.stdin.take().ok_or("missing stdin")?;
        let mut stdout = BufReader::new(child.stdout.take().ok_or("missing stdout")?);
        let mut hello_line = String::new();
        stdout
            .read_line(&mut hello_line)
            .map_err(|e| format!("failed to read worker hello: {e}"))?;
        let hello: Value = serde_json::from_str(&hello_line).map_err(|e| e.to_string())?;
        if hello.get("ready") != Some(&json!(true)) {
            return Err(format!("unexpected worker hello: {hello}"));
        }
        Ok(Self {
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            stdout: Mutex::new(stdout),
            next_id: AtomicUsize::new(1),
            role: role.to_string(),
        })
    }

    fn rpc(&self, method: &str, params: Value) -> Result<(u64, Value), String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) as u64;
        let req = format!("{}\n", json!({ "id": id, "method": method, "params": params }));
        {
            let mut stdin = self.stdin.lock().unwrap();
            stdin.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
            stdin.flush().map_err(|e| e.to_string())?;
        }
        let mut line = String::new();
        {
            let mut stdout = self.stdout.lock().unwrap();
            stdout.read_line(&mut line).map_err(|e| e.to_string())?;
        }
        let resp: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        if resp["ok"] != json!(true) {
            return Err(resp["error"].to_string());
        }
        Ok((id, resp["result"].clone()))
    }
}

impl Drop for RealStdioPythonWorker {
    fn drop(&mut self) {
        if let Ok(mut c) = self.child.lock() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl IsolatedPythonWorker for RealStdioPythonWorker {
    fn evaluate(
        &self,
        code: &str,
        exprs_in: Option<&BTreeMap<String, String>>,
        _timeout: Duration,
        _required_epoch: Option<u64>,
    ) -> Result<IsolatedEvalOutput, String> {
        let params = match exprs_in {
            Some(map) => json!({ "code": code, "exprs_in": map }),
            None => json!({ "code": code }),
        };
        let (id, result) = self.rpc("evaluate", params)?;
        Ok(IsolatedEvalOutput {
            result,
            epoch: 1,
            request_id: id,
            environment: Some(json!({ "protocol": 2, "role": self.role })),
        })
    }
    fn live_epoch(&self) -> Option<u64> {
        Some(1)
    }
    fn is_contained(&self) -> bool {
        true
    }
    fn containment_summary(&self) -> String {
        "test-stdio-python".into()
    }
}

impl CheckProcessRunner for RealStdioPythonWorker {
    fn run_check(
        &self,
        name: &str,
        handles: &BTreeMap<String, String>,
        params: &Value,
        _timeout: Duration,
        _required_epoch: Option<u64>,
    ) -> Result<CheckExecResult, CheckExecError> {
        let (id, raw) = self
            .rpc(
                "check",
                json!({ "name": name, "handles": handles, "params": params }),
            )
            .map_err(CheckExecError::Worker)?;
        Ok(CheckExecResult {
            raw,
            epoch: 1,
            request_id: id,
            environment: Some(json!({ "protocol": 2, "role": self.role })),
        })
    }
}

impl CheckCanonicalizer for RealStdioPythonWorker {
    fn canonicalize(&self, raw_srepr: &str) -> Result<CanonicalizedExpr, String> {
        let (_, res) = self.rpc("canonicalize", json!({ "srepr": raw_srepr }))?;
        let free_symbols = res["free_symbols"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| {
                        Some(SymbolSpec {
                            name: v.get("name")?.as_str()?.to_string(),
                            assumptions: v
                                .get("assumptions")
                                .and_then(Value::as_array)
                                .map(|a| {
                                    a.iter()
                                        .filter_map(Value::as_str)
                                        .map(String::from)
                                        .collect()
                                })
                                .unwrap_or_default(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(CanonicalizedExpr {
            portable: res["portable"].as_bool().unwrap_or(false),
            srepr: res["srepr"].as_str().unwrap_or_default().to_string(),
            srepr_sha256: res["srepr_sha256"].as_str().unwrap_or_default().to_string(),
            latex: res["latex"].as_str().unwrap_or_default().to_string(),
            str_preview: res["str_preview"].as_str().unwrap_or_default().to_string(),
            free_symbols,
            error: res["error"].as_str().map(String::from),
            rejected_constructor: res["rejected_constructor"].as_str().map(String::from),
        })
    }
}

pub struct TestFactory {
    pub use_real_python: bool,
    pub spawns: Arc<AtomicUsize>,
    pub dropped: Arc<AtomicBool>,
}

impl IsolatedWorkerFactory for TestFactory {
    fn spawn_isolated_python_worker(
        &self,
        _session: &str,
        _purpose: &str,
    ) -> Result<Box<dyn IsolatedPythonWorker>, String> {
        self.spawns.fetch_add(1, Ordering::SeqCst);
        if self.use_real_python {
            Ok(Box::new(RealStdioPythonWorker::spawn()?))
        } else {
            Ok(Box::new(NoopWorker {
                dropped: Arc::clone(&self.dropped),
            }))
        }
    }
    fn check_runner(&self, _session: &str) -> Result<CheckRunnerBinding, String> {
        if self.use_real_python {
            let w = Arc::new(RealStdioPythonWorker::spawn_role("check")?);
            Ok((w, Arc::new(StubCheckEpoch), true))
        } else {
            Ok((Arc::new(StubCheckRunner), Arc::new(StubCheckEpoch), true))
        }
    }
    fn check_canonicalizer(&self, _session: &str) -> Result<Arc<dyn CheckCanonicalizer>, String> {
        if self.use_real_python {
            Ok(Arc::new(RealStdioPythonWorker::spawn_role("check")?))
        } else {
            Ok(Arc::new(StubCanonicalizer))
        }
    }
    fn isolated_workers_contained(&self) -> bool {
        true
    }
}

pub struct NoopWorker {
    pub dropped: Arc<AtomicBool>,
}
impl Drop for NoopWorker {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}
impl IsolatedPythonWorker for NoopWorker {
    fn evaluate(
        &self,
        _code: &str,
        _exprs_in: Option<&BTreeMap<String, String>>,
        _timeout: Duration,
        _required_epoch: Option<u64>,
    ) -> Result<IsolatedEvalOutput, String> {
        Ok(IsolatedEvalOutput {
            result: json!({ "value": "0", "stdout": "", "stderr": "", "error": null }),
            epoch: 1,
            request_id: 1,
            environment: None,
        })
    }
    fn live_epoch(&self) -> Option<u64> {
        Some(1)
    }
    fn is_contained(&self) -> bool {
        true
    }
    fn containment_summary(&self) -> String {
        "noop".into()
    }
}

pub fn seed_solve_session(
    ledger: &Ledger,
    session: &str,
    solver: RoleModel,
    verifier: RoleModel,
    review: ReviewPolicy,
) -> (String, String) {
    ledger
        .append_with_hash(session, event_kinds::SOLVE_STARTED, 1, &json!({}))
        .unwrap();
    let math_sha = compute_math_sha256(&[], &BTreeMap::new(), &[], "Euclidean");
    let setup = SolveSetup {
        revision: 1,
        statement: "Prove x^2 >= 0 for real x".into(),
        task_kind: TaskKind::Prove,
        target: SolveTarget {
            description: "proof".into(),
            form: TargetForm::Proof,
            order: None,
            digits: None,
        },
        done: vec![DoneCriterion {
            id: "c1".into(),
            kind: DoneCriterionKind::ClosureTierAtLeast,
            params: json!({ "tier": "model_agreed" }),
        }],
        conventions: "Euclidean".into(),
        symbols: vec![],
        dimensions: BTreeMap::new(),
        assumptions: vec![],
        given_source: None,
        sources: vec![],
        roles: SolveRoles {
            solver,
            verifier,
            compute: None,
        },
        budgets: SolveBudgets::default(),
        review,
        tool_profile: event_kinds::SOLVE_TOOL_PROFILE_V1.into(),
        origin: None,
        math_sha256: math_sha.clone(),
    };
    ledger
        .append_with_hash(
            session,
            event_kinds::SOLVE_SETUP,
            1,
            &serde_json::to_value(&setup).unwrap(),
        )
        .unwrap();

    let srepr_lhs = "Pow(Add(Symbol('x'), Integer(1)), Integer(2))";
    let srepr_rhs = "Add(Pow(Symbol('x'), Integer(2)), Mul(Integer(2), Symbol('x')), Integer(1))";
    let sha_lhs = ledger.put_blob(srepr_lhs.as_bytes()).unwrap();
    let sha_rhs = ledger.put_blob(srepr_rhs.as_bytes()).unwrap();
    for (name, sha, latex, preview) in [
        ("lhs_h", &sha_lhs, "(x + 1)^2", "(x + 1)**2"),
        ("rhs_h", &sha_rhs, "x^2 + 2x + 1", "x**2 + 2*x + 1"),
    ] {
        let expr = SolveExpr {
            name: name.into(),
            version: 1,
            srepr_blob: sha.clone(),
            srepr_sha256: sha.clone(),
            latex: latex.into(),
            str_preview: preview.into(),
            free_symbols: vec![],
            produced_by: ReceiptRef {
                call_id: "c-0".into(),
                result_seq: 1,
            },
            owner_entry: Some("e-1".into()),
            portable: true,
            unevaluated_srepr: None, unevaluated_srepr_sha256: None, source_expr: None,
        };
        ledger
            .append_with_hash(
                session,
                event_kinds::SOLVE_EXPR,
                1,
                &serde_json::to_value(&expr).unwrap(),
            )
            .unwrap();
    }
    let claim = Claim {
        form: ClaimForm::Equation,
        lhs: HandleRef::parse("lhs_h@1").unwrap(),
        rhs: HandleRef::parse("rhs_h@1").unwrap(),
        relation: ClaimRelation::Eq,
        order: None,
        domain: None,
        how: None,
    };
    let claim_sha = compute_claim_sha256(std::slice::from_ref(&claim), |h| {
        Some(if h.name == "lhs_h" { sha_lhs.as_str() } else { sha_rhs.as_str() })
    })
    .unwrap();
    let e1_sha = compute_entry_sha256(Some(&claim_sha), "Identity step", "Details for e-1");
    ledger
        .append_with_hash(
            session,
            event_kinds::PROOF_ENTRY,
            1,
            &json!({
                "entry_id": "e-1",
                "entry_type": StepEntryType::Derivation.as_str(),
                "summary": "Identity step",
                "details": "Details for e-1",
                "depends_on": [],
                "claims": [claim],
                "claim_sha256": claim_sha,
                "entry_sha256": e1_sha,
                "origin": StepOrigin::Solver,
            }),
        )
        .unwrap();
    let e2_sha = compute_entry_sha256(None, "Prose conclusion", "Prose details for e-2");
    ledger
        .append_with_hash(
            session,
            event_kinds::PROOF_ENTRY,
            1,
            &json!({
                "entry_id": "e-2",
                "entry_type": StepEntryType::FinalAnswer.as_str(),
                "summary": "Prose conclusion",
                "details": "Prose details for e-2",
                "depends_on": ["e-1"],
                "entry_sha256": e2_sha,
                "origin": StepOrigin::Solver,
            }),
        )
        .unwrap();
    (math_sha, claim_sha)
}
