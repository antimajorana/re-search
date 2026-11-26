//! Self-tests of the evaluation harness (package F8, overview section 7).
//!
//! They prove the skeleton end to end on small self-written fixtures:
//! manifests load and validate, the recorded-service mock routes and
//! flags, a scripted-model case runs through the real agent loop with its
//! model calls counted, and a sheet is filled, gated and rendered.
//! No mode feature is tested here; none exists yet.

#[allow(dead_code)]
mod eval_support;
#[path = "eval_support/solve_q_tests.rs"]
mod solve_q_tests;
#[allow(dead_code)]
mod support;

use agent::{run_agent_loop, SessionOutcome};
use eval_support::manifest::{parse_manifest, validate_manifest, validate_manifests, Input, Mode, Set, SolveTask, Tier};
use eval_support::measures::{
    count_in, evaluate, judge, measure, percent, share, solve_wording, GateVerdict, Outcome, Rule,
    SolveQCounts, MEASURES,
};
use eval_support::routed_mock::{RoutedMock, Routes};
use eval_support::scripted::{load_script, ScriptStep, ScriptedModel};
use eval_support::sheet::{current_build, parse_sheet, render_markdown, today_utc, validate_sheet, write_sheet, Build, Sheet};
use eval_support::{fixture_path, fixtures_dir, load_manifests, read_fixture_text, repo_root, sheets_dir};
use ledger::Ledger;
use research_tools::http_client::{fetch_bytes_allowlisted, MAX_METADATA_BYTES};
use research_tools::LiteratureEndpoints;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Instant;
use support::{default_cfg, make_router, MockApprover, MockEpoch, RecordingSink, TempDir};

#[test]
fn fixed_tier_manifests_load_and_validate() {
    let set = load_manifests(&fixtures_dir(), Tier::Fixed).unwrap_or_else(|e| panic!("fixed tier is invalid:\n{}", e.join("\n")));
    let sets: Vec<Set> = set.manifests.iter().map(|m| m.set).collect();
    assert_eq!(sets, [Set::P, Set::Q], "fixed tier ships manifests for P and Q");
    let p_cases = set.cases_of(Set::P);
    let p_selftests = p_cases.iter().filter(|c| c.id.starts_with("P-selftest-")).count();
    let p_evals = p_cases.iter().filter(|c| c.id.starts_with("P-eval-")).count();
    assert_eq!(p_selftests, 2);
    assert_eq!(p_evals, 10);
    assert_eq!(p_cases.len(), p_selftests + p_evals);
    assert_eq!(set.cases_of(Set::Q).len(), 17, "2 Q self-tests + 15 fixed Q evaluation cases");
    for (s, case) in set.cases() {
        assert_eq!(case.mode, s.mode());
        let is_selftest = case.id.starts_with("P-selftest-") || case.id.starts_with("Q-selftest-");
        assert_eq!(
            case.notes.contains("Harness self-test case, not part of the evaluation set"),
            is_selftest,
            "{}",
            case.id
        );
        if !is_selftest && s == Set::P {
            assert!(case.notes.contains("Fixed-tier Set P evaluation case"), "{}", case.id);
        } else {
            assert!(!case.notes.trim().is_empty(), "{}", case.id);
        }
    }

    // The fixture a P case names is self-written TeX that is really there.
    let (_, thm) = set.case("P-selftest-theorems").unwrap();
    let Input::Fixture { files, .. } = &thm.input else { panic!("P self-test must be a fixture") };
    let tex = read_fixture_text(&set.dir, &files[0]).unwrap();
    for stmt in thm.answer_key["statements"].as_array().unwrap() {
        let label = format!("\\label{{{}}}", stmt["label"].as_str().unwrap());
        assert!(tex.contains(&label), "key names {label}, which the fixture lacks");
    }
    assert!(!tex.contains('\r'));

    // The planted-error case names a step that exists and is expected to end Failed.
    let (_, planted) = set.case("Q-selftest-planted").unwrap();
    let Input::Problem { task, steps, .. } = &planted.input else { panic!("Q self-test must be a problem") };
    assert_eq!(*task, SolveTask::CheckDerivation);
    let key_steps = planted.answer_key["steps"].as_array().unwrap();
    assert_eq!(steps.as_ref().unwrap().len(), key_steps.len());
    let planted_id = planted.answer_key["planted_step"].as_str().unwrap();
    let planted_step = key_steps.iter().find(|s| s["id"] == planted_id).unwrap();
    assert_eq!(planted_step["expected_tier"], "Failed");

    // A directory with nothing in it is an error, not an empty pass.
    let empty = tempfile::tempdir().unwrap();
    assert!(load_manifests(empty.path(), Tier::Fixed).is_err());
    // The repository's manifests are fixed tier; asking for them as live fails.
    assert!(load_manifests(&fixtures_dir(), Tier::Live).is_err());
}

fn manifest_json(set: &str, tier: &str, cases: Value) -> String {
    json!({ "format_version": 1, "set": set, "tier": tier, "cases": cases }).to_string()
}

fn errors_of(text: &str) -> Vec<String> {
    match parse_manifest(text) {
        Ok(m) => validate_manifest(&m),
        Err(e) => vec![e],
    }
}

fn assert_rejected(text: &str, needle: &str) {
    let errs = errors_of(text);
    assert!(errs.iter().any(|e| e.contains(needle)), "expected an error containing `{needle}`, got {errs:?}");
}

/// Syntactically valid placeholders. Deliberately not real papers: real
/// ids enter a manifest only when the owner picks them.
const FAKE_ARXIV: &str = "0000.00000";
const FAKE_DOI: &str = "10.0000/placeholder";

#[test]
fn manifest_validator_is_strict() {
    let p_key = json!({ "statements": [], "sections": [], "uses_edges": [] });
    let live_p = |input: Value| {
        manifest_json("P", "live", json!([{ "id": "P-01", "mode": "investigate", "input": input, "answer_key": p_key, "notes": "" }]))
    };

    // A live-tier paper is an arXiv id or a DOI and nothing else.
    assert_eq!(errors_of(&live_p(json!({ "kind": "arxiv", "id": FAKE_ARXIV }))), Vec::<String>::new());
    assert_eq!(errors_of(&live_p(json!({ "kind": "doi", "doi": FAKE_DOI }))), Vec::<String>::new());
    assert_eq!(errors_of(&live_p(json!({ "kind": "arxiv", "id": "hep-th/0000000v2", "locator": "Theorem 2.1" }))), Vec::<String>::new());
    assert_rejected(&live_p(json!({ "kind": "fixture", "files": ["p/x.tex"] })), "never by a stored file");
    assert_rejected(&live_p(json!({ "kind": "tex", "text": "\\begin{theorem}" })), "unknown variant");
    assert_rejected(&live_p(json!({ "kind": "arxiv", "id": FAKE_ARXIV, "tex": "x" })), "unknown field");
    assert_rejected(&live_p(json!({ "kind": "arxiv", "id": "https://arxiv.org/abs/0000.00000" })), "not an arXiv identifier");
    assert_rejected(&live_p(json!({ "kind": "doi", "doi": "doi:10.1/x" })), "not a DOI");
    assert_rejected(&live_p(json!({ "kind": "arxiv", "id": FAKE_ARXIV, "locator": "x".repeat(81) })), "locator");
    assert_rejected(&live_p(json!({ "kind": "idea", "text": "an idea" })), "not allowed in set P");
    assert_rejected(&live_p(json!({ "kind": "arxiv", "id": FAKE_ARXIV, "versions": ["v1", "v2"] })), "set P2 only");

    // Envelope and case rules.
    let case = |id: &str, mode: &str| json!({ "id": id, "mode": mode, "input": { "kind": "arxiv", "id": FAKE_ARXIV }, "answer_key": p_key, "notes": "" });
    assert_rejected(&manifest_json("P", "live", json!([case("P-01", "investigate"), case("P-01", "investigate")])), "appears twice");
    assert_rejected(&manifest_json("P", "live", json!([case("P-01", "solve")])), "does not match set P");
    assert_rejected(&manifest_json("P", "live", json!([case("P-01", "review")])), "unknown variant");
    assert_rejected(&manifest_json("P", "live", json!([case("Q-01", "investigate")])), "id must be `P-`");
    assert_rejected(&manifest_json("Z", "live", json!([])), "unknown variant");
    assert_rejected(&manifest_json("P", "recorded", json!([])), "unknown variant");
    assert_rejected(&manifest_json("R", "live", json!([])), "fixed tier only");
    assert_rejected(&json!({ "format_version": 2, "set": "P", "tier": "live", "cases": [] }).to_string(), "format_version 2");
    assert_rejected(&json!({ "format_version": 1, "set": "P", "tier": "live", "cases": [], "owner": "x" }).to_string(), "unknown field");
    let mut extra = case("P-01", "investigate");
    extra["expected"] = json!(1);
    assert_rejected(&manifest_json("P", "live", json!([extra])), "unknown field");
    let mut no_notes = case("P-01", "investigate");
    no_notes.as_object_mut().unwrap().remove("notes");
    assert_rejected(&manifest_json("P", "live", json!([no_notes])), "missing field `notes`");
    let mut thin_key = case("P-01", "investigate");
    thin_key["answer_key"] = json!({ "statements": [] });
    assert_rejected(&manifest_json("P", "live", json!([thin_key])), "answer_key lacks `uses_edges`");

    // Other sets.
    let q = |input: Value| {
        let key = json!({ "final_answer": "1", "steps": [], "planted_step": null });
        manifest_json("Q", "live", json!([{ "id": "Q-01", "mode": "solve", "input": input, "answer_key": key, "notes": "" }]))
    };
    assert_eq!(errors_of(&q(json!({ "kind": "problem", "task": "prove", "statement": "Show 1 = 1." }))), Vec::<String>::new());
    assert_rejected(&q(json!({ "kind": "problem", "task": "check_derivation", "statement": "s" })), "needs its `steps`");
    assert_rejected(&q(json!({ "kind": "problem", "task": "calculate", "statement": "  " })), "must not be empty");
    let p2 = |input: Value| {
        manifest_json("P2", "fixed", json!([{ "id": "P2-01", "mode": "investigate", "input": input, "answer_key": { "statements": [] }, "notes": "" }]))
    };
    assert_eq!(errors_of(&p2(json!({ "kind": "arxiv", "id": FAKE_ARXIV, "versions": ["v1", "v3"] }))), Vec::<String>::new());
    assert_rejected(&p2(json!({ "kind": "arxiv", "id": FAKE_ARXIV })), "needs `versions`");
    assert_rejected(&p2(json!({ "kind": "fixture", "files": ["p/a.tex"] })), "exactly 2 fixture");
    assert_rejected(&p2(json!({ "kind": "fixture", "files": ["p/a.tex", "../b.tex"] })), "stay inside the fixtures directory");
    assert_rejected(&p2(json!({ "kind": "fixture", "files": ["p/a.tex", "C:\\b.tex"] })), "stay inside the fixtures directory");
}

#[test]
fn manifests_are_checked_together() {
    let fixed = load_manifests(&fixtures_dir(), Tier::Fixed).unwrap();
    let chain = |id: &str, from: &str, to: &str| {
        json!({ "id": id, "mode": "handover", "input": { "kind": "chain", "from": from, "to": to }, "answer_key": { "target_answer": "sqrt(pi)" }, "notes": "" })
    };
    let with_chains = |cases: Value| {
        let mut all = fixed.manifests.clone();
        all.push(parse_manifest(&manifest_json("C", "fixed", cases)).unwrap());
        validate_manifests(&all)
    };
    assert_eq!(with_chains(json!([chain("C-01", "P-selftest-oscillator", "Q-selftest-gaussian")])), Vec::<String>::new());
    let errs = with_chains(json!([chain("C-01", "P-selftest-oscillator", "Q-missing"), chain("C-02", "Q-selftest-gaussian", "P-selftest-oscillator")]));
    assert!(errs.iter().any(|e| e.contains("`Q-missing` is not a known case")), "{errs:?}");
    assert!(errs.iter().any(|e| e.contains("from set Q to set P is not one of")), "{errs:?}");

    let mut twice = fixed.manifests.clone();
    twice.push(fixed.manifests[0].clone());
    let errs = validate_manifests(&twice);
    assert!(errs.iter().any(|e| e.contains("more than one fixed manifest for set P")), "{errs:?}");
}

#[test]
fn measure_table_is_the_table_of_section_7_3() {
    let ids: BTreeSet<&str> = MEASURES.iter().map(|d| d.id).collect();
    assert_eq!(ids.len(), MEASURES.len(), "measure ids must be unique");
    for d in MEASURES {
        assert!(!d.label.is_empty() && !d.unit.is_empty() && !d.spec_applies_to.is_empty(), "{}", d.id);
    }
    // Every pass rule 7.3 states, and nothing else, has a target.
    let rules: BTreeMap<&str, Rule> = MEASURES.iter().filter(|d| d.rule != Rule::Recorded).map(|d| (d.id, d.rule)).collect();
    let below = |b| Rule::Threshold(eval_support::measures::Cmp::Below, b);
    let at_least = |b| Rule::Threshold(eval_support::measures::Cmp::AtLeast, b);
    let expected: BTreeMap<&str, Rule> = [
        ("time_to_skeleton_tex_s", below(10.0)),
        ("time_to_candidates_s", below(6.0)),
        ("time_to_solve_setup_s", below(1.0)),
        ("runs_above_cap", Rule::MustBeZero),
        ("statement_recall_tex_theorem_envs", at_least(0.9)),
        ("section_recall_tex_theorem_envs", at_least(0.9)),
        ("wrong_carry_overs", Rule::MustBeZero),
        ("judged_same_key_different_unreviewed", Rule::MustBeZero),
        ("wrong_statement_located", Rule::MustBeZero),
        (solve_wording::ID_PLANTED_ERROR_ENDS_FAILED_PCT, Rule::MustEqual(100.0)),
        (solve_wording::ID_PLANTED_ERROR_ENDS_VERIFIED, Rule::MustBeZero),
        (solve_wording::ID_PLANTED_ERROR_ENDS_CHECKED_NUMERICALLY, Rule::MustBeZero),
        ("quotes_surviving_handover_pct", Rule::MustEqual(100.0)),
        ("old_ledgers_not_resuming", Rule::MustBeZero),
        ("old_ledgers_view_differs", Rule::MustBeZero),
        ("forbidden_word_property_failures", Rule::MustBeZero),
    ]
    .into_iter()
    .collect();
    assert_eq!(rules, expected);
    assert_eq!(measure("planted_error_ends_failed_pct").unwrap().applies_to, [Set::Q]);
    assert_eq!(measure("final_answer_equals_key").unwrap().rule.kind(), "recorded");
    assert!(measure("made_up").is_none());
}

#[test]
fn gate_never_passes_what_was_not_measured() {
    // Nothing measured: every measure is "not measured" and the gate is undecided.
    let empty = evaluate(&BTreeMap::new()).unwrap();
    assert_eq!(empty.verdict, GateVerdict::Incomplete);
    assert!(empty.results.iter().all(|r| r.outcome == Outcome::NotMeasured));
    assert_eq!(empty.ids_with(Outcome::Pass), Vec::<&str>::new());

    // Rule by rule.
    let rule = |id: &str| measure(id).unwrap().rule;
    assert_eq!(judge(&rule("planted_error_ends_failed_pct"), Some(100.0)), Outcome::Pass);
    assert_eq!(judge(&rule("planted_error_ends_failed_pct"), Some(99.9)), Outcome::Fail);
    assert_eq!(judge(&rule("planted_error_ends_failed_pct"), None), Outcome::NotMeasured);
    assert_eq!(judge(&rule("wrong_carry_overs"), Some(0.0)), Outcome::Pass);
    assert_eq!(judge(&rule("wrong_carry_overs"), Some(1.0)), Outcome::Fail);
    assert_eq!(judge(&rule("statement_recall_tex_theorem_envs"), Some(0.9)), Outcome::Pass);
    assert_eq!(judge(&rule("statement_recall_tex_theorem_envs"), Some(0.89)), Outcome::Fail);
    assert_eq!(judge(&rule("time_to_skeleton_tex_s"), Some(9.99)), Outcome::Pass);
    assert_eq!(judge(&rule("time_to_skeleton_tex_s"), Some(10.0)), Outcome::Fail);
    assert_eq!(judge(&rule("statement_recall"), Some(0.1)), Outcome::RecordedOnly);
    assert_eq!(judge(&rule("statement_recall"), None), Outcome::NotMeasured);

    // Zero of zero is never 100 percent, a share, or a passing zero count.
    assert_eq!(percent(0, 0), None);
    assert_eq!(percent(3, 3), Some(100.0));
    assert_eq!(percent(2, 3).map(|p| p < 100.0), Some(true));
    assert_eq!(share(0, 0), None);
    assert_eq!(share(1, 2), Some(0.5));
    assert_eq!(count_in(0, 0), None);

    // All pass rules satisfied: PASS, even with recorded-only measures missing.
    let mut all: BTreeMap<String, f64> = BTreeMap::new();
    for d in MEASURES {
        match d.rule {
            Rule::Recorded => {}
            Rule::MustEqual(v) => drop(all.insert(d.id.into(), v)),
            Rule::MustBeZero => drop(all.insert(d.id.into(), 0.0)),
            Rule::Threshold(eval_support::measures::Cmp::Below, b) => drop(all.insert(d.id.into(), b / 2.0)),
            Rule::Threshold(eval_support::measures::Cmp::AtLeast, b) => drop(all.insert(d.id.into(), b)),
        }
    }
    let report = evaluate(&all).unwrap();
    assert_eq!(report.verdict, GateVerdict::Pass);
    assert_eq!(report.outcome_of("statement_recall"), Some(Outcome::NotMeasured));

    // One planted error ending anywhere but Failed fails the gate.
    all.insert("planted_error_ends_failed_pct".into(), percent(2, 3).unwrap());
    let report = evaluate(&all).unwrap();
    assert_eq!(report.verdict, GateVerdict::Fail);
    assert_eq!(report.ids_with(Outcome::Fail), ["planted_error_ends_failed_pct"]);

    // One pass rule left out: undecided, and it is named.
    all.insert("planted_error_ends_failed_pct".into(), 100.0);
    all.remove("wrong_statement_located");
    let report = evaluate(&all).unwrap();
    assert_eq!(report.verdict, GateVerdict::Incomplete);
    assert_eq!(report.unmeasured_rules(), ["wrong_statement_located"]);

    // Numbers the table does not know, and non-numbers, are refused.
    assert!(evaluate(&BTreeMap::from([("made_up".to_string(), 1.0)])).is_err());
    assert!(evaluate(&BTreeMap::from([("statement_recall".to_string(), f64::NAN)])).is_err());
    assert!(evaluate(&BTreeMap::from([("wrong_carry_overs".to_string(), -1.0)])).is_err());
}

#[test]
fn recorded_service_mock_routes_and_flags_unexpected_requests() {
    let routes = Routes::load(&fixture_path(&fixtures_dir(), "recorded/selftest.json")).unwrap();
    assert_eq!(routes.len(), 2);
    let mock = RoutedMock::start(routes);
    // The same override the literature tools use for a loopback server.
    let endpoints = LiteratureEndpoints::loopback_mock(&mock.base_url).unwrap();
    let get = |target: &str| fetch_bytes_allowlisted(&format!("{}{target}", mock.base_url), &endpoints, MAX_METADATA_BYTES);

    assert_eq!(get("/selftest/lookup?id=one&format=json").unwrap(), br#"{"selftest":"one"}"#);
    // The order of query parts does not matter; their values do.
    assert_eq!(get("/selftest/lookup?format=json&id=one").unwrap(), br#"{"selftest":"one"}"#);
    let from_file = get("/selftest/file").unwrap();
    assert_eq!(String::from_utf8(from_file).unwrap().trim(), "self-written body served from a file");
    mock.assert_no_unexpected();
    assert_eq!(mock.requests().len(), 3);

    assert!(get("/selftest/lookup?id=two&format=json").is_err(), "an unrecorded request must not be answered");
    assert!(get("/selftest/lookup").is_err());
    let unexpected = mock.unexpected();
    assert_eq!(unexpected.len(), 2);
    assert_eq!(unexpected[0].method, "GET");
    assert_eq!(unexpected[0].target, "/selftest/lookup?id=two&format=json");
    assert_eq!(mock.requests().len(), 5);

    let mut twice = Routes::new();
    twice.add("GET", "/a?x=1&y=2", 200, "text/plain", b"a").unwrap();
    assert!(twice.add("get", "/a?y=2&x=1", 200, "text/plain", b"b").is_err(), "the same route recorded twice");
}

#[test]
fn scripted_model_refuses_a_script_the_run_did_not_use() {
    let script = vec![ScriptStep { id: "m1".into(), stop_reason: "end_turn".into(), content: json!([{ "type": "text", "text": "x" }]) }];
    let model = ScriptedModel::start(&script);
    assert_eq!(model.model_calls(), 0);
    assert!(model.finish().unwrap_err().contains("0 model call(s)"));
    assert!(eval_support::scripted::parse_script("[]").is_err());
    assert!(eval_support::scripted::parse_script(r#"[{"id":"m","stop_reason":"end_turn","content":[],"delay":1}]"#).is_err());
}

/// The whole path on one case: manifest, scripted model through the real
/// agent loop, model-call count, sheet, gate, Markdown.
#[test]
fn scripted_case_runs_end_to_end_and_fills_a_sheet() {
    let set = load_manifests(&fixtures_dir(), Tier::Fixed).unwrap();
    let (case_set, case) = set.case("Q-selftest-gaussian").unwrap();
    assert_eq!((case_set, case.mode), (Set::Q, Mode::Solve));
    let Input::Problem { statement, .. } = &case.input else { panic!("expected a problem") };
    let script = load_script(&fixture_path(&set.dir, &format!("scripts/{}.json", case.id))).unwrap();

    let tmp = TempDir::new("eval-harness");
    let session = "s-eval-selftest";
    let router = make_router(
        &tmp.path,
        session,
        Arc::new(MockApprover::new(true)),
        Arc::new(MockEpoch::new(1, false)),
        Arc::new(AtomicUsize::new(0)),
    );
    let ledger = Ledger::open(&tmp.path).unwrap();
    let model = ScriptedModel::start(&script);
    let mut sink = RecordingSink::new();
    let cfg = default_cfg(statement, false);

    let started = Instant::now();
    let outcome = run_agent_loop(&model.transport, &router, &ledger, &cfg, &mut sink).unwrap();
    let wall_time_s = started.elapsed().as_secs_f64();
    assert_eq!(outcome, SessionOutcome::Completed { turns: 2, stop_reason: "end_turn".into() });

    // One request to the provider is one model call; the ledger agrees with the wire.
    let model_calls = model.finish().unwrap();
    assert_eq!(model_calls, 2);
    let events = ledger.read_session(session).unwrap();
    assert_eq!(events.iter().filter(|e| e.kind == "model.request").count() as u64, model_calls);

    // Compare with the key. Substring matching is enough for this self-test;
    // how Solve compares a final answer with a key is Solve's to define.
    let final_text = events.iter().rev().find(|e| e.kind == "model.text").and_then(|e| e.payload["text"].as_str()).unwrap();
    let equals_key = final_text.contains(case.answer_key["final_answer"].as_str().unwrap());
    assert!(equals_key);

    let mut sheet = Sheet::blank(Tier::Fixed);
    sheet.date = "2026-11-25".into();
    sheet.build = Build { git_commit: Some("0123abc".into()), crate_version: Some(env!("CARGO_PKG_VERSION").into()) };
    sheet.models = vec![cfg.model.clone()];
    sheet.model_calls = Some(model_calls);
    sheet.wall_time_s = Some(wall_time_s);
    sheet.cases_run = vec![case.id.clone()];
    sheet.set_value_opt("final_answer_equals_key", share(u64::from(equals_key), 1));
    sheet.set_value("model_calls_typical", model_calls as f64);
    // No planted-error case or tiered step was run, so 0 of 0: Q measures stay unmeasured.
    SolveQCounts::default().record_into(&mut sheet.values).unwrap();
    sheet.measure_notes.insert("final_answer_equals_key".into(), "self-test | scripted model".into());
    sheet.remarks = "Harness self-test. Not an evaluation run.".into();
    assert_eq!(validate_sheet(&sheet), Vec::<String>::new());

    let report = sheet.gate().unwrap();
    assert_eq!(report.verdict, GateVerdict::Incomplete, "one self-test case decides no gate");
    assert_eq!(report.outcome_of("final_answer_equals_key"), Some(Outcome::RecordedOnly));
    assert_eq!(report.outcome_of("model_calls_typical"), Some(Outcome::RecordedOnly));
    assert_eq!(report.outcome_of(solve_wording::ID_PLANTED_ERROR_ENDS_FAILED_PCT), Some(Outcome::NotMeasured));
    assert_eq!(report.outcome_of(solve_wording::ID_PLANTED_ERROR_ENDS_VERIFIED), Some(Outcome::NotMeasured));
    assert_eq!(report.outcome_of(solve_wording::ID_PLANTED_ERROR_ENDS_CHECKED_NUMERICALLY), Some(Outcome::NotMeasured));
    assert_eq!(report.ids_with(Outcome::Pass), Vec::<&str>::new());

    let out = tempfile::tempdir().unwrap();
    let path = write_sheet(out.path(), &sheet).unwrap();
    assert_eq!(path.file_name().unwrap().to_str().unwrap(), "sheet-2026-11-25-fixed-0123abc.md");
    let md = std::fs::read_to_string(&path).unwrap();
    assert!(md.starts_with("# Evaluation sheet: 2026-11-25, fixed tier\n"));
    assert!(md.contains("## Gate: INCOMPLETE"));
    assert!(md.contains("| Model calls | 2 |"));
    assert!(md.contains("| Models | claude-haiku-4-5 |"));
    assert!(md.contains("| `final_answer_equals_key` | Q | recorded | 1 share | recorded |"));
    assert!(md.contains("| `planted_error_ends_failed_pct` | Q | must equal 100 | not measured | not measured |"));
    assert!(md.contains("| `planted_error_ends_verified` | Q | must be 0 | not measured | not measured |"));
    assert!(md.contains("| `planted_error_ends_checked_numerically` | Q | must be 0 | not measured | not measured |"));
    assert!(md.contains("self-test \\| scripted model"), "a `|` in a note must not break the table");
    assert_eq!(md.lines().filter(|l| l.contains("| not measured | not measured |")).count(), MEASURES.len() - 2);
    // A second run of the same day and build is a second sheet, not an overwrite.
    let second = write_sheet(out.path(), &sheet).unwrap();
    assert_eq!(second.file_name().unwrap().to_str().unwrap(), "sheet-2026-11-25-fixed-0123abc-2.md");

    // The sheet is data: it survives JSON, and an unknown field or measure is refused.
    let text = serde_json::to_string(&sheet).unwrap();
    assert_eq!(parse_sheet(&text).unwrap(), sheet);
    assert!(parse_sheet(&text.replacen("\"date\"", "\"when\"", 1)).is_err());
    sheet.set_value("made_up", 1.0);
    assert!(write_sheet(out.path(), &sheet).is_err());
    // A sheet with no identity is not filed.
    let errs = validate_sheet(&Sheet::blank(Tier::Live));
    assert!(errs.iter().any(|e| e.contains("date")) && errs.iter().any(|e| e.contains("git_commit")), "{errs:?}");
}

/// `sheet-template.md` is the rendering of a blank sheet. Regenerate it
/// with `EVAL_BLESS=1` after changing the measures or the renderer.
#[test]
fn sheet_template_is_the_rendering_of_a_blank_sheet() {
    let rendered = render_markdown(&Sheet::blank(Tier::Live)).unwrap();
    assert!(rendered.contains("## Gate: INCOMPLETE"));
    assert!(!rendered.contains("| pass |"));
    assert!(!rendered.to_ascii_lowercase().contains("computed"));
    let path = sheets_dir().join("sheet-template.md");
    if std::env::var_os("EVAL_BLESS").is_some() {
        std::fs::write(&path, &rendered).unwrap();
    }
    let on_disk = std::fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
    assert_eq!(on_disk, rendered, "docs/design/eval-2026-11/sheet-template.md is stale; rerun with EVAL_BLESS=1");
    assert_eq!(today_utc().len(), 10);
    assert_eq!(current_build(&repo_root(), "0.0.0").crate_version.as_deref(), Some("0.0.0"));
}

/// Live tier, by hand: checks the manifests kept outside the repository.
/// `EVAL_LIVE_MANIFEST_DIR=<dir> cargo test -p agent --test eval_harness -- --ignored live_manifests_validate`
#[test]
#[ignore = "live tier: needs EVAL_LIVE_MANIFEST_DIR"]
fn live_manifests_validate() {
    let dir = std::env::var("EVAL_LIVE_MANIFEST_DIR").expect("set EVAL_LIVE_MANIFEST_DIR to the directory of the live manifests");
    let set = load_manifests(std::path::Path::new(&dir), Tier::Live).unwrap_or_else(|e| panic!("live manifests are invalid:\n{}", e.join("\n")));
    for m in &set.manifests {
        println!("set {}: {} case(s)", m.set.letter(), m.cases.len());
    }
}

/// Live tier, by hand: files one sheet from a filled sheet JSON.
/// `EVAL_SHEET_JSON=<file> cargo test -p agent --test eval_harness -- --ignored file_sheet_from_json`
/// A missing date or build is filled in from today and the checked-out commit.
#[test]
#[ignore = "live tier: needs EVAL_SHEET_JSON"]
fn file_sheet_from_json() {
    let file = std::env::var("EVAL_SHEET_JSON").expect("set EVAL_SHEET_JSON to a filled sheet (JSON)");
    let mut sheet = parse_sheet(&std::fs::read_to_string(&file).unwrap()).unwrap();
    if sheet.date.is_empty() {
        sheet.date = today_utc();
    }
    if sheet.build == Build::default() {
        sheet.build = current_build(&repo_root(), env!("CARGO_PKG_VERSION"));
    }
    let path = write_sheet(&sheets_dir(), &sheet).unwrap();
    println!("filed {} (gate: {})", path.display(), sheet.gate().unwrap().verdict.label());
}
