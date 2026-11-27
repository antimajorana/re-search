use export::{
    export_session_bytes, export_session_bytes_with_options, export_session_to_path, ExportFormat,
};
use ledger::{canonical_json, sha256_hex, Ledger};
use research_tools::solve_model::{
    compute_claim_sha256, compute_entry_sha256, compute_math_sha256, event_kinds, CheckOutcome,
    Claim, ClaimForm, ClaimRelation, HandleRef, RoleModel, SetupAssumption, SolveRoles,
    SolveSetup, SolveTarget, SymbolSpec, TargetForm, TaskKind,
};
use research_tools::solve_state::SolveState;
use research_tools::solve_status::Tier;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::PathBuf;
use std::process::Command;

fn python_exe() -> Option<String> {
    ["RE_SEARCH_PYTHON", "RESEARCH_CORE_PYTHON"]
        .into_iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
}

fn repo_python_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("python")
}

fn eval_real_worker_and_check(
    py: &str,
    code: &str,
    exprs_in: &BTreeMap<String, String>,
    check_name: &str,
    lhs_key: &str,
    rhs_key: &str,
    params: &Value,
) -> Value {
    let py_dir = repo_python_dir();
    let script = r#"
import hashlib, json, sys
sys.path.insert(0, sys.argv[1])
import worker, checks

payload = json.loads(sys.stdin.read())
ns = worker.make_namespace()
eval_res = worker.evaluate(payload["code"], ns, exprs_in=payload["exprs_in"])
exprs_out = {}
for name, raw_srepr in (eval_res.get("exprs_out") or {}).items():
    exprs_out[name] = checks.canonicalize_expression(raw_srepr)

lhs_srepr = exprs_out[payload["lhs_key"]]["srepr"]
rhs_srepr = (
    exprs_out[payload["rhs_key"]]["srepr"]
    if payload["rhs_key"] in exprs_out
    else checks.canonicalize_expression(payload["exprs_in"][payload["rhs_key"]])["srepr"]
)
chk_res = checks.run_registered_check(payload["check_name"], {"lhs": lhs_srepr, "rhs": rhs_srepr}, payload["params"])
ev = dict(chk_res.get("evidence") or {})
code_sha = eval_res["code_sha256"]
ev.setdefault("lhs_code_sha256", code_sha)
ev.setdefault("rhs_code_sha256", code_sha)
ev_sha = hashlib.sha256(json.dumps(ev, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False).encode("utf-8")).hexdigest()
print(json.dumps({
    "eval_res": eval_res,
    "exprs_out": exprs_out,
    "check_res": chk_res,
    "evidence": ev,
    "evidence_sha256": ev_sha,
    "registry_sha256": checks.registry_sha256(),
}))
"#;
    let input = json!({
        "code": code,
        "exprs_in": exprs_in,
        "check_name": check_name,
        "lhs_key": lhs_key,
        "rhs_key": rhs_key,
        "params": params,
    });
    let mut child = Command::new(py)
        .args(["-c", script])
        .arg(&py_dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(serde_json::to_string(&input).unwrap().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "python helper failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn setup_val(symbols: Vec<SymbolSpec>, done: Value) -> (Value, String) {
    let assumptions = vec![SetupAssumption {
        id: "a1".into(),
        text: "Parameter x is strictly positive real".into(),
    }];
    let setup = SolveSetup {
        revision: 1,
        statement: "Prove sqrt(x^2) = x for x > 0".into(),
        task_kind: TaskKind::Calculate,
        target: SolveTarget {
            description: "Exact identity sqrt(x^2) = x".into(),
            form: TargetForm::ClosedForm,
            order: None,
            digits: None,
        },
        done: serde_json::from_value(done).unwrap(),
        conventions: "real positive domain".into(),
        symbols: symbols.clone(),
        dimensions: Default::default(),
        assumptions: assumptions.clone(),
        given_source: None,
        sources: vec!["notes".into()],
        roles: SolveRoles {
            solver: RoleModel { provider: "openai".into(), model: "m-sol".into() },
            verifier: RoleModel { provider: "anthropic".into(), model: "m-ver".into() },
            compute: None,
        },
        budgets: Default::default(),
        review: Default::default(),
        tool_profile: event_kinds::SOLVE_TOOL_PROFILE_V1.into(),
        origin: None,
        math_sha256: compute_math_sha256(&symbols, &Default::default(), &assumptions, "real positive domain"),
    };
    let msha = setup.math_sha256.clone();
    (serde_json::to_value(setup).unwrap(), msha)
}

#[test]
fn solve_notebook_runs_in_standalone_python_and_zip_replays_r19() {
    let tmp = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-solve-replay";
    let syms = vec![SymbolSpec { name: "x".into(), assumptions: vec!["positive".into()] }];
    let (sval, msha) = setup_val(
        syms.clone(),
        json!([{ "id": "c-final", "kind": "final_tier_at_least", "params": { "tier": "verified" } }]),
    );

    ledger.append_with_hash(sid, "session.start", 1, &json!({ "tool_profile": "solve/1" })).unwrap();
    ledger.append_with_hash(sid, event_kinds::SOLVE_STARTED, 1, &json!({})).unwrap();
    ledger.append_with_hash(sid, event_kinds::SOLVE_SETUP, 1, &sval).unwrap();

    // Seed an ingested source file for the bundle source manifest.
    let src_dir = tmp.path().join("sessions").join(sid).join("sources").join("notes");
    std::fs::create_dir_all(&src_dir).unwrap();
    let src_bytes = b"For x > 0, sqrt(x^2) = x.\n";
    std::fs::write(src_dir.join("ref.tex"), src_bytes).unwrap();
    let raw_blob = ledger.put_blob(src_bytes).unwrap();
    ledger
        .append_with_hash(
            sid,
            "source.ingested",
            1,
            &json!({
                "source_id": "notes",
                "raw_blob": raw_blob,
                "files": ["notes/ref.tex"],
                "files_sha256": { "notes/ref.tex": sha256_hex(src_bytes) },
            }),
        )
        .unwrap();

    let code1 = "x = assumed_symbol('x', positive=True)\nlhs = sqrt(x**2)\nrhs = x\nsave_expr('lhs', lhs)\nsave_expr('rhs', rhs)\nsimplify(lhs - rhs)";
    let code2 = "prev = expr('lhs')\nsave_expr('rt', prev * 1)\nprev - expr('rhs')";
    let code_discard = "dead = 99\ndead";

    let py_opt = python_exe();
    let (eval1, lhs_canon, rhs_canon, chk1_res, chk1_ev, chk1_sha, reg_sha) = if let Some(py) = &py_opt {
        let params = json!({ "symbols": syms, "dimensions": {}, "relation": "eq" });
        let out = eval_real_worker_and_check(py, code1, &BTreeMap::new(), "identity_symbolic", "lhs", "rhs", &params);
        (
            out["eval_res"].clone(),
            out["exprs_out"]["lhs"].clone(),
            out["exprs_out"]["rhs"].clone(),
            out["check_res"].clone(),
            out["evidence"].clone(),
            out["evidence_sha256"].as_str().unwrap().to_string(),
            out["registry_sha256"].as_str().unwrap().to_string(),
        )
    } else {
        let l_srepr = "Pow(Pow(Symbol('x', positive=True), Integer(2)), Half)";
        let r_srepr = "Symbol('x', positive=True)";
        let ev = json!({ "comparison": "exact", "exact": true });
        (
            json!({ "value": "0", "comparison": { "kind": "sympy", "value": "Integer(0)" }, "stdout": "", "stderr": "", "error": null, "truncated": false }),
            json!({ "srepr": l_srepr, "srepr_sha256": sha256_hex(l_srepr.as_bytes()), "latex": r"\sqrt{x^{2}}", "str_preview": "sqrt(x**2)" }),
            json!({ "srepr": r_srepr, "srepr_sha256": sha256_hex(r_srepr.as_bytes()), "latex": "x", "str_preview": "x" }),
            json!({ "outcome": "pass" }),
            ev.clone(),
            sha256_hex(canonical_json(&ev).as_bytes()),
            "fallback-reg".to_string(),
        )
    };

    let req1 = ledger.append_with_hash(sid, event_kinds::TOOL_REQUEST, 1, &json!({
        "call_id": "c-1", "tool": "sympy_eval", "args": { "inline": json!({ "code": code1 }).to_string() }
    })).unwrap().seq;
    let res1 = ledger.append_with_hash(sid, event_kinds::TOOL_RESULT, 1, &json!({
        "call_id": "c-1", "ok": true, "result": { "inline": eval1.to_string() },
        "worker_execution": { "instance": "w-1", "epoch": 1, "request_id": 1 }
    })).unwrap().seq;

    let h_lhs = ledger.put_blob(lhs_canon["srepr"].as_str().unwrap().as_bytes()).unwrap();
    ledger.append_with_hash(sid, event_kinds::SOLVE_EXPR, 1, &json!({
        "name": "lhs", "version": 1, "srepr_blob": h_lhs, "srepr_sha256": h_lhs,
        "latex": lhs_canon["latex"], "str_preview": lhs_canon["str_preview"],
        "free_symbols": [], "produced_by": { "call_id": "c-1", "result_seq": res1 }, "owner_entry": "e-1", "portable": true
    })).unwrap();
    let h_rhs = ledger.put_blob(rhs_canon["srepr"].as_str().unwrap().as_bytes()).unwrap();
    ledger.append_with_hash(sid, event_kinds::SOLVE_EXPR, 1, &json!({
        "name": "rhs", "version": 1, "srepr_blob": h_rhs, "srepr_sha256": h_rhs,
        "latex": rhs_canon["latex"], "str_preview": rhs_canon["str_preview"],
        "free_symbols": [], "produced_by": { "call_id": "c-1", "result_seq": res1 }, "owner_entry": "e-1", "portable": true
    })).unwrap();

    // Step e-1
    let claim1 = Claim {
        form: ClaimForm::Equation,
        lhs: HandleRef::new("lhs", 1).unwrap(),
        rhs: HandleRef::new("rhs", 1).unwrap(),
        relation: ClaimRelation::Eq,
        order: None,
        domain: None,
        how: None,
    };
    let csha1 = compute_claim_sha256(std::slice::from_ref(&claim1), |h| match h.name.as_str() {
        "lhs" => Some(h_lhs.as_str()),
        "rhs" => Some(h_rhs.as_str()),
        _ => None,
    }).unwrap();
    ledger.append_with_hash(sid, event_kinds::PROOF_ENTRY, 1, &json!({
        "entry_id": "e-1", "entry_type": "symbolic_computation",
        "summary": "Compute sqrt(x^2) - x", "details": "",
        "claims": [claim1], "claim_sha256": csha1,
        "entry_sha256": compute_entry_sha256(Some(&csha1), "Compute sqrt(x^2) - x", ""),
        "tool_call_refs": ["c-1"], "origin": "solver"
    })).unwrap();
    ledger.append_with_hash(sid, event_kinds::SOLVE_CHECK, 1, &json!({
        "check_id": "chk-1", "claim_sha256": csha1, "claim_index": 0, "entry_id": "e-1",
        "math_sha256": msha, "name": "identity_symbolic", "registry_sha256": reg_sha, "params": {},
        "outcome": chk1_res["outcome"], "asked_by": "host",
        "receipt": { "call_id": "chk-c1", "result_seq": 99 },
        "evidence": chk1_ev, "evidence_sha256": chk1_sha
    })).unwrap();

    // Receipt c-2 uses exprs_in (`expr('lhs')`) and saves `rt`.
    let (eval2, chk2_res, chk2_ev, chk2_sha) = if let Some(py) = &py_opt {
        let mut ein = BTreeMap::new();
        ein.insert("lhs".into(), lhs_canon["srepr"].as_str().unwrap().to_string());
        ein.insert("rhs".into(), rhs_canon["srepr"].as_str().unwrap().to_string());
        let params = json!({ "symbols": syms, "dimensions": {}, "relation": "eq" });
        let out = eval_real_worker_and_check(py, code2, &ein, "identity_symbolic", "rt", "rhs", &params);
        let mut ev2 = out["evidence"].clone();
        if let Some(obj) = ev2.as_object_mut() {
            obj.insert("lhs_code_sha256".into(), out["eval_res"]["code_sha256"].clone());
            obj.insert("rhs_code_sha256".into(), eval1["code_sha256"].clone());
        }
        let sha2 = sha256_hex(canonical_json(&ev2).as_bytes());
        (out["eval_res"].clone(), out["check_res"].clone(), ev2, sha2)
    } else {
        (
            json!({ "value": "0", "comparison": { "kind": "sympy", "value": "Integer(0)" }, "stdout": "", "stderr": "", "error": null, "truncated": false }),
            chk1_res.clone(),
            chk1_ev.clone(),
            chk1_sha.clone(),
        )
    };
    let _req2 = ledger.append_with_hash(sid, event_kinds::TOOL_REQUEST, 1, &json!({
        "call_id": "c-2", "tool": "sympy_eval", "args": { "inline": json!({ "code": code2 }).to_string() }
    })).unwrap().seq;
    let res2 = ledger.append_with_hash(sid, event_kinds::TOOL_RESULT, 1, &json!({
        "call_id": "c-2", "ok": true, "result": { "inline": eval2.to_string() },
        "worker_execution": { "instance": "w-1", "epoch": 1, "request_id": 2 }
    })).unwrap().seq;
    ledger.append_with_hash(sid, event_kinds::SOLVE_EXPR, 1, &json!({
        "name": "rt", "version": 1, "srepr_blob": h_lhs, "srepr_sha256": h_lhs,
        "latex": lhs_canon["latex"], "str_preview": lhs_canon["str_preview"],
        "free_symbols": [], "produced_by": { "call_id": "c-2", "result_seq": res2 }, "owner_entry": "e-2", "portable": true
    })).unwrap();

    // Discarded branch e-discarded (must disappear under default D18 closure!)
    ledger.append_with_hash(sid, event_kinds::TOOL_REQUEST, 1, &json!({
        "call_id": "c-discard", "tool": "sympy_eval", "args": { "inline": json!({ "code": code_discard }).to_string() }
    })).unwrap();
    ledger.append_with_hash(sid, event_kinds::TOOL_RESULT, 1, &json!({
        "call_id": "c-discard", "ok": true, "result": { "inline": json!({ "value": "99", "comparison": { "kind": "int", "value": "99" }, "stdout": "", "stderr": "", "error": null, "truncated": false }).to_string() },
        "worker_execution": { "instance": "w-1", "epoch": 1, "request_id": 3 }
    })).unwrap();
    ledger.append_with_hash(sid, event_kinds::PROOF_ENTRY, 1, &json!({
        "entry_id": "e-discarded", "entry_type": "derivation",
        "summary": "Unconnected scratch branch", "details": "", "tool_call_refs": ["c-discard"], "origin": "solver"
    })).unwrap();

    // Final step e-2 depends on e-1
    let claim2 = Claim {
        form: ClaimForm::Equation,
        lhs: HandleRef::new("rt", 1).unwrap(),
        rhs: HandleRef::new("rhs", 1).unwrap(),
        relation: ClaimRelation::Eq,
        order: None,
        domain: None,
        how: None,
    };
    let csha2 = compute_claim_sha256(std::slice::from_ref(&claim2), |h| match h.name.as_str() {
        "rt" => Some(h_lhs.as_str()),
        "rhs" => Some(h_rhs.as_str()),
        _ => None,
    }).unwrap();
    ledger.append_with_hash(sid, event_kinds::PROOF_ENTRY, 1, &json!({
        "entry_id": "e-2", "entry_type": "final_answer",
        "summary": "Final identity from staged lhs", "details": "", "depends_on": ["e-1"],
        "claims": [claim2], "claim_sha256": csha2,
        "entry_sha256": compute_entry_sha256(Some(&csha2), "Final identity from staged lhs", ""),
        "tool_call_refs": ["c-2"], "origin": "solver"
    })).unwrap();
    ledger.append_with_hash(sid, event_kinds::SOLVE_CHECK, 1, &json!({
        "check_id": "chk-2", "claim_sha256": csha2, "claim_index": 0, "entry_id": "e-2",
        "math_sha256": msha, "name": "identity_symbolic", "registry_sha256": reg_sha, "params": {},
        "outcome": chk2_res["outcome"], "asked_by": "host",
        "receipt": { "call_id": "chk-c2", "result_seq": 100 },
        "evidence": chk2_ev, "evidence_sha256": chk2_sha
    })).unwrap();
    ledger.append_with_hash(sid, event_kinds::SOLVE_DONE, 1, &json!({
        "final_entry_id": "e-2", "own_tier": "verified", "effective_tier": "verified",
        "criteria": [{ "id": "c-final", "met": true, "evidence": {} }],
        "rests_on": { "given": [], "cited": [], "assumed_by_model": [], "weakest": [] },
        "status": "finished", "model_calls": { "solver": 2, "verifier": 0, "workers": 0 }
    })).unwrap();

    // 1. Export Notebook (.ipynb) and check determinism & D18.
    let (nb_name, nb_bytes) = export_session_bytes(tmp.path(), sid, ExportFormat::Notebook).unwrap();
    let (_, nb_bytes_2) = export_session_bytes_with_options(tmp.path(), sid, ExportFormat::Notebook, false).unwrap();
    assert_eq!(nb_name, format!("{sid}-notebook.ipynb"));
    assert_eq!(nb_bytes, nb_bytes_2);
    let nb: Value = serde_json::from_slice(&nb_bytes).unwrap();
    assert_eq!(nb["nbformat"], 4);
    assert_eq!(nb["nbformat_minor"], 5);

    // 2. Export ZIP bundle and check determinism + contents.
    let (zip_name, zip_bytes) = export_session_bytes(tmp.path(), sid, ExportFormat::Zip).unwrap();
    let (_, zip_bytes_2) = export_session_bytes(tmp.path(), sid, ExportFormat::Zip).unwrap();
    assert_eq!(zip_name, format!("{sid}-bundle.zip"));
    assert_eq!(zip_bytes, zip_bytes_2);

    let out_dir = tmp.path().join("solve-zip-out");
    zip::ZipArchive::new(Cursor::new(&zip_bytes)).unwrap().extract(&out_dir).unwrap();
    for expected in [
        "notebook.ipynb",
        "ledger.json",
        "REPRODUCE.md",
        "run_all_checks.py",
        "replay_manifest.json",
        "sources/notes/ref.tex",
        "_re_search_runtime/worker.py",
        "_re_search_runtime/solver_calc.py",
        "_re_search_runtime/checks/__init__.py",
        "_re_search_runtime/checks/srepr_safe.py",
        "_re_search_runtime/checks/algebra.py",
        "_re_search_runtime/checks/algebra_probe.py",
        &format!("code/step_{req1}.py"),
    ] {
        assert!(out_dir.join(expected).is_file(), "missing `{expected}` in Solve ZIP");
    }

    let Some(py) = py_opt else {
        eprintln!("SKIPPED Solve notebook & ZIP Python replay: set RE_SEARCH_PYTHON");
        return;
    };

    // 3. Execute all code cells of the exported `.ipynb` sequentially in an isolated directory
    //    outside the repo (no worker.py / solver_calc.py on sys.path).
    let isolated_dir = tmp.path().join("isolated-nb-exec");
    std::fs::create_dir_all(&isolated_dir).unwrap();
    let combined_code: String = nb["cells"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["cell_type"] == "code")
        .map(|c| c["source"].as_array().unwrap().iter().map(|s| s.as_str().unwrap()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n# --- NEXT CELL ---\n");
    let nb_runner = format!(
        "{combined_code}\nassert expr('rt') == sp.sqrt(sp.Symbol('x', positive=True)**2)\nprint('STANDALONE_NB_OK')\n"
    );
    let nb_exec = Command::new(&py)
        .args(["-c", &nb_runner])
        .env_remove("PYTHONPATH")
        .current_dir(&isolated_dir)
        .output()
        .unwrap();
    assert!(
        nb_exec.status.success(),
        "standalone notebook execution failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&nb_exec.stdout),
        String::from_utf8_lossy(&nb_exec.stderr)
    );
    assert!(String::from_utf8_lossy(&nb_exec.stdout).contains("STANDALONE_NB_OK"));

    // 4. Replay Solve ZIP via run_all_checks.py (verify-only, full replay, R19 drift fail, handle hash tamper).
    let run_zip = |args: &[&str]| {
        Command::new(&py)
            .arg("run_all_checks.py")
            .args(args)
            .current_dir(&out_dir)
            .output()
            .unwrap()
    };
    let vo = run_zip(&["--verify-only"]);
    assert!(vo.status.success(), "{}", String::from_utf8_lossy(&vo.stderr));

    let ok_run = run_zip(&[]);
    assert!(ok_run.status.success(), "stderr={}", String::from_utf8_lossy(&ok_run.stderr));
    let out_s = String::from_utf8_lossy(&ok_run.stdout);
    assert!(out_s.contains("MATCH: solve_check chk-1 (identity_symbolic"), "{out_s}");
    assert!(out_s.contains("passed 3 worker step(s) and 2 solve check(s)"), "{out_s}");

    // R19 drift detection on `solve.check` evidence_sha256 mismatch
    let manifest_path = out_dir.join("replay_manifest.json");
    let orig_manifest = std::fs::read(&manifest_path).unwrap();
    let mut tampered: Value = serde_json::from_slice(&orig_manifest).unwrap();
    tampered["solve_checks"][0]["evidence_sha256"] = json!("0".repeat(64));
    std::fs::write(&manifest_path, serde_json::to_vec(&tampered).unwrap()).unwrap();
    let r19_fail = run_zip(&[]);
    assert!(!r19_fail.status.success());
    assert!(
        String::from_utf8_lossy(&r19_fail.stderr).contains("result changed on re-run"),
        "stderr={}",
        String::from_utf8_lossy(&r19_fail.stderr)
    );

    // Handle hash tamper fails --verify-only
    let mut tampered_handle: Value = serde_json::from_slice(&orig_manifest).unwrap();
    tampered_handle["solve_checks"][0]["handles"]["lhs"] = json!("Integer(999)");
    std::fs::write(&manifest_path, serde_json::to_vec(&tampered_handle).unwrap()).unwrap();
    let vo_fail = run_zip(&["--verify-only"]);
    assert!(!vo_fail.status.success());
    assert!(String::from_utf8_lossy(&vo_fail.stderr).contains("hash mismatch for solve check"));
}

#[test]
fn r22_property_sweep_across_all_solve_tiers_and_unmet_done() {
    let tmp = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let (sval, msha) = setup_val(
        vec![],
        json!([
            { "id": "c-final", "kind": "final_tier_at_least", "params": { "tier": "verified" } },
            { "id": "c-flags", "kind": "no_open_flags", "params": {} }
        ]),
    );

    let scenarios = [
        ("num", "identity_numeric", CheckOutcome::Pass, json!({ "points": 12, "digits": 50 }), Tier::CheckedNumerically),
        ("sym", "identity_symbolic", CheckOutcome::Pass, json!({ "exact": true }), Tier::Verified),
        ("fail", "identity_symbolic", CheckOutcome::Fail, json!({ "reason": "mismatch" }), Tier::Failed),
        ("inc", "identity_symbolic", CheckOutcome::Inconclusive, json!({ "reason": "unknown" }), Tier::Unchecked),
    ];

    for (tag, chk_name, outcome, ev, expected_tier) in scenarios {
        let sid = format!("s-sweep-{tag}");
        ledger.append_with_hash(&sid, "session.start", 1, &json!({ "tool_profile": "solve/1" })).unwrap();
        ledger.append_with_hash(&sid, event_kinds::SOLVE_STARTED, 1, &json!({})).unwrap();
        ledger.append_with_hash(&sid, event_kinds::SOLVE_SETUP, 1, &sval).unwrap();
        let hb = ledger.put_blob(b"Integer(1)").unwrap();
        for n in ["l", "r"] {
            ledger.append_with_hash(&sid, event_kinds::SOLVE_EXPR, 1, &json!({
                "name": n, "version": 1, "srepr_blob": hb, "srepr_sha256": hb,
                "latex": "1", "str_preview": "1", "free_symbols": [],
                "produced_by": { "call_id": "c1", "result_seq": 4 }, "owner_entry": "e-1", "portable": true
            })).unwrap();
        }
        let claim = Claim {
            form: ClaimForm::Equation, lhs: HandleRef::new("l", 1).unwrap(),
            rhs: HandleRef::new("r", 1).unwrap(), relation: ClaimRelation::Eq,
            order: None, domain: None, how: None,
        };
        let csha = compute_claim_sha256(std::slice::from_ref(&claim), |_| Some(hb.as_str())).unwrap();
        ledger.append_with_hash(&sid, event_kinds::PROOF_ENTRY, 1, &json!({
            "entry_id": "e-1", "entry_type": "final_answer", "summary": "Sweep claim", "details": "",
            "claims": [claim], "claim_sha256": csha, "entry_sha256": compute_entry_sha256(Some(&csha), "Sweep claim", ""),
            "origin": "solver"
        })).unwrap();
        ledger.append_with_hash(&sid, event_kinds::SOLVE_CHECK, 1, &json!({
            "check_id": "chk-s", "claim_sha256": csha, "claim_index": 0, "entry_id": "e-1",
            "math_sha256": msha, "name": chk_name, "registry_sha256": "reg", "params": ev,
            "outcome": outcome, "asked_by": "host", "receipt": { "call_id": "c1", "result_seq": 9 },
            "evidence": ev, "evidence_sha256": sha256_hex(canonical_json(&ev).as_bytes())
        })).unwrap();
        ledger.append_with_hash(&sid, event_kinds::SOLVE_DONE, 1, &json!({
            "final_entry_id": "e-1", "own_tier": expected_tier, "effective_tier": expected_tier,
            "criteria": [{ "id": "c-final", "met": expected_tier == Tier::Verified, "evidence": { "required": "verified" } }],
            "rests_on": { "given": [], "cited": [], "assumed_by_model": [], "weakest": [] },
            "status": if expected_tier == Tier::Verified { "finished" } else { "finished_open_items" },
            "model_calls": { "solver": 1, "verifier": 0, "workers": 0 }
        })).unwrap();

        let (_, bytes) = export_session_bytes(tmp.path(), &sid, ExportFormat::Notebook).unwrap();
        let nb: Value = serde_json::from_slice(&bytes).unwrap();
        let (_, events) = ledger.read_verified_session(&sid).unwrap();
        let state = SolveState::from_events(&events);
        let e1_verified = state.tier("e-1").is_some_and(|t| t.own == Tier::Verified);
        for cell in nb["cells"].as_array().unwrap().iter().filter(|c| c["cell_type"] == "markdown") {
            let text: String = cell["source"].as_array().unwrap().iter().map(|s| s.as_str().unwrap()).collect();
            let low = text.to_ascii_lowercase();
            assert!(!low.contains("proved") && !low.contains("proven") && !low.contains("novel"), "{text}");
            assert!(!text.contains("Computed"), "{text}");
            if !e1_verified {
                assert!(!low.contains("verified"), "non-verified `{tag}` cell emitted `verified`: {text}");
            }
        }
    }
}

#[test]
fn notebook_rejects_non_solve_session_and_export_to_path_works() {
    let tmp = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    ledger.append_with_hash("s-research", "session.start", 1, &json!({ "prompt": "hi" })).unwrap();
    ledger.append_with_hash("s-research", "session.end", 1, &json!({ "outcome": "completed" })).unwrap();
    assert_eq!(ExportFormat::parse("notebook").unwrap(), ExportFormat::Notebook);
    assert_eq!(ExportFormat::parse("ipynb").unwrap(), ExportFormat::Notebook);
    let err = export_session_bytes(tmp.path(), "s-research", ExportFormat::Notebook).unwrap_err();
    assert!(err.contains("Solve"), "{err}");

    let (sval, _) = setup_val(vec![], json!([]));
    ledger.append_with_hash("s-path", "session.start", 1, &json!({ "tool_profile": "solve/1" })).unwrap();
    ledger.append_with_hash("s-path", event_kinds::SOLVE_STARTED, 1, &json!({})).unwrap();
    ledger.append_with_hash("s-path", event_kinds::SOLVE_SETUP, 1, &sval).unwrap();
    ledger.append_with_hash("s-path", event_kinds::PROOF_ENTRY, 1, &json!({
        "entry_id": "e-1", "entry_type": "assumption", "summary": "Given", "details": "", "origin": "user"
    })).unwrap();
    let out_nb = tmp.path().join("out/solve.ipynb");
    let written = export_session_to_path(tmp.path(), "s-path", ExportFormat::Notebook, &out_nb).unwrap();
    assert_eq!(std::fs::read(&out_nb).unwrap(), written);
    let out_zip = tmp.path().join("out/solve.zip");
    export_session_to_path(tmp.path(), "s-path", ExportFormat::Zip, &out_zip).unwrap();
    assert!(out_zip.is_file());
}
