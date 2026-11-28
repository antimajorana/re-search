//! Section and node builders for the F15 Investigate write-up adapter (`writeup_paper.rs`).
//!
//! Renders deterministic Outline, Main Results (honoring user pins over model roles,
//! licence-gated full vs 300-char clipped quotes, PDF physical pages, OCR non-math tags,
//! F16a overrides tagged `"corrected by you"`, and user notes), and non-removable check rows.

use crate::writeup::{
    writeup_wording, CheckTableRow, HowChecked, ProseBasis, RejectedWriteupQuote, WriteupBlock,
    WriteupSection,
};
use crate::writeup_paper::{kind_display, quality_word, PaperWriteupOptions};
use crate::writeup_paper_anchors::{format_locator_note, narrowed_probe_end};
use crate::writeup_paper_ext::{find_forbidden_investigate_word, ExternalSourceBib};
use crate::writeup_paper_meta::{
    extract_node_statement_span, neutralize_ocr_math_delimiters, ExtractedStatement,
    LicenceResolution, PdfPageIndex, ReaderNodeOverride, CORRECTED_BY_YOU_LABEL,
    FALLBACK_RESULTS_NOTE, PINNED_MAIN_RESULT_NOTE,
};
use ledger::Event;
use research_tools::map_fold::{FoldedMap, FoldedNodeReading, ReaderPin};
use research_tools::paper_key::PaperRef;
use research_tools::quote_core::{clip_quote, verify_quote, MAX_DISPLAY_QUOTE_CHARS};
use research_tools::sandbox::WorkspaceSandbox;
use research_tools::skeleton::{PaperSkeleton, SkNode, SkNodeKind};
use research_tools::status_badge::format_heading_cites;
use std::collections::{BTreeMap, BTreeSet};

const FALLBACK_NO_MAIN_RESULTS_NOTE: &str =
    "No main results were identified or pinned; listing deterministic theorem-like nodes from the skeleton.";

pub(crate) struct BuildCtx<'a> {
    pub sandbox: &'a WorkspaceSandbox,
    pub session_id: &'a str,
    pub events: &'a [Event],
    pub skeleton: &'a PaperSkeleton,
    pub folded: &'a FoldedMap,
    pub overrides: &'a BTreeMap<String, ReaderNodeOverride>,
    pub source_files: &'a BTreeMap<String, Vec<String>>,
    pub pdf_index: &'a PdfPageIndex,
    pub pdf_indices: &'a BTreeMap<String, PdfPageIndex>,
    pub licence: &'a LicenceResolution,
    pub primary_ref: &'a PaperRef,
    pub primary_cite_key: &'a str,
    pub cite_key_map: &'a BTreeMap<String, String>,
    pub paper_ref_map: &'a BTreeMap<String, PaperRef>,
    pub source_bib_map: &'a BTreeMap<String, ExternalSourceBib>,
    pub opts: PaperWriteupOptions,
    pub check_table: Vec<CheckTableRow>,
    pub rejected_quotes: Vec<RejectedWriteupQuote>,
}

impl<'a> BuildCtx<'a> {
    pub(crate) fn push_row(
        &mut self,
        id: impl Into<String>,
        statement: impl Into<String>,
        how_checked: HowChecked,
        rests_on: Vec<String>,
        note: impl Into<String>,
    ) {
        let id = id.into();
        if self.check_table.iter().any(|r| r.id == id) {
            return;
        }
        let n = note.into();
        self.check_table.push(CheckTableRow {
            id,
            statement: statement.into(),
            how_checked,
            own_tier: None,
            rests_on,
            note: (!n.is_empty()).then_some(n),
        });
    }

    pub fn build_outline_section(&mut self) -> WriteupSection {
        let mut blocks = Vec::new();
        let discarded_sections: Vec<&SkNode> = self
            .skeleton
            .nodes
            .iter()
            .filter(|n| n.kind == SkNodeKind::Section && self.is_not_a_node(n))
            .collect();
        for sec in discarded_sections {
            if let Some(tag) = sec.tag.as_deref() {
                if let Some(ov) = self.overrides.get(tag) {
                    self.record_override_row(tag, ov, sec, false);
                }
            }
        }
        let sections: Vec<&SkNode> = self
            .skeleton
            .nodes
            .iter()
            .filter(|n| n.kind == SkNodeKind::Section && !self.is_not_a_node(n))
            .collect();
        let theorem_count = self
            .skeleton
            .nodes
            .iter()
            .filter(|n| n.kind.is_theorem_like() && !self.is_not_a_node(n))
            .count();
        let eq_count = self
            .skeleton
            .nodes
            .iter()
            .filter(|n| n.kind == SkNodeKind::Equation && !self.is_not_a_node(n))
            .count();
        let total_minutes: f64 = if !self.skeleton.stats.is_empty() {
            self.skeleton.stats.iter().map(|s| s.read_minutes).sum()
        } else {
            self.skeleton.nodes.iter().map(|n| n.read_minutes).sum()
        };

        let lic_desc = match (&self.licence.raw, self.licence.open) {
            (Some(l), true) => format!("Licence: {l} ({}; full statements permitted).", self.licence.source),
            (Some(l), false) => format!(
                "Licence: {l} ({}; excerpts capped at {MAX_DISPLAY_QUOTE_CHARS} characters).",
                self.licence.source
            ),
            (None, _) => format!("Licence: not recorded (excerpts capped at {MAX_DISPLAY_QUOTE_CHARS} characters)."),
        };
        blocks.push(WriteupBlock::Prose {
            text: format!(
                "Deterministic outline from {}: {} section(s), {} theorem-like statement(s), {} labelled equation(s), estimated {:.1} reading minutes. {lic_desc}",
                self.skeleton.builder.as_str(),
                sections.len(),
                theorem_count,
                eq_count,
                total_minutes
            ),
            basis: ProseBasis::App,
        });

        let stats_by_tag: BTreeMap<&str, (u32, f64, u32)> = self
            .skeleton
            .stats
            .iter()
            .filter_map(|st| {
                let tag = st.section_tag.as_deref()?;
                let res = st.counts.iter().filter(|(k, _)| k.is_theorem_like()).map(|(_, &c)| c).sum();
                Some((tag, (st.words, st.read_minutes, res)))
            })
            .collect();
        let parent_map: BTreeMap<&str, Option<&str>> = self
            .skeleton
            .nodes
            .iter()
            .filter_map(|n| Some((n.tag.as_deref()?, n.parent.as_deref())))
            .collect();

        for sec in &sections {
            let tag = sec.tag.as_deref().unwrap_or("");
            let indent = "  ".repeat(section_depth(tag, &parent_map));
            let (words, mins, results) = stats_by_tag.get(tag).copied().unwrap_or((sec.words, sec.read_minutes, 0));
            let ov = self.overrides.get(tag);
            let label = self.node_heading(sec, ov);
            let ov_tag = if ov.is_some() { format!(" ({CORRECTED_BY_YOU_LABEL})") } else { String::new() };
            blocks.push(WriteupBlock::Prose {
                text: format!("{indent}- {label}{ov_tag} [{results} result(s), {words} words, {mins:.1} min]"),
                basis: ProseBasis::App,
            });
            if let Some(ov_rec) = ov {
                self.record_override_row(tag, ov_rec, sec, false);
            }
        }

        self.push_row(
            "outline",
            format!("Paper outline ({} section(s), {} theorem-like statement(s))", sections.len(), theorem_count),
            HowChecked::CodeSkeleton,
            vec![self.skeleton.source_id.clone()],
            format!("skeleton by code ({}, {})", self.skeleton.builder.as_str(), quality_word(self.skeleton.quality)),
        );
        WriteupSection { title: "Outline".to_string(), label: Some("sec:outline".to_string()), blocks }
    }

    pub fn build_main_results_section(&mut self) -> (WriteupSection, BTreeSet<String>) {
        let mut blocks = Vec::new();
        let mut included = BTreeSet::new();

        let mut selected_sk: Vec<&SkNode> = self
            .skeleton
            .nodes
            .iter()
            .filter(|n| {
                !self.is_not_a_node(n)
                    && n.tag
                        .as_deref()
                        .is_some_and(|t| self.folded.nodes.get(t).is_some_and(|r| r.effective_is_main_result))
            })
            .collect();
        let selected_model_tags: Vec<String> = self
            .folded
            .nodes
            .values()
            .filter(|r| r.is_model_node && r.effective_is_main_result)
            .map(|r| r.tag.clone())
            .collect();

        if selected_sk.is_empty() && selected_model_tags.is_empty() {
            let not_pinned_out = |n: &&SkNode| {
                !self.is_not_a_node(n)
                    && !n.tag.as_deref().is_some_and(|t| {
                        self.folded.nodes.get(t).is_some_and(|r| r.pin == Some(ReaderPin::NotMainResult))
                    })
            };
            selected_sk = self
                .skeleton
                .nodes
                .iter()
                .filter(|n| n.kind.is_theorem_like() && not_pinned_out(n))
                .collect();
            if selected_sk.is_empty() {
                selected_sk = self
                    .skeleton
                    .nodes
                    .iter()
                    .filter(|n| n.kind == SkNodeKind::Equation && not_pinned_out(n))
                    .collect();
            }
            if !selected_sk.is_empty() {
                let orient_ran = self.folded.tasks.iter().any(|t| t.task == "orient");
                let note = if orient_ran { FALLBACK_NO_MAIN_RESULTS_NOTE } else { FALLBACK_RESULTS_NOTE };
                blocks.push(WriteupBlock::Prose { text: note.to_string(), basis: ProseBasis::App });
            }
        }

        for node in selected_sk {
            let tag = node.tag.clone().unwrap_or_else(|| format!("node-{}", node.order));
            self.append_skeleton_node_blocks("Main Results", node, &tag, &mut included, &mut blocks);
        }
        for mtag in selected_model_tags {
            if !included.contains(&mtag) {
                if let Some(reading) = self.folded.nodes.get(&mtag) {
                    self.append_model_node_blocks("Main Results", reading, &mut included, &mut blocks);
                }
            }
        }
        if blocks.is_empty() {
            blocks.push(WriteupBlock::Prose {
                text: "No theorem-like statements or pinned results were recorded in this session.".to_string(),
                basis: ProseBasis::App,
            });
        }
        (
            WriteupSection { title: "Main Results".to_string(), label: Some("sec:main-results".to_string()), blocks },
            included,
        )
    }

    pub fn build_additional_notes_section(&mut self, included: &mut BTreeSet<String>) -> Option<WriteupSection> {
        let mut blocks = Vec::new();
        for node in &self.skeleton.nodes {
            let Some(tag) = node.tag.as_deref() else { continue };
            if included.contains(tag) {
                continue;
            }
            let reading = self.folded.nodes.get(tag);
            let ov = self.overrides.get(tag);
            if let Some(ov_rec) = ov.filter(|o| o.not_a_node) {
                self.record_override_row(tag, ov_rec, node, false);
                included.insert(tag.to_string());
                continue;
            }
            let has_reading = reading.is_some_and(|r| self.has_reading_content(r));
            let include_node = if node.kind == SkNodeKind::Section {
                has_reading
            } else {
                has_reading || ov.is_some()
            };
            if include_node {
                self.append_skeleton_node_blocks(
                    "Additional Annotated Nodes and Reader Notes",
                    node,
                    tag,
                    included,
                    &mut blocks,
                );
            }
        }
        let orphan_models: Vec<String> = self
            .folded
            .nodes
            .values()
            .filter(|r| r.is_model_node && !included.contains(&r.tag) && self.has_reading_content(r))
            .map(|r| r.tag.clone())
            .collect();
        for mtag in orphan_models {
            if !included.contains(&mtag) {
                if let Some(reading) = self.folded.nodes.get(&mtag) {
                    self.append_model_node_blocks(
                        "Additional Annotated Nodes and Reader Notes",
                        reading,
                        included,
                        &mut blocks,
                    );
                }
            }
        }
        (!blocks.is_empty()).then_some(WriteupSection {
            title: "Additional Annotated Nodes and Reader Notes".to_string(),
            label: Some("sec:additional-notes".to_string()),
            blocks,
        })
    }

    fn has_reading_content(&self, r: &FoldedNodeReading) -> bool {
        r.gist.current.is_some()
            || r.purpose.current.is_some()
            || r.authors_claim.current.is_some()
            || r.hypotheses.current.is_some()
            || r.anchors.current.is_some()
            || !r.context_cards.is_empty()
            || (self.opts.include_notes && !r.notes.is_empty())
    }

    fn append_skeleton_node_blocks(
        &mut self,
        section_title: &str,
        node: &SkNode,
        tag: &str,
        included: &mut BTreeSet<String>,
        blocks: &mut Vec<WriteupBlock>,
    ) {
        if !included.insert(tag.to_string()) {
            return;
        }
        let reading = self.folded.nodes.get(tag);
        let ov = self.overrides.get(tag);
        if let Some(ov_rec) = ov.filter(|o| o.not_a_node) {
            self.record_override_row(tag, ov_rec, node, false);
            return;
        }

        let heading = self.node_heading(node, ov);
        let mut tags = Vec::new();
        if reading.is_some_and(|r| r.pin == Some(ReaderPin::MainResult)) {
            tags.push(PINNED_MAIN_RESULT_NOTE);
        }
        if ov.is_some() {
            tags.push(CORRECTED_BY_YOU_LABEL);
        }
        let header = if tags.is_empty() {
            format!("{heading} [{tag}]")
        } else {
            format!("{heading} [{tag}] ({})", tags.join("; "))
        };
        blocks.push(WriteupBlock::Prose { text: header, basis: ProseBasis::App });

        if !node.head_cites.is_empty() && reading.and_then(|r| r.authors_claim.current.as_ref()).is_none() {
            let mapped: Vec<String> = node
                .head_cites
                .iter()
                .map(|k| self.cite_key_map.get(k).cloned().unwrap_or_else(|| k.clone()))
                .collect();
            if let Some(hc) = format_heading_cites(&mapped) {
                blocks.push(WriteupBlock::Prose { text: format!("Heading citation: {hc}."), basis: ProseBasis::App });
            }
        }

        if node.kind != SkNodeKind::Section {
            let extracted = extract_node_statement_span(node, ov.and_then(|o| o.end_line), self.source_files);
            if let Some(ov_rec) = ov {
                self.record_override_row(tag, ov_rec, node, extracted.as_ref().is_some_and(|e| e.end_line_override_applied));
            }
            if let Some(ex) = extracted {
                self.emit_verified_node_statement(section_title, tag, &ex, blocks);
            }
        }
        if let Some(r) = reading {
            self.append_reading_blocks(section_title, tag, &node.head_cites, r, included, blocks);
        }
    }

    fn emit_verified_node_statement(
        &mut self,
        section_title: &str,
        tag: &str,
        ex: &ExtractedStatement,
        blocks: &mut Vec<WriteupBlock>,
    ) {
        let (locator, ocr, page_range) = self.pdf_index.locator_and_ocr(&ex.path, ex.start_line, ex.end_line);
        let loc_note = format_locator_note(&ex.path, &locator, page_range);
        let full_chars = ex.full_text.chars().count();
        let how = if ocr { HowChecked::MatchedQuoteOcr } else { HowChecked::MatchedQuoteFullText };

        if self.licence.open && full_chars > MAX_DISPLAY_QUOTE_CHARS {
            let probe = clip_quote(&ex.primary_text);
            let probe_end = narrowed_probe_end(ex.start_line, ex.end_line, probe);
            match verify_quote(
                self.sandbox, self.session_id, self.events, &self.skeleton.source_id, &ex.path, ex.start_line, probe_end, probe,
            ) {
                Ok(_) => {
                    let lic_name = self.licence.raw.as_deref().unwrap_or("open licence");
                    let ocr_tag = if ocr { format!(" {}", writeup_wording::OCR_TAG) } else { String::new() };
                    let stmt_text = if ocr { neutralize_ocr_math_delimiters(&ex.full_text) } else { ex.full_text.clone() };
                    blocks.push(WriteupBlock::Statement {
                        id: format!("{tag}-statement"),
                        text: stmt_text,
                        how_checked: how,
                        own_tier: None,
                        rests_on: vec![format!("{}:{}-{}", ex.path, ex.start_line, ex.end_line)],
                        note: Some(format!("{loc_note}; full statement ({full_chars} chars) permitted by {lic_name}{ocr_tag}")),
                    });
                    blocks.push(WriteupBlock::Prose {
                        text: format!("Source: [{}] ({loc_note}){ocr_tag} — full statement permitted by {lic_name}.", self.primary_cite_key),
                        basis: ProseBasis::App,
                    });
                }
                Err(fault) => self.record_unmatched_quote(section_title, &format!("{tag}-statement"), &loc_note, fault),
            }
            return;
        }

        let clipped = clip_quote(&ex.primary_text);
        let was_capped = full_chars > MAX_DISPLAY_QUOTE_CHARS;
        let verify_end = narrowed_probe_end(ex.start_line, ex.end_line, clipped);
        match self.make_verified_quote(&self.skeleton.source_id, &ex.path, ex.start_line, verify_end, clipped, locator, ocr) {
            Ok(vq) => {
                blocks.push(WriteupBlock::Quote(Box::new(vq)));
                let cap_note = if was_capped {
                    format!("capped at {MAX_DISPLAY_QUOTE_CHARS} chars (licence: {})", self.licence.raw.as_deref().unwrap_or("not recorded"))
                } else {
                    format!("within {MAX_DISPLAY_QUOTE_CHARS}-char limit")
                };
                self.push_row(format!("{tag}-statement"), format!("Verbatim statement for {tag}"), how, vec![loc_note], cap_note);
            }
            Err(fault) => self.record_unmatched_quote(section_title, &format!("{tag}-statement"), &loc_note, fault),
        }
    }

    fn append_reading_blocks(
        &mut self,
        section_title: &str,
        tag: &str,
        head_cites: &[String],
        reading: &FoldedNodeReading,
        included: &mut BTreeSet<String>,
        blocks: &mut Vec<WriteupBlock>,
    ) {
        if let Some(gist) = &reading.gist.current {
            let text = gist.value.trim();
            if find_forbidden_investigate_word(text, false).is_some() {
                self.push_row(format!("{tag}-gist"), format!("Gist for {tag} omitted"), HowChecked::ModelStatement, vec![tag.to_string()], "model gist omitted: disallowed claim wording");
            } else {
                blocks.push(WriteupBlock::Prose { text: format!("Gist: {text}"), basis: ProseBasis::Model });
                let note = if gist.stale { "model gist (stale)" } else { "model gist" };
                self.push_row(format!("{tag}-gist"), format!("Gist for {tag}"), HowChecked::ModelStatement, vec![tag.to_string()], note);
            }
        }
        if let Some(purpose) = &reading.purpose.current {
            let text = purpose.value.trim();
            if find_forbidden_investigate_word(text, false).is_some() {
                self.push_row(format!("{tag}-purpose"), format!("Purpose for {tag} omitted"), HowChecked::ModelStatement, vec![tag.to_string()], "model purpose omitted: disallowed claim wording");
            } else {
                blocks.push(WriteupBlock::Prose { text: format!("Purpose: {text}"), basis: ProseBasis::Model });
                self.push_row(format!("{tag}-purpose"), format!("Purpose for {tag}"), HowChecked::ModelStatement, vec![tag.to_string()], "model purpose");
            }
        }
        if let Some(anchors) = &reading.anchors.current {
            for (idx, a) in anchors.value.iter().enumerate() {
                if let Some((q, row)) = self.verify_anchor_quote(section_title, &format!("{tag}-q{}", idx + 1), a, None) {
                    blocks.push(WriteupBlock::Quote(Box::new(q)));
                    self.push_row(row.id, row.statement, row.how_checked, row.rests_on, row.note.unwrap_or_default());
                }
            }
        }
        if let Some(ac_val) = &reading.authors_claim.current {
            self.emit_authors_claim(section_title, tag, head_cites, &ac_val.value, blocks);
        }
        if let Some(hyps) = &reading.hypotheses.current {
            for (idx, hyp) in hyps.value.iter().enumerate() {
                let q_id = format!("{tag}-hyp-{}", idx + 1);
                if let Some((q, row)) = self.verify_anchor_quote(section_title, &q_id, &hyp.anchor, None) {
                    blocks.push(WriteupBlock::Quote(Box::new(q)));
                    self.push_row(row.id, row.statement, row.how_checked, row.rests_on, row.note.unwrap_or_default());
                    let htxt = hyp.text.trim();
                    if find_forbidden_investigate_word(htxt, false).is_some() {
                        self.push_row(format!("{q_id}-text"), format!("Hypothesis {} for {tag} omitted", idx + 1), HowChecked::ModelStatement, vec![q_id], "model hypothesis omitted: disallowed claim wording");
                    } else {
                        blocks.push(WriteupBlock::Prose { text: format!("Hypothesis: {htxt}"), basis: ProseBasis::Model });
                        self.push_row(format!("{q_id}-text"), format!("Hypothesis {} for {tag}", idx + 1), HowChecked::ModelStatement, vec![q_id], "model hypothesis");
                    }
                }
            }
        }
        for child_tag in &reading.children {
            if !included.contains(child_tag) {
                if let Some(child) = self.folded.nodes.get(child_tag) {
                    self.append_model_node_blocks(section_title, child, included, blocks);
                }
            }
        }
        for (idx, card_val) in reading.context_cards.iter().enumerate() {
            self.emit_context_card(section_title, tag, idx + 1, &card_val.value, blocks);
        }
        if reading.pin == Some(ReaderPin::MainResult) {
            self.push_row(format!("{tag}-pin"), format!("Reader pin on {tag}"), HowChecked::UserStatement, vec![tag.to_string()], PINNED_MAIN_RESULT_NOTE);
        }
        if self.opts.include_notes {
            for note in &reading.notes {
                blocks.push(WriteupBlock::Prose { text: format!("Your note: {}", note.text.trim()), basis: ProseBasis::User });
                self.push_row(format!("{tag}-note-{}", note.seq), format!("Reader note on {tag}"), HowChecked::UserStatement, vec![tag.to_string()], "user note");
            }
        }
    }

    fn append_model_node_blocks(
        &mut self,
        section_title: &str,
        node: &FoldedNodeReading,
        included: &mut BTreeSet<String>,
        blocks: &mut Vec<WriteupBlock>,
    ) {
        if !included.insert(node.tag.clone()) {
            return;
        }
        let kind = node.model_kind.as_deref().unwrap_or("step");
        blocks.push(WriteupBlock::Prose {
            text: format!("Explanation step ({kind}) [{}]", node.tag),
            basis: ProseBasis::Model,
        });
        self.append_reading_blocks(section_title, &node.tag, &[], node, included, blocks);
    }

    fn is_not_a_node(&self, node: &SkNode) -> bool {
        node.tag.as_deref().and_then(|t| self.overrides.get(t)).is_some_and(|o| o.not_a_node)
    }

    fn node_heading(&self, node: &SkNode, ov: Option<&ReaderNodeOverride>) -> String {
        let kind_str = ov.and_then(|o| o.kind.as_deref()).unwrap_or_else(|| kind_display(node.kind));
        let number = ov.and_then(|o| o.number.as_deref()).or(node.number.as_deref());
        let title = node.title.as_deref().map(str::trim).filter(|s| !s.is_empty());
        match (number, title) {
            (Some(num), Some(t)) => format!("{kind_str} {num} ({t})"),
            (Some(num), None) => format!("{kind_str} {num}"),
            (None, Some(t)) => format!("{kind_str}: {t}"),
            (None, None) => kind_str.to_string(),
        }
    }

    fn record_override_row(&mut self, tag: &str, ov: &ReaderNodeOverride, node: &SkNode, end_line_applied: bool) {
        let mut details = vec![CORRECTED_BY_YOU_LABEL.to_string()];
        if let Some(k) = &ov.kind { details.push(format!("kind: {k}")); }
        if let Some(num) = &ov.number { details.push(format!("number: {num}")); }
        if let Some(el) = ov.end_line.filter(|_| end_line_applied) { details.push(format!("end line: {el}")); }
        if ov.not_a_node { details.push("marked not a node".to_string()); }
        if self.opts.include_notes {
            if let Some(n) = &ov.note { details.push(n.clone()); }
        }
        let loc = node.spans.first().map(|s| format!("{}:{}-{}", s.path, s.start_line, s.end_line)).unwrap_or_else(|| tag.to_string());
        self.push_row(format!("{tag}-override"), format!("Reader override on {tag}"), HowChecked::UserStatement, vec![loc], details.join("; "));
    }
}

fn section_depth(tag: &str, parent_map: &BTreeMap<&str, Option<&str>>) -> usize {
    let (mut depth, mut cur, mut seen) = (0usize, parent_map.get(tag).copied().flatten(), BTreeSet::new());
    while let Some(p) = cur {
        if !seen.insert(p) || depth >= 6 { break; }
        depth += 1;
        cur = parent_map.get(p).copied().flatten();
    }
    depth
}
