//! Platform-neutral Worker Protocol v1/v2 framing, role validation, and staging manifest.
//!
//! Protocol 2 is strictly additive over protocol 1:
//! - A protocol 1 solver worker omits `role` in its hello and does not accept `exprs_in`.
//! - A protocol 2 worker reports `protocol: 2` and `role: "solver" | "check"`.
//! - `WorkerRole::Check` runs only host-shipped checks from `python/checks/` and
//!   never accepts `evaluate`.

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Current worker protocol version spoken by `python/worker.py`.
pub const PROTOCOL_VERSION: u64 = 2;

/// Lowest protocol version still accepted for backwards compatibility (`WorkerRole::Solver`).
pub const MIN_SUPPORTED_PROTOCOL_VERSION: u64 = 1;

/// Helper scripts beside `python/worker.py` staged into the AppContainer runtime.
pub const STAGED_HELPER_FILES: &[&str] = &[
    "solver_calc.py",
    "symbolic_validation.py",
    "numerical_validation.py",
];

/// Host-shipped check modules under `python/checks/` staged into the runtime.
/// Relative POSIX paths; joined component-by-component onto the filesystem path.
pub const STAGED_CHECK_FILES: &[&str] = &[
    "checks/__init__.py",
    "checks/srepr_safe.py",
    "checks/algebra.py",
    "checks/algebra_probe.py",
    "checks/physics.py",
    "checks/physics_helpers.py",
    "checks/physics_matrix.py",
];

/// Process role of a spawned `worker.py` instance (`solve.md` 4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkerRole {
    /// Runs model-authored `sympy_eval` code (`evaluate`, `shutdown`).
    Solver,
    /// Runs only host-shipped `python/checks/` code (`canonicalize`, `check`, `shutdown`).
    Check,
}

impl WorkerRole {
    pub fn as_str(self) -> &'static str {
        match self {
            WorkerRole::Solver => "solver",
            WorkerRole::Check => "check",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "solver" => Some(WorkerRole::Solver),
            "check" => Some(WorkerRole::Check),
            _ => None,
        }
    }

    /// Extra CLI arguments appended when launching `worker.py`.
    /// `Solver` returns `&[]` so older protocol 1 scripts remain launchable unchanged.
    pub fn cli_args(self) -> &'static [&'static str] {
        match self {
            WorkerRole::Solver => &[],
            WorkerRole::Check => &["--role", "check"],
        }
    }
}

impl fmt::Display for WorkerRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Validated start-up handshake sent by `worker.py`.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkerHello {
    pub protocol: u64,
    pub role: WorkerRole,
    pub pid: Option<u32>,
    pub checks_sha256: Option<String>,
    pub raw: Value,
}

/// Validate a worker start-up JSON message against `expected_role`.
///
/// Accepts protocol 1 (for `WorkerRole::Solver`, where `role` may be omitted) and
/// protocol 2 (where `role` must match `expected_role`).
pub fn validate_hello(value: &Value, expected_role: WorkerRole) -> Result<WorkerHello, String> {
    if value.get("ready").and_then(Value::as_bool) != Some(true) {
        return Err(format!("worker hello did not report ready=true: {value}"));
    }
    let protocol = value
        .get("protocol")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("worker hello missing numeric protocol: {value}"))?;
    if !(MIN_SUPPORTED_PROTOCOL_VERSION..=PROTOCOL_VERSION).contains(&protocol) {
        return Err(format!(
            "unsupported worker protocol {protocol} (supported: {MIN_SUPPORTED_PROTOCOL_VERSION}..={PROTOCOL_VERSION})"
        ));
    }

    let role = if protocol == 1 {
        if expected_role != WorkerRole::Solver {
            return Err(format!(
                "worker protocol 1 does not support role `{expected_role}` (requires protocol >= 2)"
            ));
        }
        match value.get("role").and_then(Value::as_str) {
            None | Some("solver") => WorkerRole::Solver,
            Some(other) => return Err(format!("unexpected role `{other}` in protocol 1 hello")),
        }
    } else {
        let raw_role = value
            .get("role")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("protocol {protocol} hello missing string `role`"))?;
        let parsed = WorkerRole::parse(raw_role)
            .ok_or_else(|| format!("unknown worker role `{raw_role}`"))?;
        if parsed != expected_role {
            return Err(format!(
                "worker role mismatch: expected `{expected_role}`, got `{parsed}`"
            ));
        }
        parsed
    };

    let pid = value
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok());
    let checks_sha256 = value
        .get("checks_sha256")
        .and_then(Value::as_str)
        .map(str::to_string);

    Ok(WorkerHello { protocol, role, pid, checks_sha256, raw: value.clone() })
}

/// Build `evaluate` request params. Omits `exprs_in` when `None` for protocol 1 compatibility.
pub fn build_evaluate_params(
    code: &str,
    exprs_in: Option<&BTreeMap<String, String>>,
) -> Value {
    match exprs_in {
        None => json!({ "code": code }),
        Some(map) => json!({ "code": code, "exprs_in": map }),
    }
}

/// Build `canonicalize` request params for a single `srepr` string (check role).
pub fn build_canonicalize_params(srepr: &str) -> Value {
    json!({ "srepr": srepr })
}

/// Build `canonicalize` request params for a batch of named `srepr` strings (check role).
pub fn build_canonicalize_batch_params(exprs: &BTreeMap<String, String>) -> Value {
    json!({ "exprs": exprs })
}

/// Build `check` request params for a registered host check (check role).
pub fn build_check_params(
    name: &str,
    handles: &BTreeMap<String, String>,
    params: Value,
) -> Value {
    json!({ "name": name, "handles": handles, "params": params })
}

/// Free symbol with explicit SymPy domain assumptions (matches `solve_model::SymbolSpec`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreeSymbol {
    pub name: String,
    pub assumptions: Vec<String>,
}

/// Canonicalized expression returned by the check process (`solve.md` 4.1 & 4.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalExpr {
    pub portable: bool,
    pub srepr: String,
    pub srepr_sha256: String,
    pub latex: String,
    pub str_preview: String,
    pub free_symbols: Vec<FreeSymbol>,
    pub error: Option<String>,
    pub rejected_constructor: Option<String>,
}

/// Parse a `canonicalize` result payload into a typed [`CanonicalExpr`].
pub fn parse_canonical_expr(value: &Value) -> Result<CanonicalExpr, String> {
    let portable = value
        .get("portable")
        .and_then(Value::as_bool)
        .ok_or_else(|| "canonicalize result missing boolean `portable`".to_string())?;
    let srepr = value.get("srepr").and_then(Value::as_str).unwrap_or_default().to_string();
    let srepr_sha256 =
        value.get("srepr_sha256").and_then(Value::as_str).unwrap_or_default().to_string();
    let latex = value.get("latex").and_then(Value::as_str).unwrap_or_default().to_string();
    let str_preview =
        value.get("str_preview").and_then(Value::as_str).unwrap_or_default().to_string();
    let error = value.get("error").and_then(Value::as_str).map(str::to_string);
    let rejected_constructor =
        value.get("rejected_constructor").and_then(Value::as_str).map(str::to_string);

    let mut free_symbols = Vec::new();
    if let Some(arr) = value.get("free_symbols").and_then(Value::as_array) {
        for item in arr {
            let Some(name) = item.get("name").and_then(Value::as_str) else {
                return Err("free_symbols entry missing string `name`".to_string());
            };
            let assumptions = item
                .get("assumptions")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .unwrap_or_default();
            free_symbols.push(FreeSymbol { name: name.to_string(), assumptions });
        }
    }

    if portable && (srepr.is_empty() || srepr_sha256.is_empty()) {
        return Err("portable canonicalize result missing `srepr` or `srepr_sha256`".to_string());
    }

    Ok(CanonicalExpr {
        portable,
        srepr,
        srepr_sha256,
        latex,
        str_preview,
        free_symbols,
        error,
        rejected_constructor,
    })
}

/// Extract protocol 2 `exprs_out` (`{name: srepr}`) from an `evaluate` result object.
/// Returns an empty map for protocol 1 results that lack `exprs_out`.
pub fn extract_exprs_out(result: &Value) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Some(obj) = result.get("exprs_out").and_then(Value::as_object) {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                out.insert(k.clone(), s.to_string());
            }
        }
    }
    out
}

fn join_rel_posix(base: &Path, rel_posix: &str) -> PathBuf {
    let mut path = base.to_path_buf();
    for segment in rel_posix.split('/') {
        if !segment.is_empty() {
            path.push(segment);
        }
    }
    path
}

/// Read `worker.py`, optional helper modules, and optional `checks/` modules relative to
/// `worker_script`'s directory. Returns `(rel_posix_path, bytes)` pairs in deterministic order.
pub fn collect_staged_scripts(worker_script: &Path) -> io::Result<Vec<(String, Vec<u8>)>> {
    let base_dir = worker_script.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "worker_script has no parent directory")
    })?;
    let mut scripts = vec![("worker.py".to_string(), fs::read(worker_script)?)];
    for &rel in STAGED_HELPER_FILES.iter().chain(STAGED_CHECK_FILES.iter()) {
        let src = join_rel_posix(base_dir, rel);
        match fs::read(&src) {
            Ok(bytes) => scripts.push((rel.to_string(), bytes)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    Ok(scripts)
}

/// Write collected staged scripts under `target_dir`, creating subdirectories (such as `checks/`).
pub fn write_staged_scripts(target_dir: &Path, scripts: &[(String, Vec<u8>)]) -> io::Result<()> {
    for (rel, bytes) in scripts {
        let dst = join_rel_posix(target_dir, rel);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(dst, bytes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_hello_accepts_protocol_1_and_2_and_enforces_role() {
        let v1 = json!({ "ready": true, "protocol": 1, "pid": 42 });
        let h1 = validate_hello(&v1, WorkerRole::Solver).unwrap();
        assert_eq!(h1.protocol, 1);
        assert_eq!(h1.role, WorkerRole::Solver);
        assert_eq!(h1.pid, Some(42));
        // Protocol 1 cannot run Check role
        assert!(validate_hello(&v1, WorkerRole::Check).is_err());

        let v2_solver = json!({ "ready": true, "protocol": 2, "role": "solver", "pid": 7 });
        let h2 = validate_hello(&v2_solver, WorkerRole::Solver).unwrap();
        assert_eq!(h2.protocol, 2);
        assert_eq!(h2.role, WorkerRole::Solver);
        assert!(validate_hello(&v2_solver, WorkerRole::Check).is_err());

        let v2_check = json!({
            "ready": true,
            "protocol": 2,
            "role": "check",
            "pid": 9,
            "checks_sha256": "abc123"
        });
        let hc = validate_hello(&v2_check, WorkerRole::Check).unwrap();
        assert_eq!(hc.role, WorkerRole::Check);
        assert_eq!(hc.checks_sha256.as_deref(), Some("abc123"));
        assert!(validate_hello(&v2_check, WorkerRole::Solver).is_err());

        // Unsupported protocol or ready=false rejected
        assert!(validate_hello(&json!({ "ready": true, "protocol": 3, "role": "solver" }), WorkerRole::Solver).is_err());
        assert!(validate_hello(&json!({ "ready": false, "protocol": 2, "role": "solver" }), WorkerRole::Solver).is_err());
        assert!(validate_hello(&json!({ "ready": true, "protocol": 2 }), WorkerRole::Solver).is_err());
    }

    #[test]
    fn evaluate_params_keep_protocol_1_shape_when_exprs_in_is_none() {
        let p1 = build_evaluate_params("1 + 1", None);
        assert_eq!(p1, json!({ "code": "1 + 1" }));
        assert!(p1.get("exprs_in").is_none());

        let mut exprs = BTreeMap::new();
        exprs.insert("e1".to_string(), "Symbol('x')".to_string());
        let p2 = build_evaluate_params("expr('e1')", Some(&exprs));
        assert_eq!(p2["code"], json!("expr('e1')"));
        assert_eq!(p2["exprs_in"]["e1"], json!("Symbol('x')"));
    }

    #[test]
    fn parse_canonical_expr_handles_portable_and_rejected() {
        let ok = json!({
            "portable": true,
            "srepr": "Pow(Symbol('x', real=True), Integer(2))",
            "srepr_sha256": "deadbeef",
            "latex": "x^{2}",
            "str_preview": "x**2",
            "free_symbols": [{ "name": "x", "assumptions": ["real"] }]
        });
        let parsed = parse_canonical_expr(&ok).unwrap();
        assert!(parsed.portable);
        assert_eq!(parsed.latex, "x^{2}");
        assert_eq!(parsed.free_symbols.len(), 1);
        assert_eq!(parsed.free_symbols[0].assumptions, vec!["real"]);

        let bad = json!({
            "portable": false,
            "srepr": "",
            "srepr_sha256": "",
            "latex": "",
            "str_preview": "",
            "free_symbols": [],
            "error": "disallowed constructor 'eval'",
            "rejected_constructor": "eval"
        });
        let parsed_bad = parse_canonical_expr(&bad).unwrap();
        assert!(!parsed_bad.portable);
        assert_eq!(parsed_bad.rejected_constructor.as_deref(), Some("eval"));
    }

    #[test]
    fn extract_exprs_out_handles_v1_and_v2_results() {
        let v1 = json!({ "value": "2", "stdout": "", "stderr": "", "truncated": false });
        assert!(extract_exprs_out(&v1).is_empty());

        let v2 = json!({
            "value": "x**2",
            "exprs_out": { "e1": "Pow(Symbol('x'), Integer(2))" }
        });
        let out = extract_exprs_out(&v2);
        assert_eq!(out.get("e1").map(String::as_str), Some("Pow(Symbol('x'), Integer(2))"));
    }

    #[test]
    fn collect_and_write_staged_scripts_includes_checks_package() {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let worker_py = repo_root.join("python").join("worker.py");
        let scripts = collect_staged_scripts(&worker_py).unwrap();
        let names: Vec<&str> = scripts.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"worker.py"));
        assert!(names.contains(&"solver_calc.py"));
        assert!(names.contains(&"checks/__init__.py"));
        assert!(names.contains(&"checks/srepr_safe.py"));
        assert!(names.contains(&"checks/algebra.py"));
        assert!(names.contains(&"checks/algebra_probe.py"));
        assert!(names.contains(&"checks/physics.py"));
        assert!(names.contains(&"checks/physics_helpers.py"));
        assert!(names.contains(&"checks/physics_matrix.py"));

        let dst = std::env::temp_dir().join(format!("worker-stage-proto-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dst);
        write_staged_scripts(&dst, &scripts).unwrap();
        assert!(dst.join("worker.py").is_file());
        assert!(dst.join("checks").join("__init__.py").is_file());
        assert!(dst.join("checks").join("srepr_safe.py").is_file());
        assert!(dst.join("checks").join("algebra.py").is_file());
        assert!(dst.join("checks").join("algebra_probe.py").is_file());
        assert!(dst.join("checks").join("physics.py").is_file());
        assert!(dst.join("checks").join("physics_helpers.py").is_file());
        assert!(dst.join("checks").join("physics_matrix.py").is_file());
        let _ = fs::remove_dir_all(&dst);
    }

    #[test]
    fn role_cli_args_keep_solver_backwards_compatible() {
        assert!(WorkerRole::Solver.cli_args().is_empty());
        assert_eq!(WorkerRole::Check.cli_args(), &["--role", "check"]);
    }
}
