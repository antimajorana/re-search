//! Test support (feature `testkit`): a scripted fake vendor CLI.
//!
//! [`FakeCli::new`] compiles `fixtures/fake_cli.rs` once per test process with `rustc` and
//! copies the executable into a fresh directory per test. The fake reads its script
//! (`script.ndjson`, `version.txt`, ...) from files next to its copy and records its argv,
//! stdin and `--mcp-config` path there; see the fixture's header for the format.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

static COMPILED: OnceLock<Result<PathBuf, String>> = OnceLock::new();

fn compile() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("cli-route-fake-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join("fake_cli.rs");
    let exe = dir.join(format!("fake_cli{}", std::env::consts::EXE_SUFFIX));
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
        return Err(format!("rustc fake_cli.rs failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    Ok(exe)
}

/// One copy of the fake CLI in its own directory.
pub struct FakeCli {
    pub dir: PathBuf,
    pub exe: PathBuf,
}

impl FakeCli {
    /// A fresh copy under `parent/name`.
    pub fn new(parent: &Path, name: &str) -> FakeCli {
        let compiled = COMPILED.get_or_init(compile).clone().expect("compile fake CLI");
        let dir = parent.join(name);
        std::fs::create_dir_all(&dir).expect("fake CLI dir");
        let exe = dir.join(compiled.file_name().expect("exe name"));
        std::fs::copy(&compiled, &exe).expect("copy fake CLI");
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
