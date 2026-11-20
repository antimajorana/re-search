use super::*;
#[cfg(not(windows))]
use crate::api_session::{self, ApiRunOpts};
use crate::cmd::env_str_any;
use crate::session_lock::{acquire_session_lock, validate_session_id};
#[cfg(not(windows))]
use agent::ProviderKind;
use ledger::Ledger;
use serde_json::json;
use std::path::Path;
#[cfg(not(windows))]
use std::path::PathBuf;

fn cli_exit(args: &[&str]) -> i32 {
    exit_code_for_args(args.iter().map(|s| (*s).to_string()))
}

fn write_minimal_legacy_v0_db(path: &Path) {
    let mut buf = vec![0u8; 1536];
    let patches: &[(usize, &[u8])] = &[
        (0, b"SQLite format 3\x00\x02\x00\x01\x01\x00@  \x00\x00\x00\x02\x00\x00\x00\x03"),
        (43, b"\x01\x00\x00\x00\x04"),
        (59, b"\x01"),
        (95, b"\x02\x00.\x95\xc9\r\x01\xf8\x00\x02\x00\xf3\x00\x00\xf3\x01\xcd"),
        (243, b"\x81W\x01\x07\x17\x17\x17\x01\x83\rtableeventevent\x02CREATE TABLE event (session TEXT NOT NULL, seq INTEGER NOT NULL, ts_ms INTEGER NOT NULL, kind TEXT NOT NULL, schema_version INTEGER NOT NULL, payload TEXT NOT NULL, PRIMARY KEY (session, seq)))\x02\x06\x17=\x17\x01\x00indexsqlite_autoindex_event_1event\x03\x00\x00\x00\x08"),
        (512, b"\r\x00\x00\x00\x01\x01\xef\x00\x01\xef"),
        (1007, b"\x0f\x01\x07\x11\t\x08\x15\t\x11s0note{}\n\x00\x00\x00\x01\x01\xf9\x00\x01\xf9"),
        (1529, b"\x06\x04\x11\t\ts0"),
    ];
    for (off, bytes) in patches {
        buf[*off..*off + bytes.len()].copy_from_slice(bytes);
    }
    std::fs::write(path, buf).unwrap();
}

#[test]
fn verify_cli_exit_codes() {
    let base = std::env::temp_dir().join(format!("cli-verify-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();

    let missing = base.join("missing-dir");
    let missing_s = missing.to_string_lossy().into_owned();
    assert_eq!(cli_exit(&["verify", "--data-dir", &missing_s]), 2);
    assert!(!missing.exists());

    let valid = base.join("valid");
    let valid_s = valid.to_string_lossy().into_owned();
    let ledger = Ledger::open(&valid).unwrap();
    let h = ledger.put_blob(b"raw").unwrap();
    ledger.append("s1", "model.text", 1, &json!({ "text": "ok", "raw_blob": h })).unwrap();
    assert_eq!(cli_exit(&["verify", "--data-dir", &valid_s]), 0);
    assert_eq!(cli_exit(&["verify", "s1", "--data-dir", &valid_s]), 0);
    assert_eq!(cli_exit(&["verify", "unknown", "--data-dir", &valid_s]), 1);
    assert_eq!(cli_exit(&["verify", "--bogus", "1", "--data-dir", &valid_s]), 2);
    assert_eq!(cli_exit(&["verify", "s1", "s2", "--data-dir", &valid_s]), 2);

    std::fs::write(valid.join("blobs").join(&h[..2]).join(&h), b"tampered").unwrap();
    assert_eq!(cli_exit(&["verify", "--data-dir", &valid_s]), 1);
    assert_eq!(cli_exit(&["verify", "s1", "--data-dir", &valid_s]), 1);

    let legacy = base.join("legacy");
    std::fs::create_dir_all(&legacy).unwrap();
    write_minimal_legacy_v0_db(&legacy.join("ledger.db"));
    let legacy_s = legacy.to_string_lossy().into_owned();
    assert_eq!(cli_exit(&["verify", "--data-dir", &legacy_s]), 4);
    assert_eq!(cli_exit(&["verify", "s0", "--data-dir", &legacy_s]), 4);

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn route_flags_validate_and_avoid_creating_data_dir_on_errors() {
    let base = std::env::temp_dir().join(format!("cli-route-flags-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();

    let unused = base.join("should-not-exist");
    let unused_s = unused.to_string_lossy().into_owned();

    // Unknown --route -> exit 2 and no data_dir created.
    assert_eq!(cli_exit(&["run", "--route", "bogus", "--prompt", "hi", "--data-dir", &unused_s]), 2);
    assert!(!unused.exists());

    // --route api rejects --claude / --core-exe -> exit 2 and no data_dir created.
    assert_eq!(
        cli_exit(&["run", "--route", "api", "--claude", "x", "--prompt", "hi", "--data-dir", &unused_s]),
        2
    );
    assert!(!unused.exists());
    assert_eq!(
        cli_exit(&["run", "--route", "api", "--core-exe", "x", "--prompt", "hi", "--data-dir", &unused_s]),
        2
    );
    assert!(!unused.exists());

    // --route cli rejects api-only flags (including --provider) -> exit 2 and no data_dir created.
    for (flag, val) in [
        ("--provider", "openai"),
        ("--api-base", "http://127.0.0.1:1"),
        ("--max-turns", "4"),
        ("--max-tokens", "100"),
    ] {
        assert_eq!(
            cli_exit(&["run", "--route", "cli", flag, val, "--prompt", "hi", "--data-dir", &unused_s]),
            2,
            "flag {flag}"
        );
        assert!(!unused.exists(), "data_dir created for {flag}");
    }

    // Zero limits and unknown --provider rejected -> exit 2.
    assert_eq!(
        cli_exit(&["run", "--route", "api", "--provider", "bogus", "--prompt", "hi", "--data-dir", &unused_s]),
        2
    );
    assert_eq!(
        cli_exit(&["run", "--route", "api", "--max-turns", "0", "--prompt", "hi", "--data-dir", &unused_s]),
        2
    );
    assert_eq!(
        cli_exit(&["run", "--route", "api", "--max-tokens", "0", "--prompt", "hi", "--data-dir", &unused_s]),
        2
    );
    assert!(!unused.exists());

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn resume_flags_validate_before_touching_disk() {
    let base = std::env::temp_dir().join(format!("cli-resume-flags-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();

    let missing = base.join("missing-resume-dir");
    let missing_s = missing.to_string_lossy().into_owned();

    // Missing / extra positional session IDs -> exit 2
    assert_eq!(cli_exit(&["resume", "--prompt", "hi", "--data-dir", &missing_s]), 2);
    assert_eq!(cli_exit(&["resume", "s1", "s2", "--prompt", "hi", "--data-dir", &missing_s]), 2);

    // Invalid session IDs (path traversal, uppercase, trailing dot, >43 chars) -> exit 2
    let long_id = "a".repeat(44);
    for bad_id in ["../bad", "UPPER", "trailing.", ".", "..", "a/b", &long_id] {
        assert_eq!(
            cli_exit(&["resume", bad_id, "--prompt", "hi", "--data-dir", &missing_s]),
            2,
            "bad session id {bad_id}"
        );
    }

    // Disallowed flags, empty prompt, unknown provider, zero limits, unknown flags -> exit 2
    assert_eq!(cli_exit(&["resume", "s-1", "--route", "api", "--prompt", "hi", "--data-dir", &missing_s]), 2);
    assert_eq!(cli_exit(&["resume", "s-1", "--claude", "x", "--prompt", "hi", "--data-dir", &missing_s]), 2);
    assert_eq!(cli_exit(&["resume", "s-1", "--core-exe", "x", "--prompt", "hi", "--data-dir", &missing_s]), 2);
    assert_eq!(cli_exit(&["resume", "s-1", "--prompt", "   ", "--data-dir", &missing_s]), 2);
    assert_eq!(cli_exit(&["resume", "s-1", "--provider", "bad", "--prompt", "hi", "--data-dir", &missing_s]), 2);
    assert_eq!(cli_exit(&["resume", "s-1", "--max-turns", "0", "--prompt", "hi", "--data-dir", &missing_s]), 2);
    assert_eq!(cli_exit(&["resume", "s-1", "--max-tokens", "0", "--prompt", "hi", "--data-dir", &missing_s]), 2);
    assert_eq!(cli_exit(&["resume", "s-1", "--bogus", "1", "--prompt", "hi", "--data-dir", &missing_s]), 2);

    // Resuming against missing data_dir fails closed with exit 2 and never creates directory.
    assert_eq!(cli_exit(&["resume", "s-1", "--prompt", "hi", "--data-dir", &missing_s]), 2);
    assert!(!missing.exists());

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn session_id_validation_and_lock_guards() {
    assert!(validate_session_id("s-8fda_1e0a.5933-0e46").is_ok());
    assert!(validate_session_id(&"a".repeat(43)).is_ok());
    assert!(validate_session_id("").is_err());
    assert!(validate_session_id(".").is_err());
    assert!(validate_session_id("..").is_err());
    assert!(validate_session_id("s.").is_err());
    assert!(validate_session_id("S-1").is_err());
    assert!(validate_session_id(&"a".repeat(44)).is_err());

    let base = std::env::temp_dir().join(format!("cli-lock-unit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let sess_dir = base.join("s-lock");
    std::fs::create_dir_all(&sess_dir).unwrap();

    {
        let _guard = acquire_session_lock(&sess_dir, "s-lock").unwrap();
        assert!(sess_dir.join(".active.lock").is_file());
        // Live current-PID lock refuses second acquirer.
        let err = acquire_session_lock(&sess_dir, "s-lock").err().unwrap();
        assert!(err.contains("currently active"), "{err}");
    }
    assert!(!sess_dir.join(".active.lock").exists());

    // Unparseable lock fails closed.
    std::fs::write(sess_dir.join(".active.lock"), "not-a-pid").unwrap();
    assert!(acquire_session_lock(&sess_dir, "s-lock").is_err());

    // Dead PID lock is reclaimed once (and dropped cleanly on scope exit).
    std::fs::write(sess_dir.join(".active.lock"), u32::MAX.to_string()).unwrap();
    let _guard = acquire_session_lock(&sess_dir, "s-lock").unwrap();
    drop(_guard);
    assert!(!sess_dir.join(".active.lock").exists());

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn route_api_rejects_missing_key_and_non_windows_python() {
    let base = std::env::temp_dir().join(format!("cli-route-env-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();

    // Missing API key -> exit 2 (skip only if parent test env explicitly sets one).
    if env_str_any(&["RE_SEARCH_API_KEY", "RESEARCH_CORE_API_KEY", "ANTHROPIC_API_KEY"]).is_none() {
        let d = base.join("no-key");
        let d_s = d.to_string_lossy().into_owned();
        assert_eq!(cli_exit(&["run", "--route", "api", "--prompt", "hi", "--data-dir", &d_s]), 2);
    }

    #[cfg(not(windows))]
    {
        let d = base.join("linux-py");
        let d_s = d.to_string_lossy().into_owned();
        let worker_script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../python/worker.py");
        let script_s = worker_script.to_string_lossy().into_owned();
        assert_eq!(
            cli_exit(&[
                "run",
                "--route",
                "api",
                "--provider",
                "ollama",
                "--api-base",
                "http://127.0.0.1:11434/v1",
                "--python",
                "/usr/bin/python3",
                "--worker-script",
                &script_s,
                "--prompt",
                "hi",
                "--data-dir",
                &d_s,
            ]),
            2
        );
        let err = api_session::run(&ApiRunOpts {
            prompt: "hi".into(),
            provider: ProviderKind::Ollama,
            model_requested: "qwen2.5-coder:7b".into(),
            data_dir: d,
            python: Some(PathBuf::from("/usr/bin/python3")),
            worker_script: Some(worker_script),
            api_base: "http://127.0.0.1:11434/v1".into(),
            max_turns: 4,
            max_tokens: 100,
            eval_timeout_s: 30,
        });
        assert_eq!(err.unwrap_err(), "AppContainer sandbox requires Windows");
    }

    let _ = std::fs::remove_dir_all(&base);
}
