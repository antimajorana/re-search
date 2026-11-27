//! Anchor re-verification, authors'-claim emission, and context-card emission for the F15
//! Investigate write-up adapter (`writeup_paper.rs`).
//!
//! Verifies every `VerifiedMapAnchor` via F4 `quote_core::verify_quote_ref` BEFORE emitting
//! any grounded claim or card prose, drops ungrounded claims/cards, and refuses to misattribute
//! external anchors to the primary paper.

use crate::writeup::{CheckTableRow, HowChecked, ProseBasis, QuoteLocator, RejectedWriteupQuote, WriteupBlock, WriteupQuote};
use crate::writeup_paper_nodes::BuildCtx;
use research_tools::map_context_defs::{ContextCardOrigin, ContextCardRecord};
use research_tools::map_defs::{AuthorsClaimRecord, VerifiedMapAnchor};
use research_tools::quote_core::{clip_quote, quote_sha256, verify_quote, verify_quote_ref, QuoteFault, QuoteRef};
use research_tools::status_badge::{
    authors_claim_display_text, detect_authors_claim_cues, format_cue_line, format_heading_cites,
};

const MAX_VERIFY_SPAN_LINES: usize = 64;

impl<'a> BuildCtx<'a> {
    pub(crate) fn emit_authors_claim(
        &mut self,
        section_title: &str,
        tag: &str,
        head_cites: &[String],
        claim: &AuthorsClaimRecord,
        blocks: &mut Vec<WriteupBlock>,
    ) {
        let mut verified_items = Vec::new();
        for (idx, a) in claim.anchors.iter().enumerate() {
            if let Some(item) =
                self.verify_anchor_quote(section_title, &format!("{tag}-authors-q{}", idx + 1), a, None)
            {
                verified_items.push(item);
            }
        }
        if !claim.anchors.is_empty() && verified_items.is_empty() {
            self.push_row(
                format!("{tag}-authors"),
                "Authors' claim omitted because anchors could not be matched.",
                HowChecked::CodeSkeleton,
                vec![tag.to_string()],
                "authors' claim omitted: anchor could not be matched",
            );
            return;
        }
        let quote_refs: Vec<&str> = verified_items.iter().map(|(q, _)| q.text.as_str()).collect();
        let cues = detect_authors_claim_cues(&quote_refs);
        let mut parts = vec![authors_claim_display_text(claim.label).to_string()];
        if let Some(rel) = claim.relation_note.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            parts.push(rel.to_string());
        }
        if !claim.attributed_to.is_empty() {
            let mapped: Vec<String> = claim
                .attributed_to
                .iter()
                .map(|k| self.cite_key_map.get(k).cloned().unwrap_or_else(|| k.clone()))
                .collect();
            parts.push(format!("refs: [{}]", mapped.join(", ")));
        }
        if let Some(hc) = format_heading_cites(head_cites) {
            parts.push(hc);
        }
        parts.push(format_cue_line(&cues));
        blocks.push(WriteupBlock::Statement {
            id: format!("{tag}-authors"),
            text: parts.join(" — "),
            how_checked: HowChecked::ModelStatement,
            own_tier: None,
            rests_on: vec![tag.to_string()],
            note: Some("authors' claim (model)".to_string()),
        });
        for (q, row) in verified_items {
            blocks.push(WriteupBlock::Quote(Box::new(q)));
            self.push_row(row.id, row.statement, row.how_checked, row.rests_on, row.note.unwrap_or_default());
        }
    }

    pub(crate) fn emit_context_card(
        &mut self,
        section_title: &str,
        tag: &str,
        idx: usize,
        card: &ContextCardRecord,
        blocks: &mut Vec<WriteupBlock>,
    ) {
        let card_id = format!("{tag}-ctx-{idx}");
        let ext_bib = (card.origin == ContextCardOrigin::External).then_some(card.bib_key.as_deref()).flatten();
        let mut verified_anchors = Vec::new();
        for (aidx, anchor) in card.anchors.iter().enumerate() {
            if let Some(item) =
                self.verify_anchor_quote(section_title, &format!("{card_id}-q{}", aidx + 1), anchor, ext_bib)
            {
                verified_anchors.push(item);
            }
        }
        if card.origin != ContextCardOrigin::ModelBackground && verified_anchors.is_empty() {
            self.push_row(
                &card_id,
                "Context card omitted because anchors could not be matched.",
                HowChecked::CodeSkeleton,
                vec![tag.to_string()],
                "context card omitted: anchor could not be matched",
            );
            return;
        }
        let origin_label = match card.origin {
            ContextCardOrigin::Paper => "From this paper",
            ContextCardOrigin::External => "Outside this paper",
            ContextCardOrigin::ModelBackground => "Model background, not checked",
        };
        let basis = if card.origin == ContextCardOrigin::ModelBackground {
            ProseBasis::Model
        } else {
            ProseBasis::App
        };
        blocks.push(WriteupBlock::Prose {
            text: format!(
                "Context ({}, {origin_label}): {} — {}",
                card.card_type.as_str(),
                card.title.trim(),
                card.body.trim()
            ),
            basis,
        });
        self.push_row(
            &card_id,
            format!("Context card: {}", card.title.trim()),
            HowChecked::ModelStatement,
            vec![tag.to_string()],
            origin_label,
        );
        for (q, row) in verified_anchors {
            blocks.push(WriteupBlock::Quote(Box::new(q)));
            self.push_row(row.id, row.statement, row.how_checked, row.rests_on, row.note.unwrap_or_default());
        }
        for (hidx, hyp) in card.hypotheses.iter().enumerate() {
            let h_id = format!("{card_id}-hyp-{}", hidx + 1);
            if let Some((q, row)) = self.verify_anchor_quote(section_title, &h_id, &hyp.anchor, ext_bib) {
                blocks.push(WriteupBlock::Quote(Box::new(q)));
                self.push_row(row.id, row.statement, row.how_checked, row.rests_on, row.note.unwrap_or_default());
                blocks.push(WriteupBlock::Prose {
                    text: format!("Context hypothesis: {}", hyp.text.trim()),
                    basis: ProseBasis::Model,
                });
                self.push_row(
                    format!("{h_id}-text"),
                    format!("Context hypothesis {} for {card_id}", hidx + 1),
                    HowChecked::ModelStatement,
                    vec![h_id],
                    "model hypothesis",
                );
            }
        }
    }

    pub(crate) fn verify_anchor_quote(
        &mut self,
        section_title: &str,
        row_id: &str,
        anchor: &VerifiedMapAnchor,
        external_bib_key: Option<&str>,
    ) -> Option<(WriteupQuote, CheckTableRow)> {
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
                return None;
            }
        };
        let is_primary = anchor.source_id == self.skeleton.source_id;
        let (cite_key, paper_ref) = if is_primary {
            (self.primary_cite_key.to_string(), self.primary_ref.clone())
        } else {
            let Some(k) = external_bib_key else {
                let loc = format!("{}:{}-{}", anchor.path, anchor.start_line, anchor.end_line);
                self.record_unmatched_quote(section_title, row_id, &loc, QuoteFault {
                    code: "unattributed_external_source".into(),
                    message: "External anchor has no resolved bibliography key.".into(),
                    start_line: Some(anchor.start_line),
                    end_line: Some(anchor.end_line),
                    true_text: None,
                });
                return None;
            };
            let (Some(ck), Some(pr)) = (self.cite_key_map.get(k), self.paper_ref_map.get(k)) else {
                let loc = format!("{}:{}-{}", anchor.path, anchor.start_line, anchor.end_line);
                self.record_unmatched_quote(section_title, row_id, &loc, QuoteFault {
                    code: "unresolved_external_bib_key".into(),
                    message: format!("External bib key `{k}` is not in the resolved bibliography."),
                    start_line: Some(anchor.start_line),
                    end_line: Some(anchor.end_line),
                    true_text: None,
                });
                return None;
            };
            (ck.clone(), pr.clone())
        };
        let clipped = clip_quote(raw_quote);
        let mut final_ref = verified.quote_ref;
        if clipped.len() != raw_quote.len() {
            final_ref.quote_sha256 = quote_sha256(clipped);
        }
        let (locator, ocr, page_range) = if is_primary {
            self.pdf_index.locator_and_ocr(&final_ref.path, final_ref.start_line, final_ref.end_line)
        } else {
            (QuoteLocator::Lines { start_line: final_ref.start_line, end_line: final_ref.end_line }, false, None)
        };
        let loc_note = format_locator_note(&final_ref.path, &locator, page_range);
        let how = if ocr { HowChecked::MatchedQuoteOcr } else { HowChecked::MatchedQuoteFullText };
        let note = if ocr { "matched OCR quote" } else { "matched full-text quote" };
        Some((
            WriteupQuote { text: clipped.to_string(), cite_key, paper: paper_ref, quote_ref: final_ref, locator, ocr },
            CheckTableRow {
                id: row_id.to_string(),
                statement: format!("Matched anchor quote ({row_id})"),
                how_checked: how,
                own_tier: None,
                rests_on: vec![loc_note],
                note: Some(note.to_string()),
            },
        ))
    }

    #[allow(clippy::too_many_arguments, clippy::result_large_err)]
    pub(crate) fn make_verified_quote(
        &self,
        source_id: &str,
        path: &str,
        start_line: usize,
        end_line: usize,
        clipped_quote: &str,
        locator: QuoteLocator,
        ocr: bool,
    ) -> Result<WriteupQuote, QuoteFault> {
        let verified = verify_quote(
            self.sandbox, self.session_id, self.events, source_id, path, start_line, end_line, clipped_quote,
        )?;
        Ok(WriteupQuote {
            text: clipped_quote.to_string(),
            cite_key: self.primary_cite_key.to_string(),
            paper: self.primary_ref.clone(),
            quote_ref: verified.quote_ref,
            locator,
            ocr,
        })
    }

    pub(crate) fn record_unmatched_quote(
        &mut self,
        section_title: &str,
        row_id: &str,
        loc: &str,
        fault: QuoteFault,
    ) {
        self.rejected_quotes.push(RejectedWriteupQuote {
            section_title: section_title.to_string(),
            cite_key: self.primary_cite_key.to_string(),
            fault,
        });
        self.push_row(
            row_id,
            "Quote could not be matched and was omitted.",
            HowChecked::CodeSkeleton,
            vec![loc.to_string()],
            "anchor could not be matched and was omitted",
        );
    }
}

pub(crate) fn narrowed_probe_end(start_line: usize, end_line: usize, probe: &str) -> usize {
    let probe_lines = probe.lines().count().max(1);
    end_line
        .min(start_line.saturating_add(probe_lines))
        .min(start_line.saturating_add(MAX_VERIFY_SPAN_LINES - 1))
        .max(start_line)
}

pub(crate) fn format_locator_note(
    path: &str,
    locator: &QuoteLocator,
    page_range: Option<(usize, usize)>,
) -> String {
    match page_range {
        Some((p1, p2)) if p1 != p2 => format!("{path}, pp. {p1}-{p2} ({})", locator.display_text()),
        Some(_) => format!("{path}, {}", locator.display_text()),
        None => format!("{path}: {}", locator.display_text()),
    }
}
