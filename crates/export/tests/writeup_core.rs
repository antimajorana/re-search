//! Tests for package F15 core: mode-neutral write-up model, TeX/Markdown/BibTeX writers,
//! single wording table across every tier & writer, F4 quote_core enforcement, golden output
//! checks, and `tex_audit` validation (`00-overview.md` 2.13 & 7.2 set W).

use export::writeup::{
    all_non_verified_writeup_labels, build_verified_quote, contains_forbidden_word,
    writeup_wording, BibEntry, CheckTableRow, HowChecked, ProseBasis, QuoteLocator, Writeup,
    WriteupBlock, WriteupMode, WriteupOptions, WriteupQuote, WriteupSection,
};
use export::writeup_bib::{render_bibtex, resolve_bib_entry};
use export::writeup_md::{render_verified_writeup_markdown, render_writeup_markdown};
use export::writeup_tex::{render_verified_writeup_tex, render_writeup_tex};
use ledger::Ledger;
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::library::{
    Library, LibraryIds, PaperFlags, PaperRecord, SourceItemRecord, SourceItemState, SourceKind,
    SourceRecord,
};
use research_tools::paper_key::{PaperIds, PaperRef};
use research_tools::quote_core::{quote_sha256, verify_quote, QuoteRef};
use research_tools::sandbox::WorkspaceSandbox;
use research_tools::solve_status::Tier;
use research_tools::{audit_tex_sources, AuditOptions, ResearchEnv};
use serde_json::json;
use std::collections::BTreeSet;
use std::fs;
use std::sync::Mutex;
use tempfile::TempDir;

const GOLDEN_TEX: &str = include_str!("golden/writeup_sample.tex");
const GOLDEN_MD: &str = include_str!("golden/writeup_sample.md");
const GOLDEN_BIB: &str = include_str!("golden/writeup_sample.bib");

const SYNTH_PAPER_TEX: &[u8] = br"\documentclass{article}
\begin{document}
\section{Boundary Sum}
The discrete boundary sum equals 7/12 for every even chain length.
\end{document}
";

fn sample_paper_voss() -> PaperRef {
    PaperRef::from_ids(
        PaperIds {
            arxiv: Some("2402.01234".into()),
            doi: Some("10.9999/synth.boundary.2024".into()),
            ..Default::default()
        },
        Some("v2".into()),
    )
    .unwrap()
}

fn sample_paper_okafor() -> PaperRef {
    PaperRef::from_ids(
        PaperIds {
            doi: Some("10.9999/synth.tail.2023".into()),
            ..Default::default()
        },
        None,
    )
    .unwrap()
}

fn build_hand_written_sample_doc(bib: Vec<BibEntry>, quote_ref: QuoteRef) -> Writeup {
    Writeup {
        mode: WriteupMode::Solve,
        title: "Synthetic Lattice Sum Identity".into(),
        status_line: "Done with open items: 1 step checked numerically".into(),
        final_effective_tier: Some(Tier::CheckedNumerically),
        coverage_paragraph: None,
        prose_edited_by_model: false,
        sections: vec![
            WriteupSection {
                title: "Assumptions and Sources".into(),
                label: Some("sec:setup".into()),
                blocks: vec![
                    WriteupBlock::Prose {
                        text: "User-supplied lattice conventions for the finite chain.".into(),
                        basis: ProseBasis::User,
                    },
                    WriteupBlock::Statement {
                        id: "e-1".into(),
                        text: "Let N >= 2 be an integer and let x > 0.".into(),
                        how_checked: HowChecked::Solve(Tier::Given),
                        own_tier: Some(Tier::Given),
                        rests_on: vec![],
                        note: None,
                    },
                    WriteupBlock::Quote(Box::new(WriteupQuote {
                        text: "The discrete boundary sum equals 7/12 for every even chain length."
                            .into(),
                        cite_key: "voss2024lattice".into(),
                        paper: sample_paper_voss(),
                        quote_ref,
                        locator: QuoteLocator::Page {
                            page: 2,
                            start_line: 4,
                            end_line: 4,
                        },
                        ocr: true,
                    })),
                    WriteupBlock::Statement {
                        id: "e-2".into(),
                        text: "Boundary sum lemma from the cited lattice note.".into(),
                        how_checked: HowChecked::Solve(Tier::Cited),
                        own_tier: Some(Tier::Cited),
                        rests_on: vec![],
                        note: Some("OCR source".into()),
                    },
                ],
            },
            WriteupSection {
                title: "Derivation".into(),
                label: Some("sec:derivation".into()),
                blocks: vec![
                    WriteupBlock::Prose {
                        text: "We expand the polynomial identity and test the tail bound.".into(),
                        basis: ProseBasis::Model,
                    },
                    WriteupBlock::Formula {
                        latex: "(x + 1)^2 - (x^2 + 2x + 1) = 0".into(),
                        handle: "poly_id@1".into(),
                        label: Some("eq:poly".into()),
                    },
                    WriteupBlock::Statement {
                        id: "e-3".into(),
                        text: "Symbolic expansion of the square vanishes identically.".into(),
                        how_checked: HowChecked::Solve(Tier::Verified),
                        own_tier: Some(Tier::Verified),
                        rests_on: vec!["e-1".into()],
                        note: None,
                    },
                    WriteupBlock::Formula {
                        latex: r"\left|\sum_{k=1}^{64} k^{-2} - \frac{\pi^2}{6}\right| < 0.016"
                            .into(),
                        handle: "tail_err@1".into(),
                        label: None,
                    },
                    WriteupBlock::Statement {
                        id: "e-4".into(),
                        text: "Partial sum matches the analytic limit at 12 points.".into(),
                        how_checked: HowChecked::Solve(Tier::CheckedNumerically),
                        own_tier: Some(Tier::CheckedNumerically),
                        rests_on: vec!["e-1".into(), "e-3".into()],
                        note: None,
                    },
                ],
            },
        ],
        bibliography: bib,
        check_table: vec![],
    }
}

#[test]
fn golden_outputs_and_tex_audit_validation() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ws_dir = tmp.path().join("ws");
    fs::create_dir_all(&ws_dir).unwrap();

    // Register Voss paper in F10 library with user's own citation key `voss2024lattice`.
    let lib = Library::open(&data_dir).unwrap();
    let voss = sample_paper_voss();
    let okafor = sample_paper_okafor();
    lib.upsert_paper(&PaperRecord {
        key: voss.key.clone(),
        ids: LibraryIds::from_paper_ids(&voss.ids),
        title: "Synthetic Boundary Sums on Finite Chains".into(),
        authors: vec!["Voss, Elena".into()],
        year: Some(2024),
        abstract_text: String::new(),
        bibtex: String::new(),
        flags: PaperFlags {
            owned: true,
            venue: Some("Journal of Synthetic Tests".into()),
            ..Default::default()
        },
        first_seen_ms: 100,
        last_seen_ms: 100,
    })
    .unwrap();
    lib.upsert_source(&SourceRecord {
        id: "user-vault".into(),
        kind: SourceKind::KandinskyVault,
        root: "/tmp/vault".into(),
        settings: json!({}),
        added_ms: 100,
        last_sync_ms: Some(100),
        state: "synced".into(),
    })
    .unwrap();
    lib.upsert_source_item(&SourceItemRecord {
        source_id: "user-vault".into(),
        external_id: "voss2024lattice".into(),
        key: Some(voss.key.clone()),
        citekey: Some("voss2024lattice".into()),
        ids: LibraryIds::from_paper_ids(&voss.ids),
        version: Some("v2".into()),
        title: "Synthetic Boundary Sums on Finite Chains".into(),
        authors: vec!["Voss, Elena".into()],
        year: Some(2024),
        item_kind: "journalArticle".into(),
        collections: vec![],
        tags: vec![],
        files: vec![],
        state: SourceItemState::Present,
        updated_ms: 100,
    })
    .unwrap();

    let mut used = BTreeSet::new();
    let bib_voss = resolve_bib_entry(Some(&lib), &voss, "", &[], None, &mut used).unwrap();
    let bib_okafor = resolve_bib_entry(
        Some(&lib),
        &okafor,
        "Discrete Tail Estimates",
        &["Okafor, K.".into()],
        Some(2023),
        &mut used,
    )
    .unwrap();
    assert_eq!(bib_voss.cite_key, "voss2024lattice");
    assert!(bib_okafor.cite_key.starts_with("okafor2023_"));

    let quote_text = "The discrete boundary sum equals 7/12 for every even chain length.";
    let qref = QuoteRef {
        source_id: "src-1".into(),
        path: "paper.tex".into(),
        start_line: 4,
        end_line: 4,
        file_sha256: "a".repeat(64),
        quote_sha256: quote_sha256(quote_text),
        evidence_seq: Some(1),
    };
    let doc = build_hand_written_sample_doc(vec![bib_voss, bib_okafor], qref);
    let opts = WriteupOptions {
        include_inline_badges: true,
        bib_stem: "writeup_sample".into(),
    };
    let tex = render_writeup_tex(&doc, &opts).unwrap();
    let md = render_writeup_markdown(&doc, &opts).unwrap();
    let bib = render_bibtex(&doc.bibliography);

    assert_eq!(bib, GOLDEN_BIB.replace("\r\n", "\n"), "BibTeX writer must match golden output");
    assert_eq!(md, GOLDEN_MD.replace("\r\n", "\n"), "Markdown writer must match golden output");
    assert_eq!(tex, GOLDEN_TEX.replace("\r\n", "\n"), "TeX writer must match golden output");

    // Validate TeX + BibTeX output with existing `tex_audit` checks (including quote matching)!
    fs::write(ws_dir.join("writeup_sample.tex"), &tex).unwrap();
    fs::write(ws_dir.join("writeup_sample.bib"), &bib).unwrap();
    let ledger = Mutex::new(Ledger::open(&data_dir).unwrap());
    let sandbox = WorkspaceSandbox::new(&data_dir, Some(ws_dir.clone()));
    ingest_bytes(
        &sandbox,
        &ledger,
        &IngestRequest {
            session_id: "s-audit",
            file_name: "voss2024lattice.tex",
            origin: "test:f15-golden",
            raw_bytes: SYNTH_PAPER_TEX,
            source_id_hint: Some("voss2024lattice.tex"),
            section_filter: None,
            title: Some("Synthetic Boundary Sums on Finite Chains"),
        },
    )
    .unwrap();
    let env = ResearchEnv::with_workspace(&data_dir, &ws_dir).unwrap();
    let ledger_guard = ledger.lock().unwrap();
    let audit =
        audit_tex_sources(&env, "s-audit", Some(&ledger_guard), &AuditOptions::default()).unwrap();
    assert!(audit.ok, "tex_audit failed on rendered write-up: {:?}", audit.issues);
    assert_eq!(audit.blocker_count, 0);
    assert_eq!(audit.error_count, 0);
    assert_eq!(audit.bib_keys_indexed, 2);
}

#[test]
fn walks_every_tier_and_every_writer_enforcing_word_honesty() {
    // Verify no non-verified constant in the wording table ever contains banned words.
    for label in all_non_verified_writeup_labels() {
        assert_eq!(
            contains_forbidden_word(label, false),
            None,
            "non-verified wording constant `{label}` must not contain verified/proved/new/novel"
        );
    }

    // 1. Solve Gate G7 refuses Failed and Stale across both writers.
    for bad_tier in [Tier::Failed, Tier::Stale] {
        let doc = Writeup {
            mode: WriteupMode::Solve,
            title: "Blocked Solve Derivation".into(),
            status_line: "Blocked".into(),
            final_effective_tier: Some(bad_tier),
            coverage_paragraph: None,
            prose_edited_by_model: false,
            sections: vec![WriteupSection {
                title: "Steps".into(),
                label: None,
                blocks: vec![WriteupBlock::Statement {
                    id: "e-bad".into(),
                    text: "Planted step.".into(),
                    how_checked: HowChecked::Solve(bad_tier),
                    own_tier: Some(bad_tier),
                    rests_on: vec![],
                    note: None,
                }],
            }],
            bibliography: vec![],
            check_table: vec![],
        };
        assert!(render_writeup_tex(&doc, &WriteupOptions::default()).is_err());
        assert!(render_writeup_markdown(&doc, &WriteupOptions::default()).is_err());
    }

    // 2. Walk every allowed Solve tier in both TeX and Markdown writers.
    let allowed_solve_tiers = [
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
    for tier in allowed_solve_tiers {
        let doc = Writeup {
            mode: WriteupMode::Solve,
            title: "Tier Walk".into(),
            status_line: format!("Final step tier: {}", tier.label()),
            final_effective_tier: Some(tier),
            coverage_paragraph: None,
            prose_edited_by_model: false,
            sections: vec![WriteupSection {
                title: "Step".into(),
                label: None,
                blocks: vec![WriteupBlock::Statement {
                    id: "e-1".into(),
                    text: "Synthetic step statement.".into(),
                    how_checked: HowChecked::Solve(tier),
                    own_tier: Some(tier),
                    rests_on: vec![],
                    note: None,
                }],
            }],
            bibliography: vec![],
            check_table: vec![],
        };
        let tex = render_writeup_tex(&doc, &WriteupOptions::default()).unwrap();
        let md = render_writeup_markdown(&doc, &WriteupOptions::default()).unwrap();
        for (writer_name, rendered) in [("TeX", &tex), ("Markdown", &md)] {
            assert!(
                rendered.contains(tier.label()),
                "{writer_name} missing label `{}` for {tier:?}",
                tier.label()
            );
            assert!(rendered.contains(writeup_wording::CHECK_TABLE_HEADING));
            let allow_verified = tier == Tier::Verified;
            assert_eq!(
                contains_forbidden_word(rendered, allow_verified),
                None,
                "{writer_name} emitted forbidden word for tier {tier:?}:\n{rendered}"
            );
        }
    }

    // 3. Solve status_line calling the result "verified" when final_effective_tier != Verified is refused.
    let overstated = Writeup {
        mode: WriteupMode::Solve,
        title: "Overstated Solve".into(),
        status_line: "Result is verified numerically".into(),
        final_effective_tier: Some(Tier::CheckedNumerically),
        coverage_paragraph: None,
        prose_edited_by_model: false,
        sections: vec![],
        bibliography: vec![],
        check_table: vec![CheckTableRow {
            id: "e-1".into(),
            statement: "Numeric check.".into(),
            how_checked: HowChecked::Solve(Tier::CheckedNumerically),
            own_tier: Some(Tier::CheckedNumerically),
            rests_on: vec![],
            note: None,
        }],
    };
    assert!(render_writeup_tex(&overstated, &WriteupOptions::default()).is_err());
    assert!(render_writeup_markdown(&overstated, &WriteupOptions::default()).is_err());

    // 4. Explore & Investigate modes NEVER contain "verified", "proved", "new", or "novel"
    //    across all non-Solve HowChecked variants and both writers.
    let non_solve = [
        HowChecked::CodeSkeleton,
        HowChecked::MatchedQuoteFullText,
        HowChecked::MatchedQuoteOcr,
        HowChecked::MatchedQuoteAbstract,
        HowChecked::ModelStatement,
        HowChecked::UserStatement,
    ];
    for mode in [WriteupMode::Explore, WriteupMode::Investigate] {
        for how in non_solve {
            let doc = Writeup {
                mode,
                title: "Mode Honesty Check".into(),
                status_line: "Completed summary".into(),
                final_effective_tier: None,
                coverage_paragraph: (mode == WriteupMode::Explore)
                    .then(|| "Searched arXiv and your library (12 papers).".into()),
                prose_edited_by_model: false,
                sections: vec![WriteupSection {
                    title: "Result".into(),
                    label: None,
                    blocks: vec![WriteupBlock::Statement {
                        id: "r-1".into(),
                        text: "Synthetic claim text.".into(),
                        how_checked: how,
                        own_tier: None,
                        rests_on: vec![],
                        note: None,
                    }],
                }],
                bibliography: vec![],
                check_table: vec![],
            };
            let tex = render_writeup_tex(&doc, &WriteupOptions::default()).unwrap();
            let md = render_writeup_markdown(&doc, &WriteupOptions::default()).unwrap();
            for rendered in [&tex, &md] {
                assert!(rendered.contains(how.label()));
                assert_eq!(contains_forbidden_word(rendered, false), None);
                if mode == WriteupMode::Explore {
                    assert!(rendered.contains(writeup_wording::COVERAGE_HEADING));
                }
            }
        }
    }
}

#[test]
fn quotes_pass_through_quote_core_and_unmatched_quotes_are_not_written() {
    let tmp = TempDir::new().unwrap();
    let ledger = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let sandbox = WorkspaceSandbox::new(tmp.path(), None);
    let session_id = "s-f15-quotes";

    let ingested = ingest_bytes(
        &sandbox,
        &ledger,
        &IngestRequest {
            session_id,
            file_name: "paper.tex",
            origin: "test:f15",
            raw_bytes: SYNTH_PAPER_TEX,
            source_id_hint: Some("paper.tex"),
            section_filter: None,
            title: Some("Synthetic Quote Source"),
        },
    )
    .unwrap();
    let events = ledger.lock().unwrap().read_session(session_id).unwrap();

    let good_quote = "The discrete boundary sum equals 7/12 for every even chain length.";
    let verified = verify_quote(
        &sandbox,
        session_id,
        &events,
        &ingested.source_id,
        &ingested.files[0],
        4,
        4,
        good_quote,
    )
    .unwrap();
    let vq = build_verified_quote(
        &sandbox,
        session_id,
        &events,
        "voss2024lattice".into(),
        sample_paper_voss(),
        &verified.quote_ref,
        good_quote,
        QuoteLocator::Lines {
            start_line: 4,
            end_line: 4,
        },
        false,
    )
    .unwrap();

    // Tampered / fabricated quote that does NOT match the ingested source.
    let bad_text = "Fabricated quote not present anywhere in the paper.";
    let bad_quote = WriteupQuote {
        text: bad_text.into(),
        cite_key: "voss2024lattice".into(),
        paper: sample_paper_voss(),
        quote_ref: QuoteRef {
            source_id: ingested.source_id.clone(),
            path: ingested.files[0].clone(),
            start_line: 4,
            end_line: 4,
            file_sha256: verified.quote_ref.file_sha256.clone(),
            quote_sha256: quote_sha256(bad_text),
            evidence_seq: None,
        },
        locator: QuoteLocator::Lines {
            start_line: 4,
            end_line: 4,
        },
        ocr: false,
    };

    let doc = Writeup {
        mode: WriteupMode::Investigate,
        title: "Quote Filtering Check".into(),
        status_line: "Reading notes".into(),
        final_effective_tier: None,
        coverage_paragraph: None,
        prose_edited_by_model: false,
        sections: vec![WriteupSection {
            title: "Main Result".into(),
            label: None,
            blocks: vec![
                WriteupBlock::Quote(Box::new(vq)),
                WriteupBlock::Quote(Box::new(bad_quote)),
            ],
        }],
        bibliography: vec![],
        check_table: vec![],
    };

    let (tex, rej_tex) =
        render_verified_writeup_tex(&doc, &sandbox, session_id, &events, &WriteupOptions::default())
            .unwrap();
    let (md, rej_md) = render_verified_writeup_markdown(
        &doc,
        &sandbox,
        session_id,
        &events,
        &WriteupOptions::default(),
    )
    .unwrap();

    assert_eq!(rej_tex.len(), 1);
    assert_eq!(rej_md.len(), 1);
    assert!(tex.contains(good_quote));
    assert!(md.contains(good_quote));
    assert!(!tex.contains(bad_text), "unmatched quote must never be written to TeX");
    assert!(!md.contains(bad_text), "unmatched quote must never be written to Markdown");
}

#[test]
fn turning_off_inline_badges_keeps_status_line_coverage_and_check_table() {
    let doc = Writeup {
        mode: WriteupMode::Explore,
        title: "Explore Summary".into(),
        status_line: "Completed with 2 services answered".into(),
        final_effective_tier: None,
        coverage_paragraph: Some("Searched INSPIRE-HEP, Crossref, and your library.".into()),
        prose_edited_by_model: true,
        sections: vec![WriteupSection {
            title: "Part 1".into(),
            label: None,
            blocks: vec![WriteupBlock::Statement {
                id: "sc-1".into(),
                text: "Boundary modes in finite chains.".into(),
                how_checked: HowChecked::MatchedQuoteFullText,
                own_tier: None,
                rests_on: vec![],
                note: None,
            }],
        }],
        bibliography: vec![],
        check_table: vec![],
    };
    let opts = WriteupOptions {
        include_inline_badges: false,
        bib_stem: "refs".into(),
    };
    let tex = render_writeup_tex(&doc, &opts).unwrap();
    let md = render_writeup_markdown(&doc, &opts).unwrap();
    for rendered in [&tex, &md] {
        assert!(!rendered.contains("[Matched quote (full text)]"));
        assert!(rendered.contains("Completed with 2 services answered"));
        assert!(rendered.contains(writeup_wording::PROSE_EDITED_BY_MODEL));
        assert!(rendered.contains(writeup_wording::COVERAGE_HEADING));
        assert!(rendered.contains(writeup_wording::CHECK_TABLE_HEADING));
        assert!(rendered.contains("Matched quote (full text)"));
    }
}
