use export::{compute_branch_diff, export_session_bytes, ExportFormat};
use ledger::Ledger;
use serde_json::{json, Value};
use std::io::{Cursor, Read};
use std::process::Command;
use std::sync::Barrier;
use std::time::Duration;

#[test]
fn zip_head_matches_events_during_concurrent_appends() {
    let temp = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(temp.path()).unwrap();
    ledger
        .append("s-audit", "session.start", 1, &json!({}))
        .unwrap();
    let start = Barrier::new(2);

    std::thread::scope(|scope| {
        scope.spawn(|| {
            let writer = Ledger::open_existing(temp.path()).unwrap();
            start.wait();
            for _ in 0..80 {
                writer
                    .append("s-audit", "model.text", 1, &json!({"text":"working"}))
                    .unwrap();
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        start.wait();
        for _ in 0..24 {
            let (_, bytes) =
                export_session_bytes(temp.path(), "s-audit", ExportFormat::Zip).unwrap();
            let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
            let mut raw = Vec::new();
            archive
                .by_name("ledger.json")
                .unwrap()
                .read_to_end(&mut raw)
                .unwrap();
            let audit: Value = serde_json::from_slice(&raw).unwrap();
            let last = audit["events"].as_array().unwrap().last().unwrap();
            assert_eq!(audit["head_hash"], last["event_hash"]);
        }
    });
}

#[test]
fn structural_diff_reports_a_superseded_derivation() {
    let temp = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(temp.path()).unwrap();
    let start = json!({"prompt":"Derive the result"});
    let first = json!({
        "entry_id":"e-1","entry_type":"derivation","summary":"Old derivation","details":"Old reasoning"
    });
    let replacement = json!({
        "entry_id":"e-2","entry_type":"correction","summary":"Correction","details":"New reasoning"
    });
    let prefix = [
        ("session.start", 1, &start),
        ("proof.entry", 1, &first),
        ("proof.entry", 1, &replacement),
    ];
    ledger.append_new_session("s-base", &prefix).unwrap();
    ledger.append_new_session("s-branch", &prefix).unwrap();
    ledger
        .append(
            "s-branch",
            "proof.supersede",
            1,
            &json!({
                "entry_id":"e-1","replaced_by":"e-2","reason":"Correction"
            }),
        )
        .unwrap();

    let report = compute_branch_diff(&ledger, "s-base", "s-branch").unwrap();
    assert_eq!(report.base_entries[0]["status"], "pending");
    assert_eq!(report.target_entries[0]["status"], "superseded");
    assert_eq!(report.target_entries[0]["superseded_by"], "e-2");
    assert!(report.summary_diff.contains("superseded by e-2"));
}

#[test]
fn latex_export_uses_one_display_wrapper_per_formula() {
    let temp = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(temp.path()).unwrap();
    ledger
        .append("s-tex", "session.start", 1, &json!({}))
        .unwrap();
    let bib = ledger.put_blob(serde_json::to_string(&json!({
        "bibtex": "@article{Convergence2020,title={Convergence},author={Example},year={2020}}"
    })).unwrap().as_bytes()).unwrap();
    for (i, formula) in [
        r"\begin{align*}x &= 1 \\ y &= 2\end{align*}",
        r"\[a=1\]",
        "$$b=2$$",
        "$c=3$",
        r"\(d = 4\)",
        r"\begin{aligned}e &= 5\end{aligned}",
        "f = 6",
    ]
    .iter()
    .enumerate()
    {
        ledger
            .append(
                "s-tex",
                "proof.entry",
                1,
                &json!({
                    "entry_id":format!("e-{}", i + 1),"entry_type":"derivation",
                    "summary":"Identities","details":"Verify the identity","latex":formula
                }),
            )
            .unwrap();
    }
    ledger
        .append(
            "s-tex",
            "proof.entry",
            1,
            &json!({
                "entry_id":"e-8", "entry_type":"numerical_computation", "summary":"Numerical limit",
                "latex":r"\begin{align*}R &= 4\\ \delta R &= 0\end{align*}", "depends_on":["e-2"]
            }),
        )
        .unwrap();
    ledger
        .append("s-tex", "literature.fetched", 1, &json!({"raw_blob":bib}))
        .unwrap();
    let (_, bytes) = export_session_bytes(temp.path(), "s-tex", ExportFormat::Tex).unwrap();
    let report = String::from_utf8(bytes).unwrap();
    assert!(report.contains("\\begin{lemma}[{e-1 [pending] (seq 2)}]"));
    assert!(report.contains("\\begin{align*}x &= 1 \\\\ y &= 2\\end{align*}\n\\end{lemma}"));
    assert!(report.contains("\\[a=1\\]\n\\end{lemma}"));
    assert!(!report.contains("\\[\n\\begin{align*}"));
    assert!(!report.contains("\\[\n\\["));
    assert!(report.contains("\\begin{align*}R &= 4\\\\ \\delta R &= 0\\end{align*}"));
    assert!(report.contains("\\emph{Depends on:} e-2"));
    assert!(report.contains("\\nocite{*}\n\\bibliographystyle{unsrt}"));
    for formula in [
        "b=2",
        "c=3",
        "d = 4",
        r"\begin{aligned}e &= 5\end{aligned}",
        "f = 6",
    ] {
        assert!(report.contains(&format!("\\[\n{formula}\n\\]")));
    }
}

#[test]
fn handoff_checks_source_hashes_and_compares_seeded_values() {
    let temp = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(temp.path()).unwrap();
    let session = "s-handoff";
    let commit = "a".repeat(40);
    let def_hash = "b".repeat(64);
    ledger
        .append(
            session,
            "session.start",
            1,
            &json!({
                "workspace_revision": { "git_commit": commit, "dirty": null },
                "tool_defs": [{ "id": "sympy_eval", "def_hash": def_hash }],
            }),
        )
        .unwrap();
    let sources = temp
        .path()
        .join("sessions")
        .join(session)
        .join("sources")
        .join("paper");
    std::fs::create_dir_all(&sources).unwrap();
    std::fs::write(sources.join("main.tex"), "recorded source").unwrap();
    std::fs::write(sources.join("changed.bib"), "changed after recording").unwrap();
    let original = ledger.put_blob(b"archive contents").unwrap();
    let source_hash = ledger::sha256_hex(b"recorded source");
    ledger
        .append(
            session,
            "source.ingested",
            1,
            &json!({
                "source_id": "paper",
                "raw_blob": original,
                "files": ["paper/main.tex", "paper/changed.bib"],
                "files_sha256": {
                    "paper/main.tex": source_hash,
                    "paper/changed.bib": ledger::sha256_hex(b"original text"),
                },
            }),
        )
        .unwrap();

    let workspace_hash = ledger::sha256_hex(b"full workspace file");
    ledger
        .append(
            session,
            "tool.request",
            1,
            &json!({
                "call_id": "workspace-read", "tool": "fs_read",
            }),
        )
        .unwrap();
    ledger.append(session, "tool.result", 1, &json!({
        "call_id": "workspace-read", "ok": true,
        "result": {"inline": json!({
            "root": "workspace", "path": "notes/paper.tex", "file_sha256": workspace_hash,
            "content": "full", "offset": 0, "end_offset": 4, "total_chars": 19, "truncated": true,
        }).to_string()},
    })).unwrap();

    let code = "set_random_seed(11)\nfrom pathlib import Path\nPath('replay-executed.txt').touch()\nfrom random import randrange\nrandrange(1000)";
    let request_seq = ledger
        .append(
            session,
            "tool.request",
            1,
            &json!({
                "call_id": "seeded",
                "tool": "sympy_eval",
                "args": { "inline": json!({ "code": code }).to_string() },
            }),
        )
        .unwrap();
    let recorded_result = json!({
        "value": "463",
        "comparison": { "kind": "int", "value": "463" },
        "seed_events": [{ "seed": 11, "generators": ["python.random"] }],
        "truncated": false,
    });
    let result_seq = ledger
        .append(
            session,
            "tool.result",
            1,
            &json!({
                "call_id": "seeded",
                "ok": true,
                "result": { "inline": recorded_result.to_string() },
                "worker_execution": {
                    "instance": "worker-1",
                    "epoch": 1,
                    "request_id": 1,
                    "environment": { "protocol": 1, "python": "0.0", "unrelated": "omit from manifest" },
                },
            }),
        )
        .unwrap();
    let (_, bytes) = export_session_bytes(temp.path(), session, ExportFormat::Zip).unwrap();
    let output = temp.path().join("bundle");
    zip::ZipArchive::new(Cursor::new(bytes))
        .unwrap()
        .extract(&output)
        .unwrap();
    let manifest_path = output.join("replay_manifest.json");
    let mut manifest: Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(
        manifest["turns"][0]["workspace_revision"]["git_commit"],
        commit
    );
    assert!(manifest["turns"][0]["workspace_revision"]["dirty"].is_null());
    assert_eq!(
        manifest["turns"][0]["tool_definitions"][0]["sha256"],
        def_hash
    );
    assert_eq!(manifest["workspace_files"][0]["path"], "notes/paper.tex");
    assert_eq!(manifest["workspace_files"][0]["sha256"], workspace_hash);
    assert_eq!(manifest["workspace_files"][0]["result_coverage"], "excerpt");
    assert!(manifest["workspace_files"][0].get("content").is_none());
    assert!(!output.join("notes/paper.tex").exists());
    let source = &manifest["source_files"][1];
    assert_eq!(source["status"], "matches_recorded");
    assert_eq!(source["sha256"], source_hash);
    assert_eq!(source["revisions"][0]["original_sha256"], original);
    assert_eq!(manifest["source_files"][0]["status"], "hash_mismatch");
    assert!(!output.join("sources/paper/changed.bib").exists());
    assert!(std::fs::read(output.join("references.bib"))
        .unwrap()
        .is_empty());
    let step = &manifest["workers"][0]["steps"][0];
    assert_eq!(step["request_seq"], request_seq);
    assert_eq!(step["result_seq"], result_seq);
    assert_eq!(step["sha256"], ledger::sha256_hex(code.as_bytes()));
    assert_eq!(
        step["worker_environment"],
        json!({"protocol": 1, "python": "0.0"})
    );
    assert_eq!(step["recorded_result"], recorded_result);
    for filename in ["ledger.json", "report.tex", "report.html", "references.bib"] {
        let digest = ledger::sha256_hex(&std::fs::read(output.join(filename)).unwrap());
        assert!(manifest["document_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["path"] == filename && file["sha256"] == digest));
    }

    let python = ["RE_SEARCH_PYTHON", "RESEARCH_CORE_PYTHON"]
        .into_iter()
        .find_map(|key| std::env::var_os(key).filter(|value| !value.is_empty()));
    let Some(python) = python else {
        eprintln!("SKIPPED Python replay: set RE_SEARCH_PYTHON");
        return;
    };
    let run = |args: &[&str]| {
        Command::new(&python)
            .arg("run_all_checks.py")
            .args(args)
            .current_dir(&output)
            .output()
            .unwrap()
    };
    assert!(run(&["--verify-only"]).status.success());
    assert!(!output.join("replay-executed.txt").exists());
    let report_file = output.join("report.tex");
    let original_report = std::fs::read(&report_file).unwrap();
    std::fs::write(&report_file, "accidentally edited report").unwrap();
    let edited = run(&["--verify-only"]);
    assert!(!edited.status.success());
    assert!(String::from_utf8_lossy(&edited.stderr).contains("hash mismatch for report.tex"));
    std::fs::write(&report_file, original_report).unwrap();
    let observed = run(&[]);
    assert!(
        observed.status.success(),
        "{}",
        String::from_utf8_lossy(&observed.stderr)
    );
    let proof = String::from_utf8_lossy(&observed.stdout);
    assert!(proof.contains("(computed value)"), "{proof}");
    assert!(proof.contains("runtime differs for python"), "{proof}");
    assert!(output.join("replay-executed.txt").exists());

    manifest["workers"][0]["steps"][0]["recorded_result"]["comparison"]["value"] = json!("999");
    std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let drift = run(&[]);
    assert!(!drift.status.success());
    assert!(String::from_utf8_lossy(&drift.stderr).contains("computed value differs"));

    std::fs::write(output.join("sources/paper/main.tex"), "edited bundle file").unwrap();
    let corruption = run(&["--verify-only"]);
    assert!(!corruption.status.success());
    assert!(String::from_utf8_lossy(&corruption.stderr)
        .contains("hash mismatch for sources/paper/main.tex"));
}
