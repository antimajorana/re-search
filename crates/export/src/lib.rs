//! Verified session forking, structural branch diffing, and reproducible
//! publication export (`.tex` + `.bib`, standalone `.html`, and `.zip`).

pub mod diff;
pub mod fork;
pub mod handoff;
mod handoff_types;
mod handoff_view;
pub mod html;
pub mod latex;
pub mod paper_map;
pub mod paper_map_html;
pub mod paper_map_import;
pub mod writeup;
pub mod writeup_bib;
pub mod writeup_md;
pub mod writeup_paper;
mod writeup_paper_anchors;
mod writeup_paper_meta;
mod writeup_paper_nodes;
pub mod writeup_tex;
pub mod zip;

use ledger::Ledger;
use research_tools::validate_session_id;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForkOptions {
    pub parent_session: String,
    pub at_seq: u64,
    pub modified_assumption: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForkReceipt {
    pub new_session: String,
    pub parent_session: String,
    pub parent_seq: u64,
    pub parent_event_hash: String,
    pub copied_events: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchDiffReport {
    pub base_session: String,
    pub target_session: String,
    pub shared_prefix_len: usize,
    pub divergence_seq: Option<u64>,
    pub base_only_events: usize,
    pub target_only_events: usize,
    pub base_entries: Vec<Value>,
    pub target_entries: Vec<Value>,
    pub base_final_answer: Option<String>,
    pub target_final_answer: Option<String>,
    pub summary_diff: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Tex,
    Html,
    Zip,
}

impl ExportFormat {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "tex" | "latex" => Ok(ExportFormat::Tex),
            "html" => Ok(ExportFormat::Html),
            "zip" => Ok(ExportFormat::Zip),
            other => Err(format!("unknown export format `{other}` (expected `tex`, `html`, or `zip`)")),
        }
    }
}

pub fn fork_session(data_dir: &Path, opts: &ForkOptions) -> Result<ForkReceipt, String> {
    fork::fork_session(data_dir, opts)
}

pub fn compute_branch_diff(
    ledger: &Ledger,
    base_session: &str,
    target_session: &str,
) -> Result<BranchDiffReport, String> {
    diff::compute_branch_diff(ledger, base_session, target_session)
}

pub fn export_session_to_path(
    data_dir: &Path,
    session: &str,
    format: ExportFormat,
    out_path: &Path,
) -> Result<Vec<u8>, String> {
    validate_session_id(session).map_err(|e| e.to_string())?;
    let data = latex::load_session_export_data(data_dir, session)?;
    if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| format!("failed to create export dir: {e}"))?;
    }
    let bytes = match format {
        ExportFormat::Tex => {
            let bib_stem = out_path
                .file_stem()
                .and_then(|s| s.to_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("references");
            let tex_bytes = latex::render_latex_document(&data, bib_stem).into_bytes();
            let bib_path = out_path.with_extension("bib");
            fs::write(&bib_path, data.bibtex.as_bytes())
                .map_err(|e| format!("failed to write sidecar `.bib` `{}`: {e}", bib_path.display()))?;
            tex_bytes
        }
        ExportFormat::Html => html::render_html_document(&data).into_bytes(),
        ExportFormat::Zip => zip::build_reproducible_zip(data_dir, &data)?,
    };
    fs::write(out_path, &bytes)
        .map_err(|e| format!("failed to write export `{}`: {e}", out_path.display()))?;
    Ok(bytes)
}

pub fn export_session_bytes(
    data_dir: &Path,
    session: &str,
    format: ExportFormat,
) -> Result<(String, Vec<u8>), String> {
    validate_session_id(session).map_err(|e| e.to_string())?;
    let data = latex::load_session_export_data(data_dir, session)?;
    match format {
        ExportFormat::Tex => Ok((
            format!("{session}-report.tex"),
            latex::render_latex_document(&data, "references").into_bytes(),
        )),
        ExportFormat::Html => Ok((
            format!("{session}-report.html"),
            html::render_html_document(&data).into_bytes(),
        )),
        ExportFormat::Zip => Ok((
            format!("{session}-bundle.zip"),
            zip::build_reproducible_zip(data_dir, &data)?,
        )),
    }
}

/// Output format for F15 reading-notes / derivation / summary write-ups (`00-overview.md` 2.13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WriteupFormat {
    Tex,
    Md,
    Bib,
}

impl WriteupFormat {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "tex" | "latex" => Ok(Self::Tex),
            "md" | "markdown" => Ok(Self::Md),
            "bib" | "bibtex" => Ok(Self::Bib),
            other => Err(format!(
                "unknown write-up format `{other}` (expected `tex`, `md`, or `bib`)"
            )),
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Tex => "tex",
            Self::Md => "md",
            Self::Bib => "bib",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Self::Tex => "application/x-tex; charset=utf-8",
            Self::Md => "text/markdown; charset=utf-8",
            Self::Bib => "application/x-bibtex; charset=utf-8",
        }
    }
}

/// Dispatch a verified session to its mode-specific F15 write-up adapter (zero model calls).
pub fn build_session_writeup(data_dir: &Path, session: &str) -> Result<writeup::Writeup, String> {
    validate_session_id(session).map_err(|e| e.to_string())?;
    let ledger = Ledger::open_existing(data_dir).map_err(|e| e.to_string())?;
    let (_report, events) = ledger
        .read_verified_session(session)
        .map_err(|e| format!("cannot build write-up for unverified session `{session}`: {e}"))?;
    let mode = research_tools::tool_profile::session_mode_from_events(&events);
    match mode {
        research_tools::paper_key::Mode::Investigate => writeup_paper::build_writeup_from_verified(
            data_dir,
            session,
            &ledger,
            &events,
            writeup_paper::PaperWriteupOptions::default(),
        )
        .map(|b| b.writeup),
        other => Err(format!(
            "write-up adapter not built yet for mode `{}`",
            other.as_str()
        )),
    }
}

/// Build and render a session's F15 write-up to `(filename, bytes)` in `Tex`, `Md`, or `Bib`.
pub fn writeup_session_bytes(
    data_dir: &Path,
    session: &str,
    format: WriteupFormat,
    opts: &writeup::WriteupOptions,
) -> Result<(String, Vec<u8>), String> {
    let doc = build_session_writeup(data_dir, session)?;
    match format {
        WriteupFormat::Tex => {
            let aligned_stem = if opts.bib_stem.trim().is_empty() || opts.bib_stem == "references" {
                format!("{session}-writeup")
            } else {
                opts.bib_stem.clone()
            };
            let tex_opts = writeup::WriteupOptions {
                include_inline_badges: opts.include_inline_badges,
                bib_stem: aligned_stem,
            };
            Ok((
                format!("{session}-writeup.tex"),
                writeup_tex::render_writeup_tex(&doc, &tex_opts)?.into_bytes(),
            ))
        }
        WriteupFormat::Md => Ok((
            format!("{session}-writeup.md"),
            writeup_md::render_writeup_markdown(&doc, opts)?.into_bytes(),
        )),
        WriteupFormat::Bib => Ok((
            format!("{session}-writeup.bib"),
            writeup_bib::render_bibtex(&doc.bibliography).into_bytes(),
        )),
    }
}

/// Build and write a session's F15 write-up to `out_path`.
///
/// For `WriteupFormat::Tex`, writes BOTH `<stem>.tex` and sidecar `<stem>.bib` automatically.
pub fn writeup_session_to_path(
    data_dir: &Path,
    session: &str,
    format: WriteupFormat,
    opts: &writeup::WriteupOptions,
    out_path: &Path,
) -> Result<Vec<u8>, String> {
    let doc = build_session_writeup(data_dir, session)?;
    if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| format!("failed to create write-up dir: {e}"))?;
    }
    let bytes = match format {
        WriteupFormat::Tex => {
            let stem = out_path
                .file_stem()
                .and_then(|s| s.to_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(opts.bib_stem.as_str());
            let tex_opts = writeup::WriteupOptions {
                include_inline_badges: opts.include_inline_badges,
                bib_stem: stem.to_string(),
            };
            let tex_bytes = writeup_tex::render_writeup_tex(&doc, &tex_opts)?.into_bytes();
            let bib_path = out_path.with_extension("bib");
            let bib = writeup_bib::render_bibtex(&doc.bibliography);
            fs::write(&bib_path, bib.as_bytes()).map_err(|e| {
                format!("failed to write sidecar `.bib` `{}`: {e}", bib_path.display())
            })?;
            tex_bytes
        }
        WriteupFormat::Md => writeup_md::render_writeup_markdown(&doc, opts)?.into_bytes(),
        WriteupFormat::Bib => writeup_bib::render_bibtex(&doc.bibliography).into_bytes(),
    };
    fs::write(out_path, &bytes)
        .map_err(|e| format!("failed to write write-up `{}`: {e}", out_path.display()))?;
    Ok(bytes)
}
