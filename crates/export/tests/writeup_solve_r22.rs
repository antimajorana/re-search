//! Set W R22 property sweep over all 11 Solve tiers, own-check-verified downgrade wording,
//! raw OCR flag detection, >300-char quote refusal, and fail-closed ledger integrity check.

mod common_writeup_solve;

use common_writeup_solve::{
    append_check, append_done, append_expr, append_setup, append_step, SYNTH_SOURCE_TEX,
};
use export::writeup::{contains_forbidden_word, HowChecked, WriteupBlock, WriteupOptions};
use export::writeup_md::render_writeup_markdown;
use export::writeup_solve::build_solve_writeup;
use export::writeup_tex::render_writeup_tex;
use ledger::Ledger;
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::quote_core::{quote_sha256, verify_quote, QuoteRef};
use research_tools::sandbox::{SandboxRoot, WorkspaceSandbox};
use research_tools::solve_model::{
    compute_claim_sha256, event_kinds, AckKind, Claim, ClaimForm, ClaimRelation, HandleRef,
    ReviewIndependence, SolveDoneStatus, StepEntryType, StepOrigin,
};
use research_tools::solve_status::Tier;
use serde_json::json;
use std::fs;
use std::sync::Mutex;
use tempfile::TempDir;

#[test]
fn property_sweep_r22_across_all_eleven_solve_tiers() {
    let all_tiers = [
        Tier::Failed,
        Tier::Stale,
        Tier::AssumedByModel,
        Tier::Unchecked,
        Tier::ModelAgreedSameModel,
        Tier::ModelAgreed,
        Tier::CheckedByYou,
        Tier::CheckedNumerically,
        Tier::Verified,
        Tier::Given,
        Tier::Cited,
    ];
    assert_eq!(all_tiers.len(), 11);

    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ledger = Ledger::open(&data_dir).unwrap();

    for (idx, tier) in all_tiers.into_iter().enumerate() {
        let sid = format!("s-r22-tier-{idx}");
        let math_sha = append_setup(&ledger, &sid, "R22 tier sweep.", "Tier Sweep");

        match tier {
            Tier::Failed => {
                let h1 = append_expr(&ledger, &sid, "x", 1, "x", "Symbol('x')", Some("e-1"), true);
                let claim = Claim {
                    form: ClaimForm::Equation, lhs: HandleRef::new("x", 1).unwrap(),
                    rhs: HandleRef::new("x", 1).unwrap(), relation: ClaimRelation::Eq,
                    order: None, domain: None, how: None,
                };
                let csha = compute_claim_sha256(std::slice::from_ref(&claim), |_| Some(h1.as_str())).unwrap();
                append_step(&ledger, &sid, "e-1", StepEntryType::FinalAnswer, StepOrigin::Solver, "Step.", "", &[], &[claim], Some(&csha), None, None);
                append_check(&ledger, &sid, "c1", &csha, &math_sha, "identity_symbolic", "fail", json!({}));
                assert!(build_solve_writeup(&data_dir, &sid).is_err(), "Failed tier must be refused by G7");
                continue;
            }
            Tier::Stale => {
                append_step(&ledger, &sid, "e-1", StepEntryType::Assumption, StepOrigin::Setup, "A.", "", &[], &[], None, None, None);
                append_step(&ledger, &sid, "e-2", StepEntryType::Assumption, StepOrigin::Setup, "B.", "", &[], &[], None, None, None);
                append_step(&ledger, &sid, "e-3", StepEntryType::FinalAnswer, StepOrigin::Solver, "C.", "", &["e-1"], &[], None, None, None);
                ledger.append_with_hash(&sid, event_kinds::PROOF_SUPERSEDE, 1, &json!({ "entry_id": "e-1", "replaced_by": "e-2" })).unwrap();
                assert!(build_solve_writeup(&data_dir, &sid).is_err(), "Stale tier must be refused by G7");
                continue;
            }
            Tier::AssumedByModel => {
                append_step(&ledger, &sid, "e-1", StepEntryType::Assumption, StepOrigin::Solver, "Model assumption.", "", &[], &[], None, None, None);
                append_done(&ledger, &sid, "e-1", tier, tier, SolveDoneStatus::FinishedOpenItems);
            }
            Tier::Unchecked => {
                append_step(&ledger, &sid, "e-1", StepEntryType::FinalAnswer, StepOrigin::Solver, "Unchecked step.", "", &[], &[], None, None, None);
                append_done(&ledger, &sid, "e-1", tier, tier, SolveDoneStatus::FinishedOpenItems);
            }
            Tier::ModelAgreedSameModel => {
                let esha = append_step(&ledger, &sid, "e-1", StepEntryType::FinalAnswer, StepOrigin::Solver, "Same model reviewed.", "", &[], &[], None, None, None);
                ledger.append_with_hash(&sid, event_kinds::PROOF_VERIFICATION, 1, &json!({
                    "target_entry_id": "e-1", "entry_sha256": esha, "verdict": "accepted",
                    "independence": ReviewIndependence::SameModel, "verifier_model": "synth-solver",
                })).unwrap();
                append_done(&ledger, &sid, "e-1", tier, tier, SolveDoneStatus::FinishedOpenItems);
            }
            Tier::ModelAgreed => {
                let esha = append_step(&ledger, &sid, "e-1", StepEntryType::FinalAnswer, StepOrigin::Solver, "Cross model reviewed.", "", &[], &[], None, None, None);
                ledger.append_with_hash(&sid, event_kinds::PROOF_VERIFICATION, 1, &json!({
                    "target_entry_id": "e-1", "entry_sha256": esha, "verdict": "accepted",
                    "independence": ReviewIndependence::DifferentProvider, "verifier_model": "synth-verifier",
                })).unwrap();
                append_done(&ledger, &sid, "e-1", tier, tier, SolveDoneStatus::Finished);
            }
            Tier::CheckedByYou => {
                let esha = append_step(&ledger, &sid, "e-1", StepEntryType::FinalAnswer, StepOrigin::Solver, "Hand checked step.", "", &[], &[], None, None, None);
                ledger.append_with_hash(&sid, event_kinds::SOLVE_ACK, 1, &json!({
                    "entry_id": "e-1", "kind": AckKind::HandChecked, "entry_sha256": esha,
                })).unwrap();
                append_done(&ledger, &sid, "e-1", tier, tier, SolveDoneStatus::Finished);
            }
            Tier::CheckedNumerically => {
                let h1 = append_expr(&ledger, &sid, "x", 1, "x", "Symbol('x')", Some("e-1"), true);
                let claim = Claim {
                    form: ClaimForm::Equation, lhs: HandleRef::new("x", 1).unwrap(),
                    rhs: HandleRef::new("x", 1).unwrap(), relation: ClaimRelation::Eq,
                    order: None, domain: None, how: None,
                };
                let csha = compute_claim_sha256(std::slice::from_ref(&claim), |_| Some(h1.as_str())).unwrap();
                append_step(&ledger, &sid, "e-1", StepEntryType::FinalAnswer, StepOrigin::Solver, "Numeric step.", "", &[], &[claim], Some(&csha), None, None);
                append_check(&ledger, &sid, "c-num", &csha, &math_sha, "identity_numeric", "pass", json!({ "points": 12, "digits": 50 }));
                append_done(&ledger, &sid, "e-1", tier, tier, SolveDoneStatus::Finished);
            }
            Tier::Verified => {
                let h1 = append_expr(&ledger, &sid, "x", 1, "x", "Symbol('x')", Some("e-1"), true);
                let claim = Claim {
                    form: ClaimForm::Equation, lhs: HandleRef::new("x", 1).unwrap(),
                    rhs: HandleRef::new("x", 1).unwrap(), relation: ClaimRelation::Eq,
                    order: None, domain: None, how: None,
                };
                let csha = compute_claim_sha256(std::slice::from_ref(&claim), |_| Some(h1.as_str())).unwrap();
                append_step(&ledger, &sid, "e-1", StepEntryType::FinalAnswer, StepOrigin::Solver, "Exact symbolic step.", "", &[], &[claim], Some(&csha), None, None);
                append_check(&ledger, &sid, "c-sym", &csha, &math_sha, "identity_symbolic", "pass", json!({ "residual": "0" }));
                append_done(&ledger, &sid, "e-1", tier, tier, SolveDoneStatus::Finished);
            }
            Tier::Given => {
                append_step(&ledger, &sid, "e-1", StepEntryType::Assumption, StepOrigin::Setup, "Given setup step.", "", &[], &[], None, None, None);
                append_done(&ledger, &sid, "e-1", tier, tier, SolveDoneStatus::Finished);
            }
            Tier::Cited => {
                let sandbox = WorkspaceSandbox::new(&data_dir, None);
                let mtx = Mutex::new(Ledger::open(&data_dir).unwrap());
                let ing = ingest_bytes(&sandbox, &mtx, &IngestRequest {
                    session_id: &sid, file_name: "ref.tex", origin: "test:r22",
                    raw_bytes: SYNTH_SOURCE_TEX, source_id_hint: Some("ref.tex"),
                    section_filter: None, title: Some("Synthetic Reference"),
                }).unwrap();
                let evs = ledger.read_session(&sid).unwrap();
                let q = "The discrete boundary sum equals 7/12 for every even chain length.";
                let vq = verify_quote(&sandbox, &sid, &evs, &ing.source_id, &ing.files[0], 4, 4, q).unwrap();
                let seq = ledger.append_with_hash(&sid, event_kinds::EVIDENCE_QUOTE, 1, &json!({
                    "source_id": vq.quote_ref.source_id, "path": vq.quote_ref.path,
                    "start_line": vq.quote_ref.start_line, "end_line": vq.quote_ref.end_line,
                    "file_sha256": vq.quote_ref.file_sha256, "quote_sha256": vq.quote_ref.quote_sha256,
                    "quote": q, "excerpt": vq.excerpt, "source_seq": vq.source_seq, "caller": "user",
                })).unwrap().seq;
                let qref = QuoteRef { evidence_seq: Some(seq), ..vq.quote_ref };
                append_step(&ledger, &sid, "e-1", StepEntryType::ReferenceLookup, StepOrigin::Solver, "Cited step.", "", &[], &[], None, None, Some(&qref));
                append_done(&ledger, &sid, "e-1", tier, tier, SolveDoneStatus::Finished);
            }
        }

        let doc = build_solve_writeup(&data_dir, &sid).unwrap();
        assert_eq!(doc.final_effective_tier, Some(tier));
        assert_eq!(doc.check_table[0].how_checked, HowChecked::Solve(tier));

        let tex = render_writeup_tex(&doc, &WriteupOptions::default()).unwrap();
        let md = render_writeup_markdown(&doc, &WriteupOptions::default()).unwrap();
        let allow_verified = tier == Tier::Verified;
        for (fmt, rendered) in [("TeX", &tex), ("Markdown", &md)] {
            assert!(rendered.contains(tier.label()), "{fmt} missing label for {tier:?}");
            assert_eq!(
                contains_forbidden_word(rendered, allow_verified),
                None,
                "R22 violation in {fmt} for tier {tier:?}:\n{rendered}"
            );
        }
    }
}

#[test]
fn own_verified_resting_on_unchecked_keeps_status_honest_and_notes_own_check_in_table() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ledger = Ledger::open(&data_dir).unwrap();
    let sid = "s-own-vs-eff";
    let math_sha = append_setup(&ledger, sid, "Downgraded final step.", "Own vs Effective");

    append_step(&ledger, sid, "e-1", StepEntryType::Assumption, StepOrigin::Solver, "Unacknowledged model ansatz.", "", &[], &[], None, None, None);
    let h1 = append_expr(&ledger, sid, "f", 1, "x^2", "Pow(x,2)", Some("e-2"), true);
    let claim = Claim {
        form: ClaimForm::Equation, lhs: HandleRef::new("f", 1).unwrap(),
        rhs: HandleRef::new("f", 1).unwrap(), relation: ClaimRelation::Eq,
        order: None, domain: None, how: None,
    };
    let csha = compute_claim_sha256(std::slice::from_ref(&claim), |_| Some(h1.as_str())).unwrap();
    append_step(&ledger, sid, "e-2", StepEntryType::FinalAnswer, StepOrigin::Solver, "Exact symbolic step resting on e-1.", "", &["e-1"], &[claim], Some(&csha), None, None);
    append_check(&ledger, sid, "c-sym", &csha, &math_sha, "identity_symbolic", "pass", json!({ "residual": "0" }));
    append_done(&ledger, sid, "e-2", Tier::Verified, Tier::Unchecked, SolveDoneStatus::FinishedOpenItems);

    let doc = build_solve_writeup(&data_dir, sid).unwrap();
    assert_eq!(doc.final_effective_tier, Some(Tier::Unchecked));
    assert!(!doc.status_line.to_ascii_lowercase().contains("verified"));
    assert!(doc.status_line.contains("Done with open items"));
    assert!(doc.status_line.contains("1 assumed by model"));

    let tex = render_writeup_tex(&doc, &WriteupOptions::default()).unwrap();
    let md = render_writeup_markdown(&doc, &WriteupOptions::default()).unwrap();
    assert!(tex.contains("Unchecked (own check: verified)"));
    assert!(md.contains("Unchecked (own check: verified)"));
}

#[test]
fn raw_ocr_flag_quote_cap_and_tampered_ledger_chain_are_enforced() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ledger = Ledger::open(&data_dir).unwrap();

    // 1. Raw `source_kind: "ocr"` + `page: 5` on `evidence.quote` sets ocr=true and p. 5.
    let sid = "s-raw-ocr";
    append_setup(&ledger, sid, "Raw OCR signal.", "Raw OCR");
    let sandbox = WorkspaceSandbox::new(&data_dir, None);
    let mtx = Mutex::new(Ledger::open(&data_dir).unwrap());
    let ing = ingest_bytes(&sandbox, &mtx, &IngestRequest {
        session_id: sid, file_name: "old.txt", origin: "old.txt",
        raw_bytes: b"Line one.\nShort scanned equation line.\n", source_id_hint: Some("old.txt"),
        section_filter: None, title: Some("Old Scan"),
    }).unwrap();
    let evs = ledger.read_session(sid).unwrap();
    let q = "Short scanned equation line.";
    let vq = verify_quote(&sandbox, sid, &evs, &ing.source_id, &ing.files[0], 2, 2, q).unwrap();
    let seq = ledger.append_with_hash(sid, event_kinds::EVIDENCE_QUOTE, 1, &json!({
        "source_id": vq.quote_ref.source_id, "path": vq.quote_ref.path,
        "start_line": 2, "end_line": 2, "file_sha256": vq.quote_ref.file_sha256,
        "quote_sha256": vq.quote_ref.quote_sha256, "quote": q, "excerpt": vq.excerpt,
        "source_seq": vq.source_seq, "caller": "user", "source_kind": "ocr", "page": 5,
    })).unwrap().seq;
    let qref = QuoteRef { evidence_seq: Some(seq), ..vq.quote_ref };
    append_step(&ledger, sid, "e-1", StepEntryType::ReferenceLookup, StepOrigin::Solver, "Raw OCR cite.", "", &[], &[], None, None, Some(&qref));
    append_done(&ledger, sid, "e-1", Tier::Cited, Tier::Cited, SolveDoneStatus::Finished);
    let doc = build_solve_writeup(&data_dir, sid).unwrap();
    let qblk = doc.sections.iter().flat_map(|s| &s.blocks).find_map(|b| match b {
        WriteupBlock::Quote(q) => Some(q),
        _ => None,
    }).unwrap();
    assert!(qblk.ocr);
    assert_eq!(qblk.locator.display_text(), "p. 5, line 2");

    // 2. Quote exceeding 300 characters is refused by build_verified_quote.
    let s_long = "s-long-quote";
    append_setup(&ledger, s_long, "Long quote cap.", "Long Quote");
    let long_quote = "a".repeat(305);
    let src_id = "long-src";
    let rel = format!("{src_id}/long.txt");
    let file_sha = ledger::sha256_hex(long_quote.as_bytes());
    let sroot = sandbox.canonical_root(SandboxRoot::Sources, s_long, true).unwrap().unwrap();
    fs::create_dir_all(sroot.join(src_id)).unwrap();
    fs::write(sroot.join(&rel), &long_quote).unwrap();
    let raw_blob = ledger.put_blob(long_quote.as_bytes()).unwrap();
    let sseq = ledger.append_with_hash(s_long, "source.ingested", 1, &json!({
        "source_id": src_id, "origin": "long.txt", "format": "txt", "status": "ok",
        "bytes": long_quote.len(), "raw_blob": raw_blob, "primary_path": rel,
        "files": [rel], "files_sha256": { rel.clone(): file_sha },
    })).unwrap().seq;
    let lsha = quote_sha256(&long_quote);
    let lseq = ledger.append_with_hash(s_long, event_kinds::EVIDENCE_QUOTE, 1, &json!({
        "source_id": src_id, "path": rel, "start_line": 1, "end_line": 1,
        "file_sha256": file_sha, "quote_sha256": lsha, "quote": long_quote,
        "excerpt": long_quote, "source_seq": sseq, "caller": "user",
    })).unwrap().seq;
    let lref = QuoteRef {
        source_id: src_id.into(), path: rel, start_line: 1, end_line: 1,
        file_sha256: file_sha, quote_sha256: lsha, evidence_seq: Some(lseq),
    };
    append_step(&ledger, s_long, "e-1", StepEntryType::ReferenceLookup, StepOrigin::Solver, "Overlong quote.", "", &[], &[], None, None, Some(&lref));
    append_done(&ledger, s_long, "e-1", Tier::Cited, Tier::Cited, SolveDoneStatus::Finished);
    let long_err = build_solve_writeup(&data_dir, s_long).unwrap_err();
    assert!(long_err.contains("quote_too_long"), "unexpected err: {long_err}");

    // 3. Tampered ledger chain is refused fail-closed via `read_verified_session`.
    drop(mtx);
    drop(ledger);
    let needle = b"Raw OCR cite.";
    let mut mutated = false;
    for fname in ["ledger.db", "ledger.db-wal"] {
        let path = data_dir.join(fname);
        if !path.is_file() {
            continue;
        }
        let mut raw_bytes = fs::read(&path).unwrap();
        let mut i = 0;
        while i + needle.len() <= raw_bytes.len() {
            if &raw_bytes[i..i + needle.len()] == needle {
                raw_bytes[i] = b'X';
                mutated = true;
                i += needle.len();
            } else {
                i += 1;
            }
        }
        if mutated {
            fs::write(&path, raw_bytes).unwrap();
        }
    }
    assert!(mutated, "expected stored payload bytes in ledger.db or ledger.db-wal");
    let err = build_solve_writeup(&data_dir, sid).unwrap_err();
    assert!(err.contains("ledger verification failed"), "unexpected err: {err}");
}

#[test]
fn audit_regressions_status_r22_utf8_and_ocr_fallback() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ledger = Ledger::open(&data_dir).unwrap();

    // 1. Stale `solve.done` followed by `session.resume` and `solve.paused` must not report "Done".
    let s_res = "s-resumed-after-done";
    append_setup(&ledger, s_res, "Resume after done.", "Resume Status");
    append_step(&ledger, s_res, "e-1", StepEntryType::FinalAnswer, StepOrigin::Solver, "First answer.", "", &[], &[], None, None, None);
    append_done(&ledger, s_res, "e-1", Tier::Unchecked, Tier::Unchecked, SolveDoneStatus::Finished);
    ledger.append_with_hash(s_res, event_kinds::SESSION_RESUME, 1, &json!({})).unwrap();
    let doc_running = build_solve_writeup(&data_dir, s_res).unwrap();
    assert!(doc_running.status_line.starts_with("In progress"), "got: {}", doc_running.status_line);
    ledger.append_with_hash(s_res, event_kinds::SOLVE_PAUSED, 1, &json!({
        "reason": "user", "leg": 2, "turn": 1, "model_calls_used": 2, "open_items": []
    })).unwrap();
    let doc_paused = build_solve_writeup(&data_dir, s_res).unwrap();
    assert!(doc_paused.status_line.starts_with("Paused: user"), "got: {}", doc_paused.status_line);

    // 2. Model-authored step prose claiming "verified" on non-Verified step or "proved" is refused (R22).
    let s_bad_r22 = "s-bad-r22-prose";
    append_setup(&ledger, s_bad_r22, "Honesty lint.", "R22 Prose");
    append_step(&ledger, s_bad_r22, "e-1", StepEntryType::FinalAnswer, StepOrigin::Solver, "This step is verified by inspection.", "", &[], &[], None, None, None);
    let r22_err = build_solve_writeup(&data_dir, s_bad_r22).unwrap_err();
    assert!(r22_err.contains("R22"), "expected R22 refusal, got: {r22_err}");

    // 3. Non-ASCII UTF-8 `raw_blob` and `file_sha256` must return Err cleanly (never panic on [..16]).
    let s_utf8 = "s-utf8-hash";
    append_setup(&ledger, s_utf8, "UTF-8 hash guard.", "UTF8 Guard");
    let bad_hash = "éééééééééééééééééé"; // multi-byte UTF-8 spanning byte index 16
    let qsha = quote_sha256("hello");
    ledger.append_with_hash(s_utf8, "source.ingested", 1, &json!({
        "source_id": "src-u", "origin": "u.txt", "format": "txt", "status": "ok",
        "bytes": 5, "raw_blob": bad_hash, "primary_path": "src-u/u.txt",
        "files": ["src-u/u.txt"], "files_sha256": { "src-u/u.txt": bad_hash },
    })).unwrap();
    let eq_seq = ledger.append_with_hash(s_utf8, event_kinds::EVIDENCE_QUOTE, 1, &json!({
        "source_id": "src-u", "path": "src-u/u.txt", "start_line": 1, "end_line": 1,
        "file_sha256": bad_hash, "quote_sha256": qsha, "quote": "hello", "excerpt": "hello",
        "source_seq": 3, "caller": "user",
    })).unwrap().seq;
    let qref = QuoteRef {
        source_id: "src-u".into(), path: "src-u/u.txt".into(), start_line: 1, end_line: 1,
        file_sha256: bad_hash.into(), quote_sha256: qsha, evidence_seq: Some(eq_seq),
    };
    append_step(&ledger, s_utf8, "e-1", StepEntryType::ReferenceLookup, StepOrigin::Solver, "UTF8 ref.", "", &[], &[], None, None, Some(&qref));
    append_done(&ledger, s_utf8, "e-1", Tier::Cited, Tier::Cited, SolveDoneStatus::Finished);
    assert!(build_solve_writeup(&data_dir, s_utf8).is_err());

    // 4. `pdf_pages` with missing line bounds + `pdf_page_lines` fallback still detects OCR scan on page 2.
    let s_fb = "s-ocr-fallback";
    append_setup(&ledger, s_fb, "OCR fallback.", "OCR Fallback");
    let sandbox = WorkspaceSandbox::new(&data_dir, None);
    let src_id = "scan-fb";
    let rel = format!("{src_id}/scan.pdf.extracted.txt");
    let content = "Page one text.\n\x0cPage two scanned result equals 9.\n";
    let fsha = ledger::sha256_hex(content.as_bytes());
    let sroot = sandbox.canonical_root(SandboxRoot::Sources, s_fb, true).unwrap().unwrap();
    fs::create_dir_all(sroot.join(src_id)).unwrap();
    fs::write(sroot.join(&rel), content).unwrap();
    let raw_blob = ledger.put_blob(b"%PDF-1.4 fb").unwrap();
    let sseq = ledger.append_with_hash(s_fb, "source.ingested", 1, &json!({
        "source_id": src_id, "origin": "scan.pdf", "title": "Fallback Scan",
        "format": "pdf", "status": "ok", "bytes": 11, "raw_blob": raw_blob,
        "primary_path": rel, "files": [rel], "files_sha256": { rel.clone(): fsha },
        "page_count": 2, "pdf_page_lines": [[1, 1, 1], [2, 2, 2]],
        "pdf_pages": [
            { "page": 1, "basis": "text_layer", "metrics": { "char_count": 14, "alpha_count": 11, "word_count": 3, "image_coverage_pct": 0, "has_invisible_text": false, "width_pt": 612, "height_pt": 792 } },
            { "page": 2, "basis": "scan_text_layer", "metrics": { "char_count": 30, "alpha_count": 22, "word_count": 6, "image_coverage_pct": 95, "has_invisible_text": true, "width_pt": 612, "height_pt": 792 } }
        ]
    })).unwrap().seq;
    let q = "Page two scanned result equals 9.";
    let fqsha = quote_sha256(q);
    let feq = ledger.append_with_hash(s_fb, event_kinds::EVIDENCE_QUOTE, 1, &json!({
        "source_id": src_id, "path": rel, "start_line": 2, "end_line": 2,
        "file_sha256": fsha, "quote_sha256": fqsha, "quote": q, "excerpt": q,
        "source_seq": sseq, "caller": "user",
    })).unwrap().seq;
    let fqref = QuoteRef {
        source_id: src_id.into(), path: rel, start_line: 2, end_line: 2,
        file_sha256: fsha, quote_sha256: fqsha, evidence_seq: Some(feq),
    };
    append_step(&ledger, s_fb, "e-1", StepEntryType::ReferenceLookup, StepOrigin::Solver, "Fallback scan cite.", "", &[], &[], None, None, Some(&fqref));
    append_done(&ledger, s_fb, "e-1", Tier::Cited, Tier::Cited, SolveDoneStatus::Finished);
    let doc_fb = build_solve_writeup(&data_dir, s_fb).unwrap();
    let qblk = doc_fb.sections.iter().flat_map(|s| &s.blocks).find_map(|b| match b {
        WriteupBlock::Quote(q) => Some(q),
        _ => None,
    }).unwrap();
    assert!(qblk.ocr, "fallback page lookup must still detect ScanTextLayer");
    assert_eq!(qblk.locator.display_text(), "p. 2, line 2");
}
