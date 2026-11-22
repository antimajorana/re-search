//! Verified session forking & counterfactual assumption branching (F14).

use crate::{ForkOptions, ForkReceipt};
use ledger::{Event, Ledger};
use research_tools::{validate_session_id, SandboxRoot, WorkspaceSandbox};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

pub const MAX_MODIFIED_ASSUMPTION_CHARS: usize = 4_000;
const EVENT_SCHEMA_VERSION: u32 = 1;

fn generate_session_id() -> Result<String, String> {
    let mut buf = [0u8; 8];
    getrandom::getrandom(&mut buf).map_err(|e| format!("failed to generate session id: {e}"))?;
    let hex: String = buf.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!("s-{hex}"))
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
    ledger
        .verify_session(&opts.parent_session)
        .map_err(|e| format!("parent session `{}` failed verification: {e}", opts.parent_session))?;
    let events = ledger
        .read_session(&opts.parent_session)
        .map_err(|e| format!("failed to read parent session `{}`: {e}", opts.parent_session))?;
    if opts.at_seq == 0 || (opts.at_seq as usize) > events.len() {
        return Err(format!(
            "fork sequence `at_seq` ({}) out of bounds for session `{}` (1..={})",
            opts.at_seq,
            opts.parent_session,
            events.len()
        ));
    }

    let prefix: &[Event] = &events[..opts.at_seq as usize];
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

    for ev in prefix {
        ledger
            .append_with_hash(&new_session, &ev.kind, ev.schema_version, &ev.payload)
            .map_err(|e| format!("failed copying event seq {} into `{new_session}`: {e}", ev.seq))?;
    }

    copy_session_sources(data_dir, &opts.parent_session, &new_session)?;

    let mut fork_payload = json!({
        "parent_session": opts.parent_session,
        "parent_seq": opts.at_seq,
        "parent_event_hash": anchor.event_hash,
    });
    if let (Some(mod_a), Some(map)) = (&opts.modified_assumption, fork_payload.as_object_mut()) {
        map.insert("modified_assumption".into(), json!(mod_a));
    }
    ledger
        .append_with_hash(&new_session, "session.fork", EVENT_SCHEMA_VERSION, &fork_payload)
        .map_err(|e| format!("failed to append session.fork: {e}"))?;

    if let Some(mod_a) = &opts.modified_assumption {
        let entry_id = next_entry_id(prefix);
        let summary: String = mod_a.chars().take(120).collect();
        ledger
            .append_with_hash(
                &new_session,
                "proof.entry",
                EVENT_SCHEMA_VERSION,
                &json!({
                    "entry_id": entry_id,
                    "entry_type": "assumption",
                    "summary": format!("Fork counterfactual assumption: {summary}"),
                    "details": mod_a,
                    "latex": Value::Null,
                    "depends_on": [],
                }),
            )
            .map_err(|e| format!("failed to append counterfactual assumption proof.entry: {e}"))?;
    }

    Ok(ForkReceipt {
        new_session,
        parent_session: opts.parent_session.clone(),
        parent_seq: opts.at_seq,
        parent_event_hash: anchor.event_hash.clone(),
        copied_events: prefix.len(),
    })
}

fn next_entry_id(events: &[Event]) -> String {
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
    format!("e-{}", max_n + 1)
}

fn copy_session_sources(data_dir: &Path, parent: &str, child: &str) -> Result<(), String> {
    let sandbox = WorkspaceSandbox::new(data_dir, None);
    let Ok(Some(src_root)) = sandbox.canonical_root(SandboxRoot::Sources, parent, false) else {
        return Ok(());
    };
    let Some(dst_root) = sandbox
        .canonical_root(SandboxRoot::Sources, child, true)
        .map_err(|e| e.to_string())?
    else {
        return Ok(());
    };
    copy_dir_no_symlinks(&src_root, &src_root, &dst_root)
}

fn copy_dir_no_symlinks(base_src: &Path, cur_src: &Path, dst_root: &Path) -> Result<(), String> {
    let rd = fs::read_dir(cur_src)
        .map_err(|e| format!("failed to read source dir `{}`: {e}", cur_src.display()))?;
    for entry in rd {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if research_tools::is_symlink_or_junction(&path, &meta) {
            continue;
        }
        let canon = fs::canonicalize(&path).map_err(|e| e.to_string())?;
        if !WorkspaceSandbox::is_same_or_inside(base_src, &canon) {
            return Err("source symlink escape detected during fork".into());
        }
        let rel = path
            .strip_prefix(base_src)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        if meta.is_dir() {
            copy_dir_no_symlinks(base_src, &canon, dst_root)?;
        } else if meta.is_file() {
            let bytes = fs::read(&canon).map_err(|e| e.to_string())?;
            let (parts, _) = research_tools::sandbox::sanitize_rel_components(&rel, false)
                .map_err(|e| e.to_string())?;
            WorkspaceSandbox::atomic_write_in_root(dst_root, &parts, &bytes)
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
