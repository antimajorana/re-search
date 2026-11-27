//! Stage 6 sandboxed workspace, document ingestion, LaTeX editing, and audit tools.

pub mod archive;
pub mod arxiv;
pub mod budget_pool;
pub mod formal_library;
pub mod fs_tools;
pub mod http_client;
pub mod idea_queries;
pub mod idea_rank;
pub mod idea_terms;
pub mod ingest;
pub mod investigation;
pub mod investigation_evidence;
pub mod investigation_markers;
pub mod investigation_outline;
pub mod latex_compile;
pub mod latex_repair;
pub mod lean_check;
pub mod lessons;
pub mod lessons_schema;
pub mod lessons_select;
pub mod lines_sent;
pub mod library;
mod library_ext_types;
mod library_marks;
mod library_rebuild;
mod library_rebuild_lit;
pub mod library_schema;
pub mod lit_abstracts;
pub mod lit_providers;
pub mod literature;
pub mod map_authors_claim;
pub mod map_context;
pub mod map_context_defs;
pub mod map_defs;
pub mod map_fold;
pub mod map_model_nodes;
pub mod map_node_read;
pub mod map_reading;
pub mod net_rate;
pub mod paper_fetch;
mod paper_fetch_job;
mod paper_fetch_meta;
mod paper_fetch_validate;
pub mod paper_key;
pub mod paper_lines;
pub mod paper_passages;
pub mod paper_sources;
pub mod pdf;
pub mod pdf_firstpage;
mod pdf_firstpage_norm;
pub mod pdf_match;
pub mod pdf_pages;
mod pdf_pages_bench;
mod pdf_pages_job;
mod pdf_pages_layout;
mod pdf_pages_scan;
pub mod policy_preset;
pub mod prior_work;
pub mod prior_work_wording;
pub mod proof_closure;
pub mod proof_dag;
pub mod quote_core;
pub mod run_budget;
pub mod sandbox;
pub mod sanity;
pub mod search_coverage;
pub mod session_activity;
pub mod skeleton;
mod solve_badge;
pub mod solve_checks;
pub mod solve_defs;
pub mod solve_derivation;
pub mod solve_done;
pub mod solve_exprs;
mod solve_exprs_show;
pub mod solve_gates;
pub mod solve_model;
mod solve_model_records;
mod solve_own_tier;
pub mod solve_artifacts;
mod solve_artifacts_sniff;
pub mod solve_receipts;
pub mod solve_setup;
mod solve_setup_prefill;
pub mod solve_state;
pub mod solve_status;
pub mod solve_steps;
pub mod source_bibtex;
mod source_bibtex_parse;
mod source_bibtex_tex;
pub mod source_dedupe;
pub mod source_export;
mod source_export_bib;
pub mod source_folder;
mod source_folder_parse;
mod source_frontmatter;
pub mod source_kandinsky;
mod source_kandinsky_resolve;
pub mod source_sync;
mod source_sync_reconcile;
pub mod source_zotero;
mod source_zotero_parse;
pub mod status_badge;
pub mod tex_audit;
mod tex_audit_sources;
pub mod tex_bundle;
pub mod tex_scan;
pub mod tool_profile;
pub mod write_tools;

pub use investigation_evidence::{
    note_evidence_def, register_reader_tools, EvidenceRecord, EVIDENCE_EVENT, NOTE_EVIDENCE,
    READER_CALLER_PREFIX,
};

pub use arxiv::{
    arxiv_search_def, normalize_arxiv_id, run_arxiv_search, ArxivPaper, ArxivSearchRequest,
    ARXIV_SEARCH,
};
pub use fs_tools::{FS_LIST, FS_READ};
pub use ingest::{
    assemble_tex_bundle, filter_tex_sections, ingest_bytes, ingest_path, IngestReport,
    IngestRequest, IngestedSource, TexBundle, DOC_INGEST,
};
pub use latex_compile::{LatexDiagnostic, LATEX_COMPILE};
pub use latex_repair::{
    is_likely_tsv_line, repair_json_value_latex, repair_under_escaped_latex, should_skip_json_key,
};
pub use lean_check::{
    lean_check_def, register_lean_tool, run_lean_check, scan_lean_anti_cheat, LeanAntiCheat,
    LeanDiagnostic, LEAN_CHECK,
};
pub use literature::{
    literature_search_def, run_literature_search, LiteratureHit, LiteratureRequest,
    LITERATURE_SEARCH,
};
pub use policy_preset::{
    research_policy, solve_policy, RESEARCH_POLICY_VERSION, SOLVE_POLICY_VERSION,
};
pub use proof_dag::{
    effective_verification_verdict, ledger_append_def, ledger_query_def, ledger_supersede_def, read_proof_dag, ProofDag,
    ProofEntry, ProofStatus, LEDGER_APPEND, LEDGER_QUERY, LEDGER_SUPERSEDE,
};
pub use sandbox::{
    is_symlink_or_junction, resolve_sources_session_id, strip_verbatim_prefix, strip_verbatim_str,
    validate_session_id, RootKind, SandboxError, SandboxRoot, WorkspaceSandbox,
};
pub use sanity::{check_expression_sanity_def, CHECK_EXPRESSION_SANITY};
pub use tex_audit::{
    audit_tex_sources, format_deterministic_tex_audit_xml, run_tex_audit, AuditIssue,
    AuditOptions, AuditReport, AuditSeverity, TexAuditFinding, TexAuditSeverity, TexAuditSummary,
    TEX_AUDIT,
};
pub use write_tools::{preview_workspace_edit, EditPreview, FS_WRITE, LATEX_PATCH};

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tools::{Registry, ToolDef};

pub const DEFAULT_LATEX_COMPILE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiteratureEndpoints {
    pub arxiv_api_base: String,
    pub arxiv_eprint_base: String,
    pub inspire_api_base: String,
    pub crossref_api_base: String,
    pub semantic_scholar_api_base: String,
}

impl Default for LiteratureEndpoints {
    fn default() -> Self {
        Self {
            arxiv_api_base: "https://export.arxiv.org".to_string(),
            arxiv_eprint_base: "https://arxiv.org".to_string(),
            inspire_api_base: "https://inspirehep.net".to_string(),
            crossref_api_base: "https://api.crossref.org".to_string(),
            semantic_scholar_api_base: "https://api.semanticscholar.org".to_string(),
        }
    }
}

impl LiteratureEndpoints {
    pub fn loopback_mock(base_url: &str) -> Result<Self, String> {
        let trimmed = base_url.trim_end_matches('/').to_string();
        if !trimmed.starts_with("http://127.0.0.1:") && !trimmed.starts_with("https://") {
            return Err(
                "literature endpoint override must use https:// or http://127.0.0.1:<port>".into(),
            );
        }
        Ok(Self {
            arxiv_api_base: trimmed.clone(),
            arxiv_eprint_base: trimmed.clone(),
            inspire_api_base: trimmed.clone(),
            crossref_api_base: trimmed.clone(),
            semantic_scholar_api_base: trimmed,
        })
    }
}

/// Canonical form of a path that may not exist yet: canonicalize the deepest existing ancestor
/// (resolving symlinks and junctions there) and re-append the missing tail.
fn canonicalize_lenient(path: &Path) -> PathBuf {
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut tail = Vec::new();
    let mut cur = abs.as_path();
    loop {
        if let Ok(canon) = std::fs::canonicalize(cur) {
            return tail.iter().rev().fold(canon, |acc, seg| acc.join(seg));
        }
        match (cur.parent(), cur.file_name()) {
            (Some(parent), Some(name)) => {
                tail.push(name.to_os_string());
                cur = parent;
            }
            _ => return abs,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResearchEnv {
    pub data_dir: PathBuf,
    pub workspace_root: Option<PathBuf>,
    pub network_endpoints: LiteratureEndpoints,
    pub latex_compiler_override: Option<PathBuf>,
    pub latex_compile_timeout: Duration,
    pub lean_override: Option<PathBuf>,
}

impl ResearchEnv {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            workspace_root: None,
            network_endpoints: LiteratureEndpoints::default(),
            latex_compiler_override: None,
            latex_compile_timeout: DEFAULT_LATEX_COMPILE_TIMEOUT,
            lean_override: None,
        }
    }

    pub fn with_workspace(
        data_dir: impl Into<PathBuf>,
        workspace_root: impl AsRef<Path>,
    ) -> Result<Self, SandboxError> {
        let canonical = WorkspaceSandbox::canonicalize_existing_dir(workspace_root.as_ref())?;
        let data_dir = data_dir.into();
        // `read-workspace` tools run without asking, so the workspace must never give the model
        // a path to `providers.json` (plaintext keys), `ledger.db` or `blobs/`: refuse a
        // workspace that is the data directory, contains it, or lies inside it.
        let canon_data = canonicalize_lenient(&data_dir);
        if WorkspaceSandbox::is_same_or_inside(&canonical, &canon_data)
            || WorkspaceSandbox::is_same_or_inside(&canon_data, &canonical)
        {
            return Err(SandboxError::EscapesRoot(format!(
                "workspace `{}` overlaps the re:Search data directory `{}`; choose a workspace that neither contains it nor lies inside it",
                strip_verbatim_prefix(&canonical).display(),
                strip_verbatim_prefix(&canon_data).display()
            )));
        }
        Ok(Self {
            data_dir,
            workspace_root: Some(canonical),
            network_endpoints: LiteratureEndpoints::default(),
            latex_compiler_override: None,
            latex_compile_timeout: DEFAULT_LATEX_COMPILE_TIMEOUT,
            lean_override: None,
        })
    }

    pub fn with_endpoints(mut self, endpoints: LiteratureEndpoints) -> Self {
        self.network_endpoints = endpoints;
        self
    }

    pub fn with_latex_compiler(mut self, compiler: impl Into<PathBuf>) -> Self {
        self.latex_compiler_override = Some(compiler.into());
        self
    }

    pub fn with_latex_timeout(mut self, timeout: Duration) -> Self {
        self.latex_compile_timeout = timeout;
        self
    }

    pub fn with_lean_override(mut self, path: impl Into<PathBuf>) -> Self {
        self.lean_override = Some(path.into());
        self
    }

    pub fn sandbox(&self) -> WorkspaceSandbox {
        WorkspaceSandbox::new(self.data_dir.clone(), self.workspace_root.clone())
    }
}

pub fn m1_tool_defs() -> Vec<ToolDef> {
    let mut defs = vec![
        ingest::doc_ingest_def(),
        fs_tools::fs_list_def(),
        fs_tools::fs_read_def(),
        write_tools::fs_write_def(),
        latex_compile::latex_compile_def(),
        write_tools::latex_patch_def(),
        tex_audit::tex_audit_def(),
    ];
    defs.sort_by(|a, b| a.id.cmp(&b.id));
    defs
}

pub fn research_tool_defs() -> Vec<ToolDef> {
    m1_tool_defs()
}

pub fn all_research_tool_defs() -> Vec<ToolDef> {
    let mut defs = m1_tool_defs();
    defs.extend([
        arxiv::arxiv_search_def(),
        literature::literature_search_def(),
        formal_library::search_formal_library_def(),
        paper_lines::def(),
        paper_passages::def(),
        investigation::def(),
        investigation::summary_def(),
        investigation::branch_def(),
        investigation::conventions_def(),
        proof_dag::ledger_append_def(),
        proof_dag::ledger_supersede_def(),
        proof_dag::ledger_query_def(),
        sanity::check_expression_sanity_def(),
    ]);
    defs.sort_by(|a, b| a.id.cmp(&b.id));
    defs
}

pub fn register_research_tools(registry: &mut Registry, env: Arc<ResearchEnv>) {
    fs_tools::register(registry, env.clone());
    ingest::register(registry, env.clone());
    write_tools::register(registry, env.clone());
    tex_audit::register(registry, env.clone());
    arxiv::register(registry, env.clone());
    literature::register(registry, env.clone());
    formal_library::register_formal_library(registry, env.clone());
    paper_lines::register(registry, env.clone());
    paper_passages::register(registry, env.clone());
    investigation::register(registry, env);
    proof_dag::register(registry);
    sanity::register(registry);
}
