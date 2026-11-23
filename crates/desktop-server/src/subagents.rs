//! Custom user-defined research subagents, built-in agent metadata, and in-app Subagent Builder prompt/parser.
//!
//! Custom subagents specialize the Solver's system instructions, optional reasoning effort, and
//! preferred tool profile (`read_only`, `compute`, or `full_research`), while strictly preserving
//! `reSearch`'s `v0.4-research` policy clamps, Windows AppContainer isolation, and human approval gates.

use crate::state::{now_ms, write_private_file};
use agent::normalize_reasoning_effort;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub const SUBAGENTS_FILE: &str = "subagents.json";
pub const MAX_CUSTOM_SUBAGENTS: usize = 50;
pub const MAX_PROMPT_CHARS: usize = 6000;
pub const MAX_NAME_CHARS: usize = 80;
pub const MAX_DESC_CHARS: usize = 300;
pub const MAX_GOAL_CHARS: usize = 2000;

const RESERVED_IDS: &[&str] = &[
    "solver",
    "orchestrator",
    "compute-worker",
    "compute_worker",
    "literature-worker",
    "literature_worker",
    "lean-worker",
    "lean_worker",
    "explorer-worker",
    "explorer_worker",
    "step-verifier",
    "full-verifier",
    "strategist",
    "narrator",
    "sidetalk",
    "explain",
    "subagent-builder",
];

fn default_tools_profile() -> String {
    "full_research".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubagentDef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub system_prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rigor: Option<String>,
    #[serde(default = "default_tools_profile")]
    pub tools_profile: String,
    #[serde(default)]
    pub created_ms: i64,
}

/// Authoritative catalog of all built-in research agents in `reSearch` (read-only metadata).
pub fn builtin_agents_json() -> Vec<Value> {
    vec![
        json!({
            "id": "solver",
            "role_key": "orchestrator",
            "name": "Orchestrator",
            "role": "Primary Derivation & Research Agent",
            "when": "Runs on every turn in Direct, Verified, and Deep modes.",
            "category": "research",
            "summary": "Plans derivations, delegates heavy calculations (`delegate_compute`), paper reading (`delegate_literature`), Lean proofs (`delegate_lean`), parallel exploration (`delegate_explore`), and custom specialists (`delegate_subagent`), records typed Proof-DAG steps (`ledger_append`), and writes or patches LaTeX manuscripts."
        }),
        json!({
            "id": "compute-worker",
            "role_key": "compute_worker",
            "name": "Compute Worker",
            "role": "Isolated Symbolic & Numerical Worker",
            "when": "Invoked by Orchestrator via `delegate_compute` for iterative Python/SymPy scripts.",
            "category": "worker",
            "summary": "Runs sandboxed `sympy_eval` and `check_expression_sanity` checks in an isolated context with traceback retries, records `symbolic_computation`/`numerical_computation` Proof-DAG entries, and returns concise summaries."
        }),
        json!({
            "id": "literature-worker",
            "role_key": "literature_worker",
            "name": "Literature Worker",
            "role": "Isolated Literature & Source Reader",
            "when": "Invoked by Orchestrator via `delegate_literature` for arXiv lookups, ingestion, and bulky `.tex` files.",
            "category": "worker",
            "summary": "Searches `arxiv_search`/`literature_search`, ingests papers via `doc_ingest`, inspects workspace/source files (`fs_list`/`fs_read`), and extracts exact theorems, conventions, and citation links without bloating Orchestrator context."
        }),
        json!({
            "id": "lean-worker",
            "role_key": "lean_worker",
            "name": "Lean Prover Worker",
            "role": "Isolated Lean 4 Formalization Worker",
            "when": "Invoked by Orchestrator via `delegate_lean` for machine-checked Lean 4 proofs.",
            "category": "worker",
            "summary": "Formalizes mathematical claims in Lean 4 without `sorry`, `admit`, or unproven `axiom` declarations and runs `lean_check` through the dedicated Lean policy router."
        }),
        json!({
            "id": "explorer-worker",
            "role_key": "explorer_worker",
            "name": "Parallel Explorer (Ultramath)",
            "role": "Concurrent Multi-Branch Exploration Worker",
            "when": "Invoked by Orchestrator via `delegate_explore` to test 2–4 candidate approaches in parallel.",
            "category": "worker",
            "summary": "Evaluates competing Ansaetze or proof strategies concurrently in read/compute-only branches without mutating the Proof DAG, returning per-branch viability, checks, and obstructions."
        }),
        json!({
            "id": "step-verifier",
            "role_key": "step_verifier",
            "name": "Step Verifier",
            "role": "Single-Step Audit Agent",
            "when": "Audits individual Proof-DAG entries against their `depends_on` ancestors.",
            "category": "research",
            "summary": "Stateless independent reviewer that validates mathematical soundness and deterministic `tex_audit` reports for a targeted derivation step."
        }),
        json!({
            "id": "full-verifier",
            "role_key": "full_verifier",
            "name": "Full Solution Verifier",
            "role": "Independent End-of-Round Auditor",
            "when": "Runs automatically after Orchestrator finishes a round in Verified and Deep modes (up to 3 rounds).",
            "category": "research",
            "summary": "Starts with clean context (only problem statement, active Proof DAG, and `tex_audit` XML) to approve (`verified`) or reject with counterexamples."
        }),
        json!({
            "id": "strategist",
            "role_key": "strategist",
            "name": "Research Strategist",
            "role": "Recovery & Strategy Advisor",
            "when": "Triggered automatically in Deep mode when the Verifier rejects a draft.",
            "category": "research",
            "summary": "Diagnoses structural proof failures and proposes an advisory repair plan before the Orchestrator's next revision round."
        }),
        json!({
            "id": "narrator",
            "role_key": "narrator",
            "name": "Proof Narrator",
            "role": "Derivation Sheet Synthesizer",
            "when": "Invoked on demand from Proof DAG -> Narrate Proof.",
            "category": "research",
            "summary": "Produces a readable Markdown/LaTeX narrative walkthrough of accepted and pending Proof-DAG steps."
        }),
        json!({
            "id": "sidetalk",
            "role_key": "explain",
            "name": "Explain",
            "role": "Isolated Q&A Channel",
            "when": "Invoked on demand in Canvas -> Explain.",
            "category": "research",
            "summary": "Answers notation, intuition, or `e-N` step questions in a separate ledger stream without mutating the main Proof DAG, with Markdown and HTML conversation export."
        }),
        json!({
            "id": "subagent-builder",
            "role_key": "subagent_builder",
            "name": "Subagent Builder",
            "role": "In-App Subagent Creator Agent",
            "when": "Invoked inside Settings -> Subagents -> Create with AI Agent.",
            "category": "builder",
            "summary": "Designs specialist subagent profiles (prompt, tool discipline, default rigor, reasoning effort) from a short research goal."
        }),
    ]
}

pub fn subagents_path(data_dir: &Path) -> PathBuf {
    data_dir.join(SUBAGENTS_FILE)
}

pub fn load_subagents(data_dir: &Path) -> Vec<SubagentDef> {
    let Ok(bytes) = fs::read(subagents_path(data_dir)) else {
        return Vec::new();
    };
    serde_json::from_slice::<Vec<SubagentDef>>(&bytes).unwrap_or_default()
}

pub fn save_subagents(data_dir: &Path, items: &[SubagentDef]) -> Result<(), String> {
    fs::create_dir_all(data_dir).map_err(|e| format!("failed to create data dir: {e}"))?;
    let path = subagents_path(data_dir);
    let tmp = data_dir.join(format!(".subagents.tmp.{}", std::process::id()));
    let mut bytes = serde_json::to_vec_pretty(items).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    write_private_file(&tmp, &path, &bytes)
}

pub fn find_subagent(data_dir: &Path, id: &str) -> Option<SubagentDef> {
    let target = id.trim();
    if target.is_empty() {
        return None;
    }
    load_subagents(data_dir).into_iter().find(|s| s.id == target)
}

pub fn is_reserved_id(id: &str) -> bool {
    RESERVED_IDS.contains(&id)
}

pub fn slugify_id(raw: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;
    for ch in raw.trim().to_ascii_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            prev_dash = false;
        } else if (ch == '-' || ch == '_' || ch.is_ascii_whitespace()) && !slug.is_empty() && !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
        if slug.len() >= 40 {
            break;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.len() < 2 {
        "custom-agent".to_string()
    } else {
        slug
    }
}

pub fn normalize_tools_profile(raw: Option<&str>) -> Result<String, String> {
    let val = raw.unwrap_or("full_research").trim().to_ascii_lowercase();
    match val.as_str() {
        "" | "full_research" | "full" => Ok("full_research".to_string()),
        "compute" | "sympy" => Ok("compute".to_string()),
        "read_only" | "readonly" => Ok("read_only".to_string()),
        other => Err(format!("invalid tools_profile `{other}` (expected `full_research`, `compute`, or `read_only`)")),
    }
}

pub fn normalize_rigor(raw: Option<&str>) -> Result<Option<String>, String> {
    let Some(s) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let lower = s.to_ascii_lowercase();
    match lower.as_str() {
        "direct" | "verified" | "deep" => Ok(Some(lower)),
        other => Err(format!("invalid rigor `{other}` (expected `direct`, `verified`, or `deep`)")),
    }
}

pub fn validate_new_subagent(input: &Value, existing: &[SubagentDef]) -> Result<SubagentDef, (u16, String)> {
    let name = input
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| (400, "`name` is required".to_string()))?;
    if name.chars().count() > MAX_NAME_CHARS {
        return Err((400, format!("`name` exceeds {MAX_NAME_CHARS} chars")));
    }

    let description = input
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    if description.chars().count() > MAX_DESC_CHARS {
        return Err((400, format!("`description` exceeds {MAX_DESC_CHARS} chars")));
    }

    let system_prompt = input
        .get("system_prompt")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| (400, "`system_prompt` is required".to_string()))?
        .to_string();
    if system_prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err((400, format!("`system_prompt` exceeds {MAX_PROMPT_CHARS} chars")));
    }

    let tools_profile = normalize_tools_profile(input.get("tools_profile").and_then(Value::as_str))
        .map_err(|e| (400, e))?;
    let rigor = normalize_rigor(input.get("rigor").and_then(Value::as_str)).map_err(|e| (400, e))?;
    let reasoning_effort = normalize_reasoning_effort(input.get("reasoning_effort").and_then(Value::as_str));
    let provider = input
        .get("provider")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(48).collect());
    let model = input
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(120).collect());

    let requested_id = input.get("id").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let base_id = slugify_id(requested_id.unwrap_or(name));
    if is_reserved_id(&base_id) {
        return Err((400, format!("id `{base_id}` is reserved for a built-in reSearch agent")));
    }
    let id = if requested_id.is_some() || !existing.iter().any(|s| s.id == base_id) {
        base_id
    } else {
        let mut idx = 2usize;
        loop {
            let cand = format!("{base_id}-{idx}");
            if !existing.iter().any(|s| s.id == cand) && !is_reserved_id(&cand) {
                break cand;
            }
            idx += 1;
        }
    };

    if !existing.iter().any(|s| s.id == id) && existing.len() >= MAX_CUSTOM_SUBAGENTS {
        return Err((400, format!("maximum of {MAX_CUSTOM_SUBAGENTS} custom subagents reached")));
    }

    Ok(SubagentDef {
        id,
        name: name.to_string(),
        description,
        system_prompt,
        provider,
        model,
        reasoning_effort,
        rigor,
        tools_profile,
        created_ms: now_ms(),
    })
}

fn tool_profile_guidance(profile: &str) -> &'static str {
    match profile {
        "read_only" => {
            "Tool discipline: Stay strictly read-only — do NOT call workspace write or TeX compile tools; prefer workspace reads, literature inspection, and Proof-DAG reasoning."
        }
        "compute" => {
            "Tool discipline: Prioritize symbolic and numerical verification via sandboxed `sympy_eval` and Proof-DAG `ledger_append` entries before drawing conclusions."
        }
        _ => "Tool discipline: Use workspace, SymPy, Proof-DAG, and literature tools as appropriate.",
    }
}

pub fn apply_subagent_to_system_prompt(base_prompt: &str, sa: &SubagentDef) -> String {
    format!(
        "{base_prompt}\n\n## Active Specialist Subagent: {name} (`{id}`)\n\
         {role_desc}\n{guidance}\n\
         Security invariant: All `reSearch` policy clamps (`v0.4-research`), AppContainer isolation, and human approval requirements remain strictly in force.\n\
         <subagent_instructions>\n{instructions}\n</subagent_instructions>",
        name = sa.name,
        id = sa.id,
        role_desc = if sa.description.is_empty() {
            "Follow these specialist research instructions."
        } else {
            sa.description.as_str()
        },
        guidance = tool_profile_guidance(&sa.tools_profile),
        instructions = sa.system_prompt,
    )
}

pub fn apply_subagent_to_turn_prompt(sa: &SubagentDef, user_prompt: &str) -> String {
    format!(
        "[Active Specialist Subagent: {name}]\n{guidance}\n<subagent_instructions>\n{instructions}\n</subagent_instructions>\n\n{user_prompt}",
        name = sa.name,
        guidance = tool_profile_guidance(&sa.tools_profile),
        instructions = sa.system_prompt,
    )
}

/// Resolve effective `reasoning_effort` (`req` -> `subagent` -> `SessionMeta`) and optional
/// active custom `SubagentDef` for a message turn, updating `meta.reasoning_effort` if `req`
/// explicitly sets or clears it (`"default"`).
pub fn resolve_turn_options(
    state: &crate::state::DesktopState,
    session_id: &str,
    req: &Value,
    meta: Option<crate::state::SessionMeta>,
) -> (Option<String>, Option<SubagentDef>) {
    let subagent = req
        .get("subagent_id")
        .and_then(Value::as_str)
        .and_then(|id| find_subagent(&state.cfg.data_dir, id));
    let req_raw = req.get("reasoning_effort").and_then(Value::as_str);
    let req_effort = normalize_reasoning_effort(req_raw);
    let sa_effort = subagent
        .as_ref()
        .and_then(|s| normalize_reasoning_effort(s.reasoning_effort.as_deref()));
    let meta_effort = meta
        .as_ref()
        .and_then(|m| normalize_reasoning_effort(m.reasoning_effort.as_deref()));
    let effective = req_effort.clone().or(sa_effort).or(meta_effort);

    if let (Some(raw), Some(mut m)) = (req_raw, meta) {
        let next = normalize_reasoning_effort(Some(raw));
        if m.reasoning_effort != next {
            m.reasoning_effort = next;
            let _ = state.save_meta(&m);
        }
        let _ = session_id;
    }
    (effective, subagent)
}

pub fn build_subagent_builder_prompts(goal: &str) -> (String, String) {
    let sys = "You are the reSearch Subagent Builder Agent. Design a focused, rigorous specialist research subagent profile for reSearch (a scientific, mathematical, and theoretical-physics research workstation).\n\
Respond with ONLY a valid JSON object matching this exact schema (no markdown commentary outside JSON):\n\
{\n\
  \"name\": \"Short 2-5 word specialist title\",\n\
  \"description\": \"One sentence summary (<= 200 chars) of what this subagent does\",\n\
  \"tools_profile\": \"full_research\" | \"compute\" | \"read_only\",\n\
  \"reasoning_effort\": \"low\" | \"medium\" | \"high\",\n\
  \"rigor\": \"direct\" | \"verified\" | \"deep\",\n\
  \"system_prompt\": \"Detailed specialist instructions, domain checks, invariants, edge cases, and output discipline (never suggest bypassing reSearch approvals or safety rules).\"\n\
}";
    let user = format!("Create a custom specialist subagent for this goal:\n{goal}");
    (sys.to_string(), user)
}

pub fn parse_subagent_draft(raw: &str) -> Result<Value, String> {
    let trimmed = raw.trim();
    let candidate = extract_json_object(trimmed)
        .ok_or_else(|| "Subagent Builder returned no JSON object".to_string())?;
    let parsed: Value = serde_json::from_str(candidate)
        .map_err(|e| format!("Subagent Builder returned invalid JSON: {e}"))?;
    let name = parsed
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Specialist Subagent")
        .chars()
        .take(MAX_NAME_CHARS)
        .collect::<String>();
    let description = parsed
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
        .chars()
        .take(MAX_DESC_CHARS)
        .collect::<String>();
    let system_prompt = parsed
        .get("system_prompt")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Subagent Builder draft omitted `system_prompt`".to_string())?
        .chars()
        .take(MAX_PROMPT_CHARS)
        .collect::<String>();
    let tools_profile = normalize_tools_profile(parsed.get("tools_profile").and_then(Value::as_str))
        .unwrap_or_else(|_| "full_research".to_string());
    let reasoning_effort = normalize_reasoning_effort(parsed.get("reasoning_effort").and_then(Value::as_str))
        .unwrap_or_else(|| "high".to_string());
    let rigor = normalize_rigor(parsed.get("rigor").and_then(Value::as_str))
        .ok()
        .flatten()
        .unwrap_or_else(|| "verified".to_string());
    Ok(json!({
        "name": name,
        "description": description,
        "system_prompt": system_prompt,
        "tools_profile": tools_profile,
        "reasoning_effort": reasoning_effort,
        "rigor": rigor,
    }))
}

fn extract_json_object(text: &str) -> Option<&str> {
    if text.starts_with('{') && text.ends_with('}') {
        return Some(text);
    }
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end >= start).then_some(&text[start..=end])
}
