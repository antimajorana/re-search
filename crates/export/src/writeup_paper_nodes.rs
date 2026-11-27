//! Section and node builders for the F15 Investigate write-up adapter (`writeup_paper.rs`).
//!
//! Renders deterministic Outline, Main Results (honoring user pins over model roles,
//! licence-gated full vs 300-char clipped quotes, PDF physical pages, OCR non-math tags,
//! authors'-claim half-badges, hypotheses, open-node children, V12 context cards, F16a
//! overrides tagged `"corrected by you"`, and user notes), and non-removable check rows.

use crate::writeup::{
    writeup_wording, CheckTableRow, HowChecked, ProseBasis, QuoteLocator, RejectedWriteupQuote,
    WriteupBlock, WriteupQuote, WriteupSection,
};
use crate::writeup_paper::PaperWriteupOptions;
use crate::writeup_paper_meta::{
    extract_node_statement_span, kind_display, quality_word, LicenceResolution, PdfPageIndex,
    ReaderNodeOverride, CORRECTED_BY_YOU_LABEL, FALLBACK_RESULTS_NOTE, PINNED_MAIN_RESULT_NOTE,
};
use ledger::Event;
use research_tools::map_context_defs::ContextCardOrigin;
use research_tools::map_defs::VerifiedMapAnchor;
use research_tools::map_fold::{FoldedMap, FoldedNodeReading, ReaderPin};
use research_tools::paper_key::PaperRef;
use research_tools::quote_core::{
    clip_quote, quote_sha256, verify_quote, verify_quote_ref, QuoteFault, QuoteRef,
    MAX_DISPLAY_QUOTE_CHARS,
};
use research_tools::sandbox::WorkspaceSandbox;
use research_tools::skeleton::{PaperSkeleton, SkNode, SkNodeKind};
use research_tools::status_badge::status_badge;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct BuildCtx<'a> {
    pub sandbox: &'a WorkspaceSandbox,
    pub session_id: &'a str,
    pub events: &'a [Event],
    pub skeleton: &'a PaperSkeleton,
    pub folded: &'a FoldedMap,
    pub overrides: &'a BTreeMap<String, ReaderNodeOverride>,
    pub source_files: &'a BTreeMap<String, Vec<String>>,
    pub pdf_index: &'a PdfPageIndex,
    pub licence: &'a LicenceResolution,
    pub primary_ref: &'a PaperRef,
    pub primary_cite_key: &'a str,
    pub cite_key_map: &'a BTreeMap<String, String>,
    pub paper_ref_map: &'a BTreeMap<String, PaperRef>,
    pub opts: PaperWriteupOptions,
    pub check_table: Vec<CheckTableRow>,
    pub rejected_quotes: Vec<RejectedWriteupQuote>,
}

impl<'a> BuildCtx<'a> {
    fn push_row(
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
        let sections: Vec<&SkNode> = self.skeleton.nodes.iter()
            .filter(|n| n.kind == SkNodeKind::Section && !self.is_not_a_node(n))
            .collect();
        let theorem_count = self.skeleton.nodes.iter()
            .filter(|n| n.kind.is_theorem_like() && !self.is_not_a_node(n))
            .count();
        let eq_count = self.skeleton.nodes.iter()
            .filter(|n| n.kind == SkNodeKind::Equation && !self.is_not_a_node(n))
            .count();
        let total_minutes: f64 = if !self.skeleton.stats.is_empty() {
            self.skeleton.stats.iter().map(|s| s.read_minutes).sum()
        } else {
            self.skeleton.nodes.iter().map(|n| n.read_minutes).sum()
        };

        let lic_desc = match (&self.licence.raw, self.licence.open) {
            (Some(l), true) => format!("Licence: {l} ({}; full statements permitted).", self.licence.source),
            (Some(l), false) => format!("Licence: {l} ({}; excerpts capped at {MAX_DISPLAY_QUOTE_CHARS} characters).", self.licence.source),
            (None, _) => format!("Licence: not recorded (excerpts capped at {MAX_DISPLAY_QUOTE_CHARS} characters)."),
        };
        blocks.push(WriteupBlock::Prose {
            text: format!(
                "Deterministic outline from {}: {} section(s), {} theorem-like statement(s), {} labelled equation(s), estimated {:.1} reading minutes. {lic_desc}",
                self.skeleton.builder.as_str(), sections.len(), theorem_count, eq_count, total_minutes
            ),
            basis: ProseBasis::App,
        });

        let stats_by_tag: BTreeMap<&str, (u32, f64, u32)> = self.skeleton.stats.iter()
            .filter_map(|st| {
                let tag = st.section_tag.as_deref()?;
                let res = st.counts.iter().filter(|(k, _)| k.is_theorem_like()).map(|(_, &c)| c).sum();
                Some((tag, (st.words, st.read_minutes, res)))
            })
            .collect();
        let parent_map: BTreeMap<&str, Option<&str>> = self.skeleton.nodes.iter()
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
                self.record_override_row(tag, ov_rec, sec);
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

        let mut selected_sk: Vec<&SkNode> = self.skeleton.nodes.iter()
            .filter(|n| {
                !self.is_not_a_node(n)
                    && n.tag.as_deref().is_some_and(|t| self.folded.nodes.get(t).is_some_and(|r| r.effective_is_main_result))
            })
            .collect();
        let selected_model_tags: Vec<String> = self.folded.nodes.values()
            .filter(|r| r.is_model_node && r.effective_is_main_result)
            .map(|r| r.tag.clone())
            .collect();

        if selected_sk.is_empty() && selected_model_tags.is_empty() {
            selected_sk = self.skeleton.nodes.iter()
                .filter(|n| {
                    n.kind.is_theorem_like()
                        && !self.is_not_a_node(n)
                        && !n.tag.as_deref().is_some_and(|t| {
                            self.folded.nodes.get(t).is_some_and(|r| r.pin == Some(ReaderPin::NotMainResult))
                        })
                })
                .collect();
            if selected_sk.is_empty() {
                selected_sk = self.skeleton.nodes.iter()
                    .filter(|n| n.kind == SkNodeKind::Equation && !self.is_not_a_node(n))
                    .collect();
            }
            if !selected_sk.is_empty() {
                blocks.push(WriteupBlock::Prose { text: FALLBACK_RESULTS_NOTE.to_string(), basis: ProseBasis::App });
            }
        }

        for node in selected_sk {
            let tag = node.tag.clone().unwrap_or_else(|| format!("node-{}", node.order));
            included.insert(tag.clone());
            self.append_skeleton_node_blocks("Main Results", node, &tag, &mut blocks);
        }
        for mtag in selected_model_tags {
            included.insert(mtag.clone());
            if let Some(reading) = self.folded.nodes.get(&mtag) {
                self.append_model_node_blocks("Main Results", reading, &mut blocks);
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

    pub fn build_additional_notes_section(&mut self, included: &BTreeSet<String>) -> Option<WriteupSection> {
        let mut blocks = Vec::new();
        for node in &self.skeleton.nodes {
            let Some(tag) = node.tag.as_deref() else { continue };
            if included.contains(tag) || node.kind == SkNodeKind::Section {
                continue;
            }
            let reading = self.folded.nodes.get(tag);
            let ov = self.overrides.get(tag);
            let has_content = reading.is_some_and(|r| {
                r.gist.current.is_some()
                    || r.purpose.current.is_some()
                    || r.authors_claim.current.is_some()
                    || !r.context_cards.is_empty()
                    || (self.opts.include_notes && !r.notes.is_empty())
            });
            if has_content || ov.is_some() {
                self.append_skeleton_node_blocks("Additional Annotated Nodes and Reader Notes", node, tag, &mut blocks);
            }
        }
        (!blocks.is_empty()).then_some(WriteupSection {
            title: "Additional Annotated Nodes and Reader Notes".to_string(),
            label: Some("sec:additional-notes".to_string()),
            blocks,
        })
    }

    fn append_skeleton_node_blocks(&mut self, section_title: &str, node: &SkNode, tag: &str, blocks: &mut Vec<WriteupBlock>) {
        let reading = self.folded.nodes.get(tag);
        let ov = self.overrides.get(tag);
        let heading = self.node_heading(node, ov);
        let mut tags = Vec::new();
        if reading.is_some_and(|r| r.pin == Some(ReaderPin::MainResult)) { tags.push(PINNED_MAIN_RESULT_NOTE); }
        if ov.is_some() { tags.push(CORRECTED_BY_YOU_LABEL); }
        let header = if tags.is_empty() {
            format!("{heading} [{tag}]")
        } else {
            format!("{heading} [{tag}] ({})", tags.join("; "))
        };
        blocks.push(WriteupBlock::Prose { text: header, basis: ProseBasis::App });

        if let Some(ov_rec) = ov {
            self.record_override_row(tag, ov_rec, node);
            if ov_rec.not_a_node { return; }
        }
        if let Some((path, start, end, body)) = extract_node_statement_span(node, ov.and_then(|o| o.end_line), self.source_files) {
            self.emit_verified_node_statement(section_title, tag, &path, start, end, &body, blocks);
        }
        if let Some(r) = reading {
            self.append_reading_blocks(section_title, tag, &node.head_cites, r, blocks);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_verified_node_statement(
        &mut self,
        section_title: &str,
        tag: &str,
        path: &str,
        start_line: usize,
        end_line: usize,
        full_body: &str,
        blocks: &mut Vec<WriteupBlock>,
    ) {
        let (locator, ocr, page_range) = self.pdf_index.locator_and_ocr(path, start_line, end_line);
        let loc_note = format_locator_note(path, &locator, page_range);
        let full_chars = full_body.chars().count();
        let how = if ocr { HowChecked::MatchedQuoteOcr } else { HowChecked::MatchedQuoteFullText };

        if self.licence.open && full_chars > MAX_DISPLAY_QUOTE_CHARS {
            let probe = clip_quote(full_body);
            match verify_quote(self.sandbox, self.session_id, self.events, &self.skeleton.source_id, path, start_line, end_line, probe) {
                Ok(_) => {
                    let lic_name = self.licence.raw.as_deref().unwrap_or("open licence");
                    let ocr_tag = if ocr { format!(" {}", writeup_wording::OCR_TAG) } else { String::new() };
                    blocks.push(WriteupBlock::Statement {
                        id: format!("{tag}-statement"),
                        text: full_body.to_string(),
                        how_checked: how,
                        own_tier: None,
                        rests_on: vec![format!("{path}:{start_line}-{end_line}")],
                        note: Some(format!("{loc_note}; full statement ({full_chars} chars) permitted by {lic_name}{ocr_tag}")),
                    });
                }
                Err(fault) => self.record_unmatched_quote(section_title, &format!("{tag}-statement"), &loc_note, fault),
            }
            return;
        }

        let clipped = clip_quote(full_body);
        let was_capped = full_chars > MAX_DISPLAY_QUOTE_CHARS;
        match self.make_verified_quote(&self.skeleton.source_id, path, start_line, end_line, clipped, locator, ocr) {
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
        blocks: &mut Vec<WriteupBlock>,
    ) {
        if let Some(gist) = &reading.gist.current {
            blocks.push(WriteupBlock::Prose { text: format!("Gist: {}", gist.value.trim()), basis: ProseBasis::Model });
            let note = if gist.stale { "model gist (stale)" } else { "model gist" };
            self.push_row(format!("{tag}-gist"), format!("Gist for {tag}"), HowChecked::ModelStatement, vec![tag.to_string()], note);
        }
        if let Some(purpose) = &reading.purpose.current {
            blocks.push(WriteupBlock::Prose { text: format!("Purpose: {}", purpose.value.trim()), basis: ProseBasis::Model });
            self.push_row(format!("{tag}-purpose"), format!("Purpose for {tag}"), HowChecked::ModelStatement, vec![tag.to_string()], "model purpose");
        }
        if let Some(ac_val) = &reading.authors_claim.current {
            let badge = status_badge(Some(&ac_val.value), head_cites);
            let mut parts = vec![badge.text.clone()];
            if let Some(rel) = &badge.relation_note { parts.push(rel.clone()); }
            if !badge.attributed_to.is_empty() {
                let mapped: Vec<String> = badge.attributed_to.iter()
                    .map(|k| self.cite_key_map.get(k).cloned().unwrap_or_else(|| k.clone()))
                    .collect();
                parts.push(format!("refs: [{}]", mapped.join(", ")));
            }
            if let Some(hc) = &badge.heading_cite_line { parts.push(hc.clone()); }
            parts.push(badge.cue_line.clone());
            blocks.push(WriteupBlock::Statement {
                id: format!("{tag}-authors"),
                text: parts.join(" — "),
                how_checked: HowChecked::ModelStatement,
                own_tier: None,
                rests_on: vec![tag.to_string()],
                note: Some("authors' claim (model)".to_string()),
            });
            for (idx, a) in ac_val.value.anchors.iter().enumerate() {
                self.emit_verified_anchor(section_title, &format!("{tag}-authors-q{}", idx + 1), a, None, blocks);
            }
        }
        if let Some(hyps) = &reading.hypotheses.current {
            for (idx, hyp) in hyps.value.iter().enumerate() {
                let q_id = format!("{tag}-hyp-{}", idx + 1);
                if self.emit_verified_anchor(section_title, &q_id, &hyp.anchor, None, blocks) {
                    blocks.push(WriteupBlock::Prose { text: format!("Hypothesis: {}", hyp.text.trim()), basis: ProseBasis::Model });
                    self.push_row(format!("{q_id}-text"), format!("Hypothesis {} for {tag}", idx + 1), HowChecked::ModelStatement, vec![q_id], "model hypothesis");
                }
            }
        }
        for child_tag in &reading.children {
            if let Some(child) = self.folded.nodes.get(child_tag) {
                self.append_model_node_blocks(section_title, child, blocks);
            }
        }
        for (idx, card_val) in reading.context_cards.iter().enumerate() {
            let card = &card_val.value;
            let origin_label = match card.origin {
                ContextCardOrigin::Paper => "From this paper",
                ContextCardOrigin::External => "Outside this paper",
                ContextCardOrigin::ModelBackground => "Model background, not checked",
            };
            let card_id = format!("{tag}-ctx-{}", idx + 1);
            let basis = if card.origin == ContextCardOrigin::ModelBackground { ProseBasis::Model } else { ProseBasis::App };
            blocks.push(WriteupBlock::Prose {
                text: format!("Context ({}, {origin_label}): {} — {}", card.card_type.as_str(), card.title.trim(), card.body.trim()),
                basis,
            });
            self.push_row(&card_id, format!("Context card: {}", card.title.trim()), HowChecked::ModelStatement, vec![tag.to_string()], origin_label);
            let ext_bib = (card.origin == ContextCardOrigin::External)
                .then_some(card.bib_key.as_deref())
                .flatten();
            for (aidx, anchor) in card.anchors.iter().enumerate() {
                self.emit_verified_anchor(section_title, &format!("{card_id}-q{}", aidx + 1), anchor, ext_bib, blocks);
            }
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

    fn append_model_node_blocks(&mut self, section_title: &str, node: &FoldedNodeReading, blocks: &mut Vec<WriteupBlock>) {
        let kind = node.model_kind.as_deref().unwrap_or("step");
        blocks.push(WriteupBlock::Prose { text: format!("Explanation step ({kind}) [{}]", node.tag), basis: ProseBasis::Model });
        if let Some(anchors) = &node.anchors.current {
            for (idx, a) in anchors.value.iter().enumerate() {
                self.emit_verified_anchor(section_title, &format!("{}-q{}", node.tag, idx + 1), a, None, blocks);
            }
        }
        self.append_reading_blocks(section_title, &node.tag, &[], node, blocks);
    }

    fn emit_verified_anchor(
        &mut self,
        section_title: &str,
        row_id: &str,
        anchor: &VerifiedMapAnchor,
        external_bib_key: Option<&str>,
        blocks: &mut Vec<WriteupBlock>,
    ) -> bool {
        let raw_quote = anchor.quote.trim();
        let expected_ref = QuoteRef {
            source_id: anchor.source_id.clone(),
            path: anchor.path.clone(),
            start_line: anchor.start_line,
            end_line: anchor.end_line,
            file_sha256: anchor.file_sha256.clone(),
            quote_sha256: anchor.quote_sha256.clone(),
            evidence_seq: None,
        };
        let verified = match verify_quote_ref(self.sandbox, self.session_id, self.events, &expected_ref, raw_quote) {
            Ok(v) => v,
            Err(fault) => {
                let loc = format!("{}:{}-{}", anchor.path, anchor.start_line, anchor.end_line);
                self.record_unmatched_quote(section_title, row_id, &loc, fault);
                return false;
            }
        };
        let clipped = clip_quote(raw_quote);
        let mut final_ref = verified.quote_ref;
        if clipped.len() != raw_quote.len() {
            final_ref.quote_sha256 = quote_sha256(clipped);
        }
        let is_primary = anchor.source_id == self.skeleton.source_id;
        let (cite_key, paper_ref) = if !is_primary {
            let ck = external_bib_key.and_then(|k| self.cite_key_map.get(k)).map(String::as_str).unwrap_or(self.primary_cite_key);
            let pr = external_bib_key.and_then(|k| self.paper_ref_map.get(k)).unwrap_or(self.primary_ref);
            (ck.to_string(), pr.clone())
        } else {
            (self.primary_cite_key.to_string(), self.primary_ref.clone())
        };
        let (locator, ocr, page_range) = if is_primary {
            self.pdf_index.locator_and_ocr(&final_ref.path, final_ref.start_line, final_ref.end_line)
        } else {
            (QuoteLocator::Lines { start_line: final_ref.start_line, end_line: final_ref.end_line }, false, None)
        };
        let loc_note = format_locator_note(&final_ref.path, &locator, page_range);

        blocks.push(WriteupBlock::Quote(Box::new(WriteupQuote {
            text: clipped.to_string(),
            cite_key,
            paper: paper_ref,
            quote_ref: final_ref,
            locator,
            ocr,
        })));
        let how = if ocr { HowChecked::MatchedQuoteOcr } else { HowChecked::MatchedQuoteFullText };
        let note = if ocr { "matched OCR quote" } else { "matched full-text quote" };
        self.push_row(row_id, format!("Matched anchor quote ({row_id})"), how, vec![loc_note], note);
        true
    }

    #[allow(clippy::too_many_arguments, clippy::result_large_err)]
    fn make_verified_quote(
        &self,
        source_id: &str,
        path: &str,
        start_line: usize,
        end_line: usize,
        clipped_quote: &str,
        locator: QuoteLocator,
        ocr: bool,
    ) -> Result<WriteupQuote, QuoteFault> {
        let verified = verify_quote(self.sandbox, self.session_id, self.events, source_id, path, start_line, end_line, clipped_quote)?;
        Ok(WriteupQuote {
            text: clipped_quote.to_string(),
            cite_key: self.primary_cite_key.to_string(),
            paper: self.primary_ref.clone(),
            quote_ref: verified.quote_ref,
            locator,
            ocr,
        })
    }

    fn record_unmatched_quote(&mut self, section_title: &str, row_id: &str, loc: &str, fault: QuoteFault) {
        self.rejected_quotes.push(RejectedWriteupQuote {
            section_title: section_title.to_string(),
            cite_key: self.primary_cite_key.to_string(),
            fault,
        });
        self.push_row(row_id, "Quote could not be matched and was omitted.", HowChecked::CodeSkeleton, vec![loc.to_string()], "anchor could not be matched and was omitted");
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

    fn record_override_row(&mut self, tag: &str, ov: &ReaderNodeOverride, node: &SkNode) {
        let mut details = vec![CORRECTED_BY_YOU_LABEL.to_string()];
        if let Some(k) = &ov.kind { details.push(format!("kind: {k}")); }
        if let Some(num) = &ov.number { details.push(format!("number: {num}")); }
        if let Some(el) = ov.end_line { details.push(format!("end line: {el}")); }
        if ov.not_a_node { details.push("marked not a node".to_string()); }
        if let Some(n) = &ov.note { details.push(n.clone()); }
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

fn format_locator_note(path: &str, locator: &QuoteLocator, page_range: Option<(usize, usize)>) -> String {
    match page_range {
        Some((p1, p2)) if p1 != p2 => format!("{path}, pp. {p1}-{p2} ({})", locator.display_text()),
        Some(_) => format!("{path}, {}", locator.display_text()),
        None => format!("{path}: {}", locator.display_text()),
    }
}
