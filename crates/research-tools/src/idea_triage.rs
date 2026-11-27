//! Explore Triage model output schema, Rule C3 verbatim abstract-quote verification,
//! and shared F4 `evidence.abstract` (`xa-<seq>`) recorder (`explore.md` 4.1, 5.1, rules C3 & V2;
//! package X8).
//!
//! Governance:
//! - `#[serde(deny_unknown_fields)]` rejects any extra model fields.
//! - Rule C3: an `abstract_quote` becomes shared F4 `evidence.abstract` (`xa-<seq>`) ONLY when
//!   `abstract_ref.verbatim == true` and F11's `locate_abstract_quote_in_blob` finds the
//!   whitespace-normalized quote inside the stored raw provider response blob, filling exact
//!   `(start, end)` byte offsets. Rebuilt (non-verbatim) abstracts can be triaged with
//!   `abstract_quote: null`, but can NEVER produce `evidence.abstract`.
//! - Rule S3: deduplicates identical `(paper_key, raw_blob, quote_sha256)` abstract quotes
//!   within the current statement revision by reusing the existing `xa-<seq>`.
//! - Rule V2: Quick preset (`max_papers == 0`) requires an empty `reading_list` and records
//!   `quick_abstract_only: true` so downstream mapping proceeds from abstracts only.

use crate::lit_abstracts::{locate_abstract_quote_in_blob, AbstractRef};
use crate::lit_providers::LitProvider;
use crate::paper_key::PaperKey;
use crate::quote_core::{
    abstract_id, quote_sha256, EvidenceAbstractPayload, EVIDENCE_ABSTRACT_EVENT,
    MAX_DISPLAY_QUOTE_CHARS,
};
use ledger::Ledger;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use tools::EVENT_SCHEMA_VERSION;

/// Recorder tool id offered during the Triage exchange (`explore.md` 5.1).
pub const RECORDER_IDEA_TRIAGE: &str = "record_idea_triage";
/// Explore ledger event kind written by the Triage recorder (`explore.md` 4.2).
pub const EVENT_IDEA_TRIAGE: &str = "idea.triage";
/// Required on-screen label qualifier for every triage classification (`explore.md` 3).
pub const UI_FROM_ABSTRACT_LABEL: &str = "from abstract";

/// Maximum candidate items triaged in a single Triage call (Thorough preset `max_screened = 80`).
pub const MAX_TRIAGE_ITEMS: usize = 80;
/// Maximum character length of a triage item `why` justification (`explore.md` 4.1).
pub const MAX_TRIAGE_WHY_CHARS: usize = 200;

/// Triage relevance group (`explore.md` 4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriageRelevance {
    Core,
    Related,
    Background,
    Unrelated,
}

impl TriageRelevance {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Related => "related",
            Self::Background => "background",
            Self::Unrelated => "unrelated",
        }
    }
}

/// Raw per-candidate triage item submitted by the model (`#[serde(deny_unknown_fields)]`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriageItemInput {
    pub key: String,
    pub relevance: TriageRelevance,
    #[serde(default)]
    pub subclaim_ids: Vec<String>,
    pub why: String,
    #[serde(default)]
    pub abstract_quote: Option<String>,
}

/// Raw `record_idea_triage` tool input submitted by the model (`#[serde(deny_unknown_fields)]`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriageInput {
    pub items: Vec<TriageItemInput>,
    #[serde(default)]
    pub reading_list: Vec<String>,
}

/// Host-supplied candidate metadata required to verify Rule C3 quotes from stored raw blobs.
/// `alias_keys` holds any verified canonical aliases (e.g. `doi:...`) so cross-provider merged
/// candidates whose canonical key is `arxiv:...` can still verify abstracts from a Crossref/S2 blob.
#[derive(Debug, Clone)]
pub struct TriageCandidateContext {
    pub key: PaperKey,
    pub alias_keys: Vec<PaperKey>,
    pub abstract_text: Option<String>,
    pub abstract_ref: Option<AbstractRef>,
}

/// Validated candidate triage item before ledger persistence.
#[derive(Debug, Clone)]
pub struct ValidatedTriageItem {
    pub key: PaperKey,
    pub relevance: TriageRelevance,
    pub subclaim_ids: Vec<String>,
    pub why: String,
    pub evidence_payload: Option<EvidenceAbstractPayload>,
}

/// Validated full triage output ready to emit `evidence.abstract` and `idea.triage`.
#[derive(Debug, Clone)]
pub struct ValidatedTriage {
    pub items: Vec<ValidatedTriageItem>,
    pub reading_list: Vec<PaperKey>,
    pub quick_abstract_only: bool,
}

/// Stored per-candidate item written inside `idea.triage["items"]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriageItemRecord {
    pub key: PaperKey,
    pub relevance: TriageRelevance,
    pub subclaim_ids: Vec<String>,
    pub why: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abstract_quote: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_id: Option<String>,
    pub label_basis: String,
}

/// Persisted result of [`record_idea_triage`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriageOutcome {
    pub triage_seq: u64,
    pub items: Vec<TriageItemRecord>,
    pub reading_list: Vec<PaperKey>,
    pub abstract_evidence_ids: Vec<String>,
    pub quick_abstract_only: bool,
}

/// Strict JSON Schema for the `record_idea_triage` recorder tool (`explore.md` 4.1 & 5.1).
pub fn record_idea_triage_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "items": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_TRIAGE_ITEMS,
                "items": {
                    "type": "object",
                    "properties": {
                        "key": { "type": "string", "minLength": 1, "maxLength": 512 },
                        "relevance": {
                            "type": "string",
                            "enum": ["core", "related", "background", "unrelated"]
                        },
                        "subclaim_ids": {
                            "type": "array",
                            "maxItems": 4,
                            "items": { "type": "string", "minLength": 1, "maxLength": 32 }
                        },
                        "why": { "type": "string", "minLength": 1, "maxLength": MAX_TRIAGE_WHY_CHARS },
                        "abstract_quote": {
                            "type": ["string", "null"],
                            "maxLength": MAX_DISPLAY_QUOTE_CHARS
                        }
                    },
                    "required": ["key", "relevance", "why"],
                    "additionalProperties": false
                }
            },
            "reading_list": {
                "type": "array",
                "maxItems": 12,
                "items": { "type": "string", "minLength": 1, "maxLength": 512 }
            }
        },
        "required": ["items", "reading_list"],
        "additionalProperties": false
    })
}

/// Validate `raw` against schema limits, candidate membership, Quick preset `max_papers` cap,
/// and Rule C3 verbatim blob substring matching.
pub fn validate_triage_input(
    raw: &Value,
    candidates: &[TriageCandidateContext],
    valid_subclaim_ids: &[String],
    max_papers: usize,
    ledger: &Ledger,
) -> Result<ValidatedTriage, Vec<String>> {
    let parsed: TriageInput = serde_json::from_value(raw.clone())
        .map_err(|e| vec![format!("invalid `record_idea_triage` payload: {e}")])?;

    let mut errs = Vec::new();
    if parsed.items.is_empty() || parsed.items.len() > MAX_TRIAGE_ITEMS {
        errs.push(format!(
            "`items` must contain 1..={MAX_TRIAGE_ITEMS} entries (got {})",
            parsed.items.len()
        ));
    }

    let cand_by_key: BTreeMap<&PaperKey, &TriageCandidateContext> =
        candidates.iter().map(|c| (&c.key, c)).collect();
    let valid_sc_set: BTreeSet<&str> = valid_subclaim_ids.iter().map(String::as_str).collect();
    let mut seen_item_keys: BTreeSet<PaperKey> = BTreeSet::new();
    let mut validated_items: Vec<ValidatedTriageItem> = Vec::with_capacity(parsed.items.len());

    for (idx, item) in parsed.items.iter().enumerate() {
        let key = match PaperKey::parse(item.key.trim()) {
            Ok(k) => k,
            Err(e) => {
                errs.push(format!("items[{idx}].key `{}` invalid: {e}", item.key));
                continue;
            }
        };
        if !seen_item_keys.insert(key.clone()) {
            errs.push(format!("items[{idx}].key `{key}` is duplicated"));
            continue;
        }
        let Some(&cand) = cand_by_key.get(&key) else {
            errs.push(format!(
                "items[{idx}].key `{key}` is not among the candidates supplied to Triage"
            ));
            continue;
        };

        let why = item.why.trim().to_string();
        if why.is_empty() || why.chars().count() > MAX_TRIAGE_WHY_CHARS {
            errs.push(format!(
                "items[{idx}].why must be non-empty and <= {MAX_TRIAGE_WHY_CHARS} characters"
            ));
        }

        let mut clean_scs = Vec::new();
        for sc in &item.subclaim_ids {
            let trimmed = sc.trim();
            if trimmed.is_empty() {
                errs.push(format!("items[{idx}].subclaim_ids contains an empty entry"));
                continue;
            }
            if !valid_sc_set.is_empty() && !valid_sc_set.contains(trimmed) {
                errs.push(format!(
                    "items[{idx}].subclaim_ids contains unknown subclaim `{trimmed}`"
                ));
                continue;
            }
            if !is_valid_sc_id(trimmed) {
                errs.push(format!(
                    "items[{idx}].subclaim_ids `{trimmed}` must match `sc-<positive digits>`"
                ));
                continue;
            }
            if !clean_scs.iter().any(|s| s == trimmed) {
                clean_scs.push(trimmed.to_string());
            }
        }

        let raw_quote = item
            .abstract_quote
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());

        let has_verbatim_abs = cand
            .abstract_ref
            .as_ref()
            .is_some_and(|r| r.verbatim && !r.raw_blob.is_empty())
            && cand.abstract_text.as_deref().is_some_and(|s| !s.trim().is_empty());

        if raw_quote.is_none()
            && matches!(item.relevance, TriageRelevance::Core | TriageRelevance::Related)
            && has_verbatim_abs
        {
            errs.push(format!(
                "items[{idx}] (`{key}`, {}) has a verbatim stored abstract and must supply a verbatim `abstract_quote`",
                item.relevance.as_str()
            ));
            continue;
        }

        let mut evidence_payload = None;
        if let Some(quote) = raw_quote {
            if quote.chars().count() > MAX_DISPLAY_QUOTE_CHARS {
                errs.push(format!(
                    "items[{idx}].abstract_quote exceeds {MAX_DISPLAY_QUOTE_CHARS} characters"
                ));
                continue;
            }
            match &cand.abstract_ref {
                None => {
                    errs.push(format!(
                        "items[{idx}] (`{key}`) has no stored raw response blob (`abstract_ref`); omit `abstract_quote` (Rule C3)"
                    ));
                }
                Some(aref) if !aref.verbatim => {
                    errs.push(format!(
                        "items[{idx}] (`{key}`) has a rebuilt (non-verbatim) abstract and can never produce abstract evidence; omit `abstract_quote` (Rule C3)"
                    ));
                }
                Some(aref) => {
                    let Some(provider) = LitProvider::parse(&aref.provider) else {
                        errs.push(format!(
                            "items[{idx}] (`{key}`) abstract_ref provider `{}` is unsupported",
                            aref.provider
                        ));
                        continue;
                    };
                    let raw_bytes = match ledger.get_blob(&aref.raw_blob) {
                        Ok(b) => b,
                        Err(e) => {
                            errs.push(format!(
                                "items[{idx}] (`{key}`) failed reading raw blob `{}`: {e} (Rule C3)",
                                aref.raw_blob
                            ));
                            continue;
                        }
                    };
                    let match_res = std::iter::once(&key)
                        .chain(cand.alias_keys.iter())
                        .find_map(|k| locate_abstract_quote_in_blob(provider, &raw_bytes, k, quote).ok())
                        .ok_or_else(|| {
                            locate_abstract_quote_in_blob(provider, &raw_bytes, &key, quote)
                                .unwrap_err()
                        });
                    match match_res {
                        Ok(matched) => {
                            match EvidenceAbstractPayload::new(
                                key.clone(),
                                matched.matched_text,
                                matched.start,
                                matched.end,
                                aref.raw_blob.clone(),
                                aref.provider.clone(),
                                None,
                                clean_scs.clone(),
                                Some(why.clone()),
                            ) {
                                Ok(ev) => evidence_payload = Some(ev),
                                Err(e) => errs.push(format!(
                                    "items[{idx}] (`{key}`) invalid evidence.abstract payload: {e}"
                                )),
                            }
                        }
                        Err(e) => {
                            errs.push(format!(
                                "items[{idx}] (`{key}`) abstract_quote did not match stored verbatim abstract: {e} (Rule C3)"
                            ));
                        }
                    }
                }
            }
        }

        validated_items.push(ValidatedTriageItem {
            key,
            relevance: item.relevance,
            subclaim_ids: clean_scs,
            why,
            evidence_payload,
        });
    }

    let missing_keys: Vec<String> = candidates
        .iter()
        .filter(|c| !seen_item_keys.contains(&c.key))
        .map(|c| c.key.as_str().to_string())
        .collect();
    if !missing_keys.is_empty() {
        errs.push(format!(
            "`items` must classify all {} supplied candidate(s); missing: {}",
            candidates.len(),
            missing_keys.join(", ")
        ));
    }

    let mut reading_list: Vec<PaperKey> = Vec::new();
    let mut seen_rl: BTreeSet<PaperKey> = BTreeSet::new();
    for (idx, raw_key) in parsed.reading_list.iter().enumerate() {
        let key = match PaperKey::parse(raw_key.trim()) {
            Ok(k) => k,
            Err(e) => {
                errs.push(format!("reading_list[{idx}] `{raw_key}` invalid: {e}"));
                continue;
            }
        };
        if !seen_item_keys.contains(&key) {
            errs.push(format!(
                "reading_list[{idx}] `{key}` is not among the triaged candidates"
            ));
            continue;
        }
        if seen_rl.insert(key.clone()) {
            reading_list.push(key);
        }
    }

    if max_papers == 0 && !reading_list.is_empty() {
        errs.push(
            "Quick preset reads 0 full papers (`max_papers == 0`); `reading_list` must be empty (Rule V2)"
                .to_string(),
        );
    } else if reading_list.len() > max_papers {
        errs.push(format!(
            "`reading_list` contains {} papers, exceeding preset cap of {max_papers}",
            reading_list.len()
        ));
    }

    if !errs.is_empty() {
        return Err(errs);
    }

    Ok(ValidatedTriage {
        items: validated_items,
        reading_list,
        quick_abstract_only: max_papers == 0,
    })
}

/// Validate `raw`, emit deduplicated shared F4 `evidence.abstract` (`xa-<seq>`) events for
/// verbatim matched quotes (Rule C3 + Rule S3), and append `idea.triage` to `ledger`.
#[allow(clippy::too_many_arguments)]
pub fn record_idea_triage(
    ledger: &Ledger,
    session_id: &str,
    stmt_seq: u64,
    candidates: &[TriageCandidateContext],
    valid_subclaim_ids: &[String],
    max_papers: usize,
    request_seq: Option<u64>,
    raw: &Value,
) -> Result<TriageOutcome, Vec<String>> {
    let validated =
        validate_triage_input(raw, candidates, valid_subclaim_ids, max_papers, ledger)?;

    // Build Rule S3 deduplication map over existing `evidence.abstract` events in this revision.
    let existing = ledger
        .read_session(session_id)
        .map_err(|e| vec![format!("failed reading session ledger: {e}")])?;
    let mut existing_xa: BTreeMap<(String, String, String), String> = BTreeMap::new();
    for ev in existing
        .iter()
        .filter(|e| e.kind == EVIDENCE_ABSTRACT_EVENT && e.seq > stmt_seq)
    {
        if let (Some(pk), Some(blob), Some(qsha)) = (
            ev.payload.get("paper_key").and_then(Value::as_str),
            ev.payload.get("raw_blob").and_then(Value::as_str),
            ev.payload.get("quote_sha256").and_then(Value::as_str),
        ) {
            if let Ok(id) = abstract_id(ev.seq) {
                existing_xa.insert((pk.to_string(), blob.to_string(), qsha.to_string()), id);
            }
        }
    }

    let mut records: Vec<TriageItemRecord> = Vec::with_capacity(validated.items.len());
    let mut abstract_evidence_ids: Vec<String> = Vec::new();

    for item in validated.items {
        let mut evidence_id = None;
        let mut abstract_quote = None;

        if let Some(ev_payload) = item.evidence_payload {
            let dedupe_key = (
                ev_payload.paper_key.as_str().to_string(),
                ev_payload.raw_blob.clone(),
                quote_sha256(&ev_payload.quote),
            );
            let xa_id = if let Some(reused) = existing_xa.get(&dedupe_key) {
                reused.clone()
            } else {
                let ev_val = serde_json::to_value(&ev_payload)
                    .map_err(|e| vec![format!("failed serializing evidence.abstract: {e}")])?;
                let seq = ledger
                    .append(
                        session_id,
                        EVIDENCE_ABSTRACT_EVENT,
                        EVENT_SCHEMA_VERSION,
                        &ev_val,
                    )
                    .map_err(|e| vec![format!("failed appending evidence.abstract: {e}")])?;
                let id = abstract_id(seq).map_err(|e| vec![e])?;
                existing_xa.insert(dedupe_key, id.clone());
                id
            };
            if !abstract_evidence_ids.contains(&xa_id) {
                abstract_evidence_ids.push(xa_id.clone());
            }
            abstract_quote = Some(ev_payload.quote);
            evidence_id = Some(xa_id);
        }

        records.push(TriageItemRecord {
            key: item.key,
            relevance: item.relevance,
            subclaim_ids: item.subclaim_ids,
            why: item.why,
            abstract_quote,
            evidence_id,
            label_basis: UI_FROM_ABSTRACT_LABEL.to_string(),
        });
    }

    let triage_payload = json!({
        "items": records,
        "reading_list": validated.reading_list,
        "abstract_evidence_ids": abstract_evidence_ids,
        "quick_abstract_only": validated.quick_abstract_only,
        "request_seq": request_seq,
    });
    let triage_seq = ledger
        .append(
            session_id,
            EVENT_IDEA_TRIAGE,
            EVENT_SCHEMA_VERSION,
            &triage_payload,
        )
        .map_err(|e| vec![format!("failed appending idea.triage: {e}")])?;

    Ok(TriageOutcome {
        triage_seq,
        items: records,
        reading_list: validated.reading_list,
        abstract_evidence_ids,
        quick_abstract_only: validated.quick_abstract_only,
    })
}

fn is_valid_sc_id(s: &str) -> bool {
    let Some(rest) = s.strip_prefix("sc-") else { return false };
    !rest.is_empty() && !rest.starts_with('0') && rest.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
#[path = "idea_triage_tests.rs"]
mod tests;
