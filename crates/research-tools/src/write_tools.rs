//! Approval-gated workspace file creation (`fs_write`), exact LaTeX/text
//! patching (`latex_patch`), unified diff preview, and fail-closed backups.

use crate::sandbox::{ResolvedWritePath, SandboxRoot, WorkspaceSandbox};
use crate::ResearchEnv;
use policy::EffectClass;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use similar::TextDiff;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tools::{ApprovalScope, CallCtx, Registry, ToolDef, EVENT_SCHEMA_VERSION};

pub const FS_WRITE: &str = "fs_write";
pub const LATEX_PATCH: &str = "latex_patch";
pub const EDIT_BASE_CHANGED: &str = "changed on disk. Copy your edits, then reload the file to reconcile the changes.";
const MAX_WRITE_CHARS: u64 = 2_000_000;
const MAX_WRITE_BYTES: usize = 4 * 1024 * 1024;
const PREVIEW_SESSION_PLACEHOLDER: &str = "s-preview";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditPreview {
    pub rel_path: String,
    pub unified_diff: String,
    pub sha256_before: Option<String>,
    pub sha256_after: String,
}

struct PreparedEdit {
    canon_root: PathBuf,
    rel_parts: Vec<String>,
    rel_path: String,
    orig_bytes: Option<Vec<u8>>,
    new_bytes: Vec<u8>,
    replacements: usize,
    preview: EditPreview,
}

pub fn fs_write_def() -> ToolDef {
    ToolDef {
        id: FS_WRITE.into(),
        description: "Create or overwrite a UTF-8 text/LaTeX/BibTeX file inside the attached \
                      workspace directory. Requires human approval over the unified diff and \
                      backs up any existing file into the content-addressed ledger blob store."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "maxLength": 512, "description": "Workspace-relative file path." },
                "content": { "type": "string", "maxLength": MAX_WRITE_CHARS, "description": "Complete UTF-8 file content." },
                "expected_sha256": { "type": ["string", "null"], "description": "Optional original file SHA-256. Use null to require a new file." }
            },
            "required": ["path", "content"],
            "additionalProperties": false
        }),
        effect: EffectClass::WriteWorkspace,
        approval_scope: ApprovalScope::Once,
    }
}

pub fn latex_patch_def() -> ToolDef {
    ToolDef {
        id: LATEX_PATCH.into(),
        description: "Apply an exact search-and-replace patch to an existing UTF-8 `.tex`, `.bib`, \
                      or text file in the attached workspace directory. Rejects empty `search`, \
                      missing targets, and ambiguous multi-matches unless `replace_all: true`."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "maxLength": 512, "description": "Workspace-relative file path." },
                "search": { "type": "string", "maxLength": 200000, "description": "Exact existing text block to match." },
                "replace": { "type": "string", "maxLength": 200000, "description": "Replacement text block." },
                "replace_all": { "type": "boolean", "description": "Replace every occurrence when `search` matches multiple times." },
                "expected_sha256": { "type": "string", "description": "Optional original file SHA-256." }
            },
            "required": ["path", "search", "replace"],
            "additionalProperties": false
        }),
        effect: EffectClass::WriteWorkspace,
        approval_scope: ApprovalScope::Once,
    }
}

pub fn register(registry: &mut Registry, env: Arc<ResearchEnv>) {
    let write_env = env.clone();
    registry.register(
        fs_write_def(),
        Arc::new(move |ctx: &CallCtx<'_>, args: &Value| execute_edit(&write_env, ctx, FS_WRITE, args)),
    );
    let patch_env = env.clone();
    registry.register(
        latex_patch_def(),
        Arc::new(move |ctx: &CallCtx<'_>, args: &Value| execute_edit(&patch_env, ctx, LATEX_PATCH, args)),
    );
    crate::latex_compile::register(registry, env);
}

/// Computes a zero-side-effect unified diff and before/after SHA-256 hashes for
/// `fs_write` or `latex_patch` arguments prior to human approval.
pub fn preview_workspace_edit(
    env: &ResearchEnv,
    tool_id: &str,
    canonical_args: &str,
) -> Result<EditPreview, String> {
    let args: Value = serde_json::from_str(canonical_args)
        .map_err(|e| format!("invalid canonical_args JSON: {e}"))?;
    Ok(prepare_edit(env, tool_id, &args)?.preview)
}

fn read_edit_base(env: &ResearchEnv, raw_path: &str) -> Result<(ResolvedWritePath, Option<Vec<u8>>), String> {
    let resolved = env
        .sandbox()
        .resolve_for_write(SandboxRoot::Workspace, PREVIEW_SESSION_PLACEHOLDER, raw_path)
        .map_err(|e| e.to_string())?;

    let orig_bytes = if resolved.exists {
        Some(
            fs::read(&resolved.target_path)
                .map_err(|e| format!("cannot read `{}`: {e}", resolved.rel_path))?,
        )
    } else {
        None
    };

    Ok((resolved, orig_bytes))
}

/// Read the same source bytes used by the write tool, without asking for approval.
pub fn workspace_edit_sha256(env: &ResearchEnv, path: &str) -> Result<Option<String>, String> {
    let (_, original) = read_edit_base(env, path)?;
    Ok(original.as_deref().map(ledger::sha256_hex))
}

fn prepare_edit(env: &ResearchEnv, tool_id: &str, args: &Value) -> Result<PreparedEdit, String> {
    let raw_path = args.get("path").and_then(Value::as_str).ok_or("`path` must be a string")?;
    let (resolved, orig_bytes) = read_edit_base(env, raw_path)?;
    let sha256_before = orig_bytes.as_deref().map(ledger::sha256_hex);
    if let Some(expected) = args.get("expected_sha256") {
        if expected != &serde_json::json!(sha256_before) {
            return Err(format!("{} {EDIT_BASE_CHANGED}", resolved.rel_path));
        }
    }
    let (old_text, new_text, replacements) = match tool_id {
        FS_WRITE => {
            let content = args.get("content").and_then(Value::as_str).ok_or("`content` must be a string")?;
            if content.contains('\0') {
                return Err("`content` must not contain NUL bytes".into());
            }
            if content.len() > MAX_WRITE_BYTES {
                return Err(format!("`content` exceeds {MAX_WRITE_BYTES} byte limit"));
            }
            let old_s = match &orig_bytes {
                Some(b) => String::from_utf8_lossy(b).into_owned(),
                None => String::new(),
            };
            (old_s, content.to_string(), 1usize)
        }
        LATEX_PATCH => {
            let search = args.get("search").and_then(Value::as_str).ok_or("`search` must be a string")?;
            let replace = args.get("replace").and_then(Value::as_str).ok_or("`replace` must be a string")?;
            let replace_all = args.get("replace_all").and_then(Value::as_bool).unwrap_or(false);
            if search.is_empty() {
                return Err("`search` string must not be empty".into());
            }
            if search.contains('\0') || replace.contains('\0') {
                return Err("`search` and `replace` must not contain NUL bytes".into());
            }
            let bytes = orig_bytes
                .as_ref()
                .ok_or_else(|| format!("target file `{}` does not exist", resolved.rel_path))?;
            if bytes.contains(&0) {
                return Err(format!("target file `{}` contains binary NUL bytes", resolved.rel_path));
            }
            let orig_str = std::str::from_utf8(bytes)
                .map_err(|_| format!("target file `{}` is not valid UTF-8", resolved.rel_path))?;
            let (patched, count) = apply_exact_patch(&resolved.rel_path, orig_str, search, replace, replace_all)?;
            if patched.len() > MAX_WRITE_BYTES {
                return Err(format!("patched `{}` exceeds {MAX_WRITE_BYTES} byte limit", resolved.rel_path));
            }
            (orig_str.to_string(), patched, count)
        }
        other => return Err(format!("tool `{other}` does not support workspace edit preview")),
    };

    let new_bytes = new_text.as_bytes().to_vec();
    let sha256_after = ledger::sha256_hex(&new_bytes);
    let unified_diff = build_unified_diff(&resolved.rel_path, sha256_before.is_some(), &old_text, &new_text);

    Ok(PreparedEdit {
        canon_root: resolved.canonical_root,
        rel_parts: resolved.rel_parts,
        rel_path: resolved.rel_path.clone(),
        orig_bytes,
        new_bytes,
        replacements,
        preview: EditPreview {
            rel_path: resolved.rel_path,
            unified_diff,
            sha256_before,
            sha256_after,
        },
    })
}

pub fn apply_exact_patch(
    rel_path: &str,
    orig: &str,
    search: &str,
    replace: &str,
    replace_all: bool,
) -> Result<(String, usize), String> {
    let count = orig.matches(search).count();
    if count == 0 {
        let crlf_hint = if orig.contains("\r\n") && !search.contains("\r\n") && orig.replace("\r\n", "\n").contains(search) {
            " (file uses CRLF `\\r\\n` line endings while `search` used `\\n`)"
        } else {
            ""
        };
        return Err(format!("`search` block not found in `{rel_path}`{crlf_hint}"));
    }
    if count > 1 && !replace_all {
        return Err(format!(
            "ambiguous `search` block in `{rel_path}`: matched {count} occurrences (pass `replace_all: true` to replace all)"
        ));
    }
    let out = if replace_all {
        orig.replace(search, replace)
    } else {
        orig.replacen(search, replace, 1)
    };
    Ok((out, count))
}

fn build_unified_diff(rel_path: &str, existed: bool, old_text: &str, new_text: &str) -> String {
    let diff = TextDiff::from_lines(old_text, new_text);
    let old_hdr = if existed { format!("a/{rel_path}") } else { "/dev/null".to_string() };
    let new_hdr = format!("b/{rel_path}");
    let mut udiff = diff.unified_diff();
    udiff.context_radius(3).header(&old_hdr, &new_hdr);
    udiff.to_string()
}

fn execute_edit(env: &ResearchEnv, ctx: &CallCtx<'_>, tool_id: &str, args: &Value) -> Result<Value, String> {
    let prepared = prepare_edit(env, tool_id, args)?;

    // 1. FAIL-CLOSED PRE-OVERWRITE BACKUP: persist blob + "workspace.backup" BEFORE mutating disk.
    let mut backup_blob: Option<String> = None;
    let mut backup_seq: Option<u64> = None;
    if let Some(orig) = &prepared.orig_bytes {
        let guard = ctx.ledger.lock().map_err(|_| "ledger lock poisoned".to_string())?;
        let blob_hash = guard
            .put_blob(orig)
            .map_err(|e| format!("workspace backup put_blob failed (write aborted): {e}"))?;
        let seq = guard
            .append(
                ctx.session,
                "workspace.backup",
                EVENT_SCHEMA_VERSION,
                &json!({
                    "call_id": ctx.call_id,
                    "tool_id": tool_id,
                    "rel_path": prepared.rel_path,
                    "bytes": orig.len(),
                    "raw_blob": blob_hash,
                    "sha256_after": prepared.preview.sha256_after,
                }),
            )
            .map_err(|e| format!("workspace backup append failed (write aborted): {e}"))?;
        backup_blob = Some(blob_hash);
        backup_seq = Some(seq);
    }

    // 2. Atomic same-directory temp file + sync_all + TOCTOU recheck + rename.
    WorkspaceSandbox::atomic_write_in_root(&prepared.canon_root, &prepared.rel_parts, &prepared.new_bytes)
        .map_err(|e| e.to_string())?;

    let mut out = json!({
        "ok": true,
        "path": prepared.rel_path,
        "created": prepared.orig_bytes.is_none(),
        "bytes_written": prepared.new_bytes.len(),
        "sha256_before": prepared.preview.sha256_before,
        "sha256_after": prepared.preview.sha256_after,
        "backup_blob": backup_blob,
        "backup_seq": backup_seq,
    });
    if tool_id == LATEX_PATCH {
        out["replacements"] = json!(prepared.replacements);
        out["unified_diff"] = json!(prepared.preview.unified_diff);
    }
    Ok(out)
}
