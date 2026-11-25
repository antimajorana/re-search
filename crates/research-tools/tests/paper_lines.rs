use ledger::Ledger;
use research_tools::paper_lines::{read_lines, MAIN_BUDGET_CHARS, READER_BUDGET_CHARS};
use research_tools::{ingest_bytes, IngestRequest, ResearchEnv, SandboxRoot};
use serde_json::json;
use std::sync::Mutex;

const SID: &str = "paper-lines-test";

fn upload(env: &ResearchEnv, db: &Mutex<Ledger>, name: &str, bytes: &[u8]) -> research_tools::IngestedSource {
    ingest_bytes(
        &env.sandbox(),
        db,
        &IngestRequest {
            session_id: SID,
            file_name: name,
            origin: "test",
            raw_bytes: bytes,
            source_id_hint: None,
            section_filter: None,
            title: None,
        },
    )
    .unwrap()
}

fn clip_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}

#[test]
fn numbering_and_line_boundary_cut() {
    let tmp = tempfile::tempdir().unwrap();
    let env = ResearchEnv::new(tmp.path());
    let db = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let text: String = (1..=200)
        .map(|i| format!("Line {i:03} content {}", "a".repeat(120)))
        .collect::<Vec<_>>()
        .join("\n");
    let src = upload(&env, &db, "paper.txt", text.as_bytes());
    let ledger = db.into_inner().unwrap();
    let res = read_lines(
        &env,
        &ledger,
        SID,
        &json!({"source_id": src.source_id, "start_line": 1, "end_line": 200}),
        MAIN_BUDGET_CHARS,
    )
    .unwrap();
    let canon = tools::canon::canonical_json(&res);
    assert!(canon.starts_with(r#"{"a_meta":"#), "a_meta must sort first");
    assert!(canon.chars().count() <= MAIN_BUDGET_CHARS);
    let end = res["a_meta"]["end_line"].as_u64().unwrap() as usize;
    assert!(end > 10 && end < 200);
    let body = res["text"].as_str().unwrap();
    assert!(body.starts_with("1\tLine 001 content "));
    assert!(body.ends_with('\n'));
    let delivered_lines: Vec<&str> = body.lines().collect();
    assert_eq!(delivered_lines.len(), end);
    assert!(delivered_lines.last().unwrap().starts_with(&format!("{end}\tLine {end:03}")));
}

#[test]
fn next_line_is_first_undelivered() {
    let tmp = tempfile::tempdir().unwrap();
    let env = ResearchEnv::new(tmp.path());
    let db = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let text: String = (1..=150)
        .map(|i| format!("Row {i:03} {}", "b".repeat(140)))
        .collect::<Vec<_>>()
        .join("\n");
    let src = upload(&env, &db, "paper.txt", text.as_bytes());
    let ledger = db.into_inner().unwrap();
    let first = read_lines(
        &env,
        &ledger,
        SID,
        &json!({"source_id": src.source_id, "start_line": 1, "end_line": 150}),
        READER_BUDGET_CHARS,
    )
    .unwrap();
    let end1 = first["a_meta"]["end_line"].as_u64().unwrap();
    let next1 = first["a_meta"]["next_line"].as_u64().unwrap();
    assert_eq!(next1, end1 + 1);
    let second = read_lines(
        &env,
        &ledger,
        SID,
        &json!({"source_id": src.source_id, "start_line": next1, "end_line": 150}),
        READER_BUDGET_CHARS,
    )
    .unwrap();
    assert!(second["text"].as_str().unwrap().starts_with(&format!("{next1}\tRow {next1:03}")));
    // Reading through EOF omits next_line.
    let tail = read_lines(
        &env,
        &ledger,
        SID,
        &json!({"source_id": src.source_id, "start_line": 148, "end_line": 150}),
        MAIN_BUDGET_CHARS,
    )
    .unwrap();
    assert_eq!(tail["a_meta"]["end_line"], 150);
    assert!(tail["a_meta"].get("next_line").is_none());
}

#[test]
fn budget_7000_survives_worker_clip_unchanged() {
    let tmp = tempfile::tempdir().unwrap();
    let env = ResearchEnv::new(tmp.path());
    let db = Mutex::new(Ledger::open(tmp.path()).unwrap());
    // Include quotes and backslashes so JSON escaping expands length.
    let text: String = (1..=120)
        .map(|i| format!(r#"Eq ({i}): "\alpha_\mu + \beta_\nu" = "{}"."#, "q".repeat(90)))
        .collect::<Vec<_>>()
        .join("\n");
    let src = upload(&env, &db, "escaped.txt", text.as_bytes());
    let ledger = db.into_inner().unwrap();
    let res = read_lines(
        &env,
        &ledger,
        SID,
        &json!({"source_id": src.source_id, "start_line": 1, "end_line": 120}),
        READER_BUDGET_CHARS,
    )
    .unwrap();
    let canon = tools::canon::canonical_json(&res);
    assert!(canon.chars().count() <= READER_BUDGET_CHARS);
    assert_eq!(clip_chars(&canon, 8_000), canon);
}

#[test]
fn oversize_single_line_truncated_flag() {
    let tmp = tempfile::tempdir().unwrap();
    let env = ResearchEnv::new(tmp.path());
    let db = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let text = format!("{}\nSecond short line.\n", "Z".repeat(20_000));
    let src = upload(&env, &db, "huge.txt", text.as_bytes());
    let ledger = db.into_inner().unwrap();
    let res = read_lines(
        &env,
        &ledger,
        SID,
        &json!({"source_id": src.source_id, "start_line": 1, "end_line": 2}),
        READER_BUDGET_CHARS,
    )
    .unwrap();
    assert_eq!(res["a_meta"]["end_line"], 1);
    assert_eq!(res["a_meta"]["next_line"], 2);
    assert_eq!(res["a_meta"]["line_truncated"], true);
    let canon = tools::canon::canonical_json(&res);
    assert!(canon.chars().count() <= READER_BUDGET_CHARS);
}

#[test]
fn rewritten_file_refused_by_hash_pin() {
    let tmp = tempfile::tempdir().unwrap();
    let env = ResearchEnv::new(tmp.path());
    let db = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let src = upload(&env, &db, "paper.txt", b"Original verified line.\n");
    let ledger = db.into_inner().unwrap();
    let resolved = env
        .sandbox()
        .resolve_existing(SandboxRoot::Sources, SID, src.primary_path.as_deref().unwrap(), false)
        .unwrap();
    std::fs::write(resolved.canonical_target, b"Tampered line on disk.\n").unwrap();
    let err = read_lines(
        &env,
        &ledger,
        SID,
        &json!({"source_id": src.source_id, "start_line": 1, "end_line": 1}),
        MAIN_BUDGET_CHARS,
    )
    .unwrap_err();
    assert!(err.contains("changed since ingestion"), "got: {err}");
}

#[test]
fn tex_assembled_view_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let env = ResearchEnv::new(tmp.path());
    let db = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let tex = b"\\documentclass{article}\n\\begin{document}\n\\section{Intro}\nHello world.\n\\end{document}\n";
    let src = ingest_bytes(
        &env.sandbox(),
        &db,
        &IngestRequest {
            session_id: SID,
            file_name: "paper.tex",
            origin: "test",
            raw_bytes: tex,
            source_id_hint: None,
            section_filter: Some("Intro"),
            title: None,
        },
    )
    .unwrap();
    let assembled = src.assembled_path.expect("expected assembled_path for filtered tex");
    let ledger = db.into_inner().unwrap();
    let err = read_lines(
        &env,
        &ledger,
        SID,
        &json!({"source_id": src.source_id, "path": assembled, "start_line": 1, "end_line": 3}),
        MAIN_BUDGET_CHARS,
    )
    .unwrap_err();
    assert!(err.contains("assembled"), "got: {err}");
    // Reading the original primary TeX file succeeds.
    let ok = read_lines(
        &env,
        &ledger,
        SID,
        &json!({"source_id": src.source_id, "start_line": 1, "end_line": 5}),
        MAIN_BUDGET_CHARS,
    )
    .unwrap();
    assert_eq!(ok["a_meta"]["path"], src.primary_path.unwrap());
}

#[test]
fn pages_meta_from_pdf_page_lines() {
    let tmp = tempfile::tempdir().unwrap();
    let env = ResearchEnv::new(tmp.path());
    let db = Mutex::new(Ledger::open(tmp.path()).unwrap());
    let text = "P1 L1\nP1 L2\nP2 L3\nP2 L4\nP3 L5\nP3 L6\n";
    let src = upload(&env, &db, "paper.txt", text.as_bytes());
    let ledger = db.into_inner().unwrap();
    let mut receipt = ledger
        .read_session(SID)
        .unwrap()
        .iter()
        .find(|e| e.seq == src.event_seq)
        .unwrap()
        .payload
        .clone();
    receipt["format"] = json!("pdf");
    receipt["page_count"] = json!(3);
    receipt["pdf_page_lines"] = json!([[1, 1, 2], [2, 3, 4], [3, 5, 6]]);
    ledger.append(SID, "source.ingested", 1, &receipt).unwrap();
    let res = read_lines(
        &env,
        &ledger,
        SID,
        &json!({"source_id": src.source_id, "start_line": 2, "end_line": 4}),
        MAIN_BUDGET_CHARS,
    )
    .unwrap();
    assert_eq!(res["a_meta"]["pages"], json!([[1, 1, 2], [2, 3, 4]]));
    assert_eq!(res["a_meta"]["total_lines"], 6);
}
