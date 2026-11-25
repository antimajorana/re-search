//! Integration tests for Investigate V1 TeX skeleton core (`research_tools::skeleton`).
//!
//! Validates all 8 self-written fixtures against hand-written `expected.json`, verifies
//! rule 6.2 determinism and fingerprint invariance, exercises verified `source.ingested`
//! multi-file loading, tests malformed TeX recovery, and times a 5,000-line generated paper.

use ledger::Ledger;
use research_tools::ingest::{ingest_bytes, IngestRequest};
use research_tools::paper_key::PaperKey;
use research_tools::sandbox::WorkspaceSandbox;
use research_tools::skeleton::{
    build_tex_skeleton, build_tex_skeleton_from_ingested, build_tex_skeleton_timed, MacroDef,
    NumberBasis, PaperSkeleton, ProofHint, SkConfidence, SkNodeKind, SkeletonBuilder,
    SkeletonQuality, Span, TexSkeletonInput, TheoremDef, MAX_INCLUDE_DEPTH, SKELETON_SCHEMA_VERSION,
};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tempfile::TempDir;

#[derive(Debug, Deserialize)]
struct ExpectedFixture {
    paper_key: String,
    version: Option<String>,
    primary_path: String,
    quality: SkeletonQuality,
    quality_notes_contains: Vec<String>,
    theorem_defs: Vec<TheoremDef>,
    macros: Vec<MacroDef>,
    nodes: Vec<ExpectedNode>,
}

#[derive(Debug, Deserialize)]
struct ExpectedNode {
    kind: SkNodeKind,
    env_name: Option<String>,
    title: Option<String>,
    label: Option<String>,
    parent_order: Option<usize>,
    spans: Vec<Span>,
    head_cites: Vec<String>,
    eq_count: u32,
    number: Option<String>,
    number_basis: Option<NumberBasis>,
    confidence: SkConfidence,
    proof_hint: Option<ProofHint>,
}

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skeleton")
}

fn collect_tex_files(dir: &Path) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    collect_tex_rec(dir, dir, &mut files);
    files
}

fn collect_tex_rec(base: &Path, cur: &Path, out: &mut BTreeMap<String, String>) {
    for entry in std::fs::read_dir(cur).expect("read fixture dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_tex_rec(base, &path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("tex") {
            let rel = path
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let content = std::fs::read_to_string(&path).expect("read fixture tex");
            out.insert(rel, content);
        }
    }
}

fn assert_fixture_dir(rel_dir: &str) -> research_tools::skeleton::TexSkeletonOutput {
    let dir = fixtures_root().join(rel_dir);
    let expected_raw = std::fs::read_to_string(dir.join("expected.json"))
        .unwrap_or_else(|e| panic!("missing expected.json in {rel_dir}: {e}"));
    let expected: ExpectedFixture = serde_json::from_str(&expected_raw)
        .unwrap_or_else(|e| panic!("invalid expected.json in {rel_dir}: {e}"));
    let files = collect_tex_files(&dir);
    assert!(!files.is_empty(), "fixture {rel_dir} has no .tex files");

    let input = TexSkeletonInput {
        source_id: format!("src-{}", rel_dir.replace('/', "-")),
        source_seq: 1,
        paper_key: PaperKey::parse(&expected.paper_key).expect("valid expected paper_key"),
        version: expected.version.clone(),
        files: files.clone(),
        primary_path: None,
        files_sha256: None,
    };

    let out = build_tex_skeleton(&input);
    let out2 = build_tex_skeleton(&input);

    // Determinism (rule 6.2): two builds serialize byte-identically and round-trip cleanly.
    let json1 = serde_json::to_string(&out.skeleton).expect("serialize skeleton");
    let json2 = serde_json::to_string(&out2.skeleton).expect("serialize skeleton 2");
    assert_eq!(json1, json2, "{rel_dir}: builds must be byte-identical");
    let roundtrip: PaperSkeleton = serde_json::from_str(&json1).expect("deserialize skeleton");
    assert_eq!(roundtrip, out.skeleton, "{rel_dir}: serde roundtrip mismatch");

    assert_eq!(out.skeleton.schema, SKELETON_SCHEMA_VERSION, "{rel_dir}: schema");
    assert_eq!(out.skeleton.builder, SkeletonBuilder::Tex1, "{rel_dir}: builder");
    assert_eq!(out.flattened.primary_path, expected.primary_path, "{rel_dir}: primary_path");
    assert_eq!(out.skeleton.quality, expected.quality, "{rel_dir}: quality ({:?})", out.quality_notes);
    for needle in &expected.quality_notes_contains {
        assert!(
            out.quality_notes.iter().any(|n| n.contains(needle)),
            "{rel_dir}: expected quality note containing `{needle}`, got {:?}",
            out.quality_notes
        );
    }
    if expected.quality_notes_contains.is_empty() {
        assert!(
            out.quality_notes.is_empty(),
            "{rel_dir}: unexpected quality notes: {:?}",
            out.quality_notes
        );
    }

    let actual_defs: Vec<TheoremDef> = out.theorem_table.defs.values().cloned().collect();
    assert_eq!(actual_defs, expected.theorem_defs, "{rel_dir}: theorem_defs mismatch");
    assert_eq!(out.skeleton.macros, expected.macros, "{rel_dir}: macros mismatch");

    assert_eq!(
        out.skeleton.nodes.len(),
        expected.nodes.len(),
        "{rel_dir}: node count mismatch; actual nodes: {:#?}",
        out.skeleton
            .nodes
            .iter()
            .map(|n| (n.order, n.kind, &n.env_name, &n.title, &n.label, &n.spans))
            .collect::<Vec<_>>()
    );

    for (idx, (actual, exp)) in out.skeleton.nodes.iter().zip(expected.nodes.iter()).enumerate() {
        // V1 invariants: tags, parent tags, and proof_of tags are unset.
        assert_eq!(actual.order, idx, "{rel_dir} node {idx}: order");
        assert!(actual.tag.is_none(), "{rel_dir} node {idx}: V1 must leave tag unset");
        assert!(actual.parent.is_none(), "{rel_dir} node {idx}: V1 must leave parent tag unset");
        assert!(actual.proof_of.is_none(), "{rel_dir} node {idx}: V1 must leave proof_of tag unset");
        assert_eq!(actual.fp.len(), 12, "{rel_dir} node {idx}: fp length");
        assert!(
            actual.fp.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
            "{rel_dir} node {idx}: fp hex `{}`",
            actual.fp
        );

        assert_eq!(actual.kind, exp.kind, "{rel_dir} node {idx}: kind");
        assert_eq!(actual.env_name, exp.env_name, "{rel_dir} node {idx}: env_name");
        assert_eq!(actual.title, exp.title, "{rel_dir} node {idx}: title");
        assert_eq!(actual.label, exp.label, "{rel_dir} node {idx}: label");
        assert_eq!(out.parent_orders[idx], exp.parent_order, "{rel_dir} node {idx}: parent_order");
        assert_eq!(actual.spans, exp.spans, "{rel_dir} node {idx}: spans");
        assert_eq!(actual.head_cites, exp.head_cites, "{rel_dir} node {idx}: head_cites");
        assert_eq!(actual.eq_count, exp.eq_count, "{rel_dir} node {idx}: eq_count");
        assert_eq!(actual.number, exp.number, "{rel_dir} node {idx}: number");
        assert_eq!(actual.number_basis, exp.number_basis, "{rel_dir} node {idx}: number_basis");
        assert_eq!(actual.confidence, exp.confidence, "{rel_dir} node {idx}: confidence");
        assert_eq!(out.proof_hints.get(&idx).cloned(), exp.proof_hint, "{rel_dir} node {idx}: proof_hint");
    }

    // Verify CRLF vs LF invariance (rule 6.2).
    let crlf_files: BTreeMap<String, String> = files
        .iter()
        .map(|(k, v)| (k.clone(), v.replace('\n', "\r\n")))
        .collect();
    let crlf_out = build_tex_skeleton(&TexSkeletonInput { files: crlf_files, ..input });
    assert_eq!(crlf_out.skeleton.nodes, out.skeleton.nodes, "{rel_dir}: CRLF changed nodes");

    out
}

#[test]
fn fixture_01_amsthm_shared_counters() {
    assert_fixture_dir("01_amsthm_shared_counters");
}

#[test]
fn fixture_02_custom_theorem_names() {
    assert_fixture_dir("02_custom_theorem_names");
}

#[test]
fn fixture_03_multi_file() {
    let out = assert_fixture_dir("03_multi_file");
    assert_eq!(
        out.flattened.inlined_files,
        vec![
            "preamble.tex".to_string(),
            "sections/setup.tex".to_string(),
            "sections/sub_lemma.tex".to_string(),
            "sections/results.tex".to_string(),
        ]
    );
}

#[test]
fn fixture_04_revtex_physics() {
    let out = assert_fixture_dir("04_revtex_physics");
    assert!(out.skeleton.nodes.iter().all(|n| !n.kind.is_theorem_like()));
}

#[test]
fn fixture_05_detached_proof() {
    assert_fixture_dir("05_detached_proof");
}

#[test]
fn fixture_06_unlabelled_theorems() {
    let out = assert_fixture_dir("06_unlabelled_theorems");
    // Two identical-body remarks in the same section must produce the exact same fingerprint.
    assert_eq!(out.skeleton.nodes[5].fp, out.skeleton.nodes[6].fp);
}

#[test]
fn fixture_07_two_versions() {
    let v1 = assert_fixture_dir("07_two_versions/v1");
    let v2 = assert_fixture_dir("07_two_versions/v2");

    let fp_of = |out: &research_tools::skeleton::TexSkeletonOutput, lbl: &str| {
        out.skeleton
            .nodes
            .iter()
            .find(|n| n.label.as_deref() == Some(lbl))
            .map(|n| n.fp.clone())
            .unwrap_or_else(|| panic!("missing label {lbl}"))
    };

    // Comment-only edit on thm:decay keeps fp identical.
    assert_eq!(fp_of(&v1, "thm:decay"), fp_of(&v2, "thm:decay"));
    // Shifted-line unchanged thm:sup keeps fp identical.
    assert_eq!(fp_of(&v1, "thm:sup"), fp_of(&v2, "thm:sup"));
    // Edited mathematical statement on prop:gap changes fp.
    assert_ne!(fp_of(&v1, "prop:gap"), fp_of(&v2, "prop:gap"));
}

#[test]
fn fixture_08_hard_edge_cases() {
    assert_fixture_dir("08_hard_edge_cases");
}

#[test]
fn builds_from_ingested_zip_source_without_reading_external_files() {
    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    // Place a secret file outside the ingested source; `\input{../secret.tex}` must not read it.
    std::fs::write(
        tmp.path().join("secret.tex"),
        "\\begin{theorem}\nSecret outside source.\n\\end{theorem}\n",
    )
    .unwrap();

    let mut zip_buf = Vec::new();
    {
        let mut zw = zip::ZipWriter::new(std::io::Cursor::new(&mut zip_buf));
        let opts = zip::write::SimpleFileOptions::default();
        zw.start_file("main.tex", opts).unwrap();
        zw.write_all(
            b"\\documentclass{article}\n\\newtheorem{theorem}{Theorem}\n\\begin{document}\n\\section{Intro}\n\\input{body}\n\\input{../secret}\n\\end{document}\n",
        )
        .unwrap();
        zw.start_file("body.tex", opts).unwrap();
        zw.write_all(b"\\begin{theorem}[Ingested]\\label{thm:in}\n$1+1=2$.\n\\end{theorem}\n")
            .unwrap();
        zw.finish().unwrap();
    }

    let ledger_path = data_dir.join("ledger.db");
    let ledger = Mutex::new(Ledger::open(&ledger_path).expect("open ledger"));
    let sandbox = WorkspaceSandbox::new(data_dir.clone(), None);
    let session = "sess-v1-ingest";

    let ingested = ingest_bytes(
        &sandbox,
        &ledger,
        &IngestRequest {
            session_id: session,
            file_name: "paper.zip",
            origin: "test:paper.zip",
            raw_bytes: &zip_buf,
            source_id_hint: Some("paper"),
            section_filter: None,
            title: None,
        },
    )
    .expect("ingest zip");

    let events = ledger.lock().unwrap().read_session(session).expect("read session");
    let receipt = events.iter().find(|e| e.kind == "source.ingested").expect("receipt");
    let key = PaperKey::from_file_bytes(&zip_buf);
    let (out, _timing) = build_tex_skeleton_from_ingested(&sandbox, session, receipt, key, None)
        .expect("build from ingested");

    // Physical file paths carry `<source_id>/...` and never reference `__assembled__.tex`.
    assert!(out.skeleton.nodes.iter().all(|n| {
        n.spans
            .iter()
            .all(|s| s.path.starts_with(&ingested.source_id) && !s.path.contains("__assembled__"))
    }));
    // The ingested theorem is found; the outside `secret.tex` was NOT read (recorded as unresolved).
    assert_eq!(out.skeleton.nodes.len(), 2);
    assert_eq!(out.skeleton.nodes[1].label.as_deref(), Some("thm:in"));
    assert!(out.quality_notes.iter().any(|n| n.contains("unresolved include")));

    // Tampering with a materialized source file on disk must fail closed.
    let body_on_disk = data_dir
        .join("sessions")
        .join(session)
        .join("sources")
        .join(&ingested.source_id)
        .join("body.tex");
    std::fs::write(&body_on_disk, "tampered").unwrap();
    let err = build_tex_skeleton_from_ingested(
        &sandbox,
        session,
        receipt,
        PaperKey::from_file_bytes(&zip_buf),
        None,
    )
    .unwrap_err();
    assert!(err.contains("changed since ingestion"), "unexpected err: {err}");
}

#[test]
fn malformed_tex_records_quality_notes_and_never_panics() {
    let cases = [
        (
            "unclosed_env",
            "\\begin{document}\n\\section{One}\n\\begin{theorem}\nOops no end.\n\\section{Two}\nText.\n\\end{document}\n",
            "unclosed environment `theorem`",
        ),
        (
            "mismatched_end",
            "\\begin{document}\n\\section{One}\n\\begin{theorem}\nBody.\n\\end{lemma}\n\\end{theorem}\n\\end{document}\n",
            "unmatched \\end{lemma}",
        ),
        (
            "unbalanced_brace_in_section",
            "\\begin{document}\n\\section{Broken heading\n\n\\begin{theorem}\nStill found.\n\\end{theorem}\n\\end{document}\n",
            "unclosed `{` in \\section",
        ),
        (
            "unclosed_verbatim",
            "\\begin{document}\n\\section{A}\n\\begin{verbatim}\nnever closes\n",
            "unclosed verbatim environment `verbatim`",
        ),
        (
            "unclosed_iffalse",
            "\\begin{document}\n\\section{A}\n\\iffalse\nnever closes\n",
            "unclosed \\iffalse block",
        ),
        (
            "unclosed_document",
            "\\begin{document}\n\\section{A}\nHello.\n",
            "unclosed `document` environment",
        ),
    ];

    for (name, tex, expected_note) in cases {
        let mut files = BTreeMap::new();
        files.insert("main.tex".to_string(), tex.to_string());
        let out = build_tex_skeleton(&TexSkeletonInput {
            source_id: name.into(),
            source_seq: 1,
            paper_key: PaperKey::from_file_bytes(tex.as_bytes()),
            version: None,
            files,
            primary_path: Some("main.tex".into()),
            files_sha256: None,
        });
        assert_eq!(out.skeleton.quality, SkeletonQuality::Partial, "{name}: quality");
        assert!(
            out.quality_notes.iter().any(|n| n.contains(expected_note)),
            "{name}: expected `{expected_note}`, got {:?}",
            out.quality_notes
        );
        if name == "unbalanced_brace_in_section" {
            assert!(
                out.skeleton.nodes.iter().any(|n| n.kind == SkNodeKind::Theorem),
                "theorem after broken section heading should still be recovered"
            );
        }
    }

    // Empty / non-TeX input yields Weak quality.
    let mut empty_files = BTreeMap::new();
    empty_files.insert("main.tex".into(), "Just plain text with no commands.\n".into());
    let weak = build_tex_skeleton(&TexSkeletonInput {
        source_id: "weak".into(),
        source_seq: 1,
        paper_key: PaperKey::from_file_bytes(b"weak"),
        version: None,
        files: empty_files,
        primary_path: Some("main.tex".into()),
        files_sha256: None,
    });
    assert_eq!(weak.skeleton.quality, SkeletonQuality::Weak);
}

#[test]
fn include_depth_limit_terminates_with_warning() {
    let mut files = BTreeMap::new();
    let chain_len = MAX_INCLUDE_DEPTH + 4;
    files.insert(
        "main.tex".into(),
        "\\begin{document}\n\\section{Root}\n\\input{f1}\n\\end{document}\n".into(),
    );
    for i in 1..=chain_len {
        let content = if i < chain_len {
            format!("\\begin{{lemma}}\nDepth {i}.\n\\end{{lemma}}\n\\input{{f{}}}\n", i + 1)
        } else {
            "\\begin{lemma}\nDeepest.\n\\end{lemma}\n".to_string()
        };
        files.insert(format!("f{i}.tex"), content);
    }
    let out = build_tex_skeleton(&TexSkeletonInput {
        source_id: "deep".into(),
        source_seq: 1,
        paper_key: PaperKey::from_file_bytes(b"deep"),
        version: None,
        files,
        primary_path: Some("main.tex".into()),
        files_sha256: None,
    });
    assert_eq!(out.skeleton.quality, SkeletonQuality::Partial);
    assert!(
        out.quality_notes.iter().any(|n| n.contains("include depth limit")),
        "expected depth warning, got {:?}",
        out.quality_notes
    );
}

#[test]
fn timing_5000_line_generated_paper() {
    let mut tex = String::with_capacity(220_000);
    tex.push_str("\\documentclass{article}\n\\usepackage{amsmath,amsthm}\n");
    tex.push_str("\\newtheorem{theorem}{Theorem}[section]\n\\newtheorem{lemma}[theorem]{Lemma}\n");
    tex.push_str("\\begin{document}\n");
    let mut lines = 5usize;
    let mut sec = 0usize;
    let mut thm = 0usize;
    // Build a realistic 5,000-line paper: sections, lemmas, theorems, proofs, equations, prose.
    while lines + 25 <= 5_000 {
        sec += 1;
        tex.push_str(&format!("\\section{{Section {sec}}}\\label{{sec:{sec}}}\n"));
        lines += 1;
        for j in 0..3 {
            thm += 1;
            tex.push_str(&format!(
                "\\begin{{lemma}}[Aux {sec}.{j}]\\label{{lem:{thm}}}\nLet $x_n$ converge with rate $n^{{-2}}$.\n\\end{{lemma}}\n"
            ));
            tex.push_str("\\begin{proof}\nDirect summation of the tail series.\n\\end{proof}\n");
            tex.push_str(&format!(
                "\\begin{{equation}}\\label{{eq:{thm}}}\nE_{{{thm}}} = \\sum_{{k=1}}^{{n}} k^{{-2}}.\n\\end{{equation}}\n"
            ));
            lines += 9;
        }
        tex.push_str("% Commentary line between sections.\n");
        lines += 1;
    }
    while lines + 1 < 5_000 {
        tex.push_str("Additional expository sentence for line padding.\n");
        lines += 1;
    }
    tex.push_str("\\end{document}\n");

    let mut files = BTreeMap::new();
    files.insert("main.tex".into(), tex);
    let (out, timing) = build_tex_skeleton_timed(&TexSkeletonInput {
        source_id: "bench-5000".into(),
        source_seq: 1,
        paper_key: PaperKey::from_file_bytes(b"bench-5000"),
        version: None,
        files,
        primary_path: Some("main.tex".into()),
        files_sha256: None,
    });

    eprintln!(
        "5000-line skeleton timing: lines={}, nodes={}, json_bytes={}, flatten={}us, scan={}us, total={}us",
        timing.flattened_lines,
        timing.node_count,
        timing.json_bytes,
        timing.flatten_us,
        timing.scan_us,
        timing.total_us
    );

    assert_eq!(timing.flattened_lines, 5_000);
    assert_eq!(out.skeleton.quality, SkeletonQuality::Full);
    assert_eq!(timing.node_count, sec + thm * 3);
    // Generous CI bound (spec target is <10 s for whole fetch+ingest+build; scanning 5000 lines is typically <50 ms).
    assert!(
        timing.total_duration().as_secs() < 5,
        "5000-line skeleton build took too long: {}us",
        timing.total_us
    );
}
