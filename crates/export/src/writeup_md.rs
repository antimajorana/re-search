//! Deterministic Markdown writer for F15 [`Writeup`] documents (`00-overview.md` 2.13).
//!
//! Uses the exact same [`crate::writeup::writeup_wording`] table as [`crate::writeup_tex`].
//! Zero model calls.

use crate::writeup::{
    filter_verified_quotes, writeup_wording, ProseBasis, RejectedWriteupQuote, Writeup,
    WriteupBlock, WriteupOptions,
};
use ledger::Event;
use research_tools::sandbox::WorkspaceSandbox;
use std::fmt::Write as _;

/// Render a validated [`Writeup`] to deterministic GitHub-Flavored Markdown.
///
/// The status line, optional coverage paragraph, and "How each statement was checked" table
/// are always emitted regardless of `opts.include_inline_badges`.
pub fn render_writeup_markdown(doc: &Writeup, opts: &WriteupOptions) -> Result<String, String> {
    doc.validate()?;
    let mut out = String::new();
    let check_rows = doc.derived_check_table();

    let _ = writeln!(out, "# {}\n", doc.title.trim());
    let _ = writeln!(
        out,
        "**{}:** {}\n",
        writeup_wording::STATUS_LABEL,
        doc.status_line.trim()
    );
    if doc.prose_edited_by_model {
        let _ = writeln!(out, "*{}*\n", writeup_wording::PROSE_EDITED_BY_MODEL);
    }

    for sec in &doc.sections {
        let _ = writeln!(out, "## {}\n", sec.title.trim());
        for blk in &sec.blocks {
            match blk {
                WriteupBlock::Prose { text, basis } => {
                    if *basis == ProseBasis::Model && opts.include_inline_badges {
                        let _ = writeln!(
                            out,
                            "{} *{}*\n",
                            text.trim(),
                            writeup_wording::MODEL_SUFFIX
                        );
                    } else {
                        let _ = writeln!(out, "{}\n", text.trim());
                    }
                }
                WriteupBlock::Formula { latex, handle, .. } => {
                    let _ = writeln!(out, "<!-- handle: {} -->", handle.trim());
                    let _ = writeln!(out, "$$\n{}\n$$\n", latex.trim());
                }
                WriteupBlock::Quote(q) => {
                    let loc = q.locator.display_text();
                    let ocr_suffix = if q.ocr {
                        format!(" {}", writeup_wording::OCR_TAG)
                    } else {
                        String::new()
                    };
                    let quoted = q.text.replace('\n', "\n> ");
                    let _ = writeln!(
                        out,
                        "> \"{quoted}\"\n>\n> -- [@{}, {loc}]{ocr_suffix}\n",
                        q.cite_key
                    );
                }
                WriteupBlock::Statement {
                    id,
                    text,
                    how_checked,
                    ..
                } => {
                    let badge = if opts.include_inline_badges {
                        format!(" [{}]", how_checked.label())
                    } else {
                        String::new()
                    };
                    let _ = writeln!(out, "**{id}**{badge}: {}\n", text.trim());
                }
            }
        }
    }

    // Mandatory Explore coverage paragraph ("Where I looked"), cannot be omitted when present.
    if let Some(cov) = doc
        .coverage_paragraph
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let _ = writeln!(out, "## {}\n\n{cov}\n", writeup_wording::COVERAGE_HEADING);
    }

    // Mandatory table "How each statement was checked" (cannot be omitted; `00-overview.md` 8.3 Q10).
    let _ = writeln!(out, "## {}\n", writeup_wording::CHECK_TABLE_HEADING);
    out.push_str("| Step / Claim | How checked | Rests on |\n");
    out.push_str("|---|---|---|\n");
    if check_rows.is_empty() {
        out.push_str("| - | No statements recorded. | - |\n");
    } else {
        for row in &check_rows {
            let rests = if row.rests_on.is_empty() {
                "-".to_string()
            } else {
                escape_md_cell(&row.rests_on.join(", "))
            };
            let detail = match row.note.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                Some(n) => format!("{rests} ({})", escape_md_cell(n)),
                None => rests,
            };
            let _ = writeln!(
                out,
                "| {} | {} | {} |",
                escape_md_cell(&row.id),
                escape_md_cell(&row.badge_text()),
                detail
            );
        }
    }
    out.push('\n');

    if !doc.bibliography.is_empty() {
        let _ = writeln!(out, "## {}\n", writeup_wording::REFERENCES_HEADING);
        for entry in &doc.bibliography {
            let authors = if entry.authors.is_empty() {
                "Unknown".to_string()
            } else {
                entry.authors.join(", ")
            };
            let year = entry
                .year
                .map(|y| format!(" ({y})"))
                .unwrap_or_default();
            let venue = entry
                .venue
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|v| format!(" *{v}*."))
                .unwrap_or_default();
            let _ = writeln!(
                out,
                "- [@{}] {authors}{year}. {}.{} `{}`",
                entry.cite_key,
                entry.title.trim(),
                venue,
                entry.paper.key
            );
        }
    }

    Ok(out)
}

/// Filter quotes through F4 `quote_core` first (dropping any unmatched quote) and render Markdown.
pub fn render_verified_writeup_markdown(
    doc: &Writeup,
    sandbox: &WorkspaceSandbox,
    session_id: &str,
    session_events: &[Event],
    opts: &WriteupOptions,
) -> Result<(String, Vec<RejectedWriteupQuote>), String> {
    let (cleaned, rejected) = filter_verified_quotes(doc, sandbox, session_id, session_events);
    let md = render_writeup_markdown(&cleaned, opts)?;
    Ok((md, rejected))
}

fn escape_md_cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\r', '\n'], " ")
}
