//! Solve versioned expression handle store and `expr_show` tool (package S7;
//! `solve.md` 4.1, 4.3, 5.1, rules R3, R5, R6).
//!
//! Outbox expressions emitted by protocol-2 `sympy_eval` are validated, canonicalized
//! through S6's isolated check process (`srepr_safe.py`), stored as content-addressed
//! blobs in the ledger, and recorded as `solve.expr` v1 events compatible with S2's
//! [`SolveState::from_events`]. Non-parseable or rejected expressions are recorded with
//! `portable: false` without crashing the turn and without leaking raw solver LaTeX.

pub use crate::solve_exprs_show::{expr_show_handler, run_expr_show};
use crate::solve_model::{
    event_kinds, HandleRef, ReceiptRef, SolveExpr, SymbolSpec, MAX_STR_PREVIEW_CHARS,
};
use crate::solve_state::SolveState;
use ledger::{sha256_hex, Error as LedgerError, Ledger, GENESIS_HASH};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

/// Maximum number of versioned expression handles stored per Solve run (`solve.md` 9 S7, 11).
pub const MAX_EXPRS_PER_RUN: usize = 100;
/// Maximum cumulative canonical `srepr` bytes stored across portable handles per Solve run (2 MiB).
pub const MAX_EXPR_SREPR_BYTES_PER_RUN: usize = 2 * 1024 * 1024;
/// Maximum character length stored for canonical `latex` on a `solve.expr` event (matches `worker.py`).
pub const MAX_EXPR_LATEX_CHARS: usize = 64_000;
/// Maximum character length returned by `expr_show` text fields (`solve.md` 5.1).
pub const MAX_EXPR_SHOW_CHARS: usize = 4_000;
/// Schema version for `solve.expr` ledger events (`solve.md` 4.1).
pub const SOLVE_EXPR_SCHEMA_VERSION: u32 = 1;

const RESERVED_EXPR_NAMES: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class",
    "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if",
    "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try",
    "while", "with", "yield", "constructor", "prototype", "toString", "valueOf", "eval", "exec",
    "open", "expr", "save_expr", "load_expr", "sympy", "sp", "mpmath", "numpy", "np",
];

/// Validate a Solve expression handle name: ASCII identifier `^[A-Za-z_][A-Za-z0-9_]{0,63}$`,
/// rejecting dunder (`__*`) and reserved keyword/helper names (`solve.md` 4.1, 11; S7).
pub fn validate_expr_name(name: &str) -> Result<(), String> {
    crate::solve_model::validate_expr_name(name)?;
    if name.starts_with("__") {
        return Err(format!("expression name `{name}` is reserved (dunder prefix `__`)"));
    }
    if RESERVED_EXPR_NAMES.contains(&name) {
        return Err(format!("expression name `{name}` is reserved"));
    }
    Ok(())
}

/// UTF-8 safe prefix clip to at most `max_chars` Unicode scalar values.
pub fn clip_utf8_chars(text: &str, max_chars: usize) -> (String, bool) {
    let mut chars = text.chars();
    let clipped: String = chars.by_ref().take(max_chars).collect();
    (clipped, chars.next().is_some())
}

/// Canonicalization response from S6's isolated check process (`python/checks/srepr_safe.py`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalizedExpr {
    pub portable: bool,
    #[serde(default)]
    pub srepr: String,
    #[serde(default)]
    pub srepr_sha256: String,
    #[serde(default)]
    pub latex: String,
    #[serde(default)]
    pub str_preview: String,
    #[serde(default)]
    pub free_symbols: Vec<SymbolSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejected_constructor: Option<String>,
}

/// Boundary to S6's check-role Python process (`Worker::canonicalize_expr` or
/// `python/worker.py --role check --stdio`) for parsing and rendering `srepr` strings.
pub trait CheckCanonicalizer: Send + Sync {
    fn canonicalize(&self, raw_srepr: &str) -> Result<CanonicalizedExpr, String>;
}

/// One harvested expression recorded as a `solve.expr` v1 ledger event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarvestedExpr {
    pub handle: HandleRef,
    pub seq: u64,
    pub expr: SolveExpr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejected_constructor: Option<String>,
}

/// One outbox entry skipped before writing `solve.expr` (invalid name/type or per-run cap reached).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedExpr {
    pub name: String,
    pub reason: String,
}

/// Outcome of harvesting one `sympy_eval` outbox into the session ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarvestReport {
    pub harvested: Vec<HarvestedExpr>,
    pub skipped: Vec<SkippedExpr>,
    pub total_handles_in_run: usize,
    pub total_srepr_bytes_in_run: usize,
}

/// Extract protocol-2 `exprs_out` (`{ name: raw_srepr }`) from a `sympy_eval` result object.
pub fn extract_exprs_out(sympy_result: &Value) -> BTreeMap<String, String> {
    extract_exprs_out_checked(sympy_result).0
}

fn extract_exprs_out_checked(sympy_result: &Value) -> (BTreeMap<String, String>, Vec<SkippedExpr>) {
    let mut out = BTreeMap::new();
    let mut skipped = Vec::new();
    match sympy_result.get("exprs_out") {
        None | Some(Value::Null) => {}
        Some(Value::Object(obj)) => {
            for (k, v) in obj {
                match v.as_str() {
                    Some(s) => {
                        out.insert(k.clone(), s.to_string());
                    }
                    None => skipped.push(SkippedExpr {
                        name: k.clone(),
                        reason: "exprs_out value must be a string srepr".into(),
                    }),
                }
            }
        }
        Some(_) => skipped.push(SkippedExpr {
            name: "exprs_out".into(),
            reason: "exprs_out must be a JSON object mapping handle names to srepr strings".into(),
        }),
    }
    (out, skipped)
}

struct PendingExprWrite {
    handle: HandleRef,
    expr: SolveExpr,
    srepr_bytes: Option<Vec<u8>>,
    error: Option<String>,
    rejected_constructor: Option<String>,
}

/// Harvest protocol-2 `exprs_out` expressions from one `sympy_eval` call into the session ledger.
///
/// Uses verified session reads (`read_verified_session`) and head-pinned appends (`append_at_head`)
/// so concurrent writers cannot double-issue handle versions or bypass per-run caps (`MAX_EXPRS_PER_RUN`,
/// `MAX_EXPR_SREPR_BYTES_PER_RUN`). All canonicalization completes before any ledger write.
pub fn harvest_exprs_out(
    ledger: &Ledger,
    session: &str,
    produced_by: ReceiptRef,
    raw_exprs: &BTreeMap<String, String>,
    canonicalizer: &dyn CheckCanonicalizer,
) -> Result<HarvestReport, String> {
    let (mut head_hash, events) = match ledger.read_verified_session(session) {
        Ok((report, evs)) => (report.head_hash, evs),
        Err(LedgerError::UnknownSession(_)) => (GENESIS_HASH.to_string(), Vec::new()),
        Err(e) => return Err(format!("cannot verify session `{session}` before harvesting: {e}")),
    };
    let state = SolveState::from_events(&events);

    let mut total_handles = state.handles.len();
    let mut total_srepr_bytes = count_existing_srepr_bytes(ledger, &state)?;
    let mut latest_versions: BTreeMap<String, u32> = BTreeMap::new();
    for handle in state.handles.keys() {
        let entry = latest_versions.entry(handle.name.clone()).or_insert(0);
        if handle.version > *entry {
            *entry = handle.version;
        }
    }

    let mut pending = Vec::new();
    let mut skipped = Vec::new();

    for (name, raw_srepr) in raw_exprs {
        if let Err(reason) = validate_expr_name(name) {
            skipped.push(SkippedExpr { name: name.clone(), reason });
            continue;
        }
        if total_handles >= MAX_EXPRS_PER_RUN {
            skipped.push(SkippedExpr {
                name: name.clone(),
                reason: format!("per-run handle cap reached ({MAX_EXPRS_PER_RUN} handles)"),
            });
            continue;
        }
        if raw_srepr.len() > MAX_EXPR_SREPR_BYTES_PER_RUN {
            skipped.push(SkippedExpr {
                name: name.clone(),
                reason: format!(
                    "expression srepr ({} bytes) exceeds per-run byte cap ({MAX_EXPR_SREPR_BYTES_PER_RUN} bytes)",
                    raw_srepr.len()
                ),
            });
            continue;
        }
        let prev_ver = latest_versions.get(name).copied().unwrap_or(0);
        let Some(next_version) = prev_ver.checked_add(1) else {
            skipped.push(SkippedExpr {
                name: name.clone(),
                reason: format!("version overflow for handle `{name}`"),
            });
            continue;
        };

        let (solve_expr, srepr_bytes, error, rejected_constructor, added_bytes) =
            match canonicalizer.canonicalize(raw_srepr) {
                Ok(canon) if canon.portable => {
                    match prepare_portable_expr(name, next_version, &produced_by, canon, total_srepr_bytes) {
                        PortableOutcome::Ready(expr, bytes) => {
                            let len = bytes.len();
                            (*expr, Some(bytes), None, None, len)
                        }
                        PortableOutcome::CapExceeded(reason) => {
                            skipped.push(SkippedExpr { name: name.clone(), reason });
                            continue;
                        }
                        PortableOutcome::NonPortable(reason) => {
                            (non_portable_expr(name, next_version, &produced_by), None, Some(reason), None, 0)
                        }
                    }
                }
                Ok(canon) => (
                    non_portable_expr(name, next_version, &produced_by),
                    None,
                    canon.error.or_else(|| Some("expression rejected by check process".into())),
                    canon.rejected_constructor,
                    0,
                ),
                Err(err) => (
                    non_portable_expr(name, next_version, &produced_by),
                    None,
                    Some(format!("check-process canonicalization failed: {err}")),
                    None,
                    0,
                ),
            };

        let handle = HandleRef::new(name.clone(), next_version)?;
        total_handles += 1;
        total_srepr_bytes = total_srepr_bytes.saturating_add(added_bytes);
        latest_versions.insert(name.clone(), next_version);
        pending.push(PendingExprWrite {
            handle,
            expr: solve_expr,
            srepr_bytes,
            error,
            rejected_constructor,
        });
    }

    let mut harvested = Vec::with_capacity(pending.len());
    for item in pending {
        if let Some(bytes) = &item.srepr_bytes {
            let stored = ledger
                .put_blob(bytes)
                .map_err(|e| format!("failed to store srepr blob for `{}`: {e}", item.handle))?;
            if stored != item.expr.srepr_blob {
                return Err(format!("srepr blob hash mismatch while storing `{}`", item.handle));
            }
        }
        let payload = serde_json::to_value(&item.expr)
            .map_err(|e| format!("failed to serialize solve.expr: {e}"))?;
        let appended = ledger
            .append_at_head(
                session,
                event_kinds::SOLVE_EXPR,
                SOLVE_EXPR_SCHEMA_VERSION,
                &payload,
                &head_hash,
            )
            .map_err(|e| format!("failed to append solve.expr: {e}"))?;
        head_hash = appended.event_hash;
        harvested.push(HarvestedExpr {
            handle: item.handle,
            seq: appended.seq,
            expr: item.expr,
            error: item.error,
            rejected_constructor: item.rejected_constructor,
        });
    }

    Ok(HarvestReport {
        harvested,
        skipped,
        total_handles_in_run: total_handles,
        total_srepr_bytes_in_run: total_srepr_bytes,
    })
}

/// Extract `exprs_out` from a `sympy_eval` result JSON value and harvest via [`harvest_exprs_out`].
pub fn harvest_from_sympy_result(
    ledger: &Ledger,
    session: &str,
    produced_by: ReceiptRef,
    sympy_result: &Value,
    canonicalizer: &dyn CheckCanonicalizer,
) -> Result<HarvestReport, String> {
    let (raw_exprs, malformed) = extract_exprs_out_checked(sympy_result);
    let mut report = harvest_exprs_out(ledger, session, produced_by, &raw_exprs, canonicalizer)?;
    if !malformed.is_empty() {
        let mut combined = malformed;
        combined.extend(report.skipped);
        report.skipped = combined;
    }
    Ok(report)
}

/// Attach compact check-process handle previews (`exprs`, plus `expr_warnings` if any skipped)
/// onto a `sympy_eval` tool result object (`solve.md` 5.1 & 9 row S7).
pub fn attach_harvest_previews(sympy_result: &mut Value, report: &HarvestReport) {
    let Some(obj) = sympy_result.as_object_mut() else {
        return;
    };
    let previews: Vec<Value> = report
        .harvested
        .iter()
        .map(|h| {
            let mut item = json!({
                "handle": h.handle.to_string(),
                "name": h.expr.name,
                "version": h.expr.version,
                "portable": h.expr.portable,
                "latex": h.expr.latex,
                "str_preview": h.expr.str_preview,
                "free_symbols": h.expr.free_symbols,
                "srepr_sha256": h.expr.srepr_sha256,
            });
            if let Some(err) = &h.error {
                item["error"] = json!(err);
            }
            if let Some(ctor) = &h.rejected_constructor {
                item["rejected_constructor"] = json!(ctor);
            }
            item
        })
        .collect();
    obj.insert("exprs".into(), Value::Array(previews));
    if !report.skipped.is_empty() {
        obj.insert("expr_warnings".into(), json!(report.skipped));
    }
}

/// Load and integrity-verify the canonical `srepr` blob of a stored [`SolveExpr`].
/// Returns `Ok(None)` for non-portable handles; fails closed on missing or tampered blobs.
pub fn load_handle_srepr(ledger: &Ledger, expr: &SolveExpr) -> Result<Option<String>, String> {
    if !expr.portable {
        return Ok(None);
    }
    if expr.srepr_blob.is_empty() || expr.srepr_sha256.is_empty() {
        return Err(format!("portable handle `{}@{}` is missing srepr blob digest", expr.name, expr.version));
    }
    if expr.srepr_blob != expr.srepr_sha256 {
        return Err(format!("portable handle `{}@{}` has mismatched srepr_blob and srepr_sha256", expr.name, expr.version));
    }
    let bytes = ledger
        .get_blob(&expr.srepr_blob)
        .map_err(|e| format!("cannot load srepr blob for `{}@{}`: {e}", expr.name, expr.version))?;
    let actual = sha256_hex(&bytes);
    if actual != expr.srepr_sha256 {
        return Err(format!(
            "srepr blob hash mismatch for `{}@{}` (expected {}, got {actual})",
            expr.name, expr.version, expr.srepr_sha256
        ));
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|e| format!("srepr blob for `{}@{}` is not valid UTF-8: {e}", expr.name, expr.version))
}

/// Reconstruct `{ name: canonical_srepr }` from ledger blobs for the latest portable version
/// of each handle in `state`, so `Worker::evaluate_with_exprs` re-stages expressions cleanly
/// across solver restarts without `.cas_bus.json` (`solve.md` 4.3, 5.5, S7).
pub fn stage_exprs_in_from_state(
    ledger: &Ledger,
    state: &SolveState,
) -> Result<BTreeMap<String, String>, String> {
    let mut latest: BTreeMap<&str, &SolveExpr> = BTreeMap::new();
    for (handle, expr) in &state.handles {
        if latest.get(handle.name.as_str()).is_none_or(|prev| prev.version < expr.version) {
            latest.insert(handle.name.as_str(), expr);
        }
    }
    let mut staged = BTreeMap::new();
    for (name, expr) in latest {
        if let Some(srepr) = load_handle_srepr(ledger, expr)? {
            staged.insert(name.to_string(), srepr);
        }
    }
    Ok(staged)
}

/// Read `session` from `ledger`, fold [`SolveState`], and reconstruct `{ name: canonical_srepr }`.
pub fn stage_exprs_in(ledger: &Ledger, session: &str) -> Result<BTreeMap<String, String>, String> {
    let events = ledger.read_session(session).map_err(|e| e.to_string())?;
    stage_exprs_in_from_state(ledger, &SolveState::from_events(&events))
}

enum PortableOutcome {
    Ready(Box<SolveExpr>, Vec<u8>),
    CapExceeded(String),
    NonPortable(String),
}

fn prepare_portable_expr(
    name: &str,
    version: u32,
    produced_by: &ReceiptRef,
    canon: CanonicalizedExpr,
    current_total_bytes: usize,
) -> PortableOutcome {
    if canon.srepr.is_empty() || canon.srepr_sha256.is_empty() {
        return PortableOutcome::NonPortable(
            "check process returned empty canonical srepr or srepr_sha256".into(),
        );
    }
    let bytes = canon.srepr.into_bytes();
    let actual_sha256 = sha256_hex(&bytes);
    if canon.srepr_sha256 != actual_sha256 {
        return PortableOutcome::NonPortable(format!(
            "check process srepr_sha256 mismatch (reported {}, computed {actual_sha256})",
            canon.srepr_sha256
        ));
    }
    if current_total_bytes.saturating_add(bytes.len()) > MAX_EXPR_SREPR_BYTES_PER_RUN {
        return PortableOutcome::CapExceeded(format!(
            "per-run srepr byte cap reached ({current_total_bytes} + {} > {MAX_EXPR_SREPR_BYTES_PER_RUN} bytes)",
            bytes.len()
        ));
    }
    let (latex, _) = clip_utf8_chars(&canon.latex, MAX_EXPR_LATEX_CHARS);
    let (str_preview, _) = clip_utf8_chars(&canon.str_preview, MAX_STR_PREVIEW_CHARS);
    let expr = SolveExpr {
        name: name.to_string(),
        version,
        srepr_blob: actual_sha256.clone(),
        srepr_sha256: actual_sha256,
        latex,
        str_preview,
        free_symbols: canon.free_symbols,
        produced_by: produced_by.clone(),
        owner_entry: None,
        portable: true,
    };
    PortableOutcome::Ready(Box::new(expr), bytes)
}

fn non_portable_expr(name: &str, version: u32, produced_by: &ReceiptRef) -> SolveExpr {
    SolveExpr {
        name: name.to_string(),
        version,
        srepr_blob: String::new(),
        srepr_sha256: String::new(),
        latex: String::new(),
        str_preview: String::new(),
        free_symbols: Vec::new(),
        produced_by: produced_by.clone(),
        owner_entry: None,
        portable: false,
    }
}

fn count_existing_srepr_bytes(ledger: &Ledger, state: &SolveState) -> Result<usize, String> {
    let mut size_cache: HashMap<&str, usize> = HashMap::new();
    let mut total = 0usize;
    for expr in state.handles.values() {
        if !expr.portable {
            continue;
        }
        if expr.srepr_blob.is_empty() || expr.srepr_blob != expr.srepr_sha256 {
            return Err(format!(
                "existing portable handle `{}@{}` has invalid or mismatched srepr blob digest",
                expr.name, expr.version
            ));
        }
        let len = if let Some(&cached) = size_cache.get(expr.srepr_blob.as_str()) {
            cached
        } else {
            let n = ledger
                .get_blob(&expr.srepr_blob)
                .map_err(|e| format!("cannot read existing srepr blob `{}`: {e}", expr.srepr_blob))?
                .len();
            size_cache.insert(expr.srepr_blob.as_str(), n);
            n
        };
        total = total.saturating_add(len);
    }
    Ok(total)
}

#[cfg(test)]
#[path = "solve_exprs_tests.rs"]
mod tests;
