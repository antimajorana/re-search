//! Integration and cross-process tests for Solve package S7:
//! - Frozen 16-tool `solve/1` profile & `solve_policy()` (`v0.5-solve`, rule R18)
//! - Cross-process R5: solver monkey-patching `sympy.latex` cannot alter `solve.expr.latex`
//! - Cross-process R6: non-allowlisted `Lambda` & hostile `srepr` mark `portable: false`
//! - Worker restart re-stages `exprs_in` from `Ledger` blobs without `.cas_bus.json`
//! - `expr_show` live tool execution + fail-closed S7 stubs for `step_record`, `check_run`, `solve_finish`

use ledger::Ledger;
use research_tools::policy_preset::{SOLVE_CHECK_CALLER, SOLVE_VERIFIER_CALLER};
use research_tools::solve_defs::{CHECK_RUN, EXPR_SHOW, SOLVE_FINISH, STEP_RECORD};
use research_tools::solve_exprs::{
    attach_harvest_previews, harvest_exprs_out, harvest_from_sympy_result, stage_exprs_in,
    CanonicalizedExpr, CheckCanonicalizer,
};
use research_tools::solve_model::{event_kinds, HandleRef, ReceiptRef};
use research_tools::solve_state::SolveState;
use research_tools::tool_profile::{
    register_profile, solve_v1, ProfileHostTools, SolveCheckHost, PROFILE_SOLVE_V1,
};
use research_tools::{ResearchEnv, SOLVE_POLICY_VERSION};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tools::{
    ApprovalReply, ApprovalRequest, Approver, CallCtx, CallOutcome, Registry, Router, ToolHandler,
    WorkerEpoch,
};

fn resolve_sympy_python() -> Option<(String, Vec<&'static str>)> {
    for var in ["RE_SEARCH_PYTHON", "RESEARCH_CORE_PYTHON"] {
        if let Ok(p) = std::env::var(var) {
            if !p.trim().is_empty() {
                return Some((p, Vec::new()));
            }
        }
    }
    let candidates: &[(&str, &[&str])] = if cfg!(windows) {
        &[("py", &["-3"])]
    } else {
        &[("python3", &[])]
    };
    for &(prog, prefix) in candidates {
        let ok = Command::new(prog)
            .args(prefix)
            .args(["-c", "import sympy"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if ok {
            return Some((prog.to_string(), prefix.to_vec()));
        }
    }
    None
}

fn worker_script_path() -> PathBuf {
    if let Ok(s) = std::env::var("RE_SEARCH_WORKER_SCRIPT") {
        if !s.trim().is_empty() {
            return PathBuf::from(s);
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("python")
        .join("worker.py")
}

/// Real separate OS child running `python/worker.py --stdio --role solver|check`.
struct StdioWorkerProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    pid: u32,
}

impl StdioWorkerProcess {
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
            .unwrap_or_else(|e| panic!("failed to spawn worker.py (--role {role}): {e}"));

        let pid = child.id();
        let stdin = child.stdin.take().expect("stdin piped");
        let mut stdout = BufReader::new(child.stdout.take().expect("stdout piped"));
        let mut hello_line = String::new();
        let n = stdout.read_line(&mut hello_line).expect("read hello");
        assert!(n > 0, "worker (--role {role}) exited before hello");
        let hello: Value = serde_json::from_str(&hello_line).expect("valid hello JSON");
        assert_eq!(hello["ready"], json!(true), "hello not ready: {hello}");
        assert_eq!(hello["protocol"], json!(2), "expected protocol 2: {hello}");
        assert_eq!(hello["role"], json!(role), "role mismatch: {hello}");

        Self { child, stdin, stdout, next_id: 1, pid }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let line = format!("{}\n", json!({ "id": id, "method": method, "params": params }));
        self.stdin.write_all(line.as_bytes()).expect("write request");
        self.stdin.flush().expect("flush request");

        let mut resp_line = String::new();
        let n = self.stdout.read_line(&mut resp_line).expect("read response");
        assert!(n > 0, "worker exited before answering request {id}");
        let resp: Value = serde_json::from_str(&resp_line).expect("valid response JSON");
        assert_eq!(resp["id"], json!(id), "response id mismatch: {resp}");
        resp
    }

    fn evaluate(&mut self, code: &str, exprs_in: Option<&BTreeMap<String, String>>) -> Value {
        let params = match exprs_in {
            Some(map) => json!({ "code": code, "exprs_in": map }),
            None => json!({ "code": code }),
        };
        let resp = self.request("evaluate", params);
        assert_eq!(resp["ok"], json!(true), "evaluate protocol error: {resp}");
        resp["result"].clone()
    }
}

impl Drop for StdioWorkerProcess {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "{{\"id\":999999,\"method\":\"shutdown\",\"params\":{{}}}}");
        let _ = self.stdin.flush();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct RealCheckProcessCanonicalizer(Mutex<StdioWorkerProcess>);

impl CheckCanonicalizer for RealCheckProcessCanonicalizer {
    fn canonicalize(&self, raw_srepr: &str) -> Result<CanonicalizedExpr, String> {
        let mut proc = self.0.lock().map_err(|_| "check process lock poisoned".to_string())?;
        let resp = proc.request("canonicalize", json!({ "srepr": raw_srepr }));
        if resp["ok"] != json!(true) {
            return Err(format!("check process error: {}", resp["error"]));
        }
        serde_json::from_value(resp["result"].clone())
            .map_err(|e| format!("invalid canonicalize payload: {e}"))
    }
}

struct FixedEpoch {
    epoch: AtomicU64,
    contained: bool,
}

impl FixedEpoch {
    fn new(ep: u64, contained: bool) -> Self {
        Self { epoch: AtomicU64::new(ep), contained }
    }
}

impl WorkerEpoch for FixedEpoch {
    fn live_epoch(&self) -> Option<u64> {
        match self.epoch.load(Ordering::Acquire) {
            0 => None,
            n => Some(n),
        }
    }

    fn is_contained(&self) -> bool {
        self.contained
    }
}

struct CountingApprover {
    approve: bool,
    prompts: AtomicUsize,
}

impl CountingApprover {
    fn new(approve: bool) -> Self {
        Self { approve, prompts: AtomicUsize::new(0) }
    }

    fn count(&self) -> usize {
        self.prompts.load(Ordering::SeqCst)
    }
}

impl Approver for CountingApprover {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        self.prompts.fetch_add(1, Ordering::SeqCst);
        ApprovalReply {
            approved: self.approve,
            digest: req.digest.clone(),
            by: "s7-test".into(),
        }
    }
}

fn build_solve_router(
    data_dir: &Path,
    session: &str,
    sympy_epoch: Arc<dyn WorkerEpoch>,
    sympy_handler: Arc<dyn ToolHandler>,
    check_epoch: Arc<dyn WorkerEpoch>,
    approver: Arc<dyn Approver>,
) -> Router {
    let profile = solve_v1();
    let env = Arc::new(ResearchEnv::new(data_dir));
    let mut reg = Registry::new();
    let policy = register_profile(
        &profile,
        &mut reg,
        env,
        ProfileHostTools::new(sympy_epoch, sympy_handler)
            .with_solve_check(SolveCheckHost::new(check_epoch)),
    )
    .unwrap();
    let ledger = Ledger::open(data_dir).unwrap();
    Router::new(reg, policy, ledger, session, approver).with_tool_profile(PROFILE_SOLVE_V1)
}

#[test]
fn cross_process_r5_solver_monkey_patch_latex_cannot_alter_solve_expr_latex() {
    let Some(py) = resolve_sympy_python() else {
        eprintln!("SKIPPED: set RE_SEARCH_PYTHON (Python with SymPy) to run cross-process S7 worker test");
        return;
    };
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let scratch = tmp.path().join("scratch");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&scratch).unwrap();

    let mut solver = StdioWorkerProcess::spawn(&py, "solver", &scratch);
    let checker = RealCheckProcessCanonicalizer(Mutex::new(StdioWorkerProcess::spawn(&py, "check", &scratch)));

    // Solver poisons `sympy.latex` in its own process and saves an expression.
    let code = r#"
sympy.latex = lambda *a, **kw: r"\text{POISONED_BY_SOLVER}"
x = assumed_symbol("x", positive=True)
expr_val = x**2 + 1
save_expr("E_quad", expr_val)
expr_val
"#;
    let mut eval_res = solver.evaluate(code, None);
    assert!(eval_res["error"].is_null(), "unexpected solver error: {eval_res}");
    // Confirm the poison actually corrupted the solver process's own LaTeX output.
    assert_eq!(eval_res["latex"], json!(r"\text{POISONED_BY_SOLVER}"));

    let ledger = Ledger::open(&data_dir).unwrap();
    let sid = "s-r5-cross-proc";
    ledger.append(sid, event_kinds::SOLVE_STARTED, 1, &json!({})).unwrap();

    let report = harvest_from_sympy_result(
        &ledger,
        sid,
        ReceiptRef { call_id: "call-r5".into(), result_seq: 2 },
        &eval_res,
        &checker,
    )
    .unwrap();
    assert_eq!(report.harvested.len(), 1);
    assert!(report.skipped.is_empty());

    let h = &report.harvested[0];
    assert_eq!(h.handle.to_string(), "E_quad@1");
    assert!(h.expr.portable);
    assert_eq!(h.expr.latex, "x^{2} + 1");
    assert!(!h.expr.latex.contains("POISONED"));
    assert_eq!(h.expr.free_symbols.len(), 1);
    assert_eq!(h.expr.free_symbols[0].name, "x");
    assert!(h.expr.free_symbols[0].assumptions.contains(&"positive".to_string()));

    attach_harvest_previews(&mut eval_res, &report);
    assert_eq!(eval_res["exprs"][0]["latex"], json!("x^{2} + 1"));

    // Verify `SolveState::from_events` folds it and `expr_show` through a real solve/1 Router works.
    let events = ledger.read_session(sid).unwrap();
    let state = SolveState::from_events(&events);
    let folded = state.handles.get(&HandleRef::parse("E_quad@1").unwrap()).unwrap();
    assert_eq!(folded.latex, "x^{2} + 1");

    let noop: Arc<dyn ToolHandler> = Arc::new(|_: &CallCtx<'_>, _: &Value| Ok(json!({})));
    let router = build_solve_router(
        &data_dir,
        sid,
        Arc::new(FixedEpoch::new(1, true)),
        noop,
        Arc::new(FixedEpoch::new(1, true)),
        Arc::new(CountingApprover::new(true)),
    );
    let out = router
        .try_call("solver:solve", EXPR_SHOW, &json!({ "name": "E_quad@1", "include_srepr": true }))
        .unwrap();
    let CallOutcome::Ok(shown) = out else {
        panic!("expected Ok from expr_show, got {out:?}");
    };
    assert_eq!(shown["handle"], "E_quad@1");
    assert_eq!(shown["latex"], "x^{2} + 1");
    assert_eq!(shown["portable"], true);
    assert_eq!(shown["srepr_sha256"], h.expr.srepr_sha256);
    assert!(shown["srepr"].as_str().unwrap().contains("Symbol('x'"));
}

#[test]
fn cross_process_r6_non_allowlisted_lambda_and_hostile_srepr_mark_non_portable() {
    let Some(py) = resolve_sympy_python() else {
        eprintln!("SKIPPED: set RE_SEARCH_PYTHON (Python with SymPy) to run cross-process S7 worker test");
        return;
    };
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let scratch = tmp.path().join("scratch");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&scratch).unwrap();

    let mut solver = StdioWorkerProcess::spawn(&py, "solver", &scratch);
    let checker = RealCheckProcessCanonicalizer(Mutex::new(StdioWorkerProcess::spawn(&py, "check", &scratch)));

    let code = r#"
x = Symbol("x", real=True)
save_expr("bad_lam", Lambda(x, x**2))
save_expr("good_poly", x**3 - 1)
"#;
    let eval_res = solver.evaluate(code, None);
    assert!(eval_res["error"].is_null(), "unexpected solver error: {eval_res}");

    let ledger = Ledger::open(&data_dir).unwrap();
    let sid = "s-r6-cross-proc";
    ledger.append(sid, event_kinds::SOLVE_STARTED, 1, &json!({})).unwrap();

    let report = harvest_from_sympy_result(
        &ledger,
        sid,
        ReceiptRef { call_id: "call-r6".into(), result_seq: 2 },
        &eval_res,
        &checker,
    )
    .unwrap();
    assert_eq!(report.harvested.len(), 2);

    let lam = report.harvested.iter().find(|h| h.expr.name == "bad_lam").unwrap();
    assert!(!lam.expr.portable, "Lambda must not be portable");
    assert!(lam.expr.latex.is_empty());
    assert!(lam.expr.str_preview.is_empty());
    assert!(lam.expr.srepr_blob.is_empty());
    assert!(lam.expr.srepr_sha256.is_empty());
    assert_eq!(lam.rejected_constructor.as_deref(), Some("Lambda"));

    let poly = report.harvested.iter().find(|h| h.expr.name == "good_poly").unwrap();
    assert!(poly.expr.portable);
    assert_eq!(poly.expr.latex, "x^{3} - 1");

    // Hostile srepr payloads are also marked non-portable without crashing.
    let mut hostile = BTreeMap::new();
    hostile.insert("h_import".into(), "__import__('os').system('echo pwn')".into());
    hostile.insert("h_attr".into(), "Symbol('x').__class__".into());
    hostile.insert("h_eval".into(), "eval('1 + 1')".into());
    let hostile_rep = harvest_exprs_out(
        &ledger,
        sid,
        ReceiptRef { call_id: "call-hostile".into(), result_seq: 5 },
        &hostile,
        &checker,
    )
    .unwrap();
    assert_eq!(hostile_rep.harvested.len(), 3);
    for h in &hostile_rep.harvested {
        assert!(!h.expr.portable, "hostile `{}` must be non-portable", h.expr.name);
        assert!(h.expr.latex.is_empty());
        assert!(h.expr.srepr_blob.is_empty());
    }

    let state = SolveState::from_events(&ledger.read_session(sid).unwrap());
    assert_eq!(state.handles.len(), 5);
    assert!(!state.handles[&HandleRef::parse("bad_lam@1").unwrap()].portable);
    assert!(state.handles[&HandleRef::parse("good_poly@1").unwrap()].portable);
}

#[test]
fn cross_process_worker_restart_restages_exprs_in_from_ledger_blobs_without_cas_bus() {
    let Some(py) = resolve_sympy_python() else {
        eprintln!("SKIPPED: set RE_SEARCH_PYTHON (Python with SymPy) to run cross-process S7 worker test");
        return;
    };
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let scratch = tmp.path().join("scratch");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&scratch).unwrap();

    let checker = RealCheckProcessCanonicalizer(Mutex::new(StdioWorkerProcess::spawn(&py, "check", &scratch)));
    let ledger = Ledger::open(&data_dir).unwrap();
    let sid = "s-restart-restage";
    ledger.append(sid, event_kinds::SOLVE_STARTED, 1, &json!({})).unwrap();

    // First solver process saves V1@1, updates to V1@2, and saves non-portable bad_lam@1.
    let pid1 = {
        let mut solver1 = StdioWorkerProcess::spawn(&py, "solver", &scratch);
        let r1 = solver1.evaluate(
            r#"
x = assumed_symbol("x", positive=True)
save_expr("V1", x**2 + 1)
"#,
            None,
        );
        assert!(r1["error"].is_null());
        harvest_from_sympy_result(
            &ledger,
            sid,
            ReceiptRef { call_id: "c-1".into(), result_seq: 2 },
            &r1,
            &checker,
        )
        .unwrap();

        let r2 = solver1.evaluate(
            r#"
x = assumed_symbol("x", positive=True)
save_expr("V1", x**3 + 2)
save_expr("bad_lam", Lambda(x, x + 1))
"#,
            None,
        );
        assert!(r2["error"].is_null());
        harvest_from_sympy_result(
            &ledger,
            sid,
            ReceiptRef { call_id: "c-2".into(), result_seq: 4 },
            &r2,
            &checker,
        )
        .unwrap();
        solver1.pid
    }; // solver1 is killed and dropped here

    assert!(!scratch.join(".cas_bus.json").exists(), "no .cas_bus.json may be created");

    // Reconstruct staged `exprs_in` solely from ledger blobs.
    let staged = stage_exprs_in(&ledger, sid).unwrap();
    assert_eq!(staged.len(), 1, "only portable V1@2 should be staged: {staged:?}");
    assert!(staged.contains_key("V1"));
    assert!(!staged.contains_key("bad_lam"));

    // Spawn a brand-new solver process and verify `expr("V1")` resolves to V1@2 and `bad_lam` is absent.
    let mut solver2 = StdioWorkerProcess::spawn(&py, "solver", &scratch);
    assert_ne!(solver2.pid, pid1, "must be a fresh solver OS process");

    let check_res = solver2.evaluate(
        r#"
x = assumed_symbol("x", positive=True)
simplify(expr("V1") - (x**3 + 2))
"#,
        Some(&staged),
    );
    assert!(check_res["error"].is_null(), "restaged eval failed: {check_res}");
    assert_eq!(check_res["value"], json!("0"));

    let bad_res = solver2.evaluate(r#"expr("bad_lam")"#, Some(&staged));
    assert_eq!(bad_res["error"]["type"], json!("KeyError"));
    assert!(!scratch.join(".cas_bus.json").exists(), "no .cas_bus.json may be created after restart");
}

#[test]
fn solve_v1_router_enforces_r18_policy_per_caller_worker_session_and_fail_closed_stubs() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let sid = "s-router-r18";
    let ledger = Ledger::open(&data_dir).unwrap();
    ledger.append(sid, event_kinds::SOLVE_STARTED, 1, &json!({})).unwrap();

    let sympy_epoch = Arc::new(FixedEpoch::new(1, true));
    let check_epoch = Arc::new(FixedEpoch::new(7, true));
    let sympy_handler: Arc<dyn ToolHandler> = Arc::new(|ctx: &CallCtx<'_>, _args: &Value| {
        ctx.record_worker_execution(1, 1);
        Ok(json!({ "value": "2", "stdout": "", "stderr": "", "truncated": false }))
    });

    // 1. Approving router: verify policy version, WorkerSession per-caller reuse, and S7 stubs.
    let approver = Arc::new(CountingApprover::new(true));
    let router = build_solve_router(
        &data_dir,
        sid,
        sympy_epoch.clone(),
        sympy_handler.clone(),
        check_epoch.clone(),
        approver.clone(),
    );
    assert_eq!(router.policy().version, SOLVE_POLICY_VERSION);
    assert_eq!(router.registry().defs().len(), 16);

    // `sympy_eval` prompts once for `solver:solve`, then reuses WorkerSession grant.
    assert!(matches!(
        router.try_call("solver:solve", "sympy_eval", &json!({ "code": "1 + 1" })).unwrap(),
        CallOutcome::Ok(_)
    ));
    assert_eq!(approver.count(), 1);
    assert!(matches!(
        router.try_call("solver:solve", "sympy_eval", &json!({ "code": "1 + 1" })).unwrap(),
        CallOutcome::Ok(_)
    ));
    assert_eq!(approver.count(), 1, "same caller + live epoch must reuse WorkerSession grant");

    // Different caller (`verifier:solve`) needs its own grant.
    assert!(matches!(
        router.try_call(SOLVE_VERIFIER_CALLER, "sympy_eval", &json!({ "code": "1 + 1" })).unwrap(),
        CallOutcome::Ok(_)
    ));
    assert_eq!(approver.count(), 2, "verifier caller must not inherit solver WorkerSession grant");

    // Rule R5: `step_record` rejects `latex` at schema validation; valid input returns S8 stub error.
    let r5_err = router
        .try_call("solver:solve", STEP_RECORD, &json!({ "entry_type": "derivation", "summary": "s", "details": "d", "latex": "x^2" }))
        .unwrap();
    assert!(matches!(&r5_err, CallOutcome::Error(m) if m.contains("unknown argument `latex`")), "{r5_err:?}");

    let s8_err = router
        .try_call("solver:solve", STEP_RECORD, &json!({ "entry_type": "assumption", "summary": "s", "details": "d" }))
        .unwrap();
    assert_eq!(s8_err, CallOutcome::Error("step_record is not wired yet (package S8)".into()));

    // Rule R3: `check_run` rejects `code`/`expected`/`outcome`; valid input asks approval then returns S9 stub error.
    for forbidden in [
        json!({ "entry_id": "e-1", "check": "identity_symbolic", "code": "True" }),
        json!({ "entry_id": "e-1", "check": "identity_symbolic", "expected": "0" }),
        json!({ "entry_id": "e-1", "check": "identity_symbolic", "outcome": "pass" }),
    ] {
        let out = router.try_call("solver:solve", CHECK_RUN, &forbidden).unwrap();
        assert!(matches!(out, CallOutcome::Error(ref m) if m.contains("unknown argument")), "{out:?}");
    }
    let s9_err = router
        .try_call("solver:solve", CHECK_RUN, &json!({ "entry_id": "e-1", "claim_index": 0, "check": "identity_symbolic", "params": {} }))
        .unwrap();
    assert_eq!(s9_err, CallOutcome::Error("check_run is not wired yet (package S9)".into()));
    assert_eq!(approver.count(), 3, "check_run must prompt approval before calling handler");

    // `solve_finish` returns S11 stub error writing zero `solve.done` events.
    let s11_err = router
        .try_call("solver:solve", SOLVE_FINISH, &json!({ "final_entry_id": "e-1", "summary_markdown": "done" }))
        .unwrap();
    assert_eq!(s11_err, CallOutcome::Error("solve_finish is not wired yet (package S11)".into()));

    // Excluded tools are absent from `solve/1` registry.
    for excluded in ["doc_ingest", "ledger_append", "lean_check", "fs_write", "record_paper_map", "idea_search"] {
        let out = router.try_call("solver:solve", excluded, &json!({})).unwrap();
        assert!(matches!(out, CallOutcome::Denied(_)), "excluded tool `{excluded}` must be Denied: {out:?}");
    }

    // `verifier:solve` denied solver-only tools; `host:check` denied non-`check_run` tools.
    assert!(matches!(
        router.try_call(SOLVE_VERIFIER_CALLER, STEP_RECORD, &json!({ "entry_type": "assumption", "summary": "s", "details": "d" })).unwrap(),
        CallOutcome::Denied(_)
    ));
    assert!(matches!(router.try_call(SOLVE_CHECK_CALLER, EXPR_SHOW, &json!({})).unwrap(), CallOutcome::Denied(_)));

    // Verify ZERO domain events (`proof.entry`, `solve.plan`, `solve.check`, `solve.done`) were written by S7 stubs.
    for ev in &ledger.read_session(sid).unwrap() {
        assert!(
            !matches!(
                ev.kind.as_str(),
                event_kinds::PROOF_ENTRY | event_kinds::SOLVE_PLAN | event_kinds::SOLVE_CHECK | event_kinds::SOLVE_DONE
            ),
            "S7 stub wrote forbidden domain event `{}`",
            ev.kind
        );
    }

    // 2. Denying approver: `host:check` without a grant gets Denied (rule R18: not_run, never executes).
    let denying = Arc::new(CountingApprover::new(false));
    let router_deny = build_solve_router(&data_dir, sid, sympy_epoch, sympy_handler, check_epoch, denying.clone());
    let denied = router_deny
        .try_call(SOLVE_CHECK_CALLER, CHECK_RUN, &json!({ "entry_id": "e-1", "check": "identity_symbolic" }))
        .unwrap();
    assert!(matches!(denied, CallOutcome::Denied(_)), "expected Denied without grant: {denied:?}");
    assert_eq!(denying.count(), 1);
}
