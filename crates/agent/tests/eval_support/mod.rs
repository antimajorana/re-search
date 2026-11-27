//! Evaluation harness (package F8, overview section 7): the manifest and
//! sheet formats, the measures of 7.3 with their gate, and the fixed-tier
//! plumbing (fixture loader, recorded-service mock, scripted model).
//!
//! A test file uses it with `mod support; mod eval_support;` (the scripted
//! model is built on `support`). `manifest`, `measures` and `sheet` are
//! pure and can be included from another crate's tests with `#[path]`.

pub mod manifest;
pub mod measures;
pub mod measures_catalog;
pub mod routed_mock;
pub mod scripted;
pub mod sheet;

use manifest::{parse_manifest, validate_manifests, Case, Manifest, Set, Tier};
use std::path::{Path, PathBuf};

/// The repository root, found from this crate's manifest directory
/// (`crates/agent`), so it is right on every platform and from any
/// working directory.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// `tests/fixtures/eval/` at the repository root: the fixed tier.
pub fn fixtures_dir() -> PathBuf {
    repo_root().join("tests").join("fixtures").join("eval")
}

/// `docs/design/eval-2026-11/`: where sheets are filed.
pub fn sheets_dir() -> PathBuf {
    repo_root().join("docs").join("design").join("eval-2026-11")
}

/// Turns a manifest fixture path (`p/x.tex`) into a path under `dir`.
pub fn fixture_path(dir: &Path, relative: &str) -> PathBuf {
    relative.split('/').fold(dir.to_path_buf(), |p, seg| p.join(seg))
}

/// Text of a fixture with line ends normalised, so byte offsets and
/// comparisons do not depend on how the checkout converts them.
pub fn read_fixture_text(dir: &Path, relative: &str) -> Result<String, String> {
    let path = fixture_path(dir, relative);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(text.replace("\r\n", "\n"))
}

/// Every manifest of a directory, loaded and checked.
#[derive(Debug)]
pub struct EvalSet {
    pub dir: PathBuf,
    pub manifests: Vec<Manifest>,
}

impl EvalSet {
    pub fn cases(&self) -> impl Iterator<Item = (Set, &Case)> {
        self.manifests.iter().flat_map(|m| m.cases.iter().map(move |c| (m.set, c)))
    }

    pub fn case(&self, id: &str) -> Option<(Set, &Case)> {
        self.cases().find(|(_, c)| c.id == id)
    }

    pub fn cases_of(&self, set: Set) -> Vec<&Case> {
        self.cases().filter(|(s, _)| *s == set).map(|(_, c)| c).collect()
    }
}

/// Loads every `manifest*.json` directly in `dir` (sorted by file name),
/// validates them together, requires every one to be of `tier`, and checks
/// that each fixture file a case names exists. All problems are returned
/// at once, each prefixed with its file.
pub fn load_manifests(dir: &Path, tier: Tier) -> Result<EvalSet, Vec<String>> {
    let mut names: Vec<String> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.starts_with("manifest") && n.ends_with(".json"))
            .collect(),
        Err(e) => return Err(vec![format!("cannot list {}: {e}", dir.display())]),
    };
    names.sort();
    if names.is_empty() {
        return Err(vec![format!("no manifest*.json in {}", dir.display())]);
    }
    let mut errs = Vec::new();
    let mut manifests = Vec::new();
    for name in &names {
        let parsed = std::fs::read_to_string(dir.join(name)).map_err(|e| format!("cannot read: {e}")).and_then(|t| parse_manifest(&t));
        match parsed {
            Ok(m) => {
                if m.tier != tier {
                    errs.push(format!("{name}: tier is {}, expected {}", m.tier.name(), tier.name()));
                }
                let expected = format!("manifest-{}.json", m.set.letter().to_ascii_lowercase());
                if *name != expected {
                    errs.push(format!("{name}: a manifest of set {} is named {expected}", m.set.letter()));
                }
                for case in &m.cases {
                    for rel in case.input.fixture_paths() {
                        if !fixture_path(dir, rel).is_file() {
                            errs.push(format!("{name}: {}: fixture `{rel}` does not exist", case.id));
                        }
                    }
                    for rel in case.input.fixture_dir_paths() {
                        if !fixture_path(dir, rel).is_dir() {
                            errs.push(format!("{name}: {}: fixture directory `{rel}` does not exist", case.id));
                        }
                    }
                    for rel in case.input.fixture_auxiliary_paths() {
                        if !fixture_path(dir, rel).exists() {
                            errs.push(format!("{name}: {}: auxiliary fixture `{rel}` does not exist", case.id));
                        }
                    }
                }
                manifests.push(m);
            }
            Err(e) => errs.push(format!("{name}: {e}")),
        }
    }
    errs.extend(validate_manifests(&manifests));
    if errs.is_empty() {
        Ok(EvalSet { dir: dir.to_path_buf(), manifests })
    } else {
        Err(errs)
    }
}
