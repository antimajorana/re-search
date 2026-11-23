//! Deterministic honesty linter and campaign phase inference over the hash-chained ledger.

use ledger::Event;
use research_tools::ProofDag;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

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

fn lean_result_is_verified(payload: &Value) -> bool {
    let Some(res) = payload.get("result") else {
        return false;
    };
    if res.get("status").and_then(Value::as_str) == Some("verified") {
        return true;
    }
    let check_str = |s: &str| -> bool {
        if let Ok(parsed) = serde_json::from_str::<Value>(s) {
            if parsed.get("status").and_then(Value::as_str) == Some("verified") {
                return true;
            }
        }
        s.contains("\"status\":\"verified\"") || s.contains("\"status\": \"verified\"")
    };
    if let Some(inline) = res.get("inline").and_then(Value::as_str) {
        if check_str(inline) {
            return true;
        }
    }
    if let Some(raw) = res.as_str() {
        if check_str(raw) {
            return true;
        }
    }
    false
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

/// Audit candidate prose against recorded `sympy_eval`, `lean_check`, and Verifier evidence.
pub fn lint_honesty(candidate_text: &str, dag: &ProofDag, events: &[Event]) -> HonestyReport {
    let mut tool_by_call_id: HashMap<&str, &str> = HashMap::new();
    let mut has_sympy_ok = false;
    let mut has_lean_verified = false;

    for ev in events {
        match ev.kind.as_str() {
            "tool.request" => {
                if let (Some(cid), Some(tool)) = (
                    ev.payload.get("call_id").and_then(Value::as_str),
                    ev.payload.get("tool").and_then(Value::as_str),
                ) {
                    tool_by_call_id.insert(cid, tool);
                }
            }
            "tool.result" => {
                if ev.payload.get("ok").and_then(Value::as_bool) != Some(true) {
                    continue;
                }
                let Some(cid) = ev.payload.get("call_id").and_then(Value::as_str) else {
                    continue;
                };
                match tool_by_call_id.get(cid).copied() {
                    Some("sympy_eval") => has_sympy_ok = true,
                    Some("lean_check") if lean_result_is_verified(&ev.payload) => {
                        has_lean_verified = true;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    let has_verifier_accepted = dag.latest_full_verification_accepted;
    let lower = candidate_text.to_ascii_lowercase();
    let mut findings = Vec::new();

    if !has_lean_verified {
        if let Some(claim) = find_claim(candidate_text, &lower, LEAN_PHRASES) {
            findings.push(HonestyFinding {
                severity: "error".to_string(),
                claim,
                reason: "prose claims Lean verification without a passing `lean_check` tool call with status \"verified\"".to_string(),
                suggestion: "Run `lean_check` via `delegate_lean` and verify status \"verified\", or remove the Lean verification claim.".to_string(),
            });
        }
    }

    if !has_sympy_ok {
        if let Some(claim) = find_claim(candidate_text, &lower, SYMPY_PHRASES) {
            findings.push(HonestyFinding {
                severity: "error".to_string(),
                claim,
                reason: "prose claims SymPy verification without a succeeding `sympy_eval` tool call".to_string(),
                suggestion: "Run `sympy_eval` (directly or via `delegate_compute`) to verify the calculation, or remove the SymPy claim.".to_string(),
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

        // Failed tool.result or non-verified lean status must NOT satisfy the checks.
        let unbacked = vec![
            ev(1, "tool.request", json!({"call_id": "c1", "tool": "sympy_eval"})),
            ev(2, "tool.result", json!({"call_id": "c1", "ok": false, "error": {"inline": "boom"}})),
            ev(3, "tool.request", json!({"call_id": "c2", "tool": "lean_check"})),
            ev(4, "tool.result", json!({"call_id": "c2", "ok": true, "result": {"inline": "{\"status\":\"sorry_found\"}"}})),
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
