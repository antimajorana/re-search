//! Test support (feature `testkit`): scripted fake vendor CLIs.
//!
//! [`FakeCli::new`] compiles `fixtures/fake_cli.rs` (a Claude Code stand-in) and
//! [`FakeCli::new_agy`] compiles `fixtures/fake_agy.rs` (an Antigravity CLI stand-in), each
//! once per test process with `rustc`, and copies the executable into a fresh directory per
//! test. The fake reads its script (`script.ndjson`, `version.txt`, ...) from files next to
//! its copy and records its argv, stdin and tool-config path there; see the fixtures'
//! headers for the format.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

static COMPILED: OnceLock<Result<PathBuf, String>> = OnceLock::new();
static COMPILED_AGY: OnceLock<Result<PathBuf, String>> = OnceLock::new();

fn compile(fixture: &str, out_name: &str) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("cli-route-{out_name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(fixture);
    let exe = dir.join(format!("{out_name}{}", std::env::consts::EXE_SUFFIX));
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let out = Command::new(rustc)
        .arg("--edition")
        .arg("2021")
        .arg(&src)
        .arg("-o")
        .arg(&exe)
        .output()
        .map_err(|e| format!("could not run rustc: {e}"))?;
    if !out.status.success() {
        return Err(format!("rustc {fixture} failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    Ok(exe)
}

/// One copy of a fake CLI in its own directory.
pub struct FakeCli {
    pub dir: PathBuf,
    pub exe: PathBuf,
}

impl FakeCli {
    /// A fresh copy of the fake Claude Code CLI under `parent/name`.
    pub fn new(parent: &Path, name: &str) -> FakeCli {
        let compiled = COMPILED.get_or_init(|| compile("fake_cli.rs", "fake_cli")).clone().expect("compile fake CLI");
        Self::copy(&compiled, parent, name)
    }

    /// A fresh copy of the fake Antigravity CLI (`agy.exe`) under `parent/name`.
    pub fn new_agy(parent: &Path, name: &str) -> FakeCli {
        let compiled = COMPILED_AGY.get_or_init(|| compile("fake_agy.rs", "fake_agy")).clone().expect("compile fake agy");
        Self::copy(&compiled, parent, name)
    }

    fn copy(compiled: &Path, parent: &Path, name: &str) -> FakeCli {
        let dir = parent.join(name);
        std::fs::create_dir_all(&dir).expect("fake CLI dir");
        let exe = dir.join(compiled.file_name().expect("exe name"));
        std::fs::copy(compiled, &exe).expect("copy fake CLI");
        FakeCli { dir, exe }
    }

    /// Lines to print (see the fixture for `@approval` and `@sleep MS`).
    pub fn script(&self, lines: &[String]) -> &Self {
        self.write("script.ndjson", &lines.join("\n"))
    }

    pub fn write(&self, name: &str, text: &str) -> &Self {
        std::fs::write(self.dir.join(name), text).expect("write fake CLI file");
        self
    }

    pub fn read(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.dir.join(name)).ok()
    }

    /// Arguments the fake was started with (`None` if it never ran a turn).
    pub fn argv(&self) -> Option<Vec<String>> {
        self.read("argv.txt").map(|s| s.split('\n').map(str::to_string).collect())
    }

    pub fn stdin(&self) -> Option<String> {
        self.read("stdin.txt")
    }
}
