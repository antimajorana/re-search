//! Stage 6 sandboxed workspace, document ingestion, LaTeX editing, and audit tools.

pub mod archive;
pub mod arxiv;
pub mod fs_tools;
pub mod http_client;
pub mod ingest;
pub mod latex_compile;
pub mod latex_repair;
pub mod literature;
pub mod pdf;
pub mod policy_preset;
pub mod proof_dag;
pub mod sandbox;
pub mod sanity;
pub mod tex_audit;
mod tex_audit_sources;
pub mod tex_bundle;
pub mod tex_scan;
pub mod write_tools;

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
pub use literature::{
    literature_search_def, run_literature_search, LiteratureHit, LiteratureRequest,
    LITERATURE_SEARCH,
};
pub use policy_preset::{research_policy, RESEARCH_POLICY_VERSION};
pub use proof_dag::{
    ledger_append_def, ledger_query_def, ledger_supersede_def, read_proof_dag, ProofDag,
    ProofEntry, ProofStatus, LEDGER_APPEND, LEDGER_QUERY, LEDGER_SUPERSEDE,
};
pub use sandbox::{
    resolve_sources_session_id, strip_verbatim_prefix, strip_verbatim_str, validate_session_id,
    RootKind, SandboxError, SandboxRoot, WorkspaceSandbox,
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

#[derive(Debug, Clone)]
pub struct ResearchEnv {
    pub data_dir: PathBuf,
    pub workspace_root: Option<PathBuf>,
    pub network_endpoints: LiteratureEndpoints,
    pub latex_compiler_override: Option<PathBuf>,
    pub latex_compile_timeout: Duration,
}

impl ResearchEnv {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            workspace_root: None,
            network_endpoints: LiteratureEndpoints::default(),
            latex_compiler_override: None,
            latex_compile_timeout: DEFAULT_LATEX_COMPILE_TIMEOUT,
        }
    }

    pub fn with_workspace(
        data_dir: impl Into<PathBuf>,
        workspace_root: impl AsRef<Path>,
    ) -> Result<Self, SandboxError> {
        let canonical = WorkspaceSandbox::canonicalize_existing_dir(workspace_root.as_ref())?;
        Ok(Self {
            data_dir: data_dir.into(),
            workspace_root: Some(canonical),
            network_endpoints: LiteratureEndpoints::default(),
            latex_compiler_override: None,
            latex_compile_timeout: DEFAULT_LATEX_COMPILE_TIMEOUT,
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
    literature::register(registry, env);
    proof_dag::register(registry);
    sanity::register(registry);
}
