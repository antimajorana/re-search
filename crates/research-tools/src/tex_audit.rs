//! Deterministic zero-LLM TeX & citation auditor (`tex_audit`) + `<deterministic_tex_audit>`
//! XML formatter for Step Verifier and Full-Solution Verifier.

use crate::sandbox::{validate_rel_workspace_path, SandboxRoot, WorkspaceSandbox};
use crate::tex_audit_sources::{
    collect_ledger_retractions, collect_text_files, manuscript_files, scan_json_for_retractions, LoadedFile,
};
use crate::tex_scan::{
    base_arxiv_id, extract_bibitem_keys, extract_tex_citations, extract_tex_labels,
    extract_tex_quote_blocks, extract_tex_refs, normalize_doi, normalize_ws, parse_bib_entries,
    scan_arxiv_base_ids, scan_normalized_dois, strip_latex_comments, BibEntryMeta,
};
use crate::ResearchEnv;
use ledger::Ledger;
use policy::EffectClass;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::sync::Arc;
use tools::{ApprovalScope, CallCtx, Registry, ToolDef};

pub const TEX_AUDIT: &str = "tex_audit";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuditSeverity {
    Blocker,
    Error,
    Warning,
}

pub type TexAuditSeverity = AuditSeverity;

impl AuditSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            AuditSeverity::Blocker => "blocker",
            AuditSeverity::Error => "error",
            AuditSeverity::Warning => "warning",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditIssue {
    pub severity: AuditSeverity,
    pub kind: String,
    pub file: String,
    pub line: Option<u32>,
    pub target: String,
    pub message: String,
}

pub type TexAuditFinding = AuditIssue;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuditOptions {
    pub target_path: Option<String>,
    pub candidate_texts: Vec<(String, String)>,
    #[serde(default)]
    pub candidate_only: bool,
    pub quotes: Vec<String>,
    pub withdrawn_arxiv_ids: Vec<String>,
    pub retracted_dois: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditReport {
    pub ok: bool,
    pub status: String,
    pub files_scanned: usize,
    pub labels_indexed: usize,
    pub bib_keys_indexed: usize,
    pub blocker_count: usize,
    pub error_count: usize,
    pub warning_count: usize,
    pub issues: Vec<AuditIssue>,
}

pub type TexAuditSummary = AuditReport;

impl AuditReport {
    pub fn to_deterministic_xml(&self) -> String {
        let mut out = String::new();
        let _ = write!(
            out,
            "<deterministic_tex_audit ok=\"{}\" status=\"{}\" files_scanned=\"{}\" \
             labels_indexed=\"{}\" bib_keys_indexed=\"{}\" blocker_count=\"{}\" \
             error_count=\"{}\" warning_count=\"{}\"",
            self.ok,
            xml_escape(&self.status),
            self.files_scanned,
            self.labels_indexed,
            self.bib_keys_indexed,
            self.blocker_count,
            self.error_count,
            self.warning_count,
        );
        if self.issues.is_empty() {
            out.push_str("/>");
            debug_assert!(roxmltree::Document::parse(&out).is_ok());
            return out;
        }
        out.push_str(">\n");
        for issue in &self.issues {
            let line_attr = issue.line.map(|n| format!(" line=\"{n}\"")).unwrap_or_default();
            let _ = writeln!(
                out,
                "  <issue severity=\"{}\" kind=\"{}\" file=\"{}\"{} target=\"{}\">{}</issue>",
                issue.severity.as_str(),
                xml_escape(&issue.kind),
                xml_escape(&issue.file),
                line_attr,
                xml_escape(&issue.target),
                xml_escape(&issue.message),
            );
        }
        out.push_str("</deterministic_tex_audit>");
        debug_assert!(roxmltree::Document::parse(&out).is_ok());
        out
    }
}

pub fn format_deterministic_tex_audit_xml(report: &AuditReport) -> String {
    let xml = report.to_deterministic_xml();
    let _ = roxmltree::Document::parse(&xml).expect("deterministic_tex_audit XML must be well-formed");
    xml
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if matches!(c as u32, 0x00..=0x08 | 0x0B | 0x0C | 0x0E..=0x1F | 0x7F) => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            '\u{FFFE}' | '\u{FFFF}' => out.push('\u{FFFD}'),
            c => out.push(c),
        }
    }
    out
}

pub fn tex_audit_def() -> ToolDef {
    ToolDef {
        id: TEX_AUDIT.into(),
        description: "Deterministically audit LaTeX `.tex` and `.bib` files across the workspace \
                      and ingested session sources: checks undefined `\\ref`/`\\eqref`/`\\cref` \
                      labels, missing `.bib` `\\cite` keys, `25..=2000`-char verbatim source quotes, \
                      and reliance on withdrawn arXiv papers or retracted DOIs."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "maxLength": 512, "description": "Optional workspace-relative `.tex` file to focus ref/cite checks on." },
                "candidate_tex": { "type": "string", "maxLength": 500000, "description": "Optional inline LaTeX snippet/derivation to audit against indexed sources." },
                "quotes": { "type": "array", "description": "Optional array of verbatim source quotes (25..=2000 chars) to verify." },
                "withdrawn_arxiv_ids": { "type": "array", "description": "Optional withdrawn arXiv IDs." },
                "retracted_dois": { "type": "array", "description": "Optional retracted DOIs." }
            },
            "additionalProperties": false
        }),
        effect: EffectClass::ReadWorkspace,
        approval_scope: ApprovalScope::Once,
    }
}

pub fn register(registry: &mut Registry, env: Arc<ResearchEnv>) {
    registry.register(
        tex_audit_def(),
        Arc::new(move |ctx: &CallCtx<'_>, args: &Value| {
            let mut opts = AuditOptions {
                target_path: args.get("path").and_then(Value::as_str).map(str::to_string),
                ..AuditOptions::default()
            };
            if let Some(cand) = args.get("candidate_tex").and_then(Value::as_str) {
                opts.candidate_texts.push(("candidate_tex".into(), cand.to_string()));
            }
            opts.quotes = string_array_arg(args, "quotes")?;
            opts.withdrawn_arxiv_ids = string_array_arg(args, "withdrawn_arxiv_ids")?;
            opts.retracted_dois = string_array_arg(args, "retracted_dois")?;

            let ledger_guard = ctx.ledger.lock().map_err(|_| "ledger lock poisoned".to_string())?;
            let report = run_tex_audit(&env, ctx.session, Some(&ledger_guard), &opts)?;
            let mut val = serde_json::to_value(&report).map_err(|e| e.to_string())?;
            val["deterministic_xml"] = Value::String(report.to_deterministic_xml());
            Ok(val)
        }),
    );
}

fn string_array_arg(args: &Value, key: &str) -> Result<Vec<String>, String> {
    let Some(arr) = args.get(key) else { return Ok(Vec::new()) };
    let items = arr.as_array().ok_or_else(|| format!("`{key}` must be an array of strings"))?;
    items
        .iter()
        .map(|v| v.as_str().map(str::to_string).ok_or_else(|| format!("all items in `{key}` must be strings")))
        .collect()
}

pub fn audit_tex_sources(
    env: &ResearchEnv,
    session: &str,
    ledger: Option<&Ledger>,
    opts: &AuditOptions,
) -> Result<AuditReport, String> {
    run_tex_audit(env, session, ledger, opts)
}

pub fn run_tex_audit(
    env: &ResearchEnv,
    session: &str,
    ledger: Option<&Ledger>,
    opts: &AuditOptions,
) -> Result<AuditReport, String> {
    let sandbox = env.sandbox();
    let mut files = Vec::new();
    if let Some(ws) = &env.workspace_root {
        if let Ok(canon_ws) = WorkspaceSandbox::canonicalize_existing_dir(ws) {
            collect_text_files(&canon_ws, &canon_ws, "workspace", false, &mut files);
        }
    }
    if let Ok(Some(canon_src)) = sandbox.canonical_root(SandboxRoot::Sources, session, false) {
        collect_text_files(&canon_src, &canon_src, "sources", true, &mut files);
    }
    let mut candidate_in_target = None;
    let target_scope = if let Some(raw) = &opts.target_path {
        let rel = validate_rel_workspace_path(raw)?.join("/");
        let target = files.iter().position(|file| file.ext == "tex" && !file.from_sources_dir && file.display_id == rel)
            .or_else(|| files.iter().position(|file| {
                file.ext == "tex" && file.from_sources_dir
                    && (file.display_id == rel || file.display_id.strip_prefix("sources/") == Some(rel.as_str()))
            }))
            .ok_or_else(|| format!("target `.tex` file `{rel}` not found"))?;
        let root = &mut files[target];
        if let Some((idx, (_, draft))) = opts.candidate_texts.iter().enumerate().find(|(_, (name, _))| {
            name == raw || name == &rel || name == &root.display_id
        }) {
            // The canvas passes the editor's current buffer under the manuscript path.
            root.raw.clone_from(draft);
            root.stripped_tex = strip_latex_comments(draft);
            root.norm_raw = normalize_ws(draft);
            root.norm_stripped = normalize_ws(&root.stripped_tex);
            candidate_in_target = Some(idx);
        }
        Some(manuscript_files(&files, &files[target]))
    } else {
        None
    };

    let mut withdrawn_arxiv: BTreeMap<String, String> = BTreeMap::new();
    for id in &opts.withdrawn_arxiv_ids {
        if let Some(base) = base_arxiv_id(id) {
            withdrawn_arxiv.insert(base, id.trim().to_string());
        }
    }
    let mut retracted_dois: BTreeSet<String> = opts
        .retracted_dois
        .iter()
        .filter_map(|d| normalize_doi(d))
        .collect();
    if let Some(ldg) = ledger {
        collect_ledger_retractions(ldg, session, &mut withdrawn_arxiv, &mut retracted_dois);
    }
    for f in &files {
        if f.ext == "json" {
            if let Ok(v) = serde_json::from_str::<Value>(&f.raw) {
                scan_json_for_retractions(&v, &mut withdrawn_arxiv, &mut retracted_dois);
            }
        }
    }

    let mut known_labels = BTreeSet::new();
    let mut bib_entries: BTreeMap<String, BibEntryMeta> = BTreeMap::new();
    let mut known_bib_keys = BTreeSet::new();
    let mut issues = Vec::new();

    for f in &files {
        if target_scope.as_ref().is_some_and(|scope| !scope.contains(&f.display_id)) {
            continue;
        }
        if f.ext == "tex" {
            if opts.candidate_only {
                continue;
            }
            let mut file_labels = BTreeMap::new();
            for (lbl, line) in extract_tex_labels(&f.stripped_tex) {
                if !f.from_sources_dir {
                    if let Some(prev_line) = file_labels.insert(lbl.clone(), line) {
                        issues.push(AuditIssue {
                            severity: AuditSeverity::Warning,
                            kind: "duplicate_label".into(),
                            file: f.display_id.clone(),
                            line: Some(line),
                            target: lbl.clone(),
                            message: format!("Duplicate `\\label{{{lbl}}}` (first defined on line {prev_line})."),
                        });
                    }
                }
                known_labels.insert(lbl);
            }
            for k in extract_bibitem_keys(&f.stripped_tex) {
                known_bib_keys.insert(k);
            }
        } else if f.ext == "bib" {
            for (k, meta) in parse_bib_entries(&f.raw) {
                known_bib_keys.insert(k.clone());
                bib_entries.insert(k, meta);
            }
        } else if f.ext == "bbl" {
            for k in extract_bibitem_keys(&strip_latex_comments(&f.raw)) {
                known_bib_keys.insert(k);
            }
        }
    }
    for (index, (_, cand_tex)) in opts.candidate_texts.iter().enumerate() {
        if candidate_in_target == Some(index) {
            continue;
        }
        let stripped = strip_latex_comments(cand_tex);
        for (lbl, _) in extract_tex_labels(&stripped) {
            known_labels.insert(lbl);
        }
        for k in extract_bibitem_keys(&stripped) {
            known_bib_keys.insert(k);
        }
    }

    let has_workspace_tex = files.iter().any(|f| !f.from_sources_dir && f.ext == "tex");

    let mut audit_targets: Vec<(String, String, Option<usize>)> = Vec::new();
    for (idx, f) in files.iter().enumerate() {
        if f.ext != "tex" || opts.candidate_only {
            continue;
        }
        let include = if let Some(scope) = &target_scope {
            scope.contains(&f.display_id)
        } else if !f.from_sources_dir {
            true
        } else {
            !has_workspace_tex && opts.candidate_texts.is_empty()
        };
        if include {
            audit_targets.push((f.display_id.clone(), f.stripped_tex.clone(), Some(idx)));
        }
    }
    for (index, (name, text)) in opts.candidate_texts.iter().enumerate() {
        if candidate_in_target != Some(index) {
            audit_targets.push((name.clone(), strip_latex_comments(text), None));
        }
    }

    for (doc_name, stripped, self_idx) in &audit_targets {
        for r in extract_tex_refs(stripped) {
            if r.key.is_empty() || !known_labels.contains(&r.key) {
                issues.push(AuditIssue {
                    severity: AuditSeverity::Error,
                    kind: "undefined_ref".into(),
                    file: doc_name.clone(),
                    line: Some(r.line),
                    target: r.key.clone(),
                    message: format!("Undefined LaTeX reference `{}` in `\\{}{{...}}`.", r.key, r.command),
                });
            }
        }
        for c in extract_tex_citations(stripped) {
            if c.key.is_empty() || !known_bib_keys.contains(&c.key) {
                issues.push(AuditIssue {
                    severity: AuditSeverity::Error,
                    kind: "missing_citation_key".into(),
                    file: doc_name.clone(),
                    line: Some(c.line),
                    target: c.key.clone(),
                    message: format!(
                        "Citation key `{}` in `\\{}{{...}}` not found in `.bib` or `\\bibitem`.",
                        c.key, c.command
                    ),
                });
            } else if let Some(meta) = bib_entries.get(&c.key) {
                check_bib_entry_blockers(doc_name, c.line, meta, &withdrawn_arxiv, &retracted_dois, &mut issues);
            }
        }
        for base_id in scan_arxiv_base_ids(stripped) {
            if let Some(orig_withdrawn) = withdrawn_arxiv.get(&base_id) {
                issues.push(AuditIssue {
                    severity: AuditSeverity::Blocker,
                    kind: "withdrawn_arxiv".into(),
                    file: doc_name.clone(),
                    line: None,
                    target: base_id.clone(),
                    message: format!(
                        "Relies on withdrawn arXiv paper `{base_id}` (withdrawn record: `{orig_withdrawn}`)."
                    ),
                });
            }
        }
        for doi in scan_normalized_dois(stripped) {
            if retracted_dois.contains(&doi) {
                issues.push(AuditIssue {
                    severity: AuditSeverity::Blocker,
                    kind: "retracted_doi".into(),
                    file: doc_name.clone(),
                    line: None,
                    target: doi.clone(),
                    message: format!("Relies on retracted DOI `{doi}`."),
                });
            }
        }
        for (norm_quote, q_line) in extract_tex_quote_blocks(stripped) {
            if !quote_matches_any_source(&norm_quote, &files, *self_idx) {
                issues.push(AuditIssue {
                    severity: AuditSeverity::Error,
                    kind: "unverified_quote".into(),
                    file: doc_name.clone(),
                    line: Some(q_line),
                    target: truncate_preview(&norm_quote, 60),
                    message: "Verbatim LaTeX quote block could not be verified in any source document.".into(),
                });
            }
        }
    }

    for raw_q in &opts.quotes {
        let norm_q = normalize_ws(raw_q);
        let char_len = norm_q.chars().count();
        if !(25..=2000).contains(&char_len) {
            issues.push(AuditIssue {
                severity: AuditSeverity::Warning,
                kind: "quote_length_out_of_bounds".into(),
                file: "quotes".into(),
                line: None,
                target: truncate_preview(&norm_q, 60),
                message: format!("Quote length ({char_len} chars) outside verifiable 25..=2000 char range."),
            });
            continue;
        }
        if !quote_matches_any_source(&norm_q, &files, None) {
            issues.push(AuditIssue {
                severity: AuditSeverity::Error,
                kind: "unverified_quote".into(),
                file: "quotes".into(),
                line: None,
                target: truncate_preview(&norm_q, 60),
                message: "Explicit verbatim quote not found in any workspace or session source file.".into(),
            });
        }
    }

    issues.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.target.cmp(&b.target))
    });
    issues.dedup();

    let blocker_count = issues.iter().filter(|i| i.severity == AuditSeverity::Blocker).count();
    let error_count = issues.iter().filter(|i| i.severity == AuditSeverity::Error).count();
    let warning_count = issues.iter().filter(|i| i.severity == AuditSeverity::Warning).count();
    let ok = blocker_count == 0 && error_count == 0;
    let status = if blocker_count > 0 {
        "blocker"
    } else if error_count > 0 {
        "error"
    } else {
        "clean"
    };

    Ok(AuditReport {
        ok,
        status: status.into(),
        files_scanned: files.len(),
        labels_indexed: known_labels.len(),
        bib_keys_indexed: known_bib_keys.len(),
        blocker_count,
        error_count,
        warning_count,
        issues,
    })
}

fn check_bib_entry_blockers(
    doc_name: &str,
    line: u32,
    meta: &BibEntryMeta,
    withdrawn_arxiv: &BTreeMap<String, String>,
    retracted_dois: &BTreeSet<String>,
    issues: &mut Vec<AuditIssue>,
) {
    if let Some(base) = &meta.base_arxiv_id {
        if meta.marked_withdrawn || withdrawn_arxiv.contains_key(base) {
            issues.push(AuditIssue {
                severity: AuditSeverity::Blocker,
                kind: "withdrawn_arxiv".into(),
                file: doc_name.into(),
                line: Some(line),
                target: base.clone(),
                message: format!("Citation `{}` relies on withdrawn arXiv paper `{base}`.", meta.key),
            });
        }
    }
    if let Some(doi) = &meta.doi_lower {
        if meta.marked_retracted || retracted_dois.contains(doi) {
            issues.push(AuditIssue {
                severity: AuditSeverity::Blocker,
                kind: "retracted_doi".into(),
                file: doc_name.into(),
                line: Some(line),
                target: doi.clone(),
                message: format!("Citation `{}` relies on retracted DOI `{doi}`.", meta.key),
            });
        }
    }
}

fn quote_matches_any_source(norm_quote: &str, files: &[LoadedFile], exclude_idx: Option<usize>) -> bool {
    files.iter().enumerate().any(|(idx, f)| {
        Some(idx) != exclude_idx && (f.norm_raw.contains(norm_quote) || f.norm_stripped.contains(norm_quote))
    })
}

fn truncate_preview(s: &str, max_chars: usize) -> String {
    let mut out: String = s.chars().take(max_chars).collect();
    if s.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}
