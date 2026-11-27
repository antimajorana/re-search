//! Shared harness includes and read-only tree-comparison helpers for Set B (`eval_set_b.rs`).

#[allow(dead_code)]
#[path = "../../../agent/tests/eval_support/manifest.rs"]
pub mod manifest;
#[allow(dead_code)]
#[path = "../../../agent/tests/eval_support/measures_catalog.rs"]
pub mod measures_catalog;
#[allow(dead_code)]
#[path = "../../../agent/tests/eval_support/measures.rs"]
pub mod measures;

use research_tools::paper_sources::sha256_hex;
use std::fs;
use std::path::{Path, PathBuf};

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

pub fn eval_fixtures_dir() -> PathBuf {
    repo_root().join("tests").join("fixtures").join("eval")
}

pub fn copy_dir_recursive(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    let mut entries: Vec<_> = fs::read_dir(src).unwrap().flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let meta = fs::symlink_metadata(&from).unwrap();
        assert!(!meta.file_type().is_symlink(), "unexpected symlink in fixture `{}`", from.display());
        if meta.is_dir() {
            copy_dir_recursive(&from, &to);
        } else {
            fs::copy(&from, &to).unwrap();
        }
    }
}

/// Deterministically hashes a directory tree (paths, entry kinds, sizes, and file bytes).
pub fn hash_tree(root: &Path) -> String {
    let mut records = Vec::new();
    collect_tree(root, root, &mut records);
    records.sort();
    sha256_hex(records.join("\n").as_bytes())
}

fn collect_tree(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let mut entries: Vec<_> = fs::read_dir(dir).unwrap().flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let p = entry.path();
        let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
        let lmeta = fs::symlink_metadata(&p).unwrap();
        if lmeta.file_type().is_symlink() {
            let target = fs::read_link(&p).unwrap().to_string_lossy().into_owned();
            out.push(format!("L:{rel}->{target}"));
        } else if lmeta.is_dir() {
            out.push(format!("D:{rel}"));
            collect_tree(root, &p, out);
        } else {
            let bytes = fs::read(&p).unwrap();
            out.push(format!("F:{rel}:{}:{}", bytes.len(), sha256_hex(&bytes)));
        }
    }
}
