use super::{
    alloc_cell_id, build_solve_notebook_bytes, build_solve_notebook_value,
    export_solve_notebook_bytes, NotebookOptions,
};
use ledger::{canonical_json, sha256_hex, Ledger};
use research_tools::solve_model::{
    compute_claim_sha256, compute_entry_sha256, compute_math_sha256, event_kinds, CheckOutcome,
    Claim, ClaimForm, ClaimRelation, HandleRef, SetupAssumption, SolveRoles, SolveSetup,
    SolveTarget, SymbolSpec, TargetForm, TaskKind,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;

fn setup_payload(symbols: Vec<SymbolSpec>, done: Value) -> (Value, String) {
    let assumptions = vec![SetupAssumption {
        id: "a1".into(),
        text: "Parameter alpha is positive".into(),
    }];
    let setup = SolveSetup {
        revision: 1,
        statement: "Show (x + 1)^2 - (x^2 + 2 x + 1) = 0".into(),
        task_kind: TaskKind::Calculate,
        target: SolveTarget {
            description: "Exact simplified zero expression".into(),
            form: TargetForm::ClosedForm,
            order: None,
            digits: None,
        },
        done: serde_json::from_value(done).unwrap(),
        conventions: "natural units".into(),
        symbols: symbols.clone(),
        dimensions: Default::default(),
        assumptions: assumptions.clone(),
        given_source: None,
        sources: vec![],
        roles: SolveRoles {
            solver: research_tools::solve_model::RoleModel { provider: "openai".into(), model: "m-sol".into() },
            verifier: research_tools::solve_model::RoleModel { provider: "anthropic".into(), model: "m-ver".into() },
            compute: None,
        },
        budgets: Default::default(),
        review: Default::default(),
        tool_profile: event_kinds::SOLVE_TOOL_PROFILE_V1.into(),
        origin: None,
        math_sha256: compute_math_sha256(&symbols, &Default::default(), &assumptions, "natural units"),
    };
    let math_sha = setup.math_sha256.clone();
    (serde_json::to_value(setup).unwrap(), math_sha)
}

#[allow(clippy::too_many_arguments)]
fn add_expr(
    ledger: &Ledger,
    session: &str,
    name: &str,
    version: u32,
    srepr: &str,
    latex: &str,
    call_id: &str,
    result_seq: u64,
    owner: Option<&str>,
) -> String {
    let sha = ledger.put_blob(srepr.as_bytes()).unwrap();
    ledger
        .append_with_hash(
            session,
            event_kinds::SOLVE_EXPR,
            1,
            &json!({
                "name": name,
                "version": version,
                "srepr_blob": sha,
                "srepr_sha256": sha,
                "latex": latex,
                "str_preview": srepr,
                "free_symbols": [],
                "produced_by": { "call_id": call_id, "result_seq": result_seq },
                "owner_entry": owner,
                "portable": true
            }),
        )
        .unwrap();
    sha
}

fn collect_markdown_text(nb: &Value) -> String {
    nb["cells"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["cell_type"] == "markdown")
        .map(|c| {
            c["source"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n---\n")
}

#[test]
fn notebook_structure_ids_r5_r22_and_d18_closure() {
    let tmp = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-nb-test";
    ledger.append_with_hash(sid, "session.start", 1, &json!({ "tool_profile": "solve/1" })).unwrap();
    ledger.append_with_hash(sid, event_kinds::SOLVE_STARTED, 1, &json!({})).unwrap();
    let (setup_val, math_sha) = setup_payload(
        vec![SymbolSpec { name: "x".into(), assumptions: vec!["real".into()] }],
        json!([
            { "id": "c-final", "kind": "final_tier_at_least", "params": { "tier": "verified" } },
            { "id": "c-flags", "kind": "no_open_flags", "params": {} }
        ]),
    );
    ledger.append_with_hash(sid, event_kinds::SOLVE_SETUP, 1, &setup_val).unwrap();

    // Receipt 1 (cited by e-1)
    ledger
        .append_with_hash(
            sid,
            event_kinds::TOOL_REQUEST,
            1,
            &json!({
                "call_id": "call-1",
                "tool": "sympy_eval",
                "args": { "inline": json!({ "code": "x = assumed_symbol('x', real=True)\nlhs = (x + 1)**2\nrhs = x**2 + 2*x + 1\nsave_expr('lhs', lhs)\nsave_expr('rhs', rhs)\nsimplify(lhs - rhs)" }).to_string() }
            }),
        )
        .unwrap();
    let rseq1 = ledger
        .append_with_hash(
            sid,
            event_kinds::TOOL_RESULT,
            1,
            &json!({
                "call_id": "call-1",
                "ok": true,
                "result": { "inline": json!({ "stdout": "done\n", "stderr": "", "value": "0", "error": null }).to_string() }
            }),
        )
        .unwrap()
        .seq;

    let h_lhs = add_expr(
        &ledger,
        sid,
        "lhs",
        1,
        "Pow(Add(Symbol('x', real=True), Integer(1)), Integer(2))",
        r"\left(x + 1\right)^{2}",
        "call-1",
        rseq1,
        Some("e-1"),
    );
    let h_rhs = add_expr(
        &ledger,
        sid,
        "rhs",
        1,
        "Add(Pow(Symbol('x', real=True), Integer(2)), Mul(Integer(2), Symbol('x', real=True)), Integer(1))",
        r"x^{2} + 2 x + 1",
        "call-1",
        rseq1,
        Some("e-1"),
    );

    // Uncited discarded branch step e-2 + receipt call-2
    ledger
        .append_with_hash(
            sid,
            event_kinds::TOOL_REQUEST,
            1,
            &json!({
                "call_id": "call-2",
                "tool": "sympy_eval",
                "args": { "inline": json!({ "code": "discard_var = 42\ndiscard_var" }).to_string() }
            }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            event_kinds::TOOL_RESULT,
            1,
            &json!({
                "call_id": "call-2",
                "ok": true,
                "result": { "inline": json!({ "stdout": "", "stderr": "", "value": "42", "error": null }).to_string() }
            }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            event_kinds::PROOF_ENTRY,
            1,
            &json!({
                "entry_id": "e-2",
                "entry_type": "derivation",
                "summary": "Off-branch scratch attempt",
                "details": "Should stay hidden under default D18 closure.",
                "tool_call_refs": ["call-2"],
                "origin": "solver"
            }),
        )
        .unwrap();

    // Closure step e-1 with tampered `entry.latex` (R5 must ignore this!)
    let claim1 = Claim {
        form: ClaimForm::Equation,
        lhs: HandleRef::new("lhs", 1).unwrap(),
        rhs: HandleRef::new("rhs", 1).unwrap(),
        relation: ClaimRelation::Eq,
        order: None,
        domain: None,
        how: Some("polynomial expansion".into()),
    };
    let csha1 = compute_claim_sha256(std::slice::from_ref(&claim1), |h| match h.name.as_str() {
        "lhs" => Some(h_lhs.as_str()),
        "rhs" => Some(h_rhs.as_str()),
        _ => None,
    })
    .unwrap();
    ledger
        .append_with_hash(
            sid,
            event_kinds::PROOF_ENTRY,
            1,
            &json!({
                "entry_id": "e-1",
                "entry_type": "final_answer",
                "summary": "Algebraic expansion",
                "details": "Exact match of binomial square.",
                "latex": r"BOGUS_SOLVER_LATEX = 999",
                "claims": [claim1],
                "claim_sha256": csha1,
                "entry_sha256": compute_entry_sha256(Some(&csha1), "Algebraic expansion", "Exact match of binomial square."),
                "tool_call_refs": ["call-1"],
                "origin": "solver"
            }),
        )
        .unwrap();

    let ev = json!({ "exact": true });
    let ev_sha = sha256_hex(canonical_json(&ev).as_bytes());
    ledger
        .append_with_hash(
            sid,
            event_kinds::SOLVE_CHECK,
            1,
            &json!({
                "check_id": "chk-1",
                "claim_sha256": csha1,
                "claim_index": 0,
                "entry_id": "e-1",
                "math_sha256": math_sha,
                "name": "identity_symbolic",
                "registry_sha256": "reg",
                "params": {},
                "outcome": CheckOutcome::Pass,
                "asked_by": "host",
                "receipt": { "call_id": "chk-call-1", "result_seq": 99 },
                "evidence": ev,
                "evidence_sha256": ev_sha
            }),
        )
        .unwrap();

    ledger
        .append_with_hash(
            sid,
            event_kinds::SOLVE_DONE,
            1,
            &json!({
                "final_entry_id": "e-1",
                "own_tier": "verified",
                "effective_tier": "verified",
                "criteria": [
                    { "id": "c-final", "met": true, "evidence": { "actual": "verified" } },
                    { "id": "c-flags", "met": true, "evidence": {} }
                ],
                "rests_on": { "given": [], "cited": [], "assumed_by_model": [], "weakest": [] },
                "status": "finished",
                "model_calls": { "solver": 1, "verifier": 0, "workers": 0 }
            }),
        )
        .unwrap();

    // Default D18: closure of e-1 only, deterministic byte-for-byte.
    let bytes1 = export_solve_notebook_bytes(tmp.path(), sid, NotebookOptions::default()).unwrap();
    let bytes2 = export_solve_notebook_bytes(tmp.path(), sid, NotebookOptions::default()).unwrap();
    assert_eq!(bytes1, bytes2);

    let nb: Value = serde_json::from_slice(&bytes1).unwrap();
    assert_eq!(nb["nbformat"], 4);
    assert_eq!(nb["nbformat_minor"], 5);

    let cells = nb["cells"].as_array().unwrap();
    let mut ids = BTreeSet::new();
    for cell in cells {
        let id = cell["id"].as_str().unwrap();
        assert_eq!(id.len(), 8);
        assert!(id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
        assert!(ids.insert(id.to_string()));
    }

    let md = collect_markdown_text(&nb);
    assert!(md.contains(r"\left(x + 1\right)^{2} = x^{2} + 2 x + 1"), "{md}");
    assert!(!md.contains("BOGUS_SOLVER_LATEX"), "R5 violation: leaked entry.latex");
    assert!(!md.contains("Off-branch scratch attempt"), "D18 default closure leaked e-2");
    assert!(md.contains("Status:** Done"));
    assert!(md.contains("(model)"));

    // Opt-in include_all_runs includes e-2 and call-2.
    let all_bytes = export_solve_notebook_bytes(
        tmp.path(),
        sid,
        NotebookOptions { include_all_runs: true },
    )
    .unwrap();
    let all_nb: Value = serde_json::from_slice(&all_bytes).unwrap();
    let all_md = collect_markdown_text(&all_nb);
    assert!(all_md.contains("Off-branch scratch attempt"));

    let code_joined = all_nb["cells"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["cell_type"] == "code")
        .map(|c| {
            c["source"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(code_joined.contains("discard_var = 42"));
}

#[test]
fn cell_id_collision_dedup_stays_8_lowercase_hex_and_deterministic() {
    let mut seen = BTreeSet::new();
    let id1 = alloc_cell_id(&mut seen, "step", "e-1", "same");
    let id2 = alloc_cell_id(&mut seen, "step", "e-1", "same");
    let id3 = alloc_cell_id(&mut seen, "step", "e-1", "same");
    assert_ne!(id1, id2);
    assert_ne!(id2, id3);
    assert_ne!(id1, id3);
    for id in [&id1, &id2, &id3] {
        assert_eq!(id.len(), 8);
        assert!(id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
    }

    let mut seen_again = BTreeSet::new();
    assert_eq!(alloc_cell_id(&mut seen_again, "step", "e-1", "same"), id1);
    assert_eq!(alloc_cell_id(&mut seen_again, "step", "e-1", "same"), id2);
    assert_eq!(alloc_cell_id(&mut seen_again, "step", "e-1", "same"), id3);
}

#[test]
fn r22_forbids_verified_outside_verified_own_and_unmet_criteria() {
    let tmp = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-r22-num";
    ledger.append_with_hash(sid, "session.start", 1, &json!({ "tool_profile": "solve/1" })).unwrap();
    ledger.append_with_hash(sid, event_kinds::SOLVE_STARTED, 1, &json!({})).unwrap();
    let (setup_val, math_sha) = setup_payload(
        vec![],
        json!([{ "id": "c-ver", "kind": "final_tier_at_least", "params": { "tier": "verified" } }]),
    );
    ledger.append_with_hash(sid, event_kinds::SOLVE_SETUP, 1, &setup_val).unwrap();

    let ha = add_expr(&ledger, sid, "a", 1, "Integer(1)", "1", "c1", 4, Some("e-1"));
    let hb = add_expr(&ledger, sid, "b", 1, "Integer(1)", "1", "c1", 4, Some("e-1"));
    let claim = Claim {
        form: ClaimForm::Equation,
        lhs: HandleRef::new("a", 1).unwrap(),
        rhs: HandleRef::new("b", 1).unwrap(),
        relation: ClaimRelation::Eq,
        order: None,
        domain: None,
        how: None,
    };
    let csha = compute_claim_sha256(std::slice::from_ref(&claim), |h| match h.name.as_str() {
        "a" => Some(ha.as_str()),
        "b" => Some(hb.as_str()),
        _ => None,
    })
    .unwrap();
    ledger
        .append_with_hash(
            sid,
            event_kinds::PROOF_ENTRY,
            1,
            &json!({
                "entry_id": "e-1",
                "entry_type": "result",
                "summary": "Numeric match only",
                "details": "",
                "claims": [claim],
                "claim_sha256": csha,
                "entry_sha256": compute_entry_sha256(Some(&csha), "Numeric match only", ""),
                "origin": "solver"
            }),
        )
        .unwrap();
    let ev = json!({ "points": 12, "digits": 50 });
    ledger
        .append_with_hash(
            sid,
            event_kinds::SOLVE_CHECK,
            1,
            &json!({
                "check_id": "chk-num",
                "claim_sha256": csha,
                "claim_index": 0,
                "entry_id": "e-1",
                "math_sha256": math_sha,
                "name": "identity_numeric",
                "registry_sha256": "reg",
                "params": { "points": 12, "digits": 50 },
                "outcome": CheckOutcome::Pass,
                "asked_by": "host",
                "receipt": { "call_id": "cnum", "result_seq": 8 },
                "evidence": ev,
                "evidence_sha256": sha256_hex(canonical_json(&ev).as_bytes())
            }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            event_kinds::SOLVE_DONE,
            1,
            &json!({
                "final_entry_id": "e-1",
                "own_tier": "checked_numerically",
                "effective_tier": "checked_numerically",
                "criteria": [
                    { "id": "c-ver", "met": false, "evidence": { "required": "verified", "actual": "checked_numerically" } }
                ],
                "rests_on": { "given": [], "cited": [], "assumed_by_model": [], "weakest": [] },
                "status": "finished_open_items",
                "model_calls": { "solver": 1, "verifier": 0, "workers": 0 }
            }),
        )
        .unwrap();

    let (rep, events) = ledger.read_verified_session(sid).unwrap();
    let nb = build_solve_notebook_value(&ledger, &rep.head_hash, sid, &events, NotebookOptions::default()).unwrap();
    let md = collect_markdown_text(&nb);
    let lower = md.to_ascii_lowercase();
    assert!(!lower.contains("verified"), "numeric-only session leaked `verified`: {md}");
    assert!(!lower.contains("proved") && !lower.contains("proven"), "{md}");
    assert!(!md.contains("Computed"), "{md}");
    assert!(md.contains("Done with open items"), "{md}");
    assert!(md.contains("Numerically checked"), "{md}");
}

#[test]
fn r22_emits_own_check_verified_only_when_lowered_by_dependency() {
    let tmp = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-r22-lowered";
    ledger.append_with_hash(sid, "session.start", 1, &json!({ "tool_profile": "solve/1" })).unwrap();
    ledger.append_with_hash(sid, event_kinds::SOLVE_STARTED, 1, &json!({})).unwrap();
    let (setup_val, math_sha) = setup_payload(vec![], json!([]));
    ledger.append_with_hash(sid, event_kinds::SOLVE_SETUP, 1, &setup_val).unwrap();

    // e-1: unchecked model premise
    ledger
        .append_with_hash(
            sid,
            event_kinds::PROOF_ENTRY,
            1,
            &json!({
                "entry_id": "e-1",
                "entry_type": "assumption",
                "summary": "Model premise",
                "details": "",
                "origin": "solver"
            }),
        )
        .unwrap();

    // e-2: own Verified, depends on e-1 -> effective AssumedByModel
    let ha = add_expr(&ledger, sid, "x", 1, "Integer(2)", "2", "c1", 6, Some("e-2"));
    let hb = add_expr(&ledger, sid, "y", 1, "Integer(2)", "2", "c1", 6, Some("e-2"));
    let claim = Claim {
        form: ClaimForm::Equation,
        lhs: HandleRef::new("x", 1).unwrap(),
        rhs: HandleRef::new("y", 1).unwrap(),
        relation: ClaimRelation::Eq,
        order: None,
        domain: None,
        how: None,
    };
    let csha = compute_claim_sha256(std::slice::from_ref(&claim), |h| match h.name.as_str() {
        "x" => Some(ha.as_str()),
        "y" => Some(hb.as_str()),
        _ => None,
    })
    .unwrap();
    ledger
        .append_with_hash(
            sid,
            event_kinds::PROOF_ENTRY,
            1,
            &json!({
                "entry_id": "e-2",
                "entry_type": "final_answer",
                "summary": "Exact algebra on model premise",
                "details": "",
                "depends_on": ["e-1"],
                "claims": [claim],
                "claim_sha256": csha,
                "entry_sha256": compute_entry_sha256(Some(&csha), "Exact algebra on model premise", ""),
                "origin": "solver"
            }),
        )
        .unwrap();
    let ev = json!({ "exact": true });
    ledger
        .append_with_hash(
            sid,
            event_kinds::SOLVE_CHECK,
            1,
            &json!({
                "check_id": "chk-e2",
                "claim_sha256": csha,
                "claim_index": 0,
                "entry_id": "e-2",
                "math_sha256": math_sha,
                "name": "identity_symbolic",
                "registry_sha256": "reg",
                "params": {},
                "outcome": CheckOutcome::Pass,
                "asked_by": "host",
                "receipt": { "call_id": "c-e2", "result_seq": 9 },
                "evidence": ev,
                "evidence_sha256": sha256_hex(canonical_json(&ev).as_bytes())
            }),
        )
        .unwrap();

    let (rep, events) = ledger.read_verified_session(sid).unwrap();
    let bytes = build_solve_notebook_bytes(&ledger, &rep.head_hash, sid, &events, NotebookOptions::default()).unwrap();
    let nb: Value = serde_json::from_slice(&bytes).unwrap();
    let md = collect_markdown_text(&nb);
    assert!(md.contains("Badge:** Assumed by model"), "{md}");
    assert!(md.contains("own check: verified"), "{md}");
    assert!(md.contains("rests on e-1 (assumed by model)"), "{md}");
    // e-1 must NOT say verified anywhere.
    let step1_src = nb["cells"][2]["source"].as_array().unwrap().iter().map(|s| s.as_str().unwrap()).collect::<String>();
    assert!(!step1_src.to_ascii_lowercase().contains("verified"), "{step1_src}");
}
