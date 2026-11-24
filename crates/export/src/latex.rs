//! Deterministic publication LaTeX (`.tex`) and BibTeX (`.bib`) export (F15).

use ledger::{Event, Ledger};
use research_tools::{sandbox::sanitize_rel_components, SandboxRoot, WorkspaceSandbox};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

pub(crate) struct ProofItem {
    pub entry_id: String,
    pub seq: u64,
    pub entry_type: String,
    pub status: String,
    pub superseded_by: Option<String>,
    pub summary: String,
    pub details: String,
    pub latex: Option<String>,
    pub depends_on: Vec<String>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ReplayWorker {
    LegacyLeg(u64),
    Recorded { instance: String, epoch: u64 },
}

pub(crate) struct SympyCall {
    pub seq: u64,
    pub result_seq: u64,
    pub call_id: String,
    pub code: String,
    pub output: String,
    pub result: Value,
    pub worker: ReplayWorker,
    pub worker_environment: Value,
    pub execution_order: u64,
}

pub(crate) struct SessionExportData {
    pub session: String,
    pub head_hash: String,
    pub prompt: String,
    pub model: String,
    pub policy_version: String,
    pub entries: Vec<ProofItem>,
    pub sympy_calls: Vec<SympyCall>,
    pub workspace_files: Vec<Value>,
    pub bibtex: String,
    pub events: Vec<Event>,
}

#[derive(Serialize)]
pub(crate) struct SourceRevision<'a> {
    event_seq: u64,
    source_id: &'a str,
    original_sha256: Option<&'a str>,
    pub recorded_sha256: Option<&'a str>,
}

pub(crate) fn recorded_sources(events: &[Event]) -> Result<BTreeMap<String, Vec<SourceRevision<'_>>>, String> {
    let mut by_path: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for event in events.iter().filter(|e| e.kind == "source.ingested") {
        let payload = &event.payload;
        let paths = payload["files"]
            .as_array()
            .into_iter()
            .flatten()
            .chain([&payload["primary_path"], &payload["assembled_path"]]);
        let mut seen = BTreeSet::new();
        for path in paths.filter_map(Value::as_str) {
            let (parts, normalized) = sanitize_rel_components(path, false)
                .map_err(|e| format!("invalid source path at ledger event {}: {e}", event.seq))?;
            let source_id = payload["source_id"].as_str().unwrap_or_default();
            if parts.len() < 2 || parts[0] != source_id {
                return Err(format!("source path `{path}` does not belong to its recorded source"));
            }
            if seen.insert(normalized.clone()) {
                by_path.entry(normalized).or_default().push(SourceRevision {
                    event_seq: event.seq,
                    source_id,
                    original_sha256: payload["raw_blob"].as_str(),
                    recorded_sha256: payload["files_sha256"][path].as_str(),
                });
            }
        }
    }
    Ok(by_path)
}

pub(crate) fn load_session_export_data(data_dir: &Path, session: &str) -> Result<SessionExportData, String> {
    let ledger = Ledger::open_existing(data_dir).map_err(|e| e.to_string())?;
    let (report, events) = ledger
        .read_verified_session(session)
        .map_err(|e| format!("cannot export unverified session `{session}`: {e}"))?;
    let start = events
        .iter()
        .find(|e| e.kind == "session.start")
        .map(|e| &e.payload);
    let prompt = start
        .and_then(|p| p.get("prompt"))
        .and_then(Value::as_str)
        .unwrap_or("(no prompt recorded)")
        .to_string();
    let model = start
        .and_then(|p| p.get("model"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let policy_version = start
        .and_then(|p| p.get("policy_version"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();

    let entries = collect_proof_items(&events);
    let sympy_calls = collect_sympy_calls(&ledger, &events);
    let workspace_files = collect_workspace_files(&ledger, &events);
    let bibtex = collect_bibtex(data_dir, &ledger, session, &events)?;

    Ok(SessionExportData {
        session: session.to_string(),
        head_hash: report.head_hash,
        prompt,
        model,
        policy_version,
        entries,
        sympy_calls,
        workspace_files,
        bibtex,
        events,
    })
}

pub(crate) fn collect_proof_items(events: &[Event]) -> Vec<ProofItem> {
    let mut map: BTreeMap<String, ProofItem> = BTreeMap::new();
    let mut order = Vec::new();
    for ev in events {
        match ev.kind.as_str() {
            "proof.entry" => {
                let Some(id) = ev.payload.get("entry_id").and_then(Value::as_str) else {
                    continue;
                };
                if !map.contains_key(id) {
                    order.push(id.to_string());
                }
                let deps = ev
                    .payload
                    .get("depends_on")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                    .unwrap_or_default();
                map.insert(
                    id.to_string(),
                    ProofItem {
                        entry_id: id.to_string(),
                        seq: ev.seq,
                        entry_type: ev
                            .payload
                            .get("entry_type")
                            .and_then(Value::as_str)
                            .unwrap_or("derivation")
                            .to_string(),
                        status: "pending".to_string(),
                        superseded_by: None,
                        summary: ev
                            .payload
                            .get("summary")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        details: ev
                            .payload
                            .get("details")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        latex: ev.payload.get("latex").and_then(Value::as_str).map(str::to_string),
                        depends_on: deps,
                    },
                );
            }
            "proof.supersede" => {
                if let Some(id) = ev.payload.get("entry_id").and_then(Value::as_str) {
                    if let Some(item) = map.get_mut(id) {
                        item.status = "superseded".into();
                        item.superseded_by = ev.payload["replaced_by"].as_str().map(str::to_string);
                    }
                }
            }
            "proof.verification" => {
                if let Some(id) = ev.payload.get("target_entry_id").and_then(Value::as_str) {
                    if let Some(item) = map.get_mut(id) {
                        if item.status != "superseded" {
                            item.status = ev
                                .payload
                                .get("verdict")
                                .and_then(Value::as_str)
                                .unwrap_or("pending")
                                .to_string();
                        }
                    }
                }
            }
            _ => {}
        }
    }
    order.into_iter().filter_map(|id| map.remove(&id)).collect()
}

fn resolve_slot(ledger: &Ledger, slot: Option<&Value>) -> String {
    let Some(obj) = slot.and_then(Value::as_object) else {
        return String::new();
    };
    if let Some(s) = obj.get("inline").and_then(Value::as_str) {
        return s.to_string();
    }
    if let Some(h) = obj.get("blob").and_then(Value::as_str) {
        if let Ok(bytes) = ledger.get_blob(h) {
            return String::from_utf8_lossy(&bytes).into_owned();
        }
    }
    String::new()
}

fn collect_sympy_calls(ledger: &Ledger, events: &[Event]) -> Vec<SympyCall> {
    let mut pending = BTreeMap::new();
    let mut leg_seq = 0;
    let mut calls = Vec::new();
    for ev in events {
        if matches!(ev.kind.as_str(), "session.start" | "session.resume") {
            leg_seq = ev.seq;
        }
        let Some(call_id) = ev.payload.get("call_id").and_then(Value::as_str) else {
            continue;
        };
        match ev.kind.as_str() {
            "tool.request" if ev.payload.get("tool").and_then(Value::as_str) == Some("sympy_eval") => {
                pending.insert(call_id, (ev, leg_seq));
            }
            "tool.denied" => {
                pending.remove(call_id);
            }
            "tool.result" => {
                let Some((request, request_leg)) = pending.remove(call_id) else {
                    continue;
                };
                if ev.payload.get("ok").and_then(Value::as_bool) != Some(true) {
                    continue;
                }
                let output = resolve_slot(ledger, ev.payload.get("result"));
                let Ok(result) = serde_json::from_str::<Value>(&output) else {
                    continue;
                };
                // The worker reports Python exceptions inside a successful RPC result.
                if !result.is_object() || !result["error"].is_null() {
                    continue;
                }
                let args_raw = resolve_slot(ledger, request.payload.get("args"));
                let Ok(args) = serde_json::from_str::<Value>(&args_raw) else {
                    continue;
                };
                let Some(code) = args["code"].as_str().filter(|code| !code.trim().is_empty()) else {
                    continue;
                };
                let execution = &ev.payload["worker_execution"];
                let (worker, execution_order) = match (
                    execution["instance"].as_str(),
                    execution["epoch"].as_u64(),
                    execution["request_id"].as_u64(),
                ) {
                    (Some(instance), Some(epoch), Some(request_id)) => {
                        (ReplayWorker::Recorded { instance: instance.into(), epoch }, request_id)
                    }
                    _ => (ReplayWorker::LegacyLeg(request_leg), ev.seq),
                };
                calls.push(SympyCall {
                    seq: request.seq,
                    result_seq: ev.seq,
                    call_id: call_id.into(),
                    code: code.into(),
                    output,
                    result,
                    worker,
                    worker_environment: execution["environment"].clone(),
                    execution_order,
                });
            }
            _ => {}
        }
    }
    calls.sort_by_key(|call| call.seq);
    calls
}

fn collect_workspace_files(ledger: &Ledger, events: &[Event]) -> Vec<Value> {
    let mut pending = BTreeMap::new();
    let mut reads = Vec::new();
    for event in events {
        let Some(id) = event.payload["call_id"].as_str() else { continue };
        match event.kind.as_str() {
            "tool.request" if event.payload["tool"] == "fs_read" => {
                pending.insert(id, event.seq);
            }
            "tool.denied" => {
                pending.remove(id);
            }
            "tool.result" => {
                let Some(request_seq) = pending.remove(id) else { continue };
                if event.payload["ok"] != true {
                    continue;
                }
                let output = resolve_slot(ledger, event.payload.get("result"));
                let Ok(result) = serde_json::from_str::<Value>(&output) else { continue };
                if result["root"] != "workspace" {
                    continue;
                }
                let Some(path) = result["path"].as_str() else { continue };
                let Ok((_, path)) = sanitize_rel_components(path, false) else { continue };
                let Some(hash) = result["file_sha256"].as_str().filter(|hash| {
                    hash.len() == 64 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
                }) else { continue };
                let coverage = if result["content"].is_string() {
                    if result["offset"].as_u64() == Some(0)
                        && result["truncated"].as_bool() == Some(false)
                        && result["end_offset"].as_u64().is_some()
                        && result["end_offset"] == result["total_chars"]
                    {
                        "full_text"
                    } else {
                        "excerpt"
                    }
                } else {
                    "search_results"
                };
                reads.push(json!({
                    "path": path,
                    "sha256": hash,
                    "request_seq": request_seq,
                    "result_seq": event.seq,
                    "result_coverage": coverage,
                }));
            }
            _ => {}
        }
    }
    reads
}

fn collect_bibtex(data_dir: &Path, ledger: &Ledger, session: &str, events: &[Event]) -> Result<String, String> {
    let mut chunks = Vec::new();
    let mut seen = BTreeSet::new();
    let mut push_bib = |raw: &str| {
        let t = raw.trim();
        if !t.is_empty() && seen.insert(t.to_string()) {
            chunks.push(t.to_string());
        }
    };

    for ev in events {
        if let Some(h) = ev.payload.get("raw_blob").and_then(Value::as_str) {
            if let Ok(bytes) = ledger.get_blob(h) {
                if let Ok(val) = serde_json::from_slice::<Value>(&bytes) {
                    extract_bibtex_from_json(&val, &mut push_bib);
                }
            }
        }
        if ev.kind == "tool.result" {
            let txt = resolve_slot(ledger, ev.payload.get("result"));
            if let Ok(val) = serde_json::from_str::<Value>(&txt) {
                extract_bibtex_from_json(&val, &mut push_bib);
            }
        }
    }

    let sandbox = WorkspaceSandbox::new(data_dir, None);
    let recorded = recorded_sources(events)?;
    if recorded.is_empty() {
        if let Ok(Some(sources_dir)) = sandbox.canonical_root(SandboxRoot::Sources, session, false) {
            collect_bib_files(&sources_dir, &mut push_bib);
        }
    } else {
        for (path, revisions) in recorded {
            if !is_bib_file(Path::new(&path)) {
                continue;
            }
            let Some(bytes) = read_source_file(&sandbox, session, &path) else { continue };
            if revisions.last().and_then(|rev| rev.recorded_sha256)
                .is_some_and(|hash| hash != ledger::sha256_hex(&bytes))
            {
                continue;
            }
            if let Ok(text) = std::str::from_utf8(&bytes) {
                push_bib(text);
            }
        }
    }
    if chunks.is_empty() {
        Ok(String::new())
    } else {
        let mut out = chunks.join("\n\n");
        out.push('\n');
        Ok(out)
    }
}

pub(crate) fn read_source_file(sandbox: &WorkspaceSandbox, session: &str, path: &str) -> Option<Vec<u8>> {
    let source = sandbox.resolve_existing(SandboxRoot::Sources, session, path, false).ok()?;
    if !source.metadata.is_file() {
        return None;
    }
    fs::read(source.canonical_target).ok()
}

fn extract_bibtex_from_json(val: &Value, push: &mut impl FnMut(&str)) {
    match val {
        Value::Object(map) => {
            if let Some(b) = map.get("bibtex").and_then(Value::as_str) {
                push(b);
            }
            for v in map.values() {
                extract_bibtex_from_json(v, push);
            }
        }
        Value::Array(arr) => {
            for item in arr {
                extract_bibtex_from_json(item, push);
            }
        }
        _ => {}
    }
}

fn is_bib_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("bib"))
}

fn collect_bib_files(dir: &Path, push: &mut impl FnMut(&str)) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else { continue };
        if research_tools::is_symlink_or_junction(&path, &meta) {
            continue;
        }
        if meta.is_dir() {
            collect_bib_files(&path, push);
        } else if meta.is_file() && is_bib_file(&path) {
            if let Ok(txt) = fs::read_to_string(&path) {
                push(&txt);
            }
        }
    }
}

pub(crate) fn escape_latex_unicode(input: &str, math_mode: bool) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if let Some(cmd) = unicode_math_macro(ch) {
            if math_mode {
                out.push_str(cmd);
                out.push(' ');
            } else {
                out.push('$');
                out.push_str(cmd);
                out.push('$');
            }
            continue;
        }
        if !math_mode {
            match ch {
                '\\' => out.push_str(r"\textbackslash{}"),
                '&' => out.push_str(r"\&"),
                '%' => out.push_str(r"\%"),
                '$' => out.push_str(r"\$"),
                '#' => out.push_str(r"\#"),
                '_' => out.push_str(r"\_"),
                '{' => out.push_str(r"\{"),
                '}' => out.push_str(r"\}"),
                '~' => out.push_str(r"\textasciitilde{}"),
                '^' => out.push_str(r"\textasciicircum{}"),
                _ => out.push(ch),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn unicode_math_macro(ch: char) -> Option<&'static str> {
    Some(match ch {
        'α' => r"\alpha",
        'β' => r"\beta",
        'γ' => r"\gamma",
        'δ' => r"\delta",
        'ε' | 'ϵ' => r"\epsilon",
        'ζ' => r"\zeta",
        'η' => r"\eta",
        'θ' => r"\theta",
        'ι' => r"\iota",
        'κ' => r"\kappa",
        'λ' => r"\lambda",
        'μ' => r"\mu",
        'ν' => r"\nu",
        'ξ' => r"\xi",
        'π' => r"\pi",
        'ρ' => r"\rho",
        'σ' => r"\sigma",
        'τ' => r"\tau",
        'φ' | 'ϕ' => r"\phi",
        'χ' => r"\chi",
        'ψ' => r"\psi",
        'ω' => r"\omega",
        'Γ' => r"\Gamma",
        'Δ' => r"\Delta",
        'Θ' => r"\Theta",
        'Λ' => r"\Lambda",
        'Σ' => r"\Sigma",
        'Φ' => r"\Phi",
        'Ψ' => r"\Psi",
        'Ω' => r"\Omega",
        'ℏ' => r"\hbar",
        '⊗' => r"\otimes",
        '⊕' => r"\oplus",
        '±' => r"\pm",
        '∓' => r"\mp",
        '×' => r"\times",
        '·' => r"\cdot",
        '≤' => r"\le",
        '≥' => r"\ge",
        '≠' => r"\neq",
        '≈' => r"\approx",
        '≡' => r"\equiv",
        '∞' => r"\infty",
        '∂' => r"\partial",
        '∇' => r"\nabla",
        '∑' => r"\sum",
        '∏' => r"\prod",
        '∫' => r"\int",
        '√' => r"\sqrt{}",
        '→' => r"\to",
        '⇒' => r"\Rightarrow",
        '⇔' => r"\Leftrightarrow",
        '∈' => r"\in",
        '∉' => r"\notin",
        '∀' => r"\forall",
        '∃' => r"\exists",
        '†' => r"\dagger",
        _ => return None,
    })
}

fn write_display_math(out: &mut String, latex: &str) {
    let environment = latex
        .strip_prefix(r"\begin")
        .and_then(|rest| rest.trim_start().strip_prefix('{'))
        .and_then(|rest| rest.split_once('}'))
        .map(|(name, _)| name.trim_end_matches('*'));
    if latex.starts_with(r"\[")
        || matches!(
            environment,
            Some("align" | "alignat" | "displaymath" | "eqnarray" | "equation" | "flalign" | "gather" | "multline" | "subequations")
        )
    {
        out.push_str(&escape_latex_unicode(latex, true));
        out.push('\n');
        return;
    }

    let inner = [("$$", "$$"), ("$", "$"), (r"\(", r"\)")]
        .into_iter()
        .find_map(|(open, close)| latex.strip_prefix(open)?.strip_suffix(close))
        .unwrap_or(latex)
        .trim();
    out.push_str("\\[\n");
    out.push_str(&escape_latex_unicode(inner, true));
    out.push_str("\n\\]\n");
}

pub(crate) fn render_latex_document(data: &SessionExportData, bib_stem: &str) -> String {
    let mut out = String::from(
        "\\documentclass[11pt]{article}\n\
         \\usepackage[utf8]{inputenc}\n\
         \\usepackage[T1]{fontenc}\n\
         \\usepackage{amsmath,amssymb,amsthm}\n\
         \\usepackage{hyperref}\n\n\
         \\newtheorem{assumption}{Assumption}\n\
         \\newtheorem{lemma}{Lemma}\n\
         \\newtheorem{proposition}{Proposition}\n\n",
    );
    out.push_str(&format!(
        "\\title{{re:Search Derivation Report: {}}}\n\
         \\author{{re:Search Verified Ledger (Model: {})}}\n\
         \\date{{Policy: {}, Chain Head: \\texttt{{{}}}}}\n\n\
         \\begin{{document}}\n\\maketitle\n\n\
         \\section{{Problem Statement}}\n{}\n\n",
        escape_latex_unicode(&data.session, false),
        escape_latex_unicode(&data.model, false),
        escape_latex_unicode(&data.policy_version, false),
        escape_latex_unicode(&data.head_hash, false),
        escape_latex_unicode(&data.prompt, false),
    ));

    if !data.entries.is_empty() {
        out.push_str("\\section{Derivation Steps}\n");
        for e in &data.entries {
            let env = match e.entry_type.as_str() {
                "assumption" => Some("assumption"),
                "derivation" => Some("lemma"),
                "result" | "final_answer" => Some("proposition"),
                _ => None,
            };
            let title = format!(
                "{} [{}]",
                escape_latex_unicode(&e.entry_id, false),
                escape_latex_unicode(&e.status, false)
            );
            if let Some(env_name) = env {
                out.push_str(&format!("\\begin{{{env_name}}}[{{{title} (seq {})}}]\n", e.seq));
                out.push_str(&format!(
                    "\\textbf{{{}}}\\\\[4pt]\n{}\n",
                    escape_latex_unicode(&e.summary, false),
                    escape_latex_unicode(&e.details, false)
                ));
            } else {
                out.push_str(&format!(
                    "\\paragraph{{{} ({}, seq {})}}\n{}\n\\begin{{verbatim}}\n{}\n\\end{{verbatim}}\n\n",
                    escape_latex_unicode(&e.entry_id, false),
                    escape_latex_unicode(&e.entry_type, false),
                    e.seq,
                    escape_latex_unicode(&e.summary, false),
                    e.details.replace("\\end{verbatim}", "\\end{verbatim }")
                ));
            }
            if let Some(ltx) = e.latex.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                write_display_math(&mut out, ltx);
            }
            if !e.depends_on.is_empty() {
                let deps = e.depends_on.iter().map(|d| escape_latex_unicode(d, false)).collect::<Vec<_>>().join(", ");
                out.push_str(&format!("\\emph{{Depends on:}} {deps}\n"));
            }
            if let Some(env_name) = env {
                out.push_str(&format!("\\end{{{env_name}}}\n\n"));
            }
        }
    }

    if !data.sympy_calls.is_empty() {
        out.push_str("\\section{Verified Symbolic Computations}\n");
        for c in &data.sympy_calls {
            out.push_str(&format!(
                "\\subsection*{{Step {} (\\texttt{{{}}})}}\n\\begin{{verbatim}}\n{}\n\\end{{verbatim}}\n",
                c.seq,
                escape_latex_unicode(&c.call_id, false),
                c.code.replace("\\end{verbatim}", "\\end{verbatim }")
            ));
            if !c.output.trim().is_empty() {
                out.push_str(&format!(
                    "\\paragraph{{Output}}\n\\begin{{verbatim}}\n{}\n\\end{{verbatim}}\n",
                    c.output.replace("\\end{verbatim}", "\\end{verbatim }")
                ));
            }
        }
    }

    if !data.bibtex.trim().is_empty() {
        out.push_str(&format!(
            "\\nocite{{*}}\n\\bibliographystyle{{unsrt}}\n\\bibliography{{{}}}\n",
            bib_stem
        ));
    }
    out.push_str("\\end{document}\n");
    out
}
