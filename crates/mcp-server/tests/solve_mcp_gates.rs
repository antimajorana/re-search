//! End-to-end Solve (`solve/1`) MCP parity & gate tests (Checkpoint S15c; rules R11, R12, R15, R18).
//!
//! 1. Spawns the real compiled `core` binary over stdio JSON-RPC MCP with ONLY
//!    `CORE_TOOL_PROFILE=solve/1` (no `CORE_ENABLE_RESEARCH_TOOLS`, no `CORE_WORKSPACE`) to verify
//!    `main.rs` profile activation, exact 17 `solve/1.mcp_defs()` tools, G1/G5/G7 refusals, and
//!    fail-closed `not_run` auto-checks when unapproved.
//! 2. Drives `mcp_server::solve_wire::build_solve_router_with_bindings` with real OS child
//!    Python `worker.py --stdio --role solver` & `--role check` plus a real loopback
//!    `TcpApprover` <-> `cli_route::relay::start_for_callers` to verify handle harvesting,
//!    unapproved `not_run` vs human-approved `Verified` auto-checks, and `solve_finish`.

use cli_route::relay::{start_for_callers, RelayAsk, RelayDecider, RelayDecision};
use ledger::Ledger;
use mcp_server::approver::TcpApprover;
use mcp_server::solve_wire::build_solve_router_with_bindings;
use research_tools::policy_preset::SOLVE_CHECK_CALLER;
use research_tools::solve_checks::{CheckExecError, CheckExecResult, CheckProcessRunner};
use research_tools::solve_exprs::{stage_exprs_in, CanonicalizedExpr, CheckCanonicalizer};
use research_tools::solve_model::{
    compute_math_sha256, event_kinds, CheckOutcome, ClaimForm, ClaimRelation, DoneCriterion,
    DoneCriterionKind, HandleRef, ReceiptRef, ReviewPolicy, RoleModel, SetupAssumption,
    SolveBudgets, SolveExpr, SolveRoles, SolveSetup, SolveTarget, StepEntryType, SymbolSpec,
    TargetForm, TaskKind,
};
use research_tools::solve_state::SolveState;
use research_tools::solve_status::Tier;
use research_tools::tool_profile::solve_v1;
use research_tools::ResearchEnv;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tools::{CallCtx, CallOutcome, ToolHandler, WorkerEpoch};

struct TempDir(PathBuf);
impl TempDir {
    fn new(prefix: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("re-search-{prefix}-{}-{}", std::process::id(), tools::canon::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn path(&self) -> &Path { &self.0 }
}
impl Drop for TempDir {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

struct McpClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl McpClient {
    fn spawn_core(session: &str, data_dir: &Path) -> Self {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_core"));
        for key in [
            "CORE_ENABLE_RESEARCH_TOOLS",
            "CORE_WORKSPACE",
            "RE_SEARCH_WORKSPACE",
            "CORE_APPROVAL_ADDR",
            "CORE_SESSION_TOKEN",
            "CORE_PYTHON",
            "CORE_WORKER_SCRIPT",
            "CORE_SCRATCH",
            "CORE_PYCACHE",
            "CORE_CHECK_SCRATCH",
            "CORE_CHECK_PYCACHE",
        ] {
            cmd.env_remove(key);
        }
        let mut child = cmd
            .env("CORE_SESSION", session)
            .env("CORE_DATA_DIR", data_dir)
            .env("CORE_TOOL_PROFILE", "solve/1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn core MCP binary");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut client = Self { child, stdin, stdout, next_id: 1 };
        let init = client.request("initialize", json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "solve-mcp-test", "version": "0.1.0" }
        }));
        assert!(init.get("result").is_some(), "initialize failed: {init}");
        client.notify("notifications/initialized", json!({}));
        client
    }

    fn send_json(&mut self, msg: &Value) {
        writeln!(self.stdin, "{msg}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send_json(&json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send_json(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let mut line = String::new();
            assert!(self.stdout.read_line(&mut line).unwrap() > 0, "core exited before `{method}`");
            if line.trim().is_empty() { continue; }
            let val: Value = serde_json::from_str(line.trim()).unwrap();
            if val.get("id").and_then(Value::as_u64) == Some(id) { return val; }
        }
    }

    fn call_tool(&mut self, name: &str, arguments: Value) -> (bool, String) {
        let resp = self.request("tools/call", json!({ "name": name, "arguments": arguments }));
        let result = &resp["result"];
        let is_error = result.get("isError").and_then(Value::as_bool).unwrap_or(false);
        let text = result["content"][0]["text"].as_str().unwrap_or("").to_string();
        (is_error, text)
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn seed_solve_setup(ledger: &Ledger, sid: &str, max_bounces: u32) -> String {
    let symbols = vec![SymbolSpec { name: "x".into(), assumptions: vec!["real".into()] }];
    let assumptions = vec![SetupAssumption { id: "a-1".into(), text: "x is real".into() }];
    let math_sha = compute_math_sha256(&symbols, &BTreeMap::new(), &assumptions, "standard");
    let setup = SolveSetup {
        revision: 1,
        statement: "Show x^2 - 1 = (x - 1)(x + 1)".into(),
        task_kind: TaskKind::Calculate,
        target: SolveTarget { description: "identity".into(), form: TargetForm::ClosedForm, order: None, digits: None },
        done: vec![DoneCriterion {
            id: "final_verified".into(),
            kind: DoneCriterionKind::FinalTierAtLeast,
            params: json!({ "tier": "verified" }),
        }],
        conventions: "standard".into(),
        symbols,
        dimensions: BTreeMap::new(),
        assumptions,
        given_source: None,
        sources: vec![],
        roles: SolveRoles {
            solver: RoleModel { provider: "claude-cli".into(), model: "sonnet".into() },
            verifier: RoleModel { provider: "".into(), model: "".into() },
            compute: None,
        },
        budgets: SolveBudgets { max_gate_bounces: max_bounces, ..SolveBudgets::default() },
        review: ReviewPolicy::ProseOnly,
        tool_profile: event_kinds::SOLVE_TOOL_PROFILE_V1.into(),
        origin: None,
        math_sha256: math_sha.clone(),
    };
    ledger.append_with_hash(sid, event_kinds::SOLVE_STARTED, 1, &json!({})).unwrap();
    ledger.append_with_hash(sid, event_kinds::SOLVE_SETUP, 1, &serde_json::to_value(&setup).unwrap()).unwrap();
    math_sha
}

fn seed_sympy_receipt(ledger: &Ledger, sid: &str, call_id: &str) -> u64 {
    ledger.append_with_hash(sid, "tool.request", 1, &json!({
        "call_id": call_id, "caller": format!("claude-code-cli:{sid}"), "tool": "sympy_eval",
        "args": { "inline": "{\"code\":\"1\"}" }
    })).unwrap();
    ledger.append_with_hash(sid, "tool.result", 1, &json!({
        "call_id": call_id, "ok": true, "result": { "inline": "{\"value\":\"1\",\"error\":null}" }
    })).unwrap().seq
}

fn seed_portable_handle(ledger: &Ledger, sid: &str, name: &str, srepr: &str, call_id: &str, rseq: u64) {
    let sha = ledger.put_blob(srepr.as_bytes()).unwrap();
    let expr = SolveExpr {
        name: name.into(), version: 1, srepr_blob: sha.clone(), srepr_sha256: sha,
        latex: srepr.into(), str_preview: srepr.into(), free_symbols: vec![],
        produced_by: ReceiptRef { call_id: call_id.into(), result_seq: rseq },
        owner_entry: None, portable: true,
        unevaluated_srepr: None, unevaluated_srepr_sha256: None, source_expr: None,
    };
    ledger.append_with_hash(sid, event_kinds::SOLVE_EXPR, 1, &serde_json::to_value(&expr).unwrap()).unwrap();
}

#[test]
fn core_binary_stdio_mcp_activates_solve_v1_without_research_flag_and_enforces_gates() {
    let tmp = TempDir::new("mcp-stdio-g1g5");
    let data = tmp.path().join("data");
    let ledger = Ledger::open(&data).unwrap();
    let sid = "s-mcp-g1";
    seed_solve_setup(&ledger, sid, 3);
    for idx in 1..=3 {
        seed_sympy_receipt(&ledger, sid, &format!("pending-{idx}"));
    }

    // Spawns `core` with ONLY CORE_TOOL_PROFILE=solve/1 (no CORE_ENABLE_RESEARCH_TOOLS, no workspace).
    let mut mcp = McpClient::spawn_core(sid, &data);
    let listed = mcp.request("tools/list", json!({}));
    let ids: Vec<String> = listed["result"]["tools"]
        .as_array().expect("tools array")
        .iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
    let expected: Vec<String> = solve_v1().mcp_defs().into_iter().map(|d| d.id).collect();
    assert_eq!(ids.len(), 17, "expected 17 solve/1 MCP tools, got {ids:?}");
    assert_eq!(ids, expected);

    // 4th un-cited sympy_eval hits GATE G1, and 3 consecutive bounces trigger GATE G5.
    for bounce in 1..=3 {
        let (is_err, text) = mcp.call_tool("sympy_eval", json!({ "code": "1 + 1" }));
        assert!(is_err && text.contains("GATE G1:"), "bounce {bounce} expected GATE G1, got: {text}");
    }
    let (is_err, g5_text) = mcp.call_tool("expr_show", json!({}));
    assert!(is_err && g5_text.contains("GATE G5:"), "expected GATE G5, got: {g5_text}");

    let blocked: Vec<String> = ledger.read_session(sid).unwrap().into_iter()
        .filter(|e| e.kind == "guardrail.blocked")
        .filter_map(|e| e.payload["gate"].as_str().map(str::to_string)).collect();
    assert_eq!(blocked, vec!["G1", "G1", "G1", "G5"]);
    drop(mcp);

    // Second session: unapproved `step_record` -> AutoCheckGate `not_run` (zero auto-approve),
    // and Failed closure -> `solve_finish` hits GATE G7 with zero `solve.done`.
    let sid2 = "s-mcp-g7";
    let math_sha = seed_solve_setup(&ledger, sid2, 5);
    let rseq = seed_sympy_receipt(&ledger, sid2, "sym-ok-1");
    seed_portable_handle(&ledger, sid2, "lhs", "Integer(1)", "sym-ok-1", rseq);
    seed_portable_handle(&ledger, sid2, "rhs", "Integer(1)", "sym-ok-1", rseq);

    let mut mcp2 = McpClient::spawn_core(sid2, &data);
    let (is_err, rec_text) = mcp2.call_tool("step_record", json!({
        "entry_type": "symbolic_computation",
        "summary": "Check 1 = 1",
        "details": "Recorded over MCP without approval channel",
        "depends_on": [],
        "tool_call_refs": ["sym-ok-1"],
        "claims": [{ "form": "equation", "lhs": "lhs@1", "rhs": "rhs@1", "relation": "eq" }]
    }));
    assert!(!is_err, "step_record failed: {rec_text}");
    let rec_val: Value = serde_json::from_str(&rec_text).unwrap();
    let eid = rec_val["entry_id"].as_str().unwrap().to_string();
    assert_eq!(rec_val["auto_checks"][0]["outcome"], "not_run", "unapproved host:check must be not_run: {rec_val}");
    assert_eq!(rec_val["own_tier"], "unchecked");

    // Seed a failed check on `eid` and verify `solve_finish` hits GATE G7 over MCP.
    let state = SolveState::from_events(&ledger.read_session(sid2).unwrap());
    let claim_sha = state.entry(&eid).unwrap().claim_sha256.clone().unwrap();
    ledger.append_with_hash(sid2, event_kinds::SOLVE_CHECK, 1, &json!({
        "check_id": "chk-fail-1", "claim_sha256": claim_sha, "claim_index": 0, "entry_id": eid,
        "math_sha256": math_sha, "name": "identity_symbolic", "registry_sha256": "abc", "params": {},
        "outcome": CheckOutcome::Fail, "asked_by": "host",
        "receipt": { "call_id": "c-chk", "result_seq": 1 },
        "evidence": { "counterexample": { "x": 1 } }, "evidence_sha256": "0".repeat(64)
    })).unwrap();

    let (is_err, fin_text) = mcp2.call_tool("solve_finish", json!({
        "final_entry_id": eid,
        "summary_markdown": "Attempting to finish with a failed step."
    }));
    assert!(is_err && fin_text.contains("GATE G7:"), "expected GATE G7, got: {fin_text}");
    assert!(!ledger.read_session(sid2).unwrap().iter().any(|e| e.kind == event_kinds::SOLVE_DONE));

    // Seed 2 rejected reviews on `eid` -> GATE G6 (`verification_stuck`) blocks further tool calls.
    for i in 1..=2 {
        ledger.append_with_hash(sid2, event_kinds::PROOF_VERIFICATION, 1, &json!({
            "target_entry_id": eid,
            "verdict": "rejected",
            "issues": [format!("gap #{i}")],
        })).unwrap();
    }
    let (is_err_g6, g6_text) = mcp2.call_tool("ledger_note", json!({ "text": "continue anyway" }));
    assert!(is_err_g6 && g6_text.contains("GATE G6:"), "expected GATE G6, got: {g6_text}");
}

fn require_sympy_python() -> (String, Vec<&'static str>) {
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
        if Command::new(prog).args(prefix).args(["-c", "import sympy, mpmath"])
            .env("PYTHONNOUSERSITE", "1").output().is_ok_and(|o| o.status.success())
        {
            return (prog.to_string(), prefix.to_vec());
        }
    }
    panic!("Python with sympy and mpmath required for solve_mcp_gates integration test");
}

struct StdioWorker {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl StdioWorker {
    fn spawn(py: &(String, Vec<&'static str>), role: &str, cwd: &Path) -> Self {
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join("python/worker.py");
        let mut child = Command::new(&py.0)
            .args(&py.1).arg("-u").arg(script).arg("--stdio").arg("--role").arg(role)
            .current_dir(cwd).env("PYTHONNOUSERSITE", "1").env("PYTHONIOENCODING", "utf-8")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
        let stdin = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut hello = String::new();
        stdout.read_line(&mut hello).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&hello).unwrap()["ready"], true);
        Self { child, stdin, stdout, next_id: 1 }
    }

    fn request(&mut self, method: &str, params: Value) -> (u64, Value) {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(self.stdin, "{}", json!({ "id": id, "method": method, "params": params })).unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let resp: Value = serde_json::from_str(&line).unwrap();
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

struct RealCheckBridge(Mutex<StdioWorker>);
impl CheckCanonicalizer for RealCheckBridge {
    fn canonicalize(&self, raw_srepr: &str) -> Result<CanonicalizedExpr, String> {
        let (_, resp) = self.0.lock().unwrap().request("canonicalize", json!({ "srepr": raw_srepr }));
        if resp["ok"] != true { return Err(resp["error"].to_string()); }
        serde_json::from_value(resp["result"].clone()).map_err(|e| e.to_string())
    }
}
impl CheckProcessRunner for RealCheckBridge {
    fn run_check(
        &self, name: &str, handles: &BTreeMap<String, String>, params: &Value,
        _timeout: Duration, _required_epoch: Option<u64>,
    ) -> Result<CheckExecResult, CheckExecError> {
        let (req_id, resp) = self.0.lock().unwrap().request("check", json!({ "name": name, "handles": handles, "params": params }));
        if resp["ok"] == true {
            Ok(CheckExecResult {
                raw: resp["result"].clone(), epoch: 1, request_id: req_id,
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

struct SelectiveRelayDecider {
    approve_host_check: AtomicBool,
    host_check_asks: AtomicUsize,
}
impl RelayDecider for SelectiveRelayDecider {
    fn decide(&self, ask: &RelayAsk<'_>) -> RelayDecision {
        if ask.request["caller"].as_str() == Some(SOLVE_CHECK_CALLER) {
            self.host_check_asks.fetch_add(1, Ordering::SeqCst);
            return RelayDecision::Answered {
                approved: self.approve_host_check.load(Ordering::SeqCst),
                by: "test-human-card".into(),
            };
        }
        RelayDecision::Answered { approved: true, by: "test-human-card".into() }
    }
}

#[test]
fn solve_wire_over_tcp_approver_and_real_python_workers_enforces_auto_checks_and_gates() {
    let py = require_sympy_python();
    let tmp = TempDir::new("mcp-real-py");
    let (data, ws) = (tmp.path().join("data"), tmp.path().join("ws"));
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&ws).unwrap();

    let sid = "s-mcp-py-1";
    let cli_caller = format!("claude-code-cli:{sid}");
    let ledger = Ledger::open(&data).unwrap();
    seed_solve_setup(&ledger, sid, 3);

    let decider = Arc::new(SelectiveRelayDecider {
        approve_host_check: AtomicBool::new(false),
        host_check_asks: AtomicUsize::new(0),
    });
    let token = "tok-solve-mcp-test".to_string();
    let relay = start_for_callers(
        Ledger::open(&data).unwrap(), sid.into(), token.clone(),
        vec![cli_caller.clone(), SOLVE_CHECK_CALLER.into()], decider.clone(),
    ).unwrap();
    let approver = Arc::new(TcpApprover {
        addr: relay.addr.clone(), token, containment: "test contained solver+check".into(),
        contained: true, wait: Duration::from_secs(30),
    });

    let solver = Arc::new(Mutex::new(StdioWorker::spawn(&py, "solver", &ws)));
    let checker = Arc::new(RealCheckBridge(Mutex::new(StdioWorker::spawn(&py, "check", &ws))));
    let data_clone = data.clone();
    let sympy_handler: Arc<dyn ToolHandler> = Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
        let code = args["code"].as_str().ok_or("missing `code`")?;
        let staged = stage_exprs_in(&Ledger::open(&data_clone).map_err(|e| e.to_string())?, ctx.session)?;
        let (req_id, resp) = solver.lock().unwrap().request("evaluate", json!({ "code": code, "exprs_in": staged }));
        if resp["ok"] != true { return Err(resp["error"].to_string()); }
        ctx.record_worker_execution(1, req_id);
        Ok(resp["result"].clone())
    });

    let env = Arc::new(ResearchEnv::with_workspace(data.clone(), &ws).unwrap());
    let epoch: Arc<dyn WorkerEpoch> = Arc::new(LiveEpoch);
    let wired = build_solve_router_with_bindings(
        env, ledger, sid, Arc::clone(&epoch), sympy_handler, epoch, checker.clone(), checker, approver,
    ).unwrap();

    // 1. Solver runs sympy_eval saving distinct expression pairs; SympyHarvestGate harvests them automatically.
    let mut eval_val = match wired.router.call(&cli_caller, "sympy_eval", &json!({
        "code": "import sympy as sp\nx = sp.Symbol('x', real=True)\nsave_expr('lhs', x**2 - 1)\nsave_expr('rhs', (x - 1)*(x + 1))\n1"
    })) {
        CallOutcome::Ok(v) => v,
        other => panic!("sympy_eval failed: {other:?}"),
    };
    wired.previews.attach_to_result(&mut eval_val);
    let call_id = eval_val["tool_call_ref"].as_str().unwrap().to_string();
    assert!(eval_val.get("exprs").is_some(), "expected harvested handle previews: {eval_val}");

    // 2. While host:check is denied on the relay, step_record records `not_run` (zero auto-approve).
    let step1 = match wired.router.call(&cli_caller, "step_record", &json!({
        "entry_type": StepEntryType::SymbolicComputation,
        "summary": "Factor x^2 - 1 (unapproved auto-check)",
        "details": "Factoring step",
        "depends_on": [],
        "tool_call_refs": [call_id],
        "claims": [{
            "form": ClaimForm::Equation, "lhs": HandleRef::parse("lhs@1").unwrap(),
            "rhs": HandleRef::parse("rhs@1").unwrap(), "relation": ClaimRelation::Eq
        }]
    })) {
        CallOutcome::Ok(v) => v,
        other => panic!("step_record 1 failed: {other:?}"),
    };
    assert_eq!(decider.host_check_asks.load(Ordering::SeqCst), 1);
    assert_eq!(step1["auto_checks"][0]["outcome"], "not_run");
    assert_eq!(step1["own_tier"], "unchecked");

    // 3. Human approves host:check on the relay -> Verified symbolic computation + FinalAnswer -> solve_finish!
    decider.approve_host_check.store(true, Ordering::SeqCst);
    let eval_val2 = match wired.router.call(&cli_caller, "sympy_eval", &json!({
        "code": "import sympy as sp\nx = sp.Symbol('x', real=True)\nsave_expr('lhs2', x**2 - 4)\nsave_expr('rhs2', (x - 2)*(x + 2))\nsave_expr('lhs3', x**2 - 9)\nsave_expr('rhs3', (x - 3)*(x + 3))\n1"
    })) {
        CallOutcome::Ok(v) => v,
        other => panic!("sympy_eval 2 failed: {other:?}"),
    };
    let call_id2 = eval_val2["tool_call_ref"].as_str().unwrap().to_string();
    let step2 = match wired.router.call(&cli_caller, "step_record", &json!({
        "entry_type": StepEntryType::SymbolicComputation,
        "summary": "Factor x^2 - 4 = (x - 2)(x + 2)",
        "details": "Exact algebraic identity",
        "depends_on": [],
        "tool_call_refs": [call_id2],
        "claims": [{
            "form": ClaimForm::Equation, "lhs": HandleRef::parse("lhs2@1").unwrap(),
            "rhs": HandleRef::parse("rhs2@1").unwrap(), "relation": ClaimRelation::Eq
        }]
    })) {
        CallOutcome::Ok(v) => v,
        other => panic!("step_record 2 failed: {other:?}"),
    };
    assert_eq!(decider.host_check_asks.load(Ordering::SeqCst), 2);
    assert_eq!(step2["auto_checks"][0]["outcome"], "pass");
    assert_eq!(step2["own_tier"], "verified");
    assert_eq!(step2["effective_tier"], "verified");
    let step2_id = step2["entry_id"].as_str().unwrap().to_string();

    let final_step = match wired.router.call(&cli_caller, "step_record", &json!({
        "entry_type": StepEntryType::FinalAnswer,
        "summary": "Final identity x^2 - 9 = (x - 3)(x + 3)",
        "details": "Checked final identity resting on step2.",
        "depends_on": [step2_id],
        "tool_call_refs": [call_id2],
        "claims": [{
            "form": ClaimForm::Equation, "lhs": HandleRef::parse("lhs3@1").unwrap(),
            "rhs": HandleRef::parse("rhs3@1").unwrap(), "relation": ClaimRelation::Eq
        }]
    })) {
        CallOutcome::Ok(v) => v,
        other => panic!("final step_record failed: {other:?}"),
    };
    let final_id = final_step["entry_id"].as_str().unwrap().to_string();
    assert_eq!(final_step["effective_tier"], "verified");

    let done = match wired.router.call(&cli_caller, "solve_finish", &json!({
        "final_entry_id": final_id,
        "summary_markdown": format!("Final result {final_id} is verified.")
    })) {
        CallOutcome::Ok(v) => v,
        other => panic!("solve_finish failed: {other:?}"),
    };
    assert_eq!(done["status"], "finished");

    let final_state = SolveState::from_events(&Ledger::open(&data).unwrap().read_session(sid).unwrap());
    assert_eq!(final_state.tier(&final_id).unwrap().effective, Tier::Verified);
    assert!(final_state.done.is_some());
    drop(relay);
}
