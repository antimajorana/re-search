//! Rule R2 marker test (package F5, `00-overview.md` 3.1 row 3):
//!
//! Host-only mode markers (`investigation.started`, `investigation.handoff`,
//! `idea.started`, `solve.started`) may be written ONLY by the host, never by any
//! registered tool handler.
//!
//! This test verifies rule R2 at three levels:
//! 1. Exhaustive registry enumeration across all built tool profiles, MCP tools,
//!    Lean router, and isolated paper-reader router, failing if any registered tool
//!    is missing from the emitted-event-kind inventory or declares a mode marker.
//! 2. Runtime execution of locally runnable ledger-writing tool handlers through a
//!    real `Router`, verifying no emitted ledger event is a mode marker.
//! 3. Source audit of ledger append calls in every tool handler file.

use desktop_server::session_mode::HOST_ONLY_MODE_MARKERS;
use ledger::Ledger;
use research_tools::tool_profile::{
    built_profiles, register_profile, research_v1, ProfileHostTools,
};
use research_tools::{
    register_lean_tool, register_reader_tools, research_policy, ResearchEnv,
    EVIDENCE_EVENT, READER_CALLER_PREFIX,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use tools::{
    ApprovalReply, ApprovalRequest, Approver, CallCtx, CallOutcome, Registry, Router,
    ToolHandler, WorkerEpoch,
};

struct NoWorkerEpoch;
impl WorkerEpoch for NoWorkerEpoch {
    fn live_epoch(&self) -> Option<u64> {
        None
    }
    fn is_contained(&self) -> bool {
        false
    }
}

struct TestApproveAll;
impl Approver for TestApproveAll {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        ApprovalReply {
            approved: true,
            digest: req.digest.clone(),
            by: "mode-markers-r2-test".into(),
        }
    }
}

/// Complete inventory of ledger event kinds emitted directly by every registered tool handler
/// (excluding Router's own `tool.request`, `policy.decision`, `approval.*`, and `tool.result`
/// envelope events).
///
/// Any newly registered tool MUST be added here; the test fails if the registered tool set
/// diverges from this map or if any tool emits a host-only mode marker.
fn tool_emitted_event_kinds() -> BTreeMap<&'static str, &'static [&'static str]> {
    BTreeMap::from([
        ("arxiv_search", &["literature.fetched"][..]),
        ("check_expression_sanity", &[][..]),
        ("check_run", &[][..]),
        ("doc_ingest", &["source.ingested"][..]),
        ("expr_show", &[][..]),
        ("fs_list", &[][..]),
        ("fs_read", &[][..]),
        ("fs_write", &["workspace.backup"][..]),
        ("latex_compile", &[][..]),
        ("latex_patch", &["workspace.backup"][..]),
        ("lean_check", &[][..]),
        ("ledger_append", &["proof.entry"][..]),
        ("ledger_note", &["note"][..]),
        ("ledger_query", &[][..]),
        ("ledger_supersede", &["proof.supersede"][..]),
        ("literature_search", &["literature.fetched"][..]),
        ("note_evidence", &[EVIDENCE_EVENT][..]),
        ("read_paper_lines", &[][..]),
        ("record_paper_branch", &["investigation.branch"][..]),
        ("record_paper_conventions", &["investigation.conventions"][..]),
        ("record_paper_map", &["investigation.map"][..]),
        ("record_paper_summary", &["investigation.summary"][..]),
        ("search_formal_library", &[][..]),
        ("search_paper_passages", &[][..]),
        ("set_session_title", &[tools::builtin::SESSION_TITLE_EVENT][..]),
        ("solve_finish", &[][..]),
        ("step_record", &[][..]),
        ("sympy_eval", &[][..]),
        ("tex_audit", &[][..]),
    ])
}

fn collect_all_registered_tool_ids() -> BTreeSet<String> {
    let tmp = TempDir::new().unwrap();
    let env = Arc::new(ResearchEnv::new(tmp.path()));
    let mut ids = BTreeSet::new();

    // Every built profile (including MCP `set_session_title`).
    for profile in built_profiles() {
        for def in &profile.defs {
            ids.insert(def.id.clone());
        }
        for def in profile.mcp_defs() {
            ids.insert(def.id.clone());
        }
        let mut reg = Registry::new();
        let noop_handler: Arc<dyn ToolHandler> = Arc::new(|_: &CallCtx<'_>, _: &Value| Ok(json!({})));
        let noop_title: Arc<dyn ToolHandler> = Arc::new(|_: &CallCtx<'_>, _: &Value| Ok(json!({})));
        register_profile(
            &profile,
            &mut reg,
            Arc::clone(&env),
            ProfileHostTools {
                sympy_epoch: Arc::new(NoWorkerEpoch),
                sympy_handler: noop_handler,
                session_title_handler: Some(noop_title),
                solve_check: None,
            },
        )
        .unwrap();
        for def in reg.defs() {
            ids.insert(def.id.clone());
        }
    }

    // Separate Lean router.
    let mut lean_reg = Registry::new();
    register_lean_tool(&mut lean_reg, Arc::clone(&env));
    for def in lean_reg.defs() {
        ids.insert(def.id.clone());
    }

    // Separate isolated paper-reader router (`note_evidence` + read-only paper tools).
    let mut reader_reg = Registry::new();
    register_reader_tools(&mut reader_reg, env);
    for def in reader_reg.defs() {
        ids.insert(def.id.clone());
    }

    ids
}

#[test]
fn every_registered_tool_emits_no_host_mode_marker() {
    let registered = collect_all_registered_tool_ids();
    let inventory = tool_emitted_event_kinds();
    let declared: BTreeSet<String> = inventory.keys().map(|k| (*k).to_string()).collect();

    assert_eq!(
        registered, declared,
        "registered tool set and R2 emitted-event inventory differ; update inventory and audit new tools"
    );

    let markers: BTreeSet<&str> = HOST_ONLY_MODE_MARKERS.iter().copied().collect();
    for (tool_id, emitted) in &inventory {
        for kind in *emitted {
            assert!(
                !markers.contains(kind),
                "rule R2 violation: registered tool `{tool_id}` declares host-only mode marker `{kind}`"
            );
        }
    }
}

#[test]
fn locally_executed_tool_handlers_emit_no_host_mode_markers() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let ws_dir = tmp.path().join("ws");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&ws_dir).unwrap();
    let paper_file = ws_dir.join("paper.tex");
    std::fs::write(
        &paper_file,
        "\\documentclass{article}\n\\begin{document}\nAssume x > 0.\nTherefore x^2 > 0.\n\\end{document}\n",
    )
    .unwrap();

    let sid = "s-r2-runtime";
    let env = Arc::new(ResearchEnv::with_workspace(data_dir.clone(), &ws_dir).unwrap());
    let mut reg = Registry::new();
    let noop_sympy: Arc<dyn ToolHandler> =
        Arc::new(|_: &CallCtx<'_>, _: &Value| Ok(json!({ "ok": true })));
    register_profile(
        &research_v1(),
        &mut reg,
        Arc::clone(&env),
        ProfileHostTools {
            sympy_epoch: Arc::new(NoWorkerEpoch),
            sympy_handler: noop_sympy,
            session_title_handler: None,
            solve_check: None,
        },
    )
    .unwrap();

    // Seed host-written investigation.started + request marker so investigation recorders can run.
    let ledger = Ledger::open(&data_dir).unwrap();
    ledger
        .append_new_session(sid, &[("investigation.started", 1, &json!({}))])
        .unwrap();

    let approver = Arc::new(TestApproveAll);
    let router = Router::new(reg, research_policy(), Ledger::open(&data_dir).unwrap(), sid, approver);
    let caller = "agent:test:s-r2-runtime";
    let baseline_len = ledger.read_session(sid).unwrap().len();

    // 1. ledger_note
    assert!(matches!(
        router.try_call(caller, "ledger_note", &json!({ "text": "Synthetic note" })).unwrap(),
        CallOutcome::Ok(_)
    ));
    // 2. ledger_append + ledger_supersede
    let id1 = match router
        .try_call(
            caller,
            "ledger_append",
            &json!({
                "entry_type": "assumption",
                "summary": "Positive x",
                "details": "Assume x > 0.",
                "depends_on": []
            }),
        )
        .unwrap()
    {
        CallOutcome::Ok(v) => v["entry_id"].as_str().unwrap().to_string(),
        other => panic!("ledger_append 1 failed: {other:?}"),
    };
    let id2 = match router
        .try_call(
            caller,
            "ledger_append",
            &json!({
                "entry_type": "correction",
                "summary": "Refined x",
                "details": "Assume x > 1.",
                "depends_on": []
            }),
        )
        .unwrap()
    {
        CallOutcome::Ok(v) => v["entry_id"].as_str().unwrap().to_string(),
        other => panic!("ledger_append 2 failed: {other:?}"),
    };
    let sup = router
        .try_call(
            caller,
            "ledger_supersede",
            &json!({ "entry_id": id1, "replaced_by": id2, "reason": "tightened" }),
        )
        .unwrap();
    assert!(matches!(sup, CallOutcome::Ok(_)), "ledger_supersede failed: {sup:?}");
    // 3. doc_ingest
    let ingest_out = router
        .try_call(caller, "doc_ingest", &json!({ "path": "paper.tex" }))
        .unwrap();
    let source_id = match ingest_out {
        CallOutcome::Ok(v) => v["source_id"].as_str().unwrap().to_string(),
        other => panic!("doc_ingest failed: {other:?}"),
    };
    // 4. fs_write + latex_patch
    assert!(matches!(
        router
            .try_call(
                caller,
                "fs_write",
                &json!({ "path": "draft.tex", "content": "alpha = 1\n" })
            )
            .unwrap(),
        CallOutcome::Ok(_)
    ));
    assert!(matches!(
        router
            .try_call(
                caller,
                "latex_patch",
                &json!({
                    "path": "draft.tex",
                    "search": "alpha = 1",
                    "replace": "alpha = 2"
                })
            )
            .unwrap(),
        CallOutcome::Ok(_)
    ));
    // 5. Investigation recorders (`record_paper_map`, `record_paper_summary`)
    ledger
        .append_with_hash(
            sid,
            "investigation.paper",
            1,
            &json!({ "source_id": source_id, "question": "" }),
        )
        .unwrap();
    ledger
        .append_with_hash(
            sid,
            "investigation.mapping_requested",
            1,
            &json!({ "source_id": source_id }),
        )
        .unwrap();
    let anchor = json!({
        "source_id": source_id,
        "start_line": 3,
        "end_line": 3,
        "quote": "Assume x > 0.",
        "kind": "passage"
    });
    let map_out = router
        .try_call(
            caller,
            "record_paper_map",
            &json!({
                "source_id": source_id,
                "title": "Synthetic Paper",
                "coverage": "paper",
                "limitations": [],
                "nodes": [{
                    "id": "a",
                    "sentence": "Assume x > 0.",
                    "kind": "assumption",
                    "main": true,
                    "anchors": [anchor]
                }],
                "edges": []
            }),
        )
        .unwrap();
    assert!(matches!(map_out, CallOutcome::Ok(_)), "record_paper_map failed: {map_out:?}");
    // 6. Reader tool `note_evidence` on isolated reader router
    let mut reader_reg = Registry::new();
    register_reader_tools(&mut reader_reg, env);
    let reader_router = Router::new(
        reader_reg,
        research_policy(),
        Ledger::open(&data_dir).unwrap(),
        sid,
        Arc::new(TestApproveAll),
    );
    let reader_caller = format!("{READER_CALLER_PREFIX}1:{sid}");
    let ev_out = reader_router
        .try_call(
            &reader_caller,
            "note_evidence",
            &json!({
                "items": [{
                    "source_id": source_id,
                    "start_line": 4,
                    "end_line": 4,
                    "quote": "Therefore x^2 > 0.",
                    "kind": "passage",
                    "role": "result",
                    "gist": "Positivity of x^2."
                }]
            }),
        )
        .unwrap();
    assert!(matches!(ev_out, CallOutcome::Ok(_)), "note_evidence failed: {ev_out:?}");

    // Verify zero host-only mode markers were appended after our initial host marker.
    let events = ledger.read_session(sid).unwrap();
    assert!(events.len() > baseline_len + 8);
    let markers: BTreeSet<&str> = HOST_ONLY_MODE_MARKERS.iter().copied().collect();
    for ev in &events[baseline_len..] {
        assert!(
            !markers.contains(ev.kind.as_str()),
            "rule R2 violation: tool execution appended mode marker `{}` at seq {}",
            ev.kind,
            ev.seq
        );
    }
}

#[test]
fn tool_handler_sources_do_not_append_host_mode_markers() {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();

    let tool_files = [
        "crates/tools/src/builtin.rs",
        "crates/cli-route/src/core_tools.rs",
        "crates/research-tools/src/arxiv.rs",
        "crates/research-tools/src/literature.rs",
        "crates/research-tools/src/fs_tools.rs",
        "crates/research-tools/src/write_tools.rs",
        "crates/research-tools/src/ingest.rs",
        "crates/research-tools/src/latex_compile.rs",
        "crates/research-tools/src/tex_audit.rs",
        "crates/research-tools/src/formal_library.rs",
        "crates/research-tools/src/paper_lines.rs",
        "crates/research-tools/src/paper_passages.rs",
        "crates/research-tools/src/investigation.rs",
        "crates/research-tools/src/investigation_evidence.rs",
        "crates/research-tools/src/proof_dag.rs",
        "crates/research-tools/src/sanity.rs",
        "crates/research-tools/src/lean_check.rs",
        "crates/research-tools/src/solve_defs.rs",
        "crates/research-tools/src/solve_exprs.rs",
        "crates/research-tools/src/solve_exprs_show.rs",
    ];

    for rel in tool_files {
        let full = workspace_root.join(rel);
        let src = std::fs::read_to_string(&full)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", full.display()));
        // Inspect only production code before `#[cfg(test)]`.
        let prod = src.split("#[cfg(test)]").next().unwrap_or(&src);
        let lines: Vec<&str> = prod.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if line.contains(".append(")
                || line.contains(".append_with_hash(")
                || line.contains(".append_at_head(")
                || line.contains(".append_new_session(")
            {
                let window_end = (i + 8).min(lines.len());
                let snippet = lines[i..window_end].join("\n");
                for marker in HOST_ONLY_MODE_MARKERS {
                    assert!(
                        !snippet.contains(&format!("\"{marker}\"")),
                        "rule R2 violation: {rel}:{} appends host-only mode marker `{marker}`",
                        i + 1
                    );
                }
            }
        }
    }
}
