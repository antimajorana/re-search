//! Rule 6.6 authors'-claim validation and app-filled deterministic cue extraction
//! for `record_map_reading` (package V7, `investigate.md` §4.3, rule 6.6).

use crate::investigation::anchor::bounded;
use crate::map_defs::{
    AuthorsClaimLabel, AuthorsClaimRecord, IndexedFault, VerifiedMapAnchor, MAX_READING_ANCHORS,
};
use crate::status_badge::detect_cues_from_anchors;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;

const MAX_RELATION_NOTE_CHARS: usize = 600;
const MAX_ATTRIBUTED_TO: usize = 16;
const MAX_BIB_KEY_CHARS: usize = 128;

fn default_vec<T>() -> Vec<T> {
    Vec::new()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, bound(deserialize = "A: Deserialize<'de>"))]
pub(crate) struct InputAuthorsClaim<A> {
    pub label: AuthorsClaimLabel,
    #[serde(default)]
    pub relation_note: Option<String>,
    #[serde(default)]
    pub attributed_to: Vec<String>,
    #[serde(default = "default_vec")]
    pub anchors: Vec<A>,
}

/// Fault info returned by an anchor verifier callback.
pub(crate) struct ClaimAnchorFault {
    pub owner: String,
    pub code: String,
    pub message: String,
    pub true_text: Option<String>,
}

fn fault(
    index: usize,
    code: impl Into<String>,
    message: impl Into<String>,
    true_text: Option<String>,
) -> IndexedFault {
    IndexedFault {
        index,
        code: code.into(),
        message: message.into(),
        true_text,
    }
}

/// Validates an [`InputAuthorsClaim`] under rule 6.6 and fills `cues` deterministically from
/// the matched primary-paper quoted lines.
///
/// Enforces:
/// - `new`, `restated`, and `background` require at least one matched anchor from `primary_source_id`;
/// - every `attributed_to` key must exist in `bib_keys` (the pinned skeleton bibliography);
/// - `cues` is always app-filled via [`detect_cues_from_anchors`] (caller-supplied `cues` is
///   already rejected recursively by `record_map_reading`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn validate_authors_claim<A, F>(
    index: usize,
    ac: InputAuthorsClaim<A>,
    primary_source_id: &str,
    bib_keys: &BTreeSet<&str>,
    item_kept_anchors: &[VerifiedMapAnchor],
    item_anchor_faults: &mut Vec<ClaimAnchorFault>,
    pending_dropped: &mut Vec<Value>,
    verify_anchors: F,
) -> Result<AuthorsClaimRecord, IndexedFault>
where
    F: FnOnce(&[A]) -> (Vec<VerifiedMapAnchor>, Vec<ClaimAnchorFault>),
{
    if ac
        .relation_note
        .as_deref()
        .is_some_and(|s| !bounded(s, MAX_RELATION_NOTE_CHARS))
        || ac.attributed_to.len() > MAX_ATTRIBUTED_TO
        || ac.attributed_to.iter().any(|k| !bounded(k, MAX_BIB_KEY_CHARS))
        || ac.anchors.len() > MAX_READING_ANCHORS
    {
        return Err(fault(
            index,
            "invalid_item",
            "Invalid `authors_claim` field bounds.",
            None,
        ));
    }

    let mut clean_attributed = Vec::with_capacity(ac.attributed_to.len());
    for raw_key in &ac.attributed_to {
        let key = raw_key.trim();
        if !bib_keys.contains(key) {
            return Err(fault(
                index,
                "unknown_bib_key",
                format!("Bibliography key `{key}` not in skeleton."),
                None,
            ));
        }
        if !clean_attributed.iter().any(|k: &String| k == key) {
            clean_attributed.push(key.to_string());
        }
    }

    let (mut ac_kept, ac_faults) = verify_anchors(&ac.anchors);
    if !ac.anchors.is_empty() && ac_kept.is_empty() {
        return Err(match ac_faults.into_iter().next() {
            Some(first) => fault(index, first.code, first.message, first.true_text),
            None => fault(
                index,
                "missing_authors_claim_anchor",
                "No authors_claim anchor matched.",
                None,
            ),
        });
    }
    pending_dropped.extend(
        ac_faults
            .iter()
            .map(|e| json!({ "owner": e.owner, "code": e.code })),
    );

    // Ensure every matched anchor belongs to the pinned primary paper (rule 6.6).
    if let Some(bad_anchor) = ac_kept.iter().find(|a| a.source_id != primary_source_id) {
        return Err(fault(
            index,
            "non_primary_source",
            format!(
                "Authors' claim anchor source_id `{}` must match primary paper `{primary_source_id}`.",
                bad_anchor.source_id
            ),
            None,
        ));
    }

    if ac.label != AuthorsClaimLabel::NotStated && ac_kept.is_empty() {
        let primary_item_anchors: Vec<VerifiedMapAnchor> = item_kept_anchors
            .iter()
            .filter(|a| a.source_id == primary_source_id)
            .cloned()
            .collect();
        if !primary_item_anchors.is_empty() {
            ac_kept = primary_item_anchors;
        } else if let Some(first) = item_anchor_faults.drain(..).next() {
            return Err(fault(index, first.code, first.message, first.true_text));
        } else {
            return Err(fault(
                index,
                "missing_authors_claim_anchor",
                "Non-`not_stated` authors_claim needs a matched primary-paper anchor.",
                None,
            ));
        }
    }

    // Rule 6.6: app fills deterministic cue phrases from the matched quoted lines.
    let cues = detect_cues_from_anchors(&ac_kept);

    Ok(AuthorsClaimRecord {
        label: ac.label,
        relation_note: ac
            .relation_note
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        attributed_to: clean_attributed,
        anchors: ac_kept,
        cues,
    })
}
