//! Shared real cross-process Python `worker.py --stdio` test rig for `solve_s8_s9_s11` and `solve_set_q_m3`.
#![allow(dead_code)]

use ledger::Ledger;
use research_tools::solve_checks::{CheckExecError, CheckExecResult, CheckProcessRunner};
use research_tools::solve_exprs::{
    attach_harvest_previews, harvest_from_sympy_result, stage_exprs_in, CanonicalizedExpr,
    CheckCanonicalizer,
};
use research_tools::solve_gates::{solve_check_host_v1, solve_gate_chain, SolveGate};
use research_tools::solve_model::{
    compute_math_sha256, event_kinds, ClaimForm, ClaimRelation, DoneCriterion, HandleRef,
    ReceiptRef, ReviewPolicy, RoleModel, SetupAssumption, SolveBudgets, SolveRoles, SolveSetup,
    SolveTarget, StepEntryType, SymbolSpec, TargetForm, TaskKind,
};
use research_tools::tool_profile::{register_profile, solve_v1, ProfileHostTools, PROFILE_SOLVE_V1};
use research_tools::ResearchEnv;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;
use tools::{
    ApprovalReply, ApprovalRequest, Approver, CallCtx, CallOutcome, Registry, Router, ToolHandler,
    WorkerEpoch,
};

pub fn require_sympy_python() -> (String, Vec<&'static str>) {
    for var in ["RE_SEARCH_PYTHON", "RESEARCH_CORE_PYTHON"] {
        if let Ok(p) = std::env::var(var) {
            let trimmed = p.trim();
            if !trimmed.is_empty() {
                let ok = Command::new(trimmed)
                    .args(["-c", "import sympy, mpmath"])
                    .env("PYTHONNOUSERSITE", "1")
                    .output()
                    .is_ok_and(|o| o.status.success());
                assert!(ok, "{var}=`{trimmed}` cannot import sympy/mpmath");
                return (trimmed.to_string(), vec![]);
            }
        }
    }
    let candidates: &[(&str, &[&'static str])] =
        if cfg!(windows) { &[("python", &[]), ("py", &["-3"])] } else { &[("python3", &[]), ("python", &[])] };
    for &(prog, prefix) in candidates {
        if Command::new(prog)
            .args(prefix)
            .args(["-c", "import sympy, mpmath"])
            .env("PYTHONNOUSERSITE", "1")
            .output()
            .is_ok_and(|o| o.status.success())
        {
            return (prog.to_string(), prefix.to_vec());
        }
    }
    panic!("Python with sympy and mpmath required for solve_s8_s9_s11 integration tests");
}

fn worker_script_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join("python/worker.py")
}

struct StdioWorker {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl StdioWorker {
    fn spawn(py: &(String, Vec<&'static str>), role: &str, cwd: &Path) -> Self {
        let mut child = Command::new(&py.0)
            .args(&py.1)
            .arg("-u")
            .arg(worker_script_path())
            .arg("--stdio")
            .arg("--role")
            .arg(role)
            .current_dir(cwd)
            .env("PYTHONNOUSERSITE", "1")
            .env("PYTHONIOENCODING", "utf-8")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("failed to spawn worker ({role}): {e}"));
        let stdin = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut hello = String::new();
        stdout.read_line(&mut hello).unwrap();
        let h: Value = serde_json::from_str(&hello).unwrap();
        assert_eq!(h["ready"], true, "worker hello failed: {h}");
        assert_eq!(h["role"], role);
        Self { child, stdin, stdout, next_id: 1 }
    }

    fn request(&mut self, method: &str, params: Value) -> (u64, Value) {
        let id = self.next_id;
        self.next_id += 1;
        let line = format!("{}\n", json!({ "id": id, "method": method, "params": params }));
        self.stdin.write_all(line.as_bytes()).unwrap();
        self.stdin.flush().unwrap();
        let mut resp_line = String::new();
        self.stdout.read_line(&mut resp_line).unwrap();
        let resp: Value = serde_json::from_str(&resp_line).unwrap();
        assert_eq!(resp["id"], id);
        (id, resp)
    }
}

impl Drop for StdioWorker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub struct RealCheckBridge(Arc<Mutex<StdioWorker>>);

impl CheckCanonicalizer for RealCheckBridge {
    fn canonicalize(&self, raw_srepr: &str) -> Result<CanonicalizedExpr, String> {
        let mut w = self.0.lock().map_err(|_| "check worker poisoned".to_string())?;
        let (_, resp) = w.request("canonicalize", json!({ "srepr": raw_srepr }));
        if resp["ok"] != true {
            return Err(format!("canonicalize failed: {}", resp["error"]));
        }
        serde_json::from_value(resp["result"].clone()).map_err(|e| e.to_string())
    }
}

impl CheckProcessRunner for RealCheckBridge {
    fn run_check(
        &self,
        name: &str,
        handles: &BTreeMap<String, String>,
        params: &Value,
        _timeout: Duration,
        _required_epoch: Option<u64>,
    ) -> Result<CheckExecResult, CheckExecError> {
        let mut w = self.0.lock().map_err(|_| CheckExecError::Worker("check worker poisoned".into()))?;
        let (req_id, resp) = w.request(
            "check",
            json!({ "name": name, "handles": handles, "params": params }),
        );
        if resp["ok"] == true {
            Ok(CheckExecResult {
                raw: resp["result"].clone(),
                epoch: 1,
                request_id: req_id,
                environment: Some(json!({ "role": "check", "protocol": 2 })),
            })
        } else {
            Err(CheckExecError::Worker(resp["error"].to_string()))
        }
    }
}

struct LiveContainedEpoch;
impl WorkerEpoch for LiveContainedEpoch {
    fn live_epoch(&self) -> Option<u64> { Some(1) }
    fn is_contained(&self) -> bool { true }
}

struct ApproveAll;
impl Approver for ApproveAll {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        ApprovalReply { approved: true, digest: req.digest.clone(), by: "e2e-test".into() }
    }
}

pub struct TestRig {
    pub _tmp: TempDir,
    pub data_dir: PathBuf,
    pub sid: String,
    pub checker: Arc<RealCheckBridge>,
    pub gate: Arc<SolveGate>,
    pub router: Router,
}

pub fn build_real_rig(
    sid: &str,
    task_kind: TaskKind,
    statement: &str,
    symbols: Vec<SymbolSpec>,
    assumptions: Vec<SetupAssumption>,
    done: Vec<DoneCriterion>,
    budgets: SolveBudgets,
) -> TestRig {
    let py = require_sympy_python();
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&ws).unwrap();

    let solver = Arc::new(Mutex::new(StdioWorker::spawn(&py, "solver", &ws)));
    let checker = Arc::new(RealCheckBridge(Arc::new(Mutex::new(StdioWorker::spawn(
        &py, "check", &ws,
    )))));

    let env = Arc::new(ResearchEnv::with_workspace(data_dir.clone(), &ws).unwrap());
    let ledger = Ledger::open(&data_dir).unwrap();
    let math_sha = compute_math_sha256(&symbols, &BTreeMap::new(), &assumptions, "standard conventions");
    let setup = SolveSetup {
        revision: 1,
        statement: statement.into(),
        task_kind,
        target: SolveTarget { description: "verified target".into(), form: TargetForm::ClosedForm, order: None, digits: None },
        done,
        conventions: "standard conventions".into(),
        symbols,
        dimensions: BTreeMap::new(),
        assumptions,
        given_source: None,
        sources: vec![],
        roles: SolveRoles {
            solver: RoleModel { provider: "anthropic".into(), model: "claude-sonnet-4-6".into() },
            verifier: RoleModel { provider: "openai".into(), model: "gpt-5".into() },
            compute: None,
        },
        budgets,
        review: ReviewPolicy::ProseOnly,
        tool_profile: event_kinds::SOLVE_TOOL_PROFILE_V1.into(),
        origin: None,
        math_sha256: math_sha,
    };
    ledger.append_with_hash(sid, event_kinds::SOLVE_STARTED, 1, &json!({})).unwrap();
    let seq0 = ledger
        .append_with_hash(sid, event_kinds::SOLVE_SETUP, 1, &serde_json::to_value(&setup).unwrap())
        .unwrap()
        .seq;

    let solver_clone = Arc::clone(&solver);
    let data_clone = data_dir.clone();
    let sid_clone = sid.to_string();
    let sympy_handler: Arc<dyn ToolHandler> = Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
        let code = args.get("code").and_then(Value::as_str).ok_or("missing `code`")?;
        let l = Ledger::open(&data_clone).map_err(|e| e.to_string())?;
        let staged = stage_exprs_in(&l, &sid_clone)?;
        let mut s = solver_clone.lock().map_err(|_| "solver poisoned".to_string())?;
        let (req_id, resp) = s.request("evaluate", json!({ "code": code, "exprs_in": staged }));
        if resp["ok"] != true {
            return Err(format!("solver error: {}", resp["error"]));
        }
        ctx.record_worker_execution(1, req_id);
        Ok(resp["result"].clone())
    });

    let epoch: Arc<dyn WorkerEpoch> = Arc::new(LiveContainedEpoch);
    let mut reg = Registry::new();
    let runner_dyn: Arc<dyn CheckProcessRunner> = checker.clone();
    let host = ProfileHostTools::new(Arc::clone(&epoch), sympy_handler)
        .with_solve_check(solve_check_host_v1(epoch, Arc::clone(&env), Arc::clone(&runner_dyn)));
    let policy = register_profile(&solve_v1(), &mut reg, Arc::clone(&env), host).unwrap();
    let (gate, chain) = solve_gate_chain(env, runner_dyn, seq0, seq0);
    let router = Router::new(reg, policy, ledger, sid, Arc::new(ApproveAll))
        .with_tool_profile(PROFILE_SOLVE_V1)
        .with_gate(chain);

    TestRig { _tmp: tmp, data_dir, sid: sid.into(), checker, gate, router }
}

pub fn eval_and_harvest(rig: &TestRig, code: &str) -> String {
    let out = rig
        .router
        .try_call("solver:solve", "sympy_eval", &json!({ "code": code }))
        .unwrap();
    let mut res = match out {
        CallOutcome::Ok(v) => v,
        other => panic!("sympy_eval unexpected outcome: {other:?}"),
    };
    let call_id = res["tool_call_ref"].as_str().unwrap().to_string();
    let ledger = Ledger::open(&rig.data_dir).unwrap();
    let evs = ledger.read_session(&rig.sid).unwrap();
    let result_seq = evs
        .iter()
        .rev()
        .find(|e| e.kind == event_kinds::TOOL_RESULT && e.payload["call_id"] == call_id)
        .unwrap()
        .seq;
    let report = harvest_from_sympy_result(
        &ledger,
        &rig.sid,
        ReceiptRef { call_id: call_id.clone(), result_seq },
        &res,
        rig.checker.as_ref(),
    )
    .unwrap();
    attach_harvest_previews(&mut res, &report);
    assert!(res["error"].is_null(), "sympy code error: {res}");
    call_id
}

pub fn record_equation_step(
    rig: &TestRig,
    entry_type: StepEntryType,
    summary: &str,
    deps: &[&str],
    call_id: &str,
    sides: (&str, &str),
    domain: Option<&str>,
) -> Value {
    let (lhs, rhs) = sides;
    let out = rig
        .router
        .try_call(
            "solver:solve",
            "step_record",
            &json!({
                "entry_type": entry_type,
                "summary": summary,
                "details": summary,
                "depends_on": deps,
                "tool_call_refs": [call_id],
                "claims": [{
                    "form": ClaimForm::Equation,
                    "lhs": HandleRef::parse(lhs).unwrap(),
                    "rhs": HandleRef::parse(rhs).unwrap(),
                    "relation": ClaimRelation::Eq,
                    "domain": domain,
                }]
            }),
        )
        .unwrap();
    match out {
        CallOutcome::Ok(v) => v,
        other => panic!("step_record unexpected outcome: {other:?}"),
    }
}
