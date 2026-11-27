//! Set W core integration tests for F15 Solve write-up adapter (`writeup_solve.rs`):
//! `tex_audit`, determinism, assumptions-first + topological ordering, Gate G7 refusal,
//! Rule R5 (`solve.expr.latex` exclusive), F4 quote re-verification + OCR + physical PDF page,
//! and library `.bib` key resolution (`source_item.citekey`).

mod common_writeup_solve;

use common_writeup_solve::{
    append_check, append_done, append_expr, append_setup, append_step, setup_library_with_citekey,
    SYNTH_SOURCE_TEX,
};
use export::writeup::{writeup_wording, ProseBasis, QuoteLocator, WriteupBlock, WriteupOptions};
use export::writeup_bib::render_bibtex;
use export::writeup_md::render_writeup_markdown;
use export::writeup_solve::build_solve_writeup;
use export::writeup_tex::render_writeup_tex;
use ledger::Ledger;
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::paper_key::{PaperIds, PaperRef};
use research_tools::pdf_pages::{PageBasis, PageQualityMetrics, PageRecord};
use research_tools::quote_core::{quote_sha256, verify_quote, QuoteRef};
use research_tools::sandbox::{SandboxRoot, WorkspaceSandbox};
use research_tools::solve_model::{
    compute_claim_sha256, event_kinds, Claim, ClaimForm, ClaimRelation, HandleRef,
    SolveDoneStatus, StepEntryType, StepOrigin,
};
use research_tools::solve_status::Tier;
use research_tools::{audit_tex_sources, AuditOptions, ResearchEnv};
use serde_json::json;
use std::fs;
use std::sync::Mutex;
use tempfile::TempDir;

#[test]
fn verified_solve_writeup_passes_tex_audit_r5_citekey_and_determinism() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ws_dir = tmp.path().join("ws");
    fs::create_dir_all(&ws_dir).unwrap();

    let paper = PaperRef::from_ids(
        PaperIds {
            arxiv: Some("2402.01234".into()),
            doi: Some("10.9999/synth.boundary.2024".into()),
            ..Default::default()
        },
        Some("v2".into()),
    )
    .unwrap();
    setup_library_with_citekey(&data_dir, &paper, "voss2024lattice");

    let sid = "s-solve-full";
    let ledger = Ledger::open(&data_dir).unwrap();
    let math_sha = append_setup(
        &ledger,
        sid,
        "Expand (x + 1)^2 for real positive x.",
        "Synthetic Square Identity",
    );

    let sandbox = WorkspaceSandbox::new(&data_dir, Some(ws_dir.clone()));
    let ledger_mtx = Mutex::new(Ledger::open(&data_dir).unwrap());
    let ingested = ingest_bytes(
        &sandbox,
        &ledger_mtx,
        &IngestRequest {
            session_id: sid,
            file_name: "voss2024lattice.tex",
            origin: &format!("library:{}", paper.key),
            raw_bytes: SYNTH_SOURCE_TEX,
            source_id_hint: Some("voss2024lattice.tex"),
            section_filter: None,
            title: Some("Synthetic Boundary Sums on Finite Chains"),
        },
    )
    .unwrap();
    let events_after_ingest = ledger.read_session(sid).unwrap();
    let quote_text = "The discrete boundary sum equals 7/12 for every even chain length.";
    let verified_q = verify_quote(
        &sandbox, sid, &events_after_ingest, &ingested.source_id, &ingested.files[0], 4, 4, quote_text,
    )
    .unwrap();
    let eq_seq = ledger
        .append_with_hash(
            sid,
            event_kinds::EVIDENCE_QUOTE,
            1,
            &json!({
                "source_id": verified_q.quote_ref.source_id, "path": verified_q.quote_ref.path,
                "start_line": verified_q.quote_ref.start_line, "end_line": verified_q.quote_ref.end_line,
                "file_sha256": verified_q.quote_ref.file_sha256, "quote_sha256": verified_q.quote_ref.quote_sha256,
                "quote": quote_text, "excerpt": verified_q.excerpt, "source_seq": verified_q.source_seq,
                "caller": "user", "paper_key": paper.key,
            }),
        )
        .unwrap()
        .seq;
    let qref = QuoteRef { evidence_seq: Some(eq_seq), ..verified_q.quote_ref };

    // Out-of-closure step must NOT appear.
    append_step(
        &ledger, sid, "e-99", StepEntryType::Derivation, StepOrigin::Solver,
        "Dead-end branch outside closure.", "Abandoned trial path.", &[], &[], None, None, None,
    );
    append_step(
        &ledger, sid, "e-1", StepEntryType::Assumption, StepOrigin::Setup,
        "Assume x > 0 is real.", "User setup assumption on domain.", &[], &[], None, None, None,
    );
    append_step(
        &ledger, sid, "e-2", StepEntryType::ReferenceLookup, StepOrigin::Solver,
        "Boundary sum lemma from cited note.", "Cited for chain normalization.",
        &[], &[], None, None, Some(&qref),
    );

    let s1 = append_expr(&ledger, sid, "lhs_sq", 1, "(x + 1)^2", "Pow(Add(x,1),2)", Some("e-3"), true);
    let s2 = append_expr(&ledger, sid, "rhs_sq", 1, "x^2 + 2 x + 1", "Add(Pow(x,2),Mul(2,x),1)", Some("e-3"), true);
    let claim = Claim {
        form: ClaimForm::Equation, lhs: HandleRef::new("lhs_sq", 1).unwrap(),
        rhs: HandleRef::new("rhs_sq", 1).unwrap(), relation: ClaimRelation::Eq,
        order: None, domain: None, how: None,
    };
    let csha = compute_claim_sha256(std::slice::from_ref(&claim), |h| {
        if h.name == "lhs_sq" { Some(s1.as_str()) } else { Some(s2.as_str()) }
    })
    .unwrap();
    append_step(
        &ledger, sid, "e-3", StepEntryType::FinalAnswer, StepOrigin::Solver,
        "Exact expansion of the square.", "Model algebraic expansion step.",
        &["e-1", "e-2"], &[claim], Some(&csha), Some(r"\text{POISONED\_SOLVER\_LATEX}"), None,
    );
    append_check(&ledger, sid, "chk-1", &csha, &math_sha, "identity_symbolic", "pass", json!({ "residual": "0" }));
    append_done(&ledger, sid, "e-3", Tier::Verified, Tier::Verified, SolveDoneStatus::Finished);

    let doc1 = build_solve_writeup(&data_dir, sid).unwrap();
    let doc2 = build_solve_writeup(&data_dir, sid).unwrap();
    assert_eq!(doc1, doc2, "write-up builder must be deterministic");

    let table_ids: Vec<&str> = doc1.check_table.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(table_ids, vec!["e-1", "e-2", "e-3"]);
    assert_eq!(doc1.bibliography.len(), 1);
    assert_eq!(doc1.bibliography[0].cite_key, "voss2024lattice");

    let mut saw_user_prose = false;
    let mut saw_model_prose = false;
    for sec in &doc1.sections {
        for blk in &sec.blocks {
            if let WriteupBlock::Prose { basis, text } = blk {
                if text.contains("User setup assumption") {
                    assert_eq!(*basis, ProseBasis::User);
                    saw_user_prose = true;
                }
                if text.contains("Model algebraic expansion") {
                    assert_eq!(*basis, ProseBasis::Model);
                    saw_model_prose = true;
                }
            }
        }
    }
    assert!(saw_user_prose && saw_model_prose);

    let opts = WriteupOptions { include_inline_badges: true, bib_stem: "solve_out".into() };
    let tex = render_writeup_tex(&doc1, &opts).unwrap();
    let md = render_writeup_markdown(&doc1, &opts).unwrap();
    let bib = render_bibtex(&doc1.bibliography);

    assert!(tex.contains("(x + 1)^2 = x^2 + 2 x + 1"));
    assert!(md.contains("(x + 1)^2 = x^2 + 2 x + 1"));
    assert!(!tex.contains("POISONED"), "R5 violation: SolveEntry.latex leaked");
    assert!(!md.contains("POISONED"), "R5 violation: SolveEntry.latex leaked");
    assert!(tex.contains(r"\cite{voss2024lattice}"));
    assert!(bib.contains("@article{voss2024lattice,"));
    assert!(tex.contains(writeup_wording::CHECK_TABLE_HEADING));
    assert!(tex.contains("Done (final answer: Verified)"));

    // Turning off inline marks preserves status line + check table.
    let no_inline = WriteupOptions { include_inline_badges: false, bib_stem: "solve_out".into() };
    let tex_ni = render_writeup_tex(&doc1, &no_inline).unwrap();
    let md_ni = render_writeup_markdown(&doc1, &no_inline).unwrap();
    assert!(!tex_ni.contains("[Verified]") && !md_ni.contains("[Verified]"));
    assert!(tex_ni.contains("Done (final answer: Verified)"));
    assert!(tex_ni.contains(writeup_wording::CHECK_TABLE_HEADING));
    assert!(md_ni.contains(writeup_wording::CHECK_TABLE_HEADING));

    // Set W: `tex_audit` passes with 0 blockers and 0 errors.
    fs::write(ws_dir.join("solve_out.tex"), &tex).unwrap();
    fs::write(ws_dir.join("solve_out.bib"), &bib).unwrap();
    let env = ResearchEnv::with_workspace(&data_dir, &ws_dir).unwrap();
    let audit = audit_tex_sources(&env, sid, Some(&ledger), &AuditOptions::default()).unwrap();
    assert!(audit.ok, "tex_audit failed: {:?}", audit.issues);
    assert_eq!(audit.blocker_count, 0);
    assert_eq!(audit.error_count, 0);
}

#[test]
fn gate_g7_refuses_failed_stale_and_superseded_closure_steps() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ledger = Ledger::open(&data_dir).unwrap();

    // 1. Failed step in closure.
    let s_fail = "s-g7-failed";
    let m_fail = append_setup(&ledger, s_fail, "Failed check case.", "G7 Failed");
    let h1 = append_expr(&ledger, s_fail, "a", 1, "a", "Symbol('a')", Some("e-1"), true);
    let h2 = append_expr(&ledger, s_fail, "b", 1, "b", "Symbol('b')", Some("e-1"), true);
    let claim = Claim {
        form: ClaimForm::Equation, lhs: HandleRef::new("a", 1).unwrap(),
        rhs: HandleRef::new("b", 1).unwrap(), relation: ClaimRelation::Eq,
        order: None, domain: None, how: None,
    };
    let csha = compute_claim_sha256(std::slice::from_ref(&claim), |h| {
        if h.name == "a" { Some(h1.as_str()) } else { Some(h2.as_str()) }
    })
    .unwrap();
    append_step(&ledger, s_fail, "e-1", StepEntryType::FinalAnswer, StepOrigin::Solver, "Bad identity.", "", &[], &[claim], Some(&csha), None, None);
    append_check(&ledger, s_fail, "chk-f", &csha, &m_fail, "identity_symbolic", "fail", json!({ "counterexample": "a=1,b=2" }));
    let err = build_solve_writeup(&data_dir, s_fail).unwrap_err();
    assert!(err.contains("Gate G7") && err.contains("Failed"), "unexpected err: {err}");

    // 2. Stale step (depends on superseded ancestor).
    let s_stale = "s-g7-stale";
    append_setup(&ledger, s_stale, "Stale case.", "G7 Stale");
    append_step(&ledger, s_stale, "e-1", StepEntryType::Assumption, StepOrigin::Setup, "Old assumption.", "", &[], &[], None, None, None);
    append_step(&ledger, s_stale, "e-2", StepEntryType::Assumption, StepOrigin::Setup, "Replacement assumption.", "", &[], &[], None, None, None);
    append_step(&ledger, s_stale, "e-3", StepEntryType::FinalAnswer, StepOrigin::Solver, "Depends on old e-1.", "", &["e-1"], &[], None, None, None);
    ledger.append_with_hash(s_stale, event_kinds::PROOF_SUPERSEDE, 1, &json!({ "entry_id": "e-1", "replaced_by": "e-2" })).unwrap();
    let err_stale = build_solve_writeup(&data_dir, s_stale).unwrap_err();
    assert!(err_stale.contains("Gate G7") && (err_stale.contains("Stale") || err_stale.contains("superseded")), "unexpected err: {err_stale}");

    // 3. Direct superseded target step via solve.done.
    let s_sup = "s-g7-sup";
    append_setup(&ledger, s_sup, "Superseded target.", "G7 Superseded");
    append_step(&ledger, s_sup, "e-1", StepEntryType::FinalAnswer, StepOrigin::Solver, "First target.", "", &[], &[], None, None, None);
    append_done(&ledger, s_sup, "e-1", Tier::Unchecked, Tier::Unchecked, SolveDoneStatus::FinishedOpenItems);
    append_step(&ledger, s_sup, "e-2", StepEntryType::FinalAnswer, StepOrigin::Solver, "Replacement.", "", &[], &[], None, None, None);
    ledger.append_with_hash(s_sup, event_kinds::PROOF_SUPERSEDE, 1, &json!({ "entry_id": "e-1", "replaced_by": "e-2" })).unwrap();
    let err_sup = build_solve_writeup(&data_dir, s_sup).unwrap_err();
    assert!(err_sup.contains("Gate G7") && err_sup.contains("superseded"), "unexpected err: {err_sup}");
}

#[test]
fn rule_r5_refuses_missing_and_non_portable_handles() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ledger = Ledger::open(&data_dir).unwrap();

    let s_miss = "s-r5-missing";
    append_setup(&ledger, s_miss, "Missing handle.", "R5 Missing");
    let claim_miss = Claim {
        form: ClaimForm::Equation, lhs: HandleRef::new("ghost", 1).unwrap(),
        rhs: HandleRef::new("ghost", 1).unwrap(), relation: ClaimRelation::Eq,
        order: None, domain: None, how: None,
    };
    append_step(&ledger, s_miss, "e-1", StepEntryType::FinalAnswer, StepOrigin::Solver, "Uses ghost handle.", "", &[], &[claim_miss], None, Some("x = x"), None);
    let err = build_solve_writeup(&data_dir, s_miss).unwrap_err();
    assert!(err.contains("R5") && err.contains("missing handle"), "unexpected err: {err}");

    let s_np = "s-r5-nonportable";
    append_setup(&ledger, s_np, "Non-portable handle.", "R5 Non-portable");
    append_expr(&ledger, s_np, "raw", 1, "raw(x)", "CustomObj()", Some("e-1"), false);
    let claim_np = Claim {
        form: ClaimForm::Value, lhs: HandleRef::new("raw", 1).unwrap(),
        rhs: HandleRef::new("raw", 1).unwrap(), relation: ClaimRelation::Eq,
        order: None, domain: None, how: None,
    };
    append_step(&ledger, s_np, "e-1", StepEntryType::FinalAnswer, StepOrigin::Solver, "Uses non-portable.", "", &[], &[claim_np], None, None, None);
    let err_np = build_solve_writeup(&data_dir, s_np).unwrap_err();
    assert!(err_np.contains("R5") && err_np.contains("non-portable"), "unexpected err: {err_np}");
}

#[test]
fn cited_ocr_quote_emits_ocr_tag_and_physical_pdf_page_and_refuses_tamper() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ledger = Ledger::open(&data_dir).unwrap();
    let sid = "s-ocr-pdf";
    append_setup(&ledger, sid, "Check OCR cited step.", "OCR PDF Cite");

    let sandbox = WorkspaceSandbox::new(&data_dir, None);
    let source_id = "scan-paper-1234";
    let rel_path = format!("{source_id}/scan.pdf.extracted.txt");
    let content = "Cover title line.\n\x0cPage two scanned formula equals 42.\n";
    let file_sha = ledger::sha256_hex(content.as_bytes());
    let sources_root = sandbox.canonical_root(SandboxRoot::Sources, sid, true).unwrap().unwrap();
    let full_file = sources_root.join(&rel_path);
    fs::create_dir_all(full_file.parent().unwrap()).unwrap();
    fs::write(&full_file, content).unwrap();

    let pages = vec![
        PageRecord {
            page: 1, basis: PageBasis::TextLayer, first_line: Some(1), last_line: Some(1),
            metrics: PageQualityMetrics {
                char_count: 17, alpha_count: 14, word_count: 3, image_coverage_pct: 0,
                has_invisible_text: false, width_pt: 612, height_pt: 792,
            },
            reason: None,
        },
        PageRecord {
            page: 2, basis: PageBasis::ScanTextLayer, first_line: Some(2), last_line: Some(2),
            metrics: PageQualityMetrics {
                char_count: 35, alpha_count: 26, word_count: 6, image_coverage_pct: 98,
                has_invisible_text: true, width_pt: 612, height_pt: 792,
            },
            reason: Some("full-page scan".into()),
        },
    ];
    let raw_blob = ledger.put_blob(b"%PDF-1.4 synthetic").unwrap();
    let src_seq = ledger
        .append_with_hash(
            sid, "source.ingested", 1,
            &json!({
                "source_id": source_id, "origin": "scan.pdf", "title": "Synthetic Scanned Note",
                "format": "pdf", "status": "ok", "bytes": 18, "raw_blob": raw_blob,
                "primary_path": rel_path, "files": [rel_path],
                "files_sha256": { rel_path.clone(): file_sha },
                "page_count": 2, "pdf_page_lines": [[1, 1, 1], [2, 2, 2]],
                "pdf_extractor": "pdf-extract-layout/0.12.1", "pdf_pages": pages,
            }),
        )
        .unwrap()
        .seq;

    let quote = "Page two scanned formula equals 42.";
    let qsha = quote_sha256(quote);
    let eq_seq = ledger
        .append_with_hash(
            sid, event_kinds::EVIDENCE_QUOTE, 1,
            &json!({
                "source_id": source_id, "path": rel_path, "start_line": 2, "end_line": 2,
                "file_sha256": file_sha, "quote_sha256": qsha, "quote": quote, "excerpt": quote,
                "source_seq": src_seq, "caller": "user",
            }),
        )
        .unwrap()
        .seq;
    let qref = QuoteRef {
        source_id: source_id.into(), path: rel_path.clone(), start_line: 2, end_line: 2,
        file_sha256: file_sha, quote_sha256: qsha, evidence_seq: Some(eq_seq),
    };
    append_step(&ledger, sid, "e-1", StepEntryType::ReferenceLookup, StepOrigin::Solver, "Formula quoted from scanned PDF page 2.", "", &[], &[], None, None, Some(&qref));
    append_done(&ledger, sid, "e-1", Tier::Cited, Tier::Cited, SolveDoneStatus::Finished);

    let doc = build_solve_writeup(&data_dir, sid).unwrap();
    let quote_blk = doc.sections.iter().flat_map(|s| &s.blocks).find_map(|b| match b {
        WriteupBlock::Quote(q) => Some(q),
        _ => None,
    }).expect("expected verified quote block");
    assert!(quote_blk.ocr, "ScanTextLayer must set ocr: true");
    assert_eq!(quote_blk.locator, QuoteLocator::Page { page: 2, start_line: 2, end_line: 2 });

    let tex = render_writeup_tex(&doc, &WriteupOptions::default()).unwrap();
    let md = render_writeup_markdown(&doc, &WriteupOptions::default()).unwrap();
    assert!(tex.contains("(OCR)") && tex.contains("p. 2, line 2"));
    assert!(md.contains("(OCR)") && md.contains("p. 2, line 2"));
    assert!(tex.contains("OCR source"));

    // Tamper the ingested source file on disk: F4 re-verification MUST fail closed.
    fs::write(&full_file, "Tampered content on disk.\n").unwrap();
    let tamper_err = build_solve_writeup(&data_dir, sid).unwrap_err();
    assert!(tamper_err.contains("F4 re-verification"), "unexpected err: {tamper_err}");
}
