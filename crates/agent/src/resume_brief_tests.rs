//! Unit tests for `resume_brief.rs` (byte determinism, closure/open items, last-4 turns, R24).

use super::{build_resume_brief, LESSONS_HEADING, NO_SURVIVING_PYTHON_VARS_LINE};
use ledger::Event;
use research_tools::lessons::LessonsStore;
use research_tools::lessons_schema::ConventionDefault;
use research_tools::solve_model::{compute_math_sha256, event_kinds};
use serde_json::{json, Value};

fn mk_event(seq: u64, kind: &str, payload: Value) -> Event {
    Event {
        session: "s-brief".into(),
        seq,
        ts_ms: 1_700_000_000_000 + (seq as i64) * 10,
        kind: kind.to_string(),
        schema_version: 1,
        payload,
        prev_hash: format!("prev-{seq}"),
        event_hash: format!("hash-{seq}"),
    }
}

fn setup_event(seq: u64) -> Event {
    let math_sha256 = compute_math_sha256(&[], &Default::default(), &[], "Natural units");
    mk_event(
        seq,
        event_kinds::SOLVE_SETUP,
        json!({
            "revision": 1,
            "statement": "Compute I = int exp(-x^2) dx",
            "task_kind": "calculate",
            "target": { "description": "Exact closed form", "form": "closed_form" },
            "done": [
                { "id": "c-final", "kind": "final_tier_at_least", "params": { "tier": "verified" } }
            ],
            "conventions": "Natural units",
            "symbols": [{ "name": "x", "assumptions": ["real"] }],
            "dimensions": {},
            "assumptions": [],
            "sources": [],
            "roles": {
                "solver": { "provider": "anthropic", "model": "claude-sonnet-4-5" },
                "verifier": { "provider": "openai", "model": "gpt-4.1" }
            },
            "budgets": {
                "max_model_calls": 60,
                "max_wall_minutes": 30,
                "max_model_calls_per_leg": 48,
                "max_review_loops": 6,
                "max_gate_bounces": 3
            },
            "review": { "mode": "prose_only" },
            "tool_profile": "solve/1",
            "math_sha256": math_sha256
        }),
    )
}

#[test]
fn brief_is_byte_deterministic_and_preserves_required_sections_and_last_4_turns() {
    let mut events = vec![
        mk_event(1, event_kinds::SOLVE_STARTED, json!({ "origin": "new" })),
        setup_event(2),
        mk_event(
            3,
            event_kinds::SOLVE_EXPR,
            json!({
                "name": "I_exact",
                "version": 1,
                "srepr_blob": "b1",
                "srepr_sha256": "s1",
                "latex": "\\sqrt{\\pi}",
                "str_preview": "sqrt(pi)",
                "free_symbols": [],
                "produced_by": { "call_id": "call-1", "result_seq": 1 },
                "owner_entry": "e-2",
                "portable": true
            }),
        ),
        mk_event(
            4,
            event_kinds::PROOF_ENTRY,
            json!({
                "entry_id": "e-1",
                "entry_type": "assumption",
                "summary": "Real x variable",
                "details": "Assume x is real.",
                "depends_on": []
            }),
        ),
        mk_event(
            5,
            event_kinds::PROOF_ENTRY,
            json!({
                "entry_id": "e-2",
                "entry_type": "symbolic_computation",
                "summary": "Gaussian integral equals sqrt(pi)",
                "details": "Detailed contour/polar calculation.",
                "depends_on": ["e-1"],
                "claims": [{
                    "form": "equation",
                    "lhs": "I_exact@1",
                    "rhs": "I_exact@1",
                    "relation": "eq"
                }],
                "claim_sha256": "cs-2"
            }),
        ),
        mk_event(
            6,
            event_kinds::SOLVE_CHECK,
            json!({
                "check_id": "chk-1",
                "claim_sha256": "cs-2",
                "claim_index": 0,
                "entry_id": "e-2",
                "math_sha256": "m",
                "name": "counterexample_search",
                "registry_sha256": "r",
                "params": {},
                "outcome": "fail",
                "asked_by": "host",
                "receipt": { "call_id": "call-2", "result_seq": 6 },
                "evidence": { "counterexample": { "x": 1 } },
                "evidence_sha256": "ev1"
            }),
        ),
        mk_event(
            7,
            event_kinds::TOOL_REQUEST,
            json!({ "call_id": "uninterpreted-call-9", "tool": "sympy_eval", "caller": "solver:solve" }),
        ),
        mk_event(
            8,
            event_kinds::TOOL_RESULT,
            json!({ "call_id": "uninterpreted-call-9", "tool": "sympy_eval", "ok": true }),
        ),
    ];
    for t in 1..=6u64 {
        let base = 10 + t * 3;
        events.push(mk_event(base, "model.request", json!({ "turn": t })));
        events.push(mk_event(
            base + 1,
            "model.text",
            json!({ "turn": t, "text": format!("TURN_{t}_VERBATIM") }),
        ));
    }

    let b1 = build_resume_brief(&events, 100, 16_000);
    let b2 = build_resume_brief(&events, 100, 16_000);
    assert_eq!(b1, b2, "brief must be byte-deterministic");
    assert!(b1.contains(NO_SURVIVING_PYTHON_VARS_LINE));
    assert!(b1.contains("I_exact@1 (portable, owner=e-2): `sqrt(pi)`"));
    assert!(b1.contains("Failed check `counterexample_search`"));
    assert!(b1.contains("uninterpreted-call-9"));
    assert!(b1.contains("Detailed contour/polar calculation."));
    assert!(!b1.contains("TURN_1_VERBATIM"));
    assert!(!b1.contains("TURN_2_VERBATIM"));
    for t in 3..=6 {
        assert!(b1.contains(&format!("TURN_{t}_VERBATIM")), "missing turn {t}");
    }

    let capped = build_resume_brief(&events, 100, 300);
    assert!(capped.chars().count() <= 300);
    assert!(capped.contains(NO_SURVIVING_PYTHON_VARS_LINE));
}

#[test]
fn brief_reads_frozen_lessons_supplied_only_and_never_live_lessons_db_r24() {
    let dir = tempfile::tempdir().unwrap();
    let store = LessonsStore::open(dir.path()).unwrap();
    store
        .record_convention_default(&ConventionDefault {
            key: "metric_signature".into(),
            value: "LIVE_DB_ONLY_SECRET_DO_NOT_READ".into(),
            domain: "qft".into(),
            origin_session: "other-session".into(),
            updated_ms: 1_700_000_000_000,
        })
        .unwrap();

    let frozen_blob = "Use mostly-minus signature (+,-,-,-) for momenta.";
    let events = vec![
        mk_event(1, event_kinds::SOLVE_STARTED, json!({ "origin": "new" })),
        setup_event(2),
        mk_event(
            3,
            event_kinds::LESSONS_SUPPLIED,
            json!({
                "run_id": "run-1",
                "lesson_ids": ["L-1"],
                "blob": frozen_blob,
            }),
        ),
    ];

    let brief = build_resume_brief(&events, 3, 8_000);
    assert!(brief.contains(LESSONS_HEADING));
    assert!(brief.contains(frozen_blob));
    assert!(!brief.contains("LIVE_DB_ONLY_SECRET_DO_NOT_READ"));

    // Respects `upto_seq`: earlier slice omits later lessons.supplied events.
    let earlier = build_resume_brief(&events, 2, 8_000);
    assert!(!earlier.contains(LESSONS_HEADING));
    assert!(!earlier.contains(frozen_blob));
}
