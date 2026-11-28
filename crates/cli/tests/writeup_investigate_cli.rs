//! End-to-end CLI tests for `re-search writeup` on Investigate sessions (package F15).

use export::writeup::writeup_wording;
use ledger::Ledger;
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::investigation_markers::{
    EVENT_INVESTIGATION_NOTE, EVENT_INVESTIGATION_SKELETON,
};
use research_tools::library::{
    Library, LibraryIds, PaperFlags, PaperRecord, SourceItemRecord, SourceItemState, SourceKind,
    SourceRecord,
};
use research_tools::paper_key::{PaperIds, PaperKey};
use research_tools::skeleton::{build_tex_skeleton, PaperSkeleton, SkNodeKind};
use research_tools::tool_profile::MARKER_INVESTIGATION_STARTED;
use research_tools::{audit_tex_sources, AuditOptions, ResearchEnv};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

const CLI_SYNTH_TEX: &[u8] = br"\documentclass{article}
\newtheorem{theorem}{Theorem}[section]
\begin{document}
\section{Bounded Operators}\label{sec:ops}
\begin{theorem}[Spectral Bound]\label{thm:bound}
Every symmetric operator on a finite admissible lattice has real eigenvalues bounded in magnitude by two.
\end{theorem}
\end{document}
";

fn temp_case_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "research-cli-inv-writeup-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn seed_investigate_session(data_dir: &Path, ws_dir: &Path, sid: &str) {
    let ledger = Ledger::open(data_dir).unwrap();
    ledger
        .append_with_hash(sid, MARKER_INVESTIGATION_STARTED, 1, &json!({ "title": "Spectral Bound Notes" }))
        .unwrap();

    let paper_key = PaperKey::from_arxiv("2405.01111").unwrap();
    let lib = Library::open(data_dir).unwrap();
    lib.upsert_paper(&PaperRecord {
        key: paper_key.clone(),
        ids: LibraryIds::from_paper_ids(&PaperIds {
            arxiv: Some("2405.01111".into()),
            ..Default::default()
        }),
        title: "Spectral Bounds on Admissible Lattices".into(),
        authors: vec!["Keller, Nora".into()],
        year: Some(2024),
        abstract_text: "Synthetic abstract.".into(),
        bibtex: String::new(),
        flags: PaperFlags {
            owned: true,
            venue: Some("Synthetic Analysis Notes".into()),
            ..Default::default()
        },
        first_seen_ms: 1,
        last_seen_ms: 1,
    })
    .unwrap();
    lib.upsert_source(&SourceRecord {
        id: "cli-vault".into(),
        kind: SourceKind::KandinskyVault,
        root: "/tmp/synth-cli-vault".into(),
        settings: json!({}),
        added_ms: 1,
        last_sync_ms: Some(1),
        state: "synced".into(),
    })
    .unwrap();
    lib.upsert_source_item(&SourceItemRecord {
        source_id: "cli-vault".into(),
        external_id: "keller2024bound".into(),
        key: Some(paper_key.clone()),
        citekey: Some("keller2024bound".into()),
        ids: LibraryIds::from_paper_ids(&PaperIds {
            arxiv: Some("2405.01111".into()),
            ..Default::default()
        }),
        version: Some("v1".into()),
        title: "Spectral Bounds on Admissible Lattices".into(),
        authors: vec!["Keller, Nora".into()],
        year: Some(2024),
        item_kind: "journalArticle".into(),
        collections: vec![],
        tags: vec![],
        files: vec![],
        state: SourceItemState::Present,
        updated_ms: 1,
    })
    .unwrap();

    let env = ResearchEnv::with_workspace(data_dir, ws_dir).unwrap();
    let ledger_mtx = Mutex::new(Ledger::open_existing(data_dir).unwrap());
    let ing = ingest_bytes(
        &env.sandbox(),
        &ledger_mtx,
        &IngestRequest {
            session_id: sid,
            file_name: "bound.tex",
            origin: "https://arxiv.org/e-print/2405.01111v1",
            raw_bytes: CLI_SYNTH_TEX,
            source_id_hint: None,
            section_filter: None,
            title: Some("Spectral Bounds on Admissible Lattices"),
        },
    )
    .unwrap();
    drop(ledger_mtx);

    let primary_path = ing.files[0].clone();
    let mut files = std::collections::BTreeMap::new();
    files.insert(primary_path.clone(), String::from_utf8(CLI_SYNTH_TEX.to_vec()).unwrap());
    let mut hashes = std::collections::BTreeMap::new();
    hashes.insert(primary_path.clone(), ledger::sha256_hex(CLI_SYNTH_TEX));
    let skel: PaperSkeleton = build_tex_skeleton(&research_tools::skeleton::TexSkeletonInput {
        source_id: ing.source_id.clone(),
        source_seq: ing.event_seq,
        paper_key,
        version: Some("v1".into()),
        primary_path: Some(primary_path),
        files,
        files_sha256: Some(hashes),
    })
    .skeleton;
    let skel_bytes = serde_json::to_vec(&skel).unwrap();
    let blob = ledger.put_blob(&skel_bytes).unwrap();
    let thm_tag = skel
        .nodes
        .iter()
        .find(|n| n.kind == SkNodeKind::Theorem)
        .unwrap()
        .tag
        .clone()
        .unwrap();

    ledger
        .append_with_hash(
            sid,
            EVENT_INVESTIGATION_SKELETON,
            1,
            &json!({
                "source_id": ing.source_id,
                "paper_key": skel.paper_key,
                "version": skel.version,
                "builder": skel.builder.as_str(),
                "quality": skel.quality,
                "blob": blob,
            }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            EVENT_INVESTIGATION_NOTE,
            1,
            &json!({
                "tag": thm_tag,
                "pin": "main_result",
                "text": "Check whether non-symmetric perturbations break the bound of two.",
            }),
        )
        .unwrap();
}

#[test]
fn cli_writeup_supports_investigate_tex_md_bib_and_passes_tex_audit() {
    let dir = temp_case_dir("ok");
    let data_dir = dir.join("data");
    let ws_dir = dir.join("ws");
    fs::create_dir_all(&ws_dir).unwrap();
    let sid = "s-cli-inv-ok";
    seed_investigate_session(&data_dir, &ws_dir, sid);

    let bin = env!("CARGO_BIN_EXE_re-search");
    let tex_out = ws_dir.join("reading_notes.tex");
    let run_tex = Command::new(bin)
        .args([
            "writeup",
            sid,
            "--format",
            "tex",
            "--out",
            tex_out.to_str().unwrap(),
            "--data-dir",
            data_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        run_tex.status.success(),
        "cli writeup tex failed: {}",
        String::from_utf8_lossy(&run_tex.stderr)
    );
    let tex = fs::read_to_string(&tex_out).unwrap();
    let bib = fs::read_to_string(ws_dir.join("reading_notes.bib")).unwrap();
    assert!(tex.contains("\\bibliography{reading_notes}"));
    assert!(tex.contains("\\cite{keller2024bound}"));
    assert!(tex.contains(writeup_wording::CHECK_TABLE_HEADING));
    assert!(!tex.to_ascii_lowercase().contains("verified"));
    assert!(bib.contains("@article{keller2024bound,"));

    let env = ResearchEnv::with_workspace(&data_dir, &ws_dir).unwrap();
    let ledger = Ledger::open_existing(&data_dir).unwrap();
    let audit = audit_tex_sources(&env, sid, Some(&ledger), &AuditOptions::default()).unwrap();
    assert!(audit.ok, "tex_audit failed on CLI Investigate write-up: {:?}", audit.issues);
    assert_eq!(audit.blocker_count, 0);
    assert_eq!(audit.error_count, 0);

    let md_out = dir.join("reading_notes.md");
    let run_md = Command::new(bin)
        .args([
            "writeup",
            sid,
            "--format",
            "md",
            "--no-inline-marks",
            "--out",
            md_out.to_str().unwrap(),
            "--data-dir",
            data_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        run_md.status.success(),
        "cli writeup md failed: {}",
        String::from_utf8_lossy(&run_md.stderr)
    );
    let md = fs::read_to_string(&md_out).unwrap();
    assert!(md.contains("tex-1, full"));
    assert!(md.contains("pinned as main result by you"));
    assert!(md.contains("Check whether non-symmetric perturbations break the bound of two."));
    assert!(md.contains(writeup_wording::CHECK_TABLE_HEADING));
    assert!(!md.to_ascii_lowercase().contains("verified"));

    let bib_out = dir.join("refs_only.bib");
    let run_bib = Command::new(bin)
        .args([
            "writeup",
            sid,
            "--format",
            "bib",
            "--out",
            bib_out.to_str().unwrap(),
            "--data-dir",
            data_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(run_bib.status.success());
    assert!(fs::read_to_string(&bib_out).unwrap().contains("@article{keller2024bound,"));

    let _ = fs::remove_dir_all(&dir);
}
