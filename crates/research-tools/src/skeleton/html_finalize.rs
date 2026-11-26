//! Element finalization pass for `HtmlWalker` (`html-1`, V18).

use super::html_bib::{HtmlCiteOccurrence, RawHtmlBibItem};
use super::html_classify::{
    clean_theorem_title, collapse_ws, compute_html_body_fp, extract_float_number,
    extract_local_fragment, extract_section_number, extract_theorem_number,
    infer_theorem_kind_from_tag, non_empty_str, parse_proof_header_mention, strip_equation_tag,
};
use super::html_edges::{HtmlProofHint, HtmlRefOccurrence};
use super::html_lex::HtmlToken;
use super::html_walk::{HtmlWalker, OpenElement};
use super::model::{NumberBasis, SkNodeKind, Span};

pub(crate) fn capture_href_or_bib_span(
    w: &mut HtmlWalker<'_>,
    attrs: &[(String, String)],
    classes: &[&str],
    start_line: usize,
    end_line: usize,
) {
    if let Some(href) = HtmlToken::attr(attrs, "href") {
        let trimmed_href = href.trim();
        if let Some(bib) = w.pending_bib.as_mut() {
            if !trimmed_href.is_empty() && !trimmed_href.starts_with('#') {
                bib.link_urls.push(trimmed_href.to_string());
            }
        } else if let Some(target_id) = extract_local_fragment(trimmed_href) {
            if let Some(cite) = w.active_cites.last_mut() {
                cite.target_bib_ids.push(target_id.to_string());
            } else if let Some(enc_idx) = w.current_node_order() {
                let heading_target = w.current_heading_target();
                let in_proof_header = heading_target.is_some_and(|h| {
                    h == enc_idx && w.nodes.get(h).is_some_and(|n| n.kind == SkNodeKind::Proof)
                });
                if in_proof_header {
                    w.proof_hints.entry(enc_idx).or_insert_with(|| {
                        HtmlProofHint::HeaderRef { target_html_id: target_id.to_string() }
                    });
                } else if !w.in_tag_span() {
                    w.refs.push(HtmlRefOccurrence {
                        enc_idx,
                        target_html_id: target_id.to_string(),
                        site: Span::new(w.path, start_line, end_line),
                        in_proof_header: false,
                    });
                }
            }
        }
    }
    if let Some(bib) = w.pending_bib.as_mut() {
        if classes.iter().any(|c| matches!(*c, "ltx_emph" | "ltx_bib_title")) {
            bib.in_title_span += 1;
        }
    }
}

pub(crate) fn finalize_element(w: &mut HtmlWalker<'_>, elem: OpenElement, end_line: usize) {
    if elem.is_doc_title {
        let t = collapse_ws(&w.doc_title_buf);
        if !t.is_empty() {
            w.document_title = Some(t);
        }
    }
    if elem.is_cite {
        if let Some(cite) = w.active_cites.pop() {
            w.cites.push(HtmlCiteOccurrence {
                enc_idx: cite.enc_idx,
                target_bib_ids: cite.target_bib_ids,
                raw_cite_text: cite.text_buf,
                site: cite.site,
                in_heading: cite.in_heading,
            });
        }
    }
    if elem.is_bib_item {
        if let Some(bib) = w.pending_bib.take() {
            let tag_label = non_empty_str(&bib.tag_text);
            let title_hint = bib.title_hint.or_else(|| non_empty_str(&bib.title_buf));
            w.bib_items.push(RawHtmlBibItem {
                html_id: bib.html_id,
                tag_label,
                raw_text: bib.raw_text,
                title_hint,
                link_urls: bib.link_urls,
            });
        }
    }
    if let Some(bib) = w.pending_bib.as_mut() {
        if bib.in_title_span > 0 && matches!(elem.tag_name.as_str(), "em" | "i" | "span") {
            if bib.title_hint.is_none() {
                bib.title_hint = non_empty_str(&bib.title_buf);
            }
            bib.in_title_span = bib.in_title_span.saturating_sub(1);
        }
    }

    if let Some(eq) = elem.pending_eq {
        let num = strip_equation_tag(&eq.tag_text);
        if num.is_some() || eq.label.is_some() {
            if let Some(ord) = w.push_node(
                SkNodeKind::Equation,
                Some("equation".into()),
                eq.label,
                eq.parent_order,
                eq.start_line,
            ) {
                let end = end_line.max(eq.start_line);
                w.nodes[ord].spans = vec![Span::new(w.path, eq.start_line, end)];
                w.nodes[ord].number = num;
                w.nodes[ord].number_basis = w.nodes[ord].number.as_ref().map(|_| NumberBasis::Html);
                w.nodes[ord].fp = compute_html_body_fp(&eq.body_text);
                w.node_plain_texts[ord] = eq.body_text;
                if let Some(id) = eq.html_id {
                    w.html_id_to_node_idx.insert(id, ord);
                }
                if let Some(grp) = w.elem_stack.iter_mut().rev().find(|e| e.is_eq_group) {
                    grp.eq_group_has_child_node = true;
                }
            }
        } else if !w.elem_stack.iter().any(|e| e.is_eq_group) {
            if let Some(p_ord) = eq.parent_order {
                if let Some(p) = w.nodes.get_mut(p_ord) {
                    p.eq_count = p.eq_count.saturating_add(1);
                }
            }
        }
        return;
    }

    if elem.is_eq_group {
        let grp_num = strip_equation_tag(&elem.eq_group_tag);
        if !elem.eq_group_has_child_node {
            if grp_num.is_some() || elem.eq_group_label.is_some() {
                let parent = w.current_node_order();
                if let Some(ord) = w.push_node(
                    SkNodeKind::Equation,
                    Some("equationgroup".into()),
                    elem.eq_group_label,
                    parent,
                    elem.eq_group_start_line,
                ) {
                    let end = end_line.max(elem.eq_group_start_line);
                    w.nodes[ord].spans = vec![Span::new(w.path, elem.eq_group_start_line, end)];
                    w.nodes[ord].number = grp_num;
                    w.nodes[ord].number_basis =
                        w.nodes[ord].number.as_ref().map(|_| NumberBasis::Html);
                    w.nodes[ord].fp = compute_html_body_fp(&elem.eq_group_text);
                    w.node_plain_texts[ord] = elem.eq_group_text;
                    if let Some(id) = elem.eq_group_id {
                        w.html_id_to_node_idx.insert(id, ord);
                    }
                }
            } else if let Some(p_ord) = w.current_node_order() {
                if let Some(p) = w.nodes.get_mut(p_ord) {
                    p.eq_count = p.eq_count.saturating_add(1);
                }
            }
        }
        return;
    }

    let Some(ord) = elem.node_order else { return };
    let start = w.nodes[ord].spans.first().map_or(1, |s| s.start_line);
    let mut end = end_line.max(start);
    for (idx, p) in w.parent_orders.iter().enumerate() {
        if *p == Some(ord) {
            if let Some(c_end) = w.nodes.get(idx).and_then(|c| c.spans.first()).map(|s| s.end_line) {
                end = end.max(c_end);
            }
        }
    }
    w.nodes[ord].spans = vec![Span::new(w.path, start, end)];
    w.nodes[ord].fp = compute_html_body_fp(&w.node_fp_texts[ord]);

    let raw_tag = w.node_tag_texts[ord].trim().to_string();
    let raw_title = w.node_title_texts[ord].trim().to_string();
    let kind = w.nodes[ord].kind;

    if kind == SkNodeKind::Proof {
        let head = collapse_ws(&format!("{raw_tag} {raw_title}"));
        if let Some((k, num)) = parse_proof_header_mention(&head) {
            w.proof_hints.entry(ord).or_insert(HtmlProofHint::HeaderMention { kind: k, number: num });
        }
        let clean_head = head.trim_end_matches(['.', ':']).trim();
        if !clean_head.is_empty() && !clean_head.eq_ignore_ascii_case("proof") {
            w.nodes[ord].title = Some(clean_head.to_string());
        }
        if !w.proof_hints.contains_key(&ord) {
            if let Some(&target_order) = w.pending_adjacent_proof.get(&ord) {
                w.proof_hints.insert(ord, HtmlProofHint::Adjacent { target_order });
            } else {
                w.proof_hints.insert(ord, HtmlProofHint::Unattached);
            }
        }
    } else if kind.is_theorem_like() {
        if w.nodes[ord].env_name.as_deref() == Some("theorem") {
            if let Some(inferred) = infer_theorem_kind_from_tag(&raw_tag) {
                w.nodes[ord].kind = inferred;
            }
        }
        w.nodes[ord].number = extract_theorem_number(&raw_tag);
        w.nodes[ord].number_basis = w.nodes[ord].number.as_ref().map(|_| NumberBasis::Html);
        if let Some(t) = clean_theorem_title(&raw_title) {
            w.nodes[ord].title = Some(t);
        }
    } else if kind == SkNodeKind::Section {
        w.nodes[ord].number = extract_section_number(&raw_tag);
        w.nodes[ord].number_basis = w.nodes[ord].number.as_ref().map(|_| NumberBasis::Html);
        let t = collapse_ws(&raw_title);
        if !t.is_empty() {
            w.nodes[ord].title = Some(t);
        }
    } else if matches!(kind, SkNodeKind::Figure | SkNodeKind::Table) {
        w.nodes[ord].number = extract_float_number(&raw_tag);
        w.nodes[ord].number_basis = w.nodes[ord].number.as_ref().map(|_| NumberBasis::Html);
        let t = collapse_ws(&raw_title);
        if !t.is_empty() {
            w.nodes[ord].title = Some(t);
        }
    }
}
