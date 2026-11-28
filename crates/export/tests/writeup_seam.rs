//! Integration tests for the shared F15 write-up seam in `export`:
//! `WriteupFormat::{Tex, Md, Bib}`, `writeup_session_bytes`, `writeup_session_to_path`
//! (sidecar `<stem>.bib` for TeX), Investigate fail-closed skeleton dispatch, and
//! clear `"write-up adapter not built yet"` error for Explore sessions.

mod common_writeup_solve;

use common_writeup_solve::{
    append_check, append_done, append_expr, append_setup, append_step, setup_library_with_citekey,
    SYNTH_SOURCE_TEX,
};
use export::writeup::{writeup_wording, WriteupOptions};
use export::{writeup_session_bytes, writeup_session_to_path, WriteupFormat};
use ledger::Ledger;
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::paper_key::{PaperIds, PaperRef};
use research_tools::quote_core::{verify_quote, QuoteRef};
use research_tools::sandbox::WorkspaceSandbox;
use research_tools::solve_model::{
    compute_claim_sha256, event_kinds, Claim, ClaimForm, ClaimRelation, HandleRef,
    SolveDoneStatus, StepEntryType, StepOrigin,
};
use research_tools::solve_status::Tier;
use research_tools::tool_profile::{MARKER_IDEA_STARTED, MARKER_INVESTIGATION_STARTED};
use serde_json::json;
use std::fs;
use std::sync::Mutex;
use tempfile::TempDir;

#[test]
fn writeup_format_parse_and_mode_dispatch_contracts() {
    assert_eq!(WriteupFormat::parse("tex").unwrap(), WriteupFormat::Tex);
    assert_eq!(WriteupFormat::parse("LATEX").unwrap(), WriteupFormat::Tex);
    assert_eq!(WriteupFormat::parse("md").unwrap(), WriteupFormat::Md);
    assert_eq!(WriteupFormat::parse("markdown").unwrap(), WriteupFormat::Md);
    assert_eq!(WriteupFormat::parse("bib").unwrap(), WriteupFormat::Bib);
    assert_eq!(WriteupFormat::parse("bibtex").unwrap(), WriteupFormat::Bib);
    assert!(WriteupFormat::parse("pdf").is_err());

    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ledger = Ledger::open(&data_dir).unwrap();

    ledger.append_with_hash("s-exp", MARKER_IDEA_STARTED, 1, &json!({})).unwrap();
    ledger.append_with_hash("s-inv", MARKER_INVESTIGATION_STARTED, 1, &json!({})).unwrap();
    ledger.append_with_hash("s-res", "session.start", 1, &json!({})).unwrap();

    let opts = WriteupOptions::default();
    let exp_err = writeup_session_bytes(&data_dir, "s-exp", WriteupFormat::Tex, &opts).unwrap_err();
    assert!(exp_err.contains("idea.stated"), "{exp_err}");
    let inv_err = writeup_session_bytes(&data_dir, "s-inv", WriteupFormat::Md, &opts).unwrap_err();
    assert!(inv_err.contains("investigation.skeleton"), "{inv_err}");
    let res_err = writeup_session_bytes(&data_dir, "s-res", WriteupFormat::Tex, &opts).unwrap_err();
    assert!(res_err.contains("Research"), "{res_err}");
}

#[test]
fn writeup_session_bytes_and_to_path_emit_tex_sidecar_bib_and_md() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let out_dir = tmp.path().join("out");

    let paper = PaperRef::from_ids(
        PaperIds { arxiv: Some("2402.01234".into()), ..Default::default() },
        Some("v1".into()),
    )
    .unwrap();
    setup_library_with_citekey(&data_dir, &paper, "voss2024seam");

    let sid = "s-seam-ok";
    let ledger = Ledger::open(&data_dir).unwrap();
    let math_sha = append_setup(&ledger, sid, "Seam check.", "Seam Derivation");

    let sandbox = WorkspaceSandbox::new(&data_dir, None);
    let mtx = Mutex::new(Ledger::open(&data_dir).unwrap());
    let ing = ingest_bytes(&sandbox, &mtx, &IngestRequest {
        session_id: sid, file_name: "src.tex", origin: &format!("library:{}", paper.key),
        raw_bytes: SYNTH_SOURCE_TEX, source_id_hint: Some("src.tex"),
        section_filter: None, title: Some("Seam Source"),
    })
    .unwrap();
    let evs = ledger.read_session(sid).unwrap();
    let q = "The discrete boundary sum equals 7/12 for every even chain length.";
    let vq = verify_quote(&sandbox, sid, &evs, &ing.source_id, &ing.files[0], 4, 4, q).unwrap();
    let eq_seq = ledger.append_with_hash(sid, event_kinds::EVIDENCE_QUOTE, 1, &json!({
        "source_id": vq.quote_ref.source_id, "path": vq.quote_ref.path,
        "start_line": vq.quote_ref.start_line, "end_line": vq.quote_ref.end_line,
        "file_sha256": vq.quote_ref.file_sha256, "quote_sha256": vq.quote_ref.quote_sha256,
        "quote": q, "excerpt": vq.excerpt, "source_seq": vq.source_seq, "caller": "user",
        "paper_key": paper.key,
    })).unwrap().seq;
    let qref = QuoteRef { evidence_seq: Some(eq_seq), ..vq.quote_ref };
    append_step(&ledger, sid, "e-1", StepEntryType::ReferenceLookup, StepOrigin::Solver, "Cited lemma.", "", &[], &[], None, None, Some(&qref));

    let s1 = append_expr(&ledger, sid, "p", 1, "x + 1", "Add(x,1)", Some("e-2"), true);
    let claim = Claim {
        form: ClaimForm::Equation, lhs: HandleRef::new("p", 1).unwrap(),
        rhs: HandleRef::new("p", 1).unwrap(), relation: ClaimRelation::Eq,
        order: None, domain: None, how: None,
    };
    let csha = compute_claim_sha256(std::slice::from_ref(&claim), |_| Some(s1.as_str())).unwrap();
    append_step(&ledger, sid, "e-2", StepEntryType::FinalAnswer, StepOrigin::Solver, "Final identity.", "", &["e-1"], &[claim], Some(&csha), None, None);
    append_check(&ledger, sid, "chk-1", &csha, &math_sha, "identity_symbolic", "pass", json!({ "residual": "0" }));
    append_done(&ledger, sid, "e-2", Tier::Verified, Tier::Verified, SolveDoneStatus::Finished);

    let opts = WriteupOptions::default();
    let (tex_name, tex_bytes) = writeup_session_bytes(&data_dir, sid, WriteupFormat::Tex, &opts).unwrap();
    let (md_name, md_bytes) = writeup_session_bytes(&data_dir, sid, WriteupFormat::Md, &opts).unwrap();
    let (bib_name, bib_bytes) = writeup_session_bytes(&data_dir, sid, WriteupFormat::Bib, &opts).unwrap();
    assert_eq!(tex_name, "s-seam-ok-writeup.tex");
    assert_eq!(md_name, "s-seam-ok-writeup.md");
    assert_eq!(bib_name, "s-seam-ok-writeup.bib");

    let tex_str = String::from_utf8(tex_bytes).unwrap();
    let md_str = String::from_utf8(md_bytes).unwrap();
    let bib_str = String::from_utf8(bib_bytes).unwrap();
    assert!(tex_str.contains(r"\cite{voss2024seam}"));
    assert!(md_str.contains(writeup_wording::CHECK_TABLE_HEADING));
    assert!(bib_str.contains("@article{voss2024seam,"));

    // `writeup_session_to_path` for Tex writes BOTH `<stem>.tex` and sidecar `<stem>.bib`.
    let out_tex = out_dir.join("my_derivation.tex");
    let written = writeup_session_to_path(
        &data_dir,
        sid,
        WriteupFormat::Tex,
        &WriteupOptions { include_inline_badges: false, bib_stem: String::new() },
        &out_tex,
    )
    .unwrap();
    let disk_tex = fs::read_to_string(&out_tex).unwrap();
    let disk_bib = fs::read_to_string(out_dir.join("my_derivation.bib")).unwrap();
    assert_eq!(written, disk_tex.as_bytes());
    assert!(disk_tex.contains(r"\bibliography{my_derivation}"));
    assert!(!disk_tex.contains("[Verified]"));
    assert!(disk_tex.contains(writeup_wording::CHECK_TABLE_HEADING));
    assert!(disk_bib.contains("@article{voss2024seam,"));

    // Hostile `bib_stem` is sanitized before entering `\bibliography{...}`.
    let (_, hostile_tex_bytes) = writeup_session_bytes(
        &data_dir,
        sid,
        WriteupFormat::Tex,
        &WriteupOptions { include_inline_badges: true, bib_stem: "evil,a%b{c}".into() },
    )
    .unwrap();
    let hostile_tex = String::from_utf8(hostile_tex_bytes).unwrap();
    assert!(hostile_tex.contains(r"\bibliography{evil_a_b_c}"));

    // Passing a `.bib` output path for `WriteupFormat::Tex` is refused (no self-clobber).
    let collide_err = writeup_session_to_path(
        &data_dir,
        sid,
        WriteupFormat::Tex,
        &opts,
        &out_dir.join("refs.bib"),
    )
    .unwrap_err();
    assert!(collide_err.contains(".bib"), "unexpected err: {collide_err}");
}
