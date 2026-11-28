//! Cross-process Solve S10 integration tests: real Python `worker.py --stdio --role solver`
//! & `--role check`, host `check_run` + `AutoCheckGate`, R3 parameter guards, R8 sublabel &
//! fail-latch semantics, assumption mismatch `inconclusive`, and R19 reproducibility.

use ledger::Ledger;
use research_tools::policy_preset::SOLVE_CHECK_CALLER;
use research_tools::solve_status::Tier;
use research_tools::solve_checks::{
    evaluate_recheck_diff, CheckExecError, CheckExecResult, CheckProcessRunner,
};
use research_tools::solve_defs::{CHECK_RUN, SOLVE_FINISH, STEP_RECORD};
use research_tools::solve_exprs::{
    attach_harvest_previews, harvest_from_sympy_result, stage_exprs_in, CanonicalizedExpr,
    CheckCanonicalizer,
};
use research_tools::solve_gates::{solve_check_host_v1, solve_gate_chain};
use research_tools::solve_model::{
    compute_math_sha256, event_kinds, ClaimForm, ClaimRelation, DoneCriterion, DoneCriterionKind,
    HandleRef, ReceiptRef, ReviewPolicy, RoleModel, SetupAssumption, SolveBudgets, SolveCheck,
    SolveRoles, SolveSetup, SolveTarget, StepEntryType, SymbolSpec, TargetForm, TaskKind,
};
use research_tools::solve_state::SolveState;
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

fn require_sympy_python() -> (String, Vec<&'static str>) {
    for var in ["RE_SEARCH_PYTHON", "RESEARCH_CORE_PYTHON"] {
        if let Ok(p) = std::env::var(var) {
            let trimmed = p.trim();
            if !trimmed.is_empty() {
                let ok = Command::new(trimmed)
                    .args(["-c", "import sympy, mpmath, numpy"])
                    .env("PYTHONNOUSERSITE", "1")
                    .output()
                    .is_ok_and(|o| o.status.success());
                assert!(ok, "{var}=`{trimmed}` cannot import sympy/mpmath/numpy");
                return (trimmed.to_string(), vec![]);
            }
        }
    }
    let candidates: &[(&str, &[&'static str])] = if cfg!(windows) {
        &[("python", &[]), ("py", &["-3"])]
    } else {
        &[("python3", &[]), ("python", &[])]
    };
    for &(prog, prefix) in candidates {
        if Command::new(prog)
            .args(prefix)
            .args(["-c", "import sympy, mpmath, numpy"])
            .env("PYTHONNOUSERSITE", "1")
            .output()
            .is_ok_and(|o| o.status.success())
        {
            return (prog.to_string(), prefix.to_vec());
        }
    }
    panic!("Python with sympy, mpmath, numpy required for solve_s10_physics tests");
}

struct StdioWorker {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl StdioWorker {
    fn spawn(py: &(String, Vec<&'static str>), role: &str, cwd: &Path) -> Self {
        let worker_py = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("python/worker.py");
        let mut child = Command::new(&py.0)
            .args(&py.1)
            .arg("-u")
            .arg(worker_py)
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

struct RealCheckBridge(Arc<Mutex<StdioWorker>>);

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
        let mut w = self.0.lock().map_err(|_| CheckExecError::Worker("poisoned".into()))?;
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

struct LiveEpoch;
impl WorkerEpoch for LiveEpoch {
    fn live_epoch(&self) -> Option<u64> { Some(1) }
    fn is_contained(&self) -> bool { true }
}

struct ApproveAll;
impl Approver for ApproveAll {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        ApprovalReply { approved: true, digest: req.digest.clone(), by: "s10-test".into() }
    }
}

struct PhysicsRig {
    _tmp: TempDir,
    data_dir: PathBuf,
    sid: String,
    checker: Arc<RealCheckBridge>,
    router: Router,
}

fn build_physics_rig(
    sid: &str,
    statement: &str,
    symbols: Vec<SymbolSpec>,
    dimensions: BTreeMap<String, BTreeMap<String, Value>>,
    assumptions: Vec<SetupAssumption>,
    done: Vec<DoneCriterion>,
) -> PhysicsRig {
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
    let math_sha = compute_math_sha256(&symbols, &dimensions, &assumptions, "SI physics");
    let setup = SolveSetup {
        revision: 1,
        statement: statement.into(),
        task_kind: TaskKind::Calculate,
        target: SolveTarget { description: "physics target".into(), form: TargetForm::ClosedForm, order: None, digits: None },
        done,
        conventions: "SI physics".into(),
        symbols,
        dimensions,
        assumptions,
        given_source: None,
        sources: vec![],
        roles: SolveRoles {
            solver: RoleModel { provider: "anthropic".into(), model: "claude-sonnet-4-6".into() },
            verifier: RoleModel { provider: "openai".into(), model: "gpt-5".into() },
            compute: None,
        },
        budgets: SolveBudgets::default(),
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

    let epoch: Arc<dyn WorkerEpoch> = Arc::new(LiveEpoch);
    let runner_dyn: Arc<dyn CheckProcessRunner> = checker.clone();
    let host = ProfileHostTools::new(Arc::clone(&epoch), sympy_handler)
        .with_solve_check(solve_check_host_v1(epoch, Arc::clone(&env), Arc::clone(&runner_dyn)));
    let mut reg = Registry::new();
    let policy = register_profile(&solve_v1(), &mut reg, Arc::clone(&env), host).unwrap();
    let (_gate, chain) = solve_gate_chain(env, runner_dyn, seq0, seq0);
    let router = Router::new(reg, policy, ledger, sid, Arc::new(ApproveAll))
        .with_tool_profile(PROFILE_SOLVE_V1)
        .with_gate(chain);

    PhysicsRig { _tmp: tmp, data_dir, sid: sid.into(), checker, router }
}

fn eval_harvest(rig: &PhysicsRig, code: &str) -> String {
    let out = rig.router.try_call("solver:solve", "sympy_eval", &json!({ "code": code })).unwrap();
    let mut res = match out {
        CallOutcome::Ok(v) => v,
        other => panic!("sympy_eval unexpected: {other:?}"),
    };
    let call_id = res["tool_call_ref"].as_str().unwrap().to_string();
    let ledger = Ledger::open(&rig.data_dir).unwrap();
    let evs = ledger.read_session(&rig.sid).unwrap();
    let result_seq = evs.iter().rev()
        .find(|e| e.kind == event_kinds::TOOL_RESULT && e.payload["call_id"] == call_id)
        .unwrap().seq;
    let report = harvest_from_sympy_result(
        &ledger, &rig.sid, ReceiptRef { call_id: call_id.clone(), result_seq }, &res, rig.checker.as_ref(),
    ).unwrap();
    attach_harvest_previews(&mut res, &report);
    assert!(res["error"].is_null(), "sympy code error: {res}");
    call_id
}

fn record_claim(
    rig: &PhysicsRig,
    form: ClaimForm,
    relation: ClaimRelation,
    summary: &str,
    deps: &[&str],
    call_id: &str,
    sides: (&str, &str),
) -> Value {
    let out = rig.router.try_call(
        "solver:solve",
        STEP_RECORD,
        &json!({
            "entry_type": StepEntryType::SymbolicComputation,
            "summary": summary,
            "details": summary,
            "depends_on": deps,
            "tool_call_refs": [call_id],
            "claims": [{
                "form": form,
                "lhs": HandleRef::parse(sides.0).unwrap(),
                "rhs": HandleRef::parse(sides.1).unwrap(),
                "relation": relation,
            }]
        }),
    ).unwrap();
    match out {
        CallOutcome::Ok(v) => v,
        other => panic!("step_record failed: {other:?}"),
    }
}

fn run_check(rig: &PhysicsRig, entry_id: &str, check: &str, params: Value) -> Value {
    let out = rig.router.try_call(
        SOLVE_CHECK_CALLER,
        CHECK_RUN,
        &json!({ "entry_id": entry_id, "check": check, "params": params }),
    ).unwrap();
    match out {
        CallOutcome::Ok(v) => v,
        other => panic!("check_run({check}) failed: {other:?}"),
    }
}

fn fold_state(rig: &PhysicsRig) -> SolveState {
    SolveState::from_events(&Ledger::open(&rig.data_dir).unwrap().read_session(&rig.sid).unwrap())
}

fn dim_map(pairs: &[(&str, &[(&str, i64)])]) -> BTreeMap<String, BTreeMap<String, Value>> {
    pairs.iter().map(|(sym, dims)| {
        let m = dims.iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
        (sym.to_string(), m)
    }).collect()
}

#[test]
fn s10_dimensions_autocheck_r8_and_finish_criterion() {
    let dims = dim_map(&[
        ("m", &[("mass", 1)]),
        ("v", &[("length", 1), ("time", -1)]),
        ("K", &[("mass", 1), ("length", 2), ("time", -2)]),
    ]);
    let rig = build_physics_rig(
        "s10-dim",
        "Verify kinetic energy K = m*v^2/2 dimensionality and identity",
        vec![
            SymbolSpec { name: "m".into(), assumptions: vec!["real".into(), "positive".into()] },
            SymbolSpec { name: "v".into(), assumptions: vec!["real".into()] },
            SymbolSpec { name: "K".into(), assumptions: vec!["real".into()] },
        ],
        dims,
        vec![],
        vec![
            DoneCriterion { id: "ver".into(), kind: DoneCriterionKind::FinalTierAtLeast, params: json!({ "tier": "verified" }) },
            DoneCriterion { id: "dim".into(), kind: DoneCriterionKind::NamedCheckPassed, params: json!({ "check": "dimensions", "on": "final" }) },
        ],
    );

    let c1 = eval_harvest(&rig, "m = assumed_symbol('m', real=True, positive=True)\nv = assumed_symbol('v', real=True)\nsave_expr('L1', 'Rational(1, 2)*m*v**2 + Rational(1, 2)*m*v**2', locals={'m': m, 'v': v})\nsave_expr('R1', m*v**2)\nInteger(0)");
    let s1 = record_claim(&rig, ClaimForm::Equation, ClaimRelation::Eq, "Kinetic energy term sum", &[], &c1, ("L1@1", "R1@1"));
    assert_eq!(s1["effective_tier"], "verified");
    let autos = s1["auto_checks"].as_array().unwrap();
    assert_eq!(autos.len(), 2);
    assert_eq!(autos[0]["check"], "identity_symbolic");
    assert_eq!(autos[0]["outcome"], "pass");
    assert_eq!(autos[1]["check"], "dimensions");
    assert_eq!(autos[1]["outcome"], "pass");
    assert!(s1["badge"]["sublabels"].as_array().unwrap().iter().any(|s| s == "dimensions"));

    let fin = rig.router.try_call(
        "solver:solve", SOLVE_FINISH,
        &json!({ "outcome": "finished", "final_entry_id": "e-1", "summary_markdown": "Kinetic energy check complete." }),
    ).unwrap();
    let fin_v = match fin { CallOutcome::Ok(v) => v, other => panic!("{other:?}") };
    assert_eq!(fin_v["status"], "finished");

    // Inconsistent dimensions on an unchecked step must fail `dimensions` and flip step to Failed (R8).
    let c2 = eval_harvest(&rig, "m = assumed_symbol('m', real=True, positive=True)\nv = assumed_symbol('v', real=True)\nK = assumed_symbol('K', real=True)\nsave_expr('BadL', m*v)\nsave_expr('BadR', K)\nInteger(0)");
    let s2 = record_claim(&rig, ClaimForm::Inequality, ClaimRelation::Le, "Wrong dimensions m*v vs K", &[], &c2, ("BadL@1", "BadR@1"));
    assert_eq!(s2["effective_tier"], "failed");
    assert!(s2["auto_checks"].as_array().unwrap().iter().any(|c| c["check"] == "dimensions" && c["outcome"] == "fail"));
}

#[test]
fn s10_all_six_checks_r8_sublabel_only_failure_and_r19() {
    let dims = dim_map(&[
        ("x", &[]),
        ("q", &[("length", 1)]),
        ("v", &[("length", 1), ("time", -1)]),
        ("m", &[("mass", 1)]),
    ]);
    let rig = build_physics_rig(
        "s10-all6",
        "Run all 6 S10 physics checks, verify R8 and R19",
        vec![
            SymbolSpec { name: "x".into(), assumptions: vec!["real".into()] },
            SymbolSpec { name: "q".into(), assumptions: vec!["real".into()] },
            SymbolSpec { name: "v".into(), assumptions: vec!["real".into()] },
            SymbolSpec { name: "m".into(), assumptions: vec!["real".into(), "positive".into()] },
            SymbolSpec { name: "t".into(), assumptions: vec!["real".into()] },
            SymbolSpec { name: "n".into(), assumptions: vec!["integer".into(), "positive".into()] },
        ],
        dims,
        vec![],
        vec![],
    );

    // Step e-1: Unchecked Inequality (`0 <= q**2`): passing all S10 checks adds sublabels only, never upgrades tier.
    let c1 = eval_harvest(&rig, concat!(
        "x = assumed_symbol('x', real=True)\n",
        "q = assumed_symbol('q', real=True)\n",
        "v = assumed_symbol('v', real=True)\n",
        "m = assumed_symbol('m', real=True, positive=True)\n",
        "t = assumed_symbol('t', real=True)\n",
        "n = assumed_symbol('n', integer=True, positive=True)\n",
        "save_expr('Z', Integer(0))\n",
        "save_expr('Sq', q**2)\n",
        "save_expr('Sinc', sin(x)/x)\n",
        "save_expr('Lim1', Integer(1))\n",
        "save_expr('KE', Rational(1, 2)*m*v**2)\n",
        "save_expr('Zeta', Sum(1/n**2, (n, 1, oo)))\n",
        "Integer(0)"
    ));
    let _s1 = record_claim(&rig, ClaimForm::Inequality, ClaimRelation::Le, "Non-negative q^2", &[], &c1, ("Z@1", "Sq@1"));

    // 1. dimensions passed automatically on e-1 (zero matches any dim); tier stays Unchecked.
    assert_eq!(fold_state(&rig).tier("e-1").unwrap().own, Tier::Unchecked);

    // Record helper Unchecked steps for limit_case, symmetry, conservation, convergence, matrix_property.
    let s_lim = record_claim(&rig, ClaimForm::Inequality, ClaimRelation::Le, "Sinc bound", &[], &c1, ("Sinc@1", "Lim1@1"));
    let lim_eid = s_lim["entry_id"].as_str().unwrap();
    assert_eq!(run_check(&rig, lim_eid, "limit_case", json!({ "variable": "x", "point": "0", "target_handle": "Lim1@1" }))["outcome"], "pass");
    assert_eq!(run_check(&rig, lim_eid, "symmetry", json!({ "kind": "even", "variable": "x", "side": "lhs" }))["outcome"], "pass");

    let s_cons = record_claim(&rig, ClaimForm::Inequality, ClaimRelation::Le, "Time-independent KE", &[], &c1, ("Z@1", "KE@1"));
    let cons_eid = s_cons["entry_id"].as_str().unwrap();
    assert_eq!(run_check(&rig, cons_eid, "conservation", json!({ "mode": "derivative", "variable": "t", "side": "rhs" }))["outcome"], "pass");

    let s_conv = record_claim(&rig, ClaimForm::Inequality, ClaimRelation::Le, "p-series positivity", &[], &c1, ("Z@1", "Zeta@1"));
    let conv_eid = s_conv["entry_id"].as_str().unwrap();
    assert_eq!(run_check(&rig, conv_eid, "convergence", json!({ "side": "rhs" }))["outcome"], "pass");

    let c_mat = eval_harvest(&rig, "save_expr('MatA', Matrix([[2, I], [-I, 2]]))\nsave_expr('MatZ', Matrix([[0, 0], [0, 0]]))\nInteger(0)");
    let s_mat = record_claim(&rig, ClaimForm::Inequality, ClaimRelation::Le, "Positive Hermitian matrix", &[], &c_mat, ("MatA@1", "MatZ@1"));
    let mat_eid = s_mat["entry_id"].as_str().unwrap();
    let m1 = run_check(&rig, mat_eid, "matrix_property", json!({ "properties": ["hermitian", "positive_definite"], "side": "lhs" }));
    let m2 = run_check(&rig, mat_eid, "matrix_property", json!({ "properties": ["hermitian", "positive_definite"], "side": "lhs" }));
    assert_eq!(m1["outcome"], "pass");

    // R19 reproducibility: identical inputs yield matching evidence_sha256.
    let orig_chk: SolveCheck = serde_json::from_value(m1.clone()).unwrap();
    assert_eq!(evaluate_recheck_diff(&orig_chk, &m2)["matched"], true);

    // R8: even with passing S10 checks, every Inequality step remains Unchecked and shows S10 sublabels.
    let st = fold_state(&rig);
    for (eid, label) in [("e-1", "dimensions"), (lim_eid, "limit_case"), (lim_eid, "symmetry"), (cons_eid, "conservation"), (conv_eid, "convergence"), (mat_eid, "matrix_property")] {
        let t = st.tier(eid).unwrap();
        assert_eq!(t.own, Tier::Unchecked, "{eid} must stay Unchecked after S10 pass (R8)");
        assert!(t.sublabels.iter().any(|s| s == label), "{eid} missing sublabel {label}: {:?}", t.sublabels);
    }

    // Failing S10 check on a symbolically Verified step flips it to Failed and breaks descendants (R8 + G7).
    let c_ver = eval_harvest(&rig, "x = assumed_symbol('x', real=True)\nsave_expr('V1', (x + 1)**2)\nsave_expr('V2', x**2 + 2*x + 1)\nsave_expr('V3', (x + 2)**2)\nsave_expr('V4', x**2 + 4*x + 4)\nInteger(0)");
    let s_ver = record_claim(&rig, ClaimForm::Equation, ClaimRelation::Eq, "Verified polynomial", &[], &c_ver, ("V1@1", "V2@1"));
    assert_eq!(s_ver["effective_tier"], "verified");
    let v_id = s_ver["entry_id"].as_str().unwrap();
    let s_dep = record_claim(&rig, ClaimForm::Equation, ClaimRelation::Eq, "Dependent verified step", &[v_id], &c_ver, ("V3@1", "V4@1"));
    let d_id = s_dep["entry_id"].as_str().unwrap();

    let sym_fail = run_check(&rig, v_id, "symmetry", json!({ "kind": "even", "variable": "x" }));
    assert_eq!(sym_fail["outcome"], "fail");
    let st_after = fold_state(&rig);
    assert_eq!(st_after.tier(v_id).unwrap().own, Tier::Failed);
    assert_eq!(st_after.tier(d_id).unwrap().effective, Tier::Failed);
    let g7 = rig.router.try_call("solver:solve", SOLVE_FINISH, &json!({ "final_entry_id": d_id, "summary_markdown": "Check finish" })).unwrap();
    assert!(matches!(g7, CallOutcome::Error(ref m) if m.starts_with("GATE G7:")));
}

#[test]
fn s10_assumption_mismatch_and_r3_param_refusals() {
    let rig = build_physics_rig(
        "s10-guard",
        "Verify assumption mismatch -> inconclusive and R3 parameter guards",
        vec![SymbolSpec { name: "r".into(), assumptions: vec!["real".into(), "positive".into()] }],
        dim_map(&[("r", &[("length", 1)])]),
        vec![],
        vec![],
    );

    // Handle constructed with `negative=True` contradicts setup.symbols (r: real, positive).
    let c1 = eval_harvest(&rig, "r = Symbol('r', negative=True)\nsave_expr('L', r)\nsave_expr('R', r)\nInteger(0)");
    let _s1 = record_claim(&rig, ClaimForm::Inequality, ClaimRelation::Le, "Assumption mismatch step", &[], &c1, ("L@1", "R@1"));

    // Two handles whose own free symbols carry differing assumptions also trigger inconclusive.
    let c2 = eval_harvest(&rig, "rp = Symbol('r', real=True, positive=True)\nrw = Symbol('r', real=True)\nsave_expr('Lp', rp)\nsave_expr('Rw', rw)\nInteger(0)");
    let _s2 = record_claim(&rig, ClaimForm::Inequality, ClaimRelation::Le, "Handle assumption disagreement", &[], &c2, ("Lp@1", "Rw@1"));

    for eid in ["e-1", "e-2"] {
        for (chk, p) in [
            ("dimensions", json!({})),
            ("limit_case", json!({ "variable": "r", "point": "oo" })),
            ("symmetry", json!({ "kind": "even", "variable": "r" })),
            ("conservation", json!({ "mode": "derivative", "variable": "r" })),
            ("convergence", json!({ "variable": "r", "levels": [2, 4, 8] })),
            ("matrix_property", json!({ "property": "hermitian" })),
        ] {
            let res = run_check(&rig, eid, chk, p);
            assert_eq!(res["outcome"], "inconclusive", "{chk} on {eid} must return inconclusive: {res}");
            assert!(res["evidence"]["reason"].as_str().unwrap_or_default().contains("assumptions"), "{res}");
        }
    }

    // R3 host guard refuses forged or host-reserved params before calling worker.
    for bad_params in [
        json!({ "dimensions": { "r": { "length": 1 } } }),
        json!({ "assumptions": ["r > 0"] }),
        json!({ "expected_value": 0 }),
        json!({ "code": "1+1" }),
    ] {
        let out = rig.router.try_call(SOLVE_CHECK_CALLER, CHECK_RUN, &json!({ "entry_id": "e-1", "check": "dimensions", "params": bad_params })).unwrap();
        assert!(matches!(out, CallOutcome::Error(_)), "expected R3 error for {bad_params}: {out:?}");
    }
}
