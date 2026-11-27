//! Honest provider token usage extraction, role/model rollups, and scoped OpenAI
//! SSE usage opt-in (`00-overview.md` §3.4, `audit-orchestration-routing.md` §2.4).
//!
//! Missing or `null` token fields yield `None` (`null` in JSON, `"unknown"` in UI)
//! rather than false `0`, while genuine provider-reported `0` yields `Some(0)`.

use crate::transport::ProviderKind;
use ledger::Event;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Candidate JSON field names for input token counts across Anthropic, OpenAI, and Gemini.
pub const INPUT_TOKEN_FIELDS: &[&str] = &["input_tokens", "prompt_tokens", "promptTokenCount"];
/// Candidate JSON field names for output token counts across Anthropic, OpenAI, and Gemini.
pub const OUTPUT_TOKEN_FIELDS: &[&str] = &["output_tokens", "completion_tokens", "candidatesTokenCount"];

/// Honest per-call token usage (`None` means unknown/unreported, never coerced to `0`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input: Option<u64>,
    pub output: Option<u64>,
}

impl TokenUsage {
    pub fn is_fully_known(self) -> bool {
        self.input.is_some() && self.output.is_some()
    }

    pub fn is_unknown(self) -> bool {
        self.input.is_none() && self.output.is_none()
    }
}

/// Extract honest [`TokenUsage`] from a provider `usage` JSON payload.
///
/// Checks `Value::as_u64` **per candidate field** so `{ "input_tokens": null, "prompt_tokens": 42 }`
/// recovers `Some(42)` instead of short-circuiting on `Value::Null`. Missing or non-numeric
/// values yield `None` (never false `0`).
pub fn extract_usage(usage: &Value) -> TokenUsage {
    let input = INPUT_TOKEN_FIELDS
        .iter()
        .find_map(|field| usage.get(*field).and_then(Value::as_u64));
    let output = OUTPUT_TOKEN_FIELDS
        .iter()
        .find_map(|field| usage.get(*field).and_then(Value::as_u64));
    TokenUsage { input, output }
}

/// Inject `"stream_options": { "include_usage": true }` into serialized OpenAI Chat Completions
/// bodies **only** when `stream && provider == ProviderKind::OpenAi`.
///
/// Never modifies Anthropic (`400`-breaking), Ollama/loopback non-streaming, Gemini, or ChatGPT
/// Responses bodies. Preserves any existing `stream_options` object fields.
pub fn inject_openai_stream_usage(
    raw_body: Vec<u8>,
    provider: ProviderKind,
    stream: bool,
) -> Vec<u8> {
    if !stream || provider != ProviderKind::OpenAi {
        return raw_body;
    }
    let Ok(Value::Object(mut map)) = serde_json::from_slice::<Value>(&raw_body) else {
        return raw_body;
    };
    let mut opts = match map.remove("stream_options") {
        Some(Value::Object(existing)) => existing,
        _ => serde_json::Map::new(),
    };
    opts.insert("include_usage".into(), json!(true));
    map.insert("stream_options".into(), Value::Object(opts));
    serde_json::to_vec(&Value::Object(map)).unwrap_or(raw_body)
}

/// Strip `stream_options` from a prepared JSON body (1-time fallback for OpenAI-compatible
/// gateways that return HTTP 400 on `stream_options`).
pub fn strip_openai_stream_options(raw_body: Vec<u8>) -> Vec<u8> {
    let Ok(Value::Object(mut map)) = serde_json::from_slice::<Value>(&raw_body) else {
        return raw_body;
    };
    if map.remove("stream_options").is_none() {
        return raw_body;
    }
    serde_json::to_vec(&Value::Object(map)).unwrap_or(raw_body)
}

/// Grouping key for [`RoleUsageRollup`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RoleUsageKey {
    pub role: String,
    pub provider: String,
    pub used_model: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct UsageBucket {
    calls: u64,
    input_tokens: u64,
    output_tokens: u64,
    known_input_calls: u64,
    known_output_calls: u64,
    unknown_usage_calls: u64,
}

impl UsageBucket {
    fn record(&mut self, usage: TokenUsage) {
        self.calls = self.calls.saturating_add(1);
        if let Some(inp) = usage.input {
            self.input_tokens = self.input_tokens.saturating_add(inp);
            self.known_input_calls = self.known_input_calls.saturating_add(1);
        }
        if let Some(out) = usage.output {
            self.output_tokens = self.output_tokens.saturating_add(out);
            self.known_output_calls = self.known_output_calls.saturating_add(1);
        }
        if !usage.is_fully_known() {
            self.unknown_usage_calls = self.unknown_usage_calls.saturating_add(1);
        }
    }

    fn merge(&mut self, other: &UsageBucket) {
        self.calls = self.calls.saturating_add(other.calls);
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.known_input_calls = self.known_input_calls.saturating_add(other.known_input_calls);
        self.known_output_calls = self.known_output_calls.saturating_add(other.known_output_calls);
        self.unknown_usage_calls = self.unknown_usage_calls.saturating_add(other.unknown_usage_calls);
    }

    fn input_opt(&self) -> Option<u64> {
        (self.known_input_calls > 0).then_some(self.input_tokens)
    }

    fn output_opt(&self) -> Option<u64> {
        (self.known_output_calls > 0).then_some(self.output_tokens)
    }
}

/// Public JSON-serializable usage summary for one `(role, provider, used_model)` bucket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleUsageSummary {
    pub role: String,
    pub provider: String,
    pub used_model: String,
    pub calls: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub unknown_usage_calls: u64,
}

/// Aggregated token and call accounting grouped by `(role, provider, used_model)`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoleUsageRollup {
    buckets: BTreeMap<RoleUsageKey, UsageBucket>,
}

impl RoleUsageRollup {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one provider call with honest optional token counts.
    pub fn record_call(
        &mut self,
        role: impl Into<String>,
        provider: impl Into<String>,
        used_model: impl Into<String>,
        usage: TokenUsage,
    ) {
        let key = RoleUsageKey {
            role: norm_label(&role.into(), "unknown"),
            provider: norm_label(&provider.into(), "unknown"),
            used_model: norm_label(&used_model.into(), "unknown"),
        };
        self.buckets.entry(key).or_default().record(usage);
    }

    /// Fold an entire session's ledger events into an honest per-role/provider/model rollup.
    pub fn from_events(events: &[Event]) -> Self {
        let mut rollup = Self::new();
        let mut current_provider = String::from("unknown");
        let mut current_model = String::from("unknown");
        let mut pending_main: Option<(String, String, String)> = None;
        let mut pending_verifier: Option<(String, String, String)> = None;
        let mut pending_strategist: Option<(String, String, String)> = None;
        let mut pending_narrator: Option<(String, String, String)> = None;

        #[derive(Default)]
        struct DelegateAcc {
            role: String,
            provider: String,
            model: String,
            turns: Vec<(String, TokenUsage)>,
            end_turns: u64,
            end_used_model: Option<String>,
            end_usage: Option<TokenUsage>,
        }
        let mut delegates: BTreeMap<String, DelegateAcc> = BTreeMap::new();
        let mut explores: BTreeMap<String, DelegateAcc> = BTreeMap::new();

        for ev in events {
            let p = &ev.payload;
            match ev.kind.as_str() {
                "session.start" | "session.resume" => {
                    if let Some(prov) = non_empty(p, &["provider"]) {
                        current_provider = prov.to_string();
                    }
                    if let Some(m) = non_empty(p, &["model", "model_requested"]) {
                        current_model = m.to_string();
                    }
                }
                "model.request" => {
                    if let Some((r, prov, m)) = pending_main.take() {
                        rollup.record_call(r, prov, m, TokenUsage::default());
                    }
                    let role = non_empty(p, &["role"]).unwrap_or("orchestrator").to_string();
                    let prov = non_empty(p, &["provider"]).unwrap_or(&current_provider).to_string();
                    let m = non_empty(p, &["model"]).unwrap_or(&current_model).to_string();
                    pending_main = Some((role, prov, m));
                }
                "model.response" => {
                    let (role, prov, req_m) = pending_main
                        .take()
                        .unwrap_or_else(|| ("orchestrator".into(), current_provider.clone(), current_model.clone()));
                    let role = non_empty(p, &["role"]).unwrap_or(&role).to_string();
                    let prov = non_empty(p, &["provider"]).unwrap_or(&prov).to_string();
                    let used = non_empty(p, &["used_model", "model"]).unwrap_or(&req_m).to_string();
                    let usage = p.get("usage").map(extract_usage).unwrap_or_default();
                    rollup.record_call(role, prov, used, usage);
                }
                "model.error" | "session.abort" | "session.end" => {
                    if let Some((r, prov, m)) = pending_main.take() {
                        rollup.record_call(r, prov, m, TokenUsage::default());
                    }
                }
                "delegate.start" => {
                    if let Some(cid) = non_empty(p, &["call_id"]) {
                        let entry = delegates.entry(cid.to_string()).or_default();
                        entry.role = non_empty(p, &["role", "worker"]).unwrap_or("worker").to_string();
                        entry.provider = non_empty(p, &["provider"]).unwrap_or("unknown").to_string();
                        entry.model = non_empty(p, &["model"]).unwrap_or("unknown").to_string();
                    }
                }
                "delegate.turn" => {
                    let usage = p.get("usage").map(extract_usage).unwrap_or_default();
                    let used = non_empty(p, &["used_model", "model"]).unwrap_or("").to_string();
                    if let Some(cid) = non_empty(p, &["call_id"]) {
                        let entry = delegates.entry(cid.to_string()).or_default();
                        if entry.role.is_empty() {
                            entry.role = non_empty(p, &["role"]).unwrap_or("worker").to_string();
                        }
                        if entry.provider.is_empty() {
                            if let Some(prov) = non_empty(p, &["provider"]) {
                                entry.provider = prov.to_string();
                            }
                        }
                        let m = if used.is_empty() { entry.model.clone() } else { used };
                        entry.turns.push((m, usage));
                    } else {
                        let role = non_empty(p, &["role"]).unwrap_or("worker");
                        let prov = non_empty(p, &["provider"]).unwrap_or("unknown");
                        rollup.record_call(role, prov, &used, usage);
                    }
                }
                "delegate.end" => {
                    let turns = p.get("turns").and_then(Value::as_u64).unwrap_or(0);
                    let role = non_empty(p, &["worker", "role"]).unwrap_or("worker").to_string();
                    let prov = non_empty(p, &["provider"]).unwrap_or("unknown").to_string();
                    let used = non_empty(p, &["used_model", "model"]).unwrap_or("unknown").to_string();
                    let end_usage = extract_end_usage(p);
                    if let Some(cid) = non_empty(p, &["call_id"]) {
                        let entry = delegates.entry(cid.to_string()).or_default();
                        if entry.role.is_empty() { entry.role = role; }
                        if entry.provider.is_empty() || entry.provider == "unknown" { entry.provider = prov; }
                        if entry.model.is_empty() { entry.model = used.clone(); }
                        entry.end_turns = entry.end_turns.max(turns);
                        entry.end_used_model = Some(used);
                        entry.end_usage = end_usage;
                    } else {
                        for idx in 0..turns {
                            let u = if idx == 0 && turns == 1 { end_usage.unwrap_or_default() } else { TokenUsage::default() };
                            rollup.record_call(&role, &prov, &used, u);
                        }
                    }
                }
                "explore.start" => {
                    if let Some(cid) = non_empty(p, &["call_id"]) {
                        let entry = explores.entry(cid.to_string()).or_default();
                        entry.role = non_empty(p, &["role"]).unwrap_or("explorer_worker").to_string();
                        entry.provider = non_empty(p, &["provider"]).unwrap_or("unknown").to_string();
                        entry.model = non_empty(p, &["model"]).unwrap_or("unknown").to_string();
                    }
                }
                "explore.branch" => {
                    let turns = p.get("turns").and_then(Value::as_u64).unwrap_or(0);
                    let used = non_empty(p, &["used_model", "model"]).unwrap_or("").to_string();
                    let bu = extract_end_usage(p);
                    if let Some(cid) = non_empty(p, &["call_id"]) {
                        let entry = explores.entry(cid.to_string()).or_default();
                        let m = if used.is_empty() { entry.model.clone() } else { used };
                        for idx in 0..turns {
                            let u = if idx == 0 && turns == 1 { bu.unwrap_or_default() } else { TokenUsage::default() };
                            entry.turns.push((m.clone(), u));
                        }
                    }
                }
                "explore.end" => {
                    let turns = p.get("turns").and_then(Value::as_u64).unwrap_or(0);
                    let role = non_empty(p, &["worker", "role"]).unwrap_or("explorer_worker").to_string();
                    let prov = non_empty(p, &["provider"]).unwrap_or("unknown").to_string();
                    let used = non_empty(p, &["used_model", "model"]).unwrap_or("unknown").to_string();
                    let end_usage = extract_end_usage(p);
                    if let Some(cid) = non_empty(p, &["call_id"]) {
                        let entry = explores.entry(cid.to_string()).or_default();
                        if entry.role.is_empty() { entry.role = role; }
                        if entry.provider.is_empty() || entry.provider == "unknown" { entry.provider = prov; }
                        if entry.model.is_empty() { entry.model = used.clone(); }
                        entry.end_turns = entry.end_turns.max(turns);
                        entry.end_used_model = Some(used);
                        entry.end_usage = end_usage;
                    }
                }
                "idea.exchange" => {
                    let role = non_empty(p, &["role"]).unwrap_or("orchestrator");
                    let prov = non_empty(p, &["provider"]).unwrap_or("unknown");
                    let used = non_empty(p, &["used_model", "model"]).unwrap_or("unknown");
                    let usage = p
                        .get("usage")
                        .map(extract_usage)
                        .or_else(|| extract_end_usage(p))
                        .unwrap_or_default();
                    rollup.record_call(role, prov, used, usage);
                }
                "verifier.request" => {
                    if let Some((r, prov, m)) = pending_verifier.take() {
                        rollup.record_call(r, prov, m, TokenUsage::default());
                    }
                    let role = non_empty(p, &["role"]).unwrap_or("verifier").to_string();
                    let prov = non_empty(p, &["provider"]).unwrap_or("unknown").to_string();
                    let m = non_empty(p, &["verifier_model", "model"]).unwrap_or("unknown").to_string();
                    pending_verifier = Some((role, prov, m));
                }
                "proof.verification" | "claim.review" | "verifier.response" | "verifier.incomplete" | "verifier.cancelled" => {
                    let (role, prov, req_m) = pending_verifier
                        .take()
                        .unwrap_or_else(|| ("verifier".into(), "unknown".into(), "unknown".into()));
                    let role = non_empty(p, &["role"]).unwrap_or(&role).to_string();
                    let prov = non_empty(p, &["provider"]).unwrap_or(&prov).to_string();
                    let used = non_empty(p, &["used_model", "verifier_model", "model"])
                        .unwrap_or(&req_m)
                        .to_string();
                    let usage = p.get("usage").map(extract_usage).unwrap_or_default();
                    rollup.record_call(role, prov, used, usage);
                }
                "strategist.request" => {
                    if let Some((r, prov, m)) = pending_strategist.take() {
                        rollup.record_call(r, prov, m, TokenUsage::default());
                    }
                    let prov = non_empty(p, &["provider"]).unwrap_or("unknown").to_string();
                    let m = non_empty(p, &["model"]).unwrap_or("unknown").to_string();
                    pending_strategist = Some(("strategist".into(), prov, m));
                }
                "strategist.consult" => {
                    let (role, prov, req_m) = pending_strategist
                        .take()
                        .unwrap_or_else(|| ("strategist".into(), "unknown".into(), "unknown".into()));
                    let prov = non_empty(p, &["provider"]).unwrap_or(&prov).to_string();
                    let used = non_empty(p, &["used_model", "model"]).unwrap_or(&req_m).to_string();
                    let usage = p.get("usage").map(extract_usage).unwrap_or_default();
                    rollup.record_call(role, prov, used, usage);
                }
                "narrator.request" => {
                    if let Some((r, prov, m)) = pending_narrator.take() {
                        rollup.record_call(r, prov, m, TokenUsage::default());
                    }
                    let prov = non_empty(p, &["provider"]).unwrap_or("unknown").to_string();
                    let m = non_empty(p, &["model"]).unwrap_or("unknown").to_string();
                    pending_narrator = Some(("narrator".into(), prov, m));
                }
                "narrator.report" => {
                    let (role, prov, req_m) = pending_narrator
                        .take()
                        .unwrap_or_else(|| ("narrator".into(), "unknown".into(), "unknown".into()));
                    let prov = non_empty(p, &["provider"]).unwrap_or(&prov).to_string();
                    let used = non_empty(p, &["used_model", "model"]).unwrap_or(&req_m).to_string();
                    let usage = p.get("usage").map(extract_usage).unwrap_or_default();
                    rollup.record_call(role, prov, used, usage);
                }
                _ => {}
            }
        }

        for pending in [pending_main, pending_verifier, pending_strategist, pending_narrator].into_iter().flatten() {
            rollup.record_call(pending.0, pending.1, pending.2, TokenUsage::default());
        }
        for acc in delegates.into_values().chain(explores.into_values()) {
            let role = if acc.role.is_empty() { "worker".to_string() } else { acc.role };
            let prov = if acc.provider.is_empty() { "unknown".to_string() } else { acc.provider };
            let fallback_m = acc.end_used_model.unwrap_or(acc.model);
            let seen = acc.turns.len() as u64;
            for (m, u) in acc.turns {
                let model = if m.is_empty() { fallback_m.clone() } else { m };
                rollup.record_call(&role, &prov, model, u);
            }
            if acc.end_turns > seen {
                let missing = acc.end_turns - seen;
                for _ in 0..missing {
                    let u = if seen == 0 && missing == 1 {
                        acc.end_usage.unwrap_or_default()
                    } else {
                        TokenUsage::default()
                    };
                    rollup.record_call(&role, &prov, &fallback_m, u);
                }
            }
        }
        rollup
    }

    pub fn summaries(&self) -> Vec<RoleUsageSummary> {
        self.buckets
            .iter()
            .map(|(k, b)| RoleUsageSummary {
                role: k.role.clone(),
                provider: k.provider.clone(),
                used_model: k.used_model.clone(),
                calls: b.calls,
                input_tokens: b.input_opt(),
                output_tokens: b.output_opt(),
                unknown_usage_calls: b.unknown_usage_calls,
            })
            .collect()
    }

    pub fn totals(&self) -> (u64, Option<u64>, Option<u64>, u64) {
        let mut total = UsageBucket::default();
        for b in self.buckets.values() {
            total.merge(b);
        }
        (total.calls, total.input_opt(), total.output_opt(), total.unknown_usage_calls)
    }

    /// Render the rollup as JSON where purely unknown token buckets serialize `null` (never `0`).
    pub fn to_json(&self) -> Value {
        let (calls, input_tokens, output_tokens, unknown_usage_calls) = self.totals();
        json!({
            "roles": self.summaries(),
            "totals": {
                "calls": calls,
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
                "unknown_usage_calls": unknown_usage_calls,
            }
        })
    }
}

fn non_empty<'a>(payload: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|k| payload.get(*k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()))
}

fn norm_label(raw: &str, fallback: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() { fallback.to_string() } else { trimmed.to_string() }
}

fn extract_end_usage(payload: &Value) -> Option<TokenUsage> {
    if let Some(u) = payload.get("usage") {
        let parsed = extract_usage(u);
        if !parsed.is_unknown() {
            return Some(parsed);
        }
    }
    let inp = payload.get("input_tokens").and_then(Value::as_u64);
    let out = payload.get("output_tokens").and_then(Value::as_u64);
    if inp.is_some() || out.is_some() {
        Some(TokenUsage { input: inp, output: out })
    } else {
        None
    }
}

#[cfg(test)]
#[path = "usage_tests.rs"]
mod tests;
