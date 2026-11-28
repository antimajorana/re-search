//! Contained Tesseract CLI OCR helper and platform-neutral [`OcrEngine`] trait.
//!
//! Each OCR invocation stages only the single rendered page PNG (and on Windows,
//! only the requested `.traineddata` language files) inside an ephemeral
//! per-page directory under `<data_dir>/ocr-stage/`, removes it via RAII on exit,
//! enforces a hard timeout, and bounds the returned text size.

use crate::sandbox::Containment;
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const DEFAULT_OCR_PAGE_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_OCR_JOB_MEMORY_LIMIT_BYTES: usize = 1024 * 1024 * 1024;
pub const MAX_OCR_INPUT_PNG_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_OCR_OUTPUT_TEXT_BYTES: usize = 2 * 1024 * 1024;
pub const MIN_OCR_DPI: u16 = 72;
pub const MAX_OCR_DPI: u16 = 600;

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const MAX_LANG_LEN: usize = 24;
#[cfg(not(windows))]
const MAX_STDERR_BYTES: usize = 16 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(10);
static STAGE_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrPageInput {
    pub page: usize,
    pub png_bytes: Vec<u8>,
    pub dpi: u16,
    pub langs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrPageResult {
    pub text: String,
    pub engine_id: String,
    pub containment: Containment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OcrHelperError {
    InvalidInput(String),
    Unavailable(String),
    Spawn(String),
    Timeout(Duration),
    OutputTooLarge { max_bytes: usize },
    ProcessFailed { exit_code: Option<i32>, stderr: String },
}

impl fmt::Display for OcrHelperError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(msg) => write!(f, "invalid OCR input: {msg}"),
            Self::Unavailable(msg) => write!(f, "OCR engine unavailable: {msg}"),
            Self::Spawn(msg) => write!(f, "failed to launch OCR helper: {msg}"),
            Self::Timeout(d) => write!(f, "OCR helper timed out after {} ms", d.as_millis()),
            Self::OutputTooLarge { max_bytes } => write!(f, "OCR output exceeded limit of {max_bytes} bytes"),
            Self::ProcessFailed { exit_code, stderr } if stderr.is_empty() => {
                write!(f, "OCR helper failed (exit code: {exit_code:?})")
            }
            Self::ProcessFailed { exit_code, stderr } => {
                write!(f, "OCR helper failed (exit code: {exit_code:?}): {stderr}")
            }
        }
    }
}

impl std::error::Error for OcrHelperError {}

impl From<String> for OcrHelperError {
    fn from(stderr: String) -> Self {
        Self::ProcessFailed { exit_code: Some(1), stderr }
    }
}

impl From<&str> for OcrHelperError {
    fn from(stderr: &str) -> Self {
        Self::ProcessFailed { exit_code: Some(1), stderr: stderr.to_string() }
    }
}

/// Platform-neutral interface for single-page OCR execution.
pub trait OcrEngine: Send + Sync {
    fn engine_id(&self) -> &str;
    fn containment(&self) -> Containment;
    fn ocr_page(&self, input: &OcrPageInput) -> Result<OcrPageResult, OcrHelperError>;
}

fn validate_lang_tag(lang: &str) -> Result<(), OcrHelperError> {
    if lang.is_empty() || lang.len() > MAX_LANG_LEN {
        return Err(OcrHelperError::InvalidInput(format!("language tag `{lang}` must be 1..={MAX_LANG_LEN} chars")));
    }
    if lang.starts_with('-') {
        return Err(OcrHelperError::InvalidInput(format!("language tag `{lang}` must not start with `-`")));
    }
    if !lang.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
        return Err(OcrHelperError::InvalidInput(format!("language tag `{lang}` contains disallowed characters")));
    }
    Ok(())
}

fn validate_ocr_input_parts(input: &OcrPageInput) -> Result<(Vec<String>, String), OcrHelperError> {
    if input.page == 0 {
        return Err(OcrHelperError::InvalidInput("page number must be >= 1".into()));
    }
    if input.png_bytes.is_empty() {
        return Err(OcrHelperError::InvalidInput("PNG input is empty".into()));
    }
    if input.png_bytes.len() > MAX_OCR_INPUT_PNG_BYTES {
        return Err(OcrHelperError::InvalidInput(format!(
            "PNG input ({} bytes) exceeds limit of {MAX_OCR_INPUT_PNG_BYTES} bytes",
            input.png_bytes.len()
        )));
    }
    if !input.png_bytes.starts_with(PNG_SIGNATURE) {
        return Err(OcrHelperError::InvalidInput("input is missing the 8-byte PNG signature".into()));
    }
    if !(MIN_OCR_DPI..=MAX_OCR_DPI).contains(&input.dpi) {
        return Err(OcrHelperError::InvalidInput(format!(
            "DPI {} outside allowed range {MIN_OCR_DPI}..={MAX_OCR_DPI}",
            input.dpi
        )));
    }
    if input.langs.is_empty() {
        return Ok((vec!["eng".to_string()], "eng".to_string()));
    }
    for lang in &input.langs {
        validate_lang_tag(lang)?;
    }
    Ok((input.langs.clone(), input.langs.join("+")))
}

/// Validate an [`OcrPageInput`] and return the normalized Tesseract `-l` language spec
/// (e.g. `"eng"` or `"eng+deu"`).
pub fn validate_ocr_input(input: &OcrPageInput) -> Result<String, OcrHelperError> {
    validate_ocr_input_parts(input).map(|(_, spec)| spec)
}

pub(crate) fn normalize_ocr_text(raw: &str) -> String {
    raw.replace('\0', "").replace("\r\n", "\n").replace('\r', "\n")
}

pub(crate) fn read_bounded_text(path: &Path, max_bytes: usize) -> Result<String, OcrHelperError> {
    let file = fs::File::open(path).map_err(|e| OcrHelperError::ProcessFailed {
        exit_code: None,
        stderr: format!("OCR output file unreadable ({}): {e}", path.display()),
    })?;
    let mut buf = Vec::new();
    file.take(max_bytes.saturating_add(1) as u64).read_to_end(&mut buf).map_err(|e| {
        OcrHelperError::ProcessFailed {
            exit_code: None,
            stderr: format!("failed reading OCR output ({}): {e}", path.display()),
        }
    })?;
    if buf.len() > max_bytes {
        return Err(OcrHelperError::OutputTooLarge { max_bytes });
    }
    Ok(normalize_ocr_text(&String::from_utf8_lossy(&buf)))
}

#[cfg(not(windows))]
fn read_bounded_stderr(path: &Path) -> String {
    let Ok(file) = fs::File::open(path) else {
        return String::new();
    };
    let mut buf = Vec::new();
    let _ = file.take(MAX_STDERR_BYTES as u64).read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).trim().to_string()
}

struct OcrStageGuard(PathBuf);

impl Drop for OcrStageGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn create_stage_dir(data_dir: &Path) -> Result<(PathBuf, String, OcrStageGuard), OcrHelperError> {
    let spawn_err = |e: std::io::Error| OcrHelperError::Spawn(e.to_string());
    let seq = STAGE_SEQ.fetch_add(1, Ordering::Relaxed);
    let run_id = format!("ocr-{}-{seq}", std::process::id());
    let stage_dir = data_dir.join("ocr-stage").join(&run_id);
    fs::create_dir_all(&stage_dir).map_err(spawn_err)?;
    let guard = OcrStageGuard(stage_dir.clone());
    fs::create_dir_all(stage_dir.join("out")).map_err(spawn_err)?;
    fs::create_dir_all(stage_dir.join("tmp")).map_err(spawn_err)?;
    Ok((stage_dir, run_id, guard))
}

fn wait_with_timeout<C, T>(
    child: &mut C,
    timeout: Duration,
    mut try_wait: impl FnMut(&mut C) -> std::io::Result<Option<T>>,
    mut kill: impl FnMut(&mut C),
) -> Result<T, OcrHelperError> {
    let start = Instant::now();
    loop {
        match try_wait(child) {
            Ok(Some(res)) => return Ok(res),
            Ok(None) if start.elapsed() >= timeout => {
                kill(child);
                return Err(OcrHelperError::Timeout(timeout));
            }
            Ok(None) => std::thread::sleep(POLL_INTERVAL),
            Err(e) => {
                kill(child);
                return Err(OcrHelperError::Spawn(format!("failed waiting on OCR helper: {e}")));
            }
        }
    }
}

#[cfg(windows)]
fn path_str(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[derive(Debug, Clone)]
pub struct TesseractCliConfig {
    pub tesseract_exe: PathBuf,
    pub tessdata_dir: PathBuf,
    pub data_dir: PathBuf,
    pub engine_id: String,
    pub page_timeout: Duration,
    pub max_output_bytes: usize,
}

#[derive(Debug, Clone)]
pub struct TesseractCliEngine {
    config: TesseractCliConfig,
}

impl TesseractCliEngine {
    pub fn new(config: TesseractCliConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &TesseractCliConfig {
        &self.config
    }

    fn verify_available(&self) -> Result<(), OcrHelperError> {
        if !self.config.tesseract_exe.is_file() || !self.config.tessdata_dir.is_dir() {
            return Err(OcrHelperError::Unavailable("Install Tesseract to enable OCR".into()));
        }
        Ok(())
    }

    #[cfg(windows)]
    fn run_tesseract(
        &self,
        input: &OcrPageInput,
        langs: &[String],
        lang_spec: &str,
    ) -> Result<OcrPageResult, OcrHelperError> {
        use crate::appcontainer::AppContainerProfile;
        use crate::sandbox::{Contained, WorkerLaunch};
        use crate::stage::{grant_icacls_inside, strip_verbatim};
        use crate::win::spawn_extended;

        let spawn_err = |e: std::io::Error| OcrHelperError::Spawn(e.to_string());
        for lang in langs {
            if !self.config.tessdata_dir.join(format!("{lang}.traineddata")).is_file() {
                return Err(OcrHelperError::Unavailable("Install Tesseract to enable OCR".into()));
            }
        }

        let exe_clean = strip_verbatim(&fs::canonicalize(&self.config.tesseract_exe).map_err(spawn_err)?);
        if !exe_clean.is_file() {
            return Err(OcrHelperError::Unavailable("Install Tesseract to enable OCR".into()));
        }

        let (stage_dir, profile_id, _stage_guard) = create_stage_dir(&self.config.data_dir)?;
        let profile = AppContainerProfile::create(&profile_id).map_err(spawn_err)?;
        let rx_grant = format!("*{}:(OI)(CI)RX", profile.sid_str());
        let modify_grant = format!("*{}:(OI)(CI)M", profile.sid_str());
        grant_icacls_inside(&self.config.data_dir, &stage_dir, &rx_grant).map_err(spawn_err)?;
        grant_icacls_inside(&self.config.data_dir, &stage_dir.join("out"), &modify_grant).map_err(spawn_err)?;
        grant_icacls_inside(&self.config.data_dir, &stage_dir.join("tmp"), &modify_grant).map_err(spawn_err)?;

        let stage_clean = strip_verbatim(&fs::canonicalize(&stage_dir).map_err(spawn_err)?);
        let out_clean = stage_clean.join("out");
        let tmp_clean = stage_clean.join("tmp");
        let staged_tessdata = stage_clean.join("tessdata");
        fs::create_dir_all(&staged_tessdata).map_err(spawn_err)?;

        let page_png = stage_clean.join("page.png");
        fs::write(&page_png, &input.png_bytes).map_err(spawn_err)?;
        for lang in langs {
            let filename = format!("{lang}.traineddata");
            let dst = staged_tessdata.join(&filename);
            if !dst.exists() {
                fs::copy(self.config.tessdata_dir.join(&filename), &dst).map_err(spawn_err)?;
            }
        }

        let system_root = std::env::var("SystemRoot")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| r"C:\Windows".to_string());
        let mut env = BTreeMap::new();
        env.insert("SystemRoot".into(), system_root);
        env.insert("TEMP".into(), path_str(&tmp_clean));
        env.insert("TMP".into(), path_str(&tmp_clean));
        env.insert("TESSDATA_PREFIX".into(), path_str(&staged_tessdata));
        env.insert("OMP_THREAD_LIMIT".into(), "1".into());

        let out_base = out_clean.join("result");
        let launch = WorkerLaunch {
            exe: exe_clean,
            args: vec![
                path_str(&page_png),
                path_str(&out_base),
                "--tessdata-dir".into(),
                path_str(&staged_tessdata),
                "-l".into(),
                lang_spec.into(),
                "--dpi".into(),
                input.dpi.to_string(),
            ],
            cwd: out_clean.clone(),
            env,
            inherit_handles: Vec::new(),
            job_memory_limit_bytes: Some(DEFAULT_OCR_JOB_MEMORY_LIMIT_BYTES),
        };

        let mut child = spawn_extended(&launch, Some(&profile)).map_err(spawn_err)?;
        let exit_code = wait_with_timeout(&mut child, self.config.page_timeout, |c| c.try_wait(), |c| c.kill_tree())?;
        drop(child);

        if exit_code != 0 {
            return Err(OcrHelperError::ProcessFailed {
                exit_code: Some(exit_code as i32),
                stderr: format!("tesseract exited with status {exit_code}"),
            });
        }
        let text = read_bounded_text(&out_clean.join("result.txt"), self.config.max_output_bytes)?;
        Ok(OcrPageResult { text, engine_id: self.config.engine_id.clone(), containment: self.containment() })
    }

    #[cfg(not(windows))]
    fn run_tesseract(
        &self,
        input: &OcrPageInput,
        langs: &[String],
        lang_spec: &str,
    ) -> Result<OcrPageResult, OcrHelperError> {
        use std::process::{Command, Stdio};

        for lang in langs {
            if !self.config.tessdata_dir.join(format!("{lang}.traineddata")).is_file() {
                return Err(OcrHelperError::Unavailable("Install Tesseract to enable OCR".into()));
            }
        }
        let spawn_err = |e: std::io::Error| OcrHelperError::Spawn(e.to_string());
        let (stage_dir, _run_id, _stage_guard) = create_stage_dir(&self.config.data_dir)?;
        let stage_clean = fs::canonicalize(&stage_dir).map_err(spawn_err)?;
        let out_dir = stage_clean.join("out");
        let tmp_dir = stage_clean.join("tmp");
        let page_png = stage_clean.join("page.png");
        fs::write(&page_png, &input.png_bytes).map_err(spawn_err)?;

        let exe_path = std::path::absolute(&self.config.tesseract_exe).map_err(spawn_err)?;
        let tessdata_dir = fs::canonicalize(&self.config.tessdata_dir).map_err(spawn_err)?;
        let stderr_path = out_dir.join("stderr.log");
        let stderr_file = fs::File::create(&stderr_path).map_err(spawn_err)?;
        let out_base = out_dir.join("result");

        let mut cmd = Command::new(&exe_path);
        cmd.env_clear()
            .env("TESSDATA_PREFIX", &tessdata_dir)
            .env("OMP_THREAD_LIMIT", "1")
            .env("LC_ALL", "C.UTF-8")
            .env("TMPDIR", &tmp_dir)
            .env("TEMP", &tmp_dir)
            .env("TMP", &tmp_dir)
            .current_dir(&out_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr_file))
            .args([page_png.as_os_str(), out_base.as_os_str()])
            .arg("--tessdata-dir")
            .arg(&tessdata_dir)
            .args(["-l", lang_spec, "--dpi"])
            .arg(input.dpi.to_string());
        if let Some(path) = std::env::var_os("PATH") {
            cmd.env("PATH", path);
        }

        let mut child = cmd.spawn().map_err(spawn_err)?;
        let status = wait_with_timeout(&mut child, self.config.page_timeout, |c| c.try_wait(), |c| {
            let _ = c.kill();
            let _ = c.wait();
        })?;
        if !status.success() {
            return Err(OcrHelperError::ProcessFailed {
                exit_code: status.code(),
                stderr: read_bounded_stderr(&stderr_path),
            });
        }
        let text = read_bounded_text(&out_dir.join("result.txt"), self.config.max_output_bytes)?;
        Ok(OcrPageResult { text, engine_id: self.config.engine_id.clone(), containment: self.containment() })
    }
}

impl OcrEngine for TesseractCliEngine {
    fn engine_id(&self) -> &str {
        &self.config.engine_id
    }

    fn containment(&self) -> Containment {
        #[cfg(windows)]
        {
            Containment {
                backend: "appcontainer-ocr",
                filesystem_restricted: true,
                network_restricted: true,
                process_tree_killed_with_core: true,
            }
        }
        #[cfg(not(windows))]
        {
            Containment {
                backend: "isolated-tempdir",
                filesystem_restricted: false,
                network_restricted: false,
                process_tree_killed_with_core: false,
            }
        }
    }

    fn ocr_page(&self, input: &OcrPageInput) -> Result<OcrPageResult, OcrHelperError> {
        let (langs, lang_spec) = validate_ocr_input_parts(input)?;
        self.verify_available()?;
        self.run_tesseract(input, &langs, &lang_spec)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeOcrCall {
    pub page: usize,
    pub dpi: u16,
    pub langs: Vec<String>,
    pub png_len: usize,
}

/// Deterministic in-memory [`OcrEngine`] for tests that still enforces [`validate_ocr_input`].
#[derive(Debug, Clone)]
pub struct FakeOcrEngine {
    engine_id: String,
    responses: BTreeMap<usize, Result<String, OcrHelperError>>,
    calls: Arc<Mutex<Vec<FakeOcrCall>>>,
}

impl FakeOcrEngine {
    pub fn new(engine_id: impl Into<String>) -> Self {
        Self { engine_id: engine_id.into(), responses: BTreeMap::new(), calls: Arc::new(Mutex::new(Vec::new())) }
    }

    pub fn with_page(mut self, page: usize, text: impl Into<String>) -> Self {
        self.responses.insert(page, Ok(text.into()));
        self
    }

    pub fn with_page_error(mut self, page: usize, err: impl Into<OcrHelperError>) -> Self {
        self.responses.insert(page, Err(err.into()));
        self
    }

    pub fn calls(&self) -> Vec<FakeOcrCall> {
        match self.calls.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

impl OcrEngine for FakeOcrEngine {
    fn engine_id(&self) -> &str {
        &self.engine_id
    }

    fn containment(&self) -> Containment {
        Containment {
            backend: "fake-ocr",
            filesystem_restricted: true,
            network_restricted: true,
            process_tree_killed_with_core: true,
        }
    }

    fn ocr_page(&self, input: &OcrPageInput) -> Result<OcrPageResult, OcrHelperError> {
        let _ = validate_ocr_input(input)?;
        let record = FakeOcrCall {
            page: input.page,
            dpi: input.dpi,
            langs: input.langs.clone(),
            png_len: input.png_bytes.len(),
        };
        match self.calls.lock() {
            Ok(mut guard) => guard.push(record),
            Err(poisoned) => poisoned.into_inner().push(record),
        }
        let text = match self.responses.get(&input.page) {
            Some(Ok(text)) => normalize_ocr_text(text),
            Some(Err(err)) => return Err(err.clone()),
            None => String::new(),
        };
        Ok(OcrPageResult { text, engine_id: self.engine_id.clone(), containment: self.containment() })
    }
}

#[cfg(test)]
#[path = "ocr_helper_tests.rs"]
mod tests;
