//! F15 Investigate write-up adapter: deterministic reading notes from verified ledger events
//! (`00-overview.md` 2.13, 3.2 rows 19–24, 7.2 Set W, 7.3, 8.1, 8.3 Q3 & Q10; `investigate.md`
//! §3 L', §4.1–4.4, §6 rules 6.2/6.5/6.6/6.12/6.21/6.22, §12, D9).
//!
//! Pure, deterministic function over verified ledger events + optional existing library/lessons
//! stores. Zero model calls, zero network calls, zero explicit writes.

use crate::writeup::{RejectedWriteupQuote, Writeup, WriteupMode};
use crate::writeup_paper_meta::{
    collect_reader_overrides, load_and_verify_source_files, open_existing_library,
    resolve_bibliography, resolve_licence, resolve_primary_paper_ref, PdfPageIndex,
    EVENT_INVESTIGATION_PAPER, EVENT_SOURCE_INGESTED, SKELETON_ONLY_STATUS,
};
use crate::writeup_paper_nodes::BuildCtx;
use ledger::{sha256_hex, Event, Ledger};
use research_tools::investigation_markers::{
    active_investigation_slice, EVENT_INVESTIGATION_SKELETON,
};
use research_tools::library::PaperDetail;
use research_tools::map_fold::{fold_map, FoldedMap, FoldedTask};
use research_tools::paper_key::Mode;
use research_tools::sandbox::WorkspaceSandbox;
use research_tools::skeleton::{PaperSkeleton, SkNodeKind, SkeletonBuilder, SkeletonQuality};
use research_tools::tool_profile::session_mode_from_events;
use research_tools::validate_session_id;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;

pub use crate::writeup_paper_meta::is_permissive_licence;

/// Options controlling Investigate reading-notes generation (`00-overview.md` 2.13, `investigate.md` D9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaperWriteupOptions {
    /// Include user text notes attached to nodes (defaults to `true` for reading notes;
    /// pass `false` via `include_notes=0` when sharing publicly).
    pub include_notes: bool,
}

impl Default for PaperWriteupOptions {
    fn default() -> Self {
        Self { include_notes: true }
    }
}

/// Result of building an Investigate reading-notes [`Writeup`], including any Layer-2 anchors
/// rejected during F4 `quote_core` re-verification and the resolved licence policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaperWriteupBuild {
    pub writeup: Writeup,
    pub rejected_quotes: Vec<RejectedWriteupQuote>,
    pub licence: Option<String>,
    pub open_licence: bool,
}

/// Build an Investigate reading-notes [`Writeup`] with default options (`include_notes: true`).
pub fn build_writeup(data_dir: &Path, session_id: &str) -> Result<Writeup, String> {
    build_writeup_with_options(data_dir, session_id, PaperWriteupOptions::default()).map(|b| b.writeup)
}

/// Build an Investigate reading-notes [`Writeup`] from verified ledger events and read-only
/// library/lessons metadata. Fails closed on tampered ledger chains, missing/corrupt skeleton
/// blobs, or modified physical source files.
pub fn build_writeup_with_options(
    data_dir: &Path,
    session_id: &str,
    opts: PaperWriteupOptions,
) -> Result<PaperWriteupBuild, String> {
    validate_session_id(session_id).map_err(|e| e.to_string())?;
    let ledger = Ledger::open_existing(data_dir).map_err(|e| format!("failed to open ledger: {e}"))?;
    let (_report, events) = ledger
        .read_verified_session(session_id)
        .map_err(|e| format!("session `{session_id}` failed ledger verification: {e}"))?;
    build_writeup_from_verified(data_dir, session_id, &ledger, &events, opts)
}

/// Build an Investigate reading-notes [`Writeup`] reusing an already-verified `(ledger, events)`
/// pair (eliminates duplicate ledger verification when called from the shared dispatcher).
pub(crate) fn build_writeup_from_verified(
    data_dir: &Path,
    session_id: &str,
    ledger: &Ledger,
    events: &[Event],
    opts: PaperWriteupOptions,
) -> Result<PaperWriteupBuild, String> {
    if session_mode_from_events(events) != Mode::Investigate {
        return Err("Investigate reading-notes write-up requires an Investigate session.".into());
    }
    let (slice, _) = active_investigation_slice(events)?;

    let paper_ev = slice.iter().rev().find(|e| e.kind == EVENT_INVESTIGATION_PAPER);
    let pinned_source = paper_ev
        .and_then(|e| e.payload["source_id"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let skel_ev = slice
        .iter()
        .rev()
        .find(|e| {
            e.kind == EVENT_INVESTIGATION_SKELETON
                && pinned_source.is_none_or(|sid| e.payload["source_id"].as_str() == Some(sid))
        })
        .ok_or_else(|| "No pinned `investigation.skeleton` event found for this session.".to_string())?;

    let blob_hash = skel_ev.payload["blob"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "stored `investigation.skeleton` missing `blob` hash".to_string())?;
    let skel_bytes = ledger
        .get_blob(blob_hash)
        .map_err(|e| format!("failed to load skeleton blob `{blob_hash}`: {e}"))?;
    let actual_blob_hash = sha256_hex(&skel_bytes);
    if actual_blob_hash != blob_hash {
        return Err(format!(
            "skeleton blob hash mismatch: expected `{blob_hash}`, got `{actual_blob_hash}`"
        ));
    }
    let skeleton: PaperSkeleton = serde_json::from_slice(&skel_bytes)
        .map_err(|e| format!("failed to decode pinned skeleton blob: {e}"))?;
    if skel_ev.payload["source_id"].as_str().is_some_and(|s| s != skeleton.source_id) {
        return Err("pinned skeleton blob `source_id` disagrees with ledger event".into());
    }

    let receipt = find_source_receipt(events, &skeleton.source_id, skeleton.source_seq)
        .ok_or_else(|| {
            format!(
                "missing `source.ingested` receipt for pinned source `{}` (seq {})",
                skeleton.source_id, skeleton.source_seq
            )
        })?;

    let sandbox = WorkspaceSandbox::new(data_dir.to_path_buf(), None);
    let source_files = load_and_verify_source_files(&sandbox, session_id, receipt, &skeleton)?;
    let pdf_index = PdfPageIndex::from_receipt(receipt);

    let lib = open_existing_library(data_dir);
    let lib_detail = lib.as_ref().and_then(|l| l.get_paper(&skeleton.paper_key).ok().flatten());
    let licence_res = resolve_licence(paper_ev, receipt, lib_detail.as_ref());

    let folded = fold_map(&skeleton, events, false);
    let overrides = collect_reader_overrides(slice, &skeleton, data_dir);

    let title = resolve_paper_title(&skeleton, paper_ev, receipt, lib_detail.as_ref());
    let authors = resolve_paper_authors(paper_ev, receipt, lib_detail.as_ref());
    let year = resolve_paper_year(paper_ev, receipt, lib_detail.as_ref());
    let primary_ref = resolve_primary_paper_ref(&skeleton, lib_detail.as_ref());
    let referenced_bib_keys = collect_referenced_bib_keys(&skeleton, &folded);
    let bib = resolve_bibliography(
        lib.as_ref(),
        &primary_ref,
        &title,
        &authors,
        year,
        &skeleton,
        &referenced_bib_keys,
    )?;

    let status_line = build_status_line(&skeleton, &pdf_index, &folded.tasks);

    let mut ctx = BuildCtx {
        sandbox: &sandbox,
        session_id,
        events,
        skeleton: &skeleton,
        folded: &folded,
        overrides: &overrides,
        source_files: &source_files,
        pdf_index: &pdf_index,
        licence: &licence_res,
        primary_ref: &primary_ref,
        primary_cite_key: &bib.primary_cite_key,
        cite_key_map: &bib.cite_key_map,
        paper_ref_map: &bib.paper_ref_map,
        opts,
        check_table: Vec::new(),
        rejected_quotes: Vec::new(),
    };

    let mut sections = vec![ctx.build_outline_section()];
    let (main_sec, mut included_tags) = ctx.build_main_results_section();
    sections.push(main_sec);
    if let Some(extra_sec) = ctx.build_additional_notes_section(&mut included_tags) {
        sections.push(extra_sec);
    }

    let writeup = Writeup {
        mode: WriteupMode::Investigate,
        title,
        status_line,
        final_effective_tier: None,
        coverage_paragraph: None,
        prose_edited_by_model: false,
        sections,
        bibliography: bib.entries,
        check_table: ctx.check_table,
    };
    writeup.validate()?;

    Ok(PaperWriteupBuild {
        writeup,
        rejected_quotes: ctx.rejected_quotes,
        licence: licence_res.raw,
        open_licence: licence_res.open,
    })
}

fn find_source_receipt<'a>(events: &'a [Event], source_id: &str, source_seq: u64) -> Option<&'a Event> {
    if source_seq > 0 {
        return events.iter().rev().find(|e| {
            e.kind == EVENT_SOURCE_INGESTED
                && e.seq == source_seq
                && e.payload["source_id"].as_str() == Some(source_id)
        });
    }
    events
        .iter()
        .rev()
        .find(|e| e.kind == EVENT_SOURCE_INGESTED && e.payload["source_id"].as_str() == Some(source_id))
}

fn resolve_paper_title(
    skeleton: &PaperSkeleton,
    paper_ev: Option<&Event>,
    receipt: &Event,
    lib_detail: Option<&PaperDetail>,
) -> String {
    paper_ev
        .and_then(|e| e.payload["title"].as_str())
        .or_else(|| receipt.payload["title"].as_str())
        .or_else(|| lib_detail.map(|d| d.paper.title.as_str()))
        .or_else(|| {
            skeleton
                .nodes
                .iter()
                .find(|n| n.kind == SkNodeKind::FrontMatter)
                .and_then(|n| n.title.as_deref())
        })
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("Reading Notes: {}", skeleton.paper_key))
}

fn resolve_paper_authors(
    paper_ev: Option<&Event>,
    receipt: &Event,
    lib_detail: Option<&PaperDetail>,
) -> Vec<String> {
    for ev in [paper_ev, Some(receipt)].into_iter().flatten() {
        if let Some(arr) = ev.payload.get("authors").and_then(Value::as_array) {
            let names: Vec<String> = arr
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            if !names.is_empty() {
                return names;
            }
        }
    }
    lib_detail.map(|d| d.paper.authors.clone()).unwrap_or_default()
}

fn resolve_paper_year(
    paper_ev: Option<&Event>,
    receipt: &Event,
    lib_detail: Option<&PaperDetail>,
) -> Option<i32> {
    [paper_ev, Some(receipt)]
        .into_iter()
        .flatten()
        .find_map(|e| e.payload.get("year").and_then(Value::as_i64).map(|y| y as i32))
        .or_else(|| lib_detail.and_then(|d| d.paper.year))
}

fn collect_referenced_bib_keys(skeleton: &PaperSkeleton, folded: &FoldedMap) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for node in &skeleton.nodes {
        for k in &node.head_cites {
            if !k.trim().is_empty() {
                keys.insert(k.trim().to_string());
            }
        }
    }
    for reading in folded.nodes.values() {
        if let Some(ac) = &reading.authors_claim.current {
            for k in &ac.value.attributed_to {
                if !k.trim().is_empty() {
                    keys.insert(k.trim().to_string());
                }
            }
        }
        for card in &reading.context_cards {
            if let Some(k) = card.value.bib_key.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                keys.insert(k.to_string());
            }
        }
    }
    keys
}

pub(crate) fn quality_word(q: SkeletonQuality) -> &'static str {
    match q {
        SkeletonQuality::Full => "full",
        SkeletonQuality::Partial => "partial",
        SkeletonQuality::Weak => "weak",
    }
}

/// Mandatory opening status line (`00-overview.md` 2.13 rule 1, 8.3 Q10, `investigate.md` 6.21).
/// Never contains `"verified"`, `"proved"`, `"new"`, or `"novel"`.
pub(crate) fn build_status_line(
    skeleton: &PaperSkeleton,
    pdf_index: &PdfPageIndex,
    tasks: &[FoldedTask],
) -> String {
    let paper_id = match skeleton.version.as_deref().filter(|s| !s.is_empty()) {
        Some(v) => format!("{} ({v})", skeleton.paper_key),
        None => skeleton.paper_key.to_string(),
    };
    let origin = match skeleton.builder {
        SkeletonBuilder::Tex1 => "Structure from TeX",
        SkeletonBuilder::Html1 => "Structure from arXiv HTML",
        SkeletonBuilder::Pdf1 => "Structure from PDF text",
    };
    let mut reason_text = skeleton
        .quality_reason
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_default();
    if skeleton.builder == SkeletonBuilder::Pdf1 && pdf_index.has_scanned_pages() {
        let lower = reason_text.to_ascii_lowercase();
        if !lower.contains("scan") && !lower.contains("ocr") {
            if reason_text.is_empty() {
                reason_text = "scanned pages (OCR)".to_string();
            } else {
                reason_text.push_str(" (scan)");
            }
        }
    }
    let reason = if reason_text.is_empty() {
        String::new()
    } else {
        format!(" — {reason_text}")
    };
    let structure = format!(
        "{origin} ({}, {}{reason})",
        skeleton.builder.as_str(),
        quality_word(skeleton.quality)
    );
    let task_summary = if tasks.is_empty() {
        SKELETON_ONLY_STATUS.to_string()
    } else {
        let mut parts = Vec::new();
        if let Some(orient) = tasks.iter().rev().find(|t| t.task == "orient") {
            parts.push(format!("Orient: {}", orient.run_state_label));
        }
        let open_count = tasks.iter().filter(|t| t.task == "open").count();
        if open_count > 0 {
            parts.push(format!("Open tasks: {open_count}"));
        }
        let ctx_count = tasks.iter().filter(|t| t.task == "context").count();
        if ctx_count > 0 {
            parts.push(format!("Context tasks: {ctx_count}"));
        }
        if parts.is_empty() { format!("Tasks recorded: {}", tasks.len()) } else { parts.join("; ") }
    };
    format!("{paper_id}. {structure}. {task_summary}.")
}

pub(crate) fn kind_display(kind: SkNodeKind) -> &'static str {
    match kind {
        SkNodeKind::FrontMatter => "Front matter",
        SkNodeKind::Section => "Section",
        SkNodeKind::Definition => "Definition",
        SkNodeKind::Assumption => "Assumption",
        SkNodeKind::Theorem => "Theorem",
        SkNodeKind::Lemma => "Lemma",
        SkNodeKind::Proposition => "Proposition",
        SkNodeKind::Corollary => "Corollary",
        SkNodeKind::Conjecture => "Conjecture",
        SkNodeKind::Claim => "Claim",
        SkNodeKind::Proof => "Proof",
        SkNodeKind::Example => "Example",
        SkNodeKind::Remark => "Remark",
        SkNodeKind::Equation => "Equation",
        SkNodeKind::Figure => "Figure",
        SkNodeKind::Table => "Table",
        SkNodeKind::Algorithm => "Algorithm",
        SkNodeKind::Bibliography => "Bibliography",
    }
}
