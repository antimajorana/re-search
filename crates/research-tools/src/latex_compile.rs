//! Approval-gated local LaTeX compilation (`latex_compile`, `EffectClass::ExecuteCode`).
//! Uses hardcoded non-negotiable safe flags (`-no-shell-escape`, `-interaction=nonstopmode`,
//! `-halt-on-error`), forces paranoid TeX file-access mode (`openin_any=p`, `openout_any=p`),
//! strips `shell_escape` / `shell_escape_commands` and provider API keys from the child env,
//! and returns structured `.log` diagnostics or `"compiler_not_found"`.

use crate::sandbox::{
    is_symlink_or_junction, strip_verbatim_prefix, validate_rel_workspace_path, WorkspaceSandbox,
};
use crate::ResearchEnv;
use policy::EffectClass;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tools::{ApprovalScope, CallCtx, Registry, ToolDef};

pub const LATEX_COMPILE: &str = "latex_compile";
const MAX_OUTPUT_BYTES: usize = 512 * 1024;
const MAX_LOG_BYTES: usize = 1024 * 1024;
const SCRUBBED_KEY_ENVS: &[&str] = &[
    "RE_SEARCH_API_KEY",
    "RESEARCH_CORE_API_KEY",
    "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "OLLAMA_API_KEY",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatexDiagnostic {
    pub file: Option<String>,
    pub line: Option<u32>,
    pub message: String,
}

pub fn latex_compile_def() -> ToolDef {
    ToolDef {
        id: LATEX_COMPILE.into(),
        description: "Compile a `.tex` file inside the attached workspace using `pdflatex` or \
                      `latexmk` with hardcoded safe flags (`-no-shell-escape`, `-interaction=nonstopmode`, \
                      `-halt-on-error`). Requires explicit human approval (`execute-code`). Returns \
                      structured log errors or `compiler_not_found` when TeX is not installed."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "maxLength": 512, "description": "Workspace-relative `.tex` file path." },
                "engine": { "type": "string", "maxLength": 16, "description": "Optional: 'pdflatex' (default) or 'latexmk'." },
                "runs": { "type": "integer", "description": "Optional number of pdflatex passes (1..=3, default 1)." }
            },
            "required": ["path"],
            "additionalProperties": false
        }),
        effect: EffectClass::ExecuteCode,
        approval_scope: ApprovalScope::Once,
    }
}

pub fn register(registry: &mut Registry, env: Arc<ResearchEnv>) {
    registry.register(
        latex_compile_def(),
        Arc::new(move |_ctx: &CallCtx<'_>, args: &Value| run_latex_compile(&env, args)),
    );
}

pub fn run_latex_compile(env: &ResearchEnv, args: &Value) -> Result<Value, String> {
    let ws_root = env
        .workspace_root
        .as_deref()
        .ok_or_else(|| "no workspace directory is attached to this session".to_string())?;
    let canon_root = WorkspaceSandbox::canonicalize_existing_dir(ws_root).map_err(|e| e.to_string())?;

    let raw_path = args.get("path").and_then(Value::as_str).ok_or("`path` must be a string")?;
    let rel_components = validate_rel_workspace_path(raw_path)?;
    let rel_path = rel_components.join("/");
    if !rel_path.to_ascii_lowercase().ends_with(".tex") {
        return Err("`path` must point to a `.tex` file".into());
    }
    let file_name = rel_components.last().ok_or("invalid `.tex` file path")?;
    if file_name.starts_with('-') {
        return Err("`.tex` filename must not start with `-`".into());
    }

    let engine = match args.get("engine").and_then(Value::as_str) {
        Some("pdflatex") | None => "pdflatex",
        Some("latexmk") => "latexmk",
        Some(other) => return Err(format!("unsupported `engine` `{other}` (expected `pdflatex` or `latexmk`)")),
    };
    let runs = match args.get("runs") {
        Some(v) => {
            let n = v.as_u64().ok_or("`runs` must be an integer between 1 and 3")?;
            if !(1..=3).contains(&n) {
                return Err("`runs` must be between 1 and 3".into());
            }
            n as usize
        }
        None => 1,
    };

    let mut cur = canon_root.clone();
    for comp in &rel_components[..rel_components.len() - 1] {
        let next = cur.join(comp);
        let meta = fs::symlink_metadata(&next).map_err(|e| format!("cannot access `{comp}`: {e}"))?;
        if is_symlink_or_junction(&next, &meta) || !meta.is_dir() {
            return Err(format!("unsafe directory component `{comp}`"));
        }
        cur = fs::canonicalize(&next).map_err(|e| e.to_string())?;
        if !WorkspaceSandbox::is_same_or_inside(&canon_root, &cur) {
            return Err("directory escapes workspace root".into());
        }
    }
    let tex_target = cur.join(file_name);
    let tex_meta = fs::symlink_metadata(&tex_target)
        .map_err(|_| format!("target `.tex` file `{rel_path}` does not exist"))?;
    if is_symlink_or_junction(&tex_target, &tex_meta) || !tex_meta.is_file() {
        return Err(format!("target `{rel_path}` must be a regular non-symlink file"));
    }
    let tex_canon = fs::canonicalize(&tex_target).map_err(|e| e.to_string())?;
    if !WorkspaceSandbox::is_same_or_inside(&canon_root, &tex_canon) {
        return Err("target `.tex` file escapes workspace root".into());
    }

    let Some((compiler_bin, is_latexmk, compiler_label)) = resolve_compiler(env, engine) else {
        return Ok(json!({
            "ok": false,
            "status": "compiler_not_found",
            "compiler": Value::Null,
            "exit_code": Value::Null,
            "pdf_path": Value::Null,
            "errors": [],
            "warnings": [],
            "message": "Neither `pdflatex` nor `latexmk` was found on PATH. Local PDF compilation is unavailable; `tex_audit` and live math preview remain available."
        }));
    };

    let work_dir = strip_verbatim_prefix(&cur);
    let total_passes = if is_latexmk { 1 } else { runs };
    let mut last_exit: Option<i32> = None;
    let mut last_stdout = String::new();
    let mut last_stderr = String::new();
    let mut timed_out = false;
    let mut passes_done = 0usize;

    for _ in 0..total_passes {
        let mut cmd = Command::new(&compiler_bin);
        cmd.current_dir(&work_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("max_print_line", "1000")
            // Force TeX paranoid file-access mode (`p`) so `\input` / `\openin` / `\openout`
            // refuse absolute paths and `..` parent traversal outside `work_dir`/TEXMF,
            // and strip shell-escape env overrides alongside `-no-shell-escape`.
            .env("openin_any", "p")
            .env("openout_any", "p")
            .env_remove("shell_escape")
            .env_remove("shell_escape_commands");
        for k in SCRUBBED_KEY_ENVS {
            cmd.env_remove(k);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        if is_latexmk {
            cmd.args([
                "-pdf",
                "-interaction=nonstopmode",
                "-halt-on-error",
                "-no-shell-escape",
                "-file-line-error",
            ]);
        } else {
            cmd.args([
                "-interaction=nonstopmode",
                "-halt-on-error",
                "-no-shell-escape",
                "-file-line-error",
            ]);
        }
        cmd.arg(file_name.as_str());

        let outcome = run_bounded(&mut cmd, env.latex_compile_timeout)?;
        passes_done += 1;
        last_stdout = outcome.stdout;
        last_stderr = outcome.stderr;
        last_exit = outcome.exit_code;
        if outcome.timed_out {
            timed_out = true;
            break;
        }
        if last_exit != Some(0) {
            break;
        }
    }

    let stem = file_name
        .strip_suffix(".tex")
        .or_else(|| file_name.strip_suffix(".TEX"))
        .unwrap_or(file_name);
    let log_path = cur.join(format!("{stem}.log"));
    let log_text = read_bounded_lossy(&log_path, MAX_LOG_BYTES).unwrap_or_default();
    let combined_log = if log_text.is_empty() {
        format!("{last_stdout}\n{last_stderr}")
    } else {
        format!("{log_text}\n{last_stdout}")
    };
    let (errors, warnings) = parse_latex_diagnostics(&combined_log, &rel_path);

    let pdf_candidate = cur.join(format!("{stem}.pdf"));
    let pdf_rel_path = if !timed_out && last_exit == Some(0) {
        match fs::symlink_metadata(&pdf_candidate) {
            Ok(m) if m.is_file() && !is_symlink_or_junction(&pdf_candidate, &m) => {
                let mut pdf_comps = rel_components[..rel_components.len() - 1].to_vec();
                pdf_comps.push(format!("{stem}.pdf"));
                Some(pdf_comps.join("/"))
            }
            _ => None,
        }
    } else {
        None
    };

    let status = if timed_out {
        "timeout"
    } else if last_exit == Some(0) && pdf_rel_path.is_some() {
        "compiled"
    } else {
        "failed"
    };

    Ok(json!({
        "ok": status == "compiled",
        "status": status,
        "compiler": compiler_label,
        "exit_code": last_exit,
        "runs_completed": passes_done,
        "pdf_path": pdf_rel_path,
        "errors": errors,
        "warnings": warnings,
        "stdout_excerpt": tail_chars(&last_stdout, 4000),
        "stderr_excerpt": tail_chars(&last_stderr, 2000),
    }))
}

fn resolve_compiler(env: &ResearchEnv, requested: &str) -> Option<(PathBuf, bool, String)> {
    if let Some(override_bin) = &env.latex_compiler_override {
        if !override_bin.is_file() {
            return None;
        }
        let stem = override_bin.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let is_latexmk = requested == "latexmk" || stem.to_ascii_lowercase().contains("latexmk");
        return Some((override_bin.clone(), is_latexmk, stem.to_string()));
    }
    let candidates = if requested == "latexmk" {
        ["latexmk", "pdflatex"]
    } else {
        ["pdflatex", "latexmk"]
    };
    for name in candidates {
        if let Some(found) = find_on_path(name) {
            return Some((found, name == "latexmk", name.to_string()));
        }
    }
    None
}

fn find_on_path(bin: &str) -> Option<PathBuf> {
    let path_os = std::env::var_os("PATH")?;
    #[cfg(windows)]
    let exts: &[&str] = &[".exe", ".cmd", ".bat", ""];
    #[cfg(not(windows))]
    let exts: &[&str] = &[""];
    for dir in std::env::split_paths(&path_os) {
        for ext in exts {
            let candidate = dir.join(format!("{bin}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

struct SubprocessOutput {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

fn spawn_capped_reader<R: Read + Send + 'static>(mut pipe: R, max_bytes: usize) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buf = Vec::with_capacity(4096);
        let _ = (&mut pipe).take(max_bytes as u64).read_to_end(&mut buf);
        let _ = io::copy(&mut (&mut pipe).take(4 * 1024 * 1024), &mut io::sink());
        String::from_utf8_lossy(&buf).into_owned()
    })
}

fn run_bounded(cmd: &mut Command, timeout: Duration) -> Result<SubprocessOutput, String> {
    let mut child = cmd.spawn().map_err(|e| format!("failed to spawn LaTeX compiler: {e}"))?;
    let out_handle = spawn_capped_reader(child.stdout.take().expect("piped"), MAX_OUTPUT_BYTES);
    let err_handle = spawn_capped_reader(child.stderr.take().expect("piped"), MAX_OUTPUT_BYTES);

    let start = Instant::now();
    let mut timed_out = false;
    let exit_code = loop {
        match child.try_wait().map_err(|e| format!("wait error: {e}"))? {
            Some(status) => break status.code(),
            None if start.elapsed() >= timeout => {
                timed_out = true;
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(15)),
        }
    };
    let stdout = out_handle.join().unwrap_or_default();
    let stderr = err_handle.join().unwrap_or_default();
    Ok(SubprocessOutput { exit_code, stdout, stderr, timed_out })
}

pub fn parse_latex_diagnostics(log: &str, default_file: &str) -> (Vec<LatexDiagnostic>, Vec<LatexDiagnostic>) {
    static FILE_LINE_RE: OnceLock<Regex> = OnceLock::new();
    static WARN_LINE_RE: OnceLock<Regex> = OnceLock::new();
    let fl_re = FILE_LINE_RE.get_or_init(|| {
        Regex::new(r"^(?:\./)?([^:\r\n]+\.(?:tex|bib|sty|cls|bbl)):(\d+):\s*(.+)$").unwrap()
    });
    let warn_re = WARN_LINE_RE.get_or_init(|| {
        Regex::new(r"^LaTeX (?:Warning|Error):\s*(.+?)(?: on input line (\d+)\.?)?$").unwrap()
    });

    let lines: Vec<&str> = log.lines().collect();
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut seen_err = BTreeSet::new();
    let mut seen_warn = BTreeSet::new();

    for (i, raw_line) in lines.iter().enumerate() {
        let line = raw_line.trim();
        if let Some(rest) = line.strip_prefix("! ") {
            let msg = rest.trim().to_string();
            let mut found_line = None;
            for lookahead in lines.iter().skip(i + 1).take(8) {
                if let Some(after_l) = lookahead.trim_start().strip_prefix("l.") {
                    let digits: String = after_l.chars().take_while(|c| c.is_ascii_digit()).collect();
                    if let Ok(n) = digits.parse::<u32>() {
                        found_line = Some(n);
                        break;
                    }
                }
            }
            if errors.len() < 50 && seen_err.insert((found_line, msg.clone())) {
                errors.push(LatexDiagnostic {
                    file: Some(default_file.to_string()),
                    line: found_line,
                    message: msg,
                });
            }
        } else if let Some(caps) = fl_re.captures(line) {
            let file = caps[1].to_string();
            let ln = caps[2].parse::<u32>().ok();
            let msg = caps[3].trim().to_string();
            if errors.len() < 50 && seen_err.insert((ln, msg.clone())) {
                errors.push(LatexDiagnostic { file: Some(file), line: ln, message: msg });
            }
        } else if let Some(caps) = warn_re.captures(line) {
            let msg = caps[1].trim().to_string();
            let ln = caps.get(2).and_then(|m| m.as_str().parse::<u32>().ok());
            if warnings.len() < 50 && seen_warn.insert((ln, msg.clone())) {
                warnings.push(LatexDiagnostic {
                    file: Some(default_file.to_string()),
                    line: ln,
                    message: msg,
                });
            }
        }
    }
    (errors, warnings)
}

fn read_bounded_lossy(path: &Path, max_bytes: usize) -> Option<String> {
    let f = fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    f.take(max_bytes as u64).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

fn tail_chars(s: &str, max_chars: usize) -> String {
    let total = s.chars().count();
    if total <= max_chars {
        return s.to_string();
    }
    s.chars().skip(total - max_chars).collect()
}
