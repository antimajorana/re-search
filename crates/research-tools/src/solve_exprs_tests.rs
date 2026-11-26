use super::*;
use crate::solve_model::event_kinds;
use ledger::Ledger;
use serde_json::json;
use tempfile::TempDir;

struct TestCanonicalizer {
    mode: CanonMode,
}

enum CanonMode {
    Echo,
    Reject { reason: &'static str, ctor: Option<&'static str> },
    BadHash,
    FailErr(&'static str),
    LongPreview(String),
}

impl CheckCanonicalizer for TestCanonicalizer {
    fn canonicalize(&self, raw_srepr: &str) -> Result<CanonicalizedExpr, String> {
        match &self.mode {
            CanonMode::Echo => Ok(CanonicalizedExpr {
                portable: true,
                srepr: raw_srepr.to_string(),
                srepr_sha256: sha256_hex(raw_srepr.as_bytes()),
                latex: format!("\\text{{{raw_srepr}}}"),
                str_preview: raw_srepr.to_string(),
                free_symbols: vec![SymbolSpec {
                    name: "x".into(),
                    assumptions: vec!["real".into(), "positive".into()],
                }],
                error: None,
                rejected_constructor: None,
            }),
            CanonMode::Reject { reason, ctor } => Ok(CanonicalizedExpr {
                portable: false,
                srepr: String::new(),
                srepr_sha256: String::new(),
                latex: String::new(),
                str_preview: String::new(),
                free_symbols: Vec::new(),
                error: Some((*reason).to_string()),
                rejected_constructor: ctor.map(str::to_string),
            }),
            CanonMode::BadHash => Ok(CanonicalizedExpr {
                portable: true,
                srepr: raw_srepr.to_string(),
                srepr_sha256: "0".repeat(64),
                latex: "x".into(),
                str_preview: "x".into(),
                free_symbols: Vec::new(),
                error: None,
                rejected_constructor: None,
            }),
            CanonMode::FailErr(msg) => Err((*msg).to_string()),
            CanonMode::LongPreview(preview) => Ok(CanonicalizedExpr {
                portable: true,
                srepr: raw_srepr.to_string(),
                srepr_sha256: sha256_hex(raw_srepr.as_bytes()),
                latex: "L".into(),
                str_preview: preview.clone(),
                free_symbols: Vec::new(),
                error: None,
                rejected_constructor: None,
            }),
        }
    }
}

fn open_test_session() -> (TempDir, Ledger, &'static str) {
    let tmp = TempDir::new().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let sid = "s-expr-test";
    ledger.append(sid, event_kinds::SOLVE_STARTED, 1, &json!({})).unwrap();
    (tmp, ledger, sid)
}

fn receipt(call_id: &str, seq: u64) -> ReceiptRef {
    ReceiptRef { call_id: call_id.into(), result_seq: seq }
}

#[test]
fn validate_expr_name_enforces_identifier_and_reserved_names() {
    for good in ["x", "V1_msbar", "_tmp1", "I_d", "E", "I", "a_very_long_valid_handle_name_0123456789_abcdefghijklmnop_123456"] {
        assert!(validate_expr_name(good).is_ok(), "should accept `{good}`");
    }
    let too_long = "a".repeat(65);
    for bad in [
        "",
        "1abc",
        "foo-bar",
        "foo@1",
        "foo.bar",
        "α",
        too_long.as_str(),
        "__proto__",
        "__import__",
        "__init__",
        "lambda",
        "None",
        "True",
        "False",
        "class",
        "constructor",
        "prototype",
        "eval",
        "exec",
        "expr",
        "save_expr",
        "load_expr",
        "sympy",
    ] {
        assert!(validate_expr_name(bad).is_err(), "should reject `{bad}`");
    }
}

#[test]
fn harvest_versions_handles_and_folds_into_solve_state() {
    let (_tmp, ledger, sid) = open_test_session();
    let canon = TestCanonicalizer { mode: CanonMode::Echo };

    let mut batch1 = BTreeMap::new();
    batch1.insert("V1".into(), "Symbol('x', positive=True)".into());
    batch1.insert("__bad__".into(), "Integer(1)".into());
    let r1 = harvest_exprs_out(&ledger, sid, receipt("c-1", 2), &batch1, &canon).unwrap();
    assert_eq!(r1.harvested.len(), 1);
    assert_eq!(r1.skipped.len(), 1);
    assert_eq!(r1.skipped[0].name, "__bad__");
    assert_eq!(r1.harvested[0].handle.to_string(), "V1@1");
    assert!(r1.harvested[0].expr.portable);
    assert_eq!(r1.harvested[0].expr.srepr_blob, r1.harvested[0].expr.srepr_sha256);

    let mut batch2 = BTreeMap::new();
    batch2.insert("V1".into(), "Pow(Symbol('x', positive=True), Integer(2))".into());
    let r2 = harvest_exprs_out(&ledger, sid, receipt("c-2", 4), &batch2, &canon).unwrap();
    assert_eq!(r2.harvested.len(), 1);
    assert_eq!(r2.harvested[0].handle.to_string(), "V1@2");
    assert_eq!(r2.total_handles_in_run, 2);

    let events = ledger.read_session(sid).unwrap();
    let state = SolveState::from_events(&events);
    assert_eq!(state.handles.len(), 2);
    assert!(state.handles.contains_key(&HandleRef::parse("V1@1").unwrap()));
    assert!(state.handles.contains_key(&HandleRef::parse("V1@2").unwrap()));

    let staged = stage_exprs_in(&ledger, sid).unwrap();
    assert_eq!(staged.len(), 1);
    assert_eq!(staged["V1"], "Pow(Symbol('x', positive=True), Integer(2))");
}

#[test]
fn harvest_enforces_max_exprs_per_run_cap() {
    let (_tmp, ledger, sid) = open_test_session();
    let canon = TestCanonicalizer { mode: CanonMode::Echo };

    let mut batch = BTreeMap::new();
    for i in 0..(MAX_EXPRS_PER_RUN + 5) {
        batch.insert(format!("h_{i:03}"), format!("Integer({i})"));
    }
    let report = harvest_exprs_out(&ledger, sid, receipt("c-cap", 2), &batch, &canon).unwrap();
    assert_eq!(report.harvested.len(), MAX_EXPRS_PER_RUN);
    assert_eq!(report.skipped.len(), 5);
    assert_eq!(report.total_handles_in_run, MAX_EXPRS_PER_RUN);
    assert!(report.skipped[0].reason.contains("cap"));
}

#[test]
fn harvest_enforces_max_srepr_bytes_per_run_cap() {
    let (_tmp, ledger, sid) = open_test_session();
    let canon = TestCanonicalizer { mode: CanonMode::Echo };

    // 700 KiB payload repeated 3 times = 2.1 MiB > 2 MiB cap (even though blob is deduplicated).
    let big = "a".repeat(700 * 1024);
    let mut batch = BTreeMap::new();
    batch.insert("b1".into(), big.clone());
    batch.insert("b2".into(), big.clone());
    batch.insert("b3".into(), big);

    let report = harvest_exprs_out(&ledger, sid, receipt("c-bytes", 2), &batch, &canon).unwrap();
    assert_eq!(report.harvested.len(), 2);
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(report.skipped[0].name, "b3");
    assert!(report.skipped[0].reason.contains("byte cap"));
    assert_eq!(report.total_srepr_bytes_in_run, 2 * 700 * 1024);
}

#[test]
fn rejected_or_errored_or_tampered_canonicalization_records_non_portable_without_crashing() {
    let (_tmp, ledger, sid) = open_test_session();

    // 1. Check process rejects Lambda -> portable: false, empty latex/srepr_blob.
    let rejector = TestCanonicalizer {
        mode: CanonMode::Reject { reason: "disallowed constructor 'Lambda'", ctor: Some("Lambda") },
    };
    let mut b1 = BTreeMap::new();
    b1.insert("lam".into(), "Lambda(Tuple(Symbol('x')), Symbol('x'))".into());
    let r1 = harvest_exprs_out(&ledger, sid, receipt("c-1", 2), &b1, &rejector).unwrap();
    assert_eq!(r1.harvested.len(), 1);
    assert!(!r1.harvested[0].expr.portable);
    assert!(r1.harvested[0].expr.latex.is_empty());
    assert!(r1.harvested[0].expr.srepr_blob.is_empty());
    assert!(r1.harvested[0].expr.srepr_sha256.is_empty());
    assert_eq!(r1.harvested[0].rejected_constructor.as_deref(), Some("Lambda"));

    // 2. Check process reports mismatched hash -> fail closed to portable: false.
    let bad_hash = TestCanonicalizer { mode: CanonMode::BadHash };
    let mut b2 = BTreeMap::new();
    b2.insert("tampered".into(), "Integer(7)".into());
    let r2 = harvest_exprs_out(&ledger, sid, receipt("c-2", 3), &b2, &bad_hash).unwrap();
    assert!(!r2.harvested[0].expr.portable);
    assert!(r2.harvested[0].error.as_deref().unwrap().contains("mismatch"));

    // 3. Check process returns Err -> fail closed to portable: false, never crashes turn.
    let failing = TestCanonicalizer { mode: CanonMode::FailErr("check worker timed out") };
    let mut b3 = BTreeMap::new();
    b3.insert("timed_out".into(), "Integer(9)".into());
    let r3 = harvest_exprs_out(&ledger, sid, receipt("c-3", 4), &b3, &failing).unwrap();
    assert!(!r3.harvested[0].expr.portable);
    assert!(r3.harvested[0].error.as_deref().unwrap().contains("timed out"));

    // None of the non-portable handles are staged for `exprs_in`.
    let staged = stage_exprs_in(&ledger, sid).unwrap();
    assert!(staged.is_empty());
}

#[test]
fn non_portable_newer_version_does_not_resurrect_older_portable_version_in_stage_exprs_in() {
    let (_tmp, ledger, sid) = open_test_session();
    let ok_canon = TestCanonicalizer { mode: CanonMode::Echo };
    let bad_canon = TestCanonicalizer {
        mode: CanonMode::Reject { reason: "disallowed", ctor: Some("Lambda") },
    };

    let mut b1 = BTreeMap::new();
    b1.insert("f".into(), "Integer(1)".into());
    harvest_exprs_out(&ledger, sid, receipt("c-1", 2), &b1, &ok_canon).unwrap();
    assert_eq!(stage_exprs_in(&ledger, sid).unwrap().get("f").map(String::as_str), Some("Integer(1)"));

    let mut b2 = BTreeMap::new();
    b2.insert("f".into(), "Lambda(...)".into());
    harvest_exprs_out(&ledger, sid, receipt("c-2", 3), &b2, &bad_canon).unwrap();
    assert!(
        !stage_exprs_in(&ledger, sid).unwrap().contains_key("f"),
        "latest f@2 is non-portable; must not silently re-stage stale f@1"
    );
}

#[test]
fn str_preview_and_expr_show_clip_utf8_safely_and_support_all_query_forms() {
    let (_tmp, ledger, sid) = open_test_session();
    let unicode_preview = "α∫π".repeat(200); // 600 chars, multi-byte UTF-8
    let canon = TestCanonicalizer { mode: CanonMode::LongPreview(unicode_preview) };

    let mut b1 = BTreeMap::new();
    b1.insert("E".into(), "Symbol('alpha')".into());
    let report = harvest_exprs_out(&ledger, sid, receipt("c-u", 2), &b1, &canon).unwrap();
    assert_eq!(report.harvested[0].expr.str_preview.chars().count(), MAX_STR_PREVIEW_CHARS);

    // Attach preview helper populates `exprs`.
    let mut sympy_res = json!({ "value": "alpha", "stdout": "", "stderr": "", "truncated": false });
    attach_harvest_previews(&mut sympy_res, &report);
    assert_eq!(sympy_res["exprs"][0]["handle"], "E@1");
    assert_eq!(sympy_res["exprs"][0]["portable"], true);

    // List mode (no `name`).
    let listed = run_expr_show(&ledger, sid, &json!({})).unwrap();
    assert_eq!(listed["count"], 1);
    assert_eq!(listed["handles"][0]["handle"], "E@1");

    // Inspect bare name + include_srepr.
    let shown = run_expr_show(&ledger, sid, &json!({ "name": "E", "include_srepr": true })).unwrap();
    assert_eq!(shown["handle"], "E@1");
    assert_eq!(shown["srepr"], "Symbol('alpha')");
    assert_eq!(shown["portable"], true);
    assert_eq!(shown["truncated"], false);

    // Inspect `name@version` with `form: "str"` and tight `max_chars`.
    let clipped = run_expr_show(
        &ledger,
        sid,
        &json!({ "name": "E@1", "form": "str", "max_chars": 5 }),
    )
    .unwrap();
    assert_eq!(clipped["form"], "str");
    assert_eq!(clipped["text"].as_str().unwrap().chars().count(), 5);
    assert_eq!(clipped["text"], "α∫πα∫");
    assert_eq!(clipped["truncated"], true);

    // Error cases: conflicting version, bad max_chars, version without name, unknown handle, non-string name.
    assert!(run_expr_show(&ledger, sid, &json!({ "name": "E@1", "version": 2 })).is_err());
    assert!(run_expr_show(&ledger, sid, &json!({ "version": 1 })).is_err());
    assert!(run_expr_show(&ledger, sid, &json!({ "name": 123 })).is_err());
    assert!(run_expr_show(&ledger, sid, &json!({ "name": "E", "max_chars": 0 })).is_err());
    assert!(run_expr_show(&ledger, sid, &json!({ "name": "E", "max_chars": 4001 })).is_err());
    assert!(run_expr_show(&ledger, sid, &json!({ "name": "missing" })).is_err());
}

#[test]
fn audit_fixes_empty_hash_concurrent_head_malformed_outbox_and_non_portable_note() {
    let (tmp, ledger, sid) = open_test_session();

    // 1. Empty srepr_sha256 from canonicalizer fails closed to portable: false.
    struct EmptyHashCanon;
    impl CheckCanonicalizer for EmptyHashCanon {
        fn canonicalize(&self, raw_srepr: &str) -> Result<CanonicalizedExpr, String> {
            Ok(CanonicalizedExpr {
                portable: true,
                srepr: raw_srepr.to_string(),
                srepr_sha256: String::new(),
                latex: "x".into(),
                str_preview: "x".into(),
                free_symbols: Vec::new(),
                error: None,
                rejected_constructor: None,
            })
        }
    }
    let mut b = BTreeMap::new();
    b.insert("empty_h".into(), "Integer(1)".into());
    let r = harvest_exprs_out(&ledger, sid, receipt("c-eh", 2), &b, &EmptyHashCanon).unwrap();
    assert!(!r.harvested[0].expr.portable);
    assert!(r.harvested[0].error.as_deref().unwrap().contains("empty"));

    // Non-portable handle in expr_show includes diagnostic `note`.
    let shown = run_expr_show(&ledger, sid, &json!({ "name": "empty_h", "include_srepr": true })).unwrap();
    assert_eq!(shown["portable"], false);
    assert_eq!(shown["srepr"], "");
    assert!(shown["note"].as_str().unwrap().contains("non-portable"));

    // 2. Malformed exprs_out entries are reported in `skipped` rather than silently dropped.
    let canon = TestCanonicalizer { mode: CanonMode::Echo };
    let bad_obj = json!({ "exprs_out": { "good": "Integer(3)", "bad_num": 42 } });
    let rep = harvest_from_sympy_result(&ledger, sid, receipt("c-m", 4), &bad_obj, &canon).unwrap();
    assert_eq!(rep.harvested.len(), 1);
    assert_eq!(rep.harvested[0].expr.name, "good");
    assert_eq!(rep.skipped.len(), 1);
    assert_eq!(rep.skipped[0].name, "bad_num");

    let bad_type = json!({ "exprs_out": ["not", "an", "object"] });
    let rep2 = harvest_from_sympy_result(&ledger, sid, receipt("c-m2", 6), &bad_type, &canon).unwrap();
    assert!(rep2.harvested.is_empty());
    assert_eq!(rep2.skipped.len(), 1);
    assert_eq!(rep2.skipped[0].name, "exprs_out");

    // 3. Concurrent session append between read and commit fails closed via `append_at_head`.
    struct RacingCanon {
        data_dir: std::path::PathBuf,
        session: String,
    }
    impl CheckCanonicalizer for RacingCanon {
        fn canonicalize(&self, raw_srepr: &str) -> Result<CanonicalizedExpr, String> {
            let other = Ledger::open(&self.data_dir).unwrap();
            other
                .append(&self.session, "note", 1, &json!({ "text": "concurrent write" }))
                .unwrap();
            Ok(CanonicalizedExpr {
                portable: true,
                srepr: raw_srepr.to_string(),
                srepr_sha256: sha256_hex(raw_srepr.as_bytes()),
                latex: "1".into(),
                str_preview: "1".into(),
                free_symbols: Vec::new(),
                error: None,
                rejected_constructor: None,
            })
        }
    }
    let racing = RacingCanon { data_dir: tmp.path().to_path_buf(), session: sid.to_string() };
    let mut race_batch = BTreeMap::new();
    race_batch.insert("raced".into(), "Integer(5)".into());
    let err = harvest_exprs_out(&ledger, sid, receipt("c-race", 8), &race_batch, &racing).unwrap_err();
    assert!(err.contains("changed") || err.contains("append"), "{err}");
    let state = SolveState::from_events(&ledger.read_session(sid).unwrap());
    assert!(!state.handles.keys().any(|h| h.name == "raced"), "raced handle must not be committed");
}
