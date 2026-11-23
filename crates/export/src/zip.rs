//! Reproducible ZIP bundle export (F15): `report.tex`, `references.bib`,
//! `report.html`, `ledger.json`, `code/step_<seq>.py`, `sources/*`,
//! `run_all_checks.py`, and `REPRODUCE.md`.

use crate::html::render_html_document;
use crate::latex::{render_latex_document, SessionExportData};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

fn fixed_zip_options() -> SimpleFileOptions {
    SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(DateTime::default())
        .unix_permissions(0o644)
}

pub(crate) fn build_reproducible_zip(data_dir: &Path, data: &SessionExportData) -> Result<Vec<u8>, String> {
    let mut entries: BTreeMap<String, Vec<u8>> = BTreeMap::new();

    let tex = render_latex_document(data, "references");
    let html = render_html_document(data);
    entries.insert("report.tex".into(), tex.into_bytes());
    entries.insert("references.bib".into(), data.bibtex.as_bytes().to_vec());
    entries.insert("report.html".into(), html.into_bytes());

    let ledger_doc = json!({
        "session": data.session,
        "head_hash": data.head_hash,
        "model": data.model,
        "policy_version": data.policy_version,
        "events": data.events.iter().map(|e| json!({
            "seq": e.seq,
            "ts_ms": e.ts_ms,
            "kind": e.kind,
            "schema_version": e.schema_version,
            "payload": e.payload,
            "prev_hash": e.prev_hash,
            "event_hash": e.event_hash,
        })).collect::<Vec<Value>>(),
    });
    let mut ledger_bytes = serde_json::to_vec_pretty(&ledger_doc).map_err(|e| e.to_string())?;
    ledger_bytes.push(b'\n');
    entries.insert("ledger.json".into(), ledger_bytes);

    let mut worker_indexes = BTreeMap::new();
    let mut workers: Vec<Vec<(u64, String)>> = Vec::new();
    for call in &data.sympy_calls {
        let rel = format!("code/step_{}.py", call.seq);
        let mut content = format!(
            "# re:Search step {} (call_id: {})\n",
            call.seq, call.call_id
        );
        content.push_str(&call.code);
        if !content.ends_with('\n') {
            content.push('\n');
        }
        entries.insert(rel.clone(), content.into_bytes());
        let index = *worker_indexes.entry(&call.worker).or_insert_with(|| {
            workers.push(Vec::new());
            workers.len() - 1
        });
        workers[index].push((call.execution_order, rel));
    }

    let sources_dir = data_dir.join("sessions").join(&data.session).join("sources");
    if sources_dir.is_dir() {
        collect_sources_into_map(&sources_dir, &sources_dir, &mut entries)?;
    }

    let replay_workers: Vec<Vec<String>> = workers
        .into_iter()
        .map(|mut steps| {
            steps.sort_by_key(|(order, _)| *order);
            steps.into_iter().map(|(_, path)| path).collect()
        })
        .collect();
    entries.insert("run_all_checks.py".into(), include_bytes!("replay_checks.py").to_vec());
    entries.insert(
        "_re_search_runtime/replay_steps.json".into(),
        serde_json::to_vec_pretty(&replay_workers).map_err(|e| e.to_string())?,
    );
    entries.insert(
        "_re_search_runtime/worker.py".into(),
        include_bytes!("../../../python/worker.py").to_vec(),
    );
    entries.insert(
        "_re_search_runtime/solver_calc.py".into(),
        include_bytes!("../../../python/solver_calc.py").to_vec(),
    );
    entries.insert("REPRODUCE.md".into(), build_reproduce_md(data).into_bytes());

    let buf = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(buf);
    let opts = fixed_zip_options();
    for (name, bytes) in entries {
        writer
            .start_file(&name, opts)
            .map_err(|e| format!("zip start_file `{name}` failed: {e}"))?;
        writer
            .write_all(&bytes)
            .map_err(|e| format!("zip write `{name}` failed: {e}"))?;
    }
    let cursor = writer.finish().map_err(|e| format!("zip finish failed: {e}"))?;
    Ok(cursor.into_inner())
}

fn collect_sources_into_map(
    base: &Path,
    cur: &Path,
    out: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), String> {
    let rd = fs::read_dir(cur).map_err(|e| e.to_string())?;
    let mut items: Vec<_> = rd.filter_map(Result::ok).collect();
    items.sort_by_key(|e| e.file_name());
    for entry in items {
        let path = entry.path();
        let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if research_tools::is_symlink_or_junction(&path, &meta) {
            continue;
        }
        if meta.is_dir() {
            collect_sources_into_map(base, &path, out)?;
        } else if meta.is_file() {
            let rel = path
                .strip_prefix(base)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = fs::read(&path).map_err(|e| e.to_string())?;
            out.insert(format!("sources/{rel}"), bytes);
        }
    }
    Ok(())
}

fn build_reproduce_md(data: &SessionExportData) -> String {
    format!(
        "# Reproducibility Bundle: `{}`\n\n\
         - **Chain Head SHA-256**: `{}`\n\
         - **Model**: `{}`\n\
         - **Policy Version**: `{}`\n\
         - **Extracted Checks**: {}\n\n\
         ## Verification Steps\n\n\
         1. Replay successful computations from a trusted bundle with Python and SymPy installed. \
         The runner executes the exported code locally and shares a Python namespace within each recorded worker lifetime. \
         Older ledgers group checks by session start or resume because they lack worker restart records. \
         Later steps that relied on side effects of a failed calculation may need to be rewritten.\n   ```bash\n   python3 run_all_checks.py\n   ```\n\
         2. Inspect the hash-chained session audit log in `ledger.json`.\n\
         3. Compile `report.tex` (with `references.bib`) or open `report.html`.\n",
        data.session,
        data.head_hash,
        data.model,
        data.policy_version,
        data.sympy_calls.len()
    )
}
