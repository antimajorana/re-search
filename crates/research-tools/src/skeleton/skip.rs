//! Section-level skip plan and reading-time breakdown (`investigate.md` §4.3, §6.14, §12).
//!
//! Computes backward statement and proof closures from selected main result tags over
//! deterministic skeleton cross-references (plus optional extra edges) and classifies each
//! section as needed for statements, needed for proofs, or unreferenced.

use super::model::{
    EdgeConfidence, NumberBasis, PaperSkeleton, SkEdge, SkEdgeKind, SkEdgeRole, SkEdgeRule, SkNode,
    SkNodeKind,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Basis wording when section statuses follow solely from explicit cross-references (`investigate.md` §4.3, §6.14, §12).
pub const SKIP_BASIS_EXPLICIT: &str = "by explicit cross-references";
/// Basis wording when at least one section's status changed via a model-inferred link (`investigate.md` §4.3, §6.14, §12).
pub const SKIP_BASIS_WITH_MODEL: &str = "by cross-references (includes model-inferred links)";
/// Section label for sections required to read the main statements (`investigate.md` §4.3, §6.14, §12).
pub const SKIP_LABEL_STATEMENT: &str = "needed to read the main statements";
/// Section label for sections required to check the proofs (`investigate.md` §4.3, §6.14, §12).
pub const SKIP_LABEL_PROOF: &str = "needed to check the proofs";
/// Section label for sections not reached from any main result (`investigate.md` §4.3, §6.14, §12).
pub const SKIP_LABEL_UNREFERENCED: &str = "not referred to by any main result";

/// Provenance basis for a computed [`SkipPlan`] (`investigate.md` §6.14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipBasis { Explicit, IncludesModelInferred }

impl SkipBasis {
    pub fn as_text(self) -> &'static str {
        match self { Self::Explicit => SKIP_BASIS_EXPLICIT, Self::IncludesModelInferred => SKIP_BASIS_WITH_MODEL }
    }
}

/// Classification of one section relative to the selected main results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionNeed { Statement, Proof, Unreferenced }

impl SectionNeed {
    pub fn label(self) -> &'static str {
        match self {
            Self::Statement => SKIP_LABEL_STATEMENT,
            Self::Proof => SKIP_LABEL_PROOF,
            Self::Unreferenced => SKIP_LABEL_UNREFERENCED,
        }
    }
    fn rank(self) -> u8 { match self { Self::Unreferenced => 0, Self::Proof => 1, Self::Statement => 2 } }
    fn merge(self, other: Self) -> Self { if other.rank() > self.rank() { other } else { self } }
}

/// Additional `Uses` edge supplied outside `PaperSkeleton::edges` (e.g., model-inferred).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtraUseEdge { pub from: String, pub to: String, pub role: SkEdgeRole, pub model_inferred: bool }

/// Per-section entry in a [`SkipPlan`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkipSectionEntry {
    pub section_tag: String,
    pub label: String,
    pub need: SectionNeed,
    pub reason: String,
    pub read_minutes: f64,
    pub dim: bool,
}

/// Deterministic skip plan and two-tier reading-time breakdown for one paper skeleton.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkipPlan {
    pub empty: bool,
    pub main_result_tags: Vec<String>,
    pub basis: SkipBasis,
    pub basis_text: String,
    pub statement_closure: Vec<String>,
    pub proof_closure: Vec<String>,
    pub statement_minutes: f64,
    pub proof_minutes: f64,
    pub unreferenced_minutes: f64,
    pub total_minutes: f64,
    pub sections: Vec<SkipSectionEntry>,
    pub summary_line: String,
}

/// Computes the section-level skip plan for `skeleton` relative to `main_result_tags`
/// (`investigate.md` §4.3, §6.14, §12).
pub fn compute_skip_plan(
    skeleton: &PaperSkeleton,
    main_result_tags: &[String],
    extra_edges: &[ExtraUseEdge],
) -> SkipPlan {
    let mut nodes_by_tag: BTreeMap<&str, &SkNode> = BTreeMap::new();
    let mut section_nodes: Vec<(&str, &SkNode)> = Vec::new();
    for node in &skeleton.nodes {
        if let Some(tag) = node.tag.as_deref() {
            nodes_by_tag.insert(tag, node);
            if node.kind == SkNodeKind::Section { section_nodes.push((tag, node)); }
        }
    }

    let mut seen_main = BTreeSet::new();
    let live_main: Vec<String> = main_result_tags.iter()
        .filter(|t| nodes_by_tag.contains_key(t.as_str()) && seen_main.insert(t.as_str()))
        .cloned().collect();
    let total_minutes = round2(
        section_nodes.iter().filter(|(_, n)| is_top_level_section(n, &nodes_by_tag)).map(|(_, n)| n.read_minutes).sum(),
    );

    if live_main.is_empty() {
        let sections = section_nodes.iter().map(|&(tag, node)| SkipSectionEntry {
            section_tag: tag.to_string(), label: section_display_label(node), need: SectionNeed::Unreferenced,
            reason: "no main results selected yet; not dimmed by cross-references".into(),
            read_minutes: round2(node.read_minutes), dim: false,
        }).collect();
        return SkipPlan {
            empty: true, main_result_tags: Vec::new(), basis: SkipBasis::Explicit,
            basis_text: SKIP_BASIS_EXPLICIT.into(), statement_closure: Vec::new(), proof_closure: Vec::new(),
            statement_minutes: 0.0, proof_minutes: 0.0, unreferenced_minutes: 0.0, total_minutes, sections,
            summary_line: "No main results selected yet; nothing is dimmed by cross-references.".into(),
        };
    }

    let explicit = run_pass(skeleton, &nodes_by_tag, &section_nodes, &live_main, extra_edges, false);
    let has_model = extra_edges.iter().any(|e| e.model_inferred);
    let (active, basis) = if has_model {
        let combined = run_pass(skeleton, &nodes_by_tag, &section_nodes, &live_main, extra_edges, true);
        let changed = section_nodes.iter().any(|&(tag, _)| explicit.needs.get(tag) != combined.needs.get(tag));
        (combined, if changed { SkipBasis::IncludesModelInferred } else { SkipBasis::Explicit })
    } else {
        (explicit.clone(), SkipBasis::Explicit)
    };

    let (mut stmt_sum, mut proof_sum, mut unref_sum, mut dimmed_count) = (0.0, 0.0, 0.0, 0usize);
    let sections: Vec<SkipSectionEntry> = section_nodes.iter().map(|&(tag, node)| {
        let need = active.needs.get(tag).copied().unwrap_or(SectionNeed::Unreferenced);
        let changed_by_model = has_model && explicit.needs.get(tag).copied() != Some(need);
        let sec_basis = if changed_by_model { SKIP_BASIS_WITH_MODEL } else { SKIP_BASIS_EXPLICIT };
        let dim = need == SectionNeed::Unreferenced;
        if dim { dimmed_count += 1; }
        if is_top_level_section(node, &nodes_by_tag) {
            match need {
                SectionNeed::Statement => { stmt_sum += node.read_minutes; proof_sum += node.read_minutes; }
                SectionNeed::Proof => proof_sum += node.read_minutes,
                SectionNeed::Unreferenced => unref_sum += node.read_minutes,
            }
        }
        SkipSectionEntry {
            section_tag: tag.to_string(), label: section_display_label(node), need,
            reason: format!("{}, {sec_basis}", need.label()), read_minutes: round2(node.read_minutes), dim,
        }
    }).collect();

    let (statement_minutes, proof_minutes, unreferenced_minutes) = (round2(stmt_sum), round2(proof_sum), round2(unref_sum));
    let basis_text = basis.as_text().to_string();
    let summary_line = format!(
        "Reading plan {basis_text}: {statement_minutes:.1} min for statements, {proof_minutes:.1} min for proofs, \
         {unreferenced_minutes:.1} min unreferenced ({dimmed_count} of {} sections dimmed).",
        sections.len()
    );
    SkipPlan {
        empty: false, main_result_tags: live_main, basis, basis_text,
        statement_closure: active.statement_closure, proof_closure: active.proof_closure,
        statement_minutes, proof_minutes, unreferenced_minutes, total_minutes, sections, summary_line,
    }
}

#[derive(Clone)]
struct PassResult<'a> { statement_closure: Vec<String>, proof_closure: Vec<String>, needs: BTreeMap<&'a str, SectionNeed> }

fn run_pass<'a>(
    skeleton: &'a PaperSkeleton, nodes_by_tag: &BTreeMap<&'a str, &'a SkNode>,
    section_nodes: &[(&'a str, &'a SkNode)], live_main: &[String], extra_edges: &[ExtraUseEdge], include_model: bool,
) -> PassResult<'a> {
    let mut stmt_preds: BTreeMap<&'a str, Vec<&'a str>> = BTreeMap::new();
    let mut all_preds: BTreeMap<&'a str, Vec<&'a str>> = BTreeMap::new();
    let mut add = |from: &'a str, to: &'a str, role: SkEdgeRole| match role {
        SkEdgeRole::Statement => { stmt_preds.entry(to).or_default().push(from); all_preds.entry(to).or_default().push(from); }
        SkEdgeRole::Proof => all_preds.entry(to).or_default().push(from),
        SkEdgeRole::Prose => {}
    };
    for edge in &skeleton.edges {
        if is_eligible_skeleton_edge(edge, nodes_by_tag) { add(edge.from.as_str(), edge.to.as_str(), edge.role); }
    }
    for edge in extra_edges {
        if edge.model_inferred && !include_model { continue; }
        if let (Some((&from, _)), Some((&to, _))) =
            (nodes_by_tag.get_key_value(edge.from.as_str()), nodes_by_tag.get_key_value(edge.to.as_str()))
        {
            add(from, to, edge.role);
        }
    }
    let seeds: Vec<&'a str> =
        live_main.iter().filter_map(|t| nodes_by_tag.get_key_value(t.as_str()).map(|(&k, _)| k)).collect();
    let stmt_set = bfs_closure(&seeds, &stmt_preds, nodes_by_tag, true);
    let stmt_seeds: Vec<&'a str> = stmt_set.iter().copied().collect();
    let proof_set = bfs_closure(&stmt_seeds, &all_preds, nodes_by_tag, false);
    let mut needs: BTreeMap<&'a str, SectionNeed> =
        section_nodes.iter().map(|&(tag, _)| (tag, SectionNeed::Unreferenced)).collect();
    for &tag in &proof_set { mark_enclosing_sections(tag, SectionNeed::Proof, nodes_by_tag, &mut needs); }
    for &tag in &stmt_set { mark_enclosing_sections(tag, SectionNeed::Statement, nodes_by_tag, &mut needs); }
    PassResult {
        statement_closure: ordered_closure(skeleton, &stmt_set),
        proof_closure: ordered_closure(skeleton, &proof_set),
        needs,
    }
}

fn is_eligible_skeleton_edge(edge: &SkEdge, nodes_by_tag: &BTreeMap<&str, &SkNode>) -> bool {
    if edge.kind != SkEdgeKind::Uses || !nodes_by_tag.contains_key(edge.to.as_str()) { return false; }
    let Some(from_node) = nodes_by_tag.get(edge.from.as_str()) else { return false; };
    let heuristic = edge.rule == SkEdgeRule::TextMention || edge.confidence == EdgeConfidence::Heuristic;
    !heuristic || from_node.number_basis == Some(NumberBasis::Confirmed)
}

fn bfs_closure<'a>(
    seeds: &[&'a str], preds: &BTreeMap<&'a str, Vec<&'a str>>,
    nodes_by_tag: &BTreeMap<&'a str, &'a SkNode>, expand_eq_parent: bool,
) -> BTreeSet<&'a str> {
    let (mut visited, mut queue) = (BTreeSet::new(), VecDeque::new());
    for &seed in seeds { if visited.insert(seed) { queue.push_back(seed); } }
    while let Some(cur) = queue.pop_front() {
        if expand_eq_parent {
            if let Some((&p_key, _)) = nodes_by_tag.get(cur)
                .filter(|n| n.kind == SkNodeKind::Equation)
                .and_then(|n| n.parent.as_deref())
                .and_then(|p| nodes_by_tag.get_key_value(p))
            {
                if visited.insert(p_key) { queue.push_back(p_key); }
            }
        }
        if let Some(prevs) = preds.get(cur) {
            for &prev in prevs { if visited.insert(prev) { queue.push_back(prev); } }
        }
    }
    visited
}

fn ordered_closure(skeleton: &PaperSkeleton, set: &BTreeSet<&str>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    skeleton.nodes.iter().filter_map(|n| n.tag.as_deref())
        .filter(|t| set.contains(t) && seen.insert(*t)).map(str::to_string).collect()
}

/// Walks `tag` and its `parent` chain (with a cycle guard), marking every enclosing `Section` with at least `need`.
fn mark_enclosing_sections<'a>(
    tag: &'a str, need: SectionNeed, nodes_by_tag: &BTreeMap<&'a str, &'a SkNode>, needs: &mut BTreeMap<&'a str, SectionNeed>,
) {
    let (mut cur, mut visited) = (Some(tag), BTreeSet::new());
    while let Some(t) = cur {
        if !visited.insert(t) { break; }
        let Some((&key, &node)) = nodes_by_tag.get_key_value(t) else { break; };
        if node.kind == SkNodeKind::Section {
            let entry = needs.entry(key).or_insert(SectionNeed::Unreferenced);
            *entry = entry.merge(need);
        }
        cur = node.parent.as_deref();
    }
}

fn is_top_level_section(node: &SkNode, nodes_by_tag: &BTreeMap<&str, &SkNode>) -> bool {
    node.kind == SkNodeKind::Section
        && node.parent.as_deref().and_then(|p| nodes_by_tag.get(p)).is_none_or(|p| p.kind != SkNodeKind::Section)
}

fn section_display_label(node: &SkNode) -> String {
    let num = node.number.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let title = node.title.as_deref().map(str::trim).filter(|s| !s.is_empty());
    match (num, title) {
        (Some(n), Some(t)) => format!("Section {n}: {t}"),
        (Some(n), None) => format!("Section {n}"),
        (None, Some(t)) => t.to_string(),
        (None, None) => node.tag.clone().unwrap_or_else(|| "Section".to_string()),
    }
}

fn round2(v: f64) -> f64 { (v * 100.0).round() / 100.0 }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paper_key::PaperKey;
    use crate::skeleton::model::{SkConfidence, SkeletonBuilder, SkeletonQuality, Span, SKELETON_SCHEMA_VERSION};

    fn mk_node(tag: &str, kind: SkNodeKind, parent: Option<&str>, order: usize) -> SkNode {
        SkNode {
            tag: Some(tag.into()), kind, number: None, number_basis: None, env_name: None,
            title: None, label: None, parent: parent.map(str::to_string), order,
            spans: vec![Span::new("main.tex", order + 1, order + 1)], proof_of: None,
            head_cites: vec![], intro_refs: false, fp: format!("{order:012x}"),
            words: 0, eq_count: 0, read_minutes: 0.0, confidence: SkConfidence::TexEnv,
        }
    }

    fn mk_sec(tag: &str, num: &str, title: Option<&str>, parent: Option<&str>, order: usize, mins: f64) -> SkNode {
        let mut n = mk_node(tag, SkNodeKind::Section, parent, order);
        (n.number, n.title, n.read_minutes, n.confidence) = (Some(num.into()), title.map(str::to_string), mins, SkConfidence::TexHeading);
        n
    }

    fn mk_edge(from: &str, to: &str, role: SkEdgeRole, rule: SkEdgeRule, conf: EdgeConfidence) -> SkEdge {
        SkEdge { from: from.into(), to: to.into(), kind: SkEdgeKind::Uses, role, site: Span::new("main.tex", 1, 1), rule, confidence: conf }
    }

    fn sample_skeleton() -> PaperSkeleton {
        let mut lem_h = mk_node("n-lem-h", SkNodeKind::Lemma, Some("n-s4"), 9);
        (lem_h.number, lem_h.number_basis) = (Some("4.1".into()), Some(NumberBasis::Computed));
        PaperSkeleton {
            schema: SKELETON_SCHEMA_VERSION, builder: SkeletonBuilder::Tex1,
            source_id: "s1".into(), source_seq: 1, files_root_sha256: "0".repeat(64),
            paper_key: PaperKey::from_arxiv("2401.00001").expect("valid key"), version: Some("v1".into()),
            quality: SkeletonQuality::Full, quality_reason: None,
            nodes: vec![
                mk_sec("n-s1", "1", Some("Setup"), None, 0, 2.5),
                mk_node("n-def1", SkNodeKind::Definition, Some("n-s1"), 1),
                mk_node("n-eq1", SkNodeKind::Equation, Some("n-def1"), 2),
                mk_sec("n-s2", "2", Some("Maximum principle"), None, 3, 4.0),
                mk_sec("n-s2a", "2.1", None, Some("n-s2"), 4, 1.5),
                mk_node("n-lem2", SkNodeKind::Lemma, Some("n-s2a"), 5),
                mk_sec("n-s3", "3", Some("Main theorem"), None, 6, 3.0),
                mk_node("n-thm", SkNodeKind::Theorem, Some("n-s3"), 7),
                mk_sec("n-s4", "4", Some("Digression"), None, 8, 5.0),
                lem_h,
            ],
            edges: vec![
                mk_edge("n-eq1", "n-thm", SkEdgeRole::Statement, SkEdgeRule::Eqref, EdgeConfidence::Explicit),
                mk_edge("n-lem2", "n-thm", SkEdgeRole::Proof, SkEdgeRule::Ref, EdgeConfidence::Explicit),
                mk_edge("n-lem-h", "n-thm", SkEdgeRole::Proof, SkEdgeRule::TextMention, EdgeConfidence::Heuristic),
            ],
            bib: vec![], symbols: vec![], macros: vec![], stats: vec![], carry: None,
        }
    }

    fn assert_wording(plan: &SkipPlan) {
        assert!(plan.summary_line.contains("cross-references"), "summary: {}", plan.summary_line);
        for t in [plan.summary_line.as_str(), plan.basis_text.as_str()].into_iter().chain(plan.sections.iter().map(|s| s.reason.as_str())) {
            let low = t.to_ascii_lowercase();
            assert!(!low.contains("not needed") && !low.contains("verified"), "bad wording: {t}");
        }
    }

    #[test]
    fn empty_main_results_dims_nothing() {
        let plan = compute_skip_plan(&sample_skeleton(), &["n-missing".into()], &[]);
        assert!(plan.empty && plan.basis == SkipBasis::Explicit && plan.basis_text == SKIP_BASIS_EXPLICIT);
        assert!(plan.sections.iter().all(|s| !s.dim && s.need == SectionNeed::Unreferenced));
        assert!((plan.total_minutes - 14.5).abs() < 1e-9 && plan.statement_minutes == 0.0 && plan.proof_minutes == 0.0);
        assert_wording(&plan);
    }

    #[test]
    fn explicit_plan_classifies_sections_equations_and_two_reading_times() {
        let plan = compute_skip_plan(&sample_skeleton(), &["n-thm".into(), "n-thm".into()], &[]);
        assert!(!plan.empty && plan.main_result_tags == vec!["n-thm"] && plan.basis == SkipBasis::Explicit);
        assert!(plan.statement_closure.contains(&"n-def1".to_string()));
        let by_sec: BTreeMap<&str, &SkipSectionEntry> = plan.sections.iter().map(|s| (s.section_tag.as_str(), s)).collect();
        assert_eq!((by_sec["n-s1"].need, by_sec["n-s3"].need), (SectionNeed::Statement, SectionNeed::Statement));
        assert_eq!((by_sec["n-s2a"].need, by_sec["n-s2"].need), (SectionNeed::Proof, SectionNeed::Proof));
        assert_eq!((by_sec["n-s2"].label.as_str(), by_sec["n-s2a"].label.as_str()), ("Section 2: Maximum principle", "Section 2.1"));
        assert!(by_sec["n-s4"].need == SectionNeed::Unreferenced && by_sec["n-s4"].dim && !by_sec["n-s2"].dim);
        assert!((plan.statement_minutes - 5.5).abs() < 1e-9 && (plan.proof_minutes - 9.5).abs() < 1e-9);
        assert!((plan.unreferenced_minutes - 5.0).abs() < 1e-9 && (plan.total_minutes - 14.5).abs() < 1e-9);
        assert_wording(&plan);
    }

    #[test]
    fn heuristic_text_mention_ignored_until_number_confirmed() {
        let mut skel = sample_skeleton();
        let before = compute_skip_plan(&skel, &["n-thm".into()], &[]);
        assert_eq!(before.sections.iter().find(|s| s.section_tag == "n-s4").unwrap().need, SectionNeed::Unreferenced);
        skel.nodes.iter_mut().find(|n| n.tag.as_deref() == Some("n-lem-h")).unwrap().number_basis = Some(NumberBasis::Confirmed);
        let after = compute_skip_plan(&skel, &["n-thm".into()], &[]);
        assert_eq!(after.sections.iter().find(|s| s.section_tag == "n-s4").unwrap().need, SectionNeed::Proof);
        assert_eq!(after.basis, SkipBasis::Explicit);
        assert_wording(&after);
    }

    #[test]
    fn model_inferred_edge_switches_basis_only_when_section_status_changes() {
        let skel = sample_skeleton();
        let unchanged = compute_skip_plan(&skel, &["n-thm".into()], &[ExtraUseEdge {
            from: "n-def1".into(), to: "n-thm".into(), role: SkEdgeRole::Proof, model_inferred: true,
        }]);
        assert_eq!((unchanged.basis, unchanged.basis_text.as_str()), (SkipBasis::Explicit, SKIP_BASIS_EXPLICIT));
        assert_wording(&unchanged);
        let changed = compute_skip_plan(&skel, &["n-thm".into()], &[ExtraUseEdge {
            from: "n-lem-h".into(), to: "n-thm".into(), role: SkEdgeRole::Proof, model_inferred: true,
        }]);
        assert_eq!((changed.basis, changed.basis_text.as_str()), (SkipBasis::IncludesModelInferred, SKIP_BASIS_WITH_MODEL));
        assert!(changed.summary_line.contains("includes model-inferred links"));
        assert_eq!(changed.sections.iter().find(|s| s.section_tag == "n-s4").unwrap().need, SectionNeed::Proof);
        assert_wording(&changed);
    }
}
