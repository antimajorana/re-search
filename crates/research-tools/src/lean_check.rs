//! Approval-gated Lean 4 verification (`lean_check`, `EffectClass::ExecuteCode`)
//! with pre-execution anti-cheat scanning for `sorry`, `admit`, and unexpected `axiom` declarations.

use crate::sandbox::{
    is_symlink_or_junction, strip_verbatim_prefix, validate_rel_workspace_path,
    validate_session_id, WorkspaceSandbox,
};
use crate::ResearchEnv;
use policy::EffectClass;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tools::canon::random_hex;
use tools::{ApprovalScope, CallCtx, Registry, ToolDef};

pub const LEAN_CHECK: &str = "lean_check";
pub const DEFAULT_LEAN_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_JSON_OUTPUT_CHARS: usize = 16_384;
const SCRUBBED_KEY_ENVS: &[&str] = &[
    "RE_SEARCH_API_KEY", "RESEARCH_CORE_API_KEY", "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY", "GEMINI_API_KEY", "GOOGLE_API_KEY", "OLLAMA_API_KEY",
];
static SCRATCH_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeanDiagnostic {
    pub file: Option<String>,
    pub line: Option<u32>,
    pub col: Option<u32>,
    pub severity: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeanAntiCheat {
    pub has_sorry_or_admit: bool,
    pub sorry_lines: Vec<u32>,
    pub axioms: Vec<String>,
}

struct ScratchGuard(Option<PathBuf>);
impl Drop for ScratchGuard {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() { let _ = fs::remove_file(p); }
    }
}

pub fn lean_check_def() -> ToolDef {
    ToolDef {
        id: LEAN_CHECK.into(),
        description: "Type-check a Lean 4 `.lean` workspace file or inline snippet with anti-cheat \
                      verification (rejects `sorry`, `admit`, and unexpected `axiom` declarations)."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "maxLength": 512, "description": "Workspace-relative `.lean` file path." },
                "code": { "type": "string", "maxLength": 65536, "description": "Inline Lean 4 source code to verify." },
                "allow_axioms": { "type": "boolean", "description": "Allow top-level `axiom` declarations (default false)." }
            },
            "additionalProperties": false
        }),
        effect: EffectClass::ExecuteCode,
        approval_scope: ApprovalScope::Once,
    }
}

pub fn register_lean_tool(registry: &mut Registry, env: Arc<ResearchEnv>) {
    registry.register(
        lean_check_def(),
        Arc::new(move |ctx: &CallCtx<'_>, args: &Value| run_lean_check(&env, ctx.session, args)),
    );
}

fn is_ident_char(c: char) -> bool { c.is_alphanumeric() || c == '_' || c == '\'' }
fn is_ident_start(c: char) -> bool { c.is_alphabetic() || c == '_' }

fn scan_ident_end(s: &str, mut end: usize) -> usize {
    while let Some(ch) = s[end..].chars().next().filter(|&c| is_ident_char(c)) {
        end += ch.len_utf8();
    }
    end
}

fn match_char_literal(s: &str) -> Option<&str> {
    let b = s.as_bytes();
    if b.first() != Some(&b'\'') { return None; }
    let (ch, idx) = if b.get(1) == Some(&b'\\') { (s[2..].chars().next()?, 2) } else { (s[1..].chars().next()?, 1) };
    let end = idx + ch.len_utf8();
    let bad = matches!(ch, '\n' | '\r') || (idx == 1 && ch == '\'');
    (!bad && b.get(end) == Some(&b'\'')).then(|| &s[..end + 1])
}

/// Strips Lean `--` comments, nested `/- ... -/` block comments, and string/char literals while preserving newlines.
fn strip_lean_comments_and_strings(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let (mut i, mut depth, mut in_line, mut in_str, mut prev_ident) = (0usize, 0usize, false, false, false);
    let b = source.as_bytes();
    while i < b.len() {
        if in_line {
            if b[i] == b'\n' { in_line = false; out.push('\n'); }
            i += source[i..].chars().next().unwrap().len_utf8();
        } else if depth > 0 {
            if source[i..].starts_with("/-") { depth += 1; i += 2; }
            else if source[i..].starts_with("-/") { depth -= 1; i += 2; if depth == 0 { out.push(' '); } }
            else { let ch = source[i..].chars().next().unwrap(); if ch == '\n' { out.push('\n'); } i += ch.len_utf8(); }
        } else if in_str {
            if b[i] == b'\\' {
                i += 1;
                if i < b.len() { let ch = source[i..].chars().next().unwrap(); if ch == '\n' { out.push('\n'); } i += ch.len_utf8(); }
            } else if b[i] == b'"' { in_str = false; out.push(' '); i += 1; }
            else { let ch = source[i..].chars().next().unwrap(); if ch == '\n' { out.push('\n'); } i += ch.len_utf8(); }
        } else if source[i..].starts_with("--") { in_line = true; i += 2; prev_ident = false; }
        else if source[i..].starts_with("/-") { depth = 1; i += 2; prev_ident = false; }
        else if b[i] == b'"' { in_str = true; out.push(' '); i += 1; prev_ident = false; }
        else if b[i] == b'\'' && !prev_ident {
            if let Some(lit) = match_char_literal(&source[i..]) { out.push(' '); i += lit.len(); prev_ident = false; }
            else { out.push('\''); i += 1; prev_ident = true; }
        } else {
            let ch = source[i..].chars().next().unwrap();
            out.push(ch);
            prev_ident = is_ident_char(ch);
            i += ch.len_utf8();
        }
    }
    out
}

pub fn scan_lean_anti_cheat(source: &str) -> LeanAntiCheat {
    let stripped = strip_lean_comments_and_strings(source);
    let (mut sorry_lines, mut axioms, mut expect_axiom) = (Vec::new(), Vec::new(), false);
    for (idx, line) in stripped.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let (mut i, mut prev_dot) = (0usize, false);
        while i < line.len() {
            let ch = line[i..].chars().next().unwrap();
            if ch.is_whitespace() { i += ch.len_utf8(); prev_dot = false; continue; }
            if ch == '«' {
                let s0 = i + ch.len_utf8();
                if let Some(rel) = line[s0..].find('»') {
                    let inner = &line[s0..s0 + rel];
                    if expect_axiom && !inner.is_empty() && !axioms.iter().any(|a| a == inner) { axioms.push(inner.to_string()); }
                    i = s0 + rel + '»'.len_utf8();
                } else { i = s0; }
                expect_axiom = false; prev_dot = false; continue;
            }
            if is_ident_char(ch) {
                let end = scan_ident_end(line, i + ch.len_utf8());
                let tok = &line[i..end];
                i = end;
                if expect_axiom {
                    if tok.chars().next().is_some_and(is_ident_start) {
                        let mut name = tok.to_string();
                        while line[i..].starts_with('.') {
                            let Some(c) = line[i + 1..].chars().next().filter(|&c| is_ident_start(c)) else { break };
                            let seg_end = scan_ident_end(line, i + 1 + c.len_utf8());
                            name.push('.');
                            name.push_str(&line[i + 1..seg_end]);
                            i = seg_end;
                        }
                        if !axioms.contains(&name) { axioms.push(name); }
                    }
                    expect_axiom = false;
                } else if tok == "sorry" || tok == "admit" {
                    if sorry_lines.last().copied() != Some(line_no) { sorry_lines.push(line_no); }
                } else if tok == "axiom" && !prev_dot { expect_axiom = true; }
                prev_dot = false;
            } else {
                expect_axiom = false; prev_dot = ch == '.'; i += ch.len_utf8();
            }
        }
    }
    LeanAntiCheat { has_sorry_or_admit: !sorry_lines.is_empty(), sorry_lines, axioms }
}

pub fn run_lean_check(env: &ResearchEnv, session: &str, args: &Value) -> Result<Value, String> {
    if !args.is_object() { return Err("arguments must be a JSON object".into()); }
    let path_arg = match args.get("path") { None | Some(Value::Null) => None, Some(v) => Some(v.as_str().ok_or("`path` must be a string")?) };
    let code_arg = match args.get("code") { None | Some(Value::Null) => None, Some(v) => Some(v.as_str().ok_or("`code` must be a string")?) };
    let allow_axioms = match args.get("allow_axioms") { None | Some(Value::Null) => false, Some(v) => v.as_bool().ok_or("`allow_axioms` must be a boolean")? };
    match (path_arg, code_arg) {
        (Some(_), Some(_)) => return Err("provide either `path` or `code`, not both".into()),
        (None, None) => return Err("either `path` or `code` must be provided".into()),
        (Some(p), None) if p.trim().is_empty() => return Err("`path` must not be empty".into()),
        (None, Some(c)) if c.trim().is_empty() => return Err("`code` must not be empty".into()),
        _ => {}
    }
    let (ws_paths, source) = if let Some(rel) = path_arg {
        let (w, t, s) = resolve_workspace_lean_file(env, rel)?;
        (Some((w, t)), s)
    } else {
        validate_session_id(session).map_err(|e| e.to_string())?;
        (None, code_arg.expect("checked").to_string())
    };
    let anti = scan_lean_anti_cheat(&source);
    if anti.has_sorry_or_admit {
        return Ok(json!({
            "ok": false, "status": "rejected_sorry", "sorry_lines": anti.sorry_lines,
            "axioms": anti.axioms, "diagnostics": [],
            "error": "Lean proof rejected: contains `sorry` or `admit` (incomplete proof)"
        }));
    }
    if !allow_axioms && !anti.axioms.is_empty() {
        return Ok(json!({
            "ok": false, "status": "rejected_axiom", "axioms": anti.axioms, "diagnostics": [],
            "error": "Lean proof contains unproven `axiom` declarations; prove them or pass allow_axioms: true"
        }));
    }
    let Some(lean_bin) = resolve_lean_binary(env) else {
        return Ok(json!({
            "ok": false, "status": "lean_not_found", "axioms": anti.axioms, "diagnostics": [],
            "hint": "Lean 4 (`lean`) was not found. Install via elan (https://lean-lang.org/install) or set RE_SEARCH_LEAN."
        }));
    };
    let ((work_dir, target_path), _guard) = match ws_paths {
        Some(paths) => (paths, ScratchGuard(None)),
        None => {
            let paths = prepare_scratch_lean_file(&env.data_dir, session, &source)?;
            let guard = ScratchGuard(Some(paths.1.clone()));
            (paths, guard)
        }
    };
    let mut cmd = Command::new(&lean_bin);
    cmd.current_dir(&work_dir).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).arg(&target_path);
    for k in SCRUBBED_KEY_ENVS { cmd.env_remove(k); }
    #[cfg(windows)]
    { use std::os::windows::process::CommandExt; cmd.creation_flags(0x0800_0000); }

    let timeout = env.latex_compile_timeout.max(Duration::from_secs(10));
    let out = run_bounded(&mut cmd, timeout)?;
    let diags = parse_lean_diagnostics(&format!("{}\n\n{}", out.stdout.trim_end(), out.stderr));
    let (stdout, stderr) = (clip_chars(&out.stdout, MAX_JSON_OUTPUT_CHARS), clip_chars(&out.stderr, MAX_JSON_OUTPUT_CHARS));
    if out.timed_out {
        return Ok(json!({
            "ok": false, "status": "timeout", "exit_code": Value::Null, "axioms": anti.axioms,
            "diagnostics": diags, "stdout": stdout, "stderr": stderr,
            "error": format!("Lean check timed out after {}s", timeout.as_secs())
        }));
    }
    let ok = out.exit_code == Some(0) && diags.iter().all(|d| d.severity != "error");
    Ok(json!({
        "ok": ok, "status": if ok { "verified" } else { "failed" },
        "exit_code": out.exit_code, "axioms": anti.axioms,
        "diagnostics": diags, "stdout": stdout, "stderr": stderr,
    }))
}

fn step_verified_dir(root: &Path, cur: &Path, seg: &str, create: bool) -> Result<PathBuf, String> {
    let next = cur.join(seg);
    match fs::symlink_metadata(&next) {
        Ok(m) if is_symlink_or_junction(&next, &m) || !m.is_dir() => return Err(format!("unsafe directory component `{seg}`")),
        Ok(_) => {}
        Err(e) if create && e.kind() == io::ErrorKind::NotFound => fs::create_dir(&next).map_err(|e| format!("failed to create `{seg}`: {e}"))?,
        Err(e) => return Err(format!("cannot access `{seg}`: {e}")),
    }
    let canon = fs::canonicalize(&next).map_err(|e| e.to_string())?;
    if !WorkspaceSandbox::is_same_or_inside(root, &canon) { return Err(format!("`{seg}` escapes sandbox root")); }
    Ok(canon)
}

fn resolve_workspace_lean_file(env: &ResearchEnv, raw: &str) -> Result<(PathBuf, PathBuf, String), String> {
    let ws = env.workspace_root.as_deref().ok_or("no workspace directory is attached to this session")?;
    let canon_root = WorkspaceSandbox::canonicalize_existing_dir(ws).map_err(|e| e.to_string())?;
    let parts = validate_rel_workspace_path(raw)?;
    let rel = parts.join("/");
    if !rel.to_ascii_lowercase().ends_with(".lean") { return Err("`path` must point to a `.lean` file".into()); }
    let file_name = parts.last().ok_or("invalid `.lean` file path")?;
    if file_name.starts_with('-') { return Err("`.lean` filename must not start with `-`".into()); }
    let mut cur = canon_root.clone();
    for comp in &parts[..parts.len() - 1] { cur = step_verified_dir(&canon_root, &cur, comp, false)?; }
    let target = cur.join(file_name);
    let meta = fs::symlink_metadata(&target).map_err(|_| format!("target `.lean` file `{rel}` does not exist"))?;
    if is_symlink_or_junction(&target, &meta) || !meta.is_file() { return Err(format!("target `{rel}` must be a regular non-symlink file")); }
    let canon_target = fs::canonicalize(&target).map_err(|e| e.to_string())?;
    if !WorkspaceSandbox::is_same_or_inside(&canon_root, &canon_target) { return Err("target `.lean` file escapes workspace root".into()); }
    let src = fs::read_to_string(&canon_target).map_err(|e| format!("failed to read `{rel}` as UTF-8: {e}"))?;
    Ok((strip_verbatim_prefix(&canon_root), strip_verbatim_prefix(&canon_target), src))
}

fn prepare_scratch_lean_file(data_dir: &Path, session: &str, code: &str) -> Result<(PathBuf, PathBuf), String> {
    fs::create_dir_all(data_dir).map_err(|e| format!("failed to prepare data_dir: {e}"))?;
    let canon_data = WorkspaceSandbox::canonicalize_existing_dir(data_dir).map_err(|e| e.to_string())?;
    let mut cur = canon_data.clone();
    for seg in ["sessions", session, "scratch"] { cur = step_verified_dir(&canon_data, &cur, seg, true)?; }
    let seq = SCRATCH_SEQ.fetch_add(1, Ordering::Relaxed);
    let file_path = cur.join(format!("lean_check_{}_{seq}.lean", random_hex(6)));
    let mut f = OpenOptions::new().write(true).create_new(true).open(&file_path).map_err(|e| format!("failed to create scratch Lean file: {e}"))?;
    f.write_all(code.as_bytes()).map_err(|e| format!("failed to write scratch Lean file: {e}"))?;
    Ok((strip_verbatim_prefix(&cur), strip_verbatim_prefix(&file_path)))
}

fn resolve_lean_binary(env: &ResearchEnv) -> Option<PathBuf> {
    if let Some(ov) = &env.lean_override { return ov.is_file().then(|| ov.clone()); }
    if let Some(eb) = std::env::var_os("RE_SEARCH_LEAN") { let p = PathBuf::from(eb); return p.is_file().then_some(p); }
    if let Some(found) = find_on_path("lean") { return Some(found); }
    #[cfg(windows)]
    let elan = std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join(".elan").join("bin").join("lean.exe"));
    #[cfg(not(windows))]
    let elan = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".elan").join("bin").join("lean"));
    elan.filter(|p| p.is_file())
}

fn find_on_path(bin: &str) -> Option<PathBuf> {
    let path_os = std::env::var_os("PATH")?;
    #[cfg(windows)] let exts: &[&str] = &[".exe", ".cmd", ".bat", ""];
    #[cfg(not(windows))] let exts: &[&str] = &[""];
    std::env::split_paths(&path_os)
        .filter(|d| !d.as_os_str().is_empty())
        .flat_map(|d| exts.iter().map(move |e| d.join(format!("{bin}{e}"))))
        .find(|p| p.is_file())
}

struct SubprocessOutput { exit_code: Option<i32>, stdout: String, stderr: String, timed_out: bool }

fn spawn_capped_reader<R: Read + Send + 'static>(mut pipe: R) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buf = Vec::with_capacity(4096);
        let _ = (&mut pipe).take(MAX_OUTPUT_BYTES as u64).read_to_end(&mut buf);
        let _ = io::copy(&mut (&mut pipe).take(4 * 1024 * 1024), &mut io::sink());
        String::from_utf8_lossy(&buf).into_owned()
    })
}

fn run_bounded(cmd: &mut Command, timeout: Duration) -> Result<SubprocessOutput, String> {
    let mut child = cmd.spawn().map_err(|e| format!("failed to spawn Lean: {e}"))?;
    let out_h = spawn_capped_reader(child.stdout.take().expect("piped"));
    let err_h = spawn_capped_reader(child.stderr.take().expect("piped"));
    let start = Instant::now();
    let mut timed_out = false;
    let exit_code = loop {
        match child.try_wait().map_err(|e| format!("wait error: {e}"))? {
            Some(status) => break status.code(),
            None if start.elapsed() >= timeout => { timed_out = true; let _ = child.kill(); let _ = child.wait(); break None; }
            None => std::thread::sleep(Duration::from_millis(15)),
        }
    };
    Ok(SubprocessOutput { exit_code, stdout: out_h.join().unwrap_or_default(), stderr: err_h.join().unwrap_or_default(), timed_out })
}

pub fn parse_lean_diagnostics(output: &str) -> Vec<LeanDiagnostic> {
    static DIAG_RE: OnceLock<Regex> = OnceLock::new();
    let re = DIAG_RE.get_or_init(|| Regex::new(r"(?i)^(.+?):(\d+):(\d+):\s*(error|warning|info|information|note|hint)\s*:\s*(.*)$").unwrap());
    let (mut diags, mut in_diag): (Vec<LeanDiagnostic>, bool) = (Vec::new(), false);
    for raw in output.lines() {
        let trimmed = raw.trim_start();
        if let Some(caps) = re.captures(trimmed) {
            in_diag = diags.len() < 100;
            if in_diag {
                let file = caps[1].trim().to_string();
                diags.push(LeanDiagnostic {
                    file: (!file.is_empty()).then_some(file),
                    line: caps[2].parse().ok(), col: caps[3].parse().ok(),
                    severity: caps[4].to_ascii_lowercase(), message: caps[5].trim_end().to_string(),
                });
            }
        } else if in_diag && !trimmed.is_empty() && raw.starts_with([' ', '\t']) {
            if let Some(last) = diags.last_mut().filter(|d| d.message.len() < 2048) {
                if !last.message.is_empty() { last.message.push('\n'); }
                last.message.push_str(raw.trim_end());
            }
        } else { in_diag = false; }
    }
    diags
}

fn clip_chars(s: &str, max_chars: usize) -> String {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => format!("{}...[truncated]", &s[..idx]),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anti_cheat_detects_sorry_admit_and_ignores_comments() {
        let clean = "-- sorry and admit\n/- sorry /- admit -/ -/\n/-- doc sorry -/\ndef s := \"sorry\"\ndef q : Char := '\"'\ndef not_sorry_lemma := 1\ntheorem ok' : True := trivial\n";
        assert!(!scan_lean_anti_cheat(clean).has_sorry_or_admit);

        let dirty = "theorem a : True := trivial -- sorry\n/-\n  sorry\n  /- admit -/\n-/\ntheorem b : False := by\n  sorry\ntheorem c : False := by admit\n";
        let res = scan_lean_anti_cheat(dirty);
        assert!(res.has_sorry_or_admit && res.sorry_lines == vec![7, 8]);

        let tmp = tempfile::tempdir().unwrap();
        let out = run_lean_check(&ResearchEnv::new(tmp.path().join("d")), "s-sorry", &json!({ "code": dirty })).unwrap();
        assert_eq!((out["ok"].as_bool(), out["status"].as_str()), (Some(false), Some("rejected_sorry")));
        assert_eq!(out["sorry_lines"], json!([7, 8]));
    }

    #[test]
    fn anti_cheat_rejects_axioms_unless_allowed() {
        let src = "-- axiom ignored : False\ndef axiom_helper := 1\ndef msg := \"axiom str\"\naxiom bad_ax : False\nprivate axiom priv_ax : True\n";
        assert_eq!(scan_lean_anti_cheat(src).axioms, vec!["bad_ax", "priv_ax"]);

        let tmp = tempfile::tempdir().unwrap();
        let env = ResearchEnv::new(tmp.path().join("d")).with_lean_override(tmp.path().join("no-lean"));
        let rej = run_lean_check(&env, "s-ax", &json!({ "code": src })).unwrap();
        assert_eq!((rej["ok"].as_bool(), rej["status"].as_str(), &rej["axioms"]), (Some(false), Some("rejected_axiom"), &json!(["bad_ax", "priv_ax"])));

        let allowed = run_lean_check(&env, "s-ax", &json!({ "code": src, "allow_axioms": true })).unwrap();
        assert_eq!((allowed["status"].as_str(), &allowed["axioms"]), (Some("lean_not_found"), &json!(["bad_ax", "priv_ax"])));
    }

    #[test]
    fn diagnostic_parser_extracts_line_col_and_severity() {
        let log = "Proof.lean:12:4: error: type mismatch\n  expected Nat\nC:\\ws\\Main.lean:7:2: warning: unused variable\nsrc/Lib.lean:3:0: info: checking\n";
        let d = parse_lean_diagnostics(log);
        assert_eq!(d.len(), 3);
        assert_eq!(d[0], LeanDiagnostic { file: Some("Proof.lean".into()), line: Some(12), col: Some(4), severity: "error".into(), message: "type mismatch\n  expected Nat".into() });
        assert_eq!((d[1].file.as_deref(), d[1].line, d[1].col, d[1].severity.as_str()), (Some("C:\\ws\\Main.lean"), Some(7), Some(2), "warning"));
        assert_eq!(d[2].severity, "info");
    }

    #[test]
    fn missing_lean_binary_returns_lean_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let env = ResearchEnv::new(tmp.path().join("d")).with_lean_override(tmp.path().join("no-lean"));
        let out = run_lean_check(&env, "s-miss", &json!({ "code": "theorem t : True := trivial" })).unwrap();
        assert_eq!((out["ok"].as_bool(), out["status"].as_str()), (Some(false), Some("lean_not_found")));
        assert!(out["hint"].as_str().unwrap().contains("RE_SEARCH_LEAN"));
    }

    #[test]
    fn mock_lean_binary_executes_and_reports_verified_and_failed() {
        let tmp = tempfile::tempdir().unwrap();
        let (ws, data) = (tmp.path().join("ws"), tmp.path().join("data"));
        fs::create_dir_all(&ws).unwrap();
        fs::write(ws.join("ok.lean"), "theorem ok : True := trivial\n").unwrap();
        fs::write(ws.join("fail.lean"), "theorem bad : False := TRIGGER_FAIL\n").unwrap();

        #[cfg(windows)]
        let mock_bin = {
            let p = tmp.path().join("mock_lean.cmd");
            fs::write(&p, "@echo off\r\nfindstr /L /C:\"TRIGGER_FAIL\" \"%~1\" >nul 2>nul\r\nif not errorlevel 1 (\r\n  >&2 echo %~1:2:4: error: simulated type mismatch\r\n  exit /b 1\r\n)\r\necho verified ok\r\nexit /b 0\r\n").unwrap();
            p
        };
        #[cfg(not(windows))]
        let mock_bin = {
            use std::os::unix::fs::PermissionsExt;
            let p = tmp.path().join("mock_lean.sh");
            fs::write(&p, "#!/bin/sh\nif grep -q \"TRIGGER_FAIL\" \"$1\" 2>/dev/null; then\n  echo \"$1:2:4: error: simulated type mismatch\" >&2\n  exit 1\nfi\necho \"verified ok\"\nexit 0\n").unwrap();
            let mut perms = fs::metadata(&p).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&p, perms).unwrap();
            p
        };

        let env = Arc::new(ResearchEnv::with_workspace(&data, &ws).unwrap().with_lean_override(&mock_bin));
        let mut reg = Registry::new();
        register_lean_tool(&mut reg, env.clone());
        assert_eq!(reg.get(LEAN_CHECK).unwrap().0.effect, EffectClass::ExecuteCode);

        let ok_ws = run_lean_check(&env, "s-mock", &json!({ "path": "ok.lean" })).unwrap();
        assert_eq!((ok_ws["ok"].as_bool(), ok_ws["status"].as_str()), (Some(true), Some("verified")));
        let fail_ws = run_lean_check(&env, "s-mock", &json!({ "path": "fail.lean" })).unwrap();
        assert_eq!((fail_ws["ok"].as_bool(), fail_ws["status"].as_str(), fail_ws["exit_code"].as_i64()), (Some(false), Some("failed"), Some(1)));
        assert_eq!((fail_ws["diagnostics"][0]["line"].as_u64(), fail_ws["diagnostics"][0]["col"].as_u64(), fail_ws["diagnostics"][0]["severity"].as_str()), (Some(2), Some(4), Some("error")));

        let ok_code = run_lean_check(&env, "s-mock", &json!({ "code": "theorem t : True := trivial" })).unwrap();
        assert_eq!((ok_code["ok"].as_bool(), ok_code["status"].as_str()), (Some(true), Some("verified")));
        let fail_code = run_lean_check(&env, "s-mock", &json!({ "code": "theorem b : False := TRIGGER_FAIL" })).unwrap();
        assert_eq!((fail_code["ok"].as_bool(), fail_code["status"].as_str(), fail_code["diagnostics"][0]["severity"].as_str()), (Some(false), Some("failed"), Some("error")));
    }
}
