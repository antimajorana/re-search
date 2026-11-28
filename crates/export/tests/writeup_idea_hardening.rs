//! Regression tests covering Carbon audit findings C1, C2, H1–H5, M1–M3, and L3
//! for the F15 Explore cited-summary write-up adapter.
//!
//! Zero third-party text, zero network calls, zero model calls, zero `#[ignore]` tests.

mod common_writeup_idea;

use common_writeup_idea::{
    append_arxiv_abstract_evidence, append_confirm_same, append_explore_start_and_brief,
    append_map_and_honest_verdict, append_search_and_triage, ingest_and_record_xe,
    synth_arxiv_atom_blob, SYNTH_PAPER_A_TEX, SYNTH_PAPER_OCR_TXT,
};
use export::writeup::{HowChecked, ProseBasis, WriteupBlock, WriteupOptions};
use export::writeup_idea::build_explore_writeup_detailed;
use export::{writeup_session_bytes, writeup_session_to_path, WriteupFormat};
use ledger::Ledger;
use research_tools::idea_brief::EVENT_IDEA_ANSWER;
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::paper_key::PaperKey;
use research_tools::pdf_pages::{PageBasis, PageQualityMetrics, PageRecord};
use research_tools::quote_core::{quote_sha256, EVIDENCE_ABSTRACT_EVENT, EVIDENCE_QUOTE_EVENT};
use research_tools::sandbox::WorkspaceSandbox;
use research_tools::search_coverage::IDEA_PAPER_EVENT;
use research_tools::{audit_tex_sources, AuditOptions, ResearchEnv};
use serde_json::json;
use std::fs;
use std::sync::Mutex;
use tempfile::TempDir;

#[test]
fn audit_c1_h1_h2_abstract_strict_blob_tail_tamper_and_source_paper_spoofing() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ledger = Ledger::open(&data_dir).unwrap();
    let sid = "s-exp-harden-c1-h2";

    let p_tail = PaperKey::from_arxiv("2408.01001").unwrap();
    let p_real = PaperKey::from_arxiv("2408.01002").unwrap();
    let p_spoof = PaperKey::from_arxiv("2408.01003").unwrap();
    let p_mal_oa = PaperKey::from_openalex("W55443322").unwrap();
    let p_ts = PaperKey::from_arxiv("2408.01004").unwrap();
    let p_bad_slice = PaperKey::from_arxiv("2408.01005").unwrap();

    append_explore_start_and_brief(
        &ledger, sid, "standard",
        "Check quote tail, source ownership, strict blob parsing, and abstract slice checks.",
        "Quote verification boundary suite.",
        &[
            ("sc-1", "Over-300 char tail tamper rejected.", "tail"),
            ("sc-2", "Cross-paper source spoofing rejected.", "ownership"),
            ("sc-3", "Malformed OpenAlex blob rejected without raw fallback.", "blob"),
            ("sc-4", "Disabled theoremsearch provider rejected.", "provider"),
            ("sc-5", "Out-of-bounds abstract slice rejected.", "slice"),
        ],
        None,
    );
    append_search_and_triage(
        &ledger, sid,
        vec![
            json!({ "qid": "q-1", "provider": "arxiv", "status": "ok", "count": 6 }),
            json!({ "qid": "q-2", "provider": "inspire", "status": "ok", "count": 6 }),
        ],
        vec![
            json!({ "key": p_tail, "title": "Tail Paper", "year": 2024 }),
            json!({ "key": p_real, "title": "Real Owner Paper", "year": 2024 }),
            json!({ "key": p_spoof, "title": "Spoofed Paper", "year": 2024 }),
            json!({ "key": p_mal_oa, "title": "Malformed OA Paper", "year": 2024 }),
            json!({ "key": p_ts, "title": "Disabled Provider Paper", "year": 2024 }),
            json!({ "key": p_bad_slice, "title": "Bad Slice Paper", "year": 2024 }),
        ],
        vec![
            json!({ "key": p_tail, "relevance": "core", "subclaim_ids": ["sc-1"], "why": "Core." }),
            json!({ "key": p_spoof, "relevance": "core", "subclaim_ids": ["sc-2"], "why": "Core." }),
            json!({ "key": p_mal_oa, "relevance": "related", "subclaim_ids": ["sc-3"], "why": "OA." }),
            json!({ "key": p_ts, "relevance": "related", "subclaim_ids": ["sc-4"], "why": "TS." }),
            json!({ "key": p_bad_slice, "relevance": "related", "subclaim_ids": ["sc-5"], "why": "Slice." }),
        ],
        None,
    );

    // H1: >300-char quote whose first 300 chars match disk, but tail after char 300 is fabricated.
    let prefix_305: String = (0..35).map(|i| format!("seg{i:02}abcd ")).collect();
    let real_line = format!("{prefix_305}GENUINE_TAIL_ON_DISK.");
    let fake_quote = format!("{prefix_305}FABRICATED_TAIL_AFTER_300.");
    assert!(prefix_305.chars().count() > 300);
    let tail_tex = format!("\\documentclass{{article}}\n\\begin{{document}}\n{real_line}\n\\end{{document}}\n");
    let sandbox = WorkspaceSandbox::new(&data_dir, None);
    let mtx = Mutex::new(Ledger::open(&data_dir).unwrap());
    let ing_tail = ingest_bytes(
        &sandbox, &mtx,
        &IngestRequest {
            session_id: sid, file_name: "tail.tex", origin: &format!("library:{p_tail}"),
            raw_bytes: tail_tex.as_bytes(), source_id_hint: Some("tail.tex"),
            section_filter: None, title: Some("Tail Paper"),
        },
    ).unwrap();
    drop(mtx);
    ledger.append_with_hash(sid, IDEA_PAPER_EVENT, 1, &json!({
        "key": p_tail, "source_id": ing_tail.source_id, "status": "read_full",
        "text_basis": "tex", "lines_sent": 5, "total_lines": 5, "origin": "library"
    })).unwrap();
    let ing_tail_seq = ledger.read_session(sid).unwrap().iter().rev()
        .find(|e| e.kind == "source.ingested" && e.payload["source_id"] == ing_tail.source_id)
        .unwrap().seq;
    let xe_tail = ledger.append_with_hash(sid, EVIDENCE_QUOTE_EVENT, 1, &json!({
        "source_id": ing_tail.source_id, "path": ing_tail.files[0],
        "start_line": 3, "end_line": 3,
        "file_sha256": ledger::sha256_hex(tail_tex.as_bytes()),
        "quote_sha256": quote_sha256(&fake_quote),
        "quote": fake_quote, "excerpt": fake_quote, "source_seq": ing_tail_seq,
        "caller": format!("reader:{sid}:{p_tail}"), "paper_key": p_tail,
        "role": "states_result", "subclaim_ids": ["sc-1"], "gist": "Tampered tail."
    })).unwrap().seq;

    // H1: Cross-paper source_id spoofing (p_spoof claims quote from p_real's ingested source).
    let q_real = "In a finite monitored chain the Krylov slope scales linearly with lattice size across all even lengths.";
    let (ing_real, _) = ingest_and_record_xe(
        &data_dir, &ledger, sid, &p_real, "real.tex", SYNTH_PAPER_A_TEX, 4, q_real, &[], "tex",
    );
    let real_src_seq = ledger.read_session(sid).unwrap().iter().rev()
        .find(|e| e.kind == "source.ingested" && e.payload["source_id"] == ing_real.source_id)
        .unwrap().seq;
    ledger.append_with_hash(sid, IDEA_PAPER_EVENT, 1, &json!({
        "key": p_spoof, "source_id": "spoof-own-src", "status": "read_full",
        "text_basis": "tex", "lines_sent": 5, "total_lines": 5, "origin": "library"
    })).unwrap();
    let xe_spoof = ledger.append_with_hash(sid, EVIDENCE_QUOTE_EVENT, 1, &json!({
        "source_id": ing_real.source_id, "path": ing_real.files[0], "start_line": 4, "end_line": 4,
        "file_sha256": ledger::sha256_hex(SYNTH_PAPER_A_TEX), "quote_sha256": quote_sha256(q_real),
        "quote": q_real, "excerpt": q_real, "source_seq": real_src_seq,
        "caller": format!("reader:{sid}:{p_spoof}"), "paper_key": p_spoof,
        "role": "states_result", "subclaim_ids": ["sc-2"], "gist": "Spoofed source."
    })).unwrap().seq;

    // C1: Malformed OpenAlex raw_blob with literal text — must NOT pass via raw fallback.
    let mal_q = "literal quote text inside broken openalex blob";
    let mal_hash = ledger.put_blob(format!("{{ invalid json: {mal_q} }}").as_bytes()).unwrap();
    ledger.append_with_hash(sid, EVIDENCE_ABSTRACT_EVENT, 1, &json!({
        "paper_key": p_mal_oa, "quote": mal_q, "quote_sha256": quote_sha256(mal_q),
        "start": 0, "end": mal_q.len(), "raw_blob": mal_hash,
        "provider": "openalex", "subclaim_ids": ["sc-3"]
    })).unwrap();

    // C1: Disabled provider (`theoremsearch`) with literal text — must be rejected.
    let ts_q = "literal quote text from disabled theoremsearch provider";
    let ts_hash = ledger.put_blob(ts_q.as_bytes()).unwrap();
    ledger.append_with_hash(sid, EVIDENCE_ABSTRACT_EVENT, 1, &json!({
        "paper_key": p_ts, "quote": ts_q, "quote_sha256": quote_sha256(ts_q),
        "start": 0, "end": ts_q.len(), "raw_blob": ts_hash,
        "provider": "theoremsearch", "subclaim_ids": ["sc-4"]
    })).unwrap();

    // H2: Valid arXiv Atom blob, but out-of-bounds `start..end` slice offsets.
    let arxiv_blob = synth_arxiv_atom_blob("2408.01005", "Bad Slice Paper", "Short genuine summary.");
    let arxiv_hash = ledger.put_blob(&arxiv_blob).unwrap();
    let bs_q = "Short genuine summary";
    ledger.append_with_hash(sid, EVIDENCE_ABSTRACT_EVENT, 1, &json!({
        "paper_key": p_bad_slice, "quote": bs_q, "quote_sha256": quote_sha256(bs_q),
        "start": 10, "end": 9999, "raw_blob": arxiv_hash,
        "provider": "arxiv", "subclaim_ids": ["sc-5"]
    })).unwrap();

    append_map_and_honest_verdict(&ledger, sid, vec![
        json!({ "subclaim_id": "sc-1", "paper_key": p_tail, "relation": "same", "evidence": [format!("xe-{xe_tail}")], "difference": "Tail." }),
        json!({ "subclaim_id": "sc-2", "paper_key": p_spoof, "relation": "same", "evidence": [format!("xe-{xe_spoof}")], "difference": "Spoof." }),
    ]);

    let (_rep, evs) = ledger.read_verified_session(sid).unwrap();
    let built = build_explore_writeup_detailed(&data_dir, sid, &evs).unwrap();
    let codes: Vec<&str> = built.rejected_quotes.iter().map(|r| r.fault.code.as_str()).collect();
    assert!(codes.contains(&"quote_tail_unverified"), "expected tail rejection: {codes:?}");
    assert!(codes.contains(&"source_paper_mismatch"), "expected source ownership rejection: {codes:?}");
    assert_eq!(codes.iter().filter(|&&c| c == "abstract_not_in_blob").count(), 2, "expected malformed OA + theoremsearch rejections: {codes:?}");
    assert!(codes.contains(&"abstract_offset_out_of_bounds"), "expected bad slice rejection: {codes:?}");
    assert!(built.writeup.bibliography.is_empty(), "all 5 rejected papers must stay out of .bib");
    for sc in ["sc-1", "sc-2", "sc-3", "sc-4", "sc-5"] {
        let row = built.writeup.check_table.iter().find(|r| r.id == sc).unwrap();
        assert_eq!(row.how_checked, HowChecked::CodeSkeleton, "{sc} must downgrade to None in scope");
    }
}

#[test]
fn audit_h3_abstract_quote_renders_tex_cite_and_passes_tex_audit() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ws_dir = tmp.path().join("ws");
    fs::create_dir_all(&ws_dir).unwrap();
    let ledger = Ledger::open(&data_dir).unwrap();
    let sid = "s-exp-h3-abs-tex";
    let p_full = PaperKey::from_arxiv("2408.02001").unwrap();
    let p_abs = PaperKey::from_arxiv("2408.02002").unwrap();

    append_explore_start_and_brief(
        &ledger, sid, "standard",
        "Check abstract quote TeX citation rendering and tex_audit.",
        "Abstract citation TeX integration.",
        &[
            ("sc-1", "Full-text Krylov scaling holds.", "Krylov scaling"),
            ("sc-2", "Abstract-only boundary damping holds.", "boundary damping"),
        ],
        None,
    );
    append_search_and_triage(
        &ledger, sid,
        vec![
            json!({ "qid": "q-1", "provider": "arxiv", "status": "ok", "count": 2 }),
            json!({ "qid": "q-2", "provider": "inspire", "status": "ok", "count": 2 }),
        ],
        vec![
            json!({ "key": p_full, "title": "Full Chain Paper", "authors": ["Lin, Bo"], "year": 2024 }),
            json!({ "key": p_abs, "title": "Abstract Damping Paper", "authors": ["Vega, Sol"], "year": 2024 }),
        ],
        vec![
            json!({ "key": p_full, "relevance": "core", "subclaim_ids": ["sc-1"], "why": "Core." }),
            json!({ "key": p_abs, "relevance": "related", "subclaim_ids": ["sc-2"], "why": "Abstract." }),
        ],
        None,
    );

    let q_full = "In a finite monitored chain the Krylov slope scales linearly with lattice size across all even lengths.";
    let (_, xe_seq) = ingest_and_record_xe(
        &data_dir, &ledger, sid, &p_full, "full.tex", SYNTH_PAPER_A_TEX, 4, q_full, &["sc-1"], "tex",
    );
    append_arxiv_abstract_evidence(
        &ledger, sid, "2408.02002", "Abstract Damping Paper",
        "We observe boundary damping of spread complexity in monitored chains under periodic conditions.",
        "boundary damping of spread complexity in monitored chains",
        &["sc-2"],
    );
    append_map_and_honest_verdict(&ledger, sid, vec![json!({
        "subclaim_id": "sc-1", "paper_key": p_full, "relation": "same",
        "evidence": [format!("xe-{xe_seq}")], "difference": "Periodic vs open."
    })]);

    let tex_path = ws_dir.join("summary.tex");
    let opts = WriteupOptions::default();
    writeup_session_to_path(&data_dir, sid, WriteupFormat::Tex, &opts, &tex_path).unwrap();
    let tex = fs::read_to_string(&tex_path).unwrap();
    let bib = fs::read_to_string(ws_dir.join("summary.bib")).unwrap();
    let (_, md_bytes) = writeup_session_bytes(&data_dir, sid, WriteupFormat::Md, &opts).unwrap();
    let md = String::from_utf8(md_bytes).unwrap();
    let (_rep, evs) = ledger.read_verified_session(sid).unwrap();
    let doc = build_explore_writeup_detailed(&data_dir, sid, &evs).unwrap().writeup;

    assert!(!tex.contains("[@"), "TeX must use \\cite{{}}, never Markdown [@...]: {tex}");
    let full_key = doc.bibliography.iter().find(|b| b.paper.key == p_full).unwrap().cite_key.clone();
    let abs_key = doc.bibliography.iter().find(|b| b.paper.key == p_abs).unwrap().cite_key.clone();
    assert!(tex.contains(&format!("\\cite{{{full_key}}}")) && bib.contains(&full_key));
    assert!(tex.contains(&format!("\\cite{{{abs_key}}}")) && bib.contains(&abs_key));
    assert!(md.contains(&format!("[@{abs_key}]")), "Markdown must cite abstract via [@key]: {md}");
    assert_eq!(tex.matches("\\begin{quote}").count(), 1, "only full-text xe quote uses \\begin{{quote}}");

    let env = ResearchEnv::with_workspace(&data_dir, &ws_dir).unwrap();
    let audit = audit_tex_sources(&env, sid, Some(&ledger), &AuditOptions::default()).unwrap();
    assert!(audit.ok, "mixed xe+xa Explore TeX must pass tex_audit cleanly: {:?}", audit.issues);
    assert_eq!(audit.blocker_count, 0);
    assert_eq!(audit.error_count, 0);
}

#[test]
fn audit_h4_h5_user_answer_provenance_stage_unverified_and_s8_v8_boundaries() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ledger = Ledger::open(&data_dir).unwrap();

    // H4/H5: user-edited `idea.answer` with "novel"/"unverified" + stage reason with "unverified"
    // + session id containing "verified" must all succeed cleanly without violating Rule S8.
    let sid = "s-verified-1";
    append_explore_start_and_brief(
        &ledger, sid, "standard",
        "Check unverified boundary assumptions on finite chains.",
        "Clean model restatement.",
        &[("sc-1", "Boundary scaling holds for unverified conjectures.", "boundary")],
        None,
    );
    ledger.append_with_hash(sid, EVENT_IDEA_ANSWER, 1, &json!({
        "restatement": "User-edited novel framing of unverified lattice scaling",
        "assumptions": ["User assumes novel unverified boundary coupling"]
    })).unwrap();
    ledger.append_with_hash(sid, "idea.stage", 1, &json!({
        "stage": "search", "state": "stopped", "reason": "stopped after unverified draft"
    })).unwrap();

    let (_rep, evs) = ledger.read_verified_session(sid).unwrap();
    let built = build_explore_writeup_detailed(&data_dir, sid, &evs).unwrap();
    assert!(!built.writeup.status_line.to_ascii_lowercase().contains("verified"));
    let user_blocks = built.writeup.sections[0].blocks.iter()
        .filter(|b| matches!(b, WriteupBlock::Prose { basis: ProseBasis::User, .. }))
        .count();
    assert_eq!(user_blocks, 3, "idea text + user restatement + user assumptions must be User");

    // Session id containing "verified" with empty restatement falls back without leaking.
    let sid_t = "s-verified-2";
    append_explore_start_and_brief(
        &ledger, sid_t, "standard", "User calls this verified locally.", "",
        &[("sc-1", "Clean claim.", "claim")], None,
    );
    let (_rep2, evs2) = ledger.read_verified_session(sid_t).unwrap();
    let built2 = build_explore_writeup_detailed(&data_dir, sid_t, &evs2).unwrap();
    assert!(!built2.writeup.title.to_ascii_lowercase().contains("verified"));

    // Model-authored subclaim containing word "verified" MUST fail closed (Rule S8).
    let sid_s8 = "s-exp-s8-subclaim";
    append_explore_start_and_brief(
        &ledger, sid_s8, "standard", "Clean idea.", "Clean restatement.",
        &[("sc-1", "This formula is verified analytically.", "formula")], None,
    );
    let err_sc = writeup_session_bytes(&data_dir, sid_s8, WriteupFormat::Md, &WriteupOptions::default())
        .unwrap_err();
    assert!(err_sc.contains("Rule S8"), "expected S8 refusal on subclaim: {err_sc}");

    // Direction `first_calculation.method` containing V8 banned phrase MUST fail closed.
    let sid_dir = "s-exp-h4-dir";
    append_explore_start_and_brief(
        &ledger, sid_dir, "standard", "Clean idea.", "Clean restatement.",
        &[("sc-1", "Clean claim.", "claim")], None,
    );
    ledger.append_with_hash(sid_dir, "idea.directions", 1, &json!({
        "directions": [{
            "id": "r-1", "title": "Method test", "question": "Check numerical behavior.",
            "concreteness": "concrete",
            "first_calculation": { "setup": "Run sweep.", "method": "unexplored method" }
        }]
    })).unwrap();
    let err_dir = writeup_session_bytes(&data_dir, sid_dir, WriteupFormat::Md, &WriteupOptions::default())
        .unwrap_err();
    assert!(err_dir.contains("Rule V8"), "expected V8 refusal on direction method: {err_dir}");
}

#[test]
fn audit_m1_m2_m3_multipage_ocr_alias_citekey_and_directions_staleness() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ledger = Ledger::open(&data_dir).unwrap();
    let sid = "s-exp-m1-m3";

    let p_clean = PaperKey::from_arxiv("2408.11111").unwrap();
    let p_clean_doi = PaperKey::from_doi("10.5555/alias-paper").unwrap();
    let p_multi = PaperKey::from_arxiv("2408.22222").unwrap();

    append_explore_start_and_brief(
        &ledger, sid, "standard",
        "Check multi-page OCR preference, secondary OCR scoping, alias citekey, and direction order.",
        "Multi-page OCR and alias resolution.",
        &[
            ("sc-1", "Primary clean TeX overlap plus secondary OCR overlap.", "primary clean"),
            ("sc-2", "Multi-page quote spanning clean page 2 and OCR page 3.", "multi-page"),
        ],
        None,
    );
    append_search_and_triage(
        &ledger, sid,
        vec![
            json!({ "qid": "q-1", "provider": "arxiv", "status": "ok", "count": 2 }),
            json!({ "qid": "q-2", "provider": "inspire", "status": "ok", "count": 2 }),
        ],
        vec![
            json!({
                "key": p_clean, "title": "Aliased Clean Paper", "authors": ["Sato, Ken"], "year": 2024,
                "ids": { "arxiv": "2408.11111", "doi": "10.5555/alias-paper" }
            }),
            json!({ "key": p_multi, "title": "Mixed Scan Paper", "authors": ["Bauer, Mia"], "year": 1991 }),
        ],
        vec![
            json!({ "key": p_clean, "relevance": "core", "subclaim_ids": ["sc-1"], "why": "Core." }),
            json!({ "key": p_multi, "relevance": "core", "subclaim_ids": ["sc-1", "sc-2"], "why": "Scan." }),
        ],
        None,
    );

    let q_clean = "In a finite monitored chain the Krylov slope scales linearly with lattice size across all even lengths.";
    let (_, xe_clean) = ingest_and_record_xe(
        &data_dir, &ledger, sid, &p_clean_doi, "clean.tex", SYNTH_PAPER_A_TEX, 4, q_clean, &["sc-1"], "tex",
    );

    // M1: Multi-page quote spanning line 2 (page 2 TextLayer) and line 3 (page 3 Ocr).
    let multi_q = "Page two setup notes for synthetic scanned paper.\nTheorem 3: The operator growth rate saturates at $b_n = \\alpha n$ for monitored dimer chains.";
    let sandbox = WorkspaceSandbox::new(&data_dir, None);
    let mtx = Mutex::new(Ledger::open(&data_dir).unwrap());
    let ing_m = ingest_bytes(
        &sandbox, &mtx,
        &IngestRequest {
            session_id: sid, file_name: "mixed.txt", origin: &format!("library:{p_multi}"),
            raw_bytes: SYNTH_PAPER_OCR_TXT, source_id_hint: Some("mixed.txt"),
            section_filter: None, title: Some("Mixed Scan Paper"),
        },
    ).unwrap();
    drop(mtx);
    ledger.append_with_hash(sid, IDEA_PAPER_EVENT, 1, &json!({
        "key": p_multi, "source_id": ing_m.source_id, "status": "read_full",
        "text_basis": "pdf_text", "lines_sent": 3, "total_lines": 3, "origin": "library"
    })).unwrap();
    let multi_sha = ledger::sha256_hex(SYNTH_PAPER_OCR_TXT);
    let mk_pg = |page, line, basis| PageRecord {
        page, basis, first_line: Some(line), last_line: Some(line),
        metrics: PageQualityMetrics {
            char_count: 80, alpha_count: 60, word_count: 14, image_coverage_pct: 50,
            has_invisible_text: false, width_pt: 612, height_pt: 792,
        },
        reason: None,
    };
    ledger.append_with_hash(sid, "source.ingested", 1, &json!({
        "source_id": ing_m.source_id, "format": "pdf", "files": [&ing_m.files[0]],
        "files_sha256": { &ing_m.files[0]: &multi_sha },
        "pdf_pages": [mk_pg(2, 2, PageBasis::TextLayer), mk_pg(3, 3, PageBasis::Ocr)]
    })).unwrap();
    let ing_m_seq = ledger.read_session(sid).unwrap().iter().rev()
        .find(|e| e.kind == "source.ingested" && e.payload["source_id"] == ing_m.source_id)
        .unwrap().seq;
    let xe_multi = ledger.append_with_hash(sid, EVIDENCE_QUOTE_EVENT, 1, &json!({
        "source_id": ing_m.source_id, "path": ing_m.files[0], "start_line": 2, "end_line": 3,
        "file_sha256": multi_sha, "quote_sha256": quote_sha256(multi_q),
        "quote": multi_q, "excerpt": multi_q, "source_seq": ing_m_seq,
        "caller": format!("reader:{sid}:{p_multi}"), "paper_key": p_multi,
        "role": "states_result", "subclaim_ids": ["sc-1", "sc-2"], "gist": "Mixed pages."
    })).unwrap().seq;

    // Stale pre-map `idea.directions` MUST be ignored once `idea.map` is appended.
    ledger.append_with_hash(sid, "idea.directions", 1, &json!({
        "directions": [{ "id": "stale-1", "title": "Stale direction", "concreteness": "concrete" }]
    })).unwrap();

    let (map_seq, _) = append_map_and_honest_verdict(&ledger, sid, vec![
        json!({ "subclaim_id": "sc-1", "paper_key": p_clean_doi, "relation": "same", "evidence": [format!("xe-{xe_clean}")], "difference": "Alias diff note." }),
        json!({ "subclaim_id": "sc-1", "paper_key": p_multi, "relation": "partial", "evidence": [format!("xe-{xe_multi}")], "difference": "Secondary scan note." }),
        json!({ "subclaim_id": "sc-2", "paper_key": p_multi, "relation": "same", "evidence": [format!("xe-{xe_multi}")], "difference": "Multi-page diff." }),
    ]);
    append_confirm_same(&ledger, sid, "sc-1");

    // Current `idea.directions` with Sketch before Concrete, duplicate IDs, non-object, and empty entry.
    ledger.append_with_hash(sid, "idea.directions", 1, &json!({
        "map_seq": map_seq,
        "directions": [
            "not-an-object",
            {},
            { "id": "r-1", "title": "Loose sketch", "concreteness": "sketch" },
            { "id": "r-1", "title": "Concrete sweep", "concreteness": "concrete" }
        ]
    })).unwrap();

    let (_rep, evs) = ledger.read_verified_session(sid).unwrap();
    let built = build_explore_writeup_detailed(&data_dir, sid, &evs).unwrap();

    // M1: sc-1 primary overlap is clean TeX (confirmed) -> MatchedQuoteFullText (NOT downgraded to OCR!).
    let sc1_row = built.writeup.check_table.iter().find(|r| r.id == "sc-1").unwrap();
    assert_eq!(sc1_row.how_checked, HowChecked::MatchedQuoteFullText);
    assert!(!sc1_row.note.as_deref().unwrap_or("").contains("(OCR)"));

    // M1: sc-2 multi-page quote (clean p. 2 + OCR p. 3) MUST report p. 3 (OCR), NOT p. 2.
    let sc2_note = built.writeup.check_table.iter().find(|r| r.id == "sc-2").unwrap()
        .note.as_deref().unwrap_or("");
    assert!(sc2_note.contains("p. 3") && sc2_note.contains("(OCR)"), "expected p. 3 (OCR): {sc2_note}");
    assert!(!sc2_note.contains("p. 2"), "must prefer scanned page 3 over clean page 2: {sc2_note}");

    // M2: "What differs" for DOI-aliased overlap must use resolved BibTeX cite_key, not raw `doi:...`.
    let clean_bib = built.writeup.bibliography.iter().find(|b| b.paper.key == p_clean).expect("canonical arXiv bib");
    let (_, md_bytes) = writeup_session_bytes(&data_dir, sid, WriteupFormat::Md, &WriteupOptions::default()).unwrap();
    let md = String::from_utf8(md_bytes).unwrap();
    assert!(md.contains(&format!("What differs ({}): Alias diff note.", clean_bib.cite_key)));
    assert!(!md.contains("What differs (doi:10.5555/alias-paper)"));

    // M3: Stale direction ignored; check_table lists Concrete BEFORE Sketch with deduplicated IDs.
    assert!(!md.contains("Stale direction"));
    let pos_conc = built.writeup.check_table.iter().position(|r| r.statement == "Concrete sweep").unwrap();
    let pos_sketch = built.writeup.check_table.iter().position(|r| r.statement == "Loose sketch").unwrap();
    assert!(pos_conc < pos_sketch, "check_table must place Concrete before Sketch");
    assert_ne!(built.writeup.check_table[pos_conc].id, built.writeup.check_table[pos_sketch].id);
}
