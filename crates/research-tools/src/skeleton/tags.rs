//! Permanent skeleton node tags and kind-class mapping (Investigate package V2).
//!
//! Each node receives a permanent tag `"n-" + base32(sha256(PaperKey | kind_class | key))[..10]`
//! via [`crate::paper_key::node_tag`]. Merging closely related node kinds into stable kind
//! classes (for example, `Lemma` and `Theorem` both map to `"statement"`) keeps tags stable
//! across minor edits such as promoting a lemma to a theorem or renumbering sections.

use super::model::{SkNode, SkNodeKind};
use crate::paper_key::{node_tag, PaperKey};
use std::collections::HashMap;

/// Permanent mapping from every [`SkNodeKind`] variant to its lower-case `[a-z0-9_]` kind class.
///
/// Never alter an existing mapping: tags derived from these strings are permanent across sessions.
pub const KIND_CLASS_TABLE: &[(SkNodeKind, &str)] = &[
    (SkNodeKind::FrontMatter, "front_matter"),
    (SkNodeKind::Section, "section"),
    (SkNodeKind::Definition, "definition"),
    (SkNodeKind::Assumption, "definition"),
    (SkNodeKind::Theorem, "statement"),
    (SkNodeKind::Lemma, "statement"),
    (SkNodeKind::Proposition, "statement"),
    (SkNodeKind::Corollary, "statement"),
    (SkNodeKind::Conjecture, "statement"),
    (SkNodeKind::Claim, "statement"),
    (SkNodeKind::Proof, "proof"),
    (SkNodeKind::Example, "remark"),
    (SkNodeKind::Remark, "remark"),
    (SkNodeKind::Equation, "equation"),
    (SkNodeKind::Figure, "figure"),
    (SkNodeKind::Table, "table"),
    (SkNodeKind::Algorithm, "algorithm"),
    (SkNodeKind::Bibliography, "bibliography"),
];

/// Returns the permanent kind class string for `kind`, matching [`KIND_CLASS_TABLE`].
pub fn kind_class(kind: SkNodeKind) -> &'static str {
    match kind {
        SkNodeKind::FrontMatter => "front_matter",
        SkNodeKind::Section => "section",
        SkNodeKind::Definition | SkNodeKind::Assumption => "definition",
        SkNodeKind::Theorem
        | SkNodeKind::Lemma
        | SkNodeKind::Proposition
        | SkNodeKind::Corollary
        | SkNodeKind::Conjecture
        | SkNodeKind::Claim => "statement",
        SkNodeKind::Proof => "proof",
        SkNodeKind::Example | SkNodeKind::Remark => "remark",
        SkNodeKind::Equation => "equation",
        SkNodeKind::Figure => "figure",
        SkNodeKind::Table => "table",
        SkNodeKind::Algorithm => "algorithm",
        SkNodeKind::Bibliography => "bibliography",
    }
}

/// Assigns permanent `tag` and `parent` fields to every node in `nodes` using `paper_key`.
///
/// Key derivation per node (in document order):
/// - If `node.label` is present and non-empty after trimming, base key is `"label:<trimmed>"`.
/// - Otherwise, base key is `"fp:<node.fp>"`.
/// - Duplicate `(kind_class, base_key)` occurrences across the skeleton receive suffixes
///   `#2`, `#3`, ... starting from the second occurrence so tags are always unique.
///
/// After all tags are assigned, `nodes[i].parent` is set to the tag of `nodes[p_idx]` whenever
/// `parent_orders.get(i)` yields `Some(Some(p_idx))`, or `None` otherwise.
pub fn assign_tags(paper_key: &PaperKey, nodes: &mut [SkNode], parent_orders: &[Option<usize>]) {
    let mut counts: HashMap<(&'static str, String), usize> = HashMap::new();
    let mut assigned: Vec<Option<String>> = Vec::with_capacity(nodes.len());

    for node in nodes.iter_mut() {
        let cls = kind_class(node.kind);
        let base_key = match node
            .label
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(lbl) => format!("label:{lbl}"),
            None => format!("fp:{}", node.fp),
        };

        let count = counts.entry((cls, base_key.clone())).or_insert(0);
        *count += 1;
        let key = if *count == 1 {
            base_key
        } else {
            format!("{base_key}#{}", *count)
        };

        // `cls` is statically valid `[a-z0-9_]` and `key` always starts with `label:` or `fp:`.
        let tag = node_tag(paper_key, cls, &key).ok();
        assigned.push(tag.clone());
        node.tag = tag;
    }

    for (i, node) in nodes.iter_mut().enumerate() {
        node.parent = parent_orders
            .get(i)
            .copied()
            .flatten()
            .and_then(|p_idx| assigned.get(p_idx))
            .and_then(Option::as_ref)
            .cloned();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skeleton::model::{NumberBasis, SkConfidence, Span};
    use std::collections::BTreeSet;

    fn paper(raw: &str) -> PaperKey {
        PaperKey::parse(raw).expect("valid canonical paper key")
    }

    fn make_node(kind: SkNodeKind, label: Option<&str>, fp: &str, order: usize) -> SkNode {
        SkNode {
            tag: None,
            kind,
            number: None,
            number_basis: None,
            env_name: None,
            title: None,
            label: label.map(str::to_string),
            parent: None,
            order,
            spans: vec![Span::new("main.tex", order + 1, order + 1)],
            proof_of: None,
            head_cites: Vec::new(),
            intro_refs: false,
            fp: fp.to_string(),
            words: 0,
            eq_count: 0,
            read_minutes: 0.0,
            confidence: SkConfidence::TexEnv,
        }
    }

    #[test]
    fn kind_class_table_covers_every_variant_and_matches_function() {
        let expected: &[(SkNodeKind, &str)] = &[
            (SkNodeKind::FrontMatter, "front_matter"),
            (SkNodeKind::Section, "section"),
            (SkNodeKind::Definition, "definition"),
            (SkNodeKind::Assumption, "definition"),
            (SkNodeKind::Theorem, "statement"),
            (SkNodeKind::Lemma, "statement"),
            (SkNodeKind::Proposition, "statement"),
            (SkNodeKind::Corollary, "statement"),
            (SkNodeKind::Conjecture, "statement"),
            (SkNodeKind::Claim, "statement"),
            (SkNodeKind::Proof, "proof"),
            (SkNodeKind::Example, "remark"),
            (SkNodeKind::Remark, "remark"),
            (SkNodeKind::Equation, "equation"),
            (SkNodeKind::Figure, "figure"),
            (SkNodeKind::Table, "table"),
            (SkNodeKind::Algorithm, "algorithm"),
            (SkNodeKind::Bibliography, "bibliography"),
        ];

        assert_eq!(KIND_CLASS_TABLE, expected);
        assert_eq!(KIND_CLASS_TABLE.len(), 18);

        let mut seen = BTreeSet::new();
        for &(kind, cls) in KIND_CLASS_TABLE {
            assert!(
                seen.insert(kind),
                "duplicate SkNodeKind in KIND_CLASS_TABLE: {kind:?}"
            );
            assert!(!cls.is_empty());
            assert!(
                cls.bytes()
                    .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_')),
                "invalid kind class `{cls}`"
            );
            assert_eq!(kind_class(kind), cls);
        }
    }

    /// Golden vectors computed independently via Python `hashlib.sha256` + `base64.b32encode`.
    #[test]
    fn golden_node_tags_and_lemma_promotion_stability() {
        use SkNodeKind as K;

        let arxiv = paper("arxiv:1102.0842");
        let doi = paper("doi:10.1103/physrevlett.116.061102");
        let file = PaperKey::from_file_bytes(b"hello\n");

        let vectors = [
            (&arxiv, K::Theorem, "label:thm:main", "n-sashdcm2ee"),
            (&arxiv, K::Lemma, "label:thm:main", "n-sashdcm2ee"),
            (&arxiv, K::Definition, "label:def:gap", "n-tyybaqrjn3"),
            (&arxiv, K::Assumption, "label:def:gap", "n-tyybaqrjn3"),
            (&arxiv, K::Section, "label:sec:intro", "n-5fxyjukv5v"),
            (&arxiv, K::FrontMatter, "fp:abcdef012345", "n-wgtkfcsfws"),
            (&arxiv, K::Proof, "fp:fedcba987654", "n-hnnrsfpf4q"),
            (&arxiv, K::Remark, "fp:112233445566", "n-lwx3ybsmqw"),
            (&arxiv, K::Remark, "fp:112233445566#2", "n-x4ufzhideq"),
            (&doi, K::Equation, "label:eq:ham", "n-sxfqhfuuqb"),
            (&file, K::Bibliography, "fp:000000000000", "n-n6hrlf344w"),
        ];

        for (pk, kind, key, want) in vectors {
            let got = node_tag(pk, kind_class(kind), key).expect("valid tag");
            assert_eq!(got, want, "mismatch for ({}, {kind:?}, {key})", pk.as_str());
        }

        // Promoting a Lemma to a Theorem with the same label keeps the exact same tag.
        let mut lemma_nodes = vec![make_node(K::Lemma, Some("thm:main"), "aaaa1111bbbb", 0)];
        let mut thm_nodes = vec![make_node(
            K::Theorem,
            Some("  thm:main  "),
            "cccc2222dddd",
            0,
        )];
        assign_tags(&arxiv, &mut lemma_nodes, &[None]);
        assign_tags(&arxiv, &mut thm_nodes, &[None]);
        assert_eq!(lemma_nodes[0].tag.as_deref(), Some("n-sashdcm2ee"));
        assert_eq!(thm_nodes[0].tag, lemma_nodes[0].tag);
    }

    #[test]
    fn duplicate_unlabelled_remarks_get_distinct_tags_and_parent_links_wire_up() {
        let pk = paper("arxiv:1102.0842");
        let mut nodes = vec![
            make_node(SkNodeKind::Section, Some("sec:intro"), "sec1fp000001", 0),
            make_node(SkNodeKind::Remark, None, "112233445566", 1),
            make_node(SkNodeKind::Section, Some("sec:next"), "sec2fp000002", 2),
            // Whitespace-only label falls back to `fp:` and collides across sections.
            make_node(SkNodeKind::Remark, Some("   "), "112233445566", 3),
        ];
        let parent_orders = [None, Some(0), None, Some(2)];

        assign_tags(&pk, &mut nodes, &parent_orders);

        assert_eq!(nodes[0].tag.as_deref(), Some("n-5fxyjukv5v"));
        assert_eq!(nodes[0].parent, None);

        assert_eq!(nodes[1].tag.as_deref(), Some("n-lwx3ybsmqw"));
        assert_eq!(nodes[1].parent, nodes[0].tag);

        assert!(nodes[2].tag.is_some());
        assert_eq!(nodes[2].parent, None);

        assert_eq!(nodes[3].tag.as_deref(), Some("n-x4ufzhideq"));
        assert_ne!(nodes[1].tag, nodes[3].tag);
        assert_eq!(nodes[3].parent, nodes[2].tag);
    }

    #[test]
    fn printed_numbers_do_not_affect_tags() {
        let pk = paper("arxiv:1102.0842");

        let mut run_a = vec![
            make_node(SkNodeKind::Section, Some("sec:intro"), "001122334455", 0),
            make_node(SkNodeKind::Theorem, Some("thm:main"), "66778899aabb", 1),
            make_node(SkNodeKind::Proof, None, "fedcba987654", 2),
        ];
        run_a[0].number = Some("1".to_string());
        run_a[0].number_basis = Some(NumberBasis::Computed);
        run_a[1].number = Some("1.1".to_string());
        run_a[1].number_basis = Some(NumberBasis::Printed);

        let mut run_b = run_a.clone();
        run_b[0].number = Some("A".to_string());
        run_b[0].number_basis = Some(NumberBasis::Confirmed);
        run_b[1].number = None;
        run_b[1].number_basis = None;

        let parents = [None, Some(0), Some(0)];
        assign_tags(&pk, &mut run_a, &parents);
        assign_tags(&pk, &mut run_b, &parents);

        let tags_a: Vec<_> = run_a.iter().map(|n| (&n.tag, &n.parent)).collect();
        let tags_b: Vec<_> = run_b.iter().map(|n| (&n.tag, &n.parent)).collect();
        assert_eq!(tags_a, tags_b);
        assert_eq!(run_a[1].tag.as_deref(), Some("n-sashdcm2ee"));
        assert_eq!(run_a[2].tag.as_deref(), Some("n-hnnrsfpf4q"));
    }
}
