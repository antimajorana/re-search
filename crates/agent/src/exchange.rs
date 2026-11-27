//! Generic single-job structured exchange runner over [`Transport`] (package X6).
//!
//! Governance: `00-overview.md` 3.2 row 17 ("exchange", not "leg"; owner decision 11),
//! `explore.md` 4.2 (`idea.exchange`), 5.1 (recorders), 6 (Rules P2 & P3).
//!
//! Each exchange offers the model **exactly one** [`ToolSpec`] built from a recorder schema:
//! - Any call to any other tool is rejected immediately with an error (Rule P2).
//! - A caller-supplied F9 budget check callback runs BEFORE every provider call, including
//!   repair retries (Rule P3).
//! - Runs 1 initial call plus at most `MAX_REPAIR_ATTEMPTS` (2) repair turns.
//! - Appends an `idea.exchange` audit event on every attempt sent to the provider.

use crate::transport::{
    CancelToken, ContentBlock, Message, ModelRequest, Role, StreamObserver, ToolSpec, Transport,
    TransportErrorKind,
};
use crate::usage::extract_usage;
use ledger::Ledger;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fmt;

/// Explore exchange audit event kind (`00-overview.md` 3.2 row 17 & 8.2 decision 11;
/// matches `research_tools::run_budget::IDEA_EXCHANGE_EVENT`).
pub const IDEA_EXCHANGE_EVENT: &str = "idea.exchange";

/// Ledger schema version for `idea.exchange` events (`explore.md` 4.2).
pub const EXCHANGE_SCHEMA_VERSION: u32 = 1;

/// Maximum repair turns after the initial call (`explore.md` terms: 1 call + at most 2 repairs).
pub const MAX_REPAIR_ATTEMPTS: usize = 2;

/// Default output token ceiling for a single structured exchange call.
pub const DEFAULT_EXCHANGE_MAX_TOKENS: u32 = 4_096;

/// Specification of a single structured exchange job.
#[derive(Debug, Clone)]
pub struct ExchangeSpec {
    /// Logical exchange name recorded in `idea.exchange` (e.g. `"sharpen"`, `"triage"`).
    pub exchange: String,
    /// Optional run identifier recorded in `idea.exchange["run_id"]` for F9 scoping.
    pub run_id: Option<String>,
    /// Model identifier passed in `ModelRequest.model`.
    pub model: String,
    /// Optional system prompt.
    pub system: Option<String>,
    /// Initial user prompt for the exchange.
    pub prompt: String,
    /// The single recorder [`ToolSpec`] offered to the model.
    pub tool: ToolSpec,
    /// Maximum tokens per attempt.
    pub max_tokens: u32,
    /// Optional normalized reasoning effort.
    pub reasoning_effort: Option<String>,
    /// Maximum repair turns after the first call (clamped to [`MAX_REPAIR_ATTEMPTS`]).
    pub max_repairs: usize,
}

impl ExchangeSpec {
    pub fn new(
        exchange: impl Into<String>,
        model: impl Into<String>,
        prompt: impl Into<String>,
        tool: ToolSpec,
    ) -> Self {
        Self {
            exchange: exchange.into(),
            run_id: None,
            model: model.into(),
            system: None,
            prompt: prompt.into(),
            tool,
            max_tokens: DEFAULT_EXCHANGE_MAX_TOKENS,
            reasoning_effort: None,
            max_repairs: MAX_REPAIR_ATTEMPTS,
        }
    }

    pub fn with_system(mut self, system: impl Into<String>) -> Self {
        self.system = Some(system.into());
        self
    }

    pub fn with_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = Some(run_id.into());
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    pub fn with_reasoning_effort(mut self, effort: Option<String>) -> Self {
        self.reasoning_effort = effort;
        self
    }

    pub fn with_max_repairs(mut self, max_repairs: usize) -> Self {
        self.max_repairs = max_repairs.min(MAX_REPAIR_ATTEMPTS);
        self
    }
}

/// Audit summary of one provider attempt recorded as an `idea.exchange` ledger event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExchangeAttemptAudit {
    pub attempt: usize,
    pub event_seq: u64,
    pub model: String,
    pub tokens_in: Option<u64>,
    pub tokens_out: Option<u64>,
    pub request_blob: String,
    pub response_blob: Option<String>,
    pub accepted: bool,
    pub faults: Vec<String>,
}

/// Result of a successfully validated exchange.
#[derive(Debug, Clone)]
pub struct ExchangeOutcome<T> {
    pub value: T,
    pub attempts: usize,
    pub audit: Vec<ExchangeAttemptAudit>,
    pub last_event_seq: u64,
}

/// Structured failure from [`run_exchange`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExchangeError {
    BudgetExceeded {
        attempts: usize,
        reason: String,
    },
    Cancelled {
        attempts: usize,
    },
    UnexpectedToolCall {
        attempts: usize,
        tool: String,
        expected: String,
    },
    ValidationExhausted {
        attempts: usize,
        last_faults: Vec<String>,
    },
    Transport {
        attempts: usize,
        message: String,
    },
    Ledger(String),
}

impl fmt::Display for ExchangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BudgetExceeded { reason, .. } => write!(f, "budget cap reached: {reason}"),
            Self::Cancelled { .. } => write!(f, "exchange stopped by user"),
            Self::UnexpectedToolCall { tool, expected, .. } => write!(
                f,
                "unexpected tool call `{tool}`; only `{expected}` is permitted (Rule P2)"
            ),
            Self::ValidationExhausted { attempts, last_faults } => write!(
                f,
                "exchange validation failed after {attempts} attempt(s): {}",
                last_faults.join("; ")
            ),
            Self::Transport { message, .. } => write!(f, "model transport error: {message}"),
            Self::Ledger(msg) => write!(f, "ledger error: {msg}"),
        }
    }
}

impl std::error::Error for ExchangeError {}

struct CancelObserver<'a> {
    cancel: Option<&'a CancelToken>,
}

impl StreamObserver for CancelObserver<'_> {
    fn is_cancelled(&self) -> bool {
        self.cancel.is_some_and(CancelToken::is_cancelled)
    }
}

/// Run a single-job structured exchange over `transport` with at most `1 + max_repairs`
/// (capped at 3) provider calls.
///
/// - Offers strictly `vec![spec.tool.clone()]` to the provider.
/// - Invokes `budget_check()` BEFORE every provider call (Rule P3).
/// - Rejects any non-recorder tool call immediately with [`ExchangeError::UnexpectedToolCall`]
///   (Rule P2).
/// - Appends an `idea.exchange` audit event on every attempt sent to the provider.
#[allow(clippy::too_many_arguments)]
pub fn run_exchange<T, B, V>(
    transport: &dyn Transport,
    ledger: &Ledger,
    session: &str,
    spec: &ExchangeSpec,
    cancel: Option<&CancelToken>,
    mut budget_check: B,
    mut validator: V,
) -> Result<ExchangeOutcome<T>, ExchangeError>
where
    B: FnMut() -> Result<(), String>,
    V: FnMut(&Value) -> Result<T, Vec<String>>,
{
    if spec.exchange.trim().is_empty() {
        return Err(ExchangeError::Ledger("exchange name must be non-empty".into()));
    }
    if spec.tool.name.trim().is_empty() {
        return Err(ExchangeError::Ledger("exchange tool name must be non-empty".into()));
    }

    let max_attempts = 1 + spec.max_repairs.min(MAX_REPAIR_ATTEMPTS);
    let mut messages = vec![Message {
        role: Role::User,
        content: vec![ContentBlock::Text {
            text: spec.prompt.clone(),
        }],
    }];
    let mut audit = Vec::with_capacity(max_attempts);
    let mut last_faults = Vec::new();

    for attempt in 1..=max_attempts {
        if cancel.is_some_and(CancelToken::is_cancelled) {
            return Err(ExchangeError::Cancelled {
                attempts: attempt - 1,
            });
        }
        // Rule P3: check F9 budget BEFORE every provider call, including repair retries.
        if let Err(reason) = budget_check() {
            return Err(ExchangeError::BudgetExceeded {
                attempts: attempt - 1,
                reason,
            });
        }

        let req = ModelRequest {
            model: spec.model.clone(),
            system: spec.system.clone(),
            messages: messages.clone(),
            tools: vec![spec.tool.clone()],
            max_tokens: spec.max_tokens.max(256),
            reasoning_effort: spec.reasoning_effort.clone(),
        };
        let prepared = transport.prepare_request(&req).map_err(|e| ExchangeError::Transport {
            attempts: attempt - 1,
            message: e.to_string(),
        })?;
        let request_blob = ledger
            .put_blob(&prepared.body)
            .map_err(|e| ExchangeError::Ledger(e.to_string()))?;

        let mut observer = CancelObserver { cancel };
        let resp = match transport.send_prepared_streaming(&prepared, &mut observer) {
            Ok(resp) => resp,
            Err(err) => {
                let response_blob = match err.raw_body.as_deref() {
                    Some(raw) if !raw.is_empty() => Some(
                        ledger
                            .put_blob(raw)
                            .map_err(|e| ExchangeError::Ledger(e.to_string()))?,
                    ),
                    _ => None,
                };
                let fault = err.to_string();
                append_audit(
                    ledger,
                    session,
                    spec,
                    attempt,
                    transport.info().provider,
                    &spec.model,
                    None,
                    None,
                    &request_blob,
                    response_blob.as_deref(),
                    false,
                    std::slice::from_ref(&fault),
                )?;
                if err.kind == TransportErrorKind::Cancelled {
                    return Err(ExchangeError::Cancelled { attempts: attempt });
                }
                return Err(ExchangeError::Transport {
                    attempts: attempt,
                    message: fault,
                });
            }
        };

        let response_blob = ledger
            .put_blob(&resp.raw_body)
            .map_err(|e| ExchangeError::Ledger(e.to_string()))?;
        let usage = extract_usage(&resp.usage);
        let resp_model = if resp.model.trim().is_empty() {
            spec.model.clone()
        } else {
            resp.model.clone()
        };

        let tool_uses: Vec<(&str, &str, &Value)> = resp
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolUse { id, name, input } => {
                    Some((id.as_str(), name.as_str(), input))
                }
                _ => None,
            })
            .collect();

        let provider_name = transport.info().provider;

        // Rule P2: reject any tool call other than the single offered recorder immediately.
        if let Some((_, bad_name, _)) = tool_uses.iter().find(|(_, name, _)| *name != spec.tool.name)
        {
            let fault = format!(
                "unexpected tool call `{bad_name}`; only `{}` is permitted (Rule P2)",
                spec.tool.name
            );
            append_audit(
                ledger,
                session,
                spec,
                attempt,
                provider_name,
                &resp_model,
                usage.input,
                usage.output,
                &request_blob,
                Some(&response_blob),
                false,
                std::slice::from_ref(&fault),
            )?;
            return Err(ExchangeError::UnexpectedToolCall {
                attempts: attempt,
                tool: (*bad_name).to_string(),
                expected: spec.tool.name.clone(),
            });
        }

        if tool_uses.is_empty() {
            let faults = vec![format!(
                "response did not call required recorder `{}`; call `{}` with valid JSON arguments",
                spec.tool.name, spec.tool.name
            )];
            let rec = append_audit(
                ledger,
                session,
                spec,
                attempt,
                provider_name,
                &resp_model,
                usage.input,
                usage.output,
                &request_blob,
                Some(&response_blob),
                false,
                &faults,
            )?;
            audit.push(rec);
            last_faults = faults.clone();
            if attempt < max_attempts {
                push_assistant_content(&mut messages, resp.content);
                messages.push(Message {
                    role: Role::User,
                    content: vec![ContentBlock::Text {
                        text: format_repair_feedback(&spec.tool.name, &faults),
                    }],
                });
            }
            continue;
        }

        if tool_uses.len() > 1 {
            let faults = vec![format!(
                "expected exactly one call to `{}`, received {}",
                spec.tool.name,
                tool_uses.len()
            )];
            let rec = append_audit(
                ledger,
                session,
                spec,
                attempt,
                provider_name,
                &resp_model,
                usage.input,
                usage.output,
                &request_blob,
                Some(&response_blob),
                false,
                &faults,
            )?;
            audit.push(rec);
            last_faults = faults.clone();
            if attempt < max_attempts {
                let err_text = format_repair_feedback(&spec.tool.name, &faults);
                let result_blocks = tool_uses
                    .iter()
                    .map(|(id, _, _)| ContentBlock::ToolResult {
                        tool_use_id: (*id).to_string(),
                        is_error: true,
                        content: err_text.clone(),
                    })
                    .collect();
                push_assistant_content(&mut messages, resp.content);
                messages.push(Message {
                    role: Role::User,
                    content: result_blocks,
                });
            }
            continue;
        }

        let (tool_use_id, _, input) = tool_uses[0];
        match validator(input) {
            Ok(value) => {
                let rec = append_audit(
                    ledger,
                    session,
                    spec,
                    attempt,
                    provider_name,
                    &resp_model,
                    usage.input,
                    usage.output,
                    &request_blob,
                    Some(&response_blob),
                    true,
                    &[],
                )?;
                let last_event_seq = rec.event_seq;
                audit.push(rec);
                return Ok(ExchangeOutcome {
                    value,
                    attempts: attempt,
                    audit,
                    last_event_seq,
                });
            }
            Err(mut faults) => {
                if faults.is_empty() {
                    faults.push("recorder validation rejected input".to_string());
                }
                let rec = append_audit(
                    ledger,
                    session,
                    spec,
                    attempt,
                    provider_name,
                    &resp_model,
                    usage.input,
                    usage.output,
                    &request_blob,
                    Some(&response_blob),
                    false,
                    &faults,
                )?;
                audit.push(rec);
                last_faults = faults.clone();
                if attempt < max_attempts {
                    let tool_use_id = tool_use_id.to_string();
                    push_assistant_content(&mut messages, resp.content);
                    messages.push(Message {
                        role: Role::User,
                        content: vec![ContentBlock::ToolResult {
                            tool_use_id,
                            is_error: true,
                            content: format_repair_feedback(&spec.tool.name, &faults),
                        }],
                    });
                }
            }
        }
    }

    Err(ExchangeError::ValidationExhausted {
        attempts: max_attempts,
        last_faults,
    })
}

fn push_assistant_content(messages: &mut Vec<Message>, content: Vec<ContentBlock>) {
    if content.is_empty() {
        messages.push(Message {
            role: Role::Assistant,
            content: vec![ContentBlock::Text {
                text: String::new(),
            }],
        });
    } else {
        messages.push(Message {
            role: Role::Assistant,
            content,
        });
    }
}

fn format_repair_feedback(tool_name: &str, faults: &[String]) -> String {
    let mut out = format!("Validation rejected `{tool_name}` with {} fault(s):\n", faults.len());
    for (idx, fault) in faults.iter().enumerate() {
        out.push_str(&format!("{}. {fault}\n", idx + 1));
    }
    out.push_str(&format!(
        "Fix every fault above and call `{tool_name}` once with valid structured arguments."
    ));
    out
}

#[allow(clippy::too_many_arguments)]
fn append_audit(
    ledger: &Ledger,
    session: &str,
    spec: &ExchangeSpec,
    attempt: usize,
    provider: &str,
    model: &str,
    tokens_in: Option<u64>,
    tokens_out: Option<u64>,
    request_blob: &str,
    response_blob: Option<&str>,
    accepted: bool,
    faults: &[String],
) -> Result<ExchangeAttemptAudit, ExchangeError> {
    let error_summary = (!accepted && !faults.is_empty()).then(|| faults.join("; "));
    let payload = json!({
        "exchange": spec.exchange,
        "stage": spec.exchange,
        "attempt": attempt,
        "provider": provider,
        "model": model,
        "tokens_in": tokens_in,
        "tokens_out": tokens_out,
        "input_tokens": tokens_in,
        "output_tokens": tokens_out,
        "request_blob": request_blob,
        "response_blob": response_blob,
        "accepted": accepted,
        "ok": accepted,
        "faults": faults,
        "error": error_summary,
        "run_id": spec.run_id,
        "job_id": spec.run_id,
    });
    let ev = ledger
        .append_with_hash(session, IDEA_EXCHANGE_EVENT, EXCHANGE_SCHEMA_VERSION, &payload)
        .map_err(|e| ExchangeError::Ledger(e.to_string()))?;
    Ok(ExchangeAttemptAudit {
        attempt,
        event_seq: ev.seq,
        model: model.to_string(),
        tokens_in,
        tokens_out,
        request_blob: request_blob.to_string(),
        response_blob: response_blob.map(str::to_string),
        accepted,
        faults: faults.to_vec(),
    })
}

#[cfg(test)]
#[path = "exchange_tests.rs"]
mod tests;
