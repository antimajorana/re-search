//! Definitions of the v0 tools and the static policy. Handlers live with
//! the process that owns the resources (the MCP server); the supervisor uses
//! these definitions to know which tools it expects the model to see.

use crate::approval::ApprovalScope;
use crate::registry::ToolDef;
use policy::{Decision, EffectClass, Match, Policy, Rule};
use serde_json::json;

pub const POLICY_VERSION: &str = "v0.2";
pub const SYMPY_EVAL: &str = "sympy_eval";
pub const LEDGER_NOTE: &str = "ledger_note";

pub fn sympy_eval_def() -> ToolDef {
    ToolDef {
        id: SYMPY_EVAL.into(),
        description: "Run Python code in the local compute worker and return the value of the last \
                      expression plus captured stdout/stderr. `sympy` is imported and `from sympy import *` \
                      is in effect; names persist between calls until the worker restarts. Requires the \
                      user's approval (each call when uncontained; once per live worker session when \
                      sandboxed). The result is a computation, not a verified claim."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "code": {"type": "string", "maxLength": 20000, "description": "Python source to run."}
            },
            "required": ["code"],
            "additionalProperties": false
        }),
        effect: EffectClass::ExecuteCode,
        approval_scope: ApprovalScope::WorkerSession,
    }
}

pub fn ledger_note_def() -> ToolDef {
    ToolDef {
        id: LEDGER_NOTE.into(),
        description: "Append a short note to the session ledger. Notes are stored with status \
                      `unverified`; this tool cannot mark anything as verified."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "text": {"type": "string", "maxLength": 4000, "description": "The note."}
            },
            "required": ["text"],
            "additionalProperties": false
        }),
        // Writes only to the core's own append-only ledger.
        effect: EffectClass::ObserveLocal,
        approval_scope: ApprovalScope::Once,
    }
}

pub fn defs() -> Vec<ToolDef> {
    vec![ledger_note_def(), sympy_eval_def()]
}

/// The static v0.2 policy: notes are allowed, code execution asks (each call
/// when uncontained; once per live worker session when sandboxed), anything
/// else is denied.
pub fn default_policy() -> Policy {
    let rule = |id: &str, tool: &str, decision| Rule {
        id: id.into(),
        caller: Match::Any,
        tool: Match::Exact(tool.into()),
        effect: None,
        tool_def_hash: None,
        decision,
    };
    Policy {
        version: POLICY_VERSION.into(),
        rules: vec![
            rule("note-allow", LEDGER_NOTE, Decision::Allow),
            rule("sympy-ask", SYMPY_EVAL, Decision::Ask),
        ],
        default: Decision::Deny,
    }
}
