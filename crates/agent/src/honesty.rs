//! Deterministic honesty linter and campaign phase inference over the hash-chained ledger.

use ledger::{Event, Ledger};
use research_tools::{ProofDag, ProofStatus};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HonestyFinding {
    pub severity: String,
    pub claim: String,
    pub reason: String,
    pub suggestion: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HonestyReport {
    pub passed: bool,
    pub has_sympy_ok: bool,
    pub has_lean_verified: bool,
    pub has_verifier_accepted: bool,
    pub findings: Vec<HonestyFinding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CampaignPhase {
    Ingest,
    Explore,
    Compute,
    Synthesize,
    Verify,
    Revise,
    Complete,
}

impl CampaignPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ingest => "INGEST",
            Self::Explore => "EXPLORE",
            Self::Compute => "COMPUTE",
            Self::Synthesize => "SYNTHESIZE",
            Self::Verify => "VERIFY",
            Self::Revise => "REVISE",
            Self::Complete => "COMPLETE",
        }
    }
}

const LEAN_PHRASES: &[&str] = &[
    "lean verified",
    "lean 4 verified",
    "formally verified in lean",
    "checked by lean",
];

const SYMPY_PHRASES: &[&str] = &[
    "sympy confirmed",
    "verified in sympy",
    "verified symbolically with sympy",
    "checked with sympy",
];

const PROOF_PHRASES: &[&str] = &[
    "we have rigorously proved",
    "rigorously proven",
    "the experiment proves",
    "proves unconditionally",
    "q.e.d.",
];

pub(crate) fn recorded_value(value: &Value, ledger: Option<&Ledger>) -> Option<Value> {
    if let Some(text) = value.get("inline").and_then(Value::as_str).or_else(|| value.as_str()) {
        return serde_json::from_str(text).ok();
    }
    if let Some(hash) = value.get("blob").and_then(Value::as_str) {
        let bytes = ledger?.get_blob(hash).ok()?;
        return serde_json::from_slice(&bytes).ok();
    }
    value.is_object().then(|| value.clone())
}

pub(crate) struct RecordedToolCheck<'a> {
    pub(crate) call_id: &'a str,
    pub(crate) tool: &'a str,
    pub(crate) args: &'a Value,
    pub(crate) result: Value,
    pub(crate) seq: u64,
}

impl RecordedToolCheck<'_> {
    pub(crate) fn matches_ref(&self, reference: &str) -> bool {
        research_tools::proof_dag::tool_reference_matches(reference, self.call_id, self.seq)
    }

    fn supports_claim(&self) -> bool {
        match self.tool {
            "sympy_eval" => {
                self.result.get("value").is_some()
                    && self.result["error"].is_null()
                    && self.result["validation_truncated"] != true
                    && self.result.get("validation").is_none_or(|validation| {
                        validation["kind"] == "symbolic_identity"
                            && validation["status"] == "symbolic_agreement"
                            && validation["domain"]["status"] != "restricted"
                    })
            }
            "lean_check" => {
                self.result["status"].as_str() == Some("verified")
                    && self.result["ok"].as_bool() != Some(false)
                    && self.result["axioms"].as_array().is_none_or(Vec::is_empty)
            }
            _ => false,
        }
    }
}

/// Match typed check results to the actual session tool request, including blob-backed records.
pub(crate) fn recorded_tool_checks<'a>(
    events: &'a [Event],
    ledger: Option<&Ledger>,
    tools: &[&str],
) -> Vec<RecordedToolCheck<'a>> {
    let mut requests = HashMap::new();
    let mut checks = Vec::new();
    for ev in events {
        let payload = &ev.payload;
        let Some(call_id) = payload["call_id"].as_str() else { continue };
        match ev.kind.as_str() {
            "tool.request" => {
                if let Some(tool) = payload["tool"].as_str().filter(|tool| tools.contains(tool)) {
                    requests.insert(call_id, (tool, &payload["args"]));
                }
            }
            "tool.result" if payload["ok"].as_bool() == Some(true) => {
                if let Some((tool, args)) = requests.remove(call_id) {
                    if let Some(result) = recorded_value(&payload["result"], ledger) {
                        checks.push(RecordedToolCheck { call_id, tool, args, result, seq: ev.seq });
                    }
                }
            }
            _ => {}
        }
    }
    checks
}

fn find_claim(candidate_text: &str, lower_ascii: &str, phrases: &[&str]) -> Option<String> {
    for &phrase in phrases {
        if let Some(idx) = lower_ascii.find(phrase) {
            let matched = candidate_text.get(idx..idx + phrase.len()).unwrap_or(phrase);
            return Some(matched.to_string());
        }
    }
    None
}

/// Audit candidate prose against evidence available in the supplied events.
pub fn lint_honesty(candidate_text: &str, dag: &ProofDag, events: &[Event]) -> HonestyReport {
    lint_with_results(candidate_text, dag, events, None)
}

pub(crate) fn lint_honesty_with_ledger(
    candidate_text: &str,
    dag: &ProofDag,
    events: &[Event],
    ledger: &Ledger,
) -> HonestyReport {
    lint_with_results(candidate_text, dag, events, Some(ledger))
}

fn lint_with_results(
    candidate_text: &str,
    dag: &ProofDag,
    events: &[Event],
    ledger: Option<&Ledger>,
) -> HonestyReport {
    let boundary = events.iter().rev().find(|ev| {
        matches!(ev.kind.as_str(), "session.start" | "session.resume" | "autonomous.feedback")
    });
    let candidate_seq = boundary.map_or(0, |ev| ev.seq);
    let active: HashMap<_, _> = dag.entries.iter()
        .filter(|e| !matches!(e.status, ProofStatus::Superseded | ProofStatus::Rejected))
        .map(|e| (e.entry_id.as_str(), e))
        .collect();
    let candidate = dag.entries.iter().rev().find(|e| {
        e.seq > candidate_seq && e.entry_type == "final_answer" && active.contains_key(e.entry_id.as_str())
    });
    let mut pending: Vec<_> = candidate.into_iter().collect();
    let referenced_words = candidate_text
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_');
    pending.extend(referenced_words.filter_map(|word| active.get(word).copied()));
    let mut visited = HashSet::new();
    let mut references = Vec::new();
    while let Some(entry) = pending.pop() {
        if visited.insert(entry.entry_id.as_str()) {
            references.extend(entry.tool_call_ref.as_deref().map(|reference| (reference, entry.seq)));
            pending.extend(entry.depends_on.iter().filter_map(|id| active.get(id.as_str()).copied()));
        }
    }
    let checks = recorded_tool_checks(events, ledger, &["sympy_eval", "lean_check"]);
    let supports = |tool| checks.iter().any(|check| {
        check.tool == tool && check.supports_claim()
            && (boundary.is_none() || references.iter().any(|(reference, seq)| {
                check.seq < *seq && check.matches_ref(reference)
            }))
    });
    let has_sympy_ok = supports("sympy_eval");
    let has_lean_verified = supports("lean_check");
    let has_verifier_accepted = dag.latest_full_verification_accepted && (
        boundary.is_none() || candidate.is_some_and(|entry| {
            let answer_hash = ledger::sha256_hex(candidate_text.as_bytes());
            let Some(verification) = &entry.verification else { return false };
            verification["kind"].as_str() == Some("full")
                && research_tools::effective_verification_verdict(verification) == Some("accepted")
                && verification["answer_sha256"].as_str() == Some(answer_hash.as_str())
        })
    );
    let lower = candidate_text.to_ascii_lowercase();
    let mut findings = Vec::new();

    if !has_lean_verified {
        if let Some(claim) = find_claim(candidate_text, &lower, LEAN_PHRASES) {
            findings.push(HonestyFinding {
                severity: "error".to_string(),
                claim,
                reason: "prose claims Lean verification without a matching axiom-free `lean_check` result linked to the candidate proof".to_string(),
                suggestion: "Record the verified Lean check's `tool_call_ref` on a relevant proof entry and cite that entry, or qualify the claim.".to_string(),
            });
        }
    }

    if !has_sympy_ok {
        if let Some(claim) = find_claim(candidate_text, &lower, SYMPY_PHRASES) {
            findings.push(HonestyFinding {
                severity: "error".to_string(),
                claim,
                reason: "prose claims SymPy verification without a successful `sympy_eval` result linked to the candidate proof".to_string(),
                suggestion: "Record the successful calculation's `tool_call_ref` on a relevant proof entry and cite that entry, or qualify the claim.".to_string(),
            });
        }
    }

    if !has_verifier_accepted && !has_lean_verified {
        if let Some(claim) = find_claim(candidate_text, &lower, PROOF_PHRASES) {
            findings.push(HonestyFinding {
                severity: "warning".to_string(),
                claim,
                reason: "evidence != proof: prose asserts definitive proof without an accepted Verifier or Lean 4 check".to_string(),
                suggestion: "Qualify the claim to match recorded evidence until full Verifier acceptance or Lean 4 verification succeeds.".to_string(),
            });
        }
    }

    HonestyReport {
        passed: findings.is_empty(),
        has_sympy_ok,
        has_lean_verified,
        has_verifier_accepted,
        findings,
    }
}

/// Infer the active campaign phase deterministically from the latest significant session events.
pub fn infer_campaign_phase(events: &[Event]) -> CampaignPhase {
    for ev in events.iter().rev() {
        match ev.kind.as_str() {
            "session.end" => return CampaignPhase::Complete,
            "autonomous.feedback" | "strategist.consult" => return CampaignPhase::Revise,
            "proof.verification_attempt" | "proof.verification" => return CampaignPhase::Verify,
            "explore.start" | "explore.branch" | "explore.end" => return CampaignPhase::Explore,
            "delegate.start" => match ev.payload.get("role").and_then(Value::as_str) {
                Some("compute_worker" | "lean_worker") => return CampaignPhase::Compute,
                Some("literature_worker") => return CampaignPhase::Ingest,
                _ => {}
            },
            "tool.request" => match ev.payload.get("tool").and_then(Value::as_str) {
                Some("sympy_eval" | "lean_check") => return CampaignPhase::Compute,
                Some("arxiv_search" | "literature_search") => return CampaignPhase::Ingest,
                _ => {}
            },
            "source.ingested" => return CampaignPhase::Ingest,
            "session.start" | "session.resume" => return CampaignPhase::Synthesize,
            _ => {}
        }
    }
    CampaignPhase::Synthesize
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(seq: u64, kind: &str, payload: Value) -> Event {
        Event {
            session: "s-hon".into(),
            seq,
            ts_ms: seq as i64 * 10,
            kind: kind.into(),
            schema_version: 1,
            payload,
            prev_hash: "0".repeat(64),
            event_hash: "1".repeat(64),
        }
    }

    #[test]
    fn lint_flags_unbacked_lean_sympy_and_proof_claims_and_passes_when_backed() {
        let dag = ProofDag::default();
        let bad = "This identity is SymPy confirmed and Lean 4 verified. We have rigorously proved it. Q.E.D.";
        let rep = lint_honesty(bad, &dag, &[]);
        assert!(!rep.passed);
        assert_eq!(rep.findings.len(), 3);
        assert_eq!(rep.findings.iter().filter(|f| f.severity == "error").count(), 2);
        assert_eq!(rep.findings.iter().filter(|f| f.severity == "warning").count(), 1);
        assert!(rep.findings[2].reason.contains("evidence != proof"));

        // Both transport and evaluation failures leave the claims unbacked.
        let unbacked = vec![
            ev(1, "tool.request", json!({"call_id": "c1", "tool": "sympy_eval"})),
            ev(2, "tool.result", json!({"call_id": "c1", "ok": false, "error": {"inline": "boom"}})),
            ev(3, "tool.request", json!({"call_id": "c2", "tool": "lean_check"})),
            ev(4, "tool.result", json!({"call_id": "c2", "ok": true, "result": {
                "inline": json!({"status": "sorry_found", "diagnostics": [{"status": "verified"}]}).to_string()
            }})),
            ev(5, "tool.request", json!({"call_id": "c3", "tool": "sympy_eval"})),
            ev(6, "tool.result", json!({"call_id": "c3", "ok": true, "result": {
                "inline": json!({"value": null, "error": {"type": "ValueError", "message": "math error"}}).to_string()
            }})),
            ev(7, "tool.request", json!({"call_id": "c4", "tool": "sympy_eval"})),
            ev(8, "tool.result", json!({"call_id": "c4", "ok": true, "result": {
                "value": "counterexample",
                "validation": {"kind": "symbolic_identity", "status": "counterexample", "schema_version": 1}
            }})),
            ev(9, "tool.request", json!({"call_id": "c5", "tool": "sympy_eval"})),
            ev(10, "tool.result", json!({"call_id": "c5", "ok": true, "result": {
                "value": "report",
                "validation": {"kind": "symbolic_identity", "status": "symbolic_agreement", "domain": {"status": "restricted"}, "schema_version": 1}
            }})),
        ];
        let rep_fail = lint_honesty("Checked with SymPy and checked by Lean.", &dag, &unbacked);
        assert_eq!(rep_fail.findings.len(), 2);
        assert!(!rep_fail.has_sympy_ok && !rep_fail.has_lean_verified);

        // Passing sympy_eval + verified lean_check clears all errors and proof warnings.
        let backed = vec![
            ev(1, "tool.request", json!({"call_id": "s1", "tool": "sympy_eval"})),
            ev(2, "tool.result", json!({"call_id": "s1", "ok": true, "result": {"inline": "{\"value\":\"0\"}"}})),
            ev(3, "tool.request", json!({"call_id": "l1", "tool": "lean_check"})),
            ev(4, "tool.result", json!({"call_id": "l1", "ok": true, "result": {"inline": "{\"status\":\"verified\"}"}})),
        ];
        let rep_ok = lint_honesty(bad, &dag, &backed);
        assert!(rep_ok.passed, "{:?}", rep_ok.findings);
        assert!(rep_ok.has_sympy_ok && rep_ok.has_lean_verified);

        // Accepted full verifier clears absolute-proof warning without Lean.
        let dag_acc = ProofDag { latest_full_verification_accepted: true, ..Default::default() };
        let rep_ver = lint_honesty("Rigorously proven via our derivation.", &dag_acc, &[]);
        assert!(rep_ver.passed && rep_ver.has_verifier_accepted);
    }

    #[test]
    fn lint_reads_verified_tool_results_stored_as_blobs() {
        let temp = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(temp.path()).unwrap();
        let sympy = json!({"value": null, "stdout": "x".repeat(5000)});
        let lean = json!({"status": "verified", "stdout": "x".repeat(5000)});
        let events: Vec<Event> = [("sympy_eval", sympy), ("lean_check", lean)]
            .into_iter()
            .enumerate()
            .flat_map(|(index, (tool, result))| {
                let result = serde_json::to_vec(&result).unwrap();
                let blob = ledger.put_blob(&result).unwrap();
                let seq = index as u64 * 2 + 1;
                [
                    ev(seq, "tool.request", json!({"call_id": tool, "tool": tool})),
                    ev(seq + 1, "tool.result", json!({"call_id": tool, "ok": true, "result": {
                        "blob": blob, "bytes": result.len()
                    }})),
                ]
            })
            .collect();
        let report = lint_honesty_with_ledger(
            "Checked with SymPy and checked by Lean.",
            &ProofDag::default(),
            &events,
            &ledger,
        );
        assert!(report.has_sympy_ok && report.has_lean_verified);
        assert!(report.passed);
    }

    #[test]
    fn prior_or_unlinked_checks_do_not_support_a_new_claim() {
        let mut events = vec![
            ev(1, "session.start", json!({ "prompt": "Prove A" })),
            ev(2, "tool.request", json!({ "call_id": "prior-lean", "tool": "lean_check" })),
            ev(3, "tool.result", json!({ "call_id": "prior-lean", "ok": true, "result": { "status": "verified" } })),
            ev(4, "proof.entry", json!({
                "entry_id": "e-1", "entry_type": "verification", "details": "Proof of A",
                "tool_call_ref": "prior-lean",
            })),
            ev(5, "session.resume", json!({ "prompt": "Prove B" })),
            ev(6, "tool.request", json!({ "call_id": "same-turn-lean", "tool": "lean_check" })),
            ev(7, "tool.result", json!({ "call_id": "same-turn-lean", "ok": true, "result": { "status": "verified" } })),
            ev(8, "proof.entry", json!({
                "entry_id": "e-2", "entry_type": "verification", "details": "Unrelated proof of C",
                "tool_call_ref": "same-turn-lean",
            })),
            ev(9, "proof.entry", json!({
                "entry_id": "e-3", "entry_type": "final_answer", "details": "B was checked by Lean."
            })),
        ];
        let dag = ProofDag { entries: crate::guardrails::materialize_proof_entries(&events), ..Default::default() };
        assert!(!lint_honesty("B was checked by Lean.", &dag, &events).passed);
        assert!(!lint_honesty("B was checked by Lean (see e-10).", &dag, &events).passed);

        events.push(ev(10, "proof.entry", json!({
            "entry_id": "e-4", "entry_type": "final_answer", "details": "A was checked by Lean.",
            "depends_on": ["e-1"]
        })));
        let dag = ProofDag { entries: crate::guardrails::materialize_proof_entries(&events), ..Default::default() };
        assert!(lint_honesty("A was checked by Lean.", &dag, &events).passed);
    }

    #[test]
    fn infer_campaign_phase_tracks_latest_significant_events() {
        assert_eq!(infer_campaign_phase(&[]), CampaignPhase::Synthesize);
        assert_eq!(infer_campaign_phase(&[ev(1, "source.ingested", json!({}))]), CampaignPhase::Ingest);
        assert_eq!(infer_campaign_phase(&[ev(1, "tool.request", json!({"tool": "arxiv_search"}))]), CampaignPhase::Ingest);
        assert_eq!(infer_campaign_phase(&[ev(1, "delegate.start", json!({"role": "literature_worker"}))]), CampaignPhase::Ingest);
        assert_eq!(infer_campaign_phase(&[ev(1, "delegate.start", json!({"role": "lean_worker"}))]), CampaignPhase::Compute);
        assert_eq!(infer_campaign_phase(&[ev(1, "tool.request", json!({"tool": "sympy_eval"}))]), CampaignPhase::Compute);
        assert_eq!(infer_campaign_phase(&[ev(1, "explore.start", json!({}))]), CampaignPhase::Explore);
        assert_eq!(infer_campaign_phase(&[ev(1, "proof.verification_attempt", json!({}))]), CampaignPhase::Verify);
        assert_eq!(infer_campaign_phase(&[ev(1, "strategist.consult", json!({}))]), CampaignPhase::Revise);
        assert_eq!(infer_campaign_phase(&[ev(1, "autonomous.feedback", json!({}))]), CampaignPhase::Revise);
        assert_eq!(infer_campaign_phase(&[ev(1, "session.end", json!({}))]), CampaignPhase::Complete);
        assert_eq!(
            infer_campaign_phase(&[ev(1, "session.end", json!({})), ev(2, "session.resume", json!({}))]),
            CampaignPhase::Synthesize
        );
    }
}
