//! Integration tests for F1 (`research_policy`, `ResearchEnv`), F2 (`WorkspaceSandbox`),
//! and F3 (`fs_list` & `fs_read`) routed end-to-end through `tools::Router::try_call`.

use ledger::Ledger;
use research_tools::{
    register_research_tools, research_policy, research_tool_defs, resolve_sources_session_id,
    strip_verbatim_prefix, strip_verbatim_str, validate_session_id, LiteratureEndpoints,
    ResearchEnv, SandboxRoot, FS_LIST, FS_READ, RESEARCH_POLICY_VERSION,
};
#[cfg(unix)]
use research_tools::SandboxError;
use serde_json::json;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;
use tools::approval::{ApprovalReply, ApprovalRequest, Approver};
use tools::router::{CallOutcome, Router};
use tools::Registry;

struct PanicOnAskApprover;
impl Approver for PanicOnAskApprover {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        panic!("ReadWorkspace tools must Allow without asking, got Ask for {}", req.tool_id);
    }
}

fn build_router(env: Arc<ResearchEnv>, session: &str) -> Router {
    let mut reg = Registry::new();
    register_research_tools(&mut reg, env.clone());
    let ledger = Ledger::open(&env.data_dir).expect("open ledger");
    Router::new(reg, research_policy(), ledger, session, Arc::new(PanicOnAskApprover))
}

#[test]
fn f1_tool_defs_endpoints_and_router_policy_contract() {
    let defs = research_tool_defs();
    let ids: Vec<&str> = defs.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "doc_ingest",
            "fs_list",
            "fs_read",
            "fs_write",
            "latex_compile",
            "latex_patch",
            "tex_audit",
        ]
    );
    assert!(LiteratureEndpoints::loopback_mock("http://127.0.0.1:18080/").is_ok());
    assert!(LiteratureEndpoints::loopback_mock("https://export.arxiv.org").is_ok());
    assert!(LiteratureEndpoints::loopback_mock("http://example.com").is_err());

    let tmp = TempDir::new().unwrap();
    let env = Arc::new(ResearchEnv::new(tmp.path().join("data")));
    let router = build_router(env.clone(), "s-0123456789abcdef");
    assert_eq!(router.policy().version, RESEARCH_POLICY_VERSION);
}

#[test]
fn f2_session_id_and_verbatim_prefix_rules() {
    assert!(validate_session_id("s-0123456789abcdef").is_ok());
    assert!(validate_session_id("run_1.branch-2").is_ok());
    for bad in [
        "",
        ".",
        "..",
        "trailing.",
        "sidetalk:s-0123",
        "a/b",
        "a\\b",
        "a\0b",
        "UPPER",
        &"a".repeat(44),
    ] {
        assert!(validate_session_id(bad).is_err(), "expected `{bad}` to fail");
    }

    assert_eq!(
        resolve_sources_session_id("sidetalk:s-0123456789abcdef").unwrap(),
        "s-0123456789abcdef"
    );
    assert!(resolve_sources_session_id("sidetalk:../evil").is_err());

    assert_eq!(
        strip_verbatim_str(r"\\?\C:\Workspaces\research\main.tex"),
        r"C:\Workspaces\research\main.tex"
    );
    assert_eq!(
        strip_verbatim_str(r"\\?\UNC\server\share\paper\ms.tex"),
        r"\\server\share\paper\ms.tex"
    );
    assert_eq!(
        strip_verbatim_prefix(Path::new(r"\\?\D:\tex\a.tex")),
        Path::new(r"D:\tex\a.tex")
    );
}

#[test]
fn f2_unattached_workspace_fails_closed_while_sources_works() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let session = "s-abcdef0123456789";
    let env = Arc::new(ResearchEnv::new(&data_dir));
    let router = build_router(env.clone(), session);

    // Unattached workspace fails closed on both fs_list and fs_read.
    assert!(matches!(
        router.try_call("agent:test", FS_LIST, &json!({})).unwrap(),
        CallOutcome::Error(msg) if msg.contains("no workspace")
    ));
    assert!(matches!(
        router.try_call("agent:test", FS_READ, &json!({"path": "a.tex"})).unwrap(),
        CallOutcome::Error(msg) if msg.contains("no workspace")
    ));

    // Empty uncreated sources root returns empty list cleanly.
    let list_empty = router
        .try_call("agent:test", FS_LIST, &json!({"root": "sources"}))
        .unwrap();
    assert_eq!(
        list_empty,
        CallOutcome::Ok(json!({
            "root": "sources",
            "path": "",
            "entries": [],
            "total_entries": 0,
            "truncated": false
        }))
    );

    // Populate sources file and verify sidetalk:<base> can read it, while sidetalk:<base> write is rejected.
    let sandbox = env.sandbox();
    let src_write = sandbox.resolve_for_write(SandboxRoot::Sources, session, "notes/ref.tex").unwrap();
    research_tools::WorkspaceSandbox::atomic_write_in_root(
        &src_write.canonical_root,
        &src_write.rel_parts,
        b"\\section{Source}\n",
    )
    .unwrap();
    assert!(sandbox
        .resolve_for_write(SandboxRoot::Sources, &format!("sidetalk:{session}"), "notes/evil.tex")
        .is_err());

    let sidetalk_router = build_router(env.clone(), &format!("sidetalk:{session}"));
    let read_out = sidetalk_router
        .try_call("agent:sidetalk", FS_READ, &json!({"root": "sources", "path": "notes/ref.tex"}))
        .unwrap();
    match read_out {
        CallOutcome::Ok(v) => assert_eq!(v["content"], "\\section{Source}\n"),
        other => panic!("unexpected outcome: {other:?}"),
    }

    let report = Ledger::open(&data_dir).unwrap().verify_all().unwrap();
    assert_eq!(report.unreferenced_blobs, 0);
}

#[test]
fn f2_traversal_ads_nul_and_symlink_escapes_rejected() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    let outside = tmp.path().join("outside");
    fs::create_dir_all(&ws).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret.tex"), "TOP_SECRET").unwrap();
    fs::write(ws.join("ok.tex"), "PUBLIC").unwrap();

    let env = Arc::new(ResearchEnv::with_workspace(tmp.path().join("data"), &ws).unwrap());
    let sandbox = env.sandbox();

    for bad in [
        "../outside/secret.tex",
        "sub/../../outside/secret.tex",
        "..\\outside\\secret.tex",
        "/etc/passwd",
        "C:\\Windows\\win.ini",
        "ok.tex:ads",
        "bad\0name.tex",
        "CON.tex",
        "trailing_dot.",
    ] {
        assert!(
            sandbox.resolve_existing(SandboxRoot::Workspace, "s-1", bad, false).is_err(),
            "expected read rejection for `{bad}`"
        );
        assert!(
            sandbox.resolve_for_write(SandboxRoot::Workspace, "s-1", bad).is_err(),
            "expected write rejection for `{bad}`"
        );
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        // 1. Existing file symlink pointing outside workspace.
        symlink(outside.join("secret.tex"), ws.join("escape_file.tex")).unwrap();
        assert!(matches!(
            sandbox.resolve_existing(SandboxRoot::Workspace, "s-1", "escape_file.tex", false),
            Err(SandboxError::EscapesRoot(_))
        ));
        assert!(matches!(
            sandbox.resolve_for_write(SandboxRoot::Workspace, "s-1", "escape_file.tex"),
            Err(SandboxError::EscapesRoot(_))
        ));

        // 2. Non-existent nested path under a symlinked parent pointing outside!
        symlink(&outside, ws.join("sym_out")).unwrap();
        let res = sandbox.resolve_for_write(SandboxRoot::Workspace, "s-1", "sym_out/missing_dir/new.tex");
        assert!(matches!(res, Err(SandboxError::EscapesRoot(_))));
        assert!(!outside.join("missing_dir").exists(), "must fail before creating outside dir!");

        // 3. Broken symlink and intra-root symlink loop in recursive fs_list.
        symlink(ws.join("does_not_exist.tex"), ws.join("broken.tex")).unwrap();
        fs::create_dir_all(ws.join("sub")).unwrap();
        fs::write(ws.join("sub/inner.tex"), "inner").unwrap();
        symlink(&ws, ws.join("sub/loop_root")).unwrap();

        let router = build_router(env.clone(), "s-looptest");
        let list_res = router
            .try_call("agent:test", FS_LIST, &json!({"recursive": true}))
            .unwrap();
        match list_res {
            CallOutcome::Ok(v) => {
                let paths: Vec<&str> = v["entries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|e| e["path"].as_str().unwrap())
                    .collect();
                assert!(paths.contains(&"ok.tex"));
                assert!(paths.contains(&"sub/inner.tex"));
                assert!(!paths.contains(&"escape_file.tex"), "escaping symlink must be hidden");
                assert!(!paths.contains(&"broken.tex"), "broken symlink must be hidden");
                assert!(!paths.contains(&"sym_out"), "escaping dir symlink must be hidden");
            }
            other => panic!("expected Ok listing, got {other:?}"),
        }
    }
}

#[test]
fn f3_fs_read_astral_utf8_slicing_binary_guard_and_regex_query() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ws");
    fs::create_dir_all(&ws).unwrap();

    // Astral math symbols (4-byte UTF-8 each: 𝐻 U+1D43B, 𝕏 U+1D54F) + Greek + multi-line.
    let math_doc = "𝐻 = ∑ α_i\n𝕏_1 + β_2 = 0\nTheorem 3: gap Δ > 0\n";
    fs::write(ws.join("math.tex"), math_doc).unwrap();
    fs::write(ws.join("binary.bin"), b"abc\0def").unwrap();

    let big = "λ".repeat(15_000);
    fs::write(ws.join("big.tex"), &big).unwrap();

    let env = Arc::new(ResearchEnv::with_workspace(tmp.path().join("data"), &ws).unwrap());
    let router = build_router(env.clone(), "s-fsread01");

    // 1. Astral-safe character slicing without splitting codepoints or emitting U+FFFD.
    let slice_out = router
        .try_call("agent:test", FS_READ, &json!({"path": "math.tex", "offset": 0, "max_chars": 9}))
        .unwrap();
    match slice_out {
        CallOutcome::Ok(v) => {
            assert_eq!(v["content"], "𝐻 = ∑ α_i");
            assert_eq!(v["returned_chars"], 9);
            assert_eq!(v["truncated"], true);
            assert!(!v["content"].as_str().unwrap().contains('\u{FFFD}'));
        }
        other => panic!("unexpected {other:?}"),
    }

    // 2. Cap max_chars at 12_000 even when caller requests 50_000.
    let capped = router
        .try_call("agent:test", FS_READ, &json!({"path": "big.tex", "max_chars": 50_000}))
        .unwrap();
    match capped {
        CallOutcome::Ok(v) => {
            assert_eq!(v["returned_chars"], 12_000);
            assert_eq!(v["total_chars"], 15_000);
            assert_eq!(v["truncated"], true);
        }
        other => panic!("unexpected {other:?}"),
    }

    // 3. Binary/NUL rejection.
    assert!(matches!(
        router.try_call("agent:test", FS_READ, &json!({"path": "binary.bin"})).unwrap(),
        CallOutcome::Error(msg) if msg.contains("binary")
    ));

    // 4. Substring & regex query modes with 1-based line numbers.
    let sub_q = router
        .try_call("agent:test", FS_READ, &json!({"path": "math.tex", "query": "𝕏_1"}))
        .unwrap();
    match sub_q {
        CallOutcome::Ok(v) => {
            assert_eq!(v["match_count"], 1);
            assert_eq!(v["matches"][0]["line"], 2);
        }
        other => panic!("unexpected {other:?}"),
    }

    let re_q = router
        .try_call(
            "agent:test",
            FS_READ,
            &json!({"path": "math.tex", "query": r"Theorem\s+\d+:", "regex": true}),
        )
        .unwrap();
    match re_q {
        CallOutcome::Ok(v) => {
            assert_eq!(v["match_count"], 1);
            assert_eq!(v["matches"][0]["line"], 3);
        }
        other => panic!("unexpected {other:?}"),
    }

    let verify = Ledger::open(&tmp.path().join("data")).unwrap().verify_all().unwrap();
    assert_eq!(verify.unreferenced_blobs, 0);
}

/// A workspace may not be, contain, or lie inside the data directory (keys, ledger, blobs),
/// whether or not the data directory exists yet.
#[test]
fn workspace_overlapping_data_dir_is_refused() {
    let tmp = tempfile::TempDir::new().unwrap();
    let data = tmp.path().join("data");
    let inside = data.join("sessions").join("s-1");
    let sibling = tmp.path().join("paper");
    fs::create_dir_all(&inside).unwrap();
    fs::create_dir_all(&sibling).unwrap();

    for ws in [data.as_path(), inside.as_path(), tmp.path()] {
        let err = ResearchEnv::with_workspace(&data, ws).unwrap_err().to_string();
        assert!(err.contains("overlaps"), "{}: {err}", ws.display());
    }
    // Positive control: a sibling directory is accepted.
    ResearchEnv::with_workspace(&data, &sibling).unwrap();

    // A data directory that does not exist yet is still compared (deepest existing ancestor).
    let future_data = sibling.join("not-yet").join("data");
    let err = ResearchEnv::with_workspace(&future_data, &sibling).unwrap_err().to_string();
    assert!(err.contains("overlaps"), "{err}");
    ResearchEnv::with_workspace(tmp.path().join("elsewhere").join("data"), &sibling).unwrap();

    // Windows paths compare case-insensitively.
    #[cfg(windows)]
    {
        let shouted = std::path::PathBuf::from(data.to_string_lossy().to_uppercase());
        let err = ResearchEnv::with_workspace(&shouted, &data).unwrap_err().to_string();
        assert!(err.contains("overlaps"), "{err}");
    }
}
