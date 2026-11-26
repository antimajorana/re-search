//! Integration and property tests for package F16a (corrections reused by code, evaluation set K,
//! and the invariant "a lesson never decides anything" from `00-overview.md` 2.14 & 7.3).
//!
//! No model is involved anywhere in F16a.

use ledger::Ledger;
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::lessons::{
    ConventionDefault, LessonsStore, OcrOverlay, SkeletonOverride, SymbolDefault,
    TermAliasDecision, TermAliasRecord, EVENT_IDEA_ALIAS_CONFIRMED, EVENT_IDEA_ALIAS_REJECTED,
    EVENT_INVESTIGATION_NODE_OVERRIDE, EVENT_OCR_LINE_CORRECTED, EVENT_SOLVE_SETUP_REVISED,
    OCR_OVERLAY_LABEL,
};
use research_tools::lessons_select::{
    apply_mark_ranking, apply_ocr_overlays, apply_skeleton_overrides, expand_query_aliases,
    is_pair_rejected, matching_key_paper_seeds, suggest_solve_defaults,
};
use research_tools::library::{Library, LibraryIds, MarkStatus, PaperFlags, PaperRecord};
use research_tools::paper_key::PaperKey;
use research_tools::paper_sources::sha256_hex;
use research_tools::quote_core::verify_quote;
use research_tools::sandbox::WorkspaceSandbox;
use research_tools::skeleton::{build_tex_skeleton, TexSkeletonInput};
use research_tools::solve_model::event_kinds;
use research_tools::solve_state::SolveState;
use research_tools::solve_status::{badge_for_step, compute_all_tiers};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Mutex;
use tempfile::TempDir;

const SYNTH_TEX: &str = r"\documentclass{article}
\newtheorem{theorem}{Theorem}
\begin{document}
\section{Model}
\begin{theorem}\label{thm:sum}
The synthetic spectral sum equals 7/12 for all finite lattices.
\end{theorem}
\end{document}
";

fn seed_solve_session(ledger: &Ledger, session_id: &str) {
    ledger
        .append(session_id, event_kinds::SOLVE_STARTED, 1, &json!({}))
        .unwrap();
    ledger
        .append(
            session_id,
            event_kinds::PROOF_ENTRY,
            1,
            &json!({
                "entry_id": "e-1",
                "type": "assumption",
                "origin": "user",
                "statement": "Let m > 0 be a real mass parameter.",
                "depends_on": []
            }),
        )
        .unwrap();
    ledger
        .append(
            session_id,
            event_kinds::PROOF_ENTRY,
            1,
            &json!({
                "entry_id": "e-2",
                "type": "derivation_step",
                "origin": "model",
                "statement": "Compute exact identity (x + 1)^2 = x^2 + 2x + 1.",
                "depends_on": ["e-1"],
                "claims": [{
                    "id": "c-1",
                    "kind": "identity",
                    "lhs": "h1@1",
                    "rhs": "h2@1"
                }]
            }),
        )
        .unwrap();
    ledger
        .append(
            session_id,
            event_kinds::SOLVE_CHECK,
            1,
            &json!({
                "check_id": "chk-1",
                "entry_id": "e-2",
                "claim_id": "c-1",
                "kind": "identity_symbolic",
                "passed": true,
                "receipt_id": "rcpt-1"
            }),
        )
        .unwrap();
    ledger
        .append(
            session_id,
            event_kinds::PROOF_ENTRY,
            1,
            &json!({
                "entry_id": "e-3",
                "type": "derivation_step",
                "origin": "model",
                "statement": "Numerically check integral at 12 points.",
                "depends_on": ["e-2"],
                "claims": [{
                    "id": "c-2",
                    "kind": "identity",
                    "lhs": "h3@1",
                    "rhs": "h4@1"
                }]
            }),
        )
        .unwrap();
    ledger
        .append(
            session_id,
            event_kinds::SOLVE_CHECK,
            1,
            &json!({
                "check_id": "chk-2",
                "entry_id": "e-3",
                "claim_id": "c-2",
                "kind": "identity_numeric",
                "passed": true,
                "receipt_id": "rcpt-2"
            }),
        )
        .unwrap();
    ledger
        .append(
            session_id,
            event_kinds::PROOF_ENTRY,
            1,
            &json!({
                "entry_id": "e-4",
                "type": "derivation_step",
                "origin": "model",
                "statement": "Unchecked model prose claim.",
                "depends_on": ["e-3"]
            }),
        )
        .unwrap();
}

/// Property test required by `00-overview.md` 2.14 & 7.3:
/// For the exact same ledger events, WITH and WITHOUT `lessons.db` (populated with aliases,
/// symbol/convention defaults, skeleton overrides, OCR overlays, and library marks), every
/// Solve tier, badge, quote check, and pure skeleton is bit-for-bit identical. Only user-visible
/// rankings, alias expansions, setup defaults, and display overlays change.
#[test]
fn property_lessons_never_change_any_tier_badge_quote_or_pure_skeleton() {
    let tmp_without = TempDir::new().unwrap();
    let tmp_with = TempDir::new().unwrap();
    let session_id = "s-f16a-prop";

    // Build identical ledgers and ingested sources in both data dirs.
    let mut snapshots = Vec::new();
    for (idx, root) in [tmp_without.path(), tmp_with.path()].iter().enumerate() {
        let ledger = Mutex::new(Ledger::open(root).unwrap());
        let sandbox = WorkspaceSandbox::new(root, None);
        let ingested = ingest_bytes(
            &sandbox,
            &ledger,
            &IngestRequest {
                session_id,
                file_name: "paper.tex",
                origin: "test:f16a",
                raw_bytes: SYNTH_TEX.as_bytes(),
                source_id_hint: Some("paper.tex"),
                section_filter: None,
                title: Some("Synthetic F16a Paper"),
            },
        )
        .unwrap();
        seed_solve_session(&ledger.lock().unwrap(), session_id);

        if idx == 1 {
            // Populate lessons.db and F10 library marks heavily in `tmp_with`.
            let store = LessonsStore::open(root).unwrap();
            store
                .record_term_alias(&TermAliasRecord {
                    term_a: "spectral sum".into(),
                    term_b: "trace identity".into(),
                    decision: TermAliasDecision::Confirmed,
                    origin_session: "s-prev".into(),
                    origin_node: None,
                    updated_ms: 1000,
                })
                .unwrap();
            store
                .record_symbol_default(&SymbolDefault {
                    symbol: "m".into(),
                    domain: "qft".into(),
                    assumptions: vec!["real".into(), "positive".into()],
                    dimensions: Some("mass".into()),
                    convention_note: Some("rest mass".into()),
                    origin_session: "s-prev".into(),
                    updated_ms: 1000,
                })
                .unwrap();
            store
                .record_convention_default(&ConventionDefault {
                    key: "metric_signature".into(),
                    value: "(-,+,+,+)".into(),
                    domain: "qft".into(),
                    origin_session: "s-prev".into(),
                    updated_ms: 1000,
                })
                .unwrap();
            store
                .record_skeleton_override(&SkeletonOverride {
                    node_tag: "n-override01".into(),
                    paper_key: Some(PaperKey::parse("arxiv:2401.00001").unwrap()),
                    kind_override: Some("proposition".into()),
                    end_line_override: Some(7),
                    note: "reader fixed kind".into(),
                    origin_session: "s-prev".into(),
                    updated_ms: 1000,
                })
                .unwrap();
            let file_sha = sha256_hex(SYNTH_TEX.as_bytes());
            store
                .record_ocr_overlay(&OcrOverlay {
                    file_sha256: file_sha,
                    line_no: 6,
                    corrected_text: "Edited display overlay text.".into(),
                    original_sha256: None,
                    origin_session: "s-prev".into(),
                    updated_ms: 1000,
                    label: OCR_OVERLAY_LABEL.to_string(),
                })
                .unwrap();
        }

        let events = ledger.lock().unwrap().read_session(session_id).unwrap();
        let solve_state = SolveState::from_events(&events);
        let tiers = compute_all_tiers(&solve_state);
        let badges: BTreeMap<String, _> = solve_state
            .entries
            .iter()
            .filter_map(|e| badge_for_step(&solve_state, &e.entry_id).map(|b| (e.entry_id.clone(), b)))
            .collect();

        let quote = verify_quote(
            &sandbox,
            session_id,
            &events,
            &ingested.source_id,
            &ingested.files[0],
            6,
            6,
            "The synthetic spectral sum equals 7/12 for all finite lattices.",
        )
        .unwrap();

        let mut files = BTreeMap::new();
        files.insert("paper.tex".to_string(), SYNTH_TEX.to_string());
        let skel = build_tex_skeleton(&TexSkeletonInput {
            source_id: ingested.source_id.clone(),
            source_seq: 1,
            paper_key: PaperKey::parse("arxiv:2401.00001").unwrap(),
            version: Some("v1".into()),
            files,
            primary_path: Some("paper.tex".into()),
            files_sha256: None,
        });

        snapshots.push((tiers, badges, quote.quote_ref, quote.excerpt, skel.skeleton));
    }

    // Without lessons == With lessons on EVERY check decision.
    assert_eq!(snapshots[0].0, snapshots[1].0, "Solve tiers must be identical");
    assert_eq!(snapshots[0].1, snapshots[1].1, "Solve badges must be identical");
    assert_eq!(snapshots[0].2, snapshots[1].2, "QuoteRef must be identical");
    assert_eq!(snapshots[0].3, snapshots[1].3, "Quote excerpt must be identical");
    assert_eq!(snapshots[0].4, snapshots[1].4, "Pure skeleton must be identical");

    // Meanwhile, the user-visible defaults/expansions/overlays DO reflect lessons.db.
    let store = LessonsStore::open(tmp_with.path()).unwrap();
    let expanded = expand_query_aliases(&store, &["spectral sum"]).unwrap();
    assert_eq!(expanded, vec!["spectral sum", "trace identity"]);
    let defaults = suggest_solve_defaults(&store, &["m"], Some("qft")).unwrap();
    assert_eq!(defaults.symbols.len(), 1);
    assert_eq!(defaults.conventions.len(), 1);
}

/// Evaluation set K scripted pairs for F16a:
/// 1. Confirmed other name -> expanded in later queries
/// 2. Rejected pair -> suppressed from being proposed again
/// 3. Irrelevant paper -> demoted but still listed; Key paper -> boosted + offered as seed
/// 4. Solve setup convention + symbol assumptions -> offered as default when symbols appear
/// 5. Skeleton override -> applied to display overlay, pure skeleton unchanged, deletable
/// 6. OCR line correction -> shown as "corrected by you" overlay, pinned text unchanged
#[test]
fn eval_set_k_f16a_scripted_session_pairs_and_backup_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let ledger = Ledger::open(tmp.path()).unwrap();
    let lib = Library::open(tmp.path()).unwrap();
    let mut store = LessonsStore::open(tmp.path()).unwrap();

    // Session 1 emits existing correction events (F16a adds no new way to correct).
    let s1 = "s-eval-k-1";
    ledger
        .append(
            s1,
            EVENT_IDEA_ALIAS_CONFIRMED,
            1,
            &json!({ "term_a": "Chern Number", "term_b": "TKNN invariant" }),
        )
        .unwrap();
    ledger
        .append(
            s1,
            EVENT_IDEA_ALIAS_REJECTED,
            1,
            &json!({ "term_a": "berry phase", "term_b": "hall conductance" }),
        )
        .unwrap();
    ledger
        .append(
            s1,
            EVENT_SOLVE_SETUP_REVISED,
            1,
            &json!({
                "domain": "condensed_matter",
                "conventions": {
                    "units": "hbar = 1",
                    "fourier_sign": "exp(-i k x)"
                },
                "symbols": [
                    { "name": "k", "assumptions": ["real"], "dimensions": "1/length" },
                    { "name": "Delta", "assumptions": ["real", "positive"], "dimensions": "energy" }
                ]
            }),
        )
        .unwrap();
    ledger
        .append(
            s1,
            EVENT_INVESTIGATION_NODE_OVERRIDE,
            1,
            &json!({
                "node_tag": "n-1122334455",
                "paper_key": "arxiv:2401.12345",
                "kind_override": "theorem",
                "end_line_override": 95,
                "note": "proof ends at line 95"
            }),
        )
        .unwrap();
    let ocr_sha = "c".repeat(64);
    ledger
        .append(
            s1,
            EVENT_OCR_LINE_CORRECTED,
            1,
            &json!({
                "file_sha256": ocr_sha,
                "line_no": 2,
                "corrected_text": "H = -t sum c_i^dag c_j"
            }),
        )
        .unwrap();

    let events = ledger.read_session(s1).unwrap();
    let r1 = store.ingest_session_events(s1, &events).unwrap();
    assert_eq!(r1.term_aliases_recorded, 2);
    assert_eq!(r1.symbol_defaults_recorded, 2);
    assert_eq!(r1.convention_defaults_recorded, 2);
    assert_eq!(r1.skeleton_overrides_recorded, 1);
    assert_eq!(r1.ocr_overlays_recorded, 1);

    // Re-ingesting the same session is a no-op via cursor.
    let r2 = store.ingest_session_events(s1, &events).unwrap();
    assert_eq!(r2.events_scanned, 0);

    // Session 2: confirmed alias expands query; rejected pair is suppressed.
    let expanded = expand_query_aliases(&store, &["chern number"]).unwrap();
    assert_eq!(expanded, vec!["chern number", "tknn invariant"]);
    assert!(is_pair_rejected(&store, "Hall Conductance", "Berry Phase").unwrap());
    assert!(!is_pair_rejected(&store, "chern number", "tknn invariant").unwrap());

    // Session 2: Solve defaults suggested for matching symbols.
    let suggested = suggest_solve_defaults(&store, &["Delta", "x"], Some("condensed_matter")).unwrap();
    assert_eq!(suggested.symbols.len(), 1);
    assert_eq!(suggested.symbols[0].symbol, "Delta");
    assert_eq!(suggested.symbols[0].dimensions.as_deref(), Some("energy"));
    assert_eq!(suggested.conventions.len(), 2);

    // Session 2: Library user marks demote irrelevant (still listed!) and boost/seed key papers.
    let k_irrel = PaperKey::parse("arxiv:2401.00010").unwrap();
    let k_plain = PaperKey::parse("arxiv:2401.00020").unwrap();
    let k_key = PaperKey::parse("arxiv:2401.00030").unwrap();
    for (k, title) in [
        (&k_irrel, "Unrelated Optics Note"),
        (&k_plain, "Chern Number Review"),
        (&k_key, "Exact Chern Number on Honeycomb Lattice"),
    ] {
        lib.upsert_paper(&PaperRecord {
            key: k.clone(),
            ids: LibraryIds::default(),
            title: title.into(),
            authors: vec!["Synthetic Author".into()],
            year: Some(2024),
            abstract_text: String::new(),
            bibtex: String::new(),
            flags: PaperFlags::default(),
            first_seen_ms: 100,
            last_seen_ms: 100,
        })
        .unwrap();
    }
    lib.set_user_mark(&k_irrel, MarkStatus::Irrelevant, vec![], "off topic".into())
        .unwrap();
    lib.set_user_mark(&k_key, MarkStatus::Key, vec!["topology".into()], String::new())
        .unwrap();

    let marks = lib.list_user_marks().unwrap();
    let ranked = apply_mark_ranking(
        &[
            (k_irrel.clone(), 900),
            (k_plain.clone(), 500),
            (k_key.clone(), 400),
        ],
        &marks,
    );
    assert_eq!(ranked.len(), 3, "irrelevant paper stays listed");
    assert_eq!(ranked[0].key, k_key);
    assert!(ranked[0].boosted);
    assert_eq!(ranked[1].key, k_plain);
    assert_eq!(ranked[2].key, k_irrel);
    assert!(ranked[2].demoted);

    let seeds = matching_key_paper_seeds(&lib, &["chern number"], 5).unwrap();
    assert_eq!(seeds.len(), 1);
    assert_eq!(seeds[0].paper.key, k_key);

    // Session 2: Skeleton display override without mutating pure skeleton.
    let display_nodes = apply_skeleton_overrides(
        &store,
        &[("n-1122334455".into(), "lemma".into(), 80)],
    )
    .unwrap();
    assert_eq!(display_nodes[0].pure_kind, "lemma");
    assert_eq!(display_nodes[0].effective_kind, "theorem");
    assert_eq!(display_nodes[0].pure_end_line, 80);
    assert_eq!(display_nodes[0].effective_end_line, 95);
    assert!(display_nodes[0].overridden_by_user);

    // Session 2: OCR display overlay tagged "corrected by you", pinned text unchanged.
    let pinned = ["Line 1 raw", "H = -1 sum c_i c_j", "Line 3 raw"];
    let ocr_view = apply_ocr_overlays(&store, &ocr_sha, &pinned).unwrap();
    assert!(!ocr_view[0].corrected_by_you);
    assert!(ocr_view[1].corrected_by_you);
    assert_eq!(ocr_view[1].pinned_text, "H = -1 sum c_i c_j");
    assert_eq!(ocr_view[1].display_text, "H = -t sum c_i^dag c_j");
    assert_eq!(ocr_view[1].label.as_deref(), Some(OCR_OVERLAY_LABEL));

    // Backup & restore roundtrip + tamper rejection.
    let backup_bytes = store.export_backup_bytes().unwrap();
    let mut tampered = backup_bytes.clone();
    if let Some(pos) = tampered.windows(5).position(|w| w == b"Delta") {
        tampered[pos] = b'X';
    }
    assert!(store.restore_backup_bytes(&tampered).is_err());

    assert!(store.delete_skeleton_override("n-1122334455").unwrap());
    assert!(store.list_skeleton_overrides().unwrap().is_empty());
    let restored = store.restore_backup_bytes(&backup_bytes).unwrap();
    assert_eq!(restored, 8);
    assert_eq!(store.list_skeleton_overrides().unwrap().len(), 1);
}
