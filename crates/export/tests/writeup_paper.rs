//! Integration and Set W tests for F15 Investigate reading-notes write-up adapter
//! (`export/src/writeup_paper.rs`, `00-overview.md` 2.13 & 7.2–7.3, `investigate.md` 6.21–6.22, 12, D9).

mod writeup_paper_support;

use export::writeup::{writeup_wording, WriteupBlock, WriteupOptions};
use export::writeup_bib::render_bibtex;
use export::writeup_md::render_writeup_markdown;
use export::writeup_paper::{
    build_writeup, build_writeup_with_options, is_permissive_licence, PaperWriteupOptions,
};
use export::writeup_tex::render_writeup_tex;
use ledger::Ledger;
use research_tools::investigation_markers::EVENT_INVESTIGATION_READING;
use research_tools::map_defs::{
    AuthorsClaimLabel, MapReadingEventPayload, ReadingItemRecord, ReadingRejected, ReadingRole,
    VerifiedMapAnchor,
};
use research_tools::quote_core::{quote_sha256, MAX_DISPLAY_QUOTE_CHARS};
use research_tools::sandbox::{SandboxRoot, WorkspaceSandbox};
use research_tools::status_badge::AUTHORS_LABEL_NEW;
use research_tools::{audit_tex_sources, AuditOptions, ResearchEnv};
use serde_json::json;
use std::fs;
use tempfile::TempDir;
use writeup_paper_support::{
    append_full_layer2, seed_pdf_ocr_session, seed_tex_session, TexSeedConfig, LONG_THEOREM_BODY,
    SHORT_LEMMA_BODY,
};

fn assert_no_forbidden_words_outside_authors_new(rendered: &str, context: &str) {
    let lower = rendered.to_ascii_lowercase();
    assert!(
        !lower.contains("verified"),
        "{context}: word \"verified\" must never appear in Investigate write-ups"
    );
    assert!(
        !contains_word(&lower, "proved"),
        "{context}: word \"proved\" must not appear in Investigate write-up chrome"
    );
    assert!(
        !contains_word(&lower, "novel"),
        "{context}: word \"novel\" must not appear in Investigate write-up chrome"
    );
    // Remove every allowed `"Authors: New (model)"` occurrence before asserting zero `"new"`.
    let stripped = lower.replace(&AUTHORS_LABEL_NEW.to_ascii_lowercase(), "");
    assert!(
        !contains_word(&stripped, "new"),
        "{context}: word \"new\" may appear ONLY inside `{AUTHORS_LABEL_NEW}`; remaining text contained `new`"
    );
}

fn contains_word(haystack: &str, needle: &str) -> bool {
    let (hb, nb) = (haystack.as_bytes(), needle.as_bytes());
    if nb.is_empty() || nb.len() > hb.len() {
        return false;
    }
    for i in 0..=(hb.len() - nb.len()) {
        if &hb[i..i + nb.len()] == nb {
            let left_ok = i == 0 || !hb[i - 1].is_ascii_alphanumeric();
            let right_ok = i + nb.len() == hb.len() || !hb[i + nb.len()].is_ascii_alphanumeric();
            if left_ok && right_ok {
                return true;
            }
        }
    }
    false
}

#[test]
fn skeleton_only_writeup_passes_tex_audit_and_resolves_bib_keys() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ws_dir = tmp.path().join("ws");
    fs::create_dir_all(&ws_dir).unwrap();

    // 1. With user's own `source_item.citekey` from F10/F13a Library.
    let seeded = seed_tex_session(
        &data_dir,
        &TexSeedConfig {
            session_id: "s-skel-lib",
            ledger_licence: None,
            use_us_spelling: false,
            library_licence: None,
            user_citekey: Some("lin2024stability"),
        },
    );
    let doc = build_writeup(&data_dir, &seeded.session_id).unwrap();
    assert!(doc.status_line.contains("tex-1, full"));
    assert!(doc.status_line.contains("Skeleton only"));
    assert_eq!(doc.bibliography[0].cite_key, "lin2024stability");

    let opts = WriteupOptions { include_inline_badges: true, bib_stem: "reading_notes".into() };
    let tex = render_writeup_tex(&doc, &opts).unwrap();
    let md = render_writeup_markdown(&doc, &opts).unwrap();
    let bib = render_bibtex(&doc.bibliography);

    assert!(tex.contains("\\cite{lin2024stability}"));
    assert!(bib.contains("@article{lin2024stability,"));
    assert!(md.contains("[@lin2024stability"));
    assert!(md.contains(SHORT_LEMMA_BODY));
    assert!(md.contains(writeup_wording::CHECK_TABLE_HEADING));
    assert_no_forbidden_words_outside_authors_new(&tex, "skeleton-only TeX");
    assert_no_forbidden_words_outside_authors_new(&md, "skeleton-only Markdown");

    // Validate generated TeX + BibTeX end to end with `tex_audit` (0 blockers, 0 errors).
    fs::write(ws_dir.join("reading_notes.tex"), &tex).unwrap();
    fs::write(ws_dir.join("reading_notes.bib"), &bib).unwrap();
    let env = ResearchEnv::with_workspace(&data_dir, &ws_dir).unwrap();
    let ledger = Ledger::open_existing(&data_dir).unwrap();
    let audit = audit_tex_sources(&env, &seeded.session_id, Some(&ledger), &AuditOptions::default()).unwrap();
    assert!(audit.ok, "tex_audit failed: {:?}", audit.issues);
    assert_eq!(audit.blocker_count, 0);
    assert_eq!(audit.error_count, 0);
    assert!(audit.bib_keys_indexed >= 1);

    // 2. Without `library.db`: deterministic fallback key, and never creates library.db on read.
    let tmp_nolib = TempDir::new().unwrap();
    let nolib_data = tmp_nolib.path().join("data");
    let seeded_nolib = seed_tex_session(
        &nolib_data,
        &TexSeedConfig {
            session_id: "s-skel-nolib",
            ledger_licence: None,
            use_us_spelling: false,
            library_licence: None,
            user_citekey: None,
        },
    );
    let doc2 = build_writeup(&nolib_data, &seeded_nolib.session_id).unwrap();
    assert!(doc2.bibliography[0].cite_key.starts_with("lin2024_"));
    assert!(!nolib_data.join("library").join("library.db").exists());
}

#[test]
fn full_reading_honors_user_pins_overrides_notes_and_inline_badge_toggle() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let seeded = seed_tex_session(
        &data_dir,
        &TexSeedConfig {
            session_id: "s-full-pins",
            ledger_licence: Some("http://arxiv.org/licenses/nonexclusive-distrib/1.0/"),
            use_us_spelling: false,
            library_licence: None,
            user_citekey: Some("lin2024stability"),
        },
    );
    // Pin override: user demotes model-labelled Theorem (`not_main_result`) and promotes Lemma (`main_result`).
    append_full_layer2(&data_dir, &seeded, AuthorsClaimLabel::New, true);

    let built = build_writeup_with_options(&data_dir, &seeded.session_id, PaperWriteupOptions { include_notes: true }).unwrap();
    assert!(built.rejected_quotes.is_empty());
    let doc = &built.writeup;

    // Main Results section must include user-pinned Lemma and exclude user-demoted Theorem.
    let main_sec = doc.sections.iter().find(|s| s.title == "Main Results").unwrap();
    let main_text = render_writeup_markdown(
        &export::writeup::Writeup {
            sections: vec![main_sec.clone()],
            check_table: vec![],
            ..doc.clone()
        },
        &WriteupOptions::default(),
    )
    .unwrap();
    assert!(main_text.contains(&seeded.lemma_tag), "user-pinned lemma must be in Main Results");
    assert!(main_text.contains("pinned as main result by you"));
    assert!(
        !main_text.contains(&seeded.theorem_tag),
        "user-demoted theorem must not appear in Main Results"
    );

    // Full document still carries Theorem in Additional Annotated Nodes, plus Authors: New (model),
    // V12 context card, open proof step, F16a override ("corrected by you"), and user notes.
    let opts_inline = WriteupOptions { include_inline_badges: true, bib_stem: "refs".into() };
    let md_on = render_writeup_markdown(doc, &opts_inline).unwrap();
    let tex_on = render_writeup_tex(doc, &opts_inline).unwrap();
    for rendered in [&md_on, &tex_on] {
        assert!(rendered.contains(AUTHORS_LABEL_NEW));
        assert!(rendered.contains("corrected by you"));
        assert!(rendered.contains("Assumption"));
        assert!(rendered.contains("Check whether odd chain lengths keep the upper bound of two."));
        assert!(rendered.contains("Spectral gap intuition"));
        assert!(rendered.contains("(model)"));
        assert_no_forbidden_words_outside_authors_new(rendered, "full reading (inline on)");
    }

    // Q10: turning off inline badges removes inline marks while keeping status line and check table.
    let opts_off = WriteupOptions { include_inline_badges: false, bib_stem: "refs".into() };
    let md_off = render_writeup_markdown(doc, &opts_off).unwrap();
    let tex_off = render_writeup_tex(doc, &opts_off).unwrap();
    for rendered in [&md_off, &tex_off] {
        assert!(rendered.contains("tex-1, full"));
        assert!(rendered.contains(writeup_wording::CHECK_TABLE_HEADING));
        assert!(rendered.contains(AUTHORS_LABEL_NEW));
        assert!(rendered.contains("Model statement (model)"));
        assert_no_forbidden_words_outside_authors_new(rendered, "full reading (inline off)");
    }

    // Opt-out `include_notes: false` omits user text notes while keeping structure and check table.
    let no_notes = build_writeup_with_options(&data_dir, &seeded.session_id, PaperWriteupOptions { include_notes: false }).unwrap();
    let md_no_notes = render_writeup_markdown(&no_notes.writeup, &opts_inline).unwrap();
    assert!(!md_no_notes.contains("Check whether odd chain lengths"));
    assert!(!md_no_notes.contains("Admissibility excludes zero weights"));
    assert!(md_no_notes.contains(writeup_wording::CHECK_TABLE_HEADING));
}

#[test]
fn licence_gate_enforces_300_char_cap_fail_closed_and_allows_cc_by_full_statements() {
    assert!(LONG_THEOREM_BODY.chars().count() > MAX_DISPLAY_QUOTE_CHARS);

    // Direct allow-list unit checks (including adversarial NC/ND/SA/negated bypass strings).
    for rejected in [
        None,
        Some(""),
        Some("http://arxiv.org/licenses/nonexclusive-distrib/1.0/"),
        Some("CC-BY-NC-4.0"),
        Some("CC-BY-ND-4.0"),
        Some("CC-BY-SA-4.0"),
        Some("CC BY NC 4.0"),
        Some("CC BY ND"),
        Some("CC-BY (NonCommercial)"),
        Some("CC-BY (NoDerivs)"),
        Some("CC-BY-4.0-NC"),
        Some("Not CC-BY-4.0"),
        Some("https://creativecommons.org/licenses/by/4.0/ + noncommercial"),
        Some("Proprietary Publisher Licence"),
    ] {
        assert!(!is_permissive_licence(rejected), "licence `{rejected:?}` must fail closed");
    }
    assert!(is_permissive_licence(Some("CC-BY-4.0")));
    assert!(is_permissive_licence(Some("https://creativecommons.org/licenses/by/4.0/")));
    assert!(is_permissive_licence(Some("CC0-1.0")));

    // Restricted / missing / unknown licences must all cap statements strictly <= 300 chars.
    let restricted_cases: &[(&str, Option<&str>, Option<&str>)] = &[
        ("s-lic-none", None, None),
        ("s-lic-arxiv", Some("http://arxiv.org/licenses/nonexclusive-distrib/1.0/"), None),
        ("s-lic-unknown", Some("Custom Academic Licence v1"), None),
        ("s-lic-nc", None, Some("CC-BY-NC-4.0")),
    ];
    for &(sid, ledger_lic, lib_lic) in restricted_cases {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        seed_tex_session(
            &data_dir,
            &TexSeedConfig {
                session_id: sid,
                ledger_licence: ledger_lic,
                use_us_spelling: false,
                library_licence: lib_lic,
                user_citekey: None,
            },
        );
        let built = build_writeup_with_options(&data_dir, sid, PaperWriteupOptions::default()).unwrap();
        assert!(!built.open_licence, "{sid} must fail closed");
        for sec in &built.writeup.sections {
            for blk in &sec.blocks {
                if let WriteupBlock::Quote(q) = blk {
                    assert!(q.text.chars().count() <= MAX_DISPLAY_QUOTE_CHARS);
                }
            }
        }
        let md = render_writeup_markdown(&built.writeup, &WriteupOptions::default()).unwrap();
        assert!(!md.contains(LONG_THEOREM_BODY), "{sid} must not emit full >300-char statement");
    }

    // Permissive CC-BY-4.0 from BOTH ledger `"licence"` AND library `PaperFlags.license` permits full statement.
    let open_cases: &[(&str, Option<&str>, bool, Option<&str>)] = &[
        ("s-lic-ledger-uk", Some("CC-BY-4.0"), false, None),
        ("s-lic-ledger-us", Some("https://creativecommons.org/licenses/by/4.0/"), true, None),
        ("s-lic-library", None, false, Some("CC-BY-4.0")),
    ];
    for &(sid, ledger_lic, us_spelling, lib_lic) in open_cases {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        seed_tex_session(
            &data_dir,
            &TexSeedConfig {
                session_id: sid,
                ledger_licence: ledger_lic,
                use_us_spelling: us_spelling,
                library_licence: lib_lic,
                user_citekey: None,
            },
        );
        let built = build_writeup_with_options(&data_dir, sid, PaperWriteupOptions::default()).unwrap();
        assert!(built.open_licence, "{sid} must recognize permissive CC-BY licence");
        let md = render_writeup_markdown(&built.writeup, &WriteupOptions::default()).unwrap();
        let tex = render_writeup_tex(&built.writeup, &WriteupOptions::default()).unwrap();
        assert!(md.contains(LONG_THEOREM_BODY), "{sid} Markdown must include full CC-BY statement");
        assert!(tex.contains(LONG_THEOREM_BODY), "{sid} TeX must include full CC-BY statement");
    }
}

#[test]
fn pdf_and_ocr_honesty_pages_ocr_tags_and_zero_ocr_math_delimiters() {
    for raw_ocr_basis in [false, true] {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        let sid = if raw_ocr_basis { "s-pdf-raw-ocr" } else { "s-pdf-scan-layer" };
        seed_pdf_ocr_session(&data_dir, sid, raw_ocr_basis);

        let doc = build_writeup(&data_dir, sid).unwrap();
        assert!(doc.status_line.contains("pdf-1"), "status must name pdf-1: {}", doc.status_line);
        assert!(
            doc.status_line.contains("weak") || doc.status_line.contains("partial"),
            "pdf-1 status must state weak/partial quality: {}",
            doc.status_line
        );

        // Zero Formula blocks anywhere in OCR/PDF reading notes.
        for sec in &doc.sections {
            for blk in &sec.blocks {
                assert!(!matches!(blk, WriteupBlock::Formula { .. }), "OCR write-up must emit zero Formula blocks");
            }
        }

        let tex = render_writeup_tex(&doc, &WriteupOptions::default()).unwrap();
        let md = render_writeup_markdown(&doc, &WriteupOptions::default()).unwrap();
        for rendered in [&tex, &md] {
            assert!(rendered.contains("p. 1"), "digital page 1 must carry physical page number");
            assert!(rendered.contains("p. 2"), "scanned page 2 must carry physical page number");
            assert!(rendered.contains(writeup_wording::OCR_TAG), "scanned page quote must carry (OCR) tag");
            assert!(rendered.contains(writeup_wording::LABEL_MATCHED_OCR));
            // Zero OCR math delimiters in TeX or Markdown.
            assert!(!rendered.contains("$$"), "must not wrap OCR text in $$ math delimiters");
            assert!(!rendered.contains("\\["), "must not wrap OCR text in \\[ math delimiters");
            assert!(!rendered.contains("\\begin{equation}"), "must not emit equation env for OCR");
            assert_no_forbidden_words_outside_authors_new(rendered, sid);
        }
    }
}

#[test]
fn wording_property_across_all_authors_claim_labels_and_determinism() {
    for label in [
        AuthorsClaimLabel::New,
        AuthorsClaimLabel::Restated,
        AuthorsClaimLabel::Background,
        AuthorsClaimLabel::NotStated,
    ] {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        let sid = format!("s-word-{label:?}").to_ascii_lowercase();
        let seeded = seed_tex_session(
            &data_dir,
            &TexSeedConfig {
                session_id: &sid,
                ledger_licence: None,
                use_us_spelling: false,
                library_licence: None,
                user_citekey: Some("lin2024stability"),
            },
        );
        append_full_layer2(&data_dir, &seeded, label, false);

        let d1 = build_writeup(&data_dir, &sid).unwrap();
        let d2 = build_writeup(&data_dir, &sid).unwrap();
        let opts = WriteupOptions::default();
        let (tex1, tex2) = (render_writeup_tex(&d1, &opts).unwrap(), render_writeup_tex(&d2, &opts).unwrap());
        let (md1, md2) = (render_writeup_markdown(&d1, &opts).unwrap(), render_writeup_markdown(&d2, &opts).unwrap());
        let (bib1, bib2) = (render_bibtex(&d1.bibliography), render_bibtex(&d2.bibliography));

        assert_eq!(tex1, tex2, "TeX output must be byte-deterministic");
        assert_eq!(md1, md2, "Markdown output must be byte-deterministic");
        assert_eq!(bib1, bib2, "BibTeX output must be byte-deterministic");

        assert_no_forbidden_words_outside_authors_new(&tex1, &format!("{sid} TeX"));
        assert_no_forbidden_words_outside_authors_new(&md1, &format!("{sid} Markdown"));
        if label == AuthorsClaimLabel::New {
            assert!(md1.contains(AUTHORS_LABEL_NEW));
        } else {
            assert!(!contains_word(&md1.to_ascii_lowercase(), "new"));
        }
    }
}

#[test]
fn tamper_refusal_rejects_corrupt_source_hash_corrupt_blob_and_forged_anchor() {
    // 1. Modified physical source file on disk -> hard error.
    let tmp1 = TempDir::new().unwrap();
    let d1 = tmp1.path().join("data");
    let s1 = seed_tex_session(
        &d1,
        &TexSeedConfig {
            session_id: "s-tamper-src",
            ledger_licence: None,
            use_us_spelling: false,
            library_licence: None,
            user_citekey: None,
        },
    );
    let sb1 = WorkspaceSandbox::new(d1.clone(), None);
    let src_file = sb1.resolve_existing(SandboxRoot::Sources, &s1.session_id, &s1.primary_path, false).unwrap();
    fs::write(&src_file.canonical_target, b"tampered contents").unwrap();
    let err1 = build_writeup(&d1, &s1.session_id).unwrap_err();
    assert!(err1.contains("hash mismatch"), "expected hash mismatch error, got: {err1}");

    // 2. Corrupted skeleton blob on disk -> hard error.
    let tmp2 = TempDir::new().unwrap();
    let d2 = tmp2.path().join("data");
    let s2 = seed_tex_session(
        &d2,
        &TexSeedConfig {
            session_id: "s-tamper-blob",
            ledger_licence: None,
            use_us_spelling: false,
            library_licence: None,
            user_citekey: None,
        },
    );
    let blob_path = d2.join("blobs").join(&s2.skeleton_blob[..2]).join(&s2.skeleton_blob);
    fs::write(&blob_path, b"{corrupt}").unwrap();
    let err2 = build_writeup(&d2, &s2.session_id).unwrap_err();
    assert!(
        err2.contains("mismatch") || err2.contains("does not match"),
        "expected blob integrity refusal, got: {err2}"
    );

    // 3. Forged Layer-2 anchor quote -> rejected, never emitted in TeX or Markdown.
    let tmp3 = TempDir::new().unwrap();
    let d3 = tmp3.path().join("data");
    let s3 = seed_tex_session(
        &d3,
        &TexSeedConfig {
            session_id: "s-tamper-anchor",
            ledger_licence: None,
            use_us_spelling: false,
            library_licence: None,
            user_citekey: None,
        },
    );
    let ledger = Ledger::open_existing(&d3).unwrap();
    let thm_node = s3.skeleton.nodes.iter().find(|n| n.tag.as_deref() == Some(&s3.theorem_tag)).unwrap();
    let forged_text = "Fabricated theorem statement that never appeared in the ingested source.";
    let forged_anchor = VerifiedMapAnchor {
        source_id: s3.source_id.clone(),
        path: s3.primary_path.clone(),
        start_line: thm_node.spans[0].start_line,
        end_line: thm_node.spans[0].end_line,
        file_sha256: "f".repeat(64),
        quote_sha256: quote_sha256(forged_text),
        quote: forged_text.into(),
        excerpt: forged_text.into(),
        source_seq: 1,
        cited_range: None,
        range_basis: vec![],
        excerpt_window: None,
    };
    let payload = MapReadingEventPayload {
        schema_version: 1,
        request_seq: 99,
        source_id: s3.source_id.clone(),
        skeleton_seq: s3.skeleton_seq,
        done: true,
        items: vec![ReadingItemRecord {
            tag: s3.theorem_tag.clone(),
            node_fp: thm_node.fp.clone(),
            attach_to: None,
            model_kind: None,
            role: Some(ReadingRole::MainResult),
            gist: Some("Summary of stability.".into()),
            purpose: None,
            hypotheses: vec![],
            anchors: vec![],
            authors_claim: Some(research_tools::map_defs::AuthorsClaimRecord {
                label: AuthorsClaimLabel::Background,
                relation_note: None,
                attributed_to: vec![],
                anchors: vec![forged_anchor],
                cues: vec![],
            }),
        }],
        edges: vec![],
        limitations: vec![],
        rejected: ReadingRejected::default(),
        anchor_report: json!({}),
    };
    ledger.append(&s3.session_id, EVENT_INVESTIGATION_READING, 1, &serde_json::to_value(&payload).unwrap()).unwrap();

    let built = build_writeup_with_options(&d3, &s3.session_id, PaperWriteupOptions::default()).unwrap();
    assert_eq!(built.rejected_quotes.len(), 1, "forged anchor must be recorded as rejected");
    let tex = render_writeup_tex(&built.writeup, &WriteupOptions::default()).unwrap();
    let md = render_writeup_markdown(&built.writeup, &WriteupOptions::default()).unwrap();
    assert!(!tex.contains(forged_text), "forged quote must never be written to TeX");
    assert!(!md.contains(forged_text), "forged quote must never be written to Markdown");
    assert!(
        !md.contains("Authors: Background (model)"),
        "authors' claim with zero surviving verified anchors must be omitted"
    );
    assert!(md.contains("could not be matched"), "rejection must be disclosed in check table");
}

#[test]
fn shared_writeup_session_dispatch_serves_tex_md_bib_writes_sidecar_and_gates_other_modes() {
    use export::{writeup_session_bytes, writeup_session_to_path, WriteupFormat};

    assert_eq!(WriteupFormat::parse("tex").unwrap(), WriteupFormat::Tex);
    assert_eq!(WriteupFormat::parse("markdown").unwrap(), WriteupFormat::Md);
    assert_eq!(WriteupFormat::parse("bib").unwrap(), WriteupFormat::Bib);
    assert!(WriteupFormat::parse("docx").is_err());

    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ws_dir = tmp.path().join("ws");
    fs::create_dir_all(&ws_dir).unwrap();
    let seeded = seed_tex_session(
        &data_dir,
        &TexSeedConfig {
            session_id: "s-dispatch-inv",
            ledger_licence: None,
            use_us_spelling: false,
            library_licence: None,
            user_citekey: Some("lin2024stability"),
        },
    );
    append_full_layer2(&data_dir, &seeded, AuthorsClaimLabel::New, false);

    let opts = WriteupOptions::default();
    let (tex_name, tex_bytes_a) =
        writeup_session_bytes(&data_dir, &seeded.session_id, WriteupFormat::Tex, &opts).unwrap();
    let (_, tex_bytes_b) =
        writeup_session_bytes(&data_dir, &seeded.session_id, WriteupFormat::Tex, &opts).unwrap();
    let (md_name, md_bytes) =
        writeup_session_bytes(&data_dir, &seeded.session_id, WriteupFormat::Md, &opts).unwrap();
    let (bib_name, bib_bytes) =
        writeup_session_bytes(&data_dir, &seeded.session_id, WriteupFormat::Bib, &opts).unwrap();
    assert_eq!(tex_name, "s-dispatch-inv-writeup.tex");
    assert_eq!(md_name, "s-dispatch-inv-writeup.md");
    assert_eq!(bib_name, "s-dispatch-inv-writeup.bib");
    assert_eq!(tex_bytes_a, tex_bytes_b, "writeup_session_bytes must be byte-deterministic");
    assert!(String::from_utf8_lossy(&md_bytes).contains(AUTHORS_LABEL_NEW));
    assert!(String::from_utf8_lossy(&bib_bytes).contains("@article{lin2024stability,"));

    // Writing `.tex` to disk also writes sidecar `.bib` and passes `tex_audit` cleanly.
    let out_tex = ws_dir.join("notes.tex");
    let written = writeup_session_to_path(
        &data_dir,
        &seeded.session_id,
        WriteupFormat::Tex,
        &opts,
        &out_tex,
    )
    .unwrap();
    assert!(out_tex.is_file());
    assert!(out_tex.with_extension("bib").is_file());
    assert!(String::from_utf8_lossy(&written).contains("\\bibliography{notes}"));
    let env = ResearchEnv::with_workspace(&data_dir, &ws_dir).unwrap();
    let ledger = Ledger::open_existing(&data_dir).unwrap();
    let audit =
        audit_tex_sources(&env, &seeded.session_id, Some(&ledger), &AuditOptions::default()).unwrap();
    assert!(audit.ok, "tex_audit failed on writeup_session_to_path output: {:?}", audit.issues);
    assert_eq!(audit.blocker_count, 0);
    assert_eq!(audit.error_count, 0);

    // Non-Investigate session fails cleanly with `"write-up adapter not built yet"`.
    ledger.append("s-research-other", "session.created", 1, &json!({ "title": "Research" })).unwrap();
    let err = writeup_session_bytes(&data_dir, "s-research-other", WriteupFormat::Md, &opts)
        .unwrap_err();
    assert!(
        err.contains("write-up adapter not built yet"),
        "expected unbuilt-mode error, got: {err}"
    );
}
