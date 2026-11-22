//! Integration tests for F5 (`fs_write`, `latex_patch`, `preview_workspace_edit`,
//! and `latex_compile`) routed end-to-end through `tools::Router::try_call`.

use ledger::Ledger;
use policy::{Decision, Match, Policy, Rule};
use research_tools::{
    preview_workspace_edit, register_research_tools, research_policy, ResearchEnv, FS_WRITE,
    LATEX_COMPILE, LATEX_PATCH,
};
use serde_json::json;
use std::fs;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;
use tools::approval::{ApprovalReply, ApprovalRequest, Approver};
use tools::router::{CallOutcome, Router};
use tools::Registry;

struct RecordingApprover {
    approve: AtomicBool,
    calls: AtomicUsize,
    last_preview_diff: Mutex<Option<String>>,
    env: Arc<ResearchEnv>,
}

impl RecordingApprover {
    fn new(env: Arc<ResearchEnv>, approve: bool) -> Arc<Self> {
        Arc::new(Self {
            approve: AtomicBool::new(approve),
            calls: AtomicUsize::new(0),
            last_preview_diff: Mutex::new(None),
            env,
        })
    }
}

impl Approver for RecordingApprover {
    fn decide(&self, req: &ApprovalRequest) -> ApprovalReply {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if matches!(req.tool_id.as_str(), FS_WRITE | LATEX_PATCH) {
            if let Ok(p) = preview_workspace_edit(&self.env, &req.tool_id, &req.canonical_args) {
                *self.last_preview_diff.lock().unwrap() = Some(p.unified_diff);
            }
        }
        ApprovalReply {
            approved: self.approve.load(Ordering::SeqCst),
            digest: req.digest.clone(),
            by: "human-test".into(),
        }
    }
}

#[test]
fn f5_allow_rule_clamped_to_ask_once_and_preview_zero_side_effects() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ws");
    let data = tmp.path().join("data");
    fs::create_dir_all(&ws).unwrap();

    let env = Arc::new(ResearchEnv::with_workspace(&data, &ws).unwrap());

    // 1. Preview on non-existent nested file has zero side effects and uses `--- /dev/null`.
    let new_args = tools::canon::canonical_json(&json!({
        "path": "sections/intro.tex",
        "content": "\\section{Intro}\nHello\n"
    }));
    let prev_new = preview_workspace_edit(&env, FS_WRITE, &new_args).unwrap();
    assert_eq!(prev_new.sha256_before, None);
    assert!(prev_new.unified_diff.contains("--- /dev/null"));
    assert!(prev_new.unified_diff.contains("+++ b/sections/intro.tex"));
    assert!(!ws.join("sections").exists(), "preview must not create parent dirs");

    // 2. Custom policy attempting `Decision::Allow` for EVERYTHING must still clamp
    // WriteWorkspace and ExecuteCode to `Decision::Ask` and ask on EVERY call (`Once`).
    let allow_all_policy = Policy {
        version: "custom-allow-attempt".into(),
        rules: vec![Rule {
            id: "allow-everything".into(),
            caller: Match::Any,
            tool: Match::Any,
            effect: None,
            tool_def_hash: None,
            decision: Decision::Allow,
        }],
        default: Decision::Allow,
    };

    let approver = RecordingApprover::new(env.clone(), false);
    let mut reg = Registry::new();
    register_research_tools(&mut reg, env.clone());
    let router = Router::new(
        reg,
        allow_all_policy,
        Ledger::open(&data).unwrap(),
        "s-clamp01",
        approver.clone(),
    );

    // Declined -> Denied, file not created.
    let denied = router
        .try_call(
            "agent:test",
            FS_WRITE,
            &json!({"path": "sections/intro.tex", "content": "\\section{Intro}\nHello\n"}),
        )
        .unwrap();
    assert!(matches!(denied, CallOutcome::Denied(_)));
    assert_eq!(approver.calls.load(Ordering::SeqCst), 1);
    assert!(!ws.join("sections/intro.tex").exists());

    // Approved -> creates file; second identical call MUST ask again (`ApprovalScope::Once`)!
    approver.approve.store(true, Ordering::SeqCst);
    let ok1 = router
        .try_call(
            "agent:test",
            FS_WRITE,
            &json!({"path": "sections/intro.tex", "content": "\\section{Intro}\nHello\n"}),
        )
        .unwrap();
    assert!(matches!(ok1, CallOutcome::Ok(ref v) if v["created"] == true && v["backup_blob"].is_null()));
    assert_eq!(approver.calls.load(Ordering::SeqCst), 2);

    let ok2 = router
        .try_call(
            "agent:test",
            FS_WRITE,
            &json!({"path": "sections/intro.tex", "content": "\\section{Intro}\nUpdated\n"}),
        )
        .unwrap();
    assert!(matches!(ok2, CallOutcome::Ok(ref v) if v["created"] == false && v["backup_blob"].is_string()));
    assert_eq!(approver.calls.load(Ordering::SeqCst), 3, "Once scope must re-prompt on second write");
    assert!(approver
        .last_preview_diff
        .lock()
        .unwrap()
        .as_deref()
        .unwrap()
        .contains("--- a/sections/intro.tex"));

    let ledger = Ledger::open(&data).unwrap();
    ledger.verify_session("s-clamp01").unwrap();
    assert_eq!(ledger.verify_all().unwrap().unreferenced_blobs, 0);
}

#[test]
fn f5_backup_restore_fail_closed_patch_exactness_and_symlink_safety() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ws");
    let outside = tmp.path().join("outside");
    let data = tmp.path().join("data");
    fs::create_dir_all(&ws).unwrap();
    fs::create_dir_all(&outside).unwrap();

    let orig_tex = "\\documentclass{article}\n\\begin{document}\nalpha = 1\nalpha = 1\n\\end{document}\n";
    fs::write(ws.join("main.tex"), orig_tex).unwrap();
    fs::write(ws.join("crlf.tex"), "line1\r\nline2\r\n").unwrap();
    fs::write(outside.join("ext.tex"), "OUTSIDE").unwrap();

    let env = Arc::new(ResearchEnv::with_workspace(&data, &ws).unwrap());
    let approver = RecordingApprover::new(env.clone(), true);
    let mut reg = Registry::new();
    register_research_tools(&mut reg, env.clone());
    let session = "s-patchbackup";
    let router = Router::new(
        reg,
        research_policy(),
        Ledger::open(&data).unwrap(),
        session,
        approver,
    );

    // 1. latex_patch exactness: empty search, missing search (with CRLF hint), ambiguous multi-match.
    assert!(matches!(
        router.try_call("agent:test", LATEX_PATCH, &json!({"path": "main.tex", "search": "", "replace": "x"})).unwrap(),
        CallOutcome::Error(m) if m.contains("empty")
    ));
    assert!(matches!(
        router.try_call("agent:test", LATEX_PATCH, &json!({"path": "crlf.tex", "search": "line1\nline2", "replace": "x"})).unwrap(),
        CallOutcome::Error(m) if m.contains("CRLF")
    ));
    assert!(matches!(
        router.try_call("agent:test", LATEX_PATCH, &json!({"path": "main.tex", "search": "alpha = 1", "replace": "alpha = 2"})).unwrap(),
        CallOutcome::Error(m) if m.contains("ambiguous")
    ));

    // 2. Multi-match with `replace_all: true` succeeds, backs up exact pre-edit bytes in blobs/.
    let patched = router
        .try_call(
            "agent:test",
            LATEX_PATCH,
            &json!({"path": "main.tex", "search": "alpha = 1", "replace": "alpha = 2", "replace_all": true}),
        )
        .unwrap();
    let backup_hash = match patched {
        CallOutcome::Ok(v) => {
            assert_eq!(v["replacements"], 2);
            v["backup_blob"].as_str().unwrap().to_string()
        }
        other => panic!("unexpected {other:?}"),
    };
    let check_ledger = Ledger::open(&data).unwrap();
    assert_eq!(check_ledger.get_blob(&backup_hash).unwrap(), orig_tex.as_bytes());
    assert_eq!(check_ledger.verify_all().unwrap().unreferenced_blobs, 0);

    // 3. Symlink & traversal rejection on writes, leaving zero `.re-search-tmp-*` files.
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(&outside, ws.join("sym_dir")).unwrap();
        symlink(outside.join("ext.tex"), ws.join("sym_file.tex")).unwrap();
        for bad in ["../outside.tex", "/etc/passwd", "sym_dir/new.tex", "sym_file.tex"] {
            let r = router
                .try_call("agent:test", FS_WRITE, &json!({"path": bad, "content": "x"}))
                .unwrap();
            assert!(matches!(r, CallOutcome::Error(_)), "expected error for `{bad}`");
        }
        assert_eq!(fs::read_to_string(outside.join("ext.tex")).unwrap(), "OUTSIDE");
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        fs::create_dir_all(ws.join("sub_in")).unwrap();
        for (link, target) in [
            (ws.join("junc_dir"), outside.clone()),
            (ws.join("junc_in"), ws.join("sub_in")),
        ] {
            let out = std::process::Command::new("cmd")
                .creation_flags(0x0800_0000)
                .args(["/C", "mklink", "/J"])
                .arg(&link)
                .arg(&target)
                .output()
                .expect("mklink /J");
            assert!(out.status.success(), "mklink /J failed: {}", String::from_utf8_lossy(&out.stderr));
        }
        for bad in [
            "../outside.tex",
            "C:\\Windows\\win.ini",
            "junc_dir/new.tex",
            "junc_dir/ext.tex",
            "junc_in/new.tex",
        ] {
            let r = router
                .try_call("agent:test", FS_WRITE, &json!({"path": bad, "content": "x"}))
                .unwrap();
            assert!(matches!(r, CallOutcome::Error(_)), "expected write error for `{bad}`");
        }
        let patch_junc = router
            .try_call(
                "agent:test",
                LATEX_PATCH,
                &json!({"path": "junc_dir/ext.tex", "search": "OUTSIDE", "replace": "PWNED"}),
            )
            .unwrap();
        assert!(matches!(patch_junc, CallOutcome::Error(_)));
        assert_eq!(fs::read_to_string(outside.join("ext.tex")).unwrap(), "OUTSIDE");
        assert!(!ws.join("sub_in/new.tex").exists());
    }

    // 4. FAIL-CLOSED BACKUP TEST: block blob shard creation for current `main.tex` bytes so
    // `put_blob` fails during backup -> verify `fs_write` aborts and disk file is 100% untouched!
    let current_bytes = fs::read(ws.join("main.tex")).unwrap();
    let current_sha = ledger::sha256_hex(&current_bytes);
    let shard_dir = data.join("blobs").join(&current_sha[..2]);
    if shard_dir.exists() {
        fs::remove_dir_all(&shard_dir).unwrap();
    }
    fs::create_dir_all(data.join("blobs")).unwrap();
    fs::write(&shard_dir, b"blocked-shard-file").unwrap();

    let fail_overwrite = router
        .try_call(
            "agent:test",
            FS_WRITE,
            &json!({"path": "main.tex", "content": "SHOULD_NEVER_BE_WRITTEN"}),
        )
        .unwrap();
    assert!(
        matches!(fail_overwrite, CallOutcome::Error(ref m) if m.contains("backup")),
        "expected fail-closed backup error, got {fail_overwrite:?}"
    );
    assert_eq!(
        fs::read(ws.join("main.tex")).unwrap(),
        current_bytes,
        "workspace file must remain 100% unchanged when backup fails!"
    );

    // Confirm zero temp files left behind in workspace.
    for entry in fs::read_dir(&ws).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        assert!(!name.starts_with(".re-search-tmp-"), "leftover temp file `{name}`");
    }
}

#[test]
fn f5_latex_compile_missing_compiler_mock_script_flags_env_scrub_and_timeout() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ws");
    let data = tmp.path().join("data");
    fs::create_dir_all(&ws).unwrap();
    fs::write(ws.join("main.tex"), "\\documentclass{article}\\begin{document}Hi\\end{document}\n").unwrap();
    fs::write(ws.join("-shell-escape.tex"), "bad").unwrap();

    // 1. Missing binary override -> deterministic `"status": "compiler_not_found"`.
    let missing_env = Arc::new(
        ResearchEnv::with_workspace(&data, &ws)
            .unwrap()
            .with_latex_compiler(tmp.path().join("does-not-exist-pdflatex")),
    );
    let approver = RecordingApprover::new(missing_env.clone(), true);
    let mut reg = Registry::new();
    register_research_tools(&mut reg, missing_env.clone());
    let router = Router::new(
        reg,
        research_policy(),
        Ledger::open(&data).unwrap(),
        "s-compile01",
        approver.clone(),
    );

    let not_found = router
        .try_call("agent:test", LATEX_COMPILE, &json!({"path": "main.tex"}))
        .unwrap();
    match not_found {
        CallOutcome::Ok(v) => {
            assert_eq!(v["ok"], false);
            assert_eq!(v["status"], "compiler_not_found");
        }
        other => panic!("unexpected {other:?}"),
    }

    // 2. Leading `-` filename rejected & extra `flags` rejected by schema (`additionalProperties: false`).
    assert!(matches!(
        router.try_call("agent:test", LATEX_COMPILE, &json!({"path": "-shell-escape.tex"})).unwrap(),
        CallOutcome::Error(m) if m.contains("start with `-`")
    ));
    assert!(matches!(
        router.try_call("agent:test", LATEX_COMPILE, &json!({"path": "main.tex", "flags": ["-shell-escape"]})).unwrap(),
        CallOutcome::Error(_)
    ));

    // 3. Cross-platform mock compiler binary (compiled with `rustc`) testing safe flags,
    //    `openin_any=p` / `openout_any=p`, `shell_escape*` + API key scrubbing, PDF output,
    //    log diagnostics, junction escape refusal, and timeout termination.
    {
        fs::write(ws.join("fail.tex"), "\\badmacro\n").unwrap();
        fs::write(ws.join("hang.tex"), "sleep\n").unwrap();

        let mock_src = tmp.path().join("mock_pdflatex.rs");
        let mock_bin = tmp.path().join(format!("pdflatex{}", std::env::consts::EXE_SUFFIX));
        let rust_src = r#"
use std::{env, fs, process, thread, time::Duration};
fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    for req in ["-interaction=nonstopmode", "-halt-on-error", "-no-shell-escape", "-file-line-error"] {
        if !args.iter().any(|a| a == req) {
            eprintln!("missing mandatory flag {req}: {args:?}");
            process::exit(91);
        }
    }
    for k in [
        "RE_SEARCH_API_KEY", "RESEARCH_CORE_API_KEY", "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY", "GEMINI_API_KEY", "GOOGLE_API_KEY", "OLLAMA_API_KEY",
    ] {
        if env::var_os(k).is_some() {
            eprintln!("leaked key env {k}");
            process::exit(94);
        }
    }
    if env::var("openin_any").as_deref() != Ok("p") {
        eprintln!("bad openin_any={:?}", env::var_os("openin_any"));
        process::exit(95);
    }
    if env::var("openout_any").as_deref() != Ok("p") {
        eprintln!("bad openout_any={:?}", env::var_os("openout_any"));
        process::exit(96);
    }
    for k in ["shell_escape", "shell_escape_commands"] {
        if env::var_os(k).is_some() {
            eprintln!("leaked {k}");
            process::exit(97);
        }
    }
    if env::var("max_print_line").as_deref() != Ok("1000") {
        eprintln!("bad max_print_line={:?}", env::var_os("max_print_line"));
        process::exit(99);
    }
    let last = args.last().cloned().unwrap_or_default();
    if last == "hang.tex" {
        thread::sleep(Duration::from_secs(5));
        process::exit(0);
    }
    let stem = last.strip_suffix(".tex").unwrap_or(&last);
    if last == "fail.tex" {
        fs::write(format!("{stem}.log"), "! Undefined control sequence.\nl.42 \\badmacro\n").unwrap();
        process::exit(1);
    }
    fs::write(format!("{stem}.log"), "LaTeX Warning: Reference `eq:1' on page 1 undefined on input line 12.\n").unwrap();
    fs::write(format!("{stem}.pdf"), "%PDF-1.4 mock\n").unwrap();
}
"#;
        fs::write(&mock_src, rust_src).unwrap();
        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let comp = std::process::Command::new(rustc)
            .args(["--edition", "2021"])
            .arg(&mock_src)
            .arg("-o")
            .arg(&mock_bin)
            .output()
            .expect("compile mock_pdflatex with rustc");
        assert!(comp.status.success(), "rustc failed: {}", String::from_utf8_lossy(&comp.stderr));

        struct EnvGuard(Vec<(&'static str, Option<std::ffi::OsString>)>);
        impl EnvGuard {
            fn set(pairs: &[(&'static str, &str)]) -> Self {
                let prev = pairs
                    .iter()
                    .map(|(k, v)| {
                        let old = std::env::var_os(k);
                        std::env::set_var(k, v);
                        (*k, old)
                    })
                    .collect();
                Self(prev)
            }
        }
        impl Drop for EnvGuard {
            fn drop(&mut self) {
                for (k, old) in &self.0 {
                    match old {
                        Some(v) => std::env::set_var(k, v),
                        None => std::env::remove_var(k),
                    }
                }
            }
        }

        let _env_guard = EnvGuard::set(&[
            ("ANTHROPIC_API_KEY", "sk-ant-should-not-leak"),
            ("OPENAI_API_KEY", "sk-openai-should-not-leak"),
            ("GEMINI_API_KEY", "ai-gemini-should-not-leak"),
            ("RE_SEARCH_API_KEY", "rs-generic-should-not-leak"),
            ("openin_any", "a"),
            ("openout_any", "a"),
            ("shell_escape", "1"),
            ("shell_escape_commands", "cmd"),
        ]);
        let mock_env = Arc::new(
            ResearchEnv::with_workspace(tmp.path().join("data2"), &ws)
                .unwrap()
                .with_latex_compiler(&mock_bin)
                .with_latex_timeout(Duration::from_millis(200)),
        );
        let mock_approver = RecordingApprover::new(mock_env.clone(), true);
        let mut reg2 = Registry::new();
        register_research_tools(&mut reg2, mock_env);
        let router2 = Router::new(
            reg2,
            research_policy(),
            Ledger::open(&tmp.path().join("data2")).unwrap(),
            "s-compile02",
            mock_approver,
        );

        for engine in ["latexmk", "pdflatex"] {
            let ok_res = router2
                .try_call("agent:test", LATEX_COMPILE, &json!({"path": "main.tex", "engine": engine}))
                .unwrap();
            match ok_res {
                CallOutcome::Ok(v) => {
                    assert_eq!(v["ok"], true, "engine={engine}, result={v}");
                    assert_eq!(v["status"], "compiled", "engine={engine}, result={v}");
                    assert_eq!(v["pdf_path"], "main.pdf");
                    assert_eq!(v["warnings"][0]["line"], 12);
                }
                other => panic!("unexpected {other:?} for engine={engine}"),
            }
        }

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let outside_tex = tmp.path().join("outside_compile");
            fs::create_dir_all(&outside_tex).unwrap();
            fs::write(outside_tex.join("escape.tex"), "hi\n").unwrap();
            let out = std::process::Command::new("cmd")
                .creation_flags(0x0800_0000)
                .args(["/C", "mklink", "/J"])
                .arg(ws.join("junc_compile"))
                .arg(&outside_tex)
                .output()
                .unwrap();
            assert!(out.status.success());
            let junc_res = router2
                .try_call("agent:test", LATEX_COMPILE, &json!({"path": "junc_compile/escape.tex"}))
                .unwrap();
            assert!(matches!(junc_res, CallOutcome::Error(_)), "expected error, got {junc_res:?}");
            assert!(!outside_tex.join("escape.pdf").exists());
        }

        let err_res = router2
            .try_call("agent:test", LATEX_COMPILE, &json!({"path": "fail.tex"}))
            .unwrap();
        match err_res {
            CallOutcome::Ok(v) => {
                assert_eq!(v["ok"], false);
                assert_eq!(v["status"], "failed");
                assert_eq!(v["errors"][0]["line"], 42);
                assert!(v["errors"][0]["message"]
                    .as_str()
                    .unwrap()
                    .contains("Undefined control sequence"));
            }
            other => panic!("unexpected {other:?}"),
        }

        let timeout_res = router2
            .try_call("agent:test", LATEX_COMPILE, &json!({"path": "hang.tex"}))
            .unwrap();
        match timeout_res {
            CallOutcome::Ok(v) => {
                assert_eq!(v["ok"], false);
                assert_eq!(v["status"], "timeout");
            }
            other => panic!("unexpected {other:?}"),
        }
        drop(_env_guard);

        assert_eq!(
            Ledger::open(&tmp.path().join("data2")).unwrap().verify_all().unwrap().unreferenced_blobs,
            0
        );
    }
}
