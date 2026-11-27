//! Solve scratch artifact harvesting (`solve.artifact` v1) and read-only user input staging
//! (package S18 Part A; `solve.md` 4.1, 5.6).
//!
//! Scans `scratch_dir/artifacts/` with strict symlink/junction and traversal guards, enforces
//! the per-run caps (`16` files, `20 MiB` total), sniffs MIME strictly by magic bytes (ignoring
//! filename extensions), stores verified bytes in the content-addressed ledger blob store, and
//! appends host-only `solve.artifact` v1 events compatible with [`crate::solve_state::SolveState`].

use crate::sandbox::{
    is_symlink_or_junction, sanitize_rel_components, validate_session_id, SandboxError,
    WorkspaceSandbox,
};
use crate::solve_model::{event_kinds, SolveArtifact};
use crate::solve_state::SolveState;
use ledger::{sha256_hex, Error as LedgerError, Ledger, GENESIS_HASH};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Subdirectory inside a worker `scratch_dir` where Solve artifacts are written.
pub const ARTIFACTS_SUBDIR: &str = "artifacts";
/// Subdirectory inside a worker `scratch_dir` where user data files are staged read-only.
pub const INPUTS_SUBDIR: &str = "inputs";
/// Maximum harvested artifacts allowed per Solve run (`solve.md` 5.6).
pub const MAX_ARTIFACTS_PER_RUN: usize = 16;
/// Maximum cumulative artifact bytes allowed per Solve run (`20 MiB`, `solve.md` 5.6).
pub const MAX_ARTIFACT_BYTES_PER_RUN: u64 = 20 * 1024 * 1024;
/// Maximum size per staged read-only input file (`20 MiB`).
pub const MAX_STAGED_INPUT_BYTES: u64 = 20 * 1024 * 1024;
/// Schema version for `solve.artifact` ledger events (`solve.md` 4.1).
pub const SOLVE_ARTIFACT_SCHEMA_VERSION: u32 = 1;
/// Maximum relative directory depth scanned under `scratch_dir/artifacts/`.
pub const MAX_ARTIFACT_SCAN_DEPTH: usize = 4;

pub use crate::solve_artifacts_sniff::{
    sniff_artifact_mime, MIME_CSV, MIME_JSON, MIME_PDF, MIME_PLAIN, MIME_PNG, MIME_SVG,
};

/// One artifact successfully harvested into the ledger blob store and `solve.artifact`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarvestedArtifact {
    pub seq: u64,
    pub artifact: SolveArtifact,
}

/// One candidate artifact file rejected during harvesting without crashing the run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactWarning {
    pub name: String,
    pub reason: String,
}

/// Outcome of harvesting `scratch_dir/artifacts/` into a Solve session ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarvestReport {
    pub harvested: Vec<HarvestedArtifact>,
    pub warnings: Vec<ArtifactWarning>,
    pub total_artifacts_in_run: usize,
    pub total_bytes_in_run: u64,
}

/// User data file specification for read-only staging into `scratch_dir/inputs/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StagedInputSpec {
    pub name: String,
    pub source_path: PathBuf,
}

/// Receipt for one user data file copied read-only into `scratch_dir/inputs/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StagedInputFile {
    pub name: String,
    pub staged_path: PathBuf,
    pub sha256: String,
    pub bytes: u64,
}

/// Validate a relative artifact or staged-input path, rejecting traversal, absolute paths,
/// Windows reserved stems, and control characters via [`crate::sandbox::sanitize_rel_components`].
pub fn validate_artifact_rel_path(rel: &str) -> Result<Vec<String>, SandboxError> {
    let (parts, _) = sanitize_rel_components(rel, false)?;
    if parts.iter().any(|seg| seg.starts_with(".re-search-tmp-")) {
        return Err(SandboxError::InvalidPath(format!("reserved temp prefix in `{rel}`")));
    }
    Ok(parts)
}

fn validate_non_symlink_dir(dir: &Path, label: &str) -> Result<PathBuf, SandboxError> {
    let meta = fs::symlink_metadata(dir).map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            SandboxError::NotFound(format!("{label}: {}", dir.display()))
        } else {
            SandboxError::Io(e.to_string())
        }
    })?;
    if is_symlink_or_junction(dir, &meta) {
        return Err(SandboxError::EscapesRoot(format!("symlink or junction at {label}")));
    }
    if !meta.is_dir() {
        return Err(SandboxError::NotADirectory(format!("{label}: {}", dir.display())));
    }
    WorkspaceSandbox::canonicalize_existing_dir(dir)
}

fn collect_artifact_candidates(
    canon_root: &Path,
    dir: &Path,
    rel_prefix: &str,
    depth: usize,
    files: &mut Vec<(String, PathBuf, fs::Metadata)>,
    warnings: &mut Vec<ArtifactWarning>,
) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("failed to read `{rel_prefix}`: {e}"))?;
    let mut named: Vec<(String, PathBuf)> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("failed reading artifact dir entry: {e}"))?;
        let os_name = entry.file_name();
        let Some(name_str) = os_name.to_str() else {
            warnings.push(ArtifactWarning {
                name: os_name.to_string_lossy().into_owned(),
                reason: "non-UTF-8 artifact filename rejected".into(),
            });
            continue;
        };
        if name_str.starts_with(".re-search-tmp-") {
            continue;
        }
        named.push((name_str.to_string(), entry.path()));
    }
    named.sort_by(|a, b| a.0.cmp(&b.0));

    for (seg, path) in named {
        let rel = if rel_prefix.is_empty() {
            seg.clone()
        } else {
            format!("{rel_prefix}/{seg}")
        };
        let lmeta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                warnings.push(ArtifactWarning {
                    name: rel,
                    reason: format!("cannot inspect artifact entry: {e}"),
                });
                continue;
            }
        };
        if is_symlink_or_junction(&path, &lmeta) {
            warnings.push(ArtifactWarning {
                name: rel,
                reason: "symlinks and junctions are refused in scratch/artifacts".into(),
            });
            continue;
        }
        if let Err(e) = validate_artifact_rel_path(&rel) {
            warnings.push(ArtifactWarning {
                name: rel,
                reason: format!("unsafe artifact path: {e}"),
            });
            continue;
        }
        let canon_entry = match fs::canonicalize(&path) {
            Ok(p) => p,
            Err(e) => {
                warnings.push(ArtifactWarning {
                    name: rel,
                    reason: format!("cannot resolve artifact path: {e}"),
                });
                continue;
            }
        };
        if !WorkspaceSandbox::is_same_or_inside(canon_root, &canon_entry) {
            warnings.push(ArtifactWarning {
                name: rel,
                reason: "artifact path escapes scratch/artifacts root".into(),
            });
            continue;
        }
        if lmeta.is_dir() {
            if depth >= MAX_ARTIFACT_SCAN_DEPTH {
                warnings.push(ArtifactWarning {
                    name: rel,
                    reason: format!("artifact subdirectory exceeds max depth {MAX_ARTIFACT_SCAN_DEPTH}"),
                });
                continue;
            }
            collect_artifact_candidates(canon_root, &canon_entry, &rel, depth + 1, files, warnings)?;
        } else if lmeta.is_file() {
            files.push((rel, canon_entry, lmeta));
        } else {
            warnings.push(ArtifactWarning {
                name: rel,
                reason: "non-regular file refused in scratch/artifacts".into(),
            });
        }
    }
    Ok(())
}

struct PendingArtifactWrite {
    artifact: SolveArtifact,
    bytes: Vec<u8>,
}

/// Harvest files written under `scratch_dir/artifacts/` into `ledger` as `solve.artifact` v1
/// events (`solve.md` 4.1, 5.6).
///
/// Validates paths, rejects symlinks/junctions, enforces per-run caps (`16` files / `20 MiB`
/// total), sniffs MIME by magic bytes (ignoring extensions), and writes verified blobs before
/// appending events atomically with `append_at_head`.
pub fn harvest_scratch_artifacts(
    ledger: &Ledger,
    session_id: &str,
    scratch_dir: &Path,
    call_id: &str,
    entry_id: Option<&str>,
) -> Result<HarvestReport, String> {
    validate_session_id(session_id).map_err(|e| e.to_string())?;
    let call_id = call_id.trim();
    if call_id.is_empty() {
        return Err("call_id must not be empty".into());
    }
    let clean_entry_id = match entry_id {
        Some(eid) => {
            let trimmed = eid.trim();
            if trimmed.is_empty() {
                return Err("entry_id must not be empty when provided".into());
            }
            Some(trimmed.to_string())
        }
        None => None,
    };

    let canon_scratch = validate_non_symlink_dir(scratch_dir, "scratch_dir")
        .map_err(|e| format!("invalid scratch directory: {e}"))?;
    let (mut head_hash, events) = match ledger.read_verified_session(session_id) {
        Ok((report, evs)) => (report.head_hash, evs),
        Err(LedgerError::UnknownSession(_)) => (GENESIS_HASH.to_string(), Vec::new()),
        Err(e) => return Err(format!("cannot verify session `{session_id}` before harvesting: {e}")),
    };
    let state = SolveState::from_events(&events);
    let mut total_artifacts = state.artifacts.len();
    let mut total_bytes: u64 = state
        .artifacts
        .iter()
        .fold(0u64, |acc, a| acc.saturating_add(a.bytes));

    let artifacts_path = canon_scratch.join(ARTIFACTS_SUBDIR);
    let artifacts_lmeta = match fs::symlink_metadata(&artifacts_path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(HarvestReport {
                harvested: Vec::new(),
                warnings: Vec::new(),
                total_artifacts_in_run: total_artifacts,
                total_bytes_in_run: total_bytes,
            });
        }
        Err(e) => return Err(format!("cannot inspect scratch/artifacts: {e}")),
    };
    if is_symlink_or_junction(&artifacts_path, &artifacts_lmeta) {
        return Err("symlink or junction refused at scratch/artifacts".into());
    }
    if !artifacts_lmeta.is_dir() {
        return Err("scratch/artifacts exists but is not a directory".into());
    }
    let canon_artifacts = WorkspaceSandbox::canonicalize_existing_dir(&artifacts_path)
        .map_err(|e| format!("cannot canonicalize scratch/artifacts: {e}"))?;
    if !WorkspaceSandbox::is_same_or_inside(&canon_scratch, &canon_artifacts) {
        return Err("scratch/artifacts escapes scratch_dir".into());
    }

    let mut candidates = Vec::new();
    let mut warnings = Vec::new();
    collect_artifact_candidates(
        &canon_artifacts,
        &canon_artifacts,
        "",
        0,
        &mut candidates,
        &mut warnings,
    )?;

    let mut existing_pairs: BTreeSet<(String, String)> = state
        .artifacts
        .iter()
        .map(|a| (a.name.clone(), a.sha256.clone()))
        .collect();
    let mut next_num = state
        .artifacts
        .iter()
        .filter_map(|a| a.artifact_id.strip_prefix("art-").and_then(|s| s.parse::<u64>().ok()))
        .max()
        .unwrap_or(state.artifacts.len() as u64);

    let mut pending = Vec::new();
    for (rel_name, file_path, lmeta) in candidates {
        let len_hint = lmeta.len();
        if len_hint > MAX_ARTIFACT_BYTES_PER_RUN {
            warnings.push(ArtifactWarning {
                name: rel_name,
                reason: format!(
                    "artifact ({len_hint} bytes) exceeds per-run byte cap ({MAX_ARTIFACT_BYTES_PER_RUN} bytes)"
                ),
            });
            continue;
        }
        let maybe_same = state
            .artifacts
            .iter()
            .any(|a| a.name == rel_name && a.bytes == len_hint);
        if !maybe_same && total_artifacts >= MAX_ARTIFACTS_PER_RUN {
            warnings.push(ArtifactWarning {
                name: rel_name,
                reason: format!("per-run artifact file cap reached ({MAX_ARTIFACTS_PER_RUN} files)"),
            });
            continue;
        }
        if !maybe_same && total_bytes.saturating_add(len_hint) > MAX_ARTIFACT_BYTES_PER_RUN {
            warnings.push(ArtifactWarning {
                name: rel_name,
                reason: format!(
                    "per-run artifact byte cap exceeded ({total_bytes} + {len_hint} > {MAX_ARTIFACT_BYTES_PER_RUN} bytes)"
                ),
            });
            continue;
        }
        let bytes = match fs::read(&file_path) {
            Ok(b) => b,
            Err(e) => {
                warnings.push(ArtifactWarning {
                    name: rel_name,
                    reason: format!("cannot read artifact file: {e}"),
                });
                continue;
            }
        };
        let actual_len = bytes.len() as u64;
        let sha256 = sha256_hex(&bytes);
        if existing_pairs.contains(&(rel_name.clone(), sha256.clone())) {
            continue;
        }
        if total_artifacts >= MAX_ARTIFACTS_PER_RUN {
            warnings.push(ArtifactWarning {
                name: rel_name,
                reason: format!("per-run artifact file cap reached ({MAX_ARTIFACTS_PER_RUN} files)"),
            });
            continue;
        }
        if total_bytes.saturating_add(actual_len) > MAX_ARTIFACT_BYTES_PER_RUN {
            warnings.push(ArtifactWarning {
                name: rel_name,
                reason: format!(
                    "per-run artifact byte cap exceeded ({total_bytes} + {actual_len} > {MAX_ARTIFACT_BYTES_PER_RUN} bytes)"
                ),
            });
            continue;
        }
        let mime = match sniff_artifact_mime(&bytes) {
            Ok(m) => m,
            Err(reason) => {
                warnings.push(ArtifactWarning { name: rel_name, reason });
                continue;
            }
        };

        next_num += 1;
        total_artifacts += 1;
        total_bytes = total_bytes.saturating_add(actual_len);
        existing_pairs.insert((rel_name.clone(), sha256.clone()));

        pending.push(PendingArtifactWrite {
            artifact: SolveArtifact {
                artifact_id: format!("art-{next_num}"),
                name: rel_name,
                mime: mime.to_string(),
                blob: sha256.clone(),
                sha256,
                bytes: actual_len,
                call_id: call_id.to_string(),
                entry_id: clean_entry_id.clone(),
            },
            bytes,
        });
    }

    let mut harvested = Vec::with_capacity(pending.len());
    for item in pending {
        let stored_blob = ledger
            .put_blob(&item.bytes)
            .map_err(|e| format!("failed to store artifact blob `{}`: {e}", item.artifact.name))?;
        if stored_blob != item.artifact.sha256 {
            return Err(format!(
                "artifact blob hash mismatch while storing `{}`",
                item.artifact.name
            ));
        }
        let payload = serde_json::to_value(&item.artifact)
            .map_err(|e| format!("failed to serialize solve.artifact: {e}"))?;
        let appended = ledger
            .append_at_head(
                session_id,
                event_kinds::SOLVE_ARTIFACT,
                SOLVE_ARTIFACT_SCHEMA_VERSION,
                &payload,
                &head_hash,
            )
            .map_err(|e| format!("failed to append solve.artifact: {e}"))?;
        head_hash = appended.event_hash;
        harvested.push(HarvestedArtifact {
            seq: appended.seq,
            artifact: item.artifact,
        });
    }

    Ok(HarvestReport {
        harvested,
        warnings,
        total_artifacts_in_run: total_artifacts,
        total_bytes_in_run: total_bytes,
    })
}

/// Validate filenames cleanly and copy attached user data files into `scratch_dir/inputs/` with
/// read-only permissions (where supported), leaving the original user files untouched.
pub fn stage_readonly_inputs(
    scratch_dir: &Path,
    inputs: &[StagedInputSpec],
) -> Result<Vec<StagedInputFile>, SandboxError> {
    let canon_scratch = validate_non_symlink_dir(scratch_dir, "scratch_dir")?;
    if inputs.is_empty() {
        return Ok(Vec::new());
    }

    let inputs_dir = canon_scratch.join(INPUTS_SUBDIR);
    match fs::symlink_metadata(&inputs_dir) {
        Ok(meta) => {
            if is_symlink_or_junction(&inputs_dir, &meta) {
                return Err(SandboxError::EscapesRoot("symlink or junction at scratch/inputs".into()));
            }
            if !meta.is_dir() {
                return Err(SandboxError::NotADirectory(inputs_dir.display().to_string()));
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(&inputs_dir).map_err(|e| SandboxError::Io(e.to_string()))?;
        }
        Err(e) => return Err(SandboxError::Io(e.to_string())),
    }
    let canon_inputs = WorkspaceSandbox::canonicalize_existing_dir(&inputs_dir)?;
    if !WorkspaceSandbox::is_same_or_inside(&canon_scratch, &canon_inputs) {
        return Err(SandboxError::EscapesRoot("scratch/inputs escapes scratch_dir".into()));
    }

    let mut seen_names = BTreeSet::new();
    let mut staged = Vec::with_capacity(inputs.len());
    for spec in inputs {
        let parts = validate_artifact_rel_path(&spec.name)?;
        let norm_name = parts.join("/");
        if !seen_names.insert(norm_name.clone()) {
            return Err(SandboxError::InvalidPath(format!("duplicate staged input `{norm_name}`")));
        }

        let src_meta = fs::symlink_metadata(&spec.source_path).map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                SandboxError::NotFound(spec.source_path.display().to_string())
            } else {
                SandboxError::Io(e.to_string())
            }
        })?;
        if is_symlink_or_junction(&spec.source_path, &src_meta) {
            return Err(SandboxError::EscapesRoot(format!(
                "source input `{}` must not be a symlink or junction",
                spec.source_path.display()
            )));
        }
        if !src_meta.is_file() {
            return Err(SandboxError::InvalidPath(format!(
                "source input `{}` is not a regular file",
                spec.source_path.display()
            )));
        }
        if src_meta.len() > MAX_STAGED_INPUT_BYTES {
            return Err(SandboxError::InvalidPath(format!(
                "source input `{norm_name}` exceeds {MAX_STAGED_INPUT_BYTES} bytes"
            )));
        }
        let bytes = fs::read(&spec.source_path).map_err(|e| SandboxError::Io(e.to_string()))?;
        let sha256 = sha256_hex(&bytes);

        prepare_existing_target_writable(&canon_inputs, &parts)?;
        let target = WorkspaceSandbox::atomic_write_in_root(&canon_inputs, &parts, &bytes)?;
        let mut perms = fs::metadata(&target)
            .map_err(|e| SandboxError::Io(e.to_string()))?
            .permissions();
        perms.set_readonly(true);
        fs::set_permissions(&target, perms).map_err(|e| SandboxError::Io(e.to_string()))?;

        staged.push(StagedInputFile {
            name: norm_name,
            staged_path: target,
            sha256,
            bytes: bytes.len() as u64,
        });
    }
    Ok(staged)
}

#[allow(clippy::permissions_set_readonly_false)]
fn prepare_existing_target_writable(canon_root: &Path, parts: &[String]) -> Result<(), SandboxError> {
    let mut cur = canon_root.to_path_buf();
    for (idx, seg) in parts.iter().enumerate() {
        let next = cur.join(seg);
        let is_last = idx + 1 == parts.len();
        match fs::symlink_metadata(&next) {
            Ok(meta) => {
                if is_symlink_or_junction(&next, &meta) {
                    return Err(SandboxError::EscapesRoot(format!("symlink forbidden at `{seg}`")));
                }
                let canon_next = fs::canonicalize(&next).map_err(|e| SandboxError::EscapesRoot(e.to_string()))?;
                if !WorkspaceSandbox::is_same_or_inside(canon_root, &canon_next) {
                    return Err(SandboxError::EscapesRoot(format!("`{seg}` escapes inputs root")));
                }
                if is_last {
                    if canon_next.is_dir() {
                        return Err(SandboxError::InvalidPath(format!("target `{seg}` is a directory")));
                    }
                    let mut perms = meta.permissions();
                    if perms.readonly() {
                        perms.set_readonly(false);
                        fs::set_permissions(&canon_next, perms).map_err(|e| SandboxError::Io(e.to_string()))?;
                    }
                } else if !canon_next.is_dir() {
                    return Err(SandboxError::NotADirectory(seg.to_string()));
                }
                cur = canon_next;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => break,
            Err(e) => return Err(SandboxError::Io(e.to_string())),
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "solve_artifacts_tests.rs"]
mod tests;
