//! Pre-policy tool gate and post-call hook (`solve.md` 5.1, 5.4, rule R11).
//!
//! A [`Gate`] attaches to a [`Router`] and runs at two points of every routed
//! call (in-core agent, MCP server, delegated worker, or nested host call):
//!
//! 1. [`Gate::before`] runs after `tool.request` recording, unknown-tool check,
//!    and schema argument validation, and **before** `policy::evaluate`. It may
//!    only refuse a call (`GateDecision::Block`); returning `GateDecision::Allow`
//!    never overrides a policy `Deny`, an `Ask` decision, or human approval
//!    (rule R11).
//! 2. [`Gate::after_success`] runs after the tool handler succeeds and before
//!    `tool.result` is recorded in the ledger. No ledger lock is held while it
//!    runs, so hooks (such as S9 auto-checks) may issue nested calls via
//!    `ctx.router.try_call(...)` without deadlocking.

use crate::approval::ApprovalScope;
use crate::router::{CallOutcome, Router};
use policy::EffectClass;
use serde_json::Value;
use std::sync::Arc;

/// Ledger event kind written when a Router gate blocks a tool call
/// (`solve.md` 4.1, 5.4; owner-accepted overview section 8 decisions).
pub const GUARDRAIL_BLOCKED_EVENT: &str = "guardrail.blocked";

/// Prefix used to format gate refusal tool errors as `GATE <id>: <message>`
/// (`solve.md` 5.4; owner-accepted overview section 8 decisions).
pub const GATE_ERROR_PREFIX: &str = "GATE";

/// Reason a [`Gate`] refused a tool call before policy evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateRefusal {
    /// Short gate identifier (for example `"G1"`, `"G3"`, `"G5"`, `"G7"`).
    pub gate: String,
    /// Actionable explanation shown to the caller ("what to do next").
    pub message: String,
}

impl GateRefusal {
    pub fn new(gate: impl Into<String>, message: impl Into<String>) -> Self {
        Self { gate: gate.into(), message: message.into() }
    }

    /// Format the refusal as `GATE <id>: <message>` (`solve.md` 5.4).
    pub fn formatted_error(&self) -> String {
        format!("{GATE_ERROR_PREFIX} {}: {}", self.gate, self.message)
    }
}

/// Decision returned by [`Gate::before`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    /// Proceed to policy evaluation and (if needed) approval. Never widens policy (R11).
    Allow,
    /// Refuse before policy evaluation, approval, or handler execution.
    Block(GateRefusal),
}

impl GateDecision {
    pub fn block(gate: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Block(GateRefusal::new(gate, message))
    }
}

/// Read-only view of an incoming tool call passed to [`Gate::before`] and [`Gate::on_outcome`].
#[derive(Debug, Clone, Copy)]
pub struct GateRequest<'a> {
    pub session: &'a str,
    pub caller: &'a str,
    pub tool_id: &'a str,
    pub call_id: &'a str,
    pub args: &'a Value,
    pub canonical_args: &'a str,
    pub effect: EffectClass,
    pub approval_scope: ApprovalScope,
}

/// Context passed to [`Gate::after_success`] after a handler returns `Ok(value)`
/// and before `tool.result` is written to the ledger.
pub struct PostCallCtx<'a> {
    /// Live router reference for nested calls (e.g. `host:check` -> `check_run`),
    /// which pass through the full gate, policy, and approval path.
    pub router: &'a Router,
    pub session: &'a str,
    pub caller: &'a str,
    pub tool_id: &'a str,
    pub call_id: &'a str,
    pub args: &'a Value,
    /// Mutable handler result; edits appear in both `tool.result` and `CallOutcome::Ok`.
    pub result: &'a mut Value,
}

/// Pre-policy gate and post-call hook installed on a [`Router`].
pub trait Gate: Send + Sync {
    /// Decide whether a schema-valid call to a known tool may proceed to policy evaluation.
    fn before(&self, req: &GateRequest<'_>) -> GateDecision;

    /// Run after a tool handler succeeds, before `tool.result` is recorded.
    ///
    /// Returning `Err(msg)` signals a fail-closed infrastructure or ledger error
    /// (such as propagating `Err` from a nested `ctx.router.try_call(...)`) and
    /// aborts the outer [`Router::try_call`] with `Err(msg)`. Non-fatal nested
    /// outcomes (`CallOutcome::Denied` or `CallOutcome::Error`, such as a missing
    /// `host:check` grant yielding `not_run`) should be recorded into `ctx.result`
    /// and return `Ok(())`.
    fn after_success(&self, _ctx: &mut PostCallCtx<'_>) -> Result<(), String> {
        Ok(())
    }

    /// Observe the final [`CallOutcome`] of a gated call after ledger recording completes.
    fn on_outcome(&self, _req: &GateRequest<'_>, _outcome: &CallOutcome) {}
}

/// Combinator that runs multiple [`Gate`] implementations in registration order.
///
/// - `before`: stops at the first [`GateDecision::Block`].
/// - `after_success`: runs every gate in order, stopping on the first `Err`.
/// - `on_outcome`: notifies every gate in order.
#[derive(Default)]
pub struct GateChain {
    gates: Vec<Arc<dyn Gate>>,
}

impl GateChain {
    pub fn new(gates: Vec<Arc<dyn Gate>>) -> Self {
        Self { gates }
    }

    pub fn push(&mut self, gate: Arc<dyn Gate>) {
        self.gates.push(gate);
    }

    pub fn is_empty(&self) -> bool {
        self.gates.is_empty()
    }

    pub fn len(&self) -> usize {
        self.gates.len()
    }
}

impl Gate for GateChain {
    fn before(&self, req: &GateRequest<'_>) -> GateDecision {
        for gate in &self.gates {
            if let GateDecision::Block(refusal) = gate.before(req) {
                return GateDecision::Block(refusal);
            }
        }
        GateDecision::Allow
    }

    fn after_success(&self, ctx: &mut PostCallCtx<'_>) -> Result<(), String> {
        for gate in &self.gates {
            gate.after_success(ctx)?;
        }
        Ok(())
    }

    fn on_outcome(&self, req: &GateRequest<'_>, outcome: &CallOutcome) {
        for gate in &self.gates {
            gate.on_outcome(req, outcome);
        }
    }
}
