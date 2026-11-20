//! Deterministic publication LaTeX (`.tex`) and BibTeX (`.bib`) export (F15).

use ledger::{Event, Ledger};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

pub(crate) struct ProofItem {
    pub entry_id: String,
    pub seq: u64,
    pub entry_type: String,
    pub status: String,
    pub summary: String,
    pub details: String,
    pub latex: Option<String>,
    pub depends_on: Vec<String>,
}

pub(crate) struct SympyCall {
    pub seq: u64,
    pub call_id: String,
    pub code: String,
    pub output: String,
}

pub(crate) struct SessionExportData {
    pub session: String,
    pub head_hash: String,
    pub prompt: String,
    pub model: String,
    pub policy_version: String,
    pub entries: Vec<ProofItem>,
    pub sympy_calls: Vec<SympyCall>,
    pub bibtex: String,
    pub events: Vec<Event>,
}

pub(crate) fn load_session_export_data(data_dir: &Path, session: &str) -> Result<SessionExportData, String> {
    let ledger = Ledger::open_existing(data_dir).map_err(|e| e.to_string())?;
    let report = ledger
        .verify_session(session)
        .map_err(|e| format!("cannot export unverified session `{session}`: {e}"))?;
    let events = ledger.read_session(session).map_err(|e| e.to_string())?;
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
    let bibtex = collect_bibtex(data_dir, &ledger, session, &events);

    Ok(SessionExportData {
        session: session.to_string(),
        head_hash: report.head_hash,
        prompt,
        model,
        policy_version,
        entries,
        sympy_calls,
        bibtex,
        events,
    })
}

fn collect_proof_items(events: &[Event]) -> Vec<ProofItem> {
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
    let mut results_by_id: BTreeMap<String, String> = BTreeMap::new();
    for ev in events {
        if ev.kind == "tool.result" {
            if let Some(cid) = ev.payload.get("call_id").and_then(Value::as_str) {
                let ok = ev.payload.get("ok").and_then(Value::as_bool).unwrap_or(false);
                let txt = resolve_slot(ledger, ev.payload.get(if ok { "result" } else { "error" }));
                results_by_id.insert(cid.to_string(), txt);
            }
        }
    }
    let mut calls = Vec::new();
    for ev in events {
        if ev.kind == "tool.request"
            && ev.payload.get("tool").and_then(Value::as_str) == Some("sympy_eval")
        {
            let call_id = ev
                .payload
                .get("call_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let args_raw = resolve_slot(ledger, ev.payload.get("args"));
            let code = serde_json::from_str::<Value>(&args_raw)
                .ok()
                .and_then(|v| v.get("code").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_default();
            if !code.trim().is_empty() {
                calls.push(SympyCall {
                    seq: ev.seq,
                    output: results_by_id.remove(&call_id).unwrap_or_default(),
                    call_id,
                    code,
                });
            }
        }
    }
    calls
}

fn collect_bibtex(data_dir: &Path, ledger: &Ledger, session: &str, events: &[Event]) -> String {
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

    let sources_dir = data_dir.join("sessions").join(session).join("sources");
    if sources_dir.is_dir() {
        collect_bib_files(&sources_dir, &mut push_bib);
    }
    if chunks.is_empty() {
        String::new()
    } else {
        let mut out = chunks.join("\n\n");
        out.push('\n');
        out
    }
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

fn collect_bib_files(dir: &Path, push: &mut impl FnMut(&str)) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else { continue };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            collect_bib_files(&path, push);
        } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("bib")) {
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
                out.push_str(&format!("\\begin{{{env_name}}}[{title} (seq {})]\n", e.seq));
                out.push_str(&format!(
                    "\\textbf{{{}}}\\\\[4pt]\n{}\n",
                    escape_latex_unicode(&e.summary, false),
                    escape_latex_unicode(&e.details, false)
                ));
                if let Some(ltx) = e.latex.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                    out.push_str(&format!("\\[\n{}\n\\]\n", escape_latex_unicode(ltx, true)));
                }
                if !e.depends_on.is_empty() {
                    let deps = e.depends_on.iter().map(|d| escape_latex_unicode(d, false)).collect::<Vec<_>>().join(", ");
                    out.push_str(&format!("\\emph{{Depends on:}} {deps}\n"));
                }
                out.push_str(&format!("\\end{{{env_name}}}\n\n"));
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
            "\\bibliographystyle{{unsrt}}\n\\bibliography{{{}}}\n",
            bib_stem
        ));
    }
    out.push_str("\\end{document}\n");
    out
}
