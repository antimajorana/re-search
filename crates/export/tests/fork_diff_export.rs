//! Tests-first integration suite for verified session forking (F14),
//! structural branch diff (F14), and publication export to `.tex`+`.bib`,
//! standalone `.html`, and reproducible `.zip` (F15).

use export::{
    compute_branch_diff, export_session_bytes, export_session_to_path, fork_session, ExportFormat,
    ForkOptions,
};
use ledger::Ledger;
use serde_json::{json, Value};
use std::io::Read;
use std::path::PathBuf;
use std::process::Command;

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("export-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Self { path: p }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn seed_parent_session(dir: &std::path::Path, session: &str) -> u64 {
    let ledger = Ledger::open(dir).unwrap();
    ledger
        .append_with_hash(
            session,
            "session.start",
            1,
            &json!({
                "route": "api",
                "provider": "anthropic-messages",
                "model": "claude-haiku-4-5",
                "policy_version": "v0.3-research",
                "prompt": "Derive energy spectrum with α, ℏ, and ⊗."
            }),
        )
        .unwrap();

    ledger
        .append_with_hash(
            session,
            "proof.entry",
            1,
            &json!({
                "entry_id": "e-1",
                "entry_type": "assumption",
                "summary": "Coupling α and ℏ",
                "details": "Assume Hamiltonian H = α ℏ (σ_x ⊗ σ_z).",
                "latex": "H = α ℏ (\\sigma_x ⊗ \\sigma_z)",
                "depends_on": []
            }),
        )
        .unwrap();

    let args_json = json!({ "code": "import math\nx = 3\nassert math.isclose(x**2 - 9, 0.0)\nprint('check ok')" }).to_string();
    ledger
        .append_with_hash(
            session,
            "tool.request",
            1,
            &json!({
                "call_id": "c-sym-1",
                "caller": "agent:anthropic:s-1",
                "tool": "sympy_eval",
                "args": { "inline": args_json }
            }),
        )
        .unwrap();
    let res_bytes = br#"{"value":"0","stdout":"0\n"}"#;
    let res_blob = ledger.put_blob(res_bytes).unwrap();
    ledger
        .append_with_hash(
            session,
            "tool.result",
            1,
            &json!({
                "call_id": "c-sym-1",
                "ok": true,
                "result": { "blob": res_blob, "bytes": res_bytes.len() }
            }),
        )
        .unwrap();

    let lit_blob = ledger
        .put_blob(
            json!({
                "bibtex": "@article{Kitaev:2001,\n  author = \"Kitaev, A. Yu.\",\n  title = \"Unpaired Majorana fermions\",\n  year = \"2001\"\n}\n"
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
    ledger
        .append_with_hash(session, "literature.fetched", 1, &json!({ "raw_blob": lit_blob }))
        .unwrap();

    let fork_anchor_seq = ledger
        .append_with_hash(
            session,
            "proof.entry",
            1,
            &json!({
                "entry_id": "e-2",
                "entry_type": "derivation",
                "summary": "Compute trace",
                "details": "Eigenvalues follow from Pauli tensor algebra.",
                "latex": "\\mathrm{Tr}(H^2) = 4 \\alpha^2 \\hbar^2",
                "depends_on": ["e-1"]
            }),
        )
        .unwrap()
        .seq;

    ledger
        .append_with_hash(
            session,
            "proof.entry",
            1,
            &json!({
                "entry_id": "e-3",
                "entry_type": "final_answer",
                "summary": "Base spectrum",
                "details": "E = ± α ℏ",
                "latex": "E = \\pm \\alpha \\hbar",
                "depends_on": ["e-2"]
            }),
        )
        .unwrap();

    // Seed ingested source file and an isolated sidetalk event (must NOT be copied by fork).
    let src_dir = dir.join("sessions").join(session).join("sources").join("paper-1");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(src_dir.join("notes.tex"), b"\\section{Background}\n\\label{sec:bg}\n").unwrap();
    std::fs::write(
        src_dir.join("refs.bib"),
        b"@article{Haldane:1988,\n  author = \"Haldane, F. D. M.\",\n  title = \"Quantum Hall Effect\",\n  year = \"1988\"\n}\n",
    )
    .unwrap();

    ledger
        .append_with_hash(
            &format!("sidetalk:{session}"),
            "sidetalk.turn",
            1,
            &json!({ "q": "private sidetalk", "a": "never forked" }),
        )
        .unwrap();

    fork_anchor_seq
}

#[test]
fn fork_and_branch_diff_preserve_prefix_copy_sources_exclude_sidetalk_and_reject_invalid() {
    let tmp = TempDir::new("fork-diff");
    let parent = "s-parent01";
    let anchor_seq = seed_parent_session(&tmp.path, parent);

    // Invalid fork requests fail closed.
    assert!(fork_session(
        &tmp.path,
        &ForkOptions { parent_session: parent.into(), at_seq: 0, modified_assumption: None },
    )
    .is_err());
    assert!(fork_session(
        &tmp.path,
        &ForkOptions { parent_session: parent.into(), at_seq: 999, modified_assumption: None },
    )
    .is_err());
    assert!(fork_session(
        &tmp.path,
        &ForkOptions {
            parent_session: parent.into(),
            at_seq: anchor_seq,
            modified_assumption: Some("x".repeat(4001)),
        },
    )
    .is_err());
    assert!(fork_session(
        &tmp.path,
        &ForkOptions { parent_session: format!("sidetalk:{parent}"), at_seq: 1, modified_assumption: None },
    )
    .is_err());

    let receipt = fork_session(
        &tmp.path,
        &ForkOptions {
            parent_session: parent.into(),
            at_seq: anchor_seq,
            modified_assumption: Some("Assume modified coupling α' = 2α.".into()),
        },
    )
    .unwrap();
    assert!(receipt.new_session.starts_with("s-") && receipt.new_session.len() == 18);
    assert_eq!(receipt.parent_session, parent);
    assert_eq!(receipt.parent_seq, anchor_seq);
    assert_eq!(receipt.copied_events, anchor_seq as usize);

    let ledger = Ledger::open_existing(&tmp.path).unwrap();
    let new_evs = ledger.read_session(&receipt.new_session).unwrap();
    assert!(new_evs.iter().any(|e| e.kind == "session.fork" && e.payload["modified_assumption"] == "Assume modified coupling α' = 2α."));
    assert!(new_evs.iter().any(|e| e.kind == "proof.entry" && e.payload["entry_type"] == "assumption" && e.payload["details"] == "Assume modified coupling α' = 2α."));
    assert!(tmp.path.join("sessions").join(&receipt.new_session).join("sources/paper-1/notes.tex").is_file());
    assert!(ledger.read_session(&format!("sidetalk:{}", receipt.new_session)).unwrap().is_empty());

    // Add divergent final_answer on the forked branch.
    ledger
        .append_with_hash(
            &receipt.new_session,
            "proof.entry",
            1,
            &json!({
                "entry_id": "e-4",
                "entry_type": "final_answer",
                "summary": "Modified spectrum",
                "details": "E = ± 2 α ℏ",
                "latex": "E = \\pm 2 \\alpha \\hbar",
                "depends_on": ["e-2"]
            }),
        )
        .unwrap();

    let diff = compute_branch_diff(&ledger, parent, &receipt.new_session).unwrap();
    assert_eq!(diff.shared_prefix_len, anchor_seq as usize);
    assert_eq!(diff.divergence_seq, Some(anchor_seq + 1));
    assert_eq!(diff.base_final_answer.as_deref(), Some("E = ± α ℏ"));
    assert_eq!(diff.target_final_answer.as_deref(), Some("E = ± 2 α ℏ"));
    assert!(diff.summary_diff.contains("E = ± α ℏ") && diff.summary_diff.contains("E = ± 2 α ℏ"));
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
}

#[test]
fn export_tex_html_and_reproducible_zip_and_replay_checks() {
    let tmp = TempDir::new("export-formats");
    let session = "s-export01";
    seed_parent_session(&tmp.path, session);

    assert_eq!(ExportFormat::parse("tex").unwrap(), ExportFormat::Tex);
    assert_eq!(ExportFormat::parse("html").unwrap(), ExportFormat::Html);
    assert_eq!(ExportFormat::parse("zip").unwrap(), ExportFormat::Zip);
    assert!(ExportFormat::parse("docx").is_err());

    // 1. LaTeX + sidecar .bib
    let tex_out = tmp.path.join("out").join("manuscript.tex");
    let tex_bytes = export_session_to_path(&tmp.path, session, ExportFormat::Tex, &tex_out).unwrap();
    let tex_str = String::from_utf8(tex_bytes).unwrap();
    assert!(tex_str.contains(r"\documentclass[11pt]{article}"));
    assert!(tex_str.contains("amsmath") && tex_str.contains("amsthm") && tex_str.contains("hyperref"));
    assert!(tex_str.contains(r"\alpha") && tex_str.contains(r"\hbar") && tex_str.contains(r"\otimes"));
    assert!(tex_str.contains(r"\begin{assumption}") && tex_str.contains(r"\begin{verbatim}"));
    let sidecar_bib = tex_out.with_extension("bib");
    assert!(sidecar_bib.is_file(), "sidecar .bib must be written alongside .tex");
    let bib_txt = std::fs::read_to_string(&sidecar_bib).unwrap();
    assert!(bib_txt.contains("Kitaev:2001") && bib_txt.contains("Haldane:1988"));

    // 2. Standalone HTML
    let (html_name, html_bytes) = export_session_bytes(&tmp.path, session, ExportFormat::Html).unwrap();
    assert!(html_name.ends_with(".html"));
    let html_str = String::from_utf8(html_bytes).unwrap();
    assert!(html_str.contains("<!DOCTYPE html>") && html_str.contains(session));
    assert!(!html_str.contains("https://cdn") && !html_str.contains("http://cdn"));
    let head_hash = Ledger::open_existing(&tmp.path).unwrap().verify_session(session).unwrap().head_hash;
    assert!(html_str.contains(&head_hash));

    // 3. Reproducible ZIP
    let (zip_name1, zip_bytes1) = export_session_bytes(&tmp.path, session, ExportFormat::Zip).unwrap();
    let (_, zip_bytes2) = export_session_bytes(&tmp.path, session, ExportFormat::Zip).unwrap();
    assert!(zip_name1.ends_with(".zip"));
    assert_eq!(zip_bytes1, zip_bytes2, "zip export must be byte-for-byte reproducible");

    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&zip_bytes1)).unwrap();
    let names: Vec<String> = (0..archive.len())
        .map(|i| archive.by_index(i).unwrap().name().unwrap().into_owned())
        .collect();
    for req in [
        "report.tex",
        "references.bib",
        "report.html",
        "ledger.json",
        "run_all_checks.py",
        "REPRODUCE.md",
        "sources/paper-1/notes.tex",
    ] {
        assert!(names.contains(&req.to_string()), "missing `{req}` in zip: {names:?}");
    }
    let step_py = names.iter().find(|n| n.starts_with("code/step_") && n.ends_with(".py")).cloned().expect("code/step_*.py");

    let extract_dir = tmp.path.join("unzipped");
    std::fs::create_dir_all(extract_dir.join("code")).unwrap();
    for target in ["ledger.json", "run_all_checks.py", "REPRODUCE.md", &step_py] {
        let mut f = archive.by_name(target).unwrap();
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).unwrap();
        let dest = extract_dir.join(target);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, buf).unwrap();
    }
    let ledger_doc: Value = serde_json::from_slice(&std::fs::read(extract_dir.join("ledger.json")).unwrap()).unwrap();
    assert_eq!(ledger_doc["head_hash"], head_hash);

    // Run extracted run_all_checks.py if python3 is installed.
    if let Ok(out) = Command::new("python3").arg("run_all_checks.py").current_dir(&extract_dir).output() {
        assert!(
            out.status.success(),
            "run_all_checks.py failed: stdout={}, stderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
