//! Verified session forking, structural branch diffing, and reproducible
//! publication export (`.tex` + `.bib`, standalone `.html`, `.ipynb`, and `.zip`).

pub mod diff;
pub mod fork;
pub mod handoff;
mod handoff_types;
mod handoff_view;
pub mod html;
pub mod idea;
pub mod latex;
pub mod notebook;
mod notebook_prelude;
mod notebook_render;
pub mod paper_map;
pub mod paper_map_html;
pub mod paper_map_import;
pub mod writeup;
pub mod writeup_bib;
pub mod writeup_md;
pub mod writeup_paper;
mod writeup_paper_anchors;
mod writeup_paper_ext;
mod writeup_paper_meta;
mod writeup_paper_nodes;
pub mod writeup_solve;
mod writeup_solve_cite;
pub mod writeup_tex;
pub mod zip;
mod zip_solve;

use ledger::Ledger;
use research_tools::paper_key::Mode;
use research_tools::tool_profile::session_mode_from_events;
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
    Notebook,
}

impl ExportFormat {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "tex" | "latex" => Ok(ExportFormat::Tex),
            "html" => Ok(ExportFormat::Html),
            "zip" => Ok(ExportFormat::Zip),
            "ipynb" | "notebook" => Ok(ExportFormat::Notebook),
            other => Err(format!(
                "unknown export format `{other}` (expected `tex`, `html`, `zip`, or `notebook`)"
            )),
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
    if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| format!("failed to create export dir: {e}"))?;
    }
    if format != ExportFormat::Tex {
        let (_, bytes) = export_session_bytes_with_options(data_dir, session, format, false)?;
        fs::write(out_path, &bytes)
            .map_err(|e| format!("failed to write export `{}`: {e}", out_path.display()))?;
        return Ok(bytes);
    }

    let data = latex::load_session_export_data(data_dir, session)?;
    let bib_stem = out_path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("references");
    let tex_bytes = latex::render_latex_document(&data, bib_stem).into_bytes();
    let bib_path = out_path.with_extension("bib");
    fs::write(&bib_path, data.bibtex.as_bytes())
        .map_err(|e| format!("failed to write sidecar `.bib` `{}`: {e}", bib_path.display()))?;
    fs::write(out_path, &tex_bytes)
        .map_err(|e| format!("failed to write export `{}`: {e}", out_path.display()))?;
    Ok(tex_bytes)
}

pub fn export_session_bytes(
    data_dir: &Path,
    session: &str,
    format: ExportFormat,
) -> Result<(String, Vec<u8>), String> {
    export_session_bytes_with_options(data_dir, session, format, false)
}

pub fn export_session_bytes_with_options(
    data_dir: &Path,
    session: &str,
    format: ExportFormat,
    include_all_runs: bool,
) -> Result<(String, Vec<u8>), String> {
    validate_session_id(session).map_err(|e| e.to_string())?;
    let ledger = Ledger::open_existing(data_dir).map_err(|e| e.to_string())?;
    let (report, events) = ledger
        .read_verified_session(session)
        .map_err(|e| format!("cannot export unverified session `{session}`: {e}"))?;
    let mode = session_mode_from_events(&events);

    if format == ExportFormat::Notebook {
        if mode != Mode::Solve {
            return Err(format!(
                "notebook (`.ipynb`) export is only supported for Solve sessions (`{session}` is not Solve)"
            ));
        }
        let bytes = notebook::build_solve_notebook_bytes(
            &ledger,
            &report.head_hash,
            session,
            &events,
            notebook::NotebookOptions { include_all_runs },
        )?;
        return Ok((format!("{session}-notebook.ipynb"), bytes));
    }

    if mode == Mode::Solve && format == ExportFormat::Zip {
        let bytes = zip_solve::build_solve_reproducible_zip(
            data_dir,
            &ledger,
            session,
            &report.head_hash,
            &events,
            include_all_runs,
        )?;
        return Ok((format!("{session}-bundle.zip"), bytes));
    }

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
        ExportFormat::Notebook => unreachable!(),
    }
}

/// Output format for F15 mode write-ups (`00-overview.md` 2.13).
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
            "tex" | "latex" => Ok(WriteupFormat::Tex),
            "md" | "markdown" => Ok(WriteupFormat::Md),
            "bib" | "bibtex" => Ok(WriteupFormat::Bib),
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
}

/// Dispatch a verified session to its mode-specific F15 write-up adapter (zero model calls).
pub fn build_session_writeup(data_dir: &Path, session: &str) -> Result<writeup::Writeup, String> {
    validate_session_id(session).map_err(|e| e.to_string())?;
    let ledger =
        Ledger::open_existing(data_dir).map_err(|e| format!("failed to open ledger: {e}"))?;
    let (_report, events) = ledger
        .read_verified_session(session)
        .map_err(|e| format!("ledger verification failed for `{session}`: {e}"))?;
    match research_tools::tool_profile::session_mode_from_events(&events) {
        research_tools::paper_key::Mode::Solve => {
            writeup_solve::build_solve_writeup_from_verified(data_dir, session, &events)
        }
        research_tools::paper_key::Mode::Investigate => writeup_paper::build_writeup_from_verified(
            data_dir,
            session,
            &ledger,
            &events,
            writeup_paper::PaperWriteupOptions::default(),
        )
        .map(|b| b.writeup),
        research_tools::paper_key::Mode::Explore => {
            Err("Explore write-up adapter not built yet".to_string())
        }
        research_tools::paper_key::Mode::Research => Err(
            "Research sessions do not support write-up (use `export` instead)".to_string(),
        ),
    }
}

fn sanitize_bib_stem(raw: &str) -> String {
    let cleaned: String = raw
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim_start_matches('.').trim_matches('_');
    if trimmed.is_empty() {
        "references".to_string()
    } else {
        trimmed.chars().take(120).collect()
    }
}

/// Build and render a verified session write-up to bytes in `format` (`tex`, `md`, or `bib`).
pub fn writeup_session_bytes(
    data_dir: &Path,
    session: &str,
    format: WriteupFormat,
    opts: &writeup::WriteupOptions,
) -> Result<(String, Vec<u8>), String> {
    let doc = build_session_writeup(data_dir, session)?;
    match format {
        WriteupFormat::Tex => {
            let raw_stem = if opts.bib_stem.trim().is_empty() {
                format!("{session}-writeup")
            } else {
                opts.bib_stem.clone()
            };
            let tex_opts = writeup::WriteupOptions {
                include_inline_badges: opts.include_inline_badges,
                bib_stem: sanitize_bib_stem(&raw_stem),
            };
            let tex = writeup_tex::render_writeup_tex(&doc, &tex_opts)?;
            Ok((format!("{session}-writeup.tex"), tex.into_bytes()))
        }
        WriteupFormat::Md => {
            let md = writeup_md::render_writeup_markdown(&doc, opts)?;
            Ok((format!("{session}-writeup.md"), md.into_bytes()))
        }
        WriteupFormat::Bib => {
            doc.validate()?;
            let bib = writeup_bib::render_bibtex(&doc.bibliography);
            Ok((format!("{session}-writeup.bib"), bib.into_bytes()))
        }
    }
}

/// Build and write a verified session write-up to `out_path`.
///
/// For [`WriteupFormat::Tex`], also writes the paired sidecar `<stem>.bib` beside `out_path`
/// and points `\bibliography{<stem>}` at it.
pub fn writeup_session_to_path(
    data_dir: &Path,
    session: &str,
    format: WriteupFormat,
    opts: &writeup::WriteupOptions,
    out_path: &Path,
) -> Result<Vec<u8>, String> {
    if format == WriteupFormat::Tex
        && out_path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("bib"))
    {
        return Err(
            "TeX write-up output path must not end with `.bib` (collides with sidecar `.bib`)"
                .to_string(),
        );
    }
    let doc = build_session_writeup(data_dir, session)?;
    if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create write-up dir: {e}"))?;
    }
    let bytes = match format {
        WriteupFormat::Tex => {
            let raw_stem = out_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("references");
            let bib_stem = sanitize_bib_stem(raw_stem);
            let tex_opts = writeup::WriteupOptions {
                include_inline_badges: opts.include_inline_badges,
                bib_stem: bib_stem.clone(),
            };
            let tex_bytes = writeup_tex::render_writeup_tex(&doc, &tex_opts)?.into_bytes();
            let bib_text = writeup_bib::render_bibtex(&doc.bibliography);
            let bib_path = match out_path.parent() {
                Some(p) if !p.as_os_str().is_empty() => p.join(format!("{bib_stem}.bib")),
                _ => Path::new(&format!("{bib_stem}.bib")).to_path_buf(),
            };
            if bib_path == out_path {
                return Err("TeX write-up output path collides with its sidecar `.bib`".to_string());
            }
            fs::write(&bib_path, bib_text.as_bytes()).map_err(|e| {
                format!(
                    "failed to write sidecar `.bib` `{}`: {e}",
                    bib_path.display()
                )
            })?;
            tex_bytes
        }
        WriteupFormat::Md => writeup_md::render_writeup_markdown(&doc, opts)?.into_bytes(),
        WriteupFormat::Bib => {
            doc.validate()?;
            writeup_bib::render_bibtex(&doc.bibliography).into_bytes()
        }
    };
    fs::write(out_path, &bytes)
        .map_err(|e| format!("failed to write write-up `{}`: {e}", out_path.display()))?;
    Ok(bytes)
}
