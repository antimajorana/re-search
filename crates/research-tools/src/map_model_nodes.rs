//! Content-based model-node tag derivation and arbitrary-depth parent resolution
//! (package V8, `investigate.md` §4.2–4.3, rules 6.4, 6.5, 6.12).
//!
//! Model-created nodes receive deterministic tags:
//! `"m-" + base32_lower_nopad(sha256(attach_to | kind | normalised first quote))[..10]`.
//! Re-running Open on the same node therefore produces the same `m-` tag and updates
//! its additive history instead of duplicating nodes.

use crate::investigation_markers::EVENT_INVESTIGATION_READING;
use crate::map_defs::{MapReadingEventPayload, ReadingRole};
use crate::paper_key::{base32_lower_nopad, TAG_FIELD_SEPARATOR};
use crate::skeleton::PaperSkeleton;
use crate::tool_profile::MARKER_INVESTIGATION_STARTED;
use ledger::Event;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// Reserved prefix for Investigate model-created nodes (`paper_key::ID_PREFIXES`).
pub const MODEL_NODE_PREFIX: &str = "m-";
/// Number of lowercase RFC-4648 base32 characters following `"m-"` (`investigate.md` §4.2).
pub const MODEL_NODE_TAG_CHARS: usize = 10;
/// Maximum allowed length for a model-node `kind` string.
pub const MAX_MODEL_NODE_KIND_CHARS: usize = 48;
/// Maximum allowed length for a call-local temporary `id` on `new`.
pub const MAX_MODEL_NODE_TEMP_ID_CHARS: usize = 64;
/// Maximum parent-walk depth when resolving nested `m-` chains (cycle guard).
const MAX_ANCESTOR_HOPS: usize = 64;

/// Returns `true` if `tag` has the reserved Investigate model-node prefix `"m-"`.
pub fn is_model_node_tag(tag: &str) -> bool {
    tag.starts_with(MODEL_NODE_PREFIX) && tag.len() > MODEL_NODE_PREFIX.len()
}

/// Validates and normalizes a model-node `kind` (ASCII lowercase `[a-z0-9_-]`, 1..=48 chars).
pub fn normalize_model_kind(raw: &str) -> Result<String, String> {
    let norm = raw.trim().to_ascii_lowercase();
    if norm.is_empty() || norm.chars().count() > MAX_MODEL_NODE_KIND_CHARS {
        return Err(format!(
            "Model node `kind` must be 1..={MAX_MODEL_NODE_KIND_CHARS} characters."
        ));
    }
    if !norm
        .bytes()
        .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'))
    {
        return Err(format!(
            "Model node `kind` `{norm}` must contain only ASCII letters, digits, `_`, or `-`."
        ));
    }
    Ok(norm)
}

/// Normalizes a quote for content-based `m-` tag derivation: trims, collapses runs of
/// whitespace to a single ASCII space, and lowercases.
pub fn normalize_quote_for_tag(quote: &str) -> String {
    quote
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Computes the permanent content-based tag for a model-created node (`investigate.md` §4.2):
/// `"m-" + base32_lower_nopad(sha256(attach_to | kind | normalised first quote))[..10]`.
pub fn model_node_tag(attach_to: &str, kind: &str, first_quote: &str) -> Result<String, String> {
    let parent = attach_to.trim();
    if parent.is_empty()
        || (!parent.starts_with("n-") && !parent.starts_with(MODEL_NODE_PREFIX))
        || parent.chars().any(char::is_control)
    {
        return Err(format!(
            "Model node `attach_to` `{parent}` must be a valid `n-` or `m-` tag."
        ));
    }
    let norm_kind = normalize_model_kind(kind)?;
    let norm_quote = normalize_quote_for_tag(first_quote);
    if norm_quote.is_empty() {
        return Err("Model node requires a non-empty matched quote to derive its `m-` tag.".into());
    }
    let mut hasher = Sha256::new();
    hasher.update(parent.as_bytes());
    hasher.update([TAG_FIELD_SEPARATOR as u8]);
    hasher.update(norm_kind.as_bytes());
    hasher.update([TAG_FIELD_SEPARATOR as u8]);
    hasher.update(norm_quote.as_bytes());
    let encoded = base32_lower_nopad(&hasher.finalize());
    Ok(format!("{MODEL_NODE_PREFIX}{}", &encoded[..MODEL_NODE_TAG_CHARS]))
}

/// Enforces rule 6.5 role restrictions on model-created nodes (`main_result` is never
/// allowed on proof steps, open questions, remarks, examples, figures, tables, or notes).
pub fn role_allowed_for_model_kind(role: ReadingRole, model_kind: &str) -> bool {
    if role != ReadingRole::MainResult {
        return true;
    }
    !matches!(
        model_kind.trim().to_ascii_lowercase().as_str(),
        "proof"
            | "proof_step"
            | "step"
            | "substep"
            | "open_question"
            | "question"
            | "remark"
            | "example"
            | "figure"
            | "table"
            | "bibliography"
            | "note"
    )
}

/// Resolved metadata for a previously recorded (or in-call created) `m-` model node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelNodeRef {
    pub tag: String,
    pub attach_to: String,
    pub kind: String,
    pub root_skeleton_tag: String,
    pub node_fp: String,
}

/// Collects all previously accepted `m-` model nodes for `skeleton.source_id` in the active
/// Investigation slice, resolving each node's owning `root_skeleton_tag` and current `node_fp`.
pub fn collect_existing_model_nodes(
    skeleton: &PaperSkeleton,
    events: &[Event],
) -> BTreeMap<String, ModelNodeRef> {
    let slice = events
        .iter()
        .rposition(|e| e.kind == MARKER_INVESTIGATION_STARTED)
        .map_or(events, |i| &events[i..]);
    let sk_fp: BTreeMap<&str, &str> = skeleton
        .nodes
        .iter()
        .filter_map(|n| n.tag.as_deref().map(|t| (t, n.fp.as_str())))
        .collect();

    let mut raw_links: BTreeMap<String, (String, String)> = BTreeMap::new();
    for ev in slice
        .iter()
        .filter(|e| e.kind == EVENT_INVESTIGATION_READING)
    {
        let Ok(payload) = serde_json::from_value::<MapReadingEventPayload>(ev.payload.clone())
        else {
            continue;
        };
        if payload.source_id != skeleton.source_id {
            continue;
        }
        for item in payload.items {
            if is_model_node_tag(&item.tag) {
                if let Some(parent) = item.attach_to.filter(|s| !s.trim().is_empty()) {
                    let kind = item
                        .model_kind
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or_else(|| "proof_step".to_string());
                    raw_links.insert(item.tag, (parent, kind));
                }
            }
        }
    }

    let mut resolved: BTreeMap<String, ModelNodeRef> = BTreeMap::new();
    for (tag, (attach_to, kind)) in &raw_links {
        if let Some((root_tag, root_fp)) = resolve_root_in_links(attach_to, &sk_fp, &raw_links) {
            resolved.insert(
                tag.clone(),
                ModelNodeRef {
                    tag: tag.clone(),
                    attach_to: attach_to.clone(),
                    kind: kind.clone(),
                    root_skeleton_tag: root_tag.to_string(),
                    node_fp: root_fp.to_string(),
                },
            );
        }
    }
    resolved
}

fn resolve_root_in_links<'a>(
    start_parent: &'a str,
    sk_fp: &BTreeMap<&'a str, &'a str>,
    raw_links: &'a BTreeMap<String, (String, String)>,
) -> Option<(&'a str, &'a str)> {
    let mut cur = start_parent;
    let mut visited: BTreeSet<&str> = BTreeSet::new();
    for _ in 0..MAX_ANCESTOR_HOPS {
        if let Some(&fp) = sk_fp.get(cur) {
            return Some((cur, fp));
        }
        if !visited.insert(cur) {
            return None;
        }
        let (next_parent, _) = raw_links.get(cur)?;
        cur = next_parent.as_str();
    }
    None
}

/// Resolves the ancestor chain (including `tag` itself) of a skeleton or model node back to its
/// owning skeleton `n-` tag. Returns `None` if a cycle or dangling parent is encountered.
pub fn ancestor_chain(
    tag: &str,
    sk_tags: &BTreeSet<&str>,
    model_nodes: &BTreeMap<String, ModelNodeRef>,
) -> Option<Vec<String>> {
    let mut chain = Vec::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut cur = tag;
    for _ in 0..MAX_ANCESTOR_HOPS {
        if !seen.insert(cur) {
            return None;
        }
        chain.push(cur.to_string());
        if sk_tags.contains(cur) {
            return Some(chain);
        }
        let m = model_nodes.get(cur)?;
        cur = m.attach_to.as_str();
    }
    None
}

/// Returns `true` if `candidate` equals or descends from any tag in `roots` through `model_nodes`.
pub fn is_model_descendant_of_any(
    candidate: &str,
    roots: &BTreeSet<String>,
    model_nodes: &BTreeMap<String, ModelNodeRef>,
) -> bool {
    let mut cur = candidate;
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for _ in 0..MAX_ANCESTOR_HOPS {
        if roots.contains(cur) {
            return true;
        }
        if !seen.insert(cur) {
            return false;
        }
        let Some(m) = model_nodes.get(cur) else {
            return false;
        };
        cur = m.attach_to.as_str();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_node_tag_is_deterministic_content_based_and_case_whitespace_invariant() {
        let t1 = model_node_tag(
            "n-abcdefghij",
            "proof_step",
            "Combining Lemma 1 with the discrete damping identity.",
        )
        .unwrap();
        let t2 = model_node_tag(
            "n-abcdefghij",
            "PROOF_STEP",
            "  combining   Lemma 1\n\twith the discrete damping identity. ",
        )
        .unwrap();
        assert_eq!(t1, t2);
        assert!(t1.starts_with("m-"));
        assert_eq!(t1.len(), 12);
        assert!(t1[2..]
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'2'..=b'7')));

        let diff_parent = model_node_tag(
            "m-234567abcd",
            "proof_step",
            "Combining Lemma 1 with the discrete damping identity.",
        )
        .unwrap();
        let diff_kind = model_node_tag(
            "n-abcdefghij",
            "open_question",
            "Combining Lemma 1 with the discrete damping identity.",
        )
        .unwrap();
        let diff_quote = model_node_tag(
            "n-abcdefghij",
            "proof_step",
            "Different first quote entirely.",
        )
        .unwrap();
        assert_ne!(t1, diff_parent);
        assert_ne!(t1, diff_kind);
        assert_ne!(t1, diff_quote);
    }

    #[test]
    fn model_node_tag_rejects_bad_inputs_and_enforces_role_restrictions() {
        assert!(model_node_tag("bad-tag", "proof_step", "quote").is_err());
        assert!(model_node_tag("n-abc", "", "quote").is_err());
        assert!(model_node_tag("n-abc", "bad kind!", "quote").is_err());
        assert!(model_node_tag("n-abc", "proof_step", "   ").is_err());

        assert!(!role_allowed_for_model_kind(ReadingRole::MainResult, "proof_step"));
        assert!(!role_allowed_for_model_kind(ReadingRole::MainResult, "open_question"));
        assert!(role_allowed_for_model_kind(ReadingRole::Supporting, "proof_step"));
        assert!(role_allowed_for_model_kind(ReadingRole::MainResult, "result"));
    }

    #[test]
    fn arbitrary_depth_ancestor_and_descendant_resolution() {
        let sk_tags: BTreeSet<&str> = ["n-root111111"].into_iter().collect();
        let mut models = BTreeMap::new();
        models.insert(
            "m-depth1aaaa".into(),
            ModelNodeRef {
                tag: "m-depth1aaaa".into(),
                attach_to: "n-root111111".into(),
                kind: "proof_step".into(),
                root_skeleton_tag: "n-root111111".into(),
                node_fp: "fp123".into(),
            },
        );
        models.insert(
            "m-depth2bbbb".into(),
            ModelNodeRef {
                tag: "m-depth2bbbb".into(),
                attach_to: "m-depth1aaaa".into(),
                kind: "proof_step".into(),
                root_skeleton_tag: "n-root111111".into(),
                node_fp: "fp123".into(),
            },
        );
        models.insert(
            "m-depth3cccc".into(),
            ModelNodeRef {
                tag: "m-depth3cccc".into(),
                attach_to: "m-depth2bbbb".into(),
                kind: "substep".into(),
                root_skeleton_tag: "n-root111111".into(),
                node_fp: "fp123".into(),
            },
        );

        let chain = ancestor_chain("m-depth3cccc", &sk_tags, &models).unwrap();
        assert_eq!(
            chain,
            vec!["m-depth3cccc", "m-depth2bbbb", "m-depth1aaaa", "n-root111111"]
        );

        let d1_root: BTreeSet<String> = ["m-depth1aaaa".to_string()].into_iter().collect();
        assert!(is_model_descendant_of_any("m-depth3cccc", &d1_root, &models));
        assert!(!is_model_descendant_of_any("n-root111111", &d1_root, &models));
    }
}
