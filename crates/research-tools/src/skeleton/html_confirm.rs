//! Conservative TeX number confirmation against a paired `html-1` skeleton (V18).
//!
//! When both TeX (`tex-1`) and LaTeXML HTML (`html-1`) skeletons are available for the same
//! paper, [`confirm_tex_numbers_with_html`] upgrades `NumberBasis::Computed` ->
//! `NumberBasis::Confirmed` ONLY when an unambiguous node match exists and both rendered
//! numbers match verbatim. Mismatches are recorded in [`ConfirmReport::disagreements`]
//! without ever overwriting TeX numbers (rule 6.9 / P5 = 0).

use super::model::{NumberBasis, PaperSkeleton, SkNode, SkNodeKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Recorded numbering mismatch between a matched `tex-1` node and its `html-1` counterpart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NumberDisagreement {
    pub tex_tag: Option<String>,
    pub kind: SkNodeKind,
    pub tex_number: Option<String>,
    pub html_number: Option<String>,
    pub reason: String,
}

/// Summary of conservative `tex-1` vs `html-1` number confirmation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmReport {
    pub matched: usize,
    pub confirmed: usize,
    pub already_confirmed: usize,
    pub disagreements: Vec<NumberDisagreement>,
    pub unmatched_tex: usize,
}

/// Conservatively upgrades `NumberBasis::Computed` -> `NumberBasis::Confirmed` on `tex`
/// using unambiguous matches in `html` (`SkeletonBuilder::Html1` / `NumberBasis::Html`).
pub fn confirm_tex_numbers_with_html(
    tex: &mut PaperSkeleton,
    html: &PaperSkeleton,
) -> ConfirmReport {
    let html_idx = HtmlLookup::build(&html.nodes);
    let mut tex_fp_counts: BTreeMap<(SkNodeKind, String), usize> = BTreeMap::new();
    let mut tex_title_counts: BTreeMap<(SkNodeKind, String), usize> = BTreeMap::new();
    for node in &tex.nodes {
        if !node.fp.is_empty() {
            *tex_fp_counts.entry((node.kind, node.fp.clone())).or_insert(0) += 1;
        }
        if let Some(title) = normalize_title(node.title.as_deref()) {
            *tex_title_counts.entry((node.kind, title)).or_insert(0) += 1;
        }
    }

    let mut report = ConfirmReport::default();
    for tex_node in &mut tex.nodes {
        let Some(html_node) = find_unique_html_match(
            tex_node,
            &html.nodes,
            &html_idx,
            &tex_fp_counts,
            &tex_title_counts,
        ) else {
            if tex_node.number.is_some() {
                report.unmatched_tex += 1;
            }
            continue;
        };

        report.matched += 1;
        if html_node.number_basis != Some(NumberBasis::Html) {
            if tex_node.number.is_some() {
                report.disagreements.push(NumberDisagreement {
                    tex_tag: tex_node.tag.clone(),
                    kind: tex_node.kind,
                    tex_number: tex_node.number.clone(),
                    html_number: html_node.number.clone(),
                    reason: "matched HTML node lacks NumberBasis::Html rendered number".to_string(),
                });
            }
            continue;
        }

        let tex_num = normalize_num(tex_node.number.as_deref());
        let html_num = normalize_num(html_node.number.as_deref());
        match (tex_num, html_num) {
            (Some(t), Some(h)) if t == h => match tex_node.number_basis {
                Some(NumberBasis::Computed) => {
                    tex_node.number_basis = Some(NumberBasis::Confirmed);
                    report.confirmed += 1;
                }
                Some(NumberBasis::Confirmed) => {
                    report.already_confirmed += 1;
                }
                _ => {}
            },
            (None, None) => {}
            (t, h) => {
                report.disagreements.push(NumberDisagreement {
                    tex_tag: tex_node.tag.clone(),
                    kind: tex_node.kind,
                    tex_number: t,
                    html_number: h,
                    reason: "TeX and HTML rendered numbers differ; TeX number preserved".to_string(),
                });
            }
        }
    }
    report
}

struct HtmlLookup {
    by_label: BTreeMap<(SkNodeKind, String), Option<usize>>,
    by_fp: BTreeMap<(SkNodeKind, String), Option<usize>>,
    by_title: BTreeMap<(SkNodeKind, String), Option<usize>>,
}

impl HtmlLookup {
    fn build(nodes: &[SkNode]) -> Self {
        let mut by_label = BTreeMap::new();
        let mut by_fp = BTreeMap::new();
        let mut by_title = BTreeMap::new();
        for (i, n) in nodes.iter().enumerate() {
            if let Some(lbl) = n.label.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                insert_unique(&mut by_label, (n.kind, lbl.to_string()), i);
            }
            if !n.fp.is_empty() {
                insert_unique(&mut by_fp, (n.kind, n.fp.clone()), i);
            }
            if let Some(title) = normalize_title(n.title.as_deref()) {
                insert_unique(&mut by_title, (n.kind, title), i);
            }
        }
        Self { by_label, by_fp, by_title }
    }
}

fn insert_unique(map: &mut BTreeMap<(SkNodeKind, String), Option<usize>>, key: (SkNodeKind, String), idx: usize) {
    map.entry(key).and_modify(|slot| *slot = None).or_insert(Some(idx));
}

fn find_unique_html_match<'a>(
    tex_node: &SkNode,
    html_nodes: &'a [SkNode],
    html_idx: &HtmlLookup,
    tex_fp_counts: &BTreeMap<(SkNodeKind, String), usize>,
    tex_title_counts: &BTreeMap<(SkNodeKind, String), usize>,
) -> Option<&'a SkNode> {
    if let Some(lbl) = tex_node.label.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(Some(idx)) = html_idx.by_label.get(&(tex_node.kind, lbl.to_string())).copied() {
            return html_nodes.get(idx);
        }
    }
    if !tex_node.fp.is_empty() {
        let key = (tex_node.kind, tex_node.fp.clone());
        if tex_fp_counts.get(&key).copied() == Some(1) {
            if let Some(Some(idx)) = html_idx.by_fp.get(&key).copied() {
                return html_nodes.get(idx);
            }
        }
    }
    if let Some(title) = normalize_title(tex_node.title.as_deref()) {
        let key = (tex_node.kind, title);
        if tex_title_counts.get(&key).copied() == Some(1) {
            if let Some(Some(idx)) = html_idx.by_title.get(&key).copied() {
                return html_nodes.get(idx);
            }
        }
    }
    None
}

fn normalize_title(raw: Option<&str>) -> Option<String> {
    let clean = raw?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches(['.', ':'])
        .trim()
        .to_ascii_lowercase();
    (!clean.is_empty()).then_some(clean)
}

fn normalize_num(raw: Option<&str>) -> Option<String> {
    let s = raw?.trim();
    (!s.is_empty()).then(|| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paper_key::PaperKey;
    use crate::skeleton::model::{
        SkConfidence, SkeletonBuilder, SkeletonQuality, SKELETON_SCHEMA_VERSION,
    };

    fn mk_node(
        tag: &str,
        kind: SkNodeKind,
        num: &str,
        basis: NumberBasis,
        label: Option<&str>,
        title: Option<&str>,
        fp: &str,
    ) -> SkNode {
        let confidence = match basis {
            NumberBasis::Html => SkConfidence::HtmlClass,
            _ => SkConfidence::TexEnv,
        };
        SkNode {
            tag: Some(tag.into()),
            kind,
            number: Some(num.into()),
            number_basis: Some(basis),
            env_name: None,
            title: title.map(str::to_string),
            label: label.map(str::to_string),
            parent: None,
            order: 0,
            spans: vec![],
            proof_of: None,
            head_cites: vec![],
            intro_refs: false,
            fp: fp.into(),
            words: 10,
            eq_count: 0,
            read_minutes: 0.1,
            confidence,
        }
    }

    fn mk_skeleton(builder: SkeletonBuilder, nodes: Vec<SkNode>) -> PaperSkeleton {
        PaperSkeleton {
            schema: SKELETON_SCHEMA_VERSION,
            builder,
            source_id: "s".into(),
            source_seq: 1,
            files_root_sha256: "0".repeat(64),
            paper_key: PaperKey::from_arxiv("2601.00055").unwrap(),
            version: None,
            quality: SkeletonQuality::Full,
            quality_reason: None,
            nodes,
            edges: vec![],
            bib: vec![],
            symbols: vec![],
            macros: vec![],
            stats: vec![],
            carry: None,
        }
    }

    #[test]
    fn confirms_verbatim_matches_and_never_overwrites_mismatches() {
        let mut tex = mk_skeleton(
            SkeletonBuilder::Tex1,
            vec![
                mk_node("n-t1", SkNodeKind::Theorem, "2.1", NumberBasis::Computed, Some("thm:a"), None, "fp1"),
                mk_node("n-l1", SkNodeKind::Lemma, "2.2", NumberBasis::Computed, None, Some("Decay Lemma."), "fp2"),
                mk_node("n-p1", SkNodeKind::Proposition, "2.3", NumberBasis::Computed, None, None, "fp_unique"),
                mk_node("n-d1", SkNodeKind::Theorem, "2.4", NumberBasis::Computed, Some("thm:drift"), None, "fp_d"),
                mk_node("n-amb1", SkNodeKind::Remark, "1", NumberBasis::Computed, None, Some("Note"), "fp_dup"),
            ],
        );
        let html = mk_skeleton(
            SkeletonBuilder::Html1,
            vec![
                mk_node("h-t1", SkNodeKind::Theorem, "2.1", NumberBasis::Html, Some("thm:a"), None, "different"),
                mk_node("h-l1", SkNodeKind::Lemma, "2.2", NumberBasis::Html, None, Some("decay lemma"), "other"),
                mk_node("h-p1", SkNodeKind::Proposition, "2.3", NumberBasis::Html, None, None, "fp_unique"),
                mk_node("h-d1", SkNodeKind::Theorem, "3.1", NumberBasis::Html, Some("thm:drift"), None, "fp_d"),
                mk_node("h-amb1", SkNodeKind::Remark, "1", NumberBasis::Html, None, Some("Note"), "fp_dup"),
                mk_node("h-amb2", SkNodeKind::Remark, "2", NumberBasis::Html, None, Some("Note"), "fp_dup"),
            ],
        );

        let report = confirm_tex_numbers_with_html(&mut tex, &html);
        assert_eq!(report.confirmed, 3);
        assert_eq!(report.disagreements.len(), 1);
        assert_eq!(report.unmatched_tex, 1);
        assert_eq!(tex.nodes[0].number_basis, Some(NumberBasis::Confirmed));
        assert_eq!(tex.nodes[1].number_basis, Some(NumberBasis::Confirmed));
        assert_eq!(tex.nodes[2].number_basis, Some(NumberBasis::Confirmed));
        // Mismatched node keeps Computed and original TeX number `"2.4"`.
        assert_eq!(tex.nodes[3].number.as_deref(), Some("2.4"));
        assert_eq!(tex.nodes[3].number_basis, Some(NumberBasis::Computed));
        assert_eq!(report.disagreements[0].html_number.as_deref(), Some("3.1"));
        // Ambiguous duplicate remains untouched.
        assert_eq!(tex.nodes[4].number_basis, Some(NumberBasis::Computed));
    }
}
