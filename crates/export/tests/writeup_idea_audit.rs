//! Set W integration tests (part 2) for the F15 Explore cited-summary write-up adapter:
//! - `inline_marks: false` non-removable status/coverage/check-table + Directions optionality + V8
//! - Tampered `xe` drop, non-verbatim OpenAlex `xa` drop, 300-char UTF-8 clip, `(OCR)` + page tag
//! - Rule S8 wording property test across all 7 `ExploreStageState`s and `Tex`/`Md`/`Bib` + determinism.
//!
//! Zero third-party text, zero network calls, zero model calls, zero `#[ignore]` tests.

mod common_writeup_idea;

use common_writeup_idea::{
    append_explore_start_and_brief, append_map_and_honest_verdict, append_ocr_pdf_pages_receipt,
    append_search_and_triage, ingest_and_record_xe, synth_openalex_rebuilt_blob, SYNTH_PAPER_A_TEX,
    SYNTH_PAPER_OCR_TXT,
};
use export::writeup::{contains_forbidden_word, writeup_wording, WriteupOptions};
use export::writeup_idea::{
    build_explore_writeup_detailed, build_explore_writeup_from_verified, explore_writeup_wording,
};
use export::{writeup_session_bytes, WriteupFormat};
use ledger::Ledger;
use research_tools::paper_key::PaperKey;
use research_tools::pdf_pages::PageBasis;
use research_tools::prior_work_wording::contains_forbidden_prior_work_word;
use research_tools::quote_core::{quote_sha256, EVIDENCE_ABSTRACT_EVENT};
use research_tools::run_budget::{explore_stage_run_state, ExploreStageState};
use research_tools::sandbox::SandboxRoot;
use serde_json::json;
use std::fs;
use tempfile::TempDir;

#[test]
fn set_w_inline_marks_false_and_directions_optionality_and_v8_rejection() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ledger = Ledger::open(&data_dir).unwrap();
    let sid = "s-exp-dirs";
    let paper_a = PaperKey::from_arxiv("2406.01234").unwrap();

    append_explore_start_and_brief(
        &ledger,
        sid,
        "standard",
        "Study finite Krylov slope across monitored dimer chains.",
        "Finite Krylov slope in monitored dimer chains.",
        &[("sc-1", "Krylov slope scales linearly.", "Krylov slope")],
        None,
    );
    append_search_and_triage(
        &ledger,
        sid,
        vec![
            json!({ "qid": "q-1", "provider": "arxiv", "status": "ok", "count": 1 }),
            json!({ "qid": "q-2", "provider": "inspire", "status": "ok", "count": 1 }),
        ],
        vec![json!({ "key": paper_a, "title": "Monitored Chain Paper", "authors": ["Oka, Hana"], "year": 2024 })],
        vec![json!({ "key": paper_a, "relevance": "core", "subclaim_ids": ["sc-1"], "why": "Core." })],
        None,
    );
    let q_txt = "In a finite monitored chain the Krylov slope scales linearly with lattice size across all even lengths.";
    let (_, xe_seq) = ingest_and_record_xe(
        &data_dir, &ledger, sid, &paper_a, "paper_a.tex", SYNTH_PAPER_A_TEX, 4, q_txt, &["sc-1"], "tex",
    );
    let (map_seq, _) = append_map_and_honest_verdict(
        &ledger,
        sid,
        vec![json!({
            "subclaim_id": "sc-1",
            "paper_key": paper_a,
            "relation": "partial",
            "evidence": [format!("xe-{xe_seq}")],
            "difference": "Our first calculation checks periodic chains at size 64."
        })],
    );

    // Without `idea.directions`: Directions / Loose ideas headings MUST be omitted cleanly.
    let no_marks = WriteupOptions { include_inline_badges: false, bib_stem: "refs".into() };
    let (_, md_nodir) = writeup_session_bytes(&data_dir, sid, WriteupFormat::Md, &no_marks).unwrap();
    let (_, tex_nodir) = writeup_session_bytes(&data_dir, sid, WriteupFormat::Tex, &no_marks).unwrap();
    let md_nd = String::from_utf8(md_nodir).unwrap();
    let tex_nd = String::from_utf8(tex_nodir).unwrap();
    assert!(!md_nd.contains("## Directions"));
    assert!(!md_nd.contains("## Loose ideas"));
    assert!(!md_nd.contains("*(model)*"), "inline_marks=false must hide inline (model) tags");
    assert!(!tex_nd.contains(r"\textit{(model)}"), "inline_marks=false must hide inline TeX (model) tags");
    assert!(md_nd.contains("**Status:**"), "status line is non-removable");
    assert!(md_nd.contains(writeup_wording::COVERAGE_HEADING), "Where I looked is non-removable");
    assert!(md_nd.contains(writeup_wording::CHECK_TABLE_HEADING), "check table is non-removable");
    assert!(tex_nd.contains(writeup_wording::COVERAGE_HEADING), "Where I looked is non-removable in TeX");
    assert!(tex_nd.contains(writeup_wording::CHECK_TABLE_HEADING), "check table is non-removable in TeX");

    // Add Directions (Sketch listed before Concrete in JSON -> Concrete MUST render before Loose ideas!).
    ledger
        .append_with_hash(
            sid,
            "idea.directions",
            1,
            &json!({
                "map_seq": map_seq,
                "directions": [
                    {
                        "id": "r-2",
                        "title": "Random coupling sketch",
                        "question": "Does disorder alter the saturation plateau?",
                        "concreteness": "sketch",
                        "known": [{ "text": "Clean chain slope is linear", "node_ids": ["k-1"] }],
                        "missing": [{ "text": "Disordered plateau absent in searched scope", "basis": "no_evidence_in_scope" }],
                        "risks": [{ "text": "Finite-size drift may hide scaling", "kind": "technical", "node_ids": [] }],
                        "stop_if": "Variance exceeds mean at L = 128."
                    },
                    {
                        "id": "r-1",
                        "title": "Quadratic bi-Lanczos computation",
                        "question": "What is the slope across the measurement rate threshold?",
                        "concreteness": "concrete",
                        "known": [{ "text": "Linear scaling for even lengths", "node_ids": ["k-1"] }],
                        "missing": [{ "text": "Threshold dependence not found in searched scope", "basis": "open_question(q-1)" }],
                        "first_calculation": {
                            "setup": "Build the quadratic single-particle matrix for L = 64..256 and extract b_n.",
                            "inputs": ["d-1", "k-1"],
                            "method": "numeric",
                            "expected_form": "Piecewise linear slope curve vs measurement rate",
                            "check": { "text": "Zero-measurement limit reproduces clean chain slope", "reproduces": "k-1" },
                            "effort": "days"
                        },
                        "risks": [{ "text": "Trajectory average may differ from mean dynamics", "kind": "technical", "node_ids": ["k-2"] }],
                        "stop_if": "Slopes match within numerical tolerance on both sides."
                    }
                ]
            }),
        )
        .unwrap();

    let with_marks = WriteupOptions::default();
    let (_, md_dir) = writeup_session_bytes(&data_dir, sid, WriteupFormat::Md, &with_marks).unwrap();
    let md_d = String::from_utf8(md_dir).unwrap();
    let pos_conc = md_d.find("## Directions").expect("Concrete directions section");
    let pos_loose = md_d.find("## Loose ideas").expect("Loose ideas section");
    assert!(pos_conc < pos_loose, "Concrete directions must precede Loose ideas");
    assert!(md_d.contains("First calculation:") && md_d.contains("*(model)*"));
    assert!(md_d.contains("k-1") && md_d.contains("q-1"));

    // Rule V8 banned phrase in a direction MUST be refused fail-closed.
    let sid_v8 = "s-exp-dir-v8";
    append_explore_start_and_brief(
        &ledger, sid_v8, "standard", "V8 phrase test.", "Scoped check.",
        &[("sc-1", "Claim.", "phrase test")], None,
    );
    ledger
        .append_with_hash(
            sid_v8,
            "idea.directions",
            1,
            &json!({
                "directions": [{
                    "id": "r-1",
                    "title": "Bad phrasing",
                    "question": "This has never been studied and is completely unexplored.",
                    "concreteness": "sketch"
                }]
            }),
        )
        .unwrap();
    let v8_err = writeup_session_bytes(&data_dir, sid_v8, WriteupFormat::Md, &with_marks).unwrap_err();
    assert!(v8_err.contains("Rule V8"), "expected Rule V8 refusal, got: {v8_err}");
}

#[test]
fn set_w_tampered_xe_drop_nonverbatim_xa_drop_300char_utf8_clip_and_ocr_page_tag() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ledger = Ledger::open(&data_dir).unwrap();
    let sid = "s-exp-ocr-clip";

    let paper_ocr = PaperKey::from_arxiv("2407.03333").unwrap();
    let paper_long = PaperKey::from_arxiv("2407.04444").unwrap();
    let paper_tamper = PaperKey::from_arxiv("2407.05555").unwrap();
    let paper_oa = PaperKey::from_openalex("W99887766").unwrap();

    // Non-repeating multi-byte UTF-8 line > 300 scalar chars (with Greek letters and em-dashes).
    let long_utf8_quote: String = (0..35)
        .map(|i| format!("α{i:02}—β{i:02}γ "))
        .collect::<String>()
        + "boundary scaling tail.";
    assert!(long_utf8_quote.chars().count() > 300);
    let long_tex = format!("\\documentclass{{article}}\n\\begin{{document}}\n{long_utf8_quote}\n\\end{{document}}\n");

    append_explore_start_and_brief(
        &ledger,
        sid,
        "standard",
        "Check OCR scan quotes, long UTF-8 quote clipping, tampered xe drop, and rebuilt xa drop.",
        "Quote integrity and OCR tagging check.",
        &[
            ("sc-1", "Scanned PDF theorem holds.", "OCR scan quotes"),
            ("sc-2", "Long multi-byte quote clips cleanly.", "UTF-8 quote clipping"),
            ("sc-3", "Tampered source quote is dropped.", "tampered xe drop"),
            ("sc-4", "Rebuilt non-verbatim abstract is dropped.", "rebuilt xa drop"),
        ],
        None,
    );
    append_search_and_triage(
        &ledger,
        sid,
        vec![
            json!({ "qid": "q-1", "provider": "arxiv", "status": "ok", "count": 4 }),
            json!({ "qid": "q-2", "provider": "inspire", "status": "ok", "count": 4 }),
        ],
        vec![
            json!({ "key": paper_ocr, "title": "Old Scanned Chain Paper", "authors": ["Kerr, Sam"], "year": 1988 }),
            json!({ "key": paper_long, "title": "Unicode Scaling Paper", "authors": ["Mori, Ren"], "year": 2024 }),
            json!({ "key": paper_tamper, "title": "Tampered File Paper", "authors": ["Noor, Ali"], "year": 2024 }),
            json!({ "key": paper_oa, "title": "Rebuilt OpenAlex Paper", "year": 2024 }),
        ],
        vec![
            json!({ "key": paper_ocr, "relevance": "core", "subclaim_ids": ["sc-1"], "why": "Scan." }),
            json!({ "key": paper_long, "relevance": "core", "subclaim_ids": ["sc-2"], "why": "Long." }),
            json!({ "key": paper_tamper, "relevance": "core", "subclaim_ids": ["sc-3"], "why": "Tamper." }),
            json!({ "key": paper_oa, "relevance": "related", "subclaim_ids": ["sc-4"], "why": "Rebuilt." }),
        ],
        None,
    );

    let q_ocr = "Theorem 3: The operator growth rate saturates at $b_n = \\alpha n$ for monitored dimer chains.";
    let (ing_ocr, xe_ocr) = ingest_and_record_xe(
        &data_dir, &ledger, sid, &paper_ocr, "scan.txt", SYNTH_PAPER_OCR_TXT, 3, q_ocr, &["sc-1"], "scan_text_layer",
    );
    let ocr_sha = ledger::sha256_hex(SYNTH_PAPER_OCR_TXT);
    append_ocr_pdf_pages_receipt(
        &ledger, sid, &ing_ocr.source_id, &ing_ocr.files[0], &ocr_sha, 3, 3, PageBasis::Ocr,
    );

    let sandbox_ing = research_tools::sandbox::WorkspaceSandbox::new(&data_dir, None);
    let mtx = std::sync::Mutex::new(Ledger::open(&data_dir).unwrap());
    let ing_long = research_tools::ingest::ingest_bytes(
        &sandbox_ing,
        &mtx,
        &research_tools::ingest::IngestRequest {
            session_id: sid,
            file_name: "long.tex",
            origin: &format!("library:{paper_long}"),
            raw_bytes: long_tex.as_bytes(),
            source_id_hint: Some("long.tex"),
            section_filter: None,
            title: Some("Unicode Scaling Paper"),
        },
    )
    .unwrap();
    drop(mtx);
    ledger
        .append_with_hash(
            sid,
            research_tools::search_coverage::IDEA_PAPER_EVENT,
            1,
            &json!({
                "key": paper_long, "source_id": ing_long.source_id, "status": "read_full",
                "text_basis": "tex", "lines_sent": 5, "total_lines": 5, "origin": "library"
            }),
        )
        .unwrap();
    let ing_long_seq = ledger
        .read_session(sid)
        .unwrap()
        .iter()
        .rev()
        .find(|e| e.kind == "source.ingested" && e.payload["source_id"] == ing_long.source_id)
        .unwrap()
        .seq;
    let xe_long = ledger
        .append_with_hash(
            sid,
            research_tools::quote_core::EVIDENCE_QUOTE_EVENT,
            1,
            &json!({
                "source_id": ing_long.source_id,
                "path": ing_long.files[0],
                "start_line": 3,
                "end_line": 3,
                "file_sha256": ledger::sha256_hex(long_tex.as_bytes()),
                "quote_sha256": quote_sha256(&long_utf8_quote),
                "quote": long_utf8_quote,
                "excerpt": long_utf8_quote,
                "source_seq": ing_long_seq,
                "caller": format!("reader:{sid}:{paper_long}"),
                "paper_key": paper_long,
                "role": "states_result",
                "subclaim_ids": ["sc-2"],
                "gist": "Long UTF-8 scaling claim."
            }),
        )
        .unwrap()
        .seq;

    let q_tamp = "In a finite monitored chain the Krylov slope scales linearly with lattice size across all even lengths.";
    let (ing_tamp, xe_tamp) = ingest_and_record_xe(
        &data_dir, &ledger, sid, &paper_tamper, "tamp.tex", SYNTH_PAPER_A_TEX, 4, q_tamp, &["sc-3"], "tex",
    );

    let oa_blob = synth_openalex_rebuilt_blob("W99887766", "Rebuilt OpenAlex Paper");
    let oa_hash = ledger.put_blob(&oa_blob).unwrap();
    let oa_quote = "Rebuilt abstract about dimer chains";
    ledger
        .append_with_hash(
            sid,
            EVIDENCE_ABSTRACT_EVENT,
            1,
            &json!({
                "paper_key": paper_oa,
                "quote": oa_quote,
                "quote_sha256": quote_sha256(oa_quote),
                "start": 0,
                "end": oa_quote.len(),
                "raw_blob": oa_hash,
                "provider": "openalex",
                "subclaim_ids": ["sc-4"]
            }),
        )
        .unwrap();

    append_map_and_honest_verdict(
        &ledger,
        sid,
        vec![
            json!({ "subclaim_id": "sc-1", "paper_key": paper_ocr, "relation": "same", "evidence": [format!("xe-{xe_ocr}")], "difference": "Dimer vs chain." }),
            json!({ "subclaim_id": "sc-2", "paper_key": paper_long, "relation": "partial", "evidence": [format!("xe-{xe_long}")], "difference": "Longer notation." }),
            json!({ "subclaim_id": "sc-3", "paper_key": paper_tamper, "relation": "same", "evidence": [format!("xe-{xe_tamp}")], "difference": "Source modified on disk." }),
        ],
    );

    let sandbox = research_tools::sandbox::WorkspaceSandbox::new(&data_dir, None);
    let tamp_path = sandbox.resolve_existing(SandboxRoot::Sources, sid, &ing_tamp.files[0], false).unwrap();
    fs::write(&tamp_path.canonical_target, b"corrupted source bytes").unwrap();

    let (_rep, verified_evs) = ledger.read_verified_session(sid).unwrap();
    let built = build_explore_writeup_detailed(&data_dir, sid, &verified_evs).unwrap();
    assert_eq!(built.rejected_quotes.len(), 2, "expected tampered xe + rebuilt OpenAlex xa dropped: {:?}", built.rejected_quotes);

    let clipped_q = built
        .writeup
        .sections
        .iter()
        .flat_map(|s| s.blocks.iter())
        .find_map(|b| match b {
            export::writeup::WriteupBlock::Quote(q) if q.paper.key == paper_long => Some(q),
            _ => None,
        })
        .expect("clipped long UTF-8 quote present");
    assert_eq!(clipped_q.text.chars().count(), 300);

    let opts = WriteupOptions::default();
    let (_, md_bytes) = writeup_session_bytes(&data_dir, sid, WriteupFormat::Md, &opts).unwrap();
    let (_, tex_bytes) = writeup_session_bytes(&data_dir, sid, WriteupFormat::Tex, &opts).unwrap();
    let md = String::from_utf8(md_bytes).unwrap();
    let tex = String::from_utf8(tex_bytes).unwrap();

    assert!(md.contains("p. 3") && md.contains("(OCR)"), "expected p. 3 + (OCR) in MD: {md}");
    assert!(!md.contains("$b_n"), "OCR math delimiters must be neutralized in MD: {md}");
    assert!(tex.contains("p. 3") && tex.contains("(OCR)"), "expected p. 3 + (OCR) in TeX: {tex}");
    assert!(tex.contains(r"\$b\_n"), "OCR math delimiters must stay escaped text in TeX: {tex}");
    assert!(!md.contains(q_tamp), "tampered xe quote must never be written");
    assert!(!tex.contains(q_tamp), "tampered xe quote must never be written in TeX");
    assert!(!md.contains(oa_quote), "non-verbatim OpenAlex abstract quote must never be written");
    assert!(md.contains(explore_writeup_wording::QUOTE_OMITTED_PHRASE));
}

#[test]
fn set_w_rule_s8_wording_property_across_all_stage_states_and_formats() {
    for &c in explore_writeup_wording::ALL_STATIC_CONSTANTS {
        assert_eq!(
            contains_forbidden_word(c, false),
            None,
            "static constant `{c}` leaked forbidden word"
        );
        assert_eq!(
            contains_forbidden_prior_work_word(c),
            None,
            "static constant `{c}` leaked prior-work forbidden word"
        );
    }

    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ledger = Ledger::open(&data_dir).unwrap();

    for (idx, &stage_state) in ExploreStageState::ALL.iter().enumerate() {
        let sid = format!("s-exp-stage-{idx}");
        append_explore_start_and_brief(
            &ledger,
            &sid,
            if idx % 2 == 0 { "standard" } else { "quick" },
            "Measure scaling exponent in finite lattice models.",
            "Scaling exponent in finite lattice models.",
            &[("sc-1", "Scaling exponent matches finite prediction.", "scaling exponent")],
            None,
        );
        ledger
            .append_with_hash(
                &sid,
                "idea.stage",
                1,
                &json!({ "stage": "search", "state": stage_state.as_str() }),
            )
            .unwrap();
        let (_rep, evs) = ledger.read_verified_session(&sid).unwrap();
        let doc = build_explore_writeup_from_verified(&data_dir, &sid, &evs).unwrap();
        let expected_run_state = if stage_state == ExploreStageState::Requested {
            explore_stage_run_state(ExploreStageState::Interrupted)
        } else {
            explore_stage_run_state(stage_state)
        };
        assert!(
            doc.status_line.contains(expected_run_state.label()),
            "stage {:?} expected `{}` in status_line: {}",
            stage_state,
            expected_run_state.label(),
            doc.status_line
        );
        for include_inline_badges in [true, false] {
            let opts = WriteupOptions { include_inline_badges, bib_stem: "refs".into() };
            for fmt in [WriteupFormat::Tex, WriteupFormat::Md, WriteupFormat::Bib] {
                let (_, bytes) = writeup_session_bytes(&data_dir, &sid, fmt, &opts).unwrap();
                let rendered = String::from_utf8(bytes).unwrap();
                assert!(
                    !rendered.to_ascii_lowercase().contains("verified"),
                    "Rule S8 violation: `verified` appeared in {fmt:?} (stage {stage_state:?}): {rendered}"
                );
                assert_eq!(
                    contains_forbidden_prior_work_word(&doc.status_line),
                    None,
                    "status_line leaked forbidden word: {}",
                    doc.status_line
                );
                assert_eq!(
                    contains_forbidden_prior_work_word(doc.coverage_paragraph.as_deref().unwrap()),
                    None,
                    "coverage_paragraph leaked forbidden word"
                );
            }
        }
    }

    let sid_clean = "s-exp-det";
    append_explore_start_and_brief(
        &ledger, sid_clean, "standard", "Determinism test.", "Determinism check.",
        &[("sc-1", "Deterministic summary.", "Determinism")], None,
    );
    let opts = WriteupOptions::default();
    let (_, a) = writeup_session_bytes(&data_dir, sid_clean, WriteupFormat::Tex, &opts).unwrap();
    let (_, b) = writeup_session_bytes(&data_dir, sid_clean, WriteupFormat::Tex, &opts).unwrap();
    assert_eq!(a, b, "Explore write-up must be byte-for-byte deterministic");
}
