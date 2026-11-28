//! Set W integration tests (part 1) for the F15 Explore cited-summary write-up adapter
//! (`crates/export/src/writeup_idea.rs`; `00-overview.md` 2.13, 7.2 Set W, 7.3; `explore.md` §6):
//! - `tex_audit` 0 blockers / 0 errors + `.bib` `citekey` and read-only fallback (`Tex`/`Md`/`Bib`)
//! - All 6 F12 `PriorWorkStatus` states + Rule V6 `confirm_same` + Rule V9 origin exclusion
//! - Rule V7 tampered `idea.verdict` / missing brief refusal + Rule V10 lesson & mark invariance.
//!
//! Zero third-party text, zero network calls, zero model calls, zero `#[ignore]` tests.

mod common_writeup_idea;

use common_writeup_idea::{
    append_arxiv_abstract_evidence, append_confirm_same, append_explore_start_and_brief,
    append_map_and_honest_verdict, append_search_and_triage, ingest_and_record_xe,
    setup_library_with_citekey, SYNTH_PAPER_A_TEX,
};
use export::writeup::{writeup_wording, WriteupOptions};
use export::writeup_idea::explore_writeup_wording;
use export::{writeup_session_bytes, writeup_session_to_path, WriteupFormat};
use ledger::Ledger;
use research_tools::paper_key::{PaperIds, PaperKey, PaperRef};
use research_tools::prior_work::{PriorWorkStatus, IDEA_VERDICT_EVENT};
use research_tools::prior_work_wording::wording as pw_wording;
use research_tools::search_coverage::IDEA_NOTE_EVENT;
use research_tools::{audit_tex_sources, AuditOptions, ResearchEnv};
use serde_json::json;
use std::fs;
use tempfile::TempDir;

#[test]
fn set_w_tex_audit_citekey_and_readonly_library_fallback_across_tex_md_bib() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ws_dir = tmp.path().join("ws");
    fs::create_dir_all(&ws_dir).unwrap();

    let paper_a = PaperRef::from_ids(
        PaperIds { arxiv: Some("2403.01111".into()), ..Default::default() },
        Some("v1".into()),
    )
    .unwrap();
    let paper_b = PaperRef::from_ids(
        PaperIds { arxiv: Some("2403.02222".into()), ..Default::default() },
        Some("v1".into()),
    )
    .unwrap();

    // First run WITHOUT library.db: must use deterministic fallback and NEVER create library.db.
    let sid = "s-exp-tex-audit";
    let ledger = Ledger::open(&data_dir).unwrap();
    append_explore_start_and_brief(
        &ledger,
        sid,
        "standard",
        "Check whether Krylov slope scales linearly in monitored chains and spreads via damping.",
        "Linear Krylov slope and spread complexity damping in monitored chains.",
        &[
            ("sc-1", "Krylov slope scales linearly with lattice size.", "Krylov slope scales linearly"),
            ("sc-2", "No finite-size drift appears at third order.", "monitored chains"),
        ],
        Some(json!({ "searched": true, "papers": 42, "with_full_text": 30, "hits": 2 })),
    );
    append_search_and_triage(
        &ledger,
        sid,
        vec![
            json!({ "qid": "q-1", "provider": "arxiv", "status": "ok", "count": 2 }),
            json!({ "qid": "q-2", "provider": "inspire", "status": "ok", "count": 2 }),
        ],
        vec![
            json!({ "key": paper_a.key, "title": "Linear Boundary Chains", "authors": ["Voss, Elena"], "year": 2024, "ids": { "arxiv": "2403.01111" } }),
            json!({ "key": paper_b.key, "title": "Dimer Damping Study", "authors": ["Chen, Li"], "year": 2023, "ids": { "arxiv": "2403.02222" } }),
        ],
        vec![
            json!({ "key": paper_a.key, "relevance": "core", "subclaim_ids": ["sc-1"], "why": "Direct." }),
            json!({ "key": paper_b.key, "relevance": "background", "subclaim_ids": ["sc-2"], "why": "Context." }),
        ],
        None,
    );
    let q_txt = "In a finite monitored chain the Krylov slope scales linearly with lattice size across all even lengths.";
    let (_ing, xe_seq) = ingest_and_record_xe(
        &data_dir, &ledger, sid, &paper_a.key, "paper_a.tex", SYNTH_PAPER_A_TEX, 4, q_txt, &["sc-1"], "tex",
    );
    append_map_and_honest_verdict(
        &ledger,
        sid,
        vec![json!({
            "subclaim_id": "sc-1",
            "paper_key": paper_a.key,
            "relation": "same",
            "evidence": [format!("xe-{xe_seq}")],
            "difference": "Uses open boundary conditions instead of periodic loops."
        })],
    );
    ledger
        .append_with_hash(sid, "idea.stage", 1, &json!({ "stage": "map", "state": "finished" }))
        .unwrap();

    let opts = WriteupOptions::default();
    let (_, tex_no_lib) = writeup_session_bytes(&data_dir, sid, WriteupFormat::Tex, &opts).unwrap();
    let (_, md_no_lib) = writeup_session_bytes(&data_dir, sid, WriteupFormat::Md, &opts).unwrap();
    let (_, bib_no_lib) = writeup_session_bytes(&data_dir, sid, WriteupFormat::Bib, &opts).unwrap();
    assert!(
        !data_dir.join("library").join("library.db").exists(),
        "write-up must not create library.db when absent"
    );
    let bib_fallback = String::from_utf8(bib_no_lib).unwrap();
    assert!(bib_fallback.contains("voss2024_"), "expected deterministic surname+year fallback key: {bib_fallback}");
    let tex_fallback = String::from_utf8(tex_no_lib).unwrap();
    assert!(tex_fallback.contains(r"\cite{voss2024_"));
    let md_fallback = String::from_utf8(md_no_lib).unwrap();
    assert!(md_fallback.contains("[@voss2024_"));

    // Now populate user library with pinned `source_item.citekey` and verify all 3 formats + tex_audit.
    setup_library_with_citekey(
        &data_dir, &paper_a, "voss2024krylov", "Linear Boundary Chains", "Voss, Elena", 2024,
    );
    let out_tex = ws_dir.join("explore_summary.tex");
    writeup_session_to_path(&data_dir, sid, WriteupFormat::Tex, &opts, &out_tex).unwrap();
    let disk_tex = fs::read_to_string(&out_tex).unwrap();
    let disk_bib = fs::read_to_string(ws_dir.join("explore_summary.bib")).unwrap();
    let (_, md_lib) = writeup_session_bytes(&data_dir, sid, WriteupFormat::Md, &opts).unwrap();
    let md_str = String::from_utf8(md_lib).unwrap();

    assert!(disk_tex.contains(r"\cite{voss2024krylov}"));
    assert!(disk_bib.contains("@article{voss2024krylov,"));
    assert!(md_str.contains("[@voss2024krylov,"));
    assert!(md_str.contains("your library: 42 papers, 30 with full text, 2 hits"));
    assert!(md_str.contains(writeup_wording::COVERAGE_HEADING));
    assert!(md_str.contains(writeup_wording::CHECK_TABLE_HEADING));

    let env = ResearchEnv::with_workspace(&data_dir, &ws_dir).unwrap();
    let audit = audit_tex_sources(&env, sid, Some(&ledger), &AuditOptions::default()).unwrap();
    assert!(audit.ok, "tex_audit failed: {:?}", audit.issues);
    assert_eq!(audit.blocker_count, 0);
    assert_eq!(audit.error_count, 0);
}

#[test]
fn set_w_all_six_f12_statuses_confirm_same_v6_and_origin_exclusion_v9() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ledger = Ledger::open(&data_dir).unwrap();
    let sid = "s-exp-all-6";

    let origin_key = PaperKey::from_arxiv("2001.00001").unwrap();
    let paper_a = PaperKey::from_arxiv("2404.01001").unwrap();
    let paper_abs_id = "2404.02002";

    ledger
        .append_with_hash(
            sid,
            "handoff.packet",
            1,
            &json!({
                "from": {
                    "session": "s-inv-origin",
                    "mode": "investigate",
                    "id": "n-1",
                    "paper": { "key": origin_key, "ids": { "arxiv": "2001.00001" } }
                },
                "papers": [{ "key": origin_key, "ids": { "arxiv": "2001.00001" } }]
            }),
        )
        .unwrap();

    append_explore_start_and_brief(
        &ledger,
        sid,
        "standard",
        "Check linear Krylov slope, saturation damping, partial scaling, abstract link, and gap.",
        "Five-part monitored chain exploration.",
        &[
            ("sc-1", "Linear Krylov slope holds.", "linear Krylov slope"),
            ("sc-2", "Partial scaling in open chains.", "partial scaling"),
            ("sc-3", "Saturation under another name.", "saturation damping"),
            ("sc-4", "Abstract-only link to dimer chains.", "abstract link"),
            ("sc-5", "Open gap with origin paper excluded.", "and gap"),
        ],
        None,
    );
    let (_abs_key, xa_seq) = append_arxiv_abstract_evidence(
        &ledger,
        sid,
        paper_abs_id,
        "Abstract Only Dimer Study",
        "We observe a dimer correlation plateau in finite chains.",
        "dimer correlation plateau in finite chains",
        &["sc-4"],
    );
    append_search_and_triage(
        &ledger,
        sid,
        vec![
            json!({ "qid": "q-1", "provider": "arxiv", "status": "ok", "count": 3 }),
            json!({ "qid": "q-2", "provider": "inspire", "status": "ok", "count": 3 }),
        ],
        vec![
            json!({ "key": origin_key, "title": "Origin Paper", "year": 2020 }),
            json!({ "key": paper_a, "title": "Full-Text Chain Paper", "authors": ["Lin, Bo"], "year": 2024 }),
            json!({ "key": format!("arxiv:{paper_abs_id}"), "title": "Abstract Only Dimer Study", "authors": ["Patel, Dev"], "year": 2024 }),
        ],
        vec![
            json!({ "key": origin_key, "relevance": "core", "subclaim_ids": ["sc-5"], "why": "Origin." }),
            json!({ "key": paper_a, "relevance": "core", "subclaim_ids": ["sc-1", "sc-2", "sc-3"], "why": "Core." }),
            json!({ "key": format!("arxiv:{paper_abs_id}"), "relevance": "related", "subclaim_ids": ["sc-4"], "why": "Abstract." }),
        ],
        None,
    );
    let q_origin = "In a finite monitored chain the Krylov slope scales linearly with lattice size across all even lengths.";
    let (_, xe_origin) = ingest_and_record_xe(
        &data_dir, &ledger, sid, &origin_key, "origin.tex", SYNTH_PAPER_A_TEX, 4, q_origin, &["sc-5"], "tex",
    );
    let q_same = "In a finite monitored chain the Krylov slope scales linearly with lattice size across all even lengths.";
    let (_, xe_same) = ingest_and_record_xe(
        &data_dir, &ledger, sid, &paper_a, "paper_a.tex", SYNTH_PAPER_A_TEX, 4, q_same, &["sc-1", "sc-2"], "tex",
    );
    let q_other = "We refer to this saturation regime as spread complexity damping throughout the analysis.";
    let (_, xe_other) = ingest_and_record_xe(
        &data_dir, &ledger, sid, &paper_a, "paper_a2.tex", SYNTH_PAPER_A_TEX, 5, q_other, &["sc-3"], "tex",
    );
    append_map_and_honest_verdict(
        &ledger,
        sid,
        vec![
            json!({ "subclaim_id": "sc-1", "paper_key": paper_a, "relation": "same", "evidence": [format!("xe-{xe_same}")], "difference": "Periodic vs open." }),
            json!({ "subclaim_id": "sc-2", "paper_key": paper_a, "relation": "partial", "evidence": [format!("xe-{xe_same}")], "difference": "Even sizes only." }),
            json!({ "subclaim_id": "sc-3", "paper_key": paper_a, "relation": "other_name", "evidence": [format!("xe-{xe_other}")], "other_name": { "term": "spread complexity damping" }, "difference": "Different terminology." }),
            json!({ "subclaim_id": "sc-5", "paper_key": origin_key, "relation": "same", "evidence": [format!("xe-{xe_origin}")], "difference": "Origin self-match must be excluded." }),
        ],
    );

    let opts = WriteupOptions::default();
    let (_, md_before) = writeup_session_bytes(&data_dir, sid, WriteupFormat::Md, &opts).unwrap();
    let md_b = String::from_utf8(md_before).unwrap();
    assert!(md_b.contains(pw_wording::CHIP_JUDGED_SAME_UNCONFIRMED), "{md_b}");
    assert!(md_b.contains(pw_wording::CHIP_JUDGED_PARTIAL_UNCONFIRMED), "{md_b}");
    assert!(md_b.contains(pw_wording::CHIP_JUDGED_OTHER_NAME_UNCONFIRMED), "{md_b}");
    assert!(md_b.contains(pw_wording::CHIP_ABSTRACT_ONLY), "{md_b}");
    assert!(md_b.contains(pw_wording::GRADE_ABSTRACT_ONLY), "{md_b}");
    assert!(md_b.contains(&format!("xa-{xa_seq}")), "{md_b}");
    assert!(md_b.contains(pw_wording::CHIP_NONE_IN_SCOPE), "{md_b}");
    assert!(md_b.contains(explore_writeup_wording::ORIGIN_EXCLUDED_LABEL), "{md_b}");

    append_confirm_same(&ledger, sid, "sc-1");
    append_confirm_same(&ledger, sid, "sc-2");
    append_confirm_same(&ledger, sid, "sc-3");
    let (_, md_after) = writeup_session_bytes(&data_dir, sid, WriteupFormat::Md, &opts).unwrap();
    let md_a = String::from_utf8(md_after).unwrap();
    assert!(md_a.contains(pw_wording::CHIP_JUDGED_SAME_CONFIRMED), "{md_a}");
    assert!(md_a.contains(pw_wording::CHIP_JUDGED_PARTIAL_CONFIRMED), "{md_a}");
    assert!(md_a.contains(pw_wording::CHIP_JUDGED_OTHER_NAME_CONFIRMED), "{md_a}");
    assert!(md_a.contains(pw_wording::READER_CONFIRMED_PREFIX), "{md_a}");

    // 6th F12 status: `Incomplete` (Quick preset + private library never satisfies 2-service gate).
    let sid_q = "s-exp-quick-inc";
    append_explore_start_and_brief(
        &ledger,
        sid_q,
        "quick",
        "Check quick preset incomplete coverage.",
        "Quick preset check.",
        &[("sc-1", "Quick claim.", "quick preset")],
        Some(json!({ "searched": true, "papers": 500, "with_full_text": 450, "hits": 12 })),
    );
    append_search_and_triage(
        &ledger,
        sid_q,
        vec![json!({ "qid": "q-1", "provider": "arxiv", "status": "ok", "count": 1 })],
        vec![json!({ "key": "arxiv:2405.00001", "title": "Single Candidate", "year": 2024 })],
        vec![json!({ "key": "arxiv:2405.00001", "relevance": "unrelated", "subclaim_ids": [], "why": "Off." })],
        None,
    );
    append_map_and_honest_verdict(&ledger, sid_q, vec![]);
    let (_, md_q) = writeup_session_bytes(&data_dir, sid_q, WriteupFormat::Md, &opts).unwrap();
    let md_qs = String::from_utf8(md_q).unwrap();
    assert!(md_qs.contains(pw_wording::CHIP_INCOMPLETE), "{md_qs}");
    assert!(md_qs.contains("No close match among 1 abstract; nothing was read."), "{md_qs}");
    assert!(md_qs.contains("private library does not count"), "{md_qs}");
    assert!(md_qs.contains("your library: 500 papers, 450 with full text, 12 hits"), "{md_qs}");
}

#[test]
fn set_w_rule_v7_tampered_verdict_refusal_and_v10_lesson_mark_invariance() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ledger = Ledger::open(&data_dir).unwrap();

    // Missing idea.brief fails closed.
    let sid_nobrief = "s-exp-nobrief";
    ledger.append_with_hash(sid_nobrief, "idea.started", 1, &json!({ "preset": "standard" })).unwrap();
    ledger
        .append_with_hash(sid_nobrief, "idea.stated", 1, &json!({ "rev": 1, "text": "Only stated." }))
        .unwrap();
    let nb_err = writeup_session_bytes(&data_dir, sid_nobrief, WriteupFormat::Md, &WriteupOptions::default())
        .unwrap_err();
    assert!(nb_err.contains("idea.brief"), "{nb_err}");

    // Tampered stored `idea.verdict` (claims `none_in_scope` when gate failed) MUST fail closed (Rule V7).
    let sid_tamper = "s-exp-v7-tamper";
    append_explore_start_and_brief(
        &ledger,
        sid_tamper,
        "standard",
        "Tampered verdict refusal check.",
        "Tamper test.",
        &[("sc-1", "Claim under failed gate.", "verdict refusal")],
        None,
    );
    append_search_and_triage(
        &ledger,
        sid_tamper,
        vec![json!({ "qid": "q-1", "provider": "arxiv", "status": "failed", "count": 0 })],
        vec![],
        vec![],
        None,
    );
    let evs = ledger.read_session(sid_tamper).unwrap();
    let mut forged = research_tools::idea_verdict::compute(&evs, None);
    forged.verdicts[0].status = PriorWorkStatus::NoneInScope;
    forged.verdicts[0].chip = pw_wording::CHIP_NONE_IN_SCOPE.into();
    ledger
        .append_with_hash(sid_tamper, IDEA_VERDICT_EVENT, 1, &serde_json::to_value(&forged).unwrap())
        .unwrap();
    let v7_err = writeup_session_bytes(&data_dir, sid_tamper, WriteupFormat::Tex, &WriteupOptions::default())
        .unwrap_err();
    assert!(v7_err.contains("Rule V7"), "expected Rule V7 refusal, got: {v7_err}");

    // Rule V10: identical sessions with and without `lessons.supplied` + non-confirmation marks
    // produce byte-identical TeX and Markdown write-ups.
    for (sid, with_lessons) in [("s-v10-clean", false), ("s-v10-lessons", true)] {
        append_explore_start_and_brief(
            &ledger,
            sid,
            "standard",
            "Lesson invariance test for monitored chains.",
            "Lesson invariance check.",
            &[("sc-1", "Scaling holds in chains.", "monitored chains")],
            None,
        );
        if with_lessons {
            ledger
                .append_with_hash(
                    sid,
                    "lessons.supplied",
                    1,
                    &json!({ "run_id": "r-1", "lesson_ids": ["les-1"], "blob": "a".repeat(64) }),
                )
                .unwrap();
            ledger
                .append_with_hash(
                    sid,
                    IDEA_NOTE_EVENT,
                    1,
                    &json!({ "target_kind": "candidate", "target_id": "arxiv:2401.00009", "action": "key_paper", "comment": "important" }),
                )
                .unwrap();
        }
        append_search_and_triage(
            &ledger,
            sid,
            vec![
                json!({ "qid": "q-1", "provider": "arxiv", "status": "ok", "count": 0 }),
                json!({ "qid": "q-2", "provider": "inspire", "status": "ok", "count": 0 }),
            ],
            vec![],
            vec![],
            None,
        );
        append_map_and_honest_verdict(&ledger, sid, vec![]);
    }
    let opts = WriteupOptions::default();
    let (_, tex_a) = writeup_session_bytes(&data_dir, "s-v10-clean", WriteupFormat::Tex, &opts).unwrap();
    let (_, tex_b) = writeup_session_bytes(&data_dir, "s-v10-lessons", WriteupFormat::Tex, &opts).unwrap();
    let (_, md_a) = writeup_session_bytes(&data_dir, "s-v10-clean", WriteupFormat::Md, &opts).unwrap();
    let (_, md_b) = writeup_session_bytes(&data_dir, "s-v10-lessons", WriteupFormat::Md, &opts).unwrap();
    assert_eq!(tex_a, tex_b, "Rule V10: lessons/marks must not change TeX write-up");
    assert_eq!(md_a, md_b, "Rule V10: lessons/marks must not change Markdown write-up");
}
