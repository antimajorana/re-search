//! Verified session forking & counterfactual assumption branching (F14).

use crate::{ForkOptions, ForkReceipt};
use ledger::{Event, Ledger};
use research_tools::sandbox::sanitize_rel_components;
use research_tools::{validate_session_id, SandboxRoot, WorkspaceSandbox};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const MAX_MODIFIED_ASSUMPTION_CHARS: usize = 4_000;
const EVENT_SCHEMA_VERSION: u32 = 1;

fn generate_session_id() -> Result<String, String> {
    let mut buf = [0u8; 8];
    getrandom::getrandom(&mut buf).map_err(|e| format!("failed to generate session id: {e}"))?;
    let hex: String = buf.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!("s-{hex}"))
}

/// Start a fresh Research history with the selected paper evidence and an immutable return link.
pub fn research_from_paper(data_dir: &Path, parent: &str, context: &Value) -> Result<String, String> {
    validate_session_id(parent).map_err(|e| e.to_string())?;
    let ledger = Ledger::open_existing(data_dir).map_err(|e| e.to_string())?;
    let (_, events) = ledger.read_verified_session(parent).map_err(|e| e.to_string())?;
    let (primary_source_id, default_through) = if context["map_seq"].is_u64() {
        let map = events
            .iter()
            .find(|e| {
                e.kind == "investigation.map"
                    && Some(e.seq) == context["map_seq"].as_u64()
                    && Some(e.event_hash.as_str()) == context["map_hash"].as_str()
            })
            .ok_or("Paper map revision missing.")?;
        let src = map.payload["source_id"]
            .as_str()
            .ok_or("Paper map has no primary source.")?;
        (src, map.seq)
    } else if context["answer_seq"].is_u64() {
        let ans = events
            .iter()
            .find(|e| {
                e.kind == "investigation.answer"
                    && Some(e.seq) == context["answer_seq"].as_u64()
                    && Some(e.event_hash.as_str()) == context["answer_hash"].as_str()
            })
            .ok_or("Investigation answer revision missing.")?;
        let src = context["source_id"]
            .as_str()
            .or_else(|| context["primary_source_id"].as_str())
            .or_else(|| {
                events
                    .iter()
                    .rev()
                    .find(|e| e.kind == "investigation.paper")
                    .and_then(|e| e.payload["source_id"].as_str())
            })
            .ok_or("Investigation answer has no primary source.")?;
        (src, ans.seq)
    } else {
        return Err("Paper map revision missing.".into());
    };
    let mut source_ids = std::collections::BTreeSet::new();
    source_ids.insert(primary_source_id);
    for anchor in context["sources"].as_array().into_iter().flatten() {
        source_ids.insert(anchor["source_id"].as_str().ok_or("Source identifier missing from evidence.")?);
    }
    let through = context["evidence_through_seq"].as_u64().unwrap_or(default_through);
    let sources: Vec<_> = events.iter().filter(|e| e.kind == "source.ingested" && e.seq <= through
        && e.payload["source_id"].as_str().is_some_and(|id| source_ids.contains(id))).cloned().collect();
    if source_ids.iter().any(|id| !sources.iter().any(|e| e.payload["source_id"].as_str() == Some(*id))) {
        return Err("A source required by this claim is missing from the Investigation.".into());
    }
    let child = generate_session_id()?;
    let mut files = copy_session_sources(data_dir, parent, &child, &sources)?;
    let mut batch: Vec<_> = sources.iter().map(|ev| (ev.kind.as_str(), ev.schema_version, &ev.payload)).collect();
    batch.push(("investigation.handoff", EVENT_SCHEMA_VERSION, context));
    ledger.append_new_session(&child, &batch).map_err(|e| e.to_string())?;
    if let Some(files) = &mut files { files.committed = true; }
    Ok(child)
}

pub fn fork_session(data_dir: &Path, opts: &ForkOptions) -> Result<ForkReceipt, String> {
    validate_session_id(&opts.parent_session).map_err(|e| e.to_string())?;
    if let Some(mod_a) = &opts.modified_assumption {
        let len = mod_a.chars().count();
        if len > MAX_MODIFIED_ASSUMPTION_CHARS {
            return Err(format!(
                "modified_assumption exceeds {MAX_MODIFIED_ASSUMPTION_CHARS} characters (got {len})"
            ));
        }
        if mod_a.trim().is_empty() {
            return Err("modified_assumption must not be blank when provided".into());
        }
    }

    let ledger = Ledger::open_existing(data_dir).map_err(|e| e.to_string())?;
    let (_, events) = ledger
        .read_verified_session(&opts.parent_session)
        .map_err(|e| format!("parent session `{}` failed verification: {e}", opts.parent_session))?;
    let prefix_len = usize::try_from(opts.at_seq)
        .ok()
        .filter(|&len| len > 0 && len <= events.len())
        .ok_or_else(|| format!(
            "fork sequence `at_seq` ({}) out of bounds for session `{}` (1..={})",
            opts.at_seq,
            opts.parent_session,
            events.len()
        ))?;

    let prefix = &events[..prefix_len];
    let anchor = prefix
        .last()
        .ok_or_else(|| "fork prefix must contain at least one event".to_string())?;
    if anchor.seq != opts.at_seq {
        return Err(format!(
            "parent session `{}` seq gap at {}",
            opts.parent_session, opts.at_seq
        ));
    }

    let existing_sessions: Vec<String> = ledger
        .sessions()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|(s, _)| s)
        .collect();
    let mut new_session = generate_session_id()?;
    while existing_sessions.contains(&new_session)
        || data_dir.join("sessions").join(&new_session).exists()
    {
        new_session = generate_session_id()?;
    }

    let mut fork_payload = json!({
        "parent_session": opts.parent_session,
        "parent_seq": opts.at_seq,
        "parent_event_hash": anchor.event_hash,
    });
    if let (Some(mod_a), Some(map)) = (&opts.modified_assumption, fork_payload.as_object_mut()) {
        map.insert("modified_assumption".into(), json!(mod_a));
    }
    let assumption = if let Some(mod_a) = &opts.modified_assumption {
        let entry_id = next_entry_id(prefix)?;
        let summary: String = mod_a.chars().take(120).collect();
        Some(json!({
            "entry_id": entry_id,
            "entry_type": "assumption",
            "summary": format!("Fork counterfactual assumption: {summary}"),
            "details": mod_a,
            "latex": Value::Null,
            "depends_on": [],
        }))
    } else {
        None
    };
    let mut batch: Vec<_> = prefix
        .iter()
        .map(|ev| (ev.kind.as_str(), ev.schema_version, &ev.payload))
        .collect();
    batch.push(("session.fork", EVENT_SCHEMA_VERSION, &fork_payload));
    if let Some(assumption) = &assumption {
        batch.push(("proof.entry", EVENT_SCHEMA_VERSION, assumption));
    }

    let mut staged = copy_session_sources(data_dir, &opts.parent_session, &new_session, prefix)?;
    ledger
        .append_new_session(&new_session, &batch)
        .map_err(|e| format!("failed to record fork `{new_session}`: {e}"))?;
    if let Some(staged) = &mut staged {
        staged.committed = true;
    }

    Ok(ForkReceipt {
        new_session,
        parent_session: opts.parent_session.clone(),
        parent_seq: opts.at_seq,
        parent_event_hash: anchor.event_hash.clone(),
        copied_events: prefix.len(),
    })
}

fn next_entry_id(events: &[Event]) -> Result<String, String> {
    let mut max_n = 0u64;
    for ev in events {
        if ev.kind == "proof.entry" {
            if let Some(id) = ev.payload.get("entry_id").and_then(Value::as_str) {
                if let Some(num) = id.strip_prefix("e-").and_then(|s| s.parse::<u64>().ok()) {
                    max_n = max_n.max(num);
                }
            }
        }
    }
    max_n
        .checked_add(1)
        .map(|next| format!("e-{next}"))
        .ok_or_else(|| "no available proof entry ID for the counterfactual assumption".to_string())
}

struct StagedSources {
    child_dir: PathBuf,
    committed: bool,
}

impl Drop for StagedSources {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(&self.child_dir);
        }
    }
}

fn copy_session_sources(
    data_dir: &Path,
    parent: &str,
    child: &str,
    prefix: &[Event],
) -> Result<Option<StagedSources>, String> {
    let mut files = BTreeMap::new();
    for ev in prefix.iter().filter(|ev| ev.kind == "source.ingested") {
        let payload = &ev.payload;
        let paths = payload["files"]
            .as_array()
            .into_iter()
            .flatten()
            .chain([&payload["primary_path"], &payload["assembled_path"]]);
        for path in paths.filter_map(Value::as_str) {
            let (parts, normalized) = sanitize_rel_components(path, false)
                .map_err(|e| format!("invalid fork source path at event {}: {e}", ev.seq))?;
            if parts.len() < 2 || payload["source_id"].as_str() != parts.first().map(String::as_str) {
                return Err(format!("fork source path `{path}` does not belong to its recorded source"));
            }
            let hash = payload
                .get("files_sha256")
                .map(|hashes| {
                    hashes[path]
                        .as_str()
                        .ok_or_else(|| format!("fork source `{path}` is missing its recorded hash"))
                })
                .transpose()?;
            files.insert(normalized, hash);
        }
    }
    if files.is_empty() {
        return Ok(None);
    }

    let sandbox = WorkspaceSandbox::new(data_dir, None);
    let parent_sources = sandbox
        .canonical_root(SandboxRoot::Sources, parent, false)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("recorded sources for session `{parent}` are missing"))?;
    let sessions_dir = parent_sources
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| format!("cannot locate sessions directory from `{}`", parent_sources.display()))?;
    let child_dir = sessions_dir.join(child);
    fs::create_dir(&child_dir).map_err(|e| format!("failed to reserve fork `{child}`: {e}"))?;
    let staged = StagedSources { child_dir, committed: false };
    let destination = staged.child_dir.join("sources");
    fs::create_dir(&destination).map_err(|e| format!("failed to stage sources for fork `{child}`: {e}"))?;
    let destination = fs::canonicalize(destination).map_err(|e| e.to_string())?;
    for (path, expected_hash) in files {
        // Resolve without following symlinks or junctions within the source tree.
        let source = sandbox
            .resolve_for_write(SandboxRoot::Sources, parent, &path)
            .map_err(|e| format!("failed to resolve fork source `{path}`: {e}"))?;
        if !source.exists || !source.target_path.is_file() {
            return Err(format!("recorded fork source `{path}` is missing or is not a regular file"));
        }
        let bytes = fs::read(&source.target_path).map_err(|e| format!("failed to read fork source `{path}`: {e}"))?;
        if expected_hash.is_some_and(|expected| ledger::sha256_hex(&bytes) != expected) {
            return Err(format!("fork source `{path}` does not match its recorded hash"));
        }
        WorkspaceSandbox::atomic_write_in_root(&destination, &source.rel_parts, &bytes)
            .map_err(|e| format!("failed to copy fork source `{path}`: {e}"))?;
    }
    Ok(Some(staged))
}
