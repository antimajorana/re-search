//! Integration tests for F6 (`tex_audit`, `<deterministic_tex_audit>`,
//! and `repair_under_escaped_latex` / `repair_json_value_latex`).

use ledger::Ledger;
use research_tools::{
    is_likely_tsv_line, register_research_tools, repair_json_value_latex,
    repair_under_escaped_latex, research_policy, run_tex_audit, should_skip_json_key,
    AuditOptions, AuditSeverity, ResearchEnv, SandboxRoot, WorkspaceSandbox, TEX_AUDIT,
};
use serde_json::{json, Value};
use std::fs;
use std::sync::Arc;
use tempfile::TempDir;
use tools::approval::{ApprovalReply, ApprovalRequest, Approver};
use tools::router::{CallOutcome, Router};
use tools::Registry;

struct NoAskApprover;
impl Approver for NoAskApprover {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        panic!("tex_audit is ReadWorkspace and must not ask: {}", req.tool_id);
    }
}

#[test]
fn f6_latex_comment_parity_labels_multirefs_and_bib_entries() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ws");
    let data = tmp.path().join("data");
    fs::create_dir_all(&ws).unwrap();

    let session = "s-audit01";
    let env = Arc::new(ResearchEnv::with_workspace(&data, &ws).unwrap());

    // Populate a label inside session `sources/` to verify cross-root label indexing!
    let src_write = env
        .sandbox()
        .resolve_for_write(SandboxRoot::Sources, session, "paper1/source.tex")
        .unwrap();
    WorkspaceSandbox::atomic_write_in_root(
        &src_write.canonical_root,
        &src_write.rel_parts,
        b"\\section{Source Theorem}\\label{thm:source_ok}\n\
          We prove that the ground-state spectral gap satisfies Delta >= 1/2 uniformly in N.\n",
    )
    .unwrap();

    // Workspace .bib with @article, @book(...), plus ignored @comment, @preamble, @string.
    let bib = r#"
@string{PRL = "Phys. Rev. Lett."}
@preamble{"\newcommand{\noopsort}[1]{}"}
@comment{fake_comment_key, author = "Nobody"}
% @article{commented_out_key, title = "Ignored"}
@article{kitaev2001,
  author = {Kitaev, A. Yu.},
  title = {Unpaired Majorana fermions in quantum wires},
  eprint = {cond-mat/0010440},
  year = {2001}
}
@book(sakurai2011,
  author = {Sakurai, J. J.},
  title = {Modern Quantum Mechanics},
  year = {2011}
)
"#;
    fs::write(ws.join("refs.bib"), bib).unwrap();

    // Workspace manuscript testing:
    // - `\%` (literal percent: active \label{eq:real} after it!) vs `% \label{eq:commented}`
    // - `\\%` (tabular line break `\\` + `%` comment: \ref{eq:after_linebreak_comment} MUST be ignored!)
    // - `\\\%` (`\\` + `\%` literal percent: \ref{eq:missing_after_escaped_pct} MUST be flagged!)
    // - `\verb|100% raw|` followed by `\ref{eq:missing_after_verb}` (MUST be flagged!)
    // - Multi-refs `\eqref{eq:real, eq:missing_multi}`, `\cref{thm:source_ok, thm:missing_cref}`,
    //   `\autoref{sec:missing_auto}`, `\pageref{p:missing_page}`
    // - Citations `\citep[see][p.~4]{kitaev2001, sakurai2011, fake_comment_key, missing_bib_key}`
    let draft = "\
\\documentclass{article}
\\begin{document}
Yield is 50\\% \\label{eq:real} % \\label{eq:commented}
Duplicate \\label{eq:real}
a & b \\\\% \\ref{eq:after_linebreak_comment}
c & d \\\\\\% \\ref{eq:missing_after_escaped_pct}
\\verb|100% raw| \\ref{eq:missing_after_verb}
See \\eqref{eq:real, eq:missing_multi} and \\cref{thm:source_ok, thm:missing_cref}.
Also \\autoref{sec:missing_auto} and \\pageref{p:missing_page}.
Citations: \\citep[see][p.~4]{kitaev2001, sakurai2011, fake_comment_key, missing_bib_key}.
\\end{document}
";
    fs::write(ws.join("draft.tex"), draft).unwrap();

    let report = run_tex_audit(&env, session, None, &AuditOptions::default()).unwrap();
    assert!(!report.ok);
    assert_eq!(report.status, "error");
    assert_eq!(report.warning_count, 1, "expected 1 duplicate_label warning");

    let undef_targets: Vec<&str> = report
        .issues
        .iter()
        .filter(|i| i.kind == "undefined_ref")
        .map(|i| i.target.as_str())
        .collect();
    assert!(undef_targets.contains(&"eq:missing_after_escaped_pct"));
    assert!(undef_targets.contains(&"eq:missing_after_verb"));
    assert!(undef_targets.contains(&"eq:missing_multi"));
    assert!(undef_targets.contains(&"thm:missing_cref"));
    assert!(undef_targets.contains(&"sec:missing_auto"));
    assert!(undef_targets.contains(&"p:missing_page"));
    assert!(
        !undef_targets.contains(&"eq:after_linebreak_comment"),
        "\\\\% must start a comment after \\\\"
    );
    assert!(!undef_targets.contains(&"eq:real"));
    assert!(!undef_targets.contains(&"thm:source_ok"));

    let missing_cites: Vec<&str> = report
        .issues
        .iter()
        .filter(|i| i.kind == "missing_citation_key")
        .map(|i| i.target.as_str())
        .collect();
    assert_eq!(missing_cites, vec!["fake_comment_key", "missing_bib_key"]);
}

#[test]
fn audit_scopes_reference_labels_and_candidate_blockers() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("paper");
    let data = tmp.path().join("data");
    fs::create_dir_all(ws.join("sections")).unwrap();
    fs::write(ws.join("main.tex"), r"See \ref{used} and \ref{unrelated}. \input{sections/results}\bibliography{main}").unwrap();
    fs::write(ws.join("sections/results.tex"), r"\label{used}\cite{valid}").unwrap();
    fs::write(ws.join("main.bib"), "@article{valid, title={A relevant source}}").unwrap();
    fs::write(ws.join("notes.tex"), r"\label{unrelated} arXiv:2311.00001v2").unwrap();
    let env = ResearchEnv::with_workspace(data, ws).unwrap();
    let opts = AuditOptions {
        target_path: Some("main.tex".into()),
        withdrawn_arxiv_ids: vec!["2311.00001v2".into()],
        ..Default::default()
    };
    let report = run_tex_audit(&env, "s-scope", None, &opts).unwrap();
    assert_eq!(report.error_count, 1, "{:?}", report.issues);
    assert_eq!(report.issues[0].target, "unrelated");
    assert_eq!(report.blocker_count, 0);

    let draft = AuditOptions {
        candidate_texts: vec![(
            "main.tex".into(),
            r"See \ref{used}. \input{sections/results}\bibliography{main}".into(),
        )],
        ..opts.clone()
    };
    let report = run_tex_audit(&env, "s-scope", None, &draft).unwrap();
    assert!(report.ok, "{:?}", report.issues);

    let candidate = AuditOptions {
        candidate_only: true,
        candidate_texts: vec![("answer".into(), "An unrelated calculation gives 4.".into())],
        target_path: None,
        ..opts
    };
    let report = run_tex_audit(&env, "s-scope", None, &candidate).unwrap();
    assert!(report.ok, "{:?}", report.issues);
}

#[test]
fn f6_quotes_no_self_match_and_withdrawn_arxiv_retracted_doi_blockers() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ws");
    let data = tmp.path().join("data");
    fs::create_dir_all(&ws).unwrap();

    let session = "s-blockers01";
    let env = Arc::new(ResearchEnv::with_workspace(&data, &ws).unwrap());

    // Source file containing an authentic quote.
    let src_write = env
        .sandbox()
        .resolve_for_write(SandboxRoot::Sources, session, "src1/paper.tex")
        .unwrap();
    WorkspaceSandbox::atomic_write_in_root(
        &src_write.canonical_root,
        &src_write.rel_parts,
        b"The topological invariant Z_2 remains quantized under arbitrary weak disorder.\n",
    )
    .unwrap();

    // Workspace .bib referencing base arXiv ID v1 (`2311.00001v1`) and mixed-case DOI.
    let bib = r#"
@article{bad_arxiv_v1,
  author = {Doe, J.},
  title = {Flawed Lemma},
  eprint = {2311.00001v1},
  year = {2023}
}
@article{bad_hepth,
  author = {Roe, R.},
  eprint = {HEP-TH/9901001v1},
  year = {1999}
}
@article{bad_doi_paper,
  author = {Smith, A.},
  doi = {10.1103/Bad.2024},
  year = {2024}
}
"#;
    fs::write(ws.join("refs.bib"), bib).unwrap();

    // Manuscript has:
    // - Citations to `bad_arxiv_v1` (while v2 is withdrawn in session ledger!), `bad_hepth`, and `bad_doi_paper`
    // - Inline URL `https://doi.org/10.1103/bad.2024` and inline `arXiv:2311.00001`
    // - Inline `\begin{quote}...\end{quote}` that ONLY exists in `ms.tex` itself (must NOT self-match!)
    let ms = "\
\\documentclass{article}
\\begin{document}
We rely on \\cite{bad_arxiv_v1, bad_hepth} and \\citet{bad_doi_paper} (see https://doi.org/10.1103/bad.2024 & R).
\\begin{quote}
This fabricated quote appears nowhere in any uploaded or arXiv source file at all.
\\end{quote}
\\end{document}
";
    fs::write(ws.join("ms.tex"), ms).unwrap();

    // Record withdrawn `2311.00001v2`, withdrawn `hep-th/9901001v2`, and retracted `10.1103/BAD.2024` in session ledger!
    {
        let ledger = Ledger::open(&data).unwrap();
        ledger
            .append(
                session,
                "tool.result",
                1,
                &json!({
                    "call_id": "c1",
                    "ok": true,
                    "result": {
                        "inline": serde_json::to_string(&json!({
                            "papers": [
                                { "arxiv_id": "2311.00001v2", "withdrawn": true },
                                { "arxiv_id": "hep-th/9901001v2", "withdrawn": true },
                                { "DOI": "10.1103/BAD.2024", "retracted": true }
                            ]
                        })).unwrap()
                    }
                }),
            )
            .unwrap();
    }

    let mut reg = Registry::new();
    register_research_tools(&mut reg, env.clone());
    let router = Router::new(
        reg,
        research_policy(),
        Ledger::open(&data).unwrap(),
        session,
        Arc::new(NoAskApprover),
    );

    let res = router
        .try_call(
            "agent:verifier",
            TEX_AUDIT,
            &json!({
                "quotes": [
                    "The topological invariant   Z_2\n\tremains quantized under arbitrary weak disorder.",
                    "The topological invariant Z_2 BREAKS under arbitrary weak disorder."
                ]
            }),
        )
        .unwrap();

    let v: Value = match res {
        CallOutcome::Ok(val) => val,
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(v["ok"], false);
    assert_eq!(v["status"], "blocker");
    assert!(v["blocker_count"].as_u64().unwrap() >= 3);

    let issues = v["issues"].as_array().unwrap();
    assert!(issues.iter().any(|i| i["severity"] == "blocker" && i["kind"] == "withdrawn_arxiv" && i["target"] == "2311.00001"));
    assert!(issues.iter().any(|i| i["severity"] == "blocker" && i["kind"] == "withdrawn_arxiv" && i["target"] == "hep-th/9901001"));
    assert!(issues.iter().any(|i| i["severity"] == "blocker" && i["kind"] == "retracted_doi" && i["target"] == "10.1103/bad.2024"));

    // Verify fabricated inline quote + altered explicit quote flagged, while whitespace-normalized authentic quote passed!
    let unverified: Vec<&Value> = issues.iter().filter(|i| i["kind"] == "unverified_quote").collect();
    assert_eq!(unverified.len(), 2, "expected inline self-quote + altered quote to fail, authentic quote to pass");

    let xml = v["deterministic_xml"].as_str().unwrap();
    assert!(xml.starts_with("<deterministic_tex_audit ok=\"false\" status=\"blocker\""));
    assert!(xml.contains("severity=\"blocker\""));
    assert!(xml.ends_with("</deterministic_tex_audit>"));

    // Also verify XML escaping of `<`, `>`, `&`, `"`.
    let clean_env = ResearchEnv::new(tmp.path().join("empty_data"));
    let mut cand_opts = AuditOptions::default();
    cand_opts
        .candidate_texts
        .push(("step<&>\"1".into(), "\\ref{eq:missing}".into()));
    let xml_escaped = research_tools::format_deterministic_tex_audit_xml(
        &run_tex_audit(&clean_env, "s-xml", None, &cand_opts).unwrap(),
    );
    assert!(xml_escaped.contains("file=\"step&lt;&amp;&gt;&quot;1\""));
    assert!(AuditSeverity::Blocker < AuditSeverity::Error && AuditSeverity::Error < AuditSeverity::Warning);
    assert_eq!(
        research_tools::tex_scan::normalize_doi("https://doi.org/10.1103%2FBad.2024").as_deref(),
        Some("10.1103/bad.2024")
    );
}

#[test]
fn f6_repair_under_escaped_latex_crlf_tsv_fences_and_json_keys() {
    // 1. Repairs \f (\x0c), \b (\x08), \r (\x0d), \t (\x09), \a (\x07) LaTeX stems.
    let corrupted = "\x0crac{1}{2} + \x08eta + \x0dho + \x09heta + \x07lpha + \x09o \x0dightarrow \x0dfloor x \x0dceil";
    assert_eq!(
        repair_under_escaped_latex(corrupted),
        r"\frac{1}{2} + \beta + \rho + \theta + \alpha + \to \rightarrow \rfloor x \rceil"
    );

    // 2. Preserves CRLF (\r\n) byte-for-byte even when next line begins with `ho` (`\rho` stem)!
    assert_eq!(repair_under_escaped_latex("a\r\nho\r\n\x0dho"), "a\r\nho\r\n\\rho");

    // 3. TSV detection vs multi-tab corrupted LaTeX prose.
    assert!(is_likely_tsv_line("k\ttheta\tgap"));
    assert!(is_likely_tsv_line("id\t0.5"));
    assert!(!is_likely_tsv_line("Let \x09heta and \x09au be angles"));
    assert_eq!(repair_under_escaped_latex("k\ttheta\tgap\n0\t0.1\t0.5"), "k\ttheta\tgap\n0\t0.1\t0.5");
    assert_eq!(
        repair_under_escaped_latex("Let \x09heta and \x09au be angles"),
        r"Let \theta and \tau be angles"
    );

    // 4. Skips fenced code blocks (` ``` ` and `~~~`) while repairing surrounding prose.
    let with_fence = "Before \x0crac{a}{b}\n```python\nx = \"\x0crac\\t\x08eta\"\n```\nAfter \x08eta";
    assert_eq!(
        repair_under_escaped_latex(with_fence),
        "Before \\frac{a}{b}\n```python\nx = \"\x0crac\\t\x08eta\"\n```\nAfter \\beta"
    );

    // 5. JSON value walker repairs prose fields while skipping code, IDs, hashes, blobs, paths, URLs.
    for skipped in [
        "code", "python_code", "srepr", "entry_id", "callId", "raw_blob", "sha256_after", "path", "url",
    ] {
        assert!(should_skip_json_key(skipped), "expected `{skipped}` to be skipped");
    }
    let mut payload = json!({
        "summary": "Energy \x0crac{1}{2} and \x09heta",
        "details": "See \x08egin{align}\x07lpha\x08ullet\x0dho\\end{align}",
        "latex": "\x0crac{\x08eta}{\x0dho}",
        "code": "s = '\x0crac\t\x08eta'",
        "entry_id": "e-\x09heta",
        "callId": "c-\x08eta",
        "raw_blob": "\x0crac0000",
        "sha256_after": "\x08eta1111",
        "path": "dir/\x09heta.tex",
        "url": "https://example.com/\x0crac"
    });
    repair_json_value_latex(&mut payload);

    assert_eq!(payload["summary"], r"Energy \frac{1}{2} and \theta");
    assert_eq!(payload["details"], r"See \begin{align}\alpha\bullet\rho\end{align}");
    assert_eq!(payload["latex"], r"\frac{\beta}{\rho}");
    assert_eq!(payload["code"], "s = '\x0crac\t\x08eta'");
    assert_eq!(payload["entry_id"], "e-\x09heta");
    assert_eq!(payload["callId"], "c-\x08eta");
    assert_eq!(payload["raw_blob"], "\x0crac0000");
    assert_eq!(payload["sha256_after"], "\x08eta1111");
    assert_eq!(payload["path"], "dir/\x09heta.tex");
    assert_eq!(payload["url"], "https://example.com/\x0crac");
}
