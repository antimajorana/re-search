//! Ledger-derived token and call usage aggregation (`GET /api/usage`).
//!
//! Single source of truth: aggregates strictly from existing `model.response`
//! ledger events (joining session/request metadata for provider, model, title,
//! and mode; falling back to `cli.result` only on subscription-CLI turns that
//! emit no `model.response` events). Produces zero cost estimates.

use crate::routes_session::{bad, internal, not_found, session_title, ApiResult};
use crate::state::{now_ms, DesktopState, SessionMeta, ANTIGRAVITY_ROUTE, CLAUDE_CLI_ROUTE};
use agent::usage::extract_usage;
use research_tools::validate_session_id;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

const MS_PER_DAY: i64 = 86_400_000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct UsageTotalsBucket {
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    pub unreported_calls: u64,
    #[serde(skip)]
    any_reported: bool,
}

impl UsageTotalsBucket {
    fn record(&mut self, call: &CallUsage) {
        self.calls = self.calls.saturating_add(call.calls);
        if call.has_tokens {
            self.any_reported = true;
            self.input_tokens = self.input_tokens.saturating_add(call.input_tokens);
            self.output_tokens = self.output_tokens.saturating_add(call.output_tokens);
            self.cached_tokens = self.cached_tokens.saturating_add(call.cached_tokens);
        }
        if !call.fully_reported {
            self.unreported_calls = self.unreported_calls.saturating_add(call.calls);
        }
    }

    fn tokens_reported(&self) -> bool {
        self.calls > 0 && self.any_reported
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct UsageTotals {
    pub today: UsageTotalsBucket,
    pub last_7_days: UsageTotalsBucket,
    pub all_time: UsageTotalsBucket,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderModelUsage {
    pub provider: String,
    pub model: String,
    #[serde(flatten)]
    pub bucket: UsageTotalsBucket,
    pub tokens_reported: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DailyUsage {
    pub date: String,
    #[serde(flatten)]
    pub bucket: UsageTotalsBucket,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionUsageSummary {
    pub session_id: String,
    pub title: String,
    pub mode: String,
    pub updated_at_ms: i64,
    #[serde(flatten)]
    pub bucket: UsageTotalsBucket,
    pub tokens_reported: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsageReport {
    pub totals: UsageTotals,
    pub by_provider_model: Vec<ProviderModelUsage>,
    pub by_day: Vec<DailyUsage>,
    pub by_session: Vec<SessionUsageSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionUsageSummary>,
}

struct CallUsage {
    ts_ms: i64,
    provider: String,
    model: String,
    calls: u64,
    input_tokens: u64,
    output_tokens: u64,
    cached_tokens: u64,
    has_tokens: bool,
    fully_reported: bool,
}

pub fn handle_get_usage(state: &DesktopState, query: &HashMap<String, String>) -> ApiResult {
    let session_filter = query.get("session_id").map(|s| s.trim()).filter(|s| !s.is_empty());
    if let Some(sid) = session_filter {
        validate_session_id(sid).map_err(|e| bad(e.to_string()))?;
        if !state.session_exists(sid).map_err(internal)? {
            return Err(not_found(format!("session `{sid}` not found")));
        }
    }
    let report = collect_usage_report(state, session_filter, now_ms())?;
    let val = serde_json::to_value(report).map_err(|e| internal(e.to_string()))?;
    Ok((200, val))
}

pub(crate) fn summarize_session_usage(
    session_id: &str,
    meta: Option<&SessionMeta>,
    events: &[ledger::Event],
) -> SessionUsageSummary {
    let mut bucket = UsageTotalsBucket::default();
    for call in &extract_session_calls(meta, events) {
        bucket.record(call);
    }
    build_session_summary(session_id, meta, events, bucket)
}

fn build_session_summary(
    session_id: &str,
    meta: Option<&SessionMeta>,
    events: &[ledger::Event],
    bucket: UsageTotalsBucket,
) -> SessionUsageSummary {
    let updated_at_ms = events.last().map(|e| e.ts_ms).or_else(|| meta.map(|m| m.created_ms)).unwrap_or(0);
    let tokens_reported = bucket.tokens_reported();
    SessionUsageSummary {
        session_id: session_id.to_string(),
        title: session_title(meta, events, session_id),
        mode: crate::session_mode::api_mode_name(crate::session_mode::session_mode(events)).to_string(),
        updated_at_ms,
        bucket,
        tokens_reported,
    }
}

fn collect_usage_report(
    state: &DesktopState,
    session_filter: Option<&str>,
    reference_now_ms: i64,
) -> Result<UsageReport, (u16, String)> {
    let ledger = state.open_ledger().map_err(internal)?;
    let session_ids: Vec<String> = match session_filter {
        Some(sid) => vec![sid.to_string()],
        None => ledger
            .sessions()
            .map_err(|e| internal(e.to_string()))?
            .into_iter()
            .map(|(id, _)| id)
            .filter(|id| validate_session_id(id).is_ok())
            .collect(),
    };

    let today_day = utc_day_number(reference_now_ms);
    let week_start = today_day - 6;
    let mut totals = UsageTotals::default();
    let mut pm_map: BTreeMap<(String, String), UsageTotalsBucket> = BTreeMap::new();
    let mut day_map: BTreeMap<i64, UsageTotalsBucket> = BTreeMap::new();
    let mut by_session = Vec::new();
    let mut selected_session = None;

    for sid in session_ids {
        if session_filter.is_none() && state.session_restore_pending(&sid).map_err(internal)? {
            continue;
        }
        let meta = state.load_meta(&sid);
        if session_filter.is_none() && meta.as_ref().is_some_and(|m| m.deleted) {
            continue;
        }
        let events = ledger.read_session(&sid).unwrap_or_default();
        let mut sess_bucket = UsageTotalsBucket::default();

        for call in &extract_session_calls(meta.as_ref(), &events) {
            let day_num = utc_day_number(call.ts_ms);
            totals.all_time.record(call);
            if day_num == today_day { totals.today.record(call); }
            if (week_start..=today_day).contains(&day_num) { totals.last_7_days.record(call); }
            pm_map.entry((call.provider.clone(), call.model.clone())).or_default().record(call);
            day_map.entry(day_num).or_default().record(call);
            sess_bucket.record(call);
        }

        let summary = build_session_summary(&sid, meta.as_ref(), &events, sess_bucket);
        if session_filter.is_some() { selected_session = Some(summary.clone()); }
        if summary.bucket.calls > 0 { by_session.push(summary); }
    }

    let mut by_provider_model: Vec<ProviderModelUsage> = pm_map
        .into_iter()
        .map(|((provider, model), bucket)| {
            let tokens_reported = bucket.tokens_reported();
            ProviderModelUsage { provider, model, bucket, tokens_reported }
        })
        .collect();
    by_provider_model.sort_by(|a, b| {
        b.bucket.input_tokens.saturating_add(b.bucket.output_tokens)
            .cmp(&a.bucket.input_tokens.saturating_add(a.bucket.output_tokens))
            .then_with(|| b.bucket.calls.cmp(&a.bucket.calls))
            .then_with(|| a.provider.cmp(&b.provider))
            .then_with(|| a.model.cmp(&b.model))
    });

    let by_day = day_map.into_iter().rev().map(|(d, bucket)| DailyUsage { date: format_utc_date(d), bucket }).collect();
    by_session.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms).then_with(|| a.session_id.cmp(&b.session_id)));

    Ok(UsageReport { totals, by_provider_model, by_day, by_session, session: selected_session })
}

fn extract_session_calls(meta: Option<&SessionMeta>, events: &[ledger::Event]) -> Vec<CallUsage> {
    let mut cur_provider = meta
        .and_then(|m| m.route.as_deref().or(m.default_provider.as_deref()))
        .map(normalize_provider)
        .unwrap_or_else(|| "unknown".to_string());
    let mut cur_model = meta
        .and_then(|m| m.default_model.as_deref())
        .map(str::trim).filter(|s| !s.is_empty()).unwrap_or("unknown").to_string();
    let (mut req_provider, mut req_model, mut turn_had_response) = (None, None, false);
    let mut out = Vec::new();

    for ev in events {
        let p = &ev.payload;
        match ev.kind.as_str() {
            "session.start" | "session.resume" => {
                (req_provider, req_model, turn_had_response) = (None, None, false);
                if let Some(prov) = non_empty(p, &["provider"]) {
                    cur_provider = normalize_provider(prov);
                } else if let Some(r) = non_empty(p, &["route"]).filter(|r| is_subscription_cli_route(r)) {
                    cur_provider = normalize_provider(r);
                }
                if let Some(m) = non_empty(p, &["model", "model_requested"]) { cur_model = m.to_string(); }
            }
            "cli.init" => if let Some(m) = non_empty(p, &["model"]) { cur_model = m.to_string(); },
            "model.request" => {
                req_provider = non_empty(p, &["provider"]).map(normalize_provider);
                req_model = non_empty(p, &["model"]).map(str::to_string);
            }
            "model.error" | "session.abort" | "session.end" => { (req_provider, req_model) = (None, None); }
            "model.response" => {
                let provider = non_empty(p, &["provider"]).map(normalize_provider)
                    .or_else(|| non_empty(p, &["route"]).filter(|r| is_subscription_cli_route(r)).map(normalize_provider))
                    .or_else(|| req_provider.take()).unwrap_or_else(|| cur_provider.clone());
                let model = non_empty(p, &["used_model", "model"]).map(str::to_string)
                    .or_else(|| req_model.take()).unwrap_or_else(|| cur_model.clone());
                (req_provider, req_model, turn_had_response) = (None, None, true);
                out.push(parse_call_usage(ev.ts_ms, provider, model, 1, p));
            }
            "cli.result" if !turn_had_response && is_subscription_cli_route(&cur_provider) => {
                let turns = p.get("num_turns").and_then(Value::as_u64).filter(|&n| n > 0).unwrap_or(1);
                let model = non_empty(p, &["used_model", "model"]).map(str::to_string).unwrap_or_else(|| cur_model.clone());
                out.push(parse_call_usage(ev.ts_ms, cur_provider.clone(), model, turns, p));
            }
            _ => {}
        }
    }
    out
}

fn parse_call_usage(ts_ms: i64, provider: String, model: String, calls: u64, payload: &Value) -> CallUsage {
    let usage_val = payload.get("usage").unwrap_or(&Value::Null);
    let mut tok = extract_usage(usage_val);
    let mut cached = extract_cached_tokens(usage_val);
    if tok.is_unknown() && cached.is_none() && payload.get("usage").is_none() {
        tok = extract_usage(payload);
        cached = extract_cached_tokens(payload);
    }
    let cached_val = cached.unwrap_or(0);
    let cli_zero = is_subscription_cli_route(&provider)
        && tok.input.unwrap_or(0) == 0 && tok.output.unwrap_or(0) == 0 && cached_val == 0;
    let has_tokens = (tok.input.is_some() || tok.output.is_some() || cached.is_some()) && !cli_zero;
    let fully_reported = tok.is_fully_known() && !cli_zero;
    CallUsage {
        ts_ms, provider, model, calls,
        input_tokens: if has_tokens { tok.input.unwrap_or(0) } else { 0 },
        output_tokens: if has_tokens { tok.output.unwrap_or(0) } else { 0 },
        cached_tokens: if has_tokens { cached_val } else { 0 },
        has_tokens,
        fully_reported,
    }
}

fn extract_cached_tokens(u: &Value) -> Option<u64> {
    let read = u.get("cached_tokens").and_then(Value::as_u64)
        .or_else(|| u.get("cache_read_input_tokens").and_then(Value::as_u64))
        .or_else(|| u.get("cachedContentTokenCount").and_then(Value::as_u64))
        .or_else(|| u.get("prompt_tokens_details").and_then(|d| d.get("cached_tokens")).and_then(Value::as_u64))
        .or_else(|| u.get("input_tokens_details").and_then(|d| d.get("cached_tokens")).and_then(Value::as_u64));
    let create = u.get("cache_creation_input_tokens").and_then(Value::as_u64);
    read.map(|r| r.saturating_add(create.unwrap_or(0))).or(create)
}

fn non_empty<'a>(payload: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| payload.get(*k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()))
}

fn normalize_provider(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" => "unknown".to_string(),
        "claude-code-cli" => CLAUDE_CLI_ROUTE.to_string(),
        "anthropic-messages" => "anthropic".to_string(),
        "openai-chat" => "openai".to_string(),
        "ollama-chat" => "ollama".to_string(),
        "gemini-generate-content" => "gemini".to_string(),
        "chatgpt-responses" => "chatgpt".to_string(),
        other => other.to_string(),
    }
}

fn is_subscription_cli_route(provider: &str) -> bool {
    matches!(provider.trim().to_ascii_lowercase().as_str(), CLAUDE_CLI_ROUTE | ANTIGRAVITY_ROUTE | "claude-code-cli")
}

pub(crate) fn utc_day_number(ts_ms: i64) -> i64 {
    ts_ms.div_euclid(MS_PER_DAY)
}

pub(crate) fn format_utc_date(day_num: i64) -> String {
    let z = day_num + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let (d, m) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
    let year = yoe + era * 400 + i64::from(m <= 2);
    format!("{year:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_date_conversion_handles_epoch_and_leap_years() {
        assert_eq!(format_utc_date(0), "1970-01-01");
        assert_eq!(format_utc_date(11_016), "2000-02-29");
        assert_eq!(format_utc_date(19_782), "2024-02-29");
        assert_eq!(format_utc_date(19_783), "2024-03-01");
        assert_eq!(format_utc_date(utc_day_number(1_795_824_000_000)), "2026-11-28");
    }
}
