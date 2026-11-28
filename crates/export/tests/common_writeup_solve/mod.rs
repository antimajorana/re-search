//! Shared test helpers for F15 Solve write-up tests (`writeup_solve.rs` and `writeup_solve_r22.rs`).

use ledger::Ledger;
use research_tools::library::{
    Library, LibraryIds, PaperFlags, PaperRecord, SourceItemRecord, SourceItemState, SourceKind,
    SourceRecord,
};
use research_tools::paper_key::PaperRef;
use research_tools::quote_core::QuoteRef;
use research_tools::solve_model::{
    compute_entry_sha256, compute_math_sha256, event_kinds, Claim, SetupAssumption,
    SolveDoneStatus, StepEntryType, StepOrigin, SymbolSpec,
};
use research_tools::solve_status::Tier;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

pub const SYNTH_SOURCE_TEX: &[u8] = br"\documentclass{article}
\begin{document}
\section{Finite Chain}
The discrete boundary sum equals 7/12 for every even chain length.
\end{document}
";

pub fn append_setup(ledger: &Ledger, sid: &str, statement: &str, target: &str) -> String {
    let symbols = vec![SymbolSpec {
        name: "x".into(),
        assumptions: vec!["real".into(), "positive".into()],
    }];
    let dims = BTreeMap::new();
    let assumptions = vec![SetupAssumption { id: "a-1".into(), text: "x > 0 is real".into() }];
    let conv = "hbar = c = 1";
    let math_sha = compute_math_sha256(&symbols, &dims, &assumptions, conv);
    ledger.append_with_hash(sid, event_kinds::SOLVE_STARTED, 1, &json!({})).unwrap();
    ledger
        .append_with_hash(
            sid,
            event_kinds::SOLVE_SETUP,
            1,
            &json!({
                "revision": 1,
                "statement": statement,
                "task_kind": "calculate",
                "target": { "description": target, "form": "closed_form" },
                "done": [],
                "conventions": conv,
                "symbols": symbols,
                "dimensions": dims,
                "assumptions": assumptions,
                "sources": [],
                "roles": {
                    "solver": { "provider": "anthropic", "model": "synth-solver" },
                    "verifier": { "provider": "openai", "model": "synth-verifier" }
                },
                "tool_profile": event_kinds::SOLVE_TOOL_PROFILE_V1,
                "math_sha256": math_sha,
            }),
        )
        .unwrap();
    math_sha
}

#[allow(clippy::too_many_arguments)]
pub fn append_expr(
    ledger: &Ledger,
    sid: &str,
    name: &str,
    version: u32,
    latex: &str,
    srepr: &str,
    owner: Option<&str>,
    portable: bool,
) -> String {
    let srepr_sha = ledger::sha256_hex(srepr.as_bytes());
    let blob = ledger.put_blob(srepr.as_bytes()).unwrap();
    let mut payload = json!({
        "name": name, "version": version, "srepr_blob": blob, "srepr_sha256": srepr_sha,
        "latex": latex, "str_preview": srepr, "free_symbols": [],
        "produced_by": { "call_id": format!("call-{name}-{version}"), "result_seq": 1 },
        "portable": portable,
    });
    if let Some(o) = owner {
        payload["owner_entry"] = json!(o);
    }
    ledger.append_with_hash(sid, event_kinds::SOLVE_EXPR, 1, &payload).unwrap();
    srepr_sha
}

#[allow(clippy::too_many_arguments)]
pub fn append_step(
    ledger: &Ledger,
    sid: &str,
    entry_id: &str,
    entry_type: StepEntryType,
    origin: StepOrigin,
    summary: &str,
    details: &str,
    depends_on: &[&str],
    claims: &[Claim],
    claim_sha: Option<&str>,
    bogus_entry_latex: Option<&str>,
    source_ref: Option<&QuoteRef>,
) -> String {
    let entry_sha = compute_entry_sha256(claim_sha, summary, details);
    let mut payload = json!({
        "entry_id": entry_id, "entry_type": entry_type.as_str(),
        "summary": summary, "details": details, "depends_on": depends_on,
        "origin": origin, "entry_sha256": entry_sha,
    });
    if !claims.is_empty() { payload["claims"] = json!(claims); }
    if let Some(c) = claim_sha { payload["claim_sha256"] = json!(c); }
    if let Some(lx) = bogus_entry_latex { payload["latex"] = json!(lx); }
    if let Some(sr) = source_ref { payload["source_ref"] = json!(sr); }
    ledger.append_with_hash(sid, event_kinds::PROOF_ENTRY, 1, &payload).unwrap();
    entry_sha
}

#[allow(clippy::too_many_arguments)]
pub fn append_check(
    ledger: &Ledger,
    sid: &str,
    check_id: &str,
    claim_sha: &str,
    math_sha: &str,
    name: &str,
    outcome: &str,
    evidence: Value,
) {
    let ev_sha = ledger::sha256_hex(evidence.to_string().as_bytes());
    ledger
        .append_with_hash(
            sid,
            event_kinds::SOLVE_CHECK,
            1,
            &json!({
                "check_id": check_id, "claim_sha256": claim_sha, "claim_index": 0,
                "math_sha256": math_sha, "name": name, "registry_sha256": "a".repeat(64),
                "params": {}, "outcome": outcome, "asked_by": "host",
                "receipt": { "call_id": format!("chk-{check_id}"), "result_seq": 1 },
                "evidence": evidence, "evidence_sha256": ev_sha,
            }),
        )
        .unwrap();
}

pub fn append_done(
    ledger: &Ledger,
    sid: &str,
    final_id: &str,
    own: Tier,
    eff: Tier,
    status: SolveDoneStatus,
) {
    ledger
        .append_with_hash(
            sid,
            event_kinds::SOLVE_DONE,
            1,
            &json!({
                "final_entry_id": final_id, "own_tier": own, "effective_tier": eff,
                "criteria": [], "rests_on": {}, "status": status,
                "model_calls": { "solver": 2, "verifier": 0, "workers": 0 },
            }),
        )
        .unwrap();
}

#[allow(dead_code)]
pub fn setup_library_with_citekey(data_dir: &Path, paper: &PaperRef, citekey: &str) {
    let lib = Library::open(data_dir).unwrap();
    lib.upsert_paper(&PaperRecord {
        key: paper.key.clone(),
        ids: LibraryIds::from_paper_ids(&paper.ids),
        title: "Synthetic Boundary Sums on Finite Chains".into(),
        authors: vec!["Voss, Elena".into()],
        year: Some(2024),
        abstract_text: String::new(),
        bibtex: String::new(),
        flags: PaperFlags { owned: true, venue: Some("Journal of Synthetic Tests".into()), ..Default::default() },
        first_seen_ms: 100,
        last_seen_ms: 100,
    })
    .unwrap();
    lib.upsert_source(&SourceRecord {
        id: "vault-1".into(), kind: SourceKind::KandinskyVault, root: "/tmp/synth-vault".into(),
        settings: json!({}), added_ms: 100, last_sync_ms: Some(100), state: "synced".into(),
    })
    .unwrap();
    lib.upsert_source_item(&SourceItemRecord {
        source_id: "vault-1".into(), external_id: citekey.into(), key: Some(paper.key.clone()),
        citekey: Some(citekey.into()), ids: LibraryIds::from_paper_ids(&paper.ids),
        version: paper.version.clone(), title: "Synthetic Boundary Sums on Finite Chains".into(),
        authors: vec!["Voss, Elena".into()], year: Some(2024), item_kind: "journalArticle".into(),
        collections: vec![], tags: vec![], files: vec![], state: SourceItemState::Present, updated_ms: 100,
    })
    .unwrap();
}
