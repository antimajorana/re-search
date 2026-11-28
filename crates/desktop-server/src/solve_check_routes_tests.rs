use super::*;
use crate::auth::AuthGuard;
use crate::chatgpt::ChatGptHub;
use crate::sse::SseHub;
use crate::DesktopConfig;
use ledger::sha256_hex;
use research_tools::solve_model::{event_kinds, Claim, ClaimForm, ClaimRelation, HandleRef};
use research_tools::solve_steps::run_step_record;
use research_tools::LiteratureEndpoints;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use tempfile::TempDir;

struct StdioCheckProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl StdioCheckProcess {
    fn spawn(cwd: &Path) -> Self {
        let py = find_sympy_python();
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("python/worker.py");
        let mut child = Command::new(&py.0)
            .args(&py.1)
            .arg("-u")
            .arg(script)
            .arg("--stdio")
            .arg("--role")
            .arg("check")
            .current_dir(cwd)
            .env("PYTHONNOUSERSITE", "1")
            .env("PYTHONIOENCODING", "utf-8")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn check worker");
        let stdin = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let hello: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(hello["ready"], true);
        assert_eq!(hello["role"], "check");
        Self { child, stdin, stdout, next_id: 1 }
    }
}

impl Drop for StdioCheckProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct SubprocessCheckRunner {
    proc: Mutex<StdioCheckProcess>,
}

impl CheckProcessRunner for SubprocessCheckRunner {
    fn run_check(
        &self,
        name: &str,
        handles: &BTreeMap<String, String>,
        params: &Value,
        _timeout: Duration,
        _required_epoch: Option<u64>,
    ) -> Result<CheckExecResult, CheckExecError> {
        let mut g = self.proc.lock().unwrap();
        let id = g.next_id;
        g.next_id += 1;
        let line = format!(
            "{}\n",
            json!({
                "id": id,
                "method": "check",
                "params": { "name": name, "handles": handles, "params": params }
            })
        );
        g.stdin.write_all(line.as_bytes()).unwrap();
        g.stdin.flush().unwrap();
        let mut resp_line = String::new();
        g.stdout.read_line(&mut resp_line).unwrap();
        let resp: Value = serde_json::from_str(&resp_line).unwrap();
        if resp["ok"] == true {
            Ok(CheckExecResult {
                raw: resp["result"].clone(),
                epoch: 1,
                request_id: id,
                environment: Some(json!({ "role": "check", "protocol": 2 })),
            })
        } else {
            Err(CheckExecError::Worker(resp["error"].to_string()))
        }
    }
}

fn find_sympy_python() -> (String, Vec<&'static str>) {
    if let Ok(p) = std::env::var("RE_SEARCH_PYTHON") {
        if !p.trim().is_empty() {
            return (p, vec![]);
        }
    }
    let candidates: &[(&str, &[&str])] = if cfg!(windows) {
        &[("python", &[]), ("py", &["-3"])]
    } else {
        &[("python3", &[]), ("python", &[])]
    };
    for &(prog, args) in candidates {
        if let Ok(out) = Command::new(prog)
            .args(args)
            .args(["-c", "import sympy, mpmath"])
            .env("PYTHONNOUSERSITE", "1")
            .output()
        {
            if out.status.success() {
                return (prog.to_string(), args.to_vec());
            }
        }
    }
    panic!("SymPy+mpmath Python required for desktop-server solve_check_routes tests");
}

struct TestEpoch;
impl WorkerEpoch for TestEpoch {
    fn live_epoch(&self) -> Option<u64> {
        Some(1)
    }
    fn is_contained(&self) -> bool {
        true
    }
}

fn test_state(data_dir: &Path, ws: &Path) -> Arc<DesktopState> {
    let cfg = DesktopConfig {
        port: 0,
        data_dir: data_dir.to_path_buf(),
        workspace_root: Some(ws.to_path_buf()),
        python: None,
        worker_script: None,
        api_base_override: None,
        literature_endpoints: LiteratureEndpoints::default(),
        open_window: false,
        print_url_json: false,
        eval_timeout_s: 30,
        core_exe: None,
        claude_exe: None,
        agy_exe: None,
        connection_env: Some(BTreeMap::new()),
        chatgpt_test: Some(crate::chatgpt::offline_test_hooks()),
    };
    Arc::new(DesktopState::new(
        cfg.clone(),
        AuthGuard::new(1).unwrap(),
        Arc::new(SseHub::new()),
        ChatGptHub::new(&cfg).unwrap(),
    ))
}

#[test]
fn desktop_check_and_recheck_r19_preserve_receipts_and_detect_change() {
    let tmp = TempDir::new().unwrap();
    let data = tmp.path().join("data");
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&ws).unwrap();
    let state = test_state(&data, &ws);
    let sid = "s-recheck-1";
    let ledger = state.open_ledger().unwrap();
    ledger.append_with_hash(sid, event_kinds::SOLVE_STARTED, 1, &json!({})).unwrap();
    ledger
        .append_with_hash(
            sid,
            event_kinds::SOLVE_SETUP,
            1,
            &json!({
                "revision": 1,
                "statement": "x + x = 2*x",
                "task_kind": "calculate",
                "target": { "description": "id", "form": "closed_form" },
                "symbols": [{ "name": "x", "assumptions": ["real"] }],
                "roles": {
                    "solver": { "provider": "anthropic", "model": "claude-sonnet-4-6" },
                    "verifier": { "provider": "openai", "model": "gpt-5" }
                },
                "tool_profile": "solve/1",
                "math_sha256": "m".repeat(64),
            }),
        )
        .unwrap();

    let s_lhs = "Pow(Add(Symbol('x', real=True), Integer(1)), Integer(2))";
    let s_rhs = "Add(Pow(Symbol('x', real=True), Integer(2)), Mul(Integer(2), Symbol('x', real=True)), Integer(1))";
    let h_lhs = sha256_hex(s_lhs.as_bytes());
    let h_rhs = sha256_hex(s_rhs.as_bytes());
    ledger.put_blob(s_lhs.as_bytes()).unwrap();
    ledger.put_blob(s_rhs.as_bytes()).unwrap();
    ledger.append_with_hash(sid, event_kinds::TOOL_REQUEST, 1, &json!({ "call_id": "sym1", "tool": "sympy_eval", "caller": "solver" })).unwrap();
    let rseq = ledger.append_with_hash(sid, event_kinds::TOOL_RESULT, 1, &json!({ "call_id": "sym1", "ok": true, "result": { "inline": "{\"value\":\"(x+1)**2\",\"error\":null,\"code_sha256\":\"c1\",\"process\":\"solver\"}" } })).unwrap().seq;
    for (name, sha, tex) in [("lhs", h_lhs.clone(), "(x+1)^2"), ("rhs", h_rhs.clone(), "x^2+2x+1")] {
        ledger
            .append_with_hash(
                sid,
                event_kinds::SOLVE_EXPR,
                1,
                &json!({
                    "name": name, "version": 1, "srepr_blob": sha, "srepr_sha256": sha,
                    "latex": tex, "str_preview": tex, "free_symbols": [{ "name": "x", "assumptions": ["real"] }],
                    "produced_by": { "call_id": "sym1", "result_seq": rseq }, "portable": true
                }),
            )
            .unwrap();
    }

    let env = state.research_env_for_session(sid).unwrap();
    let step = run_step_record(
        &env,
        &ledger,
        sid,
        "solver",
        &json!({
            "entry_type": "symbolic_computation",
            "summary": "x+x = 2x",
            "details": "Simplify sum",
            "tool_call_refs": ["sym1"],
            "claims": [Claim {
                form: ClaimForm::Equation,
                lhs: HandleRef::parse("lhs@1").unwrap(),
                rhs: HandleRef::parse("rhs@1").unwrap(),
                relation: ClaimRelation::Eq,
                order: None,
                domain: None,
                how: None,
            }]
        }),
    )
    .unwrap();
    assert_eq!(step["entry_id"], "e-1");

    let runner: Arc<dyn CheckProcessRunner> = Arc::new(SubprocessCheckRunner {
        proc: Mutex::new(StdioCheckProcess::spawn(&ws)),
    });
    let epoch: Arc<dyn WorkerEpoch> = Arc::new(TestEpoch);

    // 1. User calls POST .../solve/check
    let (st, checked) = handle_check_with_runner(
        &state,
        sid,
        serde_json::to_vec(&json!({ "entry_id": "e-1", "check": "identity_symbolic" }))
            .unwrap()
            .as_slice(),
        Arc::clone(&runner),
        Arc::clone(&epoch),
        true,
    )
    .unwrap();
    assert_eq!(st, 200);
    assert_eq!(checked["check"]["outcome"], "pass");
    assert_eq!(checked["view"]["steps"][0]["badge"]["effective"], "verified");
    let orig_check_id = checked["check"]["check_id"].as_str().unwrap().to_string();
    let orig_evidence_sha = checked["check"]["evidence_sha256"].as_str().unwrap().to_string();

    // 2. Identical recheck matches `evidence_sha256` and preserves original receipt.
    let (st2, rechecked) = handle_recheck_with_runner(
        &state,
        sid,
        serde_json::to_vec(&json!({ "check_id": orig_check_id }))
            .unwrap()
            .as_slice(),
        Arc::clone(&runner),
        Arc::clone(&epoch),
        true,
    )
    .unwrap();
    assert_eq!(st2, 200);
    assert_eq!(rechecked["matched"], true);
    assert_eq!(rechecked["changed"], false);
    assert_eq!(rechecked["warning"], Value::Null);
    assert_eq!(rechecked["new_evidence_sha256"], orig_evidence_sha);
    assert_eq!(rechecked["view"]["checks"].as_array().unwrap().len(), 2);

    // 3. If a handle is updated/mutated (`rhs@1` redefined to `3*x`), recheck surfaces "result changed on re-run".
    let bad_rhs = "Mul(Integer(3), Symbol('x', real=True))";
    let bad_sha = sha256_hex(bad_rhs.as_bytes());
    ledger.put_blob(bad_rhs.as_bytes()).unwrap();
    ledger
        .append_with_hash(
            sid,
            event_kinds::SOLVE_EXPR,
            1,
            &json!({
                "name": "rhs", "version": 1, "srepr_blob": bad_sha, "srepr_sha256": bad_sha,
                "latex": "3x", "str_preview": "3x", "free_symbols": [{ "name": "x", "assumptions": ["real"] }],
                "produced_by": { "call_id": "sym1", "result_seq": rseq }, "portable": true
            }),
        )
        .unwrap();
    let (st3, changed) = handle_recheck_with_runner(
        &state,
        sid,
        serde_json::to_vec(&json!({ "check_id": orig_check_id }))
            .unwrap()
            .as_slice(),
        runner,
        epoch,
        true,
    )
    .unwrap();
    assert_eq!(st3, 200);
    assert_eq!(changed["matched"], false);
    assert_eq!(changed["changed"], true);
    assert_eq!(changed["warning"], "result changed on re-run");
}
