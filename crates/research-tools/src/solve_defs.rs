//! Frozen `solve/1` tool definitions and fail-closed S7 stub handlers
//! (`solve.md` 5.1, rules R1, R3, R5, R18; package S7).
//!
//! All four Solve tool definitions (`expr_show`, `step_record`, `check_run`, `solve_finish`)
//! are frozen here so packages S8, S9, and S11 can wire their real handlers with zero
//! `def_hash` or `policy_version` changes.

use crate::{
    arxiv, fs_tools, literature, paper_lines, paper_passages, proof_dag, sanity, tex_audit,
    ResearchEnv,
};
use policy::EffectClass;
use serde_json::{json, Value};
use std::sync::Arc;
use tools::{builtin, ApprovalScope, CallCtx, Registry, ToolDef, ToolHandler, WorkerEpoch};

/// Inspect a stored versioned expression handle or list session handles (`solve.md` 5.1, S7).
pub const EXPR_SHOW: &str = "expr_show";
/// Record a Solve proof-graph step with optional claims over handles (`solve.md` 5.1, S8).
pub const STEP_RECORD: &str = "step_record";
/// Run a host-registered mathematical check on a step claim in the check process (`solve.md` 5.1, S9).
pub const CHECK_RUN: &str = "check_run";
/// Finish a Solve run and evaluate the done checklist from the ledger (`solve.md` 5.1, S11).
pub const SOLVE_FINISH: &str = "solve_finish";

/// Exact sorted 16-tool inventory of profile `solve/1` (`solve.md` 5.1).
pub const SOLVE_V1_TOOL_IDS: &[&str; 16] = &[
    arxiv::ARXIV_SEARCH,
    sanity::CHECK_EXPRESSION_SANITY,
    CHECK_RUN,
    EXPR_SHOW,
    fs_tools::FS_LIST,
    fs_tools::FS_READ,
    builtin::LEDGER_NOTE,
    proof_dag::LEDGER_QUERY,
    proof_dag::LEDGER_SUPERSEDE,
    literature::LITERATURE_SEARCH,
    "read_paper_lines",
    "search_paper_passages",
    SOLVE_FINISH,
    STEP_RECORD,
    builtin::SYMPY_EVAL,
    tex_audit::TEX_AUDIT,
];

/// Host-registered v1 check names (`solve.md` 5.1; S9 algebra + S10 physics).
pub const SOLVE_V1_CHECK_NAMES: &[&str; 11] = &[
    "identity_symbolic",
    "identity_numeric",
    "series_match",
    "vanishes_to_order",
    "counterexample_search",
    "dimensions",
    "limit_case",
    "symmetry",
    "conservation",
    "convergence",
    "matrix_property",
];

const STEP_ENTRY_TYPES: &[&str; 9] = &[
    "assumption",
    "derivation",
    "result",
    "symbolic_computation",
    "numerical_computation",
    "reference_lookup",
    "verification",
    "correction",
    "final_answer",
];

/// Canonical frozen definition of `expr_show` (`solve.md` 5.1, package S7).
pub fn expr_show_def() -> ToolDef {
    ToolDef {
        id: EXPR_SHOW.into(),
        description: "Inspect a stored Solve expression handle (`name` or `name@version`), \
                      or omit `name` to list all stored handles in this session. Returns \
                      check-process canonical LaTeX, string preview, free symbols, portability, \
                      and `srepr_sha256` (and optionally canonical `srepr`). Read-only."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "maxLength": 128,
                    "description": "Handle name (`foo`) or versioned handle (`foo@2`). Omit to list all handles."
                },
                "version": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Optional handle version when `name` is unversioned (defaults to latest)."
                },
                "form": {
                    "type": "string",
                    "enum": ["latex", "str", "srepr"],
                    "description": "Optional primary representation to include in `text`."
                },
                "include_srepr": {
                    "type": "boolean",
                    "description": "When true, include the canonical `srepr` string (clipped to `max_chars`)."
                },
                "max_chars": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 4000,
                    "description": "Maximum character length of returned text fields (1..=4000, default 4000)."
                }
            },
            "additionalProperties": false
        }),
        effect: EffectClass::ObserveLocal,
        approval_scope: ApprovalScope::Once,
    }
}

/// Canonical frozen definition of `step_record` (`solve.md` 5.1, 7.2, rule R5; wired in S8).
///
/// Intentionally omits any `latex` field (`additionalProperties: false` rejects it per rule R5):
/// rendered formulas always come from stored handles canonicalized by the check process.
pub fn step_record_def() -> ToolDef {
    ToolDef {
        id: STEP_RECORD.into(),
        description: "Record a Solve derivation step in the session ledger with dependencies, \
                      computation receipt references, and optional claims over stored expression \
                      handles. Never accepts raw LaTeX (formulas are rendered from handles by \
                      the host check process)."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "entry_type": {
                    "type": "string",
                    "enum": STEP_ENTRY_TYPES
                },
                "summary": { "type": "string", "maxLength": 500 },
                "details": { "type": "string", "maxLength": 16000 },
                "depends_on": {
                    "type": "array",
                    "items": { "type": "string", "maxLength": 64 }
                },
                "tool_call_refs": {
                    "type": "array",
                    "items": { "type": "string", "maxLength": 200 }
                },
                "claims": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "form": {
                                "type": "string",
                                "enum": ["equation", "inequality", "value", "series"]
                            },
                            "lhs": { "type": "string", "maxLength": 128 },
                            "rhs": { "type": "string", "maxLength": 128 },
                            "relation": {
                                "type": "string",
                                "enum": ["eq", "le", "ge", "approx"]
                            },
                            "order": {
                                "type": "object",
                                "properties": {
                                    "variable": { "type": "string", "maxLength": 64 },
                                    "point": { "type": "string", "maxLength": 128 },
                                    "n": { "type": "integer" }
                                },
                                "required": ["variable", "point", "n"],
                                "additionalProperties": false
                            },
                            "domain": { "type": "string", "maxLength": 500 },
                            "how": { "type": "string", "maxLength": 500 }
                        },
                        "required": ["form", "lhs", "rhs", "relation"],
                        "additionalProperties": false
                    }
                },
                "source_ref": {
                    "type": "object",
                    "properties": {
                        "source_id": { "type": "string", "maxLength": 200 },
                        "path": { "type": "string", "maxLength": 500 },
                        "start_line": { "type": "integer", "minimum": 1 },
                        "end_line": { "type": "integer", "minimum": 1 },
                        "file_sha256": { "type": "string", "maxLength": 64 },
                        "quote_sha256": { "type": "string", "maxLength": 64 },
                        "evidence_seq": { "type": "integer", "minimum": 1 }
                    },
                    "required": [
                        "source_id",
                        "path",
                        "start_line",
                        "end_line",
                        "file_sha256",
                        "quote_sha256",
                        "evidence_seq"
                    ],
                    "additionalProperties": false
                },
                "plan": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "title": { "type": "string", "maxLength": 200 }
                        },
                        "required": ["title"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["entry_type", "summary", "details"],
            "additionalProperties": false
        }),
        effect: EffectClass::ObserveLocal,
        approval_scope: ApprovalScope::Once,
    }
}

/// Canonical frozen definition of `check_run` (`solve.md` 5.1, rules R3 & R18; wired in S9).
///
/// Runs inside the isolated check process under `ExecuteCode` + `ApprovalScope::WorkerSession`
/// keyed to the check process's `WorkerEpoch`. Accepts only check name and parameters (never
/// code, expected values, or outcomes per rule R3).
pub fn check_run_def() -> ToolDef {
    ToolDef {
        id: CHECK_RUN.into(),
        description: "Run a host-registered mathematical check on a recorded step's claim in the \
                      isolated check process. Accepts only the step id, optional claim index, \
                      registered check name, and check parameters (never code, expected values, \
                      or outcomes). Requires approval per caller and check-worker session."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "entry_id": {
                    "type": "string",
                    "maxLength": 64,
                    "description": "Recorded Solve step id (`e-N`)."
                },
                "claim_index": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Zero-based index into the step's `claims` list (defaults to 0)."
                },
                "check": {
                    "type": "string",
                    "enum": SOLVE_V1_CHECK_NAMES,
                    "description": "Registered host check name."
                },
                "params": {
                    "type": "object",
                    "description": "Optional check-specific parameters."
                }
            },
            "required": ["entry_id", "check"],
            "additionalProperties": false
        }),
        effect: EffectClass::ExecuteCode,
        approval_scope: ApprovalScope::WorkerSession,
    }
}

/// Canonical frozen definition of `solve_finish` (`solve.md` 5.1, rule R15; wired in S11).
///
/// Authoritative `status` and done-checklist outcomes are always computed by the host from the
/// ledger; optional `outcome` and `open_items` carry solver notes only.
pub fn solve_finish_def() -> ToolDef {
    ToolDef {
        id: SOLVE_FINISH.into(),
        description: "Finish a Solve run by naming the final answer step and providing a summary. \
                      The host refuses if the closure contains any Failed or Stale step, and \
                      computes the authoritative status, done criteria, and rests-on summary from \
                      the ledger."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "outcome": {
                    "type": "string",
                    "enum": ["finished", "finished_open_items"],
                    "description": "Optional solver-proposed outcome note; authoritative status is computed by the host."
                },
                "final_entry_id": {
                    "type": "string",
                    "maxLength": 64,
                    "description": "Recorded step id (`e-N`) of the final answer."
                },
                "summary_markdown": {
                    "type": "string",
                    "maxLength": 16000,
                    "description": "Concise solver summary (checked by honesty lint against ledger tiers)."
                },
                "open_items": {
                    "type": "array",
                    "items": { "type": "string", "maxLength": 500 },
                    "description": "Optional solver notes on remaining open items."
                }
            },
            "required": ["final_entry_id", "summary_markdown"],
            "additionalProperties": false
        }),
        effect: EffectClass::ObserveLocal,
        approval_scope: ApprovalScope::Once,
    }
}

/// Fail-closed S7 stub for `step_record` until package S8 wires the real handler.
pub fn step_record_stub_handler() -> Arc<dyn ToolHandler> {
    Arc::new(|_: &CallCtx<'_>, _: &Value| Err("step_record is not wired yet (package S8)".to_string()))
}

/// Fail-closed S7 stub for `check_run` until package S9 wires the real handler.
pub fn check_run_stub_handler() -> Arc<dyn ToolHandler> {
    Arc::new(|_: &CallCtx<'_>, _: &Value| Err("check_run is not wired yet (package S9)".to_string()))
}

/// Fail-closed S7 stub for `solve_finish` until package S11 wires the real handler.
pub fn solve_finish_stub_handler() -> Arc<dyn ToolHandler> {
    Arc::new(|_: &CallCtx<'_>, _: &Value| Err("solve_finish is not wired yet (package S11)".to_string()))
}

/// Fallback uncontained epoch used for `check_run` when no live check worker is attached
/// (e.g., static profile enumeration in tests or `cli-route`).
pub(crate) struct FallbackCheckEpoch;

impl WorkerEpoch for FallbackCheckEpoch {
    fn live_epoch(&self) -> Option<u64> {
        None
    }

    fn is_contained(&self) -> bool {
        false
    }
}

/// Complete 16-tool `solve/1` definition list in canonical `id` order (`solve.md` 5.1).
///
/// Reuses 12 `research/1` definitions byte-for-byte and adds the 4 frozen Solve definitions.
pub fn solve_v1_defs() -> Vec<ToolDef> {
    let mut defs = vec![
        // 12 reused `research/1` definitions (exact `def_hash` preserved)
        builtin::sympy_eval_def(),
        builtin::ledger_note_def(),
        proof_dag::ledger_supersede_def(),
        proof_dag::ledger_query_def(),
        paper_lines::def(),
        paper_passages::def(),
        arxiv::arxiv_search_def(),
        literature::literature_search_def(),
        fs_tools::fs_read_def(),
        fs_tools::fs_list_def(),
        sanity::check_expression_sanity_def(),
        tex_audit::tex_audit_def(),
        // 4 frozen Solve definitions
        expr_show_def(),
        step_record_def(),
        check_run_def(),
        solve_finish_def(),
    ];
    defs.sort_by(|a, b| a.id.cmp(&b.id));
    defs
}

/// Register the 10 reused non-host `research/1` tools included in `solve/1`
/// (everything except `sympy_eval` and `ledger_note`, which `register_profile` binds directly).
///
/// Explicitly excludes `doc_ingest`, `ledger_append`, `lean_check`, `search_formal_library`,
/// `fs_write`, `latex_patch`, `latex_compile`, all `record_paper_*` / `read_map_node` /
/// `record_map_reading` / `record_context_cards`, and all `idea_*` tools.
pub(crate) fn register_solve_v1_reused_tools(registry: &mut Registry, env: Arc<ResearchEnv>) {
    fs_tools::register(registry, env.clone());
    tex_audit::register(registry, env.clone());
    arxiv::register(registry, env.clone());
    literature::register(registry, env.clone());
    paper_lines::register(registry, env.clone());
    paper_passages::register(registry, env);
    sanity::register(registry);

    // Reuse `ledger_supersede` and `ledger_query` from `proof_dag` without registering `ledger_append`.
    let mut proof_reg = Registry::new();
    proof_dag::register(&mut proof_reg);
    for id in [proof_dag::LEDGER_SUPERSEDE, proof_dag::LEDGER_QUERY] {
        if let Some((def, handler)) = proof_reg.get(id) {
            registry.register(def.clone(), Arc::clone(handler));
        }
    }
}
