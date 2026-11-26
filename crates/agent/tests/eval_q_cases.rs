//! Structural, S2-badge, and scripted-runner validation for the 15 fixed-tier
//! Solve Set Q evaluation cases (`tests/fixtures/eval/manifest-q.json`).

#[allow(dead_code)]
mod eval_support;
#[allow(dead_code)]
mod support;

use agent::{run_agent_loop, SessionOutcome};
use eval_support::manifest::{Input, Set, SolveTask, Tier};
use eval_support::scripted::{load_script, ScriptedModel};
use eval_support::{fixture_path, fixtures_dir, load_manifests};
use ledger::Ledger;
use research_tools::solve_status::Tier as SolveTier;
use std::collections::BTreeMap;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use support::{default_cfg, make_router, MockApprover, MockEpoch, RecordingSink, TempDir};

fn parse_s2_badge(label: &str) -> Option<SolveTier> {
    const ALL: [SolveTier; 11] = [
        SolveTier::Failed,
        SolveTier::Stale,
        SolveTier::AssumedByModel,
        SolveTier::Unchecked,
        SolveTier::ModelAgreedSameModel,
        SolveTier::ModelAgreed,
        SolveTier::CheckedByYou,
        SolveTier::CheckedNumerically,
        SolveTier::Verified,
        SolveTier::Given,
        SolveTier::Cited,
    ];
    ALL.into_iter().find(|t| t.label() == label)
}

#[test]
fn fixed_q_evaluation_cases_validate_keys_badges_and_scripts() {
    let set = load_manifests(&fixtures_dir(), Tier::Fixed).unwrap();
    let eval_cases: Vec<_> = set
        .cases_of(Set::Q)
        .into_iter()
        .filter(|c| !c.id.starts_with("Q-selftest-"))
        .collect();
    assert_eq!(eval_cases.len(), 15, "10 calculate + 3 check_derivation + 2 prove");

    let (mut n_calc, mut n_check, mut n_prove) = (0usize, 0usize, 0usize);
    assert!(
        eval_cases.iter().any(|c| c.id == "Q-calc-01-effective-potential"),
        "effective-potential case required"
    );

    for case in &eval_cases {
        let Input::Problem { task, statement, steps: input_steps } = &case.input else {
            panic!("{}: Q case must be a problem", case.id);
        };
        match task {
            SolveTask::Calculate => n_calc += 1,
            SolveTask::CheckDerivation => n_check += 1,
            SolveTask::Prove => n_prove += 1,
        }
        let key = &case.answer_key;
        let final_answer = key["final_answer"].as_str().unwrap();
        assert!(!final_answer.trim().is_empty(), "{}: empty final_answer", case.id);
        let expected_final = parse_s2_badge(key["expected_final_badge"].as_str().unwrap())
            .unwrap_or_else(|| panic!("{}: invalid expected_final_badge", case.id));
        let key_steps = key["steps"].as_array().unwrap();
        assert!(!key_steps.is_empty(), "{}: empty steps", case.id);

        let mut eff_by_id: BTreeMap<&str, SolveTier> = BTreeMap::new();
        let mut failed_own_ids: Vec<&str> = Vec::new();
        for st in key_steps {
            let sid = st["id"].as_str().unwrap();
            assert!(
                !sid.is_empty() && !eff_by_id.contains_key(sid),
                "{}: duplicate/empty step id {sid}",
                case.id
            );
            let own = parse_s2_badge(st["expected_own_tier"].as_str().unwrap())
                .unwrap_or_else(|| panic!("{}.{sid}: invalid S2 own badge", case.id));
            let exp_eff = parse_s2_badge(st["expected_tier"].as_str().unwrap())
                .unwrap_or_else(|| panic!("{}.{sid}: invalid S2 effective badge", case.id));
            if own == SolveTier::Failed {
                failed_own_ids.push(sid);
            }
            if st["check_kind"].as_str() == Some("review_prose") {
                assert!(
                    !own.is_computed() && !exp_eff.is_computed(),
                    "{}.{sid}: prose cannot earn computed badge",
                    case.id
                );
            }
            let deps = st["depends_on"].as_array().unwrap();
            let mut dep_effs = Vec::with_capacity(deps.len());
            for d in deps {
                let did = d.as_str().unwrap();
                dep_effs.push(
                    *eff_by_id
                        .get(did)
                        .unwrap_or_else(|| panic!("{}.{sid}: dep `{did}` missing", case.id)),
                );
            }
            if own == SolveTier::Failed || dep_effs.contains(&SolveTier::Failed) {
                assert_eq!(exp_eff, SolveTier::Failed, "{}.{sid}: failed closure must end Failed", case.id);
            } else {
                let min_dep = dep_effs
                    .iter()
                    .copied()
                    .filter(|t| !t.is_neutral())
                    .map(SolveTier::lattice_rank)
                    .min();
                let want_rank = min_dep.map_or(own.lattice_rank(), |m| m.min(own.lattice_rank()));
                assert_eq!(exp_eff.lattice_rank(), want_rank, "{}.{sid}: effective rank mismatch", case.id);
            }
            eff_by_id.insert(sid, exp_eff);
        }
        let last_id = key_steps.last().unwrap()["id"].as_str().unwrap();
        assert_eq!(
            expected_final, eff_by_id[last_id],
            "{}: expected_final_badge must match final step",
            case.id
        );

        let script_path = fixture_path(&set.dir, &format!("scripts/{}.json", case.id));
        match task {
            SolveTask::CheckDerivation => {
                assert_eq!(input_steps.as_ref().unwrap().len(), key_steps.len(), "{}: step count mismatch", case.id);
                let planted_id = key["planted_step"].as_str().expect("check_derivation requires planted_step");
                assert_eq!(failed_own_ids, [planted_id], "{}: must have exactly one planted failing step", case.id);
                assert_eq!(eff_by_id[planted_id], SolveTier::Failed);
                assert_eq!(expected_final, SolveTier::Failed);
                assert!(
                    !script_path.exists(),
                    "{}: planted case awaits S9/S11, must not have a fake script",
                    case.id
                );
            }
            SolveTask::Calculate | SolveTask::Prove => {
                assert!(key["planted_step"].is_null(), "{}: non-planted case must have null planted_step", case.id);
                if *task == SolveTask::Prove {
                    assert_eq!(expected_final, SolveTier::ModelAgreed, "{}: prose proof capped at Model agreed", case.id);
                }
                let script = load_script(&script_path).unwrap_or_else(|e| panic!("{}: {e}", case.id));
                for turn in &script {
                    for blk in turn.content.as_array().unwrap() {
                        if blk["type"] == "tool_use" {
                            let name = blk["name"].as_str().unwrap();
                            assert!(matches!(name, "ledger_note" | "sympy_eval"), "{}: unmerged tool `{name}`", case.id);
                        }
                    }
                }
                let tmp = TempDir::new(&format!("eval-q-{}", case.id));
                let session = format!("s-eval-{}", case.id);
                let router = make_router(
                    &tmp.path,
                    &session,
                    Arc::new(MockApprover::new(true)),
                    Arc::new(MockEpoch::new(1, false)),
                    Arc::new(AtomicUsize::new(0)),
                );
                let ledger = Ledger::open(&tmp.path).unwrap();
                let model = ScriptedModel::start(&script);
                let mut sink = RecordingSink::new();
                let outcome = run_agent_loop(
                    &model.transport,
                    &router,
                    &ledger,
                    &default_cfg(statement, false),
                    &mut sink,
                )
                .unwrap();
                assert_eq!(
                    outcome,
                    SessionOutcome::Completed { turns: script.len() as u32, stop_reason: "end_turn".into() }
                );
                assert_eq!(model.finish().unwrap(), script.len() as u64);
                let events = ledger.read_session(&session).unwrap();
                let final_text = events
                    .iter()
                    .rev()
                    .find(|e| e.kind == "model.text")
                    .and_then(|e| e.payload["text"].as_str())
                    .unwrap();
                assert!(final_text.contains(final_answer), "{}: scripted end_turn missing final_answer", case.id);
            }
        }
    }
    assert_eq!((n_calc, n_check, n_prove), (10, 3, 2));
}
