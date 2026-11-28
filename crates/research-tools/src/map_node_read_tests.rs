use super::{load_pinned_skeleton, read_map_node};
use crate::ingest::{ingest_bytes, IngestRequest};
use crate::investigation_markers::{
    EVENT_INVESTIGATION_SKELETON, MARKER_TASK_FINISHED, MARKER_TASK_REQUESTED,
};
use crate::lines_sent::{lines_sent_with_ledger, READ_MAP_NODE_TOOL};
use crate::paper_key::PaperKey;
use crate::skeleton::{
    build_tex_skeleton_from_ingested, EdgeConfidence, PaperSkeleton, SkEdge, SkEdgeKind,
    SkEdgeRole, SkEdgeRule, SkNodeKind, Span, SymbolDef, SymbolDefHow,
};
use crate::ResearchEnv;
use ledger::Ledger;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Mutex;

struct TestFixture {
    _tmp: tempfile::TempDir,
    data_dir: PathBuf,
    env: ResearchEnv,
    ledger: Ledger,
    session: String,
    source_id: String,
    primary_path: String,
    file_sha256: String,
    skeleton_seq: u64,
    blob_hash: String,
    skeleton: PaperSkeleton,
}

const SAMPLE_TEX: &str = r#"\documentclass{article}
\newtheorem{theorem}{Theorem}[section]
\newtheorem{lemma}[theorem]{Lemma}
\newtheorem{corollary}[theorem]{Corollary}
\newtheorem{definition}{Definition}
\newcommand{\Norm}[1]{\|#1\|}
\begin{document}
\section{Setup}\label{sec:setup}
We introduce the main notation before proving the bound.
\begin{definition}[Weighted Lattice]\label{def:lat}
Let $L$ be a finite lattice equipped with $\Norm{\cdot}$.
\end{definition}
\begin{lemma}[Step Bound]\label{lem:bound}
Every admissible state $x \in L$ satisfies a uniform step bound.
\end{lemma}
\begin{equation}\label{eq:energy}
E(x) = \Norm{x}^2 + 1
\end{equation}
\begin{theorem}[Main Stability]\label{thm:main}
Under Definition~\ref{def:lat}, the functional in~\eqref{eq:energy}
remains strictly bounded along every admissible trajectory.
Additional detail line three for testing clean line-boundary truncation.
Additional detail line four with explicit coefficient $\alpha_0 > 0$.
Additional detail line five concluding the theorem statement cleanly.
\end{theorem}
\begin{proof}
Apply Lemma~\ref{lem:bound} to each step of the trajectory, then
substitute the energy identity~\eqref{eq:energy} into the telescope sum.
Line three of the proof closes the estimate without extra assumptions.
Line four of the proof confirms every constant is independent of size.
\end{proof}
\begin{corollary}[Exponential Decay]\label{cor:decay}
By Theorem~\ref{thm:main}, the residual decays exponentially in $n$.
\end{corollary}
\begin{remark}[Later Note]\label{rem:later}
A closing remark introducing a later symbol $\Omega$.
\end{remark}
\end{document}
"#;

fn setup_fixture(tex: &str, customize: impl FnOnce(&mut PaperSkeleton)) -> TestFixture {
    let tmp = tempfile::tempdir().expect("tempdir");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).expect("mkdir data");
    let env = ResearchEnv::new(&data_dir);
    let ledger = Ledger::open(&data_dir).expect("open ledger");
    let session = "sess-map-node-read".to_string();
    ledger.append(&session, "session.start", 1, &json!({})).expect("start");
    let ledger_mutex = Mutex::new(ledger);
    let ingested = ingest_bytes(
        &env.sandbox(),
        &ledger_mutex,
        &IngestRequest {
            session_id: &session,
            file_name: "main.tex",
            origin: "test://fixture",
            raw_bytes: tex.as_bytes(),
            source_id_hint: Some("src1"),
            section_filter: None,
            title: Some("Synthetic Test Paper"),
        },
    )
    .expect("ingest tex");
    let ledger = ledger_mutex.into_inner().expect("unwrap ledger");
    let (_, events) = ledger.read_verified_session(&session).expect("events");
    let receipt = events
        .iter()
        .rev()
        .find(|e| e.kind == "source.ingested" && e.payload["source_id"] == ingested.source_id)
        .expect("source.ingested receipt");
    let primary_path = receipt.payload["primary_path"].as_str().expect("primary_path").to_string();
    let file_sha256 = receipt.payload["files_sha256"][&primary_path].as_str().expect("sha256").to_string();
    let paper_key = PaperKey::from_file_bytes(tex.as_bytes());
    let (mut output, _) = build_tex_skeleton_from_ingested(&env.sandbox(), &session, receipt, paper_key, None).expect("skeleton");
    customize(&mut output.skeleton);
    let (skeleton_seq, blob_hash) = append_skeleton(&ledger, &session, &output.skeleton);
    TestFixture {
        _tmp: tmp, data_dir, env, ledger, session, source_id: ingested.source_id,
        primary_path, file_sha256, skeleton_seq, blob_hash, skeleton: output.skeleton,
    }
}

fn append_skeleton(ledger: &Ledger, session: &str, skel: &PaperSkeleton) -> (u64, String) {
    let bytes = serde_json::to_vec(skel).expect("serialize skeleton");
    let blob_hash = ledger.put_blob(&bytes).expect("put blob");
    let seq = ledger
        .append(
            session,
            EVENT_INVESTIGATION_SKELETON,
            1,
            &json!({ "source_id": skel.source_id, "source_seq": skel.source_seq, "builder": skel.builder, "blob": blob_hash }),
        )
        .expect("append skeleton");
    (seq, blob_hash)
}

fn node_tag_by_label(skel: &PaperSkeleton, label: &str) -> String {
    skel.nodes
        .iter()
        .find(|n| n.label.as_deref() == Some(label))
        .and_then(|n| n.tag.clone())
        .unwrap_or_else(|| panic!("missing node with label {label}"))
}

#[test]
fn lookup_by_tag_label_and_printed_references() {
    let fx = setup_fixture(SAMPLE_TEX, |_| {});
    let thm_tag = node_tag_by_label(&fx.skeleton, "thm:main");
    let lem_tag = node_tag_by_label(&fx.skeleton, "lem:bound");
    let eq_tag = node_tag_by_label(&fx.skeleton, "eq:energy");
    let sec_tag = node_tag_by_label(&fx.skeleton, "sec:setup");
    let def_tag = node_tag_by_label(&fx.skeleton, "def:lat");
    let cor_tag = node_tag_by_label(&fx.skeleton, "cor:decay");

    let queries = [
        (thm_tag.as_str(), thm_tag.as_str()),
        ("thm:main", thm_tag.as_str()),
        ("Theorem 1.2", thm_tag.as_str()),
        ("Thm 1.2", thm_tag.as_str()),
        ("Thm. 1.2", thm_tag.as_str()),
        ("  theorem   1.2 ", thm_tag.as_str()),
        ("lem:bound", lem_tag.as_str()),
        ("Lemma 1.1", lem_tag.as_str()),
        ("Lem. 1.1", lem_tag.as_str()),
        ("Eq. (1)", eq_tag.as_str()),
        ("Eq (1)", eq_tag.as_str()),
        ("Equation 1", eq_tag.as_str()),
        ("Sec. 1", sec_tag.as_str()),
        ("Def 1", def_tag.as_str()),
        ("Cor 1.3", cor_tag.as_str()),
    ];
    for (query, expected_tag) in queries {
        let out = read_map_node(&fx.env, &fx.ledger, &fx.session, &json!({ "source_id": fx.source_id, "node": query }))
            .unwrap_or_else(|e| panic!("query `{query}` failed: {e}"));
        assert_eq!(out["tag"].as_str(), Some(expected_tag), "query `{query}`");
        assert_eq!(out["skeleton_seq"].as_u64(), Some(fx.skeleton_seq));
        assert_eq!(out["truncated"].as_bool(), Some(false));
        let files = out["files"].as_array().expect("files array");
        assert!(!files.is_empty());
        assert_eq!(files[0]["file_sha256"].as_str(), Some(fx.file_sha256.as_str()));
        assert_eq!(files[0]["path"].as_str(), Some(fx.primary_path.as_str()));
    }
}

#[test]
fn ambiguous_reference_and_unknown_node_errors() {
    let fx = setup_fixture(SAMPLE_TEX, |skel| {
        let mut dup = skel.nodes.iter().find(|n| n.label.as_deref() == Some("thm:main")).expect("thm").clone();
        dup.tag = Some("n-dup1111111".into());
        dup.label = Some("thm:duplicate".into());
        dup.order = skel.nodes.len();
        skel.nodes.push(dup);
    });

    let amb_err = read_map_node(&fx.env, &fx.ledger, &fx.session, &json!({ "source_id": fx.source_id, "node": "Theorem 1.2" }))
        .expect_err("duplicate printed ref must error");
    assert!(amb_err.contains("Ambiguous"), "got: {amb_err}");
    assert!(amb_err.contains("thm:main") && amb_err.contains("thm:duplicate") && amb_err.contains("n-dup1111111"));

    let missing_err = read_map_node(&fx.env, &fx.ledger, &fx.session, &json!({ "source_id": fx.source_id, "node": "Theorem 9.9" }))
        .expect_err("unknown node must error");
    assert!(missing_err.contains("not found") && missing_err.contains("Available nodes:"), "got: {missing_err}");
    let thm_tag = node_tag_by_label(&fx.skeleton, "thm:main");
    assert!(missing_err.contains(&thm_tag), "got: {missing_err}");
    let after = missing_err.split("Available nodes: ").nth(1).expect("suggestions");
    assert!(after.split("; ").count() <= 8, "too many suggestions: {after}");
}

#[test]
fn statement_proof_edges_symbols_and_selective_include() {
    let fx = setup_fixture(SAMPLE_TEX, |skel| {
        let path = skel.nodes[0].spans[0].path.clone();
        let def_tag = node_tag_by_label(skel, "def:lat");
        let thm_tag = node_tag_by_label(skel, "thm:main");
        let rem_tag = node_tag_by_label(skel, "rem:later");
        let sec_tag = node_tag_by_label(skel, "sec:setup");
        skel.edges.push(SkEdge {
            from: sec_tag, to: thm_tag.clone(), kind: SkEdgeKind::Uses, role: SkEdgeRole::Prose,
            site: Span::new(&path, 9, 9), rule: SkEdgeRule::TextMention, confidence: EdgeConfidence::Heuristic,
        });
        skel.symbols = vec![
            SymbolDef { symbol_tex: "\\Norm".into(), norm: "norm".into(), span: Span::new(&path, 6, 6), in_node: None, how: SymbolDefHow::Newcommand },
            SymbolDef { symbol_tex: "L_old".into(), norm: "lattice".into(), span: Span::new(&path, 10, 10), in_node: Some(def_tag.clone()), how: SymbolDefHow::DefinePhrase },
            SymbolDef { symbol_tex: "L".into(), norm: "lattice".into(), span: Span::new(&path, 11, 11), in_node: Some(def_tag), how: SymbolDefHow::DefinePhrase },
            SymbolDef { symbol_tex: "\\alpha_0".into(), norm: "alpha0".into(), span: Span::new(&path, 22, 22), in_node: Some(thm_tag), how: SymbolDefHow::DefinePhrase },
            SymbolDef { symbol_tex: "\\Omega".into(), norm: "omega".into(), span: Span::new(&path, 35, 35), in_node: Some(rem_tag), how: SymbolDefHow::DefinePhrase },
        ];
    });

    let def_tag = node_tag_by_label(&fx.skeleton, "def:lat");
    let lem_tag = node_tag_by_label(&fx.skeleton, "lem:bound");
    let eq_tag = node_tag_by_label(&fx.skeleton, "eq:energy");
    let sec_tag = node_tag_by_label(&fx.skeleton, "sec:setup");
    let cor_tag = node_tag_by_label(&fx.skeleton, "cor:decay");

    let full = read_map_node(&fx.env, &fx.ledger, &fx.session, &json!({ "source_id": fx.source_id, "node": "thm:main" }))
        .expect("read thm:main");
    let files = full["files"].as_array().expect("files");
    assert_eq!(files.len(), 2);
    assert_eq!(files[0]["section"].as_str(), Some("statement"));
    assert!(files[0]["text"].as_str().expect("stmt").contains("Main Stability"));
    assert_eq!(files[1]["section"].as_str(), Some("proof"));
    assert!(files[1]["text"].as_str().expect("proof").contains("telescope sum"));
    assert!(full["proof"]["tag"].as_str().is_some());

    fn tags_in(v: &serde_json::Value) -> Vec<&str> {
        v.as_array().expect("array").iter().filter_map(|x| x["tag"].as_str()).collect()
    }
    let stmt_uses = tags_in(&full["uses"]["statement"]);
    assert!(stmt_uses.contains(&def_tag.as_str()) && stmt_uses.contains(&eq_tag.as_str()));
    let proof_uses = tags_in(&full["uses"]["proof"]);
    assert!(proof_uses.contains(&lem_tag.as_str()) && proof_uses.contains(&eq_tag.as_str()));
    assert_eq!(tags_in(&full["uses"]["prose"]), vec![sec_tag.as_str()]);
    assert_eq!(tags_in(&full["used_by"]), vec![cor_tag.as_str()]);

    let sym_tex: Vec<&str> = full["symbols"].as_array().expect("symbols").iter().filter_map(|v| v["symbol_tex"].as_str()).collect();
    assert!(sym_tex.contains(&"\\alpha_0"), "local symbol missing: {sym_tex:?}");
    assert!(sym_tex.contains(&"L"), "latest preceding def symbol missing: {sym_tex:?}");
    assert!(!sym_tex.contains(&"L_old"), "shadowed norm must be superseded: {sym_tex:?}");
    assert!(!sym_tex.contains(&"\\Omega"), "later symbol must be excluded: {sym_tex:?}");

    let partial = read_map_node(
        &fx.env, &fx.ledger, &fx.session,
        &json!({ "source_id": fx.source_id, "node": "thm:main", "include": ["statement", "uses"] }),
    )
    .expect("selective include");
    assert_eq!(partial["files"].as_array().expect("files").len(), 1);
    assert_eq!(partial["files"][0]["section"].as_str(), Some("statement"));
    assert!(partial.get("uses").is_some());
    assert!(partial.get("proof").is_none() && partial.get("used_by").is_none() && partial.get("symbols").is_none());
}

#[test]
fn strict_max_chars_truncates_at_line_boundary_and_credits_only_delivered_lines() {
    let fx = setup_fixture(SAMPLE_TEX, |_| {});
    let thm = fx.skeleton.nodes.iter().find(|n| n.label.as_deref() == Some("thm:main")).expect("thm");
    let (full_start, full_end) = (thm.spans[0].start_line, thm.spans[0].end_line);
    assert!(full_end > full_start + 3, "fixture theorem needs multiple lines");

    // Ultra-tight budget where even the first line + metadata doesn't fit: emits 0 blocks, truncated=true.
    let tiny = read_map_node(
        &fx.env, &fx.ledger, &fx.session,
        &json!({ "source_id": fx.source_id, "node": "thm:main", "include": ["statement", "proof"], "max_chars": 500 }),
    )
    .expect("tiny budget read_map_node");
    assert_eq!(tiny["truncated"].as_bool(), Some(true));
    assert!(tiny["files"].as_array().expect("files").is_empty());

    // Mid-statement truncation budget: fits partial statement lines cleanly and omits the proof block.
    let budget = 820usize;
    let out = read_map_node(
        &fx.env, &fx.ledger, &fx.session,
        &json!({ "source_id": fx.source_id, "node": "thm:main", "include": ["statement", "proof"], "max_chars": budget }),
    )
    .expect("truncated read_map_node");

    let canon_len = tools::canon::canonical_json(&out).chars().count();
    assert!(canon_len <= budget, "canonical JSON {canon_len} exceeded budget {budget}");
    assert_eq!(out["truncated"].as_bool(), Some(true));

    let files = out["files"].as_array().expect("files");
    assert_eq!(files.len(), 1, "proof block must not be emitted when budget is exhausted mid-statement");
    let delivered_start = files[0]["start_line"].as_u64().expect("start") as usize;
    let delivered_end = files[0]["end_line"].as_u64().expect("end") as usize;
    assert_eq!(delivered_start, full_start);
    assert!(delivered_end >= delivered_start && delivered_end < full_end);
    assert_eq!(files[0]["line_truncated"].as_bool(), Some(false));
    assert_eq!(files[0]["text"].as_str().expect("text").lines().count(), delivered_end - delivered_start + 1);

    let blob_hash = fx.ledger.put_blob(&serde_json::to_vec(&out).expect("ser")).expect("blob");
    fx.ledger.append(&fx.session, "tool.request", 1, &json!({ "call_id": "c-ok", "caller": "agent:inv:1", "tool": READ_MAP_NODE_TOOL, "run_id": "run-1" })).expect("req ok");
    fx.ledger.append(&fx.session, "tool.result", 1, &json!({ "call_id": "c-ok", "ok": true, "result": { "blob": blob_hash } })).expect("res ok");
    fx.ledger.append(&fx.session, "tool.request", 1, &json!({ "call_id": "c-fail", "caller": "agent:inv:fail", "tool": READ_MAP_NODE_TOOL, "run_id": "run-1" })).expect("req fail");
    fx.ledger.append(&fx.session, "tool.result", 1, &json!({ "call_id": "c-fail", "ok": false, "result": out })).expect("res fail");

    let (_, events) = fx.ledger.read_verified_session(&fx.session).expect("events");
    let sent = lines_sent_with_ledger(&events, Some(&fx.ledger), Some("run-1"));
    assert!(sent.covers("agent:inv:1", &fx.source_id, &fx.primary_path, &fx.file_sha256, delivered_start, delivered_end));
    assert!(!sent.covers("agent:inv:1", &fx.source_id, &fx.primary_path, &fx.file_sha256, delivered_end + 1, delivered_end + 1));
    assert!(!sent.covers("agent:inv:fail", &fx.source_id, &fx.primary_path, &fx.file_sha256, delivered_start, delivered_end));
    assert!(!sent.covers("worker:other", &fx.source_id, &fx.primary_path, &fx.file_sha256, delivered_start, delivered_end));
}

#[test]
fn pinned_skeleton_seq_is_honored_during_active_task() {
    let fx = setup_fixture(SAMPLE_TEX, |skel| {
        skel.nodes.iter_mut().find(|n| n.label.as_deref() == Some("thm:main")).expect("thm").title =
            Some("Pinned Revision Title".into());
    });
    let seq_v1 = fx.skeleton_seq;
    let task_seq = fx.ledger.append(
        &fx.session, MARKER_TASK_REQUESTED, 1,
        &json!({ "task": "orient", "source_id": fx.source_id, "skeleton_seq": seq_v1 }),
    ).expect("task_requested");

    let mut skel_v2 = fx.skeleton.clone();
    skel_v2.nodes.iter_mut().find(|n| n.label.as_deref() == Some("thm:main")).expect("thm").title =
        Some("Newer Revision Title".into());
    let (seq_v2, _) = append_skeleton(&fx.ledger, &fx.session, &skel_v2);
    assert!(seq_v2 > seq_v1);

    let during = read_map_node(&fx.env, &fx.ledger, &fx.session, &json!({ "source_id": fx.source_id, "node": "thm:main" }))
        .expect("read pinned");
    assert_eq!(during["skeleton_seq"].as_u64(), Some(seq_v1));
    assert_eq!(during["title"].as_str(), Some("Pinned Revision Title"));

    let (_, events_mid) = fx.ledger.read_verified_session(&fx.session).expect("events");
    let (loaded_seq, _, loaded_skel) = load_pinned_skeleton(&fx.ledger, &events_mid, &fx.source_id, Some(seq_v2)).expect("explicit seq");
    assert_eq!(loaded_seq, seq_v2);
    assert_eq!(loaded_skel.source_id, fx.source_id);

    fx.ledger.append(&fx.session, MARKER_TASK_FINISHED, 1, &json!({ "request_seq": task_seq, "status": "done" })).expect("finish");
    let after = read_map_node(&fx.env, &fx.ledger, &fx.session, &json!({ "source_id": fx.source_id, "node": "thm:main" }))
        .expect("read newest after finish");
    assert_eq!(after["skeleton_seq"].as_u64(), Some(seq_v2));
    assert_eq!(after["title"].as_str(), Some("Newer Revision Title"));
}

#[test]
fn tampered_skeleton_blob_and_tampered_source_file_fail_closed() {
    let fx = setup_fixture(SAMPLE_TEX, |_| {});
    let blob_path = fx.data_dir.join("blobs").join(&fx.blob_hash[..2]).join(&fx.blob_hash);
    let orig_blob = std::fs::read(&blob_path).expect("read orig blob");

    std::fs::write(&blob_path, b"{\"tampered\":true}").expect("tamper blob");
    let blob_err = read_map_node(&fx.env, &fx.ledger, &fx.session, &json!({ "source_id": fx.source_id, "node": "thm:main" }))
        .expect_err("tampered skeleton blob must fail closed");
    assert!(blob_err.contains("blob") || blob_err.contains("digest"), "got: {blob_err}");

    std::fs::write(&blob_path, &orig_blob).expect("restore blob");
    assert!(read_map_node(&fx.env, &fx.ledger, &fx.session, &json!({ "source_id": fx.source_id, "node": "thm:main" })).is_ok());

    let src_path = fx.data_dir.join("sessions").join(&fx.session).join("sources").join(&fx.primary_path);
    std::fs::write(&src_path, b"\\begin{document}tampered\\end{document}\n").expect("tamper source");
    let file_err = read_map_node(&fx.env, &fx.ledger, &fx.session, &json!({ "source_id": fx.source_id, "node": "thm:main", "max_chars": 500 }))
        .expect_err("tampered source file must fail closed");
    assert!(file_err.contains("changed") || file_err.contains("digest") || file_err.contains("outside"), "got: {file_err}");
}

#[test]
fn bare_normalized_number_single_match_and_ambiguity_without_tie_break() {
    let fx = setup_fixture(SAMPLE_TEX, |skel| {
        let base = skel.nodes.iter().find(|n| n.label.as_deref() == Some("sec:setup")).expect("sec").clone();
        let mut sub = base.clone();
        sub.tag = Some("n-roman-ia".into());
        sub.label = Some("sec:ia".into());
        sub.number = Some("I.A".into());
        sub.title = Some("Subpart".into());
        sub.order = skel.nodes.len();
        skel.nodes.push(sub);
        // Untagged node with same bare number must be ignored by bare-number fallback.
        let mut untagged = base;
        untagged.tag = None;
        untagged.label = None;
        untagged.number = Some("I.A".into());
        untagged.order = skel.nodes.len();
        skel.nodes.push(untagged);
    });
    let thm_tag = node_tag_by_label(&fx.skeleton, "thm:main");

    // Unique bare printed numbers resolve cleanly (including parenthesized and case-folded forms).
    for (q, expected) in [("1.2", thm_tag.as_str()), ("(1.2)", thm_tag.as_str()), ("i.a", "n-roman-ia"), ("(I.A)", "n-roman-ia")] {
        let out = read_map_node(&fx.env, &fx.ledger, &fx.session, &json!({ "source_id": fx.source_id, "node": q }))
            .unwrap_or_else(|e| panic!("bare query `{q}` failed: {e}"));
        assert_eq!(out["tag"].as_str(), Some(expected), "bare query `{q}`");
    }

    // Ambiguous bare number ("1" matches Section 1, Definition 1, Equation 1) must error with NO silent tie-break.
    let amb = read_map_node(&fx.env, &fx.ledger, &fx.session, &json!({ "source_id": fx.source_id, "node": "1" }))
        .expect_err("ambiguous bare number must error");
    assert!(amb.contains("Ambiguous"), "got: {amb}");
    assert!(amb.contains("sec:setup") && amb.contains("def:lat") && amb.contains("eq:energy"), "got: {amb}");
    let amb_paren = read_map_node(&fx.env, &fx.ledger, &fx.session, &json!({ "source_id": fx.source_id, "node": "(1)" }))
        .expect_err("parenthesized ambiguous bare number must error");
    assert!(amb_paren.contains("Ambiguous"), "got: {amb_paren}");

    // Cap ambiguous bare-number list at MAX_SUGGESTED_NODES (8) when >8 tagged nodes collide.
    let fx_many = setup_fixture(SAMPLE_TEX, |skel| {
        let base = skel.nodes.iter().find(|n| n.label.as_deref() == Some("thm:main")).expect("thm").clone();
        for i in 0..10 {
            let mut n = base.clone();
            n.tag = Some(format!("n-dup-bare-{i}"));
            n.label = Some(format!("lbl:dup-{i}"));
            n.number = Some("77".into());
            n.order = skel.nodes.len();
            skel.nodes.push(n);
        }
    });
    let amb_many = read_map_node(&fx_many.env, &fx_many.ledger, &fx_many.session, &json!({ "source_id": fx_many.source_id, "node": "77" }))
        .expect_err("10 colliding bare numbers must error");
    assert!(amb_many.contains("Ambiguous") && amb_many.contains("matches 8 nodes:"), "got: {amb_many}");
    assert_eq!(amb_many.split("; ").count(), 8, "got: {amb_many}");
}

#[test]
fn not_found_ranks_dot_prefix_ahead_of_roman_ii_iii_and_truncates_cleanly() {
    let fx = setup_fixture(SAMPLE_TEX, |skel| {
        let base = skel.nodes.iter().find(|n| n.label.as_deref() == Some("sec:setup")).expect("sec").clone();
        // Place II and III BEFORE I.A in document order to prove prefix ranking beats document order.
        let extras = [
            ("n-sec-ii", "II", "Methods", 0usize),
            ("n-sec-iii", "III", "Results", 1usize),
            ("n-sec-iv", "IV", "Discussion", 2usize),
            ("n-sec-v", "V", "Outlook", 3usize),
            ("n-sec-ia", "I.A", "Background", 50usize),
            ("n-sec-ib", "I.B", "Overview", 51usize),
        ];
        for n in &mut skel.nodes {
            n.order += 10;
        }
        for (tag, num, title, order) in extras {
            let mut n = base.clone();
            n.kind = SkNodeKind::Section;
            n.tag = Some(tag.into());
            n.label = None;
            n.number = Some(num.into());
            n.title = Some(title.into());
            n.order = order;
            skel.nodes.push(n);
        }
    });

    let total_tagged = fx.skeleton.nodes.iter().filter(|n| n.tag.is_some()).count();
    assert!(total_tagged > 8, "fixture must exceed MAX_SUGGESTED_NODES");

    let err = read_map_node(&fx.env, &fx.ledger, &fx.session, &json!({ "source_id": fx.source_id, "node": "I" }))
        .expect_err("missing bare I must error");
    let after = err.split("Available nodes: ").nth(1).expect("available nodes section");
    assert!(after.split("; ").count() <= 8, "truncation suffix must not add extra '; ' separators: {after}");
    let pos_ia = after.find("n-sec-ia").expect("I.A must appear in suggestions");
    let pos_ib = after.find("n-sec-ib").expect("I.B must appear in suggestions");
    let pos_ii = after.find("n-sec-ii").expect("II must appear in suggestions");
    let pos_iii = after.find("n-sec-iii").expect("III must appear in suggestions");
    assert!(pos_ia < pos_ii && pos_ia < pos_iii, "I.A must rank ahead of II/III: {after}");
    assert!(pos_ib < pos_ii && pos_ib < pos_iii, "I.B must rank ahead of II/III: {after}");

    let expected_suffix = format!(
        ". Showing 8 of {total_tagged} tagged nodes. Pass a tag, TeX label, or printed reference such as `Section III` or `Eq. (4)`."
    );
    assert!(err.ends_with(&expected_suffix), "expected suffix `{expected_suffix}`, got: {err}");
}

