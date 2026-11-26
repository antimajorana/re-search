//! Bounded DOM stack walker for LaTeXML HTML5 documents (`html-1`, V18).

use super::html_bib::{HtmlCiteOccurrence, RawHtmlBibItem};
use super::html_classify::{
    classify_section_level, classify_structural_node, collapse_ws, is_heading_for_node,
};
use super::html_edges::{HtmlProofHint, HtmlRefOccurrence, HtmlTextLine};
use super::html_finalize::{capture_href_or_bib_span, finalize_element};
use super::html_lex::{HtmlToken, LineIndex};
use super::model::{SkConfidence, SkNode, SkNodeKind, Span, MAX_SKELETON_NODES};
use std::collections::BTreeMap;

pub(crate) const MAX_HTML_DEPTH: usize = 128;

#[derive(Debug, Clone)]
pub(crate) struct PendingEquation {
    pub(crate) html_id: Option<String>,
    pub(crate) label: Option<String>,
    pub(crate) parent_order: Option<usize>,
    pub(crate) start_line: usize,
    pub(crate) tag_text: String,
    pub(crate) body_text: String,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingBibItem {
    pub(crate) html_id: Option<String>,
    pub(crate) tag_text: String,
    pub(crate) raw_text: String,
    pub(crate) title_hint: Option<String>,
    pub(crate) in_title_span: usize,
    pub(crate) title_buf: String,
    pub(crate) link_urls: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ActiveCite {
    pub(crate) enc_idx: usize,
    pub(crate) target_bib_ids: Vec<String>,
    pub(crate) text_buf: String,
    pub(crate) site: Span,
    pub(crate) in_heading: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct OpenElement {
    pub(crate) tag_name: String,
    pub(crate) node_order: Option<usize>,
    pub(crate) sec_level: Option<u8>,
    pub(crate) heading_target: Option<usize>,
    pub(crate) is_tag_span: bool,
    pub(crate) is_cite: bool,
    pub(crate) is_bib_item: bool,
    pub(crate) is_eq_group: bool,
    pub(crate) eq_group_has_child_node: bool,
    pub(crate) eq_group_tag: String,
    pub(crate) eq_group_id: Option<String>,
    pub(crate) eq_group_label: Option<String>,
    pub(crate) eq_group_start_line: usize,
    pub(crate) eq_group_text: String,
    pub(crate) pending_eq: Option<PendingEquation>,
    pub(crate) ignore_text: bool,
    pub(crate) is_doc_title: bool,
}

pub(crate) struct HtmlWalker<'a> {
    pub(crate) path: &'a str,
    pub(crate) nodes: Vec<SkNode>,
    pub(crate) parent_orders: Vec<Option<usize>>,
    pub(crate) node_fp_texts: Vec<String>,
    pub(crate) node_plain_texts: Vec<String>,
    pub(crate) node_tag_texts: Vec<String>,
    pub(crate) node_title_texts: Vec<String>,
    pub(crate) node_has_heading: Vec<bool>,
    pub(crate) html_id_to_node_idx: BTreeMap<String, usize>,
    pub(crate) proof_hints: BTreeMap<usize, HtmlProofHint>,
    pub(crate) pending_adjacent_proof: BTreeMap<usize, usize>,
    pub(crate) refs: Vec<HtmlRefOccurrence>,
    pub(crate) cites: Vec<HtmlCiteOccurrence>,
    pub(crate) bib_items: Vec<RawHtmlBibItem>,
    pub(crate) prose_lines: Vec<HtmlTextLine>,
    pub(crate) quality_notes: Vec<String>,
    pub(crate) elem_stack: Vec<OpenElement>,
    pub(crate) active_cites: Vec<ActiveCite>,
    pub(crate) pending_bib: Option<PendingBibItem>,
    pub(crate) last_provable_by_parent: BTreeMap<Option<usize>, Option<usize>>,
    node_limit_hit: bool,
    depth_limit_hit: bool,
    pub(crate) document_title: Option<String>,
    pub(crate) doc_title_buf: String,
    in_html_title: bool,
    html_title_buf: String,
}

impl<'a> HtmlWalker<'a> {
    pub(crate) fn new(path: &'a str, _line_index: &'a LineIndex, warnings: Vec<String>) -> Self {
        Self {
            path,
            nodes: Vec::new(),
            parent_orders: Vec::new(),
            node_fp_texts: Vec::new(),
            node_plain_texts: Vec::new(),
            node_tag_texts: Vec::new(),
            node_title_texts: Vec::new(),
            node_has_heading: Vec::new(),
            html_id_to_node_idx: BTreeMap::new(),
            proof_hints: BTreeMap::new(),
            pending_adjacent_proof: BTreeMap::new(),
            refs: Vec::new(),
            cites: Vec::new(),
            bib_items: Vec::new(),
            prose_lines: Vec::new(),
            quality_notes: warnings,
            elem_stack: Vec::new(),
            active_cites: Vec::new(),
            pending_bib: None,
            last_provable_by_parent: BTreeMap::new(),
            node_limit_hit: false,
            depth_limit_hit: false,
            document_title: None,
            doc_title_buf: String::new(),
            in_html_title: false,
            html_title_buf: String::new(),
        }
    }

    pub(crate) fn current_node_order(&self) -> Option<usize> {
        self.elem_stack.iter().rev().find_map(|e| e.node_order)
    }

    pub(crate) fn current_heading_target(&self) -> Option<usize> {
        self.elem_stack.iter().rev().find_map(|e| e.heading_target)
    }

    pub(crate) fn in_tag_span(&self) -> bool {
        self.elem_stack.iter().any(|e| e.is_tag_span)
    }

    fn in_fig_or_table(&self, kind: SkNodeKind) -> bool {
        self.elem_stack
            .iter()
            .filter_map(|e| e.node_order)
            .any(|ord| self.nodes.get(ord).is_some_and(|n| n.kind == kind))
    }

    pub(crate) fn walk(&mut self, tokens: &[HtmlToken]) {
        for tok in tokens {
            match tok {
                HtmlToken::StartTag {
                    name,
                    attrs,
                    self_closing,
                    is_void,
                    start_line,
                    end_line,
                } => self.on_start_tag(name, attrs, *self_closing || *is_void, *start_line, *end_line),
                HtmlToken::EndTag { name, end_line } => self.on_end_tag(name, *end_line),
                HtmlToken::Text { decoded, start_line, end_line } => {
                    self.on_text(decoded, *start_line, *end_line)
                }
            }
        }
    }

    fn on_start_tag(
        &mut self,
        name: &str,
        attrs: &[(String, String)],
        self_closed: bool,
        start_line: usize,
        end_line: usize,
    ) {
        let classes: Vec<&str> = HtmlToken::attr(attrs, "class")
            .map(|c| c.split_whitespace().collect())
            .unwrap_or_default();
        let html_id = HtmlToken::attr(attrs, "id")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let tex_label = HtmlToken::attr(attrs, "data-tex-label")
            .or_else(|| HtmlToken::attr(attrs, "data-label"))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        capture_href_or_bib_span(self, attrs, &classes, start_line, end_line);

        if name == "math" {
            if let Some(alt) = HtmlToken::attr(attrs, "alttext").map(str::trim).filter(|s| !s.is_empty()) {
                self.append_decoded_text(alt, start_line, end_line);
                if !self_closed && self.elem_stack.len() < MAX_HTML_DEPTH {
                    self.elem_stack.push(plain_elem(name, true));
                }
                return;
            }
        }
        if matches!(name, "annotation" | "annotation-xml") {
            if !self_closed && self.elem_stack.len() < MAX_HTML_DEPTH {
                self.elem_stack.push(plain_elem(name, true));
            }
            return;
        }
        if name == "title" && !self_closed {
            self.in_html_title = true;
            self.html_title_buf.clear();
        }
        if self_closed {
            return;
        }
        if self.elem_stack.len() >= MAX_HTML_DEPTH {
            if !self.depth_limit_hit {
                self.depth_limit_hit = true;
                self.quality_notes.push(format!(
                    "HTML nesting depth exceeded {MAX_HTML_DEPTH} at line {start_line}; capped"
                ));
            }
            return;
        }

        let sec_level = classify_section_level(&classes);
        if let Some(lvl) = sec_level {
            self.auto_close_sections_at(lvl, start_line);
        }

        let mut elem = plain_elem(name, false);
        elem.is_tag_span = classes.iter().any(|c| c.starts_with("ltx_tag") || *c == "ltx_eqn_eqno");
        elem.is_doc_title = classes.contains(&"ltx_title_document");
        if elem.is_doc_title {
            self.doc_title_buf.clear();
        }

        if let Some(enc_idx) = self.current_node_order() {
            let is_heading = is_heading_for_node(self.nodes[enc_idx].kind, &classes)
                && !self.node_has_heading[enc_idx];
            if is_heading {
                self.node_has_heading[enc_idx] = true;
                elem.heading_target = Some(enc_idx);
            }
        }

        if classes.contains(&"ltx_cite") {
            if let Some(enc_idx) = self.current_node_order() {
                elem.is_cite = true;
                self.active_cites.push(ActiveCite {
                    enc_idx,
                    target_bib_ids: Vec::new(),
                    text_buf: String::new(),
                    site: Span::new(self.path, start_line, end_line),
                    in_heading: self.current_heading_target().is_some(),
                });
            }
        }

        if classes.contains(&"ltx_bibitem") {
            elem.is_bib_item = true;
            self.pending_bib = Some(PendingBibItem {
                html_id,
                tag_text: String::new(),
                raw_text: String::new(),
                title_hint: None,
                in_title_span: 0,
                title_buf: String::new(),
                link_urls: Vec::new(),
            });
            self.elem_stack.push(elem);
            return;
        }

        if classes.contains(&"ltx_equationgroup") {
            elem.is_eq_group = true;
            elem.eq_group_id = html_id;
            elem.eq_group_label = tex_label;
            elem.eq_group_start_line = start_line;
            self.elem_stack.push(elem);
            return;
        }

        if classes.contains(&"ltx_equation")
            && !self.elem_stack.iter().any(|e| e.pending_eq.is_some())
        {
            elem.pending_eq = Some(PendingEquation {
                html_id,
                label: tex_label,
                parent_order: self.current_node_order(),
                start_line,
                tag_text: String::new(),
                body_text: String::new(),
            });
            self.elem_stack.push(elem);
            return;
        }

        if let Some((kind, env_name, s_lvl)) = classify_structural_node(&classes) {
            if matches!(kind, SkNodeKind::Figure | SkNodeKind::Table) && self.in_fig_or_table(kind) {
                self.elem_stack.push(elem);
                return;
            }
            if kind.is_theorem_like() || kind == SkNodeKind::Proof {
                self.auto_close_unclosed_block_sibling(start_line);
            }
            let parent = self.current_node_order();
            if let Some(order) = self.push_node(kind, env_name, tex_label, parent, start_line) {
                if let Some(id) = html_id {
                    self.html_id_to_node_idx.insert(id, order);
                }
                if kind == SkNodeKind::FrontMatter {
                    self.nodes[order].title = Some("Abstract".to_string());
                } else if kind == SkNodeKind::Bibliography {
                    self.nodes[order].title = Some("References".to_string());
                }
                elem.node_order = Some(order);
                elem.sec_level = s_lvl;
            }
        }

        self.elem_stack.push(elem);
    }

    fn auto_close_sections_at(&mut self, min_level: u8, before_line: usize) {
        let Some(hit) = self
            .elem_stack
            .iter()
            .rposition(|e| e.sec_level.is_some_and(|l| l >= min_level))
        else {
            return;
        };
        self.quality_notes.push(format!(
            "auto-closed unclosed section element before line {before_line}"
        ));
        let close_line = before_line.saturating_sub(1).max(1);
        while self.elem_stack.len() > hit {
            if let Some(popped) = self.elem_stack.pop() {
                finalize_element(self, popped, close_line);
            }
        }
    }

    fn auto_close_unclosed_block_sibling(&mut self, before_line: usize) {
        let Some(hit) = self.elem_stack.iter().rposition(|e| {
            e.node_order.is_some_and(|ord| {
                let k = self.nodes[ord].kind;
                k.is_theorem_like() || k == SkNodeKind::Proof
            })
        }) else {
            return;
        };
        self.quality_notes.push(format!(
            "auto-closed unclosed theorem/proof element before line {before_line}"
        ));
        let close_line = before_line.saturating_sub(1).max(1);
        while self.elem_stack.len() > hit {
            if let Some(popped) = self.elem_stack.pop() {
                finalize_element(self, popped, close_line);
            }
        }
    }

    fn on_end_tag(&mut self, name: &str, end_line: usize) {
        if name == "title" && self.in_html_title {
            self.in_html_title = false;
            let t = collapse_ws(&self.html_title_buf);
            if !t.is_empty() && self.document_title.is_none() {
                self.document_title = Some(t);
            }
        }
        let Some(idx) = self.elem_stack.iter().rposition(|e| e.tag_name == name) else {
            return;
        };
        if idx + 1 < self.elem_stack.len() {
            let skipped_structural = self.elem_stack[idx + 1..]
                .iter()
                .any(|e| e.node_order.is_some() || e.pending_eq.is_some());
            if skipped_structural {
                self.quality_notes.push(format!(
                    "auto-closed unclosed HTML elements before </{name}> at line {end_line}"
                ));
            }
        }
        while self.elem_stack.len() > idx {
            if let Some(popped) = self.elem_stack.pop() {
                finalize_element(self, popped, end_line);
            }
        }
    }

    pub(crate) fn finish_all(&mut self, eof_line: usize) {
        while let Some(popped) = self.elem_stack.pop() {
            if popped.node_order.is_some() || popped.pending_eq.is_some() {
                self.quality_notes.push(format!(
                    "unclosed <{}> element recovered at EOF (line {eof_line})",
                    popped.tag_name
                ));
            }
            finalize_element(self, popped, eof_line);
        }
    }

    fn on_text(&mut self, decoded: &str, start_line: usize, end_line: usize) {
        if self.in_html_title {
            self.html_title_buf.push_str(decoded);
            return;
        }
        if self.elem_stack.iter().any(|e| e.ignore_text) {
            return;
        }
        self.append_decoded_text(decoded, start_line, end_line);
    }

    fn append_decoded_text(&mut self, decoded: &str, start_line: usize, end_line: usize) {
        let in_tag = self.in_tag_span();
        if self.elem_stack.iter().any(|e| e.is_doc_title) && !in_tag {
            self.doc_title_buf.push_str(decoded);
        }
        if let Some(bib) = self.pending_bib.as_mut() {
            if in_tag {
                bib.tag_text.push_str(decoded);
            } else {
                bib.raw_text.push_str(decoded);
                if bib.in_title_span > 0 {
                    bib.title_buf.push_str(decoded);
                }
            }
            return;
        }
        if let Some(cite) = self.active_cites.last_mut() {
            cite.text_buf.push_str(decoded);
        }
        let mut inside_pending_eq = false;
        for elem in self.elem_stack.iter_mut().rev() {
            if let Some(eq) = elem.pending_eq.as_mut() {
                inside_pending_eq = true;
                if in_tag {
                    eq.tag_text.push_str(decoded);
                } else {
                    eq.body_text.push_str(decoded);
                }
                break;
            }
            if elem.is_eq_group {
                inside_pending_eq = true;
                if in_tag {
                    elem.eq_group_tag.push_str(decoded);
                } else {
                    elem.eq_group_text.push_str(decoded);
                }
                break;
            }
        }

        if let Some(heading_ord) = self.current_heading_target() {
            if in_tag {
                self.node_tag_texts[heading_ord].push_str(decoded);
            } else {
                self.node_title_texts[heading_ord].push_str(decoded);
            }
        } else if !inside_pending_eq && in_tag {
            if let Some(ord) = self.current_node_order() {
                self.node_tag_texts[ord].push_str(decoded);
            }
        } else if !in_tag && !inside_pending_eq && !decoded.trim().is_empty() {
            if let Some(enc_idx) = self.current_node_order() {
                if self.nodes[enc_idx].kind != SkNodeKind::Bibliography {
                    self.prose_lines.push(HtmlTextLine {
                        enc_idx,
                        site: Span::new(self.path, start_line, end_line),
                        text: decoded.to_string(),
                    });
                }
            }
        }

        for elem in &self.elem_stack {
            if let Some(ord) = elem.node_order {
                if !in_tag {
                    self.node_fp_texts[ord].push_str(decoded);
                    self.node_plain_texts[ord].push_str(decoded);
                }
            }
        }
    }

    pub(crate) fn push_node(
        &mut self,
        kind: SkNodeKind,
        env_name: Option<String>,
        label: Option<String>,
        parent: Option<usize>,
        start_line: usize,
    ) -> Option<usize> {
        if self.nodes.len() >= MAX_SKELETON_NODES {
            if !self.node_limit_hit {
                self.node_limit_hit = true;
                self.quality_notes.push(format!("node limit ({MAX_SKELETON_NODES}) reached"));
            }
            return None;
        }
        let order = self.nodes.len();
        self.nodes.push(SkNode {
            tag: None,
            kind,
            number: None,
            number_basis: None,
            env_name,
            title: None,
            label,
            parent: None,
            order,
            spans: vec![Span::new(self.path, start_line, start_line)],
            proof_of: None,
            head_cites: Vec::new(),
            intro_refs: false,
            fp: String::new(),
            words: 0,
            eq_count: 0,
            read_minutes: 0.0,
            confidence: SkConfidence::HtmlClass,
        });
        self.parent_orders.push(parent);
        self.node_fp_texts.push(String::new());
        self.node_plain_texts.push(String::new());
        self.node_tag_texts.push(String::new());
        self.node_title_texts.push(String::new());
        self.node_has_heading.push(false);
        if kind.is_provable() {
            self.last_provable_by_parent.insert(parent, Some(order));
        } else if kind == SkNodeKind::Proof {
            if let Some(prev) = self.last_provable_by_parent.insert(parent, None).flatten() {
                self.pending_adjacent_proof.insert(order, prev);
            }
        } else if kind.is_theorem_like() {
            self.last_provable_by_parent.insert(parent, None);
        }
        Some(order)
    }
}

fn plain_elem(tag_name: &str, ignore_text: bool) -> OpenElement {
    OpenElement { tag_name: tag_name.to_string(), eq_group_start_line: 1, ignore_text, ..Default::default() }
}
