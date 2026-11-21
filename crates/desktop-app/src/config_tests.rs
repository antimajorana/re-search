use super::*;
use std::collections::HashMap;

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// An absolute path on any OS (nothing is created there).
fn abs(name: &str) -> PathBuf {
    std::env::temp_dir().join("re-search-app-config-test").join(name)
}

fn s(name: &str) -> String {
    abs(name).to_string_lossy().into_owned()
}

fn env_of(pairs: &[(&str, String)]) -> impl Fn(&str) -> Option<OsString> {
    let map: HashMap<String, OsString> =
        pairs.iter().map(|(k, v)| (k.to_string(), OsString::from(v.as_str()))).collect();
    move |name: &str| map.get(name).cloned()
}

#[test]
fn parses_the_five_flags_in_both_forms() {
    let f = parse_flags(args(&[
        "--data-dir", "D", "--workspace=W", "--python", "P", "--worker-script=S", "--port", "8123",
    ]))
    .unwrap();
    assert_eq!(f.data_dir.as_deref(), Some("D"));
    assert_eq!(f.workspace.as_deref(), Some("W"));
    assert_eq!(f.python.as_deref(), Some("P"));
    assert_eq!(f.worker_script.as_deref(), Some("S"));
    assert_eq!(f.port, Some(8123));
}

#[test]
fn rejects_unknown_flags_positionals_bad_ports_and_missing_values() {
    assert!(parse_flags(args(&["--token", "x"])).unwrap_err().contains("unknown flag"));
    assert!(parse_flags(args(&["--no-open", "true"])).is_err());
    assert!(parse_flags(args(&["stray"])).is_err());
    assert!(parse_flags(args(&["--port", "70000"])).is_err());
    assert!(parse_flags(args(&["--data-dir"])).is_err());
    assert!(parse_flags(args(&["--data-dir", "  "])).is_err());
}

#[test]
fn flag_beats_env_beats_file_beats_default() {
    let tmp = tempfile::tempdir().unwrap();
    let exe_dir = tmp.path().join("bin");
    std::fs::create_dir_all(&exe_dir).unwrap();
    let file = FileConfig {
        python: Some("file-py.exe".into()),
        worker_script: Some("file-worker.py".into()),
        data_dir: Some("file-data".into()),
        workspace: Some("file-ws".into()),
    };
    let env = env_of(&[
        ("RE_SEARCH_DATA", s("env-data")),
        ("RE_SEARCH_PYTHON", s("env-py.exe")),
        ("RE_SEARCH_WORKER_SCRIPT", s("env-worker.py")),
        ("LOCALAPPDATA", s("local")),
    ]);

    // 1. Flags win.
    let flags = Flags {
        data_dir: Some(s("flag-data")),
        workspace: Some(s("flag-ws")),
        python: Some(s("flag-py.exe")),
        worker_script: Some(s("flag-worker.py")),
        port: Some(9),
    };
    let c = resolve(&flags, &env, Some(&file), &exe_dir).unwrap();
    assert_eq!(c.data_dir, abs("flag-data"));
    assert_eq!(c.workspace, Some(abs("flag-ws")));
    assert_eq!(c.python, Some(abs("flag-py.exe")));
    assert_eq!(c.worker_script, Some(abs("flag-worker.py")));
    assert_eq!(c.port, 9);

    // 2. Environment next (there is no environment variable for the workspace).
    let c = resolve(&Flags::default(), &env, Some(&file), &exe_dir).unwrap();
    assert_eq!(c.data_dir, abs("env-data"));
    assert_eq!(c.python, Some(abs("env-py.exe")));
    assert_eq!(c.worker_script, Some(abs("env-worker.py")));
    assert_eq!(c.workspace, Some(exe_dir.join("file-ws")));
    assert_eq!(c.port, 0);

    // 3. Then the JSON file, relative to the executable's directory.
    let only_local = env_of(&[("LOCALAPPDATA", s("local"))]);
    let c = resolve(&Flags::default(), &only_local, Some(&file), &exe_dir).unwrap();
    assert_eq!(c.data_dir, exe_dir.join("file-data"));
    assert_eq!(c.python, Some(exe_dir.join("file-py.exe")));
    assert_eq!(c.worker_script, Some(exe_dir.join("file-worker.py")));

    // 4. Defaults: %LOCALAPPDATA%\re-search and no Python (so no worker script either).
    let c = resolve(&Flags::default(), &only_local, None, &exe_dir).unwrap();
    assert_eq!(c.data_dir, abs("local").join("re-search"));
    assert_eq!((c.python, c.worker_script, c.workspace), (None, None, None));

    // No LOCALAPPDATA and nothing else: a clear error rather than a guess.
    assert!(resolve(&Flags::default(), &env_of(&[]), None, &exe_dir).is_err());
}

#[test]
fn worker_script_defaults_to_python_worker_py_next_to_the_exe() {
    let tmp = tempfile::tempdir().unwrap();
    let exe_dir = tmp.path().join("app");
    std::fs::create_dir_all(exe_dir.join("python")).unwrap();
    let env = env_of(&[("LOCALAPPDATA", s("local"))]);
    let flags = Flags { python: Some(s("py.exe")), ..Flags::default() };

    let err = resolve(&flags, &env, None, &exe_dir).unwrap_err();
    assert!(err.contains("worker.py"), "{err}");

    std::fs::write(exe_dir.join("python").join("worker.py"), "# worker").unwrap();
    let c = resolve(&flags, &env, None, &exe_dir).unwrap();
    assert_eq!(c.worker_script, Some(exe_dir.join("python").join("worker.py")));
}

#[test]
fn json_file_is_optional_but_must_be_valid() {
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(load_file_config(tmp.path()).unwrap(), None);

    std::fs::write(tmp.path().join(CONFIG_FILE_NAME), r#"{"python":"py.exe","workspace":"ws"}"#).unwrap();
    let f = load_file_config(tmp.path()).unwrap().unwrap();
    assert_eq!(f.python.as_deref(), Some("py.exe"));
    assert_eq!(f.workspace.as_deref(), Some("ws"));

    std::fs::write(tmp.path().join(CONFIG_FILE_NAME), r#"{"token":"x"}"#).unwrap();
    assert!(load_file_config(tmp.path()).is_err(), "unknown keys are refused");
    std::fs::write(tmp.path().join(CONFIG_FILE_NAME), "{not json").unwrap();
    assert!(load_file_config(tmp.path()).is_err());
}

#[test]
fn webview_folder_is_under_localappdata_and_never_in_sessions_or_workspace() {
    let env = env_of(&[("LOCALAPPDATA", s("local"))]);
    let wv = webview_data_dir(&env).unwrap();
    assert_eq!(wv, abs("local").join("re-search").join("webview"));
    assert!(webview_data_dir(&env_of(&[])).is_err());

    let base = AppConfig {
        data_dir: abs("local").join("re-search"),
        workspace: None,
        python: None,
        worker_script: None,
        port: 0,
    };
    assert!(check_webview_dir(&wv, &base).is_ok(), "default layout is fine");

    let in_sessions = base.data_dir.join("sessions").join("webview");
    assert!(check_webview_dir(&in_sessions, &base).is_err());

    let ws = AppConfig { workspace: Some(abs("local")), ..base.clone() };
    assert!(check_webview_dir(&wv, &ws).is_err(), "workspace containing the profile is refused");
    // Windows paths ignore case: a differently-cased workspace containing the profile is refused
    // too, even though none of these folders exists yet.
    let shouted = std::path::PathBuf::from(abs("local").to_string_lossy().to_uppercase());
    let upper_ws = AppConfig { workspace: Some(shouted), ..base.clone() };
    assert!(check_webview_dir(&wv, &upper_ws).is_err(), "case must not defeat the check");
    // A sibling whose name merely starts with the same letters is not "inside".
    let lookalike = AppConfig { workspace: Some(abs("loc")), ..base.clone() };
    assert!(check_webview_dir(&wv, &lookalike).is_ok());
    let other_ws = AppConfig { workspace: Some(abs("papers")), ..base };
    assert!(check_webview_dir(&wv, &other_ws).is_ok());
}
