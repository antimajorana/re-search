use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;

/// Resolve the workspace's current commit without consulting worktree filters.
pub fn workspace_revision(workspace: &Path) -> Option<Value> {
    let output = Command::new("git")
        .current_dir(workspace)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .args(["rev-parse", "--verify", "HEAD^{commit}"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let commit = std::str::from_utf8(&output.stdout).ok()?.trim();
    if !matches!(commit.len(), 40 | 64) || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(json!({"git_commit": commit, "dirty": null}))
}
