use super::*;
use crate::proof_dag::{ProofEntry, ProofStatus};
use crate::solve_model::{ClaimForm, ClaimRelation, ReceiptRef, StepEntryType};

fn solve_entry(id: &str, deps: &[&str], derived: &[&str]) -> SolveEntry {
    SolveEntry {
        entry_id: id.into(),
        seq: 1,
        entry_type: StepEntryType::Derivation,
        summary: String::new(),
        details: String::new(),
        latex: None,
        depends_on: deps.iter().map(|s| (*s).into()).collect(),
        derived_depends_on: derived.iter().map(|s| (*s).into()).collect(),
        tool_call_ref: None,
        tool_call_refs: Vec::new(),
        claims: Vec::new(),
        claim_sha256: None,
        entry_sha256: None,
        origin: None,
        source_ref: None,
        transcription: None,
        superseded_by: None,
    }
}

fn proof_entry(id: &str, deps: &[&str]) -> ProofEntry {
    ProofEntry {
        entry_id: id.into(),
        seq: 1,
        entry_type: "derivation".into(),
        status: ProofStatus::Pending,
        summary: String::new(),
        details: String::new(),
        latex: None,
        depends_on: deps.iter().map(|s| (*s).into()).collect(),
        tool_call_ref: None,
        superseded_by: None,
        verification: None,
    }
}

#[test]
fn closure_traverses_declared_and_derived_edges_together() {
    let nodes = vec![
        solve_entry("e-1", &[], &[]),
        solve_entry("e-2", &[], &[]),
        solve_entry("e-3", &["e-1"], &[]),
        // e-4 declares e-3 and has derived dependency on e-2 (handle owner).
        solve_entry("e-4", &["e-3"], &["e-2"]),
        solve_entry("e-5", &[], &[]), // unrelated
        solve_entry("e-6", &["e-4"], &[]),
    ];

    assert_eq!(closure_ids(&nodes, &["e-6"]), vec!["e-1", "e-2", "e-3", "e-4", "e-6"]);
    assert_eq!(proper_ancestor_ids(&nodes, "e-6"), vec!["e-1", "e-2", "e-3", "e-4"]);
    assert_eq!(closure_for_target(&nodes, "e-4").unwrap(), vec!["e-1", "e-2", "e-3", "e-4"]);
    assert!(closure_for_target(&nodes, "e-99").is_err());

    let refs = closure_entries(&nodes, &["e-6"]);
    assert_eq!(refs.len(), 5);
}

#[test]
fn closure_works_on_legacy_proof_entries_from_proof_dag() {
    let nodes = vec![
        proof_entry("e-1", &[]),
        proof_entry("e-2", &["e-1"]),
        proof_entry("e-3", &[]),
        proof_entry("e-4", &["e-2"]),
    ];
    assert_eq!(closure_ids(&nodes, &["e-4"]), vec!["e-1", "e-2", "e-4"]);
    assert_eq!(topological_order(&nodes).unwrap(), vec!["e-1", "e-2", "e-3", "e-4"]);
}

#[test]
fn all_deps_deduplicates_preserving_declared_then_derived_order() {
    let node = solve_entry("e-4", &["e-2", "e-1"], &["e-1", "e-3", "e-2"]);
    assert_eq!(all_deps(&node), vec!["e-2", "e-1", "e-3"]);
}

#[test]
fn topological_order_handles_out_of_order_input_and_detects_cycles() {
    let out_of_order = vec![
        solve_entry("e-3", &["e-2"], &[]),
        solve_entry("e-2", &[], &["e-1"]),
        solve_entry("e-1", &[], &[]),
    ];
    assert_eq!(topological_order(&out_of_order).unwrap(), vec!["e-1", "e-2", "e-3"]);

    let cyclic = vec![
        solve_entry("e-1", &[], &[]),
        solve_entry("e-2", &["e-3"], &[]),
        solve_entry("e-3", &[], &["e-2"]),
    ];
    assert_eq!(topological_order(&cyclic), Err(vec!["e-2".to_string(), "e-3".to_string()]));
    // Closure still terminates safely on cycles.
    assert_eq!(closure_ids(&cyclic, &["e-2"]), vec!["e-2", "e-3"]);
    assert_eq!(proper_ancestor_ids(&cyclic, "e-2"), vec!["e-3"]);
}

#[test]
fn compute_derived_deps_collects_handle_and_receipt_owners() {
    let mut handles = BTreeMap::new();
    let h1 = HandleRef::parse("I_d@1").unwrap();
    let h2 = HandleRef::parse("V1@1").unwrap();
    handles.insert(
        h1.clone(),
        SolveExpr {
            name: "I_d".into(),
            version: 1,
            srepr_blob: "blob1".into(),
            srepr_sha256: "sha1".into(),
            latex: "I_d".into(),
            str_preview: "I_d".into(),
            free_symbols: vec![],
            produced_by: ReceiptRef { call_id: "call-1".into(), result_seq: 10 },
            owner_entry: Some("e-3".into()),
            portable: true,
        },
    );
    handles.insert(
        h2.clone(),
        SolveExpr {
            name: "V1".into(),
            version: 1,
            srepr_blob: "blob2".into(),
            srepr_sha256: "sha2".into(),
            latex: "V_1".into(),
            str_preview: "V1".into(),
            free_symbols: vec![],
            produced_by: ReceiptRef { call_id: "call-2".into(), result_seq: 12 },
            owner_entry: Some("e-4".into()),
            portable: true,
        },
    );

    let mut e5 = solve_entry("e-5", &[], &[]);
    e5.tool_call_refs = vec!["call-3".into()];
    let entries = vec![e5];

    let claims = vec![Claim {
        form: ClaimForm::Equation,
        lhs: h1,
        rhs: h2,
        relation: ClaimRelation::Eq,
        order: None,
        domain: None,
        how: None,
    }];

    // Excludes self ("e-4") and deduplicates owners from both claims and receipts.
    let derived = compute_derived_deps(
        &claims,
        &["call-1".into(), "call-3".into()],
        &handles,
        &entries,
        Some("e-4"),
    );
    assert_eq!(derived, vec!["e-3", "e-5"]);
}
