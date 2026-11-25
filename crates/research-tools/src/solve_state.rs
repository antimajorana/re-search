//! Incremental state fold from ledger events to [`SolveState`] (package S2).
//!
//! Pure code: no I/O, no ledger writes, no tool definitions. Folding a sequence
//! one event at a time, all at once, or continuing from any prefix produces the
//! exact same [`SolveState`].

use crate::proof_dag::{effective_verification_verdict, tool_reference_matches};
use crate::solve_model::{
    event_kinds, AckKind, CheckpointReason, Claim, CriterionOutcome, EvidenceQuoteRecord,
    HandleRef, ModelCallsBreakdown, PauseReason, QuoteRef, ReceiptProcess, RestsOnSummary,
    ReviewIndependence, ReviewVerdict, SolveAck, SolveArtifact, SolveCheck, SolveCheckpoint,
    SolveDone, SolveDoneStatus, SolveEntry, SolveExpr, SolvePaused, SolvePlan, SolveReceipt,
    SolveReview, SolveRunStatus, SolveSetup, StepEntryType, StepOrigin, TranscriptionMeta,
};
use crate::solve_status::{badge_for_step, compute_all_tiers, StepBadge, StepTier};
use ledger::Event;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// A `tool.request` awaiting its matching `tool.result`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InFlightTool {
    pub tool: String,
    pub request_seq: u64,
}

/// Derived in-memory Solve session state (`solve.md` 4.2). Never stored in the ledger.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SolveState {
    pub started: bool,
    pub setup: Option<SolveSetup>,
    pub plan: Option<SolvePlan>,
    pub entries: Vec<SolveEntry>,
    pub entry_index: BTreeMap<String, usize>,
    pub handles: BTreeMap<HandleRef, SolveExpr>,
    pub checks: Vec<SolveCheck>,
    pub reviews: Vec<SolveReview>,
    pub acks: Vec<SolveAck>,
    pub quotes: BTreeMap<u64, EvidenceQuoteRecord>,
    pub receipts: BTreeMap<String, SolveReceipt>,
    pub in_flight_tools: BTreeMap<String, InFlightTool>,
    pub tiers: BTreeMap<String, StepTier>,
    pub pending_runs: Vec<String>,
    pub gate_bounces: u32,
    pub run_status: SolveRunStatus,
    pub last_paused: Option<SolvePaused>,
    pub last_checkpoint: Option<SolveCheckpoint>,
    pub done: Option<SolveDone>,
    pub artifacts: Vec<SolveArtifact>,
    pub upto_seq: u64,
}

impl SolveState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_events(events: &[Event]) -> Self {
        let mut state = Self::new();
        state.apply_slice(events);
        state
    }

    pub fn apply_slice(&mut self, events: &[Event]) {
        for ev in events {
            self.apply(ev);
        }
    }

    pub fn entry(&self, entry_id: &str) -> Option<&SolveEntry> {
        self.entry_index.get(entry_id).and_then(|&idx| self.entries.get(idx))
    }

    pub fn tier(&self, entry_id: &str) -> Option<&StepTier> {
        self.tiers.get(entry_id)
    }

    pub fn badge(&self, entry_id: &str) -> Option<StepBadge> {
        badge_for_step(self, entry_id)
    }

    pub fn active_entries(&self) -> Vec<&SolveEntry> {
        self.entries.iter().filter(|e| e.superseded_by.is_none()).collect()
    }

    /// Recompute `self.tiers` from the current folded records.
    pub fn recompute_tiers(&mut self) {
        self.tiers = compute_all_tiers(self);
    }

    /// Apply one ledger event incrementally.
    /// Events with a positive `seq <= self.upto_seq` are ignored so re-applying a prefix is a no-op.
    pub fn apply(&mut self, ev: &Event) {
        if ev.seq > 0 && ev.seq <= self.upto_seq {
            return;
        }
        if ev.seq > self.upto_seq {
            self.upto_seq = ev.seq;
        }

        let mut tiers_dirty = false;
        let p = &ev.payload;

        match ev.kind.as_str() {
            event_kinds::SOLVE_STARTED => {
                self.started = true;
                if self.run_status == SolveRunStatus::Idle {
                    self.run_status = SolveRunStatus::Running;
                }
            }
            event_kinds::SOLVE_SETUP => {
                if let Ok(setup) = serde_json::from_value::<SolveSetup>(p.clone()) {
                    self.setup = Some(setup);
                    tiers_dirty = true;
                }
            }
            event_kinds::SOLVE_PLAN => {
                if let Ok(plan) = serde_json::from_value::<SolvePlan>(p.clone()) {
                    self.plan = Some(plan);
                }
            }
            event_kinds::PROOF_ENTRY => {
                if let Some(entry) = parse_proof_entry(ev.seq, p) {
                    self.retire_cited_runs(&entry);
                    if let Some(&idx) = self.entry_index.get(&entry.entry_id) {
                        self.entries[idx] = entry;
                    } else {
                        let idx = self.entries.len();
                        self.entry_index.insert(entry.entry_id.clone(), idx);
                        self.entries.push(entry);
                    }
                    tiers_dirty = true;
                }
            }
            event_kinds::PROOF_SUPERSEDE => {
                if let (Some(eid), Some(repl)) = (
                    p.get("entry_id").and_then(Value::as_str),
                    p.get("replaced_by").and_then(Value::as_str),
                ) {
                    if let Some(&idx) = self.entry_index.get(eid) {
                        self.entries[idx].superseded_by = Some(repl.to_string());
                        tiers_dirty = true;
                    }
                }
            }
            event_kinds::SOLVE_EXPR => {
                if let Ok(expr) = serde_json::from_value::<SolveExpr>(p.clone()) {
                    if let Ok(handle) = expr.handle() {
                        self.handles.insert(handle, expr);
                        tiers_dirty = true;
                    }
                }
            }
            event_kinds::SOLVE_CHECK => {
                if let Ok(check) = serde_json::from_value::<SolveCheck>(p.clone()) {
                    self.checks.push(check);
                    tiers_dirty = true;
                }
            }
            event_kinds::PROOF_VERIFICATION => {
                if let Some(review) = parse_proof_verification(ev.seq, p) {
                    self.reviews.push(review);
                    tiers_dirty = true;
                }
            }
            event_kinds::SOLVE_ACK => {
                if let Some(ack) = parse_solve_ack(ev.seq, p) {
                    self.acks.push(ack);
                    tiers_dirty = true;
                }
            }
            event_kinds::EVIDENCE_QUOTE => {
                if let Some(rec) = parse_evidence_quote(ev.seq, p) {
                    self.quotes.insert(ev.seq, rec);
                    tiers_dirty = true;
                }
            }
            event_kinds::TOOL_REQUEST => {
                if let (Some(call_id), Some(tool)) = (
                    p.get("call_id").and_then(Value::as_str),
                    p.get("tool").and_then(Value::as_str),
                ) {
                    self.in_flight_tools.insert(
                        call_id.to_string(),
                        InFlightTool { tool: tool.to_string(), request_seq: ev.seq },
                    );
                }
            }
            event_kinds::TOOL_RESULT => {
                if let Some((receipt, sympy_succeeded)) = parse_tool_result(ev.seq, p, &mut self.in_flight_tools) {
                    let call_id = receipt.call_id.clone();
                    let ok = receipt.ok;
                    let is_sympy = receipt.tool == "sympy_eval";
                    self.receipts.insert(call_id.clone(), receipt);
                    if ok {
                        self.gate_bounces = 0;
                    }
                    if is_sympy && sympy_succeeded && !self.pending_runs.contains(&call_id) {
                        self.pending_runs.push(call_id);
                    }
                    tiers_dirty = true;
                }
            }
            event_kinds::NOTE => {
                if let Some(text) = p.get("text").and_then(Value::as_str) {
                    self.apply_discard_note(text);
                }
            }
            event_kinds::GUARDRAIL_BLOCKED => {
                self.gate_bounces = self.gate_bounces.saturating_add(1);
            }
            event_kinds::SOLVE_PAUSED => {
                if let Some(paused) = parse_solve_paused(ev.seq, p) {
                    self.run_status = SolveRunStatus::Paused(paused.reason);
                    self.last_paused = Some(paused);
                    self.gate_bounces = 0;
                }
            }
            event_kinds::SOLVE_CHECKPOINT => {
                if let Some(cp) = parse_solve_checkpoint(ev.seq, p) {
                    self.last_checkpoint = Some(cp);
                    self.gate_bounces = 0;
                }
            }
            event_kinds::SESSION_RESUME => {
                self.gate_bounces = 0;
                if self.started {
                    self.run_status = SolveRunStatus::Running;
                }
            }
            event_kinds::SOLVE_DONE => {
                if let Some(done) = parse_solve_done(ev.seq, p) {
                    self.run_status = SolveRunStatus::Done(done.status);
                    self.done = Some(done);
                }
            }
            event_kinds::SOLVE_ARTIFACT => {
                if let Ok(artifact) = serde_json::from_value::<SolveArtifact>(p.clone()) {
                    self.artifacts.push(artifact);
                }
            }
            _ => {}
        }

        if tiers_dirty {
            self.recompute_tiers();
        }
    }

    fn retire_cited_runs(&mut self, entry: &SolveEntry) {
        let refs: Vec<&str> = entry
            .tool_call_ref
            .iter()
            .chain(entry.tool_call_refs.iter())
            .map(String::as_str)
            .collect();
        if refs.is_empty() || self.pending_runs.is_empty() {
            return;
        }
        let receipts = &self.receipts;
        self.pending_runs.retain(|cid| {
            let seq = receipts.get(cid).map_or(0, |r| r.result_seq);
            !refs.iter().any(|r| tool_reference_matches(r, cid, seq))
        });
    }

    fn apply_discard_note(&mut self, text: &str) {
        let trimmed = text.trim();
        let Some(rest) = trimmed.strip_prefix("discard:") else {
            return;
        };
        if self.pending_runs.is_empty() {
            return;
        }
        let receipts = &self.receipts;
        let tokens: Vec<&str> = rest
            .split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '(' | ')'))
            .filter(|t| !t.is_empty())
            .collect();
        let mut matched_any = false;
        let kept: Vec<String> = self
            .pending_runs
            .iter()
            .filter(|cid| {
                let seq = receipts.get(*cid).map_or(0, |r| r.result_seq);
                let hit = tokens.iter().any(|tok| tool_reference_matches(tok, cid, seq));
                if hit {
                    matched_any = true;
                }
                !hit
            })
            .cloned()
            .collect();
        if matched_any {
            self.pending_runs = kept;
        } else {
            // A blanket "discard: <reason>" without naming a call_id discards all pending runs.
            self.pending_runs.clear();
        }
    }
}

fn string_vec(p: &Value, key: &str) -> Vec<String> {
    p.get(key)
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

fn parse_proof_entry(seq: u64, p: &Value) -> Option<SolveEntry> {
    let entry_id = p.get("entry_id").and_then(Value::as_str)?.trim().to_string();
    if entry_id.is_empty() {
        return None;
    }
    let entry_type: StepEntryType = p
        .get("entry_type")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or(StepEntryType::Derivation);
    let claims: Vec<Claim> = p
        .get("claims")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let origin: Option<StepOrigin> =
        p.get("origin").and_then(|v| serde_json::from_value(v.clone()).ok());
    let source_ref: Option<QuoteRef> =
        p.get("source_ref").and_then(|v| serde_json::from_value(v.clone()).ok());
    let transcription: Option<TranscriptionMeta> =
        p.get("transcription").and_then(|v| serde_json::from_value(v.clone()).ok());

    Some(SolveEntry {
        entry_id,
        seq,
        entry_type,
        summary: p.get("summary").and_then(Value::as_str).unwrap_or_default().to_string(),
        details: p.get("details").and_then(Value::as_str).unwrap_or_default().to_string(),
        latex: p.get("latex").and_then(Value::as_str).map(str::to_string),
        depends_on: string_vec(p, "depends_on"),
        derived_depends_on: string_vec(p, "derived_depends_on"),
        tool_call_ref: p.get("tool_call_ref").and_then(Value::as_str).map(str::to_string),
        tool_call_refs: string_vec(p, "tool_call_refs"),
        claims,
        claim_sha256: p.get("claim_sha256").and_then(Value::as_str).map(str::to_string),
        entry_sha256: p.get("entry_sha256").and_then(Value::as_str).map(str::to_string),
        origin,
        source_ref,
        transcription,
        superseded_by: None,
    })
}

fn parse_proof_verification(seq: u64, p: &Value) -> Option<SolveReview> {
    let target_entry_id = p.get("target_entry_id").and_then(Value::as_str)?.to_string();
    let verdict = match effective_verification_verdict(p) {
        Some("accepted") => ReviewVerdict::Accepted,
        Some("rejected") => ReviewVerdict::Rejected,
        Some("incomplete") => ReviewVerdict::Incomplete,
        _ => ReviewVerdict::Pending,
    };
    let independence: Option<ReviewIndependence> =
        p.get("independence").and_then(|v| serde_json::from_value(v.clone()).ok());
    let issues = p.get("issues").and_then(Value::as_array).cloned().unwrap_or_default();
    Some(SolveReview {
        seq,
        target_entry_id,
        entry_sha256: p.get("entry_sha256").and_then(Value::as_str).map(str::to_string),
        verdict,
        independence,
        verifier_model: p.get("verifier_model").and_then(Value::as_str).map(str::to_string),
        checks_cited: string_vec(p, "checks_cited"),
        model_calls_used: p.get("model_calls_used").and_then(Value::as_u64).map(|n| n as u32),
        issues,
        counterexample: p.get("counterexample").and_then(Value::as_str).map(str::to_string),
    })
}

fn parse_solve_ack(seq: u64, p: &Value) -> Option<SolveAck> {
    let entry_id = p.get("entry_id").and_then(Value::as_str)?.to_string();
    let kind: AckKind = serde_json::from_value(p.get("kind")?.clone()).ok()?;
    let entry_sha256 = p.get("entry_sha256").and_then(Value::as_str)?.to_string();
    Some(SolveAck { seq, entry_id, kind, entry_sha256 })
}

fn parse_evidence_quote(seq: u64, p: &Value) -> Option<EvidenceQuoteRecord> {
    Some(EvidenceQuoteRecord {
        seq,
        source_id: p.get("source_id").and_then(Value::as_str)?.to_string(),
        path: p.get("path").and_then(Value::as_str)?.to_string(),
        start_line: p.get("start_line").and_then(Value::as_u64)? as usize,
        end_line: p.get("end_line").and_then(Value::as_u64)? as usize,
        file_sha256: p.get("file_sha256").and_then(Value::as_str)?.to_string(),
        quote_sha256: p.get("quote_sha256").and_then(Value::as_str)?.to_string(),
    })
}

fn parse_tool_result(
    seq: u64,
    p: &Value,
    in_flight: &mut BTreeMap<String, InFlightTool>,
) -> Option<(SolveReceipt, bool)> {
    let call_id = p.get("call_id").and_then(Value::as_str)?.to_string();
    let ok = p.get("ok").and_then(Value::as_bool).unwrap_or(false);
    let inflight = in_flight.remove(&call_id);
    let tool = inflight
        .as_ref()
        .map(|f| f.tool.clone())
        .or_else(|| p.get("tool").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default();
    let request_seq = inflight.map(|f| f.request_seq);

    let result = p.get("result");
    let result_blob = result
        .and_then(|r| r.get("blob"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let decoded: Option<Value> = match result {
        Some(r) if r.get("inline").and_then(Value::as_str).is_some() => {
            r.get("inline").and_then(Value::as_str).and_then(|s| serde_json::from_str(s).ok())
        }
        Some(_) if result_blob.is_some() => None,
        Some(r) if r.is_object() => Some(r.clone()),
        _ => None,
    };

    let meta = decoded.as_ref().unwrap_or(p);
    let code_sha256 = meta
        .get("code_sha256")
        .or_else(|| p.get("code_sha256"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let wall_ms = meta.get("wall_ms").or_else(|| p.get("wall_ms")).and_then(Value::as_u64);
    let process: Option<ReceiptProcess> = meta
        .get("process")
        .or_else(|| p.get("process"))
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    let exprs_out = string_vec(meta, "exprs_out");
    let env = meta.get("env").or_else(|| p.get("env")).cloned();

    let sympy_succeeded = ok
        && match &decoded {
            Some(v) => v.get("value").is_some() && v["error"].is_null(),
            None => result_blob.is_some(),
        };

    let receipt = SolveReceipt {
        call_id,
        tool,
        request_seq,
        result_seq: seq,
        ok,
        code_sha256,
        wall_ms,
        process,
        exprs_out,
        env,
        result_blob,
    };
    Some((receipt, sympy_succeeded))
}

fn parse_solve_paused(seq: u64, p: &Value) -> Option<SolvePaused> {
    let reason: PauseReason = serde_json::from_value(p.get("reason")?.clone()).ok()?;
    Some(SolvePaused {
        seq,
        reason,
        leg: p.get("leg").and_then(Value::as_u64).unwrap_or(0) as u32,
        turn: p.get("turn").and_then(Value::as_u64).unwrap_or(0) as u32,
        model_calls_used: p.get("model_calls_used").and_then(Value::as_u64).unwrap_or(0) as u32,
        open_items: p.get("open_items").and_then(Value::as_array).cloned().unwrap_or_default(),
    })
}

fn parse_solve_checkpoint(seq: u64, p: &Value) -> Option<SolveCheckpoint> {
    let reason: CheckpointReason = serde_json::from_value(p.get("reason")?.clone()).ok()?;
    Some(SolveCheckpoint {
        seq,
        leg: p.get("leg").and_then(Value::as_u64).unwrap_or(0) as u32,
        upto_seq: p.get("upto_seq").and_then(Value::as_u64).unwrap_or(0),
        brief_blob: p.get("brief_blob").and_then(Value::as_str)?.to_string(),
        brief_sha256: p.get("brief_sha256").and_then(Value::as_str)?.to_string(),
        reason,
    })
}

fn parse_solve_done(seq: u64, p: &Value) -> Option<SolveDone> {
    let status: SolveDoneStatus = serde_json::from_value(p.get("status")?.clone()).ok()?;
    let criteria: Vec<CriterionOutcome> = p
        .get("criteria")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let rests_on: RestsOnSummary = p
        .get("rests_on")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let model_calls: ModelCallsBreakdown = p
        .get("model_calls")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    Some(SolveDone {
        seq,
        final_entry_id: p.get("final_entry_id").and_then(Value::as_str)?.to_string(),
        own_tier: p.get("own_tier").and_then(Value::as_str)?.to_string(),
        effective_tier: p.get("effective_tier").and_then(Value::as_str)?.to_string(),
        criteria,
        rests_on,
        status,
        model_calls,
    })
}

#[cfg(test)]
#[path = "solve_state_tests.rs"]
mod tests;
