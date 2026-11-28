//! Regression tests for Gemini 4 Carbon audit findings on F15 Investigate write-up adapter
//! (`writeup_paper*.rs` and `writeup_md.rs`).

#[allow(dead_code)]
mod writeup_paper_support;

use export::writeup::{writeup_wording, WriteupOptions};
use export::writeup_bib::render_bibtex;
use export::writeup_md::render_writeup_markdown;
use export::writeup_paper::{build_writeup_with_options, PaperWriteupOptions};
use export::writeup_tex::render_writeup_tex;
use ledger::{sha256_hex, Ledger};
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::investigation_markers::{
    EVENT_INVESTIGATION_CONTEXT, EVENT_INVESTIGATION_NOTE, EVENT_INVESTIGATION_OVERRIDE,
    EVENT_INVESTIGATION_READING,
};
use research_tools::map_context_defs::{
    ContextCardOrigin, ContextCardRecord, ContextCardType, MapContextEventPayload,
    CONTEXT_SCHEMA_VERSION,
};
use research_tools::map_defs::{
    MapReadingEventPayload, ReadingItemRecord, ReadingRejected, ReadingRole, VerifiedMapAnchor,
};
use research_tools::paper_key::{PaperIds, PaperRef};
use research_tools::quote_core::{quote_sha256, verify_quote};
use research_tools::sandbox::WorkspaceSandbox;
use research_tools::skeleton::SkNodeKind;
use serde_json::json;
use std::sync::Mutex;
use tempfile::TempDir;
use writeup_paper_support::{seed_tex_session, TexSeedConfig};

#[test]
fn external_prerequisite_ocr_pdf_card_resolves_bib_pages_neutralizes_md_math_and_marks_model() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let seeded = seed_tex_session(
        &data_dir,
        &TexSeedConfig {
            session_id: "s-audit-ext-ocr",
            ledger_licence: None,
            use_us_spelling: false,
            library_licence: None,
            user_citekey: Some("lin2024stability"),
        },
    );

    // Ingest an external cited PDF with a scanned page 2 containing literal `$` math delimiters.
    let sandbox = WorkspaceSandbox::new(data_dir.clone(), None);
    let ledger_mtx = Mutex::new(Ledger::open_existing(&data_dir).unwrap());
    let ext_pdf_text = concat!(
        "1. Background\n",
        "External digital page one text.\n",
        "Theorem 2.1. Every positive matrix satisfies $x_n \\le 2$ and $$E_0 > 0$$ on page two.\n",
    );
    let ing_ext = ingest_bytes(
        &sandbox,
        &ledger_mtx,
        &IngestRequest {
            session_id: &seeded.session_id,
            file_name: "cited_scan.txt",
            origin: "https://arxiv.org/pdf/2101.09999.pdf",
            raw_bytes: ext_pdf_text.as_bytes(),
            source_id_hint: Some("ext-pdf-1"),
            section_filter: None,
            title: Some("Classical Lattice Positivity"),
        },
    )
    .unwrap();
    drop(ledger_mtx);

    let ledger = Ledger::open_existing(&data_dir).unwrap();
    let ext_path = ing_ext.files[0].clone();
    // Append a PDF-page receipt for the external source + an investigation.cited_lookup record.
    let ext_hash = sha256_hex(ext_pdf_text.as_bytes());
    ledger
        .append_with_hash(
            &seeded.session_id,
            "source.ingested",
            1,
            &json!({
                "source_id": ing_ext.source_id,
                "origin": "https://arxiv.org/pdf/2101.09999.pdf",
                "title": "Classical Lattice Positivity",
                "authors": ["Weber, Hans"],
                "year": 2021,
                "format": "pdf",
                "primary_path": ext_path,
                "files": [&ext_path],
                "files_sha256": { &ext_path: ext_hash },
                "pdf_pages": [
                    { "page": 1, "first_line": 1, "last_line": 2, "basis": "text_layer" },
                    { "page": 2, "first_line": 3, "last_line": 3, "basis": "scan_text_layer" }
                ],
            }),
        )
        .unwrap();

    let ext_ref = PaperRef::from_ids(
        PaperIds { arxiv: Some("2101.09999".into()), ..Default::default() },
        None,
    )
    .unwrap();
    ledger
        .append_with_hash(
            &seeded.session_id,
            "investigation.cited_lookup",
            1,
            &json!({
                "tag": seeded.theorem_tag,
                "bib_key": "weber2021",
                "locator": "Theorem 2.1",
                "resolved": ext_ref,
                "external_source_id": ing_ext.source_id,
                "status": "found",
            }),
        )
        .unwrap();

    // Build an external Prerequisite context card with `bib_key: None` (matching map_context.rs).
    let evs = ledger.read_session(&seeded.session_id).unwrap();
    let ext_quote = "Every positive matrix satisfies $x_n \\le 2$ and $$E_0 > 0$$ on page two.";
    let vq = verify_quote(&sandbox, &seeded.session_id, &evs, &ing_ext.source_id, &ext_path, 3, 3, ext_quote).unwrap();
    let ext_anchor = VerifiedMapAnchor {
        source_id: vq.quote_ref.source_id.clone(),
        path: vq.quote_ref.path.clone(),
        start_line: vq.quote_ref.start_line,
        end_line: vq.quote_ref.end_line,
        file_sha256: vq.quote_ref.file_sha256.clone(),
        quote_sha256: quote_sha256(ext_quote),
        quote: ext_quote.into(),
        excerpt: vq.excerpt,
        source_seq: vq.source_seq,
        cited_range: None,
        range_basis: vec![],
        excerpt_window: None,
    };
    let ctx_payload = MapContextEventPayload {
        schema_version: CONTEXT_SCHEMA_VERSION,
        request_seq: 10,
        source_id: seeded.source_id.clone(),
        skeleton_seq: seeded.skeleton_seq,
        tag: seeded.theorem_tag.clone(),
        cards: vec![ContextCardRecord {
            card_type: ContextCardType::Prerequisite,
            origin: ContextCardOrigin::External,
            title: "External positivity prerequisite".into(),
            body: "Positivity bound drawn from an earlier scan.".into(),
            bib_key: None,
            external_source_id: Some(ing_ext.source_id.clone()),
            external_tag: None,
            node_fp: None,
            anchors: vec![ext_anchor],
            hypotheses: vec![],
        }],
        rejected: vec![],
        anchor_report: json!({}),
    };
    ledger
        .append_with_hash(
            &seeded.session_id,
            EVENT_INVESTIGATION_CONTEXT,
            1,
            &serde_json::to_value(&ctx_payload).unwrap(),
        )
        .unwrap();

    let built = build_writeup_with_options(&data_dir, &seeded.session_id, PaperWriteupOptions::default()).unwrap();
    assert!(built.rejected_quotes.is_empty(), "external Prerequisite card must not be rejected: {:?}", built.rejected_quotes);
    let md = render_writeup_markdown(&built.writeup, &WriteupOptions::default()).unwrap();
    let tex = render_writeup_tex(&built.writeup, &WriteupOptions::default()).unwrap();
    let bib = render_bibtex(&built.writeup.bibliography);

    // Must preserve card, resolve external BibTeX entry, tag (model), show p. 2 + (OCR),
    // and neutralize raw `$` / `$$` OCR math delimiters in Markdown as well as TeX.
    assert!(md.contains("External positivity prerequisite"), "external Prerequisite card missing:\n{md}");
    assert!(md.contains("*(model)*"), "context card prose must be marked (model)");
    assert!(md.contains("p. 2"), "external scanned PDF anchor must carry physical page 2");
    assert!(md.contains(writeup_wording::OCR_TAG), "external scanned PDF anchor must carry (OCR)");
    assert!(!md.contains("$$"), "OCR quote in Markdown must not contain $$ math delimiters:\n{md}");
    assert!(!md.contains("$x_n"), "OCR quote in Markdown must not contain $ math delimiters:\n{md}");
    assert!(!tex.contains("$$"), "OCR quote in TeX must not contain $$ math delimiters");
    assert!(bib.contains("2101.09999"), "external source must resolve to BibTeX entry:\n{bib}");
    assert!(!md.to_ascii_lowercase().contains("verified"));
}

#[test]
fn section_notes_override_note_privacy_and_disallowed_model_wording_guard() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let seeded = seed_tex_session(
        &data_dir,
        &TexSeedConfig {
            session_id: "s-audit-wording-notes",
            ledger_licence: None,
            use_us_spelling: false,
            library_licence: None,
            user_citekey: Some("lin2024stability"),
        },
    );
    let ledger = Ledger::open_existing(&data_dir).unwrap();
    let sec_tag = seeded
        .skeleton
        .nodes
        .iter()
        .find(|n| n.kind == SkNodeKind::Section)
        .unwrap()
        .tag
        .clone()
        .unwrap();
    let thm_node = seeded
        .skeleton
        .nodes
        .iter()
        .find(|n| n.tag.as_deref() == Some(&seeded.theorem_tag))
        .unwrap();

    // Attach a user note directly to a Section node, plus an F16a override with a private note.
    ledger
        .append_with_hash(
            &seeded.session_id,
            EVENT_INVESTIGATION_NOTE,
            1,
            &json!({ "tag": sec_tag, "text": "Section-level reading memo to keep." }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            &seeded.session_id,
            EVENT_INVESTIGATION_OVERRIDE,
            1,
            &json!({
                "source_id": seeded.source_id,
                "tag": seeded.lemma_tag,
                "fix": { "kind": "Assumption" },
                "note": "Private override memo hidden when include_notes=false.",
            }),
        )
        .unwrap();

    // Inject model gist containing forbidden claim word `"verified"` and purpose with `"novel"`.
    let reading = MapReadingEventPayload {
        schema_version: 1,
        request_seq: 1,
        source_id: seeded.source_id.clone(),
        skeleton_seq: seeded.skeleton_seq,
        done: true,
        items: vec![ReadingItemRecord {
            tag: seeded.theorem_tag.clone(),
            node_fp: thm_node.fp.clone(),
            attach_to: None,
            model_kind: None,
            role: Some(ReadingRole::Supporting),
            gist: Some("This theorem is verified to hold globally.".into()),
            purpose: Some("Introduces a novel trick.".into()),
            hypotheses: vec![],
            anchors: vec![],
            authors_claim: None,
        }],
        edges: vec![],
        limitations: vec![],
        rejected: ReadingRejected::default(),
        anchor_report: json!({}),
    };
    ledger
        .append_with_hash(
            &seeded.session_id,
            EVENT_INVESTIGATION_READING,
            1,
            &serde_json::to_value(&reading).unwrap(),
        )
        .unwrap();

    // With include_notes: true -> section note AND override note appear; forbidden model text omitted!
    let with_notes = build_writeup_with_options(
        &data_dir,
        &seeded.session_id,
        PaperWriteupOptions { include_notes: true },
    )
    .unwrap();
    let md_with = render_writeup_markdown(&with_notes.writeup, &WriteupOptions::default()).unwrap();
    let tex_with = render_writeup_tex(&with_notes.writeup, &WriteupOptions::default()).unwrap();
    assert!(md_with.contains("Section-level reading memo to keep."), "section user note missing:\n{md_with}");
    assert!(md_with.contains("Private override memo hidden when include_notes=false."));
    for rendered in [&md_with, &tex_with] {
        assert!(!rendered.to_ascii_lowercase().contains("verified"), "forbidden word leaked:\n{rendered}");
        assert!(!rendered.to_ascii_lowercase().contains("novel"), "forbidden word leaked:\n{rendered}");
        assert!(rendered.contains("disallowed claim wording"), "omission must be disclosed in check table");
    }

    // With include_notes: false -> BOTH section user note AND override private note are omitted!
    let no_notes = build_writeup_with_options(
        &data_dir,
        &seeded.session_id,
        PaperWriteupOptions { include_notes: false },
    )
    .unwrap();
    let md_no = render_writeup_markdown(&no_notes.writeup, &WriteupOptions::default()).unwrap();
    assert!(!md_no.contains("Section-level reading memo to keep."));
    assert!(!md_no.contains("Private override memo hidden when include_notes=false."));
    assert!(md_no.contains("corrected by you"), "override label stays even when private note is hidden");
}
