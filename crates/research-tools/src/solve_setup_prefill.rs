//! Deterministic zero-model prefill heuristics for Solve setup (package S4b).
//! Re-exported from `solve_setup` so callers import everything from `research_tools::solve_setup`.

use crate::solve_model::{
    compute_math_sha256, event_kinds, validate_expr_name, DoneCriterion, DoneCriterionKind,
    ReviewPolicy, RoleModel, SolveBudgets, SolveTarget, SymbolSpec, TargetForm, TaskKind,
    MAX_CONVENTIONS_CHARS,
};
use crate::solve_setup::{
    validate_dimension_power, PrefillRequest, SetupDefaults, SolvePrefillResult, VanishesCue,
    CRITERION_TIER_MODEL_AGREED, CRITERION_TIER_NUMERIC, CRITERION_TIER_VERIFIED,
    PROVE_PROSE_NOTICE, SAME_MODEL_WARNING,
};
use crate::solve_status::wording;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const GREEK_SYMBOLS: &[&str] = &[
    "lambda", "phi", "mu", "eps", "epsilon", "varepsilon", "alpha", "beta", "gamma", "delta",
    "omega", "theta", "hbar", "psi", "chi", "kappa", "sigma", "tau", "rho", "nu", "eta", "zeta",
    "xi", "Delta", "Lambda", "Omega", "Phi", "Psi", "Gamma", "Sigma",
];

const POSITIVE_REAL_HINTS: &[&str] = &[
    "lambda", "mu", "m", "M", "hbar", "c", "T", "omega", "kappa", "alpha", "g", "L", "R",
];

const INTEGER_HINTS: &[&str] = &["n", "N", "j", "l"];

/// Guess the task kind deterministically from statement text (`solve.md` 2, 4.1).
pub fn guess_task_kind(statement: &str) -> TaskKind {
    let lower = statement.to_ascii_lowercase();
    let check_cues = [
        "check this derivation",
        "check the derivation",
        "check my derivation",
        "check derivation",
        "check each step",
        "check these steps",
        "find the error",
        "spot the error",
        "audit this derivation",
    ];
    if check_cues.iter().any(|c| lower.contains(c)) {
        return TaskKind::CheckDerivation;
    }

    let calc_cues = [
        "compute",
        "calculate",
        "evaluate",
        "expand",
        "integrate",
        "differentiate",
        "effective potential",
        "closed form",
        "solve for",
        "find the value",
        "determine the",
        "derive the formula",
    ];
    let prove_cues = ["prove", "establish that", "demonstrate that", "show that"];
    let has_calc = calc_cues.iter().any(|c| lower.contains(c));
    let has_prove = prove_cues.iter().any(|c| lower.contains(c));
    if has_prove && !has_calc {
        TaskKind::Prove
    } else {
        TaskKind::Calculate
    }
}

/// Detect `"show <expr> = 0 at order <var>^<n>"` / `"show ... vanishes to order <var>^<n>"` cues.
pub fn detect_show_vanishes_cue(statement: &str) -> Option<VanishesCue> {
    let lower = statement.to_ascii_lowercase();
    let show_idx = lower.find("show ")?;
    let tail = statement[show_idx + 5..].trim();
    let tail_lower = tail.to_ascii_lowercase();
    let order_pos = tail_lower
        .find("at order ")
        .map(|i| (i, 9usize))
        .or_else(|| tail_lower.find("to order ").map(|i| (i, 9usize)))?;
    let before_order = tail[..order_pos.0].trim();
    let after_order = tail[order_pos.0 + order_pos.1..].trim();

    let is_zero_claim = before_order.contains("= 0")
        || before_order.ends_with("=0")
        || before_order.to_ascii_lowercase().contains("vanish");
    if !is_zero_claim {
        return None;
    }
    let expr = before_order
        .split("= 0")
        .next()
        .unwrap_or(before_order)
        .split("=0")
        .next()
        .unwrap_or(before_order)
        .trim_end_matches(|c: char| c.is_whitespace() || c == ',')
        .trim()
        .to_string();
    if expr.is_empty() {
        return None;
    }

    let order_tok = after_order
        .split(|c: char| c.is_whitespace() || matches!(c, '.' | ',' | ';' | ')'))
        .find(|s| !s.is_empty())?
        .trim_start_matches('\\')
        .trim_matches('$');
    let (var_raw, pow_raw) = if let Some((v, p)) = order_tok.split_once('^') {
        (v.trim_matches(['{', '}']), p.trim_matches(['{', '}']))
    } else {
        (order_tok.trim_matches(['{', '}']), "1")
    };
    let var = normalize_symbol_token(var_raw)?;
    let power: i32 = pow_raw.parse().ok()?;
    Some(VanishesCue { expression: expr, order_variable: var, order_power: power })
}

fn normalize_symbol_token(raw: &str) -> Option<String> {
    let cleaned = raw.trim().trim_start_matches('\\');
    let mapped = match cleaned {
        "varepsilon" => "eps",
        "varphi" => "phi",
        "vartheta" => "theta",
        other => other,
    };
    validate_expr_name(mapped).ok()?;
    Some(mapped.to_string())
}

fn default_assumptions_for(sym: &str) -> Vec<String> {
    if POSITIVE_REAL_HINTS.contains(&sym) {
        vec!["real".into(), "positive".into()]
    } else if INTEGER_HINTS.contains(&sym) {
        vec!["integer".into()]
    } else {
        vec!["real".into()]
    }
}

/// Deterministic symbol scanner over TeX and plain-math statements (`solve.md` 2 & S4b).
pub fn scan_statement_symbols(statement: &str, defaults: Option<&SetupDefaults>) -> Vec<SymbolSpec> {
    let override_map: BTreeMap<&str, &[String]> = defaults
        .map(|d| d.symbols.iter().map(|s| (s.name.as_str(), s.assumptions.as_slice())).collect())
        .unwrap_or_default();

    let mut found: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut push_sym = |raw: &str| {
        if let Some(norm) = normalize_symbol_token(raw) {
            if seen.insert(norm.clone()) {
                found.push(norm);
            }
        }
    };

    let bytes = statement.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i] as char;
        if ch == '\\' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && (bytes[end] as char).is_ascii_alphabetic() {
                end += 1;
            }
            if end > start {
                let cmd = &statement[start..end];
                if GREEK_SYMBOLS.contains(&cmd) || matches!(cmd, "varphi" | "vartheta") {
                    push_sym(cmd);
                }
            }
            i = end.max(i + 1);
            continue;
        }
        if ch.is_ascii_alphabetic() {
            let start = i;
            let mut end = i + 1;
            while end < bytes.len()
                && ((bytes[end] as char).is_ascii_alphanumeric() || bytes[end] == b'_')
            {
                end += 1;
            }
            let tok = &statement[start..end];
            if GREEK_SYMBOLS.contains(&tok)
                || (tok.len() == 1
                    && is_math_context_single_letter(statement, start, end)
                    && !matches!(tok, "a" | "A" | "I" | "e" | "i" | "O"))
            {
                push_sym(tok);
            }
            i = end;
            continue;
        }
        i += 1;
    }

    found
        .into_iter()
        .map(|name| {
            let assumptions = override_map
                .get(name.as_str())
                .map(|slice| slice.to_vec())
                .unwrap_or_else(|| default_assumptions_for(&name));
            SymbolSpec { name, assumptions }
        })
        .collect()
}

fn is_math_context_single_letter(s: &str, start: usize, end: usize) -> bool {
    let before = s[..start].trim_end();
    let after = s[end..].trim_start();
    let next_ch = after.chars().next();
    let prev_ch = before.chars().last();
    matches!(next_ch, Some('=' | '^' | '_' | '+' | '-' | '*' | '/' | '<' | '>' | '('))
        || matches!(prev_ch, Some('=' | '^' | '+' | '-' | '*' | '/' | '<' | '>' | '(' | '$'))
}

pub(crate) fn parse_compact_dimension(raw: &str) -> Option<BTreeMap<String, Value>> {
    let mut out = BTreeMap::new();
    for part in raw.split(['*', ' ']).map(str::trim).filter(|p| !p.is_empty()) {
        let (base, pow_str) = match part.split_once('^') {
            Some((b, p)) => (b.trim(), p.trim()),
            None => (part, "1"),
        };
        if base.is_empty()
            || base.len() > 32
            || !base.as_bytes()[0].is_ascii_alphabetic()
            || !base.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            return None;
        }
        let val = if let Ok(n) = pow_str.parse::<i64>() {
            json!(n)
        } else {
            let v = Value::String(pow_str.to_string());
            validate_dimension_power(&v).ok()?;
            v
        };
        out.insert(base.to_string(), val);
    }
    (!out.is_empty()).then_some(out)
}

/// True when solver and verifier point at the same non-empty `(provider, model)` pair
/// or when solver is set and verifier is blank (`solve.md` D2).
pub fn is_same_model(solver: Option<&RoleModel>, verifier: Option<&RoleModel>) -> bool {
    let Some(s) = solver else { return false };
    if s.provider.trim().is_empty() || s.model.trim().is_empty() {
        return false;
    }
    match verifier {
        None => true,
        Some(v) if v.provider.trim().is_empty() || v.model.trim().is_empty() => true,
        Some(v) => {
            s.provider.trim().eq_ignore_ascii_case(v.provider.trim())
                && s.model.trim().eq_ignore_ascii_case(v.model.trim())
        }
    }
}

/// Build the default done checklist per task kind (`solve.md` 2, D4, D13, D21).
pub fn default_done_criteria(
    task_kind: TaskKind,
    vanishes_cue: Option<&VanishesCue>,
    has_dimensions: bool,
    allow_numeric_target: bool,
) -> Vec<DoneCriterion> {
    let target_tier = if allow_numeric_target {
        CRITERION_TIER_NUMERIC
    } else {
        CRITERION_TIER_VERIFIED
    };
    match task_kind {
        TaskKind::Calculate => {
            let mut list = vec![
                DoneCriterion {
                    id: "final_result_tier".into(),
                    kind: DoneCriterionKind::FinalTierAtLeast,
                    params: json!({ "tier": target_tier }),
                },
                DoneCriterion {
                    id: "closure_model_agreed".into(),
                    kind: DoneCriterionKind::ClosureTierAtLeast,
                    params: json!({ "tier": CRITERION_TIER_MODEL_AGREED }),
                },
                DoneCriterion {
                    id: "no_open_flags".into(),
                    kind: DoneCriterionKind::NoOpenFlags,
                    params: json!({}),
                },
            ];
            if has_dimensions {
                list.push(DoneCriterion {
                    id: "dimensions_check".into(),
                    kind: DoneCriterionKind::NamedCheckPassed,
                    params: json!({ "check": "dimensions", "on": "final" }),
                });
            }
            if let Some(cue) = vanishes_cue {
                list.push(DoneCriterion {
                    id: "show_vanishes_to_order".into(),
                    kind: DoneCriterionKind::NamedCheckPassed,
                    params: json!({
                        "check": "vanishes_to_order",
                        "on": "final",
                        "expression": cue.expression,
                        "order_variable": cue.order_variable,
                        "order_power": cue.order_power,
                    }),
                });
            }
            list
        }
        TaskKind::Prove => vec![
            DoneCriterion {
                id: "final_different_model_review".into(),
                kind: DoneCriterionKind::FinalTierAtLeast,
                params: json!({
                    "tier": CRITERION_TIER_MODEL_AGREED,
                    "require_different_model": true,
                }),
            },
            DoneCriterion {
                id: "closure_model_agreed".into(),
                kind: DoneCriterionKind::ClosureTierAtLeast,
                params: json!({
                    "tier": CRITERION_TIER_MODEL_AGREED,
                    "require_different_model": true,
                }),
            },
            DoneCriterion {
                id: "no_open_flags".into(),
                kind: DoneCriterionKind::NoOpenFlags,
                params: json!({}),
            },
        ],
        TaskKind::CheckDerivation => vec![
            DoneCriterion {
                id: "closure_checked".into(),
                kind: DoneCriterionKind::ClosureTierAtLeast,
                params: json!({ "tier": target_tier }),
            },
            DoneCriterion {
                id: "no_open_flags".into(),
                kind: DoneCriterionKind::NoOpenFlags,
                params: json!({}),
            },
            DoneCriterion {
                id: "confirm_transcriptions".into(),
                kind: DoneCriterionKind::UserConfirms,
                params: json!({ "text": "Confirm side-by-side TeX transcriptions" }),
            },
        ],
    }
}

/// Deterministic zero-model prefill from statement text, pinned conventions, and optional F16a defaults.
pub fn prefill_solve_setup(req: &PrefillRequest) -> SolvePrefillResult {
    let task_kind = req.task_kind_override.unwrap_or_else(|| guess_task_kind(&req.statement));
    let symbols = scan_statement_symbols(&req.statement, req.defaults.as_ref());
    let vanishes_cue = detect_show_vanishes_cue(&req.statement);

    let mut conv_parts: Vec<String> = Vec::new();
    let pinned = req.pinned_conventions.trim();
    if !pinned.is_empty() {
        conv_parts.push(pinned.to_string());
    }
    if let Some(defs) = &req.defaults {
        for extra in &defs.conventions {
            let trimmed = extra.trim();
            if !trimmed.is_empty() && !conv_parts.iter().any(|p| p == trimmed) {
                conv_parts.push(trimmed.to_string());
            }
        }
    }
    let mut conventions = conv_parts.join("\n");
    if conventions.chars().count() > MAX_CONVENTIONS_CHARS {
        conventions = conventions.chars().take(MAX_CONVENTIONS_CHARS).collect();
    }

    let mut dimensions = BTreeMap::new();
    if let Some(defs) = &req.defaults {
        let sym_names: BTreeSet<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        for (sym, dim) in &defs.dimensions {
            if sym_names.contains(sym.as_str()) {
                dimensions.insert(sym.clone(), dim.clone());
            }
        }
    }

    let target = match task_kind {
        TaskKind::Calculate => SolveTarget {
            description: "Final calculated expression".into(),
            form: if req.statement.to_ascii_lowercase().contains("order ")
                || req.statement.to_ascii_lowercase().contains("series")
            {
                TargetForm::Series
            } else {
                TargetForm::ClosedForm
            },
            order: vanishes_cue
                .as_ref()
                .map(|c| json!({ "variable": c.order_variable, "n": c.order_power })),
            digits: None,
        },
        TaskKind::Prove => SolveTarget {
            description: "Structured proof reviewed by an independent model".into(),
            form: TargetForm::Proof,
            order: None,
            digits: None,
        },
        TaskKind::CheckDerivation => SolveTarget {
            description: "Step-by-step derivation check".into(),
            form: TargetForm::PerStepVerdict,
            order: None,
            digits: None,
        },
    };

    let done = default_done_criteria(
        task_kind,
        vanishes_cue.as_ref(),
        !dimensions.is_empty(),
        req.allow_numeric_target,
    );
    let same_model = is_same_model(req.solver.as_ref(), req.verifier.as_ref());
    let same_model_warning = same_model.then(|| SAME_MODEL_WARNING.to_string());
    let prove_notice = (task_kind == TaskKind::Prove).then(|| PROVE_PROSE_NOTICE.to_string());
    let math_sha256 = compute_math_sha256(&symbols, &dimensions, &[], &conventions);

    let mut target_tier_labels = BTreeMap::new();
    target_tier_labels.insert(CRITERION_TIER_VERIFIED.into(), wording::BADGE_VERIFIED.into());
    target_tier_labels.insert(
        CRITERION_TIER_NUMERIC.into(),
        wording::BADGE_CHECKED_NUMERICALLY.into(),
    );
    target_tier_labels.insert(
        CRITERION_TIER_MODEL_AGREED.into(),
        wording::BADGE_MODEL_AGREED.into(),
    );

    SolvePrefillResult {
        task_kind,
        target,
        symbols,
        conventions,
        dimensions,
        done,
        budgets: SolveBudgets::default(),
        review: ReviewPolicy::default(),
        tool_profile: event_kinds::SOLVE_TOOL_PROFILE_V1.to_string(),
        prove_notice,
        same_model_warning,
        same_model,
        vanishes_cue,
        math_sha256,
        target_tier_labels,
    }
}
