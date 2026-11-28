//! Parameter validation, R3 guards, and check invocation preparation for `solve_checks` (S9 + S10).

use super::CHECK_RUN;
use crate::policy_preset::{SOLVE_CHECK_CALLER, SOLVE_VERIFIER_CALLER};
use crate::solve_defs::SOLVE_V1_CHECK_NAMES;
use crate::solve_exprs::load_handle_srepr;
use crate::solve_model::{compute_claim_sha256, CheckOutcome, CheckRequester, HandleRef};
use crate::solve_state::SolveState;
use ledger::Ledger;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

const ALLOWED_CHECK_RUN_KEYS: &[&str] = &["entry_id", "claim_index", "check", "params"];
const MAX_CHECK_PARAMS_BYTES: usize = 16 * 1024;
const MAX_CHECK_PARAMS_DEPTH: usize = 8;

/// R3 keys strictly forbidden anywhere inside caller-supplied check parameters.
const FORBIDDEN_RECURSIVE_KEYS: &[&str] = &["code", "outcome", "expected", "expected_value"];

/// Host-supplied or R3-forbidden top-level parameter keys never accepted from callers.
const FORBIDDEN_TOP_LEVEL_KEYS: &[&str] = &[
    "code",
    "outcome",
    "expected",
    "expected_value",
    "handles",
    "symbols",
    "dimensions",
    "assumptions",
    "registry_sha256",
    "claim_sha256",
    "math_sha256",
    "lhs_code_sha256",
    "rhs_code_sha256",
    "target",
    "evidence",
];

const SHARED_PARAM_KEYS: &[&str] = &["points", "digits", "domain", "relation", "seed", "tolerance"];

const MATRIX_PROPERTIES: &[&str] = &[
    "hermitian",
    "symmetric",
    "unitary",
    "orthogonal",
    "positive_definite",
    "traceless",
    "invertible",
];

pub(super) struct ParsedCheckArgs {
    pub entry_id: String,
    pub claim_index: usize,
    pub check: String,
    pub user_params: Value,
}

pub(super) struct PreparedCheck {
    pub claim_sha256: String,
    pub math_sha256: String,
    pub handles: BTreeMap<String, String>,
    pub full_params: Value,
    pub lhs_code_sha256: Option<String>,
    pub rhs_code_sha256: Option<String>,
}

pub(super) fn parse_check_run_args(args: &Value) -> Result<ParsedCheckArgs, String> {
    let obj = args
        .as_object()
        .ok_or_else(|| format!("{CHECK_RUN} arguments must be a JSON object"))?;
    for key in obj.keys() {
        if !ALLOWED_CHECK_RUN_KEYS.contains(&key.as_str()) {
            return Err(format!("unknown field `{key}` in {CHECK_RUN} (rule R3)"));
        }
    }
    let entry_id = obj
        .get("entry_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 64)
        .ok_or("`entry_id` must be a non-empty string")?
        .to_string();
    let claim_index = match obj.get("claim_index") {
        None | Some(Value::Null) => 0usize,
        Some(v) => v
            .as_u64()
            .ok_or("`claim_index` must be a non-negative integer")? as usize,
    };
    let check = obj
        .get("check")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| SOLVE_V1_CHECK_NAMES.contains(s))
        .ok_or("`check` must name a registered Solve check")?
        .to_string();
    let user_params = match obj.get("params") {
        None | Some(Value::Null) => json!({}),
        Some(v) if v.is_object() => v.clone(),
        Some(_) => return Err("`params` must be a JSON object".to_string()),
    };
    validate_check_params(&check, &user_params)?;
    Ok(ParsedCheckArgs {
        entry_id,
        claim_index,
        check,
        user_params,
    })
}

fn validate_check_params(check: &str, params: &Value) -> Result<(), String> {
    let obj = params
        .as_object()
        .ok_or_else(|| "`params` must be a JSON object".to_string())?;
    let serialized = serde_json::to_string(params).map_err(|e| e.to_string())?;
    if serialized.len() > MAX_CHECK_PARAMS_BYTES {
        return Err(format!(
            "`params` exceeds {MAX_CHECK_PARAMS_BYTES}-byte limit"
        ));
    }
    if json_depth(params) > MAX_CHECK_PARAMS_DEPTH {
        return Err(format!(
            "`params` nesting exceeds depth {MAX_CHECK_PARAMS_DEPTH}"
        ));
    }
    for key in obj.keys() {
        if FORBIDDEN_TOP_LEVEL_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "forbidden field `{key}` in `params` for `{check}` (rule R3)"
            ));
        }
    }
    reject_forbidden_nested_keys(params)?;

    let allowed = allowed_keys_for_check(check);
    for key in obj.keys() {
        if !SHARED_PARAM_KEYS.contains(&key.as_str()) && !allowed.contains(&key.as_str()) {
            return Err(format!("unknown param `{key}` for check `{check}`"));
        }
    }

    if let Some(tol) = obj.get("tolerance") {
        let Some(f) = tol.as_f64() else {
            return Err("`tolerance` must be a finite positive number".to_string());
        };
        if !f.is_finite() || f <= 0.0 {
            return Err("`tolerance` must be finite and positive".to_string());
        }
    }

    match check {
        "limit_case" => validate_limit_case_params(obj)?,
        "symmetry" => validate_symmetry_params(obj)?,
        "conservation" => validate_conservation_params(obj)?,
        "convergence" => validate_convergence_params(obj)?,
        "matrix_property" => validate_matrix_property_params(obj)?,
        _ => {}
    }
    Ok(())
}

fn allowed_keys_for_check(check: &str) -> &'static [&'static str] {
    match check {
        "identity_symbolic" | "identity_numeric" | "counterexample_search" => &["tol"],
        "series_match" | "vanishes_to_order" => &["order", "variable", "point", "n", "tol"],
        "dimensions" => &[],
        "limit_case" => &[
            "variable",
            "var",
            "point",
            "to",
            "target_point",
            "direction",
            "side",
            "target_handle",
        ],
        "symmetry" => &[
            "subs",
            "transform",
            "map",
            "kind",
            "variable",
            "variables",
            "sign",
            "factor",
            "side",
        ],
        "conservation" => &[
            "mode",
            "subs",
            "variable",
            "time_var",
            "side",
            "space_vars",
            "q_vars",
            "p_vars",
        ],
        "convergence" => &[
            "side",
            "variable",
            "var",
            "levels",
            "axis",
            "tail",
            "atol",
            "rtol",
            "work_dps",
            "use_rhs_as_reference",
            "direction",
        ],
        "matrix_property" => &["property", "properties", "side"],
        _ => &[],
    }
}

fn validate_limit_case_params(obj: &Map<String, Value>) -> Result<(), String> {
    let has_var = obj
        .get("variable")
        .or_else(|| obj.get("var"))
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty());
    if !has_var {
        return Err("`limit_case` requires non-empty `variable`".to_string());
    }
    let pt = obj
        .get("point")
        .or_else(|| obj.get("to"))
        .or_else(|| obj.get("target_point"));
    if !pt.is_some_and(|v| !v.is_null()) {
        return Err("`limit_case` requires `point`".to_string());
    }
    if let Some(dir) = obj.get("direction") {
        let s = dir.as_str().map(str::trim).unwrap_or_default();
        if !matches!(s, "+" | "-" | "+-") {
            return Err(format!("invalid `limit_case` direction `{s}`"));
        }
    }
    if let Some(side) = obj.get("side") {
        let s = side.as_str().map(str::trim).unwrap_or_default();
        if !matches!(s, "lhs" | "rhs") {
            return Err("`limit_case` `side` must be `lhs` or `rhs`".to_string());
        }
    }
    if let Some(th) = obj.get("target_handle") {
        let raw = th
            .as_str()
            .ok_or("`limit_case` `target_handle` must be a `name@version` string")?;
        HandleRef::parse(raw.trim())?;
    }
    Ok(())
}

fn validate_symmetry_params(obj: &Map<String, Value>) -> Result<(), String> {
    let subs = obj
        .get("subs")
        .or_else(|| obj.get("transform"))
        .or_else(|| obj.get("map"));
    let has_subs = match subs {
        Some(v) if v.as_object().is_some_and(|m| !m.is_empty()) => true,
        Some(_) => return Err("`symmetry` `subs` must be a non-empty JSON object".to_string()),
        None => false,
    };
    let kind = obj.get("kind").and_then(Value::as_str).map(str::trim);
    if let Some(k) = kind {
        if !matches!(k, "even" | "odd" | "exchange") {
            return Err(format!("unsupported `symmetry` kind `{k}`"));
        }
    }
    if !has_subs && kind.is_none() {
        return Err("`symmetry` requires `subs` or `kind`".to_string());
    }
    if let Some(vars) = obj.get("variables") {
        let ok = vars
            .as_array()
            .is_some_and(|a| a.len() == 2 && a.iter().all(|v| v.as_str().is_some_and(|s| !s.trim().is_empty())));
        if !ok {
            return Err("`symmetry` `variables` must contain two symbol names".to_string());
        }
    }
    if let Some(sign) = obj.get("sign") {
        let ok = sign.as_i64().is_some_and(|n| n == 1 || n == -1)
            || sign.as_str().is_some_and(|s| matches!(s.trim(), "+1" | "-1" | "1"));
        if !ok {
            return Err("`symmetry` `sign` must be +1 or -1".to_string());
        }
    }
    if let Some(side) = obj.get("side") {
        let s = side.as_str().map(str::trim).unwrap_or_default();
        if !matches!(s, "lhs" | "rhs" | "both") {
            return Err("`symmetry` `side` must be `lhs`, `rhs`, or `both`".to_string());
        }
    }
    Ok(())
}

fn validate_conservation_params(obj: &Map<String, Value>) -> Result<(), String> {
    if let Some(mode) = obj.get("mode") {
        let m = mode.as_str().map(str::trim).unwrap_or_default();
        if !matches!(
            m,
            "commutator"
                | "derivative"
                | "time_derivative"
                | "continuity"
                | "divergence"
                | "poisson_bracket"
                | "residual"
        ) {
            return Err(format!("unsupported `conservation` mode `{m}`"));
        }
    }
    if let Some(side) = obj.get("side") {
        let s = side.as_str().map(str::trim).unwrap_or_default();
        if !matches!(s, "lhs" | "rhs") {
            return Err("`conservation` `side` must be `lhs` or `rhs`".to_string());
        }
    }
    for key in ["space_vars", "q_vars", "p_vars"] {
        if let Some(val) = obj.get(key) {
            validate_symbol_list(val, key)?;
        }
    }
    if let Some(subs) = obj.get("subs") {
        if !subs.is_object() {
            return Err("`conservation` `subs` must be a JSON object".to_string());
        }
    }
    Ok(())
}

fn validate_convergence_params(obj: &Map<String, Value>) -> Result<(), String> {
    if let Some(side) = obj.get("side") {
        let s = side.as_str().map(str::trim).unwrap_or_default();
        if !matches!(s, "lhs" | "rhs") {
            return Err("`convergence` `side` must be `lhs` or `rhs`".to_string());
        }
    }
    if let Some(axis) = obj.get("axis") {
        let a = axis.as_str().map(str::trim).unwrap_or_default();
        if !matches!(a, "resolution" | "precision") {
            return Err("`convergence` `axis` must be `resolution` or `precision`".to_string());
        }
    }
    if let Some(dir) = obj.get("direction") {
        let d = dir.as_str().map(str::trim).unwrap_or_default();
        if !matches!(d, "n->oo" | "h->0") {
            return Err("`convergence` `direction` must be `n->oo` or `h->0`".to_string());
        }
    }
    if let Some(levels) = obj.get("levels") {
        let arr = levels
            .as_array()
            .filter(|a| (2..=32).contains(&a.len()))
            .ok_or("`convergence` `levels` must be an array of 2..=32 entries")?;
        if !arr.iter().all(Value::is_number) {
            return Err("`convergence` `levels` entries must be numeric".to_string());
        }
    }
    if let Some(tail) = obj.get("tail") {
        let n = tail.as_u64().ok_or("`convergence` `tail` must be 1..=16")?;
        if !(1..=16).contains(&n) {
            return Err("`convergence` `tail` must be 1..=16".to_string());
        }
    }
    if let Some(dps) = obj.get("work_dps") {
        let n = dps.as_u64().ok_or("`convergence` `work_dps` must be 20..=256")?;
        if !(20..=256).contains(&n) {
            return Err("`convergence` `work_dps` must be 20..=256".to_string());
        }
    }
    Ok(())
}

fn validate_matrix_property_params(obj: &Map<String, Value>) -> Result<(), String> {
    if let Some(side) = obj.get("side") {
        let s = side.as_str().map(str::trim).unwrap_or_default();
        if !matches!(s, "lhs" | "rhs" | "both") {
            return Err("`matrix_property` `side` must be `lhs`, `rhs`, or `both`".to_string());
        }
    }
    let mut saw_prop = false;
    if let Some(prop) = obj.get("property") {
        let p = prop.as_str().map(str::trim).unwrap_or_default();
        if !MATRIX_PROPERTIES.contains(&p) {
            return Err(format!("unsupported `matrix_property` `{p}`"));
        }
        saw_prop = true;
    }
    if let Some(props) = obj.get("properties") {
        let arr = props
            .as_array()
            .filter(|a| !a.is_empty() && a.len() <= MATRIX_PROPERTIES.len())
            .ok_or("`matrix_property` `properties` must be a non-empty array")?;
        for item in arr {
            let p = item.as_str().map(str::trim).unwrap_or_default();
            if !MATRIX_PROPERTIES.contains(&p) {
                return Err(format!("unsupported `matrix_property` `{p}`"));
            }
        }
        saw_prop = true;
    }
    if !saw_prop {
        return Err("`matrix_property` requires `property` or `properties`".to_string());
    }
    Ok(())
}

fn validate_symbol_list(val: &Value, key: &str) -> Result<(), String> {
    let arr = val
        .as_array()
        .filter(|a| !a.is_empty() && a.len() <= 16)
        .ok_or_else(|| format!("`{key}` must be an array of 1..=16 symbol names"))?;
    for item in arr {
        let ok = item.as_str().is_some_and(|s| !s.trim().is_empty());
        if !ok {
            return Err(format!("`{key}` elements must be non-empty symbol strings"));
        }
    }
    Ok(())
}

fn reject_forbidden_nested_keys(val: &Value) -> Result<(), String> {
    match val {
        Value::Object(map) => {
            for (k, v) in map {
                if FORBIDDEN_RECURSIVE_KEYS.contains(&k.as_str()) {
                    return Err(format!("forbidden nested key `{k}` in `params` (rule R3)"));
                }
                reject_forbidden_nested_keys(v)?;
            }
        }
        Value::Array(arr) => {
            for v in arr {
                reject_forbidden_nested_keys(v)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn json_depth(val: &Value) -> usize {
    match val {
        Value::Object(map) => 1 + map.values().map(json_depth).max().unwrap_or(0),
        Value::Array(arr) => 1 + arr.iter().map(json_depth).max().unwrap_or(0),
        _ => 1,
    }
}

pub(super) fn prepare_check_invocation(
    ledger: &Ledger,
    state: &SolveState,
    parsed: &ParsedCheckArgs,
) -> Result<PreparedCheck, String> {
    let setup = state
        .setup
        .as_ref()
        .ok_or("cannot run `check_run` before `solve.setup` has been recorded")?;
    let entry = state
        .entry(&parsed.entry_id)
        .ok_or_else(|| format!("unknown Solve step `{}`", parsed.entry_id))?;
    if entry.superseded_by.is_some() {
        return Err(format!("step `{}` is superseded", parsed.entry_id));
    }
    let claim = entry.claims.get(parsed.claim_index).ok_or_else(|| {
        format!(
            "step `{}` has no claim at index {}",
            parsed.entry_id, parsed.claim_index
        )
    })?;
    let claim_sha256 = match &entry.claim_sha256 {
        Some(h) if !h.is_empty() => h.clone(),
        _ => compute_claim_sha256(&entry.claims, |h| {
            state.handles.get(h).map(|e| e.srepr_sha256.as_str())
        })?,
    };

    let mut handles = BTreeMap::new();
    for (side, href) in [("lhs", &claim.lhs), ("rhs", &claim.rhs)] {
        let srepr = load_portable_handle(ledger, state, href, &format!("claim[{side}]"))?;
        handles.insert(side.into(), srepr);
    }

    if parsed.check == "limit_case" {
        if let Some(raw_target) = parsed.user_params.get("target_handle").and_then(Value::as_str) {
            let th = HandleRef::parse(raw_target.trim())?;
            let target_srepr = load_portable_handle(ledger, state, &th, "params.target_handle")?;
            handles.insert("target".into(), target_srepr);
        }
    }

    let code_hash = |h| {
        let expr = state.handles.get(h)?;
        state
            .receipts
            .get(&expr.produced_by.call_id)
            .and_then(|r| r.code_sha256.clone())
    };
    let lhs_code_sha256 = code_hash(&claim.lhs);
    let rhs_code_sha256 = code_hash(&claim.rhs);

    let mut full_params = parsed.user_params.as_object().cloned().unwrap_or_default();
    full_params.insert("symbols".into(), json!(setup.symbols));
    full_params.insert("dimensions".into(), json!(setup.dimensions));
    full_params.insert("assumptions".into(), json!(setup.assumptions));
    full_params
        .entry("relation".to_string())
        .or_insert_with(|| json!(claim.relation));
    if let Some(domain) = &claim.domain {
        full_params
            .entry("domain".to_string())
            .or_insert_with(|| json!(domain));
    }
    if let Some(order) = &claim.order {
        full_params
            .entry("order".to_string())
            .or_insert_with(|| json!(order));
    }

    Ok(PreparedCheck {
        claim_sha256,
        math_sha256: setup.math_sha256.clone(),
        handles,
        full_params: Value::Object(full_params),
        lhs_code_sha256,
        rhs_code_sha256,
    })
}

fn load_portable_handle(
    ledger: &Ledger,
    state: &SolveState,
    href: &HandleRef,
    context: &str,
) -> Result<String, String> {
    let expr = state
        .handles
        .get(href)
        .ok_or_else(|| format!("unknown handle `{href}` in {context}"))?;
    let srepr = load_handle_srepr(ledger, expr)?
        .ok_or_else(|| format!("handle `{href}` in {context} is non-portable"))?;
    if let Some(uneval) = expr.unevaluated_srepr.as_deref().filter(|s| !s.is_empty()) {
        return Ok(uneval.to_string());
    }
    Ok(srepr)
}

pub(super) fn attach_code_hashes(prepared: &PreparedCheck, evidence: &mut Value) {
    let Some(obj) = evidence.as_object_mut() else {
        return;
    };
    if let Some(h) = &prepared.lhs_code_sha256 {
        obj.entry("lhs_code_sha256".to_string())
            .or_insert_with(|| json!(h));
    }
    if let Some(h) = &prepared.rhs_code_sha256 {
        obj.entry("rhs_code_sha256".to_string())
            .or_insert_with(|| json!(h));
    }
}

pub(super) fn parse_check_outcome(val: Option<&Value>) -> Result<CheckOutcome, String> {
    let v = val.ok_or("check process reply missing `outcome`")?;
    serde_json::from_value(v.clone()).map_err(|e| format!("invalid check `outcome`: {e}"))
}

pub(super) fn requester_for_caller(caller: &str) -> CheckRequester {
    if caller == SOLVE_CHECK_CALLER {
        CheckRequester::Host
    } else if caller == SOLVE_VERIFIER_CALLER {
        CheckRequester::Verifier
    } else if caller == "user" {
        CheckRequester::User
    } else {
        CheckRequester::Solver
    }
}
